import { useEffect, useRef, useState } from 'react'
import {
  ChevronLeft, ChevronRight, Cloud, Download, File, FileText, Image as ImageIcon, Maximize2, Minimize2, X,
} from 'lucide-react'

import { api, errorText, formatSize } from './api'
import type { AttachmentInfo } from './types'

export type PreviewKind = 'image' | 'pdf' | 'text' | null

const IMAGE_EXT = /\.(png|jpe?g|gif|webp|bmp|svg|ico)$/i
const TEXT_EXT = /\.(txt|log|csv|tsv|json|xml|md|ini|conf|cfg|ya?ml|sh|bat|ps1|sql|py|js|ts|html?|css|eml)$/i

export function previewKind(a: Pick<AttachmentInfo, 'name' | 'contentType'>): PreviewKind {
  const t = a.contentType.toLowerCase()
  if (t.startsWith('image/') && !t.includes('tiff')) return 'image'
  if (IMAGE_EXT.test(a.name)) return 'image'
  if (t === 'application/pdf' || /\.pdf$/i.test(a.name)) return 'pdf'
  if (t.startsWith('text/') || t === 'application/json' || t === 'application/xml' || t === 'message/rfc822' || TEXT_EXT.test(a.name)) return 'text'
  return null
}

function mimeFor(a: AttachmentInfo): string {
  const t = a.contentType.toLowerCase()
  if (t.startsWith('image/')) return t
  const ext = a.name.split('.').pop()?.toLowerCase() ?? ''
  return { jpg: 'image/jpeg', jpeg: 'image/jpeg', png: 'image/png', gif: 'image/gif', webp: 'image/webp', bmp: 'image/bmp', svg: 'image/svg+xml', ico: 'image/x-icon' }[ext] ?? 'image/png'
}

/** Кэш содержимого вложений на время работы окна: повторный просмотр не грузит файл заново. */
const cache = new Map<string, Promise<string>>()
const MAX_CACHE = 40

export function loadAttachment(folder: string, uid: number, index: number): Promise<string> {
  const key = `${folder}\u0000${uid}\u0000${index}`
  let p = cache.get(key)
  if (!p) {
    p = api.attachmentData(folder, uid, index).then((r) => r.data)
    p.catch(() => cache.delete(key))
    cache.set(key, p)
    if (cache.size > MAX_CACHE) cache.delete(cache.keys().next().value as string)
  }
  return p
}

function bytesOf(b64: string): Uint8Array {
  const bin = atob(b64)
  const out = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i)
  return out
}

function decodeText(bytes: Uint8Array): string {
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(bytes)
  } catch {
    // Старые файлы из Windows — чаще всего cp1251.
    return new TextDecoder('windows-1251').decode(bytes)
  }
}

/** Миниатюра картинки во вложении (загружается сама, без нажатий). */
export function Thumbnail({ folder, uid, a }: { folder: string; uid: number; a: AttachmentInfo }) {
  const [src, setSrc] = useState<string | null>(null)
  useEffect(() => {
    let alive = true
    if (previewKind(a) === 'image' && a.size <= 15 * 1024 * 1024) {
      loadAttachment(folder, uid, a.index).then((d) => { if (alive) setSrc(`data:${mimeFor(a)};base64,${d}`) }).catch(() => undefined)
    }
    return () => { alive = false }
  }, [folder, uid, a])
  const kind = previewKind(a)
  if (src) return <img className="attachment-thumb" src={src} alt="" />
  return <div className="attachment-icon">{kind === 'image' ? <ImageIcon size={21} /> : kind ? <FileText size={21} /> : <File size={21} />}</div>
}

interface ListProps {
  folder: string
  uid: number
  attachments: AttachmentInfo[]
  onSave: (index: number) => void
  onCloud?: (index: number, name: string) => void
  cloudTitle?: string
}

/** Вложения письма: миниатюры картинок, щелчок — просмотр, отдельная кнопка — сохранить. */
export function AttachmentList({ folder, uid, attachments, onSave, onCloud, cloudTitle }: ListProps) {
  const [open, setOpen] = useState<number | null>(null)
  const previewable = attachments.filter((a) => previewKind(a))
  return (
    <>
      <div className="attachment-list">
        {attachments.map((a) => {
          const kind = previewKind(a)
          return (
            <div key={a.index} className={`attachment-item ${kind === 'image' ? 'has-thumb' : ''}`}>
              <button
                className="attachment-card"
                title={kind ? `Просмотр: ${a.name}` : `Сохранить ${a.name}`}
                onClick={() => (kind ? setOpen(a.index) : onSave(a.index))}
              >
                <Thumbnail folder={folder} uid={uid} a={a} />
                <div><strong>{a.name}</strong><span>{formatSize(a.size)} · {kind ? 'просмотр' : a.contentType}</span></div>
              </button>
              <button className="icon-button soft attachment-save" aria-label={`Сохранить ${a.name}`} title="Сохранить на компьютер" onClick={() => onSave(a.index)}><Download size={17} /></button>
              {onCloud && <button className="icon-button soft attachment-cloud" aria-label={`Сохранить ${a.name} в Nextcloud`} title={cloudTitle} onClick={() => onCloud(a.index, a.name)}><Cloud size={17} /></button>}
            </div>
          )
        })}
      </div>
      {open !== null && (
        <AttachmentViewer
          folder={folder}
          uid={uid}
          items={previewable}
          index={open}
          onIndex={setOpen}
          onClose={() => setOpen(null)}
          onSave={onSave}
        />
      )}
    </>
  )
}

interface ViewerProps {
  folder: string
  uid: number
  items: AttachmentInfo[]
  index: number
  onIndex: (index: number) => void
  onClose: () => void
  onSave: (index: number) => void
}

export default function AttachmentViewer({ folder, uid, items, index, onIndex, onClose, onSave }: ViewerProps) {
  const pos = Math.max(0, items.findIndex((a) => a.index === index))
  const a = items[pos]
  const kind = a ? previewKind(a) : null
  const [data, setData] = useState<string | null>(null)
  const [error, setError] = useState('')
  const [actual, setActual] = useState(false)

  useEffect(() => {
    if (!a) return
    let alive = true
    setData(null)
    setError('')
    setActual(false)
    loadAttachment(folder, uid, a.index).then((d) => { if (alive) setData(d) }).catch((e) => { if (alive) setError(errorText(e)) })
    return () => { alive = false }
  }, [folder, uid, a])

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape') { e.stopPropagation(); onClose() }
      else if (e.key === 'ArrowRight' && pos < items.length - 1) onIndex(items[pos + 1].index)
      else if (e.key === 'ArrowLeft' && pos > 0) onIndex(items[pos - 1].index)
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [pos, items, onIndex, onClose])

  if (!a) return null
  return (
    <div className="viewer-backdrop" role="dialog" aria-label={`Просмотр ${a.name}`} onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}>
      <div className="viewer-head">
        <strong title={a.name}>{a.name}</strong>
        <span>{formatSize(a.size)}{items.length > 1 ? ` · ${pos + 1} из ${items.length}` : ''}</span>
        <div className="reader-toolbar-spacer" />
        {kind === 'image' && <button className="icon-button" aria-label={actual ? 'Вписать в окно' : 'Реальный размер'} title={actual ? 'Вписать в окно' : 'Реальный размер'} onClick={() => setActual(!actual)}>{actual ? <Minimize2 size={18} /> : <Maximize2 size={18} />}</button>}
        <button className="icon-button" aria-label="Сохранить" title="Сохранить на компьютер" onClick={() => onSave(a.index)}><Download size={18} /></button>
        <button className="icon-button" aria-label="Закрыть просмотр" title="Закрыть (Esc)" onClick={onClose}><X size={20} /></button>
      </div>
      <div className={`viewer-body ${kind}-view ${actual ? 'actual' : ''}`} onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}>
        {error ? <div className="viewer-message"><strong>Не удалось открыть файл</strong><span>{error}</span></div>
          : !data ? <div className="viewer-message"><span>Загрузка…</span></div>
          : kind === 'image' ? <img className="viewer-image" src={`data:${mimeFor(a)};base64,${data}`} alt={a.name} onClick={() => setActual(!actual)} />
          : kind === 'text' ? <pre className="viewer-text">{decodeText(bytesOf(data))}</pre>
          : kind === 'pdf' ? <PdfView data={data} />
          : null}
      </div>
      {pos > 0 && <button className="viewer-nav prev" aria-label="Предыдущий файл" onClick={() => onIndex(items[pos - 1].index)}><ChevronLeft size={28} /></button>}
      {pos < items.length - 1 && <button className="viewer-nav next" aria-label="Следующий файл" onClick={() => onIndex(items[pos + 1].index)}><ChevronRight size={28} /></button>}
    </div>
  )
}

const MAX_PDF_PAGES = 60

/** PDF рисуется pdf.js постранично (работает одинаково в WebView2 и WebKitGTK). */
function PdfView({ data }: { data: string }) {
  const host = useRef<HTMLDivElement>(null)
  const [state, setState] = useState<{ pages: number; error: string }>({ pages: 0, error: '' })

  useEffect(() => {
    let cancelled = false
    const el = host.current
    if (!el) return
    el.innerHTML = ''
    void (async () => {
      try {
        const pdfjs = await import('pdfjs-dist/legacy/build/pdf')
        const worker = (await import('pdfjs-dist/legacy/build/pdf.worker.min.js?url')).default
        pdfjs.GlobalWorkerOptions.workerSrc = worker
        // isEvalSupported: false — шрифты PDF не исполняются как код (CVE-2024-4367).
        const doc = await pdfjs.getDocument({ data: bytesOf(data), isEvalSupported: false }).promise
        if (cancelled) return
        setState({ pages: doc.numPages, error: '' })
        const width = Math.min(el.clientWidth || 900, 1100) - 8
        for (let n = 1; n <= Math.min(doc.numPages, MAX_PDF_PAGES); n++) {
          const page = await doc.getPage(n)
          if (cancelled) return
          const base = page.getViewport({ scale: 1 })
          const scale = width / base.width
          const ratio = window.devicePixelRatio || 1
          const vp = page.getViewport({ scale: scale * ratio })
          const canvas = document.createElement('canvas')
          canvas.className = 'pdf-page'
          canvas.width = vp.width
          canvas.height = vp.height
          canvas.style.width = `${vp.width / ratio}px`
          el.appendChild(canvas)
          await page.render({ canvasContext: canvas.getContext('2d')!, viewport: vp }).promise
        }
        void doc.destroy()
      } catch (e) {
        if (!cancelled) setState((s) => ({ ...s, error: errorText(e) }))
      }
    })()
    return () => { cancelled = true }
  }, [data])

  return (
    <div className="pdf-view">
      {state.error && <div className="viewer-message"><strong>PDF не открылся</strong><span>{state.error}</span></div>}
      <div ref={host} className="pdf-pages" />
      {state.pages > MAX_PDF_PAGES && <div className="viewer-message"><span>Показаны первые {MAX_PDF_PAGES} страниц из {state.pages} — весь файл откройте после сохранения.</span></div>}
    </div>
  )
}
