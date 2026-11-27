// Открытие писем и вложения — вынесено из App.tsx, чтобы файл не разрастался.
// Поведение прежнее: функции перенесены целиком, зависимости приходят из App.
import type { Dispatch, MutableRefObject, SetStateAction } from 'react'

import { api, errorText, NC_MAIL_DIR } from './api'
import type { Found } from './GlobalSearch'
import { withSignature } from './mailActions'
import type { ModuleName } from './viewContext'
import type { CalEvent, Contact, Draft, MessageDetail, MessageSummary, PickedFile, UnifiedRole } from './types'

/** Куда перейти при выборе найденного в общем поиске. */
interface FocusSetters {
  setModule: Dispatch<SetStateAction<ModuleName>>
  setUnified: Dispatch<SetStateAction<UnifiedRole | null>>
  setFolderId: (id: string) => void
  setFilesFocus: Dispatch<SetStateAction<{ path: string; isDir: boolean; n: number } | null>>
  setContactFocus: Dispatch<SetStateAction<{ contact: Contact; n: number } | null>>
  setEventFocus: Dispatch<SetStateAction<{ e: CalEvent; n: number } | null>>
  setDeckFocus: Dispatch<SetStateAction<{ boardId: number; cardId: number; n: number } | null>>
  setTalkFocus: Dispatch<SetStateAction<{ token: string; messageId: number | null; n: number } | null>>
}

export interface ReadingDeps extends FocusSetters {
  setNotice: (text: string) => void
  startDraft: (draft: Draft) => void
  patchMessage: (key: string, patch: Partial<MessageSummary>) => void
  adjustUnseen: (folder: string, delta: number) => void
  messageKey: (m: MessageSummary) => string
  nextFocus: () => number
  openHistory: (domain: string) => void
  clearSearch: () => void
  lastOpened: MutableRefObject<MessageSummary | null>
  detailRequest: MutableRefObject<number>
  detail: MessageDetail | null
  myAddress: string
  signature: string
  setSelectedKey: Dispatch<SetStateAction<string>>
  setReaderOpen: Dispatch<SetStateAction<boolean>>
  setDetail: Dispatch<SetStateAction<MessageDetail | null>>
  setDetailError: Dispatch<SetStateAction<string>>
  setDetailLoading: Dispatch<SetStateAction<boolean>>
}

export function createReading(deps: ReadingDeps) {
  const {
    setNotice, startDraft, patchMessage, adjustUnseen, messageKey, nextFocus, openHistory, clearSearch,
    lastOpened, detailRequest, detail, myAddress, signature,
    setModule, setUnified, setFolderId, setSelectedKey, setReaderOpen, setDetail, setDetailError, setDetailLoading,
    setFilesFocus, setContactFocus, setEventFocus, setDeckFocus, setTalkFocus,
  } = deps

  /** Открыть найденное (общий поиск, история организации) в его разделе. */
  function openFound(f: Found) {
    const n = nextFocus()
    switch (f.kind) {
      case 'mail':
        setModule('mail')
        clearSearch()
        setUnified(null)
        setFolderId(f.m.folder)
        void openMessage(f.m)
        break
      case 'file': setFilesFocus({ path: f.f.path, isDir: f.f.isDir, n }); setModule('files'); break
      case 'contact': setContactFocus({ contact: f.c, n }); setModule('contacts'); break
      case 'event': setEventFocus({ e: f.e, n }); setModule('calendar'); break
      case 'card': setDeckFocus({ boardId: f.h.boardId, cardId: f.h.card.id, n }); setModule('deck'); break
      case 'chat': setTalkFocus({ token: f.h.token, messageId: f.h.messageId, n }); setModule('chat'); break
      case 'org': openHistory(f.domain); break
    }
  }

  /** Письмо по Message-ID (ссылка из карточки Deck). */
  async function openMailById(msgId: string) {
    const m = await api.mailByMsgid(msgId).catch(() => null)
    if (m) openFound({ kind: 'mail', m })
    else setNotice('Письмо не найдено среди хранящихся на компьютере — найдите его поиском по теме.')
  }

  /** Письмо из «Черновиков» — дописать: тот же текст, получатели и вложения; при сохранении заменяется. */
  async function continueDraft(d: MessageDetail) {
    const ids = (d.references ?? '').split(/\s+/).filter(Boolean)
    let files: PickedFile[] = []
    if (d.attachments.length) {
      try { files = await api.forwardAttachments(d.folder, d.uid) } catch (e) { setNotice(`Вложения черновика не приложены: ${errorText(e)}`) }
    }
    startDraft({
      mode: 'new', to: d.to.join(', '), cc: d.cc.join(', '), subject: d.subject, body: d.bodyText, html: d.bodyHtml ?? undefined,
      inReplyTo: ids[ids.length - 1] ?? null, references: d.references, files, readReceipt: false,
      replace: { folder: d.folder, uid: d.uid }, signatureId: null,
      from: d.fromAddress && d.fromAddress.toLowerCase() !== myAddress.toLowerCase() ? d.fromAddress.toLowerCase() : null })
  }

  async function openMessage(message: MessageSummary) {
    lastOpened.current = message
    const key = messageKey(message)
    const request = ++detailRequest.current
    setSelectedKey(key)
    setReaderOpen(true)
    setDetail(null)
    setDetailError('')
    setDetailLoading(true)
    try {
      const loaded = await api.getMessage(message.folder, message.uid)
      if (request !== detailRequest.current) return
      setDetail(loaded)
      patchMessage(key, { hasAttachments: loaded.hasAttachments })
      if (loaded.unread) {
        patchMessage(key, { unread: false })
        adjustUnseen(message.folder, -1)
        api.setFlag(message.folder, message.uid, 'seen', true).catch((error) => {
          patchMessage(key, { unread: true })
          adjustUnseen(message.folder, +1)
          setNotice(`Не удалось отметить письмо прочитанным: ${errorText(error)}`)
        })
      }
    } catch (error) {
      if (request === detailRequest.current) setDetailError(errorText(error))
    } finally {
      if (request === detailRequest.current) setDetailLoading(false)
    }
  }

  async function doUnsubscribe(d: MessageDetail) {
    try {
      const r = await api.unsubscribe(d.folder, d.uid)
      if (r.done) setNotice(`Вы отписаны от рассылки (${r.how}).`)
      else if (r.openUrl) { setNotice('Отписка — на сайте рассылки: открываю в браузере.'); void api.openLink(r.openUrl).catch(() => undefined) }
    } catch (e) {
      setNotice(`Не удалось отписаться: ${errorText(e)}`)
    }
  }

  /** Новое письмо со ссылками на файлы Nextcloud. */
  function sendLinks(links: { name: string; url: string }[]) {
    const lines = links.map((l) => `${l.name}: ${l.url}`).join('\n')
    startDraft({
      mode: 'new', to: '', cc: '', inReplyTo: null, references: null,
      subject: links.length === 1 ? links[0].name : `Файлы (${links.length})`,
      body: `\n\n${links.length === 1 ? 'Ссылка на файл' : 'Ссылки на файлы'}:\n${lines}` + withSignature('', signature) })
  }

  async function saveToNextcloud(index: number, name: string) {
    if (!detail) return
    try {
      const path = await api.ncSaveMailAttachment(detail.folder, detail.uid, index, NC_MAIL_DIR)
      setNotice(`«${name}» сохранено в Nextcloud: ${path}`)
    } catch (error) {
      setNotice(`Не удалось сохранить в Nextcloud: ${errorText(error)}`)
    }
  }

  async function saveAttachment(index: number) {
    if (!detail) return
    try {
      const path = await api.saveAttachment(detail.folder, detail.uid, index)
      if (path) setNotice(`Сохранено: ${path}`)
    } catch (error) {
      setNotice(`Не удалось сохранить вложение: ${errorText(error)}`)
    }
  }

  return { openFound, openMailById, continueDraft, openMessage, doUnsubscribe, sendLinks, saveToNextcloud, saveAttachment }
}
