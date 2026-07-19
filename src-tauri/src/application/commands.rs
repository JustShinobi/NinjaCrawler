use crate::domain::models::MigrationStatus;
use crate::domain::models::{
    AccountsWindowIntent, AppBuildInfo, AppSettingUpsert, AppUpdateStatus, BatchSourceProfilePatch,
    CheckSourceAvailabilityInput, CloneSyncPlanInput, ConnectorDebugEntry, ConnectorDebugQuery,
    DesktopRuntimeState, ImportMethodDescriptor, ImportPreview, ImportPreviewOptions,
    ImportProviderDescriptor, ImportQueueStatus, ImportRootDescriptor, ImportRunRequest,
    ImportRunResult, MediaDedupeApplyInput, MediaDedupeJobStatus, MediaDedupeScanInput,
    MoveSyncPlanInput, PlanEditorWindowIntent, ProviderAccountCookie, ProviderAccountCookieImport,
    ProviderAccountEditor, ProviderAccountSettingValue, ProviderAccountUpsert, RunSourceSyncInput,
    RunSyncPlanNowInput, RuntimeLogContext, RuntimeLogEntry, RuntimeLogQuery,
    RuntimeLogWindowIntent, RuntimeLogWindowStatus, SchedulerGroupUpsert, SchedulerSetUpsert,
    SetSyncPlanPauseInput, SkipSyncPlanInput, SourceAvailabilityCheckResult,
    SourceDeleteQueueStatus, SourceEditorWindowIntent, SourceProfileDeleteInput,
    SourceProfileUpsert, SourceSyncQueueStatus, SyncPlanTargetPreview, SyncPlanTargetPreviewInput,
    SyncPlanUpsert, WorkspaceHealthSnapshot, WorkspaceHealthWindowIntent, WorkspaceSnapshot,
};
use crate::infrastructure::{
    app_update, companion_install, connector_debug, connector_runtime, database, desktop_runtime,
    import_runtime, media_dedupe_runtime, media_path_migration_runtime, media_thumbnail_runtime,
    single_video_runtime, source_delete_runtime, source_sync_runtime, storage, workspace_backup,
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
pub fn get_app_build_info() -> AppBuildInfo {
    app_update::build_info()
}

#[tauri::command]
pub async fn check_app_update() -> Result<AppUpdateStatus, String> {
    tauri::async_runtime::spawn_blocking(app_update::check_app_update)
        .await
        .map_err(|error| format!("Update check task failed: {error}"))?
}

#[tauri::command]
pub async fn install_app_update(app: tauri::AppHandle) -> Result<(), String> {
    app_update::install_update(app).await
}

#[tauri::command]
pub fn get_companion_install_status() -> Result<companion_install::CompanionInstallStatus, String> {
    companion_install::managed_install_status()
}

#[tauri::command]
pub async fn install_companion() -> Result<companion_install::CompanionInstallStatus, String> {
    tauri::async_runtime::spawn_blocking(companion_install::install_managed_companion)
        .await
        .map_err(|error| format!("Companion install worker failed: {error}"))?
}

/// Pré-checagem de migrations pendentes (read-only, não roda nada). O frontend
/// chama isto ANTES de qualquer acesso ao banco: se retornar `Some`, mostra a
/// tela de migração; se `None`, o boot já seguiu normal e o app pode carregar.
#[tauri::command]
pub fn get_migration_status() -> Result<Option<MigrationStatus>, String> {
    let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    database::migration_precheck(&layout.db_path)
}

/// Roda o backup + as migrations pendentes com progresso (eventos
/// `migration://progress`), e ao concluir inicia os serviços de runtime que
/// foram adiados no boot. Emite `migration://done` no sucesso e
/// `migration://error` na falha.
#[tauri::command]
pub async fn run_pending_migrations(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Emitter;
    tauri::async_runtime::spawn_blocking(move || {
        let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
        let emitter = app.clone();
        let result =
            database::run_pending_migrations_with_progress(&layout.db_path, move |progress| {
                let _ = emitter.emit("migration://progress", progress);
            });
        match result {
            Ok(()) => {
                desktop_runtime::start_runtime_services(app.clone())?;
                let _ = app.emit("migration://done", ());
                Ok(())
            }
            Err(error) => {
                let _ = app.emit("migration://error", error.clone());
                Err(error)
            }
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Caminho da pasta de backups (o frontend abre no explorer após uma falha de
/// migração, para restaurar manualmente).
#[tauri::command]
pub fn backups_folder_path() -> Result<String, String> {
    let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    let backups_dir = layout.data_dir.join("backups");
    std::fs::create_dir_all(&backups_dir).map_err(|error| error.to_string())?;
    Ok(backups_dir.to_string_lossy().to_string())
}

#[tauri::command]
pub fn open_backups_folder(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;

    let backups_dir = backups_folder_path()?;
    app.opener()
        .open_path(backups_dir, None::<&str>)
        .map_err(|error| format!("Could not open the backups folder: {error}"))
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
pub async fn load_workspace_health() -> Result<WorkspaceHealthSnapshot, String> {
    tauri::async_runtime::spawn_blocking(workspace_repository::load_workspace_health)
        .await
        .map_err(|error| format!("Workspace health task failed: {error}"))?
}

#[tauri::command]
pub fn media_dedupe_status() -> Result<MediaDedupeJobStatus, String> {
    media_dedupe_runtime::media_dedupe_status()
}

#[tauri::command]
pub fn install_media_dedupe_similarity_engine(
    app: tauri::AppHandle,
) -> Result<MediaDedupeJobStatus, String> {
    media_dedupe_runtime::install_similarity_engine(&app)
}

#[tauri::command]
pub fn install_media_tool_runtime(app: tauri::AppHandle) -> Result<MediaDedupeJobStatus, String> {
    media_dedupe_runtime::install_ffmpeg_runtime(&app)
}

#[tauri::command]
pub fn enqueue_media_dedupe_scan(
    app: tauri::AppHandle,
    input: MediaDedupeScanInput,
) -> Result<MediaDedupeJobStatus, String> {
    media_dedupe_runtime::enqueue_scan(&app, input)
}

#[tauri::command]
pub fn cancel_media_dedupe(app: tauri::AppHandle) -> Result<MediaDedupeJobStatus, String> {
    media_dedupe_runtime::cancel(&app)
}

#[tauri::command]
pub fn apply_media_dedupe(
    app: tauri::AppHandle,
    input: MediaDedupeApplyInput,
) -> Result<MediaDedupeJobStatus, String> {
    media_dedupe_runtime::enqueue_apply(&app, input)
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
pub fn revert_provider_account_import(
    app: tauri::AppHandle,
    account_id: String,
) -> Result<WorkspaceSnapshot, String> {
    publish_snapshot(
        &app,
        workspace_repository::revert_provider_account_import(account_id)?,
    )
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
pub fn query_connector_debug(input: ConnectorDebugQuery) -> Vec<ConnectorDebugEntry> {
    connector_debug::query(input)
}

#[tauri::command]
pub fn clear_connector_debug() {
    connector_debug::clear();
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
pub fn enqueue_source_media_path_migration(
    app: tauri::AppHandle,
    source_ids: Vec<String>,
    target_base_path: String,
) -> Result<crate::domain::models::MediaPathMigrationQueueStatus, String> {
    if source_ids
        .iter()
        .any(|source_id| media_dedupe_runtime::is_source_locked(source_id))
    {
        return Err(
            "Media cleanup is currently applying changes to one or more selected profiles."
                .to_string(),
        );
    }
    media_path_migration_runtime::enqueue(&app, source_ids, target_base_path)
}

#[tauri::command]
pub fn media_path_migration_queue_status(
) -> Result<crate::domain::models::MediaPathMigrationQueueStatus, String> {
    media_path_migration_runtime::status()
}

#[tauri::command]
pub fn cancel_media_path_migrations(
    app: tauri::AppHandle,
) -> Result<crate::domain::models::MediaPathMigrationQueueStatus, String> {
    media_path_migration_runtime::cancel_all(&app)
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
    if media_dedupe_runtime::is_source_locked(&input.id) {
        return Err(
            "Cannot delete this profile while media cleanup is applying changes.".to_string(),
        );
    }
    if media_path_migration_runtime::is_source_migrating(&input.id) {
        return Err("This profile has a media-path migration queued or running.".to_string());
    }
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
    if media_dedupe_runtime::is_source_locked(&input.id) {
        return Err(
            "Cannot delete this profile while media cleanup is applying changes.".to_string(),
        );
    }
    if media_path_migration_runtime::is_source_migrating(&input.id) {
        return Err("This profile has a media-path migration queued or running.".to_string());
    }
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
    if media_dedupe_runtime::is_source_locked(&input.id) {
        return Err(
            "Cannot sync this profile while media cleanup is applying changes.".to_string(),
        );
    }
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
        source_sync_runtime::reorder_source_sync_provider_queue(
            &app,
            provider,
            ordered_source_ids,
        )?,
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

// Comandos read-only pesados de disco: varredura de galeria e geração de
// thumbnails (ffmpeg/decode de imagem). Rodam em `spawn_blocking` para o I/O
// não prender os workers do runtime e atrasar outros `invoke` (ex.: durante o
// scroll de um perfil grande num volume lento).
#[tauri::command]
pub async fn load_source_media_gallery(
    source_id: String,
) -> Result<crate::domain::models::SourceMediaGallery, String> {
    tauri::async_runtime::spawn_blocking(move || {
        workspace_repository::load_source_media_gallery(source_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn load_media_thumbnails(
    paths: Vec<String>,
) -> Result<crate::domain::models::MediaThumbnailBatch, String> {
    tauri::async_runtime::spawn_blocking(move || workspace_repository::load_media_thumbnails(paths))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn load_avatar_thumbnails(
    source_ids: Option<Vec<String>>,
) -> Result<crate::domain::models::AvatarThumbnailBatch, String> {
    tauri::async_runtime::spawn_blocking(move || {
        workspace_repository::load_avatar_thumbnails(source_ids)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn enqueue_media_thumbnail_generation(
    source_ids: Vec<String>,
) -> Result<crate::domain::models::MediaThumbnailQueueStatus, String> {
    media_thumbnail_runtime::enqueue(source_ids)
}

#[tauri::command]
pub fn media_thumbnail_queue_status(
) -> Result<crate::domain::models::MediaThumbnailQueueStatus, String> {
    media_thumbnail_runtime::queue_status()
}

#[tauri::command]
pub fn enqueue_single_video_download(
    app: tauri::AppHandle,
    url: String,
) -> Result<crate::domain::models::SingleVideoQueueStatus, String> {
    single_video_runtime::enqueue_single_video(&app, url)
}

#[tauri::command]
pub fn single_video_queue_status() -> Result<crate::domain::models::SingleVideoQueueStatus, String>
{
    single_video_runtime::single_video_queue_status()
}

#[tauri::command]
pub fn list_single_videos() -> Result<Vec<crate::domain::models::SingleVideo>, String> {
    workspace_repository::list_single_videos()
}

#[tauri::command]
pub fn delete_single_video(id: String) -> Result<Vec<crate::domain::models::SingleVideo>, String> {
    workspace_repository::delete_single_video(id)
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
pub async fn prepare_connector_runtimes(
    app: tauri::AppHandle,
) -> Result<WorkspaceSnapshot, String> {
    tauri::async_runtime::spawn_blocking(connector_runtime::prepare_connector_runtimes)
        .await
        .map_err(|error| format!("Connector preparation worker failed: {error}"))??;
    publish_snapshot(&app, workspace_repository::bootstrap_workspace()?)
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
pub fn open_runtime_log_window(
    app: tauri::AppHandle,
    intent: Option<RuntimeLogWindowIntent>,
) -> Result<(), String> {
    desktop_runtime::open_runtime_log_window(&app, intent)
}

#[tauri::command]
pub fn open_connector_debug_window(app: tauri::AppHandle) -> Result<(), String> {
    desktop_runtime::open_connector_debug_window(&app)
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
pub fn open_workspace_health_window(
    app: tauri::AppHandle,
    intent: Option<WorkspaceHealthWindowIntent>,
) -> Result<(), String> {
    desktop_runtime::open_workspace_health_window(&app, intent)
}

#[tauri::command]
pub fn open_profile_view_window(app: tauri::AppHandle, source_id: String) -> Result<(), String> {
    desktop_runtime::open_profile_view_window(&app, source_id)
}

#[tauri::command]
pub fn open_connector_runtimes_window(app: tauri::AppHandle) -> Result<(), String> {
    desktop_runtime::open_connector_runtimes_window(&app)
}

#[tauri::command]
pub fn open_single_videos_window(app: tauri::AppHandle) -> Result<(), String> {
    desktop_runtime::open_single_videos_window(&app)
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
pub fn export_workspace_backup(
    include_secrets: bool,
    password: Option<String>,
) -> Result<workspace_backup::BackupExportResult, String> {
    workspace_backup::export_workspace_backup(include_secrets, password)
}

#[tauri::command]
pub fn inspect_workspace_backup() -> Result<workspace_backup::BackupInspection, String> {
    workspace_backup::inspect_workspace_backup()
}

#[tauri::command]
pub fn import_workspace_backup(
    path: String,
    password: Option<String>,
) -> Result<workspace_backup::BackupImportResult, String> {
    workspace_backup::import_workspace_backup(path, password)
}

#[tauri::command]
pub fn route_notification_action(
    app: tauri::AppHandle,
    route: String,
) -> Result<DesktopRuntimeState, String> {
    desktop_runtime::route_notification_action(&app, route)
}
