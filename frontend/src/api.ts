import { invoke as rawInvoke } from '@tauri-apps/api/core'
import type { CalendarFile, CalendarImport, ConnectionStatus, Counterparty, CounterpartyTimeline, DeckActivity, Dossier, RuleSet,
  Account, Category, MailTemplate, DesktopPrefs, OfflineStatus, Scheduled, SendTicket, TotpSetup, TotpStatus, Contact, NcFile, NcShare, UploadedLink, DeckAttachment, DeckBoardDetails, DeckComment, ContactInput, ContactsResult, DeckBoard, DeckStack, EventInput, EventsResult, Folder, NcStatus,
  SieveFilter, TalkChunk, TalkMention, NcUser, TalkRoom, TalkParticipant, TalkSearchHit, TalkPoll, TalkMessage, LinkPreview, LoginResult, MessageDetail, MessagePage,
  MessageSummary, PickedFile, SendResult, Settings, UpdateInfo, InstallResult, AppInfo, ImportSource, JobStatus, AccountEntry, AccountUnread, OtherNewMail, UnifiedInbox, UnifiedRole, SavedView,
} from './types'

export interface LoginParams {
  host: string
  port: number
  username: string
  password: string
  remember: boolean
  smtpHost: string
  smtpPort: number
  davUrl: string
  /** Exchange: адрес EWS или 'auto' (автоопределение); нет — обычный IMAP. */
  ewsUrl?: string
  /** Exchange: адрес почты, если логин — DOMAIN\user. */
  address?: string
}

/** Ошибки команд приходят строкой (см. AppError в src-tauri/src/error.rs). */
export function errorText(error: unknown): string {
  if (typeof error === 'string') return error
  if (error instanceof Error) return error.message
  return 'Неизвестная ошибка'
}

export const PAGE_SIZE = 50

/** Пауза после последнего нажатия клавиши, после которой ищем и показываем подсказки. */
export const TYPING_PAUSE_MS = 300

/**
 * Команды только для чтения: одинаковый запрос, который уже выполняется, второй раз
 * не отправляется — все вызовы получают один и тот же ответ (двойной щелчок, повторный
 * рендер, несколько окон одного раздела). Изменяющие команды и long-poll идут как есть.
 */
const READ_ONLY = new Set([
  'get_settings', 'get_profile', 'get_policy', 'get_message', 'desktop_prefs_get', 'scheduled_list', 'categories_get', 'due_list',
  'templates_list', 'list_contacts', 'accounts_list', 'accounts_unread', 'unified_inbox', 'unified_folder', 'search_others', 'views_get',
  'app_info', 'search_contacts', 'contact_details', 'browse_contacts', 'list_events', 'nc_status', 'deck_boards', 'deck_stacks',
  'deck_board_details', 'deck_comments', 'deck_attachments', 'deck_activity', 'talk_rooms', 'talk_history', 'talk_mentions',
  'nc_search_users', 'nc_files_list', 'nc_shares', 'talk_file_preview', 'talk_file_data', 'totp_status', 'cached_folders',
  'cached_messages', 'search_mail', 'search_server', 'offline_status', 'offline_queue', 'signature_get', 'list_folders', 'list_messages',
  'nc_editors', 'nc_avatar', 'attachment_data', 'mail_filters_get', 'rules_get', 'counterparties', 'counterparty_timeline',
  'calendar_file_preview', 'diagnostics', 'talk_participants', 'talk_search', 'talk_poll_get', 'talk_shared', 'nc_link_preview',
])
/** Опросы и служебные вызовы: ничего не меняют, разделённые запросы не сбрасывают. */
const PASSIVE = new Set(['talk_poll', 'job_status', 'update_progress', 'nc_connect_poll', 'send_wait', 'notify', 'notify_poll_secs', 'nc_open', 'open_link'])
const inflight = new Map<string, Promise<unknown>>()

function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (PASSIVE.has(cmd)) return rawInvoke<T>(cmd, args)
  if (!READ_ONLY.has(cmd)) {
    // После изменения (перенос, флажок, отправка…) чтение должно увидеть новое состояние:
    // уже идущие запросы больше не разделяются — ни сейчас, ни когда изменение завершится.
    inflight.clear()
    return rawInvoke<T>(cmd, args).finally(() => inflight.clear())
  }
  const key = `${cmd}\u0000${JSON.stringify(args ?? {})}`
  const running = inflight.get(key)
  if (running) return running as Promise<T>
  const p = rawInvoke<T>(cmd, args).finally(() => inflight.delete(key))
  inflight.set(key, p)
  return p
}

export const api = {
  getSettings: () => invoke<Settings | null>('get_settings'),
  login: ({ host, port, username, password, remember, smtpHost, smtpPort, davUrl, ewsUrl, address }: LoginParams) =>
    invoke<LoginResult>('login', { host, port, username, password, remember, smtpHost, smtpPort, davUrl, ewsUrl: ewsUrl ?? null, address: address ?? null }),
  getProfile: () => invoke<Settings | null>('get_profile'),
  updateProfile: (p: { displayName: string; smtpHost: string; smtpPort: number; davUrl: string; password: string | null }) =>
    invoke<Settings>('update_profile', { ...p }),
  pickAttachments: () => invoke<PickedFile[]>('pick_attachments'),
  sendMessage: (m: {
    to: string; cc: string; subject: string; body: string; html: string | null
    inReplyTo: string | null; references: string | null; attachments: string[]
    readReceipt?: boolean; delaySecs?: number; sendAt?: string | null
  }) => invoke<SendTicket>('send_message', { ...m }),
  sendWait: (id: number) => invoke<SendResult>('send_wait', { id }),
  sendCancel: (id: number) => invoke<boolean>('send_cancel', { id }),
  sendNow: (id: number) => invoke<void>('send_now', { id }),
  scheduledList: () => invoke<Scheduled[]>('scheduled_list'),
  scheduledSendNow: (folder: string, uid: number, resend = false) => invoke<SendResult>('scheduled_send_now', { folder, uid, resend }),
  scheduledCancel: (folder: string, uid: number) => invoke<void>('scheduled_cancel', { folder, uid }),
  mdnRespond: (folder: string, uid: number, send: boolean) => invoke<void>('mdn_respond', { folder, uid, send }),
  desktopPrefsGet: () => invoke<DesktopPrefs>('desktop_prefs_get'),
  desktopPrefsSet: (prefs: DesktopPrefs) => invoke<void>('desktop_prefs_set', { prefs }),
  setUnread: (count: number) => invoke<void>('set_unread', { count }),
  exchangeDiagnose: (p: { url: string; address: string; username: string; password: string }) => invoke<string>('exchange_diagnose', p),
  discoverServers: (email: string) => invoke<{ imapHost: string; imapPort: number; smtpHost: string; smtpPort: number; username: string; source: string; note: string | null }>('discover_servers', { email }),
  attachData: (name: string, data: string) => invoke<PickedFile>('attach_data', { name, data }),
  forwardAttachments: (folder: string, uid: number) => invoke<PickedFile[]>('forward_attachments', { folder, uid }),
  attachmentDragOut: (folder: string, uid: number, index: number) => invoke<void>('attachment_drag_out', { folder, uid, index }),
  /** Открыть вложение программой компьютера (копия во временном каталоге); возвращает путь копии. */
  attachmentOpen: (folder: string, uid: number, index: number) => invoke<string>('attachment_open', { folder, uid, index }),
  calendarFilePreview: (folder: string, uid: number, index: number) => invoke<CalendarFile>('calendar_file_preview', { folder, uid, index }),
  calendarFileImport: (folder: string, uid: number, index: number, calendarUrl: string, uids: string[]) =>
    invoke<CalendarImport>('calendar_file_import', { folder, uid, index, calendarUrl, uids }),
  markSpam: (folder: string, uids: number[], spam: boolean) => invoke<string>('mark_spam', { folder, uids, spam }),
  unsubscribe: (folder: string, uid: number) => invoke<{ done: boolean; openUrl: string | null; how: string }>('unsubscribe', { folder, uid }),
  snooze: (folder: string, uids: number[], until: string) => invoke<string>('snooze', { folder, uids, until }),
  unsnooze: (folder: string, uids: number[]) => invoke<void>('unsnooze', { folder, uids }),
  categoriesGet: () => invoke<Category[]>('categories_get'),
  categoriesSave: (categories: Category[]) => invoke<string | null>('categories_save', { categories }),
  setKeywords: (folder: string, uids: number[], keyword: string, value: boolean) => invoke<void>('set_keywords', { folder, uids, keyword, value }),
  setDue: (folder: string, uid: number, due: string | null) => invoke<void>('set_due', { folder, uid, due }),
  dueList: () => invoke<MessageSummary[]>('due_list'),
  templatesList: () => invoke<MailTemplate[]>('templates_list'),
  templateSave: (name: string, text: string, html: string | null) => invoke<void>('template_save', { name, text, html }),
  templateDelete: (folder: string, uid: number) => invoke<void>('template_delete', { folder, uid }),
  listContacts: () => invoke<ContactsResult>('list_contacts'),
  accountsList: () => invoke<AccountEntry[]>('accounts_list'),
  accountAdd: (p: LoginParams) => invoke<AccountEntry>('account_add', { ...p, ewsUrl: p.ewsUrl ?? null, address: p.address ?? null }),
  accountSwitch: (key: string) => invoke<LoginResult>('account_switch', { key }),
  accountRemove: (key: string) => invoke<void>('account_remove', { key }),
  accountsUnread: () => invoke<AccountUnread[]>('accounts_unread'),
  accountsPoll: () => invoke<OtherNewMail[]>('accounts_poll'),
  unifiedInbox: (limit: number) => invoke<UnifiedInbox>('unified_inbox', { limit }),
  unifiedFolder: (role: UnifiedRole, limit: number) => invoke<UnifiedInbox>('unified_folder', { role, limit }),
  searchOthers: (query: string) => invoke<UnifiedInbox>('search_others', { query }),
  viewsGet: () => invoke<SavedView[]>('views_get'),
  viewsSave: (views: SavedView[]) => invoke<void>('views_save', { views }),
  importThunderbird: () => invoke<ImportSource[]>('import_thunderbird'),
  importPickFiles: () => invoke<ImportSource | null>('import_pick_files'),
  importPickFolder: () => invoke<ImportSource | null>('import_pick_folder'),
  importStart: (source: string, root: string) => invoke<number>('import_start', { source, root }),
  jobStatus: (id: number) => invoke<JobStatus>('job_status', { id }),
  jobCancel: (id: number) => invoke<void>('job_cancel', { id }),
  backupStart: (folders: string[]) => invoke<number | null>('backup_start', { folders }),
  contactsImport: (bookUrl: string) => invoke<number | null>('contacts_import', { bookUrl }),
  appInfo: () => invoke<AppInfo>('app_info'),
  checkUpdate: () => invoke<UpdateInfo | null>('check_update'),
  updateProgress: () => invoke<{ done: number; total: number }>('update_progress'),
  installUpdate: () => invoke<InstallResult>('install_update'),
  searchContacts: (query: string) => invoke<Contact[]>('search_contacts', { query }),
  contactDetails: (href: string, id: string) => invoke<Contact>('contact_details', { href, id }),
  browseContacts: (bookUrl: string) => invoke<Contact[]>('browse_contacts', { bookUrl }),
  listEvents: (start: Date, end: Date) =>
    invoke<EventsResult>('list_events', { start: start.toISOString(), end: end.toISOString() }),
  createEvent: (calendarUrl: string, input: EventInput) => invoke<void>('create_event', { calendarUrl, input }),
  updateEvent: (href: string, input: EventInput, changeTime: boolean) =>
    invoke<void>('update_event', { href, input, changeTime }),
  deleteEvent: (href: string, etag: string | null) => invoke<void>('delete_event', { href, etag }),
  createContact: (bookUrl: string, input: ContactInput) => invoke<void>('create_contact', { bookUrl, input }),
  updateContact: (href: string, input: ContactInput) => invoke<void>('update_contact', { href, input }),
  deleteContact: (href: string, etag: string | null) => invoke<void>('delete_contact', { href, etag }),

  ncStatus: () => invoke<NcStatus>('nc_status'),
  ncConnectStart: (url: string) => invoke<string>('nc_connect_start', { url }),
  ncConnectPoll: () => invoke<NcStatus | null>('nc_connect_poll'),
  ncConnectManual: (url: string, login: string, appPassword: string) =>
    invoke<NcStatus>('nc_connect_manual', { url, login, appPassword }),
  ncDisconnect: () => invoke<NcStatus>('nc_disconnect'),
  ncOpen: (path: string) => invoke<void>('nc_open', { path }),

  deckBoards: () => invoke<DeckBoard[]>('deck_boards'),
  deckStacks: (boardId: number) => invoke<DeckStack[]>('deck_stacks', { boardId }),
  deckBoardDetails: (boardId: number) => invoke<DeckBoardDetails>('deck_board_details', { boardId }),
  deckAssign: (boardId: number, stackId: number, cardId: number, uid: string, assign: boolean) =>
    invoke<void>('deck_assign', { boardId, stackId, cardId, uid, assign }),
  deckLabel: (boardId: number, stackId: number, cardId: number, labelId: number, add: boolean) =>
    invoke<void>('deck_label', { boardId, stackId, cardId, labelId, add }),
  deckComments: (cardId: number) => invoke<DeckComment[]>('deck_comments', { cardId }),
  deckCommentAdd: (cardId: number, message: string) => invoke<void>('deck_comment_add', { cardId, message }),
  deckCommentDelete: (cardId: number, commentId: number) => invoke<void>('deck_comment_delete', { cardId, commentId }),
  deckAttachments: (boardId: number, stackId: number, cardId: number) => invoke<DeckAttachment[]>('deck_attachments', { boardId, stackId, cardId }),
  deckCreateCard: (boardId: number, stackId: number, title: string, description = '', duedate: string | null = null) =>
    invoke<void>('deck_create_card', { boardId, stackId, title, description, duedate }),
  deckUpdateCard: (boardId: number, stackId: number, cardId: number, title: string, description: string, duedate: string | null, done: boolean) =>
    invoke<void>('deck_update_card', { boardId, stackId, cardId, title, description, duedate, done }),
  deckMoveCard: (boardId: number, stackId: number, cardId: number, targetStackId: number, order: number) =>
    invoke<void>('deck_move_card', { boardId, stackId, cardId, targetStackId, order }),
  deckDeleteCard: (boardId: number, stackId: number, cardId: number) =>
    invoke<void>('deck_delete_card', { boardId, stackId, cardId }),
  deckCreateStack: (boardId: number, title: string) => invoke<void>('deck_create_stack', { boardId, title }),
  deckAttachmentAdd: (boardId: number, stackId: number, cardId: number, attachment: string) =>
    invoke<void>('deck_attachment_add', { boardId, stackId, cardId, attachment }),
  deckAttachmentDelete: (boardId: number, stackId: number, cardId: number, attachmentId: number, kind: string) =>
    invoke<void>('deck_attachment_delete', { boardId, stackId, cardId, attachmentId, kind }),
  deckArchive: (boardId: number, stackId: number, cardId: number, archive: boolean) => invoke<void>('deck_archive', { boardId, stackId, cardId, archive }),
  deckActivity: (cardId: number) => invoke<DeckActivity[]>('deck_activity', { cardId }),

  talkRooms: () => invoke<TalkRoom[]>('talk_rooms'),
  talkHistory: (token: string, before: number | null = null) => invoke<TalkChunk>('talk_history', { token, before }),
  talkPoll: (token: string, lastKnown: number) => invoke<TalkChunk>('talk_poll', { token, lastKnown }),
  talkReact: (token: string, messageId: number, emoji: string, add: boolean) => invoke<void>('talk_react', { token, messageId, emoji, add }),
  talkFavorite: (token: string, on: boolean) => invoke<void>('talk_favorite', { token, on }),
  talkParticipants: (token: string) => invoke<TalkParticipant[]>('talk_participants', { token }),
  talkSearch: (token: string, term: string) => invoke<TalkSearchHit[]>('talk_search', { token, term }),
  /** Управление беседой: add, remove, promote, demote, rename, description, read_only, public, notify, read, unread, leave, delete. */
  talkManage: (token: string, action: string, value: Record<string, unknown> = {}) => invoke<void>('talk_manage', { token, action, value }),
  talkEdit: (token: string, messageId: number, message: string) => invoke<void>('talk_edit', { token, messageId, message }),
  talkDelete: (token: string, messageId: number) => invoke<void>('talk_delete', { token, messageId }),
  talkMentions: (token: string, search: string) => invoke<TalkMention[]>('talk_mentions', { token, search }),
  ncSearchUsers: (search: string) => invoke<NcUser[]>('nc_search_users', { search }),
  talkCreate: (users: string[], name: string | null) => invoke<string>('talk_create', { users, name }),
  openLink: (url: string) => invoke<void>('open_link', { url }),
  ncFilesList: (path: string) => invoke<NcFile[]>('nc_files_list', { path }),
  ncFilesMkdir: (parent: string, name: string) => invoke<string>('nc_files_mkdir', { parent, name }),
  ncFilesUpload: (dir: string, attachments: string[]) => invoke<string[]>('nc_files_upload', { dir, attachments }),
  ncFilesDownload: (path: string) => invoke<string | null>('nc_files_download', { path }),
  ncFilesDelete: (path: string) => invoke<void>('nc_files_delete', { path }),
  ncFilesRename: (path: string, name: string) => invoke<string>('nc_files_rename', { path, name }),
  ncShares: (path: string) => invoke<NcShare[]>('nc_shares', { path }),
  ncShareLink: (path: string, password: string | null, expire: string | null) => invoke<NcShare>('nc_share_link', { path, password, expire }),
  ncShareUser: (path: string, uid: string, canEdit: boolean) => invoke<NcShare>('nc_share_user', { path, uid, canEdit }),
  ncShareDelete: (id: string) => invoke<void>('nc_share_delete', { id }),
  ncLinkFor: (path: string) => invoke<string>('nc_link_for', { path }),
  ncUploadLinks: (dir: string, attachments: string[]) => invoke<UploadedLink[]>('nc_upload_links', { dir, attachments }),
  ncSaveMailAttachment: (folder: string, uid: number, index: number, dir: string) =>
    invoke<string>('nc_save_mail_attachment', { folder, uid, index, dir }),
  talkSendFile: (token: string, attachment: string, caption: string, replyTo: number | null = null, opts: { voice?: boolean; silent?: boolean } = {}) =>
    invoke<void>('talk_send_file', { token, attachment, caption, replyTo, voice: opts.voice ?? false, silent: opts.silent ?? false }),
  talkFilePreview: (fileId: string) => invoke<string | null>('talk_file_preview', { fileId }),
  talkFileData: (path: string) => invoke<string>('talk_file_data', { path }),
  forgetAttachment: (attachment: string) => invoke<void>('forget_attachment', { attachment }),
  talkSend: (token: string, message: string, replyTo: number | null = null, silent = false) =>
    invoke<void>('talk_send', { token, message, replyTo, silent }),
  talkPollCreate: (token: string, question: string, options: string[], maxVotes: number, hidden: boolean) =>
    invoke<TalkPoll>('talk_poll_create', { token, question, options, maxVotes, hidden }),
  talkPollGet: (token: string, id: number) => invoke<TalkPoll>('talk_poll_get', { token, id }),
  talkPollVote: (token: string, id: number, options: number[]) => invoke<TalkPoll>('talk_poll_vote', { token, id, options }),
  talkPollClose: (token: string, id: number) => invoke<TalkPoll>('talk_poll_close', { token, id }),
  talkShared: (token: string, kind: string, before: number | null = null) => invoke<TalkMessage[]>('talk_shared', { token, kind, before }),
  ncLinkPreview: (url: string) => invoke<LinkPreview | null>('nc_link_preview', { url }),
  talkCallWindow: (token: string) => invoke<void>('talk_call_window', { token }),
  autoLogin: () => invoke<Account | null>('auto_login'),
  logout: (resetTotp = false) => invoke<void>('logout', { resetTotp }),
  totpStatus: () => invoke<TotpStatus>('totp_status'),
  totpSetupBegin: () => invoke<TotpSetup>('totp_setup_begin'),
  totpSetupConfirm: (code: string) => invoke<string[]>('totp_setup_confirm', { code }),
  totpDisable: (code: string) => invoke<void>('totp_disable', { code }),
  totpUnlock: (code: string) => invoke<LoginResult>('totp_unlock', { code }),
  lockApp: () => invoke<void>('lock_app'),
  cachedFolders: () => invoke<Folder[] | null>('cached_folders'),
  cachedMessages: (folder: string, beforeUid: number | null, limit: number) => invoke<MessagePage | null>('cached_messages', { folder, beforeUid, limit }),
  searchMail: (query: string) => invoke<MessageSummary[]>('search_mail', { query }),
  searchServer: (query: string) => invoke<MessageSummary[]>('search_server', { query }),
  offlineStatus: () => invoke<OfflineStatus>('offline_status'),
  setOfflineDays: (days: number) => invoke<void>('set_offline_days', { days }),
  clearOffline: () => invoke<void>('clear_offline'),
  offlineQueue: () => invoke<ConnectionStatus>('offline_queue'),
  offlineCheck: () => invoke<ConnectionStatus>('offline_check'),
  offlineResolve: (id: number, retry = false) => invoke<ConnectionStatus>('offline_resolve', { id, retry }),
  signatureGet: () => invoke<string>('signature_get'),
  signatureSave: (text: string) => invoke<string | null>('signature_save', { text }),
  listFolders: () => invoke<Folder[]>('list_folders'),
  /** `beforeUid` — UID последнего показанного письма (курсор догрузки). */
  listMessages: (folder: string, beforeUid: number | null = null, limit = PAGE_SIZE) =>
    invoke<MessagePage>('list_messages', { folder, beforeUid, limit }),
  createFolder: (name: string) => invoke<void>('create_folder', { name }),
  openMessageWindow: (folder: string, uid: number, title: string) => invoke<void>('open_message_window', { folder, uid, title }),
  pollNewMail: (folder: string, afterUid: number, limit: number) =>
    invoke<{ items: MessageSummary[]; lastUid: number }>('poll_new_mail', { folder, afterUid, limit }),
  notify: (title: string, body: string) => invoke<void>('notify', { title, body }),
  notifyPollSecs: () => invoke<number>('notify_poll_secs'),
  ncEditors: () => invoke<{ editors: { id: string; name: string }[]; creators: { id: string; editor: string; name: string; extension: string }[] }>('nc_editors'),
  ncEdit: (path: string, mime: string) => invoke<EditResult>('nc_edit', { path, mime }),
  ncCreateDocument: (dir: string, name: string, kind: 'docx' | 'xlsx' | 'pptx') => invoke<EditResult>('nc_create_document', { dir, name, kind }),
  ncEditAttachment: (folder: string, uid: number, index: number, dir: string) => invoke<EditResult>('nc_edit_attachment', { folder, uid, index, dir }),
  getPolicy: () => invoke<UiPolicy>('get_policy'),
  ncAvatar: (kind: 'user' | 'room', id: string) => invoke<string | null>('nc_avatar', { kind, id }),
  messageHtmlRemote: (folder: string, uid: number) => invoke<string>('message_html_remote', { folder, uid }),
  attachmentData: (folder: string, uid: number, index: number) => invoke<{ name: string; data: string }>('attachment_data', { folder, uid, index }),
  closeWindow: () => invoke<void>('close_window'),
  moveMessages: (folder: string, uids: number[], target: string) => invoke<void>('move_messages', { folder, uids, target }),
  /** true — письма удалены безвозвратно (корзины нет или удаляли из неё). */
  deleteMessages: (folder: string, uids: number[], trash: string | null) => invoke<boolean>('delete_messages', { folder, uids, trash }),
  mailFiltersGet: () => invoke<{ filters: SieveFilter[] }>('mail_filters_get'),
  mailFiltersSave: (filters: SieveFilter[]) => invoke<void>('mail_filters_save', { filters }),
  rulesGet: () => invoke<RuleSet>('rules_get'),
  rulesSave: (set: RuleSet) => invoke<void>('rules_save', { set }),
  counterparties: () => invoke<Counterparty[]>('counterparties'),
  counterpartyTimeline: (domain: string, server: boolean, years = 1) =>
    invoke<CounterpartyTimeline>('counterparty_timeline', { domain, server, years }),
  counterpartySave: (domain: string, dossier: Dossier) => invoke<void>('counterparty_save', { domain, dossier }),
  diagnostics: (folder: string | null) => invoke<string>('diagnostics', { folder }),
  getMessage: (folder: string, uid: number) =>
    invoke<MessageDetail>('get_message', { folder, uid }),
  setFlag: (folder: string, uid: number, flag: 'seen' | 'flagged' | 'important', value: boolean) =>
    invoke<void>('set_flag', { folder, uid, flag, value }),
  saveAttachment: (folder: string, uid: number, index: number) =>
    invoke<string | null>('save_attachment', { folder, uid, index }),
}

export function messageKey(m: Pick<MessageSummary, 'folder' | 'uid'> & { account?: string; active?: boolean }): string {
  // Письмо другой учётной записи (поиск по всем ящикам) — ключ с её именем.
  return m.account && !m.active ? `${m.account}\u0002${m.folder}\u0001${m.uid}` : `${m.folder}\u0001${m.uid}`
}

export function formatListDate(value: string | null): string {
  if (!value) return ''
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return ''
  const now = new Date()
  if (date.toDateString() === now.toDateString()) {
    return date.toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' })
  }
  const yesterday = new Date(now)
  yesterday.setDate(now.getDate() - 1)
  if (date.toDateString() === yesterday.toDateString()) return 'Вчера'
  return date.toLocaleDateString('ru-RU', {
    day: '2-digit',
    month: '2-digit',
    ...(date.getFullYear() !== now.getFullYear() ? { year: '2-digit' } : {}),
  })
}

export function formatFullDate(value: string | null): string {
  if (!value) return ''
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return ''
  return date.toLocaleString('ru-RU', {
    day: 'numeric', month: 'long', year: 'numeric', hour: '2-digit', minute: '2-digit',
  })
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} Б`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} КБ`
  return `${(bytes / 1024 / 1024).toFixed(1)} МБ`
}

/** Название сборки для «О программе». */
export const VARIANT_LABEL: Record<string, string> = {
  win: 'Windows 10/11',
  win8: 'Windows 8/8.1',
  deb: 'Linux (.deb, Astra 1.8 / Debian 12)',
  astra17: 'Astra Linux SE 1.7',
  rpm: 'Linux (.rpm)',
  appimage: 'Linux (AppImage)',
  unknown: 'сборка разработчика',
}

/** Папка в Nextcloud для файлов из почты (вложения, большие файлы по ссылке). */
export const NC_MAIL_DIR = '/Почта'

/** Кэши данных учётной записи (вложения, аватары) сбрасываются при выходе и новом входе,
 *  чтобы другой сотрудник на этом компьютере не увидел чужие данные. */
const sessionResetters = new Set<() => void>()
export function onSessionReset(fn: () => void) { sessionResetters.add(fn) }
export function resetSession() { sessionResetters.forEach((fn) => fn()) }

/** Как открылся документ: в окне программы (редактор Nextcloud) или в браузере. */
export interface EditResult { inApp: boolean; path: string }

const OFFICE_EXT = /\.(docx|xlsx|pptx|doc|xls|ppt|odt|ods|odp|rtf|csv)$/i
/** Офисный документ — его можно открыть в редакторе Nextcloud (Euro-Office / ONLYOFFICE). */
export function isOfficeFile(name: string) { return OFFICE_EXT.test(name) }

export function editNotice(r: EditResult, name: string) {
  return r.inApp
    ? `«${name}» открыт в редакторе. Изменения сохраняются в Nextcloud (${r.path}).`
    : `«${name}» открыт в браузере (редактор Nextcloud). Файл: ${r.path}.`
}

/** Политика администратора (policy.json / групповая политика Windows). */
export interface UiPolicy {
  remoteImages: 'direct' | 'proxy' | 'off'
  rememberPassword: boolean
  allowRememberPassword: boolean
  shareLinkDays: number
  /** Что администратор разрешает показывать во всплывающих уведомлениях. */
  notificationContent?: 'full' | 'sender' | 'none'
  /** Файл политики администратора повреждён или доступен на запись не только администратору. */
  policyError?: string | null
}

/** Домен ссылки и, если текст ссылки выглядит как адрес другого сайта, — этот «показанный» домен. */
export function linkMismatch(href: string, text: string): { real: string; shown: string } | null {
  let real = ''
  try { real = new URL(href).hostname.toLowerCase() } catch { return null }
  const m = text.trim().toLowerCase().match(/^(?:https?:\/\/)?(?:www\.)?([a-zа-яё0-9-]+(?:\.[a-zа-яё0-9-]+)+)/i)
  if (!m) return null
  const shown = m[1].replace(/^www\./, '')
  const r = real.replace(/^www\./, '')
  if (r === shown || r.endsWith(`.${shown}`)) return null
  return { real, shown }
}
