import { useEffect, useState } from 'react'

import { setAppearance, useAppearance } from './appearance'

/** Тема — из настроек «Вид» (светлая, тёмная, как в системе); кнопка в шапке переключает явно. */
export function useTheme() {
  const appearance = useAppearance()
  const systemDark = () => window.matchMedia?.('(prefers-color-scheme: dark)').matches ?? false
  const [sysDark, setSysDark] = useState(systemDark)
  useEffect(() => {
    const m = window.matchMedia?.('(prefers-color-scheme: dark)')
    if (!m) return
    const on = () => setSysDark(m.matches)
    m.addEventListener?.('change', on)
    return () => m.removeEventListener?.('change', on)
  }, [])
  const dark = appearance.theme === 'system' ? sysDark : appearance.theme === 'dark'
  const setDark = (v: boolean) => setAppearance({ theme: v ? 'dark' : 'light' })
  // Тема запоминается — окна писем открываются в той же теме. Класс темы — и на
  // всей странице: то, что выводится поверх окна программы, тоже меняет цвета.
  useEffect(() => {
    try { localStorage.setItem('rrcki-mail.theme', dark ? 'dark' : 'light') } catch { /* нет хранилища */ }
    document.documentElement.classList.toggle('theme-dark', dark)
  }, [dark])
  return { appearance, dark, setDark }
}
