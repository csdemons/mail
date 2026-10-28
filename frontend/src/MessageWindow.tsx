import { useCallback, useEffect, useState } from 'react'
import {
  Archive, CircleAlert, Flag, Forward, Mail, Minus, Plus, Printer, Reply, ReplyAll, Trash2, X,
} from 'lucide-react'

import { api, errorText, formatFullDate } from './api'
import { AttachmentList } from './AttachmentViewer'
import Composer from './Composer'
import MessageBody from './MessageBody'
import { trashFor } from './FolderPicker'
import { buildDraft, ensureArchive, initials, senderLabel } from './mailActions'
import type { Contact, Draft, Folder, MessageDetail, Settings } from './types'

declare global {
  interface Window { __RRCKI_MESSAGE?: { folder: string; uid: number } }
}

const FONT_KEY = 'rrcki-mail.reader-font'
const THEME_KEY = 'rrcki-mail.theme'

function loadNumber(key: string, fallback: number) {
  try {
    const v = Number(localStorage.getItem(key))
    return Number.isFinite(v) && v > 0 ? v : fallback
  } catch { return fallback }
}

/** Отдельное окно письма: крупный текст для длинных писем и все действия над письмом. */
export default function MessageWindow({ folder, uid }: { folder: string; uid: number }) {
  const [detail, setDetail] = useState<MessageDetail | null>(null)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [profile, setProfile] = useState<Settings | null>(null)
  const [folders, setFolders] = useState<Folder[]>([])
  const [draft, setDraft] = useState<Draft | null>(null)
  const [contacts, setContacts] = useState<Contact[]>([])
  const [confirmDelete, setConfirmDelete] = useState(false)
  const [ncConnected, setNcConnected] = useState(false)
  const [font, setFont] = useState(() => loadNumber(FONT_KEY, 15))
  const [dark] = useState(() => { try { return localStorage.getItem(THEME_KEY) === 'dark' } catch { return false } })

  useEffect(() => {
    api.getProfile().then(setProfile).catch(() => undefined)
    api.ncStatus().then((st) => setNcConnected(st.connected)).catch(() => undefined)
    api.listFolders().then(setFolders).catch(() => undefined)
    api.getMessage(folder, uid).then((m) => {
      setDetail(m)
      document.title = `${m.subject || '(без темы)'} — Atom Mail`
      if (m.unread) {
        api.setFlag(folder, uid, 'seen', true).then(() => setDetail((d) => d && { ...d, unread: false })).catch(() => undefined)
      }
    }).catch((e) => setError(errorText(e)))
  }, [folder, uid])

  useEffect(() => {
    if (!notice) return
    const t = setTimeout(() => setNotice(''), 6000)
    return () => clearTimeout(t)
  }, [notice])

  useEffect(() => {
    try { localStorage.setItem(FONT_KEY, String(font)) } catch { /* нет хранилища */ }
  }, [font])

  const me = profile?.username ?? ''
  const startDraft = useCallback((mode: 'reply' | 'replyAll' | 'forward') => {
    if (!detail) return
    setDraft(buildDraft(mode, detail, me))
    if (contacts.length === 0) api.listContacts().then((r) => setContacts(r.contacts)).catch(() => undefined)
  }, [detail, me, contacts.length])

  async function toggle(flag: 'flagged' | 'important' | 'seen') {
    if (!detail) return
    const field = flag === 'seen' ? 'unread' : flag
    const after = !detail[field]
    setDetail({ ...detail, [field]: after })
    try {
      await api.setFlag(folder, uid, flag, flag === 'seen' ? !after : after)
    } catch (e) {
      setDetail((d) => d && { ...d, [field]: !after })
      setNotice(`Не удалось изменить письмо: ${errorText(e)}`)
    }
  }

  /** После переноса письма окно закрывается — письма в этой папке больше нет. */
  async function done(text: string) {
    setNotice(text)
    setTimeout(() => { void api.closeWindow().catch(() => undefined) }, 900)
  }

  async function archive() {
    try {
      const target = await ensureArchive(folders, folder)
      if (target.id === folder) return setNotice('Письмо уже в архиве.')
      await api.moveMessages(folder, [uid], target.id)
      await done('Письмо перемещено в архив.')
    } catch (e) {
      setNotice(`Не удалось архивировать: ${errorText(e)}`)
    }
  }

  async function remove(permanent: boolean) {
    setConfirmDelete(false)
    const trash = trashFor(folders, folder)
    const toTrash = !permanent && trash && trash.id !== folder ? trash.id : null
    if (!toTrash && !permanent) return setConfirmDelete(true)
    try {
      const gone = await api.deleteMessages(folder, [uid], toTrash)
      await done(gone ? 'Письмо удалено безвозвратно.' : 'Письмо перемещено в корзину.')
    } catch (e) {
      setNotice(`Не удалось удалить: ${errorText(e)}`)
    }
  }

  async function saveAttachment(index: number) {
    try {
      const path = await api.saveAttachment(folder, uid, index)
      if (path) setNotice(`Сохранено: ${path}`)
    } catch (e) {
      setNotice(`Не удалось сохранить вложение: ${errorText(e)}`)
    }
  }

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (draft || confirmDelete || document.querySelector('.viewer-backdrop')) return
      const t = e.target as HTMLElement | null
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return
      const ctrl = e.ctrlKey || e.metaKey
      if (ctrl && e.key.toLowerCase() === 'r') { e.preventDefault(); startDraft(e.shiftKey ? 'replyAll' : 'reply') }
      else if (ctrl && e.shiftKey && e.key.toLowerCase() === 'f') { e.preventDefault(); startDraft('forward') }
      else if (e.key === 'Delete') { e.preventDefault(); void remove(e.shiftKey) }
      else if (ctrl && (e.key === '+' || e.key === '=')) { e.preventDefault(); setFont((f) => Math.min(28, f + 1)) }
      else if (ctrl && e.key === '-') { e.preventDefault(); setFont((f) => Math.max(11, f - 1)) }
      else if (e.key === 'Escape') void api.closeWindow().catch(() => undefined)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

  return (
    <div className={`app-shell message-window ${dark ? 'theme-dark' : ''}`}>
      <div className="reader-toolbar message-window-toolbar">
        <button className="secondary-button" disabled={!detail} onClick={() => startDraft('reply')} title="Ответить (Ctrl+R)"><Reply size={16} /> Ответить</button>
        <button className="secondary-button" disabled={!detail} onClick={() => startDraft('replyAll')} title="Ответить всем (Ctrl+Shift+R)"><ReplyAll size={16} /> Всем</button>
        <button className="secondary-button" disabled={!detail} onClick={() => startDraft('forward')} title="Переслать (Ctrl+Shift+F)"><Forward size={16} /> Переслать</button>
        <span className="toolbar-sep" />
        {detail && <>
          <button className={`icon-button ${detail.important ? 'important-on' : ''}`} aria-label="Важное" aria-pressed={detail.important} title={detail.important ? 'Снять «Важное»' : 'Пометить как важное'} onClick={() => void toggle('important')}><CircleAlert size={18} /></button>
          <button className={`icon-button ${detail.flagged ? 'flag-on' : ''}`} aria-label="Флажок" aria-pressed={detail.flagged} title={detail.flagged ? 'Снять флажок' : 'Поставить флажок (к исполнению)'} onClick={() => void toggle('flagged')}><Flag size={18} fill={detail.flagged ? 'currentColor' : 'none'} /></button>
          <button className="icon-button" aria-label="Непрочитанное" title={detail.unread ? 'Отметить прочитанным' : 'Отметить непрочитанным'} onClick={() => void toggle('seen')}><Mail size={18} /></button>
          <span className="toolbar-sep" />
          <button className="icon-button" aria-label="Архивировать" title="В архив" onClick={() => void archive()}><Archive size={18} /></button>
          <button className="icon-button danger-icon" aria-label="Удалить письмо" title="Удалить (Del)" onClick={() => void remove(false)}><Trash2 size={18} /></button>
        </>}
        <div className="reader-toolbar-spacer" />
        <button className="icon-button soft" aria-label="Мельче" title="Мельче (Ctrl −)" onClick={() => setFont((f) => Math.max(11, f - 1))}><Minus size={16} /></button>
        <span className="font-size-label">{font}</span>
        <button className="icon-button soft" aria-label="Крупнее" title="Крупнее (Ctrl +)" onClick={() => setFont((f) => Math.min(28, f + 1))}><Plus size={16} /></button>
        <button className="icon-button soft" aria-label="Печать" title="Печать" onClick={() => window.print()}><Printer size={17} /></button>
      </div>
      {detail ? <div className="reader-content message-window-content">
        <div className="reader-title-row"><h1>{detail.subject || '(без темы)'}</h1></div>
        <div className="sender-block">
          <div className="sender-avatar sender-avatar-large">{initials(senderLabel(detail))}</div>
          <div className="sender-details">
            <strong>{senderLabel(detail)}</strong>
            {detail.fromName && <span>{detail.fromAddress}</span>}
            {detail.to.length > 0 && <span>Кому: {detail.to.join(', ')}</span>}
            {detail.cc.length > 0 && <span>Копия: {detail.cc.join(', ')}</span>}
            <small>{formatFullDate(detail.date)}</small>
          </div>
        </div>
        {detail.attachments.length > 0 && <AttachmentList folder={folder} uid={uid} attachments={detail.attachments} onSave={(i) => void saveAttachment(i)} />}
        <MessageBody detail={detail} scale={font / 15} onNotice={setNotice} />
      </div> : <div className="empty-reader">
        {error ? <><strong>Не удалось открыть письмо</strong><span>{error}</span></> : <span>Загрузка письма…</span>}
      </div>}

      {notice && <div className="toast" role="status"><span>{notice}</span><button onClick={() => setNotice('')} aria-label="Закрыть"><X size={16} /></button></div>}

      {confirmDelete && <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) setConfirmDelete(false) }}>
        <div className="settings-modal confirm-modal" role="alertdialog" aria-label="Подтверждение удаления">
          <strong>Удалить безвозвратно?</strong>
          <p>Письмо «{detail?.subject || '(без темы)'}» будет удалено без возможности восстановления.</p>
          <div className="settings-actions">
            <button className="secondary-button" onClick={() => setConfirmDelete(false)}>Отмена</button>
            <button className="primary-button danger-button" autoFocus onClick={() => void remove(true)}><Trash2 size={15} /> Удалить</button>
          </div>
        </div>
      </div>}

      {draft && (
        <Composer
          from={profile?.displayName ? `${profile.displayName} <${me}>` : me}
          draft={draft}
          contacts={contacts}
          ncConnected={ncConnected}
          onClose={() => setDraft(null)}
          onSent={(text) => {
            setDraft(null)
            setNotice(text)
            if (draft.mode !== 'forward') setDetail((d) => d && { ...d, answered: true })
          }}
        />
      )}
    </div>
  )
}
