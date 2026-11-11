import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import {
  BarChart3, Bell, BellOff, Check, Images, Wallpaper as WallpaperIcon, Timer, Loader2, CheckCheck, Copy, CornerUpLeft, Download, ExternalLink, FileText, Forward, KanbanSquare, LogOut, MailOpen, MessageCircle, MessageSquarePlus,
  AlarmClock, EyeOff, MoreHorizontal, Paperclip, Pin, PinOff, UserRound, Pencil, Phone, RefreshCw, Search, Send, Settings2, Smile, SmilePlus, Star, Trash2, Users, X,
} from 'lucide-react'

import { api, errorText, formatSize, onSessionReset, TYPING_PAUSE_MS } from './api'
import { EmojiPicker, QUICK_REACTIONS } from './Emoji'
import TalkText from './TalkText'
import Avatar from './Avatar'
import AttachmentViewer, { previewKind } from './AttachmentViewer'
import type { TalkSearchHit } from './types'
import type { AttachmentInfo, DeckBoard, DeckStack, NcUser, PickedFile, TalkFile, TalkMention, TalkMessage, TalkRoom } from './types'
import { SkeletonChat, SkeletonRows } from './Skeleton'
import Splitter from './Splitter'
import { openContextMenu, type MenuItem } from './PopupMenu'
import { ForwardDialog, RoomPeople, RoomSettings } from './TalkRoomPanel'
import WallpaperDialog from './WallpaperDialog'
import { useDarkTheme, useWallpaperVersion, wallpaperFor, wallpaperStyle } from './wallpaper'
import { firstUrl, LinkCard, MediaPanel, PollCard, PollDialog, VoicePlayer, VoiceRecorder } from './TalkExtras'
import { PinDialog, personMenu, ProfileDialog, ReminderDialog, type PersonActions } from './TalkPeople'

/** Файл чата открывается во встроенном просмотрщике (фото, PDF, текст), если известен его путь. */
function viewable(f: TalkFile): boolean {
  return !!f.path && previewKind({ name: f.name, contentType: f.mimetype }) !== null
}

function isImage(f: TalkFile): boolean {
  return previewKind({ name: f.name, contentType: f.mimetype }) === 'image'
}

/** Кэши на время работы окна: уменьшенные копии и содержимое открытых файлов. */
const previews = new Map<string, Promise<string | null>>()
const contents = new Map<string, Promise<string>>()

function cached<T>(map: Map<string, Promise<T>>, key: string, limit: number, make: () => Promise<T>): Promise<T> {
  let p = map.get(key)
  if (!p) {
    p = make()
    p.catch(() => map.delete(key))
    map.set(key, p)
    if (map.size > limit) map.delete(map.keys().next().value as string)
  }
  return p
}

function TalkThumb({ file }: { file: TalkFile }) {
  const [src, setSrc] = useState<string | null>(null)
  const [failed, setFailed] = useState(false)
  useEffect(() => {
    let alive = true
    cached(previews, file.id, 200, () => api.talkFilePreview(file.id))
      .then((d) => { if (alive) { if (d) setSrc(d); else setFailed(true) } })
      .catch(() => { if (alive) setFailed(true) })
    return () => { alive = false }
  }, [file.id])
  if (failed) return <span className="talk-image-placeholder"><FileText size={22} /></span>
  return src ? <img src={src} alt={file.name} draggable={false} /> : <span className="talk-image-placeholder">Загрузка…</span>
}

function expiryLabel(sec: number): string {
  if (sec % 604800 === 0) return `${sec / 604800} нед.`
  if (sec % 86400 === 0) return `${sec / 86400} дн.`
  if (sec % 3600 === 0) return `${sec / 3600} ч`
  return `${Math.round(sec / 60)} мин`
}

/** Найденные слова — полужирным (без HTML: только текст и <mark>). */
function highlight(text: string, q: string): React.ReactNode {
  if (!q) return text
  const low = text.toLowerCase(), ql = q.toLowerCase()
  const out: React.ReactNode[] = []
  let i = 0
  for (let j = low.indexOf(ql); j >= 0 && out.length < 40; j = low.indexOf(ql, i)) {
    out.push(text.slice(i, j), <mark key={j}>{text.slice(j, j + q.length)}</mark>)
    i = j + q.length
  }
  out.push(text.slice(i))
  return out
}

function timeOf(ts: number) {
  const d = new Date(ts * 1000)
  const today = new Date()
  return d.toDateString() === today.toDateString()
    ? d.toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' })
    : d.toLocaleString('ru-RU', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })
}

/** Новые сообщения добавляются, обновления (реакции, правки, удаление) заменяют уже показанные. */
function merge(list: TalkMessage[], incoming: TalkMessage[]): TalkMessage[] {
  const byId = new Map(list.map((m) => [m.id, m]))
  for (const m of incoming) {
    const cur = byId.get(m.id)
    if (cur || !m.update) byId.set(m.id, cur && m.update ? { ...m, update: false, parentText: m.parentText ?? cur.parentText, parentAuthor: m.parentAuthor ?? cur.parentAuthor } : { ...m, update: false })
  }
  return [...byId.values()].sort((a, b) => a.id - b.id)
}

/** Как в Talk: избранные беседы сверху, внутри — по последней активности. */
function sortRooms(list: TalkRoom[]): TalkRoom[] {
  return [...list].sort((a, b) => Number(b.favorite) - Number(a.favorite) || b.lastActivity - a.lastActivity)
}

/** Недописанные сообщения по беседам и последняя открытая беседа — переживают уход в другой раздел. */
interface Draft { text: string; mentions: [string, string][] }
const drafts = new Map<string, Draft>()
let lastRoom: string | null = null
onSessionReset(() => { drafts.clear(); lastRoom = null })


/** Последний выполненный переход из общего поиска. */
let talkFocusSeen = 0

export default function TalkView({ onUnread, onNotice, focus, onWrite }: { onUnread: (n: number) => void; onNotice?: (t: string) => void; focus?: { token: string; messageId: number | null; n: number } | null; onWrite?: (address: string) => void }) {
  const [rooms, setRooms] = useState<TalkRoom[] | null>(null)
  /** Беседа, история которой уже загружена (для перехода к сообщению из поиска). */
  const historyFor = useRef<string | null>(null)
  const [token, setToken] = useState<string | null>(() => lastRoom)
  const [messages, setMessages] = useState<TalkMessage[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [draft, setDraft] = useState('')
  const [replyTo, setReplyTo] = useState<TalkMessage | null>(null)
  const [editing, setEditing] = useState<TalkMessage | null>(null)
  const [query, setQuery] = useState('')
  const [sending, setSending] = useState(false)
  // Отправка уже идёт: повторный Enter (пока сервер отвечает) не отправляет сообщение второй раз.
  const sendingRef = useRef(false)
  const [pinFor, setPinFor] = useState<TalkMessage | null>(null)
  const [remindFor, setRemindFor] = useState<TalkMessage | null>(null)
  const [profileFor, setProfileFor] = useState<{ id: string; name: string; busy: boolean } | null>(null)
  const [pinned, setPinned] = useState<TalkMessage[]>([])
  const [files, setFiles] = useState<PickedFile[]>([])
  const [emojiOpen, setEmojiOpen] = useState(false)
  const [reactFor, setReactFor] = useState<number | null>(null)
  // Полный выбор эмодзи для реакции (после «＋» в быстрых реакциях).
  const [reactFull, setReactFull] = useState(false)
  useEffect(() => { if (reactFor === null) setReactFull(false) }, [reactFor])
  // Выбор эмодзи и реакции закрываются щелчком мимо (в ленте, списке бесед — где угодно) и Esc.
  useEffect(() => {
    if (!emojiOpen && reactFor === null) return
    const onDown = (e: MouseEvent) => {
      const t = e.target as Element | null
      if (t?.closest('.emoji-picker, .talk-emoji, .talk-react-picker, .talk-actions')) return
      setEmojiOpen(false)
      setReactFor(null)
    }
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') { setEmojiOpen(false); setReactFor(null) } }
    document.addEventListener('mousedown', onDown, true)
    document.addEventListener('keydown', onKey)
    return () => { document.removeEventListener('mousedown', onDown, true); document.removeEventListener('keydown', onKey) }
  }, [emojiOpen, reactFor])
  const [mentions, setMentions] = useState<TalkMention[]>([])
  const [mentionSel, setMentionSel] = useState(0)
  const [cardFrom, setCardFrom] = useState<TalkMessage | null>(null)
  const [newChat, setNewChat] = useState(false)
  // До какого сообщения беседу прочитали все: свои сообщения до него — «✓✓».
  const [readUpTo, setReadUpTo] = useState(0)
  // История приходит порциями по 100: более ранние — по кнопке вверху ленты.
  const [hasOlder, setHasOlder] = useState(false)
  const [olderBusy, setOlderBusy] = useState(false)
  // Панель участников (как в Talk) — открыта или нет, запоминается на этом компьютере.
  const [showPeople, setShowPeople] = useState(() => { try { return localStorage.getItem('atom.talk.people') === '1' } catch { return false } })
  useEffect(() => { try { localStorage.setItem('atom.talk.people', showPeople ? '1' : '0') } catch { /* без хранилища */ } }, [showPeople])
  const [settingsFor, setSettingsFor] = useState<TalkRoom | null>(null)
  const [showMedia, setShowMedia] = useState(false)
  // Обои: undefined — окно закрыто; иначе для какой беседы (null — для всех).
  const [wallpaperRoom, setWallpaperRoom] = useState<TalkRoom | null | undefined>(undefined)
  const dark = useDarkTheme()
  useWallpaperVersion()
  const [pollOpen, setPollOpen] = useState(false)
  // «Без звука»: следующее сообщение придёт получателям без звука уведомления.
  const [silent, setSilent] = useState(false)
  const [mediaView, setMediaView] = useState<{ files: TalkFile[]; index: number } | null>(null)
  // Поиск по сообщениям беседы (на сервере, после паузы в наборе).
  const [searchOpen, setSearchOpen] = useState(false)
  const [searchQ, setSearchQ] = useState('')
  const [hits, setHits] = useState<TalkSearchHit[] | null>(null)
  const [searchBusy, setSearchBusy] = useState(false)
  const [flash, setFlash] = useState<number | null>(null)
  const [forwarding, setForwarding] = useState<TalkMessage | null>(null)
  // Картинки из буфера обмена, ожидающие отправки: превью по id вложения.
  const [previews, setPreviews] = useState<Record<string, string>>({})
  useEffect(() => {
    // Превью отправленных или убранных картинок больше не нужны.
    const gone = Object.keys(previews).filter((id) => !files.some((f) => f.id === id))
    if (gone.length === 0) return
    gone.forEach((id) => URL.revokeObjectURL(previews[id]))
    setPreviews((p) => Object.fromEntries(Object.entries(p).filter(([id]) => !gone.includes(id))))
  }, [files, previews])
  const listRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  // Показанное в поле «@Имя» → что отправить в Talk («@"id"»).
  const mentionMap = useRef(new Map<string, string>())
  // Номер «сессии» опроса: при смене беседы старый цикл останавливается.
  const pollSession = useRef(0)
  const stickToBottom = useRef(true)
  // Куда поставить курсор после программной правки текста (упоминание, эмодзи).
  const pendingCaret = useRef<number | null>(null)
  useLayoutEffect(() => {
    const el = inputRef.current
    if (el && pendingCaret.current !== null) {
      el.focus()
      el.setSelectionRange(pendingCaret.current, pendingCaret.current)
      pendingCaret.current = null
    }
  })

  // Чей черновик сейчас в поле ввода: сохраняем при каждом изменении (не во время правки сообщения).
  const draftOwner = useRef<string | null>(null)
  useEffect(() => {
    const owner = draftOwner.current
    if (!owner || editing) return
    if (draft) drafts.set(owner, { text: draft, mentions: [...mentionMap.current.entries()] })
    else drafts.delete(owner)
  }, [draft, editing])
  function cancelEdit() {
    setEditing(null)
    const saved = token ? drafts.get(token) : undefined
    setDraft(saved?.text ?? '')
  }

  const notify = useCallback((t: string) => { if (onNotice) onNotice(t); else setError(t) }, [onNotice])

  // Просмотр файлов беседы: стрелками листаются все фото и документы беседы по порядку.
  const viewFiles = useMemo(() => messages.filter((m) => !m.deleted).flatMap((m) => m.files.filter(viewable)), [messages])
  const viewItems = useMemo<AttachmentInfo[]>(() => viewFiles.map((f, i) => ({ index: i, name: f.name, contentType: f.mimetype, size: f.size })), [viewFiles])
  const [viewing, setViewing] = useState<number | null>(null)
  useEffect(() => setViewing(null), [token])
  const viewLoad = useCallback((a: AttachmentInfo) => {
    const f = viewFiles[a.index]
    return f?.path ? cached(contents, f.path, 20, () => api.talkFileData(f.path as string)) : Promise.reject(new Error('Файл недоступен'))
  }, [viewFiles])

  function openFile(f: TalkFile) {
    const i = viewFiles.indexOf(f)
    if (i >= 0) setViewing(i)
    else void api.ncOpen(`f/${f.id}`).catch((e) => notify(errorText(e)))
  }

  async function saveFile(f: TalkFile) {
    if (!f.path) return
    try {
      const saved = await api.ncFilesDownload(f.path)
      if (saved) notify(`Сохранено: ${saved}`)
    } catch (e) {
      notify(errorText(e))
    }
  }

  const loadRooms = useCallback(async () => {
    try {
      const list = await api.talkRooms()
      setRooms(list)
      setToken((t) => t ?? list[0]?.token ?? null)
    } catch (e) {
      setError(errorText(e))
    }
  }, [])

  useEffect(() => {
    void loadRooms()
    const t = setInterval(() => void loadRooms(), 30000)
    return () => clearInterval(t)
  }, [loadRooms])

  useEffect(() => {
    if (!token) return
    const session = ++pollSession.current
    let lastId = 0
    setMessages([])
    setReadUpTo(0)
    setHasOlder(false)
    setReplyTo(null)
    setEditing(null)
    lastRoom = token
    draftOwner.current = token
    const saved = drafts.get(token)
    mentionMap.current = new Map(saved?.mentions ?? [])
    setDraft(saved?.text ?? '')
    setFiles((list) => { list.forEach((f) => void api.forgetAttachment(f.id)); return [] })
    setLoading(true)
    historyFor.current = null
    stickToBottom.current = true
    void (async () => {
      try {
        const hist = await api.talkHistory(token)
        if (pollSession.current !== session) return
        setMessages(merge([], hist.messages))
        historyFor.current = token
        setHasOlder(hist.more)
        setReadUpTo((r) => Math.max(r, hist.lastCommonRead ?? 0))
        lastId = hist.lastId
        setRooms((list) => list?.map((r) => (r.token === token ? { ...r, unread: 0 } : r)) ?? list)
      } catch (e) {
        setError(errorText(e))
      } finally {
        setLoading(false)
      }
      // Long-poll: сервер держит запрос до 30 с и отвечает, как только что-то изменится.
      while (pollSession.current === session) {
        try {
          const fresh = await api.talkPoll(token, lastId)
          if (pollSession.current !== session) return
          if (fresh.messages.length > 0) setMessages((list) => merge(list, fresh.messages))
          if (fresh.lastCommonRead) setReadUpTo((r) => Math.max(r, fresh.lastCommonRead))
          if (fresh.lastId > lastId) lastId = fresh.lastId
        } catch {
          await new Promise((r) => setTimeout(r, 5000))
        }
      }
    })()
    return () => { pollSession.current += 1 }
  }, [token])

  useEffect(() => {
    if (stickToBottom.current) listRef.current?.scrollTo({ top: listRef.current.scrollHeight })
  }, [messages])

  // Значок на панели: непрочитанное во всех беседах, кроме открытой (её мы читаем).
  useEffect(() => {
    onUnread((rooms ?? []).filter((r) => r.token !== token).reduce((n, r) => n + r.unread, 0))
  }, [rooms, token, onUnread])

  // Подсказка упоминаний: «@текст» перед курсором.
  // Запрос к серверу — только когда человек перестал печатать; устаревшие ответы отбрасываются.
  const mentionQuery = useRef<string | null>(null)
  const mentionTimer = useRef<number | undefined>(undefined)
  useEffect(() => () => window.clearTimeout(mentionTimer.current), [])
  function onDraftChange(value: string, caret: number) {
    setDraft(value)
    const before = value.slice(0, caret)
    const m = /(^|\s)@([^\s@"]{0,30})$/.exec(before)
    mentionQuery.current = m ? m[2] : null
    window.clearTimeout(mentionTimer.current)
    if (!m || !token) { setMentions([]); return }
    const q = m[2]
    const room = token
    mentionTimer.current = window.setTimeout(() => {
      api.talkMentions(room, q)
        .then((list) => { if (mentionQuery.current === q) { setMentions(list); setMentionSel(0) } })
        .catch(() => setMentions([]))
    }, TYPING_PAUSE_MS)
  }

  function pickMention(mn: TalkMention) {
    const el = inputRef.current
    const caret = el?.selectionStart ?? draft.length
    const before = draft.slice(0, caret).replace(/@([^\s@"]{0,30})$/, '')
    const label = mn.source === 'calls' ? 'all' : mn.label
    const shown = `@${label} `
    mentionMap.current.set(`@${label}`, mn.insert)
    const next = before + shown + draft.slice(caret)
    pendingCaret.current = before.length + shown.length
    setDraft(next)
    setMentions([])
  }

  /** «@Имя» из подсказок → «@"id"» для Talk. */
  function withMentions(text: string) {
    let out = text
    for (const [shown, insert] of [...mentionMap.current.entries()].sort((a, b) => b[0].length - a[0].length)) {
      out = out.split(shown).join(insert)
    }
    return out
  }

  function insertEmoji(e: string) {
    const el = inputRef.current
    const start = el?.selectionStart ?? draft.length
    const end = el?.selectionEnd ?? draft.length
    const next = draft.slice(0, start) + e + draft.slice(end)
    pendingCaret.current = start + e.length
    setDraft(next)
  }

  async function pickFiles() {
    try {
      const picked = await api.pickAttachments()
      setFiles((list) => [...list, ...picked])
    } catch (e) {
      setError(errorText(e))
    }
  }

  function dropFile(id: string) {
    setFiles((list) => list.filter((f) => f.id !== id))
    void api.forgetAttachment(id)
  }

  async function send() {
    if (!token || (!draft.trim() && files.length === 0) || sendingRef.current) return
    sendingRef.current = true
    setSending(true)
    setError('')
    setEmojiOpen(false)
    stickToBottom.current = true
    // Беседа и текст на момент отправки: пока сервер отвечает, можно уйти в другую беседу
    // или раздел — черновик очищается именно у этой беседы и только если текст не меняли.
    const room = token
    const sentDraft = draft
    try {
      const text = withMentions(draft)
      const wasEditing = Boolean(editing)
      if (editing) {
        await api.talkEdit(token, editing.id, text)
        setEditing(null)
      } else if (files.length > 0) {
        // Текст уходит подписью к первому файлу — как в веб-интерфейсе Talk.
        let caption = text
        for (const f of files) {
          await api.talkSendFile(token, f.id, caption, replyTo?.id ?? null, { silent })
          caption = ''
          setFiles((list) => list.filter((x) => x.id !== f.id))
        }
      } else {
        await api.talkSend(token, text, replyTo?.id ?? null, silent)
      }
      const here = draftOwner.current === room
      if (wasEditing) {
        // После правки — обратно недописанное сообщение, если оно было.
        const saved = drafts.get(room)
        if (here) {
          mentionMap.current = new Map(saved?.mentions ?? [])
          setDraft(saved?.text ?? '')
        }
      } else {
        if (drafts.get(room)?.text === sentDraft) drafts.delete(room)
        if (here) {
          mentionMap.current.clear()
          setDraft((d) => (d === sentDraft ? '' : d))
        }
      }
      if (here) { setReplyTo(null); setSilent(false) }
    } catch (e) {
      setError(errorText(e))
    } finally {
      sendingRef.current = false
      setSending(false)
    }
  }

  async function react(m: TalkMessage, emoji: string) {
    if (!token) return
    setReactFor(null)
    const mine = m.reactions.find((r) => r.emoji === emoji)?.own ?? false
    // Сразу показываем, сервер подтвердит обновлением через опрос.
    setMessages((list) => list.map((x) => {
      if (x.id !== m.id) return x
      const has = x.reactions.find((r) => r.emoji === emoji)
      const reactions = has
        ? x.reactions.map((r) => (r.emoji === emoji ? { ...r, own: !mine, count: r.count + (mine ? -1 : 1) } : r)).filter((r) => r.count > 0)
        : [...x.reactions, { emoji, count: 1, own: true }]
      return { ...x, reactions }
    }))
    try {
      await api.talkReact(token, m.id, emoji, !mine)
    } catch (e) {
      notify(`Реакция не поставлена: ${errorText(e)}`)
    }
  }

  async function remove(m: TalkMessage) {
    if (!token || !window.confirm('Удалить сообщение? Его удалят у всех участников.')) return
    try {
      await api.talkDelete(token, m.id)
      setMessages((list) => list.map((x) => (x.id === m.id ? { ...x, deleted: true, text: 'Сообщение удалено', reactions: [], editText: null, files: [] } : x)))
    } catch (e) {
      notify(`Не удалось удалить: ${errorText(e)}`)
    }
  }

  function startEdit(m: TalkMessage) {
    setEditing(m)
    setReplyTo(null)
    setDraft(m.editText ?? m.text)
    requestAnimationFrame(() => inputRef.current?.focus())
  }

  async function loadOlder() {
    const first = messages[0]
    const el = listRef.current
    if (!token || !first || olderBusy) return
    setOlderBusy(true)
    const fromBottom = el ? el.scrollHeight - el.scrollTop : 0
    try {
      const chunk = await api.talkHistory(token, first.id)
      stickToBottom.current = false
      setMessages((list) => merge(list, chunk.messages))
      setHasOlder(chunk.more && chunk.messages.length > 0)
      // Лента остаётся на том же месте, ранние сообщения появляются выше.
      requestAnimationFrame(() => { if (el) el.scrollTop = el.scrollHeight - fromBottom })
    } catch (e) {
      notify(`Не удалось загрузить ранние сообщения: ${errorText(e)}`)
    } finally {
      setOlderBusy(false)
    }
  }

  useEffect(() => { setSearchOpen(false); setSearchQ(''); setHits(null) }, [token])
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === 'f' && token) { e.preventDefault(); setSearchOpen(true) }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [token])
  useEffect(() => {
    const q = searchQ.trim()
    if (!token || !searchOpen || q.length < 2) { setHits(null); setSearchBusy(false); return }
    let cancelled = false
    setSearchBusy(true)
    const t = window.setTimeout(() => {
      api.talkSearch(token, q)
        .then((list) => { if (!cancelled) setHits(list) })
        .catch(() => {
          // Сервер не умеет искать — ищем в загруженных сообщениях.
          if (cancelled) return
          const low = q.toLowerCase()
          setHits(messages.filter((m) => !m.system && !m.deleted && m.text.toLowerCase().includes(low)).reverse()
            .map((m) => ({ messageId: m.id, author: m.author, text: m.text, timestamp: m.timestamp })))
        })
        .finally(() => { if (!cancelled) setSearchBusy(false) })
    }, TYPING_PAUSE_MS)
    return () => { cancelled = true; window.clearTimeout(t) }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchQ, searchOpen, token])

  // Переход из общего поиска: открыть беседу и, когда история загрузится, — сообщение.
  const pendingJump = useRef<{ token: string; id: number | null } | null>(null)
  useEffect(() => {
    // Каждый переход — один раз: при повторном открытии раздела он не повторяется.
    if (!focus || focus.n <= talkFocusSeen) return
    talkFocusSeen = focus.n
    pendingJump.current = { token: focus.token, id: focus.messageId }
    setToken(focus.token)
  }, [focus])
  useEffect(() => {
    const p = pendingJump.current
    if (!p || p.token !== token || loading || historyFor.current !== token) return
    pendingJump.current = null
    if (p.id) void jumpTo(p.id)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [token, loading, messages, focus])

  /** Перейти к сообщению: догрузить историю до него, прокрутить и подсветить. */
  async function jumpTo(id: number) {
    if (!token) return
    let list = messages
    for (let i = 0; i < 30 && !list.some((m) => m.id === id) && list.length > 0 && list[0].id > id; i++) {
      const chunk = await api.talkHistory(token, list[0].id).catch(() => null)
      if (!chunk || chunk.messages.length === 0) break
      list = merge(list, chunk.messages)
      setHasOlder(chunk.more)
    }
    stickToBottom.current = false
    setMessages(list)
    setFlash(id)
    requestAnimationFrame(() => requestAnimationFrame(() => {
      const el = listRef.current?.querySelector(`.talk-msg[data-id="${id}"]`)
      if (el) el.scrollIntoView({ block: 'center' })
      else notify('Сообщение не найдено в истории беседы')
    }))
    window.setTimeout(() => setFlash((f) => (f === id ? null : f)), 2500)
  }

  /** Готовая запись с микрофона — голосовым сообщением (как в Talk). */
  async function sendVoice(blob: Blob, ext: string) {
    if (!token || sendingRef.current) return
    const room = token
    sendingRef.current = true
    setSending(true)
    try {
      const buf = new Uint8Array(await blob.arrayBuffer())
      let bin = ''
      for (let i = 0; i < buf.length; i += 0x8000) bin += String.fromCharCode(...buf.subarray(i, i + 0x8000))
      const stamp = new Date().toLocaleString('ru-RU').replace(/[^\d]+/g, '-').replace(/-$/, '')
      const picked = await api.attachData(`Голосовое ${stamp}.${ext}`, btoa(bin))
      await api.talkSendFile(room, picked.id, '', replyTo?.id ?? null, { voice: true, silent })
      setReplyTo(null)
      setSilent(false)
    } catch (e) {
      notify(`Голосовое не отправлено: ${errorText(e)}`)
    } finally {
      sendingRef.current = false
      setSending(false)
    }
  }

  async function startCall(r: TalkRoom, browser = false) {
    try {
      if (browser) await api.ncOpen(`call/${r.token}`)
      else await api.talkCallWindow(r.token)
    } catch (e) {
      notify(`Звонок не открыт: ${errorText(e)}`)
    }
  }

  function copyText(text: string, what = 'Текст скопирован') {
    navigator.clipboard.writeText(text).then(() => notify(what)).catch((e) => notify(`Не удалось скопировать: ${errorText(e)}`))
  }

  // --- сотрудник: личный чат, профиль, доступность, письмо
  const person: PersonActions = {
    chat: (userId) => {
      api.talkCreate([userId], null).then(async (t) => { await loadRooms(); setToken(t); setProfileFor(null) }).catch((e) => notify(`Не удалось открыть чат: ${errorText(e)}`))
    },
    profile: (id, name, busy = false) => setProfileFor({ id, name, busy }),
    write: (userId) => {
      api.ncUserProfile(userId).then((p) => {
        if (p.email && onWrite) onWrite(p.name && p.name !== p.email ? `${p.name} <${p.email}>` : p.email)
        else notify('В профиле сотрудника не открыт адрес почты')
      }).catch((e) => notify(`Не удалось узнать адрес: ${errorText(e)}`))
    },
  }

  // --- закреплённые сообщения беседы (полоска вверху, как в Talk)
  const roomNow = rooms?.find((r) => r.token === token)
  const pinKey = roomNow ? `${roomNow.token}:${roomNow.lastPinnedId}` : ''
  useEffect(() => {
    if (!roomNow || !roomNow.lastPinnedId) { setPinned([]); return }
    let alive = true
    api.talkShared(roomNow.token, 'pinned').then((l) => { if (alive) setPinned(l) }).catch(() => { if (alive) setPinned([]) })
    return () => { alive = false }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pinKey])

  async function pin(m: TalkMessage, until: number) {
    if (!token) return
    await api.talkPin(token, m.id, until)
    notify(until ? `Сообщение закреплено до ${new Date(until * 1000).toLocaleString('ru-RU', { day: 'numeric', month: 'long', hour: '2-digit', minute: '2-digit' })}.` : 'Сообщение закреплено.')
    await loadRooms()
  }
  async function unpin(m: TalkMessage, onlyMe: boolean) {
    if (!token) return
    try {
      await api.talkUnpin(token, m.id, onlyMe)
      notify(onlyMe ? 'Закреплённое скрыто у вас.' : 'Сообщение откреплено.')
      await loadRooms()
    } catch (e) { notify(`Не удалось: ${errorText(e)}`) }
  }

  /** Меню сообщения (правая кнопка мыши или «⋯»). */
  function messageMenu(m: TalkMessage, r: TalkRoom): MenuItem[] {
    if (m.system) return []
    const canWrite = !r.readOnly && !m.deleted
    const sel = window.getSelection()?.toString() ?? ''
    const items: MenuItem[] = []
    if (canWrite) {
      items.push({ custom: <>{QUICK_REACTIONS.map((e) => <button key={e} type="button" aria-label={`Реакция ${e}`} onClick={() => void react(m, e)}>{e}</button>)}<button type="button" aria-label="Другая реакция" title="Все эмодзи" onClick={() => { setReactFor(m.id); setReactFull(true) }}>＋</button></> })
      items.push({ label: 'Ответить', icon: <CornerUpLeft size={15} />, onClick: () => { setReplyTo(m); setEditing(null); inputRef.current?.focus() } })
    }
    if (sel.trim()) items.push({ label: 'Копировать выделенное', icon: <Copy size={15} />, onClick: () => copyText(sel) })
    if (m.text && !m.deleted) items.push({ label: 'Копировать текст', icon: <Copy size={15} />, onClick: () => copyText(m.text) })
    if (!m.deleted) items.push({ label: 'Переслать…', icon: <Forward size={15} />, onClick: () => setForwarding(m) })
    if (canWrite) items.push({ label: 'Создать карточку Deck…', icon: <KanbanSquare size={15} />, onClick: () => setCardFrom(m) })
    if (!m.deleted) {
      items.push({ separator: true })
      // Закреплено ли — по списку закреплённых беседы (в ленте метка обновляется не сразу).
      const isPinned = pinned.some((p) => p.id === m.id)
      if (r.moderator && !isPinned) items.push({ label: 'Закрепить…', icon: <Pin size={15} />, onClick: () => setPinFor(m) })
      if (r.moderator && isPinned) items.push({ label: 'Открепить', icon: <PinOff size={15} />, onClick: () => void unpin(m, false) })
      items.push({ label: 'Напомнить…', icon: <AlarmClock size={15} />, onClick: () => setRemindFor(m) })
    }
    if (!m.own && m.authorId && !m.system) {
      items.push({ separator: true })
      items.push(...personMenu(m.authorId, m.author, person, { inDirect: r.kind === 1 }))
    }
    const f = m.files[0]
    if (f && !m.deleted) {
      items.push({ separator: true })
      items.push({ label: m.files.length > 1 ? `Открыть «${f.name}»` : 'Открыть файл', icon: <ExternalLink size={15} />, onClick: () => openFile(f) })
      if (f.path) items.push({ label: 'Сохранить файл на компьютер…', icon: <Download size={15} />, onClick: () => void saveFile(f) })
    }
    if (canWrite && (m.editText !== null || m.own)) items.push({ separator: true })
    if (canWrite && m.editText !== null) items.push({ label: 'Изменить', icon: <Pencil size={15} />, onClick: () => startEdit(m) })
    if (canWrite && m.own) items.push({ label: 'Удалить', icon: <Trash2 size={15} />, danger: true, onClick: () => void remove(m) })
    return items
  }

  async function manageRoom(r: TalkRoom, action: string, value: Record<string, unknown>, done: string) {
    try {
      await api.talkManage(r.token, action, value)
      notify(done)
      await loadRooms()
    } catch (e) {
      notify(`Не удалось: ${errorText(e)}`)
    }
  }

  function roomGone(r: TalkRoom) {
    drafts.delete(r.token)
    if (token === r.token) { lastRoom = null; setToken(null) }
    void loadRooms()
  }

  /** Меню беседы в списке. */
  function roomMenu(r: TalkRoom): MenuItem[] {
    const muted = r.notifyLevel === 3
    return [
      { label: r.favorite ? 'Убрать из избранного' : 'В избранное', icon: <Star size={15} />, onClick: () => void toggleFavorite(r) },
      r.unread > 0
        ? { label: 'Отметить прочитанной', icon: <MailOpen size={15} />, onClick: () => void manageRoom(r, 'read', {}, `«${r.name}» — прочитано`) }
        : { label: 'Отметить непрочитанной', icon: <MailOpen size={15} />, onClick: () => void manageRoom(r, 'unread', {}, `«${r.name}» — непрочитано`) },
      { label: muted ? 'Включить уведомления' : 'Не уведомлять', icon: muted ? <Bell size={15} /> : <BellOff size={15} />, onClick: () => void manageRoom(r, 'notify', { level: muted ? 0 : 3 }, muted ? 'Уведомления включены' : 'Уведомления беседы отключены') },
      { separator: true },
      { label: 'Участники', icon: <Users size={15} />, onClick: () => { setToken(r.token); setShowPeople(true) } },
      { label: 'Настройки беседы…', icon: <Settings2 size={15} />, onClick: () => setSettingsFor(r) },
      { label: 'Обои…', icon: <WallpaperIcon size={15} />, onClick: () => setWallpaperRoom(r) },
      { label: 'Открыть в Nextcloud', icon: <ExternalLink size={15} />, onClick: () => void api.ncOpen(`call/${r.token}`) },
      ...(r.canLeave ? [{ separator: true }, { label: 'Выйти из беседы', icon: <LogOut size={15} />, danger: true, onClick: () => { if (window.confirm(`Выйти из беседы «${r.name}»?`)) void api.talkManage(r.token, 'leave').then(() => { notify(`Вы вышли из «${r.name}»`); roomGone(r) }).catch((e) => notify(`Не удалось: ${errorText(e)}`)) } }] : []),
    ]
  }

  /** Вложения, вставленные из буфера обмена: в списке файлов, картинки — с превью. */
  async function addPasted(list: PickedFile[]) {
    if (list.length === 0) return
    setFiles((f) => [...f, ...list])
    for (const p of list) {
      if (/\.(png|jpe?g|gif|webp|bmp)$/i.test(p.name)) {
        const url = await api.pickedPreview(p.id).catch(() => null)
        if (url) setPreviews((x) => ({ ...x, [p.id]: url }))
      }
    }
    notify(list.length === 1 ? `Вставлено: ${list[0].name}` : `Вставлено файлов: ${list.length}`)
  }

  /**
   * Ctrl+V (и «Вставить» в меню): картинки и файлы из буфера — вложениями к сообщению.
   * Что WebView отдаёт сам (снимок экрана в Windows) — берём из события; файлы, скопированные
   * в проводнике, и картинки, которых WebView не видит, — из буфера обмена системы.
   */
  async function onPaste(e: React.ClipboardEvent<HTMLTextAreaElement>) {
    if (editing) return
    const dt = e.clipboardData
    const direct = [...dt.files]
    const types = [...dt.types]
    const text = dt.getData('text/plain')
    const uris = dt.getData('text/uri-list')
    const fileUris = /^file:\/\//im.test(uris) || /^(\/|file:\/\/)\S+$/m.test(text) && types.includes('text/uri-list')
    if (direct.length === 0 && text && !fileUris) return // обычный текст — вставляется как есть
    e.preventDefault()
    const target = e.currentTarget
    try {
      if (direct.length > 0) {
        const picked: PickedFile[] = []
        for (const f of direct) {
          const buf = new Uint8Array(await f.arrayBuffer())
          let bin = ''
          for (let i = 0; i < buf.length; i += 0x8000) bin += String.fromCharCode(...buf.subarray(i, i + 0x8000))
          const ext = (f.type.split('/')[1] || 'png').replace('jpeg', 'jpg').replace(/[^a-z0-9]/gi, '')
          const stamp = new Date().toLocaleString('ru-RU').replace(/[^\d]+/g, '-').replace(/-$/, '')
          const name = f.name && f.name !== 'image.png' ? f.name : `Снимок ${stamp}.${ext}`
          picked.push(await api.attachData(name, btoa(bin)))
        }
        await addPasted(picked)
        return
      }
      const fromOs = await api.clipboardPasteFiles()
      if (fromOs.length > 0) { await addPasted(fromOs); return }
      // В буфере всё-таки текст (например, путь к файлу, которого уже нет) — вставить его.
      if (text) { target.focus(); document.execCommand('insertText', false, text) }
    } catch (err) {
      notify(`Не вставлено: ${errorText(err)}`)
    }
  }

  const room = rooms?.find((r) => r.token === token)
  const wpChoice = room ? wallpaperFor(room.token) : null
  const wp = wallpaperStyle(wpChoice, dark)
  const wpData = wpChoice ? (wpChoice.kind === 'preset' ? wpChoice.id : 'image') : undefined
  const readMark = Math.max(readUpTo, room?.lastCommonRead ?? 0)

  async function toggleFavorite(r: TalkRoom) {
    const on = !r.favorite
    // Сразу наверх (или обратно), сервер подтвердит при следующем обновлении списка.
    setRooms((list) => list && sortRooms(list.map((x) => (x.token === r.token ? { ...x, favorite: on } : x))))
    try {
      await api.talkFavorite(r.token, on)
    } catch (e) {
      setRooms((list) => list && sortRooms(list.map((x) => (x.token === r.token ? { ...x, favorite: !on } : x))))
      notify(`Не удалось ${on ? 'добавить в избранное' : 'убрать из избранного'}: ${errorText(e)}`)
    }
  }
  const visibleRooms = useMemo(() => {
    const q = query.trim().toLowerCase()
    return (rooms ?? []).filter((r) => !q || r.name.toLowerCase().includes(q))
  }, [rooms, query])

  // Поиск сотрудников Nextcloud прямо из списка бесед — не только тех, с кем уже переписывались.
  const [people, setPeople] = useState<NcUser[]>([])
  const [peopleBusy, setPeopleBusy] = useState(false)
  useEffect(() => {
    const q = query.trim()
    if (q.length < 2) { setPeople([]); return }
    let cancelled = false
    const t = setTimeout(() => {
      setPeopleBusy(true)
      api.ncSearchUsers(q)
        .then((list) => { if (!cancelled) setPeople(list) })
        .catch(() => { if (!cancelled) setPeople([]) })
        .finally(() => { if (!cancelled) setPeopleBusy(false) })
    }, TYPING_PAUSE_MS)
    return () => { cancelled = true; clearTimeout(t) }
  }, [query])
  const peers = new Set((rooms ?? []).filter((r) => r.kind === 1 && r.peer).map((r) => r.peer))
  async function chatWith(u: NcUser) {
    try {
      const tok = await api.talkCreate([u.id], null)
      setQuery('')
      await loadRooms()
      setToken(tok)
    } catch (e) {
      setError(errorText(e))
    }
  }

  return (
    <div className="module-view talk-view">
      <aside className="message-column talk-rooms">
        <div className="list-toolbar">
          <label className="search-box"><Search size={17} /><input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Беседы и сотрудники" /></label>
          <button className="icon-button soft" aria-label="Новая беседа" title="Новая беседа" onClick={() => setNewChat(true)}><MessageSquarePlus size={17} /></button>
          <button className="icon-button soft" aria-label="Обновить" onClick={() => void loadRooms()}><RefreshCw size={17} /></button>
        </div>
        <div className="message-list">
          {rooms === null && !error && <SkeletonRows variant="room" count={7} label="Загрузка бесед…" />}
          {visibleRooms.map((r) => (
            <button key={r.token} className={`message-row room-row ${r.token === token ? 'selected' : ''} ${r.unread ? 'unread' : ''}`} onClick={() => setToken(r.token)}
              onContextMenu={(e) => openContextMenu(e, roomMenu(r))}>
              {r.kind === 1 && r.peer
                ? <Avatar id={r.peer} name={r.name} />
                : <Avatar kind="room" id={r.token} name={r.name} fallback={r.kind === 1 ? undefined : <Users size={16} />} />}
              <div className="message-summary">
                <div className="message-topline"><span className="sender-name">{r.name}</span>{r.favorite && <Star size={13} className="room-favorite" aria-label="Избранное" fill="currentColor" />}{r.notifyLevel === 3 && <BellOff size={12} className="room-muted" aria-label="Без уведомлений" />}{r.unread > 0 && <b className={`folder-count ${r.unreadMention ? 'mention' : ''}`}>{r.unread}</b>}</div>
                <div className="message-preview">{r.lastMessage ?? ''}</div>
              </div>
            </button>
          ))}
          {query.trim().length >= 2 && <div className="talk-people">
            <div className="message-list-header"><span>Сотрудники</span>{peopleBusy && <span>поиск…</span>}</div>
            {!peopleBusy && people.length === 0 && <div className="empty-state compact"><span>Никого не найдено в Nextcloud</span></div>}
            {people.filter((u) => !visibleRooms.some((r) => r.kind === 1 && r.peer === u.id)).map((u) => (
              <button key={u.id} className="message-row room-row person-row" title="Написать" onClick={() => void chatWith(u)}>
                <Avatar id={u.id} name={u.label} />
                <div className="message-summary">
                  <div className="message-topline"><span className="sender-name">{u.label}</span></div>
                  <div className="message-preview">{peers.has(u.id) ? 'Открыть беседу' : (u.sub || 'Начать беседу')}</div>
                </div>
              </button>
            ))}
          </div>}
        </div>
      </aside>
      <Splitter id="talk-rooms" min={220} max={640} label="Ширина списка бесед" />

      <main className={`talk-main ${wp ? 'has-wallpaper' : ''}`} data-wallpaper={wpData}>
        {wp && <div className="talk-wallpaper" style={wp.layer} aria-hidden="true" />}
        {wp?.shade && <div className="talk-wallpaper-shade" style={wp.shade} aria-hidden="true" />}
        {room ? (
          <>
            <div className="calendar-toolbar">
              <strong className="calendar-title">{room.name}</strong>
              <button className={`icon-button soft talk-favorite ${room.favorite ? 'on' : ''}`} aria-pressed={room.favorite}
                aria-label={room.favorite ? 'Убрать из избранного' : 'В избранное'} title={room.favorite ? 'Убрать из избранного' : 'В избранное — беседа будет выше остальных'}
                onClick={() => void toggleFavorite(room)}><Star size={17} fill={room.favorite ? 'currentColor' : 'none'} /></button>
              <div className="reader-toolbar-spacer" />
              {room.messageExpiration > 0 && <span className="talk-expiry-badge" title="Сообщения в этой беседе исчезают"><Timer size={13} /> {expiryLabel(room.messageExpiration)}</span>}
              <button className="secondary-button talk-call" title="Звонок и демонстрация экрана — в окне программы" onClick={() => void startCall(room)}
                onContextMenu={(e) => openContextMenu(e, [{ label: 'Звонок в окне программы', icon: <Phone size={15} />, onClick: () => void startCall(room) }, { label: 'Звонок в браузере', icon: <ExternalLink size={15} />, onClick: () => void startCall(room, true) }])}><Phone size={15} /> Звонок</button>
              <button className={`icon-button soft talk-media-toggle ${showMedia ? 'active' : ''}`} aria-pressed={showMedia} aria-label="Медиа и файлы" title="Медиа и файлы беседы" onClick={() => { setShowMedia(!showMedia); setShowPeople(false) }}><Images size={17} /></button>
              <button className={`icon-button soft talk-search-toggle ${searchOpen ? 'active' : ''}`} aria-pressed={searchOpen} aria-label="Поиск по сообщениям" title="Поиск по сообщениям (Ctrl+F)" onClick={() => setSearchOpen(!searchOpen)}><Search size={17} /></button>
              {room.kind === 1 && room.peer && <button className="icon-button soft talk-profile-toggle" aria-label="Профиль собеседника" title="Профиль, доступность, письмо" onClick={(e) => openContextMenu(e, personMenu(room.peer as string, room.name, person, { inDirect: true }))}><UserRound size={17} /></button>}
              <button className={`icon-button soft talk-people-toggle ${showPeople ? 'active' : ''}`} aria-pressed={showPeople} aria-label="Участники" title="Участники беседы" onClick={() => { setShowPeople(!showPeople); setShowMedia(false) }}><Users size={17} /></button>
              <button className="icon-button soft" aria-label="Меню беседы" title="Ещё" onClick={(e) => openContextMenu(e, roomMenu(room))}><MoreHorizontal size={17} /></button>
            </div>
            {searchOpen && (
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
            )}
            {error && <div className="inline-warning" role="alert">Talk: {error}</div>}
            {pinned.length > 0 && room.lastPinnedId > 0 && room.hiddenPinnedId !== room.lastPinnedId && (
              <div className="talk-pinned" role="region" aria-label="Закреплённое сообщение">
                <Pin size={15} />
                <button type="button" className="talk-pinned-text" title="Перейти к сообщению" onClick={() => void jumpTo(pinned[0].id)}>
                  <b>{pinned[0].author}</b> {pinned[0].text || pinned[0].files[0]?.name}
                  {pinned[0].pinnedUntil > 0 && <small> · до {new Date(pinned[0].pinnedUntil * 1000).toLocaleString('ru-RU', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })}</small>}
                </button>
                {pinned.length > 1 && <button type="button" className="text-button" onClick={(e) => openContextMenu(e, pinned.map((m) => ({ label: `${m.author}: ${(m.text || m.files[0]?.name || '').slice(0, 60)}`, icon: <Pin size={15} />, onClick: () => void jumpTo(m.id) })))}>ещё {pinned.length - 1}</button>}
                <button type="button" className="icon-button soft" aria-label="Скрыть закреплённое" title="Скрыть"
                  onClick={(e) => room.moderator
                    ? openContextMenu(e, [{ label: 'Скрыть у себя', icon: <EyeOff size={15} />, onClick: () => void unpin(pinned[0], true) }, { label: 'Открепить для всех', icon: <PinOff size={15} />, onClick: () => void unpin(pinned[0], false) }])
                    : void unpin(pinned[0], true)}><X size={15} /></button>
              </div>
            )}
            <div className="talk-messages" ref={listRef}
              onScroll={(e) => { const el = e.currentTarget; stickToBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80 }}>
              {loading && messages.length === 0 && <SkeletonChat />}
              {hasOlder && !loading && <button className="text-button talk-older" disabled={olderBusy} onClick={() => void loadOlder()}>{olderBusy ? 'Загрузка…' : 'Показать более ранние сообщения'}</button>}
              {messages.map((m) => m.system ? (
                <div key={m.id} className="talk-system">{m.text} · {timeOf(m.timestamp)}</div>
              ) : (
                <div key={m.id} className={`talk-msg ${m.own ? 'own' : ''} ${m.deleted ? 'deleted' : ''} ${flash === m.id ? 'flash' : ''}`} data-id={m.id}
                  onContextMenu={(e) => { const items = messageMenu(m, room); if (items.length) openContextMenu(e, items) }}>
                  {!m.own && <span className="talk-author-avatar" role="button" tabIndex={-1} title={m.author} onClick={(e) => m.authorId && openContextMenu(e, personMenu(m.authorId, m.author, person, { inDirect: room.kind === 1 }))}><Avatar id={m.authorId} name={m.author} /></span>}
                  <div className="talk-bubble">
                    {!m.own && <button type="button" className="talk-author" onClick={(e) => m.authorId && openContextMenu(e, personMenu(m.authorId, m.author, person, { inDirect: room.kind === 1 }))}>{m.author}</button>}
                    {m.parentText && !m.deleted && <div className="talk-quote"><b>{m.parentAuthor}</b> {m.parentText}</div>}
                    {m.text && (m.deleted ? <div className="talk-text talk-deleted">{m.text}</div> : <TalkText text={m.text} />)}
                    {m.poll && !m.deleted && <PollCard token={room.token} id={m.poll.id} question={m.poll.question} notify={notify} />}
                    {m.voice && !m.deleted && m.files[0] && <VoicePlayer file={m.files[0]} load={(f) => api.talkFileData(f.path as string)} />}
                    {!m.deleted && m.files.length === 0 && !m.poll && firstUrl(m.text) && <LinkCard url={firstUrl(m.text) as string} />}
                    {m.files.length > 0 && !m.voice && (
                      <div className="talk-files">
                        {m.files.map((f) => (isImage(f) && f.id && (f.preview || f.path)) ? (
                          <div key={f.id} className="talk-image">
                            <button className="talk-image-open" aria-label={`Открыть ${f.name}`} title="Открыть" onClick={() => openFile(f)}><TalkThumb file={f} /></button>
                            <div className="talk-image-bar">
                              <span title={f.name}>{f.name}</span>{f.size > 0 && <small>{formatSize(f.size)}</small>}
                              {f.path && <button className="icon-button soft" aria-label={`Скачать ${f.name}`} title="Сохранить на компьютер" onClick={() => void saveFile(f)}><Download size={14} /></button>}
                              <button className="icon-button soft" aria-label={`Открыть ${f.name} в Nextcloud`} title="Открыть в Nextcloud" onClick={() => void api.ncOpen(`f/${f.id}`)}><ExternalLink size={14} /></button>
                            </div>
                          </div>
                        ) : (
                          <div key={f.id || f.name} className="talk-file-row">
                            <button className="talk-file" title={viewable(f) ? 'Открыть' : 'Открыть в Nextcloud'} onClick={() => openFile(f)}>
                              <FileText size={15} /><span>{f.name}</span>{f.size > 0 && <small>{formatSize(f.size)}</small>}
                            </button>
                            {f.path && <button className="icon-button soft" aria-label={`Скачать ${f.name}`} title="Сохранить на компьютер" onClick={() => void saveFile(f)}><Download size={14} /></button>}
                          </div>
                        ))}
                      </div>
                    )}
                    {m.reactions.length > 0 && (
                      <div className="talk-reactions">
                        {m.reactions.map((r) => (
                          <button key={r.emoji} className={`talk-reaction ${r.own ? 'own' : ''}`} disabled={room.readOnly} title={r.own ? 'Убрать реакцию' : 'Поставить реакцию'} onClick={() => void react(m, r.emoji)}>
                            {r.emoji} <span>{r.count}</span>
                          </button>
                        ))}
                      </div>
                    )}
                    <div className="talk-time">{m.expires > 0 && <Timer size={11} className="talk-expires" aria-label="Исчезающее сообщение" />}{m.edited && !m.deleted && <span className="talk-edited">изменено</span>}{timeOf(m.timestamp)}
                      {m.own && !m.deleted && (m.id <= readMark
                        ? <CheckCheck size={14} className="talk-tick read" aria-label="Прочитано" />
                        : <Check size={14} className="talk-tick" aria-label="Отправлено" />)}</div>
                    {!room.readOnly && !m.deleted && (
                      <div className="talk-actions">
                        <button aria-label="Реакция" title="Реакция" onClick={() => setReactFor(reactFor === m.id ? null : m.id)}><SmilePlus size={14} /></button>
                        <button aria-label="Ответить" title="Ответить" onClick={() => { setReplyTo(m); setEditing(null); inputRef.current?.focus() }}><CornerUpLeft size={14} /></button>
                        <button aria-label="Карточка Deck" title="Создать карточку Deck" onClick={() => setCardFrom(m)}><KanbanSquare size={14} /></button>
                        {m.editText !== null && <button aria-label="Изменить" title="Изменить" onClick={() => startEdit(m)}><Pencil size={14} /></button>}
                        {m.own && <button aria-label="Удалить" title="Удалить" onClick={() => void remove(m)}><Trash2 size={14} /></button>}
                        <button aria-label="Ещё" title="Ещё действия" onClick={(e) => openContextMenu(e, messageMenu(m, room))}><MoreHorizontal size={14} /></button>
                      </div>
                    )}
                    {reactFor === m.id && (
                      reactFull
                        ? <EmojiPicker className="talk-react-full" label="Реакция" onPick={(e) => { setReactFor(null); void react(m, e) }} />
                        : <div className="talk-react-picker popover">
                          {QUICK_REACTIONS.map((e) => <button key={e} aria-label={`Реакция ${e}`} onClick={() => void react(m, e)}>{e}</button>)}
                          <button aria-label="Другая реакция" title="Все эмодзи" onClick={() => setReactFull(true)}>＋</button>
                        </div>
                    )}
                  </div>
                </div>
              ))}
              {!loading && messages.length === 0 && <div className="empty-state"><MessageCircle size={30} /><strong>Сообщений пока нет</strong></div>}
            </div>
            {room.readOnly ? <div className="muted talk-readonly">Беседа только для чтения</div> : (
              <form className="talk-compose" onSubmit={(e) => { e.preventDefault(); void send() }}>
                {files.length > 0 && (
                  <div className="talk-pending">
                    {files.map((f) => (
                      <span key={f.id} className={`talk-file ${previews[f.id] ? 'with-preview' : ''}`}>{previews[f.id] ? <img className="talk-paste-preview" src={previews[f.id]} alt="" /> : <Paperclip size={14} />}<span>{f.name}</span><small>{formatSize(f.size)}</small>
                        {!sending && <button type="button" aria-label={`Убрать ${f.name}`} onClick={() => dropFile(f.id)}><X size={13} /></button>}
                      </span>
                    ))}
                  </div>
                )}
                {replyTo && <div className="talk-quote replying"><b>{replyTo.author}</b> {replyTo.text.slice(0, 120)}<button type="button" aria-label="Отменить ответ" onClick={() => setReplyTo(null)}><X size={13} /></button></div>}
                {editing && <div className="talk-quote replying editing"><Pencil size={13} /> <b>Изменение сообщения</b><button type="button" aria-label="Отменить изменение" onClick={cancelEdit}><X size={13} /></button></div>}
                <div className="talk-input-row">
                  {!editing && <button type="button" className="icon-button soft talk-attach" aria-label="Прикрепить файл" title="Прикрепить файл" disabled={sending} onClick={() => void pickFiles()}><Paperclip size={18} /></button>}
                  <div className="talk-input-wrap">
                    {mentions.length > 0 && (
                      <div className="talk-mentions popover">
                        {mentions.map((mn, i) => (
                          <button type="button" key={mn.insert} className={i === mentionSel ? 'active' : ''} onMouseDown={(e) => { e.preventDefault(); pickMention(mn) }}>
                            {mn.source === 'calls' ? <span className="sender-avatar"><Users size={13} /></span> : <Avatar id={mn.source === 'users' ? mn.insert.replace(/^@"?|"$/g, '') : null} name={mn.label} />}{mn.source === 'calls' ? 'Все участники' : mn.label}
                          </button>
                        ))}
                      </div>
                    )}
                    {emojiOpen && <EmojiPicker onPick={insertEmoji} />}
                    <textarea
                      ref={inputRef}
                      rows={2}
                      value={draft}
                      onChange={(e) => onDraftChange(e.target.value, e.target.selectionStart)}
                      onPaste={(e) => void onPaste(e)}
                      onKeyDown={(e) => {
                        if (mentions.length > 0) {
                          if (e.key === 'ArrowDown') { e.preventDefault(); setMentionSel((i) => (i + 1) % mentions.length); return }
                          if (e.key === 'ArrowUp') { e.preventDefault(); setMentionSel((i) => (i - 1 + mentions.length) % mentions.length); return }
                          if (e.key === 'Enter' || e.key === 'Tab') { e.preventDefault(); pickMention(mentions[mentionSel]); return }
                          if (e.key === 'Escape') { setMentions([]); return }
                        }
                        if (e.key === 'Escape' && editing) { cancelEdit(); return }
                        if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); void send() }
                      }}
                      placeholder={editing ? 'Новый текст… (Enter — сохранить, Esc — отмена)' : files.length ? 'Подпись к файлу… (Enter — отправить)' : 'Сообщение… (@ — упомянуть, Enter — отправить, Shift+Enter — новая строка)'}
                    />
                  </div>
                  <button type="button" className={`icon-button soft talk-emoji ${emojiOpen ? 'active' : ''}`} aria-label="Эмодзи" title="Эмодзи" onClick={() => setEmojiOpen((v) => !v)}><Smile size={18} /></button>
                  {!editing && <button type="button" className="icon-button soft talk-poll-new" aria-label="Опрос" title="Создать опрос" disabled={sending} onClick={() => setPollOpen(true)}><BarChart3 size={18} /></button>}
                  {!editing && <button type="button" className={`icon-button soft talk-silent ${silent ? 'active' : ''}`} aria-pressed={silent} aria-label="Без звука" title={silent ? 'Отправится без звука уведомления — нажмите, чтобы отменить' : 'Отправить без звука уведомления'} onClick={() => setSilent(!silent)}><BellOff size={18} /></button>}
                  {/* Кнопка записи на месте всегда (чтобы кнопки не прыгали); активна, когда поле пустое. */}
                  {!editing && <VoiceRecorder disabled={sending || Boolean(draft.trim()) || files.length > 0} notify={notify} onRecorded={(b, ext) => void sendVoice(b, ext)} />}
                  <button className="primary-button" type="submit" disabled={sending || (!draft.trim() && files.length === 0)} aria-label={editing ? 'Сохранить' : 'Отправить'}>{sending && files.length > 0 ? 'Загрузка…' : editing ? 'Сохранить' : <Send size={16} />}</button>
                </div>
              </form>
            )}
          </>
        ) : (
          <div className="empty-reader">{error ? <><strong>Talk недоступен</strong><span>{error}</span></> : <><MessageCircle size={42} /><strong>Выберите беседу</strong></>}</div>
        )}
      </main>
      {room && showMedia && <Splitter id="talk-people" side="next" min={220} max={520} label="Ширина панели медиа и файлов" />}
      {room && showMedia && <MediaPanel token={room.token} messages={messages} onClose={() => setShowMedia(false)} thumb={(f) => <TalkThumb file={f} />}
        loadVoice={(f) => api.talkFileData(f.path as string)}
        onOpen={(f, list) => { const v = list.filter(viewable); const i = v.indexOf(f); if (i >= 0) setMediaView({ files: v, index: i }); else void api.ncOpen(`f/${f.id}`) }} />}
      {wallpaperRoom !== undefined && <WallpaperDialog room={wallpaperRoom} dark={dark} onClose={() => setWallpaperRoom(undefined)} />}
      {pollOpen && room && <PollDialog token={room.token} notify={notify} onClose={() => setPollOpen(false)} />}
      {mediaView && mediaView.files[mediaView.index] && (
        <AttachmentViewer folder="" uid={0} items={mediaView.files.map((f, i) => ({ index: i, name: f.name, contentType: f.mimetype, size: f.size }))} index={mediaView.index}
          onIndex={(i) => setMediaView((v) => v && { ...v, index: i })} onClose={() => setMediaView(null)}
          load={(a) => { const f = mediaView.files[a.index]; return f?.path ? cached(contents, f.path, 20, () => api.talkFileData(f.path as string)) : Promise.reject(new Error('Файл недоступен')) }}
          onSave={(i) => { const f = mediaView.files[i]; if (f) void saveFile(f) }} />
      )}
      {room && showPeople && <Splitter id="talk-people" side="next" min={220} max={520} label="Ширина списка участников" />}
      {room && showPeople && <RoomPeople room={room} notify={notify} person={person} onClose={() => setShowPeople(false)} onRoomChanged={() => void loadRooms()} onSettings={() => setSettingsFor(room)} />}
      {pinFor && <PinDialog message={pinFor} onPin={(u) => pin(pinFor, u)} onClose={() => setPinFor(null)} />}
      {remindFor && token && <ReminderDialog token={token} message={remindFor} notify={notify} onClose={() => setRemindFor(null)} />}
      {profileFor && <ProfileDialog userId={profileFor.id} name={profileFor.name} showBusy={profileFor.busy} onChat={() => person.chat(profileFor.id)}
        onWrite={(email, name) => { setProfileFor(null); onWrite?.(name && name !== email ? `${name} <${email}>` : email) }} onClose={() => setProfileFor(null)} />}

      {settingsFor && <RoomSettings room={rooms?.find((r) => r.token === settingsFor.token) ?? settingsFor} notify={notify} onClose={() => setSettingsFor(null)} onChanged={() => void loadRooms()} onGone={() => roomGone(settingsFor)} />}
      {forwarding && rooms && <ForwardDialog rooms={rooms} message={forwarding} notify={notify} onClose={() => setForwarding(null)} />}
      {cardFrom && room && <CardFromMessage message={cardFrom} room={room} onClose={() => setCardFrom(null)} onDone={(t) => { setCardFrom(null); notify(t) }} />}
      {newChat && <NewChat onClose={() => setNewChat(false)} onCreated={(t) => { setNewChat(false); void loadRooms().then(() => setToken(t)) }} />}
      {viewing !== null && viewItems[viewing] && (
        <AttachmentViewer folder="" uid={0} items={viewItems} index={viewing} onIndex={setViewing} onClose={() => setViewing(null)}
          load={viewLoad} onSave={(i) => { const f = viewFiles[i]; if (f) void saveFile(f) }} />
      )}
    </div>
  )
}

/** Карточка Deck из сообщения: текст, автор и ссылка на сообщение в Talk. */
function CardFromMessage({ message, room, onClose, onDone }: { message: TalkMessage; room: TalkRoom; onClose: () => void; onDone: (t: string) => void }) {
  const [boards, setBoards] = useState<DeckBoard[] | null>(null)
  const [boardId, setBoardId] = useState<number | null>(null)
  const [stacks, setStacks] = useState<DeckStack[]>([])
  const [stackId, setStackId] = useState<number | null>(null)
  const firstLine = message.text.split('\n')[0] || message.files[0]?.name || 'Сообщение из Talk'
  const [title, setTitle] = useState(firstLine.length > 80 ? `${firstLine.slice(0, 79)}…` : firstLine)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [base, setBase] = useState('')

  useEffect(() => {
    api.ncStatus().then((s) => setBase(s.url.replace(/\/?$/, '/'))).catch(() => undefined)
    api.deckBoards()
      .then((list) => {
        const editable = list.filter((b) => b.canEdit && !b.archived)
        setBoards(editable)
        let last: number | null = null
        try { last = Number(localStorage.getItem('rrcki-mail.deck-board')) || null } catch { /* нет */ }
        setBoardId(editable.find((b) => b.id === last)?.id ?? editable[0]?.id ?? null)
      })
      .catch((e) => setError(errorText(e)))
  }, [])

  useEffect(() => {
    if (boardId === null) return
    setStacks([])
    api.deckStacks(boardId).then((s) => { setStacks(s); setStackId(s[0]?.id ?? null) }).catch((e) => setError(errorText(e)))
  }, [boardId])

  async function create() {
    if (boardId === null || stackId === null || !title.trim()) return
    setBusy(true)
    setError('')
    const when = new Date(message.timestamp * 1000).toLocaleString('ru-RU')
    const link = base ? `${base}call/${room.token}#message_${message.id}` : ''
    const files = message.files.map((f) => `- ${f.name}${base && f.id ? ` (${base}f/${f.id})` : ''}`).join('\n')
    const description = [message.text, files, `— ${message.author}, ${when}, беседа «${room.name}»`, link].filter(Boolean).join('\n\n')
    try {
      await api.deckCreateCard(boardId, stackId, title, description)
      onDone(`Карточка создана: ${boards?.find((b) => b.id === boardId)?.title ?? 'Deck'} → ${stacks.find((s) => s.id === stackId)?.title ?? ''}`)
    } catch (e) {
      setError(errorText(e))
      setBusy(false)
    }
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget && !busy) onClose() }}>
      <div className="settings-modal talk-card-modal">
        <div className="compose-header"><div><strong>Карточка Deck из сообщения</strong><small>{message.author}</small></div><button className="icon-button" onClick={onClose} aria-label="Закрыть"><X size={18} /></button></div>
        <div className="talk-quote">{message.text.slice(0, 300) || message.files.map((f) => f.name).join(', ')}</div>
        <label className="login-field"><span>Доска</span>
          <select value={boardId ?? ''} onChange={(e) => setBoardId(Number(e.target.value))} disabled={!boards?.length}>
            {(boards ?? []).map((b) => <option key={b.id} value={b.id}>{b.title}</option>)}
          </select>
        </label>
        <label className="login-field"><span>Колонка</span>
          <select value={stackId ?? ''} onChange={(e) => setStackId(Number(e.target.value))} disabled={!stacks.length}>
            {stacks.map((s) => <option key={s.id} value={s.id}>{s.title}</option>)}
          </select>
        </label>
        <label className="login-field"><span>Название</span><input value={title} onChange={(e) => setTitle(e.target.value)} /></label>
        {boards && boards.length === 0 && <div className="muted">Нет досок, где вы можете создавать карточки.</div>}
        {error && <div className="login-error" role="alert">{error}</div>}
        <div className="settings-actions">
          <button className="text-button" onClick={onClose}>Отмена</button>
          <button className="primary-button" disabled={busy || boardId === null || stackId === null || !title.trim()} onClick={() => void create()}>{busy ? 'Создание…' : 'Создать карточку'}</button>
        </div>
      </div>
    </div>
  )
}

/** Новая беседа: один собеседник — личная, несколько или с названием — групповая. */
function NewChat({ onClose, onCreated }: { onClose: () => void; onCreated: (token: string) => void }) {
  const [q, setQ] = useState('')
  const [found, setFound] = useState<NcUser[]>([])
  const [picked, setPicked] = useState<NcUser[]>([])
  const [name, setName] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  useEffect(() => {
    if (q.trim().length < 2) { setFound([]); return }
    let cancelled = false
    const t = setTimeout(() => {
      api.ncSearchUsers(q).then((list) => { if (!cancelled) setFound(list) }).catch((e) => { if (!cancelled) setError(errorText(e)) })
    }, TYPING_PAUSE_MS)
    return () => { cancelled = true; clearTimeout(t) }
  }, [q])

  async function create() {
    setBusy(true)
    setError('')
    try {
      onCreated(await api.talkCreate(picked.map((u) => u.id), picked.length > 1 || name.trim() ? (name.trim() || picked.map((u) => u.label).join(', ')) : null))
    } catch (e) {
      setError(errorText(e))
      setBusy(false)
    }
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget && !busy) onClose() }}>
      <div className="settings-modal talk-new-modal">
        <div className="compose-header"><div><strong>Новая беседа</strong><small>Один собеседник — личная беседа, несколько — групповая</small></div><button className="icon-button" onClick={onClose} aria-label="Закрыть"><X size={18} /></button></div>
        {picked.length > 0 && (
          <div className="talk-pending">
            {picked.map((u) => <span key={u.id} className="talk-file"><span>{u.label}</span><button type="button" aria-label={`Убрать ${u.label}`} onClick={() => setPicked((l) => l.filter((x) => x.id !== u.id))}><X size={13} /></button></span>)}
          </div>
        )}
        <label className="search-box"><Search size={17} /><input autoFocus value={q} onChange={(e) => setQ(e.target.value)} placeholder="Имя или логин сотрудника" /></label>
        <div className="talk-user-list">
          {found.filter((u) => !picked.some((p) => p.id === u.id)).map((u) => (
            <button key={u.id} className="message-row" onClick={() => { setPicked((l) => [...l, u]); setQ('') }}>
              <Avatar id={u.id} name={u.label} />
              <div className="message-summary"><span className="sender-name">{u.label}</span>{u.sub && <span className="message-preview">{u.sub}</span>}</div>
            </button>
          ))}
        </div>
        {picked.length > 1 && <label className="login-field"><span>Название беседы</span><input value={name} onChange={(e) => setName(e.target.value)} placeholder={picked.map((u) => u.label).join(', ')} /></label>}
        {error && <div className="login-error" role="alert">{error}</div>}
        <div className="settings-actions">
          <button className="text-button" onClick={onClose}>Отмена</button>
          <button className="primary-button" disabled={busy || picked.length === 0} onClick={() => void create()}>{busy ? 'Создание…' : picked.length > 1 ? 'Создать беседу' : 'Написать'}</button>
        </div>
      </div>
    </div>
  )
}
