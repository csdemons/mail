// Входящий звонок Talk: карточка поверх окна в любом разделе.
import { Phone, PhoneOff } from 'lucide-react'

import { api, errorText } from './api'
import type { Ringing } from './useMailBackground'

export default function IncomingCall({ call, onClose, onNotice }: { call: Ringing; onClose: () => void; onNotice: (text: string) => void }) {
  async function join() {
    onClose()
    try {
      await api.talkCallWindow(call.token)
    } catch (e) {
      onNotice(`Звонок не открыт: ${errorText(e)}`)
    }
  }
  return (
    <div className="incoming-call" role="alertdialog" aria-label="Входящий звонок">
      <span className="incoming-call-icon"><Phone size={20} /></span>
      <div className="incoming-call-text">
        <strong>{call.name}</strong>
        <span>Входящий звонок в Talk</span>
      </div>
      <button className="primary-button incoming-call-join" onClick={() => void join()}><Phone size={15} /> Ответить</button>
      <button className="secondary-button incoming-call-decline" aria-label="Отклонить" title="Не отвечать — звонок продолжится без вас" onClick={onClose}><PhoneOff size={15} /></button>
    </div>
  )
}
