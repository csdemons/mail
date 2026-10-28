import { api, formatFullDate } from './api'
import { sharedMailboxOf } from './FolderPicker'
import type { Draft, Folder, MessageDetail, MessageSummary } from './types'

export function senderLabel(m: Pick<MessageSummary, 'fromName' | 'fromAddress'>): string {
  return m.fromName || m.fromAddress || '(без отправителя)'
}

export function initials(value: string) {
  return value.trim().slice(0, 1).toUpperCase() || '?'
}

/** Адрес из строки «Имя <a@b.ru>» или «a@b.ru». */
export function addressOf(entry: string): string {
  const m = entry.match(/<([^>]+)>/)
  return (m ? m[1] : entry).trim().toLowerCase()
}

const withPrefix = (prefix: string, subj: string) =>
  new RegExp(`^${prefix}:`, 'i').test(subj.trim()) ? subj.trim() : `${prefix}: ${subj.trim()}`

/** Черновик ответа, ответа всем или пересылки. `me` — свой адрес (исключается из «Ответить всем»). */
export function buildDraft(mode: 'reply' | 'replyAll' | 'forward', m: MessageDetail, me: string): Draft {
  const sender = m.fromName ? `${m.fromName} <${m.fromAddress}>` : m.fromAddress
  const header = `${formatFullDate(m.date)}, ${senderLabel(m)} пишет:`
  if (mode === 'forward') {
    return {
      mode,
      to: '',
      cc: '',
      subject: withPrefix('Fwd', m.subject),
      body: `\n\n---------- Пересланное сообщение ----------\nОт: ${sender}\nДата: ${formatFullDate(m.date)}\nТема: ${m.subject}\nКому: ${m.to.join(', ')}${m.cc.length ? `\nКопия: ${m.cc.join(', ')}` : ''}\n\n${m.bodyText}`,
      inReplyTo: null,
      references: null,
    }
  }
  const primary = m.replyTo ?? sender
  let to = primary
  let cc = ''
  if (mode === 'replyAll') {
    const self = me.trim().toLowerCase()
    const seen = new Set([self, ...primary.split(',').map(addressOf)])
    const pick = (list: string[]) => list.filter((e) => {
      const a = addressOf(e)
      if (!a || seen.has(a)) return false
      seen.add(a)
      return true
    })
    const others = pick(m.to)
    to = [primary, ...others].join(', ')
    cc = pick(m.cc).join(', ')
  }
  return {
    mode,
    to: to ? `${to}, ` : '',
    cc: cc ? `${cc}, ` : '',
    subject: withPrefix('Re', m.subject),
    body: `\n\n${header}\n${m.bodyText.split('\n').map((l) => `> ${l}`).join('\n')}`,
    inReplyTo: m.messageId,
    references: [m.references, m.messageId].filter(Boolean).join(' ') || null,
  }
}

const ARCHIVE_NAMES = ['archive', 'archives', 'архив']

/** Папка «Архив» того же ящика; если её нет — создаётся («Archive» в корне ящика). */
export async function ensureArchive(folders: Folder[], folderId: string): Promise<{ id: string; created: boolean }> {
  const folder = folders.find((f) => f.id === folderId)
  const box = folder ? sharedMailboxOf(folders, folder) : null
  const same = folders.filter((f) => f.selectable && sharedMailboxOf(folders, f) === box)
  const found = same.find((f) => f.role === 'archive')
    ?? same.find((f) => ARCHIVE_NAMES.includes(f.name.toLowerCase()) && f.depth === (box ? 2 : 0))
  if (found) return { id: found.id, created: false }
  const id = box ? `${box}/Archive` : 'Archive'
  await api.createFolder(id)
  return { id, created: true }
}
