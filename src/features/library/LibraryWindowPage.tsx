import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { convertFileSrc } from '@tauri-apps/api/core'
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
  retryFailedMediaFingerprints,
  setMediaIndexResourceProfile,
  markTimelineSeen,
  openExternalTarget,
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
  MediaVariantMember,
} from '../../domain/models'
import { WindowShell } from '../brand/WindowShell'
import { WindowTitlebar } from '../brand/WindowTitlebar'
import { MediaCard } from '../workspace/MediaCard'
import { MediaLightbox } from '../media/MediaLightbox'
import { useLightboxSession } from '../media/lightboxSession'

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
const LIBRARY_DENSITY_STORAGE_KEY = 'library.density'

type LibraryDensity = 'compact' | 'comfortable' | 'large'

const LIBRARY_DENSITIES: Array<{ value: LibraryDensity; label: string; size: number }> = [
  { value: 'compact', label: 'Compact', size: 132 },
  { value: 'comfortable', label: 'Comfortable', size: 168 },
  { value: 'large', label: 'Large', size: 216 },
]

function readStoredDensity(): LibraryDensity {
  try {
    const stored = localStorage.getItem(LIBRARY_DENSITY_STORAGE_KEY)
    if (stored === 'compact' || stored === 'comfortable' || stored === 'large') return stored
  } catch {
    /* best-effort preference */
  }
  return 'comfortable'
}

interface LibraryViewerItem {
  timeline: MediaTimelineItem
  absolutePath: string
  mediaType: string
  groupKey: string
}

interface VariantViewerItem {
  group: MediaVariantGroup
  member: MediaVariantMember
  groupKey: string
}

function variantSectionLabel(member: MediaVariantMember): string {
  const folder = member.relativePath.replace(/\\/g, '/').split('/')[0]?.toLowerCase()
  const folderLabels: Record<string, string> = {
    favorites: 'Favorites',
    favourite: 'Favorites',
    liked: 'Likes',
    likes: 'Likes',
    stories: 'Stories',
    story: 'Stories',
    reels: 'Reels',
    posts: 'Posts',
    media: 'Media',
    video: 'Videos',
  }
  if (folder && folderLabels[folder]) return folderLabels[folder]
  const section = member.mediaSection.trim()
  if (!section) return 'Imported media'
  return section.charAt(0).toUpperCase() + section.slice(1)
}

function variantMatchLabel(group: MediaVariantGroup): string {
  if (group.matchKind === 'exact_sha256') return 'Exact match'
  if (group.matchKind === 'perceptual_video') return 'Similar video'
  return 'Similar image'
}

type LibraryTextDialog = 'save-filter' | 'new-collection'

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
  const [variantLimit, setVariantLimit] = useState(100)
  const [dashboard, setDashboard] = useState<LibraryDashboard>()
  const [indexStatus, setIndexStatus] = useState<MediaIndexStatus>()
  const [selection, setSelection] = useState<Set<string>>(() => new Set())
  const [resourceProfile, setResourceProfile] = useState<'quiet' | 'balanced' | 'fast'>(
    () => readStoredIndexSpeed(),
  )
  const [density, setDensity] = useState<LibraryDensity>(readStoredDensity)
  const [textDialog, setTextDialog] = useState<LibraryTextDialog>()
  const [dialogName, setDialogName] = useState('')
  const [collectionToDelete, setCollectionToDelete] = useState<Collection>()
  const sentinelRef = useRef<HTMLDivElement>(null)
  const scrollRef = useRef<HTMLDivElement>(null)
  const lastSelectedIndexRef = useRef<number | undefined>(undefined)

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
      setVariantGroups(await loadVariantGroups(variantLimit))
    } catch {
      /* ignore */
    }
  }, [variantLimit])

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
    const videos = [
      ...items
      .filter((item) => item.mediaType === 'video')
      .map((item) => item.absolutePath),
      ...variantGroups.flatMap((group) => group.members)
        .filter((member) => member.mediaType === 'video')
        .map((member) => member.absolutePath),
    ]
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
  }, [items, thumbs, variantGroups])

  const markSeen = useCallback(async () => {
    try {
      await markTimelineSeen()
      setNewCount(0)
      if (destination === 'new') void load()
    } catch {
      /* ignore */
    }
  }, [destination, load])

  const saveFilterAsCollection = useCallback(() => {
    setDialogName('')
    setTextDialog('save-filter')
  }, [])

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

  const submitTextDialog = useCallback(async () => {
    const name = dialogName.trim()
    if (!name || !textDialog) return
    try {
      if (textDialog === 'save-filter') {
        await upsertCollection({
          name,
          kind: 'smart',
          scope: 'global',
          ruleJson: JSON.stringify(filter),
        })
        await refreshCollections()
      } else {
        const created = await upsertCollection({ name, kind: 'manual', scope: 'global' })
        if (created) await addSelectionToCollection(created.id)
      }
      setTextDialog(undefined)
      setDialogName('')
    } catch (saveError) {
      setError(saveError instanceof Error ? saveError.message : 'Failed to save the collection.')
    }
  }, [addSelectionToCollection, dialogName, filter, refreshCollections, textDialog])

  const createCollectionFromSelection = useCallback(() => {
    setDialogName('')
    setTextDialog('new-collection')
  }, [])

  const toggleSelected = useCallback((id: string, shiftKey = false) => {
    const itemIndex = items.findIndex((item) => item.id === id)
    const anchor = lastSelectedIndexRef.current
    setSelection((current) => {
      const next = new Set(current)
      if (shiftKey && anchor !== undefined && itemIndex >= 0) {
        const [from, to] = anchor < itemIndex ? [anchor, itemIndex] : [itemIndex, anchor]
        for (let index = from; index <= to; index += 1) {
          const rangeItem = items[index]
          if (rangeItem) next.add(rangeItem.id)
        }
      } else if (next.has(id)) {
        next.delete(id)
      } else {
        next.add(id)
      }
      return next
    })
    if (itemIndex >= 0) lastSelectedIndexRef.current = itemIndex
  }, [items])

  const startIndexing = useCallback(async () => {
    try {
      setIndexStatus(await startMediaIndexScan(undefined, resourceProfile))
    } catch (indexError) {
      setError(indexError instanceof Error ? indexError.message : 'Failed to start indexing.')
    }
  }, [resourceProfile])

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
          setIndexStatus(await setMediaIndexResourceProfile(next))
        } catch {
          /* ignore */
        }
      }
    },
    [indexStatus?.run?.status],
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

  const timelineViewerItems = useMemo<LibraryViewerItem[]>(() =>
    items.flatMap((timeline) => {
      const files = timeline.files.length > 0
        ? timeline.files
        : [{
          absolutePath: timeline.absolutePath,
          relativePath: timeline.relativePath,
          mediaType: timeline.mediaType,
        }]
      return files.map((file) => ({
        timeline,
        absolutePath: file.absolutePath,
        mediaType: file.mediaType,
        groupKey: timeline.id,
      }))
    }), [items])
  const lightbox = useLightboxSession({
    items: timelineViewerItems,
    groupKeyFor: useCallback((item: LibraryViewerItem) => item.groupKey, []),
  })
  const variantViewerItems = useMemo<VariantViewerItem[]>(() =>
    variantGroups.flatMap((group) => group.members.map((member) => ({
      group,
      member,
      groupKey: group.id,
    }))), [variantGroups])
  const variantLightbox = useLightboxSession({
    items: variantViewerItems,
    groupKeyFor: useCallback((item: VariantViewerItem) => item.groupKey, []),
  })
  const variantViewerIndexByMedia = useMemo(() => new Map(
    variantViewerItems.map((item, index) => [item.member.mediaId, index]),
  ), [variantViewerItems])
  const firstViewerIndexByTimeline = useMemo(() => {
    const result = new Map<string, number>()
    timelineViewerItems.forEach((item, index) => {
      if (!result.has(item.timeline.id)) result.set(item.timeline.id, index)
    })
    return result
  }, [timelineViewerItems])

  const densitySize = LIBRARY_DENSITIES.find((entry) => entry.value === density)?.size ?? 168
  const estimateDaySize = useCallback((index: number) => {
    const availableWidth = scrollRef.current?.clientWidth || 1024
    const columns = Math.max(1, Math.floor((availableWidth + 9) / (densitySize + 9)))
    const rows = Math.max(1, Math.ceil((days[index]?.items.length ?? 1) / columns))
    return 36 + rows * (densitySize * 4 / 3 + 9) + 18
  }, [days, densitySize])
  const dayVirtualizer = useVirtualizer({
    count: days.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: estimateDaySize,
    initialRect: { width: 1024, height: 768 },
    observeElementRect: (_instance, callback) => {
      const element = scrollRef.current
      const notify = () => callback({
        width: element?.clientWidth || 1024,
        height: element?.clientHeight || 768,
      })
      notify()
      if (!element || typeof ResizeObserver === 'undefined') return () => undefined
      const observer = new ResizeObserver(notify)
      observer.observe(element)
      return () => observer.disconnect()
    },
    overscan: 2,
  })

  const indexing = indexStatus?.run?.status === 'running'
    || indexStatus?.run?.status === 'queued'
    || indexStatus?.run?.status === 'pausing'
  const planningCandidates = indexing && indexStatus?.run?.stage === 'planning'
  // Nothing indexed yet is a first-run state, not an empty result: the operator
  // needs the reason and the fix, not an empty grid.
  const libraryIsUnindexed = (indexStatus?.counts.totalFiles ?? 0) === 0
  // Work left over from a previous session: duplicate detection cannot run
  // until these are hashed, so it stays visible and resumable.
  const hashingPending = (indexStatus?.counts.pendingFingerprints ?? 0) > 0
  const hashingFailed = (indexStatus?.counts.failedFingerprints ?? 0) > 0

  /**
   * Walking the profile folders is only the first stage; hashing every file for
   * duplicate detection is the long one. Reporting a single "profiles" number
   * made the run look stuck at 100% while it was still working.
   */
  const indexingDetail = (() => {
    const run = indexStatus?.run
    if (!run) return 'Starting…'
    if (run.stage === 'planning') return 'Planning duplicate candidates…'
    if (['planning', 'exact', 'image_similarity', 'video_similarity', 'grouping', 'fingerprint'].includes(run.stage)) {
      const total = run.fingerprintsTotal
      const done = run.fingerprintsDone
      const remaining = Math.max(0, total - done)
      if (remaining === 0) return 'Finishing duplicate detection · your media is already browsable'
      const percent = total > 0 ? Math.round((done / total) * 100) : 0
      const heartbeatExpired = Boolean(run.lastProgressAt)
        && Date.now() - Date.parse(run.lastProgressAt!) > 10_000
      const rate = heartbeatExpired ? 0 : run.ratePerSecond
      const phaseLabel = ({
        planning: 'Planning candidates',
        exact: 'Exact duplicates',
        image_similarity: 'Image similarity',
        video_similarity: 'Video similarity',
        grouping: 'Grouping matches',
        fingerprint: 'Fingerprinting',
      } as Record<string, string>)[run.stage] ?? 'Fingerprinting'
      const parts = [
        `${heartbeatExpired && run.status === 'running' ? 'Stopped' : phaseLabel} · ${done.toLocaleString()} of ${total.toLocaleString()} (${percent}%)`,
      ]
      if (rate > 0) {
        parts.push(`${rate < 10 ? rate.toFixed(1) : Math.round(rate)}/s`)
        parts.push(`about ${formatDuration(run.etaSeconds ?? remaining / rate)} left`)
      }
      if (run.phaseFailed > 0) parts.push(`${run.phaseFailed.toLocaleString()} failed`)
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
      <div
        className="library-scroll"
        ref={scrollRef}
        style={{ '--library-card-size': `${densitySize}px` } as CSSProperties}
      >
        <div
          className="library-virtual-days"
          style={{ height: `${dayVirtualizer.getTotalSize()}px` }}
        >
          {dayVirtualizer.getVirtualItems().map((virtualDay) => {
            const day = days[virtualDay.index]
            if (!day) return null
            return (
              <section
                className="library-day"
                data-index={virtualDay.index}
                key={day.key}
                ref={dayVirtualizer.measureElement}
                style={{ transform: `translateY(${virtualDay.start}px)` }}
              >
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
                      onToggleSelect={(shiftKey) => toggleSelected(item.id, shiftKey)}
                      onOpen={(shiftKey) => {
                        if (selection.size > 0 || shiftKey) {
                          toggleSelected(item.id, shiftKey)
                          return
                        }
                        const viewerIndex = firstViewerIndexByTimeline.get(item.id)
                        if (viewerIndex !== undefined) {
                          variantLightbox.close()
                          lightbox.open(viewerIndex)
                        }
                      }}
                      hideOnline={!item.postUrl}
                      onlineDisabled={!item.postUrl}
                      onOnline={item.postUrl ? () => void openExternalTarget(item.postUrl!) : undefined}
                      onReveal={() => void revealMediaInFolder(item.absolutePath)}
                      onContextMenu={() => void openProfileViewWindow(item.sourceId)}
                    />
                  ))}
                </div>
              </section>
            )
          })}
        </div>
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
        <header className="library-duplicates-header">
          <div>
            <strong>Review duplicate matches</strong>
            <p>Nothing was deleted. The copy marked “Kept” remains visible in the grid.</p>
          </div>
          <span>{variantGroups.length.toLocaleString()} of {(dashboard?.variantGroups ?? variantGroups.length).toLocaleString()}</span>
        </header>
        <ul className="library-duplicate-list">
          {variantGroups.map((group) => (
            <li className="library-duplicate-card" key={group.id}>
              <header className="library-duplicate-card-header">
                <div>
                  <strong>
                    {group.scope === 'cross_source'
                      ? 'Same upload across linked profiles'
                      : 'Reposted inside one profile'}
                  </strong>
                  <span>@{group.members[0]?.handle.replace(/^@/, '')}</span>
                </div>
                <span className="library-duplicate-match">
                  {variantMatchLabel(group)}
                  {group.matchKind !== 'exact_sha256'
                    ? ` · ${Math.round(group.confidence * 100)}%`
                    : ''}
                </span>
              </header>
              <div className="library-duplicate-members">
                {group.members.map((member) => {
                  const previewPath = member.mediaType === 'video'
                    ? thumbs[member.absolutePath]
                    : member.absolutePath
                  return (
                    <button
                      aria-label={`Preview ${variantSectionLabel(member)} copy`}
                      className="library-duplicate-member"
                      key={member.mediaId}
                      onClick={() => {
                        lightbox.close()
                        const index = variantViewerIndexByMedia.get(member.mediaId)
                        if (index !== undefined) variantLightbox.open(index)
                      }}
                      type="button"
                    >
                      <span className="library-duplicate-preview">
                        {previewPath ? (
                          <img alt="" loading="lazy" src={convertFileSrc(previewPath)} />
                        ) : (
                          <span className="library-duplicate-placeholder">
                            {member.mediaType === 'video' ? 'VIDEO' : 'MEDIA'}
                          </span>
                        )}
                        <span className={member.role === 'canonical' ? 'is-kept' : 'is-extra'}>
                          {member.role === 'canonical' ? 'Kept' : 'Extra'}
                        </span>
                      </span>
                      <span className="library-duplicate-member-meta">
                        <strong>{variantSectionLabel(member)}</strong>
                        <span>{providerLabel(member.provider)} · {formatBytes(member.sizeBytes)}</span>
                      </span>
                    </button>
                  )
                })}
              </div>
              <footer className="library-duplicate-card-actions">
                <span>{group.members.length} copies linked</span>
                <button
                  className="ghost-button"
                  onClick={() => void dismissVariantGroup(group.id).then(refreshVariants)}
                  title="These are different posts — stop grouping them"
                  type="button"
                >
                  Not duplicates
                </button>
              </footer>
            </li>
          ))}
        </ul>
        {(dashboard?.variantGroups ?? 0) > variantGroups.length ? (
          <button
            className="ghost-button library-duplicates-more"
            onClick={() => setVariantLimit((current) => current + 100)}
            type="button"
          >
            Load 100 more
          </button>
        ) : null}
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
          {(indexing || hashingPending || hashingFailed) && !libraryIsUnindexed ? (
            <div className="library-indexing-strip">
              <div className="library-indexing-summary">
                <span className="library-indexing-state">
                  {indexing ? <span aria-hidden="true" className="health-activity-indicator" /> : null}
                  <span className="library-indexing-detail">
                    {indexing
                      ? indexingDetail
                      : hashingPending
                        ? `Duplicate detection is paused · ${(
                          indexStatus?.counts.pendingFingerprints ?? 0
                        ).toLocaleString()} candidates remain`
                        : `Duplicate detection completed · ${(
                          indexStatus?.counts.failedFingerprints ?? 0
                        ).toLocaleString()} files need review or retry`}
                  </span>
                </span>
                <div
                  aria-label="Fingerprint progress"
                  aria-valuemax={indexStatus?.run?.fingerprintsTotal || 1}
                  aria-valuemin={0}
                  aria-valuenow={planningCandidates ? undefined : (indexStatus?.run?.fingerprintsDone ?? 0)}
                  className={planningCandidates
                    ? 'library-indexing-progress is-indeterminate'
                    : 'library-indexing-progress'}
                  role="progressbar"
                >
                  <span
                    style={{
                      width: planningCandidates
                        ? undefined
                        : `${Math.min(100, Math.max(0,
                          ((indexStatus?.run?.fingerprintsDone ?? 0)
                            / Math.max(1, indexStatus?.run?.fingerprintsTotal ?? 0)) * 100,
                        ))}%`,
                    }}
                  />
                </div>
              </div>
              <div className="library-indexing-controls">
                <span aria-label="Indexing speed" className="library-speed-picker" role="group">
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
                {(indexStatus?.counts.failedFingerprints ?? 0) > 0 ? (
                  <button
                    className="ghost-button library-indexing-action"
                    onClick={() => void retryFailedMediaFingerprints().then(setIndexStatus)}
                    type="button"
                  >
                    Retry failed
                  </button>
                ) : null}
                {indexing ? (
                  <button
                    className="ghost-button library-indexing-action"
                    disabled={indexStatus?.run?.status === 'pausing'}
                    onClick={() => void cancelMediaIndexScan().then(setIndexStatus)}
                    type="button"
                  >
                    {indexStatus?.run?.status === 'pausing' ? 'Pausing…' : 'Pause'}
                  </button>
                ) : hashingPending ? (
                  <button
                    className="ghost-button library-indexing-action"
                    onClick={() => void resumeFingerprints()}
                    type="button"
                  >
                    Resume
                  </button>
                ) : null}
              </div>
            </div>
          ) : null}

          {showsMedia && !libraryIsUnindexed ? (
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
                <span aria-label="Card density" className="library-density-picker" role="group">
                  {LIBRARY_DENSITIES.map((entry) => (
                    <button
                      aria-pressed={density === entry.value}
                      className={density === entry.value ? 'is-active' : ''}
                      key={entry.value}
                      onClick={() => {
                        setDensity(entry.value)
                        try {
                          localStorage.setItem(LIBRARY_DENSITY_STORAGE_KEY, entry.value)
                        } catch {
                          /* best-effort preference */
                        }
                      }}
                      title={`${entry.label} · ${entry.size}px`}
                      type="button"
                    >
                      {entry.label}
                    </button>
                  ))}
                </span>
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
                      onClick={() => setCollectionToDelete(activeCollection)}
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
      {lightbox.active ? (
        <MediaLightbox
          fileAbsPath={lightbox.active.item.absolutePath}
          isVideo={lightbox.active.item.mediaType === 'video'}
          audioAbsPath={lightbox.active.item.timeline.audioAbsolutePath}
          title={`@${lightbox.active.item.timeline.handle.replace(/^@/, '')}`}
          meta={[
            providerLabel(lightbox.active.item.timeline.provider),
            lightbox.active.item.timeline.mediaSection,
            lightbox.active.slideCount > 1
              ? `${lightbox.active.slideIndex + 1}/${lightbox.active.slideCount}`
              : undefined,
          ].filter(Boolean).join(' · ')}
          caption={lightbox.active.item.timeline.title}
          hasPrev={lightbox.active.hasPrev}
          hasNext={lightbox.active.hasNext}
          hasSlidePrev={lightbox.active.hasSlidePrev}
          hasSlideNext={lightbox.active.hasSlideNext}
          onPrev={() => lightbox.stepPost(-1)}
          onNext={() => lightbox.stepPost(1)}
          onSlidePrev={() => lightbox.stepSlide(-1)}
          onSlideNext={() => lightbox.stepSlide(1)}
          onClose={lightbox.close}
          actions={(
            <>
              {lightbox.active.item.timeline.postUrl ? (
                <button
                  className="ghost-button"
                  onClick={() => void openExternalTarget(lightbox.active!.item.timeline.postUrl!)}
                  type="button"
                >
                  Open original
                </button>
              ) : null}
              <button
                className="ghost-button"
                onClick={() => void revealMediaInFolder(lightbox.active!.item.absolutePath)}
                type="button"
              >
                Reveal
              </button>
              <button
                className="ghost-button"
                onClick={() => void openProfileViewWindow(lightbox.active!.item.timeline.sourceId)}
                type="button"
              >
                Open profile
              </button>
            </>
          )}
        />
      ) : null}
      {variantLightbox.active ? (
        <MediaLightbox
          fileAbsPath={variantLightbox.active.item.member.absolutePath}
          isVideo={variantLightbox.active.item.member.mediaType === 'video'}
          title={`@${variantLightbox.active.item.member.handle.replace(/^@/, '')}`}
          meta={[
            variantSectionLabel(variantLightbox.active.item.member),
            variantMatchLabel(variantLightbox.active.item.group),
            variantLightbox.active.item.member.role === 'canonical' ? 'Kept copy' : 'Extra copy',
            variantLightbox.active.slideCount > 1
              ? `${variantLightbox.active.slideIndex + 1}/${variantLightbox.active.slideCount}`
              : undefined,
          ].filter(Boolean).join(' · ')}
          hasPrev={variantLightbox.active.hasPrev}
          hasNext={variantLightbox.active.hasNext}
          hasSlidePrev={variantLightbox.active.hasSlidePrev}
          hasSlideNext={variantLightbox.active.hasSlideNext}
          onPrev={() => variantLightbox.stepPost(-1)}
          onNext={() => variantLightbox.stepPost(1)}
          onSlidePrev={() => variantLightbox.stepSlide(-1)}
          onSlideNext={() => variantLightbox.stepSlide(1)}
          onClose={variantLightbox.close}
          actions={(
            <>
              <button
                className="ghost-button"
                onClick={() => void revealMediaInFolder(variantLightbox.active!.item.member.absolutePath)}
                type="button"
              >
                Reveal
              </button>
              <button
                className="ghost-button"
                onClick={() => void openProfileViewWindow(variantLightbox.active!.item.member.sourceId)}
                type="button"
              >
                Open profile
              </button>
              <button
                className="ghost-button"
                onClick={() => {
                  const groupId = variantLightbox.active!.item.group.id
                  variantLightbox.close()
                  void dismissVariantGroup(groupId).then(refreshVariants)
                }}
                type="button"
              >
                Not duplicates
              </button>
            </>
          )}
        />
      ) : null}
      {textDialog ? (
        <div className="library-dialog-backdrop" role="presentation">
          <form
            aria-labelledby="library-text-dialog-title"
            aria-modal="true"
            className="library-dialog"
            onSubmit={(event) => {
              event.preventDefault()
              void submitTextDialog()
            }}
            role="dialog"
          >
            <h2 id="library-text-dialog-title">
              {textDialog === 'save-filter' ? 'Save filter as collection' : 'Create collection'}
            </h2>
            <label>
              <span>Name</span>
              <input
                autoFocus
                onChange={(event) => setDialogName(event.target.value)}
                value={dialogName}
              />
            </label>
            <div className="library-dialog-actions">
              <button className="ghost-button" onClick={() => setTextDialog(undefined)} type="button">
                Cancel
              </button>
              <button className="primary-button" disabled={!dialogName.trim()} type="submit">
                Save
              </button>
            </div>
          </form>
        </div>
      ) : null}
      {collectionToDelete ? (
        <div className="library-dialog-backdrop" role="presentation">
          <div
            aria-labelledby="library-delete-dialog-title"
            aria-modal="true"
            className="library-dialog"
            role="alertdialog"
          >
            <h2 id="library-delete-dialog-title">Delete “{collectionToDelete.name}”?</h2>
            <p>The collection will be removed. Its media stays on disk.</p>
            <div className="library-dialog-actions">
              <button className="ghost-button" onClick={() => setCollectionToDelete(undefined)} type="button">
                Cancel
              </button>
              <button
                className="danger-button"
                onClick={() => {
                  const collectionId = collectionToDelete.id
                  setCollectionToDelete(undefined)
                  void deleteCollection(collectionId).then(() => {
                    setDestination('all')
                    void refreshCollections()
                  })
                }}
                type="button"
              >
                Delete collection
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </WindowShell>
  )
}
