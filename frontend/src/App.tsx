import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  Mail, Paperclip, PanelRightOpen, X } from 'lucide-react'

import {
  api, errorText, messageKey, NC_MAIL_DIR, PAGE_SIZE } from './api'
import type {
  Account, Contact, Draft, PickedFile, SignatureSet, MailFilter, MessageDetail, MessageSummary, Scheduled, Category, UnifiedFilters, UnifiedRole, UnifiedItem } from './types'
import Composer from './Composer'
import DeckView from './DeckView'
import TalkView from './TalkView'
import NcConnect from './NcConnect'
import FilesView from './FilesView'
import type { CalEvent, NcStatus } from './types'
import ContactsView from './ContactsView'
import CounterpartyView from './CounterpartyView'
import { QueuePanel, useConnection } from './OfflineQueue'
import CalendarView from './CalendarView'
import MailFilters from './MailFilters'
import ContextMenu from './ContextMenu'
import GlobalSearch, { type Found } from './GlobalSearch'
import TaskFromMail from './TaskFromMail'
import type { MenuItem } from './ContextMenu'
import DuePicker from './DuePicker'
import { fileToBase64 } from './mailTools'
import { threadKeys, userCategories } from './mailExtras'
import ScheduledMail from './ScheduledMail'
import { useOutbox } from './sending'
import { signatureById } from './drafting'
import { NO_FILTERS, UnifiedColumn, useAccounts } from './Accounts'
import { saveView, savedView, type ModuleName, type ViewContext } from './viewContext'
import { buildDraft, ensureArchive, forwardDraft, withSignature } from './mailActions'
import FolderPicker, { trashFor } from './FolderPicker'
import { loadPrefs, savePrefs } from './notifications'
import type { NotifyPrefs } from './notifications'
import Splitter from './Splitter'

import { byFolder, dateGroup, folderLabel, pluralLetters } from './mailLabels'
import { useAutolock } from './useAutolock'
import { useTheme } from './useTheme'
import { useDiagnostics } from './useDiagnostics'
import { useMessageDrag } from './useMessageDrag'
import { useMessageFlags } from './useMessageFlags'
import { useFolders } from './useFolders'
import { useNewMailPolling } from './useNewMailPolling'
import { rowMenuItems as rowMenuFor } from './mailRowMenu'
import { useSavedViews } from './useSavedViews'
import { useMailSearch } from './useMailSearch'
import { useProfile } from './useProfile'
import { useChatNotifications, useDueReminders, useZoomShortcuts } from './useMailBackground'
import SettingsDialog from './SettingsDialog'
import ReaderPane from './ReaderPane'
import MessageListPane from './MessageListPane'
import FolderSidebar from './FolderSidebar'
import ModuleRail from './ModuleRail'
import TopBar from './TopBar'
import ConfirmDeleteDialog from './ConfirmDeleteDialog'
import RecoveredDraftDialog from './RecoveredDraftDialog'

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
  const { totpOn, setTotpOn, autolock, updateAutolock, composingRef } = useAutolock(onLock)

  const { appearance, dark, setDark } = useTheme()
  const [foldersOpen, setFoldersOpen] = useState(true)
  const [readerOpen, setReaderOpen] = useState(true)
  const [showAccountMenu, setShowAccountMenu] = useState(false)
  const [showSettings, setShowSettings] = useState(false)
  const [notice, setNotice] = useState(initialNotice)

  const { folders, setFolders, folderId, setFolderId, expanded, toggleExpanded, currentFolder, folderName, visibleFolders, nodeUnseen, loadFolders } = useFolders(saved?.folderId, setNotice)

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
  /** Контекстное меню письма: где открыто и к каким письмам относится. */
  const [rowMenu, setRowMenu] = useState<{ x: number; y: number; items: MessageSummary[] } | null>(null)
  /** Было открыто отдельное окно письма — при возврате в главное окно список обновляется. */
  const windowOpened = useRef(false)
  const [detail, setDetail] = useState<MessageDetail | null>(null)
  const [detailLoading, setDetailLoading] = useState(false)
  const [detailError, setDetailError] = useState('')

  const [prefs, setPrefs] = useState<NotifyPrefs>(loadPrefs)
  const prefsRef = useRef(prefs)
  prefsRef.current = prefs
  const pollSecs = useRef(60)
  const acc = useAccounts(() => pollSecs.current, () => prefsRef.current, setNotice)
  const [filter, setFilter] = useState<MailFilter>(saved?.filter ?? 'Все')
  const {
    query, setQuery, searchScope, setSearchScope, otherErrors, othersBusy, searchBusy, serverSearched,
    searchResults, setSearchResults, searching, clearSearch, searchOnServer } = useMailSearch(saved, acc.multi, setNotice)
  /** Письма текущего списка: найденные или письма папки. */
  const pool = searchResults ?? messages


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
  const { profile, profileForm, setProfileForm, confirmPassword, setConfirmPassword, serversChanged, profileBusy, saveProfile } = useProfile(setNotice)

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
  const moduleRef = useRef(module)
  moduleRef.current = module
  useEffect(() => { api.notifyPollSecs().then((s) => { pollSecs.current = Math.max(5, s) }).catch(() => undefined) }, [])
  useDueReminders(setNotice, pollSecs)
  useChatNotifications(Boolean(ncStatus?.connected), moduleRef, prefsRef, pollSecs, setChatUnread, setNotice)
  useZoomShortcuts()
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



  // Защита от гонок: ответ на устаревший запрос игнорируется.
  const listRequest = useRef(0)
  const detailRequest = useRef(0)
  const searchRef = useRef<HTMLInputElement>(null)


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

  const folderIdRef = useRef(folderId)
  folderIdRef.current = folderId
  useNewMailPolling({ folderIdRef, prefsRef, pollSecs, setFolders, setNotice, loadFirstPage })

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
      from: d.fromAddress && d.fromAddress.toLowerCase() !== myAddress.toLowerCase() ? d.fromAddress.toLowerCase() : null })
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

  const { toggleFlag, setCategory, setDueFor, dueMenuItems, categoryMenuItems, markSpam, snoozeItems, unsnoozeItems, snoozeMenuItems } = useMessageFlags({
    patchMessage, adjustUnseen, dropFromList, categories, setDuePick, setSnoozePick, setNotice,
    openSettings: () => setShowSettings(true) })

  async function doUnsubscribe(d: MessageDetail) {
    try {
      const r = await api.unsubscribe(d.folder, d.uid)
      if (r.done) setNotice(`Вы отписаны от рассылки (${r.how}).`)
      else if (r.openUrl) { setNotice('Отписка — на сайте рассылки: открываю в браузере.'); void api.openLink(r.openUrl).catch(() => undefined) }
    } catch (e) {
      setNotice(`Не удалось отписаться: ${errorText(e)}`)
    }
  }

  const { diagText, diagBusy, runDiagnostics, copyDiagnostics } = useDiagnostics(folderId, currentFolder, messages.length, setNotice)

  /** Новое письмо со ссылками на файлы Nextcloud. */
  function sendLinks(links: { name: string; url: string }[]) {
    const lines = links.map((l) => `${l.name}: ${l.url}`).join('\n')
    startDraft({
      mode: 'new', to: '', cc: '', inReplyTo: null, references: null,
      subject: links.length === 1 ? links[0].name : `Файлы (${links.length})`,
      body: `\n\n${links.length === 1 ? 'Ссылка на файл' : 'Ссылки на файлы'}:\n${lines}` + withSignature('', signature) })
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


  const { views, viewsOpen, setViewsOpen, saveViewName, setSaveViewName, runView, storeViews, saveCurrentSearch, activeView } = useSavedViews({
    query, searchScope, multi: acc.multi, setNotice,
    onRun: (v, scope) => {
      setModule('mail')
      setUnified(null)
      setSearchScope(scope)
      setQuery(v.query)
    } })


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

  const { dragInfo, beginDrag, suppressRowClick } = useMessageDrag((items, target) => void moveTo(items, target))
  function startMessageDrag(e: React.PointerEvent, m: MessageSummary) {
    const key = messageKey(m)
    beginDrag(e, checked.has(key) ? pool.filter((x) => checked.has(messageKey(x))) : [m])
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

  const rowMenuItems = (items: MessageSummary[]) => rowMenuFor(items, {
    toggleFlag, openMessage, openInWindow, composeFor, ncStatus, setTaskFor, setDuePick, unsnoozeItems, snoozeItems, setSnoozePick, categories, setCategory, archive, setMovePicker, setFilterSeed, setShowFilters, folders, markSpam, detail, setNotice, requestDelete })

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


  const listTitle = currentFolder ? folderLabel(currentFolder) : 'Входящие'

  return (
    <div className={`app-shell ${dark ? 'theme-dark' : ''}`}>
      <TopBar
        showAccountMenu={showAccountMenu}
        acc={acc}
        setShowAccountMenu={setShowAccountMenu}
        myAddress={myAddress}
        account={account}
        openComposer={openComposer}
        setGlobalOpen={setGlobalOpen}
        setDark={setDark}
        dark={dark}
        setShowSettings={setShowSettings}
        switchAccount={switchAccount}
        totpOn={totpOn}
        onLock={onLock}
        onLogout={onLogout}
        setNotice={setNotice}
      />

      <div
        className={`workspace ${fileOver ? 'file-over' : ''}`}
        onDragOver={(e) => { if (module === 'mail' && !draft && hasFiles(e)) { e.preventDefault(); e.dataTransfer.dropEffect = 'copy'; if (!fileOver) setFileOver(true) } }}
        onDragLeave={(e) => { if (fileOver && !e.currentTarget.contains(e.relatedTarget as Node | null)) setFileOver(false) }}
        onDrop={(e) => { if (module === 'mail' && !draft && e.dataTransfer.files.length > 0) { e.preventDefault(); void dropToNewMail(e.dataTransfer.files) } else setFileOver(false) }}
      >
        {fileOver && <div className="drop-hint" aria-hidden="true"><Paperclip size={22} /> Отпустите — файлы будут вложены в новое письмо</div>}
        <ModuleRail {...{ module, setModule, chatUnread }} />

        {module === 'calendar' && <CalendarView onNotice={setNotice} focus={eventFocus} />}
        {module === 'counterparty' && <CounterpartyView domain={cpDomain} onDomain={setCpDomain} onCompose={composeTo} onNotice={setNotice} onOpen={openFound}
          ncConnected={Boolean(ncStatus?.connected)} onTask={ncStatus?.connected ? setTaskFor : undefined} />}
        {(module === 'deck' || module === 'chat' || module === 'files') && ncStatus && !ncStatus.connected && (
          <div className="module-view nc-gate"><NcConnect status={ncStatus} onChange={setNcStatus} /></div>
        )}
        {module === 'deck' && ncStatus?.connected && <DeckView onNotice={setNotice} focus={deckFocus} onOpenMail={openMailById} onOpenFolder={(path) => openFound({ kind: 'file', f: { path, name: '', isDir: true, size: 0, modified: '', mime: '', fileId: null, shared: false, favorite: false, canWrite: true } })} onOpenOrg={openHistory} />}
        {module === 'chat' && ncStatus?.connected && <TalkView onUnread={setChatUnread} onNotice={setNotice} focus={talkFocus} onWrite={(address) => composeTo(address)} />}
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
        {foldersOpen && <FolderSidebar
          acc={acc}
          unifiedOpen={unifiedOpen}
          setUnifiedOpen={setUnifiedOpen}
          unified={unified}
          clearSearch={clearSearch}
          setUnified={setUnified}
          viewsOpen={viewsOpen}
          setViewsOpen={setViewsOpen}
          views={views}
          activeView={activeView}
          runView={runView}
          storeViews={storeViews}
          folders={folders}
          visibleFolders={visibleFolders}
          expanded={expanded}
          toggleExpanded={toggleExpanded}
          folderId={folderId}
          dragInfo={dragInfo}
          setFolderId={setFolderId}
          nodeUnseen={nodeUnseen}
          scheduled={scheduled}
          loadScheduled={loadScheduled}
          setShowScheduled={setShowScheduled}
          offline={offline}
          setShowQueue={setShowQueue}
          conn={conn}
          account={account}
        />}
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
        /> : <MessageListPane
          setFoldersOpen={setFoldersOpen}
          foldersOpen={foldersOpen}
          searchRef={searchRef}
          query={query}
          setQuery={setQuery}
          clearSearch={clearSearch}
          loading={loading}
          refresh={refresh}
          setFilterSeed={setFilterSeed}
          setShowFilters={setShowFilters}
          filter={filter}
          setFilter={setFilter}
          categories={categories}
          catFilter={catFilter}
          setCatFilter={setCatFilter}
          threadsOn={threadsOn}
          setThreadsOn={setThreadsOn}
          searching={searching}
          searchResults={searchResults}
          othersBusy={othersBusy}
          serverSearched={serverSearched}
          searchBusy={searchBusy}
          searchOnServer={searchOnServer}
          acc={acc}
          searchScope={searchScope}
          setSearchScope={setSearchScope}
          activeView={activeView}
          saveViewName={saveViewName}
          setSaveViewName={setSaveViewName}
          saveCurrentSearch={saveCurrentSearch}
          otherErrors={otherErrors}
          listTitle={listTitle}
          total={total}
          conn={conn}
          setShowQueue={setShowQueue}
          checkConnection={checkConnection}
          checkingConn={checkingConn}
          checked={checked}
          archive={archive}
          actionTargets={actionTargets}
          setMovePicker={setMovePicker}
          requestDelete={requestDelete}
          setChecked={setChecked}
          listRef={listRef}
          scrollTop={scrollTop}
          messages={messages}
          listError={listError}
          loadFirstPage={loadFirstPage}
          folderId={folderId}
          listRows={listRows}
          dateGroup={dateGroup}
          folders={folders}
          openThreads={openThreads}
          selectedKey={selectedKey}
          rowMenu={rowMenu}
          onRowClick={onRowClick}
          otherOf={otherOf}
          openInWindow={openInWindow}
          openRowMenu={openRowMenu}
          startMessageDrag={startMessageDrag}
          setOpenThreads={setOpenThreads}
          folderName={folderName}
          visibleMessages={visibleMessages}
          currentFolder={currentFolder}
          setShowSettings={setShowSettings}
          runDiagnostics={runDiagnostics}
          hasMore={hasMore}
          loadingMore={loadingMore}
          loadMore={loadMore}
          offline={offline}
        />}
        {readerOpen && <Splitter id="mail-list" min={260} max={900} label="Ширина списка писем" />}

        {readerOpen && <ReaderPane
          setReaderOpen={setReaderOpen}
          selected={selected}
          currentFolder={currentFolder}
          detail={detail}
          continueDraft={continueDraft}
          openComposer={openComposer}
          ncStatus={ncStatus}
          setTaskFor={setTaskFor}
          toggleFlag={toggleFlag}
          archive={archive}
          setMovePicker={setMovePicker}
          requestDelete={requestDelete}
          setPopMenu={setPopMenu}
          dueMenuItems={dueMenuItems}
          categoryMenuItems={categoryMenuItems}
          snoozeMenuItems={snoozeMenuItems}
          setFilterSeed={setFilterSeed}
          setShowFilters={setShowFilters}
          markSpam={markSpam}
          openInWindow={openInWindow}
          composeTo={composeTo}
          setQuery={setQuery}
          openHistory={openHistory}
          setNotice={setNotice}
          doUnsubscribe={doUnsubscribe}
          saveAttachment={saveAttachment}
          saveToNextcloud={saveToNextcloud}
          myAddress={myAddress}
          setDetail={setDetail}
          patchMessage={patchMessage}
          appearance={appearance}
          dark={dark}
          detailLoading={detailLoading}
          detailError={detailError}
        />}
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
        <RecoveredDraftDialog {...{ setRecovered, recovered, startDraft }} />
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
      {confirmDelete && <ConfirmDeleteDialog {...{ setConfirmDelete, confirmDelete, deleteNow }} />}
      {showFilters && <MailFilters folders={folders} seed={filterSeed} categories={userCategories(categories)} onClose={() => setShowFilters(false)} onNotice={setNotice} label={folderLabel} />}
      {showSettings && <SettingsDialog
        setShowSettings={setShowSettings}
        myAddress={myAddress}
        account={account}
        profileForm={profileForm}
        setProfileForm={setProfileForm}
        serversChanged={serversChanged}
        confirmPassword={confirmPassword}
        setConfirmPassword={setConfirmPassword}
        profileBusy={profileBusy}
        saveProfile={saveProfile}
        loadFolders={loadFolders}
        setNotice={setNotice}
        ncStatus={ncStatus}
        setNcStatus={setNcStatus}
        prefs={prefs}
        updatePrefs={updatePrefs}
        notifyPolicy={notifyPolicy}
        sigs={sigs}
        setSigs={setSigs}
        categories={categories}
        setCategories={setCategories}
        folders={folders}
        setTotpOn={setTotpOn}
        onLock={onLock}
        autolock={autolock}
        updateAutolock={updateAutolock}
        setFilterSeed={setFilterSeed}
        setShowFilters={setShowFilters}
        diagBusy={diagBusy}
        runDiagnostics={runDiagnostics}
        onLogout={onLogout}
        diagText={diagText}
        copyDiagnostics={copyDiagnostics}
      />}
    </div>
  )
}
