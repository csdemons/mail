import { useCallback, useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { Copy, Download, RefreshCw, X } from 'lucide-react'

import { api, errorText, formatSize } from './api'
import type { UpdateInfo } from './types'

const CHECK_EVERY_MS = 6 * 60 * 60 * 1000
const FIRST_CHECK_MS = 8000
/** Событие «проверить сейчас» (кнопка в настройках). */
export const CHECK_UPDATE_EVENT = 'rrcki:check-update'

type Phase =
  | { kind: 'idle' }
  | { kind: 'available'; info: UpdateInfo }
  | { kind: 'downloading'; info: UpdateInfo; done: number; total: number }
  | { kind: 'installing'; info: UpdateInfo }
  | { kind: 'manual'; info: UpdateInfo; command: string }
  | { kind: 'message'; text: string }

/** Место для значка обновления в шапке окна (App). */
export const UPDATE_SLOT_ID = 'update-slot'

/**
 * Уведомление о новой версии: проверка при запуске и каждые 6 часов. В рабочем окне —
 * значок «Обновление» в шапке и окошко по щелчку (раньше карточка закрывала угол с папками);
 * на экране входа — карточка, как раньше.
 */
export default function UpdateNotice() {
  const [phase, setPhase] = useState<Phase>({ kind: 'idle' })
  const [open, setOpen] = useState(false)
  const [slot, setSlot] = useState<HTMLElement | null>(null)
  const popRef = useRef<HTMLDivElement>(null)

  // Шапка появляется и пропадает (вход, блокировка) — следим за местом для значка.
  useEffect(() => {
    const find = () => setSlot(document.getElementById(UPDATE_SLOT_ID))
    find()
    const ob = new MutationObserver(find)
    ob.observe(document.body, { childList: true, subtree: true })
    return () => ob.disconnect()
  }, [])
  // Окошко закрывается щелчком мимо.
  useEffect(() => {
    if (!open) return
    const down = (e: MouseEvent) => {
      const t = e.target as Node
      if (popRef.current?.contains(t) || slot?.contains(t)) return
      setOpen(false)
    }
    document.addEventListener('mousedown', down)
    return () => document.removeEventListener('mousedown', down)
  }, [open, slot])
  const [showNotes, setShowNotes] = useState(false)
  // «Позже» — не напоминать об этой версии до перезапуска программы.
  const dismissed = useRef<string | null>(null)
  const busy = useRef(false)

  const check = useCallback(async (manual: boolean) => {
    if (busy.current) return
    try {
      const info = await api.checkUpdate()
      if (info && (manual || dismissed.current !== info.version)) {
        setPhase((p) => (p.kind === 'downloading' || p.kind === 'installing' ? p : { kind: 'available', info }))
      } else if (manual) {
        setPhase({ kind: 'message', text: info ? '' : 'У вас последняя версия.' })
      }
      if (manual) setOpen(true)
    } catch (e) {
      if (manual) { setPhase({ kind: 'message', text: `Проверка обновлений: ${errorText(e)}` }); setOpen(true) }
    }
  }, [])

  useEffect(() => {
    const first = setTimeout(() => void check(false), FIRST_CHECK_MS)
    const every = setInterval(() => void check(false), CHECK_EVERY_MS)
    const onManual = () => void check(true)
    window.addEventListener(CHECK_UPDATE_EVENT, onManual)
    return () => { clearTimeout(first); clearInterval(every); window.removeEventListener(CHECK_UPDATE_EVENT, onManual) }
  }, [check])

  async function install(info: UpdateInfo) {
    busy.current = true
    setOpen(true)
    setPhase({ kind: 'downloading', info, done: 0, total: info.size })
    const timer = setInterval(() => {
      api.updateProgress()
        .then((p) => setPhase((cur) => (cur.kind === 'downloading' ? { ...cur, done: p.done, total: p.total || cur.total } : cur)))
        .catch(() => undefined)
    }, 400)
    try {
      const result = await api.installUpdate()
      clearInterval(timer)
      if (result.restarting) setPhase({ kind: 'installing', info })
      else setPhase({ kind: 'manual', info, command: result.manual ?? '' })
    } catch (e) {
      clearInterval(timer)
      setPhase({ kind: 'message', text: `Обновление не установлено: ${errorText(e)}` })
    } finally {
      busy.current = false
    }
  }

  if (phase.kind === 'idle' || (phase.kind === 'message' && !phase.text)) return null

  const close = () => { setPhase({ kind: 'idle' }); setOpen(false) }
  // «Позже» — окошко закрывается, значок в шапке остаётся (на экране входа карточка пропадает).
  const later = (version: string) => { dismissed.current = version; if (slot) setOpen(false); else close() }

  const version = 'info' in phase ? phase.info.version : ''
  const pill = slot && createPortal(
    <button className={`update-pill ${phase.kind === 'downloading' || phase.kind === 'installing' ? 'busy' : ''}`} aria-label="Обновление" aria-expanded={open}
      title={version ? `Доступна версия ${version}` : 'Обновление'} onClick={() => setOpen((o) => !o)}>
      {phase.kind === 'downloading' || phase.kind === 'installing' ? <RefreshCw size={15} className="spin" /> : <Download size={15} />}
      <span>{phase.kind === 'message' ? 'Обновления' : 'Обновление'}</span>
    </button>,
    slot,
  )
  if (slot && !open) return pill

  return (
    <>
    {pill}
    <div className={`update-card ${slot ? 'update-pop' : ''}`} role="status" ref={popRef}>
      {phase.kind === 'message' && (
        <><span className="update-text">{phase.text}</span><button className="icon-button" aria-label="Закрыть" onClick={close}><X size={15} /></button></>
      )}
      {phase.kind === 'available' && (
        <div className="update-body">
          <div className="update-head">
            <Download size={18} />
            <div><strong>Доступна версия {phase.info.version}</strong><span>у вас {phase.info.current}{phase.info.size ? ` · ${formatSize(phase.info.size)}` : ''}</span></div>
            <button className="icon-button" aria-label="Позже" title="Позже" onClick={() => later(phase.info.version)}><X size={15} /></button>
          </div>
          {phase.info.notes && (showNotes
            ? <pre className="update-notes">{phase.info.notes}</pre>
            : <button className="text-button" onClick={() => setShowNotes(true)}>Что нового</button>)}
          <div className="update-actions">
            <button className="secondary-button" onClick={() => later(phase.info.version)}>Позже</button>
            <button className="primary-button" onClick={() => void install(phase.info)}><RefreshCw size={15} /> Обновить</button>
          </div>
        </div>
      )}
      {phase.kind === 'downloading' && (
        <div className="update-body">
          <strong>Загрузка {phase.info.version}…</strong>
          <div className="update-bar"><i style={{ width: `${phase.total ? Math.min(100, (phase.done / phase.total) * 100) : 5}%` }} /></div>
          <span className="update-text">{formatSize(phase.done)}{phase.total ? ` из ${formatSize(phase.total)}` : ''}</span>
        </div>
      )}
      {phase.kind === 'installing' && (
        <div className="update-body"><strong>Устанавливается {phase.info.version}</strong><span className="update-text">Программа перезапустится через несколько секунд.</span></div>
      )}
      {phase.kind === 'manual' && (
        <div className="update-body">
          <div className="update-head"><Download size={18} /><div><strong>Версия {phase.info.version} скачана</strong><span>Для установки нужны права администратора</span></div><button className="icon-button" aria-label="Закрыть" onClick={close}><X size={15} /></button></div>
          <code className="update-cmd">{phase.command}</code>
          <div className="update-actions"><button className="secondary-button" onClick={() => void navigator.clipboard?.writeText(phase.command)}><Copy size={14} /> Скопировать команду</button></div>
        </div>
      )}
    </div>
    </>
  )
}
