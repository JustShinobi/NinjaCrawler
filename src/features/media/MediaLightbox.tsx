import { useCallback, useEffect, useRef, useState } from 'react'
import type { ReactNode } from 'react'
import { MediaViewer } from './MediaViewer'

/**
 * Shared media lightbox for Profile View and Single Videos. Plays video/image
 * inline (via convertFileSrc, without the opener path-scope) with previous/next
 * navigation. Single source of truth for the preview.
 *
 * Shortcuts:
 * - ↑/↓: previous/next post or top-level item (vertical axis — does NOT walk slides)
 * - ←/→ on carousel: previous/next slide of the same post
 * - ←/→ on video: seek ±1s
 * - ←/→ while the slideshow `<audio>` is focused: native audio seek (does NOT change slide)
 * - Space: play/pause the active media (video, or slideshow soundtrack)
 * - M: mute/unmute the active media
 * - Enter: fullscreen the lightbox (state survives media type switches)
 * - Escape: exit fullscreen if active; otherwise close
 *
 * Volume/mute are persisted (localStorage) and re-applied to every media
 * element that mounts, so they survive post switches and window reopens.
 */
export interface MediaLightboxProps {
  fileAbsPath: string
  isVideo: boolean
  /** Vertical navigation (between posts / top-level items). */
  hasPrev: boolean
  hasNext: boolean
  onPrev: () => void
  onNext: () => void
  onClose: () => void
  /**
   * Horizontal navigation within a carousel/slideshow. When omitted, ←/→ on
   * photos do not navigate (only video seek); side buttons fall back to the
   * vertical axis.
   */
  hasSlidePrev?: boolean
  hasSlideNext?: boolean
  onSlidePrev?: () => void
  onSlideNext?: () => void
  /** Label above the media (@like author or profile handle). */
  title?: string
  /** Secondary meta (e.g. "1.2K views · 2/5"). */
  meta?: string
  /**
   * Post caption, shown under the media. Instagram captions run long and carry
   * their own line breaks, so it is clamped to a few lines behind a "more"
   * toggle instead of pushing the actions out of view.
   */
  caption?: string
  /** Separate audio track for slideshows. */
  audioAbsPath?: string
  /** Actions below the preview (Open online / Reveal / etc.). */
  actions?: ReactNode
}

const VIDEO_SEEK_SECONDS = 1

/**
 * Debounce for hydrating `<video src>` while stepping quickly through posts:
 * the previous element unmounts immediately (stops stale playback) and the
 * next src only attaches once navigation settles, so skipped-past videos never
 * start decoding. The first video after opening hydrates synchronously.
 */
function isInteractiveKeyTarget(target: EventTarget | null, root: HTMLElement | null): boolean {
  if (!(target instanceof Element)) return false
  // Media elements are handled per-key (e.g. a focused <audio> keeps native
  // ←/→ seek) instead of blanket-ignoring shortcuts while they are focused.
  const interactive = target.closest(
    'button, input, textarea, select, a[href], [contenteditable="true"]',
  )
  return Boolean(interactive && root?.contains(interactive))
}

/** True if the lightbox (or a descendant) is the document fullscreen element. */
function isLightboxFullscreen(root: HTMLElement | null): boolean {
  const active = document.fullscreenElement
  if (!root || !active) return false
  return active === root || root.contains(active)
}

function isArrow(event: KeyboardEvent, direction: 'Up' | 'Down' | 'Left' | 'Right'): boolean {
  return event.key === `Arrow${direction}` || event.code === `Arrow${direction}`
}

export function MediaLightbox({
  fileAbsPath,
  isVideo,
  hasPrev,
  hasNext,
  onPrev,
  onNext,
  onClose,
  hasSlidePrev = false,
  hasSlideNext = false,
  onSlidePrev,
  onSlideNext,
  title,
  meta,
  caption,
  audioAbsPath,
  actions,
}: MediaLightboxProps) {
  const lightboxRef = useRef<HTMLDivElement>(null)
  const videoRef = useRef<HTMLVideoElement | null>(null)
  const audioRef = useRef<HTMLAudioElement | null>(null)

  // Caption clamp: only offer the toggle when the text actually overflows,
  // otherwise a one-line caption would show a useless "more".
  const [captionExpanded, setCaptionExpanded] = useState(false)
  const [captionOverflows, setCaptionOverflows] = useState(false)

  // Collapse again on every new post. Adjusting state during render is the
  // documented alternative to an effect for deriving from changed props.
  const captionIdentity = `${fileAbsPath} ${caption ?? ''}`
  const [lastCaptionIdentity, setLastCaptionIdentity] = useState(captionIdentity)
  if (lastCaptionIdentity !== captionIdentity) {
    setLastCaptionIdentity(captionIdentity)
    setCaptionExpanded(false)
  }

  // Measured through a callback ref so it runs exactly when the node mounts.
  // The caption element is keyed by its text, so a new caption remounts it and
  // re-measures. Measuring only while collapsed: expanding lifts the clamp, so
  // the comparison would always come out false.
  const measureCaption = useCallback((element: HTMLDivElement | null) => {
    if (!element) return
    setCaptionOverflows(element.scrollHeight > element.clientHeight + 1)
  }, [])

  const setVideoEl = useCallback((element: HTMLVideoElement | null) => {
    videoRef.current = element
  }, [])
  const setAudioEl = useCallback((element: HTMLAudioElement | null) => {
    audioRef.current = element
  }, [])

  // Refs: keyboard listener mounts once and always reads current state.
  // Avoids “dead” arrows from a stale closure after switching slide/post.
  // Updated in an effect (not during render) to satisfy react-hooks/refs.
  const navRef = useRef({
    isVideo,
    hasPrev,
    hasNext,
    hasSlidePrev,
    hasSlideNext,
    onPrev,
    onNext,
    onClose,
    onSlidePrev,
    onSlideNext,
  })
  useEffect(() => {
    navRef.current = {
      isVideo,
      hasPrev,
      hasNext,
      hasSlidePrev,
      hasSlideNext,
      onPrev,
      onNext,
      onClose,
      onSlidePrev,
      onSlideNext,
    }
  })

  useEffect(() => {
    lightboxRef.current?.focus()
  }, [])

  // Re-focus the dialog when media changes (e.g. after ←/→) so arrows do not
  // land on action buttons / native controls.
  useEffect(() => {
    lightboxRef.current?.focus()
  }, [fileAbsPath])

  useEffect(() => {
    const seekVideo = (delta: number) => {
      const video = videoRef.current
      if (!video) return false
      const duration = video.duration
      const nextTime = video.currentTime + delta
      video.currentTime = Number.isFinite(duration)
        ? Math.min(Math.max(0, nextTime), duration)
        : Math.max(0, nextTime)
      return true
    }

    const toggleFullscreen = () => {
      const root = lightboxRef.current
      if (!root) return false
      if (isLightboxFullscreen(root)) {
        const exitFullscreen = document.exitFullscreen?.()
        void exitFullscreen?.catch(() => undefined)
      } else {
        const requestFullscreen = root.requestFullscreen?.()
        void requestFullscreen?.catch(() => undefined)
      }
      return true
    }

    const togglePlayback = () => {
      const media = videoRef.current ?? audioRef.current
      if (!media) return
      if (media.paused) {
        const play = media.play()
        void play?.catch(() => undefined)
      } else {
        media.pause()
      }
    }

    const toggleMute = () => {
      const media = videoRef.current ?? audioRef.current
      if (!media) return
      // Dispatches `volumechange`, which persists the new prefs.
      media.muted = !media.muted
    }

    const handleKeyDown = (event: KeyboardEvent) => {
      if (isInteractiveKeyTarget(event.target, lightboxRef.current)) return

      const nav = navRef.current
      // A focused media element keeps its own transport keys native
      // (Space/M on <video>/<audio>, and ←/→ seek on <audio>).
      const mediaFocused = event.target instanceof HTMLMediaElement
      const audioFocused = event.target === audioRef.current && audioRef.current !== null
      let handled = false

      if (event.key === 'Escape') {
        if (isLightboxFullscreen(lightboxRef.current)) {
          const exitFullscreen = document.exitFullscreen?.()
          void exitFullscreen?.catch(() => undefined)
        } else {
          nav.onClose()
        }
        handled = true
      } else if (isArrow(event, 'Down')) {
        // Vertical = post/item (never slide).
        if (nav.hasNext) nav.onNext()
        handled = true
      } else if (isArrow(event, 'Up')) {
        if (nav.hasPrev) nav.onPrev()
        handled = true
      } else if (isArrow(event, 'Right')) {
        if (audioFocused) {
          // Focused slideshow soundtrack: leave native seek alone.
        } else if (nav.isVideo) {
          handled = seekVideo(VIDEO_SEEK_SECONDS)
        } else if (nav.hasSlideNext && nav.onSlideNext) {
          nav.onSlideNext()
          handled = true
        }
      } else if (isArrow(event, 'Left')) {
        if (audioFocused) {
          // Focused slideshow soundtrack: leave native seek alone.
        } else if (nav.isVideo) {
          handled = seekVideo(-VIDEO_SEEK_SECONDS)
        } else if (nav.hasSlidePrev && nav.onSlidePrev) {
          nav.onSlidePrev()
          handled = true
        }
      } else if (event.key === ' ' || event.code === 'Space') {
        if (!mediaFocused) {
          togglePlayback()
          // Handled even without media so Space never scrolls the page behind.
          handled = true
        }
      } else if ((event.key === 'm' || event.key === 'M') && !event.ctrlKey && !event.metaKey && !event.altKey) {
        if (!mediaFocused) {
          toggleMute()
          handled = true
        }
      } else if (event.key === 'Enter') {
        handled = toggleFullscreen()
      }

      if (handled) {
        event.preventDefault()
        event.stopImmediatePropagation()
      }
    }

    document.addEventListener('keydown', handleKeyDown, true)
    return () => document.removeEventListener('keydown', handleKeyDown, true)
  }, [])

  const canGoSidePrev = hasSlidePrev || hasPrev
  const canGoSideNext = hasSlideNext || hasNext
  const goSidePrev = () => {
    if (hasSlidePrev && onSlidePrev) onSlidePrev()
    else if (hasPrev) onPrev()
  }
  const goSideNext = () => {
    if (hasSlideNext && onSlideNext) onSlideNext()
    else if (hasNext) onNext()
  }

  return (
    <div
      className="profile-view-lightbox"
      role="dialog"
      aria-modal="true"
      onClick={onClose}
      ref={lightboxRef}
      tabIndex={-1}
    >
      <button className="profile-view-lightbox-close" onClick={onClose} type="button" aria-label="Close">
        ✕
      </button>
      {canGoSidePrev ? (
        <button
          className="profile-view-lightbox-nav prev"
          onClick={(event) => {
            event.stopPropagation()
            goSidePrev()
          }}
          type="button"
          aria-label="Previous"
        >
          ◀
        </button>
      ) : null}
      <div className="profile-view-lightbox-stage" onClick={(event) => event.stopPropagation()}>
        {title ? <div className="profile-view-lightbox-title">{title}</div> : null}
        {meta ? <div className="profile-view-lightbox-meta">{meta}</div> : null}
        <MediaViewer
          fileAbsPath={fileAbsPath}
          isVideo={isVideo}
          alt={title ?? fileAbsPath.split(/[\\/]/).pop() ?? 'Media preview'}
          audioAbsPath={audioAbsPath}
          autoPlay
          loop
          onVideoElement={setVideoEl}
          onAudioElement={setAudioEl}
        />
        {caption ? (
          <div className="profile-view-lightbox-caption">
            <div
              key={caption}
              ref={captionExpanded ? undefined : measureCaption}
              className={
                captionExpanded
                  ? 'profile-view-lightbox-caption-text expanded'
                  : 'profile-view-lightbox-caption-text'
              }
            >
              {caption}
            </div>
            {captionOverflows || captionExpanded ? (
              <button
                className="profile-view-lightbox-caption-toggle"
                onClick={() => setCaptionExpanded((expanded) => !expanded)}
                type="button"
              >
                {captionExpanded ? 'less' : 'more'}
              </button>
            ) : null}
          </div>
        ) : null}
        {actions ? <div className="profile-view-lightbox-actions">{actions}</div> : null}
      </div>
      {canGoSideNext ? (
        <button
          className="profile-view-lightbox-nav next"
          onClick={(event) => {
            event.stopPropagation()
            goSideNext()
          }}
          type="button"
          aria-label="Next"
        >
          ▶
        </button>
      ) : null}
    </div>
  )
}
