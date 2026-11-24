// Регрессионные проверки цикла фоновой проверки: одна проверка одновременно, один владелец
// таймера и не больше одного отложенного повтора. Как и test-talk-read.mjs, обходится
// компилятором TypeScript проекта и раннером Node — браузер, сервер и лишние пакеты не нужны.
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { setImmediate } from 'node:timers/promises'
import test from 'node:test'
import ts from 'typescript'

const flush = () => setImmediate()

function deferred() {
  let resolvePromise
  const request = {
    settled: false,
    promise: new Promise((resolve) => { resolvePromise = resolve }),
    resolve(value) { assert.equal(request.settled, false); request.settled = true; resolvePromise(value) },
  }
  return request
}

function fakeClock() {
  let now = 1_000_000
  let nextTimer = 0
  const timers = new Map()
  return {
    get now() { return now },
    get pending() { return timers.size },
    setTimeout(callback, ms = 0) {
      const id = ++nextTimer
      timers.set(id, { at: now + ms, run: callback })
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
      assert.fail('цикл таймеров не успокоился')
    },
  }
}

const source = await readFile(new URL('../src/pollLoop.ts', import.meta.url), 'utf8')
const code = ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  fileName: 'pollLoop.ts',
}).outputText
const { createPollLoop, DEFAULT_POLL_DELAY_MS, WAKE_BURST_MS } =
  await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`)

/** Цикл с управляемыми запросами: каждая проверка ждёт, пока тест не ответит. */
function harness(t, delay = 30_000) {
  const clock = fakeClock()
  const requests = []
  const loop = createPollLoop(() => {
    const request = deferred()
    requests.push(request)
    return request.promise
  }, clock, () => clock.now)
  t.after(() => loop.stop())
  const respond = async () => {
    const request = requests.find((r) => !r.settled)
    assert.ok(request, 'ожидался запрос проверки')
    request.resolve(delay)
    await flush()
  }
  return { clock, requests, loop, respond }
}

test('возврат фокуса во время запроса не создаёт второй цикл опроса', async (t) => {
  const h = harness(t)
  h.loop.wake()
  await flush()
  assert.equal(h.requests.length, 1, 'первая проверка началась')
  assert.equal(h.clock.pending, 0, 'пока запрос идёт, таймер не нужен')

  // Запрос всё ещё ждёт сеть, окно вернулось в фокус (focus + visibilitychange).
  await h.clock.advance(WAKE_BURST_MS + 100)
  h.loop.wake()
  h.loop.wake()
  assert.equal(h.requests.length, 1, 'второй запрос не идёт одновременно с первым')

  await h.respond()
  assert.equal(h.requests.length, 2, 'после завершения — ровно один повтор')
  assert.equal(h.clock.pending, 0, 'оба события фокуса дали один повтор, а не таймер')

  await h.respond()
  assert.equal(h.clock.pending, 1, 'дальше работает ровно один таймер')

  // Каждый интервал — ровно одна проверка: цикл не размножается.
  await h.clock.advance(30_000)
  assert.equal(h.requests.length, 3)
  await h.respond()
  await h.clock.advance(30_000)
  assert.equal(h.requests.length, 4)
  await h.respond()
  assert.equal(h.clock.pending, 1)
})

test('focus и visibilitychange одной парой дают один запрос', async (t) => {
  const h = harness(t)
  h.loop.wake()
  h.loop.wake()
  await flush()
  assert.equal(h.requests.length, 1, 'оба события объединены')
})

test('проверка, начатая давно, всё равно повторяется после завершения', async (t) => {
  const h = harness(t)
  h.loop.wake()
  await flush()
  await h.clock.advance(5_000)
  h.loop.wake()
  h.loop.wake()
  await h.respond()
  assert.equal(h.requests.length, 2, 'устаревший запрос вызвал один повтор')
  await h.respond()
  assert.equal(h.clock.pending, 1, 'и снова один таймер')
})

test('остановка цикла не оставляет работающих таймеров', async (t) => {
  const h = harness(t)
  h.loop.wake()
  await flush()
  h.loop.stop()
  assert.equal(h.clock.pending, 0, 'таймер снят')
  await h.respond()
  await h.clock.advance(120_000)
  assert.equal(h.requests.length, 1, 'после остановки проверок больше нет')
})

test('ошибка проверки не останавливает цикл', async (t) => {
  const clock = fakeClock()
  let calls = 0
  const loop = createPollLoop(async () => {
    calls++
    if (calls === 1) throw new Error('сеть недоступна')
    return 30_000
  }, clock, () => clock.now)
  t.after(() => loop.stop())

  loop.wake()
  await flush()
  assert.equal(calls, 1)
  assert.equal(clock.pending, 1, 'после ошибки таймер всё равно запланирован')
  await clock.advance(DEFAULT_POLL_DELAY_MS)
  assert.equal(calls, 2)
})

test('нулевая пауза заменяется обычной, иначе проверки шли бы подряд', async (t) => {
  const clock = fakeClock()
  let calls = 0
  const loop = createPollLoop(async () => { calls++; return 0 }, clock, () => clock.now)
  t.after(() => loop.stop())

  loop.wake()
  await flush()
  assert.equal(calls, 1)
  await clock.advance(1)
  assert.equal(calls, 1, 'нулевая пауза не запускает проверку сразу же')
  await clock.advance(DEFAULT_POLL_DELAY_MS)
  assert.equal(calls, 2)
})
