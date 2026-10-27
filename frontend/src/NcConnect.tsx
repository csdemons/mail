import { useEffect, useRef, useState } from 'react'
import { Cloud, ExternalLink, KeyRound, Loader2 } from 'lucide-react'

import { api, errorText } from './api'
import type { NcStatus } from './types'

interface Props {
  status: NcStatus
  onChange: (s: NcStatus) => void
  compact?: boolean
}

/** Подключение Nextcloud через Login Flow v2: вход в браузере, пароль приложения
 *  возвращается клиенту и хранится в системном хранилище. */
export default function NcConnect({ status, onChange, compact }: Props) {
  const [url, setUrl] = useState(status.url)
  const [phase, setPhase] = useState<'idle' | 'waiting' | 'manual'>('idle')
  const [loginUrl, setLoginUrl] = useState('')
  const [login, setLogin] = useState('')
  const [appPassword, setAppPassword] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const polling = useRef(0)

  useEffect(() => () => { polling.current += 1 }, [])

  async function start() {
    setError('')
    setBusy(true)
    try {
      const link = await api.ncConnectStart(url)
      setLoginUrl(link)
      setPhase('waiting')
      const id = ++polling.current
      const deadline = Date.now() + 15 * 60 * 1000
      while (polling.current === id && Date.now() < deadline) {
        await new Promise((r) => setTimeout(r, 2000))
        if (polling.current !== id) return
        const result = await api.ncConnectPoll()
        if (result) {
          setPhase('idle')
          onChange(result)
          return
        }
      }
      if (polling.current === id) {
        setPhase('idle')
        setError('Время ожидания входа истекло — попробуйте ещё раз.')
      }
    } catch (err) {
      setPhase('idle')
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  async function manual(event: React.FormEvent) {
    event.preventDefault()
    setBusy(true)
    setError('')
    try {
      onChange(await api.ncConnectManual(url, login, appPassword))
      setPhase('idle')
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  async function disconnect() {
    if (!window.confirm('Отключить Nextcloud? Пароль приложения будет отозван.')) return
    try {
      onChange(await api.ncDisconnect())
    } catch (err) {
      setError(errorText(err))
    }
  }

  if (status.connected) {
    return (
      <div className="nc-connect connected">
        <Cloud size={18} />
        <div><strong>Nextcloud подключён</strong><span>{status.login} · {status.url}</span></div>
        <button className="secondary-button" onClick={() => void disconnect()}>Отключить</button>
        {error && <div className="login-error">{error}</div>}
      </div>
    )
  }

  return (
    <div className={`nc-connect ${compact ? 'compact' : 'card'}`}>
      {!compact && <div className="nc-head"><Cloud size={28} /><div><strong>Подключите Nextcloud</strong><span>Доски Deck и чат Talk работают через вашу учётную запись Nextcloud.</span></div></div>}
      <label className="login-field"><span>Адрес Nextcloud</span><input value={url} onChange={(e) => setUrl(e.target.value)} disabled={phase === 'waiting'} /></label>

      {phase === 'waiting' ? (
        <div className="nc-waiting">
          <Loader2 size={16} className="spin" />
          <span>Войдите в Nextcloud в открывшемся браузере и нажмите «Разрешить доступ». Клиент подключится сам.</span>
          <button type="button" className="text-button" onClick={() => void navigator.clipboard?.writeText(loginUrl)} title={loginUrl}><ExternalLink size={14} /> Скопировать ссылку входа</button>
          <button type="button" className="text-button" onClick={() => { polling.current += 1; setPhase('idle'); setBusy(false) }}>Отмена</button>
        </div>
      ) : phase === 'manual' ? (
        <form className="nc-manual" onSubmit={manual}>
          <label className="login-field"><span>Логин Nextcloud</span><input value={login} onChange={(e) => setLogin(e.target.value)} autoFocus /></label>
          <label className="login-field"><span>Пароль приложения</span><input type="password" value={appPassword} onChange={(e) => setAppPassword(e.target.value)} placeholder="Nextcloud → Настройки → Безопасность" /></label>
          <div className="settings-actions">
            <button type="button" className="text-button" onClick={() => setPhase('idle')}>Назад</button>
            <button className="primary-button" disabled={busy}>Подключить</button>
          </div>
        </form>
      ) : (
        <div className="settings-actions">
          <button type="button" className="text-button" onClick={() => setPhase('manual')}><KeyRound size={14} /> Ввести пароль приложения</button>
          <button className="primary-button nc-start" disabled={busy} onClick={() => void start()}>Войти через браузер</button>
        </div>
      )}
      {error && <div className="login-error" role="alert">{error}</div>}
    </div>
  )
}
