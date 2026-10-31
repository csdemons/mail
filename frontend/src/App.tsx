import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  Archive, CalendarDays, ChevronDown, ChevronLeft, ChevronRight, Circle, Cloud, ContactRound, Download,
  FilePenLine, FileText, Folder as FolderIcon, FolderOpen, Forward, Inbox, LogOut, Mail,
  MessageCircle, Moon, Paperclip, PanelLeftClose, PanelRightClose, PanelRightOpen, RefreshCw,
  Reply, Search, Send, Settings, ShieldCheck, Lock, CalendarClock, Tag, MessagesSquare, AlarmClock, ShieldAlert, Printer, SquarePen, Stethoscope, Sun, Trash2, Users, X, Copy, FolderInput, ListFilter, ReplyAll, Flag, CircleAlert, ExternalLink, MailOpen,
} from 'lucide-react'

import {
  api, editNotice, errorText, formatFullDate, formatListDate, messageKey, NC_MAIL_DIR, PAGE_SIZE,
} from './api'
import type {
  Account, Contact, Draft, Folder, FolderRole, MailFilter, MessageDetail, MessageSummary, Scheduled, Category, Settings as Profile,
} from './types'
import About from './About'
import kurchatovLogo from './assets/kurchatov-logo.png'
import Composer from './Composer'
import { CHECK_UPDATE_EVENT } from './UpdateNotice'
import DeckView from './DeckView'
import TalkView from './TalkView'
import NcConnect from './NcConnect'
import FilesView from './FilesView'
import type { NcStatus } from './types'
import ContactsView from './ContactsView'
import CalendarView from './CalendarView'
import MailFilters from './MailFilters'
import ContextMenu from './ContextMenu'
import type { MenuItem } from './ContextMenu'
import { AttachmentList } from './AttachmentViewer'
import MessageBody from './MessageBody'
import AddressLine from './AddressLine'
import MdnBar from './MdnBar'
import DuePicker from './DuePicker'
import { printMessage, snoozePresets, snoozedUntil } from './mailTools'
import { categoriesOf, dueLabel, dueOf, duePresets, threadKeys, userCategories, withDue } from './mailExtras'
import ScheduledMail from './ScheduledMail'
import { useOutbox } from './sending'
import { CategoriesSettings, DesktopSettings, OfflineSettings, SendingSettings, SignatureSettings } from './MailSettings'
import { TransferSettings } from './Transfer'
import TotpSettings, { AUTOLOCK_KEY, loadAutolock } from './TotpSettings'
import { buildDraft, ensureArchive, withSignature } from './mailActions'
import FolderPicker, { sharedMailboxOf, trashFor } from './FolderPicker'
import { appInFront, loadPrefs, notify, savePrefs } from './notifications'
import type { NotifyPrefs } from './notifications'

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
  onLock: () => void
}

export default function App({ account, initialNotice, onLogout, onLock }: Props) {
  // Защита кодом: блокировка по Ctrl+L, из меню и при бездействии.
  const [totpOn, setTotpOn] = useState(false)
  const [autolock, setAutolock] = useState(loadAutolock)
  useEffect(() => { api.totpStatus().then((s) => setTotpOn(s.enabled)).catch(() => undefined) }, [])
  const lockRef = useRef(onLock)
  lockRef.current = onLock
  const composingRef = useRef(false)
  useEffect(() => {
    if (!totpOn) return
    function onKey(event: KeyboardEvent) {
      if ((event.ctrlKey || event.metaKey) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'l') {
        event.preventDefault()
        lockRef.current()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [totpOn])
  useEffect(() => {
    if (!totpOn || autolock <= 0) return
    let last = Date.now()
    const touch = () => { last = Date.now() }
    const events = ['mousemove', 'mousedown', 'keydown', 'wheel', 'touchstart'] as const
    events.forEach((e) => window.addEventListener(e, touch, { passive: true, capture: true }))
    const timer = window.setInterval(() => {
      // Незаконченное письмо не теряем: пока открыт редактор, автоблокировка ждёт.
      if (Date.now() - last >= autolock * 60_000 && !composingRef.current) lockRef.current()
    }, 10_000)
    return () => {
      window.clearInterval(timer)
      events.forEach((e) => window.removeEventListener(e, touch, { capture: true }))
    }
  }, [totpOn, autolock])
  function updateAutolock(minutes: number) {
    setAutolock(minutes)
    try { localStorage.setItem(AUTOLOCK_KEY, String(minutes)) } catch { /* нет хранилища */ }
  }

  const [dark, setDark] = useState(() => {
    try {
      const saved = localStorage.getItem('rrcki-mail.theme')
      if (saved) return saved === 'dark'
    } catch { /* нет хранилища */ }
    return window.matchMedia?.('(prefers-color-scheme: dark)').matches ?? false
  })
  // Тема запоминается — окна писем открываются в той же теме.
  useEffect(() => {
    try { localStorage.setItem('rrcki-mail.theme', dark ? 'dark' : 'light') } catch { /* нет хранилища */ }
  }, [dark])
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
  /** Отмеченные письма (Ctrl/Shift+клик); пусто — действия над открытым письмом. */
  const [checked, setChecked] = useState<Set<string>>(new Set())
  const anchorKey = useRef('')
  const [movePicker, setMovePicker] = useState<MessageSummary[] | null>(null)
  const [confirmDelete, setConfirmDelete] = useState<MessageSummary[] | null>(null)
  const [showFilters, setShowFilters] = useState(false)
  const [filterSeed, setFilterSeed] = useState<MessageSummary | null>(null)
  const [dragInfo, setDragInfo] = useState<{ x: number; y: number; count: number; over: string | null } | null>(null)
  const msgDrag = useRef<{ x0: number; y0: number; items: MessageSummary[]; active: boolean } | null>(null)
  const suppressRowClick = useRef(false)
  /** Контекстное меню письма: где открыто и к каким письмам относится. */
  const [rowMenu, setRowMenu] = useState<{ x: number; y: number; items: MessageSummary[] } | null>(null)
  /** Было открыто отдельное окно письма — при возврате в главное окно список обновляется. */
  const windowOpened = useRef(false)
  const [detail, setDetail] = useState<MessageDetail | null>(null)
  const [detailLoading, setDetailLoading] = useState(false)
  const [detailError, setDetailError] = useState('')

  const [query, setQuery] = useState('')
  const queryRef = useRef('')
  queryRef.current = query
  const [filter, setFilter] = useState<MailFilter>('Все')
  /** Поиск по всей почте: результаты из локального индекса (и, по кнопке, с сервера). */
  const [localResults, setLocalResults] = useState<MessageSummary[] | null>(null)
  const [serverResults, setServerResults] = useState<MessageSummary[] | null>(null)
  const [searchBusy, setSearchBusy] = useState(false)
  const serverSearched = serverResults !== null
  /** Найденное в индексе и (по кнопке) на сервере — без повторов, новые сверху. */
  const searchResults = useMemo(() => {
    if (localResults === null && serverResults === null) return null
    const seen = new Set<string>()
    return [...(localResults ?? []), ...(serverResults ?? [])]
      .filter((m) => { const k = messageKey(m); if (seen.has(k)) return false; seen.add(k); return true })
      .sort((a, b) => (Date.parse(b.date ?? '') || 0) - (Date.parse(a.date ?? '') || 0))
  }, [localResults, serverResults])
  function setSearchResults(update: (items: MessageSummary[] | null) => MessageSummary[] | null) {
    setLocalResults(update)
    setServerResults(update)
  }
  const searching = searchResults !== null
  /** Письма текущего списка: найденные или письма папки. */
  const pool = searchResults ?? messages
  function clearSearch() {
    setQuery('')
    setLocalResults(null)
    setServerResults(null)
  }

  const [diagText, setDiagText] = useState<string | null>(null)
  const [diagBusy, setDiagBusy] = useState(false)

  const [module, setModule] = useState<'mail' | 'calendar' | 'contacts' | 'deck' | 'chat' | 'files'>('mail')
  const [ncStatus, setNcStatus] = useState<NcStatus | null>(null)
  const [chatUnread, setChatUnread] = useState(0)
  const [draft, setDraft] = useState<Draft | null>(null)
  const [signature, setSignature] = useState('')
  // Категории (общие с веб-почтой), фильтр по категории, цепочки писем.
  const [categories, setCategories] = useState<Category[]>([])
  useEffect(() => { api.categoriesGet().then(setCategories).catch(() => undefined) }, [])
  const [catFilter, setCatFilter] = useState('')
  const [threadsOn, setThreadsOn] = useState(() => { try { return localStorage.getItem('rrcki-mail.threads') !== 'off' } catch { return true } })
  useEffect(() => { try { localStorage.setItem('rrcki-mail.threads', threadsOn ? 'on' : 'off') } catch { /* нет хранилища */ } }, [threadsOn])
  const [openThreads, setOpenThreads] = useState<Set<string>>(new Set())
  const [popMenu, setPopMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null)
  const [duePick, setDuePick] = useState<MessageSummary[] | null>(null)
  const [snoozePick, setSnoozePick] = useState<MessageSummary[] | null>(null)

  // Напоминания по флажкам со сроком: проверка раз в 2 минуты (и при запуске).
  useEffect(() => {
    let stopped = false
    const KEY = 'rrcki-mail.reminded'
    async function check() {
      try {
        const list = await api.dueList()
        if (stopped) return
        let done: string[] = []
        try { done = JSON.parse(localStorage.getItem(KEY) ?? '[]') as string[] } catch { /* нет хранилища */ }
        const now = Date.now()
        for (const m of list) {
          const due = dueOf(m)
          const id = `${m.msgId ?? `${m.folder}/${m.uid}`}|${due?.getTime()}`
          if (!due || due.getTime() > now || done.includes(id)) continue
          done.push(id)
          const text = `${senderLabel(m)}: ${m.subject || '(без темы)'}`
          if (appInFront()) setNotice(`Напоминание по флажку — ${text}`)
          else notify('Напоминание по флажку', text)
        }
        try { localStorage.setItem(KEY, JSON.stringify(done.slice(-500))) } catch { /* нет хранилища */ }
      } catch { /* попробуем позже */ }
    }
    let timer = 0
    const loop = () => { timer = window.setTimeout(() => { void check().finally(() => { if (!stopped) loop() }) }, timer === 0 ? 5000 : pollSecs.current < 60 ? 10_000 : 120_000) }
    loop()
    return () => { stopped = true; window.clearTimeout(timer) }
  }, [])
  // Непрочитанные во «Входящих» — на значке в трее и на панели задач.
  const inboxUnseen = folders.find((f) => f.role === 'inbox')?.unseen ?? 0
  useEffect(() => { void api.setUnread(inboxUnseen).catch(() => undefined) }, [inboxUnseen])
  useEffect(() => () => { void api.setUnread(0).catch(() => undefined) }, [])
  const outbox = useOutbox((d) => startDraft(d), (t) => setNotice(t))
  const [scheduled, setScheduled] = useState<Scheduled[]>([])
  const [showScheduled, setShowScheduled] = useState(false)
  const loadScheduled = useCallback(() => { api.scheduledList().then(setScheduled).catch(() => undefined) }, [])
  useEffect(() => {
    loadScheduled()
    const t = window.setInterval(loadScheduled, 60_000)
    return () => window.clearInterval(t)
  }, [loadScheduled])
  useEffect(() => { api.signatureGet().then(setSignature).catch(() => undefined) }, [])
  composingRef.current = draft !== null
  const [contacts, setContacts] = useState<Contact[]>([])
  const contactsRequested = useRef(false)
  const [profile, setProfile] = useState<Profile | null>(null)
  const [profileForm, setProfileForm] = useState({ displayName: '', smtpHost: '', smtpPort: '465', davUrl: '' })
  const [confirmPassword, setConfirmPassword] = useState('')
  const serversChanged = Boolean(profile) && (profileForm.smtpHost.trim() !== profile?.smtpHost || profileForm.smtpPort.trim() !== String(profile?.smtpPort) || profileForm.davUrl.trim() !== profile?.davUrl)
  const [profileBusy, setProfileBusy] = useState(false)

  useEffect(() => {
    api.ncStatus().then(setNcStatus).catch(() => undefined)
  }, [])

  // Уведомления исчезают сами, чтобы не закрывать поле ввода.
  useEffect(() => {
    if (!notice) return
    const t = setTimeout(() => setNotice(''), 6000)
    return () => clearTimeout(t)
  }, [notice])

  // Уведомления: настройки, интервал проверки, текущий раздел/папка для фоновых проверок.
  const [prefs, setPrefs] = useState<NotifyPrefs>(loadPrefs)
  const prefsRef = useRef(prefs)
  prefsRef.current = prefs
  const pollSecs = useRef(60)
  const moduleRef = useRef(module)
  moduleRef.current = module
  useEffect(() => { api.notifyPollSecs().then((s) => { pollSecs.current = Math.max(5, s) }).catch(() => undefined) }, [])

  function updatePrefs(patch: Partial<NotifyPrefs>) {
    setPrefs((p) => {
      const next = { ...p, ...patch }
      savePrefs(next)
      return next
    })
  }

  // Чат: счётчик непрочитанного и уведомления о новых сообщениях (в любом разделе).
  const roomSeen = useRef(new Map<string, { unread: number; activity: number }>())
  useEffect(() => {
    if (!ncStatus?.connected) return
    let stopped = false
    let timer: ReturnType<typeof setTimeout> | undefined
    async function tick() {
      try {
        const rooms = await api.talkRooms()
        if (stopped) return
        if (moduleRef.current !== 'chat') setChatUnread(rooms.reduce((n, x) => n + x.unread, 0))
        const first = roomSeen.current.size === 0
        for (const r of rooms) {
          const prev = roomSeen.current.get(r.token)
          roomSeen.current.set(r.token, { unread: r.unread, activity: r.lastActivity })
          if (first || !prev || r.unread <= prev.unread) continue
          if (!prefsRef.current.chat) continue
          const text = prefsRef.current.preview ? (r.lastMessage ?? 'Новое сообщение') : 'Новое сообщение'
          if (!appInFront()) notify(`Чат: ${r.name}`, text)
          else if (moduleRef.current !== 'chat') setNotice(`Чат «${r.name}»: ${text}`)
        }
      } catch { /* сеть — попробуем в следующий раз */ }
      finally {
        if (!stopped) timer = setTimeout(tick, Math.min(pollSecs.current, 30) * 1000)
      }
    }
    void tick()
    return () => { stopped = true; if (timer) clearTimeout(timer) }
  }, [ncStatus?.connected])

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
  function folderName(id: string): string {
    const f = folders.find((x) => x.id === id)
    return f ? folderLabel(f) : id
  }

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
    // Сначала — папки с компьютера (сразу после запуска и разблокировки), затем с сервера.
    api.cachedFolders().then((cached) => {
      if (cached?.length) setFolders((cur) => (cur.length ? cur : cached))
    }).catch(() => undefined)
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
      setChecked(new Set())
      setSelectedKey('')
      setDetail(null)
    }
    if (!keepSelection) {
      // Письма с компьютера показываются сразу, пока идёт запрос к серверу.
      const cached = await api.cachedMessages(folder, null, PAGE_SIZE).catch(() => null)
      if (cached && request === listRequest.current) {
        setMessages(cached.items)
        setTotal(cached.total)
        setHasMore(cached.hasMore)
      }
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

  // Новая почта: «Входящие» (свои и, по настройке, подключённых ящиков) проверяются по UID —
  // надёжно; остальные папки (куда раскладывают фильтры) — по росту числа непрочитанных.
  const folderIdRef = useRef(folderId)
  folderIdRef.current = folderId
  const unseenSeen = useRef(new Map<string, number>())
  const lastUid = useRef(new Map<string, number>())
  useEffect(() => {
    let stopped = false
    let timer: ReturnType<typeof setTimeout> | undefined
    const skip = new Set(['trash', 'junk', 'sent', 'drafts'])
    function announce(f: Folder, all: Folder[], items: MessageSummary[], count: number) {
      const p = prefsRef.current
      const where = f.role === 'inbox' && !sharedMailboxOf(all, f) ? '' : ` («${f.path}»)`
      if (appInFront()) {
        const m = items[0]
        setNotice(count > 1 ? `Новых писем: ${count}${where}` : `Новое письмо от ${senderLabel(m)}${p.preview && m.subject ? `: ${m.subject}` : ''}`)
        return
      }
      if (count > 3) {
        notify(`Новых писем: ${count}${where}`, items.slice(0, 3).map((m) => senderLabel(m)).join(', '))
        return
      }
      for (const m of items.slice(0, 3)) {
        notify(`Новое письмо — ${senderLabel(m)}${where}`, p.preview ? (m.subject || '(без темы)') : 'Откройте Atom Mail, чтобы прочитать')
      }
    }
    async function tick() {
      try {
        const list = await api.listFolders()
        if (stopped) return
        setFolders(list)
        const p = prefsRef.current
        for (const f of list) {
          if (!f.selectable || skip.has(f.role)) continue
          const shared = sharedMailboxOf(list, f) !== null
          if (shared && !p.shared) { unseenSeen.current.set(f.id, f.unseen); continue }
          const prev = unseenSeen.current.get(f.id)
          unseenSeen.current.set(f.id, f.unseen)
          const known = lastUid.current.get(f.id)
          const isInbox = f.role === 'inbox'
          // Не «Входящие»: смотрим только если непрочитанных стало больше.
          if (!isInbox && (prev === undefined || f.unseen <= prev)) continue
          if (isInbox && known === undefined) {
            // Первая проверка — запоминаем последний UID, без уведомлений.
            const base = await api.pollNewMail(f.id, 0, 0)
            lastUid.current.set(f.id, base.lastUid)
            continue
          }
          const after = known ?? 0
          const growth = prev === undefined ? 0 : Math.max(0, f.unseen - prev)
          const limit = after ? 5 : Math.min(growth, 5)
          if (limit === 0) continue
          const r = await api.pollNewMail(f.id, after, limit)
          lastUid.current.set(f.id, Math.max(after, r.lastUid))
          if (r.items.length === 0) continue
          if (f.id === folderIdRef.current) void loadFirstPage(f.id, true)
          if (p.mail) announce(f, list, r.items, Math.max(r.items.length, growth))
        }
      } catch { /* сеть — попробуем в следующий раз */ }
      finally {
        if (!stopped) timer = setTimeout(tick, pollSecs.current * 1000)
      }
    }
    timer = setTimeout(tick, 3000)
    return () => { stopped = true; if (timer) clearTimeout(timer) }
  }, [loadFirstPage])

  // Действия в окне письма (ответ, флажок, архив, удаление) видны в списке после возврата.
  useEffect(() => {
    let last = 0
    function onFocus() {
      if (!windowOpened.current || Date.now() - last < 3000) return
      last = Date.now()
      void loadFolders()
      void loadFirstPage(folderId, true)
    }
    window.addEventListener('focus', onFocus)
    return () => window.removeEventListener('focus', onFocus)
  }, [folderId, loadFirstPage, loadFolders])

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
    setSearchResults((items) => items && items.map((m) => (messageKey(m) === key ? { ...m, ...patch } : m)))
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

  async function toggleFlag(message: MessageSummary, flag: 'seen' | 'flagged' | 'important') {
    const key = messageKey(message)
    const field = flag === 'seen' ? 'unread' : flag
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

  /** Категория у писем (по папкам): поставить или снять. */
  async function setCategory(items: MessageSummary[], cat: Category, want: boolean) {
    const target = items.filter((m) => (m.keywords ?? []).some((k) => k.toLowerCase() === cat.keyword.toLowerCase()) !== want)
    for (const m of target) {
      const kw = (m.keywords ?? []).filter((k) => k.toLowerCase() !== cat.keyword.toLowerCase())
      patchMessage(messageKey(m), { keywords: want ? [...kw, cat.keyword] : kw })
    }
    for (const g of byFolder(target)) {
      try {
        await api.setKeywords(g[0].folder, g.map((m) => m.uid), cat.keyword, want)
      } catch (error) {
        setNotice(`Не удалось изменить категорию: ${errorText(error)}`)
      }
    }
  }

  /** Флажок со сроком (или без срока) — напоминание придёт в срок. */
  async function setDueFor(items: MessageSummary[], due: Date | null) {
    for (const m of items) {
      patchMessage(messageKey(m), { flagged: true, keywords: withDue(m.keywords ?? [], due) })
      try {
        await api.setDue(m.folder, m.uid, due ? due.toISOString() : null)
      } catch (error) {
        setNotice(`Не удалось поставить срок: ${errorText(error)}`)
      }
    }
    if (due) setNotice(`Флажок до ${dueLabel(due)} — напомню в срок.`)
  }

  function dueMenuItems(items: MessageSummary[]): MenuItem[] {
    return [
      ...duePresets().map((p) => ({ label: `К исполнению: ${p.label.toLowerCase()}`, icon: <Flag size={15} />, hint: dueLabel(p.at), onSelect: () => void setDueFor(items, p.at) })),
      { label: 'Выбрать срок…', icon: <CalendarClock size={15} />, onSelect: () => setDuePick(items) },
      { label: 'Флажок без срока', icon: <Flag size={15} />, onSelect: () => void setDueFor(items, null) },
    ]
  }

  function categoryMenuItems(items: MessageSummary[]): MenuItem[] {
    const cats = userCategories(categories)
    return [
      ...cats.map((c) => {
        const has = items.every((m) => (m.keywords ?? []).some((k) => k.toLowerCase() === c.keyword.toLowerCase()))
        return { label: c.name, icon: <span className="cat-dot" style={{ background: c.color }}>{has ? '✓' : ''}</span>, onSelect: () => void setCategory(items, c, !has) }
      }),
      ...(cats.length ? [{ separator: true as const }] : []),
      { label: 'Настроить категории…', icon: <Tag size={15} />, onSelect: () => setShowSettings(true) },
    ]
  }

  /** «Это спам» / «Не спам» (в папке «Спам»). */
  async function markSpam(items: MessageSummary[], spam: boolean) {
    for (const g of byFolder(items)) {
      try {
        const target = await api.markSpam(g[0].folder, g.map((m) => m.uid), spam)
        dropFromList(g, target)
        setNotice(spam ? `${g.length === 1 ? 'Письмо перенесено' : `Писем: ${g.length} —`} в «Спам».` : 'Письмо возвращено во «Входящие».')
      } catch (e) {
        setNotice(`Не удалось: ${errorText(e)}`)
      }
    }
  }

  /** «Отложить»: письмо вернётся во «Входящие» в выбранное время. */
  async function snoozeItems(items: MessageSummary[], until: Date) {
    for (const g of byFolder(items)) {
      try {
        const target = await api.snooze(g[0].folder, g.map((m) => m.uid), until.toISOString())
        dropFromList(g, target)
        setNotice(`Отложено до ${dueLabel(until)} — письмо вернётся во «Входящие».`)
      } catch (e) {
        setNotice(`Не удалось отложить: ${errorText(e)}`)
      }
    }
  }

  async function unsnoozeItems(items: MessageSummary[]) {
    for (const g of byFolder(items)) {
      try {
        await api.unsnooze(g[0].folder, g.map((m) => m.uid))
        dropFromList(g, 'INBOX')
        setNotice('Письмо возвращено во «Входящие».')
      } catch (e) {
        setNotice(`Не удалось: ${errorText(e)}`)
      }
    }
  }

  function snoozeMenuItems(items: MessageSummary[]): MenuItem[] {
    return [
      ...snoozePresets().map((p) => ({ label: p.label, icon: <AlarmClock size={15} />, hint: dueLabel(p.at), onSelect: () => void snoozeItems(items, p.at) })),
      { label: 'Выбрать дату и время…', icon: <CalendarClock size={15} />, onSelect: () => setSnoozePick(items) },
    ]
  }

  async function doUnsubscribe(d: MessageDetail) {
    try {
      const r = await api.unsubscribe(d.folder, d.uid)
      if (r.done) setNotice(`Вы отписаны от рассылки (${r.how}).`)
      else if (r.openUrl) { setNotice('Отписка — на сайте рассылки: открываю в браузере.'); void api.openLink(r.openUrl).catch(() => undefined) }
    } catch (e) {
      setNotice(`Не удалось отписаться: ${errorText(e)}`)
    }
  }

  function openPop(e: React.MouseEvent, items: MenuItem[]) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
    setPopMenu({ x: r.left, y: r.bottom + 4, items })
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

  /** Новое письмо со ссылками на файлы Nextcloud. */
  function sendLinks(links: { name: string; url: string }[]) {
    const lines = links.map((l) => `${l.name}: ${l.url}`).join('\n')
    startDraft({
      mode: 'new', to: '', cc: '', inReplyTo: null, references: null,
      subject: links.length === 1 ? links[0].name : `Файлы (${links.length})`,
      body: `\n\n${links.length === 1 ? 'Ссылка на файл' : 'Ссылки на файлы'}:\n${lines}` + withSignature('', signature),
    })
  }

  async function saveToNextcloud(index: number, name: string) {
    if (!detail) return
    try {
      const path = await api.ncSaveMailAttachment(detail.folder, detail.uid, index, NC_MAIL_DIR)
      setNotice(`«${name}» сохранено в Nextcloud: ${path}`)
    } catch (error) {
      setNotice(`Не удалось сохранить в Nextcloud: ${errorText(error)}`)
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
    const q = searching ? '' : query.trim().toLowerCase()
    return pool.filter((m) => {
      const filterMatch =
        filter === 'Все' ? true :
        filter === 'Непрочитанные' ? m.unread :
        filter === 'Важные' ? m.important :
        filter === 'С флажком' ? m.flagged :
        m.hasAttachments
      if (!filterMatch) return false
      if (catFilter && !(m.keywords ?? []).some((k) => k.toLowerCase() === catFilter.toLowerCase())) return false
      if (!q) return true
      return `${m.fromName} ${m.fromAddress} ${m.subject} ${m.to.join(' ')}`.toLowerCase().includes(q)
    })
  }, [pool, searching, filter, query, catFilter])

  /** Строки списка: цепочки свёрнуты в одну строку (новое письмо сверху), раскрываются стрелкой. */
  const listRows = useMemo(() => {
    if (!threadsOn || searching) return visibleMessages.map((m) => ({ m, count: 1, child: false, thread: '' }))
    const keys = threadKeys(visibleMessages)
    const groups = new Map<string, MessageSummary[]>()
    for (const m of visibleMessages) {
      const k = keys.get(m) ?? messageKey(m)
      groups.set(k, [...(groups.get(k) ?? []), m])
    }
    const rows: { m: MessageSummary; count: number; child: boolean; thread: string }[] = []
    for (const [k, items] of groups) {
      rows.push({ m: items[0], count: items.length, child: false, thread: k })
      if (items.length > 1 && openThreads.has(k)) for (const c of items.slice(1)) rows.push({ m: c, count: 1, child: true, thread: k })
    }
    return rows
  }, [visibleMessages, threadsOn, searching, openThreads])

  const selected = pool.find((m) => messageKey(m) === selectedKey)

  // Поиск по всей почте (индекс на компьютере), с задержкой после ввода.
  useEffect(() => {
    const q = query.trim()
    setServerResults(null)
    if (q.length < 2) { setLocalResults(null); return }
    let cancelled = false
    const timer = window.setTimeout(() => {
      api.searchMail(q).then((found) => { if (!cancelled) setLocalResults(found) }).catch(() => { if (!cancelled) setLocalResults([]) })
    }, 200)
    return () => { cancelled = true; window.clearTimeout(timer) }
  }, [query])

  async function searchOnServer() {
    const q = query.trim()
    if (q.length < 2) return
    setSearchBusy(true)
    try {
      const found = await api.searchServer(q)
      if (queryRef.current.trim() === q) setServerResults(found)
    } catch (error) {
      setNotice(`Поиск на сервере не удался: ${errorText(error)}`)
    } finally {
      setSearchBusy(false)
    }
  }

  /** Письма, над которыми выполняется действие: отмеченные, иначе открытое. */
  function actionTargets(): MessageSummary[] {
    if (checked.size > 0) return pool.filter((m) => checked.has(messageKey(m)))
    return selected ? [selected] : []
  }

  function onRowClick(e: React.MouseEvent, m: MessageSummary) {
    if (suppressRowClick.current) return
    const key = messageKey(m)
    if (e.ctrlKey || e.metaKey) {
      setChecked((prev) => {
        const next = new Set(prev.size === 0 && selectedKey ? [selectedKey] : prev)
        if (next.has(key)) next.delete(key)
        else next.add(key)
        return next
      })
      anchorKey.current = key
      return
    }
    if (e.shiftKey && (anchorKey.current || selectedKey)) {
      const keys = visibleMessages.map(messageKey)
      const a = keys.indexOf(anchorKey.current || selectedKey)
      const b = keys.indexOf(key)
      if (a >= 0 && b >= 0) {
        setChecked(new Set(keys.slice(Math.min(a, b), Math.max(a, b) + 1)))
        return
      }
    }
    anchorKey.current = key
    setChecked(new Set())
    void openMessage(m)
  }

  /** Убирает письма из списка и поправляет счётчики папок. */
  function dropFromList(items: MessageSummary[], target: string | null) {
    const keys = new Set(items.map(messageKey))
    const unread = items.filter((m) => m.unread).length
    const source = items[0]?.folder
    setMessages((list) => list.filter((m) => !keys.has(messageKey(m))))
    setSearchResults((list) => list && list.filter((m) => !keys.has(messageKey(m))))
    setTotal((t) => Math.max(0, t - items.length))
    setChecked(new Set())
    setFolders((list) => list.map((f) => {
      if (f.id === source) return { ...f, total: Math.max(0, f.total - items.length), unseen: Math.max(0, f.unseen - unread) }
      if (target && f.id === target) return { ...f, total: f.total + items.length, unseen: f.unseen + unread }
      return f
    }))
    if (keys.has(selectedKey)) {
      // Открываем следующее письмо, как в обычных почтовых клиентах.
      const order = visibleMessages.map(messageKey)
      const idx = order.indexOf(selectedKey)
      const next = order.slice(idx + 1).concat(order.slice(0, idx).reverse()).find((k) => !keys.has(k))
      const nextMsg = next ? pool.find((m) => messageKey(m) === next) : undefined
      setSelectedKey('')
      setDetail(null)
      if (nextMsg) void openMessage(nextMsg)
    }
  }

  /** Письма из найденных могут быть из разных папок — операции выполняются по папкам. */
  function byFolder(items: MessageSummary[]): MessageSummary[][] {
    const groups = new Map<string, MessageSummary[]>()
    for (const m of items) groups.set(m.folder, [...(groups.get(m.folder) ?? []), m])
    return [...groups.values()]
  }

  async function moveTo(items: MessageSummary[], target: string) {
    const groups = byFolder(items)
    if (groups.length > 1) {
      for (const g of groups) await moveTo(g, target)
      return
    }
    if (items.length === 0) return
    const source = items[0].folder
    if (target === source) return
    const dest = folders.find((f) => f.id === target)
    try {
      await api.moveMessages(source, items.map((m) => m.uid), target)
      dropFromList(items, target)
      setNotice(`${items.length === 1 ? 'Письмо перемещено' : `Перемещено писем: ${items.length}`} в «${dest ? folderLabel(dest) : target}».`)
    } catch (error) {
      setNotice(`Не удалось переместить: ${errorText(error)}`)
    }
  }

  /** Удаление: в корзину своего ящика; из корзины (или без корзины) — безвозвратно, с подтверждением. */
  function requestDelete(items: MessageSummary[], force = false) {
    if (items.length === 0) return
    const trash = trashFor(folders, items[0].folder)
    if (!trash || trash.id === items[0].folder || force) setConfirmDelete(items)
    else void deleteNow(items, false)
  }

  async function deleteNow(items: MessageSummary[], permanentAsked: boolean) {
    setConfirmDelete(null)
    const groups = byFolder(items)
    if (groups.length > 1) {
      for (const g of groups) await deleteNow(g, permanentAsked)
      return
    }
    const source = items[0].folder
    const trash = trashFor(folders, source)
    const toTrash = !permanentAsked && trash && trash.id !== source ? trash.id : null
    try {
      const permanent = await api.deleteMessages(source, items.map((m) => m.uid), toTrash)
      dropFromList(items, permanent ? null : toTrash)
      const n = items.length === 1 ? 'Письмо' : `Писем: ${items.length},`
      setNotice(permanent ? `${n} удалено безвозвратно.` : `${n} перемещено в корзину.`)
    } catch (error) {
      setNotice(`Не удалось удалить: ${errorText(error)}`)
    }
  }

  // Del — удалить, Shift+Del — безвозвратно; Ctrl+A — отметить все.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (module !== 'mail' || draft || showSettings || showFilters || movePicker || confirmDelete) return
      if (document.querySelector('.viewer-backdrop')) return
      const t = e.target as HTMLElement | null
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable)) return
      const ctrl = e.ctrlKey || e.metaKey
      if (ctrl && e.key.toLowerCase() === 'r') {
        // Ctrl+R в WebView перезагрузил бы окно — здесь это «Ответить», как в Outlook.
        e.preventDefault()
        if (detail) openComposer(e.shiftKey ? 'replyAll' : 'reply')
      } else if (ctrl && e.shiftKey && e.key.toLowerCase() === 'f') {
        e.preventDefault()
        if (detail) openComposer('forward')
      } else if (e.key === 'Enter' && selected && checked.size === 0 && (t === document.body || t?.classList.contains('message-row'))) {
        e.preventDefault()
        openInWindow(selected)
      } else if (e.key === 'Delete') {
        e.preventDefault()
        requestDelete(actionTargets(), e.shiftKey)
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'a') {
        e.preventDefault()
        setChecked(new Set(visibleMessages.map(messageKey)))
      } else if (e.key === 'Escape' && checked.size > 0) {
        setChecked(new Set())
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

  // Перетаскивание писем на папку (указатель, а не HTML5 DnD — одинаково в WebView2 и WebKitGTK).
  useEffect(() => {
    function folderAt(x: number, y: number): string | null {
      const el = document.elementFromPoint(x, y)?.closest<HTMLElement>('[data-folder-id]')
      return el?.dataset.folderId ?? null
    }
    function onMove(e: PointerEvent) {
      const d = msgDrag.current
      if (!d) return
      if (!d.active) {
        if (Math.abs(e.clientX - d.x0) + Math.abs(e.clientY - d.y0) < 8) return
        d.active = true
      }
      setDragInfo({ x: e.clientX, y: e.clientY, count: d.items.length, over: folderAt(e.clientX, e.clientY) })
    }
    function onUp(e: PointerEvent) {
      const d = msgDrag.current
      msgDrag.current = null
      if (!d?.active) return
      setDragInfo(null)
      suppressRowClick.current = true
      setTimeout(() => { suppressRowClick.current = false }, 0)
      const target = folderAt(e.clientX, e.clientY)
      if (target) void moveTo(d.items, target)
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    return () => { window.removeEventListener('pointermove', onMove); window.removeEventListener('pointerup', onUp) }
  })

  function startMessageDrag(e: React.PointerEvent, m: MessageSummary) {
    if (e.button !== 0 || e.ctrlKey || e.shiftKey || e.metaKey) return
    const key = messageKey(m)
    const items = checked.has(key) ? pool.filter((x) => checked.has(messageKey(x))) : [m]
    msgDrag.current = { x0: e.clientX, y0: e.clientY, items, active: false }
  }

  function startDraft(next: Draft) {
    setDraft(next)
    // Контакты для подсказок адресов — один раз за сессию, ошибки не мешают письму.
    if (!contactsRequested.current) {
      contactsRequested.current = true
      api.listContacts().then((r) => setContacts(r.contacts)).catch(() => undefined)
    }
  }

  function openComposer(mode: 'new' | 'reply' | 'replyAll' | 'forward') {
    if (mode === 'new' || !detail) {
      startDraft({ mode: 'new', to: '', cc: '', subject: '', body: withSignature('', signature), inReplyTo: null, references: null })
      return
    }
    startDraft(buildDraft(mode, detail, account.username, signature))
  }

  function composeTo(to: string) {
    startDraft({ mode: 'new', to: `${to}, `, cc: '', subject: '', body: withSignature('', signature), inReplyTo: null, references: null })
  }

  /** Ответ/пересылка из меню — письмо может быть ещё не открыто: загружаем его. */
  async function composeFor(m: MessageSummary, mode: 'reply' | 'replyAll' | 'forward') {
    try {
      const full = detail && messageKey(detail) === messageKey(m) ? detail : await api.getMessage(m.folder, m.uid)
      startDraft(buildDraft(mode, full, account.username, signature))
    } catch (error) {
      setNotice(`Не удалось открыть письмо: ${errorText(error)}`)
    }
  }

  function openRowMenu(e: React.MouseEvent, m: MessageSummary) {
    e.preventDefault()
    const key = messageKey(m)
    // Как в Outlook: правый щелчок по отмеченным — меню для всех отмеченных, иначе — для этого письма.
    const items = checked.has(key) && checked.size > 1 ? pool.filter((x) => checked.has(messageKey(x))) : [m]
    if (!checked.has(key) && checked.size > 0) setChecked(new Set())
    setRowMenu({ x: e.clientX, y: e.clientY, items })
  }

  function rowMenuItems(items: MessageSummary[]): MenuItem[] {
    const one = items.length === 1 ? items[0] : null
    const allUnread = items.every((m) => m.unread)
    const allFlagged = items.every((m) => m.flagged)
    const allImportant = items.every((m) => m.important)
    const each = (flag: 'seen' | 'flagged' | 'important', want: boolean) => () => {
      for (const m of items) {
        const has = flag === 'seen' ? !m.unread : m[flag]
        if (has !== want) void toggleFlag(m, flag)
      }
    }
    const list: MenuItem[] = []
    if (one) {
      list.push(
        { label: 'Открыть', icon: <Mail size={15} />, onSelect: () => void openMessage(one) },
        { label: 'Открыть в отдельном окне', icon: <ExternalLink size={15} />, hint: 'Enter', onSelect: () => openInWindow(one) },
        { separator: true },
        { label: 'Ответить', icon: <Reply size={15} />, hint: 'Ctrl+R', onSelect: () => void composeFor(one, 'reply') },
        { label: 'Ответить всем', icon: <ReplyAll size={15} />, hint: 'Ctrl+Shift+R', onSelect: () => void composeFor(one, 'replyAll') },
        { label: 'Переслать', icon: <Forward size={15} />, hint: 'Ctrl+Shift+F', onSelect: () => void composeFor(one, 'forward') },
        { separator: true },
      )
    }
    list.push(
      allUnread
        ? { label: 'Отметить как прочитанное', icon: <MailOpen size={15} />, onSelect: each('seen', true) }
        : { label: 'Отметить как непрочитанное', icon: <Mail size={15} />, onSelect: each('seen', false) },
      allImportant
        ? { label: 'Снять «Важное»', icon: <CircleAlert size={15} />, onSelect: each('important', false) }
        : { label: 'Пометить как важное', icon: <CircleAlert size={15} />, onSelect: each('important', true) },
      allFlagged
        ? { label: 'Снять флажок', icon: <Flag size={15} />, onSelect: each('flagged', false) }
        : { label: 'Поставить флажок', icon: <Flag size={15} />, onSelect: each('flagged', true) },
      { label: 'Флажок со сроком…', icon: <CalendarClock size={15} />, onSelect: () => setDuePick(items) },
      items.some((m) => snoozedUntil(m.keywords))
        ? { label: 'Вернуть во «Входящие» сейчас', icon: <AlarmClock size={15} />, onSelect: () => void unsnoozeItems(items) }
        : { label: 'Отложить до завтра утром', icon: <AlarmClock size={15} />, onSelect: () => void snoozeItems(items, snoozePresets().find((p) => p.label === 'Завтра утром')!.at) },
      { label: 'Отложить до…', icon: <AlarmClock size={15} />, onSelect: () => setSnoozePick(items) },
      { separator: true },
      ...userCategories(categories).map((c) => {
        const has = items.every((m) => (m.keywords ?? []).some((k) => k.toLowerCase() === c.keyword.toLowerCase()))
        return { label: `Категория «${c.name}»`, icon: <span className="cat-dot" style={{ background: c.color }}>{has ? '✓' : ''}</span>, onSelect: () => void setCategory(items, c, !has) }
      }),
      { separator: true },
      { label: 'В архив', icon: <Archive size={15} />, onSelect: () => void archive(items) },
      { label: 'Переместить в папку…', icon: <FolderInput size={15} />, onSelect: () => setMovePicker(items) },
    )
    if (one) list.push({ label: 'Создать фильтр по отправителю…', icon: <ListFilter size={15} />, onSelect: () => { setFilterSeed(one); setShowFilters(true) } })
    const inJunk = folders.find((f) => f.id === items[0]?.folder)?.role === 'junk'
    list.push(inJunk
      ? { label: 'Не спам', icon: <ShieldCheck size={15} />, onSelect: () => void markSpam(items, false) }
      : { label: 'Это спам', icon: <ShieldAlert size={15} />, onSelect: () => void markSpam(items, true) })
    if (one) list.push({ label: 'Печать…', icon: <Printer size={15} />, onSelect: () => { void (detail && messageKey(detail) === messageKey(one) ? Promise.resolve(detail) : api.getMessage(one.folder, one.uid)).then(printMessage).catch((e) => setNotice(errorText(e))) } })
    list.push(
      { separator: true },
      { label: items.length > 1 ? `Удалить (${items.length})` : 'Удалить', icon: <Trash2 size={15} />, hint: 'Del', danger: true, onSelect: () => requestDelete(items) },
    )
    return list
  }

  async function archive(items: MessageSummary[]) {
    const groups = byFolder(items)
    if (groups.length > 1) {
      for (const g of groups) await archive(g)
      return
    }
    if (items.length === 0) return
    try {
      const target = await ensureArchive(folders, items[0].folder)
      if (target.id === items[0].folder) {
        setNotice('Письмо уже в архиве.')
        return
      }
      if (target.created) await loadFolders()
      await moveTo(items, target.id)
    } catch (error) {
      setNotice(`Не удалось архивировать: ${errorText(error)}`)
    }
  }

  function openInWindow(m: MessageSummary) {
    api.openMessageWindow(m.folder, m.uid, m.subject || '(без темы)')
      .then(() => { windowOpened.current = true })
      .catch((error) => setNotice(`Не удалось открыть окно: ${errorText(error)}`))
  }

  async function saveProfile() {
    const port = Number(profileForm.smtpPort)
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
      setNotice('Некорректный порт SMTP')
      return
    }
    setProfileBusy(true)
    try {
      const p = await api.updateProfile({ ...profileForm, smtpPort: port, password: serversChanged ? confirmPassword : null })
      setProfile(p)
      setConfirmPassword('')
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
          <img className="brand-logo" src={kurchatovLogo} alt="НИЦ «Курчатовский институт»" />
          <span>Atom Mail</span>
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
          {totpOn && <button className="text-button" onClick={() => { setShowAccountMenu(false); onLock() }}><Lock size={15} /> Заблокировать <kbd>Ctrl+L</kbd></button>}
          <button className="text-button" onClick={() => { setShowAccountMenu(false); onLogout() }}><LogOut size={15} /> Выйти</button>
        </div>}
      </header>

      <div className="workspace">
        <nav className="module-rail" aria-label="Модули">
          <button className={`module ${module === 'mail' ? 'active' : ''}`} onClick={() => setModule('mail')}><Mail /><span>Почта</span></button>
          <button className={`module ${module === 'calendar' ? 'active' : ''}`} onClick={() => setModule('calendar')}><CalendarDays /><span>Календарь</span></button>
          <button className={`module ${module === 'contacts' ? 'active' : ''}`} onClick={() => setModule('contacts')}><ContactRound /><span>Контакты</span></button>
          <button className={`module ${module === 'deck' ? 'active' : ''}`} onClick={() => setModule('deck')}><FileText /><span>Задачи</span></button>
          <button className={`module ${module === 'chat' ? 'active' : ''}`} onClick={() => setModule('chat')}><MessageCircle /><span>Чат</span>{chatUnread > 0 && <b className="rail-badge">{chatUnread}</b>}</button>
          <button className={`module ${module === 'files' ? 'active' : ''}`} onClick={() => setModule('files')}><Cloud /><span>Файлы</span></button>
          <div className="rail-spacer" />
        </nav>

        {module === 'calendar' && <CalendarView onNotice={setNotice} />}
        {(module === 'deck' || module === 'chat' || module === 'files') && ncStatus && !ncStatus.connected && (
          <div className="module-view nc-gate"><NcConnect status={ncStatus} onChange={setNcStatus} /></div>
        )}
        {module === 'deck' && ncStatus?.connected && <DeckView onNotice={setNotice} />}
        {module === 'chat' && ncStatus?.connected && <TalkView onUnread={setChatUnread} onNotice={setNotice} />}
        {module === 'files' && ncStatus?.connected && <FilesView onNotice={setNotice} onSendLinks={sendLinks} />}
        {module === 'contacts' && (
          <ContactsView
            onLoaded={(list) => { setContacts(list); contactsRequested.current = true }}
            onNotice={setNotice}
            onWrite={(address) => composeTo(address)}
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
                  className={`folder-item ${folderId === f.id ? 'selected' : ''} ${f.total === 0 ? 'folder-empty' : ''} ${dragInfo?.over === f.id && f.id !== folderId ? 'drop-target' : ''}`}
                  data-folder-id={f.id}
                  style={indent}
                  title={`${f.path} — ${f.total} ${pluralLetters(f.total)}${f.unseen ? `, непрочитанных ${f.unseen}` : ''}`}
                  onClick={() => { clearSearch(); setFolderId(f.id) }}
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
          {scheduled.length > 0 && <button className="scheduled-link" onClick={() => { loadScheduled(); setShowScheduled(true) }}>
            <CalendarClock size={16} /> Запланировано: {scheduled.length}
          </button>}
          <div className="sidebar-footer">
            <div className="secure-label"><ShieldCheck size={15} /> Подключено по TLS</div>
            <span>{account.host}</span>
          </div>
        </aside>}

        <main className="message-column">
          <div className="list-toolbar">
            <button className="icon-button soft" aria-label="Показать папки" onClick={() => setFoldersOpen(!foldersOpen)}><PanelLeftClose size={18} /></button>
            <label className="search-box">
              <Search size={17} />
              <input
                ref={searchRef}
                value={query}
                aria-label="Поиск писем"
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Escape' && query) { e.preventDefault(); e.stopPropagation(); clearSearch() } }}
                placeholder="Поиск по почте"
              />
              {query
                ? <button type="button" className="search-clear" aria-label="Очистить поиск" title="Очистить (Esc)" onClick={() => { clearSearch(); searchRef.current?.focus() }}><X size={15} /></button>
                : <kbd>Ctrl K</kbd>}
            </label>
            <button
              className="icon-button soft refresh-button"
              aria-label="Обновить"
              title={loading ? 'Загрузка писем…' : 'Обновить'}
              disabled={loading}
              onClick={() => void refresh()}
            >
              <RefreshCw size={17} className={loading ? 'spin' : ''} />
            </button>
            <button className="icon-button soft" aria-label="Фильтры почты" title="Фильтры почты (правила сортировки)" onClick={() => { setFilterSeed(null); setShowFilters(true) }}>
              <ListFilter size={17} />
            </button>
          </div>
          <div className="filter-row">
            {(['Все', 'Непрочитанные', 'Важные', 'С флажком', 'С вложениями'] as MailFilter[]).map((item) => (
              <button key={item} className={`filter-chip ${filter === item ? 'active' : ''}`} onClick={() => setFilter(item)}>{item}</button>
            ))}
            {userCategories(categories).length > 0 && <select className={`filter-chip filter-select ${catFilter ? 'active' : ''}`} aria-label="Категория" value={catFilter} onChange={(e) => setCatFilter(e.target.value)}>
              <option value="">Все категории</option>
              {userCategories(categories).map((c) => <option key={c.keyword} value={c.keyword}>{c.name}</option>)}
            </select>}
            <button className={`filter-chip ${threadsOn ? 'active' : ''}`} aria-pressed={threadsOn} title="Группировать письма одной переписки" onClick={() => setThreadsOn(!threadsOn)}><MessagesSquare size={13} /> Цепочки</button>
          </div>
          {searching
            ? <div className="message-list-header search-header">
              <span>Найдено: {searchResults.length}{searchResults.length >= 300 ? '+' : ''}</span>
              {serverSearched
                ? <span>с учётом текста писем на сервере</span>
                : <button className="text-button" disabled={searchBusy} onClick={() => void searchOnServer()}>{searchBusy ? 'Поиск на сервере…' : 'Искать и в тексте старых писем'}</button>}
            </div>
            : <div className="message-list-header"><span>{listTitle}</span><span>{total} {pluralLetters(total)}</span></div>}
          {checked.size > 0 && <div className="bulk-bar">
            <span>Отмечено: {checked.size}</span>
            <button className="text-button" onClick={() => void archive(actionTargets())}><Archive size={15} /> В архив</button>
            <button className="text-button" onClick={() => setMovePicker(actionTargets())}><FolderInput size={15} /> Переместить</button>
            <button className="text-button danger-text" onClick={() => requestDelete(actionTargets())}><Trash2 size={15} /> Удалить</button>
            <button className="icon-button soft" aria-label="Снять отметку" onClick={() => setChecked(new Set())}><X size={15} /></button>
          </div>}
          <div className="message-list">
            {loading && messages.length === 0 && <div className="empty-state"><span>Загрузка писем с сервера…</span></div>}
            {listError && (
              <div className="empty-state" role="alert">
                <strong>Не удалось загрузить письма</strong>
                <span>{listError}</span>
                <button className="text-button" onClick={() => void loadFirstPage(folderId, true)}>Повторить</button>
              </div>
            )}
            {listRows.map(({ m, count, child, thread }) => {
              const key = messageKey(m)
              const sender = senderLabel(m)
              const due = dueOf(m)
              const cats = categoriesOf(m, categories)
              const open = openThreads.has(thread)
              return (
                <button
                  key={key}
                  className={`message-row ${child ? 'thread-child' : ''} ${selectedKey === key ? 'selected' : ''} ${checked.has(key) ? 'checked' : ''} ${m.unread ? 'unread' : ''} ${rowMenu?.items.some((x) => messageKey(x) === key) ? 'menu-target' : ''}`}
                  onClick={(e) => onRowClick(e, m)}
                  onDoubleClick={() => openInWindow(m)}
                  onContextMenu={(e) => openRowMenu(e, m)}
                  title="Двойной щелчок — открыть в отдельном окне"
                  onPointerDown={(e) => startMessageDrag(e, m)}
                  onDragStart={(e) => e.preventDefault()}
                >
                  <div className={`sender-avatar avatar-${m.uid % 5}`}>{initials(sender)}</div>
                  <div className="message-summary">
                    <div className="message-topline"><span className="sender-name">{sender}</span><time>{formatListDate(m.date)}</time></div>
                    <div className="message-subject">
                      {count > 1 && <span
                        className={`thread-count ${open ? 'open' : ''}`}
                        role="button"
                        aria-label={open ? 'Свернуть цепочку' : 'Развернуть цепочку'}
                        title={`Писем в цепочке: ${count}`}
                        onClick={(e) => { e.stopPropagation(); setOpenThreads((s) => { const n = new Set(s); if (n.has(thread)) n.delete(thread); else n.add(thread); return n }) }}
                        onPointerDown={(e) => e.stopPropagation()}
                      ><ChevronRight size={12} />{count}</span>}
                      {m.subject || '(без темы)'}
                    </div>
                    {searching && <div className="message-folder">{folderName(m.folder)}</div>}
                    {(cats.length > 0 || due || snoozedUntil(m.keywords)) && <div className="message-tags">
                      {snoozedUntil(m.keywords) && <span className="snooze-tag" title="Отложено"><AlarmClock size={11} /> до {dueLabel(snoozedUntil(m.keywords)!)}</span>}
                      {due && <span className={`due-tag ${due.getTime() < Date.now() ? 'overdue' : ''}`} title="Срок флажка"><Flag size={11} /> {dueLabel(due)}</span>}
                      {cats.map((c) => <span key={c.keyword} className="cat-tag" style={{ borderColor: c.color, color: c.color }}><span className="cat-dot" style={{ background: c.color }} />{c.name}</span>)}
                    </div>}
                    <div className="message-meta">
                      {m.hasAttachments && <Paperclip size={13} />}
                      {m.important && <CircleAlert size={13} className="important-mark" />}
                      {m.flagged && <Flag size={13} className="flag-mark" fill="currentColor" />}
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
                <strong>{searching || messages.length > 0 ? 'Ничего не найдено' : 'Папка пуста'}</strong>
                {searching && <button className="text-button" onClick={clearSearch}>Очистить поиск</button>}
                {!searching && messages.length === 0 && (currentFolder?.total ?? 0) > 0 && <>
                  <span>Сервер сообщает о {currentFolder?.total} письмах в этой папке, но список не получен.</span>
                  <button className="text-button" onClick={() => { setShowSettings(true); void runDiagnostics() }}>Открыть диагностику</button>
                </>}
                {!searching && messages.length > 0 && <>
                  <button className="text-button" onClick={() => { clearSearch(); setFilter('Все') }}>Сбросить фильтры</button>
                </>}
              </div>
            )}
            {hasMore && !listError && !searching && (
              <button className="text-button load-more" disabled={loadingMore} onClick={() => void loadMore()}>
                {loadingMore ? 'Загрузка…' : `Показать ещё (загружено ${messages.length} из ${total})`}
              </button>
            )}
          </div>
          <div className="list-footer">
            <span>{searching ? `${visibleMessages.length} найдено` : `${visibleMessages.length} из ${messages.length} загруженных`}</span>
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
              <button className="icon-button" aria-label="Ответить" title="Ответить" disabled={!detail} onClick={() => openComposer('reply')}><Reply size={18} /></button>
              <button className="icon-button" aria-label="Ответить всем" title="Ответить всем" disabled={!detail} onClick={() => openComposer('replyAll')}><ReplyAll size={18} /></button>
              <button className="icon-button" aria-label="Переслать" title="Переслать" disabled={!detail} onClick={() => openComposer('forward')}><Forward size={18} /></button>
              <span className="toolbar-sep" />
              <button className={`icon-button ${selected.important ? 'important-on' : ''}`} aria-label="Важное" aria-pressed={selected.important} title={selected.important ? 'Снять «Важное»' : 'Пометить как важное'} onClick={() => void toggleFlag(selected, 'important')}>
                <CircleAlert size={18} />
              </button>
              <button className={`icon-button ${selected.flagged ? 'flag-on' : ''}`} aria-label="Флажок" aria-pressed={selected.flagged} title={selected.flagged ? 'Снять флажок' : 'Поставить флажок (к исполнению)'} onClick={() => void toggleFlag(selected, 'flagged')}>
                <Flag size={18} fill={selected.flagged ? 'currentColor' : 'none'} />
              </button>
              <button className="icon-button" aria-label="Срок" title="Флажок со сроком (напоминание)" onClick={(e) => openPop(e, dueMenuItems([selected]))}><CalendarClock size={18} /></button>
              <button className="icon-button" aria-label="Категории" title="Категории" onClick={(e) => openPop(e, categoryMenuItems([selected]))}><Tag size={18} /></button>
              <button className="icon-button" aria-label="Отложить" title="Отложить (вернётся во «Входящие» в срок)" onClick={(e) => openPop(e, snoozeMenuItems([selected]))}><AlarmClock size={18} /></button>
              <button className="icon-button" aria-label="Непрочитанное" title={selected.unread ? 'Отметить прочитанным' : 'Отметить непрочитанным'} onClick={() => void toggleFlag(selected, 'seen')}>
                <Mail size={18} />
              </button>
              <span className="toolbar-sep" />
              <button className="icon-button" aria-label="Архивировать" title="В архив" onClick={() => void archive([selected])}><Archive size={18} /></button>
              <button className="icon-button" aria-label="Переместить в папку" title="Переместить в папку…" onClick={() => setMovePicker([selected])}>
                <FolderInput size={18} />
              </button>
              <button className="icon-button" aria-label="Создать фильтр" title="Создать фильтр по этому письму" onClick={() => { setFilterSeed(selected); setShowFilters(true) }}>
                <ListFilter size={18} />
              </button>
              <button className="icon-button danger-icon" aria-label="Удалить письмо" title="Удалить (Del)" onClick={() => requestDelete([selected])}>
                <Trash2 size={18} />
              </button>
              <span className="toolbar-sep" />
              {currentFolder?.role === 'junk'
                ? <button className="icon-button" aria-label="Не спам" title="Не спам — вернуть во «Входящие»" onClick={() => void markSpam([selected], false)}><ShieldCheck size={18} /></button>
                : <button className="icon-button" aria-label="Это спам" title="Это спам" onClick={() => void markSpam([selected], true)}><ShieldAlert size={18} /></button>}
              <button className="icon-button" aria-label="Печать" title="Печать или сохранение в PDF" disabled={!detail} onClick={() => detail && printMessage(detail)}><Printer size={18} /></button>
              <button className="icon-button" aria-label="Открыть в новом окне" title="Открыть в отдельном окне" onClick={() => openInWindow(selected)}><ExternalLink size={18} /></button>
            </>}
            <button className="icon-button soft" aria-label="Скрыть чтение" onClick={() => setReaderOpen(false)}><PanelRightClose size={18} /></button>
          </div>
          {detail ? <div className="reader-content">
            <div className="reader-title-row"><h1>{detail.subject || '(без темы)'}</h1></div>
            <div className="sender-block">
              <div className="sender-avatar sender-avatar-large">{initials(senderLabel(detail))}</div>
              <div className="sender-details">
                <span className="sender-line"><AddressLine strong entries={[detail.fromName ? `${detail.fromName} <${detail.fromAddress}>` : detail.fromAddress]} onCompose={composeTo} onSearch={setQuery} onNotice={setNotice} /></span>
                {detail.to.length > 0 && <span>Кому: <AddressLine entries={detail.to} onCompose={composeTo} onSearch={setQuery} onNotice={setNotice} /></span>}
                {detail.cc.length > 0 && <span>Копия: <AddressLine entries={detail.cc} onCompose={composeTo} onSearch={setQuery} onNotice={setNotice} /></span>}
                {detail.unsubscribe && <span className="unsub-bar">Это рассылка · <button className="text-button" onClick={() => void doUnsubscribe(detail)}>Отписаться</button></span>}
                <small>{formatFullDate(detail.date)}</small>
              </div>
            </div>
            {detail.attachments.length > 0 && <AttachmentList
              key={`${detail.folder}/${detail.uid}`}
              folder={detail.folder}
              uid={detail.uid}
              attachments={detail.attachments}
              onSave={(i) => void saveAttachment(i)}
              onCloud={ncStatus?.connected ? (i, name) => void saveToNextcloud(i, name) : undefined}
              cloudTitle={`Сохранить в Nextcloud (${NC_MAIL_DIR})`}
              onEdit={ncStatus?.connected ? (i, name) => { setNotice(`Открываю «${name}»…`); api.ncEditAttachment(detail.folder, detail.uid, i, NC_MAIL_DIR).then((r) => setNotice(editNotice(r, name))).catch((e) => setNotice(`Не удалось открыть в редакторе: ${errorText(e)}`)) } : undefined}
            />}
            <MdnBar detail={detail} me={account.username} onNotice={setNotice} onDone={() => { setDetail((d) => d && { ...d, keywords: [...d.keywords, '$MDNSent'] }); patchMessage(messageKey(detail), { keywords: [...detail.keywords, '$MDNSent'] }) }} />
            <MessageBody detail={detail} onNotice={setNotice} />
            <div className="reader-actions">
              <button className="primary-button" onClick={() => openComposer('reply')}><Reply size={16} /> Ответить</button>
              <button className="secondary-button" onClick={() => openComposer('replyAll')}><ReplyAll size={16} /> Ответить всем</button>
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
          ncConnected={Boolean(ncStatus?.connected)}
          onClose={() => setDraft(null)}
          onSent={(text) => {
            setDraft(null)
            setNotice(text)
            loadScheduled()
            if ((draft.mode === 'reply' || draft.mode === 'replyAll') && detail) patchMessage(messageKey(detail), { answered: true })
          }}
          onQueued={(id, snapshot, delay) => {
            setDraft(null)
            outbox.queued(id, snapshot, delay)
            if ((draft.mode === 'reply' || draft.mode === 'replyAll') && detail) patchMessage(messageKey(detail), { answered: true })
          }}
        />
      )}

      {outbox.element}
      {popMenu && <ContextMenu x={popMenu.x} y={popMenu.y} items={popMenu.items} onClose={() => setPopMenu(null)} />}
      {snoozePick && <DuePicker title="Отложить письмо" hint="В это время письмо вернётся во «Входящие» непрочитанным" action="Отложить" onClose={() => setSnoozePick(null)} onPick={(d) => { const items = snoozePick; setSnoozePick(null); void snoozeItems(items, d) }} />}
      {duePick && <DuePicker onClose={() => setDuePick(null)} onPick={(d) => { const items = duePick; setDuePick(null); void setDueFor(items, d) }} />}
      {showScheduled && <ScheduledMail items={scheduled} onClose={() => setShowScheduled(false)} onChanged={loadScheduled} onNotice={setNotice} />}
      {rowMenu && <ContextMenu x={rowMenu.x} y={rowMenu.y} items={rowMenuItems(rowMenu.items)} onClose={() => setRowMenu(null)} />}
      {dragInfo && <div className="mail-drag-ghost" style={{ left: dragInfo.x + 12, top: dragInfo.y + 8 }}>
        <Mail size={14} /> {dragInfo.count === 1 ? '1 письмо' : `${dragInfo.count} ${pluralLetters(dragInfo.count)}`}
      </div>}
      {movePicker && <FolderPicker
        folders={folders}
        exclude={movePicker[0]?.folder}
        title={movePicker.length === 1 ? 'Переместить письмо' : `Переместить письма (${movePicker.length})`}
        label={folderLabel}
        onClose={() => setMovePicker(null)}
        onPick={(id) => { const items = movePicker; setMovePicker(null); void moveTo(items, id) }}
      />}
      {confirmDelete && <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) setConfirmDelete(null) }}>
        <div className="settings-modal confirm-modal" role="alertdialog" aria-label="Подтверждение удаления">
          <strong>Удалить безвозвратно?</strong>
          <p>{confirmDelete.length === 1 ? `Письмо «${confirmDelete[0].subject || '(без темы)'}»` : `Писем: ${confirmDelete.length}`} будет удалено без возможности восстановления.</p>
          <div className="settings-actions">
            <button className="secondary-button" onClick={() => setConfirmDelete(null)}>Отмена</button>
            <button className="primary-button danger-button" autoFocus onClick={() => void deleteNow(confirmDelete, true)}><Trash2 size={15} /> Удалить</button>
          </div>
        </div>
      </div>}
      {showFilters && <MailFilters folders={folders} seed={filterSeed} onClose={() => setShowFilters(false)} onNotice={setNotice} label={folderLabel} />}
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
            {serversChanged && <label className="login-field"><span>Пароль почты — для смены адресов серверов</span><input type="password" aria-label="Пароль для подтверждения" autoComplete="current-password" value={confirmPassword} onChange={(e) => setConfirmPassword(e.target.value)} /></label>}
            <div className="settings-actions"><button className="primary-button" disabled={profileBusy || (serversChanged && !confirmPassword)} onClick={() => void saveProfile()}>Сохранить</button></div>
          </div>
          {ncStatus && <div className="settings-section"><strong>Nextcloud (Deck, Talk)</strong><NcConnect status={ncStatus} onChange={setNcStatus} compact /></div>}
          <div className="settings-section notify-settings">
            <strong>Уведомления</strong>
            <label><input type="checkbox" checked={prefs.mail} onChange={(e) => updatePrefs({ mail: e.target.checked })} /> О новых письмах</label>
            <label><input type="checkbox" checked={prefs.shared} disabled={!prefs.mail} onChange={(e) => updatePrefs({ shared: e.target.checked })} /> Включая подключённые (общие) ящики</label>
            <label><input type="checkbox" checked={prefs.chat} onChange={(e) => updatePrefs({ chat: e.target.checked })} /> О сообщениях в чате</label>
            <label><input type="checkbox" checked={prefs.preview} onChange={(e) => updatePrefs({ preview: e.target.checked })} /> Показывать тему письма и текст сообщения</label>
            <button className="text-button" onClick={() => notify('Atom Mail', 'Так выглядят уведомления о новой почте и сообщениях')}>Показать пример уведомления</button>
          </div>
          <SignatureSettings signature={signature} onSaved={(text, msg) => { setSignature(text); setNotice(msg) }} />
          <SendingSettings />
          <CategoriesSettings categories={categories} onSaved={(list, msg) => { setCategories(list); setNotice(msg) }} />
          <DesktopSettings onNotice={setNotice} />
          <OfflineSettings onNotice={setNotice} />
          <TransferSettings folders={folders} label={folderLabel} onNotice={setNotice} onImported={() => void loadFolders()} />
          <TotpSettings onChange={setTotpOn} onLock={() => { setShowSettings(false); onLock() }} autolock={autolock} onAutolock={updateAutolock} />
          <div className="settings-note"><ShieldCheck size={18} /><span>Пароль хранится только в системном хранилище (Windows Credential Manager / GNOME Keyring) и не записывается в файлы настроек. Для SMTP, календаря и контактов используются те же логин и пароль.</span></div>
          <div className="settings-actions">
            <button className="secondary-button" onClick={() => { setShowSettings(false); setFilterSeed(null); setShowFilters(true) }}><ListFilter size={16} /> Фильтры почты</button>
            <button className="secondary-button" disabled={diagBusy} onClick={() => void runDiagnostics()}><Stethoscope size={16} /> Диагностика</button>
            <button className="secondary-button" onClick={() => window.dispatchEvent(new Event(CHECK_UPDATE_EVENT))}><Download size={16} /> Проверить обновления</button>
            <button className="secondary-button" onClick={() => { setShowSettings(false); onLogout() }}><LogOut size={16} /> Выйти и забыть пароль</button>
          </div>
          <About />
          {diagText !== null && <div className="diag-block">
            <div className="diag-head"><span>Отчёт: IMAP по текущей папке, SMTP, CalDAV/CardDAV (пароль не включается)</span><button className="text-button" onClick={() => void copyDiagnostics()}><Copy size={14} /> Скопировать</button></div>
            <textarea className="diag-text" readOnly value={diagText} />
          </div>}
        </div>
      </div>}
    </div>
  )
}
