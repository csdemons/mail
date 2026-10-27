import { useEffect, useState } from 'react'
import { LockKeyhole, Mail, ServerCog } from 'lucide-react'

import { api, errorText } from './api'
import type { LoginResult, Settings } from './types'

const DEFAULT_HOST = 'rp.nrcki.ru'
const DEFAULT_PORT = 993
const DEFAULT_SMTP_HOST = 'rp.nrcki.ru'
const DEFAULT_SMTP_PORT = 465

interface Props {
  initial: Settings | null
  initialError: string
  onLoggedIn: (result: LoginResult) => void
}

export default function Login({ initial, initialError, onLoggedIn }: Props) {
  const [version, setVersion] = useState('')
  useEffect(() => { api.appInfo().then((i) => setVersion(i.version)).catch(() => undefined) }, [])
  const [host, setHost] = useState(initial?.host ?? DEFAULT_HOST)
  const [port, setPort] = useState(String(initial?.port ?? DEFAULT_PORT))
  const [smtpHost, setSmtpHost] = useState(initial?.smtpHost ?? DEFAULT_SMTP_HOST)
  const [smtpPort, setSmtpPort] = useState(String(initial?.smtpPort ?? DEFAULT_SMTP_PORT))
  const [davUrl, setDavUrl] = useState(initial?.davUrl ?? '')
  const [username, setUsername] = useState(initial?.username ?? '')
  const [password, setPassword] = useState('')
  const [remember, setRemember] = useState(true)
  const [showServer, setShowServer] = useState(Boolean(initial && initial.host !== DEFAULT_HOST))
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState(initialError)

  async function submit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const portNumber = Number(port)
    const smtpPortNumber = Number(smtpPort)
    const validPort = (n: number) => Number.isInteger(n) && n >= 1 && n <= 65535
    if (!validPort(portNumber) || !validPort(smtpPortNumber)) {
      setError('Некорректный порт')
      return
    }
    setBusy(true)
    setError('')
    try {
      const result = await api.login({
        host, port: portNumber, username, password, remember,
        smtpHost, smtpPort: smtpPortNumber, davUrl,
      })
      setPassword('')
      onLoggedIn(result)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="login-screen">
      <form className="login-card" onSubmit={submit}>
        <div className="login-brand">
          <div className="brand-mark"><Mail size={19} fill="currentColor" /></div>
          <span>RRCKI Mail</span>
        </div>
        <h1>Вход в почту</h1>
        <p className="login-hint">Используйте корпоративную учётную запись почтового сервера.</p>

        <label className="login-field">
          <span>Логин</span>
          <input autoFocus required autoComplete="username" value={username}
            onChange={(e) => setUsername(e.target.value)} placeholder="user@nrcki.ru" />
        </label>
        <label className="login-field">
          <span>Пароль</span>
          <input required type="password" autoComplete="current-password" value={password}
            onChange={(e) => setPassword(e.target.value)} />
        </label>

        {showServer ? (<>
          <div className="login-server">
            <label className="login-field">
              <span>Входящая почта (IMAP)</span>
              <input required value={host} onChange={(e) => setHost(e.target.value)} />
            </label>
            <label className="login-field login-port">
              <span>Порт</span>
              <input required inputMode="numeric" value={port} onChange={(e) => setPort(e.target.value)} />
            </label>
          </div>
          <div className="login-server">
            <label className="login-field">
              <span>Исходящая почта (SMTP)</span>
              <input required value={smtpHost} onChange={(e) => setSmtpHost(e.target.value)} />
            </label>
            <label className="login-field login-port">
              <span>Порт</span>
              <input required inputMode="numeric" value={smtpPort} onChange={(e) => setSmtpPort(e.target.value)} />
            </label>
          </div>
          <label className="login-field">
            <span>Календарь и контакты (CalDAV/CardDAV)</span>
            <input value={davUrl} onChange={(e) => setDavUrl(e.target.value)} placeholder={`https://${host}/ (автоопределение)`} />
          </label>
        </>) : (
          <button type="button" className="text-button login-server-toggle" onClick={() => setShowServer(true)}>
            <ServerCog size={15} /> Серверы: {host}:{port}, SMTP {smtpHost}:{smtpPort}
          </button>
        )}

        <label className="login-remember">
          <input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} />
          <span>Запомнить пароль в системном хранилище</span>
        </label>

        {error && <div className="login-error" role="alert">{error}</div>}

        <button className="primary-button login-submit" type="submit" disabled={busy}>
          <LockKeyhole size={16} /> {busy ? 'Подключение…' : 'Войти'}
        </button>
        <p className="login-note">Соединение защищено TLS. Пароль не передаётся никуда, кроме почтового сервера.</p>
        <p className="login-version">RRCKI Mail{version ? ` ${version}` : ''} · НИЦ «Курчатовский институт»</p>
      </form>
    </div>
  )
}
