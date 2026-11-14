import { PanelLeftClose, Search, X, RefreshCw, ListFilter, MessagesSquare, Bookmark, BookmarkPlus, Archive, FolderInput, Trash2, Paperclip, CircleAlert, Flag, Reply, ChevronRight, AlarmClock, Inbox, Circle } from 'lucide-react'
import { type MailFilter, type MessageSummary, type Category, type AccountEntry, type SavedView, type ConnectionStatus, type Folder, type UnifiedItem } from './types'
import { userCategories, dueOf, categoriesOf, dueLabel } from './mailExtras'
import { SEARCH_HELP, pluralLetters, recipientsLabel, senderLabel, initials } from './mailLabels'
import { OfflineBanner } from './OfflineQueue'
import { SkeletonRows } from './Skeleton'
import { messageKey, formatListDate } from './api'
import { Fragment } from 'react'
import { snoozedUntil } from './mailTools'

export interface MessageListPaneProps {
  setFoldersOpen: React.Dispatch<React.SetStateAction<boolean>>
  foldersOpen: boolean
  searchRef: React.RefObject<HTMLInputElement>
  query: string
  setQuery: React.Dispatch<React.SetStateAction<string>>
  clearSearch: () => void
  loading: boolean
  refresh: () => Promise<void>
  setFilterSeed: React.Dispatch<React.SetStateAction<MessageSummary | null>>
  setShowFilters: React.Dispatch<React.SetStateAction<boolean>>
  filter: MailFilter
  setFilter: React.Dispatch<React.SetStateAction<MailFilter>>
  categories: Category[]
  catFilter: string
  setCatFilter: React.Dispatch<React.SetStateAction<string>>
  threadsOn: boolean
  setThreadsOn: React.Dispatch<React.SetStateAction<boolean>>
  searching: boolean
  searchResults: MessageSummary[] | null
  othersBusy: boolean
  serverSearched: boolean
  searchBusy: boolean
  searchOnServer: () => Promise<void>
  acc: { accounts: AccountEntry[]; unread: Record<string, number | null>; multi: boolean; reload: () => Promise<void>; }
  searchScope: "current" | "all"
  setSearchScope: React.Dispatch<React.SetStateAction<"current" | "all">>
  activeView: SavedView | undefined
  saveViewName: string | null
  setSaveViewName: React.Dispatch<React.SetStateAction<string | null>>
  saveCurrentSearch: () => void
  otherErrors: string[]
  listTitle: string
  total: number
  conn: { status: ConnectionStatus | null; setStatus: (s: ConnectionStatus) => void; refresh: () => Promise<void>; }
  setShowQueue: React.Dispatch<React.SetStateAction<boolean>>
  checkConnection: () => Promise<void>
  checkingConn: boolean
  checked: Set<string>
  archive: (items: MessageSummary[]) => Promise<void>
  actionTargets: () => MessageSummary[]
  setMovePicker: React.Dispatch<React.SetStateAction<MessageSummary[] | null>>
  requestDelete: (items: MessageSummary[], force?: boolean) => void
  setChecked: React.Dispatch<React.SetStateAction<Set<string>>>
  listRef: React.RefObject<HTMLDivElement>
  scrollTop: React.MutableRefObject<number>
  messages: MessageSummary[]
  listError: string
  loadFirstPage: (folder: string, keepSelection: boolean) => Promise<number | undefined>
  folderId: string
  listRows: { m: MessageSummary; count: number; child: boolean; thread: string; }[]
  dateGroup: (value: string | null) => string
  folders: Folder[]
  openThreads: Set<string>
  selectedKey: string
  rowMenu: { x: number; y: number; items: MessageSummary[]; } | null
  onRowClick: (e: React.MouseEvent<Element, MouseEvent>, m: MessageSummary) => void
  otherOf: (m: MessageSummary) => UnifiedItem | null
  openInWindow: (m: MessageSummary) => void
  openRowMenu: (e: React.MouseEvent<Element, MouseEvent>, m: MessageSummary) => void
  startMessageDrag: (e: React.PointerEvent<Element>, m: MessageSummary) => void
  setOpenThreads: React.Dispatch<React.SetStateAction<Set<string>>>
  folderName: (id: string) => string
  visibleMessages: MessageSummary[]
  currentFolder: Folder | undefined
  setShowSettings: React.Dispatch<React.SetStateAction<boolean>>
  runDiagnostics: () => Promise<void>
  hasMore: boolean
  loadingMore: boolean
  loadMore: () => Promise<void>
  offline: boolean
}

export default function MessageListPane({ setFoldersOpen, foldersOpen, searchRef, query, setQuery, clearSearch, loading, refresh, setFilterSeed, setShowFilters, filter, setFilter, categories, catFilter, setCatFilter, threadsOn, setThreadsOn, searching, searchResults, othersBusy, serverSearched, searchBusy, searchOnServer, acc, searchScope, setSearchScope, activeView, saveViewName, setSaveViewName, saveCurrentSearch, otherErrors, listTitle, total, conn, setShowQueue, checkConnection, checkingConn, checked, archive, actionTargets, setMovePicker, requestDelete, setChecked, listRef, scrollTop, messages, listError, loadFirstPage, folderId, listRows, dateGroup, folders, openThreads, selectedKey, rowMenu, onRowClick, otherOf, openInWindow, openRowMenu, startMessageDrag, setOpenThreads, folderName, visibleMessages, currentFolder, setShowSettings, runDiagnostics, hasMore, loadingMore, loadMore, offline }: MessageListPaneProps) {
  return (
    <main className="message-column">
      <div className="list-toolbar">
        <button className="icon-button soft" aria-label="Показать папки" onClick={() => setFoldersOpen(!foldersOpen)}><PanelLeftClose size={18} /></button>
        <label className="search-box">
          <Search size={17} />
          <input
            ref={searchRef}
            value={query}
            aria-label="Поиск писем"
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Escape' && query) { e.preventDefault(); e.stopPropagation(); clearSearch() } }}
            placeholder="Поиск по почте"
          />
          {query
            ? <button type="button" className="search-clear" aria-label="Очистить поиск" title="Очистить (Esc)" onClick={() => { clearSearch(); searchRef.current?.focus() }}><X size={15} /></button>
            : <kbd>Ctrl K</kbd>}
        </label>
        <button
          className="icon-button soft refresh-button"
          aria-label="Обновить"
          title={loading ? 'Загрузка писем…' : 'Обновить'}
          disabled={loading}
          onClick={() => void refresh()}
        >
          <RefreshCw size={17} className={loading ? 'spin' : ''} />
        </button>
        <button className="icon-button soft" aria-label="Правила обработки почты" title="Правила обработки почты: сортировка, пометки, уведомления, задачи" onClick={() => { setFilterSeed(null); setShowFilters(true) }}>
          <ListFilter size={17} />
        </button>
      </div>
      <div className="filter-row">
        {(['Все', 'Непрочитанные', 'Важные', 'С флажком', 'С вложениями'] as MailFilter[]).map((item) => (
          <button key={item} className={`filter-chip ${filter === item ? 'active' : ''}`} onClick={() => setFilter(item)}>{item}</button>
        ))}
        {userCategories(categories).length > 0 && <select className={`filter-chip filter-select ${catFilter ? 'active' : ''}`} aria-label="Категория" value={catFilter} onChange={(e) => setCatFilter(e.target.value)}>
          <option value="">Все категории</option>
          {userCategories(categories).map((c) => <option key={c.keyword} value={c.keyword}>{c.name}</option>)}
        </select>}
        <button className={`filter-chip ${threadsOn ? 'active' : ''}`} aria-pressed={threadsOn} title="Группировать письма одной переписки" onClick={() => setThreadsOn(!threadsOn)}><MessagesSquare size={13} /> Цепочки</button>
      </div>
      {searching
        ? <><div className="message-list-header search-header">
          <span>Найдено: {searchResults?.length ?? 0}{(searchResults?.length ?? 0) >= 300 ? '+' : ''}{othersBusy ? ' · ищу в других ящиках…' : ''}</span>
          {serverSearched
            ? <span>с учётом текста писем на сервере</span>
            : <button className="text-button" disabled={searchBusy} onClick={() => void searchOnServer()}>{searchBusy ? 'Поиск на сервере…' : 'Искать и в тексте старых писем'}</button>}
        </div>
        <div className="search-tools">
          {acc.multi && <div className="search-scope" role="group" aria-label="Где искать">
            <button className={`filter-chip ${searchScope === 'current' ? 'active' : ''}`} aria-pressed={searchScope === 'current'} onClick={() => setSearchScope('current')}>Этот ящик</button>
            <button className={`filter-chip ${searchScope === 'all' ? 'active' : ''}`} aria-pressed={searchScope === 'all'} onClick={() => setSearchScope('all')}>Все ящики</button>
          </div>}
          {activeView
            ? <span className="search-saved"><Bookmark size={13} /> {activeView.name}</span>
            : saveViewName === null
              ? <button className="text-button" onClick={() => setSaveViewName('')}><BookmarkPlus size={14} /> Сохранить поиск</button>
              : <form className="save-view" onSubmit={(e) => { e.preventDefault(); saveCurrentSearch() }}>
                <input autoFocus value={saveViewName} maxLength={60} aria-label="Название поиска" placeholder="Название" onChange={(e) => setSaveViewName(e.target.value)} onKeyDown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); setSaveViewName(null) } }} />
                <button className="text-button" type="submit" disabled={!saveViewName.trim()}>Сохранить</button>
              </form>}
          <span className="search-help" title={SEARCH_HELP}>?</span>
        </div>
        {otherErrors.length > 0 && <div className="unified-errors" role="alert">Не ответили: {otherErrors.join('; ')}</div>}
        </>
        : <div className="message-list-header"><span>{listTitle}</span><span>{total} {pluralLetters(total)}</span></div>}
      <OfflineBanner status={conn.status} onOpen={() => setShowQueue(true)} onCheck={() => void checkConnection()} checking={checkingConn} />
      {checked.size > 0 && <div className="bulk-bar">
        <span>Отмечено: {checked.size}</span>
        <button className="text-button" onClick={() => void archive(actionTargets())}><Archive size={15} /> В архив</button>
        <button className="text-button" onClick={() => setMovePicker(actionTargets())}><FolderInput size={15} /> Переместить</button>
        <button className="text-button danger-text" onClick={() => requestDelete(actionTargets())}><Trash2 size={15} /> Удалить</button>
        <button className="icon-button soft" aria-label="Снять отметку" onClick={() => setChecked(new Set())}><X size={15} /></button>
      </div>}
      <div className="message-list" ref={listRef} onScroll={(e) => { scrollTop.current = e.currentTarget.scrollTop }}>
        {loading && messages.length === 0 && <SkeletonRows variant="message" count={9} label="Загрузка писем с сервера…" />}
        {listError && (
          <div className="empty-state" role="alert">
            <strong>Не удалось загрузить письма</strong>
            <span>{listError}</span>
            <button className="text-button" onClick={() => void loadFirstPage(folderId, true)}>Повторить</button>
          </div>
        )}
        {listRows.map(({ m, count, child, thread }, rowIndex) => {
          const key = messageKey(m)
          // Заголовок группы по дате: «Сегодня», «Вчера», «На этой неделе»… (у вложенных писем цепочки — нет).
          const group = child ? null : dateGroup(m.date)
          const prevTop = listRows.slice(0, rowIndex).reverse().find((r) => !r.child)
          const showGroup = group !== null && (!prevTop || dateGroup(prevTop.m.date) !== group)
          const outgoing = ['sent', 'drafts'].includes(folders.find((f) => f.id === m.folder)?.role ?? '')
          const sender = outgoing ? recipientsLabel(m) : senderLabel(m)
          const due = dueOf(m)
          const cats = categoriesOf(m, categories)
          const open = openThreads.has(thread)
          return (
            <Fragment key={key}>
            {showGroup && <div className="message-group" role="presentation">{group}</div>}
            <button
              className={`message-row ${child ? 'thread-child' : ''} ${selectedKey === key ? 'selected' : ''} ${checked.has(key) ? 'checked' : ''} ${m.unread ? 'unread' : ''} ${rowMenu?.items.some((x) => messageKey(x) === key) ? 'menu-target' : ''}`}
              onClick={(e) => onRowClick(e, m)}
              onDoubleClick={() => { if (!otherOf(m)) openInWindow(m) }}
              onContextMenu={(e) => { if (otherOf(m)) e.preventDefault(); else openRowMenu(e, m) }}
              title="Двойной щелчок — открыть в отдельном окне"
              onPointerDown={(e) => startMessageDrag(e, m)}
              onDragStart={(e) => e.preventDefault()}
            >
              <div className={`sender-avatar avatar-${m.uid % 5}`}>{initials(outgoing ? sender.slice(6) : sender)}</div>
              <div className="message-summary">
                <div className="message-topline">
                  {m.unread && <span className="unread-dot" aria-label="Не прочитано" />}
                  <span className="sender-name">{sender}</span>
                  <span className="message-icons">
                    {m.hasAttachments && <Paperclip size={13} aria-label="Есть вложения" />}
                    {m.important && <CircleAlert size={13} className="important-mark" aria-label="Важное" />}
                    {m.flagged && <Flag size={13} className="flag-mark" fill="currentColor" aria-label="С флажком" />}
                    {m.answered && <Reply size={13} aria-label="Есть ответ" />}
                  </span>
                  <time>{formatListDate(m.date)}</time>
                </div>
                <div className="message-subject">
                  {count > 1 && <span
                    className={`thread-count ${open ? 'open' : ''}`}
                    role="button"
                    aria-label={open ? 'Свернуть цепочку' : 'Развернуть цепочку'}
                    title={`Писем в цепочке: ${count}`}
                    onClick={(e) => { e.stopPropagation(); setOpenThreads((s) => { const n = new Set(s); if (n.has(thread)) n.delete(thread); else n.add(thread); return n }) }}
                    onPointerDown={(e) => e.stopPropagation()}
                  ><ChevronRight size={12} />{count}</span>}
                  {m.subject || '(без темы)'}
                </div>
                {m.snippet && <div className="message-snippet">{m.snippet}</div>}
                {searching && <div className="message-folder">{otherOf(m) ? <><span className="search-account">{otherOf(m)!.label}</span> · {m.folder}</> : folderName(m.folder)}</div>}
                {(cats.length > 0 || due || snoozedUntil(m.keywords)) && <div className="message-tags">
                  {snoozedUntil(m.keywords) && <span className="snooze-tag" title="Отложено"><AlarmClock size={11} /> до {dueLabel(snoozedUntil(m.keywords)!)}</span>}
                  {due && <span className={`due-tag ${due.getTime() < Date.now() ? 'overdue' : ''}`} title="Срок флажка"><Flag size={11} /> {dueLabel(due)}</span>}
                  {cats.map((c) => <span key={c.keyword} className="cat-tag" style={{ borderColor: c.color, color: c.color }}><span className="cat-dot" style={{ background: c.color }} />{c.name}</span>)}
                </div>}
              </div>
            </button>
            </Fragment>
          )
        })}
        {!loading && !listError && visibleMessages.length === 0 && (
          <div className="empty-state">
            <Inbox size={30} />
            <strong>{searching || messages.length > 0 ? 'Ничего не найдено' : 'Папка пуста'}</strong>
            {searching && <button className="text-button" onClick={clearSearch}>Очистить поиск</button>}
            {!searching && messages.length === 0 && (currentFolder?.total ?? 0) > 0 && <>
              <span>Сервер сообщает о {currentFolder?.total} письмах в этой папке, но список не получен.</span>
              <button className="text-button" onClick={() => { setShowSettings(true); void runDiagnostics() }}>Открыть диагностику</button>
            </>}
            {!searching && messages.length > 0 && <>
              <button className="text-button" onClick={() => { clearSearch(); setFilter('Все') }}>Сбросить фильтры</button>
            </>}
          </div>
        )}
        {hasMore && !listError && !searching && (
          <button className="text-button load-more" disabled={loadingMore} onClick={() => void loadMore()}>
            {loadingMore ? 'Загрузка…' : `Показать ещё (загружено ${messages.length} из ${total})`}
          </button>
        )}
      </div>
      <div className="list-footer">
        <span>{searching ? `${visibleMessages.length} найдено` : `${visibleMessages.length} из ${messages.length} загруженных`}</span>
        <span>
          <Circle size={7} fill={listError ? '#ef4444' : offline ? '#f59e0b' : '#22c55e'} />
          {listError ? 'Ошибка' : offline ? 'Без сети' : loading ? 'Синхронизация' : 'Онлайн'}
        </span>
      </div>
    </main>
  )
}
