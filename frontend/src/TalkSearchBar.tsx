import { Search, Loader2 } from 'lucide-react'
import { highlight } from './talkUtils'
import type { TalkSearchHit } from './types'

/** Поиск по сообщениям беседы: поле и найденные сообщения. */
export interface TalkSearchBarProps {
  searchQ: string
  setSearchQ: React.Dispatch<React.SetStateAction<string>>
  setSearchOpen: React.Dispatch<React.SetStateAction<boolean>>
  hits: TalkSearchHit[] | null
  jumpTo: (id: number) => Promise<void>
  searchBusy: boolean
}

export default function TalkSearchBar({ searchQ, setSearchQ, setSearchOpen, hits, jumpTo, searchBusy }: TalkSearchBarProps) {
  return (
    <div className="talk-search">
      <label className="search-box"><Search size={15} />
        <input autoFocus aria-label="Найти в беседе" placeholder="Найти в сообщениях беседы" value={searchQ} onChange={(e) => setSearchQ(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Escape') { setSearchOpen(false); setSearchQ('') } else if (e.key === 'Enter' && hits?.[0]) void jumpTo(hits[0].messageId) }} />
        {searchBusy && <Loader2 size={15} className="spin" />}
      </label>
      {hits && (
        <div className="talk-search-results" role="listbox" aria-label="Найденные сообщения">
          {hits.length === 0 && <div className="muted talk-search-empty">Ничего не найдено</div>}
          {hits.map((h) => (
            <button key={h.messageId} role="option" className="talk-search-hit" onClick={() => void jumpTo(h.messageId)}>
              <span className="talk-search-meta"><b>{h.author}</b>{h.timestamp > 0 && <small>{new Date(h.timestamp * 1000).toLocaleString('ru-RU', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })}</small>}</span>
              <span className="talk-search-text">{highlight(h.text, searchQ.trim())}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  )
}
