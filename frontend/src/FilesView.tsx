import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  ChevronRight, Cloud, Copy, Download, ExternalLink, File, FilePen, FilePlus, FileSpreadsheet, FileText, Folder, FolderPlus, Home, Image, Link2, Lock, Mail,
  Pencil, Presentation, RefreshCw, Search, Share2, Trash2, Upload, UserPlus, X,
} from 'lucide-react'

import { api, editNotice, isOfficeFile, errorText, formatSize, NC_MAIL_DIR } from './api'
import type { NcFile, NcShare, NcUser } from './types'

function fileIcon(f: NcFile) {
  if (f.isDir) return <Folder size={18} className="file-ico dir" />
  if (f.mime.startsWith('image/')) return <Image size={18} className="file-ico" />
  if (f.mime.includes('pdf') || f.mime.startsWith('text/') || f.mime.includes('document') || f.mime.includes('sheet')) return <FileText size={18} className="file-ico" />
  return <File size={18} className="file-ico" />
}

function when(modified: string) {
  const d = new Date(modified)
  return Number.isNaN(d.getTime()) ? '' : d.toLocaleString('ru-RU', { day: 'numeric', month: 'short', year: d.getFullYear() === new Date().getFullYear() ? undefined : 'numeric', hour: '2-digit', minute: '2-digit' })
}

export default function FilesView({ onNotice, onSendLinks }: {
  onNotice: (t: string) => void
  onSendLinks: (links: { name: string; url: string }[]) => void
}) {
  const [path, setPath] = useState('/')
  const [files, setFiles] = useState<NcFile[] | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [selected, setSelected] = useState<NcFile | null>(null)
  const [filter, setFilter] = useState('')
  const [busy, setBusy] = useState('')
  const [createMenu, setCreateMenu] = useState(false)
  const [createKind, setCreateKind] = useState<'docx' | 'xlsx' | 'pptx' | null>(null)
  const [createName, setCreateName] = useState('')
  const DEFAULT_NAMES = { docx: 'Новый документ', xlsx: 'Новая таблица', pptx: 'Новая презентация' } as const
  const pickKind = (k: 'docx' | 'xlsx' | 'pptx') => { setCreateKind(k); setCreateName(DEFAULT_NAMES[k]) }

  const load = useCallback(async (p: string) => {
    setLoading(true)
    setError('')
    try {
      const list = await api.ncFilesList(p)
      setFiles(list)
      setSelected((cur) => (cur ? list.find((f) => f.path === cur.path) ?? null : null))
    } catch (e) {
      setFiles([])
      setError(errorText(e))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => { setSelected(null); void load(path) }, [path, load])

  const crumbs = useMemo(() => {
    const parts = path.split('/').filter(Boolean)
    return parts.map((name, i) => ({ name, path: `/${parts.slice(0, i + 1).join('/')}` }))
  }, [path])

  const visible = useMemo(() => {
    const q = filter.trim().toLowerCase()
    return (files ?? []).filter((f) => !q || f.name.toLowerCase().includes(q))
  }, [files, filter])

  async function upload() {
    try {
      const picked = await api.pickAttachments()
      if (picked.length === 0) return
      setBusy(`Загрузка: ${picked.map((p) => p.name).join(', ')}`)
      await api.ncFilesUpload(path, picked.map((p) => p.id))
      onNotice(picked.length === 1 ? `Загружено: ${picked[0].name}` : `Загружено файлов: ${picked.length}`)
      await load(path)
    } catch (e) {
      onNotice(`Загрузка не удалась: ${errorText(e)}`)
    } finally {
      setBusy('')
    }
  }

  async function edit(f: NcFile) {
    try {
      setBusy(`Открываю «${f.name}»…`)
      onNotice(editNotice(await api.ncEdit(f.path, f.mime), f.name))
    } catch (e) {
      onNotice(`Не удалось открыть в редакторе: ${errorText(e)}`)
    } finally {
      setBusy('')
    }
  }

  async function createDoc(kind: 'docx' | 'xlsx' | 'pptx', raw: string) {
    const name = raw.trim()
    if (!name) return
    setCreateMenu(false)
    setCreateKind(null)
    try {
      setBusy(`Создаю «${name}.${kind}»…`)
      const r = await api.ncCreateDocument(path, name, kind)
      onNotice(editNotice(r, `${name}.${kind}`))
      await load(path)
    } catch (e) {
      onNotice(`Не удалось создать: ${errorText(e)}`)
    } finally {
      setBusy('')
    }
  }

  async function mkdir() {
    const name = window.prompt('Название папки')?.trim()
    if (!name) return
    try {
      await api.ncFilesMkdir(path, name)
      await load(path)
    } catch (e) {
      onNotice(errorText(e))
    }
  }

  return (
    <div className="module-view files-view">
      <aside className="folder-sidebar">
        <div className="sidebar-heading"><span>Nextcloud</span></div>
        <div className="folder-list">
          <button className={`folder-item ${path === '/' ? 'selected' : ''}`} onClick={() => setPath('/')}><Home size={17} /><span>Все файлы</span></button>
          <button className={`folder-item ${path === NC_MAIL_DIR ? 'selected' : ''}`} onClick={() => setPath(NC_MAIL_DIR)} title="Сюда сохраняются вложения писем и большие файлы, отправленные ссылкой"><Mail size={17} /><span>Почта</span></button>
          <button className={`folder-item ${path === '/Talk' ? 'selected' : ''}`} onClick={() => setPath('/Talk')}><Cloud size={17} /><span>Talk</span></button>
        </div>
      </aside>

      <main className="files-main">
        <div className="calendar-toolbar files-toolbar">
          <nav className="files-crumbs" aria-label="Путь">
            <button onClick={() => setPath('/')}><Home size={15} /></button>
            {crumbs.map((c) => <span key={c.path}><ChevronRight size={14} /><button onClick={() => setPath(c.path)}>{c.name}</button></span>)}
          </nav>
          <div className="reader-toolbar-spacer" />
          <label className="search-box files-filter"><Search size={15} /><input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Фильтр" /></label>
          <button className="icon-button soft" aria-label="Обновить" onClick={() => void load(path)}><RefreshCw size={17} className={loading ? 'spin' : ''} /></button>
          <div className="create-doc">
            <button className="secondary-button" aria-haspopup="menu" aria-expanded={createMenu} onClick={() => { setCreateMenu(!createMenu); setCreateKind(null) }}><FilePlus size={15} /> Создать</button>
            {createMenu && <div className="create-doc-menu popover" role="menu">
              {createKind === null ? <>
                <button role="menuitem" onClick={() => pickKind('docx')}><FileText size={15} /> Документ (.docx)</button>
                <button role="menuitem" onClick={() => pickKind('xlsx')}><FileSpreadsheet size={15} /> Таблица (.xlsx)</button>
                <button role="menuitem" onClick={() => pickKind('pptx')}><Presentation size={15} /> Презентация (.pptx)</button>
              </> : <form className="create-doc-form" onSubmit={(e) => { e.preventDefault(); void createDoc(createKind, createName) }}>
                <label>Название<input autoFocus aria-label="Название файла" value={createName} onChange={(e) => setCreateName(e.target.value)} /></label>
                <span className="muted">.{createKind} в папке {path}</span>
                <div className="settings-actions">
                  <button type="button" className="secondary-button" onClick={() => { setCreateMenu(false); setCreateKind(null) }}>Отмена</button>
                  <button type="submit" className="primary-button" disabled={!createName.trim()}>Создать и открыть</button>
                </div>
              </form>}
            </div>}
          </div>
          <button className="secondary-button" onClick={() => void mkdir()}><FolderPlus size={15} /> Папка</button>
          <button className="primary-button files-upload" disabled={Boolean(busy)} onClick={() => void upload()}><Upload size={15} /> Загрузить</button>
        </div>
        {busy && <div className="inline-warning files-busy">{busy}</div>}
        {error && <div className="inline-warning" role="alert">{error.includes('не найдено') && path === NC_MAIL_DIR ? 'Папки «Почта» пока нет — она появится при первом сохранении вложения или отправке файла ссылкой.' : error}</div>}
        <div className="files-list" role="list">
          {visible.map((f) => (
            <div key={f.path} role="listitem" className={`file-row ${selected?.path === f.path ? 'selected' : ''}`}
              onClick={() => setSelected(f)} onDoubleClick={() => (f.isDir ? setPath(f.path) : isOfficeFile(f.name) ? void edit(f) : void api.ncOpen(f.fileId ? `f/${f.fileId}` : 'apps/files'))}>
              {fileIcon(f)}
              {f.isDir
                ? <button className="file-name link" onClick={(e) => { e.stopPropagation(); setPath(f.path) }}>{f.name}</button>
                : <span className="file-name">{f.name}</span>}
              {f.shared && <Share2 size={14} className="file-shared" aria-label="Есть общий доступ" />}
              <span className="file-size">{f.isDir ? '' : formatSize(f.size)}</span>
              <span className="file-date">{when(f.modified)}</span>
            </div>
          ))}
          {files && !loading && visible.length === 0 && !error && <div className="empty-state"><Folder size={30} /><strong>{filter ? 'Ничего не найдено' : 'Папка пуста'}</strong><span>Загрузите файлы кнопкой «Загрузить».</span></div>}
          {files === null && <div className="empty-state"><span>Загрузка…</span></div>}
        </div>
        <div className="list-footer"><span>{visible.length} объектов</span><span>Nextcloud</span></div>
      </main>

      {selected && (
        <FilePanel
          key={selected.path}
          file={selected}
          onClose={() => setSelected(null)}
          onChanged={async (text) => { if (text) onNotice(text); await load(path) }}
          onOpenDir={(p) => setPath(p)}
          onSendLinks={onSendLinks}
          onNotice={onNotice}
          onEdit={() => void edit(selected)}
        />
      )}
    </div>
  )
}

function FilePanel({ file, onClose, onChanged, onOpenDir, onSendLinks, onNotice, onEdit }: {
  onEdit: () => void
  file: NcFile
  onClose: () => void
  onChanged: (text: string) => Promise<void>
  onOpenDir: (path: string) => void
  onSendLinks: (links: { name: string; url: string }[]) => void
  onNotice: (t: string) => void
}) {
  const [shares, setShares] = useState<NcShare[] | null>(null)
  const [error, setError] = useState('')
  const [password, setPassword] = useState('')
  const [expire, setExpire] = useState('')
  const [linkOpts, setLinkOpts] = useState(false)
  const [userQuery, setUserQuery] = useState('')
  const [users, setUsers] = useState<NcUser[]>([])
  const [canEdit, setCanEdit] = useState(false)
  const [busy, setBusy] = useState(false)

  const loadShares = useCallback(async () => {
    try { setShares(await api.ncShares(file.path)) } catch (e) { setShares([]); setError(errorText(e)) }
  }, [file.path])
  useEffect(() => { void loadShares() }, [loadShares])

  useEffect(() => {
    if (userQuery.trim().length < 2) { setUsers([]); return }
    let cancelled = false
    const t = setTimeout(() => { api.ncSearchUsers(userQuery).then((u) => { if (!cancelled) setUsers(u) }).catch(() => undefined) }, 250)
    return () => { cancelled = true; clearTimeout(t) }
  }, [userQuery])

  async function run(fn: () => Promise<void>) {
    setBusy(true)
    setError('')
    try { await fn() } catch (e) { setError(errorText(e)) } finally { setBusy(false) }
  }

  const links = (shares ?? []).filter((s) => s.shareType === 3)
  const people = (shares ?? []).filter((s) => s.shareType !== 3)

  async function copy(url: string) {
    try { await navigator.clipboard.writeText(url); onNotice('Ссылка скопирована.') } catch { window.prompt('Ссылка', url) }
  }

  return (
    <section className="reader-pane file-panel">
      <div className="reader-toolbar">
        <button className="icon-button soft" aria-label="Открыть в Nextcloud" title="Открыть в Nextcloud" onClick={() => void api.ncOpen(file.fileId ? `f/${file.fileId}` : 'apps/files')}><ExternalLink size={17} /></button>
        <div className="reader-toolbar-spacer" />
        <button className="icon-button soft" aria-label="Закрыть" onClick={onClose}><X size={18} /></button>
      </div>
      <div className="reader-content file-detail">
        <div className="file-head">{fileIcon(file)}<div><strong>{file.name}</strong><span>{file.isDir ? 'Папка' : formatSize(file.size)} · {when(file.modified)}</span></div></div>

        <div className="file-actions">
          {!file.isDir && isOfficeFile(file.name) && <button className="primary-button" onClick={onEdit}><FilePen size={15} /> Редактировать</button>}
          {file.isDir
            ? <button className="secondary-button" onClick={() => onOpenDir(file.path)}><Folder size={15} /> Открыть</button>
            : <button className="secondary-button" onClick={() => void run(async () => { const p = await api.ncFilesDownload(file.path); if (p) onNotice(`Сохранено: ${p}`) })}><Download size={15} /> Скачать</button>}
          <button className="primary-button" disabled={busy} onClick={() => void run(async () => { const url = await api.ncLinkFor(file.path); onSendLinks([{ name: file.name, url }]); await loadShares() })}><Mail size={15} /> Отправить ссылкой</button>
          {file.canWrite && <button className="secondary-button" onClick={() => void run(async () => {
            const name = window.prompt('Новое имя', file.name)?.trim()
            if (name && name !== file.name) { await api.ncFilesRename(file.path, name); await onChanged('Переименовано.') }
          })}><Pencil size={15} /> Переименовать</button>}
          {file.canWrite && <button className="secondary-button danger" onClick={() => void run(async () => {
            if (!window.confirm(`Удалить «${file.name}»? Его можно будет восстановить из корзины Nextcloud.`)) return
            await api.ncFilesDelete(file.path); onClose(); await onChanged('Удалено (в корзину Nextcloud).')
          })}><Trash2 size={15} /> Удалить</button>}
        </div>

        <div className="settings-section share-section">
          <strong><Link2 size={15} /> Публичные ссылки</strong>
          {links.length === 0 && shares !== null && <span className="muted">Ссылок нет.</span>}
          {links.map((s) => (
            <div key={s.id} className="share-row">
              <Link2 size={15} />
              <div className="share-info"><span className="share-url">{s.url}</span>
                <small>{[s.hasPassword && 'с паролем', s.expiration && `до ${new Date(s.expiration).toLocaleDateString('ru-RU')}`].filter(Boolean).join(' · ') || 'только просмотр'}</small></div>
              <button className="icon-button soft" aria-label="Скопировать ссылку" title="Скопировать" onClick={() => void copy(s.url)}><Copy size={15} /></button>
              <button className="icon-button soft" aria-label="Удалить ссылку" title="Удалить ссылку" onClick={() => void run(async () => { await api.ncShareDelete(s.id); await loadShares(); await onChanged('') })}><Trash2 size={15} /></button>
            </div>
          ))}
          {linkOpts ? (
            <div className="share-form">
              <label className="login-field"><span><Lock size={12} /> Пароль (необязательно)</span><input type="password" value={password} onChange={(e) => setPassword(e.target.value)} /></label>
              <label className="login-field"><span>Действует до (необязательно)</span><input type="date" value={expire} onChange={(e) => setExpire(e.target.value)} /></label>
              <div className="settings-actions">
                <button className="text-button" onClick={() => setLinkOpts(false)}>Отмена</button>
                <button className="primary-button" disabled={busy} onClick={() => void run(async () => {
                  const s = await api.ncShareLink(file.path, password || null, expire || null)
                  setPassword(''); setExpire(''); setLinkOpts(false); await loadShares(); await onChanged('')
                  if (s.url) void copy(s.url)
                })}>Создать ссылку</button>
              </div>
            </div>
          ) : <button className="text-button share-add" onClick={() => setLinkOpts(true)}><Link2 size={14} /> Создать ссылку</button>}
        </div>

        <div className="settings-section share-section">
          <strong><UserPlus size={15} /> Доступ сотрудникам</strong>
          {people.map((s) => (
            <div key={s.id} className="share-row">
              <span className="sender-avatar">{(s.shareWithName || '?').slice(0, 2).toUpperCase()}</span>
              <div className="share-info"><span>{s.shareWithName}</span><small>{s.shareType === 10 ? 'беседа Talk' : s.canEdit ? 'может изменять' : 'только просмотр'}</small></div>
              <button className="icon-button soft" aria-label={`Закрыть доступ ${s.shareWithName}`} title="Закрыть доступ" onClick={() => void run(async () => { await api.ncShareDelete(s.id); await loadShares(); await onChanged('') })}><X size={15} /></button>
            </div>
          ))}
          <label className="search-box"><Search size={15} /><input value={userQuery} onChange={(e) => setUserQuery(e.target.value)} placeholder="Имя сотрудника" /></label>
          <label className="login-remember"><input type="checkbox" checked={canEdit} onChange={(e) => setCanEdit(e.target.checked)} /><span>Разрешить изменять</span></label>
          {users.filter((u) => !people.some((p) => p.shareWith === u.id)).map((u) => (
            <button key={u.id} className="message-row share-user" onClick={() => void run(async () => {
              await api.ncShareUser(file.path, u.id, canEdit); setUserQuery(''); await loadShares(); await onChanged(`Доступ открыт: ${u.label}`)
            })}>
              <span className="sender-avatar">{u.label.slice(0, 2).toUpperCase()}</span>
              <span className="message-summary"><span className="sender-name">{u.label}</span>{u.sub && <span className="message-preview">{u.sub}</span>}</span>
            </button>
          ))}
        </div>
        {error && <div className="login-error" role="alert">{error}</div>}
      </div>
    </section>
  )
}
