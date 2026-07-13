use chrono::Utc;
use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};

use crate::domain::models::{
    MediaPathMigrationQueueJob, MediaPathMigrationQueueRecentResult, MediaPathMigrationQueueStatus,
};
use crate::infrastructure::{desktop_runtime, workspace_repository};

pub const MEDIA_PATH_MIGRATION_QUEUE_CHANGED_EVENT: &str =
    "runtime://media-path-migration-queue-changed";
const RECENT_LIMIT: usize = 80;

#[derive(Clone)]
struct QueuedJob {
    job_id: String,
    source_id: String,
    provider: String,
    handle: String,
    source_path: String,
    target_base_path: String,
    target_path: String,
    queued_at: String,
    started_at: Option<String>,
    files_total: u64,
    bytes_total: u64,
    files_processed: u64,
    bytes_processed: u64,
    progress_stage: String,
    progress_indeterminate: bool,
    current_file: Option<String>,
    cancel_requested: Arc<AtomicBool>,
}
#[derive(Default)]
struct State {
    queue: VecDeque<QueuedJob>,
    ids: HashSet<String>,
    active: Option<QueuedJob>,
    worker: bool,
    completed: u32,
    failed: u32,
    recent: VecDeque<MediaPathMigrationQueueRecentResult>,
}
type Shared = Arc<Mutex<State>>;
fn state() -> &'static Shared {
    static VALUE: OnceLock<Shared> = OnceLock::new();
    VALUE.get_or_init(|| Arc::new(Mutex::new(State::default())))
}

pub fn is_source_migrating(source_id: &str) -> bool {
    state().lock().ok().is_some_and(|value| {
        value.ids.contains(source_id)
            || value
                .active
                .as_ref()
                .is_some_and(|job| job.source_id == source_id)
    })
}

pub fn enqueue(
    app: &AppHandle,
    source_ids: Vec<String>,
    target_base_path: String,
) -> Result<MediaPathMigrationQueueStatus, String> {
    let target_base_path = target_base_path.trim().to_string();
    if target_base_path.is_empty() {
        return Err("The new save path is required.".to_string());
    }
    let mut added = false;
    {
        let mut value = state()
            .lock()
            .map_err(|_| "Media migration queue lock is poisoned.".to_string())?;
        for source_id in source_ids {
            if value.ids.contains(&source_id)
                || value
                    .active
                    .as_ref()
                    .is_some_and(|job| job.source_id == source_id)
            {
                continue;
            }
            let (provider, handle, source_path) =
                workspace_repository::media_path_migration_seed(source_id.clone())?;
            let job_id = uuid::Uuid::new_v4().to_string();
            let queued_at = Utc::now().to_rfc3339();
            let target_path = std::path::Path::new(&target_base_path)
                .join(handle.trim_start_matches('@'))
                .display()
                .to_string();
            workspace_repository::persist_media_path_migration_job(
                &job_id,
                &source_id,
                &target_base_path,
                &queued_at,
            )?;
            value.ids.insert(source_id.clone());
            value.queue.push_back(QueuedJob {
                job_id,
                source_id,
                provider,
                handle,
                source_path,
                target_base_path: target_base_path.clone(),
                target_path,
                queued_at,
                started_at: None,
                files_total: 0,
                bytes_total: 0,
                files_processed: 0,
                bytes_processed: 0,
                progress_stage: "queued".to_string(),
                progress_indeterminate: true,
                current_file: None,
                cancel_requested: Arc::new(AtomicBool::new(false)),
            });
            added = true;
        }
        if added && !value.worker {
            value.worker = true;
            spawn(app.clone());
        }
    }
    publish(app);
    status()
}

pub fn restore_persisted_queue(app: &AppHandle) {
    let Ok(rows) = workspace_repository::load_media_path_migration_jobs() else {
        return;
    };
    if rows.is_empty() {
        return;
    }
    let mut value = match state().lock() {
        Ok(value) => value,
        Err(_) => return,
    };
    for (job_id, source_id, target_base_path, queued_at) in rows {
        if value.ids.contains(&source_id) {
            continue;
        }
        let Ok((provider, handle, source_path)) =
            workspace_repository::media_path_migration_seed(source_id.clone())
        else {
            let _ = workspace_repository::remove_media_path_migration_job(&job_id);
            continue;
        };
        let target_path = std::path::Path::new(&target_base_path)
            .join(handle.trim_start_matches('@'))
            .display()
            .to_string();
        value.ids.insert(source_id.clone());
        value.queue.push_back(QueuedJob {
            job_id,
            source_id,
            provider,
            handle,
            source_path,
            target_base_path,
            target_path,
            queued_at,
            started_at: None,
            files_total: 0,
            bytes_total: 0,
            files_processed: 0,
            bytes_processed: 0,
            progress_stage: "queued".to_string(),
            progress_indeterminate: true,
            current_file: None,
            cancel_requested: Arc::new(AtomicBool::new(false)),
        });
    }
    if !value.queue.is_empty() && !value.worker {
        value.worker = true;
        spawn(app.clone());
    }
    drop(value);
    publish(app);
}

pub fn status() -> Result<MediaPathMigrationQueueStatus, String> {
    let value = state()
        .lock()
        .map_err(|_| "Media migration queue lock is poisoned.".to_string())?;
    Ok(build(&value))
}

pub fn cancel_all(app: &AppHandle) -> Result<MediaPathMigrationQueueStatus, String> {
    let queued_job_ids = {
        let mut value = state()
            .lock()
            .map_err(|_| "Media migration queue lock is poisoned.".to_string())?;
        if let Some(active) = value.active.as_ref() {
            active.cancel_requested.store(true, Ordering::Release);
        }
        let job_ids = value.queue.iter().map(|job| job.job_id.clone()).collect::<Vec<_>>();
        value.queue.clear();
        value.ids.clear();
        job_ids
    };
    let mut removal_error = None;
    for job_id in queued_job_ids {
        if let Err(error) = workspace_repository::remove_media_path_migration_job(&job_id) {
            removal_error.get_or_insert(error);
        }
    }
    publish(app);
    if let Some(error) = removal_error {
        return Err(error);
    }
    status()
}

fn spawn(app: AppHandle) {
    thread::spawn(move || loop {
        let job = {
            let mut value = match state().lock() {
                Ok(value) => value,
                Err(_) => return,
            };
            match value.queue.pop_front() {
                Some(mut job) => {
                    value.ids.remove(&job.source_id);
                    job.started_at = Some(Utc::now().to_rfc3339());
                    let _ = workspace_repository::set_media_path_migration_job_running(
                        &job.job_id,
                        job.started_at.as_deref().unwrap_or_default(),
                    );
                    value.active = Some(job.clone());
                    job
                }
                None => {
                    value.active = None;
                    value.worker = false;
                    drop(value);
                    publish(&app);
                    return;
                }
            }
        };
        publish(&app);
        update_active(&app, &job.job_id, |active| {
            active.progress_stage = "scanning".to_string();
            active.progress_indeterminate = true;
        });
        let (files, bytes) = scan_totals(std::path::Path::new(&job.source_path));
        update_active(&app, &job.job_id, |active| {
            active.files_total = files;
            active.bytes_total = bytes;
            active.progress_stage = "moving".to_string();
            active.progress_indeterminate = true;
        });
        let mut last_publish = Instant::now() - Duration::from_secs(1);
        let mut last_percent = 0;
        let app_for_progress = app.clone();
        let job_id_for_progress = job.job_id.clone();
        let cancel_requested = job.cancel_requested.clone();
        let outcome = workspace_repository::change_source_media_path_migration(
            job.source_id.clone(),
            job.target_base_path.clone(),
            &job.job_id,
            move |progress, atomic| {
                if cancel_requested.load(Ordering::Acquire) {
                    return Err("Media path migration cancelled.".to_string());
                }
                let percent = if files == 0 {
                    100
                } else {
                    ((progress.files_processed * 100) / files).min(100) as u32
                };
                let should_publish = atomic
                    || percent != last_percent
                    || last_publish.elapsed() >= Duration::from_millis(150);
                if should_publish {
                    last_percent = percent;
                    last_publish = Instant::now();
                    update_active(&app_for_progress, &job_id_for_progress, |active| {
                        if !atomic {
                            active.files_processed = progress.files_processed;
                            active.bytes_processed = progress.bytes_processed;
                            active.current_file = (!progress.current_file.is_empty())
                                .then_some(progress.current_file);
                        }
                        active.progress_stage =
                            if atomic { "finalizing" } else { "moving" }.to_string();
                        active.progress_indeterminate = atomic;
                    });
                }
                Ok(())
            },
        );
        if outcome.is_ok() {
            update_active(&app, &job.job_id, |active| {
                active.files_processed = active.files_total;
                active.bytes_processed = active.bytes_total;
                active.progress_stage = "updating_profile".to_string();
                active.progress_indeterminate = true;
                active.current_file = None;
            });
        }
        let cancelled = job.cancel_requested.load(Ordering::Acquire) && outcome.is_err();
        if cancelled {
            let staging_root = std::path::Path::new(&job.target_base_path)
                .join(format!(".ninjacrawler-moving-{}", job.job_id));
            let _ = std::fs::remove_dir_all(staging_root);
        }
        let (status_value, summary, error) = match outcome {
            Ok(snapshot) => {
                let _ = desktop_runtime::publish_workspace_runtime(&app, &snapshot);
                (
                    "succeeded",
                    format!("Moved {} files ({} bytes).", files, bytes),
                    None,
                )
            }
            Err(_error) if cancelled => (
                "cancelled",
                "Media path migration cancelled. The original folder was preserved.".to_string(),
                None,
            ),
            Err(error) => (
                "failed",
                "Media path migration failed.".to_string(),
                Some(error),
            ),
        };
        let _ = workspace_repository::remove_media_path_migration_job(&job.job_id);
        if let Ok(mut value) = state().lock() {
            if status_value == "succeeded" {
                value.completed = value.completed.saturating_add(1)
            } else if status_value == "failed" {
                value.failed = value.failed.saturating_add(1)
            };
            value
                .recent
                .push_front(MediaPathMigrationQueueRecentResult {
                    job_id: job.job_id.clone(),
                    source_id: job.source_id.clone(),
                    provider: job.provider.clone(),
                    handle: job.handle.clone(),
                    source_path: job.source_path.clone(),
                    target_path: job.target_path.clone(),
                    status: status_value.to_string(),
                    summary,
                    finished_at: Utc::now().to_rfc3339(),
                    error,
                });
            while value.recent.len() > RECENT_LIMIT {
                value.recent.pop_back();
            }
            value.active = None;
        }
        publish(&app);
    });
}

fn scan_totals(path: &std::path::Path) -> (u64, u64) {
    fn walk(path: &std::path::Path, counts: &mut (u64, u64)) {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, counts)
            } else if let Ok(meta) = entry.metadata() {
                counts.0 += 1;
                counts.1 += meta.len();
            }
        }
    }
    let mut counts = (0, 0);
    if path.exists() {
        walk(path, &mut counts)
    };
    counts
}
fn update_active(app: &AppHandle, job_id: &str, update: impl FnOnce(&mut QueuedJob)) {
    if let Ok(mut value) = state().lock() {
        if let Some(job) = value.active.as_mut() {
            if job.job_id == job_id {
                update(job);
            }
        }
    }
    publish(app)
}
fn model(job: &QueuedJob, state_name: &str, done: bool) -> MediaPathMigrationQueueJob {
    MediaPathMigrationQueueJob {
        job_id: job.job_id.clone(),
        source_id: job.source_id.clone(),
        provider: job.provider.clone(),
        handle: job.handle.clone(),
        source_path: job.source_path.clone(),
        target_path: job.target_path.clone(),
        state: state_name.to_string(),
        queued_at: job.queued_at.clone(),
        started_at: job.started_at.clone(),
        progress_percent: if done {
            Some(100)
        } else if job.progress_indeterminate || job.files_total == 0 {
            None
        } else {
            Some(((job.files_processed * 100) / job.files_total).min(100) as u32)
        },
        progress_stage: job.progress_stage.clone(),
        progress_indeterminate: job.progress_indeterminate,
        progress_label: Some(if done {
            "Completed".to_string()
        } else if state_name == "running" {
            match job.progress_stage.as_str() {
                "scanning" => "Scanning media",
                "updating_profile" => "Updating profile path",
                "finalizing" => "Finalizing move",
                _ => "Moving media",
            }
            .to_string()
        } else {
            "Queued for migration".to_string()
        }),
        progress_detail: Some(if state_name == "running" {
            "Moving files and updating the profile path.".to_string()
        } else {
            "Waiting for the media migration worker.".to_string()
        }),
        files_processed: if done {
            job.files_total
        } else {
            job.files_processed
        },
        files_total: job.files_total,
        bytes_processed: if done {
            job.bytes_total
        } else {
            job.bytes_processed
        },
        bytes_total: job.bytes_total,
        current_file: job.current_file.clone(),
    }
}
fn build(value: &State) -> MediaPathMigrationQueueStatus {
    let queued_items = value
        .queue
        .iter()
        .map(|job| model(job, "queued", false))
        .collect::<Vec<_>>();
    let running_items = value
        .active
        .as_ref()
        .map(|job| vec![model(job, "running", false)])
        .unwrap_or_default();
    let queued_count = queued_items.len() as u32;
    let running_count = running_items.len() as u32;
    MediaPathMigrationQueueStatus {
        queued_count,
        running_count,
        completed_count: value.completed,
        failed_count: value.failed,
        total_count: queued_count + running_count + value.completed + value.failed,
        queued_items,
        running_items,
        recent_results: value.recent.iter().cloned().collect(),
        updated_at: Utc::now().to_rfc3339(),
    }
}
fn publish(app: &AppHandle) {
    if let Ok(payload) = status() {
        let _ = app.emit(MEDIA_PATH_MIGRATION_QUEUE_CHANGED_EVENT, payload);
    }
}
