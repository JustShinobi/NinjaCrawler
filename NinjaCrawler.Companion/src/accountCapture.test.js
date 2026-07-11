import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import {
  cookieToPayload,
  providerUserIdFromCookies,
  resolveCapturedUsername,
  unwrapPageProbeResult,
} from './accountCapture.js'
import { detectProviderFromUrl } from './core.js'

const captureSource = readFileSync(new URL('./accountCapture.js', import.meta.url), 'utf8')

describe('Companion account capture helpers', () => {
  it('normalizes Chrome cookies without losing security flags', () => {
    expect(cookieToPayload({
      domain: '.instagram.com',
      name: 'sessionid',
      value: 'secret',
      path: '/',
      expirationDate: 1_800_000_000,
      secure: true,
      httpOnly: true,
    })).toEqual({
      domain: '.instagram.com',
      name: 'sessionid',
      value: 'secret',
      path: '/',
      expiresAt: new Date(1_800_000_000 * 1000).toISOString(),
      secure: true,
      httpOnly: true,
    })
  })

  it('extracts stable provider ids from provider-owned cookies', () => {
    expect(providerUserIdFromCookies('instagram', [{ name: 'ds_user_id', value: '123' }])).toBe('123')
    expect(providerUserIdFromCookies('twitter', [{ name: 'twid', value: 'u%3D456' }])).toBe('456')
    expect(providerUserIdFromCookies('tiktok', [{ name: 'uid_tt', value: '789' }])).toBe('789')
  })

  it('allows account import from provider pages that are not profile URLs', () => {
    expect(detectProviderFromUrl('https://www.instagram.com/')).toBe('instagram')
    expect(detectProviderFromUrl('https://x.com/home')).toBe('twitter')
    expect(detectProviderFromUrl('https://www.tiktok.com/foryou')).toBe('tiktok')
    expect(detectProviderFromUrl('https://example.com/')).toBeNull()
  })

  it('falls back to the stable cookie identity for an account unknown to NinjaCrawler', () => {
    expect(resolveCapturedUsername(undefined, '123456')).toBe('123456')
  })

  it('bounds every provider capture with a timeout, not just Instagram', () => {
    // Twitter/TikTok tomavam o caminho `runPageProbe` sem corte: uma aba não
    // responsiva deixava a captura pendente para sempre e travava o popup.
    expect(captureSource).toContain('PAGE_PROBE_TIMEOUT_MS')
    expect(captureSource).toMatch(/:\s*await withTimeout\(\s*runPageProbe\(tab\.id, provider\)/)
  })

  it('collects browser client-hints only for the provider that consumes them', () => {
    // Twitter/TikTok baixam com impersonation própria e nunca aplicam Sec-CH-UA*,
    // então o fingerprint só é coletado para o Instagram. O userAgent segue geral.
    expect(captureSource).toContain("if (provider === 'instagram' && navigator.userAgentData)")
    expect(captureSource).toContain('const browser = { userAgent: navigator.userAgent }')
  })

  it('derives the Twitter handle from the signed-in profile link, not the timeline', () => {
    // Uma varredura global de `a[href^="/"]` percorria a timeline inteira (caro)
    // e podia capturar o perfil de um terceiro como se fosse a conta logada.
    expect(captureSource).toContain('a[data-testid="AppTabBar_Profile_Link"]')
    expect(captureSource).not.toContain("document.querySelectorAll('a[href^=\"/\"]')")
  })

  it('preserves a partial probe when the provider identity endpoint fails', () => {
    expect(unwrapPageProbeResult([{
      result: {
        ok: false,
        error: 'Instagram identity request failed (401).',
        partial: {
          identity: {},
          browser: { userAgent: 'Test' },
        },
      },
    }])).toEqual({
      identity: {},
      browser: { userAgent: 'Test' },
      warning: 'Instagram identity request failed (401).',
    })
  })
})
