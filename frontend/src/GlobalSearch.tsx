import { useEffect, useMemo, useRef, useState } from 'react'
import { Building2, CalendarDays, ContactRound, FileText, Folder, Loader2, Mail, MessageCircle, Search, SquareKanban, X } from 'lucide-react'

import { api, errorText, TYPING_PAUSE_MS } from './api'
import type { CalEvent, ChatHit, Contact, DeckHit, MessageSummary, NcFile } from './types'

/**
 * Общий поиск (Ctrl+K): письма, файлы Nextcloud, контакты, встречи, задачи Deck и чат —
 * одним полем. По домену («@niiar.ru», «niiar.ru») находит всё, что связано с организацией.
 * Щелчок открывает найденное в своём разделе.
 */
export type Found =
  | { kind: 'mail'; m: MessageSummary }
  | { kind: 'file'; f: NcFile }
  | { kind: 'contact'; c: Contact }
  | { kind: 'event'; e: CalEvent }
  | { kind: 'card'; h: DeckHit }
  | { kind: 'chat'; h: ChatHit }
  | { kind: 'org'; domain: string }

type Section = 'mail' | 'file' | 'contact' | 'event' | 'card' | 'chat'
const SECTIONS: { id: Section; label: string; icon: typeof Mail }[] = [
  { id: 'mail', label: 'Письма', icon: Mail },
  { id: 'file', label: 'Файлы', icon: FileText },
  { id: 'contact', label: 'Контакты', icon: ContactRound },
  { id: 'event', label: 'Встречи', icon: CalendarDays },
  { id: 'card', label: 'Задачи', icon: SquareKanban },
  { id: 'chat', label: 'Чат', icon: MessageCircle },
]
const PERIODS: { id: number; label: string }[] = [
  { id: 0, label: 'Любое время' },
  { id: 7, label: 'За неделю' },
  { id: 31, label: 'За месяц' },
  { id: 92, label: 'За 3 месяца' },
  { id: 365, label: 'За год' },
]
const PREVIEW = 5

const fold = (s: string) => s.toLowerCase().replace(/ё/g, 'е')

/** «@niiar.ru», «niiar.ru» → домен организации. */
export function asDomain(term: string): string | null {
  const t = term.trim().toLowerCase().replace(/^@/, '')
  return /^[\p{L}\d-]+(\.[\p{L}\d-]+)+$/u.test(t) ? t : null
}
function inDomain(addr: string, dom: string): boolean {
  const m = /<([^>]+)>/.exec(addr)
  const d = (m ? m[1] : addr).trim().toLowerCase().split('@')[1] ?? ''
  return d === dom || d.endsWith(`.${dom}`)
}

/** Дата найденного (для фильтра по времени и подписи); у контактов даты нет. */
export function foundDate(f: Found): Date | null {
  switch (f.kind) {
    case 'mail': return f.m.date ? new Date(f.m.date) : null
    case 'file': { const d = new Date(f.f.modified); return Number.isNaN(d.getTime()) ? null : d }
    case 'event': return f.e.allDay ? new Date(`${f.e.start}T12:00:00`) : new Date(f.e.start)
    case 'card': { const t = Math.max(f.h.card.lastModified, f.h.card.createdAt); return t > 0 ? new Date(t * 1000) : null }
    case 'chat': return f.h.timestamp > 0 ? new Date(f.h.timestamp * 1000) : null
    default: return null
  }
}

function eventMatches(e: CalEvent, term: string, dom: string | null): boolean {
  if (dom && [e.organizer ?? '', ...(e.participants ?? [])].some((a) => inDomain(a, dom))) return true
  const t = fold(term.trim().replace(/^@/, ''))
  return fold([e.title, e.location ?? '', e.description ?? '', e.organizer ?? '', ...(e.participants ?? [])].join(' ')).includes(t)
}

/** Встречи за год назад и полгода вперёд — один запрос на открытие поиска. */
let eventsCache: { at: number; list: Promise<CalEvent[]> } | null = null
function recentEvents(): Promise<CalEvent[]> {
  if (!eventsCache || Date.now() - eventsCache.at > 5 * 60_000) {
    const now = Date.now()
    const list = api.listEvents(new Date(now - 365 * 86400000), new Date(now + 183 * 86400000)).then((r) => {
      const seen = new Set<string>()
      return r.events.filter((e) => !seen.has(e.id) && !!seen.add(e.id))
    })
    list.catch(() => { eventsCache = null })
    eventsCache = { at: now, list }
  }
  return eventsCache.list
}

const fmtDate = (d: Date | null) => {
  if (!d) return ''
  const now = new Date()
  return d.toLocaleDateString('ru-RU', { day: 'numeric', month: 'short', ...(d.getFullYear() !== now.getFullYear() ? { year: 'numeric' } : {}) })
}

interface Results { mail: Found[]; file: Found[]; contact: Found[]; event: Found[]; card: Found[]; chat: Found[] }
const EMPTY: Results = { mail: [], file: [], contact: [], event: [], card: [], chat: [] }

export default function GlobalSearch({ ncConnected, onOpen, onClose }: { ncConnected: boolean; onOpen: (f: Found) => void; onClose: () => void }) {
  const [q, setQ] = useState('')
  const [section, setSection] = useState<Section | 'all'>('all')
  const [period, setPeriod] = useState(0)
  const [res, setRes] = useState<Results>(EMPTY)
  const [busy, setBusy] = useState<Set<Section>>(new Set())
  const [errors, setErrors] = useState<string[]>([])
  const [expanded, setExpanded] = useState<Set<Section>>(new Set())
  const [cursor, setCursor] = useState(0)
  const request = useRef(0)
  const listRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const term = q.trim()
    setExpanded(new Set())
    setCursor(0)
    if (term.replace(/^@/, '').length < 2) { setRes(EMPTY); setBusy(new Set()); setErrors([]); return }
    const id = ++request.current
    const dom = asDomain(term)
    const timer = window.setTimeout(() => {
      setErrors([])
      const pending = new Set<Section>(['mail', 'contact', 'event', ...(ncConnected ? ['file', 'card', 'chat'] as Section[] : [])])
      setBusy(new Set(pending))
      setRes(EMPTY)
      const done = (s: Section[], patch: Partial<Results>, error?: string) => {
        if (request.current !== id) return
        setRes((r) => ({ ...r, ...patch }))
        if (error) setErrors((e) => [...e, error])
        setBusy((b) => { const n = new Set(b); s.forEach((x) => n.delete(x)); return n })
      }
      api.searchMail(dom ? `domain:${dom}` : term)
        .then((list) => done(['mail'], { mail: list.map((m) => ({ kind: 'mail', m })) }))
        .catch((e) => done(['mail'], {}, `Письма: ${errorText(e)}`))
      api.searchContacts(dom ? `@${dom}` : term)
        .then((list) => done(['contact'], { contact: (dom ? list.filter((c) => c.emails.some((a) => inDomain(a, dom)) || fold(c.org ?? '').includes(dom.split('.')[0])) : list).map((c) => ({ kind: 'contact', c })) }))
        .catch((e) => done(['contact'], {}, `Контакты: ${errorText(e)}`))
      recentEvents()
        .then((list) => done(['event'], { event: list.filter((e) => eventMatches(e, term, dom)).sort((a, b) => b.start.localeCompare(a.start)).map((e) => ({ kind: 'event', e })) }))
        .catch((e) => done(['event'], {}, `Календарь: ${errorText(e)}`))
      if (ncConnected) {
        api.ncSearch(term, dom, 50)
          .then((r) => done(['file', 'card', 'chat'], {
            file: r.files.map((f) => ({ kind: 'file', f })),
            card: r.cards.map((h) => ({ kind: 'card', h })),
            chat: r.chat.map((h) => ({ kind: 'chat', h })),
          }, r.errors.length ? r.errors.join('; ') : undefined))
          .catch((e) => done(['file', 'card', 'chat'], {}, `Nextcloud: ${errorText(e)}`))
      }
    }, TYPING_PAUSE_MS)
    return () => window.clearTimeout(timer)
  }, [q, ncConnected])

  const since = period ? Date.now() - period * 86400000 : 0
  const filtered = useMemo(() => {
    const out: Record<Section, Found[]> = { ...EMPTY }
    for (const s of SECTIONS) {
      out[s.id] = res[s.id].filter((f) => {
        if (!since) return true
        const d = foundDate(f)
        return d ? d.getTime() >= since : s.id === 'contact'
      })
    }
    return out
  }, [res, since])

  const dom = asDomain(q)
  const visibleSections = SECTIONS.filter((s) => section === 'all' || section === s.id)
  /** Строки по порядку — для стрелок и Enter. */
  const rows: Found[] = useMemo(() => {
    const out: Found[] = dom ? [{ kind: 'org', domain: dom }] : []
    for (const s of visibleSections) {
      const all = filtered[s.id]
      out.push(...(section === s.id || expanded.has(s.id) ? all : all.slice(0, PREVIEW)))
    }
    return out
  }, [filtered, visibleSections, section, expanded, dom])

  useEffect(() => {
    listRef.current?.querySelector(`[data-row="${cursor}"]`)?.scrollIntoView({ block: 'nearest' })
  }, [cursor])

  function open(f: Found) {
    onOpen(f)
    onClose()
  }
  function onKey(e: React.KeyboardEvent) {
    if (e.key === 'Escape') { e.preventDefault(); onClose() }
    else if (e.key === 'ArrowDown') { e.preventDefault(); setCursor((c) => Math.min(rows.length - 1, c + 1)) }
    else if (e.key === 'ArrowUp') { e.preventDefault(); setCursor((c) => Math.max(0, c - 1)) }
    else if (e.key === 'Enter' && rows[cursor]) { e.preventDefault(); open(rows[cursor]) }
  }

  const total = SECTIONS.reduce((n, s) => n + filtered[s.id].length, 0)
  const searching = busy.size > 0
  let index = dom ? 1 : 0

  return (
    <div className="modal-backdrop global-search-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}>
      <div className="global-search" role="dialog" aria-label="Общий поиск" onKeyDown={onKey}>
        <label className="global-search-input">
          <Search size={18} />
          <input autoFocus value={q} onChange={(e) => setQ(e.target.value)} aria-label="Искать везде"
            placeholder="Искать везде: письма, файлы, контакты, встречи, задачи, чат — или @домен организации" />
          {searching && <Loader2 className="spin" size={16} />}
          <button className="icon-button" onClick={onClose} aria-label="Закрыть поиск"><X size={16} /></button>
        </label>
        <div className="global-search-filters">
          <div className="filter-row" role="tablist" aria-label="Раздел">
            <button role="tab" aria-selected={section === 'all'} className={`filter-chip ${section === 'all' ? 'active' : ''}`} onClick={() => setSection('all')}>Всё{total > 0 && <span className="muted-count"> {total}</span>}</button>
            {SECTIONS.map((s) => (
              <button key={s.id} role="tab" data-section={s.id} aria-selected={section === s.id} className={`filter-chip ${section === s.id ? 'active' : ''}`} onClick={() => setSection(s.id)}>
                {s.label}{filtered[s.id].length > 0 && <span className="muted-count"> {filtered[s.id].length}</span>}
              </button>
            ))}
          </div>
          <select value={period} onChange={(e) => setPeriod(Number(e.target.value))} aria-label="Период">
            {PERIODS.map((p) => <option key={p.id} value={p.id}>{p.label}</option>)}
          </select>
        </div>
        <div className="global-search-results" ref={listRef} role="listbox" aria-label="Результаты поиска">
          {q.trim().replace(/^@/, '').length < 2 && <div className="global-search-hint muted">
            Введите хотя бы два символа. Домен организации (<b>@niiar.ru</b>) найдёт переписку, людей, встречи, документы, задачи и чат с ней.
            <div className="global-search-keys"><kbd>↑</kbd><kbd>↓</kbd> выбрать · <kbd>Enter</kbd> открыть · <kbd>Esc</kbd> закрыть</div>
          </div>}
          {dom && (
            <button className={`global-search-row org ${cursor === 0 ? 'active' : ''}`} data-row={0} role="option" aria-selected={cursor === 0} onMouseEnter={() => setCursor(0)} onClick={() => open({ kind: 'org', domain: dom })}>
              <Building2 size={16} /><span className="gs-main"><b>История организации {dom}</b><small className="muted">письма, встречи, документы, задачи и чат на одной шкале</small></span>
            </button>
          )}
          {visibleSections.map((s) => {
            const all = filtered[s.id]
            const isBusy = busy.has(s.id)
            if (all.length === 0 && !isBusy) return null
            const full = section === s.id || expanded.has(s.id)
            const list = full ? all : all.slice(0, PREVIEW)
            const Icon = s.icon
            return (
              <section key={s.id} className="global-search-section" data-section={s.id}>
                <div className="global-search-head"><Icon size={14} /><b>{s.label}</b>{all.length > 0 && <span className="muted">{all.length}</span>}{isBusy && <Loader2 className="spin" size={12} />}</div>
                {isBusy && all.length === 0 && <div className="skeleton-row compact" aria-hidden="true"><span className="skeleton-square" /><span className="skeleton-lines"><span className="skeleton-bar" style={{ width: '55%' }} /><span className="skeleton-bar thin" style={{ width: '35%' }} /></span></div>}
                {list.map((f) => {
                  const i = index++
                  return <Row key={keyOf(f)} f={f} i={i} active={cursor === i} onHover={() => setCursor(i)} onOpen={() => open(f)} />
                })}
                {!full && all.length > PREVIEW && (
                  <button className="text-button global-search-more" onClick={() => setExpanded((x) => new Set(x).add(s.id))}>Показать все ({all.length})</button>
                )}
              </section>
            )
          })}
          {q.trim().replace(/^@/, '').length >= 2 && !searching && total === 0 && <div className="global-search-hint muted">Ничего не найдено{period ? ' за выбранный период' : ''}.</div>}
          {!ncConnected && q.trim().length >= 2 && <div className="global-search-hint muted">Файлы, задачи и чат ищутся, когда подключён Nextcloud (Настройки).</div>}
          {errors.map((e) => <div key={e} className="inline-warning">{e}</div>)}
        </div>
      </div>
    </div>
  )
}

function keyOf(f: Found): string {
  switch (f.kind) {
    case 'mail': return `m:${f.m.folder}:${f.m.uid}`
    case 'file': return `f:${f.f.path}`
    case 'contact': return `c:${f.c.href || f.c.id}`
    case 'event': return `e:${f.e.id}`
    case 'card': return `k:${f.h.card.id}`
    case 'chat': return `t:${f.h.token}:${f.h.messageId ?? 'room'}`
    case 'org': return `o:${f.domain}`
  }
}

function Row({ f, i, active, onHover, onOpen }: { f: Found; i: number; active: boolean; onHover: () => void; onOpen: () => void }) {
  const date = fmtDate(foundDate(f))
  let icon = <FileText size={16} />
  let title = ''
  let sub = ''
  switch (f.kind) {
    case 'mail':
      icon = <Mail size={16} />
      title = f.m.subject || '(без темы)'
      sub = f.m.fromName || f.m.fromAddress
      break
    case 'file':
      icon = f.f.isDir ? <Folder size={16} /> : <FileText size={16} />
      title = f.f.name
      sub = f.f.path.replace(/\/[^/]*$/, '') || '/'
      break
    case 'contact':
      icon = <ContactRound size={16} />
      title = f.c.fullName || f.c.emails[0] || '(без имени)'
      sub = [f.c.org, f.c.title, f.c.emails[0]].filter(Boolean).join(' · ')
      break
    case 'event':
      icon = <CalendarDays size={16} />
      title = f.e.title
      sub = [f.e.allDay ? '' : new Date(f.e.start).toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' }), f.e.location, f.e.calendar].filter(Boolean).join(' · ')
      break
    case 'card':
      icon = <SquareKanban size={16} />
      title = f.h.card.title
      sub = `${f.h.board} › ${f.h.stack}${f.h.card.done ? ' · выполнена' : f.h.card.duedate ? ` · срок ${new Date(f.h.card.duedate).toLocaleDateString('ru-RU')}` : ''}`
      break
    case 'chat':
      icon = <MessageCircle size={16} />
      title = f.h.messageId === null ? f.h.room : `${f.h.author}: ${f.h.text}`
      sub = f.h.messageId === null ? 'беседа' : f.h.room
      break
    case 'org':
      break
  }
  return (
    <button className={`global-search-row ${active ? 'active' : ''}`} data-row={i} data-kind={f.kind} role="option" aria-selected={active} onMouseEnter={onHover} onClick={onOpen}>
      {icon}
      <span className="gs-main"><b>{title}</b>{sub && <small className="muted">{sub}</small>}</span>
      {date && <time className="muted">{date}</time>}
    </button>
  )
}
