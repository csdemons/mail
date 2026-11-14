import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { CalendarClock, CheckCircle2, ChevronDown, ChevronRight, CornerDownRight, ExternalLink, Filter, FolderInput, FolderMinus, GitBranch, ListChecks, ListTree, Lock, MessageSquare, Minus, MoveHorizontal, Paperclip, Plus, RefreshCw, Search, SkipForward, X } from 'lucide-react'

import { api, errorText } from './api'
import { checklistProgress } from './checklist'
import Avatar from './Avatar'
import type { DeckBoard, DeckBoardDetails, DeckCard, DeckLabel, DeckStack, DeckUser } from './types'
import { SkeletonRows } from './Skeleton'
import { openContextMenu, type MenuItem } from './PopupMenu'
import { blockedBy, boardLinks, nestInColumn, projectOf, related } from './deckLinks'
import { BOARD_COLORS, type CardFilter, type ChainActions, DECK_SCALES, FILTER_KEY, FOLDED_KEY, GROUPS_KEY, LAST_BOARD_KEY, NO_FILTER, cardMatches, dueInfo, isFiltered, loadDeckView, loadFilter, loadJson, saveJson } from './deckUtils'
import CardPanel from './DeckCardPanel'

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
