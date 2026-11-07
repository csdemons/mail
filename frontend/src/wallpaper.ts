import { useEffect, useState } from 'react'

/**
 * Обои чата — личная настройка на этом компьютере (в Talk обоев нет). Общие для всех бесед
 * и свои у отдельных бесед; своя картинка хранится уменьшенной копией (JPEG, до 1920 px).
 */
export type Wallpaper =
  | { kind: 'preset'; id: string }
  | { kind: 'image'; img: string; dim: number; blur: number }

interface Store {
  all: Wallpaper | null
  /** Беседа → свои обои; null — без обоев в этой беседе (даже если общие есть). */
  rooms: Record<string, Wallpaper | null>
}

export interface Preset { id: string; name: string; light: string; dark: string }

const svg = (body: string, size = 40) => `url("data:image/svg+xml,${encodeURIComponent(`<svg xmlns='http://www.w3.org/2000/svg' width='${size}' height='${size}' viewBox='0 0 ${size} ${size}'>${body}</svg>`)}")`

/** Спокойные фоны: светлый и тёмный вариант, текст на пузырях читается в обоих. */
export const PRESETS: Preset[] = [
  { id: 'mint', name: 'Мята', light: 'linear-gradient(160deg, #e6f6ef 0%, #d5efe6 50%, #e3f1f7 100%)', dark: 'linear-gradient(160deg, #13261f 0%, #162a2b 50%, #141f2b 100%)' },
  { id: 'sky', name: 'Небо', light: 'linear-gradient(180deg, #e5f0ff 0%, #f3f7ff 60%, #fdf6ec 100%)', dark: 'linear-gradient(180deg, #121c2e 0%, #18213a 60%, #221d2a 100%)' },
  { id: 'sand', name: 'Песок', light: 'linear-gradient(135deg, #fbf3e4 0%, #f5e9d6 100%)', dark: 'linear-gradient(135deg, #26211a 0%, #1f1b16 100%)' },
  { id: 'lavender', name: 'Лаванда', light: 'linear-gradient(150deg, #f1ecfb 0%, #e8eefb 100%)', dark: 'linear-gradient(150deg, #1f1a2e 0%, #18203a 100%)' },
  { id: 'dots', name: 'Точки', light: `${svg("<circle cx='4' cy='4' r='1.6' fill='#c9d5e6'/>", 22)}, #f4f7fb`, dark: `${svg("<circle cx='4' cy='4' r='1.6' fill='#2b3547'/>", 22)}, #151b26` },
  { id: 'grid', name: 'Клетка', light: `${svg("<path d='M40 0H0V40' fill='none' stroke='#dde4ee' stroke-width='1'/>")}, #f7f9fc`, dark: `${svg("<path d='M40 0H0V40' fill='none' stroke='#232c3b' stroke-width='1'/>")}, #141922` },
  { id: 'waves', name: 'Волны', light: `${svg("<path d='M0 30 Q 15 22 30 30 T 60 30' fill='none' stroke='#d3e3f3' stroke-width='2'/>", 60)}, #f2f7fc`, dark: `${svg("<path d='M0 30 Q 15 22 30 30 T 60 30' fill='none' stroke='#22324a' stroke-width='2'/>", 60)}, #121a26` },
  { id: 'atom', name: 'Атом', light: `${svg("<g fill='none' stroke='#d7e1f0' stroke-width='1.2'><ellipse cx='40' cy='40' rx='26' ry='9'/><ellipse cx='40' cy='40' rx='26' ry='9' transform='rotate(60 40 40)'/><ellipse cx='40' cy='40' rx='26' ry='9' transform='rotate(120 40 40)'/></g><circle cx='40' cy='40' r='3' fill='#d7e1f0'/>", 80)}, #f5f8fc`, dark: `${svg("<g fill='none' stroke='#26324a' stroke-width='1.2'><ellipse cx='40' cy='40' rx='26' ry='9'/><ellipse cx='40' cy='40' rx='26' ry='9' transform='rotate(60 40 40)'/><ellipse cx='40' cy='40' rx='26' ry='9' transform='rotate(120 40 40)'/></g><circle cx='40' cy='40' r='3' fill='#26324a'/>", 80)}, #131925` },
]

const KEY = 'atom.wallpaper'
const IMG_PREFIX = 'atom.wallpaper.img.'

function read(): Store {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? 'null') as Store | null
    return v && typeof v === 'object' ? { all: v.all ?? null, rooms: v.rooms ?? {} } : { all: null, rooms: {} }
  } catch {
    return { all: null, rooms: {} }
  }
}

let store = read()
const listeners = new Set<() => void>()

function persist() {
  try {
    localStorage.setItem(KEY, JSON.stringify(store))
    // Картинки, на которые больше никто не ссылается, удаляем.
    const used = new Set([store.all, ...Object.values(store.rooms)].flatMap((w) => (w?.kind === 'image' ? [w.img] : [])))
    for (let i = localStorage.length - 1; i >= 0; i--) {
      const k = localStorage.key(i)
      if (k?.startsWith(IMG_PREFIX) && !used.has(k.slice(IMG_PREFIX.length))) localStorage.removeItem(k)
    }
  } catch { /* без хранилища — до перезапуска */ }
  listeners.forEach((l) => l())
}

/** Обои беседы: свои, иначе общие. */
export function wallpaperFor(token: string | null): Wallpaper | null {
  if (token && token in store.rooms) return store.rooms[token]
  return store.all
}

export function hasOwn(token: string): boolean {
  return token in store.rooms
}

export function setWallpaper(scope: 'all' | string, w: Wallpaper | null) {
  if (scope === 'all') store = { ...store, all: w }
  else store = { ...store, rooms: { ...store.rooms, [scope]: w } }
  persist()
}

/** Беседа снова с общими обоями. */
export function clearOwn(token: string) {
  const rooms = { ...store.rooms }
  delete rooms[token]
  store = { ...store, rooms }
  persist()
}

export function imageUrl(id: string): string | null {
  try { return localStorage.getItem(IMG_PREFIX + id) } catch { return null }
}

/** Своя картинка: уменьшить до 1920 px и сохранить JPEG; возвращает id картинки. */
export async function storeImage(file: File): Promise<string> {
  if (!file.type.startsWith('image/')) throw new Error('Нужна картинка (JPEG, PNG, WebP…)')
  const url = URL.createObjectURL(file)
  try {
    const img = await new Promise<HTMLImageElement>((ok, fail) => {
      const i = new Image()
      i.onload = () => ok(i)
      i.onerror = () => fail(new Error('Не удалось открыть картинку'))
      i.src = url
    })
    const scale = Math.min(1, 1920 / Math.max(img.naturalWidth, img.naturalHeight))
    const c = document.createElement('canvas')
    c.width = Math.max(1, Math.round(img.naturalWidth * scale))
    c.height = Math.max(1, Math.round(img.naturalHeight * scale))
    c.getContext('2d')!.drawImage(img, 0, 0, c.width, c.height)
    const data = c.toDataURL('image/jpeg', 0.85)
    const id = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`
    try {
      localStorage.setItem(IMG_PREFIX + id, data)
    } catch {
      throw new Error('Картинка слишком большая для хранения — выберите другую')
    }
    return id
  } finally {
    URL.revokeObjectURL(url)
  }
}

/** Стиль слоя обоев. */
export function wallpaperStyle(w: Wallpaper | null, dark: boolean): { layer: React.CSSProperties; shade: React.CSSProperties | null } | null {
  if (!w) return null
  if (w.kind === 'preset') {
    const p = PRESETS.find((x) => x.id === w.id)
    return p ? { layer: { background: dark ? p.dark : p.light }, shade: null } : null
  }
  const src = imageUrl(w.img)
  if (!src) return null
  return {
    layer: { backgroundImage: `url("${src}")`, backgroundSize: 'cover', backgroundPosition: 'center', filter: w.blur ? `blur(${w.blur}px)` : undefined, inset: w.blur ? -w.blur * 2 : 0 },
    shade: w.dim ? { background: dark ? `rgba(0,0,0,${w.dim / 100})` : `rgba(255,255,255,${w.dim / 100})` } : null,
  }
}

export function useWallpaperVersion(): number {
  const [n, setN] = useState(0)
  useEffect(() => { const l = () => setN((x) => x + 1); listeners.add(l); return () => { listeners.delete(l) } }, [])
  return n
}

/** Тёмная тема сейчас включена (следит за классом theme-dark у <html>). */
export function useDarkTheme(): boolean {
  const [dark, setDark] = useState(() => document.documentElement.classList.contains('theme-dark'))
  useEffect(() => {
    const ob = new MutationObserver(() => setDark(document.documentElement.classList.contains('theme-dark')))
    ob.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] })
    return () => ob.disconnect()
  }, [])
  return dark
}
