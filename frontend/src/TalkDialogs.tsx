// Диалоги чата: карточка Deck из сообщения и новая беседа.
import { useEffect, useState } from 'react'
import {
  Search, X } from 'lucide-react'

import { api, errorText, TYPING_PAUSE_MS } from './api'
import Avatar from './Avatar'
import type { DeckBoard, DeckStack, NcUser, TalkMessage, TalkRoom } from './types'


/** Карточка Deck из сообщения: текст, автор и ссылка на сообщение в Talk. */
export function CardFromMessage({ message, room, onClose, onDone }: { message: TalkMessage; room: TalkRoom; onClose: () => void; onDone: (t: string) => void }) {
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
export function NewChat({ onClose, onCreated }: { onClose: () => void; onCreated: (token: string) => void }) {
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
    }, TYPING_PAUSE_MS)
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
