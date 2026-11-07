import { useRef, useState } from 'react'
import { Check, ImagePlus, X } from 'lucide-react'
import { clearOwn, hasOwn, imageUrl, PRESETS, setWallpaper, storeImage, wallpaperFor, wallpaperStyle, type Wallpaper } from './wallpaper'

/**
 * «Обои…»: готовые фоны или своя картинка (затемнение, размытие); для всех бесед или
 * только для одной. Настройка — у каждого пользователя своя, на этом компьютере.
 */
export default function WallpaperDialog({ room, dark, onClose }: { room: { token: string; name: string } | null; dark: boolean; onClose: () => void }) {
  const [scope, setScope] = useState<'all' | 'room'>(room ? 'room' : 'all')
  const start = wallpaperFor(scope === 'room' && room ? room.token : null)
  const [w, setW] = useState<Wallpaper | null>(start)
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const fileRef = useRef<HTMLInputElement>(null)
  const preview = wallpaperStyle(w, dark)

  function switchScope(next: 'all' | 'room') {
    setScope(next)
    setW(wallpaperFor(next === 'room' && room ? room.token : null))
  }
  async function pickImage(f: File | undefined) {
    if (!f) return
    setBusy(true)
    setError('')
    try {
      const img = await storeImage(f)
      setW({ kind: 'image', img, dim: 25, blur: 0 })
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
      if (fileRef.current) fileRef.current.value = ''
    }
  }
  function apply() {
    setWallpaper(scope === 'room' && room ? room.token : 'all', w)
    onClose()
  }

  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}>
      <div className="settings-modal wallpaper-modal" role="dialog" aria-label="Обои чата">
        <div className="compose-header"><div><strong>Обои чата</strong><small>Видны только вам, на этом компьютере</small></div><button className="icon-button" onClick={onClose} aria-label="Закрыть"><X size={18} /></button></div>
        {room && (
          <div className="segmented wallpaper-scope" role="radiogroup" aria-label="Для каких бесед">
            <label><input type="radio" name="wp-scope" checked={scope === 'room'} onChange={() => switchScope('room')} /> Только «{room.name}»</label>
            <label><input type="radio" name="wp-scope" checked={scope === 'all'} onChange={() => switchScope('all')} /> Все беседы</label>
          </div>
        )}
        <div className="wallpaper-preview" aria-label="Предпросмотр">
          {preview && <div className="talk-wallpaper" style={preview.layer} />}
          {preview?.shade && <div className="talk-wallpaper-shade" style={preview.shade} />}
          <div className="wallpaper-sample">
            <div className="talk-msg"><div className="talk-bubble"><span className="talk-author">Коллега</span><div className="talk-text">Добрый день! Документы во вложении.</div></div></div>
            <div className="talk-msg own"><div className="talk-bubble"><div className="talk-text">Спасибо, посмотрю сегодня.</div></div></div>
          </div>
        </div>
        <div className="wallpaper-grid" role="radiogroup" aria-label="Готовые фоны">
          <button type="button" className={`wallpaper-tile none ${w === null ? 'current' : ''}`} aria-label="Без обоев" aria-pressed={w === null} onClick={() => setW(null)}>Без обоев</button>
          {PRESETS.map((p) => {
            const on = w?.kind === 'preset' && w.id === p.id
            return (
              <button key={p.id} type="button" className={`wallpaper-tile ${on ? 'current' : ''}`} style={{ background: dark ? p.dark : p.light }} aria-label={p.name} aria-pressed={on} title={p.name} data-preset={p.id}
                onClick={() => setW({ kind: 'preset', id: p.id })}>
                {on && <Check size={16} />}<span>{p.name}</span>
              </button>
            )
          })}
          <button type="button" className={`wallpaper-tile own ${w?.kind === 'image' ? 'current' : ''}`} disabled={busy} onClick={() => fileRef.current?.click()}
            style={w?.kind === 'image' && imageUrl(w.img) ? { backgroundImage: `url("${imageUrl(w.img)}")`, backgroundSize: 'cover' } : undefined}>
            <ImagePlus size={18} /><span>{busy ? 'Загрузка…' : 'Своя картинка…'}</span>
          </button>
          <input ref={fileRef} type="file" accept="image/*" hidden aria-label="Файл картинки для обоев" onChange={(e) => void pickImage(e.target.files?.[0])} />
        </div>
        {w?.kind === 'image' && (
          <div className="wallpaper-sliders">
            <label><span>Затемнение</span><input type="range" min={0} max={80} step={5} value={w.dim} aria-label="Затемнение" onChange={(e) => setW({ ...w, dim: Number(e.target.value) })} /><b>{w.dim}%</b></label>
            <label><span>Размытие</span><input type="range" min={0} max={20} step={1} value={w.blur} aria-label="Размытие" onChange={(e) => setW({ ...w, blur: Number(e.target.value) })} /><b>{w.blur}px</b></label>
          </div>
        )}
        {error && <div className="login-error" role="alert">{error}</div>}
        <div className="settings-actions">
          {room && scope === 'room' && hasOwn(room.token) && <button className="text-button" onClick={() => { clearOwn(room.token); onClose() }}>Как во всех беседах</button>}
          <div className="reader-toolbar-spacer" />
          <button className="text-button" onClick={onClose}>Отмена</button>
          <button className="primary-button" disabled={busy} onClick={apply}>Применить</button>
        </div>
      </div>
    </div>
  )
}
