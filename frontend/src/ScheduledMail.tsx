import { useState } from 'react'
import { CalendarClock, Send, X } from 'lucide-react'

import { api, errorText } from './api'
import { formatWhen } from './sending'
import type { Scheduled } from './types'

/** Запланированные письма: отправить сейчас или отменить (письмо остаётся черновиком). */
export default function ScheduledMail({ items, onClose, onChanged, onNotice }: {
  items: Scheduled[]
  onClose: () => void
  onChanged: () => void
  onNotice: (text: string) => void
}) {
  const [busy, setBusy] = useState('')

  async function run(s: Scheduled, action: 'send' | 'cancel') {
    setBusy(`${s.folder}/${s.uid}`)
    try {
      if (action === 'send') {
        const r = await api.scheduledSendNow(s.folder, s.uid)
        onNotice(r.warning ?? 'Письмо отправлено.')
      } else {
        await api.scheduledCancel(s.folder, s.uid)
        onNotice('Отправка отменена — письмо осталось в «Черновиках».')
      }
      onChanged()
    } catch (e) {
      onNotice(errorText(e))
    } finally {
      setBusy('')
    }
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}>
      <div className="settings-modal scheduled-modal" role="dialog" aria-label="Запланированные письма">
        <div className="compose-header">
          <div><strong><CalendarClock size={16} /> Запланированные письма</strong><small>Уйдут в указанное время, пока Atom Mail запущен</small></div>
          <button className="icon-button" onClick={onClose} aria-label="Закрыть"><X size={18} /></button>
        </div>
        {items.length === 0 && <span className="settings-hint">Запланированных писем нет.</span>}
        {items.map((s) => (
          <div key={`${s.folder}/${s.uid}`} className="scheduled-item">
            <div>
              <strong>{s.subject || '(без темы)'}</strong>
              <small>{formatWhen(s.sendAt)} · {s.to.join(', ')}</small>
            </div>
            <button className="secondary-button" disabled={busy !== ''} onClick={() => void run(s, 'send')}><Send size={14} /> Сейчас</button>
            <button className="text-button" disabled={busy !== ''} onClick={() => void run(s, 'cancel')}>Отменить</button>
          </div>
        ))}
      </div>
    </div>
  )
}
