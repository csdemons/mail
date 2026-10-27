import { useEffect, useState } from 'react'
import { Mail } from 'lucide-react'

import { api, VARIANT_LABEL } from './api'
import type { AppInfo } from './types'

/** Блок «О программе» в настройках. */
export default function About() {
  const [info, setInfo] = useState<AppInfo | null>(null)
  useEffect(() => { api.appInfo().then(setInfo).catch(() => undefined) }, [])
  return (
    <div className="settings-section about-block">
      <div className="about-head">
        <div className="brand-mark"><Mail size={18} fill="currentColor" /></div>
        <div>
          <strong>RRCKI Mail{info ? ` ${info.version}` : ''}</strong>
          <span>{info ? VARIANT_LABEL[info.variant] ?? info.variant : ''}</span>
        </div>
      </div>
      <p>Почтовый клиент разработан специально для НИЦ «Курчатовский институт»: почта, календарь, контакты, задачи и чат в одном окне.</p>
      <p className="about-author">Разработчик: <b>Юшков Дмитрий Олегович</b></p>
    </div>
  )
}
