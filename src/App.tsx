import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import {
  changeSourceMediaPath,
  checkSourceAvailability,
  enqueueSourceDelete,
  loadSourceDeleteQueueStatus,
  loadSourceSyncQueueStatus,
  openAccountsWindow,
  openConnectorRuntimesWindow,
  openSingleVideosWindow,
  openExternalTarget,
  openSourceEditorWindow,
  openBatchEditorWindow,
  openImportWindow,
  openProfileViewWindow,
  openRuntimeLogWindow,
  openSchedulerWindow,
  openSourceSyncQueueWindow,
  pickImportRootFolder,
  subscribeToDesktopRuntimeEvents,
  subscribeToFocusSourceRequest,
  upsertSchedulerGroup,
} from './bridge/desktop'
import {
  createSourceSyncOptions,
  DEFAULT_INSTAGRAM_PRESET_LABELS,
  resolveInstagramGlobalSyncPreset,
} from './domain/sourceSyncOptions'
import type {
  AccountsWindowIntent,
  ConnectorRuntimeStatus,
  InstagramPresetSlot,
  ProviderKey,
  SourceDeleteQueueStatus,
  SourceAvailabilityCheckResult,
  SourceProfileDeleteMode,
  SourceSyncQueueStatus,
} from './domain/models'
import { SettingsPage } from './features/settings/SettingsPage'
import { SourceDeleteConfirmDialog } from './features/sources/SourceDeleteConfirmDialog'
import { AccountsMenu } from './features/workspace/AccountsMenu'
import { InternalDialog } from './features/workspace/InternalDialog'
import { ProfileWorkspace, type SourceSelectionOptions } from './features/workspace/ProfileWorkspace'
import { RuntimeLogWindowPage } from './features/workspace/RuntimeLogWindowPage'
import { invalidateSource, preloadAllThumbnails } from './features/workspace/thumbnailCache'
import {
  buildSourceProfileUrl,
  buildServiceTabs,
  filterSourcesForWorkspace,
  mediaPathBaseDir,
  parseClipboardProfileSeed,
  type ClipboardProfileSeed,
  type GroupSortSwap,
  type ServiceTabKey,
} from './features/workspace/workspaceProfiles'
import { useAppStore } from './state/appStore'

interface ProfileContextMenuState {
  sourceId: string
  x: number
  y: number
}

interface SourceDeleteDialogState {
  sourceIds: string[]
}

interface MenuItem {
  disabled?: boolean
  hint?: string
  label: string
  onSelect: () => void | Promise<void>
}

interface ProfileContextMenuItem extends MenuItem {
  danger?: boolean
}

interface BatchSyncSummary {
  requested: number
  queued: number
  skippedUnsupportedProvider: string[]
  skippedPresetDisabled: string[]
  failed: { handle: string; reason: string }[]
}

interface AvailabilityCheckDialogState {
  summary: SourceAvailabilityCheckResult
}

interface AvailabilityAccountPromptState {
  sourceIds: string[]
  selectedAccountId: string
}

interface ContextMenuPresetAction {
  slot: InstagramPresetSlot
  label: string
  sourceIds: string[]
}

type WindowKind = 'unknown' | 'main' | 'runtime-log'

const sectionDescriptors: Record<'settings', { title: string; subtitle?: string; width: 'medium' | 'large' | 'wide' }> = {
  settings: {
    title: 'Settings',
    width: 'wide',
  },
}

function App() {
  const [windowKind, setWindowKind] = useState<WindowKind>(() => {
    if (typeof window === 'undefined') {
      return 'main'
    }

    return new URLSearchParams(window.location.search).get('window') === 'runtime-log'
      ? 'runtime-log'
      : 'unknown'
  })
  const isRuntimeLogWindow = windowKind === 'runtime-log'
  const activeSection = useAppStore((state) => state.activeSection)
  const bootstrap = useAppStore((state) => state.bootstrap)
  const loading = useAppStore((state) => state.loading)
  const pendingCommand = useAppStore((state) => state.pendingCommand)
  const applySnapshot = useAppStore((state) => state.applySnapshot)
  const refreshSnapshot = useAppStore((state) => state.refreshSnapshot)
  const routeAction = useAppStore((state) => state.routeAction)
  const runSourceSync = useAppStore((state) => state.runSourceSync)
  const setActiveSection = useAppStore((state) => state.setActiveSection)
  const snapshot = useAppStore((state) => state.snapshot)
  const error = useAppStore((state) => state.error)
  const cloneProviderAccount = useAppStore((state) => state.cloneProviderAccount)
  const deleteProviderAccount = useAppStore((state) => state.deleteProviderAccount)
  const pickProfileImage = useAppStore((state) => state.pickSourceProfileImage)
  const resetProfileImage = useAppStore((state) => state.resetSourceProfileImage)
  const openSourceFolder = useAppStore((state) => state.openSourceFolder)
  const upsertSourceProfile = useAppStore((state) => state.upsertSourceProfile)

  const [aboutOpen, setAboutOpen] = useState(false)
  const [availabilityCheckDialog, setAvailabilityCheckDialog] = useState<AvailabilityCheckDialogState>()
  const [availabilityAccountPrompt, setAvailabilityAccountPrompt] = useState<AvailabilityAccountPromptState>()
  const [openMenu, setOpenMenu] = useState<string | null>(null)
  const [profileContextMenu, setProfileContextMenu] = useState<ProfileContextMenuState>()
  const [searchText, setSearchText] = useState('')
  const [savePathFilter, setSavePathFilter] = useState<string>('')
  const [mediaPathChange, setMediaPathChange] = useState<{ sourceIds: string[]; basePath: string } | undefined>()
  const [mediaPathSubmitting, setMediaPathSubmitting] = useState(false)
  const [mediaPathError, setMediaPathError] = useState<string>()
  const [selectedSourceIds, setSelectedSourceIds] = useState<string[]>([])
  const [selectionAnchorId, setSelectionAnchorId] = useState<string>()
  const [serviceTab, setServiceTab] = useState<ServiceTabKey>('all')
  const [sourceDeleteDialogState, setSourceDeleteDialogState] = useState<SourceDeleteDialogState>()
  const [sourceDeleteSubmitting, setSourceDeleteSubmitting] = useState(false)
  const [queueStatus, setQueueStatus] = useState<SourceSyncQueueStatus>(() => createEmptyQueueStatus())
  const [deleteQueueStatus, setDeleteQueueStatus] = useState<SourceDeleteQueueStatus>(() => createEmptyDeleteQueueStatus())
  const runPresetSyncShortcutRef = useRef<(slot: InstagramPresetSlot) => void>(() => undefined)
  const searchInputRef = useRef<HTMLInputElement | null>(null)

  const openAddDialog = useCallback(async (preferredProvider?: ProviderKey, preferredAccountId?: string) => {
    setOpenMenu(null)
    setProfileContextMenu(undefined)
    const clipboardSeed = await readClipboardSeed()
    const seed = clipboardSeed && (!preferredProvider || clipboardSeed.provider === preferredProvider)
      ? clipboardSeed
      : undefined
    try {
      await openSourceEditorWindow({ preferredProvider, preferredAccountId, seed })
    } catch (openError) {
      const message = openError instanceof Error ? openError.message : String(openError)
      if (typeof window !== 'undefined' && typeof window.alert === 'function') {
        window.alert(`Failed to open Profile Editor.\n${message}`)
      }
    }
  }, [])

  useEffect(() => {
    if (windowKind !== 'unknown') {
      return
    }

    let attempts = 0

    const resolveWindowKind = () => {
      try {
        const label = getCurrentWebviewWindow().label
        setWindowKind(label === 'runtime-log' ? 'runtime-log' : 'main')
        return true
      } catch {
        return false
      }
    }

    if (resolveWindowKind()) {
      return
    }

    const timer = window.setInterval(() => {
      attempts += 1
      if (resolveWindowKind()) {
        window.clearInterval(timer)
        return
      }

      if (attempts >= 20) {
        window.clearInterval(timer)
        setWindowKind((current) => (current === 'unknown' ? 'main' : current))
      }
    }, 100)

    return () => {
      window.clearInterval(timer)
    }
  }, [windowKind])

  useEffect(() => {
    if (windowKind !== 'main') {
      return
    }

    void Promise.resolve(bootstrap()).then((snapshot) => {
      if (snapshot?.sources) {
        void preloadAllThumbnails(snapshot.sources)
      }
    }).catch(() => undefined)
  }, [bootstrap, windowKind])

  useEffect(() => {
    if (windowKind !== 'main') {
      return
    }

    void loadSourceSyncQueueStatus()
      .then((status) => {
        setQueueStatus(status)
      })
      .catch(() => {
        setQueueStatus(createEmptyQueueStatus())
      })

    void loadSourceDeleteQueueStatus()
      .then((status) => {
        setDeleteQueueStatus(status)
      })
      .catch(() => {
        setDeleteQueueStatus(createEmptyDeleteQueueStatus())
      })
  }, [windowKind])

  useEffect(() => {
    if (windowKind !== 'main') {
      return undefined
    }

    let disposed = false
    let unsubscribe: (() => void) | undefined

    void subscribeToDesktopRuntimeEvents({
      onSchedulerTick: () => {
        void refreshSnapshot().catch(() => undefined)
      },
      onWorkspaceSnapshotChanged: (nextSnapshot) => {
        applySnapshot(nextSnapshot)
        void preloadAllThumbnails(nextSnapshot.sources)
      },
      onRouteActivation: (actionRoute) => {
        if (actionRoute === 'scheduler') {
          void handleOpenSchedulerConsole()
          void refreshSnapshot().catch(() => undefined)
          return
        }
        routeAction(actionRoute)
        void refreshSnapshot().catch(() => undefined)
      },
      onSourceSyncQueueChanged: (status) => {
        setQueueStatus(status)
      },
      onSourceDeleteQueueChanged: (status) => {
        setDeleteQueueStatus(status)
      },
      onConnectorRuntimeChanged: () => {
        void refreshSnapshot().catch(() => undefined)
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
  }, [applySnapshot, refreshSnapshot, routeAction, windowKind])

  useEffect(() => {
    if (!openMenu) {
      return undefined
    }

    function handlePointerDown(event: MouseEvent) {
      const target = event.target
      if (target instanceof Element && target.closest('[data-menu-root]')) {
        return
      }

      setOpenMenu(null)
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') {
        setOpenMenu(null)
      }
    }

    document.addEventListener('mousedown', handlePointerDown)
    document.addEventListener('keydown', handleKeyDown)
    return () => {
      document.removeEventListener('mousedown', handlePointerDown)
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [openMenu])

  useEffect(() => {
    if (!profileContextMenu) {
      return undefined
    }

    function handlePointerDown(event: MouseEvent) {
      const target = event.target
      if (target instanceof Element && target.closest('[data-profile-context-menu-root]')) {
        return
      }

      setProfileContextMenu(undefined)
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') {
        setProfileContextMenu(undefined)
      }
    }

    document.addEventListener('mousedown', handlePointerDown)
    document.addEventListener('keydown', handleKeyDown)

    return () => {
      document.removeEventListener('mousedown', handlePointerDown)
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [profileContextMenu])

  const serviceTabs = useMemo(
    () => (snapshot ? buildServiceTabs(snapshot.sources, snapshot.providerCatalog) : []),
    [snapshot],
  )
  const filteredSources = useMemo(() => {
    if (!snapshot) {
      return []
    }
    let result = filterSourcesForWorkspace(snapshot.sources, serviceTab, searchText)
    if (savePathFilter) {
      const paths = snapshot.sourceMediaPaths ?? {}
      result = result.filter((source) => mediaPathBaseDir(paths[source.id] ?? '') === savePathFilter)
    }
    return result
  }, [searchText, serviceTab, savePathFilter, snapshot])
  const providerLabels = useMemo(
    () => (snapshot ? new Map(snapshot.providerCatalog.map((provider) => [provider.key, provider.displayName])) : new Map()),
    [snapshot],
  )
  const sourcesById = useMemo(
    () => (snapshot ? new Map(snapshot.sources.map((source) => [source.id, source])) : new Map()),
    [snapshot],
  )
  const selectedSourceSet = useMemo(() => new Set(selectedSourceIds), [selectedSourceIds])
  const selectedSourceId = selectedSourceIds.length > 0 ? selectedSourceIds[selectedSourceIds.length - 1] : undefined
  const selectedSource = useMemo(
    () => (selectedSourceId ? sourcesById.get(selectedSourceId) : undefined),
    [selectedSourceId, sourcesById],
  )
  const selectedSources = useMemo(
    () => selectedSourceIds.map((id) => sourcesById.get(id)).filter((source): source is NonNullable<typeof source> => Boolean(source)),
    [selectedSourceIds, sourcesById],
  )
  const contextMenuSource = useMemo(
    () => (profileContextMenu ? sourcesById.get(profileContextMenu.sourceId) : undefined),
    [profileContextMenu, sourcesById],
  )
  const contextMenuSelectionIds = useMemo(() => {
    if (!profileContextMenu) {
      return []
    }

    if (selectedSourceSet.has(profileContextMenu.sourceId)) {
      return selectedSourceIds
    }

    return [profileContextMenu.sourceId]
  }, [profileContextMenu, selectedSourceIds, selectedSourceSet])
  const sourceDeleteDialogSources = useMemo(
    () =>
      sourceDeleteDialogState
        ? sourceDeleteDialogState.sourceIds
            .map((sourceId) => sourcesById.get(sourceId))
            .filter((source): source is NonNullable<typeof source> => Boolean(source))
        : [],
    [sourceDeleteDialogState, sourcesById],
  )
  const sourceDeleteSyncBlockedIds = useMemo(
    () =>
      sourceDeleteDialogSources
        .filter((source) => isSourceSyncQueuedOrRunning(queueStatus, source.id))
        .map((source) => source.id),
    [queueStatus, sourceDeleteDialogSources],
  )
  const deletingSourceIds = useMemo(
    () => new Set(getDeletingSourceIds(deleteQueueStatus)),
    [deleteQueueStatus],
  )
  const contextMenuHasDeletingSelection = useMemo(
    () => contextMenuSelectionIds.some((sourceId) => deletingSourceIds.has(sourceId)),
    [contextMenuSelectionIds, deletingSourceIds],
  )
  const selectedDeletingCount = useMemo(
    () => selectedSourceIds.filter((sourceId) => deletingSourceIds.has(sourceId)).length,
    [deletingSourceIds, selectedSourceIds],
  )
  const selectedDeleteBlocked = selectedDeletingCount > 0
  const contextMenuSourceUrl = useMemo(
    () => (contextMenuSource ? buildSourceProfileUrl(contextMenuSource) : undefined),
    [contextMenuSource],
  )
  const contextMenuPresetActions = useMemo(() => {
    const actions: ContextMenuPresetAction[] = []
    const slots: InstagramPresetSlot[] = ['preset1', 'preset2']
    const contextSources = contextMenuSelectionIds
      .map((sourceId) => sourcesById.get(sourceId))
      .filter((source): source is NonNullable<typeof source> => Boolean(source))

    for (const slot of slots) {
      const preset = resolveInstagramGlobalSyncPreset(snapshot?.appSettings, slot)
      if (!preset.enabled) {
        continue
      }

      const sourceIds = contextSources
        .filter((source) => source.provider === 'instagram')
        .map((source) => source.id)
      if (sourceIds.length === 0) {
        continue
      }

      const presetLabel = preset.label.trim() || DEFAULT_INSTAGRAM_PRESET_LABELS[slot]
      const label = sourceIds.length > 1
        ? `Download ${presetLabel} (${sourceIds.length})`
        : `Download ${presetLabel}`

      actions.push({
        slot,
        label,
        sourceIds,
      })
    }

    return actions
  }, [contextMenuSelectionIds, snapshot?.appSettings, sourcesById])
  useEffect(() => {
    if (!serviceTabs.some((tab) => tab.key === serviceTab)) {
      setServiceTab('all')
    }
  }, [serviceTab, serviceTabs])

  useEffect(() => {
    const sourceIdSet = new Set(snapshot?.sources.map((source) => source.id) ?? [])
    const visibleIdSet = new Set(filteredSources.map((source) => source.id))

    setSelectedSourceIds((current) => {
      const next = current.filter((sourceId) => sourceIdSet.has(sourceId) && visibleIdSet.has(sourceId))
      return arraysEqual(current, next) ? current : next
    })

    if (selectionAnchorId && (!sourceIdSet.has(selectionAnchorId) || !visibleIdSet.has(selectionAnchorId))) {
      setSelectionAnchorId(undefined)
    }
  }, [filteredSources, selectionAnchorId, snapshot])

  useEffect(() => {
    if (profileContextMenu && !snapshot?.sources.some((source) => source.id === profileContextMenu.sourceId)) {
      setProfileContextMenu(undefined)
    }
  }, [profileContextMenu, snapshot])

  useEffect(() => {
    if (!sourceDeleteDialogState) {
      return
    }

    const sourceIdSet = new Set(snapshot?.sources.map((source) => source.id) ?? [])
    const nextIds = sourceDeleteDialogState.sourceIds.filter((sourceId) => sourceIdSet.has(sourceId))
    if (nextIds.length === 0) {
      setSourceDeleteDialogState(undefined)
      return
    }

    if (!arraysEqual(nextIds, sourceDeleteDialogState.sourceIds)) {
      setSourceDeleteDialogState({ sourceIds: nextIds })
    }
  }, [snapshot, sourceDeleteDialogState])

  runPresetSyncShortcutRef.current = (slot) => {
    void handleRunPresetSync(slot)
  }

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      if (
        windowKind === 'main'
        && !isEditableEventTarget(event.target)
        && (event.ctrlKey || event.metaKey)
        && !event.altKey
        && !event.shiftKey
        && event.key.toLowerCase() === 'f'
      ) {
        event.preventDefault()
        searchInputRef.current?.focus()
        searchInputRef.current?.select()
        return
      }

      if (
        windowKind !== 'main'
        || activeSection !== 'sources'
        || isEditableEventTarget(event.target)
      ) {
        return
      }

      if (!(event.ctrlKey || event.metaKey) || event.altKey || event.shiftKey) {
        return
      }

      const loweredKey = event.key.toLowerCase()
      if (loweredKey === 'v') {
        event.preventDefault()
        void openAddDialog()
        return
      }

      if (loweredKey === '1') {
        event.preventDefault()
        runPresetSyncShortcutRef.current('preset1')
        return
      }

      if (loweredKey === '2') {
        event.preventDefault()
        runPresetSyncShortcutRef.current('preset2')
      }
    }

    document.addEventListener('keydown', handleKeyDown)
    return () => {
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [activeSection, windowKind, openAddDialog])

  useEffect(() => {
    if (windowKind !== 'main' || activeSection !== 'accounts') {
      return
    }

    void openAccountsDialog().finally(() => {
      setActiveSection('sources')
    })
  }, [activeSection, setActiveSection, windowKind])

  useEffect(() => {
    if (windowKind !== 'main' || activeSection !== 'scheduler') {
      return
    }

    void handleOpenSchedulerConsole().finally(() => {
      setActiveSection('sources')
    })
  }, [activeSection, setActiveSection, windowKind])

  // Mantém a referência atual de handleSourceSaved para o listener de foco abaixo,
  // evitando re-assinar o evento a cada render.
  const focusSourceHandlerRef = useRef<(sourceId: string) => void>(() => {})
  useEffect(() => {
    focusSourceHandlerRef.current = handleSourceSaved
  })
  useEffect(() => {
    if (windowKind !== 'main') {
      return undefined
    }

    let disposed = false
    let unsubscribe: (() => void) | undefined
    void subscribeToFocusSourceRequest((sourceId) => {
      focusSourceHandlerRef.current(sourceId)
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
  }, [windowKind])

  if (isRuntimeLogWindow) {
    return <RuntimeLogWindowPage />
  }

  if (windowKind === 'unknown') {
    return <div className="app-shell loading-shell">Initializing window...</div>
  }

  if (loading) {
    return <div className="app-shell loading-shell">Bootstrapping NinjaCrawler workspace...</div>
  }

  if (!snapshot) {
    return <div className="app-shell loading-shell">Failed to load workspace: {error ?? 'missing snapshot'}</div>
  }

  const workspaceSnapshot = snapshot
  const instagramAccounts = workspaceSnapshot.accounts.filter((account) => account.provider === 'instagram')

  const openSection = activeSection === 'settings' ? 'settings' : undefined
  const selectedCount = selectedSources.length
  const combinedQueueCounts = combineQueueCounts(queueStatus, deleteQueueStatus)
  const workspaceInfo = `${filteredSources.length}/${workspaceSnapshot.sources.length} profiles`
  const queueFinishedCount = combinedQueueCounts.completedCount + combinedQueueCounts.failedCount
  const queueProgressPercent = combinedQueueCounts.totalCount > 0
    ? Math.min(100, Math.round((queueFinishedCount / combinedQueueCounts.totalCount) * 100))
    : 0
  const queueSummaryText = formatCombinedQueueSummary(queueStatus, deleteQueueStatus)
  const queueProgressText = combinedQueueCounts.totalCount > 0
    ? `${queueFinishedCount}/${combinedQueueCounts.totalCount}`
    : '0/0'
  const connectorToolbarSummaryText = connectorToolbarSummary(workspaceSnapshot.connectorRuntimes)
  const connectorToolbarTone = connectorToolbarToneClassName(workspaceSnapshot.connectorRuntimes)
  const statusText = error
    ? error
    : deleteQueueStatus.runningCount > 0
      ? `Deleting ${deleteQueueStatus.activeHandle ?? 'profile'}`
      : queueStatus.runningCount > 0
        ? `Syncing ${queueStatus.activeHandle ?? 'profile'}`
        : pendingCommand
          ? formatCommandLabel(pendingCommand)
          : 'Ready'
  const contextMenuPosition = profileContextMenu ? clampContextMenuPosition(profileContextMenu.x, profileContextMenu.y) : undefined

  const fileMenuItems: MenuItem[] = [
    { label: 'Import', onSelect: () => void handleOpenImportWindow() },
    { label: 'Add profile', onSelect: () => void openAddDialog() },
    { label: 'Edit selected', disabled: selectedCount !== 1 || selectedDeleteBlocked, onSelect: () => void openEditDialog() },
    { label: 'Refresh workspace', onSelect: () => void handleRefreshWorkspace() },
  ]
  const downloadMenuItems: MenuItem[] = [
    { label: 'Run selected sync', disabled: selectedCount === 0 || selectedDeleteBlocked, onSelect: () => void handleRunSelectedSync() },
    { label: 'Run preset 1 (Ctrl/Cmd+1)', disabled: selectedCount === 0 || selectedDeleteBlocked, onSelect: () => void handleRunPresetSync('preset1') },
    { label: 'Run preset 2 (Ctrl/Cmd+2)', disabled: selectedCount === 0 || selectedDeleteBlocked, onSelect: () => void handleRunPresetSync('preset2') },
  ]
  const toolsMenuItems: MenuItem[] = [
    { label: 'Scheduler', onSelect: () => void handleOpenSchedulerConsole() },
    { label: 'Queue status', onSelect: () => void handleOpenQueueStatus() },
    { label: 'Runtime log', onSelect: () => void handleOpenRuntimeLog() },
    { label: 'Connectors', onSelect: () => void handleOpenConnectorRuntimes() },
    { label: 'Single videos', onSelect: () => void handleOpenSingleVideos() },
    { label: 'Settings', onSelect: () => openSectionDialog('settings') },
  ]
  const helpMenuItems: MenuItem[] = [
    { label: 'About NinjaCrawler', onSelect: () => openAboutDialog() },
  ]
  const contextSelectionCount = contextMenuSelectionIds.length
  const singleContextSelection = contextSelectionCount <= 1
  const profileContextMenuItems: ProfileContextMenuItem[] = contextMenuSource
    ? [
        {
          label: contextSelectionCount > 1 ? `Download now (${contextSelectionCount})` : 'Download now',
          disabled: contextMenuHasDeletingSelection,
          onSelect: () => void handleRunSelectedSync(contextMenuSource.id),
        },
        ...contextMenuPresetActions.map((presetAction) => ({
          label: presetAction.label,
          disabled: contextMenuHasDeletingSelection,
          onSelect: () => void handleRunPresetSync(
            presetAction.slot,
            contextMenuSource.id,
            presetAction.sourceIds,
          ),
        })),
        { label: 'Edit profile', disabled: !singleContextSelection || contextMenuHasDeletingSelection, onSelect: () => void openEditDialog(contextMenuSource.id) },
        {
          label: contextSelectionCount > 1 ? `Change parameters (${contextSelectionCount})` : 'Change parameters',
          disabled: contextMenuHasDeletingSelection,
          onSelect: () => void handleOpenBatchEditor(contextMenuSelectionIds),
        },
        {
          label: contextSelectionCount > 1 ? `Check availability (${contextSelectionCount})` : 'Check availability',
          disabled: contextMenuHasDeletingSelection,
          onSelect: () => void handleCheckSourceAvailability(contextMenuSource.id),
        },
        {
          label: contextMenuSource.readyForDownload ? 'Pause automatic download' : 'Mark ready for download',
          disabled: !singleContextSelection || contextMenuHasDeletingSelection,
          onSelect: () => void handleToggleSourceReady(contextMenuSource.id),
        },
        {
          label: 'View media',
          disabled: !singleContextSelection || contextMenuHasDeletingSelection,
          onSelect: () => void handleOpenProfileView(contextMenuSource.id),
        },
        {
          label: 'Open containing folder',
          disabled: !singleContextSelection || contextMenuHasDeletingSelection,
          onSelect: () => void handleOpenSourceFolder(contextMenuSource.id),
        },
        {
          label: contextSelectionCount > 1 ? `Change save path (${contextSelectionCount})` : 'Change save path',
          disabled: contextMenuHasDeletingSelection,
          onSelect: () => void handleChangeSourceMediaPath(contextMenuSource.id),
        },
        {
          label: 'Open site',
          disabled: !singleContextSelection || !contextMenuSourceUrl || contextMenuHasDeletingSelection,
          onSelect: () => void handleOpenSourceSite(contextMenuSource.id),
        },
        { label: 'Copy handle', disabled: !singleContextSelection || contextMenuHasDeletingSelection, onSelect: () => void handleCopySourceHandle(contextMenuSource.id) },
        { label: 'Change profile image', disabled: !singleContextSelection || contextMenuHasDeletingSelection, onSelect: () => void handlePickProfileImage(contextMenuSource.id) },
        ...(contextMenuSource.profileImageCustom
          ? [{ label: 'Reset profile image', disabled: !singleContextSelection || contextMenuHasDeletingSelection, onSelect: () => void handleResetProfileImage(contextMenuSource.id) }]
          : []),
        {
          label: contextSelectionCount > 1 ? `Delete selected profiles (${contextSelectionCount})` : 'Delete profile',
          danger: true,
          disabled: contextMenuHasDeletingSelection,
          onSelect: () => void handleDeleteSource(contextMenuSource.id),
        },
      ]
    : []

  async function openEditDialog(sourceId = selectedSource?.id) {
    if (!sourceId) {
      return
    }

    if (deletingSourceIds.has(sourceId)) {
      return
    }

    setOpenMenu(null)
    setProfileContextMenu(undefined)
    setSelectedSourceIds([sourceId])
    setSelectionAnchorId(sourceId)

    try {
      await openSourceEditorWindow({ sourceId })
    } catch (openError) {
      const message = openError instanceof Error ? openError.message : String(openError)
      if (typeof window !== 'undefined' && typeof window.alert === 'function') {
        window.alert(`Failed to open Profile Editor.\n${message}`)
      }
    }
  }

  async function handleOpenBatchEditor(sourceIds: string[]) {
    setOpenMenu(null)
    setProfileContextMenu(undefined)
    try {
      await openBatchEditorWindow(sourceIds)
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      console.error('Failed to open batch editor:', message)
    }
  }

  function openSectionDialog(section: 'settings') {
    setOpenMenu(null)
    setProfileContextMenu(undefined)
    setActiveSection(section)
  }

  async function openAccountsDialog(intent: AccountsWindowIntent = {}) {
    setOpenMenu(null)
    setProfileContextMenu(undefined)
    try {
      await openAccountsWindow(intent)
    } catch (openError) {
      const message = openError instanceof Error ? openError.message : String(openError)
      if (typeof window !== 'undefined' && typeof window.alert === 'function') {
        window.alert(`Failed to open Accounts.\n${message}`)
      }
    }
  }

  function closeSectionDialog() {
    setActiveSection('sources')
  }

  function openAboutDialog() {
    setOpenMenu(null)
    setProfileContextMenu(undefined)
    setAboutOpen(true)
  }

  async function handleReorderGroup(swap: GroupSortSwap) {
    try {
      await upsertSchedulerGroup({ id: swap.groupA.id, name: swap.groupA.name, sortIndex: swap.groupA.sortIndex, criteria: swap.groupA.criteria })
      const nextSnapshot = await upsertSchedulerGroup({ id: swap.groupB.id, name: swap.groupB.name, sortIndex: swap.groupB.sortIndex, criteria: swap.groupB.criteria })
      applySnapshot(nextSnapshot)
    } catch (error) {
      console.error('Failed to reorder groups:', error)
      await refreshSnapshot()
    }
  }

  async function handleRefreshWorkspace() {
    setOpenMenu(null)
    setProfileContextMenu(undefined)
    await refreshSnapshot()
  }

  async function handleOpenRuntimeLog() {
    setOpenMenu(null)
    setProfileContextMenu(undefined)
    try {
      await openRuntimeLogWindow()
    } catch (openError) {
      const message = openError instanceof Error ? openError.message : String(openError)
      if (typeof window !== 'undefined' && typeof window.alert === 'function') {
        window.alert(`Failed to open Runtime Log.\n${message}`)
      }
    }
  }

  async function handleOpenSchedulerConsole() {
    setOpenMenu(null)
    setProfileContextMenu(undefined)
    try {
      await openSchedulerWindow()
    } catch (openError) {
      const message = openError instanceof Error ? openError.message : String(openError)
      if (typeof window !== 'undefined' && typeof window.alert === 'function') {
        window.alert(`Failed to open Scheduler.\n${message}`)
      }
    }
  }

  async function handleOpenQueueStatus() {
    setOpenMenu(null)
    setProfileContextMenu(undefined)
    try {
      await openSourceSyncQueueWindow()
    } catch (openError) {
      const message = openError instanceof Error ? openError.message : String(openError)
      if (typeof window !== 'undefined' && typeof window.alert === 'function') {
        window.alert(`Failed to open Queue Status.\n${message}`)
      }
    }
  }

  async function handleOpenConnectorRuntimes() {
    setOpenMenu(null)
    setProfileContextMenu(undefined)
    try {
      await openConnectorRuntimesWindow()
    } catch (openError) {
      const message = openError instanceof Error ? openError.message : String(openError)
      if (typeof window !== 'undefined' && typeof window.alert === 'function') {
        window.alert(`Failed to open Connector Runtimes.\n${message}`)
      }
    }
  }

  async function handleOpenSingleVideos() {
    setOpenMenu(null)
    setProfileContextMenu(undefined)
    try {
      await openSingleVideosWindow()
    } catch (openError) {
      const message = openError instanceof Error ? openError.message : String(openError)
      if (typeof window !== 'undefined' && typeof window.alert === 'function') {
        window.alert(`Failed to open Single Videos.\n${message}`)
      }
    }
  }

  async function handleOpenImportWindow() {
    setOpenMenu(null)
    setProfileContextMenu(undefined)
    try {
      await openImportWindow()
    } catch (openError) {
      const message = openError instanceof Error ? openError.message : String(openError)
      if (typeof window !== 'undefined' && typeof window.alert === 'function') {
        window.alert(`Failed to open Import.\n${message}`)
      }
    }
  }

  function resolveActionSourceIds(sourceId?: string): string[] {
    if (sourceId) {
      if (selectedSourceSet.has(sourceId)) {
        return selectedSourceIds
      }

      return [sourceId]
    }

    return selectedSourceIds
  }

  function handleSelectSource(sourceId: string, options?: SourceSelectionOptions) {
    const append = Boolean(options?.append)
    const range = Boolean(options?.range)
    const fallbackVisibleSourceIds = filteredSources.map((source) => source.id)
    const visibleSourceIds = options?.visibleIds ?? fallbackVisibleSourceIds

    if (!append && !range && selectedSourceIds.length === 1 && selectedSourceIds[0] === sourceId) {
      setSelectedSourceIds([])
      setSelectionAnchorId(undefined)
      return
    }

    setSelectedSourceIds((current) => {
      if (range && selectionAnchorId) {
        const fromIndex = visibleSourceIds.indexOf(selectionAnchorId)
        const toIndex = visibleSourceIds.indexOf(sourceId)
        if (fromIndex >= 0 && toIndex >= 0) {
          const [start, end] = fromIndex <= toIndex ? [fromIndex, toIndex] : [toIndex, fromIndex]
          const rangeIds = visibleSourceIds.slice(start, end + 1)
          return append
            ? Array.from(new Set([...current, ...rangeIds]))
            : rangeIds
        }
      }

      if (append) {
        return current.includes(sourceId)
          ? current.filter((id) => id !== sourceId)
          : [...current, sourceId]
      }

      return [sourceId]
    })

    if (!range) {
      setSelectionAnchorId(sourceId)
    }
  }

  function handleClearSelection() {
    setSelectedSourceIds([])
    setSelectionAnchorId(undefined)
    setProfileContextMenu(undefined)
  }

  async function runBatchSourceSync(
    sourceIds: string[],
    presetSlot?: InstagramPresetSlot,
  ): Promise<BatchSyncSummary> {
    const globalPreset = presetSlot
      ? resolveInstagramGlobalSyncPreset(snapshot?.appSettings, presetSlot)
      : undefined
    const uniqueSourceIds = Array.from(new Set(sourceIds))
      .filter((sourceId) => !deletingSourceIds.has(sourceId))
    const summary: BatchSyncSummary = {
      requested: uniqueSourceIds.length,
      queued: 0,
      skippedUnsupportedProvider: [],
      skippedPresetDisabled: [],
      failed: [],
    }

    for (const sourceId of uniqueSourceIds) {
      const source = sourcesById.get(sourceId)
      if (!source) {
        continue
      }

      if (presetSlot) {
        if (source.provider !== 'instagram') {
          summary.skippedUnsupportedProvider.push(source.handle)
          continue
        }

        if (!globalPreset?.enabled) {
          summary.skippedPresetDisabled.push(source.handle)
          continue
        }
      }

      try {
        await runSourceSync(sourceId, presetSlot && globalPreset
          ? {
              trigger: presetSlot === 'preset1' ? 'manual_preset_1' : 'manual_preset_2',
              syncOptionsOverride: createSourceSyncOptions('instagram', {
                instagram: {
                  timeline: globalPreset.sections.timeline,
                  reels: globalPreset.sections.reels,
                  stories: globalPreset.sections.stories,
                  storiesUser: globalPreset.sections.storiesUser,
                  tagged: globalPreset.sections.tagged,
                },
              }),
            }
          : undefined)
        summary.queued += 1
      } catch (syncError) {
        const reason = syncError instanceof Error ? syncError.message : String(syncError)
        summary.failed.push({
          handle: source.handle,
          reason,
        })
      }
    }

    return summary
  }

  async function handleRunSelectedSync(sourceId?: string) {
    const actionSourceIds = resolveActionSourceIds(sourceId ?? selectedSourceId)
    if (actionSourceIds.length === 0 || actionSourceIds.some((id) => deletingSourceIds.has(id))) {
      return
    }

    setOpenMenu(null)
    setProfileContextMenu(undefined)
    if (sourceId && !selectedSourceSet.has(sourceId)) {
      setSelectedSourceIds([sourceId])
      setSelectionAnchorId(sourceId)
    }

    // Apenas enfileira; o andamento e eventuais falhas aparecem na fila (Queue
    // Status), sem modal de resumo.
    await runBatchSourceSync(actionSourceIds)
  }

  async function handleRunPresetSync(slot: InstagramPresetSlot, sourceId?: string, sourceIdsOverride?: string[]) {
    const actionSourceIds = sourceIdsOverride && sourceIdsOverride.length > 0
      ? sourceIdsOverride
      : resolveActionSourceIds(sourceId ?? selectedSourceId)
    if (actionSourceIds.length === 0 || actionSourceIds.some((id) => deletingSourceIds.has(id))) {
      return
    }

    setOpenMenu(null)
    setProfileContextMenu(undefined)
    if (!sourceIdsOverride && sourceId && !selectedSourceSet.has(sourceId)) {
      setSelectedSourceIds([sourceId])
      setSelectionAnchorId(sourceId)
    }

    // Presets (P1/P2) também apenas enfileiram — sem modal de resumo. O que for
    // relevante (progresso, falhas) aparece na fila.
    await runBatchSourceSync(actionSourceIds, slot)
  }

  async function runAvailabilityCheckForSelection(sourceIds: string[], accountIdOverride?: string) {
    const summary = accountIdOverride
      ? await checkSourceAvailability(sourceIds, { accountIdOverride })
      : await checkSourceAvailability(sourceIds)
    applySnapshot(summary.snapshot)
    setAvailabilityCheckDialog({ summary })
  }

  async function handleCheckSourceAvailability(sourceId?: string) {
    const actionSourceIds = resolveActionSourceIds(sourceId ?? selectedSourceId)
    if (actionSourceIds.length === 0 || actionSourceIds.some((id) => deletingSourceIds.has(id))) {
      return
    }

    setAvailabilityCheckDialog(undefined)
    setAvailabilityAccountPrompt(undefined)
    setOpenMenu(null)
    setProfileContextMenu(undefined)
    if (sourceId && !selectedSourceSet.has(sourceId)) {
      setSelectedSourceIds([sourceId])
      setSelectionAnchorId(sourceId)
    }

    const hasInstagramSelection = actionSourceIds.some((id) => sourcesById.get(id)?.provider === 'instagram')
    if (!hasInstagramSelection || instagramAccounts.length === 0) {
      await runAvailabilityCheckForSelection(actionSourceIds)
      return
    }

    if (instagramAccounts.length === 1) {
      await runAvailabilityCheckForSelection(actionSourceIds, instagramAccounts[0]?.id)
      return
    }

    setAvailabilityAccountPrompt({
      sourceIds: actionSourceIds,
      selectedAccountId: instagramAccounts[0]?.id ?? '',
    })
  }

  function handleSourceSaved(sourceId: string) {
    const source = workspaceSnapshot.sources.find((entry) => entry.id === sourceId)
    setProfileContextMenu(undefined)
    if (!source) {
      setSelectedSourceIds([sourceId])
      setSelectionAnchorId(sourceId)
      setSearchText('')
      return
    }

    setSelectedSourceIds([source.id])
    setSelectionAnchorId(source.id)
    setSearchText('')
    setServiceTab(source.provider)
  }

  function handleOpenSourceContextMenu(sourceId: string, x: number, y: number, preserveSelection: boolean) {
    if (deletingSourceIds.has(sourceId)) {
      return
    }

    setOpenMenu(null)
    if (!preserveSelection) {
      setSelectedSourceIds([sourceId])
      setSelectionAnchorId(sourceId)
    }
    setProfileContextMenu({ sourceId, x, y })
  }

  async function handleToggleSourceReady(sourceId: string) {
    const source = sourcesById.get(sourceId)
    if (!source || deletingSourceIds.has(sourceId)) {
      return
    }

    setOpenMenu(null)
    setProfileContextMenu(undefined)
    setSelectedSourceIds([source.id])
    setSelectionAnchorId(source.id)

    const savedSnapshot = await upsertSourceProfile({
      id: source.id,
      provider: source.provider,
      sourceKind: source.sourceKind,
      handle: source.handle,
      displayName: source.displayName,
      accountId: source.accountId ?? null,
      labels: [...source.labels],
      readyForDownload: !source.readyForDownload,
    })
    const updatedSource = savedSnapshot.sources.find((entry) => entry.id === source.id)
    if (updatedSource) {
      handleSourceSaved(updatedSource.id)
    }
  }

  function handleDeleteSource(sourceId: string) {
    const actionSourceIds = resolveActionSourceIds(sourceId)
    if (actionSourceIds.length === 0 || actionSourceIds.some((id) => deletingSourceIds.has(id))) {
      return
    }

    setOpenMenu(null)
    setProfileContextMenu(undefined)
    setSelectedSourceIds(actionSourceIds)
    setSelectionAnchorId(actionSourceIds[actionSourceIds.length - 1])
    setSourceDeleteDialogState({ sourceIds: actionSourceIds })
  }

  async function handleConfirmDeleteSource(mode: SourceProfileDeleteMode) {
    if (sourceDeleteDialogSources.length === 0 || sourceDeleteSubmitting) {
      return
    }

    setSourceDeleteSubmitting(true)
    try {
      for (const source of sourceDeleteDialogSources) {
        await enqueueSourceDelete(source.id, mode)
      }
      setSourceDeleteDialogState(undefined)
    } catch (deleteError) {
      const message = deleteError instanceof Error ? deleteError.message : String(deleteError)
      if (typeof window !== 'undefined' && typeof window.alert === 'function') {
        window.alert(`Failed to queue profile delete.\n${message}`)
      }
    } finally {
      setSourceDeleteSubmitting(false)
    }
  }

  async function handleCopySourceHandle(sourceId: string) {
    const source = sourcesById.get(sourceId)
    if (!source || typeof navigator === 'undefined' || !navigator.clipboard?.writeText) {
      return
    }

    setProfileContextMenu(undefined)

    try {
      await navigator.clipboard.writeText(source.handle)
    } catch {
      return
    }
  }

  async function handlePickProfileImage(sourceId: string) {
    if (deletingSourceIds.has(sourceId)) {
      return
    }

    setProfileContextMenu(undefined)
    await pickProfileImage(sourceId)
    invalidateSource(sourceId)
    const updatedSource = useAppStore.getState().snapshot?.sources.find((s) => s.id === sourceId)
    if (updatedSource) {
      void preloadAllThumbnails([updatedSource])
    }
  }

  async function handleResetProfileImage(sourceId: string) {
    if (deletingSourceIds.has(sourceId)) {
      return
    }

    setProfileContextMenu(undefined)
    await resetProfileImage(sourceId)
    invalidateSource(sourceId)
    const updatedSource = useAppStore.getState().snapshot?.sources.find((s) => s.id === sourceId)
    if (updatedSource) {
      void preloadAllThumbnails([updatedSource])
    }
  }

  async function handleOpenSourceFolder(sourceId: string) {
    setProfileContextMenu(undefined)
    await openSourceFolder(sourceId)
  }

  async function handleOpenProfileView(sourceId: string) {
    setProfileContextMenu(undefined)
    await openProfileViewWindow(sourceId)
  }

  async function handleChangeSourceMediaPath(sourceId: string) {
    const ids = contextMenuSelectionIds.length > 0 ? [...contextMenuSelectionIds] : [sourceId]
    setProfileContextMenu(undefined)
    setMediaPathError(undefined)
    const basePath = await pickImportRootFolder()
    if (!basePath) {
      return
    }
    setMediaPathChange({ sourceIds: ids, basePath })
  }

  async function confirmChangeSourceMediaPath() {
    if (!mediaPathChange) {
      return
    }
    setMediaPathSubmitting(true)
    setMediaPathError(undefined)
    try {
      await changeSourceMediaPath(mediaPathChange.sourceIds, mediaPathChange.basePath, true)
      setMediaPathChange(undefined)
    } catch (error) {
      setMediaPathError(error instanceof Error ? error.message : String(error))
    } finally {
      setMediaPathSubmitting(false)
    }
  }

  async function handleOpenSourceSite(sourceId: string) {
    const source = sourcesById.get(sourceId)
    if (!source) {
      return
    }

    const url = buildSourceProfileUrl(source)
    if (!url) {
      return
    }

    setProfileContextMenu(undefined)
    await openExternalTarget(url)
  }

  function handleAccountsMenuOpenSettings(provider: ProviderKey, accountId?: string) {
    if (accountId) {
      void openAccountsDialog({ initialAccountId: accountId, initialMode: 'edit' })
      return
    }

    void openAccountsDialog({ initialProvider: provider, initialMode: 'create' })
  }

  function handleAccountsMenuCreateAccount(provider: ProviderKey) {
    openAccountsDialog({ initialProvider: provider, initialMode: 'create' })
  }

  async function handleAccountsMenuAccountAction(accountId: string, action: 'edit' | 'clone' | 'delete') {
    const account = workspaceSnapshot.accounts.find((entry) => entry.id === accountId)
    if (!account) {
      return
    }

    if (action === 'edit') {
      void openAccountsDialog({ initialAccountId: account.id, initialMode: 'edit' })
      return
    }

    if (action === 'clone') {
      setOpenMenu(null)
      const knownIds = new Set(workspaceSnapshot.accounts.map((entry) => entry.id))
      const savedSnapshot = await cloneProviderAccount(account.id)
      const clonedAccount = savedSnapshot.accounts.find((entry) => !knownIds.has(entry.id))
      if (clonedAccount) {
        void openAccountsDialog({ initialAccountId: clonedAccount.id, initialMode: 'edit' })
      }
      return
    }

    setOpenMenu(null)
    await deleteProviderAccount(account.id)
  }

  return (
    <div className="app-shell">
      <header className="menu-bar" data-menu-root>
        <div className="menu-bar-group">
          <MenuButton items={fileMenuItems} label="File" openMenu={openMenu} setOpenMenu={setOpenMenu} />
          <div className="menu-root">
            <button
              className={openMenu === 'Accounts' ? 'menu-button menu-button-open' : 'menu-button'}
              onClick={() => setOpenMenu(openMenu === 'Accounts' ? null : 'Accounts')}
              type="button"
            >
              Accounts
            </button>
            {openMenu === 'Accounts' ? (
              <AccountsMenu
                accounts={workspaceSnapshot.accounts}
                onAccountAction={(accountId, action) => void handleAccountsMenuAccountAction(accountId, action)}
                onCreateAccount={(provider) => handleAccountsMenuCreateAccount(provider)}
                onOpenSettings={(provider, accountId) => handleAccountsMenuOpenSettings(provider, accountId)}
                providerCatalog={workspaceSnapshot.providerCatalog}
              />
            ) : null}
          </div>
          <MenuButton items={downloadMenuItems} label="Download" openMenu={openMenu} setOpenMenu={setOpenMenu} />
          <MenuButton items={toolsMenuItems} label="Tools" openMenu={openMenu} setOpenMenu={setOpenMenu} />
          <MenuButton items={helpMenuItems} label="Help" openMenu={openMenu} setOpenMenu={setOpenMenu} />
        </div>
        <div className="menu-bar-title">NinjaCrawler</div>
      </header>

      <section className="toolbar-strip">
        <div className="toolbar-group">
          <button className="toolbar-button toolbar-button-primary" onClick={() => void openAddDialog()} type="button">
            + Add
          </button>
          <div aria-hidden="true" className="toolbar-separator" />
          <button className="toolbar-button" disabled={selectedCount !== 1 || selectedDeleteBlocked} onClick={() => void openEditDialog()} type="button">
            Edit
          </button>
          <button className="toolbar-button" onClick={() => void handleRefreshWorkspace()} type="button">
            Refresh
          </button>
          <div aria-hidden="true" className="toolbar-separator" />
          <button className="toolbar-button" disabled={selectedCount === 0 || selectedDeleteBlocked} onClick={() => void handleRunSelectedSync()} type="button">
            Download
          </button>
          <button className="toolbar-button" disabled={selectedCount === 0 || selectedDeleteBlocked} onClick={() => void handleRunPresetSync('preset1')} type="button">
            P1
          </button>
          <button className="toolbar-button" disabled={selectedCount === 0 || selectedDeleteBlocked} onClick={() => void handleRunPresetSync('preset2')} type="button">
            P2
          </button>
          <div aria-hidden="true" className="toolbar-separator" />
          <button className="toolbar-button" onClick={() => void handleOpenQueueStatus()} type="button">
            Queue
          </button>
          <button className="toolbar-button" onClick={() => void handleOpenSchedulerConsole()} type="button">
            Scheduler
          </button>
          <button className="toolbar-button" onClick={() => void handleOpenRuntimeLog()} type="button">
            Log
          </button>
        </div>
        <div className="toolbar-trailing">
          <label className="toolbar-search-field">
            <span>Search</span>
            <input
              aria-label="Search current service tab"
              ref={searchInputRef}
              onChange={(event) => setSearchText(event.target.value)}
              placeholder="Search by handle, name or bio"
              type="search"
              value={searchText}
            />
          </label>
        </div>
      </section>

      <main className="workspace-main">
        <ProfileWorkspace
          deletingSourceIds={Array.from(deletingSourceIds)}
          onClearSelection={handleClearSelection}
          onEditSource={handleSourceSavedFromDoubleClick}
          onReorderGroup={(swap) => void handleReorderGroup(swap)}
          onSelectSource={handleSelectSource}
          onServiceTabChange={setServiceTab}
          onSavePathFilterChange={setSavePathFilter}
          onOpenSourceContextMenu={handleOpenSourceContextMenu}
          searchText={searchText}
          savePathFilter={savePathFilter}
          selectedSourceIds={selectedSourceIds}
          serviceTab={serviceTab}
          snapshot={snapshot}
        />
      </main>

      {contextMenuSource && contextMenuPosition ? (
        <ProfileContextMenu
          handle={contextMenuSource.handle}
          items={profileContextMenuItems}
          position={contextMenuPosition}
          providerLabel={providerLabels.get(contextMenuSource.provider) ?? contextMenuSource.provider}
        />
      ) : null}

      <footer className="status-bar">
        <div className={error ? 'status-cell status-cell-error' : 'status-cell'}>
          <span>Status</span>
          <strong>{statusText}</strong>
        </div>
        <div className="status-cell">
          <span>Profiles</span>
          <strong>{workspaceInfo}</strong>
        </div>
        <div className={`status-cell status-cell-connector status-cell-connector-${connectorToolbarTone}`}>
          <button
            className="status-connector-button"
            onClick={() => void handleOpenConnectorRuntimes()}
            title={`Open connector runtimes (${connectorToolbarSummaryText})`}
            type="button"
          >
            <span>Connectors</span>
            <strong>
              <span aria-hidden="true" className={`status-connector-dot status-connector-dot-${connectorToolbarTone}`} />
              {connectorToolbarSummaryText}
            </strong>
          </button>
        </div>
        <div className="status-cell status-cell-queue">
          <span>Queue</span>
          <strong>{queueSummaryText}</strong>
        </div>
        <div className="status-cell status-cell-queue-progress">
          <span>Progress</span>
          <div className="status-queue-progress-track" aria-hidden>
            <div className="status-queue-progress-fill" style={{ width: `${queueProgressPercent}%` }} />
          </div>
          <strong>{queueProgressText}</strong>
        </div>
        <div className="status-cell status-cell-actions">
          <button className="status-open-queue-button" onClick={() => void handleOpenSingleVideos()} type="button">
            Single Videos
          </button>
          <button className="status-open-queue-button" onClick={() => void handleOpenQueueStatus()} type="button">
            Queue Status
          </button>
        </div>
      </footer>

      {sourceDeleteDialogState && sourceDeleteDialogSources.length > 0 ? (
        <SourceDeleteConfirmDialog
          onCancel={() => setSourceDeleteDialogState(undefined)}
          onConfirm={(mode) => void handleConfirmDeleteSource(mode)}
          pending={sourceDeleteSubmitting}
          sourceCount={sourceDeleteDialogSources.length}
          sourceLabel={sourceDeleteDialogSources.length === 1 ? sourceDeleteDialogSources[0].handle : undefined}
          syncBlockedCount={sourceDeleteSyncBlockedIds.length}
        />
      ) : null}

      {mediaPathChange ? (
        <InternalDialog
          height="fit"
          onClose={() => {
            if (!mediaPathSubmitting) {
              setMediaPathChange(undefined)
            }
          }}
          subtitle="A mídia já baixada será movida para o novo local."
          title={mediaPathChange.sourceIds.length > 1
            ? `Change save path (${mediaPathChange.sourceIds.length})`
            : 'Change save path'}
          width="medium"
        >
          <section className="section-stack">
            <article className="panel">
              <p>
                {mediaPathChange.sourceIds.length > 1
                  ? `${mediaPathChange.sourceIds.length} profiles will be saved under:`
                  : 'This profile will be saved under:'}
              </p>
              <p className="media-path-preview">
                <code>{mediaPathChange.basePath}{'\\<handle>'}</code>
              </p>
              <p className="muted">
                Existing files in each profile folder are moved to the new location. The download ledgers stay consistent.
              </p>
              {mediaPathError ? (
                <p className="source-editor-submit-error" role="alert">{mediaPathError}</p>
              ) : null}
            </article>
            <div className="action-row">
              <button
                className="ghost-button"
                disabled={mediaPathSubmitting}
                onClick={() => setMediaPathChange(undefined)}
                type="button"
              >
                Cancel
              </button>
              <button
                className="primary-button"
                disabled={mediaPathSubmitting}
                onClick={() => void confirmChangeSourceMediaPath()}
                type="button"
              >
                {mediaPathSubmitting ? 'Moving…' : 'Move and change path'}
              </button>
            </div>
          </section>
        </InternalDialog>
      ) : null}

      {availabilityCheckDialog ? (
        <InternalDialog
          height="fit"
          onClose={() => setAvailabilityCheckDialog(undefined)}
          subtitle="Checks profile availability without login when possible, updates handle by user id fallback, and marks problematic profiles."
          title="Availability check summary"
          width="medium"
        >
          <section className="section-stack">
            <article className="panel panel-accent">
              <div className="panel-header compact-header">
                <div>
                  <p className="eyebrow">Overview</p>
                  <h2>Batch result</h2>
                </div>
              </div>
              <div className="compact-grid">
                <article className="stat-card">
                  <span>Requested</span>
                  <strong>{availabilityCheckDialog.summary.requested}</strong>
                </article>
                <article className="stat-card">
                  <span>Processed</span>
                  <strong>{availabilityCheckDialog.summary.processed}</strong>
                </article>
                <article className="stat-card">
                  <span>Unchanged</span>
                  <strong>{availabilityCheckDialog.summary.unchanged}</strong>
                </article>
                <article className="stat-card">
                  <span>Handle updated</span>
                  <strong>{availabilityCheckDialog.summary.updatedHandle}</strong>
                </article>
                <article className="stat-card">
                  <span>Marked problem</span>
                  <strong>{availabilityCheckDialog.summary.markedProblem}</strong>
                </article>
                <article className="stat-card">
                  <span>Skipped</span>
                  <strong>{availabilityCheckDialog.summary.skipped}</strong>
                </article>
                <article className="stat-card">
                  <span>Failed</span>
                  <strong>{availabilityCheckDialog.summary.failed}</strong>
                </article>
              </div>
            </article>

            <article className={availabilityCheckDialog.summary.failed > 0 ? 'panel panel-alert' : 'panel'}>
              <div className="panel-header compact-header">
                <div>
                  <p className="eyebrow">Details</p>
                  <h2>Profiles</h2>
                </div>
                <span className="pill">{availabilityCheckDialog.summary.items.length}</span>
              </div>
              <div className="section-stack">
                {availabilityCheckDialog.summary.items.map((item) => (
                  <div className="list-row" key={`availability-${item.sourceId}`}>
                    <div>
                      <strong>{item.previousHandle || item.sourceId}</strong>
                      <p>
                        {item.status}
                        {item.currentHandle ? ` -> ${item.currentHandle}` : ''}
                      </p>
                      <p>{item.message}</p>
                    </div>
                  </div>
                ))}
              </div>
            </article>
          </section>
        </InternalDialog>
      ) : null}

      {availabilityAccountPrompt ? (
        <InternalDialog
          height="fit"
          onClose={() => setAvailabilityAccountPrompt(undefined)}
          subtitle="Choose which Instagram account session will authenticate this availability check run."
          title="Availability account"
          width="medium"
        >
          <section className="section-stack">
            <article className="panel">
              <div className="panel-header compact-header">
                <div>
                  <p className="eyebrow">Instagram</p>
                  <h2>Account override</h2>
                </div>
              </div>
              <label className="field field-full">
                <span>Use this account for all Instagram profiles in the selection</span>
                <select
                  onChange={(event) =>
                    setAvailabilityAccountPrompt((current) => (current
                      ? { ...current, selectedAccountId: event.target.value }
                      : current))
                  }
                  value={availabilityAccountPrompt.selectedAccountId}
                >
                  {instagramAccounts.map((account) => (
                    <option key={account.id} value={account.id}>
                      {account.displayName || account.id}
                    </option>
                  ))}
                </select>
              </label>
              <div className="inline-note">
                This selection overrides source account bindings for this availability run only.
              </div>
              <div className="action-row">
                <button
                  className="secondary-button"
                  onClick={() => setAvailabilityAccountPrompt(undefined)}
                  type="button"
                >
                  Cancel
                </button>
                <button
                  className="primary-button"
                  disabled={!availabilityAccountPrompt.selectedAccountId}
                  onClick={() => {
                    const payload = availabilityAccountPrompt
                    setAvailabilityAccountPrompt(undefined)
                    void runAvailabilityCheckForSelection(payload.sourceIds, payload.selectedAccountId)
                  }}
                  type="button"
                >
                  Run availability check
                </button>
              </div>
            </article>
          </section>
        </InternalDialog>
      ) : null}

      {openSection ? (
        <InternalDialog
          onClose={closeSectionDialog}
          subtitle={sectionDescriptors[openSection].subtitle}
          title={sectionDescriptors[openSection].title}
          width={sectionDescriptors[openSection].width}
        >
          {openSection === 'settings' ? (
            <SettingsPage />
          ) : null}
        </InternalDialog>
      ) : null}

      {aboutOpen ? (
        <InternalDialog
          onClose={() => setAboutOpen(false)}
          subtitle="Workspace paths are available here while live queue progress stays in the footer and Queue Status window."
          title="About NinjaCrawler"
          width="medium"
        >
          <section className="about-grid">
            <article className="panel panel-accent">
              <div className="panel-header compact-header">
                <div>
                  <p className="eyebrow">Workspace root</p>
                  <h2>Paths</h2>
                </div>
              </div>
              <div className="section-stack">
                <div className="list-row">
                  <div>
                    <strong>Workspace</strong>
                    <p>{workspaceSnapshot.workspaceRoot}</p>
                  </div>
                </div>
                <div className="list-row">
                  <div>
                    <strong>Database</strong>
                    <p>{workspaceSnapshot.dbPath}</p>
                  </div>
                </div>
                <div className="list-row">
                  <div>
                    <strong>Media root</strong>
                    <p>{workspaceSnapshot.mediaRoot}</p>
                  </div>
                </div>
              </div>
            </article>
            <article className="panel">
              <div className="panel-header compact-header">
                <div>
                  <p className="eyebrow">Runtime</p>
                  <h2>Environment</h2>
                </div>
              </div>
              <div className="section-stack">
                <div className="list-row">
                  <div>
                    <strong>Profiles</strong>
                    <p>{workspaceSnapshot.sources.length} registered</p>
                  </div>
                </div>
                <div className="list-row">
                  <div>
                    <strong>Accounts</strong>
                    <p>{workspaceSnapshot.accounts.length} configured</p>
                  </div>
                </div>
                <div className="list-row">
                  <div>
                    <strong>Plans</strong>
                    <p>{workspaceSnapshot.schedulerSets.reduce((count, schedulerSet) => count + schedulerSet.plans.length, 0)} scheduled</p>
                  </div>
                </div>
              </div>
            </article>
          </section>
        </InternalDialog>
      ) : null}
    </div>
  )

  function handleSourceSavedFromDoubleClick(sourceId: string) {
    setProfileContextMenu(undefined)
    setSelectedSourceIds([sourceId])
    setSelectionAnchorId(sourceId)
    void openEditDialog(sourceId)
  }
}

function connectorToolbarSummary(runtimes: ConnectorRuntimeStatus[]): string {
  if (runtimes.some((runtime) => runtime.status === 'error')) {
    return 'Review'
  }
  const updateCount = runtimes.filter((runtime) => runtime.updateAvailable).length
  if (updateCount > 0) {
    return updateCount === 1 ? '1 new' : `${updateCount} new`
  }
  if (runtimes.some((runtime) => runtime.status === 'pending_activation')) {
    return 'Pending'
  }
  if (runtimes.some((runtime) => runtime.status === 'downloading')) {
    return 'Syncing'
  }
  if (runtimes.some((runtime) => runtime.status === 'checking')) {
    return 'Scan'
  }
  if (runtimes.some((runtime) => runtime.status === 'custom_override')) {
    return 'Custom'
  }
  return 'Ready'
}

function connectorToolbarToneClassName(runtimes: ConnectorRuntimeStatus[]): 'ready' | 'degraded' | 'failed' {
  if (runtimes.some((runtime) => runtime.status === 'error')) {
    return 'failed'
  }
  const updateCount = runtimes.filter((runtime) => runtime.updateAvailable).length
  if (
    updateCount > 0
    || runtimes.some((runtime) => runtime.status === 'pending_activation')
    || runtimes.some((runtime) => runtime.status === 'downloading')
    || runtimes.some((runtime) => runtime.status === 'checking')
    || runtimes.some((runtime) => runtime.status === 'custom_override')
  ) {
    return 'degraded'
  }
  return 'ready'
}

interface MenuButtonProps {
  items: MenuItem[]
  label: string
  openMenu: string | null
  setOpenMenu: (value: string | null) => void
}

function MenuButton({ items, label, openMenu, setOpenMenu }: MenuButtonProps) {
  const isOpen = openMenu === label

  return (
    <div className="menu-root">
      <button
        className={isOpen ? 'menu-button menu-button-open' : 'menu-button'}
        onClick={() => setOpenMenu(isOpen ? null : label)}
        type="button"
      >
        {label}
      </button>
      {isOpen ? (
        <div className="menu-dropdown">
          {items.map((item) => (
            <button
              key={item.label}
              className="menu-item"
              disabled={item.disabled}
              onClick={() => {
                void item.onSelect()
                setOpenMenu(null)
              }}
              type="button"
            >
              <strong>{item.label}</strong>
              {item.hint ? <span>{item.hint}</span> : null}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  )
}

interface ProfileContextMenuProps {
  handle: string
  items: ProfileContextMenuItem[]
  position: {
    left: number
    top: number
  }
  providerLabel: string
}

function ProfileContextMenu({ handle, items, position, providerLabel }: ProfileContextMenuProps) {
  return (
    <div
      className="profile-context-menu"
      data-profile-context-menu-root
      role="menu"
      style={{ left: `${position.left}px`, top: `${position.top}px` }}
    >
      <div className="profile-context-menu-header">
        <strong>{handle}</strong>
        <span>{providerLabel}</span>
      </div>
      <div className="profile-context-menu-group">
        {items.map((item, index) => (
          <button
            key={item.label}
            className={item.danger ? 'profile-context-menu-item profile-context-menu-item-danger' : 'profile-context-menu-item'}
            data-menu-last={index === items.length - 1 ? 'true' : undefined}
            disabled={item.disabled}
            onClick={() => void item.onSelect()}
            role="menuitem"
            type="button"
          >
            <strong>{item.label}</strong>
            {item.hint ? <span>{item.hint}</span> : null}
          </button>
        ))}
      </div>
    </div>
  )
}

async function readClipboardSeed(): Promise<ClipboardProfileSeed | undefined> {
  if (typeof navigator === 'undefined' || !navigator.clipboard?.readText) {
    return undefined
  }

  try {
    const clipboardText = await navigator.clipboard.readText()
    return parseClipboardProfileSeed(clipboardText)
  } catch {
    return undefined
  }
}

function createEmptyQueueStatus(): SourceSyncQueueStatus {
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

function getDeletingSourceIds(status: SourceDeleteQueueStatus): string[] {
  return Array.from(
    new Set(
      [...status.queuedItems, ...status.runningItems].map((item) => item.sourceId),
    ),
  )
}

function combineQueueCounts(
  syncStatus: SourceSyncQueueStatus,
  deleteStatus: SourceDeleteQueueStatus,
): {
  queuedCount: number
  runningCount: number
  completedCount: number
  failedCount: number
  totalCount: number
} {
  return {
    queuedCount: syncStatus.queuedCount + deleteStatus.queuedCount,
    runningCount: syncStatus.runningCount + deleteStatus.runningCount,
    completedCount: syncStatus.completedCount + deleteStatus.completedCount,
    failedCount: syncStatus.failedCount + deleteStatus.failedCount,
    totalCount: syncStatus.totalCount + deleteStatus.totalCount,
  }
}

function formatCombinedQueueSummary(
  syncStatus: SourceSyncQueueStatus,
  deleteStatus: SourceDeleteQueueStatus,
): string {
  if (deleteStatus.runningCount > 0) {
    const queuedCount = syncStatus.queuedCount + deleteStatus.queuedCount
    return queuedCount > 0
      ? `${deleteStatus.runningCount} deleting · ${queuedCount} queued`
      : `${deleteStatus.runningCount} deleting`
  }

  if (syncStatus.runningCount > 0) {
    const extraQueued = syncStatus.queuedCount + deleteStatus.queuedCount
    return extraQueued > 0
      ? `${syncStatus.runningCount} running · ${extraQueued} queued`
      : `${syncStatus.runningCount} running`
  }

  const queuedCount = syncStatus.queuedCount + deleteStatus.queuedCount
  if (queuedCount > 0) {
    return `${queuedCount} queued`
  }

  const totalCount = syncStatus.totalCount + deleteStatus.totalCount
  return totalCount > 0 ? 'Queue idle' : 'No queued jobs'
}

function isSourceSyncQueuedOrRunning(status: SourceSyncQueueStatus, sourceId: string): boolean {
  return status.queuedItems.some((item) => item.sourceId === sourceId)
    || status.runningItems.some((item) => item.sourceId === sourceId)
}

function formatCommandLabel(command: string): string {
  return command
    .split('_')
    .filter((segment) => segment.length > 0)
    .map((segment) => segment.charAt(0).toUpperCase() + segment.slice(1))
    .join(' ')
}

function clampContextMenuPosition(x: number, y: number): { left: number; top: number } {
  if (typeof window === 'undefined') {
    return { left: x, top: y }
  }

  const width = 248
  const height = 360
  const margin = 12

  return {
    left: Math.max(margin, Math.min(x, window.innerWidth - width - margin)),
    top: Math.max(44, Math.min(y, window.innerHeight - height - margin)),
  }
}

function isEditableEventTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) {
    return false
  }

  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) {
    return true
  }

  if (target instanceof HTMLElement && target.isContentEditable) {
    return true
  }

  return Boolean(target.closest('[contenteditable="true"], [role="textbox"]'))
}

function arraysEqual(values: string[], otherValues: string[]): boolean {
  if (values === otherValues) {
    return true
  }

  if (values.length !== otherValues.length) {
    return false
  }

  for (let index = 0; index < values.length; index += 1) {
    if (values[index] !== otherValues[index]) {
      return false
    }
  }

  return true
}

export default App
