import { Mail, CalendarDays, ContactRound, Handshake, FileText, MessageCircle, Cloud } from 'lucide-react'
import type { ModuleName } from './viewContext'

export interface ModuleRailProps {
  module: ModuleName
  setModule: React.Dispatch<React.SetStateAction<ModuleName>>
  chatUnread: number
}

export default function ModuleRail({ module, setModule, chatUnread }: ModuleRailProps) {
  return (
    <nav className="module-rail" aria-label="Модули">
      <button className={`module ${module === 'mail' ? 'active' : ''}`} onClick={() => setModule('mail')} title="Почта"><Mail /><span>Почта</span></button>
      <button className={`module ${module === 'calendar' ? 'active' : ''}`} onClick={() => setModule('calendar')} title="Календарь"><CalendarDays /><span>Календарь</span></button>
      <button className={`module ${module === 'contacts' ? 'active' : ''}`} onClick={() => setModule('contacts')} title="Контакты"><ContactRound /><span>Контакты</span></button>
      <button className={`module ${module === 'counterparty' ? 'active' : ''}`} onClick={() => setModule('counterparty')} title="История взаимодействия с организациями"><Handshake /><span>Контрагенты</span></button>
      <button className={`module ${module === 'deck' ? 'active' : ''}`} onClick={() => setModule('deck')} title="Задачи (Deck)"><FileText /><span>Задачи</span></button>
      <button className={`module ${module === 'chat' ? 'active' : ''}`} onClick={() => setModule('chat')} title="Чат"><MessageCircle /><span>Чат</span>{chatUnread > 0 && <b className="rail-badge">{chatUnread}</b>}</button>
      <button className={`module ${module === 'files' ? 'active' : ''}`} onClick={() => setModule('files')}><Cloud /><span>Файлы</span></button>
      <div className="rail-spacer" />
    </nav>
  )
}
