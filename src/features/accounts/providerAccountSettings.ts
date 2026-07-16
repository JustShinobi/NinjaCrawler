import {
  createInstagramExtractImageFromVideoSections,
  createInstagramSourceSyncOptions,
  createTikTokSourceSyncOptions,
  createTwitterSourceSyncOptions,
} from '../../domain/sourceSyncOptions'
import type { ProviderAccountSettingValue, ProviderKey, SourceSyncOptions } from '../../domain/models'

export type ProviderAccountSettingsCategoryKey =
  | 'account'
  | 'extractVideo'
  | 'authorization'
  | 'download'
  | 'timers'
  | 'defaults'
  | 'errors'

export type ProviderAccountSettingsFieldKind = 'text' | 'textarea' | 'number' | 'toggle'

export interface ProviderAccountSettingsCategory {
  key: ProviderAccountSettingsCategoryKey
  label: string
  description: string
}

export interface ProviderAccountSettingsField {
  key: string
  category: ProviderAccountSettingsCategoryKey
  label: string
  description?: string
  tooltip?: string
  kind: ProviderAccountSettingsFieldKind
  placeholder?: string
  defaultValue: string
  /** Technical / rarely-edited fields collapsed under Advanced. */
  advanced?: boolean
  /** Render value with mono font (paths, tokens, headers). */
  mono?: boolean
}

export interface ProviderAccountSettingsLayout {
  categories: ProviderAccountSettingsCategory[]
  fields: ProviderAccountSettingsField[]
}

const INSTAGRAM_SETTINGS_LAYOUT: ProviderAccountSettingsLayout = {
  categories: [
    { key: 'account', label: 'Paths', description: '' },
    { key: 'defaults', label: 'New profile defaults', description: '' },
    { key: 'extractVideo', label: 'Extract image from video', description: '' },
    { key: 'authorization', label: 'Authorization', description: '' },
    { key: 'download', label: 'Download', description: '' },
    { key: 'timers', label: 'Timers', description: '' },
    { key: 'errors', label: 'Errors & limits', description: '' },
  ],
  fields: [
    { key: 'instagram.account.mediaPath', category: 'account', label: 'Media path', kind: 'text', placeholder: 'D:/Media/Instagram/Main', defaultValue: '', mono: true },
    { key: 'instagram.account.savedPostsPath', category: 'account', label: 'Saved posts path', kind: 'text', placeholder: 'D:/Media/Instagram/Saved', defaultValue: '', mono: true },
    { key: 'instagram.account.downloadSavedPosts', category: 'account', label: 'Download saved posts', tooltip: 'Includes saved posts in this account workflow.', kind: 'toggle', defaultValue: 'false' },

    { key: 'instagram.auth.csrfToken', category: 'authorization', label: 'x-csrftoken', tooltip: 'Usually comes from the current cookies.', kind: 'text', defaultValue: '', mono: true },
    { key: 'instagram.auth.appId', category: 'authorization', label: 'x-ig-app-id', kind: 'text', defaultValue: '', mono: true },
    { key: 'instagram.auth.asbdId', category: 'authorization', label: 'x-asbd-id', kind: 'text', defaultValue: '', mono: true },
    { key: 'instagram.auth.igWwwClaim', category: 'authorization', label: 'x-ig-www-claim', kind: 'text', placeholder: 'Optional', defaultValue: '', mono: true },
    { key: 'instagram.auth.secChUa', category: 'authorization', label: 'sec-ch-ua', kind: 'textarea', defaultValue: '', advanced: true, mono: true },
    { key: 'instagram.auth.secChUaFullVersionList', category: 'authorization', label: 'sec-ch-ua-full-version-list', tooltip: 'Optional browser client hint header.', kind: 'textarea', defaultValue: '', advanced: true, mono: true },
    { key: 'instagram.auth.secChUaPlatformVersion', category: 'authorization', label: 'sec-ch-ua-platform-version', tooltip: 'Optional platform version client hint header.', kind: 'text', defaultValue: '', advanced: true, mono: true },
    { key: 'instagram.auth.userAgent', category: 'authorization', label: 'User agent', kind: 'textarea', defaultValue: '', advanced: true, mono: true },
    { key: 'instagram.download.graphQlPrimary', category: 'download', label: 'Use GraphQL to download', kind: 'toggle', defaultValue: 'true' },

    { key: 'instagram.download.timeline', category: 'download', label: 'Download timeline', kind: 'toggle', defaultValue: 'true' },
    { key: 'instagram.download.reels', category: 'download', label: 'Download reels', kind: 'toggle', defaultValue: 'false' },
    { key: 'instagram.download.stories', category: 'download', label: 'Download stories', kind: 'toggle', defaultValue: 'true' },
    { key: 'instagram.download.storiesUser', category: 'download', label: 'Download stories:user', kind: 'toggle', defaultValue: 'true' },
    { key: 'instagram.download.taggedPosts', category: 'download', label: 'Download tagged posts', kind: 'toggle', defaultValue: 'false' },
    { key: 'instagram.download.postCountVerified', category: 'download', label: 'Post count verified', kind: 'number', defaultValue: '0' },
    { key: 'instagram.download.postCountUnverified', category: 'download', label: 'Post count unverified', kind: 'number', defaultValue: '0' },

    { key: 'instagram.timers.requestAnyMs', category: 'timers', label: 'Request timer (any) ms', tooltip: 'Base delay before every Instagram API request.', kind: 'number', defaultValue: '1500' },
    { key: 'instagram.timers.requestMs', category: 'timers', label: 'Request timer ms', tooltip: 'Extra delay applied once every N requests, where N is the request timer counter.', kind: 'number', defaultValue: '1000' },
    { key: 'instagram.timers.requestCounter', category: 'timers', label: 'Request timer counter', tooltip: 'How many requests run before the extra request timer is applied.', kind: 'number', defaultValue: '10' },
    { key: 'instagram.timers.postsLimitMs', category: 'timers', label: 'Posts limit timer ms', tooltip: 'Pause between post pages while listing a profile.', kind: 'number', defaultValue: '3000' },

    { key: 'instagram.defaults.labels', category: 'defaults', label: 'Default labels', kind: 'text', placeholder: 'reference, priority', defaultValue: '' },
    { key: 'instagram.defaults.readyForDownload', category: 'defaults', label: 'Ready for download by default', kind: 'toggle', defaultValue: 'true' },
    { key: 'instagram.defaults.downloadTimeline', category: 'defaults', label: 'Get timeline', kind: 'toggle', defaultValue: 'true' },
    { key: 'instagram.defaults.downloadReels', category: 'defaults', label: 'Get reels', kind: 'toggle', defaultValue: 'false' },
    { key: 'instagram.defaults.downloadStories', category: 'defaults', label: 'Get stories', kind: 'toggle', defaultValue: 'false' },
    { key: 'instagram.defaults.downloadStoriesUser', category: 'defaults', label: 'Get stories: user', kind: 'toggle', defaultValue: 'false' },
    { key: 'instagram.defaults.downloadTaggedPosts', category: 'defaults', label: 'Get tagged posts', kind: 'toggle', defaultValue: 'false' },
    { key: 'instagram.defaults.downloadText', category: 'defaults', label: 'Download text', kind: 'toggle', defaultValue: 'false' },
    { key: 'instagram.defaults.downloadTextPosts', category: 'defaults', label: 'Download text posts', kind: 'toggle', defaultValue: 'false' },
    { key: 'instagram.defaults.textSpecialFolder', category: 'defaults', label: 'Text special folder', kind: 'toggle', defaultValue: 'true' },

    { key: 'instagram.defaults.extractImageFromVideo.timeline', category: 'extractVideo', label: 'From timeline', kind: 'toggle', defaultValue: 'true' },
    { key: 'instagram.defaults.extractImageFromVideo.reels', category: 'extractVideo', label: 'From reels', kind: 'toggle', defaultValue: 'true' },
    { key: 'instagram.defaults.extractImageFromVideo.stories', category: 'extractVideo', label: 'From stories', kind: 'toggle', defaultValue: 'true' },
    { key: 'instagram.defaults.extractImageFromVideo.storiesUser', category: 'extractVideo', label: 'From stories: user', kind: 'toggle', defaultValue: 'true' },
    { key: 'instagram.defaults.extractImageFromVideo.tagged', category: 'extractVideo', label: 'From tagged posts', kind: 'toggle', defaultValue: 'true' },
    { key: 'instagram.account.extractSavedPostsImageFromVideo', category: 'extractVideo', label: 'From saved posts', tooltip: 'Controls saved-post image extraction for this account.', kind: 'toggle', defaultValue: 'true' },
    { key: 'instagram.defaults.placeExtractedImageIntoVideoFolder', category: 'extractVideo', label: 'Place extracted image into the video folder', kind: 'toggle', defaultValue: 'false' },

    { key: 'instagram.errors.ignoreStories560', category: 'errors', label: 'Ignore stories 560', tooltip: 'Skips error 560 and continues the download.', kind: 'toggle', defaultValue: 'true' },
    { key: 'instagram.errors.skipErrors', category: 'errors', label: 'Skip recoverable errors', tooltip: 'Skips listed errors and continues the download.', kind: 'toggle', defaultValue: 'true' },
    { key: 'instagram.errors.addSkippedErrorsToLog', category: 'errors', label: 'Add skipped errors to log', kind: 'toggle', defaultValue: 'true' },
    { key: 'instagram.errors.skipErrorsExclude', category: 'errors', label: 'Skip errors (exclude)', tooltip: 'Comma-separated terms that must not be skipped: a matching error fails the section even with skip enabled.', kind: 'textarea', placeholder: 'checkpoint_required, login_required', defaultValue: '' },
    { key: 'instagram.errors.taggedNotifyLimit', category: 'errors', label: 'Tagged notify limit', tooltip: 'Adds a warning to the sync summary when tagged posts exceed this count. 0 disables the warning.', kind: 'number', defaultValue: '25' },

  ],
}

const TWITTER_SETTINGS_LAYOUT: ProviderAccountSettingsLayout = {
  categories: [
    { key: 'account', label: 'Paths', description: '' },
    { key: 'defaults', label: 'New profile defaults', description: '' },
    { key: 'authorization', label: 'Authorization', description: '' },
    { key: 'download', label: 'Download', description: '' },
  ],
  fields: [
    { key: 'twitter.account.mediaPath', category: 'account', label: 'Path', kind: 'text', placeholder: 'F:/SCrawler/Data/Twitter', defaultValue: '', mono: true },

    { key: 'twitter.auth.useUserAgent', category: 'authorization', label: 'Use user agent', kind: 'toggle', defaultValue: 'true' },
    { key: 'twitter.auth.userAgent', category: 'authorization', label: 'User agent', kind: 'textarea', defaultValue: '', advanced: true, mono: true },

    { key: 'twitter.defaults.labels', category: 'defaults', label: 'Default labels', kind: 'text', placeholder: 'reference, priority', defaultValue: '' },
    { key: 'twitter.defaults.readyForDownload', category: 'defaults', label: 'Ready for download by default', kind: 'toggle', defaultValue: 'true' },
    { key: 'twitter.defaults.mediaModel', category: 'defaults', label: 'Profile posts with media', tooltip: 'Media-only subset of the profile timeline at x.com/<user>/media.', kind: 'toggle', defaultValue: 'true' },
    { key: 'twitter.defaults.searchModel', category: 'defaults', label: "Model 'Search'", tooltip: 'x.com/search?q=from:<user>+include:nativeretweets', kind: 'toggle', defaultValue: 'false' },
    { key: 'twitter.defaults.likesModel', category: 'defaults', label: 'Liked posts', tooltip: 'x.com/<user>/likes; requires an Account that can view the Likes tab.', kind: 'toggle', defaultValue: 'false' },
    { key: 'twitter.defaults.allowNonUserTweets', category: 'defaults', label: 'Media model: allow non-user tweets', kind: 'toggle', defaultValue: 'false' },
    { key: 'twitter.defaults.temporary', category: 'defaults', label: 'Temporary', kind: 'toggle', defaultValue: 'false' },
    { key: 'twitter.defaults.downloadImages', category: 'defaults', label: 'Download images', kind: 'toggle', defaultValue: 'true' },
    { key: 'twitter.defaults.downloadVideos', category: 'defaults', label: 'Download videos', kind: 'toggle', defaultValue: 'true' },
    { key: 'twitter.defaults.downloadGifs', category: 'defaults', label: 'Download GIFs', kind: 'toggle', defaultValue: 'true' },
    { key: 'twitter.defaults.separateVideoFolder', category: 'defaults', label: 'Separate video folder', tooltip: 'Download videos into a "Video" subfolder.', kind: 'toggle', defaultValue: 'true' },
    { key: 'twitter.defaults.gifsSpecialFolder', category: 'defaults', label: 'GIFs special folder', kind: 'text', defaultValue: '' },
    { key: 'twitter.defaults.gifsPrefix', category: 'defaults', label: 'GIF prefix', kind: 'text', defaultValue: 'GIF_' },
    { key: 'twitter.defaults.useMd5Comparison', category: 'defaults', label: 'Use MD5 comparison', kind: 'toggle', defaultValue: 'false' },

    { key: 'twitter.defaults.searchUseGraphqlEndpoint', category: 'download', label: 'New endpoint: search', tooltip: '-o search-endpoint=graphql for the search model.', kind: 'toggle', defaultValue: 'true' },
    { key: 'twitter.defaults.profileUseGraphqlEndpoint', category: 'download', label: 'New endpoint: profiles', tooltip: '-o search-endpoint=graphql for the media/profile models.', kind: 'toggle', defaultValue: 'true' },
  ],
}

const TIKTOK_SETTINGS_LAYOUT: ProviderAccountSettingsLayout = {
  categories: [
    { key: 'account', label: 'Paths', description: '' },
    { key: 'defaults', label: 'New profile defaults', description: '' },
    { key: 'authorization', label: 'Authorization', description: '' },
    { key: 'download', label: 'Download', description: '' },
    { key: 'timers', label: 'Timers', description: '' },
  ],
  fields: [
    { key: 'tiktok.account.mediaPath', category: 'account', label: 'Path', kind: 'text', placeholder: 'F:/SCrawler/Data/TikTok', defaultValue: '', mono: true },
    { key: 'tiktok.auth.useUserAgent', category: 'authorization', label: 'Use user agent', kind: 'toggle', defaultValue: 'true' },
    { key: 'tiktok.auth.userAgent', category: 'authorization', label: 'User agent', kind: 'textarea', defaultValue: '', advanced: true, mono: true },

    { key: 'tiktok.defaults.labels', category: 'defaults', label: 'Default labels', kind: 'text', placeholder: 'reference, priority', defaultValue: '' },
    { key: 'tiktok.defaults.readyForDownload', category: 'defaults', label: 'Ready for download by default', kind: 'toggle', defaultValue: 'true' },
    { key: 'tiktok.defaults.getTimeline', category: 'defaults', label: 'Get Timeline', tooltip: 'tiktok.com/@<user>', kind: 'toggle', defaultValue: 'true' },
    { key: 'tiktok.defaults.getStoriesUser', category: 'defaults', label: 'Get User Stories', kind: 'toggle', defaultValue: 'false' },
    { key: 'tiktok.defaults.getReposts', category: 'defaults', label: 'Get Reposts', kind: 'toggle', defaultValue: 'false' },
    { key: 'tiktok.defaults.getLikedVideos', category: 'defaults', label: 'Get Liked Videos', kind: 'toggle', defaultValue: 'false' },
    { key: 'tiktok.defaults.downloadVideos', category: 'defaults', label: 'Download videos', tooltip: 'Videos are fetched with yt-dlp.', kind: 'toggle', defaultValue: 'true' },
    { key: 'tiktok.defaults.downloadPhotos', category: 'defaults', label: 'Download photos', tooltip: 'Slideshow posts are parsed with gallery-dl.', kind: 'toggle', defaultValue: 'true' },
    { key: 'tiktok.defaults.separateVideoFolder', category: 'defaults', label: 'Separate video folder', tooltip: 'Download videos into a "Video" subfolder.', kind: 'toggle', defaultValue: 'false' },
    { key: 'tiktok.defaults.temporary', category: 'defaults', label: 'Temporary', kind: 'toggle', defaultValue: 'false' },

    { key: 'tiktok.defaults.useNativeTitle', category: 'download', label: 'Use native title', tooltip: 'Use the video title for the filename instead of the post id.', kind: 'toggle', defaultValue: 'false' },
    { key: 'tiktok.defaults.addVideoIdToTitle', category: 'download', label: 'Add video id to title', kind: 'toggle', defaultValue: 'true' },
    { key: 'tiktok.defaults.removeTagsFromTitle', category: 'download', label: 'Remove tags from title', kind: 'toggle', defaultValue: 'false' },
    { key: 'tiktok.defaults.useParsedVideoDate', category: 'download', label: 'Use video date as file date', kind: 'toggle', defaultValue: 'true' },
    { key: 'tiktok.defaults.abortOnLimit', category: 'download', label: 'Abort on limit', kind: 'toggle', defaultValue: 'true' },
    { key: 'tiktok.defaults.sleepTimerSecs', category: 'timers', label: 'Sleep timer (s)', tooltip: 'Seconds between sections. -1 disables.', kind: 'number', defaultValue: '-1' },
    { key: 'tiktok.account.delayBetweenDownloadsSecs', category: 'timers', label: 'Delay between profiles (s)', tooltip: "Seconds the sync queue waits after each of this account's profiles before the next download. 0 falls back to the global default on the Accounts → Workspace tab.", kind: 'number', defaultValue: '0' },
  ],
}

export function getProviderAccountSettingsLayout(provider: ProviderKey): ProviderAccountSettingsLayout | undefined {
  if (provider === 'instagram') {
    return INSTAGRAM_SETTINGS_LAYOUT
  }

  if (provider === 'twitter') {
    return TWITTER_SETTINGS_LAYOUT
  }

  if (provider === 'tiktok') {
    return TIKTOK_SETTINGS_LAYOUT
  }

  return undefined
}

export function buildProviderAccountSettingsDraft(
  provider: ProviderKey,
  settings: ProviderAccountSettingValue[],
): Record<string, string> {
  const layout = getProviderAccountSettingsLayout(provider)
  if (!layout) {
    return {}
  }

  const settingsByKey = new Map(settings.map((setting) => [setting.settingKey, setting]))
  const draft: Record<string, string> = {}

  for (const field of layout.fields) {
    const savedValue = settingsByKey.get(field.key)
    if (!savedValue) {
      draft[field.key] = field.defaultValue
      continue
    }

    draft[field.key] = savedValue.valueKind === 'json'
      ? JSON.stringify(savedValue.jsonValue ?? '')
      : savedValue.stringValue ?? field.defaultValue
  }

  return draft
}

export function serializeProviderAccountSettingsDraft(
  provider: ProviderKey,
  draft: Record<string, string>,
): ProviderAccountSettingValue[] {
  const layout = getProviderAccountSettingsLayout(provider)
  if (!layout) {
    return []
  }

  return layout.fields.map((field) => ({
    settingKey: field.key,
    valueKind: 'string',
    stringValue: draft[field.key] ?? field.defaultValue,
    jsonValue: undefined,
  }))
}

export function getProviderAccountSettingsFields(
  provider: ProviderKey,
  category: ProviderAccountSettingsCategoryKey,
): ProviderAccountSettingsField[] {
  const layout = getProviderAccountSettingsLayout(provider)
  if (!layout) {
    return []
  }

  return layout.fields.filter((field) => field.category === category)
}

export interface SourceDefaultsFromAccountSettings {
  labels: string[]
  readyForDownload?: boolean
  syncOptions?: SourceSyncOptions
}

function parseOptionalToggle(value: string | undefined): boolean | undefined {
  if (value === 'true') {
    return true
  }

  if (value === 'false') {
    return false
  }

  return undefined
}

function parseOptionalNumber(value: string | undefined): number | undefined {
  if (value === undefined || value.trim() === '') {
    return undefined
  }
  const parsed = Number.parseInt(value, 10)
  return Number.isNaN(parsed) ? undefined : parsed
}

function splitLabels(value: string | undefined): string[] {
  return (value ?? '')
    .split(',')
    .map((entry) => entry.trim())
    .filter((entry) => entry.length > 0)
}

export function extractSourceDefaultsFromAccountSettings(
  provider: ProviderKey,
  draft: Record<string, string>,
): SourceDefaultsFromAccountSettings {
  if (provider === 'twitter') {
    const readyValue = draft['twitter.defaults.readyForDownload']
    return {
      labels: splitLabels(draft['twitter.defaults.labels']),
      readyForDownload: readyValue === 'true' ? true : readyValue === 'false' ? false : undefined,
      syncOptions: {
        twitter: createTwitterSourceSyncOptions({
          mediaModel: parseOptionalToggle(draft['twitter.defaults.mediaModel']),
          profileModel: false,
          searchModel: parseOptionalToggle(draft['twitter.defaults.searchModel']),
          likesModel: parseOptionalToggle(draft['twitter.defaults.likesModel']),
          searchUseGraphqlEndpoint: parseOptionalToggle(draft['twitter.defaults.searchUseGraphqlEndpoint']),
          profileUseGraphqlEndpoint: parseOptionalToggle(draft['twitter.defaults.profileUseGraphqlEndpoint']),
          allowNonUserTweets: parseOptionalToggle(draft['twitter.defaults.allowNonUserTweets']),
          downloadImages: parseOptionalToggle(draft['twitter.defaults.downloadImages']),
          downloadVideos: parseOptionalToggle(draft['twitter.defaults.downloadVideos']),
          downloadGifs: parseOptionalToggle(draft['twitter.defaults.downloadGifs']),
          separateVideoFolder: parseOptionalToggle(draft['twitter.defaults.separateVideoFolder']),
          gifsSpecialFolder: draft['twitter.defaults.gifsSpecialFolder'],
          gifsPrefix: draft['twitter.defaults.gifsPrefix'],
          useMd5Comparison: parseOptionalToggle(draft['twitter.defaults.useMd5Comparison']),
          temporary: parseOptionalToggle(draft['twitter.defaults.temporary']),
        }),
      },
    }
  }

  if (provider === 'tiktok') {
    const readyValue = draft['tiktok.defaults.readyForDownload']
    return {
      labels: splitLabels(draft['tiktok.defaults.labels']),
      readyForDownload: readyValue === 'true' ? true : readyValue === 'false' ? false : undefined,
      syncOptions: {
        tiktok: createTikTokSourceSyncOptions({
          getTimeline: parseOptionalToggle(draft['tiktok.defaults.getTimeline']),
          getStoriesUser: parseOptionalToggle(draft['tiktok.defaults.getStoriesUser']),
          getReposts: parseOptionalToggle(draft['tiktok.defaults.getReposts']),
          getLikedVideos: parseOptionalToggle(draft['tiktok.defaults.getLikedVideos']),
          downloadVideos: parseOptionalToggle(draft['tiktok.defaults.downloadVideos']),
          downloadPhotos: parseOptionalToggle(draft['tiktok.defaults.downloadPhotos']),
          useNativeTitle: parseOptionalToggle(draft['tiktok.defaults.useNativeTitle']),
          addVideoIdToTitle: parseOptionalToggle(draft['tiktok.defaults.addVideoIdToTitle']),
          removeTagsFromTitle: parseOptionalToggle(draft['tiktok.defaults.removeTagsFromTitle']),
          useParsedVideoDate: parseOptionalToggle(draft['tiktok.defaults.useParsedVideoDate']),
          separateVideoFolder: parseOptionalToggle(draft['tiktok.defaults.separateVideoFolder']),
          abortOnLimit: parseOptionalToggle(draft['tiktok.defaults.abortOnLimit']),
          sleepTimerSecs: parseOptionalNumber(draft['tiktok.defaults.sleepTimerSecs']),
          temporary: parseOptionalToggle(draft['tiktok.defaults.temporary']),
        }),
      },
    }
  }

  if (provider !== 'instagram') {
    return { labels: [] }
  }

  const labels = (draft['instagram.defaults.labels'] ?? '')
    .split(',')
    .map((entry) => entry.trim())
    .filter((entry) => entry.length > 0)

  const readyValue = draft['instagram.defaults.readyForDownload']

  return {
    labels,
    readyForDownload: readyValue === 'true'
      ? true
      : readyValue === 'false'
        ? false
        : undefined,
    syncOptions: {
      instagram: createInstagramSourceSyncOptions({
        timeline:
          parseOptionalToggle(draft['instagram.defaults.downloadTimeline'])
          ?? parseOptionalToggle(draft['instagram.defaults.timeline']),
        reels:
          parseOptionalToggle(draft['instagram.defaults.downloadReels'])
          ?? parseOptionalToggle(draft['instagram.defaults.reels']),
        stories: parseOptionalToggle(draft['instagram.defaults.downloadStories']),
        storiesUser:
          parseOptionalToggle(draft['instagram.defaults.downloadStoriesUser'])
          ?? parseOptionalToggle(draft['instagram.defaults.storiesUser']),
        tagged:
          parseOptionalToggle(draft['instagram.defaults.downloadTaggedPosts'])
          ?? parseOptionalToggle(draft['instagram.defaults.taggedPosts']),
        downloadText: parseOptionalToggle(draft['instagram.defaults.downloadText']),
        downloadTextPosts: parseOptionalToggle(draft['instagram.defaults.downloadTextPosts']),
        textSpecialFolder: parseOptionalToggle(draft['instagram.defaults.textSpecialFolder']),
        extractImageFromVideo: createInstagramExtractImageFromVideoSections({
          timeline: parseOptionalToggle(draft['instagram.defaults.extractImageFromVideo.timeline']),
          reels: parseOptionalToggle(draft['instagram.defaults.extractImageFromVideo.reels']),
          stories: parseOptionalToggle(draft['instagram.defaults.extractImageFromVideo.stories']),
          storiesUser: parseOptionalToggle(draft['instagram.defaults.extractImageFromVideo.storiesUser']),
          tagged: parseOptionalToggle(draft['instagram.defaults.extractImageFromVideo.tagged']),
        }),
        placeExtractedImageIntoVideoFolder: parseOptionalToggle(draft['instagram.defaults.placeExtractedImageIntoVideoFolder']),
      }),
    },
  }
}
