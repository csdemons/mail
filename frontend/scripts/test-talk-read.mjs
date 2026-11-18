// Regression tests for Talk read/notification races. Uses the project's TypeScript
// compiler and Node's test runner; no browser, server or extra packages are needed.
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { setImmediate } from 'node:timers/promises'
import test from 'node:test'
import ts from 'typescript'

const contexts = new Map()
globalThis.__atomTalkReadTestContexts = contexts
let nextContext = 0

const flush = () => setImmediate()
const room = (overrides = {}) => ({
  token: 'room-a', name: 'Test room', kind: 2, unread: 1, unreadMention: true,
  lastMessageId: 100, lastRead: 99, lastActivity: 1234, lastMessage: 'Message',
  ...overrides,
})

function deferred() {
  let resolvePromise, rejectPromise
  const request = {
    settled: false,
    promise: new Promise((resolve, reject) => { resolvePromise = resolve; rejectPromise = reject }),
    resolve(value) { assert.equal(request.settled, false); request.settled = true; resolvePromise(value) },
    reject(error) { assert.equal(request.settled, false); request.settled = true; rejectPromise(error) },
  }
  return request
}

function fakeClock() {
  let now = 1_000_000, nextTimer = 0
  const timers = new Map()
  return {
    get now() { return now },
    setTimeout(callback, ms = 0, ...args) {
      const id = ++nextTimer
      timers.set(id, { at: now + ms, run: () => callback(...args) })
      return id
    },
    clearTimeout(id) { timers.delete(id) },
    async advance(ms) {
      const until = now + ms
      for (let n = 0; n < 1000; n++) {
        const entry = [...timers].filter(([, t]) => t.at <= until).sort((a, b) => a[1].at - b[1].at)[0]
        if (!entry) { now = until; await flush(); return }
        const [id, timer] = entry
        now = timer.at
        timers.delete(id)
        timer.run()
        await flush()
      }
      assert.fail('Timer loop did not settle')
    },
  }
}

async function transpile(filename) {
  const source = await readFile(new URL(`../src/${filename}`, import.meta.url), 'utf8')
  return ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
    fileName: filename,
  }).outputText
}

async function makeStore(t) {
  const id = ++nextContext
  const clock = fakeClock(), gets = [], writes = [], resets = [], warnings = []
  const context = {
    clock,
    api: {
      talkRooms() { const req = deferred(); gets.push(req); return req.promise },
      talkManage(token, action, value = {}) {
        const req = Object.assign(deferred(), { token, action, value })
        writes.push(req)
        return req.promise
      },
    },
    onSessionReset(fn) { resets.push(fn) },
    errorText(error) { return error instanceof Error ? error.message : String(error) },
    useSyncExternalStore(_subscribe, snapshot) { return snapshot() },
    console: { warn(...args) { warnings.push(args) }, error(...args) { warnings.push(args) } },
  }
  contexts.set(id, context)
  let code = await transpile('talkRoomsStore.ts')
  code = code.replace(/import\s*\{([^}]+)\}\s*from\s*['"](?:react|\.\/api)['"];?/g,
    (_, names) => `const { ${names} } = context;`)
  code = `const context = globalThis.__atomTalkReadTestContexts.get(${id});
const setTimeout = context.clock.setTimeout, clearTimeout = context.clock.clearTimeout;
const Date = class extends globalThis.Date { static now() { return context.clock.now; } };
const console = context.console;
${code}`
  const store = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`)
  t.after(() => { store.resetRooms(); contexts.delete(id) })
  const respond = (list) => {
    const req = gets.find((g) => !g.settled)
    assert.ok(req, 'Expected a pending rooms request')
    req.resolve(structuredClone(list))
  }
  const fetch = async (list) => {
    const promise = store.refreshRooms()
    respond(list)
    await promise
    await flush()
  }
  return { store, clock, gets, writes, resets, warnings, respond, fetch }
}

test('a delayed rooms response cannot undo an observed read; raw notification state is preserved', async (t) => {
  const h = await makeStore(t)
  await h.fetch([room()])
  const late = h.store.refreshRooms()
  h.store.markRoomRead('room-a', 100)
  assert.equal(h.store.currentRooms()[0].unread, 0)
  assert.equal(h.store.currentServerRooms()[0].unread, 1)
  h.respond([room()])
  await late
  assert.equal(h.store.currentRooms()[0].unread, 0)
  assert.equal(h.store.currentRooms()[0].unreadMention, false)
})

test('pending read coverage does not expire after 60 seconds', async (t) => {
  const h = await makeStore(t)
  await h.fetch([room()])
  h.store.markRoomRead('room-a', 100)
  await h.clock.advance(120_000)
  await h.fetch([room()])
  assert.equal(h.store.currentRooms()[0].unread, 0)
})

test('a newer message in the same second remains unread while the older read is pending', async (t) => {
  const h = await makeStore(t)
  await h.fetch([room()])
  h.store.markRoomRead('room-a', 100)
  await h.fetch([room({ lastMessageId: 101, unread: 2 })])
  assert.equal(h.store.currentRooms()[0].unread, 2)
  assert.equal(h.store.currentRooms()[0].unreadMention, true)
})

test('missing message IDs are never treated as proof that all unread messages were seen', async (t) => {
  const h = await makeStore(t)
  await h.fetch([room({ lastMessageId: 0, sensitive: true })])
  h.store.markRoomRead('room-a', 100)
  assert.equal(h.store.currentRooms()[0].unread, 1)
  await h.fetch([room({ lastMessageId: 0, sensitive: true, unread: 2 })])
  assert.equal(h.store.currentRooms()[0].unread, 2)
})

test('writes are serialized and a failed older target retries the newest observed ID', async (t) => {
  const h = await makeStore(t)
  await h.fetch([room()])
  h.store.markRoomRead('room-a', 100)
  h.store.markRoomRead('room-a', 102)
  await flush()
  assert.equal(h.writes.length, 1)
  assert.equal(h.writes[0].value.lastId, 100)
  h.writes[0].reject(new Error('Temporary network failure'))
  await flush()
  await h.clock.advance(1000)
  assert.deepEqual(h.writes.map((w) => w.value.lastId), [100, 102])
  h.writes[1].resolve()
  await flush()
  h.respond([room({ lastMessageId: 102, lastRead: 102, unread: 0 })])
  await flush()
  await h.clock.advance(120_000)
  assert.deepEqual(h.writes.map((w) => w.value.lastId), [100, 102])
})

test('successful writes coalesce queued reads without sending an older target afterward', async (t) => {
  const h = await makeStore(t)
  await h.fetch([room()])
  h.store.markRoomRead('room-a', 100)
  h.store.markRoomRead('room-a', 101)
  h.store.markRoomRead('room-a', 103)
  h.store.markRoomRead('room-a', 102)
  await flush()
  assert.equal(h.writes.length, 1)
  h.writes[0].resolve()
  await flush()
  assert.deepEqual(h.writes.map((w) => w.value.lastId), [100, 103])
})

test('a GET begun before a successful read is discarded and followed by a fresh GET', async (t) => {
  const h = await makeStore(t)
  await h.fetch([room()])
  const late = h.store.refreshRooms()
  h.store.markRoomRead('room-a', 100)
  h.writes[0].resolve()
  await flush()
  h.respond([room({ unread: 7 })])
  await late
  await flush()
  assert.equal(h.store.currentServerRooms()[0].unread, 1)
  assert.equal(h.store.currentRooms()[0].unread, 0)
  assert.equal(h.gets.length, 3)
  h.respond([room({ lastRead: 100, unread: 0, unreadMention: false })])
  await flush()
  assert.equal(h.store.currentServerRooms()[0].unread, 0)
})

test('confirmed pending reads retire so later explicit unread from another client is respected', async (t) => {
  const h = await makeStore(t)
  await h.fetch([room()])
  h.store.markRoomRead('room-a', 100)
  h.writes[0].resolve()
  await flush()
  h.respond([room({ lastRead: 100, unread: 0 })])
  await flush()
  await h.fetch([room({ lastRead: 0, unread: 1 })])
  assert.equal(h.store.currentRooms()[0].unread, 1)
})

test('manual read without a last-message ID preserves the explicit mark-all operation', async (t) => {
  const h = await makeStore(t)
  await h.fetch([room({ lastMessageId: 0, lastRead: 100, unread: 1 })])
  const manual = h.store.setRoomRead('room-a', true)
  await flush()
  assert.equal(h.writes.length, 1)
  assert.equal(h.writes[0].action, 'read')
  assert.deepEqual(h.writes[0].value, {})
  h.writes[0].resolve()
  await flush()
  h.respond([room({ lastMessageId: 0, lastRead: 101, unread: 0 })])
  await manual
  assert.equal(h.store.currentRooms()[0].unread, 0)
})

test('messages seen during a manual read are queued and sent afterward without overlapping writes', async (t) => {
  const h = await makeStore(t)
  await h.fetch([room()])
  const manual = h.store.setRoomRead('room-a', true)
  await flush()
  h.store.markRoomRead('room-a', 102)
  assert.equal(h.writes.length, 1)
  h.writes[0].resolve()
  await flush()
  assert.equal(h.writes.length, 1)
  h.respond([room({ lastMessageId: 102, lastRead: 100, unread: 1 })])
  await manual
  await flush()
  assert.equal(h.writes.length, 2)
  assert.equal(h.writes[1].action, 'read')
  assert.deepEqual(h.writes[1].value, { lastId: 102 })
  h.writes[1].resolve()
  await flush()
  h.respond([room({ lastMessageId: 102, lastRead: 102, unread: 0 })])
  await flush()
  assert.equal(h.store.currentRooms()[0].unread, 0)
})

test('manual unread drains an in-flight automatic read and blocks new automatic writes', async (t) => {
  const h = await makeStore(t)
  await h.fetch([room()])
  h.store.markRoomRead('room-a', 100)
  const manual = h.store.setRoomRead('room-a', false)
  h.store.markRoomRead('room-a', 101)
  await flush()
  assert.equal(h.writes.length, 1)
  h.writes[0].resolve()
  await flush()
  assert.deepEqual(h.writes.map((w) => w.action), ['read', 'unread'])
  h.writes[1].resolve()
  await flush()
  h.respond([room({ lastRead: 0, unread: 1 })])
  await manual
  await h.clock.advance(120_000)
  assert.equal(h.store.currentRooms()[0].unread, 1)
  assert.deepEqual(h.writes.map((w) => w.action), ['read', 'unread'])
})

test('a failed manual unread restores the previously pending automatic read without overlapping writes', async (t) => {
  const h = await makeStore(t)
  await h.fetch([room()])
  h.store.markRoomRead('room-a', 100)
  h.writes[0].reject(new Error('Read temporarily offline'))
  await flush()
  const failed = assert.rejects(h.store.setRoomRead('room-a', false), /Unread temporarily offline/)
  await flush()
  assert.deepEqual(h.writes.map((w) => w.action), ['read', 'unread'])
  // The previous read's retry timer must not run alongside the manual request.
  await h.clock.advance(30_000)
  assert.equal(h.writes.length, 2)
  h.writes[1].reject(new Error('Unread temporarily offline'))
  await failed
  await flush()
  assert.deepEqual(h.writes.map((w) => [w.action, w.value.lastId]), [
    ['read', 100], ['unread', undefined], ['read', 100],
  ])
  h.writes[2].resolve()
  await flush()
  h.respond([room({ lastRead: 100, unread: 0 })])
  await flush()
  await h.clock.advance(120_000)
  assert.equal(h.writes.length, 3)
  assert.equal(h.store.currentRooms()[0].unread, 0)
})

test('account reset cancels retry timers and discards old rooms responses', async (t) => {
  const h = await makeStore(t)
  await h.fetch([room()])
  h.store.markRoomRead('room-a', 100)
  h.writes[0].reject(new Error('Offline'))
  await flush()
  const oldGet = h.store.refreshRooms()
  assert.ok(h.resets.length, 'Store must register with onSessionReset')
  h.resets.forEach((reset) => reset())
  h.respond([room({ unread: 4 })])
  await oldGet
  await h.clock.advance(120_000)
  assert.equal(h.store.currentRooms(), null)
  assert.equal(h.store.currentServerRooms(), null)
  assert.equal(h.writes.length, 1)
})

test('a late failed write after reset does not schedule work for the new account', async (t) => {
  const h = await makeStore(t)
  await h.fetch([room()])
  h.store.markRoomRead('room-a', 100)
  h.store.resetRooms()
  h.writes[0].reject(new Error('Old connection closed'))
  await flush()
  await h.clock.advance(120_000)
  await h.fetch([room({ lastMessageId: 10, lastRead: 9 })])
  assert.equal(h.writes.length, 1)
  assert.equal(h.store.currentRooms()[0].unread, 1)
})

async function notificationObserver() {
  const code = await transpile('talkNotificationState.ts')
  const module = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`)
  return module.observeTalkRoom
}

test('notification baseline and 1→0→1 unread bounce do not notify for the same ID', async () => {
  const observe = await notificationObserver()
  let result = observe(undefined, room())
  assert.equal(result.isNew, false)
  result = observe(result.seen, room({ unread: 0 }))
  assert.equal(result.isNew, false)
  result = observe(result.seen, room({ unread: 1 }))
  assert.equal(result.isNew, false)
})

test('notification identity advances for a higher ID with unchanged timestamp and unread count', async () => {
  const observe = await notificationObserver()
  const previous = observe(undefined, room()).seen
  assert.equal(observe(previous, room({ lastMessageId: 101 })).isNew, true)
})

test('a regressing room snapshot cannot reset the notification ID high-water mark', async () => {
  const observe = await notificationObserver()
  let result = observe(undefined, room({ lastMessageId: 102 }))
  result = observe(result.seen, room({ lastMessageId: 100 }))
  assert.equal(result.isNew, false)
  result = observe(result.seen, room({ lastMessageId: 102 }))
  assert.equal(result.isNew, false)
})

test('sensitive rooms without message IDs still notify for raw unread growth within one second', async () => {
  const observe = await notificationObserver()
  const previous = observe(undefined, room({ lastMessageId: 0, sensitive: true })).seen
  assert.equal(observe(previous, room({ lastMessageId: 0, sensitive: true, unread: 2 })).isNew, true)
})

test('marking a missing-ID conversation unread does not impersonate a new message', async () => {
  const observe = await notificationObserver()
  const previous = observe(undefined, room({ lastMessageId: 0, lastRead: 100, unread: 0 })).seen
  assert.equal(observe(previous, room({ lastMessageId: 0, lastRead: 0, unread: 1 })).isNew, false)
})

// The marker has no dependency on layout libraries: control the real event/RAF
// boundaries and DOM dimensions while exercising its production guards.
async function makeMarker(t, { focused = true, hidden = false, bottom = true } = {}) {
  const id = ++nextContext
  const window = new EventTarget(), document = new EventTarget(), el = new EventTarget()
  const frames = new Map(), marks = [], resets = [], cleanups = []
  let nextFrame = 0
  const context = {
    window, document, focused,
    useRef: (value) => ({ current: value }),
    useLayoutEffect(effect) { const cleanup = effect(); if (cleanup) cleanups.push(cleanup) },
    onSessionReset(fn) { resets.push(fn) },
    markRoomRead(token, lastId) { marks.push({ token, lastId }) },
    requestAnimationFrame(fn) { const frame = ++nextFrame; frames.set(frame, fn); return frame },
    cancelAnimationFrame(frame) { frames.delete(frame) },
  }
  document.hidden = hidden
  document.hasFocus = () => context.focused
  Object.assign(el, { clientHeight: 500, scrollHeight: 1000, scrollTop: bottom ? 500 : 0 })
  const session = { current: 7 }, stick = { current: bottom }, suspended = { current: null }
  contexts.set(id, context)
  let code = await transpile('useTalkReadMarker.ts')
  code = code.replace(/import\s*\{([^}]+)\}\s*from\s*['"](?:react|\.\/api|\.\/talkRoomsStore)['"];?/g,
    (_, names) => `const { ${names} } = context;`)
  code = `const context = globalThis.__atomTalkReadTestContexts.get(${id});
const { window, document, requestAnimationFrame, cancelAnimationFrame } = context;
${code}`
  const { useTalkReadMarker } = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`)
  useTalkReadMarker('room-a', { token: 'room-a', session: 7, lastId: 100 }, true,
    { current: el }, session, stick, suspended)
  const cleanup = () => { for (const fn of cleanups.splice(0)) fn() }
  t.after(() => { cleanup(); contexts.delete(id) })
  const paint = () => {
    const ready = [...frames.values()]
    frames.clear()
    for (const callback of ready) callback()
  }
  return { context, window, document, el, session, stick, suspended, marks, resets, paint, cleanup }
}

test('returning focus reads already-rendered messages without another poll response', async (t) => {
  const h = await makeMarker(t, { focused: false })
  h.paint()
  assert.equal(h.marks.length, 0)
  h.context.focused = true
  h.window.dispatchEvent(new Event('focus'))
  h.paint()
  assert.deepEqual(h.marks, [{ token: 'room-a', lastId: 100 }])
  h.window.dispatchEvent(new Event('focus'))
  h.paint()
  assert.equal(h.marks.length, 1)
})

test('a hidden chat is marked only after it becomes visible and focused', async (t) => {
  const h = await makeMarker(t, { hidden: true })
  h.paint()
  assert.equal(h.marks.length, 0)
  h.document.hidden = false
  h.document.dispatchEvent(new Event('visibilitychange'))
  h.paint()
  assert.equal(h.marks.length, 1)
})

test('reading older history does not mark the unseen end; scrolling to the end does', async (t) => {
  const h = await makeMarker(t, { bottom: false })
  h.paint()
  assert.equal(h.marks.length, 0)
  h.el.scrollTop = 500
  h.stick.current = true
  h.el.dispatchEvent(new Event('scroll'))
  h.paint()
  assert.deepEqual(h.marks, [{ token: 'room-a', lastId: 100 }])
})

test('manual unread suspension and obsolete conversation sessions block queued frames', async (t) => {
  const h = await makeMarker(t)
  h.suspended.current = 'room-a'
  h.paint()
  assert.equal(h.marks.length, 0)
  h.suspended.current = null
  h.session.current += 1
  h.window.dispatchEvent(new Event('focus'))
  h.paint()
  assert.equal(h.marks.length, 0)
})

test('account reset and effect cleanup prevent late frame or focus read markers', async (t) => {
  const h = await makeMarker(t)
  h.resets.forEach((reset) => reset())
  h.paint()
  assert.equal(h.marks.length, 0)
  h.cleanup()
  h.window.dispatchEvent(new Event('focus'))
  h.document.dispatchEvent(new Event('visibilitychange'))
  h.el.dispatchEvent(new Event('scroll'))
  h.paint()
  assert.equal(h.marks.length, 0)
})
