import { api } from './api'
import { Trash2 } from 'lucide-react'
import type { Draft } from './types'

/** Предложение продолжить письмо, не сохранённое при сбое или закрытии программы. */
export interface RecoveredDraftDialogProps {
  setRecovered: React.Dispatch<React.SetStateAction<(Draft & { savedAt?: string | undefined; lostFiles?: string[] | undefined; }) | null>>
  recovered: Draft & { savedAt?: string | undefined; lostFiles?: string[] | undefined; }
  startDraft: (next: Draft, folder?: string | null) => void
}

export default function RecoveredDraftDialog({ setRecovered, recovered, startDraft }: RecoveredDraftDialogProps) {
  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) setRecovered(null) }}>
      <div className="settings-modal confirm-modal" role="alertdialog" aria-label="Несохранённое письмо">
        <div className="compose-header"><div><strong>Несохранённое письмо</strong><small>{recovered.subject || '(без темы)'}{recovered.to ? ` — ${recovered.to.replace(/,\s*$/, '')}` : ''}</small></div></div>
        <p className="confirm-text">Письмо осталось незаконченным{recovered.savedAt ? ` (${new Date(recovered.savedAt).toLocaleString('ru-RU', { day: 'numeric', month: 'long', hour: '2-digit', minute: '2-digit' })})` : ''} — программа была закрыта или прервана. Продолжить его?</p>
        {recovered.lostFiles && recovered.lostFiles.length > 0 && <p className="confirm-text muted">Вложения нужно будет приложить заново: {recovered.lostFiles.join(', ')}.</p>}
        <div className="settings-actions">
          <button className="text-button danger-text" onClick={() => { if (recovered.key) void api.draftDiscard(recovered.key, null).catch(() => undefined); void api.draftRecoverySet(null); setRecovered(null) }}><Trash2 size={14} /> Удалить</button>
          <div className="reader-toolbar-spacer" />
          <button className="secondary-button" onClick={() => setRecovered(null)}>Позже</button>
          <button className="primary-button" autoFocus onClick={() => { const { savedAt: _s, lostFiles: _l, ...d } = recovered; setRecovered(null); startDraft(d) }}>Продолжить письмо</button>
        </div>
      </div>
    </div>
  )
}
