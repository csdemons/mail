import { useState } from 'react'
import { Search, MessageSquarePlus, RefreshCw, Users, Star, BellOff, Phone, Archive, ChevronDown, ChevronRight, CircleAlert, Lock, NotebookPen, AlarmClock } from 'lucide-react'
import { SkeletonRows } from './Skeleton'
import { openContextMenu, type MenuItem } from './PopupMenu'
import Avatar from './Avatar'
import type { TalkRoom, NcUser } from './types'

/** Список бесед чата и поиск сотрудников для новой личной беседы. */
export interface TalkRoomListProps {
  query: string
  setQuery: React.Dispatch<React.SetStateAction<string>>
  setNewChat: React.Dispatch<React.SetStateAction<boolean>>
  loadRooms: () => Promise<void>
  rooms: TalkRoom[] | null
  error: string
  visibleRooms: TalkRoom[]
  token: string | null
  setToken: React.Dispatch<React.SetStateAction<string | null>>
  roomMenu: (r: TalkRoom) => MenuItem[]
  peopleBusy: boolean
  people: NcUser[]
  chatWith: (u: NcUser) => Promise<void>
  peers: Set<string | null>
  /** Открыть «Заметки для себя»; нет — сервер их не поддерживает. */
  onNoteToSelf?: () => void
  /** Все напоминания (Talk 21.1). */
  onReminders?: () => void
}

export default function TalkRoomList({ onNoteToSelf, onReminders, query, setQuery, setNewChat, loadRooms, rooms, error, visibleRooms, token, setToken, roomMenu, peopleBusy, people, chatWith, peers }: TalkRoomListProps) {
  const [showArchive, setShowArchive] = useState(false)
  // Архивные беседы — отдельной свёрнутой группой внизу; при поиске — вместе со всеми.
  const searching = query.trim().length > 0
  const active = searching ? visibleRooms : visibleRooms.filter((r) => !r.archived)
  const archived = searching ? [] : visibleRooms.filter((r) => r.archived)
  const row = (r: TalkRoom) => (
    <button key={r.token} className={`message-row room-row ${r.token === token ? 'selected' : ''} ${r.unread ? 'unread' : ''} ${r.archived ? 'archived' : ''}`} onClick={() => setToken(r.token)}
      onContextMenu={(e) => openContextMenu(e, roomMenu(r))}>
      {r.kind === 1 && r.peer
        ? <Avatar id={r.peer} name={r.name} />
        : r.kind === 6 ? <span className="sender-avatar room-notes"><NotebookPen size={16} /></span>
          : <Avatar kind="room" id={r.token} name={r.name} fallback={r.kind === 1 ? undefined : <Users size={16} />} />}
      <div className="message-summary">
        <div className="message-topline"><span className="sender-name">{r.name}</span>
          {r.hasCall && <Phone size={13} className="room-call" aria-label="Идёт звонок" />}
          {r.important && <CircleAlert size={13} className="room-important" aria-label="Важная беседа" />}
          {r.sensitive && <Lock size={12} className="room-sensitive" aria-label="Конфиденциальная беседа" />}
          {r.favorite && <Star size={13} className="room-favorite" aria-label="Избранное" fill="currentColor" />}{r.notifyLevel === 3 && <BellOff size={12} className="room-muted" aria-label="Без уведомлений" />}{r.unread > 0 && <b className={`folder-count ${r.unreadMention ? 'mention' : ''}`}>{r.unread}</b>}</div>
        <div className="message-preview">{r.sensitive ? 'Конфиденциальная беседа' : (r.lastMessage ?? '')}</div>
      </div>
    </button>
  )
  return (
    <aside className="message-column talk-rooms">
      <div className="list-toolbar">
        <label className="search-box"><Search size={17} /><input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Беседы и сотрудники" /></label>
        {onReminders && <button className="icon-button soft" aria-label="Напоминания" title="Все напоминания о сообщениях" onClick={onReminders}><AlarmClock size={17} /></button>}
        {onNoteToSelf && <button className="icon-button soft" aria-label="Заметки для себя" title="Заметки для себя — беседа только с собой: ссылки, файлы, черновики" onClick={onNoteToSelf}><NotebookPen size={17} /></button>}
        <button className="icon-button soft" aria-label="Новая беседа" title="Новая беседа" onClick={() => setNewChat(true)}><MessageSquarePlus size={17} /></button>
        <button className="icon-button soft" aria-label="Обновить" onClick={() => void loadRooms()}><RefreshCw size={17} /></button>
      </div>
      <div className="message-list">
        {rooms === null && !error && <SkeletonRows variant="room" count={7} label="Загрузка бесед…" />}
        {active.map(row)}
        {archived.length > 0 && <>
          <button className="talk-archive-toggle" aria-expanded={showArchive} onClick={() => setShowArchive(!showArchive)}>
            {showArchive ? <ChevronDown size={14} /> : <ChevronRight size={14} />}<Archive size={14} /> Архив <span className="muted">{archived.length}</span>
            {archived.some((r) => r.unreadMention) && <b className="folder-count mention">@</b>}
          </button>
          {showArchive && archived.map(row)}
        </>}
        {query.trim().length >= 2 && <div className="talk-people">
          <div className="message-list-header"><span>Сотрудники</span>{peopleBusy && <span>поиск…</span>}</div>
          {!peopleBusy && people.length === 0 && <div className="empty-state compact"><span>Никого не найдено в Nextcloud</span></div>}
          {people.filter((u) => !visibleRooms.some((r) => r.kind === 1 && r.peer === u.id)).map((u) => (
            <button key={u.id} className="message-row room-row person-row" title="Написать" onClick={() => void chatWith(u)}>
              <Avatar id={u.id} name={u.label} />
              <div className="message-summary">
                <div className="message-topline"><span className="sender-name">{u.label}</span></div>
                <div className="message-preview">{peers.has(u.id) ? 'Открыть беседу' : (u.sub || 'Начать беседу')}</div>
              </div>
            </button>
          ))}
        </div>}
      </div>
    </aside>
  )
}
