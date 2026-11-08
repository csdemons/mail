import { useEffect, useMemo, useRef, useState } from 'react'
import { CalendarDays, ChevronLeft, ChevronRight, X } from 'lucide-react'

/**
 * Свой выбор даты вместо <input type="date">: WebKit рисует его в американском формате
 * (11/15/2026). Здесь — ДД.ММ.ГГГГ, календарик с понедельника и быстрые кнопки.
 * Значение — как у <input type="date">: «ГГГГ-ММ-ДД» или пустая строка; поле принимает
 * и ввод «15.11.2026», «15112026», «2026-11-15».
 */

const pad = (n: number) => String(n).padStart(2, '0')
export const isoOf = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
const dateOf = (iso: string) => { const [y, m, d] = iso.split('-').map(Number); return new Date(y, m - 1, d, 12) }

export function formatDate(iso: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso)
  return m ? `${m[3]}.${m[2]}.${m[1]}` : ''
}

/** Разобрать ввод: «ГГГГ-ММ-ДД», «ДД.ММ.ГГГГ», «ДД.ММ.ГГ», «ДДММГГГГ»; null — пока не дата. */
export function parseDate(raw: string): string | null {
  const t = raw.trim()
  let y: number, m: number, d: number
  let r = /^(\d{4})-(\d{1,2})-(\d{1,2})$/.exec(t)
  if (r) { y = +r[1]; m = +r[2]; d = +r[3] }
  else if ((r = /^(\d{1,2})[./\s-](\d{1,2})[./\s-](\d{2}|\d{4})$/.exec(t))) { d = +r[1]; m = +r[2]; y = r[3].length === 2 ? 2000 + +r[3] : +r[3] }
  else if ((r = /^(\d{2})(\d{2})(\d{4})$/.exec(t))) { d = +r[1]; m = +r[2]; y = +r[3] }
  else return null
  const dt = new Date(y, m - 1, d, 12)
  return dt.getFullYear() === y && dt.getMonth() === m - 1 && dt.getDate() === d && y >= 1900 && y <= 2200 ? isoOf(dt) : null
}

/** Ввод цифрами без точек — точки ставятся сами: «1511» → «15.11». */
function mask(raw: string): string {
  if (/[^\d.]/.test(raw)) return raw
  if (raw.includes('.')) return raw.slice(0, 10)
  const d = raw.slice(0, 8)
  return d.length > 4 ? `${d.slice(0, 2)}.${d.slice(2, 4)}.${d.slice(4)}` : d.length > 2 ? `${d.slice(0, 2)}.${d.slice(2)}` : d
}

const MONTHS = ['Январь', 'Февраль', 'Март', 'Апрель', 'Май', 'Июнь', 'Июль', 'Август', 'Сентябрь', 'Октябрь', 'Ноябрь', 'Декабрь']
const WEEK = ['Пн', 'Вт', 'Ср', 'Чт', 'Пт', 'Сб', 'Вс']

function quickDays(): { label: string; iso: string }[] {
  const now = new Date(); now.setHours(12, 0, 0, 0)
  const add = (n: number) => { const d = new Date(now); d.setDate(d.getDate() + n); return isoOf(d) }
  const toFriday = ((5 - now.getDay() + 7) % 7) || 7
  return [
    { label: 'Сегодня', iso: add(0) },
    { label: 'Завтра', iso: add(1) },
    { label: 'Пятница', iso: add(toFriday) },
    { label: 'Через неделю', iso: add(7) },
  ]
}

export function DateField({ value, onChange, label, disabled = false, min, clearable = true, className = '' }: {
  value: string
  onChange: (iso: string) => void
  label: string
  disabled?: boolean
  /** Не раньше этой даты («ГГГГ-ММ-ДД»). */
  min?: string
  clearable?: boolean
  className?: string
}) {
  const [text, setText] = useState(formatDate(value))
  const [open, setOpen] = useState(false)
  const [at, setAt] = useState<{ top: number; left: number } | null>(null)
  const [month, setMonth] = useState(() => (value ? dateOf(value) : new Date()))
  const box = useRef<HTMLDivElement>(null)
  const focused = useRef(false)

  // Значение поменяли снаружи — показать его (если человек не набирает сейчас то же самое).
  useEffect(() => {
    if (!focused.current || parseDate(text) !== value) setText(formatDate(value))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value])

  useEffect(() => {
    if (!open) return
    const down = (e: MouseEvent) => { if (box.current && !box.current.contains(e.target as Node)) setOpen(false) }
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.stopPropagation(); setOpen(false) } }
    document.addEventListener('mousedown', down)
    document.addEventListener('keydown', key, true)
    return () => { document.removeEventListener('mousedown', down); document.removeEventListener('keydown', key, true) }
  }, [open])

  function input(raw: string) {
    const t = mask(raw)
    setText(t)
    if (!t.trim()) { if (clearable) onChange(''); return }
    const iso = parseDate(t)
    if (iso && (!min || iso >= min)) { onChange(iso); setMonth(dateOf(iso)) }
  }
  function pick(iso: string) {
    onChange(iso)
    setText(formatDate(iso))
    setOpen(false)
  }

  const cells = useMemo(() => {
    const first = new Date(month.getFullYear(), month.getMonth(), 1, 12)
    const start = new Date(first)
    start.setDate(1 - ((first.getDay() + 6) % 7))
    return Array.from({ length: 42 }, (_, i) => { const d = new Date(start); d.setDate(start.getDate() + i); return d })
  }, [month])
  const today = isoOf(new Date())
  const bad = text.trim() !== '' && parseDate(text) === null

  return (
    <div className={`date-field ${className} ${bad ? 'invalid' : ''}`} ref={box}>
      <input className="date-field-input" data-kind="date" aria-label={label} placeholder="ДД.ММ.ГГГГ" inputMode="numeric" value={text} disabled={disabled}
        onFocus={() => { focused.current = true }}
        onBlur={() => { focused.current = false; if (parseDate(text) === null && text.trim()) setText(formatDate(value)) }}
        onChange={(e) => input(e.target.value)}
        onKeyDown={(e) => { if (e.key === 'ArrowDown' && e.altKey) { e.preventDefault(); setOpen(true) } }} />
      {clearable && value && !disabled && <button type="button" className="date-field-clear" aria-label={`${label}: очистить`} title="Очистить" onClick={() => pick('')}><X size={13} /></button>}
      <button type="button" className="date-field-button" aria-label={`${label}: календарь`} title="Выбрать в календаре" disabled={disabled}
        onClick={() => {
          setMonth(value ? dateOf(value) : new Date())
          // Над окнами с прокруткой: координаты окна, вверх — если внизу не хватает места.
          const r = box.current?.getBoundingClientRect()
          if (r) {
            const h = 330, w = 268
            const top = r.bottom + 6 + h > window.innerHeight && r.top - 6 - h > 0 ? r.top - 6 - h : r.bottom + 6
            setAt({ top, left: Math.max(8, Math.min(r.left, window.innerWidth - w - 8)) })
          }
          setOpen((o) => !o)
        }}><CalendarDays size={15} /></button>
      {open && (
        <div className="date-pop popover" role="dialog" aria-label={`${label}: календарь`} style={at ? { position: 'fixed', top: at.top, left: at.left } : undefined}>
          <div className="date-quick">
            {quickDays().filter((q) => !min || q.iso >= min).map((q) => (
              <button type="button" key={q.label} className={`filter-chip ${q.iso === value ? 'active' : ''}`} onClick={() => pick(q.iso)}>{q.label}</button>
            ))}
          </div>
          <div className="date-head">
            <button type="button" className="icon-button" aria-label="Предыдущий месяц" onClick={() => setMonth((m) => new Date(m.getFullYear(), m.getMonth() - 1, 1, 12))}><ChevronLeft size={16} /></button>
            <b>{MONTHS[month.getMonth()]} {month.getFullYear()}</b>
            <button type="button" className="icon-button" aria-label="Следующий месяц" onClick={() => setMonth((m) => new Date(m.getFullYear(), m.getMonth() + 1, 1, 12))}><ChevronRight size={16} /></button>
          </div>
          <div className="date-grid" role="grid">
            {WEEK.map((w, i) => <span key={w} className={`date-dow ${i > 4 ? 'weekend' : ''}`}>{w}</span>)}
            {cells.map((d) => {
              const iso = isoOf(d)
              const off = d.getMonth() !== month.getMonth()
              const blocked = Boolean(min && iso < min)
              return (
                <button type="button" key={iso} data-date={iso} disabled={blocked} aria-label={d.toLocaleDateString('ru-RU', { day: 'numeric', month: 'long', year: 'numeric' })}
                  className={`date-day ${off ? 'other' : ''} ${iso === today ? 'today' : ''} ${iso === value ? 'selected' : ''} ${(d.getDay() + 6) % 7 > 4 ? 'weekend' : ''}`}
                  onClick={() => pick(iso)}>{d.getDate()}</button>
              )
            })}
          </div>
        </div>
      )}
    </div>
  )
}

/** «930», «9:30», «09.30» → «09:30»; null — пока не время. */
export function parseTime(raw: string): string | null {
  const t = raw.trim()
  const r = /^(\d{1,2})[:.\s]?(\d{2})$/.exec(t) ?? /^(\d{1,2})$/.exec(t)
  if (!r) return null
  const h = +r[1]; const m = r[2] ? +r[2] : 0
  return h < 24 && m < 60 ? `${pad(h)}:${pad(m)}` : null
}

const SLOTS = Array.from({ length: 48 }, (_, i) => `${pad(Math.floor(i / 2))}:${i % 2 ? '30' : '00'}`)

/** Время в 24-часовом формате (WebKit показывал бы 10:00 AM). Значение «ЧЧ:ММ». */
export function TimeField({ value, onChange, label, disabled = false }: { value: string; onChange: (hhmm: string) => void; label: string; disabled?: boolean }) {
  const [text, setText] = useState(value)
  const focused = useRef(false)
  useEffect(() => { if (!focused.current || parseTime(text) !== value) setText(value) }, [value]) // eslint-disable-line react-hooks/exhaustive-deps
  const id = useMemo(() => `times-${Math.random().toString(36).slice(2, 8)}`, [])
  return (
    <div className={`date-field time-field ${text && parseTime(text) === null ? 'invalid' : ''}`}>
      <input className="date-field-input" data-kind="time" aria-label={label} placeholder="ЧЧ:ММ" inputMode="numeric" list={id} value={text} disabled={disabled}
        onFocus={() => { focused.current = true }}
        onBlur={() => { focused.current = false; const p = parseTime(text); setText(p ?? value) }}
        onChange={(e) => { setText(e.target.value); const p = parseTime(e.target.value); if (p) onChange(p) }} />
      <datalist id={id}>{SLOTS.map((s) => <option key={s} value={s} />)}</datalist>
    </div>
  )
}

/** Дата и время («ГГГГ-ММ-ДДTЧЧ:ММ», как у datetime-local) — два поля рядом. */
export function DateTimeField({ value, onChange, label, min }: { value: string; onChange: (v: string) => void; label: string; min?: string }) {
  const [date, time] = value ? value.split('T') : ['', '']
  return (
    <div className="datetime-field" role="group" aria-label={label}>
      <DateField label={`${label}: дата`} value={date} min={min?.slice(0, 10)} clearable={false} onChange={(d) => onChange(d ? `${d}T${time || '09:00'}` : '')} />
      <TimeField label={`${label}: время`} value={time ?? ''} onChange={(t) => onChange(`${date || isoOf(new Date())}T${t}`)} />
    </div>
  )
}
