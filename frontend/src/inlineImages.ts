// Встроенные картинки письма: в редакторе они видны как data:-адреса, а в самом письме
// уходят отдельными частями (multipart/related) и упоминаются в тексте как cid:.
// Без DOM и React — правило одно для отправки и сохранения черновика, поэтому тестируется.

/** Картинка внутри текста письма: файл во временном каталоге, адрес для редактора
 *  и Content-ID, под которым она уходит в письме. */
export interface InlineImage {
  id: string
  cid: string
  url: string
}

/** Content-ID встроенной картинки: уникален внутри письма, только безопасные символы. */
export function newCid(): string {
  return `img-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`
}

/** HTML для отправки или черновика: адреса картинок из редактора заменяются на `cid:`,
 *  а возвращаются только те картинки, которые остались в тексте (удалённые не отправляются). */
export function outboundInline(html: string, inline: InlineImage[]): { html: string; inline: { id: string; cid: string }[] } {
  let out = html
  for (const img of inline) {
    // Пустой адрес — картинку не удалось сохранить: подставлять нечего.
    if (!img.url) continue
    out = out.split(img.url).join(`cid:${img.cid}`)
  }
  const used = inline.filter((img) => img.url && out.includes(`cid:${img.cid}`))
  return { html: out, inline: used.map(({ id, cid }) => ({ id, cid })) }
}

/** Обратное к outboundInline: транспортный HTML (с `cid:`) снова показывает картинки
 *  в редакторе. Нужно для черновика, сохранённого в транспортном виде, — например,
 *  снимка для отмены отправки, сделанного до этого исправления. */
export function restoreInline(html: string, inline: InlineImage[]): string {
  let out = html
  for (const img of inline) {
    if (!img.url || !img.cid) continue
    out = out.split(`cid:${img.cid}`).join(img.url)
  }
  return out
}
