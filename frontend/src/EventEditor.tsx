import { useState } from 'react'
import { X } from 'lucide-react'

import { api, errorText } from './api'
import type { CalEvent, DavCollection, EventInput } from './types'

function pad(n: number) {
  return String(n).padStart(2, '0')
}
function dateInput(d: Date) {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}
function timeInput(d: Date) {
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`
}
function addDaysStr(ymd: string, n: number) {
  const d = new Date(`${ymd}T00:00:00`)
  d.setDate(d.getDate() + n)
  return dateInput(d)
}

export type EditorTarget =
  | { kind: 'new'; start: Date; allDay?: boolean }
  | { kind: 'edit'; event: CalEvent }

interface Props {
  target: EditorTarget
  calendars: DavCollection[]
  onClose: () => void
  onSaved: (notice: string) => void
}

export default function EventEditor({ target, calendars, onClose, onSaved }: Props) {
  const writable = calendars.filter((c) => c.writable)
  const ev = target.kind === 'edit' ? target.event : null
  const initialStart = ev ? (ev.allDay ? new Date(`${ev.start}T09:00:00`) : new Date(ev.start)) : target.kind === 'new' ? target.start : new Date()
  const initialEnd = ev
    ? (ev.allDay ? new Date(`${addDaysStr(ev.end, -1)}T10:00:00`) : new Date(ev.end))
    : new Date(initialStart.getTime() + 60 * 60000)

  const [title, setTitle] = useState(ev?.title ?? '')
  const [calendarUrl, setCalendarUrl] = useState(ev?.calendarUrl ?? writable[0]?.url ?? '')
  const [allDay, setAllDay] = useState(ev?.allDay ?? (target.kind === 'new' && Boolean(target.allDay)))
  const [startDate, setStartDate] = useState(dateInput(initialStart))
  const [startTime, setStartTime] = useState(timeInput(initialStart))
  const [endDate, setEndDate] = useState(dateInput(initialEnd))
  const [endTime, setEndTime] = useState(timeInput(initialEnd))
  const [location, setLocation] = useState(ev?.location ?? '')
  const [description, setDescription] = useState(ev?.description ?? '')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  // Время повторяющегося события меняется для всей серии — это делаем в веб-интерфейсе,
  // чтобы случайно не сдвинуть все прошлые и будущие встречи.
  const timeLocked = Boolean(ev?.recurring)

  async function save(event: React.FormEvent) {
    event.preventDefault()
    if (!title.trim()) { setError('Укажите название'); return }
    let input: EventInput
    if (allDay) {
      if (endDate < startDate) { setError('Окончание раньше начала'); return }
      input = { title, location, description, allDay: true, start: startDate, end: addDaysStr(endDate, 1) }
    } else {
      const s = new Date(`${startDate}T${startTime}:00`)
      const e = new Date(`${endDate}T${endTime}:00`)
      if (Number.isNaN(s.getTime()) || Number.isNaN(e.getTime())) { setError('Некорректные дата или время'); return }
      if (e < s) { setError('Окончание раньше начала'); return }
      input = { title, location, description, allDay: false, start: s.toISOString(), end: e.toISOString() }
    }
    setBusy(true)
    setError('')
    try {
      if (ev) {
        await api.updateEvent(ev.href, input, !timeLocked)
        onSaved('Событие сохранено.')
      } else {
        await api.createEvent(calendarUrl, input)
        onSaved('Событие создано.')
      }
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget && !busy) onClose() }}>
      <form className="compose-modal editor-modal" onSubmit={save}>
        <div className="compose-header">
          <div><strong>{ev ? 'Изменить событие' : 'Новое событие'}</strong>{ev && <small>{ev.calendar}</small>}</div>
          <button type="button" className="icon-button" onClick={onClose} aria-label="Закрыть"><X size={18} /></button>
        </div>
        <label className="login-field"><span>Название</span><input autoFocus value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Совещание" /></label>
        {!ev && (
          <label className="login-field"><span>Календарь</span>
            <select value={calendarUrl} onChange={(e) => setCalendarUrl(e.target.value)}>
              {writable.map((c) => <option key={c.url} value={c.url}>{c.name}</option>)}
            </select>
          </label>
        )}
        <label className="login-remember"><input type="checkbox" checked={allDay} disabled={timeLocked} onChange={(e) => setAllDay(e.target.checked)} /><span>Весь день</span></label>
        <div className="editor-row">
          <label className="login-field"><span>Начало</span><input type="date" value={startDate} disabled={timeLocked} onChange={(e) => { setStartDate(e.target.value); if (endDate < e.target.value) setEndDate(e.target.value) }} /></label>
          {!allDay && <label className="login-field"><span>&nbsp;</span><input type="time" value={startTime} disabled={timeLocked} onChange={(e) => setStartTime(e.target.value)} /></label>}
          <label className="login-field"><span>Окончание</span><input type="date" value={endDate} disabled={timeLocked} onChange={(e) => setEndDate(e.target.value)} /></label>
          {!allDay && <label className="login-field"><span>&nbsp;</span><input type="time" value={endTime} disabled={timeLocked} onChange={(e) => setEndTime(e.target.value)} /></label>}
        </div>
        {timeLocked && <div className="inline-warning editor-note">Повторяющееся событие: здесь меняются название, место и описание всей серии. Время серии меняйте в веб-интерфейсе RuPost.</div>}
        <label className="login-field"><span>Место</span><input value={location} onChange={(e) => setLocation(e.target.value)} placeholder="Переговорная, ссылка на ВКС" /></label>
        <label className="login-field"><span>Описание</span><textarea rows={4} value={description} onChange={(e) => setDescription(e.target.value)} /></label>
        {error && <div className="login-error" role="alert">{error}</div>}
        <div className="compose-footer">
          <span />
          <button className="primary-button" type="submit" disabled={busy || (!ev && !calendarUrl)}>{busy ? 'Сохранение…' : 'Сохранить'}</button>
        </div>
      </form>
    </div>
  )
}
