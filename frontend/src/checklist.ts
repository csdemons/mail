// Подзадачи Deck — это чек-лист Markdown в описании карточки: «- [ ] пункт» /
// «- [x] пункт». Веб-Deck показывает их так же, поэтому храним там же.

export interface CheckItem {
  /** Номер строки в описании. */
  line: number
  done: boolean
  text: string
}

const ITEM = /^(\s*[-*+]\s+)\[( |x|X)\]\s?(.*)$/

export function parseChecklist(description: string): CheckItem[] {
  return description.split('\n').flatMap((l, line) => {
    const m = ITEM.exec(l)
    return m ? [{ line, done: m[2] !== ' ', text: m[3] }] : []
  })
}

export function checklistProgress(description: string): { done: number; total: number } {
  const items = parseChecklist(description)
  return { done: items.filter((i) => i.done).length, total: items.length }
}

export function toggleItem(description: string, line: number): string {
  const lines = description.split('\n')
  const m = ITEM.exec(lines[line] ?? '')
  if (m) lines[line] = `${m[1]}[${m[2] === ' ' ? 'x' : ' '}] ${m[3]}`
  return lines.join('\n')
}

export function renameItem(description: string, line: number, text: string): string {
  const lines = description.split('\n')
  const m = ITEM.exec(lines[line] ?? '')
  if (m) lines[line] = `${m[1]}[${m[2]}] ${text.replace(/\n/g, ' ').trim()}`
  return lines.join('\n')
}

export function removeItem(description: string, line: number): string {
  const lines = description.split('\n')
  if (ITEM.test(lines[line] ?? '')) lines.splice(line, 1)
  return lines.join('\n')
}

/** Новый пункт — сразу после последнего пункта чек-листа (или в конце описания). */
export function addItem(description: string, text: string): string {
  const item = `- [ ] ${text.replace(/\n/g, ' ').trim()}`
  const items = parseChecklist(description)
  if (items.length === 0) {
    const base = description.replace(/\s+$/, '')
    return base ? `${base}\n\n${item}` : item
  }
  const lines = description.split('\n')
  lines.splice(items[items.length - 1].line + 1, 0, item)
  return lines.join('\n')
}
