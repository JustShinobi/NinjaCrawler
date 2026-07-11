import type { TwitterSourceSyncOptions } from './models'

export type TwitterEditableSyncOptionKey = {
  [Key in keyof TwitterSourceSyncOptions]-?: NonNullable<TwitterSourceSyncOptions[Key]> extends boolean | string
    ? Key
    : never
}[keyof TwitterSourceSyncOptions]

export interface TwitterSyncOptionDefinition {
  key: TwitterEditableSyncOptionKey
  label: string
  type: 'boolean' | 'string'
  tooltip?: string
}

export interface TwitterSyncOptionGroupDefinition {
  title: string
  className: string
  options: readonly TwitterSyncOptionDefinition[]
}

// This is the single user-facing schema for Twitter profile parameters. Both
// the profile editor and the batch editor render from it, so adding or removing
// an option here updates both surfaces together.
export const TWITTER_SYNC_OPTION_GROUPS: readonly TwitterSyncOptionGroupDefinition[] = [
  {
    title: 'Download models',
    className: 'source-editor-sync-group-sections',
    options: [
      {
        key: 'mediaModel',
        label: 'Profile posts with media',
        type: 'boolean',
        tooltip: 'Download posts shown in x.com/<user>/media. This is a media-only subset of the profile timeline and excludes reposts from other users.',
      },
      {
        key: 'searchModel',
        label: 'Search',
        type: 'boolean',
        tooltip: 'Download via search (from:<user>, including native retweets). More complete but more rate-limit prone.',
      },
      {
        key: 'likesModel',
        label: 'Liked posts',
        type: 'boolean',
        tooltip: "Download media from posts liked by this profile. The authenticated Account must be able to view the profile's Likes tab.",
      },
      {
        key: 'searchUseGraphqlEndpoint',
        label: 'Search: new endpoint (graphql)',
        type: 'boolean',
        tooltip: "Use -o search-endpoint=graphql for the search model (SCrawler's UseNewEndPointSearch).",
      },
      {
        key: 'profileUseGraphqlEndpoint',
        label: 'Profile: new endpoint (graphql)',
        type: 'boolean',
        tooltip: "Use -o search-endpoint=graphql for the media/profile models (SCrawler's UseNewEndPointProfiles).",
      },
      {
        key: 'allowNonUserTweets',
        label: 'Media: allow non-user tweets',
        type: 'boolean',
        tooltip: 'Allow reposts of other users in the media model (MediaModelAllowNonUserTweets).',
      },
    ],
  },
  {
    title: 'Media',
    className: 'source-editor-sync-group-media',
    options: [
      { key: 'downloadImages', label: 'Download images', type: 'boolean' },
      { key: 'downloadVideos', label: 'Download videos', type: 'boolean' },
      {
        key: 'downloadGifs',
        label: 'Download GIFs',
        type: 'boolean',
        tooltip: 'Download animated GIFs (saved as mp4).',
      },
      {
        key: 'separateVideoFolder',
        label: 'Separate video folder',
        type: 'boolean',
        tooltip: 'Download videos into a "Video" subfolder (SCrawler layout).',
      },
      {
        key: 'gifsSpecialFolder',
        label: 'GIFs special folder',
        type: 'string',
        tooltip: 'Subfolder for GIFs (relative to the profile folder). Empty keeps them with the rest.',
      },
      {
        key: 'gifsPrefix',
        label: 'GIF prefix',
        type: 'string',
        tooltip: 'Filename prefix applied to GIFs (default GIF_).',
      },
      {
        key: 'useMd5Comparison',
        label: 'Use MD5 comparison',
        type: 'boolean',
        tooltip: 'Discard byte-identical downloads by comparing content hashes.',
      },
    ],
  },
  {
    title: 'Storage',
    className: 'source-editor-sync-group-automation',
    options: [
      {
        key: 'specialPath',
        label: 'Special path',
        type: 'string',
        tooltip: "Absolute folder for this profile's media. Leave empty to use the account/global media root.",
      },
    ],
  },
]
