import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  enqueueImportBackfill,
  enqueueImportPreview,
  enqueueImportRun,
  listImportMethods,
  listImportProviders,
  listImportRoots,
  loadImportQueueStatus,
  loadWorkspaceSnapshot,
  pickImportRootFolder,
  subscribeToDesktopRuntimeEvents,
  upsertAppSetting,
} from '../../bridge/desktop'
import type {
  ImportMethodDescriptor,
  ImportPreview,
  ImportPreviewProfile,
  ImportProviderDescriptor,
  ImportQueueJob,
  ImportQueueRecentResult,
  ImportQueueStatus,
  ImportRootDescriptor,
  ImportResolutionAction,
  ImportRunResult,
  ProviderAccount,
  ProviderKey,
  WorkspaceSnapshot,
} from '../../domain/models'
import { WindowShell } from '../brand/WindowShell'
import { WindowTitlebar } from '../brand/WindowTitlebar'

interface ImportResolutionDraft {
  action: ImportResolutionAction
  accountId?: string
}

const IMPORT_MANUAL_ROOTS_SETTING_CATEGORY = 'imports'

function normalizeManualRoot(value: string): string {
  return value.trim().replace(/[\\/]+$/, '')
}

function sameManualRoot(left: string, right: string): boolean {
  return normalizeManualRoot(left).localeCompare(normalizeManualRoot(right), undefined, { sensitivity: 'accent' }) === 0
}

function importManualRootsSettingKey(importerId: string): string {
  return `imports.${importerId}.manualRoots`
}

function importDisabledRootsSettingKey(importerId: string): string {
  return `imports.${importerId}.disabledRoots`
}

function parsePersistedManualRoots(value: string | undefined): string[] {
  if (!value) {
    return []
  }

  try {
    const parsed = JSON.parse(value)
    if (!Array.isArray(parsed)) {
      return []
    }

    const roots: string[] = []
    for (const entry of parsed) {
      if (typeof entry !== 'string') {
        continue
      }

      const normalized = normalizeManualRoot(entry)
      if (!normalized || roots.some((current) => sameManualRoot(current, normalized))) {
        continue
      }

      roots.push(normalized)
    }

    return roots
  } catch {
    return []
  }
}

function loadPersistedManualRoots(snapshot: WorkspaceSnapshot | undefined, importerId: string | undefined): string[] {
  if (!snapshot || !importerId) {
    return []
  }

  return parsePersistedManualRoots(
    snapshot.appSettings.find((setting) => setting.key === importManualRootsSettingKey(importerId))?.value,
  )
}

function loadPersistedDisabledRoots(snapshot: WorkspaceSnapshot | undefined, importerId: string | undefined): string[] {
  if (!snapshot || !importerId) {
    return []
  }

  return parsePersistedManualRoots(
    snapshot.appSettings.find((setting) => setting.key === importDisabledRootsSettingKey(importerId))?.value,
  )
}

function createEmptyImportQueueStatus(): ImportQueueStatus {
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

function buildDefaultResolutions(preview: ImportPreview): Record<string, ImportResolutionDraft> {
  return Object.fromEntries(
    preview.profiles.map((profile) => [
      profile.profileRoot,
      {
        action:
          profile.importState === 'already_imported' && !preview.forceReimport
            ? 'skip'
            : 'import',
        accountId: profile.accountId,
      },
    ]),
  )
}

function stateLabel(profile: ImportPreviewProfile): string {
  switch (profile.importState) {
    case 'already_imported':
      return 'Imported'
    case 'needs_account_link':
      return 'Link account'
    case 'duplicate_conflict':
      return 'Duplicate'
    case 'no_media':
      return 'No media'
    default:
      return 'Ready'
  }
}

function profileSummaryLabel(profile: ImportPreviewProfile): string {
  if (profile.sourceId) {
    return `Existing source · ${profile.sourceDisplayName ?? profile.sourceHandle ?? profile.handle}`
  }

  return 'New source'
}

function renderCounts(profile: ImportPreviewProfile): string {
  return `${profile.fileCount} on disk · ${profile.newFileCount} new · ${profile.alreadyCatalogedCount} already present`
}

function queueJobLabel(job: ImportQueueJob): string {
  const jobLabel = job.jobKind === 'import'
    ? 'Import'
    : job.jobKind === 'backfill'
      ? 'Backfill'
      : 'Dry-run'
  return `${job.methodLabel} · ${jobLabel}`
}

function queueResultLabel(result: ImportQueueRecentResult): string {
  const jobLabel = result.jobKind === 'import'
    ? 'Import'
    : result.jobKind === 'backfill'
      ? 'Backfill'
      : 'Dry-run'
  return `${result.methodLabel} · ${jobLabel}`
}

function isManagedImportRoot(root: ImportRootDescriptor): boolean {
  return root.source !== 'manual' && !root.removable
}

function queueHeadline(status: ImportQueueStatus): string {
  if (status.runningCount > 0) {
    return `${status.runningCount} running`
  }

  if (status.queuedCount > 0) {
    return `${status.queuedCount} queued`
  }

  if (status.recentResults.length > 0) {
    return 'Completed'
  }

  return 'Idle'
}

function queueActivityLabel(job?: ImportQueueJob): string {
  if (!job) {
    return 'No job running.'
  }

  return job.progressLabel ?? (job.jobKind === 'import' ? 'Applying import' : 'Scanning folders')
}

function queueDetailLabel(status: ImportQueueStatus, job?: ImportQueueJob): string {
  if (job?.progressDetail) {
    return job.progressDetail
  }

  if (status.runningCount > 0) {
    return 'Worker is processing the current import job.'
  }

  if (status.queuedCount > 0) {
    return 'Job accepted. Waiting for the worker to pick it up and begin scanning.'
  }

  return 'Waiting for the next queue event.'
}

function formatTimestamp(value?: string): string {
  if (!value) {
    return '—'
  }

  const parsed = new Date(value)
  if (Number.isNaN(parsed.getTime())) {
    return value
  }

  return parsed.toLocaleString()
}

export function ImportWindowPage() {
  const [loading, setLoading] = useState(true)
  const [browsePending, setBrowsePending] = useState(false)
  const [error, setError] = useState<string>()
  const [providers, setProviders] = useState<ImportProviderDescriptor[]>([])
  const [methodsByProvider, setMethodsByProvider] = useState<Record<string, ImportMethodDescriptor[]>>({})
  const [selectedProvider, setSelectedProvider] = useState<ProviderKey>('instagram')
  const [snapshot, setSnapshot] = useState<WorkspaceSnapshot>()
  const [preview, setPreview] = useState<ImportPreview>()
  const [result, setResult] = useState<ImportRunResult>()
  const [queueStatus, setQueueStatus] = useState<ImportQueueStatus>(() => createEmptyImportQueueStatus())
  const [forceReimport, setForceReimport] = useState(false)
  const [resolutions, setResolutions] = useState<Record<string, ImportResolutionDraft>>({})
  const [manualRootInput, setManualRootInput] = useState('')
  const [manualRoots, setManualRoots] = useState<string[]>([])
  const [disabledRoots, setDisabledRoots] = useState<string[]>([])
  const [effectiveRoots, setEffectiveRoots] = useState<ImportRootDescriptor[]>([])
  const [profileFilter, setProfileFilter] = useState<'all' | 'attention' | 'no_media' | 'needs_account_link' | 'duplicate_conflict' | 'already_imported'>('all')

  useEffect(() => {
    let cancelled = false

    async function loadContext() {
      setLoading(true)
      setError(undefined)

      try {
        const [providerList, workspaceSnapshot, importStatus] = await Promise.all([
          listImportProviders(),
          loadWorkspaceSnapshot(),
          loadImportQueueStatus(),
        ])
        const methods = await Promise.all(
          providerList.map(async (provider) => [provider.key, await listImportMethods(provider.key)] as const),
        )

        if (cancelled) {
          return
        }

        setProviders(providerList)
        setMethodsByProvider(Object.fromEntries(methods))
        setSelectedProvider((current) => providerList.some((provider) => provider.key === current) ? current : (providerList[0]?.key ?? 'instagram'))
        setSnapshot(workspaceSnapshot)
        setQueueStatus(importStatus)
        if (importStatus.latestPreview) {
          setPreview(importStatus.latestPreview)
          setResolutions(buildDefaultResolutions(importStatus.latestPreview))
        }
        if (importStatus.latestRunResult) {
          setResult(importStatus.latestRunResult)
        }
      } catch (loadError) {
        if (!cancelled) {
          setError(loadError instanceof Error ? loadError.message : String(loadError))
        }
      } finally {
        if (!cancelled) {
          setLoading(false)
        }
      }
    }

    void loadContext()

    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    let disposed = false
    let unsubscribe: (() => void) | undefined

    void subscribeToDesktopRuntimeEvents({
      onImportQueueChanged: (status) => {
        if (disposed) {
          return
        }

        setQueueStatus(status)
        if (status.latestPreview) {
          setPreview(status.latestPreview)
          setResolutions((current) => {
            const defaults = buildDefaultResolutions(status.latestPreview!)
            const merged: Record<string, ImportResolutionDraft> = {}

            for (const [profileRoot, draft] of Object.entries(defaults)) {
              merged[profileRoot] = {
                action: current[profileRoot]?.action ?? draft.action,
                accountId: current[profileRoot]?.accountId ?? draft.accountId,
              }
            }

            return merged
          })
        }
        if (status.latestRunResult) {
          setResult(status.latestRunResult)
        }
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
    function handleVisibilityChange() {
      if (document.visibilityState !== 'visible') {
        return
      }

      void loadImportQueueStatus()
        .then((status) => {
          setQueueStatus(status)
          if (status.latestPreview) {
            setPreview(status.latestPreview)
            setResolutions((current) => {
              const defaults = buildDefaultResolutions(status.latestPreview!)
              const merged: Record<string, ImportResolutionDraft> = {}

              for (const [profileRoot, draft] of Object.entries(defaults)) {
                merged[profileRoot] = {
                  action: current[profileRoot]?.action ?? draft.action,
                  accountId: current[profileRoot]?.accountId ?? draft.accountId,
                }
              }

              return merged
            })
          }
          if (status.latestRunResult) {
            setResult(status.latestRunResult)
          }
        })
        .catch(() => undefined)
    }

    document.addEventListener('visibilitychange', handleVisibilityChange)
    window.addEventListener('focus', handleVisibilityChange)

    return () => {
      document.removeEventListener('visibilitychange', handleVisibilityChange)
      window.removeEventListener('focus', handleVisibilityChange)
    }
  }, [])

  useEffect(() => {
    if (queueStatus.runningCount === 0 && queueStatus.queuedCount === 0) {
      return undefined
    }

    const intervalId = window.setInterval(() => {
      void loadImportQueueStatus()
        .then((status) => {
          setQueueStatus(status)
          if (status.latestPreview) {
            setPreview(status.latestPreview)
            setResolutions((current) => {
              const defaults = buildDefaultResolutions(status.latestPreview!)
              const merged: Record<string, ImportResolutionDraft> = {}

              for (const [profileRoot, draft] of Object.entries(defaults)) {
                merged[profileRoot] = {
                  action: current[profileRoot]?.action ?? draft.action,
                  accountId: current[profileRoot]?.accountId ?? draft.accountId,
                }
              }

              return merged
            })
          }
          if (status.latestRunResult) {
            setResult(status.latestRunResult)
          }
        })
        .catch(() => undefined)
    }, 1000)

    return () => window.clearInterval(intervalId)
  }, [queueStatus.queuedCount, queueStatus.runningCount])

  const activeMethods = methodsByProvider[selectedProvider] ?? []
  const activeMethod = activeMethods[0]
  const providerAccounts = useMemo(
    () => (snapshot?.accounts ?? []).filter((account) => account.provider === selectedProvider),
    [selectedProvider, snapshot],
  )
  const actionableProfiles = preview?.profiles.filter((profile) => resolutions[profile.profileRoot]?.action !== 'skip') ?? []
  const pendingProfiles = (preview?.profiles ?? []).filter(
    (profile) =>
      (profile.importState !== 'ready' && profile.importState !== 'already_imported')
      || profile.problems.length > 0,
  )
  const filteredProfiles = useMemo(() => {
    const all = preview?.profiles ?? []
    switch (profileFilter) {
      case 'attention':
        return all.filter(
          (p) => (p.importState !== 'ready' && p.importState !== 'already_imported') || p.problems.length > 0,
        )
      case 'no_media':
        return all.filter((p) => p.importState === 'no_media')
      case 'needs_account_link':
        return all.filter((p) => p.importState === 'needs_account_link')
      case 'duplicate_conflict':
        return all.filter((p) => p.importState === 'duplicate_conflict')
      case 'already_imported':
        return all.filter((p) => p.importState === 'already_imported')
      default:
        return all
    }
  }, [preview?.profiles, profileFilter])
  const blockingProfiles = actionableProfiles.filter((profile) => {
    if (profile.importState === 'needs_account_link') {
      return !resolutions[profile.profileRoot]?.accountId
    }

    return profile.importState === 'duplicate_conflict' || profile.importState === 'no_media'
  })
  const queueBusy = queueStatus.runningCount > 0 || queueStatus.queuedCount > 0
  const canRunImport = Boolean(
    activeMethod
    && preview
    && actionableProfiles.length > 0
    && blockingProfiles.length === 0
    && !queueBusy,
  )
  const reviewSummaryPills = preview
    ? [
        { label: 'Profiles', value: preview.summary.detectedProfiles },
        { label: 'Ready', value: preview.summary.readyProfiles },
        { label: 'Attention', value: preview.summary.blockedProfiles },
        { label: 'New media', value: preview.summary.importableFiles },
      ]
    : []
  const activeQueueJob = queueStatus.runningItems[0]
  const queueHeadlineLabel = queueHeadline(queueStatus)
  const queuedJobCount = queueStatus.queuedItems.length
  const recentQueueResults = queueStatus.recentResults.slice(0, 4)
  const showProviderSidebar = providers.length > 1
  const persistedManualRoots = useMemo(
    () => loadPersistedManualRoots(snapshot, activeMethod?.importerId),
    [activeMethod?.importerId, snapshot],
  )
  const persistedDisabledRoots = useMemo(
    () => loadPersistedDisabledRoots(snapshot, activeMethod?.importerId),
    [activeMethod?.importerId, snapshot],
  )
  const previewJobRunning = queueStatus.runningItems.some((job) => job.jobKind === 'preview')
  const previewJobQueued = queueStatus.queuedItems.some((job) => job.jobKind === 'preview')
  const importJobRunning = queueStatus.runningItems.some((job) => job.jobKind === 'import')
  const importJobQueued = queueStatus.queuedItems.some((job) => job.jobKind === 'import')
  const backfillJobRunning = queueStatus.runningItems.some((job) => job.jobKind === 'backfill')
  const backfillJobQueued = queueStatus.queuedItems.some((job) => job.jobKind === 'backfill')
  const scanButtonLabel = previewJobRunning ? 'Scanning...' : previewJobQueued ? 'Scan queued' : 'Run scan'
  const importButtonLabel = importJobRunning ? 'Importing...' : importJobQueued ? 'Import queued' : 'Confirm import'
  const backfillButtonLabel = backfillJobRunning ? 'Backfilling...' : backfillJobQueued ? 'Backfill queued' : 'Run naming backfill'
  const canRunBackfill = Boolean(activeMethod && selectedProvider === 'instagram' && !queueBusy)
  const workspaceStatusLabel = preview
    ? `${actionableProfiles.length} selected`
    : queueBusy
      ? queueHeadlineLabel
      : 'Ready to scan'
  const headerFacts = [
    { label: 'Accounts', value: providerAccounts.length },
    { label: 'Manual roots', value: manualRoots.length },
    { label: 'Detected roots', value: effectiveRoots.length },
  ]

  useEffect(() => {
    setManualRoots(persistedManualRoots)
  }, [persistedManualRoots])

  useEffect(() => {
    setDisabledRoots(persistedDisabledRoots)
  }, [persistedDisabledRoots])

  async function handleRunPreview() {
    if (!activeMethod) {
      return
    }

    setError(undefined)

    try {
      const status = await enqueueImportPreview(activeMethod.importerId, {
        forceReimport,
        manualRoots,
        disabledRoots,
      })
      setQueueStatus(status)
    } catch (previewError) {
      setError(previewError instanceof Error ? previewError.message : String(previewError))
    }
  }

  async function handleRunImport() {
    if (!activeMethod || !preview || !canRunImport) {
      return
    }

    setError(undefined)

    try {
      const status = await enqueueImportRun(activeMethod.importerId, {
        forceReimport,
        manualRoots,
        disabledRoots,
        resolutions: preview.profiles.map((profile) => ({
          profileRoot: profile.profileRoot,
          action: resolutions[profile.profileRoot]?.action ?? 'import',
          accountId: resolutions[profile.profileRoot]?.accountId,
        })),
      })
      setQueueStatus(status)
    } catch (runError) {
      setError(runError instanceof Error ? runError.message : String(runError))
    }
  }

  async function handleRunBackfill() {
    if (!activeMethod || !canRunBackfill) {
      return
    }

    setError(undefined)

    try {
      const status = await enqueueImportBackfill(activeMethod.importerId)
      setQueueStatus(status)
    } catch (runError) {
      setError(runError instanceof Error ? runError.message : String(runError))
    }
  }

  function updateResolution(profileRoot: string, nextDraft: Partial<ImportResolutionDraft>) {
    setResolutions((current) => ({
      ...current,
      [profileRoot]: {
        action: current[profileRoot]?.action ?? 'import',
        accountId: current[profileRoot]?.accountId,
        ...nextDraft,
      },
    }))
  }

  const persistManualRoots = useCallback(async (nextRoots: string[], previousRoots: string[]) => {
    if (!activeMethod) {
      return
    }

    try {
      const nextSnapshot = await upsertAppSetting({
        key: importManualRootsSettingKey(activeMethod.importerId),
        value: JSON.stringify(nextRoots),
        category: IMPORT_MANUAL_ROOTS_SETTING_CATEGORY,
        description: 'Persisted manual scan roots for external import review.',
        mutable: true,
      })
      setSnapshot(nextSnapshot)
    } catch (persistError) {
      setManualRoots(previousRoots)
      setError(persistError instanceof Error ? persistError.message : String(persistError))
    }
  }, [activeMethod])

  const persistDisabledRoots = useCallback(async (nextRoots: string[], previousRoots: string[]) => {
    if (!activeMethod) {
      return
    }

    try {
      const nextSnapshot = await upsertAppSetting({
        key: importDisabledRootsSettingKey(activeMethod.importerId),
        value: JSON.stringify(nextRoots),
        category: IMPORT_MANUAL_ROOTS_SETTING_CATEGORY,
        description: 'Disabled scan roots for external import review.',
        mutable: true,
      })
      setSnapshot(nextSnapshot)
    } catch (persistError) {
      setDisabledRoots(previousRoots)
      setError(persistError instanceof Error ? persistError.message : String(persistError))
    }
  }, [activeMethod])

  useEffect(() => {
    let cancelled = false

    async function loadRoots() {
      if (!activeMethod) {
        setEffectiveRoots([])
        return
      }

      try {
        const roots = await listImportRoots(activeMethod.importerId, manualRoots, disabledRoots)
        if (!cancelled) {
          setEffectiveRoots(roots)
        }
      } catch (loadError) {
        if (!cancelled) {
          setError(loadError instanceof Error ? loadError.message : String(loadError))
        }
      }
    }

    void loadRoots()

    return () => {
      cancelled = true
    }
  }, [activeMethod, disabledRoots, manualRoots])

  function addManualRoot(value: string) {
    const normalized = normalizeManualRoot(value)
    if (!normalized) {
      return
    }

    if (disabledRoots.some((entry) => sameManualRoot(entry, normalized))) {
      const nextDisabledRoots = disabledRoots.filter((entry) => !sameManualRoot(entry, normalized))
      setError(undefined)
      setDisabledRoots(nextDisabledRoots)
      setManualRootInput('')
      void persistDisabledRoots(nextDisabledRoots, disabledRoots)
      return
    }

    if (effectiveRoots.some((root) => isManagedImportRoot(root) && sameManualRoot(root.path, normalized))) {
      setManualRootInput('')
      setError('This path is already active in import roots. Remove it from the list first if you want it excluded.')
      return
    }

    const nextRoots = manualRoots.some((entry) => sameManualRoot(entry, normalized))
      ? manualRoots
      : [...manualRoots, normalized]
    if (nextRoots === manualRoots) {
      return
    }

    setError(undefined)
    setManualRoots(nextRoots)
    setManualRootInput('')
    void persistManualRoots(nextRoots, manualRoots)
  }

  async function handleBrowseManualRoot() {
    setBrowsePending(true)
    setError(undefined)

    try {
      const picked = await pickImportRootFolder()
      if (picked) {
        addManualRoot(picked)
      }
    } catch (browseError) {
      setError(browseError instanceof Error ? browseError.message : String(browseError))
    } finally {
      setBrowsePending(false)
    }
  }

  function removeManualRoot(root: string) {
    const nextRoots = manualRoots.filter((entry) => !sameManualRoot(entry, root))
    if (nextRoots.length === manualRoots.length) {
      return
    }

    setError(undefined)
    setManualRoots(nextRoots)
    void persistManualRoots(nextRoots, manualRoots)
  }

  function removeManagedRoot(root: string) {
    const normalized = normalizeManualRoot(root)
    if (!normalized) {
      return
    }

    const nextRoots = disabledRoots.some((entry) => sameManualRoot(entry, normalized))
      ? disabledRoots
      : [...disabledRoots, normalized]
    if (nextRoots === disabledRoots) {
      return
    }

    setError(undefined)
    setDisabledRoots(nextRoots)
    void persistDisabledRoots(nextRoots, disabledRoots)
  }

  const activeProviderName =
    providers.find((provider) => provider.key === selectedProvider)?.displayName ?? 'Import'

  return (
    <WindowShell
      className="import-window-frame"
      contentClassName="import-window-content"
      density="compact"
      titlebar={
        <WindowTitlebar
          title="Import"
          trailing={
            <span className="window-titlebar-status-meta">
              {activeProviderName}
              {queueBusy ? ` · ${queueHeadlineLabel}` : ''}
            </span>
          }
        />
      }
    >
    <div className={`import-window-shell${showProviderSidebar ? '' : ' import-window-shell-compact'}`}>
      {showProviderSidebar ? (
        <aside className="panel settings-nav-panel import-window-sidebar">
          <div className="import-window-sidebar-header">
            <h2 className="import-window-sidebar-title">Providers</h2>
          </div>
          <div className="settings-category-list import-provider-nav" role="tablist" aria-label="Import providers">
            {providers.map((provider) => (
              <button
                aria-selected={provider.key === selectedProvider}
                className={`settings-category-button import-provider-nav-item${provider.key === selectedProvider ? ' settings-category-button-active import-provider-nav-item-active' : ''}`}
                key={provider.key}
                onClick={() => setSelectedProvider(provider.key)}
                role="tab"
                type="button"
              >
                <div className="settings-category-copy">
                  <span className="eyebrow">Provider</span>
                  <strong>{provider.displayName}</strong>
                </div>
                <div className="settings-category-meta">
                  <span className="pill">{(methodsByProvider[provider.key] ?? []).length}</span>
                </div>
              </button>
            ))}
          </div>
        </aside>
      ) : null}

      <main className="import-window-main">
        <section className="panel import-window-toolbar">
          <div className="import-window-toolbar-copy">
            <span className="eyebrow">{providers.find((provider) => provider.key === selectedProvider)?.displayName ?? 'Import'}</span>
            <h1>{activeMethod?.label ?? 'Import'}</h1>
            <p>Imports legacy media in place. Files are not moved.</p>
          </div>

          <div className="import-window-toolbar-summary" aria-label="Import method facts">
            {headerFacts.map((fact) => (
              <div className="import-window-toolbar-stat" key={fact.label}>
                <span>{fact.label}</span>
                <strong>{fact.value}</strong>
              </div>
            ))}
          </div>

          <div className="import-window-toolbar-actions">
            <span className="pill">{workspaceStatusLabel}</span>
            {queueBusy ? <span className="status status-degraded">{queueHeadlineLabel}</span> : null}
            {selectedProvider === 'instagram' ? (
              <button
                className="ghost-button"
                disabled={!canRunBackfill}
                onClick={() => void handleRunBackfill()}
                type="button"
              >
                {backfillButtonLabel}
              </button>
            ) : null}
          </div>
        </section>

        <section className="panel import-window-roots">
          <header className="import-window-section-header">
            <div>
              <span className="eyebrow">Roots</span>
              <h2>Scan roots</h2>
            </div>
            <div className="import-window-roots-header-actions">
              <details className="import-window-advanced">
                <summary>Advanced options</summary>
                <label className="import-window-toggle">
                  <input
                    checked={forceReimport}
                    onChange={(event) => setForceReimport(event.target.checked)}
                    type="checkbox"
                  />
                  <span>Force re-import already imported folders</span>
                </label>
              </details>
              <button className="ghost-button" disabled={!activeMethod || queueBusy} onClick={() => void handleRunPreview()} type="button">
                {scanButtonLabel}
              </button>
            </div>
          </header>

          <div className="import-window-root-entry">
            <input
              aria-label="Manual import root"
              onChange={(event) => setManualRootInput(event.target.value)}
              placeholder="Paste a legacy profile root or Instagram parent folder"
              value={manualRootInput}
            />
            <button className="ghost-button" disabled={browsePending} onClick={() => void handleBrowseManualRoot()} type="button">
              {browsePending ? 'Opening...' : 'Browse'}
            </button>
            <button
              className="ghost-button"
              disabled={!normalizeManualRoot(manualRootInput)}
              onClick={() => addManualRoot(manualRootInput)}
              type="button"
            >
              Add root
            </button>
          </div>

          <div className="import-window-root-list">
            {effectiveRoots.map((root) => (
              <div className="import-window-root-row" key={`${root.source}:${root.path}`}>
                <div className="import-window-root-row-main">
                  <span className="pill">{root.label}</span>
                  <code>{root.path}</code>
                </div>
                <button
                  className="ghost-button"
                  onClick={() => (root.removable ? removeManualRoot(root.path) : removeManagedRoot(root.path))}
                  type="button"
                >
                  Remove
                </button>
              </div>
            ))}
            {!effectiveRoots.length ? (
              <div className="empty-state">No scan roots are active yet. Add a manual root or use the detected media paths below.</div>
            ) : null}
          </div>
        </section>

        <section className="import-window-workspace">
          <section className="panel import-window-results">
            <header className="import-window-results-header">
              <div>
                <span className="eyebrow">Review</span>
                <h2>Import review</h2>
              </div>
              <div className="import-window-results-meta">
                {error ? <div className="runtime-log-window-error">{error}</div> : null}
                {pendingProfiles.length > 0 ? (
                  <button
                    className="ghost-button"
                    disabled={queueBusy}
                    onClick={() =>
                      setResolutions((prev) => {
                        const next = { ...prev }
                        for (const profile of pendingProfiles) {
                          next[profile.profileRoot] = { ...prev[profile.profileRoot], action: 'skip' }
                        }
                        return next
                      })}
                    type="button"
                  >
                    Skip all pending
                  </button>
                ) : null}
                <button className="primary-button" disabled={!canRunImport} onClick={() => void handleRunImport()} type="button">
                  {importButtonLabel}
                </button>
              </div>
            </header>

            {reviewSummaryPills.length ? (
              <div className="import-review-summary-strip">
                {reviewSummaryPills.map((pill) => (
                  <div className="import-review-summary-card" key={pill.label}>
                    <span>{pill.label}</span>
                    <strong>{pill.value}</strong>
                  </div>
                ))}
              </div>
            ) : null}

            {preview?.profiles.length ? (
              <div className="import-profile-filter-bar">
                {(
                  [
                    { key: 'all', label: `All (${preview.profiles.length})` },
                    { key: 'attention', label: `Attention (${pendingProfiles.length})` },
                    { key: 'no_media', label: 'No media' },
                    { key: 'needs_account_link', label: 'Link account' },
                    { key: 'duplicate_conflict', label: 'Duplicate' },
                    { key: 'already_imported', label: 'Imported' },
                  ] as const
                ).map(({ key, label }) => (
                  <button
                    className={`ghost-button${profileFilter === key ? ' active' : ''}`}
                    key={key}
                    onClick={() => setProfileFilter(key)}
                    type="button"
                  >
                    {label}
                  </button>
                ))}
              </div>
            ) : null}

            {loading ? <div className="runtime-log-window-empty">Loading import context...</div> : null}
            {!loading && !preview ? (
              <div className="runtime-log-window-empty">
                Run a scan after confirming the roots to review account bindings before import.
              </div>
            ) : null}

            {preview && !preview.profiles.length ? (
              <div className="runtime-log-window-empty">The current roots did not yield any importable legacy profiles.</div>
            ) : null}

            {preview?.profiles.length ? (
              <div className="import-profile-list" role="list">
                <div className="import-profile-table-head" aria-hidden="true">
                  <span>Profile</span>
                  <span>Target account</span>
                  <span>State</span>
                  <span>Media</span>
                  <span>Action</span>
                </div>
                {filteredProfiles.length === 0 ? (
                  <div className="runtime-log-window-empty">No profiles match this filter.</div>
                ) : null}
                {filteredProfiles.map((profile) => {
                  const draft = resolutions[profile.profileRoot] ?? {
                    action: 'import' as const,
                    accountId: profile.accountId,
                  }
                  const needsAccountPicker = !profile.sourceId
                  return (
                    <article className={`import-profile-row${draft.action === 'skip' ? ' import-profile-row-skipped' : ''}`} key={profile.profileRoot} role="listitem">
                      <div className="import-profile-cell import-profile-cell-profile">
                        <h3>{profile.handle}</h3>
                        <p>{profileSummaryLabel(profile)}</p>
                      </div>

                      <div className="import-profile-cell import-profile-cell-account">
                        {needsAccountPicker ? (
                          <label className="import-profile-account-field">
                            <span>Target account</span>
                            <select
                              disabled={draft.action === 'skip' || queueBusy}
                              onChange={(event) =>
                                updateResolution(profile.profileRoot, {
                                  accountId: event.target.value || undefined,
                                })}
                              value={draft.accountId ?? ''}
                            >
                              <option value="">Select an Instagram account</option>
                              {providerAccounts.map((account: ProviderAccount) => (
                                <option key={account.id} value={account.id}>
                                  {account.displayName}
                                </option>
                              ))}
                            </select>
                          </label>
                        ) : (
                          <div className="import-profile-account-fixed">
                            <span>Target account</span>
                            <strong>{profile.accountDisplayName ?? 'Bound by existing source'}</strong>
                          </div>
                        )}
                      </div>

                      <div className="import-profile-cell import-profile-cell-state">
                        <span className={`status-pill status-pill-${profile.importState === 'ready' ? 'ready' : profile.importState === 'already_imported' ? 'warning' : 'error'}`}>
                          {stateLabel(profile)}
                        </span>
                      </div>

                      <div className="import-profile-cell import-profile-cell-media">
                        <strong>{profile.newFileCount} new</strong>
                        <small>{profile.alreadyCatalogedCount} already present · {profile.fileCount} on disk</small>
                      </div>

                      <div className="import-profile-cell import-profile-cell-action">
                        <label className="import-profile-skip">
                          <input
                            checked={draft.action === 'skip'}
                            onChange={(event) =>
                              updateResolution(profile.profileRoot, {
                                action: event.target.checked ? 'skip' : 'import',
                              })}
                            type="checkbox"
                          />
                          <span>Skip</span>
                        </label>
                      </div>

                      <div className="import-profile-row-detail">
                        <div className="import-profile-facts">
                          <span>{profile.accountName ? `Legacy account · ${profile.accountName}` : 'Legacy account missing'}</span>
                          <span>{renderCounts(profile)}</span>
                        </div>
                        <div className="import-profile-paths">
                          <code>{profile.profileRoot}</code>
                        </div>
                        {profile.problems.length ? (
                          <ul className="import-profile-problems">
                            {profile.problems.map((problem) => (
                              <li key={`${profile.profileRoot}:${problem.code}`} className={`import-profile-problem import-profile-problem-${problem.severity}`}>
                                <strong>{problem.code}</strong>
                                <span>{problem.message}</span>
                              </li>
                            ))}
                          </ul>
                        ) : null}
                      </div>
                    </article>
                  )
                })}
              </div>
            ) : null}
          </section>

          <aside className="panel import-window-queue">
            <header className="import-window-results-header">
              <div>
                <span className="eyebrow">Operations</span>
                <h2>Queue status</h2>
              </div>
              <span className="pill">{queueHeadlineLabel}</span>
            </header>

            <div className="import-queue-summary-strip">
              <div className="import-queue-summary-block">
                <span>Now</span>
                <strong>{queueActivityLabel(activeQueueJob)}</strong>
                <small>
                  {queuedJobCount > 0 && activeQueueJob
                    ? `${queueDetailLabel(queueStatus, activeQueueJob)} ${queuedJobCount} more queued behind this job.`
                    : queueDetailLabel(queueStatus, activeQueueJob)}
                </small>
              </div>
              <div className="import-queue-summary-metrics">
                <div>
                  <span>Running</span>
                  <strong>{queueStatus.runningCount}</strong>
                </div>
                <div>
                  <span>Queued</span>
                  <strong>{queueStatus.queuedCount}</strong>
                </div>
                <div>
                  <span>Done</span>
                  <strong>{queueStatus.completedCount}</strong>
                </div>
                <div>
                  <span>Failed</span>
                  <strong>{queueStatus.failedCount}</strong>
                </div>
              </div>
            </div>

            {activeQueueJob ? (
              <article className="import-queue-job import-queue-job-running">
                <header>
                  <strong>{queueJobLabel(activeQueueJob)}</strong>
                  <span className="status status-degraded">running</span>
                </header>
                <p>
                  {queueActivityLabel(activeQueueJob)}
                  {activeQueueJob.progressDetail ? ` · ${activeQueueJob.progressDetail}` : ''}
                </p>
                <small>Started {formatTimestamp(activeQueueJob.startedAt)}</small>
              </article>
            ) : null}

            {result ? (
              <div className="import-window-run-summary">
                <span>Last import</span>
                <strong>{result.importedProfiles} imported · {result.skippedProfiles} skipped · {result.failedProfiles} failed</strong>
                <small>New media {result.importedMediaCount} · Already present {result.alreadyCatalogedCount}</small>
              </div>
            ) : null}
            {queueStatus.latestBackfillResult ? (
              <div className="import-window-run-summary">
                <span>Last naming backfill</span>
                <strong>
                  {queueStatus.latestBackfillResult.insertedEntries} inserted · {queueStatus.latestBackfillResult.updatedEntries} updated
                </strong>
                <small>
                  {queueStatus.latestBackfillResult.scannedProfiles} profiles · {queueStatus.latestBackfillResult.scannedFiles} files · XML sem arquivo {queueStatus.latestBackfillResult.legacyRecordsMissingFiles}
                </small>
              </div>
            ) : null}

            {queueStatus.queuedItems.length > 0 ? (
              <div className="import-queue-list" role="list" aria-label="Queued import jobs">
                {queueStatus.queuedItems.map((job) => (
                  <article className="import-queue-job" key={job.jobId} role="listitem">
                    <header>
                      <strong>{queueJobLabel(job)}</strong>
                      <span className="status">queued</span>
                    </header>
                    <small>Queued {formatTimestamp(job.queuedAt)}</small>
                  </article>
                ))}
              </div>
            ) : null}

            {recentQueueResults.length > 0 ? (
              <div className="import-queue-list" role="list" aria-label="Recent import jobs">
                {recentQueueResults.map((entry) => (
                  <article className="import-queue-job" key={`${entry.jobId}-${entry.finishedAt}`} role="listitem">
                    <header>
                      <strong>{queueResultLabel(entry)}</strong>
                      <span className={entry.status === 'failed' ? 'status status-failed' : 'status status-succeeded'}>
                        {entry.status}
                      </span>
                    </header>
                    <p>{entry.summary}</p>
                    <small>{formatTimestamp(entry.finishedAt)}</small>
                  </article>
                ))}
              </div>
            ) : null}

            {queueStatus.runningCount === 0 && queueStatus.queuedCount === 0 && queueStatus.recentResults.length === 0 ? (
              <div className="runtime-log-window-empty">No import jobs queued yet.</div>
            ) : null}
          </aside>
        </section>
      </main>
    </div>
    </WindowShell>
  )
}
