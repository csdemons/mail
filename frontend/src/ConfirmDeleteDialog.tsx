import { Trash2 } from 'lucide-react'
import type { MessageSummary } from './types'

export interface ConfirmDeleteDialogProps {
  setConfirmDelete: React.Dispatch<React.SetStateAction<MessageSummary[] | null>>
  confirmDelete: MessageSummary[]
  deleteNow: (items: MessageSummary[], permanentAsked: boolean) => Promise<void>
}

export default function ConfirmDeleteDialog({ setConfirmDelete, confirmDelete, deleteNow }: ConfirmDeleteDialogProps) {
  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) setConfirmDelete(null) }}>
      <div className="settings-modal confirm-modal" role="alertdialog" aria-label="Подтверждение удаления">
        <strong>Удалить безвозвратно?</strong>
        <p>{confirmDelete.length === 1 ? `Письмо «${confirmDelete[0].subject || '(без темы)'}»` : `Писем: ${confirmDelete.length}`} будет удалено без возможности восстановления.</p>
        <div className="settings-actions">
          <button className="secondary-button" onClick={() => setConfirmDelete(null)}>Отмена</button>
          <button className="primary-button danger-button" autoFocus onClick={() => void deleteNow(confirmDelete, true)}><Trash2 size={15} /> Удалить</button>
        </div>
      </div>
    </div>
  )
}
