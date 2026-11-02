import { useCallback, useEffect, useRef, useState } from 'react'
import { CloudOff, Loader2, RefreshCw, RotateCcw, Send, Trash2, TriangleAlert, X } from 'lucide-react'

import { api, errorText } from './api'
import type { ConnectionStatus } from './types'

const fmt = (iso: string | null) => (iso ? new Date(iso).toLocaleString('ru-RU', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' }) : '')

/** Связь с почтовым сервером и очередь действий без сети. Опрос — часто, пока
 *  связи нет или очередь не пуста; `onOnline` — связь появилась (обновить списки). */
export function useConnection(onOnline: () => void) {
  const [status, setStatus] = useState<ConnectionStatus | null>(null)
  const wasOnline = useRef(true)
  const cb = useRef(onOnline)
  cb.current = onOnline

  const apply = useCallback((s: ConnectionStatus) => {
    setStatus(s)
    if (s.online && !wasOnline.current) cb.current()
    wasOnline.current = s.online
  }, [])

  useEffect(() => {
    let stopped = false
    let timer: ReturnType<typeof setTimeout> | undefined
    async function tick() {
      let busy = false
      try {
        const s = await api.offlineQueue()
        if (stopped) return
        apply(s)
        busy = !s.online || s.items.length > 0 || s.syncing
      } catch { /* нет сеанса */ }
      if (!stopped) timer = setTimeout(tick, busy ? 2000 : 8000)
    }
    void tick()
    return () => { stopped = true; if (timer) clearTimeout(timer) }
  }, [apply])

  const refresh = useCallback(async () => { try { apply(await api.offlineQueue()) } catch { /* нет сеанса */ } }, [apply])
  return { status, setStatus: apply, refresh }
}

/** Плашка над списком писем: нет связи / идёт отправка очереди / есть конфликты. */
export function OfflineBanner({ status, onOpen, onCheck, checking }: {
  status: ConnectionStatus | null
  onOpen: () => void
  onCheck: () => void
  checking: boolean
}) {
  if (!status) return null
  const n = status.items.length
  const c = status.conflicts.length
  if (status.online && n === 0 && c === 0) return null
  if (!status.online) {
    return <div className="offline-banner" role="status" aria-label="Работа без сети">
      <CloudOff size={16} />
      <span>
        <b>Нет связи с почтовым сервером{status.offlineSince ? ` с ${fmt(status.offlineSince)}` : ''}.</b>{' '}
        {status.available ? 'Показаны письма с компьютера; действия и новые письма выполнятся при подключении.' : 'Хранение писем на компьютере выключено — без связи работа невозможна.'}
      </span>
      {(n > 0 || c > 0) && <button className="text-button" onClick={onOpen}>В очереди: {n}{c > 0 ? ` · конфликты: ${c}` : ''}</button>}
      <button className="text-button" disabled={checking} onClick={onCheck}>{checking ? <Loader2 className="spin" size={14} /> : <RefreshCw size={14} />} Проверить связь</button>
    </div>
  }
  return <div className={`offline-banner ${c > 0 && n === 0 ? 'conflicts' : 'syncing'}`} role="status" aria-label="Очередь действий">
    {n > 0 ? <Loader2 className="spin" size={16} /> : <TriangleAlert size={16} />}
    <span>{n > 0 ? `Связь появилась — выполняются действия без сети: ${n}` : `Не всё из сделанного без сети удалось выполнить: ${c}`}</span>
    <button className="text-button" onClick={onOpen}>Подробнее</button>
  </div>
}

/** Очередь без сети: ожидающие действия и письма, конфликты после подключения. */
export function QueuePanel({ status, onStatus, onClose, onNotice }: {
  status: ConnectionStatus
  onStatus: (s: ConnectionStatus) => void
  onClose: () => void
  onNotice: (t: string) => void
}) {
  const [busy, setBusy] = useState(false)
  async function run(f: () => Promise<ConnectionStatus>, done?: string) {
    setBusy(true)
    try { onStatus(await f()); if (done) onNotice(done) } catch (e) { onNotice(errorText(e)) } finally { setBusy(false) }
  }
  return <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}>
    <div className="settings-modal queue-modal" role="dialog" aria-label="Очередь без сети">
      <div className="compose-header">
        <div><strong><CloudOff size={16} /> Работа без сети</strong><small>Очередь действий и писем, конфликты после подключения</small></div>
        <button className="icon-button" aria-label="Закрыть" onClick={onClose}><X size={18} /></button>
      </div>
      <div className="queue-state">
        {status.online ? <span className="queue-online">Связь с сервером есть{status.syncing ? ' — очередь выполняется' : ''}</span>
          : <span className="queue-offline"><CloudOff size={15} /> Нет связи{status.offlineSince ? ` с ${fmt(status.offlineSince)}` : ''}{status.lastError ? ` — ${status.lastError}` : ''}</span>}
        <button className="secondary-button" disabled={busy} onClick={() => void run(() => api.offlineCheck())}><RefreshCw size={14} /> {status.online ? 'Выполнить сейчас' : 'Проверить связь'}</button>
      </div>
      <h3>Ожидают подключения <span className="muted">{status.items.length}</span></h3>
      {status.items.length === 0 && <div className="muted queue-empty">Очередь пуста.</div>}
      <ul className="queue-list">
        {status.items.map((i) => <li key={i.id} className="queue-item" data-id={i.id}>
          {i.send ? <Send size={14} /> : <RotateCcw size={14} />}
          <span className="queue-text">{i.what}<small className="muted">{fmt(i.created)}{i.error ? ` · ошибка: ${i.error}` : ''}</small></span>
          <button className="icon-button" disabled={busy} title={i.send ? 'Не отправлять' : 'Не выполнять'} aria-label="Убрать из очереди"
            onClick={() => { if (!i.send || window.confirm('Убрать письмо из очереди? Оно не будет отправлено.')) void run(() => api.offlineResolve(i.id), 'Убрано из очереди.') }}><Trash2 size={14} /></button>
        </li>)}
      </ul>
      {status.conflicts.length > 0 && <>
        <h3>Не выполнено после подключения <span className="muted">{status.conflicts.length}</span></h3>
        <ul className="queue-list">
          {status.conflicts.map((c) => <li key={c.id} className="queue-item conflict" data-id={c.id}>
            <TriangleAlert size={14} />
            <span className="queue-text">{c.what}<small>{c.reason}</small><small className="muted">{fmt(c.when)}</small></span>
            {c.canRetry && <button className="secondary-button" disabled={busy} onClick={() => void run(() => api.offlineResolve(c.id, true), 'Письмо снова в очереди исходящих.')}>Отправить ещё раз</button>}
            <button className="icon-button" disabled={busy} title="Скрыть" aria-label="Скрыть конфликт" onClick={() => void run(() => api.offlineResolve(c.id))}><X size={14} /></button>
          </li>)}
        </ul>
      </>}
      <p className="muted queue-help">Пока нет связи, программа показывает письма, сохранённые на компьютере, а отметки, категории, перемещение, удаление и отправку писем выполняет у себя и ставит в очередь. При подключении очередь выполняется по порядку. Если письмо за это время переместили или удалили в другом месте, действие с ним не выполняется и попадает в список выше. Письмо, при отправке которого оборвалась связь, само повторно не отправляется — решите сами.</p>
    </div>
  </div>
}
