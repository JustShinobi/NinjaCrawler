import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { CSSProperties } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { convertFileSrc } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import {
  deleteSourceMedia,
  loadMediaThumbnails,
  loadSourceMediaGallery,
  loadWorkspaceSnapshot,
  openExternalTarget,
  openMediaFile,
  revealMediaInFolder,
  runSourceSync,
  subscribeToSourceSyncQueue,
} from '../../bridge/desktop'
import { DEFAULT_PROVIDER_CATALOG } from '../../domain/defaults'
import type { MediaGalleryPost, ProviderKey, SourceMediaGallery } from '../../domain/models'
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

const VIEW_MODE_STORAGE_KEY = 'profileView.mode'
const HIGHLIGHTS_MODE_STORAGE_KEY = 'profileView.highlightsMode'
const LIKES_MODE_STORAGE_KEY = 'profileView.likesMode'
const DENSITY_STORAGE_KEY = 'profileView.density'
const SORT_FIELD_STORAGE_KEY = 'profileView.sortField'
const SORT_DIR_STORAGE_KEY = 'profileView.sortDir'

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
  const [avatarPath, setAvatarPath] = useState<string>()
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

  // Conjunto exibido: filtro de seção → busca por autor (só nos Likes) →
  // ordenação pelo eixo escolhido. Posts sem valor no eixo (ex.: like sem
  // download registrado, ou vídeo sem views) vão sempre ao fim, independentemente
  // da direção.
  const sortedPosts = useMemo<MediaGalleryPost[]>(() => {
    let base = visiblePosts
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
  }, [visiblePosts, effectiveSortField, sortDir, sectionFilter, likesQuery])

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
      if (likesSearchOpen) {
        event.preventDefault()
        event.stopImmediatePropagation()
        setLikesQuery('')
        setLikesSearchOpen(false)
      }
    }
    document.addEventListener('keydown', handler, true)
    return () => document.removeEventListener('keydown', handler, true)
  }, [confirmPosts, deleting, selectMode, sortMenuOpen, likesSearchOpen, exitSelectMode])

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
          <h1>{gallery?.handle ?? '…'}</h1>
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
        </div>
        <div className="profile-view-header-actions">
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
              : activeSectionLabel
                ? `Nothing downloaded in ${activeSectionLabel} yet — switch to All or pick another section.`
                : 'Adjust filters to see more media.'}
          </span>
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
