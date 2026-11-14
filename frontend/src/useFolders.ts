import { useCallback, useMemo, useState } from 'react'

import { api, errorText } from './api'
import { folderLabel, loadExpanded, saveExpanded } from './mailLabels'
import type { Folder } from './types'

/** Папки учётной записи: список (сначала с компьютера, затем с сервера), открытая папка,
 *  раскрытые узлы дерева и непрочитанные под свёрнутыми узлами. */
export function useFolders(initialFolderId: string | undefined, setNotice: (text: string) => void) {
  const [folders, setFolders] = useState<Folder[]>([])
  const [folderId, setFolderId] = useState(initialFolderId ?? 'INBOX')
  const [expanded, setExpanded] = useState<Set<string>>(loadExpanded)

  const currentFolder = folders.find((f) => f.id === folderId)
  function folderName(id: string): string {
    const f = folders.find((x) => x.id === id)
    return f ? folderLabel(f) : id
  }

  const visibleFolders = useMemo(() => {
    const byId = new Map(folders.map((f) => [f.id, f]))
    return folders.filter((f) => {
      for (let p = f.parent; p; p = byId.get(p)?.parent ?? null) {
        if (!expanded.has(p)) return false
      }
      return true
    })
  }, [folders, expanded])

  // Непрочитанные под свёрнутым узлом (например, в подключённом ящике) — видно и без раскрытия.
  const nodeUnseen = useMemo(() => {
    const byId = new Map(folders.map((f) => [f.id, f]))
    const out = new Map<string, number>()
    for (const f of folders) {
      if (!f.selectable || !f.unseen) continue
      for (let p = f.parent; p; p = byId.get(p)?.parent ?? null) {
        if (byId.get(p)?.selectable === false) out.set(p, (out.get(p) ?? 0) + f.unseen)
      }
    }
    return out
  }, [folders])

  function toggleExpanded(id: string) {
    setExpanded((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      saveExpanded(next)
      return next
    })
  }

  const loadFolders = useCallback(async () => {
    // Сначала — папки с компьютера (сразу после запуска и разблокировки), затем с сервера.
    api.cachedFolders().then((cached) => {
      if (cached?.length) setFolders((cur) => (cur.length ? cur : cached))
    }).catch(() => undefined)
    try {
      const list = await api.listFolders()
      setFolders(list)
      setFolderId((id) => (list.some((f) => f.id === id && f.selectable)
        ? id
        : list.find((f) => f.role === 'inbox')?.id ?? list.find((f) => f.selectable)?.id ?? 'INBOX'))
    } catch (error) {
      setNotice(`Не удалось получить список папок: ${errorText(error)}`)
    }
  }, [])

  return { folders, setFolders, folderId, setFolderId, expanded, toggleExpanded, currentFolder, folderName, visibleFolders, nodeUnseen, loadFolders }
}
