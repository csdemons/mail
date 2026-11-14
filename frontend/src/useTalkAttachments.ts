import { useEffect, useState, type ClipboardEvent } from 'react'

import { api, errorText } from './api'
import type { PickedFile } from './types'

/** Вложения сообщения чата: выбор файлов, вставка из буфера обмена (картинки — с превью). */
export function useTalkAttachments(editing: boolean, notify: (text: string) => void, setError: (text: string) => void) {
  const [files, setFiles] = useState<PickedFile[]>([])
  // Картинки из буфера обмена, ожидающие отправки: превью по id вложения.
  const [previews, setPreviews] = useState<Record<string, string>>({})
  useEffect(() => {
    // Превью отправленных или убранных картинок больше не нужны.
    const gone = Object.keys(previews).filter((id) => !files.some((f) => f.id === id))
    if (gone.length === 0) return
    gone.forEach((id) => URL.revokeObjectURL(previews[id]))
    setPreviews((p) => Object.fromEntries(Object.entries(p).filter(([id]) => !gone.includes(id))))
  }, [files, previews])


  async function pickFiles() {
    try {
      const picked = await api.pickAttachments()
      setFiles((list) => [...list, ...picked])
    } catch (e) {
      setError(errorText(e))
    }
  }

  function dropFile(id: string) {
    setFiles((list) => list.filter((f) => f.id !== id))
    void api.forgetAttachment(id)
  }

  /** Вложения, вставленные из буфера обмена: в списке файлов, картинки — с превью. */
  async function addPasted(list: PickedFile[]) {
    if (list.length === 0) return
    setFiles((f) => [...f, ...list])
    for (const p of list) {
      if (/\.(png|jpe?g|gif|webp|bmp)$/i.test(p.name)) {
        const url = await api.pickedPreview(p.id).catch(() => null)
        if (url) setPreviews((x) => ({ ...x, [p.id]: url }))
      }
    }
    notify(list.length === 1 ? `Вставлено: ${list[0].name}` : `Вставлено файлов: ${list.length}`)
  }

  /**
   * Ctrl+V (и «Вставить» в меню): картинки и файлы из буфера — вложениями к сообщению.
   * Что WebView отдаёт сам (снимок экрана в Windows) — берём из события; файлы, скопированные
   * в проводнике, и картинки, которых WebView не видит, — из буфера обмена системы.
   */
  async function onPaste(e: ClipboardEvent<HTMLTextAreaElement>) {
    if (editing) return
    const dt = e.clipboardData
    const direct = [...dt.files]
    const types = [...dt.types]
    const text = dt.getData('text/plain')
    const uris = dt.getData('text/uri-list')
    const fileUris = /^file:\/\//im.test(uris) || /^(\/|file:\/\/)\S+$/m.test(text) && types.includes('text/uri-list')
    if (direct.length === 0 && text && !fileUris) return // обычный текст — вставляется как есть
    e.preventDefault()
    const target = e.currentTarget
    try {
      if (direct.length > 0) {
        const picked: PickedFile[] = []
        for (const f of direct) {
          const buf = new Uint8Array(await f.arrayBuffer())
          let bin = ''
          for (let i = 0; i < buf.length; i += 0x8000) bin += String.fromCharCode(...buf.subarray(i, i + 0x8000))
          const ext = (f.type.split('/')[1] || 'png').replace('jpeg', 'jpg').replace(/[^a-z0-9]/gi, '')
          const stamp = new Date().toLocaleString('ru-RU').replace(/[^\d]+/g, '-').replace(/-$/, '')
          const name = f.name && f.name !== 'image.png' ? f.name : `Снимок ${stamp}.${ext}`
          picked.push(await api.attachData(name, btoa(bin)))
        }
        await addPasted(picked)
        return
      }
      const fromOs = await api.clipboardPasteFiles()
      if (fromOs.length > 0) { await addPasted(fromOs); return }
      // В буфере всё-таки текст (например, путь к файлу, которого уже нет) — вставить его.
      if (text) { target.focus(); document.execCommand('insertText', false, text) }
    } catch (err) {
      notify(`Не вставлено: ${errorText(err)}`)
    }
  }

  return { files, setFiles, previews, pickFiles, dropFile, onPaste }
}
