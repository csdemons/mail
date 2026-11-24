import { useState } from 'react'
import { Paperclip, X, Pencil, Users, Smile, BarChart3, BellOff, Send, MessagesSquare, Clock } from 'lucide-react'
import { formatSize } from './api'
import Avatar from './Avatar'
import { EmojiPicker } from './Emoji'
import { VoiceRecorder } from './TalkExtras'
import DuePicker from './DuePicker'
import { snoozePresets } from './mailTools'
import { openContextMenu } from './PopupMenu'
import { useAutoGrow } from './useAutoGrow'
import type { PickedFile, TalkFeatures, TalkMessage, TalkMention } from './types'

/** Поле ввода сообщения: вложения, ответ и правка, упоминания, эмодзи, опрос, голосовое. */
export interface TalkComposeProps {
  /** Что умеет сервер Talk (ветки, отложенная отправка). */
  features: TalkFeatures
  /** Название новой ветки; null — обычное сообщение. */
  threadTitle: string | null
  setThreadTitle: (title: string | null) => void
  /** Отправить сообщение позже (в это время). */
  onSchedule: (at: Date) => void
  send: () => Promise<void>
  files: PickedFile[]
  previews: Record<string, string>
  sending: boolean
  dropFile: (id: string) => void
  replyTo: TalkMessage | null
  setReplyTo: React.Dispatch<React.SetStateAction<TalkMessage | null>>
  editing: TalkMessage | null
  cancelEdit: () => void
  pickFiles: () => Promise<void>
  mentions: TalkMention[]
  mentionSel: number
  pickMention: (mn: TalkMention) => void
  emojiOpen: boolean
  insertEmoji: (e: string) => void
  inputRef: React.RefObject<HTMLTextAreaElement>
  draft: string
  onDraftChange: (value: string, caret: number) => void
  onPaste: (e: React.ClipboardEvent<HTMLTextAreaElement>) => Promise<void>
  setMentionSel: React.Dispatch<React.SetStateAction<number>>
  setMentions: React.Dispatch<React.SetStateAction<TalkMention[]>>
  setEmojiOpen: React.Dispatch<React.SetStateAction<boolean>>
  setPollOpen: React.Dispatch<React.SetStateAction<boolean>>
  silent: boolean
  setSilent: React.Dispatch<React.SetStateAction<boolean>>
  notify: (t: string) => void
  sendVoice: (blob: Blob, ext: string) => Promise<void>
}

export default function TalkCompose({ features, threadTitle, setThreadTitle, onSchedule, send, files, previews, sending, dropFile, replyTo, setReplyTo, editing, cancelEdit, pickFiles, mentions, mentionSel, pickMention, emojiOpen, insertEmoji, inputRef, draft, onDraftChange, onPaste, setMentionSel, setMentions, setEmojiOpen, setPollOpen, silent, setSilent, notify, sendVoice }: TalkComposeProps) {
  const [pickTime, setPickTime] = useState(false)
  // Поле ввода растёт под текст: длинное сообщение видно целиком, выше предела — прокрутка.
  useAutoGrow(inputRef, draft)
  const canSchedule = features.scheduled && !editing && files.length === 0 && Boolean(draft.trim()) && !sending
  function scheduleMenu(e: React.MouseEvent) {
    openContextMenu(e, [
      ...snoozePresets().map((p) => ({ label: `${p.label} (${p.at.toLocaleString('ru-RU', { weekday: 'short', day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })})`, icon: <Clock size={15} />, onClick: () => onSchedule(p.at) })),
      { label: 'Выбрать дату и время…', icon: <Clock size={15} />, onClick: () => setPickTime(true) },
    ])
  }
  return (<>
    {/* Окно выбора времени — вне формы: его кнопки не должны отправлять сообщение. */}
    {pickTime && <DuePicker title="Отправить позже" hint="Сообщение уйдёт в это время, даже если программа будет закрыта" action="Отложить" onClose={() => setPickTime(false)} onPick={(d) => { setPickTime(false); onSchedule(d) }} />}
    <form className="talk-compose" onSubmit={(e) => { e.preventDefault(); void send() }}>
      {files.length > 0 && (
        <div className="talk-pending">
          {files.map((f) => (
            <span key={f.id} className={`talk-file ${previews[f.id] ? 'with-preview' : ''}`}>{previews[f.id] ? <img className="talk-paste-preview" src={previews[f.id]} alt="" /> : <Paperclip size={14} />}<span>{f.name}</span><small>{formatSize(f.size)}</small>
              {!sending && <button type="button" aria-label={`Убрать ${f.name}`} onClick={() => dropFile(f.id)}><X size={13} /></button>}
            </span>
          ))}
        </div>
      )}
      {replyTo && <div className="talk-quote replying"><b>{replyTo.author}</b> {replyTo.text.slice(0, 120)}<button type="button" aria-label="Отменить ответ" onClick={() => setReplyTo(null)}><X size={13} /></button></div>}
      {threadTitle !== null && !replyTo && !editing && <div className="talk-quote replying talk-new-thread"><MessagesSquare size={13} /> <b>Новая ветка:</b>
        <input aria-label="Название ветки" placeholder="Название ветки" value={threadTitle} maxLength={200} onChange={(e) => setThreadTitle(e.target.value)} />
        <button type="button" aria-label="Не создавать ветку" onClick={() => setThreadTitle(null)}><X size={13} /></button></div>}
      {editing && <div className="talk-quote replying editing"><Pencil size={13} /> <b>Изменение сообщения</b><button type="button" aria-label="Отменить изменение" onClick={cancelEdit}><X size={13} /></button></div>}
      <div className="talk-input-row">
        {!editing && <button type="button" className="icon-button soft talk-attach" aria-label="Прикрепить файл" title="Прикрепить файл" disabled={sending} onClick={() => void pickFiles()}><Paperclip size={18} /></button>}
        <div className="talk-input-wrap">
          {mentions.length > 0 && (
            <div className="talk-mentions popover">
              {mentions.map((mn, i) => (
                <button type="button" key={mn.insert} className={i === mentionSel ? 'active' : ''} onMouseDown={(e) => { e.preventDefault(); pickMention(mn) }}>
                  {mn.source === 'calls' ? <span className="sender-avatar"><Users size={13} /></span> : <Avatar id={mn.source === 'users' ? mn.insert.replace(/^@"?|"$/g, '') : null} name={mn.label} />}{mn.source === 'calls' ? 'Все участники' : mn.label}
                </button>
              ))}
            </div>
          )}
          {emojiOpen && <EmojiPicker onPick={insertEmoji} />}
          <textarea
            ref={inputRef}
            rows={2}
            value={draft}
            onChange={(e) => onDraftChange(e.target.value, e.target.selectionStart)}
            onPaste={(e) => void onPaste(e)}
            onKeyDown={(e) => {
              if (mentions.length > 0) {
                if (e.key === 'ArrowDown') { e.preventDefault(); setMentionSel((i) => (i + 1) % mentions.length); return }
                if (e.key === 'ArrowUp') { e.preventDefault(); setMentionSel((i) => (i - 1 + mentions.length) % mentions.length); return }
                if (e.key === 'Enter' || e.key === 'Tab') { e.preventDefault(); pickMention(mentions[mentionSel]); return }
                if (e.key === 'Escape') { setMentions([]); return }
              }
              if (e.key === 'Escape' && editing) { cancelEdit(); return }
              if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); void send() }
            }}
            placeholder={editing ? 'Новый текст… (Enter — сохранить, Esc — отмена)' : files.length ? 'Подпись к файлу… (Enter — отправить)' : 'Сообщение… (@ — упомянуть, Enter — отправить, Shift+Enter — новая строка)'}
          />
        </div>
        <button type="button" className={`icon-button soft talk-emoji ${emojiOpen ? 'active' : ''}`} aria-label="Эмодзи" title="Эмодзи" onClick={() => setEmojiOpen((v) => !v)}><Smile size={18} /></button>
        {features.threads && !editing && !replyTo && <button type="button" className={`icon-button soft talk-thread-new ${threadTitle !== null ? 'active' : ''}`} aria-pressed={threadTitle !== null} aria-label="Новая ветка" title="Начать ветку обсуждения с этим сообщением" onClick={() => setThreadTitle(threadTitle === null ? '' : null)}><MessagesSquare size={18} /></button>}
        {features.scheduled && !editing && <button type="button" className="icon-button soft talk-schedule" aria-label="Отправить позже" title={files.length ? 'Отложить можно только текст' : 'Отправить позже'} disabled={!canSchedule} onClick={scheduleMenu}><Clock size={18} /></button>}
        {!editing && <button type="button" className="icon-button soft talk-poll-new" aria-label="Опрос" title="Создать опрос" disabled={sending} onClick={() => setPollOpen(true)}><BarChart3 size={18} /></button>}
        {!editing && <button type="button" className={`icon-button soft talk-silent ${silent ? 'active' : ''}`} aria-pressed={silent} aria-label="Без звука" title={silent ? 'Отправится без звука уведомления — нажмите, чтобы отменить' : 'Отправить без звука уведомления'} onClick={() => setSilent(!silent)}><BellOff size={18} /></button>}
        {/* Кнопка записи на месте всегда (чтобы кнопки не прыгали); активна, когда поле пустое. */}
        {!editing && <VoiceRecorder disabled={sending || Boolean(draft.trim()) || files.length > 0} notify={notify} onRecorded={(b, ext) => void sendVoice(b, ext)} />}
        <button className="primary-button" type="submit" disabled={sending || (!draft.trim() && files.length === 0)} aria-label={editing ? 'Сохранить' : 'Отправить'}>{sending && files.length > 0 ? 'Загрузка…' : editing ? 'Сохранить' : <Send size={16} />}</button>
      </div>
    </form>
  </>)
}
