/**
 * Isolated-world content script for Instagram story SPA navigation.
 * Network caching lives in the MAIN-world hook (injected by the background).
 * This script re-probes on history changes and notifies the service worker.
 */

const STORY_PATH = /^\/stories\/([^/]+)(?:\/(\d+))?\/?/i

function activeStoryMediaUrls() {
  const viewportWidth = globalThis.innerWidth || document.documentElement?.clientWidth || 0
  const viewportHeight = globalThis.innerHeight || document.documentElement?.clientHeight || 0
  return [...document.querySelectorAll('video, img')]
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
    .sort((left, right) => right.score - left.score)[0]?.urls ?? []
}

function normalizeMediaUrl(value) {
  try {
    const url = new URL(value, globalThis.location?.href)
    if (!/^https?:$/.test(url.protocol)) return []
    return [`${url.origin}${url.pathname}`, url.pathname]
  } catch {
    return []
  }
}

function storyIdForRenderedMedia(items, mediaUrls) {
  const renderedKeys = new Set(mediaUrls.flatMap(normalizeMediaUrl))
  if (!renderedKeys.size) return null
  return items.find((item) =>
    Array.isArray(item?.mediaUrls)
    && item.mediaUrls.some((value) => normalizeMediaUrl(value).some((key) => renderedKeys.has(key))))?.storyId ?? null
}

function mediaUrlsOfStoryItem(item) {
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

function storyIdOfItem(item) {
  const raw = item?.pk ?? item?.pk_id ?? item?.id ?? item?.media_id
  const value = String(raw ?? '').split('_')[0]
  return /^\d+$/.test(value) ? value : null
}

function embeddedStoryContext(wantedHandle) {
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
            storyId: storyIdOfItem(item),
            mediaUrls: mediaUrlsOfStoryItem(item),
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

function currentStorySnapshot() {
  const href = globalThis.location?.href || ''
  const path = globalThis.location?.pathname || ''
  const match = path.match(STORY_PATH)
  if (!match) {
    return { url: href, target: null }
  }

  const handle = match[1]
  const pathStoryId = match[2] && /^\d+$/.test(match[2]) ? match[2] : null

  let items = []
  let cachedHandle = null
  try {
    const raw = document.documentElement?.getAttribute('data-nc-story-media')
    if (raw) {
      const parsed = JSON.parse(raw)
      cachedHandle = parsed?.handle ? String(parsed.handle).replace(/^@/, '') : null
      if (Array.isArray(parsed?.items)) {
        items = parsed.items
      }
    }
  } catch {
    items = []
  }

  const cacheMatches = !cachedHandle
    || cachedHandle.toLowerCase() === handle.toLowerCase()

  const embedded = embeddedStoryContext(handle)
  if (embedded) {
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
  }

  const storyId = pathStoryId
    ?? (cacheMatches ? storyIdForRenderedMedia(items, activeStoryMediaUrls()) : null)
    ?? embedded?.initialStoryId

  if (!storyId) {
    return {
      url: href,
      target: null,
      handle: `@${handle.replace(/^@/, '')}`,
    }
  }

  const cleanHandle = handle.replace(/^@/, '')
  const url = `https://www.instagram.com/stories/${cleanHandle}/${storyId}/`
  return {
    url,
    target: {
      kind: 'instagramStory',
      provider: 'instagram',
      handle: `@${cleanHandle}`,
      displayName: cleanHandle,
      storyId,
      url,
    },
  }
}

let lastPayloadKey = ''
let publishTimer = 0

function publishStoryTarget(force = false) {
  const snapshot = currentStorySnapshot()
  const key = snapshot.target
    ? `${snapshot.target.handle}:${snapshot.target.storyId}`
    : `bare:${snapshot.handle || snapshot.url}`
  if (!force && key === lastPayloadKey) return
  lastPayloadKey = key

  try {
    chrome.runtime.sendMessage({
      type: 'storyTargetChanged',
      url: snapshot.url,
      target: snapshot.target,
      handle: snapshot.handle ?? snapshot.target?.handle ?? null,
    }).catch(() => undefined)
  } catch {
    // Extension context invalidated during reload.
  }
}

function schedulePublish() {
  clearTimeout(publishTimer)
  publishTimer = setTimeout(() => publishStoryTarget(), 50)
}

function patchHistoryMethod(methodName) {
  const original = history[methodName]
  if (typeof original !== 'function') return
  history[methodName] = function ncStoryHistory(...args) {
    const result = original.apply(this, args)
    schedulePublish()
    return result
  }
}

patchHistoryMethod('pushState')
patchHistoryMethod('replaceState')
window.addEventListener('popstate', schedulePublish)
window.addEventListener('nc-story-media', () => schedulePublish())
document.addEventListener('play', schedulePublish, true)
document.addEventListener('loadedmetadata', schedulePublish, true)
document.addEventListener('load', schedulePublish, true)

const mediaObserver = new MutationObserver(schedulePublish)
mediaObserver.observe(document, {
  subtree: true,
  childList: true,
  attributes: true,
  attributeFilter: ['src', 'poster'],
})

// Attribute writes from MAIN world do not emit DOM events — poll lightly while on stories.
setInterval(() => {
  if (STORY_PATH.test(globalThis.location?.pathname || '')) {
    publishStoryTarget(false)
  }
}, 750)

// Initial probe after the page settles.
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', () => publishStoryTarget(true), { once: true })
} else {
  publishStoryTarget(true)
}
