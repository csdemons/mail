import { useEffect, useState } from 'react'
import { ChevronRight, File, Folder, Home, Upload, X } from 'lucide-react'

import { api, errorText, formatSize, NC_MAIL_DIR } from './api'
import type { NcFile } from './types'

/** Выбор файлов в Nextcloud (или загрузка с компьютера) — в письмо вставляются ссылки. */
export default function NcFilePicker({ onClose, onPick }: { onClose: () => void; onPick: (links: { name: string; url: string }[]) => void }) {
  const [path, setPath] = useState('/')
  const [files, setFiles] = useState<NcFile[] | null>(null)
  const [chosen, setChosen] = useState<NcFile[]>([])
  const [busy, setBusy] = useState('')
  const [error, setError] = useState('')

  useEffect(() => {
    setFiles(null)
    api.ncFilesList(path).then(setFiles).catch((e) => { setFiles([]); setError(errorText(e)) })
  }, [path])

  const parts = path.split('/').filter(Boolean)

  async function insertChosen() {
    setBusy('Создание ссылок…')
    setError('')
    try {
      const links = []
      for (const f of chosen) links.push({ name: f.name, url: await api.ncLinkFor(f.path) })
      onPick(links)
    } catch (e) {
      setError(errorText(e))
      setBusy('')
    }
  }

  async function uploadFromComputer() {
    setError('')
    try {
      const picked = await api.pickAttachments()
      if (picked.length === 0) return
      setBusy(`Загрузка в Nextcloud (${NC_MAIL_DIR}): ${picked.map((p) => p.name).join(', ')}`)
      const links = await api.ncUploadLinks(NC_MAIL_DIR, picked.map((p) => p.id))
      onPick(links.map((l) => ({ name: l.name, url: l.url })))
    } catch (e) {
      setError(errorText(e))
      setBusy('')
    }
  }

  return (
    <div className="modal-backdrop nc-picker-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget && !busy) onClose() }}>
      <div className="settings-modal nc-picker">
        <div className="compose-header"><div><strong>Ссылка на файл в Nextcloud</strong><small>Получатель скачает файл по ссылке — письмо останется лёгким</small></div><button className="icon-button" onClick={onClose} aria-label="Закрыть"><X size={18} /></button></div>
        <nav className="files-crumbs">
          <button onClick={() => setPath('/')}><Home size={15} /></button>
          {parts.map((name, i) => <span key={i}><ChevronRight size={14} /><button onClick={() => setPath(`/${parts.slice(0, i + 1).join('/')}`)}>{name}</button></span>)}
        </nav>
        <div className="nc-picker-list">
          {files === null && <div className="muted">Загрузка…</div>}
          {files?.map((f) => f.isDir ? (
            <button key={f.path} className="file-row" onClick={() => setPath(f.path)}><Folder size={17} className="file-ico dir" /><span className="file-name">{f.name}</span></button>
          ) : (
            <label key={f.path} className={`file-row ${chosen.some((c) => c.path === f.path) ? 'selected' : ''}`}>
              <input type="checkbox" checked={chosen.some((c) => c.path === f.path)} onChange={(e) => setChosen((l) => e.target.checked ? [...l, f] : l.filter((c) => c.path !== f.path))} />
              <File size={17} className="file-ico" /><span className="file-name">{f.name}</span><span className="file-size">{formatSize(f.size)}</span>
            </label>
          ))}
          {files?.length === 0 && !error && <div className="muted">Папка пуста.</div>}
        </div>
        {busy && <div className="muted">{busy}</div>}
        {error && <div className="login-error" role="alert">{error}</div>}
        <div className="settings-actions">
          <button className="secondary-button" disabled={Boolean(busy)} onClick={() => void uploadFromComputer()}><Upload size={15} /> С компьютера…</button>
          <div className="reader-toolbar-spacer" />
          <button className="primary-button" disabled={Boolean(busy) || chosen.length === 0} onClick={() => void insertChosen()}>Вставить {chosen.length > 1 ? `ссылки (${chosen.length})` : 'ссылку'}</button>
        </div>
      </div>
    </div>
  )
}
