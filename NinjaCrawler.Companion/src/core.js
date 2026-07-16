export const API_BASE = 'http://127.0.0.1:47219/ninjacrawler-companion/v1'

const RESERVED_INSTAGRAM = new Set([
  'accounts',
  'direct',
  'explore',
  // Instagram highlight reels use `/stories/highlights/{id}/` — not a user handle.
  'highlights',
  'p',
  'reel',
  'reels',
  'stories',
  'tv',
])
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

function isReservedInstagramSegment(value) {
  const segment = String(value ?? '').trim().replace(/^@/, '').toLowerCase()
  return !segment || RESERVED_INSTAGRAM.has(segment)
}

export function collectDetectedProfiles(tabs) {
  const profiles = new Map()

  for (const tab of tabs ?? []) {
    const detected = detectProfileFromUrl(tab?.url)
    if (!detected) continue

    const key = `${detected.provider}:${detected.handle.toLocaleLowerCase()}`
    const current = profiles.get(key)
    if (current) {
      current.tabIds.push(tab.id)
      continue
    }

    profiles.set(key, {
      key,
      ...detected,
      url: tab.url,
      tabIds: tab.id == null ? [] : [tab.id],
    })
  }

  return [...profiles.values()].sort((left, right) => {
    const providerOrder = PROVIDER_LABELS[left.provider].localeCompare(PROVIDER_LABELS[right.provider])
    return providerOrder || left.handle.localeCompare(right.handle)
  })
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

/**
 * Prefer a known good story URL (background cache), then inspect the live page.
 * Instagram often keeps the first story at `/stories/{handle}/` without a media id
 * in the path — the page probe / network cache fills that gap.
 */
export async function resolveLiveTabUrl(tab, options = {}) {
  const preferredUrl = options.preferredUrl
  if (preferredUrl && detectTargetFromUrl(preferredUrl)) {
    return preferredUrl
  }

  if (globalThis.chrome?.runtime?.sendMessage && tab?.id != null && !options.skipCacheLookup) {
    try {
      const cached = await chrome.runtime.sendMessage({ type: 'getStoryTarget', tabId: tab.id })
      if (cached?.url && detectTargetFromUrl(cached.url)) {
        return cached.url
      }
    } catch {
      // Service worker may be restarting; fall through to page inspection.
    }
  }

  if (!tab?.id || !globalThis.chrome?.scripting?.executeScript) {
    return tab?.url ?? ''
  }

  try {
    const [{ result } = {}] = await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      func: inspectLiveStoryPage,
    })
    return pickBestLiveUrl(result, tab.url)
  } catch {
    return tab.url ?? ''
  }
}

/**
 * Injected into the page (must stay self-contained — no closed-over imports).
 * Reads location/canonical/anchors and the optional network cache written by the
 * MAIN-world story hook (`data-nc-story-media`).
 */
export function inspectLiveStoryPage() {
  const candidates = []
  const push = (value) => {
    if (typeof value !== 'string' || !value) return
    if (!candidates.includes(value)) candidates.push(value)
  }

  push(globalThis.location?.href)
  push(document.querySelector('link[rel="canonical"]')?.href)
  push(document.querySelector('meta[property="og:url"]')?.content)

  let handle = null
  try {
    const match = String(globalThis.location?.pathname || '').match(
      /^\/stories\/([^/]+)(?:\/(\d+))?\/?/i,
    )
    // Ignore highlight trays — `/stories/highlights/{id}/` is not a user story.
    if (match?.[1] && match[1].toLowerCase() !== 'highlights') {
      handle = match[1]
      if (match[2]) {
        push(`${globalThis.location.origin}/stories/${match[1]}/${match[2]}/`)
      }
    }
  } catch {
    // Ignore malformed location access.
  }

  for (const anchor of document.querySelectorAll('a[href*="/stories/"]')) {
    try {
      const href = anchor.getAttribute('href')
      if (!href) continue
      // Profile pages always expose highlight circles as /stories/highlights/{id}/.
      // Collecting those made every Instagram profile resolve as @highlights.
      if (/\/stories\/highlights\//i.test(href)) continue
      push(new URL(href, globalThis.location.href).href)
    } catch {
      // Ignore invalid anchor hrefs.
    }
  }

  let mediaIds = []
  try {
    const raw = document.documentElement?.getAttribute('data-nc-story-media')
    if (raw) {
      const parsed = JSON.parse(raw)
      const cachedHandle = parsed?.handle ? String(parsed.handle).replace(/^@/, '') : ''
      if (cachedHandle && !handle) handle = cachedHandle
      if (Array.isArray(parsed?.mediaIds)) {
        mediaIds = parsed.mediaIds
          .map((value) => String(value).split('_')[0])
          .filter((value) => /^\d+$/.test(value))
      }
      if (handle && cachedHandle && handle.toLowerCase() !== cachedHandle.toLowerCase()) {
        // Cache belongs to a different tray; ignore media ids.
        mediaIds = []
      }
    }
  } catch {
    mediaIds = []
  }

  return { candidates, handle, mediaIds }
}

/**
 * MAIN-world hook: observes Instagram story/reels_media responses and stores the
 * ordered media ids for the open tray on `document.documentElement`.
 * Injected via chrome.scripting.executeScript({ world: 'MAIN' }).
 */
export function installInstagramStoryNetworkHook() {
  if (globalThis.__ncStoryHookInstalled) return
  globalThis.__ncStoryHookInstalled = true

  const writeCache = (payload) => {
    try {
      document.documentElement?.setAttribute('data-nc-story-media', JSON.stringify(payload))
      globalThis.dispatchEvent(new CustomEvent('nc-story-media', { detail: payload }))
    } catch {
      // Page may be tearing down.
    }
  }

  const mediaIdOf = (item) => {
    if (item == null) return null
    if (typeof item === 'string' || typeof item === 'number') {
      const value = String(item).split('_')[0]
      return /^\d+$/.test(value) ? value : null
    }
    const raw = item.pk ?? item.pk_id ?? item.id ?? item.media_id
    if (raw == null) return null
    const value = String(raw).split('_')[0]
    return /^\d+$/.test(value) ? value : null
  }

  const extract = (data) => {
    const byHandle = new Map()

    const addItem = (username, item) => {
      const handle = String(username || '').replace(/^@/, '').trim()
      const mediaId = mediaIdOf(item)
      if (!handle || !mediaId) return
      const list = byHandle.get(handle.toLowerCase()) ?? []
      if (!list.includes(mediaId)) list.push(mediaId)
      byHandle.set(handle.toLowerCase(), list)
      byHandle.set(`__name__:${handle.toLowerCase()}`, handle)
    }

    const visit = (node, usernameHint) => {
      if (!node || typeof node !== 'object') return
      if (Array.isArray(node)) {
        for (const entry of node) visit(entry, usernameHint)
        return
      }

      const username = node.user?.username
        ?? node.owner?.username
        ?? node.user?.user_name
        ?? usernameHint

      if (Array.isArray(node.items) && username) {
        for (const item of node.items) addItem(username, item)
      }
      if (Array.isArray(node.reel_media) && username) {
        for (const item of node.reel_media) addItem(username, item)
      }

      if (node.reels && typeof node.reels === 'object' && !Array.isArray(node.reels)) {
        for (const value of Object.values(node.reels)) visit(value)
      }
      if (Array.isArray(node.reels_media)) visit(node.reels_media)
      if (Array.isArray(node.tray)) visit(node.tray)
    }

    visit(data)

    // Prefer the handle matching the current stories path when present.
    let preferredHandle = null
    try {
      const match = String(globalThis.location?.pathname || '').match(/^\/stories\/([^/]+)/i)
      // Highlight trays are not user story trays.
      if (match?.[1] && match[1].toLowerCase() !== 'highlights') preferredHandle = match[1]
    } catch {
      preferredHandle = null
    }

    if (preferredHandle) {
      const key = preferredHandle.toLowerCase()
      const mediaIds = byHandle.get(key)
      if (mediaIds?.length) {
        const handle = byHandle.get(`__name__:${key}`) || preferredHandle
        return { handle, mediaIds }
      }
    }

    for (const [key, mediaIds] of byHandle.entries()) {
      if (key.startsWith('__name__:') || !mediaIds?.length) continue
      const handle = byHandle.get(`__name__:${key}`) || key
      return { handle, mediaIds }
    }
    return null
  }

  const maybeCapture = async (response, requestUrl) => {
    try {
      if (!response || typeof response.clone !== 'function') return
      const url = String(requestUrl || '')
      if (!/reels_media|story_tray|feed\/reels_media|\/stories\//i.test(url)) return
      const contentType = response.headers?.get?.('content-type') || ''
      if (contentType && !/json/i.test(contentType)) return
      const data = await response.clone().json()
      const payload = extract(data)
      if (payload?.mediaIds?.length) writeCache(payload)
    } catch {
      // Ignore non-JSON or partial responses.
    }
  }

  const originalFetch = globalThis.fetch
  if (typeof originalFetch === 'function') {
    globalThis.fetch = function ncStoryFetch(...args) {
      const requestUrl = args[0]?.url ?? args[0]
      return originalFetch.apply(this, args).then((response) => {
        void maybeCapture(response, requestUrl)
        return response
      })
    }
  }

  const originalOpen = XMLHttpRequest.prototype.open
  const originalSend = XMLHttpRequest.prototype.send
  XMLHttpRequest.prototype.open = function ncStoryXhrOpen(method, url, ...rest) {
    this.__ncStoryUrl = url
    return originalOpen.call(this, method, url, ...rest)
  }
  XMLHttpRequest.prototype.send = function ncStoryXhrSend(...args) {
    this.addEventListener('load', function onLoad() {
      try {
        const url = String(this.__ncStoryUrl || '')
        if (!/reels_media|story_tray|feed\/reels_media|\/stories\//i.test(url)) return
        if (!this.responseText) return
        const data = JSON.parse(this.responseText)
        const payload = extract(data)
        if (payload?.mediaIds?.length) writeCache(payload)
      } catch {
        // Ignore.
      }
    })
    return originalSend.apply(this, args)
  }
}

export function pickBestLiveUrl(inspection, fallbackUrl = '') {
  const candidates = inspection?.candidates ?? []
  const fromCandidates = candidates.find((candidate) => detectTargetFromUrl(candidate))
  if (fromCandidates) return fromCandidates

  const handle = String(inspection?.handle || '').replace(/^@/, '').trim()
  // Never reconstruct a story URL for reserved segments (e.g. highlights trays).
  if (!isReservedInstagramSegment(handle)) {
    const mediaId = (inspection?.mediaIds ?? [])
      .map((value) => String(value).split('_')[0])
      .find((value) => /^\d+$/.test(value))

    if (handle && mediaId) {
      return buildInstagramStoryUrl(handle, mediaId)
    }
  }

  // Prefer a non-highlight candidate (profile or real story path) over highlight
  // reel URLs that profile pages always expose in the DOM.
  const nonHighlight = candidates.find((candidate) => !isInstagramHighlightsUrl(candidate))
  if (nonHighlight) return nonHighlight
  if (fallbackUrl && !isInstagramHighlightsUrl(fallbackUrl)) return fallbackUrl

  return candidates[0] ?? fallbackUrl ?? ''
}

function isInstagramHighlightsUrl(rawUrl) {
  if (!rawUrl) return false
  try {
    const url = new URL(rawUrl)
    const host = url.hostname.replace(/^www\./, '').toLowerCase()
    if (!(host === 'instagram.com' || host.endsWith('.instagram.com'))) return false
    const segments = url.pathname.split('/').filter(Boolean)
    return segments[0] === 'stories' && segments[1]?.toLowerCase() === 'highlights'
  } catch {
    return false
  }
}

export function buildInstagramStoryUrl(handle, storyId) {
  const cleanHandle = String(handle || '').replace(/^@/, '').trim()
  const cleanId = String(storyId || '').trim()
  return `https://www.instagram.com/stories/${cleanHandle}/${cleanId}/`
}

export function detectInstagramStoryPath(rawUrl) {
  if (!rawUrl) return null
  let url
  try {
    url = new URL(rawUrl)
  } catch {
    return null
  }
  const host = url.hostname.replace(/^www\./, '').toLowerCase()
  if (!(host === 'instagram.com' || host.endsWith('.instagram.com'))) return null
  const segments = url.pathname.split('/').filter(Boolean)
  if (segments[0] !== 'stories' || !segments[1]) return null
  // Highlight reels: /stories/highlights/{id}/ — not a user profile/story tray.
  if (isReservedInstagramSegment(segments[1])) return null
  const handle = normalizeHandle(segments[1])
  if (!handle) return null
  const storyId = segments[2] && /^\d+$/.test(segments[2]) ? segments[2] : null
  return { handle, storyId, displayName: handle.replace(/^@/, '') }
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
    // Skip highlight trays (`/stories/highlights/{id}/`) — not a user story.
    if (isReservedInstagramSegment(segments[1])) return null
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

  // Bare `/stories/{handle}/` (no media id yet) is still a profile context.
  const storyPath = detectInstagramStoryPath(rawUrl)
  if (storyPath) {
    return {
      provider: 'instagram',
      handle: storyPath.handle,
      displayName: storyPath.displayName,
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
  const companionVersion = globalThis.chrome?.runtime?.getManifest?.()?.version ?? ''
  const response = await fetch(`${API_BASE}/context?url=${encodeURIComponent(tabUrl ?? '')}&companionVersion=${encodeURIComponent(companionVersion)}`, {
    method: 'GET',
    cache: 'no-store',
  })
  if (!response.ok) throw new Error(await readError(response))
  return response.json()
}

export async function loadHealth() {
  const companionVersion = globalThis.chrome?.runtime?.getManifest?.()?.version ?? ''
  const response = await fetch(`${API_BASE}/health?companionVersion=${encodeURIComponent(companionVersion)}`, {
    method: 'GET',
    cache: 'no-store',
  })
  if (!response.ok) throw new Error(await readError(response))
  return response.json()
}

/** Ask NinjaCrawler to download the available Companion ZIP into AppData. */
export async function stageCompanionUpdate() {
  const companionVersion = globalThis.chrome?.runtime?.getManifest?.()?.version ?? ''
  const response = await fetch(`${API_BASE}/update/stage`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ companionVersion }),
  })
  if (!response.ok) throw new Error(await readError(response))
  return response.json()
}

export async function loadCompanionUpdateStatus() {
  const companionVersion = globalThis.chrome?.runtime?.getManifest?.()?.version ?? ''
  const response = await fetch(
    `${API_BASE}/update/status?companionVersion=${encodeURIComponent(companionVersion)}`,
    { method: 'GET', cache: 'no-store' },
  )
  if (!response.ok) throw new Error(await readError(response))
  return response.json()
}

export async function loadContexts(tabUrls) {
  const companionVersion = globalThis.chrome?.runtime?.getManifest?.()?.version ?? ''
  const response = await fetch(`${API_BASE}/contexts`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ urls: tabUrls, companionVersion }),
  })
  if (!response.ok) throw new Error(await companionApiError(response))
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

export async function addSources(sources) {
  const response = await fetch(`${API_BASE}/sources`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ sources }),
  })
  if (!response.ok) throw new Error(await companionApiError(response))
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

async function companionApiError(response) {
  const message = await readError(response)
  if (message.toLocaleLowerCase().includes('unknown ninjacrawler companion api endpoint')) {
    return 'Update and restart NinjaCrawler to use this Companion version.'
  }
  return message
}
