import type { Category, MessageSummary } from './types'

// ---------------------------------------------------------------- сроки флажков

const DUE_PREFIX = 'atomdue-'

/** Срок флажка из ключевого слова `atomdue-ГГГГММДДЧЧММ` (UTC). */
export function dueOf(m: Pick<MessageSummary, 'keywords'>): Date | null {
  const k = m.keywords?.find((x) => x.startsWith(DUE_PREFIX))
  if (!k) return null
  const s = k.slice(DUE_PREFIX.length)
  const d = new Date(Date.UTC(+s.slice(0, 4), +s.slice(4, 6) - 1, +s.slice(6, 8), +s.slice(8, 10), +s.slice(10, 12)))
  return Number.isNaN(d.getTime()) ? null : d
}

export function withDue(keywords: string[], due: Date | null): string[] {
  const rest = keywords.filter((k) => !k.startsWith(DUE_PREFIX))
  if (!due) return rest
  const p = (n: number) => String(n).padStart(2, '0')
  return [...rest, `${DUE_PREFIX}${due.getUTCFullYear()}${p(due.getUTCMonth() + 1)}${p(due.getUTCDate())}${p(due.getUTCHours())}${p(due.getUTCMinutes())}`]
}

/** Варианты срока, как «К исполнению» в Outlook. */
export function duePresets(now = new Date()): { label: string; at: Date }[] {
  const at = (days: number, hour: number) => { const d = new Date(now); d.setDate(d.getDate() + days); d.setHours(hour, 0, 0, 0); return d }
  const today = at(0, 17)
  const out = [{ label: 'Сегодня', at: today > now ? today : new Date(now.getTime() + 3600_000) }, { label: 'Завтра', at: at(1, 9) }]
  const toFriday = (5 - now.getDay() + 7) % 7
  if (toFriday >= 2) out.push({ label: 'На этой неделе (пт)', at: at(toFriday, 9) })
  const toMonday = ((8 - now.getDay()) % 7) || 7
  out.push({ label: 'На следующей неделе (пн)', at: at(toMonday, 9) })
  return out
}

export function dueLabel(d: Date, now = new Date()): string {
  const sameDay = d.toDateString() === now.toDateString()
  const tomorrow = new Date(now); tomorrow.setDate(now.getDate() + 1)
  const time = d.toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' })
  if (sameDay) return `сегодня ${time}`
  if (d.toDateString() === tomorrow.toDateString()) return `завтра ${time}`
  return d.toLocaleDateString('ru-RU', { day: 'numeric', month: 'short' })
}

// ---------------------------------------------------------------- категории

/** $label1 — это «Важное» (отдельная кнопка), остальные — категории. */
export function userCategories(all: Category[]): Category[] {
  return all.filter((c) => c.keyword.toLowerCase() !== '$label1')
}

export function categoriesOf(m: Pick<MessageSummary, 'keywords'>, all: Category[]): Category[] {
  const kw = new Set((m.keywords ?? []).map((k) => k.toLowerCase()))
  return userCategories(all).filter((c) => kw.has(c.keyword.toLowerCase()))
}

/** Следующее свободное ключевое слово для новой категории ($label6, $label7…). */
export function nextCategoryKeyword(all: Category[]): string {
  const used = new Set(all.map((c) => c.keyword.toLowerCase()))
  for (let i = 2; ; i++) if (!used.has(`$label${i}`)) return `$label${i}`
}

// ---------------------------------------------------------------- цепочки

/** Цепочки среди загруженных писем: по Message-ID/References/In-Reply-To
 *  (как «Беседы» в Outlook и Gmail). Возвращает ключ цепочки для каждого письма. */
export function threadKeys(list: MessageSummary[]): Map<MessageSummary, string> {
  const parent = new Map<string, string>()
  const find = (x: string): string => {
    let r = x
    while (parent.has(r) && parent.get(r) !== r) r = parent.get(r)!
    let c = x
    while (parent.get(c) && parent.get(c) !== r) { const n = parent.get(c)!; parent.set(c, r); c = n }
    return r
  }
  const union = (a: string, b: string) => {
    if (!parent.has(a)) parent.set(a, a)
    if (!parent.has(b)) parent.set(b, b)
    const ra = find(a), rb = find(b)
    if (ra !== rb) parent.set(ra, rb)
  }
  const own = (m: MessageSummary) => m.msgId ?? `${m.folder}/${m.uid}`
  for (const m of list) {
    const id = own(m)
    if (!parent.has(id)) parent.set(id, id)
    for (const t of m.thread ?? []) union(id, t)
  }
  const out = new Map<MessageSummary, string>()
  for (const m of list) out.set(m, find(own(m)))
  return out
}
