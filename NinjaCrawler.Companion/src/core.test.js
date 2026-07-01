import { describe, expect, it } from 'vitest'
import { detectVideoFromUrl } from './core.js'

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
