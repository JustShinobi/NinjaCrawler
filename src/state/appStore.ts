import { create } from 'zustand'
import type { AppSection } from '../appSections'
import {
  checkConnectorUpdates as checkConnectorUpdatesCommand,
  clearConnectorCustomOverride as clearConnectorCustomOverrideCommand,
  cancelSourceSyncProfile as cancelSourceSyncProfileCommand,
  clearProviderAccountCookies as clearProviderAccountCookiesCommand,
  clearSyncPlanPause as clearSyncPlanPauseCommand,
  deleteProviderAccount as deleteProviderAccountCommand,
  deleteSchedulerGroup as deleteSchedulerGroupCommand,
  deleteSchedulerSet as deleteSchedulerSetCommand,
  deleteSourceProfile as deleteSourceProfileCommand,
  pickSourceProfileImage as pickSourceProfileImageCommand,
  resetSourceProfileImage as resetSourceProfileImageCommand,
  deleteSyncPlan as deleteSyncPlanCommand,
  openSourceFolder as openSourceFolderCommand,
  cloneProviderAccount as cloneProviderAccountCommand,
  importProviderAccountCookies as importProviderAccountCookiesCommand,
  loadProviderAccountCookies as loadProviderAccountCookiesCommand,
  loadProviderAccountEditor as loadProviderAccountEditorCommand,
  loadWorkspaceSnapshot,
  runInstagramSavedPostsSync as runInstagramSavedPostsSyncCommand,
  runSourceSync as runSourceSyncCommand,
  runSyncPlanNow as runSyncPlanNowCommand,
  saveProviderAccountCookies as saveProviderAccountCookiesCommand,
  setSyncPlanPause as setSyncPlanPauseCommand,
  pauseSyncPlan as pauseSyncPlanCommand,
  previewSyncPlanTarget as previewSyncPlanTargetCommand,
  resumeSyncPlan as resumeSyncPlanCommand,
  setDesktopSilentMode as setDesktopSilentModeCommand,
  saveProviderAccountSettings as saveProviderAccountSettingsCommand,
  setConnectorCustomOverride as setConnectorCustomOverrideCommand,
  applySyncPlanSkip as applySyncPlanSkipCommand,
  skipSyncPlan as skipSyncPlanCommand,
  upsertAppSetting as upsertAppSettingCommand,
  validateProviderAccount as validateProviderAccountCommand,
  revertProviderAccountImport as revertProviderAccountImportCommand,
  upsertProviderAccount as upsertProviderAccountCommand,
  upsertSchedulerGroup as upsertSchedulerGroupCommand,
  upsertSchedulerSet as upsertSchedulerSetCommand,
  upsertSourceProfile as upsertSourceProfileCommand,
  upsertSyncPlan as upsertSyncPlanCommand,
  updateConnectorRuntime as updateConnectorRuntimeCommand,
  moveSyncPlan as moveSyncPlanCommand,
  cloneSyncPlan as cloneSyncPlanCommand,
} from '../bridge/desktop'
import { resolveAppSectionFromActionRoute } from '../features/operator/actionRoutes'
import type {
  AppSettingUpsert,
  ProviderAccountCookie,
  ProviderAccountCookieImport,
  ProviderAccountEditor,
  ProviderAccountSettingValue,
  ProviderAccountUpsert,
  RunSyncPlanNowInput,
  RunSourceSyncOptions,
  SchedulerGroupUpsert,
  SchedulerSetUpsert,
  SetSyncPlanPauseInput,
  SkipSyncPlanInput,
  SourceProfileDeleteMode,
  SourceProfileUpsert,
  SyncPlanTargetPreview,
  SyncPlanTargetPreviewInput,
  SyncPlanUpsert,
  WorkspaceSnapshot,
} from '../domain/models'

const OPERATOR_SILENT_MODE_STORAGE_KEY = 'ninjacrawler.operator.silent-mode'

function readOperatorSilentMode(): boolean {
  if (typeof window === 'undefined') {
    return false
  }

  try {
    return window.localStorage.getItem(OPERATOR_SILENT_MODE_STORAGE_KEY) === 'true'
  } catch {
    return false
  }
}

function persistOperatorSilentMode(value: boolean) {
  if (typeof window === 'undefined') {
    return
  }

  try {
    window.localStorage.setItem(OPERATOR_SILENT_MODE_STORAGE_KEY, String(value))
  } catch {
    // Ignore local storage failures and keep the toggle runtime-local.
  }
}

interface AppStore {
  activeSection: AppSection
  operatorSilentMode: boolean
  snapshot?: WorkspaceSnapshot
  loading: boolean
  error?: string
  pendingCommand?: string
  setActiveSection: (section: AppSection) => void
  routeAction: (actionRoute?: string) => void
  toggleOperatorSilentMode: () => Promise<void>
  clearError: () => void
  bootstrap: () => Promise<WorkspaceSnapshot>
  refreshSnapshot: () => Promise<WorkspaceSnapshot>
  applySnapshot: (snapshot: WorkspaceSnapshot) => void
  upsertProviderAccount: (draft: ProviderAccountUpsert) => Promise<WorkspaceSnapshot>
  cloneProviderAccount: (accountId: string) => Promise<WorkspaceSnapshot>
  deleteProviderAccount: (id: string) => Promise<WorkspaceSnapshot>
  loadProviderAccountCookies: (accountId: string) => Promise<ProviderAccountCookie[]>
  saveProviderAccountCookies: (accountId: string, cookies: ProviderAccountCookie[]) => Promise<WorkspaceSnapshot>
  importProviderAccountCookies: (draft: ProviderAccountCookieImport) => Promise<WorkspaceSnapshot>
  clearProviderAccountCookies: (accountId: string) => Promise<WorkspaceSnapshot>
  loadProviderAccountEditor: (accountId: string) => Promise<ProviderAccountEditor>
  saveProviderAccountSettings: (accountId: string, values: ProviderAccountSettingValue[]) => Promise<ProviderAccountEditor>
  checkConnectorUpdates: (key?: string) => Promise<WorkspaceSnapshot>
  updateConnectorRuntime: (key: string) => Promise<WorkspaceSnapshot>
  setConnectorCustomOverride: (key: string, customPath: string) => Promise<WorkspaceSnapshot>
  clearConnectorCustomOverride: (key: string) => Promise<WorkspaceSnapshot>
  validateProviderAccount: (id: string) => Promise<WorkspaceSnapshot>
  revertProviderAccountImport: (accountId: string) => Promise<WorkspaceSnapshot>
  upsertSourceProfile: (draft: SourceProfileUpsert) => Promise<WorkspaceSnapshot>
  deleteSourceProfile: (id: string, mode: SourceProfileDeleteMode) => Promise<WorkspaceSnapshot>
  cancelSourceSyncProfile: (sourceId: string) => Promise<WorkspaceSnapshot>
  pickSourceProfileImage: (sourceId: string) => Promise<WorkspaceSnapshot>
  resetSourceProfileImage: (sourceId: string) => Promise<WorkspaceSnapshot>
  runSourceSync: (id: string, options?: RunSourceSyncOptions) => Promise<WorkspaceSnapshot>
  runInstagramSavedPostsSync: (accountId: string) => Promise<WorkspaceSnapshot>
  upsertSchedulerGroup: (draft: SchedulerGroupUpsert) => Promise<WorkspaceSnapshot>
  deleteSchedulerGroup: (id: string) => Promise<WorkspaceSnapshot>
  upsertSchedulerSet: (draft: SchedulerSetUpsert) => Promise<WorkspaceSnapshot>
  deleteSchedulerSet: (id: string) => Promise<WorkspaceSnapshot>
  upsertSyncPlan: (draft: SyncPlanUpsert) => Promise<WorkspaceSnapshot>
  previewSyncPlanTarget: (input: SyncPlanTargetPreviewInput) => Promise<SyncPlanTargetPreview>
  deleteSyncPlan: (id: string) => Promise<WorkspaceSnapshot>
  runSyncPlanNow: (input: RunSyncPlanNowInput) => Promise<WorkspaceSnapshot>
  pauseSyncPlan: (id: string) => Promise<WorkspaceSnapshot>
  resumeSyncPlan: (id: string) => Promise<WorkspaceSnapshot>
  skipSyncPlan: (id: string) => Promise<WorkspaceSnapshot>
  setSyncPlanPause: (input: SetSyncPlanPauseInput) => Promise<WorkspaceSnapshot>
  clearSyncPlanPause: (id: string) => Promise<WorkspaceSnapshot>
  applySyncPlanSkip: (input: SkipSyncPlanInput) => Promise<WorkspaceSnapshot>
  moveSyncPlan: (id: string, direction: 'up' | 'down') => Promise<WorkspaceSnapshot>
  cloneSyncPlan: (id: string) => Promise<WorkspaceSnapshot>
  openSourceFolder: (sourceId: string) => Promise<WorkspaceSnapshot>
  upsertAppSetting: (draft: AppSettingUpsert) => Promise<WorkspaceSnapshot>
}

function getErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : 'Unknown workspace error'
}

function resolveOperatorSilentMode(snapshot: WorkspaceSnapshot, fallback: boolean): boolean {
  return snapshot.desktopRuntime.reportedByBackend ? snapshot.desktopRuntime.silentMode : fallback
}

export const useAppStore = create<AppStore>((set) => {
  function applySnapshot(snapshot: WorkspaceSnapshot) {
    set((state) => ({
      snapshot,
      error: undefined,
      operatorSilentMode: resolveOperatorSilentMode(snapshot, state.operatorSilentMode),
    }))
  }

  async function runSnapshotMutation(
    label: string,
    operation: () => Promise<WorkspaceSnapshot>,
  ): Promise<WorkspaceSnapshot> {
    set({ pendingCommand: label, error: undefined })

    try {
      const snapshot = await operation()
      set((state) => ({
        snapshot,
        pendingCommand: undefined,
        operatorSilentMode: resolveOperatorSilentMode(snapshot, state.operatorSilentMode),
      }))
      return snapshot
    } catch (error) {
      const message = getErrorMessage(error)
      set({ error: message, pendingCommand: undefined })
      throw error
    }
  }

  async function runEditorMutation<T>(
    label: string,
    operation: () => Promise<T>,
  ): Promise<T> {
    set({ pendingCommand: label, error: undefined })

    try {
      const result = await operation()
      set({ pendingCommand: undefined })
      return result
    } catch (error) {
      const message = getErrorMessage(error)
      set({ error: message, pendingCommand: undefined })
      throw error
    }
  }

  async function runConnectorMutation(
    label: string,
    operation: () => Promise<unknown>,
  ): Promise<WorkspaceSnapshot> {
    set({ pendingCommand: label, error: undefined })

    try {
      await operation()
      const snapshot = await loadWorkspaceSnapshot()
      set((state) => ({
        snapshot,
        pendingCommand: undefined,
        error: undefined,
        operatorSilentMode: resolveOperatorSilentMode(snapshot, state.operatorSilentMode),
      }))
      return snapshot
    } catch (error) {
      const message = getErrorMessage(error)
      set({ error: message, pendingCommand: undefined })
      throw error
    }
  }

  return {
    activeSection: 'sources',
    operatorSilentMode: readOperatorSilentMode(),
    loading: true,
    setActiveSection: (activeSection) => set({ activeSection }),
    routeAction: (actionRoute) => {
      const targetSection = resolveAppSectionFromActionRoute(actionRoute)
      if (targetSection) {
        set({ activeSection: targetSection })
      }
    },
    toggleOperatorSilentMode: async () => {
      const state = useAppStore.getState()

      if (state.snapshot?.desktopRuntime.reportedByBackend) {
        await runSnapshotMutation('set_silent_mode', () =>
          setDesktopSilentModeCommand(!state.snapshot!.desktopRuntime.silentMode),
        )
        return
      }

      const nextValue = !state.operatorSilentMode
      persistOperatorSilentMode(nextValue)
      set({ operatorSilentMode: nextValue })
    },
    clearError: () => set({ error: undefined }),
    applySnapshot,
    bootstrap: async () => {
      set({ loading: true, error: undefined })
      try {
        const snapshot = await loadWorkspaceSnapshot()
        applySnapshot(snapshot)
        set({ loading: false })
        return snapshot
      } catch (error) {
        const message = getErrorMessage(error)
        set({ loading: false, error: message })
        throw error
      }
    },
    refreshSnapshot: async () => {
      try {
        const snapshot = await loadWorkspaceSnapshot()
        applySnapshot(snapshot)
        return snapshot
      } catch (error) {
        const message = getErrorMessage(error)
        set({ error: message })
        throw error
      }
    },
    upsertProviderAccount: (draft) =>
      runSnapshotMutation('upsert_provider_account', () => upsertProviderAccountCommand(draft)),
    cloneProviderAccount: (accountId) =>
      runSnapshotMutation('clone_provider_account', () => cloneProviderAccountCommand(accountId)),
    deleteProviderAccount: (id) =>
      runSnapshotMutation('delete_provider_account', () => deleteProviderAccountCommand(id)),
    loadProviderAccountCookies: (accountId) =>
      runEditorMutation('load_provider_account_cookies', () => loadProviderAccountCookiesCommand(accountId)),
    saveProviderAccountCookies: (accountId, cookies) =>
      runSnapshotMutation('save_provider_account_cookies', () => saveProviderAccountCookiesCommand(accountId, cookies)),
    importProviderAccountCookies: (draft) =>
      runSnapshotMutation('import_provider_account_cookies', () => importProviderAccountCookiesCommand(draft)),
    clearProviderAccountCookies: (accountId) =>
      runSnapshotMutation('clear_provider_account_cookies', () => clearProviderAccountCookiesCommand(accountId)),
    loadProviderAccountEditor: (accountId) =>
      runEditorMutation('load_provider_account_editor', () => loadProviderAccountEditorCommand(accountId)),
    saveProviderAccountSettings: (accountId, values) =>
      runEditorMutation('save_provider_account_settings', () => saveProviderAccountSettingsCommand(accountId, values)),
    checkConnectorUpdates: (key) =>
      runConnectorMutation('check_connector_updates', () => checkConnectorUpdatesCommand(key)),
    updateConnectorRuntime: (key) =>
      runConnectorMutation('update_connector_runtime', () => updateConnectorRuntimeCommand(key)),
    setConnectorCustomOverride: (key, customPath) =>
      runConnectorMutation('set_connector_custom_override', () => setConnectorCustomOverrideCommand(key, customPath)),
    clearConnectorCustomOverride: (key) =>
      runConnectorMutation('clear_connector_custom_override', () => clearConnectorCustomOverrideCommand(key)),
    validateProviderAccount: (id) =>
      runSnapshotMutation('validate_provider_account', () => validateProviderAccountCommand(id)),
    revertProviderAccountImport: (accountId) =>
      runSnapshotMutation('revert_provider_account_import', () => revertProviderAccountImportCommand(accountId)),
    upsertSourceProfile: (draft) =>
      runSnapshotMutation('upsert_source_profile', () => upsertSourceProfileCommand(draft)),
    deleteSourceProfile: (id, mode) =>
      runSnapshotMutation('delete_source_profile', () => deleteSourceProfileCommand(id, mode)),
    cancelSourceSyncProfile: (sourceId) =>
      runSnapshotMutation('cancel_source_sync_profile', () => cancelSourceSyncProfileCommand(sourceId)),
    pickSourceProfileImage: (sourceId) =>
      runSnapshotMutation('pick_source_profile_image', () => pickSourceProfileImageCommand(sourceId)),
    resetSourceProfileImage: (sourceId) =>
      runSnapshotMutation('reset_source_profile_image', () => resetSourceProfileImageCommand(sourceId)),
    runSourceSync: (id, options) =>
      runSnapshotMutation('run_source_sync', () => runSourceSyncCommand(id, options)),
    runInstagramSavedPostsSync: (accountId) =>
      runSnapshotMutation('run_instagram_saved_posts_sync', () => runInstagramSavedPostsSyncCommand(accountId)),
    upsertSchedulerGroup: (draft) =>
      runSnapshotMutation('upsert_scheduler_group', () => upsertSchedulerGroupCommand(draft)),
    deleteSchedulerGroup: (id) =>
      runSnapshotMutation('delete_scheduler_group', () => deleteSchedulerGroupCommand(id)),
    upsertSchedulerSet: (draft) =>
      runSnapshotMutation('upsert_scheduler_set', () => upsertSchedulerSetCommand(draft)),
    deleteSchedulerSet: (id) =>
      runSnapshotMutation('delete_scheduler_set', () => deleteSchedulerSetCommand(id)),
    upsertSyncPlan: (draft) =>
      runSnapshotMutation('upsert_sync_plan', () => upsertSyncPlanCommand(draft)),
    previewSyncPlanTarget: (input) =>
      runEditorMutation('preview_sync_plan_target', () => previewSyncPlanTargetCommand(input)),
    deleteSyncPlan: (id) =>
      runSnapshotMutation('delete_sync_plan', () => deleteSyncPlanCommand(id)),
    runSyncPlanNow: (input) =>
      runSnapshotMutation('run_sync_plan_now', () => runSyncPlanNowCommand(input)),
    pauseSyncPlan: (id) =>
      runSnapshotMutation('pause_sync_plan', () => pauseSyncPlanCommand(id)),
    resumeSyncPlan: (id) =>
      runSnapshotMutation('resume_sync_plan', () => resumeSyncPlanCommand(id)),
    skipSyncPlan: (id) =>
      runSnapshotMutation('skip_sync_plan', () => skipSyncPlanCommand(id)),
    setSyncPlanPause: (input) =>
      runSnapshotMutation('set_sync_plan_pause', () => setSyncPlanPauseCommand(input)),
    clearSyncPlanPause: (id) =>
      runSnapshotMutation('clear_sync_plan_pause', () => clearSyncPlanPauseCommand(id)),
    applySyncPlanSkip: (input) =>
      runSnapshotMutation('apply_sync_plan_skip', () => applySyncPlanSkipCommand(input)),
    moveSyncPlan: (id, direction) =>
      runSnapshotMutation('move_sync_plan', () => moveSyncPlanCommand({ id, direction })),
    cloneSyncPlan: (id) =>
      runSnapshotMutation('clone_sync_plan', () => cloneSyncPlanCommand({ id })),
    openSourceFolder: (sourceId) =>
      runSnapshotMutation('open_source_folder', () => openSourceFolderCommand(sourceId)),
    upsertAppSetting: (draft) =>
      runSnapshotMutation('upsert_app_setting', () => upsertAppSettingCommand(draft)),
  }
})
