// Ветки беседы (Talk 22): список недавних веток и лента одной ветки с полем ответа.
import { useCallback, useEffect, useRef, useState } from 'react'
import { ArrowLeft, Bell, MessagesSquare, Pencil, Send, X } from 'lucide-react'

import { api, errorText } from './api'
import Avatar from './Avatar'
import { SkeletonRows } from './Skeleton'
import TalkText from './TalkText'
import { timeOf } from './talkUtils'
import type { TalkMessage, TalkThread } from './types'

/** Уровни уведомлений ветки — как у беседы в Talk. */
const NOTIFY_LEVELS: [number, string][] = [[0, 'Как в беседе'], [1, 'Все сообщения'], [2, 'Только упоминания'], [3, 'Не уведомлять']]

function preview(m: TalkMessage | null): string {
  if (!m) return ''
  const t = m.text || (m.files[0] ? `📎 ${m.files[0].name}` : '')
  return `${m.author}: ${t}`.slice(0, 140)
}

function ThreadMessage({ m }: { m: TalkMessage }) {
  return (
    <div className={`talk-thread-msg ${m.own ? 'own' : ''}`} data-message-id={m.id}>
      <Avatar id={m.authorId} name={m.author} />
      <div className="talk-thread-msg-body">
        <div className="talk-thread-msg-head"><b>{m.author}</b><small>{timeOf(m.timestamp)}</small>{m.edited && <small>изменено</small>}</div>
        {m.parentText && <div className="talk-quote"><b>{m.parentAuthor}</b> {m.parentText.slice(0, 120)}</div>}
        {m.text && <div className="talk-thread-msg-text"><TalkText text={m.text} /></div>}
        {m.files.map((f) => <div key={f.id} className="talk-thread-file">📎 {f.name}</div>)}
      </div>
    </div>
  )
}

export default function TalkThreadPanel({ token, threadId, readOnly, onOpen, onClose, notify }: {
  token: string
  /** Открытая ветка; null — список веток. */
  threadId: number | null
  readOnly: boolean
  onOpen: (id: number | null) => void
  onClose: () => void
  notify: (text: string) => void
}) {
  const [threads, setThreads] = useState<TalkThread[] | null>(null)
  const [thread, setThread] = useState<TalkThread | null>(null)
  const [messages, setMessages] = useState<TalkMessage[] | null>(null)
  const [draft, setDraft] = useState('')
  const [sending, setSending] = useState(false)
  const [error, setError] = useState('')
  /** Новое название ветки (правка в шапке панели). */
  const [title, setTitle] = useState<string | null>(null)
  const listRef = useRef<HTMLDivElement>(null)
  // Ответ сервера для прежней ветки (или беседы) не показывается в новой.
  const current = useRef('')
  current.current = `${token}:${threadId ?? ''}`

  const loadList = useCallback(async () => {
    const key = `${token}:`
    try {
      const list = await api.talkThreads(token)
      if (current.current === key) { setThreads(list); setError('') }
    } catch (e) { if (current.current === key) setError(errorText(e)) }
  }, [token])

  const loadThread = useCallback(async (id: number) => {
    const key = `${token}:${id}`
    try {
      const [info, chunk] = await Promise.all([api.talkThread(token, id), api.talkThreadHistory(token, id)])
      if (current.current !== key) return
      setThread(info)
      setMessages(chunk.messages.filter((m) => !m.system))
      setError('')
    } catch (e) { if (current.current === key) setError(errorText(e)) }
  }, [token])

  useEffect(() => {
    setError('')
    if (threadId === null) {
      setThreads(null)
      void loadList()
      return
    }
    setThread(null)
    setMessages(null)
    setDraft('')
    setTitle(null)
    void loadThread(threadId)
    // Новые ответы — раз в 10 секунд, пока ветка открыта.
    const t = window.setInterval(() => void loadThread(threadId), 10_000)
    return () => window.clearInterval(t)
  }, [threadId, loadList, loadThread])

  // Новые сообщения — внизу, как в беседе.
  useEffect(() => {
    const el = listRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [messages?.length])

  async function send() {
    if (threadId === null || !draft.trim() || sending) return
    setSending(true)
    try {
      await api.talkSend(token, draft, null, false, { threadId })
      setDraft('')
      await loadThread(threadId)
    } catch (e) {
      notify(`Не отправлено: ${errorText(e)}`)
    } finally {
      setSending(false)
    }
  }

  async function setLevel(level: number) {
    if (threadId === null) return
    try {
      await api.talkThreadNotify(token, threadId, level)
      setThread((t) => t && { ...t, notifyLevel: level })
      notify(level === 3 ? 'Уведомления ветки отключены' : 'Уведомления ветки изменены')
    } catch (e) { notify(`Не удалось: ${errorText(e)}`) }
  }

  async function rename() {
    const next = title?.trim()
    setTitle(null)
    if (threadId === null || !thread || !next || next === thread.title) return
    try {
      await api.talkThreadRename(token, threadId, next)
      setThread({ ...thread, title: next })
    } catch (e) { notify(`Не удалось переименовать: ${errorText(e)}`) }
  }

  return (
    <aside className="talk-people-panel talk-thread-panel" aria-label={threadId === null ? 'Ветки беседы' : 'Ветка'}>
      <div className="calendar-toolbar">
        {threadId !== null && <button className="icon-button soft" aria-label="Все ветки" title="Все ветки беседы" onClick={() => onOpen(null)}><ArrowLeft size={17} /></button>}
        {title !== null
          ? <form className="talk-thread-rename" onSubmit={(e) => { e.preventDefault(); void rename() }}>
              <input autoFocus aria-label="Название ветки" value={title} maxLength={200} onChange={(e) => setTitle(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Escape') setTitle(null) }} />
            </form>
          : <strong className="calendar-title" title={thread?.title}>{threadId === null ? 'Ветки' : (thread?.title || 'Ветка')}</strong>}
        <div className="reader-toolbar-spacer" />
        {threadId !== null && thread && !readOnly && title === null && <button className="icon-button soft" aria-label="Переименовать ветку" title="Переименовать" onClick={() => setTitle(thread.title)}><Pencil size={15} /></button>}
        <button className="icon-button soft" aria-label="Закрыть ветки" onClick={onClose}><X size={17} /></button>
      </div>
      {error && <div className="inline-warning" role="alert">{error}</div>}
      {threadId === null ? (
        <div className="talk-media-body">
          {threads === null && !error && <SkeletonRows variant="file" count={4} />}
          {threads?.length === 0 && <div className="muted talk-media-empty">Веток в беседе пока нет. Новую ветку начинают из поля ввода: кнопка «Новая ветка».</div>}
          {threads?.map((t) => (
            <button key={t.id} className="talk-media-row file talk-thread-row" onClick={() => onOpen(t.id)}>
              <MessagesSquare size={16} />
              <span className="talk-media-meta">
                <b>{t.title || preview(t.first)}</b>
                <small>{t.replies} {t.replies % 10 === 1 && t.replies % 100 !== 11 ? 'ответ' : t.replies % 10 >= 2 && t.replies % 10 <= 4 && (t.replies % 100 < 12 || t.replies % 100 > 14) ? 'ответа' : 'ответов'}{t.lastActivity > 0 ? ` · ${timeOf(t.lastActivity)}` : ''}</small>
                {t.last && <small className="talk-thread-last">{preview(t.last)}</small>}
              </span>
            </button>
          ))}
        </div>
      ) : (
        <>
          {thread && <label className="talk-thread-notify"><Bell size={13} /> Уведомления:
            <select aria-label="Уведомления ветки" value={thread.notifyLevel} onChange={(e) => void setLevel(Number(e.target.value))}>
              {NOTIFY_LEVELS.map(([v, l]) => <option key={v} value={v}>{l}</option>)}
            </select>
          </label>}
          <div className="talk-media-body talk-thread-messages" ref={listRef}>
            {messages === null && !error && <SkeletonRows variant="file" count={4} />}
            {messages?.map((m) => <ThreadMessage key={m.id} m={m} />)}
          </div>
          {!readOnly && <form className="talk-thread-compose" onSubmit={(e) => { e.preventDefault(); void send() }}>
            <textarea rows={2} aria-label="Ответ в ветке" placeholder="Ответить в ветке… (Enter — отправить)" value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); void send() } }} />
            <button className="primary-button" type="submit" aria-label="Отправить в ветку" disabled={sending || !draft.trim()}><Send size={15} /></button>
          </form>}
        </>
      )}
    </aside>
  )
}
