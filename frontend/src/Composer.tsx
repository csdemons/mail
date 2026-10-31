import { useEffect, useMemo, useRef, useState } from 'react'
import { CalendarClock, ChevronDown, Cloud, FileText, MailCheck, Paperclip, Send, Trash2, X } from 'lucide-react'

import { api, errorText, formatSize } from './api'
import RichEditor, { type RichEditorHandle } from './RichEditor'
import NcFilePicker from './NcFilePicker'
import type { Contact, Draft, MailTemplate, PickedFile } from './types'
import { formatWhen, loadSendPrefs, localInputValue, schedulePresets } from './sending'
import { fileToBase64 } from './mailTools'

interface Props {
  from: string
  draft: Draft
  contacts: Contact[]
  /** Nextcloud подключён — можно вставлять ссылки на файлы вместо вложений. */
  ncConnected?: boolean
  onClose: () => void
  onSent: (notice: string) => void
  /** Письмо ушло в «окно отмены»: родитель показывает отсчёт и может вернуть его в редактор. */
  onQueued?: (id: number, snapshot: Draft, delaySecs: number) => void
}

const TITLES: Record<Draft['mode'], string> = {
  new: 'Новое сообщение',
  reply: 'Ответить на письмо',
  replyAll: 'Ответить всем',
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

export default function Composer({ from, draft, contacts, ncConnected, onClose, onSent, onQueued }: Props) {
  const [ncPicker, setNcPicker] = useState(false)
  const [to, setTo] = useState(draft.to)
  const [cc, setCc] = useState(draft.cc)
  const [showCc, setShowCc] = useState(Boolean(draft.cc))
  const [subject, setSubject] = useState(draft.subject)
  const editor = useRef<RichEditorHandle>(null)
  const [files, setFiles] = useState<PickedFile[]>(draft.files ?? [])
  const [receipt, setReceipt] = useState(draft.readReceipt ?? loadSendPrefs().receiptDefault)
  const [schedOpen, setSchedOpen] = useState(false)
  const [dropping, setDropping] = useState(false)

  /** Файлы, перетащенные в окно письма (из проводника или с рабочего стола), — во вложения. */
  async function dropFiles(list: FileList) {
    setDropping(false)
    for (const file of Array.from(list)) {
      try {
        const picked = await api.attachData(file.name, await fileToBase64(file))
        setFiles((l) => [...l, picked])
      } catch (e) {
        setError(`${file.name}: ${errorText(e)}`)
      }
    }
  }
  const [tplOpen, setTplOpen] = useState(false)
  const [templates, setTemplates] = useState<MailTemplate[] | null>(null)
  const [tplName, setTplName] = useState('')

  function openTemplates() {
    setTplOpen((o) => !o)
    setSchedOpen(false)
    api.templatesList().then(setTemplates).catch((e) => { setTemplates([]); setError(errorText(e)) })
  }

  function applyTemplate(t: MailTemplate) {
    setTplOpen(false)
    if (!subject.trim()) setSubject(t.name)
    const html = t.html ?? t.text.split('\n').map((l) => `<div>${l.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;') || '<br>'}</div>`).join('')
    editor.current?.insertHtml(html)
  }

  async function saveTemplate() {
    const name = tplName.trim() || subject.trim()
    if (!name) { setError('Укажите название шаблона'); return }
    try {
      await api.templateSave(name, editor.current?.text() ?? '', editor.current?.html() ?? null)
      setTplName('')
      setTemplates(await api.templatesList())
      setError('')
    } catch (e) {
      setError(errorText(e))
    }
  }

  async function deleteTemplate(t: MailTemplate) {
    try {
      await api.templateDelete(t.folder, t.uid)
      setTemplates((l) => (l ?? []).filter((x) => x.uid !== t.uid))
    } catch (e) {
      setError(errorText(e))
    }
  }
  const [schedAt, setSchedAt] = useState(() => localInputValue(schedulePresets()[0].at))
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

  async function submit(event: React.FormEvent<HTMLFormElement> | null, sendAt: Date | null = null) {
    event?.preventDefault()
    if (!to.trim()) {
      setError('Укажите получателя')
      return
    }
    if (!subject.trim() && !window.confirm('Отправить письмо без темы?')) return
    setBusy(true)
    setError('')
    try {
      const html = editor.current?.html() ?? null
      const body = editor.current?.text() ?? ''
      const delaySecs = onQueued && !sendAt ? loadSendPrefs().undoSecs : 0
      const ticket = await api.sendMessage({
        to, cc, subject, body, html,
        inReplyTo: draft.inReplyTo,
        references: draft.references,
        attachments: files.map((f) => f.id),
        readReceipt: receipt,
        delaySecs,
        sendAt: sendAt ? sendAt.toISOString() : null,
      })
      if (ticket.id !== null && onQueued) {
        onQueued(ticket.id, { ...draft, to, cc, subject, body, html: html ?? undefined, files, readReceipt: receipt }, delaySecs)
      } else if (ticket.scheduledAt) {
        onSent(`Письмо будет отправлено ${formatWhen(ticket.scheduledAt)}. Оно ждёт в «Черновиках».`)
      } else {
        onSent(ticket.result?.warning ?? 'Письмо отправлено.')
      }
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  const totalSize = files.reduce((sum, f) => sum + f.size, 0)

  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget && !busy) onClose() }}>
      <form
        className={`compose-modal ${dropping ? 'dropping' : ''}`}
        onSubmit={submit}
        onDragOver={(e) => { if (e.dataTransfer.types.includes('Files')) { e.preventDefault(); e.dataTransfer.dropEffect = 'copy'; setDropping(true) } }}
        onDragLeave={(e) => { if (e.currentTarget === e.target) setDropping(false) }}
        onDrop={(e) => { if (e.dataTransfer.files.length > 0) { e.preventDefault(); e.stopPropagation(); void dropFiles(e.dataTransfer.files) } }}
      >
        {dropping && <div className="drop-overlay"><Paperclip size={22} /> Отпустите, чтобы вложить файлы</div>}
        <div className="compose-header">
          <div><strong>{TITLES[draft.mode]}</strong><small>От: {from}</small></div>
          <button type="button" className="icon-button" disabled={busy} onClick={onClose} aria-label="Закрыть"><X size={18} /></button>
        </div>
        <RecipientField label="Кому" value={to} onChange={setTo} contacts={contacts} autoFocus={draft.mode !== 'reply' && draft.mode !== 'replyAll'} />
        {showCc
          ? <RecipientField label="Копия" value={cc} onChange={setCc} contacts={contacts} />
          : <button type="button" className="text-button add-cc" onClick={() => setShowCc(true)}>+ Копия</button>}
        <label className="compose-field"><span>Тема</span><input value={subject} spellCheck lang="ru" onChange={(e) => setSubject(e.target.value)} placeholder="Тема письма" /></label>
        <RichEditor ref={editor} initialText={draft.body} initialHtml={draft.html} autoFocus={draft.mode === 'reply' || draft.mode === 'replyAll'} onNotice={setError} />
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
          {ncConnected && <button type="button" className="text-button nc-link-button" disabled={busy} onClick={() => setNcPicker(true)} title="Вставить ссылку на файл из Nextcloud или загрузить большой файл в облако"><Cloud size={15} /> Ссылка Nextcloud</button>}
          <div className="tpl-wrap">
            <button type="button" className="text-button" disabled={busy} aria-label="Шаблоны" onClick={openTemplates}><FileText size={15} /> Шаблоны</button>
            {tplOpen && <div className="schedule-pop tpl-pop popover" role="dialog" aria-label="Шаблоны писем">
              <strong><FileText size={15} /> Шаблоны</strong>
              {templates === null && <small className="settings-hint">Загрузка…</small>}
              {templates?.length === 0 && <small className="settings-hint">Шаблонов пока нет — сохраните текущее письмо как шаблон.</small>}
              {templates?.map((t) => (
                <div key={`${t.folder}/${t.uid}`} className="tpl-item">
                  <button type="button" className="schedule-preset" onClick={() => applyTemplate(t)}><span>{t.name}</span></button>
                  <button type="button" className="icon-button" aria-label={`Удалить шаблон ${t.name}`} onClick={() => void deleteTemplate(t)}><Trash2 size={14} /></button>
                </div>
              ))}
              <label className="schedule-custom">
                <input aria-label="Название шаблона" placeholder={subject || 'Название шаблона'} value={tplName} onChange={(e) => setTplName(e.target.value)} />
                <button type="button" className="secondary-button" onClick={() => void saveTemplate()}>Сохранить текущее</button>
              </label>
              <small className="settings-hint">Шаблоны хранятся в папке «Templates» на сервере, как в Thunderbird.</small>
            </div>}
          </div>
          <label className="text-button receipt-toggle" title="Попросить получателя подтвердить, что письмо прочитано">
            <input type="checkbox" checked={receipt} onChange={(e) => setReceipt(e.target.checked)} /> <MailCheck size={15} /> Уведомить о прочтении
          </label>
          <div className="reader-toolbar-spacer" />
          <div className="send-split">
            <button className="primary-button" type="submit" disabled={busy}>
              <Send size={16} /> {busy ? 'Отправка…' : 'Отправить'}
            </button>
            <button type="button" className="primary-button send-more" disabled={busy} aria-label="Отправить позже" title="Отправить позже" onClick={() => { setTplOpen(false); setSchedOpen((o) => !o) }}><ChevronDown size={16} /></button>
            {schedOpen && <div className="schedule-pop popover" role="dialog" aria-label="Отправить позже">
              <strong><CalendarClock size={15} /> Отправить позже</strong>
              {schedulePresets().map((p) => (
                <button type="button" key={p.label} className="schedule-preset" onClick={() => { setSchedOpen(false); void submit(null, p.at) }}>
                  <span>{p.label}</span><small>{formatWhen(p.at.toISOString())}</small>
                </button>
              ))}
              <label className="schedule-custom">
                <input type="datetime-local" aria-label="Дата и время отправки" value={schedAt} min={localInputValue(new Date())} onChange={(e) => setSchedAt(e.target.value)} />
                <button type="button" className="secondary-button" disabled={!schedAt} onClick={() => { setSchedOpen(false); void submit(null, new Date(schedAt)) }}>Запланировать</button>
              </label>
              <small className="settings-hint">Письмо ждёт в «Черновиках» и уходит в срок, пока Atom Mail запущен (можно свернуть в трей); если программа была закрыта — при следующем запуске.</small>
            </div>}
          </div>
        </div>
      </form>
      {ncPicker && <NcFilePicker onClose={() => setNcPicker(false)} onPick={(links) => { setNcPicker(false); editor.current?.insertLinks(links) }} />}
    </div>
  )
}
