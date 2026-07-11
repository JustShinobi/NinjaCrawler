import { describe, expect, it } from 'vitest'
import {
  createSourceSyncOptions,
  createTikTokSourceSyncOptions,
  createTwitterSourceSyncOptions,
  DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS,
  resolveTwitterSourceSyncOptions,
} from './sourceSyncOptions'

describe('tiktok source sync options', () => {
  it('preserves explicit false values for every section', () => {
    const options = createTikTokSourceSyncOptions({
      getTimeline: false,
      getStoriesUser: false,
      getReposts: false,
      getLikedVideos: false,
      likedVideosLimit: 0,
      likedVideosIncremental: false,
      likedVideosKnownPageThreshold: 5,
    })

    expect(options.getTimeline).toBe(false)
    expect(options.getStoriesUser).toBe(false)
    expect(options.getReposts).toBe(false)
    expect(options.getLikedVideos).toBe(false)
    expect(options.likedVideosLimit).toBe(0)
    expect(options.likedVideosIncremental).toBe(false)
    expect(options.likedVideosKnownPageThreshold).toBe(5)
  })

  it('enables safe incremental liked-video scans by default', () => {
    const options = createTikTokSourceSyncOptions()

    expect(options.likedVideosIncremental).toBe(true)
    expect(options.likedVideosKnownPageThreshold).toBe(3)
  })

  it('collects stats for new media without refreshing existing media by default', () => {
    const options = createTikTokSourceSyncOptions()

    expect(options.collectMediaStats).toBe(true)
    expect(options.refreshExistingMediaStats).toBe(false)
  })
})

describe('twitter source sync options', () => {
  it('uses the media feed for normal sync and keeps full timeline as backfill', () => {
    const options = createTwitterSourceSyncOptions()
    expect(options.mediaModel).toBe(true)
    expect(options.profileModel).toBe(false)
    // Search/Likes são opcionais e vêm desligados; endpoints graphql prontos.
    expect(options.searchModel).toBe(false)
    expect(options.likesModel).toBe(false)
    expect(options.searchUseGraphqlEndpoint).toBe(true)
    expect(options.profileUseGraphqlEndpoint).toBe(true)
    expect(options.allowNonUserTweets).toBe(false)
    expect(options.downloadGifs).toBe(true)
    expect(options.gifsPrefix).toBe('GIF_')
    expect(options.useMd5Comparison).toBe(false)
    expect(options.abortOnLimit).toBe(true)
    expect(options.downloadAlreadyParsed).toBe(true)
    // TimerDisabled / TimerFirstUseTheSame
    expect(options.sleepTimerSecs).toBe(-1)
    expect(options.sleepTimerBeforeFirstSecs).toBe(-2)
    expect(options).toEqual(DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS)
  })

  it('keeps provided overrides and fills the rest with defaults', () => {
    const options = createTwitterSourceSyncOptions({ mediaModel: false, sleepTimerSecs: 20, specialPath: '  F:/x  ' })
    expect(options.mediaModel).toBe(false)
    expect(options.sleepTimerSecs).toBe(20)
    // specialPath is trimmed
    expect(options.specialPath).toBe('F:/x')
    // untouched fields fall back to defaults
    expect(options.profileModel).toBe(false)
    expect(options.downloadVideos).toBe(true)
  })

  it('createSourceSyncOptions wires the twitter section for twitter sources', () => {
    const wrapped = createSourceSyncOptions('twitter')
    expect(wrapped.twitter).toBeDefined()
    expect(wrapped.instagram).toBeUndefined()
    expect(wrapped.twitter?.profileModel).toBe(false)
  })

  it('resolveTwitterSourceSyncOptions only resolves for twitter provider', () => {
    expect(resolveTwitterSourceSyncOptions('instagram', { twitter: { mediaModel: false } })).toBeUndefined()
    const resolved = resolveTwitterSourceSyncOptions('twitter', { twitter: { mediaModel: false } })
    expect(resolved?.mediaModel).toBe(false)
    expect(resolved?.profileModel).toBe(false)
  })
})
