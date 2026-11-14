import { useState } from 'react'

import { api, errorText } from './api'
import type { Folder } from './types'

/** Диагностика подключения (Настройки → Диагностика): отчёт по IMAP открытой папки, SMTP, DAV. */
export function useDiagnostics(folderId: string, folder: Folder | undefined, loaded: number, setNotice: (text: string) => void) {
  const [diagText, setDiagText] = useState<string | null>(null)
  const [diagBusy, setDiagBusy] = useState(false)

  async function runDiagnostics() {
    setDiagBusy(true)
    setDiagText('Сбор данных…')
    try {
      const report = await api.diagnostics(folder?.selectable ? folderId : null)
      setDiagText(`Папка в интерфейсе: ${folder?.path ?? folderId}, загружено писем: ${loaded}, сервер сообщал: ${folder?.total ?? '?'}\n${report}`)
    } catch (error) {
      setDiagText(`Ошибка диагностики: ${errorText(error)}`)
    } finally {
      setDiagBusy(false)
    }
  }

  async function copyDiagnostics() {
    if (!diagText) return
    try {
      await navigator.clipboard.writeText(diagText)
      setNotice('Отчёт скопирован в буфер обмена.')
    } catch {
      const area = document.querySelector<HTMLTextAreaElement>('.diag-text')
      area?.select()
      setNotice('Выделите текст и нажмите Ctrl+C.')
    }
  }

  return { diagText, diagBusy, runDiagnostics, copyDiagnostics }
}
