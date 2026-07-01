import { useCallback, useEffect, useMemo, useState } from 'react'
import type { CSSProperties, FormEvent } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import {
  downloadSingleVideo,
  listSingleVideos,
  openExternalTarget,
  openMediaFile,
  revealMediaInFolder,
} from '../../bridge/desktop'
import type { SingleVideo } from '../../domain/models'

const PROVIDER_LABELS: Record<string, string> = {
  tiktok: 'TikTok',
  instagram: 'Instagram',
  twitter: 'Twitter/X',
  youtube: 'YouTube',
}

function providerLabel(provider: string): string {
  return PROVIDER_LABELS[provider] ?? provider
}

function formatDate(capturedAt?: number): string {
  if (!capturedAt) return ''
  return new Date(capturedAt * 1000).toLocaleDateString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  })
}

export function SingleVideosPage() {
  const [videos, setVideos] = useState<SingleVideo[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string>()
  const [urlInput, setUrlInput] = useState('')
  const [adding, setAdding] = useState(false)
  const [providerFilter, setProviderFilter] = useState<string>('all')
  const [query, setQuery] = useState('')

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

  const handleAdd = useCallback(
    async (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault()
      const url = urlInput.trim()
      if (!url) return
      setAdding(true)
      setError(undefined)
      try {
        await downloadSingleVideo(url)
        setUrlInput('')
        await load()
      } catch (addError) {
        setError(addError instanceof Error ? addError.message : 'Failed to download the video.')
      } finally {
        setAdding(false)
      }
    },
    [urlInput, load],
  )

  // Providers presentes, em ordem estável, para os chips de filtro.
  const providers = useMemo(() => {
    const present = new Set(videos.map((video) => video.provider))
    return ['tiktok', 'instagram', 'twitter', 'youtube'].filter((provider) => present.has(provider))
  }, [videos])

  // Se o filtro aponta para um provider que sumiu, volta a "all".
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
      const haystack = `${video.uploader ?? ''} ${video.title ?? ''}`.toLowerCase()
      return haystack.includes(needle)
    })
  }, [videos, providerFilter, query])

  const gridStyle = { '--pv-thumb-min': '200px' } as CSSProperties

  const renderCard = (video: SingleVideo) => (
    <article className="profile-view-card single-videos-card" key={video.id}>
      <button
        className="profile-view-thumb"
        onClick={() => void openMediaFile(video.absolutePath)}
        type="button"
        title="Open video"
      >
        <video src={convertFileSrc(video.absolutePath)} preload="metadata" muted />
        <span className="profile-view-play" aria-hidden="true">▶</span>
        <span className="profile-view-section" aria-hidden="true">{providerLabel(video.provider)}</span>
        {video.capturedAt ? (
          <span className="profile-view-thumb-overlay" aria-hidden="true">{formatDate(video.capturedAt)}</span>
        ) : null}
      </button>
      <div className="single-videos-card-body">
        {video.uploader ? <span className="single-videos-uploader">@{video.uploader}</span> : null}
        {video.title ? <span className="single-videos-title" title={video.title}>{video.title}</span> : null}
      </div>
      <div className="profile-view-card-actions">
        <button
          className="ghost-button queue-icon-button"
          disabled={!video.sourceUrl}
          onClick={() => video.sourceUrl && void openExternalTarget(video.sourceUrl)}
          type="button"
          title="Open original online"
        >
          Online
        </button>
        <button
          className="ghost-button queue-icon-button"
          onClick={() => void revealMediaInFolder(video.absolutePath)}
          type="button"
          title="Reveal in folder"
        >
          Folder
        </button>
      </div>
    </article>
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
          {adding ? 'Downloading…' : 'Add video'}
        </button>
      </form>

      {videos.length > 0 ? (
        <div className="profile-view-toolbar single-videos-toolbar">
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
          <div className="profile-view-grid" style={gridStyle}>
            {filtered.map(renderCard)}
          </div>
        </div>
      )}
    </div>
  )
}
