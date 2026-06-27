import { detectProfileFromUrl, detectTargetFromUrl, loadContext } from './core.js'

chrome.runtime.onInstalled.addListener(() => {
  void safeAction(() => chrome.action.setBadgeBackgroundColor({ color: '#2f855a' }))
})

chrome.tabs.onActivated.addListener(({ tabId }) => {
  chrome.tabs.get(tabId, (tab) => {
    if (chrome.runtime.lastError || !tab) {
      return
    }

    void refreshBadge(tab).catch(() => undefined)
  })
})

chrome.tabs.onUpdated.addListener((_tabId, changeInfo, tab) => {
  if (changeInfo.status === 'complete' || changeInfo.url) {
    void refreshBadge(tab).catch(() => undefined)
  }
})

async function refreshBadge(tab) {
  if (!tab?.id) return

  const detected = detectProfileFromUrl(tab.url)
  const target = detectTargetFromUrl(tab.url)
  if (!detected) {
    await clearBadge(tab.id)
    return
  }

  try {
    const context = await loadContext(tab.url)
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
