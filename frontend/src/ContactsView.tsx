import { useEffect, useMemo, useState } from 'react'
import { BookUser, Building2, Loader2, Mail, MapPin, Pencil, Phone, Plus, RefreshCw, Search, StickyNote, Trash2 } from 'lucide-react'

import { api, errorText } from './api'
import type { Contact, ContactsResult } from './types'
import ContactEditor from './ContactEditor'

interface Props {
  onWrite: (address: string) => void
  onLoaded: (contacts: Contact[]) => void
  onNotice: (text: string) => void
}

function initials(name: string) {
  const parts = name.trim().split(/\s+/)
  return ((parts[0]?.[0] ?? '') + (parts[1]?.[0] ?? '')).toUpperCase() || '?'
}

export default function ContactsView({ onWrite, onLoaded, onNotice }: Props) {
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
    }, 350)
    return () => { cancelled = true; clearTimeout(t) }
  }, [query])

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
  useEffect(() => {
    if (!selectedBook || selectedBook.writable || browsed[selectedBook.url]) return
    const url = selectedBook.url
    setBrowsed((b) => ({ ...b, [url]: 'loading' }))
    api.browseContacts(url)
      .then((list) => setBrowsed((b) => ({ ...b, [url]: list })))
      .catch(() => setBrowsed((b) => ({ ...b, [url]: 'error' })))
  }, [selectedBook, browsed])
  const canCreate = Boolean(data?.books.some((b) => b.writable))

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
            <BookUser size={17} /><span>Все контакты</span><b className="folder-count">{data?.contacts.length ?? 0}</b>
          </button>
          {data?.books.map((b) => (
            <button key={b.url} className={`folder-item ${book === b.name ? 'selected' : ''}`} onClick={() => setBook(b.name)} title={b.url}>
              <BookUser size={17} /><span>{b.name}</span>
            </button>
          ))}
        </div>
      </aside>

      <main className="message-column">
        <div className="list-toolbar">
          <label className="search-box"><Search size={17} /><input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Имя, e-mail, телефон, отдел" /></label>
          <button className="icon-button soft" aria-label="Обновить" disabled={loading} onClick={() => void load()}><RefreshCw size={17} className={loading ? 'spin' : ''} /></button>
          <button className="icon-button soft" aria-label="Новый контакт" title={canCreate ? 'Новый контакт' : 'Нет адресной книги с правом записи'} disabled={!canCreate} onClick={() => setEditing('new')}><Plus size={18} /></button>
        </div>
        <div className="message-list">
          {loading && !data && <div className="empty-state"><span>Загрузка контактов…</span></div>}
          {error && (
            <div className="empty-state" role="alert">
              <strong>Не удалось загрузить контакты</strong>
              <span>{error}</span>
              <span>Адрес сервера можно указать в Настройках, подробности — в Диагностике.</span>
            </div>
          )}
          {data?.errors.map((e) => <div key={e} className="inline-warning">{e}</div>)}
          {visible.map((c) => (
            <button key={c.id} className={`message-row contact-row ${selectedId === c.id ? 'selected' : ''}`} onClick={() => setSelectedId(c.id)}>
              <div className="sender-avatar">{initials(c.fullName)}</div>
              <div className="message-summary">
                <div className="message-topline"><span className="sender-name">{c.fullName}</span></div>
                <div className="message-subject">{c.emails[0] ?? c.phones[0] ?? ''}</div>
                {(c.org || c.title) && <div className="message-preview">{[c.title, c.org].filter(Boolean).join(' · ')}</div>}
              </div>
            </button>
          ))}
          {searching && <div className="folder-placeholder">Поиск в корпоративной книге…</div>}
          {browseState === 'loading' && <div className="folder-placeholder">Загрузка «{selectedBook?.name}»…</div>}
          {data && !loading && !searching && browseState !== 'loading' && visible.length === 0 && (selectedBook && !selectedBook.writable && query.trim().length < 2
            ? <div className="empty-state"><Search size={30} /><strong>{selectedBook.name}</strong><span>Сервер не отдаёт эту книгу списком — сотрудники находятся по поиску: введите фамилию, имя, e-mail или отдел.</span></div>
            : <div className="empty-state"><BookUser size={30} /><strong>Контакты не найдены</strong>{query.trim().length < 2 && <span>Корпоративная книга ищется по имени или e-mail — начните вводить.</span>}</div>)}
        </div>
        <div className="list-footer"><span>{visible.length}{selectedBook && Array.isArray(browseState) ? '' : ` из ${data?.contacts.length ?? 0}`}</span><span>{selectedBook && !selectedBook.writable ? 'SOGo' : 'CardDAV'}</span></div>
      </main>

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
