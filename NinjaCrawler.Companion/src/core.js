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
  reddit: 'Reddit',
  twitter: 'X / Twitter',
}

export function detectProviderFromUrl(rawUrl) {
  if (!rawUrl) return null
  try {
    const host = new URL(rawUrl).hostname.replace(/^www\./, '').toLowerCase()
    if (host === 'instagram.com' || host.endsWith('.instagram.com')) return 'instagram'
    if (host === 'x.com' || host.endsWith('.x.com') || host === 'twitter.com' || host.endsWith('.twitter.com')) return 'twitter'
    if (host === 'tiktok.com' || host.endsWith('.tiktok.com')) return 'tiktok'
    if (host === 'reddit.com' || host.endsWith('.reddit.com')) return 'reddit'
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
  } else if (host === 'reddit.com' || host.endsWith('.reddit.com')) {
    if (segments.length < 2 || !['user', 'u'].includes(segments[0])) return null
    provider = 'reddit'
    handle = segments[1]
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
