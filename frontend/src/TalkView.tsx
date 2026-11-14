import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import {
  Loader2, MessageCircle, EyeOff, Pin, PinOff, Search, X } from 'lucide-react'

import { api, errorText, onSessionReset, TYPING_PAUSE_MS } from './api'
import AttachmentViewer from './AttachmentViewer'
import type { TalkSearchHit } from './types'
import type { AttachmentInfo, NcUser, TalkFile, TalkMention, TalkMessage, TalkRoom } from './types'
import Splitter from './Splitter'
import { openContextMenu } from './PopupMenu'
import { ForwardDialog, RoomPeople, RoomSettings } from './TalkRoomPanel'
import WallpaperDialog from './WallpaperDialog'
import { useDarkTheme, useWallpaperVersion, wallpaperFor, wallpaperStyle } from './wallpaper'
import { MediaPanel, PollDialog } from './TalkExtras'
import { PinDialog, ProfileDialog, ReminderDialog, type PersonActions } from './TalkPeople'

import { CardFromMessage, NewChat } from './TalkDialogs'
import { useTalkAttachments } from './useTalkAttachments'
import { messageMenu as messageMenuFor, roomMenu as roomMenuFor } from './talkMenus'
import { cached, contents, highlight, merge, sortRooms, TalkThumb, viewable } from './talkUtils'
import TalkCompose from './TalkCompose'
import TalkMessageList from './TalkMessageList'
import TalkChatHeader from './TalkChatHeader'
import TalkRoomList from './TalkRoomList'

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
  const { files, setFiles, previews, pickFiles, dropFile, onPaste } = useTalkAttachments(editing !== null, notify, setError)

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
    } }

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

  const messageMenu = (m: TalkMessage, r: TalkRoom) => messageMenuFor(m, r, {
    react, setReactFor, setReactFull, setReplyTo, setEditing, inputRef, copyText, setForwarding, setCardFrom, pinned, setPinFor, unpin, setRemindFor, person, openFile, saveFile, startEdit, remove })
  const roomMenu = (r: TalkRoom) => roomMenuFor(r, {
    toggleFavorite, manageRoom, setToken, setShowPeople, setSettingsFor, setWallpaperRoom, notify, roomGone })

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
      <TalkRoomList
        query={query}
        setQuery={setQuery}
        setNewChat={setNewChat}
        loadRooms={loadRooms}
        rooms={rooms}
        error={error}
        visibleRooms={visibleRooms}
        token={token}
        setToken={setToken}
        roomMenu={roomMenu}
        peopleBusy={peopleBusy}
        people={people}
        chatWith={chatWith}
        peers={peers}
      />
      <Splitter id="talk-rooms" min={220} max={640} label="Ширина списка бесед" />

      <main className={`talk-main ${wp ? 'has-wallpaper' : ''}`} data-wallpaper={wpData}>
        {wp && <div className="talk-wallpaper" style={wp.layer} aria-hidden="true" />}
        {wp?.shade && <div className="talk-wallpaper-shade" style={wp.shade} aria-hidden="true" />}
        {room ? (
          <>
            <TalkChatHeader
              room={room}
              toggleFavorite={toggleFavorite}
              startCall={startCall}
              showMedia={showMedia}
              setShowMedia={setShowMedia}
              setShowPeople={setShowPeople}
              searchOpen={searchOpen}
              setSearchOpen={setSearchOpen}
              person={person}
              showPeople={showPeople}
              roomMenu={roomMenu}
            />
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
            <TalkMessageList
              listRef={listRef}
              stickToBottom={stickToBottom}
              loading={loading}
              messages={messages}
              hasOlder={hasOlder}
              olderBusy={olderBusy}
              loadOlder={loadOlder}
              flash={flash}
              messageMenu={messageMenu}
              room={room}
              person={person}
              notify={notify}
              openFile={openFile}
              saveFile={saveFile}
              react={react}
              readMark={readMark}
              setReactFor={setReactFor}
              reactFor={reactFor}
              setReplyTo={setReplyTo}
              setEditing={setEditing}
              inputRef={inputRef}
              setCardFrom={setCardFrom}
              startEdit={startEdit}
              remove={remove}
              reactFull={reactFull}
              setReactFull={setReactFull}
            />
            {room.readOnly ? <div className="muted talk-readonly">Беседа только для чтения</div> : (
              <TalkCompose
                send={send}
                files={files}
                previews={previews}
                sending={sending}
                dropFile={dropFile}
                replyTo={replyTo}
                setReplyTo={setReplyTo}
                editing={editing}
                cancelEdit={cancelEdit}
                pickFiles={pickFiles}
                mentions={mentions}
                mentionSel={mentionSel}
                pickMention={pickMention}
                emojiOpen={emojiOpen}
                insertEmoji={insertEmoji}
                inputRef={inputRef}
                draft={draft}
                onDraftChange={onDraftChange}
                onPaste={onPaste}
                setMentionSel={setMentionSel}
                setMentions={setMentions}
                setEmojiOpen={setEmojiOpen}
                setPollOpen={setPollOpen}
                silent={silent}
                setSilent={setSilent}
                notify={notify}
                sendVoice={sendVoice}
              />
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
