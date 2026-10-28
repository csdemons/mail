export type MailFilter = 'Все' | 'Непрочитанные' | 'Важные' | 'С флажком' | 'С вложениями'

export type FolderRole = 'inbox' | 'drafts' | 'sent' | 'archive' | 'junk' | 'trash' | 'other'

export interface Account {
  username: string
  host: string
  port: number
}

export interface Settings {
  host: string
  port: number
  username: string
  smtpHost: string
  smtpPort: number
  davUrl: string
  displayName: string
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
}

export interface PickedFile {
  id: string
  name: string
  size: number
}

export interface SendResult {
  savedTo: string | null
  warning: string | null
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
}

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
export interface SieveFilter {
  name: string
  active: number | boolean | string
  match: 'all' | 'any' | 'allmessages'
  rules: SieveRule[]
  actions: SieveAction[]
  [extra: string]: unknown
}
