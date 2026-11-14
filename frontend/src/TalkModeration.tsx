// Модерация беседы (Talk 20+): зал ожидания и заблокированные участники — разделы настроек беседы.
import { useEffect, useState } from 'react'
import { ShieldBan } from 'lucide-react'

import { api, errorText } from './api'
import { DateTimeField } from './DateField'
import { localInputValue } from './sending'
import type { TalkBan, TalkRoom } from './types'

/** Зал ожидания: участники, кроме модераторов, ждут, пока его не откроют (вручную или в назначенное время). */
export function LobbySection({ room, notify, onChanged }: { room: TalkRoom; notify: (t: string) => void; onChanged: () => void }) {
  const [timer, setTimer] = useState(room.lobbyTimer ? localInputValue(new Date(room.lobbyTimer * 1000)) : '')
  const [busy, setBusy] = useState(false)
  async function set(on: boolean, at: string) {
    setBusy(true)
    try {
      const ts = at ? Math.floor(new Date(at).getTime() / 1000) : null
      await api.talkLobby(room.token, on, ts)
      notify(on ? (ts ? `Зал ожидания до ${new Date(ts * 1000).toLocaleString('ru-RU', { day: 'numeric', month: 'long', hour: '2-digit', minute: '2-digit' })}` : 'Зал ожидания включён') : 'Зал ожидания выключен')
      onChanged()
    } catch (e) { notify(`Не удалось: ${errorText(e)}`) } finally { setBusy(false) }
  }
  const on = room.lobbyState === 1
  return (
    <div className="settings-section notify-settings">
      <strong>Зал ожидания</strong>
      <label><input type="checkbox" checked={on} disabled={busy} onChange={(e) => void set(e.target.checked, timer)} /> Участники ждут, пока модератор откроет беседу</label>
      {on && <DateTimeField label="Открыть само в" value={timer} onChange={(v) => { setTimer(v); void set(true, v) }} />}
      <span className="settings-hint">Модераторы входят сразу. Удобно для вебинаров и совещаний по расписанию.</span>
    </div>
  )
}

/** Заблокированные участники: кто, кем, когда и заметка для модераторов; снять блокировку. */
export function BansSection({ room, notify }: { room: TalkRoom; notify: (t: string) => void }) {
  const [bans, setBans] = useState<TalkBan[] | null>(null)
  useEffect(() => { api.talkBans(room.token).then(setBans).catch(() => setBans([])) }, [room.token])
  async function unban(b: TalkBan) {
    try {
      await api.talkUnban(room.token, b.id)
      setBans((l) => l && l.filter((x) => x.id !== b.id))
      notify(`${b.name || b.actorId} снова может войти в беседу`)
    } catch (e) { notify(`Не удалось: ${errorText(e)}`) }
  }
  if (!bans?.length) return null
  return (
    <div className="settings-section talk-bans">
      <strong><ShieldBan size={14} /> Заблокированы</strong>
      {bans.map((b) => (
        <div key={b.id} className="talk-ban-row">
          <span><b>{b.name || b.actorId}</b> <small className="muted">— {b.by}, {new Date(b.time * 1000).toLocaleDateString('ru-RU')}{b.note ? `: ${b.note}` : ''}</small></span>
          <button type="button" className="text-button" onClick={() => void unban(b)}>Разблокировать</button>
        </div>
      ))}
    </div>
  )
}

/** Подтвердить и заблокировать участника (он удаляется из беседы и не вернётся по ссылке). */
export async function banParticipant(room: TalkRoom, actorType: string, actorId: string, name: string, notify: (t: string) => void, done: () => void) {
  if (!window.confirm(`Заблокировать ${name} в беседе «${room.name}»? Участник будет удалён и не сможет вернуться, пока его не разблокируют.`)) return
  try {
    await api.talkBan(room.token, actorType, actorId, `Заблокирован(а) в Atom Workspace ${new Date().toLocaleString('ru-RU')}`)
    notify(`${name} заблокирован в беседе`)
    done()
  } catch (e) { notify(`Не удалось заблокировать: ${errorText(e)}`) }
}
