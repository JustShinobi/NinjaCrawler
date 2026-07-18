// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App'
import { createEmptyWorkspaceSnapshot } from './domain/workspaceSnapshot'

const bootstrapMock = vi.fn()
const refreshSnapshotMock = vi.fn()
const routeActionMock = vi.fn()
const runSourceSyncMock = vi.fn()
const setActiveSectionMock = vi.fn()
const cloneProviderAccountMock = vi.fn()
const deleteProviderAccountMock = vi.fn()
const deleteSourceProfileMock = vi.fn()
const openMediaItemMock = vi.fn()
const openSourceFolderMock = vi.fn()
const pickSourceProfileImageMock = vi.fn()
const resetSourceProfileImageMock = vi.fn()
const upsertSourceProfileMock = vi.fn()

const bridgeMocks = vi.hoisted(() => ({
  checkAppUpdate: vi.fn(),
  getAppBuildInfo: vi.fn(),
  loadSourceDeleteQueueStatus: vi.fn(),
  loadSourceSyncQueueStatus: vi.fn(),
  loadWorkspaceHealth: vi.fn(() => Promise.resolve(undefined)),
  openConnectorRuntimesWindow: vi.fn(),
  openExternalTarget: vi.fn(),
  openRuntimeLogWindow: vi.fn(),
  openWorkspaceHealthWindow: vi.fn(),
  openSchedulerWindow: vi.fn(),
  openSourceSyncQueueWindow: vi.fn(),
  subscribeToDesktopRuntimeEvents: vi.fn(),
  subscribeToFocusSourceRequest: vi.fn(() => Promise.resolve(() => undefined)),
}))

vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({ label: 'main' }),
}))
vi.mock('./bridge/desktop', () => bridgeMocks)
vi.mock('./features/accounts/AccountsPage', () => ({
  AccountsPage: () => <div>AccountsPage</div>,
}))
vi.mock('./features/feed/FeedPage', () => ({
  FeedPage: () => <div>FeedPage</div>,
}))
vi.mock('./features/library/LibraryPage', () => ({
  LibraryPage: () => <div>LibraryPage</div>,
}))
vi.mock('./features/settings/SettingsPage', () => ({
  SettingsPage: () => <div>SettingsPage</div>,
}))
vi.mock('./features/workspace/AccountsMenu', () => ({
  AccountsMenu: () => <div>AccountsMenu</div>,
}))
vi.mock('./features/workspace/InternalDialog', () => ({
  InternalDialog: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
}))
vi.mock('./features/workspace/ProfileWorkspace', () => ({
  ProfileWorkspace: () => <div>ProfileWorkspace</div>,
}))
vi.mock('./features/workspace/RuntimeLogWindowPage', () => ({
  RuntimeLogWindowPage: () => <div>RuntimeLogWindowPage</div>,
}))
vi.mock('./features/workspace/SourceEditorDialog', () => ({
  SourceEditorDialog: () => <div>SourceEditorDialog</div>,
}))
vi.mock('./features/workspace/workspaceProfiles', () => ({
  buildSourcePreviewMap: () => new Map(),
  buildSourceProfileUrl: () => undefined,
  buildServiceTabs: () => [{ key: 'all', label: 'All', count: 0 }],
  filterSourcesForWorkspace: () => [],
  formatSourceHandleLabel: (value: string) => value,
  parseClipboardProfileSeed: () => undefined,
}))

const workspaceSnapshot = {
  ...createEmptyWorkspaceSnapshot(),
  desktopRuntime: {
    silentMode: false,
    closeToTray: false,
    closeToTraySupported: true,
    reportedByBackend: true,
  },
}

vi.mock('./state/appStore', () => ({
  useAppStore: (
    selector: (state: {
      activeSection: 'sources'
      bootstrap: typeof bootstrapMock
      loading: false
      pendingCommand: undefined
      refreshSnapshot: typeof refreshSnapshotMock
      routeAction: typeof routeActionMock
      runSourceSync: typeof runSourceSyncMock
      setActiveSection: typeof setActiveSectionMock
      snapshot: typeof workspaceSnapshot
      error: undefined
      cloneProviderAccount: typeof cloneProviderAccountMock
      deleteProviderAccount: typeof deleteProviderAccountMock
      deleteSourceProfile: typeof deleteSourceProfileMock
      openMediaItem: typeof openMediaItemMock
      openSourceFolder: typeof openSourceFolderMock
      pickSourceProfileImage: typeof pickSourceProfileImageMock
      resetSourceProfileImage: typeof resetSourceProfileImageMock
      upsertSourceProfile: typeof upsertSourceProfileMock
    }) => unknown,
  ) =>
    selector({
      activeSection: 'sources',
      bootstrap: bootstrapMock,
      loading: false,
      pendingCommand: undefined,
      refreshSnapshot: refreshSnapshotMock,
      routeAction: routeActionMock,
      runSourceSync: runSourceSyncMock,
      setActiveSection: setActiveSectionMock,
      snapshot: workspaceSnapshot,
      error: undefined,
      cloneProviderAccount: cloneProviderAccountMock,
      deleteProviderAccount: deleteProviderAccountMock,
      deleteSourceProfile: deleteSourceProfileMock,
      openMediaItem: openMediaItemMock,
      openSourceFolder: openSourceFolderMock,
      pickSourceProfileImage: pickSourceProfileImageMock,
      resetSourceProfileImage: resetSourceProfileImageMock,
      upsertSourceProfile: upsertSourceProfileMock,
    }),
}))

describe('App scheduler window integration', () => {
  beforeEach(() => {
    bootstrapMock.mockReset()
    bootstrapMock.mockResolvedValue(workspaceSnapshot)
    refreshSnapshotMock.mockReset()
    refreshSnapshotMock.mockResolvedValue(workspaceSnapshot)
    routeActionMock.mockReset()
    setActiveSectionMock.mockReset()
    bridgeMocks.loadSourceSyncQueueStatus.mockReset()
    bridgeMocks.loadSourceDeleteQueueStatus.mockReset()
    bridgeMocks.openSchedulerWindow.mockReset()
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockReset()
    bridgeMocks.loadSourceSyncQueueStatus.mockResolvedValue({
      queuedCount: 0,
      runningCount: 0,
      completedCount: 0,
      failedCount: 0,
      totalCount: 0,
      providers: [],
      queuedItems: [],
      runningItems: [],
      recentResults: [],
      updatedAt: '2026-03-12T03:00:00Z',
    })
    bridgeMocks.loadSourceDeleteQueueStatus.mockResolvedValue({
      queuedCount: 0,
      runningCount: 0,
      completedCount: 0,
      failedCount: 0,
      totalCount: 0,
      queuedItems: [],
      runningItems: [],
      recentResults: [],
      updatedAt: '2026-03-12T03:00:00Z',
    })
    bridgeMocks.openSchedulerWindow.mockResolvedValue(undefined)
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockResolvedValue(() => undefined)
    window.history.replaceState({}, '', '/')
  })

  afterEach(() => {
    cleanup()
  })

  it('opens the dedicated Scheduler window from the tools menu', async () => {
    render(<App />)

    fireEvent.click(screen.getByRole('button', { name: /^tools$/i }))
    fireEvent.click(screen.getAllByRole('button', { name: /^scheduler$/i })[0])

    await waitFor(() => {
      expect(bridgeMocks.openSchedulerWindow).toHaveBeenCalledTimes(1)
    })
  })

  it('routes scheduler activation into the dedicated window instead of changing the main section', async () => {
    let routeActivation: ((actionRoute?: string) => void) | undefined
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockImplementation(
      async (handlers: { onRouteActivation?: (actionRoute?: string) => void }) => {
        routeActivation = handlers.onRouteActivation
        return () => undefined
      },
    )

    render(<App />)

    await waitFor(() => {
      expect(routeActivation).toBeTypeOf('function')
    })

    routeActivation?.('scheduler')

    await waitFor(() => {
      expect(bridgeMocks.openSchedulerWindow).toHaveBeenCalledTimes(1)
      expect(routeActionMock).not.toHaveBeenCalled()
      expect(refreshSnapshotMock).toHaveBeenCalled()
    })
  })
})
