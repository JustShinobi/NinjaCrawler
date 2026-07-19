import { useState } from 'react'
import type { MouseEvent } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'

/**
 * Card de mídia compartilhado entre o Profile View e o Single Videos. É a fonte
 * única de verdade do visual do card (thumb, play, badges, seleção, lixeira,
 * ações Online/Folder): mudanças aqui refletem nas duas janelas.
 */
export interface MediaCardProps {
  /** Imagem de capa (poster/foto). Quando ausente, cai no <video> como thumb. */
  posterAbsPath?: string
  /** Caminho do vídeo usado como thumb quando não há poster. */
  videoThumbAbsPath?: string
  isVideo: boolean
  /** >1 mostra o selo de contagem (slideshow). */
  slideshowCount?: number
  /** Selo no canto superior (seção/provider). */
  badge?: string
  /** Texto do overlay inferior (hora/data/@autor). */
  overlayText?: string
  /** Modo feed YouTube: thumb 16:9 + rótulo (título/meta) abaixo do card. */
  youtube?: boolean
  /** Selo de duração no canto inferior direito do thumb (`M:SS`/`H:MM:SS`). */
  durationBadge?: string
  /** Título do vídeo, exibido abaixo do thumb no modo YouTube (2 linhas máx). */
  captionTitle?: string
  /** Linha secundária abaixo do título (views · data) no modo YouTube. */
  captionMeta?: string
  selected: boolean
  selectMode: boolean
  onToggleSelect: (shiftKey: boolean) => void
  onOpen: (shiftKey: boolean) => void
  /** Oculta o botão "Online" (ex.: stories efêmeros / sem link). */
  hideOnline?: boolean
  onlineDisabled?: boolean
  onlineTitle?: string
  onOnline?: () => void
  onReveal?: () => void
  onDelete?: () => void
  /** Menu de contexto (botão direito) sobre o card. */
  onContextMenu?: (event: MouseEvent<HTMLElement>) => void
}

const TRASH_PATH =
  'M9 3h6m-9 3h12M6 6l1 14a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-14M10 10v7M14 10v7'
const EXTERNAL_PATH =
  'M14 4h6v6M20 4l-9 9M12 6H7a3 3 0 0 0-3 3v8a3 3 0 0 0 3 3h8a3 3 0 0 0 3-3v-5'
const FOLDER_PATH =
  'M3 7.5A2.5 2.5 0 0 1 5.5 5H10l2 2h6.5A2.5 2.5 0 0 1 21 9.5v7A2.5 2.5 0 0 1 18.5 19h-13A2.5 2.5 0 0 1 3 16.5v-9Z'

export function MediaCard({
  posterAbsPath,
  videoThumbAbsPath,
  isVideo,
  slideshowCount,
  badge,
  overlayText,
  youtube,
  durationBadge,
  captionTitle,
  captionMeta,
  selected,
  selectMode,
  onToggleSelect,
  onOpen,
  hideOnline,
  onlineDisabled,
  onlineTitle,
  onOnline,
  onReveal,
  onDelete,
  onContextMenu,
}: MediaCardProps) {
  // Guardamos qual caminho falhou (não um boolean): quando o card virtualizado
  // é reutilizado com outro poster, o novo caminho volta a ser tentado sem
  // precisar de efeito/setState extra.
  const [failedPosterPath, setFailedPosterPath] = useState<string>()
  const usablePoster = posterAbsPath !== failedPosterPath ? posterAbsPath : undefined

  return (
    <article
      className={`profile-view-card${youtube ? ' profile-view-card--youtube' : ''}${selected ? ' is-selected' : ''}${selectMode ? ' is-selecting' : ''}`}
      onContextMenu={onContextMenu}
    >
      <button
        className={`profile-view-select${selected ? ' is-checked' : ''}`}
        onClick={(event) => onToggleSelect(event.shiftKey)}
        type="button"
        aria-pressed={selected}
        aria-label={selected ? 'Deselect media' : 'Select media'}
        title={selected ? 'Deselect (Shift: range)' : 'Select (Shift: range)'}
      >
        <span aria-hidden="true">{selected ? '✓' : ''}</span>
      </button>
      <button
        className="profile-view-thumb"
        onClick={(event) => onOpen(event.shiftKey)}
        type="button"
        title={selectMode ? 'Toggle selection (Shift: range)' : 'Open preview'}
      >
        {usablePoster ? (
          <img
            src={convertFileSrc(usablePoster)}
            alt=""
            loading="lazy"
            onError={() => setFailedPosterPath(posterAbsPath)}
          />
        ) : videoThumbAbsPath ? (
          <video src={convertFileSrc(videoThumbAbsPath)} preload="metadata" muted />
        ) : null}
        {isVideo ? <span className="profile-view-play" aria-hidden="true">▶</span> : null}
        {slideshowCount && slideshowCount > 1 ? (
          <span className="profile-view-badge" aria-hidden="true">▣ {slideshowCount}</span>
        ) : null}
        {badge ? <span className="profile-view-section" aria-hidden="true">{badge}</span> : null}
        {durationBadge ? (
          <span className="profile-view-duration" aria-hidden="true">{durationBadge}</span>
        ) : null}
        <span className="profile-view-thumb-overlay" aria-hidden="true">{overlayText ?? ''}</span>
      </button>
      {youtube && (captionTitle || captionMeta) ? (
        <div className="profile-view-caption">
          {captionTitle ? (
            <span className="profile-view-caption-title" title={captionTitle}>{captionTitle}</span>
          ) : null}
          {captionMeta ? (
            <span className="profile-view-caption-meta">{captionMeta}</span>
          ) : null}
        </div>
      ) : null}
      {/* Ações por card só fora do modo seleção (durante a seleção o card é só
          alvo de clique, sem ruído de botões). */}
      {selectMode ? null : (
        <>
          <div className="profile-view-card-actions">
            {hideOnline ? null : (
              <button
                className="ghost-button queue-icon-button"
                disabled={onlineDisabled}
                onClick={onOnline}
                type="button"
                aria-label="Online"
                title={onlineTitle ?? 'Open original online'}
              >
                <svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true" focusable="false">
                  <path
                    d={EXTERNAL_PATH}
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.8"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
              </button>
            )}
            {onReveal ? (
              <button
                className="ghost-button queue-icon-button"
                onClick={onReveal}
                type="button"
                aria-label="Folder"
                title="Reveal in folder"
              >
                <svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true" focusable="false">
                  <path
                    d={FOLDER_PATH}
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.8"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
              </button>
            ) : null}
          </div>
          {onDelete ? (
            <button
              className="profile-view-trash"
              onClick={onDelete}
              type="button"
              aria-label="Delete"
              title="Delete (move to Recycle Bin)"
            >
              <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true" focusable="false">
                <path
                  d={TRASH_PATH}
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.8"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              </svg>
            </button>
          ) : null}
        </>
      )}
    </article>
  )
}
