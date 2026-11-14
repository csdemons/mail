// Чат: встреча из беседы, «в отпуске» и общие встречи собеседника, все напоминания,
// сводка непрочитанного и перевод (ИИ на сервере), комнаты для групп.
import { useEffect, useRef, useState } from 'react'
import { AlarmClock, CalendarDays, CalendarPlus, Languages, Palmtree, Sparkles, Trash2, Users, X } from 'lucide-react'

import { api, errorText } from './api'
import { DateTimeField } from './DateField'
import { localInputValue } from './sending'
import { SkeletonRows } from './Skeleton'
import TalkText from './TalkText'
import { waitTask } from './talkUtils'
import type { NcAbsence, NcCalendar, TalkEvent, TalkFeatures, TalkParticipant, TalkReminderItem, TalkRoom } from './types'

const when = (ts: number) => new Date(ts * 1000).toLocaleString('ru-RU', { weekday: 'short', day: 'numeric', month: 'long', hour: '2-digit', minute: '2-digit' })
const day = (iso: string) => new Date(`${iso}T12:00:00`).toLocaleDateString('ru-RU', { day: 'numeric', month: 'long' })

function Modal({ label, title, hint, onClose, children, wide = false }: { label: string; title: React.ReactNode; hint?: string; onClose: () => void; children: React.ReactNode; wide?: boolean }) {
  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}>
      <div className={`settings-modal talk-tool-modal ${wide ? 'wide' : ''}`} role="dialog" aria-label={label}>
        <div className="compose-header"><div><strong>{title}</strong>{hint && <small>{hint}</small>}</div><button className="icon-button" onClick={onClose} aria-label="Закрыть"><X size={18} /></button></div>
        {children}
      </div>
    </div>
  )
}

/** Запланировать встречу беседы: событие в календаре Nextcloud и приглашения участникам. */
export function MeetingDialog({ room, notify, onClose }: { room: TalkRoom; notify: (t: string) => void; onClose: () => void }) {
  const next = new Date(); next.setHours(next.getHours() + 1, 0, 0, 0)
  const [start, setStart] = useState(localInputValue(next))
  const [minutes, setMinutes] = useState(60)
  const [title, setTitle] = useState(room.name)
  const [description, setDescription] = useState('')
  const [calendars, setCalendars] = useState<NcCalendar[] | null>(null)
  const [calendar, setCalendar] = useState('')
  const [people, setPeople] = useState<TalkParticipant[]>([])
  const [invited, setInvited] = useState<Set<number>>(new Set())
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  useEffect(() => {
    api.ncCalendars().then((l) => { setCalendars(l); setCalendar(l.find((c) => c.uri === 'personal')?.uri ?? l[0]?.uri ?? '') }).catch((e) => { setCalendars([]); setError(errorText(e)) })
    api.talkParticipants(room.token).then((l) => { const others = l.filter((p) => !p.me); setPeople(others); setInvited(new Set(others.map((p) => p.attendeeId))) }).catch(() => undefined)
  }, [room.token])

  async function save() {
    const from = Math.floor(new Date(start).getTime() / 1000)
    if (!calendar || !Number.isFinite(from)) return
    setBusy(true)
    setError('')
    try {
      // Все отмечены — сервер пригласит всех участников беседы (и тех, кто придёт позже в список).
      const ids = invited.size === people.length ? null : [...invited]
      await api.talkScheduleMeeting(room.token, calendar, from, from + minutes * 60, title, description, ids)
      notify(`Встреча «${title.trim() || room.name}» ${when(from)} — в календаре, приглашения отправлены`)
      onClose()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Modal label="Запланировать встречу" title={<><CalendarPlus size={16} /> Запланировать встречу</>} hint="Событие появится в календаре Nextcloud, участники получат приглашения" onClose={onClose}>
      <label className="login-field"><span>Тема</span><input aria-label="Тема встречи" value={title} maxLength={250} onChange={(e) => setTitle(e.target.value)} /></label>
      <DateTimeField label="Начало" value={start} onChange={setStart} />
      <label className="login-field"><span>Длительность</span>
        <select aria-label="Длительность" value={minutes} onChange={(e) => setMinutes(Number(e.target.value))}>
          {[[15, '15 минут'], [30, '30 минут'], [45, '45 минут'], [60, '1 час'], [90, '1,5 часа'], [120, '2 часа'], [180, '3 часа']].map(([v, l]) => <option key={v} value={v}>{l}</option>)}
        </select>
      </label>
      <label className="login-field"><span>Календарь</span>
        {calendars === null ? <span className="muted">Загрузка…</span> : (
          <select aria-label="Календарь" value={calendar} onChange={(e) => setCalendar(e.target.value)} disabled={!calendars.length}>
            {calendars.map((c) => <option key={c.uri} value={c.uri}>{c.name}</option>)}
          </select>
        )}
      </label>
      <label className="login-field"><span>Описание</span><textarea rows={2} value={description} onChange={(e) => setDescription(e.target.value)} placeholder="Повестка (необязательно)" /></label>
      {people.length > 0 && <div className="settings-section talk-meeting-people">
        <strong><Users size={14} /> Пригласить ({invited.size} из {people.length})</strong>
        {people.map((p) => (
          <label key={p.attendeeId}><input type="checkbox" checked={invited.has(p.attendeeId)}
            onChange={(e) => setInvited((s) => { const n = new Set(s); if (e.target.checked) n.add(p.attendeeId); else n.delete(p.attendeeId); return n })} /> {p.name}</label>
        ))}
      </div>}
      {error && <div className="login-error" role="alert">{error}</div>}
      <div className="settings-actions">
        <button className="secondary-button" onClick={onClose}>Отмена</button>
        <button className="primary-button" disabled={busy || !calendar || !start} onClick={() => void save()}>Запланировать</button>
      </div>
    </Modal>
  )
}

/** Под шапкой личной беседы: собеседник в отпуске и ближайшие общие встречи. */
export function PeerInfo({ room, features }: { room: TalkRoom; features: TalkFeatures }) {
  const [absence, setAbsence] = useState<NcAbsence | null>(null)
  const [events, setEvents] = useState<TalkEvent[]>([])
  useEffect(() => {
    setAbsence(null)
    setEvents([])
    if (room.kind !== 1 || !room.peer) return
    let alive = true
    api.ncAbsence(room.peer).then((a) => { if (alive) setAbsence(a) }).catch(() => undefined)
    if (features.mutualEvents) api.talkMutualEvents(room.token).then((l) => { if (alive) setEvents(l) }).catch(() => undefined)
    return () => { alive = false }
  }, [room.token, room.kind, room.peer, features.mutualEvents])
  if (!absence && events.length === 0) return null
  return (
    <div className="talk-peer-info">
      {absence && <div className="talk-absence" role="status"><Palmtree size={14} />
        <span><b>{room.name}</b> {absence.until ? `в отпуске до ${day(absence.until)}` : 'сейчас отсутствует'}{absence.message ? ` — ${absence.message}` : ''}{absence.replacement ? `. Замещает: ${absence.replacement}` : ''}</span></div>}
      {events.length > 0 && <div className="talk-mutual-events"><CalendarDays size={14} />
        <span>Общие встречи: {events.map((e, i) => <button key={`${e.start}-${i}`} type="button" className="text-button" title={e.link ? 'Открыть в календаре' : undefined}
          onClick={() => { if (e.link) void api.openLink(e.link) }}>{when(e.start)} — {e.name || 'встреча'}</button>)}</span></div>}
    </div>
  )
}

/** Все напоминания о сообщениях (из всех бесед): перейти к сообщению или убрать напоминание. */
export function RemindersDialog({ rooms, onOpen, notify, onClose }: { rooms: TalkRoom[]; onOpen: (token: string, messageId: number) => void; notify: (t: string) => void; onClose: () => void }) {
  const [items, setItems] = useState<TalkReminderItem[] | null>(null)
  const [error, setError] = useState('')
  useEffect(() => { api.talkUpcomingReminders().then(setItems).catch((e) => { setItems([]); setError(errorText(e)) }) }, [])
  async function remove(r: TalkReminderItem) {
    try {
      await api.talkSetReminder(r.token, r.messageId, null)
      setItems((l) => l && l.filter((x) => !(x.token === r.token && x.messageId === r.messageId)))
      notify('Напоминание убрано')
    } catch (e) { notify(`Не удалось: ${errorText(e)}`) }
  }
  return (
    <Modal label="Напоминания" title={<><AlarmClock size={16} /> Напоминания</>} hint="О каких сообщениях Talk напомнит — во всех беседах" onClose={onClose} wide>
      {error && <div className="login-error" role="alert">{error}</div>}
      {items === null && <SkeletonRows variant="file" count={3} />}
      {items?.length === 0 && !error && <div className="muted talk-media-empty">Напоминаний нет. Поставить его можно правой кнопкой по сообщению → «Напомнить…».</div>}
      <div className="talk-reminder-list">
        {items?.map((r) => (
          <div key={`${r.token}-${r.messageId}`} className="talk-reminder-item">
            <button type="button" className="talk-reminder-open" onClick={() => { onClose(); onOpen(r.token, r.messageId) }}>
              <span className="talk-reminder-when">{when(r.at)}</span>
              <span className="talk-reminder-room">{rooms.find((x) => x.token === r.token)?.name ?? 'Беседа'} · {r.author}</span>
              <span className="talk-reminder-text">{r.text}</span>
            </button>
            <button type="button" className="icon-button soft" aria-label="Убрать напоминание" title="Убрать напоминание" onClick={() => void remove(r)}><Trash2 size={14} /></button>
          </div>
        ))}
      </div>
    </Modal>
  )
}

/** Сводка непрочитанного: ИИ-ассистент сервера пересказывает сообщения после `from`. */
export function SummaryDialog({ room, from, count, onClose }: { room: TalkRoom; from: number; count: number; onClose: () => void }) {
  const [parts, setParts] = useState<string[]>([])
  const [busy, setBusy] = useState(true)
  const [error, setError] = useState('')
  const alive = useRef(true)
  useEffect(() => {
    alive.current = true
    void (async () => {
      try {
        // Много сообщений — сервер пересказывает частями: следующая с nextOffset.
        let offset = from
        for (let n = 0; n < 5 && alive.current; n++) {
          const t = await api.talkSummarize(room.token, offset)
          if (!t) break
          const text = await waitTask(t.taskId, () => alive.current)
          if (alive.current) setParts((p) => [...p, text])
          if (!t.nextOffset) break
          offset = t.nextOffset
        }
      } catch (e) {
        if (alive.current) setError(errorText(e))
      } finally {
        if (alive.current) setBusy(false)
      }
    })()
    return () => { alive.current = false }
  }, [room.token, from])
  return (
    <Modal label="Кратко о непрочитанном" title={<><Sparkles size={16} /> Кратко о непрочитанном</>} hint={`${room.name} · ${count} непрочитанных · пересказ ИИ-ассистента сервера, проверяйте важное в сообщениях`} onClose={onClose} wide>
      <div className="talk-summary-text">
        {parts.map((p, i) => <div key={i}><TalkText text={p} /></div>)}
        {busy && <div className="muted talk-ai-wait"><Sparkles size={14} className="spin" /> Ассистент читает сообщения…</div>}
        {!busy && !error && parts.length === 0 && <div className="muted">Новых сообщений для пересказа нет.</div>}
      </div>
      {error && <div className="login-error" role="alert">{error}</div>}
      <div className="settings-actions"><button className="primary-button" onClick={onClose}>Закрыть</button></div>
    </Modal>
  )
}

const LANGS: [string, string][] = [['ru', 'Русский'], ['en', 'Английский'], ['de', 'Немецкий'], ['fr', 'Французский'], ['es', 'Испанский'], ['it', 'Итальянский'], ['zh', 'Китайский'], ['ja', 'Японский'], ['uk', 'Украинский'], ['be', 'Белорусский'], ['kk', 'Казахский']]

/** Перевод сообщения ИИ-переводчиком сервера. */
export function TranslateDialog({ text, author, mode, onClose }: { text: string; author: string; mode: string; onClose: () => void }) {
  const [to, setTo] = useState(() => { try { return localStorage.getItem('atom.talk.translate-to') || 'ru' } catch { return 'ru' } })
  const [result, setResult] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const run = useRef(0)
  useEffect(() => {
    try { localStorage.setItem('atom.talk.translate-to', to) } catch { /* без хранилища */ }
    const n = ++run.current
    setResult(''); setError(''); setBusy(true)
    void (async () => {
      try {
        const r = await api.talkTranslate(mode, text, to)
        const out = r.text ?? (r.taskId ? await waitTask(r.taskId, () => run.current === n) : '')
        if (run.current === n) setResult(out)
      } catch (e) {
        if (run.current === n) setError(errorText(e))
      } finally {
        if (run.current === n) setBusy(false)
      }
    })()
    return () => { run.current++ }
  }, [to, text, mode])
  return (
    <Modal label="Перевод сообщения" title={<><Languages size={16} /> Перевод</>} hint={`Сообщение ${author} · переводит сервер Nextcloud`} onClose={onClose} wide>
      <label className="login-field"><span>На язык</span>
        <select aria-label="Язык перевода" value={to} onChange={(e) => setTo(e.target.value)}>{LANGS.map(([v, l]) => <option key={v} value={v}>{l}</option>)}</select>
      </label>
      <div className="talk-translate-original muted">{text}</div>
      <div className="talk-translate-result">{busy ? <span className="muted talk-ai-wait"><Languages size={14} className="spin" /> Перевожу…</span> : <TalkText text={result} />}</div>
      {error && <div className="login-error" role="alert">{error}</div>}
      <div className="settings-actions">
        {result && <button className="secondary-button" onClick={() => void navigator.clipboard.writeText(result).catch(() => undefined)}>Скопировать перевод</button>}
        <button className="primary-button" onClick={onClose}>Закрыть</button>
      </div>
    </Modal>
  )
}

/** Комнаты для групп (модератор): создать, запустить, остановить, написать во все, удалить. */
export function BreakoutDialog({ room, rooms, notify, onChanged, onOpen, onClose }: {
  room: TalkRoom
  rooms: TalkRoom[]
  notify: (t: string) => void
  onChanged: () => Promise<void>
  onOpen: (token: string) => void
  onClose: () => void
}) {
  const [amount, setAmount] = useState(2)
  const [mode, setMode] = useState(1)
  const [message, setMessage] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const children = rooms.filter((r) => r.breakoutParent === room.token)
  const configured = room.breakoutMode > 0
  const started = room.breakoutStatus === 1
  async function act(action: string, value: Record<string, unknown>, done: string) {
    setBusy(true)
    setError('')
    try {
      await api.talkBreakout(room.token, action, value)
      notify(done)
      await onChanged()
      if (action === 'broadcast') setMessage('')
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }
  return (
    <Modal label="Комнаты для групп" title={<><Users size={16} /> Комнаты для групп</>} hint="Участники делятся на группы — каждая в своей беседе со своим звонком" onClose={onClose} wide>
      {!configured ? <>
        <label className="login-field"><span>Сколько комнат</span><input type="number" aria-label="Сколько комнат" min={1} max={20} value={amount} onChange={(e) => setAmount(Math.max(1, Math.min(20, Number(e.target.value) || 1)))} /></label>
        <label className="login-field"><span>Распределение</span>
          <select aria-label="Распределение" value={mode} onChange={(e) => setMode(Number(e.target.value))}>
            <option value={1}>Автоматически, поровну</option>
            <option value={3}>Каждый выбирает сам</option>
          </select>
        </label>
        <div className="settings-actions"><button className="primary-button" disabled={busy} onClick={() => void act('configure', { mode, amount }, `Созданы комнаты для групп: ${amount}`)}>Создать комнаты</button></div>
      </> : <>
        <div className="talk-breakout-list">
          {children.map((r) => <button key={r.token} type="button" className="talk-media-row file" onClick={() => { onClose(); onOpen(r.token) }}><Users size={15} /><span className="talk-media-meta"><b>{r.name}</b></span></button>)}
          {children.length === 0 && <div className="muted">Комнаты созданы — они появятся в списке бесед после обновления.</div>}
        </div>
        <div className="settings-actions">
          {started
            ? <button className="secondary-button" disabled={busy} onClick={() => void act('stop', {}, 'Комнаты для групп остановлены — все возвращаются в общую беседу')}>Остановить</button>
            : <button className="primary-button" disabled={busy} onClick={() => void act('start', {}, 'Комнаты для групп запущены')}>Запустить</button>}
          <button className="text-button danger" disabled={busy} onClick={() => { if (window.confirm('Удалить все комнаты для групп?')) void act('remove', {}, 'Комнаты для групп удалены') }}><Trash2 size={14} /> Удалить комнаты</button>
        </div>
        <label className="login-field"><span>Сообщение во все комнаты</span>
          <textarea rows={2} aria-label="Сообщение во все комнаты" value={message} onChange={(e) => setMessage(e.target.value)} placeholder="Например: через 5 минут возвращаемся" />
        </label>
        <div className="settings-actions"><button className="secondary-button" disabled={busy || !message.trim()} onClick={() => void act('broadcast', { message }, 'Сообщение отправлено во все комнаты')}>Отправить во все</button></div>
      </>}
      {error && <div className="login-error" role="alert">{error}</div>}
    </Modal>
  )
}
