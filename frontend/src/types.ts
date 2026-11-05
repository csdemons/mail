export type MailFilter = 'Все' | 'Непрочитанные' | 'Важные' | 'С флажком' | 'С вложениями'

export type FolderRole = 'inbox' | 'drafts' | 'sent' | 'archive' | 'junk' | 'trash' | 'other'

export interface Account {
  username: string
  host: string
  port: number
  /** Программа заблокирована: нужен код TOTP. */
  locked?: boolean
  /** Сеанс открыт без связи с сервером (письма с компьютера). */
  offline?: boolean
  /** Адрес почты (у Exchange логин может быть DOMAIN\user). */
  email?: string
  exchange?: boolean
}

export interface OfflineStatus {
  enabled: boolean
  allowed: boolean
  days: number
  maxDays: number
  messages: number
  cached: number
  syncing: boolean
  bytes: number
  /** Почему хранилище не открылось (например, недоступно хранилище паролей). */
  error: string | null
}

export interface TotpStatus {
  enabled: boolean
  keyringAvailable: boolean
  recoveryLeft: number
}

export interface TotpSetup {
  secret: string
  uri: string
  qrSvg: string
}

export interface Settings {
  host: string
  port: number
  username: string
  smtpHost: string
  smtpPort: number
  davUrl: string
  displayName: string
  /** Exchange: адрес EWS (иначе нет). */
  ewsUrl?: string
  /** Exchange: адрес почты, если логин — DOMAIN\user. */
  address?: string
}

export interface LoginResult {
  account: Account
  warning: string | null
}

export interface Folder {
  /** Имя на сервере (modified UTF-7), передаётся в команды. */
  id: string
  name: string
  path: string
  parent: string | null
  depth: number
  role: FolderRole
  /** Служебная папка своего ящика верхнего уровня — подписывается стандартно. */
  system: boolean
  selectable: boolean
  hasChildren: boolean
  total: number
  unseen: number
}

export interface MessageSummary {
  folder: string
  uid: number
  fromName: string
  fromAddress: string
  to: string[]
  subject: string
  date: string | null
  unread: boolean
  flagged: boolean
  /** «Важное» (метка $label1, как в веб-почте SOGo). */
  important: boolean
  answered: boolean
  /** Ключевые слова IMAP: категории, $MDNSent и т. п. */
  keywords: string[]
  /** Message-ID и предки (References, In-Reply-To) — для цепочек. */
  msgId?: string
  thread?: string[]
  hasAttachments: boolean
  size: number
}

export interface MessagePage {
  items: MessageSummary[]
  total: number
  hasMore: boolean
}

export interface AttachmentInfo {
  index: number
  name: string
  contentType: string
  size: number
}

export interface MessageDetail extends MessageSummary {
  cc: string[]
  messageId: string | null
  references: string | null
  replyTo: string | null
  bodyText: string
  /** Очищенный HTML письма (встроенные картинки уже внутри) или null для текстовых писем. */
  bodyHtml: string | null
  /** Сколько картинок из интернета скрыто. */
  remoteImages: number
  attachments: AttachmentInfo[]
  /** Отправитель просит уведомление о прочтении. */
  dispositionTo: string | null
  /** Рассылка: как отписаться. */
  unsubscribe: { https: string | null; mailto: string | null; oneClick: boolean } | null
}

export interface PickedFile {
  id: string
  name: string
  size: number
}

export interface SendResult {
  savedTo: string | null
  warning: string | null
  /** Нет связи — письмо в очереди исходящих. */
  queued?: boolean
}

export interface SendTicket {
  /** Письмо в «окне отмены». */
  id: number | null
  result: SendResult | null
  /** Отложенная отправка: время (RFC 3339). */
  scheduledAt: string | null
}

export interface Scheduled {
  folder: string
  uid: number
  sendAt: string
  subject: string
  to: string[]
  /** pending — ждёт; sending — отправляется другой копией; unknown — неизвестно, ушло ли; sent — ушло, убирается из «Черновиков». */
  status: 'pending' | 'sending' | 'unknown' | 'sent'
}

export interface Category {
  keyword: string
  name: string
  color: string
}

export interface MailTemplate {
  folder: string
  uid: number
  name: string
  text: string
  html: string | null
}

export interface DesktopPrefs {
  closeToTray: boolean
  autostart: boolean
  tray: boolean
}

export interface DavCollection {
  url: string
  name: string
  color: string | null
  writable: boolean
}

export interface Contact {
  id: string
  book: string
  href: string
  etag: string | null
  editable: boolean
  fullName: string
  emails: string[]
  phones: string[]
  org: string | null
  title: string | null
  note: string | null
  address: string | null
}

export interface ContactsResult {
  books: DavCollection[]
  contacts: Contact[]
  errors: string[]
}

export interface CalEvent {
  id: string
  uid: string
  href: string
  etag: string | null
  editable: boolean
  calendarUrl: string
  calendar: string
  color: string | null
  title: string
  location: string | null
  description: string | null
  organizer: string | null
  /** Адреса организатора и участников (для истории контрагента) */
  participants?: string[]
  /** ISO 8601 (UTC) или YYYY-MM-DD для событий на весь день */
  start: string
  end: string
  allDay: boolean
  recurring: boolean
}

export interface EventsResult {
  calendars: DavCollection[]
  events: CalEvent[]
  errors: string[]
}

export interface Draft {
  mode: 'new' | 'reply' | 'replyAll' | 'forward'
  to: string
  cc: string
  subject: string
  body: string
  inReplyTo: string | null
  references: string | null
  /** Возврат в редактор после отмены отправки: оформленный текст, вложения, запрос о прочтении. */
  html?: string
  files?: PickedFile[]
  readReceipt?: boolean
}

export interface EventInput {
  title: string
  location: string | null
  description: string | null
  /** ISO 8601 или YYYY-MM-DD для «весь день» */
  start: string
  /** Для «весь день» — день после последнего */
  end: string
  allDay: boolean
}

export interface ContactInput {
  fullName: string
  emails: string[]
  phones: string[]
  org: string | null
  title: string | null
  note: string | null
}

export interface NcStatus {
  url: string
  connected: boolean
  login: string
  /** ID пользователя Nextcloud. */
  uid: string
}

export interface DeckBoard {
  id: number
  title: string
  color: string
  archived: boolean
  canEdit: boolean
  canManage: boolean
  owner: string
}

export interface DeckLabel {
  id: number
  title: string
  color: string
}

export interface DeckCard {
  id: number
  stackId: number
  title: string
  description: string
  order: number
  duedate: string | null
  done: boolean
  archived: boolean
  labels: DeckLabel[]
  assigned: DeckUser[]
  owner: string
  commentsUnread: number
  attachmentCount: number
  createdAt: number
  lastModified: number
}

export interface DeckStack {
  id: number
  title: string
  order: number
  cards: DeckCard[]
}

export interface TalkRoom {
  token: string
  name: string
  kind: number
  unread: number
  unreadMention: boolean
  lastActivity: number
  lastMessage: string | null
  readOnly: boolean
  /** Личная беседа: ID собеседника. */
  peer: string | null
  /** Избранная — показывается выше остальных. */
  favorite: boolean
  /** До какого сообщения прочитали все участники (0 — неизвестно). */
  lastCommonRead: number
}

export interface TalkMessage {
  id: number
  author: string
  authorId: string
  own: boolean
  timestamp: number
  text: string
  system: boolean
  parentText: string | null
  parentAuthor: string | null
  files: TalkFile[]
  reactions: TalkReaction[]
  edited: boolean
  deleted: boolean
  editText: string | null
  update: boolean
}

export interface TalkReaction {
  emoji: string
  count: number
  own: boolean
}

export interface TalkChunk {
  messages: TalkMessage[]
  lastId: number
  lastCommonRead: number
  /** История: есть ли, вероятно, более ранние сообщения. */
  more: boolean
}

export interface TalkMention {
  insert: string
  label: string
  source: string
}

export interface NcUser {
  id: string
  label: string
  sub: string
}

export interface TalkFile {
  id: string
  name: string
  size: number
  mimetype: string
  /** Путь у текущего пользователя (обычно «Talk/имя»); null — старый сервер, только «Открыть в Nextcloud». */
  path: string | null
  preview: boolean
}

export interface UpdateInfo {
  current: string
  version: string
  notes: string
  size: number
  fileName: string
  variant: string
}

export interface InstallResult {
  restarting: boolean
  manual: string | null
}

export interface AppInfo {
  version: string
  variant: string
  os: string
}

export interface DeckUser {
  uid: string
  name: string
}

export interface DeckBoardDetails {
  labels: DeckLabel[]
  users: DeckUser[]
}

export interface DeckComment {
  id: number
  author: string
  authorId: string
  own: boolean
  message: string
  created: string
}

export interface DeckAttachment {
  id: number
  name: string
  size: number
  mimetype: string
  fileId: number | null
  kind: string
}

export interface DeckActivity { id: number; author: string; authorId: string; subject: string; date: string }

export interface NcFile {
  path: string
  name: string
  isDir: boolean
  size: number
  modified: string
  mime: string
  fileId: number | null
  shared: boolean
  favorite: boolean
  canWrite: boolean
}

export interface NcShare {
  id: string
  /** 0 — сотрудник, 1 — группа, 3 — публичная ссылка, 4 — e-mail, 10 — Talk */
  shareType: number
  shareWith: string
  shareWithName: string
  url: string
  expiration: string
  hasPassword: boolean
  canEdit: boolean
}

export interface UploadedLink {
  name: string
  path: string
  url: string
}

/** Правило фильтра почты в формате SOGo (SOGoSieveFilters). */
export interface SieveRule { field: string; operator: string; value: string; custom_header?: string }
export interface SieveAction { method: string; argument?: string }
/** Правила, которые хранит и выполняет программа (см. rules.rs). */
export interface RuleSet { program: SieveFilter[]; ext: Record<string, SieveAction[]> }
export interface SieveFilter {
  name: string
  active: number | boolean | string
  match: 'all' | 'any' | 'allmessages'
  rules: SieveRule[]
  actions: SieveAction[]
  [extra: string]: unknown
}

/** Источник импорта почты (файлы остаются в программе, сюда — только описание). */
export interface ImportSource {
  id: string
  title: string
  folders: string[]
  folderCount: number
  bytes: number
  outlook: boolean
}

/** Ход фоновой задачи: импорт, резервная копия, загрузка контактов. */
export interface JobStatus {
  kind: 'import' | 'backup' | 'contacts'
  phase: string
  done: number
  total: number
  imported: number
  duplicates: number
  skipped: number
  finished: boolean
  cancelled: boolean
  error: string | null
  summary: string | null
  notes: string[]
}

/** Подключённая учётная запись (несколько ящиков). */
export interface AccountEntry {
  key: string
  username: string
  host: string
  port: number
  displayName: string
  active: boolean
  canSwitch: boolean
  email?: string
  exchange?: boolean
}

export interface AccountUnread { key: string; unread: number | null; error: string | null }
export interface OtherNewMail { key: string; label: string; items: MessageSummary[] }
export type UnifiedItem = MessageSummary & { account: string; label: string; active: boolean }
export interface UnifiedInbox { items: UnifiedItem[]; errors: string[] }
/** Сохранённый поиск (быстрое представление): запрос с операторами и где искать. */
export interface SavedView { id: string; name: string; query: string; scope: 'current' | 'all' }
/** Общие папки нескольких учётных записей. */
export type UnifiedRole = 'inbox' | 'sent' | 'drafts' | 'archive' | 'junk' | 'trash'
export type UnifiedMark = 'all' | 'unread' | 'important' | 'flagged' | 'attachments'
/** Фильтры общей папки: ящик, подразделение (домен адреса), отправитель/получатель, важность. */
export interface UnifiedFilters { account: string; domain: string; person: string; mark: UnifiedMark }

/** Организация-контрагент (по домену адреса), см. counterparty.rs. */
export interface Counterparty {
  domain: string
  name: string | null
  messages: number
  incoming: number
  outgoing: number
  last: string | null
  own: boolean
  openAgreements: number
}
export interface Agreement {
  id: string
  /** ГГГГ-ММ-ДД */
  date: string
  text: string
  due?: string
  done: boolean
  folder?: string
  uid?: number
  subject?: string
}
export interface Dossier { name?: string | null; note?: string; agreements?: Agreement[] }
export interface CounterpartyPerson { address: string; name: string; messages: number; last: string | null }
export interface CounterpartyTimeline {
  domain: string
  own: boolean
  messages: MessageSummary[]
  events: CalEvent[]
  contacts: Contact[]
  people: CounterpartyPerson[]
  dossier: Dossier
  server: boolean
  eventsFrom: string
  errors: string[]
}

/** Работа без сети: связь и очередь действий (см. offline.rs). */
export interface QueueItem { id: number; created: string; what: string; send: boolean; error: string | null }
export interface QueueConflict { id: number; when: string; what: string; reason: string; canRetry: boolean }
export interface ConnectionStatus {
  online: boolean
  syncing: boolean
  available: boolean
  offlineSince: string | null
  lastError: string | null
  items: QueueItem[]
  conflicts: QueueConflict[]
}

/** Файл календаря (.ics) из письма. */
export interface IcsEvent { uid: string; title: string; start: string; end: string; allDay: boolean; location: string | null; recurring: boolean }
export interface CalendarFile { events: IcsEvent[]; calendars: DavCollection[] }
export interface CalendarImport { added: number; updated: number; errors: string[] }
