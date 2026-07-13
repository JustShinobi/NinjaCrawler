// @vitest-environment jsdom

import { cleanup, render, screen } from '@testing-library/react'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App'

const bootstrapMock = vi.fn()
const applySnapshotMock = vi.fn()
const refreshSnapshotMock = vi.fn()
const routeActionMock = vi.fn()
const runSourceSyncMock = vi.fn()
const setActiveSectionMock = vi.fn()
const cloneProviderAccountMock = vi.fn()
const deleteProviderAccountMock = vi.fn()
const deleteSourceProfileMock = vi.fn()
const openSourceFolderMock = vi.fn()
const upsertSourceProfileMock = vi.fn()

const bridgeMocks = vi.hoisted(() => ({
  checkAppUpdate: vi.fn(),
  getAppBuildInfo: vi.fn(),
  loadSourceSyncQueueStatus: vi.fn(),
  openAccountsWindow: vi.fn(),
  openConnectorRuntimesWindow: vi.fn(),
  openExternalTarget: vi.fn(),
  openSourceEditorWindow: vi.fn(),
  openImportWindow: vi.fn(),
  openRuntimeLogWindow: vi.fn(),
  openSchedulerWindow: vi.fn(),
  openSourceSyncQueueWindow: vi.fn(),
  subscribeToDesktopRuntimeEvents: vi.fn(),
  subscribeToFocusSourceRequest: vi.fn(() => Promise.resolve(() => undefined)),
}))

vi.mock('./bridge/desktop', () => bridgeMocks)
vi.mock('./features/accounts/AccountsPage', () => ({
  AccountsPage: () => <div>AccountsPage</div>,
}))
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
vi.mock('./features/workspace/SourceEditorDialog', () => ({
  SourceEditorDialog: () => <div>SourceEditorDialog</div>,
}))
vi.mock('./features/workspace/ToolbarAddMenu', () => ({
  ToolbarAddMenu: () => <div>ToolbarAddMenu</div>,
}))
vi.mock('./features/workspace/workspaceProfiles', () => ({
  buildSourceProfileUrl: () => undefined,
  buildServiceTabs: () => [],
  filterSourcesForWorkspace: () => [],
  parseClipboardProfileSeed: () => undefined,
}))
vi.mock('./state/appStore', () => ({
  useAppStore: (
    selector: (state: {
      activeSection: 'sources'
      applySnapshot: typeof applySnapshotMock
      bootstrap: typeof bootstrapMock
      loading: false
      pendingCommand: undefined
      refreshSnapshot: typeof refreshSnapshotMock
      routeAction: typeof routeActionMock
      runSourceSync: typeof runSourceSyncMock
      setActiveSection: typeof setActiveSectionMock
      snapshot: null
      error: undefined
      cloneProviderAccount: typeof cloneProviderAccountMock
      deleteProviderAccount: typeof deleteProviderAccountMock
      deleteSourceProfile: typeof deleteSourceProfileMock
      openSourceFolder: typeof openSourceFolderMock
      upsertSourceProfile: typeof upsertSourceProfileMock
    }) => unknown,
  ) =>
    selector({
      activeSection: 'sources',
      applySnapshot: applySnapshotMock,
      bootstrap: bootstrapMock,
      loading: false,
      pendingCommand: undefined,
      refreshSnapshot: refreshSnapshotMock,
      routeAction: routeActionMock,
      runSourceSync: runSourceSyncMock,
      setActiveSection: setActiveSectionMock,
      snapshot: null,
      error: undefined,
      cloneProviderAccount: cloneProviderAccountMock,
      deleteProviderAccount: deleteProviderAccountMock,
      deleteSourceProfile: deleteSourceProfileMock,
      openSourceFolder: openSourceFolderMock,
      upsertSourceProfile: upsertSourceProfileMock,
    }),
}))

describe('App runtime-log window', () => {
  beforeEach(() => {
    applySnapshotMock.mockReset()
    bootstrapMock.mockReset()
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockReset()
    window.history.replaceState({}, '', '/?window=runtime-log')
  })

  afterEach(() => {
    cleanup()
    window.history.replaceState({}, '', '/')
  })

  it('renders the runtime log page without bootstrapping the main workspace', () => {
    render(<App />)

    expect(screen.getByText('RuntimeLogWindowPage')).toBeTruthy()
    expect(bootstrapMock).not.toHaveBeenCalled()
    expect(bridgeMocks.subscribeToDesktopRuntimeEvents).not.toHaveBeenCalled()
  })
})
