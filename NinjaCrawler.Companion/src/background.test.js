import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

const backgroundSource = readFileSync(new URL('./background.js', import.meta.url), 'utf8')

describe('Companion badge lifecycle', () => {
  it('refreshes active-tab feedback when the service worker starts', () => {
    expect(backgroundSource).toContain('void initializeBadgeFeedback()')
    expect(backgroundSource).toContain('chrome.tabs.query({ active: true })')
  })

  it('refreshes feedback again on browser startup', () => {
    expect(backgroundSource).toContain('chrome.runtime.onStartup.addListener')
  })
})

describe('Companion keyboard commands', () => {
  it('handles profile sync and story download from the active tab', () => {
    expect(backgroundSource).toContain('chrome.commands.onCommand.addListener')
    expect(backgroundSource).toContain("command === 'sync-profile'")
    expect(backgroundSource).toContain("command === 'download-story'")
    expect(backgroundSource).toContain('syncSource({ sourceId: existing.id })')
    expect(backgroundSource).toContain('downloadTarget({ sourceId: existing.id, target })')
  })

  it('reports command failures through the extension badge', () => {
    expect(backgroundSource).toContain("text: '!'")
    expect(backgroundSource).toContain('Command failed.')
  })
})

describe('Companion update feedback', () => {
  it('prioritizes incompatible and available updates in the badge', () => {
    expect(backgroundSource).toContain("compatibility?.status === 'incompatible'")
    expect(backgroundSource).toContain("compatibility?.status === 'update_available'")
    expect(backgroundSource).toContain("text: '↑'")
    expect(backgroundSource).toContain('update required')
  })
})
