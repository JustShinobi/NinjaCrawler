// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createEmptyWorkspaceSnapshot } from '../../domain/workspaceSnapshot'
import { ImportWindowPage } from './ImportWindowPage'

const bridgeMocks = vi.hoisted(() => ({
  listImportProviders: vi.fn(),
  listImportMethods: vi.fn(),
  listImportRoots: vi.fn(),
  loadWorkspaceSnapshot: vi.fn(),
  loadImportQueueStatus: vi.fn(),
  enqueueImportPreview: vi.fn(),
  enqueueImportRun: vi.fn(),
  enqueueImportBackfill: vi.fn(),
  pickImportRootFolder: vi.fn(),
  subscribeToDesktopRuntimeEvents: vi.fn(),
  upsertAppSetting: vi.fn(),
}))

vi.mock('../../bridge/desktop', () => bridgeMocks)
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    close: vi.fn(),
    isFocused: () => Promise.resolve(true),
    isMaximized: () => Promise.resolve(false),
    minimize: vi.fn(),
    onFocusChanged: () => Promise.resolve(() => undefined),
    onResized: () => Promise.resolve(() => undefined),
    startDragging: vi.fn(),
    toggleMaximize: vi.fn(),
    setTitle: vi.fn(() => Promise.resolve()),
  }),
}))

describe('ImportWindowPage', () => {
  let runtimeHandlers: { onImportQueueChanged?: (status: unknown) => void } = {}

  beforeEach(() => {
    cleanup()
    bridgeMocks.listImportProviders.mockReset()
    bridgeMocks.listImportMethods.mockReset()
    bridgeMocks.listImportRoots.mockReset()
    bridgeMocks.loadWorkspaceSnapshot.mockReset()
    bridgeMocks.loadImportQueueStatus.mockReset()
    bridgeMocks.enqueueImportPreview.mockReset()
    bridgeMocks.enqueueImportRun.mockReset()
    bridgeMocks.enqueueImportBackfill.mockReset()
    bridgeMocks.pickImportRootFolder.mockReset()
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockReset()
    bridgeMocks.upsertAppSetting.mockReset()
    runtimeHandlers = {}

    bridgeMocks.listImportProviders.mockResolvedValue([
      {
        key: 'instagram',
        displayName: 'Instagram',
        description: 'Legacy imports',
      },
    ])
    bridgeMocks.listImportMethods.mockResolvedValue([
      {
        importerId: 'instagram.scrawler',
        provider: 'instagram',
        label: 'SCrawler',
        description: 'Imports from legacy folders',
      },
    ])
    bridgeMocks.listImportRoots.mockResolvedValue([
      {
        path: 'D:\\Media\\Instagram',
        source: 'default',
        label: 'Media root',
        removable: false,
      },
    ])

    const snapshot = createEmptyWorkspaceSnapshot()
    snapshot.accounts = [
      {
        id: 'account-1',
        provider: 'instagram',
        displayName: 'Main account',
        authMode: 'imported_session',
        authState: 'ready',
        capabilities: [],
        lastValidatedAt: '2026-03-12T10:00:00Z',
      },
    ]
    bridgeMocks.loadWorkspaceSnapshot.mockResolvedValue(snapshot)
    bridgeMocks.upsertAppSetting.mockImplementation(async (draft) => ({
      ...snapshot,
      appSettings: [
        ...snapshot.appSettings.filter((setting) => setting.key !== draft.key),
        {
          key: draft.key,
          value: draft.value,
          category: draft.category ?? 'imports',
          description: draft.description,
          mutable: draft.mutable ?? true,
        },
      ],
    }))
    bridgeMocks.loadImportQueueStatus.mockResolvedValue({
      queuedCount: 0,
      runningCount: 0,
      completedCount: 0,
      failedCount: 0,
      totalCount: 0,
      queuedItems: [],
      runningItems: [],
      recentResults: [],
      updatedAt: '2026-03-12T10:00:00Z',
    })
    bridgeMocks.enqueueImportPreview.mockResolvedValue({
      queuedCount: 1,
      runningCount: 0,
      completedCount: 0,
      failedCount: 0,
      totalCount: 1,
      queuedItems: [
        {
          jobId: 'job-preview-1',
          importerId: 'instagram.scrawler',
          provider: 'instagram',
          methodLabel: 'SCrawler',
          jobKind: 'preview',
          queuedAt: '2026-03-12T10:01:00Z',
        },
      ],
      runningItems: [],
      recentResults: [],
      updatedAt: '2026-03-12T10:01:00Z',
    })
    bridgeMocks.enqueueImportBackfill.mockResolvedValue({
      queuedCount: 1,
      runningCount: 0,
      completedCount: 0,
      failedCount: 0,
      totalCount: 1,
      queuedItems: [
        {
          jobId: 'job-backfill-1',
          importerId: 'instagram.scrawler',
          provider: 'instagram',
          methodLabel: 'SCrawler',
          jobKind: 'backfill',
          queuedAt: '2026-03-12T10:01:00Z',
        },
      ],
      runningItems: [],
      recentResults: [],
      updatedAt: '2026-03-12T10:01:00Z',
    })
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockImplementation(async (handlers) => {
      runtimeHandlers = handlers
      return () => undefined
    })
  })

  it('queues the dry-run and renders review rows when the queue event arrives', async () => {
    const preview = {
      importerId: 'instagram.scrawler',
      provider: 'instagram',
      methodLabel: 'SCrawler',
      forceReimport: false,
      roots: ['D:\\Media\\Instagram'],
      profiles: [
        {
          profileRoot: 'D:\\Media\\Instagram\\alpha',
          userXmlPath: 'D:\\Media\\Instagram\\alpha\\Settings\\User_Instagram_alpha.xml',
          handle: 'alpha',
          displayName: 'Alpha',
          accountName: 'Legacy account',
          alreadyImported: false,
          importState: 'needs_account_link',
          fileCount: 12,
          alreadyCatalogedCount: 4,
          newFileCount: 8,
          problems: [
            {
              severity: 'error',
              code: 'account-match-missing',
              message: 'Link an account.',
            },
          ],
        },
      ],
      summary: {
        detectedProfiles: 1,
        readyProfiles: 0,
        blockedProfiles: 1,
        alreadyImportedProfiles: 0,
        importableFiles: 12,
      },
    }

    render(<ImportWindowPage />)

    await waitFor(() => expect(bridgeMocks.listImportProviders).toHaveBeenCalledTimes(1))
    expect(await screen.findByText('D:\\Media\\Instagram')).toBeTruthy()
    expect(screen.queryByRole('tablist', { name: 'Import providers' })).toBeNull()

    fireEvent.change(screen.getByLabelText('Manual import root'), {
      target: { value: 'D:\\SCrawler\\Data\\Instagram' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Add root' }))
    fireEvent.click(screen.getByRole('button', { name: 'Run scan' }))

    await waitFor(() => expect(bridgeMocks.enqueueImportPreview).toHaveBeenCalledTimes(1))
    await waitFor(() =>
      expect(bridgeMocks.listImportRoots).toHaveBeenLastCalledWith('instagram.scrawler', ['D:\\SCrawler\\Data\\Instagram'], []),
    )
    expect(bridgeMocks.upsertAppSetting).toHaveBeenCalledWith({
      key: 'imports.instagram.scrawler.manualRoots',
      value: JSON.stringify(['D:\\SCrawler\\Data\\Instagram']),
      category: 'imports',
      description: 'Persisted manual scan roots for external import review.',
      mutable: true,
    })
    expect(bridgeMocks.enqueueImportPreview).toHaveBeenCalledWith('instagram.scrawler', {
      forceReimport: false,
      manualRoots: ['D:\\SCrawler\\Data\\Instagram'],
      disabledRoots: [],
    })
    await waitFor(() => expect(screen.getAllByText('1 queued').length).toBeGreaterThan(0))

    runtimeHandlers.onImportQueueChanged?.({
      queuedCount: 0,
      runningCount: 1,
      completedCount: 0,
      failedCount: 0,
      totalCount: 1,
      queuedItems: [],
      runningItems: [
        {
          jobId: 'job-preview-1',
          importerId: 'instagram.scrawler',
          provider: 'instagram',
          methodLabel: 'SCrawler',
          jobKind: 'preview',
          queuedAt: '2026-03-12T10:01:00Z',
          startedAt: '2026-03-12T10:01:05Z',
          progressLabel: 'Scanning folders',
          progressDetail: 'Scanning legacy folders and matching accounts.',
        },
      ],
      recentResults: [],
      latestPreview: preview,
      updatedAt: '2026-03-12T10:01:05Z',
    })

    expect(await screen.findByText('alpha')).toBeTruthy()
    expect(screen.getAllByText('Link account').length).toBeGreaterThan(0)
    expect(screen.getByText('Link an account.')).toBeTruthy()
    expect(screen.getByRole('option', { name: 'Main account' })).toBeTruthy()
    expect(screen.getByText('Scanning folders')).toBeTruthy()
  })

  it('removes a managed root from import by persisting it as disabled', async () => {
    const snapshot = createEmptyWorkspaceSnapshot()
    snapshot.accounts = [
      {
        id: 'account-1',
        provider: 'instagram',
        displayName: 'Main account',
        authMode: 'imported_session',
        authState: 'ready',
        capabilities: [],
        lastValidatedAt: '2026-03-12T10:00:00Z',
      },
    ]
    bridgeMocks.loadWorkspaceSnapshot.mockResolvedValue(snapshot)
    bridgeMocks.listImportRoots.mockResolvedValue([
      {
        path: 'F:\\SCrawler\\Data\\Instagram',
        source: 'default',
        label: 'Media root',
        removable: false,
      },
    ])
    bridgeMocks.upsertAppSetting.mockImplementation(async (draft) => ({
      ...snapshot,
      appSettings: [
        {
          key: draft.key,
          value: draft.value,
          category: draft.category ?? 'imports',
          description: draft.description,
          mutable: draft.mutable ?? true,
        },
      ],
    }))

    render(<ImportWindowPage />)

    expect(await screen.findByText('F:\\SCrawler\\Data\\Instagram')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Remove' }))

    await waitFor(() =>
      expect(bridgeMocks.upsertAppSetting).toHaveBeenCalledWith({
        key: 'imports.instagram.scrawler.disabledRoots',
        value: JSON.stringify(['F:\\SCrawler\\Data\\Instagram']),
        category: 'imports',
        description: 'Disabled scan roots for external import review.',
        mutable: true,
      }),
    )
    await waitFor(() =>
      expect(bridgeMocks.listImportRoots).toHaveBeenLastCalledWith('instagram.scrawler', [], ['F:\\SCrawler\\Data\\Instagram']),
    )
  })

  it('rejects adding a manual root that is already covered by a managed root', async () => {
    bridgeMocks.listImportRoots.mockResolvedValue([
      {
        path: 'F:\\SCrawler\\Data\\instagram',
        source: 'default',
        label: 'Media root',
        removable: false,
      },
    ])

    render(<ImportWindowPage />)

    expect(await screen.findByText('F:\\SCrawler\\Data\\instagram')).toBeTruthy()
    fireEvent.change(screen.getByLabelText('Manual import root'), {
      target: { value: 'F:\\SCrawler\\Data\\Instagram\\' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Add root' }))

    expect(await screen.findByText('This path is already active in import roots. Remove it from the list first if you want it excluded.')).toBeTruthy()
    expect(bridgeMocks.upsertAppSetting).not.toHaveBeenCalled()
    await waitFor(() =>
      expect(bridgeMocks.listImportRoots).toHaveBeenLastCalledWith('instagram.scrawler', [], []),
    )
  })

  it('queues naming backfill from the import toolbar', async () => {
    render(<ImportWindowPage />)

    const button = await screen.findByRole('button', { name: 'Run naming backfill' })
    fireEvent.click(button)

    await waitFor(() => expect(bridgeMocks.enqueueImportBackfill).toHaveBeenCalledTimes(1))
    expect(bridgeMocks.enqueueImportBackfill).toHaveBeenCalledWith('instagram.scrawler')
    await waitFor(() => expect(screen.getAllByText('1 queued').length).toBeGreaterThan(0))
  })
})
