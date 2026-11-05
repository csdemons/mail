/** Серые заглушки на время загрузки: форма будущего списка вместо надписи «Загрузка…». */
type Variant = 'message' | 'folder' | 'room' | 'contact' | 'file' | 'card' | 'line'

const WIDTHS = [72, 54, 88, 63, 80, 46, 69, 58]

export function SkeletonRows({ count = 8, variant = 'message', label = 'Загрузка…' }: { count?: number; variant?: Variant; label?: string }) {
  return (
    <div className={`skeleton-list skeleton-${variant}`} role="status" aria-live="polite" aria-label={label} aria-busy="true">
      {Array.from({ length: count }, (_, i) => {
        const w = WIDTHS[i % WIDTHS.length]
        if (variant === 'folder' || variant === 'line') {
          return <div key={i} className="skeleton-row compact"><span className="skeleton-dot" /><span className="skeleton-bar" style={{ width: `${w}%` }} /></div>
        }
        if (variant === 'file') {
          return <div key={i} className="skeleton-row compact"><span className="skeleton-square" /><span className="skeleton-bar" style={{ width: `${w * 0.6}%` }} /><span className="skeleton-bar short" /></div>
        }
        if (variant === 'card') {
          return <div key={i} className="skeleton-card"><span className="skeleton-bar" style={{ width: `${w}%` }} /><span className="skeleton-bar thin" style={{ width: `${w - 20}%` }} /></div>
        }
        return (
          <div key={i} className="skeleton-row">
            <span className="skeleton-avatar" />
            <div className="skeleton-lines">
              <span className="skeleton-bar" style={{ width: `${Math.min(90, w * 0.7)}%` }} />
              <span className="skeleton-bar" style={{ width: `${w}%` }} />
              {variant === 'message' && <span className="skeleton-bar thin" style={{ width: `${w - 15}%` }} />}
            </div>
          </div>
        )
      })}
      <span className="visually-hidden">{label}</span>
    </div>
  )
}

/** Заглушка письма или карточки контакта в области чтения. */
export function SkeletonReader({ label = 'Загрузка письма…' }: { label?: string }) {
  return (
    <div className="skeleton-reader" role="status" aria-label={label} aria-busy="true">
      <div className="skeleton-row"><span className="skeleton-avatar large" /><div className="skeleton-lines"><span className="skeleton-bar" style={{ width: '45%' }} /><span className="skeleton-bar thin" style={{ width: '30%' }} /></div></div>
      <span className="skeleton-bar title" style={{ width: '60%' }} />
      {WIDTHS.slice(0, 6).map((w, i) => <span key={i} className="skeleton-bar" style={{ width: `${w + 10}%` }} />)}
      <span className="visually-hidden">{label}</span>
    </div>
  )
}

/** Чат: пузыри сообщений слева и справа. */
export function SkeletonChat() {
  return (
    <div className="skeleton-chat" role="status" aria-label="Загрузка сообщений…" aria-busy="true">
      {WIDTHS.slice(0, 6).map((w, i) => <span key={i} className={`skeleton-bubble ${i % 3 === 2 ? 'own' : ''}`} style={{ width: `${w * 0.6}%` }} />)}
      <span className="visually-hidden">Загрузка сообщений…</span>
    </div>
  )
}
