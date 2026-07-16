import { describe, expect, it, vi } from 'vitest'
import {
  collectDetectedProfiles,
  detectProfileFromUrl,
  detectTargetFromUrl,
  detectVideoFromUrl,
  installInstagramStoryNetworkHook,
  inspectLiveStoryPage,
  loadContext,
  loadContexts,
  loadHealth,
  pickBestLiveUrl,
  resolveLiveTabUrl,
} from './core.js'

describe('collectDetectedProfiles', () => {
  it('collects supported profiles across tabs and removes duplicates', () => {
    expect(collectDetectedProfiles([
      { id: 1, url: 'https://www.instagram.com/someone/' },
      { id: 2, url: 'https://x.com/another' },
      { id: 3, url: 'https://www.instagram.com/SOMEONE/reels/' },
      { id: 4, url: 'https://example.com/not-supported' },
    ])).toEqual([
      {
        key: 'instagram:@someone',
        provider: 'instagram',
        handle: '@someone',
        displayName: 'someone',
        url: 'https://www.instagram.com/someone/',
        tabIds: [1, 3],
      },
      {
        key: 'twitter:@another',
        provider: 'twitter',
        handle: '@another',
        displayName: 'another',
        url: 'https://x.com/another',
        tabIds: [2],
      },
    ])
  })

  it('returns no entries when no profile tabs are open', () => {
    expect(collectDetectedProfiles([{ id: 1, url: 'chrome://extensions' }])).toEqual([])
  })
})

describe('detectTargetFromUrl', () => {
  it('detects a TikTok story from a /video/ link', () => {
    const target = detectTargetFromUrl('https://www.tiktok.com/@sgaby.tls/video/7657248568637394194')
    expect(target).toMatchObject({
      kind: 'tiktokStory',
      provider: 'tiktok',
      handle: '@sgaby.tls',
      storyId: '7657248568637394194',
    })
  })

  it('still detects Instagram stories', () => {
    const target = detectTargetFromUrl('https://www.instagram.com/stories/someone/1234567890/')
    expect(target).toMatchObject({ kind: 'instagramStory', provider: 'instagram', storyId: '1234567890' })
  })

  it('ignores a plain TikTok profile', () => {
    expect(detectTargetFromUrl('https://www.tiktok.com/@sgaby.tls')).toBeNull()
  })
})

describe('detectProfileFromUrl', () => {
  it('detects a profile from a bare Instagram stories path without media id', () => {
    expect(detectProfileFromUrl('https://www.instagram.com/stories/someone/')).toMatchObject({
      provider: 'instagram',
      handle: '@someone',
    })
  })
})

describe('pickBestLiveUrl', () => {
  it('reconstructs the first story URL from network media cache when path has no id', () => {
    expect(pickBestLiveUrl({
      candidates: ['https://www.instagram.com/stories/someone/'],
      handle: 'someone',
      mediaIds: ['1234567890', '999'],
    })).toBe('https://www.instagram.com/stories/someone/1234567890/')
  })

  it('prefers a full story candidate over media cache', () => {
    expect(pickBestLiveUrl({
      candidates: [
        'https://www.instagram.com/stories/someone/',
        'https://www.instagram.com/stories/someone/555/',
      ],
      handle: 'someone',
      mediaIds: ['1234567890'],
    })).toBe('https://www.instagram.com/stories/someone/555/')
  })
})

describe('resolveLiveTabUrl', () => {
  it('prefers the live Instagram story URL exposed by the page over the stale tab URL', async () => {
    const previousChrome = globalThis.chrome
    globalThis.chrome = {
      scripting: {
        executeScript: vi.fn().mockResolvedValue([{
          result: {
            candidates: [
              'https://www.instagram.com/stories/someone/',
              'https://www.instagram.com/stories/someone/1234567890/',
            ],
            handle: 'someone',
            mediaIds: [],
          },
        }]),
      },
    }

    try {
      await expect(resolveLiveTabUrl(
        { id: 7, url: 'https://www.instagram.com/someone/' },
        { skipCacheLookup: true },
      )).resolves.toBe('https://www.instagram.com/stories/someone/1234567890/')
    } finally {
      if (previousChrome) globalThis.chrome = previousChrome
      else delete globalThis.chrome
    }
  })

  it('uses network media cache when the first story URL has no media id', async () => {
    const previousChrome = globalThis.chrome
    globalThis.chrome = {
      scripting: {
        executeScript: vi.fn().mockResolvedValue([{
          result: {
            candidates: ['https://www.instagram.com/stories/someone/'],
            handle: 'someone',
            mediaIds: ['4242424242'],
          },
        }]),
      },
    }

    try {
      await expect(resolveLiveTabUrl(
        { id: 7, url: 'https://www.instagram.com/stories/someone/' },
        { skipCacheLookup: true },
      )).resolves.toBe('https://www.instagram.com/stories/someone/4242424242/')
    } finally {
      if (previousChrome) globalThis.chrome = previousChrome
      else delete globalThis.chrome
    }
  })

  it('prefers an explicit preferredUrl from the background story cache', async () => {
    await expect(resolveLiveTabUrl(
      { id: 7, url: 'https://www.instagram.com/stories/someone/' },
      { preferredUrl: 'https://www.instagram.com/stories/someone/111/', skipCacheLookup: true },
    )).resolves.toBe('https://www.instagram.com/stories/someone/111/')
  })

  it('falls back to the tab URL when page inspection is unavailable', async () => {
    const previousChrome = globalThis.chrome
    globalThis.chrome = {
      scripting: {
        executeScript: vi.fn().mockRejectedValue(new Error('Cannot access page')),
      },
    }

    try {
      await expect(resolveLiveTabUrl(
        { id: 7, url: 'https://x.com/someone' },
        { skipCacheLookup: true },
      )).resolves.toBe('https://x.com/someone')
    } finally {
      if (previousChrome) globalThis.chrome = previousChrome
      else delete globalThis.chrome
    }
  })
})

describe('story page probes', () => {
  it('exports self-contained injectables for scripting.executeScript', () => {
    expect(typeof inspectLiveStoryPage).toBe('function')
    expect(typeof installInstagramStoryNetworkHook).toBe('function')
  })
})

describe('detectVideoFromUrl', () => {
  it('detects TikTok /video/ and /photo/ links', () => {
    expect(detectVideoFromUrl('https://www.tiktok.com/@amandagobbi14/video/7647174028368612615')).toEqual({
      kind: 'video',
      provider: 'tiktok',
      handle: '@amandagobbi14',
      videoId: '7647174028368612615',
      url: 'https://www.tiktok.com/@amandagobbi14/video/7647174028368612615',
    })
    expect(detectVideoFromUrl('https://www.tiktok.com/@user/photo/123')?.provider).toBe('tiktok')
  })

  it('detects Instagram reel/post links', () => {
    expect(detectVideoFromUrl('https://www.instagram.com/reel/AbC123/')?.provider).toBe('instagram')
    expect(detectVideoFromUrl('https://www.instagram.com/p/AbC123/')?.provider).toBe('instagram')
  })

  it('detects Twitter/X status links', () => {
    const result = detectVideoFromUrl('https://x.com/someone/status/1780000000000000000')
    expect(result?.provider).toBe('twitter')
    expect(result?.videoId).toBe('1780000000000000000')
  })

  it('detects YouTube watch, shorts and youtu.be links', () => {
    expect(detectVideoFromUrl('https://www.youtube.com/watch?v=dQw4w9WgXcQ')?.videoId).toBe('dQw4w9WgXcQ')
    expect(detectVideoFromUrl('https://www.youtube.com/shorts/abc123')?.videoId).toBe('abc123')
    expect(detectVideoFromUrl('https://youtu.be/abc123')?.videoId).toBe('abc123')
  })

  it('ignores profile pages and unsupported URLs', () => {
    expect(detectVideoFromUrl('https://www.tiktok.com/@amandagobbi14')).toBeNull()
    expect(detectVideoFromUrl('https://example.com/video/1')).toBeNull()
    expect(detectVideoFromUrl('not a url')).toBeNull()
    expect(detectVideoFromUrl('')).toBeNull()
  })
})

describe('Companion version reporting', () => {
  it('reports the installed version without requiring a custom CORS header', async () => {
    const previousChrome = globalThis.chrome
    globalThis.chrome = { runtime: { getManifest: () => ({ version: '0.3.1' }) } }
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockResolvedValue({
      ok: true,
      json: async () => ({}),
    })

    try {
      await loadContext('https://www.instagram.com/example/')
      await loadContexts(['https://www.instagram.com/example/'])
      await loadHealth()

      expect(fetchMock.mock.calls[0][0]).toContain('companionVersion=0.3.1')
      expect(JSON.parse(fetchMock.mock.calls[1][1].body)).toMatchObject({
        companionVersion: '0.3.1',
      })
      expect(fetchMock.mock.calls[0][1].headers).toBeUndefined()
      expect(fetchMock.mock.calls[2][0]).toContain('/health?companionVersion=0.3.1')
    } finally {
      fetchMock.mockRestore()
      if (previousChrome) globalThis.chrome = previousChrome
      else delete globalThis.chrome
    }
  })
})
