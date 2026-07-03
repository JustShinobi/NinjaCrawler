use chrono::Utc;
use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use tauri::{AppHandle, Emitter};

use crate::domain::models::{
    SingleVideoQueueItem, SingleVideoQueueRecentResult, SingleVideoQueueStatus,
};
use crate::infrastructure::workspace_repository;

/// Emitido a cada mudança na fila de single videos (para a janela Queue Status).
pub const SINGLE_VIDEO_QUEUE_CHANGED_EVENT: &str = "runtime://single-video-queue-changed";
/// Emitido quando o catálogo de single videos muda (para a janela Single Videos
/// recarregar a lista).
pub const SINGLE_VIDEOS_CHANGED_EVENT: &str = "runtime://single-videos-changed";

const RECENT_RESULTS_LIMIT: usize = 40;

#[derive(Clone)]
struct SingleVideoJob {
    id: String,
    url: String,
    provider: Option<String>,
    queued_at: String,
    started_at: Option<String>,
}

impl SingleVideoJob {
    fn to_item(&self, state: &str) -> SingleVideoQueueItem {
        SingleVideoQueueItem {
            id: self.id.clone(),
            url: self.url.clone(),
            provider: self.provider.clone(),
            state: state.to_string(),
            queued_at: self.queued_at.clone(),
            started_at: self.started_at.clone(),
            progress_label: Some("Downloading".to_string()),
            progress_indeterminate: true,
        }
    }
}

#[derive(Default)]
struct SingleVideoQueueState {
    queue: VecDeque<SingleVideoJob>,
    queued_urls: HashSet<String>,
    active: Option<SingleVideoJob>,
    worker_running: bool,
    completed_count: u32,
    failed_count: u32,
    recent_results: VecDeque<SingleVideoQueueRecentResult>,
}

type SharedState = Arc<Mutex<SingleVideoQueueState>>;
type SharedAppHandle = Arc<Mutex<Option<AppHandle>>>;

fn queue_state() -> &'static SharedState {
    static STATE: OnceLock<SharedState> = OnceLock::new();
    STATE.get_or_init(|| Arc::new(Mutex::new(SingleVideoQueueState::default())))
}

fn app_handle_slot() -> &'static SharedAppHandle {
    static APP_HANDLE: OnceLock<SharedAppHandle> = OnceLock::new();
    APP_HANDLE.get_or_init(|| Arc::new(Mutex::new(None)))
}

fn register_app_handle(app: &AppHandle) {
    if let Ok(mut holder) = app_handle_slot().lock() {
        *holder = Some(app.clone());
    }
}

fn registered_app() -> Option<AppHandle> {
    app_handle_slot()
        .lock()
        .ok()
        .and_then(|holder| holder.as_ref().cloned())
}

fn build_status(state: &SingleVideoQueueState) -> SingleVideoQueueStatus {
    let queued_items: Vec<SingleVideoQueueItem> =
        state.queue.iter().map(|job| job.to_item("queued")).collect();
    let active = state.active.as_ref().map(|job| job.to_item("running"));
    SingleVideoQueueStatus {
        queued_count: state.queue.len() as u32,
        running_count: if active.is_some() { 1 } else { 0 },
        completed_count: state.completed_count,
        failed_count: state.failed_count,
        active,
        queued_items,
        recent_results: state.recent_results.iter().cloned().collect(),
        updated_at: Utc::now().to_rfc3339(),
    }
}

pub fn single_video_queue_status() -> Result<SingleVideoQueueStatus, String> {
    let state = queue_state()
        .lock()
        .map_err(|_| "Single video queue lock is poisoned.".to_string())?;
    Ok(build_status(&state))
}

fn publish_queue_status() {
    if let Some(app) = registered_app() {
        if let Ok(status) = single_video_queue_status() {
            let _ = app.emit(SINGLE_VIDEO_QUEUE_CHANGED_EVENT, status);
        }
    }
}

fn publish_catalog_changed() {
    if let Some(app) = registered_app() {
        let _ = app.emit(SINGLE_VIDEOS_CHANGED_EVENT, ());
    }
}

/// Enfileira o download de um single video e garante que o worker esteja vivo.
/// Retorna o status atual da fila. Duplicatas (mesma URL já em fila ou ativa)
/// são ignoradas silenciosamente.
pub fn enqueue_single_video(app: &AppHandle, url: String) -> Result<SingleVideoQueueStatus, String> {
    let url = url.trim().to_string();
    if url.is_empty() {
        return Err("A video URL is required.".to_string());
    }
    register_app_handle(app);

    let should_spawn_worker = {
        let mut state = queue_state()
            .lock()
            .map_err(|_| "Single video queue lock is poisoned.".to_string())?;

        let already_active = state
            .active
            .as_ref()
            .is_some_and(|job| job.url == url);
        if already_active || state.queued_urls.contains(&url) {
            return build_status_locked(&state);
        }

        let job = SingleVideoJob {
            id: format!("sv_{}", Utc::now().timestamp_micros()),
            url: url.clone(),
            provider: detect_provider(&url),
            queued_at: Utc::now().to_rfc3339(),
            started_at: None,
        };
        state.queued_urls.insert(url.clone());
        state.queue.push_back(job);

        if state.worker_running {
            false
        } else {
            state.worker_running = true;
            true
        }
    };

    publish_queue_status();
    if should_spawn_worker {
        spawn_worker(app.clone());
    }
    single_video_queue_status()
}

fn build_status_locked(state: &SingleVideoQueueState) -> Result<SingleVideoQueueStatus, String> {
    Ok(build_status(state))
}

fn spawn_worker(app: AppHandle) {
    thread::spawn(move || loop {
        let job = {
            let mut state = match queue_state().lock() {
                Ok(state) => state,
                Err(_) => break,
            };
            match state.queue.pop_front() {
                Some(mut job) => {
                    state.queued_urls.remove(&job.url);
                    job.started_at = Some(Utc::now().to_rfc3339());
                    state.active = Some(job.clone());
                    job
                }
                None => {
                    state.active = None;
                    state.worker_running = false;
                    break;
                }
            }
        };
        publish_queue_status();

        let result = workspace_repository::download_single_video(job.url.clone());
        record_result(&job, &result);
        publish_queue_status();
        if result.is_ok() {
            publish_catalog_changed();
        }
        let _ = &app; // handle mantido vivo para os emits registrados.
    });
}

fn record_result(job: &SingleVideoJob, result: &Result<crate::domain::models::SingleVideo, String>) {
    if let Ok(mut state) = queue_state().lock() {
        state.active = None;
        let recent = match result {
            Ok(video) => {
                state.completed_count = state.completed_count.saturating_add(1);
                SingleVideoQueueRecentResult {
                    url: job.url.clone(),
                    provider: Some(video.provider.clone()),
                    uploader: video.uploader.clone(),
                    title: video.title.clone(),
                    status: "succeeded".to_string(),
                    summary: "Single video downloaded.".to_string(),
                    finished_at: Utc::now().to_rfc3339(),
                }
            }
            Err(error) => {
                state.failed_count = state.failed_count.saturating_add(1);
                SingleVideoQueueRecentResult {
                    url: job.url.clone(),
                    provider: job.provider.clone(),
                    uploader: None,
                    title: None,
                    status: "failed".to_string(),
                    summary: error.clone(),
                    finished_at: Utc::now().to_rfc3339(),
                }
            }
        };
        state.recent_results.push_front(recent);
        while state.recent_results.len() > RECENT_RESULTS_LIMIT {
            state.recent_results.pop_back();
        }
    }
}

/// Detecção leve de provider pelo host (para rotular o item na fila antes do
/// download resolver os metadados). Mantém a lista alinhada ao backend.
fn detect_provider(url: &str) -> Option<String> {
    let lowered = url.to_ascii_lowercase();
    if lowered.contains("tiktok.com") {
        Some("tiktok".to_string())
    } else if lowered.contains("instagram.com") {
        Some("instagram".to_string())
    } else if lowered.contains("twitter.com") || lowered.contains("x.com") {
        Some("twitter".to_string())
    } else if lowered.contains("youtube.com") || lowered.contains("youtu.be") {
        Some("youtube".to_string())
    } else {
        None
    }
}
