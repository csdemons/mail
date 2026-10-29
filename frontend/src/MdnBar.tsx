import { useEffect, useState } from 'react'
import { MailCheck } from 'lucide-react'

import { api, errorText } from './api'
import { loadSendPrefs } from './sending'
import type { MessageDetail } from './types'

/** Отправитель попросил уведомление о прочтении: спросить, как Outlook/Thunderbird.
 *  Ответ (или отказ) запоминается на сервере меткой $MDNSent — больше не спрашиваем. */
export default function MdnBar({ detail, me, onDone, onNotice }: {
  detail: MessageDetail
  me: string
  onDone: () => void
  onNotice: (text: string) => void
}) {
  const [busy, setBusy] = useState(false)
  const own = detail.fromAddress.toLowerCase() === me.toLowerCase()
  const asked = Boolean(detail.dispositionTo) && !detail.keywords.includes('$MDNSent') && !own
  const never = loadSendPrefs().mdnPolicy === 'never'

  useEffect(() => {
    if (asked && never) {
      void api.mdnRespond(detail.folder, detail.uid, false).then(onDone).catch(() => undefined)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [detail.folder, detail.uid, asked, never])

  if (!asked || never) return null

  async function respond(send: boolean) {
    setBusy(true)
    try {
      await api.mdnRespond(detail.folder, detail.uid, send)
      onDone()
      if (send) onNotice('Уведомление о прочтении отправлено.')
    } catch (e) {
      onNotice(`Не удалось: ${errorText(e)}`)
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="mdn-bar" role="note">
      <MailCheck size={16} />
      <span>Отправитель просит уведомление о прочтении.</span>
      <button className="text-button" disabled={busy} onClick={() => void respond(true)}>Отправить</button>
      <button className="text-button" disabled={busy} onClick={() => void respond(false)}>Не отправлять</button>
    </div>
  )
}
