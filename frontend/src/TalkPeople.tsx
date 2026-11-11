import { useEffect, useMemo, useState } from 'react'
import { AlarmClock, CalendarDays, Mail, MessageCircle, Pin, User, X } from 'lucide-react'
import { api, errorText } from './api'
import Avatar from './Avatar'
import { DateTimeField } from './DateField'
import type { MenuItem } from './PopupMenu'
import type { BusySlot, NcProfile, TalkMessage } from './types'

// ---------------------------------------------------------------- закрепление

const DAY = 86400

/** Закрепить сообщение: на 24 часа, 7 или 30 дней, до даты — или бессрочно (как в Talk). */
export function PinDialog({ message, onPin, onClose }: { message: TalkMessage; onPin: (until: number) => Promise<void>; onClose: () => void }) {
  const [choice, setChoice] = useState<'1' | '7' | '30' | 'date' | 'forever'>('1')
  const [date, setDate] = useState(() => isoLocal(new Date(Date.now() + 3 * DAY * 1000)))
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const until = () => {
    const now = Math.floor(Date.now() / 1000)
    if (choice === 'forever') return 0
    if (choice === 'date') return Math.floor(new Date(date).getTime() / 1000)
    return now + Number(choice) * DAY
  }
  async function submit() {
    const u = until()
    if (choice === 'date' && (!Number.isFinite(u) || u <= Date.now() / 1000)) { setError('Выберите время в будущем'); return }
    setBusy(true); setError('')
    try { await onPin(u); onClose() } catch (e) { setError(errorText(e)); setBusy(false) }
  }
  const options: [typeof choice, string][] = [['1', 'На 24 часа'], ['7', 'На 7 дней'], ['30', 'На 30 дней'], ['date', 'До даты…'], ['forever', 'Бессрочно']]
  return (
    <div className="modal-backdrop nested" onMouseDown={(e) => { if (e.target === e.currentTarget && !busy) onClose() }}>
      <form className="settings-modal confirm-modal talk-small-dialog" role="dialog" aria-label="Закрепить сообщение" onSubmit={(e) => { e.preventDefault(); void submit() }}
        onKeyDown={(e) => { if (e.key === 'Escape' && !busy) { e.stopPropagation(); onClose() } }}>
        <div className="compose-header"><div><strong><Pin size={15} /> Закрепить сообщение</strong><small>Его увидят все участники вверху беседы</small></div></div>
        <blockquote className="talk-small-quote">{message.text.slice(0, 200) || (message.files[0]?.name ?? '')}</blockquote>
        <div className="talk-small-options" role="radiogroup" aria-label="Срок">
          {options.map(([v, t]) => (
            <label key={v}><input type="radio" name="pin-for" checked={choice === v} onChange={() => setChoice(v)} /> {t}</label>
          ))}
          {choice === 'date' && <DateTimeField label="Закрепить до" value={date} onChange={setDate} />}
        </div>
        {error && <div className="inline-warning" role="alert">{error}</div>}
        <div className="settings-actions">
          <button type="button" className="secondary-button" disabled={busy} onClick={onClose}>Отмена</button>
          <button type="submit" className="primary-button" disabled={busy}>{busy ? 'Закрепляю…' : 'Закрепить'}</button>
        </div>
      </form>
    </div>
  )
}

// ---------------------------------------------------------------- напоминание

function at9(d: Date): Date {
  const x = new Date(d); x.setHours(9, 0, 0, 0); return x
}
function isoLocal(d: Date): string {
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`
}
const fmtWhen = (t: number) => new Date(t * 1000).toLocaleString('ru-RU', { day: 'numeric', month: 'long', hour: '2-digit', minute: '2-digit' })

/** Напомнить о сообщении: Talk пришлёт уведомление в выбранное время. */
export function ReminderDialog({ token, message, notify, onClose }: { token: string; message: TalkMessage; notify: (t: string) => void; onClose: () => void }) {
  const [current, setCurrent] = useState<number | null | undefined>(undefined)
  const [custom, setCustom] = useState(false)
  const [date, setDate] = useState(() => isoLocal(at9(new Date(Date.now() + DAY * 1000))))
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  useEffect(() => { api.talkReminder(token, message.id).then(setCurrent).catch(() => setCurrent(null)) }, [token, message.id])
  const presets = useMemo(() => {
    const now = new Date()
    const tomorrow = at9(new Date(now.getTime() + DAY * 1000))
    const monday = at9(new Date(now)); monday.setDate(monday.getDate() + ((8 - monday.getDay()) % 7 || 7))
    return [
      ['Через 30 минут', new Date(now.getTime() + 30 * 60000)],
      ['Через час', new Date(now.getTime() + 3600000)],
      ['Через 3 часа', new Date(now.getTime() + 3 * 3600000)],
      ['Завтра в 9:00', tomorrow],
      ['В понедельник в 9:00', monday],
    ] as [string, Date][]
  }, [])
  async function set(at: number | null) {
    if (at !== null && at <= Date.now() / 1000) { setError('Выберите время в будущем'); return }
    setBusy(true); setError('')
    try {
      await api.talkSetReminder(token, message.id, at)
      notify(at === null ? 'Напоминание убрано.' : `Напомню ${fmtWhen(at)}.`)
      onClose()
    } catch (e) { setError(errorText(e)); setBusy(false) }
  }
  return (
    <div className="modal-backdrop nested" onMouseDown={(e) => { if (e.target === e.currentTarget && !busy) onClose() }}>
      <div className="settings-modal confirm-modal talk-small-dialog" role="dialog" aria-label="Напомнить о сообщении"
        onKeyDown={(e) => { if (e.key === 'Escape' && !busy) { e.stopPropagation(); onClose() } }}>
        <div className="compose-header"><div><strong><AlarmClock size={15} /> Напомнить о сообщении</strong><small>{current ? `Сейчас: ${fmtWhen(current)}` : 'Уведомление придёт от Talk — и здесь, и на телефоне'}</small></div></div>
        <blockquote className="talk-small-quote">{message.text.slice(0, 200) || (message.files[0]?.name ?? '')}</blockquote>
        <div className="talk-small-presets">
          {presets.map(([t, d]) => <button key={t} type="button" className="secondary-button" disabled={busy} onClick={() => void set(Math.floor(d.getTime() / 1000))}>{t} <small>{d.toLocaleString('ru-RU', { weekday: 'short', day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })}</small></button>)}
          <button type="button" className="secondary-button" disabled={busy} onClick={() => setCustom(!custom)}>Выбрать дату и время…</button>
          {custom && (
            <div className="talk-small-custom">
              <DateTimeField label="Напомнить" value={date} onChange={setDate} />
              <button type="button" className="primary-button" disabled={busy} onClick={() => void set(Math.floor(new Date(date).getTime() / 1000))}>Напомнить</button>
            </div>
          )}
        </div>
        {error && <div className="inline-warning" role="alert">{error}</div>}
        <div className="settings-actions">
          {current && <button type="button" className="secondary-button danger" disabled={busy} onClick={() => void set(null)}>Убрать напоминание</button>}
          <button type="button" className="secondary-button" disabled={busy} onClick={onClose}>Закрыть</button>
        </div>
      </div>
    </div>
  )
}

// ---------------------------------------------------------------- сотрудник

export interface PersonActions {
  /** Открыть (или создать) личную беседу. */
  chat: (userId: string) => void
  /** Карточка сотрудника; `availability` — сразу показать занятость. */
  profile: (userId: string, name: string, availability?: boolean) => void
  /** Написать письмо (адрес — из профиля). */
  write: (userId: string, name: string) => void
}

/** Пункты меню о сотруднике: чат, профиль, занятость, письмо. */
export function personMenu(userId: string, name: string, a: PersonActions, opts: { inDirect?: boolean } = {}): MenuItem[] {
  return [
    ...(opts.inDirect ? [] : [{ label: 'Чат с пользователем', icon: <MessageCircle size={15} />, onClick: () => a.chat(userId) }]),
    { label: 'Профиль', icon: <User size={15} />, onClick: () => a.profile(userId, name) },
    { label: 'Доступность (календарь)', icon: <CalendarDays size={15} />, onClick: () => a.profile(userId, name, true) },
    { label: 'Написать письмо', icon: <Mail size={15} />, onClick: () => a.write(userId, name) },
  ]
}

const STATUS: Record<string, string> = { online: 'в сети', away: 'отошёл', dnd: 'не беспокоить', busy: 'занят', offline: 'не в сети', invisible: 'не в сети' }

/** Рабочие дни с сегодняшнего: [начало дня, конец] (местное время). */
function workDays(n: number): Date[] {
  const out: Date[] = []
  const d = new Date(); d.setHours(0, 0, 0, 0)
  while (out.length < n) {
    if (d.getDay() !== 0 && d.getDay() !== 6) out.push(new Date(d))
    d.setDate(d.getDate() + 1)
  }
  return out
}
const H_FROM = 8
const H_TO = 20

/** Карточка сотрудника: профиль, статус, занятость на неделю; чат и письмо. */
export function ProfileDialog({ userId, name, showBusy, onChat, onWrite, onClose }: {
  userId: string; name: string; showBusy: boolean
  onChat: () => void; onWrite: (email: string, name: string) => void; onClose: () => void
}) {
  const [p, setP] = useState<NcProfile | null>(null)
  const [error, setError] = useState('')
  const [busy, setBusy] = useState<BusySlot[] | null>(null)
  const [busyErr, setBusyErr] = useState('')
  const [wantBusy, setWantBusy] = useState(showBusy)
  const days = useMemo(() => workDays(5), [])
  useEffect(() => { api.ncUserProfile(userId).then(setP).catch((e) => setError(errorText(e))) }, [userId])
  useEffect(() => {
    if (!wantBusy || !p) return
    if (!p.email) { setBusyErr('В профиле сотрудника не открыт адрес почты — занятость не узнать'); return }
    const from = Math.floor(days[0].getTime() / 1000)
    const last = new Date(days[days.length - 1]); last.setDate(last.getDate() + 1)
    api.ncFreeBusy(p.email, from, Math.floor(last.getTime() / 1000)).then(setBusy).catch((e) => setBusyErr(errorText(e)))
  }, [wantBusy, p, days])
  const shownName = p?.name || name
  const rows: [string, string][] = p ? ([['Должность', [p.role, p.headline].filter(Boolean).join(' · ')], ['Организация', p.organisation], ['Почта', p.email], ['Телефон', p.phone], ['Адрес', p.address]] as [string, string][]).filter(([, v]) => v) : []
  return (
    <div className="modal-backdrop nested" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}>
      <div className="settings-modal talk-profile" role="dialog" aria-label={`Профиль: ${shownName}`} onKeyDown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); onClose() } }}>
        <div className="compose-header">
          <div className="talk-profile-head">
            <Avatar id={userId} name={shownName} />
            <div><strong>{shownName}</strong><small>{p?.status ? `${STATUS[p.status] ?? p.status}${p.statusMessage ? ` · ${p.statusMessage}` : ''}` : userId}</small></div>
          </div>
          <button type="button" className="icon-button" aria-label="Закрыть" onClick={onClose}><X size={18} /></button>
        </div>
        {error && <div className="inline-warning" role="alert">{error}</div>}
        {!p && !error && <p className="muted talk-profile-loading">Загрузка профиля…</p>}
        {rows.length > 0 && <dl className="talk-profile-fields">{rows.map(([k, v]) => <div key={k}><dt>{k}</dt><dd>{k === 'Почта' ? <a href={`mailto:${v}`} onClick={(e) => { e.preventDefault(); onWrite(v, shownName) }}>{v}</a> : v}</dd></div>)}</dl>}
        <div className="talk-profile-actions">
          <button type="button" className="secondary-button" onClick={onChat}><MessageCircle size={15} /> Чат с пользователем</button>
          <button type="button" className="secondary-button" disabled={!p?.email} title={p && !p.email ? 'Адрес почты не указан в профиле' : ''} onClick={() => p?.email && onWrite(p.email, shownName)}><Mail size={15} /> Написать письмо</button>
          <button type="button" className={`secondary-button ${wantBusy ? 'active' : ''}`} aria-pressed={wantBusy} onClick={() => setWantBusy(!wantBusy)}><CalendarDays size={15} /> Доступность</button>
        </div>
        {wantBusy && (
          <section className="free-busy" aria-label="Занятость">
            <strong>Занятость на неделю <small className="muted">({H_FROM}:00–{H_TO}:00, по календарю Nextcloud)</small></strong>
            {busyErr && <div className="inline-warning" role="alert">{busyErr}</div>}
            {!busyErr && busy === null && <p className="muted">Узнаю занятость…</p>}
            {!busyErr && busy && (
              <div className="free-busy-grid">
                <div className="free-busy-hours">{Array.from({ length: H_TO - H_FROM + 1 }, (_, i) => <span key={i} style={{ left: `${(i / (H_TO - H_FROM)) * 100}%` }}>{H_FROM + i}</span>)}</div>
                {days.map((d) => {
                  const start = new Date(d); start.setHours(H_FROM, 0, 0, 0)
                  const end = new Date(d); end.setHours(H_TO, 0, 0, 0)
                  const s0 = start.getTime() / 1000, s1 = end.getTime() / 1000
                  const slots = busy.filter((b) => b.end > s0 && b.start < s1)
                  return (
                    <div key={d.toISOString()} className="free-busy-row" data-day={isoLocal(d).slice(0, 10)}>
                      <span className="free-busy-day">{d.toLocaleDateString('ru-RU', { weekday: 'short', day: 'numeric', month: 'short' })}</span>
                      <div className="free-busy-track">
                        {slots.map((b) => {
                          const a = Math.max(b.start, s0), z = Math.min(b.end, s1)
                          const t = `${new Date(a * 1000).toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' })}–${new Date(z * 1000).toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' })}`
                          return <i key={`${b.start}-${b.end}`} className={`free-busy-slot ${b.kind === 'BUSY-TENTATIVE' ? 'tentative' : ''}`} title={`${b.kind === 'BUSY-TENTATIVE' ? 'Под вопросом' : 'Занят'}: ${t}`}
                            style={{ left: `${((a - s0) / (s1 - s0)) * 100}%`, width: `${((z - a) / (s1 - s0)) * 100}%` }} />
                        })}
                      </div>
                    </div>
                  )
                })}
                {busy.length === 0 && <p className="muted">Свободен всю неделю.</p>}
              </div>
            )}
          </section>
        )}
      </div>
    </div>
  )
}
