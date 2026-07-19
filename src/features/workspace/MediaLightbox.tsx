import { useEffect, useRef } from 'react'
import type { ReactNode } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'

/**
 * Shared media lightbox for Profile View and Single Videos. Plays video/image
 * inline (via convertFileSrc, without the opener path-scope) with previous/next
 * navigation. Single source of truth for the preview.
 *
 * Shortcuts:
 * - ↑/↓: previous/next post or top-level item (vertical axis — does NOT walk slides)
 * - ←/→ on carousel: previous/next slide of the same post
 * - ←/→ on video: seek ±1s
 * - Enter: fullscreen the lightbox (state survives media type switches)
 * - Escape: exit fullscreen if active; otherwise close
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
  /** Separate audio track for slideshows. */
  audioAbsPath?: string
  /** Actions below the preview (Open online / Reveal / etc.). */
  actions?: ReactNode
}

const VIDEO_SEEK_SECONDS = 1

function isInteractiveKeyTarget(target: EventTarget | null, root: HTMLElement | null): boolean {
  if (!(target instanceof Element)) return false
  // Do not treat <audio>/<video> as “interactive” for arrows — otherwise a
  // carousel with a soundtrack steals ←/→ while the player is focused.
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
  audioAbsPath,
  actions,
}: MediaLightboxProps) {
  const lightboxRef = useRef<HTMLDivElement>(null)
  const videoRef = useRef<HTMLVideoElement>(null)

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

    const handleKeyDown = (event: KeyboardEvent) => {
      if (isInteractiveKeyTarget(event.target, lightboxRef.current)) return

      const nav = navRef.current
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
        if (nav.isVideo) {
          handled = seekVideo(VIDEO_SEEK_SECONDS)
        } else if (nav.hasSlideNext && nav.onSlideNext) {
          nav.onSlideNext()
          handled = true
        }
      } else if (isArrow(event, 'Left')) {
        if (nav.isVideo) {
          handled = seekVideo(-VIDEO_SEEK_SECONDS)
        } else if (nav.hasSlidePrev && nav.onSlidePrev) {
          nav.onSlidePrev()
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
        {isVideo ? (
          <video ref={videoRef} src={convertFileSrc(fileAbsPath)} controls autoPlay loop />
        ) : (
          <img src={convertFileSrc(fileAbsPath)} alt="" />
        )}
        {!isVideo && audioAbsPath ? (
          <audio
            key={audioAbsPath}
            src={convertFileSrc(audioAbsPath)}
            controls
            autoPlay
            loop
          />
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
