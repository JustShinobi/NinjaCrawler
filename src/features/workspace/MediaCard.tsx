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

export function MediaCard({
  posterAbsPath,
  videoThumbAbsPath,
  isVideo,
  slideshowCount,
  badge,
  overlayText,
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
  return (
    <article
      className={`profile-view-card${selected ? ' is-selected' : ''}${selectMode ? ' is-selecting' : ''}`}
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
        {posterAbsPath ? (
          <img src={convertFileSrc(posterAbsPath)} alt="" loading="lazy" />
        ) : videoThumbAbsPath ? (
          <video src={convertFileSrc(videoThumbAbsPath)} preload="metadata" muted />
        ) : null}
        {isVideo ? <span className="profile-view-play" aria-hidden="true">▶</span> : null}
        {slideshowCount && slideshowCount > 1 ? (
          <span className="profile-view-badge" aria-hidden="true">▣ {slideshowCount}</span>
        ) : null}
        {badge ? <span className="profile-view-section" aria-hidden="true">{badge}</span> : null}
        <span className="profile-view-thumb-overlay" aria-hidden="true">{overlayText ?? ''}</span>
      </button>
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
                title={onlineTitle ?? 'Open original online'}
              >
                Online
              </button>
            )}
            {onReveal ? (
              <button
                className="ghost-button queue-icon-button"
                onClick={onReveal}
                type="button"
                title="Reveal in folder"
              >
                Folder
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
