// Разбор вставки из буфера обмена: файл или картинка — вложение, обычный текст — как есть.
// Правило одно для письма и чата, поэтому вынесено из общего кода (без DOM и React — тестируется).

/** Что событие вставки отдало из буфера обмена. */
export interface PasteData {
  files: File[]
  types: string[]
  text: string
  uris: string
}

/** Это вставка файла или картинки, а не обычного текста. */
export function isFilePaste(d: PasteData): boolean {
  // Снимок экрана или картинка из браузера: WebView отдаёт их файлами.
  if (d.files.length > 0) return true
  if (!d.text) return false
  // Файл, скопированный в проводнике: путь в text/uri-list (file://…).
  if (/^file:\/\//im.test(d.uris)) return true
  // Некоторые файловые менеджеры кладут путь обычным текстом, но с пометкой uri-list.
  return d.types.includes('text/uri-list') && /^(\/|file:\/\/)\S+$/m.test(d.text)
}

/** Имя картинки без имени: «Снимок 24-11-2026 22-40-05.png». */
export function pastedName(name: string, type: string, now = new Date()): string {
  if (name && name !== 'image.png') return name
  const ext = (type.split('/')[1] || 'png').replace('jpeg', 'jpg').replace(/[^a-z0-9]/gi, '') || 'png'
  const stamp = now.toLocaleString('ru-RU').replace(/[^\d]+/g, '-').replace(/-$/, '')
  return `Снимок ${stamp}.${ext}`
}
