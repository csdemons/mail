import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import {
  CornerUpLeft, FileText, KanbanSquare, MessageCircle, MessageSquarePlus, Paperclip, Pencil, Phone, RefreshCw,
  Search, Send, Smile, SmilePlus, Trash2, Users, X,
} from 'lucide-react'

import { api, errorText, formatSize } from './api'
import { EmojiPicker, QUICK_REACTIONS } from './Emoji'
import TalkText from './TalkText'
import Avatar from './Avatar'
import type { DeckBoard, DeckStack, NcUser, PickedFile, TalkMention, TalkMessage, TalkRoom } from './types'

function timeOf(ts: number) {
  const d = new Date(ts * 1000)
  const today = new Date()
  return d.toDateString() === today.toDateString()
    ? d.toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' })
    : d.toLocaleString('ru-RU', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })
}

/** Новые сообщения добавляются, обновления (реакции, правки, удаление) заменяют уже показанные. */
function merge(list: TalkMessage[], incoming: TalkMessage[]): TalkMessage[] {
  const byId = new Map(list.map((m) => [m.id, m]))
  for (const m of incoming) {
    const cur = byId.get(m.id)
    if (cur || !m.update) byId.set(m.id, cur && m.update ? { ...m, update: false, parentText: m.parentText ?? cur.parentText, parentAuthor: m.parentAuthor ?? cur.parentAuthor } : { ...m, update: false })
  }
  return [...byId.values()].sort((a, b) => a.id - b.id)
}

export default function TalkView({ onUnread, onNotice }: { onUnread: (n: number) => void; onNotice?: (t: string) => void }) {
  const [rooms, setRooms] = useState<TalkRoom[] | null>(null)
  const [token, setToken] = useState<string | null>(null)
  const [messages, setMessages] = useState<TalkMessage[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [draft, setDraft] = useState('')
  const [replyTo, setReplyTo] = useState<TalkMessage | null>(null)
  const [editing, setEditing] = useState<TalkMessage | null>(null)
  const [query, setQuery] = useState('')
  const [sending, setSending] = useState(false)
  const [files, setFiles] = useState<PickedFile[]>([])
  const [emojiOpen, setEmojiOpen] = useState(false)
  const [reactFor, setReactFor] = useState<number | null>(null)
  const [mentions, setMentions] = useState<TalkMention[]>([])
  const [mentionSel, setMentionSel] = useState(0)
  const [cardFrom, setCardFrom] = useState<TalkMessage | null>(null)
  const [newChat, setNewChat] = useState(false)
  const listRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  // Показанное в поле «@Имя» → что отправить в Talk («@"id"»).
  const mentionMap = useRef(new Map<string, string>())
  // Номер «сессии» опроса: при смене беседы старый цикл останавливается.
  const pollSession = useRef(0)
  const stickToBottom = useRef(true)
  // Куда поставить курсор после программной правки текста (упоминание, эмодзи).
  const pendingCaret = useRef<number | null>(null)
  useLayoutEffect(() => {
    const el = inputRef.current
    if (el && pendingCaret.current !== null) {
      el.focus()
      el.setSelectionRange(pendingCaret.current, pendingCaret.current)
      pendingCaret.current = null
    }
  })

  const notify = useCallback((t: string) => { if (onNotice) onNotice(t); else setError(t) }, [onNotice])

  const loadRooms = useCallback(async () => {
    try {
      const list = await api.talkRooms()
      setRooms(list)
      setToken((t) => t ?? list[0]?.token ?? null)
    } catch (e) {
      setError(errorText(e))
    }
  }, [])

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
    setEditing(null)
    setDraft('')
    mentionMap.current.clear()
    setFiles((list) => { list.forEach((f) => void api.forgetAttachment(f.id)); return [] })
    setLoading(true)
    stickToBottom.current = true
    void (async () => {
      try {
        const hist = await api.talkHistory(token)
        if (pollSession.current !== session) return
        setMessages(merge([], hist.messages))
        lastId = hist.lastId
        setRooms((list) => list?.map((r) => (r.token === token ? { ...r, unread: 0 } : r)) ?? list)
      } catch (e) {
        setError(errorText(e))
      } finally {
        setLoading(false)
      }
      // Long-poll: сервер держит запрос до 30 с и отвечает, как только что-то изменится.
      while (pollSession.current === session) {
        try {
          const fresh = await api.talkPoll(token, lastId)
          if (pollSession.current !== session) return
          if (fresh.messages.length > 0) setMessages((list) => merge(list, fresh.messages))
          if (fresh.lastId > lastId) lastId = fresh.lastId
        } catch {
          await new Promise((r) => setTimeout(r, 5000))
        }
      }
    })()
    return () => { pollSession.current += 1 }
  }, [token])

  useEffect(() => {
    if (stickToBottom.current) listRef.current?.scrollTo({ top: listRef.current.scrollHeight })
  }, [messages])

  // Значок на панели: непрочитанное во всех беседах, кроме открытой (её мы читаем).
  useEffect(() => {
    onUnread((rooms ?? []).filter((r) => r.token !== token).reduce((n, r) => n + r.unread, 0))
  }, [rooms, token, onUnread])

  // Подсказка упоминаний: «@текст» перед курсором.
  const mentionQuery = useRef<string | null>(null)
  function onDraftChange(value: string, caret: number) {
    setDraft(value)
    const before = value.slice(0, caret)
    const m = /(^|\s)@([^\s@"]{0,30})$/.exec(before)
    mentionQuery.current = m ? m[2] : null
    if (!m || !token) { setMentions([]); return }
    const q = m[2]
    api.talkMentions(token, q)
      .then((list) => { if (mentionQuery.current === q) { setMentions(list); setMentionSel(0) } })
      .catch(() => setMentions([]))
  }

  function pickMention(mn: TalkMention) {
    const el = inputRef.current
    const caret = el?.selectionStart ?? draft.length
    const before = draft.slice(0, caret).replace(/@([^\s@"]{0,30})$/, '')
    const label = mn.source === 'calls' ? 'all' : mn.label
    const shown = `@${label} `
    mentionMap.current.set(`@${label}`, mn.insert)
    const next = before + shown + draft.slice(caret)
    pendingCaret.current = before.length + shown.length
    setDraft(next)
    setMentions([])
  }

  /** «@Имя» из подсказок → «@"id"» для Talk. */
  function withMentions(text: string) {
    let out = text
    for (const [shown, insert] of [...mentionMap.current.entries()].sort((a, b) => b[0].length - a[0].length)) {
      out = out.split(shown).join(insert)
    }
    return out
  }

  function insertEmoji(e: string) {
    const el = inputRef.current
    const start = el?.selectionStart ?? draft.length
    const end = el?.selectionEnd ?? draft.length
    const next = draft.slice(0, start) + e + draft.slice(end)
    pendingCaret.current = start + e.length
    setDraft(next)
  }

  async function pickFiles() {
    try {
      const picked = await api.pickAttachments()
      setFiles((list) => [...list, ...picked])
    } catch (e) {
      setError(errorText(e))
    }
  }

  function dropFile(id: string) {
    setFiles((list) => list.filter((f) => f.id !== id))
    void api.forgetAttachment(id)
  }

  async function send() {
    if (!token || (!draft.trim() && files.length === 0)) return
    setSending(true)
    setError('')
    setEmojiOpen(false)
    stickToBottom.current = true
    try {
      const text = withMentions(draft)
      if (editing) {
        await api.talkEdit(token, editing.id, text)
        setEditing(null)
      } else if (files.length > 0) {
        // Текст уходит подписью к первому файлу — как в веб-интерфейсе Talk.
        let caption = text
        for (const f of files) {
          await api.talkSendFile(token, f.id, caption, replyTo?.id ?? null)
          caption = ''
          setFiles((list) => list.filter((x) => x.id !== f.id))
        }
      } else {
        await api.talkSend(token, text, replyTo?.id ?? null)
      }
      setDraft('')
      setReplyTo(null)
      mentionMap.current.clear()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setSending(false)
    }
  }

  async function react(m: TalkMessage, emoji: string) {
    if (!token) return
    setReactFor(null)
    const mine = m.reactions.find((r) => r.emoji === emoji)?.own ?? false
    // Сразу показываем, сервер подтвердит обновлением через опрос.
    setMessages((list) => list.map((x) => {
      if (x.id !== m.id) return x
      const has = x.reactions.find((r) => r.emoji === emoji)
      const reactions = has
        ? x.reactions.map((r) => (r.emoji === emoji ? { ...r, own: !mine, count: r.count + (mine ? -1 : 1) } : r)).filter((r) => r.count > 0)
        : [...x.reactions, { emoji, count: 1, own: true }]
      return { ...x, reactions }
    }))
    try {
      await api.talkReact(token, m.id, emoji, !mine)
    } catch (e) {
      notify(`Реакция не поставлена: ${errorText(e)}`)
    }
  }

  async function remove(m: TalkMessage) {
    if (!token || !window.confirm('Удалить сообщение? Его удалят у всех участников.')) return
    try {
      await api.talkDelete(token, m.id)
      setMessages((list) => list.map((x) => (x.id === m.id ? { ...x, deleted: true, text: 'Сообщение удалено', reactions: [], editText: null, files: [] } : x)))
    } catch (e) {
      notify(`Не удалось удалить: ${errorText(e)}`)
    }
  }

  function startEdit(m: TalkMessage) {
    setEditing(m)
    setReplyTo(null)
    setDraft(m.editText ?? m.text)
    requestAnimationFrame(() => inputRef.current?.focus())
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
          <button className="icon-button soft" aria-label="Новая беседа" title="Новая беседа" onClick={() => setNewChat(true)}><MessageSquarePlus size={17} /></button>
          <button className="icon-button soft" aria-label="Обновить" onClick={() => void loadRooms()}><RefreshCw size={17} /></button>
        </div>
        <div className="message-list">
          {rooms === null && !error && <div className="empty-state"><span>Загрузка бесед…</span></div>}
          {visibleRooms.map((r) => (
            <button key={r.token} className={`message-row room-row ${r.token === token ? 'selected' : ''} ${r.unread ? 'unread' : ''}`} onClick={() => setToken(r.token)}>
              {r.kind === 1 && r.peer
                ? <Avatar id={r.peer} name={r.name} />
                : <Avatar kind="room" id={r.token} name={r.name} fallback={r.kind === 1 ? undefined : <Users size={16} />} />}
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
            <div className="talk-messages" ref={listRef}
              onScroll={(e) => { const el = e.currentTarget; stickToBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80 }}>
              {loading && <div className="empty-state"><span>Загрузка сообщений…</span></div>}
              {messages.map((m) => m.system ? (
                <div key={m.id} className="talk-system">{m.text} · {timeOf(m.timestamp)}</div>
              ) : (
                <div key={m.id} className={`talk-msg ${m.own ? 'own' : ''} ${m.deleted ? 'deleted' : ''}`} data-id={m.id}>
                  {!m.own && <Avatar id={m.authorId} name={m.author} />}
                  <div className="talk-bubble">
                    {!m.own && <div className="talk-author">{m.author}</div>}
                    {m.parentText && !m.deleted && <div className="talk-quote"><b>{m.parentAuthor}</b> {m.parentText}</div>}
                    {m.text && (m.deleted ? <div className="talk-text talk-deleted">{m.text}</div> : <TalkText text={m.text} />)}
                    {m.files.length > 0 && (
                      <div className="talk-files">
                        {m.files.map((f) => (
                          <button key={f.id || f.name} className="talk-file" title="Открыть в Nextcloud" onClick={() => void api.ncOpen(`f/${f.id}`)}>
                            <FileText size={15} /><span>{f.name}</span>{f.size > 0 && <small>{formatSize(f.size)}</small>}
                          </button>
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
                    <div className="talk-time">{m.edited && !m.deleted && <span className="talk-edited">изменено</span>}{timeOf(m.timestamp)}</div>
                    {!room.readOnly && !m.deleted && (
                      <div className="talk-actions">
                        <button aria-label="Реакция" title="Реакция" onClick={() => setReactFor(reactFor === m.id ? null : m.id)}><SmilePlus size={14} /></button>
                        <button aria-label="Ответить" title="Ответить" onClick={() => { setReplyTo(m); setEditing(null); inputRef.current?.focus() }}><CornerUpLeft size={14} /></button>
                        <button aria-label="Карточка Deck" title="Создать карточку Deck" onClick={() => setCardFrom(m)}><KanbanSquare size={14} /></button>
                        {m.editText !== null && <button aria-label="Изменить" title="Изменить" onClick={() => startEdit(m)}><Pencil size={14} /></button>}
                        {m.own && <button aria-label="Удалить" title="Удалить" onClick={() => void remove(m)}><Trash2 size={14} /></button>}
                      </div>
                    )}
                    {reactFor === m.id && (
                      <div className="talk-react-picker popover">
                        {QUICK_REACTIONS.map((e) => <button key={e} aria-label={`Реакция ${e}`} onClick={() => void react(m, e)}>{e}</button>)}
                      </div>
                    )}
                  </div>
                </div>
              ))}
              {!loading && messages.length === 0 && <div className="empty-state"><MessageCircle size={30} /><strong>Сообщений пока нет</strong></div>}
            </div>
            {room.readOnly ? <div className="muted talk-readonly">Беседа только для чтения</div> : (
              <form className="talk-compose" onSubmit={(e) => { e.preventDefault(); void send() }}>
                {files.length > 0 && (
                  <div className="talk-pending">
                    {files.map((f) => (
                      <span key={f.id} className="talk-file"><Paperclip size={14} /><span>{f.name}</span><small>{formatSize(f.size)}</small>
                        {!sending && <button type="button" aria-label={`Убрать ${f.name}`} onClick={() => dropFile(f.id)}><X size={13} /></button>}
                      </span>
                    ))}
                  </div>
                )}
                {replyTo && <div className="talk-quote replying"><b>{replyTo.author}</b> {replyTo.text.slice(0, 120)}<button type="button" aria-label="Отменить ответ" onClick={() => setReplyTo(null)}><X size={13} /></button></div>}
                {editing && <div className="talk-quote replying editing"><Pencil size={13} /> <b>Изменение сообщения</b><button type="button" aria-label="Отменить изменение" onClick={() => { setEditing(null); setDraft('') }}><X size={13} /></button></div>}
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
                      onKeyDown={(e) => {
                        if (mentions.length > 0) {
                          if (e.key === 'ArrowDown') { e.preventDefault(); setMentionSel((i) => (i + 1) % mentions.length); return }
                          if (e.key === 'ArrowUp') { e.preventDefault(); setMentionSel((i) => (i - 1 + mentions.length) % mentions.length); return }
                          if (e.key === 'Enter' || e.key === 'Tab') { e.preventDefault(); pickMention(mentions[mentionSel]); return }
                          if (e.key === 'Escape') { setMentions([]); return }
                        }
                        if (e.key === 'Escape' && editing) { setEditing(null); setDraft(''); return }
                        if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); void send() }
                      }}
                      placeholder={editing ? 'Новый текст… (Enter — сохранить, Esc — отмена)' : files.length ? 'Подпись к файлу… (Enter — отправить)' : 'Сообщение… (@ — упомянуть, Enter — отправить, Shift+Enter — новая строка)'}
                    />
                  </div>
                  <button type="button" className={`icon-button soft talk-emoji ${emojiOpen ? 'active' : ''}`} aria-label="Эмодзи" title="Эмодзи" onClick={() => setEmojiOpen((v) => !v)}><Smile size={18} /></button>
                  <button className="primary-button" type="submit" disabled={sending || (!draft.trim() && files.length === 0)} aria-label={editing ? 'Сохранить' : 'Отправить'}>{sending && files.length > 0 ? 'Загрузка…' : editing ? 'Сохранить' : <Send size={16} />}</button>
                </div>
              </form>
            )}
          </>
        ) : (
          <div className="empty-reader">{error ? <><strong>Talk недоступен</strong><span>{error}</span></> : <><MessageCircle size={42} /><strong>Выберите беседу</strong></>}</div>
        )}
      </main>

      {cardFrom && room && <CardFromMessage message={cardFrom} room={room} onClose={() => setCardFrom(null)} onDone={(t) => { setCardFrom(null); notify(t) }} />}
      {newChat && <NewChat onClose={() => setNewChat(false)} onCreated={(t) => { setNewChat(false); void loadRooms().then(() => setToken(t)) }} />}
    </div>
  )
}

/** Карточка Deck из сообщения: текст, автор и ссылка на сообщение в Talk. */
function CardFromMessage({ message, room, onClose, onDone }: { message: TalkMessage; room: TalkRoom; onClose: () => void; onDone: (t: string) => void }) {
  const [boards, setBoards] = useState<DeckBoard[] | null>(null)
  const [boardId, setBoardId] = useState<number | null>(null)
  const [stacks, setStacks] = useState<DeckStack[]>([])
  const [stackId, setStackId] = useState<number | null>(null)
  const firstLine = message.text.split('\n')[0] || message.files[0]?.name || 'Сообщение из Talk'
  const [title, setTitle] = useState(firstLine.length > 80 ? `${firstLine.slice(0, 79)}…` : firstLine)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [base, setBase] = useState('')

  useEffect(() => {
    api.ncStatus().then((s) => setBase(s.url.replace(/\/?$/, '/'))).catch(() => undefined)
    api.deckBoards()
      .then((list) => {
        const editable = list.filter((b) => b.canEdit && !b.archived)
        setBoards(editable)
        let last: number | null = null
        try { last = Number(localStorage.getItem('rrcki-mail.deck-board')) || null } catch { /* нет */ }
        setBoardId(editable.find((b) => b.id === last)?.id ?? editable[0]?.id ?? null)
      })
      .catch((e) => setError(errorText(e)))
  }, [])

  useEffect(() => {
    if (boardId === null) return
    setStacks([])
    api.deckStacks(boardId).then((s) => { setStacks(s); setStackId(s[0]?.id ?? null) }).catch((e) => setError(errorText(e)))
  }, [boardId])

  async function create() {
    if (boardId === null || stackId === null || !title.trim()) return
    setBusy(true)
    setError('')
    const when = new Date(message.timestamp * 1000).toLocaleString('ru-RU')
    const link = base ? `${base}call/${room.token}#message_${message.id}` : ''
    const files = message.files.map((f) => `- ${f.name}${base && f.id ? ` (${base}f/${f.id})` : ''}`).join('\n')
    const description = [message.text, files, `— ${message.author}, ${when}, беседа «${room.name}»`, link].filter(Boolean).join('\n\n')
    try {
      await api.deckCreateCard(boardId, stackId, title, description)
      onDone(`Карточка создана: ${boards?.find((b) => b.id === boardId)?.title ?? 'Deck'} → ${stacks.find((s) => s.id === stackId)?.title ?? ''}`)
    } catch (e) {
      setError(errorText(e))
      setBusy(false)
    }
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget && !busy) onClose() }}>
      <div className="settings-modal talk-card-modal">
        <div className="compose-header"><div><strong>Карточка Deck из сообщения</strong><small>{message.author}</small></div><button className="icon-button" onClick={onClose} aria-label="Закрыть"><X size={18} /></button></div>
        <div className="talk-quote">{message.text.slice(0, 300) || message.files.map((f) => f.name).join(', ')}</div>
        <label className="login-field"><span>Доска</span>
          <select value={boardId ?? ''} onChange={(e) => setBoardId(Number(e.target.value))} disabled={!boards?.length}>
            {(boards ?? []).map((b) => <option key={b.id} value={b.id}>{b.title}</option>)}
          </select>
        </label>
        <label className="login-field"><span>Колонка</span>
          <select value={stackId ?? ''} onChange={(e) => setStackId(Number(e.target.value))} disabled={!stacks.length}>
            {stacks.map((s) => <option key={s.id} value={s.id}>{s.title}</option>)}
          </select>
        </label>
        <label className="login-field"><span>Название</span><input value={title} onChange={(e) => setTitle(e.target.value)} /></label>
        {boards && boards.length === 0 && <div className="muted">Нет досок, где вы можете создавать карточки.</div>}
        {error && <div className="login-error" role="alert">{error}</div>}
        <div className="settings-actions">
          <button className="text-button" onClick={onClose}>Отмена</button>
          <button className="primary-button" disabled={busy || boardId === null || stackId === null || !title.trim()} onClick={() => void create()}>{busy ? 'Создание…' : 'Создать карточку'}</button>
        </div>
      </div>
    </div>
  )
}

/** Новая беседа: один собеседник — личная, несколько или с названием — групповая. */
function NewChat({ onClose, onCreated }: { onClose: () => void; onCreated: (token: string) => void }) {
  const [q, setQ] = useState('')
  const [found, setFound] = useState<NcUser[]>([])
  const [picked, setPicked] = useState<NcUser[]>([])
  const [name, setName] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  useEffect(() => {
    if (q.trim().length < 2) { setFound([]); return }
    let cancelled = false
    const t = setTimeout(() => {
      api.ncSearchUsers(q).then((list) => { if (!cancelled) setFound(list) }).catch((e) => { if (!cancelled) setError(errorText(e)) })
    }, 250)
    return () => { cancelled = true; clearTimeout(t) }
  }, [q])

  async function create() {
    setBusy(true)
    setError('')
    try {
      onCreated(await api.talkCreate(picked.map((u) => u.id), picked.length > 1 || name.trim() ? (name.trim() || picked.map((u) => u.label).join(', ')) : null))
    } catch (e) {
      setError(errorText(e))
      setBusy(false)
    }
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget && !busy) onClose() }}>
      <div className="settings-modal talk-new-modal">
        <div className="compose-header"><div><strong>Новая беседа</strong><small>Один собеседник — личная беседа, несколько — групповая</small></div><button className="icon-button" onClick={onClose} aria-label="Закрыть"><X size={18} /></button></div>
        {picked.length > 0 && (
          <div className="talk-pending">
            {picked.map((u) => <span key={u.id} className="talk-file"><span>{u.label}</span><button type="button" aria-label={`Убрать ${u.label}`} onClick={() => setPicked((l) => l.filter((x) => x.id !== u.id))}><X size={13} /></button></span>)}
          </div>
        )}
        <label className="search-box"><Search size={17} /><input autoFocus value={q} onChange={(e) => setQ(e.target.value)} placeholder="Имя или логин сотрудника" /></label>
        <div className="talk-user-list">
          {found.filter((u) => !picked.some((p) => p.id === u.id)).map((u) => (
            <button key={u.id} className="message-row" onClick={() => { setPicked((l) => [...l, u]); setQ('') }}>
              <Avatar id={u.id} name={u.label} />
              <div className="message-summary"><span className="sender-name">{u.label}</span>{u.sub && <span className="message-preview">{u.sub}</span>}</div>
            </button>
          ))}
        </div>
        {picked.length > 1 && <label className="login-field"><span>Название беседы</span><input value={name} onChange={(e) => setName(e.target.value)} placeholder={picked.map((u) => u.label).join(', ')} /></label>}
        {error && <div className="login-error" role="alert">{error}</div>}
        <div className="settings-actions">
          <button className="text-button" onClick={onClose}>Отмена</button>
          <button className="primary-button" disabled={busy || picked.length === 0} onClick={() => void create()}>{busy ? 'Создание…' : picked.length > 1 ? 'Создать беседу' : 'Написать'}</button>
        </div>
      </div>
    </div>
  )
}
