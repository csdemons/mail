import { useEffect, useState } from 'react'
import { Building, Copy, LockKeyhole, Mail, ServerCog, Stethoscope } from 'lucide-react'

import { api, errorText } from './api'
import kurchatovLogo from './assets/kurchatov-logo.png'
import type { LoginResult, Settings } from './types'

const DEFAULT_HOST = 'rp.nrcki.ru'
const DEFAULT_PORT = 993
const DEFAULT_SMTP_HOST = 'rp.nrcki.ru'
const DEFAULT_SMTP_PORT = 465

interface Props {
  initial: Settings | null
  initialError: string
  onLoggedIn: (result: LoginResult) => void
  /** Добавление ещё одной учётной записи (окно поверх программы, без переключения). */
  add?: { onAdded: (key: string) => void; onCancel: () => void }
}

export default function Login({ initial, initialError, onLoggedIn, add }: Props) {
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
  const [allowRemember, setAllowRemember] = useState(true)
  useEffect(() => {
    api.getPolicy().then((p) => { setRemember(p.rememberPassword); setAllowRemember(p.allowRememberPassword) }).catch(() => undefined)
  }, [])
  const [showServer, setShowServer] = useState(false)
  // Вид ящика: обычная почта (IMAP/SMTP) или Exchange (EWS).
  const [kind, setKind] = useState<'imap' | 'exchange'>(initial?.ewsUrl ? 'exchange' : 'imap')
  const [email, setEmail] = useState(initial?.ewsUrl ? (initial.address || initial.username) : '')
  const [exLogin, setExLogin] = useState(initial?.ewsUrl && initial.address ? initial.username : '')
  const [ewsUrl, setEwsUrl] = useState(initial?.ewsUrl ?? '')
  /** Серверы определены автоматически (или взяты из прошлого входа) — показываем, какие. */
  const [found, setFound] = useState<string>(initial ? `${initial.host}:${initial.port}, SMTP ${initial.smtpHost}:${initial.smtpPort}` : '')
  const [note, setNote] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState(initialError)
  const [diag, setDiag] = useState<string | null>(null)
  const [diagBusy, setDiagBusy] = useState(false)

  async function runExchangeDiagnose() {
    if (!password) { setError('Введите пароль — диагностика проверяет вход'); return }
    setDiagBusy(true)
    setDiag('Проверка подключения к Exchange…')
    try {
      setDiag(await api.exchangeDiagnose({ url: ewsUrl.trim() || 'auto', address: email.trim(), username: exLogin.trim(), password }))
    } catch (err) {
      setDiag(`Ошибка диагностики: ${errorText(err)}`)
    } finally {
      setDiagBusy(false)
    }
  }

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
    if (kind === 'exchange') {
      try {
        const params = {
          host: '', port: 443, username: exLogin.trim() || email.trim(), password, remember,
          smtpHost: '', smtpPort: 465, davUrl: '', ewsUrl: ewsUrl.trim() || 'auto', address: email.trim(),
        }
        if (add) {
          const entry = await api.accountAdd(params)
          setPassword('')
          add.onAdded(entry.key)
          return
        }
        const result = await api.login(params)
        setPassword('')
        onLoggedIn(result)
      } catch (err) {
        const text = errorText(err)
        if (text.includes('не найден автоматически') || text.includes('укажите его адрес вручную')) setShowServer(true)
        setError(text)
      } finally {
        setBusy(false)
      }
      return
    }
    try {
      let server = { host, port: portNumber, smtpHost, smtpPort: smtpPortNumber, login: username.trim() }
      const domainOf = (s: string) => s.split('@')[1]?.toLowerCase() ?? ''
      const sameAsBefore = initial && domainOf(initial.username) === domainOf(username)
      // Адрес и пароль: серверы — из прошлого входа того же домена или автоопределением.
      if (!showServer && !sameAsBefore) {
        if (!username.includes('@')) {
          setShowServer(true)
          throw new Error('Введите адрес почты целиком (например, ivanov@yandex.ru) или укажите серверы вручную')
        }
        let d
        try {
          d = await api.discoverServers(username)
        } catch (err) {
          setShowServer(true)
          throw err
        }
        setHost(d.imapHost); setPort(String(d.imapPort)); setSmtpHost(d.smtpHost); setSmtpPort(String(d.smtpPort))
        setFound(`${d.imapHost}:${d.imapPort}, SMTP ${d.smtpHost}:${d.smtpPort} — ${d.source}`)
        setNote(d.note ?? '')
        server = { host: d.imapHost, port: d.imapPort, smtpHost: d.smtpHost, smtpPort: d.smtpPort, login: d.username.includes('@') ? d.username : username.trim() }
      }
      const params = {
        host: server.host, port: server.port, username: server.login, password, remember,
        smtpHost: server.smtpHost, smtpPort: server.smtpPort, davUrl,
      }
      if (add) {
        const entry = await api.accountAdd(params)
        setPassword('')
        add.onAdded(entry.key)
        return
      }
      const result = await api.login(params)
      setPassword('')
      onLoggedIn(result)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className={add ? 'modal-backdrop add-account' : 'login-screen'} onMouseDown={(e) => { if (add && e.target === e.currentTarget) add.onCancel() }}>
      <form className="login-card" onSubmit={submit} aria-label={add ? 'Добавить учётную запись' : undefined}>
        <div className="login-brand">
          <img className="brand-logo" src={kurchatovLogo} alt="НИЦ «Курчатовский институт»" />
          <span>Atom Workspace</span>
        </div>
        <h1>{add ? 'Добавить учётную запись' : 'Вход в почту'}</h1>
        <p className="login-hint">{add
          ? 'Ещё один почтовый ящик: рабочий, личный или общий. Переключение — в меню учётной записи, письма всех ящиков — в «Все входящие».'
          : 'Адрес почты и пароль — серверы программа определит сама (Яндекс, Mail.ru, Gmail, корпоративная почта).'}</p>

        <div className="login-kind" role="tablist" aria-label="Вид почтового ящика">
          <button type="button" role="tab" aria-selected={kind === 'imap'} className={kind === 'imap' ? 'active' : ''} onClick={() => { setKind('imap'); setError('') }}><Mail size={15} /> Почта (IMAP)</button>
          <button type="button" role="tab" aria-selected={kind === 'exchange'} className={kind === 'exchange' ? 'active' : ''} onClick={() => { setKind('exchange'); setError(''); setShowServer(false) }}><Building size={15} /> Exchange</button>
        </div>

        {kind === 'exchange' ? <>
          <label className="login-field">
            <span>Адрес почты</span>
            <input autoFocus required type="email" autoComplete="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder="ivanov@организация.ru" aria-label="Адрес почты Exchange" />
          </label>
          <label className="login-field">
            <span>Логин Windows (если отличается от адреса)</span>
            <input autoComplete="username" value={exLogin} onChange={(e) => setExLogin(e.target.value)} placeholder="ДОМЕН\ivanov" aria-label="Логин Exchange" />
          </label>
          <label className="login-field">
            <span>Пароль</span>
            <input required type="password" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} aria-label="Пароль Exchange" />
          </label>
          {showServer ? <label className="login-field">
            <span>Сервер Exchange (адрес EWS)</span>
            <input value={ewsUrl} onChange={(e) => setEwsUrl(e.target.value)} placeholder="mail.организация.ru или https://…/EWS/Exchange.asmx" aria-label="Сервер Exchange" />
          </label> : <button type="button" className="text-button login-server-toggle" onClick={() => setShowServer(true)}>
            <ServerCog size={15} /> {ewsUrl ? `Сервер: ${ewsUrl}` : 'Сервер определится автоматически (Autodiscover)'} · Указать вручную
          </button>}
        </> : <>
        <label className="login-field">
          <span>Адрес почты</span>
          <input autoFocus required autoComplete="username" value={username}
            onChange={(e) => setUsername(e.target.value)} placeholder="ivanov@организация.ru" />
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
            <ServerCog size={15} /> {found ? `Серверы: ${found}` : 'Серверы определятся автоматически'} · Настроить вручную
          </button>
        )}
        </>}

        <label className="login-remember">
          <input type="checkbox" checked={remember && allowRemember} disabled={!allowRemember} title={allowRemember ? undefined : 'Запрещено политикой администратора'} onChange={(e) => setRemember(e.target.checked)} />
          <span>Запомнить пароль в системном хранилище</span>
        </label>

        {note && <div className="login-note-warn" role="note">{note}</div>}
        {error && <div className="login-error" role="alert">{error}</div>}
        {kind === 'exchange' && (error || diag !== null) && (
          <button type="button" className="text-button login-diag-button" disabled={diagBusy} onClick={() => void runExchangeDiagnose()}>
            <Stethoscope size={15} /> {diagBusy ? 'Проверка…' : 'Диагностика подключения'}
          </button>
        )}
        {kind === 'exchange' && diag !== null && (
          <div className="diag-block login-diag">
            <textarea className="diag-text" readOnly value={diag} aria-label="Отчёт диагностики Exchange" />
            <div className="login-diag-actions">
              <button type="button" className="text-button" onClick={() => void navigator.clipboard.writeText(diag).catch(() => undefined)}><Copy size={14} /> Копировать отчёт</button>
              <span>Пароля в отчёте нет. Проверка делает до трёх попыток входа.</span>
            </div>
          </div>
        )}

        <button className="primary-button login-submit" type="submit" disabled={busy}>
          <LockKeyhole size={16} /> {busy ? 'Подключение…' : add ? 'Добавить' : 'Войти'}
        </button>
        {add && <button type="button" className="text-button" onClick={add.onCancel}>Отмена</button>}
        <p className="login-note">Соединение защищено TLS. Пароль не передаётся никуда, кроме почтового сервера.</p>
        <p className="login-version">Atom Workspace{version ? ` ${version}` : ''} · НИЦ «Курчатовский институт»</p>
      </form>
    </div>
  )
}
