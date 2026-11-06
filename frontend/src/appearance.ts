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
}

export const ZOOM_STEPS = [0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75]
export const TEXT_MIN = 12
export const TEXT_MAX = 22
const KEY = 'atom.appearance'
const DEFAULTS: Appearance = { zoom: 1, text: 14 }

function read(): Appearance {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? '{}') as Partial<Appearance>
    const zoom = ZOOM_STEPS.includes(Number(v.zoom)) ? Number(v.zoom) : DEFAULTS.zoom
    const text = Number.isFinite(v.text) ? Math.min(TEXT_MAX, Math.max(TEXT_MIN, Number(v.text))) : DEFAULTS.text
    return { zoom, text }
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
  invoke<boolean>('set_zoom', { factor: a.zoom })
    .then((native) => { root.style.zoom = native || a.zoom === 1 ? '' : String(a.zoom) })
    .catch(() => { root.style.zoom = a.zoom === 1 ? '' : String(a.zoom) })
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
