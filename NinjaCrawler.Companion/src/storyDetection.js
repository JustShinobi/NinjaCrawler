/**
 * Isolated-world content script for Instagram story SPA navigation.
 * Network caching lives in the MAIN-world hook (injected by the background).
 * This script re-probes on history changes and notifies the service worker.
 */

const STORY_PATH = /^\/stories\/([^/]+)(?:\/(\d+))?\/?/i

function currentStorySnapshot() {
  const href = globalThis.location?.href || ''
  const path = globalThis.location?.pathname || ''
  const match = path.match(STORY_PATH)
  if (!match) {
    return { url: href, target: null }
  }

  const handle = match[1]
  const pathStoryId = match[2] && /^\d+$/.test(match[2]) ? match[2] : null

  let mediaIds = []
  let cachedHandle = null
  try {
    const raw = document.documentElement?.getAttribute('data-nc-story-media')
    if (raw) {
      const parsed = JSON.parse(raw)
      cachedHandle = parsed?.handle ? String(parsed.handle).replace(/^@/, '') : null
      if (Array.isArray(parsed?.mediaIds)) {
        mediaIds = parsed.mediaIds
          .map((value) => String(value).split('_')[0])
          .filter((value) => /^\d+$/.test(value))
      }
    }
  } catch {
    mediaIds = []
  }

  const cacheMatches = !cachedHandle
    || cachedHandle.toLowerCase() === handle.toLowerCase()

  const storyId = pathStoryId
    ?? (cacheMatches ? mediaIds[0] : null)

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
