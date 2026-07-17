/**
 * MAIN-world content script (document_start).
 * Installs fetch/XHR hooks before Instagram story API calls fire.
 * Mirrors installInstagramStoryNetworkHook() from core.js — keep in sync.
 */
(() => {
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

    let preferredHandle = null
    try {
      const match = String(globalThis.location?.pathname || '').match(/^\/stories\/([^/]+)/i)
      if (match?.[1]) preferredHandle = match[1]
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
})()
