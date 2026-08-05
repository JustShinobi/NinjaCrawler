//! Candidate-driven background indexing for the canonical media library.
//!
//! Reconciliation still discovers files per profile, but expensive fingerprints
//! are only produced when another eligible file can actually be compared with
//! them. The runtime owns leases, progress snapshots and child-process lifetime.

use crate::domain::models::{MediaIndexCounts, MediaIndexRun, MediaIndexStatus};
use crate::infrastructure::{media_dedupe_runtime, media_tool_runtime, workspace_repository};
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

const VIDEO_SIGNATURE_POSITIONS: [f64; 5] = [0.1, 0.3, 0.5, 0.7, 0.9];
const PROCESS_TIMEOUT: Duration = Duration::from_secs(60);
const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(500);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);

pub const MEDIA_INDEX_STATUS_CHANGED_EVENT: &str = "media-index://status-changed";

#[derive(Default)]
struct RuntimeState {
    run: Option<MediaIndexRun>,
    counts: Option<MediaIndexCounts>,
    cancel: Option<Arc<AtomicBool>>,
    resource_profile: Option<Arc<Mutex<String>>>,
    last_persisted_at: Option<Instant>,
}

fn runtime_state() -> &'static Mutex<RuntimeState> {
    static STATE: OnceLock<Mutex<RuntimeState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(RuntimeState::default()))
}

fn is_active(run: &MediaIndexRun) -> bool {
    matches!(run.status.as_str(), "queued" | "running" | "pausing")
}

pub fn status() -> Result<MediaIndexStatus, String> {
    if let Ok(state) = runtime_state().lock() {
        if let (Some(run), Some(counts)) = (state.run.clone(), state.counts.clone()) {
            return Ok(MediaIndexStatus {
                counts,
                run: Some(run),
            });
        }
    }
    workspace_repository::load_media_index_status()
}

fn publish(app: &AppHandle) {
    if let Ok(status) = status() {
        let _ = app.emit(MEDIA_INDEX_STATUS_CHANGED_EVENT, status);
    }
}

fn update_run(app: &AppHandle, force_persist: bool, update: impl FnOnce(&mut MediaIndexRun)) {
    let (snapshot, should_persist) = {
        let Ok(mut state) = runtime_state().lock() else {
            return;
        };
        let now = Instant::now();
        let should_persist = force_persist
            || state
                .last_persisted_at
                .is_none_or(|last| now.duration_since(last) >= HEARTBEAT_INTERVAL);
        if should_persist {
            state.last_persisted_at = Some(now);
        }
        let Some(run) = state.run.as_mut() else {
            return;
        };
        update(run);
        (run.clone(), should_persist)
    };
    if should_persist {
        let _ = workspace_repository::persist_media_index_run(&snapshot);
    }
    publish(app);
}

fn refresh_cached_counts() {
    let Ok(status) = workspace_repository::load_media_index_status() else {
        return;
    };
    if let Ok(mut state) = runtime_state().lock() {
        state.counts = Some(status.counts);
    }
}

pub fn recover_interrupted_runs() {
    let _ = workspace_repository::recover_interrupted_media_index_runs();
    cleanup_owned_signature_temporaries();
}

fn cleanup_owned_signature_temporaries() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with("ninjacrawler-vsig-")
        {
            let _ = std::fs::remove_dir_all(entry.path());
        }
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

fn current_profile(profile: &Arc<Mutex<String>>) -> String {
    profile
        .lock()
        .map(|value| value.clone())
        .unwrap_or_else(|_| "balanced".to_string())
}

fn worker_count(kind: &str, profile: &str) -> usize {
    match kind {
        "perceptual_video" => match profile {
            "fast" => 2,
            _ => 1,
        },
        "exact" => match profile {
            "fast" => 2,
            _ => 1,
        },
        // Image decoding is CPU-bound after the read, but each worker is also
        // a storage reader. Capping this lane prevents a high-core machine from
        // turning one volume into the old 15-reader seek storm.
        _ => match profile {
            "fast" => 2,
            _ => 1,
        },
    }
}

fn blank_run(
    id: String,
    stage: &str,
    scope_source_id: Option<String>,
    resource_profile: String,
) -> MediaIndexRun {
    MediaIndexRun {
        id,
        status: "running".to_string(),
        stage: stage.to_string(),
        scope_source_id,
        sources_total: 0,
        sources_processed: 0,
        files_indexed: 0,
        files_updated: 0,
        files_missing: 0,
        hashes_inherited: 0,
        fingerprints_total: 0,
        fingerprints_done: 0,
        fingerprint_started_at: None,
        resource_profile,
        phase_total: 0,
        phase_done: 0,
        phase_failed: 0,
        bytes_processed: 0,
        last_progress_at: Some(Utc::now().to_rfc3339()),
        rate_per_second: 0.0,
        eta_seconds: None,
        current_source_handle: None,
        error: None,
        started_at: Utc::now().to_rfc3339(),
        finished_at: None,
    }
}

fn install_run(
    run: MediaIndexRun,
    cancel: Arc<AtomicBool>,
    profile: Arc<Mutex<String>>,
) -> Result<(), String> {
    let counts = workspace_repository::load_media_index_status()?.counts;
    let mut state = runtime_state()
        .lock()
        .map_err(|_| "The media index runtime is unavailable.".to_string())?;
    if state.run.as_ref().is_some_and(is_active) {
        return Err("An indexing run is already in progress.".to_string());
    }
    state.run = Some(run);
    state.counts = Some(counts);
    state.cancel = Some(cancel);
    state.resource_profile = Some(profile);
    state.last_persisted_at = Some(Instant::now());
    Ok(())
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
    let profile = Arc::new(Mutex::new(resource_profile.clone()));
    let mut run = blank_run(
        Uuid::new_v4().to_string(),
        "reconcile",
        scope_source_id,
        resource_profile,
    );
    run.sources_total = targets.len() as i64;
    install_run(run.clone(), Arc::clone(&cancel), Arc::clone(&profile))?;
    workspace_repository::insert_media_index_run(&run)?;
    publish(app);

    let app = app.clone();
    std::thread::spawn(move || run_scan(app, targets, cancel, profile));
    status()
}

pub fn resume_fingerprints(
    app: &AppHandle,
    resource_profile: Option<String>,
) -> Result<MediaIndexStatus, String> {
    let resource_profile = normalize_resource_profile(resource_profile);
    let cancel = Arc::new(AtomicBool::new(false));
    let profile = Arc::new(Mutex::new(resource_profile.clone()));
    let previous = workspace_repository::load_media_index_status()?
        .run
        .filter(|run| run.status == "paused");
    let resumed_existing = previous.is_some();
    let mut run = previous.unwrap_or_else(|| {
        blank_run(
            Uuid::new_v4().to_string(),
            "planning",
            None,
            resource_profile.clone(),
        )
    });
    run.status = "running".to_string();
    run.stage = "planning".to_string();
    run.resource_profile = resource_profile;
    run.finished_at = None;
    run.error = None;
    // Candidate planning can take tens of seconds on a very large catalog. It
    // must only run on the background worker below; doing it here blocks the
    // Tauri command (and the Library window) and then plans the same run twice.
    run.fingerprints_total = 0;
    run.fingerprints_done = 0;
    run.fingerprint_started_at = Some(Utc::now().to_rfc3339());
    run.last_progress_at = Some(Utc::now().to_rfc3339());
    install_run(run.clone(), Arc::clone(&cancel), Arc::clone(&profile))?;
    if resumed_existing {
        workspace_repository::persist_media_index_run(&run)?;
    } else {
        workspace_repository::insert_media_index_run(&run)?;
    }
    publish(app);

    let app = app.clone();
    std::thread::spawn(move || finish_fingerprint_run(&app, &cancel, &profile, Vec::new()));
    status()
}

pub fn cancel_scan(app: &AppHandle) -> Result<MediaIndexStatus, String> {
    let cancel = runtime_state()
        .lock()
        .ok()
        .and_then(|state| state.cancel.clone());
    if let Some(cancel) = cancel {
        cancel.store(true, Ordering::SeqCst);
        update_run(app, true, |run| {
            if run.status == "running" {
                run.status = "pausing".to_string();
            }
        });
    }
    status()
}

pub fn set_resource_profile(
    app: &AppHandle,
    resource_profile: Option<String>,
) -> Result<MediaIndexStatus, String> {
    let normalized = normalize_resource_profile(resource_profile);
    let profile = runtime_state()
        .lock()
        .ok()
        .and_then(|state| state.resource_profile.clone());
    if let Some(profile) = profile {
        if let Ok(mut value) = profile.lock() {
            *value = normalized.clone();
        }
        update_run(app, true, |run| run.resource_profile = normalized);
    }
    status()
}

pub fn retry_failed_fingerprints(app: &AppHandle) -> Result<MediaIndexStatus, String> {
    let retried = workspace_repository::retry_failed_media_fingerprint_jobs()?;
    if retried == 0 {
        refresh_cached_counts();
        publish(app);
        return status();
    }
    let resource_profile = runtime_state()
        .lock()
        .ok()
        .and_then(|state| state.run.as_ref().map(|run| run.resource_profile.clone()));
    resume_fingerprints(app, resource_profile)
}

fn file_sha256(path: &Path) -> Result<String, &'static str> {
    let mut file = std::fs::File::open(path).map_err(|_| "unreadable")?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|_| "unreadable")?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn bytes_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn configure_child(command: &mut Command) {
    media_tool_runtime::configure_tool_path(command);
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
}

#[cfg(windows)]
struct KillOnCloseJob(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl KillOnCloseJob {
    fn attach(child: &Child) -> Option<Self> {
        use std::mem::{size_of, zeroed};
        use std::os::windows::io::AsRawHandle;
        use std::ptr::null;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        unsafe {
            let job = CreateJobObjectW(null(), null());
            if job.is_null() {
                return None;
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let configured = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) != 0;
            let assigned = configured
                && AssignProcessToJobObject(job, child.as_raw_handle() as _) != 0;
            if assigned {
                Some(Self(job))
            } else {
                let _ = windows_sys::Win32::Foundation::CloseHandle(job);
                None
            }
        }
    }
}

#[cfg(windows)]
impl Drop for KillOnCloseJob {
    fn drop(&mut self) {
        unsafe {
            let _ = windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

fn terminate_child_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

trait ProcessControl {
    fn poll(&mut self) -> Result<Option<bool>, ()>;
    fn terminate(&mut self);
}

struct ChildProcess<'a>(&'a mut Child);

impl ProcessControl for ChildProcess<'_> {
    fn poll(&mut self) -> Result<Option<bool>, ()> {
        self.0
            .try_wait()
            .map(|status| status.map(|status| status.success()))
            .map_err(|_| ())
    }

    fn terminate(&mut self) {
        terminate_child_tree(self.0);
    }
}

fn wait_for_process(
    process: &mut impl ProcessControl,
    cancel: &AtomicBool,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<(), &'static str> {
    let started = Instant::now();
    loop {
        if cancel.load(Ordering::SeqCst) {
            process.terminate();
            return Err("cancelled");
        }
        if started.elapsed() >= timeout {
            process.terminate();
            return Err("timeout");
        }
        match process.poll() {
            Ok(Some(true)) => return Ok(()),
            Ok(Some(false)) => return Err("process_failed"),
            Ok(None) => std::thread::sleep(poll_interval),
            Err(()) => {
                process.terminate();
                return Err("process_failed");
            }
        }
    }
}

fn wait_for_child(
    child: &mut Child,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<(), &'static str> {
    wait_for_process(
        &mut ChildProcess(child),
        cancel,
        timeout,
        Duration::from_millis(100),
    )
}

fn probe_duration_seconds(path: &Path, cancel: &AtomicBool) -> Option<f64> {
    let ffprobe = media_tool_runtime::ffprobe_executable()?;
    let output_path = std::env::temp_dir().join(format!(
        "ninjacrawler-vsig-{}-duration.txt",
        Uuid::new_v4()
    ));
    let output = std::fs::File::create(&output_path).ok()?;
    let mut command = Command::new(ffprobe);
    media_tool_runtime::configure_tool_path(&mut command);
    command
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(output))
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().ok()?;
    #[cfg(windows)]
    let _job = KillOnCloseJob::attach(&child);
    let result = wait_for_child(&mut child, cancel, Duration::from_secs(15));
    let text = result
        .ok()
        .and_then(|_| std::fs::read_to_string(&output_path).ok());
    let _ = std::fs::remove_file(output_path);
    text?.trim().parse::<f64>().ok().filter(|value| *value > 0.0)
}

fn video_signature(
    path: &Path,
    duration_ms: Option<i64>,
    cancel: &AtomicBool,
) -> Result<String, &'static str> {
    let ffmpeg = media_tool_runtime::ffmpeg_executable().ok_or("ffmpeg_unavailable")?;
    let duration_seconds = duration_ms
        .map(|value| value as f64 / 1000.0)
        .filter(|value| *value > 0.0)
        .or_else(|| probe_duration_seconds(path, cancel))
        .ok_or("probe_failed")?;
    let temp_dir = std::env::temp_dir().join(format!("ninjacrawler-vsig-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).map_err(|_| "temporary_directory")?;
    let sheet_path = temp_dir.join("samples.png");

    let mut command = Command::new(ffmpeg);
    configure_child(&mut command);
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    for position in VIDEO_SIGNATURE_POSITIONS {
        command
            .arg("-ss")
            .arg(format!("{:.3}", duration_seconds * position))
            .arg("-i")
            .arg(path);
    }
    command
        .arg("-filter_complex")
        .arg("[0:v]scale=64:64[v0];[1:v]scale=64:64[v1];[2:v]scale=64:64[v2];[3:v]scale=64:64[v3];[4:v]scale=64:64[v4];[v0][v1][v2][v3][v4]hstack=inputs=5[out]")
        .args(["-map", "[out]", "-frames:v", "1"])
        .arg(&sheet_path);
    let result = match command.spawn() {
        Ok(mut child) => {
            #[cfg(windows)]
            let _job = KillOnCloseJob::attach(&child);
            wait_for_child(&mut child, cancel, PROCESS_TIMEOUT)
        }
        Err(_) => Err("process_failed"),
    };
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&temp_dir);
        return Err(error);
    }

    let sheet = image::open(&sheet_path).map_err(|_| "frame_decode_failed")?;
    if sheet.width() < 64 * VIDEO_SIGNATURE_POSITIONS.len() as u32 || sheet.height() < 64 {
        let _ = std::fs::remove_dir_all(&temp_dir);
        return Err("frame_decode_failed");
    }
    let mut hashes = Vec::with_capacity(VIDEO_SIGNATURE_POSITIONS.len());
    for index in 0..VIDEO_SIGNATURE_POSITIONS.len() {
        let frame = sheet.crop_imm(index as u32 * 64, 0, 64, 64);
        let (_, dhash) = media_dedupe_runtime::image_hashes(&frame);
        hashes.push(dhash);
    }
    let _ = std::fs::remove_dir_all(&temp_dir);
    serde_json::to_string(&hashes).map_err(|_| "signature_encode_failed")
}

#[derive(Default)]
struct FingerprintOutcome {
    settled_jobs: i64,
    phase_done: i64,
    phase_failed: i64,
    bytes: i64,
}

fn process_fingerprint(
    item: &workspace_repository::PendingFingerprint,
    cancel: &AtomicBool,
) -> FingerprintOutcome {
    if cancel.load(Ordering::SeqCst) {
        return FingerprintOutcome::default();
    }
    let result = match item.kind.as_str() {
        "exact" => file_sha256(&item.absolute_path).map(|sha| {
            (Some(sha), None, None, None, None, None)
        }),
        "perceptual_image" => std::fs::read(&item.absolute_path)
            .map_err(|_| "unreadable")
            .and_then(|bytes| {
                let image = image::load_from_memory(&bytes).map_err(|_| "image_decode_failed")?;
                let (ahash, dhash) = media_dedupe_runtime::image_hashes(&image);
                let sha = item.also_needs_exact.then(|| bytes_sha256(&bytes));
                Ok((
                    sha,
                    Some(ahash),
                    Some(dhash),
                    None,
                    Some(i64::from(image.width())),
                    Some(i64::from(image.height())),
                ))
            }),
        "perceptual_video" => video_signature(&item.absolute_path, item.duration_ms, cancel).and_then(
            |signature| {
                let sha = if item.also_needs_exact {
                    Some(file_sha256(&item.absolute_path)?)
                } else {
                    None
                };
                Ok((sha, None, None, Some(signature), None, None))
            }),
        _ => Err("unsupported_job"),
    };

    match result {
        Ok((sha, ahash, dhash, signature, width, height)) => {
            let stored = workspace_repository::complete_media_fingerprint_job(
                item,
                sha.as_deref(),
                ahash.as_deref(),
                dhash.as_deref(),
                signature.as_deref(),
                width,
                height,
            )
            .unwrap_or(false);
            FingerprintOutcome {
                settled_jobs: if stored {
                    1 + i64::from(item.also_needs_exact && sha.is_some())
                } else {
                    0
                },
                phase_done: i64::from(stored),
                phase_failed: 0,
                bytes: if stored { item.size_bytes } else { 0 },
            }
        }
        Err("cancelled") => FingerprintOutcome::default(),
        Err(error) => {
            let terminal = workspace_repository::mark_fingerprint_job_failed(item, error)
                .unwrap_or(false);
            FingerprintOutcome {
                settled_jobs: i64::from(terminal),
                phase_done: i64::from(terminal),
                phase_failed: i64::from(terminal),
                bytes: 0,
            }
        }
    }
}

fn run_phase(
    app: &AppHandle,
    cancel: &Arc<AtomicBool>,
    profile: &Arc<Mutex<String>>,
    lease_owner: &str,
    kind: &str,
    stage: &str,
    initial_total: i64,
) {
    update_run(app, true, |run| {
        run.stage = stage.to_string();
        run.phase_total = initial_total;
        run.phase_done = 0;
        run.phase_failed = 0;
        run.last_progress_at = Some(Utc::now().to_rfc3339());
        run.rate_per_second = 0.0;
        run.eta_seconds = None;
    });
    let phase_started = Instant::now();
    let mut last_snapshot = Instant::now();
    let mut phase_done = 0_i64;
    let mut phase_failed = 0_i64;
    let mut settled_jobs = 0_i64;
    let mut bytes = 0_i64;

    while !cancel.load(Ordering::SeqCst) {
        let workers = worker_count(kind, &current_profile(profile));
        let batch = (workers * 4).clamp(4, 64) as u32;
        let Ok(mut pending) = workspace_repository::lease_pending_fingerprints(kind, batch, lease_owner)
        else {
            break;
        };
        if pending.is_empty() {
            break;
        }
        pending.reverse();
        let queue = Arc::new(Mutex::new(pending));
        let (sender, receiver) = mpsc::channel();
        let mut handles = Vec::with_capacity(workers);
        for _ in 0..workers {
            let queue = Arc::clone(&queue);
            let cancel = Arc::clone(cancel);
            let sender = sender.clone();
            handles.push(std::thread::spawn(move || loop {
                let next = queue.lock().ok().and_then(|mut queue| queue.pop());
                let Some(item) = next else {
                    return;
                };
                let outcome = process_fingerprint(&item, &cancel);
                let _ = sender.send(outcome);
                if cancel.load(Ordering::SeqCst) {
                    return;
                }
            }));
        }
        drop(sender);

        while let Ok(outcome) = receiver.recv() {
            phase_done += outcome.phase_done;
            phase_failed += outcome.phase_failed;
            settled_jobs += outcome.settled_jobs;
            bytes += outcome.bytes;
            if last_snapshot.elapsed() >= SNAPSHOT_INTERVAL {
                let elapsed = phase_started.elapsed().as_secs_f64().max(0.001);
                let rate = phase_done as f64 / elapsed;
                let remaining = initial_total.saturating_sub(phase_done);
                update_run(app, false, |run| {
                    run.phase_done = phase_done;
                    run.phase_failed = phase_failed;
                    run.fingerprints_done += settled_jobs;
                    settled_jobs = 0;
                    run.bytes_processed += bytes;
                    bytes = 0;
                    run.last_progress_at = Some(Utc::now().to_rfc3339());
                    run.rate_per_second = rate;
                    run.eta_seconds = (rate > 0.0).then(|| (remaining as f64 / rate) as i64);
                });
                last_snapshot = Instant::now();
            }
        }
        for handle in handles {
            let _ = handle.join();
        }
    }

    update_run(app, true, |run| {
        run.phase_done = phase_done;
        run.phase_failed = phase_failed;
        run.fingerprints_done += settled_jobs;
        run.bytes_processed += bytes;
        run.last_progress_at = Some(Utc::now().to_rfc3339());
        let elapsed = phase_started.elapsed().as_secs_f64().max(0.001);
        run.rate_per_second = phase_done as f64 / elapsed;
        run.eta_seconds = None;
    });
}

fn finish_fingerprint_run(
    app: &AppHandle,
    cancel: &Arc<AtomicBool>,
    profile: &Arc<Mutex<String>>,
    _source_ids: Vec<String>,
) {
    update_run(app, true, |run| {
        run.stage = "planning".to_string();
        run.last_progress_at = Some(Utc::now().to_rfc3339());
    });
    let planned = match workspace_repository::plan_media_fingerprint_jobs() {
        Ok(value) => value,
        Err(error) => {
            complete_run(app, false, Some(error));
            return;
        }
    };
    update_run(app, true, |run| {
        run.fingerprints_total = planned.pending();
        run.fingerprints_done = 0;
        run.fingerprint_started_at = Some(Utc::now().to_rfc3339());
    });
    let lease_owner = runtime_state()
        .lock()
        .ok()
        .and_then(|state| state.run.as_ref().map(|run| run.id.clone()))
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    // Perceptual jobs run first so their single file read can also settle an
    // exact job for the same media. Exact-only work then drains the remainder.
    run_phase(
        app,
        cancel,
        profile,
        &lease_owner,
        "perceptual_image",
        "image_similarity",
        planned.perceptual_image,
    );
    run_phase(
        app,
        cancel,
        profile,
        &lease_owner,
        "perceptual_video",
        "video_similarity",
        planned.perceptual_video,
    );
    run_phase(
        app,
        cancel,
        profile,
        &lease_owner,
        "exact",
        "exact",
        planned.exact,
    );

    if cancel.load(Ordering::SeqCst) {
        let _ = workspace_repository::release_media_fingerprint_leases(&lease_owner);
        complete_run(app, true, None);
        return;
    }

    update_run(app, true, |run| {
        run.stage = "grouping".to_string();
        run.phase_total = 1;
        run.phase_done = 0;
        run.phase_failed = 0;
    });
    let failed = workspace_repository::detect_variants_for_all_scopes().is_err();
    update_run(app, true, |run| {
        run.phase_done = 1;
        run.phase_failed = i64::from(failed);
        run.last_progress_at = Some(Utc::now().to_rfc3339());
    });
    let _ = workspace_repository::finalize_media_fingerprint_jobs();
    refresh_cached_counts();
    complete_run(app, cancel.load(Ordering::SeqCst), None);
}

fn complete_run(app: &AppHandle, paused: bool, error: Option<String>) {
    update_run(app, true, |run| {
        run.status = if paused {
            "paused".to_string()
        } else if error.is_some() {
            "failed".to_string()
        } else {
            "completed".to_string()
        };
        if !paused {
            run.stage = "done".to_string();
            run.finished_at = Some(Utc::now().to_rfc3339());
        }
        run.current_source_handle = None;
        if error.is_some() {
            run.error = error;
        }
    });
    if let Ok(mut state) = runtime_state().lock() {
        state.cancel = None;
        state.resource_profile = None;
    }
    publish(app);
}

fn run_scan(
    app: AppHandle,
    targets: Vec<(String, String)>,
    cancel: Arc<AtomicBool>,
    profile: Arc<Mutex<String>>,
) {
    let mut failures = Vec::new();
    let source_ids = targets.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
    for (source_id, handle) in targets {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        update_run(&app, false, |run| {
            run.current_source_handle = Some(handle.clone());
            run.last_progress_at = Some(Utc::now().to_rfc3339());
        });
        match workspace_repository::reconcile_source_media_index(&source_id) {
            Ok(outcome) => update_run(&app, false, |run| {
                run.files_indexed += outcome.indexed as i64;
                run.files_updated += outcome.updated as i64;
                run.files_missing += outcome.missing as i64;
                run.hashes_inherited += outcome.inherited as i64;
                run.sources_processed += 1;
            }),
            Err(error) => {
                failures.push(format!("{handle}: {error}"));
                update_run(&app, false, |run| run.sources_processed += 1);
            }
        }
    }
    if cancel.load(Ordering::SeqCst) {
        complete_run(&app, true, None);
        return;
    }
    if !failures.is_empty() {
        update_run(&app, true, |run| run.error = Some(failures.join("; ")));
    }
    finish_fingerprint_run(&app, &cancel, &profile, source_ids);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_lanes_limit_storage_pressure() {
        assert_eq!(worker_count("exact", "quiet"), 1);
        assert_eq!(worker_count("exact", "balanced"), 1);
        assert_eq!(worker_count("exact", "fast"), 2);
        assert_eq!(worker_count("perceptual_video", "fast"), 2);
    }

    #[test]
    fn resource_profiles_are_normalized() {
        assert_eq!(normalize_resource_profile(Some("FAST".to_string())), "fast");
        assert_eq!(normalize_resource_profile(Some("other".to_string())), "balanced");
    }

    #[derive(Default)]
    struct FakeProcess {
        polls_before_exit: usize,
        polls: usize,
        terminated: bool,
    }

    impl ProcessControl for FakeProcess {
        fn poll(&mut self) -> Result<Option<bool>, ()> {
            self.polls += 1;
            Ok((self.polls > self.polls_before_exit).then_some(true))
        }

        fn terminate(&mut self) {
            self.terminated = true;
        }
    }

    #[test]
    fn fake_process_is_terminated_on_timeout() {
        let mut process = FakeProcess { polls_before_exit: usize::MAX, ..Default::default() };
        let cancel = AtomicBool::new(false);
        let result = wait_for_process(
            &mut process,
            &cancel,
            Duration::ZERO,
            Duration::ZERO,
        );
        assert_eq!(result, Err("timeout"));
        assert!(process.terminated);
    }

    #[test]
    fn fake_process_is_terminated_immediately_on_cancel() {
        let mut process = FakeProcess::default();
        let cancel = AtomicBool::new(true);
        let result = wait_for_process(
            &mut process,
            &cancel,
            Duration::from_secs(1),
            Duration::ZERO,
        );
        assert_eq!(result, Err("cancelled"));
        assert!(process.terminated);
        assert_eq!(process.polls, 0);
    }
}
