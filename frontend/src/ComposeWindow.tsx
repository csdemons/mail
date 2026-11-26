// Окно нового письма: открывается отдельным окном ОС (метка msg-compose-*), поэтому его
// можно перенести на другой монитор — в отличие от модального окна внутри главного.
import { useEffect, useState } from 'react'

import { api } from './api'
import Composer from './Composer'
import { useOutbox } from './sending'
import type { Contact, Draft, SignatureSet } from './types'

declare global {
  interface Window {
    /** Начальные данные окна нового письма (ставит Rust в скрипте инициализации). */
    __RRCKI_COMPOSE?: { from: string; draft: Draft }
  }
}

/** Черновик, отправитель и подпись приходят из главного окна; контакты, подписи и
 *  политика предупреждения о внешних адресатах читаются здесь же, как в окне письма. */
export default function ComposeWindow({ payload }: { payload: { from: string; draft: Draft } }) {
  const [draft, setDraft] = useState<Draft | null>(payload.draft)
  const [contacts, setContacts] = useState<Contact[]>([])
  const [sigs, setSigs] = useState<SignatureSet | null>(null)
  const [ncConnected, setNcConnected] = useState(false)
  const [notice, setNotice] = useState('')
  const [policy, setPolicy] = useState<{ internalDomains: string[]; mode: 'smart' | 'always' | 'off' }>({ internalDomains: [], mode: 'smart' })
  const close = () => { void api.closeWindow().catch(() => undefined) }
  // «Отменить» после отправки работает в этом же окне: черновик возвращается сюда,
  // а после успешной отправки окно закрывается само.
  const outbox = useOutbox((d) => { setDraft(d); setNotice('') }, setNotice, close)

  useEffect(() => {
    api.listContacts().then((r) => setContacts(r.contacts)).catch(() => undefined)
    api.signaturesGet().then(setSigs).catch(() => undefined)
    api.ncStatus().then((st) => setNcConnected(st.connected)).catch(() => undefined)
    api.getPolicy()
      .then((p) => setPolicy({ internalDomains: p.internalDomains ?? [], mode: p.externalWarning ?? 'smart' }))
      .catch(() => undefined)
  }, [])

  return (
    <div className="compose-window">
      {draft ? (
        <Composer
          from={payload.from}
          draft={draft}
          contacts={contacts}
          ncConnected={ncConnected}
          signatures={sigs}
          recipientPolicy={policy}
          recovery={false}
          onClose={close}
          onSent={(text) => {
            setDraft(null)
            setNotice(text)
            // Даём прочитать подтверждение и закрываем окно: список обновится в главном.
            window.setTimeout(close, 1400)
          }}
          onQueued={(id, snapshot, delay) => {
            setDraft(null)
            outbox.queued(id, snapshot, delay)
          }}
        />
      ) : (
        <div className="compose-window-wait" role="status">
          <span>{notice || 'Письмо отправлено.'}</span>
          <button className="secondary-button" onClick={close}>Закрыть окно</button>
        </div>
      )}
      {outbox.element}
    </div>
  )
}
