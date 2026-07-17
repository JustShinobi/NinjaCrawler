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

  it('does not treat Instagram highlight reels as user stories', () => {
    expect(detectTargetFromUrl(
      'https://www.instagram.com/stories/highlights/1789554678901234567/',
    )).toBeNull()
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

  it('detects a normal Instagram profile URL', () => {
    expect(detectProfileFromUrl('https://www.instagram.com/someone/')).toMatchObject({
      provider: 'instagram',
      handle: '@someone',
    })
  })

  it('does not treat highlight trays as @highlights profiles', () => {
    expect(detectProfileFromUrl(
      'https://www.instagram.com/stories/highlights/1789554678901234567/',
    )).toBeNull()
    expect(detectProfileFromUrl('https://www.instagram.com/highlights/1789554678901234567/')).toBeNull()
  })
})

describe('pickBestLiveUrl', () => {
  it('reconstructs the first story URL from the media rendered in the viewer', () => {
    expect(pickBestLiveUrl({
      candidates: ['https://www.instagram.com/stories/someone/'],
      handle: 'someone',
      mediaIds: ['1234567890', '999'],
      currentStoryId: '999',
    })).toBe('https://www.instagram.com/stories/someone/999/')
  })

  it('does not guess the current story from the next item in the network queue', () => {
    expect(pickBestLiveUrl({
      candidates: ['https://www.instagram.com/stories/someone/'],
      handle: 'someone',
      mediaIds: ['1234567890', '999'],
    })).toBe('https://www.instagram.com/stories/someone/')
  })

  it('prefers a full current location over the rendered-media cache', () => {
    expect(pickBestLiveUrl({
      candidates: [
        'https://www.instagram.com/stories/someone/555/',
        'https://www.instagram.com/stories/someone/',
      ],
      handle: 'someone',
      mediaIds: ['1234567890'],
      currentStoryId: '1234567890',
    })).toBe('https://www.instagram.com/stories/someone/555/')
  })

  it('keeps the profile URL when only highlight reel candidates are present', () => {
    const profileUrl = 'https://www.instagram.com/someone/'
    expect(pickBestLiveUrl({
      candidates: [
        profileUrl,
        'https://www.instagram.com/stories/highlights/1789554678901234567/',
      ],
      handle: null,
      mediaIds: [],
    }, profileUrl)).toBe(profileUrl)
  })

  it('does not reconstruct a story URL for the reserved highlights handle', () => {
    const profileUrl = 'https://www.instagram.com/someone/'
    expect(pickBestLiveUrl({
      candidates: [profileUrl],
      handle: 'highlights',
      mediaIds: ['1789554678901234567'],
    }, profileUrl)).toBe(profileUrl)
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

  it('uses the rendered-media match when the first story URL has no media id', async () => {
    const previousChrome = globalThis.chrome
    globalThis.chrome = {
      scripting: {
        executeScript: vi.fn().mockResolvedValue([{
          result: {
            candidates: ['https://www.instagram.com/stories/someone/'],
            handle: 'someone',
            mediaIds: ['4242424242'],
            currentStoryId: '4242424242',
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

  it('briefly retries while the first story media is still rendering', async () => {
    const previousChrome = globalThis.chrome
    globalThis.chrome = {
      scripting: {
        executeScript: vi.fn()
          .mockResolvedValueOnce([{
            result: {
              candidates: ['https://www.instagram.com/stories/someone/'],
              handle: 'someone',
              mediaIds: ['222', '111'],
              currentStoryId: null,
            },
          }])
          .mockResolvedValueOnce([{
            result: {
              candidates: ['https://www.instagram.com/stories/someone/'],
              handle: 'someone',
              mediaIds: ['222', '111'],
              currentStoryId: '111',
            },
          }]),
      },
    }

    try {
      await expect(resolveLiveTabUrl({
        id: 7,
        url: 'https://www.instagram.com/stories/someone/',
      })).resolves.toBe('https://www.instagram.com/stories/someone/111/')
      expect(globalThis.chrome.scripting.executeScript).toHaveBeenCalledTimes(2)
    } finally {
      if (previousChrome) globalThis.chrome = previousChrome
      else delete globalThis.chrome
    }
  })

  it('does not let a cached queue item override the story rendered in the tab', async () => {
    const previousChrome = globalThis.chrome
    globalThis.chrome = {
      scripting: {
        executeScript: vi.fn().mockResolvedValue([{
          result: {
            candidates: ['https://www.instagram.com/stories/someone/'],
            handle: 'someone',
            mediaIds: ['111', '222'],
            currentStoryId: '222',
          },
        }]),
      },
    }

    try {
      await expect(resolveLiveTabUrl(
        { id: 7, url: 'https://www.instagram.com/stories/someone/' },
        { preferredUrl: 'https://www.instagram.com/stories/someone/111/' },
      )).resolves.toBe('https://www.instagram.com/stories/someone/222/')
    } finally {
      if (previousChrome) globalThis.chrome = previousChrome
      else delete globalThis.chrome
    }
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

  it('does not rewrite Instagram profiles to highlight reel URLs from the page', async () => {
    const previousChrome = globalThis.chrome
    const profileUrl = 'https://www.instagram.com/someone/'
    globalThis.chrome = {
      scripting: {
        executeScript: vi.fn().mockResolvedValue([{
          result: {
            candidates: [
              profileUrl,
              'https://www.instagram.com/stories/highlights/1789554678901234567/',
            ],
            handle: null,
            mediaIds: [],
          },
        }]),
      },
    }

    try {
      await expect(resolveLiveTabUrl(
        { id: 7, url: profileUrl },
        { skipCacheLookup: true },
      )).resolves.toBe(profileUrl)
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

  it('matches the visible story media instead of taking the next queue item', () => {
    const previous = {
      document: globalThis.document,
      getComputedStyle: globalThis.getComputedStyle,
      innerHeight: globalThis.innerHeight,
      innerWidth: globalThis.innerWidth,
      location: globalThis.location,
    }
    const currentMediaUrl = 'https://scontent.example.net/current-story.jpg?signature=current'
    const mediaElement = {
      currentSrc: currentMediaUrl,
      src: currentMediaUrl,
      poster: '',
      tagName: 'IMG',
      getBoundingClientRect: () => ({
        top: 50,
        left: 300,
        right: 700,
        bottom: 750,
        width: 400,
        height: 700,
      }),
    }

    globalThis.location = {
      href: 'https://www.instagram.com/stories/someone/',
      pathname: '/stories/someone/',
      origin: 'https://www.instagram.com',
    }
    globalThis.innerWidth = 1_000
    globalThis.innerHeight = 800
    globalThis.getComputedStyle = () => ({ display: 'block', visibility: 'visible', opacity: '1' })
    globalThis.document = {
      documentElement: {
        clientWidth: 1_000,
        clientHeight: 800,
        getAttribute: () => JSON.stringify({
          handle: 'someone',
          mediaIds: ['222', '111'],
          items: [
            { storyId: '222', mediaUrls: ['https://scontent.example.net/next-story.jpg'] },
            { storyId: '111', mediaUrls: ['https://scontent.example.net/current-story.jpg?other=token'] },
          ],
        }),
      },
      querySelector: () => null,
      querySelectorAll: () => [mediaElement],
    }

    try {
      expect(inspectLiveStoryPage()).toMatchObject({
        handle: 'someone',
        mediaIds: ['222', '111'],
        currentStoryId: '111',
      })
    } finally {
      Object.assign(globalThis, previous)
    }
  })

  it('reads a single bare-URL story from Instagram hydration when the player uses a blob URL', () => {
    const previous = {
      document: globalThis.document,
      getComputedStyle: globalThis.getComputedStyle,
      innerHeight: globalThis.innerHeight,
      innerWidth: globalThis.innerWidth,
      location: globalThis.location,
    }
    const mediaElement = {
      currentSrc: 'blob:https://www.instagram.com/story-player',
      src: 'blob:https://www.instagram.com/story-player',
      poster: '',
      paused: true,
      tagName: 'VIDEO',
      getBoundingClientRect: () => ({
        top: 20,
        left: 250,
        right: 750,
        bottom: 780,
        width: 500,
        height: 760,
      }),
    }
    const hydration = JSON.stringify({
      result: {
        data: {
          xdt_api__v1__feed__reels_media: {
            reels_media: [{
              user: { username: 'nataliebournias' },
              seen: 1_784_233_561,
              items: [{
                id: '3942759357498900592_33474071478',
                pk: '3942759357498900592',
                taken_at: 1_784_233_561,
                video_versions: [{ url: 'https://scontent.example.net/story.mp4' }],
              }],
            }],
          },
        },
      },
    })

    globalThis.location = {
      href: 'https://www.instagram.com/stories/nataliebournias/',
      pathname: '/stories/nataliebournias/',
      origin: 'https://www.instagram.com',
    }
    globalThis.innerWidth = 1_000
    globalThis.innerHeight = 800
    globalThis.getComputedStyle = () => ({ display: 'block', visibility: 'visible', opacity: '1' })
    globalThis.document = {
      documentElement: {
        clientWidth: 1_000,
        clientHeight: 800,
        getAttribute: () => null,
      },
      querySelector: () => null,
      querySelectorAll: (selector) => selector.startsWith('script')
        ? [{ textContent: hydration }]
        : [mediaElement],
    }

    try {
      expect(inspectLiveStoryPage()).toMatchObject({
        handle: 'nataliebournias',
        mediaIds: ['3942759357498900592'],
        currentStoryId: '3942759357498900592',
        initialStoryId: '3942759357498900592',
      })
    } finally {
      Object.assign(globalThis, previous)
    }
  })

  it('selects the first unseen hydrated story instead of the next arbitrary payload item', () => {
    const previous = {
      document: globalThis.document,
      getComputedStyle: globalThis.getComputedStyle,
      innerHeight: globalThis.innerHeight,
      innerWidth: globalThis.innerWidth,
      location: globalThis.location,
    }
    const hydration = JSON.stringify({
      reels_media: [{
        user: { username: 'someone' },
        seen: 200,
        items: [
          { pk: '111', taken_at: 100 },
          { pk: '222', taken_at: 300 },
          { pk: '333', taken_at: 400 },
        ],
      }],
    })

    globalThis.location = {
      href: 'https://www.instagram.com/stories/someone/',
      pathname: '/stories/someone/',
      origin: 'https://www.instagram.com',
    }
    globalThis.innerWidth = 1_000
    globalThis.innerHeight = 800
    globalThis.getComputedStyle = () => ({ display: 'block', visibility: 'visible', opacity: '1' })
    globalThis.document = {
      documentElement: {
        clientWidth: 1_000,
        clientHeight: 800,
        getAttribute: () => null,
      },
      querySelector: () => null,
      querySelectorAll: (selector) => selector.startsWith('script')
        ? [{ textContent: hydration }]
        : [],
    }

    try {
      expect(inspectLiveStoryPage()).toMatchObject({
        mediaIds: ['111', '222', '333'],
        currentStoryId: '222',
        initialStoryId: '222',
      })
    } finally {
      Object.assign(globalThis, previous)
    }
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
