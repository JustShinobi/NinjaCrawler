import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  addTimelineItemsToCollection,
  deleteCollection,
  dismissVariantGroup,
  listCollections,
  loadCollectionTimeline,
  loadLibraryDashboard,
  loadMediaIndexStatus,
  loadMediaThumbnails,
  loadMediaTimeline,
  loadVariantGroups,
  cancelMediaIndexScan,
  resumeMediaFingerprints,
  markTimelineSeen,
  openMediaFile,
  openProfileViewWindow,
  openWorkspaceHealthWindow,
  promoteCollectionToGlobal,
  revealMediaInFolder,
  startMediaIndexScan,
  subscribeToDesktopRuntimeEvents,
  upsertCollection,
} from '../../bridge/desktop'
import type {
  Collection,
  LibraryDashboard,
  MediaIndexStatus,
  MediaTimelineCursor,
  MediaTimelineFilter,
  MediaTimelineItem,
  MediaVariantGroup,
} from '../../domain/models'
import { WindowShell } from '../brand/WindowShell'
import { WindowTitlebar } from '../brand/WindowTitlebar'
import { MediaCard } from '../workspace/MediaCard'

const PROVIDER_LABELS: Record<string, string> = {
  instagram: 'Instagram',
  tiktok: 'TikTok',
  twitter: 'X / Twitter',
  youtube: 'YouTube',
  vsco: 'VSCO',
}

/**
 * Where the operator can be in the library. Media destinations all render the
 * same grid with a different base filter; the rest are their own panels.
 * A collection destination carries its id after the colon.
 */
type Destination = 'new' | 'all' | 'archived' | 'duplicates' | 'summary' | string

type MediaTypeFilter = 'all' | 'image' | 'video'

const MEDIA_DESTINATIONS = new Set(['new', 'all', 'archived'])

function isCollectionDestination(destination: Destination): boolean {
  return destination.startsWith('collection:')
}

function providerLabel(provider: string): string {
  return PROVIDER_LABELS[provider] ?? provider
}

function formatBytes(value: number): string {
  if (value <= 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const exponent = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1)
  const scaled = value / 1024 ** exponent
  return `${scaled >= 10 || exponent === 0 ? Math.round(scaled) : scaled.toFixed(1)} ${units[exponent]}`
}

const INDEX_SPEED_STORAGE_KEY = 'library.indexSpeed'

/**
 * How much of the machine indexing may use. The wording says what it costs, not
 * how many threads it spawns — the operator decides based on what else they are
 * doing, not on core counts.
 */
const INDEX_SPEED_OPTIONS = [
  { value: 'quiet', label: 'Quiet', hint: 'One file at a time — stays out of the way' },
  { value: 'balanced', label: 'Balanced', hint: 'Half the processor cores' },
  { value: 'fast', label: 'Fast', hint: 'Nearly every core — the machine will feel busy' },
] as const

function readStoredIndexSpeed(): 'quiet' | 'balanced' | 'fast' {
  try {
    const stored = localStorage.getItem(INDEX_SPEED_STORAGE_KEY)
    if (stored === 'quiet' || stored === 'balanced' || stored === 'fast') return stored
  } catch {
    /* ignore */
  }
  return 'balanced'
}

/** Coarse on purpose: "about 3 h left" ages better than a false minute count. */
function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return 'a moment'
  if (seconds < 90) return `${Math.round(seconds)} s`
  const minutes = seconds / 60
  if (minutes < 90) return `${Math.round(minutes)} min`
  const hours = minutes / 60
  if (hours < 36) return `${hours < 10 ? hours.toFixed(1) : Math.round(hours)} h`
  return `${Math.round(hours / 24)} days`
}

function dayKey(item: MediaTimelineItem): string {
  if (!item.capturedAt) return 'undated'
  const date = new Date(item.capturedAt * 1000)
  return `${date.getFullYear()}-${date.getMonth() + 1}-${date.getDate()}`
}

function dayLabel(item: MediaTimelineItem): string {
  if (!item.capturedAt) return 'No date'
  const date = new Date(item.capturedAt * 1000)
  const sameYear = date.getFullYear() === new Date().getFullYear()
  return date.toLocaleDateString(undefined, {
    weekday: 'short',
    day: 'numeric',
    month: 'short',
    ...(sameYear ? {} : { year: 'numeric' }),
  })
}

export function LibraryWindowPage() {
  // Opens on what arrived since the last visit: the library is for looking at
  // media, and the numbers are a destination of their own further down.
  const [destination, setDestination] = useState<Destination>('new')
  const [items, setItems] = useState<MediaTimelineItem[]>([])
  const [cursor, setCursor] = useState<MediaTimelineCursor | undefined>()
  const [loading, setLoading] = useState(true)
  const [loadingMore, setLoadingMore] = useState(false)
  const [error, setError] = useState<string>()
  const [newCount, setNewCount] = useState(0)
  const [provider, setProvider] = useState('all')
  const [mediaType, setMediaType] = useState<MediaTypeFilter>('all')
  const [thumbs, setThumbs] = useState<Record<string, string>>({})
  const [collections, setCollections] = useState<Collection[]>([])
  const [variantGroups, setVariantGroups] = useState<MediaVariantGroup[]>([])
  const [dashboard, setDashboard] = useState<LibraryDashboard>()
  const [indexStatus, setIndexStatus] = useState<MediaIndexStatus>()
  const [selection, setSelection] = useState<Set<string>>(() => new Set())
  const [resourceProfile, setResourceProfile] = useState<'quiet' | 'balanced' | 'fast'>(
    () => readStoredIndexSpeed(),
  )
  const sentinelRef = useRef<HTMLDivElement>(null)

  const activeCollection = useMemo(
    () =>
      isCollectionDestination(destination)
        ? collections.find((entry) => `collection:${entry.id}` === destination)
        : undefined,
    [collections, destination],
  )

  const filter = useMemo<MediaTimelineFilter>(
    () => ({
      providers: provider === 'all' ? [] : [provider],
      mediaType: mediaType === 'all' ? undefined : mediaType,
      upstreamMissingOnly: destination === 'archived',
      unseenOnly: destination === 'new',
    }),
    [destination, mediaType, provider],
  )

  const showsMedia = MEDIA_DESTINATIONS.has(destination) || isCollectionDestination(destination)

  const load = useCallback(async () => {
    if (!showsMedia) return
    setLoading(true)
    setError(undefined)
    try {
      const page = activeCollection
        ? await loadCollectionTimeline(activeCollection.id)
        : await loadMediaTimeline(filter)
      setItems(page.items)
      setCursor(page.nextCursor)
      setNewCount(page.newSinceLastVisit)
    } catch (loadError) {
      setError(loadError instanceof Error ? loadError.message : 'Failed to load the library.')
      setItems([])
      setCursor(undefined)
    } finally {
      setLoading(false)
    }
  }, [activeCollection, filter, showsMedia])

  useEffect(() => {
    void load()
  }, [load])

  const refreshCollections = useCallback(async () => {
    try {
      setCollections(await listCollections())
    } catch {
      /* the rest of the window stays usable */
    }
  }, [])

  const refreshDashboard = useCallback(async () => {
    try {
      setDashboard(await loadLibraryDashboard())
    } catch {
      /* counters degrade to blank rather than breaking navigation */
    }
  }, [])

  const refreshVariants = useCallback(async () => {
    try {
      setVariantGroups(await loadVariantGroups(100))
    } catch {
      /* ignore */
    }
  }, [])

  // The dashboard doubles as the source of the sidebar counters, so it is
  // fetched once on open rather than only when its own destination is visited.
  useEffect(() => {
    void refreshCollections()
    void refreshDashboard()
    void loadMediaIndexStatus().then(setIndexStatus).catch(() => undefined)
  }, [refreshCollections, refreshDashboard])

  useEffect(() => {
    if (destination === 'duplicates') void refreshVariants()
    if (destination === 'summary') void refreshDashboard()
  }, [destination, refreshDashboard, refreshVariants])

  useEffect(() => {
    let unsubscribe: (() => void) | undefined
    void subscribeToDesktopRuntimeEvents({
      onMediaIndexStatusChanged: (status) => {
        setIndexStatus((current) => {
          // Reconciliation finishing is what makes media browsable; waiting for
          // the whole run would keep an empty grid on screen while hashing.
          const reachedFingerprint =
            current?.run?.stage !== 'fingerprint' && status.run?.stage === 'fingerprint'
          if (reachedFingerprint || status.run?.status === 'completed') {
            void refreshDashboard()
            void load()
          }
          return status
        })
      },
    })
      .then((value) => {
        unsubscribe = value
      })
      .catch(() => undefined)
    return () => unsubscribe?.()
  }, [load, refreshDashboard])

  const loadMore = useCallback(async () => {
    if (!cursor || loadingMore) return
    setLoadingMore(true)
    try {
      const page = activeCollection
        ? await loadCollectionTimeline(activeCollection.id, cursor)
        : await loadMediaTimeline(filter, cursor)
      setItems((current) => [...current, ...page.items])
      setCursor(page.nextCursor)
    } catch {
      /* transient: the sentinel retries on the next scroll */
    } finally {
      setLoadingMore(false)
    }
  }, [activeCollection, cursor, filter, loadingMore])

  useEffect(() => {
    const sentinel = sentinelRef.current
    if (!sentinel || !cursor) return undefined
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) void loadMore()
    })
    observer.observe(sentinel)
    return () => observer.disconnect()
  }, [cursor, loadMore])

  useEffect(() => {
    const videos = items
      .filter((item) => item.mediaType === 'video')
      .map((item) => item.absolutePath)
      .filter((path) => path.length > 0 && !(path in thumbs))
    if (videos.length === 0) return
    let active = true
    void loadMediaThumbnails(videos)
      .then((batch) => {
        if (active && batch.available) setThumbs((current) => ({ ...current, ...batch.thumbs }))
      })
      .catch(() => undefined)
    return () => {
      active = false
    }
  }, [items, thumbs])

  const markSeen = useCallback(async () => {
    try {
      await markTimelineSeen()
      setNewCount(0)
      if (destination === 'new') void load()
    } catch {
      /* ignore */
    }
  }, [destination, load])

  const saveFilterAsCollection = useCallback(async () => {
    const name = window.prompt('Name this smart collection:')?.trim()
    if (!name) return
    try {
      await upsertCollection({
        name,
        kind: 'smart',
        scope: 'global',
        ruleJson: JSON.stringify(filter),
      })
      await refreshCollections()
    } catch (saveError) {
      setError(saveError instanceof Error ? saveError.message : 'Failed to save the collection.')
    }
  }, [filter, refreshCollections])

  const addSelectionToCollection = useCallback(
    async (collectionId: string) => {
      const ids = [...selection]
      if (ids.length === 0) return
      try {
        await addTimelineItemsToCollection(collectionId, ids)
        setSelection(new Set())
        await refreshCollections()
      } catch (addError) {
        setError(addError instanceof Error ? addError.message : 'Failed to add to the collection.')
      }
    },
    [refreshCollections, selection],
  )

  const createCollectionFromSelection = useCallback(async () => {
    const name = window.prompt('Name the new collection:')?.trim()
    if (!name) return
    try {
      const created = await upsertCollection({ name, kind: 'manual', scope: 'global' })
      if (created) await addSelectionToCollection(created.id)
    } catch (createError) {
      setError(
        createError instanceof Error ? createError.message : 'Failed to create the collection.',
      )
    }
  }, [addSelectionToCollection])

  const toggleSelected = useCallback((id: string) => {
    setSelection((current) => {
      const next = new Set(current)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }, [])

  const startIndexing = useCallback(async () => {
    try {
      setIndexStatus(await startMediaIndexScan(undefined, resourceProfile))
    } catch (indexError) {
      setError(indexError instanceof Error ? indexError.message : 'Failed to start indexing.')
    }
  }, [resourceProfile])

  /**
   * Changing speed mid-run restarts the backlog with a different worker count.
   * Work already hashed is kept — only what is still pending is redone at the
   * new pace.
   */
  const resumeFingerprints = useCallback(async () => {
    try {
      setIndexStatus(await resumeMediaFingerprints(resourceProfile))
    } catch (resumeError) {
      setError(resumeError instanceof Error ? resumeError.message : 'Failed to resume hashing.')
    }
  }, [resourceProfile])

  const changeIndexingSpeed = useCallback(
    async (next: 'quiet' | 'balanced' | 'fast') => {
      setResourceProfile(next)
      try {
        localStorage.setItem(INDEX_SPEED_STORAGE_KEY, next)
      } catch {
        /* ignore */
      }
      if (indexStatus?.run?.status === 'running') {
        try {
          await cancelMediaIndexScan()
          // Only the pending backlog is redone at the new pace; the profile
          // walk does not need repeating.
          setIndexStatus(
            indexStatus.run.stage === 'fingerprint'
              ? await resumeMediaFingerprints(next)
              : await startMediaIndexScan(undefined, next),
          )
        } catch {
          /* ignore */
        }
      }
    },
    [indexStatus?.run?.stage, indexStatus?.run?.status],
  )

  const days = useMemo(() => {
    const grouped = new Map<string, { label: string; items: MediaTimelineItem[] }>()
    for (const item of items) {
      const key = dayKey(item)
      const bucket = grouped.get(key)
      if (bucket) bucket.items.push(item)
      else grouped.set(key, { label: dayLabel(item), items: [item] })
    }
    return [...grouped.entries()].map(([key, value]) => ({ key, ...value }))
  }, [items])

  const indexing = indexStatus?.run?.status === 'running' || indexStatus?.run?.status === 'queued'
  // Nothing indexed yet is a first-run state, not an empty result: the operator
  // needs the reason and the fix, not an empty grid.
  const libraryIsUnindexed = (indexStatus?.counts.totalFiles ?? 0) === 0
  // Work left over from a previous session: duplicate detection cannot run
  // until these are hashed, so it stays visible and resumable.
  const hashingPending = (indexStatus?.counts.pendingFingerprints ?? 0) > 0

  /**
   * Walking the profile folders is only the first stage; hashing every file for
   * duplicate detection is the long one. Reporting a single "profiles" number
   * made the run look stuck at 100% while it was still working.
   */
  const indexingDetail = (() => {
    const run = indexStatus?.run
    if (!run) return 'Starting…'
    if (run.stage === 'fingerprint') {
      const total = run.fingerprintsTotal
      const done = run.fingerprintsDone
      const remaining = Math.max(0, total - done)
      if (remaining === 0) return 'Finishing duplicate detection · your media is already browsable'
      const percent = total > 0 ? Math.round((done / total) * 100) : 0
      const elapsedMs = run.fingerprintStartedAt
        ? Date.now() - Date.parse(run.fingerprintStartedAt)
        : 0
      // A rate needs a sample to be honest: below ~20 files the estimate swings
      // wildly, so it is simply not shown yet.
      const rate = done > 20 && elapsedMs > 0 ? done / (elapsedMs / 1000) : 0
      const parts = [
        `Hashing for duplicate detection · ${done.toLocaleString()} of ${total.toLocaleString()} (${percent}%)`,
      ]
      if (rate > 0) {
        parts.push(`${rate < 10 ? rate.toFixed(1) : Math.round(rate)}/s`)
        parts.push(`about ${formatDuration(remaining / rate)} left`)
      }
      return parts.join(' · ')
    }
    const percent =
      run.sourcesTotal > 0 ? Math.round((run.sourcesProcessed / run.sourcesTotal) * 100) : 0
    return `${run.currentSourceHandle ? `Reading ${run.currentSourceHandle} · ` : ''}${run.sourcesProcessed} of ${run.sourcesTotal} profiles (${percent}%)`
  })()
  const failingSyncs =
    dashboard?.stalledProfiles.filter((profile) => profile.reason === 'sync_failing').length ?? 0

  const destinations: Array<{ key: Destination; icon: string; label: string; badge?: number }> = [
    { key: 'new', icon: '✦', label: 'New', badge: newCount },
    { key: 'all', icon: '▦', label: 'Everything' },
    { key: 'archived', icon: '⛊', label: 'Only here', badge: dashboard?.upstreamMissing },
    { key: 'duplicates', icon: '❐', label: 'Duplicates', badge: dashboard?.variantGroups },
    { key: 'summary', icon: '▤', label: 'Library summary' },
  ]

  function renderMedia() {
    if (libraryIsUnindexed && !indexing) {
      return (
        <div className="library-firstrun">
          <p className="library-firstrun-title">Your library has not been indexed yet</p>
          <p className="library-firstrun-body">
            Anything you download from now on is indexed automatically. To bring in what is
            already on disk — including libraries imported from other tools — run the indexer
            once.
          </p>
          <button className="primary-button" onClick={() => void startIndexing()} type="button">
            Index library
          </button>
          <div className="library-speed-picker">
            <span className="library-speed-label">Speed</span>
            {INDEX_SPEED_OPTIONS.map((option) => (
              <button
                aria-pressed={resourceProfile === option.value}
                className={
                  resourceProfile === option.value
                    ? 'library-speed-option is-active'
                    : 'library-speed-option'
                }
                key={option.value}
                onClick={() => void changeIndexingSpeed(option.value)}
                title={option.hint}
                type="button"
              >
                {option.label}
              </button>
            ))}
          </div>
          <p className="library-speed-hint">
            {INDEX_SPEED_OPTIONS.find((option) => option.value === resourceProfile)?.hint}
          </p>
        </div>
      )
    }
    // The full-screen panel is only for a library with nothing to show yet.
    // Once media is indexed, hashing continues in the background and the grid
    // takes over — blocking it made a working run look frozen at 100%.
    if (indexing && libraryIsUnindexed) {
      return (
        <div className="library-firstrun">
          <p className="library-firstrun-title">Indexing your library…</p>
          <p className="library-firstrun-body">{indexingDetail}</p>
        </div>
      )
    }
    if (loading) return <div className="library-loading">Loading…</div>
    if (items.length === 0) {
      return (
        <div className="library-empty">
          <p>
            {destination === 'new'
              ? 'Nothing new since your last visit.'
              : destination === 'archived'
                ? 'Nothing here has been removed from its source yet.'
                : 'Nothing matches these filters.'}
          </p>
        </div>
      )
    }
    return (
      <div className="library-scroll">
        {days.map((day) => (
          <section className="library-day" key={day.key}>
            <h2 className="library-day-heading">
              {day.label}
              <span className="library-day-count">{day.items.length}</span>
            </h2>
            <div className="library-grid">
              {day.items.map((item) => (
                <MediaCard
                  key={item.id}
                  posterAbsPath={
                    item.mediaType === 'video' ? thumbs[item.absolutePath] : item.absolutePath
                  }
                  videoThumbAbsPath={item.mediaType === 'video' ? item.absolutePath : undefined}
                  eagerPoster
                  isVideo={item.mediaType === 'video'}
                  archivedOnly={item.upstreamMissing}
                  slideshowCount={item.fileCount > 1 ? item.fileCount : undefined}
                  badge={`${providerLabel(item.provider)} · ${item.handle.replace(/^@/, '')}`}
                  overlayText={
                    item.capturedAt
                      ? new Date(item.capturedAt * 1000).toLocaleTimeString(undefined, {
                        hour: '2-digit',
                        minute: '2-digit',
                      })
                      : ''
                  }
                  selected={selection.has(item.id)}
                  selectMode={selection.size > 0}
                  onToggleSelect={() => toggleSelected(item.id)}
                  onOpen={() => void openMediaFile(item.absolutePath)}
                  hideOnline
                  onReveal={() => void revealMediaInFolder(item.absolutePath)}
                  onContextMenu={() => void openProfileViewWindow(item.sourceId)}
                />
              ))}
            </div>
          </section>
        ))}
        <div ref={sentinelRef} className="library-sentinel">
          {loadingMore ? 'Loading more…' : cursor ? '' : 'End of the library.'}
        </div>
      </div>
    )
  }

  function renderDuplicates() {
    if (variantGroups.length === 0) {
      return (
        <div className="library-empty">
          <p>No duplicate groups found.</p>
          <p className="library-empty-hint">
            These are found while the library is indexed — a story reposted to the feed, or the
            same upload on two providers of the same person.
          </p>
        </div>
      )
    }
    return (
      <div className="library-scroll">
        <p className="library-empty-hint">
          Nothing was deleted. Extra copies are collapsed behind the best one in the grid.
        </p>
        <ul className="library-collection-list">
          {variantGroups.map((group) => (
            <li className="library-collection" key={group.id}>
              <div className="library-collection-open">
                <strong>
                  {group.scope === 'cross_source'
                    ? 'Same upload on two providers'
                    : 'Reposted inside one profile'}
                </strong>
                <span className="library-collection-meta">
                  {group.members
                    .map(
                      (member) =>
                        `${providerLabel(member.provider)} ${member.mediaSection || 'timeline'}${member.role === 'canonical' ? ' (kept)' : ''}`,
                    )
                    .join(' · ')}
                </span>
              </div>
              <span className="library-collection-actions">
                <button
                  className="ghost-button"
                  onClick={() => void dismissVariantGroup(group.id).then(refreshVariants)}
                  title="These are different posts — stop grouping them"
                  type="button"
                >
                  Not duplicates
                </button>
              </span>
            </li>
          ))}
        </ul>
      </div>
    )
  }

  function renderSummary() {
    return (
      <div className="library-scroll">
        <div className="library-stat-grid">
          <article className="library-stat">
            <span>Archived media</span>
            <strong>{(dashboard?.totalFiles ?? 0).toLocaleString()}</strong>
            <small>
              {formatBytes(dashboard?.totalBytes ?? 0)} across {dashboard?.totalSources ?? 0}{' '}
              profiles
            </small>
          </article>
          <article className="library-stat">
            <span>Only here</span>
            <strong>{(dashboard?.upstreamMissing ?? 0).toLocaleString()}</strong>
            <small>Removed from the source — this is the last copy</small>
          </article>
          <article className="library-stat">
            <span>Duplicate groups</span>
            <strong>{(dashboard?.variantGroups ?? 0).toLocaleString()}</strong>
            <small>{formatBytes(dashboard?.variantReclaimableBytes ?? 0)} in extra copies</small>
          </article>
        </div>

        {dashboard && dashboard.topProfiles.length > 0 ? (
          <section className="library-overview-block">
            <h2>Largest profiles</h2>
            <ul className="library-bar-list">
              {dashboard.topProfiles.slice(0, 10).map((profile) => (
                <li key={profile.sourceId}>
                  <span>{profile.handle.replace(/^@/, '')}</span>
                  <span
                    aria-hidden="true"
                    className="library-bar"
                    style={{
                      width: `${Math.max(
                        2,
                        (profile.bytes / Math.max(1, dashboard.topProfiles[0].bytes)) * 100,
                      )}%`,
                    }}
                  />
                  <span className="library-bar-value">{formatBytes(profile.bytes)}</span>
                </li>
              ))}
            </ul>
          </section>
        ) : null}

        {failingSyncs > 0 ? (
          <button
            className="library-health-link"
            onClick={() => void openWorkspaceHealthWindow()}
            type="button"
          >
            {failingSyncs} profile{failingSyncs === 1 ? '' : 's'} failing to sync — open Workspace
            Health
          </button>
        ) : null}
      </div>
    )
  }

  return (
    <WindowShell
      className="library-window-frame"
      contentClassName="library-window-content"
      density="compact"
      titlebar={
        <WindowTitlebar
          title="Library"
          trailing={
            <span className="window-titlebar-status-meta">
              {libraryIsUnindexed
                ? 'not indexed'
                : `${(dashboard?.totalFiles ?? 0).toLocaleString()} items`}
            </span>
          }
        />
      }
    >
      <div className="library-window-body">
        <nav aria-label="Library sections" className="library-sidebar">
          {destinations.map((entry) => (
            <button
              aria-current={destination === entry.key}
              className={
                destination === entry.key ? 'library-nav-item is-active' : 'library-nav-item'
              }
              key={entry.key}
              onClick={() => setDestination(entry.key)}
              type="button"
            >
              <span aria-hidden="true" className="library-nav-icon">
                {entry.icon}
              </span>
              <span className="library-nav-label">{entry.label}</span>
              {entry.badge ? <span className="library-nav-badge">{entry.badge}</span> : null}
            </button>
          ))}

          <div className="library-sidebar-section">
            <span className="library-sidebar-heading">Collections</span>
            {collections.length === 0 ? (
              <p className="library-sidebar-hint">
                Select posts to start one, or save the filters you are using.
              </p>
            ) : (
              collections.map((collection) => (
                <button
                  aria-current={destination === `collection:${collection.id}`}
                  className={
                    destination === `collection:${collection.id}`
                      ? 'library-nav-item is-active'
                      : 'library-nav-item'
                  }
                  key={collection.id}
                  onClick={() => setDestination(`collection:${collection.id}`)}
                  type="button"
                >
                  <span aria-hidden="true" className="library-nav-icon">
                    {collection.kind === 'smart' ? '⚡' : '❏'}
                  </span>
                  <span className="library-nav-label">{collection.name}</span>
                  {collection.kind === 'manual' && collection.itemCount > 0 ? (
                    <span className="library-nav-badge">{collection.itemCount}</span>
                  ) : null}
                </button>
              ))
            )}
          </div>
        </nav>

        <main className="library-content">
          {error ? (
            <div className="maintenance-error" role="alert">
              <strong>The library is unavailable.</strong> {error}
            </div>
          ) : null}

          {/* Hashing outlives a session, so the strip stays whenever work is
              pending — not only while a run happens to be alive. */}
          {(indexing || hashingPending) && !libraryIsUnindexed ? (
            <div className="library-indexing-strip">
              {indexing ? <span aria-hidden="true" className="health-activity-indicator" /> : null}
              <span className="library-indexing-detail">
                {indexing
                  ? indexingDetail
                  : `Duplicate detection is paused · ${(
                    indexStatus?.counts.pendingFingerprints ?? 0
                  ).toLocaleString()} files still to hash`}
              </span>
              <span className="library-speed-picker">
                {INDEX_SPEED_OPTIONS.map((option) => (
                  <button
                    aria-pressed={resourceProfile === option.value}
                    className={
                      resourceProfile === option.value
                        ? 'library-speed-option is-active'
                        : 'library-speed-option'
                    }
                    key={option.value}
                    onClick={() => void changeIndexingSpeed(option.value)}
                    title={option.hint}
                    type="button"
                  >
                    {option.label}
                  </button>
                ))}
              </span>
              {indexing ? (
                <button
                  className="ghost-button"
                  onClick={() => void cancelMediaIndexScan().then(setIndexStatus)}
                  type="button"
                >
                  Pause
                </button>
              ) : (
                <button
                  className="ghost-button"
                  onClick={() => void resumeFingerprints()}
                  type="button"
                >
                  Resume
                </button>
              )}
            </div>
          ) : null}

          {showsMedia && !libraryIsUnindexed && !indexing ? (
            <header className="library-toolbar">
              <div className="library-filters">
                <label className="library-filter">
                  <span>Provider</span>
                  <select value={provider} onChange={(event) => setProvider(event.target.value)}>
                    <option value="all">All providers</option>
                    {Object.keys(PROVIDER_LABELS).map((key) => (
                      <option key={key} value={key}>
                        {providerLabel(key)}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="library-filter">
                  <span>Media</span>
                  <select
                    value={mediaType}
                    onChange={(event) => setMediaType(event.target.value as MediaTypeFilter)}
                  >
                    <option value="all">Photos and videos</option>
                    <option value="image">Photos</option>
                    <option value="video">Videos</option>
                  </select>
                </label>
              </div>
              <div className="library-toolbar-actions">
                {activeCollection ? (
                  <>
                    {activeCollection.scope !== 'global' ? (
                      <button
                        className="ghost-button"
                        onClick={() =>
                          void promoteCollectionToGlobal(activeCollection.id).then(
                            refreshCollections,
                          )
                        }
                        type="button"
                      >
                        Promote to library
                      </button>
                    ) : null}
                    <button
                      className="ghost-button"
                      onClick={() => {
                        if (!window.confirm(`Delete "${activeCollection.name}"? The media stays on disk.`)) {
                          return
                        }
                        void deleteCollection(activeCollection.id).then(() => {
                          setDestination('all')
                          void refreshCollections()
                        })
                      }}
                      type="button"
                    >
                      Delete collection
                    </button>
                  </>
                ) : (
                  <button
                    className="ghost-button"
                    onClick={() => void saveFilterAsCollection()}
                    title="Save these filters as a collection that keeps itself up to date"
                    type="button"
                  >
                    Save filter as collection
                  </button>
                )}
                {destination === 'new' && newCount > 0 ? (
                  <button className="ghost-button" onClick={() => void markSeen()} type="button">
                    Mark {newCount} as seen
                  </button>
                ) : null}
              </div>
            </header>
          ) : null}

          {selection.size > 0 ? (
            <div className="library-selection-bar">
              <span>{selection.size} selected</span>
              <button
                className="ghost-button"
                onClick={() => void createCollectionFromSelection()}
                type="button"
              >
                New collection…
              </button>
              {collections
                .filter((collection) => collection.kind === 'manual')
                .map((collection) => (
                  <button
                    className="ghost-button"
                    key={collection.id}
                    onClick={() => void addSelectionToCollection(collection.id)}
                    type="button"
                  >
                    Add to {collection.name}
                  </button>
                ))}
              <button className="ghost-button" onClick={() => setSelection(new Set())} type="button">
                Clear
              </button>
            </div>
          ) : null}

          {destination === 'duplicates'
            ? renderDuplicates()
            : destination === 'summary'
              ? renderSummary()
              : renderMedia()}
        </main>
      </div>
    </WindowShell>
  )
}
