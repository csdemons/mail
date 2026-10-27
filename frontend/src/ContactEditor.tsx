import { useState } from 'react'
import { X } from 'lucide-react'

import { api, errorText } from './api'
import type { Contact, ContactInput, DavCollection } from './types'

interface Props {
  contact: Contact | null
  books: DavCollection[]
  defaultBook: string
  onClose: () => void
  onSaved: (notice: string) => void
}

const lines = (v: string) => v.split(/[\n,;]/).map((x) => x.trim()).filter(Boolean)

export default function ContactEditor({ contact, books, defaultBook, onClose, onSaved }: Props) {
  const writable = books.filter((b) => b.writable)
  const [fullName, setFullName] = useState(contact?.fullName ?? '')
  const [emails, setEmails] = useState(contact?.emails.join('\n') ?? '')
  const [phones, setPhones] = useState(contact?.phones.join('\n') ?? '')
  const [org, setOrg] = useState(contact?.org ?? '')
  const [title, setTitle] = useState(contact?.title ?? '')
  const [note, setNote] = useState(contact?.note ?? '')
  const [bookUrl, setBookUrl] = useState(
    writable.find((b) => b.name === defaultBook)?.url ?? writable[0]?.url ?? '',
  )
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  async function save(event: React.FormEvent) {
    event.preventDefault()
    const input: ContactInput = {
      fullName, emails: lines(emails), phones: lines(phones),
      org: org || null, title: title || null, note: note || null,
    }
    if (!fullName.trim()) { setError('Укажите имя'); return }
    const bad = input.emails.find((e) => !/^[^@\s]+@[^@\s]+$/.test(e))
    if (bad) { setError(`Некорректный e-mail: ${bad}`); return }
    setBusy(true)
    setError('')
    try {
      if (contact) {
        await api.updateContact(contact.href, input)
        onSaved('Контакт сохранён.')
      } else {
        await api.createContact(bookUrl, input)
        onSaved('Контакт добавлен.')
      }
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget && !busy) onClose() }}>
      <form className="compose-modal editor-modal" onSubmit={save}>
        <div className="compose-header">
          <div><strong>{contact ? 'Изменить контакт' : 'Новый контакт'}</strong>{contact && <small>{contact.book}</small>}</div>
          <button type="button" className="icon-button" onClick={onClose} aria-label="Закрыть"><X size={18} /></button>
        </div>
        <label className="login-field"><span>Фамилия Имя Отчество</span><input autoFocus value={fullName} onChange={(e) => setFullName(e.target.value)} placeholder="Иванов Иван Иванович" /></label>
        {!contact && (
          <label className="login-field"><span>Адресная книга</span>
            <select value={bookUrl} onChange={(e) => setBookUrl(e.target.value)}>
              {writable.map((b) => <option key={b.url} value={b.url}>{b.name}</option>)}
            </select>
          </label>
        )}
        <div className="editor-row">
          <label className="login-field"><span>E-mail (по одному в строке)</span><textarea rows={2} value={emails} onChange={(e) => setEmails(e.target.value)} placeholder="ivanov@nrcki.ru" /></label>
          <label className="login-field"><span>Телефоны</span><textarea rows={2} value={phones} onChange={(e) => setPhones(e.target.value)} placeholder="+7 495 000-00-00" /></label>
        </div>
        <div className="editor-row">
          <label className="login-field"><span>Организация, отдел</span><input value={org} onChange={(e) => setOrg(e.target.value)} placeholder="НИЦ КИ, УИТ" /></label>
          <label className="login-field"><span>Должность</span><input value={title} onChange={(e) => setTitle(e.target.value)} /></label>
        </div>
        <label className="login-field"><span>Заметка</span><textarea rows={3} value={note} onChange={(e) => setNote(e.target.value)} /></label>
        {error && <div className="login-error" role="alert">{error}</div>}
        <div className="compose-footer">
          <span />
          <button className="primary-button" type="submit" disabled={busy || (!contact && !bookUrl)}>{busy ? 'Сохранение…' : 'Сохранить'}</button>
        </div>
      </form>
    </div>
  )
}
