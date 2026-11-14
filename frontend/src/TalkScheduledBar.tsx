// Отложенные сообщения беседы (Talk 23): полоска над полем ввода, список, перенос и отмена.
import { useEffect, useState } from 'react'
import { ChevronDown, ChevronRight, Clock, Trash2 } from 'lucide-react'

import { api, errorText } from './api'
import DuePicker from './DuePicker'
import type { TalkScheduled } from './types'

const when = (ts: number) => new Date(ts * 1000).toLocaleString('ru-RU', { weekday: 'short', day: 'numeric', month: 'long', hour: '2-digit', minute: '2-digit' })

export default function TalkScheduledBar({ token, version, notify }: {
  token: string
  /** Растёт после каждого нового отложенного сообщения — список перечитывается. */
  version: number
  notify: (text: string) => void
}) {
  const [items, setItems] = useState<TalkScheduled[]>([])
  const [open, setOpen] = useState(false)
  const [moving, setMoving] = useState<TalkScheduled | null>(null)

  useEffect(() => {
    let alive = true
    setItems([])
    api.talkScheduled(token).then((l) => { if (alive) setItems(l) }).catch(() => { if (alive) setItems([]) })
    // Ушедшие по времени — убрать из списка.
    const t = window.setInterval(() => setItems((l) => l.filter((m) => m.sendAt > Date.now() / 1000)), 30_000)
    return () => { alive = false; window.clearInterval(t) }
  }, [token, version])

  async function cancel(m: TalkScheduled) {
    try {
      await api.talkScheduleDelete(token, m.id)
      setItems((l) => l.filter((x) => x.id !== m.id))
      notify('Отложенное сообщение отменено')
    } catch (e) { notify(`Не удалось: ${errorText(e)}`) }
  }

  async function move(m: TalkScheduled, at: Date) {
    try {
      const next = await api.talkScheduleEdit(token, m.id, m.text, Math.floor(at.getTime() / 1000), m.silent)
      setItems((l) => l.map((x) => (x.id === m.id ? next : x)).sort((a, b) => a.sendAt - b.sendAt))
      notify(`Сообщение уйдёт ${when(next.sendAt)}`)
    } catch (e) { notify(`Не удалось перенести: ${errorText(e)}`) }
  }

  if (items.length === 0) return null
  return (
    <div className="talk-scheduled" aria-label="Отложенные сообщения">
      <button type="button" className="talk-scheduled-head" aria-expanded={open} onClick={() => setOpen(!open)}>
        {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}<Clock size={14} /> Отложено: {items.length}
        {!open && <span className="muted"> · ближайшее {when(items[0].sendAt)}</span>}
      </button>
      {open && items.map((m) => (
        <div key={m.id} className="talk-scheduled-item">
          <span className="talk-scheduled-when">{when(m.sendAt)}{m.silent ? ' · без звука' : ''}{m.threadTitle ? ` · ветка «${m.threadTitle}»` : ''}</span>
          <span className="talk-scheduled-text">{m.parentAuthor ? `↩ ${m.parentAuthor}: ` : ''}{m.text}</span>
          <button type="button" className="text-button" onClick={() => setMoving(m)}>Перенести</button>
          <button type="button" className="icon-button soft" aria-label="Отменить отложенное сообщение" title="Отменить" onClick={() => void cancel(m)}><Trash2 size={14} /></button>
        </div>
      ))}
      {moving && <DuePicker title="Перенести сообщение" hint="Сообщение уйдёт в новое время" action="Перенести" onClose={() => setMoving(null)} onPick={(d) => { const m = moving; setMoving(null); void move(m, d) }} />}
    </div>
  )
}
