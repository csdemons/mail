import { useCallback, useEffect, useMemo, useState } from 'react'
import { CalendarClock, CheckCircle2, ExternalLink, Lock, MessageSquare, Paperclip, Plus, RefreshCw, Trash2, X } from 'lucide-react'

import { api, errorText } from './api'
import type { DeckBoard, DeckCard, DeckStack } from './types'

const LAST_BOARD_KEY = 'rrcki-mail.deck-board'

function initials(name: string) {
  const p = name.trim().split(/\s+/)
  return ((p[0]?.[0] ?? '') + (p[1]?.[0] ?? '')).toUpperCase() || '?'
}

function dueInfo(card: DeckCard) {
  if (!card.duedate) return null
  const d = new Date(card.duedate)
  if (Number.isNaN(d.getTime())) return null
  const overdue = !card.done && d.getTime() < Date.now()
  return { text: d.toLocaleDateString('ru-RU', { day: 'numeric', month: 'short' }), overdue }
}

function dateInputValue(iso: string | null) {
  if (!iso) return ''
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? '' : `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
}

export default function DeckView({ onNotice }: { onNotice: (t: string) => void }) {
  const [boards, setBoards] = useState<DeckBoard[] | null>(null)
  const [boardId, setBoardId] = useState<number | null>(() => {
    try { return Number(localStorage.getItem(LAST_BOARD_KEY)) || null } catch { return null }
  })
  const [stacks, setStacks] = useState<DeckStack[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [selected, setSelected] = useState<DeckCard | null>(null)
  const [adding, setAdding] = useState<number | null>(null)
  const [newTitle, setNewTitle] = useState('')
  const [dragging, setDragging] = useState<DeckCard | null>(null)
  const [dropStack, setDropStack] = useState<number | null>(null)

  const board = boards?.find((b) => b.id === boardId) ?? null
  const editable = Boolean(board?.canEdit)

  useEffect(() => {
    api.deckBoards()
      .then((list) => {
        setBoards(list)
        setBoardId((id) => (list.some((b) => b.id === id) ? id : list.find((b) => !b.archived)?.id ?? null))
      })
      .catch((e) => setError(errorText(e)))
  }, [])

  const loadStacks = useCallback(async (id: number) => {
    setLoading(true)
    setError('')
    try {
      const s = await api.deckStacks(id)
      setStacks(s)
      setSelected((cur) => (cur ? s.flatMap((x) => x.cards).find((c) => c.id === cur.id) ?? null : null))
    } catch (e) {
      setError(errorText(e))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    if (boardId === null) return
    try { localStorage.setItem(LAST_BOARD_KEY, String(boardId)) } catch { /* не критично */ }
    setSelected(null)
    void loadStacks(boardId)
  }, [boardId, loadStacks])

  async function addCard(stackId: number) {
    if (!board || !newTitle.trim()) { setAdding(null); return }
    try {
      await api.deckCreateCard(board.id, stackId, newTitle)
      setNewTitle('')
      setAdding(null)
      await loadStacks(board.id)
    } catch (e) {
      onNotice(`Не удалось создать карточку: ${errorText(e)}`)
    }
  }

  async function addStack() {
    if (!board) return
    const title = window.prompt('Название колонки')
    if (!title?.trim()) return
    try {
      await api.deckCreateStack(board.id, title)
      await loadStacks(board.id)
    } catch (e) {
      onNotice(`Не удалось создать колонку: ${errorText(e)}`)
    }
  }

  async function move(card: DeckCard, targetStack: number) {
    if (!board || card.stackId === targetStack) return
    // Оптимистично переносим, при ошибке перечитываем доску.
    setStacks((list) => list.map((s) => ({
      ...s,
      cards: s.id === targetStack ? [...s.cards.filter((c) => c.id !== card.id), { ...card, stackId: targetStack }]
        : s.cards.filter((c) => c.id !== card.id),
    })))
    try {
      const target = stacks.find((s) => s.id === targetStack)
      await api.deckMoveCard(board.id, card.stackId, card.id, targetStack, target?.cards.length ?? 0)
      await loadStacks(board.id)
    } catch (e) {
      onNotice(`Не удалось перенести: ${errorText(e)}`)
      await loadStacks(board.id)
    }
  }

  const activeBoards = useMemo(() => (boards ?? []).filter((b) => !b.archived), [boards])

  return (
    <div className="module-view deck-view">
      <aside className="folder-sidebar">
        <div className="sidebar-heading"><span>Доски</span></div>
        <div className="folder-list">
          {boards === null && !error && <div className="folder-placeholder">Загрузка досок…</div>}
          {activeBoards.map((b) => (
            <button key={b.id} className={`folder-item ${b.id === boardId ? 'selected' : ''}`} onClick={() => setBoardId(b.id)} title={`Владелец: ${b.owner}`}>
              <span className="board-dot" style={{ background: `#${b.color || '0082c9'}` }} />
              <span>{b.title}</span>
              {!b.canEdit && <Lock size={13} />}
            </button>
          ))}
          {boards && activeBoards.length === 0 && <div className="folder-placeholder">Досок нет. Создайте доску в Nextcloud или попросите доступ.</div>}
        </div>
      </aside>

      <main className="deck-main">
        <div className="calendar-toolbar">
          <strong className="calendar-title">{board?.title ?? 'Deck'}</strong>
          {board && !board.canEdit && <span className="muted"><Lock size={13} /> только чтение</span>}
          {loading && <span className="muted">Загрузка…</span>}
          <div className="reader-toolbar-spacer" />
          {board && <button className="icon-button soft" aria-label="Открыть в Nextcloud" title="Открыть в Nextcloud" onClick={() => void api.ncOpen(`apps/deck/board/${board.id}`)}><ExternalLink size={17} /></button>}
          {board && <button className="icon-button soft" aria-label="Обновить" onClick={() => void loadStacks(board.id)}><RefreshCw size={17} className={loading ? 'spin' : ''} /></button>}
          {editable && <button className="secondary-button" onClick={() => void addStack()}><Plus size={15} /> Колонка</button>}
        </div>
        {error && <div className="inline-warning" role="alert">Deck: {error}</div>}
        <div className="deck-columns">
          {stacks.map((s) => (
            <section
              key={s.id}
              className={`deck-column ${dropStack === s.id ? 'drop' : ''}`}
              onDragOver={(e) => { if (dragging && editable) { e.preventDefault(); setDropStack(s.id) } }}
              onDragLeave={() => setDropStack((cur) => (cur === s.id ? null : cur))}
              onDrop={(e) => { e.preventDefault(); setDropStack(null); if (dragging) void move(dragging, s.id); setDragging(null) }}
            >
              <header><strong>{s.title}</strong><span>{s.cards.length}</span></header>
              <div className="deck-cards">
                {s.cards.map((c) => {
                  const due = dueInfo(c)
                  return (
                    <button
                      key={c.id}
                      className={`deck-card ${selected?.id === c.id ? 'selected' : ''} ${c.done ? 'done' : ''}`}
                      draggable={editable}
                      onDragStart={(e) => { setDragging(c); e.dataTransfer.effectAllowed = 'move'; e.dataTransfer.setData('text/plain', String(c.id)) }}
                      onDragEnd={() => { setDragging(null); setDropStack(null) }}
                      onClick={() => setSelected(c)}
                    >
                      {c.labels.length > 0 && <div className="deck-labels">{c.labels.map((l) => <span key={l.id} style={{ background: `#${l.color}` }}>{l.title}</span>)}</div>}
                      <div className="deck-card-title">{c.done && <CheckCircle2 size={14} />} {c.title}</div>
                      <div className="deck-card-meta">
                        {due && <span className={due.overdue ? 'overdue' : ''}><CalendarClock size={12} /> {due.text}</span>}
                        {c.commentsUnread > 0 && <span><MessageSquare size={12} /> {c.commentsUnread}</span>}
                        {c.attachmentCount > 0 && <span><Paperclip size={12} /> {c.attachmentCount}</span>}
                        <span className="deck-people">{c.assigned.map((a) => <i key={a} title={a}>{initials(a)}</i>)}</span>
                      </div>
                    </button>
                  )
                })}
              </div>
              {editable && (adding === s.id ? (
                <form className="deck-add" onSubmit={(e) => { e.preventDefault(); void addCard(s.id) }}>
                  <input autoFocus value={newTitle} onChange={(e) => setNewTitle(e.target.value)} placeholder="Название карточки" onBlur={() => { if (!newTitle.trim()) setAdding(null) }} />
                </form>
              ) : (
                <button className="text-button deck-add-button" onClick={() => { setAdding(s.id); setNewTitle('') }}><Plus size={14} /> Карточка</button>
              ))}
            </section>
          ))}
          {board && stacks.length === 0 && !loading && <div className="empty-state">На доске нет колонок.</div>}
        </div>
      </main>

      {selected && board && (
        <CardPanel
          key={selected.id}
          board={board}
          card={selected}
          stacks={stacks}
          onClose={() => setSelected(null)}
          onChanged={async (text) => { onNotice(text); await loadStacks(board.id) }}
          onMove={(target) => void move(selected, target)}
        />
      )}
    </div>
  )
}

function CardPanel({ board, card, stacks, onClose, onChanged, onMove }: {
  board: DeckBoard
  card: DeckCard
  stacks: DeckStack[]
  onClose: () => void
  onChanged: (text: string) => Promise<void>
  onMove: (stackId: number) => void
}) {
  const [title, setTitle] = useState(card.title)
  const [description, setDescription] = useState(card.description)
  const [due, setDue] = useState(dateInputValue(card.duedate))
  const [done, setDone] = useState(card.done)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const ro = !board.canEdit

  async function save() {
    setBusy(true)
    setError('')
    try {
      const duedate = due ? new Date(`${due}T12:00:00`).toISOString() : null
      await api.deckUpdateCard(board.id, card.stackId, card.id, title, description, duedate, done)
      await onChanged('Карточка сохранена.')
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  async function remove() {
    if (!window.confirm(`Удалить карточку «${card.title}»?`)) return
    try {
      await api.deckDeleteCard(board.id, card.stackId, card.id)
      onClose()
      await onChanged('Карточка удалена.')
    } catch (e) {
      setError(errorText(e))
    }
  }

  return (
    <section className="reader-pane deck-panel">
      <div className="reader-toolbar">
        <button className="icon-button soft" aria-label="Открыть в Nextcloud" title="Открыть в Nextcloud (комментарии, вложения)" onClick={() => void api.ncOpen(`apps/deck/board/${board.id}/card/${card.id}`)}><ExternalLink size={17} /></button>
        <div className="reader-toolbar-spacer" />
        <button className="icon-button soft" aria-label="Закрыть" onClick={onClose}><X size={18} /></button>
      </div>
      <div className="reader-content editor-modal">
        <label className="login-field"><span>Название</span><input value={title} disabled={ro} onChange={(e) => setTitle(e.target.value)} /></label>
        <label className="login-field"><span>Колонка</span>
          <select value={card.stackId} disabled={ro} onChange={(e) => onMove(Number(e.target.value))}>
            {stacks.map((s) => <option key={s.id} value={s.id}>{s.title}</option>)}
          </select>
        </label>
        <div className="editor-row">
          <label className="login-field"><span>Срок</span><input type="date" value={due} disabled={ro} onChange={(e) => setDue(e.target.value)} /></label>
          <label className="login-remember deck-done"><input type="checkbox" checked={done} disabled={ro} onChange={(e) => setDone(e.target.checked)} /><span>Выполнено</span></label>
        </div>
        <label className="login-field"><span>Описание</span><textarea rows={8} value={description} disabled={ro} onChange={(e) => setDescription(e.target.value)} placeholder="Поддерживается Markdown" /></label>
        {(card.assigned.length > 0 || card.labels.length > 0) && (
          <div className="deck-card-info">
            {card.assigned.length > 0 && <div><span className="muted">Исполнители:</span> {card.assigned.join(', ')}</div>}
            {card.labels.length > 0 && <div className="deck-labels">{card.labels.map((l) => <span key={l.id} style={{ background: `#${l.color}` }}>{l.title}</span>)}</div>}
          </div>
        )}
        {error && <div className="login-error" role="alert">{error}</div>}
        {!ro && (
          <div className="reader-actions">
            <button className="primary-button" disabled={busy} onClick={() => void save()}>{busy ? 'Сохранение…' : 'Сохранить'}</button>
            <button className="secondary-button danger" onClick={() => void remove()}><Trash2 size={15} /> Удалить</button>
          </div>
        )}
      </div>
    </section>
  )
}
