// Хук роста поля ввода: пересчитать высоту при смене текста и при смене ширины поля
// (окно, разделитель, появление панели) — иначе строки переливаются заново, а высота старая.
import { useLayoutEffect, type RefObject } from 'react'

import { autoGrow, COMPOSE_MAX_PX } from './autoGrow'

export function useAutoGrow(ref: RefObject<HTMLTextAreaElement | null>, value: string, maxPx = COMPOSE_MAX_PX) {
  useLayoutEffect(() => { autoGrow(ref.current, maxPx) }, [value, maxPx])
  useLayoutEffect(() => {
    const el = ref.current
    if (!el || typeof ResizeObserver === 'undefined') return
    let width = el.clientWidth
    const observer = new ResizeObserver(() => {
      // Высота меняется от нашего же autoGrow — пересчитываем только при смене ширины,
      // иначе наблюдатель вызывал бы сам себя.
      if (el.clientWidth === width) return
      width = el.clientWidth
      autoGrow(el, maxPx)
    })
    observer.observe(el)
    return () => observer.disconnect()
  }, [maxPx])
}
