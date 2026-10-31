import type { MessageDetail } from './types'

const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')

/** Печать письма (и «Сохранить как PDF» в диалоге печати): заголовки и текст в
 *  скрытой рамке без скриптов. */
export function printMessage(d: MessageDetail) {
  const sender = d.fromName ? `${d.fromName} <${d.fromAddress}>` : d.fromAddress
  const when = d.date ? new Date(d.date).toLocaleString('ru-RU') : ''
  const body = d.bodyHtml ?? `<pre>${esc(d.bodyText)}</pre>`
  const head = [
    ['От', sender], ['Кому', d.to.join(', ')], ...(d.cc.length ? [['Копия', d.cc.join(', ')]] : []), ['Дата', when],
    ...(d.attachments.length ? [['Вложения', d.attachments.map((a) => a.name).join(', ')]] : []),
  ].map(([k, v]) => `<tr><th>${esc(k)}:</th><td>${esc(v)}</td></tr>`).join('')
  const doc = `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'"><title>${esc(d.subject || 'Письмо')}</title>
<style>body{font:13px/1.45 Arial,"Liberation Sans",sans-serif;color:#111;margin:24px}h1{font-size:18px;margin:0 0 10px}table.h{border-collapse:collapse;margin-bottom:14px}table.h th{text-align:left;color:#555;font-weight:normal;padding:1px 10px 1px 0;vertical-align:top}hr{border:0;border-top:1px solid #ccc;margin:12px 0}pre{white-space:pre-wrap;font:inherit}img{max-width:100%}</style></head>
<body><h1>${esc(d.subject || '(без темы)')}</h1><table class="h">${head}</table><hr>${body}</body></html>`
  // Автотесты: диалог печати не открывается, документ сохраняется для проверки.
  const w = window as unknown as { __E2E_PRINT?: string[] }
  if (w.__E2E_PRINT) { w.__E2E_PRINT.push(doc); return }
  const frame = document.createElement('iframe')
  frame.setAttribute('aria-hidden', 'true')
  frame.style.cssText = 'position:fixed;right:0;bottom:0;width:0;height:0;border:0;visibility:hidden'
  frame.srcdoc = doc
  frame.onload = () => {
    frame.contentWindow?.focus()
    frame.contentWindow?.print()
    setTimeout(() => frame.remove(), 60_000)
  }
  document.body.appendChild(frame)
}

/** Файл → base64 (для вложения перетаскиванием). */
export function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = new FileReader()
    r.onload = () => resolve(String(r.result).split(',')[1] ?? '')
    r.onerror = () => reject(r.error)
    r.readAsDataURL(file)
  })
}

/** Варианты «Отложить», как в Gmail. */
export function snoozePresets(now = new Date()): { label: string; at: Date }[] {
  const at = (days: number, hour: number) => { const d = new Date(now); d.setDate(d.getDate() + days); d.setHours(hour, 0, 0, 0); return d }
  const out = [{ label: 'Через 3 часа', at: new Date(now.getTime() + 3 * 3600_000) }]
  if (now.getHours() < 17) out.push({ label: 'Сегодня вечером', at: at(0, 18) })
  out.push({ label: 'Завтра утром', at: at(1, 9) })
  const toMonday = ((8 - now.getDay()) % 7) || 7
  out.push({ label: 'В понедельник', at: at(toMonday, 9) })
  return out
}

/** Срок «отложено до» из ключевого слова `atomsnooze-ГГГГММДДЧЧММ` (UTC). */
export function snoozedUntil(keywords: string[] | undefined): Date | null {
  const k = keywords?.find((x) => x.startsWith('atomsnooze-'))
  if (!k) return null
  const s = k.slice('atomsnooze-'.length)
  const d = new Date(Date.UTC(+s.slice(0, 4), +s.slice(4, 6) - 1, +s.slice(6, 8), +s.slice(8, 10), +s.slice(10, 12)))
  return Number.isNaN(d.getTime()) ? null : d
}
