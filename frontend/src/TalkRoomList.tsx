import { Search, MessageSquarePlus, RefreshCw, Users, Star, BellOff } from 'lucide-react'
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
}

export default function TalkRoomList({ query, setQuery, setNewChat, loadRooms, rooms, error, visibleRooms, token, setToken, roomMenu, peopleBusy, people, chatWith, peers }: TalkRoomListProps) {
  return (
    <aside className="message-column talk-rooms">
      <div className="list-toolbar">
        <label className="search-box"><Search size={17} /><input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Беседы и сотрудники" /></label>
        <button className="icon-button soft" aria-label="Новая беседа" title="Новая беседа" onClick={() => setNewChat(true)}><MessageSquarePlus size={17} /></button>
        <button className="icon-button soft" aria-label="Обновить" onClick={() => void loadRooms()}><RefreshCw size={17} /></button>
      </div>
      <div className="message-list">
        {rooms === null && !error && <SkeletonRows variant="room" count={7} label="Загрузка бесед…" />}
        {visibleRooms.map((r) => (
          <button key={r.token} className={`message-row room-row ${r.token === token ? 'selected' : ''} ${r.unread ? 'unread' : ''}`} onClick={() => setToken(r.token)}
            onContextMenu={(e) => openContextMenu(e, roomMenu(r))}>
            {r.kind === 1 && r.peer
              ? <Avatar id={r.peer} name={r.name} />
              : <Avatar kind="room" id={r.token} name={r.name} fallback={r.kind === 1 ? undefined : <Users size={16} />} />}
            <div className="message-summary">
              <div className="message-topline"><span className="sender-name">{r.name}</span>{r.favorite && <Star size={13} className="room-favorite" aria-label="Избранное" fill="currentColor" />}{r.notifyLevel === 3 && <BellOff size={12} className="room-muted" aria-label="Без уведомлений" />}{r.unread > 0 && <b className={`folder-count ${r.unreadMention ? 'mention' : ''}`}>{r.unread}</b>}</div>
              <div className="message-preview">{r.lastMessage ?? ''}</div>
            </div>
          </button>
        ))}
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
