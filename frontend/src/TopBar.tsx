import kurchatovLogo from './assets/kurchatov-logo.png'
import { Users, ChevronDown, SquarePen, Search, Sun, Moon, Settings } from 'lucide-react'
import { AccountMenu } from './Accounts'
import type { AccountEntry, Account, MessageSummary } from './types'

/** Шапка окна: логотип, учётные записи, «Новое сообщение», общий поиск, тема и настройки. */
export interface TopBarProps {
  showAccountMenu: boolean
  acc: { accounts: AccountEntry[]; unread: Record<string, number | null>; multi: boolean; reload: () => Promise<void>; }
  setShowAccountMenu: React.Dispatch<React.SetStateAction<boolean>>
  myAddress: string
  account: Account
  openComposer: (mode: "new" | "reply" | "replyAll" | "forward") => void
  setGlobalOpen: React.Dispatch<React.SetStateAction<boolean>>
  setDark: (v: boolean) => void
  dark: boolean
  setShowSettings: React.Dispatch<React.SetStateAction<boolean>>
  switchAccount: (key: string, open?: MessageSummary | undefined) => void
  totpOn: boolean
  onLock: () => void
  onLogout: () => void
  setNotice: React.Dispatch<React.SetStateAction<string>>
}

export default function TopBar({ showAccountMenu, acc, setShowAccountMenu, myAddress, account, openComposer, setGlobalOpen, setDark, dark, setShowSettings, switchAccount, totpOn, onLock, onLogout, setNotice }: TopBarProps) {
  return (
    <header className="topbar">
      <div className="brand">
        <img className="brand-logo" src={kurchatovLogo} alt="НИЦ «Курчатовский институт»" />
        <span>Atom Workspace</span>
      </div>
      <button className="account-switch" aria-label="Учётные записи" onClick={() => { if (!showAccountMenu) void acc.reload(); setShowAccountMenu(!showAccountMenu) }}>
        <div className="account-avatar"><Users size={20} /></div>
        <span><strong>{myAddress}</strong><small>{account.exchange ? 'Exchange' : 'IMAP'} · {account.host}{acc.multi ? ` · ещё ${acc.accounts.length - 1}` : ''}</small></span>
        {Object.values(acc.unread).some((n) => (n ?? 0) > 0) && <b className="rail-badge account-badge" title="Непрочитанные в других учётных записях">{Object.values(acc.unread).reduce<number>((a, n) => a + (n ?? 0), 0)}</b>}
        <ChevronDown size={17} />
      </button>
      <button className="primary-button compose-top" onClick={() => openComposer('new')}>
        <SquarePen size={17} /> Новое сообщение
      </button>
      <button className="global-search-open" onClick={() => setGlobalOpen(true)} aria-label="Общий поиск" title="Искать везде: письма, файлы, контакты, встречи, задачи, чат (Ctrl+K)">
        <Search size={16} /><span>Искать везде…</span><kbd>Ctrl+K</kbd>
      </button>
      <div className="topbar-actions">
        <span id="update-slot" className="update-slot" />
        <button className="icon-button" aria-label="Тема" onClick={() => setDark(!dark)}>{dark ? <Sun /> : <Moon />}</button>
        <button className="icon-button" aria-label="Настройки" onClick={() => setShowSettings(true)}><Settings /></button>
      </div>
      {showAccountMenu && <AccountMenu
        accounts={acc.accounts}
        unread={acc.unread}
        onSwitch={(key) => switchAccount(key)}
        onReload={() => void acc.reload()}
        onLock={totpOn ? onLock : null}
        onLogout={onLogout}
        onClose={() => setShowAccountMenu(false)}
        onNotice={setNotice}
      />}
    </header>
  )
}
