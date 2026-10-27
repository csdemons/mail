import { invoke } from '@tauri-apps/api/core'
import type {
  Account, ContactInput, ContactsResult, EventInput, EventsResult, Folder, LoginResult, MessageDetail, MessagePage,
  MessageSummary, PickedFile, SendResult, Settings,
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
}

/** Ошибки команд приходят строкой (см. AppError в src-tauri/src/error.rs). */
export function errorText(error: unknown): string {
  if (typeof error === 'string') return error
  if (error instanceof Error) return error.message
  return 'Неизвестная ошибка'
}

export const PAGE_SIZE = 50

export const api = {
  getSettings: () => invoke<Settings | null>('get_settings'),
  login: ({ host, port, username, password, remember, smtpHost, smtpPort, davUrl }: LoginParams) =>
    invoke<LoginResult>('login', { host, port, username, password, remember, smtpHost, smtpPort, davUrl }),
  getProfile: () => invoke<Settings | null>('get_profile'),
  updateProfile: (p: { displayName: string; smtpHost: string; smtpPort: number; davUrl: string }) =>
    invoke<Settings>('update_profile', { ...p }),
  pickAttachments: () => invoke<PickedFile[]>('pick_attachments'),
  sendMessage: (m: {
    to: string; cc: string; subject: string; body: string
    inReplyTo: string | null; references: string | null; attachments: string[]
  }) => invoke<SendResult>('send_message', { ...m }),
  listContacts: () => invoke<ContactsResult>('list_contacts'),
  listEvents: (start: Date, end: Date) =>
    invoke<EventsResult>('list_events', { start: start.toISOString(), end: end.toISOString() }),
  createEvent: (calendarUrl: string, input: EventInput) => invoke<void>('create_event', { calendarUrl, input }),
  updateEvent: (href: string, input: EventInput, changeTime: boolean) =>
    invoke<void>('update_event', { href, input, changeTime }),
  deleteEvent: (href: string, etag: string | null) => invoke<void>('delete_event', { href, etag }),
  createContact: (bookUrl: string, input: ContactInput) => invoke<void>('create_contact', { bookUrl, input }),
  updateContact: (href: string, input: ContactInput) => invoke<void>('update_contact', { href, input }),
  deleteContact: (href: string, etag: string | null) => invoke<void>('delete_contact', { href, etag }),
  autoLogin: () => invoke<Account | null>('auto_login'),
  logout: () => invoke<void>('logout'),
  listFolders: () => invoke<Folder[]>('list_folders'),
  /** `beforeUid` — UID последнего показанного письма (курсор догрузки). */
  listMessages: (folder: string, beforeUid: number | null = null, limit = PAGE_SIZE) =>
    invoke<MessagePage>('list_messages', { folder, beforeUid, limit }),
  diagnostics: (folder: string | null) => invoke<string>('diagnostics', { folder }),
  getMessage: (folder: string, uid: number) =>
    invoke<MessageDetail>('get_message', { folder, uid }),
  setFlag: (folder: string, uid: number, flag: 'seen' | 'flagged', value: boolean) =>
    invoke<void>('set_flag', { folder, uid, flag, value }),
  saveAttachment: (folder: string, uid: number, index: number) =>
    invoke<string | null>('save_attachment', { folder, uid, index }),
}

export function messageKey(m: Pick<MessageSummary, 'folder' | 'uid'>): string {
  return `${m.folder}\u0001${m.uid}`
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
