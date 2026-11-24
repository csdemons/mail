// Проверки роста поля ввода: длинное сообщение видно целиком, выше предела — прокрутка,
// после удаления текста поле сжимается обратно. Как и остальные тесты интерфейса,
// обходится компилятором TypeScript проекта и раннером Node — без DOM и лишних пакетов.
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import ts from 'typescript'

const source = await readFile(new URL('../src/autoGrow.ts', import.meta.url), 'utf8')
const code = ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  fileName: 'autoGrow.ts',
}).outputText
const { autoGrow, COMPOSE_MAX_PX } =
  await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`)

/** Заглушка поля ввода: содержимое и рамка подставляются тестом. */
function stub(scrollHeight, border = 2) {
  return {
    style: { height: '', overflowY: '' },
    scrollHeight,
    offsetHeight: scrollHeight + border,
    clientHeight: scrollHeight,
  }
}

test('поле растёт под текст целиком, рамка учтена', () => {
  const el = stub(120)
  autoGrow(el)
  assert.equal(el.style.height, '122px')
  assert.equal(el.style.overflowY, 'hidden')
})

test('выше предела поле не растёт, а прокручивается', () => {
  const el = stub(400)
  autoGrow(el)
  assert.equal(el.style.height, `${COMPOSE_MAX_PX}px`)
  assert.equal(el.style.overflowY, 'auto')
})

test('после удаления строк поле сжимается — высота сбрасывается перед измерением', () => {
  const el = {
    style: { height: '300px', overflowY: 'auto' },
    // Пока высота не сброшена, браузер отдаёт старую (большую) высоту содержимого.
    get scrollHeight() { return this.style.height === 'auto' ? 40 : 300 },
    offsetHeight: 42,
    clientHeight: 40,
  }
  autoGrow(el)
  assert.equal(el.style.height, '42px')
  assert.equal(el.style.overflowY, 'hidden')
})

test('скрытое поле не получает нулевую высоту', () => {
  const el = stub(0)
  autoGrow(el)
  assert.equal(el.style.height, '')
  assert.equal(el.style.overflowY, 'hidden')
})

test('предел можно задать меньше — для поля ответа в ветке', () => {
  const el = stub(200)
  autoGrow(el, 90)
  assert.equal(el.style.height, '90px')
  assert.equal(el.style.overflowY, 'auto')
})

test('без поля ничего не падает', () => {
  assert.doesNotThrow(() => autoGrow(null))
})
