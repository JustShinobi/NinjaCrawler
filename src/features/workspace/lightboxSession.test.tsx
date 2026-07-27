// @vitest-environment jsdom

import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import {
  applyLightboxMediaPrefs,
  getStoredLightboxMediaPrefs,
  readPrefsFromElement,
  setStoredLightboxMediaPrefs,
  useLightboxSession,
} from './lightboxSession'

interface Item {
  postId: string
  slide: number
}

/** postId:N with N>1 files behaves like a carousel; postId:1 is a single-file post. */
function makeItems(postSizes: number[]): Item[] {
  const items: Item[] = []
  postSizes.forEach((size, postIndex) => {
    for (let slide = 0; slide < size; slide++) {
      items.push({ postId: `p${postIndex}`, slide })
    }
  })
  return items
}

const groupKeyFor = (item: Item) => item.postId

describe('useLightboxSession', () => {
  describe('group derivation', () => {
    it('groups multi-slide carousels and single-file posts from mixed keys', () => {
      // p0: 1 file, p1: 3 files (carousel), p2: 1 file, p3: 2 files (carousel)
      const items = makeItems([1, 3, 1, 2])
      const { result } = renderHook(() => useLightboxSession({ items, groupKeyFor }))

      act(() => result.current.open(0))
      expect(result.current.active?.slideCount).toBe(1)

      act(() => result.current.open(1))
      expect(result.current.active?.slideCount).toBe(3)
      expect(result.current.active?.slideIndex).toBe(0)

      act(() => result.current.open(4))
      expect(result.current.active?.slideCount).toBe(1)

      act(() => result.current.open(5))
      expect(result.current.active?.slideCount).toBe(2)
      expect(result.current.active?.slideIndex).toBe(0)
    })
  })

  describe('open', () => {
    it('opens at the given index', () => {
      const items = makeItems([1, 2])
      const { result } = renderHook(() => useLightboxSession({ items, groupKeyFor }))

      act(() => result.current.open(2))
      expect(result.current.index).toBe(2)
    })

    it('clamps an out-of-range index into bounds', () => {
      const items = makeItems([1, 2])
      const { result } = renderHook(() => useLightboxSession({ items, groupKeyFor }))

      act(() => result.current.open(99))
      expect(result.current.index).toBe(items.length - 1)

      act(() => result.current.open(-5))
      expect(result.current.index).toBe(0)
    })

    it('is a no-op (stays closed) when items is empty', () => {
      const { result } = renderHook(() => useLightboxSession({ items: [] as Item[], groupKeyFor }))

      act(() => result.current.open(0))
      expect(result.current.index).toBeUndefined()
    })
  })

  describe('close', () => {
    it('closes the session', () => {
      const items = makeItems([1])
      const { result } = renderHook(() => useLightboxSession({ items, groupKeyFor }))

      act(() => result.current.open(0))
      expect(result.current.index).toBe(0)

      act(() => result.current.close())
      expect(result.current.index).toBeUndefined()
    })
  })

  describe('stepPost', () => {
    it('moves to the adjacent group start and clamps at the last group', () => {
      const items = makeItems([1, 3, 2]) // groups: [0], [1,2,3], [4,5]
      const { result } = renderHook(() => useLightboxSession({ items, groupKeyFor }))

      act(() => result.current.open(0))
      act(() => result.current.stepPost(1))
      expect(result.current.index).toBe(1) // start of group 1

      act(() => result.current.stepPost(1))
      expect(result.current.index).toBe(4) // start of group 2

      act(() => result.current.stepPost(1)) // already last group: clamp, no move
      expect(result.current.index).toBe(4)
    })

    it('clamps at the first group', () => {
      const items = makeItems([1, 3, 2])
      const { result } = renderHook(() => useLightboxSession({ items, groupKeyFor }))

      act(() => result.current.open(2)) // inside group 1
      act(() => result.current.stepPost(-1))
      expect(result.current.index).toBe(0) // start of group 0

      act(() => result.current.stepPost(-1)) // already first group: clamp, no move
      expect(result.current.index).toBe(0)
    })
  })

  describe('stepSlide', () => {
    it('clamps within the active group bounds', () => {
      const items = makeItems([1, 3, 2]) // groups: [0], [1,2,3], [4,5]
      const { result } = renderHook(() => useLightboxSession({ items, groupKeyFor }))

      act(() => result.current.open(1)) // group [1,3], start
      act(() => result.current.stepSlide(-1)) // already at group start: no move
      expect(result.current.index).toBe(1)

      act(() => result.current.stepSlide(1))
      expect(result.current.index).toBe(2)

      act(() => result.current.stepSlide(1))
      expect(result.current.index).toBe(3)

      act(() => result.current.stepSlide(1)) // already at group end: no move
      expect(result.current.index).toBe(3)
    })

    it('does not move within a single-item group', () => {
      const items = makeItems([1, 3])
      const { result } = renderHook(() => useLightboxSession({ items, groupKeyFor }))

      act(() => result.current.open(0))
      act(() => result.current.stepSlide(1))
      expect(result.current.index).toBe(0)
    })
  })

  describe('active flags', () => {
    it('reports flags for the first position (first post, first slide)', () => {
      const items = makeItems([1, 3, 2])
      const { result } = renderHook(() => useLightboxSession({ items, groupKeyFor }))

      act(() => result.current.open(0))
      const active = result.current.active!
      expect(active.hasPrev).toBe(false)
      expect(active.hasNext).toBe(true)
      expect(active.hasSlidePrev).toBe(false)
      expect(active.hasSlideNext).toBe(false)
      expect(active.slideIndex).toBe(0)
      expect(active.slideCount).toBe(1)
    })

    it('reports flags for a middle slide of a middle group', () => {
      const items = makeItems([1, 3, 2]) // group 1 = indices [1,2,3]
      const { result } = renderHook(() => useLightboxSession({ items, groupKeyFor }))

      act(() => result.current.open(2)) // middle slide of group 1
      const active = result.current.active!
      expect(active.hasPrev).toBe(true)
      expect(active.hasNext).toBe(true)
      expect(active.hasSlidePrev).toBe(true)
      expect(active.hasSlideNext).toBe(true)
      expect(active.slideIndex).toBe(1)
      expect(active.slideCount).toBe(3)
    })

    it('reports flags for the last position (last post, last slide)', () => {
      const items = makeItems([1, 3, 2]) // group 2 = indices [4,5]
      const { result } = renderHook(() => useLightboxSession({ items, groupKeyFor }))

      act(() => result.current.open(5))
      const active = result.current.active!
      expect(active.hasPrev).toBe(true)
      expect(active.hasNext).toBe(false)
      expect(active.hasSlidePrev).toBe(true)
      expect(active.hasSlideNext).toBe(false)
      expect(active.slideIndex).toBe(1)
      expect(active.slideCount).toBe(2)
    })

    it('is undefined when the session is closed', () => {
      const items = makeItems([1])
      const { result } = renderHook(() => useLightboxSession({ items, groupKeyFor }))
      expect(result.current.active).toBeUndefined()
    })
  })

  describe('clamp on shrink', () => {
    it('clamps the index to the last item when the list shrinks below it', () => {
      const items = makeItems([1, 2, 1]) // 4 items
      const { result, rerender } = renderHook(
        ({ items }) => useLightboxSession({ items, groupKeyFor }),
        { initialProps: { items } },
      )

      act(() => result.current.open(3))
      expect(result.current.index).toBe(3)

      const shrunk = items.slice(0, 2)
      rerender({ items: shrunk })
      expect(result.current.index).toBe(1)
    })

    it('closes when the list becomes empty', () => {
      const items = makeItems([1, 2])
      const { result, rerender } = renderHook(
        ({ items }) => useLightboxSession({ items, groupKeyFor }),
        { initialProps: { items } },
      )

      act(() => result.current.open(1))
      expect(result.current.index).toBe(1)

      rerender({ items: [] })
      expect(result.current.index).toBeUndefined()
    })

    it('leaves the index untouched when it still fits', () => {
      const items = makeItems([1, 2, 1])
      const { result, rerender } = renderHook(
        ({ items }) => useLightboxSession({ items, groupKeyFor }),
        { initialProps: { items } },
      )

      act(() => result.current.open(1))
      rerender({ items: items.slice(0, 3) })
      expect(result.current.index).toBe(1)
    })
  })
})

describe('lightbox media prefs', () => {
  beforeEach(() => {
    localStorage.clear()
  })

  afterEach(() => {
    localStorage.clear()
  })

  it('returns defaults when nothing is stored', () => {
    expect(getStoredLightboxMediaPrefs()).toEqual({ volume: 1, muted: false })
  })

  it('returns defaults when the stored value is corrupt JSON', () => {
    localStorage.setItem('lightbox.mediaPrefs', '{not json')
    expect(getStoredLightboxMediaPrefs()).toEqual({ volume: 1, muted: false })
  })

  it('returns defaults when the stored value is not an object', () => {
    localStorage.setItem('lightbox.mediaPrefs', '"oops"')
    expect(getStoredLightboxMediaPrefs()).toEqual({ volume: 1, muted: false })
  })

  it('round-trips a stored value', () => {
    setStoredLightboxMediaPrefs({ volume: 0.42, muted: true })
    expect(getStoredLightboxMediaPrefs()).toEqual({ volume: 0.42, muted: true })
  })

  it('clamps volume into [0, 1] on write and on read', () => {
    setStoredLightboxMediaPrefs({ volume: 2.5, muted: false })
    expect(getStoredLightboxMediaPrefs().volume).toBe(1)

    setStoredLightboxMediaPrefs({ volume: -1, muted: false })
    expect(getStoredLightboxMediaPrefs().volume).toBe(0)

    localStorage.setItem('lightbox.mediaPrefs', JSON.stringify({ volume: 5, muted: false }))
    expect(getStoredLightboxMediaPrefs().volume).toBe(1)
  })

  it('falls back to defaults for partial/malformed fields', () => {
    localStorage.setItem('lightbox.mediaPrefs', JSON.stringify({ muted: true }))
    expect(getStoredLightboxMediaPrefs()).toEqual({ volume: 1, muted: true })

    localStorage.setItem('lightbox.mediaPrefs', JSON.stringify({ volume: 0.3, muted: 'yes' }))
    expect(getStoredLightboxMediaPrefs()).toEqual({ volume: 0.3, muted: false })
  })

  it('applyLightboxMediaPrefs sets volume/muted on a media element', () => {
    const el = document.createElement('video')
    applyLightboxMediaPrefs(el, { volume: 0.7, muted: true })
    expect(el.volume).toBe(0.7)
    expect(el.muted).toBe(true)
  })

  it('readPrefsFromElement reads volume/muted back off a media element', () => {
    const el = document.createElement('audio')
    el.volume = 0.25
    el.muted = true
    expect(readPrefsFromElement(el)).toEqual({ volume: 0.25, muted: true })
  })
})
