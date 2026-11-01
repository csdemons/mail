import { useState } from 'react'
import { AlertTriangle, CalendarClock, Send, X } from 'lucide-react'

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
    const resend = action === 'send' && s.status === 'unknown'
    if (resend && !window.confirm('Неизвестно, дошло ли это письмо: связь с почтовым сервером оборвалась во время отправки.\n\nОтправьте ещё раз, только если уверены, что получатель его не получил (например, его нет у вас в «Отправленных» и получатель подтвердил). Иначе он получит письмо дважды.\n\nОтправить ещё раз?')) return
    setBusy(`${s.folder}/${s.uid}`)
    try {
      if (action === 'send') {
        const r = await api.scheduledSendNow(s.folder, s.uid, resend)
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
              {s.status === 'unknown' && <small className="scheduled-status warn"><AlertTriangle size={12} /> Неизвестно, дошло ли письмо — автоматически не повторяется</small>}
              {s.status === 'sending' && <small className="scheduled-status">Отправляется с другого компьютера…</small>}
              {s.status === 'sent' && <small className="scheduled-status">Отправлено, черновик скоро уберётся</small>}
            </div>
            {(s.status === 'pending' || s.status === 'unknown') && (
              <button className="secondary-button" disabled={busy !== ''} onClick={() => void run(s, 'send')}><Send size={14} /> {s.status === 'unknown' ? 'Ещё раз' : 'Сейчас'}</button>
            )}
            {s.status !== 'sent' && s.status !== 'sending' && <button className="text-button" disabled={busy !== ''} onClick={() => void run(s, 'cancel')}>Отменить</button>}
          </div>
        ))}
      </div>
    </div>
  )
}
