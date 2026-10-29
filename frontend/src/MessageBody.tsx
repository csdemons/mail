import { useEffect, useRef, useState } from 'react'
import { ImageOff } from 'lucide-react'

import { api, errorText, linkMismatch } from './api'
import type { MessageDetail } from './types'

/** Письмо в HTML показывается как задумал отправитель — в изолированной рамке
 *  (без скриптов, без обращений в сеть); иначе — обычным текстом. */
export default function MessageBody({ detail, scale = 1, onNotice }: {
  detail: MessageDetail
  scale?: number
  onNotice?: (text: string) => void
}) {
  const [html, setHtml] = useState(detail.bodyHtml)
  const [remoteLeft, setRemoteLeft] = useState(detail.remoteImages)
  const [asText, setAsText] = useState(false)
  const [loadingImages, setLoadingImages] = useState(false)
  const [imagesPolicy, setImagesPolicy] = useState<'direct' | 'proxy' | 'off'>('direct')
  useEffect(() => { api.getPolicy().then((p) => setImagesPolicy(p.remoteImages)).catch(() => undefined) }, [])

  useEffect(() => {
    setHtml(detail.bodyHtml)
    setRemoteLeft(detail.remoteImages)
    setAsText(false)
  }, [detail])

  const [hover, setHover] = useState('')
  const [suspicious, setSuspicious] = useState<{ href: string; real: string; shown: string } | null>(null)
  /** Открыть ссылку из письма: если текст ссылки — другой домен, сначала спросить. */
  function openChecked(href: string, text: string) {
    if (!/^(https?:|mailto:)/i.test(href)) return
    const m = /^https?:/i.test(href) ? linkMismatch(href, text) : null
    if (m) setSuspicious({ href, ...m })
    else void api.openLink(href).catch(() => undefined)
  }

  async function showImages() {
    setLoadingImages(true)
    try {
      setHtml(await api.messageHtmlRemote(detail.folder, detail.uid))
      setRemoteLeft(0)
    } catch (e) {
      onNotice?.(`Не удалось загрузить картинки: ${errorText(e)}`)
    } finally {
      setLoadingImages(false)
    }
  }

  if (!html || asText) {
    return <>
      {html && <div className="html-bar"><span>Показан текст письма</span><button className="text-button" onClick={() => setAsText(false)}>Показать оформление</button></div>}
      <article className="message-body plain" style={scale !== 1 ? { fontSize: 15 * scale } : undefined}>{detail.bodyText || '(пустое письмо)'}</article>
    </>
  }
  return <>
    {remoteLeft > 0 && <div className="html-bar remote">
      <ImageOff size={15} />
      {imagesPolicy === 'off'
        ? <span>Картинки из интернета скрыты ({remoteLeft}) — загрузка отключена администратором.</span>
        : <>
          <span>Картинки из интернета скрыты ({remoteLeft}) — так отправитель не узнает, что письмо открыто.</span>
          <button className="text-button" disabled={loadingImages} onClick={() => void showImages()}>{loadingImages ? 'Загрузка…' : 'Показать картинки'}</button>
        </>}
    </div>}
    <HtmlFrame html={html} scale={scale} onHover={setHover} onLink={openChecked} />
    {hover && <div className="link-status" title={hover}>{hover}</div>}
    {suspicious && <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) setSuspicious(null) }}>
      <div className="settings-modal confirm-modal" role="alertdialog" aria-label="Подозрительная ссылка">
        <strong>Ссылка ведёт не туда, куда написано</strong>
        <p>В тексте письма — <b>{suspicious.shown}</b>, а на самом деле ссылка открывает <b>{suspicious.real}</b>. Так часто выглядят фишинговые письма.</p>
        <p className="link-full">{suspicious.href}</p>
        <div className="settings-actions">
          <button className="primary-button" autoFocus onClick={() => setSuspicious(null)}>Не открывать</button>
          <button className="secondary-button" onClick={() => { void api.openLink(suspicious.href).catch(() => undefined); setSuspicious(null) }}>Всё равно открыть</button>
        </div>
      </div>
    </div>}
    <div className="html-foot"><button className="text-button" onClick={() => setAsText(true)}>Показать как текст</button></div>
  </>
}

const FRAME_CSS = `
html { background: #fff; }
body { margin: 0; padding: 14px 16px; color: #111827; font: 14px/1.5 "Segoe UI", Arial, "Liberation Sans", sans-serif; overflow-wrap: anywhere; }
img { max-width: 100%; }
pre { white-space: pre-wrap; }
blockquote { margin: 0 0 0 6px; padding-left: 10px; border-left: 3px solid #cbd5e1; color: #475569; }
a { color: #1d4ed8; }
`

const FRAME_CSP = "default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:; script-src 'none'; object-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'; connect-src 'none'"

function HtmlFrame({ html, scale, onHover, onLink }: { html: string; scale: number; onHover: (href: string) => void; onLink: (href: string, text: string) => void }) {
  const frame = useRef<HTMLIFrameElement>(null)
  // Обработчики — через ref: иначе каждое наведение мыши перезапускало бы подписки рамки.
  const cb = useRef({ onHover, onLink })
  cb.current = { onHover, onLink }
  const [height, setHeight] = useState(200)
  // Своя политика CSP рамки письма — строже, чем у окна: никаких скриптов, сети, форм,
  // встраивания; картинки — только data: (внешние программа подставляет сама).
  // Атрибут sandbox не используется: в WebKitGTK с ним не срабатывают обработчики
  // щелчков, и ссылки из писем нельзя было бы проверить и открыть в браузере.
  const doc = `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${FRAME_CSP}"><style>${FRAME_CSS} html { zoom: ${scale}; }</style></head><body>${html}</body></html>`

  useEffect(() => {
    const el = frame.current
    if (!el) return
    let observer: ResizeObserver | null = null
    function measure() {
      const d = el?.contentDocument
      if (!d?.documentElement) return
      // С zoom движки считают по-разному — берём наибольшее; не хватит — рамка прокрутится сама.
      const h = Math.max(d.documentElement.scrollHeight, d.body?.getBoundingClientRect().bottom ?? 0, d.documentElement.getBoundingClientRect().height)
      setHeight(Math.max(60, Math.ceil(h) + 4))
    }
    const bound = new WeakSet<Document>()
    function onLoad() {
      const d = el?.contentDocument
      if (!d || bound.has(d)) return
      bound.add(d)
      measure()
      observer?.disconnect()
      observer = new ResizeObserver(measure)
      if (d.body) observer.observe(d.body)
      d.querySelectorAll('img').forEach((img) => img.addEventListener('load', measure))
      // Ссылки — в браузере (http/https) или почтовой программе по умолчанию (mailto).
      d.addEventListener('click', (e) => {
        const a = (e.target as Element | null)?.closest?.('a')
        if (!a) return
        e.preventDefault()
        cb.current.onLink(a.getAttribute('href') ?? '', a.textContent ?? '')
      })
      // Реальный адрес ссылки под письмом — как строка состояния браузера.
      d.addEventListener('mouseover', (e) => {
        const a = (e.target as Element | null)?.closest?.('a')
        cb.current.onHover(a?.getAttribute('href') ?? '')
      })
      d.addEventListener('mouseleave', () => cb.current.onHover(''))
    }
    el.addEventListener('load', onLoad)
    // srcdoc может загрузиться раньше, чем подписались на «load», — тогда сразу.
    const ready = el.contentDocument
    if (ready?.readyState === 'complete' && ready.body && ready.body.childNodes.length > 0) onLoad()
    return () => { el.removeEventListener('load', onLoad); observer?.disconnect() }
  }, [html, scale])

  return (
    <iframe
      ref={frame}
      className="message-html"
      title="Текст письма"
      srcDoc={doc}
      style={{ height }}
    />
  )
}
