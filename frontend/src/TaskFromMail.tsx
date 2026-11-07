import { useEffect, useMemo, useState } from 'react'
import { CalendarDays, FolderUp, Handshake, Loader2, X } from 'lucide-react'

import { api, errorText } from './api'
import { cardUrl, joinLinks } from './deckLinks'
import type { DavCollection, DeckBoard, DeckStack, MessageDetail, MessageSummary } from './types'

/**
 * «Задача из письма»: карточка Deck со ссылкой на письмо, организацией и сроком;
 * срок — ещё и событием в календаре, вложения — в папку организации в Nextcloud
 * («/Контрагенты/<домен>/<задача>»). Вся цепочка потом видна в истории организации.
 */
export const COUNTERPARTY_DIR = '/Контрагенты'
const TARGET_KEY = 'atom.task.target'

interface Target { boardId?: number; stackId?: number; calendarUrl?: string }
function loadTarget(): Target {
  try { return JSON.parse(localStorage.getItem(TARGET_KEY) ?? '{}') as Target } catch { return {} }
}

export function domainOf(addr: string): string | null {
  const m = /<([^>]+)>/.exec(addr)
  const d = (m ? m[1] : addr).trim().toLowerCase().split('@')[1]
  return d && d.includes('.') ? d.replace(/\.$/, '') : null
}

/** Имя папки из названия задачи: без символов, недопустимых в Windows и Nextcloud. */
export function folderName(title: string): string {
  return title.replace(/[\\/:*?"<>|\u0000-\u001f]+/g, ' ').replace(/\s+/g, ' ').trim().replace(/[. ]+$/, '').slice(0, 80) || 'Задача'
}

const pad = (n: number) => String(n).padStart(2, '0')
const ymd = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`

export default function TaskFromMail({ message, ownAddress, onClose, onNotice, onOpenCard }: {
  message: MessageSummary
  /** Свой адрес — свою организацию не предлагаем. */
  ownAddress?: string
  onClose: () => void
  onNotice: (text: string) => void
  onOpenCard?: (boardId: number, cardId: number) => void
}) {
  const saved = useMemo(loadTarget, [])
  const [detail, setDetail] = useState<MessageDetail | null>(null)
  const [boards, setBoards] = useState<DeckBoard[] | null>(null)
  const [stacks, setStacks] = useState<DeckStack[]>([])
  const [calendars, setCalendars] = useState<DavCollection[]>([])
  const ownDomain = ownAddress ? domainOf(ownAddress) : null
  const [base, setBase] = useState('')
  const [boardId, setBoardId] = useState<number | null>(saved.boardId ?? null)
  const [stackId, setStackId] = useState<number | null>(saved.stackId ?? null)
  const [title, setTitle] = useState(message.subject.trim() || `Письмо от ${message.fromName || message.fromAddress}`)
  const [body, setBody] = useState('')
  const [due, setDue] = useState('')
  const [toCalendar, setToCalendar] = useState(true)
  const [calendarUrl, setCalendarUrl] = useState(saved.calendarUrl ?? '')
  const [org, setOrg] = useState('')
  const [saveFiles, setSaveFiles] = useState(true)
  const [files, setFiles] = useState<Set<number>>(new Set())
  const [busy, setBusy] = useState('')
  const [error, setError] = useState('')

  useEffect(() => {
    api.getMessage(message.folder, message.uid).then((d) => {
      setDetail(d)
      setFiles(new Set(d.attachments.map((a) => a.index)))
      const when = d.date ? new Date(d.date).toLocaleString('ru-RU', { day: 'numeric', month: 'long', year: 'numeric', hour: '2-digit', minute: '2-digit' }) : ''
      const text = d.bodyText.replace(/\r/g, '').replace(/\n{3,}/g, '\n\n').trim()
      setBody(`Письмо от ${d.fromName ? `${d.fromName} <${d.fromAddress}>` : d.fromAddress}${when ? `, ${when}` : ''}\n\n${text.length > 800 ? `${text.slice(0, 800)}…` : text}`)
    }).catch((e) => setError(`Письмо: ${errorText(e)}`))
    api.deckBoards().then((list) => {
      const ok = list.filter((b) => b.canEdit && !b.archived)
      setBoards(ok)
      setBoardId((id) => (ok.some((b) => b.id === id) ? id : ok[0]?.id ?? null))
    }).catch((e) => { setBoards([]); setError(`Доски Deck: ${errorText(e)}`) })
    const now = new Date()
    api.listEvents(now, new Date(now.getTime() + 86400000)).then((r) => {
      const w = r.calendars.filter((c) => c.writable)
      setCalendars(w)
      setCalendarUrl((u) => (w.some((c) => c.url === u) ? u : w[0]?.url ?? ''))
    }).catch(() => setCalendars([]))
    api.ncStatus().then((s) => setBase(s.url)).catch(() => undefined)
  }, [message.folder, message.uid])

  useEffect(() => {
    if (boardId === null) { setStacks([]); return }
    api.deckStacks(boardId).then((s) => {
      setStacks(s)
      setStackId((id) => (s.some((x) => x.id === id) ? id : s[0]?.id ?? null))
    }).catch((e) => setError(`Колонки: ${errorText(e)}`))
  }, [boardId])

  /** Организации письма: отправитель и получатели, кроме своей. */
  const domains = useMemo(() => {
    const out: string[] = []
    for (const a of [message.fromAddress, ...message.to, ...(detail?.cc ?? [])]) {
      const d = domainOf(a)
      if (d && d !== ownDomain && !out.includes(d)) out.push(d)
    }
    return out
  }, [message, detail, ownDomain])
  useEffect(() => { setOrg((o) => o || domains[0] || '') }, [domains])

  const orgDomain = org.trim().toLowerCase().replace(/^@/, '')
  const orgOk = !orgDomain || /^[\p{L}\d-]+(\.[\p{L}\d-]+)+$/u.test(orgDomain)
  const dir = `${orgDomain ? `${COUNTERPARTY_DIR}/${orgDomain}` : '/Задачи'}/${folderName(title)}`
  const attachments = detail?.attachments ?? []

  async function create() {
    if (!boardId || !stackId) { setError('Выберите доску и колонку'); return }
    if (!title.trim()) { setError('Укажите название задачи'); return }
    if (!orgOk) { setError('Организация — это домен адреса, например niiar.ru'); return }
    setError('')
    try { localStorage.setItem(TARGET_KEY, JSON.stringify({ boardId, stackId, calendarUrl })) } catch { /* необязательно */ }
    const warnings: string[] = []
    // 1. Вложения — в папку задачи.
    const saved: string[] = []
    const chosen = saveFiles ? attachments.filter((a) => files.has(a.index)) : []
    for (const [i, a] of chosen.entries()) {
      setBusy(`Сохраняю вложения в Nextcloud: ${i + 1} из ${chosen.length}…`)
      try {
        const path = await api.ncSaveMailAttachment(message.folder, message.uid, a.index, dir)
        saved.push(path.split('/').pop() || a.name)
      } catch (e) {
        warnings.push(`«${a.name}»: ${errorText(e)}`)
      }
    }
    // 2. Карточка со связями: организация, письмо, папка документов.
    setBusy('Создаю задачу…')
    const msgId = (detail?.messageId ?? message.msgId ?? '').replace(/^<|>$/g, '')
    const text = body.trim() + (saved.length ? `\n\nДокументы в Nextcloud: ${saved.join(', ')}` : '')
    const description = joinLinks(text, {
      parent: null,
      after: [],
      org: orgDomain || null,
      mail: msgId ? { title: message.subject || '(без темы)', msgId } : null,
      folder: saved.length ? { title: dir.replace(/^\//, ''), path: dir } : null,
    }, base || 'https://nextcloud')
    let cardId: number
    try {
      cardId = await api.deckCreateCard(boardId, stackId, title.trim(), description, due ? new Date(`${due}T12:00:00`).toISOString() : null)
    } catch (e) {
      setBusy('')
      setError(`Задача не создана: ${errorText(e)}${saved.length ? ` (вложения уже в «${dir}»)` : ''}`)
      return
    }
    // 3. Срок — событием на весь день.
    if (due && toCalendar && calendarUrl) {
      setBusy('Добавляю срок в календарь…')
      const next = new Date(`${due}T12:00:00`)
      next.setDate(next.getDate() + 1)
      try {
        await api.createEvent(calendarUrl, {
          title: `Срок: ${title.trim()}`,
          location: null,
          description: [
            base && cardId ? `Задача: ${cardUrl(base, cardId)}` : 'Задача в Deck',
            orgDomain ? `Контрагент: ${orgDomain}` : '',
            `Письмо: ${message.subject || '(без темы)'} — ${message.fromName || message.fromAddress}`,
          ].filter(Boolean).join('\n'),
          start: due,
          end: ymd(next),
          allDay: true,
        })
      } catch (e) {
        warnings.push(`Календарь: ${errorText(e)}`)
      }
    }
    setBusy('')
    onNotice(warnings.length ? `Задача создана, но не всё получилось: ${warnings.join('; ')}` : `Задача «${title.trim()}» создана${orgDomain ? ` и связана с ${orgDomain}` : ''}.`)
    onClose()
    if (cardId && onOpenCard) onOpenCard(boardId, cardId)
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget && !busy) onClose() }}>
      <div className="settings-modal task-from-mail" role="dialog" aria-label="Задача из письма">
        <div className="compose-header">
          <div><strong>Задача из письма</strong><small>{message.subject || '(без темы)'} — {message.fromName || message.fromAddress}</small></div>
          <button className="icon-button" onClick={onClose} disabled={Boolean(busy)} aria-label="Закрыть"><X size={18} /></button>
        </div>
        <div className="task-form">
          <label className="task-wide"><span>Название</span><input autoFocus value={title} maxLength={255} onChange={(e) => setTitle(e.target.value)} aria-label="Название задачи" /></label>
          <label><span>Доска</span>
            <select value={boardId ?? ''} onChange={(e) => setBoardId(Number(e.target.value) || null)} aria-label="Доска">
              {boards === null && <option value="">Загрузка…</option>}
              {boards?.length === 0 && <option value="">Нет досок, где можно создавать карточки</option>}
              {boards?.map((b) => <option key={b.id} value={b.id}>{b.title}</option>)}
            </select>
          </label>
          <label><span>Колонка</span>
            <select value={stackId ?? ''} onChange={(e) => setStackId(Number(e.target.value) || null)} aria-label="Колонка">
              {stacks.map((s) => <option key={s.id} value={s.id}>{s.title}</option>)}
            </select>
          </label>
          <label><span><Handshake size={14} /> Организация</span>
            <input list="task-org-list" value={org} onChange={(e) => setOrg(e.target.value)} placeholder="домен, например niiar.ru" aria-label="Организация" className={orgOk ? '' : 'invalid'} />
            <datalist id="task-org-list">{domains.map((d) => <option key={d} value={d} />)}</datalist>
          </label>
          <label><span><CalendarDays size={14} /> Срок</span><input type="date" value={due} onChange={(e) => setDue(e.target.value)} aria-label="Срок" /></label>
          {due && (
            <label className="task-check task-wide">
              <input type="checkbox" checked={toCalendar && calendars.length > 0} disabled={calendars.length === 0} onChange={(e) => setToCalendar(e.target.checked)} aria-label="Срок в календарь" />
              <span>Добавить срок в календарь</span>
              {calendars.length > 1 && toCalendar && (
                <select value={calendarUrl} onChange={(e) => setCalendarUrl(e.target.value)} aria-label="Календарь">
                  {calendars.map((c) => <option key={c.url} value={c.url}>{c.name}</option>)}
                </select>
              )}
              {calendars.length === 0 && <small className="muted">нет календаря для записи</small>}
            </label>
          )}
          {attachments.length > 0 && (
            <div className="task-files task-wide">
              <label className="task-check">
                <input type="checkbox" checked={saveFiles} onChange={(e) => setSaveFiles(e.target.checked)} aria-label="Сохранить вложения в Nextcloud" />
                <span><FolderUp size={14} /> Сохранить вложения в Nextcloud</span>
              </label>
              {saveFiles && <>
                <small className="muted task-dir" title={dir}>в папку {dir}</small>
                <div className="task-file-list">
                  {attachments.map((a) => (
                    <label key={a.index} className="task-check">
                      <input type="checkbox" checked={files.has(a.index)} aria-label={a.name}
                        onChange={(e) => setFiles((s) => { const n = new Set(s); if (e.target.checked) n.add(a.index); else n.delete(a.index); return n })} />
                      <span>{a.name}</span>
                    </label>
                  ))}
                </div>
              </>}
            </div>
          )}
          <label className="task-wide"><span>Описание</span><textarea value={body} rows={6} onChange={(e) => setBody(e.target.value)} aria-label="Описание задачи" /></label>
        </div>
        {error && <div className="login-error" role="alert">{error}</div>}
        <div className="settings-actions">
          {busy && <span className="muted task-busy"><Loader2 className="spin" size={14} /> {busy}</span>}
          <div className="reader-toolbar-spacer" />
          <button className="text-button" onClick={onClose} disabled={Boolean(busy)}>Отмена</button>
          <button className="primary-button" disabled={Boolean(busy) || !boardId || !stackId || !detail} onClick={() => void create()}>Создать задачу</button>
        </div>
      </div>
    </div>
  )
}
