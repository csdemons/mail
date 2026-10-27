import { invoke } from '@tauri-apps/api/core'
import type {
  Account, Contact, ContactInput, ContactsResult, DeckBoard, DeckStack, EventInput, EventsResult, Folder, NcStatus,
  TalkChunk, TalkMention, NcUser, TalkRoom, LoginResult, MessageDetail, MessagePage,
  MessageSummary, PickedFile, SendResult, Settings, UpdateInfo, InstallResult, AppInfo,
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
    to: string; cc: string; subject: string; body: string; html: string | null
    inReplyTo: string | null; references: string | null; attachments: string[]
  }) => invoke<SendResult>('send_message', { ...m }),
  listContacts: () => invoke<ContactsResult>('list_contacts'),
  appInfo: () => invoke<AppInfo>('app_info'),
  checkUpdate: () => invoke<UpdateInfo | null>('check_update'),
  updateProgress: () => invoke<{ done: number; total: number }>('update_progress'),
  installUpdate: () => invoke<InstallResult>('install_update'),
  searchContacts: (query: string) => invoke<Contact[]>('search_contacts', { query }),
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
  deckCreateCard: (boardId: number, stackId: number, title: string, description = '', duedate: string | null = null) =>
    invoke<void>('deck_create_card', { boardId, stackId, title, description, duedate }),
  deckUpdateCard: (boardId: number, stackId: number, cardId: number, title: string, description: string, duedate: string | null, done: boolean) =>
    invoke<void>('deck_update_card', { boardId, stackId, cardId, title, description, duedate, done }),
  deckMoveCard: (boardId: number, stackId: number, cardId: number, targetStackId: number, order: number) =>
    invoke<void>('deck_move_card', { boardId, stackId, cardId, targetStackId, order }),
  deckDeleteCard: (boardId: number, stackId: number, cardId: number) =>
    invoke<void>('deck_delete_card', { boardId, stackId, cardId }),
  deckCreateStack: (boardId: number, title: string) => invoke<void>('deck_create_stack', { boardId, title }),

  talkRooms: () => invoke<TalkRoom[]>('talk_rooms'),
  talkHistory: (token: string, before: number | null = null) => invoke<TalkChunk>('talk_history', { token, before }),
  talkPoll: (token: string, lastKnown: number) => invoke<TalkChunk>('talk_poll', { token, lastKnown }),
  talkReact: (token: string, messageId: number, emoji: string, add: boolean) => invoke<void>('talk_react', { token, messageId, emoji, add }),
  talkEdit: (token: string, messageId: number, message: string) => invoke<void>('talk_edit', { token, messageId, message }),
  talkDelete: (token: string, messageId: number) => invoke<void>('talk_delete', { token, messageId }),
  talkMentions: (token: string, search: string) => invoke<TalkMention[]>('talk_mentions', { token, search }),
  ncSearchUsers: (search: string) => invoke<NcUser[]>('nc_search_users', { search }),
  talkCreate: (users: string[], name: string | null) => invoke<string>('talk_create', { users, name }),
  openLink: (url: string) => invoke<void>('open_link', { url }),
  talkSendFile: (token: string, attachment: string, caption: string, replyTo: number | null = null) =>
    invoke<void>('talk_send_file', { token, attachment, caption, replyTo }),
  forgetAttachment: (attachment: string) => invoke<void>('forget_attachment', { attachment }),
  talkSend: (token: string, message: string, replyTo: number | null = null) =>
    invoke<void>('talk_send', { token, message, replyTo }),
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
