import { useState } from 'react'
import { Minus, Plus, Wallpaper } from 'lucide-react'
import WallpaperDialog from './WallpaperDialog'
import { useDarkTheme } from './wallpaper'
import { setAppearance, TEXT_MAX, TEXT_MIN, useAppearance, ZOOM_STEPS } from './appearance'

/** Настройки → «Вид»: масштаб интерфейса и размер текста сообщений и писем. */
export default function AppearanceSettings() {
  const a = useAppearance()
  const [wall, setWall] = useState(false)
  const dark = useDarkTheme()
  return (
    <div className="settings-section appearance-settings">
      <strong>Вид</strong>
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
      <p className="appearance-sample" style={{ fontSize: a.text }}>Так будет выглядеть текст сообщения: «Коллеги, встреча в 15:00».</p>
      <label>
        <span>Обои чата</span>
        <button type="button" className="secondary-button" onClick={() => setWall(true)}><Wallpaper size={15} /> Выбрать обои…</button>
      </label>
      {wall && <WallpaperDialog room={null} dark={dark} onClose={() => setWall(false)} />}
      <span className="settings-hint">Ширину колонок в «Почте», «Чате», «Контактах» и «Контрагентах» можно менять, перетаскивая разделитель; двойной щелчок по нему возвращает исходную.</span>
    </div>
  )
}
