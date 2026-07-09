import { useEffect, useId, useMemo, useRef, useState, type ReactNode } from 'react'
import { loadSystemShortDatePattern } from '../../bridge/desktop'
import { HelpTip } from '../shared/HelpTip'
import {
  resolveInstagramSourceSyncOptions,
  resolveTikTokSourceSyncOptions,
  resolveTwitterSourceSyncOptions,
} from '../../domain/sourceSyncOptions'
import type {
  ProviderKey,
  SourceProfile,
  SourceProfileUpsert,
} from '../../domain/models'

type InstagramSyncUpsertOptions = NonNullable<NonNullable<SourceProfileUpsert['syncOptions']>['instagram']>
type TwitterSyncUpsertOptions = NonNullable<NonNullable<SourceProfileUpsert['syncOptions']>['twitter']>
type TikTokSyncUpsertOptions = NonNullable<NonNullable<SourceProfileUpsert['syncOptions']>['tiktok']>

interface LocalDateFormat {
  pattern: string
  order: Array<'day' | 'month' | 'year'>
  placeholder: string
}

interface SourceEditorSyncPanelProps {
  provider: ProviderKey
  providerDisplayName: string
  providerNote?: string
  source?: SourceProfile
  onForceImportedBackfill?: () => void | Promise<void>
  syncOptions: SourceProfileUpsert['syncOptions']
  onInstagramSyncOptionsChange: (mutate: (current: InstagramSyncUpsertOptions) => InstagramSyncUpsertOptions) => void
  onTwitterSyncOptionsChange: (mutate: (current: TwitterSyncUpsertOptions) => TwitterSyncUpsertOptions) => void
  onTikTokSyncOptionsChange: (mutate: (current: TikTokSyncUpsertOptions) => TikTokSyncUpsertOptions) => void
}

interface ToggleDefinition {
  key: keyof InstagramSyncUpsertOptions
  label: string
  tooltip?: string
}

const SECTION_TOGGLES: Array<{ key: 'timeline' | 'reels' | 'stories' | 'storiesUser' | 'tagged', label: string, tooltip?: string }> = [
  { key: 'timeline', label: 'Timeline' },
  { key: 'reels', label: 'Reels' },
  { key: 'stories', label: 'Stories' },
  { key: 'storiesUser', label: 'Stories (user)' },
  { key: 'tagged', label: 'Tagged' },
]

const BEHAVIOR_TOGGLES: ToggleDefinition[] = [
  { key: 'temporary', label: 'Temporary' },
  { key: 'favorite', label: 'Favorite' },
  { key: 'getUserMediaOnly', label: 'User media only', tooltip: 'Focus on media instead of extra metadata.' },
  { key: 'verifiedProfile', label: 'Verified profile', tooltip: 'Use verified-profile post count behavior.' },
  { key: 'forceUpdateUserName', label: 'Force update username' },
  { key: 'forceUpdateUserInformation', label: 'Force update user information' },
  { key: 'downloadText', label: 'Download text' },
  { key: 'downloadTextPosts', label: 'Download text posts' },
]

const MEDIA_TOGGLES: ToggleDefinition[] = [
  { key: 'downloadImages', label: 'Download images' },
  { key: 'downloadVideos', label: 'Download videos' },
  { key: 'placeExtractedImageIntoVideoFolder', label: 'Place extracted image in video folder', tooltip: 'Keep extracted images beside the video.' },
]

const EXTRACT_TOGGLES: Array<{ key: 'timeline' | 'reels' | 'stories' | 'storiesUser' | 'tagged', label: string, tooltip: string }> = [
  { key: 'timeline', label: 'Extract timeline', tooltip: 'Extract still images from timeline videos.' },
  { key: 'reels', label: 'Extract reels', tooltip: 'Extract still images from reels.' },
  { key: 'stories', label: 'Extract stories', tooltip: 'Extract still images from stories.' },
  { key: 'storiesUser', label: 'Extract stories (user)', tooltip: 'Extract still images from user-story downloads.' },
  { key: 'tagged', label: 'Extract tagged', tooltip: 'Extract still images from tagged posts.' },
]

export function SourceEditorSyncPanel({
  provider,
  providerDisplayName,
  providerNote,
  source,
  onForceImportedBackfill,
  syncOptions,
  onInstagramSyncOptionsChange,
  onTwitterSyncOptionsChange,
  onTikTokSyncOptionsChange,
}: SourceEditorSyncPanelProps) {
  const [localDateFormat, setLocalDateFormat] = useState<LocalDateFormat>(() => detectLocalDateFormat())
  const instagramSyncOptions = useMemo(
    () => resolveInstagramSourceSyncOptions(provider, syncOptions),
    [provider, syncOptions],
  )
  const twitterSyncOptions = useMemo(
    () => resolveTwitterSourceSyncOptions(provider, syncOptions),
    [provider, syncOptions],
  )
  const tiktokSyncOptions = useMemo(
    () => resolveTikTokSourceSyncOptions(provider, syncOptions),
    [provider, syncOptions],
  )

  useEffect(() => {
    let disposed = false
    void loadSystemShortDatePattern()
      .then((pattern) => {
        if (!disposed && pattern?.trim()) {
          setLocalDateFormat(localDateFormatFromPattern(pattern))
        }
      })
      .catch(() => undefined)

    return () => {
      disposed = true
    }
  }, [])

  if (twitterSyncOptions) {
    return (
      <TwitterSyncPanel
        onTwitterSyncOptionsChange={onTwitterSyncOptionsChange}
        twitterSyncOptions={twitterSyncOptions}
      />
    )
  }

  if (tiktokSyncOptions) {
    return (
      <TikTokSyncPanel
        localDateFormat={localDateFormat}
        onTikTokSyncOptionsChange={onTikTokSyncOptionsChange}
        tiktokSyncOptions={tiktokSyncOptions}
      />
    )
  }

  if (!instagramSyncOptions) {
    return (
      <div className="source-editor-sync-provider-placeholder">
        <strong>{providerDisplayName} sync editor not modeled yet</strong>
        <p>
          This provider keeps the shared `Profile` workflow, but its provider-specific sync controls still need a dedicated layout.
        </p>
        {providerNote ? <small>{providerNote}</small> : null}
      </div>
    )
  }

  return (
    <div className="source-editor-sync-shell">
      {source?.importedAt ? (
        <section className="source-editor-sync-group-card source-editor-sync-group-legacy-import">
          <header className="source-editor-sync-group-header">
            <strong>Legacy Import</strong>
          </header>
          <div className="source-editor-setting-list">
            <div className="source-editor-setting-row">
              <div className="source-editor-setting-copy">
                <span>Imported at</span>
              </div>
              <span>{source.importedAt}</span>
            </div>
            <div className="source-editor-setting-row">
              <div className="source-editor-setting-copy">
                <span>Importer</span>
              </div>
              <span>{source.importerId ?? 'unknown'}</span>
            </div>
            {source.id && onForceImportedBackfill ? (
              <div className="source-editor-setting-row">
                <div className="source-editor-setting-copy">
                  <span>Recovery</span>
                  <small>Run one sync pass without the implicit imported cutoff.</small>
                </div>
                <button className="ghost-button" onClick={() => void onForceImportedBackfill()} type="button">
                  Force legacy backfill
                </button>
              </div>
            ) : null}
          </div>
        </section>
      ) : null}
      <div className="source-editor-sync-groups">
        <div className="source-editor-sync-column">
          <SyncGroupCard className="source-editor-sync-group-sections" title="Sections">
            {SECTION_TOGGLES.map((entry) => (
              <ToggleRow
                checked={Boolean(instagramSyncOptions[entry.key])}
                key={entry.key}
                label={entry.label}
                onChange={(checked) => updateInstagramOption(onInstagramSyncOptionsChange, entry.key, checked)}
                tooltip={entry.tooltip}
              />
            ))}
          </SyncGroupCard>

          <SyncGroupCard className="source-editor-sync-group-media" title="Media">
            {MEDIA_TOGGLES.map((entry) => (
              <ToggleRow
                checked={Boolean(instagramSyncOptions[entry.key])}
                key={entry.key}
                label={entry.label}
                onChange={(checked) => updateInstagramOption(onInstagramSyncOptionsChange, entry.key, checked)}
                tooltip={entry.tooltip}
              />
            ))}
            {EXTRACT_TOGGLES.map((entry) => (
              <ToggleRow
                checked={Boolean(instagramSyncOptions.extractImageFromVideo?.[entry.key])}
                key={`extract-${entry.key}`}
                label={entry.label}
                onChange={(checked) => updateExtractSection(onInstagramSyncOptionsChange, entry.key, checked)}
                tooltip={entry.tooltip}
              />
            ))}
          </SyncGroupCard>

        </div>

        <div className="source-editor-sync-column">
          <SyncGroupCard className="source-editor-sync-group-behavior" title="Behavior">
            {BEHAVIOR_TOGGLES.map((entry) => (
              <ToggleRow
                checked={Boolean(instagramSyncOptions[entry.key])}
                key={entry.key}
                label={entry.label}
                onChange={(checked) => updateInstagramOption(onInstagramSyncOptionsChange, entry.key, checked)}
                tooltip={entry.tooltip}
              />
            ))}
          </SyncGroupCard>

          <SyncGroupCard className="source-editor-sync-group-filtering" title="Filtering">
            <ToggleRow
              checked={Boolean(instagramSyncOptions.missingOnly)}
              label="Missing only"
              onChange={(checked) => updateInstagramOption(onInstagramSyncOptionsChange, 'missingOnly', checked)}
              tooltip="Only download files still missing on disk."
            />
            <ToggleRow
              checked={Boolean(instagramSyncOptions.fullScan)}
              label="Always full scan"
              onChange={(checked) => updateInstagramOption(onInstagramSyncOptionsChange, 'fullScan', checked)}
              tooltip="Disable incremental stop and re-walk the whole profile every sync. Slower; use when the profile re-exposes previously hidden media."
            />
            <LocalizedDateFieldRow
              label="Date from"
              localDateFormat={localDateFormat}
              onChange={(value) => updateInstagramOption(onInstagramSyncOptionsChange, 'dateFrom', value)}
              value={instagramSyncOptions.dateFrom ?? ''}
            />
            <LocalizedDateFieldRow
              label="Date to"
              localDateFormat={localDateFormat}
              onChange={(value) => updateInstagramOption(onInstagramSyncOptionsChange, 'dateTo', value)}
              value={instagramSyncOptions.dateTo ?? ''}
            />
          </SyncGroupCard>
        </div>

        <SyncGroupCard className="source-editor-sync-group-automation" title="Automation">
          <ToggleRow
            checked={Boolean(instagramSyncOptions.textSpecialFolder)}
            label="Text special folder"
            onChange={(checked) => updateInstagramOption(onInstagramSyncOptionsChange, 'textSpecialFolder', checked)}
            tooltip='Save downloaded post text into a "Text" subfolder instead of next to the media.'
          />
          <FieldRow
            label="Username override"
            onChange={(value) => updateInstagramOption(onInstagramSyncOptionsChange, 'usernameOverride', value)}
            tooltip="Override the username used in folder names."
            value={instagramSyncOptions.usernameOverride ?? ''}
          />
          <ToggleRow
            checked={Boolean(instagramSyncOptions.scriptEnabled)}
            label="Enable post-sync script"
            onChange={(checked) => updateInstagramOption(onInstagramSyncOptionsChange, 'scriptEnabled', checked)}
          />
          <FieldRow
            disabled={!instagramSyncOptions.scriptEnabled}
            label="Script"
            onChange={(value) => updateInstagramOption(onInstagramSyncOptionsChange, 'script', value)}
            value={instagramSyncOptions.script ?? ''}
          />
        </SyncGroupCard>

        <SyncGroupCard className="source-editor-sync-group-storage" title="Storage">
          <FieldRow
            label="Special path"
            onChange={(value) => updateInstagramOption(onInstagramSyncOptionsChange, 'specialPath', value)}
            tooltip="Media folder for this profile (set automatically by legacy imports). Absolute paths are used as-is; relative values resolve under the account media root. Leave empty to use the default folder."
            value={instagramSyncOptions.specialPath ?? ''}
          />
        </SyncGroupCard>
      </div>
    </div>
  )
}

interface TwitterSyncPanelProps {
  twitterSyncOptions: TwitterSyncUpsertOptions
  onTwitterSyncOptionsChange: (mutate: (current: TwitterSyncUpsertOptions) => TwitterSyncUpsertOptions) => void
}

function TwitterSyncPanel({ twitterSyncOptions, onTwitterSyncOptionsChange }: TwitterSyncPanelProps) {
  const sleepDisabled = (twitterSyncOptions.sleepTimerSecs ?? -1) < 0

  return (
    <div className="source-editor-sync-shell">
      <div className="source-editor-sync-groups">
        <div className="source-editor-sync-column">
          <SyncGroupCard className="source-editor-sync-group-sections" title="Download models">
            <ToggleRow
              checked={Boolean(twitterSyncOptions.mediaModel)}
              label="Media"
              onChange={(checked) => updateTwitterOption(onTwitterSyncOptionsChange, 'mediaModel', checked)}
              tooltip="Download the profile media tab (x.com/<user>/media). Excludes reposts of other users."
            />
            <ToggleRow
              checked={Boolean(twitterSyncOptions.profileModel)}
              label="Profile (timeline)"
              onChange={(checked) => updateTwitterOption(onTwitterSyncOptionsChange, 'profileModel', checked)}
            />
            <ToggleRow
              checked={Boolean(twitterSyncOptions.searchModel)}
              label="Search"
              onChange={(checked) => updateTwitterOption(onTwitterSyncOptionsChange, 'searchModel', checked)}
              tooltip="Download via search (from:<user>, including native retweets). More complete but more rate-limit prone."
            />
            <ToggleRow
              checked={Boolean(twitterSyncOptions.likesModel)}
              label="Likes"
              onChange={(checked) => updateTwitterOption(onTwitterSyncOptionsChange, 'likesModel', checked)}
            />
            <ToggleRow
              checked={Boolean(twitterSyncOptions.searchUseGraphqlEndpoint)}
              label="Search: new endpoint (graphql)"
              onChange={(checked) => updateTwitterOption(onTwitterSyncOptionsChange, 'searchUseGraphqlEndpoint', checked)}
              tooltip="Use -o search-endpoint=graphql for the search model (SCrawler's UseNewEndPointSearch)."
            />
            <ToggleRow
              checked={Boolean(twitterSyncOptions.profileUseGraphqlEndpoint)}
              label="Profile: new endpoint (graphql)"
              onChange={(checked) => updateTwitterOption(onTwitterSyncOptionsChange, 'profileUseGraphqlEndpoint', checked)}
              tooltip="Use -o search-endpoint=graphql for the media/profile models (SCrawler's UseNewEndPointProfiles)."
            />
            <ToggleRow
              checked={Boolean(twitterSyncOptions.allowNonUserTweets)}
              label="Media: allow non-user tweets"
              onChange={(checked) => updateTwitterOption(onTwitterSyncOptionsChange, 'allowNonUserTweets', checked)}
              tooltip="Allow reposts of other users in the media model (MediaModelAllowNonUserTweets)."
            />
          </SyncGroupCard>

          <SyncGroupCard className="source-editor-sync-group-media" title="Media">
            <ToggleRow
              checked={Boolean(twitterSyncOptions.downloadImages)}
              label="Download images"
              onChange={(checked) => updateTwitterOption(onTwitterSyncOptionsChange, 'downloadImages', checked)}
            />
            <ToggleRow
              checked={Boolean(twitterSyncOptions.downloadVideos)}
              label="Download videos"
              onChange={(checked) => updateTwitterOption(onTwitterSyncOptionsChange, 'downloadVideos', checked)}
            />
            <ToggleRow
              checked={Boolean(twitterSyncOptions.downloadGifs)}
              label="Download GIFs"
              onChange={(checked) => updateTwitterOption(onTwitterSyncOptionsChange, 'downloadGifs', checked)}
              tooltip="Download animated GIFs (saved as mp4)."
            />
            <ToggleRow
              checked={Boolean(twitterSyncOptions.separateVideoFolder)}
              label="Separate video folder"
              onChange={(checked) => updateTwitterOption(onTwitterSyncOptionsChange, 'separateVideoFolder', checked)}
              tooltip='Download videos into a "Video" subfolder (SCrawler layout).'
            />
            <FieldRow
              label="GIFs special folder"
              onChange={(value) => updateTwitterOption(onTwitterSyncOptionsChange, 'gifsSpecialFolder', value)}
              tooltip="Subfolder for GIFs (relative to the profile folder). Empty keeps them with the rest."
              value={twitterSyncOptions.gifsSpecialFolder ?? ''}
            />
            <FieldRow
              label="GIF prefix"
              onChange={(value) => updateTwitterOption(onTwitterSyncOptionsChange, 'gifsPrefix', value)}
              tooltip="Filename prefix applied to GIFs (default GIF_)."
              value={twitterSyncOptions.gifsPrefix ?? ''}
            />
            <ToggleRow
              checked={Boolean(twitterSyncOptions.useMd5Comparison)}
              label="Use MD5 comparison"
              onChange={(checked) => updateTwitterOption(onTwitterSyncOptionsChange, 'useMd5Comparison', checked)}
              tooltip="Discard byte-identical downloads by comparing content hashes."
            />
          </SyncGroupCard>
        </div>

        <div className="source-editor-sync-column">
          <SyncGroupCard className="source-editor-sync-group-behavior" title="Rate limit">
            <ToggleRow
              checked={Boolean(twitterSyncOptions.abortOnLimit)}
              label="Abort on limit"
              onChange={(checked) => updateTwitterOption(onTwitterSyncOptionsChange, 'abortOnLimit', checked)}
              tooltip="Stop remaining download models when Twitter's rate limit is reached."
            />
            <ToggleRow
              checked={Boolean(twitterSyncOptions.downloadAlreadyParsed)}
              label="Download already parsed"
              onChange={(checked) => updateTwitterOption(onTwitterSyncOptionsChange, 'downloadAlreadyParsed', checked)}
              tooltip="On rate limit, still download whatever was already parsed before aborting."
            />
            <NumberFieldRow
              label="Sleep timer (s)"
              onChange={(value) => updateTwitterOption(onTwitterSyncOptionsChange, 'sleepTimerSecs', value)}
              tooltip="Seconds to wait between download models. -1 disables (SCrawler default)."
              value={twitterSyncOptions.sleepTimerSecs ?? -1}
            />
            <NumberFieldRow
              disabled={sleepDisabled}
              label="Sleep before first (s)"
              onChange={(value) => updateTwitterOption(onTwitterSyncOptionsChange, 'sleepTimerBeforeFirstSecs', value)}
              tooltip="Seconds before the first request. -1 disables, -2 reuses the sleep timer value."
              value={twitterSyncOptions.sleepTimerBeforeFirstSecs ?? -2}
            />
          </SyncGroupCard>

          <SyncGroupCard className="source-editor-sync-group-automation" title="Storage">
            <FieldRow
              label="Special path"
              onChange={(value) => updateTwitterOption(onTwitterSyncOptionsChange, 'specialPath', value)}
              tooltip="Absolute folder for this profile's media. Leave empty to use the account/global media root."
              value={twitterSyncOptions.specialPath ?? ''}
            />
          </SyncGroupCard>
        </div>
      </div>
    </div>
  )
}

interface TikTokSyncPanelProps {
  tiktokSyncOptions: TikTokSyncUpsertOptions
  localDateFormat: LocalDateFormat
  onTikTokSyncOptionsChange: (mutate: (current: TikTokSyncUpsertOptions) => TikTokSyncUpsertOptions) => void
}

function TikTokSyncPanel({ tiktokSyncOptions, localDateFormat, onTikTokSyncOptionsChange }: TikTokSyncPanelProps) {
  return (
    <div className="source-editor-sync-shell">
      <div className="source-editor-sync-groups">
        <div className="source-editor-sync-column">
          <SyncGroupCard className="source-editor-sync-group-sections" title="Sections">
            <ToggleRow
              checked={Boolean(tiktokSyncOptions.getTimeline)}
              label="Timeline"
              onChange={(checked) => updateTikTokOption(onTikTokSyncOptionsChange, 'getTimeline', checked)}
            />
            <ToggleRow
              checked={Boolean(tiktokSyncOptions.getStoriesUser)}
              label="User Stories"
              onChange={(checked) => updateTikTokOption(onTikTokSyncOptionsChange, 'getStoriesUser', checked)}
              tooltip="Sync active user stories (saved to the Stories/ subfolder). Stories expire after 24h, so only active ones are fetched."
            />
            <ToggleRow
              checked={Boolean(tiktokSyncOptions.getReposts)}
              label="Reposts"
              onChange={(checked) => updateTikTokOption(onTikTokSyncOptionsChange, 'getReposts', checked)}
              tooltip="Sync reposts (saved to the Reposts/ subfolder)."
            />
            <ToggleRow
              checked={Boolean(tiktokSyncOptions.getLikedVideos)}
              label="Liked videos"
              onChange={(checked) => updateTikTokOption(onTikTokSyncOptionsChange, 'getLikedVideos', checked)}
              tooltip="Sync videos liked by the authenticated TikTok account. Files are saved to this profile's Liked/ subfolder."
            />
            <NumberFieldRow
              disabled={!tiktokSyncOptions.getLikedVideos}
              label="Liked videos limit"
              min={0}
              onChange={(value) => updateTikTokOption(onTikTokSyncOptionsChange, 'likedVideosLimit', Math.max(0, value))}
              tooltip="Maximum liked videos per sync. Use 0 to fetch every available liked video."
              value={tiktokSyncOptions.likedVideosLimit ?? 100}
            />
            <ToggleRow
              checked={tiktokSyncOptions.likedVideosIncremental !== false}
              disabled={!tiktokSyncOptions.getLikedVideos}
              label="Incremental liked videos"
              onChange={(checked) => updateTikTokOption(onTikTokSyncOptionsChange, 'likedVideosIncremental', checked)}
              tooltip="After one complete scan, stop when consecutive pages contain only downloaded media. Disable this to force a full rescan, including media that may have become visible again."
            />
            <NumberFieldRow
              disabled={!tiktokSyncOptions.getLikedVideos || tiktokSyncOptions.likedVideosIncremental === false}
              label="Known pages before stopping"
              min={1}
              onChange={(value) => updateTikTokOption(onTikTokSyncOptionsChange, 'likedVideosKnownPageThreshold', Math.max(1, value))}
              tooltip="Number of consecutive pages containing only existing downloads before an incremental scan stops."
              value={tiktokSyncOptions.likedVideosKnownPageThreshold ?? 3}
            />
          </SyncGroupCard>

          <SyncGroupCard className="source-editor-sync-group-media" title="Media">
            <ToggleRow
              checked={Boolean(tiktokSyncOptions.downloadVideos)}
              label="Download videos"
              onChange={(checked) => updateTikTokOption(onTikTokSyncOptionsChange, 'downloadVideos', checked)}
            />
            <ToggleRow
              checked={Boolean(tiktokSyncOptions.downloadPhotos)}
              label="Download photos"
              onChange={(checked) => updateTikTokOption(onTikTokSyncOptionsChange, 'downloadPhotos', checked)}
            />
            <ToggleRow
              checked={Boolean(tiktokSyncOptions.separateVideoFolder)}
              label="Separate video folder"
              onChange={(checked) => updateTikTokOption(onTikTokSyncOptionsChange, 'separateVideoFolder', checked)}
              tooltip='Download videos into a "Video" subfolder.'
            />
            <ToggleRow
              checked={Boolean(tiktokSyncOptions.useParsedVideoDate)}
              label="Use video date as file date"
              onChange={(checked) => updateTikTokOption(onTikTokSyncOptionsChange, 'useParsedVideoDate', checked)}
              tooltip="Set the file modified date to the post date (yt-dlp --mtime)."
            />
          </SyncGroupCard>

          <SyncGroupCard className="source-editor-sync-group-stats" title="Stats">
            <ToggleRow
              checked={tiktokSyncOptions.collectMediaStats !== false}
              label="Collect media stats"
              onChange={(checked) => updateTikTokOption(onTikTokSyncOptionsChange, 'collectMediaStats', checked)}
              tooltip="Store views, likes, comments, and shares reported by TikTok for newly downloaded media. To re-collect stats for media you already have, use 'Refresh stats' in the Profile View header."
            />
          </SyncGroupCard>

          <SyncGroupCard className="source-editor-sync-group-naming" title="Video naming">
            <ToggleRow
              checked={Boolean(tiktokSyncOptions.tokkitFileNaming)}
              label="4K Tokkit filename style"
              onChange={(checked) => updateTikTokOption(onTikTokSyncOptionsChange, 'tokkitFileNaming', checked)}
              tooltip="Name files like 4K Tokkit (handle_unixtime_postid), without the date prefix. Overrides native title. Use to stay consistent with imported media."
            />
            <ToggleRow
              checked={Boolean(tiktokSyncOptions.useNativeTitle)}
              disabled={Boolean(tiktokSyncOptions.tokkitFileNaming)}
              label="Use native title"
              onChange={(checked) => updateTikTokOption(onTikTokSyncOptionsChange, 'useNativeTitle', checked)}
              tooltip="Name video files using the post's native caption instead of just the id."
            />
            <ToggleRow
              checked={Boolean(tiktokSyncOptions.addVideoIdToTitle)}
              disabled={!tiktokSyncOptions.useNativeTitle}
              label="Add video id to title"
              onChange={(checked) => updateTikTokOption(onTikTokSyncOptionsChange, 'addVideoIdToTitle', checked)}
              tooltip="Append the video id to the title (keeps names unique). Applies when 'Use native title' is on."
            />
            <ToggleRow
              checked={Boolean(tiktokSyncOptions.removeTagsFromTitle)}
              disabled={!tiktokSyncOptions.useNativeTitle}
              label="Remove hashtags from title"
              onChange={(checked) => updateTikTokOption(onTikTokSyncOptionsChange, 'removeTagsFromTitle', checked)}
            />
          </SyncGroupCard>
        </div>

        <div className="source-editor-sync-column">
          <SyncGroupCard className="source-editor-sync-group-behavior" title="Rate limit">
            <ToggleRow
              checked={Boolean(tiktokSyncOptions.abortOnLimit)}
              label="Abort on limit"
              onChange={(checked) => updateTikTokOption(onTikTokSyncOptionsChange, 'abortOnLimit', checked)}
              tooltip="Stop remaining posts when TikTok's rate limit is reached."
            />
            <NumberFieldRow
              label="Sleep timer (s)"
              onChange={(value) => updateTikTokOption(onTikTokSyncOptionsChange, 'sleepTimerSecs', value)}
              tooltip="Seconds to wait between download batches. -1 disables."
              value={tiktokSyncOptions.sleepTimerSecs ?? -1}
            />
          </SyncGroupCard>

          <SyncGroupCard className="source-editor-sync-group-naming" title="Date range">
            <EpochDateFieldRow
              label="Download from"
              localDateFormat={localDateFormat}
              onChange={(epoch) => updateTikTokOption(onTikTokSyncOptionsChange, 'downloadFromDate', epoch)}
              value={tiktokSyncOptions.downloadFromDate}
            />
            <EpochDateFieldRow
              label="Download until"
              localDateFormat={localDateFormat}
              onChange={(epoch) => updateTikTokOption(onTikTokSyncOptionsChange, 'downloadToDate', epoch)}
              value={tiktokSyncOptions.downloadToDate}
            />
          </SyncGroupCard>

          <SyncGroupCard className="source-editor-sync-group-automation" title="Storage">
            <FieldRow
              label="Special path"
              onChange={(value) => updateTikTokOption(onTikTokSyncOptionsChange, 'specialPath', value)}
              tooltip="Absolute folder for this profile's media. Leave empty to use the account/global media root."
              value={tiktokSyncOptions.specialPath ?? ''}
            />
          </SyncGroupCard>
        </div>
      </div>
    </div>
  )
}

interface SyncGroupCardProps {
  className?: string
  title: string
  children: ReactNode
}

function SyncGroupCard({ className, title, children }: SyncGroupCardProps) {
  return (
    <section className={className ? `source-editor-sync-group-card ${className}` : 'source-editor-sync-group-card'}>
      <header className="source-editor-sync-group-header">
        <strong>{title}</strong>
      </header>
      <div className="source-editor-setting-list">{children}</div>
    </section>
  )
}

interface ToggleRowProps {
  label: string
  tooltip?: string
  checked: boolean
  disabled?: boolean
  onChange: (checked: boolean) => void
}

function ToggleRow({ label, tooltip, checked, disabled = false, onChange }: ToggleRowProps) {
  const inputId = useId()

  return (
    <div className={disabled ? 'source-editor-setting-row source-editor-setting-row-disabled' : 'source-editor-setting-row'}>
      <div className="source-editor-setting-copy">
        <label htmlFor={inputId}>{label}</label>
        <HelpTip label={label} tooltip={tooltip} />
      </div>
      <input
        aria-label={label}
        checked={checked}
        disabled={disabled}
        id={inputId}
        onChange={(event) => onChange(event.target.checked)}
        type="checkbox"
      />
    </div>
  )
}

interface FieldRowProps {
  label: string
  tooltip?: string
  value: string
  disabled?: boolean
  onChange: (value: string) => void
}

function FieldRow({
  label,
  tooltip,
  value,
  disabled = false,
  onChange,
}: FieldRowProps) {
  const inputId = useId()

  return (
    <div className={disabled ? 'source-editor-setting-row source-editor-setting-row-disabled' : 'source-editor-setting-row'}>
      <div className="source-editor-setting-copy">
        <label htmlFor={inputId}>{label}</label>
        <HelpTip label={label} tooltip={tooltip} />
      </div>
      <input aria-label={label} disabled={disabled} id={inputId} onChange={(event) => onChange(event.target.value)} type="text" value={value} />
    </div>
  )
}

interface EpochDateFieldRowProps {
  label: string
  tooltip?: string
  value?: number
  localDateFormat: LocalDateFormat
  onChange: (epochSeconds: number | undefined) => void
}

// Converte entre o <input type="date"> (YYYY-MM-DD, UTC) e unix seconds, que é
// como o range fica persistido (espelho do 4K Tokkit).
function epochToDateInput(value?: number): string {
  if (!value || value <= 0) {
    return ''
  }
  return new Date(value * 1000).toISOString().slice(0, 10)
}

function EpochDateFieldRow({
  label,
  tooltip,
  value,
  localDateFormat,
  onChange,
}: EpochDateFieldRowProps) {
  return (
    <LocalizedDateFieldRow
      label={label}
      localDateFormat={localDateFormat}
      onChange={(isoDate) => {
        if (!isoDate) {
          onChange(undefined)
          return
        }
        const epoch = Math.floor(Date.parse(`${isoDate}T00:00:00Z`) / 1000)
        onChange(Number.isFinite(epoch) ? epoch : undefined)
      }}
      tooltip={tooltip}
      value={epochToDateInput(value)}
    />
  )
}

interface NumberFieldRowProps {
  label: string
  tooltip?: string
  value: number
  disabled?: boolean
  min?: number
  onChange: (value: number) => void
}

function NumberFieldRow({ label, tooltip, value, disabled = false, min, onChange }: NumberFieldRowProps) {
  const inputId = useId()

  return (
    <div className={disabled ? 'source-editor-setting-row source-editor-setting-row-disabled' : 'source-editor-setting-row'}>
      <div className="source-editor-setting-copy">
        <label htmlFor={inputId}>{label}</label>
        <HelpTip label={label} tooltip={tooltip} />
      </div>
      <input
        aria-label={label}
        disabled={disabled}
        id={inputId}
        min={min}
        onChange={(event) => {
          const parsed = Number.parseInt(event.target.value, 10)
          onChange(Number.isNaN(parsed) ? -1 : parsed)
        }}
        type="number"
        value={value}
      />
    </div>
  )
}

interface LocalizedDateFieldRowProps {
  label: string
  tooltip?: string
  value: string
  disabled?: boolean
  localDateFormat: LocalDateFormat
  onChange: (value: string) => void
}

function LocalizedDateFieldRow({
  label,
  tooltip,
  value,
  disabled = false,
  localDateFormat,
  onChange,
}: LocalizedDateFieldRowProps) {
  const inputId = useId()
  const [inputValue, setInputValue] = useState(() => formatIsoDateForLocale(value, localDateFormat))
  const [isOpen, setIsOpen] = useState(false)
  const [calendarMonth, setCalendarMonth] = useState(() => calendarMonthFromValue(value))
  const fieldRef = useRef<HTMLDivElement | null>(null)
  const calendarDaysForMonth = useMemo(() => calendarDays(calendarMonth), [calendarMonth])

  useEffect(() => {
    setInputValue(formatIsoDateForLocale(value, localDateFormat))
  }, [localDateFormat, value])

  useEffect(() => {
    if (!isOpen) {
      return undefined
    }

    function handlePointerDown(event: PointerEvent) {
      if (fieldRef.current?.contains(event.target as Node)) {
        return
      }
      setIsOpen(false)
    }

    window.addEventListener('pointerdown', handlePointerDown)
    return () => window.removeEventListener('pointerdown', handlePointerDown)
  }, [isOpen])

  function openCalendar() {
    if (disabled) {
      return
    }
    setCalendarMonth(calendarMonthFromValue(value))
    setIsOpen(true)
  }

  function normalizeInput(nextRawValue: string) {
    const parsed = parseLocalizedDateInput(nextRawValue, localDateFormat)
    const normalized = formatIsoDateForLocale(parsed, localDateFormat)
    setInputValue(normalized)
    onChange(parsed ?? '')
  }

  function selectDate(nextDate: Date) {
    const isoValue = isoDateFromCalendarDate(nextDate)
    setInputValue(formatIsoDateForLocale(isoValue, localDateFormat))
    onChange(isoValue)
    setCalendarMonth(new Date(nextDate.getFullYear(), nextDate.getMonth(), 1))
    setIsOpen(false)
  }

  return (
    <div className={disabled ? 'source-editor-setting-row source-editor-setting-row-disabled source-editor-setting-row-field' : 'source-editor-setting-row source-editor-setting-row-field'}>
      <div className="source-editor-setting-copy">
        <label htmlFor={inputId}>{label}</label>
        <HelpTip label={label} tooltip={tooltip} />
      </div>
      <div className="source-editor-setting-input plans-date-picker-field" ref={fieldRef}>
        <div className="plans-date-picker-control">
          <input
            aria-label={label}
            disabled={disabled}
            id={inputId}
            onBlur={(event) => normalizeInput(event.target.value)}
            onChange={(event) => setInputValue(event.target.value)}
            placeholder={localDateFormat.placeholder}
            type="text"
            value={inputValue}
          />
          <button
            aria-expanded={isOpen}
            aria-label={`Pick ${label}`}
            className="ghost-button plans-date-picker-button"
            disabled={disabled}
            onClick={openCalendar}
            type="button"
          >
            <CalendarIcon />
          </button>
        </div>
        {isOpen ? (
          <div aria-label={`${label} calendar`} className="plans-date-picker-popover" role="dialog">
            <div className="plans-date-picker-header">
              <button aria-label="Previous month" className="ghost-button plans-date-picker-nav" onClick={() => setCalendarMonth((current) => shiftMonth(current, -1))} type="button">‹</button>
              <strong>{monthLabel(calendarMonth)}</strong>
              <button aria-label="Next month" className="ghost-button plans-date-picker-nav" onClick={() => setCalendarMonth((current) => shiftMonth(current, 1))} type="button">›</button>
            </div>
            <div className="plans-date-picker-weekdays">
              {['S', 'T', 'Q', 'Q', 'S', 'S', 'D'].map((weekday, index) => <span key={`${label}-weekday-${index}`}>{weekday}</span>)}
            </div>
            <div className="plans-date-picker-grid">
              {calendarDaysForMonth.map((day) => {
                const isoValue = isoDateFromCalendarDate(day)
                const isCurrentMonth = day.getMonth() === calendarMonth.getMonth()
                const isSelected = value === isoValue
                const isToday = isoValue === isoDateFromCalendarDate(new Date())
                return (
                  <button
                    className={`plans-date-picker-day${isCurrentMonth ? '' : ' is-outside'}${isSelected ? ' is-selected' : ''}${isToday ? ' is-today' : ''}`}
                    key={`${label}-${isoValue}`}
                    onClick={() => selectDate(day)}
                    type="button"
                  >
                    {day.getDate()}
                  </button>
                )
              })}
            </div>
          </div>
        ) : null}
      </div>
    </div>
  )
}


function detectLocalDateFormat(): LocalDateFormat {
  return localDateFormatFromPattern(Intl.DateTimeFormat().resolvedOptions().locale.toLowerCase().startsWith('pt')
    ? 'dd/MM/yyyy'
    : 'MM/dd/yyyy')
}

function localDateFormatFromPattern(pattern: string): LocalDateFormat {
  const normalizedPattern = pattern.trim() || 'yyyy-MM-dd'
  const tokens = normalizedPattern.match(/d+|M+|y+/gi) ?? []
  const order = tokens
    .map((token) => {
      const lowerToken = token.toLowerCase()
      if (lowerToken.startsWith('d')) return 'day'
      if (lowerToken.startsWith('m')) return 'month'
      if (lowerToken.startsWith('y')) return 'year'
      return null
    })
    .filter((token): token is 'day' | 'month' | 'year' => token !== null)

  return {
    pattern: normalizedPattern,
    order: order.length === 3 ? order : ['day', 'month', 'year'],
    placeholder: normalizedPattern
      .replace(/d+/gi, 'dd')
      .replace(/m+/gi, 'mm')
      .replace(/y+/gi, 'aaaa'),
  }
}

function formatIsoDateForLocale(value: string | undefined, format: LocalDateFormat): string {
  if (!value) {
    return ''
  }

  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value)
  if (!match) {
    return value
  }

  const [, year, month, day] = match
  const parts = { day, month, year }
  return format.pattern.replace(/d+|M+|y+/gi, (token) => {
    const lowerToken = token.toLowerCase()
    if (lowerToken.startsWith('d')) {
      return token.length === 1 ? String(Number(parts.day)) : parts.day
    }
    if (lowerToken.startsWith('m')) {
      return token.length === 1 ? String(Number(parts.month)) : parts.month
    }
    if (lowerToken.startsWith('y')) {
      return token.length <= 2 ? parts.year.slice(-2) : parts.year
    }
    return token
  })
}

function parseLocalizedDateInput(rawValue: string, format: LocalDateFormat): string | undefined {
  const normalized = rawValue.trim()
  if (!normalized) {
    return undefined
  }

  const chunks = normalized.split(/[.\-/\s]+/).filter(Boolean)
  if (chunks.length !== 3) {
    return undefined
  }

  const dateParts = { day: 0, month: 0, year: 0 }
  for (const [index, partType] of format.order.entries()) {
    dateParts[partType] = Number(chunks[index])
  }

  if (!Number.isInteger(dateParts.day) || !Number.isInteger(dateParts.month) || !Number.isInteger(dateParts.year)) {
    return undefined
  }

  if (dateParts.year < 100) {
    dateParts.year += 2000
  }

  const candidate = new Date(dateParts.year, dateParts.month - 1, dateParts.day)
  if (
    candidate.getFullYear() !== dateParts.year
    || candidate.getMonth() !== dateParts.month - 1
    || candidate.getDate() !== dateParts.day
  ) {
    return undefined
  }

  return `${String(dateParts.year).padStart(4, '0')}-${String(dateParts.month).padStart(2, '0')}-${String(dateParts.day).padStart(2, '0')}`
}

function calendarMonthFromValue(value?: string): Date {
  if (value) {
    const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value)
    if (match) {
      return new Date(Number(match[1]), Number(match[2]) - 1, 1)
    }
  }
  const now = new Date()
  return new Date(now.getFullYear(), now.getMonth(), 1)
}

function shiftMonth(month: Date, delta: number): Date {
  return new Date(month.getFullYear(), month.getMonth() + delta, 1)
}

function calendarDays(month: Date): Date[] {
  const firstDayOfMonth = new Date(month.getFullYear(), month.getMonth(), 1)
  const weekday = firstDayOfMonth.getDay()
  const start = new Date(firstDayOfMonth)
  start.setDate(firstDayOfMonth.getDate() - weekday)

  return Array.from({ length: 42 }, (_, index) => {
    const next = new Date(start)
    next.setDate(start.getDate() + index)
    return next
  })
}

function isoDateFromCalendarDate(value: Date): string {
  return `${value.getFullYear()}-${String(value.getMonth() + 1).padStart(2, '0')}-${String(value.getDate()).padStart(2, '0')}`
}

function monthLabel(value: Date): string {
  return value.toLocaleString(undefined, { month: 'long', year: 'numeric' })
}

function CalendarIcon() {
  return (
    <svg aria-hidden="true" className="plans-date-picker-icon" viewBox="0 0 24 24">
      <path d="M7 2a1 1 0 0 1 1 1v1h8V3a1 1 0 1 1 2 0v1h1.5A2.5 2.5 0 0 1 22 6.5v13a2.5 2.5 0 0 1-2.5 2.5h-15A2.5 2.5 0 0 1 2 19.5v-13A2.5 2.5 0 0 1 4.5 4H6V3a1 1 0 0 1 1-1Zm12.5 8h-15a.5.5 0 0 0-.5.5v9a.5.5 0 0 0 .5.5h15a.5.5 0 0 0 .5-.5v-9a.5.5 0 0 0-.5-.5ZM6 6H4.5a.5.5 0 0 0-.5.5V8h16V6.5a.5.5 0 0 0-.5-.5H18v1a1 1 0 1 1-2 0V6H8v1a1 1 0 1 1-2 0V6Z" />
    </svg>
  )
}

function updateInstagramOption<K extends keyof InstagramSyncUpsertOptions>(
  mutate: (mutator: (current: InstagramSyncUpsertOptions) => InstagramSyncUpsertOptions) => void,
  key: K,
  value: InstagramSyncUpsertOptions[K],
) {
  mutate((current) => ({
    ...current,
    [key]: value,
  } as InstagramSyncUpsertOptions))
}

function updateTwitterOption<K extends keyof TwitterSyncUpsertOptions>(
  mutate: (mutator: (current: TwitterSyncUpsertOptions) => TwitterSyncUpsertOptions) => void,
  key: K,
  value: TwitterSyncUpsertOptions[K],
) {
  mutate((current) => ({
    ...current,
    [key]: value,
  } as TwitterSyncUpsertOptions))
}

function updateTikTokOption<K extends keyof TikTokSyncUpsertOptions>(
  mutate: (mutator: (current: TikTokSyncUpsertOptions) => TikTokSyncUpsertOptions) => void,
  key: K,
  value: TikTokSyncUpsertOptions[K],
) {
  mutate((current) => ({
    ...current,
    [key]: value,
  } as TikTokSyncUpsertOptions))
}

function updateExtractSection(
  mutate: (mutator: (current: InstagramSyncUpsertOptions) => InstagramSyncUpsertOptions) => void,
  section: 'timeline' | 'reels' | 'stories' | 'storiesUser' | 'tagged',
  value: boolean,
) {
  mutate((current) => ({
    ...current,
    extractImageFromVideo: {
      timeline: current.extractImageFromVideo?.timeline ?? true,
      reels: current.extractImageFromVideo?.reels ?? true,
      stories: current.extractImageFromVideo?.stories ?? true,
      storiesUser: current.extractImageFromVideo?.storiesUser ?? true,
      tagged: current.extractImageFromVideo?.tagged ?? true,
      [section]: value,
    },
  }))
}
