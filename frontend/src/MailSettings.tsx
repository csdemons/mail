import { useEffect, useState } from 'react'
import { HardDrive, PenLine, RefreshCw, Trash2 } from 'lucide-react'

import { api, errorText } from './api'
import type { OfflineStatus } from './types'

/** Личная подпись: вставляется в новые письма, ответы и пересылку; сохраняется и в веб-почте. */
export function SignatureSettings({ signature, onSaved }: { signature: string; onSaved: (text: string, notice: string) => void }) {
  const [text, setText] = useState(signature)
  const [busy, setBusy] = useState(false)
  useEffect(() => setText(signature), [signature])

  async function save() {
    setBusy(true)
    try {
      const warning = await api.signatureSave(text)
      onSaved(text.trim(), warning ?? 'Подпись сохранена — в программе и в веб-почте.')
    } catch (error) {
      onSaved(signature, `Не удалось сохранить подпись: ${errorText(error)}`)
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="settings-section signature-settings">
      <strong><PenLine size={15} /> Подпись</strong>
      <span className="settings-hint">Добавляется в новые письма, ответы и пересылку (над цитатой). Пусто — без подписи.</span>
      <textarea
        aria-label="Подпись"
        spellCheck
        rows={4}
        value={text}
        placeholder={'С уважением,\nИванов Иван Иванович\nНИЦ «Курчатовский институт»\nтел. +7 (499) 000-00-00'}
        onChange={(e) => setText(e.target.value)}
      />
      <div className="settings-actions">
        <button className="secondary-button" disabled={busy || text === signature} onClick={() => void save()}>Сохранить подпись</button>
      </div>
    </div>
  )
}

function size(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} КБ`
  return `${(bytes / 1024 / 1024).toFixed(bytes < 100 * 1024 * 1024 ? 1 : 0)} МБ`
}

/** Письма на компьютере: срок хранения, объём, очистка. */
export function OfflineSettings({ onNotice }: { onNotice: (text: string) => void }) {
  const [st, setSt] = useState<OfflineStatus | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    let alive = true
    const load = () => api.offlineStatus().then((s) => { if (alive) setSt(s) }).catch(() => undefined)
    void load()
    const t = window.setInterval(load, 4000)
    return () => { alive = false; window.clearInterval(t) }
  }, [])

  if (!st) return null
  if (!st.allowed) {
    return (
      <div className="settings-section offline-settings">
        <strong><HardDrive size={15} /> Письма на компьютере</strong>
        <span className="settings-hint">Хранение писем на компьютере отключено администратором — письма загружаются с сервера, поиск — на сервере.</span>
      </div>
    )
  }
  const options = [0, 7, 14, 30, 90].filter((d) => d <= st.maxDays || d === st.days)

  async function run(fn: () => Promise<void>, done: string) {
    setBusy(true)
    try { await fn(); onNotice(done); setSt(await api.offlineStatus()) } catch (e) { onNotice(errorText(e)) } finally { setBusy(false) }
  }

  return (
    <div className="settings-section offline-settings">
      <strong><HardDrive size={15} /> Письма на компьютере</strong>
      <span className="settings-hint">
        Письма за выбранный срок хранятся на компьютере в зашифрованном виде: открываются мгновенно и без сети, вложения не скачиваются заново.
        По заголовкам всех писем строится индекс — поиск сразу по всей почте. Ключ шифрования — в системном хранилище паролей; «Выйти и забыть пароль» удаляет всё.
      </span>
      <label className="totp-row">
        <span>Хранить письма</span>
        <select aria-label="Срок хранения писем" value={st.days} disabled={busy} onChange={(e) => void run(() => api.setOfflineDays(Number(e.target.value)), 'Срок хранения изменён.')}>
          {options.map((d) => <option key={d} value={d}>{d === 0 ? 'не хранить (только индекс для поиска)' : `за последние ${d} дней`}</option>)}
        </select>
      </label>
      <span className="offline-stat" aria-label="Состояние хранилища">
        {st.syncing && <RefreshCw size={13} className="spin" />}
        В индексе писем: {st.messages.toLocaleString('ru-RU')} · на компьютере: {st.cached.toLocaleString('ru-RU')} · {size(st.bytes)}
        {st.syncing ? ' · синхронизация…' : ''}
      </span>
      <div className="settings-actions">
        <button className="secondary-button" disabled={busy} onClick={() => void run(() => api.clearOffline(), 'Письма на компьютере удалены, индекс собирается заново.')}><Trash2 size={15} /> Очистить</button>
      </div>
    </div>
  )
}
