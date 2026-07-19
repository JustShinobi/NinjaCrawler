// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { MediaLightbox } from './MediaLightbox'

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
})
