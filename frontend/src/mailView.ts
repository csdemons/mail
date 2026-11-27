// Отбор писем для списка и свёртка цепочек — вынесено из App.tsx.
// Чистые функции: те же правила, что были в useMemo внутри окна почты.
import { messageKey } from './api'
import { threadKeys } from './mailExtras'
import type { MailFilter, MessageSummary } from './types'

export interface VisibleOptions {
  searching: boolean
  query: string
  filter: MailFilter
  catFilter: string
}

/** Письма текущего списка с учётом фильтра, категории и поиска по уже загруженным. */
export function filterVisible(pool: MessageSummary[], opts: VisibleOptions): MessageSummary[] {
  const q = opts.searching ? '' : opts.query.trim().toLowerCase()
  return pool.filter((m) => {
    const filterMatch =
      opts.filter === 'Все' ? true :
      opts.filter === 'Непрочитанные' ? m.unread :
      opts.filter === 'Важные' ? m.important :
      opts.filter === 'С флажком' ? m.flagged :
      m.hasAttachments
    if (!filterMatch) return false
    if (opts.catFilter && !(m.keywords ?? []).some((k) => k.toLowerCase() === opts.catFilter.toLowerCase())) return false
    if (!q) return true
    return `${m.fromName} ${m.fromAddress} ${m.subject} ${m.to.join(' ')}`.toLowerCase().includes(q)
  })
}

export interface Row {
  m: MessageSummary
  count: number
  child: boolean
  thread: string
}

/** Строки списка: цепочки свёрнуты в одну строку (новое письмо сверху), раскрываются стрелкой. */
export function threadRows(visible: MessageSummary[], opts: { threadsOn: boolean; searching: boolean; openThreads: Set<string> }): Row[] {
  if (!opts.threadsOn || opts.searching) return visible.map((m) => ({ m, count: 1, child: false, thread: '' }))
  const keys = threadKeys(visible)
  const groups = new Map<string, MessageSummary[]>()
  for (const m of visible) {
    const k = keys.get(m) ?? messageKey(m)
    groups.set(k, [...(groups.get(k) ?? []), m])
  }
  const rows: Row[] = []
  for (const [k, items] of groups) {
    rows.push({ m: items[0], count: items.length, child: false, thread: k })
    if (items.length > 1 && opts.openThreads.has(k)) for (const c of items.slice(1)) rows.push({ m: c, count: 1, child: true, thread: k })
  }
  return rows
}
