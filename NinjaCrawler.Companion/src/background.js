import {
  detectProfileFromUrl,
  detectTargetFromUrl,
  downloadTarget,
  installInstagramStoryNetworkHook,
  loadContext,
  resolveLiveTabUrl,
  syncSource,
} from './core.js'
import { captureAccountFromTab } from './accountCapture.js'

/** @type {Map<number, { url: string, target: object|null, updatedAt: number }>} */
const storyTargetsByTab = new Map()
/** @type {Set<number>} */
const storyHookedTabs = new Set()

chrome.runtime.onInstalled.addListener(() => {
  void initializeBadgeFeedback()
})

chrome.runtime.onStartup.addListener(() => {
  void initializeBadgeFeedback()
})

chrome.tabs.onActivated.addListener(({ tabId }) => {
  chrome.tabs.get(tabId, (tab) => {
    if (chrome.runtime.lastError || !tab) {
      return
    }

    void ensureStoryNetworkHook(tab)
    void refreshBadge(tab).catch(() => undefined)
  })
})

chrome.tabs.onRemoved.addListener((tabId) => {
  storyTargetsByTab.delete(tabId)
  storyHookedTabs.delete(tabId)
})

chrome.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
  if (changeInfo.url) {
    // Full navigation resets MAIN-world hooks.
    storyHookedTabs.delete(tabId)
    const stillStory = /\/stories\//i.test(changeInfo.url)
    if (!stillStory) {
      storyTargetsByTab.delete(tabId)
    }
  }

  if (changeInfo.status === 'complete' || changeInfo.url) {
    void ensureStoryNetworkHook(tab)
    void refreshBadge(tab).catch(() => undefined)
  }
})

chrome.commands.onCommand.addListener((command, tab) => {
  void runActiveTabCommand(command, tab)
})

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message?.type === 'storyTargetChanged') {
    const tabId = sender.tab?.id
    if (tabId != null) {
      if (message.target?.storyId) {
        storyTargetsByTab.set(tabId, {
          url: message.target.url || message.url,
          target: message.target,
          updatedAt: Date.now(),
        })
      } else if (message.url) {
        // Keep bare stories URL for profile context; clear resolved target.
        const existing = storyTargetsByTab.get(tabId)
        if (existing?.target && detectTargetFromUrl(message.url)) {
          // Page now has a full story URL.
          storyTargetsByTab.set(tabId, {
            url: message.url,
            target: detectTargetFromUrl(message.url),
            updatedAt: Date.now(),
          })
        } else if (!message.target) {
          // No media id yet — do not wipe a fresher cached target for the same handle.
          const cached = storyTargetsByTab.get(tabId)
          const cachedHandle = cached?.target?.handle?.toLocaleLowerCase?.()
          const messageHandle = message.handle?.toLocaleLowerCase?.()
          if (!cached || (messageHandle && cachedHandle && messageHandle !== cachedHandle)) {
            storyTargetsByTab.delete(tabId)
          }
        }
      }

      chrome.tabs.get(tabId, (tab) => {
        if (!chrome.runtime.lastError && tab) {
          void refreshBadge(tab).catch(() => undefined)
        }
      })
    }
    sendResponse({ ok: true })
    return true
  }

  if (message?.type === 'getStoryTarget') {
    sendResponse(storyTargetsByTab.get(message.tabId) ?? null)
    return true
  }

  if (message?.type === 'refreshBadges') {
    chrome.tabs.query({ active: true })
      .then((tabs) => Promise.all(tabs.map((tab) => refreshBadge(tab))))
      .then(() => sendResponse({ ok: true }))
      .catch((error) => sendResponse({ ok: false, error: error?.message || 'Badge refresh failed.' }))
    return true
  }

  if (message?.type === 'reloadExtension') {
    try {
      chrome.runtime.reload()
      sendResponse({ ok: true })
    } catch (error) {
      sendResponse({ ok: false, error: error?.message || 'Reload failed.' })
    }
    return true
  }

  if (message?.type !== 'captureAccount') return undefined

  chrome.tabs.get(message.tabId)
    .then((tab) => captureAccountFromTab(tab, message.provider))
    .then((capture) => sendResponse({ ok: true, capture }))
    .catch((error) => sendResponse({ ok: false, error: error?.message || 'Account capture failed.' }))
  return true
})

// "Reload" em chrome://extensions reinicia o service worker sem necessariamente
// ativar ou atualizar a aba atual. Atualiza o badge assim que o worker sobe.
void initializeBadgeFeedback()

async function initializeBadgeFeedback() {
  await safeAction(() => chrome.action.setBadgeBackgroundColor({ color: '#2f855a' }))
  const tabs = await chrome.tabs.query({ active: true })
  await Promise.all(tabs.map(async (tab) => {
    await ensureStoryNetworkHook(tab)
    await refreshBadge(tab)
  }))
}

async function ensureStoryNetworkHook(tab) {
  if (!tab?.id || !isInstagramTab(tab.url)) return
  if (storyHookedTabs.has(tab.id)) return

  try {
    await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      world: 'MAIN',
      func: installInstagramStoryNetworkHook,
    })
    storyHookedTabs.add(tab.id)
  } catch {
    // Restricted pages or missing host access.
  }
}

function isInstagramTab(url) {
  if (!url) return false
  try {
    const host = new URL(url).hostname.replace(/^www\./, '').toLowerCase()
    return host === 'instagram.com' || host.endsWith('.instagram.com')
  } catch {
    return false
  }
}

async function refreshBadge(tab) {
  if (!tab?.id) return

  const cached = storyTargetsByTab.get(tab.id)
  const liveUrl = await resolveLiveTabUrl(tab, {
    preferredUrl: cached?.url,
    skipCacheLookup: true,
  })

  // Persist a resolved target discovered via page inspection.
  const inspectedTarget = detectTargetFromUrl(liveUrl)
  if (inspectedTarget) {
    storyTargetsByTab.set(tab.id, {
      url: liveUrl,
      target: inspectedTarget,
      updatedAt: Date.now(),
    })
  }

  const detected = detectProfileFromUrl(liveUrl)
  const target = inspectedTarget ?? cached?.target ?? detectTargetFromUrl(liveUrl)
  if (!detected) {
    await clearBadge(tab.id)
    return
  }

  try {
    const context = await loadContext(liveUrl)
    const compatibility = context.companionCompatibility
    if (compatibility?.status === 'incompatible') {
      await safeAction(() => chrome.action.setBadgeText({ tabId: tab.id, text: '!' }))
      await safeAction(() => chrome.action.setBadgeBackgroundColor({ tabId: tab.id, color: '#b42318' }))
      await safeAction(() => chrome.action.setTitle({
        tabId: tab.id,
        title: `NinjaCrawler Companion: update required (${compatibility.availableVersion})`,
      }))
      return
    }
    if (compatibility?.status === 'update_available') {
      await safeAction(() => chrome.action.setBadgeText({ tabId: tab.id, text: '↑' }))
      await safeAction(() => chrome.action.setBadgeBackgroundColor({ tabId: tab.id, color: '#b45309' }))
      await safeAction(() => chrome.action.setTitle({
        tabId: tab.id,
        title: `NinjaCrawler Companion: update ${compatibility.availableVersion} available`,
      }))
      return
    }
    if (context.existingSource) {
      await safeAction(() => chrome.action.setBadgeText({ tabId: tab.id, text: target ? '↓' : '✓' }))
      await safeAction(() => chrome.action.setBadgeBackgroundColor({ tabId: tab.id, color: '#25835a' }))
      await safeAction(() => chrome.action.setTitle({
        tabId: tab.id,
        title: target
          ? `NinjaCrawler Companion: download selected story from ${detected.handle}`
          : `NinjaCrawler Companion: ${detected.handle} is already added`,
      }))
      return
    }

    await safeAction(() => chrome.action.setBadgeText({ tabId: tab.id, text: '+' }))
    await safeAction(() => chrome.action.setBadgeBackgroundColor({ tabId: tab.id, color: '#2563eb' }))
    await safeAction(() => chrome.action.setTitle({
      tabId: tab.id,
      title: `NinjaCrawler Companion: add ${detected.handle}`,
    }))
  } catch {
    await safeAction(() => chrome.action.setBadgeText({ tabId: tab.id, text: '!' }))
    await safeAction(() => chrome.action.setBadgeBackgroundColor({ tabId: tab.id, color: '#b42318' }))
    await safeAction(() => chrome.action.setTitle({
      tabId: tab.id,
      title: 'NinjaCrawler Companion: desktop API unavailable',
    }))
  }
}

async function runActiveTabCommand(command, commandTab) {
  const [activeTab] = commandTab?.id
    ? [commandTab]
    : await chrome.tabs.query({ active: true, lastFocusedWindow: true })
  const tab = activeTab
  if (!tab?.id) return

  try {
    await ensureStoryNetworkHook(tab)
    const cached = storyTargetsByTab.get(tab.id)
    const liveUrl = await resolveLiveTabUrl(tab, {
      preferredUrl: cached?.url,
      skipCacheLookup: true,
    })
    const detected = detectProfileFromUrl(liveUrl)
    if (!detected) {
      throw new Error('Open a supported profile or story first.')
    }

    const context = await loadContext(liveUrl)
    const existing = context.existingSource
    if (!existing) {
      throw new Error(`${detected.handle} is not added to NinjaCrawler.`)
    }

    if (command === 'sync-profile') {
      await showCommandProgress(tab.id, '↻', `Syncing ${detected.handle}…`)
      await syncSource({ sourceId: existing.id })
      await showCommandSuccess(tab.id, `Sync queued for ${detected.handle}.`)
      return
    }

    if (command === 'download-story') {
      const target = context.detectedTarget
        ?? cached?.target
        ?? detectTargetFromUrl(liveUrl)
      if (!target) {
        throw new Error('The active tab is not a supported story URL.')
      }
      await showCommandProgress(tab.id, '↓', `Queueing story ${target.storyId}…`)
      await downloadTarget({ sourceId: existing.id, target })
      await showCommandSuccess(tab.id, `Story ${target.storyId} queued.`)
    }
  } catch (error) {
    await safeAction(() => chrome.action.setBadgeText({ tabId: tab.id, text: '!' }))
    await safeAction(() => chrome.action.setBadgeBackgroundColor({ tabId: tab.id, color: '#b42318' }))
    await safeAction(() => chrome.action.setTitle({
      tabId: tab.id,
      title: `NinjaCrawler Companion: ${error?.message || 'Command failed.'}`,
    }))
  }
}

async function showCommandProgress(tabId, text, title) {
  await safeAction(() => chrome.action.setBadgeText({ tabId, text }))
  await safeAction(() => chrome.action.setBadgeBackgroundColor({ tabId, color: '#2563eb' }))
  await safeAction(() => chrome.action.setTitle({ tabId, title: `NinjaCrawler Companion: ${title}` }))
}

async function showCommandSuccess(tabId, title) {
  await safeAction(() => chrome.action.setBadgeText({ tabId, text: '✓' }))
  await safeAction(() => chrome.action.setBadgeBackgroundColor({ tabId, color: '#25835a' }))
  await safeAction(() => chrome.action.setTitle({ tabId, title: `NinjaCrawler Companion: ${title}` }))
}

async function clearBadge(tabId) {
  await safeAction(() => chrome.action.setBadgeText({ tabId, text: '' }))
  await safeAction(() => chrome.action.setTitle({ tabId, title: 'NinjaCrawler Companion' }))
}

async function safeAction(action) {
  try {
    await action()
  } catch {
    // Tabs can disappear while Chrome is dispatching update events.
  }
}
