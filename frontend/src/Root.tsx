import { useEffect, useState } from 'react'

import { api, errorText, resetSession } from './api'
import App from './App'
import MessageWindow from './MessageWindow'
import Login from './Login'
import LockScreen from './LockScreen'
import UpdateNotice from './UpdateNotice'
import kurchatovLogo from './assets/kurchatov-logo.png'
import type { Account, Settings } from './types'

type State =
  | { kind: 'starting' }
  | { kind: 'login'; settings: Settings | null; error: string }
  | { kind: 'locked'; account: Account }
  | { kind: 'ready'; account: Account; warning: string }

export default function Root() {
  // Окно отдельного письма: вход уже выполнен в главном окне (общее состояние бэкенда).
  const msg = window.__RRCKI_MESSAGE
  if (msg) return <MessageWindow folder={msg.folder} uid={msg.uid} />
  return <MainRoot />
}

function MainRoot() {
  const [state, setState] = useState<State>({ kind: 'starting' })

  useEffect(() => {
    let cancelled = false
    void (async () => {
      const settings = await api.getSettings().catch(() => null)
      try {
        const account = await api.autoLogin()
        if (cancelled) return
        setState(!account
          ? { kind: 'login', settings, error: '' }
          : account.locked ? { kind: 'locked', account } : { kind: 'ready', account, warning: '' })
      } catch (error) {
        if (!cancelled) setState({ kind: 'login', settings, error: errorText(error) })
      }
    })()
    return () => { cancelled = true }
  }, [])

  if (state.kind === 'starting') {
    return (
      <div className="splash">
        <img className="brand-logo" src={kurchatovLogo} alt="" />
        <span>Подключение к почте…</span>
      </div>
    )
  }

  async function logout(resetTotp = false) {
    const settings = await api.getSettings().catch(() => null)
    await api.logout(resetTotp).catch(() => undefined)
    resetSession()
    setState({ kind: 'login', settings, error: '' })
  }

  if (state.kind === 'locked') {
    return (
      <LockScreen
        account={state.account}
        onUnlocked={(result) => { resetSession(); setState({ kind: 'ready', account: result.account, warning: result.warning ?? '' }) }}
        onLogout={(reset) => void logout(reset)}
      />
    )
  }

  if (state.kind === 'login') {
    return (
      <>
      <UpdateNotice />
      <Login
        initial={state.settings}
        initialError={state.error}
        onLoggedIn={(result) => {
          resetSession()
          setState(result.account.locked
            ? { kind: 'locked', account: result.account }
            : { kind: 'ready', account: result.account, warning: result.warning ?? '' })
        }}
      />
      </>
    )
  }

  return (
    <>
    <UpdateNotice />
    <App
      account={state.account}
      initialNotice={state.warning}
      onLogout={() => void logout()}
      onLock={() => {
        const account = state.account
        void api.lockApp().then(() => { resetSession(); setState({ kind: 'locked', account }) }).catch(() => undefined)
      }}
    />
    </>
  )
}
