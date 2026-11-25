// Проверки встроенных картинок: в редакторе они видны как data:-, а в письме упоминаются
// как cid: и уходят отдельными частями. От этого зависит, увидит ли получатель картинку.
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import ts from 'typescript'

const source = await readFile(new URL('../src/inlineImages.ts', import.meta.url), 'utf8')
const code = ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  fileName: 'inlineImages.ts',
}).outputText
const { newCid, outboundInline, restoreInline } =
  await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`)

const URL_A = 'data:image/png;base64,AAAA'
const URL_B = 'data:image/jpeg;base64,BBBB'

test('Send → Undo → Send: картинка не теряется', () => {
  // В редакторе картинка видна как data:-адрес.
  const editable = `<p>схема <img src="${URL_A}" alt="схема"></p>`
  const inline = [{ id: '1', cid: 'img-a', url: URL_A }]

  // Первая отправка: в письме cid:-ссылка, картинка — отдельной частью.
  const first = outboundInline(editable, inline)
  assert.equal(first.inline.length, 1, 'первая отправка: картинка уходит частью письма')

  // «Отменить»: снимок хранит редактируемый HTML и сами картинки — единый формат черновика.
  const snapshot = { html: editable, inline }

  // Новый Composer после отмены: картинка снова видна в редакторе и известна ему.
  const reopened = restoreInline(snapshot.html, snapshot.inline)
  assert.equal(reopened, editable, 'после отмены картинка на месте')
  assert.equal(snapshot.inline.length, 1, 'после отмены картинка известна редактору')

  // Повторная отправка: снова cid: и снова часть письма.
  const second = outboundInline(reopened, snapshot.inline)
  assert.equal(second.html, `<p>схема <img src="cid:img-a" alt="схема"></p>`)
  assert.deepEqual(second.inline, [{ id: '1', cid: 'img-a' }], 'повторная отправка не теряет картинку')
})

test('снимок в транспортном виде (сделанный до исправления) тоже восстанавливается', () => {
  const snapshot = { html: '<p><img src="cid:img-a"></p>', inline: [{ id: '1', cid: 'img-a', url: URL_A }] }
  const reopened = restoreInline(snapshot.html, snapshot.inline)
  assert.equal(reopened, `<p><img src="${URL_A}"></p>`)
  // cid снова превращается в ссылку, а картинка остаётся в списке частей.
  assert.equal(outboundInline(reopened, snapshot.inline).inline.length, 1)
})

test('адрес картинки заменяется на cid: — по нему письмо находит related-часть', () => {
  const html = `<p>схема <img src="${URL_A}" alt="схема"></p>`
  const out = outboundInline(html, [{ id: '1', cid: 'img-a', url: URL_A }])
  assert.equal(out.html, '<p>схема <img src="cid:img-a" alt="схема"></p>')
  assert.deepEqual(out.inline, [{ id: '1', cid: 'img-a' }])
})

test('картинка, повторённая в тексте дважды, ссылается на одну часть', () => {
  const html = `<p><img src="${URL_A}"> и ещё раз <img src="${URL_A}"></p>`
  const out = outboundInline(html, [{ id: '1', cid: 'img-a', url: URL_A }])
  assert.equal(out.html.match(/cid:img-a/g)?.length, 2)
  assert.equal(out.inline.length, 1)
})

test('картинку убрали из текста — её файл не отправляется', () => {
  const html = `<p>без картинок</p>`
  const out = outboundInline(html, [{ id: '1', cid: 'img-a', url: URL_A }])
  assert.deepEqual(out.inline, [])
  assert.equal(out.html, html)
})

test('несколько картинок различаются и не путаются', () => {
  const html = `<p><img src="${URL_A}"><img src="${URL_B}"></p>`
  const out = outboundInline(html, [
    { id: '1', cid: 'img-a', url: URL_A },
    { id: '2', cid: 'img-b', url: URL_B },
  ])
  assert.ok(out.html.includes('src="cid:img-a"') && out.html.includes('src="cid:img-b"'), out.html)
  assert.deepEqual(out.inline.map((i) => i.cid), ['img-a', 'img-b'])
})

test('картинку без адреса (не сохранилась) не отправляем и HTML не портим', () => {
  const html = '<p><img data-cid="img-a" alt=""></p>'
  const out = outboundInline(html, [{ id: '1', cid: 'img-a', url: '' }])
  assert.equal(out.html, html)
  assert.deepEqual(out.inline, [])
})

test('Content-ID уникален и годится для заголовка MIME', () => {
  const a = newCid()
  const b = newCid()
  assert.notEqual(a, b)
  assert.match(a, /^img-[a-z0-9]+-[a-z0-9]+$/)
})
