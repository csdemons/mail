import { useEffect, useState } from 'react'
import type { ReactNode } from 'react'

import { api, onSessionReset } from './api'

/** Аватары Nextcloud на время работы программы: один запрос на человека/беседу. */
const cache = new Map<string, Promise<string | null>>()
onSessionReset(() => cache.clear())

function load(kind: 'user' | 'room', id: string): Promise<string | null> {
  const key = `${kind}:${id}`
  let p = cache.get(key)
  if (!p) {
    p = api.ncAvatar(kind, id).catch(() => null)
    cache.set(key, p)
  }
  return p
}

export function initialsOf(name: string) {
  const p = name.trim().split(/\s+/)
  return ((p[0]?.[0] ?? '') + (p[1]?.[0] ?? '')).toUpperCase() || '?'
}

/** Аватар из Nextcloud; пока грузится или если его нет — инициалы (или `fallback`). */
export default function Avatar({ kind = 'user', id, name, fallback, className = '' }: {
  kind?: 'user' | 'room'
  id: string | null | undefined
  name: string
  fallback?: ReactNode
  className?: string
}) {
  const [src, setSrc] = useState<string | null>(null)
  useEffect(() => {
    let alive = true
    setSrc(null)
    if (id) load(kind, id).then((s) => { if (alive) setSrc(s) })
    return () => { alive = false }
  }, [kind, id])
  return (
    <div className={`sender-avatar ${src ? 'has-image' : ''} ${className}`} title={name}>
      {src ? <img src={src} alt="" /> : fallback ?? initialsOf(name)}
    </div>
  )
}
