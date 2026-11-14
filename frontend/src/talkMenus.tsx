// Контекстные меню чата: сообщения и беседы.
import type { RefObject } from 'react'
import {
  AlarmClock, Bell, BellOff, Copy, CornerUpLeft, Download, ExternalLink, Forward, KanbanSquare, LogOut, MailOpen, Pencil, Pin, PinOff,
  Settings2, Star, Trash2, Users, Wallpaper as WallpaperIcon, Phone, PhoneOff, CircleAlert, Lock, Archive, MessagesSquare, Languages, CalendarPlus, Sparkles,
} from 'lucide-react'

import { api, errorText } from './api'
import { QUICK_REACTIONS } from './Emoji'
import type { MenuItem } from './PopupMenu'
import { personMenu, type PersonActions } from './TalkPeople'
import type { TalkFeatures, TalkFile, TalkMessage, TalkRoom } from './types'

export interface MessageMenuActions {
  react: (m: TalkMessage, emoji: string) => Promise<void>
  setReactFor: (id: number | null) => void
  setReactFull: (full: boolean) => void
  setReplyTo: (m: TalkMessage | null) => void
  setEditing: (m: TalkMessage | null) => void
  inputRef: RefObject<HTMLTextAreaElement>
  copyText: (text: string, what?: string) => void
  setForwarding: (m: TalkMessage) => void
  setCardFrom: (m: TalkMessage) => void
  /** Закреплённые в беседе — по ним видно, закреплено ли сообщение. */
  pinned: TalkMessage[]
  setPinFor: (m: TalkMessage) => void
  unpin: (m: TalkMessage, onlyMe: boolean) => Promise<void>
  setRemindFor: (m: TalkMessage) => void
  person: PersonActions
  openFile: (f: TalkFile) => void
  saveFile: (f: TalkFile) => Promise<void>
  startEdit: (m: TalkMessage) => void
  remove: (m: TalkMessage) => Promise<void>
  /** Открыть ветку сообщения (Talk 22). */
  openThread?: (id: number) => void
  /** Перевести сообщение (ИИ-переводчик сервера). */
  translate?: (m: TalkMessage) => void
}

export interface RoomMenuActions {
  toggleFavorite: (r: TalkRoom) => Promise<void>
  manageRoom: (r: TalkRoom, action: string, value: Record<string, unknown>, done: string) => Promise<void>
  setToken: (token: string) => void
  setShowPeople: (show: boolean) => void
  setSettingsFor: (r: TalkRoom) => void
  setWallpaperRoom: (r: TalkRoom) => void
  notify: (text: string) => void
  /** Беседа покинута — убрать её черновик и выбор. */
  roomGone: (r: TalkRoom) => void
  features: TalkFeatures
  /** Окна инструментов беседы: встреча, сводка, комнаты для групп. */
  openTool: (kind: 'meeting' | 'summary' | 'breakout', r: TalkRoom) => void
}

/** Меню сообщения (правая кнопка мыши или «⋯»). */
export function messageMenu(m: TalkMessage, r: TalkRoom, a: MessageMenuActions): MenuItem[] {
  const { react, setReactFor, setReactFull, setReplyTo, setEditing, inputRef, copyText, setForwarding, setCardFrom, pinned, setPinFor, unpin, setRemindFor, person, openFile, saveFile, startEdit, remove, openThread, translate } = a
  if (m.system) return []
  const canWrite = !r.readOnly && !m.deleted
  const sel = window.getSelection()?.toString() ?? ''
  const items: MenuItem[] = []
  if (canWrite) {
    items.push({ custom: <>{QUICK_REACTIONS.map((e) => <button key={e} type="button" aria-label={`Реакция ${e}`} onClick={() => void react(m, e)}>{e}</button>)}<button type="button" aria-label="Другая реакция" title="Все эмодзи" onClick={() => { setReactFor(m.id); setReactFull(true) }}>＋</button></> })
    items.push({ label: 'Ответить', icon: <CornerUpLeft size={15} />, onClick: () => { setReplyTo(m); setEditing(null); inputRef.current?.focus() } })
  }
  if (sel.trim()) items.push({ label: 'Копировать выделенное', icon: <Copy size={15} />, onClick: () => copyText(sel) })
  if (translate && m.text && !m.deleted) items.push({ label: 'Перевести…', icon: <Languages size={15} />, onClick: () => translate(m) })
  if (m.text && !m.deleted) items.push({ label: 'Копировать текст', icon: <Copy size={15} />, onClick: () => copyText(m.text) })
  if (openThread && (m.isThread || m.threadId > 0)) items.push({ label: 'Открыть ветку', icon: <MessagesSquare size={15} />, onClick: () => openThread(m.threadId || m.id) })
  if (!m.deleted) items.push({ label: 'Переслать…', icon: <Forward size={15} />, onClick: () => setForwarding(m) })
  if (canWrite) items.push({ label: 'Создать карточку Deck…', icon: <KanbanSquare size={15} />, onClick: () => setCardFrom(m) })
  if (!m.deleted) {
    items.push({ separator: true })
    // Закреплено ли — по списку закреплённых беседы (в ленте метка обновляется не сразу).
    const isPinned = pinned.some((p) => p.id === m.id)
    if (r.moderator && !isPinned) items.push({ label: 'Закрепить…', icon: <Pin size={15} />, onClick: () => setPinFor(m) })
    if (r.moderator && isPinned) items.push({ label: 'Открепить', icon: <PinOff size={15} />, onClick: () => void unpin(m, false) })
    items.push({ label: 'Напомнить…', icon: <AlarmClock size={15} />, onClick: () => setRemindFor(m) })
  }
  if (!m.own && m.authorId && !m.system) {
    items.push({ separator: true })
    items.push(...personMenu(m.authorId, m.author, person, { inDirect: r.kind === 1 }))
  }
  const f = m.files[0]
  if (f && !m.deleted) {
    items.push({ separator: true })
    items.push({ label: m.files.length > 1 ? `Открыть «${f.name}»` : 'Открыть файл', icon: <ExternalLink size={15} />, onClick: () => openFile(f) })
    if (f.path) items.push({ label: 'Сохранить файл на компьютер…', icon: <Download size={15} />, onClick: () => void saveFile(f) })
  }
  if (canWrite && (m.editText !== null || m.own)) items.push({ separator: true })
  if (canWrite && m.editText !== null) items.push({ label: 'Изменить', icon: <Pencil size={15} />, onClick: () => startEdit(m) })
  if (canWrite && m.own) items.push({ label: 'Удалить', icon: <Trash2 size={15} />, danger: true, onClick: () => void remove(m) })
  return items
}

/** Меню беседы в списке. */
export function roomMenu(r: TalkRoom, a: RoomMenuActions): MenuItem[] {
  const { toggleFavorite, manageRoom, setToken, setShowPeople, setSettingsFor, setWallpaperRoom, notify, roomGone, features, openTool } = a
  const muted = r.notifyLevel === 3
  // Ветки, архив, «важная», «конфиденциальная» — только если сервер их знает (Talk 20–21).
  const flags: MenuItem[] = [
    { label: r.notifyCalls === 0 ? 'Сообщать о звонках' : 'Не сообщать о звонках', icon: r.notifyCalls === 0 ? <Phone size={15} /> : <PhoneOff size={15} />,
      onClick: () => void manageRoom(r, 'notify_calls', { on: r.notifyCalls === 0 }, r.notifyCalls === 0 ? 'О звонках в беседе будет сообщаться' : 'О звонках в беседе сообщаться не будет') },
    ...(features.important ? [{ label: r.important ? 'Снять отметку «Важная»' : 'Важная беседа', icon: <CircleAlert size={15} />,
      onClick: () => void manageRoom(r, 'important', { on: !r.important }, r.important ? `«${r.name}» — обычная беседа` : `«${r.name}» — важная: уведомления приходят и в режиме «Не беспокоить»`) }] : []),
    ...(features.sensitive ? [{ label: r.sensitive ? 'Снять отметку «Конфиденциальная»' : 'Конфиденциальная беседа', icon: <Lock size={15} />,
      onClick: () => void manageRoom(r, 'sensitive', { on: !r.sensitive }, r.sensitive ? `«${r.name}» — текст сообщений снова виден в списке и уведомлениях` : `«${r.name}» — текст сообщений скрыт в списке бесед и уведомлениях`) }] : []),
    ...(features.archive ? [{ label: r.archived ? 'Вернуть из архива' : 'В архив', icon: <Archive size={15} />,
      onClick: () => void manageRoom(r, 'archive', { on: !r.archived }, r.archived ? `«${r.name}» возвращена из архива` : `«${r.name}» — в архиве (внизу списка бесед)`) }] : []),
  ]
  return [
    { label: r.favorite ? 'Убрать из избранного' : 'В избранное', icon: <Star size={15} />, onClick: () => void toggleFavorite(r) },
    r.unread > 0
      ? { label: 'Отметить прочитанной', icon: <MailOpen size={15} />, onClick: () => void manageRoom(r, 'read', {}, `«${r.name}» — прочитано`) }
      : { label: 'Отметить непрочитанной', icon: <MailOpen size={15} />, onClick: () => void manageRoom(r, 'unread', {}, `«${r.name}» — непрочитано`) },
    { label: muted ? 'Включить уведомления' : 'Не уведомлять', icon: muted ? <Bell size={15} /> : <BellOff size={15} />, onClick: () => void manageRoom(r, 'notify', { level: muted ? 0 : 3 }, muted ? 'Уведомления включены' : 'Уведомления беседы отключены') },
    ...flags,
    { separator: true },
    ...(features.scheduleMeeting && r.kind !== 6 ? [{ label: 'Запланировать встречу…', icon: <CalendarPlus size={15} />, onClick: () => openTool('meeting', r) }] : []),
    ...(features.summary && r.unread > 0 ? [{ label: `Кратко о непрочитанном (${r.unread})`, icon: <Sparkles size={15} />, onClick: () => openTool('summary', r) }] : []),
    ...(features.breakout && r.moderator && (r.kind === 2 || r.kind === 3) && !r.breakoutParent ? [{ label: 'Комнаты для групп…', icon: <Users size={15} />, onClick: () => openTool('breakout', r) }] : []),
    { label: 'Участники', icon: <Users size={15} />, onClick: () => { setToken(r.token); setShowPeople(true) } },
    { label: 'Настройки беседы…', icon: <Settings2 size={15} />, onClick: () => setSettingsFor(r) },
    { label: 'Обои…', icon: <WallpaperIcon size={15} />, onClick: () => setWallpaperRoom(r) },
    { label: 'Открыть в Nextcloud', icon: <ExternalLink size={15} />, onClick: () => void api.ncOpen(`call/${r.token}`) },
    ...(r.canLeave ? [{ separator: true }, { label: 'Выйти из беседы', icon: <LogOut size={15} />, danger: true, onClick: () => { if (window.confirm(`Выйти из беседы «${r.name}»?`)) void api.talkManage(r.token, 'leave').then(() => { notify(`Вы вышли из «${r.name}»`); roomGone(r) }).catch((e) => notify(`Не удалось: ${errorText(e)}`)) } }] : []),
  ]
}
