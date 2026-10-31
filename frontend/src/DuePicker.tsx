import { useState } from 'react'
import { CalendarClock, X } from 'lucide-react'

import { localInputValue } from './sending'

/** Выбор срока флажка (дата и время напоминания). */
export default function DuePicker({ onPick, onClose, title = 'Флажок со сроком', hint = 'В срок придёт напоминание', action = 'Поставить флажок' }: {
  onPick: (d: Date) => void
  onClose: () => void
  title?: string
  hint?: string
  action?: string
}) {
  const tomorrow = new Date(); tomorrow.setDate(tomorrow.getDate() + 1); tomorrow.setHours(9, 0, 0, 0)
  const [value, setValue] = useState(localInputValue(tomorrow))
  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}>
      <div className="settings-modal confirm-modal" role="dialog" aria-label={title}>
        <div className="compose-header">
          <div><strong><CalendarClock size={16} /> {title}</strong><small>{hint}</small></div>
          <button className="icon-button" onClick={onClose} aria-label="Закрыть"><X size={18} /></button>
        </div>
        <input className="due-input" type="datetime-local" aria-label="Срок" value={value} onChange={(e) => setValue(e.target.value)} />
        <div className="settings-actions">
          <button className="secondary-button" onClick={onClose}>Отмена</button>
          <button className="primary-button" disabled={!value} onClick={() => onPick(new Date(value))}>{action}</button>
        </div>
      </div>
    </div>
  )
}
