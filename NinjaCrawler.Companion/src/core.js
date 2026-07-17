export const API_BASE = 'http://127.0.0.1:47219/ninjacrawler-companion/v1'
const STORY_PROBE_ATTEMPTS = 6
const STORY_PROBE_INTERVAL_MS = 125

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
 * Inspect the live page at request time. Instagram can leave the first story at
 * `/stories/{handle}/`, while its network response also contains queued stories.
 */
export async function resolveLiveTabUrl(tab) {
  if (!tab?.id || !globalThis.chrome?.scripting?.executeScript) {
    return tab?.url ?? ''
  }

  try {
    let liveUrl = tab.url ?? ''
    for (let attempt = 0; attempt < STORY_PROBE_ATTEMPTS; attempt += 1) {
      const [{ result } = {}] = await chrome.scripting.executeScript({
        target: { tabId: tab.id },
        func: inspectLiveStoryPage,
      })
      liveUrl = pickBestLiveUrl(result, tab.url)
      if (detectTargetFromUrl(liveUrl)) return liveUrl

      const storyPath = detectInstagramStoryPath(liveUrl)
      const shouldRetry = storyPath && !storyPath.storyId && attempt < STORY_PROBE_ATTEMPTS - 1
      if (!shouldRetry) return liveUrl
      await new Promise((resolve) => setTimeout(resolve, STORY_PROBE_INTERVAL_MS))
    }
    return liveUrl
  } catch {
    return tab.url ?? ''
  }
}

/**
 * Injected into the page (must stay self-contained — no closed-over imports).
 * Reads location metadata, the visibly rendered media, and the optional network
 * cache written by the MAIN-world story hook (`data-nc-story-media`).
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

  let mediaIds = []
  let items = []
  let initialStoryId = null
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
      if (Array.isArray(parsed?.items)) {
        items = parsed.items
      }
      if (handle && cachedHandle && handle.toLowerCase() !== cachedHandle.toLowerCase()) {
        // Cache belongs to a different tray; ignore media ids.
        mediaIds = []
        items = []
      }
    }
  } catch {
    mediaIds = []
  }


  const mediaUrlsOf = (item) => {
    const urls = new Set()
    const add = (value) => {
      if (typeof value === 'string' && /^https?:\/\//i.test(value)) urls.add(value)
    }
    for (const candidate of item?.image_versions2?.candidates ?? []) add(candidate?.url)
    for (const candidate of Object.values(item?.image_versions2?.additional_candidates ?? {})) add(candidate?.url)
    for (const version of item?.video_versions ?? []) add(version?.url)
    add(item?.display_url)
    add(item?.thumbnail_url)
    add(item?.video_url)
    add(item?.image_url)
    return [...urls]
  }

  const storyIdOf = (item) => {
    const raw = item?.pk ?? item?.pk_id ?? item?.id ?? item?.media_id
    const value = String(raw ?? '').split('_')[0]
    return /^\d+$/.test(value) ? value : null
  }

  const embeddedStoryContext = (wantedHandle) => {
    const cleanHandle = String(wantedHandle || '').replace(/^@/, '').toLowerCase()
    if (!cleanHandle) return null

    let matched = null
    let visited = 0
    const visit = (node) => {
      if (matched || !node || typeof node !== 'object' || visited > 20_000) return
      visited += 1
      if (Array.isArray(node)) {
        for (const entry of node) visit(entry)
        return
      }

      if (Array.isArray(node.reels_media)) {
        for (const reel of node.reels_media) {
          const username = String(reel?.user?.username || '').replace(/^@/, '').toLowerCase()
          if (username !== cleanHandle || !Array.isArray(reel?.items)) continue
          const embeddedItems = reel.items
            .map((item) => ({
              storyId: storyIdOf(item),
              mediaUrls: mediaUrlsOf(item),
              takenAt: Number(item?.taken_at) || 0,
            }))
            .filter((item) => item.storyId)
          if (!embeddedItems.length) continue

          const seenAt = Number(reel?.seen) || 0
          const firstUnseen = embeddedItems.find((item) => item.takenAt > seenAt)
          matched = {
            items: embeddedItems.map(({ storyId, mediaUrls }) => ({ storyId, mediaUrls })),
            initialStoryId: firstUnseen?.storyId ?? embeddedItems[0].storyId,
          }
          return
        }
      }

      for (const value of Object.values(node)) visit(value)
    }

    for (const script of document.querySelectorAll('script[type="application/json"]')) {
      const raw = script.textContent || ''
      const lower = raw.toLowerCase()
      if (!lower.includes('reels_media') || !lower.includes(cleanHandle)) continue
      try {
        visit(JSON.parse(raw))
      } catch {
        // Ignore unrelated or partial hydration scripts.
      }
      if (matched) return matched
    }
    return null
  }

  const embedded = embeddedStoryContext(handle)
  if (embedded) {
    initialStoryId = embedded.initialStoryId
    const mergedById = new Map(items.map((item) => [String(item?.storyId), item]))
    for (const item of embedded.items) {
      const previous = mergedById.get(item.storyId)
      if (previous) {
        previous.mediaUrls = [...new Set([...(previous.mediaUrls ?? []), ...item.mediaUrls])]
      } else {
        items.push(item)
        mergedById.set(item.storyId, item)
      }
    }
    mediaIds = items.map((item) => item.storyId).filter(Boolean)
  }

  const viewportWidth = globalThis.innerWidth || document.documentElement?.clientWidth || 0
  const viewportHeight = globalThis.innerHeight || document.documentElement?.clientHeight || 0
  const visibleMedia = [...document.querySelectorAll('video, img')]
    .map((element) => {
      const rect = element.getBoundingClientRect()
      const style = globalThis.getComputedStyle?.(element)
      const visibleWidth = Math.max(0, Math.min(rect.right, viewportWidth) - Math.max(rect.left, 0))
      const visibleHeight = Math.max(0, Math.min(rect.bottom, viewportHeight) - Math.max(rect.top, 0))
      const visibleArea = visibleWidth * visibleHeight
      if (
        visibleArea <= 0
        || style?.display === 'none'
        || style?.visibility === 'hidden'
        || Number(style?.opacity ?? 1) === 0
      ) return null

      const urls = [element.currentSrc, element.src, element.poster]
        .filter((value, index, all) => typeof value === 'string' && value && all.indexOf(value) === index)
      if (!urls.length) return null

      const centerX = rect.left + (rect.width / 2)
      const centerY = rect.top + (rect.height / 2)
      const distance = Math.hypot(centerX - (viewportWidth / 2), centerY - (viewportHeight / 2))
      const playingBonus = element.tagName === 'VIDEO' && !element.paused ? 1_000_000_000 : 0
      return { urls, score: playingBonus + visibleArea - distance }
    })
    .filter(Boolean)
    .sort((left, right) => right.score - left.score)

  const currentMediaUrls = visibleMedia[0]?.urls ?? []
  const normalize = (value) => {
    try {
      const url = new URL(value, globalThis.location?.href)
      if (!/^https?:$/.test(url.protocol)) return []
      return [`${url.origin}${url.pathname}`, url.pathname]
    } catch {
      return []
    }
  }
  const currentKeys = new Set(currentMediaUrls.flatMap(normalize))
  const currentStoryId = items.find((item) =>
    Array.isArray(item?.mediaUrls)
    && item.mediaUrls.some((value) => normalize(value).some((key) => currentKeys.has(key))))?.storyId
    ?? initialStoryId
    ?? null

  return { candidates, handle, mediaIds, currentStoryId, currentMediaUrls, initialStoryId }
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
      let existing = null
      const raw = document.documentElement?.getAttribute('data-nc-story-media')
      if (raw) existing = JSON.parse(raw)
      const sameHandle = existing?.handle
        && String(existing.handle).toLowerCase() === String(payload.handle).toLowerCase()
      const mergedItems = sameHandle && Array.isArray(existing?.items) ? [...existing.items] : []
      for (const item of payload.items ?? []) {
        const previous = mergedItems.find((entry) => entry.storyId === item.storyId)
        if (previous) {
          previous.mediaUrls = [...new Set([...(previous.mediaUrls ?? []), ...(item.mediaUrls ?? [])])]
        } else {
          mergedItems.push(item)
        }
      }
      const merged = {
        handle: payload.handle,
        items: mergedItems.slice(-100),
        mediaIds: mergedItems.slice(-100).map((item) => item.storyId),
      }
      document.documentElement?.setAttribute('data-nc-story-media', JSON.stringify(merged))
      globalThis.dispatchEvent(new CustomEvent('nc-story-media', { detail: merged }))
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

    const mediaUrlsOf = (item) => {
      const urls = new Set()
      const add = (value) => {
        if (typeof value === 'string' && /^https?:\/\//i.test(value)) urls.add(value)
      }
      for (const candidate of item?.image_versions2?.candidates ?? []) add(candidate?.url)
      for (const candidate of Object.values(item?.image_versions2?.additional_candidates ?? {})) add(candidate?.url)
      for (const version of item?.video_versions ?? []) add(version?.url)
      add(item?.display_url)
      add(item?.thumbnail_url)
      add(item?.video_url)
      add(item?.image_url)
      return [...urls]
    }

    const addItem = (username, item) => {
      const handle = String(username || '').replace(/^@/, '').trim()
      const mediaId = mediaIdOf(item)
      if (!handle || !mediaId) return
      const list = byHandle.get(handle.toLowerCase()) ?? []
      const existing = list.find((entry) => entry.storyId === mediaId)
      const mediaUrls = mediaUrlsOf(item)
      if (existing) {
        existing.mediaUrls = [...new Set([...existing.mediaUrls, ...mediaUrls])]
      } else {
        list.push({ storyId: mediaId, mediaUrls })
      }
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
      const items = byHandle.get(key)
      if (items?.length) {
        const handle = byHandle.get(`__name__:${key}`) || preferredHandle
        return { handle, items, mediaIds: items.map((item) => item.storyId) }
      }
    }

    for (const [key, items] of byHandle.entries()) {
      if (key.startsWith('__name__:') || !items?.length) continue
      const handle = byHandle.get(`__name__:${key}`) || key
      return { handle, items, mediaIds: items.map((item) => item.storyId) }
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
  const locationTarget = detectTargetFromUrl(candidates[0])
  if (locationTarget) return candidates[0]

  const handle = String(inspection?.handle || '').replace(/^@/, '').trim()
  // Never reconstruct a story URL for reserved segments (e.g. highlights trays).
  if (!isReservedInstagramSegment(handle)) {
    const mediaId = String(inspection?.currentStoryId ?? '').split('_')[0]

    if (handle && /^\d+$/.test(mediaId)) {
      return buildInstagramStoryUrl(handle, mediaId)
    }
  }

  const metadataTarget = candidates.slice(1).find((candidate) => detectTargetFromUrl(candidate))
  if (metadataTarget) return metadataTarget

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

  // A TikTok `/@handle/video/<id>` (or `/photo/<id>`) opened from a story is a
  // profile story target and downloads into the tracked source's Stories folder.
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
 * Detect a standalone downloadable video URL (TikTok, Instagram, Twitter, or
 * YouTube). Unlike a profile story from `detectTargetFromUrl`, any supported
 * video link is enough here because the backend downloads it through yt-dlp.
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
