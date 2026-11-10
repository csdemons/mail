// Данные pdf.js для просмотра PDF: карты кодировок CID-шрифтов (cmaps), стандартные шрифты
// (не встроенные в документ), декодеры картинок сканов (JPEG 2000, JBIG2 — wasm и запасные JS),
// цветовые профили. Без них такие PDF открываются белым листом. Копируются в public/pdfjs
// перед каждой сборкой (каталог не в git).
import { cpSync, existsSync, mkdirSync, rmSync } from 'node:fs'

const from = 'node_modules/pdfjs-dist'
const to = 'public/pdfjs'
if (!existsSync(from)) {
  console.error('pdfjs-dist не установлен — выполните npm ci')
  process.exit(1)
}
rmSync(to, { recursive: true, force: true })
mkdirSync(to, { recursive: true })
for (const dir of ['cmaps', 'standard_fonts', 'iccs']) cpSync(`${from}/${dir}`, `${to}/${dir}`, { recursive: true })
// Скрипты из PDF (quickjs) не нужны — JavaScript в документах не выполняется.
cpSync(`${from}/wasm`, `${to}/wasm`, { recursive: true, filter: (src) => !/quickjs/i.test(src) })
