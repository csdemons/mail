import { useEffect, useMemo, useRef, useState } from 'react'
import { CalendarDays, ChevronLeft, ChevronRight, Clock, MapPin, Repeat, User, X } from 'lucide-react'

import { api, errorText } from './api'
import type { CalEvent, EventsResult } from './types'

const HOUR_PX = 48
const DAY_NAMES = ['Пн', 'Вт', 'Ср', 'Чт', 'Пт', 'Сб', 'Вс']
const FALLBACK_COLORS = ['#2474ed', '#e66b2e', '#1f9d74', '#9b51e0', '#d6336c']

function startOfWeek(d: Date): Date {
  const r = new Date(d.getFullYear(), d.getMonth(), d.getDate())
  r.setDate(r.getDate() - ((r.getDay() + 6) % 7))
  return r
}

function addDays(d: Date, n: number): Date {
  const r = new Date(d)
  r.setDate(r.getDate() + n)
  return r
}

function ymd(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
}

function sameDay(a: Date, b: Date) {
  return a.toDateString() === b.toDateString()
}

function hm(d: Date) {
  return d.toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' })
}

interface Placed {
  ev: CalEvent
  top: number
  height: number
  col: number
  cols: number
}

/** Раскладка событий одного дня: пересекающиеся встают рядом. */
function layoutDay(events: CalEvent[], day: Date): Placed[] {
  const dayStart = day.getTime()
  const dayEnd = addDays(day, 1).getTime()
  const items = events
    .map((ev) => ({ ev, s: Math.max(new Date(ev.start).getTime(), dayStart), e: Math.min(new Date(ev.end).getTime(), dayEnd) }))
    .filter((x) => x.s < dayEnd && (x.e > dayStart || x.s === x.e))
    .sort((a, b) => a.s - b.s || b.e - a.e)

  const placed: Placed[] = []
  let cluster: { item: (typeof items)[number]; col: number }[] = []
  let clusterEnd = -Infinity
  const flush = () => {
    const cols = Math.max(1, ...cluster.map((c) => c.col + 1))
    for (const c of cluster) {
      const minutesFrom = (c.item.s - dayStart) / 60000
      const minutes = Math.max(25, (c.item.e - c.item.s) / 60000)
      placed.push({ ev: c.item.ev, top: (minutesFrom / 60) * HOUR_PX, height: (minutes / 60) * HOUR_PX - 2, col: c.col, cols })
    }
    cluster = []
  }
  for (const item of items) {
    const end = Math.max(item.e, item.s + 25 * 60000)
    if (item.s >= clusterEnd) {
      flush()
      clusterEnd = -Infinity
    }
    const used = new Set(cluster.filter((c) => Math.max(c.item.e, c.item.s + 25 * 60000) > item.s).map((c) => c.col))
    let col = 0
    while (used.has(col)) col++
    cluster.push({ item, col })
    clusterEnd = Math.max(clusterEnd, end)
  }
  flush()
  return placed
}

export default function CalendarView() {
  const [weekStart, setWeekStart] = useState(() => startOfWeek(new Date()))
  const [data, setData] = useState<EventsResult | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [hidden, setHidden] = useState<Set<string>>(new Set())
  const [selected, setSelected] = useState<CalEvent | null>(null)
  const [now, setNow] = useState(new Date())
  const gridRef = useRef<HTMLDivElement>(null)
  const request = useRef(0)

  const days = useMemo(() => Array.from({ length: 7 }, (_, i) => addDays(weekStart, i)), [weekStart])

  useEffect(() => {
    const id = request.current + 1
    request.current = id
    setLoading(true)
    setError('')
    api.listEvents(weekStart, addDays(weekStart, 7))
      .then((r) => { if (request.current === id) setData(r) })
      .catch((e) => { if (request.current === id) setError(errorText(e)) })
      .finally(() => { if (request.current === id) setLoading(false) })
  }, [weekStart])

  useEffect(() => {
    const t = setInterval(() => setNow(new Date()), 60000)
    return () => clearInterval(t)
  }, [])

  useEffect(() => {
    // Прокрутка к 8:00 при открытии.
    gridRef.current?.scrollTo({ top: 8 * HOUR_PX - 10 })
  }, [])

  const colorOf = useMemo(() => {
    const map = new Map<string, string>()
    data?.calendars.forEach((c, i) => map.set(c.name, c.color ?? FALLBACK_COLORS[i % FALLBACK_COLORS.length]))
    return (name: string) => map.get(name) ?? FALLBACK_COLORS[0]
  }, [data])

  const events = (data?.events ?? []).filter((e) => !hidden.has(e.calendar))
  const allDay = events.filter((e) => e.allDay)
  const timed = events.filter((e) => !e.allDay)

  const title = `${days[0].toLocaleDateString('ru-RU', { day: 'numeric', month: 'long' })} — ${days[6].toLocaleDateString('ru-RU', { day: 'numeric', month: 'long', year: 'numeric' })}`

  function eventTime(ev: CalEvent) {
    if (ev.allDay) {
      const s = new Date(`${ev.start}T00:00:00`)
      const e = addDays(new Date(`${ev.end}T00:00:00`), -1)
      return sameDay(s, e)
        ? `${s.toLocaleDateString('ru-RU', { weekday: 'long', day: 'numeric', month: 'long' })}, весь день`
        : `${s.toLocaleDateString('ru-RU', { day: 'numeric', month: 'long' })} — ${e.toLocaleDateString('ru-RU', { day: 'numeric', month: 'long' })}`
    }
    const s = new Date(ev.start)
    const e = new Date(ev.end)
    return `${s.toLocaleDateString('ru-RU', { weekday: 'long', day: 'numeric', month: 'long' })}, ${hm(s)}–${hm(e)}`
  }

  return (
    <div className="module-view calendar-view">
      <aside className="folder-sidebar">
        <div className="sidebar-heading"><span>Календари</span></div>
        <div className="folder-list">
          {data?.calendars.map((c) => (
            <label key={c.url} className="folder-item calendar-toggle" title={c.url}>
              <input type="checkbox" checked={!hidden.has(c.name)} onChange={() => setHidden((h) => {
                const n = new Set(h)
                if (n.has(c.name)) n.delete(c.name)
                else n.add(c.name)
                return n
              })} style={{ accentColor: colorOf(c.name) }} />
              <span>{c.name}</span>
            </label>
          ))}
          {data && data.calendars.length === 0 && <div className="folder-placeholder">Календари не найдены</div>}
        </div>
      </aside>

      <main className="calendar-main">
        <div className="calendar-toolbar">
          <button className="secondary-button" onClick={() => setWeekStart(startOfWeek(new Date()))}>Сегодня</button>
          <button className="icon-button soft" aria-label="Предыдущая неделя" onClick={() => setWeekStart((w) => addDays(w, -7))}><ChevronLeft size={18} /></button>
          <button className="icon-button soft" aria-label="Следующая неделя" onClick={() => setWeekStart((w) => addDays(w, 7))}><ChevronRight size={18} /></button>
          <strong className="calendar-title">{title}</strong>
          {loading && <span className="muted">Загрузка…</span>}
        </div>
        {error && <div className="inline-warning" role="alert">Не удалось загрузить календарь: {error}. Адрес сервера — в Настройках, подробности — в Диагностике.</div>}
        {data?.errors.map((e) => <div key={e} className="inline-warning">{e}</div>)}

        <div className="week-head">
          <div className="time-gutter" />
          {days.map((d, i) => (
            <div key={i} className={`week-day-head ${sameDay(d, now) ? 'today' : ''}`}>
              <span>{DAY_NAMES[i]}</span><b>{d.getDate()}</b>
            </div>
          ))}
        </div>
        <div className="week-allday">
          <div className="time-gutter">весь день</div>
          {days.map((d, i) => (
            <div key={i} className="allday-cell">
              {allDay.filter((e) => e.start <= ymd(d) && ymd(d) < e.end).map((e) => (
                <button key={e.id} className="event-chip" style={{ background: colorOf(e.calendar) }} onClick={() => setSelected(e)}>{e.title}</button>
              ))}
            </div>
          ))}
        </div>
        <div className="week-grid" ref={gridRef}>
          <div className="time-gutter">
            {Array.from({ length: 24 }, (_, h) => <div key={h} className="hour-label" style={{ height: HOUR_PX }}>{h > 0 ? `${String(h).padStart(2, '0')}:00` : ''}</div>)}
          </div>
          {days.map((d, i) => (
            <div key={i} className={`day-column ${sameDay(d, now) ? 'today' : ''}`} style={{ height: 24 * HOUR_PX }}>
              {Array.from({ length: 24 }, (_, h) => <div key={h} className="hour-line" style={{ top: h * HOUR_PX }} />)}
              {sameDay(d, now) && <div className="now-line" style={{ top: ((now.getHours() * 60 + now.getMinutes()) / 60) * HOUR_PX }} />}
              {layoutDay(timed, d).map((p) => (
                <button
                  key={`${p.ev.id}-${i}`}
                  className={`event-block ${p.height < 40 ? 'compact' : ''}`}
                  title={`${p.ev.title}\n${eventTime(p.ev)}`}
                  style={{
                    top: p.top,
                    height: Math.max(20, p.height),
                    left: `calc(${(p.col / p.cols) * 100}% + 2px)`,
                    width: `calc(${100 / p.cols}% - 4px)`,
                    borderColor: colorOf(p.ev.calendar),
                    background: `${colorOf(p.ev.calendar)}22`,
                  }}
                  onClick={() => setSelected(p.ev)}
                >
                  {p.height < 40
                    ? <b><span>{hm(new Date(p.ev.start))}</span> {p.ev.title}</b>
                    : <>
                      <b>{p.ev.title}</b>
                      <span>{hm(new Date(p.ev.start))}{p.ev.location ? ` · ${p.ev.location}` : ''}</span>
                    </>}
                </button>
              ))}
            </div>
          ))}
        </div>
      </main>

      {selected && (
        <section className="reader-pane event-details">
          <div className="reader-toolbar"><div className="reader-toolbar-spacer" /><button className="icon-button soft" aria-label="Закрыть" onClick={() => setSelected(null)}><X size={18} /></button></div>
          <div className="reader-content">
            <div className="reader-title-row"><span className="event-dot" style={{ background: colorOf(selected.calendar) }} /><h1>{selected.title}</h1></div>
            <div className="contact-line"><Clock size={16} /><span>{eventTime(selected)}</span></div>
            {selected.recurring && <div className="contact-line"><Repeat size={16} /><span>Повторяющееся событие</span></div>}
            {selected.location && <div className="contact-line"><MapPin size={16} /><span>{selected.location}</span></div>}
            {selected.organizer && <div className="contact-line"><User size={16} /><span>Организатор: {selected.organizer}</span></div>}
            <div className="contact-line"><CalendarDays size={16} /><span>{selected.calendar}</span></div>
            {selected.description && <article className="message-body plain">{selected.description}</article>}
          </div>
        </section>
      )}
    </div>
  )
}
