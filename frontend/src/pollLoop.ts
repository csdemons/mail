// Один цикл фоновой проверки: одна проверка одновременно, один владелец таймера и не больше
// одного отложенного повтора. Нужен там, где пробуждение окна приходит во время сетевого запроса.
//
// Раньше tick() в конце планировал следующий таймер, а возврат фокуса запускал ещё один tick().
// Пока первый запрос ждал сеть, циклов становилось два: каждый планировал свой следующий таймер,
// а переменная timer хранила только последний. Лишний цикл не виден в интерфейсе, но на
// развёртывании умножает нагрузку на Nextcloud и сбивает расчёт частоты обращений.

/** Одна проверка. Возвращает паузу до следующей, в миллисекундах. */
export type PollCheck = () => Promise<number>

export interface PollTimers {
  setTimeout(callback: () => void, ms: number): number
  clearTimeout(id: number): void
}

export interface PollLoop {
  /** Проверить при первой возможности: окно вернулось в фокус или появилось на экране. */
  wake(): void
  /** Остановить цикл: следующий таймер не планируется, идущая проверка уже ничего не запустит. */
  stop(): void
}

/** Пауза, если проверка завершилась ошибкой или не назвала срок. */
export const DEFAULT_POLL_DELAY_MS = 60_000

/** Пробуждение в этом окне от начала проверки считается частью того же события, что её
 *  запустило: focus и visibilitychange приходят парой, и второго запроса они не требуют. */
export const WAKE_BURST_MS = 1_000

/** Запустить цикл. Проверка начинается по wake(), дальше — по таймеру или отложенному повтору. */
export function createPollLoop(check: PollCheck, timers: PollTimers, now: () => number = Date.now): PollLoop {
  let timer: number | undefined
  let running = false
  let again = false
  let stopped = false
  let startedAt = 0

  function schedule(delay: number) {
    const ms = Number.isFinite(delay) && delay > 0 ? delay : DEFAULT_POLL_DELAY_MS
    timer = timers.setTimeout(() => { timer = undefined; void tick() }, ms)
  }

  async function tick() {
    if (stopped) return
    if (running) { again = true; return }
    running = true
    startedAt = now()
    let delay = DEFAULT_POLL_DELAY_MS
    try {
      delay = await check()
    } catch {
      // Ошибку разбирает сама проверка; цикл повторяет через обычную паузу.
    } finally {
      running = false
      if (!stopped) {
        // Отложенный повтор важнее таймера: данные нужны сразу после завершения запроса.
        if (again) { again = false; void tick() } else schedule(delay)
      }
    }
  }

  function wake() {
    if (stopped) return
    if (running) {
      // Идёт проверка, начатая этим же событием (пара focus + visibilitychange), — повтор не нужен.
      // Проверка, начатая раньше, могла застать старое состояние, поэтому нужен один повтор.
      if (now() - startedAt >= WAKE_BURST_MS) again = true
      return
    }
    if (timer !== undefined) { timers.clearTimeout(timer); timer = undefined }
    void tick()
  }

  function stop() {
    stopped = true
    again = false
    if (timer !== undefined) { timers.clearTimeout(timer); timer = undefined }
  }

  return { wake, stop }
}
