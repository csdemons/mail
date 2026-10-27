import { useEffect, useMemo, useRef, useState } from 'react'
import { Paperclip, Send, X } from 'lucide-react'

import { api, errorText, formatSize } from './api'
import RichEditor, { type RichEditorHandle } from './RichEditor'
import type { Contact, Draft, PickedFile } from './types'

interface Props {
  from: string
  draft: Draft
  contacts: Contact[]
  onClose: () => void
  onSent: (notice: string) => void
}

const TITLES: Record<Draft['mode'], string> = {
  new: 'Новое сообщение',
  reply: 'Ответить на письмо',
  forward: 'Переслать письмо',
}

/** Последний (редактируемый) адрес в строке «a@b.ru, Иван» → «Иван». */
function currentToken(value: string): string {
  const parts = value.split(/[,;]/)
  return parts[parts.length - 1].trim()
}

function replaceToken(value: string, replacement: string): string {
  const idx = Math.max(value.lastIndexOf(','), value.lastIndexOf(';'))
  const head = idx >= 0 ? `${value.slice(0, idx + 1)} ` : ''
  return `${head}${replacement}, `
}

function RecipientField({ label, value, onChange, contacts, autoFocus }: {
  label: string
  value: string
  onChange: (v: string) => void
  contacts: Contact[]
  autoFocus?: boolean
}) {
  const [focused, setFocused] = useState(false)
  const [remote, setRemote] = useState<{ q: string; list: Contact[] }>({ q: '', list: [] })
  const [searching, setSearching] = useState(false)
  const token = currentToken(value).toLowerCase()

  // Поиск на сервере (корпоративная книга не отдаёт список целиком), с паузой после ввода.
  useEffect(() => {
    if (!focused || token.length < 2 || token.includes('@') && token.includes('>')) return
    let cancelled = false
    const t = setTimeout(() => {
      setSearching(true)
      api.searchContacts(token)
        .then((list) => { if (!cancelled) setRemote({ q: token, list }) })
        .catch(() => { if (!cancelled) setRemote({ q: token, list: [] }) })
        .finally(() => { if (!cancelled) setSearching(false) })
    }, 300)
    return () => { cancelled = true; clearTimeout(t) }
  }, [token, focused])

  const suggestions = useMemo(() => {
    if (token.length < 2) return []
    const out: { label: string; value: string; hint: string }[] = []
    const seen = new Set<string>()
    const pool = [...contacts, ...(remote.q && token.startsWith(remote.q) || remote.q === token ? remote.list : [])]
    for (const c of pool) {
      for (const email of c.emails) {
        const key = email.toLowerCase()
        if (seen.has(key)) continue
        const hay = [c.fullName, email, c.org ?? '', c.title ?? ''].join(' ').toLowerCase()
        if (!hay.includes(token)) continue
        seen.add(key)
        const name = c.fullName && c.fullName !== email ? c.fullName : ''
        out.push({ label: name ? `${name} <${email}>` : email, value: name ? `${name} <${email}>` : email, hint: [c.title, c.org].filter(Boolean).join(' · ') })
        if (out.length >= 10) return out
      }
    }
    return out
  }, [contacts, remote, token])

  return (
    <label className="compose-field recipient-field">
      <span>{label}</span>
      <input
        autoFocus={autoFocus}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onFocus={() => setFocused(true)}
        onBlur={() => setTimeout(() => setFocused(false), 150)}
        placeholder="user@nrcki.ru, Иванов Иван <ivanov@nrcki.ru>"
      />
      {focused && token.length >= 2 && (suggestions.length > 0 || searching) && (
        <div className="suggestions popover">
          {suggestions.map((s) => (
            <button type="button" key={s.label} onMouseDown={(e) => { e.preventDefault(); onChange(replaceToken(value, s.value)) }}>
              <span>{s.label}</span>{s.hint && <small>{s.hint}</small>}
            </button>
          ))}
          {searching && <div className="suggestions-status">Поиск в адресных книгах…</div>}
        </div>
      )}
    </label>
  )
}

export default function Composer({ from, draft, contacts, onClose, onSent }: Props) {
  const [to, setTo] = useState(draft.to)
  const [cc, setCc] = useState(draft.cc)
  const [showCc, setShowCc] = useState(Boolean(draft.cc))
  const [subject, setSubject] = useState(draft.subject)
  const editor = useRef<RichEditorHandle>(null)
  const [files, setFiles] = useState<PickedFile[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  async function attach() {
    try {
      const picked = await api.pickAttachments()
      setFiles((list) => [...list, ...picked])
    } catch (err) {
      setError(errorText(err))
    }
  }

  async function submit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!to.trim()) {
      setError('Укажите получателя')
      return
    }
    if (!subject.trim() && !window.confirm('Отправить письмо без темы?')) return
    setBusy(true)
    setError('')
    try {
      const result = await api.sendMessage({
        to, cc, subject,
        body: editor.current?.text() ?? '',
        html: editor.current?.html() ?? null,
        inReplyTo: draft.inReplyTo,
        references: draft.references,
        attachments: files.map((f) => f.id),
      })
      onSent(result.warning ?? 'Письмо отправлено.')
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  const totalSize = files.reduce((sum, f) => sum + f.size, 0)

  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget && !busy) onClose() }}>
      <form className="compose-modal" onSubmit={submit}>
        <div className="compose-header">
          <div><strong>{TITLES[draft.mode]}</strong><small>От: {from}</small></div>
          <button type="button" className="icon-button" disabled={busy} onClick={onClose} aria-label="Закрыть"><X size={18} /></button>
        </div>
        <RecipientField label="Кому" value={to} onChange={setTo} contacts={contacts} autoFocus={draft.mode !== 'reply'} />
        {showCc
          ? <RecipientField label="Копия" value={cc} onChange={setCc} contacts={contacts} />
          : <button type="button" className="text-button add-cc" onClick={() => setShowCc(true)}>+ Копия</button>}
        <label className="compose-field"><span>Тема</span><input value={subject} onChange={(e) => setSubject(e.target.value)} placeholder="Тема письма" /></label>
        <RichEditor ref={editor} initialText={draft.body} autoFocus={draft.mode === 'reply'} onNotice={setError} />
        {files.length > 0 && (
          <div className="compose-files">
            {files.map((f) => (
              <span key={f.id} className="file-chip">
                <Paperclip size={13} /> {f.name} <small>{formatSize(f.size)}</small>
                <button type="button" aria-label={`Убрать ${f.name}`} onClick={() => setFiles((l) => l.filter((x) => x.id !== f.id))}><X size={12} /></button>
              </span>
            ))}
          </div>
        )}
        {error && <div className="login-error compose-error" role="alert">{error}</div>}
        <div className="compose-footer">
          <button type="button" className="text-button attach-button" disabled={busy} onClick={() => void attach()}>
            <Paperclip size={15} /> Вложить файл{files.length > 0 ? ` (${files.length}, ${formatSize(totalSize)})` : ''}
          </button>
          <button className="primary-button" type="submit" disabled={busy}>
            <Send size={16} /> {busy ? 'Отправка…' : 'Отправить'}
          </button>
        </div>
      </form>
    </div>
  )
}
