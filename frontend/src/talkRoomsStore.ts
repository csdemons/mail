// Один список бесед Talk на всю программу: его читают и фоновая проверка (счётчик, уведомления,
// звонки), и раздел «Чат». Одновременные запросы объединяются, свежий список не перечитывается —
// сервер Nextcloud получает один запрос вместо нескольких.
import { useSyncExternalStore } from 'react'

import { api } from './api'
import type { TalkRoom } from './types'

let rooms: TalkRoom[] | null = null
let fetchedAt = 0
let inflight: Promise<TalkRoom[]> | null = null
// Поколение: после смены учётной записи ответ на прежний запрос не попадает в список.
let generation = 0
const listeners = new Set<() => void>()

// Ожидающие подтверждения сервером отметки прочтения.
// Когда пользователь открыл беседу, мы локально ставим unread=0 и просим сервер отметить
// прочитанным. Сервер может ответить устаревшим списком (ещё не обработал отметку) — пока
// lastActivity не вырос, мы не даём этому ответу вернуть счётчик.
// `until` — когда отметка должна устареть сама (мс от эпохи), чтобы не висела вечно.
const pendingReads = new Map<string, { lastActivity: number; until: number }>()
const PENDING_READ_TTL_MS = 60_000

function emit() {
  listeners.forEach((l) => l())
}

/** Перечитать беседы с сервера; `maxAgeMs` — не перечитывать, если список свежее. */
export function refreshRooms(maxAgeMs = 0): Promise<TalkRoom[]> {
  if (rooms && maxAgeMs > 0 && Date.now() - fetchedAt < maxAgeMs) return Promise.resolve(rooms)
  if (inflight) return inflight
  const gen = generation
  inflight = api.talkRooms().then((list) => {
    if (gen === generation) {
      const now = Date.now()
      // Применить ожидающие отметки: если сервер ещё не «увидел» прочитанное и не принёс
      // новых сообщений с той же lastActivity — счётчик unread остаётся нулевым локально.
      if (pendingReads.size > 0) {
        list = list.map((r) => {
          const p = pendingReads.get(r.token)
          if (!p || now > p.until) return r
          if (r.lastActivity <= p.lastActivity) return { ...r, unread: 0 }
          return r
        })
        // Чистим истёкшие.
        for (const [tok, p] of pendingReads) if (now > p.until) pendingReads.delete(tok)
      }
      rooms = list
      fetchedAt = Date.now()
      emit()
    }
    return list
  }).finally(() => { if (gen === generation) inflight = null })
  return inflight
}

/** Поправить список на месте (избранное, прочитанное) — до следующего ответа сервера. */
export function patchRooms(fn: (list: TalkRoom[] | null) => TalkRoom[] | null) {
  rooms = fn(rooms)
  emit()
}

/**
 * Записать, что беседа `token` прочитана пользователем до сообщения с `lastActivity`.
 * Пока отметка не подтверждена сервером (или не истёк TTL), фоновые опросы не возвращают
 * устаревший счётчик unread для этой беседы — если lastActivity не вырос.
 */
export function markRead(token: string, lastActivity: number) {
  pendingReads.set(token, { lastActivity, until: Date.now() + PENDING_READ_TTL_MS })
}

/** Снять ожидающую отметку прочтения (например, при уходе из открытой беседы без подтверждения). */
export function clearPendingRead(token: string) {
  pendingReads.delete(token)
}

/** Забыть список (выход, другая учётная запись, Nextcloud отключён). */
export function resetRooms() {
  generation += 1
  rooms = null
  fetchedAt = 0
  inflight = null
  pendingReads.clear()
  emit()
}

export function currentRooms(): TalkRoom[] | null {
  return rooms
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
