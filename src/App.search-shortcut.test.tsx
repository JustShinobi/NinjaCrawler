// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
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
const openSourceFolderMock = vi.fn()
const upsertSourceProfileMock = vi.fn()

const bridgeMocks = vi.hoisted(() => ({
  checkAppUpdate: vi.fn(),
  getAppBuildInfo: vi.fn(),
  loadSourceDeleteQueueStatus: vi.fn(),
  loadSourceSyncQueueStatus: vi.fn(),
  loadWorkspaceHealth: vi.fn(() => Promise.resolve(undefined)),
  openConnectorRuntimesWindow: vi.fn(),
  openExternalTarget: vi.fn(),
  openImportWindow: vi.fn(),
  openRuntimeLogWindow: vi.fn(),
  openWorkspaceHealthWindow: vi.fn(),
  openSchedulerWindow: vi.fn(),
  openSourceSyncQueueWindow: vi.fn(),
  subscribeToDesktopRuntimeEvents: vi.fn(),
  subscribeToFocusSourceRequest: vi.fn(() => Promise.resolve(() => undefined)),
}))

vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({
    label: 'main',
  }),
}))

vi.mock('./bridge/desktop', () => bridgeMocks)
vi.mock('./features/accounts/AccountsPage', () => ({
  AccountsPage: () => <div>AccountsPage</div>,
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
  buildServiceTabs: () => [{ key: 'all', label: 'All', count: 0 }],
  filterSourcesForWorkspace: () => [],
  formatSourceHandleLabel: (handle: string) => handle,
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
      snapshot: ReturnType<typeof createEmptyWorkspaceSnapshot>
      error: undefined
      cloneProviderAccount: typeof cloneProviderAccountMock
      deleteProviderAccount: typeof deleteProviderAccountMock
      deleteSourceProfile: typeof deleteSourceProfileMock
      openSourceFolder: typeof openSourceFolderMock
      upsertSourceProfile: typeof upsertSourceProfileMock
      pickSourceProfileImage: () => Promise<void>
      resetSourceProfileImage: () => Promise<void>
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
      snapshot: createEmptyWorkspaceSnapshot(),
      error: undefined,
      cloneProviderAccount: cloneProviderAccountMock,
      deleteProviderAccount: deleteProviderAccountMock,
      deleteSourceProfile: deleteSourceProfileMock,
      openSourceFolder: openSourceFolderMock,
      upsertSourceProfile: upsertSourceProfileMock,
      pickSourceProfileImage: vi.fn(),
      resetSourceProfileImage: vi.fn(),
    }),
}))

describe('App search shortcut', () => {
  beforeEach(() => {
    applySnapshotMock.mockReset()
    bootstrapMock.mockReset()
    bridgeMocks.checkAppUpdate.mockReset()
    bridgeMocks.getAppBuildInfo.mockReset()
    bridgeMocks.loadSourceDeleteQueueStatus.mockReset()
    bridgeMocks.loadSourceSyncQueueStatus.mockReset()
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockReset()
    bridgeMocks.openWorkspaceHealthWindow.mockReset()
    bridgeMocks.loadSourceDeleteQueueStatus.mockResolvedValue({
      queuedCount: 0,
      runningCount: 0,
      completedCount: 0,
      failedCount: 0,
      totalCount: 0,
      queuedItems: [],
      runningItems: [],
      recentResults: [],
      updatedAt: new Date().toISOString(),
    })
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
      updatedAt: new Date().toISOString(),
    })
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockResolvedValue(() => undefined)
    bridgeMocks.openWorkspaceHealthWindow.mockResolvedValue(undefined)
    bridgeMocks.getAppBuildInfo.mockResolvedValue({
      version: '0.15.0',
      commitSha: '44ed4e3a',
      dirty: false,
      channel: 'development',
      displayVersion: 'Dev 44ed4e3a',
    })
    bridgeMocks.checkAppUpdate.mockResolvedValue({
      build: {
        version: '0.15.0',
        commitSha: '44ed4e3a',
        dirty: false,
        channel: 'development',
        displayVersion: 'Dev 44ed4e3a',
      },
      latestVersion: '0.15.0',
      releaseUrl: 'https://github.com/MetalDevOps/NinjaCrawler/releases/tag/v0.15.0',
      publishedAt: '2026-07-12T07:16:25Z',
      updateAvailable: false,
    })
    window.history.replaceState({}, '', '/')
  })

  it('opens Workspace Health from the compact toolbar action', async () => {
    render(<App />)

    fireEvent.click(await screen.findByRole('button', { name: /^health$/i }))

    await waitFor(() => expect(bridgeMocks.openWorkspaceHealthWindow).toHaveBeenCalledTimes(1))
  })

  afterEach(() => {
    cleanup()
  })

  it('focuses the toolbar search field on Ctrl+F', () => {
    render(<App />)

    const searchInput = screen.getByRole('searchbox', { name: /search current service tab/i })
    expect(document.activeElement).not.toBe(searchInput)

    fireEvent.keyDown(document, { key: 'f', ctrlKey: true })

    expect(document.activeElement).toBe(searchInput)
  })

  it('keeps menu sections grouped by responsibility', () => {
    render(<App />)
    const titlebar = document.querySelector('.main-titlebar')
    expect(titlebar).toBeTruthy()
    const titlebarWithin = within(titlebar as HTMLElement)

    expect(titlebarWithin.getAllByLabelText('NinjaCrawler')).toHaveLength(1)
    expect(titlebarWithin.queryByRole('button', { name: 'View' })).toBeNull()
    expect(titlebarWithin.queryByRole('button', { name: 'Download' })).toBeNull()

    fireEvent.click(titlebarWithin.getByRole('button', { name: 'Tools' }))
    const toolsMenu = document.querySelector('.menu-dropdown')
    expect(toolsMenu).toBeTruthy()
    const toolsWithin = within(toolsMenu as HTMLElement)
    expect(toolsWithin.getByRole('button', { name: 'Queue status' })).toBeTruthy()
    expect(toolsWithin.getByRole('button', { name: 'Scheduler' })).toBeTruthy()
    expect(toolsWithin.getByRole('button', { name: 'Runtime log' })).toBeTruthy()
    expect(toolsWithin.getByRole('button', { name: 'Connectors' })).toBeTruthy()
    expect(toolsWithin.getByRole('button', { name: 'Preferences' })).toBeTruthy()

    const toolbar = document.querySelector('.toolbar-strip')
    expect(toolbar).toBeTruthy()
    const toolbarWithin = within(toolbar as HTMLElement)
    expect(toolbarWithin.getByRole('button', { name: '+ Add' })).toBeTruthy()
    expect(toolbarWithin.getByRole('button', { name: 'Refresh' })).toBeTruthy()
    expect(toolbarWithin.getByRole('button', { name: 'Scheduler' })).toBeTruthy()
    expect(toolbarWithin.getByRole('button', { name: 'Log' })).toBeTruthy()
    expect(toolbarWithin.queryByRole('button', { name: /^(Edit|Download|P1|P2|Queue)$/ })).toBeNull()
  })

  it('shows build identity and offers the newer GitHub release after one automatic check', async () => {
    bridgeMocks.getAppBuildInfo.mockResolvedValue({
      version: '0.15.0',
      commitSha: 'd6a23804',
      dirty: false,
      channel: 'release',
      displayVersion: 'v0.15.0',
    })
    bridgeMocks.checkAppUpdate.mockResolvedValue({
      build: {
        version: '0.15.0',
        commitSha: 'd6a23804',
        dirty: false,
        channel: 'release',
        displayVersion: 'v0.15.0',
      },
      latestVersion: '0.16.0',
      releaseUrl: 'https://github.com/MetalDevOps/NinjaCrawler/releases/tag/v0.16.0',
      publishedAt: '2026-07-13T00:00:00Z',
      updateAvailable: true,
    })

    render(<App />)

    await waitFor(() => expect(bridgeMocks.checkAppUpdate).toHaveBeenCalledTimes(1))
    const versionButton = screen.getByRole('button', { name: /update available v0\.16\.0/i })
    fireEvent.click(versionButton)

    const downloadButton = screen.getByRole('button', { name: /view \/ download v0\.16\.0 on github/i })
    fireEvent.click(downloadButton)
    expect(bridgeMocks.openExternalTarget).toHaveBeenCalledWith(
      'https://github.com/MetalDevOps/NinjaCrawler/releases/tag/v0.16.0',
    )
  })
})
