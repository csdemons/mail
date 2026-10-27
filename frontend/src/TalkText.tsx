import type { ReactNode } from 'react'

import { api } from './api'

/** Безопасный показ текста сообщения Talk: ссылки, **жирный**, *курсив*,
 *  `код` и блоки ```кода```. HTML не используется — только React-элементы. */
// Без lookbehind: старый WebKit (Astra 1.7) его не знает; границу слова для
// *курсива* и _курсива_ проверяем вручную.
const INLINE = /(https?:\/\/[^\s<>"]+[^\s<>".,;:!?)\]]|\*\*[^*\n]+\*\*|`[^`\n]+`|\*[^*\s][^*\n]*\*|_[^_\s][^_\n]*_|@[^\s@,.:;!?]+(?: [А-ЯЁA-Z][^\s@,.:;!?]+)?)/g
const WORD = /[\p{L}\p{N}_]/u

function inline(text: string, key: string): ReactNode[] {
  const out: ReactNode[] = []
  let last = 0
  let i = 0
  for (const m of text.matchAll(INLINE)) {
    const t = m[0]
    const at = m.index ?? 0
    if (at > last) out.push(text.slice(last, at))
    const k = `${key}-${i++}`
    const italic = (t.startsWith('*') && !t.startsWith('**')) || t.startsWith('_')
    if (italic && ((at > 0 && WORD.test(text[at - 1])) || WORD.test(text[at + t.length] ?? ''))) {
      out.push(t)
    } else if (t.startsWith('http')) {
      out.push(<a key={k} href={t} onClick={(e) => { e.preventDefault(); void api.openLink(t) }}>{t}</a>)
    } else if (t.startsWith('**')) {
      out.push(<b key={k}>{t.slice(2, -2)}</b>)
    } else if (t.startsWith('`')) {
      out.push(<code key={k}>{t.slice(1, -1)}</code>)
    } else if (t.startsWith('@')) {
      out.push(<span key={k} className="talk-mention">{t}</span>)
    } else {
      out.push(<i key={k}>{t.slice(1, -1)}</i>)
    }
    last = at + t.length
  }
  if (last < text.length) out.push(text.slice(last))
  return out
}

const ONLY_EMOJI = /^(?:\p{Extended_Pictographic}(?:\uFE0F|\u200D\p{Extended_Pictographic}|[\u{1F3FB}-\u{1F3FF}])*\s*){1,3}$/u

export default function TalkText({ text }: { text: string }) {
  const parts = text.split(/```\n?/)
  return (
    <div className={`talk-text ${ONLY_EMOJI.test(text.trim()) ? 'talk-big-emoji' : ''}`}>
      {parts.map((p, n) => (n % 2 === 1
        ? <pre key={n} className="talk-code">{p.replace(/\n$/, '')}</pre>
        : <span key={n}>{inline(p, String(n))}</span>))}
    </div>
  )
}
