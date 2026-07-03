import { useCallback, useEffect, useMemo, useState } from 'react'
import type { SourceProfile, WorkspaceSnapshot } from '../../domain/models'
import { getPreviewSource } from './thumbnailCache'
import {
  buildServiceTabs,
  filterSourcesForWorkspace,
  formatSourceHandleLabel,
  groupSourcesForWorkspace,
  mediaPathBaseDir,
  sortSourcesInGroups,
  swapGroupSortIndex,
  type GroupMode,
  type GroupSortSwap,
  type ServiceTabKey,
  type SortMode,
} from './workspaceProfiles'
import { syncProblemBadgeLabel } from './syncProblemBadges'
import { computeSyncFreshness } from './syncFreshness'

const VIEW_MODE_KEY = 'nc-view-mode'
const GROUP_MODE_KEY = 'nc-group-mode'
const SORT_MODE_KEY = 'nc-sort-mode'

type ViewMode = 'grid' | 'list'

/** Rótulo compacto e não ambíguo para um path: mantém a raiz (drive) e os dois
 * últimos segmentos, abreviando o meio com "…" apenas quando necessário.
 * Ex.: `C:\Users\ninja\Pictures\NinjaCrawler` -> `C:\…\Pictures\NinjaCrawler`. */
function savePathLabel(path: string): string {
  const normalized = path.replace(/[\\/]+$/, '')
  const separator = normalized.includes('\\') ? '\\' : '/'
  const segments = normalized.split(/[\\/]/).filter((segment) => segment.length > 0)
  if (segments.length <= 3) {
    return normalized
  }
  const root = segments[0]
  const tail = segments.slice(-2).join(separator)
  return `${root}${separator}…${separator}${tail}`
}

function getStoredViewMode(): ViewMode {
  return (localStorage.getItem(VIEW_MODE_KEY) as ViewMode | null) ?? 'grid'
}

function getStoredGroupMode(): GroupMode {
  const stored = localStorage.getItem(GROUP_MODE_KEY)
  if (stored === 'category' || stored === 'group') return stored
  return 'none'
}

function getStoredSortMode(): SortMode {
  const stored = localStorage.getItem(SORT_MODE_KEY)
  if (stored === 'name-desc' || stored === 'date-added' || stored === 'last-synced') return stored
  return 'name-asc'
}

export interface SourceSelectionOptions {
  append?: boolean
  range?: boolean
  visibleIds?: string[]
}

interface ProfileWorkspaceProps {
  deletingSourceIds?: string[]
  snapshot: WorkspaceSnapshot
  searchText: string
  savePathFilter: string
  selectedSourceIds: string[]
  serviceTab: ServiceTabKey
  onSelectSource: (id: string, options?: SourceSelectionOptions) => void
  onClearSelection: () => void
  onServiceTabChange: (value: ServiceTabKey) => void
  onSavePathFilterChange: (value: string) => void
  onEditSource: (id: string) => void
  onOpenSourceContextMenu: (id: string, x: number, y: number, preserveSelection: boolean) => void
  onReorderGroup?: (swap: GroupSortSwap) => void
}

export function ProfileWorkspace({
  deletingSourceIds = [],
  snapshot,
  searchText,
  savePathFilter,
  selectedSourceIds,
  serviceTab,
  onSelectSource,
  onClearSelection,
  onServiceTabChange,
  onSavePathFilterChange,
  onEditSource,
  onOpenSourceContextMenu,
  onReorderGroup,
}: ProfileWorkspaceProps) {
  const [viewMode, setViewModeState] = useState<ViewMode>(getStoredViewMode)
  const [groupMode, setGroupModeState] = useState<GroupMode>(getStoredGroupMode)
  const [sortMode, setSortModeState] = useState<SortMode>(getStoredSortMode)
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set())
  // Reference time for last-sync freshness badges, captured once on mount.
  // Freshness tiers are coarse (24h / 7d / 30d), so a fixed reference is fine;
  // reopening the workspace recomputes it against the latest data.
  const [now] = useState(() => Date.now())

  const serviceTabs = useMemo(
    () => buildServiceTabs(snapshot.sources, snapshot.providerCatalog),
    [snapshot.providerCatalog, snapshot.sources],
  )
  const savePathOptions = useMemo(() => {
    const paths = snapshot.sourceMediaPaths
    if (!paths) {
      return [] as { path: string; count: number }[]
    }
    const counts = new Map<string, number>()
    for (const value of Object.values(paths)) {
      const base = mediaPathBaseDir(value)
      if (base) {
        counts.set(base, (counts.get(base) ?? 0) + 1)
      }
    }
    return Array.from(counts.entries())
      .map(([path, count]) => ({ path, count }))
      .sort((left, right) => left.path.localeCompare(right.path))
  }, [snapshot.sourceMediaPaths])
  const filteredSources = useMemo(() => {
    let result = filterSourcesForWorkspace(snapshot.sources, serviceTab, searchText)
    if (savePathFilter) {
      const paths = snapshot.sourceMediaPaths ?? {}
      result = result.filter((source) => mediaPathBaseDir(paths[source.id] ?? '') === savePathFilter)
    }
    return result
  }, [searchText, serviceTab, savePathFilter, snapshot.sources, snapshot.sourceMediaPaths])
  useEffect(() => {
    if (savePathFilter && !savePathOptions.some((option) => option.path === savePathFilter)) {
      onSavePathFilterChange('')
    }
  }, [savePathFilter, savePathOptions, onSavePathFilterChange])
  const groups = useMemo(
    () => groupSourcesForWorkspace(filteredSources, groupMode, snapshot.schedulerGroups),
    [filteredSources, groupMode, snapshot.schedulerGroups],
  )
  const sortedGroups = useMemo(
    () => sortSourcesInGroups(groups, sortMode),
    [groups, sortMode],
  )
  const providerLabels = useMemo(
    () => new Map(snapshot.providerCatalog.map((provider) => [provider.key, provider.displayName])),
    [snapshot.providerCatalog],
  )
  const selectedSourceSet = useMemo(() => new Set(selectedSourceIds), [selectedSourceIds])
  const deletingSourceSet = useMemo(() => new Set(deletingSourceIds), [deletingSourceIds])
  const visibleSourceIds = useMemo(
    () => sortedGroups.flatMap((group) => group.sources.map((source) => source.id)),
    [sortedGroups],
  )

  const sourceGroupMap = useMemo(() => {
    const map = new Map<string, string>()
    for (const group of sortedGroups) {
      for (const source of group.sources) {
        if (!map.has(source.id)) {
          map.set(source.id, group.key)
        }
      }
    }
    return map
  }, [sortedGroups])

  const selectedGroupKeys = useMemo(
    () => new Set(
      selectedSourceIds
        .map((sourceId) => sourceGroupMap.get(sourceId))
        .filter((groupKey): groupKey is string => Boolean(groupKey)),
    ),
    [selectedSourceIds, sourceGroupMap],
  )

  useEffect(() => {
    const selectedSourceId = selectedSourceIds[selectedSourceIds.length - 1]
    if (!selectedSourceId) {
      return
    }

    const element = Array.from(document.querySelectorAll<HTMLElement>('[data-source-id]'))
      .find((candidate) => candidate.dataset.sourceId === selectedSourceId)
    if (element && typeof element.scrollIntoView === 'function') {
      element.scrollIntoView({ block: 'nearest' })
    }
  }, [collapsedGroups, selectedSourceIds, sortedGroups])

  function changeGroupMode(next: GroupMode) {
    localStorage.setItem(GROUP_MODE_KEY, next)
    setGroupModeState(next)
    setCollapsedGroups(new Set())
  }

  function changeSortMode(next: SortMode) {
    localStorage.setItem(SORT_MODE_KEY, next)
    setSortModeState(next)
  }

  function toggleGroupCollapse(groupKey: string) {
    setCollapsedGroups((prev) => {
      const next = new Set(prev)
      if (next.has(groupKey)) {
        next.delete(groupKey)
      } else {
        next.add(groupKey)
      }
      return next
    })
  }

  function setViewMode(next: ViewMode) {
    localStorage.setItem(VIEW_MODE_KEY, next)
    setViewModeState(next)
  }

  const handleLetterJump = useCallback(
    (event: React.KeyboardEvent) => {
      if (event.key.length !== 1 || event.metaKey || event.ctrlKey || event.altKey) return
      const letter = event.key.toLowerCase()
      if (letter < 'a' || letter > 'z') return

      const currentId = selectedSourceIds[selectedSourceIds.length - 1]
      if (!currentId) return
      const currentGroupKey = sourceGroupMap.get(currentId)
      if (!currentGroupKey) return

      const group = sortedGroups.find((g) => g.key === currentGroupKey)
      if (!group) return

      const matches = group.sources.filter((s) => {
        const name = formatSourceHandleLabel(s.handle).toLowerCase()
        return name.startsWith(letter)
      })
      if (matches.length === 0) return

      const currentIndex = matches.findIndex((s) => s.id === currentId)
      const nextMatch = matches[(currentIndex + 1) % matches.length]

      onSelectSource(nextMatch.id)
      const element = document.querySelector(`[data-source-id="${nextMatch.id}"]`)
      element?.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
    },
    [selectedSourceIds, sourceGroupMap, sortedGroups, onSelectSource],
  )

  function handleCardKeyDown(
    event: React.KeyboardEvent,
    sourceId: string,
    deleting: boolean,
  ) {
    if (deleting) return

    if (event.key !== 'ContextMenu' && !(event.shiftKey && event.key === 'F10')) {
      return
    }

    event.preventDefault()
    const rect = event.currentTarget.getBoundingClientRect()
    const preserveSelection = selectedSourceSet.has(sourceId) && selectedSourceSet.size > 0
    if (!preserveSelection) {
      onSelectSource(sourceId)
    }
    onOpenSourceContextMenu(sourceId, rect.left + 18, rect.top + 18, preserveSelection)
  }

  function handleCardContextMenu(
    event: React.MouseEvent,
    sourceId: string,
    deleting: boolean,
  ) {
    if (deleting) {
      event.preventDefault()
      return
    }

    event.preventDefault()
    const preserveSelection = selectedSourceSet.has(sourceId) && selectedSourceSet.size > 0
    if (!preserveSelection) {
      onSelectSource(sourceId)
    }
    onOpenSourceContextMenu(sourceId, event.clientX, event.clientY, preserveSelection)
  }

  const hasAnyProfiles = filteredSources.length > 0
  const showGroupHeaders = groupMode !== 'none' && sortedGroups.length > 0
  const displayedGroupKeys = useMemo(() => sortedGroups.map((g) => g.key), [sortedGroups])
  const handleGridShellMouseDown = useCallback(
    (event: React.MouseEvent<HTMLElement>) => {
      const target = event.target
      if (!(target instanceof Element)) {
        return
      }

      if (target.closest('[data-source-id]') || target.closest('.profile-group-header-row')) {
        return
      }

      onClearSelection()
    },
    [onClearSelection],
  )

  return (
    <section className="workspace-board panel panel-accent">
      <nav aria-label="Service tabs" className="workspace-tabs">
        {serviceTabs.map((tab) => (
          <button
            key={tab.key}
            className={tab.key === serviceTab ? 'service-tab service-tab-active' : 'service-tab'}
            onClick={() => onServiceTabChange(tab.key)}
            type="button"
          >
            <strong>{tab.label}</strong>
            <span>{tab.count}</span>
          </button>
        ))}
        <div className="workspace-tabs-end">
          {savePathOptions.length > 1 ? (
            <select
              aria-label="Filter by save path"
              className="workspace-toolbar-select"
              onChange={(e) => onSavePathFilterChange(e.target.value)}
              title={savePathFilter || 'All save paths'}
              value={savePathFilter}
            >
              <option value="">All paths</option>
              {savePathOptions.map(({ path, count }) => (
                <option key={path} title={path} value={path}>
                  {`${savePathLabel(path)} (${count})`}
                </option>
              ))}
            </select>
          ) : null}
          <select
            aria-label="Group by"
            className="workspace-toolbar-select"
            onChange={(e) => changeGroupMode(e.target.value as GroupMode)}
            value={groupMode}
          >
            <option value="none">No grouping</option>
            <option value="category">Category</option>
            <option value="group">Group</option>
          </select>
          <select
            aria-label="Sort by"
            className="workspace-toolbar-select"
            onChange={(e) => changeSortMode(e.target.value as SortMode)}
            value={sortMode}
          >
            <option value="name-asc">Name A-Z</option>
            <option value="name-desc">Name Z-A</option>
            <option value="date-added">Date added</option>
            <option value="last-synced">Last synced</option>
          </select>
          <div className="view-toggle-group">
            <button
              aria-label="Grid view"
              className={`view-toggle-button${viewMode === 'grid' ? ' view-toggle-button-active' : ''}`}
              onClick={() => setViewMode('grid')}
              title="Grid view"
              type="button"
            >
              <svg viewBox="0 0 16 16" xmlns="http://www.w3.org/2000/svg"><path d="M1 1h6v6H1zm8 0h6v6H9zM1 9h6v6H1zm8 0h6v6H9z" /></svg>
            </button>
            <button
              aria-label="List view"
              className={`view-toggle-button${viewMode === 'list' ? ' view-toggle-button-active' : ''}`}
              onClick={() => setViewMode('list')}
              title="List view"
              type="button"
            >
              <svg viewBox="0 0 16 16" xmlns="http://www.w3.org/2000/svg"><path d="M1 1h14v3H1zm0 5h14v3H1zm0 5h14v3H1z" /></svg>
            </button>
          </div>
        </div>
      </nav>

      <section className="profile-grid-shell" onKeyDown={handleLetterJump} onMouseDown={handleGridShellMouseDown}>
        {hasAnyProfiles ? (
          sortedGroups.map((group, groupIndex) => {
            const collapsed = collapsedGroups.has(group.key) && !selectedGroupKeys.has(group.key)
            const canReorder = groupMode === 'group' && onReorderGroup && group.key !== 'group:__ungrouped__'
            const groupClassName = [
              'profile-group',
              showGroupHeaders ? 'profile-group-framed' : '',
              collapsed ? 'profile-group-collapsed' : '',
            ].filter(Boolean).join(' ')
            return (
              <div key={group.key} className={groupClassName}>
                {showGroupHeaders ? (
                  <div className="profile-group-header-row">
                    <button
                      className="profile-group-header"
                      onClick={() => toggleGroupCollapse(group.key)}
                      type="button"
                    >
                      <span className={collapsed ? 'profile-group-chevron collapsed' : 'profile-group-chevron'}>&#9656;</span>
                      <strong>{group.label}</strong>
                      <span className="profile-group-count">{group.sources.length}</span>
                    </button>
                    {canReorder ? (
                      <div className="profile-group-order-buttons">
                        <button
                          aria-label="Move group up"
                          className="profile-group-order-button"
                          disabled={groupIndex === 0}
                          onClick={() => {
                            const swap = swapGroupSortIndex(snapshot.schedulerGroups ?? [], displayedGroupKeys, group.key, 'up')
                            if (swap) onReorderGroup(swap)
                          }}
                          title="Move up"
                          type="button"
                        >
                          <svg viewBox="0 0 8 5" xmlns="http://www.w3.org/2000/svg"><path d="M4 0L0 5h8z" /></svg>
                        </button>
                        <button
                          aria-label="Move group down"
                          className="profile-group-order-button"
                          disabled={groupIndex >= sortedGroups.length - 1 || sortedGroups[groupIndex + 1]?.key === 'group:__ungrouped__'}
                          onClick={() => {
                            const swap = swapGroupSortIndex(snapshot.schedulerGroups ?? [], displayedGroupKeys, group.key, 'down')
                            if (swap) onReorderGroup(swap)
                          }}
                          title="Move down"
                          type="button"
                        >
                          <svg viewBox="0 0 8 5" xmlns="http://www.w3.org/2000/svg"><path d="M0 0l4 5 4-5z" /></svg>
                        </button>
                      </div>
                    ) : null}
                  </div>
                ) : null}
                {!collapsed ? (
                  <div className="profile-group-content">
                    {viewMode === 'grid' ? (
                      <div className="profile-grid" role="list">
                        {group.sources.map((source) => (
                          <GridCard
                            key={source.id}
                            deleting={deletingSourceSet.has(source.id)}
                            now={now}
                            onCardContextMenu={handleCardContextMenu}
                            onCardKeyDown={handleCardKeyDown}
                            onEditSource={onEditSource}
                            onSelectSource={onSelectSource}
                            providerLabel={providerLabels.get(source.provider) ?? source.provider}
                            selected={selectedSourceSet.has(source.id)}
                            source={source}
                            visibleSourceIds={visibleSourceIds}
                          />
                        ))}
                      </div>
                    ) : (
                      <div className="profile-list" role="list">
                        <div aria-hidden="true" className="profile-list-header">
                          <span className="profile-list-col-thumb" />
                          <span className="profile-list-col-name">Handle</span>
                          <span className="profile-list-col-provider">Provider</span>
                          <span className="profile-list-col-status">Status</span>
                        </div>
                        {group.sources.map((source) => (
                          <ListRow
                            key={source.id}
                            deleting={deletingSourceSet.has(source.id)}
                            now={now}
                            onCardContextMenu={handleCardContextMenu}
                            onCardKeyDown={handleCardKeyDown}
                            onEditSource={onEditSource}
                            onSelectSource={onSelectSource}
                            providerLabel={providerLabels.get(source.provider) ?? source.provider}
                            selected={selectedSourceSet.has(source.id)}
                            source={source}
                            visibleSourceIds={visibleSourceIds}
                          />
                        ))}
                      </div>
                    )}
                  </div>
                ) : null}
              </div>
            )
          })
        ) : (
          <div className="workspace-empty-state">
            <strong>No profiles in this view.</strong>
            <span>Change the service tab, clear the search, or add a new profile.</span>
          </div>
        )}
      </section>
    </section>
  )
}

// -- Shared card props --

interface CardProps {
  source: SourceProfile
  selected: boolean
  deleting: boolean
  now: number
  providerLabel: string
  visibleSourceIds: string[]
  onSelectSource: (id: string, options?: SourceSelectionOptions) => void
  onEditSource: (id: string) => void
  onCardContextMenu: (event: React.MouseEvent, sourceId: string, deleting: boolean) => void
  onCardKeyDown: (event: React.KeyboardEvent, sourceId: string, deleting: boolean) => void
}

// -- Grid card --

function GridCard({
  source,
  selected,
  deleting,
  now,
  providerLabel,
  visibleSourceIds,
  onSelectSource,
  onEditSource,
  onCardContextMenu,
  onCardKeyDown,
}: CardProps) {
  const previewSrc = getPreviewSource(source)
  const displayHandle = formatSourceHandleLabel(source.handle)
  const syncIssueLabel = source.syncProblemMessage ?? source.syncProblemCode
  const syncIssueBadge = syncProblemBadgeLabel(source.syncProblemCode)
  const freshness = computeSyncFreshness(source.lastSyncedAt, now)

  return (
    <button
      aria-disabled={deleting}
      aria-haspopup="menu"
      className={[
        'profile-card',
        selected ? 'profile-card-selected' : '',
        source.syncProblemCode ? 'profile-card-has-sync-issue' : '',
      ].filter(Boolean).join(' ')}
      data-source-id={source.id}
      onClick={(event) =>
        onSelectSource(source.id, {
          append: event.metaKey || event.ctrlKey,
          range: event.shiftKey,
          visibleIds: visibleSourceIds,
        })}
      onContextMenu={(event) => onCardContextMenu(event, source.id, deleting)}
      onDoubleClick={() => { if (!deleting) onEditSource(source.id) }}
      onKeyDown={(event) => onCardKeyDown(event, source.id, deleting)}
      role="listitem"
      type="button"
    >
      <div className="profile-thumb-frame">
        {previewSrc ? (
          <img alt={displayHandle} className="profile-thumb"src={previewSrc} />
        ) : (
          <div className={`profile-thumb profile-thumb-fallback provider-${source.provider}`}>
            <span>{providerLabel.slice(0, 2).toUpperCase()}</span>
          </div>
        )}
        <span className={`profile-provider-badge provider-${source.provider}`}>{providerLabel}</span>
        {syncIssueLabel || freshness ? (
          <div className="profile-badge-stack">
            {syncIssueLabel ? (
              <span className="profile-sync-issue-badge" title={syncIssueLabel}>
                {syncIssueBadge}
              </span>
            ) : null}
            {freshness ? (
              <span
                className={`profile-sync-age-badge profile-sync-age-${freshness.tier}`}
                title={freshness.longLabel}
              >
                {freshness.shortLabel}
              </span>
            ) : null}
          </div>
        ) : null}
        {selected ? <span aria-hidden className="profile-selection-indicator" /> : null}
      </div>
      <span className="profile-name" title={displayHandle}>
        {displayHandle}
      </span>
    </button>
  )
}

// -- List row --

function ListRow({
  source,
  selected,
  deleting,
  now,
  providerLabel,
  visibleSourceIds,
  onSelectSource,
  onEditSource,
  onCardContextMenu,
  onCardKeyDown,
}: CardProps) {
  const previewSrc = getPreviewSource(source)
  const displayHandle = formatSourceHandleLabel(source.handle)
  const syncIssueLabel = source.syncProblemMessage ?? source.syncProblemCode
  const syncIssueBadge = syncProblemBadgeLabel(source.syncProblemCode)
  const freshness = computeSyncFreshness(source.lastSyncedAt, now)

  return (
    <button
      aria-disabled={deleting}
      aria-haspopup="menu"
      className={[
        'profile-list-row',
        selected ? 'profile-list-row-selected' : '',
        source.syncProblemCode ? 'profile-list-row-has-sync-issue' : '',
      ].filter(Boolean).join(' ')}
      data-source-id={source.id}
      onClick={(event) =>
        onSelectSource(source.id, {
          append: event.metaKey || event.ctrlKey,
          range: event.shiftKey,
          visibleIds: visibleSourceIds,
        })}
      onContextMenu={(event) => onCardContextMenu(event, source.id, deleting)}
      onDoubleClick={() => { if (!deleting) onEditSource(source.id) }}
      onKeyDown={(event) => onCardKeyDown(event, source.id, deleting)}
      role="listitem"
      type="button"
    >
      <div className="profile-list-col-thumb">
        {previewSrc ? (
          <img alt={displayHandle} className="profile-list-thumb"src={previewSrc} />
        ) : (
          <div className={`profile-list-thumb profile-list-thumb-fallback provider-${source.provider}`}>
            <span>{providerLabel.slice(0, 2).toUpperCase()}</span>
          </div>
        )}
      </div>
      <span className="profile-list-col-name" title={displayHandle}>
        {displayHandle}
      </span>
      <span className={`profile-list-col-provider provider-badge-inline provider-${source.provider}`}>
        {providerLabel}
      </span>
      <div className="profile-list-col-status">
        {selected ? <span aria-hidden className="profile-selection-indicator profile-selection-indicator-inline" /> : null}
        {syncIssueLabel ? (
          <span className="profile-sync-issue-pill" title={syncIssueLabel}>
            {syncIssueBadge}
          </span>
        ) : null}
        {freshness ? (
          <span
            className={`profile-sync-age-inline profile-sync-age-${freshness.tier}`}
            title={freshness.longLabel}
          >
            {freshness.shortLabel}
          </span>
        ) : null}
        {!deleting ? (
          <span
            aria-label={source.readyForDownload ? 'Ready' : 'Pending'}
            className={`profile-status-dot profile-status-dot-${source.readyForDownload ? 'ready' : 'pending'}`}
          />
        ) : null}
      </div>
    </button>
  )
}

