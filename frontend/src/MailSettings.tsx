import { useEffect, useState } from 'react'
import { HardDrive, MonitorSmartphone, PenLine, RefreshCw, Send, Tag, Trash2, X } from 'lucide-react'
import { nextCategoryKeyword } from './mailExtras'
import { loadSendPrefs, saveSendPrefs, type SendPrefs } from './sending'

import { api, errorText } from './api'
import type { Category, DesktopPrefs, OfflineStatus } from './types'

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
      {st.error && <span className="login-error" role="alert">Письма сейчас не сохраняются на компьютере: {st.error}. Уже сохранённое не удалено — программа попробует снова при следующем входе.</span>}
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

/** Отправка: окно отмены, уведомления о прочтении. */
export function SendingSettings() {
  const [p, setP] = useState<SendPrefs>(loadSendPrefs)
  const update = (patch: Partial<SendPrefs>) => setP((cur) => { const next = { ...cur, ...patch }; saveSendPrefs(next); return next })
  return (
    <div className="settings-section notify-settings">
      <strong><Send size={15} /> Отправка</strong>
      <label className="totp-row">
        <span>Отмена отправки</span>
        <select aria-label="Отмена отправки" value={p.undoSecs} onChange={(e) => update({ undoSecs: Number(e.target.value) })}>
          <option value={0}>выключена — отправлять сразу</option>
          <option value={5}>5 секунд</option>
          <option value={10}>10 секунд</option>
          <option value={20}>20 секунд</option>
          <option value={30}>30 секунд</option>
        </select>
      </label>
      <label><input type="checkbox" checked={p.receiptDefault} onChange={(e) => update({ receiptDefault: e.target.checked })} /> Запрашивать уведомление о прочтении во всех письмах</label>
      <label className="totp-row">
        <span>Если просят уведомить о прочтении</span>
        <select aria-label="Ответ на запрос уведомления о прочтении" value={p.mdnPolicy} onChange={(e) => update({ mdnPolicy: e.target.value as SendPrefs['mdnPolicy'] })}>
          <option value="ask">спрашивать</option>
          <option value="never">никогда не отправлять</option>
        </select>
      </label>
    </div>
  )
}

/** Окно программы: трей и автозапуск. */
export function DesktopSettings({ onNotice }: { onNotice: (text: string) => void }) {
  const [p, setP] = useState<DesktopPrefs | null>(null)
  useEffect(() => { api.desktopPrefsGet().then(setP).catch(() => undefined) }, [])
  if (!p) return null
  async function update(patch: Partial<DesktopPrefs>) {
    if (!p) return
    const next = { ...p, ...patch }
    try {
      await api.desktopPrefsSet(next)
      setP(next)
    } catch (e) {
      onNotice(`Не удалось сохранить: ${errorText(e)}`)
    }
  }
  return (
    <div className="settings-section notify-settings">
      <strong><MonitorSmartphone size={15} /> Окно программы</strong>
      <label title={p.tray ? undefined : 'Значок в трее недоступен в этой системе'}>
        <input type="checkbox" checked={p.tray && p.closeToTray} disabled={!p.tray} onChange={(e) => void update({ closeToTray: e.target.checked })} />
        Закрытие окна сворачивает в трей (почта, уведомления и отложенная отправка продолжают работать)
      </label>
      <label><input type="checkbox" checked={p.autostart} onChange={(e) => void update({ autostart: e.target.checked })} /> Запускать вместе с системой{p.tray ? ' (свёрнутой в трей)' : ''}</label>
    </div>
  )
}

/** Категории писем: название и цвет; общие с веб-почтой. */
export function CategoriesSettings({ categories, onSaved }: { categories: Category[]; onSaved: (list: Category[], notice: string) => void }) {
  const [list, setList] = useState<Category[]>(categories)
  const [busy, setBusy] = useState(false)
  useEffect(() => setList(categories), [categories])
  const changed = JSON.stringify(list) !== JSON.stringify(categories)
  const edit = (i: number, patch: Partial<Category>) => setList((l) => l.map((c, j) => (j === i ? { ...c, ...patch } : c)))

  async function save() {
    setBusy(true)
    try {
      const warning = await api.categoriesSave(list)
      onSaved(list, warning ?? 'Категории сохранены — в программе и в веб-почте.')
    } catch (e) {
      onSaved(categories, `Не удалось сохранить категории: ${errorText(e)}`)
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="settings-section categories-settings">
      <strong><Tag size={15} /> Категории писем</strong>
      <span className="settings-hint">Цветные метки писем. Хранятся на почтовом сервере — видны в веб-почте и на других компьютерах.</span>
      {list.map((c, i) => c.keyword.toLowerCase() === '$label1' ? null : (
        <div key={c.keyword} className="category-edit">
          <input type="color" aria-label={`Цвет категории ${c.name}`} value={c.color.length === 4 ? `#${c.color[1]}${c.color[1]}${c.color[2]}${c.color[2]}${c.color[3]}${c.color[3]}` : c.color} onChange={(e) => edit(i, { color: e.target.value })} />
          <input aria-label="Название категории" value={c.name} maxLength={60} onChange={(e) => edit(i, { name: e.target.value })} />
          <button className="icon-button" aria-label={`Удалить категорию ${c.name}`} onClick={() => setList((l) => l.filter((_, j) => j !== i))}><X size={15} /></button>
        </div>
      ))}
      <div className="settings-actions">
        <button className="text-button" onClick={() => setList((l) => [...l, { keyword: nextCategoryKeyword(l), name: 'Новая категория', color: '#0ea5e9' }])}>+ Добавить категорию</button>
        <button className="secondary-button" disabled={busy || !changed || list.some((c) => !c.name.trim())} onClick={() => void save()}>Сохранить категории</button>
      </div>
    </div>
  )
}
