import { useEffect, useRef } from 'react'
import { saveWidth, savedWidth } from './appearance'

/**
 * Перетаскиваемый разделитель колонок. Меняет ширину соседней колонки (по умолчанию —
 * предыдущей, `side="next"` — следующей, для правой панели) и запоминает её по `id`.
 * Двойной щелчок — исходная ширина; стрелки ← → — с клавиатуры.
 */
export default function Splitter({ id, min = 180, max = 900, side = 'prev', label = 'Изменить ширину колонки' }: {
  id: string
  min?: number
  max?: number
  side?: 'prev' | 'next'
  label?: string
}) {
  const ref = useRef<HTMLDivElement>(null)

  function pane(): HTMLElement | null {
    const el = ref.current
    return (side === 'prev' ? el?.previousElementSibling : el?.nextElementSibling) as HTMLElement | null
  }
  function apply(px: number | null) {
    const p = pane()
    if (!p) return
    if (px === null) {
      p.classList.remove('sized')
      p.style.removeProperty('--w')
    } else {
      p.classList.add('sized')
      p.style.setProperty('--w', `${Math.round(px)}px`)
    }
  }
  function clamp(px: number): number {
    return Math.max(min, Math.min(max, px, window.innerWidth * 0.75))
  }

  useEffect(() => {
    const w = savedWidth(id, 0)
    if (w > 0) apply(clamp(w))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id])

  function onPointerDown(e: React.PointerEvent<HTMLDivElement>) {
    const p = pane()
    if (!p || e.button !== 0) return
    e.preventDefault()
    const start = e.clientX
    const startW = p.getBoundingClientRect().width
    const el = e.currentTarget
    try { el.setPointerCapture(e.pointerId) } catch { /* синтетическое событие без указателя */ }
    el.classList.add('dragging')
    document.body.classList.add('resizing')
    let last = startW
    const move = (ev: PointerEvent) => {
      const dx = (ev.clientX - start) * (side === 'prev' ? 1 : -1)
      last = clamp(startW + dx)
      apply(last)
    }
    const up = () => {
      el.removeEventListener('pointermove', move)
      el.removeEventListener('pointerup', up)
      el.removeEventListener('pointercancel', up)
      el.classList.remove('dragging')
      document.body.classList.remove('resizing')
      saveWidth(id, last)
    }
    el.addEventListener('pointermove', move)
    el.addEventListener('pointerup', up)
    el.addEventListener('pointercancel', up)
  }

  function onKeyDown(e: React.KeyboardEvent) {
    const p = pane()
    if (!p || (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight')) return
    e.preventDefault()
    const grow = (e.key === 'ArrowRight') === (side === 'prev')
    const w = clamp(p.getBoundingClientRect().width + (grow ? 16 : -16))
    apply(w)
    saveWidth(id, w)
  }

  return (
    <div ref={ref} className="splitter" role="separator" aria-orientation="vertical" aria-label={label} tabIndex={0}
      title="Потяните, чтобы изменить ширину; двойной щелчок — как было"
      onPointerDown={onPointerDown} onKeyDown={onKeyDown}
      onDoubleClick={() => { apply(null); saveWidth(id, null) }} />
  )
}
