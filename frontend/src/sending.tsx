import { useEffect, useRef, useState } from 'react'
import { Send, Undo2 } from 'lucide-react'

import { api, errorText } from './api'
import type { Draft } from './types'

const PREFS_KEY = 'rrcki-mail.send-prefs'

export interface SendPrefs {
  /** «Окно отмены» после «Отправить», секунд (0 — отправлять сразу). */
  undoSecs: number
  /** Запрашивать уведомление о прочтении по умолчанию. */
  receiptDefault: boolean
  /** Ответ на запросы уведомлений о прочтении. */
  mdnPolicy: 'ask' | 'never'
}

const DEFAULTS: SendPrefs = { undoSecs: 10, receiptDefault: false, mdnPolicy: 'ask' }

export function loadSendPrefs(): SendPrefs {
  try {
    return { ...DEFAULTS, ...JSON.parse(localStorage.getItem(PREFS_KEY) ?? '{}') as Partial<SendPrefs> }
  } catch { return DEFAULTS }
}

export function saveSendPrefs(p: SendPrefs) {
  try { localStorage.setItem(PREFS_KEY, JSON.stringify(p)) } catch { /* нет хранилища */ }
}

interface Queued { id: number; snapshot: Draft; deadline: number }

/** Письма в «окне отмены»: плашка с обратным отсчётом, «Отменить» и «Отправить сейчас».
 *  Отменённое (или не отправленное из-за ошибки) письмо возвращается в редактор. */
export function useOutbox(onReopen: (draft: Draft) => void, onNotice: (text: string) => void) {
  const [queue, setQueue] = useState<Queued[]>([])
  const [, tick] = useState(0)
  const cb = useRef({ onReopen, onNotice })
  cb.current = { onReopen, onNotice }

  useEffect(() => {
    if (queue.length === 0) return
    const t = window.setInterval(() => tick((n) => n + 1), 500)
    return () => window.clearInterval(t)
  }, [queue.length])

  function queued(id: number, snapshot: Draft, delaySecs: number) {
    setQueue((q) => [...q, { id, snapshot, deadline: Date.now() + delaySecs * 1000 }])
    api.sendWait(id)
      .then((r) => cb.current.onNotice(r.warning ?? 'Письмо отправлено.'))
      .catch((e) => {
        const text = errorText(e)
        if (text.includes('отменена')) {
          cb.current.onNotice('Отправка отменена — письмо снова в редакторе.')
        } else {
          cb.current.onNotice(`Письмо не отправлено: ${text}`)
        }
        cb.current.onReopen(snapshot)
      })
      .finally(() => setQueue((q) => q.filter((x) => x.id !== id)))
  }

  async function cancel(id: number) {
    const ok = await api.sendCancel(id).catch(() => false)
    if (!ok) cb.current.onNotice('Письмо уже отправляется — отменить нельзя.')
  }

  const element = queue.length > 0 && <div className="outbox-toasts">
    {queue.map((q) => {
      const left = Math.max(0, Math.ceil((q.deadline - Date.now()) / 1000))
      return <div key={q.id} className="outbox-toast" role="status" aria-label="Отправка письма">
        <Send size={15} />
        <span>{left > 0 ? `Письмо «${q.snapshot.subject || '(без темы)'}» уйдёт через ${left} с` : 'Отправка…'}</span>
        <button className="text-button" disabled={left === 0} onClick={() => void cancel(q.id)}><Undo2 size={14} /> Отменить</button>
        {left > 0 && <button className="text-button" onClick={() => void api.sendNow(q.id)}>Отправить сейчас</button>}
      </div>
    })}
  </div>

  return { queued, element }
}

/** Варианты «Отправить позже». */
export function schedulePresets(now = new Date()): { label: string; at: Date }[] {
  const at = (days: number, hour: number) => { const d = new Date(now); d.setDate(d.getDate() + days); d.setHours(hour, 0, 0, 0); return d }
  const out: { label: string; at: Date }[] = []
  if (now.getHours() < 16) out.push({ label: 'Сегодня в 17:00', at: at(0, 17) })
  out.push({ label: 'Завтра в 9:00', at: at(1, 9) })
  out.push({ label: 'Завтра в 14:00', at: at(1, 14) })
  const toMonday = ((8 - now.getDay()) % 7) || 7
  out.push({ label: 'В понедельник в 9:00', at: at(toMonday, 9) })
  return out
}

/** Дата для поля datetime-local. */
export function localInputValue(d: Date): string {
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`
}

export function formatWhen(iso: string): string {
  const d = new Date(iso)
  return d.toLocaleString('ru-RU', { weekday: 'short', day: 'numeric', month: 'long', hour: '2-digit', minute: '2-digit' })
}
