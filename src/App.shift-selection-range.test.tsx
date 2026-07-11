// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App'
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
  ProfileWorkspace: ({ onSelectSource, selectedSourceIds }: { onSelectSource: (id: string, options?: { append?: boolean; range?: boolean; visibleIds?: string[] }) => void; selectedSourceIds: string[] }) => {
    const visibleIds = ['source-a', 'source-e', 'source-b', 'source-c', 'source-d']

    return (
      <div>
        <output aria-label="Selected sources">{selectedSourceIds.join(',')}</output>
        <button
          onClick={() => onSelectSource('source-a')}
          type="button"
        >
          Select single source
        </button>
        <button
          onClick={() => onSelectSource('source-a')}
          type="button"
        >
          Toggle same source
        </button>
        <button
          onClick={() => onSelectSource('source-e', { visibleIds })}
          type="button"
        >
          Select anchor source
        </button>
        <button
          onClick={() => onSelectSource('source-c', { range: true, visibleIds })}
          type="button"
        >
          Shift select source
        </button>
      </div>
    )
  },
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
  filterSourcesForWorkspace: (_sources: unknown, _serviceTab: string, searchText: string) => {
    const query = searchText.trim().toLowerCase()
    return query
      ? currentSnapshot.sources.filter((source) => source.handle.toLowerCase().includes(query))
      : currentSnapshot.sources
  },
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

function createSource(id: string, handle: string) {
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
  }
}

describe('App shift selection range', () => {
  beforeEach(() => {
    applySnapshotMock.mockReset()
    bootstrapMock.mockReset()
    bridgeMocks.loadSourceDeleteQueueStatus.mockReset()
    bridgeMocks.loadSourceSyncQueueStatus.mockReset()
    bridgeMocks.openConnectorRuntimesWindow.mockReset()
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockReset()
    runSourceSyncMock.mockReset()

    currentSnapshot = createEmptyWorkspaceSnapshot()
    currentSnapshot.sources = [
      createSource('source-a', '@a'),
      createSource('source-b', '@b'),
      createSource('source-c', '@c'),
      createSource('source-d', '@d'),
      createSource('source-e', '@e'),
    ]

    bridgeMocks.loadSourceDeleteQueueStatus.mockResolvedValue({
      queuedCount: 0,
      runningCount: 0,
      completedCount: 0,
      failedCount: 0,
      totalCount: 0,
      queuedItems: [],
      runningItems: [],
      recentResults: [],
      updatedAt: '2026-03-19T00:00:00Z',
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
      updatedAt: '2026-03-19T00:00:00Z',
    })
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockResolvedValue(() => undefined)

    runSourceSyncMock.mockImplementation(async () => currentSnapshot)
  })

  afterEach(() => {
    cleanup()
    vi.restoreAllMocks()
  })

  it('uses visible UI order for Shift+click range selection', async () => {
    render(<App />)

    fireEvent.click(screen.getByRole('button', { name: 'Select anchor source' }))
    fireEvent.click(screen.getByRole('button', { name: 'Shift select source' }))

    const toolbar = document.querySelector('.toolbar-strip')
    expect(toolbar).toBeTruthy()
    const downloadButton = within(toolbar as HTMLElement).getByRole('button', { name: 'Download' }) as HTMLButtonElement

    await waitFor(() => expect(downloadButton.disabled).toBe(false))

    fireEvent.click(downloadButton)

    await waitFor(() => expect(runSourceSyncMock).toHaveBeenCalledTimes(3))
    expect(runSourceSyncMock.mock.calls.map(([sourceId]) => sourceId)).toEqual([
      'source-e',
      'source-b',
      'source-c',
    ])
  })

  it('deselects when clicking the same single profile twice', async () => {
    render(<App />)

    fireEvent.click(screen.getByRole('button', { name: 'Select single source' }))
    fireEvent.click(screen.getByRole('button', { name: 'Toggle same source' }))

    const toolbar = document.querySelector('.toolbar-strip')
    expect(toolbar).toBeTruthy()
    const downloadButton = within(toolbar as HTMLElement).getByRole('button', { name: 'Download' }) as HTMLButtonElement

    await waitFor(() => expect(downloadButton.disabled).toBe(true))
  })

  it('selects every currently filtered profile with Ctrl+A', async () => {
    currentSnapshot.sources[0].handle = '@match-a'
    currentSnapshot.sources[2].handle = '@match-c'
    render(<App />)

    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'match' } })
    fireEvent.keyDown(document, { key: 'a', ctrlKey: true })

    await waitFor(() => {
      expect(screen.getByLabelText('Selected sources').textContent).toBe(
        'source-a,source-c',
      )
    })
  })
})
