use crate::domain::models::{
    MediaDedupeApplyInput, MediaDedupeEngineStatus, MediaDedupeJobStatus,
    MediaDedupeResultPage, MediaDedupeScanInput, MediaDedupeScanResult,
    MediaDedupeSummaryStatus,
};
use crate::infrastructure::{
    media_dedupe_vdf, media_metadata_probe, media_path_migration_runtime, media_tool_runtime,
    source_delete_runtime, source_sync_runtime, workspace_repository,
};
use chrono::{DateTime, Utc};
use image::{imageops::FilterType, GenericImageView};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

pub const MEDIA_DEDUPE_STATUS_CHANGED_EVENT: &str = "media-dedupe://status-changed";
pub const MEDIA_DEDUPE_RESULTS_CHANGED_EVENT: &str = "media-dedupe://results-changed";

struct RuntimeState {
    job_id: Option<String>,
    state: String,
    stage: String,
    scan_id: Option<String>,
    provider_scope: Option<String>,
    source_scope: Option<String>,
    resource_profile: String,
    scan_profile: String,
    started_at: Option<String>,
    phase_started_at: Option<Instant>,
    files_processed: u64,
    files_total: u64,
    bytes_processed: u64,
    bytes_total: u64,
    current_path: Option<String>,
    current_root: Option<String>,
    error: Option<String>,
    engine_status: Option<MediaDedupeEngineStatus>,
    perceptual_sources_processed: u32,
    perceptual_sources_total: u32,
    perceptual_sources_failed: u32,
    latest_scan: Option<MediaDedupeScanResult>,
    cancel: Arc<AtomicBool>,
    locked_sources: HashSet<String>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            job_id: None,
            state: "idle".to_string(),
            stage: "idle".to_string(),
            scan_id: None,
            provider_scope: None,
            source_scope: None,
            resource_profile: "balanced".to_string(),
            scan_profile: "recommended".to_string(),
            started_at: None,
            phase_started_at: None,
            files_processed: 0,
            files_total: 0,
            bytes_processed: 0,
            bytes_total: 0,
            current_path: None,
            current_root: None,
            error: None,
            engine_status: None,
            perceptual_sources_processed: 0,
            perceptual_sources_total: 0,
            perceptual_sources_failed: 0,
            latest_scan: None,
            cancel: Arc::new(AtomicBool::new(false)),
            locked_sources: HashSet::new(),
        }
    }
}

fn runtime_state() -> &'static Mutex<RuntimeState> {
    static STATE: OnceLock<Mutex<RuntimeState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(RuntimeState::default()))
}

pub fn recover_interrupted_jobs(app: &AppHandle) {
    let _ = workspace_repository::recover_interrupted_media_dedupe_jobs();
    let latest = workspace_repository::load_latest_media_dedupe_scan().ok().flatten();
    if let Ok(mut state) = runtime_state().lock() {
        state.engine_status = Some(media_dedupe_vdf::status());
        state.latest_scan = latest.clone();
    }
    if let Some(scan) = latest.as_ref() {
        spawn_missing_video_thumbnail_generation(app, scan);
        spawn_missing_media_metadata_probe(app, scan);
    }
    publish(app);
    publish_results_changed(app);
}

pub fn is_source_locked(source_id: &str) -> bool {
    runtime_state()
        .lock()
        .map(|state| state.locked_sources.contains(source_id))
        .unwrap_or(false)
}

pub fn media_dedupe_status() -> Result<MediaDedupeJobStatus, String> {
    let state = runtime_state().lock().map_err(|error| error.to_string())?;
    let source_job_scan_id = state
        .scan_id
        .clone()
        .or_else(|| state.latest_scan.as_ref().map(|scan| scan.scan_id.clone()));
    let mut status = status_from_state(&state);
    drop(state);
    if let Some(scan_id) = source_job_scan_id {
        status.source_jobs = workspace_repository::load_media_dedupe_source_jobs(&scan_id)?;
    }
    Ok(status)
}

pub fn media_dedupe_summary_status() -> Result<MediaDedupeSummaryStatus, String> {
    let state = runtime_state().lock().map_err(|error| error.to_string())?;
    Ok(summary_from_state(&state))
}

pub fn media_dedupe_result_page(
    exact_offset: usize,
    similar_offset: usize,
    page_size: usize,
) -> Result<Option<MediaDedupeResultPage>, String> {
    let state = runtime_state().lock().map_err(|error| error.to_string())?;
    let Some(scan) = state.latest_scan.as_ref() else {
        return Ok(None);
    };
    Ok(Some(result_page_from_scan(
        scan,
        exact_offset,
        similar_offset,
        page_size,
    )))
}

pub fn install_similarity_engine(app: &AppHandle) -> Result<MediaDedupeSummaryStatus, String> {
    {
        let mut state = runtime_state().lock().map_err(|error| error.to_string())?;
        if matches!(state.state.as_str(), "queued" | "scanning" | "applying") {
            return Err(
                "Wait for media cleanup to finish before changing its runtime.".to_string(),
            );
        }
        if state
            .engine_status
            .as_ref()
            .is_some_and(|status| status.status == "installing")
        {
            return Ok(summary_from_state(&state));
        }
        let tools = media_tool_runtime::status();
        state.engine_status = Some(MediaDedupeEngineStatus {
            status: "installing".to_string(),
            version: media_dedupe_vdf::VDF_VERSION.to_string(),
            installed: false,
            ffmpeg_available: tools.available,
            ffmpeg_status: tools.status,
            ffmpeg_source: tools.source,
            ffmpeg_version: tools.version,
            ffmpeg_install_path: tools.install_path,
            ffmpeg_error: tools.error,
            install_path: None,
            error: None,
        });
    }
    publish(app);
    let app = app.clone();
    std::thread::spawn(move || {
        let installed = media_dedupe_vdf::install();
        update_state(&app, |state| {
            state.engine_status = Some(match installed {
                Ok(status) => status,
                Err(error) => {
                    let tools = media_tool_runtime::status();
                    MediaDedupeEngineStatus {
                        status: "error".to_string(),
                        version: media_dedupe_vdf::VDF_VERSION.to_string(),
                        installed: false,
                        ffmpeg_available: tools.available,
                        ffmpeg_status: tools.status,
                        ffmpeg_source: tools.source,
                        ffmpeg_version: tools.version,
                        ffmpeg_install_path: tools.install_path,
                        ffmpeg_error: tools.error,
                        install_path: None,
                        error: Some(error),
                    }
                }
            });
        });
    });
    media_dedupe_summary_status()
}

pub fn install_ffmpeg_runtime(app: &AppHandle) -> Result<MediaDedupeSummaryStatus, String> {
    {
        let mut state = runtime_state().lock().map_err(|error| error.to_string())?;
        if matches!(state.state.as_str(), "queued" | "scanning" | "applying") {
            return Err(
                "Wait for media cleanup to finish before changing its runtime.".to_string(),
            );
        }
        let mut engine = media_dedupe_vdf::status();
        if engine.ffmpeg_status == "installing" {
            return Ok(summary_from_state(&state));
        }
        engine.ffmpeg_status = "installing".to_string();
        engine.ffmpeg_available = false;
        engine.ffmpeg_error = None;
        state.engine_status = Some(engine);
    }
    publish(app);
    let app = app.clone();
    std::thread::spawn(move || {
        let installed = media_tool_runtime::install();
        update_state(&app, |state| {
            let mut engine = media_dedupe_vdf::status();
            if let Err(error) = installed {
                engine.ffmpeg_status = "error".to_string();
                engine.ffmpeg_available = false;
                engine.ffmpeg_error = Some(error);
            }
            state.engine_status = Some(engine);
        });
    });
    media_dedupe_summary_status()
}

pub fn enqueue_scan(
    app: &AppHandle,
    input: MediaDedupeScanInput,
) -> Result<MediaDedupeSummaryStatus, String> {
    let provider_scope = normalize_provider_scope(input.provider)?;
    let source_scope = normalize_source_scope(input.source_id);
    let resource_profile = normalize_resource_profile(input.resource_profile)?;
    let scan_profile = normalize_scan_profile(input.scan_profile)?;
    workspace_repository::media_dedupe_scan_context(
        provider_scope.as_deref(),
        source_scope.as_deref(),
    )?;
    let scan_id = Uuid::new_v4().to_string();
    let started_at = Utc::now().to_rfc3339();
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut state = runtime_state().lock().map_err(|error| error.to_string())?;
        if matches!(state.state.as_str(), "queued" | "scanning" | "applying") {
            return Err("Media cleanup is already running.".to_string());
        }
        workspace_repository::begin_media_dedupe_scan(
            &scan_id,
            &started_at,
            provider_scope.as_deref(),
            source_scope.as_deref(),
            &resource_profile,
        )?;
        let job_id = format!("scan:{scan_id}");
        if let Err(error) = workspace_repository::begin_media_dedupe_job(
            &job_id,
            &scan_id,
            "scan",
            "inventory",
            0,
            0,
        ) {
            let _ = workspace_repository::finish_media_dedupe_scan_with_error(
                &scan_id,
                "failed",
                Some(&error),
            );
            return Err(error);
        }
        state.job_id = Some(job_id);
        state.state = "queued".to_string();
        state.stage = "inventory".to_string();
        state.scan_id = Some(scan_id.clone());
        state.provider_scope = provider_scope.clone();
        state.source_scope = source_scope.clone();
        state.resource_profile = resource_profile.clone();
        state.scan_profile = scan_profile.clone();
        state.started_at = Some(started_at);
        state.phase_started_at = Some(Instant::now());
        state.files_processed = 0;
        state.files_total = 0;
        state.bytes_processed = 0;
        state.bytes_total = 0;
        state.current_path = None;
        state.current_root = None;
        state.error = None;
        state.perceptual_sources_processed = 0;
        state.perceptual_sources_total = 0;
        state.perceptual_sources_failed = 0;
        state.cancel = cancel.clone();
    }
    publish(app);

    let app = app.clone();
    std::thread::spawn(move || {
        run_scan(
            app,
            scan_id,
            provider_scope,
            source_scope,
            resource_profile,
            scan_profile,
            cancel,
        )
    });
    media_dedupe_summary_status()
}

pub fn cancel(app: &AppHandle) -> Result<MediaDedupeSummaryStatus, String> {
    {
        let state = runtime_state().lock().map_err(|error| error.to_string())?;
        if !matches!(state.state.as_str(), "queued" | "scanning") {
            return Ok(summary_from_state(&state));
        }
        state.cancel.store(true, Ordering::Release);
    }
    update_state(app, |state| state.stage = "cancelling".to_string());
    media_dedupe_summary_status()
}

pub fn enqueue_apply(
    app: &AppHandle,
    input: MediaDedupeApplyInput,
) -> Result<MediaDedupeSummaryStatus, String> {
    let scan = workspace_repository::load_media_dedupe_scan_result(&input.scan_id)?;
    if scan.status != "completed" {
        return Err("Only a completed media scan can be applied.".to_string());
    }
    let locked_sources = affected_sources(&scan, &input);
    {
        let mut state = runtime_state().lock().map_err(|error| error.to_string())?;
        if matches!(state.state.as_str(), "queued" | "scanning" | "applying") {
            return Err("Media cleanup is already running.".to_string());
        }
        state.state = "applying".to_string();
        state.stage = "acquiring_lock".to_string();
        state.locked_sources = locked_sources.clone();
    }
    if let Err(error) = ensure_sources_idle(&locked_sources) {
        if let Ok(mut state) = runtime_state().lock() {
            state.state = "idle".to_string();
            state.stage = "idle".to_string();
            state.locked_sources.clear();
        }
        return Err(error);
    }
    let job_id = Uuid::new_v4().to_string();
    if let Err(error) = workspace_repository::begin_media_dedupe_job(
        &job_id,
        &input.scan_id,
        "apply",
        "preparing",
        apply_target_count(&scan, &input),
        scan.reclaimable_bytes,
    ) {
        if let Ok(mut state) = runtime_state().lock() {
            state.state = "idle".to_string();
            state.stage = "idle".to_string();
            state.locked_sources.clear();
        }
        return Err(error);
    }
    {
        let mut state = runtime_state().lock().map_err(|error| error.to_string())?;
        state.job_id = Some(job_id);
        state.state = "applying".to_string();
        state.stage = "preparing".to_string();
        state.scan_id = Some(input.scan_id.clone());
        state.resource_profile = scan.resource_profile.clone();
        state.provider_scope = scan.provider_scope.clone();
        state.source_scope = scan.source_scope.clone();
        state.started_at = Some(Utc::now().to_rfc3339());
        state.phase_started_at = Some(Instant::now());
        state.files_processed = 0;
        state.files_total = apply_target_count(&scan, &input);
        state.bytes_processed = 0;
        state.bytes_total = scan.reclaimable_bytes;
        state.current_path = None;
        state.current_root = None;
        state.error = None;
        state.locked_sources = locked_sources;
    }
    publish(app);
    let app = app.clone();
    std::thread::spawn(move || run_apply(app, scan, input));
    media_dedupe_summary_status()
}

fn run_scan(
    app: AppHandle,
    scan_id: String,
    provider_scope: Option<String>,
    source_scope: Option<String>,
    resource_profile: String,
    scan_profile: String,
    cancel: Arc<AtomicBool>,
) {
    update_state(&app, |state| {
        state.state = "scanning".to_string();
        state.stage = "inventory".to_string();
    });
    let outcome = scan_library(
        &app,
        &scan_id,
        provider_scope.as_deref(),
        source_scope.as_deref(),
        &resource_profile,
        &scan_profile,
        &cancel,
    );
    match outcome {
        Ok(result) => {
            let enrichment_scan = result.clone();
            update_state(&app, |state| {
                state.state = "completed".to_string();
                state.stage = "completed".to_string();
                state.current_path = None;
                state.current_root = None;
                state.latest_scan = Some(result);
                state.error = None;
            });
            publish_results_changed(&app);
            spawn_missing_video_thumbnail_generation(&app, &enrichment_scan);
            spawn_missing_media_metadata_probe(&app, &enrichment_scan);
        }
        Err(error) if cancel.load(Ordering::Acquire) => {
            let _ = workspace_repository::finish_media_dedupe_scan_with_error(
                &scan_id,
                "cancelled",
                None,
            );
            update_state(&app, |state| {
                state.state = "cancelled".to_string();
                state.stage = "cancelled".to_string();
                state.current_path = None;
                state.current_root = None;
                state.error = None;
            });
            let _ = error;
        }
        Err(error) => {
            let _ = workspace_repository::finish_media_dedupe_scan_with_error(
                &scan_id,
                "failed",
                Some(&error),
            );
            update_state(&app, |state| {
                state.state = "failed".to_string();
                state.stage = "failed".to_string();
                state.current_path = None;
                state.current_root = None;
                state.error = Some(error);
            });
        }
    }
}

fn scan_library(
    app: &AppHandle,
    scan_id: &str,
    provider_scope: Option<&str>,
    source_scope: Option<&str>,
    resource_profile: &str,
    scan_profile: &str,
    cancel: &AtomicBool,
) -> Result<MediaDedupeScanResult, String> {
    let context = workspace_repository::media_dedupe_scan_context(provider_scope, source_scope)?;
    let inventory = collect_inventory(app, scan_id, &context, cancel)?;
    let files_total = inventory.candidate_files;
    let bytes_total = inventory.candidate_bytes;
    update_state(app, |state| {
        state.stage = "hashing_exact_candidates".to_string();
        state.files_processed = 0;
        state.files_total = files_total;
        state.bytes_processed = 0;
        state.bytes_total = bytes_total;
        state.current_path = None;
        state.phase_started_at = Some(Instant::now());
    });
    workspace_repository::update_media_dedupe_scan_progress(
        scan_id,
        "hashing_exact_candidates",
        0,
        files_total,
        0,
        bytes_total,
        None,
    )?;

    let mut skipped_video_similarity_count =
        workspace_repository::media_dedupe_video_count(scan_id)?;
    let mut bytes_processed = 0u64;
    let mut files_processed = 0u64;
    let mut cursor: Option<String> = None;
    loop {
        let candidates = workspace_repository::load_media_dedupe_catalog_candidates(
            scan_id,
            cursor.as_deref(),
            128,
        )?;
        if candidates.is_empty() {
            break;
        }
        for candidate in candidates {
            cursor = Some(candidate.normalized_path.clone());
            if cancel.load(Ordering::Acquire) {
                return Err("Media scan cancelled.".to_string());
            }
            let current = candidate.path.to_string_lossy().to_string();
            let current_root = candidate.root_path.to_string_lossy().to_string();
            let Ok(before) = fs::metadata(&candidate.path) else {
                continue;
            };
            if before.len() != candidate.size_bytes
                || metadata_modified_ms(&before) != candidate.modified_at_ms
            {
                continue;
            }
            let sha256 = match candidate.sha256.clone() {
                Some(value) => value,
                None => match file_sha256(&candidate.path) {
                    Ok(value) => value,
                    Err(_) => continue,
                },
            };
            let (width, height, ahash64, dhash64) = if candidate.media_type == "image"
                && (candidate.width.is_none()
                    || candidate.height.is_none()
                    || candidate.ahash64.is_none()
                    || candidate.dhash64.is_none())
            {
                match image::open(&candidate.path) {
                    Ok(image) => {
                        let (width, height) = image.dimensions();
                        let (ahash, dhash) = image_hashes(&image);
                        (Some(width), Some(height), Some(ahash), Some(dhash))
                    }
                    Err(_) => (None, None, None, None),
                }
            } else {
                (
                    candidate.width,
                    candidate.height,
                    candidate.ahash64.clone(),
                    candidate.dhash64.clone(),
                )
            };
            let indexed = workspace_repository::MediaDedupeIndexedFileOwned {
                scan_id: scan_id.to_string(),
                path: candidate.path.clone(),
                normalized_path: candidate.normalized_path,
                source_id: candidate.source_id,
                provider: candidate.provider,
                root_path: candidate.root_path,
                volume_key: candidate.volume_key,
                media_type: candidate.media_type,
                size_bytes: candidate.size_bytes,
                modified_at_ms: candidate.modified_at_ms,
                sha256,
                width,
                height,
                duration_ms: candidate.duration_ms,
                ahash64,
                dhash64,
                video_hashes_json: None,
            };
            let Ok(after) = fs::metadata(&indexed.path) else {
                continue;
            };
            if before.len() != after.len() || indexed.modified_at_ms != metadata_modified_ms(&after)
            {
                continue;
            }
            if workspace_repository::update_media_dedupe_catalog_hash(scan_id, &indexed).is_err() {
                continue;
            }
            files_processed = files_processed.saturating_add(1);
            bytes_processed = bytes_processed.saturating_add(indexed.size_bytes);
            if files_processed % 10 == 0 || files_processed == files_total {
                update_state(app, |state| {
                    state.files_processed = files_processed;
                    state.bytes_processed = bytes_processed;
                    state.current_path = Some(current.clone());
                    state.current_root = Some(current_root.clone());
                });
                workspace_repository::update_media_dedupe_scan_progress(
                    scan_id,
                    "hashing_exact_candidates",
                    files_processed,
                    files_total,
                    bytes_processed,
                    bytes_total,
                    Some(&current),
                )?;
            }
        }
    }
    update_state(app, |state| {
        state.stage = "grouping".to_string();
        state.files_processed = files_processed;
        state.bytes_processed = bytes_processed;
        state.current_path = None;
        state.current_root = None;
    });
    workspace_repository::update_media_dedupe_scan_progress(
        scan_id,
        "grouping",
        files_processed,
        files_total,
        bytes_processed,
        bytes_total,
        None,
    )?;
    let engine = media_dedupe_vdf::status();
    if engine.installed && engine.ffmpeg_available {
        let (_, total, _, compared_videos) =
            run_perceptual_scans(app, scan_id, &context, resource_profile, scan_profile, cancel)?;
        if total > 0 {
            skipped_video_similarity_count =
                skipped_video_similarity_count.saturating_sub(compared_videos);
        }
    }
    workspace_repository::complete_media_dedupe_scan(scan_id, skipped_video_similarity_count)?;
    workspace_repository::set_media_dedupe_inventory_totals(
        scan_id,
        inventory.files,
        inventory.bytes,
    )
}

fn run_perceptual_scans(
    app: &AppHandle,
    scan_id: &str,
    context: &workspace_repository::MediaDedupeScanContext,
    resource_profile: &str,
    scan_profile: &str,
    cancel: &AtomicBool,
) -> Result<(u32, u32, u32, u32), String> {
    let vdf_profile = media_dedupe_vdf::VdfScanProfile::parse(scan_profile);
    let mut sources = Vec::new();
    for source in &context.source_roots {
        let inventory =
            workspace_repository::media_dedupe_source_video_inventory(scan_id, &source.source_id)?;
        if inventory.video_count > 0 {
            sources.push((source.clone(), inventory));
        }
    }
    let total = sources.len().min(u32::MAX as usize) as u32;
    update_state(app, |state| {
        state.stage = "perceptual_scan".to_string();
        state.phase_started_at = Some(Instant::now());
        state.perceptual_sources_processed = 0;
        state.perceptual_sources_total = total;
        state.perceptual_sources_failed = 0;
        state.files_processed = 0;
        state.files_total = 0;
        state.bytes_processed = 0;
        state.bytes_total = 0;
    });
    if sources.is_empty() {
        return Ok((0, 0, 0, 0));
    }

    let mut volume_groups = BTreeMap::<String, Vec<_>>::new();
    for source in sources {
        volume_groups
            .entry(volume_key(&source.0.path))
            .or_default()
            .push(source);
    }
    let worker_count = volume_groups
        .len()
        .min(resource_profile_source_workers(resource_profile))
        .max(1);
    let mut worker_lanes = (0..worker_count).map(|_| Vec::new()).collect::<Vec<_>>();
    for (index, group) in volume_groups.into_values().enumerate() {
        worker_lanes[index % worker_count].extend(group);
    }

    let settings = media_dedupe_vdf::settings_fingerprint(vdf_profile);
    let options = media_dedupe_vdf::VdfRunOptions::for_profile(resource_profile, worker_count);
    let completed = AtomicU32::new(0);
    let failed = AtomicU32::new(0);
    let compared_videos = AtomicU32::new(0);
    let abort = AtomicBool::new(false);
    let first_error = Mutex::new(None::<String>);

    std::thread::scope(|scope| {
        for lane in worker_lanes {
            let settings = &settings;
            let completed = &completed;
            let failed = &failed;
            let compared_videos = &compared_videos;
            let abort = &abort;
            let first_error = &first_error;
            scope.spawn(move || {
                for (source, inventory) in lane {
                    if cancel.load(Ordering::Acquire) || abort.load(Ordering::Acquire) {
                        break;
                    }
                    let outcome = run_perceptual_source(
                        app, scan_id, &source, &inventory, settings, options, vdf_profile, cancel,
                    );
                    let outcome = match outcome {
                        Ok(outcome) => outcome,
                        Err(error) => {
                            if !cancel.load(Ordering::Acquire) {
                                if let Ok(mut current) = first_error.lock() {
                                    if current.is_none() {
                                        *current = Some(error);
                                    }
                                }
                                abort.store(true, Ordering::Release);
                            }
                            break;
                        }
                    };
                    if outcome.failed {
                        failed.fetch_add(1, Ordering::AcqRel);
                    }
                    compared_videos.fetch_add(outcome.compared_videos, Ordering::AcqRel);
                    let completed_count = completed.fetch_add(1, Ordering::AcqRel) + 1;
                    let failed_count = failed.load(Ordering::Acquire);
                    let source_path = source.path.to_string_lossy();
                    update_state(app, |state| {
                        state.perceptual_sources_processed = completed_count;
                        state.perceptual_sources_failed = failed_count;
                        if state.current_root.as_deref() == Some(source_path.as_ref()) {
                            state.current_path = None;
                            state.current_root = None;
                        }
                    });
                }
            });
        }
    });

    if cancel.load(Ordering::Acquire) {
        return Err("Media scan cancelled.".to_string());
    }
    let error = first_error
        .lock()
        .map_err(|_| "Media cleanup worker state is unavailable.".to_string())?
        .take();
    if let Some(error) = error {
        return Err(error);
    }
    Ok((
        completed.load(Ordering::Acquire),
        total,
        failed.load(Ordering::Acquire),
        compared_videos.load(Ordering::Acquire),
    ))
}

struct PerceptualSourceOutcome {
    failed: bool,
    compared_videos: u32,
}

fn run_perceptual_source(
    app: &AppHandle,
    scan_id: &str,
    source: &workspace_repository::MediaDedupeSourceRoot,
    inventory: &workspace_repository::MediaDedupeSourceVideoInventory,
    settings: &str,
    options: media_dedupe_vdf::VdfRunOptions,
    profile: media_dedupe_vdf::VdfScanProfile,
    cancel: &AtomicBool,
) -> Result<PerceptualSourceOutcome, String> {
    let paths = media_dedupe_vdf::source_paths(scan_id, &source.source_id)?;
    workspace_repository::begin_media_dedupe_source_job(
        &format!("vdf:{scan_id}:{}", source.source_id),
        scan_id,
        &source.source_id,
        &source.provider,
        &source.path,
        media_dedupe_vdf::VDF_VERSION,
        media_dedupe_vdf::runtime_digest(),
        settings,
        &inventory.fingerprint,
        &paths.database_dir,
        &paths.result_path,
    )?;
    if inventory.video_count < 2 {
        workspace_repository::update_media_dedupe_source_job(
            scan_id,
            &source.source_id,
            "completed",
            "not_applicable",
            Some(100),
            inventory.video_count.into(),
            inventory.video_count.into(),
            None,
            None,
        )?;
        return Ok(PerceptualSourceOutcome {
            failed: false,
            compared_videos: inventory.video_count,
        });
    }
    if let Some(previous_scan_id) = workspace_repository::find_reusable_media_dedupe_source_scan(
        scan_id,
        &source.source_id,
        media_dedupe_vdf::runtime_digest(),
        settings,
        &inventory.fingerprint,
    )? {
        workspace_repository::reuse_media_dedupe_vdf_candidates(
            scan_id,
            &source.source_id,
            &previous_scan_id,
        )?;
        workspace_repository::update_media_dedupe_source_job(
            scan_id,
            &source.source_id,
            "completed",
            "cached",
            Some(100),
            inventory.video_count.into(),
            inventory.video_count.into(),
            None,
            None,
        )?;
        return Ok(PerceptualSourceOutcome {
            failed: false,
            compared_videos: inventory.video_count,
        });
    }
    workspace_repository::update_media_dedupe_source_job(
        scan_id,
        &source.source_id,
        "running",
        "scanning",
        None,
        0,
        inventory.video_count.into(),
        Some(&source.path.to_string_lossy()),
        None,
    )?;
    let source_path = source.path.to_string_lossy().to_string();
    let outcome =
        media_dedupe_vdf::run_source_scan(&source.path, &paths, options, profile, cancel, |progress| {
            update_state(app, |state| {
                state.stage = "perceptual_scan".to_string();
                state.files_processed = progress.files_processed;
                state.files_total = progress.files_total;
                state.current_root = Some(source_path.clone());
                state.current_path = progress
                    .current_path
                    .clone()
                    .or_else(|| Some(source_path.clone()));
            });
            let _ = workspace_repository::update_media_dedupe_source_job(
                scan_id,
                &source.source_id,
                "running",
                "scanning",
                progress.percent,
                progress.files_processed,
                progress.files_total,
                progress.current_path.as_deref(),
                None,
            );
        });
    match outcome {
        Ok(candidates) => {
            workspace_repository::replace_media_dedupe_vdf_candidates(
                scan_id,
                &source.source_id,
                media_dedupe_vdf::VDF_VERSION,
                media_dedupe_vdf::runtime_digest(),
                settings,
                &candidates,
            )?;
            workspace_repository::update_media_dedupe_source_job(
                scan_id,
                &source.source_id,
                "completed",
                "completed",
                Some(100),
                inventory.video_count.into(),
                inventory.video_count.into(),
                None,
                None,
            )?;
            Ok(PerceptualSourceOutcome {
                failed: false,
                compared_videos: inventory.video_count,
            })
        }
        Err(error) if cancel.load(Ordering::Acquire) => Err(error),
        Err(error) => {
            workspace_repository::update_media_dedupe_source_job(
                scan_id,
                &source.source_id,
                "failed",
                "failed",
                None,
                0,
                inventory.video_count.into(),
                None,
                Some(&error),
            )?;
            Ok(PerceptualSourceOutcome {
                failed: true,
                compared_videos: 0,
            })
        }
    }
}

fn run_apply(app: AppHandle, scan: MediaDedupeScanResult, input: MediaDedupeApplyInput) {
    let outcome = apply_scan(&app, &scan, &input);
    match outcome {
        Ok(bytes_reclaimed) => update_state(&app, |state| {
            state.state = "completed".to_string();
            state.stage = "completed".to_string();
            state.bytes_processed = bytes_reclaimed;
            state.current_path = None;
            state.current_root = None;
            state.error = None;
            state.locked_sources.clear();
        }),
        Err(error) => update_state(&app, |state| {
            state.state = "failed".to_string();
            state.stage = "failed".to_string();
            state.current_path = None;
            state.current_root = None;
            state.error = Some(error);
            state.locked_sources.clear();
        }),
    }
}

fn apply_scan(
    app: &AppHandle,
    scan: &MediaDedupeScanResult,
    input: &MediaDedupeApplyInput,
) -> Result<u64, String> {
    let mut completed = 0u64;
    let mut reclaimed = 0u64;
    if input.consolidate_exact {
        update_state(app, |state| state.stage = "consolidating_exact".to_string());
        for group in &scan.exact_groups {
            if !group.consolidatable || group.files.len() < 2 {
                continue;
            }
            let mut paths = group
                .files
                .iter()
                .map(|file| file.path.clone())
                .collect::<Vec<_>>();
            paths.sort_by_key(|path| path.to_ascii_lowercase());
            let canonical = PathBuf::from(&paths[0]);
            validate_scanned_file(&scan.scan_id, &canonical)?;
            for target in paths.iter().skip(1) {
                let target_path = PathBuf::from(target);
                validate_scanned_file(&scan.scan_id, &target_path)?;
                completed = completed.saturating_add(1);
                update_state(app, |state| {
                    state.files_processed = completed;
                    state.current_path = Some(target.clone());
                });
                if same_file::is_same_file(&canonical, &target_path).unwrap_or(false) {
                    continue;
                }
                let action_id = Uuid::new_v4().to_string();
                workspace_repository::begin_media_dedupe_action(
                    &action_id,
                    &scan.scan_id,
                    "hardlink",
                    canonical.to_str(),
                    target_path.to_str(),
                )?;
                match consolidate_hardlink(&canonical, &target_path) {
                    Ok(bytes) => {
                        reclaimed = reclaimed.saturating_add(bytes);
                        workspace_repository::finish_media_dedupe_action(
                            &action_id,
                            "succeeded",
                            bytes,
                            None,
                        )?;
                    }
                    Err(error) => {
                        workspace_repository::finish_media_dedupe_action(
                            &action_id,
                            "failed",
                            0,
                            Some(&error),
                        )?;
                        return Err(error);
                    }
                }
            }
        }
    }

    update_state(app, |state| state.stage = "recycling_similar".to_string());
    let groups_by_id = scan
        .similar_groups
        .iter()
        .map(|group| (group.id.as_str(), group))
        .collect::<HashMap<_, _>>();
    for selection in &input.similar_selections {
        let group = groups_by_id
            .get(selection.group_id.as_str())
            .ok_or_else(|| {
                format!(
                    "Similarity group '{}' is not part of this scan.",
                    selection.group_id
                )
            })?;
        if !group
            .files
            .iter()
            .any(|file| file.path == selection.keep_path)
        {
            return Err(format!(
                "Keep path is not part of group '{}'.",
                selection.group_id
            ));
        }
        validate_scanned_file(&scan.scan_id, Path::new(&selection.keep_path))?;
        for target in &selection.remove_paths {
            let file = group
                .files
                .iter()
                .find(|file| file.path == *target)
                .ok_or_else(|| {
                    format!(
                        "Selected path is not part of group '{}'.",
                        selection.group_id
                    )
                })?;
            if target == &selection.keep_path {
                continue;
            }
            validate_scanned_file(&scan.scan_id, Path::new(target))?;
            completed = completed.saturating_add(1);
            update_state(app, |state| {
                state.files_processed = completed;
                state.current_path = Some(target.clone());
            });
            let action_id = Uuid::new_v4().to_string();
            workspace_repository::begin_media_dedupe_action(
                &action_id,
                &scan.scan_id,
                "recycle_similar",
                Some(&selection.keep_path),
                Some(target),
            )?;
            let source_id = file.source_id.as_deref().ok_or_else(|| {
                format!(
                    "'{}' is not associated with a profile and cannot be safely removed.",
                    target
                )
            })?;
            let relative_path = workspace_repository::media_dedupe_source_relative_path(
                source_id,
                Path::new(target),
            )?;
            match workspace_repository::delete_source_media(
                source_id.to_string(),
                vec![relative_path],
            ) {
                Ok(_) => {
                    reclaimed = reclaimed.saturating_add(file.size_bytes);
                    workspace_repository::finish_media_dedupe_action(
                        &action_id,
                        "succeeded",
                        file.size_bytes,
                        None,
                    )?;
                }
                Err(error) => {
                    let message =
                        format!("Failed to move '{}' to the Recycle Bin: {error}", target);
                    workspace_repository::finish_media_dedupe_action(
                        &action_id,
                        "failed",
                        0,
                        Some(&message),
                    )?;
                    return Err(message);
                }
            }
        }
    }
    Ok(reclaimed)
}

fn validate_scanned_file(scan_id: &str, path: &Path) -> Result<(), String> {
    let path_value = path.to_string_lossy();
    let (expected_size, expected_sha) =
        workspace_repository::media_dedupe_file_signature(scan_id, &path_value)?
            .ok_or_else(|| format!("'{}' is not part of this media scan.", path.display()))?;
    let actual_size = fs::metadata(path)
        .map_err(|error| format!("Cannot inspect '{}': {error}", path.display()))?
        .len();
    let actual_sha = file_sha256(path)?;
    if actual_size != expected_size || actual_sha != expected_sha {
        return Err(format!(
            "'{}' changed after the scan; run the scan again.",
            path.display()
        ));
    }
    Ok(())
}

fn consolidate_hardlink(canonical: &Path, target: &Path) -> Result<u64, String> {
    let canonical_meta = fs::metadata(canonical).map_err(|error| error.to_string())?;
    let target_meta = fs::metadata(target).map_err(|error| error.to_string())?;
    if canonical_meta.len() != target_meta.len() || file_sha256(canonical)? != file_sha256(target)?
    {
        return Err(format!(
            "'{}' changed after the scan; run the scan again.",
            target.display()
        ));
    }
    let backup = target.with_file_name(format!(
        ".{}.ninjacrawler-dedupe-{}.backup",
        target
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("media"),
        Uuid::new_v4()
    ));
    fs::rename(target, &backup).map_err(|error| {
        format!(
            "Failed to stage '{}' for consolidation: {error}",
            target.display()
        )
    })?;
    let result = fs::hard_link(canonical, target).and_then(|_| {
        if same_file::is_same_file(canonical, target).unwrap_or(false) {
            Ok(())
        } else {
            Err(std::io::Error::other("hardlink verification failed"))
        }
    });
    if let Err(error) = result {
        let _ = fs::remove_file(target);
        let _ = fs::rename(&backup, target);
        return Err(format!(
            "Failed to consolidate '{}': {error}",
            target.display()
        ));
    }
    if let Err(error) = fs::remove_file(&backup) {
        let _ = fs::remove_file(target);
        let _ = fs::rename(&backup, target);
        return Err(format!(
            "Failed to finalize '{}': {error}",
            target.display()
        ));
    }
    Ok(target_meta.len())
}

fn collect_inventory(
    app: &AppHandle,
    scan_id: &str,
    context: &workspace_repository::MediaDedupeScanContext,
    cancel: &AtomicBool,
) -> Result<workspace_repository::MediaDedupeCatalogStats, String> {
    let source_roots = context
        .source_roots
        .iter()
        .map(|source| {
            (
                normalized_path(&source.path),
                (source.source_id.clone(), source.provider.clone()),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut discovered_files = 0u64;
    let mut discovered_bytes = 0u64;
    let mut catalog_batch = Vec::with_capacity(512);
    let mut last_checkpoint = std::time::Instant::now();
    for root in &context.roots {
        if cancel.load(Ordering::Acquire) {
            return Err("Media scan cancelled.".to_string());
        }
        if !root.exists() {
            continue;
        }
        let root_label = root.to_string_lossy().to_string();
        update_state(app, |state| {
            state.stage = "inventory".to_string();
            state.current_root = Some(root_label.clone());
            state.current_path = Some(root_label.clone());
            state.files_processed = discovered_files;
            state.bytes_processed = discovered_bytes;
        });
        let root_source = source_roots.get(&normalized_path(root)).cloned();
        let mut pending = vec![(root.clone(), root_source)];
        while let Some((directory, inherited_source)) = pending.pop() {
            if cancel.load(Ordering::Acquire) {
                return Err("Media scan cancelled.".to_string());
            }
            let Ok(entries) = fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if file_type.is_dir() {
                    if !excluded_directory(&path) {
                        let source = source_roots
                            .get(&normalized_path(&path))
                            .cloned()
                            .or_else(|| inherited_source.clone());
                        pending.push((path, source));
                    }
                    continue;
                }
                if !file_type.is_file() || excluded_file(&path) || media_type(&path).is_none() {
                    continue;
                }
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                if metadata.len() == 0 {
                    continue;
                }
                let Some(kind) = media_type(&path) else {
                    continue;
                };
                discovered_files = discovered_files.saturating_add(1);
                discovered_bytes = discovered_bytes.saturating_add(metadata.len());
                catalog_batch.push(workspace_repository::MediaDedupeCatalogEntryOwned {
                    normalized_path: normalized_path(&path),
                    source_id: inherited_source.as_ref().map(|value| value.0.clone()),
                    provider: inherited_source.as_ref().map(|value| value.1.clone()),
                    root_path: root.clone(),
                    volume_key: volume_key(&path),
                    media_type: kind.to_string(),
                    size_bytes: metadata.len(),
                    modified_at_ms: metadata_modified_ms(&metadata),
                    path,
                });
                if catalog_batch.len() >= 512 {
                    workspace_repository::persist_media_dedupe_catalog_inventory(
                        scan_id,
                        &catalog_batch,
                    )?;
                    catalog_batch.clear();
                }
                if discovered_files % 250 == 0
                    || last_checkpoint.elapsed() >= std::time::Duration::from_secs(1)
                {
                    let current_path = directory.to_string_lossy().to_string();
                    update_state(app, |state| {
                        state.current_root = Some(root_label.clone());
                        state.current_path = Some(current_path.clone());
                        state.files_processed = discovered_files;
                        state.bytes_processed = discovered_bytes;
                    });
                    workspace_repository::update_media_dedupe_scan_progress(
                        scan_id,
                        "inventory",
                        discovered_files,
                        0,
                        discovered_bytes,
                        0,
                        Some(&current_path),
                    )?;
                    last_checkpoint = std::time::Instant::now();
                }
            }
        }
    }
    workspace_repository::persist_media_dedupe_catalog_inventory(scan_id, &catalog_batch)?;
    workspace_repository::media_dedupe_catalog_stats(scan_id)
}

fn excluded_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| {
            name.eq_ignore_ascii_case(".thumbs")
                || name.eq_ignore_ascii_case("cover")
                || name.eq_ignore_ascii_case("Settings")
        })
}

fn excluded_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    name.eq_ignore_ascii_case("ProfilePicture.jpg")
        || name.eq_ignore_ascii_case("ProfilePicture.jpeg")
        || name.ends_with(".download")
        || name.ends_with(".tmp")
        || name.contains(".ninjacrawler-dedupe-")
}

fn media_type(path: &Path) -> Option<&'static str> {
    match path
        .extension()?
        .to_string_lossy()
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" | "png" | "webp" => Some("image"),
        "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v" => Some("video"),
        _ => None,
    }
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 1024 * 128];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Shared with the media index fingerprint backlog on purpose: hashes computed
/// by two different implementations would never match, and the index inherits
/// hashes from this very catalog.
pub(crate) fn image_hashes(image: &image::DynamicImage) -> (String, String) {
    let average_image = image
        .resize_exact(8, 8, FilterType::Triangle)
        .grayscale()
        .to_luma8();
    let average = average_image
        .pixels()
        .map(|pixel| u64::from(pixel[0]))
        .sum::<u64>()
        / 64;
    let mut ahash = 0u64;
    for (index, pixel) in average_image.pixels().enumerate() {
        if u64::from(pixel[0]) >= average {
            ahash |= 1u64 << index;
        }
    }
    let difference_image = image
        .resize_exact(9, 8, FilterType::Triangle)
        .grayscale()
        .to_luma8();
    let mut dhash = 0u64;
    let mut bit = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            if difference_image.get_pixel(x, y)[0] >= difference_image.get_pixel(x + 1, y)[0] {
                dhash |= 1u64 << bit;
            }
            bit += 1;
        }
    }
    (format!("{ahash:016x}"), format!("{dhash:016x}"))
}

fn metadata_modified_ms(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn normalized_path(path: &Path) -> String {
    let value = path.to_string_lossy().replace('/', "\\");
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value
    }
}

fn volume_key(path: &Path) -> String {
    let value = path.to_string_lossy().replace('/', "\\");
    if value.as_bytes().get(1) == Some(&b':') {
        return value[..2].to_ascii_uppercase();
    }
    if value.starts_with("\\\\") {
        let parts = value
            .trim_start_matches('\\')
            .split('\\')
            .collect::<Vec<_>>();
        if parts.len() >= 2 {
            return format!("\\\\{}\\{}", parts[0], parts[1]).to_ascii_lowercase();
        }
    }
    "/".to_string()
}

fn affected_sources(
    scan: &MediaDedupeScanResult,
    input: &MediaDedupeApplyInput,
) -> HashSet<String> {
    let mut sources = HashSet::new();
    if input.consolidate_exact {
        for group in &scan.exact_groups {
            for file in &group.files {
                if let Some(source_id) = &file.source_id {
                    sources.insert(source_id.clone());
                }
            }
        }
    }
    let selected_groups = input
        .similar_selections
        .iter()
        .map(|selection| selection.group_id.as_str())
        .collect::<HashSet<_>>();
    for group in &scan.similar_groups {
        if selected_groups.contains(group.id.as_str()) {
            for file in &group.files {
                if let Some(source_id) = &file.source_id {
                    sources.insert(source_id.clone());
                }
            }
        }
    }
    sources
}

fn ensure_sources_idle(source_ids: &HashSet<String>) -> Result<(), String> {
    let sync = source_sync_runtime::source_sync_queue_status()?;
    if sync
        .queued_items
        .iter()
        .chain(sync.running_items.iter())
        .any(|job| source_ids.contains(&job.source_id))
    {
        return Err(
            "Wait for affected source sync jobs to finish before applying media cleanup."
                .to_string(),
        );
    }
    let deletes = source_delete_runtime::source_delete_queue_status()?;
    if deletes
        .queued_items
        .iter()
        .chain(deletes.running_items.iter())
        .any(|job| source_ids.contains(&job.source_id))
    {
        return Err(
            "Wait for affected source delete jobs to finish before applying media cleanup."
                .to_string(),
        );
    }
    if source_ids
        .iter()
        .any(|source_id| media_path_migration_runtime::is_source_migrating(source_id))
    {
        return Err(
            "Wait for affected media-path migrations to finish before applying media cleanup."
                .to_string(),
        );
    }
    Ok(())
}

fn apply_target_count(scan: &MediaDedupeScanResult, input: &MediaDedupeApplyInput) -> u64 {
    let exact = if input.consolidate_exact {
        scan.exact_groups
            .iter()
            .filter(|group| group.consolidatable)
            .map(|group| group.files.len().saturating_sub(1) as u64)
            .sum()
    } else {
        0
    };
    exact
        + input
            .similar_selections
            .iter()
            .map(|selection| selection.remove_paths.len() as u64)
            .sum::<u64>()
}

fn update_state(app: &AppHandle, update: impl FnOnce(&mut RuntimeState)) {
    let persisted = if let Ok(mut state) = runtime_state().lock() {
        update(&mut state);
        state.job_id.as_ref().map(|job_id| {
            (
                job_id.clone(),
                state.state.clone(),
                state.stage.clone(),
                state.files_processed,
                state.files_total,
                state.bytes_processed,
                state.bytes_total,
                state.current_path.clone(),
                state.current_root.clone(),
                state.error.clone(),
            )
        })
    } else {
        None
    };
    if let Some((
        job_id,
        status,
        stage,
        files_processed,
        files_total,
        bytes_processed,
        bytes_total,
        current_path,
        current_root,
        error,
    )) = persisted
    {
        let _ = workspace_repository::update_media_dedupe_job(
            &job_id,
            &status,
            &stage,
            files_processed,
            files_total,
            bytes_processed,
            bytes_total,
            current_path.as_deref(),
            current_root.as_deref(),
            error.as_deref(),
        );
    }
    publish(app);
}

fn progress_metrics(state: &RuntimeState) -> (u64, Option<u64>, Option<f64>) {
    let elapsed_seconds = state
        .started_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|started| {
            (Utc::now() - started.with_timezone(&Utc))
                .num_seconds()
                .max(0) as u64
        })
        .unwrap_or(0);
    let (progress_done, progress_total) = if state.stage == "perceptual_scan" {
        (
            u64::from(state.perceptual_sources_processed),
            u64::from(state.perceptual_sources_total),
        )
    } else {
        (state.files_processed, state.files_total)
    };
    let throughput_per_second = state.phase_started_at.and_then(|started| {
        let seconds = started.elapsed().as_secs_f64();
        (seconds >= 1.0 && progress_done > 0).then_some(progress_done as f64 / seconds)
    });
    let estimated_seconds_remaining = throughput_per_second.and_then(|throughput| {
        let enough_samples = if state.stage == "perceptual_scan" {
            progress_done >= 2
        } else {
            progress_done > 0
        };
        (enough_samples && progress_total > progress_done && throughput > 0.0)
            .then_some(((progress_total - progress_done) as f64 / throughput).ceil() as u64)
    });
    (
        elapsed_seconds,
        estimated_seconds_remaining,
        throughput_per_second,
    )
}

fn default_engine_status() -> MediaDedupeEngineStatus {
    MediaDedupeEngineStatus {
        status: "not_installed".to_string(),
        version: media_dedupe_vdf::VDF_VERSION.to_string(),
        installed: false,
        ffmpeg_available: false,
        ffmpeg_status: "not_installed".to_string(),
        ffmpeg_source: None,
        ffmpeg_version: None,
        ffmpeg_install_path: None,
        ffmpeg_error: None,
        install_path: None,
        error: None,
    }
}

fn summary_from_state(state: &RuntimeState) -> MediaDedupeSummaryStatus {
    let (elapsed_seconds, estimated_seconds_remaining, throughput_per_second) =
        progress_metrics(state);
    MediaDedupeSummaryStatus {
        state: state.state.clone(),
        stage: state.stage.clone(),
        scan_id: state.scan_id.clone(),
        provider_scope: state.provider_scope.clone(),
        source_scope: state.source_scope.clone(),
        resource_profile: state.resource_profile.clone(),
        scan_profile: state.scan_profile.clone(),
        similarity_scope: "source".to_string(),
        files_processed: state.files_processed,
        files_total: state.files_total,
        bytes_processed: state.bytes_processed,
        bytes_total: state.bytes_total,
        current_path: state.current_path.clone(),
        current_root: state.current_root.clone(),
        cancellable: matches!(state.state.as_str(), "queued" | "scanning"),
        error: state.error.clone(),
        similarity_engine: state
            .engine_status
            .clone()
            .unwrap_or_else(default_engine_status),
        perceptual_sources_processed: state.perceptual_sources_processed,
        perceptual_sources_total: state.perceptual_sources_total,
        perceptual_sources_failed: state.perceptual_sources_failed,
        elapsed_seconds,
        estimated_seconds_remaining,
        throughput_per_second,
        updated_at: Utc::now().to_rfc3339(),
    }
}

fn scan_summary(scan: &MediaDedupeScanResult) -> MediaDedupeScanResult {
    MediaDedupeScanResult {
        scan_id: scan.scan_id.clone(),
        provider_scope: scan.provider_scope.clone(),
        source_scope: scan.source_scope.clone(),
        resource_profile: scan.resource_profile.clone(),
        similarity_scope: scan.similarity_scope.clone(),
        status: scan.status.clone(),
        files_scanned: scan.files_scanned,
        bytes_scanned: scan.bytes_scanned,
        exact_group_count: scan.exact_group_count,
        similar_group_count: scan.similar_group_count,
        reclaimable_bytes: scan.reclaimable_bytes,
        skipped_video_similarity_count: scan.skipped_video_similarity_count,
        started_at: scan.started_at.clone(),
        finished_at: scan.finished_at.clone(),
        exact_groups: Vec::new(),
        similar_groups: Vec::new(),
    }
}

fn result_page_from_scan(
    scan: &MediaDedupeScanResult,
    exact_offset: usize,
    similar_offset: usize,
    page_size: usize,
) -> MediaDedupeResultPage {
    let page_size = page_size.clamp(1, 100);
    let exact_offset = exact_offset.min(scan.exact_groups.len());
    let similar_offset = similar_offset.min(scan.similar_groups.len());
    let exact_end = exact_offset
        .saturating_add(page_size)
        .min(scan.exact_groups.len());
    let similar_end = similar_offset
        .saturating_add(page_size)
        .min(scan.similar_groups.len());
    MediaDedupeResultPage {
        scan_id: scan.scan_id.clone(),
        exact_groups: scan.exact_groups[exact_offset..exact_end].to_vec(),
        similar_groups: scan.similar_groups[similar_offset..similar_end].to_vec(),
        exact_offset,
        similar_offset,
        exact_total: scan.exact_groups.len(),
        consolidatable_exact_total: scan
            .exact_groups
            .iter()
            .filter(|group| group.consolidatable)
            .count(),
        similar_total: scan.similar_groups.len(),
        page_size,
        has_more: exact_end < scan.exact_groups.len() || similar_end < scan.similar_groups.len(),
    }
}

fn status_from_state(state: &RuntimeState) -> MediaDedupeJobStatus {
    let summary = summary_from_state(state);
    MediaDedupeJobStatus {
        state: summary.state,
        stage: summary.stage,
        scan_id: summary.scan_id,
        provider_scope: summary.provider_scope,
        source_scope: summary.source_scope,
        resource_profile: summary.resource_profile,
        scan_profile: summary.scan_profile,
        similarity_scope: summary.similarity_scope,
        files_processed: summary.files_processed,
        files_total: summary.files_total,
        bytes_processed: summary.bytes_processed,
        bytes_total: summary.bytes_total,
        current_path: summary.current_path,
        current_root: summary.current_root,
        cancellable: summary.cancellable,
        error: summary.error,
        similarity_engine: summary.similarity_engine,
        perceptual_sources_processed: summary.perceptual_sources_processed,
        perceptual_sources_total: summary.perceptual_sources_total,
        perceptual_sources_failed: summary.perceptual_sources_failed,
        elapsed_seconds: summary.elapsed_seconds,
        estimated_seconds_remaining: summary.estimated_seconds_remaining,
        throughput_per_second: summary.throughput_per_second,
        source_jobs: Vec::new(),
        latest_scan: state.latest_scan.as_ref().map(scan_summary),
        updated_at: summary.updated_at,
    }
}

fn normalize_resource_profile(profile: Option<String>) -> Result<String, String> {
    let profile = profile
        .unwrap_or_else(|| "balanced".to_string())
        .trim()
        .to_ascii_lowercase();
    if !matches!(profile.as_str(), "quiet" | "balanced" | "fast") {
        return Err(format!(
            "Unsupported media cleanup resource profile: {profile}."
        ));
    }
    Ok(profile)
}

fn normalize_scan_profile(profile: Option<String>) -> Result<String, String> {
    let profile = profile
        .unwrap_or_else(|| "recommended".to_string())
        .trim()
        .to_ascii_lowercase();
    match profile.as_str() {
        "recommended" | "ai" | "deep" => Ok(profile),
        other => Err(format!("Unsupported media cleanup scan profile: {other}.")),
    }
}

fn resource_profile_source_workers(profile: &str) -> usize {
    match profile {
        "quiet" => 1,
        "fast" => 4,
        _ => 2,
    }
}

fn normalize_provider_scope(provider: Option<String>) -> Result<Option<String>, String> {
    let Some(provider) = provider else {
        return Ok(None);
    };
    let provider = provider.trim().to_ascii_lowercase();
    if provider.is_empty() || provider == "all" {
        return Ok(None);
    }
    if crate::providers::provider_runtime(&provider).is_none() {
        return Err(format!(
            "Unsupported media cleanup provider scope: {provider}."
        ));
    }
    Ok(Some(provider))
}

fn normalize_source_scope(source_id: Option<String>) -> Option<String> {
    source_id.map(|value| value.trim().to_string()).filter(|value| !value.is_empty())
}

fn publish(app: &AppHandle) {
    if let Ok(status) = media_dedupe_summary_status() {
        let _ = app.emit(MEDIA_DEDUPE_STATUS_CHANGED_EVENT, status);
    }
}

fn publish_results_changed(app: &AppHandle) {
    let _ = app.emit(MEDIA_DEDUPE_RESULTS_CHANGED_EVENT, ());
}

fn refresh_cached_scan_assets(app: &AppHandle) {
    let mut refreshed = runtime_state()
        .lock()
        .ok()
        .and_then(|state| state.latest_scan.clone());
    let Some(scan) = refreshed.as_mut() else {
        return;
    };
    workspace_repository::refresh_media_dedupe_thumbnail_paths(scan);
    workspace_repository::refresh_media_dedupe_metadata(scan);
    if let Ok(mut state) = runtime_state().lock() {
        if state
            .latest_scan
            .as_ref()
            .is_some_and(|current| current.scan_id == scan.scan_id)
        {
            state.latest_scan = refreshed;
        }
    }
    publish_results_changed(app);
}

/// Paths of grouped video files (exact + similar duplicate groups) that don't have a
/// resolved thumbnail yet. Pure/testable — the caller decides what to do with the result.
fn grouped_video_files_missing_thumbnails(scan: &MediaDedupeScanResult) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut missing = Vec::new();
    for group in scan.exact_groups.iter().chain(scan.similar_groups.iter()) {
        for file in &group.files {
            if file.media_type != "video" || file.thumbnail_path.is_some() {
                continue;
            }
            if seen.insert(file.path.clone()) {
                missing.push(file.path.clone());
            }
        }
    }
    missing
}

fn video_thumbnail_generation_in_flight() -> &'static Mutex<HashSet<String>> {
    static IN_FLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    IN_FLIGHT.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Generates missing thumbnails for videos that ended up in a dedupe result group, so the
/// UI stops showing the "VIDEO" placeholder for files the scan already flagged as
/// duplicates. Deliberately scoped to grouped files only (not the whole scanned
/// inventory) and runs off the calling thread — thumbnail generation (ffmpeg) must never
/// block serving scan results. The cached scan is refreshed once after generation
/// finishes and the UI is notified through the results-changed event.
fn spawn_missing_video_thumbnail_generation(app: &AppHandle, scan: &MediaDedupeScanResult) {
    let candidates = grouped_video_files_missing_thumbnails(scan);
    if candidates.is_empty() {
        return;
    }
    let to_generate: Vec<String> = {
        let Ok(mut in_flight) = video_thumbnail_generation_in_flight().lock() else {
            return;
        };
        candidates
            .into_iter()
            .filter(|path| in_flight.insert(path.clone()))
            .collect()
    };
    if to_generate.is_empty() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        for path in &to_generate {
            // `generate_media_thumbnail` is idempotent (skips already-current thumbs)
            // and classifies failures instead of panicking/propagating: a missing
            // ffmpeg or a corrupt video is logged and skipped, never fails the caller.
            let _ = workspace_repository::generate_media_thumbnail(Path::new(path));
        }
        if let Ok(mut in_flight) = video_thumbnail_generation_in_flight().lock() {
            for path in &to_generate {
                in_flight.remove(path);
            }
        }
        refresh_cached_scan_assets(&app);
    });
}

/// Paths of grouped video files that don't have ffprobe metadata (bitrate/codec/
/// fps/audio) resolved yet. Pure/testable, same shape as
/// `grouped_video_files_missing_thumbnails` — the caller decides what to do.
fn grouped_video_files_missing_metadata(scan: &MediaDedupeScanResult) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut missing = Vec::new();
    for group in scan.exact_groups.iter().chain(scan.similar_groups.iter()) {
        for file in &group.files {
            if file.media_type != "video" {
                continue;
            }
            let has_metadata = file.bitrate_kbps.is_some()
                || file.video_codec.is_some()
                || file.frame_rate.is_some()
                || file.audio_summary.is_some();
            if has_metadata {
                continue;
            }
            if seen.insert(file.path.clone()) {
                missing.push(file.path.clone());
            }
        }
    }
    missing
}

fn media_metadata_probe_in_flight() -> &'static Mutex<HashSet<String>> {
    static IN_FLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    IN_FLIGHT.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Probes ffprobe metadata (bitrate/codec/fps/audio) for videos that ended up in
/// a dedupe result group, so the UI's metadata columns fill in progressively
/// instead of showing `–` forever. Scoped to grouped files only, runs off the
/// calling thread — ffprobe must never block serving scan results. The cached scan
/// is refreshed once after probing finishes and then published to interested views.
fn spawn_missing_media_metadata_probe(app: &AppHandle, scan: &MediaDedupeScanResult) {
    let candidates = grouped_video_files_missing_metadata(scan);
    if candidates.is_empty() {
        return;
    }
    let to_probe: Vec<String> = {
        let Ok(mut in_flight) = media_metadata_probe_in_flight().lock() else {
            return;
        };
        candidates
            .into_iter()
            .filter(|path| in_flight.insert(path.clone()))
            .collect()
    };
    if to_probe.is_empty() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        for path in &to_probe {
            media_metadata_probe::probe_and_cache(Path::new(path));
        }
        if let Ok(mut in_flight) = media_metadata_probe_in_flight().lock() {
            for path in &to_probe {
                in_flight.remove(path);
            }
        }
        refresh_cached_scan_assets(&app);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::{MediaDedupeFile, MediaDedupeGroup};

    #[test]
    fn scanner_excludes_generated_and_profile_assets() {
        assert!(excluded_directory(Path::new(r"C:\media\.thumbs")));
        assert!(excluded_directory(Path::new(r"C:\media\cover")));
        assert!(excluded_directory(Path::new(
            r"S:\NinjaCrawler\Twitter\JessKsada\Settings"
        )));
        // `excluded_directory` only judges the directory itself; nested folders like
        // `Settings\Pictures` are never visited because the walker (see
        // `collect_inventory`) skips pushing excluded directories onto its traversal
        // stack, so their children are never enumerated either.
        assert!(!excluded_directory(Path::new(
            r"S:\NinjaCrawler\Twitter\JessKsada\SettingsBackup"
        )));
        assert!(excluded_file(Path::new(r"C:\media\ProfilePicture.jpg")));
        assert!(excluded_file(Path::new(r"C:\media\video.mp4.download")));
        assert!(!excluded_file(Path::new(r"C:\media\post.jpg")));
    }

    #[test]
    fn image_hashes_are_stable_for_identical_pixels() {
        let image = image::DynamicImage::new_rgb8(16, 16);
        assert_eq!(image_hashes(&image), image_hashes(&image));
    }

    #[test]
    fn provider_scope_accepts_supported_providers_and_normalizes_all() {
        assert_eq!(normalize_provider_scope(None).expect("library"), None);
        assert_eq!(
            normalize_provider_scope(Some(" all ".to_string())).expect("all"),
            None
        );
        assert_eq!(
            normalize_provider_scope(Some("TikTok".to_string())).expect("provider"),
            Some("tiktok".to_string())
        );
        assert!(normalize_provider_scope(Some("unknown".to_string())).is_err());
    }

    #[test]
    fn source_scope_trims_identifiers_and_ignores_empty_values() {
        assert_eq!(
            normalize_source_scope(Some(" source-1 ".to_string())),
            Some("source-1".to_string())
        );
        assert_eq!(normalize_source_scope(Some("  ".to_string())), None);
        assert_eq!(normalize_source_scope(None), None);
    }

    #[test]
    fn resource_profile_defaults_to_balanced_and_rejects_unknown_values() {
        assert_eq!(
            normalize_resource_profile(None).expect("default"),
            "balanced"
        );
        assert_eq!(
            normalize_resource_profile(Some(" FAST ".to_string())).expect("fast"),
            "fast"
        );
        assert!(normalize_resource_profile(Some("unbounded".to_string())).is_err());
        assert_eq!(resource_profile_source_workers("quiet"), 1);
        assert_eq!(resource_profile_source_workers("balanced"), 2);
        assert_eq!(resource_profile_source_workers("fast"), 4);
    }

    #[test]
    fn perceptual_eta_waits_for_more_than_one_completed_source() {
        let mut state = RuntimeState {
            stage: "perceptual_scan".to_string(),
            perceptual_sources_processed: 1,
            perceptual_sources_total: 10,
            phase_started_at: Some(Instant::now() - std::time::Duration::from_secs(10)),
            ..RuntimeState::default()
        };
        assert!(status_from_state(&state)
            .estimated_seconds_remaining
            .is_none());

        state.perceptual_sources_processed = 2;
        assert!(status_from_state(&state)
            .estimated_seconds_remaining
            .is_some());
    }

    #[test]
    fn exact_consolidation_preserves_both_paths_as_hardlinks() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let canonical = temp.path().join("canonical.jpg");
        let duplicate = temp.path().join("duplicate.jpg");
        fs::write(&canonical, b"identical media").expect("canonical media");
        fs::write(&duplicate, b"identical media").expect("duplicate media");

        let reclaimed =
            consolidate_hardlink(&canonical, &duplicate).expect("hardlink consolidation");

        assert_eq!(reclaimed, 15);
        assert!(canonical.exists());
        assert!(duplicate.exists());
        assert!(same_file::is_same_file(&canonical, &duplicate).expect("hardlink identity"));
    }

    fn media_file(path: &str, media_type: &str, thumbnail_path: Option<&str>) -> MediaDedupeFile {
        MediaDedupeFile {
            path: path.to_string(),
            source_id: Some("source-1".to_string()),
            provider: Some("instagram".to_string()),
            media_type: media_type.to_string(),
            size_bytes: 1,
            width: None,
            height: None,
            duration_ms: None,
            thumbnail_path: thumbnail_path.map(str::to_string),
            modified_at_ms: None,
            bitrate_kbps: None,
            video_codec: None,
            frame_rate: None,
            audio_summary: None,
        }
    }

    fn media_file_with_metadata(path: &str, media_type: &str) -> MediaDedupeFile {
        MediaDedupeFile {
            bitrate_kbps: Some(2500),
            video_codec: Some("h264".to_string()),
            frame_rate: Some(30.0),
            audio_summary: Some("aac (stereo)".to_string()),
            ..media_file(path, media_type, None)
        }
    }

    fn group(kind: &str, files: Vec<MediaDedupeFile>) -> MediaDedupeGroup {
        MediaDedupeGroup {
            id: format!("{kind}:1"),
            kind: kind.to_string(),
            confidence_percent: Some(100),
            reclaimable_bytes: 0,
            consolidatable: kind == "exact",
            reason: None,
            files,
        }
    }

    fn scan_result(
        exact_groups: Vec<MediaDedupeGroup>,
        similar_groups: Vec<MediaDedupeGroup>,
    ) -> MediaDedupeScanResult {
        MediaDedupeScanResult {
            scan_id: "scan-1".to_string(),
            provider_scope: None,
            source_scope: None,
            resource_profile: "balanced".to_string(),
            similarity_scope: "source".to_string(),
            status: "completed".to_string(),
            files_scanned: 0,
            bytes_scanned: 0,
            exact_group_count: exact_groups.len() as u32,
            similar_group_count: similar_groups.len() as u32,
            reclaimable_bytes: 0,
            skipped_video_similarity_count: 0,
            started_at: "2026-01-01T00:00:00Z".to_string(),
            finished_at: Some("2026-01-01T00:00:01Z".to_string()),
            exact_groups,
            similar_groups,
        }
    }

    #[test]
    fn summary_status_size_does_not_grow_with_scan_results() {
        let exact_groups = (0..3_000)
            .map(|index| {
                group(
                    "exact",
                    vec![media_file(
                        &format!(r"S:\library\unique-large-result-{index}.mp4"),
                        "video",
                        None,
                    )],
                )
            })
            .collect();
        let mut state = RuntimeState::default();
        state.latest_scan = Some(scan_result(exact_groups, Vec::new()));

        let payload = serde_json::to_vec(&summary_from_state(&state)).expect("summary payload");
        let detail = status_from_state(&state);

        assert!(payload.len() < 4_096, "summary grew to {} bytes", payload.len());
        assert!(!String::from_utf8_lossy(&payload).contains("unique-large-result"));
        assert!(detail.latest_scan.expect("scan metadata").exact_groups.is_empty());
    }

    #[test]
    fn result_pages_are_bounded_and_report_remaining_groups() {
        let exact_groups = (0..250)
            .map(|index| {
                group(
                    "exact",
                    vec![media_file(&format!(r"S:\library\exact-{index}.jpg"), "image", None)],
                )
            })
            .collect();
        let similar_groups = (0..3)
            .map(|index| {
                group(
                    "similar",
                    vec![media_file(&format!(r"S:\library\similar-{index}.jpg"), "image", None)],
                )
            })
            .collect();
        let scan = scan_result(exact_groups, similar_groups);

        let first = result_page_from_scan(&scan, 0, 0, 1_000);
        assert_eq!(first.page_size, 100);
        assert_eq!(first.exact_groups.len(), 100);
        assert_eq!(first.similar_groups.len(), 3);
        assert_eq!(first.exact_total, 250);
        assert_eq!(first.consolidatable_exact_total, 250);
        assert_eq!(first.similar_total, 3);
        assert!(first.has_more);

        let last = result_page_from_scan(&scan, 200, 3, 100);
        assert_eq!(last.exact_groups.len(), 50);
        assert!(last.similar_groups.is_empty());
        assert!(!last.has_more);
    }

    #[test]
    fn missing_thumbnails_only_selects_grouped_videos_without_a_thumbnail() {
        let scan = scan_result(
            vec![group(
                "exact",
                vec![
                    media_file(r"C:\media\a.mp4", "video", None),
                    // Already has a thumbnail — must not be re-selected.
                    media_file(r"C:\media\b.mp4", "video", Some(r"C:\media\.thumbs\b.mp4.jpg")),
                    // Images are left alone in this pass.
                    media_file(r"C:\media\c.jpg", "image", None),
                ],
            )],
            vec![group(
                "similar",
                vec![media_file(r"C:\media\d.mp4", "video", None)],
            )],
        );

        let mut missing = grouped_video_files_missing_thumbnails(&scan);
        missing.sort();
        assert_eq!(
            missing,
            vec![r"C:\media\a.mp4".to_string(), r"C:\media\d.mp4".to_string()]
        );
    }

    #[test]
    fn missing_thumbnails_deduplicates_a_video_that_appears_in_multiple_groups() {
        let scan = scan_result(
            vec![group(
                "exact",
                vec![media_file(r"C:\media\a.mp4", "video", None)],
            )],
            vec![group(
                "similar",
                vec![media_file(r"C:\media\a.mp4", "video", None)],
            )],
        );

        let missing = grouped_video_files_missing_thumbnails(&scan);
        assert_eq!(missing, vec![r"C:\media\a.mp4".to_string()]);
    }

    #[test]
    fn missing_thumbnails_returns_empty_when_no_scan_data_or_nothing_missing() {
        assert!(grouped_video_files_missing_thumbnails(&scan_result(Vec::new(), Vec::new())).is_empty());

        let scan = scan_result(
            vec![group(
                "exact",
                vec![media_file(
                    r"C:\media\a.mp4",
                    "video",
                    Some(r"C:\media\.thumbs\a.mp4.jpg"),
                )],
            )],
            Vec::new(),
        );
        assert!(grouped_video_files_missing_thumbnails(&scan).is_empty());
    }

    #[test]
    fn missing_metadata_selects_grouped_videos_without_any_probed_field() {
        let scan = scan_result(
            vec![group(
                "exact",
                vec![
                    media_file(r"C:\media\a.mp4", "video", None),
                    // Already probed — must not be re-selected even though only
                    // one field is set (audio_summary alone still counts).
                    media_file_with_metadata(r"C:\media\b.mp4", "video"),
                    // Images are never probed.
                    media_file(r"C:\media\c.jpg", "image", None),
                ],
            )],
            vec![group(
                "similar",
                vec![media_file(r"C:\media\d.mp4", "video", None)],
            )],
        );

        let mut missing = grouped_video_files_missing_metadata(&scan);
        missing.sort();
        assert_eq!(
            missing,
            vec![r"C:\media\a.mp4".to_string(), r"C:\media\d.mp4".to_string()]
        );
    }

    #[test]
    fn missing_metadata_deduplicates_a_video_that_appears_in_multiple_groups() {
        let scan = scan_result(
            vec![group(
                "exact",
                vec![media_file(r"C:\media\a.mp4", "video", None)],
            )],
            vec![group(
                "similar",
                vec![media_file(r"C:\media\a.mp4", "video", None)],
            )],
        );

        let missing = grouped_video_files_missing_metadata(&scan);
        assert_eq!(missing, vec![r"C:\media\a.mp4".to_string()]);
    }

    #[test]
    fn missing_metadata_returns_empty_when_nothing_is_missing() {
        assert!(grouped_video_files_missing_metadata(&scan_result(Vec::new(), Vec::new())).is_empty());

        let scan = scan_result(
            vec![group(
                "exact",
                vec![media_file_with_metadata(r"C:\media\a.mp4", "video")],
            )],
            Vec::new(),
        );
        assert!(grouped_video_files_missing_metadata(&scan).is_empty());
    }

    #[test]
    fn exact_consolidation_rejects_files_changed_after_scan() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let canonical = temp.path().join("canonical.jpg");
        let changed = temp.path().join("changed.jpg");
        fs::write(&canonical, b"original bytes").expect("canonical media");
        fs::write(&changed, b"changed! bytes").expect("changed media");

        assert!(consolidate_hardlink(&canonical, &changed).is_err());
        assert_eq!(
            fs::read(&changed).expect("changed file remains"),
            b"changed! bytes"
        );
        assert!(!same_file::is_same_file(&canonical, &changed).expect("separate files"));
    }
}
