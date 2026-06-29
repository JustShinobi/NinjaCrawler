use crate::domain::models::{
    AccountsWindowIntent, AppSettingUpsert, BatchSourceProfilePatch, CheckSourceAvailabilityInput,
    CloneSyncPlanInput, DesktopRuntimeState, ImportMethodDescriptor, ImportPreview,
    ImportPreviewOptions, ImportProviderDescriptor, ImportQueueStatus, ImportRootDescriptor,
    ImportRunRequest, ImportRunResult, MoveSyncPlanInput, PlanEditorWindowIntent,
    ProviderAccountCookie, ProviderAccountCookieImport, ProviderAccountEditor,
    ProviderAccountSettingValue, ProviderAccountUpsert, RunSourceSyncInput, RunSyncPlanNowInput,
    RuntimeLogContext, RuntimeLogEntry, RuntimeLogQuery, RuntimeLogWindowStatus,
    SchedulerGroupUpsert, SchedulerSetUpsert, SetSyncPlanPauseInput, SkipSyncPlanInput,
    SourceAvailabilityCheckResult, SourceDeleteQueueStatus, SourceEditorWindowIntent,
    SourceProfileDeleteInput, SourceProfileUpsert, SourceSyncQueueStatus, SyncPlanTargetPreview,
    SyncPlanTargetPreviewInput, SyncPlanUpsert, WorkspaceSnapshot,
};
use crate::infrastructure::{
    connector_runtime, desktop_runtime, import_runtime, source_delete_runtime, source_sync_runtime,
    workspace_repository,
};

fn publish_snapshot(
    app: &tauri::AppHandle,
    snapshot: WorkspaceSnapshot,
) -> Result<WorkspaceSnapshot, String> {
    desktop_runtime::publish_workspace_runtime(app, &snapshot)?;
    Ok(snapshot)
}

#[tauri::command]
pub fn bootstrap_workspace(app: tauri::AppHandle) -> Result<WorkspaceSnapshot, String> {
    connector_runtime::register_app_handle(&app);
    let snapshot = publish_snapshot(&app, workspace_repository::bootstrap_workspace()?)?;

    // Migrate profile pictures to Settings/ in background
    let bg_app = app.clone();
    std::thread::spawn(move || {
        if let Ok(updated) = workspace_repository::migrate_profile_pictures_to_settings() {
            let _ = publish_snapshot(&bg_app, updated);
        }
    });

    Ok(snapshot)
}

#[tauri::command]
pub fn upsert_provider_account(
    app: tauri::AppHandle,
    input: ProviderAccountUpsert,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(&app, workspace_repository::upsert_provider_account(input)?)
}

#[tauri::command]
pub fn delete_provider_account(
    app: tauri::AppHandle,
    id: String,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(&app, workspace_repository::delete_provider_account(id)?)
}

#[tauri::command]
pub fn load_provider_account_cookies(
    account_id: String,
) -> Result<Vec<ProviderAccountCookie>, String> {
    workspace_repository::load_provider_account_cookies(account_id)
}

#[tauri::command]
pub fn save_provider_account_cookies(
    app: tauri::AppHandle,
    account_id: String,
    cookies: Vec<ProviderAccountCookie>,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(
        &app,
        workspace_repository::save_provider_account_cookies(account_id, cookies)?,
    )
}

#[tauri::command]
pub fn import_provider_account_cookies(
    app: tauri::AppHandle,
    input: ProviderAccountCookieImport,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(
        &app,
        workspace_repository::import_provider_account_cookies(input)?,
    )
}

#[tauri::command]
pub fn clear_provider_account_cookies(
    app: tauri::AppHandle,
    account_id: String,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(
        &app,
        workspace_repository::clear_provider_account_cookies(account_id)?,
    )
}

#[tauri::command]
pub fn validate_provider_account(
    app: tauri::AppHandle,
    id: String,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(&app, workspace_repository::validate_provider_account(id)?)
}

#[tauri::command]
pub fn query_runtime_logs(input: RuntimeLogQuery) -> Result<Vec<RuntimeLogEntry>, String> {
    workspace_repository::query_runtime_logs(input)
}

#[tauri::command]
pub fn load_runtime_log_context() -> Result<RuntimeLogContext, String> {
    workspace_repository::load_runtime_log_context()
}

#[tauri::command]
pub fn list_import_providers() -> Result<Vec<ImportProviderDescriptor>, String> {
    workspace_repository::list_import_providers()
}

#[tauri::command]
pub fn list_import_methods(provider: String) -> Result<Vec<ImportMethodDescriptor>, String> {
    workspace_repository::list_import_methods(provider)
}

#[tauri::command]
pub fn list_import_roots(
    importer_id: String,
    manual_roots: Vec<String>,
    disabled_roots: Vec<String>,
) -> Result<Vec<ImportRootDescriptor>, String> {
    workspace_repository::list_import_roots(importer_id, manual_roots, disabled_roots)
}

#[tauri::command]
pub fn preview_import_method(
    importer_id: String,
    options: ImportPreviewOptions,
) -> Result<ImportPreview, String> {
    workspace_repository::preview_import_method(importer_id, options)
}

#[tauri::command]
pub fn run_import_method(
    importer_id: String,
    input: ImportRunRequest,
) -> Result<ImportRunResult, String> {
    workspace_repository::run_import_method(importer_id, input)
}

#[tauri::command]
pub fn pick_import_root_folder() -> Result<Option<String>, String> {
    workspace_repository::pick_import_root_folder()
}

#[tauri::command]
pub fn enqueue_import_preview(
    app: tauri::AppHandle,
    importer_id: String,
    options: ImportPreviewOptions,
) -> Result<ImportQueueStatus, String> {
    import_runtime::enqueue_import_preview(&app, importer_id, options)
}

#[tauri::command]
pub fn enqueue_import_run(
    app: tauri::AppHandle,
    importer_id: String,
    input: ImportRunRequest,
) -> Result<ImportQueueStatus, String> {
    import_runtime::enqueue_import_run(&app, importer_id, input)
}

#[tauri::command]
pub fn enqueue_import_backfill(
    app: tauri::AppHandle,
    importer_id: String,
) -> Result<ImportQueueStatus, String> {
    import_runtime::enqueue_import_backfill(&app, importer_id)
}

#[tauri::command]
pub fn import_queue_status() -> Result<ImportQueueStatus, String> {
    import_runtime::import_queue_status()
}

#[tauri::command]
pub fn load_provider_account_editor(account_id: String) -> Result<ProviderAccountEditor, String> {
    workspace_repository::load_provider_account_editor(account_id)
}

#[tauri::command]
pub fn save_provider_account_settings(
    app: tauri::AppHandle,
    account_id: String,
    values: Vec<ProviderAccountSettingValue>,
) -> Result<ProviderAccountEditor, String> {
    let result = workspace_repository::save_provider_account_settings(account_id, values)?;
    desktop_runtime::apply_asset_scope(&app)?;
    Ok(result)
}

#[tauri::command]
pub fn clone_provider_account(
    app: tauri::AppHandle,
    account_id: String,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(
        &app,
        workspace_repository::clone_provider_account(account_id)?,
    )
}

#[tauri::command]
pub fn upsert_source_profile(
    app: tauri::AppHandle,
    input: SourceProfileUpsert,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(&app, workspace_repository::upsert_source_profile(input)?)
}

#[tauri::command]
pub fn batch_update_source_profiles(
    app: tauri::AppHandle,
    patch: BatchSourceProfilePatch,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(
        &app,
        workspace_repository::batch_update_source_profiles(patch)?,
    )
}

#[tauri::command]
pub fn change_source_media_path(
    app: tauri::AppHandle,
    source_ids: Vec<String>,
    target_base_path: String,
    move_media: bool,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(
        &app,
        workspace_repository::change_source_media_path(source_ids, target_base_path, move_media)?,
    )
}

#[tauri::command]
pub fn open_batch_editor_window(
    app: tauri::AppHandle,
    source_ids: Vec<String>,
) -> Result<(), String> {
    desktop_runtime::open_batch_editor_window(&app, source_ids)
}

#[tauri::command]
pub fn delete_source_profile(
    app: tauri::AppHandle,
    input: SourceProfileDeleteInput,
) -> Result<WorkspaceSnapshot, String> {
    let status = source_sync_runtime::source_sync_queue_status()?;
    let blocked = status
        .queued_items
        .iter()
        .chain(status.running_items.iter())
        .any(|item| item.source_id == input.id);

    if blocked {
        return Err(
            "Cannot delete profile while sync is queued or running for this source.".to_string(),
        );
    }

    publish_snapshot(
        &app,
        workspace_repository::delete_source_profile(input.id, input.mode)?,
    )
}

#[tauri::command]
pub fn enqueue_source_delete(
    app: tauri::AppHandle,
    input: SourceProfileDeleteInput,
) -> Result<SourceDeleteQueueStatus, String> {
    source_delete_runtime::enqueue_source_delete(&app, input)
}

#[tauri::command]
pub fn source_delete_queue_status() -> Result<SourceDeleteQueueStatus, String> {
    source_delete_runtime::source_delete_queue_status()
}

#[tauri::command]
pub fn run_source_sync(
    app: tauri::AppHandle,
    input: RunSourceSyncInput,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(&app, source_sync_runtime::enqueue_source_sync(&app, input)?)
}

#[tauri::command]
pub fn check_source_availability(
    input: CheckSourceAvailabilityInput,
) -> Result<SourceAvailabilityCheckResult, String> {
    workspace_repository::check_source_availability(input.source_ids, input.account_id_override)
}

#[tauri::command]
pub fn run_instagram_saved_posts_sync(
    app: tauri::AppHandle,
    account_id: String,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(
        &app,
        workspace_repository::run_instagram_saved_posts_sync(account_id)?,
    )
}

#[tauri::command]
pub fn cancel_source_sync_profile(
    app: tauri::AppHandle,
    source_id: String,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(
        &app,
        source_sync_runtime::cancel_source_sync_profile(&app, source_id)?,
    )
}

#[tauri::command]
pub fn cancel_source_sync_provider(
    app: tauri::AppHandle,
    provider: String,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(
        &app,
        source_sync_runtime::cancel_source_sync_provider(&app, provider)?,
    )
}

#[tauri::command]
pub fn pause_source_sync_provider(
    app: tauri::AppHandle,
    provider: String,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(
        &app,
        source_sync_runtime::pause_source_sync_provider(&app, provider)?,
    )
}

#[tauri::command]
pub fn resume_source_sync_provider(
    app: tauri::AppHandle,
    provider: String,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(
        &app,
        source_sync_runtime::resume_source_sync_provider(&app, provider)?,
    )
}

#[tauri::command]
pub fn reorder_source_sync_provider_queue(
    app: tauri::AppHandle,
    provider: String,
    ordered_source_ids: Vec<String>,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(
        &app,
        source_sync_runtime::reorder_source_sync_provider_queue(&app, provider, ordered_source_ids)?,
    )
}

#[tauri::command]
pub fn source_sync_queue_status() -> Result<SourceSyncQueueStatus, String> {
    source_sync_runtime::source_sync_queue_status()
}

#[tauri::command]
pub fn pick_source_profile_image(
    app: tauri::AppHandle,
    source_id: String,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(
        &app,
        workspace_repository::pick_source_profile_image(source_id)?,
    )
}

#[tauri::command]
pub fn reset_source_profile_image(
    app: tauri::AppHandle,
    source_id: String,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(
        &app,
        workspace_repository::reset_source_profile_image(source_id)?,
    )
}

#[tauri::command]
pub fn upsert_scheduler_set(
    app: tauri::AppHandle,
    input: SchedulerSetUpsert,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(&app, workspace_repository::upsert_scheduler_set(input)?)
}

#[tauri::command]
pub fn delete_scheduler_set(
    app: tauri::AppHandle,
    id: String,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(&app, workspace_repository::delete_scheduler_set(id)?)
}

#[tauri::command]
pub fn upsert_scheduler_group(
    app: tauri::AppHandle,
    input: SchedulerGroupUpsert,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(&app, workspace_repository::upsert_scheduler_group(input)?)
}

#[tauri::command]
pub fn delete_scheduler_group(
    app: tauri::AppHandle,
    id: String,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(&app, workspace_repository::delete_scheduler_group(id)?)
}

#[tauri::command]
pub fn upsert_sync_plan(
    app: tauri::AppHandle,
    input: SyncPlanUpsert,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(&app, workspace_repository::upsert_sync_plan(input)?)
}

#[tauri::command]
pub fn preview_sync_plan_target(
    input: SyncPlanTargetPreviewInput,
) -> Result<SyncPlanTargetPreview, String> {
    workspace_repository::preview_sync_plan_target(input)
}

#[tauri::command]
pub fn delete_sync_plan(app: tauri::AppHandle, id: String) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(&app, workspace_repository::delete_sync_plan(id)?)
}

#[tauri::command]
pub fn run_sync_plan_now(
    app: tauri::AppHandle,
    input: RunSyncPlanNowInput,
) -> Result<WorkspaceSnapshot, String> {
    // O plano apenas resolve as fontes; os downloads são enfileirados na fila
    // sequencial de sync (não rodam inline, para não congelar o app).
    let (snapshot, requests) = workspace_repository::run_sync_plan_now(input)?;
    let mut latest = snapshot;
    for request in requests {
        match source_sync_runtime::enqueue_source_sync(
            &app,
            RunSourceSyncInput {
                id: request.source_id,
                trigger: Some(request.trigger),
                run_mode: None,
                sync_options_override: None,
            },
        ) {
            Ok(updated) => latest = updated,
            Err(error) => eprintln!("failed to enqueue sync-plan source: {error}"),
        }
    }
    publish_snapshot(&app, latest)
}

#[tauri::command]
pub fn pause_sync_plan(app: tauri::AppHandle, id: String) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(&app, workspace_repository::pause_sync_plan(id)?)
}

#[tauri::command]
pub fn resume_sync_plan(app: tauri::AppHandle, id: String) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(&app, workspace_repository::resume_sync_plan(id)?)
}

#[tauri::command]
pub fn skip_sync_plan(app: tauri::AppHandle, id: String) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(&app, workspace_repository::skip_sync_plan(id)?)
}

#[tauri::command]
pub fn set_sync_plan_pause(
    app: tauri::AppHandle,
    input: SetSyncPlanPauseInput,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(&app, workspace_repository::set_sync_plan_pause(input)?)
}

#[tauri::command]
pub fn clear_sync_plan_pause(
    app: tauri::AppHandle,
    id: String,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(&app, workspace_repository::clear_sync_plan_pause(id)?)
}

#[tauri::command]
pub fn apply_sync_plan_skip(
    app: tauri::AppHandle,
    input: SkipSyncPlanInput,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(
        &app,
        workspace_repository::skip_sync_plan_with_input(input)?,
    )
}

#[tauri::command]
pub fn move_sync_plan(
    app: tauri::AppHandle,
    input: MoveSyncPlanInput,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(&app, workspace_repository::move_sync_plan(input)?)
}

#[tauri::command]
pub fn clone_sync_plan(
    app: tauri::AppHandle,
    input: CloneSyncPlanInput,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(&app, workspace_repository::clone_sync_plan(input)?)
}

#[tauri::command]
pub fn open_source_folder(
    app: tauri::AppHandle,
    source_id: String,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(&app, workspace_repository::open_source_folder(source_id)?)
}

#[tauri::command]
pub fn load_source_media_gallery(
    source_id: String,
) -> Result<crate::domain::models::SourceMediaGallery, String> {
    workspace_repository::load_source_media_gallery(source_id)
}

#[tauri::command]
pub fn delete_source_media(
    source_id: String,
    relative_paths: Vec<String>,
) -> Result<crate::domain::models::SourceMediaGallery, String> {
    workspace_repository::delete_source_media(source_id, relative_paths)
}

#[tauri::command]
pub fn upsert_app_setting(
    app: tauri::AppHandle,
    input: AppSettingUpsert,
) -> Result<WorkspaceSnapshot, String> {
    let snapshot = workspace_repository::upsert_app_setting(input)?;
    desktop_runtime::apply_asset_scope(&app)?;
    publish_snapshot(&app, snapshot)
}

#[tauri::command]
pub fn check_connector_updates(
    app: tauri::AppHandle,
    key: Option<String>,
) -> Result<WorkspaceSnapshot, String> {
    connector_runtime::check_connector_updates(key.as_deref())?;
    publish_snapshot(&app, workspace_repository::bootstrap_workspace()?)
}

#[tauri::command]
pub fn update_connector_runtime(
    app: tauri::AppHandle,
    key: String,
) -> Result<WorkspaceSnapshot, String> {
    connector_runtime::update_connector_runtime(&key)?;
    publish_snapshot(&app, workspace_repository::bootstrap_workspace()?)
}

#[tauri::command]
pub fn set_connector_custom_override(
    app: tauri::AppHandle,
    key: String,
    custom_path: String,
) -> Result<WorkspaceSnapshot, String> {
    connector_runtime::set_connector_custom_override(&key, &custom_path)?;
    publish_snapshot(&app, workspace_repository::bootstrap_workspace()?)
}

#[tauri::command]
pub fn clear_connector_custom_override(
    app: tauri::AppHandle,
    key: String,
) -> Result<WorkspaceSnapshot, String> {
    connector_runtime::clear_connector_custom_override(&key)?;
    publish_snapshot(&app, workspace_repository::bootstrap_workspace()?)
}

#[tauri::command]
pub fn desktop_runtime_state() -> Result<DesktopRuntimeState, String> {
    workspace_repository::desktop_runtime_state()
}

#[tauri::command]
pub fn system_short_date_pattern() -> Result<String, String> {
    desktop_runtime::system_short_date_pattern()
}

#[tauri::command]
pub fn set_close_to_tray(
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<WorkspaceSnapshot, String> {
    desktop_runtime::set_close_to_tray(&app, enabled)
}

#[tauri::command]
pub fn set_silent_mode(app: tauri::AppHandle, enabled: bool) -> Result<WorkspaceSnapshot, String> {
    desktop_runtime::set_silent_mode(&app, enabled)
}

#[tauri::command]
pub fn open_runtime_log_window(app: tauri::AppHandle) -> Result<(), String> {
    desktop_runtime::open_runtime_log_window(&app)
}

#[tauri::command]
pub fn open_scheduler_window(app: tauri::AppHandle) -> Result<(), String> {
    desktop_runtime::open_scheduler_window(&app)
}

#[tauri::command]
pub fn open_plans_window(
    app: tauri::AppHandle,
    intent: Option<PlanEditorWindowIntent>,
) -> Result<(), String> {
    desktop_runtime::open_plans_window(&app, intent)
}

#[tauri::command]
pub fn open_source_sync_queue_window(app: tauri::AppHandle) -> Result<(), String> {
    desktop_runtime::open_source_sync_queue_window(&app)
}

#[tauri::command]
pub fn open_profile_view_window(
    app: tauri::AppHandle,
    source_id: String,
) -> Result<(), String> {
    desktop_runtime::open_profile_view_window(&app, source_id)
}

#[tauri::command]
pub fn open_connector_runtimes_window(app: tauri::AppHandle) -> Result<(), String> {
    desktop_runtime::open_connector_runtimes_window(&app)
}

#[tauri::command]
pub fn open_accounts_window(
    app: tauri::AppHandle,
    intent: Option<AccountsWindowIntent>,
) -> Result<(), String> {
    desktop_runtime::open_accounts_window(&app, intent)
}

#[tauri::command]
pub fn open_source_editor_window(
    app: tauri::AppHandle,
    intent: Option<SourceEditorWindowIntent>,
) -> Result<(), String> {
    desktop_runtime::open_source_editor_window(&app, intent)
}

#[tauri::command]
pub fn open_profile_editor_window(
    app: tauri::AppHandle,
    intent: Option<SourceEditorWindowIntent>,
) -> Result<(), String> {
    desktop_runtime::open_profile_editor_window(&app, intent)
}

#[tauri::command]
pub fn close_profile_editor_window(app: tauri::AppHandle) -> Result<(), String> {
    desktop_runtime::close_profile_editor_window(&app)
}

#[tauri::command]
pub fn open_import_window(app: tauri::AppHandle) -> Result<(), String> {
    desktop_runtime::open_import_window(&app)
}

#[tauri::command]
pub fn report_runtime_log_window_ready(
    app: tauri::AppHandle,
) -> Result<RuntimeLogWindowStatus, String> {
    desktop_runtime::report_runtime_log_window_ready(&app)
}

#[tauri::command]
pub fn report_runtime_log_window_bootstrap_failure(
    app: tauri::AppHandle,
    message: String,
) -> Result<RuntimeLogWindowStatus, String> {
    desktop_runtime::report_runtime_log_window_bootstrap_failure(&app, message)
}

#[tauri::command]
pub fn runtime_log_window_status(app: tauri::AppHandle) -> Result<RuntimeLogWindowStatus, String> {
    desktop_runtime::runtime_log_window_status(&app)
}

#[tauri::command]
pub fn activate_main_window(
    app: tauri::AppHandle,
    route: Option<String>,
) -> Result<DesktopRuntimeState, String> {
    desktop_runtime::activate_main_window(&app, route, "command")
}

#[tauri::command]
pub fn hide_main_window(app: tauri::AppHandle) -> Result<DesktopRuntimeState, String> {
    desktop_runtime::hide_main_window(&app)
}

#[tauri::command]
pub fn route_notification_action(
    app: tauri::AppHandle,
    route: String,
) -> Result<DesktopRuntimeState, String> {
    desktop_runtime::route_notification_action(&app, route)
}
