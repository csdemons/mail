// Подписи и мелкие помощники вида почты: названия папок, отправители, раскрытые папки.
import { Archive, FilePenLine, Folder as FolderIcon, Inbox, Send, ShieldCheck, Trash2 } from 'lucide-react'

import type { Folder, FolderRole, MessageSummary, SavedView } from './types'

export const ROLE_LABELS: Partial<Record<FolderRole, string>> = {
  inbox: 'Входящие',
  drafts: 'Черновики',
  sent: 'Отправленные',
  archive: 'Архив',
  junk: 'Спам',
  trash: 'Корзина',
}

/** Быстрые представления — готовые поиски по всем ящикам. */
export const QUICK_VIEWS: SavedView[] = [
  { id: 'q-unread', name: 'Непрочитанные', query: 'непрочитанные', scope: 'all' },
  { id: 'q-important', name: 'Важные', query: 'важные', scope: 'all' },
  { id: 'q-files', name: 'С вложениями за неделю', query: 'есть:вложение после:7д', scope: 'all' },
]

export const SEARCH_HELP = [
  'Поиск: слова ищутся в отправителе, получателях, теме, именах вложений и тексте писем.',
  'от:петрова  кому:иванов  тема:"план работ"  вложение:xlsx  есть:вложение',
  'непрочитанные  важные  флажок  отвеченные',
  'после:2026-10-01  до:01.12.2026  после:7д  после:вчера',
  'папка:входящие  ящик:noc  подразделение:ihep.ru',
].join('\n')

export const ROLE_ICONS: Record<FolderRole, typeof Inbox> = {
  inbox: Inbox,
  drafts: FilePenLine,
  sent: Send,
  archive: Archive,
  junk: ShieldCheck,
  trash: Trash2,
  other: FolderIcon,
}

export function folderLabel(folder: Folder): string {
  if (folder.system) return ROLE_LABELS[folder.role] ?? folder.name
  return folder.name.toUpperCase() === 'INBOX' ? 'Входящие' : folder.name
}

export const EXPANDED_KEY = 'rrcki-mail.expanded-folders'

export function loadExpanded(): Set<string> {
  try {
    const raw = localStorage.getItem(EXPANDED_KEY)
    if (raw) return new Set(JSON.parse(raw) as string[])
  } catch { /* хранилище недоступно — используем значение по умолчанию */ }
  return new Set(['INBOX', 'shared'])
}

export function saveExpanded(ids: Set<string>) {
  try { localStorage.setItem(EXPANDED_KEY, JSON.stringify([...ids])) } catch { /* не критично */ }
}

export function senderLabel(m: MessageSummary): string {
  return m.fromName || m.fromAddress || '(без отправителя)'
}

/** Отправленные и черновики: в списке важнее, кому письмо, а не от кого (от вас). */
export function recipientsLabel(m: MessageSummary): string {
  const names = m.to.map((t) => {
    const lt = t.indexOf('<')
    const name = lt > 0 ? t.slice(0, lt).trim().replace(/^"|"$/g, '') : ''
    return name || (lt >= 0 ? t.slice(lt + 1).replace(/>.*$/, '') : t).trim()
  }).filter(Boolean)
  if (names.length === 0) return 'Кому: (без получателей)'
  return `Кому: ${names.slice(0, 3).join(', ')}${names.length > 3 ? ` и ещё ${names.length - 3}` : ''}`
}

export function pluralLetters(n: number): string {
  const mod10 = n % 10
  const mod100 = n % 100
  if (mod10 === 1 && mod100 !== 11) return 'письмо'
  if (mod10 >= 2 && mod10 <= 4 && (mod100 < 12 || mod100 > 14)) return 'письма'
  return 'писем'
}

export function initials(value: string) {
  return value.trim().slice(0, 1).toUpperCase() || '?'
}

/** Группа списка по дате письма. */
export function dateGroup(value: string | null): string {
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

/** Письма из найденных могут быть из разных папок — операции выполняются по папкам. */
export function byFolder(items: MessageSummary[]): MessageSummary[][] {
  const groups = new Map<string, MessageSummary[]>()
  for (const m of items) groups.set(m.folder, [...(groups.get(m.folder) ?? []), m])
  return [...groups.values()]
}
