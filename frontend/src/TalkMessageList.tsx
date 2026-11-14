import { SkeletonChat } from './Skeleton'
import { timeOf, isImage, TalkThumb, viewable } from './talkUtils'
import { openContextMenu, type MenuItem } from './PopupMenu'
import { personMenu, type PersonActions } from './TalkPeople'
import Avatar from './Avatar'
import TalkText from './TalkText'
import { PollCard, VoicePlayer, firstUrl, LinkCard } from './TalkExtras'
import { api, formatSize } from './api'
import { Download, ExternalLink, FileText, Timer, CheckCheck, Check, SmilePlus, CornerUpLeft, KanbanSquare, Pencil, Trash2, MoreHorizontal, MessageCircle } from 'lucide-react'
import { EmojiPicker, QUICK_REACTIONS } from './Emoji'
import type { TalkMessage, TalkRoom, TalkFile } from './types'

/** Лента сообщений беседы: даты, сообщения, файлы, реакции, отметки прочтения. */
export interface TalkMessageListProps {
  listRef: React.RefObject<HTMLDivElement>
  stickToBottom: React.MutableRefObject<boolean>
  loading: boolean
  messages: TalkMessage[]
  hasOlder: boolean
  olderBusy: boolean
  loadOlder: () => Promise<void>
  flash: number | null
  messageMenu: (m: TalkMessage, r: TalkRoom) => MenuItem[]
  room: TalkRoom
  person: PersonActions
  notify: (t: string) => void
  openFile: (f: TalkFile) => void
  saveFile: (f: TalkFile) => Promise<void>
  react: (m: TalkMessage, emoji: string) => Promise<void>
  readMark: number
  setReactFor: React.Dispatch<React.SetStateAction<number | null>>
  reactFor: number | null
  setReplyTo: React.Dispatch<React.SetStateAction<TalkMessage | null>>
  setEditing: React.Dispatch<React.SetStateAction<TalkMessage | null>>
  inputRef: React.RefObject<HTMLTextAreaElement>
  setCardFrom: React.Dispatch<React.SetStateAction<TalkMessage | null>>
  startEdit: (m: TalkMessage) => void
  remove: (m: TalkMessage) => Promise<void>
  reactFull: boolean
  setReactFull: React.Dispatch<React.SetStateAction<boolean>>
}

export default function TalkMessageList({ listRef, stickToBottom, loading, messages, hasOlder, olderBusy, loadOlder, flash, messageMenu, room, person, notify, openFile, saveFile, react, readMark, setReactFor, reactFor, setReplyTo, setEditing, inputRef, setCardFrom, startEdit, remove, reactFull, setReactFull }: TalkMessageListProps) {
  return (
    <div className="talk-messages" ref={listRef}
      onScroll={(e) => { const el = e.currentTarget; stickToBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80 }}>
      {loading && messages.length === 0 && <SkeletonChat />}
      {hasOlder && !loading && <button className="text-button talk-older" disabled={olderBusy} onClick={() => void loadOlder()}>{olderBusy ? 'Загрузка…' : 'Показать более ранние сообщения'}</button>}
      {messages.map((m) => m.system ? (
        <div key={m.id} className="talk-system">{m.text} · {timeOf(m.timestamp)}</div>
      ) : (
        <div key={m.id} className={`talk-msg ${m.own ? 'own' : ''} ${m.deleted ? 'deleted' : ''} ${flash === m.id ? 'flash' : ''}`} data-id={m.id}
          onContextMenu={(e) => { const items = messageMenu(m, room); if (items.length) openContextMenu(e, items) }}>
          {!m.own && <span className="talk-author-avatar" role="button" tabIndex={-1} title={m.author} onClick={(e) => m.authorId && openContextMenu(e, personMenu(m.authorId, m.author, person, { inDirect: room.kind === 1 }))}><Avatar id={m.authorId} name={m.author} /></span>}
          <div className="talk-bubble">
            {!m.own && <button type="button" className="talk-author" onClick={(e) => m.authorId && openContextMenu(e, personMenu(m.authorId, m.author, person, { inDirect: room.kind === 1 }))}>{m.author}</button>}
            {m.parentText && !m.deleted && <div className="talk-quote"><b>{m.parentAuthor}</b> {m.parentText}</div>}
            {m.text && (m.deleted ? <div className="talk-text talk-deleted">{m.text}</div> : <TalkText text={m.text} />)}
            {m.poll && !m.deleted && <PollCard token={room.token} id={m.poll.id} question={m.poll.question} notify={notify} />}
            {m.voice && !m.deleted && m.files[0] && <VoicePlayer file={m.files[0]} load={(f) => api.talkFileData(f.path as string)} />}
            {!m.deleted && m.files.length === 0 && !m.poll && firstUrl(m.text) && <LinkCard url={firstUrl(m.text) as string} />}
            {m.files.length > 0 && !m.voice && (
              <div className="talk-files">
                {m.files.map((f) => (isImage(f) && f.id && (f.preview || f.path)) ? (
                  <div key={f.id} className="talk-image">
                    <button className="talk-image-open" aria-label={`Открыть ${f.name}`} title="Открыть" onClick={() => openFile(f)}><TalkThumb file={f} /></button>
                    <div className="talk-image-bar">
                      <span title={f.name}>{f.name}</span>{f.size > 0 && <small>{formatSize(f.size)}</small>}
                      {f.path && <button className="icon-button soft" aria-label={`Скачать ${f.name}`} title="Сохранить на компьютер" onClick={() => void saveFile(f)}><Download size={14} /></button>}
                      <button className="icon-button soft" aria-label={`Открыть ${f.name} в Nextcloud`} title="Открыть в Nextcloud" onClick={() => void api.ncOpen(`f/${f.id}`)}><ExternalLink size={14} /></button>
                    </div>
                  </div>
                ) : (
                  <div key={f.id || f.name} className="talk-file-row">
                    <button className="talk-file" title={viewable(f) ? 'Открыть' : 'Открыть в Nextcloud'} onClick={() => openFile(f)}>
                      <FileText size={15} /><span>{f.name}</span>{f.size > 0 && <small>{formatSize(f.size)}</small>}
                    </button>
                    {f.path && <button className="icon-button soft" aria-label={`Скачать ${f.name}`} title="Сохранить на компьютер" onClick={() => void saveFile(f)}><Download size={14} /></button>}
                  </div>
                ))}
              </div>
            )}
            {m.reactions.length > 0 && (
              <div className="talk-reactions">
                {m.reactions.map((r) => (
                  <button key={r.emoji} className={`talk-reaction ${r.own ? 'own' : ''}`} disabled={room.readOnly} title={r.own ? 'Убрать реакцию' : 'Поставить реакцию'} onClick={() => void react(m, r.emoji)}>
                    {r.emoji} <span>{r.count}</span>
                  </button>
                ))}
              </div>
            )}
            <div className="talk-time">{m.expires > 0 && <Timer size={11} className="talk-expires" aria-label="Исчезающее сообщение" />}{m.edited && !m.deleted && <span className="talk-edited">изменено</span>}{timeOf(m.timestamp)}
              {m.own && !m.deleted && (m.id <= readMark
                ? <CheckCheck size={14} className="talk-tick read" aria-label="Прочитано" />
                : <Check size={14} className="talk-tick" aria-label="Отправлено" />)}</div>
            {!room.readOnly && !m.deleted && (
              <div className="talk-actions">
                <button aria-label="Реакция" title="Реакция" onClick={() => setReactFor(reactFor === m.id ? null : m.id)}><SmilePlus size={14} /></button>
                <button aria-label="Ответить" title="Ответить" onClick={() => { setReplyTo(m); setEditing(null); inputRef.current?.focus() }}><CornerUpLeft size={14} /></button>
                <button aria-label="Карточка Deck" title="Создать карточку Deck" onClick={() => setCardFrom(m)}><KanbanSquare size={14} /></button>
                {m.editText !== null && <button aria-label="Изменить" title="Изменить" onClick={() => startEdit(m)}><Pencil size={14} /></button>}
                {m.own && <button aria-label="Удалить" title="Удалить" onClick={() => void remove(m)}><Trash2 size={14} /></button>}
                <button aria-label="Ещё" title="Ещё действия" onClick={(e) => openContextMenu(e, messageMenu(m, room))}><MoreHorizontal size={14} /></button>
              </div>
            )}
            {reactFor === m.id && (
              reactFull
                ? <EmojiPicker className="talk-react-full" label="Реакция" onPick={(e) => { setReactFor(null); void react(m, e) }} />
                : <div className="talk-react-picker popover">
                  {QUICK_REACTIONS.map((e) => <button key={e} aria-label={`Реакция ${e}`} onClick={() => void react(m, e)}>{e}</button>)}
                  <button aria-label="Другая реакция" title="Все эмодзи" onClick={() => setReactFull(true)}>＋</button>
                </div>
            )}
          </div>
        </div>
      ))}
      {!loading && messages.length === 0 && <div className="empty-state"><MessageCircle size={30} /><strong>Сообщений пока нет</strong></div>}
    </div>
  )
}
