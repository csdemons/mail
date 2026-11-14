import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'

import type { MessageSummary } from './types'

/** Перетаскивание писем на папку (указатель, а не HTML5 DnD — одинаково в WebView2 и WebKitGTK).
 *  `dragInfo` — где сейчас «призрак» и над какой папкой; `suppressRowClick` — отпускание после
 *  перетаскивания не должно открывать письмо. */
export function useMessageDrag(onDrop: (items: MessageSummary[], target: string) => void) {
  const [dragInfo, setDragInfo] = useState<{ x: number; y: number; count: number; over: string | null } | null>(null)
  const msgDrag = useRef<{ x0: number; y0: number; items: MessageSummary[]; active: boolean } | null>(null)
  const suppressRowClick = useRef(false)
  useEffect(() => {
    function folderAt(x: number, y: number): string | null {
      const el = document.elementFromPoint(x, y)?.closest<HTMLElement>('[data-folder-id]')
      return el?.dataset.folderId ?? null
    }
    function onMove(e: PointerEvent) {
      const d = msgDrag.current
      if (!d) return
      if (!d.active) {
        if (Math.abs(e.clientX - d.x0) + Math.abs(e.clientY - d.y0) < 8) return
        d.active = true
      }
      setDragInfo({ x: e.clientX, y: e.clientY, count: d.items.length, over: folderAt(e.clientX, e.clientY) })
    }
    function onUp(e: PointerEvent) {
      const d = msgDrag.current
      msgDrag.current = null
      if (!d?.active) return
      setDragInfo(null)
      suppressRowClick.current = true
      setTimeout(() => { suppressRowClick.current = false }, 0)
      const target = folderAt(e.clientX, e.clientY)
      if (target) onDrop(d.items, target)
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    return () => { window.removeEventListener('pointermove', onMove); window.removeEventListener('pointerup', onUp) }
  })

  /** Нажатие на строку: перетаскивание начнётся, если указатель сдвинется. */
  function beginDrag(e: ReactPointerEvent, items: MessageSummary[]) {
    if (e.button !== 0 || e.ctrlKey || e.shiftKey || e.metaKey) return
    msgDrag.current = { x0: e.clientX, y0: e.clientY, items, active: false }
  }

  return { dragInfo, beginDrag, suppressRowClick }
}
