import { Pin, EyeOff, PinOff, X } from 'lucide-react'
import { openContextMenu } from './PopupMenu'
import type { TalkMessage, TalkRoom } from './types'

/** Полоска закреплённых сообщений вверху беседы. */
export interface TalkPinnedBarProps {
  jumpTo: (id: number) => Promise<void>
  pinned: TalkMessage[]
  room: TalkRoom
  unpin: (m: TalkMessage, onlyMe: boolean) => Promise<void>
}

export default function TalkPinnedBar({ jumpTo, pinned, room, unpin }: TalkPinnedBarProps) {
  return (
    <div className="talk-pinned" role="region" aria-label="Закреплённое сообщение">
      <Pin size={15} />
      <button type="button" className="talk-pinned-text" title="Перейти к сообщению" onClick={() => void jumpTo(pinned[0].id)}>
        <b>{pinned[0].author}</b> {pinned[0].text || pinned[0].files[0]?.name}
        {pinned[0].pinnedUntil > 0 && <small> · до {new Date(pinned[0].pinnedUntil * 1000).toLocaleString('ru-RU', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })}</small>}
      </button>
      {pinned.length > 1 && <button type="button" className="text-button" onClick={(e) => openContextMenu(e, pinned.map((m) => ({ label: `${m.author}: ${(m.text || m.files[0]?.name || '').slice(0, 60)}`, icon: <Pin size={15} />, onClick: () => void jumpTo(m.id) })))}>ещё {pinned.length - 1}</button>}
      <button type="button" className="icon-button soft" aria-label="Скрыть закреплённое" title="Скрыть"
        onClick={(e) => room.moderator
          ? openContextMenu(e, [{ label: 'Скрыть у себя', icon: <EyeOff size={15} />, onClick: () => void unpin(pinned[0], true) }, { label: 'Открепить для всех', icon: <PinOff size={15} />, onClick: () => void unpin(pinned[0], false) }])
          : void unpin(pinned[0], true)}><X size={15} /></button>
    </div>
  )
}
