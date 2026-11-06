import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'

/** Пункт контекстного меню (правая кнопка мыши). */
export interface MenuItem {
  label?: string
  icon?: ReactNode
  onClick?: () => void
  danger?: boolean
  disabled?: boolean
  /** Разделитель вместо пункта. */
  separator?: boolean
  /** Свой ряд (например, быстрые реакции) вместо обычного пункта. */
  custom?: ReactNode
}

type Opener = (x: number, y: number, items: MenuItem[]) => void
let opener: Opener | null = null

/** Открыть меню у курсора (из onContextMenu) или у кнопки «⋯» (из onClick). */
export function openContextMenu(e: { clientX: number; clientY: number; preventDefault(): void; stopPropagation(): void; currentTarget?: EventTarget | null }, items: MenuItem[]) {
  e.preventDefault()
  e.stopPropagation()
  let { clientX: x, clientY: y } = e
  // С клавиатуры (Shift+F10, клавиша меню) координаты нулевые — открываем у элемента.
  if (x === 0 && y === 0 && e.currentTarget instanceof Element) {
    const r = e.currentTarget.getBoundingClientRect()
    x = r.left + 12
    y = r.top + Math.min(r.height, 28)
  }
  opener?.(x, y, items.filter((i, n, all) => !(i.separator && (n === 0 || n === all.length - 1 || all[n - 1].separator))))
}

function editable(t: EventTarget | null): boolean {
  if (!(t instanceof Element)) return false
  return Boolean(t.closest('input, textarea, [contenteditable="true"], [contenteditable=""]'))
}

/**
 * Хост меню: один на окно. Заодно убирает стандартное меню веб-движка («Назад», «Обновить»,
 * «Печать»…) — оно здесь не к месту. В полях ввода остаётся системное (вырезать/копировать/
 * вставить, орфография); на выделенном тексте — «Копировать».
 */
export function ContextMenuHost() {
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null)
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null)
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    opener = (x, y, items) => { setPos(null); setMenu({ x, y, items }) }
    const onContext = (e: MouseEvent) => {
      if (e.defaultPrevented || editable(e.target)) return
      e.preventDefault()
      const sel = window.getSelection()?.toString() ?? ''
      if (sel.trim()) {
        opener?.(e.clientX, e.clientY, [{ label: 'Копировать', onClick: () => void navigator.clipboard.writeText(sel) }])
      }
    }
    document.addEventListener('contextmenu', onContext)
    return () => { opener = null; document.removeEventListener('contextmenu', onContext) }
  }, [])

  useEffect(() => {
    if (!menu) return
    const close = () => setMenu(null)
    const onDown = (e: MouseEvent) => { if (!ref.current?.contains(e.target as Node)) close() }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { e.preventDefault(); close(); return }
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault()
        const btns = [...(ref.current?.querySelectorAll<HTMLButtonElement>('button.ctx-item:not(:disabled)') ?? [])]
        const i = btns.indexOf(document.activeElement as HTMLButtonElement)
        btns[(i + (e.key === 'ArrowDown' ? 1 : -1) + btns.length) % btns.length]?.focus()
      }
    }
    document.addEventListener('mousedown', onDown, true)
    document.addEventListener('keydown', onKey, true)
    window.addEventListener('blur', close)
    window.addEventListener('resize', close)
    document.addEventListener('scroll', close, true)
    return () => {
      document.removeEventListener('mousedown', onDown, true)
      document.removeEventListener('keydown', onKey, true)
      window.removeEventListener('blur', close)
      window.removeEventListener('resize', close)
      document.removeEventListener('scroll', close, true)
    }
  }, [menu])

  // Не выходим за край окна.
  useLayoutEffect(() => {
    if (!menu || !ref.current) return
    const r = ref.current.getBoundingClientRect()
    const left = Math.max(4, Math.min(menu.x, window.innerWidth - r.width - 4))
    const top = menu.y + r.height > window.innerHeight - 4 ? Math.max(4, menu.y - r.height) : menu.y
    setPos({ left, top })
    ref.current.querySelector<HTMLButtonElement>('button.ctx-item:not(:disabled)')?.focus({ preventScroll: true })
  }, [menu])

  if (!menu) return null
  return (
    <div ref={ref} className="ctx-menu" role="menu" style={{ left: pos?.left ?? menu.x, top: pos?.top ?? menu.y, visibility: pos ? 'visible' : 'hidden' }}
      onContextMenu={(e) => e.preventDefault()}>
      {menu.items.map((it, i) => it.separator ? <div key={i} className="ctx-sep" role="separator" />
        : it.custom ? <div key={i} className="ctx-custom" onClick={() => setMenu(null)}>{it.custom}</div>
        : (
          <button key={i} type="button" role="menuitem" className={`ctx-item ${it.danger ? 'danger' : ''}`} disabled={it.disabled}
            onClick={() => { setMenu(null); it.onClick?.() }}>
            <span className="ctx-icon">{it.icon}</span><span>{it.label}</span>
          </button>
        ))}
    </div>
  )
}
