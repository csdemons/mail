import { useEffect, useMemo, useState } from 'react'
import { BookUser, Building2, Mail, Phone, RefreshCw, Search, StickyNote } from 'lucide-react'

import { api, errorText } from './api'
import type { Contact, ContactsResult } from './types'

interface Props {
  onWrite: (address: string) => void
  onLoaded: (contacts: Contact[]) => void
}

function initials(name: string) {
  const parts = name.trim().split(/\s+/)
  return ((parts[0]?.[0] ?? '') + (parts[1]?.[0] ?? '')).toUpperCase() || '?'
}

export default function ContactsView({ onWrite, onLoaded }: Props) {
  const [data, setData] = useState<ContactsResult | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [query, setQuery] = useState('')
  const [book, setBook] = useState('')
  const [selectedId, setSelectedId] = useState('')

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
    return (data?.contacts ?? []).filter((c) =>
      (!book || c.book === book) &&
      (!q || [c.fullName, c.org ?? '', c.title ?? '', ...c.emails, ...c.phones].join(' ').toLowerCase().includes(q)))
  }, [data, query, book])

  const selected = data?.contacts.find((c) => c.id === selectedId)

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
          {data && !loading && visible.length === 0 && <div className="empty-state"><BookUser size={30} /><strong>Контакты не найдены</strong></div>}
        </div>
        <div className="list-footer"><span>{visible.length} из {data?.contacts.length ?? 0}</span><span>CardDAV</span></div>
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
            {selected.note && <div className="contact-line"><StickyNote size={16} /><span className="contact-note">{selected.note}</span></div>}
          </div>
        ) : (
          <div className="empty-reader"><BookUser size={42} /><strong>Выберите контакт</strong></div>
        )}
      </section>
    </div>
  )
}
