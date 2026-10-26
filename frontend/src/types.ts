export type MailFilter = 'Все' | 'Непрочитанные' | 'С вложениями' | 'Помеченные'

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
  depth: number
  role: FolderRole
  selectable: boolean
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
  bodyText: string
  attachments: AttachmentInfo[]
}
