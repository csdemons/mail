import { useEffect, useState } from 'react'
import { Copy, Lock, ShieldCheck } from 'lucide-react'

import { api, errorText } from './api'
import type { TotpSetup, TotpStatus } from './types'

export const AUTOLOCK_KEY = 'rrcki-mail.autolock-minutes'

export function loadAutolock(): number {
  try {
    const v = Number(localStorage.getItem(AUTOLOCK_KEY) ?? '15')
    return Number.isFinite(v) && v >= 0 ? v : 15
  } catch { return 15 }
}

/** Настройка «Защита кодом (TOTP)»: замок на программе, код из приложения на телефоне. */
export default function TotpSettings({ onChange, onLock, autolock, onAutolock }: {
  onChange: (enabled: boolean) => void
  onLock: () => void
  autolock: number
  onAutolock: (minutes: number) => void
}) {
  const [status, setStatus] = useState<TotpStatus | null>(null)
  const [setup, setSetup] = useState<TotpSetup | null>(null)
  const [codes, setCodes] = useState<string[] | null>(null)
  const [code, setCode] = useState('')
  const [disabling, setDisabling] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  useEffect(() => { api.totpStatus().then(setStatus).catch(() => undefined) }, [])

  async function run(fn: () => Promise<void>) {
    setBusy(true)
    setError('')
    try { await fn() } catch (e) { setError(errorText(e)) } finally { setBusy(false) }
  }

  const begin = () => run(async () => { setSetup(await api.totpSetupBegin()); setCode('') })
  const confirm = () => run(async () => {
    try {
      setCodes(await api.totpSetupConfirm(code))
    } finally { setCode('') }
    setSetup(null)
    setStatus(await api.totpStatus())
    onChange(true)
  })
  const disable = () => run(async () => {
    try {
      await api.totpDisable(code)
    } finally { setCode('') }
    setDisabling(false)
    setCodes(null)
    setStatus(await api.totpStatus())
    onChange(false)
  })

  if (!status) return null
  const codeInput = (label: string, recovery: boolean) => (
    <input
      autoFocus
      className={recovery ? undefined : 'totp-input'}
      aria-label={label}
      placeholder={recovery ? '000000 или xxxx-xxxx' : '000000'}
      inputMode={recovery ? 'text' : 'numeric'}
      autoComplete="one-time-code"
      maxLength={recovery ? 12 : 6}
      value={code}
      onChange={(e) => setCode(recovery ? e.target.value : e.target.value.replace(/\D/g, ''))}
    />
  )

  return (
    <div className="settings-section totp-settings">
      <strong>Защита кодом (TOTP)</strong>
      <span className="settings-hint">
        Программа не откроет почту без 6-значного кода из приложения на телефоне (Яндекс Ключ, Google Authenticator, FreeOTP и т. п.) —
        даже если пароль сохранён. Это замок на этой программе: пароль почты по-прежнему действует в веб-почте и других клиентах.
      </span>

      {codes && <div className="totp-codes" role="region" aria-label="Резервные коды">
        <b>Резервные коды — сохраните их сейчас, повторно они не показываются.</b>
        <span>Каждый код действует один раз и заменяет код с телефона, если телефона нет под рукой.</span>
        <pre>{codes.join('\n')}</pre>
        <div className="settings-actions">
          <button className="secondary-button" onClick={() => void navigator.clipboard?.writeText(codes.join('\n')).catch(() => undefined)}><Copy size={15} /> Скопировать</button>
          <button className="primary-button" onClick={() => setCodes(null)}>Я сохранил коды</button>
        </div>
      </div>}

      {!status.enabled && !setup && <>
        {!status.keyringAvailable && <span className="login-error">Системное хранилище паролей недоступно — защиту кодом включить нельзя.</span>}
        <div className="settings-actions"><button className="secondary-button" disabled={busy || !status.keyringAvailable} onClick={() => void begin()}><ShieldCheck size={16} /> Включить защиту кодом</button></div>
      </>}

      {setup && <div className="totp-setup">
        <img className="totp-qr" alt="QR-код для приложения-аутентификатора" src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(setup.qrSvg)}`} />
        <div className="totp-steps">
          <span>1. Отсканируйте QR-код в приложении-аутентификаторе.</span>
          <span>Или введите ключ вручную: <code className="totp-secret" data-secret={setup.secret}>{setup.secret.replace(/(.{4})/g, '$1 ').trim()}</code></span>
          <span>2. Введите код, который покажет приложение:</span>
          <form className="totp-row" onSubmit={(e) => { e.preventDefault(); void confirm() }}>
            {codeInput('Код подтверждения', false)}
            <button className="primary-button" disabled={busy || code.length !== 6}>Включить</button>
            <button type="button" className="text-button" onClick={() => { setSetup(null); setCode(''); setError('') }}>Отмена</button>
          </form>
        </div>
      </div>}

      {status.enabled && <>
        <span className="status-pill">Включена · резервных кодов: {status.recoveryLeft}</span>
        <label className="totp-row">
          <span>Блокировать при бездействии</span>
          <select aria-label="Автоблокировка" value={autolock} onChange={(e) => onAutolock(Number(e.target.value))}>
            <option value={0}>Никогда</option>
            <option value={5}>через 5 минут</option>
            <option value={15}>через 15 минут</option>
            <option value={30}>через 30 минут</option>
            <option value={60}>через час</option>
          </select>
        </label>
        {disabling
          ? <form className="totp-row" onSubmit={(e) => { e.preventDefault(); void disable() }}>
            {codeInput('Код для отключения', true)}
            <button className="secondary-button danger" disabled={busy || !code}>Отключить</button>
            <button type="button" className="text-button" onClick={() => { setDisabling(false); setCode(''); setError('') }}>Отмена</button>
          </form>
          : <div className="settings-actions">
            <button className="secondary-button" onClick={onLock}><Lock size={16} /> Заблокировать сейчас (Ctrl+L)</button>
            <button className="secondary-button" onClick={() => { setDisabling(true); setCode('') }}>Отключить защиту</button>
          </div>}
      </>}
      {error && <span className="login-error" role="alert">{error}</span>}
    </div>
  )
}
