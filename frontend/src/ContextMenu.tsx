import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { ReactNode } from 'react'

export type MenuItem =
  | { separator: true }
  | { label: string; icon?: ReactNode; hint?: string; danger?: boolean; disabled?: boolean; onSelect: () => void }

/** Контекстное меню (правая кнопка мыши), как в Outlook: у курсора, не выходит за окно,
 *  закрывается щелчком мимо, Esc, прокруткой; стрелки и Enter — с клавиатуры. */
export default function ContextMenu({ x, y, items, onClose }: { x: number; y: number; items: MenuItem[]; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState({ left: x, top: y })
  const [active, setActive] = useState(-1)
  const actionable = items.map((it, i) => ('separator' in it || it.disabled ? -1 : i)).filter((i) => i >= 0)

  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const r = el.getBoundingClientRect()
    const left = Math.max(4, Math.min(x, window.innerWidth - r.width - 4))
    const top = y + r.height > window.innerHeight - 4 ? Math.max(4, y - r.height) : y
    setPos({ left, top })
    el.focus()
  }, [x, y])

  useEffect(() => {
    const close = (e: Event) => {
      if (e.type === 'mousedown' && ref.current?.contains(e.target as Node)) return
      onClose()
    }
    // Esc закрывает меню, даже если фокус ушёл из меню (и не доходит до горячих клавиш окна).
    const esc = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.stopPropagation(); e.preventDefault(); onClose() } }
    window.addEventListener('keydown', esc, true)
    window.addEventListener('mousedown', close, true)
    window.addEventListener('wheel', close, { capture: true, passive: true })
    window.addEventListener('resize', close)
    window.addEventListener('blur', close)
    return () => {
      window.removeEventListener('keydown', esc, true)
      window.removeEventListener('mousedown', close, true)
      window.removeEventListener('wheel', close, true)
      window.removeEventListener('resize', close)
      window.removeEventListener('blur', close)
    }
  }, [onClose])

  function onKey(e: React.KeyboardEvent) {
    e.stopPropagation()
    if (e.key === 'Escape') { e.preventDefault(); onClose(); return }
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      const pos = actionable.indexOf(active)
      const next = e.key === 'ArrowDown' ? (pos + 1) % actionable.length : (pos <= 0 ? actionable.length - 1 : pos - 1)
      setActive(actionable[next])
    }
    if (e.key === 'Enter' && active >= 0) {
      e.preventDefault()
      const it = items[active]
      if (!('separator' in it)) { onClose(); it.onSelect() }
    }
  }

  return (
    <div ref={ref} className="context-menu" role="menu" tabIndex={-1} style={pos} onKeyDown={onKey} onContextMenu={(e) => e.preventDefault()}>
      {items.map((it, i) => 'separator' in it
        ? <div key={i} className="context-sep" role="separator" />
        : (
          <button
            key={i}
            role="menuitem"
            className={`${it.danger ? 'danger' : ''} ${active === i ? 'active' : ''}`}
            disabled={it.disabled}
            onMouseEnter={() => setActive(i)}
            onClick={() => { onClose(); it.onSelect() }}
          >
            <span className="context-icon">{it.icon}</span>
            <span className="context-label">{it.label}</span>
            {it.hint && <kbd>{it.hint}</kbd>}
          </button>
        ))}
    </div>
  )
}
