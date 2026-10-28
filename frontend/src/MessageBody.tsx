import { useEffect, useRef, useState } from 'react'
import { ImageOff } from 'lucide-react'

import { api, errorText } from './api'
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

  useEffect(() => {
    setHtml(detail.bodyHtml)
    setRemoteLeft(detail.remoteImages)
    setAsText(false)
  }, [detail])

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
      <span>Картинки из интернета скрыты ({remoteLeft}) — так отправитель не узнает, что письмо открыто.</span>
      <button className="text-button" disabled={loadingImages} onClick={() => void showImages()}>{loadingImages ? 'Загрузка…' : 'Показать картинки'}</button>
    </div>}
    <HtmlFrame html={html} scale={scale} />
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

function HtmlFrame({ html, scale }: { html: string; scale: number }) {
  const frame = useRef<HTMLIFrameElement>(null)
  const [height, setHeight] = useState(200)
  const doc = `<!doctype html><html><head><meta charset="utf-8"><style>${FRAME_CSS} html { zoom: ${scale}; }</style></head><body>${html}</body></html>`

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
    function onLoad() {
      const d = el?.contentDocument
      if (!d) return
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
        const href = a.getAttribute('href') ?? ''
        if (/^(https?:|mailto:)/i.test(href)) void api.openLink(href).catch(() => undefined)
      })
    }
    el.addEventListener('load', onLoad)
    return () => { el.removeEventListener('load', onLoad); observer?.disconnect() }
  }, [html, scale])

  return (
    <iframe
      ref={frame}
      className="message-html"
      title="Текст письма"
      sandbox="allow-same-origin"
      srcDoc={doc}
      style={{ height }}
    />
  )
}
