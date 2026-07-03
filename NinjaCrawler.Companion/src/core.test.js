import { describe, expect, it } from 'vitest'
import { collectDetectedProfiles, detectTargetFromUrl, detectVideoFromUrl } from './core.js'

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
