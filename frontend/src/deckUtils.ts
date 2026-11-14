// Помощники Deck: настройки вида и группы досок, фильтр карточек, сроки.
import type { DeckCard } from './types'

/** Цвета новой доски (как в Deck). */
export const BOARD_COLORS = ['0082c9', '31cc7c', 'e9322d', 'f1db50', 'a463bf', 'ff7f0e', '7f8c8d']

/** Ручные группы досок (id доски → название группы; '' — «без группы») и свёрнутые группы. */
export const GROUPS_KEY = 'atom.deck.groups'
export const FOLDED_KEY = 'atom.deck.folded'
export function loadJson<T>(key: string, fallback: T): T {
  try { return (JSON.parse(localStorage.getItem(key) ?? 'null') as T | null) ?? fallback } catch { return fallback }
}
export function saveJson(key: string, v: unknown) {
  try { localStorage.setItem(key, JSON.stringify(v)) } catch { /* без хранилища — до перезапуска */ }
}

/** Масштаб колонок и карточек Deck и растяжение по ширине окна — настройка рабочего места. */
export const DECK_SCALES = [0.85, 1, 1.15, 1.3, 1.5]
export function loadDeckView(): { scale: number; stretch: boolean; nest?: boolean } {
  try {
    const v = JSON.parse(localStorage.getItem('atom.deck.view') ?? '{}') as { scale?: number; stretch?: boolean; nest?: boolean }
    return { scale: DECK_SCALES.includes(Number(v.scale)) ? Number(v.scale) : 1, stretch: v.stretch !== false, ...(v.nest === false ? { nest: false } : {}) }
  } catch {
    return { scale: 1, stretch: true }
  }
}

export const LAST_BOARD_KEY = 'rrcki-mail.deck-board'
export const FILTER_KEY = 'rrcki-mail.deck-filter.'

/** Фильтр карточек, как в веб-Deck: исполнитель, метка, срок, выполненные, текст. */
export interface CardFilter {
  /** 'all' | 'me' | 'none' | 'u:<uid>' */
  who: string
  label: number
  due: 'all' | 'overdue' | 'today' | 'week' | 'none'
  hideDone: boolean
  q: string
}
export const NO_FILTER: CardFilter = { who: 'all', label: 0, due: 'all', hideDone: false, q: '' }

export function loadFilter(boardId: number | null): CardFilter {
  if (boardId === null) return NO_FILTER
  try {
    const raw = localStorage.getItem(FILTER_KEY + boardId)
    if (raw) return { ...NO_FILTER, ...(JSON.parse(raw) as Partial<CardFilter>), q: '' }
  } catch { /* нет хранилища */ }
  return NO_FILTER
}

export function isFiltered(f: CardFilter) {
  return f.who !== 'all' || f.label !== 0 || f.due !== 'all' || f.hideDone || f.q.trim() !== ''
}

export function cardMatches(c: DeckCard, f: CardFilter, me: string): boolean {
  if (f.hideDone && c.done) return false
  if (f.who === 'me' && !c.assigned.some((a) => a.uid === me)) return false
  if (f.who === 'none' && c.assigned.length > 0) return false
  if (f.who.startsWith('u:') && !c.assigned.some((a) => a.uid === f.who.slice(2))) return false
  if (f.label && !c.labels.some((l) => l.id === f.label)) return false
  if (f.due !== 'all') {
    const d = c.duedate ? new Date(c.duedate) : null
    const valid = d !== null && !Number.isNaN(d.getTime())
    if (f.due === 'none') {
      if (valid) return false
    } else {
      if (!valid || !d) return false
      const now = new Date()
      const endOfDay = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1).getTime()
      if (f.due === 'overdue' && (c.done || d.getTime() >= now.getTime())) return false
      if (f.due === 'today' && d.getTime() >= endOfDay) return false
      if (f.due === 'week' && d.getTime() >= endOfDay + 6 * 86400000) return false
    }
  }
  const q = f.q.trim().toLowerCase()
  if (q && !`${c.title} ${c.description} ${c.labels.map((l) => l.title).join(' ')} ${c.assigned.map((a) => a.name).join(' ')}`.toLowerCase().includes(q)) return false
  return true
}

export function dueInfo(card: DeckCard) {
  if (!card.duedate) return null
  const d = new Date(card.duedate)
  if (Number.isNaN(d.getTime())) return null
  const overdue = !card.done && d.getTime() < Date.now()
  return { text: d.toLocaleDateString('ru-RU', { day: 'numeric', month: 'short' }), overdue }
}

export function dateInputValue(iso: string | null) {
  if (!iso) return ''
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? '' : `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
}

/** Переходы из блока «Связи»: письмо, папка документов, история организации. */
export interface ChainActions {
  onOpenMail?: (msgId: string) => void
  onOpenFolder?: (path: string) => void
  onOpenOrg?: (domain: string) => void
}
