// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { MediaLightbox } from './MediaLightbox'
import { getStoredLightboxMediaPrefs, setStoredLightboxMediaPrefs } from './lightboxSession'

vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (path: string) => `asset://${path}` }))

function renderVideoLightbox(overrides: Partial<Parameters<typeof MediaLightbox>[0]> = {}) {
  const props = {
    fileAbsPath: 'S:/clip.mp4',
    isVideo: true,
    hasPrev: true,
    hasNext: true,
    onPrev: vi.fn(),
    onNext: vi.fn(),
    onClose: vi.fn(),
    ...overrides,
  }
  const result = render(<MediaLightbox {...props} />)
  const video = result.container.querySelector('video')
  if (!video) throw new Error('expected lightbox video')
  return { ...result, props, video }
}

function renderPhotoLightbox(overrides: Partial<Parameters<typeof MediaLightbox>[0]> = {}) {
  const props = {
    fileAbsPath: 'S:/photo.jpeg',
    isVideo: false,
    hasPrev: true,
    hasNext: true,
    onPrev: vi.fn(),
    onNext: vi.fn(),
    onClose: vi.fn(),
    ...overrides,
  }
  const result = render(<MediaLightbox {...props} />)
  const image = result.container.querySelector('img')
  if (!image) throw new Error('expected lightbox image')
  return { ...result, props, image }
}

function setVideoDuration(video: HTMLVideoElement, duration: number) {
  Object.defineProperty(video, 'duration', { configurable: true, value: duration })
}

function mockFullscreenApi(root: HTMLElement) {
  const requestFullscreen = vi.fn().mockResolvedValue(undefined)
  const exitFullscreen = vi.fn().mockResolvedValue(undefined)
  Object.defineProperty(root, 'requestFullscreen', {
    configurable: true,
    value: requestFullscreen,
  })
  Object.defineProperty(document, 'exitFullscreen', {
    configurable: true,
    value: exitFullscreen,
  })
  let fullscreenElement: Element | null = null
  Object.defineProperty(document, 'fullscreenElement', {
    configurable: true,
    get: () => fullscreenElement,
  })
  requestFullscreen.mockImplementation(() => {
    fullscreenElement = root
    return Promise.resolve()
  })
  exitFullscreen.mockImplementation(() => {
    fullscreenElement = null
    return Promise.resolve()
  })
  return {
    requestFullscreen,
    exitFullscreen,
    setFullscreenElement: (el: Element | null) => {
      fullscreenElement = el
    },
  }
}

describe('MediaLightbox', () => {
  afterEach(() => {
    cleanup()
    try {
      Object.defineProperty(document, 'fullscreenElement', {
        configurable: true,
        get: () => null,
      })
    } catch {
      // ignore
    }
  })

  it('uses vertical arrows to move between posts, not slides', () => {
    const { props } = renderVideoLightbox({
      hasSlidePrev: true,
      hasSlideNext: true,
      onSlidePrev: vi.fn(),
      onSlideNext: vi.fn(),
    })

    fireEvent.keyDown(document, { key: 'ArrowDown' })
    fireEvent.keyDown(document, { key: 'ArrowUp' })

    expect(props.onNext).toHaveBeenCalledTimes(1)
    expect(props.onPrev).toHaveBeenCalledTimes(1)
    expect(props.onSlideNext).not.toHaveBeenCalled()
    expect(props.onSlidePrev).not.toHaveBeenCalled()
  })

  it('focuses the dialog on mount so shortcuts work after opening from a button', async () => {
    render(<button type="button">Open preview</button>)
    screen.getByRole('button', { name: 'Open preview' }).focus()
    const { props } = renderVideoLightbox()
    const dialog = screen.getByRole('dialog')

    await waitFor(() => expect(document.activeElement).toBe(dialog))
    fireEvent.keyDown(dialog, { key: 'ArrowDown' })

    expect(props.onNext).toHaveBeenCalledTimes(1)
  })

  it('seeks short videos by one second with horizontal arrows and clamps to duration', () => {
    const { video, props } = renderVideoLightbox({
      hasSlideNext: true,
      onSlideNext: vi.fn(),
    })
    setVideoDuration(video, 10)

    video.currentTime = 5
    fireEvent.keyDown(document, { key: 'ArrowRight' })
    expect(video.currentTime).toBe(6)
    expect(props.onNext).not.toHaveBeenCalled()
    expect(props.onSlideNext).not.toHaveBeenCalled()

    video.currentTime = 9.75
    fireEvent.keyDown(document, { key: 'ArrowRight' })
    expect(video.currentTime).toBe(10)

    video.currentTime = 0.25
    fireEvent.keyDown(document, { key: 'ArrowLeft' })
    expect(video.currentTime).toBe(0)
    expect(props.onPrev).not.toHaveBeenCalled()
  })

  it('navigates carousel slides with horizontal arrows without moving posts', () => {
    const onSlidePrev = vi.fn()
    const onSlideNext = vi.fn()
    const { props } = renderPhotoLightbox({
      hasSlidePrev: true,
      hasSlideNext: true,
      onSlidePrev,
      onSlideNext,
    })

    fireEvent.keyDown(document, { key: 'ArrowRight' })
    fireEvent.keyDown(document, { key: 'ArrowLeft' })

    expect(onSlideNext).toHaveBeenCalledTimes(1)
    expect(onSlidePrev).toHaveBeenCalledTimes(1)
    expect(props.onNext).not.toHaveBeenCalled()
    expect(props.onPrev).not.toHaveBeenCalled()
  })

  it('keeps slide shortcuts working after media path changes (no stale keyboard state)', () => {
    const onSlideNext = vi.fn()
    const { rerender } = render(
      <MediaLightbox
        fileAbsPath="S:/a.jpeg"
        isVideo={false}
        hasPrev={false}
        hasNext={false}
        hasSlidePrev={false}
        hasSlideNext={true}
        onPrev={vi.fn()}
        onNext={vi.fn()}
        onClose={vi.fn()}
        onSlideNext={onSlideNext}
      />,
    )

    rerender(
      <MediaLightbox
        fileAbsPath="S:/b.jpeg"
        isVideo={false}
        hasPrev={false}
        hasNext={false}
        hasSlidePrev={true}
        hasSlideNext={true}
        onPrev={vi.fn()}
        onNext={vi.fn()}
        onClose={vi.fn()}
        onSlideNext={onSlideNext}
      />,
    )

    fireEvent.keyDown(document, { key: 'ArrowRight' })
    expect(onSlideNext).toHaveBeenCalledTimes(1)
  })

  it('does not use horizontal arrows for single photos without slides', () => {
    const { props } = renderPhotoLightbox()

    fireEvent.keyDown(document, { key: 'ArrowRight' })
    fireEvent.keyDown(document, { key: 'ArrowLeft' })

    expect(props.onNext).not.toHaveBeenCalled()
    expect(props.onPrev).not.toHaveBeenCalled()
  })

  it('prefers slide navigation on side buttons when a carousel is active', () => {
    const onSlidePrev = vi.fn()
    const onSlideNext = vi.fn()
    const { props } = renderPhotoLightbox({
      hasSlidePrev: true,
      hasSlideNext: true,
      onSlidePrev,
      onSlideNext,
    })

    fireEvent.click(screen.getByRole('button', { name: 'Next' }))
    fireEvent.click(screen.getByRole('button', { name: 'Previous' }))

    expect(onSlideNext).toHaveBeenCalledTimes(1)
    expect(onSlidePrev).toHaveBeenCalledTimes(1)
    expect(props.onNext).not.toHaveBeenCalled()
    expect(props.onPrev).not.toHaveBeenCalled()
  })

  it('toggles lightbox fullscreen with Enter for video and photo', () => {
    renderVideoLightbox()
    const videoDialog = screen.getByRole('dialog')
    const videoFs = mockFullscreenApi(videoDialog)

    fireEvent.keyDown(document, { key: 'Enter' })
    expect(videoFs.requestFullscreen).toHaveBeenCalledTimes(1)

    cleanup()

    renderPhotoLightbox()
    const photoDialog = screen.getByRole('dialog')
    const photoFs = mockFullscreenApi(photoDialog)

    fireEvent.keyDown(document, { key: 'Enter' })
    expect(photoFs.requestFullscreen).toHaveBeenCalledTimes(1)
  })

  it('exits fullscreen on first Escape before closing the lightbox', () => {
    const { props } = renderVideoLightbox()
    const dialog = screen.getByRole('dialog')
    const { exitFullscreen, setFullscreenElement } = mockFullscreenApi(dialog)
    setFullscreenElement(dialog)

    fireEvent.keyDown(document, { key: 'Escape' })
    expect(exitFullscreen).toHaveBeenCalledTimes(1)
    expect(props.onClose).not.toHaveBeenCalled()

    fireEvent.keyDown(document, { key: 'Escape' })
    expect(props.onClose).toHaveBeenCalledTimes(1)
  })

  it('closes with Escape when not fullscreen', () => {
    const { props } = renderVideoLightbox()

    fireEvent.keyDown(document, { key: 'Escape' })

    expect(props.onClose).toHaveBeenCalledTimes(1)
  })

  it('renders optional meta under the title', () => {
    renderPhotoLightbox({ title: '@alice', meta: '1.2K views' })
    const dialog = screen.getByRole('dialog')
    expect(dialog.textContent).toContain('@alice')
    expect(dialog.textContent).toContain('1.2K views')
  })

  it('plays slideshow audio when provided', () => {
    const { container } = renderPhotoLightbox({ audioAbsPath: 'S:/track.m4a' })
    const audio = container.querySelector('audio')
    expect(audio).toBeTruthy()
    expect(audio?.getAttribute('src')).toBe('asset://S:/track.m4a')
  })

  it('ignores player shortcuts from interactive controls', () => {
    const { props } = renderVideoLightbox({
      actions: <button type="button">Keep focus</button>,
    })
    const button = screen.getByRole('button', { name: 'Keep focus' })

    fireEvent.keyDown(button, { key: 'ArrowDown' })

    expect(props.onNext).not.toHaveBeenCalled()
  })

  describe('transport shortcuts and persisted prefs', () => {
    beforeEach(() => {
      localStorage.clear()
    })

    function mockPlayback(media: HTMLMediaElement) {
      const play = vi.fn().mockResolvedValue(undefined)
      const pause = vi.fn()
      Object.defineProperty(media, 'play', { configurable: true, value: play })
      Object.defineProperty(media, 'pause', { configurable: true, value: pause })
      const setPaused = (paused: boolean) =>
        Object.defineProperty(media, 'paused', { configurable: true, value: paused })
      setPaused(true)
      return { play, pause, setPaused }
    }

    it('toggles video play/pause with Space', () => {
      const { video } = renderVideoLightbox()
      const { play, pause, setPaused } = mockPlayback(video)

      fireEvent.keyDown(document, { key: ' ' })
      expect(play).toHaveBeenCalledTimes(1)
      expect(pause).not.toHaveBeenCalled()

      setPaused(false)
      fireEvent.keyDown(document, { key: ' ' })
      expect(pause).toHaveBeenCalledTimes(1)
    })

    it('toggles the slideshow soundtrack with Space when showing a photo with audio', () => {
      const { container } = renderPhotoLightbox({ audioAbsPath: 'S:/track.m4a' })
      const audio = container.querySelector('audio')!
      const { play } = mockPlayback(audio)

      fireEvent.keyDown(document, { key: ' ' })
      expect(play).toHaveBeenCalledTimes(1)
    })

    it('leaves Space to the native player when a media element is focused', () => {
      const { video } = renderVideoLightbox()
      const { play, pause } = mockPlayback(video)

      fireEvent.keyDown(video, { key: ' ' })
      expect(play).not.toHaveBeenCalled()
      expect(pause).not.toHaveBeenCalled()
    })

    it('mutes and unmutes the active media with M', () => {
      const { video } = renderVideoLightbox()
      expect(video.muted).toBe(false)

      fireEvent.keyDown(document, { key: 'm' })
      expect(video.muted).toBe(true)

      fireEvent.keyDown(document, { key: 'M' })
      expect(video.muted).toBe(false)
    })

    it('persists volume/mute on volumechange and re-applies them to new media', () => {
      const { video, unmount } = renderVideoLightbox()
      video.volume = 0.4
      video.muted = true
      fireEvent.volumeChange(video)

      expect(getStoredLightboxMediaPrefs()).toEqual({ volume: 0.4, muted: true })
      unmount()

      const { container } = renderPhotoLightbox({ audioAbsPath: 'S:/track.m4a' })
      const audio = container.querySelector('audio')!
      expect(audio.volume).toBe(0.4)
      expect(audio.muted).toBe(true)
    })

    it('applies previously stored prefs to the video on mount', () => {
      setStoredLightboxMediaPrefs({ volume: 0.25, muted: true })
      const { video } = renderVideoLightbox()

      expect(video.volume).toBe(0.25)
      expect(video.muted).toBe(true)
    })

    it('keeps native ←/→ seek when the slideshow audio is focused (no slide change)', () => {
      const onSlidePrev = vi.fn()
      const onSlideNext = vi.fn()
      const { container, props } = renderPhotoLightbox({
        audioAbsPath: 'S:/track.m4a',
        hasSlidePrev: true,
        hasSlideNext: true,
        onSlidePrev,
        onSlideNext,
      })
      const audio = container.querySelector('audio')!

      const rightNotCancelled = fireEvent.keyDown(audio, { key: 'ArrowRight' })
      const leftNotCancelled = fireEvent.keyDown(audio, { key: 'ArrowLeft' })

      // Not prevented → the native audio element keeps its own seek behavior.
      expect(rightNotCancelled).toBe(true)
      expect(leftNotCancelled).toBe(true)
      expect(onSlideNext).not.toHaveBeenCalled()
      expect(onSlidePrev).not.toHaveBeenCalled()
      expect(props.onNext).not.toHaveBeenCalled()
      expect(props.onPrev).not.toHaveBeenCalled()
    })
  })

  describe('video src hydration debounce', () => {
    afterEach(() => {
      vi.useRealTimers()
    })

    it('hydrates the first video synchronously and debounces subsequent switches', () => {
      vi.useFakeTimers()
      const props = {
        isVideo: true,
        hasPrev: true,
        hasNext: true,
        onPrev: vi.fn(),
        onNext: vi.fn(),
        onClose: vi.fn(),
      }
      const { container, rerender } = render(
        <MediaLightbox {...props} fileAbsPath="S:/first.mp4" />,
      )
      expect(container.querySelector('video')?.getAttribute('src')).toBe('asset://S:/first.mp4')

      rerender(<MediaLightbox {...props} fileAbsPath="S:/second.mp4" />)
      // Old element detached immediately; new src waits for the debounce.
      expect(container.querySelector('video')).toBeNull()

      rerender(<MediaLightbox {...props} fileAbsPath="S:/third.mp4" />)
      act(() => {
        vi.advanceTimersByTime(200)
      })
      expect(container.querySelector('video')?.getAttribute('src')).toBe('asset://S:/third.mp4')
    })

    it('never attaches the src of a video that was skipped past', () => {
      vi.useFakeTimers()
      const props = {
        isVideo: true,
        hasPrev: true,
        hasNext: true,
        onPrev: vi.fn(),
        onNext: vi.fn(),
        onClose: vi.fn(),
      }
      const { container, rerender } = render(
        <MediaLightbox {...props} fileAbsPath="S:/first.mp4" />,
      )
      rerender(<MediaLightbox {...props} fileAbsPath="S:/skipped.mp4" />)
      act(() => {
        vi.advanceTimersByTime(50)
      })
      rerender(<MediaLightbox {...props} fileAbsPath="S:/final.mp4" />)
      act(() => {
        vi.advanceTimersByTime(200)
      })

      expect(container.querySelector('video')?.getAttribute('src')).toBe('asset://S:/final.mp4')
    })
  })

  describe('caption', () => {
    // jsdom does no layout, so the clamp overflow is simulated on the prototype
    // — the caption is measured by a callback ref the moment it mounts, which is
    // too early to stub the node itself.
    const CLAMP_HEIGHT = 40
    let captionScrollHeight = CLAMP_HEIGHT

    function isCaptionText(element: HTMLElement) {
      return element.classList?.contains('profile-view-lightbox-caption-text') ?? false
    }

    beforeEach(() => {
      captionScrollHeight = CLAMP_HEIGHT
      Object.defineProperty(HTMLElement.prototype, 'scrollHeight', {
        configurable: true,
        get(this: HTMLElement) {
          return isCaptionText(this) ? captionScrollHeight : 0
        },
      })
      Object.defineProperty(HTMLElement.prototype, 'clientHeight', {
        configurable: true,
        get(this: HTMLElement) {
          return isCaptionText(this) ? CLAMP_HEIGHT : 0
        },
      })
    })

    afterEach(() => {
      // `delete` on a readonly DOM property does not typecheck; Reflect does the
      // same thing and restores jsdom's own descriptor for other suites.
      Reflect.deleteProperty(HTMLElement.prototype, 'scrollHeight')
      Reflect.deleteProperty(HTMLElement.prototype, 'clientHeight')
    })

    function captionText(container: HTMLElement) {
      return container.querySelector('.profile-view-lightbox-caption-text')
    }

    it('renders the caption under the media', () => {
      const { container } = renderPhotoLightbox({ caption: 'a nice sunset' })
      expect(captionText(container)?.textContent).toBe('a nice sunset')
    })

    it('omits the caption block when there is no caption', () => {
      const { container } = renderPhotoLightbox()
      expect(container.querySelector('.profile-view-lightbox-caption')).toBeNull()
    })

    it('hides the toggle when the caption fits the clamp', () => {
      renderPhotoLightbox({ caption: 'short' })
      expect(screen.queryByRole('button', { name: 'more' })).toBeNull()
    })

    it('offers the toggle when the caption overflows the clamp', () => {
      captionScrollHeight = 200
      renderPhotoLightbox({ caption: 'a very long caption' })
      expect(screen.getByRole('button', { name: 'more' })).toBeTruthy()
    })

    it('expands and collapses the caption', () => {
      captionScrollHeight = 200
      const { container } = renderPhotoLightbox({ caption: 'long one' })

      fireEvent.click(screen.getByRole('button', { name: 'more' }))
      expect(captionText(container)?.className).toContain('expanded')

      fireEvent.click(screen.getByRole('button', { name: 'less' }))
      expect(captionText(container)?.className).not.toContain('expanded')
    })

    it('collapses again when navigating to another post', () => {
      captionScrollHeight = 200
      const { container, rerender, props } = renderPhotoLightbox({ caption: 'long one' })
      fireEvent.click(screen.getByRole('button', { name: 'more' }))
      expect(captionText(container)?.className).toContain('expanded')

      rerender(<MediaLightbox {...props} fileAbsPath="S:/next.jpeg" caption="another caption" />)
      expect(captionText(container)?.className).not.toContain('expanded')
    })
  })
})
