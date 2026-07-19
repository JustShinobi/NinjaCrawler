import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import {
  cancelSourceSyncProfile,
  cancelSourceSyncProvider,
  cancelMediaPathMigrations,
  enqueueMediaThumbnailGeneration,
  loadMediaThumbnailQueueStatus,
  loadMediaPathMigrationQueueStatus,
  loadMediaDedupeStatus,
  loadSourceDeleteQueueStatus,
  loadSourceSyncQueueStatus,
  loadWorkspaceSnapshot,
  openConnectorDebugWindow,
  openWorkspaceHealthWindow,
  pauseSourceSyncProvider,
  reorderSourceSyncProviderQueue,
  resolveMediaThumbnailReview,
  resumeSourceSyncProvider,
  runSourceSync,
  loadSingleVideoQueueStatus,
  subscribeToDesktopRuntimeEvents,
  subscribeToSingleVideoQueue,
} from '../../bridge/desktop'
import { DEFAULT_PROVIDER_CATALOG } from '../../domain/defaults'
import type {
  ProviderKey,
  SourceDeleteQueueJob,
  SourceDeleteQueueRecentResult,
  SourceDeleteQueueStatus,
  SourceSyncQueueItem,
  SourceSyncQueueProviderStatus,
  SourceSyncQueueRecentResult,
  SourceSyncQueueStatus,
  MediaThumbnailQueueStatus,
  MediaThumbnailReviewItem,
  MediaPathMigrationQueueStatus,
  MediaDedupeJobStatus,
  SchedulerGroup,
  SingleVideoQueueRecentResult,
  SingleVideoQueueStatus,
  SourceProfile,
} from '../../domain/models'
import { WindowShell } from '../brand/WindowShell'
import { WindowTitlebar } from '../brand/WindowTitlebar'

type QueueOperation = 'Sync' | 'Delete' | 'Single' | 'Migration' | 'Thumbnail'

const QUEUED_COLLAPSE_LIMIT = 6

interface QueueLiveTask {
  key: string
  queueKey?: string
  sourceId: string
  provider: ProviderKey
  providerLabel: string
  handle: string
  operation: QueueOperation
  modeDetail?: string
  state: 'queued' | 'running' | 'held'
  queuedAt: string
  startedAt?: string
  progressPercent?: number
  progressLabel?: string
  progressDetail?: string
  progressIndeterminate?: boolean
  filesProcessed?: number
  filesTotal?: number
  holdUntil?: string
  cancelSourceId?: string
}

interface QueueResultTask {
  key: string
  sourceId: string
  provider: ProviderKey
  providerLabel: string
  handle: string
  operation: QueueOperation
  modeDetail?: string
  status: 'succeeded' | 'warning' | 'failed' | 'skipped'
  summary: string
  finishedAt: string
  error?: string
  /** Thumbnail jobs only: invalid/corrupt media for manual review. */
  reviewItems?: MediaThumbnailReviewItem[]
  invalidMedia?: number
  generationFailed?: number
}

interface ProviderLane {
  provider: ProviderKey
  displayName: string
  running: QueueLiveTask[]
  queued: QueueLiveTask[]
  completed: number
  failed: number
  paused: boolean
  activeProgressPercent?: number
}

function resultStatusClassName(status: 'succeeded' | 'warning' | 'failed' | 'skipped'): string {
  switch (status) {
    case 'failed':
      return 'status status-failed'
    case 'warning':
      return 'status status-warning'
    case 'skipped':
      return 'status status-skipped queue-recent-status-quiet'
    default:
      return 'status status-succeeded queue-recent-status-quiet'
  }
}

function createEmptySyncQueueStatus(): SourceSyncQueueStatus {
  return {
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
  }
}

function createEmptyDeleteQueueStatus(): SourceDeleteQueueStatus {
  return {
    queuedCount: 0,
    runningCount: 0,
    completedCount: 0,
    failedCount: 0,
    totalCount: 0,
    queuedItems: [],
    runningItems: [],
    recentResults: [],
    updatedAt: new Date().toISOString(),
  }
}

function absoluteTimestamp(value?: string): string {
  if (!value) {
    return '—'
  }
  const parsed = new Date(value)
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleString()
}

// Tempo relativo curto ("just now", "2m ago", "1h ago", "3d ago").
function relativeTime(value: string | undefined, now: number): string {
  if (!value) {
    return ''
  }
  const ms = Date.parse(value)
  if (Number.isNaN(ms)) {
    return ''
  }
  const diff = Math.max(0, now - ms)
  const secs = Math.floor(diff / 1000)
  if (secs < 10) return 'just now'
  if (secs < 60) return `${secs}s ago`
  const mins = Math.floor(secs / 60)
  if (mins < 60) return `${mins}m ago`
  const hours = Math.floor(mins / 60)
  if (hours < 24) return `${hours}h ago`
  const days = Math.floor(hours / 24)
  return `${days}d ago`
}

// Duração corrida ("2m 14s", "45s", "1h 03m").
function elapsed(value: string | undefined, now: number): string {
  if (!value) {
    return ''
  }
  const ms = Date.parse(value)
  if (Number.isNaN(ms)) {
    return ''
  }
  const secs = Math.max(0, Math.floor((now - ms) / 1000))
  if (secs < 60) return `${secs}s`
  const mins = Math.floor(secs / 60)
  const rem = secs % 60
  if (mins < 60) return `${mins}m ${String(rem).padStart(2, '0')}s`
  const hours = Math.floor(mins / 60)
  return `${hours}h ${String(mins % 60).padStart(2, '0')}m`
}

function providerOrderKey(provider: ProviderKey): number {
  const index = DEFAULT_PROVIDER_CATALOG.findIndex((entry) => entry.key === provider)
  return index >= 0 ? index : Number.MAX_SAFE_INTEGER
}

function providerDisplayName(provider: ProviderKey): string {
  return DEFAULT_PROVIDER_CATALOG.find((entry) => entry.key === provider)?.displayName ?? provider
}

function formatDeleteModeDetail(mode: 'user_only' | 'with_media'): string {
  return mode === 'user_only' ? 'user only' : 'with media'
}

function createSyncLiveTask(item: SourceSyncQueueItem): QueueLiveTask {
  const jobKey = item.jobKey ?? item.sourceId
  return {
    key: `sync-${item.state}-${jobKey}`,
    queueKey: jobKey,
    sourceId: item.sourceId,
    provider: item.provider,
    providerLabel: providerDisplayName(item.provider),
    handle: item.handle,
    operation: 'Sync',
    state: item.state,
    queuedAt: item.queuedAt,
    startedAt: item.startedAt,
    progressPercent: item.progressPercent,
    progressLabel: item.progressLabel,
    progressDetail: item.progressDetail,
    progressIndeterminate: item.progressIndeterminate,
    holdUntil: item.holdUntil,
    cancelSourceId: item.sourceId,
  }
}

function createDeleteLiveTask(item: SourceDeleteQueueJob): QueueLiveTask {
  return {
    key: `delete-${item.state}-${item.jobId}`,
    sourceId: item.sourceId,
    provider: item.provider,
    providerLabel: providerDisplayName(item.provider),
    handle: item.handle,
    operation: 'Delete',
    modeDetail: formatDeleteModeDetail(item.mode),
    state: item.state,
    queuedAt: item.queuedAt,
    startedAt: item.startedAt,
    progressPercent: item.progressPercent,
    progressLabel: item.progressLabel,
    progressDetail: item.progressDetail,
    progressIndeterminate: item.progressIndeterminate,
    filesProcessed: item.filesProcessed,
    filesTotal: item.filesTotal,
  }
}

function createSyncResultTask(result: SourceSyncQueueRecentResult): QueueResultTask {
  return {
    key: `sync-result-${result.sourceId}-${result.finishedAt}`,
    sourceId: result.sourceId,
    provider: result.provider,
    providerLabel: providerDisplayName(result.provider),
    handle: result.handle,
    operation: 'Sync',
    status: result.status,
    summary: result.summary,
    finishedAt: result.finishedAt,
  }
}

function createDeleteResultTask(result: SourceDeleteQueueRecentResult): QueueResultTask {
  return {
    key: `delete-result-${result.jobId}-${result.finishedAt}`,
    sourceId: result.sourceId,
    provider: result.provider,
    providerLabel: providerDisplayName(result.provider),
    handle: result.handle,
    operation: 'Delete',
    modeDetail: formatDeleteModeDetail(result.mode),
    status: result.status,
    summary: result.summary,
    finishedAt: result.finishedAt,
    error: result.error,
  }
}

function createSingleVideoResultTask(result: SingleVideoQueueRecentResult): QueueResultTask {
  const provider = (result.provider ?? 'tiktok') as ProviderKey
  const handle = result.uploader
    ? `@${result.uploader}`
    : result.title?.trim() || result.url
  return {
    key: `single-result-${result.url}-${result.finishedAt}`,
    sourceId: '',
    provider,
    providerLabel: result.provider ? providerDisplayName(provider) : 'Single video',
    handle,
    operation: 'Single',
    status: result.status,
    summary: result.status === 'failed' ? result.summary : result.url,
    finishedAt: result.finishedAt,
    error: result.status === 'failed' ? result.summary : undefined,
  }
}

function migrationStageLabel(stage: string): string {
  switch (stage) {
    case 'scanning': return 'Scanning media'
    case 'updating_profile': return 'Updating profile path'
    case 'finalizing': return 'Finalizing move'
    case 'moving': return 'Moving media'
    default: return 'Waiting to move'
  }
}

function avatarInitial(handle: string): string {
  const cleaned = handle.replace(/^@/, '').trim()
  return cleaned ? cleaned[0]!.toUpperCase() : '?'
}

interface TaskAvatarProps {
  handle: string
  provider: ProviderKey
  imagePath?: string
}

function TaskAvatar({ handle, provider, imagePath }: TaskAvatarProps) {
  const [failed, setFailed] = useState(false)
  const src = imagePath && !failed ? convertFileSrc(imagePath) : undefined
  return (
    <span className={`queue-task-avatar provider-ring-${provider}`} aria-hidden="true">
      {src ? (
        <img src={src} alt="" draggable={false} onError={() => setFailed(true)} loading="lazy" />
      ) : (
        <span className="queue-task-avatar-fallback">{avatarInitial(handle)}</span>
      )}
    </span>
  )
}

export function SourceSyncQueueWindowPage() {
  const [syncStatus, setSyncStatus] = useState<SourceSyncQueueStatus>(() => createEmptySyncQueueStatus())
  const [deleteStatus, setDeleteStatus] = useState<SourceDeleteQueueStatus>(() => createEmptyDeleteQueueStatus())
  const [avatarsBySource, setAvatarsBySource] = useState<Record<string, string>>({})
  const [expandedProviders, setExpandedProviders] = useState<Set<ProviderKey>>(() => new Set())
  const [busyProviders, setBusyProviders] = useState<Set<ProviderKey>>(() => new Set())
  // Override otimista da ordem por provider (aplicado até o backend confirmar).
  const [queueOrderOverride, setQueueOrderOverride] = useState<Record<string, string[]>>({})
  const [dragState, setDragState] = useState<{ provider: ProviderKey; jobKey: string } | null>(null)
  const [dropTargetKey, setDropTargetKey] = useState<string | null>(null)
  const [now, setNow] = useState(() => Date.now())
  const [error, setError] = useState<string>()
  const [singleVideoStatus, setSingleVideoStatus] = useState<SingleVideoQueueStatus | undefined>()
  const [openingDebugger, setOpeningDebugger] = useState(false)
  const [librarySources, setLibrarySources] = useState<SourceProfile[]>([])
  const [libraryGroups, setLibraryGroups] = useState<SchedulerGroup[]>([])
  const [thumbnailScope, setThumbnailScope] = useState<'all' | 'provider' | 'group' | 'profile'>('profile')
  const [thumbnailScopeValue, setThumbnailScopeValue] = useState('')
  const [thumbnailStatus, setThumbnailStatus] = useState<MediaThumbnailQueueStatus>()
  const [migrationStatus, setMigrationStatus] = useState<MediaPathMigrationQueueStatus>()
  const [dedupeStatus, setDedupeStatus] = useState<MediaDedupeJobStatus>()
  const [queueingThumbnails, setQueueingThumbnails] = useState(false)
  const [maintenanceOpen, setMaintenanceOpen] = useState(false)
  const [maintenanceError, setMaintenanceError] = useState<string>()
  const [cancellingMigrations, setCancellingMigrations] = useState(false)
  const [resolvingReviewKey, setResolvingReviewKey] = useState<string>()
  const maintenanceButtonRef = useRef<HTMLButtonElement>(null)

  const refreshQueueStatus = useCallback(async (silent = false) => {
    try {
      const [nextSyncStatus, nextDeleteStatus] = await Promise.all([
        loadSourceSyncQueueStatus(),
        loadSourceDeleteQueueStatus(),
      ])
      void loadMediaPathMigrationQueueStatus().then(setMigrationStatus).catch(() => undefined)
      setSyncStatus(nextSyncStatus)
      setDeleteStatus(nextDeleteStatus)
      if (!silent) {
        setError(undefined)
      }
    } catch (refreshError) {
      if (!silent) {
        setError(
          refreshError instanceof Error ? refreshError.message : 'Failed to load queue status.',
        )
      }
    }
  }, [])

  // Avatares: o status da fila não traz o caminho da imagem, então mapeamos
  // sourceId -> profileImagePath a partir do snapshot (carregado uma vez; avatar
  // é estável). Recarrega periodicamente para captar perfis novos.
  const refreshAvatars = useCallback(async () => {
    try {
      const snapshot = await loadWorkspaceSnapshot()
      setLibrarySources(snapshot.sources)
      setLibraryGroups(snapshot.schedulerGroups)
      const map: Record<string, string> = {}
      for (const source of snapshot.sources) {
        if (source.profileImagePath) {
          map[source.id] = source.profileImagePath
        }
      }
      setAvatarsBySource(map)
    } catch {
      // avatar é cosmético; ignora falha
    }
  }, [])

  useEffect(() => {
    const refresh = () => {
      void loadMediaThumbnailQueueStatus().then(setThumbnailStatus).catch(() => undefined)
      void loadMediaDedupeStatus().then(setDedupeStatus).catch(() => undefined)
    }
    refresh()
    const timer = window.setInterval(refresh, 750)
    return () => window.clearInterval(timer)
  }, [])

  const thumbnailTargetIds = useMemo(() => {
    switch (thumbnailScope) {
      case 'all':
        return librarySources.map((source) => source.id)
      case 'provider':
        return librarySources
          .filter((source) => source.provider === thumbnailScopeValue)
          .map((source) => source.id)
      case 'group':
        return librarySources
          .filter((source) => source.groupId === thumbnailScopeValue)
          .map((source) => source.id)
      default:
        return thumbnailScopeValue ? [thumbnailScopeValue] : []
    }
  }, [librarySources, thumbnailScope, thumbnailScopeValue])

  const handleResolveThumbnailReview = async (
    taskKey: string,
    sourceId: string,
    items: MediaThumbnailReviewItem[],
  ) => {
    const relativePaths = items
      .map((item) => item.relativePath)
      .filter((path) => path.trim().length > 0)
    if (relativePaths.length === 0) return
    setResolvingReviewKey(taskKey)
    try {
      setThumbnailStatus(await resolveMediaThumbnailReview(sourceId, relativePaths))
      setError(undefined)
    } catch (resolveError) {
      setError(
        resolveError instanceof Error
          ? resolveError.message
          : 'Failed to move invalid media to the Recycle Bin.',
      )
    } finally {
      setResolvingReviewKey(undefined)
    }
  }

  const handleQueueThumbnails = async () => {
    if (thumbnailTargetIds.length === 0) return
    setQueueingThumbnails(true)
    try {
      setThumbnailStatus(await enqueueMediaThumbnailGeneration(thumbnailTargetIds))
      setMaintenanceError(undefined)
    } catch (queueError) {
      setMaintenanceError(queueError instanceof Error ? queueError.message : 'Failed to queue thumbnails.')
      setMaintenanceOpen(true)
    } finally {
      setQueueingThumbnails(false)
    }
  }

  const handleCancelMigrations = async () => {
    setCancellingMigrations(true)
    try {
      setMigrationStatus(await cancelMediaPathMigrations())
      setMaintenanceError(undefined)
    } catch (cancelError) {
      setMaintenanceError(cancelError instanceof Error ? cancelError.message : 'Failed to cancel media path migrations.')
      setMaintenanceOpen(true)
    } finally {
      setCancellingMigrations(false)
    }
  }

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void refreshQueueStatus()
      void refreshAvatars()
      void loadSingleVideoQueueStatus()
        .then(setSingleVideoStatus)
        .catch(() => undefined)
    }, 0)
    return () => window.clearTimeout(timer)
  }, [refreshQueueStatus, refreshAvatars])

  useEffect(() => {
    if ((migrationStatus?.recentResults.length ?? 0) > 0) {
      void refreshAvatars()
    }
  }, [migrationStatus?.recentResults.length, refreshAvatars])

  useEffect(() => {
    const timer = window.setInterval(() => void loadMediaPathMigrationQueueStatus().then(setMigrationStatus).catch(() => undefined), 1000)
    return () => window.clearInterval(timer)
  }, [])

  useEffect(() => {
    let disposed = false
    let unsubscribe: (() => void) | undefined
    void subscribeToSingleVideoQueue((next) => {
      if (!disposed) setSingleVideoStatus(next)
    })
      .then((teardown) => {
        if (disposed) teardown()
        else unsubscribe = teardown
      })
      .catch(() => undefined)
    return () => {
      disposed = true
      unsubscribe?.()
    }
  }, [])

  useEffect(() => {
    let disposed = false
    let unsubscribe: (() => void) | undefined
    void subscribeToDesktopRuntimeEvents({
      onSourceSyncQueueChanged: (next) => {
        if (!disposed) setSyncStatus(next)
      },
      onSourceDeleteQueueChanged: (next) => {
        if (!disposed) setDeleteStatus(next)
      },
      onMediaPathMigrationQueueChanged: setMigrationStatus,
    })
      .then((teardown) => {
        if (disposed) {
          teardown()
          return
        }
        unsubscribe = teardown
      })
      .catch(() => undefined)
    return () => {
      disposed = true
      unsubscribe?.()
    }
  }, [])

  useEffect(() => {
    // Poll faster while a delete is running so stage/file progress stays live
    // even if a few IPC events are coalesced.
    const hasActiveDelete =
      deleteStatus.runningCount > 0 || deleteStatus.queuedCount > 0
    const timer = window.setInterval(
      () => void refreshQueueStatus(true),
      hasActiveDelete ? 400 : 1200,
    )
    return () => window.clearInterval(timer)
  }, [refreshQueueStatus, deleteStatus.runningCount, deleteStatus.queuedCount])

  useEffect(() => {
    const timer = window.setInterval(() => void refreshAvatars(), 30_000)
    return () => window.clearInterval(timer)
  }, [refreshAvatars])

  // Relógio de 1s para tempos relativos/durações ao vivo.
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [])

  const withProviderBusy = useCallback(
    async (provider: ProviderKey, action: () => Promise<unknown>, failureMessage: string) => {
      setBusyProviders((prev) => new Set(prev).add(provider))
      try {
        await action()
        setError(undefined)
      } catch (actionError) {
        setError(actionError instanceof Error ? actionError.message : failureMessage)
      } finally {
        setBusyProviders((prev) => {
          const next = new Set(prev)
          next.delete(provider)
          return next
        })
      }
    },
    [],
  )

  const handleCancelProvider = useCallback(
    (provider: ProviderKey) =>
      withProviderBusy(provider, () => cancelSourceSyncProvider(provider), `Failed to cancel '${provider}' queue.`),
    [withProviderBusy],
  )

  const handleTogglePause = useCallback(
    (provider: ProviderKey, paused: boolean) =>
      withProviderBusy(
        provider,
        () => (paused ? resumeSourceSyncProvider(provider) : pauseSourceSyncProvider(provider)),
        `Failed to ${paused ? 'resume' : 'pause'} '${provider}'.`,
      ),
    [withProviderBusy],
  )

  const handleCancelProfile = useCallback(async (sourceId: string) => {
    try {
      await cancelSourceSyncProfile(sourceId)
      setError(undefined)
    } catch (cancelError) {
      setError(cancelError instanceof Error ? cancelError.message : `Failed to cancel job '${sourceId}'.`)
    }
  }, [])

  const handleRetry = useCallback(async (sourceId: string) => {
    try {
      await runSourceSync(sourceId, { trigger: 'manual' })
      setError(undefined)
    } catch (retryError) {
      setError(retryError instanceof Error ? retryError.message : `Failed to retry '${sourceId}'.`)
    }
  }, [])

  const handleOpenDebugger = useCallback(async () => {
    setOpeningDebugger(true)
    try {
      await openConnectorDebugWindow()
      setError(undefined)
    } catch (openError) {
      setError(
        openError instanceof Error ? openError.message : 'Failed to open the realtime debugger.',
      )
    } finally {
      setOpeningDebugger(false)
    }
  }, [])

  const toggleExpanded = useCallback((provider: ProviderKey) => {
    setExpandedProviders((prev) => {
      const next = new Set(prev)
      if (next.has(provider)) {
        next.delete(provider)
      } else {
        next.add(provider)
      }
      return next
    })
  }, [])

  const runningTasks = useMemo(
    () => [
      ...syncStatus.runningItems.map(createSyncLiveTask),
      ...deleteStatus.runningItems.map(createDeleteLiveTask),
    ],
    [deleteStatus.runningItems, syncStatus.runningItems],
  )

  const queuedTasks = useMemo(
    () => [
      ...syncStatus.queuedItems.map(createSyncLiveTask),
      ...deleteStatus.queuedItems.map(createDeleteLiveTask),
    ],
    [deleteStatus.queuedItems, syncStatus.queuedItems],
  )

  const recentTasks = useMemo(
    () =>
      [
        ...syncStatus.recentResults.map(createSyncResultTask),
        ...deleteStatus.recentResults.map(createDeleteResultTask),
        ...(singleVideoStatus?.recentResults ?? []).map(createSingleVideoResultTask),
        ...(migrationStatus?.recentResults ?? []).map((result): QueueResultTask => ({
          key: `migration-result-${result.jobId}-${result.finishedAt}`,
          sourceId: result.sourceId,
          provider: result.provider,
          providerLabel: providerDisplayName(result.provider),
          handle: result.handle,
          operation: 'Migration',
          status: result.status === 'cancelled' ? 'skipped' : result.status,
          summary: result.summary,
          finishedAt: result.finishedAt,
          error: result.error,
        })),
        ...(thumbnailStatus?.recentResults ?? []).map((result): QueueResultTask => {
          const invalidMedia = result.invalidMedia ?? 0
          const generationFailed = result.failed ?? 0
          const reviewItems = result.reviewItems ?? []
          const summaryParts = [
            `${result.generated} generated`,
            `${result.skippedExisting} existing`,
          ]
          if (invalidMedia > 0) summaryParts.push(`${invalidMedia} invalid`)
          if (generationFailed > 0) summaryParts.push(`${generationFailed} failed`)
          return {
            key: `thumbnail-result-${result.sourceId}-${result.finishedAt}`,
            sourceId: result.sourceId,
            provider: result.provider,
            providerLabel: providerDisplayName(result.provider),
            handle: result.handle,
            operation: 'Thumbnail',
            status: result.status,
            summary: result.summary?.trim()
              ? result.summary
              : summaryParts.join(' · '),
            finishedAt: result.finishedAt,
            reviewItems,
            invalidMedia,
            generationFailed,
          }
        }),
      ].sort((left, right) => Date.parse(right.finishedAt) - Date.parse(left.finishedAt)),
    [deleteStatus.recentResults, migrationStatus?.recentResults, syncStatus.recentResults, singleVideoStatus?.recentResults, thumbnailStatus?.recentResults],
  )

  const providerStatusByKey = useMemo(() => {
    const map = new Map<ProviderKey, SourceSyncQueueProviderStatus>()
    for (const row of syncStatus.providers) {
      map.set(row.provider, row)
    }
    return map
  }, [syncStatus.providers])

  const lanes = useMemo<ProviderLane[]>(() => {
    const keys = new Set<ProviderKey>()
    for (const task of runningTasks) keys.add(task.provider)
    for (const task of queuedTasks) keys.add(task.provider)
    for (const row of syncStatus.providers) {
      if (row.running > 0 || row.queued > 0 || row.completed > 0 || row.failed > 0) {
        keys.add(row.provider)
      }
    }

    return Array.from(keys)
      .map((provider) => {
        const status = providerStatusByKey.get(provider)
        const running = runningTasks
          .filter((task) => task.provider === provider)
          .sort((a, b) => Date.parse(a.startedAt ?? a.queuedAt) - Date.parse(b.startedAt ?? b.queuedAt))
        // A ordem do payload é autoritativa: o backend emite cada sub-fila na
        // ordem real de execução (incluindo reordenação manual). Ordenar por
        // queuedAt aqui desfazia o drag-and-drop a cada evento da fila.
        let queued = queuedTasks.filter((task) => task.provider === provider)
        const override = queueOrderOverride[provider]
        if (override) {
          const rank = new Map(override.map((id, index) => [id, index]))
          queued = [...queued].sort(
            (a, b) =>
              (rank.get(a.queueKey ?? a.sourceId) ?? Infinity)
              - (rank.get(b.queueKey ?? b.sourceId) ?? Infinity),
          )
        }
        return {
          provider,
          displayName: providerDisplayName(provider),
          running,
          queued,
          completed: status?.completed ?? 0,
          failed: status?.failed ?? 0,
          paused: status?.paused ?? false,
          activeProgressPercent: status?.activeProgressPercent,
        }
      })
      .sort((a, b) => providerOrderKey(a.provider) - providerOrderKey(b.provider))
  }, [providerStatusByKey, queueOrderOverride, queuedTasks, runningTasks, syncStatus.providers])

  // Reconcilia o override otimista: descarta quando o backend já refletiu a nova
  // ordem (igual) ou quando o conjunto de itens em fila mudou (override obsoleto).
  useEffect(() => {
    setQueueOrderOverride((prev) => {
      const keys = Object.keys(prev)
      if (keys.length === 0) {
        return prev
      }
      const next = { ...prev }
      let changed = false
      for (const provider of keys) {
        const currentIds = syncStatus.queuedItems
          .filter((item) => item.provider === provider)
          .map((item) => item.jobKey ?? item.sourceId)
        const overrideIds = prev[provider]
        const sameSet =
          currentIds.length === overrideIds.length && currentIds.every((id) => overrideIds.includes(id))
        const sameOrder = sameSet && currentIds.every((id, index) => overrideIds[index] === id)
        if (!sameSet || sameOrder) {
          delete next[provider]
          changed = true
        }
      }
      return changed ? next : prev
    })
  }, [syncStatus.queuedItems])

  const idleProviders = useMemo(() => {
    const active = new Set(lanes.map((lane) => lane.provider))
    return DEFAULT_PROVIDER_CATALOG.filter((descriptor) => !active.has(descriptor.key))
  }, [lanes])

  const totals = useMemo(
    () => ({
      queued: syncStatus.queuedCount + deleteStatus.queuedCount + (singleVideoStatus?.queuedCount ?? 0) + (migrationStatus?.queuedCount ?? 0) + (thumbnailStatus?.queuedCount ?? 0) + (dedupeStatus?.state === 'queued' ? 1 : 0),
      running: syncStatus.runningCount + deleteStatus.runningCount + (singleVideoStatus?.runningCount ?? 0) + (migrationStatus?.runningCount ?? 0) + (thumbnailStatus?.runningCount ?? 0) + (['scanning', 'applying'].includes(dedupeStatus?.state ?? '') ? 1 : 0),
      completed: syncStatus.completedCount + deleteStatus.completedCount + (singleVideoStatus?.completedCount ?? 0) + (migrationStatus?.completedCount ?? 0) + (thumbnailStatus?.completedCount ?? 0),
      failed: syncStatus.failedCount + deleteStatus.failedCount + (singleVideoStatus?.failedCount ?? 0) + (migrationStatus?.failedCount ?? 0) + (thumbnailStatus?.failedCount ?? 0),
    }),
    [
      deleteStatus.completedCount,
      deleteStatus.failedCount,
      deleteStatus.queuedCount,
      deleteStatus.runningCount,
      syncStatus.completedCount,
      syncStatus.failedCount,
      syncStatus.queuedCount,
      syncStatus.runningCount,
      singleVideoStatus,
      migrationStatus,
      thumbnailStatus,
      dedupeStatus,
    ],
  )

  const dedupeRunning = ['scanning', 'applying'].includes(dedupeStatus?.state ?? '') ? 1 : 0
  const dedupeQueued = dedupeStatus?.state === 'queued' ? 1 : 0
  const maintenanceRunning = (migrationStatus?.runningCount ?? 0) + (thumbnailStatus?.runningCount ?? 0) + dedupeRunning
  const maintenanceQueued = (migrationStatus?.queuedCount ?? 0) + (thumbnailStatus?.queuedCount ?? 0) + dedupeQueued
  const maintenanceActive = maintenanceRunning + maintenanceQueued > 0
  const globalState = totals.failed > 0 ? 'Needs attention' : totals.running > 0 ? `${totals.running} active` : totals.queued > 0 ? `${totals.queued} queued` : 'Idle'

  useEffect(() => {
    if (!maintenanceOpen) return
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      // Capture + stop so the entrypoint does not close the window first.
      event.preventDefault()
      event.stopImmediatePropagation()
      setMaintenanceOpen(false)
      maintenanceButtonRef.current?.focus()
    }
    window.addEventListener('keydown', handleKeyDown, true)
    return () => window.removeEventListener('keydown', handleKeyDown, true)
  }, [maintenanceOpen])

  const activeProviderCount = lanes.filter((lane) => lane.running.length > 0).length
  const queuedProviderCount = lanes.filter((lane) => lane.queued.length > 0).length

  // Lista de job keys (apenas Sync em fila) de uma raia, na ordem exibida.
  const laneQueuedSyncIds = useCallback(
    (provider: ProviderKey): string[] => {
      return syncStatus.queuedItems
        .filter((item) => item.provider === provider)
        .map((item) => item.jobKey ?? item.sourceId)
    },
    [syncStatus.queuedItems],
  )

  const applyQueueOrder = useCallback((provider: ProviderKey, reordered: string[]) => {
    setQueueOrderOverride((prev) => ({ ...prev, [provider]: reordered }))
    void reorderSourceSyncProviderQueue(provider, reordered).catch((reorderError) => {
      setError(
        reorderError instanceof Error ? reorderError.message : `Failed to reorder '${provider}' queue.`,
      )
    })
  }, [])

  // Move um job em fila uma posição para cima (-1) ou para baixo (+1).
  const moveQueued = useCallback(
    (provider: ProviderKey, jobKey: string, direction: -1 | 1) => {
      const ids = laneQueuedSyncIds(provider)
      const index = ids.indexOf(jobKey)
      const target = index + direction
      if (index < 0 || target < 0 || target >= ids.length) {
        return
      }
      const reordered = [...ids]
      ;[reordered[index], reordered[target]] = [reordered[target], reordered[index]]
      applyQueueOrder(provider, reordered)
    },
    [applyQueueOrder, laneQueuedSyncIds],
  )

  const reorderQueuedToIndex = useCallback(
    (provider: ProviderKey, jobKey: string, toIndex: number) => {
      const ids = laneQueuedSyncIds(provider)
      const from = ids.indexOf(jobKey)
      if (from < 0 || toIndex < 0 || toIndex >= ids.length || from === toIndex) {
        return
      }
      const next = [...ids]
      const [item] = next.splice(from, 1)
      next.splice(toIndex, 0, item)
      applyQueueOrder(provider, next)
    },
    [applyQueueOrder, laneQueuedSyncIds],
  )

  // Pointer-based reorder (HTML5 DnD is unreliable in Tauri/WebView2 on Windows).
  const dragSessionRef = useRef<{
    provider: ProviderKey
    jobKey: string
    pointerId: number
  } | null>(null)
  const dropTargetKeyRef = useRef<string | null>(null)

  const clearDragState = useCallback(() => {
    dragSessionRef.current = null
    dropTargetKeyRef.current = null
    setDragState(null)
    setDropTargetKey(null)
    document.body.classList.remove('queue-is-reordering')
  }, [])

  const resolveDropTarget = useCallback((clientX: number, clientY: number, provider: ProviderKey, sourceJobKey: string) => {
    const node = document.elementFromPoint(clientX, clientY)
    if (!(node instanceof Element)) {
      return null
    }
    const row = node.closest('[data-queue-job-key][data-queue-provider]') as HTMLElement | null
    if (!row) {
      return null
    }
    const targetProvider = row.dataset.queueProvider as ProviderKey | undefined
    const targetJobKey = row.dataset.queueJobKey
    if (!targetProvider || !targetJobKey || targetProvider !== provider || targetJobKey === sourceJobKey) {
      return null
    }
    return targetJobKey
  }, [])

  const endPointerReorder = useCallback(
    (clientX: number, clientY: number) => {
      const session = dragSessionRef.current
      if (!session) {
        clearDragState()
        return
      }
      const targetKey =
        dropTargetKeyRef.current ?? resolveDropTarget(clientX, clientY, session.provider, session.jobKey)
      if (targetKey) {
        const ids = laneQueuedSyncIds(session.provider)
        const toIndex = ids.indexOf(targetKey)
        if (toIndex >= 0) {
          reorderQueuedToIndex(session.provider, session.jobKey, toIndex)
        }
      }
      clearDragState()
    },
    [clearDragState, laneQueuedSyncIds, reorderQueuedToIndex, resolveDropTarget],
  )

  const beginPointerReorder = useCallback(
    (event: ReactPointerEvent<HTMLButtonElement>, provider: ProviderKey, jobKey: string) => {
      if (event.button !== 0) {
        return
      }
      event.preventDefault()
      event.stopPropagation()
      const pointerId = event.pointerId
      dragSessionRef.current = { provider, jobKey, pointerId }
      dropTargetKeyRef.current = null
      setDragState({ provider, jobKey })
      setDropTargetKey(null)
      document.body.classList.add('queue-is-reordering')

      try {
        event.currentTarget.setPointerCapture(pointerId)
      } catch {
        // Capture is best-effort; document listeners still drive the session.
      }

      const onPointerMove = (moveEvent: PointerEvent) => {
        if (moveEvent.pointerId !== pointerId || !dragSessionRef.current) {
          return
        }
        const nextTarget = resolveDropTarget(
          moveEvent.clientX,
          moveEvent.clientY,
          dragSessionRef.current.provider,
          dragSessionRef.current.jobKey,
        )
        if (dropTargetKeyRef.current !== nextTarget) {
          dropTargetKeyRef.current = nextTarget
          setDropTargetKey(nextTarget)
        }
      }

      const onPointerUp = (upEvent: PointerEvent) => {
        if (upEvent.pointerId !== pointerId) {
          return
        }
        window.removeEventListener('pointermove', onPointerMove)
        window.removeEventListener('pointerup', onPointerUp)
        window.removeEventListener('pointercancel', onPointerUp)
        endPointerReorder(upEvent.clientX, upEvent.clientY)
      }

      window.addEventListener('pointermove', onPointerMove)
      window.addEventListener('pointerup', onPointerUp)
      window.addEventListener('pointercancel', onPointerUp)
    },
    [endPointerReorder, resolveDropTarget],
  )

  const renderLiveTask = (task: QueueLiveTask, position?: number) => {
    const isRunning = task.state === 'running'
    const isHeld = task.state === 'held'
    const jobKey = task.queueKey ?? task.sourceId
    const detailBits: string[] = []
    if (task.progressDetail) detailBits.push(task.progressDetail)
    if (task.filesProcessed !== undefined && task.filesTotal !== undefined) {
      detailBits.push(`files ${task.filesProcessed}/${task.filesTotal}`)
    }
    // Só jobs Sync em fila são reordenáveis (pointer drag + Alt+↑/↓).
    const reorderable = !isRunning && !isHeld && task.operation === 'Sync' && task.state === 'queued'
    const isDragging = dragState?.jobKey === jobKey && dragState.provider === task.provider
    const isDropTarget = dropTargetKey === jobKey && dragState !== null && dragState.jobKey !== jobKey
    const progressMeta = [
      task.progressLabel
        ?? (task.operation === 'Delete'
          ? task.state === 'queued'
            ? 'Queued for delete'
            : 'Deleting…'
          : 'Downloading'),
      ...detailBits,
      task.progressPercent !== undefined && !task.progressIndeterminate
        ? `${task.progressPercent}%`
        : null,
      task.state === 'running'
        ? `running ${elapsed(task.startedAt ?? task.queuedAt, now)}`
        : task.state === 'queued'
          ? `waiting ${elapsed(task.queuedAt, now)}`
          : null,
    ]
      .filter(Boolean)
      .join(' · ')

    const rowClassName = [
      'queue-task-row',
      isRunning ? 'queue-task-row-running' : 'queue-task-row-queued',
      isDragging ? 'is-dragging' : '',
      isDropTarget ? 'is-drop-target' : '',
    ]
      .filter(Boolean)
      .join(' ')

    return (
      <article
        className={rowClassName}
        key={task.key}
        role="listitem"
        data-queue-provider={reorderable ? task.provider : undefined}
        data-queue-job-key={reorderable ? jobKey : undefined}
      >
        {reorderable ? (
          <button
            type="button"
            className="queue-drag-handle"
            title="Drag to reorder · Alt+↑/↓"
            aria-label="Drag to reorder. Use Alt+ArrowUp or Alt+ArrowDown to move."
            onPointerDown={(event) => beginPointerReorder(event, task.provider, jobKey)}
            onKeyDown={(event) => {
              if (!event.altKey) {
                return
              }
              if (event.key === 'ArrowUp') {
                event.preventDefault()
                moveQueued(task.provider, jobKey, -1)
              } else if (event.key === 'ArrowDown') {
                event.preventDefault()
                moveQueued(task.provider, jobKey, 1)
              }
            }}
          >
            <span aria-hidden="true">⠿</span>
          </button>
        ) : (
          <span className="queue-drag-handle-spacer" aria-hidden="true" />
        )}
        <TaskAvatar handle={task.handle} provider={task.provider} imagePath={avatarsBySource[task.sourceId]} />
        <div className="queue-task-main">
          <div className="queue-task-headline">
            <strong title={task.handle}>{task.handle}</strong>
            {task.operation === 'Delete' ? (
              <span className="queue-tag queue-tag-delete">Delete{task.modeDetail ? ` · ${task.modeDetail}` : ''}</span>
            ) : null}
            {isHeld ? (
              <span className="queue-tag queue-tag-held">
                {task.progressLabel === 'Waiting for media move' ? 'Migration hold' : 'Account hold'}
              </span>
            ) : null}
            {!isRunning && position !== undefined ? (
              <span className="queue-tag queue-tag-position">{position === 1 ? 'Next' : `#${position}`}</span>
            ) : null}
          </div>
          {isRunning ? (
            <>
              <div
                className={`queue-status-progress-track ${task.progressIndeterminate ? 'indeterminate' : ''}`}
                aria-hidden={task.progressIndeterminate ? true : undefined}
              >
                <div
                  className="queue-status-progress-fill"
                  style={
                    task.progressIndeterminate || task.progressPercent === undefined
                      ? undefined
                      : { width: `${Math.max(0, Math.min(100, task.progressPercent))}%` }
                  }
                />
              </div>
              <small className="queue-task-meta queue-task-meta-running" title={progressMeta}>
                {progressMeta}
              </small>
            </>
          ) : (
            <small className="queue-task-meta" title={absoluteTimestamp(task.queuedAt)}>
              {isHeld
                ? `${task.progressLabel ?? 'On hold'}${task.holdUntil ? ` · retry after ${absoluteTimestamp(task.holdUntil)}` : ''}`
                : `queued ${relativeTime(task.queuedAt, now)}`}
              {isHeld && detailBits.length ? ` · ${detailBits.join(' · ')}` : ''}
            </small>
          )}
        </div>
        <div className="queue-task-actions">
          {task.cancelSourceId ? (
            <button
              className="ghost-button queue-icon-button"
              onClick={() => void handleCancelProfile(task.cancelSourceId!)}
              type="button"
              title="Cancel this job"
            >
              Cancel
            </button>
          ) : null}
        </div>
      </article>
    )
  }

  return (
    <WindowShell
      density="compact"
      titlebar={
        <WindowTitlebar
          title="Queue Status"
          trailing={
            <span className={`queue-global-state ${totals.failed > 0 ? 'has-failures' : ''}`}>{globalState}</span>
          }
        />
      }
    >
      <div className="queue-status-window-body">
      <div className="queue-status-action-strip">
        <button
          ref={maintenanceButtonRef}
          aria-controls="queue-maintenance-panel"
          aria-expanded={maintenanceOpen}
          className="ghost-button"
          onClick={() => setMaintenanceOpen((open) => !open)}
          type="button"
        >
          Maintenance{maintenanceActive ? ` · ${maintenanceRunning} active · ${maintenanceQueued} queued` : ''}
        </button>
        <button
          aria-label="Open realtime debugger"
          className="ghost-button"
          disabled={openingDebugger}
          onClick={() => void handleOpenDebugger()}
          type="button"
        >
          {openingDebugger ? 'Opening…' : 'Realtime debugger'}
        </button>
      </div>

      {maintenanceActive ? <section className="maintenance-activity" aria-label="Maintenance activity">
        <header className="maintenance-activity-header">
          <span className="eyebrow">Maintenance activity</span>
          <div className="maintenance-activity-actions">
            {maintenanceQueued > 0 ? <span className="pill">{maintenanceQueued} waiting</span> : null}
            {(migrationStatus?.runningCount ?? 0) + (migrationStatus?.queuedCount ?? 0) > 0 ? <button className="ghost-button queue-icon-button queue-icon-button-danger" disabled={cancellingMigrations} onClick={() => void handleCancelMigrations()} type="button">{cancellingMigrations ? 'Cancelling…' : 'Cancel migrations'}</button> : null}
          </div>
        </header>
        {migrationStatus?.runningItems.map((job) => (
          <article className="maintenance-job" key={job.jobId}>
            <div className="maintenance-job-heading"><span className="queue-tag">Migration</span><strong>{job.handle}</strong><span className="maintenance-percent">{job.progressPercent === undefined ? 'Working…' : `${job.progressPercent}%`}</span></div>
            <div
              aria-label={`${job.handle} media migration`}
              aria-valuemax={job.progressIndeterminate ? undefined : 100}
              aria-valuemin={job.progressIndeterminate ? undefined : 0}
              aria-valuenow={job.progressIndeterminate ? undefined : job.progressPercent}
              aria-valuetext={`${migrationStageLabel(job.progressStage)}${job.progressPercent === undefined ? '' : ` ${job.progressPercent}%`}`}
              className={`queue-status-progress-track ${job.progressIndeterminate ? 'indeterminate' : ''}`}
              role="progressbar"
            ><div className="queue-status-progress-fill" style={job.progressIndeterminate ? undefined : { width: `${job.progressPercent ?? 0}%` }} /></div>
            <div className="maintenance-metrics">
              <span><small>Stage</small><strong>{migrationStageLabel(job.progressStage)}</strong></span>
              <span><small>Files</small><strong>{job.filesProcessed.toLocaleString()} <i>of</i> {job.filesTotal.toLocaleString()}</strong></span>
              <span><small>Data moved</small><strong>{formatMigrationBytes(job.bytesProcessed)} <i>of</i> {formatMigrationBytes(job.bytesTotal)}</strong></span>
            </div>
            {job.currentFile ? <small className="maintenance-current-file" title={`${job.currentFile}\n${job.sourcePath} → ${job.targetPath}`}>Current file · {migrationFileName(job.currentFile)}</small> : null}
          </article>
        ))}
        {thumbnailStatus?.active ? <article className="maintenance-job">
          <div className="maintenance-job-heading"><span className="queue-tag">Thumbnails</span><strong>{thumbnailStatus.active.handle}</strong><span className="queue-data">{thumbnailStatus.active.filesProcessed}/{thumbnailStatus.active.filesTotal} files</span></div>
          <div aria-label={`${thumbnailStatus.active.handle} thumbnail generation`} aria-valuemax={100} aria-valuemin={0} aria-valuenow={thumbnailStatus.active.progressPercent ?? 0} className="queue-status-progress-track" role="progressbar"><div className="queue-status-progress-fill" style={{ width: `${thumbnailStatus.active.progressPercent ?? 0}%` }} /></div>
          <small title={thumbnailStatus.active.currentFile}>
            Generating missing thumbnails · {thumbnailStatus.active.generated} generated · {thumbnailStatus.active.skippedExisting} existing
            {(thumbnailStatus.active.invalidMedia ?? 0) > 0 ? ` · ${thumbnailStatus.active.invalidMedia} invalid` : ''}
            {thumbnailStatus.active.failed > 0 ? ` · ${thumbnailStatus.active.failed} failed` : ''}
          </small>
        </article> : null}
        {dedupeRunning || dedupeQueued ? <article className="maintenance-job">
          <div className="maintenance-job-heading"><span className="queue-tag">Media cleanup</span><strong>{dedupeStatus?.state === 'applying' ? 'Applying reviewed changes' : dedupeStatus?.stage === 'perceptual_scan' ? 'Comparing similar media' : 'Scanning library'}</strong><span className="queue-data">{dedupeStatus?.stage === 'perceptual_scan' ? `${dedupeStatus.perceptualSourcesProcessed}/${dedupeStatus.perceptualSourcesTotal} sources` : `${dedupeStatus?.filesProcessed.toLocaleString()}/${dedupeStatus?.filesTotal.toLocaleString()} files`}</span></div>
          <div aria-label="Media cleanup progress" aria-valuemax={100} aria-valuemin={0} aria-valuenow={dedupeStatus?.filesTotal ? Math.round(dedupeStatus.filesProcessed * 100 / dedupeStatus.filesTotal) : 0} className="queue-status-progress-track" role="progressbar"><div className="queue-status-progress-fill" style={{ width: `${dedupeStatus?.filesTotal ? Math.round(dedupeStatus.filesProcessed * 100 / dedupeStatus.filesTotal) : 0}%` }} /></div>
          <small title={dedupeStatus?.currentPath}>{dedupeStatus?.stage.replaceAll('_', ' ')}{dedupeStatus?.currentRoot ? ` · ${dedupeStatus.currentRoot}` : ''} · Review and cleanup controls are available in Workspace Health.</small>
          {dedupeStatus?.sourceJobs.find((job) => job.status === 'running') ? <small className="maintenance-current-file" title={dedupeStatus.sourceJobs.find((job) => job.status === 'running')?.currentPath}>Current source · {dedupeStatus.sourceJobs.find((job) => job.status === 'running')?.sourcePath}{dedupeStatus.perceptualSourcesFailed ? ` · ${dedupeStatus.perceptualSourcesFailed} failed` : ''}</small> : null}
        </article> : null}
      </section> : null}

      {maintenanceOpen ? <section className="maintenance-panel panel" id="queue-maintenance-panel" aria-label="Maintenance controls">
        <div className="maintenance-panel-heading">
          <div><h2>Generate thumbnails</h2><p>Create only missing video previews. Existing thumbnails remain untouched.</p></div>
          <button aria-label="Close maintenance" className="ghost-button queue-icon-button" onClick={() => { setMaintenanceOpen(false); maintenanceButtonRef.current?.focus() }} type="button">Close</button>
        </div>
        {maintenanceError ? <div className="maintenance-error" role="alert">{maintenanceError}</div> : null}
        <article className="maintenance-cleanup-link">
          <div><h2>Duplicate media</h2><p>Scan the library, consolidate exact copies with hardlinks, and review similar candidates.</p></div>
          <button className="ghost-button" onClick={() => void openWorkspaceHealthWindow()} type="button">Open Workspace Health</button>
        </article>
        <div className="thumbnail-queue-controls thumbnail-generation-controls">
          <label>
            <span>Scope</span>
            <select
              value={thumbnailScope}
              onChange={(event) => {
                setThumbnailScope(event.target.value as typeof thumbnailScope)
                setThumbnailScopeValue('')
              }}
            >
              <option value="profile">Profile</option>
              <option value="group">Group</option>
              <option value="provider">Provider</option>
              <option value="all">Entire library</option>
            </select>
          </label>
          {thumbnailScope === 'profile' ? (
            <label>
              <span>Profile</span>
              <select value={thumbnailScopeValue} onChange={(event) => setThumbnailScopeValue(event.target.value)}>
                <option value="">Select a profile…</option>
                {librarySources.map((source) => <option key={source.id} value={source.id}>{source.handle}</option>)}
              </select>
            </label>
          ) : null}
          {thumbnailScope === 'group' ? (
            <label>
              <span>Group</span>
              <select value={thumbnailScopeValue} onChange={(event) => setThumbnailScopeValue(event.target.value)}>
                <option value="">Select a group…</option>
                {libraryGroups.map((group) => <option key={group.id} value={group.id}>{group.name}</option>)}
              </select>
            </label>
          ) : null}
          {thumbnailScope === 'provider' ? (
            <label>
              <span>Provider</span>
              <select value={thumbnailScopeValue} onChange={(event) => setThumbnailScopeValue(event.target.value)}>
                <option value="">Select a provider…</option>
                {DEFAULT_PROVIDER_CATALOG.map((provider) => <option key={provider.key} value={provider.key}>{provider.displayName}</option>)}
              </select>
            </label>
          ) : null}
          <button
            className="primary-button"
            disabled={thumbnailTargetIds.length === 0 || queueingThumbnails}
            onClick={() => void handleQueueThumbnails()}
            type="button"
          >
            {queueingThumbnails ? 'Adding to queue…' : 'Generate missing thumbnails'}
          </button>
        </div>
        <div className="thumbnail-queue-state"><span className="queue-tag">Thumbnail queue</span><small>{thumbnailStatus?.queuedCount ? `${thumbnailStatus.queuedCount} profiles waiting` : 'Ready — no thumbnail jobs waiting.'}</small></div>
      </section> : null}

      <section className="queue-status-summary-strip" role="list" aria-label="Queue totals">
        <article
          className={`queue-status-summary-card${totals.running === 0 ? ' is-quiet' : ''}`}
          role="listitem"
        >
          <span>Running</span>
          <strong>{totals.running}</strong>
          {totals.running > 0 ? (
            <small>
              {activeProviderCount} provider lane{activeProviderCount === 1 ? '' : 's'}
              {maintenanceRunning > 0 ? ` · ${maintenanceRunning} maintenance` : ''}
            </small>
          ) : null}
        </article>
        <article
          className={`queue-status-summary-card${totals.queued === 0 ? ' is-quiet' : ''}`}
          role="listitem"
        >
          <span>Queued</span>
          <strong>{totals.queued}</strong>
          {totals.queued > 0 ? (
            <small>
              {queuedProviderCount} provider lane{queuedProviderCount === 1 ? '' : 's'}
              {maintenanceQueued > 0 ? ` · ${maintenanceQueued} maintenance` : ''}
            </small>
          ) : null}
        </article>
        <article
          className={`queue-status-summary-card${totals.completed === 0 ? ' is-quiet' : ''}`}
          role="listitem"
        >
          <span>Done</span>
          <strong>{totals.completed}</strong>
        </article>
        <article
          className={`queue-status-summary-card${totals.failed === 0 ? ' is-quiet' : ' is-attention'}`}
          role="listitem"
        >
          <span>Failed</span>
          <strong>{totals.failed}</strong>
          {totals.failed > 0 ? <small>Needs attention</small> : null}
        </article>
      </section>

      {error ? <div className="runtime-log-window-error">{error}</div> : null}

      <section className="queue-status-main">
        <div className="queue-lanes" role="list" aria-label="Provider lanes">
          {lanes.length === 0 ? (
            <div className="queue-lanes-empty">
              <p className="queue-lanes-empty-message">No active downloads. Queue a profile to get started.</p>
              {idleProviders.length > 0 ? (
                <div className="queue-idle-providers">
                  <span className="eyebrow">Idle</span>
                  {idleProviders.map((descriptor) => (
                    <span className={`queue-provider-pill provider-${descriptor.key} is-idle`} key={descriptor.key}>
                      {descriptor.displayName}
                    </span>
                  ))}
                </div>
              ) : null}
            </div>
          ) : (
            lanes.map((lane) => {
              const busy = busyProviders.has(lane.provider)
              const hasLive = lane.running.length > 0 || lane.queued.length > 0
              const expanded = expandedProviders.has(lane.provider)
              const visibleQueued = expanded ? lane.queued : lane.queued.slice(0, QUEUED_COLLAPSE_LIMIT)
              const hiddenQueued = lane.queued.length - visibleQueued.length
              return (
                <article className="queue-lane" key={lane.provider} role="listitem">
                  <header className="queue-lane-header">
                    <div className="queue-lane-identity">
                      <span className={`queue-provider-pill provider-${lane.provider}`}>{lane.displayName}</span>
                      {lane.paused ? <span className="queue-tag queue-tag-paused">Paused</span> : null}
                    </div>
                    <div className="queue-lane-counts">
                      <span title="Running"><b>{lane.running.length}</b> running</span>
                      <span title="Queued"><b>{lane.queued.length}</b> queued</span>
                      <span title="Completed this session"><b>{lane.completed}</b> done</span>
                      {lane.failed > 0 ? <span className="queue-count-failed" title="Failed"><b>{lane.failed}</b> failed</span> : null}
                    </div>
                    <div className="queue-lane-actions">
                      {hasLive || lane.paused ? (
                        <button
                          className="ghost-button queue-icon-button"
                          disabled={busy}
                          onClick={() => void handleTogglePause(lane.provider, lane.paused)}
                          type="button"
                        >
                          {lane.paused ? 'Resume' : 'Pause'}
                        </button>
                      ) : null}
                      {hasLive ? (
                        <button
                          className="ghost-button queue-icon-button queue-icon-button-danger"
                          disabled={busy}
                          onClick={() => void handleCancelProvider(lane.provider)}
                          type="button"
                        >
                          Cancel all
                        </button>
                      ) : null}
                    </div>
                  </header>

                  {lane.running.length === 0 && lane.queued.length === 0 ? (
                    <p className="queue-lane-idle">Idle — nothing running for this provider.</p>
                  ) : (
                    <div className="queue-lane-body">
                      {lane.running.length > 0 ? (
                        <div className="queue-task-list" role="list" aria-label={`${lane.displayName} running`}>
                          {lane.running.map((task) => renderLiveTask(task))}
                        </div>
                      ) : null}
                      {lane.queued.length > 0 ? (
                        <div className="queue-task-list queue-task-list-queued" role="list" aria-label={`${lane.displayName} queued`}>
                          {visibleQueued.map((task, index) => renderLiveTask(task, index + 1))}
                          {hiddenQueued > 0 || expanded ? (
                            <button className="queue-more-button" onClick={() => toggleExpanded(lane.provider)} type="button">
                              {expanded ? 'Show less' : `+${hiddenQueued} more queued`}
                            </button>
                          ) : null}
                        </div>
                      ) : null}
                    </div>
                  )}
                </article>
              )
            })
          )}

          {singleVideoStatus &&
          (singleVideoStatus.active ||
            singleVideoStatus.queuedItems.length > 0 ||
            singleVideoStatus.recentResults.length > 0) ? (
            <article className="queue-lane" role="listitem">
              <header className="queue-lane-header">
                <div className="queue-lane-identity">
                  <span className="queue-provider-pill">Single videos</span>
                </div>
                <div className="queue-lane-counts">
                  <span title="Running"><b>{singleVideoStatus.runningCount}</b> running</span>
                  <span title="Queued"><b>{singleVideoStatus.queuedCount}</b> queued</span>
                  <span title="Completed this session"><b>{singleVideoStatus.completedCount}</b> done</span>
                  {singleVideoStatus.failedCount > 0 ? (
                    <span className="queue-count-failed" title="Failed"><b>{singleVideoStatus.failedCount}</b> failed</span>
                  ) : null}
                </div>
              </header>
              {singleVideoStatus.active || singleVideoStatus.queuedItems.length > 0 ? (
                <div className="queue-lane-body">
                  <div className="queue-task-list" role="list" aria-label="Single video downloads">
                    {singleVideoStatus.active ? (
                      <article className="queue-task-row queue-task-row-running" role="listitem">
                        <span className="queue-drag-handle-spacer" aria-hidden="true" />
                        <span className="queue-task-avatar" aria-hidden="true" />
                        <div className="queue-task-main">
                          <div className="queue-task-headline">
                            <strong title={singleVideoStatus.active.url}>
                              {singleVideoStatus.active.provider ?? 'Single video'}
                            </strong>
                          </div>
                          <div className="queue-status-progress-track indeterminate">
                            <div className="queue-status-progress-fill" />
                          </div>
                          <small className="queue-task-meta queue-task-meta-running queue-task-url" title={singleVideoStatus.active.url}>
                            Downloading · {singleVideoStatus.active.url}
                          </small>
                        </div>
                      </article>
                    ) : null}
                    {singleVideoStatus.queuedItems.map((item, index) => (
                      <article className="queue-task-row queue-task-row-queued" key={item.id} role="listitem">
                        <span className="queue-drag-handle-spacer" aria-hidden="true" />
                        <span className="queue-task-avatar" aria-hidden="true" />
                        <div className="queue-task-main">
                          <div className="queue-task-headline">
                            <strong title={item.url}>{item.provider ?? 'Single video'}</strong>
                            <span className="queue-tag queue-tag-position">{index === 0 ? 'Next' : `#${index + 1}`}</span>
                          </div>
                          <small className="queue-task-meta queue-task-url" title={item.url}>{item.url}</small>
                        </div>
                      </article>
                    ))}
                  </div>
                </div>
              ) : (
                <p className="queue-lane-idle">Idle — no single video downloads running.</p>
              )}
            </article>
          ) : null}

          {lanes.length > 0 && idleProviders.length > 0 ? (
            <div className="queue-idle-providers">
              <span className="eyebrow">Idle</span>
              {idleProviders.map((descriptor) => (
                <span className={`queue-provider-pill provider-${descriptor.key} is-idle`} key={descriptor.key}>
                  {descriptor.displayName}
                </span>
              ))}
            </div>
          ) : null}
        </div>

        <aside className="queue-recent-panel">
          <div className="queue-status-section-header">
            <span className="eyebrow">Recent</span>
            <span className="pill">{recentTasks.length}</span>
          </div>
          {recentTasks.length === 0 ? (
            <p className="queue-lane-idle">Nothing finished yet this session.</p>
          ) : (
            <div className="queue-recent-list" role="list" aria-label="Recent results">
              {recentTasks.map((task) => (
                <article className={`queue-recent-item queue-recent-${task.status}`} key={task.key} role="listitem">
                  <TaskAvatar handle={task.handle} provider={task.provider} imagePath={avatarsBySource[task.sourceId]} />
                  <div className="queue-task-main">
                    <div className="queue-task-headline">
                      <strong title={task.handle}>{task.handle}</strong>
                      <span className={`queue-provider-pill provider-${task.provider} is-mini`}>{task.providerLabel}</span>
                      {task.operation === 'Delete' ? <span className="queue-tag queue-tag-delete">Delete</span> : null}
                      {task.operation === 'Single' ? <span className="queue-tag">Single</span> : null}
                      {task.operation === 'Migration' ? <span className="queue-tag">Migration</span> : null}
                      {task.operation === 'Thumbnail' ? <span className="queue-tag">Thumbnail</span> : null}
                      <span className={resultStatusClassName(task.status)}>{task.status}</span>
                    </div>
                    <small className="queue-task-meta" title={absoluteTimestamp(task.finishedAt)}>
                      {relativeTime(task.finishedAt, now)}
                    </small>
                    <p className="queue-recent-summary">{task.summary}</p>
                    {task.error ? <p className="queue-recent-error">{task.error}</p> : null}
                    {task.operation === 'Thumbnail' && (task.reviewItems?.length ?? 0) > 0 ? (
                      <div className="thumbnail-review-panel">
                        <p className="thumbnail-review-lead">
                          Manual check recommended
                          {task.invalidMedia ? ` · ${task.invalidMedia} invalid media` : ''}
                          {task.generationFailed ? ` · ${task.generationFailed} generation failure(s)` : ''}
                          . Remove only after confirming the online post is also broken.
                        </p>
                        <ul className="thumbnail-review-list">
                          {(task.reviewItems ?? []).map((item) => (
                            <li key={`${task.key}-${item.relativePath}`}>
                              <code title={item.absolutePath}>{item.fileName}</code>
                              <span className="thumbnail-review-kind">
                                {item.kind === 'invalid_media' ? 'invalid media' : 'generation failed'}
                              </span>
                              <small title={item.reason}>{item.reason}</small>
                            </li>
                          ))}
                        </ul>
                        <button
                          className="ghost-button profile-view-delete thumbnail-review-delete"
                          disabled={resolvingReviewKey === task.key}
                          onClick={() =>
                            void handleResolveThumbnailReview(
                              task.key,
                              task.sourceId,
                              task.reviewItems ?? [],
                            )
                          }
                          type="button"
                          title="Move listed files to the Recycle Bin and mark posts so they are not re-downloaded"
                        >
                          {resolvingReviewKey === task.key
                            ? 'Moving to Recycle Bin…'
                            : 'Move invalid media to Recycle Bin'}
                        </button>
                      </div>
                    ) : null}
                  </div>
                  {task.operation === 'Sync' && task.status === 'failed' ? (
                    <button
                      className="ghost-button queue-icon-button"
                      onClick={() => void handleRetry(task.sourceId)}
                      type="button"
                      title="Re-queue this profile"
                    >
                      Retry
                    </button>
                  ) : null}
                </article>
              ))}
            </div>
          )}
        </aside>
      </section>
      </div>
    </WindowShell>
  )
}

function formatMigrationBytes(value: number): string {
  if (value < 1024) return `${value} B`
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`
  if (value < 1024 * 1024 * 1024) return `${(value / (1024 * 1024)).toFixed(1)} MB`
  return `${(value / (1024 * 1024 * 1024)).toFixed(2)} GB`
}

function migrationFileName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path
}
