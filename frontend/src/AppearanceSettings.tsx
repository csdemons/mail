import { Minus, Plus } from 'lucide-react'
import { setAppearance, TEXT_MAX, TEXT_MIN, useAppearance, ZOOM_STEPS } from './appearance'

/** Настройки → «Вид»: масштаб интерфейса и размер текста сообщений и писем. */
export default function AppearanceSettings() {
  const a = useAppearance()
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
      <span className="settings-hint">Ширину колонок в «Почте», «Чате», «Контактах» и «Контрагентах» можно менять, перетаскивая разделитель; двойной щелчок по нему возвращает исходную.</span>
    </div>
  )
}
