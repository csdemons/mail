import { useEffect, useMemo, useRef, useState } from 'react'
import { BarChart3, Check, FileText, Image as ImageIcon, Link2, Lock, Mic, Plus, Square, Trash2, Volume2, X } from 'lucide-react'
import { api, errorText, formatSize } from './api'
import { SkeletonRows } from './Skeleton'
import type { LinkPreview, TalkFile, TalkMessage, TalkPoll } from './types'

// ------------------------------------------------------------------ Опросы

/** Карточка опроса в ленте: голосование и результаты (как в Talk). */
export function PollCard({ token, id, question, notify }: { token: string; id: number; question: string; notify: (t: string) => void }) {
  const [poll, setPoll] = useState<TalkPoll | null>(null)
  const [choice, setChoice] = useState<number[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [revote, setRevote] = useState(false)

  useEffect(() => {
    let alive = true
    api.talkPollGet(token, id).then((p) => { if (alive) setPoll(p) }).catch((e) => { if (alive) setError(errorText(e)) })
    return () => { alive = false }
  }, [token, id])

  async function run(f: () => Promise<TalkPoll>, done?: string) {
    setBusy(true)
    try {
      const p = await f()
      setPoll(p)
      setRevote(false)
      if (done) notify(done)
    } catch (e) {
      notify(`Опрос: ${errorText(e)}`)
    } finally {
      setBusy(false)
    }
  }

  if (error) return <div className="talk-poll error"><BarChart3 size={15} /> {question} — {error}</div>
  if (!poll) return <div className="talk-poll"><BarChart3 size={15} /> {question}<SkeletonRows variant="line" count={2} label="Загрузка опроса…" /></div>

  const voted = poll.voted.length > 0
  const showResults = (voted && !revote) || poll.closed
  const total = poll.votes.reduce((a, b) => a + b, 0)
  const single = poll.maxVotes === 1
  const toggle = (i: number) => setChoice((c) => (single ? [i] : c.includes(i) ? c.filter((x) => x !== i) : [...c, i]))

  return (
    <div className={`talk-poll ${poll.closed ? 'closed' : ''}`} data-poll={poll.id}>
      <div className="talk-poll-head"><BarChart3 size={15} /> <b>{poll.question}</b>{poll.closed && <span className="talk-poll-badge"><Lock size={11} /> завершён</span>}</div>
      {showResults ? (
        <div className="talk-poll-results">
          {poll.options.map((o, i) => {
            const n = poll.votes[i] ?? 0
            const pct = total ? Math.round((n / total) * 100) : 0
            return (
              <div key={i} className={`talk-poll-result ${poll.voted.includes(i) ? 'mine' : ''}`}>
                <span className="talk-poll-option">{poll.voted.includes(i) && <Check size={13} />}{o}</span>
                {poll.hidden && !poll.closed ? <span className="muted">скрыто до завершения</span> : <span className="talk-poll-count">{pct}% · {n}</span>}
                {!(poll.hidden && !poll.closed) && <i className="talk-poll-bar" style={{ width: `${pct}%` }} />}
              </div>
            )
          })}
        </div>
      ) : (
        <div className="talk-poll-options" role={single ? 'radiogroup' : 'group'}>
          {poll.options.map((o, i) => (
            <label key={i} className="talk-poll-choice">
              <input type={single ? 'radio' : 'checkbox'} name={`poll-${poll.id}`} checked={choice.includes(i)} disabled={busy} onChange={() => toggle(i)} />
              <span>{o}</span>
            </label>
          ))}
        </div>
      )}
      <div className="talk-poll-foot">
        <span className="muted">{poll.voters} {plural(poll.voters, 'голос', 'голоса', 'голосов')}{single ? '' : ' · можно несколько вариантов'}{poll.hidden ? ' · анонимно до завершения' : ''}</span>
        <span className="reader-toolbar-spacer" />
        {!poll.closed && !showResults && <button className="primary-button small" disabled={busy || choice.length === 0} onClick={() => void run(() => api.talkPollVote(token, poll.id, choice), 'Голос учтён')}>Голосовать</button>}
        {!poll.closed && showResults && <button className="text-button" disabled={busy} onClick={() => { setChoice(poll.voted); setRevote(true) }}>Изменить голос</button>}
        {!poll.closed && revote && <button className="text-button" disabled={busy} onClick={() => setRevote(false)}>Отмена</button>}
        {!poll.closed && poll.own && <button className="text-button" disabled={busy} onClick={() => { if (window.confirm('Завершить опрос? Голосовать больше будет нельзя.')) void run(() => api.talkPollClose(token, poll.id), 'Опрос завершён') }}>Завершить</button>}
      </div>
    </div>
  )
}

function plural(n: number, one: string, few: string, many: string) {
  const m10 = n % 10, m100 = n % 100
  return m10 === 1 && m100 !== 11 ? one : m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14) ? few : many
}

/** Создание опроса. */
export function PollDialog({ token, onClose, notify }: { token: string; onClose: () => void; notify: (t: string) => void }) {
  const [question, setQuestion] = useState('')
  const [options, setOptions] = useState(['', ''])
  const [multi, setMulti] = useState(false)
  const [hidden, setHidden] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const filled = options.filter((o) => o.trim())

  async function create() {
    setBusy(true)
    setError('')
    try {
      await api.talkPollCreate(token, question, filled, multi ? 0 : 1, hidden)
      notify('Опрос отправлен')
      onClose()
    } catch (e) {
      setError(errorText(e))
      setBusy(false)
    }
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget && !busy) onClose() }}>
      <div className="settings-modal talk-poll-modal" role="dialog" aria-label="Новый опрос">
        <div className="compose-header"><div><strong>Новый опрос</strong><small>Увидят все участники беседы</small></div><button className="icon-button" onClick={onClose} aria-label="Закрыть"><X size={18} /></button></div>
        <label className="login-field"><span>Вопрос</span><input autoFocus value={question} onChange={(e) => setQuestion(e.target.value)} placeholder="Например: когда удобно провести встречу?" /></label>
        <div className="talk-poll-edit">
          <span className="muted">Варианты ответа</span>
          {options.map((o, i) => (
            <div key={i} className="talk-poll-edit-row">
              <input aria-label={`Вариант ${i + 1}`} value={o} onChange={(e) => setOptions((l) => l.map((x, j) => (j === i ? e.target.value : x)))}
                onKeyDown={(e) => { if (e.key === 'Enter' && i === options.length - 1 && o.trim()) { e.preventDefault(); setOptions((l) => [...l, '']) } }}
                placeholder={`Вариант ${i + 1}`} />
              {options.length > 2 && <button type="button" className="icon-button soft" aria-label={`Убрать вариант ${i + 1}`} onClick={() => setOptions((l) => l.filter((_, j) => j !== i))}><Trash2 size={14} /></button>}
            </div>
          ))}
          {options.length < 20 && <button type="button" className="text-button" onClick={() => setOptions((l) => [...l, ''])}><Plus size={14} /> Добавить вариант</button>}
        </div>
        <div className="settings-section notify-settings">
          <label><input type="checkbox" checked={multi} onChange={(e) => setMulti(e.target.checked)} /> Можно выбрать несколько вариантов</label>
          <label><input type="checkbox" checked={hidden} onChange={(e) => setHidden(e.target.checked)} /> Скрыть результаты до завершения</label>
        </div>
        {error && <div className="login-error" role="alert">{error}</div>}
        <div className="settings-actions">
          <div className="reader-toolbar-spacer" />
          <button className="text-button" onClick={onClose}>Отмена</button>
          <button className="primary-button" disabled={busy || !question.trim() || filled.length < 2} onClick={() => void create()}>Отправить опрос</button>
        </div>
      </div>
    </div>
  )
}

// ------------------------------------------------------------------ Голосовые

function pickMime(): string {
  for (const t of ['audio/ogg;codecs=opus', 'audio/webm;codecs=opus', 'audio/webm', 'audio/mp4']) {
    if (typeof MediaRecorder !== 'undefined' && MediaRecorder.isTypeSupported?.(t)) return t
  }
  return ''
}

/** Кнопка записи голосового: нажать — запись, ещё раз — отправить, × — отменить. */
export function VoiceRecorder({ disabled, onRecorded, notify }: { disabled: boolean; onRecorded: (blob: Blob, ext: string, seconds: number) => void; notify: (t: string) => void }) {
  const [rec, setRec] = useState<{ mr: MediaRecorder; started: number } | null>(null)
  const [now, setNow] = useState(Date.now())
  const cancelled = useRef(false)
  useEffect(() => {
    if (!rec) return
    const t = window.setInterval(() => setNow(Date.now()), 250)
    return () => window.clearInterval(t)
  }, [rec])

  async function start() {
    if (typeof MediaRecorder === 'undefined' || !navigator.mediaDevices?.getUserMedia) {
      notify('Запись голоса не поддерживается в этой системе')
      return
    }
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true })
      const mime = pickMime()
      const mr = new MediaRecorder(stream, mime ? { mimeType: mime } : undefined)
      const chunks: BlobPart[] = []
      const started = Date.now()
      cancelled.current = false
      mr.ondataavailable = (e) => { if (e.data.size) chunks.push(e.data) }
      mr.onstop = () => {
        stream.getTracks().forEach((t) => t.stop())
        setRec(null)
        if (cancelled.current) return
        const type = mr.mimeType || mime || 'audio/webm'
        const ext = type.includes('ogg') ? 'ogg' : type.includes('mp4') ? 'm4a' : 'webm'
        const secs = Math.round((Date.now() - started) / 1000)
        if (secs < 1) { notify('Слишком короткая запись'); return }
        onRecorded(new Blob(chunks, { type: type.split(';')[0] }), ext, secs)
      }
      mr.start(250)
      setRec({ mr, started })
      setNow(started)
    } catch (e) {
      notify(`Нет доступа к микрофону: ${errorText(e)}`)
    }
  }
  function stop(cancel: boolean) {
    cancelled.current = cancel
    rec?.mr.stop()
  }

  if (rec) {
    const s = Math.max(0, Math.floor((now - rec.started) / 1000))
    return (
      <span className="talk-recording" role="status">
        <i className="rec-dot" /> {Math.floor(s / 60)}:{String(s % 60).padStart(2, '0')}
        <button type="button" className="icon-button soft" aria-label="Отменить запись" title="Отменить" onClick={() => stop(true)}><X size={16} /></button>
        <button type="button" className="icon-button soft talk-rec-send" aria-label="Отправить голосовое" title="Отправить" onClick={() => stop(false)}><Square size={14} fill="currentColor" /></button>
      </span>
    )
  }
  return <button type="button" className="icon-button soft talk-mic" aria-label="Записать голосовое" title="Голосовое сообщение" disabled={disabled} onClick={() => void start()}><Mic size={18} /></button>
}

/** Проигрыватель голосового сообщения в ленте (файл загружается по нажатию). */
export function VoicePlayer({ file, load }: { file: TalkFile; load: (f: TalkFile) => Promise<string> }) {
  const [url, setUrl] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  useEffect(() => () => { if (url) URL.revokeObjectURL(url) }, [url])
  async function open() {
    setBusy(true)
    try {
      const b64 = await load(file)
      const bin = atob(b64)
      const bytes = new Uint8Array(bin.length)
      for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i)
      setUrl(URL.createObjectURL(new Blob([bytes], { type: file.mimetype || 'audio/ogg' })))
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="talk-voice">
      {url ? <audio src={url} controls autoPlay aria-label="Голосовое сообщение" />
        : <button type="button" className="secondary-button" disabled={busy} onClick={() => void open()}><Volume2 size={15} /> {busy ? 'Загрузка…' : 'Прослушать'}</button>}
      <small className="muted">{error || `голосовое · ${formatSize(file.size)}`}</small>
    </div>
  )
}

// ------------------------------------------------------------------ Превью ссылок

const URL_RE = /\bhttps?:\/\/[^\s<>"')\]]+/i
export function firstUrl(text: string): string | null {
  const m = URL_RE.exec(text)
  return m ? m[0].replace(/[.,;:!?]+$/, '') : null
}

const previews = new Map<string, Promise<LinkPreview | null>>()
function preview(url: string) {
  let p = previews.get(url)
  if (!p) {
    p = api.ncLinkPreview(url).catch(() => null)
    previews.set(url, p)
    if (previews.size > 300) previews.delete(previews.keys().next().value as string)
  }
  return p
}

/** Карточка ссылки под сообщением: заголовок, описание и картинка страницы. */
export function LinkCard({ url }: { url: string }) {
  const [p, setP] = useState<LinkPreview | null>(null)
  useEffect(() => {
    let alive = true
    void preview(url).then((v) => { if (alive) setP(v) })
    return () => { alive = false }
  }, [url])
  if (!p) return null
  return (
    <button type="button" className="talk-link-card" title={p.url} onClick={() => void api.openLink(p.url)}>
      {p.image && <img src={p.image} alt="" />}
      <span className="talk-link-text">
        <small>{p.site}</small>
        <b>{p.title}</b>
        {p.description && <span>{p.description}</span>}
      </span>
    </button>
  )
}

// ------------------------------------------------------------------ Медиа и файлы

type MediaTab = 'media' | 'file' | 'voice' | 'links'

/** Панель «Медиа и файлы» беседы: фото и видео, документы, голосовые, ссылки. */
export function MediaPanel({ token, messages, onClose, onOpen, thumb, loadVoice }: {
  token: string
  messages: TalkMessage[]
  onClose: () => void
  onOpen: (f: TalkFile, list: TalkFile[]) => void
  thumb: (f: TalkFile) => React.ReactNode
  loadVoice: (f: TalkFile) => Promise<string>
}) {
  const [tab, setTab] = useState<MediaTab>('media')
  const [items, setItems] = useState<TalkMessage[] | null>(null)
  const [error, setError] = useState('')
  useEffect(() => {
    if (tab === 'links') return
    let alive = true
    setItems(null)
    setError('')
    api.talkShared(token, tab).then((l) => { if (alive) setItems(l) }).catch((e) => { if (alive) setError(errorText(e)) })
    return () => { alive = false }
  }, [token, tab])
  const links = useMemo(() => {
    const out: { url: string; m: TalkMessage }[] = []
    const seen = new Set<string>()
    for (const m of [...messages].reverse()) {
      for (const u of m.text.match(/\bhttps?:\/\/[^\s<>"')\]]+/gi) ?? []) {
        const url = u.replace(/[.,;:!?]+$/, '')
        if (!seen.has(url)) { seen.add(url); out.push({ url, m }) }
      }
    }
    return out
  }, [messages])
  const files = useMemo(() => (items ?? []).flatMap((m) => m.files.map((f) => ({ f, m }))), [items])

  return (
    <aside className="talk-people-panel talk-media-panel" aria-label="Медиа и файлы беседы">
      <div className="calendar-toolbar">
        <strong className="calendar-title">Медиа и файлы</strong>
        <div className="reader-toolbar-spacer" />
        <button className="icon-button soft" aria-label="Закрыть медиа и файлы" onClick={onClose}><X size={17} /></button>
      </div>
      <div className="deck-tabs talk-media-tabs" role="tablist">
        <button role="tab" className={tab === 'media' ? 'active' : ''} onClick={() => setTab('media')}><ImageIcon size={14} /> Медиа</button>
        <button role="tab" className={tab === 'file' ? 'active' : ''} onClick={() => setTab('file')}><FileText size={14} /> Файлы</button>
        <button role="tab" className={tab === 'voice' ? 'active' : ''} onClick={() => setTab('voice')}><Mic size={14} /> Голосовые</button>
        <button role="tab" className={tab === 'links' ? 'active' : ''} onClick={() => setTab('links')}><Link2 size={14} /> Ссылки</button>
      </div>
      <div className="talk-media-body">
        {error && <div className="inline-warning" role="alert">{error}</div>}
        {tab !== 'links' && items === null && !error && <SkeletonRows variant={tab === 'media' ? 'card' : 'file'} count={4} />}
        {tab === 'media' && items && (
          files.length ? <div className="talk-media-grid">
            {files.map(({ f }) => (
              <button key={f.id} className="talk-media-tile" title={f.name} aria-label={`Открыть ${f.name}`} onClick={() => onOpen(f, files.map((x) => x.f))}>{thumb(f)}{f.mimetype.startsWith('video/') && <i className="talk-media-video">▶</i>}</button>
            ))}
          </div> : <div className="muted talk-media-empty">Фото и видео в беседе пока нет</div>
        )}
        {(tab === 'file' || tab === 'voice') && items && (
          files.length ? files.map(({ f, m }) => (
            tab === 'voice'
              ? <div key={f.id} className="talk-media-row"><span className="talk-media-meta"><b>{m.author}</b><small>{new Date(m.timestamp * 1000).toLocaleString('ru-RU', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })}</small></span><VoicePlayer file={f} load={loadVoice} /></div>
              : <button key={f.id} className="talk-media-row file" onClick={() => onOpen(f, files.map((x) => x.f))}><FileText size={16} /><span className="talk-media-meta"><b>{f.name}</b><small>{m.author} · {formatSize(f.size)}</small></span></button>
          )) : <div className="muted talk-media-empty">{tab === 'voice' ? 'Голосовых сообщений нет' : 'Файлов в беседе пока нет'}</div>
        )}
        {tab === 'links' && (
          links.length ? links.map(({ url, m }) => (
            <button key={url} className="talk-media-row file" title={url} onClick={() => void api.openLink(url)}><Link2 size={16} /><span className="talk-media-meta"><b>{url.replace(/^https?:\/\//, '')}</b><small>{m.author}</small></span></button>
          )) : <div className="muted talk-media-empty">Ссылок в загруженных сообщениях нет</div>
        )}
      </div>
    </aside>
  )
}
