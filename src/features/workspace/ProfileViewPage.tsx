import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { CSSProperties } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { convertFileSrc } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import {
  deleteSourceMedia,
  enqueueMediaDedupeScan,
  loadMediaDedupeStatus,
  loadMediaThumbnails,
  loadSourceMediaGallery,
  loadWorkspaceSnapshot,
  openExternalTarget,
  openMediaFile,
  openWorkspaceHealthWindow,
  revealMediaInFolder,
  runSourceSync,
  subscribeToSourceSyncQueue,
  subscribeToDesktopRuntimeEvents,
} from '../../bridge/desktop'
import { DEFAULT_PROVIDER_CATALOG } from '../../domain/defaults'
import type {
  MediaGalleryPost,
  ProviderKey,
  SourceMediaGallery,
  SourceProfile,
  MediaDedupeJobStatus,
} from '../../domain/models'
import { WindowShell } from '../brand/WindowShell'
import { WindowTitlebar } from '../brand/WindowTitlebar'
import { MediaCard } from './MediaCard'
import { MediaLightbox } from './MediaLightbox'

interface ProfileViewPageProps {
  initialSourceId?: string
}

type ViewMode = 'day' | 'grid'
/** Modos de visualização dos Highlights: por álbum (padrão) ou os comuns. */
type HighlightsMode = 'album' | 'day' | 'grid'
/** Modos das abas de conteúdo de terceiros (Likes/Favorites): + por autor. */
type LikesMode = 'day' | 'user' | 'grid'
/** Modo efetivo de renderização, unindo os três controles acima. */
type EffectiveMode = 'album' | 'day' | 'grid' | 'user'
/**
 * Eixo de ordenação. `creation`/`download` são datas (usadas também no
 * agrupamento "By day"); `popularity` ordena pela contagem de views (TikTok).
 */
type SortField = 'creation' | 'download' | 'popularity'
type SortDir = 'newest' | 'oldest'
/** Filtro de tipo de mídia, ortogonal à seção. `photo` inclui slideshows. */
type MediaTypeFilter = 'all' | 'photo' | 'video'
/** Janela de datas dos filtros avançados (aplicada sobre o eixo de ordenação). */
type DateRangeFilter = 'all' | '7d' | '30d' | '90d' | 'year'

const VIEW_MODE_STORAGE_KEY = 'profileView.mode'
const HIGHLIGHTS_MODE_STORAGE_KEY = 'profileView.highlightsMode'
const LIKES_MODE_STORAGE_KEY = 'profileView.likesMode'
const DENSITY_STORAGE_KEY = 'profileView.density'
const SORT_FIELD_STORAGE_KEY = 'profileView.sortField'
const SORT_DIR_STORAGE_KEY = 'profileView.sortDir'
const MEDIA_TYPE_STORAGE_KEY = 'profileView.mediaType'
const PRESETS_STORAGE_KEY = 'profileView.filterPresets'

/** Janelas de data (em segundos) para o filtro de período. `all`/`year` à parte. */
const DATE_RANGE_SECONDS: Record<Exclude<DateRangeFilter, 'all' | 'year'>, number> = {
  '7d': 7 * 86_400,
  '30d': 30 * 86_400,
  '90d': 90 * 86_400,
}

/** Degraus do slider de engajamento mínimo (0 = qualquer). */
const ENGAGEMENT_STEPS = [
  0, 100, 500, 1_000, 5_000, 10_000, 50_000, 100_000, 500_000, 1_000_000,
] as const

/** Índice do degrau atual (para o slider) a partir do valor de engajamento. */
function engagementToSlider(value: number): number {
  const index = ENGAGEMENT_STEPS.findIndex((step) => step >= value)
  return index === -1 ? ENGAGEMENT_STEPS.length - 1 : index
}

/** Valor de engajamento a partir do índice do slider. */
function sliderToEngagement(index: number): number {
  return ENGAGEMENT_STEPS[Math.max(0, Math.min(ENGAGEMENT_STEPS.length - 1, index))] ?? 0
}

const DATE_RANGE_LABEL: Record<DateRangeFilter, string> = {
  all: 'Any date',
  '7d': 'Last 7 days',
  '30d': 'Last 30 days',
  '90d': 'Last 90 days',
  year: 'This year',
}

/** Preset de filtros salvo pelo operador (persistido em localStorage). */
interface FilterPreset {
  id: string
  name: string
  mediaType: MediaTypeFilter
  section: string
  dateRange: DateRangeFilter
  minEngagement: number
  carouselOnly: boolean
}

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

function readStoredLikesMode(): LikesMode {
  try {
    const stored = localStorage.getItem(LIKES_MODE_STORAGE_KEY)
    if (stored === 'day' || stored === 'grid') return stored
  } catch {
    /* ignore */
  }
  return 'user'
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

function readStoredSortField(): SortField {
  try {
    const stored = localStorage.getItem(SORT_FIELD_STORAGE_KEY)
    if (stored === 'download' || stored === 'popularity') return stored
  } catch {
    /* ignore */
  }
  return 'creation'
}

function readStoredSortDir(): SortDir {
  try {
    return localStorage.getItem(SORT_DIR_STORAGE_KEY) === 'oldest' ? 'oldest' : 'newest'
  } catch {
    return 'newest'
  }
}

function readStoredMediaType(): MediaTypeFilter {
  try {
    const stored = localStorage.getItem(MEDIA_TYPE_STORAGE_KEY)
    if (stored === 'photo' || stored === 'video') return stored
  } catch {
    /* ignore */
  }
  return 'all'
}

function readStoredPresets(): FilterPreset[] {
  try {
    const raw = localStorage.getItem(PRESETS_STORAGE_KEY)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed.filter(
      (entry): entry is FilterPreset =>
        typeof entry?.id === 'string' && typeof entry?.name === 'string',
    )
  } catch {
    return []
  }
}

/** Um post é "foto" para o filtro quando NÃO é vídeo (imagem ou slideshow). */
function isPhotoPost(post: MediaGalleryPost): boolean {
  return post.mediaType !== 'video'
}

/** Maior contagem de engajamento do post (views ou likes), para o filtro de faixa. */
function postEngagement(post: MediaGalleryPost): number {
  return Math.max(post.viewCount ?? 0, post.likeCount ?? 0)
}

/** "há 2 h", "há 3 d"… a partir de um ISO. Vazio quando não parseável. */
function formatSyncedAgo(iso?: string): string {
  if (!iso) return ''
  const then = Date.parse(iso)
  if (Number.isNaN(then)) return ''
  const seconds = Math.max(0, Math.round((Date.now() - then) / 1000))
  if (seconds < 60) return 'just now'
  const minutes = Math.round(seconds / 60)
  if (minutes < 60) return `${minutes} min ago`
  const hours = Math.round(minutes / 60)
  if (hours < 24) return `${hours} h ago`
  const days = Math.round(hours / 24)
  if (days < 30) return `${days} d ago`
  const months = Math.round(days / 30)
  if (months < 12) return `${months} mo ago`
  return `${Math.round(months / 12)} y ago`
}

/**
 * Timestamp usado no agrupamento "By day". Para `popularity` (sem eixo de
 * data próprio) cai na data de criação, para que os dias continuem coerentes.
 */
function orderTimestamp(post: MediaGalleryPost, field: SortField): number | undefined {
  return field === 'download' ? post.downloadedAt : post.capturedAt
}

/** Valor comparado na ordenação: data do eixo escolhido ou views (popularity). */
function orderValue(post: MediaGalleryPost, field: SortField): number | undefined {
  if (field === 'popularity') return post.viewCount
  return orderTimestamp(post, field)
}

/** Normaliza para busca: minúsculas, sem acentos e sem o `@` inicial. */
function normalizeForSearch(value: string): string {
  return value
    .normalize('NFD')
    .replace(/\p{Diacritic}/gu, '')
    .toLowerCase()
    .trim()
    .replace(/^@/, '')
}

/** Autor, nomes dos arquivos e post id pesquisáveis nos Likes/Favorites. */
function postAuthorSearchText(post: MediaGalleryPost): string {
  const fileNames = post.files.map((file) => file.relativePath.split('/').pop() ?? '')
  return [post.author ?? '', post.postId ?? '', ...fileNames].join(' ')
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

function compactCount(value: number): string {
  return new Intl.NumberFormat(undefined, {
    notation: 'compact',
    maximumFractionDigits: 1,
  }).format(value)
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
const SECTION_ORDER = [
  'timeline',
  'reels',
  'stories_user',
  'stories',
  'tagged',
  'reposts',
  'video',
  'favorites',
  'likes',
]

/** Seções cujo conteúdo é de OUTROS autores (busca/agrupamento por autor). */
function isAuthorSection(section: string): boolean {
  return section === 'likes' || section === 'favorites'
}

/**
 * Rótulo da seção. No Instagram, `timeline` é o Feed (distinto dos Reels, que
 * são conteúdos diferentes); nos demais providers vira "Timeline". Sobre
 * `stories` vs `stories_user`, ver {@link isEphemeralStorySection}.
 */
function sectionLabel(provider: ProviderKey, section: string): string {
  switch (section) {
    case 'timeline':
      return provider === 'instagram' ? 'Feed' : 'Timeline'
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

// Virtualização por linhas: perfis com milhares de itens só montam as linhas
// visíveis (+ overscan), com a altura total reservada — assim a barra de
// rolagem nativa reflete o volume real e a UI não trava.
/** Gap do grid (`.profile-view-grid`, 0.55rem @16px) — usado no cálculo de colunas. */
const GRID_GAP_PX = 8.8
/** Miniatura tem aspect-ratio 3/4, então altura = largura × 4/3. */
const THUMB_ASPECT = 4 / 3
/** Altura estimada do cabeçalho de dia (o measure real ajusta depois). */
const DAY_HEADER_ESTIMATE = 44
/** Linhas extras montadas fora da viewport, para rolagem sem flashes. */
const ROW_OVERSCAN = 3
/** Quantos thumbnails de vídeo pedir ao backend por lote. */
const THUMBNAIL_BATCH = 32

function profileWindowTitle(handle?: string, provider?: ProviderKey): string {
  if (!handle) return 'Profile View'
  const clean = handle.replace(/^@/, '')
  if (!provider) return `${clean} · Profile View`
  return `${clean} · ${providerDisplayName(provider)}`
}

function isEditableKeyboardTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  const tag = target.tagName
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || target.isContentEditable
}

/** Uma linha virtual: cabeçalho de grupo (dia/autor) ou uma fileira de cards. */
type VirtualRow =
  | { type: 'header'; key: string; label: string; count: number; plain?: boolean }
  | { type: 'grid'; key: string; posts: MediaGalleryPost[] }

export function ProfileViewPage({ initialSourceId }: ProfileViewPageProps) {
  const [sourceId] = useState<string | undefined>(initialSourceId)
  const [gallery, setGallery] = useState<SourceMediaGallery>()
  const [sourceProfile, setSourceProfile] = useState<SourceProfile>()
  const avatarPath = sourceProfile?.profileImagePath
  // Fase 3 — bio longa recolhida por padrão (expande sob demanda).
  const [bioExpanded, setBioExpanded] = useState(false)
  // Estado ao vivo deste perfil na fila de sync (dirige o botão "Sync now").
  const [syncActivity, setSyncActivity] = useState<'idle' | 'queued' | 'running'>('idle')
  const [dedupeStatus, setDedupeStatus] = useState<MediaDedupeJobStatus>()
  const [dedupeLaunching, setDedupeLaunching] = useState(false)
  const [dedupeFeedback, setDedupeFeedback] = useState<{
    tone: 'success' | 'error'
    message: string
  }>()
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string>()
  const [lightboxIndex, setLightboxIndex] = useState<number>()
  const [viewMode, setViewMode] = useState<ViewMode>(readStoredMode)
  const [highlightsMode, setHighlightsMode] = useState<HighlightsMode>(readStoredHighlightsMode)
  const [likesMode, setLikesMode] = useState<LikesMode>(readStoredLikesMode)
  const [densityIndex, setDensityIndex] = useState<number>(readStoredDensity)
  const [sortField, setSortField] = useState<SortField>(readStoredSortField)
  const [sortDir, setSortDir] = useState<SortDir>(readStoredSortDir)
  // Só o TikTok coleta contagem de views hoje; nos demais providers o eixo
  // Popularity fica oculto e uma preferência persistida cai em Creation date.
  const popularitySortAvailable = gallery?.provider === 'tiktok'
  const effectiveSortField: SortField =
    !popularitySortAvailable && sortField === 'popularity' ? 'creation' : sortField
  // Ação pontual (TikTok): enfileira um sync que re-coleta stats da mídia já
  // baixada, sem alterar as opções persistidas do perfil.
  const [statsRefreshState, setStatsRefreshState] = useState<'idle' | 'queueing' | 'queued'>('idle')
  const [sortMenuOpen, setSortMenuOpen] = useState(false)
  const sortMenuRef = useRef<HTMLDivElement>(null)
  // Busca por autor, exclusiva da aba Likes: a lupa expande o campo inline.
  const [likesSearchOpen, setLikesSearchOpen] = useState(false)
  const [likesQuery, setLikesQuery] = useState('')
  const likesSearchInputRef = useRef<HTMLInputElement>(null)
  const [sectionFilter, setSectionFilter] = useState<string>(SECTION_FILTER_ALL)
  // Fase 1 — filtro de tipo de mídia (ortogonal à seção), persistido.
  const [mediaTypeFilter, setMediaTypeFilter] = useState<MediaTypeFilter>(readStoredMediaType)
  // Fase 2 — filtros avançados (popover). Nenhum persiste sozinho: o operador
  // salva o conjunto atual como um preset quando quiser reaproveitá-lo.
  const [filtersOpen, setFiltersOpen] = useState(false)
  const [dateRange, setDateRange] = useState<DateRangeFilter>('all')
  const [minEngagement, setMinEngagement] = useState(0)
  const [carouselOnly, setCarouselOnly] = useState(false)
  const [presets, setPresets] = useState<FilterPreset[]>(readStoredPresets)
  const filtersMenuRef = useRef<HTMLDivElement>(null)
  // Largura útil do container de rolagem (medida), base do cálculo de colunas.
  const [containerWidth, setContainerWidth] = useState(0)
  const [selectMode, setSelectMode] = useState(false)
  const [selectedKeys, setSelectedKeys] = useState<Set<string>>(() => new Set())
  // Âncora para seleção por intervalo (shift+clique): o último item alternado.
  const selectAnchorRef = useRef<string | null>(null)
  const [confirmPosts, setConfirmPosts] = useState<MediaGalleryPost[]>()
  const [deleting, setDeleting] = useState(false)
  const scrollRef = useRef<HTMLDivElement>(null)

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
      localStorage.setItem(LIKES_MODE_STORAGE_KEY, likesMode)
    } catch {
      /* ignore */
    }
  }, [likesMode])
  useEffect(() => {
    try {
      localStorage.setItem(DENSITY_STORAGE_KEY, String(densityIndex))
    } catch {
      /* ignore */
    }
  }, [densityIndex])
  useEffect(() => {
    try {
      localStorage.setItem(SORT_FIELD_STORAGE_KEY, sortField)
    } catch {
      /* ignore */
    }
  }, [sortField])
  useEffect(() => {
    try {
      localStorage.setItem(SORT_DIR_STORAGE_KEY, sortDir)
    } catch {
      /* ignore */
    }
  }, [sortDir])
  useEffect(() => {
    try {
      localStorage.setItem(MEDIA_TYPE_STORAGE_KEY, mediaTypeFilter)
    } catch {
      /* ignore */
    }
  }, [mediaTypeFilter])
  useEffect(() => {
    try {
      localStorage.setItem(PRESETS_STORAGE_KEY, JSON.stringify(presets))
    } catch {
      /* ignore */
    }
  }, [presets])

  // Fecha o menu de ordenação ao clicar fora (mesmo padrão do date picker).
  useEffect(() => {
    if (!sortMenuOpen) return undefined
    const handlePointerDown = (event: PointerEvent) => {
      if (sortMenuRef.current?.contains(event.target as Node)) return
      setSortMenuOpen(false)
    }
    window.addEventListener('pointerdown', handlePointerDown)
    return () => window.removeEventListener('pointerdown', handlePointerDown)
  }, [sortMenuOpen])

  // Fecha o popover de filtros ao clicar fora (idem menu de ordenação).
  useEffect(() => {
    if (!filtersOpen) return undefined
    const handlePointerDown = (event: PointerEvent) => {
      if (filtersMenuRef.current?.contains(event.target as Node)) return
      setFiltersOpen(false)
    }
    window.addEventListener('pointerdown', handlePointerDown)
    return () => window.removeEventListener('pointerdown', handlePointerDown)
  }, [filtersOpen])

  // Foca o campo de busca dos Likes assim que a lupa é expandida.
  useEffect(() => {
    if (likesSearchOpen) likesSearchInputRef.current?.focus()
  }, [likesSearchOpen])

  // Título nativo (Alt+Tab) e da titlebar compartilham a identidade do perfil.
  useEffect(() => {
    const title = profileWindowTitle(gallery?.handle, gallery?.provider)
    try {
      void getCurrentWindow()
        .setTitle(title)
        .catch(() => undefined)
    } catch {
      /* browser / test harness without Tauri */
    }
  }, [gallery?.handle, gallery?.provider])

  const load = useCallback(async (id: string) => {
    setLoading(true)
    setError(undefined)
    try {
      const [nextGallery, snapshot] = await Promise.all([
        loadSourceMediaGallery(id),
        loadWorkspaceSnapshot().catch(() => undefined),
      ])
      setGallery(nextGallery)
      setSourceProfile(snapshot?.sources.find((entry) => entry.id === id))
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
  // a galeria E o snapshot em silêncio (sem spinner, preservando a rolagem) — assim
  // posts, bio/contadores e o "Synced X ago" atualizam sem reabrir a janela. O
  // sourceId corrente fica num ref para o listener não reassinar a cada troca.
  const sourceIdRef = useRef(sourceId)
  sourceIdRef.current = sourceId
  const lastSyncSignatureRef = useRef<string | undefined>(undefined)
  const reloadSilently = useCallback(async (id: string) => {
    try {
      const [nextGallery, snapshot] = await Promise.all([
        loadSourceMediaGallery(id),
        loadWorkspaceSnapshot().catch(() => undefined),
      ])
      setGallery(nextGallery)
      const source = snapshot?.sources.find((entry) => entry.id === id)
      if (source) setSourceProfile(source)
    } catch {
      /* refresh em segundo plano: ignora erros transitórios */
    }
  }, [])
  useEffect(() => {
    // Ao trocar de perfil, zera a assinatura e o estado de sync do anterior.
    lastSyncSignatureRef.current = undefined
    setSyncActivity('idle')
  }, [sourceId])
  useEffect(() => {
    let unlisten: (() => void) | undefined
    let active = true
    void subscribeToSourceSyncQueue((status) => {
      const id = sourceIdRef.current
      if (!id) return
      // Estado ao vivo deste perfil na fila, para o botão refletir "Syncing…".
      const running =
        status.activeSourceId === id || status.runningItems.some((item) => item.sourceId === id)
      const queued = !running && status.queuedItems.some((item) => item.sourceId === id)
      setSyncActivity(running ? 'running' : queued ? 'queued' : 'idle')
      // Conclusão: recarrega galeria + snapshot uma vez por resultado novo.
      const latest = status.recentResults.find((result) => result.sourceId === id)
      const signature = latest ? `${latest.finishedAt}:${latest.status}` : undefined
      if (signature && signature !== lastSyncSignatureRef.current) {
        lastSyncSignatureRef.current = signature
        void reloadSilently(id)
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
  }, [reloadSilently])

  useEffect(() => {
    let unlisten: (() => void) | undefined
    let active = true
    void loadMediaDedupeStatus()
      .then((status) => {
        if (active) setDedupeStatus(status)
      })
      .catch(() => undefined)
    void subscribeToDesktopRuntimeEvents({
      onMediaDedupeStatusChanged: (status) => setDedupeStatus(status),
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
  }, [])

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

  // Contagens por chip — espelham a lógica de filtro (Highlights = posts com álbum).
  const sectionCounts = useMemo(() => {
    const counts = new Map<string, number>()
    if (!gallery) return counts
    counts.set(SECTION_FILTER_ALL, gallery.posts.length)
    let highlights = 0
    for (const post of gallery.posts) {
      const section = post.section || 'timeline'
      counts.set(section, (counts.get(section) ?? 0) + 1)
      if ((post.albums?.length ?? 0) > 0) highlights += 1
    }
    if (highlights > 0) counts.set(HIGHLIGHTS_SECTION, highlights)
    return counts
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

  // Contagens do filtro de tipo de mídia — sobre o recorte de seção corrente,
  // para "Fotos/Vídeos" refletirem o que está sendo olhado (ex.: Feed + Vídeos).
  const mediaTypeCounts = useMemo(() => {
    let photo = 0
    let video = 0
    for (const post of visiblePosts) {
      if (isPhotoPost(post)) photo += 1
      else video += 1
    }
    return { all: visiblePosts.length, photo, video }
  }, [visiblePosts])

  // Se o tipo escolhido não existe na seção atual (ex.: preferência persistida
  // "Fotos" ao abrir um perfil só de vídeos), volta a "All" para não mostrar
  // uma grade vazia sem motivo aparente.
  useEffect(() => {
    if (mediaTypeCounts.all === 0) return
    if (mediaTypeFilter === 'photo' && mediaTypeCounts.photo === 0) setMediaTypeFilter('all')
    else if (mediaTypeFilter === 'video' && mediaTypeCounts.video === 0) setMediaTypeFilter('all')
  }, [mediaTypeFilter, mediaTypeCounts])

  // Quantos filtros avançados (popover) estão ativos — dirige o badge do botão.
  const activeAdvancedFilters =
    (dateRange !== 'all' ? 1 : 0) + (minEngagement > 0 ? 1 : 0) + (carouselOnly ? 1 : 0)

  // Conjunto exibido: seção → tipo de mídia → filtros avançados → busca por autor
  // (só nos Likes) → ordenação pelo eixo escolhido. Posts sem valor no eixo (ex.:
  // like sem download registrado, ou vídeo sem views) vão sempre ao fim,
  // independentemente da direção.
  const sortedPosts = useMemo<MediaGalleryPost[]>(() => {
    let base = visiblePosts
    if (mediaTypeFilter !== 'all') {
      const wantVideo = mediaTypeFilter === 'video'
      base = base.filter((post) => isPhotoPost(post) !== wantVideo)
    }
    if (carouselOnly) {
      base = base.filter((post) => post.mediaType === 'slideshow' || post.files.length > 1)
    }
    if (minEngagement > 0) {
      base = base.filter((post) => postEngagement(post) >= minEngagement)
    }
    if (dateRange !== 'all') {
      const now = Date.now() / 1000
      if (dateRange === 'year') {
        const yearStart = new Date(new Date().getFullYear(), 0, 1).getTime() / 1000
        base = base.filter((post) => {
          const ts = orderTimestamp(post, effectiveSortField)
          return ts != null && ts >= yearStart
        })
      } else {
        const floor = now - DATE_RANGE_SECONDS[dateRange]
        base = base.filter((post) => {
          const ts = orderTimestamp(post, effectiveSortField)
          return ts != null && ts >= floor
        })
      }
    }
    const query = normalizeForSearch(likesQuery)
    if (isAuthorSection(sectionFilter) && query) {
      base = base.filter((post) => normalizeForSearch(postAuthorSearchText(post)).includes(query))
    }
    const dir = sortDir === 'oldest' ? 1 : -1
    return [...base].sort((a, b) => {
      const va = orderValue(a, effectiveSortField)
      const vb = orderValue(b, effectiveSortField)
      if (va == null && vb == null) return 0
      if (va == null) return 1
      if (vb == null) return -1
      if (va !== vb) return (va - vb) * dir
      // Empate por popularidade: desempata pela data de criação (mais recente antes).
      if (effectiveSortField === 'popularity') return (b.capturedAt ?? 0) - (a.capturedAt ?? 0)
      return 0
    })
  }, [
    visiblePosts,
    effectiveSortField,
    sortDir,
    sectionFilter,
    likesQuery,
    mediaTypeFilter,
    carouselOnly,
    minEngagement,
    dateRange,
  ])

  // Agrupa por dia usando Map (não sequencial): quando a ordem não é por data
  // — ex.: ordenação por popularidade — posts do mesmo dia podem não ser
  // adjacentes, então indexamos por chave para não duplicar grupos.
  const days = useMemo<DayGroup[]>(() => {
    const groups: DayGroup[] = []
    const byKey = new Map<string, DayGroup>()
    for (const post of sortedPosts) {
      const ts = orderTimestamp(post, effectiveSortField)
      const key = dayKey(ts)
      let group = byKey.get(key)
      if (!group) {
        group = { key, label: dayLabel(key, ts), posts: [] }
        byKey.set(key, group)
        groups.push(group)
      }
      group.posts.push(post)
    }
    return groups
  }, [sortedPosts, effectiveSortField])

  // Agrupa os Highlights por álbum (subpasta sob `Stories/`). Os posts já vêm
  // do mais recente ao mais antigo, então a 1ª aparição ordena os álbuns pelo
  // item mais recente; o 1º post de cada álbum vira a capa.
  const albums = useMemo<AlbumGroup[]>(() => {
    const groups: AlbumGroup[] = []
    const byKey = new Map<string, AlbumGroup>()
    for (const post of sortedPosts) {
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
  }, [sortedPosts])

  // "By album" só faz sentido nos Highlights; nas demais seções vale o viewMode.
  // Cobre também o perfil que só tem Highlights (sem chip de seção para clicar).
  const isHighlights =
    sectionFilter === HIGHLIGHTS_SECTION ||
    (sectionFilter === SECTION_FILTER_ALL &&
      sections.length === 1 &&
      sections[0] === HIGHLIGHTS_SECTION)
  // Likes/Favorites têm o próprio controle (com "By user"); Highlights o seu
  // ("By album"); o resto usa o viewMode comum.
  const isAuthorTab = isAuthorSection(sectionFilter)
  const effectiveMode: EffectiveMode = isHighlights
    ? highlightsMode
    : isAuthorTab
      ? likesMode
      : viewMode
  // O modo "By album" (Highlights) tem poucos itens e mantém a renderização
  // simples; grid, day e user são virtualizados por linha (onde mora o volume).
  const isVirtualized = effectiveMode !== 'album'

  // Grupos por autor (Likes/Favorites): quem tem mais likes primeiro; dentro
  // do grupo vale a ordenação corrente. Posts sem autor caem em "Unknown".
  const userGroups = useMemo<Array<{ key: string; label: string; posts: MediaGalleryPost[] }>>(() => {
    if (effectiveMode !== 'user') return []
    const byAuthor = new Map<string, { key: string; label: string; posts: MediaGalleryPost[] }>()
    for (const post of sortedPosts) {
      const author = post.author?.trim() ?? ''
      const key = author ? author.toLowerCase() : ''
      let group = byAuthor.get(key)
      if (!group) {
        group = { key: key || 'unknown', label: author ? `@${author}` : 'Unknown', posts: [] }
        byAuthor.set(key, group)
      }
      group.posts.push(post)
    }
    return [...byAuthor.values()].sort((a, b) => b.posts.length - a.posts.length)
  }, [effectiveMode, sortedPosts])

  // Mede a largura útil do container de rolagem (exclui a barra) para derivar as
  // colunas. Reage a resize da janela e à (re)montagem do container.
  useEffect(() => {
    const element = scrollRef.current
    if (!element) return undefined
    const update = () => setContainerWidth(element.clientWidth)
    update()
    const observer = new ResizeObserver(update)
    observer.observe(element)
    return () => observer.disconnect()
  }, [gallery])

  // Colunas pela densidade (thumbs com largura fixa, alinhados à esquerda —
  // poucos itens não esticam para preencher a janela).
  const gridMetrics = useMemo(() => {
    const min = DENSITY_STEPS[densityIndex]
    const width = containerWidth
    if (width <= 0) return { cols: 1, rowHeight: Math.round(min * THUMB_ASPECT + GRID_GAP_PX) }
    const cols = Math.max(1, Math.floor((width + GRID_GAP_PX) / (min + GRID_GAP_PX)))
    return { cols, rowHeight: Math.round(min * THUMB_ASPECT + GRID_GAP_PX) }
  }, [containerWidth, densityIndex])

  // Achata o conteúdo do modo atual (grid, day ou user) em linhas virtuais.
  // Nos modos agrupados, cada grupo gera um cabeçalho + N fileiras de cards.
  const virtualRows = useMemo<VirtualRow[]>(() => {
    const cols = gridMetrics.cols
    const rows: VirtualRow[] = []
    const pushGroup = (key: string, label: string, posts: MediaGalleryPost[], plain?: boolean) => {
      rows.push({ type: 'header', key: `h-${key}`, label, count: posts.length, plain })
      for (let i = 0; i < posts.length; i += cols) {
        rows.push({ type: 'grid', key: `${key}-${i}`, posts: posts.slice(i, i + cols) })
      }
    }
    if (effectiveMode === 'grid') {
      for (let i = 0; i < sortedPosts.length; i += cols) {
        rows.push({ type: 'grid', key: `g-${i}`, posts: sortedPosts.slice(i, i + cols) })
      }
    } else if (effectiveMode === 'day') {
      for (const day of days) pushGroup(day.key, day.label, day.posts)
    } else if (effectiveMode === 'user') {
      // `plain`: o handle preserva a caixa original (sem o uppercase do .eyebrow).
      for (const group of userGroups) pushGroup(group.key, group.label, group.posts, true)
    }
    return rows
  }, [effectiveMode, sortedPosts, days, userGroups, gridMetrics.cols])

  const rowVirtualizer = useVirtualizer({
    count: virtualRows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) =>
      virtualRows[index]?.type === 'header' ? DAY_HEADER_ESTIMATE : gridMetrics.rowHeight,
    overscan: ROW_OVERSCAN,
    getItemKey: (index) => virtualRows[index]?.key ?? index,
  })

  // Densidade/largura mudam a altura das linhas: descarta as medidas em cache
  // para o virtualizer re-medir com o novo tamanho.
  useEffect(() => {
    rowVirtualizer.measure()
  }, [rowVirtualizer, gridMetrics.rowHeight, containerWidth])

  // Trocar de perfil ou de seção muda o conteúdo por completo — volta ao topo
  // (com virtualização o scrollTop não reseta sozinho e ficaria fora do fim).
  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = 0
  }, [sourceId, sectionFilter])

  // Thumbnails gerados pelo backend, por caminho absoluto: vídeo via ffmpeg
  // (o grid nunca monta <video> quando existem — media elements aos milhares
  // travam o webview) e foto via crate image (~480px, evita decodificar a
  // imagem em resolução original no webview). `''` marca falha de geração (não
  // re-pede). Cache válido entre perfis/abas.
  const [thumbs, setThumbs] = useState<ReadonlyMap<string, string>>(() => new Map())
  // ffmpeg ausente → cai no thumb por <video> (gated pelo isScrolling).
  const [thumbsUnavailable, setThumbsUnavailable] = useState(false)
  const pendingThumbsRef = useRef<Set<string>>(new Set())
  const isScrolling = rowVirtualizer.isScrolling
  const virtualItems = rowVirtualizer.getVirtualItems()
  const rangeKey = virtualItems.length
    ? `${virtualItems[0].index}:${virtualItems[virtualItems.length - 1].index}`
    : ''
  useEffect(() => {
    // Durante a rolagem não adianta pedir: a viewport ainda está mudando.
    if (isScrolling || !isVirtualized) return
    const wanted: string[] = []
    for (const item of rowVirtualizer.getVirtualItems()) {
      const row = virtualRows[item.index]
      if (!row || row.type !== 'grid') continue
      for (const post of row.posts) {
        const file = post.files[0]
        if (!file) continue
        if (post.posterPath) continue
        // Vídeo depende de ffmpeg; se indisponível não adianta pedir (cai no
        // <video>). Foto gera sempre (crate image, sem ffmpeg).
        if (isVideo(file.mediaType) && thumbsUnavailable) continue
        const key = file.absolutePath
        if (!thumbs.has(key) && !pendingThumbsRef.current.has(key)) {
          wanted.push(key)
          if (wanted.length >= THUMBNAIL_BATCH) break
        }
      }
      if (wanted.length >= THUMBNAIL_BATCH) break
    }
    if (wanted.length === 0) return
    for (const key of wanted) pendingThumbsRef.current.add(key)
    let cancelled = false
    void loadMediaThumbnails(wanted)
      .then((batch) => {
        for (const key of wanted) pendingThumbsRef.current.delete(key)
        if (cancelled) return
        // available=false sinaliza só ffmpeg ausente (afeta vídeo); as fotos do
        // lote ainda vêm no batch, então mesclamos de qualquer forma.
        if (!batch.available) setThumbsUnavailable(true)
        setThumbs((current) => {
          const next = new Map(current)
          // Sem resultado = falha de geração → '' impede novo pedido.
          for (const key of wanted) next.set(key, batch.thumbs[key] ?? '')
          return next
        })
      })
      .catch(() => {
        for (const key of wanted) pendingThumbsRef.current.delete(key)
      })
    return () => {
      cancelled = true
    }
    // rangeKey cobre o deslocamento da janela; thumbs reagenda o próximo lote.
  }, [rangeKey, isScrolling, thumbs, thumbsUnavailable, isVirtualized, rowVirtualizer, virtualRows])

  // Lista plana (post → cada arquivo) para o lightbox navegar (respeita filtro
  // e ordenação, para o avançar/voltar seguir a ordem exibida).
  const flatItems = useMemo<FlatItem[]>(() => {
    const items: FlatItem[] = []
    for (const post of sortedPosts) {
      post.files.forEach((file, fileIndex) => items.push({ file, post, fileIndex }))
    }
    return items
  }, [sortedPosts])

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

  // A lista encolheu (exclusão/refresh) e o índice estourou: gruda no último
  // item ou fecha quando não sobrou nada.
  useEffect(() => {
    if (lightboxIndex !== undefined && lightboxIndex >= flatItems.length) {
      setLightboxIndex(flatItems.length > 0 ? flatItems.length - 1 : undefined)
    }
  }, [flatItems.length, lightboxIndex])

  /**
   * Shift+Del no lightbox: manda o post ativo para a Lixeira SEM diálogo (o
   * Shift é a confirmação) — o backend também o marca como excluído no ledger
   * para nunca ser baixado de novo. O índice reancora no 1º arquivo do post,
   * então após a exclusão a mesma posição exibe o item seguinte.
   */
  const deleteActivePost = useCallback(async () => {
    if (!sourceId || deleting || lightboxIndex === undefined) return
    const item = flatItems[lightboxIndex]
    if (!item) return
    const anchor = firstFlatIndexByPost.get(item.post) ?? lightboxIndex
    setDeleting(true)
    setError(undefined)
    try {
      const next = await deleteSourceMedia(
        sourceId,
        item.post.files.map((file) => file.relativePath),
      )
      setGallery(next)
      setLightboxIndex(anchor)
    } catch (deleteError) {
      setError(deleteError instanceof Error ? deleteError.message : 'Failed to delete media.')
    } finally {
      setDeleting(false)
    }
  }, [sourceId, deleting, lightboxIndex, flatItems, firstFlatIndexByPost])
  // Ref para o listener de teclado (estável) sempre ver a versão corrente.
  const deleteActivePostRef = useRef(deleteActivePost)
  deleteActivePostRef.current = deleteActivePost

  // Atalho destrutivo próprio do Profile View; navegação/seek ficam no
  // MediaLightbox compartilhado.
  const lightboxOpen = lightboxIndex !== undefined
  const lightboxOpenRef = useRef(lightboxOpen)
  lightboxOpenRef.current = lightboxOpen
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (!lightboxOpenRef.current) return
      if (event.key === 'Delete' && event.shiftKey) {
        event.preventDefault()
        event.stopImmediatePropagation()
        void deleteActivePostRef.current()
      }
    }
    document.addEventListener('keydown', handler, true)
    return () => document.removeEventListener('keydown', handler, true)
  }, [])

  const handleActionError = useCallback((action: string, actionError: unknown) => {
    const message = actionError instanceof Error ? actionError.message : String(actionError)
    setError(`${action} failed. ${message}`)
  }, [])

  const handleOpenOnline = useCallback((post: MediaGalleryPost, fallbackUrl?: string) => {
    const url = post.postUrl ?? fallbackUrl
    if (url) void openExternalTarget(url).catch((actionError) => handleActionError('Open online', actionError))
  }, [handleActionError])

  const handleOpenProfileOnline = useCallback(() => {
    const url = gallery?.profileUrl
    if (url) {
      void openExternalTarget(url).catch((actionError) => handleActionError('Open profile online', actionError))
    }
  }, [gallery?.profileUrl, handleActionError])

  const handleOpenMediaFile = useCallback((path: string) => {
    void openMediaFile(path).catch((actionError) => handleActionError('Open file', actionError))
  }, [handleActionError])

  const handleRevealMedia = useCallback((path: string) => {
    void revealMediaInFolder(path).catch((actionError) => handleActionError('Reveal in folder', actionError))
  }, [handleActionError])

  // Enfileira um sync normal deste perfil. O estado ao vivo (queued/running) e o
  // refresh ao terminar vêm da assinatura da fila; aqui só marcamos otimista para
  // o botão reagir na hora, sem esperar o primeiro tick da fila.
  const handleSyncNow = useCallback(() => {
    if (!sourceId || syncActivity !== 'idle') return
    setSyncActivity('queued')
    runSourceSync(sourceId, { trigger: 'manual' }).catch((syncError) => {
      setSyncActivity('idle')
      const message = syncError instanceof Error ? syncError.message : String(syncError)
      window.alert(`Failed to start the sync.\n${message}`)
    })
  }, [sourceId, syncActivity])

  const mediaCleanupActive = Boolean(
    dedupeStatus && ['queued', 'scanning', 'applying'].includes(dedupeStatus.state),
  )
  const thisProfileDedupeActive = Boolean(
    mediaCleanupActive && sourceId && dedupeStatus?.sourceScope === sourceId,
  )

  const handleDedupe = useCallback(async () => {
    if (!sourceId || dedupeLaunching) return
    if (thisProfileDedupeActive) {
      await openWorkspaceHealthWindow({ initialTab: 'storage' }).catch((actionError) =>
        handleActionError('Open media cleanup', actionError),
      )
      return
    }
    setDedupeLaunching(true)
    setDedupeFeedback(undefined)
    try {
      const status = await enqueueMediaDedupeScan({
        sourceId,
        ...(gallery?.provider ? { provider: gallery.provider } : {}),
        resourceProfile: 'balanced',
      })
      setDedupeStatus(status)
      setDedupeFeedback({
        tone: 'success',
        message: 'Profile dedupe started in Workspace Health › Storage & Cleanup.',
      })
      try {
        await openWorkspaceHealthWindow({ initialTab: 'storage' })
      } catch {
        setDedupeFeedback({
          tone: 'success',
          message: 'Profile dedupe started. Open Tools › Workspace Health › Storage & Cleanup to follow it.',
        })
      }
    } catch (dedupeError) {
      const message = dedupeError instanceof Error ? dedupeError.message : String(dedupeError)
      setDedupeFeedback({ tone: 'error', message: `Could not start profile dedupe. ${message}` })
    } finally {
      setDedupeLaunching(false)
    }
  }, [dedupeLaunching, gallery?.provider, handleActionError, sourceId, thisProfileDedupeActive])

  const handleRefreshMediaStats = useCallback(() => {
    if (!sourceId || statsRefreshState === 'queueing') return
    setStatsRefreshState('queueing')
    runSourceSync(sourceId, {
      trigger: 'manual_stats_refresh',
      runMode: 'refresh_media_stats',
    })
      .then(() => {
        setStatsRefreshState('queued')
        window.setTimeout(() => setStatsRefreshState('idle'), 4000)
      })
      .catch((refreshError) => {
        setStatsRefreshState('idle')
        const message = refreshError instanceof Error ? refreshError.message : String(refreshError)
        window.alert(`Failed to queue the media stats refresh.\n${message}`)
      })
  }, [sourceId, statsRefreshState])

  // Sai do modo seleção / limpa ao trocar de perfil ou de filtro. A busca por
  // autor também é zerada (ela só existe na aba Likes).
  useEffect(() => {
    setSelectMode(false)
    setSelectedKeys(new Set())
    selectAnchorRef.current = null
    setLikesSearchOpen(false)
    setLikesQuery('')
  }, [sourceId, sectionFilter])

  // Filtros avançados são efêmeros por perfil: trocar de perfil zera período,
  // engajamento e "só carrosséis" (mediaType persiste como preferência do
  // operador). Assim um recorte agressivo não some com a mídia do próximo perfil.
  useEffect(() => {
    setDateRange('all')
    setMinEngagement(0)
    setCarouselOnly(false)
    setFiltersOpen(false)
    setBioExpanded(false)
  }, [sourceId])

  const clearAdvancedFilters = useCallback(() => {
    setDateRange('all')
    setMinEngagement(0)
    setCarouselOnly(false)
  }, [])

  const clearAllFilters = useCallback(() => {
    clearAdvancedFilters()
    setMediaTypeFilter('all')
  }, [clearAdvancedFilters])

  const applyPreset = useCallback(
    (preset: FilterPreset) => {
      setMediaTypeFilter(preset.mediaType)
      setDateRange(preset.dateRange)
      setMinEngagement(preset.minEngagement)
      setCarouselOnly(preset.carouselOnly)
      // Só aplica a seção do preset se ela existir neste perfil.
      if (preset.section === SECTION_FILTER_ALL || sections.includes(preset.section)) {
        setSectionFilter(preset.section)
      }
      setFiltersOpen(false)
    },
    [sections],
  )

  const saveCurrentPreset = useCallback(() => {
    const name = window.prompt('Name this filter preset:')?.trim()
    if (!name) return
    const preset: FilterPreset = {
      id: `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
      name,
      mediaType: mediaTypeFilter,
      section: sectionFilter,
      dateRange,
      minEngagement,
      carouselOnly,
    }
    setPresets((current) => [...current, preset])
  }, [mediaTypeFilter, sectionFilter, dateRange, minEngagement, carouselOnly])

  const deletePreset = useCallback((id: string) => {
    setPresets((current) => current.filter((preset) => preset.id !== id))
  }, [])

  // Índice de cada post visível (na ordem exibida) para o range do shift+clique.
  const indexByKey = useMemo(() => {
    const map = new Map<string, number>()
    sortedPosts.forEach((post, index) => map.set(postKey(post), index))
    return map
  }, [sortedPosts])

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
            next.add(postKey(sortedPosts[i]))
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
    [indexByKey, sortedPosts],
  )

  const exitSelectMode = useCallback(() => {
    setSelectMode(false)
    setSelectedKeys(new Set())
    selectAnchorRef.current = null
  }, [])

  // Escape: lightbox (MediaLightbox) → confirm → select → sort/search → window close.
  // Capture phase + stopImmediatePropagation impede o entrypoint de fechar cedo.
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      if (lightboxOpenRef.current) return
      if (confirmPosts && confirmPosts.length > 0) {
        if (deleting) {
          event.preventDefault()
          event.stopImmediatePropagation()
          return
        }
        event.preventDefault()
        event.stopImmediatePropagation()
        setConfirmPosts(undefined)
        return
      }
      if (selectMode) {
        event.preventDefault()
        event.stopImmediatePropagation()
        exitSelectMode()
        return
      }
      if (sortMenuOpen) {
        event.preventDefault()
        event.stopImmediatePropagation()
        setSortMenuOpen(false)
        return
      }
      if (filtersOpen) {
        event.preventDefault()
        event.stopImmediatePropagation()
        setFiltersOpen(false)
        return
      }
      if (likesSearchOpen) {
        event.preventDefault()
        event.stopImmediatePropagation()
        setLikesQuery('')
        setLikesSearchOpen(false)
      }
    }
    document.addEventListener('keydown', handler, true)
    return () => document.removeEventListener('keydown', handler, true)
  }, [confirmPosts, deleting, selectMode, sortMenuOpen, filtersOpen, likesSearchOpen, exitSelectMode])

  // Atalhos de operador: S = select mode (fora de campos editáveis / overlays).
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (isEditableKeyboardTarget(event.target)) return
      if (lightboxOpenRef.current || (confirmPosts && confirmPosts.length > 0)) return
      if (event.key === 's' || event.key === 'S') {
        if (event.ctrlKey || event.metaKey || event.altKey) return
        event.preventDefault()
        if (selectMode) exitSelectMode()
        else setSelectMode(true)
      }
    }
    document.addEventListener('keydown', handler)
    return () => document.removeEventListener('keydown', handler)
  }, [confirmPosts, selectMode, exitSelectMode])

  const selectedPosts = useMemo(
    () => sortedPosts.filter((post) => selectedKeys.has(postKey(post))),
    [sortedPosts, selectedKeys],
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
  // Album (não virtualizado) usa colunas auto-fill; o grid virtualizado fixa o
  // número de colunas (`--pv-cols`) para todas as linhas ficarem alinhadas.
  const gridStyle = { '--pv-thumb-min': `${DENSITY_STEPS[densityIndex]}px` } as CSSProperties
  const virtualGridStyle = {
    '--pv-thumb-min': `${DENSITY_STEPS[densityIndex]}px`,
    '--pv-cols': gridMetrics.cols,
  } as CSSProperties
  const sortFieldLabel =
    effectiveSortField === 'popularity'
      ? 'Popularity'
      : effectiveSortField === 'download'
        ? 'Download'
        : 'Creation'
  const sortDirectionLabel =
    effectiveSortField === 'popularity'
      ? sortDir === 'newest'
        ? 'Most viewed'
        : 'Least viewed'
      : sortDir === 'newest'
        ? 'Newest'
        : 'Oldest'
  const sortControlLabel = `${sortFieldLabel}: ${sortDirectionLabel}`
  const densityLevelLabel = `${densityIndex + 1}/${DENSITY_STEPS.length}`
  const handleDisplay = gallery?.handle?.replace(/^@/, '') ?? ''
  const titlebarTitle = handleDisplay
    ? `@${handleDisplay}`
    : 'Profile View'
  const titlebarStatus = gallery
    ? loading
      ? 'Loading…'
      : sortedPosts.length === gallery.posts.length
        ? `${gallery.posts.length} posts · ${totalMedia} files`
        : `${sortedPosts.length} of ${gallery.posts.length} posts`
    : loading
      ? 'Loading…'
      : undefined
  const activeSectionLabel =
    sectionFilter === SECTION_FILTER_ALL || !gallery
      ? undefined
      : sectionLabel(gallery.provider, sectionFilter)
  const filteredEmpty =
    !!gallery && gallery.posts.length > 0 && sortedPosts.length === 0
  // Filtros ativos além da seção (para o empty state oferecer "limpar").
  const hasActiveContentFilters = mediaTypeFilter !== 'all' || activeAdvancedFilters > 0

  // Header enriquecido (Fase 1) — tudo já vem do SourceProfile do snapshot.
  const rawDisplayName = sourceProfile?.displayName?.trim() ?? ''
  const displayName =
    rawDisplayName && rawDisplayName.replace(/^@/, '').toLowerCase() !== handleDisplay.toLowerCase()
      ? rawDisplayName
      : ''
  // Fase 3 — metadados de perfil da última sync (vêm da galeria).
  const bioText = gallery?.biography?.trim() ?? ''
  const bioIsLong = bioText.length > 170 || bioText.includes('\n')
  const isVerified = gallery?.isVerified === true
  // A contagem local de posts já aparece na linha de meta; o `mediaCount` remoto
  // é redundante e, em alguns endpoints do Instagram, vem inconsistente — então
  // o header mostra só seguidores/seguindo (valores confiáveis do provider).
  const profileStats: Array<{ key: string; label: string; value: number }> = []
  if (gallery?.followerCount != null)
    profileStats.push({ key: 'followers', label: 'followers', value: gallery.followerCount })
  if (gallery?.followingCount != null)
    profileStats.push({ key: 'following', label: 'following', value: gallery.followingCount })
  const syncedAgo = formatSyncedAgo(sourceProfile?.lastSyncedAt)
  const hasSyncProblem = Boolean(sourceProfile?.syncProblemCode)
  const syncHealthLabel = hasSyncProblem
    ? sourceProfile?.syncProblemMessage?.trim() || 'Last sync had a problem'
    : syncedAgo
      ? `Synced ${syncedAgo} · no errors`
      : 'Synced · no errors'
  const profileLabels = sourceProfile?.labels ?? []

  // Sticky group header for virtualized day/user modes (absolute rows can't sticky natively).
  const stickyGroupHeader = useMemo(() => {
    if (!isVirtualized || effectiveMode === 'grid' || virtualItems.length === 0) return null
    const firstVisible = virtualItems[0]?.index ?? 0
    let header: Extract<VirtualRow, { type: 'header' }> | null = null
    for (let i = 0; i <= firstVisible; i++) {
      const row = virtualRows[i]
      if (row?.type === 'header') header = row
    }
    if (!header) return null
    // Hide when the live header row is the topmost visible item (avoid double label).
    if (virtualRows[firstVisible]?.type === 'header' && (virtualItems[0]?.start ?? 0) <= 2) {
      return null
    }
    return header
  }, [isVirtualized, effectiveMode, virtualItems, virtualRows])

  const renderCard = (post: MediaGalleryPost, key: string) => {
    const thumb = post.files[0]
    if (!thumb) return null
    const thumbIsVideo = isVideo(thumb.mediaType)
    // Poster: cover em disco > thumb gerado (.thumbs) > o próprio arquivo. Foto
    // sem thumb ainda cai no original full-res (transitório, até o thumb
    // chegar); vídeo sem thumb cai no <video>. Nunca imagem quebrada.
    const generatedThumb = thumbs.get(thumb.absolutePath)
    const posterSrc =
      post.posterPath
      ?? (thumbIsVideo ? generatedThumb || undefined : generatedThumb || thumb.absolutePath)
    // O <video> como thumb é o último recurso (ffmpeg indisponível) e nunca
    // monta durante a rolagem — media elements em massa travam o webview. O
    // modo álbum (Highlights, poucos itens, fora do virtualizer/efeito de
    // thumbs) mantém o comportamento antigo.
    const allowVideoThumb = !isVirtualized || (thumbsUnavailable && !isScrolling)
    const video = isVideo(post.mediaType === 'video' ? 'video' : thumb.mediaType)
    const selected = selectedKeys.has(postKey(post))
    return (
      <MediaCard
        key={key}
        posterAbsPath={posterSrc}
        // Se um jpg derivado estiver corrompido/inacessível, o MediaCard cai
        // para o próprio vídeo apenas naquele card; não deixa ícone quebrado.
        videoThumbAbsPath={
          thumbIsVideo && (allowVideoThumb || Boolean(generatedThumb))
            ? thumb.absolutePath
            : undefined
        }
        isVideo={video}
        slideshowCount={post.mediaType === 'slideshow' ? post.files.length : undefined}
        badge={
          // Com filtro de seção ativo o badge só repete o chip — mostre só em All.
          sectionFilter === SECTION_FILTER_ALL && post.section && post.section !== 'timeline'
            ? gallery
              ? sectionLabel(gallery.provider, post.section)
              : post.section
            : undefined
        }
        overlayText={
          [
            post.viewCount !== undefined ? `${compactCount(post.viewCount)} views` : '',
            post.capturedAt
              ? new Date(post.capturedAt * 1000).toLocaleTimeString(undefined, {
                hour: '2-digit',
                minute: '2-digit',
              })
              : '',
          ].filter(Boolean).join(' · ')
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
        onReveal={() => handleRevealMedia(thumb.absolutePath)}
        onDelete={() => setConfirmPosts([post])}
      />
    )
  }

  const hasMedia = !!gallery && gallery.posts.length > 0

  return (
    <WindowShell
      className="profile-view-window-shell"
      contentClassName="profile-view-window-content"
      density="compact"
      titlebar={
        <WindowTitlebar
          title={titlebarTitle}
          trailing={
            titlebarStatus ? (
              <span className="window-titlebar-status-meta">{titlebarStatus}</span>
            ) : undefined
          }
        />
      }
    >
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
          <h1 className="profile-view-handle-row">
            <span className="profile-view-handle-text">{gallery?.handle ?? '…'}</span>
            {isVerified ? (
              <span className="profile-view-verified" title="Verified account" aria-label="Verified account">
                <svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true" focusable="false">
                  <path
                    d="M12 2l2.35 1.76 2.94-.27 1.02 2.77 2.55 1.5-.82 2.84.82 2.84-2.55 1.5-1.02 2.77-2.94-.27L12 22l-2.35-1.76-2.94.27-1.02-2.77-2.55-1.5.82-2.84L3.14 8.53l2.55-1.5 1.02-2.77 2.94.27z"
                    fill="currentColor"
                  />
                  <path d="M8.6 12.2l2.3 2.3 4.5-4.7" fill="none" stroke="var(--bg-window-shell)" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" />
                </svg>
              </span>
            ) : null}
          </h1>
          {displayName ? <p className="profile-view-display-name">{displayName}</p> : null}
          {profileStats.length > 0 ? (
            <p className="profile-view-stats" aria-label="Profile counts">
              {profileStats.map((stat) => (
                <span key={stat.key} className="profile-view-stat">
                  <b>{compactCount(stat.value)}</b> {stat.label}
                </span>
              ))}
            </p>
          ) : null}
          {bioText ? (
            <div className="profile-view-bio-wrap">
              <span className={`profile-view-bio${bioIsLong && !bioExpanded ? ' is-clamped' : ''}`}>
                {bioText}
              </span>
              {bioIsLong ? (
                <button
                  className="profile-view-bio-toggle"
                  onClick={() => setBioExpanded((open) => !open)}
                  type="button"
                >
                  {bioExpanded ? 'show less' : 'show more'}
                </button>
              ) : null}
            </div>
          ) : null}
          <p className="profile-view-meta">
            {gallery ? (
              <>
                <span className={`queue-provider-pill provider-${gallery.provider}`}>
                  {providerDisplayName(gallery.provider)}
                </span>
                <span className="muted-text">
                  {gallery.posts.length} post{gallery.posts.length === 1 ? '' : 's'}
                  {' · '}
                  {totalMedia} file{totalMedia === 1 ? '' : 's'}
                  {activeSectionLabel ? ` · ${activeSectionLabel}` : ''}
                </span>
              </>
            ) : null}
          </p>
          {gallery ? (
            <div className="profile-view-identity-meta">
              <span
                className={`profile-view-sync-chip${hasSyncProblem ? ' has-problem' : ''}`}
                title={syncHealthLabel}
              >
                <span className="profile-view-sync-led" aria-hidden="true" />
                {syncHealthLabel}
              </span>
              {profileLabels.length > 0 ? (
                <span className="profile-view-identity-labels">
                  {profileLabels.map((label) => (
                    <span key={label} className="profile-view-identity-label">
                      {label}
                    </span>
                  ))}
                </span>
              ) : null}
            </div>
          ) : null}
        </div>
        <div className="profile-view-header-actions">
          {sourceId ? (
            <button
              className={`ghost-button profile-view-header-action profile-view-sync-now is-${syncActivity}`}
              onClick={handleSyncNow}
              disabled={syncActivity !== 'idle'}
              type="button"
              aria-label="Sync now"
              title={
                syncActivity === 'running'
                  ? 'Sync in progress'
                  : syncActivity === 'queued'
                    ? 'Sync queued'
                    : 'Sync this profile now'
              }
            >
              <svg
                className={syncActivity === 'running' ? 'profile-view-sync-spin' : ''}
                viewBox="0 0 24 24"
                width="15"
                height="15"
                aria-hidden="true"
                focusable="false"
              >
                <path
                  d="M20 11a8 8 0 1 0-2.34 5.66M20 4v7h-7"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.8"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              </svg>
              <span className="profile-view-header-action-label">
                {syncActivity === 'running'
                  ? 'Syncing…'
                  : syncActivity === 'queued'
                    ? 'Queued'
                    : 'Sync now'}
              </span>
            </button>
          ) : null}
          {sourceId ? (
            <button
              className={`ghost-button profile-view-header-action profile-view-sync-now profile-view-dedupe${thisProfileDedupeActive ? ' is-running' : ''}`}
              disabled={dedupeLaunching || (mediaCleanupActive && !thisProfileDedupeActive)}
              onClick={() => void handleDedupe()}
              type="button"
              aria-label={thisProfileDedupeActive ? 'View profile dedupe progress' : 'Dedupe profile media'}
              title={
                thisProfileDedupeActive
                  ? 'View this profile scan in Workspace Health › Storage & Cleanup'
                  : mediaCleanupActive
                    ? 'Another media cleanup job is already running'
                    : 'Scan this profile for duplicate media'
              }
            >
              <svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true" focusable="false">
                <rect x="4" y="5" width="10" height="10" rx="2" fill="none" stroke="currentColor" strokeWidth="1.7" />
                <path d="M10 9h6a4 4 0 0 1 4 4v2a4 4 0 0 1-4 4h-5a4 4 0 0 1-4-4" fill="none" stroke="currentColor" strokeLinecap="round" strokeWidth="1.7" />
              </svg>
              <span className="profile-view-header-action-label">
                {dedupeLaunching
                  ? 'Starting…'
                  : thisProfileDedupeActive
                    ? 'View dedupe'
                    : mediaCleanupActive
                      ? 'Cleanup busy'
                      : 'Dedupe'}
              </span>
            </button>
          ) : null}
          {popularitySortAvailable && sourceId ? (
            <button
              className={`ghost-button profile-view-header-action profile-view-stats-refresh is-${statsRefreshState}`}
              disabled={statsRefreshState === 'queueing'}
              onClick={handleRefreshMediaStats}
              type="button"
              aria-label="Refresh media stats"
              title={
                statsRefreshState === 'queued'
                  ? 'Media stats refresh queued'
                  : 'Refresh media stats (views, likes, comments, shares) for downloaded media'
              }
            >
              {statsRefreshState === 'queued' ? (
                <svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true" focusable="false">
                  <path d="M5 12.5 10 17.5 19 7" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
                </svg>
              ) : (
                <svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true" focusable="false">
                  <path d="M20 11a8 8 0 1 0-2.34 5.66M20 4v7h-7" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
                </svg>
              )}
              <span className="profile-view-header-action-label">
                {statsRefreshState === 'queueing'
                  ? 'Queueing…'
                  : statsRefreshState === 'queued'
                    ? 'Queued'
                    : 'Refresh stats'}
              </span>
            </button>
          ) : null}
          {gallery?.profileUrl ? (
            <button
              className="ghost-button profile-view-header-action profile-view-open-online"
              onClick={handleOpenProfileOnline}
              type="button"
              aria-label="Open profile online"
              title="Open profile online"
            >
              <svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true" focusable="false">
                <path
                  d="M14 4h6v6M20 4l-9 9M12 6H7a3 3 0 0 0-3 3v8a3 3 0 0 0 3 3h8a3 3 0 0 0 3-3v-5"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.8"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              </svg>
              <span className="profile-view-header-action-label">Open profile online</span>
            </button>
          ) : null}
        </div>
      </header>

      {dedupeFeedback ? (
        <div
          className={`profile-view-dedupe-feedback is-${dedupeFeedback.tone}`}
          role={dedupeFeedback.tone === 'error' ? 'alert' : 'status'}
        >
          <span>{dedupeFeedback.message}</span>
          {dedupeFeedback.tone === 'success' ? (
            <button type="button" onClick={() => void openWorkspaceHealthWindow({ initialTab: 'storage' })}>
              View progress
            </button>
          ) : null}
          <button
            className="profile-view-dedupe-feedback-close"
            type="button"
            aria-label="Dismiss dedupe message"
            onClick={() => setDedupeFeedback(undefined)}
          >
            ×
          </button>
        </div>
      ) : null}

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
                onClick={() => setSelectedKeys(new Set(sortedPosts.map(postKey)))}
                type="button"
                disabled={sortedPosts.length === 0 || selectedPosts.length === sortedPosts.length}
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
              <div className="profile-view-toolbar-primary">
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
                        Flat grid
                      </button>
                    </>
                  ) : isAuthorTab ? (
                    // Likes/Favorites: agrupamento por autor disponível (padrão),
                    // preferência própria — não mexe no modo das outras abas.
                    <>
                      <button
                        className={likesMode === 'user' ? 'is-active' : ''}
                        onClick={() => setLikesMode('user')}
                        type="button"
                        aria-pressed={likesMode === 'user'}
                      >
                        By user
                      </button>
                      <button
                        className={likesMode === 'day' ? 'is-active' : ''}
                        onClick={() => setLikesMode('day')}
                        type="button"
                        aria-pressed={likesMode === 'day'}
                      >
                        By day
                      </button>
                      <button
                        className={likesMode === 'grid' ? 'is-active' : ''}
                        onClick={() => setLikesMode('grid')}
                        type="button"
                        aria-pressed={likesMode === 'grid'}
                      >
                        Flat grid
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
                        Flat grid
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
                      aria-label={`All ${sectionCounts.get(SECTION_FILTER_ALL) ?? 0}`}
                    >
                      <span className="profile-view-section-label">All</span>
                      <span className="profile-view-section-count" aria-hidden="true">
                        {sectionCounts.get(SECTION_FILTER_ALL) ?? 0}
                      </span>
                    </button>
                    {sections.map((section) => {
                      const label = gallery ? sectionLabel(gallery.provider, section) : section
                      const count = sectionCounts.get(section) ?? 0
                      return (
                        <button
                          key={section}
                          className={sectionFilter === section ? 'is-active' : ''}
                          onClick={() => setSectionFilter(section)}
                          type="button"
                          aria-pressed={sectionFilter === section}
                          aria-label={`${label} ${count}`}
                        >
                          <span className="profile-view-section-label">{label}</span>
                          <span className="profile-view-section-count" aria-hidden="true">
                            {count}
                          </span>
                        </button>
                      )
                    })}
                  </div>
                ) : null}
                {mediaTypeCounts.all > 0 ? (
                  <div
                    className="profile-view-mediatype"
                    role="group"
                    aria-label="Media type filter"
                  >
                    <button
                      className={mediaTypeFilter === 'all' ? 'is-active' : ''}
                      onClick={() => setMediaTypeFilter('all')}
                      type="button"
                      aria-pressed={mediaTypeFilter === 'all'}
                    >
                      All
                      <span className="profile-view-mediatype-count">{mediaTypeCounts.all}</span>
                    </button>
                    <button
                      className={mediaTypeFilter === 'photo' ? 'is-active' : ''}
                      onClick={() => setMediaTypeFilter('photo')}
                      type="button"
                      aria-pressed={mediaTypeFilter === 'photo'}
                      disabled={mediaTypeCounts.photo === 0}
                      title="Photos and slideshows"
                    >
                      <svg viewBox="0 0 24 24" width="13" height="13" aria-hidden="true" focusable="false">
                        <rect x="3" y="5" width="18" height="14" rx="2" fill="none" stroke="currentColor" strokeWidth="1.9" />
                        <circle cx="8.5" cy="10" r="1.6" fill="currentColor" />
                        <path d="M21 16l-5-5-9 8" fill="none" stroke="currentColor" strokeWidth="1.9" strokeLinejoin="round" />
                      </svg>
                      Photos
                      <span className="profile-view-mediatype-count">{mediaTypeCounts.photo}</span>
                    </button>
                    <button
                      className={mediaTypeFilter === 'video' ? 'is-active' : ''}
                      onClick={() => setMediaTypeFilter('video')}
                      type="button"
                      aria-pressed={mediaTypeFilter === 'video'}
                      disabled={mediaTypeCounts.video === 0}
                      title="Videos and reels"
                    >
                      <svg viewBox="0 0 24 24" width="13" height="13" aria-hidden="true" focusable="false">
                        <rect x="3" y="5" width="14" height="14" rx="2" fill="none" stroke="currentColor" strokeWidth="1.9" />
                        <path d="M17 9l4-2v10l-4-2" fill="none" stroke="currentColor" strokeWidth="1.9" strokeLinejoin="round" />
                      </svg>
                      Videos
                      <span className="profile-view-mediatype-count">{mediaTypeCounts.video}</span>
                    </button>
                  </div>
                ) : null}
              </div>
              <div className="profile-view-toolbar-actions">
              {isAuthorSection(sectionFilter) ? (
                <div className={`profile-view-search${likesSearchOpen ? ' is-open' : ''}`}>
                  {likesSearchOpen ? (
                    <>
                      <svg
                        className="profile-view-search-icon"
                        viewBox="0 0 24 24"
                        width="15"
                        height="15"
                        aria-hidden="true"
                        focusable="false"
                      >
                        <circle cx="11" cy="11" r="6" fill="none" stroke="currentColor" strokeWidth="1.8" />
                        <path d="M20 20l-4.2-4.2" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
                      </svg>
                      <input
                        ref={likesSearchInputRef}
                        className="profile-view-search-input"
                        type="search"
                        value={likesQuery}
                        placeholder="Search by author…"
                        onChange={(event) => setLikesQuery(event.target.value)}
                        onKeyDown={(event) => {
                          if (event.key === 'Escape') {
                            setLikesQuery('')
                            setLikesSearchOpen(false)
                          }
                        }}
                        aria-label="Search likes by author"
                      />
                      <button
                        className="ghost-button queue-icon-button profile-view-search-clear"
                        onClick={() => {
                          setLikesQuery('')
                          setLikesSearchOpen(false)
                        }}
                        type="button"
                        aria-label="Close author search"
                        title="Close search"
                      >
                        ×
                      </button>
                    </>
                  ) : (
                    <button
                      className="ghost-button queue-icon-button profile-view-search-toggle"
                      onClick={() => setLikesSearchOpen(true)}
                      type="button"
                      aria-label="Search likes by author"
                      title="Search by author"
                    >
                      <svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true" focusable="false">
                        <circle cx="11" cy="11" r="6" fill="none" stroke="currentColor" strokeWidth="1.8" />
                        <path d="M20 20l-4.2-4.2" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
                      </svg>
                    </button>
                  )}
                </div>
              ) : null}
              <div className="profile-view-filters" ref={filtersMenuRef}>
                <button
                  className={`ghost-button profile-view-filters-toggle${activeAdvancedFilters > 0 ? ' has-active' : ''}`}
                  onClick={() => setFiltersOpen((open) => !open)}
                  type="button"
                  aria-haspopup="dialog"
                  aria-expanded={filtersOpen}
                  aria-label="Advanced filters"
                  title="Advanced filters"
                >
                  <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true" focusable="false">
                    <path d="M4 6h16M7 12h10M10 18h4" fill="none" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round" />
                  </svg>
                  <span className="profile-view-filters-label">Filters</span>
                  {activeAdvancedFilters > 0 ? (
                    <span className="profile-view-filters-badge" aria-hidden="true">
                      {activeAdvancedFilters}
                    </span>
                  ) : null}
                </button>
                {filtersOpen ? (
                  <div className="profile-view-filters-menu" role="dialog" aria-label="Advanced filters">
                    <div className="profile-view-filters-group">
                      <span className="profile-view-filters-heading">Period</span>
                      <div className="profile-view-filters-chips">
                        {(['all', '7d', '30d', '90d', 'year'] as DateRangeFilter[]).map((range) => (
                          <button
                            key={range}
                            className={dateRange === range ? 'is-active' : ''}
                            onClick={() => setDateRange(range)}
                            type="button"
                            aria-pressed={dateRange === range}
                          >
                            {DATE_RANGE_LABEL[range]}
                          </button>
                        ))}
                      </div>
                    </div>
                    <div className="profile-view-filters-group">
                      <span className="profile-view-filters-heading">
                        Minimum engagement
                        <span className="profile-view-filters-heading-value">
                          {minEngagement > 0 ? `${compactCount(minEngagement)}+` : 'any'}
                        </span>
                      </span>
                      <input
                        className="profile-view-filters-range"
                        type="range"
                        min={0}
                        max={ENGAGEMENT_STEPS.length - 1}
                        step={1}
                        value={engagementToSlider(minEngagement)}
                        onChange={(event) => setMinEngagement(sliderToEngagement(Number(event.target.value)))}
                        aria-label="Minimum views or likes"
                      />
                    </div>
                    <div className="profile-view-filters-group">
                      <span className="profile-view-filters-heading">Format</span>
                      <label className="profile-view-filters-check">
                        <input
                          type="checkbox"
                          checked={carouselOnly}
                          onChange={(event) => setCarouselOnly(event.target.checked)}
                        />
                        Carousels / slideshows only
                      </label>
                    </div>
                    {presets.length > 0 ? (
                      <div className="profile-view-filters-group">
                        <span className="profile-view-filters-heading">Saved presets</span>
                        <div className="profile-view-filters-presets">
                          {presets.map((preset) => (
                            <span key={preset.id} className="profile-view-filters-preset">
                              <button
                                className="profile-view-filters-preset-apply"
                                onClick={() => applyPreset(preset)}
                                type="button"
                                title={`Apply "${preset.name}"`}
                              >
                                {preset.name}
                              </button>
                              <button
                                className="profile-view-filters-preset-remove"
                                onClick={() => deletePreset(preset.id)}
                                type="button"
                                aria-label={`Delete preset ${preset.name}`}
                                title="Delete preset"
                              >
                                ×
                              </button>
                            </span>
                          ))}
                        </div>
                      </div>
                    ) : null}
                    <div className="profile-view-filters-footer">
                      <button
                        className="ghost-button"
                        onClick={clearAdvancedFilters}
                        type="button"
                        disabled={activeAdvancedFilters === 0}
                      >
                        Clear
                      </button>
                      <button
                        className="ghost-button"
                        onClick={saveCurrentPreset}
                        type="button"
                        title="Save the current filters as a preset"
                      >
                        Save preset
                      </button>
                    </div>
                  </div>
                ) : null}
              </div>
              <div className="profile-view-sort" ref={sortMenuRef}>
                <button
                  className="ghost-button profile-view-sort-toggle"
                  onClick={() => setSortMenuOpen((open) => !open)}
                  type="button"
                  aria-haspopup="menu"
                  aria-expanded={sortMenuOpen}
                  aria-label="Sort order"
                  title={`Sort order: ${sortControlLabel}`}
                >
                  <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true" focusable="false">
                    <path
                      d="M8 4v16M4.5 16.5 8 20l3.5-3.5M16 20V4M12.5 7.5 16 4l3.5 3.5"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="1.8"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    />
                  </svg>
                  <span className="profile-view-sort-label">{sortControlLabel}</span>
                </button>
                {sortMenuOpen ? (
                  <div className="profile-view-sort-menu" role="menu">
                    <span className="profile-view-sort-group">Sort by</span>
                    <button
                      className="menu-item profile-view-sort-item"
                      role="menuitemradio"
                      aria-checked={effectiveSortField === 'creation'}
                      onClick={() => setSortField('creation')}
                      type="button"
                    >
                      <span className="profile-view-sort-check" aria-hidden="true">
                        {effectiveSortField === 'creation' ? '✓' : ''}
                      </span>
                      Creation date
                    </button>
                    <button
                      className="menu-item profile-view-sort-item"
                      role="menuitemradio"
                      aria-checked={effectiveSortField === 'download'}
                      onClick={() => setSortField('download')}
                      type="button"
                    >
                      <span className="profile-view-sort-check" aria-hidden="true">
                        {effectiveSortField === 'download' ? '✓' : ''}
                      </span>
                      Download date
                    </button>
                    {popularitySortAvailable ? (
                      <button
                        className="menu-item profile-view-sort-item"
                        role="menuitemradio"
                        aria-checked={effectiveSortField === 'popularity'}
                        onClick={() => setSortField('popularity')}
                        type="button"
                      >
                        <span className="profile-view-sort-check" aria-hidden="true">
                          {effectiveSortField === 'popularity' ? '✓' : ''}
                        </span>
                        Popularity
                      </button>
                    ) : null}
                    <div className="profile-view-sort-divider" role="separator" />
                    <button
                      className="menu-item profile-view-sort-item"
                      role="menuitemradio"
                      aria-checked={sortDir === 'newest'}
                      onClick={() => setSortDir('newest')}
                      type="button"
                    >
                      <span className="profile-view-sort-check" aria-hidden="true">
                        {sortDir === 'newest' ? '✓' : ''}
                      </span>
                      {effectiveSortField === 'popularity' ? 'Most viewed first' : 'Newest first'}
                    </button>
                    <button
                      className="menu-item profile-view-sort-item"
                      role="menuitemradio"
                      aria-checked={sortDir === 'oldest'}
                      onClick={() => setSortDir('oldest')}
                      type="button"
                    >
                      <span className="profile-view-sort-check" aria-hidden="true">
                        {sortDir === 'oldest' ? '✓' : ''}
                      </span>
                      {effectiveSortField === 'popularity' ? 'Least viewed first' : 'Oldest first'}
                    </button>
                  </div>
                ) : null}
              </div>
              <div className="profile-view-actions-cluster">
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
                  <span
                    className="profile-view-density-level"
                    title={`Thumbnail size ${densityLevelLabel}`}
                    aria-label={`Thumbnail size ${densityLevelLabel}`}
                  >
                    {densityLevelLabel}
                  </span>
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
                  title="Select media (S)"
                >
                  Select
                </button>
              </div>
              </div>
            </>
          )}
        </div>
      ) : null}

      {error ? <div className="profile-view-banner profile-view-banner-error" role="alert">{error}</div> : null}

      {loading && !gallery ? (
        <div className="profile-view-empty" role="status">
          <span className="profile-view-empty-title">Loading media…</span>
          <span className="muted-text">Fetching the local gallery for this profile.</span>
        </div>
      ) : gallery && gallery.posts.length === 0 ? (
        <div className="profile-view-empty" role="status">
          <span className="profile-view-empty-title">No downloaded media</span>
          <span className="muted-text">Sync this profile to download posts into the local gallery.</span>
        </div>
      ) : filteredEmpty ? (
        <div className="profile-view-empty" role="status">
          <span className="profile-view-empty-title">
            {activeSectionLabel
              ? `No ${activeSectionLabel} media`
              : likesQuery.trim()
                ? 'No matches'
                : 'No media in this view'}
          </span>
          <span className="muted-text">
            {likesQuery.trim()
              ? 'Try another author or clear the search.'
              : hasActiveContentFilters
                ? 'No media matches the active filters.'
                : activeSectionLabel
                  ? `Nothing downloaded in ${activeSectionLabel} yet — switch to All or pick another section.`
                  : 'Adjust filters to see more media.'}
          </span>
          {hasActiveContentFilters ? (
            <button className="ghost-button" onClick={clearAllFilters} type="button">
              Clear filters
            </button>
          ) : null}
        </div>
      ) : (
        <div className="profile-view-days" ref={scrollRef}>
          {stickyGroupHeader ? (
            <div className="profile-view-sticky-header" aria-hidden="true">
              <span className="profile-view-day-title">
                <span className={stickyGroupHeader.plain ? 'eyebrow profile-view-user-title' : 'eyebrow'}>
                  {stickyGroupHeader.label}
                </span>
                <span className="pill">{stickyGroupHeader.count}</span>
              </span>
            </div>
          ) : null}
          {!isVirtualized ? (
            albums.map((album) => (
              <section className="profile-view-day profile-view-album" key={album.key}>
                <div className="profile-view-day-header profile-view-album-header">
                  <span className="profile-view-album-cover" aria-hidden="true">
                    {album.coverSrc ? <img src={album.coverSrc} alt="" loading="lazy" /> : null}
                  </span>
                  <span className="profile-view-day-title">
                    <span className="eyebrow profile-view-album-title">{album.label}</span>
                    <span className="pill">{album.posts.length}</span>
                  </span>
                </div>
                <div className="profile-view-grid" style={gridStyle}>
                  {album.posts.map((post, index) => renderCard(post, post.postId ?? `${album.key}-${index}`))}
                </div>
              </section>
            ))
          ) : (
            // Grid e "By day" virtualizados: só as linhas visíveis (+ overscan)
            // são montadas; a altura total é reservada para a barra ser fiel.
            <div
              className="profile-view-virtual"
              style={{ height: `${rowVirtualizer.getTotalSize()}px` }}
            >
              {rowVirtualizer.getVirtualItems().map((virtualItem) => {
                const row = virtualRows[virtualItem.index]
                if (!row) return null
                return (
                  <div
                    key={row.key}
                    className="profile-view-virtual-row"
                    data-index={virtualItem.index}
                    ref={rowVirtualizer.measureElement}
                    style={{ transform: `translateY(${virtualItem.start}px)` }}
                  >
                    {row.type === 'header' ? (
                      <div className="profile-view-day-header">
                        <span className="profile-view-day-title">
                          <span className={row.plain ? 'eyebrow profile-view-user-title' : 'eyebrow'}>
                            {row.label}
                          </span>
                          <span className="pill">{row.count}</span>
                        </span>
                      </div>
                    ) : (
                      <div className="profile-view-grid profile-view-grid-virtual" style={virtualGridStyle}>
                        {row.posts.map((post) => renderCard(post, postKey(post)))}
                      </div>
                    )}
                  </div>
                )
              })}
            </div>
          )}
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
            <p className="profile-view-confirm-hint muted-text">
              Tip: in the lightbox, Shift+Delete skips this dialog.
            </p>
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
          title={activeItem.post.author ? `@${activeItem.post.author}` : gallery?.handle}
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
                onClick={() => handleOpenMediaFile(activeItem.file.absolutePath)}
                type="button"
              >
                Open file
              </button>
              <button
                className="ghost-button"
                onClick={() => handleRevealMedia(activeItem.file.absolutePath)}
                type="button"
              >
                Reveal in folder
              </button>
            </>
          }
        />
      ) : null}
      </div>
    </WindowShell>
  )
}
