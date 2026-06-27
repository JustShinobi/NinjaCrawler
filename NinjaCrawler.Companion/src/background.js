import { detectProfileFromUrl, loadContext } from './core.js'

chrome.runtime.onInstalled.addListener(() => {
  chrome.action.setBadgeBackgroundColor({ color: '#2f855a' })
})

chrome.tabs.onActivated.addListener(async ({ tabId }) => {
  const tab = await chrome.tabs.get(tabId).catch(() => null)
  await refreshBadge(tab)
})

chrome.tabs.onUpdated.addListener(async (_tabId, changeInfo, tab) => {
  if (changeInfo.status === 'complete' || changeInfo.url) {
    await refreshBadge(tab)
  }
})

async function refreshBadge(tab) {
  if (!tab?.id) return

  const detected = detectProfileFromUrl(tab.url)
  if (!detected) {
    await clearBadge(tab.id)
    return
  }

  try {
    const context = await loadContext(tab.url)
    if (context.existingSource) {
      await chrome.action.setBadgeText({ tabId: tab.id, text: '✓' })
      await chrome.action.setBadgeBackgroundColor({ tabId: tab.id, color: '#25835a' })
      await chrome.action.setTitle({
        tabId: tab.id,
        title: `NinjaCrawler Companion: ${detected.handle} is already added`,
      })
      return
    }

    await chrome.action.setBadgeText({ tabId: tab.id, text: '+' })
    await chrome.action.setBadgeBackgroundColor({ tabId: tab.id, color: '#2563eb' })
    await chrome.action.setTitle({
      tabId: tab.id,
      title: `NinjaCrawler Companion: add ${detected.handle}`,
    })
  } catch {
    await chrome.action.setBadgeText({ tabId: tab.id, text: '!' })
    await chrome.action.setBadgeBackgroundColor({ tabId: tab.id, color: '#b42318' })
    await chrome.action.setTitle({
      tabId: tab.id,
      title: 'NinjaCrawler Companion: desktop API unavailable',
    })
  }
}

async function clearBadge(tabId) {
  await chrome.action.setBadgeText({ tabId, text: '' })
  await chrome.action.setTitle({ tabId, title: 'NinjaCrawler Companion' })
}
