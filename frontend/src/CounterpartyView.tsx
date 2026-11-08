import { useEffect, useMemo, useState } from 'react'
import { DateField } from './DateField'
import { Building2, CalendarDays, Check, Cloud, CloudDownload, FileText, Handshake, Inbox, Loader2, Mail, MessageCircle, Paperclip, Pencil, Plus, RefreshCw, Search, Send, SquareKanban, SquarePen, Trash2, Users, X } from 'lucide-react'

import { api, errorText } from './api'
import type { Agreement, CalEvent, ChatHit, Counterparty, CounterpartyTimeline, DeckHit, Dossier, MessageSummary, NcFile, NcSearch } from './types'
import type { Found } from './GlobalSearch'
import Splitter from './Splitter'

interface Props {
  /** Организация, которую открыть (например, из меню адреса в письме). */
  domain: string | null
  onDomain: (domain: string | null) => void
  onCompose: (to: string) => void
  onNotice: (text: string) => void
  /** Открыть найденное в своём разделе (файл, задачу, беседу, встречу). */
  onOpen?: (f: Found) => void
  /** Nextcloud подключён — в историю попадают файлы, задачи Deck и чат. */
  ncConnected?: boolean
  /** Создать задачу из письма. */
  onTask?: (m: MessageSummary) => void
}

type Kind = 'all' | 'mail' | 'event' | 'document' | 'file' | 'task' | 'chat' | 'agreement'

type Item =
  | { kind: 'mail'; key: string; date: Date; m: MessageSummary; incoming: boolean }
  | { kind: 'event'; key: string; date: Date; e: CalEvent }
  | { kind: 'document'; key: string; date: Date; m: MessageSummary; name: string; incoming: boolean }
  | { kind: 'agreement'; key: string; date: Date; a: Agreement }
  | { kind: 'file'; key: string; date: Date; f: NcFile }
  | { kind: 'task'; key: string; date: Date; h: DeckHit }
  | { kind: 'chat'; key: string; date: Date; h: ChatHit }

const KINDS: { id: Kind; label: string }[] = [
  { id: 'all', label: 'Всё' },
  { id: 'mail', label: 'Письма' },
  { id: 'event', label: 'Встречи' },
  { id: 'document', label: 'Вложения' },
  { id: 'file', label: 'Файлы' },
  { id: 'task', label: 'Задачи' },
  { id: 'chat', label: 'Чат' },
  { id: 'agreement', label: 'Договорённости' },
]

const today = () => {
  const d = new Date()
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
}
const dayDate = (s: string) => new Date(`${s}T12:00:00`)
const fmtDay = (d: Date) => d.toLocaleDateString('ru-RU', { day: 'numeric', month: 'short', year: 'numeric' })
const fmtTime = (d: Date) => d.toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' })
const monthKey = (d: Date) => d.toLocaleDateString('ru-RU', { month: 'long', year: 'numeric' })

function inDomain(addr: string, dom: string): boolean {
  const m = /<([^>]+)>/.exec(addr)
  const a = (m ? m[1] : addr).trim().toLowerCase()
  const d = a.split('@')[1] ?? ''
  return d === dom || d.endsWith(`.${dom}`)
}

function eventDate(e: CalEvent): Date {
  return e.allDay ? dayDate(e.start) : new Date(e.start)
}

/** Хронология: письма, их вложения, встречи, договорённости, файлы, задачи и чат — новые сверху. */
export function buildTimeline(t: CounterpartyTimeline, nc: NcSearch | null = null): Item[] {
  const out: Item[] = []
  for (const m of t.messages) {
    const date = m.date ? new Date(m.date) : new Date(0)
    const incoming = inDomain(m.fromAddress, t.domain)
    out.push({ kind: 'mail', key: `m:${m.folder}:${m.uid}`, date, m, incoming })
    // Сводка письма несёт имена вложений (`attachments`), у полного письма это поле другое.
    for (const [i, name] of ((m as MessageSummary & { attachments?: string[] }).attachments ?? []).entries()) {
      out.push({ kind: 'document', key: `d:${m.folder}:${m.uid}:${i}`, date, m, name, incoming })
    }
  }
  for (const e of t.events) out.push({ kind: 'event', key: `e:${e.id}`, date: eventDate(e), e })
  for (const a of t.dossier.agreements ?? []) out.push({ kind: 'agreement', key: `a:${a.id}`, date: dayDate(a.date), a })
  for (const f of nc?.files ?? []) {
    if (f.isDir) continue
    const d = new Date(f.modified)
    out.push({ kind: 'file', key: `f:${f.path}`, date: Number.isNaN(d.getTime()) ? new Date(0) : d, f })
  }
  for (const h of nc?.cards ?? []) out.push({ kind: 'task', key: `k:${h.card.id}`, date: new Date((h.card.createdAt || h.card.lastModified) * 1000), h })
  for (const h of nc?.chat ?? []) {
    if (h.messageId !== null) out.push({ kind: 'chat', key: `t:${h.token}:${h.messageId}`, date: new Date(h.timestamp * 1000), h })
  }
  out.sort((x, y) => y.date.getTime() - x.date.getTime())
  return out
}

/** Объединить результаты нескольких поисков Nextcloud без повторов. */
function mergeNc(list: NcSearch[]): NcSearch {
  const out: NcSearch = { files: [], cards: [], chat: [], errors: [] }
  for (const r of list) {
    for (const f of r.files) if (!out.files.some((x) => x.path === f.path)) out.files.push(f)
    for (const h of r.cards) if (!out.cards.some((x) => x.card.id === h.card.id)) out.cards.push(h)
    for (const h of r.chat) if (!out.chat.some((x) => x.token === h.token && x.messageId === h.messageId)) out.chat.push(h)
    for (const e of r.errors) if (!out.errors.includes(e)) out.errors.push(e)
  }
  return out
}

export default function CounterpartyView({ domain, onDomain, onCompose, onNotice, onOpen, ncConnected = false, onTask }: Props) {
  const [list, setList] = useState<Counterparty[] | null>(null)
  const [listError, setListError] = useState('')
  const [filter, setFilter] = useState('')
  const [data, setData] = useState<CounterpartyTimeline | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [kind, setKind] = useState<Kind>('all')
  const [years, setYears] = useState(1)
  const [openEvent, setOpenEvent] = useState<string | null>(null)
  const [editingName, setEditingName] = useState<string | null>(null)
  const [note, setNote] = useState('')
  const [form, setForm] = useState<Agreement | null>(null)

  async function loadList() {
    setListError('')
    try {
      setList(await api.counterparties())
    } catch (err) {
      setListError(errorText(err))
      setList([])
    }
  }
  useEffect(() => { void loadList() }, [])

  async function load(dom: string, server: boolean, y = years) {
    setLoading(true)
    setError('')
    try {
      const t = await api.counterpartyTimeline(dom, server, y)
      setData(t)
      setNote(t.dossier.note ?? '')
    } catch (err) {
      setError(errorText(err))
      setData(null)
    } finally {
      setLoading(false)
    }
  }
  useEffect(() => {
    setData(null)
    setForm(null)
    setEditingName(null)
    setKind('all')
    if (domain) void load(domain, false)
  }, [domain]) // eslint-disable-line react-hooks/exhaustive-deps

  async function saveDossier(next: Dossier, done?: string) {
    if (!data) return
    try {
      await api.counterpartySave(data.domain, next)
      setData({ ...data, dossier: next })
      if (done) onNotice(done)
      void loadList()
    } catch (err) {
      onNotice(`Не удалось сохранить: ${errorText(err)}`)
    }
  }

  // Файлы, задачи Deck и чат — общим поиском Nextcloud: по домену, по названию организации
  // и всё из её папки «/Контрагенты/<домен>».
  const [nc, setNc] = useState<NcSearch | null>(null)
  const [ncBusy, setNcBusy] = useState(false)
  const loadedDomain = data?.domain
  const dossierName = data?.dossier.name ?? null
  useEffect(() => {
    setNc(null)
    if (!loadedDomain || !ncConnected) return
    let off = false
    setNcBusy(true)
    const name = dossierName?.trim()
    Promise.all([api.ncSearch(loadedDomain, loadedDomain, 100), ...(name && name.length >= 3 ? [api.ncSearch(name, null, 100)] : [])])
      .then((list) => { if (!off) setNc(mergeNc(list)) })
      .catch((e) => { if (!off) setNc({ files: [], cards: [], chat: [], errors: [`Nextcloud: ${errorText(e)}`] }) })
      .finally(() => { if (!off) setNcBusy(false) })
    return () => { off = true }
  }, [loadedDomain, dossierName, ncConnected])

  const items = useMemo(() => (data ? buildTimeline(data, nc) : []), [data, nc])
  const shown = useMemo(() => items.filter((i) => kind === 'all' || i.kind === kind), [items, kind])
  const counts = useMemo(() => {
    const c: Record<Kind, number> = { all: items.length, mail: 0, event: 0, document: 0, file: 0, task: 0, chat: 0, agreement: 0 }
    for (const i of items) c[i.kind] += 1
    return c
  }, [items])

  const orgName = data ? (data.dossier.name || data.contacts.find((c) => c.org)?.org || null) : null
  const contactOf = (address: string) => data?.contacts.find((c) => c.emails.some((e) => e.toLowerCase() === address))

  const visibleList = useMemo(() => {
    const q = filter.trim().toLowerCase().replace(/^@/, '')
    return (list ?? []).filter((c) => !q || c.domain.includes(q) || (c.name ?? '').toLowerCase().includes(q))
  }, [list, filter])
  const typedDomain = (() => {
    const q = filter.trim().toLowerCase().replace(/^@/, '')
    return /^[\p{L}\d-]+(\.[\p{L}\d-]+)+$/u.test(q) && !(list ?? []).some((c) => c.domain === q) ? q : null
  })()

  function agreementFrom(m?: MessageSummary): Agreement {
    return {
      id: `a${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`,
      date: m?.date ? m.date.slice(0, 10) : today(),
      text: '',
      done: false,
      ...(m ? { folder: m.folder, uid: m.uid, subject: m.subject } : {}),
    }
  }

  function submitAgreement() {
    if (!data || !form) return
    const text = form.text.trim()
    if (!text) { onNotice('Опишите договорённость.'); return }
    const list = data.dossier.agreements ?? []
    const next = { ...form, text, due: form.due || undefined }
    const agreements = list.some((a) => a.id === form.id) ? list.map((a) => (a.id === form.id ? next : a)) : [...list, next]
    void saveDossier({ ...data.dossier, agreements }, 'Договорённость сохранена.')
    setForm(null)
  }

  function setAgreementDone(a: Agreement, done: boolean) {
    if (!data) return
    void saveDossier({ ...data.dossier, agreements: (data.dossier.agreements ?? []).map((x) => (x.id === a.id ? { ...x, done } : x)) })
  }

  function deleteAgreement(a: Agreement) {
    if (!data) return
    void saveDossier({ ...data.dossier, agreements: (data.dossier.agreements ?? []).filter((x) => x.id !== a.id) }, 'Договорённость удалена.')
  }

  const openMessage = (m: { folder?: string; uid?: number; subject?: string }) => {
    if (m.folder == null || m.uid == null) return
    api.openMessageWindow(m.folder, m.uid, m.subject || '(без темы)').catch((err) => onNotice(`Не удалось открыть письмо: ${errorText(err)}`))
  }

  const overdue = (a: Agreement) => !a.done && !!a.due && a.due < today()
  const openAgreements = (data?.dossier.agreements ?? []).filter((a) => !a.done)
  const firstDate = items.length ? items[items.length - 1].date : null
  const lastMail = data?.messages[0]?.date ? new Date(data.messages[0].date) : null

  let lastMonth = ''

  return <div className="module-view counterparty-view">
    <section className="message-column counterparty-list">
      <div className="list-toolbar">
        <label className="search-box">
          <Search size={15} />
          <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Организация или домен" aria-label="Найти организацию" />
        </label>
        <button className="icon-button" title="Обновить список" aria-label="Обновить список" onClick={() => void loadList()}><RefreshCw size={16} /></button>
      </div>
      {typedDomain && <button className="counterparty-row counterparty-new" onClick={() => { onDomain(typedDomain); setFilter('') }}>
        <Plus size={15} /><span>Открыть историю <b>{typedDomain}</b></span>
      </button>}
      {listError && <div className="inline-warning">{listError}</div>}
      <div className="counterparty-items">
        {list === null && <div className="empty-state"><Loader2 className="spin" size={18} /></div>}
        {list !== null && visibleList.length === 0 && !typedDomain && <div className="empty-state muted">
          {filter ? 'Ничего не найдено. Введите домен, например ihep.ru, чтобы открыть его историю.' : 'Переписки с организациями на компьютере пока нет.'}
        </div>}
        {visibleList.map((c) => (
          <button key={c.domain} className={`counterparty-row ${c.domain === domain ? 'selected' : ''}`} data-domain={c.domain} onClick={() => onDomain(c.domain)}>
            <span className="counterparty-icon"><Building2 size={16} /></span>
            <span className="counterparty-main">
              <span className="counterparty-title">
                <b>{c.name || c.domain}</b>
                {c.own && <small className="counterparty-own">своя организация</small>}
              </span>
              <small className="muted">{c.name ? `${c.domain} · ` : ''}{c.messages > 0 ? `${c.messages} ${plural(c.messages, 'письмо', 'письма', 'писем')}` : 'без писем'}{c.last ? ` · ${fmtDay(new Date(c.last))}` : ''}</small>
            </span>
            {c.openAgreements > 0 && <span className="counterparty-badge" title="Открытые договорённости">{c.openAgreements}</span>}
          </button>
        ))}
      </div>
    </section>
    <Splitter id="counterparty-list" min={240} max={700} label="Ширина списка контрагентов" />

    <section className="counterparty-main-pane">
      {!domain && <div className="empty-state counterparty-empty">
        <Handshake size={34} />
        <b>История взаимодействия с организацией</b>
        <span className="muted">Выберите организацию слева — письма, вложения, встречи, договорённости, а с Nextcloud ещё файлы, задачи и чат соберутся на одной временной шкале. Организация определяется по домену адреса.</span>
      </div>}
      {domain && <>
        <header className="counterparty-head">
          <div className="counterparty-name">
            {editingName === null
              ? <h2>{orgName || domain}<button className="icon-button" title="Название организации" aria-label="Изменить название" onClick={() => setEditingName(orgName || '')}><Pencil size={14} /></button></h2>
              : <form className="counterparty-name-form" onSubmit={(e) => { e.preventDefault(); if (data) void saveDossier({ ...data.dossier, name: editingName.trim() || null }); setEditingName(null) }}>
                <input autoFocus value={editingName} maxLength={200} onChange={(e) => setEditingName(e.target.value)} placeholder="Название организации" aria-label="Название организации" />
                <button className="primary-button" type="submit"><Check size={14} /></button>
                <button className="secondary-button" type="button" onClick={() => setEditingName(null)}><X size={14} /></button>
              </form>}
            <small className="muted">@{domain}{data?.own ? ' · своя организация' : ''}</small>
          </div>
          <div className="counterparty-actions">
            <label className="counterparty-years" title="За какой срок показывать встречи">
              Встречи за
              <select value={years} onChange={(e) => { const y = Number(e.target.value); setYears(y); if (domain) void load(domain, data?.server ?? false, y) }}>
                <option value={1}>год</option>
                <option value={3}>3 года</option>
                <option value={5}>5 лет</option>
              </select>
            </label>
            <button className="secondary-button" disabled={loading} onClick={() => void load(domain, true)} title="Найти и старые письма на сервере, которых нет на компьютере">
              {loading ? <Loader2 className="spin" size={14} /> : <CloudDownload size={14} />} {data?.server ? 'Обновить с сервера' : 'Искать и на сервере'}
            </button>
          </div>
        </header>

        {data && <div className="counterparty-stats">
          <span><Mail size={14} /> {data.messages.length} {plural(data.messages.length, 'письмо', 'письма', 'писем')}</span>
          <span><Inbox size={14} /> {data.messages.filter((m) => inDomain(m.fromAddress, data.domain)).length} от них</span>
          <span><Send size={14} /> {data.messages.filter((m) => !inDomain(m.fromAddress, data.domain)).length} им</span>
          <span><CalendarDays size={14} /> {data.events.length} {plural(data.events.length, 'встреча', 'встречи', 'встреч')}</span>
          <span><Paperclip size={14} /> {counts.document} {plural(counts.document, 'вложение', 'вложения', 'вложений')}</span>
          {ncConnected && <span><Cloud size={14} /> {ncBusy ? <Loader2 className="spin" size={12} /> : counts.file} {plural(counts.file, 'файл', 'файла', 'файлов')}</span>}
          {ncConnected && <span><SquareKanban size={14} /> {ncBusy ? <Loader2 className="spin" size={12} /> : counts.task} {plural(counts.task, 'задача', 'задачи', 'задач')}</span>}
          <span><Handshake size={14} /> {openAgreements.length} открытых</span>
          {firstDate && firstDate.getTime() > 0 && <span className="muted">с {fmtDay(firstDate)}</span>}
          {lastMail && <span className="muted">последнее письмо {fmtDay(lastMail)}</span>}
        </div>}
        {error && <div className="inline-warning">{error}</div>}
        {data && data.errors.map((e) => <div key={e} className="inline-warning">{e}</div>)}
        {nc && nc.errors.map((e) => <div key={e} className="inline-warning">{e}</div>)}
        {data && !data.server && <div className="counterparty-hint muted">Письма — из хранящихся на компьютере. Более старые найдутся кнопкой «Искать и на сервере».</div>}

        <div className="counterparty-body">
          <div className="counterparty-timeline">
            <div className="filter-row">
              {KINDS.map((k) => <button key={k.id} className={`filter-chip ${kind === k.id ? 'active' : ''}`} data-kind={k.id} onClick={() => setKind(k.id)}>{k.label}{counts[k.id] > 0 && <span className="muted-count"> {counts[k.id]}</span>}</button>)}
            </div>
            {loading && !data && <div className="empty-state"><Loader2 className="spin" size={18} /> Собираю историю…</div>}
            {data && shown.length === 0 && <div className="empty-state muted">Здесь пока пусто.</div>}
            <ol className="timeline">
              {shown.map((i) => {
                const month = i.date.getTime() > 0 ? monthKey(i.date) : 'Без даты'
                const head = month !== lastMonth ? (lastMonth = month) : null
                return <li key={i.key} className={`timeline-item timeline-${i.kind}`} data-kind={i.kind}>
                  {head && <div className="timeline-month">{head}</div>}
                  <div className="timeline-entry">
                    <span className="timeline-icon">{icon(i)}</span>
                    {i.kind === 'mail' && <button className="timeline-card" onClick={() => openMessage(i.m)} title="Открыть письмо">
                      <span className="timeline-line"><b>{i.m.subject || '(без темы)'}</b><time>{fmtDay(i.date)} {fmtTime(i.date)}</time></span>
                      <small className="muted">{i.incoming ? `От: ${i.m.fromName || i.m.fromAddress}` : `Кому: ${i.m.to.filter((a) => inDomain(a, data!.domain)).join(', ') || i.m.to.join(', ')}`}{i.m.hasAttachments ? ' · есть вложения' : ''}</small>
                    </button>}
                    {i.kind === 'mail' && <button className="icon-button timeline-extra" title="Договорённость из этого письма" aria-label="Договорённость из письма" onClick={() => setForm(agreementFrom(i.m))}><Handshake size={14} /></button>}
                    {i.kind === 'mail' && onTask && <button className="icon-button timeline-extra" title="Задача из этого письма" aria-label="Задача из письма" onClick={() => onTask(i.m)}><SquareKanban size={14} /></button>}
                    {i.kind === 'file' && <button className="timeline-card" onClick={() => onOpen?.({ kind: 'file', f: i.f })} title="Показать в «Файлах»">
                      <span className="timeline-line"><b>{i.f.name}</b><time>{i.date.getTime() > 0 ? fmtDay(i.date) : ''}</time></span>
                      <small className="muted">Nextcloud · {i.f.path.replace(/\/[^/]*$/, '') || '/'}</small>
                    </button>}
                    {i.kind === 'task' && <button className={`timeline-card ${i.h.card.done ? 'done' : ''}`} onClick={() => onOpen?.({ kind: 'card', h: i.h })} title="Открыть задачу в Deck">
                      <span className="timeline-line"><b>{i.h.card.title}</b><time>{fmtDay(i.date)}</time></span>
                      <small className="muted">{i.h.board} › {i.h.stack}{i.h.card.done ? ' · выполнена' : i.h.card.duedate ? ` · срок ${fmtDay(new Date(i.h.card.duedate))}` : ''}</small>
                    </button>}
                    {i.kind === 'chat' && <button className="timeline-card" onClick={() => onOpen?.({ kind: 'chat', h: i.h })} title="Открыть сообщение в чате">
                      <span className="timeline-line"><b>{i.h.author}: {i.h.text.length > 140 ? `${i.h.text.slice(0, 140)}…` : i.h.text}</b><time>{fmtDay(i.date)} {fmtTime(i.date)}</time></span>
                      <small className="muted">чат «{i.h.room}»</small>
                    </button>}
                    {i.kind === 'document' && <button className="timeline-card" onClick={() => openMessage(i.m)} title="Открыть письмо с документом">
                      <span className="timeline-line"><b>{i.name}</b><time>{fmtDay(i.date)}</time></span>
                      <small className="muted">{i.incoming ? 'получен' : 'отправлен'} в письме «{i.m.subject || '(без темы)'}»</small>
                    </button>}
                    {i.kind === 'event' && <div className="timeline-card" role="button" tabIndex={0} onClick={() => setOpenEvent(openEvent === i.e.id ? null : i.e.id)} onKeyDown={(ev) => { if (ev.key === 'Enter') setOpenEvent(openEvent === i.e.id ? null : i.e.id) }}>
                      <span className="timeline-line"><b>{i.e.title}</b><time>{fmtDay(i.date)}{i.e.allDay ? '' : ` ${fmtTime(i.date)}`}</time></span>
                      <small className="muted">{i.date > new Date() ? 'предстоит' : 'встреча'}{i.e.location ? ` · ${i.e.location}` : ''} · {i.e.calendar}</small>
                      {openEvent === i.e.id && <div className="timeline-details">
                        {i.e.organizer && <div>Организатор: {i.e.organizer}</div>}
                        {(i.e.participants ?? []).length > 0 && <div>Участники: {(i.e.participants ?? []).join(', ')}</div>}
                        {i.e.description && <div className="timeline-description">{i.e.description}</div>}
                        {onOpen && <button className="text-button" onClick={(ev) => { ev.stopPropagation(); onOpen({ kind: 'event', e: i.e }) }}>Открыть в календаре</button>}
                      </div>}
                    </div>}
                    {i.kind === 'agreement' && <div className={`timeline-card agreement-card ${i.a.done ? 'done' : ''} ${overdue(i.a) ? 'overdue' : ''}`}>
                      <span className="timeline-line"><b>{i.a.text}</b><time>{fmtDay(i.date)}</time></span>
                      <small className="muted">
                        {i.a.done ? 'выполнено' : i.a.due ? `срок ${fmtDay(dayDate(i.a.due))}${overdue(i.a) ? ' — просрочено' : ''}` : 'без срока'}
                        {i.a.subject && <> · <a href="#" onClick={(e) => { e.preventDefault(); openMessage(i.a) }}>письмо «{i.a.subject}»</a></>}
                      </small>
                    </div>}
                  </div>
                </li>
              })}
            </ol>
          </div>

          <Splitter id="counterparty-side" side="next" min={240} max={640} label="Ширина боковой панели" />
          <aside className="counterparty-side">
            <section className="counterparty-panel">
              <div className="panel-head"><Handshake size={15} /><b>Договорённости</b><button className="icon-button" title="Добавить договорённость" aria-label="Добавить договорённость" disabled={!data} onClick={() => setForm(agreementFrom())}><Plus size={15} /></button></div>
              {form && <form className="agreement-form" onSubmit={(e) => { e.preventDefault(); submitAgreement() }}>
                <textarea autoFocus value={form.text} maxLength={4000} onChange={(e) => setForm({ ...form, text: e.target.value })} placeholder="О чём договорились" aria-label="Текст договорённости" rows={3} />
                <div className="agreement-date"><span>Дата</span><DateField label="Дата договорённости" value={form.date} clearable={false} onChange={(v) => setForm({ ...form, date: v || today() })} /></div>
                <div className="agreement-date"><span>Срок</span><DateField label="Срок выполнения" value={form.due ?? ''} onChange={(v) => setForm({ ...form, due: v || undefined })} /></div>
                {form.subject && <small className="muted">Из письма «{form.subject}»</small>}
                <div className="agreement-form-buttons">
                  <button className="primary-button" type="submit">Сохранить</button>
                  <button className="secondary-button" type="button" onClick={() => setForm(null)}>Отмена</button>
                </div>
              </form>}
              {data && (data.dossier.agreements ?? []).length === 0 && !form && <div className="muted panel-empty">Нет записей. Зафиксируйте, о чём договорились, — со сроком и ссылкой на письмо.</div>}
              {data && [...(data.dossier.agreements ?? [])].sort((a, b) => Number(a.done) - Number(b.done) || (a.due ?? '9999').localeCompare(b.due ?? '9999')).map((a) => (
                <div key={a.id} className={`agreement-row ${a.done ? 'done' : ''} ${overdue(a) ? 'overdue' : ''}`} data-id={a.id}>
                  <input type="checkbox" checked={a.done} onChange={(e) => setAgreementDone(a, e.target.checked)} aria-label="Выполнено" />
                  <span className="agreement-text">{a.text}<small className="muted">{a.due ? `срок ${fmtDay(dayDate(a.due))}` : fmtDay(dayDate(a.date))}{overdue(a) ? ' — просрочено' : ''}</small></span>
                  <button className="icon-button" title="Изменить" aria-label="Изменить договорённость" onClick={() => setForm({ ...a })}><Pencil size={13} /></button>
                  <button className="icon-button" title="Удалить" aria-label="Удалить договорённость" onClick={() => deleteAgreement(a)}><Trash2 size={13} /></button>
                </div>
              ))}
            </section>

            <section className="counterparty-panel">
              <div className="panel-head"><Users size={15} /><b>Люди</b></div>
              {data && data.people.length === 0 && data.contacts.length === 0 && <div className="muted panel-empty">Нет собеседников.</div>}
              {data && peopleList(data).map((p) => {
                const c = contactOf(p.address)
                return <div key={p.address} className="person-row" data-address={p.address}>
                  <span className="person-text">
                    <b>{c?.fullName || p.name || p.address}</b>
                    <small className="muted">{[c?.title, p.address, ...(c?.phones ?? []).slice(0, 1)].filter(Boolean).join(' · ')}</small>
                    {p.messages > 0 && <small className="muted">{p.messages} {plural(p.messages, 'письмо', 'письма', 'писем')}{p.last ? `, последнее ${fmtDay(new Date(p.last))}` : ''}</small>}
                  </span>
                  <button className="icon-button" title="Написать" aria-label={`Написать ${p.address}`} onClick={() => onCompose(p.name ? `${p.name} <${p.address}>` : p.address)}><SquarePen size={14} /></button>
                </div>
              })}
            </section>

            <section className="counterparty-panel">
              <div className="panel-head"><FileText size={15} /><b>Заметка</b></div>
              <textarea
                className="counterparty-note"
                value={note}
                disabled={!data}
                maxLength={16000}
                rows={5}
                placeholder="Реквизиты, договор, контактное лицо, особенности работы…"
                aria-label="Заметка об организации"
                onChange={(e) => setNote(e.target.value)}
                onBlur={() => { if (data && note !== (data.dossier.note ?? '')) void saveDossier({ ...data.dossier, note }, 'Заметка сохранена.') }}
              />
            </section>
          </aside>
        </div>
      </>}
    </section>
  </div>
}

/** Собеседники по письмам и контакты организации, у которых писем нет. */
function peopleList(t: CounterpartyTimeline) {
  const out = [...t.people]
  for (const c of t.contacts) {
    for (const e of c.emails) {
      const address = e.toLowerCase()
      if (inDomain(address, t.domain) && !out.some((p) => p.address === address)) {
        out.push({ address, name: c.fullName, messages: 0, last: null })
      }
    }
  }
  return out.slice(0, 50)
}

function icon(i: Item) {
  switch (i.kind) {
    case 'mail': return i.incoming ? <Inbox size={14} /> : <Send size={14} />
    case 'document': return <Paperclip size={14} />
    case 'event': return <CalendarDays size={14} />
    case 'agreement': return <Handshake size={14} />
    case 'file': return <Cloud size={14} />
    case 'task': return <SquareKanban size={14} />
    case 'chat': return <MessageCircle size={14} />
  }
}

function plural(n: number, one: string, few: string, many: string) {
  const a = n % 10, b = n % 100
  if (a === 1 && b !== 11) return one
  if (a >= 2 && a <= 4 && (b < 12 || b > 14)) return few
  return many
}
