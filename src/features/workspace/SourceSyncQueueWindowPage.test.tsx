// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SourceDeleteQueueStatus, SourceSyncQueueStatus } from '../../domain/models'
import { SourceSyncQueueWindowPage } from './SourceSyncQueueWindowPage'

const bridgeMocks = vi.hoisted(() => ({
  cancelSourceSyncProfile: vi.fn(),
  cancelSourceSyncProvider: vi.fn(),
  pauseSourceSyncProvider: vi.fn(),
  resumeSourceSyncProvider: vi.fn(),
  reorderSourceSyncProviderQueue: vi.fn(),
  runSourceSync: vi.fn(),
  loadSourceDeleteQueueStatus: vi.fn(),
  loadSourceSyncQueueStatus: vi.fn(),
  loadWorkspaceSnapshot: vi.fn(),
  loadMediaThumbnailQueueStatus: vi.fn(),
  enqueueMediaThumbnailGeneration: vi.fn(),
  openConnectorDebugWindow: vi.fn(),
  subscribeToDesktopRuntimeEvents: vi.fn(),
  loadSingleVideoQueueStatus: vi.fn(),
  subscribeToSingleVideoQueue: vi.fn(),
}))

vi.mock('../../bridge/desktop', () => bridgeMocks)
vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (path: string) => path }))

function statusFixture(overrides: Partial<SourceSyncQueueStatus> = {}): SourceSyncQueueStatus {
  return {
    queuedCount: 1,
    runningCount: 1,
    completedCount: 2,
    failedCount: 0,
    totalCount: 4,
    activeSourceId: 'source-1',
    activeHandle: '@active',
    activeProvider: 'instagram',
    activeStartedAt: '2026-03-11T12:00:00Z',
    providers: [
      {
        provider: 'instagram',
        displayName: 'Instagram',
        queued: 1,
        running: 1,
        completed: 2,
        failed: 0,
        total: 4,
        paused: false,
      },
    ],
    queuedItems: [],
    runningItems: [],
    recentResults: [],
    updatedAt: '2026-03-11T12:01:00Z',
    ...overrides,
  }
}

function deleteStatusFixture(overrides: Partial<SourceDeleteQueueStatus> = {}): SourceDeleteQueueStatus {
  return {
    queuedCount: 0,
    runningCount: 0,
    completedCount: 0,
    failedCount: 0,
    totalCount: 0,
    queuedItems: [],
    runningItems: [],
    recentResults: [],
    updatedAt: '2026-03-11T12:01:00Z',
    ...overrides,
  }
}

const runningSyncFixture = () =>
  statusFixture({
    queuedItems: [
      {
        sourceId: 'source-queued',
        provider: 'instagram',
        handle: '@queued',
        state: 'queued',
        queuedAt: '2026-03-11T12:00:00Z',
      },
    ],
    runningItems: [
      {
        sourceId: 'source-running',
        provider: 'instagram',
        handle: '@running',
        state: 'running',
        queuedAt: '2026-03-11T11:59:00Z',
        startedAt: '2026-03-11T12:00:00Z',
        progressLabel: 'Downloading profile',
        progressPercent: 40,
        progressIndeterminate: false,
      },
    ],
  })

describe('SourceSyncQueueWindowPage', () => {
  beforeEach(() => {
    for (const mock of Object.values(bridgeMocks)) {
      mock.mockReset()
    }
    bridgeMocks.cancelSourceSyncProfile.mockResolvedValue({})
    bridgeMocks.cancelSourceSyncProvider.mockResolvedValue({})
    bridgeMocks.pauseSourceSyncProvider.mockResolvedValue({})
    bridgeMocks.resumeSourceSyncProvider.mockResolvedValue({})
    bridgeMocks.reorderSourceSyncProviderQueue.mockResolvedValue({})
    bridgeMocks.runSourceSync.mockResolvedValue({})
    bridgeMocks.loadSourceSyncQueueStatus.mockResolvedValue(statusFixture())
    bridgeMocks.loadSourceDeleteQueueStatus.mockResolvedValue(deleteStatusFixture())
    bridgeMocks.loadWorkspaceSnapshot.mockResolvedValue({ sources: [], schedulerGroups: [] })
    bridgeMocks.loadMediaThumbnailQueueStatus.mockResolvedValue({
      queuedCount: 0,
      runningCount: 0,
      completedCount: 0,
      failedCount: 0,
      queuedItems: [],
      recentResults: [],
      updatedAt: '',
    })
    bridgeMocks.enqueueMediaThumbnailGeneration.mockResolvedValue({
      queuedCount: 0,
      runningCount: 0,
      completedCount: 0,
      failedCount: 0,
      queuedItems: [],
      recentResults: [],
      updatedAt: '',
    })
    bridgeMocks.openConnectorDebugWindow.mockResolvedValue(undefined)
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockResolvedValue(() => undefined)
    bridgeMocks.loadSingleVideoQueueStatus.mockResolvedValue({
      queuedCount: 0,
      runningCount: 0,
      completedCount: 0,
      failedCount: 0,
      queuedItems: [],
      recentResults: [],
      updatedAt: '',
    })
    bridgeMocks.subscribeToSingleVideoQueue.mockResolvedValue(() => undefined)
  })

  afterEach(() => {
    cleanup()
  })

  it('shows the summary strip and a provider lane', async () => {
    bridgeMocks.loadSourceSyncQueueStatus.mockResolvedValue(runningSyncFixture())
    render(<SourceSyncQueueWindowPage />)

    await waitFor(() => {
      expect(bridgeMocks.loadSourceSyncQueueStatus).toHaveBeenCalled()
      expect(bridgeMocks.loadSourceDeleteQueueStatus).toHaveBeenCalled()
    })

    expect(screen.getAllByText(/^Running$/i).length).toBeGreaterThan(0)
    expect(screen.getAllByText(/^Queued$/i).length).toBeGreaterThan(0)
    expect(screen.getAllByText(/^Done$/i).length).toBeGreaterThan(0)
    expect(screen.getAllByText(/^Failed$/i).length).toBeGreaterThan(0)
    // running + queued tasks rendered in the Instagram lane
    expect(await screen.findByText(/^@running$/i)).toBeTruthy()
    expect(await screen.findByText(/^@queued$/i)).toBeTruthy()
    // queue position tag for the next queued item
    expect(screen.getByText(/^Next$/i)).toBeTruthy()
  })

  it('opens the realtime backend debugger from the queue window', async () => {
    render(<SourceSyncQueueWindowPage />)

    fireEvent.click(await screen.findByRole('button', { name: /open realtime debugger/i }))

    await waitFor(() => {
      expect(bridgeMocks.openConnectorDebugWindow).toHaveBeenCalledTimes(1)
    })
  })

  it('shows the empty state when there is no activity', async () => {
    let queueEventHandler: ((status: SourceSyncQueueStatus) => void) | undefined
    bridgeMocks.loadSourceSyncQueueStatus.mockResolvedValue(runningSyncFixture())
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockImplementation(
      async (handlers: { onSourceSyncQueueChanged?: (status: SourceSyncQueueStatus) => void }) => {
        queueEventHandler = handlers.onSourceSyncQueueChanged
        return () => undefined
      },
    )

    render(<SourceSyncQueueWindowPage />)
    await waitFor(() => expect(queueEventHandler).toBeTypeOf('function'))

    queueEventHandler?.(
      statusFixture({
        queuedCount: 0,
        runningCount: 0,
        completedCount: 0,
        failedCount: 0,
        totalCount: 0,
        providers: [],
        queuedItems: [],
        runningItems: [],
        recentResults: [],
      }),
    )

    expect(await screen.findByText(/no active downloads/i)).toBeTruthy()
  })

  it('cancels the provider lane and an individual job', async () => {
    bridgeMocks.loadSourceSyncQueueStatus.mockResolvedValue(runningSyncFixture())
    render(<SourceSyncQueueWindowPage />)

    await screen.findByText(/^@running$/i)

    fireEvent.click(await screen.findByRole('button', { name: /cancel all/i }))
    await waitFor(() => {
      expect(bridgeMocks.cancelSourceSyncProvider).toHaveBeenCalledWith('instagram')
    })

    const runningRow = screen.getByText(/^@running$/i).closest('.queue-task-row') as HTMLElement | null
    expect(runningRow).toBeTruthy()
    fireEvent.click(within(runningRow!).getByRole('button', { name: /^cancel$/i }))
    await waitFor(() => {
      expect(bridgeMocks.cancelSourceSyncProfile).toHaveBeenCalledWith('source-running')
    })
  })

  it('pauses the provider lane', async () => {
    bridgeMocks.loadSourceSyncQueueStatus.mockResolvedValue(runningSyncFixture())
    render(<SourceSyncQueueWindowPage />)
    await screen.findByText(/^@running$/i)

    fireEvent.click(await screen.findByRole('button', { name: /^pause$/i }))
    await waitFor(() => {
      expect(bridgeMocks.pauseSourceSyncProvider).toHaveBeenCalledWith('instagram')
    })
  })

  it('retries a finished sync from the recent panel', async () => {
    bridgeMocks.loadSourceSyncQueueStatus.mockResolvedValue(
      statusFixture({
        recentResults: [
          {
            sourceId: 'source-finished',
            provider: 'instagram',
            handle: '@finished',
            status: 'failed',
            summary: 'Instagram sync failed.',
            finishedAt: '2026-03-11T12:05:00Z',
          },
        ],
      }),
    )
    render(<SourceSyncQueueWindowPage />)
    await screen.findByText(/^@finished$/i)

    fireEvent.click(await screen.findByRole('button', { name: /retry/i }))
    await waitFor(() => {
      expect(bridgeMocks.runSourceSync).toHaveBeenCalledWith('source-finished', { trigger: 'manual' })
    })
  })

  it('reorders the queue with the move buttons', async () => {
    bridgeMocks.loadSourceSyncQueueStatus.mockResolvedValue(
      statusFixture({
        queuedCount: 2,
        runningCount: 0,
        queuedItems: [
          { jobKey: 'source-1:instagram-story:111', sourceId: 'source-1', provider: 'instagram', handle: '@q1', state: 'queued', queuedAt: '2026-03-11T12:00:00Z' },
          { jobKey: 'source-1:instagram-story:222', sourceId: 'source-1', provider: 'instagram', handle: '@q2', state: 'queued', queuedAt: '2026-03-11T12:01:00Z' },
        ],
        runningItems: [],
      }),
    )

    render(<SourceSyncQueueWindowPage />)

    const firstRow = (await screen.findByText(/^@q1$/i)).closest('.queue-task-row') as HTMLElement
    expect(firstRow).toBeTruthy()

    // move @q1 (topo) para baixo: troca com @q2
    fireEvent.click(within(firstRow).getByRole('button', { name: /move down in queue/i }))

    await waitFor(() => {
      expect(bridgeMocks.reorderSourceSyncProviderQueue).toHaveBeenCalledWith('instagram', [
        'source-1:instagram-story:222',
        'source-1:instagram-story:111',
      ])
    })
  })

  it('renders sync and delete work together in the same lane', async () => {
    bridgeMocks.loadSourceSyncQueueStatus.mockResolvedValue(runningSyncFixture())
    bridgeMocks.loadSourceDeleteQueueStatus.mockResolvedValue(
      deleteStatusFixture({
        runningCount: 1,
        totalCount: 1,
        runningItems: [
          {
            jobId: 'delete-running-1',
            sourceId: 'source-delete-1',
            provider: 'instagram',
            handle: '@delete-me',
            mode: 'with_media',
            state: 'running',
            queuedAt: '2026-03-11T12:00:00Z',
            startedAt: '2026-03-11T12:00:30Z',
            progressLabel: 'Removing files',
            progressDetail: 'phase 2/4',
            progressPercent: 45,
            progressIndeterminate: false,
            filesProcessed: 9,
            filesTotal: 20,
          },
        ],
      }),
    )

    render(<SourceSyncQueueWindowPage />)

    expect(await screen.findByText(/^@running$/i)).toBeTruthy()
    expect(await screen.findByText(/^@delete-me$/i)).toBeTruthy()
    expect(screen.getAllByText(/^Delete/i).length).toBeGreaterThan(0)
    expect(screen.getByText(/removing files · phase 2\/4 · files 9\/20 · 45%/i)).toBeTruthy()
  })

  it('queues missing thumbnails for a provider scope', async () => {
    bridgeMocks.loadWorkspaceSnapshot.mockResolvedValue({
      sources: [
        { id: 'tk-1', provider: 'tiktok', handle: '@one' },
        { id: 'tk-2', provider: 'tiktok', handle: '@two' },
        { id: 'ig-1', provider: 'instagram', handle: '@three' },
      ],
      schedulerGroups: [],
    })
    render(<SourceSyncQueueWindowPage />)

    fireEvent.change(await screen.findByLabelText('Scope'), { target: { value: 'provider' } })
    fireEvent.change(screen.getByLabelText('Provider'), { target: { value: 'tiktok' } })
    fireEvent.click(screen.getByRole('button', { name: /generate missing thumbnails/i }))

    await waitFor(() => {
      expect(bridgeMocks.enqueueMediaThumbnailGeneration).toHaveBeenCalledWith(['tk-1', 'tk-2'])
    })
  })
})
