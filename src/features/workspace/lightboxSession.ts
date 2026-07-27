import { useCallback, useMemo, useState } from 'react'

/**
 * Shared headless "lightbox media session" — navigation model used by both
 * ProfileViewPage and SingleVideosPage so post/slide stepping, group
 * derivation and shrink-clamping only live in one place.
 *
 * A "group" is a contiguous run of flat items sharing the same group key
 * (e.g. one carousel/post = one group; a standalone file is its own
 * single-item group). Navigation moves either between groups (`stepPost`,
 * bound to ↑/↓) or within the active group (`stepSlide`, bound to ←/→).
 */

/** Contiguous [start, end] range on the flat item list sharing one group key. */
interface LightboxGroup {
  key: string
  start: number
  end: number
}

export interface LightboxSessionInput<T> {
  /** Flat, ordered list of navigable items (e.g. one entry per media file). */
  items: T[]
  /**
   * Returns the group key for an item at a given flat index. Items with the
   * same key that are adjacent in `items` are merged into one group.
   * Should be stable (e.g. wrapped in `useCallback` by the caller) to avoid
   * recomputing groups on every render.
   */
  groupKeyFor: (item: T, index: number) => string
}

export interface LightboxSessionActive<T> {
  item: T
  index: number
  /** Whether there is a previous/next group (post/carousel) to step to. */
  hasPrev: boolean
  hasNext: boolean
  /** Whether there is a previous/next slide within the active group. */
  hasSlidePrev: boolean
  hasSlideNext: boolean
  /** 0-based position of the active item within its group. */
  slideIndex: number
  /** Total number of items in the active group. */
  slideCount: number
}

export interface LightboxSessionResult<T> {
  /** Currently open flat index, or `undefined` when the lightbox is closed. */
  index: number | undefined
  /** Opens the lightbox at `index`, clamped into range; no-op when items is empty. */
  open: (index: number) => void
  /** Closes the lightbox. */
  close: () => void
  /** Moves to the adjacent group's first item (↑/↓). Clamped at the first/last group (no wrap). */
  stepPost: (delta: number) => void
  /** Moves within the active group's bounds (←/→). Clamped at the group's edges (no wrap). */
  stepSlide: (delta: number) => void
  /** Derived info about the active item, or `undefined` when closed. */
  active: LightboxSessionActive<T> | undefined
}

function findGroupIndexForFlatIndex(groups: LightboxGroup[], flatIndex: number): number {
  for (let i = 0; i < groups.length; i++) {
    const group = groups[i]!
    if (flatIndex >= group.start && flatIndex <= group.end) return i
  }
  return -1
}

export function useLightboxSession<T>(input: LightboxSessionInput<T>): LightboxSessionResult<T> {
  const { items, groupKeyFor } = input
  const [index, setIndex] = useState<number | undefined>(undefined)

  const groups = useMemo<LightboxGroup[]>(() => {
    const result: LightboxGroup[] = []
    for (let i = 0; i < items.length; i++) {
      const key = groupKeyFor(items[i]!, i)
      const last = result[result.length - 1]
      if (last && last.key === key) {
        last.end = i
      } else {
        result.push({ key, start: i, end: i })
      }
    }
    return result
  }, [items, groupKeyFor])

  const open = useCallback(
    (target: number) => {
      setIndex(() => {
        if (items.length === 0) return undefined
        return Math.min(Math.max(target, 0), items.length - 1)
      })
    },
    [items.length],
  )

  const close = useCallback(() => setIndex(undefined), [])

  const stepPost = useCallback(
    (delta: number) => {
      setIndex((current) => {
        if (current === undefined || groups.length === 0) return current
        // The list may have shrunk since the index was set: re-anchor first.
        const flat = Math.min(current, items.length - 1)
        const groupPos = findGroupIndexForFlatIndex(groups, flat)
        if (groupPos < 0) return current
        const nextPos = groupPos + delta
        if (nextPos < 0 || nextPos >= groups.length) return flat
        return groups[nextPos]!.start
      })
    },
    [groups, items.length],
  )

  const stepSlide = useCallback(
    (delta: number) => {
      setIndex((current) => {
        if (current === undefined || items.length === 0) return current
        const flat = Math.min(current, items.length - 1)
        const groupPos = findGroupIndexForFlatIndex(groups, flat)
        if (groupPos < 0) return current
        const group = groups[groupPos]!
        if (group.start === group.end) return flat
        const next = flat + delta
        if (next < group.start || next > group.end) return flat
        return next
      })
    },
    [groups, items.length],
  )

  // Items may shrink (delete/refresh) while open: expose a clamped view of the
  // index (derived, not synced via an effect — avoids cascading renders).
  // Steppers re-anchor the raw index on their next call.
  const clampedIndex = useMemo<number | undefined>(() => {
    if (index === undefined || items.length === 0) return undefined
    return Math.min(index, items.length - 1)
  }, [index, items.length])

  const active = useMemo<LightboxSessionActive<T> | undefined>(() => {
    if (clampedIndex === undefined) return undefined
    const item = items[clampedIndex]
    if (!item) return undefined
    const groupPos = findGroupIndexForFlatIndex(groups, clampedIndex)
    const group = groupPos >= 0 ? groups[groupPos] : undefined
    const slideIndex = group ? clampedIndex - group.start : 0
    const slideCount = group ? group.end - group.start + 1 : 1
    return {
      item,
      index: clampedIndex,
      hasPrev: groupPos > 0,
      hasNext: groupPos >= 0 && groupPos < groups.length - 1,
      hasSlidePrev: slideCount > 1 && slideIndex > 0,
      hasSlideNext: slideCount > 1 && slideIndex < slideCount - 1,
      slideIndex,
      slideCount,
    }
  }, [clampedIndex, items, groups])

  return { index: clampedIndex, open, close, stepPost, stepSlide, active }
}

// ---------------------------------------------------------------------------
// Persisted media prefs (volume/mute) — shared across posts and sessions so
// the lightbox player doesn't reset to full volume every time it hydrates a
// new file.
// ---------------------------------------------------------------------------

const MEDIA_PREFS_STORAGE_KEY = 'lightbox.mediaPrefs'

export interface LightboxMediaPrefs {
  volume: number
  muted: boolean
}

const DEFAULT_MEDIA_PREFS: LightboxMediaPrefs = { volume: 1, muted: false }

function clampVolume(value: number): number {
  if (!Number.isFinite(value)) return DEFAULT_MEDIA_PREFS.volume
  return Math.min(1, Math.max(0, value))
}

/** Reads persisted volume/mute prefs; defaults + clamps when missing or corrupt. */
export function getStoredLightboxMediaPrefs(): LightboxMediaPrefs {
  try {
    const raw = localStorage.getItem(MEDIA_PREFS_STORAGE_KEY)
    if (!raw) return { ...DEFAULT_MEDIA_PREFS }
    const parsed = JSON.parse(raw) as Partial<LightboxMediaPrefs> | null
    if (!parsed || typeof parsed !== 'object') return { ...DEFAULT_MEDIA_PREFS }
    return {
      volume: typeof parsed.volume === 'number' ? clampVolume(parsed.volume) : DEFAULT_MEDIA_PREFS.volume,
      muted: typeof parsed.muted === 'boolean' ? parsed.muted : DEFAULT_MEDIA_PREFS.muted,
    }
  } catch {
    return { ...DEFAULT_MEDIA_PREFS }
  }
}

/** Persists volume/mute prefs; silently no-ops on storage errors (quota, private mode, etc.). */
export function setStoredLightboxMediaPrefs(prefs: LightboxMediaPrefs): void {
  try {
    const safe: LightboxMediaPrefs = { volume: clampVolume(prefs.volume), muted: !!prefs.muted }
    localStorage.setItem(MEDIA_PREFS_STORAGE_KEY, JSON.stringify(safe))
  } catch {
    // Best-effort persistence only.
  }
}

/** Applies volume/mute prefs to a `<video>`/`<audio>` element. */
export function applyLightboxMediaPrefs(el: HTMLMediaElement, prefs: LightboxMediaPrefs): void {
  el.volume = clampVolume(prefs.volume)
  el.muted = !!prefs.muted
}

/** Reads the current volume/mute state off a media element as prefs (e.g. on `volumechange`). */
export function readPrefsFromElement(el: HTMLMediaElement): LightboxMediaPrefs {
  return { volume: clampVolume(el.volume), muted: !!el.muted }
}
