//! Background indexing of the media library into the canonical `media_index`.
//!
//! The sync path registers every file it downloads, but that only covers media
//! this app fetched itself. Everything else — libraries imported from SCrawler
//! or 4K Stogram, files moved or deleted outside the app, media downloaded
//! before the index existed — only reaches the index through a run started
//! here.
//!
//! Work is scheduled per profile: walking one profile folder is the unit of
//! progress. Fingerprints are not computed here; the run inherits whatever the
//! dedupe catalog already hashed and leaves the rest `pending` for the
//! fingerprint backlog.

use crate::domain::models::{MediaIndexRun, MediaIndexStatus};
use crate::infrastructure::{media_dedupe_runtime, media_tool_runtime, workspace_repository};
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

/// Frames sampled along a video to build its signature. Five points spread
/// across the runtime survive a story being a trimmed cut of the feed post,
/// while staying cheap enough to run over a whole library.
const VIDEO_SIGNATURE_POSITIONS: [f64; 5] = [0.1, 0.3, 0.5, 0.7, 0.9];

fn file_sha256(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Some(format!("{:x}", hasher.finalize()))
}

/// Perceptual hashes of frames sampled along the video, as a JSON array.
///
/// This is what makes "the story and the feed post are the same video"
/// detectable at all: the two files differ byte for byte (different encode,
/// different crop), so only their visual content can pair them.
fn video_signature(path: &Path, duration_ms: Option<i64>) -> Option<String> {
    let ffmpeg = media_tool_runtime::ffmpeg_executable()?;
    let duration_seconds = duration_ms.map(|value| value as f64 / 1000.0).unwrap_or(0.0);
    let temp_dir = std::env::temp_dir().join(format!("ninjacrawler-vsig-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).ok()?;

    let mut hashes = Vec::new();
    for position in VIDEO_SIGNATURE_POSITIONS {
        let seek = if duration_seconds > 1.0 {
            duration_seconds * position
        } else {
            0.0
        };
        let frame_path = temp_dir.join(format!("frame-{position}.png"));
        let mut command = Command::new(&ffmpeg);
        media_tool_runtime::configure_tool_path(&mut command);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let output = command
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error")
            .arg("-y")
            .arg("-ss")
            .arg(format!("{seek:.2}"))
            .arg("-i")
            .arg(path)
            .arg("-frames:v")
            .arg("1")
            .arg("-vf")
            .arg("scale=64:64")
            .arg(&frame_path)
            .output()
            .ok();
        let produced = output.is_some_and(|value| value.status.success()) && frame_path.is_file();
        if produced {
            if let Ok(image) = image::open(&frame_path) {
                let (_, dhash) = media_dedupe_runtime::image_hashes(&image);
                hashes.push(dhash);
            }
        }
        let _ = std::fs::remove_file(&frame_path);
    }
    let _ = std::fs::remove_dir_all(&temp_dir);

    // A single frame says almost nothing; refuse to publish a signature that
    // would produce confident-looking but meaningless matches.
    if hashes.len() < 2 {
        return None;
    }
    serde_json::to_string(&hashes).ok()
}

pub const MEDIA_INDEX_STATUS_CHANGED_EVENT: &str = "media-index://status-changed";

#[derive(Default)]
struct RuntimeState {
    run: Option<MediaIndexRun>,
    cancel: Option<Arc<AtomicBool>>,
}

fn runtime_state() -> &'static Mutex<RuntimeState> {
    static STATE: OnceLock<Mutex<RuntimeState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(RuntimeState::default()))
}

fn is_running(run: &MediaIndexRun) -> bool {
    run.status == "queued" || run.status == "running"
}

/// Publishes the in-memory run when there is one, falling back to whatever the
/// database holds. Callers get a consistent answer whether or not a run is
/// active in this process.
pub fn status() -> Result<MediaIndexStatus, String> {
    let mut status = workspace_repository::load_media_index_status()?;
    if let Ok(state) = runtime_state().lock() {
        if let Some(run) = state.run.clone() {
            status.run = Some(run);
        }
    }
    Ok(status)
}

fn publish(app: &AppHandle) {
    if let Ok(status) = status() {
        let _ = app.emit(MEDIA_INDEX_STATUS_CHANGED_EVENT, status);
    }
}

/// Mutates the active run, persists it and notifies the UI. Persistence errors
/// are swallowed on purpose: losing a progress row must not abort an indexing
/// pass that is otherwise making progress.
fn update_run(app: &AppHandle, update: impl FnOnce(&mut MediaIndexRun)) {
    let snapshot = {
        let Ok(mut state) = runtime_state().lock() else {
            return;
        };
        let Some(run) = state.run.as_mut() else {
            return;
        };
        update(run);
        run.clone()
    };
    let _ = workspace_repository::persist_media_index_run(&snapshot);
    publish(app);
}

pub fn recover_interrupted_runs() {
    let _ = workspace_repository::recover_interrupted_media_index_runs();
}

/// How many files are fingerprinted at once, from the same vocabulary the media
/// cleanup already uses. Nothing takes the whole machine unless the operator
/// asks for it — `balanced` is half the logical cores.
fn worker_count(resource_profile: &str) -> usize {
    let cores = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(2)
        .max(1);
    match resource_profile {
        "quiet" => 1,
        "fast" => cores.saturating_sub(1).max(1),
        _ => (cores / 2).max(2),
    }
}

fn normalize_resource_profile(profile: Option<String>) -> String {
    match profile
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "quiet" => "quiet".to_string(),
        "fast" => "fast".to_string(),
        _ => "balanced".to_string(),
    }
}

pub fn start_scan(
    app: &AppHandle,
    scope_source_id: Option<String>,
    resource_profile: Option<String>,
) -> Result<MediaIndexStatus, String> {
    let resource_profile = normalize_resource_profile(resource_profile);
    let scope_source_id = scope_source_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let targets = workspace_repository::media_index_reconcile_targets(scope_source_id.as_deref())?;
    if targets.is_empty() {
        return Err("There are no profiles to index.".to_string());
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let run = MediaIndexRun {
        id: Uuid::new_v4().to_string(),
        status: "running".to_string(),
        stage: "reconcile".to_string(),
        scope_source_id: scope_source_id.clone(),
        sources_total: targets.len() as i64,
        sources_processed: 0,
        files_indexed: 0,
        files_updated: 0,
        files_missing: 0,
        hashes_inherited: 0,
        fingerprints_total: 0,
        fingerprints_done: 0,
        fingerprint_started_at: None,
        resource_profile: resource_profile.clone(),
        current_source_handle: None,
        error: None,
        started_at: Utc::now().to_rfc3339(),
        finished_at: None,
    };

    {
        let mut state = runtime_state()
            .lock()
            .map_err(|_| "The media index runtime is unavailable.".to_string())?;
        if state.run.as_ref().is_some_and(is_running) {
            return Err("An indexing run is already in progress.".to_string());
        }
        state.run = Some(run.clone());
        state.cancel = Some(cancel.clone());
    }
    workspace_repository::insert_media_index_run(&run)?;
    publish(app);

    let workers = worker_count(&resource_profile);
    let app = app.clone();
    std::thread::spawn(move || run_scan(app, targets, cancel, workers));
    status()
}

/// Resumes only the fingerprint backlog.
///
/// Hashing a large library takes far longer than one session, and re-walking
/// every profile folder first would waste minutes before the first hash. This
/// picks up exactly where the previous run stopped.
pub fn resume_fingerprints(
    app: &AppHandle,
    resource_profile: Option<String>,
) -> Result<MediaIndexStatus, String> {
    let resource_profile = normalize_resource_profile(resource_profile);
    let pending = workspace_repository::count_pending_fingerprints()?;
    if pending == 0 {
        return status();
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let run = MediaIndexRun {
        id: Uuid::new_v4().to_string(),
        status: "running".to_string(),
        stage: "fingerprint".to_string(),
        scope_source_id: None,
        sources_total: 0,
        sources_processed: 0,
        files_indexed: 0,
        files_updated: 0,
        files_missing: 0,
        hashes_inherited: 0,
        fingerprints_total: pending,
        fingerprints_done: 0,
        fingerprint_started_at: Some(Utc::now().to_rfc3339()),
        resource_profile: resource_profile.clone(),
        current_source_handle: None,
        error: None,
        started_at: Utc::now().to_rfc3339(),
        finished_at: None,
    };

    {
        let mut state = runtime_state()
            .lock()
            .map_err(|_| "The media index runtime is unavailable.".to_string())?;
        if state.run.as_ref().is_some_and(is_running) {
            return Err("An indexing run is already in progress.".to_string());
        }
        state.run = Some(run.clone());
        state.cancel = Some(cancel.clone());
    }
    workspace_repository::insert_media_index_run(&run)?;
    publish(app);

    let workers = worker_count(&resource_profile);
    let app_handle = app.clone();
    std::thread::spawn(move || {
        run_fingerprint_backlog(&app_handle, &cancel, workers);
        let cancelled = cancel.load(Ordering::SeqCst);
        // Variant detection needs fingerprints, so it only runs on a complete pass.
        if !cancelled {
            if let Ok(sources) = workspace_repository::media_index_reconcile_targets(None) {
                for (source_id, _) in sources {
                    if cancel.load(Ordering::SeqCst) {
                        break;
                    }
                    let _ = workspace_repository::detect_variants_for_source(source_id);
                }
            }
        }
        update_run(&app_handle, |run| {
            run.status = if cancelled { "cancelled" } else { "completed" }.to_string();
            run.stage = "done".to_string();
            run.finished_at = Some(Utc::now().to_rfc3339());
        });
        if let Ok(mut state) = runtime_state().lock() {
            state.cancel = None;
        }
        publish(&app_handle);
    });
    status()
}

pub fn cancel_scan(app: &AppHandle) -> Result<MediaIndexStatus, String> {
    let cancel = runtime_state()
        .lock()
        .ok()
        .and_then(|state| state.cancel.clone());
    if let Some(cancel) = cancel {
        cancel.store(true, Ordering::SeqCst);
    }
    publish(app);
    status()
}

/// Works through the fingerprint backlog: sha256 for everything, perceptual
/// hashes for images, sampled-frame signatures for videos.
///
/// Runs after the reconciliation pass so newly indexed media is included, and
/// stops as soon as cancellation is requested — hashing a library is long, and
/// an operator who cancels means it.
/// Hashes one file. Returns false when the file changed mid-flight, in which
/// case it stays pending for the next run instead of storing a stale hash.
fn fingerprint_one(item: &workspace_repository::PendingFingerprint) -> bool {
    let Some(sha256) = file_sha256(&item.absolute_path) else {
        let _ = workspace_repository::mark_fingerprint_failed(&item.id);
        return true;
    };
    let (ahash, dhash, signature, width, height) = if item.media_type == "video" {
        (
            None,
            None,
            video_signature(&item.absolute_path, item.duration_ms),
            None,
            None,
        )
    } else {
        match image::open(&item.absolute_path) {
            Ok(image) => {
                let (ahash, dhash) = media_dedupe_runtime::image_hashes(&image);
                let width = i64::from(image::GenericImageView::width(&image));
                let height = i64::from(image::GenericImageView::height(&image));
                (Some(ahash), Some(dhash), None, Some(width), Some(height))
            }
            Err(_) => (None, None, None, None, None),
        }
    };

    !matches!(
        workspace_repository::store_media_fingerprint(
            &item.id,
            Some(&sha256),
            ahash.as_deref(),
            dhash.as_deref(),
            signature.as_deref(),
            width,
            height,
            item.size_bytes,
            item.modified_at_ms,
        ),
        Ok(false)
    )
}

fn run_fingerprint_backlog(app: &AppHandle, cancel: &Arc<AtomicBool>, workers: usize) {
    // Each worker keeps a file in flight; SQLite is in WAL mode with a busy
    // timeout, so the short writes at the end of each file serialize without
    // failing.
    let batch = (workers * 8).clamp(8, 240) as u32;
    let total = workspace_repository::count_pending_fingerprints().unwrap_or(0);
    let done = Arc::new(AtomicI64::new(0));
    update_run(app, |run| {
        run.fingerprints_total = total;
        run.fingerprints_done = 0;
        run.fingerprint_started_at = Some(Utc::now().to_rfc3339());
    });

    loop {
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        let Ok(pending) = workspace_repository::load_pending_fingerprints(batch) else {
            return;
        };
        if pending.is_empty() {
            return;
        }

        let queue = Arc::new(Mutex::new(pending));
        let mut handles = Vec::with_capacity(workers);
        for _ in 0..workers.max(1) {
            let queue = Arc::clone(&queue);
            let cancel = Arc::clone(cancel);
            let done = Arc::clone(&done);
            handles.push(std::thread::spawn(move || loop {
                if cancel.load(Ordering::SeqCst) {
                    return;
                }
                let next = {
                    let Ok(mut queue) = queue.lock() else {
                        return;
                    };
                    queue.pop()
                };
                let Some(item) = next else {
                    return;
                };
                if fingerprint_one(&item) {
                    done.fetch_add(1, Ordering::Relaxed);
                }
            }));
        }
        for handle in handles {
            let _ = handle.join();
        }

        let processed = done.load(Ordering::Relaxed);
        update_run(app, |run| {
            run.fingerprints_done = processed;
        });
    }
}

fn run_scan(
    app: AppHandle,
    targets: Vec<(String, String)>,
    cancel: Arc<AtomicBool>,
    workers: usize,
) {
    let mut failures: Vec<String> = Vec::new();
    let source_ids: Vec<String> = targets.iter().map(|(id, _)| id.clone()).collect();

    for (source_id, handle) in targets {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        update_run(&app, |run| {
            run.current_source_handle = Some(handle.clone());
        });

        match workspace_repository::reconcile_source_media_index(&source_id) {
            Ok(outcome) => update_run(&app, |run| {
                run.files_indexed += outcome.indexed as i64;
                run.files_updated += outcome.updated as i64;
                run.files_missing += outcome.missing as i64;
                run.hashes_inherited += outcome.inherited as i64;
                run.sources_processed += 1;
            }),
            Err(error) => {
                // One unreadable profile folder (offline drive, permissions)
                // must not abandon the rest of the library.
                failures.push(format!("{handle}: {error}"));
                update_run(&app, |run| {
                    run.sources_processed += 1;
                });
            }
        }
    }

    if !cancel.load(Ordering::SeqCst) {
        update_run(&app, |run| {
            run.stage = "fingerprint".to_string();
        });
        run_fingerprint_backlog(&app, &cancel, workers);

        // Variant detection only makes sense once fingerprints exist, so it
        // trails the backlog in the same run.
        for source_id in &source_ids {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            let _ = workspace_repository::detect_variants_for_source(source_id.clone());
        }
    }

    let cancelled = cancel.load(Ordering::SeqCst);
    update_run(&app, |run| {
        run.status = if cancelled {
            "cancelled".to_string()
        } else if failures.is_empty() {
            "completed".to_string()
        } else {
            "failed".to_string()
        };
        run.stage = "done".to_string();
        run.current_source_handle = None;
        run.finished_at = Some(Utc::now().to_rfc3339());
        if !failures.is_empty() {
            run.error = Some(failures.join("; "));
        }
    });

    if let Ok(mut state) = runtime_state().lock() {
        state.cancel = None;
    }
    publish(&app);
}
