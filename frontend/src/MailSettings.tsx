import { useEffect, useState } from 'react'
import { HardDrive, Inbox, MonitorSmartphone, PenLine, Plus, RefreshCw, Send, Tag, Trash2, X } from 'lucide-react'
import { nextCategoryKeyword } from './mailExtras'
import { loadSendPrefs, saveSendPrefs, type SendPrefs } from './sending'

import { api, errorText } from './api'
import type { Category, DesktopPrefs, MailboxSet, OfflineStatus, Signature, SignatureSet } from './types'

/**
 * Подписи: несколько (отделы, языки), у каждой — название. Подпись по умолчанию для новых
 * писем и для ответов/пересылки; в окне письма можно выбрать другую. Подпись для новых
 * писем сохраняется и в веб-почте (SOGo).
 */
export function SignaturesSettings({ set, onSaved }: { set: SignatureSet | null; onSaved: (set: SignatureSet, notice: string) => void }) {
  const [draft, setDraft] = useState<SignatureSet>(set ?? { items: [], newId: null, replyId: null })
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  useEffect(() => { if (set) setDraft(set) }, [set])
  const dirty = JSON.stringify(draft) !== JSON.stringify(set ?? { items: [], newId: null, replyId: null })

  function patch(id: string, p: Partial<Signature>) {
    setDraft((d) => ({ ...d, items: d.items.map((x) => (x.id === id ? { ...x, ...p } : x)) }))
  }
  function add() {
    const id = `s${Date.now().toString(36)}`
    setDraft((d) => ({
      items: [...d.items, { id, name: d.items.length ? `Подпись ${d.items.length + 1}` : 'Основная', text: '' }],
      newId: d.newId ?? id,
      replyId: d.replyId ?? id,
    }))
  }
  function remove(id: string) {
    setDraft((d) => ({ items: d.items.filter((x) => x.id !== id), newId: d.newId === id ? null : d.newId, replyId: d.replyId === id ? null : d.replyId }))
  }
  async function save() {
    if (draft.items.some((x) => !x.name.trim())) { setError('У каждой подписи должно быть название'); return }
    setBusy(true)
    setError('')
    try {
      const warning = await api.signaturesSave(draft)
      onSaved(draft, warning ?? 'Подписи сохранены.')
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="settings-section signature-settings">
      <strong><PenLine size={15} /> Подписи</strong>
      <span className="settings-hint">Добавляется в письмо над цитатой. Можно завести несколько — для отделов или языков — и выбрать нужную в окне письма.</span>
      {draft.items.map((x) => (
        <div key={x.id} className="signature-item">
          <div className="signature-item-head">
            <input aria-label="Название подписи" value={x.name} maxLength={60} onChange={(e) => patch(x.id, { name: e.target.value })} placeholder="Название" />
            <button type="button" className="icon-button" aria-label={`Удалить подпись ${x.name}`} title="Удалить подпись" onClick={() => remove(x.id)}><Trash2 size={14} /></button>
          </div>
          <textarea
            aria-label={`Текст подписи ${x.name}`}
            spellCheck
            rows={4}
            value={x.text}
            placeholder={'С уважением,\nИванов Иван Иванович\nНИЦ «Курчатовский институт»\nтел. +7 (499) 000-00-00'}
            onChange={(e) => patch(x.id, { text: e.target.value })}
          />
        </div>
      ))}
      <button type="button" className="text-button" onClick={add}><Plus size={14} /> Добавить подпись</button>
      {draft.items.length > 0 && (
        <div className="signature-defaults">
          <label>
            <span>Новые письма</span>
            <select aria-label="Подпись для новых писем" value={draft.newId ?? ''} onChange={(e) => setDraft({ ...draft, newId: e.target.value || null })}>
              <option value="">Без подписи</option>
              {draft.items.map((x) => <option key={x.id} value={x.id}>{x.name}</option>)}
            </select>
          </label>
          <label>
            <span>Ответы и пересылка</span>
            <select aria-label="Подпись для ответов" value={draft.replyId ?? ''} onChange={(e) => setDraft({ ...draft, replyId: e.target.value || null })}>
              <option value="">Без подписи</option>
              {draft.items.map((x) => <option key={x.id} value={x.id}>{x.name}</option>)}
            </select>
          </label>
        </div>
      )}
      {error && <div className="login-error" role="alert">{error}</div>}
      <div className="settings-actions">
        <button className="secondary-button" disabled={busy || !dirty} onClick={() => void save()}>Сохранить подписи</button>
      </div>
    </div>
  )
}

function size(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} КБ`
  return `${(bytes / 1024 / 1024).toFixed(bytes < 100 * 1024 * 1024 ? 1 : 0)} МБ`
}

/**
 * Подключённые ящики Exchange (общие, делегированные). Ящики с полным доступом сервер сообщает
 * сам (Autodiscover), если администратор включил автоподключение — как в Outlook; остальные
 * добавляются по имени или адресу. Здесь же видно, что ответил сервер и какие ящики не открылись.
 */
export function MailboxesSettings({ onChanged, onNotice }: { onChanged: () => void; onNotice: (text: string) => void }) {
  const [set, setSet] = useState<MailboxSet | null>(null)
  const [address, setAddress] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [found, setFound] = useState<{ name: string; email: string }[]>([])
  useEffect(() => { api.mailboxesGet().then(setSet).catch(() => undefined) }, [])
  // Поиск ящика по имени в адресной книге организации (адрес можно не знать).
  useEffect(() => {
    const q = address.trim()
    if (q.length < 2 || q.includes('@')) { setFound([]); return }
    let alive = true
    const t = window.setTimeout(() => {
      api.searchContacts(q).then((list) => {
        if (!alive) return
        const seen = new Set<string>()
        setFound(list.flatMap((c) => c.emails.slice(0, 1).map((email) => ({ name: c.fullName, email: email.toLowerCase() })))
          .filter((x) => !seen.has(x.email) && seen.add(x.email)).slice(0, 6))
      }).catch(() => { if (alive) setFound([]) })
    }, 300)
    return () => { alive = false; window.clearTimeout(t) }
  }, [address])
  if (!set) return null

  async function save(list: string[], done: string) {
    setBusy(true); setError('')
    try {
      await api.mailboxesSave(list)
      setSet(await api.mailboxesGet())
      setAddress('')
      onNotice(done)
      onChanged()
    } catch (e) { setError(errorText(e)) } finally { setBusy(false) }
  }

  async function recheck() {
    setBusy(true); setError('')
    try {
      const next = await api.mailboxesRediscover()
      setSet(next)
      onNotice(next.discoveryOk ? `Найдено ящиков с полным доступом: ${next.discovered.length}` : 'Сервер не ответил на автопоиск ящиков')
      onChanged()
    } catch (e) { setError(errorText(e)) } finally { setBusy(false) }
  }

  const add = (value = address) => {
    const a = value.trim().toLowerCase()
    if (!a) return
    setFound([])
    void save([...set.manual, a], `Ящик ${a} подключён.`)
  }
  const connected = new Set([...set.manual, ...set.discovered.map((m) => m.address)])

  return (
    <div className="settings-section mailboxes-settings">
      <strong><Inbox size={15} /> Подключённые ящики</strong>
      <span className="settings-hint">Общие и делегированные ящики Exchange показываются в списке папок в разделе «Подключённые ящики». Ящики с полным доступом подключаются сами, как в Outlook, — если администратор включил для них автоподключение. Если доступ выдан без автоподключения, найдите ящик по имени или адресу ниже.</span>
      {set.discovered.length > 0 && (
        <ul className="mailbox-list" aria-label="Подключены автоматически">
          {set.discovered.map((m) => <li key={m.address}><span>{m.name !== m.address ? <>{m.name} <small>{m.address}</small></> : m.address}</span><small>автоматически</small></li>)}
        </ul>
      )}
      <div className="mailbox-discovery">
        {set.discoveryNote && <span className={set.discoveryOk ? 'settings-hint' : 'inline-warning'} aria-label="Автопоиск ящиков">{set.discoveryNote}</span>}
        <button type="button" className="text-button" disabled={busy} onClick={() => void recheck()}><RefreshCw size={13} /> Проверить снова</button>
      </div>
      {set.manual.length > 0 && (
        <ul className="mailbox-list" aria-label="Подключены вручную">
          {set.manual.map((a) => (
            <li key={a}><span>{a}</span>
              <button className="icon-button soft" aria-label={`Отключить ящик ${a}`} title="Отключить" disabled={busy} onClick={() => void save(set.manual.filter((x) => x !== a), `Ящик ${a} отключён.`)}><X size={14} /></button>
            </li>
          ))}
        </ul>
      )}
      {set.unavailable.length > 0 && (
        <ul className="mailbox-list unavailable" aria-label="Не открылись">
          {set.unavailable.map((m) => <li key={m.address}><span>{m.address} <small>{m.name}</small></span></li>)}
        </ul>
      )}
      <div className="mailbox-add">
        <input aria-label="Адрес подключаемого ящика" placeholder="Имя ящика или office@организация.ru" value={address} disabled={busy}
          onChange={(e) => setAddress(e.target.value)} onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); add() } }} />
        <button className="secondary-button" disabled={busy || !address.trim()} onClick={() => add()}><Plus size={15} /> Подключить</button>
      </div>
      {found.length > 0 && (
        <ul className="mailbox-list mailbox-found" aria-label="Найдено в адресной книге">
          {found.map((c) => (
            <li key={c.email}><span>{c.name} <small>{c.email}</small></span>
              {connected.has(c.email) ? <small>подключён</small> : <button type="button" className="text-button" disabled={busy} onClick={() => add(c.email)}>Подключить</button>}
            </li>
          ))}
        </ul>
      )}
      {error && <span className="login-error" role="alert">{error}</span>}
    </div>
  )
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
