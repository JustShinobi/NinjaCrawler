import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

const popupHtml = readFileSync(new URL('../popup.html', import.meta.url), 'utf8')
const popupSource = readFileSync(new URL('./popup.js', import.meta.url), 'utf8')
const manifest = JSON.parse(readFileSync(new URL('../manifest.json', import.meta.url), 'utf8'))

describe('Companion popup layout', () => {
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

  it('offers a guided update when NinjaCrawler reports a newer Companion', () => {
    expect(popupHtml).toContain('id="updatePanel"')
    expect(popupHtml).toContain('id="downloadUpdateButton"')
    expect(popupHtml).toContain('id="openExtensionsButton"')
    expect(popupSource).toContain("compatibility?.status === 'update_available'")
    expect(popupSource).toContain("compatibility?.status === 'incompatible'")
    expect(popupSource).toContain("chrome.tabs.create({ url: 'chrome://extensions' })")
    expect(popupSource).toContain('const compatibilityTask = loadCompatibility()')
  })
})
