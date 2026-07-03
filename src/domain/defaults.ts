import type { AppSetting, ProviderDescriptor } from './models'
import {
  INSTAGRAM_GLOBAL_PRESET_SETTING_KEYS,
  serializeInstagramGlobalSyncPreset,
  createInstagramSourceSyncPreset,
} from './sourceSyncOptions'

export const DEFAULT_PROVIDER_CATALOG: ProviderDescriptor[] = [
  {
    key: 'instagram',
    displayName: 'Instagram',
    authModes: ['imported_session'],
    sourceKinds: ['profile'],
    supportsMultipleAccounts: true,
    defaultCapabilities: ['posts', 'reels', 'stories', 'saved_posts'],
    notes: 'Primary V1 account model. Multi-account QA is mandatory.',
  },
  {
    key: 'tiktok',
    displayName: 'TikTok',
    authModes: ['imported_session'],
    sourceKinds: ['profile'],
    supportsMultipleAccounts: true,
    defaultCapabilities: ['videos', 'photos'],
    notes: 'Session-backed connector with explicit account binding for every source.',
  },
  {
    key: 'twitter',
    displayName: 'X / Twitter',
    authModes: ['imported_session'],
    sourceKinds: ['profile'],
    supportsMultipleAccounts: false,
    defaultCapabilities: ['posts', 'media_timeline'],
    notes: 'Media timeline sync is session-backed in V1.',
  },
]

export const DEFAULT_APP_SETTINGS: AppSetting[] = [
  {
    key: 'tool.yt-dlp.path',
    value: 'yt-dlp.exe',
    category: 'tools',
    description: 'Binary used for TikTok and video-first provider extraction.',
    mutable: true,
  },
  {
    key: 'tool.gallery-dl.path',
    value: 'gallery-dl.exe',
    category: 'tools',
    description: 'Binary used for gallery-oriented providers and image-heavy flows.',
    mutable: true,
  },
  {
    key: 'tool.instaloader.path',
    value: 'instaloader.exe',
    category: 'tools',
    description: 'Optional Instagram helper for best-effort connector paths.',
    mutable: true,
  },
  {
    key: 'policy.notifications.default',
    value: 'summary',
    category: 'policy',
    description: 'Default post-run notification mode for scheduler plans.',
    mutable: true,
  },
  {
    key: 'policy.session_import.enabled',
    value: 'true',
    category: 'policy',
    description: 'Controls whether manual session import remains available in the UI.',
    mutable: true,
  },
  {
    key: 'policy.sync.blockDuplicateUserId',
    value: 'true',
    category: 'policy',
    description:
      'On a profile first sync, if the resolved user id already belongs to another profile, cancel the sync and remove the newly added duplicate.',
    mutable: true,
  },
  {
    key: 'policy.sync.delayBetweenProfilesSecs',
    value: '0',
    category: 'policy',
    description:
      'Global fallback for the delay (seconds) between consecutive downloads in the sync queue. Used when an account does not set its own per-account delay. Each cookie has its own rate limit, so prefer the per-account field in the account settings. 0 disables.',
    mutable: true,
  },
  {
    key: INSTAGRAM_GLOBAL_PRESET_SETTING_KEYS.preset1,
    value: serializeInstagramGlobalSyncPreset('preset1', createInstagramSourceSyncPreset('preset1')),
    category: 'policy',
    description: 'Provider-wide quick preset slot 1 for Instagram.',
    mutable: true,
  },
  {
    key: INSTAGRAM_GLOBAL_PRESET_SETTING_KEYS.preset2,
    value: serializeInstagramGlobalSyncPreset('preset2', createInstagramSourceSyncPreset('preset2')),
    category: 'policy',
    description: 'Provider-wide quick preset slot 2 for Instagram.',
    mutable: true,
  },
  {
    key: 'storage.media_root',
    value: 'F:\\SCrawler\\Data',
    category: 'storage',
    description: 'Default media storage root for new sources.',
    mutable: true,
  },
  {
    key: 'naming.instagram.media_file_pattern_mode',
    value: 'preset_new_default',
    category: 'storage',
    description: 'Controls how Instagram media file names are generated.',
    mutable: true,
  },
  {
    key: 'naming.instagram.media_file_pattern_template',
    value: '{datetime} {provider_media_key}.{ext}',
    category: 'storage',
    description: 'Template used when custom Instagram media naming mode is selected.',
    mutable: true,
  },
]
