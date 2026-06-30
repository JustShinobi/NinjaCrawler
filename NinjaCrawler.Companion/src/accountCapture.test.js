import { describe, expect, it } from 'vitest'
import {
  cookieToPayload,
  providerUserIdFromCookies,
  resolveCapturedUsername,
  unwrapPageProbeResult,
} from './accountCapture.js'
import { detectProviderFromUrl } from './core.js'

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
