import { describe, expect, it } from 'vitest'
import type { SourceProfile } from '../../domain/models'
import { resolveSyncSectionChips, summarizeEnabledSections } from './profileSyncSections'

function buildSource(overrides: Partial<SourceProfile>): SourceProfile {
  return {
    id: 'source-1',
    provider: 'instagram',
    sourceKind: 'profile',
    handle: '@visual_lab',
    displayName: 'visual_lab',
    labels: [],
    readyForDownload: true,
    remoteState: 'exists',
    isSubscription: false,
    profileImageCustom: false,
    ...overrides,
  }
}

describe('resolveSyncSectionChips', () => {
  it('reflects Instagram defaults (timeline on, rest off) when syncOptions is absent', () => {
    const chips = resolveSyncSectionChips(buildSource({ provider: 'instagram' }))
    expect(chips.map((chip) => chip.code)).toEqual(['TL', 'RE', 'ST', 'SU', 'TG'])
    expect(chips.find((chip) => chip.code === 'TL')?.enabled).toBe(true)
    expect(chips.filter((chip) => chip.enabled).map((chip) => chip.code)).toEqual(['TL'])
  })

  it('reflects per-source Instagram overrides', () => {
    const chips = resolveSyncSectionChips(
      buildSource({
        provider: 'instagram',
        syncOptions: { instagram: { reels: true, stories: true } as never },
      }),
    )
    expect(chips.filter((chip) => chip.enabled).map((chip) => chip.code)).toEqual(['TL', 'RE', 'ST'])
  })

  it('maps TikTok sections in fixed order', () => {
    const chips = resolveSyncSectionChips(buildSource({ provider: 'tiktok' }))
    expect(chips.map((chip) => chip.code)).toEqual(['TL', 'US', 'RP', 'LK'])
  })

  it('maps Twitter models in fixed order', () => {
    const chips = resolveSyncSectionChips(buildSource({ provider: 'twitter' }))
    expect(chips.map((chip) => chip.code)).toEqual(['MD', 'PR', 'SE', 'LK'])
    // Media model é o default ligado do provider.
    expect(chips.find((chip) => chip.code === 'MD')?.enabled).toBe(true)
  })
})

describe('summarizeEnabledSections', () => {
  it('lists enabled section labels', () => {
    const chips = resolveSyncSectionChips(
      buildSource({ provider: 'instagram', syncOptions: { instagram: { reels: true } as never } }),
    )
    expect(summarizeEnabledSections(chips)).toBe('Sync sections: Timeline, Reels')
  })

  it('warns when nothing is enabled', () => {
    expect(summarizeEnabledSections([{ code: 'TL', label: 'Timeline', enabled: false }])).toBe(
      'No sync sections enabled',
    )
  })
})
