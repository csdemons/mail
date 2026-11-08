import type { Contact, Signature } from './types'

/** Ключ автосохранения черновика (латиница и цифры — уходит в заголовок письма). */
export function newDraftKey(): string {
  const r = Array.from(crypto.getRandomValues(new Uint8Array(12)), (b) => b.toString(16).padStart(2, '0')).join('')
  return `d-${Date.now().toString(36)}-${r}`
}

// ---------------------------------------------------------------- подписи

const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')

/** Блок подписи в редакторе: «-- » и текст; помечен data-signature, чтобы менять при выборе другой. */
export function signatureHtml(text: string): string {
  const t = text.trim()
  if (!t) return ''
  return `<div data-signature="1"><div>-- </div>${t.split('\n').map((l) => `<div>${esc(l) || '<br>'}</div>`).join('')}</div>`
}

export function signatureById(list: Signature[], id: string | null | undefined): Signature | null {
  return (id && list.find((s) => s.id === id)) || null
}

// ---------------------------------------------------------------- готовые ответы

export const TEMPLATE_VARS: { key: string; hint: string }[] = [
  { key: 'имя', hint: 'имя (и отчество) получателя' },
  { key: 'получатель', hint: 'получатель полностью' },
  { key: 'организация', hint: 'организация получателя' },
  { key: 'тема', hint: 'тема письма' },
  { key: 'дата', hint: 'сегодняшняя дата' },
  { key: 'моё имя', hint: 'ваше имя' },
]

/** «Иванов Иван Иванович» → «Иван Иванович»; «Иван Петров» → «Иван»; одно слово — оно само. */
export function givenName(full: string): string {
  const w = full.trim().split(/\s+/).filter(Boolean)
  const patronymic = (x: string) => /(вич|вна|чна|ична|оглы|кызы)$/i.test(x)
  if (w.length >= 3 && patronymic(w[2])) return `${w[1]} ${w[2]}`
  if (w.length >= 2 && patronymic(w[1])) return `${w[0]} ${w[1]}`
  return w[0] ?? ''
}

function firstRecipient(to: string): { name: string; email: string } | null {
  const first = to.split(/[,;]/).map((x) => x.trim()).find(Boolean)
  if (!first) return null
  const m = /^(.*?)\s*<([^>]+)>$/.exec(first)
  return m ? { name: m[1].replace(/^"|"$/g, '').trim(), email: m[2].trim().toLowerCase() } : { name: '', email: first.toLowerCase() }
}

/** Значения переменных шаблона для этого письма; неизвестные — undefined. */
export function templateValues(ctx: { to: string; subject: string; from: string; contacts: Contact[] }): Record<string, string | undefined> {
  const r = firstRecipient(ctx.to)
  const contact = r ? ctx.contacts.find((c) => c.emails.some((e) => e.toLowerCase() === r.email)) : undefined
  const fullName = contact?.fullName || r?.name || ''
  const me = /^(.*?)\s*<[^>]+>$/.exec(ctx.from)?.[1]?.trim() || ''
  return {
    'имя': fullName ? givenName(fullName) : undefined,
    'получатель': fullName || r?.email || undefined,
    'организация': contact?.org || undefined,
    'тема': ctx.subject.trim() || undefined,
    'дата': new Date().toLocaleDateString('ru-RU'),
    'моё имя': me || undefined,
  }
}

/** Подставить {переменные}; неизвестные остаются выделенными, чтобы их заметили. */
export function fillTemplate(html: string, values: Record<string, string | undefined>): string {
  return html.replace(/\{([а-яёa-z ]{2,20})\}/gi, (whole, name: string) => {
    const key = name.trim().toLowerCase().replace(/ё/g, 'е')
    const hit = Object.entries(values).find(([k]) => k.replace(/ё/g, 'е') === key)
    if (!hit) return whole
    return hit[1] !== undefined ? esc(hit[1]) : `<mark>${esc(whole)}</mark>`
  })
}

// ---------------------------------------------------------------- проверка адресатов

export function addressList(value: string): string[] {
  return value.split(/[,;]/).map((x) => x.trim()).filter(Boolean).map((x) => (/<([^>]+)>/.exec(x)?.[1] ?? x).trim().toLowerCase()).filter((x) => x.includes('@'))
}

const domainOf = (addr: string) => addr.split('@')[1]?.replace(/\.$/, '') ?? ''
const within = (d: string, dom: string) => d === dom || d.endsWith(`.${dom}`)

export interface ExternalCheck {
  external: { address: string; domain: string; isNew: boolean }[]
  reasons: string[]
}

/**
 * Нужно ли предупредить перед отправкой. «smart»: внешние получатели и вложения, смешанные
 * (свои и внешние) получатели, домен, с которым ещё не переписывались (опечатка); «always» —
 * любой внешний адрес; «off» — никогда.
 */
export function checkRecipients(opts: {
  to: string; cc: string; ownAddress: string; internalDomains: string[]; mode: 'smart' | 'always' | 'off'
  hasAttachments: boolean; knownDomains: Set<string>
}): ExternalCheck | null {
  if (opts.mode === 'off') return null
  const own = opts.internalDomains.length ? opts.internalDomains : [domainOf(opts.ownAddress.toLowerCase())].filter(Boolean)
  const all = [...addressList(opts.to), ...addressList(opts.cc)]
  const ext = all.filter((a) => !own.some((d) => within(domainOf(a), d)))
  if (ext.length === 0) return null
  const external = [...new Set(ext)].map((address) => {
    const domain = domainOf(address)
    return { address, domain, isNew: ![...opts.knownDomains].some((k) => within(domain, k)) }
  })
  const reasons: string[] = []
  if (opts.hasAttachments) reasons.push('Письмо с вложениями уходит за пределы организации.')
  if (ext.length < all.length) reasons.push('Среди получателей и сотрудники, и внешние адреса — внешние увидят весь список и переписку.')
  if (external.some((e) => e.isNew)) reasons.push('С некоторыми доменами вы ещё не переписывались — проверьте, нет ли опечатки в адресе.')
  if (opts.mode === 'smart' && reasons.length === 0) return null
  if (reasons.length === 0) reasons.push('Письмо уходит за пределы организации.')
  return { external, reasons }
}
