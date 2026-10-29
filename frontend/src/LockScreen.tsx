import { useState } from 'react'
import { KeyRound, LogOut, ShieldCheck } from 'lucide-react'

import { api, errorText } from './api'
import kurchatovLogo from './assets/kurchatov-logo.png'
import type { Account, LoginResult } from './types'

/** Программа заблокирована: почта не подключена, пока не введён код из приложения
 *  на телефоне (или одноразовый резервный код). */
export default function LockScreen({ account, onUnlocked, onLogout }: {
  account: Account
  onUnlocked: (result: LoginResult) => void
  onLogout: (resetTotp: boolean) => void
}) {
  const [confirmReset, setConfirmReset] = useState(false)
  const [code, setCode] = useState('')
  const [recovery, setRecovery] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  async function submit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setBusy(true)
    setError('')
    try {
      const result = await api.totpUnlock(code)
      setCode('')
      onUnlocked(result)
    } catch (err) {
      setError(errorText(err))
      setCode('')
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="login-screen">
      <form className="login-card lock-card" onSubmit={submit} aria-label="Программа заблокирована">
        <div className="login-brand">
          <img className="brand-logo" src={kurchatovLogo} alt="НИЦ «Курчатовский институт»" />
          <span>Atom Mail</span>
        </div>
        <h1>Программа заблокирована</h1>
        <p className="login-hint">
          <b>{account.username}</b><br />
          {recovery
            ? 'Введите один из резервных кодов, полученных при включении защиты. Каждый код действует один раз.'
            : 'Введите 6-значный код из приложения-аутентификатора на телефоне.'}
        </p>
        <label className="login-field">
          <span>{recovery ? 'Резервный код' : 'Код из приложения'}</span>
          <input
            key={recovery ? 'r' : 't'}
            autoFocus
            required
            className={recovery ? undefined : 'totp-input'}
            aria-label={recovery ? 'Резервный код' : 'Код из приложения'}
            autoComplete="one-time-code"
            inputMode={recovery ? 'text' : 'numeric'}
            maxLength={recovery ? 12 : 6}
            placeholder={recovery ? 'xxxx-xxxx' : '000000'}
            value={code}
            onChange={(e) => setCode(recovery ? e.target.value : e.target.value.replace(/\D/g, ''))}
          />
        </label>
        {error && <div className="login-error" role="alert">{error}</div>}
        <button className="primary-button login-submit" type="submit" disabled={busy || (!recovery && code.length !== 6)}>
          <ShieldCheck size={16} /> {busy ? 'Проверка…' : 'Разблокировать'}
        </button>
        <button type="button" className="text-button" onClick={() => { setRecovery(!recovery); setCode(''); setError('') }}>
          <KeyRound size={15} /> {recovery ? 'Ввести код из приложения' : 'Нет телефона — ввести резервный код'}
        </button>
        <button type="button" className="text-button" onClick={() => onLogout(false)}>
          <LogOut size={15} /> Выйти и забыть пароль
        </button>
        {confirmReset
          ? <div className="lock-reset">
            <span>Защита кодом будет отключена, сохранённый пароль — забыт. Войти снова можно будет только по паролю почты, затем включите защиту заново.</span>
            <div className="settings-actions">
              <button type="button" className="secondary-button" onClick={() => setConfirmReset(false)}>Отмена</button>
              <button type="button" className="secondary-button danger" onClick={() => onLogout(true)}>Сбросить и выйти</button>
            </div>
          </div>
          : <button type="button" className="text-button" onClick={() => setConfirmReset(true)}>Потерян телефон и резервные коды</button>}
        <p className="login-note">Без кода программа не подключается к почте и не показывает переписку.</p>
      </form>
    </div>
  )
}
