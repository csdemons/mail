import { useEffect, useState } from 'react'
import { Mail } from 'lucide-react'

import { api, errorText } from './api'
import App from './App'
import Login from './Login'
import UpdateNotice from './UpdateNotice'
import type { Account, Settings } from './types'

type State =
  | { kind: 'starting' }
  | { kind: 'login'; settings: Settings | null; error: string }
  | { kind: 'ready'; account: Account; warning: string }

export default function Root() {
  const [state, setState] = useState<State>({ kind: 'starting' })

  useEffect(() => {
    let cancelled = false
    void (async () => {
      const settings = await api.getSettings().catch(() => null)
      try {
        const account = await api.autoLogin()
        if (cancelled) return
        setState(account
          ? { kind: 'ready', account, warning: '' }
          : { kind: 'login', settings, error: '' })
      } catch (error) {
        if (!cancelled) setState({ kind: 'login', settings, error: errorText(error) })
      }
    })()
    return () => { cancelled = true }
  }, [])

  if (state.kind === 'starting') {
    return (
      <div className="splash">
        <div className="brand-mark"><Mail size={19} fill="currentColor" /></div>
        <span>Подключение к почте…</span>
      </div>
    )
  }

  if (state.kind === 'login') {
    return (
      <>
      <UpdateNotice />
      <Login
        initial={state.settings}
        initialError={state.error}
        onLoggedIn={(result) => setState({ kind: 'ready', account: result.account, warning: result.warning ?? '' })}
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
      onLogout={async () => {
        const settings = await api.getSettings().catch(() => null)
        await api.logout().catch(() => undefined)
        setState({ kind: 'login', settings, error: '' })
      }}
    />
    </>
  )
}
