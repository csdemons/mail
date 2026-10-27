import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { CornerUpLeft, MessageCircle, Phone, RefreshCw, Search, Send, Users, X } from 'lucide-react'

import { api, errorText } from './api'
import type { TalkMessage, TalkRoom } from './types'

function initials(name: string) {
  const p = name.trim().split(/\s+/)
  return ((p[0]?.[0] ?? '') + (p[1]?.[0] ?? '')).toUpperCase() || '?'
}

function timeOf(ts: number) {
  const d = new Date(ts * 1000)
  const today = new Date()
  return d.toDateString() === today.toDateString()
    ? d.toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' })
    : d.toLocaleString('ru-RU', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })
}

export default function TalkView({ onUnread }: { onUnread: (n: number) => void }) {
  const [rooms, setRooms] = useState<TalkRoom[] | null>(null)
  const [token, setToken] = useState<string | null>(null)
  const [messages, setMessages] = useState<TalkMessage[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [draft, setDraft] = useState('')
  const [replyTo, setReplyTo] = useState<TalkMessage | null>(null)
  const [query, setQuery] = useState('')
  const [sending, setSending] = useState(false)
  const listRef = useRef<HTMLDivElement>(null)
  // Номер «сессии» опроса: при смене беседы старый цикл останавливается.
  const pollSession = useRef(0)

  const loadRooms = useCallback(async () => {
    try {
      const list = await api.talkRooms()
      setRooms(list)
      setToken((t) => t ?? list[0]?.token ?? null)
    } catch (e) {
      setError(errorText(e))
    }
  }, [onUnread])

  useEffect(() => {
    void loadRooms()
    const t = setInterval(() => void loadRooms(), 30000)
    return () => clearInterval(t)
  }, [loadRooms])

  useEffect(() => {
    if (!token) return
    const session = ++pollSession.current
    let lastId = 0
    setMessages([])
    setReplyTo(null)
    setLoading(true)
    void (async () => {
      try {
        const hist = await api.talkHistory(token)
        if (pollSession.current !== session) return
        setMessages(hist)
        lastId = hist[hist.length - 1]?.id ?? 0
        setRooms((list) => list?.map((r) => (r.token === token ? { ...r, unread: 0 } : r)) ?? list)
      } catch (e) {
        setError(errorText(e))
      } finally {
        setLoading(false)
      }
      // Long-poll: сервер держит запрос до 30 с и отвечает, как только появится сообщение.
      while (pollSession.current === session) {
        try {
          const fresh = await api.talkPoll(token, lastId)
          if (pollSession.current !== session) return
          if (fresh.length > 0) {
            lastId = fresh[fresh.length - 1].id
            setMessages((list) => {
              const known = new Set(list.map((m) => m.id))
              return [...list, ...fresh.filter((m) => !known.has(m.id))]
            })
          }
        } catch {
          await new Promise((r) => setTimeout(r, 5000))
        }
      }
    })()
    return () => { pollSession.current += 1 }
  }, [token])

  useEffect(() => {
    listRef.current?.scrollTo({ top: listRef.current.scrollHeight })
  }, [messages])

  // Значок на панели: непрочитанное во всех беседах, кроме открытой (её мы читаем).
  useEffect(() => {
    onUnread((rooms ?? []).filter((r) => r.token !== token).reduce((n, r) => n + r.unread, 0))
  }, [rooms, token, onUnread])

  async function send() {
    if (!token || !draft.trim()) return
    setSending(true)
    try {
      await api.talkSend(token, draft, replyTo?.id ?? null)
      setDraft('')
      setReplyTo(null)
    } catch (e) {
      setError(errorText(e))
    } finally {
      setSending(false)
    }
  }

  const room = rooms?.find((r) => r.token === token)
  const visibleRooms = useMemo(() => {
    const q = query.trim().toLowerCase()
    return (rooms ?? []).filter((r) => !q || r.name.toLowerCase().includes(q))
  }, [rooms, query])

  return (
    <div className="module-view talk-view">
      <aside className="message-column talk-rooms">
        <div className="list-toolbar">
          <label className="search-box"><Search size={17} /><input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Беседы" /></label>
          <button className="icon-button soft" aria-label="Обновить" onClick={() => void loadRooms()}><RefreshCw size={17} /></button>
        </div>
        <div className="message-list">
          {rooms === null && !error && <div className="empty-state"><span>Загрузка бесед…</span></div>}
          {visibleRooms.map((r) => (
            <button key={r.token} className={`message-row room-row ${r.token === token ? 'selected' : ''} ${r.unread ? 'unread' : ''}`} onClick={() => setToken(r.token)}>
              <div className="sender-avatar">{r.kind === 1 ? initials(r.name) : <Users size={16} />}</div>
              <div className="message-summary">
                <div className="message-topline"><span className="sender-name">{r.name}</span>{r.unread > 0 && <b className={`folder-count ${r.unreadMention ? 'mention' : ''}`}>{r.unread}</b>}</div>
                <div className="message-preview">{r.lastMessage ?? ''}</div>
              </div>
            </button>
          ))}
        </div>
      </aside>

      <main className="talk-main">
        {room ? (
          <>
            <div className="calendar-toolbar">
              <strong className="calendar-title">{room.name}</strong>
              <div className="reader-toolbar-spacer" />
              <button className="secondary-button" title="Звонок откроется в браузере" onClick={() => void api.ncOpen(`call/${room.token}`)}><Phone size={15} /> Звонок</button>
            </div>
            {error && <div className="inline-warning" role="alert">Talk: {error}</div>}
            <div className="talk-messages" ref={listRef}>
              {loading && <div className="empty-state"><span>Загрузка сообщений…</span></div>}
              {messages.map((m) => m.system ? (
                <div key={m.id} className="talk-system">{m.text} · {timeOf(m.timestamp)}</div>
              ) : (
                <div key={m.id} className={`talk-msg ${m.own ? 'own' : ''}`}>
                  {!m.own && <div className="sender-avatar">{initials(m.author)}</div>}
                  <div className="talk-bubble">
                    {!m.own && <div className="talk-author">{m.author}</div>}
                    {m.parentText && <div className="talk-quote"><b>{m.parentAuthor}</b> {m.parentText}</div>}
                    <div className="talk-text">{m.text}</div>
                    <div className="talk-time">{timeOf(m.timestamp)}
                      {!room.readOnly && <button className="talk-reply" aria-label="Ответить" title="Ответить" onClick={() => setReplyTo(m)}><CornerUpLeft size={12} /></button>}
                    </div>
                  </div>
                </div>
              ))}
              {!loading && messages.length === 0 && <div className="empty-state"><MessageCircle size={30} /><strong>Сообщений пока нет</strong></div>}
            </div>
            {room.readOnly ? <div className="muted talk-readonly">Беседа только для чтения</div> : (
              <form className="talk-compose" onSubmit={(e) => { e.preventDefault(); void send() }}>
                {replyTo && <div className="talk-quote replying"><b>{replyTo.author}</b> {replyTo.text.slice(0, 120)}<button type="button" aria-label="Отменить ответ" onClick={() => setReplyTo(null)}><X size={13} /></button></div>}
                <div className="talk-input-row">
                  <textarea
                    rows={2}
                    value={draft}
                    onChange={(e) => setDraft(e.target.value)}
                    onKeyDown={(e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); void send() } }}
                    placeholder="Сообщение… (Enter — отправить, Shift+Enter — новая строка)"
                  />
                  <button className="primary-button" type="submit" disabled={sending || !draft.trim()} aria-label="Отправить"><Send size={16} /></button>
                </div>
              </form>
            )}
          </>
        ) : (
          <div className="empty-reader">{error ? <><strong>Talk недоступен</strong><span>{error}</span></> : <><MessageCircle size={42} /><strong>Выберите беседу</strong></>}</div>
        )}
      </main>
    </div>
  )
}
