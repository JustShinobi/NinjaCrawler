use chrono::Utc;
use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use crate::domain::models::{
    MediaThumbnailQueueItem, MediaThumbnailQueueResult, MediaThumbnailQueueStatus,
    MediaThumbnailReviewItem,
};
use crate::infrastructure::workspace_repository;

const RECENT_LIMIT: usize = 40;
/// After requesting cancel, wait this long for in-flight thumbnail workers to
/// notice the flag and release file handles. Delete must not block for minutes.
const CANCEL_WAIT_TIMEOUT: Duration = Duration::from_secs(8);
const CANCEL_WAIT_POLL: Duration = Duration::from_millis(50);

#[derive(Default)]
struct QueueState {
    queued: VecDeque<MediaThumbnailQueueItem>,
    active: Option<MediaThumbnailQueueItem>,
    recent: VecDeque<MediaThumbnailQueueResult>,
    completed_count: u32,
    failed_count: u32,
    worker_running: bool,
    /// Source ids that should abort their active thumbnail job ASAP (delete-with-media).
    cancel_requested: HashSet<String>,
}

fn state() -> &'static Mutex<QueueState> {
    static STATE: OnceLock<Mutex<QueueState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(QueueState::default()))
}

fn status_from(state: &QueueState) -> MediaThumbnailQueueStatus {
    MediaThumbnailQueueStatus {
        queued_count: state.queued.len() as u32,
        running_count: u32::from(state.active.is_some()),
        completed_count: state.completed_count,
        failed_count: state.failed_count,
        active: state.active.clone(),
        queued_items: state.queued.iter().cloned().collect(),
        recent_results: state.recent.iter().cloned().collect(),
        updated_at: Utc::now().to_rfc3339(),
    }
}

pub fn queue_status() -> Result<MediaThumbnailQueueStatus, String> {
    state()
        .lock()
        .map(|state| status_from(&state))
        .map_err(|_| "Thumbnail queue lock is poisoned.".to_string())
}

pub fn enqueue(source_ids: Vec<String>) -> Result<MediaThumbnailQueueStatus, String> {
    let mut queue = state()
        .lock()
        .map_err(|_| "Thumbnail queue lock is poisoned.".to_string())?;
    let mut known: HashSet<String> = queue
        .queued
        .iter()
        .map(|item| item.source_id.clone())
        .collect();
    if let Some(active) = &queue.active {
        known.insert(active.source_id.clone());
    }
    for source_id in source_ids {
        if source_id.trim().is_empty() || known.contains(&source_id) {
            continue;
        }
        // Clear a prior cancel so a fresh enqueue is not immediately aborted.
        queue.cancel_requested.remove(&source_id);
        let (provider, handle) = workspace_repository::media_thumbnail_source_seed(&source_id)?;
        known.insert(source_id.clone());
        queue.queued.push_back(MediaThumbnailQueueItem {
            source_id,
            provider,
            handle,
            state: "queued".to_string(),
            queued_at: Utc::now().to_rfc3339(),
            started_at: None,
            files_scanned: 0,
            files_total: 0,
            files_processed: 0,
            generated: 0,
            skipped_existing: 0,
            failed: 0,
            invalid_media: 0,
            current_file: None,
            progress_percent: None,
            review_items: Vec::new(),
        });
    }
    let spawn = !queue.worker_running && !queue.queued.is_empty();
    if spawn {
        queue.worker_running = true;
    }
    let status = status_from(&queue);
    drop(queue);
    if spawn {
        thread::spawn(run_worker);
    }
    Ok(status)
}

/// Drop any queued thumbnail job for `source_id`, request cancel of an active
/// job for that source, and wait briefly for workers to release file handles.
/// Used by delete-with-media — must not block the app for a full thumbnail run.
pub fn cancel_queued_and_wait(source_id: &str) -> Result<(), String> {
    {
        let mut queue = state()
            .lock()
            .map_err(|_| "Thumbnail queue lock is poisoned.".to_string())?;
        queue.queued.retain(|job| job.source_id != source_id);
        queue.cancel_requested.insert(source_id.to_string());
    }

    let deadline = std::time::Instant::now() + CANCEL_WAIT_TIMEOUT;
    loop {
        let active_match = {
            let queue = state()
                .lock()
                .map_err(|_| "Thumbnail queue lock is poisoned.".to_string())?;
            queue
                .active
                .as_ref()
                .is_some_and(|job| job.source_id == source_id)
        };
        if !active_match {
            // Drop cancel flag once the job is gone so a later re-add is clean.
            if let Ok(mut queue) = state().lock() {
                queue.cancel_requested.remove(source_id);
            }
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            // Do not fail delete: proceed and rely on resilient disk delete.
            // Leave cancel flag set so workers keep aborting until they exit.
            return Ok(());
        }
        thread::sleep(CANCEL_WAIT_POLL);
    }
}

fn is_cancel_requested(source_id: &str) -> bool {
    state()
        .lock()
        .ok()
        .is_some_and(|queue| queue.cancel_requested.contains(source_id))
}

/// After the operator deletes reviewed invalid media (recycle bin + ledger),
/// drop matching review items from the recent thumbnail result so the UI
/// stops asking for manual check.
pub fn dismiss_review_items(
    source_id: &str,
    relative_paths: &[String],
) -> Result<MediaThumbnailQueueStatus, String> {
    let mut removed: HashSet<String> = relative_paths
        .iter()
        .map(|path| path.replace('\\', "/").trim_start_matches('/').to_ascii_lowercase())
        .collect();
    let mut queue = state()
        .lock()
        .map_err(|_| "Thumbnail queue lock is poisoned.".to_string())?;
    for result in queue.recent.iter_mut() {
        if result.source_id != source_id {
            continue;
        }
        let before = result.review_items.len();
        result.review_items.retain(|item| {
            let key = item
                .relative_path
                .replace('\\', "/")
                .trim_start_matches('/')
                .to_ascii_lowercase();
            !removed.contains(&key)
        });
        let dropped = before.saturating_sub(result.review_items.len());
        if dropped == 0 {
            continue;
        }
        result.invalid_media = result.invalid_media.saturating_sub(dropped as u32);
        result.summary = build_result_summary(
            result.generated,
            result.skipped_existing,
            result.failed,
            result.invalid_media,
            None,
        );
        if result.failed == 0 && result.invalid_media == 0 {
            result.status = "succeeded".to_string();
        } else if result.failed > 0 && result.generated == 0 && result.invalid_media == 0 {
            result.status = "failed".to_string();
        } else if result.failed > 0 || result.invalid_media > 0 {
            result.status = "warning".to_string();
        }
        for item_key in removed.clone() {
            let still_present = result.review_items.iter().any(|item| {
                item.relative_path
                    .replace('\\', "/")
                    .trim_start_matches('/')
                    .to_ascii_lowercase()
                    == item_key
            });
            if !still_present {
                removed.remove(&item_key);
            }
        }
    }
    Ok(status_from(&queue))
}

fn run_worker() {
    loop {
        let job = {
            let Ok(mut queue) = state().lock() else {
                return;
            };
            let Some(mut job) = queue.queued.pop_front() else {
                queue.worker_running = false;
                return;
            };
            job.state = "running".to_string();
            job.started_at = Some(Utc::now().to_rfc3339());
            queue.active = Some(job.clone());
            job
        };
        run_job(job);
    }
}

fn run_job(mut job: MediaThumbnailQueueItem) {
    if is_cancel_requested(&job.source_id) {
        finish_cancelled(job);
        return;
    }

    let profile_root = match workspace_repository::media_thumbnail_source_root(&job.source_id) {
        Ok(root) => root,
        Err(error) => {
            finish(job, Some(error));
            return;
        }
    };
    let result = workspace_repository::media_thumbnail_source_paths(&job.source_id);
    let paths = match result {
        Ok(paths) => paths,
        Err(error) => {
            finish(job, Some(error));
            return;
        }
    };
    job.files_scanned = paths.len() as u32;
    let mut missing = Vec::new();
    for path in paths {
        if workspace_repository::media_thumbnail_is_current(Path::new(&path)) {
            job.skipped_existing += 1;
        } else {
            missing.push(path);
        }
    }
    job.files_total = missing.len() as u32;
    update_active(&job);

    let pending = Arc::new(Mutex::new(VecDeque::from(missing)));
    let profile_root = Arc::new(profile_root);
    let cancel_flag = Arc::new(AtomicBool::new(false));
    thread::scope(|scope| {
        for _ in 0..4 {
            let pending = Arc::clone(&pending);
            let source_id = job.source_id.clone();
            let profile_root = Arc::clone(&profile_root);
            let cancel_flag = Arc::clone(&cancel_flag);
            scope.spawn(move || loop {
                if cancel_flag.load(Ordering::Relaxed) || is_cancel_requested(&source_id) {
                    cancel_flag.store(true, Ordering::Relaxed);
                    // Drain remaining work so sibling workers exit quickly.
                    if let Ok(mut paths) = pending.lock() {
                        paths.clear();
                    }
                    break;
                }
                let path = pending.lock().ok().and_then(|mut paths| paths.pop_front());
                let Some(path) = path else { break };
                set_current_file(&source_id, &path);
                let outcome = workspace_repository::generate_media_thumbnail(Path::new(&path));
                record_file_result(&source_id, Path::new(&path), profile_root.as_path(), outcome);
            });
        }
    });

    let cancelled = is_cancel_requested(&job.source_id) || cancel_flag.load(Ordering::Relaxed);
    let finished = state()
        .lock()
        .ok()
        .and_then(|queue| queue.active.clone())
        .unwrap_or(job);
    if cancelled {
        finish_cancelled(finished);
    } else {
        finish(finished, None);
    }
}

fn finish_cancelled(job: MediaThumbnailQueueItem) {
    if let Ok(mut queue) = state().lock() {
        queue.cancel_requested.remove(&job.source_id);
        queue.completed_count = queue.completed_count.saturating_add(1);
        queue.recent.push_front(MediaThumbnailQueueResult {
            source_id: job.source_id.clone(),
            provider: job.provider.clone(),
            handle: job.handle.clone(),
            status: "skipped".to_string(),
            summary: format!(
                "Thumbnail generation cancelled (profile delete). Generated {}, kept {} existing.",
                job.generated, job.skipped_existing
            ),
            generated: job.generated,
            skipped_existing: job.skipped_existing,
            failed: job.failed,
            invalid_media: job.invalid_media,
            review_items: job.review_items,
            finished_at: Utc::now().to_rfc3339(),
        });
        while queue.recent.len() > RECENT_LIMIT {
            queue.recent.pop_back();
        }
        queue.active = None;
    }
}

fn update_active(job: &MediaThumbnailQueueItem) {
    if let Ok(mut queue) = state().lock() {
        queue.active = Some(job.clone());
    }
}

fn set_current_file(source_id: &str, path: &str) {
    if let Ok(mut queue) = state().lock() {
        if let Some(active) = queue
            .active
            .as_mut()
            .filter(|job| job.source_id == source_id)
        {
            active.current_file = Path::new(path)
                .file_name()
                .map(|name| name.to_string_lossy().to_string());
        }
    }
}

fn relative_media_path(profile_root: &Path, absolute: &Path) -> String {
    absolute
        .strip_prefix(profile_root)
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| {
            absolute
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
}

fn record_file_result(
    source_id: &str,
    absolute: &Path,
    profile_root: &Path,
    outcome: workspace_repository::MediaThumbnailGenerationOutcome,
) {
    if let Ok(mut queue) = state().lock() {
        if let Some(active) = queue
            .active
            .as_mut()
            .filter(|job| job.source_id == source_id)
        {
            let file_name = absolute
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| absolute.to_string_lossy().into_owned());
            let post_url = workspace_repository::derive_review_item_post_url(
                &active.provider,
                &active.handle,
                &file_name,
            );
            match outcome {
                workspace_repository::MediaThumbnailGenerationOutcome::Generated => {
                    active.generated += 1;
                }
                workspace_repository::MediaThumbnailGenerationOutcome::NotNeeded => {
                    active.skipped_existing += 1;
                }
                workspace_repository::MediaThumbnailGenerationOutcome::InvalidMedia { reason } => {
                    active.invalid_media += 1;
                    active.review_items.push(MediaThumbnailReviewItem {
                        absolute_path: absolute.to_string_lossy().into_owned(),
                        relative_path: relative_media_path(profile_root, absolute),
                        file_name,
                        kind: "invalid_media".to_string(),
                        reason,
                        post_url,
                    });
                }
                workspace_repository::MediaThumbnailGenerationOutcome::Failed { reason } => {
                    active.failed += 1;
                    active.review_items.push(MediaThumbnailReviewItem {
                        absolute_path: absolute.to_string_lossy().into_owned(),
                        relative_path: relative_media_path(profile_root, absolute),
                        file_name,
                        kind: "generation_failed".to_string(),
                        reason,
                        post_url,
                    });
                }
            }
            active.files_processed += 1;
            active.progress_percent =
                Some(active.files_processed.saturating_mul(100) / active.files_total.max(1));
        }
    }
}

fn build_result_summary(
    generated: u32,
    skipped_existing: u32,
    failed: u32,
    invalid_media: u32,
    job_error: Option<&str>,
) -> String {
    if let Some(error) = job_error {
        return error.to_string();
    }
    let mut parts = vec![format!(
        "Generated {generated}, kept {skipped_existing} existing"
    )];
    if invalid_media > 0 {
        parts.push(format!(
            "{invalid_media} invalid media file(s) need manual check"
        ));
    }
    if failed > 0 {
        parts.push(format!("{failed} generation failure(s)"));
    }
    if invalid_media == 0 && failed == 0 {
        parts.push("no issues".to_string());
    }
    parts.join(". ") + "."
}

fn finish(job: MediaThumbnailQueueItem, error: Option<String>) {
    if let Ok(mut queue) = state().lock() {
        queue.cancel_requested.remove(&job.source_id);
        let status = if error.is_some() {
            "failed"
        } else if job.failed > 0 && job.generated == 0 && job.invalid_media == 0 {
            "failed"
        } else if job.failed > 0 || job.invalid_media > 0 {
            "warning"
        } else {
            "succeeded"
        };
        if status == "failed" {
            queue.failed_count = queue.failed_count.saturating_add(1);
        } else {
            queue.completed_count = queue.completed_count.saturating_add(1);
        }
        queue.recent.push_front(MediaThumbnailQueueResult {
            source_id: job.source_id.clone(),
            provider: job.provider.clone(),
            handle: job.handle.clone(),
            status: status.to_string(),
            summary: build_result_summary(
                job.generated,
                job.skipped_existing,
                job.failed,
                job.invalid_media,
                error.as_deref(),
            ),
            generated: job.generated,
            skipped_existing: job.skipped_existing,
            failed: job.failed,
            invalid_media: job.invalid_media,
            review_items: job.review_items,
            finished_at: Utc::now().to_rfc3339(),
        });
        while queue.recent.len() > RECENT_LIMIT {
            queue.recent.pop_back();
        }
        queue.active = None;
    }
}
