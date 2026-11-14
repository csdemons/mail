// Карточка Deck: описание, подзадачи, исполнители, метки, комментарии, файлы, связи, история.
import { useEffect, useMemo, useState } from 'react'
import { DateField } from './DateField'
import { Archive, ArchiveRestore, CheckCircle2, CornerDownRight, ExternalLink, FileText, FolderOpen, GitBranch, Handshake, History, Link2, ListChecks, Lock, Mail, Maximize2, MessageSquare, Minimize2, Paperclip, Pencil, Plus, SkipForward, Trash2, Upload, X } from 'lucide-react'

import { api, errorText, formatSize } from './api'
import { addItem, parseChecklist, removeItem, renameItem, toggleItem } from './checklist'
import Avatar from './Avatar'
import TalkText from './TalkText'
import type { DeckActivity, DeckAttachment, DeckBoard, DeckBoardDetails, DeckCard, DeckComment, DeckLabel, DeckStack, DeckUser } from './types'
import { SkeletonRows } from './Skeleton'
import { blockedBy, joinLinks, splitLinks, wouldCycle, type BoardLinks, type CardLinks } from './deckLinks'
import { type ChainActions, dateInputValue } from './deckUtils'

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

export default function CardPanel({ board, card, stacks, full, onFull, onClose, onChanged, onRefresh, onMove, links, ncUrl, onOpenCard, chain }: {
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
