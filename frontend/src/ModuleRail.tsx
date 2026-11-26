import { Mail, CalendarDays, ContactRound, Handshake, FileText, MessageCircle, Cloud, type LucideIcon } from 'lucide-react'
import type { ModuleName } from './viewContext'

/** Панель разделов слева — только значки: подпись видна в подсказке, чтобы полоса
 *  не отнимала ширину у списка писем. */
export interface ModuleRailProps {
  module: ModuleName
  setModule: React.Dispatch<React.SetStateAction<ModuleName>>
  /** Непрочитанные сообщения чата — счётчик на значке. */
  chatUnread: number
}

const ITEMS: Record<ModuleName, { label: string; icon: LucideIcon }> = {
  mail: { label: 'Почта', icon: Mail },
  calendar: { label: 'Календарь', icon: CalendarDays },
  contacts: { label: 'Контакты', icon: ContactRound },
  counterparty: { label: 'Контрагенты', icon: Handshake },
  deck: { label: 'Задачи (Deck)', icon: FileText },
  chat: { label: 'Чат', icon: MessageCircle },
  files: { label: 'Файлы', icon: Cloud },
}

/** Порядок значков: почта первой, файлы последними. */
const ORDER: ModuleName[] = ['mail', 'calendar', 'contacts', 'counterparty', 'deck', 'chat', 'files']

export default function ModuleRail({ module, setModule, chatUnread }: ModuleRailProps) {
  return (
    <nav className="module-rail" aria-label="Модули">
      {ORDER.map((id) => {
        const { label, icon: Icon } = ITEMS[id]
        return (
          <button
            key={id}
            className={`module ${module === id ? 'active' : ''}`}
            onClick={() => setModule(id)}
            title={label}
            aria-label={label}
            aria-current={module === id ? 'page' : undefined}
          >
            <Icon size={22} strokeWidth={1.75} aria-hidden="true" />
            {id === 'chat' && chatUnread > 0 && <b className="rail-badge">{chatUnread}</b>}
          </button>
        )
      })}
      <div className="rail-spacer" />
    </nav>
  )
}
