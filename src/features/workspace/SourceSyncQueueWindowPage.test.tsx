// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SourceDeleteQueueStatus, SourceSyncQueueStatus } from '../../domain/models'
import { SourceSyncQueueWindowPage } from './SourceSyncQueueWindowPage'

const bridgeMocks = vi.hoisted(() => ({
  cancelSourceSyncProfile: vi.fn(),
  cancelSourceSyncProvider: vi.fn(),
  cancelMediaPathMigrations: vi.fn(),
  pauseSourceSyncProvider: vi.fn(),
  resumeSourceSyncProvider: vi.fn(),
  reorderSourceSyncProviderQueue: vi.fn(),
  runSourceSync: vi.fn(),
  loadSourceDeleteQueueStatus: vi.fn(),
  loadSourceSyncQueueStatus: vi.fn(),
  loadWorkspaceSnapshot: vi.fn(),
  loadMediaThumbnailQueueStatus: vi.fn(),
  loadMediaPathMigrationQueueStatus: vi.fn(),
  loadMediaDedupeStatus: vi.fn(),
  enqueueMediaThumbnailGeneration: vi.fn(),
  openConnectorDebugWindow: vi.fn(),
  openWorkspaceHealthWindow: vi.fn(),
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
    bridgeMocks.cancelMediaPathMigrations.mockResolvedValue({
      queuedCount: 0, runningCount: 1, completedCount: 0, failedCount: 0, totalCount: 1,
      queuedItems: [], runningItems: [], recentResults: [], updatedAt: '',
    })
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
    bridgeMocks.loadMediaPathMigrationQueueStatus.mockResolvedValue({
      queuedCount: 0,
      runningCount: 0,
      completedCount: 0,
      failedCount: 0,
      totalCount: 0,
      queuedItems: [],
      runningItems: [],
      recentResults: [],
      updatedAt: '',
    })
    bridgeMocks.loadMediaDedupeStatus.mockResolvedValue({
      state: 'idle', stage: 'idle', filesProcessed: 0, filesTotal: 0,
      bytesProcessed: 0, bytesTotal: 0, cancellable: false, updatedAt: '',
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
    bridgeMocks.openWorkspaceHealthWindow.mockResolvedValue(undefined)
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

  it('shows an automatic Account hold with its retry deadline', async () => {
    bridgeMocks.loadSourceSyncQueueStatus.mockResolvedValue(
      statusFixture({
        queuedCount: 1,
        runningCount: 0,
        providers: [{
          provider: 'twitter',
          displayName: 'X / Twitter',
          queued: 1,
          running: 0,
          completed: 0,
          failed: 0,
          total: 1,
          paused: false,
        }],
        queuedItems: [{
          sourceId: 'twitter-held',
          provider: 'twitter',
          handle: '@held',
          state: 'held',
          queuedAt: '2026-07-11T00:00:00Z',
          progressLabel: 'On hold',
          progressDetail: 'Twitter Account rate limit.',
          holdUntil: '2026-07-11T00:15:30Z',
        }],
        runningItems: [],
      }),
    )
    render(<SourceSyncQueueWindowPage />)

    expect(await screen.findByText('Account hold')).toBeTruthy()
    expect(screen.getByText(/On hold.*retry after/i)).toBeTruthy()
    expect(screen.getByText(/Twitter Account rate limit/i)).toBeTruthy()
  })

  it('shows sync jobs waiting for a media-path migration as Migration hold', async () => {
    bridgeMocks.loadSourceSyncQueueStatus.mockResolvedValue(
      statusFixture({
        queuedCount: 1,
        runningCount: 0,
        providers: [{ provider: 'twitter', displayName: 'X / Twitter', queued: 1, running: 0, completed: 0, failed: 0, total: 1, paused: false }],
        queuedItems: [{
          sourceId: 'twitter-migrating', provider: 'twitter', handle: '@moving', state: 'held',
          queuedAt: '2026-07-12T00:00:00Z', progressLabel: 'Waiting for media move',
          progressDetail: "This sync will start automatically after the profile's media-path migration finishes.",
        }],
        runningItems: [],
      }),
    )
    render(<SourceSyncQueueWindowPage />)

    expect(await screen.findByText('Migration hold')).toBeTruthy()
    expect(screen.getByText(/start automatically after.*migration finishes/i)).toBeTruthy()
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

  it('reorders the queue with Alt+ArrowDown on the drag handle', async () => {
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

    const handle = within(firstRow).getByRole('button', { name: /drag to reorder/i })
    fireEvent.keyDown(handle, { key: 'ArrowDown', altKey: true })

    await waitFor(() => {
      expect(bridgeMocks.reorderSourceSyncProviderQueue).toHaveBeenCalledWith('instagram', [
        'source-1:instagram-story:222',
        'source-1:instagram-story:111',
      ])
    })
  })

  it('reorders the queue with pointer drag on the handle', async () => {
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
    const secondRow = (await screen.findByText(/^@q2$/i)).closest('.queue-task-row') as HTMLElement
    expect(firstRow).toBeTruthy()
    expect(secondRow).toBeTruthy()

    const handle = within(firstRow).getByRole('button', { name: /drag to reorder/i })
    const elementFromPointMock = vi.fn(() => secondRow)
    Object.defineProperty(document, 'elementFromPoint', {
      configurable: true,
      writable: true,
      value: elementFromPointMock,
    })

    fireEvent.pointerDown(handle, { button: 0, pointerId: 1, clientX: 20, clientY: 40 })
    fireEvent.pointerMove(window, { pointerId: 1, clientX: 50, clientY: 130 })
    fireEvent.pointerUp(window, { pointerId: 1, clientX: 50, clientY: 130 })

    await waitFor(() => {
      expect(bridgeMocks.reorderSourceSyncProviderQueue).toHaveBeenCalledWith('instagram', [
        'source-1:instagram-story:222',
        'source-1:instagram-story:111',
      ])
    })
  })

  it('shows retry only for failed sync results', async () => {
    bridgeMocks.loadSourceSyncQueueStatus.mockResolvedValue(
      statusFixture({
        recentResults: [
          {
            sourceId: 'source-ok',
            provider: 'instagram',
            handle: '@ok',
            status: 'succeeded',
            summary: 'Instagram sync succeeded.',
            finishedAt: '2026-03-11T12:04:00Z',
          },
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
    expect(screen.getByText(/^@ok$/i)).toBeTruthy()
    expect(screen.getAllByRole('button', { name: /retry/i })).toHaveLength(1)
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

    fireEvent.click(await screen.findByRole('button', { name: /^maintenance/i }))
    fireEvent.change(await screen.findByLabelText('Scope'), { target: { value: 'provider' } })
    fireEvent.change(screen.getByLabelText('Provider'), { target: { value: 'tiktok' } })
    fireEvent.click(screen.getByRole('button', { name: /generate missing thumbnails/i }))

    await waitFor(() => {
      expect(bridgeMocks.enqueueMediaThumbnailGeneration).toHaveBeenCalledWith(['tk-1', 'tk-2'])
    })
  })

  it('keeps maintenance controls collapsed until requested', async () => {
    render(<SourceSyncQueueWindowPage />)

    const toggle = await screen.findByRole('button', { name: /^maintenance/i })
    expect(toggle.getAttribute('aria-expanded')).toBe('false')
    expect(screen.queryByRole('region', { name: /maintenance controls/i })).toBeNull()

    fireEvent.click(toggle)
    expect(toggle.getAttribute('aria-expanded')).toBe('true')
    expect(screen.getByRole('region', { name: /maintenance controls/i })).toBeTruthy()
  })

  it('shows real migration progress outside the collapsed maintenance panel', async () => {
    bridgeMocks.loadMediaPathMigrationQueueStatus.mockResolvedValue({
      queuedCount: 2,
      runningCount: 1,
      completedCount: 0,
      failedCount: 0,
      totalCount: 3,
      queuedItems: [],
      runningItems: [{
        jobId: 'migration-1', sourceId: 'tk-1', provider: 'tiktok', handle: '@moving',
        sourcePath: 'F:\\old', targetPath: 'S:\\new', state: 'running', queuedAt: '2026-07-12T12:00:00Z',
        progressPercent: 42, progressStage: 'moving', progressIndeterminate: false,
        filesProcessed: 42, filesTotal: 100, bytesProcessed: 4200, bytesTotal: 10000,
      }],
      recentResults: [],
      updatedAt: '2026-07-12T12:00:05Z',
    })

    render(<SourceSyncQueueWindowPage />)

    const progress = await screen.findByRole('progressbar', { name: /moving media migration/i })
    expect(progress.getAttribute('aria-valuenow')).toBe('42')
    expect(screen.getByText('Files').parentElement?.textContent).toContain('42 of 100')
    expect(screen.queryByRole('region', { name: /maintenance controls/i })).toBeNull()
  })

  it('cancels queued and active path migrations without affecting completed work', async () => {
    bridgeMocks.loadMediaPathMigrationQueueStatus.mockResolvedValue({
      queuedCount: 2, runningCount: 1, completedCount: 4, failedCount: 0, totalCount: 7,
      queuedItems: [],
      runningItems: [{
        jobId: 'migration-1', sourceId: 'tk-1', provider: 'tiktok', handle: '@moving',
        sourcePath: 'F:\\old', targetPath: 'S:\\new', state: 'running', queuedAt: '2026-07-12T12:00:00Z',
        progressPercent: 42, progressStage: 'moving', progressIndeterminate: false,
        filesProcessed: 42, filesTotal: 100, bytesProcessed: 4200, bytesTotal: 10000,
      }],
      recentResults: [], updatedAt: '',
    })
    render(<SourceSyncQueueWindowPage />)

    fireEvent.click(await screen.findByRole('button', { name: /cancel migrations/i }))

    await waitFor(() => expect(bridgeMocks.cancelMediaPathMigrations).toHaveBeenCalledTimes(1))
  })

  it('uses the refreshed profile avatar for completed migrations in Recent', async () => {
    bridgeMocks.loadWorkspaceSnapshot.mockResolvedValue({
      sources: [{ id: 'tw-1', provider: 'twitter', handle: '@moved', profileImagePath: 'S:\\moved\\ProfilePicture.jpg' }],
      schedulerGroups: [],
    })
    bridgeMocks.loadMediaPathMigrationQueueStatus.mockResolvedValue({
      queuedCount: 0, runningCount: 0, completedCount: 1, failedCount: 0, totalCount: 1,
      queuedItems: [], runningItems: [],
      recentResults: [{
        jobId: 'migration-done', sourceId: 'tw-1', provider: 'twitter', handle: '@moved',
        sourcePath: 'F:\\old', targetPath: 'S:\\moved', status: 'succeeded',
        summary: 'Moved 10 files.', finishedAt: '2026-07-12T12:05:00Z',
      }], updatedAt: '',
    })

    const { container } = render(<SourceSyncQueueWindowPage />)
    await screen.findByText('@moved')

    await waitFor(() => expect(container.querySelector('.queue-recent-item img')?.getAttribute('src')).toBe('S:\\moved\\ProfilePicture.jpg'))
  })
})
