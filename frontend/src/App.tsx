import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  Archive, CalendarDays, ChevronDown, ChevronLeft, ChevronRight, Circle, Cloud, ContactRound, Download,
  FilePenLine, FileText, Folder as FolderIcon, FolderOpen, Forward, Inbox, Layers, LogOut, Mail, Bookmark, BookmarkPlus, Zap,
  MessageCircle, Moon, MoreHorizontal, Paperclip, PanelLeftClose, PanelRightClose, PanelRightOpen, RefreshCw,
  Reply, Search, Send, Settings, ShieldCheck, CalendarClock, Tag, MessagesSquare, AlarmClock, ShieldAlert, Printer, SquarePen, SquareKanban, Stethoscope, Sun, Trash2, Users, X, Copy, FolderInput, ListFilter, ReplyAll, Flag, CircleAlert, ExternalLink, MailOpen, Handshake, CloudOff,
} from 'lucide-react'

import {
  api, editNotice, errorText, formatFullDate, formatListDate, messageKey, NC_MAIL_DIR, PAGE_SIZE, TYPING_PAUSE_MS } from './api'
import type {
  Account, Contact, Draft, PickedFile, SignatureSet, Folder, FolderRole, MailFilter, MessageDetail, MessageSummary, Scheduled, Category, Settings as Profile, UnifiedFilters, UnifiedRole, UnifiedItem, SavedView,
} from './types'
import About from './About'
import kurchatovLogo from './assets/kurchatov-logo.png'
import Composer from './Composer'
import { CHECK_UPDATE_EVENT } from './UpdateNotice'
import DeckView from './DeckView'
import TalkView from './TalkView'
import NcConnect from './NcConnect'
import FilesView from './FilesView'
import type { CalEvent, NcStatus } from './types'
import ContactsView from './ContactsView'
import CounterpartyView from './CounterpartyView'
import { OfflineBanner, QueuePanel, useConnection } from './OfflineQueue'
import CalendarView from './CalendarView'
import MailFilters from './MailFilters'
import ContextMenu from './ContextMenu'
import GlobalSearch, { type Found } from './GlobalSearch'
import TaskFromMail from './TaskFromMail'
import type { MenuItem } from './ContextMenu'
import { AttachmentList } from './AttachmentViewer'
import MessageBody from './MessageBody'
import AddressLine from './AddressLine'
import MdnBar from './MdnBar'
import DuePicker from './DuePicker'
import { fileToBase64, printMessage, snoozePresets, snoozedUntil } from './mailTools'
import { categoriesOf, dueLabel, dueOf, duePresets, threadKeys, userCategories, withDue } from './mailExtras'
import ScheduledMail from './ScheduledMail'
import { useOutbox } from './sending'
import { CategoriesSettings, DesktopSettings, MailboxesSettings, OfflineSettings, SendingSettings, SignaturesSettings } from './MailSettings'
import { signatureById } from './drafting'
import { TransferSettings } from './Transfer'
import { AccountMenu, NO_FILTERS, UNIFIED_ROLES, UnifiedColumn, useAccounts } from './Accounts'
import { saveView, savedView, type ModuleName, type ViewContext } from './viewContext'
import TotpSettings, { AUTOLOCK_KEY, loadAutolock } from './TotpSettings'
import { buildDraft, ensureArchive, forwardDraft, withSignature } from './mailActions'
import FolderPicker, { sharedMailboxOf, trashFor } from './FolderPicker'
import { appInFront, loadPrefs, notify, savePrefs } from './notifications'
import type { NotifyPrefs } from './notifications'
import { SkeletonReader, SkeletonRows } from './Skeleton'
import Splitter from './Splitter'
import AppearanceSettings from './AppearanceSettings'
import { fontStack, setAppearance, useAppearance, zoomStep } from './appearance'

const ROLE_LABELS: Partial<Record<FolderRole, string>> = {
  inbox: 'Входящие',
  drafts: 'Черновики',
  sent: 'Отправленные',
  archive: 'Архив',
  junk: 'Спам',
  trash: 'Корзина',
}

/** Быстрые представления — готовые поиски по всем ящикам. */
const QUICK_VIEWS: SavedView[] = [
  { id: 'q-unread', name: 'Непрочитанные', query: 'непрочитанные', scope: 'all' },
  { id: 'q-important', name: 'Важные', query: 'важные', scope: 'all' },
  { id: 'q-files', name: 'С вложениями за неделю', query: 'есть:вложение после:7д', scope: 'all' },
]

const SEARCH_HELP = [
  'Поиск: слова ищутся в отправителе, получателях, теме, именах вложений и тексте писем.',
  'от:петрова  кому:иванов  тема:"план работ"  вложение:xlsx  есть:вложение',
  'непрочитанные  важные  флажок  отвеченные',
  'после:2026-10-01  до:01.12.2026  после:7д  после:вчера',
  'папка:входящие  ящик:noc  подразделение:ihep.ru',
].join('\n')

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

/** Отправленные и черновики: в списке важнее, кому письмо, а не от кого (от вас). */
function recipientsLabel(m: MessageSummary): string {
  const names = m.to.map((t) => {
    const lt = t.indexOf('<')
    const name = lt > 0 ? t.slice(0, lt).trim().replace(/^"|"$/g, '') : ''
    return name || (lt >= 0 ? t.slice(lt + 1).replace(/>.*$/, '') : t).trim()
  }).filter(Boolean)
  if (names.length === 0) return 'Кому: (без получателей)'
  return `Кому: ${names.slice(0, 3).join(', ')}${names.length > 3 ? ` и ещё ${names.length - 3}` : ''}`
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
  /** Переключиться на другую учётную запись (и открыть в ней письмо). */
  onSwitch: (key: string, open?: MessageSummary) => Promise<string | null>
  /** Письмо, которое открыть сразу (выбрано в «Все входящие» другой учётной записи). */
  initialOpen?: MessageSummary | null
}


/** Номер перехода к найденному: растёт всё время работы окна, разделы открывают каждый переход один раз. */
let focusSeq = 0
const nextFocus = () => ++focusSeq

export default function App({ account, initialNotice, onLogout, onLock, onSwitch, initialOpen }: Props) {
  // Контекст этой учётной записи с прошлого раза (переключение туда и обратно).
  const viewKey = `${account.username}@${account.host}`.toLowerCase()
  // Свой адрес: у Exchange логин может быть DOMAIN\\user.
  const myAddress = account.email || account.username
  const [saved] = useState(() => savedView(viewKey))
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

  const appearance = useAppearance()
  // Тема — из настроек «Вид» (светлая, тёмная, как в системе); кнопка в шапке переключает явно.
  const systemDark = () => window.matchMedia?.('(prefers-color-scheme: dark)').matches ?? false
  const [sysDark, setSysDark] = useState(systemDark)
  useEffect(() => {
    const m = window.matchMedia?.('(prefers-color-scheme: dark)')
    if (!m) return
    const on = () => setSysDark(m.matches)
    m.addEventListener?.('change', on)
    return () => m.removeEventListener?.('change', on)
  }, [])
  const dark = appearance.theme === 'system' ? sysDark : appearance.theme === 'dark'
  const setDark = (v: boolean) => setAppearance({ theme: v ? 'dark' : 'light' })
  // Тема запоминается — окна писем открываются в той же теме. Класс темы — и на
  // всей странице: то, что выводится поверх окна программы, тоже меняет цвета.
  useEffect(() => {
    try { localStorage.setItem('rrcki-mail.theme', dark ? 'dark' : 'light') } catch { /* нет хранилища */ }
    document.documentElement.classList.toggle('theme-dark', dark)
  }, [dark])
  const [foldersOpen, setFoldersOpen] = useState(true)
  const [readerOpen, setReaderOpen] = useState(true)
  const [showAccountMenu, setShowAccountMenu] = useState(false)
  const [showSettings, setShowSettings] = useState(false)
  const [notice, setNotice] = useState(initialNotice)

  const [folders, setFolders] = useState<Folder[]>([])
  const [folderId, setFolderId] = useState(saved?.folderId ?? 'INBOX')
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

  const [query, setQuery] = useState(saved?.query ?? '')
  const queryRef = useRef('')
  queryRef.current = query
  const [filter, setFilter] = useState<MailFilter>(saved?.filter ?? 'Все')
  /** Поиск по всей почте: результаты из локального индекса (и, по кнопке, с сервера). */
  const [localResults, setLocalResults] = useState<MessageSummary[] | null>(null)
  const [serverResults, setServerResults] = useState<MessageSummary[] | null>(null)
  /** Поиск в других учётных записях (область «Все ящики»). */
  const [searchScope, setSearchScope] = useState<'current' | 'all'>(saved?.searchScope ?? 'current')
  const [otherResults, setOtherResults] = useState<UnifiedItem[] | null>(null)
  const [otherErrors, setOtherErrors] = useState<string[]>([])
  const [othersBusy, setOthersBusy] = useState(false)
  const [searchBusy, setSearchBusy] = useState(false)
  const serverSearched = serverResults !== null
  /** Найденное в индексе и (по кнопке) на сервере — без повторов, новые сверху. */
  const searchResults = useMemo(() => {
    if (localResults === null && serverResults === null && otherResults === null) return null
    const seen = new Set<string>()
    return [...(localResults ?? []), ...(serverResults ?? []), ...(otherResults ?? [])]
      .filter((m) => { const k = messageKey(m); if (seen.has(k)) return false; seen.add(k); return true })
      .sort((a, b) => (Date.parse(b.date ?? '') || 0) - (Date.parse(a.date ?? '') || 0))
  }, [localResults, serverResults, otherResults])
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
    setOtherResults(null)
    setOtherErrors([])
  }

  const [diagText, setDiagText] = useState<string | null>(null)
  const [diagBusy, setDiagBusy] = useState(false)

  const [module, setModule] = useState<ModuleName>(saved?.module ?? 'mail')
  // История взаимодействия: открытая организация (домен адреса).
  const [cpDomain, setCpDomain] = useState<string | null>(null)
  const openHistory = useCallback((domain: string) => { setCpDomain(domain); setModule('counterparty') }, [])
  const [ncStatus, setNcStatus] = useState<NcStatus | null>(null)
  /** Общий поиск (Ctrl+K), задача из письма и переходы к найденному в других разделах. */
  const [globalOpen, setGlobalOpen] = useState(false)
  const [taskFor, setTaskFor] = useState<MessageSummary | null>(null)
  const [deckFocus, setDeckFocus] = useState<{ boardId: number; cardId: number; n: number } | null>(null)
  const [talkFocus, setTalkFocus] = useState<{ token: string; messageId: number | null; n: number } | null>(null)
  const [filesFocus, setFilesFocus] = useState<{ path: string; isDir: boolean; n: number } | null>(null)
  const [contactFocus, setContactFocus] = useState<{ contact: Contact; n: number } | null>(null)
  const [eventFocus, setEventFocus] = useState<{ e: CalEvent; n: number } | null>(null)
  const [chatUnread, setChatUnread] = useState(0)
  const [draft, setDraft] = useState<Draft | null>(null)
  const [composeMin, setComposeMin] = useState(false)
  // Подписи: для новых писем и для ответов/пересылки — свои (Настройки → Подписи).
  const [sigs, setSigs] = useState<SignatureSet | null>(null)
  const signature = signatureById(sigs?.items ?? [], sigs?.newId)?.text ?? ''
  const replySignature = signatureById(sigs?.items ?? [], sigs?.replyId)?.text ?? ''
  const [recipientPolicy, setRecipientPolicy] = useState<{ internalDomains: string[]; mode: 'smart' | 'always' | 'off' }>({ internalDomains: [], mode: 'smart' })
  /** Письмо, не сохранённое при сбое или закрытии программы, — предложить продолжить. */
  const [recovered, setRecovered] = useState<(Draft & { savedAt?: string; lostFiles?: string[] }) | null>(null)
  // Категории (общие с веб-почтой), фильтр по категории, цепочки писем.
  const [categories, setCategories] = useState<Category[]>([])
  useEffect(() => { api.categoriesGet().then(setCategories).catch(() => undefined) }, [])
  const [catFilter, setCatFilter] = useState(saved?.catFilter ?? '')
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
  useEffect(() => { api.signaturesGet().then(setSigs).catch(() => undefined) }, [])
  useEffect(() => {
    api.draftRecoveryGet().then((raw) => {
      if (!raw) return
      try {
        const d = JSON.parse(raw) as Draft & { savedAt?: string; files?: { name: string }[] }
        if (!d || typeof d !== 'object' || typeof d.to !== 'string') return
        // Выбранные файлы после перезапуска недоступны — их нужно приложить заново.
        setRecovered({ ...d, files: [], lostFiles: (d.files ?? []).map((f) => f.name) })
      } catch { /* испорченная копия — забыть */ void api.draftRecoverySet(null) }
    }).catch(() => undefined)
  }, [])
  composingRef.current = draft !== null
  const draftRef = useRef<Draft | null>(null)
  draftRef.current = draft
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
  const acc = useAccounts(() => pollSecs.current, () => prefsRef.current, setNotice)
  const switchAccount = (key: string, open?: MessageSummary) => { setNotice('Переключение учётной записи…'); void onSwitch(key, open).then((err) => { if (err) setNotice(err) }) }
  const [unified, setUnified] = useState<UnifiedRole | null>(saved?.unified ?? null)
  const [unifiedFilters, setUnifiedFilters] = useState<UnifiedFilters>(saved?.unifiedFilters ?? NO_FILTERS)
  const [unifiedOpen, setUnifiedOpen] = useState(() => { try { return localStorage.getItem('rrcki-mail.unified-open') !== 'off' } catch { return true } })
  useEffect(() => { try { localStorage.setItem('rrcki-mail.unified-open', unifiedOpen ? 'on' : 'off') } catch { /* необязательно */ } }, [unifiedOpen])
  // Контекст учётной записи: запоминается при каждом изменении, восстанавливается при возврате.
  const listRef = useRef<HTMLDivElement>(null)
  const scrollTop = useRef(saved?.scrollTop ?? 0)
  const lastOpened = useRef<MessageSummary | null>(saved?.selected ?? null)
  const viewBase = useRef<ViewContext>({ module: 'mail', folderId: 'INBOX', unified: null, unifiedFilters: NO_FILTERS, filter: 'Все', catFilter: '', query: '', searchScope: 'current', selected: null, scrollTop: 0 })
  const [notifyPolicy, setNotifyPolicy] = useState<'full' | 'sender' | 'none'>('full')
  useEffect(() => {
    api.getPolicy().then((p) => {
      setNotifyPolicy(p.notificationContent ?? 'full')
      setRecipientPolicy({ internalDomains: p.internalDomains ?? [], mode: p.externalWarning ?? 'smart' })
      if (p.policyError) setNotice(`Политика администратора не принята (${p.policyError}) — включены строгие ограничения: внешние картинки и текст уведомлений скрыты, пароль не запоминается. Сообщите администратору.`)
    }).catch(() => undefined)
  }, [])
  // Значок в трее: непрочитанные во «Входящих» всех учётных записей.
  const othersUnseen = Object.values(acc.unread).reduce<number>((a, n) => a + (n ?? 0), 0)
  useEffect(() => { void api.setUnread(inboxUnseen + othersUnseen).catch(() => undefined) }, [inboxUnseen, othersUnseen])

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

  // Непрочитанные под свёрнутым узлом (например, в подключённом ящике) — видно и без раскрытия.
  const nodeUnseen = useMemo(() => {
    const byId = new Map(folders.map((f) => [f.id, f]))
    const out = new Map<string, number>()
    for (const f of folders) {
      if (!f.selectable || !f.unseen) continue
      for (let p = f.parent; p; p = byId.get(p)?.parent ?? null) {
        if (byId.get(p)?.selectable === false) out.set(p, (out.get(p) ?? 0) + f.unseen)
      }
    }
    return out
  }, [folders])

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
        notify(`Новое письмо — ${senderLabel(m)}${where}`, p.preview ? (m.subject || '(без темы)') : 'Откройте Atom Workspace, чтобы прочитать')
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

  // Связь с сервером и очередь действий без сети; связь вернулась — обновить списки.
  const conn = useConnection(() => {
    void loadFolders()
    void loadFirstPage(folderIdRef.current, true)
    setNotice('Связь с почтовым сервером восстановлена.')
  })
  const [showQueue, setShowQueue] = useState(false)
  const [checkingConn, setCheckingConn] = useState(false)
  async function checkConnection() {
    setCheckingConn(true)
    try {
      const s = await api.offlineCheck()
      conn.setStatus(s)
      if (!s.online) setNotice('Связи с сервером пока нет.')
    } catch (error) {
      setNotice(errorText(error))
    } finally {
      setCheckingConn(false)
    }
  }
  const offline = conn.status ? !conn.status.online : Boolean(account.offline)

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
      if ((event.ctrlKey || event.metaKey) && !event.shiftKey && event.key.toLowerCase() === 'k') {
        event.preventDefault()
        setGlobalOpen(true)
      } else if ((event.ctrlKey || event.metaKey) && event.shiftKey && event.key.toLowerCase() === 'k') {
        // Ctrl+Shift+K — поиск в почте, как раньше по Ctrl+K.
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

  // Переключились из общей папки на другую учётную запись — сразу открыть выбранное
  // письмо; иначе — вернуть письмо, открытое в этой учётной записи в прошлый раз.
  const openedInitial = useRef(false)
  useEffect(() => {
    if (openedInitial.current) return
    openedInitial.current = true
    if (initialOpen) {
      setUnified(null)
      setFolderId(initialOpen.folder)
      void openMessage(initialOpen)
    } else if (saved?.selected) {
      void openMessage(saved.selected)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialOpen])

  useEffect(() => {
    saveView(viewKey, { module, folderId, unified, unifiedFilters, filter, catFilter, query, searchScope, selected: selectedKey ? lastOpened.current : null }, viewBase.current)
  }, [viewKey, module, folderId, unified, unifiedFilters, filter, catFilter, query, searchScope, selectedKey])
  // Прокрутка списка — при уходе из учётной записи (и при каждой смене папки).
  useEffect(() => () => saveView(viewKey, { scrollTop: scrollTop.current }, viewBase.current), [viewKey])
  // Вернуть прокрутку, когда список той же папки загрузился.
  const scrollRestored = useRef(false)
  useEffect(() => {
    if (scrollRestored.current || loading || messages.length === 0 || unified) return
    scrollRestored.current = true
    if (saved && saved.folderId === folderId && saved.scrollTop > 0 && listRef.current) listRef.current.scrollTop = saved.scrollTop
  }, [loading, messages.length, unified, folderId, saved])

  /** Открыть найденное (общий поиск, история организации) в его разделе. */
  function openFound(f: Found) {
    const n = nextFocus()
    switch (f.kind) {
      case 'mail':
        setModule('mail')
        clearSearch()
        setUnified(null)
        setFolderId(f.m.folder)
        void openMessage(f.m)
        break
      case 'file': setFilesFocus({ path: f.f.path, isDir: f.f.isDir, n }); setModule('files'); break
      case 'contact': setContactFocus({ contact: f.c, n }); setModule('contacts'); break
      case 'event': setEventFocus({ e: f.e, n }); setModule('calendar'); break
      case 'card': setDeckFocus({ boardId: f.h.boardId, cardId: f.h.card.id, n }); setModule('deck'); break
      case 'chat': setTalkFocus({ token: f.h.token, messageId: f.h.messageId, n }); setModule('chat'); break
      case 'org': openHistory(f.domain); break
    }
  }

  /** Письмо по Message-ID (ссылка из карточки Deck). */
  async function openMailById(msgId: string) {
    const m = await api.mailByMsgid(msgId).catch(() => null)
    if (m) openFound({ kind: 'mail', m })
    else setNotice('Письмо не найдено среди хранящихся на компьютере — найдите его поиском по теме.')
  }

  /** Письмо из «Черновиков» — дописать: тот же текст, получатели и вложения; при сохранении заменяется. */
  async function continueDraft(d: MessageDetail) {
    const ids = (d.references ?? '').split(/\s+/).filter(Boolean)
    let files: PickedFile[] = []
    if (d.attachments.length) {
      try { files = await api.forwardAttachments(d.folder, d.uid) } catch (e) { setNotice(`Вложения черновика не приложены: ${errorText(e)}`) }
    }
    startDraft({
      mode: 'new', to: d.to.join(', '), cc: d.cc.join(', '), subject: d.subject, body: d.bodyText, html: d.bodyHtml ?? undefined,
      inReplyTo: ids[ids.length - 1] ?? null, references: d.references, files, readReceipt: false,
      replace: { folder: d.folder, uid: d.uid }, signatureId: null,
      from: d.fromAddress && d.fromAddress.toLowerCase() !== myAddress.toLowerCase() ? d.fromAddress.toLowerCase() : null,
    })
  }

  async function openMessage(message: MessageSummary) {
    lastOpened.current = message
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

  /** Группа списка по дате письма. */
  function dateGroup(value: string | null): string {
    if (!value) return 'Без даты'
    const d = new Date(value)
    if (Number.isNaN(d.getTime())) return 'Без даты'
    const day = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime()
    const today = day(new Date())
    const diff = Math.round((today - day(d)) / 86400000)
    const weekStart = today - ((new Date().getDay() + 6) % 7) * 86400000
    if (diff <= 0) return 'Сегодня'
    if (diff === 1) return 'Вчера'
    if (day(d) >= weekStart) return 'На этой неделе'
    if (day(d) >= weekStart - 7 * 86400000) return 'На прошлой неделе'
    const now = new Date()
    if (d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth()) return 'В этом месяце'
    const m = d.toLocaleDateString('ru-RU', { month: 'long', ...(d.getFullYear() !== now.getFullYear() ? { year: 'numeric' } : {}) }).replace(' г.', '')
    return m.charAt(0).toUpperCase() + m.slice(1)
  }

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
    }, TYPING_PAUSE_MS)
    return () => { cancelled = true; window.clearTimeout(timer) }
  }, [query])

  // «Все ящики»: те же условия на серверах остальных учётных записей.
  useEffect(() => {
    const q = query.trim()
    setOtherResults(null)
    setOtherErrors([])
    if (q.length < 2 || searchScope !== 'all' || !acc.multi) return
    let cancelled = false
    const timer = window.setTimeout(() => {
      setOthersBusy(true)
      api.searchOthers(q)
        .then((r) => { if (!cancelled) { setOtherResults(r.items); setOtherErrors(r.errors) } })
        .catch((e) => { if (!cancelled) { setOtherResults([]); setOtherErrors([errorText(e)]) } })
        .finally(() => { if (!cancelled) setOthersBusy(false) })
    }, 600)
    return () => { cancelled = true; window.clearTimeout(timer) }
  }, [query, searchScope, acc.multi])

  /** Сохранённые поиски и быстрые представления. */
  const [views, setViews] = useState<SavedView[]>([])
  const [viewsOpen, setViewsOpen] = useState(() => { try { return localStorage.getItem('rrcki-mail.views-open') !== 'off' } catch { return true } })
  useEffect(() => { try { localStorage.setItem('rrcki-mail.views-open', viewsOpen ? 'on' : 'off') } catch { /* необязательно */ } }, [viewsOpen])
  useEffect(() => { api.viewsGet().then(setViews).catch(() => undefined) }, [])
  const [saveViewName, setSaveViewName] = useState<string | null>(null)
  function runView(v: SavedView) {
    setModule('mail')
    setUnified(null)
    setSearchScope(v.scope === 'all' && acc.multi ? 'all' : 'current')
    setQuery(v.query)
  }
  async function storeViews(next: SavedView[], done: string) {
    try {
      await api.viewsSave(next)
      setViews(next)
      setNotice(done)
    } catch (e) { setNotice(errorText(e)) }
  }
  function saveCurrentSearch() {
    const name = (saveViewName ?? '').trim()
    const q = query.trim()
    if (!name || q.length < 2) return
    const view: SavedView = { id: `v${Date.now().toString(36)}`, name: name.slice(0, 60), query: q, scope: searchScope }
    setSaveViewName(null)
    void storeViews([...views.filter((v) => v.name !== view.name), view], `Поиск «${view.name}» сохранён`)
  }
  const activeView = [...QUICK_VIEWS, ...views].find((v) => v.query === query.trim() && (v.scope === searchScope || !acc.multi))

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

  /** Письмо другой учётной записи (поиск по всем ящикам) открывается с переключением на неё. */
  function otherOf(m: MessageSummary): UnifiedItem | null {
    const u = m as UnifiedItem
    return u.account && !u.active ? u : null
  }

  function onRowClick(e: React.MouseEvent, m: MessageSummary) {
    if (suppressRowClick.current) return
    const other = otherOf(m)
    if (other) { switchAccount(other.account, other); return }
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

  // Масштаб интерфейса: Ctrl + / Ctrl − / Ctrl 0 (и Ctrl+колесо мыши) — в любом разделе.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (!(e.ctrlKey || e.metaKey) || e.altKey) return
      if (e.key === '+' || e.key === '=') { e.preventDefault(); zoomStep(1) }
      else if (e.key === '-' || e.key === '_') { e.preventDefault(); zoomStep(-1) }
      else if (e.key === '0') { e.preventDefault(); zoomStep(0) }
    }
    function onWheel(e: WheelEvent) {
      if (!(e.ctrlKey || e.metaKey) || e.deltaY === 0) return
      e.preventDefault()
      zoomStep(e.deltaY < 0 ? 1 : -1)
    }
    window.addEventListener('keydown', onKey)
    window.addEventListener('wheel', onWheel, { passive: false })
    return () => { window.removeEventListener('keydown', onKey); window.removeEventListener('wheel', onWheel) }
  }, [])

  // Del — удалить, Shift+Del — безвозвратно; Ctrl+A — отметить все.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (module !== 'mail' || (draft && !composeMin) || showSettings || showFilters || movePicker || confirmDelete) return
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

  // Файлы с компьютера, брошенные на окно почты (редактор закрыт), — новое письмо с ними.
  const [fileOver, setFileOver] = useState(false)
  const hasFiles = (e: React.DragEvent) => Array.from(e.dataTransfer.types).includes('Files')
  async function dropToNewMail(list: FileList) {
    setFileOver(false)
    const files: PickedFile[] = []
    const errors: string[] = []
    for (const file of Array.from(list)) {
      try {
        files.push(await api.attachData(file.name, await fileToBase64(file)))
      } catch (e) {
        errors.push(`${file.name}: ${errorText(e)}`)
      }
    }
    if (files.length > 0) {
      startDraft({ mode: 'new', to: '', cc: '', subject: files.length === 1 ? files[0].name : '', body: withSignature('', signature), inReplyTo: null, references: null, files })
    }
    if (errors.length > 0) setNotice(`Не приложено: ${errors.join('; ')}`)
  }

  /** Адрес подключённого ящика, которому принадлежит папка (`shared/<адрес>/…`), — отвечать от его имени. */
  function mailboxOf(folder: string | null | undefined): string | null {
    const path = folders.find((f) => f.id === folder)?.path ?? folder ?? ''
    const m = /^shared\/([^/]+@[^/]+)(\/|$)/i.exec(path)
    return m ? m[1].toLowerCase() : null
  }
  /** «Я» для «Ответить всем»: свой адрес и ящик, из которого отвечают. */
  const meFor = (folder: string | null | undefined) => [myAddress, mailboxOf(folder)].filter(Boolean).join(',')

  function startDraft(next: Draft, folder: string | null = folderId) {
    // Свёрнутое письмо не теряем: сначала его нужно дописать или закрыть.
    if (draftRef.current) { setComposeMin(false); setNotice('Уже открыто письмо — допишите или закройте его, затем начните новое.'); return }
    // Письмо из подключённого ящика (или новое, пока открыт его ящик) — от имени этого ящика.
    if (next.from === undefined) next = { ...next, from: mailboxOf(folder) }
    // Какая подпись вставлена — чтобы в окне письма можно было выбрать другую.
    setDraft(next.signatureId !== undefined ? next : { ...next, signatureId: (next.mode === 'new' ? sigs?.newId : sigs?.replyId) ?? null })
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
    if (mode === 'forward') {
      void forwardDraft(detail, myAddress, replySignature).then(({ draft, error }) => { startDraft(draft, detail.folder); if (error) setNotice(error) })
      return
    }
    startDraft(buildDraft(mode, detail, meFor(detail.folder), replySignature), detail.folder)
  }

  function composeTo(to: string) {
    startDraft({ mode: 'new', to: `${to}, `, cc: '', subject: '', body: withSignature('', signature), inReplyTo: null, references: null })
  }

  /** Ответ/пересылка из меню — письмо может быть ещё не открыто: загружаем его. */
  async function composeFor(m: MessageSummary, mode: 'reply' | 'replyAll' | 'forward') {
    try {
      const full = detail && messageKey(detail) === messageKey(m) ? detail : await api.getMessage(m.folder, m.uid)
      if (mode === 'forward') {
        const { draft, error } = await forwardDraft(full, myAddress, replySignature)
        startDraft(draft, full.folder)
        if (error) setNotice(error)
        return
      }
      startDraft(buildDraft(mode, full, meFor(full.folder), replySignature), full.folder)
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
        ...(ncStatus?.connected ? [{ label: 'Создать задачу…', icon: <SquareKanban size={15} />, onSelect: () => setTaskFor(one) }] : []),
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
    if (one) list.push({ label: 'Создать правило по отправителю…', icon: <ListFilter size={15} />, onSelect: () => { setFilterSeed(one); setShowFilters(true) } })
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
          <span>Atom Workspace</span>
        </div>
        <button className="account-switch" aria-label="Учётные записи" onClick={() => { if (!showAccountMenu) void acc.reload(); setShowAccountMenu(!showAccountMenu) }}>
          <div className="account-avatar"><Users size={20} /></div>
          <span><strong>{myAddress}</strong><small>{account.exchange ? 'Exchange' : 'IMAP'} · {account.host}{acc.multi ? ` · ещё ${acc.accounts.length - 1}` : ''}</small></span>
          {Object.values(acc.unread).some((n) => (n ?? 0) > 0) && <b className="rail-badge account-badge" title="Непрочитанные в других учётных записях">{Object.values(acc.unread).reduce<number>((a, n) => a + (n ?? 0), 0)}</b>}
          <ChevronDown size={17} />
        </button>
        <button className="primary-button compose-top" onClick={() => openComposer('new')}>
          <SquarePen size={17} /> Новое сообщение
        </button>
        <button className="global-search-open" onClick={() => setGlobalOpen(true)} aria-label="Общий поиск" title="Искать везде: письма, файлы, контакты, встречи, задачи, чат (Ctrl+K)">
          <Search size={16} /><span>Искать везде…</span><kbd>Ctrl+K</kbd>
        </button>
        <div className="topbar-actions">
          <span id="update-slot" className="update-slot" />
          <button className="icon-button" aria-label="Тема" onClick={() => setDark(!dark)}>{dark ? <Sun /> : <Moon />}</button>
          <button className="icon-button" aria-label="Настройки" onClick={() => setShowSettings(true)}><Settings /></button>
        </div>
        {showAccountMenu && <AccountMenu
          accounts={acc.accounts}
          unread={acc.unread}
          onSwitch={(key) => switchAccount(key)}
          onReload={() => void acc.reload()}
          onLock={totpOn ? onLock : null}
          onLogout={onLogout}
          onClose={() => setShowAccountMenu(false)}
          onNotice={setNotice}
        />}
      </header>

      <div
        className={`workspace ${fileOver ? 'file-over' : ''}`}
        onDragOver={(e) => { if (module === 'mail' && !draft && hasFiles(e)) { e.preventDefault(); e.dataTransfer.dropEffect = 'copy'; if (!fileOver) setFileOver(true) } }}
        onDragLeave={(e) => { if (fileOver && !e.currentTarget.contains(e.relatedTarget as Node | null)) setFileOver(false) }}
        onDrop={(e) => { if (module === 'mail' && !draft && e.dataTransfer.files.length > 0) { e.preventDefault(); void dropToNewMail(e.dataTransfer.files) } else setFileOver(false) }}
      >
        {fileOver && <div className="drop-hint" aria-hidden="true"><Paperclip size={22} /> Отпустите — файлы будут вложены в новое письмо</div>}
        <nav className="module-rail" aria-label="Модули">
          <button className={`module ${module === 'mail' ? 'active' : ''}`} onClick={() => setModule('mail')} title="Почта"><Mail /><span>Почта</span></button>
          <button className={`module ${module === 'calendar' ? 'active' : ''}`} onClick={() => setModule('calendar')} title="Календарь"><CalendarDays /><span>Календарь</span></button>
          <button className={`module ${module === 'contacts' ? 'active' : ''}`} onClick={() => setModule('contacts')} title="Контакты"><ContactRound /><span>Контакты</span></button>
          <button className={`module ${module === 'counterparty' ? 'active' : ''}`} onClick={() => setModule('counterparty')} title="История взаимодействия с организациями"><Handshake /><span>Контрагенты</span></button>
          <button className={`module ${module === 'deck' ? 'active' : ''}`} onClick={() => setModule('deck')} title="Задачи (Deck)"><FileText /><span>Задачи</span></button>
          <button className={`module ${module === 'chat' ? 'active' : ''}`} onClick={() => setModule('chat')} title="Чат"><MessageCircle /><span>Чат</span>{chatUnread > 0 && <b className="rail-badge">{chatUnread}</b>}</button>
          <button className={`module ${module === 'files' ? 'active' : ''}`} onClick={() => setModule('files')}><Cloud /><span>Файлы</span></button>
          <div className="rail-spacer" />
        </nav>

        {module === 'calendar' && <CalendarView onNotice={setNotice} focus={eventFocus} />}
        {module === 'counterparty' && <CounterpartyView domain={cpDomain} onDomain={setCpDomain} onCompose={composeTo} onNotice={setNotice} onOpen={openFound}
          ncConnected={Boolean(ncStatus?.connected)} onTask={ncStatus?.connected ? setTaskFor : undefined} />}
        {(module === 'deck' || module === 'chat' || module === 'files') && ncStatus && !ncStatus.connected && (
          <div className="module-view nc-gate"><NcConnect status={ncStatus} onChange={setNcStatus} /></div>
        )}
        {module === 'deck' && ncStatus?.connected && <DeckView onNotice={setNotice} focus={deckFocus} onOpenMail={openMailById} onOpenFolder={(path) => openFound({ kind: 'file', f: { path, name: '', isDir: true, size: 0, modified: '', mime: '', fileId: null, shared: false, favorite: false, canWrite: true } })} onOpenOrg={openHistory} />}
        {module === 'chat' && ncStatus?.connected && <TalkView onUnread={setChatUnread} onNotice={setNotice} focus={talkFocus} />}
        {module === 'files' && ncStatus?.connected && <FilesView onNotice={setNotice} onSendLinks={sendLinks} focus={filesFocus} />}
        {module === 'contacts' && (
          <ContactsView
            onLoaded={(list) => { setContacts(list); contactsRequested.current = true }}
            onNotice={setNotice}
            onWrite={(address) => composeTo(address)}
            focus={contactFocus}
          />
        )}
        {module === 'mail' && <>
        {foldersOpen && <aside className="folder-sidebar">
          <div className="sidebar-heading"><span>Папки</span><ChevronDown size={15} /></div>
          <div className="folder-list">
            {acc.multi && <>
              <button className="folder-item folder-node unified-group" aria-expanded={unifiedOpen} onClick={() => setUnifiedOpen(!unifiedOpen)}>
                <span className="folder-toggle">{unifiedOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}</span>
                <Layers size={17} /><span>Все ящики</span>
              </button>
              {unifiedOpen && UNIFIED_ROLES.map((r) => {
                const Icon = ROLE_ICONS[r.role]
                return (
                  <button key={r.role} className={`folder-item unified-item ${unified === r.role ? 'selected' : ''}`} data-role={r.role} style={{ paddingLeft: 18 }}
                    onClick={() => { clearSearch(); setUnified(r.role) }}>
                    <span className="folder-toggle" /><Icon size={17} /><span>{r.title}</span>
                  </button>
                )
              })}
              <div className="unified-separator" />
            </>}
            <button className="folder-item folder-node views-group" aria-expanded={viewsOpen} onClick={() => setViewsOpen(!viewsOpen)}>
              <span className="folder-toggle">{viewsOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}</span>
              <Search size={17} /><span>Поиски</span>
            </button>
            {viewsOpen && [...QUICK_VIEWS, ...views].map((v) => {
              const quick = v.id.startsWith('q-')
              return (
                <div key={v.id} className={`folder-item view-item ${activeView?.id === v.id ? 'selected' : ''}`} data-view={v.id} style={{ paddingLeft: 18 }} role="button" tabIndex={0}
                  title={`${v.query}${v.scope === 'all' ? ' — все ящики' : ''}`}
                  onClick={() => runView(v)} onKeyDown={(e) => { if (e.key === 'Enter') runView(v) }}>
                  <span className="folder-toggle" />{quick ? <Zap size={16} /> : <Bookmark size={16} />}<span>{v.name}</span>
                  {!quick && <button className="view-remove" aria-label={`Удалить поиск ${v.name}`} title="Удалить" onClick={(e) => { e.stopPropagation(); void storeViews(views.filter((x) => x.id !== v.id), `Поиск «${v.name}» удалён`) }}><X size={13} /></button>}
                </div>
              )
            })}
            <div className="unified-separator" />
            {folders.length === 0 && <SkeletonRows variant="folder" count={7} label="Загрузка папок…" />}
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
                  className={`folder-item ${!unified && folderId === f.id ? 'selected' : ''} ${f.total === 0 ? 'folder-empty' : ''} ${dragInfo?.over === f.id && f.id !== folderId ? 'drop-target' : ''}`}
                  data-folder-id={f.id}
                  style={indent}
                  title={`${f.path} — ${f.total} ${pluralLetters(f.total)}${f.unseen ? `, непрочитанных ${f.unseen}` : ''}`}
                  onClick={() => { clearSearch(); setUnified(null); setFolderId(f.id) }}
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
                  {!isOpen && (nodeUnseen.get(f.id) ?? 0) > 0 && <b className="folder-count muted" title="Непрочитанных в свёрнутых папках">{nodeUnseen.get(f.id)}</b>}
                </button>
              )
            })}
          </div>
          {scheduled.length > 0 && <button className="scheduled-link" onClick={() => { loadScheduled(); setShowScheduled(true) }}>
            <CalendarClock size={16} /> Запланировано: {scheduled.length}
          </button>}
          <div className="sidebar-footer">
            {offline
              ? <button className="secure-label offline text-button" onClick={() => setShowQueue(true)}><CloudOff size={15} /> Без сети{conn.status && conn.status.items.length > 0 ? ` · в очереди ${conn.status.items.length}` : ''}</button>
              : <div className="secure-label"><ShieldCheck size={15} /> Подключено по TLS</div>}
            <span>{account.host}</span>
          </div>
        </aside>}
        {foldersOpen && <Splitter id="mail-folders" min={150} max={420} label="Ширина списка папок" />}

        {unified ? <UnifiedColumn
          role={unified}
          accounts={acc.accounts}
          filters={unifiedFilters}
          onFilters={setUnifiedFilters}
          onNotice={setNotice}
          onOpen={(m) => {
            if (m.active) { void openMessage(m) } else { switchAccount(m.account, m) }
          }}
        /> : <main className="message-column">
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
            <button className="icon-button soft" aria-label="Правила обработки почты" title="Правила обработки почты: сортировка, пометки, уведомления, задачи" onClick={() => { setFilterSeed(null); setShowFilters(true) }}>
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
            ? <><div className="message-list-header search-header">
              <span>Найдено: {searchResults.length}{searchResults.length >= 300 ? '+' : ''}{othersBusy ? ' · ищу в других ящиках…' : ''}</span>
              {serverSearched
                ? <span>с учётом текста писем на сервере</span>
                : <button className="text-button" disabled={searchBusy} onClick={() => void searchOnServer()}>{searchBusy ? 'Поиск на сервере…' : 'Искать и в тексте старых писем'}</button>}
            </div>
            <div className="search-tools">
              {acc.multi && <div className="search-scope" role="group" aria-label="Где искать">
                <button className={`filter-chip ${searchScope === 'current' ? 'active' : ''}`} aria-pressed={searchScope === 'current'} onClick={() => setSearchScope('current')}>Этот ящик</button>
                <button className={`filter-chip ${searchScope === 'all' ? 'active' : ''}`} aria-pressed={searchScope === 'all'} onClick={() => setSearchScope('all')}>Все ящики</button>
              </div>}
              {activeView
                ? <span className="search-saved"><Bookmark size={13} /> {activeView.name}</span>
                : saveViewName === null
                  ? <button className="text-button" onClick={() => setSaveViewName('')}><BookmarkPlus size={14} /> Сохранить поиск</button>
                  : <form className="save-view" onSubmit={(e) => { e.preventDefault(); saveCurrentSearch() }}>
                    <input autoFocus value={saveViewName} maxLength={60} aria-label="Название поиска" placeholder="Название" onChange={(e) => setSaveViewName(e.target.value)} onKeyDown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); setSaveViewName(null) } }} />
                    <button className="text-button" type="submit" disabled={!saveViewName.trim()}>Сохранить</button>
                  </form>}
              <span className="search-help" title={SEARCH_HELP}>?</span>
            </div>
            {otherErrors.length > 0 && <div className="unified-errors" role="alert">Не ответили: {otherErrors.join('; ')}</div>}
            </>
            : <div className="message-list-header"><span>{listTitle}</span><span>{total} {pluralLetters(total)}</span></div>}
          <OfflineBanner status={conn.status} onOpen={() => setShowQueue(true)} onCheck={() => void checkConnection()} checking={checkingConn} />
          {checked.size > 0 && <div className="bulk-bar">
            <span>Отмечено: {checked.size}</span>
            <button className="text-button" onClick={() => void archive(actionTargets())}><Archive size={15} /> В архив</button>
            <button className="text-button" onClick={() => setMovePicker(actionTargets())}><FolderInput size={15} /> Переместить</button>
            <button className="text-button danger-text" onClick={() => requestDelete(actionTargets())}><Trash2 size={15} /> Удалить</button>
            <button className="icon-button soft" aria-label="Снять отметку" onClick={() => setChecked(new Set())}><X size={15} /></button>
          </div>}
          <div className="message-list" ref={listRef} onScroll={(e) => { scrollTop.current = e.currentTarget.scrollTop }}>
            {loading && messages.length === 0 && <SkeletonRows variant="message" count={9} label="Загрузка писем с сервера…" />}
            {listError && (
              <div className="empty-state" role="alert">
                <strong>Не удалось загрузить письма</strong>
                <span>{listError}</span>
                <button className="text-button" onClick={() => void loadFirstPage(folderId, true)}>Повторить</button>
              </div>
            )}
            {listRows.map(({ m, count, child, thread }, rowIndex) => {
              const key = messageKey(m)
              // Заголовок группы по дате: «Сегодня», «Вчера», «На этой неделе»… (у вложенных писем цепочки — нет).
              const group = child ? null : dateGroup(m.date)
              const prevTop = listRows.slice(0, rowIndex).reverse().find((r) => !r.child)
              const showGroup = group !== null && (!prevTop || dateGroup(prevTop.m.date) !== group)
              const outgoing = ['sent', 'drafts'].includes(folders.find((f) => f.id === m.folder)?.role ?? '')
              const sender = outgoing ? recipientsLabel(m) : senderLabel(m)
              const due = dueOf(m)
              const cats = categoriesOf(m, categories)
              const open = openThreads.has(thread)
              return (
                <Fragment key={key}>
                {showGroup && <div className="message-group" role="presentation">{group}</div>}
                <button
                  className={`message-row ${child ? 'thread-child' : ''} ${selectedKey === key ? 'selected' : ''} ${checked.has(key) ? 'checked' : ''} ${m.unread ? 'unread' : ''} ${rowMenu?.items.some((x) => messageKey(x) === key) ? 'menu-target' : ''}`}
                  onClick={(e) => onRowClick(e, m)}
                  onDoubleClick={() => { if (!otherOf(m)) openInWindow(m) }}
                  onContextMenu={(e) => { if (otherOf(m)) e.preventDefault(); else openRowMenu(e, m) }}
                  title="Двойной щелчок — открыть в отдельном окне"
                  onPointerDown={(e) => startMessageDrag(e, m)}
                  onDragStart={(e) => e.preventDefault()}
                >
                  <div className={`sender-avatar avatar-${m.uid % 5}`}>{initials(outgoing ? sender.slice(6) : sender)}</div>
                  <div className="message-summary">
                    <div className="message-topline">
                      {m.unread && <span className="unread-dot" aria-label="Не прочитано" />}
                      <span className="sender-name">{sender}</span>
                      <span className="message-icons">
                        {m.hasAttachments && <Paperclip size={13} aria-label="Есть вложения" />}
                        {m.important && <CircleAlert size={13} className="important-mark" aria-label="Важное" />}
                        {m.flagged && <Flag size={13} className="flag-mark" fill="currentColor" aria-label="С флажком" />}
                        {m.answered && <Reply size={13} aria-label="Есть ответ" />}
                      </span>
                      <time>{formatListDate(m.date)}</time>
                    </div>
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
                    {m.snippet && <div className="message-snippet">{m.snippet}</div>}
                    {searching && <div className="message-folder">{otherOf(m) ? <><span className="search-account">{otherOf(m)!.label}</span> · {m.folder}</> : folderName(m.folder)}</div>}
                    {(cats.length > 0 || due || snoozedUntil(m.keywords)) && <div className="message-tags">
                      {snoozedUntil(m.keywords) && <span className="snooze-tag" title="Отложено"><AlarmClock size={11} /> до {dueLabel(snoozedUntil(m.keywords)!)}</span>}
                      {due && <span className={`due-tag ${due.getTime() < Date.now() ? 'overdue' : ''}`} title="Срок флажка"><Flag size={11} /> {dueLabel(due)}</span>}
                      {cats.map((c) => <span key={c.keyword} className="cat-tag" style={{ borderColor: c.color, color: c.color }}><span className="cat-dot" style={{ background: c.color }} />{c.name}</span>)}
                    </div>}
                  </div>
                </button>
                </Fragment>
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
              <Circle size={7} fill={listError ? '#ef4444' : offline ? '#f59e0b' : '#22c55e'} />
              {listError ? 'Ошибка' : offline ? 'Без сети' : loading ? 'Синхронизация' : 'Онлайн'}
            </span>
          </div>
        </main>}
        {readerOpen && <Splitter id="mail-list" min={260} max={900} label="Ширина списка писем" />}

        {readerOpen && <section className="reader-pane">
          <div className="reader-toolbar">
            <button className="icon-button soft mobile-back" onClick={() => setReaderOpen(false)} aria-label="Назад"><ChevronLeft /></button>
            <div className="reader-toolbar-spacer" />
            {selected && <>
              {currentFolder?.role === 'drafts' && detail && <button className="primary-button small continue-draft" onClick={() => void continueDraft(detail)} title="Дописать и отправить этот черновик"><SquarePen size={15} /> Продолжить</button>}
              <button className="icon-button" aria-label="Ответить" title="Ответить" disabled={!detail} onClick={() => openComposer('reply')}><Reply size={18} /></button>
              <button className="icon-button" aria-label="Ответить всем" title="Ответить всем" disabled={!detail} onClick={() => openComposer('replyAll')}><ReplyAll size={18} /></button>
              <button className="icon-button" aria-label="Переслать" title="Переслать" disabled={!detail} onClick={() => openComposer('forward')}><Forward size={18} /></button>
              {ncStatus?.connected && <button className="icon-button" aria-label="Создать задачу" title="Создать задачу Deck из письма: срок, организация, документы в Nextcloud" onClick={() => setTaskFor(selected)}><SquareKanban size={18} /></button>}
              <span className="toolbar-sep" />
              <button className={`icon-button ${selected.flagged ? 'flag-on' : ''}`} aria-label="Флажок" aria-pressed={selected.flagged} title={selected.flagged ? 'Снять флажок' : 'Поставить флажок (к исполнению)'} onClick={() => void toggleFlag(selected, 'flagged')}>
                <Flag size={18} fill={selected.flagged ? 'currentColor' : 'none'} />
              </button>
              <span className="toolbar-sep" />
              <button className="icon-button" aria-label="Архивировать" title="В архив" onClick={() => void archive([selected])}><Archive size={18} /></button>
              <button className="icon-button" aria-label="Переместить в папку" title="Переместить в папку…" onClick={() => setMovePicker([selected])}>
                <FolderInput size={18} />
              </button>
              <button className="icon-button danger-icon" aria-label="Удалить письмо" title="Удалить (Del)" onClick={() => requestDelete([selected])}>
                <Trash2 size={18} />
              </button>
              <span className="toolbar-sep" />
              {/* Редкие действия — в меню «Ещё»: на панели остаются частые (как в Outlook). */}
              <button className="icon-button" aria-label="Ещё действия" title="Ещё: важное, срок, категории, отложить, правило, спам, печать…" onClick={(e) => {
                const r = e.currentTarget.getBoundingClientRect()
                const at = { x: Math.max(8, r.right - 260), y: r.bottom + 4 }
                // Пункт с подменю: открыть его на месте этого меню, когда оно закроется.
                const sub = (items: MenuItem[]) => () => window.setTimeout(() => setPopMenu({ ...at, items }), 0)
                setPopMenu({ ...at, items: [
                  { label: selected.important ? 'Снять «Важное»' : 'Пометить как важное', icon: <CircleAlert size={15} />, onSelect: () => void toggleFlag(selected, 'important') },
                  { label: 'Срок флажка…', icon: <CalendarClock size={15} />, onSelect: sub(dueMenuItems([selected])) },
                  { label: 'Категории…', icon: <Tag size={15} />, onSelect: sub(categoryMenuItems([selected])) },
                  { label: 'Отложить…', icon: <AlarmClock size={15} />, onSelect: sub(snoozeMenuItems([selected])) },
                  { label: selected.unread ? 'Отметить прочитанным' : 'Отметить непрочитанным', icon: <Mail size={15} />, onSelect: () => void toggleFlag(selected, 'seen') },
                  { separator: true },
                  { label: 'Создать правило…', icon: <ListFilter size={15} />, onSelect: () => { setFilterSeed(selected); setShowFilters(true) } },
                  currentFolder?.role === 'junk'
                    ? { label: 'Не спам', icon: <ShieldCheck size={15} />, onSelect: () => void markSpam([selected], false) }
                    : { label: 'Это спам', icon: <ShieldAlert size={15} />, onSelect: () => void markSpam([selected], true) },
                  { separator: true },
                  { label: 'Печать или PDF', icon: <Printer size={15} />, disabled: !detail, onSelect: () => { if (detail) printMessage(detail) } },
                  { label: 'Открыть в отдельном окне', icon: <ExternalLink size={15} />, onSelect: () => openInWindow(selected) },
                ] })
              }}><MoreHorizontal size={18} /></button>
            </>}
            <button className="icon-button soft" aria-label="Скрыть чтение" onClick={() => setReaderOpen(false)}><PanelRightClose size={18} /></button>
          </div>
          {detail ? <div className="reader-content">
            <div className="reader-title-row"><h1>{detail.subject || '(без темы)'}</h1></div>
            <div className="sender-block">
              <div className="sender-avatar sender-avatar-large">{initials(senderLabel(detail))}</div>
              <div className="sender-details">
                <span className="sender-line"><AddressLine strong entries={[detail.fromName ? `${detail.fromName} <${detail.fromAddress}>` : detail.fromAddress]} onCompose={composeTo} onSearch={setQuery} onHistory={openHistory} onNotice={setNotice} /></span>
                {detail.to.length > 0 && <span>Кому: <AddressLine entries={detail.to} onCompose={composeTo} onSearch={setQuery} onHistory={openHistory} onNotice={setNotice} /></span>}
                {detail.cc.length > 0 && <span>Копия: <AddressLine entries={detail.cc} onCompose={composeTo} onSearch={setQuery} onHistory={openHistory} onNotice={setNotice} /></span>}
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
              onNotice={setNotice}
              onEdit={ncStatus?.connected ? (i, name) => { setNotice(`Открываю «${name}»…`); api.ncEditAttachment(detail.folder, detail.uid, i, NC_MAIL_DIR).then((r) => setNotice(editNotice(r, name))).catch((e) => setNotice(`Не удалось открыть в редакторе: ${errorText(e)}`)) } : undefined}
            />}
            <MdnBar detail={detail} me={myAddress} onNotice={setNotice} onDone={() => { setDetail((d) => d && { ...d, keywords: [...d.keywords, '$MDNSent'] }); patchMessage(messageKey(detail), { keywords: [...detail.keywords, '$MDNSent'] }) }} />
            <MessageBody detail={detail} scale={appearance.text / 14} dark={dark} font={appearance.readFont ? fontStack(appearance.readFont) : ''} onNotice={setNotice} />
            <div className="reader-actions">
              <button className="primary-button" onClick={() => openComposer('reply')}><Reply size={16} /> Ответить</button>
              <button className="secondary-button" onClick={() => openComposer('replyAll')}><ReplyAll size={16} /> Ответить всем</button>
              <button className="secondary-button" onClick={() => openComposer('forward')}><Forward size={16} /> Переслать</button>
            </div>
          </div> : <div className="empty-reader">
            {detailLoading ? <SkeletonReader />
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
          from={profile?.displayName ? `${profile.displayName} <${myAddress}>` : myAddress}
          draft={draft}
          contacts={contacts}
          ncConnected={Boolean(ncStatus?.connected)}
          signatures={sigs}
          recovery
          recipientPolicy={recipientPolicy}
          minimized={composeMin}
          onMinimize={setComposeMin}
          onClose={() => { setDraft(null); setComposeMin(false); if (folders.find((f) => f.id === folderId)?.role === 'drafts') void loadFirstPage(folderId, true) }}
          onSent={(text) => {
            setDraft(null)
            setComposeMin(false)
            setNotice(text)
            loadScheduled()
            if ((draft.mode === 'reply' || draft.mode === 'replyAll') && detail) patchMessage(messageKey(detail), { answered: true })
          }}
          onQueued={(id, snapshot, delay) => {
            setDraft(null)
            setComposeMin(false)
            outbox.queued(id, snapshot, delay)
            if ((draft.mode === 'reply' || draft.mode === 'replyAll') && detail) patchMessage(messageKey(detail), { answered: true })
          }}
        />
      )}

      {outbox.element}
      {recovered && !draft && (
        <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) setRecovered(null) }}>
          <div className="settings-modal confirm-modal" role="alertdialog" aria-label="Несохранённое письмо">
            <div className="compose-header"><div><strong>Несохранённое письмо</strong><small>{recovered.subject || '(без темы)'}{recovered.to ? ` — ${recovered.to.replace(/,\s*$/, '')}` : ''}</small></div></div>
            <p className="confirm-text">Письмо осталось незаконченным{recovered.savedAt ? ` (${new Date(recovered.savedAt).toLocaleString('ru-RU', { day: 'numeric', month: 'long', hour: '2-digit', minute: '2-digit' })})` : ''} — программа была закрыта или прервана. Продолжить его?</p>
            {recovered.lostFiles && recovered.lostFiles.length > 0 && <p className="confirm-text muted">Вложения нужно будет приложить заново: {recovered.lostFiles.join(', ')}.</p>}
            <div className="settings-actions">
              <button className="text-button danger-text" onClick={() => { if (recovered.key) void api.draftDiscard(recovered.key, null).catch(() => undefined); void api.draftRecoverySet(null); setRecovered(null) }}><Trash2 size={14} /> Удалить</button>
              <div className="reader-toolbar-spacer" />
              <button className="secondary-button" onClick={() => setRecovered(null)}>Позже</button>
              <button className="primary-button" autoFocus onClick={() => { const { savedAt: _s, lostFiles: _l, ...d } = recovered; setRecovered(null); startDraft(d) }}>Продолжить письмо</button>
            </div>
          </div>
        </div>
      )}
      {globalOpen && <GlobalSearch ncConnected={Boolean(ncStatus?.connected)} onOpen={openFound} onClose={() => setGlobalOpen(false)} />}
      {taskFor && <TaskFromMail message={taskFor} ownAddress={myAddress} onClose={() => setTaskFor(null)} onNotice={setNotice}
        onOpenCard={(boardId, cardId) => { const n = nextFocus(); setDeckFocus({ boardId, cardId, n }); setModule('deck') }} />}
      {popMenu && <ContextMenu x={popMenu.x} y={popMenu.y} items={popMenu.items} onClose={() => setPopMenu(null)} />}
      {snoozePick && <DuePicker title="Отложить письмо" hint="В это время письмо вернётся во «Входящие» непрочитанным" action="Отложить" onClose={() => setSnoozePick(null)} onPick={(d) => { const items = snoozePick; setSnoozePick(null); void snoozeItems(items, d) }} />}
      {duePick && <DuePicker onClose={() => setDuePick(null)} onPick={(d) => { const items = duePick; setDuePick(null); void setDueFor(items, d) }} />}
      {showQueue && conn.status && <QueuePanel status={conn.status} onStatus={conn.setStatus} onClose={() => setShowQueue(false)} onNotice={setNotice} />}
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
      {showFilters && <MailFilters folders={folders} seed={filterSeed} categories={userCategories(categories)} onClose={() => setShowFilters(false)} onNotice={setNotice} label={folderLabel} />}
      {showSettings && <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) setShowSettings(false) }}>
        <div className="settings-modal">
          <div className="compose-header"><div><strong>Учётная запись</strong><small>Параметры подключения</small></div><button className="icon-button" onClick={() => setShowSettings(false)} aria-label="Закрыть"><X size={18} /></button></div>
          <div className="settings-account">
            <div className="account-avatar"><Mail size={20} /></div>
            <div><strong>{myAddress}</strong><span>{account.exchange ? `Exchange (EWS) · ${account.host}` : `IMAP · ${account.host}:${account.port}`} · TLS</span></div>
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
          {account.exchange && <MailboxesSettings onChanged={() => void loadFolders()} onNotice={setNotice} />}
          {ncStatus && <div className="settings-section"><strong>Nextcloud (Deck, Talk)</strong><NcConnect status={ncStatus} onChange={setNcStatus} compact /></div>}
          <div className="settings-section notify-settings">
            <strong>Уведомления</strong>
            <label><input type="checkbox" checked={prefs.mail} onChange={(e) => updatePrefs({ mail: e.target.checked })} /> О новых письмах</label>
            <label><input type="checkbox" checked={prefs.shared} disabled={!prefs.mail} onChange={(e) => updatePrefs({ shared: e.target.checked })} /> Включая подключённые (общие) ящики</label>
            <label><input type="checkbox" checked={prefs.chat} onChange={(e) => updatePrefs({ chat: e.target.checked })} /> О сообщениях в чате</label>
            <label><input type="checkbox" checked={prefs.preview && notifyPolicy === 'full'} disabled={notifyPolicy !== 'full'} onChange={(e) => updatePrefs({ preview: e.target.checked })} /> Показывать тему письма и текст сообщения</label>
            {notifyPolicy !== 'full' && <span className="settings-hint">{notifyPolicy === 'none' ? 'Администратор разрешил показывать в уведомлениях только сам факт нового письма или сообщения.' : 'Администратор запретил показывать в уведомлениях тему и текст.'}</span>}
            <button className="text-button" onClick={() => notify('Atom Workspace', 'Так выглядят уведомления о новой почте и сообщениях')}>Показать пример уведомления</button>
          </div>
          <AppearanceSettings />
          <SignaturesSettings set={sigs} onSaved={(set, msg) => { setSigs(set); setNotice(msg) }} />
          <SendingSettings />
          <CategoriesSettings categories={categories} onSaved={(list, msg) => { setCategories(list); setNotice(msg) }} />
          <DesktopSettings onNotice={setNotice} />
          <OfflineSettings onNotice={setNotice} />
          <TransferSettings folders={folders} label={folderLabel} onNotice={setNotice} onImported={() => void loadFolders()} />
          <TotpSettings onChange={setTotpOn} onLock={() => { setShowSettings(false); onLock() }} autolock={autolock} onAutolock={updateAutolock} />
          <div className="settings-note"><ShieldCheck size={18} /><span>Пароль хранится только в системном хранилище (Windows Credential Manager, GNOME Keyring, Связка ключей macOS) и не записывается в файлы настроек. Для SMTP, календаря и контактов используются те же логин и пароль.</span></div>
          <div className="settings-actions">
            <button className="secondary-button" onClick={() => { setShowSettings(false); setFilterSeed(null); setShowFilters(true) }}><ListFilter size={16} /> Правила обработки почты</button>
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
