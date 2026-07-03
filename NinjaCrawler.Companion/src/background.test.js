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
