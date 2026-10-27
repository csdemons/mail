import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  Archive, CalendarDays, ChevronDown, ChevronLeft, ChevronRight, Circle, ContactRound, Download,
  FilePenLine, FileText, Folder as FolderIcon, FolderOpen, Forward, Inbox, LogOut, Mail,
  MessageCircle, Moon, Paperclip, PanelLeftClose, PanelRightClose, PanelRightOpen, RefreshCw,
  Reply, Search, Send, Settings, ShieldCheck, SquarePen, Star, Stethoscope, Sun, Trash2, Users, X, Copy,
} from 'lucide-react'

import {
  api, errorText, formatFullDate, formatListDate, formatSize, messageKey, PAGE_SIZE,
} from './api'
import type {
  Account, Contact, Draft, Folder, FolderRole, MailFilter, MessageDetail, MessageSummary, Settings as Profile,
} from './types'
import Composer from './Composer'
import ContactsView from './ContactsView'
import CalendarView from './CalendarView'

const ROLE_LABELS: Partial<Record<FolderRole, string>> = {
  inbox: 'Входящие',
  drafts: 'Черновики',
  sent: 'Отправленные',
  archive: 'Архив',
  junk: 'Спам',
  trash: 'Корзина',
}

const ROLE_ICONS: Record<FolderRole, typeof Inbox> = {
  inbox: Inbox,
  drafts: FilePenLine,
  sent: Send,
  archive: Archive,
  junk: ShieldCheck,
  trash: Trash2,
  other: FolderIcon,
}

function folderLabel(folder: Folder): string {
  if (folder.system) return ROLE_LABELS[folder.role] ?? folder.name
  return folder.name.toUpperCase() === 'INBOX' ? 'Входящие' : folder.name
}

const EXPANDED_KEY = 'rrcki-mail.expanded-folders'

function loadExpanded(): Set<string> {
  try {
    const raw = localStorage.getItem(EXPANDED_KEY)
    if (raw) return new Set(JSON.parse(raw) as string[])
  } catch { /* хранилище недоступно — используем значение по умолчанию */ }
  return new Set(['INBOX', 'shared'])
}

function saveExpanded(ids: Set<string>) {
  try { localStorage.setItem(EXPANDED_KEY, JSON.stringify([...ids])) } catch { /* не критично */ }
}

function senderLabel(m: MessageSummary): string {
  return m.fromName || m.fromAddress || '(без отправителя)'
}

function pluralLetters(n: number): string {
  const mod10 = n % 10
  const mod100 = n % 100
  if (mod10 === 1 && mod100 !== 11) return 'письмо'
  if (mod10 >= 2 && mod10 <= 4 && (mod100 < 12 || mod100 > 14)) return 'письма'
  return 'писем'
}

function initials(value: string) {
  return value.trim().slice(0, 1).toUpperCase() || '?'
}

interface Props {
  account: Account
  initialNotice: string
  onLogout: () => void
}

export default function App({ account, initialNotice, onLogout }: Props) {
  const [dark, setDark] = useState(() => window.matchMedia?.('(prefers-color-scheme: dark)').matches ?? false)
  const [foldersOpen, setFoldersOpen] = useState(true)
  const [readerOpen, setReaderOpen] = useState(true)
  const [showAccountMenu, setShowAccountMenu] = useState(false)
  const [showSettings, setShowSettings] = useState(false)
  const [notice, setNotice] = useState(initialNotice)

  const [folders, setFolders] = useState<Folder[]>([])
  const [folderId, setFolderId] = useState('INBOX')
  const [expanded, setExpanded] = useState<Set<string>>(loadExpanded)

  const [messages, setMessages] = useState<MessageSummary[]>([])
  const [total, setTotal] = useState(0)
  const [hasMore, setHasMore] = useState(false)
  const [loading, setLoading] = useState(false)
  const [loadingMore, setLoadingMore] = useState(false)
  const [listError, setListError] = useState('')

  const [selectedKey, setSelectedKey] = useState('')
  const [detail, setDetail] = useState<MessageDetail | null>(null)
  const [detailLoading, setDetailLoading] = useState(false)
  const [detailError, setDetailError] = useState('')

  const [query, setQuery] = useState('')
  const [filter, setFilter] = useState<MailFilter>('Все')

  const [diagText, setDiagText] = useState<string | null>(null)
  const [diagBusy, setDiagBusy] = useState(false)

  const [module, setModule] = useState<'mail' | 'calendar' | 'contacts'>('mail')
  const [draft, setDraft] = useState<Draft | null>(null)
  const [contacts, setContacts] = useState<Contact[]>([])
  const contactsRequested = useRef(false)
  const [profile, setProfile] = useState<Profile | null>(null)
  const [profileForm, setProfileForm] = useState({ displayName: '', smtpHost: '', smtpPort: '465', davUrl: '' })
  const [profileBusy, setProfileBusy] = useState(false)

  useEffect(() => {
    api.getProfile().then((p) => {
      setProfile(p)
      if (p) setProfileForm({ displayName: p.displayName, smtpHost: p.smtpHost, smtpPort: String(p.smtpPort), davUrl: p.davUrl })
    }).catch(() => undefined)
  }, [])

  // Защита от гонок: ответ на устаревший запрос игнорируется.
  const listRequest = useRef(0)
  const detailRequest = useRef(0)
  const searchRef = useRef<HTMLInputElement>(null)

  const currentFolder = folders.find((f) => f.id === folderId)

  const visibleFolders = useMemo(() => {
    const byId = new Map(folders.map((f) => [f.id, f]))
    return folders.filter((f) => {
      for (let p = f.parent; p; p = byId.get(p)?.parent ?? null) {
        if (!expanded.has(p)) return false
      }
      return true
    })
  }, [folders, expanded])

  function toggleExpanded(id: string) {
    setExpanded((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      saveExpanded(next)
      return next
    })
  }

  const loadFolders = useCallback(async () => {
    try {
      const list = await api.listFolders()
      setFolders(list)
      setFolderId((id) => (list.some((f) => f.id === id && f.selectable)
        ? id
        : list.find((f) => f.role === 'inbox')?.id ?? list.find((f) => f.selectable)?.id ?? 'INBOX'))
    } catch (error) {
      setNotice(`Не удалось получить список папок: ${errorText(error)}`)
    }
  }, [])

  const loadFirstPage = useCallback(async (folder: string, keepSelection: boolean) => {
    const request = ++listRequest.current
    setLoading(true)
    setListError('')
    if (!keepSelection) {
      setMessages([])
      setSelectedKey('')
      setDetail(null)
    }
    try {
      const page = await api.listMessages(folder, null, PAGE_SIZE)
      if (request !== listRequest.current) return
      setMessages(page.items)
      setTotal(page.total)
      setHasMore(page.hasMore)
      setSelectedKey((key) => (page.items.some((m) => messageKey(m) === key) ? key : ''))
      return page.items.length
    } catch (error) {
      if (request === listRequest.current) setListError(errorText(error))
    } finally {
      if (request === listRequest.current) setLoading(false)
    }
  }, [])

  useEffect(() => { void loadFolders() }, [loadFolders])
  useEffect(() => { void loadFirstPage(folderId, false) }, [folderId, loadFirstPage])

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {
        event.preventDefault()
        searchRef.current?.focus()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  async function loadMore() {
    const request = listRequest.current
    setLoadingMore(true)
    try {
      const oldest = messages.reduce<number | null>((min, m) => (min === null || m.uid < min ? m.uid : min), null)
      const page = await api.listMessages(folderId, oldest, PAGE_SIZE)
      if (request !== listRequest.current) return
      setMessages((items) => {
        const known = new Set(items.map(messageKey))
        return [...items, ...page.items.filter((m) => !known.has(messageKey(m)))]
      })
      setTotal(page.total)
      setHasMore(page.hasMore)
    } catch (error) {
      setNotice(`Не удалось загрузить письма: ${errorText(error)}`)
    } finally {
      setLoadingMore(false)
    }
  }

  async function refresh() {
    const [, count] = await Promise.all([loadFolders(), loadFirstPage(folderId, true)])
    if (typeof count === 'number') setNotice(`Список обновлён: ${count} из ${total || count} писем загружено.`)
  }

  function patchMessage(key: string, patch: Partial<MessageSummary>) {
    setMessages((items) => items.map((m) => (messageKey(m) === key ? { ...m, ...patch } : m)))
    setDetail((d) => (d && messageKey(d) === key ? { ...d, ...patch } : d))
  }

  function adjustUnseen(folder: string, delta: number) {
    setFolders((list) => list.map((f) => (f.id === folder ? { ...f, unseen: Math.max(0, f.unseen + delta) } : f)))
  }

  async function openMessage(message: MessageSummary) {
    const key = messageKey(message)
    const request = ++detailRequest.current
    setSelectedKey(key)
    setReaderOpen(true)
    setDetail(null)
    setDetailError('')
    setDetailLoading(true)
    try {
      const loaded = await api.getMessage(message.folder, message.uid)
      if (request !== detailRequest.current) return
      setDetail(loaded)
      patchMessage(key, { hasAttachments: loaded.hasAttachments })
      if (loaded.unread) {
        patchMessage(key, { unread: false })
        adjustUnseen(message.folder, -1)
        api.setFlag(message.folder, message.uid, 'seen', true).catch((error) => {
          patchMessage(key, { unread: true })
          adjustUnseen(message.folder, +1)
          setNotice(`Не удалось отметить письмо прочитанным: ${errorText(error)}`)
        })
      }
    } catch (error) {
      if (request === detailRequest.current) setDetailError(errorText(error))
    } finally {
      if (request === detailRequest.current) setDetailLoading(false)
    }
  }

  async function toggleFlag(message: MessageSummary, flag: 'seen' | 'flagged') {
    const key = messageKey(message)
    const field = flag === 'seen' ? 'unread' : 'flagged'
    const before = message[field]
    const after = !before
    patchMessage(key, { [field]: after })
    if (flag === 'seen') adjustUnseen(message.folder, after ? +1 : -1)
    try {
      // unread=true ⇔ флаг \Seen снят
      await api.setFlag(message.folder, message.uid, flag, flag === 'seen' ? !after : after)
    } catch (error) {
      patchMessage(key, { [field]: before })
      if (flag === 'seen') adjustUnseen(message.folder, after ? -1 : +1)
      setNotice(`Не удалось изменить письмо: ${errorText(error)}`)
    }
  }

  async function runDiagnostics() {
    setDiagBusy(true)
    setDiagText('Сбор данных…')
    try {
      const report = await api.diagnostics(currentFolder?.selectable ? folderId : null)
      setDiagText(`Папка в интерфейсе: ${currentFolder?.path ?? folderId}, загружено писем: ${messages.length}, сервер сообщал: ${currentFolder?.total ?? '?'}\n${report}`)
    } catch (error) {
      setDiagText(`Ошибка диагностики: ${errorText(error)}`)
    } finally {
      setDiagBusy(false)
    }
  }

  async function copyDiagnostics() {
    if (!diagText) return
    try {
      await navigator.clipboard.writeText(diagText)
      setNotice('Отчёт скопирован в буфер обмена.')
    } catch {
      const area = document.querySelector<HTMLTextAreaElement>('.diag-text')
      area?.select()
      setNotice('Выделите текст и нажмите Ctrl+C.')
    }
  }

  async function saveAttachment(index: number) {
    if (!detail) return
    try {
      const path = await api.saveAttachment(detail.folder, detail.uid, index)
      if (path) setNotice(`Сохранено: ${path}`)
    } catch (error) {
      setNotice(`Не удалось сохранить вложение: ${errorText(error)}`)
    }
  }

  const visibleMessages = useMemo(() => {
    const q = query.trim().toLowerCase()
    return messages.filter((m) => {
      const filterMatch =
        filter === 'Все' ? true :
        filter === 'Непрочитанные' ? m.unread :
        filter === 'Помеченные' ? m.flagged :
        m.hasAttachments
      if (!filterMatch) return false
      if (!q) return true
      return `${m.fromName} ${m.fromAddress} ${m.subject} ${m.to.join(' ')}`.toLowerCase().includes(q)
    })
  }, [messages, filter, query])

  const selected = messages.find((m) => messageKey(m) === selectedKey)

  function startDraft(next: Draft) {
    setDraft(next)
    // Контакты для подсказок адресов — один раз за сессию, ошибки не мешают письму.
    if (!contactsRequested.current) {
      contactsRequested.current = true
      api.listContacts().then((r) => setContacts(r.contacts)).catch(() => undefined)
    }
  }

  function openComposer(mode: 'new' | 'reply' | 'forward') {
    const m = mode === 'new' ? null : detail
    const withPrefix = (prefix: string, subj: string) =>
      new RegExp(`^${prefix}:`, 'i').test(subj.trim()) ? subj.trim() : `${prefix}: ${subj.trim()}`
    if (!m) {
      startDraft({ mode: 'new', to: '', cc: '', subject: '', body: '', inReplyTo: null, references: null })
      return
    }
    const sender = m.fromName ? `${m.fromName} <${m.fromAddress}>` : m.fromAddress
    const header = `${formatFullDate(m.date)}, ${senderLabel(m)} пишет:`
    if (mode === 'reply') {
      startDraft({
        mode,
        to: m.replyTo ?? sender,
        cc: '',
        subject: withPrefix('Re', m.subject),
        body: `\n\n${header}\n${m.bodyText.split('\n').map((l) => `> ${l}`).join('\n')}`,
        inReplyTo: m.messageId,
        references: [m.references, m.messageId].filter(Boolean).join(' ') || null,
      })
    } else {
      startDraft({
        mode,
        to: '',
        cc: '',
        subject: withPrefix('Fwd', m.subject),
        body: `\n\n---------- Пересланное сообщение ----------\nОт: ${sender}\nДата: ${formatFullDate(m.date)}\nТема: ${m.subject}\nКому: ${m.to.join(', ')}\n\n${m.bodyText}`,
        inReplyTo: null,
        references: null,
      })
    }
  }

  async function saveProfile() {
    const port = Number(profileForm.smtpPort)
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
      setNotice('Некорректный порт SMTP')
      return
    }
    setProfileBusy(true)
    try {
      const p = await api.updateProfile({ ...profileForm, smtpPort: port })
      setProfile(p)
      setNotice('Настройки сохранены.')
    } catch (error) {
      setNotice(`Не удалось сохранить настройки: ${errorText(error)}`)
    } finally {
      setProfileBusy(false)
    }
  }

  const listTitle = currentFolder ? folderLabel(currentFolder) : 'Входящие'

  return (
    <div className={`app-shell ${dark ? 'theme-dark' : ''}`}>
      <header className="topbar">
        <div className="brand">
          <div className="brand-mark"><Mail size={19} fill="currentColor" /></div>
          <span>RRCKI Mail</span>
        </div>
        <button className="account-switch" onClick={() => setShowAccountMenu(!showAccountMenu)}>
          <div className="account-avatar"><Users size={20} /></div>
          <span><strong>{account.username}</strong><small>IMAP · {account.host}</small></span>
          <ChevronDown size={17} />
        </button>
        <button className="primary-button compose-top" onClick={() => openComposer('new')}>
          <SquarePen size={17} /> Новое сообщение
        </button>
        <div className="topbar-actions">
          <button className="icon-button" aria-label="Тема" onClick={() => setDark(!dark)}>{dark ? <Sun /> : <Moon />}</button>
          <button className="icon-button" aria-label="Настройки" onClick={() => setShowSettings(true)}><Settings /></button>
        </div>
        {showAccountMenu && <div className="account-menu popover">
          <strong>Учётная запись</strong>
          <div className="account-menu-item"><span className="status-dot" /> {account.username} <small>{account.host}:{account.port}</small></div>
          <button className="text-button" onClick={() => { setShowAccountMenu(false); onLogout() }}><LogOut size={15} /> Выйти</button>
        </div>}
      </header>

      <div className="workspace">
        <nav className="module-rail" aria-label="Модули">
          <button className={`module ${module === 'mail' ? 'active' : ''}`} onClick={() => setModule('mail')}><Mail /><span>Почта</span></button>
          <button className={`module ${module === 'calendar' ? 'active' : ''}`} onClick={() => setModule('calendar')}><CalendarDays /><span>Календарь</span></button>
          <button className={`module ${module === 'contacts' ? 'active' : ''}`} onClick={() => setModule('contacts')}><ContactRound /><span>Контакты</span></button>
          <button className="module" onClick={() => setNotice('Задачи будут добавлены в следующих версиях.')}><FileText /><span>Задачи</span></button>
          <button className="module" onClick={() => setNotice('Чат будет добавлен в следующих версиях.')}><MessageCircle /><span>Чат</span></button>
          <div className="rail-spacer" />
        </nav>

        {module === 'calendar' && <CalendarView onNotice={setNotice} />}
        {module === 'contacts' && (
          <ContactsView
            onLoaded={(list) => { setContacts(list); contactsRequested.current = true }}
            onNotice={setNotice}
            onWrite={(address) => startDraft({ mode: 'new', to: `${address}, `, cc: '', subject: '', body: '', inReplyTo: null, references: null })}
          />
        )}
        {module === 'mail' && <>
        {foldersOpen && <aside className="folder-sidebar">
          <div className="sidebar-heading"><span>Папки</span><ChevronDown size={15} /></div>
          <div className="folder-list">
            {folders.length === 0 && <div className="folder-placeholder">Загрузка папок…</div>}
            {visibleFolders.map((f) => {
              const Icon = f.role === 'other' && !f.selectable ? FolderOpen : ROLE_ICONS[f.role]
              const isOpen = expanded.has(f.id)
              const indent = { paddingLeft: 4 + f.depth * 14 }
              const toggle = f.hasChildren ? (
                <span
                  role="button"
                  aria-label={isOpen ? 'Свернуть' : 'Развернуть'}
                  className="folder-toggle"
                  onClick={(e) => { e.stopPropagation(); toggleExpanded(f.id) }}
                >
                  {isOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                </span>
              ) : <span className="folder-toggle" />
              return f.selectable ? (
                <button
                  key={f.id}
                  className={`folder-item ${folderId === f.id ? 'selected' : ''} ${f.total === 0 ? 'folder-empty' : ''}`}
                  style={indent}
                  title={`${f.path} — ${f.total} ${pluralLetters(f.total)}${f.unseen ? `, непрочитанных ${f.unseen}` : ''}`}
                  onClick={() => setFolderId(f.id)}
                >
                  {toggle}
                  <Icon size={17} />
                  <span>{folderLabel(f)}</span>
                  {f.unseen > 0 && <b className="folder-count">{f.unseen}</b>}
                </button>
              ) : (
                <button key={f.id} className="folder-item folder-node" style={indent} title={f.path} onClick={() => f.hasChildren && toggleExpanded(f.id)}>
                  {toggle}
                  <Icon size={17} /><span>{folderLabel(f)}</span>
                </button>
              )
            })}
          </div>
          <div className="sidebar-footer">
            <div className="secure-label"><ShieldCheck size={15} /> Подключено по TLS</div>
            <span>{account.host}</span>
          </div>
        </aside>}

        <main className="message-column">
          <div className="list-toolbar">
            <button className="icon-button soft" aria-label="Показать папки" onClick={() => setFoldersOpen(!foldersOpen)}><PanelLeftClose size={18} /></button>
            <label className="search-box"><Search size={17} /><input ref={searchRef} value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Искать в загруженных письмах" /><kbd>Ctrl K</kbd></label>
            <button
              className="icon-button soft refresh-button"
              aria-label="Обновить"
              title={loading ? 'Загрузка писем…' : 'Обновить'}
              disabled={loading}
              onClick={() => void refresh()}
            >
              <RefreshCw size={17} className={loading ? 'spin' : ''} />
            </button>
          </div>
          <div className="filter-row">
            {(['Все', 'Непрочитанные', 'Помеченные', 'С вложениями'] as MailFilter[]).map((item) => (
              <button key={item} className={`filter-chip ${filter === item ? 'active' : ''}`} onClick={() => setFilter(item)}>{item}</button>
            ))}
          </div>
          <div className="message-list-header"><span>{listTitle}</span><span>{total} {pluralLetters(total)}</span></div>
          <div className="message-list">
            {loading && messages.length === 0 && <div className="empty-state"><span>Загрузка писем с сервера…</span></div>}
            {listError && (
              <div className="empty-state" role="alert">
                <strong>Не удалось загрузить письма</strong>
                <span>{listError}</span>
                <button className="text-button" onClick={() => void loadFirstPage(folderId, true)}>Повторить</button>
              </div>
            )}
            {visibleMessages.map((m) => {
              const key = messageKey(m)
              const sender = senderLabel(m)
              return (
                <button key={key} className={`message-row ${selectedKey === key ? 'selected' : ''} ${m.unread ? 'unread' : ''}`} onClick={() => void openMessage(m)}>
                  <div className={`sender-avatar avatar-${m.uid % 5}`}>{initials(sender)}</div>
                  <div className="message-summary">
                    <div className="message-topline"><span className="sender-name">{sender}</span><time>{formatListDate(m.date)}</time></div>
                    <div className="message-subject">{m.subject || '(без темы)'}</div>
                    <div className="message-meta">
                      {m.hasAttachments && <Paperclip size={13} />}
                      {m.flagged && <Star size={13} className="flag-mark" fill="currentColor" />}
                      {m.answered && <Reply size={13} />}
                      {m.unread && <span className="unread-dot" />}
                    </div>
                  </div>
                </button>
              )
            })}
            {!loading && !listError && visibleMessages.length === 0 && (
              <div className="empty-state">
                <Inbox size={30} />
                <strong>{messages.length === 0 ? 'Папка пуста' : 'Ничего не найдено'}</strong>
                {messages.length === 0 && (currentFolder?.total ?? 0) > 0 && <>
                  <span>Сервер сообщает о {currentFolder?.total} письмах в этой папке, но список не получен.</span>
                  <button className="text-button" onClick={() => { setShowSettings(true); void runDiagnostics() }}>Открыть диагностику</button>
                </>}
                {messages.length > 0 && <>
                  <span>Поиск и фильтры работают по загруженным письмам.</span>
                  <button className="text-button" onClick={() => { setQuery(''); setFilter('Все') }}>Сбросить фильтры</button>
                </>}
              </div>
            )}
            {hasMore && !listError && (
              <button className="text-button load-more" disabled={loadingMore} onClick={() => void loadMore()}>
                {loadingMore ? 'Загрузка…' : `Показать ещё (загружено ${messages.length} из ${total})`}
              </button>
            )}
          </div>
          <div className="list-footer">
            <span>{visibleMessages.length} из {messages.length} загруженных</span>
            <span>
              <Circle size={7} fill={listError ? '#ef4444' : '#22c55e'} />
              {listError ? 'Ошибка' : loading ? 'Синхронизация' : 'Онлайн'}
            </span>
          </div>
        </main>

        {readerOpen && <section className="reader-pane">
          <div className="reader-toolbar">
            <button className="icon-button soft mobile-back" onClick={() => setReaderOpen(false)} aria-label="Назад"><ChevronLeft /></button>
            <div className="reader-toolbar-spacer" />
            {selected && <>
              <button className={`icon-button ${selected.flagged ? 'starred' : ''}`} aria-label="Пометить" title="Пометить" onClick={() => void toggleFlag(selected, 'flagged')}>
                <Star size={19} fill={selected.flagged ? 'currentColor' : 'none'} />
              </button>
              <button className="icon-button" aria-label="Непрочитанное" title={selected.unread ? 'Отметить прочитанным' : 'Отметить непрочитанным'} onClick={() => void toggleFlag(selected, 'seen')}>
                <Mail size={18} />
              </button>
            </>}
            <button className="icon-button soft" aria-label="Скрыть чтение" onClick={() => setReaderOpen(false)}><PanelRightClose size={18} /></button>
          </div>
          {detail ? <div className="reader-content">
            <div className="reader-title-row"><h1>{detail.subject || '(без темы)'}</h1></div>
            <div className="sender-block">
              <div className="sender-avatar sender-avatar-large">{initials(senderLabel(detail))}</div>
              <div className="sender-details">
                <strong>{senderLabel(detail)}</strong>
                {detail.fromName && <span>{detail.fromAddress}</span>}
                {detail.to.length > 0 && <span>Кому: {detail.to.join(', ')}</span>}
                {detail.cc.length > 0 && <span>Копия: {detail.cc.join(', ')}</span>}
                <small>{formatFullDate(detail.date)}</small>
              </div>
            </div>
            {detail.attachments.length > 0 && <div className="attachment-list">
              {detail.attachments.map((a) => (
                <button key={a.index} className="attachment-card" title={`Сохранить ${a.name}`} onClick={() => void saveAttachment(a.index)}>
                  <div className="attachment-icon"><FileText size={21} /></div>
                  <div><strong>{a.name}</strong><span>{formatSize(a.size)} · {a.contentType}</span></div>
                  <Download size={17} />
                </button>
              ))}
            </div>}
            <article className="message-body plain">{detail.bodyText || '(пустое письмо)'}</article>
            <div className="reader-actions">
              <button className="primary-button" onClick={() => openComposer('reply')}><Reply size={16} /> Ответить</button>
              <button className="secondary-button" onClick={() => openComposer('forward')}><Forward size={16} /> Переслать</button>
            </div>
          </div> : <div className="empty-reader">
            {detailLoading ? <span>Загрузка письма…</span>
              : detailError ? <><strong>Не удалось открыть письмо</strong><span>{detailError}</span></>
              : <><Mail size={42} /><strong>Выберите письмо</strong><span>Выберите сообщение в списке для просмотра.</span></>}
          </div>}
        </section>}
        {!readerOpen && <button className="show-reader-button" onClick={() => setReaderOpen(true)}><PanelRightOpen size={18} /> Открыть письмо</button>}
        </>}
      </div>

      {notice && <div className="toast" role="status"><span>{notice}</span><button onClick={() => setNotice('')} aria-label="Закрыть"><X size={16} /></button></div>}

      {draft && (
        <Composer
          from={profile?.displayName ? `${profile.displayName} <${account.username}>` : account.username}
          draft={draft}
          contacts={contacts}
          onClose={() => setDraft(null)}
          onSent={(text) => {
            setDraft(null)
            setNotice(text)
            if (draft.mode === 'reply' && detail) patchMessage(messageKey(detail), { answered: true })
          }}
        />
      )}

      {showSettings && <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) setShowSettings(false) }}>
        <div className="settings-modal">
          <div className="compose-header"><div><strong>Учётная запись</strong><small>Параметры подключения</small></div><button className="icon-button" onClick={() => setShowSettings(false)} aria-label="Закрыть"><X size={18} /></button></div>
          <div className="settings-account">
            <div className="account-avatar"><Mail size={20} /></div>
            <div><strong>{account.username}</strong><span>IMAP · {account.host}:{account.port} · TLS</span></div>
            <span className="status-pill">Подключён</span>
          </div>
          <div className="profile-form">
            <label className="login-field"><span>Имя отправителя</span><input value={profileForm.displayName} onChange={(e) => setProfileForm((f) => ({ ...f, displayName: e.target.value }))} placeholder="Юшков Дмитрий" /></label>
            <div className="login-server">
              <label className="login-field"><span>Исходящая почта (SMTP)</span><input value={profileForm.smtpHost} onChange={(e) => setProfileForm((f) => ({ ...f, smtpHost: e.target.value }))} /></label>
              <label className="login-field login-port"><span>Порт</span><input inputMode="numeric" value={profileForm.smtpPort} onChange={(e) => setProfileForm((f) => ({ ...f, smtpPort: e.target.value }))} /></label>
            </div>
            <label className="login-field"><span>Календарь и контакты (CalDAV/CardDAV)</span><input value={profileForm.davUrl} onChange={(e) => setProfileForm((f) => ({ ...f, davUrl: e.target.value }))} placeholder={`https://${account.host}/ (автоопределение)`} /></label>
            <div className="settings-actions"><button className="primary-button" disabled={profileBusy} onClick={() => void saveProfile()}>Сохранить</button></div>
          </div>
          <div className="settings-note"><ShieldCheck size={18} /><span>Пароль хранится только в системном хранилище (Windows Credential Manager / GNOME Keyring) и не записывается в файлы настроек. Для SMTP, календаря и контактов используются те же логин и пароль.</span></div>
          <div className="settings-actions">
            <button className="secondary-button" disabled={diagBusy} onClick={() => void runDiagnostics()}><Stethoscope size={16} /> Диагностика</button>
            <button className="secondary-button" onClick={() => { setShowSettings(false); onLogout() }}><LogOut size={16} /> Выйти и забыть пароль</button>
          </div>
          {diagText !== null && <div className="diag-block">
            <div className="diag-head"><span>Отчёт: IMAP по текущей папке, SMTP, CalDAV/CardDAV (пароль не включается)</span><button className="text-button" onClick={() => void copyDiagnostics()}><Copy size={14} /> Скопировать</button></div>
            <textarea className="diag-text" readOnly value={diagText} />
          </div>}
        </div>
      </div>}
    </div>
  )
}
