export type ProviderKey = 'instagram' | 'tiktok' | 'twitter'
export type AuthMode = 'imported_session'
export type AuthState = 'ready' | 'degraded' | 'expired'
export type SourceKind = 'profile'
export type SourceProfileDeleteMode = 'user_only' | 'with_media'
export type SyncMode = 'automatic' | 'manual'
export type NotificationMode = 'summary' | 'detailed'
export type PlanRunStatus = 'idle' | 'succeeded' | 'failed' | 'skipped'
export type NotificationLevel = 'info' | 'warning' | 'error'
export type SchedulerRemoteState = 'exists' | 'suspended' | 'deleted'
export type SchedulerPauseMode =
  | 'disabled'
  | 'unlimited'
  | '1h'
  | '2h'
  | '3h'
  | '4h'
  | '6h'
  | '12h'
  | 'until'
export type SchedulerSkipMode = 'default' | 'minutes' | 'until' | 'reset'
export type ConnectorRuntimeManagementMode = 'managed' | 'custom'
export type InstagramPresetSlot = 'preset1' | 'preset2'
export type ConnectorRuntimeStatusKind =
  | 'up_to_date'
  | 'update_available'
  | 'checking'
  | 'downloading'
  | 'pending_activation'
  | 'custom_override'
  | 'error'

export interface AccountsWindowIntent {
  initialAccountId?: string
  initialProvider?: ProviderKey
  initialMode?: 'create' | 'edit'
}

export interface SourceEditorSeedIntent {
  provider: ProviderKey
  handle: string
  displayName: string
}

export interface SourceEditorWindowIntent {
  sourceId?: string
  preferredProvider?: ProviderKey
  preferredAccountId?: string
  seed?: SourceEditorSeedIntent
}

export type ProfileEditorSeedIntent = SourceEditorSeedIntent
export type ProfileEditorWindowIntent = SourceEditorWindowIntent

export interface PlanEditorWindowIntent {
  mode: 'new' | 'edit' | 'clone'
  planId?: string
  schedulerSetId?: string
}

export interface InstagramSourceSyncSections {
  timeline: boolean
  reels: boolean
  stories: boolean
  storiesUser: boolean
  tagged: boolean
}

export interface InstagramExtractImageFromVideoSections {
  timeline: boolean
  reels: boolean
  stories: boolean
  storiesUser: boolean
  tagged: boolean
}

export interface InstagramSourceSyncPreset {
  enabled: boolean
  label: string
  sections: InstagramSourceSyncSections
}

export interface InstagramSourceSyncOptions {
  timeline: InstagramSourceSyncSections['timeline']
  reels: InstagramSourceSyncSections['reels']
  stories: InstagramSourceSyncSections['stories']
  storiesUser: InstagramSourceSyncSections['storiesUser']
  tagged: InstagramSourceSyncSections['tagged']
  temporary?: boolean
  favorite?: boolean
  downloadImages?: boolean
  downloadVideos?: boolean
  getUserMediaOnly?: boolean
  missingOnly?: boolean
  dateFrom?: string
  dateTo?: string
  verifiedProfile?: boolean
  forceUpdateUserName?: boolean
  forceUpdateUserInformation?: boolean
  extractImageFromVideo?: InstagramExtractImageFromVideoSections
  placeExtractedImageIntoVideoFolder?: boolean
  downloadText?: boolean
  downloadTextPosts?: boolean
  targetStoryMediaId?: string
  textSpecialFolder?: boolean
  specialPath?: string
  usernameOverride?: string
  scriptEnabled?: boolean
  script?: string
  description?: string
  color?: string
  userIdHint?: string
  previousHandles?: string[]
}

export interface TwitterSourceSyncOptions {
  mediaModel?: boolean
  profileModel?: boolean
  searchModel?: boolean
  likesModel?: boolean
  searchUseGraphqlEndpoint?: boolean
  profileUseGraphqlEndpoint?: boolean
  allowNonUserTweets?: boolean
  abortOnLimit?: boolean
  downloadAlreadyParsed?: boolean
  sleepTimerSecs?: number
  sleepTimerBeforeFirstSecs?: number
  downloadImages?: boolean
  downloadVideos?: boolean
  downloadGifs?: boolean
  separateVideoFolder?: boolean
  gifsSpecialFolder?: string
  gifsPrefix?: string
  useMd5Comparison?: boolean
  temporary?: boolean
  specialPath?: string
  description?: string
  color?: string
  userIdHint?: string
}

export interface TikTokSourceSyncOptions {
  getTimeline?: boolean
  getStoriesUser?: boolean
  getReposts?: boolean
  getLikedVideos?: boolean
  likedVideosLimit?: number
  likedVideosIncremental?: boolean
  likedVideosKnownPageThreshold?: number
  collectMediaStats?: boolean
  refreshExistingMediaStats?: boolean
  downloadVideos?: boolean
  downloadPhotos?: boolean
  useNativeTitle?: boolean
  addVideoIdToTitle?: boolean
  removeTagsFromTitle?: boolean
  tokkitFileNaming?: boolean
  useParsedVideoDate?: boolean
  downloadFromDate?: number
  downloadToDate?: number
  separateVideoFolder?: boolean
  abortOnLimit?: boolean
  sleepTimerSecs?: number
  temporary?: boolean
  specialPath?: string
  description?: string
  color?: string
  userIdHint?: string
}

export interface SourceSyncOptions {
  instagram?: InstagramSourceSyncOptions
  twitter?: TwitterSourceSyncOptions
  tiktok?: TikTokSourceSyncOptions
}

export interface RunSourceSyncOptions {
  trigger?: string
  runMode?: 'force_imported_backfill' | 'refresh_media_stats'
  syncOptionsOverride?: SourceSyncOptions
}

export interface SourceAvailabilityCheckItem {
  sourceId: string
  provider: string
  previousHandle: string
  currentHandle?: string
  status: 'unchanged' | 'updated_handle' | 'marked_problem' | 'skipped' | 'failed'
  message: string
}

export interface SourceAvailabilityCheckResult {
  snapshot: WorkspaceSnapshot
  requested: number
  processed: number
  unchanged: number
  updatedHandle: number
  markedProblem: number
  skipped: number
  failed: number
  items: SourceAvailabilityCheckItem[]
}

export interface ProviderDescriptor {
  key: ProviderKey
  displayName: string
  authModes: AuthMode[]
  sourceKinds: SourceKind[]
  supportsMultipleAccounts: boolean
  defaultCapabilities: string[]
  notes: string
}

export interface AppSetting {
  key: string
  value: string
  category: string
  description?: string
  mutable: boolean
}

export interface ConnectorRuntimeStatus {
  key: string
  displayName: string
  managementMode: ConnectorRuntimeManagementMode
  activeVersion: string
  bundledVersion: string
  latestVersion?: string
  updateAvailable: boolean
  status: ConnectorRuntimeStatusKind
  lastCheckedAt?: string
  lastError?: string
  pendingVersion?: string
  progressPercent?: number
  progressDetail?: string
  customPath?: string
}

export interface ImportProviderDescriptor {
  key: ProviderKey
  displayName: string
  description: string
}

export interface ImportMethodDescriptor {
  importerId: string
  provider: ProviderKey
  label: string
  description: string
}

export type ImportRootSource = 'default' | 'account' | 'manual'

export interface ImportRootDescriptor {
  path: string
  source: ImportRootSource
  label: string
  removable: boolean
}

export type ImportProblemSeverity = 'warning' | 'error'
export type ImportPreviewState =
  | 'ready'
  | 'already_imported'
  | 'needs_account_link'
  | 'duplicate_conflict'
  | 'no_media'

export interface ImportProblem {
  severity: ImportProblemSeverity
  code: string
  message: string
}

export interface ImportPreviewProfile {
  profileRoot: string
  userXmlPath: string
  handle: string
  displayName: string
  accountName?: string
  sourceId?: string
  sourceDisplayName?: string
  sourceHandle?: string
  accountId?: string
  accountDisplayName?: string
  avatarPath?: string
  alreadyImported: boolean
  importState: ImportPreviewState
  fileCount: number
  alreadyCatalogedCount: number
  newFileCount: number
  problems: ImportProblem[]
}

export interface ImportPreviewSummary {
  detectedProfiles: number
  readyProfiles: number
  blockedProfiles: number
  alreadyImportedProfiles: number
  importableFiles: number
}

export interface ImportPreview {
  importerId: string
  provider: ProviderKey
  methodLabel: string
  forceReimport: boolean
  roots: string[]
  profiles: ImportPreviewProfile[]
  summary: ImportPreviewSummary
}

export interface ImportPreviewOptions {
  forceReimport: boolean
  manualRoots: string[]
  disabledRoots: string[]
}

export type ImportResolutionAction = 'import' | 'skip'

export interface ImportResolution {
  profileRoot: string
  action: ImportResolutionAction
  accountId?: string
}

export interface ImportRunRequest {
  forceReimport: boolean
  manualRoots: string[]
  disabledRoots: string[]
  resolutions: ImportResolution[]
}

export type ImportRunStatus = 'imported' | 'skipped' | 'failed'

export interface ImportRunProfileResult {
  profileRoot: string
  handle: string
  status: ImportRunStatus
  sourceId?: string
  importedMediaCount: number
  alreadyCatalogedCount: number
  message: string
}

export interface ImportRunResult {
  importerId: string
  importedProfiles: number
  skippedProfiles: number
  failedProfiles: number
  importedMediaCount: number
  alreadyCatalogedCount: number
  profiles: ImportRunProfileResult[]
}

export interface InstagramNamingLedgerBackfillResult {
  scannedSources: number
  scannedProfiles: number
  scannedFiles: number
  insertedEntries: number
  updatedEntries: number
  skippedFiles: number
  legacyRecordsTotal: number
  legacyRecordsMatched: number
  legacyRecordsMissingFiles: number
  backfilledAt: string
}

export type ImportQueueJobKind = 'preview' | 'import' | 'backfill'

export interface ImportQueueJob {
  jobId: string
  importerId: string
  provider: ProviderKey
  methodLabel: string
  jobKind: ImportQueueJobKind
  queuedAt: string
  startedAt?: string
  progressPercent?: number
  progressLabel?: string
  progressDetail?: string
  progressIndeterminate?: boolean
}

export interface ImportQueueRecentResult {
  jobId: string
  importerId: string
  provider: ProviderKey
  methodLabel: string
  jobKind: ImportQueueJobKind
  status: 'succeeded' | 'failed'
  summary: string
  finishedAt: string
  error?: string
}

export interface ImportQueueStatus {
  queuedCount: number
  runningCount: number
  completedCount: number
  failedCount: number
  totalCount: number
  activeJobId?: string
  activeImporterId?: string
  activeProvider?: ProviderKey
  activeMethodLabel?: string
  activeJobKind?: ImportQueueJobKind
  activeStartedAt?: string
  queuedItems: ImportQueueJob[]
  runningItems: ImportQueueJob[]
  recentResults: ImportQueueRecentResult[]
  latestPreview?: ImportPreview
  latestRunResult?: ImportRunResult
  latestBackfillResult?: InstagramNamingLedgerBackfillResult
  updatedAt: string
}

export interface DesktopRuntimeState {
  closeToTray: boolean
  silentMode: boolean
  trayAvailable: boolean
  reportedByBackend?: boolean
}

export interface ProviderAccount {
  id: string
  provider: ProviderKey
  displayName: string
  authMode: AuthMode
  authState: AuthState
  capabilities: string[]
  lastValidatedAt: string
}

export interface ProviderAccountSession {
  accountId: string
  authMode: AuthMode
  sessionFormat: string
  fingerprint: string
  cookieCount: number
  importedAt: string
  lastValidatedAt?: string
  lastValidationError?: string
  hasSecret: boolean
}

export interface ProviderAccountCookie {
  domain: string
  name: string
  value: string
  path: string
  expiresAt?: string
  secure: boolean
  httpOnly: boolean
}

export type ProviderAccountSettingValueKind = 'string' | 'json'

export interface ProviderAccountSettingValue {
  settingKey: string
  valueKind: ProviderAccountSettingValueKind
  stringValue?: string
  jsonValue?: unknown
}

export interface ProviderAccountImportState {
  accountId: string
  providerUserId?: string
  providerUsername?: string
  lastImportedAt: string
  canRevert: boolean
  backupImportedAt?: string
}

export interface ProviderAccountEditor {
  account: ProviderAccount
  session?: ProviderAccountSession | null
  settings: ProviderAccountSettingValue[]
  importState?: ProviderAccountImportState | null
}

export interface RuntimeLogEntry {
  id: string
  timestamp: string
  scope: string
  level: NotificationLevel | 'debug'
  accountId?: string
  provider?: ProviderKey
  sourceId?: string
  sourceHandle?: string
  message: string
  detail?: string
}

export type ConnectorDebugEventType =
  | 'call'
  | 'stdout'
  | 'stderr'
  | 'response'
  | 'error'
  | 'system'

export interface ConnectorDebugEntry {
  id: string
  timestamp: string
  sourceId?: string
  provider?: ProviderKey
  sourceHandle?: string
  connector: string
  eventType: ConnectorDebugEventType
  operation: string
  raw: string
}

export interface ConnectorDebugQuery {
  limit?: number
  provider?: ProviderKey
  sourceId?: string
  eventType?: ConnectorDebugEventType
}

export interface RuntimeLogContext {
  providerCatalog: ProviderDescriptor[]
  accounts: ProviderAccount[]
}

export interface MediaGalleryFile {
  relativePath: string
  absolutePath: string
  mediaType: string
}

export interface MediaGalleryPost {
  postId?: string
  postUrl?: string
  capturedAt?: number
  /** When the media was first downloaded/seen by the app (unix seconds). */
  downloadedAt?: number
  /** Original author — only set for TikTok Likes (used for author search). */
  author?: string
  mediaType: 'video' | 'image' | 'slideshow'
  section: string
  /**
   * Highlight albums this post belongs to (the `Stories/<album>/` subfolder
   * and/or membership of a highlight whose media lives in another folder).
   */
  albums?: string[]
  posterPath?: string
  viewCount?: number
  likeCount?: number
  commentCount?: number
  shareCount?: number
  statsUpdatedAt?: string
  files: MediaGalleryFile[]
}

export interface SourceMediaGallery {
    sourceId: string
    provider: ProviderKey
    handle: string
    profileUrl: string
    posts: MediaGalleryPost[]
}

export interface MediaThumbnailQueueItem {
  sourceId: string
  provider: ProviderKey
  handle: string
  state: 'queued' | 'running'
  queuedAt: string
  startedAt?: string
  filesScanned: number
  filesTotal: number
  filesProcessed: number
  generated: number
  skippedExisting: number
  failed: number
  currentFile?: string
  progressPercent?: number
}

export interface MediaThumbnailQueueResult {
  sourceId: string
  provider: ProviderKey
  handle: string
  status: 'succeeded' | 'failed'
  summary: string
  generated: number
  skippedExisting: number
  failed: number
  finishedAt: string
}

export interface MediaThumbnailQueueStatus {
  queuedCount: number
  runningCount: number
  completedCount: number
  failedCount: number
  active?: MediaThumbnailQueueItem
  queuedItems: MediaThumbnailQueueItem[]
  recentResults: MediaThumbnailQueueResult[]
  updatedAt: string
}

export interface SingleVideo {
  id: string
  provider: string
  sourceUrl: string
  providerVideoId?: string
  uploader?: string
  title?: string
  relativePath: string
  absolutePath: string
  mediaType: string
  capturedAt?: number
  downloadedAt: string
  files: SingleVideoFile[]
  audioRelativePath?: string
  audioAbsolutePath?: string
}

export interface SingleVideoFile {
  relativePath: string
  absolutePath: string
  mediaType: string
}

export interface SingleVideoQueueItem {
  id: string
  url: string
  provider?: string
  state: 'queued' | 'running'
  queuedAt: string
  startedAt?: string
  progressLabel?: string
  progressIndeterminate?: boolean
}

export interface SingleVideoQueueRecentResult {
  url: string
  provider?: string
  uploader?: string
  title?: string
  status: 'succeeded' | 'failed'
  summary: string
  finishedAt: string
}

export interface SingleVideoQueueStatus {
  queuedCount: number
  runningCount: number
  completedCount: number
  failedCount: number
  active?: SingleVideoQueueItem
  queuedItems: SingleVideoQueueItem[]
  recentResults: SingleVideoQueueRecentResult[]
  updatedAt: string
}

export interface SourceSyncQueueProviderStatus {
  provider: ProviderKey
  displayName: string
  queued: number
  running: number
  completed: number
  failed: number
  total: number
  activeProgressPercent?: number
  paused: boolean
}

export interface SourceSyncQueueItem {
  sourceId: string
  provider: ProviderKey
  handle: string
  accountId?: string
  state: 'queued' | 'running'
  queuedAt: string
  startedAt?: string
  progressPercent?: number
  progressLabel?: string
  progressDetail?: string
  progressIndeterminate?: boolean
  downloadedItems?: number
}

export interface SourceSyncQueueRecentResult {
  sourceId: string
  provider: ProviderKey
  handle: string
  accountId?: string
  status: 'succeeded' | 'failed' | 'skipped'
  summary: string
  finishedAt: string
}

export interface SourceSyncQueueStatus {
  queuedCount: number
  runningCount: number
  completedCount: number
  failedCount: number
  totalCount: number
  activeSourceId?: string
  activeHandle?: string
  activeProvider?: ProviderKey
  activeStartedAt?: string
  providers: SourceSyncQueueProviderStatus[]
  queuedItems: SourceSyncQueueItem[]
  runningItems: SourceSyncQueueItem[]
  recentResults: SourceSyncQueueRecentResult[]
  updatedAt: string
}

export interface SourceDeleteQueueJob {
  jobId: string
  sourceId: string
  provider: ProviderKey
  handle: string
  mode: SourceProfileDeleteMode
  state: 'queued' | 'running'
  queuedAt: string
  startedAt?: string
  progressPercent?: number
  progressLabel?: string
  progressDetail?: string
  progressIndeterminate?: boolean
  filesProcessed?: number
  filesTotal?: number
}

export interface SourceDeleteQueueRecentResult {
  jobId: string
  sourceId: string
  provider: ProviderKey
  handle: string
  mode: SourceProfileDeleteMode
  status: 'succeeded' | 'failed'
  summary: string
  finishedAt: string
  error?: string
}

export interface SourceDeleteQueueStatus {
  queuedCount: number
  runningCount: number
  completedCount: number
  failedCount: number
  totalCount: number
  activeJobId?: string
  activeSourceId?: string
  activeHandle?: string
  activeProvider?: ProviderKey
  activeMode?: SourceProfileDeleteMode
  activeStartedAt?: string
  queuedItems: SourceDeleteQueueJob[]
  runningItems: SourceDeleteQueueJob[]
  recentResults: SourceDeleteQueueRecentResult[]
  updatedAt: string
}

export interface SourceProfile {
  id: string
  provider: ProviderKey
  sourceKind: SourceKind
  handle: string
  displayName: string
  accountId?: string
  groupId?: string
  labels: string[]
  readyForDownload: boolean
  syncOptions?: SourceSyncOptions
  profileImagePath?: string
  profileImageCustom: boolean
  remoteState: SchedulerRemoteState
  isSubscription: boolean
  lastSyncedAt?: string
  syncProblemCode?: string
  syncProblemMessage?: string
  syncProblemAt?: string
  createdAt?: string
  importerId?: string
  importedAt?: string
}

export interface SourceSyncRun {
  id: string
  sourceId: string
  accountId: string
  provider: ProviderKey
  tool: string
  trigger: string
  status: 'succeeded' | 'failed' | 'skipped'
  summary: string
  commandPreview: string
  manifestSummary?: InstagramManifestSummary
  degradedCapabilities: string[]
  startedAt: string
  finishedAt: string
}

export interface InstagramManifestSectionSummary {
  section: string
  label: string
  itemCount: number
  normalizedPostCount: number
  discoveredAssetCount: number
  queuedAssetCount: number
  skippedExistingPostCount: number
  skippedDuplicatePostCount: number
  skippedUnavailablePostCount: number
  skippedExistingAssetCount: number
  skippedDuplicateAssetCount: number
}

export interface InstagramManifestSummary {
  sectionCount: number
  discoveredItemCount: number
  normalizedPostCount: number
  discoveredAssetCount: number
  queuedAssetCount: number
  skippedExistingPostCount: number
  skippedDuplicatePostCount: number
  skippedUnavailablePostCount: number
  skippedExistingAssetCount: number
  skippedDuplicateAssetCount: number
  downloadedAssetCount: number
  sections: InstagramManifestSectionSummary[]
}

export interface AccountSyncRun {
  id: string
  accountId: string
  provider: ProviderKey
  tool: string
  trigger: string
  status: 'succeeded' | 'failed'
  summary: string
  commandPreview: string
  degradedCapabilities: string[]
  startedAt: string
  finishedAt: string
}

export interface SyncPlan {
  id: string
  schedulerSetId: string
  name: string
  enabled: boolean
  mode: SyncMode
  intervalMinutes: number
  startupDelayMinutes: number
  notificationMode: NotificationMode
  targetFilter: string
  sortIndex: number
  paused: boolean
  pauseMode: SchedulerPauseMode
  pauseUntil?: string
  skipUntil?: string
  lastRunAt?: string
  lastRunStatus: PlanRunStatus
  lastRunSummary?: string
  nextDueAt?: string
  notifications: SchedulerPlanNotifications
  criteria: SchedulerPlanCriteria
}

export interface SchedulerSet {
  id: string
  name: string
  active: boolean
  plans: SyncPlan[]
}

export interface SchedulerGroup {
  id: string
  name: string
  sortIndex: number
  criteria: SchedulerPlanCriteria
}

export interface SyncPlanRun {
  id: string
  planId: string
  schedulerSetId: string
  trigger: string
  status: PlanRunStatus
  summary: string
  sourceCount: number
  startedAt: string
  finishedAt: string
}

export interface WorkspaceSnapshot {
  workspaceRoot: string
  dbPath: string
  mediaRoot: string
  desktopRuntime: DesktopRuntimeState
  providerCatalog: ProviderDescriptor[]
  appSettings: AppSetting[]
  connectorRuntimes: ConnectorRuntimeStatus[]
  accounts: ProviderAccount[]
  accountSessions: ProviderAccountSession[]
  sources: SourceProfile[]
  sourceSyncRuns: SourceSyncRun[]
  accountSyncRuns: AccountSyncRun[]
  schedulerSets: SchedulerSet[]
  schedulerGroups: SchedulerGroup[]
  syncPlanRuns: SyncPlanRun[]
  sourceMediaPaths?: Record<string, string>
}

export interface ProviderAccountUpsert {
  id?: string
  provider: ProviderKey
  displayName: string
  authMode: AuthMode
  authState: AuthState
  capabilities: string[]
  lastValidatedAt?: string
}

export interface ProviderAccountCookieImport {
  accountId: string
  importFormat: 'json' | 'netscape'
  content: string
}

export interface RuntimeLogQuery {
  limit?: number
  level?: NotificationLevel | 'debug'
  scope?: string
  provider?: ProviderKey
  accountId?: string
}

export interface SourceProfileUpsert {
  id?: string
  provider: ProviderKey
  sourceKind: SourceKind
  handle: string
  displayName: string
  accountId?: string | null
  groupId?: string | null
  labels: string[]
  readyForDownload: boolean
  syncOptions?: SourceSyncOptions
  remoteState?: SchedulerRemoteState
  isSubscription?: boolean
}

export interface SchedulerSetUpsert {
  id?: string
  name: string
  active: boolean
}

export interface SchedulerGroupUpsert {
  id?: string
  name: string
  sortIndex?: number
  criteria: SchedulerPlanCriteria
}

export interface SyncPlanUpsert {
  id?: string
  schedulerSetId: string
  name: string
  enabled: boolean
  mode: SyncMode
  intervalMinutes: number
  startupDelayMinutes: number
  notificationMode: NotificationMode
  targetFilter: string
  sortIndex?: number
  pauseMode?: SchedulerPauseMode
  pauseUntil?: string
  notifications: SchedulerPlanNotifications
  criteria: SchedulerPlanCriteria
}

export interface SchedulerPlanNotifications {
  enabled: boolean
  simple: boolean
  showImage: boolean
  showUserIcon: boolean
}

export interface SchedulerPlanCriteria {
  regular: boolean
  temporary: boolean
  favorite: boolean
  readyForDownload: boolean
  ignoreReadyForDownload: boolean
  downloadUsers: boolean
  downloadSubscriptions: boolean
  userExists: boolean
  userSuspended: boolean
  userDeleted: boolean
  labelsNo: boolean
  labelsIncluded: string[]
  labelsExcluded: string[]
  ignoreExcludedLabels: boolean
  sitesIncluded: ProviderKey[]
  sitesExcluded: ProviderKey[]
  groupIdsIncluded: string[]
  groupIdsExcluded: string[]
  usersCount?: number
  daysNumber?: number
  daysIsDownloaded: boolean
  dateFrom?: string
  dateTo?: string
  dateInRange: boolean
  advancedExpression?: string
}

export interface SyncPlanTargetPreviewInput {
  schedulerSetId?: string
  planId?: string
  criteria: SchedulerPlanCriteria
}

export interface SyncPlanTargetPreviewSource {
  id: string
  handle: string
  provider: ProviderKey
  labels: string[]
  readyForDownload: boolean
  remoteState: SchedulerRemoteState
  subscription: boolean
  lastSyncedAt?: string
}

export interface SyncPlanTargetPreview {
  sourceCount: number
  sources: SyncPlanTargetPreviewSource[]
}

export interface SetSyncPlanPauseInput {
  id: string
  pauseMode: SchedulerPauseMode
  pauseUntil?: string
}

export interface RunSyncPlanNowInput {
  id: string
  force?: boolean
}

export interface SkipSyncPlanInput {
  id: string
  mode: SchedulerSkipMode
  minutes?: number
  until?: string
}

export interface MoveSyncPlanInput {
  id: string
  direction: 'up' | 'down'
}

export interface CloneSyncPlanInput {
  id: string
}

export interface AppSettingUpsert {
  key: string
  value: string
  category?: string
  description?: string
  mutable?: boolean
}
