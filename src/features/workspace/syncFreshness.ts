const MS_HOUR = 3_600_000
const MS_DAY = 86_400_000

/** A profile is considered stale once its last sync is older than this. */
export const SYNC_STALE_AFTER_HOURS = 24

const STALE_MS = SYNC_STALE_AFTER_HOURS * MS_HOUR
const OLD_MS = 7 * MS_DAY
const ANCIENT_MS = 30 * MS_DAY

/**
 * Gradual staleness tiers, from least to most overdue. `never` covers profiles
 * that have never completed a sync (no `lastSyncedAt`).
 */
export type SyncFreshnessTier = 'stale' | 'old' | 'ancient' | 'never'

export interface SyncFreshness {
  tier: SyncFreshnessTier
  /** Compact badge label, e.g. `3d`, `2w`, `Never`. */
  shortLabel: string
  /** Descriptive tooltip label, e.g. `Last synced 3 days ago`. */
  longLabel: string
}

/**
 * Classifies how overdue a profile's last sync is. Returns `null` for profiles
 * synced within the stale window (they need no marker); a `never` freshness for
 * profiles that have never synced.
 */
export function computeSyncFreshness(
  lastSyncedAt: string | undefined,
  now: number,
): SyncFreshness | null {
  const timestamp = lastSyncedAt ? Date.parse(lastSyncedAt) : Number.NaN
  if (Number.isNaN(timestamp)) {
    return { tier: 'never', shortLabel: 'Never', longLabel: 'Never synced' }
  }

  const age = now - timestamp
  if (age < STALE_MS) {
    return null
  }

  const tier: SyncFreshnessTier = age >= ANCIENT_MS ? 'ancient' : age >= OLD_MS ? 'old' : 'stale'
  return {
    tier,
    shortLabel: formatCompactAge(age),
    longLabel: `Last synced ${formatHumanAge(age)} ago`,
  }
}

function formatCompactAge(age: number): string {
  const days = Math.floor(age / MS_DAY)
  if (days < 7) return `${Math.max(days, 1)}d`
  if (days < 30) return `${Math.floor(days / 7)}w`
  if (days < 365) return `${Math.floor(days / 30)}mo`
  return `${Math.floor(days / 365)}y`
}

function formatHumanAge(age: number): string {
  const days = Math.floor(age / MS_DAY)
  if (days < 1) return pluralize(Math.max(Math.floor(age / MS_HOUR), 1), 'hour')
  if (days < 7) return pluralize(days, 'day')
  if (days < 30) return pluralize(Math.floor(days / 7), 'week')
  if (days < 365) return pluralize(Math.floor(days / 30), 'month')
  return pluralize(Math.floor(days / 365), 'year')
}

function pluralize(value: number, unit: string): string {
  return `${value} ${unit}${value === 1 ? '' : 's'}`
}
