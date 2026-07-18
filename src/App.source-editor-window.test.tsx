// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App'
import { createEmptyWorkspaceSnapshot } from './domain/workspaceSnapshot'

const bootstrapMock = vi.fn()
const applySnapshotMock = vi.fn()
const refreshSnapshotMock = vi.fn()
const routeActionMock = vi.fn()
const runSourceSyncMock = vi.fn()
const setActiveSectionMock = vi.fn()
const cloneProviderAccountMock = vi.fn()
const deleteProviderAccountMock = vi.fn()
const deleteSourceProfileMock = vi.fn()
const cancelSourceSyncProfileMock = vi.fn()
const pickProfileImageMock = vi.fn()
const resetProfileImageMock = vi.fn()
const openSourceFolderMock = vi.fn()
const upsertSourceProfileMock = vi.fn()

const bridgeMocks = vi.hoisted(() => ({
  checkAppUpdate: vi.fn(),
  getAppBuildInfo: vi.fn(),
  enqueueSourceDelete: vi.fn(),
  loadSourceDeleteQueueStatus: vi.fn(),
  loadSourceSyncQueueStatus: vi.fn(),
  loadWorkspaceHealth: vi.fn(() => Promise.resolve(undefined)),
  openAccountsWindow: vi.fn(),
  openConnectorRuntimesWindow: vi.fn(),
  openExternalTarget: vi.fn(),
  openImportWindow: vi.fn(),
  openRuntimeLogWindow: vi.fn(),
  openWorkspaceHealthWindow: vi.fn(),
  openSourceEditorWindow: vi.fn(),
  openSourceSyncQueueWindow: vi.fn(),
  subscribeToDesktopRuntimeEvents: vi.fn(),
  subscribeToFocusSourceRequest: vi.fn(() => Promise.resolve(() => undefined)),
}))

vi.mock('./bridge/desktop', () => bridgeMocks)
vi.mock('./features/scheduler/SchedulerPage', () => ({
  SchedulerPage: () => <div>SchedulerPage</div>,
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
vi.mock('./features/workspace/workspaceProfiles', () => ({
  buildSourceProfileUrl: () => undefined,
  buildServiceTabs: () => [],
  filterSourcesForWorkspace: () => [],
  parseClipboardProfileSeed: () => undefined,
  formatSourceHandleLabel: (value: string) => value,
}))
vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({ label: 'main' }),
}))
vi.mock('./state/appStore', () => ({
  useAppStore: (
    selector: (state: Record<string, unknown>) => unknown,
  ) => {
    const snapshot = createEmptyWorkspaceSnapshot()
    return selector({
      activeSection: 'sources',
      applySnapshot: applySnapshotMock,
      bootstrap: bootstrapMock,
      loading: false,
      pendingCommand: undefined,
      refreshSnapshot: refreshSnapshotMock,
      routeAction: routeActionMock,
      runSourceSync: runSourceSyncMock,
      setActiveSection: setActiveSectionMock,
      snapshot,
      error: undefined,
      cloneProviderAccount: cloneProviderAccountMock,
      deleteProviderAccount: deleteProviderAccountMock,
      deleteSourceProfile: deleteSourceProfileMock,
      cancelSourceSyncProfile: cancelSourceSyncProfileMock,
      pickSourceProfileImage: pickProfileImageMock,
      resetSourceProfileImage: resetProfileImageMock,
      openSourceFolder: openSourceFolderMock,
      upsertSourceProfile: upsertSourceProfileMock,
    })
  },
}))

describe('App source editor window flow', () => {
  beforeEach(() => {
    applySnapshotMock.mockReset()
    bootstrapMock.mockReset()
    bridgeMocks.openSourceEditorWindow.mockReset()
    bridgeMocks.loadSourceDeleteQueueStatus.mockReset()
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
      updatedAt: '2026-03-13T00:00:00Z',
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
      updatedAt: '2026-03-13T00:00:00Z',
    })
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockResolvedValue(() => undefined)
    window.history.replaceState({}, '', '/')
  })

  afterEach(() => {
    cleanup()
  })

  it('opens source editor window from +Add without rendering in-app modal', async () => {
    render(<App />)

    fireEvent.click(screen.getByRole('button', { name: /\+ add/i }))

    await waitFor(() => {
      expect(bridgeMocks.openSourceEditorWindow).toHaveBeenCalledTimes(1)
    })
    expect(screen.queryByText('SourceEditorDialog')).toBeNull()
  })

  it('applies workspace snapshots pushed from desktop runtime events', async () => {
    let runtimeHandlers:
      | {
          onWorkspaceSnapshotChanged?: (snapshot: ReturnType<typeof createEmptyWorkspaceSnapshot>) => void
        }
      | undefined
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockImplementation(async (handlers) => {
      runtimeHandlers = handlers
      return () => undefined
    })

    render(<App />)

    const nextSnapshot = createEmptyWorkspaceSnapshot()
    nextSnapshot.desktopRuntime.reportedByBackend = true
    runtimeHandlers?.onWorkspaceSnapshotChanged?.(nextSnapshot)

    expect(applySnapshotMock).toHaveBeenCalledWith(nextSnapshot)
  })
})
