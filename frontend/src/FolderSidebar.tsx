import { ChevronDown, ChevronRight, Layers, Search, Zap, Bookmark, X, FolderOpen, CalendarClock, CloudOff, ShieldCheck } from 'lucide-react'
import { UNIFIED_ROLES } from './Accounts'
import { ROLE_ICONS, QUICK_VIEWS, pluralLetters, folderLabel } from './mailLabels'
import { SkeletonRows } from './Skeleton'
import type { AccountEntry, UnifiedRole, SavedView, Folder, Scheduled, ConnectionStatus, Account } from './types'

/** Колонка папок: общие папки всех учётных записей, сохранённые поиски, дерево папок, отложенные, состояние связи. */
export interface FolderSidebarProps {
  acc: { accounts: AccountEntry[]; unread: Record<string, number | null>; multi: boolean; reload: () => Promise<void>; }
  unifiedOpen: boolean
  setUnifiedOpen: React.Dispatch<React.SetStateAction<boolean>>
  unified: UnifiedRole | null
  clearSearch: () => void
  setUnified: React.Dispatch<React.SetStateAction<UnifiedRole | null>>
  viewsOpen: boolean
  setViewsOpen: React.Dispatch<React.SetStateAction<boolean>>
  views: SavedView[]
  activeView: SavedView | undefined
  runView: (v: SavedView) => void
  storeViews: (next: SavedView[], done: string) => Promise<void>
  folders: Folder[]
  visibleFolders: Folder[]
  expanded: Set<string>
  toggleExpanded: (id: string) => void
  folderId: string
  dragInfo: { x: number; y: number; count: number; over: string | null; } | null
  setFolderId: React.Dispatch<React.SetStateAction<string>>
  nodeUnseen: Map<string, number>
  scheduled: Scheduled[]
  loadScheduled: () => void
  setShowScheduled: React.Dispatch<React.SetStateAction<boolean>>
  offline: boolean
  setShowQueue: React.Dispatch<React.SetStateAction<boolean>>
  conn: { status: ConnectionStatus | null; setStatus: (s: ConnectionStatus) => void; refresh: () => Promise<void>; }
  account: Account
}

export default function FolderSidebar({ acc, unifiedOpen, setUnifiedOpen, unified, clearSearch, setUnified, viewsOpen, setViewsOpen, views, activeView, runView, storeViews, folders, visibleFolders, expanded, toggleExpanded, folderId, dragInfo, setFolderId, nodeUnseen, scheduled, loadScheduled, setShowScheduled, offline, setShowQueue, conn, account }: FolderSidebarProps) {
  return (
    <aside className="folder-sidebar">
      <div className="sidebar-heading"><span>Папки</span><ChevronDown size={15} /></div>
      <div className="folder-list">
        {acc.multi && <>
          <button className="folder-item folder-node unified-group" aria-expanded={unifiedOpen} onClick={() => setUnifiedOpen(!unifiedOpen)}>
            <span className="folder-toggle">{unifiedOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}</span>
            <Layers size={17} /><span>Все ящики</span>
          </button>
          {unifiedOpen && UNIFIED_ROLES.map((r) => {
            const Icon = ROLE_ICONS[r.role]
            return (
              <button key={r.role} className={`folder-item unified-item ${unified === r.role ? 'selected' : ''}`} data-role={r.role} style={{ paddingLeft: 18 }}
                onClick={() => { clearSearch(); setUnified(r.role) }}>
                <span className="folder-toggle" /><Icon size={17} /><span>{r.title}</span>
              </button>
            )
          })}
          <div className="unified-separator" />
        </>}
        <button className="folder-item folder-node views-group" aria-expanded={viewsOpen} onClick={() => setViewsOpen(!viewsOpen)}>
          <span className="folder-toggle">{viewsOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}</span>
          <Search size={17} /><span>Поиски</span>
        </button>
        {viewsOpen && [...QUICK_VIEWS, ...views].map((v) => {
          const quick = v.id.startsWith('q-')
          return (
            <div key={v.id} className={`folder-item view-item ${activeView?.id === v.id ? 'selected' : ''}`} data-view={v.id} style={{ paddingLeft: 18 }} role="button" tabIndex={0}
              title={`${v.query}${v.scope === 'all' ? ' — все ящики' : ''}`}
              onClick={() => runView(v)} onKeyDown={(e) => { if (e.key === 'Enter') runView(v) }}>
              <span className="folder-toggle" />{quick ? <Zap size={16} /> : <Bookmark size={16} />}<span>{v.name}</span>
              {!quick && <button className="view-remove" aria-label={`Удалить поиск ${v.name}`} title="Удалить" onClick={(e) => { e.stopPropagation(); void storeViews(views.filter((x) => x.id !== v.id), `Поиск «${v.name}» удалён`) }}><X size={13} /></button>}
            </div>
          )
        })}
        <div className="unified-separator" />
        {folders.length === 0 && <SkeletonRows variant="folder" count={7} label="Загрузка папок…" />}
        {visibleFolders.map((f) => {
          const Icon = f.role === 'other' && !f.selectable ? FolderOpen : ROLE_ICONS[f.role]
          const isOpen = expanded.has(f.id)
          const indent = { paddingLeft: 4 + f.depth * 14 }
          const toggle = f.hasChildren ? (
            <span
              role="button"
              aria-label={isOpen ? 'Свернуть' : 'Развернуть'}
              className="folder-toggle"
              onClick={(e) => { e.stopPropagation(); toggleExpanded(f.id) }}
            >
              {isOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
            </span>
          ) : <span className="folder-toggle" />
          return f.selectable ? (
            <button
              key={f.id}
              className={`folder-item ${!unified && folderId === f.id ? 'selected' : ''} ${f.total === 0 ? 'folder-empty' : ''} ${dragInfo?.over === f.id && f.id !== folderId ? 'drop-target' : ''}`}
              data-folder-id={f.id}
              style={indent}
              title={`${f.path} — ${f.total} ${pluralLetters(f.total)}${f.unseen ? `, непрочитанных ${f.unseen}` : ''}`}
              onClick={() => { clearSearch(); setUnified(null); setFolderId(f.id) }}
            >
              {toggle}
              <Icon size={17} />
              <span>{folderLabel(f)}</span>
              {f.unseen > 0 && <b className="folder-count">{f.unseen}</b>}
            </button>
          ) : (
            <button key={f.id} className="folder-item folder-node" style={indent} title={f.path} onClick={() => f.hasChildren && toggleExpanded(f.id)}>
              {toggle}
              <Icon size={17} /><span>{folderLabel(f)}</span>
              {!isOpen && (nodeUnseen.get(f.id) ?? 0) > 0 && <b className="folder-count muted" title="Непрочитанных в свёрнутых папках">{nodeUnseen.get(f.id)}</b>}
            </button>
          )
        })}
      </div>
      {scheduled.length > 0 && <button className="scheduled-link" onClick={() => { loadScheduled(); setShowScheduled(true) }}>
        <CalendarClock size={16} /> Запланировано: {scheduled.length}
      </button>}
      <div className="sidebar-footer">
        {offline
          ? <button className="secure-label offline text-button" onClick={() => setShowQueue(true)}><CloudOff size={15} /> Без сети{conn.status && conn.status.items.length > 0 ? ` · в очереди ${conn.status.items.length}` : ''}</button>
          : <div className="secure-label"><ShieldCheck size={15} /> Подключено по TLS</div>}
        <span>{account.host}</span>
      </div>
    </aside>
  )
}
