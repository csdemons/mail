import { useEffect, useMemo, useRef, useState } from 'react'
import { Search } from 'lucide-react'

/**
 * Эмодзи как в Talk: весь набор Unicode (данные emojibase с русскими названиями и тегами),
 * категории, поиск по-русски, оттенок кожи, недавние. Данные подгружаются при первом открытии.
 */
export const QUICK_REACTIONS = ['👍', '❤️', '😂', '😮', '😢', '🎉', '👀', '✅']

interface Raw { emoji: string; label: string; tags?: string[]; group?: number; order?: number; version: number; skins?: { emoji: string; tone?: number | number[] }[] }
export interface Emoji { e: string; label: string; search: string; group: number; skins: string[] | null }

/**
 * Не новее Emoji 14 (2021): их показывают системные шрифты Windows 10/11, Astra 1.8, macOS 12+;
 * более новые на части компьютеров выглядели бы квадратиками.
 */
const MAX_VERSION = 14

let loaded: Promise<Emoji[]> | null = null
export function loadEmoji(): Promise<Emoji[]> {
  if (!loaded) {
    loaded = import('emojibase-data/ru/data.json').then((m) => {
      const raw = (m.default ?? m) as unknown as Raw[]
      return raw
        .filter((r) => r.group !== undefined && r.group !== 2 && r.version <= MAX_VERSION && r.emoji)
        .sort((a, b) => (a.order ?? 0) - (b.order ?? 0))
        .map((r) => ({
          e: r.emoji,
          label: r.label,
          search: [r.label, ...(r.tags ?? [])].join(' ').toLowerCase().replace(/ё/g, 'е'),
          group: r.group as number,
          // Пять оттенков (по одному на тон); составные (пары людей) не берём.
          skins: r.skins ? r.skins.filter((s) => typeof s.tone === 'number').sort((a, b) => Number(a.tone) - Number(b.tone)).map((s) => s.emoji) : null,
        }))
    })
  }
  return loaded
}

const GROUPS: { id: number; icon: string; name: string }[] = [
  { id: -1, icon: '🕘', name: 'Недавние' },
  { id: 0, icon: '😀', name: 'Смайлы' },
  { id: 1, icon: '👋', name: 'Люди и жесты' },
  { id: 3, icon: '🐱', name: 'Животные и природа' },
  { id: 4, icon: '🍕', name: 'Еда и напитки' },
  { id: 6, icon: '⚽', name: 'Занятия' },
  { id: 5, icon: '✈️', name: 'Путешествия и места' },
  { id: 7, icon: '💡', name: 'Предметы' },
  { id: 8, icon: '❤️', name: 'Символы' },
  { id: 9, icon: '🏳️', name: 'Флаги' },
]

const TONES = ['✋', '✋🏻', '✋🏼', '✋🏽', '✋🏾', '✋🏿']
const RECENT_KEY = 'atom.emoji.recent'
const TONE_KEY = 'atom.emoji.tone'
const DEFAULT_RECENT = ['👍', '😀', '😂', '❤️', '🎉', '🙏', '👀', '✅', '🔥', '😊', '🤔', '👌', '😢', '😮', '💯', '🙂']

function loadRecent(): string[] {
  try { const v = JSON.parse(localStorage.getItem(RECENT_KEY) ?? 'null'); return Array.isArray(v) && v.length ? v.slice(0, 32) : DEFAULT_RECENT } catch { return DEFAULT_RECENT }
}
function remember(e: string) {
  try { localStorage.setItem(RECENT_KEY, JSON.stringify([e, ...loadRecent().filter((x) => x !== e)].slice(0, 32))) } catch { /* без хранилища */ }
}
function loadTone(): number {
  try { const t = Number(localStorage.getItem(TONE_KEY)); return t >= 0 && t <= 5 ? t : 0 } catch { return 0 }
}

/** Выбор эмодзи: вкладки категорий, поиск, оттенок кожи, недавние. */
export function EmojiPicker({ onPick, className = '', label = 'Эмодзи' }: { onPick: (emoji: string) => void; className?: string; label?: string }) {
  const [all, setAll] = useState<Emoji[] | null>(null)
  const [error, setError] = useState('')
  const [q, setQ] = useState('')
  const [tone, setTone] = useState(loadTone)
  const [recent, setRecent] = useState(loadRecent)
  const [active, setActive] = useState(-1)
  const [hover, setHover] = useState<Emoji | null>(null)
  const gridRef = useRef<HTMLDivElement>(null)
  const sections = useRef(new Map<number, HTMLDivElement>())

  useEffect(() => { loadEmoji().then(setAll).catch((e) => setError(String(e))) }, [])

  const withTone = (x: Emoji) => (tone > 0 && x.skins && x.skins[tone - 1]) || x.e
  const byEmoji = useMemo(() => new Map((all ?? []).map((x) => [x.e, x])), [all])
  const found = useMemo(() => {
    const t = q.trim().toLowerCase().replace(/ё/g, 'е')
    if (!t || !all) return null
    const words = t.split(/\s+/)
    return all.filter((x) => words.every((w) => x.search.includes(w))).slice(0, 240)
  }, [q, all])

  function pick(x: Emoji | string) {
    const e = typeof x === 'string' ? x : withTone(x)
    remember(typeof x === 'string' ? x : x.e)
    setRecent(loadRecent())
    onPick(e)
  }
  function jump(id: number) {
    setQ('')
    setActive(id)
    requestAnimationFrame(() => {
      const el = sections.current.get(id)
      if (el && gridRef.current) gridRef.current.scrollTop = el.offsetTop - gridRef.current.offsetTop
    })
  }
  function onScroll() {
    const g = gridRef.current
    if (!g || found) return
    let cur = -1
    for (const { id } of GROUPS) {
      const el = sections.current.get(id)
      if (el && el.offsetTop - g.offsetTop <= g.scrollTop + 8) cur = id
    }
    setActive(cur)
  }

  const button = (x: Emoji) => (
    <button type="button" key={x.e} aria-label={x.label} title={x.label} onMouseEnter={() => setHover(x)} onClick={() => pick(x)}>{withTone(x)}</button>
  )

  return (
    <div className={`emoji-picker popover ${className}`} role="dialog" aria-label={label} onMouseDown={(e) => { if (!(e.target instanceof HTMLInputElement)) e.preventDefault() }}>
      <label className="emoji-search"><Search size={14} /><input autoFocus aria-label="Найти эмодзи" placeholder="Найти: улыбка, сердце, ок…" value={q} onChange={(e) => setQ(e.target.value)} /></label>
      {!found && (
        <div className="emoji-tabs" role="tablist">
          {GROUPS.map((g) => <button type="button" role="tab" key={g.id} aria-selected={active === g.id} className={active === g.id ? 'active' : ''} title={g.name} aria-label={g.name} onClick={() => jump(g.id)}>{g.icon}</button>)}
        </div>
      )}
      <div className="emoji-scroll" ref={gridRef} onScroll={onScroll}>
        {error && <div className="muted">Не удалось загрузить эмодзи: {error}</div>}
        {!all && !error && <div className="muted emoji-loading">Загрузка…</div>}
        {found && (found.length ? <div className="emoji-grid">{found.map(button)}</div> : <div className="muted emoji-empty">Ничего не найдено</div>)}
        {all && !found && GROUPS.map((g) => (
          <div key={g.id} className="emoji-section" ref={(el) => { if (el) sections.current.set(g.id, el) }}>
            <div className="emoji-section-title">{g.name}</div>
            <div className="emoji-grid">
              {g.id === -1
                ? recent.map((e) => byEmoji.get(e) ?? { e, label: e, search: '', group: -1, skins: null }).map(button)
                : all.filter((x) => x.group === g.id).map(button)}
            </div>
          </div>
        ))}
      </div>
      <div className="emoji-foot">
        <span className="emoji-preview">{hover ? <><b>{withTone(hover)}</b><span>{hover.label}</span></> : <span className="muted">Наведите на эмодзи</span>}</span>
        <span className="emoji-tones" role="radiogroup" aria-label="Оттенок кожи">
          {TONES.map((t, i) => (
            <button type="button" key={t} role="radio" aria-checked={tone === i} aria-label={i ? `Оттенок ${i}` : 'Без оттенка'} className={tone === i ? 'active' : ''}
              onClick={() => { setTone(i); try { localStorage.setItem(TONE_KEY, String(i)) } catch { /* без хранилища */ } }}>{t}</button>
          ))}
        </span>
      </div>
    </div>
  )
}
