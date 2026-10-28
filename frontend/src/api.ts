import { invoke } from '@tauri-apps/api/core'
import type {
  Account, Contact, NcFile, NcShare, UploadedLink, DeckAttachment, DeckBoardDetails, DeckComment, ContactInput, ContactsResult, DeckBoard, DeckStack, EventInput, EventsResult, Folder, NcStatus,
  SieveFilter, TalkChunk, TalkMention, NcUser, TalkRoom, LoginResult, MessageDetail, MessagePage,
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
  createFolder: (name: string) => invoke<void>('create_folder', { name }),
  openMessageWindow: (folder: string, uid: number, title: string) => invoke<void>('open_message_window', { folder, uid, title }),
  ncAvatar: (kind: 'user' | 'room', id: string) => invoke<string | null>('nc_avatar', { kind, id }),
  messageHtmlRemote: (folder: string, uid: number) => invoke<string>('message_html_remote', { folder, uid }),
  attachmentData: (folder: string, uid: number, index: number) => invoke<{ name: string; data: string }>('attachment_data', { folder, uid, index }),
  closeWindow: () => invoke<void>('close_window'),
  moveMessages: (folder: string, uids: number[], target: string) => invoke<void>('move_messages', { folder, uids, target }),
  /** true — письма удалены безвозвратно (корзины нет или удаляли из неё). */
  deleteMessages: (folder: string, uids: number[], trash: string | null) => invoke<boolean>('delete_messages', { folder, uids, trash }),
  mailFiltersGet: () => invoke<{ filters: SieveFilter[] }>('mail_filters_get'),
  mailFiltersSave: (filters: SieveFilter[]) => invoke<void>('mail_filters_save', { filters }),
  diagnostics: (folder: string | null) => invoke<string>('diagnostics', { folder }),
  getMessage: (folder: string, uid: number) =>
    invoke<MessageDetail>('get_message', { folder, uid }),
  setFlag: (folder: string, uid: number, flag: 'seen' | 'flagged' | 'important', value: boolean) =>
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

/** Папка в Nextcloud для файлов из почты (вложения, большие файлы по ссылке). */
export const NC_MAIL_DIR = '/Почта'

/** Кэши данных учётной записи (вложения, аватары) сбрасываются при выходе и новом входе,
 *  чтобы другой сотрудник на этом компьютере не увидел чужие данные. */
const sessionResetters = new Set<() => void>()
export function onSessionReset(fn: () => void) { sessionResetters.add(fn) }
export function resetSession() { sessionResetters.forEach((fn) => fn()) }
