import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'
import { resolve } from 'node:path'

const host = process.env.TAURI_DEV_HOST

export default defineConfig(({ command }) => ({
  base: command === 'build' ? './' : '/',
  plugins: [react()],
  clearScreen: false,
  build: {
    rollupOptions: {
      input: {
        main: resolve(__dirname, 'index.html'),
        accounts: resolve(__dirname, 'accounts.html'),
        profileEditor: resolve(__dirname, 'profile-editor.html'),
        import: resolve(__dirname, 'import.html'),
        connectorRuntimes: resolve(__dirname, 'connector-runtimes.html'),
        runtimeLog: resolve(__dirname, 'runtime-log.html'),
        connectorDebug: resolve(__dirname, 'connector-debug.html'),
        queueStatus: resolve(__dirname, 'queue-status.html'),
        scheduler: resolve(__dirname, 'scheduler.html'),
        plans: resolve(__dirname, 'plans.html'),
        batchEditor: resolve(__dirname, 'batch-editor.html'),
        profileView: resolve(__dirname, 'profile-view.html'),
        singleVideos: resolve(__dirname, 'single-videos.html'),
      },
    },
  },
  test: {
    environment: 'node',
  },
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: 'ws',
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
}))
