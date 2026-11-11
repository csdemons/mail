import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { DateField } from './DateField'
import { Archive, ArchiveRestore, CalendarClock, CheckCircle2, ChevronDown, ChevronRight, CornerDownRight, ExternalLink, FileText, Filter, FolderInput, FolderMinus, FolderOpen, GitBranch, Handshake, History, Link2, ListChecks, ListTree, Lock, Mail, Maximize2, MessageSquare, Minimize2, Minus, MoveHorizontal, Paperclip, Pencil, Plus, RefreshCw, Search, SkipForward, Trash2, Upload, X } from 'lucide-react'

import { api, errorText, formatSize } from './api'
import { addItem, checklistProgress, parseChecklist, removeItem, renameItem, toggleItem } from './checklist'
import Avatar from './Avatar'
import TalkText from './TalkText'
import type { DeckActivity, DeckAttachment, DeckBoard, DeckBoardDetails, DeckCard, DeckComment, DeckLabel, DeckStack, DeckUser } from './types'
import { SkeletonRows } from './Skeleton'
import { openContextMenu, type MenuItem } from './PopupMenu'
import { blockedBy, boardLinks, joinLinks, nestInColumn, projectOf, related, splitLinks, wouldCycle, type BoardLinks, type CardLinks } from './deckLinks'

/** Цвета новой доски (как в Deck). */
const BOARD_COLORS = ['0082c9', '31cc7c', 'e9322d', 'f1db50', 'a463bf', 'ff7f0e', '7f8c8d']

/** Ручные группы досок (id доски → название группы; '' — «без группы») и свёрнутые группы. */
const GROUPS_KEY = 'atom.deck.groups'
const FOLDED_KEY = 'atom.deck.folded'
function loadJson<T>(key: string, fallback: T): T {
  try { return (JSON.parse(localStorage.getItem(key) ?? 'null') as T | null) ?? fallback } catch { return fallback }
}
function saveJson(key: string, v: unknown) {
  try { localStorage.setItem(key, JSON.stringify(v)) } catch { /* без хранилища — до перезапуска */ }
}

/** Масштаб колонок и карточек Deck и растяжение по ширине окна — настройка рабочего места. */
const DECK_SCALES = [0.85, 1, 1.15, 1.3, 1.5]
function loadDeckView(): { scale: number; stretch: boolean; nest?: boolean } {
  try {
    const v = JSON.parse(localStorage.getItem('atom.deck.view') ?? '{}') as { scale?: number; stretch?: boolean; nest?: boolean }
    return { scale: DECK_SCALES.includes(Number(v.scale)) ? Number(v.scale) : 1, stretch: v.stretch !== false, ...(v.nest === false ? { nest: false } : {}) }
  } catch {
    return { scale: 1, stretch: true }
  }
}

const LAST_BOARD_KEY = 'rrcki-mail.deck-board'
const FILTER_KEY = 'rrcki-mail.deck-filter.'

/** Фильтр карточек, как в веб-Deck: исполнитель, метка, срок, выполненные, текст. */
interface CardFilter {
  /** 'all' | 'me' | 'none' | 'u:<uid>' */
  who: string
  label: number
  due: 'all' | 'overdue' | 'today' | 'week' | 'none'
  hideDone: boolean
  q: string
}
const NO_FILTER: CardFilter = { who: 'all', label: 0, due: 'all', hideDone: false, q: '' }

function loadFilter(boardId: number | null): CardFilter {
  if (boardId === null) return NO_FILTER
  try {
    const raw = localStorage.getItem(FILTER_KEY + boardId)
    if (raw) return { ...NO_FILTER, ...(JSON.parse(raw) as Partial<CardFilter>), q: '' }
  } catch { /* нет хранилища */ }
  return NO_FILTER
}

function isFiltered(f: CardFilter) {
  return f.who !== 'all' || f.label !== 0 || f.due !== 'all' || f.hideDone || f.q.trim() !== ''
}

function cardMatches(c: DeckCard, f: CardFilter, me: string): boolean {
  if (f.hideDone && c.done) return false
  if (f.who === 'me' && !c.assigned.some((a) => a.uid === me)) return false
  if (f.who === 'none' && c.assigned.length > 0) return false
  if (f.who.startsWith('u:') && !c.assigned.some((a) => a.uid === f.who.slice(2))) return false
  if (f.label && !c.labels.some((l) => l.id === f.label)) return false
  if (f.due !== 'all') {
    const d = c.duedate ? new Date(c.duedate) : null
    const valid = d !== null && !Number.isNaN(d.getTime())
    if (f.due === 'none') {
      if (valid) return false
    } else {
      if (!valid || !d) return false
      const now = new Date()
      const endOfDay = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1).getTime()
      if (f.due === 'overdue' && (c.done || d.getTime() >= now.getTime())) return false
      if (f.due === 'today' && d.getTime() >= endOfDay) return false
      if (f.due === 'week' && d.getTime() >= endOfDay + 6 * 86400000) return false
    }
  }
  const q = f.q.trim().toLowerCase()
  if (q && !`${c.title} ${c.description} ${c.labels.map((l) => l.title).join(' ')} ${c.assigned.map((a) => a.name).join(' ')}`.toLowerCase().includes(q)) return false
  return true
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

/** Переходы из блока «Связи»: письмо, папка документов, история организации. */
export interface ChainActions {
  onOpenMail?: (msgId: string) => void
  onOpenFolder?: (path: string) => void
  onOpenOrg?: (domain: string) => void
}


/** Последний выполненный переход из общего поиска. */
let deckFocusSeen = 0

export default function DeckView({ onNotice, focus, onOpenMail, onOpenFolder, onOpenOrg }: { onNotice: (t: string) => void; focus?: { boardId: number; cardId: number; n: number } | null } & ChainActions) {
  const [boards, setBoards] = useState<DeckBoard[] | null>(null)
  const [boardId, setBoardId] = useState<number | null>(() => {
    try { return Number(localStorage.getItem(LAST_BOARD_KEY)) || null } catch { return null }
  })
  const [stacks, setStacks] = useState<DeckStack[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [selected, setSelected] = useState<DeckCard | null>(null)
  /** Карточка целиком (двойной щелчок), как в веб-Deck; иначе — панель справа. */
  const [full, setFull] = useState(false)
  const [adding, setAdding] = useState<number | null>(null)
  const [newTitle, setNewTitle] = useState('')
  // Перетаскивание сделано на pointer-событиях: HTML5 drag-and-drop в WebView2
  // (Windows) и WebKitGTK (Linux) внутри окна Tauri срабатывает ненадёжно.
  const [dragging, setDragging] = useState<DeckCard | null>(null)
  const [dropStack, setDropStack] = useState<number | null>(null)
  // Куда встанет карточка: перед этой карточкой (null — в конец колонки).
  const [dropBefore, setDropBefore] = useState<number | null>(null)
  const dropBeforeRef = useRef<number | null>(null)
  const [ghost, setGhost] = useState<{ x: number; y: number; w: number; html: string } | null>(null)
  const drag = useRef<{ card: DeckCard; el: HTMLElement; x0: number; y0: number; dx: number; dy: number; active: boolean } | null>(null)
  const dropRef = useRef<number | null>(null)
  const suppressClick = useRef(false)
  const columnsRef = useRef<HTMLDivElement>(null)
  const [view, setView] = useState(loadDeckView)
  function updateView(patch: Partial<{ scale: number; stretch: boolean; nest: boolean }>) {
    setView((v) => {
      const next = { ...v, ...patch }
      try { localStorage.setItem('atom.deck.view', JSON.stringify(next)) } catch { /* без хранилища */ }
      return next
    })
  }
  const scaleIdx = DECK_SCALES.indexOf(view.scale)
  const moveRef = useRef<(card: DeckCard, stack: number, before?: number | null) => Promise<void>>(async () => {})

  const [filter, setFilter] = useState<CardFilter>(() => loadFilter(boardId))
  const [details, setDetails] = useState<DeckBoardDetails | null>(null)
  const [me, setMe] = useState('')
  const [ncUrl, setNcUrl] = useState('')
  useEffect(() => { api.ncStatus().then((s) => { setMe(s.uid || s.login); setNcUrl(s.url) }).catch(() => undefined) }, [])

  // Ввод названия (вместо window.prompt: в WebView2 он показывает чужое системное окно, а
  // где-то не работает вовсе).
  const [ask, setAsk] = useState<{ title: string; label: string; value: string; resolve: (v: string | null) => void } | null>(null)
  const askText = (title: string, label: string, value = '') => new Promise<string | null>((resolve) => setAsk({ title, label, value, resolve }))
  const [newBoard, setNewBoard] = useState<{ title: string; color: string; columns: boolean; busy: boolean } | null>(null)

  async function createBoard() {
    if (!newBoard || !newBoard.title.trim()) return
    setNewBoard({ ...newBoard, busy: true })
    try {
      const b = await api.deckCreateBoard(newBoard.title.trim(), newBoard.color, newBoard.columns ? ['К выполнению', 'В работе', 'Готово'] : [])
      const list = await api.deckBoards()
      setBoards(list)
      setBoardId(b.id)
      setNewBoard(null)
      onNotice(`Доска «${b.title}» создана.`)
    } catch (e) {
      onNotice(`Не удалось создать доску: ${errorText(e)}`)
      setNewBoard((x) => (x ? { ...x, busy: false } : x))
    }
  }

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

  /** Доска, колонки которой загружены, и карточка, которую открыть (переход из поиска). */
  const stacksFor = useRef<number | null>(null)
  const pendingCard = useRef<{ boardId: number; cardId: number } | null>(null)
  useEffect(() => {
    // Каждый переход — один раз: при повторном открытии раздела он не повторяется.
    if (!focus || focus.n <= deckFocusSeen) return
    deckFocusSeen = focus.n
    pendingCard.current = { boardId: focus.boardId, cardId: focus.cardId }
    setBoardId(focus.boardId)
  }, [focus])
  useEffect(() => {
    const p = pendingCard.current
    if (!p || p.boardId !== boardId || loading || stacksFor.current !== boardId) return
    pendingCard.current = null
    const card = stacks.flatMap((x) => x.cards).find((c) => c.id === p.cardId)
    if (card) setSelected(card)
    else onNotice('Карточка не найдена на доске (возможно, она в архиве)')
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [boardId, loading, stacks, focus])

  // Какая доска открыта сейчас: ответ по доске, с которой уже ушли, не показываем
  // (иначе под заголовком новой доски оказывались карточки прежней).
  const currentBoard = useRef<number | null>(null)
  currentBoard.current = boardId
  const loadStacks = useCallback(async (id: number, quiet = false) => {
    if (!quiet) setLoading(true)
    setError('')
    try {
      const s = await api.deckStacks(id)
      if (currentBoard.current !== id) return
      setStacks(s)
      stacksFor.current = id
      setSelected((cur) => (cur ? s.flatMap((x) => x.cards).find((c) => c.id === cur.id) ?? null : null))
    } catch (e) {
      if (currentBoard.current === id) setError(errorText(e))
    } finally {
      if (currentBoard.current === id) setLoading(false)
    }
  }, [])

  useEffect(() => {
    if (boardId === null) return
    try { localStorage.setItem(LAST_BOARD_KEY, String(boardId)) } catch { /* не критично */ }
    stacksFor.current = null
    currentBoard.current = boardId
    setStacks([])
    setSelected(null)
    setFilter(loadFilter(boardId))
    setDetails(null)
    api.deckBoardDetails(boardId).then(setDetails).catch(() => undefined)
    void loadStacks(boardId)
  }, [boardId, loadStacks])

  function updateFilter(patch: Partial<CardFilter>) {
    setFilter((f) => {
      const next = { ...f, ...patch }
      try { if (boardId !== null) localStorage.setItem(FILTER_KEY + boardId, JSON.stringify({ ...next, q: '' })) } catch { /* не критично */ }
      return next
    })
  }

  // Участники доски и все, кто назначен на карточки (на случай, если участник ушёл с доски).
  const people = useMemo(() => {
    const map = new Map<string, DeckUser>()
    for (const u of details?.users ?? []) map.set(u.uid, u)
    for (const c of stacks.flatMap((s) => s.cards)) for (const a of c.assigned) if (!map.has(a.uid)) map.set(a.uid, a)
    return [...map.values()].sort((a, b) => a.name.localeCompare(b.name, 'ru'))
  }, [details, stacks])
  const labels = useMemo(() => {
    const map = new Map<number, DeckLabel>()
    for (const l of details?.labels ?? []) map.set(l.id, l)
    for (const c of stacks.flatMap((s) => s.cards)) for (const l of c.labels) if (!map.has(l.id)) map.set(l.id, l)
    return [...map.values()]
  }, [details, stacks])
  const filtered = isFiltered(filter)
  const totalCards = stacks.reduce((n, s) => n + s.cards.length, 0)
  const shownCards = filtered ? stacks.reduce((n, s) => n + s.cards.filter((c) => cardMatches(c, filter, me)).length, 0) : totalCards

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
    const title = await askText('Новая колонка', 'Название колонки')
    if (!title?.trim()) return
    try {
      await api.deckCreateStack(board.id, title)
      await loadStacks(board.id)
    } catch (e) {
      onNotice(`Не удалось создать колонку: ${errorText(e)}`)
    }
  }

  /** Перенести карточку в колонку — перед карточкой `before` (null — в конец); и внутри колонки. */
  async function move(card: DeckCard, targetStack: number, before: number | null = null) {
    if (!board) return
    const target = stacks.find((s) => s.id === targetStack)
    if (!target) return
    const rest = target.cards.filter((c) => c.id !== card.id)
    const pos = before === null ? -1 : rest.findIndex((c) => c.id === before)
    const index = pos < 0 ? rest.length : pos
    if (card.stackId === targetStack && target.cards.findIndex((c) => c.id === card.id) === index) return
    // Оптимистично переносим, при ошибке перечитываем доску.
    setStacks((list) => list.map((s) => {
      const cards = s.cards.filter((c) => c.id !== card.id)
      if (s.id === targetStack) cards.splice(index, 0, { ...card, stackId: targetStack })
      return { ...s, cards }
    }))
    try {
      await api.deckMoveCard(board.id, card.stackId, card.id, targetStack, index)
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
    /** Карточка колонки, перед которой окажется перетаскиваемая (по середине карточек). */
    function cardBefore(stack: number, y: number, dragged: number): number | null {
      const col = document.querySelector<HTMLElement>(`[data-stack="${stack}"]`)
      for (const el of col?.querySelectorAll<HTMLElement>('.deck-card[data-card]') ?? []) {
        const id = Number(el.dataset.card)
        if (id === dragged) continue
        const r = el.getBoundingClientRect()
        if (y < r.top + r.height / 2) return id
      }
      return null
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
      const before = target === null ? null : cardBefore(target, e.clientY, d.card.id)
      dropBeforeRef.current = before
      setDropBefore(before)
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
      const before = dropBeforeRef.current
      dropRef.current = null
      dropBeforeRef.current = null
      setGhost(null)
      setDragging(null)
      setDropStack(null)
      setDropBefore(null)
      if (commit && target !== null) void moveRef.current(d.card, target, before)
    }
    const up = (e: PointerEvent) => {
      // Колонку и место определяем и по точке отпускания — последнего pointermove могло не быть.
      const d = drag.current
      if (d?.active) {
        const at = stackAt(e.clientX, e.clientY)
        if (at !== null) { dropRef.current = at; dropBeforeRef.current = cardBefore(at, e.clientY, d.card.id) }
      }
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
  const links = useMemo(() => boardLinks(stacks), [stacks])
  const [hover, setHover] = useState<number | null>(null)
  const focusId = hover ?? selected?.id ?? null
  const relatedIds = useMemo(() => (focusId === null ? new Set<number>() : related(focusId, links)), [focusId, links])
  const nest = view.nest !== false

  // Группы досок: вручную назначенные, иначе — по названию проекта (общая часть до «:», «—», «/», […]).
  const [manualGroups, setManualGroups] = useState<Record<string, string>>(() => loadJson(GROUPS_KEY, {}))
  const [folded, setFolded] = useState<string[]>(() => loadJson(FOLDED_KEY, []))
  const grouped = useMemo(() => {
    const auto = new Map<string, number>()
    for (const b of activeBoards) { const p = projectOf(b.title); if (p) auto.set(p, (auto.get(p) ?? 0) + 1) }
    const groupOf = (b: DeckBoard): string => {
      const m = manualGroups[String(b.id)]
      if (m !== undefined) return m
      const p = projectOf(b.title)
      return p && (auto.get(p) ?? 0) >= 2 ? p : ''
    }
    const map = new Map<string, DeckBoard[]>()
    for (const b of activeBoards) { const g = groupOf(b); map.set(g, [...(map.get(g) ?? []), b]) }
    const named = [...map.keys()].filter(Boolean).sort((a, b) => a.localeCompare(b, 'ru'))
    return { named: named.map((g) => [g, map.get(g)!] as const), loose: map.get('') ?? [] }
  }, [activeBoards, manualGroups])
  function setGroup(b: DeckBoard, g: string | null) {
    setManualGroups((m) => {
      const next = { ...m }
      if (g === null) delete next[String(b.id)]
      else next[String(b.id)] = g
      saveJson(GROUPS_KEY, next)
      return next
    })
  }
  function toggleFold(g: string) {
    setFolded((f) => { const next = f.includes(g) ? f.filter((x) => x !== g) : [...f, g]; saveJson(FOLDED_KEY, next); return next })
  }
  function boardMenu(b: DeckBoard): MenuItem[] {
    const current = grouped.named.find(([, list]) => list.some((x) => x.id === b.id))?.[0] ?? ''
    const others = grouped.named.map(([g]) => g).filter((g) => g !== current)
    return [
      ...others.slice(0, 8).map((g): MenuItem => ({ label: `В группу «${g}»`, icon: <FolderInput size={15} />, onClick: () => setGroup(b, g) })),
      { label: 'В новую группу…', icon: <FolderInput size={15} />, onClick: () => { void askText('Новая группа', 'Название группы (проекта)', projectOf(b.title) ?? '').then((g) => { if (g?.trim()) setGroup(b, g.trim()) }) } },
      ...(current ? [{ label: 'Убрать из группы', icon: <FolderMinus size={15} />, onClick: () => setGroup(b, '') }] : []),
      ...(manualGroups[String(b.id)] !== undefined ? [{ label: 'Группа по названию (как было)', icon: <ListTree size={15} />, onClick: () => setGroup(b, null) }] : []),
      { separator: true },
      { label: 'Открыть в Nextcloud', icon: <ExternalLink size={15} />, onClick: () => void api.ncOpen(`apps/deck/board/${b.id}`) },
    ]
  }
  const boardButton = (b: DeckBoard, inGroup: boolean) => (
    <button key={b.id} className={`folder-item ${inGroup ? 'in-group' : ''} ${b.id === boardId ? 'selected' : ''}`} onClick={() => setBoardId(b.id)} title={`Владелец: ${b.owner}`}
      onContextMenu={(e) => openContextMenu(e, boardMenu(b))}>
      <span className="board-dot" style={{ background: `#${b.color || '0082c9'}` }} />
      <span>{b.title}</span>
      {!b.canEdit && <Lock size={13} />}
    </button>
  )

  return (
    <div className="module-view deck-view">
      {newBoard && (
        <div className="modal-backdrop nested" onMouseDown={(e) => { if (e.target === e.currentTarget && !newBoard.busy) setNewBoard(null) }}>
          <form className="settings-modal confirm-modal deck-ask" role="dialog" aria-label="Новая доска" onSubmit={(e) => { e.preventDefault(); void createBoard() }}
            onKeyDown={(e) => { if (e.key === 'Escape' && !newBoard.busy) { e.stopPropagation(); setNewBoard(null) } }}>
            <div className="compose-header"><div><strong>Новая доска</strong></div></div>
            <label className="deck-ask-field">Название
              <input autoFocus aria-label="Название доски" value={newBoard.title} maxLength={100} placeholder="Например: Проект Альфа"
                onChange={(e) => setNewBoard({ ...newBoard, title: e.target.value })} />
            </label>
            <div className="board-colors" role="radiogroup" aria-label="Цвет доски">
              {BOARD_COLORS.map((c) => (
                <button key={c} type="button" role="radio" aria-checked={newBoard.color === c} aria-label={`Цвет #${c}`} className={`board-color ${newBoard.color === c ? 'selected' : ''}`}
                  style={{ background: `#${c}` }} onClick={() => setNewBoard({ ...newBoard, color: c })} />
              ))}
            </div>
            <label className="deck-ask-check"><input type="checkbox" checked={newBoard.columns} onChange={(e) => setNewBoard({ ...newBoard, columns: e.target.checked })} /> Сразу создать колонки «К выполнению», «В работе», «Готово»</label>
            <div className="settings-actions">
              <button type="button" className="secondary-button" disabled={newBoard.busy} onClick={() => setNewBoard(null)}>Отмена</button>
              <button type="submit" className="primary-button" disabled={newBoard.busy || !newBoard.title.trim()}>{newBoard.busy ? 'Создаю…' : 'Создать доску'}</button>
            </div>
          </form>
        </div>
      )}
      {ask && (
        <div className="modal-backdrop nested" onMouseDown={(e) => { if (e.target === e.currentTarget) { ask.resolve(null); setAsk(null) } }}>
          <form className="settings-modal confirm-modal deck-ask" role="dialog" aria-label={ask.title} onSubmit={(e) => { e.preventDefault(); ask.resolve(ask.value); setAsk(null) }}
            onKeyDown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); ask.resolve(null); setAsk(null) } }}>
            <div className="compose-header"><div><strong>{ask.title}</strong></div></div>
            <label className="deck-ask-field">{ask.label}
              <input autoFocus aria-label={ask.label} value={ask.value} maxLength={100} onChange={(e) => setAsk({ ...ask, value: e.target.value })} />
            </label>
            <div className="settings-actions">
              <button type="button" className="secondary-button" onClick={() => { ask.resolve(null); setAsk(null) }}>Отмена</button>
              <button type="submit" className="primary-button" disabled={!ask.value.trim()}>Готово</button>
            </div>
          </form>
        </div>
      )}
      <aside className="folder-sidebar">
        <div className="sidebar-heading"><span>Доски</span>
          <button className="icon-button soft" aria-label="Новая доска" title="Новая доска" onClick={() => setNewBoard({ title: '', color: BOARD_COLORS[0], columns: true, busy: false })}><Plus size={16} /></button>
        </div>
        <div className="folder-list">
          {boards === null && !error && <SkeletonRows variant="folder" count={5} label="Загрузка досок…" />}
          {grouped.named.map(([g, list]) => {
            const open = !folded.includes(g) || list.some((b) => b.id === boardId)
            return (
              <div key={g} className="board-group" data-group={g}>
                <button className="board-group-head" aria-expanded={open} onClick={() => toggleFold(g)}>
                  {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}<span>{g}</span><b className="folder-count">{list.length}</b>
                </button>
                {open && list.map((b) => boardButton(b, true))}
              </div>
            )
          })}
          {grouped.named.length > 0 && grouped.loose.length > 0 && <div className="board-group-head static"><span>Без группы</span></div>}
          {grouped.loose.map((b) => boardButton(b, false))}
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
          <span className="deck-view-controls" role="group" aria-label="Размер колонок">
            <button className="icon-button soft" aria-label="Колонки мельче" title="Мельче" disabled={scaleIdx <= 0} onClick={() => updateView({ scale: DECK_SCALES[scaleIdx - 1] })}><Minus size={15} /></button>
            <span className="deck-scale-label" title="Размер колонок и карточек">{Math.round(view.scale * 100)} %</span>
            <button className="icon-button soft" aria-label="Колонки крупнее" title="Крупнее" disabled={scaleIdx >= DECK_SCALES.length - 1} onClick={() => updateView({ scale: DECK_SCALES[scaleIdx + 1] })}><Plus size={15} /></button>
            <button className={`icon-button soft ${nest ? 'active' : ''}`} aria-pressed={nest} aria-label="Подзадачи под главной" title={nest ? 'Подзадачи показаны под главной задачей — нажмите, чтобы показывать по порядку колонки' : 'Показывать подзадачи под главной задачей'} onClick={() => updateView({ nest: !nest })}><ListTree size={16} /></button>
            <button className={`icon-button soft ${view.stretch ? 'active' : ''}`} aria-pressed={view.stretch} aria-label="По ширине окна" title={view.stretch ? 'Колонки растянуты по ширине окна — нажмите, чтобы сделать фиксированной ширины' : 'Растянуть колонки по ширине окна'} onClick={() => updateView({ stretch: !view.stretch })}><MoveHorizontal size={16} /></button>
          </span>
          {editable && <button className="secondary-button" onClick={() => void addStack()}><Plus size={15} /> Колонка</button>}
        </div>
        {board && <div className="deck-filters" role="search" aria-label="Фильтр карточек">
          <Filter size={15} className="muted" />
          <select aria-label="Исполнитель" value={filter.who} onChange={(e) => updateFilter({ who: e.target.value })} className={filter.who !== 'all' ? 'active' : ''}>
            <option value="all">Все исполнители</option>
            <option value="me">Мои карточки</option>
            <option value="none">Без исполнителя</option>
            {people.length > 0 && <option disabled>──────────</option>}
            {people.map((u) => <option key={u.uid} value={`u:${u.uid}`}>{u.name}</option>)}
          </select>
          <select aria-label="Метка" value={filter.label} onChange={(e) => updateFilter({ label: Number(e.target.value) })} className={filter.label ? 'active' : ''}>
            <option value={0}>Все метки</option>
            {labels.map((l) => <option key={l.id} value={l.id}>{l.title}</option>)}
          </select>
          <select aria-label="Срок" value={filter.due} onChange={(e) => updateFilter({ due: e.target.value as CardFilter['due'] })} className={filter.due !== 'all' ? 'active' : ''}>
            <option value="all">Любой срок</option>
            <option value="overdue">Просрочено</option>
            <option value="today">Сегодня</option>
            <option value="week">На 7 дней</option>
            <option value="none">Без срока</option>
          </select>
          <label className="deck-filter-check"><input type="checkbox" checked={filter.hideDone} onChange={(e) => updateFilter({ hideDone: e.target.checked })} /> Скрыть выполненные</label>
          <label className="search-box deck-search"><Search size={15} /><input aria-label="Поиск карточек" value={filter.q} onChange={(e) => updateFilter({ q: e.target.value })} placeholder="Поиск по карточкам" /></label>
          {filtered && <>
            <span className="muted deck-filter-count">Показано {shownCards} из {totalCards}</span>
            <button className="text-button" onClick={() => updateFilter(NO_FILTER)}><X size={14} /> Сбросить</button>
          </>}
        </div>}
        {error && <div className="inline-warning" role="alert">Deck: {error}</div>}
        <div className={`deck-columns ${view.stretch ? 'stretch' : ''}`} ref={columnsRef}
          style={{ '--deck-col': `${Math.round(272 * view.scale)}px`, '--deck-font': `${(13 * view.scale).toFixed(1)}px` } as React.CSSProperties}>
          {stacks.map((s) => (
            <section
              key={s.id}
              data-stack={s.id}
              className={`deck-column ${dropStack === s.id && dragging ? 'drop' : ''} ${dropStack === s.id && dragging && dropBefore === null ? 'drop-end' : ''}`}
            >
              <header><strong>{s.title}</strong><span>{filtered ? `${s.cards.filter((c) => cardMatches(c, filter, me)).length}/${s.cards.length}` : s.cards.length}</span></header>
              <div className="deck-cards">
                {(() => {
                  const list = filtered ? s.cards.filter((c) => cardMatches(c, filter, me)) : s.cards
                  return nest ? nestInColumn(list, links) : list.map((card) => ({ card, depth: 0 }))
                })().map(({ card: c, depth }) => {
                  const due = dueInfo(c)
                  const l = links.links.get(c.id)
                  const kids = links.children.get(c.id) ?? []
                  const waits = blockedBy(c, links)
                  const parentTitle = l?.parent ? links.cards.get(l.parent.id)?.title ?? l.parent.title : null
                  return (
                    <button
                      key={c.id}
                      className={`deck-card ${selected?.id === c.id ? 'selected' : ''} ${c.done ? 'done' : ''} ${dragging?.id === c.id ? 'drag-source' : ''} ${relatedIds.has(c.id) ? 'related' : ''} ${waits.length ? 'blocked' : ''} ${depth ? `sub depth-${depth}` : ''} ${l?.parent ? 'is-subtask' : ''} ${dragging && dropStack === s.id && dropBefore === c.id ? 'drop-before' : ''}`}
                      data-card={c.id}
                      onMouseEnter={() => setHover(c.id)}
                      onMouseLeave={() => setHover((h) => (h === c.id ? null : h))}
                      onPointerDown={(e) => {
                        if (!editable || e.button !== 0) return
                        const r = e.currentTarget.getBoundingClientRect()
                        drag.current = { card: c, el: e.currentTarget, x0: e.clientX, y0: e.clientY, dx: e.clientX - r.left, dy: e.clientY - r.top, active: false }
                      }}
                      onDragStart={(e) => e.preventDefault()}
                      onClick={() => { if (!suppressClick.current) setSelected(c) }}
                      onDoubleClick={() => { setSelected(c); setFull(true) }}
                      title="Двойной щелчок — открыть карточку целиком"
                    >
                      {c.labels.length > 0 && <div className="deck-labels">{c.labels.map((l) => <span key={l.id} style={{ background: `#${l.color}` }}>{l.title}</span>)}</div>}
                      {parentTitle && !(nest && depth > 0) && <div className="deck-card-parent" title="Подзадача для этой карточки"><CornerDownRight size={12} /> {parentTitle}</div>}
                      <div className="deck-card-title">{c.done && <CheckCircle2 size={14} />} {c.title}</div>
                      {waits.length > 0 && <div className="deck-card-wait" title="Начинать после выполнения"><Lock size={11} /> после: {waits.map((w) => w.title).join(', ')}</div>}
                      <div className="deck-card-meta">
                        {due && <span className={due.overdue ? 'overdue' : ''}><CalendarClock size={12} /> {due.text}</span>}
                        {c.commentsUnread > 0 && <span><MessageSquare size={12} /> {c.commentsUnread}</span>}
                        {kids.length > 0 && <span className={`deck-kids ${kids.every((k) => k.done) ? 'all-done' : ''}`} title={`Подзадачи: ${kids.map((k) => k.title).join(', ')}`}><GitBranch size={12} /> {kids.filter((k) => k.done).length}/{kids.length}</span>}
                        {(links.next.get(c.id)?.length ?? 0) > 0 && <span title={`Далее: ${links.next.get(c.id)!.map((n) => n.title).join(', ')}`}><SkipForward size={12} /> {links.next.get(c.id)!.length}</span>}
                        {(() => { const p = checklistProgress(c.description); return p.total > 0 && <span className={p.done === p.total ? 'all-done' : ''} title="Чек-лист"><ListChecks size={12} /> {p.done}/{p.total}</span> })()}
                        {c.attachmentCount > 0 && <span><Paperclip size={12} /> {c.attachmentCount}</span>}
                        <span className="deck-people">{c.assigned.map((a) => <Avatar key={a.uid} id={a.uid} name={a.name} className="deck-avatar" />)}</span>
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
          full={full}
          onFull={setFull}
          onClose={() => { setSelected(null); setFull(false) }}
          onChanged={async (text) => { onNotice(text); await loadStacks(board.id) }}
          onRefresh={() => loadStacks(board.id, true)}
          onMove={(target) => void move(selected, target)}
          links={links}
          ncUrl={ncUrl}
          onOpenCard={(id) => { const c = links.cards.get(id); if (c) setSelected(c) }}
          chain={{ onOpenMail, onOpenFolder, onOpenOrg }}
        />
      )}
    </div>
  )
}

/** Описание карточки с оформлением Markdown (заголовки, списки, жирный, ссылки, код);
 *  строки-подзадачи «- [ ] …» показываются отдельно, списком с галочками. */
function DeckMarkdown({ text }: { text: string }) {
  const blocks: React.ReactNode[] = []
  let para: string[] = []
  let list: string[] = []
  const flush = () => {
    if (para.length) { blocks.push(<p key={`p${blocks.length}`}><TalkText text={para.join('\n')} /></p>); para = [] }
    if (list.length) { blocks.push(<ul key={`u${blocks.length}`}>{list.map((x, i) => <li key={i}><TalkText text={x} /></li>)}</ul>); list = [] }
  }
  for (const raw of text.split('\n')) {
    const line = raw.trimEnd()
    if (/^\s*[-*+]\s+\[[ xX]\]/.test(line)) continue
    const h = /^(#{1,4})\s+(.*)$/.exec(line)
    const li = /^\s*(?:[-*+]|\d+\.)\s+(.*)$/.exec(line)
    if (h) { flush(); blocks.push(h[1].length <= 2 ? <h3 key={`h${blocks.length}`}>{h[2]}</h3> : <h4 key={`h${blocks.length}`}>{h[2]}</h4>) }
    else if (li) { if (para.length) { const l = list; list = []; flush(); list = l } list.push(li[1]) }
    else if (!line.trim()) flush()
    else { if (list.length) { const p = para; para = []; flush(); para = p } para.push(line) }
  }
  flush()
  return <div className="deck-markdown">{blocks}</div>
}

function when(ts: string | number) {
  const d = typeof ts === 'number' ? new Date(ts * 1000) : new Date(ts)
  return Number.isNaN(d.getTime()) || d.getTime() <= 0 ? '' : d.toLocaleString('ru-RU', { day: 'numeric', month: 'short', year: 'numeric', hour: '2-digit', minute: '2-digit' })
}

function CardPanel({ board, card, stacks, full, onFull, onClose, onChanged, onRefresh, onMove, links, ncUrl, onOpenCard, chain }: {
  board: DeckBoard
  card: DeckCard
  stacks: DeckStack[]
  full: boolean
  onFull: (full: boolean) => void
  onClose: () => void
  onChanged: (text: string) => Promise<void>
  onRefresh: () => Promise<void>
  onMove: (stackId: number) => void
  links: BoardLinks
  ncUrl: string
  onOpenCard: (id: number) => void
  chain: ChainActions
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
  const [activity, setActivity] = useState<DeckActivity[] | null>(null)
  const [uploading, setUploading] = useState(false)
  const [editDesc, setEditDesc] = useState(false)
  const [picker, setPicker] = useState<'users' | 'labels' | null>(null)
  const ro = !board.canEdit
  // Описание без блока связей (его не показываем и не даём случайно стереть) и сами связи.
  const { body, links: own } = useMemo(() => splitLinks(description), [description])
  const [bodyEdit, setBodyEdit] = useState<string | null>(null)
  const [linkBusy, setLinkBusy] = useState(false)
  const [newSub, setNewSub] = useState('')
  const items = useMemo(() => parseChecklist(description), [description])
  const progress = items.length ? Math.round((items.filter((i) => i.done).length / items.length) * 100) : 0

  useEffect(() => {
    api.deckBoardDetails(board.id).then(setDetails).catch(() => undefined)
    api.deckComments(card.id).then(setComments).catch((e) => { setComments([]); setError(errorText(e)) })
    api.deckAttachments(board.id, card.stackId, card.id).then(setAttachments).catch(() => setAttachments([]))
    api.deckActivity(card.id).then(setActivity).catch(() => setActivity([]))
  }, [board.id, card.id, card.stackId])

  // Esc закрывает карточку целиком (возврат к доске), а не всю программу.
  useEffect(() => {
    if (!full) return
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.stopPropagation(); onFull(false) } }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [full, onFull])

  async function addFiles() {
    try {
      const picked = await api.pickAttachments()
      if (picked.length === 0) return
      setUploading(true)
      for (const f of picked) await api.deckAttachmentAdd(board.id, card.stackId, card.id, f.id)
      setAttachments(await api.deckAttachments(board.id, card.stackId, card.id))
      await onChanged(picked.length === 1 ? `Файл «${picked[0].name}» добавлен к карточке.` : `Добавлено файлов: ${picked.length}.`)
    } catch (e) {
      setError(errorText(e))
    } finally {
      setUploading(false)
    }
  }

  async function removeFile(a: DeckAttachment) {
    if (!window.confirm(`Убрать вложение «${a.name}» из карточки?`)) return
    try {
      await api.deckAttachmentDelete(board.id, card.stackId, card.id, a.id, a.kind)
      setAttachments((list) => list?.filter((x) => x.id !== a.id) ?? list)
      await onRefresh()
    } catch (e) {
      setError(errorText(e))
    }
  }

  async function toggleArchive() {
    try {
      await api.deckArchive(board.id, card.stackId, card.id, !card.archived)
      onClose()
      await onChanged(card.archived ? 'Карточка возвращена из архива.' : 'Карточка перенесена в архив доски.')
    } catch (e) {
      setError(errorText(e))
    }
  }

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

  // --- Связи карточек: главная задача, подзадачи, «выполняется после» ---
  const base = ncUrl || 'https://nextcloud'
  const kids = links.children.get(card.id) ?? []
  const nextCards = links.next.get(card.id) ?? []
  const titleOf = (id: number, fallback: string) => links.cards.get(id)?.title ?? fallback

  async function setOwnLinks(next: CardLinks, done: string) {
    setLinkBusy(true)
    try {
      await saveDescription(joinLinks(body, next, base))
      await onChanged(done)
    } finally {
      setLinkBusy(false)
    }
  }
  /** Изменить связи другой карточки (сделать её подзадачей этой и т. п.). */
  async function setLinksOf(other: DeckCard, change: (l: CardLinks) => CardLinks, done: string) {
    setLinkBusy(true)
    try {
      const { body: b, links: l } = splitLinks(other.description)
      await api.deckUpdateCard(board.id, other.stackId, other.id, other.title, joinLinks(b, change(l), base), other.duedate, other.done)
      await onChanged(done)
    } catch (e) {
      setError(errorText(e))
    } finally {
      setLinkBusy(false)
    }
  }
  async function createSubtask() {
    const t = newSub.trim()
    if (!t) return
    setNewSub('')
    setLinkBusy(true)
    try {
      await api.deckCreateCard(board.id, card.stackId, t, joinLinks('', { parent: { id: card.id, title: title || card.title }, after: [] }, base))
      await onChanged(`Подзадача «${t}» создана`)
    } catch (e) {
      setError(errorText(e))
    } finally {
      setLinkBusy(false)
    }
  }
  /** Карточки доски для выбора связи (без самой карточки). */
  const pickable = (exclude: (c: DeckCard) => boolean) => (
    stacks.map((st) => ({ st, cards: st.cards.filter((c) => c.id !== card.id && !exclude(c)) })).filter((g) => g.cards.length > 0)
  )
  const cardSelect = (label: string, exclude: (c: DeckCard) => boolean, onPick: (c: DeckCard) => void) => (
    <select className="deck-link-select" aria-label={label} value="" disabled={ro || linkBusy}
      onChange={(e) => { const c = links.cards.get(Number(e.target.value)); if (c) onPick(c) }}>
      <option value="">{label}</option>
      {pickable(exclude).map(({ st, cards }) => (
        <optgroup key={st.id} label={st.title}>{cards.map((c) => <option key={c.id} value={c.id}>{c.title}</option>)}</optgroup>
      ))}
    </select>
  )
  const chip = (id: number, text: string, onRemove?: () => void) => (
    <span className={`deck-link-chip ${links.cards.get(id)?.done ? 'done' : ''}`} key={id}>
      <button type="button" className="deck-link-open" title="Открыть карточку" disabled={!links.cards.has(id)} onClick={() => onOpenCard(id)}>
        {links.cards.get(id)?.done && <CheckCircle2 size={12} />}{text}
      </button>
      {onRemove && !ro && <button type="button" className="deck-link-x" aria-label={`Убрать связь с «${text}»`} title="Убрать связь" disabled={linkBusy} onClick={onRemove}><X size={12} /></button>}
    </span>
  )
  const sectionLinks = (
    <div className="deck-section deck-links">
      <div className="deck-section-head"><b><Link2 size={14} /> Связи</b></div>
      {(own.org || own.mail || own.folder) && (
        <div className="deck-chain" aria-label="Цепочка задачи">
          {own.org && <button type="button" className="deck-chain-item" title="История взаимодействия с организацией" onClick={() => chain.onOpenOrg?.(own.org!)}><Handshake size={13} /> {own.org}</button>}
          {own.mail && <button type="button" className="deck-chain-item" title="Открыть письмо, из которого создана задача" onClick={() => chain.onOpenMail?.(own.mail!.msgId)}><Mail size={13} /> {own.mail.title}</button>}
          {own.folder && <button type="button" className="deck-chain-item" title={`Папка документов: ${own.folder.path}`} onClick={() => chain.onOpenFolder?.(own.folder!.path)}><FolderOpen size={13} /> Документы</button>}
        </div>
      )}
      <div className="deck-link-row">
        <span className="deck-link-label"><CornerDownRight size={13} /> Главная задача</span>
        {own.parent
          ? chip(own.parent.id, titleOf(own.parent.id, own.parent.title), () => void setOwnLinks({ ...own, parent: null }, 'Карточка больше не подзадача'))
          : !ro && cardSelect('Сделать подзадачей…', (c) => kids.some((k) => k.id === c.id) || links.links.get(c.id)?.parent?.id === card.id,
              (c) => void setOwnLinks({ ...own, parent: { id: c.id, title: c.title } }, `Теперь подзадача «${c.title}»`))}
        {!own.parent && ro && <span className="muted">нет</span>}
      </div>
      <div className="deck-link-row">
        <span className="deck-link-label"><GitBranch size={13} /> Подзадачи{kids.length ? ` ${kids.filter((k) => k.done).length}/${kids.length}` : ''}</span>
        <span className="deck-link-chips">
          {kids.map((k) => chip(k.id, k.title, () => void setLinksOf(k, (l) => ({ ...l, parent: null }), `«${k.title}» больше не подзадача`)))}
          {kids.length === 0 && ro && <span className="muted">нет</span>}
        </span>
      </div>
      {!ro && (
        <div className="deck-link-row deck-link-add">
          <form onSubmit={(e) => { e.preventDefault(); void createSubtask() }}>
            <Plus size={14} />
            <input value={newSub} disabled={linkBusy} onChange={(e) => setNewSub(e.target.value)} aria-label="Новая подзадача-карточка" placeholder="Новая подзадача-карточка — Enter" />
          </form>
          {cardSelect('Привязать существующую…', (c) => kids.some((k) => k.id === c.id) || own.parent?.id === c.id,
            (c) => void setLinksOf(c, (l) => ({ ...l, parent: { id: card.id, title: title || card.title } }), `«${c.title}» — подзадача этой карточки`))}
        </div>
      )}
      <div className="deck-link-row">
        <span className="deck-link-label"><Lock size={13} /> Выполняется после</span>
        <span className="deck-link-chips">
          {own.after.map((a) => chip(a.id, titleOf(a.id, a.title), () => void setOwnLinks({ ...own, after: own.after.filter((x) => x.id !== a.id) }, 'Связь убрана')))}
          {!ro && cardSelect('Добавить предыдущую…', (c) => own.after.some((a) => a.id === c.id) || wouldCycle(card.id, c.id, links),
            (c) => void setOwnLinks({ ...own, after: [...own.after, { id: c.id, title: c.title }] }, `Выполняется после «${c.title}»`))}
          {own.after.length === 0 && ro && <span className="muted">нет</span>}
        </span>
      </div>
      {nextCards.length > 0 && (
        <div className="deck-link-row">
          <span className="deck-link-label"><SkipForward size={13} /> Следующие</span>
          <span className="deck-link-chips">{nextCards.map((n) => chip(n.id, n.title))}</span>
        </div>
      )}
      {blockedBy(card, links).length > 0 && <div className="inline-warning deck-wait-note"><Lock size={13} /> Ждёт выполнения: {blockedBy(card, links).map((c) => c.title).join(', ')}</div>}
    </div>
  )

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

  const sectionTasks = (
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
            {!ro && <button aria-label={`Удалить пункт ${it.text}`} title="Удалить" onClick={() => void saveDescription(removeItem(description, it.line))}><X size={13} /></button>}
          </li>
        ))}
      </ul>
      {!ro && (
        <form className="deck-add-item" onSubmit={(e) => { e.preventDefault(); void addSubtask() }}>
          <Plus size={15} />
          <input value={newItem} onChange={(e) => setNewItem(e.target.value)} placeholder="Новый пункт чек-листа — Enter, чтобы добавить" />
        </form>
      )}
      {items.length === 0 && ro && <div className="muted">Подзадач нет.</div>}
    </div>
  )

  const descriptionEditor = <textarea rows={full ? 10 : 6} value={bodyEdit ?? body} disabled={ro} aria-label="Описание"
    onChange={(e) => { setBodyEdit(e.target.value); setDescription(joinLinks(e.target.value, own, ncUrl || 'https://nextcloud')) }}
    placeholder="Поддерживается Markdown; пункты чек-листа — строки «- [ ] …»" />
  const sectionDescription = full ? (
    <div className="deck-section">
      <div className="deck-section-head"><b>Описание</b>{!ro && <button className="text-button" onClick={() => setEditDesc(!editDesc)}><Pencil size={13} /> {editDesc ? 'Просмотр' : 'Изменить'}</button>}</div>
      {editDesc ? descriptionEditor : body.replace(/^\s*[-*+]\s+\[[ xX]\].*$/gm, '').trim() ? <DeckMarkdown text={body} /> : <div className="muted">Описания нет.</div>}
    </div>
  ) : <label className="login-field"><span>Описание</span>{descriptionEditor}</label>

  const sectionComments = (
    <div className="deck-comments">
      {comments === null && <SkeletonRows variant="line" count={3} label="Загрузка комментариев…" />}
      {comments?.length === 0 && <div className="muted">Комментариев пока нет.</div>}
      {comments?.map((c) => (
        <div key={c.id} className={`deck-comment ${c.own ? 'own' : ''}`}>
          <Avatar id={c.authorId} name={c.author} />
          <div>
            <div className="deck-comment-head"><b>{c.author}</b><span>{c.created ? new Date(c.created).toLocaleString('ru-RU', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' }) : ''}</span>
              {c.own && <button aria-label="Удалить комментарий" title="Удалить" onClick={() => void deleteComment(c)}><Trash2 size={12} /></button>}</div>
            <div className="deck-comment-text"><TalkText text={c.message} /></div>
          </div>
        </div>
      ))}
      <form className="deck-add-item" onSubmit={(e) => { e.preventDefault(); void sendComment() }}>
        <MessageSquare size={15} />
        <input value={comment} onChange={(e) => setComment(e.target.value)} aria-label="Новый комментарий" placeholder="Комментарий — Enter, чтобы отправить" />
      </form>
    </div>
  )

  const sectionFiles = (
    <div className="deck-files">
      {attachments === null && <SkeletonRows variant="file" count={2} label="Загрузка файлов…" />}
      {attachments?.length === 0 && <div className="muted">Вложений нет.</div>}
      {attachments?.map((a) => (
        <div key={a.id} className="talk-file-row deck-file-row">
          <button className="talk-file" title="Открыть в Nextcloud"
            onClick={() => void api.ncOpen(a.fileId ? `f/${a.fileId}` : `apps/deck/board/${board.id}/card/${card.id}`)}>
            <FileText size={15} /><span>{a.name}</span>{a.size > 0 && <small>{formatSize(a.size)}</small>}
          </button>
          {!ro && <button className="icon-button soft" aria-label={`Убрать вложение ${a.name}`} title="Убрать из карточки" onClick={() => void removeFile(a)}><X size={14} /></button>}
        </div>
      ))}
      {!ro && <button className="text-button deck-upload" disabled={uploading} onClick={() => void addFiles()}><Upload size={14} /> {uploading ? 'Загрузка…' : 'Добавить файл'}</button>}
    </div>
  )

  const sectionActivity = activity && activity.length > 0 && (
    <div className="deck-section deck-activity">
      <div className="deck-section-head"><b><History size={14} /> История</b></div>
      <ul>{activity.map((a) => <li key={a.id}><span>{a.subject}</span><time>{when(a.date)}</time></li>)}</ul>
    </div>
  )

  const meta = (
    <>
      <div className="editor-row">
        <label className="login-field"><span>Колонка</span>
          <select value={card.stackId} disabled={ro} onChange={(e) => onMove(Number(e.target.value))}>
            {stacks.map((s) => <option key={s.id} value={s.id}>{s.title}</option>)}
          </select>
        </label>
        <div className="login-field"><span>Срок</span><DateField label="Срок карточки" value={due} disabled={ro} onChange={setDue} /></div>
        <label className="login-remember deck-done"><input type="checkbox" checked={done} disabled={ro} onChange={(e) => setDone(e.target.checked)} /><span>Выполнено</span></label>
      </div>
      <div className="deck-people-row">
        <span className="muted">Исполнители</span>
        <div className="deck-chips">
          {assigned.map((u) => <span key={u.uid} className="deck-chip"><Avatar id={u.uid} name={u.name} className="deck-avatar" />{u.name}{!ro && <button aria-label={`Снять ${u.name}`} onClick={() => void toggleUser(u)}><X size={11} /></button>}</span>)}
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
    </>
  )

  const actions = !ro && (
    <div className="reader-actions">
      <button className="primary-button" disabled={busy} onClick={() => void save()}>{busy ? 'Сохранение…' : 'Сохранить'}</button>
      <button className="secondary-button" onClick={() => void toggleArchive()}>{card.archived ? <><ArchiveRestore size={15} /> Из архива</> : <><Archive size={15} /> В архив</>}</button>
      <button className="secondary-button danger" onClick={() => void remove()}><Trash2 size={15} /> Удалить</button>
    </div>
  )

  const toolbar = (
    <div className="reader-toolbar">
      <button className="icon-button soft" aria-label="Открыть в Nextcloud" title="Открыть в Nextcloud" onClick={() => void api.ncOpen(`apps/deck/board/${board.id}/card/${card.id}`)}><ExternalLink size={17} /></button>
      <button className="icon-button soft" aria-label={full ? 'Свернуть карточку' : 'Открыть карточку целиком'} title={full ? 'Свернуть в панель' : 'Открыть целиком'} onClick={() => onFull(!full)}>{full ? <Minimize2 size={17} /> : <Maximize2 size={17} />}</button>
      {full && <span className="deck-card-info">{[card.owner && `Создал(а): ${card.owner}`, when(card.createdAt) && `создана ${when(card.createdAt)}`, when(card.lastModified) && `изменена ${when(card.lastModified)}`].filter(Boolean).join(' · ')}</span>}
      <div className="reader-toolbar-spacer" />
      <button className="icon-button soft" aria-label="Закрыть" onClick={onClose}><X size={18} /></button>
    </div>
  )

  if (full) {
    return (
      <div className="modal-backdrop deck-full-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onFull(false) }}>
        <section className="deck-full" role="dialog" aria-label={`Карточка ${card.title}`}>
          {toolbar}
          <div className="deck-full-body editor-modal deck-card-detail">
            <div className="deck-full-main">
              <input className="deck-title-input" value={title} disabled={ro} onChange={(e) => setTitle(e.target.value)} aria-label="Название" />
              {card.archived && <div className="inline-warning">Карточка в архиве доски.</div>}
              {meta}
              {sectionDescription}
              {sectionLinks}
              <div className="deck-section"><div className="deck-section-head"><b><ListChecks size={14} /> Чек-лист{items.length ? ` ${items.filter((i) => i.done).length}/${items.length}` : ''}</b></div>{sectionTasks}</div>
              <div className="deck-section"><div className="deck-section-head"><b><Paperclip size={14} /> Вложения{attachments?.length ? ` ${attachments.length}` : ''}</b></div>{sectionFiles}</div>
              {error && <div className="login-error" role="alert">{error}</div>}
              {actions}
            </div>
            <aside className="deck-full-side">
              <div className="deck-section"><div className="deck-section-head"><b><MessageSquare size={14} /> Комментарии{comments?.length ? ` ${comments.length}` : ''}</b></div>{sectionComments}</div>
              {sectionActivity}
            </aside>
          </div>
        </section>
      </div>
    )
  }

  return (
    <section className="reader-pane deck-panel">
      {toolbar}
      <div className="reader-content editor-modal deck-card-detail">
        <input className="deck-title-input" value={title} disabled={ro} onChange={(e) => setTitle(e.target.value)} aria-label="Название" />
        {meta}
        <div className="deck-tabs" role="tablist">
          <button role="tab" className={tab === 'tasks' ? 'active' : ''} onClick={() => setTab('tasks')}><ListChecks size={14} /> Задачи{items.length ? ` ${items.filter((i) => i.done).length}/${items.length}` : ''}</button>
          <button role="tab" className={tab === 'comments' ? 'active' : ''} onClick={() => setTab('comments')}><MessageSquare size={14} /> Комментарии{comments?.length ? ` ${comments.length}` : ''}</button>
          <button role="tab" className={tab === 'files' ? 'active' : ''} onClick={() => setTab('files')}><Paperclip size={14} /> Вложения{attachments?.length ? ` ${attachments.length}` : ''}</button>
        </div>
        {tab === 'tasks' && <>{sectionLinks}{sectionTasks}{sectionDescription}</>}
        {tab === 'comments' && sectionComments}
        {tab === 'files' && sectionFiles}
        {error && <div className="login-error" role="alert">{error}</div>}
        {actions}
      </div>
    </section>
  )
}
