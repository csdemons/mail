// Помощники чата: превью и кэш файлов, подписи времени, слияние ленты, порядок бесед.
import { useEffect, useState } from 'react'
import { FileText } from 'lucide-react'

import { api } from './api'
import { previewKind } from './AttachmentViewer'
import type { TalkFile, TalkMessage, TalkRoom } from './types'

/** Файл чата открывается во встроенном просмотрщике (фото, PDF, текст), если известен его путь. */
export function viewable(f: TalkFile): boolean {
  return !!f.path && previewKind({ name: f.name, contentType: f.mimetype }) !== null
}

export function isImage(f: TalkFile): boolean {
  return previewKind({ name: f.name, contentType: f.mimetype }) === 'image'
}

/** Кэши на время работы окна: уменьшенные копии и содержимое открытых файлов. */
const previews = new Map<string, Promise<string | null>>()
export const contents = new Map<string, Promise<string>>()

export function cached<T>(map: Map<string, Promise<T>>, key: string, limit: number, make: () => Promise<T>): Promise<T> {
  let p = map.get(key)
  if (!p) {
    p = make()
    p.catch(() => map.delete(key))
    map.set(key, p)
    if (map.size > limit) map.delete(map.keys().next().value as string)
  }
  return p
}

export function TalkThumb({ file }: { file: TalkFile }) {
  const [src, setSrc] = useState<string | null>(null)
  const [failed, setFailed] = useState(false)
  useEffect(() => {
    let alive = true
    cached(previews, file.id, 200, () => api.talkFilePreview(file.id))
      .then((d) => { if (alive) { if (d) setSrc(d); else setFailed(true) } })
      .catch(() => { if (alive) setFailed(true) })
    return () => { alive = false }
  }, [file.id])
  if (failed) return <span className="talk-image-placeholder"><FileText size={22} /></span>
  return src ? <img src={src} alt={file.name} draggable={false} /> : <span className="talk-image-placeholder">Загрузка…</span>
}

export function expiryLabel(sec: number): string {
  if (sec % 604800 === 0) return `${sec / 604800} нед.`
  if (sec % 86400 === 0) return `${sec / 86400} дн.`
  if (sec % 3600 === 0) return `${sec / 3600} ч`
  return `${Math.round(sec / 60)} мин`
}

/** Найденные слова — полужирным (без HTML: только текст и <mark>). */
export function highlight(text: string, q: string): React.ReactNode {
  if (!q) return text
  const low = text.toLowerCase(), ql = q.toLowerCase()
  const out: React.ReactNode[] = []
  let i = 0
  for (let j = low.indexOf(ql); j >= 0 && out.length < 40; j = low.indexOf(ql, i)) {
    out.push(text.slice(i, j), <mark key={j}>{text.slice(j, j + q.length)}</mark>)
    i = j + q.length
  }
  out.push(text.slice(i))
  return out
}

export function timeOf(ts: number) {
  const d = new Date(ts * 1000)
  const today = new Date()
  return d.toDateString() === today.toDateString()
    ? d.toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' })
    : d.toLocaleString('ru-RU', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })
}

/** Новые сообщения добавляются, обновления (реакции, правки, удаление) заменяют уже показанные. */
export function merge(list: TalkMessage[], incoming: TalkMessage[]): TalkMessage[] {
  const byId = new Map(list.map((m) => [m.id, m]))
  for (const m of incoming) {
    const cur = byId.get(m.id)
    if (cur || !m.update) byId.set(m.id, cur && m.update ? { ...m, update: false, parentText: m.parentText ?? cur.parentText, parentAuthor: m.parentAuthor ?? cur.parentAuthor } : { ...m, update: false })
  }
  return [...byId.values()].sort((a, b) => a.id - b.id)
}

/** Как в Talk: избранные беседы сверху, внутри — по последней активности. */
export function sortRooms(list: TalkRoom[]): TalkRoom[] {
  return [...list].sort((a, b) => Number(b.favorite) - Number(a.favorite) || b.lastActivity - a.lastActivity)
}
