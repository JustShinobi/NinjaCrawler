// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { LibraryDashboard, MediaTimelineItem, MediaTimelinePage } from '../../domain/models'
import { LibraryWindowPage } from './LibraryWindowPage'

const bridgeMocks = vi.hoisted(() => ({
  addTimelineItemsToCollection: vi.fn(),
  deleteCollection: vi.fn(),
  dismissVariantGroup: vi.fn(),
  listCollections: vi.fn(),
  loadCollectionTimeline: vi.fn(),
  loadLibraryDashboard: vi.fn(),
  loadMediaIndexStatus: vi.fn(),
  loadMediaThumbnails: vi.fn(),
  loadMediaTimeline: vi.fn(),
  loadVariantGroups: vi.fn(),
  cancelMediaIndexScan: vi.fn(),
  resumeMediaFingerprints: vi.fn(),
  markTimelineSeen: vi.fn(),
  openMediaFile: vi.fn(),
  openProfileViewWindow: vi.fn(),
  openWorkspaceHealthWindow: vi.fn(),
  promoteCollectionToGlobal: vi.fn(),
  revealMediaInFolder: vi.fn(),
  startMediaIndexScan: vi.fn(),
  subscribeToDesktopRuntimeEvents: vi.fn(),
  upsertCollection: vi.fn(),
}))

vi.mock('../../bridge/desktop', () => bridgeMocks)
vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (path: string) => path,
}))

function item(overrides: Partial<MediaTimelineItem> = {}): MediaTimelineItem {
  return {
    id: 'item-1',
    sourceId: 'src-1',
    provider: 'tiktok',
    handle: '@creator',
    mediaType: 'image',
    mediaSection: 'timeline',
    capturedAt: Math.floor(Date.parse('2026-05-19T12:00:00Z') / 1000),
    downloadedAt: Math.floor(Date.parse('2026-05-19T13:00:00Z') / 1000),
    absolutePath: 'S:/media/a.jpg',
    relativePath: 'a.jpg',
    fileCount: 1,
    sizeBytes: 1000,
    upstreamMissing: false,
    ...overrides,
  }
}

function page(overrides: Partial<MediaTimelinePage> = {}): MediaTimelinePage {
  return { items: [item()], nextCursor: undefined, newSinceLastVisit: 0, ...overrides }
}

function dashboard(overrides: Partial<LibraryDashboard> = {}): LibraryDashboard {
  return {
    totalFiles: 480,
    totalBytes: 45_000_000,
    totalSources: 12,
    upstreamMissing: 9,
    pendingFingerprints: 0,
    variantGroups: 4,
    variantReclaimableBytes: 2_000_000,
    providers: [],
    topProfiles: [],
    growth: [],
    stalledProfiles: [],
    ...overrides,
  }
}

/** A library that is indexed and browsable while the hashing stage runs. */
function hashingStatus(runOverrides: Record<string, unknown> = {}) {
  return {
    counts: { ...indexed(51_983).counts, pendingFingerprints: 6_000 },
    run: {
      id: 'run-1',
      status: 'running',
      stage: 'fingerprint',
      sourcesTotal: 1434,
      sourcesProcessed: 1434,
      filesIndexed: 51_983,
      filesUpdated: 0,
      filesMissing: 0,
      hashesInherited: 0,
      fingerprintsTotal: 12_000,
      fingerprintsDone: 6_000,
      resourceProfile: 'balanced',
      startedAt: '2026-07-31T00:00:00Z',
      ...runOverrides,
    },
  }
}

/** An indexed library, so the grid renders instead of the first-run screen. */
function indexed(totalFiles = 480) {
  return {
    counts: {
      totalFiles,
      totalBytes: 45_000_000,
      pendingFingerprints: 0,
      failedFingerprints: 0,
      missingOnDisk: 0,
      upstreamMissing: 9,
      indexedSources: 12,
    },
  }
}

describe('LibraryWindowPage', () => {
  beforeEach(() => {
    for (const mock of Object.values(bridgeMocks)) mock.mockReset()
    bridgeMocks.loadMediaTimeline.mockResolvedValue(page())
    bridgeMocks.loadCollectionTimeline.mockResolvedValue(page())
    bridgeMocks.loadMediaThumbnails.mockResolvedValue({ available: false, thumbs: {} })
    bridgeMocks.markTimelineSeen.mockResolvedValue('2026-07-31T00:00:00Z')
    bridgeMocks.listCollections.mockResolvedValue([])
    bridgeMocks.loadVariantGroups.mockResolvedValue([])
    bridgeMocks.loadLibraryDashboard.mockResolvedValue(dashboard())
    bridgeMocks.loadMediaIndexStatus.mockResolvedValue(indexed())
    bridgeMocks.addTimelineItemsToCollection.mockResolvedValue(1)
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockResolvedValue(() => undefined)
    bridgeMocks.cancelMediaIndexScan.mockResolvedValue(indexed())
    bridgeMocks.resumeMediaFingerprints.mockResolvedValue(indexed())
    localStorage.clear()
  })

  afterEach(() => cleanup())

  it('opens on what arrived since the last visit', async () => {
    bridgeMocks.loadMediaTimeline.mockResolvedValue(page({ newSinceLastVisit: 37 }))
    render(<LibraryWindowPage />)

    await waitFor(() =>
      expect(bridgeMocks.loadMediaTimeline).toHaveBeenCalledWith(
        expect.objectContaining({ unseenOnly: true }),
      ),
    )
    const nav = screen.getByRole('navigation', { name: /library sections/i })
    const first = within(nav).getAllByRole('button')[0]
    expect(first.textContent).toContain('New')
    expect(first.getAttribute('aria-current')).toBe('true')
  })

  /** The state the operator actually hit: nothing indexed, four zeroes, no way out. */
  it('explains an unindexed library and offers to index it', async () => {
    bridgeMocks.loadMediaIndexStatus.mockResolvedValue(indexed(0))
    bridgeMocks.startMediaIndexScan.mockResolvedValue(indexed(0))
    render(<LibraryWindowPage />)

    expect(await screen.findByText(/has not been indexed yet/i)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: /index library/i }))
    await waitFor(() => expect(bridgeMocks.startMediaIndexScan).toHaveBeenCalledTimes(1))
  })

  it('reports indexing progress instead of an empty grid while it runs', async () => {
    bridgeMocks.loadMediaIndexStatus.mockResolvedValue({
      ...indexed(0),
      run: {
        id: 'run-1',
        status: 'running',
        stage: 'reconcile',
        sourcesTotal: 40,
        sourcesProcessed: 10,
        filesIndexed: 0,
        filesUpdated: 0,
        filesMissing: 0,
        hashesInherited: 0,
        currentSourceHandle: '@creator',
        startedAt: '2026-07-31T00:00:00Z',
      },
    })
    render(<LibraryWindowPage />)

    expect(await screen.findByText(/indexing your library/i)).toBeTruthy()
    expect(screen.getByText(/10 of 40 profiles \(25%\)/i)).toBeTruthy()
  })

  /**
   * The reported "stuck at 100%": profiles were done and hashing had started,
   * but the blocking panel reported only the profile count.
   */
  it('keeps the grid usable while hashing continues in the background', async () => {
    bridgeMocks.loadMediaIndexStatus.mockResolvedValue(hashingStatus())
    render(<LibraryWindowPage />)

    // Media renders; the run is reported in a strip rather than a blocking panel.
    await waitFor(() =>
      expect(screen.getAllByRole('button', { name: /open preview/i }).length).toBe(1),
    )
    expect(screen.queryByText(/indexing your library/i)).toBeNull()
    expect(screen.getByText(/hashing for duplicate detection/i)).toBeTruthy()
  })

  /** Reported gap: a percentage with no sense of how long it will take. */
  it('reports rate and a finish estimate once hashing has a sample', async () => {
    bridgeMocks.loadMediaIndexStatus.mockResolvedValue(
      hashingStatus({
        fingerprintsDone: 6_000,
        fingerprintsTotal: 12_000,
        // 6,000 files in 60 s = 100/s, so 6,000 left is about a minute.
        fingerprintStartedAt: new Date(Date.now() - 60_000).toISOString(),
      }),
    )
    render(<LibraryWindowPage />)

    const strip = await screen.findByText(/hashing for duplicate detection/i)
    expect(strip.textContent).toMatch(/50%/)
    expect(strip.textContent).toMatch(/\/s/)
    expect(strip.textContent).toMatch(/left/)
  })

  /**
   * Reported gap: hashing outlives a session, and after a restart there was no
   * progress shown, no way to resume, and nowhere to change the speed.
   */
  it('offers to resume hashing left over from a previous session', async () => {
    bridgeMocks.loadMediaIndexStatus.mockResolvedValue({
      counts: { ...indexed(51_983).counts, pendingFingerprints: 604_513 },
    })
    render(<LibraryWindowPage />)

    expect(await screen.findByText(/duplicate detection is paused/i)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: /^resume$/i }))

    // Resuming skips the profile walk and goes straight back to hashing.
    await waitFor(() =>
      expect(bridgeMocks.resumeMediaFingerprints).toHaveBeenCalledWith('balanced'),
    )
    expect(bridgeMocks.startMediaIndexScan).not.toHaveBeenCalled()
  })

  it('lets the operator pick how much of the machine indexing may use', async () => {
    bridgeMocks.loadMediaIndexStatus.mockResolvedValue(hashingStatus())
    bridgeMocks.startMediaIndexScan.mockResolvedValue(hashingStatus())
    render(<LibraryWindowPage />)

    await screen.findByText(/hashing for duplicate detection/i)
    const fast = screen.getByRole('button', { name: /^fast$/i })
    fireEvent.click(fast)
    await waitFor(() => expect(fast.getAttribute('aria-pressed')).toBe('true'))

    // Changing speed mid-hashing restarts only the pending backlog at the new
    // pace — the profile walk is not repeated.
    await waitFor(() => expect(bridgeMocks.cancelMediaIndexScan).toHaveBeenCalled())
    await waitFor(() => expect(bridgeMocks.resumeMediaFingerprints).toHaveBeenCalledWith('fast'))
    expect(bridgeMocks.startMediaIndexScan).not.toHaveBeenCalled()
  })

  it('carries the sidebar counters from the library summary', async () => {
    render(<LibraryWindowPage />)

    const archived = await screen.findByRole('button', { name: /only here/i })
    expect(archived.textContent).toContain('9')
    const duplicates = screen.getByRole('button', { name: /duplicates/i })
    expect(duplicates.textContent).toContain('4')
  })

  it('filters to media removed from the source', async () => {
    render(<LibraryWindowPage />)
    await waitFor(() => expect(bridgeMocks.loadMediaTimeline).toHaveBeenCalled())

    fireEvent.click(screen.getByRole('button', { name: /only here/i }))

    await waitFor(() =>
      expect(bridgeMocks.loadMediaTimeline).toHaveBeenLastCalledWith(
        expect.objectContaining({ upstreamMissingOnly: true, unseenOnly: false }),
      ),
    )
  })

  it('lists collections as destinations and opens one', async () => {
    bridgeMocks.listCollections.mockResolvedValue([
      {
        id: 'col-1',
        kind: 'manual',
        scope: 'global',
        name: 'Favourites',
        pinned: false,
        itemCount: 3,
        createdAt: '2026-07-01T00:00:00Z',
        updatedAt: '2026-07-01T00:00:00Z',
      },
    ])
    render(<LibraryWindowPage />)

    fireEvent.click(await screen.findByRole('button', { name: /favourites/i }))
    await waitFor(() =>
      expect(bridgeMocks.loadCollectionTimeline).toHaveBeenCalledWith('col-1'),
    )
  })

  it('saves the filters in use as a smart collection', async () => {
    bridgeMocks.upsertCollection.mockResolvedValue(undefined)
    vi.spyOn(window, 'prompt').mockReturnValue('Only videos')
    render(<LibraryWindowPage />)

    await waitFor(() => expect(bridgeMocks.loadMediaTimeline).toHaveBeenCalled())
    fireEvent.change(await screen.findByLabelText('Media'), { target: { value: 'video' } })
    fireEvent.click(screen.getByRole('button', { name: /save filter as collection/i }))

    await waitFor(() =>
      expect(bridgeMocks.upsertCollection).toHaveBeenCalledWith(
        expect.objectContaining({
          kind: 'smart',
          ruleJson: expect.stringContaining('"mediaType":"video"'),
        }),
      ),
    )
  })

  it('adds the selected posts to an existing collection', async () => {
    bridgeMocks.listCollections.mockResolvedValue([
      {
        id: 'col-1',
        kind: 'manual',
        scope: 'global',
        name: 'Favourites',
        pinned: false,
        itemCount: 0,
        createdAt: '2026-07-01T00:00:00Z',
        updatedAt: '2026-07-01T00:00:00Z',
      },
    ])
    render(<LibraryWindowPage />)

    fireEvent.click(await screen.findByRole('button', { name: /^select media$/i }))
    fireEvent.click(await screen.findByRole('button', { name: /add to favourites/i }))

    await waitFor(() =>
      expect(bridgeMocks.addTimelineItemsToCollection).toHaveBeenCalledWith('col-1', ['item-1']),
    )
  })

  it('reviews duplicate groups and can reject one', async () => {
    bridgeMocks.loadVariantGroups.mockResolvedValue([
      {
        id: 'group-1',
        scope: 'intra_source',
        matchKind: 'perceptual_video',
        confidence: 0.85,
        policyApplied: 'link_only',
        reviewed: false,
        createdAt: '2026-07-30T00:00:00Z',
        members: [
          {
            mediaId: 'm1',
            role: 'canonical',
            sourceId: 'src-1',
            provider: 'instagram',
            handle: '@creator',
            mediaSection: 'timeline',
            relativePath: 'feed.mp4',
            sizeBytes: 9000,
          },
          {
            mediaId: 'm2',
            role: 'variant',
            sourceId: 'src-1',
            provider: 'instagram',
            handle: '@creator',
            mediaSection: 'stories',
            relativePath: 'story.mp4',
            sizeBytes: 5000,
          },
        ],
      },
    ])
    bridgeMocks.dismissVariantGroup.mockResolvedValue(undefined)
    render(<LibraryWindowPage />)

    fireEvent.click(await screen.findByRole('button', { name: /duplicates/i }))
    expect(await screen.findByText(/reposted inside one profile/i)).toBeTruthy()
    // The non-destructive default has to be stated, not implied.
    expect(screen.getByText(/nothing was deleted/i)).toBeTruthy()

    fireEvent.click(screen.getByRole('button', { name: /not duplicates/i }))
    await waitFor(() => expect(bridgeMocks.dismissVariantGroup).toHaveBeenCalledWith('group-1'))
  })

  /** Sync problems belong to Workspace Health; the summary only points at them. */
  it('links failing profiles to workspace health instead of listing them', async () => {
    bridgeMocks.loadLibraryDashboard.mockResolvedValue(
      dashboard({
        stalledProfiles: [
          {
            sourceId: 'src-2',
            provider: 'instagram',
            handle: '@broken',
            reason: 'sync_failing',
            syncProblemCode: 'auth_required',
          },
        ],
      }),
    )
    render(<LibraryWindowPage />)

    fireEvent.click(await screen.findByRole('button', { name: /library summary/i }))
    const link = await screen.findByRole('button', { name: /failing to sync/i })
    expect(screen.queryByText('@broken')).toBeNull()

    fireEvent.click(link)
    await waitFor(() => expect(bridgeMocks.openWorkspaceHealthWindow).toHaveBeenCalledTimes(1))
  })

  it('surfaces a backend failure instead of rendering an empty library', async () => {
    bridgeMocks.loadMediaTimeline.mockRejectedValue(new Error('no such table: media_index'))
    render(<LibraryWindowPage />)

    expect(await screen.findByRole('alert')).toBeTruthy()
    expect(screen.getByText(/no such table: media_index/i)).toBeTruthy()
  })
})
