import type { ReactNode } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'

/**
 * Lightbox de mídia compartilhado entre Profile View e Single Videos. Reproduz
 * vídeo/imagem inline (via convertFileSrc, sem passar pelo path-scope do opener)
 * com navegação anterior/próximo. Fonte única de verdade do preview.
 */
export interface MediaLightboxProps {
  fileAbsPath: string
  isVideo: boolean
  hasPrev: boolean
  hasNext: boolean
  onPrev: () => void
  onNext: () => void
  onClose: () => void
  /** Nome exibido acima da mídia (@autor do like ou handle do perfil). */
  title?: string
  /** Faixa de áudio separada para slideshows. */
  audioAbsPath?: string
  /** Ações abaixo do preview (Open online / Reveal / etc.). */
  actions?: ReactNode
}

export function MediaLightbox({
  fileAbsPath,
  isVideo,
  hasPrev,
  hasNext,
  onPrev,
  onNext,
  onClose,
  title,
  audioAbsPath,
  actions,
}: MediaLightboxProps) {
  return (
    <div className="profile-view-lightbox" role="dialog" aria-modal="true" onClick={onClose}>
      <button className="profile-view-lightbox-close" onClick={onClose} type="button" aria-label="Close">
        ✕
      </button>
      {hasPrev ? (
        <button
          className="profile-view-lightbox-nav prev"
          onClick={(event) => {
            event.stopPropagation()
            onPrev()
          }}
          type="button"
          aria-label="Previous"
        >
          ◀
        </button>
      ) : null}
      <div className="profile-view-lightbox-stage" onClick={(event) => event.stopPropagation()}>
        {title ? <div className="profile-view-lightbox-title">{title}</div> : null}
        {isVideo ? (
          // TikTok-style: o vídeo repete sozinho ao terminar.
          <video src={convertFileSrc(fileAbsPath)} controls autoPlay loop />
        ) : (
          <img src={convertFileSrc(fileAbsPath)} alt="" />
        )}
        {!isVideo && audioAbsPath ? (
          <audio src={convertFileSrc(audioAbsPath)} controls autoPlay loop />
        ) : null}
        {actions ? <div className="profile-view-lightbox-actions">{actions}</div> : null}
      </div>
      {hasNext ? (
        <button
          className="profile-view-lightbox-nav next"
          onClick={(event) => {
            event.stopPropagation()
            onNext()
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
