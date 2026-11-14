import { useEffect, useState } from 'react'

import { api, errorText } from './api'
import { QUICK_VIEWS } from './mailLabels'
import type { SavedView } from './types'

/** Сохранённые поиски и быстрые представления: список, запуск, сохранение текущего поиска. */
export function useSavedViews(opts: {
  query: string
  searchScope: 'current' | 'all'
  multi: boolean
  /** Открыть почту с этим поиском. */
  onRun: (v: SavedView, scope: 'current' | 'all') => void
  setNotice: (text: string) => void
}) {
  const { query, searchScope, multi, onRun, setNotice } = opts
  /** Сохранённые поиски и быстрые представления. */
  const [views, setViews] = useState<SavedView[]>([])
  const [viewsOpen, setViewsOpen] = useState(() => { try { return localStorage.getItem('rrcki-mail.views-open') !== 'off' } catch { return true } })
  useEffect(() => { try { localStorage.setItem('rrcki-mail.views-open', viewsOpen ? 'on' : 'off') } catch { /* необязательно */ } }, [viewsOpen])
  useEffect(() => { api.viewsGet().then(setViews).catch(() => undefined) }, [])
  const [saveViewName, setSaveViewName] = useState<string | null>(null)
  function runView(v: SavedView) {
    onRun(v, v.scope === 'all' && multi ? 'all' : 'current')
  }
  async function storeViews(next: SavedView[], done: string) {
    try {
      await api.viewsSave(next)
      setViews(next)
      setNotice(done)
    } catch (e) { setNotice(errorText(e)) }
  }
  function saveCurrentSearch() {
    const name = (saveViewName ?? '').trim()
    const q = query.trim()
    if (!name || q.length < 2) return
    const view: SavedView = { id: `v${Date.now().toString(36)}`, name: name.slice(0, 60), query: q, scope: searchScope }
    setSaveViewName(null)
    void storeViews([...views.filter((v) => v.name !== view.name), view], `Поиск «${view.name}» сохранён`)
  }
  const activeView = [...QUICK_VIEWS, ...views].find((v) => v.query === query.trim() && (v.scope === searchScope || !multi))

  return { views, viewsOpen, setViewsOpen, saveViewName, setSaveViewName, runView, storeViews, saveCurrentSearch, activeView }
}
