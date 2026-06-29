import { invoke } from '@tauri-apps/api/core'
import { emit, listen } from '@tauri-apps/api/event'
import { openPath, openUrl, revealItemInDir } from '@tauri-apps/plugin-opener'
import { DEFAULT_APP_SETTINGS, DEFAULT_PROVIDER_CATALOG } from '../domain/defaults'
import {
  createSourceSyncOptions,
  createTikTokSourceSyncOptions,
  createTwitterSourceSyncOptions,
  DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS,
  DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS,
  DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS,
} from '../domain/sourceSyncOptions'
import type {
  AccountsWindowIntent,
  AccountSyncRun,
  AppSetting,
  AppSettingUpsert,
  AuthMode,
  AuthState,
  ConnectorRuntimeStatus,
  DesktopRuntimeState,
  ImportMethodDescriptor,
  ImportPreview,
  ImportPreviewOptions,
  ImportPreviewProfile,
  ImportPreviewSummary,
  ImportProblem,
  ImportProviderDescriptor,
  ImportRootDescriptor,
  ImportQueueJob,
  ImportQueueRecentResult,
  ImportQueueStatus,
  ImportRunProfileResult,
  ImportRunRequest,
  ImportRunResult,
  InstagramNamingLedgerBackfillResult,
  InstagramManifestSectionSummary,
  InstagramManifestSummary,
  InstagramSourceSyncOptions,
  NotificationMode,
  PlanEditorWindowIntent,
  PlanRunStatus,
  ProviderAccount,
  ProviderAccountCookie,
  ProviderAccountCookieImport,
  ProviderAccountEditor,
  ProviderAccountSession,
  ProviderAccountSettingValue,
  ProviderAccountSettingValueKind,
  ProviderAccountUpsert,
  ProviderDescriptor,
  ProviderKey,
  RunSyncPlanNowInput,
  SourceEditorSeedIntent,
  SourceEditorWindowIntent,
  RunSourceSyncOptions,
  RuntimeLogEntry,
  RuntimeLogContext,
  RuntimeLogQuery,
  SourceAvailabilityCheckItem,
  SourceAvailabilityCheckResult,
  SourceDeleteQueueJob,
  SourceDeleteQueueRecentResult,
  SourceDeleteQueueStatus,
  SourceSyncQueueItem,
  SourceSyncQueueProviderStatus,
  SourceSyncQueueRecentResult,
  SourceSyncQueueStatus,
  SourceMediaGallery,
  MediaGalleryPost,
  SchedulerSet,
  SchedulerGroup,
  SchedulerGroupUpsert,
  SchedulerPlanCriteria,
  SchedulerPlanNotifications,
  SchedulerPauseMode,
  SchedulerRemoteState,
  SetSyncPlanPauseInput,
  SkipSyncPlanInput,
  SyncPlanTargetPreview,
  SyncPlanTargetPreviewInput,
  SchedulerSetUpsert,
  SourceKind,
  SourceProfile,
  SourceProfileDeleteMode,
  SourceProfileUpsert,
  SourceSyncOptions,
  TikTokSourceSyncOptions,
  TwitterSourceSyncOptions,
  SourceSyncRun,
  MoveSyncPlanInput,
  CloneSyncPlanInput,
  SyncPlanRun,
  SyncMode,
  SyncPlan,
  SyncPlanUpsert,
  WorkspaceSnapshot,
} from '../domain/models'
import { createEmptyWorkspaceSnapshot } from '../domain/workspaceSnapshot'

const PROVIDER_KEYS: ProviderKey[] = ['instagram', 'tiktok', 'reddit', 'twitter']
const AUTH_MODES: AuthMode[] = ['imported_session']
const AUTH_STATES: AuthState[] = ['ready', 'degraded', 'expired']
const SOURCE_KINDS: SourceKind[] = ['profile']
const SYNC_MODES: SyncMode[] = ['automatic', 'manual']
const NOTIFICATION_MODES: NotificationMode[] = ['summary', 'detailed']
const SCHEDULER_REMOTE_STATES: SchedulerRemoteState[] = ['exists', 'suspended', 'deleted']
const SCHEDULER_PAUSE_MODES: SchedulerPauseMode[] = ['disabled', 'unlimited', '1h', '2h', '3h', '4h', '6h', '12h', 'until']
const PLAN_RUN_STATUSES: PlanRunStatus[] = ['idle', 'succeeded', 'failed', 'skipped']
const APP_SETTING_KEY_ALIASES: Record<string, string> = {
  yt_dlp_path: 'tool.yt-dlp.path',
  gallery_dl_path: 'tool.gallery-dl.path',
  media_root: 'storage.media_root',
  notification_mode: 'policy.notifications.default',
}
const DEFAULT_SETTING_METADATA = new Map(DEFAULT_APP_SETTINGS.map((setting) => [setting.key, setting]))

let localSnapshot: WorkspaceSnapshot = createEmptyWorkspaceSnapshot()
const DESKTOP_ROUTE_ACTIVATION_EVENT_NAMES = [
  'runtime://foreground-route',
  'desktop://route-activation',
  'desktop://route_activation',
  'desktop://activate-route',
  'desktop://activate_route',
] as const
const DESKTOP_SCHEDULER_TICK_EVENT_NAME = 'runtime://scheduler-tick'
const DESKTOP_WORKSPACE_SNAPSHOT_CHANGED_EVENT_NAME = 'runtime://workspace-snapshot-changed'
const DESKTOP_SOURCE_SYNC_QUEUE_CHANGED_EVENT_NAME = 'runtime://source-sync-queue-changed'
const DESKTOP_SOURCE_DELETE_QUEUE_CHANGED_EVENT_NAME = 'runtime://source-delete-queue-changed'
const DESKTOP_IMPORT_QUEUE_CHANGED_EVENT_NAME = 'runtime://import-queue-changed'
const DESKTOP_CONNECTOR_RUNTIME_CHANGED_EVENT_NAME = 'runtime://connector-runtime-changed'
const DESKTOP_ACCOUNTS_WINDOW_INTENT_EVENT_NAME = 'runtime://accounts-window-intent'
const DESKTOP_SOURCE_EDITOR_WINDOW_INTENT_EVENT_NAME = 'runtime://source-editor-window-intent'
const DESKTOP_PROFILE_EDITOR_WINDOW_INTENT_EVENT_NAME = 'runtime://profile-editor-window-intent'
const DESKTOP_PLANS_WINDOW_INTENT_EVENT_NAME = 'runtime://plans-window-intent'
const DESKTOP_BATCH_EDITOR_WINDOW_INTENT_EVENT_NAME = 'runtime://batch-editor-window-intent'
const DESKTOP_RUNTIME_LOG_APPENDED_EVENT_NAME = 'runtime://runtime-log-appended'
const DESKTOP_FOCUS_SOURCE_EVENT_NAME = 'runtime://focus-source'
const RUNTIME_LOG_TIMEOUT_MS = 5000

type UnknownRecord = Record<string, unknown>

function isRecord(value: unknown): value is UnknownRecord {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value)
}

function pick(record: UnknownRecord, ...keys: string[]): unknown {
  for (const key of keys) {
    if (key in record) {
      return record[key]
    }
  }

  return undefined
}

function stringValue(record: UnknownRecord, keys: string[], fallback = ''): string {
  const value = pick(record, ...keys)
  return typeof value === 'string' ? value : fallback
}

function booleanValue(record: UnknownRecord, keys: string[], fallback = false): boolean {
  const value = pick(record, ...keys)
  return typeof value === 'boolean' ? value : fallback
}

function numberValue(record: UnknownRecord, keys: string[], fallback = 0): number {
  const value = pick(record, ...keys)

  if (typeof value === 'number' && Number.isFinite(value)) {
    return value
  }

  if (typeof value === 'string') {
    const parsed = Number(value)
    if (Number.isFinite(parsed)) {
      return parsed
    }
  }

  return fallback
}

function arrayValue(record: UnknownRecord, keys: string[]): unknown[] {
  const value = pick(record, ...keys)
  return Array.isArray(value) ? value : []
}

function stringArray(value: unknown): string[] {
  if (!Array.isArray(value)) {
    return []
  }

  return Array.from(
    new Set(
      value
        .map((entry) => (typeof entry === 'string' ? entry.trim() : ''))
        .filter((entry) => entry.length > 0),
    ),
  )
}

function cleanStringList(values: string[]): string[] {
  return stringArray(values)
}

function enumValue<T extends string>(value: unknown, options: readonly T[], fallback: T): T {
  return typeof value === 'string' && options.includes(value as T) ? (value as T) : fallback
}

function createId(prefix: string): string {
  return `${prefix}-${crypto.randomUUID().slice(0, 8)}`
}

function canonicalizeAppSettingKey(key: string): string {
  return APP_SETTING_KEY_ALIASES[key] ?? key
}

function guessSettingCategory(key: string): string {
  if (key.startsWith('tool.')) {
    return 'tools'
  }

  if (key.startsWith('policy.')) {
    return 'policy'
  }

  if (key.startsWith('storage.')) {
    return 'storage'
  }

  return 'general'
}

function optionalStringValue(record: UnknownRecord, keys: string[]): string | undefined {
  const value = pick(record, ...keys)
  return typeof value === 'string' && value.trim().length > 0 ? value : undefined
}

function optionalNumberValue(record: UnknownRecord, keys: string[]): number | undefined {
  const value = pick(record, ...keys)
  if (typeof value === 'number' && Number.isFinite(value)) {
    return value
  }

  if (typeof value === 'string') {
    const parsed = Number(value)
    return Number.isFinite(parsed) ? parsed : undefined
  }

  return undefined
}

function optionalEnumValue<T extends string>(value: unknown, options: readonly T[]): T | undefined {
  return typeof value === 'string' && options.includes(value as T) ? (value as T) : undefined
}

function toSnakeCaseKey(key: string): string {
  return key.replace(/[A-Z]/g, (character) => `_${character.toLowerCase()}`)
}

function buildInvokeArgs<T extends object>(
  payload?: T,
  extra?: Record<string, unknown>,
): Record<string, unknown> | undefined {
  if (!payload && !extra) {
    return undefined
  }

  const args: Record<string, unknown> = {}

  if (payload) {
    args.input = payload
    for (const [key, value] of Object.entries(payload as Record<string, unknown>)) {
      args[key] = value
      args[toSnakeCaseKey(key)] = value
    }
  }

  if (extra) {
    for (const [key, value] of Object.entries(extra)) {
      args[key] = value
    }
  }

  return args
}

function providerDescriptorFor(provider: ProviderKey, catalog: ProviderDescriptor[]): ProviderDescriptor {
  return catalog.find((descriptor) => descriptor.key === provider)
    ?? DEFAULT_PROVIDER_CATALOG.find((descriptor) => descriptor.key === provider)
    ?? DEFAULT_PROVIDER_CATALOG[0]
}

function sortByLabel<T>(items: T[], getLabel: (item: T) => string): T[] {
  return [...items].sort((left, right) => getLabel(left).localeCompare(getLabel(right), undefined, { sensitivity: 'base' }))
}

function sortAccounts(accounts: ProviderAccount[]): ProviderAccount[] {
  return [...accounts].sort((left, right) => {
    if (left.provider !== right.provider) {
      return PROVIDER_KEYS.indexOf(left.provider) - PROVIDER_KEYS.indexOf(right.provider)
    }

    return left.displayName.localeCompare(right.displayName, undefined, { sensitivity: 'base' })
  })
}

function sortSources(sources: SourceProfile[]): SourceProfile[] {
  return [...sources].sort((left, right) => {
    if (left.provider !== right.provider) {
      return PROVIDER_KEYS.indexOf(left.provider) - PROVIDER_KEYS.indexOf(right.provider)
    }

    return left.handle.localeCompare(right.handle, undefined, { sensitivity: 'base' })
  })
}

function sortSchedulerSets(schedulerSets: SchedulerSet[]): SchedulerSet[] {
  return [...schedulerSets].sort((left, right) => {
    if (left.active !== right.active) {
      return left.active ? -1 : 1
    }

    return left.name.localeCompare(right.name, undefined, { sensitivity: 'base' })
  })
}

function sortPlans(plans: SyncPlan[]): SyncPlan[] {
  return [...plans].sort((left, right) => {
    const sortOrder = left.sortIndex - right.sortIndex
    return sortOrder !== 0
      ? sortOrder
      : left.name.localeCompare(right.name, undefined, { sensitivity: 'base' })
  })
}

function sortSettings(settings: AppSetting[]): AppSetting[] {
  return [...settings].sort((left, right) => {
    const categoryOrder = left.category.localeCompare(right.category, undefined, { sensitivity: 'base' })
    return categoryOrder !== 0
      ? categoryOrder
      : left.key.localeCompare(right.key, undefined, { sensitivity: 'base' })
  })
}

function sortProviderCatalog(catalog: ProviderDescriptor[]): ProviderDescriptor[] {
  return [...catalog].sort((left, right) => PROVIDER_KEYS.indexOf(left.key) - PROVIDER_KEYS.indexOf(right.key))
}

function normalizeProviderKey(value: unknown, fallback: ProviderKey = 'instagram'): ProviderKey {
  return enumValue(value, PROVIDER_KEYS, fallback)
}

function normalizeAccountsWindowIntent(value: unknown): AccountsWindowIntent | null {
  if (!isRecord(value)) {
    return null
  }

  const providerValue = optionalStringValue(value, ['initialProvider', 'initial_provider'])
  const modeValue = optionalStringValue(value, ['initialMode', 'initial_mode'])
  const accountId = optionalStringValue(value, ['initialAccountId', 'initial_account_id'])
  const provider = providerValue ? normalizeProviderKey(providerValue, 'instagram') : undefined
  const mode = modeValue === 'create' || modeValue === 'edit' ? modeValue : undefined

  return {
    initialAccountId: accountId,
    initialProvider: provider,
    initialMode: mode,
  }
}

function normalizeSourceEditorWindowIntent(value: unknown): SourceEditorWindowIntent | null {
  if (!isRecord(value)) {
    return null
  }

  const sourceId = optionalStringValue(value, ['sourceId', 'source_id'])
  const providerValue = optionalStringValue(value, ['preferredProvider', 'preferred_provider'])
  const preferredAccountId = optionalStringValue(value, ['preferredAccountId', 'preferred_account_id'])
  const preferredProvider = providerValue ? normalizeProviderKey(providerValue, 'instagram') : undefined
  const seed = normalizeSourceEditorSeedIntent(pick(value, 'seed'))

  return {
    sourceId,
    preferredProvider,
    preferredAccountId,
    seed,
  }
}

function normalizeSourceEditorSeedIntent(value: unknown): SourceEditorSeedIntent | undefined {
  if (!isRecord(value)) {
    return undefined
  }

  const providerValue = optionalStringValue(value, ['provider'])
  const handle = optionalStringValue(value, ['handle'])
  if (!providerValue || !handle) {
    return undefined
  }

  const provider = normalizeProviderKey(providerValue, 'instagram')
  const displayName = optionalStringValue(value, ['displayName', 'display_name'])
    ?? handle.replace(/^@+/, '')

  return {
    provider,
    handle,
    displayName,
  }
}

function normalizePlanEditorWindowIntent(value: unknown): PlanEditorWindowIntent | null {
  if (!isRecord(value)) {
    return null
  }

  return {
    mode: enumValue(pick(value, 'mode'), ['new', 'edit', 'clone'] as const, 'edit'),
    planId: optionalStringValue(value, ['planId', 'plan_id']),
    schedulerSetId: optionalStringValue(value, ['schedulerSetId', 'scheduler_set_id']),
  }
}

function optionalProviderKey(record: UnknownRecord, keys: string[]): ProviderKey | undefined {
  const value = pick(record, ...keys)
  return typeof value === 'string' && PROVIDER_KEYS.includes(value as ProviderKey)
    ? (value as ProviderKey)
    : undefined
}

function normalizeProviderCatalog(value: unknown): ProviderDescriptor[] {
  if (!Array.isArray(value)) {
    return structuredClone(DEFAULT_PROVIDER_CATALOG)
  }

  const catalog = value
    .map((entry) => {
      if (!isRecord(entry)) {
        return null
      }

      const key = normalizeProviderKey(
        pick(entry, 'key', 'provider', 'providerKey', 'provider_key'),
        'instagram',
      )
      const fallback = providerDescriptorFor(key, DEFAULT_PROVIDER_CATALOG)

      return {
        key,
        displayName: stringValue(entry, ['displayName', 'display_name', 'name'], fallback.displayName),
        authModes: stringArray(pick(entry, 'authModes', 'auth_modes')).map((mode) => enumValue(mode, AUTH_MODES, fallback.authModes[0])) as AuthMode[],
        sourceKinds: stringArray(pick(entry, 'sourceKinds', 'source_kinds')).map((kind) => enumValue(kind, SOURCE_KINDS, fallback.sourceKinds[0])) as SourceKind[],
        supportsMultipleAccounts: booleanValue(
          entry,
          ['supportsMultipleAccounts', 'supports_multiple_accounts', 'multiAccount', 'multi_account'],
          fallback.supportsMultipleAccounts,
        ),
        defaultCapabilities: stringArray(
          pick(entry, 'defaultCapabilities', 'default_capabilities', 'capabilities'),
        ).length > 0
          ? stringArray(pick(entry, 'defaultCapabilities', 'default_capabilities', 'capabilities'))
          : fallback.defaultCapabilities,
        notes: stringValue(entry, ['notes', 'description'], fallback.notes),
      } satisfies ProviderDescriptor
    })
    .filter((entry): entry is ProviderDescriptor => entry !== null)

  if (catalog.length === 0) {
    return structuredClone(DEFAULT_PROVIDER_CATALOG)
  }

  return sortProviderCatalog(
    catalog.map((descriptor) => ({
      ...descriptor,
      authModes:
        descriptor.authModes.length > 0
          ? descriptor.authModes
          : providerDescriptorFor(descriptor.key, DEFAULT_PROVIDER_CATALOG).authModes,
      sourceKinds: descriptor.sourceKinds.length > 0 ? descriptor.sourceKinds : ['profile'],
      defaultCapabilities:
        descriptor.defaultCapabilities.length > 0
          ? descriptor.defaultCapabilities
          : providerDescriptorFor(descriptor.key, DEFAULT_PROVIDER_CATALOG).defaultCapabilities,
    })),
  )
}

function normalizeAppSettings(value: unknown): AppSetting[] {
  const settingsByKey = new Map<string, AppSetting>(
    DEFAULT_APP_SETTINGS.map((setting) => [setting.key, { ...setting }]),
  )

  if (!Array.isArray(value)) {
    return sortSettings(Array.from(settingsByKey.values()))
  }

  for (const entry of value) {
    if (!isRecord(entry)) {
      continue
    }

    const rawKey = stringValue(entry, ['key', 'name'])
    const key = canonicalizeAppSettingKey(rawKey)

    if (!key) {
      continue
    }

    const metadata = DEFAULT_SETTING_METADATA.get(key)
    settingsByKey.set(key, {
      key,
      value: stringValue(entry, ['value'], metadata?.value ?? ''),
      category: metadata?.category ?? stringValue(entry, ['category'], guessSettingCategory(key)),
      description: metadata?.description ?? optionalStringValue(entry, ['description']),
      mutable: metadata?.mutable ?? booleanValue(entry, ['mutable', 'isMutable', 'is_mutable'], true),
    })
  }

  return sortSettings(Array.from(settingsByKey.values()))
}

function normalizeProviderAccount(value: unknown, catalog: ProviderDescriptor[]): ProviderAccount | null {
  if (!isRecord(value)) {
    return null
  }

  const provider = normalizeProviderKey(pick(value, 'provider', 'provider_key', 'providerKey'))
  const descriptor = providerDescriptorFor(provider, catalog)

  return {
    id: stringValue(value, ['id'], createId('account')),
    provider,
    displayName: stringValue(value, ['displayName', 'display_name', 'name'], 'Unnamed account'),
    authMode: enumValue(
      pick(value, 'authMode', 'auth_mode'),
      AUTH_MODES,
      descriptor.authModes[0] ?? 'imported_session',
    ),
    authState: enumValue(pick(value, 'authState', 'auth_state', 'state'), AUTH_STATES, 'ready'),
    capabilities: stringArray(pick(value, 'capabilities')).length > 0
      ? stringArray(pick(value, 'capabilities'))
      : descriptor.defaultCapabilities,
    lastValidatedAt: stringValue(value, ['lastValidatedAt', 'last_validated_at'], new Date().toISOString()),
  }
}

function normalizeSourceProfile(value: unknown): SourceProfile | null {
  if (!isRecord(value)) {
    return null
  }

  const provider = normalizeProviderKey(pick(value, 'provider', 'provider_key', 'providerKey'))

  return {
    id: stringValue(value, ['id'], createId('source')),
    provider,
    sourceKind: enumValue(pick(value, 'sourceKind', 'source_kind'), SOURCE_KINDS, 'profile'),
    handle: stringValue(value, ['handle'], ''),
    displayName: stringValue(value, ['displayName', 'display_name', 'name'], ''),
    accountId: stringValue(value, ['accountId', 'account_id']) || undefined,
    groupId: stringValue(value, ['groupId', 'group_id']) || undefined,
    labels: stringArray(pick(value, 'labels')),
    readyForDownload: booleanValue(value, ['readyForDownload', 'ready_for_download'], false),
    syncOptions: normalizeSourceSyncOptions(
      pick(value, 'syncOptions', 'sync_options', 'sync_options_json'),
      provider,
    ),
    profileImagePath: stringValue(value, ['profileImagePath', 'profile_image_path']) || undefined,
    profileImageCustom: booleanValue(value, ['profileImageCustom', 'profile_image_custom'], false),
    remoteState: enumValue(
      pick(value, 'remoteState', 'remote_state'),
      SCHEDULER_REMOTE_STATES,
      'exists',
    ),
    isSubscription: booleanValue(value, ['isSubscription', 'is_subscription'], false),
    lastSyncedAt: optionalStringValue(value, ['lastSyncedAt', 'last_synced_at']),
    syncProblemCode: stringValue(value, ['syncProblemCode', 'sync_problem_code']) || undefined,
    syncProblemMessage: stringValue(value, ['syncProblemMessage', 'sync_problem_message']) || undefined,
    syncProblemAt: optionalStringValue(value, ['syncProblemAt', 'sync_problem_at']),
  }
}

function normalizeSourceAvailabilityCheckItem(value: unknown): SourceAvailabilityCheckItem | null {
  if (!isRecord(value)) {
    return null
  }

  const status = enumValue(
    pick(value, 'status'),
    ['unchanged', 'updated_handle', 'marked_problem', 'skipped', 'failed'] as const,
    'failed',
  )

  return {
    sourceId: stringValue(value, ['sourceId', 'source_id'], ''),
    provider: stringValue(value, ['provider'], ''),
    previousHandle: stringValue(value, ['previousHandle', 'previous_handle'], ''),
    currentHandle: optionalStringValue(value, ['currentHandle', 'current_handle']),
    status,
    message: stringValue(value, ['message'], ''),
  }
}

function normalizeSourceAvailabilityCheckResult(value: unknown): SourceAvailabilityCheckResult | null {
  if (!isRecord(value)) {
    return null
  }

  const snapshot = normalizeWorkspaceSnapshot(pick(value, 'snapshot'))
  const items = arrayValue(value, ['items'])
    .map((entry) => normalizeSourceAvailabilityCheckItem(entry))
    .filter((entry): entry is SourceAvailabilityCheckItem => entry !== null)

  return {
    snapshot,
    requested: numberValue(value, ['requested'], items.length),
    processed: numberValue(value, ['processed'], items.length),
    unchanged: numberValue(value, ['unchanged'], 0),
    updatedHandle: numberValue(value, ['updatedHandle', 'updated_handle'], 0),
    markedProblem: numberValue(value, ['markedProblem', 'marked_problem'], 0),
    skipped: numberValue(value, ['skipped'], 0),
    failed: numberValue(value, ['failed'], 0),
    items,
  }
}

function normalizeSourceSyncRun(value: unknown): SourceSyncRun | null {
  if (!isRecord(value)) {
    return null
  }

  const manifestSummary =
    normalizeInstagramManifestSummary(
      pick(value, 'manifestSummary', 'manifest_summary'),
    )
    ?? normalizeInstagramManifestSummaryFromJson(
      stringValue(value, ['manifestSummaryJson', 'manifest_summary_json']),
    )

  return {
    id: stringValue(value, ['id'], createId('sync-run')),
    sourceId: stringValue(value, ['sourceId', 'source_id'], ''),
    accountId: stringValue(value, ['accountId', 'account_id'], ''),
    provider: normalizeProviderKey(pick(value, 'provider', 'provider_key', 'providerKey')),
    tool: stringValue(value, ['tool'], ''),
    trigger: stringValue(value, ['trigger'], 'manual'),
    status: enumValue(pick(value, 'status'), ['succeeded', 'failed', 'skipped'] as const, 'failed'),
    summary: stringValue(value, ['summary'], ''),
    commandPreview: stringValue(value, ['commandPreview', 'command_preview'], ''),
    ...(manifestSummary ? { manifestSummary } : {}),
    degradedCapabilities: stringArray(
      pick(value, 'degradedCapabilities', 'degraded_capabilities'),
    ),
    startedAt: stringValue(value, ['startedAt', 'started_at'], new Date().toISOString()),
    finishedAt: stringValue(value, ['finishedAt', 'finished_at'], new Date().toISOString()),
  }
}

function normalizeAccountSyncRun(value: unknown): AccountSyncRun | null {
  if (!isRecord(value)) {
    return null
  }

  return {
    id: stringValue(value, ['id'], createId('account-sync-run')),
    accountId: stringValue(value, ['accountId', 'account_id'], ''),
    provider: normalizeProviderKey(pick(value, 'provider', 'provider_key', 'providerKey')),
    tool: stringValue(value, ['tool'], ''),
    trigger: stringValue(value, ['trigger'], 'manual'),
    status: enumValue(pick(value, 'status'), ['succeeded', 'failed'] as const, 'failed'),
    summary: stringValue(value, ['summary'], ''),
    commandPreview: stringValue(value, ['commandPreview', 'command_preview'], ''),
    degradedCapabilities: stringArray(
      pick(value, 'degradedCapabilities', 'degraded_capabilities'),
    ),
    startedAt: stringValue(value, ['startedAt', 'started_at'], new Date().toISOString()),
    finishedAt: stringValue(value, ['finishedAt', 'finished_at'], new Date().toISOString()),
  }
}

function normalizeProviderAccountSession(value: unknown): ProviderAccountSession | null {
  if (!isRecord(value)) {
    return null
  }

  return {
    accountId: stringValue(value, ['accountId', 'account_id'], ''),
    authMode: enumValue(pick(value, 'authMode', 'auth_mode'), AUTH_MODES, 'imported_session'),
    sessionFormat: stringValue(value, ['sessionFormat', 'session_format'], 'unknown'),
    fingerprint: stringValue(value, ['fingerprint'], ''),
    cookieCount: numberValue(value, ['cookieCount', 'cookie_count'], 0),
    importedAt: stringValue(value, ['importedAt', 'imported_at'], new Date().toISOString()),
    lastValidatedAt: optionalStringValue(value, ['lastValidatedAt', 'last_validated_at']),
    lastValidationError: optionalStringValue(value, ['lastValidationError', 'last_validation_error']),
    hasSecret: booleanValue(value, ['hasSecret', 'has_secret'], false),
  }
}

async function withTimeout<T>(operation: Promise<T>, message: string, timeoutMs = RUNTIME_LOG_TIMEOUT_MS): Promise<T> {
  let timeoutHandle: ReturnType<typeof setTimeout> | undefined

  try {
    return await Promise.race([
      operation,
      new Promise<T>((_, reject) => {
        timeoutHandle = setTimeout(() => reject(new Error(message)), timeoutMs)
      }),
    ])
  } finally {
    if (timeoutHandle) {
      clearTimeout(timeoutHandle)
    }
  }
}

function normalizeProviderAccountCookie(value: unknown): ProviderAccountCookie | null {
  if (!isRecord(value)) {
    return null
  }

  return {
    domain: stringValue(value, ['domain']),
    name: stringValue(value, ['name']),
    value: stringValue(value, ['value']),
    path: stringValue(value, ['path'], '/'),
    expiresAt: optionalStringValue(value, ['expiresAt', 'expires_at']),
    secure: booleanValue(value, ['secure']),
    httpOnly: booleanValue(value, ['httpOnly', 'http_only']),
  }
}

function normalizeProviderAccountSettingValueKind(value: unknown): ProviderAccountSettingValueKind {
  return value === 'json' ? 'json' : 'string'
}

function normalizeProviderAccountSettingValue(value: unknown): ProviderAccountSettingValue | null {
  if (!isRecord(value)) {
    return null
  }

  return {
    settingKey: stringValue(value, ['settingKey', 'setting_key'], ''),
    valueKind: normalizeProviderAccountSettingValueKind(pick(value, 'valueKind', 'value_kind')),
    stringValue: optionalStringValue(value, ['stringValue', 'string_value']),
    jsonValue: pick(value, 'jsonValue', 'json_value'),
  }
}

function normalizeProviderAccountEditor(value: unknown, snapshot: WorkspaceSnapshot): ProviderAccountEditor | null {
  if (!isRecord(value)) {
    return null
  }

  const account = normalizeProviderAccount(pick(value, 'account'), snapshot.providerCatalog)
  if (!account) {
    return null
  }

  const session = normalizeProviderAccountSession(pick(value, 'session'))
  const settings = arrayValue(value, ['settings'])
    .map((entry) => normalizeProviderAccountSettingValue(entry))
    .filter((entry): entry is ProviderAccountSettingValue => entry !== null)

  return {
    account,
    session,
    settings,
  }
}

function normalizeSyncPlan(value: unknown, schedulerSetId?: string): SyncPlan | null {
  if (!isRecord(value)) {
    return null
  }

  const notificationMode = enumValue(
    pick(value, 'notificationMode', 'notification_mode'),
    NOTIFICATION_MODES,
    'summary',
  )
  const targetFilter = stringValue(value, ['targetFilter', 'target_filter'], '')

  return {
    id: stringValue(value, ['id'], createId('plan')),
    schedulerSetId: stringValue(
      value,
      ['schedulerSetId', 'scheduler_set_id', 'setId', 'set_id'],
      schedulerSetId ?? '',
    ),
    name: stringValue(value, ['name'], 'Unnamed plan'),
    enabled: booleanValue(value, ['enabled'], true),
    mode: enumValue(pick(value, 'mode'), SYNC_MODES, 'automatic'),
    intervalMinutes: numberValue(value, ['intervalMinutes', 'interval_minutes'], 30),
    startupDelayMinutes: numberValue(value, ['startupDelayMinutes', 'startup_delay_minutes'], 0),
    notificationMode,
    targetFilter,
    sortIndex: numberValue(value, ['sortIndex', 'sort_index'], 0),
    paused: booleanValue(value, ['paused'], false),
    pauseMode: enumValue(pick(value, 'pauseMode', 'pause_mode'), SCHEDULER_PAUSE_MODES, 'disabled'),
    pauseUntil: optionalStringValue(value, ['pauseUntil', 'pause_until']),
    skipUntil: optionalStringValue(value, ['skipUntil', 'skip_until']),
    lastRunAt: optionalStringValue(value, ['lastRunAt', 'last_run_at']),
    lastRunStatus: enumValue(
      pick(value, 'lastRunStatus', 'last_run_status'),
      PLAN_RUN_STATUSES,
      'idle',
    ),
    lastRunSummary: optionalStringValue(value, ['lastRunSummary', 'last_run_summary']),
    nextDueAt: optionalStringValue(value, ['nextDueAt', 'next_due_at']),
    notifications: normalizeSchedulerPlanNotifications(pick(value, 'notifications'), notificationMode),
    criteria: normalizeSchedulerPlanCriteria(pick(value, 'criteria'), targetFilter),
  }
}

function normalizeSchedulerPlanNotifications(value: unknown, notificationMode: NotificationMode): SchedulerPlanNotifications {
  if (!isRecord(value)) {
    return {
      enabled: true,
      simple: notificationMode === 'summary',
      showImage: notificationMode === 'detailed',
      showUserIcon: notificationMode === 'detailed',
    }
  }

  return {
    enabled: booleanValue(value, ['enabled'], true),
    simple: booleanValue(value, ['simple'], notificationMode === 'summary'),
    showImage: booleanValue(value, ['showImage', 'show_image'], notificationMode === 'detailed'),
    showUserIcon: booleanValue(value, ['showUserIcon', 'show_user_icon'], notificationMode === 'detailed'),
  }
}

function normalizeSchedulerPlanCriteria(value: unknown, targetFilter = ''): SchedulerPlanCriteria {
  const record = isRecord(value) ? value : {}
  return {
    regular: booleanValue(record, ['regular'], false),
    temporary: booleanValue(record, ['temporary'], false),
    favorite: booleanValue(record, ['favorite'], false),
    readyForDownload: booleanValue(record, ['readyForDownload', 'ready_for_download'], true),
    ignoreReadyForDownload: booleanValue(record, ['ignoreReadyForDownload', 'ignore_ready_for_download'], false),
    downloadUsers: booleanValue(record, ['downloadUsers', 'download_users'], true),
    downloadSubscriptions: booleanValue(record, ['downloadSubscriptions', 'download_subscriptions'], true),
    userExists: booleanValue(record, ['userExists', 'user_exists'], true),
    userSuspended: booleanValue(record, ['userSuspended', 'user_suspended'], false),
    userDeleted: booleanValue(record, ['userDeleted', 'user_deleted'], false),
    labelsNo: booleanValue(record, ['labelsNo', 'labels_no'], false),
    labelsIncluded: stringArray(pick(record, 'labelsIncluded', 'labels_included')),
    labelsExcluded: stringArray(pick(record, 'labelsExcluded', 'labels_excluded')),
    ignoreExcludedLabels: booleanValue(record, ['ignoreExcludedLabels', 'ignore_excluded_labels'], false),
    sitesIncluded: stringArray(pick(record, 'sitesIncluded', 'sites_included')).map((entry) => normalizeProviderKey(entry)),
    sitesExcluded: stringArray(pick(record, 'sitesExcluded', 'sites_excluded')).map((entry) => normalizeProviderKey(entry)),
    groupIdsIncluded: stringArray(pick(record, 'groupIdsIncluded', 'group_ids_included')),
    groupIdsExcluded: stringArray(pick(record, 'groupIdsExcluded', 'group_ids_excluded')),
    groupsOnly: booleanValue(record, ['groupsOnly', 'groups_only'], false),
    usersCount: optionalNumberValue(record, ['usersCount', 'users_count']),
    daysNumber: optionalNumberValue(record, ['daysNumber', 'days_number']),
    daysIsDownloaded: booleanValue(record, ['daysIsDownloaded', 'days_is_downloaded'], false),
    dateFrom: optionalStringValue(record, ['dateFrom', 'date_from']),
    dateTo: optionalStringValue(record, ['dateTo', 'date_to']),
    dateInRange: booleanValue(record, ['dateInRange', 'date_in_range'], true),
    dateMode: optionalStringValue(record, ['dateMode', 'date_mode']) as SchedulerPlanCriteria['dateMode'],
    advancedExpression: optionalStringValue(record, ['advancedExpression', 'advanced_expression']) ?? targetFilter,
  }
}

function normalizeSchedulerGroup(value: unknown): SchedulerGroup | null {
  if (!isRecord(value)) {
    return null
  }

  return {
    id: stringValue(value, ['id'], createId('scheduler-group')),
    name: stringValue(value, ['name'], 'Unnamed group'),
    sortIndex: numberValue(value, ['sortIndex', 'sort_index'], 0),
    criteria: normalizeSchedulerPlanCriteria(pick(value, 'criteria')),
  }
}

function normalizeSyncPlanRun(value: unknown): SyncPlanRun | null {
  if (!isRecord(value)) {
    return null
  }

  return {
    id: stringValue(value, ['id'], createId('plan-run')),
    planId: stringValue(value, ['planId', 'plan_id'], ''),
    schedulerSetId: stringValue(value, ['schedulerSetId', 'scheduler_set_id'], ''),
    trigger: stringValue(value, ['trigger'], 'manual'),
    status: enumValue(pick(value, 'status'), PLAN_RUN_STATUSES, 'idle'),
    summary: stringValue(value, ['summary'], ''),
    sourceCount: numberValue(value, ['sourceCount', 'source_count'], 0),
    startedAt: stringValue(value, ['startedAt', 'started_at'], new Date().toISOString()),
    finishedAt: stringValue(value, ['finishedAt', 'finished_at'], new Date().toISOString()),
  }
}

function normalizeRuntimeLogEntry(value: unknown): RuntimeLogEntry | null {
  if (!isRecord(value)) {
    return null
  }

  const levelValue = stringValue(value, ['level'], 'info')
  const level: RuntimeLogEntry['level'] =
    levelValue === 'warning' ||
    levelValue === 'error' ||
    levelValue === 'debug'
      ? levelValue
      : 'info'

  return {
    id: stringValue(value, ['id'], createId('runtime-log')),
    timestamp: stringValue(value, ['timestamp'], new Date().toISOString()),
    scope: stringValue(value, ['scope'], 'runtime'),
    level,
    accountId: optionalStringValue(value, ['accountId', 'account_id']),
    provider: optionalStringValue(value, ['provider', 'provider_key', 'providerKey']) as RuntimeLogEntry['provider'],
    sourceId: optionalStringValue(value, ['sourceId', 'source_id']),
    sourceHandle: optionalStringValue(value, ['sourceHandle', 'source_handle']),
    message: stringValue(value, ['message'], ''),
    detail: optionalStringValue(value, ['detail']),
  }
}

function normalizeRuntimeLogContext(value: unknown): RuntimeLogContext | null {
  if (!isRecord(value)) {
    return null
  }

  const providerCatalog = normalizeProviderCatalog(
    pick(value, 'providerCatalog', 'provider_catalog', 'providers', 'providerRegistry', 'provider_registry'),
  )
  const accounts = sortAccounts(
    arrayValue(value, ['accounts', 'providerAccounts', 'provider_accounts'])
      .map((entry) => normalizeProviderAccount(entry, providerCatalog))
      .filter((entry): entry is ProviderAccount => entry !== null),
  )

  return {
    providerCatalog,
    accounts,
  }
}

function normalizeSourceSyncQueueProviderStatus(
  value: unknown,
): SourceSyncQueueProviderStatus | null {
  if (!isRecord(value)) {
    return null
  }

  const provider = normalizeProviderKey(pick(value, 'provider', 'providerKey', 'provider_key'))
  const fallback = providerDescriptorFor(provider, localSnapshot.providerCatalog)

  const queued = numberValue(value, ['queued'], 0)
  const running = numberValue(value, ['running'], 0)
  const completed = numberValue(value, ['completed'], 0)
  const failed = numberValue(value, ['failed'], 0)

  return {
    provider,
    displayName: stringValue(
      value,
      ['displayName', 'display_name', 'providerDisplayName', 'provider_display_name'],
      fallback.displayName,
    ),
    queued,
    running,
    completed,
    failed,
    total: numberValue(value, ['total'], queued + running + completed + failed),
    activeProgressPercent: optionalNumberValue(value, ['activeProgressPercent', 'active_progress_percent']),
    paused: booleanValue(value, ['paused'], false),
  }
}

function normalizeSourceSyncQueueItem(value: unknown): SourceSyncQueueItem | null {
  if (!isRecord(value)) {
    return null
  }

  const progressPercentRaw = pick(value, 'progressPercent', 'progress_percent')
  const progressPercent = typeof progressPercentRaw === 'number' && Number.isFinite(progressPercentRaw)
    ? Math.max(0, Math.min(100, Math.round(progressPercentRaw)))
    : undefined
  const downloadedItemsRaw = pick(value, 'downloadedItems', 'downloaded_items')
  const downloadedItems = typeof downloadedItemsRaw === 'number' && Number.isFinite(downloadedItemsRaw)
    ? Math.max(0, Math.round(downloadedItemsRaw))
    : undefined

  return {
    sourceId: stringValue(value, ['sourceId', 'source_id'], ''),
    provider: normalizeProviderKey(pick(value, 'provider', 'providerKey', 'provider_key')),
    handle: stringValue(value, ['handle'], ''),
    accountId: optionalStringValue(value, ['accountId', 'account_id']),
    state: enumValue(pick(value, 'state'), ['queued', 'running'] as const, 'queued'),
    queuedAt: stringValue(value, ['queuedAt', 'queued_at'], new Date().toISOString()),
    startedAt: optionalStringValue(value, ['startedAt', 'started_at']),
    progressPercent,
    progressLabel: optionalStringValue(value, ['progressLabel', 'progress_label']),
    progressDetail: optionalStringValue(value, ['progressDetail', 'progress_detail']),
    progressIndeterminate: booleanValue(
      value,
      ['progressIndeterminate', 'progress_indeterminate'],
      progressPercent === undefined,
    ),
    downloadedItems,
  }
}

function normalizeSourceSyncQueueRecentResult(value: unknown): SourceSyncQueueRecentResult | null {
  if (!isRecord(value)) {
    return null
  }

  return {
    sourceId: stringValue(value, ['sourceId', 'source_id'], ''),
    provider: normalizeProviderKey(pick(value, 'provider', 'providerKey', 'provider_key')),
    handle: stringValue(value, ['handle'], ''),
    accountId: optionalStringValue(value, ['accountId', 'account_id']),
    status: enumValue(pick(value, 'status'), ['succeeded', 'failed', 'skipped'] as const, 'failed'),
    summary: stringValue(value, ['summary'], ''),
    finishedAt: stringValue(value, ['finishedAt', 'finished_at'], new Date().toISOString()),
  }
}

function normalizeInstagramManifestSummaryFromJson(raw: string): InstagramManifestSummary | undefined {
  const trimmed = raw.trim()
  if (!trimmed) {
    return undefined
  }

  try {
    return normalizeInstagramManifestSummary(JSON.parse(trimmed))
  } catch {
    return undefined
  }
}

function normalizeInstagramManifestSummary(value: unknown): InstagramManifestSummary | undefined {
  if (!isRecord(value)) {
    return undefined
  }

  const rawSections = arrayValue(value, ['sections'])
  const sections = rawSections
    .map((entry) => normalizeInstagramManifestSectionSummary(entry))
    .filter((entry): entry is InstagramManifestSectionSummary => entry !== undefined)

  return {
    sectionCount: numberValue(value, ['sectionCount', 'section_count'], sections.length),
    discoveredItemCount: numberValue(value, ['discoveredItemCount', 'discovered_item_count'], 0),
    normalizedPostCount: numberValue(value, ['normalizedPostCount', 'normalized_post_count'], 0),
    discoveredAssetCount: numberValue(value, ['discoveredAssetCount', 'discovered_asset_count'], 0),
    queuedAssetCount: numberValue(value, ['queuedAssetCount', 'queued_asset_count'], 0),
    skippedExistingPostCount: numberValue(
      value,
      ['skippedExistingPostCount', 'skipped_existing_post_count'],
      0,
    ),
    skippedDuplicatePostCount: numberValue(
      value,
      ['skippedDuplicatePostCount', 'skipped_duplicate_post_count'],
      0,
    ),
    skippedUnavailablePostCount: numberValue(
      value,
      ['skippedUnavailablePostCount', 'skipped_unavailable_post_count'],
      0,
    ),
    skippedExistingAssetCount: numberValue(
      value,
      ['skippedExistingAssetCount', 'skipped_existing_asset_count'],
      0,
    ),
    skippedDuplicateAssetCount: numberValue(
      value,
      ['skippedDuplicateAssetCount', 'skipped_duplicate_asset_count'],
      0,
    ),
    downloadedAssetCount: numberValue(value, ['downloadedAssetCount', 'downloaded_asset_count'], 0),
    sections,
  }
}

function normalizeInstagramManifestSectionSummary(
  value: unknown,
): InstagramManifestSectionSummary | undefined {
  if (!isRecord(value)) {
    return undefined
  }

  return {
    section: stringValue(value, ['section'], ''),
    label: stringValue(value, ['label'], ''),
    itemCount: numberValue(value, ['itemCount', 'item_count'], 0),
    normalizedPostCount: numberValue(value, ['normalizedPostCount', 'normalized_post_count'], 0),
    discoveredAssetCount: numberValue(value, ['discoveredAssetCount', 'discovered_asset_count'], 0),
    queuedAssetCount: numberValue(value, ['queuedAssetCount', 'queued_asset_count'], 0),
    skippedExistingPostCount: numberValue(
      value,
      ['skippedExistingPostCount', 'skipped_existing_post_count'],
      0,
    ),
    skippedDuplicatePostCount: numberValue(
      value,
      ['skippedDuplicatePostCount', 'skipped_duplicate_post_count'],
      0,
    ),
    skippedUnavailablePostCount: numberValue(
      value,
      ['skippedUnavailablePostCount', 'skipped_unavailable_post_count'],
      0,
    ),
    skippedExistingAssetCount: numberValue(
      value,
      ['skippedExistingAssetCount', 'skipped_existing_asset_count'],
      0,
    ),
    skippedDuplicateAssetCount: numberValue(
      value,
      ['skippedDuplicateAssetCount', 'skipped_duplicate_asset_count'],
      0,
    ),
  }
}

function normalizeSourceSyncQueueStatus(value: unknown): SourceSyncQueueStatus {
  if (!isRecord(value)) {
    return {
      queuedCount: 0,
      runningCount: 0,
      completedCount: 0,
      failedCount: 0,
      totalCount: 0,
      providers: [],
      queuedItems: [],
      runningItems: [],
      recentResults: [],
      updatedAt: new Date().toISOString(),
    }
  }

  return {
    queuedCount: numberValue(value, ['queuedCount', 'queued_count'], 0),
    runningCount: numberValue(value, ['runningCount', 'running_count'], 0),
    completedCount: numberValue(value, ['completedCount', 'completed_count'], 0),
    failedCount: numberValue(value, ['failedCount', 'failed_count'], 0),
    totalCount: numberValue(value, ['totalCount', 'total_count'], 0),
    activeSourceId: optionalStringValue(value, ['activeSourceId', 'active_source_id']),
    activeHandle: optionalStringValue(value, ['activeHandle', 'active_handle']),
    activeProvider: optionalStringValue(value, ['activeProvider', 'active_provider'])
      ? normalizeProviderKey(
        optionalStringValue(value, ['activeProvider', 'active_provider']),
      )
      : undefined,
    activeStartedAt: optionalStringValue(value, ['activeStartedAt', 'active_started_at']),
    providers: arrayValue(value, ['providers'])
      .map((entry) => normalizeSourceSyncQueueProviderStatus(entry))
      .filter((entry): entry is SourceSyncQueueProviderStatus => entry !== null),
    queuedItems: arrayValue(value, ['queuedItems', 'queued_items'])
      .map((entry) => normalizeSourceSyncQueueItem(entry))
      .filter((entry): entry is SourceSyncQueueItem => entry !== null),
    runningItems: arrayValue(value, ['runningItems', 'running_items'])
      .map((entry) => normalizeSourceSyncQueueItem(entry))
      .filter((entry): entry is SourceSyncQueueItem => entry !== null),
    recentResults: arrayValue(value, ['recentResults', 'recent_results'])
      .map((entry) => normalizeSourceSyncQueueRecentResult(entry))
      .filter((entry): entry is SourceSyncQueueRecentResult => entry !== null),
    updatedAt: stringValue(value, ['updatedAt', 'updated_at'], new Date().toISOString()),
  }
}

function createEmptySourceDeleteQueueStatus(): SourceDeleteQueueStatus {
  return {
    queuedCount: 0,
    runningCount: 0,
    completedCount: 0,
    failedCount: 0,
    totalCount: 0,
    queuedItems: [],
    runningItems: [],
    recentResults: [],
    updatedAt: new Date().toISOString(),
  }
}

function normalizeSourceDeleteQueueJob(value: unknown): SourceDeleteQueueJob | null {
  if (!isRecord(value)) {
    return null
  }

  const progressPercentRaw = pick(value, 'progressPercent', 'progress_percent')
  const progressPercent = typeof progressPercentRaw === 'number' && Number.isFinite(progressPercentRaw)
    ? Math.max(0, Math.min(100, Math.round(progressPercentRaw)))
    : undefined

  return {
    jobId: stringValue(value, ['jobId', 'job_id']),
    sourceId: stringValue(value, ['sourceId', 'source_id']),
    provider: normalizeProviderKey(pick(value, 'provider', 'providerKey', 'provider_key')),
    handle: stringValue(value, ['handle']),
    mode: enumValue(pick(value, 'mode'), ['user_only', 'with_media'] as const, 'with_media'),
    state: enumValue(pick(value, 'state'), ['queued', 'running'] as const, 'queued'),
    queuedAt: stringValue(value, ['queuedAt', 'queued_at'], new Date().toISOString()),
    startedAt: optionalStringValue(value, ['startedAt', 'started_at']),
    progressPercent,
    progressLabel: optionalStringValue(value, ['progressLabel', 'progress_label']),
    progressDetail: optionalStringValue(value, ['progressDetail', 'progress_detail']),
    progressIndeterminate: booleanValue(
      value,
      ['progressIndeterminate', 'progress_indeterminate'],
      progressPercent === undefined,
    ),
    filesProcessed: optionalNumberValue(value, ['filesProcessed', 'files_processed']),
    filesTotal: optionalNumberValue(value, ['filesTotal', 'files_total']),
  }
}

function normalizeSourceDeleteQueueRecentResult(value: unknown): SourceDeleteQueueRecentResult | null {
  if (!isRecord(value)) {
    return null
  }

  return {
    jobId: stringValue(value, ['jobId', 'job_id']),
    sourceId: stringValue(value, ['sourceId', 'source_id']),
    provider: normalizeProviderKey(pick(value, 'provider', 'providerKey', 'provider_key')),
    handle: stringValue(value, ['handle']),
    mode: enumValue(pick(value, 'mode'), ['user_only', 'with_media'] as const, 'with_media'),
    status: enumValue(pick(value, 'status'), ['succeeded', 'failed'] as const, 'failed'),
    summary: stringValue(value, ['summary']),
    finishedAt: stringValue(value, ['finishedAt', 'finished_at'], new Date().toISOString()),
    error: optionalStringValue(value, ['error']),
  }
}

function normalizeSourceDeleteQueueStatus(value: unknown): SourceDeleteQueueStatus {
  if (!isRecord(value)) {
    return createEmptySourceDeleteQueueStatus()
  }

  return {
    queuedCount: numberValue(value, ['queuedCount', 'queued_count'], 0),
    runningCount: numberValue(value, ['runningCount', 'running_count'], 0),
    completedCount: numberValue(value, ['completedCount', 'completed_count'], 0),
    failedCount: numberValue(value, ['failedCount', 'failed_count'], 0),
    totalCount: numberValue(value, ['totalCount', 'total_count'], 0),
    activeJobId: optionalStringValue(value, ['activeJobId', 'active_job_id']),
    activeSourceId: optionalStringValue(value, ['activeSourceId', 'active_source_id']),
    activeHandle: optionalStringValue(value, ['activeHandle', 'active_handle']),
    activeProvider: optionalProviderKey(value, ['activeProvider', 'active_provider']),
    activeMode: optionalEnumValue(pick(value, 'activeMode', 'active_mode'), ['user_only', 'with_media'] as const),
    activeStartedAt: optionalStringValue(value, ['activeStartedAt', 'active_started_at']),
    queuedItems: arrayValue(value, ['queuedItems', 'queued_items'])
      .map((entry) => normalizeSourceDeleteQueueJob(entry))
      .filter((entry): entry is SourceDeleteQueueJob => entry !== null),
    runningItems: arrayValue(value, ['runningItems', 'running_items'])
      .map((entry) => normalizeSourceDeleteQueueJob(entry))
      .filter((entry): entry is SourceDeleteQueueJob => entry !== null),
    recentResults: arrayValue(value, ['recentResults', 'recent_results'])
      .map((entry) => normalizeSourceDeleteQueueRecentResult(entry))
      .filter((entry): entry is SourceDeleteQueueRecentResult => entry !== null),
    updatedAt: stringValue(value, ['updatedAt', 'updated_at'], new Date().toISOString()),
  }
}

function normalizeDesktopRuntime(value: unknown): DesktopRuntimeState {
  if (!isRecord(value)) {
    return {
      closeToTray: false,
      silentMode: false,
      trayAvailable: false,
      reportedByBackend: false,
    }
  }

  return {
    closeToTray: booleanValue(value, ['closeToTray', 'close_to_tray']),
    silentMode: booleanValue(value, ['silentMode', 'silent_mode']),
    trayAvailable: booleanValue(value, ['trayAvailable', 'tray_available']),
    reportedByBackend: true,
  }
}

function normalizeRouteActivationPayload(value: unknown): string | undefined {
  if (typeof value === 'string' && value.trim().length > 0) {
    return value.trim()
  }

  if (!isRecord(value)) {
    return undefined
  }

  return optionalStringValue(value, [
    'actionRoute',
    'action_route',
    'route',
    'targetRoute',
    'target_route',
  ])
}

function normalizeSchedulerSet(value: unknown): SchedulerSet | null {
  if (!isRecord(value)) {
    return null
  }

  const id = stringValue(value, ['id'], createId('scheduler'))
  const plans = arrayValue(value, ['plans', 'syncPlans', 'sync_plans'])
    .map((plan) => normalizeSyncPlan(plan, id))
    .filter((plan): plan is SyncPlan => plan !== null)

  return {
    id,
    name: stringValue(value, ['name'], 'Unnamed scheduler set'),
    active: booleanValue(value, ['active', 'isActive', 'is_active'], false),
    plans: sortPlans(plans),
  }
}

function cloneDefaultInstagramSourceSyncOptions(): InstagramSourceSyncOptions {
  return {
    ...DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS,
    extractImageFromVideo: DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.extractImageFromVideo
      ? { ...DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.extractImageFromVideo }
      : undefined,
  }
}

function normalizeInstagramSourceSyncOptions(value: unknown): InstagramSourceSyncOptions {
  if (!isRecord(value)) {
    return cloneDefaultInstagramSourceSyncOptions()
  }

  const baseSections = {
    timeline: booleanValue(
      value,
      ['timeline', 'downloadTimeline', 'download_timeline'],
      DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.timeline,
    ),
    reels: booleanValue(
      value,
      ['reels', 'downloadReels', 'download_reels'],
      DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.reels,
    ),
    stories: booleanValue(
      value,
      ['stories', 'downloadStories', 'download_stories'],
      DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.stories,
    ),
    storiesUser: booleanValue(
      value,
      ['storiesUser', 'stories_user', 'downloadStoriesUser', 'download_stories_user'],
      DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.storiesUser,
    ),
    tagged: booleanValue(
      value,
      ['tagged', 'taggedPosts', 'tagged_posts', 'downloadTaggedPosts', 'download_tagged_posts'],
      DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.tagged,
    ),
  }

  const extractFromVideoValue = pick(value, 'extractImageFromVideo', 'extract_image_from_video')
  const extractFromVideo = isRecord(extractFromVideoValue)
    ? extractFromVideoValue
    : {}

  const specialPath = stringValue(value, ['specialPath', 'special_path']).trim()
  const usernameOverride = stringValue(value, ['usernameOverride', 'username_override']).trim()
  const dateFrom = stringValue(value, ['dateFrom', 'date_from']).trim()
  const dateTo = stringValue(value, ['dateTo', 'date_to']).trim()

  return {
    ...baseSections,
    temporary: booleanValue(value, ['temporary'], DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.temporary),
    favorite: booleanValue(value, ['favorite'], DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.favorite),
    downloadImages: booleanValue(
      value,
      ['downloadImages', 'download_images'],
      DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.downloadImages,
    ),
    downloadVideos: booleanValue(
      value,
      ['downloadVideos', 'download_videos'],
      DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.downloadVideos,
    ),
    getUserMediaOnly: booleanValue(
      value,
      ['getUserMediaOnly', 'get_user_media_only'],
      DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.getUserMediaOnly,
    ),
    missingOnly: booleanValue(
      value,
      ['missingOnly', 'missing_only'],
      DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.missingOnly,
    ),
    dateFrom,
    dateTo,
    verifiedProfile: booleanValue(
      value,
      ['verifiedProfile', 'verified_profile'],
      DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.verifiedProfile,
    ),
    forceUpdateUserName: booleanValue(
      value,
      ['forceUpdateUserName', 'force_update_user_name'],
      DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.forceUpdateUserName,
    ),
    forceUpdateUserInformation: booleanValue(
      value,
      ['forceUpdateUserInformation', 'force_update_user_information'],
      DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.forceUpdateUserInformation,
    ),
    extractImageFromVideo: {
      timeline: booleanValue(
        extractFromVideo,
        ['timeline', 'extractTimeline', 'extract_timeline'],
        DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.extractImageFromVideo?.timeline ?? true,
      ),
      reels: booleanValue(
        extractFromVideo,
        ['reels', 'extractReels', 'extract_reels'],
        DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.extractImageFromVideo?.reels ?? true,
      ),
      stories: booleanValue(
        extractFromVideo,
        ['stories', 'extractStories', 'extract_stories'],
        DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.extractImageFromVideo?.stories ?? true,
      ),
      storiesUser: booleanValue(
        extractFromVideo,
        ['storiesUser', 'stories_user', 'extractStoriesUser', 'extract_stories_user'],
        DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.extractImageFromVideo?.storiesUser ?? true,
      ),
      tagged: booleanValue(
        extractFromVideo,
        ['tagged', 'extractTagged', 'extract_tagged'],
        DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.extractImageFromVideo?.tagged ?? true,
      ),
    },
    placeExtractedImageIntoVideoFolder: booleanValue(
      value,
      ['placeExtractedImageIntoVideoFolder', 'place_extracted_image_into_video_folder'],
      DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.placeExtractedImageIntoVideoFolder,
    ),
    downloadText: booleanValue(
      value,
      ['downloadText', 'download_text'],
      DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.downloadText,
    ),
    downloadTextPosts: booleanValue(
      value,
      ['downloadTextPosts', 'download_text_posts'],
      DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.downloadTextPosts,
    ),
    textSpecialFolder: booleanValue(
      value,
      ['textSpecialFolder', 'text_special_folder'],
      DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.textSpecialFolder,
    ),
    specialPath,
    usernameOverride,
    scriptEnabled: booleanValue(
      value,
      ['scriptEnabled', 'script_enabled'],
      DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.scriptEnabled,
    ),
    script: stringValue(value, ['script'], DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.script ?? ''),
    description: stringValue(
      value,
      ['description'],
      DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.description ?? '',
    ),
    color: stringValue(value, ['color'], DEFAULT_INSTAGRAM_SOURCE_SYNC_OPTIONS.color ?? ''),
  }
}

function normalizeSourceSyncOptions(value: unknown, provider: ProviderKey): SourceSyncOptions {
  if (provider === 'twitter') {
    const twitterValue = isRecord(value) && isRecord(pick(value, 'twitter')) ? pick(value, 'twitter') : value
    return createSourceSyncOptions('twitter', { twitter: normalizeTwitterSourceSyncOptions(twitterValue) })
  }

  if (provider === 'tiktok') {
    const tiktokValue = isRecord(value) && isRecord(pick(value, 'tiktok')) ? pick(value, 'tiktok') : value
    return createSourceSyncOptions('tiktok', { tiktok: normalizeTikTokSourceSyncOptions(tiktokValue) })
  }

  if (provider !== 'instagram') {
    return createSourceSyncOptions(provider)
  }

  if (!isRecord(value)) {
    return createSourceSyncOptions(provider)
  }

  const instagramValue = isRecord(pick(value, 'instagram')) ? pick(value, 'instagram') : value

  return {
    instagram: normalizeInstagramSourceSyncOptions(instagramValue),
  }
}

function normalizeTwitterSourceSyncOptions(value: unknown): TwitterSourceSyncOptions {
  if (!isRecord(value)) {
    return createTwitterSourceSyncOptions()
  }

  return createTwitterSourceSyncOptions({
    mediaModel: booleanValue(value, ['mediaModel', 'media_model'], DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.mediaModel ?? true),
    profileModel: booleanValue(value, ['profileModel', 'profile_model'], DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.profileModel ?? true),
    searchModel: booleanValue(value, ['searchModel', 'search_model'], DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.searchModel ?? false),
    likesModel: booleanValue(value, ['likesModel', 'likes_model'], DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.likesModel ?? false),
    searchUseGraphqlEndpoint: booleanValue(
      value,
      ['searchUseGraphqlEndpoint', 'search_use_graphql_endpoint'],
      DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.searchUseGraphqlEndpoint ?? true,
    ),
    profileUseGraphqlEndpoint: booleanValue(
      value,
      ['profileUseGraphqlEndpoint', 'profile_use_graphql_endpoint'],
      DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.profileUseGraphqlEndpoint ?? true,
    ),
    allowNonUserTweets: booleanValue(
      value,
      ['allowNonUserTweets', 'allow_non_user_tweets'],
      DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.allowNonUserTweets ?? false,
    ),
    abortOnLimit: booleanValue(value, ['abortOnLimit', 'abort_on_limit'], DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.abortOnLimit ?? true),
    downloadAlreadyParsed: booleanValue(
      value,
      ['downloadAlreadyParsed', 'download_already_parsed'],
      DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.downloadAlreadyParsed ?? true,
    ),
    sleepTimerSecs: numberValue(value, ['sleepTimerSecs', 'sleep_timer_secs'], DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.sleepTimerSecs ?? -1),
    sleepTimerBeforeFirstSecs: numberValue(
      value,
      ['sleepTimerBeforeFirstSecs', 'sleep_timer_before_first_secs'],
      DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.sleepTimerBeforeFirstSecs ?? -2,
    ),
    downloadImages: booleanValue(value, ['downloadImages', 'download_images'], DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.downloadImages ?? true),
    downloadVideos: booleanValue(value, ['downloadVideos', 'download_videos'], DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.downloadVideos ?? true),
    downloadGifs: booleanValue(value, ['downloadGifs', 'download_gifs'], DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.downloadGifs ?? true),
    separateVideoFolder: booleanValue(value, ['separateVideoFolder', 'separate_video_folder'], DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.separateVideoFolder ?? true),
    gifsSpecialFolder: stringValue(value, ['gifsSpecialFolder', 'gifs_special_folder'], DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.gifsSpecialFolder ?? ''),
    gifsPrefix: stringValue(value, ['gifsPrefix', 'gifs_prefix'], DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.gifsPrefix ?? 'GIF_'),
    useMd5Comparison: booleanValue(value, ['useMd5Comparison', 'use_md5_comparison'], DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.useMd5Comparison ?? false),
    temporary: booleanValue(value, ['temporary'], DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.temporary ?? false),
    specialPath: stringValue(value, ['specialPath', 'special_path'], DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.specialPath ?? ''),
    description: stringValue(value, ['description'], DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.description ?? ''),
    color: stringValue(value, ['color'], DEFAULT_TWITTER_SOURCE_SYNC_OPTIONS.color ?? ''),
    userIdHint: optionalStringValue(value, ['userIdHint', 'user_id_hint']),
  })
}

function normalizeTikTokSourceSyncOptions(value: unknown): TikTokSourceSyncOptions {
  if (!isRecord(value)) {
    return createTikTokSourceSyncOptions()
  }

  return createTikTokSourceSyncOptions({
    getTimeline: booleanValue(value, ['getTimeline', 'get_timeline'], DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.getTimeline ?? true),
    getStoriesUser: booleanValue(value, ['getStoriesUser', 'get_stories_user'], DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.getStoriesUser ?? false),
    getReposts: booleanValue(value, ['getReposts', 'get_reposts'], DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.getReposts ?? false),
    downloadVideos: booleanValue(value, ['downloadVideos', 'download_videos'], DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.downloadVideos ?? true),
    downloadPhotos: booleanValue(value, ['downloadPhotos', 'download_photos'], DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.downloadPhotos ?? true),
    useNativeTitle: booleanValue(value, ['useNativeTitle', 'use_native_title'], DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.useNativeTitle ?? false),
    addVideoIdToTitle: booleanValue(value, ['addVideoIdToTitle', 'add_video_id_to_title'], DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.addVideoIdToTitle ?? true),
    removeTagsFromTitle: booleanValue(value, ['removeTagsFromTitle', 'remove_tags_from_title'], DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.removeTagsFromTitle ?? false),
    tokkitFileNaming: booleanValue(value, ['tokkitFileNaming', 'tokkit_file_naming'], DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.tokkitFileNaming ?? false),
    useParsedVideoDate: booleanValue(value, ['useParsedVideoDate', 'use_parsed_video_date'], DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.useParsedVideoDate ?? true),
    downloadFromDate: optionalNumberValue(value, ['downloadFromDate', 'download_from_date']),
    downloadToDate: optionalNumberValue(value, ['downloadToDate', 'download_to_date']),
    separateVideoFolder: booleanValue(value, ['separateVideoFolder', 'separate_video_folder'], DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.separateVideoFolder ?? false),
    abortOnLimit: booleanValue(value, ['abortOnLimit', 'abort_on_limit'], DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.abortOnLimit ?? true),
    sleepTimerSecs: numberValue(value, ['sleepTimerSecs', 'sleep_timer_secs'], DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.sleepTimerSecs ?? -1),
    temporary: booleanValue(value, ['temporary'], DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.temporary ?? false),
    specialPath: stringValue(value, ['specialPath', 'special_path'], DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.specialPath ?? ''),
    description: stringValue(value, ['description'], DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.description ?? ''),
    color: stringValue(value, ['color'], DEFAULT_TIKTOK_SOURCE_SYNC_OPTIONS.color ?? ''),
    userIdHint: optionalStringValue(value, ['userIdHint', 'user_id_hint']),
  })
}

function normalizeConnectorRuntimeStatus(value: unknown): ConnectorRuntimeStatus | null {
  if (!isRecord(value)) {
    return null
  }

  const managementMode = enumValue(
    pick(value, 'managementMode', 'management_mode'),
    ['managed', 'custom'],
    'managed',
  )
  const latestVersion = optionalStringValue(value, ['latestVersion', 'latest_version'])
  const activeVersion = stringValue(value, ['activeVersion', 'active_version'], 'unknown')
  const pendingVersion = optionalStringValue(value, ['pendingVersion', 'pending_version'])
  const status = enumValue(
    pick(value, 'status', 'updateStatus', 'update_status'),
    ['up_to_date', 'update_available', 'checking', 'downloading', 'pending_activation', 'custom_override', 'error'],
    'up_to_date',
  )

  return {
    key: stringValue(value, ['key'], createId('connector')),
    displayName: stringValue(value, ['displayName', 'display_name'], 'Connector'),
    managementMode,
    activeVersion,
    bundledVersion: stringValue(value, ['bundledVersion', 'bundled_version'], activeVersion),
    latestVersion,
    updateAvailable: managementMode === 'managed'
      && booleanValue(value, ['updateAvailable', 'update_available'], Boolean(latestVersion && latestVersion !== activeVersion && !pendingVersion)),
    status,
    lastCheckedAt: optionalStringValue(value, ['lastCheckedAt', 'last_checked_at']),
    lastError: optionalStringValue(value, ['lastError', 'last_error']),
    pendingVersion,
    progressPercent: typeof pick(value, 'progressPercent', 'progress_percent') === 'number'
      ? numberValue(value, ['progressPercent', 'progress_percent'])
      : undefined,
    progressDetail: optionalStringValue(value, ['progressDetail', 'progress_detail']),
    customPath: optionalStringValue(value, ['customPath', 'custom_path']),
  }
}

function normalizeImportProblem(value: unknown): ImportProblem | null {
  if (!isRecord(value)) {
    return null
  }

  return {
    severity: enumValue(pick(value, 'severity'), ['warning', 'error'], 'error'),
    code: stringValue(value, ['code'], 'unknown'),
    message: stringValue(value, ['message'], ''),
  }
}

function normalizeImportPreviewProfile(value: unknown): ImportPreviewProfile | null {
  if (!isRecord(value)) {
    return null
  }

  return {
    profileRoot: stringValue(value, ['profileRoot', 'profile_root']),
    userXmlPath: stringValue(value, ['userXmlPath', 'user_xml_path']),
    handle: stringValue(value, ['handle']),
    displayName: stringValue(value, ['displayName', 'display_name'], stringValue(value, ['handle'])),
    accountName: optionalStringValue(value, ['accountName', 'account_name']),
    sourceId: optionalStringValue(value, ['sourceId', 'source_id']),
    sourceDisplayName: optionalStringValue(value, ['sourceDisplayName', 'source_display_name']),
    sourceHandle: optionalStringValue(value, ['sourceHandle', 'source_handle']),
    accountId: optionalStringValue(value, ['accountId', 'account_id']),
    accountDisplayName: optionalStringValue(value, ['accountDisplayName', 'account_display_name']),
    avatarPath: optionalStringValue(value, ['avatarPath', 'avatar_path']),
    alreadyImported: booleanValue(value, ['alreadyImported', 'already_imported']),
    importState: enumValue(
      pick(value, 'importState', 'import_state'),
      ['ready', 'already_imported', 'needs_account_link', 'duplicate_conflict', 'no_media'] as const,
      'ready',
    ),
    fileCount: numberValue(value, ['fileCount', 'file_count']),
    alreadyCatalogedCount: numberValue(value, ['alreadyCatalogedCount', 'already_cataloged_count']),
    newFileCount: numberValue(value, ['newFileCount', 'new_file_count']),
    problems: arrayValue(value, ['problems'])
      .map((entry) => normalizeImportProblem(entry))
      .filter((entry): entry is ImportProblem => entry !== null),
  }
}

function normalizeImportPreviewSummary(value: unknown): ImportPreviewSummary {
  if (!isRecord(value)) {
    return {
      detectedProfiles: 0,
      readyProfiles: 0,
      blockedProfiles: 0,
      alreadyImportedProfiles: 0,
      importableFiles: 0,
    }
  }

  return {
    detectedProfiles: numberValue(value, ['detectedProfiles', 'detected_profiles']),
    readyProfiles: numberValue(value, ['readyProfiles', 'ready_profiles']),
    blockedProfiles: numberValue(value, ['blockedProfiles', 'blocked_profiles']),
    alreadyImportedProfiles: numberValue(value, ['alreadyImportedProfiles', 'already_imported_profiles']),
    importableFiles: numberValue(value, ['importableFiles', 'importable_files']),
  }
}

function normalizeImportProviderDescriptor(value: unknown): ImportProviderDescriptor | null {
  if (!isRecord(value)) {
    return null
  }

  return {
    key: normalizeProviderKey(pick(value, 'key', 'provider', 'providerKey', 'provider_key')),
    displayName: stringValue(value, ['displayName', 'display_name'], 'Provider'),
    description: stringValue(value, ['description'], ''),
  }
}

function normalizeImportRootDescriptor(value: unknown): ImportRootDescriptor | null {
  if (!isRecord(value)) {
    return null
  }

  const path = optionalStringValue(value, ['path'])
  if (!path) {
    return null
  }

  const source = optionalEnumValue(pick(value, 'source'), ['default', 'account', 'manual'] as const) ?? 'manual'

  return {
    path,
    source,
    label: optionalStringValue(value, ['label']) ?? 'Import root',
    removable: booleanValue(value, ['removable'], false),
  }
}

function normalizeImportMethodDescriptor(value: unknown): ImportMethodDescriptor | null {
  if (!isRecord(value)) {
    return null
  }

  return {
    importerId: stringValue(value, ['importerId', 'importer_id']),
    provider: normalizeProviderKey(pick(value, 'provider', 'providerKey', 'provider_key')),
    label: stringValue(value, ['label'], 'Method'),
    description: stringValue(value, ['description'], ''),
  }
}

function normalizeImportPreview(value: unknown): ImportPreview | null {
  if (!isRecord(value)) {
    return null
  }

  return {
    importerId: stringValue(value, ['importerId', 'importer_id']),
    provider: normalizeProviderKey(pick(value, 'provider', 'providerKey', 'provider_key')),
    methodLabel: stringValue(value, ['methodLabel', 'method_label'], 'Import'),
    forceReimport: booleanValue(value, ['forceReimport', 'force_reimport']),
    roots: stringArray(pick(value, 'roots')),
    profiles: arrayValue(value, ['profiles'])
      .map((entry) => normalizeImportPreviewProfile(entry))
      .filter((entry): entry is ImportPreviewProfile => entry !== null),
    summary: normalizeImportPreviewSummary(pick(value, 'summary')),
  }
}

function normalizeImportRunProfileResult(value: unknown): ImportRunProfileResult | null {
  if (!isRecord(value)) {
    return null
  }

  return {
    profileRoot: stringValue(value, ['profileRoot', 'profile_root']),
    handle: stringValue(value, ['handle']),
    status: enumValue(pick(value, 'status'), ['imported', 'skipped', 'failed'], 'failed'),
    sourceId: optionalStringValue(value, ['sourceId', 'source_id']),
    importedMediaCount: numberValue(value, ['importedMediaCount', 'imported_media_count']),
    alreadyCatalogedCount: numberValue(value, ['alreadyCatalogedCount', 'already_cataloged_count']),
    message: stringValue(value, ['message']),
  }
}

function normalizeImportRunResult(value: unknown): ImportRunResult | null {
  if (!isRecord(value)) {
    return null
  }

  return {
    importerId: stringValue(value, ['importerId', 'importer_id']),
    importedProfiles: numberValue(value, ['importedProfiles', 'imported_profiles']),
    skippedProfiles: numberValue(value, ['skippedProfiles', 'skipped_profiles']),
    failedProfiles: numberValue(value, ['failedProfiles', 'failed_profiles']),
    importedMediaCount: numberValue(value, ['importedMediaCount', 'imported_media_count']),
    alreadyCatalogedCount: numberValue(value, ['alreadyCatalogedCount', 'already_cataloged_count']),
    profiles: arrayValue(value, ['profiles'])
      .map((entry) => normalizeImportRunProfileResult(entry))
      .filter((entry): entry is ImportRunProfileResult => entry !== null),
  }
}

function normalizeInstagramNamingLedgerBackfillResult(value: unknown): InstagramNamingLedgerBackfillResult | null {
  if (!isRecord(value)) {
    return null
  }

  return {
    scannedSources: numberValue(value, ['scannedSources', 'scanned_sources']),
    scannedProfiles: numberValue(value, ['scannedProfiles', 'scanned_profiles']),
    scannedFiles: numberValue(value, ['scannedFiles', 'scanned_files']),
    insertedEntries: numberValue(value, ['insertedEntries', 'inserted_entries']),
    updatedEntries: numberValue(value, ['updatedEntries', 'updated_entries']),
    skippedFiles: numberValue(value, ['skippedFiles', 'skipped_files']),
    legacyRecordsTotal: numberValue(value, ['legacyRecordsTotal', 'legacy_records_total']),
    legacyRecordsMatched: numberValue(value, ['legacyRecordsMatched', 'legacy_records_matched']),
    legacyRecordsMissingFiles: numberValue(value, ['legacyRecordsMissingFiles', 'legacy_records_missing_files']),
    backfilledAt: stringValue(value, ['backfilledAt', 'backfilled_at']),
  }
}

function createEmptyImportQueueStatus(): ImportQueueStatus {
  return {
    queuedCount: 0,
    runningCount: 0,
    completedCount: 0,
    failedCount: 0,
    totalCount: 0,
    queuedItems: [],
    runningItems: [],
    recentResults: [],
    updatedAt: new Date().toISOString(),
  }
}

function normalizeImportQueueJob(value: unknown): ImportQueueJob | null {
  if (!isRecord(value)) {
    return null
  }

  return {
    jobId: stringValue(value, ['jobId', 'job_id']),
    importerId: stringValue(value, ['importerId', 'importer_id']),
    provider: normalizeProviderKey(pick(value, 'provider', 'providerKey', 'provider_key')),
    methodLabel: stringValue(value, ['methodLabel', 'method_label'], 'Import'),
    jobKind: enumValue(pick(value, 'jobKind', 'job_kind'), ['preview', 'import', 'backfill'] as const, 'preview'),
    queuedAt: stringValue(value, ['queuedAt', 'queued_at'], new Date().toISOString()),
    startedAt: optionalStringValue(value, ['startedAt', 'started_at']),
    progressPercent: optionalNumberValue(value, ['progressPercent', 'progress_percent']),
    progressLabel: optionalStringValue(value, ['progressLabel', 'progress_label']),
    progressDetail: optionalStringValue(value, ['progressDetail', 'progress_detail']),
    progressIndeterminate: booleanValue(value, ['progressIndeterminate', 'progress_indeterminate'], false),
  }
}

function normalizeImportQueueRecentResult(value: unknown): ImportQueueRecentResult | null {
  if (!isRecord(value)) {
    return null
  }

  return {
    jobId: stringValue(value, ['jobId', 'job_id']),
    importerId: stringValue(value, ['importerId', 'importer_id']),
    provider: normalizeProviderKey(pick(value, 'provider', 'providerKey', 'provider_key')),
    methodLabel: stringValue(value, ['methodLabel', 'method_label'], 'Import'),
    jobKind: enumValue(pick(value, 'jobKind', 'job_kind'), ['preview', 'import', 'backfill'] as const, 'preview'),
    status: enumValue(pick(value, 'status'), ['succeeded', 'failed'] as const, 'failed'),
    summary: stringValue(value, ['summary']),
    finishedAt: stringValue(value, ['finishedAt', 'finished_at'], new Date().toISOString()),
    error: optionalStringValue(value, ['error']),
  }
}

function normalizeImportQueueStatus(value: unknown): ImportQueueStatus {
  if (!isRecord(value)) {
    return createEmptyImportQueueStatus()
  }

  return {
    queuedCount: numberValue(value, ['queuedCount', 'queued_count'], 0),
    runningCount: numberValue(value, ['runningCount', 'running_count'], 0),
    completedCount: numberValue(value, ['completedCount', 'completed_count'], 0),
    failedCount: numberValue(value, ['failedCount', 'failed_count'], 0),
    totalCount: numberValue(value, ['totalCount', 'total_count'], 0),
    activeJobId: optionalStringValue(value, ['activeJobId', 'active_job_id']),
    activeImporterId: optionalStringValue(value, ['activeImporterId', 'active_importer_id']),
    activeProvider: optionalProviderKey(value, ['activeProvider', 'active_provider']),
    activeMethodLabel: optionalStringValue(value, ['activeMethodLabel', 'active_method_label']),
    activeJobKind: optionalEnumValue(pick(value, 'activeJobKind', 'active_job_kind'), ['preview', 'import', 'backfill'] as const),
    activeStartedAt: optionalStringValue(value, ['activeStartedAt', 'active_started_at']),
    queuedItems: arrayValue(value, ['queuedItems', 'queued_items'])
      .map((entry) => normalizeImportQueueJob(entry))
      .filter((entry): entry is ImportQueueJob => entry !== null),
    runningItems: arrayValue(value, ['runningItems', 'running_items'])
      .map((entry) => normalizeImportQueueJob(entry))
      .filter((entry): entry is ImportQueueJob => entry !== null),
    recentResults: arrayValue(value, ['recentResults', 'recent_results'])
      .map((entry) => normalizeImportQueueRecentResult(entry))
      .filter((entry): entry is ImportQueueRecentResult => entry !== null),
    latestPreview: normalizeImportPreview(pick(value, 'latestPreview', 'latest_preview')) ?? undefined,
    latestRunResult: normalizeImportRunResult(pick(value, 'latestRunResult', 'latest_run_result')) ?? undefined,
    latestBackfillResult: normalizeInstagramNamingLedgerBackfillResult(
      pick(value, 'latestBackfillResult', 'latest_backfill_result'),
    ) ?? undefined,
    updatedAt: stringValue(value, ['updatedAt', 'updated_at'], new Date().toISOString()),
  }
}

function normalizeWorkspaceSnapshot(raw: unknown): WorkspaceSnapshot {
  const emptySnapshot = createEmptyWorkspaceSnapshot()

  if (!isRecord(raw)) {
    return structuredClone(emptySnapshot)
  }

  const providerCatalog = normalizeProviderCatalog(
    pick(raw, 'providerCatalog', 'provider_catalog', 'providers', 'providerRegistry', 'provider_registry'),
  )
  const schedulerSets = arrayValue(raw, ['schedulerSets', 'scheduler_sets'])
    .map((entry) => normalizeSchedulerSet(entry))
    .filter((entry): entry is SchedulerSet => entry !== null)
  const schedulerGroups = arrayValue(raw, ['schedulerGroups', 'scheduler_groups'])
    .map((entry) => normalizeSchedulerGroup(entry))
    .filter((entry): entry is SchedulerGroup => entry !== null)

  const normalized: WorkspaceSnapshot = {
    workspaceRoot: stringValue(raw, ['workspaceRoot', 'workspace_root'], emptySnapshot.workspaceRoot),
    dbPath: stringValue(raw, ['dbPath', 'db_path'], emptySnapshot.dbPath),
    mediaRoot: stringValue(raw, ['mediaRoot', 'media_root'], emptySnapshot.mediaRoot),
    desktopRuntime: normalizeDesktopRuntime(pick(raw, 'desktopRuntime', 'desktop_runtime')),
    providerCatalog,
    appSettings: normalizeAppSettings(pick(raw, 'appSettings', 'app_settings')),
    connectorRuntimes: sortByLabel(
      arrayValue(raw, ['connectorRuntimes', 'connector_runtimes'])
        .map((entry) => normalizeConnectorRuntimeStatus(entry))
        .filter((entry): entry is ConnectorRuntimeStatus => entry !== null),
      (entry) => entry.displayName,
    ),
    accounts: sortAccounts(
      arrayValue(raw, ['accounts', 'providerAccounts', 'provider_accounts'])
        .map((entry) => normalizeProviderAccount(entry, providerCatalog))
        .filter((entry): entry is ProviderAccount => entry !== null),
    ),
    accountSessions: sortByLabel(
      arrayValue(raw, ['accountSessions', 'account_sessions', 'providerAccountSessions', 'provider_account_sessions'])
        .map((entry) => normalizeProviderAccountSession(entry))
        .filter((entry): entry is ProviderAccountSession => entry !== null),
      (session) => session.accountId,
    ),
    sources: sortSources(
      arrayValue(raw, ['sources', 'sourceProfiles', 'source_profiles'])
        .map((entry) => normalizeSourceProfile(entry))
        .filter((entry): entry is SourceProfile => entry !== null),
    ),
    sourceSyncRuns: sortByLabel(
      arrayValue(raw, ['sourceSyncRuns', 'source_sync_runs'])
        .map((entry) => normalizeSourceSyncRun(entry))
        .filter((entry): entry is SourceSyncRun => entry !== null),
      (run) => `${run.finishedAt}-${run.id}`,
    ).reverse(),
    accountSyncRuns: sortByLabel(
      arrayValue(raw, ['accountSyncRuns', 'account_sync_runs'])
        .map((entry) => normalizeAccountSyncRun(entry))
        .filter((entry): entry is AccountSyncRun => entry !== null),
      (run) => `${run.finishedAt}-${run.id}`,
    ).reverse(),
    schedulerSets: sortSchedulerSets(schedulerSets),
    schedulerGroups: sortByLabel(schedulerGroups, (group) => `${group.sortIndex}-${group.name}`),
    syncPlanRuns: sortByLabel(
      arrayValue(raw, ['syncPlanRuns', 'sync_plan_runs'])
        .map((entry) => normalizeSyncPlanRun(entry))
        .filter((entry): entry is SyncPlanRun => entry !== null),
      (run) => `${run.finishedAt}-${run.id}`,
    ).reverse(),
    sourceMediaPaths: normalizeStringMap(pick(raw, 'sourceMediaPaths', 'source_media_paths')),
  }

  return normalized
}

function normalizeStringMap(raw: unknown): Record<string, string> {
  if (!isRecord(raw)) {
    return {}
  }
  const result: Record<string, string> = {}
  for (const [key, value] of Object.entries(raw)) {
    if (typeof value === 'string') {
      result[key] = value
    }
  }
  return result
}

async function invokeWorkspaceCommand(
  command: string,
  args?: Record<string, unknown>,
): Promise<WorkspaceSnapshot> {
  const result = await invoke<unknown>(command, args)
  return replaceLocalSnapshot(result)
}

function replaceLocalSnapshot(raw: unknown): WorkspaceSnapshot {
  localSnapshot = normalizeWorkspaceSnapshot(raw)
  return structuredClone(localSnapshot)
}

function sanitizeText(value: string, fallback = ''): string {
  const trimmed = value.trim()
  return trimmed.length > 0 ? trimmed : fallback
}

export async function loadWorkspaceSnapshot(): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand('bootstrap_workspace', undefined)
}

export async function loadSystemShortDatePattern(): Promise<string> {
  const result = await invoke<unknown>('system_short_date_pattern')
  return typeof result === 'string' && result.trim().length > 0
    ? result.trim()
    : 'yyyy-MM-dd'
}

export async function subscribeToDesktopRuntimeEvents(handlers: {
  onSchedulerTick?: () => void
  onWorkspaceSnapshotChanged?: (snapshot: WorkspaceSnapshot) => void
  onRouteActivation?: (actionRoute?: string) => void
  onSourceSyncQueueChanged?: (status: SourceSyncQueueStatus) => void
  onSourceDeleteQueueChanged?: (status: SourceDeleteQueueStatus) => void
  onImportQueueChanged?: (status: ImportQueueStatus) => void
  onConnectorRuntimeChanged?: () => void
  onRuntimeLogAppended?: (entry: RuntimeLogEntry) => void
}): Promise<() => void> {
  const unlisteners = await Promise.all([
    listen(DESKTOP_SCHEDULER_TICK_EVENT_NAME, () => {
      handlers.onSchedulerTick?.()
    }),
    listen(DESKTOP_WORKSPACE_SNAPSHOT_CHANGED_EVENT_NAME, (event) => {
      handlers.onWorkspaceSnapshotChanged?.(replaceLocalSnapshot(event.payload))
    }),
    listen(DESKTOP_CONNECTOR_RUNTIME_CHANGED_EVENT_NAME, () => {
      handlers.onConnectorRuntimeChanged?.()
    }),
    listen(DESKTOP_SOURCE_SYNC_QUEUE_CHANGED_EVENT_NAME, (event) => {
      handlers.onSourceSyncQueueChanged?.(normalizeSourceSyncQueueStatus(event.payload))
    }),
    listen(DESKTOP_SOURCE_DELETE_QUEUE_CHANGED_EVENT_NAME, (event) => {
      handlers.onSourceDeleteQueueChanged?.(normalizeSourceDeleteQueueStatus(event.payload))
    }),
    listen(DESKTOP_IMPORT_QUEUE_CHANGED_EVENT_NAME, (event) => {
      handlers.onImportQueueChanged?.(normalizeImportQueueStatus(event.payload))
    }),
    listen(DESKTOP_RUNTIME_LOG_APPENDED_EVENT_NAME, (event) => {
      const entry = normalizeRuntimeLogEntry(event.payload)
      if (entry) {
        handlers.onRuntimeLogAppended?.(entry)
      }
    }),
    ...DESKTOP_ROUTE_ACTIVATION_EVENT_NAMES.map((eventName) =>
      listen(eventName, (event) => {
        handlers.onRouteActivation?.(normalizeRouteActivationPayload(event.payload))
      }),
    ),
  ])

  return () => {
    for (const unlisten of unlisteners) {
      unlisten()
    }
  }
}

export async function setDesktopSilentMode(enabled: boolean): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand('set_silent_mode', { enabled })
}

export async function checkConnectorUpdates(key?: string): Promise<ConnectorRuntimeStatus[]> {
  const result = await invoke<unknown>('check_connector_updates', key ? { key } : {})
  return Array.isArray(result)
    ? result
        .map((entry) => normalizeConnectorRuntimeStatus(entry))
        .filter((entry): entry is ConnectorRuntimeStatus => entry !== null)
    : []
}

export async function updateConnectorRuntime(key: string): Promise<ConnectorRuntimeStatus[]> {
  const result = await invoke<unknown>('update_connector_runtime', { key })
  return Array.isArray(result)
    ? result
        .map((entry) => normalizeConnectorRuntimeStatus(entry))
        .filter((entry): entry is ConnectorRuntimeStatus => entry !== null)
    : []
}

export async function setConnectorCustomOverride(
  key: string,
  customPath: string,
): Promise<ConnectorRuntimeStatus[]> {
  const result = await invoke<unknown>('set_connector_custom_override', {
    key,
    customPath,
    custom_path: customPath,
  })
  return Array.isArray(result)
    ? result
        .map((entry) => normalizeConnectorRuntimeStatus(entry))
        .filter((entry): entry is ConnectorRuntimeStatus => entry !== null)
    : []
}

export async function clearConnectorCustomOverride(key: string): Promise<ConnectorRuntimeStatus[]> {
  const result = await invoke<unknown>('clear_connector_custom_override', { key })
  return Array.isArray(result)
    ? result
        .map((entry) => normalizeConnectorRuntimeStatus(entry))
        .filter((entry): entry is ConnectorRuntimeStatus => entry !== null)
    : []
}

export async function upsertProviderAccount(draft: ProviderAccountUpsert): Promise<WorkspaceSnapshot> {
  const payload: ProviderAccountUpsert = {
    ...draft,
    displayName: sanitizeText(draft.displayName, 'Unnamed account'),
    capabilities: cleanStringList(draft.capabilities),
  }

  return invokeWorkspaceCommand(
    'upsert_provider_account',
    buildInvokeArgs(payload),
  )
}

export async function deleteProviderAccount(id: string): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'delete_provider_account',
    buildInvokeArgs({ id }, { accountId: id, account_id: id }),
  )
}

export async function cloneProviderAccount(accountId: string): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'clone_provider_account',
    buildInvokeArgs({ accountId }, { accountId, account_id: accountId }),
  )
}

export async function loadProviderAccountCookies(accountId: string): Promise<ProviderAccountCookie[]> {
  const result = await invoke<unknown>(
    'load_provider_account_cookies',
    buildInvokeArgs({ accountId }, { accountId, account_id: accountId }),
  )

  if (!Array.isArray(result)) {
    throw new Error(`Invalid provider account cookie payload for '${accountId}'`)
  }

  return result
    .map((entry) => normalizeProviderAccountCookie(entry))
    .filter((entry): entry is ProviderAccountCookie => entry !== null)
}

export async function saveProviderAccountCookies(
  accountId: string,
  cookies: ProviderAccountCookie[],
): Promise<WorkspaceSnapshot> {
  const payload = {
    accountId: sanitizeText(accountId),
    cookies: cookies.map((cookie) => ({
      ...cookie,
      domain: sanitizeText(cookie.domain),
      name: sanitizeText(cookie.name),
      path: sanitizeText(cookie.path, '/'),
      value: cookie.value,
      expiresAt: cookie.expiresAt?.trim() ? cookie.expiresAt.trim() : undefined,
    })),
  }

  return invokeWorkspaceCommand(
    'save_provider_account_cookies',
    buildInvokeArgs(payload, { accountId: payload.accountId, account_id: payload.accountId }),
  )
}

export async function importProviderAccountCookies(
  draft: ProviderAccountCookieImport,
): Promise<WorkspaceSnapshot> {
  const payload: ProviderAccountCookieImport = {
    accountId: sanitizeText(draft.accountId),
    importFormat: draft.importFormat,
    content: draft.content,
  }

  return invokeWorkspaceCommand(
    'import_provider_account_cookies',
    buildInvokeArgs(payload),
  )
}

export async function clearProviderAccountCookies(accountId: string): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'clear_provider_account_cookies',
    buildInvokeArgs({ accountId }, { accountId, account_id: accountId }),
  )
}

export async function validateProviderAccount(id: string): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'validate_provider_account',
    buildInvokeArgs({ id }, { accountId: id, account_id: id }),
  )
}

export async function loadProviderAccountEditor(accountId: string): Promise<ProviderAccountEditor> {
  const result = await invoke<unknown>(
    'load_provider_account_editor',
    buildInvokeArgs({ accountId }, { accountId, account_id: accountId }),
  )
  const editor = normalizeProviderAccountEditor(result, localSnapshot)
  if (!editor) {
    throw new Error(`Invalid provider account editor payload for '${accountId}'`)
  }

  return editor
}

export async function saveProviderAccountSettings(
  accountId: string,
  values: ProviderAccountSettingValue[],
): Promise<ProviderAccountEditor> {
  const result = await invoke<unknown>(
    'save_provider_account_settings',
    buildInvokeArgs({ accountId, values }, { accountId, account_id: accountId }),
  )
  const editor = normalizeProviderAccountEditor(result, localSnapshot)
  if (!editor) {
    throw new Error(`Invalid provider account editor payload for '${accountId}'`)
  }

  return editor
}

export async function queryRuntimeLogs(input: RuntimeLogQuery): Promise<RuntimeLogEntry[]> {
  const result = await withTimeout(
    invoke<unknown>('query_runtime_logs', buildInvokeArgs(input)),
    'Timed out while querying the runtime log.',
  )
  if (!Array.isArray(result)) {
    throw new Error('Invalid runtime log payload.')
  }

  return result
    .map((entry) => normalizeRuntimeLogEntry(entry))
    .filter((entry): entry is RuntimeLogEntry => entry !== null)
}

export async function loadRuntimeLogContext(): Promise<RuntimeLogContext> {
  const result = await withTimeout(
    invoke<unknown>('load_runtime_log_context'),
    'Timed out while loading runtime log context.',
  )
  const context = normalizeRuntimeLogContext(result)
  if (!context) {
    throw new Error('Invalid runtime log context payload.')
  }

  return context
}

export async function listImportProviders(): Promise<ImportProviderDescriptor[]> {
  const result = await invoke<unknown>('list_import_providers')
  if (!Array.isArray(result)) {
    throw new Error('Invalid import provider payload.')
  }

  return result
    .map((entry) => normalizeImportProviderDescriptor(entry))
    .filter((entry): entry is ImportProviderDescriptor => entry !== null)
}

export async function listImportMethods(provider: ProviderKey): Promise<ImportMethodDescriptor[]> {
  const result = await invoke<unknown>('list_import_methods', { provider })
  if (!Array.isArray(result)) {
    throw new Error('Invalid import method payload.')
  }

  return result
    .map((entry) => normalizeImportMethodDescriptor(entry))
    .filter((entry): entry is ImportMethodDescriptor => entry !== null)
}

export async function listImportRoots(
  importerId: string,
  manualRoots: string[],
  disabledRoots: string[],
): Promise<ImportRootDescriptor[]> {
  const result = await invoke<unknown>('list_import_roots', {
    importerId,
    importer_id: importerId,
    manualRoots,
    manual_roots: manualRoots,
    disabledRoots,
    disabled_roots: disabledRoots,
  })
  if (!Array.isArray(result)) {
    throw new Error('Invalid import root payload.')
  }

  return result
    .map((entry) => normalizeImportRootDescriptor(entry))
    .filter((entry): entry is ImportRootDescriptor => entry !== null)
}

export async function previewImportMethod(
  importerId: string,
  options: ImportPreviewOptions,
): Promise<ImportPreview> {
  const result = await invoke<unknown>('preview_import_method', {
    importerId,
    importer_id: importerId,
      options,
      forceReimport: options.forceReimport,
      force_reimport: options.forceReimport,
      manualRoots: options.manualRoots,
      manual_roots: options.manualRoots,
      disabledRoots: options.disabledRoots,
      disabled_roots: options.disabledRoots,
  })
  const preview = normalizeImportPreview(result)
  if (!preview) {
    throw new Error('Invalid import preview payload.')
  }

  return preview
}

export async function runImportMethod(
  importerId: string,
  input: ImportRunRequest,
): Promise<ImportRunResult> {
  const result = await invoke<unknown>('run_import_method', buildInvokeArgs(input, {
    importerId,
    importer_id: importerId,
  }))
  const runResult = normalizeImportRunResult(result)
  if (!runResult) {
    throw new Error('Invalid import run payload.')
  }

  return runResult
}

export async function pickImportRootFolder(): Promise<string | null> {
  const result = await invoke<unknown>('pick_import_root_folder')
  if (typeof result === 'string') {
    const trimmed = result.trim()
    return trimmed.length > 0 ? trimmed : null
  }

  return null
}

export async function enqueueImportPreview(
  importerId: string,
  options: ImportPreviewOptions,
): Promise<ImportQueueStatus> {
  const result = await invoke<unknown>('enqueue_import_preview', {
    importerId,
    importer_id: importerId,
    options,
    forceReimport: options.forceReimport,
    force_reimport: options.forceReimport,
    manualRoots: options.manualRoots,
    manual_roots: options.manualRoots,
    disabledRoots: options.disabledRoots,
    disabled_roots: options.disabledRoots,
  })
  return normalizeImportQueueStatus(result)
}

export async function enqueueImportRun(
  importerId: string,
  input: ImportRunRequest,
): Promise<ImportQueueStatus> {
  const result = await invoke<unknown>('enqueue_import_run', buildInvokeArgs(input, {
    importerId,
    importer_id: importerId,
  }))
  return normalizeImportQueueStatus(result)
}

export async function enqueueImportBackfill(importerId: string): Promise<ImportQueueStatus> {
  const result = await invoke<unknown>('enqueue_import_backfill', {
    importerId,
    importer_id: importerId,
  })
  return normalizeImportQueueStatus(result)
}

export async function loadImportQueueStatus(): Promise<ImportQueueStatus> {
  const result = await invoke<unknown>('import_queue_status')
  return normalizeImportQueueStatus(result)
}

export async function loadSourceSyncQueueStatus(): Promise<SourceSyncQueueStatus> {
  const result = await withTimeout(
    invoke<unknown>('source_sync_queue_status'),
    'Timed out while loading source sync queue status.',
  )
  return normalizeSourceSyncQueueStatus(result)
}

export async function loadSourceDeleteQueueStatus(): Promise<SourceDeleteQueueStatus> {
  const result = await withTimeout(
    invoke<unknown>('source_delete_queue_status'),
    'Timed out while loading source delete queue status.',
  )
  return normalizeSourceDeleteQueueStatus(result)
}

export async function openRuntimeLogWindow(): Promise<void> {
  await invoke<void>('open_runtime_log_window')
}

export async function openSchedulerWindow(): Promise<void> {
  await invoke<void>('open_scheduler_window')
}

export async function openPlansWindow(intent?: PlanEditorWindowIntent): Promise<void> {
  if (!intent) {
    await invoke<void>('open_plans_window')
    return
  }

  const payload: PlanEditorWindowIntent = {
    mode: intent.mode,
    planId: intent.planId?.trim() || undefined,
    schedulerSetId: intent.schedulerSetId?.trim() || undefined,
  }
  await invoke<void>('open_plans_window', { intent: payload })
}

export async function openSourceSyncQueueWindow(): Promise<void> {
  await invoke<void>('open_source_sync_queue_window')
}

const DESKTOP_PROFILE_VIEW_SOURCE_EVENT_NAME = 'runtime://profile-view-source'

export async function openProfileViewWindow(sourceId: string): Promise<void> {
  await invoke<void>('open_profile_view_window', buildInvokeArgs({ sourceId }, { source_id: sourceId }))
}

function parseSourceMediaGallery(raw: unknown, sourceId: string): SourceMediaGallery {
  const value = isRecord(raw) ? raw : {}
  const posts = Array.isArray(value.posts) ? value.posts : []
  return {
    sourceId: stringValue(value, ['sourceId', 'source_id'], sourceId),
    provider: stringValue(value, ['provider'], 'instagram') as SourceMediaGallery['provider'],
    handle: stringValue(value, ['handle'], ''),
    profileUrl: stringValue(value, ['profileUrl', 'profile_url'], ''),
    posts: posts.filter(isRecord).map((post) => ({
      postId: optionalStringValue(post, ['postId', 'post_id']),
      postUrl: optionalStringValue(post, ['postUrl', 'post_url']),
      capturedAt: optionalNumberValue(post, ['capturedAt', 'captured_at']),
      mediaType: stringValue(post, ['mediaType', 'media_type'], 'image') as MediaGalleryPost['mediaType'],
      section: stringValue(post, ['section'], 'timeline'),
      posterPath: optionalStringValue(post, ['posterPath', 'poster_path']),
      files: (Array.isArray(post.files) ? post.files : []).filter(isRecord).map((file) => ({
        relativePath: stringValue(file, ['relativePath', 'relative_path'], ''),
        absolutePath: stringValue(file, ['absolutePath', 'absolute_path'], ''),
        mediaType: stringValue(file, ['mediaType', 'media_type'], 'image'),
      })),
    })),
  }
}

export async function loadSourceMediaGallery(sourceId: string): Promise<SourceMediaGallery> {
  const raw = await invoke<unknown>(
    'load_source_media_gallery',
    buildInvokeArgs({ sourceId }, { source_id: sourceId }),
  )
  return parseSourceMediaGallery(raw, sourceId)
}

export async function deleteSourceMedia(
  sourceId: string,
  relativePaths: string[],
): Promise<SourceMediaGallery> {
  const raw = await invoke<unknown>(
    'delete_source_media',
    buildInvokeArgs({ sourceId, relativePaths }, { source_id: sourceId, relative_paths: relativePaths }),
  )
  return parseSourceMediaGallery(raw, sourceId)
}

export async function subscribeToProfileViewSource(
  handler: (sourceId: string) => void,
): Promise<() => void> {
  return listen(DESKTOP_PROFILE_VIEW_SOURCE_EVENT_NAME, (event) => {
    if (typeof event.payload === 'string' && event.payload.trim().length > 0) {
      handler(event.payload)
    }
  })
}

export async function revealMediaInFolder(path: string): Promise<void> {
  if (path.trim().length > 0) {
    await revealItemInDir(path)
  }
}

export async function openMediaFile(path: string): Promise<void> {
  if (path.trim().length > 0) {
    await openPath(path)
  }
}

export async function openConnectorRuntimesWindow(): Promise<void> {
  await invoke<void>('open_connector_runtimes_window')
}

export async function openAccountsWindow(intent?: AccountsWindowIntent): Promise<void> {
  if (!intent) {
    await invoke<void>('open_accounts_window')
    return
  }

  const payload: AccountsWindowIntent = {
    initialAccountId: intent.initialAccountId?.trim() || undefined,
    initialProvider: intent.initialProvider,
    initialMode: intent.initialMode,
  }
  await invoke<void>('open_accounts_window', { intent: payload })
}

export async function openSourceEditorWindow(intent?: SourceEditorWindowIntent): Promise<void> {
  if (!intent) {
    await invoke<void>('open_source_editor_window')
    return
  }

  const seed = intent.seed && intent.seed.handle.trim().length > 0
    ? {
        provider: intent.seed.provider,
        handle: intent.seed.handle.trim(),
        displayName: intent.seed.displayName.trim(),
      }
    : undefined
  const payload: SourceEditorWindowIntent = {
    sourceId: intent.sourceId?.trim() || undefined,
    preferredProvider: intent.preferredProvider,
    preferredAccountId: intent.preferredAccountId?.trim() || undefined,
    seed,
  }
  await invoke<void>('open_source_editor_window', { intent: payload })
}

export async function openProfileEditorWindow(intent?: SourceEditorWindowIntent): Promise<void> {
  await openSourceEditorWindow(intent)
}

export async function closeProfileEditorWindow(): Promise<void> {
  await invoke<void>('close_profile_editor_window')
}

export async function subscribeToAccountsWindowIntent(
  handler: (intent: AccountsWindowIntent) => void,
): Promise<() => void> {
  return listen(DESKTOP_ACCOUNTS_WINDOW_INTENT_EVENT_NAME, (event) => {
    const intent = normalizeAccountsWindowIntent(event.payload)
    if (intent) {
      handler(intent)
    }
  })
}

/**
 * Pede à janela principal que selecione e revele um perfil existente.
 * Usado quando o editor bloqueia a criação de um perfil duplicado e quer
 * trazer o original para o foco do usuário. Também ativa a janela principal.
 */
export async function emitFocusSourceRequest(sourceId: string): Promise<void> {
  const trimmed = sanitizeText(sourceId)
  if (trimmed.length === 0) {
    return
  }

  await emit(DESKTOP_FOCUS_SOURCE_EVENT_NAME, { sourceId: trimmed })
  try {
    await invoke<void>('activate_main_window')
  } catch {
    // A ativação da janela é best-effort; o evento de foco já foi emitido.
  }
}

export async function subscribeToFocusSourceRequest(
  handler: (sourceId: string) => void,
): Promise<() => void> {
  return listen(DESKTOP_FOCUS_SOURCE_EVENT_NAME, (event) => {
    const payload = event.payload as { sourceId?: unknown } | null
    const sourceId =
      payload && typeof payload.sourceId === 'string' ? payload.sourceId.trim() : ''
    if (sourceId.length > 0) {
      handler(sourceId)
    }
  })
}

export async function subscribeToSourceEditorWindowIntent(
  handler: (intent: SourceEditorWindowIntent) => void,
): Promise<() => void> {
  return listen(DESKTOP_SOURCE_EDITOR_WINDOW_INTENT_EVENT_NAME, (event) => {
    const intent = normalizeSourceEditorWindowIntent(event.payload)
    if (intent) {
      handler(intent)
    }
  })
}

export async function subscribeToProfileEditorWindowIntent(
  handler: (intent: SourceEditorWindowIntent) => void,
): Promise<() => void> {
  return listen(DESKTOP_PROFILE_EDITOR_WINDOW_INTENT_EVENT_NAME, (event) => {
    const intent = normalizeSourceEditorWindowIntent(event.payload)
    if (intent) {
      handler(intent)
    }
  })
}

export async function subscribeToPlansWindowIntent(
  handler: (intent: PlanEditorWindowIntent) => void,
): Promise<() => void> {
  return listen(DESKTOP_PLANS_WINDOW_INTENT_EVENT_NAME, (event) => {
    const intent = normalizePlanEditorWindowIntent(event.payload)
    if (intent) {
      handler(intent)
    }
  })
}

export async function openBatchEditorWindow(sourceIds: string[]): Promise<void> {
  await invoke<void>('open_batch_editor_window', { sourceIds })
}

export interface BatchSourceProfilePatch {
  sourceIds: string[]
  labelsToAdd: string[]
  labelsToRemove: string[]
  readyForDownload?: boolean
  syncOptionsPatch?: BatchInstagramSyncOptionsPatch
  setGroupId?: string | null
}

export interface BatchInstagramSyncOptionsPatch {
  timeline?: boolean
  reels?: boolean
  stories?: boolean
  storiesUser?: boolean
  tagged?: boolean
  temporary?: boolean
  favorite?: boolean
  downloadImages?: boolean
  downloadVideos?: boolean
  placeExtractedImageIntoVideoFolder?: boolean
  extractImageFromVideo?: {
    timeline?: boolean
    reels?: boolean
    stories?: boolean
    storiesUser?: boolean
    tagged?: boolean
  }
  getUserMediaOnly?: boolean
  missingOnly?: boolean
  verifiedProfile?: boolean
  forceUpdateUserName?: boolean
  forceUpdateUserInformation?: boolean
  downloadText?: boolean
  downloadTextPosts?: boolean
}

export async function batchUpdateSourceProfiles(
  patch: BatchSourceProfilePatch,
): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand('batch_update_source_profiles', { patch })
}

export async function changeSourceMediaPath(
  sourceIds: string[],
  targetBasePath: string,
  moveMedia: boolean,
): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'change_source_media_path',
    buildInvokeArgs(
      { sourceIds, targetBasePath, moveMedia },
      { source_ids: sourceIds, target_base_path: targetBasePath, move_media: moveMedia },
    ),
  )
}

export interface BatchEditorIntent {
  sourceIds: string[]
}

export async function subscribeToBatchEditorWindowIntent(
  handler: (intent: BatchEditorIntent) => void,
): Promise<() => void> {
  return listen(DESKTOP_BATCH_EDITOR_WINDOW_INTENT_EVENT_NAME, (event) => {
    const payload = event.payload as { sourceIds?: unknown } | undefined
    if (payload && Array.isArray(payload.sourceIds)) {
      handler({ sourceIds: payload.sourceIds as string[] })
    }
  })
}

export async function openImportWindow(): Promise<void> {
  await invoke<void>('open_import_window')
}

export async function reportRuntimeLogWindowReady(): Promise<void> {
  try {
    await invoke('report_runtime_log_window_ready')
  } catch {
    // Ignore telemetry failures; the runtime log window should keep running.
  }
}

export async function reportRuntimeLogWindowBootstrapFailure(message: string): Promise<void> {
  const sanitized = sanitizeText(message)
  if (sanitized.length === 0) {
    return
  }

  try {
    await invoke('report_runtime_log_window_bootstrap_failure', {
      message: sanitized.slice(0, 2048),
    })
  } catch {
    // Ignore telemetry failures; the bootstrap failure is already visible in-window.
  }
}

export async function upsertSourceProfile(draft: SourceProfileUpsert): Promise<WorkspaceSnapshot> {
  const payload: SourceProfileUpsert = {
    ...draft,
    handle: sanitizeText(draft.handle),
    displayName: sanitizeText(draft.displayName, sanitizeText(draft.handle).replace(/^@/, '')),
    accountId: draft.accountId?.trim() ? draft.accountId.trim() : null,
    labels: cleanStringList(draft.labels),
    syncOptions: createSourceSyncOptions(draft.provider, draft.syncOptions),
  }

  return invokeWorkspaceCommand(
    'upsert_source_profile',
    buildInvokeArgs(payload),
  )
}

export async function openExternalTarget(target: string): Promise<void> {
  const sanitizedTarget = sanitizeText(target)
  if (sanitizedTarget.length === 0) {
    return
  }

  await openUrl(sanitizedTarget)
}

export async function deleteSourceProfile(
  id: string,
  mode: SourceProfileDeleteMode,
): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'delete_source_profile',
    buildInvokeArgs({ id, mode }, { sourceId: id, source_id: id }),
  )
}

export async function enqueueSourceDelete(
  id: string,
  mode: SourceProfileDeleteMode,
): Promise<SourceDeleteQueueStatus> {
  const result = await invoke<unknown>(
    'enqueue_source_delete',
    buildInvokeArgs({ id, mode }, { sourceId: id, source_id: id }),
  )
  return normalizeSourceDeleteQueueStatus(result)
}

export async function pickSourceProfileImage(sourceId: string): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'pick_source_profile_image',
    buildInvokeArgs({ sourceId }, { source_id: sourceId }),
  )
}

export async function resetSourceProfileImage(sourceId: string): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'reset_source_profile_image',
    buildInvokeArgs({ sourceId }, { source_id: sourceId }),
  )
}

export async function runSourceSync(
  id: string,
  options: RunSourceSyncOptions = {},
): Promise<WorkspaceSnapshot> {
  const payload = {
    id,
    trigger: options.trigger?.trim() || undefined,
    runMode: options.runMode,
    syncOptionsOverride: options.syncOptionsOverride,
  }

  return invokeWorkspaceCommand(
    'run_source_sync',
    buildInvokeArgs(payload, { sourceId: id, source_id: id }),
  )
}

export async function checkSourceAvailability(
  sourceIds: string[],
  options: { accountIdOverride?: string } = {},
): Promise<SourceAvailabilityCheckResult> {
  const payload = {
    sourceIds: cleanStringList(sourceIds),
    accountIdOverride: options.accountIdOverride?.trim() || undefined,
  }

  const result = await invoke<unknown>(
    'check_source_availability',
    buildInvokeArgs(payload),
  )
  const normalized = normalizeSourceAvailabilityCheckResult(result)
  if (!normalized) {
    throw new Error('Invalid source availability check payload.')
  }

  return {
    ...normalized,
    snapshot: replaceLocalSnapshot(normalized.snapshot),
  }
}

export async function runInstagramSavedPostsSync(accountId: string): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'run_instagram_saved_posts_sync',
    buildInvokeArgs({ accountId }, { account_id: accountId }),
  )
}

export async function cancelSourceSyncProfile(sourceId: string): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'cancel_source_sync_profile',
    buildInvokeArgs({ sourceId }, { source_id: sourceId }),
  )
}

export async function cancelSourceSyncProvider(provider: ProviderKey): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'cancel_source_sync_provider',
    buildInvokeArgs({ provider }),
  )
}

export async function pauseSourceSyncProvider(provider: ProviderKey): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'pause_source_sync_provider',
    buildInvokeArgs({ provider }),
  )
}

export async function resumeSourceSyncProvider(provider: ProviderKey): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'resume_source_sync_provider',
    buildInvokeArgs({ provider }),
  )
}

export async function reorderSourceSyncProviderQueue(
  provider: ProviderKey,
  orderedSourceIds: string[],
): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'reorder_source_sync_provider_queue',
    buildInvokeArgs(
      { provider, orderedSourceIds },
      { ordered_source_ids: orderedSourceIds },
    ),
  )
}

export async function upsertSchedulerSet(draft: SchedulerSetUpsert): Promise<WorkspaceSnapshot> {
  const payload: SchedulerSetUpsert = {
    ...draft,
    name: sanitizeText(draft.name, 'New scheduler set'),
  }

  return invokeWorkspaceCommand(
    'upsert_scheduler_set',
    buildInvokeArgs(payload),
  )
}

export async function deleteSchedulerSet(id: string): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'delete_scheduler_set',
    buildInvokeArgs({ id }, { schedulerSetId: id, scheduler_set_id: id, setId: id, set_id: id }),
  )
}

export async function upsertSchedulerGroup(draft: SchedulerGroupUpsert): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'upsert_scheduler_group',
    buildInvokeArgs({
      ...draft,
      name: sanitizeText(draft.name, 'New group'),
    }),
  )
}

export async function deleteSchedulerGroup(id: string): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'delete_scheduler_group',
    buildInvokeArgs({ id }, { groupId: id, group_id: id }),
  )
}

export async function upsertSyncPlan(draft: SyncPlanUpsert): Promise<WorkspaceSnapshot> {
  const payload: SyncPlanUpsert = {
    ...draft,
    name: sanitizeText(draft.name, 'New plan'),
    targetFilter: sanitizeText(draft.targetFilter),
  }

  return invokeWorkspaceCommand(
    'upsert_sync_plan',
    buildInvokeArgs(payload),
  )
}

export async function previewSyncPlanTarget(input: SyncPlanTargetPreviewInput): Promise<SyncPlanTargetPreview> {
  const result = await invoke<unknown>(
    'preview_sync_plan_target',
    buildInvokeArgs(input),
  )
  if (!isRecord(result)) {
    return { sourceCount: 0, sources: [] }
  }
  return {
    sourceCount: numberValue(result, ['sourceCount', 'source_count'], 0),
    sources: arrayValue(result, ['sources']).map((entry) => {
      if (!isRecord(entry)) {
        return null
      }
      return {
        id: stringValue(entry, ['id']),
        handle: stringValue(entry, ['handle']),
        provider: normalizeProviderKey(pick(entry, 'provider')),
        labels: stringArray(pick(entry, 'labels')),
        readyForDownload: booleanValue(entry, ['readyForDownload', 'ready_for_download'], false),
        remoteState: enumValue(pick(entry, 'remoteState', 'remote_state'), SCHEDULER_REMOTE_STATES, 'exists'),
        subscription: booleanValue(entry, ['subscription', 'isSubscription', 'is_subscription'], false),
        lastSyncedAt: optionalStringValue(entry, ['lastSyncedAt', 'last_synced_at']),
      }
    }).filter(Boolean) as SyncPlanTargetPreview['sources'],
  }
}

export async function deleteSyncPlan(id: string): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'delete_sync_plan',
    buildInvokeArgs({ id }, { planId: id, plan_id: id }),
  )
}

export async function runSyncPlanNow(input: RunSyncPlanNowInput): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'run_sync_plan_now',
    buildInvokeArgs(input, { planId: input.id, plan_id: input.id }),
  )
}

export async function pauseSyncPlan(id: string): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'pause_sync_plan',
    buildInvokeArgs({ id }, { planId: id, plan_id: id }),
  )
}

export async function resumeSyncPlan(id: string): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'resume_sync_plan',
    buildInvokeArgs({ id }, { planId: id, plan_id: id }),
  )
}

export async function skipSyncPlan(id: string): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'skip_sync_plan',
    buildInvokeArgs({ id }, { planId: id, plan_id: id }),
  )
}

export async function setSyncPlanPause(input: SetSyncPlanPauseInput): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand('set_sync_plan_pause', buildInvokeArgs(input))
}

export async function clearSyncPlanPause(id: string): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'clear_sync_plan_pause',
    buildInvokeArgs({ id }, { planId: id, plan_id: id }),
  )
}

export async function applySyncPlanSkip(input: SkipSyncPlanInput): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand('apply_sync_plan_skip', buildInvokeArgs(input))
}

export async function moveSyncPlan(input: MoveSyncPlanInput): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand('move_sync_plan', buildInvokeArgs(input))
}

export async function cloneSyncPlan(input: CloneSyncPlanInput): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand('clone_sync_plan', buildInvokeArgs(input))
}

export async function openSourceFolder(sourceId: string): Promise<WorkspaceSnapshot> {
  return invokeWorkspaceCommand(
    'open_source_folder',
    buildInvokeArgs(
      { sourceId },
      { source_id: sourceId, id: sourceId },
    ),
  )
}

export async function upsertAppSetting(draft: AppSettingUpsert): Promise<WorkspaceSnapshot> {
  const payload: AppSettingUpsert = {
    ...draft,
    key: sanitizeText(draft.key),
  }

  return invokeWorkspaceCommand(
    'upsert_app_setting',
    buildInvokeArgs(payload),
  )
}
