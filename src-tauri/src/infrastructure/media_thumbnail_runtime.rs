use chrono::Utc;
use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use crate::domain::models::{
    MediaThumbnailQueueItem, MediaThumbnailQueueResult, MediaThumbnailQueueStatus,
};
use crate::infrastructure::workspace_repository;

const RECENT_LIMIT: usize = 40;

#[derive(Default)]
struct QueueState {
    queued: VecDeque<MediaThumbnailQueueItem>,
    active: Option<MediaThumbnailQueueItem>,
    recent: VecDeque<MediaThumbnailQueueResult>,
    completed_count: u32,
    failed_count: u32,
    worker_running: bool,
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
            current_file: None,
            progress_percent: None,
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
    thread::scope(|scope| {
        for _ in 0..4 {
            let pending = Arc::clone(&pending);
            let source_id = job.source_id.clone();
            scope.spawn(move || loop {
                let path = pending.lock().ok().and_then(|mut paths| paths.pop_front());
                let Some(path) = path else { break };
                set_current_file(&source_id, &path);
                let generated =
                    workspace_repository::ensure_video_thumbnail(Path::new(&path)).is_some();
                record_file_result(&source_id, generated);
            });
        }
    });
    let finished = state()
        .lock()
        .ok()
        .and_then(|queue| queue.active.clone())
        .unwrap_or(job);
    finish(finished, None);
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

fn record_file_result(source_id: &str, generated: bool) {
    if let Ok(mut queue) = state().lock() {
        if let Some(active) = queue
            .active
            .as_mut()
            .filter(|job| job.source_id == source_id)
        {
            if generated {
                active.generated += 1;
            } else {
                active.failed += 1;
            }
            active.files_processed += 1;
            active.progress_percent =
                Some(active.files_processed.saturating_mul(100) / active.files_total.max(1));
        }
    }
}

fn finish(job: MediaThumbnailQueueItem, error: Option<String>) {
    if let Ok(mut queue) = state().lock() {
        let failed = error.is_some() || job.failed > 0;
        if failed {
            queue.failed_count = queue.failed_count.saturating_add(1);
        } else {
            queue.completed_count = queue.completed_count.saturating_add(1);
        }
        queue.recent.push_front(MediaThumbnailQueueResult {
            source_id: job.source_id.clone(),
            provider: job.provider.clone(),
            handle: job.handle.clone(),
            status: if failed { "failed" } else { "succeeded" }.to_string(),
            summary: error.unwrap_or_else(|| {
                format!(
                    "Generated {}, kept {} existing, failed {}.",
                    job.generated, job.skipped_existing, job.failed
                )
            }),
            generated: job.generated,
            skipped_existing: job.skipped_existing,
            failed: job.failed,
            finished_at: Utc::now().to_rfc3339(),
        });
        while queue.recent.len() > RECENT_LIMIT {
            queue.recent.pop_back();
        }
        queue.active = None;
    }
}
