// Вынести кусок JSX из компонента в отдельный компонент (инструмент разработки).
//
//   node scripts/extract-component.mjs <файл> <Компонент> <первая строка> <последняя строка> <Новый> <путь нового файла>
//
// Строки — 1-based, включительно, ровно один JSX-элемент. Всё, что JSX берёт из тела
// компонента (состояние, функции, ref), становится свойствами нового компонента с типами,
// выведенными TypeScript; импорты переносятся те, что нужны. В исходном файле на месте
// куска остаётся <Новый {...{ a, b }} />. Модульные (не импортированные) помощники исходного
// файла сначала нужно вынести в отдельный модуль — скрипт перечислит их и остановится.
import ts from 'typescript'
import fs from 'node:fs'
import path from 'node:path'

// --narrow: типы свойств — суженные в месте использования (кусок стоит под `x && …`).
const narrow = process.argv.includes('--narrow')
const [file, compName, l1, l2, newName, newPath] = process.argv.slice(2).filter((a) => a !== '--narrow')
if (!newPath) { console.error('usage: extract-component.mjs <file> <Component> <from> <to> <NewName> <newPath>'); process.exit(2) }
const abs = path.resolve(file)
const cfg = ts.getParsedCommandLineOfConfigFile(path.resolve('tsconfig.app.json'), {}, { ...ts.sys, onUnRecoverableConfigFileDiagnostic: () => {} })
const program = ts.createProgram([abs], cfg.options)
const checker = program.getTypeChecker()
const sf = program.getSourceFile(abs)
const text = sf.text
const lineStart = (n) => sf.getPositionOfLineAndCharacter(n - 1, 0)
const start = lineStart(Number(l1))
const end = Number(l2) >= sf.getLineStarts().length ? text.length : lineStart(Number(l2) + 1)

// Тело компонента
let body = null
ts.forEachChild(sf, function find(n) {
  if ((ts.isFunctionDeclaration(n) && n.name?.text === compName) && n.body) body = n
  else ts.forEachChild(n, find)
})
if (!body) throw new Error(`компонент ${compName} не найден`)

// JSX-элемент в диапазоне
let jsx = null
ts.forEachChild(sf, function find(n) {
  if (jsx) return
  if ((ts.isJsxElement(n) || ts.isJsxSelfClosingElement(n) || ts.isJsxFragment(n)) && n.getStart(sf) >= start && n.end <= end) { jsx = n; return }
  ts.forEachChild(n, find)
})
if (!jsx) throw new Error('в диапазоне нет JSX-элемента')
const jStart = jsx.getStart(sf), jEnd = jsx.end

const props = new Map()   // имя → тип
const imports = new Map() // модуль → { default, names: Map(name → {type, alias}), ns }
const helpers = new Set()
const relModule = (spec) => spec // относительные пути — от того же каталога, что и исходный файл
const fromDir = path.dirname(abs), toDir = path.dirname(path.resolve(newPath))
function rebase(spec) {
  if (!spec.startsWith('.')) return spec
  let r = path.relative(toDir, path.resolve(fromDir, spec)).replaceAll(path.sep, '/')
  if (!r.startsWith('.')) r = './' + r
  return r
}
function typeText(sym, at) {
  const t = narrow ? checker.getTypeOfSymbolAtLocation(sym, at) : checker.getTypeOfSymbol(sym)
  let s = checker.typeToString(t, undefined, ts.TypeFormatFlags.NoTruncation | ts.TypeFormatFlags.UseFullyQualifiedType | ts.TypeFormatFlags.WriteArrowStyleSignature)
  // import("/абсолютный/путь/types") → import("./types") относительно нового файла
  s = s.replace(/import\("([^"]+)"\)/g, (_, p) => {
    if (p.includes('/node_modules/')) {
      const m = p.split('/node_modules/').pop().replace(/\/(index|dist\/.*)$/,'').replace(/\.d$/, '')
      return `import("${m.startsWith('@types/') ? m.slice(7) : m}")`
    }
    let r = path.relative(toDir, p).replaceAll(path.sep, '/')
    if (!r.startsWith('.')) r = './' + r
    return `import("${r}")`
  })
  return s
}
function visit(n) {
  if (ts.isIdentifier(n)) {
    let sym = checker.getSymbolAtLocation(n)
    if (ts.isShorthandPropertyAssignment(n.parent) && n.parent.name === n) sym = checker.getShorthandAssignmentValueSymbol(n.parent)
    const decl = sym?.declarations?.[0]
    if (decl && decl.getSourceFile() === sf && !(decl.pos >= jStart && decl.end <= jEnd)) {
      const name = n.text
      if (ts.isBindingElement(n.parent) && n.parent.propertyName === n) { ts.forEachChild(n, visit); return }
      if (ts.isImportSpecifier(decl) || ts.isImportClause(decl) || ts.isNamespaceImport(decl)) {
        const imp = decl.parent.parent.parent ?? decl.parent.parent
        const id = ts.isImportClause(decl) ? decl.parent : (ts.isNamespaceImport(decl) ? decl.parent.parent : imp)
        const spec = (ts.isImportClause(decl) ? decl.parent : ts.isNamespaceImport(decl) ? decl.parent.parent : decl.parent.parent.parent).moduleSpecifier.text
        const clause = ts.isImportClause(decl) ? decl : ts.isNamespaceImport(decl) ? decl.parent : decl.parent.parent
        const e = imports.get(spec) ?? { def: null, names: new Map(), ns: null }
        if (ts.isImportClause(decl)) e.def = { name, type: clause.isTypeOnly }
        else if (ts.isNamespaceImport(decl)) e.ns = name
        else e.names.set(decl.propertyName ? `${decl.propertyName.text} as ${name}` : name, decl.isTypeOnly || clause.isTypeOnly)
        imports.set(spec, e)
        void id
      } else if (decl.pos >= body.pos && decl.end <= body.end) {
        // объявлено в теле компонента (но не внутри куска) — свойство
        if (!props.has(name) && !(ts.isPropertyAccessExpression(n.parent) && n.parent.name === n) && !(ts.isJsxAttribute(n.parent) && n.parent.name === n) && !(ts.isPropertyAssignment(n.parent) && n.parent.name === n)) {
          props.set(name, typeText(sym, n))
        }
      } else if (!(ts.isPropertyAccessExpression(n.parent) && n.parent.name === n) && !(ts.isJsxAttribute(n.parent) && n.parent.name === n) && !(ts.isPropertyAssignment(n.parent) && n.parent.name === n) && !ts.isTypeParameterDeclaration(decl)) {
        helpers.add(name)
      }
    }
  }
  ts.forEachChild(n, visit)
}
visit(jsx)
if (helpers.size) {
  console.error(`Сначала вынесите модульные помощники ${file}: ${[...helpers].join(', ')}`)
  process.exit(1)
}

// Новый файл
const jsxText = text.slice(jStart, jEnd)
const indent = (text.slice(text.lastIndexOf('\n', jStart) + 1, jStart).match(/^\s*/) ?? [''])[0].length
const dedented = jsxText.split('\n').map((l, i) => (i === 0 ? l : l.slice(Math.min(indent, l.match(/^\s*/)[0].length)))).join('\n')
const body2 = dedented.split('\n').map((l) => (l ? '    ' + l : l)).join('\n')
let impLines = [...imports.entries()].map(([spec, e]) => {
  const parts = []
  const names = [...e.names.entries()].map(([n, t]) => (t ? `type ${n}` : n))
  if (e.ns) return `import * as ${e.ns} from '${rebase(spec)}'`
  if (e.def && names.length) return `import ${e.def.name}, { ${names.join(', ')} } from '${rebase(spec)}'`
  if (e.def) return `import ${e.def.type ? 'type ' : ''}${e.def.name} from '${rebase(spec)}'`
  return `import { ${names.join(', ')} } from '${rebase(spec)}'`
})
const names = [...props.keys()]
// import("./types").Account → Account + import type { Account } from './types'
const typeImports = new Map()
for (const [n, t] of props) {
  props.set(n, t.replace(/import\("([^"]+)"\)\.(\w+)/g, (_, mod, name) => {
    if (!typeImports.has(mod)) typeImports.set(mod, new Set())
    typeImports.get(mod).add(name)
    return name
  }))
}
for (const [mod, set] of typeImports) {
  const i = impLines.findIndex((l) => l.endsWith(` from '${mod}'`) && l.includes('{') && !l.startsWith('import type'))
  const have = i >= 0 ? impLines[i] : ''
  const add = [...set].filter((n) => !new RegExp(`\\b${n}\\b`).test(have))
  if (!add.length) continue
  if (i >= 0) impLines[i] = have.replace(/ \}/, `, ${add.map((n) => 'type ' + n).join(', ')} }`)
  else impLines.push(`import type { ${add.join(', ')} } from '${mod}'`)
}
const iface = names.map((n) => `  ${n}: ${props.get(n)}`).join('\n')
const out = `${impLines.join('\n')}\n\nexport interface ${newName}Props {\n${iface}\n}\n\nexport default function ${newName}({ ${names.join(', ')} }: ${newName}Props) {\n  return (\n${body2}\n  )\n}\n`
fs.mkdirSync(path.dirname(path.resolve(newPath)), { recursive: true })
fs.writeFileSync(newPath, out)

// Исходный файл: на месте куска — вызов
const callIndent = ' '.repeat(indent)
const call = !names.length ? `<${newName} />`
  : names.join(', ').length < 90 ? `<${newName} {...{ ${names.join(', ')} }} />`
  : `<${newName}\n${callIndent}  ${names.map((n) => `${n}={${n}}`).join(`\n${callIndent}  `)}\n${callIndent}/>`
let src = text.slice(0, jStart) + call + text.slice(jEnd)
let imp = rebase('./' + path.relative(fromDir, path.resolve(newPath)).replace(/\.tsx?$/, ''))
imp = path.relative(fromDir, path.resolve(newPath)).replaceAll(path.sep, '/').replace(/\.tsx?$/, '')
if (!imp.startsWith('.')) imp = './' + imp
const lastImport = [...src.matchAll(/^import [^\n]*\n(?:(?: {2}[^\n]*\n)*?\}[^\n]*\n)?/gm)].pop()
const at = lastImport ? lastImport.index + lastImport[0].length : 0
src = src.slice(0, at) + `import ${newName} from '${relModule(imp)}'\n` + src.slice(at)
fs.writeFileSync(abs, src)
console.log(`${newName}: ${names.length} свойств, ${impLines.length} импортов → ${newPath}`)
