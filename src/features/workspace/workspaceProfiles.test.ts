import { describe, expect, it } from 'vitest'
import { DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS } from '../../domain/sourceSyncOptions'
import type { SchedulerGroup, SchedulerPlanCriteria, SourceProfile } from '../../domain/models'
import {
  buildSourceProfileUrl,
  filterSourcesForWorkspace,
  mediaPathBaseDir,
  findDuplicateSource,
  sortSourcesInGroups,
  sourceDedupeKey,
  swapGroupSortIndex,
  type SourceProfileGroup,
} from './workspaceProfiles'

function emptyCriteria(): SchedulerPlanCriteria {
  return {
    regular: false, temporary: false, favorite: false,
    readyForDownload: false, ignoreReadyForDownload: false,
    downloadUsers: false, downloadSubscriptions: false,
    userExists: false, userSuspended: false, userDeleted: false,
    labelsNo: false, labelsIncluded: [], labelsExcluded: [], ignoreExcludedLabels: false,
    sitesIncluded: [], sitesExcluded: [],
    groupIdsIncluded: [], groupIdsExcluded: [], groupsOnly: false,
    daysIsDownloaded: false, dateInRange: true,
  }
}

function makeGroup(id: string, name: string, sortIndex: number): SchedulerGroup {
  return { id, name, sortIndex, criteria: emptyCriteria() }
}

function makeSource(overrides: Partial<SourceProfile> & Pick<SourceProfile, 'id' | 'handle'>): SourceProfile {
  return {
    id: overrides.id,
    provider: overrides.provider ?? 'instagram',
    sourceKind: overrides.sourceKind ?? 'profile',
    handle: overrides.handle,
    displayName: overrides.displayName ?? '',
    labels: overrides.labels ?? [],
    readyForDownload: overrides.readyForDownload ?? false,
    profileImageCustom: overrides.profileImageCustom ?? false,
    remoteState: overrides.remoteState ?? 'exists',
    isSubscription: overrides.isSubscription ?? false,
    accountId: overrides.accountId,
    groupId: overrides.groupId,
    syncOptions: overrides.syncOptions,
    profileImagePath: overrides.profileImagePath,
    lastSyncedAt: overrides.lastSyncedAt,
    createdAt: overrides.createdAt,
  }
}

describe('workspaceProfiles', () => {
  it('matches search against the profile bio/description', () => {
    const sources = [
      makeSource({
        id: 'a',
        handle: 'visual_lab',
        syncOptions: {
          instagram: { ...DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS, description: 'fotografia de paisagens em Lisboa' },
        },
      }),
      makeSource({
        id: 'b',
        handle: 'other_one',
        syncOptions: {
          instagram: { ...DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS, description: 'culinária vegana' },
        },
      }),
    ]

    const result = filterSourcesForWorkspace(sources, 'all', 'paisagens')
    expect(result.map((s) => s.id)).toEqual(['a'])
    // termo ausente da bio e do handle não retorna nada
    expect(filterSourcesForWorkspace(sources, 'all', 'inexistente')).toHaveLength(0)
  })

  it('matches search against previous handles (renamed profiles)', () => {
    const sources = [
      makeSource({
        id: 'a',
        handle: 'rinanyi_oficial',
        syncOptions: {
          instagram: { ...DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS, previousHandles: ['deleonrinanyi'] },
        },
      }),
    ]

    expect(filterSourcesForWorkspace(sources, 'all', 'deleonrinanyi').map((s) => s.id)).toEqual(['a'])
    expect(filterSourcesForWorkspace(sources, 'all', 'rinanyi_oficial').map((s) => s.id)).toEqual(['a'])
  })

  it('extracts the parent directory of a save path', () => {
    expect(mediaPathBaseDir('C:\\Users\\ninja\\Pictures\\NinjaCrawler\\handle')).toBe(
      'C:\\Users\\ninja\\Pictures\\NinjaCrawler',
    )
    expect(mediaPathBaseDir('F:\\SCrawler\\Data\\Instagram\\handle\\')).toBe('F:\\SCrawler\\Data\\Instagram')
    expect(mediaPathBaseDir('/media/instagram/handle')).toBe('/media/instagram')
  })

  it('normalizes dedupe keys across @ prefix and case', () => {
    expect(sourceDedupeKey('instagram', '@Poliana')).toBe(sourceDedupeKey('instagram', 'poliana'))
    expect(sourceDedupeKey('instagram', '  @Foo/ ')).toBe('foo')
    expect(sourceDedupeKey('tiktok', 'Bar')).toBe(sourceDedupeKey('tiktok', '@bar'))
  })

  it('detects duplicate sources ignoring @ prefix and self id', () => {
    const sources = [
      makeSource({ id: 'keep', handle: 'polianaarapiraca' }),
      makeSource({ id: 'other', handle: '@someoneelse' }),
      makeSource({ id: 'tt', provider: 'tiktok', handle: '@polianaarapiraca' }),
    ]

    // '@polianaarapiraca' colide com o registro existente sem '@'.
    expect(findDuplicateSource(sources, 'instagram', '@polianaarapiraca')?.id).toBe('keep')
    // O próprio registro nunca conflita consigo mesmo.
    expect(findDuplicateSource(sources, 'instagram', 'polianaarapiraca', 'keep')).toBeUndefined()
    // Provider diferente não conta como duplicata.
    expect(findDuplicateSource(sources, 'twitter', 'polianaarapiraca')).toBeUndefined()
    // Handle inédito não acusa conflito.
    expect(findDuplicateSource(sources, 'instagram', '@brand_new')).toBeUndefined()
  })

  it('builds provider profile URLs from source handles', () => {
    expect(buildSourceProfileUrl({ provider: 'instagram', handle: '@visual_lab' })).toBe('https://www.instagram.com/visual_lab/')
    expect(buildSourceProfileUrl({ provider: 'tiktok', handle: '@visual_lab' })).toBe('https://www.tiktok.com/@visual_lab/')
    expect(buildSourceProfileUrl({ provider: 'twitter', handle: 'visual_lab' })).toBe('https://x.com/visual_lab')
  })

  it('sorts name ascending by the visible handle instead of displayName metadata', () => {
    const groups: SourceProfileGroup[] = [{
      key: 'all',
      label: 'All',
      sources: [
        makeSource({ id: '2', handle: '@bravo', displayName: 'Zulu Display' }),
        makeSource({ id: '1', handle: '@alpha', displayName: 'Charlie Display' }),
      ],
    }]

    const [group] = sortSourcesInGroups(groups, 'name-asc')
    expect(group.sources.map((source) => source.handle)).toEqual(['@alpha', '@bravo'])
  })

  it('falls back to visible-name sorting when date sort values tie', () => {
    const groups: SourceProfileGroup[] = [{
      key: 'all',
      label: 'All',
      sources: [
        makeSource({ id: '2', handle: '@bravo', createdAt: '2026-03-18T10:00:00Z' }),
        makeSource({ id: '1', handle: '@alpha', createdAt: '2026-03-18T10:00:00Z' }),
      ],
    }]

    const [group] = sortSourcesInGroups(groups, 'date-added')
    expect(group.sources.map((source) => source.handle)).toEqual(['@alpha', '@bravo'])
  })
})

describe('swapGroupSortIndex', () => {
  it('swaps sortIndex between two groups with distinct values', () => {
    const groups = [makeGroup('fav', 'Favorito', 0), makeGroup('arq', 'Arquivo', 1)]
    const displayedKeys = ['group:fav', 'group:arq']
    const swap = swapGroupSortIndex(groups, displayedKeys, 'group:fav', 'down')
    expect(swap).toBeDefined()
    expect(swap!.groupA.id).toBe('fav')
    expect(swap!.groupA.sortIndex).toBe(1)
    expect(swap!.groupB.id).toBe('arq')
    expect(swap!.groupB.sortIndex).toBe(0)
  })

  it('assigns position-based sortIndex when all groups share the same value', () => {
    const groups = [makeGroup('a', 'A', 0), makeGroup('b', 'B', 0), makeGroup('c', 'C', 0)]
    const displayedKeys = ['group:a', 'group:b', 'group:c']
    const swap = swapGroupSortIndex(groups, displayedKeys, 'group:a', 'down')
    expect(swap).toBeDefined()
    expect(swap!.groupA.sortIndex).toBe(1)
    expect(swap!.groupB.sortIndex).toBe(0)
  })

  it('returns undefined when moving the first group up', () => {
    const groups = [makeGroup('a', 'A', 0), makeGroup('b', 'B', 1)]
    const displayedKeys = ['group:a', 'group:b']
    expect(swapGroupSortIndex(groups, displayedKeys, 'group:a', 'up')).toBeUndefined()
  })

  it('returns undefined when moving the last group down', () => {
    const groups = [makeGroup('a', 'A', 0), makeGroup('b', 'B', 1)]
    const displayedKeys = ['group:a', 'group:b']
    expect(swapGroupSortIndex(groups, displayedKeys, 'group:b', 'down')).toBeUndefined()
  })

  it('returns undefined when neighbor is ungrouped (not in schedulerGroups)', () => {
    const groups = [makeGroup('a', 'A', 0)]
    const displayedKeys = ['group:a', 'group:__ungrouped__']
    expect(swapGroupSortIndex(groups, displayedKeys, 'group:a', 'down')).toBeUndefined()
  })

  it('returns undefined for a key not in the displayed list', () => {
    const groups = [makeGroup('a', 'A', 0)]
    const displayedKeys = ['group:a']
    expect(swapGroupSortIndex(groups, displayedKeys, 'group:missing', 'down')).toBeUndefined()
  })

  it('correctly swaps when moving middle group up', () => {
    const groups = [makeGroup('a', 'A', 0), makeGroup('b', 'B', 1), makeGroup('c', 'C', 2)]
    const displayedKeys = ['group:a', 'group:b', 'group:c']
    const swap = swapGroupSortIndex(groups, displayedKeys, 'group:b', 'up')
    expect(swap).toBeDefined()
    expect(swap!.groupA.id).toBe('b')
    expect(swap!.groupA.sortIndex).toBe(0)
    expect(swap!.groupB.id).toBe('a')
    expect(swap!.groupB.sortIndex).toBe(1)
  })
})
