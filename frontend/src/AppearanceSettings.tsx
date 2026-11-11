import { useMemo, useState } from 'react'
import { Minus, Plus, RotateCcw, Wallpaper } from 'lucide-react'
import WallpaperDialog from './WallpaperDialog'
import { useDarkTheme } from './wallpaper'
import { cleanFont, COMPOSE_SIZES, fontInstalled, fontStack, resetAppearance, setAppearance, TEXT_MAX, TEXT_MIN, useAppearance, ZOOM_STEPS } from './appearance'

/** Распространённые шрифты; в списке — только те, что есть на этом компьютере. */
const FONTS = ['Segoe UI', 'Arial', 'Calibri', 'Verdana', 'Tahoma', 'Trebuchet MS', 'Times New Roman', 'Georgia', 'Cambria', 'PT Astra Sans', 'PT Astra Serif', 'PT Sans', 'PT Serif', 'Roboto', 'Open Sans', 'Inter', 'Noto Sans', 'Noto Serif', 'Liberation Sans', 'Liberation Serif', 'DejaVu Sans', 'Ubuntu', 'Cantarell', 'Consolas']

const ACCENTS = ['#2474ed', '#0f8a7e', '#2f9e44', '#e8590c', '#c92a2a', '#a61e4d', '#7048e8', '#495057']

/** Выбор шрифта: установленные на компьютере и «Другой…» по названию. */
function FontPick({ label, value, empty, installed, onChange }: { label: string; value: string; empty: string; installed: string[]; onChange: (v: string) => void }) {
  const known = !value || installed.includes(value)
  const [custom, setCustom] = useState(!known)
  const [text, setText] = useState(known ? '' : value)
  return (
    <span className="font-pick">
      <select aria-label={label} value={custom ? '*' : value} onChange={(e) => { if (e.target.value === '*') { setCustom(true) } else { setCustom(false); onChange(e.target.value) } }}>
        <option value="">{empty}</option>
        {installed.map((f) => <option key={f} value={f} style={{ fontFamily: fontStack(f) }}>{f}</option>)}
        <option value="*">Другой…</option>
      </select>
      {custom && (
        <input aria-label={`${label}: название`} placeholder="Название шрифта" value={text} maxLength={60}
          onChange={(e) => setText(e.target.value)} onBlur={() => onChange(cleanFont(text))} onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); onChange(cleanFont(text)) } }} />
      )}
      {custom && text && cleanFont(text) && !fontInstalled(text) && <small className="settings-hint">На этом компьютере такого шрифта нет — будет системный</small>}
    </span>
  )
}

/** Настройки → «Вид»: масштаб интерфейса и размер текста сообщений и писем. */
export default function AppearanceSettings() {
  const a = useAppearance()
  const [wall, setWall] = useState(false)
  const dark = useDarkTheme()
  const installed = useMemo(() => FONTS.filter(fontInstalled), [])
  return (
    <div className="settings-section appearance-settings">
      <strong>Вид</strong>
      <label>
        <span>Тема</span>
        <span className="segmented" role="radiogroup" aria-label="Тема">
          {([['light', 'Светлая'], ['dark', 'Тёмная'], ['system', 'Как в системе']] as const).map(([v, t]) => (
            <button key={v} type="button" role="radio" aria-checked={a.theme === v} className={a.theme === v ? 'active' : ''} onClick={() => setAppearance({ theme: v })}>{t}</button>
          ))}
        </span>
      </label>
      <label>
        <span>Цвет кнопок и выделения</span>
        <span className="accent-colors" role="radiogroup" aria-label="Цвет">
          {ACCENTS.map((c, i) => (
            <button key={c} type="button" role="radio" aria-checked={(a.accent || ACCENTS[0]) === c} aria-label={i === 0 ? 'Цвет: стандартный' : `Цвет ${c}`}
              className={`accent-color ${(a.accent || ACCENTS[0]) === c ? 'selected' : ''}`} style={{ background: c }} onClick={() => setAppearance({ accent: i === 0 ? '' : c })} />
          ))}
          <input type="color" aria-label="Свой цвет" title="Свой цвет" value={a.accent || ACCENTS[0]} onChange={(e) => setAppearance({ accent: e.target.value.toLowerCase() === ACCENTS[0] ? '' : e.target.value.toLowerCase() })} />
        </span>
      </label>
      <label>
        <span>Шрифт интерфейса</span>
        <FontPick label="Шрифт интерфейса" value={a.uiFont} empty="Системный" installed={installed} onChange={(v) => setAppearance({ uiFont: v })} />
      </label>
      <label>
        <span>Шрифт писем и сообщений</span>
        <FontPick label="Шрифт писем и сообщений" value={a.readFont} empty="Как в интерфейсе" installed={installed} onChange={(v) => setAppearance({ readFont: v })} />
        <small className="settings-hint">Письма, оформленные своим шрифтом, показываются как есть</small>
      </label>
      <label>
        <span>Масштаб интерфейса</span>
        <select aria-label="Масштаб интерфейса" value={a.zoom} onChange={(e) => setAppearance({ zoom: Number(e.target.value) })}>
          {ZOOM_STEPS.map((z) => <option key={z} value={z}>{Math.round(z * 100)} %</option>)}
        </select>
        <small className="settings-hint">Ctrl + / Ctrl − / Ctrl 0</small>
      </label>
      <label>
        <span>Текст сообщений в чате и писем</span>
        <span className="font-stepper">
          <button type="button" className="icon-button soft" aria-label="Мельче" disabled={a.text <= TEXT_MIN} onClick={() => setAppearance({ text: a.text - 1 })}><Minus size={15} /></button>
          <b aria-label="Размер текста">{a.text}</b>
          <button type="button" className="icon-button soft" aria-label="Крупнее" disabled={a.text >= TEXT_MAX} onClick={() => setAppearance({ text: a.text + 1 })}><Plus size={15} /></button>
        </span>
      </label>
      <p className="appearance-sample" style={{ fontSize: a.text, fontFamily: a.readFont ? fontStack(a.readFont) : undefined }}>Так будет выглядеть текст сообщения: «Коллеги, встреча в 15:00».</p>
      <label>
        <span>Список писем</span>
        <span className="segmented" role="radiogroup" aria-label="Список писем">
          <button type="button" role="radio" aria-checked={a.density === 'normal'} className={a.density === 'normal' ? 'active' : ''} onClick={() => setAppearance({ density: 'normal' })}>Обычно</button>
          <button type="button" role="radio" aria-checked={a.density === 'compact'} className={a.density === 'compact' ? 'active' : ''} onClick={() => setAppearance({ density: 'compact' })}>Компактно</button>
        </span>
        <small className="settings-hint">«Обычно» — с началом текста письма (если письмо хранится на компьютере)</small>
      </label>
      <label>
        <span>Шрифт новых писем</span>
        <FontPick label="Шрифт новых писем" value={a.composeFont} empty="Arial (стандартный)" installed={installed} onChange={(v) => setAppearance({ composeFont: v })} />
        <select aria-label="Размер шрифта новых писем" value={a.composeSize} onChange={(e) => setAppearance({ composeSize: Number(e.target.value) })}>
          {COMPOSE_SIZES.map((z) => <option key={z} value={z}>{z} пт</option>)}
        </select>
      </label>
      <p className="appearance-sample compose-sample" style={{ fontFamily: fontStack(a.composeFont, 'Arial, Helvetica, sans-serif'), fontSize: a.composeSize }}>Добрый день! Так будет выглядеть ваше письмо у получателя.</p>
      <small className="settings-hint">Получатель увидит этот шрифт, если он есть у него на компьютере; иначе — похожий.</small>
      <label>
        <span>Обои чата</span>
        <button type="button" className="secondary-button" onClick={() => setWall(true)}><Wallpaper size={15} /> Выбрать обои…</button>
      </label>
      {wall && <WallpaperDialog room={null} dark={dark} onClose={() => setWall(false)} />}
      <label>
        <span>Оформление</span>
        <button type="button" className="secondary-button" onClick={resetAppearance}><RotateCcw size={15} /> Вернуть как было</button>
      </label>
      <span className="settings-hint">Ширину колонок в «Почте», «Чате», «Контактах» и «Контрагентах» можно менять, перетаскивая разделитель; двойной щелчок по нему возвращает исходную.</span>
    </div>
  )
}
