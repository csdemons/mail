import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Inbox, Lock, LogOut, Paperclip, Plus, RefreshCw, X } from 'lucide-react'

import { api, errorText } from './api'
import Login from './Login'
import { appInFront, notify } from './notifications'
import type { AccountEntry, MessageSummary, UnifiedFilters, UnifiedItem, UnifiedMark, UnifiedRole } from './types'

export function accountTitle(a: { displayName: string; username: string }) {
  return a.displayName.trim() || a.username
}

/** Подключённые учётные записи и непрочитанные в остальных; уведомления о новых письмах в них. */
export function useAccounts(pollSecs: () => number, mailNotify: () => { mail: boolean; preview: boolean }, onNotice: (t: string) => void) {
  const [accounts, setAccounts] = useState<AccountEntry[]>([])
  const [unread, setUnread] = useState<Record<string, number | null>>({})
  const multi = accounts.length > 1
  const cb = useRef({ pollSecs, mailNotify, onNotice })
  cb.current = { pollSecs, mailNotify, onNotice }

  const reload = useCallback(async () => {
    try { setAccounts(await api.accountsList()) } catch { /* до входа */ }
  }, [])

  useEffect(() => { void reload() }, [reload])

  useEffect(() => {
    if (!multi) return
    let stopped = false
    let timer: number | undefined
    async function tick() {
      try {
        const list = await api.accountsUnread()
        if (stopped) return
        setUnread(Object.fromEntries(list.map((u) => [u.key, u.unread])))
        const fresh = await api.accountsPoll()
        const p = cb.current.mailNotify()
        for (const n of fresh) {
          if (!p.mail) continue
          const m = n.items[0]
          const who = m.fromName || m.fromAddress
          if (appInFront()) {
            cb.current.onNotice(n.items.length > 1 ? `Новых писем в ${n.label}: ${n.items.length}` : `Новое письмо в ${n.label} от ${who}`)
          } else {
            notify(n.items.length > 1 ? `Новых писем: ${n.items.length} — ${n.label}` : `Новое письмо — ${who} (${n.label})`, p.preview ? (m.subject || '(без темы)') : 'Откройте Atom Mail, чтобы прочитать')
          }
        }
      } catch { /* сеть — в следующий раз */ }
      finally {
        if (!stopped) timer = window.setTimeout(tick, Math.max(cb.current.pollSecs(), 10) * 1000)
      }
    }
    timer = window.setTimeout(tick, 2000)
    return () => { stopped = true; if (timer) window.clearTimeout(timer) }
  }, [multi])

  return { accounts, unread, multi, reload }
}

/** Меню учётной записи: переключение, добавление, отключение, выход. */
export function AccountMenu({ accounts, unread, onSwitch, onReload, onLock, onLogout, onClose, onNotice }: {
  accounts: AccountEntry[]
  unread: Record<string, number | null>
  onSwitch: (key: string) => void
  onReload: () => void
  onLock: (() => void) | null
  onLogout: () => void
  onClose: () => void
  onNotice: (t: string) => void
}) {
  const [adding, setAdding] = useState(false)
  const [removing, setRemoving] = useState<AccountEntry | null>(null)

  async function remove(a: AccountEntry) {
    try {
      await api.accountRemove(a.key)
      onNotice(`Учётная запись ${a.username} отключена: пароль и письма на компьютере удалены.`)
      onReload()
    } catch (e) { onNotice(errorText(e)) } finally { setRemoving(null) }
  }

  return <>
    <div className="account-menu popover" role="menu" aria-label="Учётные записи">
      <strong>Учётные записи</strong>
      {accounts.map((a) => (
        <div key={a.key} className={`account-menu-item ${a.active ? 'active' : ''}`} data-account={a.key}>
          <button className="account-pick" disabled={a.active || !a.canSwitch} title={a.canSwitch ? a.username : 'Пароль не сохранён'} onClick={() => { onClose(); onSwitch(a.key) }}>
            <span className={`status-dot ${a.active ? '' : 'idle'}`} />
            <span className="account-pick-text"><b>{accountTitle(a)}</b><small>{a.username === accountTitle(a) ? a.host : a.username}</small></span>
            {!a.active && (unread[a.key] ?? 0) > 0 && <b className="folder-count" aria-label="Непрочитанные">{unread[a.key]}</b>}
            {a.active && <small className="account-current">текущая</small>}
          </button>
          {!a.active && <button className="icon-button soft" aria-label={`Отключить ${a.username}`} title="Отключить учётную запись" onClick={() => setRemoving(a)}><X size={14} /></button>}
        </div>
      ))}
      <button className="text-button" onClick={() => setAdding(true)}><Plus size={15} /> Добавить учётную запись…</button>
      {onLock && <button className="text-button" onClick={() => { onClose(); onLock() }}><Lock size={15} /> Заблокировать <kbd>Ctrl+L</kbd></button>}
      <button className="text-button" onClick={() => { onClose(); onLogout() }}><LogOut size={15} /> Выйти из текущей</button>
    </div>
    {removing && <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) setRemoving(null) }}>
      <div className="settings-modal confirm-modal" role="dialog" aria-label="Отключить учётную запись">
        <strong>Отключить {removing.username}?</strong>
        <span className="settings-hint">Пароль и письма этой учётной записи будут удалены с компьютера. Письма на сервере не затрагиваются.</span>
        <div className="settings-actions">
          <button className="secondary-button" onClick={() => setRemoving(null)}>Отмена</button>
          <button className="primary-button danger" onClick={() => void remove(removing)}>Отключить</button>
        </div>
      </div>
    </div>}
    {adding && <Login
      initial={null}
      initialError=""
      onLoggedIn={() => undefined}
      add={{
        onCancel: () => setAdding(false),
        onAdded: () => { setAdding(false); onClose(); onReload(); onNotice('Учётная запись добавлена. Письма всех ящиков — в «Все входящие».') },
      }}
    />}
  </>
}

function when(iso: string | null) {
  if (!iso) return ''
  const d = new Date(iso)
  const today = new Date()
  return d.toDateString() === today.toDateString()
    ? d.toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' })
    : d.toLocaleDateString('ru-RU', { day: '2-digit', month: '2-digit' })
}

/** «Все входящие»: последние письма всех учётных записей. */
export const UNIFIED_ROLES: { role: UnifiedRole; title: string }[] = [
  { role: 'inbox', title: 'Входящие' },
  { role: 'sent', title: 'Отправленные' },
  { role: 'drafts', title: 'Черновики' },
  { role: 'archive', title: 'Архив' },
  { role: 'junk', title: 'Спам' },
  { role: 'trash', title: 'Корзина' },
]

export const NO_FILTERS: UnifiedFilters = { account: '', domain: '', person: '', mark: 'all' }

const MARKS: { mark: UnifiedMark; title: string }[] = [
  { mark: 'all', title: 'Все' },
  { mark: 'unread', title: 'Непрочитанные' },
  { mark: 'important', title: 'Важные' },
  { mark: 'flagged', title: 'С флажком' },
  { mark: 'attachments', title: 'С вложениями' },
]

/** Адрес из «Имя <a@b.ru>» или «a@b.ru». */
function addressOf(s: string): string {
  const m = /<([^>]+)>/.exec(s)
  return (m ? m[1] : s).trim().toLowerCase()
}

/** Собеседник письма: в «Отправленных» и «Черновиках» — первый получатель, иначе отправитель. */
export function peerOf(m: MessageSummary, role: UnifiedRole): { name: string; address: string } {
  if (role === 'sent' || role === 'drafts') {
    const first = m.to[0] ?? ''
    const name = first.includes('<') ? first.slice(0, first.indexOf('<')).trim().replace(/^"|"$/g, '') : ''
    return { name, address: addressOf(first) }
  }
  return { name: m.fromName, address: m.fromAddress.toLowerCase() }
}

/** Подразделение — домен адреса собеседника. */
export function domainOf(address: string): string {
  const at = address.lastIndexOf('@')
  return at >= 0 ? address.slice(at + 1).toLowerCase() : ''
}

export function matchesFilters(m: UnifiedItem, role: UnifiedRole, f: UnifiedFilters): boolean {
  if (f.account && m.account !== f.account) return false
  const peer = peerOf(m, role)
  if (f.domain && domainOf(peer.address) !== f.domain) return false
  const q = f.person.trim().toLowerCase()
  if (q && !peer.address.includes(q) && !peer.name.toLowerCase().includes(q)) return false
  switch (f.mark) {
    case 'unread': return m.unread
    case 'important': return m.important
    case 'flagged': return m.flagged
    case 'attachments': return m.hasAttachments
    default: return true
  }
}

export function UnifiedColumn({ role, accounts, filters, onFilters, onOpen, onNotice }: {
  role: UnifiedRole
  accounts: AccountEntry[]
  filters: UnifiedFilters
  onFilters: (f: UnifiedFilters) => void
  onOpen: (item: UnifiedItem) => void
  onNotice: (t: string) => void
}) {
  const [items, setItems] = useState<UnifiedItem[] | null>(null)
  const [errors, setErrors] = useState<string[]>([])
  const [busy, setBusy] = useState(false)
  const title = UNIFIED_ROLES.find((r) => r.role === role)?.title ?? 'Входящие'

  const load = useCallback(async () => {
    setBusy(true)
    try {
      const r = role === 'inbox' ? await api.unifiedInbox(50) : await api.unifiedFolder(role, 50)
      setItems(r.items)
      setErrors(r.errors)
    } catch (e) { onNotice(errorText(e)) } finally { setBusy(false) }
  }, [onNotice, role])

  useEffect(() => { setItems(null); void load() }, [load])

  const domains = useMemo(() => {
    const counts = new Map<string, number>()
    for (const m of items ?? []) {
      const d = domainOf(peerOf(m, role).address)
      if (d) counts.set(d, (counts.get(d) ?? 0) + 1)
    }
    if (filters.domain && !counts.has(filters.domain)) counts.set(filters.domain, 0)
    return [...counts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
  }, [items, role, filters.domain])
  const shown = (items ?? []).filter((m) => matchesFilters(m, role, filters))
  const filtered = filters.account || filters.domain || filters.person.trim() || filters.mark !== 'all'
  const colors = ['#2474ed', '#1f9d74', '#d9822b', '#9b51e0', '#e5484d', '#0ea5a4']
  const colorOf = (key: string) => colors[Math.max(0, accounts.findIndex((a) => a.key === key)) % colors.length]
  const set = (patch: Partial<UnifiedFilters>) => onFilters({ ...filters, ...patch })
  const personLabel = role === 'sent' || role === 'drafts' ? 'Получатель' : 'Отправитель'

  return (
    <main className="message-column unified-column" aria-label={`Все ящики — ${title}`} data-role={role}>
      <div className="list-toolbar">
        <strong className="unified-title"><Inbox size={17} /> {role === 'inbox' ? 'Все входящие' : `Все ящики — ${title}`}</strong>
        <button className="icon-button soft refresh-button" aria-label="Обновить" disabled={busy} onClick={() => void load()}><RefreshCw size={17} className={busy ? 'spin' : ''} /></button>
      </div>
      <div className="unified-filters">
        <select className="filter-chip filter-select" aria-label="Учётная запись" value={filters.account} onChange={(e) => set({ account: e.target.value })}>
          <option value="">Все ящики</option>
          {accounts.map((a) => <option key={a.key} value={a.key}>{accountTitle(a)}</option>)}
        </select>
        <select className="filter-chip filter-select" aria-label="Подразделение" value={filters.domain} onChange={(e) => set({ domain: e.target.value })}>
          <option value="">Все подразделения</option>
          {domains.map(([d, n]) => <option key={d} value={d}>{d}{n ? ` (${n})` : ''}</option>)}
        </select>
        <label className="unified-person">
          <input value={filters.person} aria-label={personLabel} placeholder={personLabel} onChange={(e) => set({ person: e.target.value })} />
          {filters.person && <button type="button" aria-label={`Очистить: ${personLabel}`} onClick={() => set({ person: '' })}><X size={13} /></button>}
        </label>
        <div className="unified-marks" role="group" aria-label="Важность">
          {MARKS.map((x) => (
            <button key={x.mark} className={`filter-chip ${filters.mark === x.mark ? 'active' : ''}`} aria-pressed={filters.mark === x.mark} onClick={() => set({ mark: x.mark })}>{x.title}</button>
          ))}
        </div>
        {filtered && <button className="text-button unified-reset" onClick={() => onFilters(NO_FILTERS)}>Сбросить</button>}
      </div>
      {errors.length > 0 && <div className="unified-errors" role="alert">Не ответили: {errors.join('; ')}</div>}
      <div className="message-list">
        {items === null && <div className="empty-state"><span>Загрузка писем всех учётных записей…</span></div>}
        {items !== null && shown.length === 0 && <div className="empty-state"><span>{filtered ? 'Ничего не найдено' : 'Писем нет'}</span>{filtered && <button className="text-button" onClick={() => onFilters(NO_FILTERS)}>Сбросить фильтры</button>}</div>}
        {shown.map((m: UnifiedItem & MessageSummary) => {
          const peer = peerOf(m, role)
          return (
            <button key={`${m.account}|${m.folder}|${m.uid}`} className={`message-row unified-row ${m.unread ? 'unread' : ''}`} data-account={m.account} onClick={() => onOpen(m)}>
              <span className="unified-account" style={{ background: colorOf(m.account) }} title={m.label} />
              <div className="message-summary">
                <div className="message-topline"><span className="sender-name">{(role === 'sent' || role === 'drafts' ? 'Кому: ' : '') + (peer.name || peer.address || '—')}</span><time>{when(m.date)}</time></div>
                <div className="message-subject">{m.important && <b className="unified-important" title="Важное">!</b>}{m.subject || '(без темы)'}</div>
                <div className="message-folder unified-label">{m.label}{domainOf(peer.address) && <span className="unified-domain">· {domainOf(peer.address)}</span>}{m.hasAttachments && <Paperclip size={12} />}</div>
              </div>
            </button>
          )
        })}
      </div>
    </main>
  )
}
