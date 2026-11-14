// Пометки писем: прочитано/флажок/важное, категории, срок, «Отложить», спам — и меню для них.
import { AlarmClock, CalendarClock, Flag, Tag } from 'lucide-react'

import { api, errorText, messageKey } from './api'
import type { MenuItem } from './ContextMenu'
import { dueLabel, duePresets, userCategories, withDue } from './mailExtras'
import { byFolder } from './mailLabels'
import { snoozePresets } from './mailTools'
import type { Category, MessageSummary } from './types'

export function useMessageFlags(opts: {
  /** Изменить письмо в списке, найденных и открытом. */
  patchMessage: (key: string, patch: Partial<MessageSummary>) => void
  /** Поправить счётчик непрочитанных папки. */
  adjustUnseen: (folder: string, delta: number) => void
  /** Письма ушли в другую папку — убрать из списка. */
  dropFromList: (items: MessageSummary[], target: string | null) => void
  categories: Category[]
  setDuePick: (items: MessageSummary[]) => void
  setSnoozePick: (items: MessageSummary[]) => void
  openSettings: () => void
  setNotice: (text: string) => void
}) {
  const { patchMessage, adjustUnseen, dropFromList, categories, setDuePick, setSnoozePick, openSettings, setNotice } = opts

  async function toggleFlag(message: MessageSummary, flag: 'seen' | 'flagged' | 'important') {
    const key = messageKey(message)
    const field = flag === 'seen' ? 'unread' : flag
    const before = message[field]
    const after = !before
    patchMessage(key, { [field]: after })
    if (flag === 'seen') adjustUnseen(message.folder, after ? +1 : -1)
    try {
      // unread=true ⇔ флаг \Seen снят
      await api.setFlag(message.folder, message.uid, flag, flag === 'seen' ? !after : after)
    } catch (error) {
      patchMessage(key, { [field]: before })
      if (flag === 'seen') adjustUnseen(message.folder, after ? -1 : +1)
      setNotice(`Не удалось изменить письмо: ${errorText(error)}`)
    }
  }

  /** Категория у писем (по папкам): поставить или снять. */
  async function setCategory(items: MessageSummary[], cat: Category, want: boolean) {
    const target = items.filter((m) => (m.keywords ?? []).some((k) => k.toLowerCase() === cat.keyword.toLowerCase()) !== want)
    for (const m of target) {
      const kw = (m.keywords ?? []).filter((k) => k.toLowerCase() !== cat.keyword.toLowerCase())
      patchMessage(messageKey(m), { keywords: want ? [...kw, cat.keyword] : kw })
    }
    for (const g of byFolder(target)) {
      try {
        await api.setKeywords(g[0].folder, g.map((m) => m.uid), cat.keyword, want)
      } catch (error) {
        setNotice(`Не удалось изменить категорию: ${errorText(error)}`)
      }
    }
  }

  /** Флажок со сроком (или без срока) — напоминание придёт в срок. */
  async function setDueFor(items: MessageSummary[], due: Date | null) {
    for (const m of items) {
      patchMessage(messageKey(m), { flagged: true, keywords: withDue(m.keywords ?? [], due) })
      try {
        await api.setDue(m.folder, m.uid, due ? due.toISOString() : null)
      } catch (error) {
        setNotice(`Не удалось поставить срок: ${errorText(error)}`)
      }
    }
    if (due) setNotice(`Флажок до ${dueLabel(due)} — напомню в срок.`)
  }

  function dueMenuItems(items: MessageSummary[]): MenuItem[] {
    return [
      ...duePresets().map((p) => ({ label: `К исполнению: ${p.label.toLowerCase()}`, icon: <Flag size={15} />, hint: dueLabel(p.at), onSelect: () => void setDueFor(items, p.at) })),
      { label: 'Выбрать срок…', icon: <CalendarClock size={15} />, onSelect: () => setDuePick(items) },
      { label: 'Флажок без срока', icon: <Flag size={15} />, onSelect: () => void setDueFor(items, null) },
    ]
  }

  function categoryMenuItems(items: MessageSummary[]): MenuItem[] {
    const cats = userCategories(categories)
    return [
      ...cats.map((c) => {
        const has = items.every((m) => (m.keywords ?? []).some((k) => k.toLowerCase() === c.keyword.toLowerCase()))
        return { label: c.name, icon: <span className="cat-dot" style={{ background: c.color }}>{has ? '✓' : ''}</span>, onSelect: () => void setCategory(items, c, !has) }
      }),
      ...(cats.length ? [{ separator: true as const }] : []),
      { label: 'Настроить категории…', icon: <Tag size={15} />, onSelect: openSettings },
    ]
  }

  /** «Это спам» / «Не спам» (в папке «Спам»). */
  async function markSpam(items: MessageSummary[], spam: boolean) {
    for (const g of byFolder(items)) {
      try {
        const target = await api.markSpam(g[0].folder, g.map((m) => m.uid), spam)
        dropFromList(g, target)
        setNotice(spam ? `${g.length === 1 ? 'Письмо перенесено' : `Писем: ${g.length} —`} в «Спам».` : 'Письмо возвращено во «Входящие».')
      } catch (e) {
        setNotice(`Не удалось: ${errorText(e)}`)
      }
    }
  }

  /** «Отложить»: письмо вернётся во «Входящие» в выбранное время. */
  async function snoozeItems(items: MessageSummary[], until: Date) {
    for (const g of byFolder(items)) {
      try {
        const target = await api.snooze(g[0].folder, g.map((m) => m.uid), until.toISOString())
        dropFromList(g, target)
        setNotice(`Отложено до ${dueLabel(until)} — письмо вернётся во «Входящие».`)
      } catch (e) {
        setNotice(`Не удалось отложить: ${errorText(e)}`)
      }
    }
  }

  async function unsnoozeItems(items: MessageSummary[]) {
    for (const g of byFolder(items)) {
      try {
        await api.unsnooze(g[0].folder, g.map((m) => m.uid))
        dropFromList(g, 'INBOX')
        setNotice('Письмо возвращено во «Входящие».')
      } catch (e) {
        setNotice(`Не удалось: ${errorText(e)}`)
      }
    }
  }

  function snoozeMenuItems(items: MessageSummary[]): MenuItem[] {
    return [
      ...snoozePresets().map((p) => ({ label: p.label, icon: <AlarmClock size={15} />, hint: dueLabel(p.at), onSelect: () => void snoozeItems(items, p.at) })),
      { label: 'Выбрать дату и время…', icon: <CalendarClock size={15} />, onSelect: () => setSnoozePick(items) },
    ]
  }

  return { toggleFlag, setCategory, setDueFor, dueMenuItems, categoryMenuItems, markSpam, snoozeItems, unsnoozeItems, snoozeMenuItems }
}
