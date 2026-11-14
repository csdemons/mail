// Размер исходников: большие файлы трудно читать и проверять.
//
//   node frontend/scripts/check-file-size.mjs            — проверка (из корня репозитория)
//   node frontend/scripts/check-file-size.mjs --update   — записать текущие размеры в список исключений
//
// Пределы: .ts/.tsx — 800 строк, .rs — 1500. Файлы, которые уже больше, перечислены в
// file-size-baseline.json с их размером: рост — предупреждение, рост больше чем на SLACK
// строк — ошибка (уменьшились — обновите список через --update). Новый файл сверх предела — ошибка.
import fs from 'node:fs'
import path from 'node:path'

const LIMITS = { '.ts': 800, '.tsx': 800, '.rs': 1500 }
const ROOTS = ['frontend/src', 'src-tauri/src', 'legacy/src']
const BASELINE = 'file-size-baseline.json'
const SLACK = 50
const ci = Boolean(process.env.GITHUB_ACTIONS)
const warn = (f, text) => console.log(ci ? `::warning file=${f}::${text}` : `внимание ${f}: ${text}`)

function walk(dir, out = []) {
  if (!fs.existsSync(dir)) return out
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) walk(p, out)
    else if (LIMITS[path.extname(e.name)]) out.push(p.split(path.sep).join('/'))
  }
  return out
}

const lines = (f) => fs.readFileSync(f, 'utf8').split('\n').length - 1
const sizes = Object.fromEntries(ROOTS.flatMap((r) => walk(r)).sort().map((f) => [f, lines(f)]))
const over = Object.entries(sizes).filter(([f, n]) => n > LIMITS[path.extname(f)])

if (process.argv.includes('--update')) {
  fs.writeFileSync(BASELINE, JSON.stringify(Object.fromEntries(over), null, 2) + '\n')
  console.log(`${BASELINE}: ${over.length} файлов сверх предела`)
  process.exit(0)
}

const baseline = fs.existsSync(BASELINE) ? JSON.parse(fs.readFileSync(BASELINE, 'utf8')) : {}
let failed = false
for (const [f, n] of over) {
  const limit = LIMITS[path.extname(f)]
  const allowed = baseline[f]
  if (allowed === undefined) {
    console.error(`ОШИБКА ${f}: ${n} строк (предел ${limit}) — разделите файл`)
    failed = true
  } else if (n > allowed + SLACK) {
    console.error(`ОШИБКА ${f}: ${n} строк, было ${allowed} — файл сверх предела ${limit} вырос больше чем на ${SLACK} строк; вынесите часть в отдельный модуль`)
    failed = true
  } else if (n > allowed) {
    warn(f, `${n} строк (было ${allowed}, предел ${limit}) — файл растёт; лучше вынести часть в отдельный модуль`)
  } else {
    console.log(`${f}: ${n} строк (предел ${limit}, в списке исключений)`)
  }
}
for (const f of Object.keys(baseline)) {
  if (!(f in sizes) || sizes[f] <= LIMITS[path.extname(f)]) console.log(`${f} больше не превышает предел — уберите его из ${BASELINE} (--update)`)
}
process.exit(failed ? 1 : 0)
