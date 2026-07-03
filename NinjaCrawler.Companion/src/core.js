export const API_BASE = 'http://127.0.0.1:47219/ninjacrawler-companion/v1'

const RESERVED_INSTAGRAM = new Set(['accounts', 'direct', 'explore', 'p', 'reel', 'reels', 'stories', 'tv'])
const RESERVED_TWITTER = new Set([
  'compose',
  'explore',
  'home',
  'i',
  'intent',
  'login',
  'messages',
  'notifications',
  'search',
  'settings',
  'share',
])

export const PROVIDER_LABELS = {
  instagram: 'Instagram',
  tiktok: 'TikTok',
  twitter: 'X / Twitter',
}

export function detectProviderFromUrl(rawUrl) {
  if (!rawUrl) return null
  try {
    const host = new URL(rawUrl).hostname.replace(/^www\./, '').toLowerCase()
    if (host === 'instagram.com' || host.endsWith('.instagram.com')) return 'instagram'
    if (host === 'x.com' || host.endsWith('.x.com') || host === 'twitter.com' || host.endsWith('.twitter.com')) return 'twitter'
    if (host === 'tiktok.com' || host.endsWith('.tiktok.com')) return 'tiktok'
  } catch {
    return null
  }
  return null
}

export function detectTargetFromUrl(rawUrl) {
  if (!rawUrl) return null

  let url
  try {
    url = new URL(rawUrl)
  } catch {
    return null
  }

  const host = url.hostname.replace(/^www\./, '').toLowerCase()
  const segments = url.pathname.split('/').filter(Boolean)

  if ((host === 'instagram.com' || host.endsWith('.instagram.com'))
    && segments[0] === 'stories'
    && segments[1]
    && segments[2]) {
    const handle = normalizeHandle(segments[1])
    const storyId = segments[2].trim()
    if (!handle || !/^\d+$/.test(storyId)) return null
    return {
      kind: 'instagramStory',
      provider: 'instagram',
      handle,
      displayName: handle.replace(/^@/, ''),
      storyId,
      url: url.href,
    }
  }

  // TikTok story: a `/@handle/video/<id>` (ou `/photo/<id>`) aberta a partir de um
  // story. Vira um "story" do perfil (baixa na pasta Stories/ do source rastreado).
  if (host === 'tiktok.com' || host.endsWith('.tiktok.com')) {
    const index = segments.findIndex((segment) => segment === 'video' || segment === 'photo')
    if (
      index >= 0
      && segments[index + 1]
      && /^\d+$/.test(segments[index + 1])
      && segments[0]?.startsWith('@')
    ) {
      const handle = normalizeHandle(segments[0])
      return {
        kind: 'tiktokStory',
        provider: 'tiktok',
        handle,
        displayName: handle.replace(/^@/, ''),
        storyId: segments[index + 1],
        url: url.href,
      }
    }
  }

  return null
}

/**
 * Detecta uma URL de vídeo avulso baixável (TikTok/Instagram/Twitter/YouTube)
 * para a captura "Single video". Diferente de `detectTargetFromUrl` (story do
 * perfil), aqui basta ser um link de vídeo — o backend baixa via yt-dlp.
 */
export function detectVideoFromUrl(rawUrl) {
  if (!rawUrl) return null
  let url
  try {
    url = new URL(rawUrl)
  } catch {
    return null
  }
  const host = url.hostname.replace(/^www\./, '').toLowerCase()
  const segments = url.pathname.split('/').filter(Boolean)

  if (host === 'tiktok.com' || host.endsWith('.tiktok.com')) {
    const index = segments.findIndex((segment) => segment === 'video' || segment === 'photo')
    if (index >= 0 && segments[index + 1] && /^\d+$/.test(segments[index + 1])) {
      const handle = segments[0]?.startsWith('@') ? normalizeHandle(segments[0]) : ''
      return { kind: 'video', provider: 'tiktok', handle, videoId: segments[index + 1], url: url.href }
    }
    return null
  }

  if (host === 'instagram.com' || host.endsWith('.instagram.com')) {
    if (['reel', 'reels', 'p', 'tv'].includes(segments[0]) && segments[1]) {
      return { kind: 'video', provider: 'instagram', handle: '', videoId: segments[1], url: url.href }
    }
    return null
  }

  if (host === 'x.com' || host.endsWith('.x.com') || host === 'twitter.com' || host.endsWith('.twitter.com')) {
    const statusIndex = segments.indexOf('status')
    if (statusIndex > 0 && segments[statusIndex + 1] && /^\d+$/.test(segments[statusIndex + 1])) {
      return {
        kind: 'video',
        provider: 'twitter',
        handle: normalizeHandle(segments[0]),
        videoId: segments[statusIndex + 1],
        url: url.href,
      }
    }
    return null
  }

  if (host === 'youtube.com' || host.endsWith('.youtube.com')) {
    if (segments[0] === 'watch' && url.searchParams.get('v')) {
      return { kind: 'video', provider: 'youtube', handle: '', videoId: url.searchParams.get('v'), url: url.href }
    }
    if (segments[0] === 'shorts' && segments[1]) {
      return { kind: 'video', provider: 'youtube', handle: '', videoId: segments[1], url: url.href }
    }
    return null
  }

  if (host === 'youtu.be' && segments[0]) {
    return { kind: 'video', provider: 'youtube', handle: '', videoId: segments[0], url: url.href }
  }

  return null
}

export function detectProfileFromUrl(rawUrl) {
  if (!rawUrl) return null

  const target = detectTargetFromUrl(rawUrl)
  if (target) {
    return {
      provider: target.provider,
      handle: target.handle,
      displayName: target.displayName,
    }
  }

  let url
  try {
    url = new URL(rawUrl)
  } catch {
    return null
  }

  const host = url.hostname.replace(/^www\./, '').toLowerCase()
  const segments = url.pathname.split('/').filter(Boolean)
  let provider
  let handle

  if (host === 'instagram.com' || host.endsWith('.instagram.com')) {
    const first = segments[0]
    if (!first || RESERVED_INSTAGRAM.has(first)) return null
    provider = 'instagram'
    handle = first
  } else if (host === 'x.com' || host === 'twitter.com' || host.endsWith('.twitter.com')) {
    const first = segments[0]
    if (!first || RESERVED_TWITTER.has(first)) return null
    provider = 'twitter'
    handle = first
  } else if (host === 'tiktok.com' || host.endsWith('.tiktok.com')) {
    const first = segments[0]
    if (!first?.startsWith('@')) return null
    provider = 'tiktok'
    handle = first
  } else {
    return null
  }

  const normalizedHandle = normalizeHandle(handle)
  return {
    provider,
    handle: normalizedHandle,
    displayName: normalizedHandle.replace(/^@/, ''),
  }
}

export function normalizeHandle(value) {
  const clean = String(value ?? '').trim().replace(/^\/+|\/+$/g, '')
  if (!clean) return ''
  return clean.startsWith('@') ? clean : `@${clean}`
}

export async function loadContext(tabUrl) {
  const response = await fetch(`${API_BASE}/context?url=${encodeURIComponent(tabUrl ?? '')}`, {
    method: 'GET',
    cache: 'no-store',
  })
  if (!response.ok) throw new Error(await readError(response))
  return response.json()
}

export async function addSource(payload) {
  const response = await fetch(`${API_BASE}/source`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  })
  if (!response.ok) throw new Error(await readError(response))
  return response.json()
}

export async function syncSource(payload) {
  const response = await fetch(`${API_BASE}/sync`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  })
  if (!response.ok) throw new Error(await readError(response))
  return response.json()
}

export async function downloadTarget(payload) {
  const response = await fetch(`${API_BASE}/target`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  })
  if (!response.ok) throw new Error(await readError(response))
  return response.json()
}

export async function downloadSingleVideo(url) {
  const response = await fetch(`${API_BASE}/single-video`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ url }),
  })
  if (!response.ok) throw new Error(await readError(response))
  return response.json()
}

export async function previewAccount(capture) {
  const response = await fetch(`${API_BASE}/account/preview`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(capture),
  })
  if (!response.ok) throw new Error(await readError(response))
  return response.json()
}

export async function importAccount(payload) {
  const response = await fetch(`${API_BASE}/account/import`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  })
  if (!response.ok) throw new Error(await readError(response))
  return response.json()
}

async function readError(response) {
  try {
    const payload = await response.json()
    return payload.error || response.statusText
  } catch {
    return response.statusText
  }
}
