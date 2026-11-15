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

/** Забыть список (выход, другая учётная запись, Nextcloud отключён). */
export function resetRooms() {
  generation += 1
  rooms = null
  fetchedAt = 0
  inflight = null
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
