import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { CSSProperties } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import {
  deleteSourceMedia,
  loadSourceMediaGallery,
  loadWorkspaceSnapshot,
  openExternalTarget,
  openMediaFile,
  revealMediaInFolder,
  subscribeToProfileViewSource,
  subscribeToSourceSyncQueue,
} from '../../bridge/desktop'
import { DEFAULT_PROVIDER_CATALOG } from '../../domain/defaults'
import type { MediaGalleryPost, ProviderKey, SourceMediaGallery } from '../../domain/models'
import { MediaCard } from './MediaCard'
import { MediaLightbox } from './MediaLightbox'

interface ProfileViewPageProps {
  initialSourceId?: string
}

type ViewMode = 'day' | 'grid'
/** Modos de visualização dos Highlights: por álbum (padrão) ou os comuns. */
type HighlightsMode = 'album' | 'day' | 'grid'

const VIEW_MODE_STORAGE_KEY = 'profileView.mode'
const HIGHLIGHTS_MODE_STORAGE_KEY = 'profileView.highlightsMode'
const DENSITY_STORAGE_KEY = 'profileView.density'

/** Seção dos Highlights do Instagram (ver isEphemeralStorySection). */
const HIGHLIGHTS_SECTION = 'stories'

/** Largura mínima do thumbnail (px) por nível de densidade — do mais denso ao maior. */
const DENSITY_STEPS = [110, 140, 160, 190, 230] as const
const DEFAULT_DENSITY_INDEX = 2 // 160px, o tamanho original

function readStoredMode(): ViewMode {
  try {
    return localStorage.getItem(VIEW_MODE_STORAGE_KEY) === 'grid' ? 'grid' : 'day'
  } catch {
    return 'day'
  }
}

function readStoredHighlightsMode(): HighlightsMode {
  try {
    const stored = localStorage.getItem(HIGHLIGHTS_MODE_STORAGE_KEY)
    if (stored === 'day' || stored === 'grid') return stored
  } catch {
    /* ignore */
  }
  return 'album'
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

interface FlatItem {
  file: { relativePath: string; absolutePath: string; mediaType: string }
  post: MediaGalleryPost
  /** índice da imagem dentro do post (slideshow) */
  fileIndex: number
}

interface DayGroup {
  key: string
  label: string
  posts: MediaGalleryPost[]
}

interface AlbumGroup {
  key: string
  label: string
  posts: MediaGalleryPost[]
  /** Capa do álbum: poster/1ª imagem do post mais recente que tiver. */
  coverSrc?: string
}

function providerDisplayName(provider: ProviderKey): string {
  return DEFAULT_PROVIDER_CATALOG.find((entry) => entry.key === provider)?.displayName ?? provider
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

function isVideo(mediaType: string): boolean {
  return mediaType === 'video'
}

/** Identificador estável de um post para seleção (o 1º arquivo é único por post). */
function postKey(post: MediaGalleryPost): string {
  return post.files[0]?.relativePath ?? post.postId ?? ''
}

/**
 * Atenção à nomenclatura herdada do backend (instagram_connector.rs): a seção
 * `stories_user` ("Stories (user)") são as Stories ao vivo do perfil — efêmeras,
 * expiram em ~24h; já `stories` é populada pela descoberta de Highlights, que são
 * permanentes. Só as efêmeras não têm link "Online" útil.
 */
function isEphemeralStorySection(section: string): boolean {
  return section === 'stories_user'
}

const SECTION_FILTER_ALL = 'all'

/** Ordem estável dos chips de seção (feed antes de reels etc.). */
const SECTION_ORDER = ['timeline', 'reels', 'stories_user', 'stories', 'tagged', 'reposts', 'video']

/**
 * Rótulo da seção. No Instagram, `timeline` é o Feed (distinto dos Reels, que
 * são conteúdos diferentes); nos demais providers vira "Posts". Sobre `stories`
 * vs `stories_user`, ver {@link isEphemeralStorySection}.
 */
function sectionLabel(provider: ProviderKey, section: string): string {
  switch (section) {
    case 'timeline':
      return provider === 'instagram' ? 'Feed' : 'Posts'
    case 'reels':
      return 'Reels'
    case 'stories':
      // `stories` carrega os Highlights no Instagram; nos demais, Stories comuns.
      return provider === 'instagram' ? 'Highlights' : 'Stories'
    case 'stories_user':
      return 'Stories'
    case 'tagged':
      return 'Tagged'
    case 'reposts':
      return 'Reposts'
    case 'video':
      return 'Videos'
    default:
      return section.charAt(0).toUpperCase() + section.slice(1)
  }
}

function sortSections(sections: string[]): string[] {
  return [...sections].sort((a, b) => {
    const ia = SECTION_ORDER.indexOf(a)
    const ib = SECTION_ORDER.indexOf(b)
    return (ia === -1 ? SECTION_ORDER.length : ia) - (ib === -1 ? SECTION_ORDER.length : ib)
  })
}

// Renderização progressiva: perfis com milhares de itens renderizam por janela,
// que cresce conforme o usuário rola (evita montar dezenas de milhares de nós).
const INITIAL_RENDER_LIMIT = 120
const RENDER_BATCH = 120

export function ProfileViewPage({ initialSourceId }: ProfileViewPageProps) {
  const [sourceId, setSourceId] = useState<string | undefined>(initialSourceId)
  const [gallery, setGallery] = useState<SourceMediaGallery>()
  const [avatarPath, setAvatarPath] = useState<string>()
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string>()
  const [lightboxIndex, setLightboxIndex] = useState<number>()
  const [viewMode, setViewMode] = useState<ViewMode>(readStoredMode)
  const [highlightsMode, setHighlightsMode] = useState<HighlightsMode>(readStoredHighlightsMode)
  const [densityIndex, setDensityIndex] = useState<number>(readStoredDensity)
  const [sectionFilter, setSectionFilter] = useState<string>(SECTION_FILTER_ALL)
  const [renderLimit, setRenderLimit] = useState(INITIAL_RENDER_LIMIT)
  const [selectMode, setSelectMode] = useState(false)
  const [selectedKeys, setSelectedKeys] = useState<Set<string>>(() => new Set())
  // Âncora para seleção por intervalo (shift+clique): o último item alternado.
  const selectAnchorRef = useRef<string | null>(null)
  const [confirmPosts, setConfirmPosts] = useState<MediaGalleryPost[]>()
  const [deleting, setDeleting] = useState(false)
  const scrollRef = useRef<HTMLDivElement>(null)
  const sentinelRef = useRef<HTMLDivElement>(null)

  // Persiste as preferências de visualização.
  useEffect(() => {
    try {
      localStorage.setItem(VIEW_MODE_STORAGE_KEY, viewMode)
    } catch {
      /* ignore */
    }
  }, [viewMode])
  useEffect(() => {
    try {
      localStorage.setItem(HIGHLIGHTS_MODE_STORAGE_KEY, highlightsMode)
    } catch {
      /* ignore */
    }
  }, [highlightsMode])
  useEffect(() => {
    try {
      localStorage.setItem(DENSITY_STORAGE_KEY, String(densityIndex))
    } catch {
      /* ignore */
    }
  }, [densityIndex])

  // Reabrir a janela para outro perfil emite o novo sourceId.
  useEffect(() => {
    let unsubscribe: (() => void) | undefined
    void subscribeToProfileViewSource((nextSourceId) => setSourceId(nextSourceId))
      .then((teardown) => {
        unsubscribe = teardown
      })
      .catch(() => undefined)
    return () => unsubscribe?.()
  }, [])

  const load = useCallback(async (id: string) => {
    setLoading(true)
    setError(undefined)
    try {
      const [nextGallery, snapshot] = await Promise.all([
        loadSourceMediaGallery(id),
        loadWorkspaceSnapshot().catch(() => undefined),
      ])
      setGallery(nextGallery)
      const source = snapshot?.sources.find((entry) => entry.id === id)
      setAvatarPath(source?.profileImagePath)
    } catch (loadError) {
      setError(loadError instanceof Error ? loadError.message : 'Failed to load profile media.')
      setGallery(undefined)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    if (sourceId) {
      void load(sourceId)
    }
  }, [sourceId, load])

  // Auto-refresh: quando um sync deste perfil termina (fila do backend), recarrega
  // a galeria em silêncio (sem spinner, preservando a rolagem). O sourceId corrente
  // fica num ref para o listener não reassinar a cada troca de perfil.
  const sourceIdRef = useRef(sourceId)
  sourceIdRef.current = sourceId
  const lastSyncSignatureRef = useRef<string | undefined>(undefined)
  const reloadGallerySilently = useCallback(async (id: string) => {
    try {
      setGallery(await loadSourceMediaGallery(id))
    } catch {
      /* refresh em segundo plano: ignora erros transitórios */
    }
  }, [])
  useEffect(() => {
    // Ao trocar de perfil, zera a assinatura para não herdar o resultado do anterior.
    lastSyncSignatureRef.current = undefined
  }, [sourceId])
  useEffect(() => {
    let unlisten: (() => void) | undefined
    let active = true
    void subscribeToSourceSyncQueue((status) => {
      const id = sourceIdRef.current
      if (!id) return
      const latest = status.recentResults.find((result) => result.sourceId === id)
      const signature = latest ? `${latest.finishedAt}:${latest.status}` : undefined
      if (signature && signature !== lastSyncSignatureRef.current) {
        lastSyncSignatureRef.current = signature
        void reloadGallerySilently(id)
      }
    })
      .then((dispose) => {
        if (active) unlisten = dispose
        else dispose()
      })
      .catch(() => undefined)
    return () => {
      active = false
      unlisten?.()
    }
  }, [reloadGallerySilently])

  // Seções presentes (feed/reels/stories/…), em ordem estável. O chip de
  // Highlights aparece se qualquer post pertence a um álbum, mesmo que o arquivo
  // viva no Feed (associação) e o post não tenha a seção física `stories`.
  const sections = useMemo<string[]>(() => {
    if (!gallery) return []
    const present = new Set<string>()
    for (const post of gallery.posts) {
      present.add(post.section || 'timeline')
      if (post.albums && post.albums.length > 0) {
        present.add(HIGHLIGHTS_SECTION)
      }
    }
    return sortSections([...present])
  }, [gallery])

  // Se o filtro aponta para uma seção que sumiu (troca de perfil), volta a "all".
  useEffect(() => {
    if (sectionFilter !== SECTION_FILTER_ALL && !sections.includes(sectionFilter)) {
      setSectionFilter(SECTION_FILTER_ALL)
    }
  }, [sections, sectionFilter])

  const visiblePosts = useMemo<MediaGalleryPost[]>(() => {
    if (!gallery) return []
    if (sectionFilter === SECTION_FILTER_ALL) return gallery.posts
    // Highlights reúne todos os posts que pertencem a algum álbum (inclusive os
    // que moram no Feed via associação), não só os da seção física `stories`.
    if (sectionFilter === HIGHLIGHTS_SECTION) {
      return gallery.posts.filter((post) => (post.albums?.length ?? 0) > 0)
    }
    return gallery.posts.filter((post) => (post.section || 'timeline') === sectionFilter)
  }, [gallery, sectionFilter])

  // Reinicia a janela ao trocar de perfil ou de filtro (conteúdo diferente).
  useEffect(() => {
    setRenderLimit(INITIAL_RENDER_LIMIT)
  }, [sourceId, sectionFilter])

  // Subconjunto efetivamente montado no DOM (a janela cresce no scroll).
  const renderedPosts = useMemo(
    () => visiblePosts.slice(0, renderLimit),
    [visiblePosts, renderLimit],
  )
  const hasMoreToRender = renderLimit < visiblePosts.length

  const days = useMemo<DayGroup[]>(() => {
    const groups: DayGroup[] = []
    let current: DayGroup | undefined
    for (const post of renderedPosts) {
      const key = dayKey(post.capturedAt)
      if (!current || current.key !== key) {
        current = { key, label: dayLabel(key, post.capturedAt), posts: [] }
        groups.push(current)
      }
      current.posts.push(post)
    }
    return groups
  }, [renderedPosts])

  // Agrupa os Highlights por álbum (subpasta sob `Stories/`). Os posts já vêm
  // do mais recente ao mais antigo, então a 1ª aparição ordena os álbuns pelo
  // item mais recente; o 1º post de cada álbum vira a capa.
  const albums = useMemo<AlbumGroup[]>(() => {
    const groups: AlbumGroup[] = []
    const byKey = new Map<string, AlbumGroup>()
    for (const post of renderedPosts) {
      // Um post pode pertencer a vários álbuns → aparece em cada seção.
      const labels = post.albums && post.albums.length > 0 ? post.albums : ['Highlights']
      for (const label of labels) {
        let group = byKey.get(label)
        if (!group) {
          group = { key: label, label, posts: [] }
          byKey.set(label, group)
          groups.push(group)
        }
        group.posts.push(post)
        if (!group.coverSrc) {
          const cover =
            post.posterPath ??
            post.files.find((file) => !isVideo(file.mediaType))?.absolutePath
          if (cover) group.coverSrc = convertFileSrc(cover)
        }
      }
    }
    return groups
  }, [renderedPosts])

  // "By album" só faz sentido nos Highlights; nas demais seções vale o viewMode.
  // Cobre também o perfil que só tem Highlights (sem chip de seção para clicar).
  const isHighlights =
    sectionFilter === HIGHLIGHTS_SECTION ||
    (sectionFilter === SECTION_FILTER_ALL &&
      sections.length === 1 &&
      sections[0] === HIGHLIGHTS_SECTION)
  const effectiveMode: HighlightsMode = isHighlights ? highlightsMode : viewMode

  // Cresce a janela quando o sentinel (fim da lista) entra na viewport. O
  // rootMargin pré-carrega antes de chegar ao fim; re-observa a cada cresc.
  // para o caso de um lote não preencher a tela inteira.
  useEffect(() => {
    if (!hasMoreToRender) return
    const sentinel = sentinelRef.current
    if (!sentinel) return
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setRenderLimit((current) => current + RENDER_BATCH)
        }
      },
      { root: scrollRef.current ?? null, rootMargin: '800px 0px' },
    )
    observer.observe(sentinel)
    return () => observer.disconnect()
  }, [hasMoreToRender, renderLimit, viewMode])

  // Lista plana (post → cada arquivo) para o lightbox navegar (respeita o filtro).
  const flatItems = useMemo<FlatItem[]>(() => {
    const items: FlatItem[] = []
    for (const post of visiblePosts) {
      post.files.forEach((file, fileIndex) => items.push({ file, post, fileIndex }))
    }
    return items
  }, [visiblePosts])

  const firstFlatIndexByPost = useMemo(() => {
    const map = new Map<MediaGalleryPost, number>()
    flatItems.forEach((item, index) => {
      if (item.fileIndex === 0) {
        map.set(item.post, index)
      }
    })
    return map
  }, [flatItems])

  const openLightboxForPost = useCallback(
    (post: MediaGalleryPost) => {
      const index = firstFlatIndexByPost.get(post)
      if (index !== undefined) {
        setLightboxIndex(index)
      }
    },
    [firstFlatIndexByPost],
  )

  const closeLightbox = useCallback(() => setLightboxIndex(undefined), [])
  const stepLightbox = useCallback(
    (delta: number) => {
      setLightboxIndex((current) => {
        if (current === undefined) return current
        const next = current + delta
        if (next < 0 || next >= flatItems.length) return current
        return next
      })
    },
    [flatItems.length],
  )

  // Teclado no lightbox (captura para ter prioridade sobre o Escape global).
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

  const handleOpenOnline = useCallback((post: MediaGalleryPost, fallbackUrl?: string) => {
    const url = post.postUrl ?? fallbackUrl
    if (url) void openExternalTarget(url)
  }, [])

  // Sai do modo seleção / limpa ao trocar de perfil ou de filtro.
  useEffect(() => {
    setSelectMode(false)
    setSelectedKeys(new Set())
    selectAnchorRef.current = null
  }, [sourceId, sectionFilter])

  // Índice de cada post visível (na ordem exibida) para o range do shift+clique.
  const indexByKey = useMemo(() => {
    const map = new Map<string, number>()
    visiblePosts.forEach((post, index) => map.set(postKey(post), index))
    return map
  }, [visiblePosts])

  /**
   * Seleciona/alterna um post. Marcar qualquer item entra automaticamente no
   * modo de seleção (a barra de ações aparece sem precisar do botão "Select").
   * Com `shift`, seleciona todo o intervalo a partir da última âncora.
   */
  const handleSelect = useCallback(
    (post: MediaGalleryPost, shiftKey: boolean) => {
      const key = postKey(post)
      setSelectMode(true)
      setSelectedKeys((current) => {
        const next = new Set(current)
        const anchor = selectAnchorRef.current
        if (shiftKey && anchor !== null && indexByKey.has(anchor) && indexByKey.has(key)) {
          const a = indexByKey.get(anchor)!
          const b = indexByKey.get(key)!
          const [lo, hi] = a <= b ? [a, b] : [b, a]
          for (let i = lo; i <= hi; i++) {
            next.add(postKey(visiblePosts[i]))
          }
          return next
        }
        if (next.has(key)) next.delete(key)
        else next.add(key)
        return next
      })
      // O shift estende a partir da âncora existente; o clique simples redefine-a.
      if (!shiftKey) selectAnchorRef.current = key
    },
    [indexByKey, visiblePosts],
  )

  const exitSelectMode = useCallback(() => {
    setSelectMode(false)
    setSelectedKeys(new Set())
    selectAnchorRef.current = null
  }, [])

  const selectedPosts = useMemo(
    () => visiblePosts.filter((post) => selectedKeys.has(postKey(post))),
    [visiblePosts, selectedKeys],
  )

  const performDelete = useCallback(async () => {
    if (!sourceId || !confirmPosts || confirmPosts.length === 0) return
    const relativePaths = confirmPosts.flatMap((post) => post.files.map((file) => file.relativePath))
    setDeleting(true)
    setError(undefined)
    try {
      const next = await deleteSourceMedia(sourceId, relativePaths)
      setGallery(next)
      setConfirmPosts(undefined)
      exitSelectMode()
    } catch (deleteError) {
      setError(deleteError instanceof Error ? deleteError.message : 'Failed to delete media.')
    } finally {
      setDeleting(false)
    }
  }, [sourceId, confirmPosts, exitSelectMode])

  const totalMedia = gallery?.posts.reduce((sum, post) => sum + post.files.length, 0) ?? 0
  const activeItem = lightboxIndex !== undefined ? flatItems[lightboxIndex] : undefined
  const gridStyle = { '--pv-thumb-min': `${DENSITY_STEPS[densityIndex]}px` } as CSSProperties

  const renderCard = (post: MediaGalleryPost, key: string) => {
    const thumb = post.files[0]
    if (!thumb) return null
    const posterSrc = post.posterPath ?? (isVideo(thumb.mediaType) ? undefined : thumb.absolutePath)
    const video = isVideo(post.mediaType === 'video' ? 'video' : thumb.mediaType)
    const selected = selectedKeys.has(postKey(post))
    return (
      <MediaCard
        key={key}
        posterAbsPath={posterSrc}
        videoThumbAbsPath={thumb.absolutePath}
        isVideo={video}
        slideshowCount={post.mediaType === 'slideshow' ? post.files.length : undefined}
        badge={
          post.section && post.section !== 'timeline'
            ? gallery
              ? sectionLabel(gallery.provider, post.section)
              : post.section
            : undefined
        }
        overlayText={
          post.capturedAt
            ? new Date(post.capturedAt * 1000).toLocaleTimeString(undefined, {
                hour: '2-digit',
                minute: '2-digit',
              })
            : ''
        }
        selected={selected}
        selectMode={selectMode}
        onToggleSelect={(shiftKey) => handleSelect(post, shiftKey)}
        onOpen={(shiftKey) => {
          if (selectMode) handleSelect(post, shiftKey)
          else if (shiftKey && selectAnchorRef.current !== null) handleSelect(post, true)
          else openLightboxForPost(post)
        }}
        hideOnline={isEphemeralStorySection(post.section)}
        onlineDisabled={!post.postUrl && !gallery?.profileUrl}
        onlineTitle={
          post.postUrl ? 'Open original post online' : 'Original link unavailable — open profile'
        }
        onOnline={() => handleOpenOnline(post, gallery?.profileUrl)}
        onReveal={() => void revealMediaInFolder(thumb.absolutePath)}
        onDelete={() => setConfirmPosts([post])}
      />
    )
  }

  const hasMedia = !!gallery && gallery.posts.length > 0

  return (
    <div className="profile-view-shell">
      <header className="profile-view-header">
        <span className="profile-view-avatar" aria-hidden="true">
          {avatarPath ? (
            <img src={convertFileSrc(avatarPath)} alt="" />
          ) : (
            <span className="profile-view-avatar-fallback">
              {(gallery?.handle ?? '?').replace(/^@/, '').charAt(0).toUpperCase() || '?'}
            </span>
          )}
        </span>
        <div className="profile-view-identity">
          <h1>{gallery?.handle ?? '…'}</h1>
          <p className="profile-view-meta">
            {gallery ? (
              <>
                <span className={`queue-provider-pill provider-${gallery.provider}`}>
                  {providerDisplayName(gallery.provider)}
                </span>
                <span className="muted-text">
                  {gallery.posts.length} post{gallery.posts.length === 1 ? '' : 's'} · {totalMedia} file{totalMedia === 1 ? '' : 's'}
                </span>
              </>
            ) : null}
          </p>
        </div>
        {gallery?.profileUrl ? (
          <button className="ghost-button" onClick={() => void openExternalTarget(gallery.profileUrl)} type="button">
            Open profile online
          </button>
        ) : null}
      </header>

      {hasMedia ? (
        <div className={`profile-view-toolbar${selectMode ? ' is-selecting' : ''}`}>
          {selectMode ? (
            // Selection controls take over the toolbar in place — same row, so
            // entering/leaving the mode never shifts the media grid below.
            <>
              <span className="profile-view-selectbar-count">
                {selectedPosts.length > 0
                  ? `${selectedPosts.length} selected`
                  : 'Click items to select · Shift+click for a range'}
              </span>
              <span className="profile-view-selectbar-spacer" />
              <button
                className="ghost-button queue-icon-button"
                onClick={() => setSelectedKeys(new Set(visiblePosts.map(postKey)))}
                type="button"
                disabled={visiblePosts.length === 0 || selectedPosts.length === visiblePosts.length}
              >
                Select all
              </button>
              <button
                className="ghost-button queue-icon-button"
                onClick={() => setSelectedKeys(new Set())}
                type="button"
                disabled={selectedPosts.length === 0}
              >
                Clear
              </button>
              <button
                className="profile-view-delete-selected"
                onClick={() => setConfirmPosts(selectedPosts)}
                type="button"
                disabled={selectedPosts.length === 0}
                aria-label="Delete selected"
              >
                Delete{selectedPosts.length > 0 ? ` (${selectedPosts.length})` : ''}
              </button>
              <button
                className="ghost-button profile-view-select-toggle is-active"
                onClick={() => exitSelectMode()}
                type="button"
                aria-pressed={true}
              >
                Done
              </button>
            </>
          ) : (
            <>
              <div className="profile-view-segmented" role="group" aria-label="View mode">
                {isHighlights ? (
                  // Nos Highlights o controle alterna o agrupamento por álbum
                  // (padrão) com as formas comuns — preferência própria, persistida.
                  <>
                    <button
                      className={highlightsMode === 'album' ? 'is-active' : ''}
                      onClick={() => setHighlightsMode('album')}
                      type="button"
                      aria-pressed={highlightsMode === 'album'}
                    >
                      By album
                    </button>
                    <button
                      className={highlightsMode === 'day' ? 'is-active' : ''}
                      onClick={() => setHighlightsMode('day')}
                      type="button"
                      aria-pressed={highlightsMode === 'day'}
                    >
                      By day
                    </button>
                    <button
                      className={highlightsMode === 'grid' ? 'is-active' : ''}
                      onClick={() => setHighlightsMode('grid')}
                      type="button"
                      aria-pressed={highlightsMode === 'grid'}
                    >
                      All media
                    </button>
                  </>
                ) : (
                  <>
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
                  </>
                )}
              </div>
              {sections.length > 1 ? (
                <div className="profile-view-sections" role="group" aria-label="Section filter">
                  <button
                    className={sectionFilter === SECTION_FILTER_ALL ? 'is-active' : ''}
                    onClick={() => setSectionFilter(SECTION_FILTER_ALL)}
                    type="button"
                    aria-pressed={sectionFilter === SECTION_FILTER_ALL}
                  >
                    All
                  </button>
                  {sections.map((section) => (
                    <button
                      key={section}
                      className={sectionFilter === section ? 'is-active' : ''}
                      onClick={() => setSectionFilter(section)}
                      type="button"
                      aria-pressed={sectionFilter === section}
                    >
                      {gallery ? sectionLabel(gallery.provider, section) : section}
                    </button>
                  ))}
                </div>
              ) : null}
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
                aria-pressed={false}
              >
                Select
              </button>
            </>
          )}
        </div>
      ) : null}

      {error ? <div className="runtime-log-window-error">{error}</div> : null}

      {loading && !gallery ? (
        <div className="runtime-log-window-empty">Loading media…</div>
      ) : gallery && gallery.posts.length === 0 ? (
        <div className="runtime-log-window-empty">No downloaded media found for this profile.</div>
      ) : (
        <div className="profile-view-days" ref={scrollRef}>
          {effectiveMode === 'album' ? (
            albums.map((album) => (
              <section className="profile-view-day profile-view-album" key={album.key}>
                <div className="profile-view-day-header profile-view-album-header">
                  <span className="profile-view-album-cover" aria-hidden="true">
                    {album.coverSrc ? <img src={album.coverSrc} alt="" loading="lazy" /> : null}
                  </span>
                  <span className="eyebrow profile-view-album-title">{album.label}</span>
                  <span className="pill">{album.posts.length}</span>
                </div>
                <div className="profile-view-grid" style={gridStyle}>
                  {album.posts.map((post, index) => renderCard(post, post.postId ?? `${album.key}-${index}`))}
                </div>
              </section>
            ))
          ) : effectiveMode === 'grid' ? (
            <div className="profile-view-grid" style={gridStyle}>
              {renderedPosts.map((post, index) => renderCard(post, post.postId ?? `post-${index}`))}
            </div>
          ) : (
            days.map((day) => (
              <section className="profile-view-day" key={day.key}>
                <div className="profile-view-day-header">
                  <span className="eyebrow">{day.label}</span>
                  <span className="pill">{day.posts.length}</span>
                </div>
                <div className="profile-view-grid" style={gridStyle}>
                  {day.posts.map((post, index) => renderCard(post, post.postId ?? `${day.key}-${index}`))}
                </div>
              </section>
            ))
          )}
          {hasMoreToRender ? (
            <div ref={sentinelRef} className="profile-view-sentinel" aria-hidden="true" />
          ) : null}
        </div>
      )}

      {confirmPosts && confirmPosts.length > 0 ? (
        <div
          className="profile-view-lightbox profile-view-confirm"
          role="dialog"
          aria-modal="true"
          onClick={() => (deleting ? undefined : setConfirmPosts(undefined))}
        >
          <div className="profile-view-confirm-card" onClick={(event) => event.stopPropagation()}>
            {(() => {
              const fileCount = confirmPosts.reduce((sum, post) => sum + post.files.length, 0)
              return (
                <>
                  <h2>Delete media?</h2>
                  <p>
                    Move {confirmPosts.length} post{confirmPosts.length === 1 ? '' : 's'}
                    {' '}({fileCount} file{fileCount === 1 ? '' : 's'}) to the Recycle Bin?
                  </p>
                  <p className="muted-text">
                    They will be marked as deleted so they are not downloaded again.
                  </p>
                </>
              )
            })()}
            <div className="profile-view-confirm-actions">
              <button
                className="ghost-button"
                onClick={() => setConfirmPosts(undefined)}
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

      {activeItem ? (
        <MediaLightbox
          fileAbsPath={activeItem.file.absolutePath}
          isVideo={isVideo(activeItem.file.mediaType)}
          hasPrev={lightboxIndex! > 0}
          hasNext={lightboxIndex! < flatItems.length - 1}
          onPrev={() => stepLightbox(-1)}
          onNext={() => stepLightbox(1)}
          onClose={closeLightbox}
          actions={
            <>
              {isEphemeralStorySection(activeItem.post.section) ? null : (
                <button
                  className="ghost-button"
                  disabled={!activeItem.post.postUrl && !gallery?.profileUrl}
                  onClick={() => handleOpenOnline(activeItem.post, gallery?.profileUrl)}
                  type="button"
                >
                  Open online
                </button>
              )}
              <button
                className="ghost-button"
                onClick={() => void openMediaFile(activeItem.file.absolutePath)}
                type="button"
              >
                Open file
              </button>
              <button
                className="ghost-button"
                onClick={() => void revealMediaInFolder(activeItem.file.absolutePath)}
                type="button"
              >
                Reveal in folder
              </button>
            </>
          }
        />
      ) : null}
    </div>
  )
}
