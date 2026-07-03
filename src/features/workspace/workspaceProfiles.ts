import type { ProviderDescriptor, ProviderKey, SchedulerGroup, SourceProfile } from '../../domain/models'

export type ServiceTabKey = 'all' | ProviderKey

export interface ServiceTab {
  key: ServiceTabKey
  label: string
  count: number
}

export interface ClipboardProfileSeed {
  provider: ProviderKey
  handle: string
  displayName: string
}

export function formatSourceHandleLabel(handle: string): string {
  return handle.trim().replace(/^@+/, '')
}

export function buildServiceTabs(
  sources: SourceProfile[],
  providerCatalog: ProviderDescriptor[],
): ServiceTab[] {
  const counts = new Map<ProviderKey, number>()

  for (const source of sources) {
    counts.set(source.provider, (counts.get(source.provider) ?? 0) + 1)
  }

  const tabs: ServiceTab[] = [{ key: 'all', label: 'All', count: sources.length }]

  for (const provider of providerCatalog) {
    const count = counts.get(provider.key) ?? 0
    if (count > 0) {
      tabs.push({ key: provider.key, label: provider.displayName, count })
    }
  }

  return tabs
}

export function filterSourcesForWorkspace(
  sources: SourceProfile[],
  serviceTab: ServiceTabKey,
  searchText: string,
): SourceProfile[] {
  const normalizedSearch = searchText.trim().toLowerCase()

  return sources.filter((source) => {
    if (serviceTab !== 'all' && source.provider !== serviceTab) {
      return false
    }

    if (normalizedSearch.length === 0) {
      return true
    }

    const haystack = [
      source.handle,
      source.displayName,
      source.provider,
      source.syncOptions?.instagram?.description ?? '',
      ...(source.syncOptions?.instagram?.previousHandles ?? []),
      ...source.labels,
    ]
      .join(' ')
      .toLowerCase()

    return haystack.includes(normalizedSearch)
  })
}

// -- Grouping --

export type GroupMode = 'none' | 'category' | 'group'

export interface SourceProfileGroup {
  key: string
  label: string
  sources: SourceProfile[]
}

export function sourceProfileCategory(source: SourceProfile): 'favorite' | 'temporary' | 'regular' {
  if (source.syncOptions?.instagram?.favorite) {
    return 'favorite'
  }
  if (source.syncOptions?.instagram?.temporary) {
    return 'temporary'
  }
  return 'regular'
}

const CATEGORY_LABELS: Record<string, string> = { favorite: 'Favorite', regular: 'Regular', temporary: 'Temporary' }

export function groupSourcesForWorkspace(
  sources: SourceProfile[],
  mode: GroupMode,
  schedulerGroups?: SchedulerGroup[],
): SourceProfileGroup[] {
  if (mode === 'none') {
    return [{ key: 'all', label: 'All', sources }]
  }

  if (mode === 'category') {
    const buckets = new Map<string, SourceProfile[]>()
    for (const source of sources) {
      const category = sourceProfileCategory(source)
      const bucket = buckets.get(category)
      if (bucket) {
        bucket.push(source)
      } else {
        buckets.set(category, [source])
      }
    }
    return ['favorite', 'regular', 'temporary']
      .filter((key) => buckets.has(key))
      .map((key) => ({
        key,
        label: CATEGORY_LABELS[key],
        sources: buckets.get(key)!,
      }))
  }

  // mode === 'group'
  const groupMap = new Map((schedulerGroups ?? []).map((g) => [g.id, g]))
  const buckets = new Map<string, SourceProfile[]>()
  const ungrouped: SourceProfile[] = []
  for (const source of sources) {
    if (!source.groupId || !groupMap.has(source.groupId)) {
      ungrouped.push(source)
    } else {
      const bucket = buckets.get(source.groupId)
      if (bucket) {
        bucket.push(source)
      } else {
        buckets.set(source.groupId, [source])
      }
    }
  }

  const sortedGroupIds = (schedulerGroups ?? [])
    .slice()
    .sort((a, b) => a.sortIndex - b.sortIndex)
    .map((g) => g.id)

  const groups: SourceProfileGroup[] = sortedGroupIds
    .filter((id) => buckets.has(id))
    .map((id) => ({
      key: `group:${id}`,
      label: groupMap.get(id)!.name,
      sources: buckets.get(id)!,
    }))
  if (ungrouped.length > 0) {
    groups.push({ key: 'group:__ungrouped__', label: 'Ungrouped', sources: ungrouped })
  }
  return groups
}

// -- Group reordering --

export interface GroupSortSwap {
  groupA: SchedulerGroup
  groupB: SchedulerGroup
}

export function swapGroupSortIndex(
  schedulerGroups: SchedulerGroup[],
  displayedGroupKeys: string[],
  groupKey: string,
  direction: 'up' | 'down',
): GroupSortSwap | undefined {
  const displayIndex = displayedGroupKeys.indexOf(groupKey)
  if (displayIndex < 0) return undefined

  const neighborDisplayIndex = direction === 'up' ? displayIndex - 1 : displayIndex + 1
  if (neighborDisplayIndex < 0 || neighborDisplayIndex >= displayedGroupKeys.length) return undefined

  const neighborKey = displayedGroupKeys[neighborDisplayIndex]

  const groupId = groupKey.replace(/^group:/, '')
  const neighborId = neighborKey.replace(/^group:/, '')

  const group = schedulerGroups.find((g) => g.id === groupId)
  const neighbor = schedulerGroups.find((g) => g.id === neighborId)
  if (!group || !neighbor) return undefined

  let groupSortIndex = group.sortIndex
  let neighborSortIndex = neighbor.sortIndex
  if (groupSortIndex === neighborSortIndex) {
    groupSortIndex = displayIndex
    neighborSortIndex = neighborDisplayIndex
  }

  return {
    groupA: { ...group, sortIndex: neighborSortIndex },
    groupB: { ...neighbor, sortIndex: groupSortIndex },
  }
}

// -- Sorting --

export type SortMode = 'name-asc' | 'name-desc' | 'date-added' | 'last-synced'

export function sortSourcesInGroups(
  groups: SourceProfileGroup[],
  sortMode: SortMode,
): SourceProfileGroup[] {
  const comparator = sortComparator(sortMode)
  return groups.map((group) => ({
    ...group,
    sources: [...group.sources].sort(comparator),
  }))
}

function nameSortKey(source: SourceProfile): string {
  return formatSourceHandleLabel(source.handle).toLowerCase()
}

function sortComparator(mode: SortMode): (a: SourceProfile, b: SourceProfile) => number {
  switch (mode) {
    case 'name-asc':
      return (a, b) => nameSortKey(a).localeCompare(nameSortKey(b))
    case 'name-desc':
      return (a, b) => nameSortKey(b).localeCompare(nameSortKey(a))
    case 'date-added':
      return (a, b) => compareOptionalDatesDesc(a.createdAt, b.createdAt) || nameSortKey(a).localeCompare(nameSortKey(b))
    case 'last-synced':
      return (a, b) => compareOptionalDatesDesc(a.lastSyncedAt, b.lastSyncedAt) || nameSortKey(a).localeCompare(nameSortKey(b))
  }
}

function compareOptionalDatesDesc(a?: string, b?: string): number {
  if (!a && !b) return 0
  if (!a) return 1
  if (!b) return -1
  return b.localeCompare(a)
}

export function parseClipboardProfileSeed(rawText: string): ClipboardProfileSeed | undefined {
  const text = rawText.trim()
  if (text.length === 0) {
    return undefined
  }

  let parsedUrl: URL
  try {
    parsedUrl = new URL(text)
  } catch {
    return undefined
  }

  const host = parsedUrl.hostname.replace(/^www\./, '').toLowerCase()
  const segments = parsedUrl.pathname
    .split('/')
    .map((segment) => segment.trim())
    .filter((segment) => segment.length > 0)

  const singleHandle = segments[0]

  if (host === 'instagram.com' && singleHandle) {
    return createClipboardSeed('instagram', normalizeAtHandle(singleHandle))
  }

  if (host === 'tiktok.com' && singleHandle?.startsWith('@')) {
    return createClipboardSeed('tiktok', normalizeAtHandle(singleHandle))
  }

  if ((host === 'x.com' || host === 'twitter.com') && singleHandle) {
    return createClipboardSeed('twitter', normalizeAtHandle(singleHandle))
  }

  return undefined
}

/** Retorna o diretório-pai de um path de salvamento (remove o último segmento,
 * que costuma ser o handle), para agrupar/filtrar perfis pela pasta-base. */
export function mediaPathBaseDir(path: string): string {
  const normalized = path.replace(/[\\/]+$/, '')
  const index = Math.max(normalized.lastIndexOf('\\'), normalized.lastIndexOf('/'))
  return index > 0 ? normalized.slice(0, index) : normalized
}

export function buildSourceProfileUrl(source: Pick<SourceProfile, 'provider' | 'handle'>): string | undefined {
  const handle = source.handle.trim()
  if (handle.length === 0) {
    return undefined
  }

  switch (source.provider) {
    case 'instagram':
      return `https://www.instagram.com/${normalizeAtHandle(handle).slice(1)}/`
    case 'tiktok':
      return `https://www.tiktok.com/${normalizeAtHandle(handle)}/`
    case 'twitter':
      return `https://x.com/${normalizeAtHandle(handle).slice(1)}`
    default:
      return undefined
  }
}

/**
 * Chave canônica de deduplicação de perfis, espelhando `source_dedupe_key` do
 * backend (Rust): remove `@` líder e `/` das bordas e normaliza para minúsculas.
 * Handles são case-insensitive nas plataformas suportadas; TikTok preserva o `@`.
 */
export function sourceDedupeKey(provider: string, handle: string): string {
  const trimmed = handle.trim().replace(/^\/+|\/+$/g, '')
  const withoutAt = trimmed.startsWith('@') ? trimmed.slice(1) : trimmed
  const canonical = provider === 'tiktok' ? `@${withoutAt}` : withoutAt
  return canonical.toLowerCase()
}

/**
 * Retorna um perfil já existente cujo handle normalizado colide com o informado
 * (mesmo provider), ignorando o próprio `selfId`. Usado para bloquear duplicatas
 * antes de enviar o upsert ao backend.
 */
export function findDuplicateSource(
  sources: SourceProfile[],
  provider: string,
  handle: string,
  selfId?: string,
): SourceProfile | undefined {
  const key = sourceDedupeKey(provider, handle)
  if (key.length === 0) {
    return undefined
  }

  return sources.find(
    (entry) =>
      entry.provider === provider &&
      entry.id !== selfId &&
      sourceDedupeKey(entry.provider, entry.handle) === key,
  )
}

function createClipboardSeed(provider: ProviderKey, handle: string): ClipboardProfileSeed {
  return {
    provider,
    handle,
    displayName: formatSourceHandleLabel(handle).replace(/^u\//, ''),
  }
}

function normalizeAtHandle(value: string): string {
  return value.startsWith('@') ? value : `@${value}`
}

