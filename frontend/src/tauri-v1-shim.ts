// Сборка для Astra 1.7 (Tauri 1): вместо '@tauri-apps/api/core' (Tauri 2)
// вызываем IPC Tauri 1 напрямую. Подключается алиасом в vite.config.ts (`--mode legacy`).
type Invoke = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>

export function invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  const w = window as unknown as { __TAURI_INVOKE__?: Invoke; __TAURI__?: { invoke?: Invoke; tauri?: { invoke?: Invoke } } }
  const fn = w.__TAURI_INVOKE__ ?? w.__TAURI__?.tauri?.invoke ?? w.__TAURI__?.invoke
  if (!fn) return Promise.reject(new Error('Tauri IPC недоступен'))
  return fn(cmd, args) as Promise<T>
}
