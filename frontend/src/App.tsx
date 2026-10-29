import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  Archive, CalendarDays, ChevronDown, ChevronLeft, ChevronRight, Circle, Cloud, ContactRound, Download,
  FilePenLine, FileText, Folder as FolderIcon, FolderOpen, Forward, Inbox, LogOut, Mail,
  MessageCircle, Moon, Paperclip, PanelLeftClose, PanelRightClose, PanelRightOpen, RefreshCw,
  Reply, Search, Send, Settings, ShieldCheck, SquarePen, Stethoscope, Sun, Trash2, Users, X, Copy, FolderInput, ListFilter, ReplyAll, Flag, CircleAlert, ExternalLink, MailOpen,
} from 'lucide-react'

import {
  api, editNotice, errorText, formatFullDate, formatListDate, messageKey, NC_MAIL_DIR, PAGE_SIZE,
} from './api'
import type {
  Account, Contact, Draft, Folder, FolderRole, MailFilter, MessageDetail, MessageSummary, Settings as Profile,
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
import { buildDraft, ensureArchive } from './mailActions'
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
}

export default function App({ account, initialNotice, onLogout }: Props) {
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
  const [filter, setFilter] = useState<MailFilter>('Все')

  const [diagText, setDiagText] = useState<string | null>(null)
  const [diagBusy, setDiagBusy] = useState(false)

  const [module, setModule] = useState<'mail' | 'calendar' | 'contacts' | 'deck' | 'chat' | 'files'>('mail')
  const [ncStatus, setNcStatus] = useState<NcStatus | null>(null)
  const [chatUnread, setChatUnread] = useState(0)
  const [draft, setDraft] = useState<Draft | null>(null)
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
      setChecked(new Set())
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
      body: `\n\n${links.length === 1 ? 'Ссылка на файл' : 'Ссылки на файлы'}:\n${lines}`,
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
    const q = query.trim().toLowerCase()
    return messages.filter((m) => {
      const filterMatch =
        filter === 'Все' ? true :
        filter === 'Непрочитанные' ? m.unread :
        filter === 'Важные' ? m.important :
        filter === 'С флажком' ? m.flagged :
        m.hasAttachments
      if (!filterMatch) return false
      if (!q) return true
      return `${m.fromName} ${m.fromAddress} ${m.subject} ${m.to.join(' ')}`.toLowerCase().includes(q)
    })
  }, [messages, filter, query])

  const selected = messages.find((m) => messageKey(m) === selectedKey)

  /** Письма, над которыми выполняется действие: отмеченные, иначе открытое. */
  function actionTargets(): MessageSummary[] {
    if (checked.size > 0) return messages.filter((m) => checked.has(messageKey(m)))
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
      const nextMsg = next ? messages.find((m) => messageKey(m) === next) : undefined
      setSelectedKey('')
      setDetail(null)
      if (nextMsg) void openMessage(nextMsg)
    }
  }

  async function moveTo(items: MessageSummary[], target: string) {
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
    const items = checked.has(key) ? messages.filter((x) => checked.has(messageKey(x))) : [m]
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
      startDraft({ mode: 'new', to: '', cc: '', subject: '', body: '', inReplyTo: null, references: null })
      return
    }
    startDraft(buildDraft(mode, detail, account.username))
  }

  /** Ответ/пересылка из меню — письмо может быть ещё не открыто: загружаем его. */
  async function composeFor(m: MessageSummary, mode: 'reply' | 'replyAll' | 'forward') {
    try {
      const full = detail && messageKey(detail) === messageKey(m) ? detail : await api.getMessage(m.folder, m.uid)
      startDraft(buildDraft(mode, full, account.username))
    } catch (error) {
      setNotice(`Не удалось открыть письмо: ${errorText(error)}`)
    }
  }

  function openRowMenu(e: React.MouseEvent, m: MessageSummary) {
    e.preventDefault()
    const key = messageKey(m)
    // Как в Outlook: правый щелчок по отмеченным — меню для всех отмеченных, иначе — для этого письма.
    const items = checked.has(key) && checked.size > 1 ? messages.filter((x) => checked.has(messageKey(x))) : [m]
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
      { separator: true },
      { label: 'В архив', icon: <Archive size={15} />, onSelect: () => void archive(items) },
      { label: 'Переместить в папку…', icon: <FolderInput size={15} />, onSelect: () => setMovePicker(items) },
    )
    if (one) list.push({ label: 'Создать фильтр по отправителю…', icon: <ListFilter size={15} />, onSelect: () => { setFilterSeed(one); setShowFilters(true) } })
    list.push(
      { separator: true },
      { label: items.length > 1 ? `Удалить (${items.length})` : 'Удалить', icon: <Trash2 size={15} />, hint: 'Del', danger: true, onSelect: () => requestDelete(items) },
    )
    return list
  }

  async function archive(items: MessageSummary[]) {
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
                  className={`folder-item ${folderId === f.id ? 'selected' : ''} ${f.total === 0 ? 'folder-empty' : ''} ${dragInfo?.over === f.id && f.id !== folderId ? 'drop-target' : ''}`}
                  data-folder-id={f.id}
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
            <button className="icon-button soft" aria-label="Фильтры почты" title="Фильтры почты (правила сортировки)" onClick={() => { setFilterSeed(null); setShowFilters(true) }}>
              <ListFilter size={17} />
            </button>
          </div>
          <div className="filter-row">
            {(['Все', 'Непрочитанные', 'Важные', 'С флажком', 'С вложениями'] as MailFilter[]).map((item) => (
              <button key={item} className={`filter-chip ${filter === item ? 'active' : ''}`} onClick={() => setFilter(item)}>{item}</button>
            ))}
          </div>
          <div className="message-list-header"><span>{listTitle}</span><span>{total} {pluralLetters(total)}</span></div>
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
            {visibleMessages.map((m) => {
              const key = messageKey(m)
              const sender = senderLabel(m)
              return (
                <button
                  key={key}
                  className={`message-row ${selectedKey === key ? 'selected' : ''} ${checked.has(key) ? 'checked' : ''} ${m.unread ? 'unread' : ''} ${rowMenu?.items.some((x) => messageKey(x) === key) ? 'menu-target' : ''}`}
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
                    <div className="message-subject">{m.subject || '(без темы)'}</div>
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
              <button className="icon-button" aria-label="Открыть в новом окне" title="Открыть в отдельном окне" onClick={() => openInWindow(selected)}><ExternalLink size={18} /></button>
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
            if ((draft.mode === 'reply' || draft.mode === 'replyAll') && detail) patchMessage(messageKey(detail), { answered: true })
          }}
        />
      )}

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
