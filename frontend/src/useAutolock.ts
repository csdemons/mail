import { useEffect, useRef, useState } from 'react'

import { api } from './api'
import { AUTOLOCK_KEY, loadAutolock } from './TotpSettings'

/** Защита кодом: блокировка по Ctrl+L, из меню и при бездействии.
 *  `composingRef` — открыт редактор письма: автоблокировка ждёт, пока его закроют. */
export function useAutolock(onLock: () => void) {
  const [totpOn, setTotpOn] = useState(false)
  const [autolock, setAutolock] = useState(loadAutolock)
  useEffect(() => { api.totpStatus().then((s) => setTotpOn(s.enabled)).catch(() => undefined) }, [])
  const lockRef = useRef(onLock)
  lockRef.current = onLock
  const composingRef = useRef(false)
  useEffect(() => {
    if (!totpOn) return
    function onKey(event: KeyboardEvent) {
      if ((event.ctrlKey || event.metaKey) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'l') {
        event.preventDefault()
        lockRef.current()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [totpOn])
  useEffect(() => {
    if (!totpOn || autolock <= 0) return
    let last = Date.now()
    const touch = () => { last = Date.now() }
    const events = ['mousemove', 'mousedown', 'keydown', 'wheel', 'touchstart'] as const
    events.forEach((e) => window.addEventListener(e, touch, { passive: true, capture: true }))
    const timer = window.setInterval(() => {
      // Незаконченное письмо не теряем: пока открыт редактор, автоблокировка ждёт.
      if (Date.now() - last >= autolock * 60_000 && !composingRef.current) lockRef.current()
    }, 10_000)
    return () => {
      window.clearInterval(timer)
      events.forEach((e) => window.removeEventListener(e, touch, { capture: true }))
    }
  }, [totpOn, autolock])
  function updateAutolock(minutes: number) {
    setAutolock(minutes)
    try { localStorage.setItem(AUTOLOCK_KEY, String(minutes)) } catch { /* нет хранилища */ }
  }
  return { totpOn, setTotpOn, autolock, updateAutolock, composingRef }
}
