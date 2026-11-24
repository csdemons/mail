// Проверки разбора вставки из буфера: картинка или файл — вложение, обычный текст — как есть.
// От этого зависит, попадёт ли картинка из Ctrl+V во вложение письма.
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import ts from 'typescript'

const source = await readFile(new URL('../src/pasteClipboard.ts', import.meta.url), 'utf8')
const code = ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  fileName: 'pasteClipboard.ts',
}).outputText
const { isFilePaste, pastedName } =
  await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`)

/** Что отдаёт событие вставки; по умолчанию — пустой буфер. */
const paste = (o = {}) => ({ files: [], types: [], text: '', uris: '', ...o })

test('картинка или файл из WebView — вставка файла', () => {
  assert.equal(isFilePaste(paste({ files: ['x'] })), true)
})

test('обычный текст вставляется как текст', () => {
  assert.equal(isFilePaste(paste({ text: 'Здравствуйте, коллеги!', types: ['text/plain'] })), false)
})

test('пустой буфер — не вставка файла', () => {
  assert.equal(isFilePaste(paste()), false)
})

test('файл из проводника приходит ссылкой file://', () => {
  assert.equal(isFilePaste(paste({ uris: 'file:///C:/Temp/photo.png', text: 'photo.png' })), true)
})

test('путь обычным текстом считается файлом только с пометкой text/uri-list', () => {
  assert.equal(isFilePaste(paste({ text: '/home/user/photo.png', types: ['text/uri-list'] })), true)
  assert.equal(isFilePaste(paste({ text: '/home/user/photo.png', types: ['text/plain'] })), false)
})

test('имя, которое дал браузер, сохраняется', () => {
  assert.equal(pastedName('схема.png', 'image/png'), 'схема.png')
})

test('безымянный снимок получает имя по дате', () => {
  const name = pastedName('image.png', 'image/png', new Date('2026-11-24T22:40:05'))
  assert.match(name, /^Снимок \d{2}-\d{2}-\d{4}-\d{2}-\d{2}-\d{2}\.png$/)
})

test('расширение берётся из типа, jpeg становится jpg', () => {
  assert.match(pastedName('', 'image/jpeg', new Date('2026-11-24T10:00:00')), /\.jpg$/)
  assert.match(pastedName('', 'image/webp', new Date('2026-11-24T10:00:00')), /\.webp$/)
  assert.match(pastedName('', '', new Date('2026-11-24T10:00:00')), /\.png$/)
})
