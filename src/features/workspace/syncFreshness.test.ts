import { describe, expect, it } from 'vitest'
import { computeSyncFreshness } from './syncFreshness'

const NOW = Date.parse('2026-07-03T12:00:00Z')
const HOUR = 3_600_000
const DAY = 86_400_000

function ago(ms: number): string {
  return new Date(NOW - ms).toISOString()
}

describe('computeSyncFreshness', () => {
  it('returns null for profiles synced within the stale window', () => {
    expect(computeSyncFreshness(ago(2 * HOUR), NOW)).toBeNull()
    expect(computeSyncFreshness(ago(23 * HOUR), NOW)).toBeNull()
  })

  it('treats a missing or unparseable timestamp as never synced', () => {
    expect(computeSyncFreshness(undefined, NOW)).toEqual({
      tier: 'never',
      shortLabel: 'Never',
      longLabel: 'Never synced',
    })
    expect(computeSyncFreshness('not-a-date', NOW)?.tier).toBe('never')
  })

  it('flags the stale tier between 24h and 7 days', () => {
    const result = computeSyncFreshness(ago(3 * DAY), NOW)
    expect(result?.tier).toBe('stale')
    expect(result?.shortLabel).toBe('3d')
    expect(result?.longLabel).toBe('Last synced 3 days ago')
  })

  it('flags the old tier between 7 and 30 days with a week label', () => {
    const result = computeSyncFreshness(ago(10 * DAY), NOW)
    expect(result?.tier).toBe('old')
    expect(result?.shortLabel).toBe('1w')
    expect(result?.longLabel).toBe('Last synced 1 week ago')
  })

  it('flags the ancient tier past 30 days with a month label', () => {
    const result = computeSyncFreshness(ago(65 * DAY), NOW)
    expect(result?.tier).toBe('ancient')
    expect(result?.shortLabel).toBe('2mo')
    expect(result?.longLabel).toBe('Last synced 2 months ago')
  })

  it('uses a year label for very old syncs', () => {
    const result = computeSyncFreshness(ago(400 * DAY), NOW)
    expect(result?.tier).toBe('ancient')
    expect(result?.shortLabel).toBe('1y')
    expect(result?.longLabel).toBe('Last synced 1 year ago')
  })

  it('handles boundaries and future timestamps gracefully', () => {
    expect(computeSyncFreshness(ago(24 * HOUR), NOW)?.tier).toBe('stale')
    expect(computeSyncFreshness(ago(7 * DAY), NOW)?.tier).toBe('old')
    expect(computeSyncFreshness(ago(30 * DAY), NOW)?.tier).toBe('ancient')
    // Clock skew (sync timestamp in the future) counts as fresh.
    expect(computeSyncFreshness(ago(-HOUR), NOW)).toBeNull()
  })
})
