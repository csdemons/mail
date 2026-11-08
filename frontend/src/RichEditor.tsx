import ColorPalette from './ColorPalette'
import { forwardRef, useCallback, useEffect, useImperativeHandle, useRef, useState } from 'react'
import {
  AlignCenter, AlignLeft, AlignRight, Bold, Highlighter, IndentDecrease, IndentIncrease, Italic, Link, List,
  ListOrdered, Quote, RemoveFormatting, Strikethrough, Underline, Unlink,
} from 'lucide-react'

/** Шрифты, которые есть и в Windows, и в большинстве Linux-сборок (или заменяются похожими). */
const FONTS = ['Arial', 'Calibri', 'Times New Roman', 'Georgia', 'Verdana', 'Tahoma', 'Courier New']
/** Ступени execCommand('fontSize') 1–7; обычный текст письма (14px) — ступень 2. */
const SIZES: [string, string][] = [
  ['1', 'Мелкий'], ['2', 'Обычный'], ['3', 'Средний'], ['4', 'Крупный'], ['5', 'Очень крупный'], ['6', 'Заголовок'], ['7', 'Огромный'],
]
const CSS_SIZE: Record<string, string> = { 'x-small': '1', small: '2', medium: '3', large: '4', 'x-large': '5', 'xx-large': '6', 'xxx-large': '7' }

export interface RichEditorHandle {
  html: () => string
  text: () => string
  focus: () => void
  /** Вставляет ссылки на файлы (в место курсора или в конец текста). */
  insertLinks: (links: { name: string; url: string }[]) => void
  /** Вставляет готовый фрагмент (шаблон) в место курсора или в начало письма. */
  insertHtml: (html: string) => void
  /** Заменить подпись (блок «-- …») другой; пустая — убрать. */
  setSignature: (html: string) => void
}

/** Начало цитаты или пересылаемого письма — подпись стоит над ним. */
function isQuoteStart(n: Element): boolean {
  if (n.tagName === 'BLOCKQUOTE') return true
  const t = (n.textContent ?? '').trim()
  return t.startsWith('---------- Пересланное') || /пишет:$/.test(t)
}

/** Отметить подпись в тексте черновика (строка «-- » и строки под ней до цитаты), чтобы её можно было заменить. */
function markSignature(el: HTMLElement) {
  if (el.querySelector('[data-signature]')) return
  const kids = Array.from(el.children)
  const start = kids.findIndex((k) => k.tagName === 'DIV' && (k.textContent ?? '').replace(/\u00a0/g, ' ') === '-- ')
  if (start < 0) return
  let end = start
  while (end + 1 < kids.length && !isQuoteStart(kids[end + 1])) end++
  while (end > start && !(kids[end].textContent ?? '').trim()) end--
  const box = document.createElement('div')
  box.dataset.signature = '1'
  kids[start].before(box)
  for (const k of kids.slice(start, end + 1)) box.append(k)
}

function escapeHtml(s: string) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}

/** Текст черновика (ответ с цитатой «> ») → HTML: цитата становится blockquote. */
/** Экранирует строку и делает ссылки кликабельными. */
function linkify(line: string): string {
  return escapeHtml(line).replace(/https?:\/\/[^\s<>"]+[^\s<>".,;:!?)]/g, (u) => `<a href="${u}">${u}</a>`)
}

export function textToHtml(text: string): string {
  if (!text) return ''
  const out: string[] = []
  let quote: string[] = []
  const flush = () => {
    if (quote.length) out.push(`<blockquote>${quote.join('<br>')}</blockquote>`)
    quote = []
  }
  for (const line of text.split('\n')) {
    if (/^>\s?/.test(line)) {
      quote.push(linkify(line.replace(/^>\s?/, '')) || '&nbsp;')
    } else {
      flush()
      out.push(`<div>${linkify(line) || '<br>'}</div>`)
    }
  }
  flush()
  return out.join('')
}

interface Props {
  initialText: string
  /** Оформленный текст (возврат в редактор после отмены отправки). */
  initialHtml?: string
  autoFocus?: boolean
  onNotice?: (text: string) => void
  /** Текст изменился (для автосохранения черновика). */
  onChange?: () => void
}

/** Редактор письма на contentEditable: шрифт, размер, начертание, цвет, списки,
 *  выравнивание, цитата, ссылки. Итоговый HTML дополнительно чистится в бэкенде. */
const RichEditor = forwardRef<RichEditorHandle, Props>(function RichEditor({ initialText, initialHtml, autoFocus, onNotice, onChange }, ref) {
  const editor = useRef<HTMLDivElement>(null)
  const saved = useRef<Range | null>(null)
  const [state, setState] = useState({ bold: false, italic: false, underline: false, strike: false, ul: false, ol: false, font: '', size: '' })

  function placeCursor(el: HTMLDivElement) {
    el.focus()
    const sel = window.getSelection()
    if (saved.current && sel && el.contains(saved.current.startContainer)) {
      sel.removeAllRanges()
      sel.addRange(saved.current)
    } else if (sel) {
      const r = document.createRange()
      r.setStart(el, 0)
      r.collapse(true)
      sel.removeAllRanges()
      sel.addRange(r)
    }
  }

  useImperativeHandle(ref, () => ({
    insertHtml: (html: string) => {
      const el = editor.current
      if (!el) return
      placeCursor(el)
      document.execCommand('insertHTML', false, html)
    },
    setSignature: (html: string) => {
      const el = editor.current
      if (!el) return
      markSignature(el)
      const box = el.querySelector<HTMLElement>('[data-signature]')
      if (box) {
        if (html) box.outerHTML = html
        else box.remove()
      } else if (html) {
        // Подписи не было: над цитатой (ответ, пересылка) или в конце письма.
        const quote = Array.from(el.children).find(isQuoteStart)
        const frag = document.createRange().createContextualFragment(`<div><br></div>${html}<div><br></div>`)
        if (quote) quote.before(frag)
        else el.append(document.createRange().createContextualFragment(`<div><br></div>${html}`))
      }
      onChange?.()
    },
    html: () => editor.current?.innerHTML ?? '',
    text: () => (editor.current?.innerText ?? '').replace(/ /g, ' ').replace(/\n{3,}/g, '\n\n'),
    focus: () => editor.current?.focus(),
    insertLinks: (links) => {
      const el = editor.current
      if (!el || links.length === 0) return
      const html = links.map((l) => `<div>📎 <a href="${escapeHtml(l.url)}">${escapeHtml(l.name)}</a></div>`).join('')
      el.focus()
      const sel = window.getSelection()
      if (saved.current && sel && el.contains(saved.current.startContainer)) {
        sel.removeAllRanges()
        sel.addRange(saved.current)
      } else if (sel) {
        // Курсор не в письме — в начало, над цитатой (если это ответ).
        const r = document.createRange()
        r.setStart(el, 0)
        r.collapse(true)
        sel.removeAllRanges()
        sel.addRange(r)
      }
      document.execCommand('insertHTML', false, html)
    },
  }))

  useEffect(() => {
    const el = editor.current
    if (!el) return
    // Свой же HTML из этого редактора (после отмены отправки); при отправке он ещё раз чистится в бэкенде.
    el.innerHTML = initialHtml ?? textToHtml(initialText)
    markSignature(el)
    try { document.execCommand('styleWithCSS', false, 'true') } catch { /* старые движки */ }
    if (autoFocus) {
      el.focus()
      // Курсор в начало — над цитатой, как в почтовых клиентах.
      const r = document.createRange()
      r.setStart(el, 0)
      r.collapse(true)
      const sel = window.getSelection()
      sel?.removeAllRanges()
      sel?.addRange(r)
    }
    // Черновик задаётся один раз при открытии окна.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const refresh = useCallback(() => {
    const q = (c: string) => { try { return document.queryCommandState(c) } catch { return false } }
    const v = (c: string) => { try { return document.queryCommandValue(c) } catch { return '' } }
    const font = v('fontName').replace(/["']/g, '').split(',')[0].trim()
    const rawSize = v('fontSize')
    setState({
      bold: q('bold'), italic: q('italic'), underline: q('underline'), strike: q('strikeThrough'),
      ul: q('insertUnorderedList'), ol: q('insertOrderedList'),
      font: FONTS.find((f) => f.toLowerCase() === font.toLowerCase()) ?? '',
      size: CSS_SIZE[rawSize] ?? (SIZES.some(([k]) => k === rawSize) ? rawSize : ''),
    })
  }, [])

  useEffect(() => {
    function onSel() {
      const sel = window.getSelection()
      if (sel && sel.rangeCount > 0 && editor.current?.contains(sel.anchorNode)) {
        saved.current = sel.getRangeAt(0).cloneRange()
        refresh()
      }
    }
    document.addEventListener('selectionchange', onSel)
    return () => document.removeEventListener('selectionchange', onSel)
  }, [refresh])

  /** Возвращает выделение в редактор (выпадающие списки и выбор цвета его забирают). */
  function restore() {
    const el = editor.current
    if (!el) return
    el.focus()
    const sel = window.getSelection()
    if (saved.current && sel) {
      sel.removeAllRanges()
      sel.addRange(saved.current)
    }
  }

  const [textColor, setTextColor] = useState('#d0021b')
  const [markColor, setMarkColor] = useState('#fff59d')

  function exec(cmd: string, value?: string) {
    restore()
    try { document.execCommand('styleWithCSS', false, 'true') } catch { /* нет */ }
    document.execCommand(cmd, false, value)
    refresh()
  }

  function toggleQuote() {
    restore()
    const sel = window.getSelection()
    const inQuote = sel?.anchorNode && (sel.anchorNode instanceof Element ? sel.anchorNode : sel.anchorNode.parentElement)?.closest('blockquote')
    document.execCommand('formatBlock', false, inQuote ? 'div' : 'blockquote')
    refresh()
  }

  function link() {
    restore()
    const url = window.prompt('Адрес ссылки', 'https://')?.trim()
    if (!url || url === 'https://') return
    if (!/^(https?:|mailto:|tel:)/i.test(url)) {
      onNotice?.('Ссылка должна начинаться с https://, http://, mailto: или tel:')
      return
    }
    restore()
    const sel = window.getSelection()
    if (sel && sel.isCollapsed) {
      document.execCommand('insertHTML', false, `<a href="${escapeHtml(url)}">${escapeHtml(url)}</a>`)
    } else {
      document.execCommand('createLink', false, url)
    }
  }

  function onPaste(e: React.ClipboardEvent<HTMLDivElement>) {
    if (e.clipboardData.files.length > 0) {
      e.preventDefault()
      onNotice?.('Картинки и файлы добавьте кнопкой «Вложить файл».')
    }
  }

  // mousedown + preventDefault: кнопка не забирает фокус и выделение у текста.
  const btn = (label: string, icon: React.ReactNode, onClick: () => void, active = false) => (
    <button type="button" className={`rt-btn ${active ? 'active' : ''}`} title={label} aria-label={label} aria-pressed={active}
      onMouseDown={(e) => { e.preventDefault(); onClick() }}>{icon}</button>
  )

  return (
    <div className="rich-editor">
      <div className="rt-toolbar" role="toolbar" aria-label="Оформление текста">
        <select className="rt-font" aria-label="Шрифт" value={state.font}
          onChange={(e) => exec('fontName', e.target.value)}>
          <option value="" disabled>Шрифт</option>
          {FONTS.map((f) => <option key={f} value={f} style={{ fontFamily: f }}>{f}</option>)}
        </select>
        <select className="rt-size" aria-label="Размер" value={state.size} onChange={(e) => exec('fontSize', e.target.value)}>
          <option value="" disabled>Размер</option>
          {SIZES.map(([k, pt]) => <option key={k} value={k}>{pt}</option>)}
        </select>
        <span className="rt-sep" />
        {btn('Жирный (Ctrl+B)', <Bold size={16} />, () => exec('bold'), state.bold)}
        {btn('Курсив (Ctrl+I)', <Italic size={16} />, () => exec('italic'), state.italic)}
        {btn('Подчёркнутый (Ctrl+U)', <Underline size={16} />, () => exec('underline'), state.underline)}
        {btn('Зачёркнутый', <Strikethrough size={16} />, () => exec('strikeThrough'), state.strike)}
        <ColorPalette label="Цвет текста" icon={<span className="rt-a">A</span>} current={textColor} clearLabel="Цвет по умолчанию"
          onPick={(c) => { if (c) setTextColor(c); exec('foreColor', c ?? '#202939') }} />
        <ColorPalette label="Выделение цветом" icon={<Highlighter size={16} />} current={markColor} clearLabel="Без выделения"
          onPick={(c) => { if (c) setMarkColor(c); exec('hiliteColor', c ?? 'transparent') }} />
        <span className="rt-sep" />
        {btn('Маркированный список', <List size={16} />, () => exec('insertUnorderedList'), state.ul)}
        {btn('Нумерованный список', <ListOrdered size={16} />, () => exec('insertOrderedList'), state.ol)}
        {btn('Уменьшить отступ', <IndentDecrease size={16} />, () => exec('outdent'))}
        {btn('Увеличить отступ', <IndentIncrease size={16} />, () => exec('indent'))}
        <span className="rt-sep" />
        {btn('По левому краю', <AlignLeft size={16} />, () => exec('justifyLeft'))}
        {btn('По центру', <AlignCenter size={16} />, () => exec('justifyCenter'))}
        {btn('По правому краю', <AlignRight size={16} />, () => exec('justifyRight'))}
        <span className="rt-sep" />
        {btn('Цитата', <Quote size={16} />, toggleQuote)}
        {btn('Ссылка', <Link size={16} />, link)}
        {btn('Убрать ссылку', <Unlink size={16} />, () => exec('unlink'))}
        {btn('Очистить оформление', <RemoveFormatting size={16} />, () => exec('removeFormat'))}
      </div>
      <div
        ref={editor}
        className="compose-body rt-content"
        contentEditable
        spellCheck
        lang="ru"
        role="textbox"
        aria-multiline="true"
        aria-label="Текст письма"
        data-placeholder="Напишите сообщение…"
        onPaste={onPaste}
        onInput={() => onChange?.()}
        onKeyUp={refresh}
        onMouseUp={refresh}
      />
    </div>
  )
})

export default RichEditor
