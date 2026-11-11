import { invoke } from '@tauri-apps/api/core'
import { useEffect, useState } from 'react'

/**
 * Внешний вид на этом компьютере: масштаб интерфейса и размер текста сообщений и писем.
 * Хранится в localStorage (настройка рабочего места, а не учётной записи).
 */
export interface Appearance {
  /** Масштаб всего окна: 0.8–1.75. */
  zoom: number
  /** Размер текста в чате и письмах, px. */
  text: number
  /** Список писем: «обычно» — с началом текста, «компактно» — плотнее, без него. */
  density: 'normal' | 'compact'
  /** Шрифт интерфейса; '' — системный. */
  uiFont: string
  /** Шрифт текста писем и сообщений чата; '' — как в интерфейсе. Письма со своим шрифтом — как есть. */
  readFont: string
  /** Шрифт и размер новых писем (то, что увидит получатель); '' — Arial. */
  composeFont: string
  composeSize: number
  /** Цвет акцента (кнопки, выделение) — #rrggbb; '' — фирменный синий. */
  accent: string
  /** Тема: светлая, тёмная или как в системе. */
  theme: 'light' | 'dark' | 'system'
}

export const ZOOM_STEPS = [0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75]
export const TEXT_MIN = 12
export const TEXT_MAX = 22
const KEY = 'atom.appearance'
const DEFAULTS: Appearance = { zoom: 1, text: 14, density: 'normal', uiFont: '', readFont: '', composeFont: '', composeSize: 14, accent: '', theme: 'system' }
export const COMPOSE_SIZES = [10, 11, 12, 13, 14, 15, 16, 18, 20, 24]

/** Системный набор шрифтов интерфейса (как в styles.css). */
export const SYSTEM_FONT = '"Segoe UI Variable Text", "Segoe UI", system-ui, -apple-system, BlinkMacSystemFont, "PT Astra Sans", "Noto Sans", "Ubuntu", "Cantarell", "Liberation Sans", Arial, sans-serif'

/** Название шрифта: только буквы, цифры, пробел, дефис, точка — оно попадает в CSS (и в письма). */
export function cleanFont(name: unknown): string {
  const n = typeof name === 'string' ? name.trim() : ''
  return /^[\p{L}\p{N} .\-]{1,60}$/u.test(n) ? n.replace(/\s+/g, ' ') : ''
}

/** CSS font-family: выбранный шрифт, затем системные — если его нет на компьютере. */
export function fontStack(name: string, fallback = SYSTEM_FONT): string {
  const n = cleanFont(name)
  return n ? `"${n}", ${fallback}` : fallback
}

const cleanAccent = (v: unknown) => (typeof v === 'string' && /^#[0-9a-f]{6}$/i.test(v) ? v.toLowerCase() : '')

/** Оттенки акцента: светлый фон выделения, рамка, тёмный текст на светлом. */
function mix(hex: string, to: [number, number, number], k: number): string {
  const n = parseInt(hex.slice(1), 16)
  const c = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((v, i) => Math.round(v + (to[i] - v) * k))
  return `#${c.map((v) => v.toString(16).padStart(2, '0')).join('')}`
}

/** Есть ли шрифт на этом компьютере (сравнение ширины текста с запасными гарнитурами). */
export function fontInstalled(name: string): boolean {
  const n = cleanFont(name)
  if (!n) return false
  try {
    const c = document.createElement('canvas').getContext('2d')
    if (!c) return true
    const sample = 'mmmmmmmmmmlliЖжШщ0123456789'
    return ['monospace', 'serif', 'sans-serif'].some((base) => {
      c.font = `72px ${base}`; const w0 = c.measureText(sample).width
      c.font = `72px "${n}", ${base}`; return c.measureText(sample).width !== w0
    })
  } catch {
    return true
  }
}

function read(): Appearance {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? '{}') as Partial<Appearance>
    const zoom = ZOOM_STEPS.includes(Number(v.zoom)) ? Number(v.zoom) : DEFAULTS.zoom
    const text = Number.isFinite(v.text) ? Math.min(TEXT_MAX, Math.max(TEXT_MIN, Number(v.text))) : DEFAULTS.text
    const density = v.density === 'compact' ? 'compact' : 'normal'
    const composeSize = COMPOSE_SIZES.includes(Number(v.composeSize)) ? Number(v.composeSize) : DEFAULTS.composeSize
    let theme: Appearance['theme'] = v.theme === 'light' || v.theme === 'dark' ? v.theme : 'system'
    // Прежняя настройка темы (кнопка в шапке) — если новой ещё нет.
    if (v.theme === undefined) {
      try { const old = localStorage.getItem('rrcki-mail.theme'); if (old === 'dark' || old === 'light') theme = old } catch { /* нет хранилища */ }
    }
    return { zoom, text, density, uiFont: cleanFont(v.uiFont), readFont: cleanFont(v.readFont), composeFont: cleanFont(v.composeFont), composeSize, accent: cleanAccent(v.accent), theme }
  } catch {
    return { ...DEFAULTS }
  }
}

let current = read()
const listeners = new Set<(a: Appearance) => void>()

/** Применить к окну: масштаб веб-движка (или CSS, если движок не умеет) и переменная размера текста. */
export function applyAppearance(a: Appearance = current) {
  const root = document.documentElement
  root.style.setProperty('--content-font', `${a.text}px`)
  root.dataset.density = a.density
  root.style.setProperty('--ui-font', fontStack(a.uiFont))
  root.style.setProperty('--read-font', a.readFont ? fontStack(a.readFont) : 'inherit')
  root.style.setProperty('--compose-font', fontStack(a.composeFont, 'Arial, Helvetica, sans-serif'))
  root.style.setProperty('--compose-size', `${a.composeSize}px`)
  if (a.accent) {
    root.dataset.accent = '1'
    root.style.setProperty('--accent', a.accent)
    root.style.setProperty('--accent-soft', mix(a.accent, [255, 255, 255], 0.9))
    root.style.setProperty('--accent-soft2', mix(a.accent, [255, 255, 255], 0.8))
    root.style.setProperty('--accent-line', mix(a.accent, [255, 255, 255], 0.65))
    root.style.setProperty('--accent-ink', mix(a.accent, [0, 0, 0], 0.15))
    root.style.setProperty('--accent-dark', mix(a.accent, [255, 255, 255], 0.25))
  } else {
    delete root.dataset.accent
    for (const k of ['--accent', '--accent-soft', '--accent-soft2', '--accent-line', '--accent-ink', '--accent-dark']) root.style.removeProperty(k)
  }
  // Масштаб через CSS (движок не умеет, например Astra 1.7) увеличивает и 100vh — высоту окна
  // делим обратно (--css-zoom), иначе низ окна со списками уходит за край без прокрутки.
  const css = (on: boolean) => {
    root.style.zoom = on ? String(a.zoom) : ''
    root.style.setProperty('--css-zoom', on ? String(a.zoom) : '1')
  }
  invoke<boolean>('set_zoom', { factor: a.zoom })
    .then((native) => css(!native && a.zoom !== 1))
    .catch(() => css(a.zoom !== 1))
}

export function getAppearance(): Appearance {
  return current
}

export function setAppearance(patch: Partial<Appearance>) {
  current = { ...current, ...patch }
  try { localStorage.setItem(KEY, JSON.stringify(current)) } catch { /* нет хранилища — только на этот сеанс */ }
  applyAppearance(current)
  listeners.forEach((l) => l(current))
}

/** Всё как было: шрифты, цвет, тема (масштаб и размер текста — тоже). */
export function resetAppearance() {
  setAppearance({ ...DEFAULTS })
}

/** Оформление нового письма: обёртка с выбранным шрифтом (видит получатель). */
export function composeWrap(html: string): string {
  const a = current
  if (!a.composeFont && a.composeSize === DEFAULTS.composeSize) return html
  return `<div style="font-family: ${fontStack(a.composeFont, 'Arial, Helvetica, sans-serif').replace(/"/g, "'")}; font-size: ${a.composeSize}px;">${html}</div>`
}

export function zoomStep(dir: 1 | -1 | 0) {
  if (dir === 0) { setAppearance({ zoom: 1 }); return }
  const i = ZOOM_STEPS.indexOf(current.zoom)
  const next = ZOOM_STEPS[Math.min(ZOOM_STEPS.length - 1, Math.max(0, (i < 0 ? 2 : i) + dir))]
  setAppearance({ zoom: next })
}

export function useAppearance(): Appearance {
  const [a, setA] = useState(current)
  useEffect(() => { listeners.add(setA); return () => { listeners.delete(setA) } }, [])
  return a
}

/** Ширина колонки, которую человек перетащил разделителем (по ключу), в px. */
export function savedWidth(key: string, fallback: number): number {
  try {
    const v = Number(localStorage.getItem(`atom.width.${key}`))
    return Number.isFinite(v) && v > 0 ? v : fallback
  } catch {
    return fallback
  }
}

export function saveWidth(key: string, px: number | null) {
  try {
    if (px === null) localStorage.removeItem(`atom.width.${key}`)
    else localStorage.setItem(`atom.width.${key}`, String(Math.round(px)))
  } catch { /* без хранилища — до перезапуска */ }
}
