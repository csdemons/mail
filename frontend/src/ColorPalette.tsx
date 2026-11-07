import { useEffect, useRef, useState, type ReactNode } from 'react'

const COLORS = [
  '#000000', '#434343', '#666666', '#999999', '#cccccc', '#ffffff',
  '#d0021b', '#e8590c', '#f5a623', '#f8e71c', '#7ed321', '#2e7d32',
  '#00a3a3', '#4a90e2', '#1f5fbf', '#6f42c1', '#bd10e0', '#c2185b',
  '#ffcdd2', '#ffe0b2', '#fff59d', '#c8e6c9', '#bbdefb', '#e1bee7',
]

/**
 * Выбор цвета в редакторе: палитра, щелчок по цвету — сразу применить. «Другой цвет…» —
 * системный выбор с кнопкой «Применить». Кнопки не забирают выделение у текста.
 */
export default function ColorPalette({ label, icon, current, onPick, clearLabel }: {
  label: string
  icon: ReactNode
  current: string
  onPick: (color: string | null) => void
  clearLabel: string
}) {
  const [open, setOpen] = useState(false)
  const [custom, setCustom] = useState(current)
  const ref = useRef<HTMLSpanElement>(null)

  useEffect(() => {
    if (!open) return
    const close = (e: MouseEvent) => { if (!ref.current?.contains(e.target as Node)) setOpen(false) }
    const esc = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.stopPropagation(); setOpen(false) } }
    document.addEventListener('mousedown', close, true)
    document.addEventListener('keydown', esc, true)
    return () => { document.removeEventListener('mousedown', close, true); document.removeEventListener('keydown', esc, true) }
  }, [open])

  function pick(c: string | null) {
    setOpen(false)
    onPick(c)
  }
  const keep = (e: React.MouseEvent) => e.preventDefault()

  return (
    <span className="rt-palette-wrap" ref={ref}>
      <button type="button" className={`rt-btn rt-color ${open ? 'active' : ''}`} title={label} aria-label={label} aria-haspopup="dialog" aria-expanded={open}
        onMouseDown={(e) => { e.preventDefault(); setOpen(!open) }}>
        {icon}<i className="rt-color-bar" style={{ background: current }} />
      </button>
      {open && (
        <div className="rt-palette" role="dialog" aria-label={label} onMouseDown={keep}>
          <div className="rt-palette-grid">
            {COLORS.map((c) => (
              <button key={c} type="button" className={`rt-swatch ${c === current ? 'current' : ''}`} style={{ background: c }} title={c} aria-label={`Цвет ${c}`}
                onMouseDown={(e) => { e.preventDefault(); pick(c) }} />
            ))}
          </div>
          <button type="button" className="text-button rt-palette-clear" onMouseDown={(e) => { e.preventDefault(); pick(null) }}>{clearLabel}</button>
          <div className="rt-palette-custom">
            <input type="color" aria-label="Другой цвет" value={custom} onChange={(e) => setCustom(e.target.value)} />
            <span>Другой цвет</span>
            <button type="button" className="primary-button small" onMouseDown={(e) => { e.preventDefault(); pick(custom) }}>Применить</button>
          </div>
        </div>
      )}
    </span>
  )
}
