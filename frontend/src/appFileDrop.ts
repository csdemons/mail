// Файлы, брошенные на окно почты, — новый черновик с вложениями. Вынесено из App.tsx.
import { api, errorText } from './api'
import { fileToBase64 } from './mailTools'
import type { Draft, PickedFile } from './types'

export interface FileDropDeps {
  setFileOver: (value: boolean) => void
  composeInWindow: (draft: Draft) => void
  newDraft: (to: string, files?: PickedFile[]) => Draft
  setNotice: (text: string) => void
}

export function createFileDrop(deps: FileDropDeps) {
  const { setFileOver, composeInWindow, newDraft, setNotice } = deps

  async function dropToNewMail(list: FileList) {
    setFileOver(false)
    const files: PickedFile[] = []
    const errors: string[] = []
    for (const file of Array.from(list)) {
      try {
        files.push(await api.attachData(file.name, await fileToBase64(file)))
      } catch (e) {
        errors.push(`${file.name}: ${errorText(e)}`)
      }
    }
    if (files.length > 0) {
      composeInWindow({ ...newDraft(''), subject: files.length === 1 ? files[0].name : '', files })
    }
    if (errors.length > 0) setNotice(`Не приложено: ${errors.join('; ')}`)
  }

  return { dropToNewMail }
}
