import type {
  AppSetting,
  InstagramExtractImageFromVideoSections,
  InstagramPresetSlot,
  InstagramSourceSyncOptions,
  InstagramSourceSyncPreset,
  InstagramSourceSyncSections,
  ProviderKey,
  SourceSyncOptions,
  TikTokSourceSyncOptions,
  TwitterSourceSyncOptions,
} from './models'

type InstagramSourceSyncPresetOverrides = Partial<Omit<InstagramSourceSyncPreset, 'sections'>> & {
  sections?: Partial<InstagramSourceSyncSections> | null
}

export const DEFAULT_INSTAGRAM_SOURCE_SYNC_SECTIONS: InstagramSourceSyncSections = {
  timeline: true,
  reels: false,
  stories: false,
  storiesUser: false,
  tagged: false,
}

export const DEFAULT_INSTAGRAM_EXTRACT_IMAGE_FROM_VIDEO_SECTIONS: InstagramExtractImageFromVideoSections = {
  timeline: true,
  reels: true,
  stories: true,
  storiesUser: true,
  tagged: true,
}

export const DEFAULT_INSTAGRAM_PRESET_LABELS: Record<InstagramPresetSlot, string> = {
  preset1: 'Preset 1',
  preset2: 'Preset 2',
}

export const INSTAGRAM_GLOBAL_PRESET_SETTING_KEYS: Record<InstagramPresetSlot, string> = {
  preset1: 'instagram.sync.globalPreset1',
  preset2: 'instagram.sync.globalPreset2',
}

export const DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS: InstagramSourceSyncOptions = {
  ...DEFAULT_INSTAGRAM_SOURCE_SYNC_SECTIONS,
  temporary: false,
  favorite: false,
  downloadImages: true,
  downloadVideos: true,
  getUserMediaOnly: false,
  missingOnly: false,
  fullScan: false,
  dateFrom: '',
  dateTo: '',
  verifiedProfile: true,
  forceUpdateUserName: true,
  forceUpdateUserInformation: false,
  extractImageFromVideo: { ...DEFAULT_INSTAGRAM_EXTRACT_IMAGE_FROM_VIDEO_SECTIONS },
  placeExtractedImageIntoVideoFolder: false,
  downloadText: false,
  downloadTextPosts: false,
  textSpecialFolder: true,
  specialPath: '',
  usernameOverride: '',
  scriptEnabled: false,
  script: '',
  description: '',
  color: '',
}

export function createInstagramSourceSyncSections(
  overrides?: Partial<InstagramSourceSyncSections> | null,
): InstagramSourceSyncSections {
  return {
    timeline: overrides?.timeline ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_SECTIONS.timeline,
    reels: overrides?.reels ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_SECTIONS.reels,
    stories: overrides?.stories ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_SECTIONS.stories,
    storiesUser: overrides?.storiesUser ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_SECTIONS.storiesUser,
    tagged: overrides?.tagged ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_SECTIONS.tagged,
  }
}

export function createInstagramExtractImageFromVideoSections(
  overrides?: Partial<InstagramExtractImageFromVideoSections> | null,
): InstagramExtractImageFromVideoSections {
  return {
    timeline: overrides?.timeline ?? DEFAULT_INSTAGRAM_EXTRACT_IMAGE_FROM_VIDEO_SECTIONS.timeline,
    reels: overrides?.reels ?? DEFAULT_INSTAGRAM_EXTRACT_IMAGE_FROM_VIDEO_SECTIONS.reels,
    stories: overrides?.stories ?? DEFAULT_INSTAGRAM_EXTRACT_IMAGE_FROM_VIDEO_SECTIONS.stories,
    storiesUser: overrides?.storiesUser ?? DEFAULT_INSTAGRAM_EXTRACT_IMAGE_FROM_VIDEO_SECTIONS.storiesUser,
    tagged: overrides?.tagged ?? DEFAULT_INSTAGRAM_EXTRACT_IMAGE_FROM_VIDEO_SECTIONS.tagged,
  }
}

function normalizedTextValue(value: string | undefined): string {
  return value?.trim() ?? ''
}

export function createInstagramSourceSyncPreset(
  slot: InstagramPresetSlot,
  overrides?: InstagramSourceSyncPresetOverrides | null,
  fallbackSections?: Partial<InstagramSourceSyncSections> | null,
): InstagramSourceSyncPreset {
  const seedSections = createInstagramSourceSyncSections(fallbackSections)

  return {
    enabled: overrides?.enabled ?? false,
    label: overrides?.label?.trim() || DEFAULT_INSTAGRAM_PRESET_LABELS[slot],
    sections: createInstagramSourceSyncSections(overrides?.sections ?? seedSections),
  }
}

export function createInstagramSourceSyncOptions(
  overrides?: Partial<InstagramSourceSyncOptions> | null,
): InstagramSourceSyncOptions {
  const baseSections = createInstagramSourceSyncSections(overrides)
  const extractImageFromVideo = createInstagramExtractImageFromVideoSections(overrides?.extractImageFromVideo)

  return {
    ...baseSections,
    temporary: overrides?.temporary ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.temporary,
    favorite: overrides?.favorite ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.favorite,
    downloadImages: overrides?.downloadImages ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.downloadImages,
    downloadVideos: overrides?.downloadVideos ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.downloadVideos,
    getUserMediaOnly: overrides?.getUserMediaOnly ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.getUserMediaOnly,
    missingOnly: overrides?.missingOnly ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.missingOnly,
    fullScan: overrides?.fullScan ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.fullScan,
    dateFrom: normalizedTextValue(overrides?.dateFrom),
    dateTo: normalizedTextValue(overrides?.dateTo),
    verifiedProfile: overrides?.verifiedProfile ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.verifiedProfile,
    forceUpdateUserName: overrides?.forceUpdateUserName ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.forceUpdateUserName,
    forceUpdateUserInformation: overrides?.forceUpdateUserInformation ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.forceUpdateUserInformation,
    extractImageFromVideo,
    placeExtractedImageIntoVideoFolder:
      overrides?.placeExtractedImageIntoVideoFolder
      ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.placeExtractedImageIntoVideoFolder,
    downloadText: overrides?.downloadText ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.downloadText,
    downloadTextPosts: overrides?.downloadTextPosts ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.downloadTextPosts,
    targetStoryMediaId: normalizedTextValue(overrides?.targetStoryMediaId),
    textSpecialFolder: overrides?.textSpecialFolder ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.textSpecialFolder,
    specialPath: normalizedTextValue(overrides?.specialPath),
    usernameOverride: normalizedTextValue(overrides?.usernameOverride),
    scriptEnabled: overrides?.scriptEnabled ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.scriptEnabled,
    script: overrides?.script ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.script,
    description: overrides?.description ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.description,
    color: overrides?.color ?? DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.color,
    userIdHint: overrides?.userIdHint,
  }
}

// Espelho dos defaults do módulo Twitter do SCrawler: ambos os modelos ligados,
// abort/cooldown ligados, sleep desabilitado (-1) e before-first reutilizando o
// valor do sleep timer (-2).
export const DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS: TwitterSourceSyncOptions = {
  mediaModel: true,
  profileModel: true,
  searchModel: false,
  likesModel: false,
  searchUseGraphqlEndpoint: true,
  profileUseGraphqlEndpoint: true,
  allowNonUserTweets: false,
  abortOnLimit: true,
  downloadAlreadyParsed: true,
  sleepTimerSecs: -1,
  sleepTimerBeforeFirstSecs: -2,
  downloadImages: true,
  downloadVideos: true,
  downloadGifs: true,
  separateVideoFolder: true,
  gifsSpecialFolder: '',
  gifsPrefix: 'GIF_',
  useMd5Comparison: false,
  temporary: false,
  specialPath: '',
  description: '',
  color: '',
}

export function createTwitterSourceSyncOptions(
  overrides?: Partial<TwitterSourceSyncOptions> | null,
): TwitterSourceSyncOptions {
  return {
    mediaModel: overrides?.mediaModel ?? DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.mediaModel,
    profileModel: overrides?.profileModel ?? DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.profileModel,
    searchModel: overrides?.searchModel ?? DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.searchModel,
    likesModel: overrides?.likesModel ?? DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.likesModel,
    searchUseGraphqlEndpoint:
      overrides?.searchUseGraphqlEndpoint ?? DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.searchUseGraphqlEndpoint,
    profileUseGraphqlEndpoint:
      overrides?.profileUseGraphqlEndpoint ?? DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.profileUseGraphqlEndpoint,
    allowNonUserTweets: overrides?.allowNonUserTweets ?? DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.allowNonUserTweets,
    abortOnLimit: overrides?.abortOnLimit ?? DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.abortOnLimit,
    downloadAlreadyParsed:
      overrides?.downloadAlreadyParsed ?? DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.downloadAlreadyParsed,
    sleepTimerSecs: overrides?.sleepTimerSecs ?? DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.sleepTimerSecs,
    sleepTimerBeforeFirstSecs:
      overrides?.sleepTimerBeforeFirstSecs ?? DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.sleepTimerBeforeFirstSecs,
    downloadImages: overrides?.downloadImages ?? DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.downloadImages,
    downloadVideos: overrides?.downloadVideos ?? DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.downloadVideos,
    downloadGifs: overrides?.downloadGifs ?? DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.downloadGifs,
    separateVideoFolder: overrides?.separateVideoFolder ?? DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.separateVideoFolder,
    gifsSpecialFolder: normalizedTextValue(overrides?.gifsSpecialFolder),
    gifsPrefix: overrides?.gifsPrefix ?? DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.gifsPrefix,
    useMd5Comparison: overrides?.useMd5Comparison ?? DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.useMd5Comparison,
    temporary: overrides?.temporary ?? DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.temporary,
    specialPath: normalizedTextValue(overrides?.specialPath),
    description: overrides?.description ?? DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.description,
    color: overrides?.color ?? DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.color,
    userIdHint: overrides?.userIdHint,
  }
}

// Espelho dos defaults do módulo TikTok do SCrawler: timeline ligada,
// stories/reposts desligadas, vídeos (yt-dlp) e fotos (gallery-dl) ligados.
export const DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS: TikTokSourceSyncOptions = {
  getTimeline: true,
  getStoriesUser: false,
  getReposts: false,
  getLikedVideos: false,
  likedVideosLimit: 100,
  likedVideosIncremental: true,
  likedVideosKnownPageThreshold: 3,
  collectMediaStats: true,
  refreshExistingMediaStats: false,
  downloadVideos: true,
  downloadPhotos: true,
  useNativeTitle: false,
  addVideoIdToTitle: true,
  removeTagsFromTitle: false,
  tokkitFileNaming: false,
  useParsedVideoDate: true,
  separateVideoFolder: false,
  abortOnLimit: true,
  sleepTimerSecs: -1,
  temporary: false,
  specialPath: '',
  description: '',
  color: '',
}

export function createTikTokSourceSyncOptions(
  overrides?: Partial<TikTokSourceSyncOptions> | null,
): TikTokSourceSyncOptions {
  return {
    getTimeline: overrides?.getTimeline ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.getTimeline,
    getStoriesUser: overrides?.getStoriesUser ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.getStoriesUser,
    getReposts: overrides?.getReposts ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.getReposts,
    getLikedVideos: overrides?.getLikedVideos ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.getLikedVideos,
    likedVideosLimit: Math.max(0, Math.trunc(overrides?.likedVideosLimit ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.likedVideosLimit ?? 100)),
    likedVideosIncremental:
      overrides?.likedVideosIncremental ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.likedVideosIncremental,
    likedVideosKnownPageThreshold: Math.max(
      1,
      Math.trunc(
        overrides?.likedVideosKnownPageThreshold
          ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.likedVideosKnownPageThreshold
          ?? 3,
      ),
    ),
    collectMediaStats: overrides?.collectMediaStats ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.collectMediaStats,
    refreshExistingMediaStats:
      overrides?.refreshExistingMediaStats ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.refreshExistingMediaStats,
    downloadVideos: overrides?.downloadVideos ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.downloadVideos,
    downloadPhotos: overrides?.downloadPhotos ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.downloadPhotos,
    useNativeTitle: overrides?.useNativeTitle ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.useNativeTitle,
    addVideoIdToTitle: overrides?.addVideoIdToTitle ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.addVideoIdToTitle,
    removeTagsFromTitle: overrides?.removeTagsFromTitle ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.removeTagsFromTitle,
    tokkitFileNaming: overrides?.tokkitFileNaming ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.tokkitFileNaming,
    useParsedVideoDate: overrides?.useParsedVideoDate ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.useParsedVideoDate,
    downloadFromDate: overrides?.downloadFromDate ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.downloadFromDate,
    downloadToDate: overrides?.downloadToDate ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.downloadToDate,
    separateVideoFolder: overrides?.separateVideoFolder ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.separateVideoFolder,
    abortOnLimit: overrides?.abortOnLimit ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.abortOnLimit,
    sleepTimerSecs: overrides?.sleepTimerSecs ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.sleepTimerSecs,
    temporary: overrides?.temporary ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.temporary,
    specialPath: normalizedTextValue(overrides?.specialPath),
    description: overrides?.description ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.description,
    color: overrides?.color ?? DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.color,
    userIdHint: overrides?.userIdHint,
  }
}

export function createSourceSyncOptions(
  provider: ProviderKey,
  overrides?: SourceSyncOptions | null,
): SourceSyncOptions {
  if (provider === 'instagram') {
    return {
      instagram: createInstagramSourceSyncOptions(overrides?.instagram),
    }
  }

  if (provider === 'twitter') {
    return {
      twitter: createTwitterSourceSyncOptions(overrides?.twitter),
    }
  }

  if (provider === 'tiktok') {
    return {
      tiktok: createTikTokSourceSyncOptions(overrides?.tiktok),
    }
  }

  return {}
}

export function resolveTikTokSourceSyncOptions(
  provider: ProviderKey,
  syncOptions?: SourceSyncOptions | null,
): TikTokSourceSyncOptions | undefined {
  if (provider !== 'tiktok') {
    return undefined
  }

  return createTikTokSourceSyncOptions(syncOptions?.tiktok)
}

export function resolveInstagramSourceSyncOptions(
  provider: ProviderKey,
  syncOptions?: SourceSyncOptions | null,
): InstagramSourceSyncOptions | undefined {
  if (provider !== 'instagram') {
    return undefined
  }

  return createInstagramSourceSyncOptions(syncOptions?.instagram)
}

export function resolveTwitterSourceSyncOptions(
  provider: ProviderKey,
  syncOptions?: SourceSyncOptions | null,
): TwitterSourceSyncOptions | undefined {
  if (provider !== 'twitter') {
    return undefined
  }

  return createTwitterSourceSyncOptions(syncOptions?.twitter)
}

function parseGlobalPresetSettingValue(
  rawValue: string | undefined,
  slot: InstagramPresetSlot,
): InstagramSourceSyncPreset {
  if (!rawValue?.trim()) {
    return createInstagramSourceSyncPreset(slot)
  }

  try {
    const parsed = JSON.parse(rawValue) as InstagramSourceSyncPresetOverrides
    return createInstagramSourceSyncPreset(slot, parsed)
  } catch {
    return createInstagramSourceSyncPreset(slot)
  }
}

export function serializeInstagramGlobalSyncPreset(
  slot: InstagramPresetSlot,
  preset: InstagramSourceSyncPreset,
): string {
  return JSON.stringify(createInstagramSourceSyncPreset(slot, preset))
}

export function resolveInstagramGlobalSyncPreset(
  appSettings: AppSetting[] | undefined,
  slot: InstagramPresetSlot,
): InstagramSourceSyncPreset {
  const settingKey = INSTAGRAM_GLOBAL_PRESET_SETTING_KEYS[slot]
  const value = appSettings?.find((setting) => setting.key === settingKey)?.value
  return parseGlobalPresetSettingValue(value, slot)
}
