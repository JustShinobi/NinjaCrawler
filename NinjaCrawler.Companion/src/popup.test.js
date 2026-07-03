import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

const popupHtml = readFileSync(new URL('../popup.html', import.meta.url), 'utf8')

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
})
