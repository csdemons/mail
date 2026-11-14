// Контекстное меню письма в списке (одно письмо или отмеченные).
import {
  AlarmClock, Archive, CalendarClock, CircleAlert, ExternalLink, Flag, FolderInput, Forward, ListFilter, Mail, MailOpen,
  Printer, Reply, ReplyAll, ShieldAlert, ShieldCheck, SquareKanban, Trash2,
} from 'lucide-react'

import { api, errorText, messageKey } from './api'
import type { MenuItem } from './ContextMenu'
import { userCategories } from './mailExtras'
import { printMessage, snoozePresets, snoozedUntil } from './mailTools'
import type { Category, Folder, MessageDetail, MessageSummary, NcStatus } from './types'

/** Что нужно меню письма от главного окна. */
export interface RowMenuActions {
  toggleFlag: (m: MessageSummary, flag: 'seen' | 'flagged' | 'important') => Promise<void>
  openMessage: (m: MessageSummary) => Promise<void>
  openInWindow: (m: MessageSummary) => void
  composeFor: (m: MessageSummary, mode: 'reply' | 'replyAll' | 'forward') => Promise<void>
  ncStatus: NcStatus | null
  setTaskFor: (m: MessageSummary) => void
  setDuePick: (items: MessageSummary[]) => void
  unsnoozeItems: (items: MessageSummary[]) => Promise<void>
  snoozeItems: (items: MessageSummary[], until: Date) => Promise<void>
  setSnoozePick: (items: MessageSummary[]) => void
  categories: Category[]
  setCategory: (items: MessageSummary[], cat: Category, want: boolean) => Promise<void>
  archive: (items: MessageSummary[]) => Promise<void>
  setMovePicker: (items: MessageSummary[]) => void
  setFilterSeed: (m: MessageSummary) => void
  setShowFilters: (open: boolean) => void
  folders: Folder[]
  markSpam: (items: MessageSummary[], spam: boolean) => Promise<void>
  detail: MessageDetail | null
  setNotice: (text: string) => void
  requestDelete: (items: MessageSummary[]) => void
}

export function rowMenuItems(items: MessageSummary[], a: RowMenuActions): MenuItem[] {
  const { toggleFlag, openMessage, openInWindow, composeFor, ncStatus, setTaskFor, setDuePick, unsnoozeItems, snoozeItems, setSnoozePick, categories, setCategory, archive, setMovePicker, setFilterSeed, setShowFilters, folders, markSpam, detail, setNotice, requestDelete } = a
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
      ...(ncStatus?.connected ? [{ label: 'Создать задачу…', icon: <SquareKanban size={15} />, onSelect: () => setTaskFor(one) }] : []),
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
    { label: 'Флажок со сроком…', icon: <CalendarClock size={15} />, onSelect: () => setDuePick(items) },
    items.some((m) => snoozedUntil(m.keywords))
      ? { label: 'Вернуть во «Входящие» сейчас', icon: <AlarmClock size={15} />, onSelect: () => void unsnoozeItems(items) }
      : { label: 'Отложить до завтра утром', icon: <AlarmClock size={15} />, onSelect: () => void snoozeItems(items, snoozePresets().find((p) => p.label === 'Завтра утром')!.at) },
    { label: 'Отложить до…', icon: <AlarmClock size={15} />, onSelect: () => setSnoozePick(items) },
    { separator: true },
    ...userCategories(categories).map((c) => {
      const has = items.every((m) => (m.keywords ?? []).some((k) => k.toLowerCase() === c.keyword.toLowerCase()))
      return { label: `Категория «${c.name}»`, icon: <span className="cat-dot" style={{ background: c.color }}>{has ? '✓' : ''}</span>, onSelect: () => void setCategory(items, c, !has) }
    }),
    { separator: true },
    { label: 'В архив', icon: <Archive size={15} />, onSelect: () => void archive(items) },
    { label: 'Переместить в папку…', icon: <FolderInput size={15} />, onSelect: () => setMovePicker(items) },
  )
  if (one) list.push({ label: 'Создать правило по отправителю…', icon: <ListFilter size={15} />, onSelect: () => { setFilterSeed(one); setShowFilters(true) } })
  const inJunk = folders.find((f) => f.id === items[0]?.folder)?.role === 'junk'
  list.push(inJunk
    ? { label: 'Не спам', icon: <ShieldCheck size={15} />, onSelect: () => void markSpam(items, false) }
    : { label: 'Это спам', icon: <ShieldAlert size={15} />, onSelect: () => void markSpam(items, true) })
  if (one) list.push({ label: 'Печать…', icon: <Printer size={15} />, onSelect: () => { void (detail && messageKey(detail) === messageKey(one) ? Promise.resolve(detail) : api.getMessage(one.folder, one.uid)).then(printMessage).catch((e) => setNotice(errorText(e))) } })
  list.push(
    { separator: true },
    { label: items.length > 1 ? `Удалить (${items.length})` : 'Удалить', icon: <Trash2 size={15} />, hint: 'Del', danger: true, onSelect: () => requestDelete(items) },
  )
  return list
}
