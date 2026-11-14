import { ChevronLeft, SquarePen, Reply, ReplyAll, Forward, SquareKanban, Flag, Archive, FolderInput, Trash2, CircleAlert, CalendarClock, Tag, AlarmClock, Mail, ListFilter, ShieldCheck, ShieldAlert, Printer, ExternalLink, MoreHorizontal, PanelRightClose } from 'lucide-react'
import { type MenuItem } from './ContextMenu'
import { printMessage } from './mailTools'
import { initials, senderLabel } from './mailLabels'
import AddressLine from './AddressLine'
import { formatFullDate, NC_MAIL_DIR, api, editNotice, errorText, messageKey } from './api'
import { AttachmentList } from './AttachmentViewer'
import MdnBar from './MdnBar'
import MessageBody from './MessageBody'
import { fontStack, type Appearance } from './appearance'
import { SkeletonReader } from './Skeleton'
import type { MessageSummary, Folder, MessageDetail, NcStatus } from './types'

export interface ReaderPaneProps {
  setReaderOpen: React.Dispatch<React.SetStateAction<boolean>>
  selected: MessageSummary | undefined
  currentFolder: Folder | undefined
  detail: MessageDetail | null
  continueDraft: (d: MessageDetail) => Promise<void>
  openComposer: (mode: "new" | "reply" | "replyAll" | "forward") => void
  ncStatus: NcStatus | null
  setTaskFor: React.Dispatch<React.SetStateAction<MessageSummary | null>>
  toggleFlag: (message: MessageSummary, flag: "seen" | "flagged" | "important") => Promise<void>
  archive: (items: MessageSummary[]) => Promise<void>
  setMovePicker: React.Dispatch<React.SetStateAction<MessageSummary[] | null>>
  requestDelete: (items: MessageSummary[], force?: boolean) => void
  setPopMenu: React.Dispatch<React.SetStateAction<{ x: number; y: number; items: MenuItem[]; } | null>>
  dueMenuItems: (items: MessageSummary[]) => MenuItem[]
  categoryMenuItems: (items: MessageSummary[]) => MenuItem[]
  snoozeMenuItems: (items: MessageSummary[]) => MenuItem[]
  setFilterSeed: React.Dispatch<React.SetStateAction<MessageSummary | null>>
  setShowFilters: React.Dispatch<React.SetStateAction<boolean>>
  markSpam: (items: MessageSummary[], spam: boolean) => Promise<void>
  openInWindow: (m: MessageSummary) => void
  composeTo: (to: string) => void
  setQuery: React.Dispatch<React.SetStateAction<string>>
  openHistory: (domain: string) => void
  setNotice: React.Dispatch<React.SetStateAction<string>>
  doUnsubscribe: (d: MessageDetail) => Promise<void>
  saveAttachment: (index: number) => Promise<void>
  saveToNextcloud: (index: number, name: string) => Promise<void>
  myAddress: string
  setDetail: React.Dispatch<React.SetStateAction<MessageDetail | null>>
  patchMessage: (key: string, patch: Partial<MessageSummary>) => void
  appearance: Appearance
  dark: boolean
  detailLoading: boolean
  detailError: string
}

export default function ReaderPane({ setReaderOpen, selected, currentFolder, detail, continueDraft, openComposer, ncStatus, setTaskFor, toggleFlag, archive, setMovePicker, requestDelete, setPopMenu, dueMenuItems, categoryMenuItems, snoozeMenuItems, setFilterSeed, setShowFilters, markSpam, openInWindow, composeTo, setQuery, openHistory, setNotice, doUnsubscribe, saveAttachment, saveToNextcloud, myAddress, setDetail, patchMessage, appearance, dark, detailLoading, detailError }: ReaderPaneProps) {
  return (
    <section className="reader-pane">
      <div className="reader-toolbar">
        <button className="icon-button soft mobile-back" onClick={() => setReaderOpen(false)} aria-label="Назад"><ChevronLeft /></button>
        <div className="reader-toolbar-spacer" />
        {selected && <>
          {currentFolder?.role === 'drafts' && detail && <button className="primary-button small continue-draft" onClick={() => void continueDraft(detail)} title="Дописать и отправить этот черновик"><SquarePen size={15} /> Продолжить</button>}
          <button className="icon-button" aria-label="Ответить" title="Ответить" disabled={!detail} onClick={() => openComposer('reply')}><Reply size={18} /></button>
          <button className="icon-button" aria-label="Ответить всем" title="Ответить всем" disabled={!detail} onClick={() => openComposer('replyAll')}><ReplyAll size={18} /></button>
          <button className="icon-button" aria-label="Переслать" title="Переслать" disabled={!detail} onClick={() => openComposer('forward')}><Forward size={18} /></button>
          {ncStatus?.connected && <button className="icon-button" aria-label="Создать задачу" title="Создать задачу Deck из письма: срок, организация, документы в Nextcloud" onClick={() => setTaskFor(selected)}><SquareKanban size={18} /></button>}
          <span className="toolbar-sep" />
          <button className={`icon-button ${selected.flagged ? 'flag-on' : ''}`} aria-label="Флажок" aria-pressed={selected.flagged} title={selected.flagged ? 'Снять флажок' : 'Поставить флажок (к исполнению)'} onClick={() => void toggleFlag(selected, 'flagged')}>
            <Flag size={18} fill={selected.flagged ? 'currentColor' : 'none'} />
          </button>
          <span className="toolbar-sep" />
          <button className="icon-button" aria-label="Архивировать" title="В архив" onClick={() => void archive([selected])}><Archive size={18} /></button>
          <button className="icon-button" aria-label="Переместить в папку" title="Переместить в папку…" onClick={() => setMovePicker([selected])}>
            <FolderInput size={18} />
          </button>
          <button className="icon-button danger-icon" aria-label="Удалить письмо" title="Удалить (Del)" onClick={() => requestDelete([selected])}>
            <Trash2 size={18} />
          </button>
          <span className="toolbar-sep" />
          {/* Редкие действия — в меню «Ещё»: на панели остаются частые (как в Outlook). */}
          <button className="icon-button" aria-label="Ещё действия" title="Ещё: важное, срок, категории, отложить, правило, спам, печать…" onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect()
            const at = { x: Math.max(8, r.right - 260), y: r.bottom + 4 }
            // Пункт с подменю: открыть его на месте этого меню, когда оно закроется.
            const sub = (items: MenuItem[]) => () => window.setTimeout(() => setPopMenu({ ...at, items }), 0)
            setPopMenu({ ...at, items: [
              { label: selected.important ? 'Снять «Важное»' : 'Пометить как важное', icon: <CircleAlert size={15} />, onSelect: () => void toggleFlag(selected, 'important') },
              { label: 'Срок флажка…', icon: <CalendarClock size={15} />, onSelect: sub(dueMenuItems([selected])) },
              { label: 'Категории…', icon: <Tag size={15} />, onSelect: sub(categoryMenuItems([selected])) },
              { label: 'Отложить…', icon: <AlarmClock size={15} />, onSelect: sub(snoozeMenuItems([selected])) },
              { label: selected.unread ? 'Отметить прочитанным' : 'Отметить непрочитанным', icon: <Mail size={15} />, onSelect: () => void toggleFlag(selected, 'seen') },
              { separator: true },
              { label: 'Создать правило…', icon: <ListFilter size={15} />, onSelect: () => { setFilterSeed(selected); setShowFilters(true) } },
              currentFolder?.role === 'junk'
                ? { label: 'Не спам', icon: <ShieldCheck size={15} />, onSelect: () => void markSpam([selected], false) }
                : { label: 'Это спам', icon: <ShieldAlert size={15} />, onSelect: () => void markSpam([selected], true) },
              { separator: true },
              { label: 'Печать или PDF', icon: <Printer size={15} />, disabled: !detail, onSelect: () => { if (detail) printMessage(detail) } },
              { label: 'Открыть в отдельном окне', icon: <ExternalLink size={15} />, onSelect: () => openInWindow(selected) },
            ] })
          }}><MoreHorizontal size={18} /></button>
        </>}
        <button className="icon-button soft" aria-label="Скрыть чтение" onClick={() => setReaderOpen(false)}><PanelRightClose size={18} /></button>
      </div>
      {detail ? <div className="reader-content">
        <div className="reader-title-row"><h1>{detail.subject || '(без темы)'}</h1></div>
        <div className="sender-block">
          <div className="sender-avatar sender-avatar-large">{initials(senderLabel(detail))}</div>
          <div className="sender-details">
            <span className="sender-line"><AddressLine strong entries={[detail.fromName ? `${detail.fromName} <${detail.fromAddress}>` : detail.fromAddress]} onCompose={composeTo} onSearch={setQuery} onHistory={openHistory} onNotice={setNotice} /></span>
            {detail.to.length > 0 && <span>Кому: <AddressLine entries={detail.to} onCompose={composeTo} onSearch={setQuery} onHistory={openHistory} onNotice={setNotice} /></span>}
            {detail.cc.length > 0 && <span>Копия: <AddressLine entries={detail.cc} onCompose={composeTo} onSearch={setQuery} onHistory={openHistory} onNotice={setNotice} /></span>}
            {detail.unsubscribe && <span className="unsub-bar">Это рассылка · <button className="text-button" onClick={() => void doUnsubscribe(detail)}>Отписаться</button></span>}
            <small>{formatFullDate(detail.date)}</small>
          </div>
        </div>
        {detail.attachments.length > 0 && <AttachmentList
          key={`${detail.folder}/${detail.uid}`}
          folder={detail.folder}
          uid={detail.uid}
          attachments={detail.attachments}
          onSave={(i) => void saveAttachment(i)}
          onCloud={ncStatus?.connected ? (i, name) => void saveToNextcloud(i, name) : undefined}
          cloudTitle={`Сохранить в Nextcloud (${NC_MAIL_DIR})`}
          onNotice={setNotice}
          onEdit={ncStatus?.connected ? (i, name) => { setNotice(`Открываю «${name}»…`); api.ncEditAttachment(detail.folder, detail.uid, i, NC_MAIL_DIR).then((r) => setNotice(editNotice(r, name))).catch((e) => setNotice(`Не удалось открыть в редакторе: ${errorText(e)}`)) } : undefined}
        />}
        <MdnBar detail={detail} me={myAddress} onNotice={setNotice} onDone={() => { setDetail((d) => d && { ...d, keywords: [...d.keywords, '$MDNSent'] }); patchMessage(messageKey(detail), { keywords: [...detail.keywords, '$MDNSent'] }) }} />
        <MessageBody detail={detail} scale={appearance.text / 14} dark={dark} font={appearance.readFont ? fontStack(appearance.readFont) : ''} onNotice={setNotice} />
        <div className="reader-actions">
          <button className="primary-button" onClick={() => openComposer('reply')}><Reply size={16} /> Ответить</button>
          <button className="secondary-button" onClick={() => openComposer('replyAll')}><ReplyAll size={16} /> Ответить всем</button>
          <button className="secondary-button" onClick={() => openComposer('forward')}><Forward size={16} /> Переслать</button>
        </div>
      </div> : <div className="empty-reader">
        {detailLoading ? <SkeletonReader />
          : detailError ? <><strong>Не удалось открыть письмо</strong><span>{detailError}</span></>
          : <><Mail size={42} /><strong>Выберите письмо</strong><span>Выберите сообщение в списке для просмотра.</span></>}
      </div>}
    </section>
  )
}
