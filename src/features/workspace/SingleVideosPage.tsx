import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { CSSProperties, FormEvent } from 'react'
import {
  deleteSingleVideo,
  enqueueSingleVideoDownload,
  listSingleVideos,
  loadWorkspaceSnapshot,
  openExternalTarget,
  revealMediaInFolder,
  subscribeToSingleVideosChanged,
  upsertSourceProfile,
} from '../../bridge/desktop'
import type { ProviderKey, SingleVideo } from '../../domain/models'
import { MediaCard } from './MediaCard'
import { MediaLightbox } from './MediaLightbox'

const PROVIDER_LABELS: Record<string, string> = {
  tiktok: 'TikTok',
  instagram: 'Instagram',
  twitter: 'Twitter/X',
  youtube: 'YouTube',
}

type ViewMode = 'day' | 'grid'

/** Providers de single video que também podem virar perfil rastreado. */
const TRACKABLE_PROVIDERS: ProviderKey[] = ['instagram', 'tiktok', 'twitter']

const VIEW_MODE_STORAGE_KEY = 'singleVideos.mode'
const DENSITY_STORAGE_KEY = 'singleVideos.density'
const DENSITY_STEPS = [110, 140, 160, 190, 230] as const
const DEFAULT_DENSITY_INDEX = 2

function providerLabel(provider: string): string {
  return PROVIDER_LABELS[provider] ?? provider
}

function readStoredMode(): ViewMode {
  try {
    return localStorage.getItem(VIEW_MODE_STORAGE_KEY) === 'grid' ? 'grid' : 'day'
  } catch {
    return 'day'
  }
}

function readStoredDensity(): number {
  try {
    const stored = localStorage.getItem(DENSITY_STORAGE_KEY)
    if (stored !== null) {
      const raw = Number(stored)
      if (Number.isInteger(raw) && raw >= 0 && raw < DENSITY_STEPS.length) return raw
    }
  } catch {
    /* ignore */
  }
  return DEFAULT_DENSITY_INDEX
}

function dayKey(capturedAt?: number): string {
  if (!capturedAt) return 'unknown'
  return new Date(capturedAt * 1000).toISOString().slice(0, 10)
}

function dayLabel(key: string, capturedAt?: number): string {
  if (key === 'unknown' || !capturedAt) return 'Date unknown'
  return new Date(capturedAt * 1000).toLocaleDateString(undefined, {
    weekday: 'short',
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  })
}

interface DayGroup {
  key: string
  label: string
  videos: SingleVideo[]
}

interface SingleVideoPreviewItem {
  video: SingleVideo
  file: SingleVideo['files'][number]
}

export function SingleVideosPage() {
  const [videos, setVideos] = useState<SingleVideo[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string>()
  const [urlInput, setUrlInput] = useState('')
  const [adding, setAdding] = useState(false)
  const [notice, setNotice] = useState<string>()
  const [providerFilter, setProviderFilter] = useState<string>('all')
  const [query, setQuery] = useState('')
  const [viewMode, setViewMode] = useState<ViewMode>(readStoredMode)
  const [densityIndex, setDensityIndex] = useState<number>(readStoredDensity)
  const [lightboxIndex, setLightboxIndex] = useState<number>()
  const [selectMode, setSelectMode] = useState(false)
  const [selectedIds, setSelectedIds] = useState<Set<string>>(() => new Set())
  const selectAnchorRef = useRef<string | null>(null)
  const [confirmIds, setConfirmIds] = useState<string[]>()
  const [deleting, setDeleting] = useState(false)
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; video: SingleVideo }>()
  const [addingProfile, setAddingProfile] = useState(false)

  useEffect(() => {
    try {
      localStorage.setItem(VIEW_MODE_STORAGE_KEY, viewMode)
    } catch {
      /* ignore */
    }
  }, [viewMode])
  useEffect(() => {
    try {
      localStorage.setItem(DENSITY_STORAGE_KEY, String(densityIndex))
    } catch {
      /* ignore */
    }
  }, [densityIndex])

  const load = useCallback(async () => {
    setLoading(true)
    setError(undefined)
    try {
      setVideos(await listSingleVideos())
    } catch (loadError) {
      setError(loadError instanceof Error ? loadError.message : 'Failed to load single videos.')
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void load()
  }, [load])

  // Recarrega quando a fila termina um download (evento do backend).
  useEffect(() => {
    let unlisten: (() => void) | undefined
    let active = true
    void subscribeToSingleVideosChanged(() => {
      void load()
    }).then((dispose) => {
      if (active) unlisten = dispose
      else dispose()
    })
    return () => {
      active = false
      unlisten?.()
    }
  }, [load])

  const handleAdd = useCallback(
    async (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault()
      const url = urlInput.trim()
      if (!url) return
      setAdding(true)
      setError(undefined)
      setNotice(undefined)
      try {
        await enqueueSingleVideoDownload(url)
        setUrlInput('')
        setNotice('Queued for download — it appears here when finished. Track progress in Queue Status.')
      } catch (addError) {
        setError(addError instanceof Error ? addError.message : 'Failed to queue the video.')
      } finally {
        setAdding(false)
      }
    },
    [urlInput],
  )

  const providers = useMemo(() => {
    const present = new Set(videos.map((video) => video.provider))
    return ['tiktok', 'instagram', 'twitter', 'youtube'].filter((provider) => present.has(provider))
  }, [videos])

  useEffect(() => {
    if (providerFilter !== 'all' && !providers.includes(providerFilter)) {
      setProviderFilter('all')
    }
  }, [providers, providerFilter])

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase()
    return videos.filter((video) => {
      if (providerFilter !== 'all' && video.provider !== providerFilter) return false
      if (!needle) return true
      const haystack = [
        video.uploader,
        video.title,
        video.sourceUrl,
        video.providerVideoId,
        video.relativePath,
        video.mediaType,
      ]
        .filter(Boolean)
        .join(' ')
        .toLowerCase()
      return haystack.includes(needle)
    })
  }, [videos, providerFilter, query])

  const days = useMemo<DayGroup[]>(() => {
    const groups: DayGroup[] = []
    let current: DayGroup | undefined
    for (const video of filtered) {
      const key = dayKey(video.capturedAt)
      if (!current || current.key !== key) {
        current = { key, label: dayLabel(key, video.capturedAt), videos: [] }
        groups.push(current)
      }
      current.videos.push(video)
    }
    return groups
  }, [filtered])

  // Índice de cada vídeo (na ordem exibida) para range do shift+clique e lightbox.
  const indexById = useMemo(() => {
    const map = new Map<string, number>()
    filtered.forEach((video, index) => map.set(video.id, index))
    return map
  }, [filtered])

  const previewItems = useMemo<SingleVideoPreviewItem[]>(() => {
    return filtered.flatMap((video) => {
      const files = video.files.length > 0
        ? video.files
        : [{ relativePath: video.relativePath, absolutePath: video.absolutePath, mediaType: video.mediaType }]
      return files.map((file) => ({ video, file }))
    })
  }, [filtered])

  // Sai/limpa a seleção ao trocar de filtro.
  useEffect(() => {
    setSelectMode(false)
    setSelectedIds(new Set())
    selectAnchorRef.current = null
  }, [providerFilter, query])

  const handleSelect = useCallback(
    (video: SingleVideo, shiftKey: boolean) => {
      setSelectMode(true)
      setSelectedIds((current) => {
        const next = new Set(current)
        const anchor = selectAnchorRef.current
        if (shiftKey && anchor !== null && indexById.has(anchor) && indexById.has(video.id)) {
          const a = indexById.get(anchor)!
          const b = indexById.get(video.id)!
          const [lo, hi] = a <= b ? [a, b] : [b, a]
          for (let i = lo; i <= hi; i++) next.add(filtered[i].id)
          return next
        }
        if (next.has(video.id)) next.delete(video.id)
        else next.add(video.id)
        return next
      })
      if (!shiftKey) selectAnchorRef.current = video.id
    },
    [indexById, filtered],
  )

  const exitSelectMode = useCallback(() => {
    setSelectMode(false)
    setSelectedIds(new Set())
    selectAnchorRef.current = null
  }, [])

  const openLightbox = useCallback(
    (video: SingleVideo) => {
      const index = previewItems.findIndex(
        (item) => item.video.id === video.id && item.file.absolutePath === video.absolutePath,
      )
      const fallbackIndex = previewItems.findIndex((item) => item.video.id === video.id)
      if (index >= 0) setLightboxIndex(index)
      else if (fallbackIndex >= 0) setLightboxIndex(fallbackIndex)
    },
    [previewItems],
  )
  const closeLightbox = useCallback(() => setLightboxIndex(undefined), [])
  const stepLightbox = useCallback(
    (delta: number) => {
      setLightboxIndex((current) => {
        if (current === undefined) return current
        const next = current + delta
        if (next < 0 || next >= previewItems.length) return current
        return next
      })
    },
    [previewItems.length],
  )

  // Teclado no lightbox.
  const lightboxOpen = lightboxIndex !== undefined
  const lightboxOpenRef = useRef(lightboxOpen)
  lightboxOpenRef.current = lightboxOpen
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (!lightboxOpenRef.current) return
      if (event.key === 'Escape') {
        event.stopImmediatePropagation()
        closeLightbox()
      } else if (event.key === 'ArrowLeft') {
        stepLightbox(-1)
      } else if (event.key === 'ArrowRight') {
        stepLightbox(1)
      }
    }
    document.addEventListener('keydown', handler, true)
    return () => document.removeEventListener('keydown', handler, true)
  }, [closeLightbox, stepLightbox])

  const performDelete = useCallback(async () => {
    if (!confirmIds || confirmIds.length === 0) return
    setDeleting(true)
    setError(undefined)
    try {
      let next: SingleVideo[] = videos
      for (const id of confirmIds) {
        next = await deleteSingleVideo(id)
      }
      setVideos(next)
      setConfirmIds(undefined)
      exitSelectMode()
    } catch (deleteError) {
      setError(deleteError instanceof Error ? deleteError.message : 'Failed to delete the video.')
    } finally {
      setDeleting(false)
    }
  }, [confirmIds, videos, exitSelectMode])

  const safeReveal = useCallback((path: string) => {
    void revealMediaInFolder(path).catch((revealError) => {
      setError(revealError instanceof Error ? revealError.message : String(revealError))
    })
  }, [])
  const safeOpenOnline = useCallback((url: string) => {
    void openExternalTarget(url).catch((openError) => {
      setError(openError instanceof Error ? openError.message : String(openError))
    })
  }, [])

  // "Adicionar perfil ao NinjaCrawler": cria um source rastreado a partir do
  // provider + @autor do single video (para baixar tudo do perfil depois).
  const addProfileFromVideo = useCallback(async (video: SingleVideo) => {
    setContextMenu(undefined)
    const provider = video.provider as ProviderKey
    const handle = video.uploader?.trim()
    if (!handle || !TRACKABLE_PROVIDERS.includes(provider)) {
      setError('This video has no trackable profile.')
      return
    }
    setAddingProfile(true)
    setError(undefined)
    setNotice(undefined)
    try {
      const snapshot = await loadWorkspaceSnapshot()
      const normalized = handle.replace(/^@/, '').toLowerCase()
      const existing = snapshot.sources.find(
        (source) =>
          source.provider === provider &&
          source.handle.replace(/^@/, '').toLowerCase() === normalized,
      )
      if (existing) {
        setNotice(`@${handle} is already tracked in NinjaCrawler.`)
        return
      }
      const account = snapshot.accounts.find((entry) => entry.provider === provider)
      await upsertSourceProfile({
        provider,
        sourceKind: 'profile',
        handle,
        displayName: handle,
        accountId: account?.id ?? null,
        labels: [],
        readyForDownload: true,
      })
      setNotice(
        account
          ? `Added @${handle} to NinjaCrawler.`
          : `Added @${handle} — assign a ${provider} account to start downloading.`,
      )
    } catch (addError) {
      setError(addError instanceof Error ? addError.message : 'Failed to add the profile.')
    } finally {
      setAddingProfile(false)
    }
  }, [])

  // Fecha o menu de contexto em clique fora / rolagem / Esc.
  useEffect(() => {
    if (!contextMenu) return
    const close = () => setContextMenu(undefined)
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') close()
    }
    window.addEventListener('click', close)
    window.addEventListener('scroll', close, true)
    window.addEventListener('resize', close)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('click', close)
      window.removeEventListener('scroll', close, true)
      window.removeEventListener('resize', close)
      window.removeEventListener('keydown', onKey)
    }
  }, [contextMenu])

  const gridStyle = { '--pv-thumb-min': `${DENSITY_STEPS[densityIndex]}px` } as CSSProperties
  const activeItem = lightboxIndex !== undefined ? previewItems[lightboxIndex] : undefined
  const activeVideo = activeItem?.video
  const activeFileIndex =
    activeItem && activeVideo
      ? activeVideo.files.findIndex((file) => file.absolutePath === activeItem.file.absolutePath)
      : -1
  const activeTitle =
    activeVideo && activeVideo.files.length > 1 && activeFileIndex >= 0
      ? `${activeVideo.uploader ? `@${activeVideo.uploader}` : providerLabel(activeVideo.provider)} · ${activeFileIndex + 1} / ${activeVideo.files.length}`
      : activeVideo?.uploader
        ? `@${activeVideo.uploader}`
        : activeVideo
          ? providerLabel(activeVideo.provider)
          : undefined
  const selectedCount = selectedIds.size

  const isSingleVideoVideo = (mediaType: string) => mediaType === 'video'

  const renderCard = (video: SingleVideo) => (
    <MediaCard
      key={video.id}
      posterAbsPath={isSingleVideoVideo(video.mediaType) ? undefined : video.absolutePath}
      videoThumbAbsPath={isSingleVideoVideo(video.mediaType) ? video.absolutePath : undefined}
      isVideo={isSingleVideoVideo(video.mediaType)}
      slideshowCount={video.mediaType === 'slideshow' ? video.files.length : undefined}
      badge={providerLabel(video.provider)}
      overlayText={video.uploader ? `@${video.uploader}` : undefined}
      selected={selectedIds.has(video.id)}
      selectMode={selectMode}
      onToggleSelect={(shiftKey) => handleSelect(video, shiftKey)}
      onOpen={(shiftKey) => {
        if (selectMode) handleSelect(video, shiftKey)
        else if (shiftKey && selectAnchorRef.current !== null) handleSelect(video, true)
        else openLightbox(video)
      }}
      hideOnline={!video.sourceUrl}
      onOnline={() => safeOpenOnline(video.sourceUrl)}
      onReveal={() => safeReveal(video.absolutePath)}
      onDelete={() => setConfirmIds([video.id])}
      onContextMenu={(event) => {
        event.preventDefault()
        setContextMenu({ x: event.clientX, y: event.clientY, video })
      }}
    />
  )

  return (
    <div className="profile-view-shell single-videos-shell">
      <header className="profile-view-header">
        <div className="profile-view-identity">
          <h1>Single videos</h1>
          <p className="profile-view-meta">
            <span className="muted-text">
              {videos.length} video{videos.length === 1 ? '' : 's'}
            </span>
          </p>
        </div>
      </header>

      <form className="single-videos-add" onSubmit={handleAdd}>
        <input
          className="single-videos-add-input"
          placeholder="Paste a TikTok / Instagram / Twitter / YouTube video URL…"
          value={urlInput}
          onChange={(event) => setUrlInput(event.target.value)}
        />
        <button className="ghost-button" disabled={adding || !urlInput.trim()} type="submit">
          {adding ? 'Queuing…' : 'Add video'}
        </button>
      </form>

      {notice ? <div className="runtime-log-window-empty single-videos-notice">{notice}</div> : null}

      {videos.length > 0 ? (
        <div className={`profile-view-toolbar single-videos-toolbar${selectMode ? ' is-selecting' : ''}`}>
          {selectMode ? (
            <>
              <span className="profile-view-selectbar-count">
                {selectedCount > 0
                  ? `${selectedCount} selected`
                  : 'Click items to select · Shift+click for a range'}
              </span>
              <span className="profile-view-selectbar-spacer" />
              <button
                className="ghost-button queue-icon-button"
                onClick={() => setSelectedIds(new Set(filtered.map((video) => video.id)))}
                type="button"
                disabled={filtered.length === 0 || selectedCount === filtered.length}
              >
                Select all
              </button>
              <button
                className="ghost-button queue-icon-button"
                onClick={() => setSelectedIds(new Set())}
                type="button"
                disabled={selectedCount === 0}
              >
                Clear
              </button>
              <button
                className="profile-view-delete-selected"
                onClick={() => setConfirmIds(Array.from(selectedIds))}
                type="button"
                disabled={selectedCount === 0}
                aria-label="Delete selected"
              >
                Delete{selectedCount > 0 ? ` (${selectedCount})` : ''}
              </button>
              <button
                className="ghost-button profile-view-select-toggle is-active"
                onClick={exitSelectMode}
                type="button"
                aria-pressed={true}
              >
                Done
              </button>
            </>
          ) : (
            <>
              <div className="profile-view-segmented" role="group" aria-label="View mode">
                <button
                  className={viewMode === 'day' ? 'is-active' : ''}
                  onClick={() => setViewMode('day')}
                  type="button"
                  aria-pressed={viewMode === 'day'}
                >
                  By day
                </button>
                <button
                  className={viewMode === 'grid' ? 'is-active' : ''}
                  onClick={() => setViewMode('grid')}
                  type="button"
                  aria-pressed={viewMode === 'grid'}
                >
                  All media
                </button>
              </div>
              {providers.length > 0 ? (
                <div className="profile-view-sections" role="group" aria-label="Provider filter">
                  <button
                    className={providerFilter === 'all' ? 'is-active' : ''}
                    onClick={() => setProviderFilter('all')}
                    type="button"
                  >
                    All
                  </button>
                  {providers.map((provider) => (
                    <button
                      key={provider}
                      className={providerFilter === provider ? 'is-active' : ''}
                      onClick={() => setProviderFilter(provider)}
                      type="button"
                    >
                      {providerLabel(provider)}
                    </button>
                  ))}
                </div>
              ) : null}
              <input
                className="single-videos-search"
                placeholder="Filter by uploader or title…"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
              />
              <div className="profile-view-density" role="group" aria-label="Thumbnail size">
                <button
                  className="ghost-button queue-icon-button"
                  onClick={() => setDensityIndex((index) => Math.max(0, index - 1))}
                  disabled={densityIndex <= 0}
                  type="button"
                  aria-label="Smaller thumbnails"
                  title="Smaller thumbnails (more per row)"
                >
                  −
                </button>
                <button
                  className="ghost-button queue-icon-button"
                  onClick={() => setDensityIndex((index) => Math.min(DENSITY_STEPS.length - 1, index + 1))}
                  disabled={densityIndex >= DENSITY_STEPS.length - 1}
                  type="button"
                  aria-label="Larger thumbnails"
                  title="Larger thumbnails (fewer per row)"
                >
                  +
                </button>
              </div>
              <button
                className="ghost-button profile-view-select-toggle"
                onClick={() => setSelectMode(true)}
                type="button"
              >
                Select
              </button>
            </>
          )}
        </div>
      ) : null}

      {error ? <div className="runtime-log-window-error">{error}</div> : null}

      {loading && videos.length === 0 ? (
        <div className="runtime-log-window-empty">Loading…</div>
      ) : filtered.length === 0 ? (
        <div className="runtime-log-window-empty">
          {videos.length === 0
            ? 'No single videos yet. Paste a video URL above to download one.'
            : 'No videos match the current filters.'}
        </div>
      ) : (
        <div className="profile-view-days">
          {viewMode === 'grid' ? (
            <div className="profile-view-grid" style={gridStyle}>
              {filtered.map(renderCard)}
            </div>
          ) : (
            days.map((day) => (
              <section className="profile-view-day" key={day.key}>
                <div className="profile-view-day-header">
                  <span className="eyebrow">{day.label}</span>
                  <span className="pill">{day.videos.length}</span>
                </div>
                <div className="profile-view-grid" style={gridStyle}>
                  {day.videos.map(renderCard)}
                </div>
              </section>
            ))
          )}
        </div>
      )}

      {confirmIds && confirmIds.length > 0 ? (
        <div
          className="profile-view-lightbox profile-view-confirm"
          role="dialog"
          aria-modal="true"
          onClick={() => (deleting ? undefined : setConfirmIds(undefined))}
        >
          <div className="profile-view-confirm-card" onClick={(event) => event.stopPropagation()}>
            <h2>Delete video{confirmIds.length === 1 ? '' : 's'}?</h2>
            <p>
              Move {confirmIds.length} video{confirmIds.length === 1 ? '' : 's'} to the Recycle Bin?
            </p>
            <div className="profile-view-confirm-actions">
              <button
                className="ghost-button"
                onClick={() => setConfirmIds(undefined)}
                type="button"
                disabled={deleting}
              >
                Cancel
              </button>
              <button
                className="ghost-button profile-view-delete"
                onClick={() => void performDelete()}
                type="button"
                disabled={deleting}
              >
                {deleting ? 'Deleting…' : 'Delete'}
              </button>
            </div>
          </div>
        </div>
      ) : null}

      {activeItem && activeVideo ? (
        <MediaLightbox
          fileAbsPath={activeItem.file.absolutePath}
          isVideo={isSingleVideoVideo(activeItem.file.mediaType)}
          hasPrev={lightboxIndex! > 0}
          hasNext={lightboxIndex! < previewItems.length - 1}
          onPrev={() => stepLightbox(-1)}
          onNext={() => stepLightbox(1)}
          onClose={closeLightbox}
          title={activeTitle}
          audioAbsPath={activeVideo.mediaType === 'slideshow' ? activeVideo.audioAbsolutePath : undefined}
          actions={
            <>
              {activeVideo.sourceUrl ? (
                <button className="ghost-button" onClick={() => safeOpenOnline(activeVideo.sourceUrl)} type="button">
                  Open online
                </button>
              ) : null}
              <button className="ghost-button" onClick={() => safeReveal(activeItem.file.absolutePath)} type="button">
                Reveal in folder
              </button>
            </>
          }
        />
      ) : null}

      {contextMenu ? (
        <div
          className="single-videos-context-menu"
          style={{
            top: Math.min(contextMenu.y, window.innerHeight - 60),
            left: Math.min(contextMenu.x, window.innerWidth - 300),
          }}
          onClick={(event) => event.stopPropagation()}
          role="menu"
        >
          <button
            type="button"
            role="menuitem"
            disabled={
              addingProfile ||
              !contextMenu.video.uploader ||
              !TRACKABLE_PROVIDERS.includes(contextMenu.video.provider as ProviderKey)
            }
            onClick={() => void addProfileFromVideo(contextMenu.video)}
          >
            {addingProfile ? 'Adding…' : 'Add profile to NinjaCrawler (download everything)'}
          </button>
        </div>
      ) : null}
    </div>
  )
}
