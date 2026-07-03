// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App'
import { serializeInstagramGlobalSyncPreset } from './domain/sourceSyncOptions'
import type { WorkspaceSnapshot } from './domain/models'
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
const pickProfileImageMock = vi.fn()
const resetProfileImageMock = vi.fn()
const openSourceFolderMock = vi.fn()
const upsertSourceProfileMock = vi.fn()

let currentSnapshot: WorkspaceSnapshot = createEmptyWorkspaceSnapshot()

const bridgeMocks = vi.hoisted(() => ({
  checkSourceAvailability: vi.fn(),
  loadSourceDeleteQueueStatus: vi.fn(),
  loadSourceSyncQueueStatus: vi.fn(),
  openAccountsWindow: vi.fn(),
  openConnectorRuntimesWindow: vi.fn(),
  openExternalTarget: vi.fn(),
  openImportWindow: vi.fn(),
  openRuntimeLogWindow: vi.fn(),
  openSourceEditorWindow: vi.fn(),
  openSourceSyncQueueWindow: vi.fn(),
  subscribeToDesktopRuntimeEvents: vi.fn(),
  subscribeToFocusSourceRequest: vi.fn(() => Promise.resolve(() => undefined)),
}))

vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({ label: 'main' }),
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
vi.mock('./features/workspace/ProfileWorkspace', () => ({
  ProfileWorkspace: ({
    snapshot,
    onSelectSource,
    onOpenSourceContextMenu,
  }: {
    snapshot: WorkspaceSnapshot
    onSelectSource: (id: string, options?: { append?: boolean }) => void
    onOpenSourceContextMenu: (sourceId: string, x: number, y: number, preserveSelection: boolean) => void
  }) => (
    <div>
      <button
        onClick={() => {
          snapshot.sources.forEach((source, index) => {
            onSelectSource(source.id, index === 0 ? undefined : { append: true })
          })
        }}
        type="button"
      >
        Select all sources
      </button>
      <button
        onClick={() => {
          const first = snapshot.sources[0]
          if (first) {
            onOpenSourceContextMenu(first.id, 10, 10, false)
          }
        }}
        type="button"
      >
        Open source context menu
      </button>
    </div>
  ),
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
  buildServiceTabs: () => [{ key: 'all', label: 'All', count: currentSnapshot.sources.length }],
  filterSourcesForWorkspace: () => currentSnapshot.sources,
  formatSourceHandleLabel: (handle: string) => handle,
  parseClipboardProfileSeed: () => undefined,
}))
vi.mock('./state/appStore', () => ({
  useAppStore: (selector: (state: Record<string, unknown>) => unknown) =>
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
      snapshot: currentSnapshot,
      error: undefined,
      cloneProviderAccount: cloneProviderAccountMock,
      deleteProviderAccount: deleteProviderAccountMock,
      deleteSourceProfile: deleteSourceProfileMock,
      pickSourceProfileImage: pickProfileImageMock,
      resetSourceProfileImage: resetProfileImageMock,
      openSourceFolder: openSourceFolderMock,
      upsertSourceProfile: upsertSourceProfileMock,
    }),
}))

function createSource(id: string, handle: string, overrides?: Partial<WorkspaceSnapshot['sources'][number]>) {
  return {
    id,
    provider: 'instagram' as const,
    sourceKind: 'profile' as const,
    handle,
    displayName: handle,
    labels: [],
    readyForDownload: true,
    remoteState: 'exists' as const,
    isSubscription: false,
    profileImageCustom: false,
    ...overrides,
  }
}

describe('App batch sync summary', () => {
  beforeEach(() => {
    applySnapshotMock.mockReset()
    bootstrapMock.mockReset()
    bridgeMocks.checkSourceAvailability.mockReset()
    bridgeMocks.loadSourceDeleteQueueStatus.mockReset()
    bridgeMocks.loadSourceSyncQueueStatus.mockReset()
    bridgeMocks.openConnectorRuntimesWindow.mockReset()
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockReset()
    runSourceSyncMock.mockReset()

    currentSnapshot = createEmptyWorkspaceSnapshot()
    bridgeMocks.loadSourceDeleteQueueStatus.mockResolvedValue({
      queuedCount: 0,
      runningCount: 0,
      completedCount: 0,
      failedCount: 0,
      totalCount: 0,
      queuedItems: [],
      runningItems: [],
      recentResults: [],
      updatedAt: '2026-03-14T00:00:00Z',
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
      updatedAt: '2026-03-14T00:00:00Z',
    })
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockResolvedValue(() => undefined)
    bridgeMocks.checkSourceAvailability.mockImplementation(async (sourceIds: string[]) => ({
      snapshot: currentSnapshot,
      requested: sourceIds.length,
      processed: sourceIds.length,
      unchanged: sourceIds.length,
      updatedHandle: 0,
      markedProblem: 0,
      skipped: 0,
      failed: 0,
      items: sourceIds.map((sourceId) => ({
        sourceId,
        provider: 'instagram',
        previousHandle: sourceId,
        currentHandle: sourceId,
        status: 'unchanged',
        message: 'Profile is still available with the same handle.',
      })),
    }))
    vi.spyOn(window, 'alert').mockImplementation(() => undefined)
    window.history.replaceState({}, '', '/')
  })

  afterEach(() => {
    cleanup()
    vi.restoreAllMocks()
    window.history.replaceState({}, '', '/')
  })

  it('enqueues a preset run without showing a summary dialog', async () => {
    currentSnapshot.appSettings = [
      {
        key: 'instagram.sync.globalPreset1',
        value: serializeInstagramGlobalSyncPreset('preset1', {
          enabled: true,
          label: 'Preset 1',
          sections: {
            timeline: true,
            reels: false,
            stories: false,
            storiesUser: false,
            tagged: false,
          },
        }),
        category: 'policy',
        mutable: true,
      },
    ]
    currentSnapshot.sources = [
      createSource('ig-1', '@ig-1'),
      createSource('tw-1', '@tw-1', { provider: 'twitter' }),
    ]
    runSourceSyncMock.mockResolvedValue(undefined)

    render(<App />)

    fireEvent.click(screen.getByRole('button', { name: /select all sources/i }))
    const presetButton = screen.getByRole('button', { name: 'P1' }) as HTMLButtonElement
    await waitFor(() => expect(presetButton.disabled).toBe(false))

    fireEvent.click(presetButton)

    // Apenas enfileira o source suportado; nenhum modal de resumo aparece.
    await waitFor(() => {
      expect(runSourceSyncMock).toHaveBeenCalledWith('ig-1', expect.objectContaining({ trigger: 'manual_preset_1' }))
    })
    expect(runSourceSyncMock).not.toHaveBeenCalledWith('tw-1', expect.anything())
    expect(screen.queryByRole('dialog', { name: /sync summary/i })).toBeNull()
    expect(window.alert).not.toHaveBeenCalled()
  })

  it('does not show a summary dialog when a regular batch enqueue fails', async () => {
    currentSnapshot.sources = [createSource('failed', '@failed')]
    runSourceSyncMock.mockRejectedValue(new Error('Queue unavailable'))

    render(<App />)

    fireEvent.click(screen.getByRole('button', { name: /select all sources/i }))
    const toolbar = document.querySelector('.toolbar-strip')
    expect(toolbar).toBeTruthy()
    const downloadButton = within(toolbar as HTMLElement).getByRole('button', { name: 'Download' }) as HTMLButtonElement
    await waitFor(() => expect(downloadButton.disabled).toBe(false))

    fireEvent.click(downloadButton)

    await waitFor(() => {
      expect(runSourceSyncMock).toHaveBeenCalledWith('failed', undefined)
    })
    expect(screen.queryByRole('dialog', { name: /sync summary/i })).toBeNull()
    expect(window.alert).not.toHaveBeenCalled()
  })

  it('runs availability check from context menu and shows the summary dialog', async () => {
    currentSnapshot.sources = [createSource('source-1', '@source-1')]

    render(<App />)

    fireEvent.click(screen.getByRole('button', { name: /open source context menu/i }))
    const contextMenu = await screen.findByRole('menu')
    fireEvent.click(within(contextMenu).getByRole('menuitem', { name: /check availability/i }))

    await waitFor(() => {
      expect(bridgeMocks.checkSourceAvailability).toHaveBeenCalledWith(['source-1'])
    })

    expect(applySnapshotMock).toHaveBeenCalled()
    expect(screen.getByRole('dialog', { name: /availability check summary/i })).toBeTruthy()
    expect(screen.getByText('Profile is still available with the same handle.')).toBeTruthy()
  })

  it('keeps the connector status action clickable from the status bar', async () => {
    currentSnapshot.connectorRuntimes = [{
      key: 'instagram',
      displayName: 'Instagram',
      managementMode: 'managed',
      activeVersion: '1.0.0',
      bundledVersion: '1.0.0',
      updateAvailable: false,
      status: 'up_to_date',
    }]

    const view = render(<App />)
    const statusBar = view.container.querySelector('.status-bar')
    expect(statusBar).toBeTruthy()
    expect(statusBar?.querySelectorAll('.status-cell')).toHaveLength(6)

    const connectorCell = statusBar?.querySelector('.status-cell-connector')
    expect(connectorCell).toBeTruthy()
    expect(connectorCell?.querySelector('.status-connector-button')).toBeTruthy()

    fireEvent.click(within(statusBar as HTMLElement).getByRole('button', { name: /connectors/i }))

    await waitFor(() => {
      expect(bridgeMocks.openConnectorRuntimesWindow).toHaveBeenCalledTimes(1)
    })
  })
})
