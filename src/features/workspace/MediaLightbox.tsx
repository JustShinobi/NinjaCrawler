import { useEffect, useRef } from 'react'
import type { ReactNode } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'

/**
 * Lightbox de mídia compartilhado entre Profile View e Single Videos. Reproduz
 * vídeo/imagem inline (via convertFileSrc, sem passar pelo path-scope do opener)
 * com navegação anterior/próximo. Fonte única de verdade do preview.
 *
 * Atalhos:
 * - ↑/↓: post/item anterior/próximo (eixo vertical — NÃO percorre slides)
 * - ←/→ em carrossel: slide anterior/próximo do mesmo post
 * - ←/→ em vídeo: seek ±1s
 * - Enter: tela cheia do lightbox (preserva estado ao trocar mídia)
 * - Escape: sai da tela cheia se ativa; senão fecha
 */
export interface MediaLightboxProps {
  fileAbsPath: string
  isVideo: boolean
  /** Navegação vertical (entre posts / itens de nível superior). */
  hasPrev: boolean
  hasNext: boolean
  onPrev: () => void
  onNext: () => void
  onClose: () => void
  /**
   * Navegação horizontal dentro de um carrossel/slideshow. Quando omitida,
   * ←/→ em foto não navegam (só seek em vídeo); os botões laterais caem no
   * eixo vertical.
   */
  hasSlidePrev?: boolean
  hasSlideNext?: boolean
  onSlidePrev?: () => void
  onSlideNext?: () => void
  /** Nome exibido acima da mídia (@autor do like ou handle do perfil). */
  title?: string
  /** Meta secundária (ex.: "1.2K views · 2/5"). */
  meta?: string
  /** Faixa de áudio separada para slideshows. */
  audioAbsPath?: string
  /** Ações abaixo do preview (Open online / Reveal / etc.). */
  actions?: ReactNode
}

const VIDEO_SEEK_SECONDS = 1

function isInteractiveKeyTarget(target: EventTarget | null, root: HTMLElement | null): boolean {
  if (!(target instanceof Element)) return false
  // Não tratar <audio>/<video> como “interactive” para setas — senão o carrossel
  // com trilha some as teclas ←/→ enquanto o player tem foco.
  const interactive = target.closest(
    'button, input, textarea, select, a[href], [contenteditable="true"]',
  )
  return Boolean(interactive && root?.contains(interactive))
}

/** True se o lightbox (ou um descendente) estiver em fullscreen do documento. */
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

  // Refs: o listener de teclado fica montado 1× e sempre lê o estado atual.
  // Evita setas “mortas” por closure stale após trocar slide/post.
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

  useEffect(() => {
    lightboxRef.current?.focus()
  }, [])

  // Re-foca o dialog ao trocar mídia (ex.: após ←/→), para as setas não caírem
  // em botões de ação / controles nativos.
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
        // Vertical = post/item (nunca slide).
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
