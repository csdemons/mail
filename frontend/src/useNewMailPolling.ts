import { useEffect, useRef, type MutableRefObject } from 'react'

import { api } from './api'
import { sharedMailboxOf } from './FolderPicker'
import { senderLabel } from './mailLabels'
import { appInFront, notify, type NotifyPrefs } from './notifications'
import type { Folder, MessageSummary } from './types'

/** Новая почта: «Входящие» (свои и, по настройке, подключённых ящиков) проверяются по UID —
 *  надёжно; остальные папки (куда раскладывают фильтры) — по росту числа непрочитанных.
 *  Заодно обновляет список папок (счётчики) и, если новое пришло в открытую папку, — список писем. */
export function useNewMailPolling(opts: {
  folderIdRef: MutableRefObject<string>
  prefsRef: MutableRefObject<NotifyPrefs>
  pollSecs: MutableRefObject<number>
  setFolders: (list: Folder[]) => void
  setNotice: (text: string) => void
  loadFirstPage: (folder: string, keepSelection: boolean) => Promise<unknown>
}) {
  const { folderIdRef, prefsRef, pollSecs, setFolders, setNotice, loadFirstPage } = opts
  const unseenSeen = useRef(new Map<string, number>())
  const lastUid = useRef(new Map<string, number>())
  useEffect(() => {
    let stopped = false
    let timer: ReturnType<typeof setTimeout> | undefined
    const skip = new Set(['trash', 'junk', 'sent', 'drafts'])
    function announce(f: Folder, all: Folder[], items: MessageSummary[], count: number) {
      const p = prefsRef.current
      const where = f.role === 'inbox' && !sharedMailboxOf(all, f) ? '' : ` («${f.path}»)`
      if (appInFront()) {
        const m = items[0]
        setNotice(count > 1 ? `Новых писем: ${count}${where}` : `Новое письмо от ${senderLabel(m)}${p.preview && m.subject ? `: ${m.subject}` : ''}`)
        return
      }
      if (count > 3) {
        notify(`Новых писем: ${count}${where}`, items.slice(0, 3).map((m) => senderLabel(m)).join(', '))
        return
      }
      for (const m of items.slice(0, 3)) {
        notify(`Новое письмо — ${senderLabel(m)}${where}`, p.preview ? (m.subject || '(без темы)') : 'Откройте Atom Workspace, чтобы прочитать')
      }
    }
    async function tick() {
      try {
        const list = await api.listFolders()
        if (stopped) return
        setFolders(list)
        const p = prefsRef.current
        for (const f of list) {
          if (!f.selectable || skip.has(f.role)) continue
          const shared = sharedMailboxOf(list, f) !== null
          if (shared && !p.shared) { unseenSeen.current.set(f.id, f.unseen); continue }
          const prev = unseenSeen.current.get(f.id)
          unseenSeen.current.set(f.id, f.unseen)
          const known = lastUid.current.get(f.id)
          const isInbox = f.role === 'inbox'
          // Не «Входящие»: смотрим только если непрочитанных стало больше.
          if (!isInbox && (prev === undefined || f.unseen <= prev)) continue
          if (isInbox && known === undefined) {
            // Первая проверка — запоминаем последний UID, без уведомлений.
            const base = await api.pollNewMail(f.id, 0, 0)
            lastUid.current.set(f.id, base.lastUid)
            continue
          }
          const after = known ?? 0
          const growth = prev === undefined ? 0 : Math.max(0, f.unseen - prev)
          const limit = after ? 5 : Math.min(growth, 5)
          if (limit === 0) continue
          const r = await api.pollNewMail(f.id, after, limit)
          lastUid.current.set(f.id, Math.max(after, r.lastUid))
          if (r.items.length === 0) continue
          if (f.id === folderIdRef.current) void loadFirstPage(f.id, true)
          if (p.mail) announce(f, list, r.items, Math.max(r.items.length, growth))
        }
      } catch { /* сеть — попробуем в следующий раз */ }
      finally {
        if (!stopped) timer = setTimeout(tick, pollSecs.current * 1000)
      }
    }
    timer = setTimeout(tick, 3000)
    return () => { stopped = true; if (timer) clearTimeout(timer) }
  }, [loadFirstPage])
}
