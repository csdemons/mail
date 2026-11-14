// Фоновые задачи главного окна: напоминания по флажкам, счётчик и уведомления чата, масштаб.
import { useEffect, useRef, type MutableRefObject } from 'react'

import { api } from './api'
import { dueOf } from './mailExtras'
import { senderLabel } from './mailLabels'
import { appInFront, notify, type NotifyPrefs } from './notifications'
import type { ModuleName } from './viewContext'
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

/** Чат: счётчик непрочитанного и уведомления о новых сообщениях (в любом разделе). */
export function useChatNotifications(
  connected: boolean,
  moduleRef: MutableRefObject<ModuleName>,
  prefsRef: MutableRefObject<NotifyPrefs>,
  pollSecs: MutableRefObject<number>,
  setChatUnread: (n: number) => void,
  setNotice: (text: string) => void,
) {
  const roomSeen = useRef(new Map<string, { unread: number; activity: number }>())
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
          if (!prefsRef.current.chat) continue
          const text = prefsRef.current.preview ? (r.lastMessage ?? 'Новое сообщение') : 'Новое сообщение'
          if (!appInFront()) notify(`Чат: ${r.name}`, text)
          else if (moduleRef.current !== 'chat') setNotice(`Чат «${r.name}»: ${text}`)
        }
      } catch { /* сеть — попробуем в следующий раз */ }
      finally {
        if (!stopped) timer = setTimeout(tick, Math.min(pollSecs.current, 30) * 1000)
      }
    }
    void tick()
    return () => { stopped = true; if (timer) clearTimeout(timer) }
  }, [connected])
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
