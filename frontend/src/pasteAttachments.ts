// Вставленные из буфера картинки и файлы — вложениями к письму или сообщению.
// Что WebView отдаёт сам (снимок экрана, картинка из браузера) — берём из события;
// файлы из проводника и картинки, которых WebView не видит, — из буфера обмена системы.
import { api } from './api'
import { fileToBase64 } from './mailTools'
import { pastedName, type PasteData } from './pasteClipboard'
import type { PickedFile } from './types'

/** Вложения из буфера. Пусто — в буфере только текст. */
export async function attachmentsFromPaste(d: PasteData): Promise<PickedFile[]> {
  if (d.files.length > 0) {
    const picked: PickedFile[] = []
    for (const f of d.files) {
      picked.push(await api.attachData(pastedName(f.name, f.type), await fileToBase64(f)))
    }
    return picked
  }
  return api.clipboardPasteFiles()
}
