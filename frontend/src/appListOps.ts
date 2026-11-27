// Операции со списком писем — вынесено из App.tsx, чтобы файл не разрастался.
// Функции перенесены целиком; зависимости приходят из App явно.
import type { Dispatch, MutableRefObject, SetStateAction } from 'react'

import { api, errorText, messageKey, PAGE_SIZE } from './api'
import { savePrefs, type NotifyPrefs } from './notifications'
import type { ConnectionStatus, Folder, MessageDetail, MessageSummary } from './types'

export interface ListOpsDeps {
  setNotice: (text: string) => void
  setPrefs: Dispatch<SetStateAction<NotifyPrefs>>
  conn: { setStatus: (status: ConnectionStatus) => void }
  setCheckingConn: Dispatch<SetStateAction<boolean>>
  messages: MessageSummary[]
  folderId: string
  listRequest: MutableRefObject<number>
  setLoadingMore: Dispatch<SetStateAction<boolean>>
  setMessages: Dispatch<SetStateAction<MessageSummary[]>>
  setTotal: Dispatch<SetStateAction<number>>
  setHasMore: Dispatch<SetStateAction<boolean>>
  total: number
  loadFolders: () => Promise<unknown>
  loadFirstPage: (folder: string, keepSelection: boolean) => Promise<number | void>
  setSearchResults: (update: (prev: MessageSummary[] | null) => MessageSummary[] | null) => void
  setDetail: Dispatch<SetStateAction<MessageDetail | null>>
  setFolders: Dispatch<SetStateAction<Folder[]>>
}

export function createListOps(deps: ListOpsDeps) {
  const {
    setNotice, setPrefs, conn, setCheckingConn, messages, folderId, listRequest, setLoadingMore,
    setMessages, setTotal, setHasMore, total, loadFolders, loadFirstPage,
    setSearchResults, setDetail, setFolders,
  } = deps

  function updatePrefs(patch: Partial<NotifyPrefs>) {
    setPrefs((p) => {
      const next = { ...p, ...patch }
      savePrefs(next)
      return next
    })
  }

  async function checkConnection() {
    setCheckingConn(true)
    try {
      const s = await api.offlineCheck()
      conn.setStatus(s)
      if (!s.online) setNotice('Связи с сервером пока нет.')
    } catch (error) {
      setNotice(errorText(error))
    } finally {
      setCheckingConn(false)
    }
  }

  async function loadMore() {
    const request = listRequest.current
    setLoadingMore(true)
    try {
      const oldest = messages.reduce<number | null>((min, m) => (min === null || m.uid < min ? m.uid : min), null)
      const page = await api.listMessages(folderId, oldest, PAGE_SIZE)
      if (request !== listRequest.current) return
      setMessages((items) => {
        const known = new Set(items.map(messageKey))
        return [...items, ...page.items.filter((m) => !known.has(messageKey(m)))]
      })
      setTotal(page.total)
      setHasMore(page.hasMore)
    } catch (error) {
      setNotice(`Не удалось загрузить письма: ${errorText(error)}`)
    } finally {
      setLoadingMore(false)
    }
  }

  async function refresh() {
    const [, count] = await Promise.all([loadFolders(), loadFirstPage(folderId, true)])
    if (typeof count === 'number') setNotice(`Список обновлён: ${count} из ${total || count} писем загружено.`)
  }

  function patchMessage(key: string, patch: Partial<MessageSummary>) {
    setMessages((items) => items.map((m) => (messageKey(m) === key ? { ...m, ...patch } : m)))
    setSearchResults((items) => items && items.map((m) => (messageKey(m) === key ? { ...m, ...patch } : m)))
    setDetail((d) => (d && messageKey(d) === key ? { ...d, ...patch } : d))
  }

  function adjustUnseen(folder: string, delta: number) {
    setFolders((list) => list.map((f) => (f.id === folder ? { ...f, unseen: Math.max(0, f.unseen + delta) } : f)))
  }

  return { updatePrefs, checkConnection, loadMore, refresh, patchMessage, adjustUnseen }
}
