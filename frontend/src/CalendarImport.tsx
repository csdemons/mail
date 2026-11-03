import { useEffect, useState } from 'react'
import { CalendarPlus, Loader2, MapPin, Repeat, X } from 'lucide-react'

import { api, errorText } from './api'
import type { AttachmentInfo, CalendarFile, IcsEvent } from './types'

/** Вложение — файл календаря (.ics/.vcs, text/calendar). */
export function isCalendarFile(a: Pick<AttachmentInfo, 'name' | 'contentType'>): boolean {
  const t = a.contentType.toLowerCase()
  return t === 'text/calendar' || t === 'application/ics' || /\.(ics|vcs|ical|ifb)$/i.test(a.name)
}

const CAL_KEY = 'rrcki-mail.import-calendar'

function when(e: IcsEvent): string {
  if (e.allDay) {
    const s = new Date(`${e.start}T12:00:00`)
    const last = new Date(`${e.end}T12:00:00`)
    last.setDate(last.getDate() - 1)
    const day = (d: Date) => d.toLocaleDateString('ru-RU', { weekday: 'short', day: 'numeric', month: 'long', year: 'numeric' })
    return last > s ? `${day(s)} — ${day(last)}, весь день` : `${day(s)}, весь день`
  }
  const s = new Date(e.start)
  const f = new Date(e.end)
  const date = s.toLocaleDateString('ru-RU', { weekday: 'short', day: 'numeric', month: 'long', year: 'numeric' })
  const t = (d: Date) => d.toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' })
  return `${date}, ${t(s)}–${t(f)}`
}

/** «Добавить в календарь» для файла календаря из письма: события файла, выбор
 *  календаря и каких событий; уже добавленные (тот же UID) обновляются. */
export default function CalendarImportDialog({ folder, uid, attachment, onClose, onNotice }: {
  folder: string
  uid: number
  attachment: AttachmentInfo
  onClose: () => void
  onNotice: (text: string) => void
}) {
  const [file, setFile] = useState<CalendarFile | null>(null)
  const [error, setError] = useState('')
  const [calendar, setCalendar] = useState('')
  const [chosen, setChosen] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    let alive = true
    api.calendarFilePreview(folder, uid, attachment.index)
      .then((f) => {
        if (!alive) return
        setFile(f)
        setChosen(new Set(f.events.map((e) => e.uid)))
        let last = ''
        try { last = localStorage.getItem(CAL_KEY) ?? '' } catch { /* нет хранилища */ }
        setCalendar(f.calendars.find((c) => c.url === last)?.url ?? f.calendars[0]?.url ?? '')
      })
      .catch((e) => { if (alive) setError(errorText(e)) })
    return () => { alive = false }
  }, [folder, uid, attachment.index])

  async function add() {
    if (!file || !calendar || chosen.size === 0) return
    setBusy(true)
    try {
      const r = await api.calendarFileImport(folder, uid, attachment.index, calendar, [...chosen])
      try { localStorage.setItem(CAL_KEY, calendar) } catch { /* нет хранилища */ }
      const name = file.calendars.find((c) => c.url === calendar)?.name ?? 'календарь'
      const parts = [r.added > 0 ? `добавлено: ${r.added}` : '', r.updated > 0 ? `обновлено: ${r.updated}` : ''].filter(Boolean).join(', ')
      onNotice(`Календарь «${name}» — ${parts}.${r.errors.length ? ` Не удалось: ${r.errors.join('; ')}` : ''}`)
      onClose()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  const toggle = (u: string) => setChosen((s) => { const n = new Set(s); if (n.has(u)) n.delete(u); else n.add(u); return n })

  return <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}>
    <div className="settings-modal ics-modal" role="dialog" aria-label="Добавить в календарь">
      <div className="compose-header">
        <div><strong><CalendarPlus size={16} /> Добавить в календарь</strong><small>{attachment.name}</small></div>
        <button className="icon-button" aria-label="Закрыть" onClick={onClose}><X size={18} /></button>
      </div>
      {!file && !error && <div className="empty-state"><Loader2 className="spin" size={18} /> Читаю файл…</div>}
      {error && <div className="inline-warning">{error}</div>}
      {file && <>
        {file.calendars.length === 0
          ? <div className="inline-warning">Нет календаря, в который можно добавлять события.</div>
          : <label className="ics-calendar">Календарь
            <select value={calendar} onChange={(e) => setCalendar(e.target.value)} aria-label="Календарь">
              {file.calendars.map((c) => <option key={c.url} value={c.url}>{c.name}</option>)}
            </select>
          </label>}
        {file.events.length > 1 && <div className="ics-all">
          <button className="text-button" onClick={() => setChosen(new Set(file.events.map((e) => e.uid)))}>Выбрать все</button>
          <button className="text-button" onClick={() => setChosen(new Set())}>Снять все</button>
          <span className="muted">Выбрано {chosen.size} из {file.events.length}</span>
        </div>}
        <ul className="ics-events">
          {file.events.map((e) => <li key={e.uid}>
            <label>
              <input type="checkbox" checked={chosen.has(e.uid)} onChange={() => toggle(e.uid)} aria-label={e.title} />
              <span className="ics-event">
                <b>{e.title}</b>
                <small>{when(e)}{e.recurring && <> · <Repeat size={11} /> повторяется</>}</small>
                {e.location && <small className="muted"><MapPin size={11} /> {e.location}</small>}
              </span>
            </label>
          </li>)}
        </ul>
        <p className="muted ics-help">Если событие из этого файла уже добавлено (например, пришло обновлённое расписание), оно заменится новой версией, а не задвоится.</p>
        <div className="ics-buttons">
          <button className="primary-button" disabled={busy || !calendar || chosen.size === 0} onClick={() => void add()}>
            {busy ? <Loader2 className="spin" size={15} /> : <CalendarPlus size={15} />} Добавить{chosen.size > 1 ? ` (${chosen.size})` : ''}
          </button>
          <button className="secondary-button" onClick={onClose}>Отмена</button>
        </div>
      </>}
    </div>
  </div>
}
