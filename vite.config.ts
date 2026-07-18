import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'
import { resolve } from 'node:path'
import { realpathSync } from 'node:fs'

const host = process.env.TAURI_DEV_HOST
// Windows worktrees may be exposed through a junction (C:\Users\...\worktrees)
// while Node resolves dependencies through their physical path (D:\worktrees).
// Rollup rejects HTML inputs when those two roots are mixed, so keep Vite and
// every entry point on the same canonical path.
const projectRoot = realpathSync.native(__dirname)

export default defineConfig(({ command }) => ({
  root: projectRoot,
  base: command === 'build' ? './' : '/',
  plugins: [react()],
  clearScreen: false,
  build: {
    rollupOptions: {
      input: {
        main: resolve(projectRoot, 'index.html'),
        accounts: resolve(projectRoot, 'accounts.html'),
        profileEditor: resolve(projectRoot, 'profile-editor.html'),
        import: resolve(projectRoot, 'import.html'),
        connectorRuntimes: resolve(projectRoot, 'connector-runtimes.html'),
        runtimeLog: resolve(projectRoot, 'runtime-log.html'),
        connectorDebug: resolve(projectRoot, 'connector-debug.html'),
        queueStatus: resolve(projectRoot, 'queue-status.html'),
        scheduler: resolve(projectRoot, 'scheduler.html'),
        plans: resolve(projectRoot, 'plans.html'),
        batchEditor: resolve(projectRoot, 'batch-editor.html'),
        profileView: resolve(projectRoot, 'profile-view.html'),
        singleVideos: resolve(projectRoot, 'single-videos.html'),
        workspaceHealth: resolve(projectRoot, 'workspace-health.html'),
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
