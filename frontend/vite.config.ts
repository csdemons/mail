import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// `vite build --mode legacy` — интерфейс для сборки на Tauri 1 (Astra 1.7):
// другой IPC и отдельный каталог сборки.
// Конфигурация для Tauri: фиксированный порт, dev-сервер только на localhost.
export default defineConfig(({ mode }) => {
  const legacy = mode === 'legacy'
  return {
    plugins: [react()],
    clearScreen: false,
    resolve: legacy ? { alias: { '@tauri-apps/api/core': '/src/tauri-v1-shim.ts' } } : {},
    server: {
      host: '127.0.0.1',
      port: 5173,
      strictPort: true,
    },
    build: {
      // WebKitGTK в Astra 1.7 старее — es2020 безопасно.
      target: legacy ? 'es2020' : 'es2021',
      outDir: legacy ? 'dist-legacy' : 'dist',
      sourcemap: false,
    },
  }
})
