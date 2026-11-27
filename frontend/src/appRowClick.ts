// Клик по строке письма и начало перетаскивания — вынесено из App.tsx.
// Нужен suppressRowClick из useMessageDrag, поэтому фабрика вызывается после этого хука.
import type { Dispatch, MutableRefObject, SetStateAction } from 'react'

import { messageKey } from './api'
import type { MessageSummary, UnifiedItem } from './types'

export interface RowClickDeps {
  suppressRowClick: MutableRefObject<boolean>
  beginDrag: (e: React.PointerEvent, items: MessageSummary[]) => void
  anchorKey: MutableRefObject<string>
  selectedKey: string
  visibleMessages: MessageSummary[]
  checked: Set<string>
  pool: MessageSummary[]
  setChecked: Dispatch<SetStateAction<Set<string>>>
  openMessage: (m: MessageSummary) => Promise<void>
  switchAccount: (account: string, open?: MessageSummary) => void
  otherOf: (m: MessageSummary) => UnifiedItem | null
}

export function createRowClick(deps: RowClickDeps) {
  const { suppressRowClick, beginDrag, anchorKey, selectedKey, visibleMessages, checked, pool, setChecked, openMessage, switchAccount, otherOf } = deps

  function onRowClick(e: React.MouseEvent, m: MessageSummary) {
    if (suppressRowClick.current) return
    const other = otherOf(m)
    if (other) { switchAccount(other.account, other); return }
    const key = messageKey(m)
    if (e.ctrlKey || e.metaKey) {
      setChecked((prev) => {
        const next = new Set(prev.size === 0 && selectedKey ? [selectedKey] : prev)
        if (next.has(key)) next.delete(key)
        else next.add(key)
        return next
      })
      anchorKey.current = key
      return
    }
    if (e.shiftKey && (anchorKey.current || selectedKey)) {
      const keys = visibleMessages.map(messageKey)
      const a = keys.indexOf(anchorKey.current || selectedKey)
      const b = keys.indexOf(key)
      if (a >= 0 && b >= 0) {
        setChecked(new Set(keys.slice(Math.min(a, b), Math.max(a, b) + 1)))
        return
      }
    }
    anchorKey.current = key
    setChecked(new Set())
    void openMessage(m)
  }

  function startMessageDrag(e: React.PointerEvent, m: MessageSummary) {
    const key = messageKey(m)
    beginDrag(e, checked.has(key) ? pool.filter((x) => checked.has(messageKey(x))) : [m])
  }

  return { onRowClick, startMessageDrag }
}
