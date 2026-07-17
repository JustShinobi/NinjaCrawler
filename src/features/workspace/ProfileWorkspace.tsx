import { memo, useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import type { SourceProfile, WorkspaceSnapshot } from '../../domain/models'
import {
  getAvatarThumbnailsEpoch,
  getPreviewSource,
  subscribeToAvatarThumbnails,
} from './thumbnailCache'
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
import { resolveSyncSectionChips, summarizeEnabledSections } from './profileSyncSections'

const VIEW_MODE_KEY = 'nc-view-mode'
const GROUP_MODE_KEY = 'nc-group-mode'
const SORT_MODE_KEY = 'nc-sort-mode'
const SECTIONS_MODE_KEY = 'nc-sections-mode'

type ViewMode = 'grid' | 'list'
// Visibilidade do fingerprint de sections no grid: escondido, só no hover do
// card, ou sempre visível.
type SectionsMode = 'off' | 'hover' | 'on'

// Virtualização da lista de perfis (padrão do ProfileViewPage): grupos são
// achatados em linhas virtuais — header, fileiras de grid ou linhas de lista.
// A moldura visual do grupo é desenhada por segmentos (start/meio/end) porque
// header e conteúdo viram linhas irmãs em vez de um box único.
type WorkspaceRow =
  | { type: 'group-header'; key: string; groupIndex: number; collapsed: boolean; frameEnd: boolean }
  | { type: 'grid-row'; key: string; sources: SourceProfile[]; frameEnd: boolean }
  | { type: 'list-header'; key: string }
  | { type: 'list-row'; key: string; source: SourceProfile; frameEnd: boolean }

// Espelha o CSS: .profile-grid tem células fixas de 118px com gap de 0.7rem;
// .profile-grid-shell tem padding horizontal de 0.6rem; .workspace-vframe usa
// 0.62rem + 1px de borda por lado.
const GRID_CELL_WIDTH_PX = 118
const GRID_GAP_PX = 11.2
const SHELL_HORIZONTAL_PADDING_PX = 19.2
const FRAME_HORIZONTAL_PADDING_PX = 21.9
const ROW_OVERSCAN = 6
const GROUP_HEADER_ROW_ESTIMATE_PX = 56
const GRID_ROW_ESTIMATE_PX = 195
const LIST_HEADER_ROW_ESTIMATE_PX = 30
const LIST_ROW_ESTIMATE_PX = 42

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

function getStoredSectionsMode(): SectionsMode {
  const stored = localStorage.getItem(SECTIONS_MODE_KEY)
  if (stored === 'hover' || stored === 'on') return stored
  return 'off'
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
  onVisibleSourceIdsChange?: (sourceIds: string[]) => void
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
  onVisibleSourceIdsChange,
  onEditSource,
  onOpenSourceContextMenu,
  onReorderGroup,
}: ProfileWorkspaceProps) {
  const [viewMode, setViewModeState] = useState<ViewMode>(getStoredViewMode)
  const [groupMode, setGroupModeState] = useState<GroupMode>(getStoredGroupMode)
  const [sortMode, setSortModeState] = useState<SortMode>(getStoredSortMode)
  const [sectionsMode, setSectionsModeState] = useState<SectionsMode>(getStoredSectionsMode)
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
    onVisibleSourceIdsChange?.(filteredSources.map((source) => source.id))
  }, [filteredSources, onVisibleSourceIdsChange])
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
  const avatarEpoch = useSyncExternalStore(subscribeToAvatarThumbnails, getAvatarThumbnailsEpoch)
  const previewSrcBySource = useMemo(() => {
    // avatarEpoch é a assinatura do cache de thumbs: muda quando um lote de
    // thumbnails chega do backend e força o recálculo das URLs.
    void avatarEpoch
    const map = new Map<string, string | undefined>()
    for (const group of sortedGroups) {
      for (const source of group.sources) {
        map.set(source.id, getPreviewSource(source))
      }
    }
    return map
  }, [sortedGroups, avatarEpoch])

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

  const hasAnyProfiles = filteredSources.length > 0
  const showGroupHeaders = groupMode !== 'none' && sortedGroups.length > 0
  const displayedGroupKeys = useMemo(() => sortedGroups.map((g) => g.key), [sortedGroups])

  // Mede a largura útil do scroll container (clientWidth exclui a barra) para
  // derivar as colunas do grid; reage a resize via ResizeObserver.
  const shellRef = useRef<HTMLElement | null>(null)
  const [containerWidth, setContainerWidth] = useState(0)
  useEffect(() => {
    const element = shellRef.current
    if (!element) return undefined
    const update = () => setContainerWidth(element.clientWidth - SHELL_HORIZONTAL_PADDING_PX)
    update()
    const observer = new ResizeObserver(update)
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

  const gridCols = useMemo(() => {
    const available = showGroupHeaders
      ? containerWidth - FRAME_HORIZONTAL_PADDING_PX
      : containerWidth
    if (available <= 0) return 1
    return Math.max(1, Math.floor((available + GRID_GAP_PX) / (GRID_CELL_WIDTH_PX + GRID_GAP_PX)))
  }, [containerWidth, showGroupHeaders])

  const { workspaceRows, rowIndexBySourceId } = useMemo(() => {
    const rows: WorkspaceRow[] = []
    const indexBySource = new Map<string, number>()
    for (let groupIndex = 0; groupIndex < sortedGroups.length; groupIndex += 1) {
      const group = sortedGroups[groupIndex]
      const collapsed = collapsedGroups.has(group.key) && !selectedGroupKeys.has(group.key)
      if (showGroupHeaders) {
        rows.push({
          type: 'group-header',
          key: `header:${group.key}`,
          groupIndex,
          collapsed,
          frameEnd: collapsed || group.sources.length === 0,
        })
        if (collapsed) continue
      }
      if (viewMode === 'grid') {
        for (let start = 0; start < group.sources.length; start += gridCols) {
          const slice = group.sources.slice(start, start + gridCols)
          const rowIndex = rows.length
          for (const source of slice) {
            if (!indexBySource.has(source.id)) indexBySource.set(source.id, rowIndex)
          }
          rows.push({
            type: 'grid-row',
            key: `grid:${group.key}:${slice[0]?.id ?? start}`,
            sources: slice,
            frameEnd: start + gridCols >= group.sources.length,
          })
        }
      } else {
        if (group.sources.length > 0) {
          rows.push({ type: 'list-header', key: `list-header:${group.key}` })
        }
        for (let index = 0; index < group.sources.length; index += 1) {
          const source = group.sources[index]
          if (!indexBySource.has(source.id)) indexBySource.set(source.id, rows.length)
          rows.push({
            type: 'list-row',
            key: `list:${group.key}:${source.id}`,
            source,
            frameEnd: index === group.sources.length - 1,
          })
        }
      }
    }
    return { workspaceRows: rows, rowIndexBySourceId: indexBySource }
  }, [sortedGroups, collapsedGroups, selectedGroupKeys, showGroupHeaders, viewMode, gridCols])

  // eslint-disable-next-line react-hooks/incompatible-library -- TanStack Virtual returns unstable functions by design; same usage pattern as ProfileViewPage.
  const rowVirtualizer = useVirtualizer({
    count: workspaceRows.length,
    getScrollElement: () => shellRef.current,
    estimateSize: (index) => {
      const row = workspaceRows[index]
      switch (row?.type) {
        case 'group-header':
          return GROUP_HEADER_ROW_ESTIMATE_PX
        case 'list-header':
          return LIST_HEADER_ROW_ESTIMATE_PX
        case 'list-row':
          return LIST_ROW_ESTIMATE_PX
        default:
          return GRID_ROW_ESTIMATE_PX
      }
    },
    overscan: ROW_OVERSCAN,
    getItemKey: (index) => workspaceRows[index]?.key ?? index,
  })

  // Colunas/modo/largura mudam a altura das linhas: descarta as medidas em
  // cache para o virtualizer re-medir com o novo tamanho.
  useEffect(() => {
    rowVirtualizer.measure()
  }, [rowVirtualizer, gridCols, viewMode, containerWidth])

  // Rola até o card apenas quando a SELEÇÃO muda. O snapshot é atualizado o
  // tempo todo durante um sync (novas referências de sortedGroups), e sem este
  // guard cada refresh puxava o scroll de volta ao card selecionado, brigando
  // com a roda do mouse.
  const lastScrolledSelectionRef = useRef<string | undefined>(undefined)
  useEffect(() => {
    const selectedSourceId = selectedSourceIds[selectedSourceIds.length - 1]
    if (!selectedSourceId) {
      lastScrolledSelectionRef.current = undefined
      return
    }
    if (lastScrolledSelectionRef.current === selectedSourceId) {
      return
    }

    // Com virtualização o card fora da viewport não existe no DOM; rola pela
    // linha virtual em vez de procurar o elemento.
    const rowIndex = rowIndexBySourceId.get(selectedSourceId)
    if (rowIndex !== undefined) {
      rowVirtualizer.scrollToIndex(rowIndex, { align: 'auto' })
      lastScrolledSelectionRef.current = selectedSourceId
    }
  }, [selectedSourceIds, rowIndexBySourceId, rowVirtualizer])

  function changeGroupMode(next: GroupMode) {
    localStorage.setItem(GROUP_MODE_KEY, next)
    setGroupModeState(next)
    setCollapsedGroups(new Set())
  }

  function changeSortMode(next: SortMode) {
    localStorage.setItem(SORT_MODE_KEY, next)
    setSortModeState(next)
  }

  function changeSectionsMode(next: SectionsMode) {
    localStorage.setItem(SECTIONS_MODE_KEY, next)
    setSectionsModeState(next)
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

  // Função simples de propósito: vai no onKeyDown da section (não memoizada)
  // e usar o virtualizer como dep de useCallback dispara o alerta de
  // memoização do react-hooks (a instância muda a cada render).
  function handleLetterJump(event: React.KeyboardEvent) {
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
    const rowIndex = rowIndexBySourceId.get(nextMatch.id)
    if (rowIndex !== undefined) {
      rowVirtualizer.scrollToIndex(rowIndex, { align: 'auto' })
    }
  }

  const handleCardKeyDown = useCallback((
    event: React.KeyboardEvent,
    sourceId: string,
    deleting: boolean,
  ) => {
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
  }, [selectedSourceSet, onSelectSource, onOpenSourceContextMenu])

  const handleCardContextMenu = useCallback((
    event: React.MouseEvent,
    sourceId: string,
    deleting: boolean,
  ) => {
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
  }, [selectedSourceSet, onSelectSource, onOpenSourceContextMenu])

  // Ref em vez de prop: visibleSourceIds é um array novo a cada snapshot e
  // como prop invalidaria o React.memo dos cards a cada atualização.
  const visibleSourceIdsRef = useRef(visibleSourceIds)
  visibleSourceIdsRef.current = visibleSourceIds
  const handleCardClick = useCallback((sourceId: string, event: React.MouseEvent) => {
    onSelectSource(sourceId, {
      append: event.metaKey || event.ctrlKey,
      range: event.shiftKey,
      visibleIds: visibleSourceIdsRef.current,
    })
  }, [onSelectSource])

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
          {viewMode === 'grid' ? (
            <select
              aria-label="Sync sections overlay"
              className="workspace-toolbar-select"
              onChange={(e) => changeSectionsMode(e.target.value as SectionsMode)}
              title="Show which sync sections are enabled on each card"
              value={sectionsMode}
            >
              <option value="off">Sections: off</option>
              <option value="hover">Sections: on hover</option>
              <option value="on">Sections: always</option>
            </select>
          ) : null}
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

      <section
        ref={shellRef}
        className="profile-grid-shell"
        onKeyDown={handleLetterJump}
        onMouseDown={handleGridShellMouseDown}
      >
        {hasAnyProfiles ? (
          // Somente as linhas visíveis (+ overscan) são montadas; a altura
          // total é reservada para a barra de rolagem ser fiel.
          <div
            className="workspace-virtual"
            style={{ height: `${rowVirtualizer.getTotalSize()}px` }}
          >
            {rowVirtualizer.getVirtualItems().map((virtualItem) => {
              const row = workspaceRows[virtualItem.index]
              if (!row) return null
              const groupGap = showGroupHeaders && row.type !== 'list-header' && row.frameEnd
              return (
                <div
                  key={row.key}
                  className={groupGap ? 'workspace-virtual-row workspace-virtual-row-group-gap' : 'workspace-virtual-row'}
                  data-index={virtualItem.index}
                  ref={rowVirtualizer.measureElement}
                  style={{ transform: `translateY(${virtualItem.start}px)` }}
                >
                  {row.type === 'group-header' ? (() => {
                    const group = sortedGroups[row.groupIndex]
                    if (!group) return null
                    const canReorder = groupMode === 'group' && onReorderGroup && group.key !== 'group:__ungrouped__'
                    const frameClassName = [
                      'workspace-vframe',
                      'workspace-vframe-start',
                      row.frameEnd ? 'workspace-vframe-end' : 'workspace-vframe-header-pad',
                      row.collapsed ? 'profile-group-collapsed' : '',
                    ].filter(Boolean).join(' ')
                    return (
                      <div className={frameClassName}>
                        <div className="profile-group-header-row">
                          <button
                            className="profile-group-header"
                            onClick={() => toggleGroupCollapse(group.key)}
                            type="button"
                          >
                            <span className={row.collapsed ? 'profile-group-chevron collapsed' : 'profile-group-chevron'}>&#9656;</span>
                            <strong>{group.label}</strong>
                            <span className="profile-group-count">{group.sources.length}</span>
                          </button>
                          {canReorder ? (
                            <div className="profile-group-order-buttons">
                              <button
                                aria-label="Move group up"
                                className="profile-group-order-button"
                                disabled={row.groupIndex === 0}
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
                                disabled={row.groupIndex >= sortedGroups.length - 1 || sortedGroups[row.groupIndex + 1]?.key === 'group:__ungrouped__'}
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
                      </div>
                    )
                  })() : row.type === 'grid-row' ? (
                    <div
                      className={[
                        showGroupHeaders ? 'workspace-vframe' : '',
                        showGroupHeaders && row.frameEnd ? 'workspace-vframe-end' : '',
                        row.frameEnd ? '' : 'workspace-vrow-gap',
                      ].filter(Boolean).join(' ') || undefined}
                    >
                      <div
                        className="profile-grid"
                        role="list"
                        style={{ gridTemplateColumns: `repeat(${gridCols}, ${GRID_CELL_WIDTH_PX}px)` }}
                      >
                        {row.sources.map((source) => (
                          <GridCard
                            key={source.id}
                            deleting={deletingSourceSet.has(source.id)}
                            now={now}
                            onCardClick={handleCardClick}
                            onCardContextMenu={handleCardContextMenu}
                            onCardKeyDown={handleCardKeyDown}
                            onEditSource={onEditSource}
                            previewSrc={previewSrcBySource.get(source.id)}
                            providerLabel={providerLabels.get(source.provider) ?? source.provider}
                            sectionsMode={sectionsMode}
                            selected={selectedSourceSet.has(source.id)}
                            source={source}
                          />
                        ))}
                      </div>
                    </div>
                  ) : row.type === 'list-header' ? (
                    <div className={showGroupHeaders ? 'workspace-vframe' : undefined}>
                      <div aria-hidden="true" className="profile-list-header">
                        <span className="profile-list-col-thumb" />
                        <span className="profile-list-col-name">Handle</span>
                        <span className="profile-list-col-provider">Provider</span>
                        <span className="profile-list-col-status">Status</span>
                      </div>
                    </div>
                  ) : (
                    <div
                      className={[
                        showGroupHeaders ? 'workspace-vframe' : '',
                        showGroupHeaders && row.frameEnd ? 'workspace-vframe-end' : '',
                        row.frameEnd ? '' : 'workspace-vlist-gap',
                      ].filter(Boolean).join(' ') || undefined}
                    >
                      <ListRow
                        deleting={deletingSourceSet.has(row.source.id)}
                        now={now}
                        onCardClick={handleCardClick}
                        onCardContextMenu={handleCardContextMenu}
                        onCardKeyDown={handleCardKeyDown}
                        onEditSource={onEditSource}
                        previewSrc={previewSrcBySource.get(row.source.id)}
                        providerLabel={providerLabels.get(row.source.provider) ?? row.source.provider}
                        selected={selectedSourceSet.has(row.source.id)}
                        source={row.source}
                      />
                    </div>
                  )}
                </div>
              )
            })}
          </div>
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
  previewSrc: string | undefined
  providerLabel: string
  // Só o GridCard consome; o ListRow ignora.
  sectionsMode?: SectionsMode
  onCardClick: (id: string, event: React.MouseEvent) => void
  onEditSource: (id: string) => void
  onCardContextMenu: (event: React.MouseEvent, sourceId: string, deleting: boolean) => void
  onCardKeyDown: (event: React.KeyboardEvent, sourceId: string, deleting: boolean) => void
}

// -- Grid card --

// React.memo: durante a rolagem o virtualizer re-renderiza o pai a cada
// mudança de viewport; cards com props inalteradas não re-renderizam.
const GridCard = memo(function GridCard({
  source,
  selected,
  deleting,
  now,
  previewSrc,
  providerLabel,
  sectionsMode = 'off',
  onCardClick,
  onEditSource,
  onCardContextMenu,
  onCardKeyDown,
}: CardProps) {
  const displayHandle = formatSourceHandleLabel(source.handle)
  const syncIssueLabel = source.syncProblemMessage ?? source.syncProblemCode
  const syncIssueBadge = syncProblemBadgeLabel(source.syncProblemCode)
  const freshness = computeSyncFreshness(source.lastSyncedAt, now)
  // Perfil pausado (não Ready for Download) recua no grid. Um problema de sync
  // já traz o próprio badge de aviso, então o pill "paused" só aparece na pausa
  // manual — sem empilhar dois avisos no mesmo card.
  const paused = !source.readyForDownload
  const showPausedBadge = paused && !syncIssueLabel
  // Fingerprint de sections: só computa quando o overlay não está desligado.
  const sectionChips = sectionsMode === 'off' ? [] : resolveSyncSectionChips(source)

  return (
    <button
      aria-disabled={deleting}
      aria-haspopup="menu"
      className={[
        'profile-card',
        selected ? 'profile-card-selected' : '',
        source.syncProblemCode ? 'profile-card-has-sync-issue' : '',
        paused ? 'profile-card-paused' : '',
        sectionsMode === 'hover' ? 'profile-card-sections-hover' : '',
      ].filter(Boolean).join(' ')}
      data-source-id={source.id}
      onClick={(event) => onCardClick(source.id, event)}
      onContextMenu={(event) => onCardContextMenu(event, source.id, deleting)}
      onDoubleClick={() => { if (!deleting) onEditSource(source.id) }}
      onKeyDown={(event) => onCardKeyDown(event, source.id, deleting)}
      role="listitem"
      type="button"
    >
      <div className="profile-thumb-frame">
        {previewSrc ? (
          <img alt={displayHandle} className="profile-thumb" decoding="async" loading="lazy" src={previewSrc} />
        ) : (
          <div className={`profile-thumb profile-thumb-fallback provider-${source.provider}`}>
            <span>{providerLabel.slice(0, 2).toUpperCase()}</span>
          </div>
        )}
        <span className={`profile-provider-badge provider-${source.provider}`}>{providerLabel}</span>
        {syncIssueLabel || freshness || showPausedBadge ? (
          <div className="profile-badge-stack">
            {syncIssueLabel ? (
              <span className="profile-sync-issue-badge" title={syncIssueLabel}>
                {syncIssueBadge}
              </span>
            ) : null}
            {showPausedBadge ? (
              <span className="profile-paused-badge" title="Not ready for download — automatic downloads paused">
                Paused
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
        {sectionChips.length > 0 ? (
          <div className="profile-sections-strip" data-provider={source.provider} title={summarizeEnabledSections(sectionChips)}>
            {sectionChips.map((chip) => (
              <span
                className={`profile-section-chip${chip.enabled ? ' profile-section-chip-on' : ' profile-section-chip-off'}`}
                key={chip.code}
                title={`${chip.label}: ${chip.enabled ? 'on' : 'off'}`}
              >
                {chip.code}
              </span>
            ))}
          </div>
        ) : null}
      </div>
      <span className="profile-name" title={displayHandle}>
        {displayHandle}
      </span>
    </button>
  )
})
// -- List row --

const ListRow = memo(function ListRow({
  source,
  selected,
  deleting,
  now,
  previewSrc,
  providerLabel,
  onCardClick,
  onEditSource,
  onCardContextMenu,
  onCardKeyDown,
}: CardProps) {
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
      onClick={(event) => onCardClick(source.id, event)}
      onContextMenu={(event) => onCardContextMenu(event, source.id, deleting)}
      onDoubleClick={() => { if (!deleting) onEditSource(source.id) }}
      onKeyDown={(event) => onCardKeyDown(event, source.id, deleting)}
      role="listitem"
      type="button"
    >
      <div className="profile-list-col-thumb">
        {previewSrc ? (
          <img alt={displayHandle} className="profile-list-thumb" decoding="async" loading="lazy" src={previewSrc} />
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
})
