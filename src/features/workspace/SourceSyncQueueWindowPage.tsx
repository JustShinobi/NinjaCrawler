import { useCallback, useEffect, useMemo, useState } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import {
  cancelSourceSyncProfile,
  cancelSourceSyncProvider,
  loadSourceDeleteQueueStatus,
  loadSourceSyncQueueStatus,
  loadWorkspaceSnapshot,
  openConnectorDebugWindow,
  pauseSourceSyncProvider,
  reorderSourceSyncProviderQueue,
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
  SingleVideoQueueRecentResult,
  SingleVideoQueueStatus,
} from '../../domain/models'

type QueueOperation = 'Sync' | 'Delete' | 'Single'

const QUEUED_COLLAPSE_LIMIT = 6

interface QueueLiveTask {
  key: string
  sourceId: string
  provider: ProviderKey
  providerLabel: string
  handle: string
  operation: QueueOperation
  modeDetail?: string
  state: 'queued' | 'running'
  queuedAt: string
  startedAt?: string
  progressPercent?: number
  progressLabel?: string
  progressDetail?: string
  progressIndeterminate?: boolean
  filesProcessed?: number
  filesTotal?: number
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
  status: 'succeeded' | 'failed' | 'skipped'
  summary: string
  finishedAt: string
  error?: string
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

function resultStatusClassName(status: 'succeeded' | 'failed' | 'skipped'): string {
  switch (status) {
    case 'failed':
      return 'status status-failed'
    case 'skipped':
      return 'status status-skipped'
    default:
      return 'status status-succeeded'
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
  return {
    key: `sync-${item.state}-${item.sourceId}`,
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
  const [now, setNow] = useState(() => Date.now())
  const [error, setError] = useState<string>()
  const [singleVideoStatus, setSingleVideoStatus] = useState<SingleVideoQueueStatus | undefined>()
  const [openingDebugger, setOpeningDebugger] = useState(false)

  const refreshQueueStatus = useCallback(async (silent = false) => {
    try {
      const [nextSyncStatus, nextDeleteStatus] = await Promise.all([
        loadSourceSyncQueueStatus(),
        loadSourceDeleteQueueStatus(),
      ])
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
    const timer = window.setInterval(() => void refreshQueueStatus(true), 1200)
    return () => window.clearInterval(timer)
  }, [refreshQueueStatus])

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
      ].sort((left, right) => Date.parse(right.finishedAt) - Date.parse(left.finishedAt)),
    [deleteStatus.recentResults, syncStatus.recentResults, singleVideoStatus?.recentResults],
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
        let queued = queuedTasks
          .filter((task) => task.provider === provider)
          .sort((a, b) => Date.parse(a.queuedAt) - Date.parse(b.queuedAt))
        const override = queueOrderOverride[provider]
        if (override) {
          const rank = new Map(override.map((id, index) => [id, index]))
          queued = [...queued].sort(
            (a, b) => (rank.get(a.sourceId) ?? Infinity) - (rank.get(b.sourceId) ?? Infinity),
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
          .map((item) => item.sourceId)
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
      queued: syncStatus.queuedCount + deleteStatus.queuedCount,
      running: syncStatus.runningCount + deleteStatus.runningCount,
      completed: syncStatus.completedCount + deleteStatus.completedCount,
      failed: syncStatus.failedCount + deleteStatus.failedCount,
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
    ],
  )

  const activeProviderCount = lanes.filter((lane) => lane.running.length > 0).length
  const queuedProviderCount = lanes.filter((lane) => lane.queued.length > 0).length

  // Lista de source ids (apenas Sync em fila) de uma raia, na ordem exibida.
  const laneQueuedSyncIds = (provider: ProviderKey): string[] => {
    const lane = lanes.find((entry) => entry.provider === provider)
    return lane ? lane.queued.filter((task) => task.operation === 'Sync').map((task) => task.sourceId) : []
  }

  // Move um job em fila uma posição para cima (-1) ou para baixo (+1), trocando
  // com o vizinho. Atualiza otimista e persiste a nova ordem no backend.
  const moveQueued = (provider: ProviderKey, sourceId: string, direction: -1 | 1) => {
    const ids = laneQueuedSyncIds(provider)
    const index = ids.indexOf(sourceId)
    const target = index + direction
    if (index < 0 || target < 0 || target >= ids.length) {
      return
    }
    const reordered = [...ids]
    ;[reordered[index], reordered[target]] = [reordered[target], reordered[index]]
    setQueueOrderOverride((prev) => ({ ...prev, [provider]: reordered }))
    void reorderSourceSyncProviderQueue(provider, reordered).catch((reorderError) => {
      setError(
        reorderError instanceof Error ? reorderError.message : `Failed to reorder '${provider}' queue.`,
      )
    })
  }

  const renderLiveTask = (task: QueueLiveTask, position?: number) => {
    const isRunning = task.state === 'running'
    const detailBits: string[] = []
    if (task.progressDetail) detailBits.push(task.progressDetail)
    if (task.filesProcessed !== undefined && task.filesTotal !== undefined) {
      detailBits.push(`files ${task.filesProcessed}/${task.filesTotal}`)
    }
    // Só os jobs de Sync em fila são reordenáveis (botões ▲/▼).
    const reorderable = !isRunning && task.operation === 'Sync'
    let canMoveUp = false
    let canMoveDown = false
    if (reorderable) {
      const ids = laneQueuedSyncIds(task.provider)
      const index = ids.indexOf(task.sourceId)
      canMoveUp = index > 0
      canMoveDown = index >= 0 && index < ids.length - 1
    }
    const rowClassName = ['queue-task-row', isRunning ? 'queue-task-row-running' : '']
      .filter(Boolean)
      .join(' ')
    return (
      <article className={rowClassName} key={task.key} role="listitem">
        <TaskAvatar handle={task.handle} provider={task.provider} imagePath={avatarsBySource[task.sourceId]} />
        <div className="queue-task-main">
          <div className="queue-task-headline">
            <strong title={task.handle}>{task.handle}</strong>
            {task.operation === 'Delete' ? (
              <span className="queue-tag queue-tag-delete">Delete{task.modeDetail ? ` · ${task.modeDetail}` : ''}</span>
            ) : null}
            {!isRunning && position !== undefined ? (
              <span className="queue-tag queue-tag-position">{position === 1 ? 'Next' : `#${position}`}</span>
            ) : null}
          </div>
          {isRunning ? (
            <>
              <div className={`queue-status-progress-track ${task.progressIndeterminate ? 'indeterminate' : ''}`}>
                <div
                  className="queue-status-progress-fill"
                  style={
                    task.progressIndeterminate || task.progressPercent === undefined
                      ? undefined
                      : { width: `${Math.max(0, Math.min(100, task.progressPercent))}%` }
                  }
                />
              </div>
              <small className="queue-task-meta">
                {task.progressLabel ?? (task.operation === 'Delete' ? 'Processing' : 'Downloading')}
                {detailBits.length ? ` · ${detailBits.join(' · ')}` : ''}
                {task.progressPercent !== undefined && !task.progressIndeterminate ? ` · ${task.progressPercent}%` : ''}
                {` · running ${elapsed(task.startedAt ?? task.queuedAt, now)}`}
              </small>
            </>
          ) : (
            <small className="queue-task-meta" title={absoluteTimestamp(task.queuedAt)}>
              queued {relativeTime(task.queuedAt, now)}
            </small>
          )}
        </div>
        <div className="queue-task-actions">
          {reorderable ? (
            <span className="queue-reorder-buttons">
              <button
                className="ghost-button queue-reorder-button"
                disabled={!canMoveUp}
                onClick={() => moveQueued(task.provider, task.sourceId, -1)}
                type="button"
                title="Move up"
                aria-label="Move up in queue"
              >
                ▲
              </button>
              <button
                className="ghost-button queue-reorder-button"
                disabled={!canMoveDown}
                onClick={() => moveQueued(task.provider, task.sourceId, 1)}
                type="button"
                title="Move down"
                aria-label="Move down in queue"
              >
                ▼
              </button>
            </span>
          ) : null}
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
    <div className="queue-status-window-shell">
      <section className="queue-status-debug-toolbar panel">
        <div>
          <span className="eyebrow">Backend diagnostics</span>
          <p>Follow queue events, sync stages, connector activity, warnings, and errors live.</p>
        </div>
        <button
          className="primary-button"
          disabled={openingDebugger}
          onClick={() => void handleOpenDebugger()}
          type="button"
        >
          {openingDebugger ? 'Opening debugger…' : 'Open realtime debugger'}
        </button>
      </section>

      <section className="queue-status-summary-strip" role="list" aria-label="Queue totals">
        <article className="queue-status-summary-card" role="listitem">
          <span>Running</span>
          <strong>{totals.running}</strong>
          <small>{totals.running > 0 ? `across ${activeProviderCount} provider${activeProviderCount === 1 ? '' : 's'}` : 'No active work'}</small>
        </article>
        <article className="queue-status-summary-card" role="listitem">
          <span>Queued</span>
          <strong>{totals.queued}</strong>
          <small>{totals.queued > 0 ? `in ${queuedProviderCount} provider lane${queuedProviderCount === 1 ? '' : 's'}` : 'Queue is clear'}</small>
        </article>
        <article className="queue-status-summary-card" role="listitem">
          <span>Done</span>
          <strong>{totals.completed}</strong>
          <small>Completed this session</small>
        </article>
        <article className="queue-status-summary-card" role="listitem">
          <span>Failed</span>
          <strong>{totals.failed}</strong>
          <small>{totals.failed > 0 ? 'Needs attention' : 'No failures'}</small>
        </article>
      </section>

      {error ? <div className="runtime-log-window-error">{error}</div> : null}

      <section className="queue-status-main">
        <div className="queue-lanes" role="list" aria-label="Provider lanes">
          {lanes.length === 0 ? (
            <div className="runtime-log-window-empty">No active downloads. Queue a profile to get started.</div>
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
                        <div className="queue-task-main">
                          <div className="queue-task-headline">
                            <strong title={singleVideoStatus.active.url}>
                              {singleVideoStatus.active.provider ?? 'Single video'}
                            </strong>
                          </div>
                          <div className="queue-status-progress-track indeterminate">
                            <div className="queue-status-progress-fill" />
                          </div>
                          <small className="queue-task-meta queue-task-url" title={singleVideoStatus.active.url}>
                            Downloading · {singleVideoStatus.active.url}
                          </small>
                        </div>
                      </article>
                    ) : null}
                    {singleVideoStatus.queuedItems.map((item, index) => (
                      <article className="queue-task-row" key={item.id} role="listitem">
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
                      <span className={resultStatusClassName(task.status)}>{task.status}</span>
                    </div>
                    <small className="queue-task-meta" title={absoluteTimestamp(task.finishedAt)}>
                      {relativeTime(task.finishedAt, now)}
                    </small>
                    <p className="queue-recent-summary">{task.summary}</p>
                    {task.error ? <p className="queue-recent-error">{task.error}</p> : null}
                  </div>
                  {task.operation === 'Sync' ? (
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
  )
}
