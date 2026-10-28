import { useMemo, useState } from 'react'
import { Folder as FolderIcon, Search, X } from 'lucide-react'
import type { Folder } from './types'

/** Корень пространства общих ящиков (не выбираемая папка верхнего уровня, например «shared»). */
function sharedMailbox(folders: Folder[], folder: Folder): string | null {
  const byId = new Map(folders.map((f) => [f.id, f]))
  const chain: Folder[] = []
  for (let f: Folder | undefined = folder; f; f = f.parent ? byId.get(f.parent) : undefined) chain.unshift(f)
  const top = chain[0]
  if (chain.length < 2 || top.selectable || top.role !== 'other') return null
  return chain[1].id
}

/** Корзина того же ящика: для подключённого ящика — его корзина, иначе — своя. */
export function trashFor(folders: Folder[], folderId: string): Folder | null {
  const folder = folders.find((f) => f.id === folderId)
  if (!folder) return null
  const box = sharedMailbox(folders, folder)
  return folders.find((f) => f.role === 'trash' && f.selectable && sharedMailbox(folders, f) === box && (box !== null || f.system))
    ?? folders.find((f) => f.role === 'trash' && f.selectable && sharedMailbox(folders, f) === box)
    ?? null
}

/** Папки своего ящика (фильтры сервера раскладывают письма только в них). */
export function personalFolders(folders: Folder[]): Folder[] {
  return folders.filter((f) => sharedMailbox(folders, f) === null && !(f.parent === null && !f.selectable && f.role === 'other' && folders.some((c) => c.parent === f.id && sharedMailbox(folders, c) !== null)))
}

interface Props {
  folders: Folder[]
  exclude?: string
  title: string
  personalOnly?: boolean
  label: (f: Folder) => string
  onPick: (id: string) => void
  onClose: () => void
}

export default function FolderPicker({ folders, exclude, title, personalOnly, label, onPick, onClose }: Props) {
  const [q, setQ] = useState('')
  const list = useMemo(() => {
    const base = personalOnly ? personalFolders(folders) : folders
    const needle = q.trim().toLowerCase()
    return base.filter((f) => !needle || f.path.toLowerCase().includes(needle) || label(f).toLowerCase().includes(needle))
  }, [folders, personalOnly, q, label])

  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}>
      <div className="settings-modal folder-picker" role="dialog" aria-label={title}>
        <div className="compose-header">
          <div><strong>{title}</strong><small>Выберите папку</small></div>
          <button className="icon-button" onClick={onClose} aria-label="Закрыть"><X size={18} /></button>
        </div>
        <label className="search-box"><Search size={16} /><input autoFocus value={q} onChange={(e) => setQ(e.target.value)} placeholder="Найти папку" /></label>
        <div className="folder-picker-list">
          {list.map((f) => (
            <button
              key={f.id}
              className="folder-item"
              style={{ paddingLeft: 8 + (q ? 0 : f.depth * 14) }}
              disabled={!f.selectable || f.id === exclude}
              title={f.path}
              onClick={() => onPick(f.id)}
            >
              <FolderIcon size={16} /><span>{q ? f.path : label(f)}</span>
            </button>
          ))}
          {list.length === 0 && <div className="folder-placeholder">Папки не найдены</div>}
        </div>
      </div>
    </div>
  )
}
