import { useEffect, useRef, useState } from 'react'
import { Archive, FileUp, FolderOpen, Import, Search, Square, Users } from 'lucide-react'

import { api, errorText } from './api'
import type { DavCollection, Folder, ImportSource, JobStatus } from './types'

function size(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} КБ`
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(bytes < 100 * 1024 * 1024 ? 1 : 0)} МБ`
  return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} ГБ`
}

/** Опрос хода фоновой задачи; `onDone` — один раз по завершении. */
function useJob(onDone: (s: JobStatus) => void) {
  const [id, setId] = useState<number | null>(null)
  const [status, setStatus] = useState<JobStatus | null>(null)
  const done = useRef(onDone)
  done.current = onDone

  useEffect(() => {
    if (id === null) return
    let alive = true
    const tick = async () => {
      try {
        const s = await api.jobStatus(id)
        if (!alive) return
        setStatus(s)
        if (s.finished) {
          alive = false
          done.current(s)
        }
      } catch { /* задача исчезла (выход) */ }
    }
    void tick()
    const t = window.setInterval(() => { if (alive) void tick() }, 700)
    return () => { alive = false; window.clearInterval(t) }
  }, [id])

  return { id, status, start: (n: number) => { setStatus(null); setId(n) }, reset: () => { setId(null); setStatus(null) } }
}

function JobView({ status, id, label }: { status: JobStatus | null; id: number | null; label: string }) {
  if (id === null) return null
  if (!status) return <div className="job-view"><span className="settings-hint">Запуск…</span></div>
  const pct = status.total > 0 ? Math.min(100, Math.round((status.done / status.total) * 100)) : 0
  const counters = status.kind === 'backup'
    ? `Сохранено писем: ${status.imported.toLocaleString('ru-RU')} из ${status.total.toLocaleString('ru-RU')}`
    : `${status.kind === 'contacts' ? 'Добавлено' : 'Загружено'}: ${status.imported.toLocaleString('ru-RU')}`
      + (status.duplicates ? ` · уже есть: ${status.duplicates.toLocaleString('ru-RU')}` : '')
      + (status.skipped ? ` · пропущено: ${status.skipped.toLocaleString('ru-RU')}` : '')
  return (
    <div className="job-view" role="status" aria-label={label}>
      {!status.finished && <>
        <span className="job-phase">{status.phase}</span>
        <div className="deck-progress" aria-label="Ход выполнения"><i style={{ width: `${pct}%` }} /></div>
        <span className="job-counters">{counters}</span>
        <div className="settings-actions"><button className="secondary-button" onClick={() => void api.jobCancel(id)}><Square size={14} /> Остановить</button></div>
      </>}
      {status.finished && status.summary && <span className="job-summary">{status.summary}</span>}
      {status.finished && status.error && <span className="login-error" role="alert">{status.error}</span>}
      {status.notes.length > 0 && <details className="job-notes">
        <summary>Подробности ({status.notes.length}{status.notes.length >= 30 ? '+' : ''})</summary>
        <ul>{status.notes.map((n, i) => <li key={i}>{n}</li>)}</ul>
      </details>}
    </div>
  )
}

/** Перенос почты из Outlook и Thunderbird, загрузка контактов, резервная копия. */
export function TransferSettings({ folders, label, onNotice, onImported }: {
  folders: Folder[]
  label: (f: Folder) => string
  onNotice: (text: string) => void
  onImported: () => void
}) {
  const [source, setSource] = useState<ImportSource | null>(null)
  const [found, setFound] = useState<ImportSource[] | null>(null)
  const [root, setRoot] = useState('Импорт')
  const [busy, setBusy] = useState(false)
  const importJob = useJob((s) => { onImported(); if (s.summary) onNotice(s.summary) })
  const contactsJob = useJob((s) => { if (s.summary) onNotice(s.summary) })
  const backupJob = useJob((s) => { if (s.summary) onNotice(s.cancelled ? 'Копирование остановлено.' : 'Резервная копия готова.') })
  const [books, setBooks] = useState<DavCollection[] | null>(null)
  const [book, setBook] = useState('')
  const [allFolders, setAllFolders] = useState(true)
  const [chosen, setChosen] = useState<string[]>([])

  const importing = importJob.status !== null ? !importJob.status.finished : importJob.id !== null

  async function pick(fn: () => Promise<ImportSource | null>) {
    setBusy(true)
    try {
      const s = await fn()
      if (s) { setSource(s); setFound(null); importJob.reset() }
    } catch (e) { onNotice(errorText(e)) } finally { setBusy(false) }
  }

  async function findThunderbird() {
    setBusy(true)
    try {
      const list = await api.importThunderbird()
      setFound(list)
      if (list.length === 0) onNotice('Профили Thunderbird на этом компьютере не найдены. Выберите папку профиля вручную.')
    } catch (e) { onNotice(errorText(e)) } finally { setBusy(false) }
  }

  async function startImport() {
    if (!source) return
    try { importJob.start(await api.importStart(source.id, root)) } catch (e) { onNotice(errorText(e)) }
  }

  async function loadBooks() {
    try {
      const r = await api.listContacts()
      const writable = r.books.filter((b) => b.writable)
      setBooks(writable)
      if (writable[0]) setBook(writable[0].url)
    } catch (e) { onNotice(`Контакты недоступны: ${errorText(e)}`) }
  }

  async function importContacts() {
    try {
      const id = await api.contactsImport(book)
      if (id !== null) contactsJob.start(id)
    } catch (e) { onNotice(errorText(e)) }
  }

  async function startBackup() {
    try {
      const id = await api.backupStart(allFolders ? [] : chosen)
      if (id !== null) backupJob.start(id)
    } catch (e) { onNotice(errorText(e)) }
  }

  const selectable = folders.filter((f) => f.selectable)

  return (
    <div className="settings-section transfer-settings">
      <strong><Import size={15} /> Перенос почты из Outlook и Thunderbird</strong>
      <span className="settings-hint">
        Письма загружаются на почтовый сервер в отдельную папку с сохранением структуры, флагов «прочитано» и «важное» и дат.
        Outlook: файл данных .pst («Файл → Открыть и экспортировать → Импорт и экспорт → Экспорт в файл»). Thunderbird: профиль находится автоматически, или выберите его папку.
        Повторный импорт не создаёт копий уже загруженных писем.
      </span>
      <div className="settings-actions transfer-pick">
        <button className="secondary-button" disabled={busy || importing} onClick={() => void findThunderbird()}><Search size={15} /> Найти Thunderbird</button>
        <button className="secondary-button" disabled={busy || importing} onClick={() => void pick(api.importPickFiles)}><FileUp size={15} /> Файлы (.pst, .eml, mbox)…</button>
        <button className="secondary-button" disabled={busy || importing} onClick={() => void pick(api.importPickFolder)}><FolderOpen size={15} /> Папка…</button>
      </div>
      {found && found.length > 0 && <div className="transfer-found" aria-label="Найденные учётные записи Thunderbird">
        {found.map((f) => <button key={f.id} className="transfer-source-option" onClick={() => { setSource(f); setFound(null); importJob.reset() }}>
          <strong>{f.title}</strong><span>папок: {f.folderCount} · {size(f.bytes)}</span>
        </button>)}
      </div>}
      {source && <div className="transfer-source" aria-label="Источник импорта">
        <div><strong>{source.title}</strong> <span className="settings-hint">· папок: {source.folderCount} · {size(source.bytes)}</span></div>
        <ul className="transfer-folders">
          {source.folders.slice(0, 8).map((f) => <li key={f}>{f}</li>)}
          {source.folderCount > 8 && <li>… и ещё {source.folderCount - 8}</li>}
        </ul>
        <label className="totp-row">
          <span>Папка на сервере</span>
          <input aria-label="Папка для импорта" value={root} maxLength={80} disabled={importing} onChange={(e) => setRoot(e.target.value)} />
        </label>
        <div className="settings-actions">
          <button className="primary-button" disabled={importing || !root.trim()} onClick={() => void startImport()}><Import size={15} /> Начать импорт</button>
        </div>
      </div>}
      <JobView status={importJob.status} id={importJob.id} label="Ход импорта" />

      <strong className="transfer-sub"><Users size={15} /> Контакты из файла</strong>
      <span className="settings-hint">vCard (.vcf) из Outlook, Thunderbird или телефона, CSV — экспорт адресной книги Outlook или Thunderbird. Контакты с уже известными адресами пропускаются.</span>
      {books === null
        ? <div className="settings-actions"><button className="secondary-button" onClick={() => void loadBooks()}><Users size={15} /> Выбрать адресную книгу…</button></div>
        : books.length === 0
          ? <span className="settings-hint">Нет адресной книги, в которую можно добавлять контакты.</span>
          : <div className="settings-actions">
            <select aria-label="Адресная книга для импорта" value={book} onChange={(e) => setBook(e.target.value)}>
              {books.map((b) => <option key={b.url} value={b.url}>{b.name}</option>)}
            </select>
            <button className="secondary-button" disabled={!book || (contactsJob.id !== null && !contactsJob.status?.finished)} onClick={() => void importContacts()}><FileUp size={15} /> Загрузить контакты…</button>
          </div>}
      <JobView status={contactsJob.status} id={contactsJob.id} label="Ход загрузки контактов" />

      <strong className="transfer-sub"><Archive size={15} /> Резервная копия писем</strong>
      <span className="settings-hint">Папки сохраняются в файлы .mbox в выбранной папке компьютера — их открывает Thunderbird, а импорт выше возвращает письма на сервер с флагами.</span>
      <span className="settings-hint transfer-warning" role="note">Файлы копии не зашифрованы: в них вся переписка открытым текстом. Сохраняйте копию только на зашифрованный диск или носитель и не оставляйте в общих папках.</span>
      <label><input type="radio" name="backup-scope" checked={allFolders} onChange={() => setAllFolders(true)} /> Все папки</label>
      <label><input type="radio" name="backup-scope" checked={!allFolders} onChange={() => setAllFolders(false)} /> Выбранные папки</label>
      {!allFolders && <div className="transfer-backup-folders" aria-label="Папки для копии">
        {selectable.map((f) => <label key={f.id} data-folder={f.id} style={{ paddingLeft: f.depth * 14 }}>
          <input type="checkbox" checked={chosen.includes(f.id)} onChange={(e) => setChosen((c) => e.target.checked ? [...c, f.id] : c.filter((x) => x !== f.id))} />
          {' '}{label(f)} <span className="settings-hint">{f.total.toLocaleString('ru-RU')}</span>
        </label>)}
      </div>}
      <div className="settings-actions">
        <button className="secondary-button" disabled={(!allFolders && chosen.length === 0) || (backupJob.id !== null && !backupJob.status?.finished)} onClick={() => void startBackup()}><Archive size={15} /> Сохранить копию…</button>
      </div>
      <JobView status={backupJob.status} id={backupJob.id} label="Ход резервного копирования" />
    </div>
  )
}
