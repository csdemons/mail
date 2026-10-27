import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { CalendarClock, CheckCircle2, ExternalLink, FileText, ListChecks, Lock, MessageSquare, Paperclip, Plus, RefreshCw, Trash2, X } from 'lucide-react'

import { api, errorText, formatSize } from './api'
import { addItem, checklistProgress, parseChecklist, removeItem, renameItem, toggleItem } from './checklist'
import type { DeckAttachment, DeckBoard, DeckBoardDetails, DeckCard, DeckComment, DeckLabel, DeckStack, DeckUser } from './types'

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
  // Перетаскивание сделано на pointer-событиях: HTML5 drag-and-drop в WebView2
  // (Windows) и WebKitGTK (Linux) внутри окна Tauri срабатывает ненадёжно.
  const [dragging, setDragging] = useState<DeckCard | null>(null)
  const [dropStack, setDropStack] = useState<number | null>(null)
  const [ghost, setGhost] = useState<{ x: number; y: number; w: number; html: string } | null>(null)
  const drag = useRef<{ card: DeckCard; el: HTMLElement; x0: number; y0: number; dx: number; dy: number; active: boolean } | null>(null)
  const dropRef = useRef<number | null>(null)
  const suppressClick = useRef(false)
  const columnsRef = useRef<HTMLDivElement>(null)
  const moveRef = useRef<(card: DeckCard, stack: number) => Promise<void>>(async () => {})

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

  const loadStacks = useCallback(async (id: number, quiet = false) => {
    if (!quiet) setLoading(true)
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

  moveRef.current = move

  useEffect(() => {
    function stackAt(x: number, y: number): number | null {
      const el = document.elementFromPoint(x, y)?.closest<HTMLElement>('[data-stack]')
      return el ? Number(el.dataset.stack) : null
    }
    function onMove(e: PointerEvent) {
      const d = drag.current
      if (!d) return
      if (!d.active) {
        if (Math.hypot(e.clientX - d.x0, e.clientY - d.y0) < 6) return
        d.active = true
        setDragging(d.card)
        document.body.classList.add('deck-dragging')
        window.getSelection()?.removeAllRanges()
      }
      e.preventDefault()
      setGhost({ x: e.clientX - d.dx, y: e.clientY - d.dy, w: d.el.offsetWidth, html: d.el.innerHTML })
      const target = stackAt(e.clientX, e.clientY)
      dropRef.current = target
      setDropStack(target)
      // Автопрокрутка колонок у краёв.
      const box = columnsRef.current?.getBoundingClientRect()
      if (box && columnsRef.current) {
        if (e.clientX < box.left + 60) columnsRef.current.scrollLeft -= 18
        else if (e.clientX > box.right - 60) columnsRef.current.scrollLeft += 18
      }
    }
    function finish(commit: boolean) {
      const d = drag.current
      drag.current = null
      if (!d?.active) return
      suppressClick.current = true
      setTimeout(() => { suppressClick.current = false }, 0)
      document.body.classList.remove('deck-dragging')
      const target = dropRef.current
      dropRef.current = null
      setGhost(null)
      setDragging(null)
      setDropStack(null)
      if (commit && target !== null) void moveRef.current(d.card, target)
    }
    const up = (e: PointerEvent) => {
      // Колонку определяем и по точке отпускания — последнего pointermove могло не быть.
      if (drag.current?.active) dropRef.current = stackAt(e.clientX, e.clientY) ?? dropRef.current
      finish(true)
    }
    const cancel = () => finish(false)
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') finish(false) }
    window.addEventListener('pointermove', onMove, { passive: false })
    window.addEventListener('pointerup', up)
    window.addEventListener('pointercancel', cancel)
    window.addEventListener('blur', cancel)
    window.addEventListener('keydown', key)
    return () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', up)
      window.removeEventListener('pointercancel', cancel)
      window.removeEventListener('blur', cancel)
      window.removeEventListener('keydown', key)
      document.body.classList.remove('deck-dragging')
    }
  }, [])

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
        <div className="deck-columns" ref={columnsRef}>
          {stacks.map((s) => (
            <section
              key={s.id}
              data-stack={s.id}
              className={`deck-column ${dropStack === s.id && dragging?.stackId !== s.id ? 'drop' : ''}`}
            >
              <header><strong>{s.title}</strong><span>{s.cards.length}</span></header>
              <div className="deck-cards">
                {s.cards.map((c) => {
                  const due = dueInfo(c)
                  return (
                    <button
                      key={c.id}
                      className={`deck-card ${selected?.id === c.id ? 'selected' : ''} ${c.done ? 'done' : ''} ${dragging?.id === c.id ? 'drag-source' : ''}`}
                      data-card={c.id}
                      onPointerDown={(e) => {
                        if (!editable || e.button !== 0) return
                        const r = e.currentTarget.getBoundingClientRect()
                        drag.current = { card: c, el: e.currentTarget, x0: e.clientX, y0: e.clientY, dx: e.clientX - r.left, dy: e.clientY - r.top, active: false }
                      }}
                      onDragStart={(e) => e.preventDefault()}
                      onClick={() => { if (!suppressClick.current) setSelected(c) }}
                    >
                      {c.labels.length > 0 && <div className="deck-labels">{c.labels.map((l) => <span key={l.id} style={{ background: `#${l.color}` }}>{l.title}</span>)}</div>}
                      <div className="deck-card-title">{c.done && <CheckCircle2 size={14} />} {c.title}</div>
                      <div className="deck-card-meta">
                        {due && <span className={due.overdue ? 'overdue' : ''}><CalendarClock size={12} /> {due.text}</span>}
                        {c.commentsUnread > 0 && <span><MessageSquare size={12} /> {c.commentsUnread}</span>}
                        {(() => { const p = checklistProgress(c.description); return p.total > 0 && <span className={p.done === p.total ? 'all-done' : ''} title="Подзадачи"><ListChecks size={12} /> {p.done}/{p.total}</span> })()}
                        {c.attachmentCount > 0 && <span><Paperclip size={12} /> {c.attachmentCount}</span>}
                        <span className="deck-people">{c.assigned.map((a) => <i key={a.uid} title={a.name}>{initials(a.name)}</i>)}</span>
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
        {ghost && <div className="deck-card deck-ghost" style={{ left: ghost.x, top: ghost.y, width: ghost.w }} dangerouslySetInnerHTML={{ __html: ghost.html }} />}
      </main>

      {selected && board && (
        <CardPanel
          key={selected.id}
          board={board}
          card={selected}
          stacks={stacks}
          onClose={() => setSelected(null)}
          onChanged={async (text) => { onNotice(text); await loadStacks(board.id) }}
          onRefresh={() => loadStacks(board.id, true)}
          onMove={(target) => void move(selected, target)}
        />
      )}
    </div>
  )
}

function CardPanel({ board, card, stacks, onClose, onChanged, onRefresh, onMove }: {
  board: DeckBoard
  card: DeckCard
  stacks: DeckStack[]
  onClose: () => void
  onChanged: (text: string) => Promise<void>
  onRefresh: () => Promise<void>
  onMove: (stackId: number) => void
}) {
  const [title, setTitle] = useState(card.title)
  const [description, setDescription] = useState(card.description)
  const [due, setDue] = useState(dateInputValue(card.duedate))
  const [done, setDone] = useState(card.done)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [details, setDetails] = useState<DeckBoardDetails | null>(null)
  const [assigned, setAssigned] = useState<DeckUser[]>(card.assigned)
  const [labels, setLabels] = useState<DeckLabel[]>(card.labels)
  const [newItem, setNewItem] = useState('')
  const [editingItem, setEditingItem] = useState<{ line: number; text: string } | null>(null)
  const [comments, setComments] = useState<DeckComment[] | null>(null)
  const [comment, setComment] = useState('')
  const [attachments, setAttachments] = useState<DeckAttachment[] | null>(null)
  const [tab, setTab] = useState<'tasks' | 'comments' | 'files'>('tasks')
  const [picker, setPicker] = useState<'users' | 'labels' | null>(null)
  const ro = !board.canEdit
  const items = useMemo(() => parseChecklist(description), [description])
  const progress = items.length ? Math.round((items.filter((i) => i.done).length / items.length) * 100) : 0

  useEffect(() => {
    api.deckBoardDetails(board.id).then(setDetails).catch(() => undefined)
    api.deckComments(card.id).then(setComments).catch((e) => { setComments([]); setError(errorText(e)) })
    api.deckAttachments(board.id, card.stackId, card.id).then(setAttachments).catch(() => setAttachments([]))
  }, [board.id, card.id, card.stackId])

  const dueIso = () => (due ? new Date(`${due}T12:00:00`).toISOString() : null)

  async function save() {
    setBusy(true)
    setError('')
    try {
      await api.deckUpdateCard(board.id, card.stackId, card.id, title, description, dueIso(), done)
      await onChanged('Карточка сохранена.')
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  /** Подзадачи сохраняются сразу — как галочки в веб-Deck. */
  async function saveDescription(next: string) {
    const prev = description
    setDescription(next)
    try {
      await api.deckUpdateCard(board.id, card.stackId, card.id, title, next, dueIso(), done)
      await onRefresh()
    } catch (e) {
      setDescription(prev)
      setError(errorText(e))
    }
  }

  async function addSubtask() {
    if (!newItem.trim()) return
    const text = newItem
    setNewItem('')
    await saveDescription(addItem(description, text))
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

  async function toggleUser(u: DeckUser) {
    const on = assigned.some((a) => a.uid === u.uid)
    setAssigned((list) => (on ? list.filter((a) => a.uid !== u.uid) : [...list, u]))
    try {
      await api.deckAssign(board.id, card.stackId, card.id, u.uid, !on)
      await onRefresh()
    } catch (e) {
      setAssigned(card.assigned)
      setError(errorText(e))
    }
  }

  async function toggleLabel(l: DeckLabel) {
    const on = labels.some((x) => x.id === l.id)
    setLabels((list) => (on ? list.filter((x) => x.id !== l.id) : [...list, l]))
    try {
      await api.deckLabel(board.id, card.stackId, card.id, l.id, !on)
      await onRefresh()
    } catch (e) {
      setLabels(card.labels)
      setError(errorText(e))
    }
  }

  async function sendComment() {
    if (!comment.trim()) return
    try {
      await api.deckCommentAdd(card.id, comment)
      setComment('')
      setComments(await api.deckComments(card.id))
    } catch (e) {
      setError(errorText(e))
    }
  }

  async function deleteComment(c: DeckComment) {
    if (!window.confirm('Удалить комментарий?')) return
    try {
      await api.deckCommentDelete(card.id, c.id)
      setComments((list) => list?.filter((x) => x.id !== c.id) ?? list)
    } catch (e) {
      setError(errorText(e))
    }
  }

  return (
    <section className="reader-pane deck-panel">
      <div className="reader-toolbar">
        <button className="icon-button soft" aria-label="Открыть в Nextcloud" title="Открыть в Nextcloud" onClick={() => void api.ncOpen(`apps/deck/board/${board.id}/card/${card.id}`)}><ExternalLink size={17} /></button>
        <div className="reader-toolbar-spacer" />
        <button className="icon-button soft" aria-label="Закрыть" onClick={onClose}><X size={18} /></button>
      </div>
      <div className="reader-content editor-modal deck-card-detail">
        <input className="deck-title-input" value={title} disabled={ro} onChange={(e) => setTitle(e.target.value)} aria-label="Название" />
        <div className="editor-row">
          <label className="login-field"><span>Колонка</span>
            <select value={card.stackId} disabled={ro} onChange={(e) => onMove(Number(e.target.value))}>
              {stacks.map((s) => <option key={s.id} value={s.id}>{s.title}</option>)}
            </select>
          </label>
          <label className="login-field"><span>Срок</span><input type="date" value={due} disabled={ro} onChange={(e) => setDue(e.target.value)} /></label>
          <label className="login-remember deck-done"><input type="checkbox" checked={done} disabled={ro} onChange={(e) => setDone(e.target.checked)} /><span>Выполнено</span></label>
        </div>

        <div className="deck-people-row">
          <span className="muted">Исполнители</span>
          <div className="deck-chips">
            {assigned.map((u) => <span key={u.uid} className="deck-chip"><i>{initials(u.name)}</i>{u.name}{!ro && <button aria-label={`Снять ${u.name}`} onClick={() => void toggleUser(u)}><X size={11} /></button>}</span>)}
            {!ro && <button className="deck-chip add" onClick={() => setPicker(picker === 'users' ? null : 'users')}><Plus size={12} /> {assigned.length ? '' : 'Назначить'}</button>}
          </div>
          {picker === 'users' && details && (
            <div className="deck-picker">
              {details.users.map((u) => (
                <label key={u.uid}><input type="checkbox" checked={assigned.some((a) => a.uid === u.uid)} onChange={() => void toggleUser(u)} /> {u.name}</label>
              ))}
              {details.users.length === 0 && <span className="muted">Нет участников доски</span>}
            </div>
          )}
        </div>
        <div className="deck-people-row">
          <span className="muted">Метки</span>
          <div className="deck-chips">
            {labels.map((l) => <span key={l.id} className="deck-label-chip" style={{ background: `#${l.color}` }}>{l.title}{!ro && <button aria-label={`Снять ${l.title}`} onClick={() => void toggleLabel(l)}><X size={11} /></button>}</span>)}
            {!ro && <button className="deck-chip add" onClick={() => setPicker(picker === 'labels' ? null : 'labels')}><Plus size={12} /> {labels.length ? '' : 'Метка'}</button>}
          </div>
          {picker === 'labels' && details && (
            <div className="deck-picker">
              {details.labels.map((l) => (
                <label key={l.id}><input type="checkbox" checked={labels.some((x) => x.id === l.id)} onChange={() => void toggleLabel(l)} /> <span className="deck-label-chip" style={{ background: `#${l.color}` }}>{l.title}</span></label>
              ))}
              {details.labels.length === 0 && <span className="muted">На доске нет меток</span>}
            </div>
          )}
        </div>

        <div className="deck-tabs" role="tablist">
          <button role="tab" className={tab === 'tasks' ? 'active' : ''} onClick={() => setTab('tasks')}><ListChecks size={14} /> Подзадачи{items.length ? ` ${items.filter((i) => i.done).length}/${items.length}` : ''}</button>
          <button role="tab" className={tab === 'comments' ? 'active' : ''} onClick={() => setTab('comments')}><MessageSquare size={14} /> Комментарии{comments?.length ? ` ${comments.length}` : ''}</button>
          <button role="tab" className={tab === 'files' ? 'active' : ''} onClick={() => setTab('files')}><Paperclip size={14} /> Вложения{attachments?.length ? ` ${attachments.length}` : ''}</button>
        </div>

        {tab === 'tasks' && (
          <div className="deck-tasks">
            {items.length > 0 && <div className="deck-progress"><i style={{ width: `${progress}%` }} /><span>{progress}%</span></div>}
            <ul className="deck-checklist">
              {items.map((it) => (
                <li key={it.line} className={it.done ? 'done' : ''}>
                  <input type="checkbox" checked={it.done} disabled={ro} aria-label={it.text} onChange={() => void saveDescription(toggleItem(description, it.line))} />
                  {editingItem?.line === it.line ? (
                    <input autoFocus className="deck-item-edit" value={editingItem.text}
                      onChange={(e) => setEditingItem({ line: it.line, text: e.target.value })}
                      onBlur={() => { const t = editingItem.text.trim(); setEditingItem(null); if (t && t !== it.text) void saveDescription(renameItem(description, it.line, t)) }}
                      onKeyDown={(e) => { if (e.key === 'Enter') (e.target as HTMLInputElement).blur(); if (e.key === 'Escape') setEditingItem(null) }} />
                  ) : (
                    <span onDoubleClick={() => !ro && setEditingItem({ line: it.line, text: it.text })}>{it.text}</span>
                  )}
                  {!ro && <button aria-label={`Удалить подзадачу ${it.text}`} title="Удалить" onClick={() => void saveDescription(removeItem(description, it.line))}><X size={13} /></button>}
                </li>
              ))}
            </ul>
            {!ro && (
              <form className="deck-add-item" onSubmit={(e) => { e.preventDefault(); void addSubtask() }}>
                <Plus size={15} />
                <input value={newItem} onChange={(e) => setNewItem(e.target.value)} placeholder="Новая подзадача — Enter, чтобы добавить" />
              </form>
            )}
            {items.length === 0 && ro && <div className="muted">Подзадач нет.</div>}
            <label className="login-field"><span>Описание</span><textarea rows={6} value={description} disabled={ro} onChange={(e) => setDescription(e.target.value)} placeholder="Поддерживается Markdown; подзадачи — строки «- [ ] …»" /></label>
          </div>
        )}

        {tab === 'comments' && (
          <div className="deck-comments">
            {comments === null && <div className="muted">Загрузка…</div>}
            {comments?.length === 0 && <div className="muted">Комментариев пока нет.</div>}
            {comments?.map((c) => (
              <div key={c.id} className={`deck-comment ${c.own ? 'own' : ''}`}>
                <div className="sender-avatar">{initials(c.author)}</div>
                <div>
                  <div className="deck-comment-head"><b>{c.author}</b><span>{c.created ? new Date(c.created).toLocaleString('ru-RU', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' }) : ''}</span>
                    {c.own && <button aria-label="Удалить комментарий" title="Удалить" onClick={() => void deleteComment(c)}><Trash2 size={12} /></button>}</div>
                  <div className="deck-comment-text">{c.message}</div>
                </div>
              </div>
            ))}
            <form className="deck-add-item" onSubmit={(e) => { e.preventDefault(); void sendComment() }}>
              <MessageSquare size={15} />
              <input value={comment} onChange={(e) => setComment(e.target.value)} placeholder="Комментарий — Enter, чтобы отправить" />
            </form>
          </div>
        )}

        {tab === 'files' && (
          <div className="deck-files">
            {attachments === null && <div className="muted">Загрузка…</div>}
            {attachments?.length === 0 && <div className="muted">Вложений нет. Добавить файл можно в Nextcloud.</div>}
            {attachments?.map((a) => (
              <button key={a.id} className="talk-file" title="Открыть в Nextcloud"
                onClick={() => void api.ncOpen(a.fileId ? `f/${a.fileId}` : `apps/deck/board/${board.id}/card/${card.id}`)}>
                <FileText size={15} /><span>{a.name}</span>{a.size > 0 && <small>{formatSize(a.size)}</small>}
              </button>
            ))}
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
