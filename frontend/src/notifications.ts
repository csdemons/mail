import { api } from './api'

/** Настройки всплывающих уведомлений (на этом компьютере). */
export interface NotifyPrefs {
  mail: boolean
  chat: boolean
  /** Письма в подключённых (общих) ящиках. */
  shared: boolean
  /** Показывать тему письма / текст сообщения (иначе только «Новое письмо от …»). */
  preview: boolean
}

const KEY = 'rrcki-mail.notify'
export const DEFAULT_PREFS: NotifyPrefs = { mail: true, chat: true, shared: false, preview: true }

export function loadPrefs(): NotifyPrefs {
  try {
    const raw = localStorage.getItem(KEY)
    if (raw) return { ...DEFAULT_PREFS, ...(JSON.parse(raw) as Partial<NotifyPrefs>) }
  } catch { /* нет хранилища */ }
  return DEFAULT_PREFS
}

export function savePrefs(p: NotifyPrefs) {
  try { localStorage.setItem(KEY, JSON.stringify(p)) } catch { /* не критично */ }
}

/** Окно программы сейчас перед пользователем. */
export function appInFront() {
  return !document.hidden && document.hasFocus()
}

export function notify(title: string, body: string) {
  api.notify(title, body).catch(() => undefined)
}
