// Действия над письмами и черновики — вынесено из App.tsx.
// Функции перенесены целиком; зависимости приходят из App явно.
import type { Dispatch, MutableRefObject, SetStateAction } from 'react'

import { api, errorText, messageKey } from './api'
import { trashFor } from './FolderPicker'
import { buildDraft, ensureArchive, forwardDraft, withSignature } from './mailActions'
import { byFolder, folderLabel } from './mailLabels'
import type { Draft, Folder, MessageDetail, MessageSummary, PickedFile, SignatureSet, UnifiedItem } from './types'

export interface ActionsDeps {
  setNotice: (text: string) => void
  folders: Folder[]
  loadFolders: () => Promise<unknown>
  setConfirmDelete: Dispatch<SetStateAction<MessageSummary[] | null>>
  setRowMenu: Dispatch<SetStateAction<{ x: number; y: number; items: MessageSummary[] } | null>>
  checked: Set<string>
  pool: MessageSummary[]
  selected: MessageSummary | undefined
  setChecked: Dispatch<SetStateAction<Set<string>>>
  /** Убирает письма из списка (объявлена в App: её использует и useMessageFlags). */
  dropFromList: (items: MessageSummary[], target: string | null) => void
  detail: MessageDetail | null
  windowOpened: MutableRefObject<boolean>
  profile: { displayName?: string | null } | null
  myAddress: string
  sigs: SignatureSet | null
  signature: string
  replySignature: string
  startDraft: (draft: Draft, folder?: string | null) => void
  meFor: (folder: string | null | undefined) => string
}

export function createActions(deps: ActionsDeps) {
  const {
    setNotice, folders, loadFolders, setConfirmDelete, setRowMenu, checked, pool, selected,
    setChecked, dropFromList, detail, windowOpened, profile, myAddress, sigs, signature,
    replySignature, startDraft, meFor,
  } = deps

  /** Письма, над которыми выполняется действие: отмеченные, иначе открытое. */
  function actionTargets(): MessageSummary[] {
    if (checked.size > 0) return pool.filter((m) => checked.has(messageKey(m)))
    return selected ? [selected] : []
  }

  /** Письмо другой учётной записи (поиск по всем ящикам) открывается с переключением на неё. */
  function otherOf(m: MessageSummary): UnifiedItem | null {
    const u = m as UnifiedItem
    return u.account && !u.active ? u : null
  }

  async function moveTo(items: MessageSummary[], target: string) {
    const groups = byFolder(items)
    if (groups.length > 1) {
      for (const g of groups) await moveTo(g, target)
      return
    }
    if (items.length === 0) return
    const source = items[0].folder
    if (target === source) return
    const dest = folders.find((f) => f.id === target)
    try {
      await api.moveMessages(source, items.map((m) => m.uid), target)
      dropFromList(items, target)
      setNotice(`${items.length === 1 ? 'Письмо перемещено' : `Перемещено писем: ${items.length}`} в «${dest ? folderLabel(dest) : target}».`)
    } catch (error) {
      setNotice(`Не удалось переместить: ${errorText(error)}`)
    }
  }

  /** Удаление: в корзину своего ящика; из корзины (или без корзины) — безвозвратно, с подтверждением. */
  function requestDelete(items: MessageSummary[], force = false) {
    if (items.length === 0) return
    const trash = trashFor(folders, items[0].folder)
    if (!trash || trash.id === items[0].folder || force) setConfirmDelete(items)
    else void deleteNow(items, false)
  }

  async function deleteNow(items: MessageSummary[], permanentAsked: boolean) {
    setConfirmDelete(null)
    const groups = byFolder(items)
    if (groups.length > 1) {
      for (const g of groups) await deleteNow(g, permanentAsked)
      return
    }
    const source = items[0].folder
    const trash = trashFor(folders, source)
    const toTrash = !permanentAsked && trash && trash.id !== source ? trash.id : null
    try {
      const permanent = await api.deleteMessages(source, items.map((m) => m.uid), toTrash)
      dropFromList(items, permanent ? null : toTrash)
      const n = items.length === 1 ? 'Письмо' : `Писем: ${items.length},`
      setNotice(permanent ? `${n} удалено безвозвратно.` : `${n} перемещено в корзину.`)
    } catch (error) {
      setNotice(`Не удалось удалить: ${errorText(error)}`)
    }
  }

  async function archive(items: MessageSummary[]) {
    const groups = byFolder(items)
    if (groups.length > 1) {
      for (const g of groups) await archive(g)
      return
    }
    if (items.length === 0) return
    try {
      const target = await ensureArchive(folders, items[0].folder)
      if (target.id === items[0].folder) {
        setNotice('Письмо уже в архиве.')
        return
      }
      if (target.created) await loadFolders()
      await moveTo(items, target.id)
    } catch (error) {
      setNotice(`Не удалось архивировать: ${errorText(error)}`)
    }
  }

  function openInWindow(m: MessageSummary) {
    api.openMessageWindow(m.folder, m.uid, m.subject || '(без темы)')
      .then(() => { windowOpened.current = true })
      .catch((error) => setNotice(`Не удалось открыть окно: ${errorText(error)}`))
  }

  function openRowMenu(e: React.MouseEvent, m: MessageSummary) {
    e.preventDefault()
    const key = messageKey(m)
    // Как в Outlook: правый щелчок по отмеченным — меню для всех отмеченных, иначе — для этого письма.
    const items = checked.has(key) && checked.size > 1 ? pool.filter((x) => checked.has(messageKey(x))) : [m]
    if (!checked.has(key) && checked.size > 0) setChecked(new Set())
    setRowMenu({ x: e.clientX, y: e.clientY, items })
  }

  /** Новое письмо: черновик для отдельного окна (там его можно перенести на другой монитор). */
  function newDraft(to: string, files?: PickedFile[]): Draft {
    return { mode: 'new', to, cc: '', subject: '', body: withSignature('', signature), inReplyTo: null, references: null, files, signatureId: sigs?.newId ?? null }
  }

  /** Открыть новое письмо отдельным окном; если окно не открылось — обычным окном-модалом. */
  function composeInWindow(draft: Draft) {
    const from = profile?.displayName ? `${profile.displayName} <${myAddress}>` : myAddress
    void api.openComposeWindow(JSON.stringify({ from, draft })).then(() => {
      // Возврат фокуса в главное окно обновит папки и список после отправки.
      windowOpened.current = true
    }).catch((error) => {
      setNotice(`Не удалось открыть окно письма: ${errorText(error)}`)
      startDraft(draft)
    })
  }

  function openComposer(mode: 'new' | 'reply' | 'replyAll' | 'forward') {
    if (mode === 'new' || !detail) {
      composeInWindow(newDraft(''))
      return
    }
    if (mode === 'forward') {
      void forwardDraft(detail, myAddress, replySignature).then(({ draft, error }) => { startDraft(draft, detail.folder); if (error) setNotice(error) })
      return
    }
    startDraft(buildDraft(mode, detail, meFor(detail.folder), replySignature), detail.folder)
  }

  function composeTo(to: string) {
    composeInWindow(newDraft(`${to}, `))
  }

  /** Ответ/пересылка из меню — письмо может быть ещё не открыто: загружаем его. */
  async function composeFor(m: MessageSummary, mode: 'reply' | 'replyAll' | 'forward') {
    try {
      const full = detail && messageKey(detail) === messageKey(m) ? detail : await api.getMessage(m.folder, m.uid)
      if (mode === 'forward') {
        const { draft, error } = await forwardDraft(full, myAddress, replySignature)
        startDraft(draft, full.folder)
        if (error) setNotice(error)
        return
      }
      startDraft(buildDraft(mode, full, meFor(full.folder), replySignature), full.folder)
    } catch (error) {
      setNotice(`Не удалось открыть письмо: ${errorText(error)}`)
    }
  }

  return { actionTargets, otherOf, moveTo, requestDelete, deleteNow, archive, openInWindow, openRowMenu, newDraft, composeInWindow, openComposer, composeTo, composeFor }
}
