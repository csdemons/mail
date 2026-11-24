import { useLayoutEffect, useRef } from 'react'
import type { MutableRefObject, RefObject } from 'react'

import { onSessionReset } from './api'
import { markRoomRead } from './talkRoomsStore'

export interface TalkReadCursor { token: string; session: number; lastId: number }

// Старое окно не должно возобновить отметки после смены учётной записи.
let accountGeneration = 0
onSessionReset(() => { accountGeneration += 1 })

/** Отметка только для уже отрисованного конца беседы в активном окне. */
export function useTalkReadMarker(
  token: string | null,
  cursor: TalkReadCursor | null,
  ready: boolean,
  listRef: RefObject<HTMLDivElement>,
  pollSession: MutableRefObject<number>,
  stickToBottom: MutableRefObject<boolean>,
  suspended: MutableRefObject<string | null>,
) {
  const account = useRef(accountGeneration)
  const submitted = useRef<{ session: number; lastId: number }>({ session: 0, lastId: 0 })

  useLayoutEffect(() => {
    const el = listRef.current
    if (!token || !cursor || cursor.token !== token || !ready || !el) return
    let frame = 0
    const markVisible = () => {
      frame = 0
      if (account.current !== accountGeneration || cursor.session !== pollSession.current) return
      if (suspended.current === token || document.hidden || !document.hasFocus()) return
      if (!stickToBottom.current || el.clientHeight <= 0 || el.scrollHeight - el.scrollTop - el.clientHeight > 2) return
      if (cursor.lastId <= 0 || (submitted.current.session === cursor.session && submitted.current.lastId >= cursor.lastId)) return
      submitted.current = { session: cursor.session, lastId: cursor.lastId }
      markRoomRead(token, cursor.lastId)
    }
    const schedule = () => {
      if (frame) cancelAnimationFrame(frame)
      frame = requestAnimationFrame(markVisible)
    }
    // В том числе возврат к уже загруженным сообщениям, без нового ответа long-poll.
    schedule()
    el.addEventListener('scroll', schedule, { passive: true })
    window.addEventListener('focus', schedule)
    document.addEventListener('visibilitychange', schedule)
    return () => {
      if (frame) cancelAnimationFrame(frame)
      el.removeEventListener('scroll', schedule)
      window.removeEventListener('focus', schedule)
      document.removeEventListener('visibilitychange', schedule)
    }
  }, [token, cursor, ready, listRef, pollSession, stickToBottom, suspended])
}
