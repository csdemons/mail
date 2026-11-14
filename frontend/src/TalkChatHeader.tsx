import { Star, Timer, Phone, ExternalLink, Images, Search, UserRound, Users, MoreHorizontal } from 'lucide-react'
import { expiryLabel } from './talkUtils'
import { openContextMenu, type MenuItem } from './PopupMenu'
import { personMenu, type PersonActions } from './TalkPeople'
import type { TalkRoom } from './types'

/** Шапка беседы: название, избранное, звонок, медиа, участники, поиск, меню. */
export interface TalkChatHeaderProps {
  room: TalkRoom
  toggleFavorite: (r: TalkRoom) => Promise<void>
  startCall: (r: TalkRoom, browser?: boolean) => Promise<void>
  showMedia: boolean
  setShowMedia: React.Dispatch<React.SetStateAction<boolean>>
  setShowPeople: React.Dispatch<React.SetStateAction<boolean>>
  searchOpen: boolean
  setSearchOpen: React.Dispatch<React.SetStateAction<boolean>>
  person: PersonActions
  showPeople: boolean
  roomMenu: (r: TalkRoom) => MenuItem[]
}

export default function TalkChatHeader({ room, toggleFavorite, startCall, showMedia, setShowMedia, setShowPeople, searchOpen, setSearchOpen, person, showPeople, roomMenu }: TalkChatHeaderProps) {
  return (
    <div className="calendar-toolbar">
      <strong className="calendar-title">{room.name}</strong>
      <button className={`icon-button soft talk-favorite ${room.favorite ? 'on' : ''}`} aria-pressed={room.favorite}
        aria-label={room.favorite ? 'Убрать из избранного' : 'В избранное'} title={room.favorite ? 'Убрать из избранного' : 'В избранное — беседа будет выше остальных'}
        onClick={() => void toggleFavorite(room)}><Star size={17} fill={room.favorite ? 'currentColor' : 'none'} /></button>
      <div className="reader-toolbar-spacer" />
      {room.messageExpiration > 0 && <span className="talk-expiry-badge" title="Сообщения в этой беседе исчезают"><Timer size={13} /> {expiryLabel(room.messageExpiration)}</span>}
      <button className="secondary-button talk-call" title="Звонок и демонстрация экрана — в окне программы" onClick={() => void startCall(room)}
        onContextMenu={(e) => openContextMenu(e, [{ label: 'Звонок в окне программы', icon: <Phone size={15} />, onClick: () => void startCall(room) }, { label: 'Звонок в браузере', icon: <ExternalLink size={15} />, onClick: () => void startCall(room, true) }])}><Phone size={15} /> Звонок</button>
      <button className={`icon-button soft talk-media-toggle ${showMedia ? 'active' : ''}`} aria-pressed={showMedia} aria-label="Медиа и файлы" title="Медиа и файлы беседы" onClick={() => { setShowMedia(!showMedia); setShowPeople(false) }}><Images size={17} /></button>
      <button className={`icon-button soft talk-search-toggle ${searchOpen ? 'active' : ''}`} aria-pressed={searchOpen} aria-label="Поиск по сообщениям" title="Поиск по сообщениям (Ctrl+F)" onClick={() => setSearchOpen(!searchOpen)}><Search size={17} /></button>
      {room.kind === 1 && room.peer && <button className="icon-button soft talk-profile-toggle" aria-label="Профиль собеседника" title="Профиль, доступность, письмо" onClick={(e) => openContextMenu(e, personMenu(room.peer as string, room.name, person, { inDirect: true }))}><UserRound size={17} /></button>}
      <button className={`icon-button soft talk-people-toggle ${showPeople ? 'active' : ''}`} aria-pressed={showPeople} aria-label="Участники" title="Участники беседы" onClick={() => { setShowPeople(!showPeople); setShowMedia(false) }}><Users size={17} /></button>
      <button className="icon-button soft" aria-label="Меню беседы" title="Ещё" onClick={(e) => openContextMenu(e, roomMenu(room))}><MoreHorizontal size={17} /></button>
    </div>
  )
}
