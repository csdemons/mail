import { useEffect, useMemo, useState } from 'react'
import { BookUser, Building2, Copy, Loader2, Mail, MapPin, Pencil, Phone, Plus, RefreshCw, Search, StickyNote, Trash2 } from 'lucide-react'

import { api, errorText, TYPING_PAUSE_MS } from './api'
import type { Contact, ContactsResult } from './types'
import ContactEditor from './ContactEditor'
import { SkeletonRows } from './Skeleton'
import Splitter from './Splitter'
import { openContextMenu, type MenuItem } from './PopupMenu'

interface Props {
  onWrite: (address: string) => void
  onLoaded: (contacts: Contact[]) => void
  onNotice: (text: string) => void
  /** Переход из общего поиска: показать этот контакт. */
  focus?: { contact: Contact; n: number } | null
}

function initials(name: string) {
  const parts = name.trim().split(/\s+/)
  return ((parts[0]?.[0] ?? '') + (parts[1]?.[0] ?? '')).toUpperCase() || '?'
}

const CONTACTS_PAGE = 200


/** Последний выполненный переход из общего поиска. */
let contactFocusSeen = 0

export default function ContactsView({ onWrite, onLoaded, onNotice, focus }: Props) {
  const [data, setData] = useState<ContactsResult | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [query, setQuery] = useState('')
  const [book, setBook] = useState('')
  const [selectedId, setSelectedId] = useState('')
  const [editing, setEditing] = useState<Contact | 'new' | null>(null)
  // Результаты поиска на сервере: корпоративная книга (LDAP) целиком не загружается.
  const [remote, setRemote] = useState<Contact[]>([])
  const [searching, setSearching] = useState(false)
  // Корпоративная книга списком (загружается при её выборе).
  const [browsed, setBrowsed] = useState<Record<string, Contact[] | 'loading' | 'error'>>({})
  // Подробности корпоративных контактов (поиск отдаёт только имя и почту).
  const [details, setDetails] = useState<Record<string, Contact | 'loading' | 'none'>>({})

  useEffect(() => {
    const q = query.trim()
    if (q.length < 2) { setRemote([]); return }
    let cancelled = false
    const t = setTimeout(() => {
      setSearching(true)
      api.searchContacts(q)
        .then((list) => { if (!cancelled) setRemote(list) })
        .catch(() => { if (!cancelled) setRemote([]) })
        .finally(() => { if (!cancelled) setSearching(false) })
    }, TYPING_PAUSE_MS)
    return () => { cancelled = true; clearTimeout(t) }
  }, [query])

  useEffect(() => {
    // Каждый переход — один раз: при повторном открытии раздела он не повторяется.
    if (!focus || focus.n <= contactFocusSeen) return
    contactFocusSeen = focus.n
    const c = focus.contact
    setBook('')
    setRemote((r) => (r.some((x) => x.href === c.href) ? r : [c, ...r]))
    setQuery(c.emails[0] ?? c.fullName)
    setSelectedId(c.id)
  }, [focus])

  const all = useMemo(() => {
    const local = data?.contacts ?? []
    const known = new Set(local.map((c) => c.href))
    const extraLists = Object.values(browsed).flatMap((v) => (Array.isArray(v) ? v : []))
    const out = [...local]
    for (const c of [...remote, ...extraLists]) {
      if (!known.has(c.href)) { known.add(c.href); out.push(c) }
    }
    return out
  }, [data, remote, browsed])

  async function load() {
    setLoading(true)
    setError('')
    try {
      const result = await api.listContacts()
      setData(result)
      onLoaded(result.contacts)
      setSelectedId((id) => id || result.contacts[0]?.id || '')
    } catch (err) {
      setError(errorText(err))
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => { void load() }, []) // eslint-disable-line react-hooks/exhaustive-deps

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase()
    const found = new Set(remote.map((c) => c.id))
    return all.filter((c) =>
      (!book || c.book === book) &&
      // Найденное сервером показываем как есть: он ищет и по полям, которых нет в списке.
      (!q || found.has(c.id) || [c.fullName, c.org ?? '', c.title ?? '', ...c.emails, ...c.phones].join(' ').toLowerCase().includes(q)))
  }, [all, remote, query, book])

  // Длинные книги (корпоративная — тысячи сотрудников) показываем страницами.
  const [shown, setShown] = useState(CONTACTS_PAGE)
  useEffect(() => setShown(CONTACTS_PAGE), [book, query])
  const page = visible.slice(0, shown)

  const base = all.find((c) => c.id === selectedId)
  const extra = base ? details[base.id] : undefined
  const selected = base && extra && typeof extra === 'object' ? { ...base, ...extra, id: base.id, book: base.book } : base

  useEffect(() => {
    if (!base || base.editable || details[base.id]) return
    setDetails((d) => ({ ...d, [base.id]: 'loading' }))
    api.contactDetails(base.href, base.id)
      .then((c) => setDetails((d) => ({ ...d, [base.id]: c })))
      .catch(() => setDetails((d) => ({ ...d, [base.id]: 'none' })))
  }, [base, details])
  const selectedBook = data?.books.find((b) => b.name === book)
  const browseState = selectedBook ? browsed[selectedBook.url] : undefined
  // Книги только для чтения (корпоративная SOGo/LDAP) сервер не отдаёт общим списком —
  // загружаем их сразу после списка, чтобы «Все контакты» были полными с первого раза,
  // а не только после захода в саму книгу.
  useEffect(() => {
    const pending = (data?.books ?? []).filter((b) => !b.writable && !browsed[b.url])
    if (pending.length === 0) return
    setBrowsed((m) => ({ ...m, ...Object.fromEntries(pending.map((b) => [b.url, 'loading' as const])) }))
    for (const { url } of pending) {
      api.browseContacts(url)
        .then((list) => setBrowsed((m) => ({ ...m, [url]: list })))
        .catch(() => setBrowsed((m) => ({ ...m, [url]: 'error' })))
    }
  }, [data, browsed])
  const browsingAny = Object.values(browsed).some((v) => v === 'loading')
  const canCreate = Boolean(data?.books.some((b) => b.writable))

  function copy(text: string, what: string) {
    navigator.clipboard.writeText(text).then(() => onNotice(what)).catch((err) => onNotice(`Не удалось скопировать: ${errorText(err)}`))
  }

  /** Меню контакта (правая кнопка мыши). */
  function contactMenu(c: Contact): MenuItem[] {
    const email = c.emails[0]
    const phone = c.phones[0]
    return [
      ...c.emails.slice(0, 3).map((e): MenuItem => ({ label: c.emails.length > 1 ? `Написать: ${e}` : 'Написать письмо', icon: <Mail size={15} />, onClick: () => onWrite(`${c.fullName} <${e}>`) })),
      ...(email || phone ? [{ separator: true } as MenuItem] : []),
      ...(email ? [{ label: 'Копировать адрес', icon: <Copy size={15} />, onClick: () => copy(email, `Скопировано: ${email}`) }] : []),
      ...(phone ? [{ label: 'Копировать телефон', icon: <Copy size={15} />, onClick: () => copy(phone, `Скопировано: ${phone}`) }] : []),
      { label: 'Копировать имя и адрес', icon: <Copy size={15} />, disabled: !email, onClick: () => copy(`${c.fullName} <${email}>`, 'Имя и адрес скопированы') },
      ...(c.editable ? [
        { separator: true } as MenuItem,
        { label: 'Изменить', icon: <Pencil size={15} />, onClick: () => setEditing(c) },
        { label: 'Удалить', icon: <Trash2 size={15} />, danger: true, onClick: () => void remove(c) },
      ] : []),
    ]
  }

  async function remove(c: Contact) {
    if (!window.confirm(`Удалить контакт «${c.fullName}»?`)) return
    try {
      await api.deleteContact(c.href, c.etag)
      setSelectedId('')
      onNotice('Контакт удалён.')
      await load()
    } catch (err) {
      onNotice(`Не удалось удалить: ${errorText(err)}`)
    }
  }

  return (
    <div className="module-view contacts-view">
      <aside className="folder-sidebar">
        <div className="sidebar-heading"><span>Адресные книги</span></div>
        <div className="folder-list">
          <button className={`folder-item ${book === '' ? 'selected' : ''}`} onClick={() => setBook('')}>
            <BookUser size={17} /><span>Все контакты</span><b className="folder-count">{data ? all.length : 0}</b>
          </button>
          {data?.books.map((b) => (
            <button key={b.url} className={`folder-item ${book === b.name ? 'selected' : ''}`} onClick={() => setBook(b.name)} title={b.url}>
              <BookUser size={17} /><span>{b.name}</span>
            </button>
          ))}
        </div>
      </aside>
      <Splitter id="contacts-books" min={150} max={420} label="Ширина списка адресных книг" />

      <main className="message-column">
        <div className="list-toolbar">
          <label className="search-box"><Search size={17} /><input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Имя, e-mail, телефон, отдел" /></label>
          <button className="icon-button soft" aria-label="Обновить" disabled={loading} onClick={() => { setBrowsed({}); void load() }}><RefreshCw size={17} className={loading ? 'spin' : ''} /></button>
          <button className="icon-button soft" aria-label="Новый контакт" title={canCreate ? 'Новый контакт' : 'Нет адресной книги с правом записи'} disabled={!canCreate} onClick={() => setEditing('new')}><Plus size={18} /></button>
        </div>
        <div className="message-list">
          {loading && !data && <SkeletonRows variant="contact" count={9} label="Загрузка контактов…" />}
          {error && (
            <div className="empty-state" role="alert">
              <strong>Не удалось загрузить контакты</strong>
              <span>{error}</span>
              <span>Адрес сервера можно указать в Настройках, подробности — в Диагностике.</span>
            </div>
          )}
          {data?.errors.map((e) => <div key={e} className="inline-warning">{e}</div>)}
          {page.map((c) => (
            <button key={c.id} className={`message-row contact-row ${selectedId === c.id ? 'selected' : ''}`} onClick={() => setSelectedId(c.id)}
              onContextMenu={(e) => { setSelectedId(c.id); openContextMenu(e, contactMenu(c)) }}>
              <div className="sender-avatar">{initials(c.fullName)}</div>
              <div className="message-summary">
                <div className="message-topline"><span className="sender-name">{c.fullName}</span></div>
                <div className="message-subject">{c.emails[0] ?? c.phones[0] ?? ''}</div>
                {(c.org || c.title) && <div className="message-preview">{[c.title, c.org].filter(Boolean).join(' · ')}</div>}
              </div>
            </button>
          ))}
          {visible.length > shown && <button className="text-button load-more" onClick={() => setShown((n) => n + CONTACTS_PAGE)}>Показать ещё (показано {shown} из {visible.length})</button>}
          {searching && <div className="folder-placeholder">Поиск в корпоративной книге…</div>}
          {(browseState === 'loading' || (!selectedBook && browsingAny)) && <SkeletonRows variant="contact" count={visible.length ? 3 : 8} label={`Загрузка ${selectedBook ? `«${selectedBook.name}»` : 'адресных книг'}…`} />}
          {data && !loading && !searching && browseState !== 'loading' && !(!selectedBook && browsingAny) && visible.length === 0 && (selectedBook && !selectedBook.writable && query.trim().length < 2
            ? <div className="empty-state"><Search size={30} /><strong>{selectedBook.name}</strong><span>Сервер не отдаёт эту книгу списком — сотрудники находятся по поиску: введите фамилию, имя, e-mail или отдел.</span></div>
            : <div className="empty-state"><BookUser size={30} /><strong>Контакты не найдены</strong>{query.trim().length < 2 && <span>Корпоративная книга ищется по имени или e-mail — начните вводить.</span>}</div>)}
        </div>
        <div className="list-footer"><span>{visible.length}{selectedBook && Array.isArray(browseState) ? '' : ` из ${all.length}`}</span><span>{(data?.books ?? []).some((b) => b.url.startsWith('ews:')) ? 'Exchange' : selectedBook && !selectedBook.writable ? 'SOGo' : 'CardDAV'}</span></div>
      </main>
      <Splitter id="contacts-list" min={240} max={800} label="Ширина списка контактов" />

      <section className="reader-pane">
        {selected ? (
          <div className="reader-content contact-card">
            <div className="sender-block">
              <div className="sender-avatar sender-avatar-large">{initials(selected.fullName)}</div>
              <div className="sender-details">
                <strong className="contact-name">{selected.fullName}</strong>
                {selected.title && <span>{selected.title}</span>}
                <small>{selected.book}</small>
              </div>
            </div>
            {selected.emails.map((e) => (
              <div key={e} className="contact-line"><Mail size={16} /><span>{e}</span><button className="secondary-button" onClick={() => onWrite(`${selected.fullName} <${e}>`)}>Написать</button></div>
            ))}
            {selected.phones.map((p) => <div key={p} className="contact-line"><Phone size={16} /><span>{p}</span></div>)}
            {selected.org && <div className="contact-line"><Building2 size={16} /><span>{selected.org}</span></div>}
            {selected.address && <div className="contact-line"><MapPin size={16} /><span>{selected.address}</span></div>}
            {extra === 'loading' && <div className="contact-line muted"><Loader2 size={16} className="spin" /><span>Загрузка подробностей…</span></div>}
            {selected.note && <div className="contact-line"><StickyNote size={16} /><span className="contact-note">{selected.note}</span></div>}
            {selected.editable
              ? <div className="reader-actions">
                  <button className="secondary-button" onClick={() => setEditing(selected)}><Pencil size={15} /> Изменить</button>
                  <button className="secondary-button danger" onClick={() => void remove(selected)}><Trash2 size={15} /> Удалить</button>
                </div>
              : <div className="muted editor-note">«{selected.book}» — только для чтения.</div>}
          </div>
        ) : (
          <div className="empty-reader"><BookUser size={42} /><strong>Выберите контакт</strong></div>
        )}
      </section>
      {editing && data && (
        <ContactEditor
          contact={editing === 'new' ? null : editing}
          books={data.books}
          defaultBook={book}
          onClose={() => setEditing(null)}
          onSaved={(text) => { setEditing(null); onNotice(text); void load() }}
        />
      )}
    </div>
  )
}
