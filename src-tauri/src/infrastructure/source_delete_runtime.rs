use chrono::Utc;
use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use crate::domain::models::{
    SourceDeleteQueueJob, SourceDeleteQueueRecentResult, SourceDeleteQueueStatus,
    SourceProfileDeleteInput, SourceProfileDeleteMode, WorkspaceSnapshot,
};
use crate::infrastructure::{
    desktop_runtime, media_thumbnail_runtime, source_sync_runtime, workspace_repository,
};

pub const SOURCE_DELETE_QUEUE_CHANGED_EVENT: &str = "runtime://source-delete-queue-changed";
const RECENT_RESULTS_LIMIT: usize = 80;

#[derive(Clone)]
struct SourceDeleteQueuedJob {
    job_id: String,
    source_id: String,
    provider: String,
    handle: String,
    mode: SourceProfileDeleteMode,
    queued_at: String,
    started_at: Option<String>,
    progress_percent: Option<u32>,
    progress_label: Option<String>,
    progress_detail: Option<String>,
    progress_indeterminate: bool,
    files_processed: Option<u32>,
    files_total: Option<u32>,
}

#[derive(Default)]
struct SourceDeleteRuntimeState {
    queue: VecDeque<SourceDeleteQueuedJob>,
    queued_ids: HashSet<String>,
    active_job: Option<SourceDeleteQueuedJob>,
    worker_running: bool,
    completed_count: u32,
    failed_count: u32,
    recent_results: VecDeque<SourceDeleteQueueRecentResult>,
}

type SharedDeleteRuntimeState = Arc<Mutex<SourceDeleteRuntimeState>>;
type SharedDeleteRuntimeAppHandle = Arc<Mutex<Option<AppHandle>>>;

fn runtime_state() -> &'static SharedDeleteRuntimeState {
    static STATE: OnceLock<SharedDeleteRuntimeState> = OnceLock::new();
    STATE.get_or_init(|| Arc::new(Mutex::new(SourceDeleteRuntimeState::default())))
}

fn runtime_app_handle() -> &'static SharedDeleteRuntimeAppHandle {
    static APP_HANDLE: OnceLock<SharedDeleteRuntimeAppHandle> = OnceLock::new();
    APP_HANDLE.get_or_init(|| Arc::new(Mutex::new(None)))
}

fn register_runtime_app_handle(app: &AppHandle) {
    if let Ok(mut holder) = runtime_app_handle().lock() {
        *holder = Some(app.clone());
    }
}

fn publish_delete_status_event_from_registered_app() {
    let app = runtime_app_handle()
        .lock()
        .ok()
        .and_then(|holder| holder.as_ref().cloned());

    if let Some(app) = app {
        publish_delete_status_event(&app);
    }
}

pub fn enqueue_source_delete(
    app: &AppHandle,
    input: SourceProfileDeleteInput,
) -> Result<SourceDeleteQueueStatus, String> {
    let source_id = input.id.trim().to_string();
    if source_id.is_empty() {
        return Err("Source id is required.".to_string());
    }

    register_runtime_app_handle(app);
    let seed = workspace_repository::source_delete_queue_item_seed(source_id.clone())?;

    let should_spawn_worker = {
        let mut state = runtime_state()
            .lock()
            .map_err(|_| "Source delete queue lock is poisoned.".to_string())?;

        let already_active = state
            .active_job
            .as_ref()
            .is_some_and(|job| job.source_id == source_id);
        let already_queued = state.queued_ids.contains(&source_id);

        if !already_active && !already_queued {
            state.queue.push_back(SourceDeleteQueuedJob {
                job_id: uuid::Uuid::new_v4().to_string(),
                source_id: seed.source_id,
                provider: seed.provider,
                handle: seed.handle,
                mode: input.mode,
                queued_at: Utc::now().to_rfc3339(),
                started_at: None,
                progress_percent: Some(0),
                progress_label: Some("Queued for delete".to_string()),
                progress_detail: Some("Waiting for the delete worker.".to_string()),
                progress_indeterminate: true,
                files_processed: None,
                files_total: None,
            });
            state.queued_ids.insert(source_id);
        }

        if state.worker_running {
            false
        } else {
            state.worker_running = true;
            true
        }
    };

    publish_delete_status_event(app);

    if should_spawn_worker {
        spawn_worker(app.clone());
    }

    source_delete_queue_status()
}

pub fn source_delete_queue_status() -> Result<SourceDeleteQueueStatus, String> {
    let state = runtime_state()
        .lock()
        .map_err(|_| "Source delete queue lock is poisoned.".to_string())?;
    Ok(build_queue_status(&state))
}

fn spawn_worker(app: AppHandle) {
    thread::spawn(move || loop {
        let job = match dequeue_next() {
            Ok(Some(job)) => job,
            Ok(None) => {
                publish_delete_status_event(&app);
                break;
            }
            Err(error) => {
                eprintln!("source delete worker failed to dequeue: {error}");
                publish_delete_status_event(&app);
                break;
            }
        };
        publish_delete_status_event(&app);

        match execute_job(&app, &job) {
            Ok(snapshot) => {
                // Only mark the queue job succeeded after the full delete + snapshot.
                report_delete_progress(
                    &job.source_id,
                    Some(100),
                    Some("Delete complete".to_string()),
                    Some("Profile removed. Refreshing library…".to_string()),
                    false,
                    None,
                    None,
                );
                finish_job(&job, "succeeded", success_summary(job.mode), None);
                publish_delete_status_event(&app);
                publish_workspace_refresh(&app, &snapshot);
            }
            Err(error) => {
                report_delete_progress(
                    &job.source_id,
                    None,
                    Some("Delete failed".to_string()),
                    Some(error.clone()),
                    false,
                    None,
                    None,
                );
                finish_job(
                    &job,
                    "failed",
                    failure_summary(job.mode),
                    Some(error.clone()),
                );
                publish_delete_status_event(&app);
                if let Ok(snapshot) = workspace_repository::bootstrap_workspace() {
                    publish_workspace_refresh(&app, &snapshot);
                }
            }
        }

        publish_delete_status_event(&app);
    });
}

fn execute_job(app: &AppHandle, job: &SourceDeleteQueuedJob) -> Result<WorkspaceSnapshot, String> {
    if source_sync_is_live(&job.source_id)? {
        report_delete_progress(
            &job.source_id,
            Some(1),
            Some("Cancelling sync".to_string()),
            Some("Stopping queued/running sync for this profile before delete.".to_string()),
            false,
            None,
            None,
        );

        // Do not publish a workspace snapshot here — that made the UI look like
        // the profile was already gone while media delete was still running.
        let _snapshot = source_sync_runtime::cancel_source_sync_profile(app, job.source_id.clone())?;

        wait_for_source_sync_to_clear(&job.source_id)?;
    }

    // Thumbnail workers keep `.thumbs/*.jpg` open on Windows; deleting the
    // profile folder while they run yields ERROR_DIR_NOT_EMPTY (145). Cancel
    // the job (do not wait for a full generation run — that froze the app).
    report_delete_progress(
        &job.source_id,
        Some(2),
        Some("Releasing media locks".to_string()),
        Some("Cancelling thumbnail generation for this profile (brief wait).".to_string()),
        false,
        None,
        None,
    );
    media_thumbnail_runtime::cancel_queued_and_wait(&job.source_id)?;

    report_delete_progress(
        &job.source_id,
        Some(3),
        Some("Starting profile delete".to_string()),
        Some(match job.mode {
            SourceProfileDeleteMode::UserOnly => {
                "Soft-deleting profile; media files stay on disk.".to_string()
            }
            SourceProfileDeleteMode::WithMedia => {
                "Delete-with-media: inventory → disk wipe → database cascade.".to_string()
            }
        }),
        false,
        None,
        None,
    );

    workspace_repository::delete_source_profile_with_progress(
        job.source_id.clone(),
        job.mode,
        |update| {
            report_delete_progress(
                &job.source_id,
                update.progress_percent,
                update.progress_label,
                update.progress_detail,
                update.progress_indeterminate,
                update.files_processed,
                update.files_total,
            );
            Ok(())
        },
    )
}

fn source_sync_is_live(source_id: &str) -> Result<bool, String> {
    let status = source_sync_runtime::source_sync_queue_status()?;
    Ok(status
        .queued_items
        .iter()
        .chain(status.running_items.iter())
        .any(|item| item.source_id == source_id))
}

fn wait_for_source_sync_to_clear(source_id: &str) -> Result<(), String> {
    let mut wait_cycles = 0_u32;

    while source_sync_is_live(source_id)? {
        wait_cycles = wait_cycles.saturating_add(1);
        report_delete_progress(
            source_id,
            Some(8),
            Some("Waiting for queue to clear".to_string()),
            Some(format!(
                "Waiting for source sync cancellation to settle ({}).",
                wait_cycles
            )),
            true,
            None,
            None,
        );
        thread::sleep(Duration::from_millis(300));
    }

    Ok(())
}

fn dequeue_next() -> Result<Option<SourceDeleteQueuedJob>, String> {
    let mut state = runtime_state()
        .lock()
        .map_err(|_| "Source delete queue lock is poisoned.".to_string())?;

    match state.queue.pop_front() {
        Some(mut job) => {
            state.queued_ids.remove(&job.source_id);
            job.started_at = Some(Utc::now().to_rfc3339());
            job.progress_percent = Some(0);
            job.progress_label = Some("Starting delete".to_string());
            job.progress_detail = Some("Delete worker is preparing this profile.".to_string());
            job.progress_indeterminate = true;
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

fn finish_job(job: &SourceDeleteQueuedJob, status: &str, summary: String, error: Option<String>) {
    if let Ok(mut state) = runtime_state().lock() {
        if status == "failed" {
            state.failed_count = state.failed_count.saturating_add(1);
        } else {
            state.completed_count = state.completed_count.saturating_add(1);
        }

        state
            .recent_results
            .push_front(SourceDeleteQueueRecentResult {
                job_id: job.job_id.clone(),
                source_id: job.source_id.clone(),
                provider: job.provider.clone(),
                handle: job.handle.clone(),
                mode: job.mode,
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

fn publish_delete_status_event(app: &AppHandle) {
    if let Ok(status) = source_delete_queue_status() {
        let _ = app.emit(SOURCE_DELETE_QUEUE_CHANGED_EVENT, status);
    }
}

fn report_delete_progress(
    source_id: &str,
    progress_percent: Option<u32>,
    progress_label: Option<String>,
    progress_detail: Option<String>,
    progress_indeterminate: bool,
    files_processed: Option<u32>,
    files_total: Option<u32>,
) {
    if let Ok(mut state) = runtime_state().lock() {
        let Some(active_job) = state.active_job.as_mut() else {
            return;
        };
        if active_job.source_id != source_id {
            return;
        }

        let normalized_percent = progress_percent.map(|value| value.min(100));
        let changed = active_job.progress_percent != normalized_percent
            || active_job.progress_label != progress_label
            || active_job.progress_detail != progress_detail
            || active_job.progress_indeterminate != progress_indeterminate
            || active_job.files_processed != files_processed
            || active_job.files_total != files_total;

        if !changed {
            return;
        }

        active_job.progress_percent = normalized_percent;
        active_job.progress_label = progress_label;
        active_job.progress_detail = progress_detail;
        active_job.progress_indeterminate = progress_indeterminate;
        active_job.files_processed = files_processed;
        active_job.files_total = files_total;
    }

    publish_delete_status_event_from_registered_app();
}

fn build_queue_status(state: &SourceDeleteRuntimeState) -> SourceDeleteQueueStatus {
    let queued_items = state
        .queue
        .iter()
        .map(|job| queue_job_to_model(job, "queued"))
        .collect::<Vec<_>>();

    let running_items = state
        .active_job
        .as_ref()
        .map(|job| vec![queue_job_to_model(job, "running")])
        .unwrap_or_default();

    let queued_count = queued_items.len() as u32;
    let running_count = running_items.len() as u32;

    SourceDeleteQueueStatus {
        queued_count,
        running_count,
        completed_count: state.completed_count,
        failed_count: state.failed_count,
        total_count: queued_count + running_count + state.completed_count + state.failed_count,
        active_job_id: state.active_job.as_ref().map(|job| job.job_id.clone()),
        active_source_id: state.active_job.as_ref().map(|job| job.source_id.clone()),
        active_handle: state.active_job.as_ref().map(|job| job.handle.clone()),
        active_provider: state.active_job.as_ref().map(|job| job.provider.clone()),
        active_mode: state.active_job.as_ref().map(|job| job.mode),
        active_started_at: state
            .active_job
            .as_ref()
            .and_then(|job| job.started_at.clone()),
        queued_items,
        running_items,
        recent_results: state.recent_results.iter().cloned().collect(),
        updated_at: Utc::now().to_rfc3339(),
    }
}

fn queue_job_to_model(job: &SourceDeleteQueuedJob, state: &str) -> SourceDeleteQueueJob {
    SourceDeleteQueueJob {
        job_id: job.job_id.clone(),
        source_id: job.source_id.clone(),
        provider: job.provider.clone(),
        handle: job.handle.clone(),
        mode: job.mode,
        state: state.to_string(),
        queued_at: job.queued_at.clone(),
        started_at: job.started_at.clone(),
        progress_percent: job.progress_percent,
        progress_label: job.progress_label.clone(),
        progress_detail: job.progress_detail.clone(),
        progress_indeterminate: job.progress_indeterminate,
        files_processed: job.files_processed,
        files_total: job.files_total,
    }
}

fn success_summary(mode: SourceProfileDeleteMode) -> String {
    match mode {
        SourceProfileDeleteMode::UserOnly => {
            "Profile deleted (user only). Media files were kept on disk.".to_string()
        }
        SourceProfileDeleteMode::WithMedia => {
            "Profile deleted with media. Disk wipe and ledger cascade finished.".to_string()
        }
    }
}

fn failure_summary(mode: SourceProfileDeleteMode) -> String {
    match mode {
        SourceProfileDeleteMode::UserOnly => "Delete user-only job failed.".to_string(),
        SourceProfileDeleteMode::WithMedia => "Delete-with-media job failed.".to_string(),
    }
}

pub fn clear_runtime_state_for_tests() {
    if let Ok(mut state) = runtime_state().lock() {
        *state = SourceDeleteRuntimeState::default();
    }
    if let Ok(mut holder) = runtime_app_handle().lock() {
        *holder = None;
    }
}
