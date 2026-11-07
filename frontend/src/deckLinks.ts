/**
 * Связи карточек Deck: «главная задача» (подзадачи) и «выполняется после» (последовательность).
 * В Deck своих связей нет, поэтому они хранятся в описании карточки — коротким блоком со
 * ссылками на карточки. Так связи видны на любом компьютере, а в веб-Deck это обычные ссылки:
 *
 *     ---
 *     🔗 Главная задача: [Подготовить релиз](https://next.example/index.php/apps/deck/card/12)
 *     ⏭ Выполняется после: [Сборка](https://next.example/index.php/apps/deck/card/10)
 *
 * Связь хранится у подчинённой карточки; обратные (подзадачи, «следующие») вычисляются по доске.
 *
 * Карточка, созданная из письма, хранит в том же блоке организацию, письмо и папку документов:
 *
 *     🏢 Контрагент: niiar.ru
 *     ✉ Письмо: [Запрос документов](mid:abc%40niiar.ru)
 *     📁 Документы: [Контрагенты/niiar.ru/Запрос](https://next.example/index.php/apps/files/?dir=%2F…)
 */
import type { DeckCard, DeckStack } from './types'

export interface CardLink { id: number; title: string }
export interface CardLinks {
  parent: CardLink | null
  after: CardLink[]
  /** Организация (домен), с которой связана задача. */
  org?: string | null
  /** Письмо, из которого создана задача (Message-ID без угловых скобок). */
  mail?: { title: string; msgId: string } | null
  /** Папка документов задачи в Nextcloud. */
  folder?: { title: string; path: string } | null
}

const PARENT_RE = /^\s*(?:🔗\s*)?Главная задача:\s*\[(.*)\]\((\S+?)\)\s*$/
const AFTER_RE = /^\s*(?:⏭\s*)?(?:Выполняется\s+)?[Пп]осле:\s*\[(.*)\]\((\S+?)\)\s*$/
const ORG_RE = /^\s*(?:🏢\s*)?Контрагент:\s*@?([\p{L}\d-]+(?:\.[\p{L}\d-]+)+)\s*$/u
const MAIL_RE = /^\s*(?:✉️?\s*)?Письмо:\s*\[(.*)\]\(mid:(\S+?)\)\s*$/u
const FOLDER_RE = /^\s*(?:📁\s*)?Документы:\s*\[(.*)\]\((\S+?)\)\s*$/u
const CARD_ID_RE = /\/card\/(\d+)(?:[/?#]|$)/

function idOf(url: string): number | null {
  const m = CARD_ID_RE.exec(url)
  return m ? Number(m[1]) : null
}

/** Описание без блока связей и сами связи. */
export function splitLinks(description: string): { body: string; links: CardLinks } {
  const links: CardLinks = { parent: null, after: [] }
  const keep: string[] = []
  for (const line of description.split('\n')) {
    const o = ORG_RE.exec(line)
    if (o) { links.org = o[1].toLowerCase(); continue }
    const ml = MAIL_RE.exec(line)
    if (ml) { links.mail = { title: unesc(ml[1]), msgId: safeDecode(ml[2]) }; continue }
    const f = FOLDER_RE.exec(line)
    const fp = f ? folderPath(f[2]) : null
    if (f && fp) { links.folder = { title: unesc(f[1]), path: fp }; continue }
    const p = PARENT_RE.exec(line)
    const a = p ? null : AFTER_RE.exec(line)
    const m = p ?? a
    const id = m ? idOf(m[2]) : null
    if (m && id !== null) {
      const link = { id, title: unesc(m[1]) }
      if (p) links.parent = link
      else if (!links.after.some((x) => x.id === id)) links.after.push(link)
      continue
    }
    keep.push(line)
  }
  // Разделитель «---», оставшийся в конце от блока связей, тоже убираем.
  while (keep.length && /^\s*$/.test(keep[keep.length - 1])) keep.pop()
  if (keep.length && /^\s*---\s*$/.test(keep[keep.length - 1]) && hasLinks(links)) keep.pop()
  while (keep.length && /^\s*$/.test(keep[keep.length - 1])) keep.pop()
  return { body: keep.join('\n'), links }
}

const unesc = (t: string) => t.replace(/\\([[\]])/g, '$1')
function safeDecode(s: string): string {
  try { return decodeURIComponent(s) } catch { return s }
}
/** Путь папки из ссылки на приложение «Файлы» (`?dir=…`). */
function folderPath(url: string): string | null {
  try { const d = new URL(url).searchParams.get('dir'); return d && d.startsWith('/') ? d : null } catch { return null }
}
function hasLinks(l: CardLinks): boolean {
  return !!(l.parent || l.after.length || l.org || l.mail || l.folder)
}

export function folderUrl(base: string, path: string): string {
  return `${base.replace(/\/+$/, '')}/index.php/apps/files/?dir=${encodeURIComponent(path)}`
}

export function cardUrl(base: string, id: number): string {
  return `${base.replace(/\/+$/, '')}/index.php/apps/deck/card/${id}`
}

const esc = (t: string) => t.replace(/[[\]]/g, (c) => `\\${c}`).replace(/\n/g, ' ')

/** Собрать описание обратно: текст + блок связей (если связи есть). */
export function joinLinks(body: string, links: CardLinks, base: string): string {
  const lines: string[] = []
  if (links.org) lines.push(`🏢 Контрагент: ${links.org}`)
  if (links.mail) lines.push(`✉ Письмо: [${esc(links.mail.title)}](mid:${encodeURIComponent(links.mail.msgId)})`)
  if (links.folder) lines.push(`📁 Документы: [${esc(links.folder.title)}](${folderUrl(base, links.folder.path)})`)
  if (links.parent) lines.push(`🔗 Главная задача: [${esc(links.parent.title)}](${cardUrl(base, links.parent.id)})`)
  for (const a of links.after) lines.push(`⏭ Выполняется после: [${esc(a.title)}](${cardUrl(base, a.id)})`)
  const text = body.replace(/\s+$/, '')
  if (lines.length === 0) return text
  return `${text}${text ? '\n\n' : ''}---\n${lines.join('\n')}`
}

/** Связи всей доски: у кого какая главная, подзадачи, предшествующие и следующие. */
export interface BoardLinks {
  links: Map<number, CardLinks>
  children: Map<number, DeckCard[]>
  next: Map<number, DeckCard[]>
  cards: Map<number, DeckCard>
}

export function boardLinks(stacks: DeckStack[]): BoardLinks {
  const cards = new Map<number, DeckCard>()
  const links = new Map<number, CardLinks>()
  const children = new Map<number, DeckCard[]>()
  const next = new Map<number, DeckCard[]>()
  for (const s of stacks) for (const c of s.cards) cards.set(c.id, c)
  for (const c of cards.values()) {
    const l = splitLinks(c.description).links
    links.set(c.id, l)
    if (l.parent && l.parent.id !== c.id) children.set(l.parent.id, [...(children.get(l.parent.id) ?? []), c])
    for (const a of l.after) if (a.id !== c.id) next.set(a.id, [...(next.get(a.id) ?? []), c])
  }
  return { links, children, next, cards }
}

/** Карточка ждёт: хоть одна предшествующая (на этой доске) ещё не выполнена. */
export function blockedBy(card: DeckCard, b: BoardLinks): DeckCard[] {
  return (b.links.get(card.id)?.after ?? []).map((a) => b.cards.get(a.id)).filter((c): c is DeckCard => Boolean(c && !c.done))
}

/** Все связанные карточки (для подсветки при наведении): главная, подзадачи, до и после. */
export function related(id: number, b: BoardLinks): Set<number> {
  const out = new Set<number>()
  const l = b.links.get(id)
  if (l?.parent) { out.add(l.parent.id); for (const s of b.children.get(l.parent.id) ?? []) out.add(s.id) }
  for (const c of b.children.get(id) ?? []) out.add(c.id)
  for (const a of l?.after ?? []) out.add(a.id)
  for (const n of b.next.get(id) ?? []) out.add(n.id)
  out.delete(id)
  return out
}

/** Связь «после» не должна замыкать цепочку в круг (A после B, B после A). */
export function wouldCycle(cardId: number, afterId: number, b: BoardLinks): boolean {
  const seen = new Set<number>()
  const stack = [afterId]
  while (stack.length) {
    const x = stack.pop()!
    if (x === cardId) return true
    if (seen.has(x)) continue
    seen.add(x)
    for (const a of b.links.get(x)?.after ?? []) stack.push(a.id)
  }
  return false
}

/** Порядок карточек в колонке: подзадачи из той же колонки — сразу под главной. */
export function nestInColumn(list: DeckCard[], b: BoardLinks): { card: DeckCard; depth: number }[] {
  const ids = new Set(list.map((c) => c.id))
  const out: { card: DeckCard; depth: number }[] = []
  const placed = new Set<number>()
  const put = (c: DeckCard, depth: number) => {
    if (placed.has(c.id)) return
    placed.add(c.id)
    out.push({ card: c, depth })
    for (const ch of list.filter((x) => b.links.get(x.id)?.parent?.id === c.id)) put(ch, Math.min(depth + 1, 3))
  }
  for (const c of list) {
    const p = b.links.get(c.id)?.parent?.id
    if (p !== undefined && ids.has(p) && p !== c.id) continue
    put(c, 0)
  }
  for (const c of list) put(c, 0) // на случай круга из подзадач
  return out
}

/** Группа доски по названию проекта: «ЦОД: сеть», «ЦОД — серверы», «ЦОД / СХД», «[ЦОД] …». */
export function projectOf(title: string): string | null {
  const t = title.trim()
  const br = /^\[([^\]]{2,40})\]/.exec(t)
  if (br) return br[1].trim()
  const m = /^(.{2,40}?)\s*(?::|\s[—–-]\s|\s\/\s)\s*\S/.exec(t)
  return m ? m[1].trim() : null
}
