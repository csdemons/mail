// Поле ввода сообщения растёт по содержимому: длинный текст виден целиком, а не в окошке
// на две строки. Выше предела поле прокручивается внутри, иначе оно заняло бы весь чат.
//
// Без DOM и React, чтобы поведение проверялось тестами (scripts/test-auto-grow.mjs).

/** Предел высоты поля ввода в чате (около 10 строк): дальше — прокрутка. */
export const COMPOSE_MAX_PX = 260

/** Что нужно от поля ввода. В тестах вместо элемента подставляется заглушка. */
export interface Growable {
  style: { height: string; overflowY: string }
  /** Высота содержимого с отступами, но без рамки. */
  scrollHeight: number
  /** Полная высота с рамкой. */
  offsetHeight?: number
  /** Высота внутри рамки. */
  clientHeight?: number
}

/** Подогнать высоту поля под текст. Вызывается после каждого изменения текста. */
export function autoGrow(el: Growable | null, maxPx = COMPOSE_MAX_PX): void {
  if (!el) return
  // Сначала «auto»: иначе scrollHeight не уменьшится после удаления строк и поле не сожмётся.
  el.style.height = 'auto'
  const needed = el.scrollHeight
  if (needed <= 0) {
    // Поле скрыто (другая вкладка, свёрнутая панель) — высоту задавать нечему.
    el.style.height = ''
    el.style.overflowY = 'hidden'
    return
  }
  // box-sizing: border-box — рамка не входит в scrollHeight, но входит в height.
  const border = (el.offsetHeight ?? 0) - (el.clientHeight ?? 0)
  el.style.height = `${Math.min(needed + border, maxPx)}px`
  // Пока текст помещается — полосы прокрутки нет; дальше она нужна.
  el.style.overflowY = needed + border > maxPx ? 'auto' : 'hidden'
}
