use chrono::Utc;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use tauri::{AppHandle, Emitter};

use crate::domain::models::{
    ImportPreview, ImportPreviewOptions, ImportQueueJob, ImportQueueRecentResult,
    ImportQueueStatus, ImportRunRequest, ImportRunResult, InstagramNamingLedgerBackfillResult,
    WorkspaceSnapshot,
};
use crate::infrastructure::{desktop_runtime, workspace_repository};

pub const IMPORT_QUEUE_CHANGED_EVENT: &str = "runtime://import-queue-changed";
const RECENT_RESULTS_LIMIT: usize = 30;

#[derive(Clone)]
enum ImportQueuedPayload {
    Preview(ImportPreviewOptions),
    Run(ImportRunRequest),
    Backfill,
}

#[derive(Clone)]
struct ImportQueuedJob {
    job_id: String,
    importer_id: String,
    provider: String,
    method_label: String,
    job_kind: String,
    queued_at: String,
    started_at: Option<String>,
    progress_percent: Option<u32>,
    progress_label: Option<String>,
    progress_detail: Option<String>,
    progress_indeterminate: bool,
    payload: ImportQueuedPayload,
}

#[derive(Default)]
struct ImportRuntimeState {
    queue: VecDeque<ImportQueuedJob>,
    active_job: Option<ImportQueuedJob>,
    worker_running: bool,
    completed_count: u32,
    failed_count: u32,
    recent_results: VecDeque<ImportQueueRecentResult>,
    latest_preview: Option<ImportPreview>,
    latest_run_result: Option<ImportRunResult>,
    latest_backfill_result: Option<InstagramNamingLedgerBackfillResult>,
}

type SharedImportRuntimeState = Arc<Mutex<ImportRuntimeState>>;
type SharedImportRuntimeAppHandle = Arc<Mutex<Option<AppHandle>>>;

fn runtime_state() -> &'static SharedImportRuntimeState {
    static STATE: OnceLock<SharedImportRuntimeState> = OnceLock::new();
    STATE.get_or_init(|| Arc::new(Mutex::new(ImportRuntimeState::default())))
}

fn runtime_app_handle() -> &'static SharedImportRuntimeAppHandle {
    static APP_HANDLE: OnceLock<SharedImportRuntimeAppHandle> = OnceLock::new();
    APP_HANDLE.get_or_init(|| Arc::new(Mutex::new(None)))
}

fn register_runtime_app_handle(app: &AppHandle) {
    if let Ok(mut holder) = runtime_app_handle().lock() {
        *holder = Some(app.clone());
    }
}

fn importer_descriptor(importer_id: &str) -> Result<(String, String), String> {
    match importer_id {
        "instagram.scrawler" => Ok(("instagram".to_string(), "SCrawler".to_string())),
        _ => Err(format!("Unsupported importer '{importer_id}'.")),
    }
}

pub fn enqueue_import_preview(
    app: &AppHandle,
    importer_id: String,
    options: ImportPreviewOptions,
) -> Result<ImportQueueStatus, String> {
    enqueue_job(
        app,
        importer_id,
        "preview",
        Some("Scanning folders".to_string()),
        Some("Scanning legacy folders and matching accounts.".to_string()),
        ImportQueuedPayload::Preview(options),
    )
}

pub fn enqueue_import_run(
    app: &AppHandle,
    importer_id: String,
    input: ImportRunRequest,
) -> Result<ImportQueueStatus, String> {
    enqueue_job(
        app,
        importer_id,
        "import",
        Some("Applying import".to_string()),
        Some("Cataloging reviewed media into the workspace.".to_string()),
        ImportQueuedPayload::Run(input),
    )
}

pub fn enqueue_import_backfill(
    app: &AppHandle,
    importer_id: String,
) -> Result<ImportQueueStatus, String> {
    enqueue_job(
        app,
        importer_id,
        "backfill",
        Some("Reconciling naming ledger".to_string()),
        Some("Scanning imported Instagram media and matching SCrawler XML.".to_string()),
        ImportQueuedPayload::Backfill,
    )
}

pub fn import_queue_status() -> Result<ImportQueueStatus, String> {
    let state = runtime_state()
        .lock()
        .map_err(|_| "Import runtime queue lock is poisoned.".to_string())?;
    Ok(build_queue_status(&state))
}

fn enqueue_job(
    app: &AppHandle,
    importer_id: String,
    job_kind: &str,
    progress_label: Option<String>,
    progress_detail: Option<String>,
    payload: ImportQueuedPayload,
) -> Result<ImportQueueStatus, String> {
    register_runtime_app_handle(app);
    let (provider, method_label) = importer_descriptor(&importer_id)?;

    let should_spawn_worker = {
        let mut state = runtime_state()
            .lock()
            .map_err(|_| "Import runtime queue lock is poisoned.".to_string())?;

        let duplicate_live_job = state
            .active_job
            .as_ref()
            .is_some_and(|job| job.importer_id == importer_id && job.job_kind == job_kind)
            || state
                .queue
                .iter()
                .any(|job| job.importer_id == importer_id && job.job_kind == job_kind);

        if !duplicate_live_job {
            state.queue.push_back(ImportQueuedJob {
                job_id: uuid::Uuid::new_v4().to_string(),
                importer_id: importer_id.clone(),
                provider,
                method_label,
                job_kind: job_kind.to_string(),
                queued_at: Utc::now().to_rfc3339(),
                started_at: None,
                progress_percent: Some(0),
                progress_label,
                progress_detail,
                progress_indeterminate: true,
                payload,
            });
        }

        if state.worker_running {
            false
        } else {
            state.worker_running = true;
            true
        }
    };

    publish_import_status_event(app);

    if should_spawn_worker {
        spawn_worker(app.clone());
    }

    import_queue_status()
}

fn spawn_worker(app: AppHandle) {
    thread::spawn(move || loop {
        let job = match dequeue_next() {
            Ok(Some(job)) => job,
            Ok(None) => {
                publish_import_status_event(&app);
                break;
            }
            Err(error) => {
                eprintln!("import runtime worker failed to dequeue: {error}");
                publish_import_status_event(&app);
                break;
            }
        };
        publish_import_status_event(&app);

        match execute_job(&app, &job) {
            Ok(ImportExecutionResult::Preview(preview)) => {
                finish_job(
                    &job,
                    "succeeded",
                    format!(
                        "Dry-run finished with {} profile(s) across {} root(s).",
                        preview.summary.detected_profiles,
                        preview.roots.len()
                    ),
                    None,
                    Some(preview),
                    None,
                    None,
                );
            }
            Ok(ImportExecutionResult::Run(run_result)) => {
                finish_job(
                    &job,
                    "succeeded",
                    format!(
                        "Imported {} profile(s) and copied {} new media file(s).",
                        run_result.imported_profiles, run_result.imported_media_count
                    ),
                    None,
                    None,
                    Some(run_result),
                    None,
                );
                if let Ok(snapshot) = workspace_repository::bootstrap_workspace() {
                    publish_workspace_refresh(&app, &snapshot);
                }
            }
            Ok(ImportExecutionResult::Backfill(backfill_result)) => {
                finish_job(
                    &job,
                    "succeeded",
                    format!(
                        "Backfill finished: {} inserted, {} updated, {} XML entries without matching files.",
                        backfill_result.inserted_entries,
                        backfill_result.updated_entries,
                        backfill_result.legacy_records_missing_files
                    ),
                    None,
                    None,
                    None,
                    Some(backfill_result),
                );
                if let Ok(snapshot) = workspace_repository::bootstrap_workspace() {
                    publish_workspace_refresh(&app, &snapshot);
                }
            }
            Err(error) => {
                finish_job(
                    &job,
                    "failed",
                    match job.job_kind.as_str() {
                        "import" => "Import job failed.".to_string(),
                        "backfill" => "Backfill job failed.".to_string(),
                        _ => "Dry-run job failed.".to_string(),
                    },
                    Some(error),
                    None,
                    None,
                    None,
                );
            }
        }

        publish_import_status_event(&app);
    });
}

enum ImportExecutionResult {
    Preview(ImportPreview),
    Run(ImportRunResult),
    Backfill(InstagramNamingLedgerBackfillResult),
}

fn execute_job(app: &AppHandle, job: &ImportQueuedJob) -> Result<ImportExecutionResult, String> {
    match &job.payload {
        ImportQueuedPayload::Preview(options) => {
            workspace_repository::preview_import_method(job.importer_id.clone(), options.clone())
                .map(ImportExecutionResult::Preview)
        }
        ImportQueuedPayload::Run(input) => {
            workspace_repository::run_import_method(job.importer_id.clone(), input.clone())
                .map(ImportExecutionResult::Run)
        }
        ImportQueuedPayload::Backfill => {
            workspace_repository::run_instagram_media_naming_ledger_backfill(|progress| {
                let percent = if progress.total_sources > 0 {
                    Some(
                        (progress.processed_sources.saturating_mul(100) / progress.total_sources)
                            .min(100),
                    )
                } else {
                    None
                };
                let detail = if let Some(handle) = progress.source_handle.as_deref() {
                    format!(
                        "Profile {}/{} · @{} · {} files scanned.",
                        progress.processed_sources,
                        progress.total_sources.max(1),
                        handle,
                        progress.scanned_files
                    )
                } else {
                    format!(
                        "Profiles {}/{} · {} files scanned.",
                        progress.processed_sources,
                        progress.total_sources.max(1),
                        progress.scanned_files
                    )
                };

                update_active_job_progress(
                    &job.job_id,
                    percent,
                    Some("Reconciling naming ledger".to_string()),
                    Some(detail),
                    progress.total_sources == 0,
                );
                publish_import_status_event(app);
            })
            .map(ImportExecutionResult::Backfill)
        }
    }
}

fn update_active_job_progress(
    job_id: &str,
    progress_percent: Option<u32>,
    progress_label: Option<String>,
    progress_detail: Option<String>,
    progress_indeterminate: bool,
) {
    if let Ok(mut state) = runtime_state().lock() {
        if let Some(active_job) = state.active_job.as_mut() {
            if active_job.job_id == job_id {
                active_job.progress_percent = progress_percent;
                active_job.progress_label = progress_label;
                active_job.progress_detail = progress_detail;
                active_job.progress_indeterminate = progress_indeterminate;
            }
        }
    }
}

fn dequeue_next() -> Result<Option<ImportQueuedJob>, String> {
    let mut state = runtime_state()
        .lock()
        .map_err(|_| "Import runtime queue lock is poisoned.".to_string())?;

    match state.queue.pop_front() {
        Some(mut job) => {
            job.started_at = Some(Utc::now().to_rfc3339());
            state.active_job = Some(job.clone());
            Ok(Some(job))
        }
        None => {
            state.active_job = None;
            state.worker_running = false;
            Ok(None)
        }
    }
}

fn finish_job(
    job: &ImportQueuedJob,
    status: &str,
    summary: String,
    error: Option<String>,
    latest_preview: Option<ImportPreview>,
    latest_run_result: Option<ImportRunResult>,
    latest_backfill_result: Option<InstagramNamingLedgerBackfillResult>,
) {
    if let Ok(mut state) = runtime_state().lock() {
        if status == "failed" {
            state.failed_count += 1;
        } else {
            state.completed_count += 1;
        }

        if let Some(preview) = latest_preview {
            state.latest_preview = Some(preview);
        }

        if let Some(run_result) = latest_run_result {
            state.latest_run_result = Some(run_result);
        }
        if let Some(backfill_result) = latest_backfill_result {
            state.latest_backfill_result = Some(backfill_result);
        }

        state.recent_results.push_front(ImportQueueRecentResult {
            job_id: job.job_id.clone(),
            importer_id: job.importer_id.clone(),
            provider: job.provider.clone(),
            method_label: job.method_label.clone(),
            job_kind: job.job_kind.clone(),
            status: status.to_string(),
            summary,
            finished_at: Utc::now().to_rfc3339(),
            error,
        });

        while state.recent_results.len() > RECENT_RESULTS_LIMIT {
            state.recent_results.pop_back();
        }

        state.active_job = None;
    }
}

fn publish_workspace_refresh(app: &AppHandle, snapshot: &WorkspaceSnapshot) {
    let _ = desktop_runtime::publish_workspace_runtime(app, snapshot);
}

fn publish_import_status_event(app: &AppHandle) {
    if let Ok(status) = import_queue_status() {
        let _ = app.emit(IMPORT_QUEUE_CHANGED_EVENT, status);
    }
}

fn build_queue_status(state: &ImportRuntimeState) -> ImportQueueStatus {
    let queued_items = state
        .queue
        .iter()
        .map(|job| ImportQueueJob {
            job_id: job.job_id.clone(),
            importer_id: job.importer_id.clone(),
            provider: job.provider.clone(),
            method_label: job.method_label.clone(),
            job_kind: job.job_kind.clone(),
            queued_at: job.queued_at.clone(),
            started_at: None,
            progress_percent: job.progress_percent,
            progress_label: None,
            progress_detail: None,
            progress_indeterminate: false,
        })
        .collect::<Vec<_>>();

    let running_items = state
        .active_job
        .as_ref()
        .map(|job| {
            vec![ImportQueueJob {
                job_id: job.job_id.clone(),
                importer_id: job.importer_id.clone(),
                provider: job.provider.clone(),
                method_label: job.method_label.clone(),
                job_kind: job.job_kind.clone(),
                queued_at: job.queued_at.clone(),
                started_at: job.started_at.clone(),
                progress_percent: job.progress_percent,
                progress_label: job.progress_label.clone(),
                progress_detail: job.progress_detail.clone(),
                progress_indeterminate: job.progress_indeterminate,
            }]
        })
        .unwrap_or_default();

    let queued_count = queued_items.len() as u32;
    let running_count = running_items.len() as u32;

    ImportQueueStatus {
        queued_count,
        running_count,
        completed_count: state.completed_count,
        failed_count: state.failed_count,
        total_count: queued_count + running_count + state.completed_count + state.failed_count,
        active_job_id: state.active_job.as_ref().map(|job| job.job_id.clone()),
        active_importer_id: state.active_job.as_ref().map(|job| job.importer_id.clone()),
        active_provider: state.active_job.as_ref().map(|job| job.provider.clone()),
        active_method_label: state
            .active_job
            .as_ref()
            .map(|job| job.method_label.clone()),
        active_job_kind: state.active_job.as_ref().map(|job| job.job_kind.clone()),
        active_started_at: state
            .active_job
            .as_ref()
            .and_then(|job| job.started_at.clone()),
        queued_items,
        running_items,
        recent_results: state.recent_results.iter().cloned().collect(),
        latest_preview: state.latest_preview.clone(),
        latest_run_result: state.latest_run_result.clone(),
        latest_backfill_result: state.latest_backfill_result.clone(),
        updated_at: Utc::now().to_rfc3339(),
    }
}

pub fn clear_runtime_state_for_tests() {
    if let Ok(mut state) = runtime_state().lock() {
        *state = ImportRuntimeState::default();
    }
    if let Ok(mut holder) = runtime_app_handle().lock() {
        *holder = None;
    }
}
