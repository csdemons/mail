// Фоновые задачи главного окна: напоминания по флажкам, счётчик и уведомления чата, входящие звонки, масштаб.
import { useEffect, useRef, useState, type MutableRefObject } from 'react'

import { api } from './api'
import { dueOf } from './mailExtras'
import { senderLabel } from './mailLabels'
import { appInFront, notify, type NotifyPrefs } from './notifications'
import type { ModuleName } from './viewContext'
import type { TalkRoom } from './types'
import { zoomStep } from './appearance'

/** Напоминания по флажкам со сроком: проверка раз в 2 минуты (и при запуске). */
export function useDueReminders(setNotice: (text: string) => void, pollSecs: MutableRefObject<number>) {
  useEffect(() => {
    let stopped = false
    const KEY = 'rrcki-mail.reminded'
    async function check() {
      try {
        const list = await api.dueList()
        if (stopped) return
        let done: string[] = []
        try { done = JSON.parse(localStorage.getItem(KEY) ?? '[]') as string[] } catch { /* нет хранилища */ }
        const now = Date.now()
        for (const m of list) {
          const due = dueOf(m)
          const id = `${m.msgId ?? `${m.folder}/${m.uid}`}|${due?.getTime()}`
          if (!due || due.getTime() > now || done.includes(id)) continue
          done.push(id)
          const text = `${senderLabel(m)}: ${m.subject || '(без темы)'}`
          if (appInFront()) setNotice(`Напоминание по флажку — ${text}`)
          else notify('Напоминание по флажку', text)
        }
        try { localStorage.setItem(KEY, JSON.stringify(done.slice(-500))) } catch { /* нет хранилища */ }
      } catch { /* попробуем позже */ }
    }
    let timer = 0
    const loop = () => { timer = window.setTimeout(() => { void check().finally(() => { if (!stopped) loop() }) }, timer === 0 ? 5000 : pollSecs.current < 60 ? 10_000 : 120_000) }
    loop()
    return () => { stopped = true; window.clearTimeout(timer) }
  }, [])
}

/** Звонок, о котором сообщить: беседа и когда он начался. */
export interface Ringing { token: string; name: string; since: number }

/** Сколько звонит входящий звонок, если на него не ответили (с). */
const RING_SECS = 60

/** Новые входящие звонки: звонок идёт, я в нём не участвую, уведомления о звонках беседы
 *  включены и этот звонок ещё не отклонён. При запуске — только начавшиеся недавно. */
export function newCalls(rooms: TalkRoom[], known: Set<string>, startup: boolean, now = Date.now() / 1000): TalkRoom[] {
  return rooms.filter((r) => r.hasCall && !r.inCall && r.notifyCalls !== 0 && !r.archived
    && !known.has(`${r.token}:${r.callStart}`) && (!startup || now - r.callStart < RING_SECS))
}

/** Сообщать ли о новом сообщении беседы — как в Talk: «никогда» молчит, «при упоминании» —
 *  только с упоминанием (личные беседы — всегда); «важные» беседы — всегда. */
export function chatNotifies(r: TalkRoom): boolean {
  if (r.important) return true
  if (r.notifyLevel === 3) return false
  if (r.notifyLevel === 2) return r.unreadMention || r.kind === 1
  return true
}

/** Чат: счётчик непрочитанного, уведомления о новых сообщениях и входящие звонки (в любом разделе). */
export function useChatNotifications(
  connected: boolean,
  moduleRef: MutableRefObject<ModuleName>,
  prefsRef: MutableRefObject<NotifyPrefs>,
  pollSecs: MutableRefObject<number>,
  setChatUnread: (n: number) => void,
  setNotice: (text: string) => void,
) {
  const roomSeen = useRef(new Map<string, { unread: number; activity: number }>())
  // Звонки, о которых уже сообщили (беседа:начало) — второй раз не звоним.
  const callsSeen = useRef(new Set<string>())
  const [ringing, setRinging] = useState<Ringing | null>(null)
  const ringRef = useRef<Ringing | null>(null)
  ringRef.current = ringing
  useEffect(() => {
    if (!connected) return
    let stopped = false
    let timer: ReturnType<typeof setTimeout> | undefined
    async function tick() {
      try {
        const rooms = await api.talkRooms()
        if (stopped) return
        if (moduleRef.current !== 'chat') setChatUnread(rooms.reduce((n, x) => n + x.unread, 0))
        const first = roomSeen.current.size === 0
        for (const r of rooms) {
          const prev = roomSeen.current.get(r.token)
          roomSeen.current.set(r.token, { unread: r.unread, activity: r.lastActivity })
          if (first || !prev || r.unread <= prev.unread) continue
          if (!prefsRef.current.chat || !chatNotifies(r)) continue
          // «Конфиденциальная» беседа: текст сообщения в уведомлении не показывается.
          const text = prefsRef.current.preview && !r.sensitive ? (r.lastMessage ?? 'Новое сообщение') : 'Новое сообщение'
          if (!appInFront()) notify(`Чат: ${r.name}`, text)
          else if (moduleRef.current !== 'chat') setNotice(`Чат «${r.name}»: ${text}`)
        }
        // Входящий звонок: показать (и сообщить системе); закончился или ответили — убрать.
        const cur = ringRef.current
        if (cur && (Date.now() / 1000 - cur.since > RING_SECS || !rooms.some((r) => r.token === cur.token && r.hasCall && !r.inCall))) setRinging(null)
        const fresh = newCalls(rooms, callsSeen.current, first)
        rooms.filter((r) => r.hasCall).forEach((r) => callsSeen.current.add(`${r.token}:${r.callStart}`))
        const call = fresh.sort((a, b) => b.callStart - a.callStart)[0]
        if (call) {
          setRinging({ token: call.token, name: call.name, since: Date.now() / 1000 })
          if (prefsRef.current.chat && !appInFront()) notify(`Звонок: ${call.name}`, 'Входящий звонок в Talk — откройте программу, чтобы ответить')
        }
      } catch { /* сеть — попробуем в следующий раз */ }
      finally {
        // Чаще, чем почту: о звонке нужно узнать сразу.
        if (!stopped) timer = setTimeout(tick, Math.min(pollSecs.current, 10) * 1000)
      }
    }
    void tick()
    return () => { stopped = true; if (timer) clearTimeout(timer) }
  }, [connected])
  return { ringing, dismissCall: () => setRinging(null) }
}

/** Масштаб интерфейса: Ctrl + / Ctrl − / Ctrl 0 (и Ctrl+колесо мыши) — в любом разделе. */
export function useZoomShortcuts() {
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (!(e.ctrlKey || e.metaKey) || e.altKey) return
      if (e.key === '+' || e.key === '=') { e.preventDefault(); zoomStep(1) }
      else if (e.key === '-' || e.key === '_') { e.preventDefault(); zoomStep(-1) }
      else if (e.key === '0') { e.preventDefault(); zoomStep(0) }
    }
    function onWheel(e: WheelEvent) {
      if (!(e.ctrlKey || e.metaKey) || e.deltaY === 0) return
      e.preventDefault()
      zoomStep(e.deltaY < 0 ? 1 : -1)
    }
    window.addEventListener('keydown', onKey)
    window.addEventListener('wheel', onWheel, { passive: false })
    return () => { window.removeEventListener('keydown', onKey); window.removeEventListener('wheel', onWheel) }
  }, [])
}
