import {
  detectProfileFromUrl,
  detectTargetFromUrl,
  downloadTarget,
  installInstagramStoryNetworkHook,
  loadCompanionUpdateStatus,
  loadContext,
  resolveLiveTabUrl,
  syncSource,
} from './core.js'
import { captureAccountFromTab } from './accountCapture.js'

/** @type {Set<number>} */
const storyHookedTabs = new Set()
const COMPANION_UPDATE_ALARM = 'ninjacrawler-companion-update'
const COMPANION_UPDATE_PERIOD_MINUTES = 5

chrome.runtime.onInstalled.addListener(() => {
  void initializeBadgeFeedback()
  void initializeCompanionLiveReload()
})

chrome.runtime.onStartup.addListener(() => {
  void initializeBadgeFeedback()
  void initializeCompanionLiveReload()
})

chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === COMPANION_UPDATE_ALARM) {
    void reloadManagedCompanionWhenReady()
  }
})

chrome.storage.onChanged.addListener((changes, areaName) => {
  if (areaName === 'sync' && changes.autoReloadCompanionUpdates) {
    void reloadManagedCompanionWhenReady()
  }
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
  storyHookedTabs.delete(tabId)
})

chrome.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
  if (changeInfo.url) {
    // Full navigation resets MAIN-world hooks.
    storyHookedTabs.delete(tabId)
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
      chrome.tabs.get(tabId, (tab) => {
        if (!chrome.runtime.lastError && tab) {
          void refreshBadge(tab).catch(() => undefined)
        }
      })
    }
    sendResponse({ ok: true })
    return true
  }

  if (message?.type === 'refreshBadges') {
    chrome.tabs.query({ active: true })
      .then((tabs) => Promise.all(tabs.map((tab) => refreshBadge(tab))))
      .then(() => sendResponse({ ok: true }))
      .catch((error) => sendResponse({ ok: false, error: error?.message || 'Badge refresh failed.' }))
    return true
  }

  if (message?.type !== 'captureAccount') return undefined

  chrome.tabs.get(message.tabId)
    .then((tab) => captureAccountFromTab(tab, message.provider))
    .then((capture) => sendResponse({ ok: true, capture }))
    .catch((error) => sendResponse({ ok: false, error: error?.message || 'Account capture failed.' }))
  return true
})

// Reloading from chrome://extensions restarts the service worker without
// necessarily activating or updating the current tab. Refresh feedback now.
void initializeBadgeFeedback()
void initializeCompanionLiveReload()

async function initializeCompanionLiveReload() {
  await chrome.alarms.create(COMPANION_UPDATE_ALARM, {
    periodInMinutes: COMPANION_UPDATE_PERIOD_MINUTES,
  })
  await reloadManagedCompanionWhenReady()
}

async function reloadManagedCompanionWhenReady() {
  try {
    const installedVersion = chrome.runtime.getManifest().version
    const { pendingCompanionVersion = null } = await chrome.storage.local.get({
      pendingCompanionVersion: null,
    })

    if (pendingCompanionVersion === installedVersion) {
      await chrome.storage.local.remove(['pendingCompanionVersion', 'pendingCompanionReloadAt'])
    }

    const updateStatus = await loadCompanionUpdateStatus()
    const stagedVersion = updateStatus?.stagedVersion
    if (!updateStatus?.updateReady || !versionIsOlder(installedVersion, stagedVersion)) return

    await notifyManagedCompanionUpdate(stagedVersion)
    const { autoReloadCompanionUpdates = false } = await chrome.storage.sync.get({
      autoReloadCompanionUpdates: false,
    })
    if (!autoReloadCompanionUpdates) return

    if (pendingCompanionVersion === stagedVersion) {
      return
    }

    await chrome.storage.local.set({
      pendingCompanionVersion: stagedVersion,
      pendingCompanionReloadAt: Date.now(),
    })
    chrome.runtime.reload()
  } catch {
    // NinjaCrawler may be closed; the next alarm retries without disturbing browsing.
  }
}

async function notifyManagedCompanionUpdate(stagedVersion) {
  await safeAction(() => chrome.action.setBadgeText({ text: '↑' }))
  await safeAction(() => chrome.action.setBadgeBackgroundColor({ color: '#b45309' }))
  await safeAction(() => chrome.action.setTitle({
    title: `NinjaCrawler Companion: managed update ${stagedVersion} is ready`,
  }))
}

function versionIsOlder(installedVersion, stagedVersion) {
  const installed = parseVersion(installedVersion)
  const staged = parseVersion(stagedVersion)
  if (!installed || !staged) return false
  for (let index = 0; index < Math.max(installed.length, staged.length); index += 1) {
    const left = installed[index] ?? 0
    const right = staged[index] ?? 0
    if (left !== right) return left < right
  }
  return false
}

function parseVersion(value) {
  if (typeof value !== 'string' || !/^\d+(?:\.\d+)*$/.test(value)) return null
  return value.split('.').map(Number)
}

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

  const liveUrl = await resolveLiveTabUrl(tab)

  const inspectedTarget = detectTargetFromUrl(liveUrl)

  const detected = detectProfileFromUrl(liveUrl)
  const target = inspectedTarget ?? detectTargetFromUrl(liveUrl)
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
    const liveUrl = await resolveLiveTabUrl(tab)
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
      const target = context.detectedTarget ?? detectTargetFromUrl(liveUrl)
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
