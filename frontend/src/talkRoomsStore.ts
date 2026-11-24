// Один список бесед Talk на всю программу: его читают и фоновая проверка (счётчик, уведомления,
// звонки), и раздел «Чат». Одновременные запросы объединяются, свежий список не перечитывается —
// сервер Nextcloud получает один запрос вместо нескольких.
import { useSyncExternalStore } from 'react'

import { api, errorText, onSessionReset } from './api'
import type { TalkRoom } from './types'

let rooms: TalkRoom[] | null = null
// Уведомления сравнивают только серверные снимки: локальное unread=0 не означает новое событие.
let serverRooms: TalkRoom[] | null = null
let fetchedAt = 0
let inflight: Promise<TalkRoom[]> | null = null
// Поколение: после смены учётной записи ответ на прежний запрос не попадает в список.
let generation = 0
let readVersion = 0
const listeners = new Set<() => void>()

interface PendingRead {
  target: number
  sent: number
  running: Promise<void> | null
  timer: ReturnType<typeof setTimeout> | null
  failures: number
}
const pendingReads = new Map<string, PendingRead>()
const manualReads = new Map<string, { operation: Promise<void>; read: boolean; target: number }>()

function projectRead(room: TalkRoom): TalkRoom {
  const pending = pendingReads.get(room.token)
  // Без lastMessage.id нельзя доказать, что все сообщения этого снимка уже просмотрены.
  if (!pending || !(room.lastMessageId > 0) || room.lastMessageId > pending.target) return room
  return room.unread || room.unreadMention ? { ...room, unread: 0, unreadMention: false } : room
}

async function refreshAfterRead(gen: number) {
  // Не переиспользовать GET, начатый до записи отметки. Его результат может быть устаревшим.
  if (inflight) await inflight.catch(() => undefined)
  if (gen === generation) await refreshRooms().catch(() => undefined)
}

function sendRead(token: string, pending: PendingRead) {
  if (pending.running || pending.timer || pending.sent >= pending.target) return
  const gen = generation
  const current = () => gen === generation && pendingReads.get(token) === pending
  pending.running = (async () => {
    while (current() && pending.sent < pending.target) {
      const lastId = pending.target
      try {
        await api.talkManage(token, 'read', { lastId })
      } catch (e) {
        if (!current()) return
        if (pending.failures === 0) console.warn('Talk: не удалось сохранить прочтение:', errorText(e))
        const delay = Math.min(1000 * 2 ** Math.min(pending.failures++, 5), 30_000)
        pending.timer = setTimeout(() => {
          pending.timer = null
          if (current()) sendRead(token, pending)
        }, delay)
        return
      }
      if (gen !== generation) return
      readVersion += 1
      fetchedAt = 0
      if (!current()) return
      pending.sent = lastId
      pending.failures = 0
      // Только один запрос на беседу: старый повтор не может откатить более новую отметку.
    }
  })().finally(() => {
    pending.running = null
    if (current() && pending.sent >= pending.target) void refreshAfterRead(gen)
  })
}

/** Пользователь увидел сообщения до lastId. Досылаем эту отметку и после ухода из беседы. */
export function markRoomRead(token: string, lastId: number) {
  if (!Number.isSafeInteger(lastId) || lastId <= 0) return
  const manual = manualReads.get(token)
  if (manual) {
    // Новое сообщение могло отрисоваться, пока выполнялось ручное «прочитано».
    if (manual.read) manual.target = Math.max(manual.target, lastId)
    return
  }
  let pending = pendingReads.get(token)
  if (!pending) {
    const lastRead = serverRooms?.find((r) => r.token === token)?.lastRead ?? 0
    if (lastRead >= lastId) return
    pending = { target: lastId, sent: lastRead, running: null, timer: null, failures: 0 }
    pendingReads.set(token, pending)
  } else pending.target = Math.max(pending.target, lastId)
  if (rooms) {
    const projected = rooms.map(projectRead)
    if (projected.some((r, i) => r !== rooms![i])) { rooms = projected; emit() }
  }
  sendRead(token, pending)
}

/** Явное «прочитано/непрочитано» выполняется после уже отправленной автоматической отметки. */
export function setRoomRead(token: string, read: boolean): Promise<void> {
  const gen = generation
  const pending = pendingReads.get(token)
  if (pending?.timer) { clearTimeout(pending.timer); pending.timer = null }
  pendingReads.delete(token)
  const previousManual = manualReads.get(token)
  const previous = previousManual?.operation ?? pending?.running
  let restoreRead = false
  const operation = (async () => {
    await previous?.catch(() => undefined)
    if (gen !== generation) return
    try {
      // Явное действие пользователя — вся беседа, включая скрытый сервером lastMessage.
      await api.talkManage(token, read ? 'read' : 'unread', {})
    } catch (e) {
      restoreRead = true
      throw e
    }
    if (gen !== generation) return
    readVersion += 1
    fetchedAt = 0
    await refreshAfterRead(gen)
  })()
  const manual = { operation, read, target: read ? previousManual?.target ?? 0 : 0 }
  manualReads.set(token, manual)
  return operation.finally(() => {
    if (manualReads.get(token) !== manual) return
    manualReads.delete(token)
    if (restoreRead && pending && gen === generation) {
      pendingReads.set(token, pending)
      if (pending.sent < pending.target) sendRead(token, pending)
      else void refreshAfterRead(gen)
    }
    if (gen === generation && manual.target > 0) markRoomRead(token, manual.target)
  })
}

function emit() {
  listeners.forEach((l) => l())
}

/** Перечитать беседы с сервера; `maxAgeMs` — не перечитывать, если список свежее. */
export function refreshRooms(maxAgeMs = 0): Promise<TalkRoom[]> {
  if (rooms && maxAgeMs > 0 && Date.now() - fetchedAt < maxAgeMs) return Promise.resolve(rooms)
  if (inflight) return inflight
  const gen = generation
  const version = readVersion
  inflight = api.talkRooms().then((list) => {
    if (gen !== generation) return []
    if (version !== readVersion) return rooms ?? []
    serverRooms = list
    for (const room of list) {
      const pending = pendingReads.get(room.token)
      // Снимаем защиту только по снимку, запрошенному после успешной записи отметки.
      if (pending && pending.sent >= pending.target && room.lastRead >= pending.target) pendingReads.delete(room.token)
    }
    rooms = list.map(projectRead)
    fetchedAt = Date.now()
    emit()
    return rooms
  }).finally(() => { if (gen === generation) inflight = null })
  return inflight
}

/** Поправить список на месте (избранное, прочитанное) — до следующего ответа сервера. */
export function patchRooms(fn: (list: TalkRoom[] | null) => TalkRoom[] | null) {
  rooms = fn(rooms)
  emit()
}

/** Забыть список (выход, другая учётная запись, Nextcloud отключён). */
export function resetRooms() {
  generation += 1
  rooms = null
  serverRooms = null
  fetchedAt = 0
  inflight = null
  for (const pending of pendingReads.values()) if (pending.timer) clearTimeout(pending.timer)
  pendingReads.clear()
  manualReads.clear()
  emit()
}

onSessionReset(resetRooms)

export function currentRooms(): TalkRoom[] | null {
  return rooms
}

export function currentServerRooms(): TalkRoom[] | null {
  return serverRooms
}

/** Сколько миллисекунд назад список пришёл с сервера (Infinity — ещё не приходил). */
export function roomsAge(): number {
  return fetchedAt ? Date.now() - fetchedAt : Infinity
}

export function subscribeRooms(l: () => void): () => void {
  listeners.add(l)
  return () => { listeners.delete(l) }
}

export function useTalkRooms(): TalkRoom[] | null {
  return useSyncExternalStore(subscribeRooms, currentRooms)
}
