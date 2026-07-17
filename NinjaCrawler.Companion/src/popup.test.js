import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

const popupHtml = readFileSync(new URL('../popup.html', import.meta.url), 'utf8')
const popupSource = readFileSync(new URL('./popup.js', import.meta.url), 'utf8')
const storyDetectionSource = readFileSync(new URL('./storyDetection.js', import.meta.url), 'utf8')
const storyNetworkHookSource = readFileSync(new URL('./storyNetworkHook.js', import.meta.url), 'utf8')
const manifest = JSON.parse(readFileSync(new URL('../manifest.json', import.meta.url), 'utf8'))

describe('Companion popup layout', () => {
  it('uses the NinjaCrawler visual identity in the popup and extension chrome', () => {
    expect(popupHtml).toContain('class="companion-brand"')
    expect(popupHtml).toContain('class="brand-mark"')
    expect(popupHtml).toContain('<h1>NinjaCrawler</h1>')
    expect(manifest.action.default_icon['16']).toBe('icons/16.png')
    expect(manifest.icons['128']).toBe('icons/128.png')
  })

  it('keeps active-tab controls above the open-profiles section', () => {
    expect(popupHtml.indexOf('id="profileForm"')).toBeGreaterThan(-1)
    expect(popupHtml.indexOf('id="profilesPanel"')).toBeGreaterThan(
      popupHtml.indexOf('id="profileForm"'),
    )
  })

  it('exposes one concise loading status for profile discovery', () => {
    expect(popupHtml).toContain('id="profilesLoading"')
    expect(popupHtml).toContain('role="status"')
    expect(popupHtml).toContain('aria-live="polite"')
  })

  it('keeps profile sync available alongside selected-story download', () => {
    const storyAction = popupHtml.indexOf('id="targetButton"')
    const syncAction = popupHtml.indexOf('id="syncButton"')
    const importAction = popupHtml.indexOf('id="importAccountButton"')

    expect(storyAction).toBeGreaterThan(-1)
    expect(syncAction).toBeGreaterThan(storyAction)
    expect(importAction).toBeGreaterThan(syncAction)
    expect(popupHtml).toContain('Sync profile')
  })

  it('keeps last sync visible separately from selected story context', () => {
    expect(popupHtml).toContain('id="existingMeta"')
    expect(popupHtml).toContain('id="targetMeta"')
    expect(popupSource).toContain('Last sync ${formatDate(existing.lastSyncedAt)}')
    expect(popupSource).toContain('Selected story ${target.storyId}')
  })

  it('offers persisted theme selection and native shortcut configuration', () => {
    expect(popupHtml).toContain('id="themeSelect"')
    expect(popupHtml).toContain('<option value="dark">Dark</option>')
    expect(popupHtml).toContain('id="configureShortcutsButton"')
    expect(popupSource).toContain("chrome.storage.sync.set({ theme })")
    expect(popupSource).toContain("chrome://extensions/shortcuts")
  })

  it('registers commands for every active supported provider context', () => {
    expect(manifest.commands['sync-profile']).toBeDefined()
    expect(manifest.commands['download-story']).toBeDefined()
  })

  it('bounds account capture so a stuck service worker cannot freeze the popup', () => {
    expect(popupSource).toContain('ACCOUNT_CAPTURE_TIMEOUT_MS')
    expect(popupSource).toMatch(/await withTimeout\(\s*chrome\.runtime\.sendMessage\(/)
    expect(popupSource).toContain('function withTimeout(promise, timeoutMs, message)')
  })

  it('warns about and blocks import of an incomplete browser session', () => {
    expect(popupHtml).toContain('id="accountImportWarnings"')
    expect(popupSource).toContain('preview.missingRequiredFields')
    expect(popupSource).toContain('Incomplete browser session')
    expect(popupSource).toContain('elements.confirmAccountImport.disabled = state.isBusy || incomplete')
    expect(popupSource).toContain('function formatMissingField(field)')
  })

  it('grants host access to the bare instagram.com origin used by the probe filter', () => {
    expect(manifest.host_permissions).toContain('https://instagram.com/*')
  })

  it('offers a guided update when NinjaCrawler reports a newer Companion', () => {
    expect(popupHtml).toContain('id="updatePanel"')
    expect(popupHtml).toContain('id="copyInstallPathButton"')
    expect(popupHtml).toContain('id="openExtensionsButton"')
    expect(popupHtml).toContain("NinjaCrawler's Connector Runtimes window")
    expect(popupSource).toContain("compatibility?.status === 'update_available'")
    expect(popupSource).toContain("compatibility?.status === 'incompatible'")
    expect(popupSource).not.toContain('stageCompanionUpdate')
    expect(popupSource).toContain('navigator.clipboard.writeText(installPath)')
    expect(popupSource).toContain("chrome.tabs.create({ url: 'chrome://extensions' })")
    expect(popupSource).toContain('const compatibilityTask = loadCompatibility()')
  })

  it('grants alarm access for managed live reload', () => {
    expect(manifest.permissions).toContain('alarms')
  })

  it('keeps managed auto reload disabled until the user opts in', () => {
    expect(popupHtml).toContain('id="autoReloadCompanionUpdates"')
    expect(popupHtml).toContain('Off by default')
    expect(popupSource).toContain('autoReloadCompanionUpdates = false')
    expect(popupSource).toContain('saveUpdatePreferences()')
  })

  it('registers Instagram story content scripts in MAIN and isolated worlds', () => {
    const scripts = manifest.content_scripts ?? []
    expect(scripts.some((entry) => entry.world === 'MAIN' && entry.js.includes('src/storyNetworkHook.js'))).toBe(true)
    expect(scripts.some((entry) => entry.js.includes('src/storyDetection.js'))).toBe(true)
  })

  it('identifies the rendered story instead of assuming the first network item is active', () => {
    expect(storyDetectionSource).toContain('storyIdForRenderedMedia(items, activeStoryMediaUrls())')
    expect(storyDetectionSource).not.toContain('mediaIds[0]')
    expect(storyNetworkHookSource).toContain('items: mergedItems.slice(-100)')
    expect(storyNetworkHookSource).toContain('const mediaUrls = mediaUrlsOf(item)')
  })
})
