import { useEffect, useState } from 'react'
import { Crown, LogOut, MoreHorizontal, Search, Settings2, Shield, ShieldOff, Trash2, UserMinus, UserPlus, X } from 'lucide-react'
import { api, errorText, TYPING_PAUSE_MS } from './api'
import Avatar from './Avatar'
import { openContextMenu, type MenuItem } from './PopupMenu'
import { SkeletonRows } from './Skeleton'
import type { NcUser, TalkMessage, TalkParticipant, TalkRoom } from './types'

export function roleLabel(role: number): string {
  switch (role) {
    case 1: return 'владелец'
    case 2: return 'модератор'
    case 4: return 'гость'
    case 6: return 'гость-модератор'
    default: return ''
  }
}

const STATUS: Record<string, string> = { online: 'в сети', away: 'отошёл', dnd: 'не беспокоить', busy: 'занят', offline: 'не в сети' }

/** Боковая панель «Участники» — как в Talk: роли, статус, управление для модератора. */
export function RoomPeople({ room, notify, onClose, onRoomChanged, onSettings }: {
  room: TalkRoom
  notify: (t: string) => void
  onClose: () => void
  onRoomChanged: () => void
  onSettings: () => void
}) {
  const [list, setList] = useState<TalkParticipant[] | null>(null)
  const [error, setError] = useState('')
  const [adding, setAdding] = useState(false)
  const [q, setQ] = useState('')
  const [found, setFound] = useState<NcUser[]>([])

  function load() {
    api.talkParticipants(room.token).then((l) => { setList(l); setError('') }).catch((e) => setError(errorText(e)))
  }
  useEffect(() => { setList(null); setAdding(false); load() /* eslint-disable-next-line react-hooks/exhaustive-deps */ }, [room.token])

  useEffect(() => {
    if (!adding || q.trim().length < 2) { setFound([]); return }
    let cancelled = false
    const t = window.setTimeout(() => {
      api.ncSearchUsers(q.trim()).then((u) => { if (!cancelled) setFound(u) }).catch(() => undefined)
    }, TYPING_PAUSE_MS)
    return () => { cancelled = true; window.clearTimeout(t) }
  }, [q, adding])

  async function act(action: string, value: Record<string, unknown>, done: string) {
    try {
      await api.talkManage(room.token, action, value)
      notify(done)
      load()
      onRoomChanged()
    } catch (e) {
      notify(`Не удалось: ${errorText(e)}`)
    }
  }

  function menuFor(p: TalkParticipant): MenuItem[] {
    if (!room.moderator || p.me || p.role === 1) return []
    const isMod = p.role === 2 || p.role === 6
    return [
      isMod
        ? { label: 'Снять права модератора', icon: <ShieldOff size={15} />, onClick: () => void act('demote', { attendeeId: p.attendeeId }, `${p.name} больше не модератор`) }
        : { label: 'Назначить модератором', icon: <Shield size={15} />, onClick: () => void act('promote', { attendeeId: p.attendeeId }, `${p.name} — модератор`) },
      { separator: true },
      { label: 'Удалить из беседы', icon: <UserMinus size={15} />, danger: true, onClick: () => { if (window.confirm(`Удалить ${p.name} из беседы «${room.name}»?`)) void act('remove', { attendeeId: p.attendeeId }, `${p.name} удалён из беседы`) } },
    ]
  }

  const present = new Set((list ?? []).filter((p) => p.actorType === 'users').map((p) => p.actorId))
  return (
    <aside className="talk-people-panel" aria-label="Участники беседы">
      <div className="calendar-toolbar">
        <strong className="calendar-title">Участники{list ? ` · ${list.length}` : ''}</strong>
        <div className="reader-toolbar-spacer" />
        {room.moderator && room.kind !== 1 && <button className="icon-button soft" aria-label="Добавить участника" title="Добавить участника" onClick={() => setAdding(!adding)}><UserPlus size={17} /></button>}
        <button className="icon-button soft" aria-label="Настройки беседы" title="Настройки беседы" onClick={onSettings}><Settings2 size={17} /></button>
        <button className="icon-button soft" aria-label="Закрыть список участников" onClick={onClose}><X size={17} /></button>
      </div>
      {room.description && <p className="talk-room-description">{room.description}</p>}
      {adding && (
        <div className="talk-people-add">
          <label className="search-box"><Search size={15} /><input autoFocus aria-label="Найти сотрудника" value={q} onChange={(e) => setQ(e.target.value)} placeholder="Имя, логин или e-mail" /></label>
          {found.filter((u) => !present.has(u.id)).map((u) => (
            <button key={u.id} className="message-row room-row person-row" onClick={() => { setQ(''); void act('add', { id: u.id, source: 'users' }, `${u.label} добавлен в беседу`) }}>
              <Avatar id={u.id} name={u.label} />
              <div className="message-summary"><span className="sender-name">{u.label}</span>{u.sub && <span className="message-preview">{u.sub}</span>}</div>
            </button>
          ))}
        </div>
      )}
      {error && <div className="inline-warning" role="alert">{error}</div>}
      <div className="talk-people-list">
        {list === null && !error && <SkeletonRows variant="room" count={5} label="Загрузка участников…" />}
        {(list ?? []).map((p) => {
          const items = menuFor(p)
          return (
            <div key={p.attendeeId} className="talk-person" data-actor={p.actorId}
              onContextMenu={(e) => { if (items.length) openContextMenu(e, items) }}>
              <span className="talk-person-avatar">
                {p.actorType === 'users' ? <Avatar id={p.actorId} name={p.name} /> : <Avatar id={null} name={p.name} />}
                {p.status && <i className={`presence ${p.status}`} title={STATUS[p.status] ?? p.status} />}
              </span>
              <div className="message-summary">
                <span className="sender-name">{p.name}{p.me && <small className="muted"> (вы)</small>}</span>
                <span className="message-preview">
                  {roleLabel(p.role) && <b className={`talk-role role-${p.role}`}>{p.role === 1 ? <Crown size={11} /> : <Shield size={11} />} {roleLabel(p.role)}</b>}
                  {p.statusMessage || (p.status ? STATUS[p.status] ?? '' : '')}
                </span>
              </div>
              {items.length > 0 && <button className="icon-button soft" aria-label={`Действия: ${p.name}`} title="Действия" onClick={(e) => openContextMenu(e, items)}><MoreHorizontal size={16} /></button>}
            </div>
          )
        })}
      </div>
    </aside>
  )
}

/** Настройки беседы: название, описание, режимы (модератор) и свои уведомления, выход. */
export function RoomSettings({ room, notify, onClose, onChanged, onGone }: {
  room: TalkRoom
  notify: (t: string) => void
  onClose: () => void
  onChanged: () => void
  onGone: () => void
}) {
  const [name, setName] = useState(room.name)
  const [description, setDescription] = useState(room.description)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const group = room.kind !== 1

  async function run(steps: [string, Record<string, unknown>][], done: string, gone = false) {
    setBusy(true)
    setError('')
    try {
      for (const [action, value] of steps) await api.talkManage(room.token, action, value)
      notify(done)
      if (gone) onGone(); else onChanged()
      onClose()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  function save() {
    const steps: [string, Record<string, unknown>][] = []
    if (room.moderator && group && name.trim() && name.trim() !== room.name) steps.push(['rename', { name: name.trim() }])
    if (room.moderator && group && description !== room.description) steps.push(['description', { description }])
    if (steps.length === 0) { onClose(); return }
    void run(steps, 'Настройки беседы сохранены')
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget && !busy) onClose() }}>
      <div className="settings-modal talk-settings-modal" role="dialog" aria-label="Настройки беседы">
        <div className="compose-header"><div><strong>Настройки беседы</strong><small>{room.moderator ? 'Вы управляете этой беседой' : 'Изменять беседу могут владелец и модераторы'}</small></div><button className="icon-button" onClick={onClose} aria-label="Закрыть"><X size={18} /></button></div>
        {group && <label className="login-field"><span>Название</span><input value={name} disabled={!room.moderator} onChange={(e) => setName(e.target.value)} /></label>}
        {group && <label className="login-field"><span>Описание</span><textarea rows={3} value={description} disabled={!room.moderator} onChange={(e) => setDescription(e.target.value)} placeholder={room.moderator ? 'О чём эта беседа' : ''} /></label>}
        {room.moderator && group && (
          <div className="settings-section notify-settings">
            <strong>Режим</strong>
            <label><input type="checkbox" checked={room.readOnly} disabled={busy} onChange={(e) => void run([['read_only', { on: e.target.checked }]], e.target.checked ? 'Беседа только для чтения' : 'В беседе снова можно писать')} /> Только чтение — писать никто не может</label>
            <label><input type="checkbox" checked={room.kind === 3} disabled={busy} onChange={(e) => void run([['public', { on: e.target.checked }]], e.target.checked ? 'Открыт доступ по ссылке' : 'Доступ по ссылке закрыт')} /> Доступ по ссылке (гости)</label>
          </div>
        )}
        <div className="settings-section notify-settings">
          <strong>Мои уведомления</strong>
          <select aria-label="Уведомления" value={room.notifyLevel} disabled={busy} onChange={(e) => void run([['notify', { level: Number(e.target.value) }]], 'Уведомления беседы изменены')}>
            <option value={0}>По умолчанию</option>
            <option value={1}>Обо всех сообщениях</option>
            <option value={2}>Только при упоминании</option>
            <option value={3}>Не уведомлять</option>
          </select>
        </div>
        {error && <div className="login-error" role="alert">{error}</div>}
        <div className="settings-actions talk-settings-actions">
          {room.canLeave && <button className="text-button" disabled={busy} onClick={() => { if (window.confirm(`Выйти из беседы «${room.name}»?`)) void run([['leave', {}]], `Вы вышли из беседы «${room.name}»`, true) }}><LogOut size={15} /> Выйти</button>}
          {room.canDelete && <button className="text-button danger" disabled={busy} onClick={() => { if (window.confirm(`Удалить беседу «${room.name}» для всех участников? Сообщения будут удалены.`)) void run([['delete', {}]], `Беседа «${room.name}» удалена`, true) }}><Trash2 size={15} /> Удалить беседу</button>}
          <div className="reader-toolbar-spacer" />
          <button className="text-button" onClick={onClose}>Отмена</button>
          {room.moderator && group && <button className="primary-button" disabled={busy} onClick={save}>Сохранить</button>}
        </div>
      </div>
    </div>
  )
}

/** Переслать сообщение в другую беседу (текстом, с указанием автора). */
export function ForwardDialog({ rooms, message, notify, onClose }: { rooms: TalkRoom[]; message: TalkMessage; notify: (t: string) => void; onClose: () => void }) {
  const [q, setQ] = useState('')
  const [busy, setBusy] = useState(false)
  const list = rooms.filter((r) => !r.readOnly && r.name.toLowerCase().includes(q.trim().toLowerCase()))
  const files = message.files.map((f) => `📎 ${f.name}`).join('\n')
  const text = [message.text, files].filter(Boolean).join('\n')

  async function send(r: TalkRoom) {
    setBusy(true)
    try {
      await api.talkSend(r.token, `↪ ${message.author}:\n${text}`)
      notify(`Переслано в «${r.name}»`)
      onClose()
    } catch (e) {
      notify(`Не удалось переслать: ${errorText(e)}`)
      setBusy(false)
    }
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget && !busy) onClose() }}>
      <div className="settings-modal talk-new-modal" role="dialog" aria-label="Переслать сообщение">
        <div className="compose-header"><div><strong>Переслать сообщение</strong><small>{message.author}: {text.slice(0, 80)}</small></div><button className="icon-button" onClick={onClose} aria-label="Закрыть"><X size={18} /></button></div>
        <label className="search-box"><Search size={17} /><input autoFocus value={q} onChange={(e) => setQ(e.target.value)} placeholder="Беседа" /></label>
        <div className="talk-user-list">
          {list.map((r) => (
            <button key={r.token} className="message-row" disabled={busy} onClick={() => void send(r)}>
              {r.kind === 1 && r.peer ? <Avatar id={r.peer} name={r.name} /> : <Avatar kind="room" id={r.token} name={r.name} />}
              <div className="message-summary"><span className="sender-name">{r.name}</span></div>
            </button>
          ))}
        </div>
      </div>
    </div>
  )
}
