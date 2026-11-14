import { useEffect, useMemo, useRef, useState } from 'react'

import { api, errorText, messageKey, TYPING_PAUSE_MS } from './api'
import type { MessageSummary, UnifiedItem } from './types'

/** Поиск писем: локальный индекс (с задержкой после ввода), сервер по кнопке и,
 *  в области «Все ящики», остальные учётные записи. */
export function useMailSearch(initial: { query?: string; searchScope?: 'current' | 'all' } | null | undefined, multi: boolean, setNotice: (text: string) => void) {
  const [query, setQuery] = useState(initial?.query ?? '')
  const queryRef = useRef('')
  queryRef.current = query
  /** Поиск по всей почте: результаты из локального индекса (и, по кнопке, с сервера). */
  const [localResults, setLocalResults] = useState<MessageSummary[] | null>(null)
  const [serverResults, setServerResults] = useState<MessageSummary[] | null>(null)
  /** Поиск в других учётных записях (область «Все ящики»). */
  const [searchScope, setSearchScope] = useState<'current' | 'all'>(initial?.searchScope ?? 'current')
  const [otherResults, setOtherResults] = useState<UnifiedItem[] | null>(null)
  const [otherErrors, setOtherErrors] = useState<string[]>([])
  const [othersBusy, setOthersBusy] = useState(false)
  const [searchBusy, setSearchBusy] = useState(false)
  const serverSearched = serverResults !== null
  /** Найденное в индексе и (по кнопке) на сервере — без повторов, новые сверху. */
  const searchResults = useMemo(() => {
    if (localResults === null && serverResults === null && otherResults === null) return null
    const seen = new Set<string>()
    return [...(localResults ?? []), ...(serverResults ?? []), ...(otherResults ?? [])]
      .filter((m) => { const k = messageKey(m); if (seen.has(k)) return false; seen.add(k); return true })
      .sort((a, b) => (Date.parse(b.date ?? '') || 0) - (Date.parse(a.date ?? '') || 0))
  }, [localResults, serverResults, otherResults])
  function setSearchResults(update: (items: MessageSummary[] | null) => MessageSummary[] | null) {
    setLocalResults(update)
    setServerResults(update)
  }
  const searching = searchResults !== null
  function clearSearch() {
    setQuery('')
    setLocalResults(null)
    setServerResults(null)
    setOtherResults(null)
    setOtherErrors([])
  }

  // Поиск по всей почте (индекс на компьютере), с задержкой после ввода.
  useEffect(() => {
    const q = query.trim()
    setServerResults(null)
    if (q.length < 2) { setLocalResults(null); return }
    let cancelled = false
    const timer = window.setTimeout(() => {
      api.searchMail(q).then((found) => { if (!cancelled) setLocalResults(found) }).catch(() => { if (!cancelled) setLocalResults([]) })
    }, TYPING_PAUSE_MS)
    return () => { cancelled = true; window.clearTimeout(timer) }
  }, [query])

  // «Все ящики»: те же условия на серверах остальных учётных записей.
  useEffect(() => {
    const q = query.trim()
    setOtherResults(null)
    setOtherErrors([])
    if (q.length < 2 || searchScope !== 'all' || !multi) return
    let cancelled = false
    const timer = window.setTimeout(() => {
      setOthersBusy(true)
      api.searchOthers(q)
        .then((r) => { if (!cancelled) { setOtherResults(r.items); setOtherErrors(r.errors) } })
        .catch((e) => { if (!cancelled) { setOtherResults([]); setOtherErrors([errorText(e)]) } })
        .finally(() => { if (!cancelled) setOthersBusy(false) })
    }, 600)
    return () => { cancelled = true; window.clearTimeout(timer) }
  }, [query, searchScope, multi])

  async function searchOnServer() {
    const q = query.trim()
    if (q.length < 2) return
    setSearchBusy(true)
    try {
      const found = await api.searchServer(q)
      if (queryRef.current.trim() === q) setServerResults(found)
    } catch (error) {
      setNotice(`Поиск на сервере не удался: ${errorText(error)}`)
    } finally {
      setSearchBusy(false)
    }
  }

  return {
    query, setQuery, searchScope, setSearchScope, otherErrors, othersBusy, searchBusy, serverSearched,
    searchResults, setSearchResults, searching, clearSearch, searchOnServer,
  }
}
