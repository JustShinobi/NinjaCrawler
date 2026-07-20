//! One-shot repair: re-download only the soundtrack for TikTok slideshows already
//! on disk (audio dropped by older connector versions).
//!
//! **Does not use the source-sync queue.** Inline download, 1 attempt per post.
//! Failing posts (deleted/private/misleading TikTok “IP blocked”) enter the
//! inaccessible ledger and are **not retried** until the ledger is cleared.
//!
//! “IP address is blocked” and “No video formats found” are **not** trusted as
//! terminal by themselves (deleted, private, missing soundtrack, or real
//! block can all look like that). Failures in the confirmation set are
//! resolved with per-post evidence (validated 2026-07-19, see
//! `docs/design/slideshow-audio-repair-zero-failures-plan.md`):
//!
//! 1. TikTok **oEmbed** for the post: alive → HTTP 200 + JSON; gone → HTTP 400.
//! 2. On gone, the **profile embed page** refines the cause: public → post
//!    deleted; `errorCode:10221` → account gone; `errorCode:10222` → account
//!    private → one authenticated retry with the account cookies before
//!    marking (private accounts the session follows are still downloadable).
//! 3. oEmbed **alive** → **re-queue unmarked** (the post still needs repair).
//!    Download-path failures (IP block / rate limit / …) also count toward
//!    path-health cooldowns; `photo_no_av_format` does **not** (soundtrack
//!    simply not extractable right now — leave queued for a later run).
//! 4. oEmbed unreachable → live control post distinguishes “down for everyone”
//!    (abort) from a target-only hiccup (requeue).
//!
//! A streak of suspect failures (download failing while posts are provably
//! online) pauses the run with bounded cooldowns and only then aborts —
//! nothing is rolled back because nothing was marked without evidence.
//!
//! Scan and ledger persist under
//! `cache/slideshow-audio-repair/state.json`. Entry point is the headless CLI
//! (`slideshow_audio_repair_cli`); there is no in-app UI panel.
//!
//! Mapping vs Single Videos / TikTok connector:
//! - Single photo: `--impersonate chrome`, `-f ba`, video URL (no cookies on single)
//! - Profile sync batch: cookies + UA + `--sleep-requests 1` + retries 5/3
//! - Repair: `/video/` URL (yt-dlp does NOT support `/photo/`) + `-f ba`;
//!   first attempt without cookies (same as Single Videos — cookies break
//!   “universal data” extraction), cookie fallback for private posts.

use super::*;
use crate::domain::models::{
    SlideshowAudioRepairPreview, SlideshowAudioRepairProfileSummary, SlideshowAudioRepairProgress,
    SlideshowAudioRepairResult, SlideshowAudioRepairSample,
};
use crate::infrastructure::atomic_file;
use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio};
use tauri::{AppHandle, Emitter};

pub const SLIDESHOW_AUDIO_REPAIR_DISMISSED_KEY: &str = "repair.slideshow_audio.dismissed";
pub const SLIDESHOW_AUDIO_REPAIR_PROGRESS_EVENT: &str = "slideshow-audio-repair://progress";
const STATE_FILE_NAME: &str = "state.json";
const SAMPLE_LIMIT: usize = 40;
const PROFILE_SUMMARY_LIMIT: usize = 80;
const FAILURE_LIMIT: usize = 40;
const RECENT_FAILURE_UI_LIMIT: usize = 12;
const INTER_POST_DELAY: std::time::Duration = std::time::Duration::from_secs(1);
const YT_DLP_IMPERSONATE: &str = "chrome";
// Repair: 1 logical attempt per post. High sync-style retries only delay
// deleted posts (TikTok often returns "IP blocked" in a loop).
const YT_DLP_RETRIES: &str = "1";
const YT_DLP_EXTRACTOR_RETRIES: &str = "1";
const YT_DLP_SLEEP_REQUESTS: &str = "1";
// Extra pause after an explicit rate-limit failure before the next job.
const RATE_LIMIT_EXTRA_DELAY: std::time::Duration = std::time::Duration::from_secs(5);
// oEmbed / profile-embed availability probes (plain HTTPS GET, no auth).
const AVAILABILITY_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
const AVAILABILITY_PROBE_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";
// Live control post used to prove the oEmbed endpoint itself is reachable
// when a target check fails at the network level. Preference is the last post
// recovered in this run; this constant is the fallback for runs that have not
// recovered anything yet. It is TikTok's own embed-docs example post
// (validated alive from this codebase on 2026-07-19).
const CONTROL_POST_URL: &str = "https://www.tiktok.com/@scout2015/video/6718335390845095173";
// Download-path health: a streak of suspect failures (downloads failing for
// posts that are provably still online, or availability checks unreachable
// with the control alive) means yt-dlp's path is being blocked/rate-limited.
// The run cools down a bounded number of times before giving up — posts in
// those streaks were never marked, so there is nothing to roll back.
const PATH_SUSPECT_THRESHOLD: usize = 8;
const PATH_COOLDOWN_CYCLES: usize = 2;
const PATH_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(180);

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MissingSlideshowAudio {
    source_id: String,
    handle: String,
    account_id: Option<String>,
    profile_root: String,
    post_id: String,
    image_count: usize,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnavailablePost {
    source_id: String,
    handle: String,
    post_id: String,
    class: String,
    error: String,
    url_photo: String,
    url_video: String,
    failed_at: String,
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RepairPersistedState {
    scanned_at: Option<String>,
    profiles_scanned: usize,
    profiles_with_slideshows: usize,
    slideshows_found: usize,
    already_have_audio: usize,
    missing: Vec<MissingSlideshowAudio>,
    profiles: Vec<SlideshowAudioRepairProfileSummary>,
    /// key = `source_id|post_id`
    unavailable: HashMap<String, UnavailablePost>,
}

type ProgressFn<'a> = dyn Fn(SlideshowAudioRepairProgress) + Send + 'a;

fn repair_cache_dir(layout: &StorageLayout) -> PathBuf {
    layout.cache_root.join("slideshow-audio-repair")
}

fn repair_state_path(layout: &StorageLayout) -> PathBuf {
    repair_cache_dir(layout).join(STATE_FILE_NAME)
}

fn unavailable_key(source_id: &str, post_id: &str) -> String {
    format!("{source_id}|{post_id}")
}

fn load_state(layout: &StorageLayout) -> RepairPersistedState {
    let path = repair_state_path(layout);
    let Ok(raw) = fs::read_to_string(&path) else {
        return RepairPersistedState::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save_state(layout: &StorageLayout, state: &RepairPersistedState) -> Result<(), String> {
    let dir = repair_cache_dir(layout);
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let path = repair_state_path(layout);
    let raw = serde_json::to_string_pretty(state).map_err(|error| error.to_string())?;
    fs::write(path, raw).map_err(|error| error.to_string())
}

fn preview_from_state(dismissed: bool, state: &RepairPersistedState) -> SlideshowAudioRepairPreview {
    let scanned = state.scanned_at.is_some();
    // Missing queue excludes unavailable (download will skip them).
    let actionable: Vec<&MissingSlideshowAudio> = state
        .missing
        .iter()
        .filter(|item| {
            !state
                .unavailable
                .contains_key(&unavailable_key(&item.source_id, &item.post_id))
        })
        .collect();
    let samples: Vec<SlideshowAudioRepairSample> = actionable
        .iter()
        .take(SAMPLE_LIMIT)
        .map(|item| SlideshowAudioRepairSample {
            source_id: item.source_id.clone(),
            handle: item.handle.clone(),
            post_id: item.post_id.clone(),
            image_count: item.image_count,
        })
        .collect();
    SlideshowAudioRepairPreview {
        dismissed,
        scanned,
        scanned_at: state.scanned_at.clone(),
        profiles_scanned: state.profiles_scanned,
        profiles_with_slideshows: state.profiles_with_slideshows,
        slideshows_found: state.slideshows_found,
        missing_audio: actionable.len(),
        already_have_audio: state.already_have_audio,
        unavailable_count: state.unavailable.len(),
        profiles: state.profiles.clone(),
        samples,
    }
}

fn empty_preview(dismissed: bool, scanned: bool) -> SlideshowAudioRepairPreview {
    SlideshowAudioRepairPreview {
        dismissed,
        scanned,
        scanned_at: None,
        profiles_scanned: 0,
        profiles_with_slideshows: 0,
        slideshows_found: 0,
        missing_audio: 0,
        already_have_audio: 0,
        unavailable_count: 0,
        profiles: Vec::new(),
        samples: Vec::new(),
    }
}

/// Read dismiss + persisted scan only (cheap).
pub fn load_slideshow_audio_repair_panel() -> Result<SlideshowAudioRepairPreview, String> {
    with_workspace(|connection, layout| {
        let dismissed = is_slideshow_audio_repair_dismissed(connection)?;
        let state = load_state(layout);
        if state.scanned_at.is_some() {
            // Re-check on-disk audio for missing items (cheap per post_id_audio) and drop recovered.
            let refreshed = refresh_state_against_disk(state);
            let _ = save_state(layout, &refreshed);
            return Ok(preview_from_state(dismissed, &refreshed));
        }
        Ok(empty_preview(dismissed, false))
    })
}

/// Full disk scan; persists result and keeps the inaccessible ledger.
pub fn preview_slideshow_audio_repair(
    app: &AppHandle,
) -> Result<SlideshowAudioRepairPreview, String> {
    preview_slideshow_audio_repair_full(Some(app))
}

pub fn dismiss_slideshow_audio_repair() -> Result<SlideshowAudioRepairPreview, String> {
    with_workspace(|connection, layout| {
        let now = now_timestamp();
        connection
            .execute(
                "INSERT INTO app_settings (key, value, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET
                   value = excluded.value,
                   updated_at = excluded.updated_at",
                params![SLIDESHOW_AUDIO_REPAIR_DISMISSED_KEY, "true", now],
            )
            .map_err(|error| error.to_string())?;
        let state = load_state(layout);
        Ok(preview_from_state(true, &state))
    })
}

/// Clear the inaccessible-post ledger (allows retry on the next download).
/// Posts stay in the persisted `missing` queue; only the skip ledger is wiped,
/// so the operator does not need a full disk rescan to retry them.
pub fn clear_slideshow_audio_unavailable() -> Result<SlideshowAudioRepairPreview, String> {
    with_workspace(|connection, layout| {
        let dismissed = is_slideshow_audio_repair_dismissed(connection)?;
        let mut state = load_state(layout);
        state.unavailable.clear();
        // Cheap disk pass: drop any that already gained audio since the mark.
        state = refresh_state_against_disk(state);
        save_state(layout, &state)?;
        Ok(preview_from_state(dismissed, &state))
    })
}

fn is_slideshow_audio_repair_dismissed(connection: &Connection) -> Result<bool, String> {
    Ok(
        load_app_setting_value(connection, SLIDESHOW_AUDIO_REPAIR_DISMISSED_KEY)?
            .map(|value| value.trim().eq_ignore_ascii_case("true"))
            .unwrap_or(false),
    )
}

fn emit_progress(app: Option<&AppHandle>, progress: SlideshowAudioRepairProgress) {
    if let Some(app) = app {
        let _ = app.emit(SLIDESHOW_AUDIO_REPAIR_PROGRESS_EVENT, &progress);
    }
}

/// Index `post_id`s that already have `<post_id>_audio.*` in a directory (1× read_dir).
fn audio_post_ids_in_dir(dir: &Path) -> HashSet<String> {
    let mut found = HashSet::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some((stem, ext)) = name.rsplit_once('.') else {
            continue;
        };
        if !GALLERY_AUDIO_EXTS.contains(&ext.to_ascii_lowercase().as_str()) {
            continue;
        }
        // Convention: `<post_id>_audio.<ext>`
        if let Some(post_id) = stem.strip_suffix("_audio") {
            if !post_id.is_empty() {
                found.insert(post_id.to_string());
            }
        }
    }
    found
}

/// Drop from the missing queue items that already have audio on disk.
/// O(profiles) with 1 `read_dir` per profile_root — NOT O(missing × read_dir).
fn refresh_state_against_disk(mut state: RepairPersistedState) -> RepairPersistedState {
    let mut audio_by_root: HashMap<String, HashSet<String>> = HashMap::new();
    let mut still_missing = Vec::new();
    let mut newly_have = 0usize;

    for item in state.missing.drain(..) {
        let have = audio_by_root
            .entry(item.profile_root.clone())
            .or_insert_with(|| audio_post_ids_in_dir(Path::new(&item.profile_root)));
        if have.contains(&item.post_id) {
            newly_have += 1;
            state
                .unavailable
                .remove(&unavailable_key(&item.source_id, &item.post_id));
        } else {
            still_missing.push(item);
        }
    }
    state.missing = still_missing;
    state.already_have_audio = state.already_have_audio.saturating_add(newly_have);
    state
}

struct AccountAuthBundle {
    cookie_path: PathBuf,
    user_agent: Option<String>,
    cookie_count: usize,
}

/// Optional filters for a repair run (UI uses defaults; CLI passes limits/handles).
#[derive(Clone, Debug, Default)]
pub struct SlideshowAudioRepairRunOptions {
    /// Process at most N actionable jobs (after filters).
    pub limit: Option<usize>,
    /// Only jobs whose handle matches (case-insensitive, optional leading `@`).
    pub handle_filter: Option<String>,
    /// Wipe the inaccessible ledger before building the job list.
    pub clear_unavailable: bool,
}

/// UI entry point — full queue, live progress events.
pub fn run_slideshow_audio_repair(app: &AppHandle) -> Result<SlideshowAudioRepairResult, String> {
    run_slideshow_audio_repair_with_options(Some(app), SlideshowAudioRepairRunOptions::default())
}

/// Headless / CLI entry point. `app = None` skips Tauri progress events (log still written).
pub fn run_slideshow_audio_repair_with_options(
    app: Option<&AppHandle>,
    options: SlideshowAudioRepairRunOptions,
) -> Result<SlideshowAudioRepairResult, String> {
    // Immediate feedback — refreshing state for 10k items must not look “stuck”.
    emit_progress(
        app,
        SlideshowAudioRepairProgress {
            phase: "downloading".to_string(),
            message: "Preparing download queue from saved scan…".to_string(),
            ..Default::default()
        },
    );

    let handle_filter = options.handle_filter.as_ref().map(|value| {
        value
            .trim()
            .trim_start_matches('@')
            .to_ascii_lowercase()
    });

    let (mut state, jobs, yt_dlp, auth_by_account, log_path, layout_for_log) =
        with_workspace(|connection, layout| {
            let mut state = load_state(layout);
            if state.scanned_at.is_none() {
                return Err(
                    "No saved scan. Run \"Scan TikTok slideshows\" before downloading.".to_string(),
                );
            }

            if options.clear_unavailable {
                state.unavailable.clear();
            }

            // 1 read_dir per profile_root (not 1 per post).
            state = refresh_state_against_disk(state);
            let mut jobs: Vec<MissingSlideshowAudio> = state
                .missing
                .iter()
                .filter(|item| {
                    !state
                        .unavailable
                        .contains_key(&unavailable_key(&item.source_id, &item.post_id))
                })
                .filter(|item| {
                    let Some(filter) = handle_filter.as_ref() else {
                        return true;
                    };
                    item.handle
                        .trim_start_matches('@')
                        .to_ascii_lowercase()
                        == *filter
                })
                .cloned()
                .collect();
            if let Some(limit) = options.limit {
                if jobs.len() > limit {
                    jobs.truncate(limit);
                }
            }

            let yt_dlp =
                connector_runtime::resolve_connector_executable(connection, layout, "yt-dlp")?;
            let mut auth_by_account: HashMap<String, AccountAuthBundle> = HashMap::new();
            let cache_root = repair_cache_dir(layout);
            fs::create_dir_all(&cache_root).map_err(|error| error.to_string())?;
            fs::create_dir_all(&layout.logs_dir).map_err(|error| error.to_string())?;
            let stamp = Utc::now().format("%Y%m%d-%H%M%S");
            let log_path = layout
                .logs_dir
                .join(format!("slideshow-audio-repair-{stamp}.log"));

            // Auth once per account (not per job) — O(accounts), not O(jobs).
            let account_ids: HashSet<String> = jobs
                .iter()
                .filter_map(|job| job.account_id.clone())
                .collect();
            for account_id in account_ids {
                let Ok(secret_ref) = load_account_session_secret_ref(connection, &account_id) else {
                    continue;
                };
                let Some(secret_ref) = secret_ref else {
                    continue;
                };
                let Ok(secret) = session_secret_store::load_secret(layout, &secret_ref) else {
                    continue;
                };
                let Ok(parsed) = parse_session_payload(&secret) else {
                    continue;
                };
                if parsed.cookies.is_empty() {
                    continue;
                }
                let cookie_path = cache_root.join(format!("cookies-{account_id}.txt"));
                if write_netscape_cookie_file(&cookie_path, &parsed.cookies).is_err() {
                    continue;
                }
                let settings =
                    load_provider_account_settings_map(connection, &account_id).unwrap_or_default();
                let user_agent = settings
                    .get("tiktok.auth.userAgent")
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .or_else(|| {
                        parsed
                            .metadata
                            .user_agent
                            .as_ref()
                            .map(|value| value.trim().to_string())
                            .filter(|value| !value.is_empty())
                    });
                auth_by_account.insert(
                    account_id,
                    AccountAuthBundle {
                        cookie_path,
                        user_agent,
                        cookie_count: parsed.cookies.len(),
                    },
                );
            }

            // Persist already-filtered missing (audio that arrived between scan and download).
            let _ = save_state(layout, &state);

            Ok((
                state,
                jobs,
                PathBuf::from(yt_dlp),
                auth_by_account,
                log_path,
                layout.clone(),
            ))
        })?;

    let attempted = jobs.len();
    let log_path_str = log_path.to_string_lossy().to_string();
    let accounts_with_auth = auth_by_account.len();
    let jobs_with_auth = jobs
        .iter()
        .filter(|job| {
            job.account_id
                .as_deref()
                .is_some_and(|id| auth_by_account.contains_key(id))
        })
        .count();

    append_repair_log(
        &log_path,
        &format!(
            "Slideshow audio repair started at {}\n\
             mode=inline one-shot (NOT source-sync queue)\n\
             policy=one_attempt_per_post; mark unavailable only with evidence (oEmbed gone or terminal yt-dlp class); otherwise requeue unmarked\n\
             yt-dlp={}\n\
             tracks_to_download={attempted}\n\
             already_unavailable={}\n\
             accounts_with_cookies={accounts_with_auth}\n\
             jobs_with_cookies={jobs_with_auth}/{attempted}\n\
             inter_post_delay_secs={}\n\
             yt_dlp_sleep_requests={YT_DLP_SLEEP_REQUESTS}\n\
             yt_dlp_retries={YT_DLP_RETRIES}\n\
             yt_dlp_extractor_retries={YT_DLP_EXTRACTOR_RETRIES}\n\
             yt_dlp_impersonate={YT_DLP_IMPERSONATE}\n\
             path_suspect_threshold={PATH_SUSPECT_THRESHOLD}\n\
             path_cooldown_cycles={PATH_COOLDOWN_CYCLES}\n\
             path_cooldown_secs={}\n\
             url_form=/video/ (yt-dlp does not support /photo/); attempt1=no-cookies, attempt2=cookies fallback\n\
             note=TikTok reports deleted posts AND real blocks as \"IP address is blocked\" (ambiguous_ip_block). Every ambiguous failure is resolved per post: oEmbed alive → requeue (never marked); oEmbed gone → profile embed refines (public=post_gone mark; private → cookie retry then requeue if still failing — do not abandon). Unreachable checks fall back to a live control post; only a control-confirmed outage or a persistent online-but-blocked streak (after {PATH_COOLDOWN_CYCLES} cooldowns) aborts — nothing is marked without evidence, so aborts roll back nothing.\n\
             ---\n",
            Utc::now().to_rfc3339(),
            yt_dlp.display(),
            state.unavailable.len(),
            INTER_POST_DELAY.as_secs(),
            PATH_COOLDOWN.as_secs(),
        ),
    );

    emit_progress(
        app,
        SlideshowAudioRepairProgress {
            phase: "downloading".to_string(),
            message: if attempted == 0 {
                "Nothing to download (queue empty or all marked unavailable).".to_string()
            } else {
                format!(
                    "Starting download of {attempted} track(s) · 1 attempt each · log: {log_path_str}"
                )
            },
            download_total: attempted,
            download_done: 0,
            log_path: Some(log_path_str.clone()),
            ..Default::default()
        },
    );

    let mut recovered = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;
    let mut marked_unavailable = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let mut recent_failures: Vec<String> = Vec::new();
    let mut last_error: Option<String> = None;
    // Per-post availability confirmation + download-path health (see the
    // constants above). Nothing is marked unavailable without evidence, so
    // aborts never need a ledger rollback.
    let http_client = build_availability_probe_client();
    let mut profile_status_by_handle: HashMap<String, ProfileEmbedStatus> = HashMap::new();
    let mut path_health = DownloadPathHealth::default();
    let mut last_recovered: Option<(String, String)> = None;
    let mut confirmed_gone = 0usize;
    let mut requeued_transient = 0usize;
    let mut recovered_via_cookie_retry = 0usize;
    let mut aborted_on_network_block = false;

    for (index, job) in jobs.iter().enumerate() {
        let handle_display = job.handle.trim_start_matches('@');
        emit_progress(
            app,
            SlideshowAudioRepairProgress {
                phase: "downloading".to_string(),
                message: format!(
                    "Downloading audio {}/{} · @{handle_display} · {}",
                    index + 1,
                    attempted,
                    job.post_id
                ),
                current_handle: Some(job.handle.clone()),
                current_source_id: Some(job.source_id.clone()),
                current_post_id: Some(job.post_id.clone()),
                download_total: attempted,
                download_done: index,
                recovered,
                failed,
                skipped,
                missing_audio: attempted,
                last_error: last_error.clone(),
                recent_failures: recent_failures.clone(),
                log_path: Some(log_path_str.clone()),
                ..Default::default()
            },
        );

        let profile_root = PathBuf::from(&job.profile_root);
        // Fast skip: `{post_id}_audio.*` file (without re-listing the whole dir).
        if has_audio_file_fast(&profile_root, &job.post_id) {
            skipped += 1;
            append_repair_log(
                &log_path,
                &format!(
                    "SKIP  @{handle_display}  {}  (audio already on disk)\n{}",
                    job.post_id,
                    format_post_urls_line(handle_display, &job.post_id),
                ),
            );
            continue;
        }

        if index > 0 {
            std::thread::sleep(INTER_POST_DELAY);
        }

        let auth = job
            .account_id
            .as_deref()
            .and_then(|id| auth_by_account.get(id));
        let has_cookies = auth.is_some();
        let cookie_count = auth.map(|bundle| bundle.cookie_count).unwrap_or(0);
        let ua_note = auth
            .and_then(|bundle| bundle.user_agent.as_deref())
            .map(|ua| format!("ua={}", &ua[..ua.len().min(48)]))
            .unwrap_or_else(|| "ua=default".to_string());

        match download_slideshow_audio_track_once(
            &yt_dlp,
            &job.handle,
            &job.post_id,
            &profile_root,
            auth.map(|bundle| bundle.cookie_path.as_path()),
            auth.and_then(|bundle| bundle.user_agent.as_deref()),
        ) {
            Ok(path) => {
                recovered += 1;
                // Any success restores the path-health budget and becomes the
                // preferred known-alive control target.
                path_health.on_recovered();
                last_recovered = Some((job.handle.clone(), job.post_id.clone()));
                state
                    .unavailable
                    .remove(&unavailable_key(&job.source_id, &job.post_id));
                append_repair_log(
                    &log_path,
                    &format!(
                        "OK    @{handle_display}  {}  cookies={has_cookies}({cookie_count})  {ua_note}  -> {}\n{}",
                        job.post_id,
                        path.display(),
                        format_post_urls_line(handle_display, &job.post_id),
                    ),
                );
            }
            Err(error) => {
                let class = classify_tiktok_audio_error(&error);
                let (photo_url, video_url) = tiktok_post_urls(handle_display, &job.post_id);

                // Decision tree (module docs): only mark when the post is
                // provably gone/private (or a true terminal yt-dlp class).
                // If the post is still online, leave it queued for repair —
                // including photo_no_av (soundtrack not extractable *yet*).
                let outcome = if needs_availability_confirmation(class) {
                    match http_client.as_ref() {
                        None => FailureOutcome::Requeue {
                            confirm_note:
                                "no_http_client → cannot confirm availability; requeued unmarked"
                                    .to_string(),
                            // Without confirmation, do not treat as path block.
                            path_suspect: false,
                        },
                        Some(client) => match probe_post_oembed(client, handle_display, &job.post_id)
                        {
                            // Post online + no extractable audio: still needs
                            // repair later (yt-dlp/TikTok may start exposing
                            // the track). Never mark unavailable — that would
                            // abandon slideshows that Single Videos can still
                            // show as photos.
                            PostOembedStatus::Alive if class == "photo_no_av_format" => {
                                FailureOutcome::Requeue {
                                    confirm_note:
                                        "oembed=alive → post online; soundtrack not extractable via yt-dlp right now — left queued for a later repair run (NOT marked unavailable)"
                                            .to_string(),
                                    // Content-side gap, not a blocked download path.
                                    path_suspect: false,
                                }
                            }
                            PostOembedStatus::Alive => FailureOutcome::Requeue {
                                confirm_note:
                                    "oembed=alive → post still online; failure is a blocked/limited download path, not missing content"
                                        .to_string(),
                                path_suspect: true,
                            },
                            PostOembedStatus::Gone => {
                                let profile = profile_status_by_handle
                                    .entry(handle_display.to_string())
                                    .or_insert_with(|| probe_profile_embed(client, handle_display))
                                    .clone();
                                let cookie_retry_makes_sense = matches!(
                                    profile,
                                    ProfileEmbedStatus::Private
                                        | ProfileEmbedStatus::Inconclusive(_)
                                );
                                match (cookie_retry_makes_sense, auth) {
                                    (true, Some(auth)) => match retry_download_with_cookies(
                                        &yt_dlp,
                                        &job.handle,
                                        &job.post_id,
                                        &profile_root,
                                        &auth.cookie_path,
                                        auth.user_agent.as_deref(),
                                    ) {
                                        Ok(path) => FailureOutcome::Recovered {
                                            path,
                                            confirm_note: format!(
                                                "oembed=gone profile={} cookie_retry=recovered (content requires the session)",
                                                profile.as_log_str()
                                            ),
                                        },
                                        // Private / inconclusive: post may still be
                                        // downloadable later (follow, session refresh).
                                        // Never abandon the repair queue.
                                        Err(retry_error) => FailureOutcome::Requeue {
                                            confirm_note: format!(
                                                "oembed=gone profile={} cookie_retry=failed ({retry_error}) — left queued for a later repair run (NOT marked unavailable)",
                                                profile.as_log_str()
                                            ),
                                            path_suspect: false,
                                        },
                                    },
                                    // No auth bundle, or profile private without cookies.
                                    _ => match profile {
                                        ProfileEmbedStatus::Gone => FailureOutcome::Mark {
                                            final_class: "account_gone".to_string(),
                                            confirm_note: format!(
                                                "oembed=gone profile=gone (yt-dlp class was {class})"
                                            ),
                                        },
                                        ProfileEmbedStatus::Private
                                        | ProfileEmbedStatus::Inconclusive(_) => {
                                            FailureOutcome::Requeue {
                                                confirm_note: format!(
                                                    "oembed=gone profile={} — left queued (private/inconclusive; retry when session can access)",
                                                    profile.as_log_str()
                                                ),
                                                path_suspect: false,
                                            }
                                        }
                                        ProfileEmbedStatus::Public => FailureOutcome::Mark {
                                            final_class: "post_gone".to_string(),
                                            confirm_note: format!(
                                                "oembed=gone profile=public (yt-dlp class was {class})"
                                            ),
                                        },
                                    },
                                }
                            }
                            PostOembedStatus::Unreachable(detail) => {
                                if control_post_alive(client, last_recovered.as_ref()) {
                                    FailureOutcome::Requeue {
                                        confirm_note: format!(
                                            "oembed=unreachable({detail}) control=alive → inconclusive for this post"
                                        ),
                                        path_suspect: false,
                                    }
                                } else {
                                    FailureOutcome::AbortNetwork {
                                        detail: format!(
                                            "oEmbed unreachable for the target AND the live control ({detail})"
                                        ),
                                    }
                                }
                            }
                        },
                    }
                } else {
                    // True terminal yt-dlp classes (post_gone / private_or_auth
                    // after the download already tried cookie fallback).
                    FailureOutcome::Mark {
                        final_class: class.to_string(),
                        confirm_note: "terminal yt-dlp class (no oEmbed check needed)".to_string(),
                    }
                };

                match outcome {
                    FailureOutcome::Recovered { path, confirm_note } => {
                        recovered += 1;
                        recovered_via_cookie_retry += 1;
                        path_health.on_recovered();
                        last_recovered = Some((job.handle.clone(), job.post_id.clone()));
                        state
                            .unavailable
                            .remove(&unavailable_key(&job.source_id, &job.post_id));
                        append_repair_log(
                            &log_path,
                            &format!(
                                "OK    @{handle_display}  {}  cookies=confirmed-private-retry  -> {}\n      confirm: {confirm_note}\n{}",
                                job.post_id,
                                path.display(),
                                format_post_urls_line(handle_display, &job.post_id),
                            ),
                        );
                    }
                    FailureOutcome::Mark {
                        final_class,
                        confirm_note,
                    } => {
                        failed += 1;
                        marked_unavailable += 1;
                        if is_ambiguous_availability_class(class) {
                            confirmed_gone += 1;
                        }
                        path_health.on_content_answer();
                        let line = format!(
                            "@{handle_display} / {} — [{final_class}] {error} | {photo_url}",
                            job.post_id
                        );
                        last_error = Some(line.clone());
                        recent_failures.insert(0, line.clone());
                        if recent_failures.len() > RECENT_FAILURE_UI_LIMIT {
                            recent_failures.truncate(RECENT_FAILURE_UI_LIMIT);
                        }
                        if failures.len() < FAILURE_LIMIT {
                            failures.push(line);
                        }
                        state.unavailable.insert(
                            unavailable_key(&job.source_id, &job.post_id),
                            UnavailablePost {
                                source_id: job.source_id.clone(),
                                handle: job.handle.clone(),
                                post_id: job.post_id.clone(),
                                class: final_class.clone(),
                                error: error.clone(),
                                url_photo: photo_url,
                                url_video: video_url,
                                failed_at: Utc::now().to_rfc3339(),
                            },
                        );
                        append_repair_log(
                            &log_path,
                            &format!(
                                "FAIL  @{handle_display}  {}  class={final_class}  cookies={has_cookies}({cookie_count})  {ua_note}  source={}  root={}\n{}\
      error: {error}\n      confirm: {confirm_note}\n      action=marked_unavailable (will not retry until ledger cleared)\n",
                                job.post_id,
                                job.source_id,
                                job.profile_root,
                                format_post_urls_line(handle_display, &job.post_id),
                            ),
                        );
                        if failed <= 25 || failed % 50 == 0 {
                            log_runtime_event(
                                &layout_for_log,
                                "repair.slideshow_audio",
                                "warn",
                                RuntimeLogAnchor {
                                    account_id: job.account_id.as_deref(),
                                    provider: Some("tiktok"),
                                    source_id: Some(&job.source_id),
                                    source_handle: Some(&job.handle),
                                },
                                format!(
                                    "Slideshow audio repair failed for @{} / {} [{final_class}] (marked unavailable)",
                                    handle_display, job.post_id
                                ),
                                Some(error),
                            );
                        }
                    }
                    FailureOutcome::Requeue {
                        confirm_note,
                        path_suspect,
                    } => {
                        requeued_transient += 1;
                        let line = format!(
                            "@{handle_display} / {} — [requeued] {error} | {photo_url}",
                            job.post_id
                        );
                        last_error = Some(line.clone());
                        recent_failures.insert(0, line.clone());
                        if recent_failures.len() > RECENT_FAILURE_UI_LIMIT {
                            recent_failures.truncate(RECENT_FAILURE_UI_LIMIT);
                        }
                        append_repair_log(
                            &log_path,
                            &format!(
                                "FAIL  @{handle_display}  {}  class={class}  cookies={has_cookies}({cookie_count})  {ua_note}\n{}\
      error: {error}\n      confirm: {confirm_note}\n      action=requeued (NOT marked; retried on a later run)\n",
                                job.post_id,
                                format_post_urls_line(handle_display, &job.post_id),
                            ),
                        );
                        // Only download-path blocks feed cooldowns/abort.
                        // photo_no_av / inconclusive checks stay queued without
                        // stalling the whole run.
                        if !path_suspect {
                            path_health.on_content_answer();
                        } else {
                            match path_health.on_suspect() {
                                PathHealthAction::Continue => {}
                                PathHealthAction::Cooldown => {
                                    let cooldown_secs = PATH_COOLDOWN.as_secs();
                                    append_repair_log(
                                        &log_path,
                                        &format!(
                                            "COOLDOWN {cooldown_secs}s — {PATH_SUSPECT_THRESHOLD} consecutive suspect failures (posts online but downloads failing); pausing before continuing (cycle {}/{PATH_COOLDOWN_CYCLES}).\n",
                                            path_health.cooldowns_used,
                                        ),
                                    );
                                    emit_progress(
                                        app,
                                        SlideshowAudioRepairProgress {
                                            phase: "downloading".to_string(),
                                            message: format!(
                                                "Cooling down {cooldown_secs}s — TikTok is refusing downloads for posts that are still online (cycle {}/{PATH_COOLDOWN_CYCLES})…",
                                                path_health.cooldowns_used,
                                            ),
                                            download_total: attempted,
                                            download_done: index,
                                            recovered,
                                            failed,
                                            skipped,
                                            missing_audio: attempted,
                                            last_error: last_error.clone(),
                                            recent_failures: recent_failures.clone(),
                                            log_path: Some(log_path_str.clone()),
                                            ..Default::default()
                                        },
                                    );
                                    std::thread::sleep(PATH_COOLDOWN);
                                }
                                PathHealthAction::Abort => {
                                    let abort_line = format!(
                                        "ABORTED: downloads keep failing for posts that are provably still online, even after {PATH_COOLDOWN_CYCLES} cooldown cycle(s) — the download path is blocked/limited. Nothing was marked without evidence; {requeued_transient} post(s) stay queued for a later run."
                                    );
                                    append_repair_log(
                                        &log_path,
                                        &format!(
                                            "ABORT confirmed_by=oembed_alive_streak — {abort_line}\n"
                                        ),
                                    );
                                    log_runtime_event(
                                        &layout_for_log,
                                        "repair.slideshow_audio",
                                        "warn",
                                        RuntimeLogAnchor::default(),
                                        "Slideshow audio repair aborted: download path blocked while posts remain online"
                                            .to_string(),
                                        None,
                                    );
                                    last_error = Some(abort_line.clone());
                                    failures.push(abort_line);
                                    aborted_on_network_block = true;
                                    break;
                                }
                            }
                        }
                    }
                    FailureOutcome::AbortNetwork { detail } => {
                        let abort_line = format!(
                            "ABORTED: TikTok unreachable — availability check failed for the target AND for a live control post ({detail}). Nothing was marked; posts stay queued for a later run."
                        );
                        append_repair_log(
                            &log_path,
                            &format!("ABORT confirmed_by=control_probe — {abort_line}\n"),
                        );
                        log_runtime_event(
                            &layout_for_log,
                            "repair.slideshow_audio",
                            "warn",
                            RuntimeLogAnchor::default(),
                            "Slideshow audio repair aborted: TikTok unreachable (live control probe also failed)"
                                .to_string(),
                            Some(detail),
                        );
                        last_error = Some(abort_line.clone());
                        failures.push(abort_line);
                        aborted_on_network_block = true;
                        break;
                    }
                }

                // Explicit rate-limit: back off a little before the next job.
                if class == "rate_limit" {
                    std::thread::sleep(RATE_LIMIT_EXTRA_DELAY);
                }
            }
        }

        // Persist progress every 50 jobs — a full run can take hours, and an
        // interrupted run must not lose ledger progress accumulated so far.
        if (index + 1) % 50 == 0 {
            let _ = save_state(&layout_for_log, &state);
        }
    }

    // Drop recovered audio from the queue; keep unavailable entries in
    // `missing` so "Clear unavailable" can re-enable them without a full rescan.
    // Jobs / preview already skip ledger keys when building the actionable set.
    state = refresh_state_against_disk(state);
    let remaining_missing = state
        .missing
        .iter()
        .filter(|item| {
            !state
                .unavailable
                .contains_key(&unavailable_key(&item.source_id, &item.post_id))
        })
        .count();
    let _ = with_workspace(|_connection, layout| save_state(layout, &state));

    if remaining_missing == 0 && state.unavailable.is_empty() {
        let _ = with_workspace(|connection, _layout| {
            let now = now_timestamp();
            connection
                .execute(
                    "INSERT INTO app_settings (key, value, updated_at)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(key) DO UPDATE SET
                       value = excluded.value,
                       updated_at = excluded.updated_at",
                    params![SLIDESHOW_AUDIO_REPAIR_DISMISSED_KEY, "true", now],
                )
                .map_err(|error| error.to_string())?;
            Ok(())
        });
    }

    append_repair_log(
        &log_path,
        &format!(
            "---\n\
             finished_at={}\n\
             attempted={attempted}\n\
             recovered={recovered}\n\
             failed={failed}\n\
             skipped={skipped}\n\
             marked_unavailable={marked_unavailable}\n\
             remaining_missing={remaining_missing}\n\
             unavailable_ledger_total={}\n\
             confirmed_gone={confirmed_gone}\n\
             recovered_via_cookie_retry={recovered_via_cookie_retry}\n\
             requeued_transient={requeued_transient}\n\
             cooldowns_used={}\n\
             aborted_on_network_block={aborted_on_network_block}\n",
            Utc::now().to_rfc3339(),
            state.unavailable.len(),
            path_health.cooldowns_used,
        ),
    );

    let abort_suffix = if aborted_on_network_block {
        format!(" · paused by a confirmed network block ({requeued_transient} re-queued)")
    } else if requeued_transient > 0 {
        format!(" · {requeued_transient} re-queued (still online or check inconclusive)")
    } else {
        String::new()
    };
    log_runtime_event(
        &layout_for_log,
        "repair.slideshow_audio",
        if failed > 0 || aborted_on_network_block { "warn" } else { "info" },
        RuntimeLogAnchor::default(),
        format!(
            "Slideshow audio repair finished: recovered {recovered}, failed {failed} (marked unavailable), skipped {skipped}, remaining {remaining_missing}{abort_suffix}"
        ),
        Some(format!("log={log_path_str}")),
    );

    emit_progress(
        app,
        SlideshowAudioRepairProgress {
            phase: "done".to_string(),
            message: format!(
                "Done · recovered {recovered}, failed {failed} (marked unavailable), skipped {skipped}, remaining {remaining_missing}{abort_suffix}. Log: {log_path_str}"
            ),
            download_total: attempted,
            download_done: attempted,
            recovered,
            failed,
            skipped,
            missing_audio: remaining_missing,
            last_error: last_error.clone(),
            recent_failures: recent_failures.clone(),
            log_path: Some(log_path_str.clone()),
            ..Default::default()
        },
    );

    Ok(SlideshowAudioRepairResult {
        attempted,
        recovered,
        failed,
        skipped,
        marked_unavailable,
        remaining_missing,
        failures,
        log_path: Some(log_path_str),
        aborted_on_network_block,
        requeued_transient,
    })
}

fn append_repair_log(path: &Path, chunk: &str) {
    use std::io::Write;
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(chunk.as_bytes());
    }
}

fn tiktok_post_urls(handle: &str, post_id: &str) -> (String, String) {
    let handle = handle.trim().trim_start_matches('@');
    let post_id = post_id.trim();
    (
        format!("https://www.tiktok.com/@{handle}/photo/{post_id}"),
        format!("https://www.tiktok.com/@{handle}/video/{post_id}"),
    )
}

fn format_post_urls_line(handle: &str, post_id: &str) -> String {
    let (photo, video) = tiktok_post_urls(handle, post_id);
    format!("      url_photo={photo}\n      url_video={video}\n")
}

struct ScanTotals {
    profiles_scanned: usize,
    profiles_with_slideshows: usize,
    slideshows_found: usize,
    already_have_audio: usize,
    missing: Vec<MissingSlideshowAudio>,
    profiles: Vec<SlideshowAudioRepairProfileSummary>,
}

fn collect_missing_slideshow_audio_detailed(
    connection: &Connection,
    layout: &StorageLayout,
    on_progress: Option<&ProgressFn>,
) -> Result<ScanTotals, String> {
    let mut statement = connection
        .prepare(
            "SELECT id, handle, account_id, sync_options_json
             FROM source_profiles
             WHERE deleted_at IS NULL
               AND lower(provider) = 'tiktok'
             ORDER BY handle COLLATE NOCASE",
        )
        .map_err(|error| error.to_string())?;
    let rows: Vec<(String, String, Option<String>, String)> = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    let profiles_total = rows.len();
    let mut totals = ScanTotals {
        profiles_scanned: 0,
        profiles_with_slideshows: 0,
        slideshows_found: 0,
        already_have_audio: 0,
        missing: Vec::new(),
        profiles: Vec::new(),
    };

    if let Some(on_progress) = on_progress {
        on_progress(SlideshowAudioRepairProgress {
            phase: "scanning".to_string(),
            message: format!("Scanning {profiles_total} TikTok profile(s) on disk…"),
            profiles_total,
            profiles_done: 0,
            ..Default::default()
        });
    }

    for (source_id, handle, account_id, sync_options_json) in rows {
        totals.profiles_scanned += 1;
        if let Some(on_progress) = on_progress {
            on_progress(SlideshowAudioRepairProgress {
                phase: "scanning".to_string(),
                message: format!(
                    "Scanning profile {}/{} · @{}",
                    totals.profiles_scanned,
                    profiles_total,
                    handle.trim_start_matches('@')
                ),
                profiles_total,
                profiles_done: totals.profiles_scanned.saturating_sub(1),
                current_handle: Some(handle.clone()),
                current_source_id: Some(source_id.clone()),
                slideshows_found: totals.slideshows_found,
                missing_audio: totals.missing.len(),
                already_have_audio: totals.already_have_audio,
                ..Default::default()
            });
        }

        let source_profile = SourceProfile {
            id: source_id.clone(),
            provider: "tiktok".to_string(),
            source_kind: "profile".to_string(),
            handle: handle.clone(),
            display_name: String::new(),
            account_id: account_id.clone(),
            group_id: None,
            labels: Vec::new(),
            ready_for_download: false,
            sync_options: deserialize_source_sync_options("tiktok", &sync_options_json),
            profile_image_path: None,
            profile_image_custom: false,
            remote_state: "exists".to_string(),
            is_subscription: false,
            last_synced_at: None,
            sync_problem_code: None,
            sync_problem_message: None,
            sync_problem_at: None,
            created_at: None,
            importer_id: None,
            imported_at: None,
        };
        let profile_root =
            resolved_source_media_output_root_with_connection(connection, layout, &source_profile)?;
        if !profile_root.is_dir() {
            continue;
        }

        let slideshows = scan_profile_slideshows(&profile_root)?;
        if slideshows.is_empty() {
            continue;
        }
        totals.profiles_with_slideshows += 1;
        let mut profile_missing = 0usize;
        let mut profile_have = 0usize;
        let profile_slideshows = slideshows.len();
        for (post_id, image_count) in slideshows {
            totals.slideshows_found += 1;
            let (rel, abs) = find_slideshow_audio(&profile_root, &[], Some(&post_id));
            if rel.is_some() || abs.is_some() {
                totals.already_have_audio += 1;
                profile_have += 1;
                continue;
            }
            profile_missing += 1;
            totals.missing.push(MissingSlideshowAudio {
                source_id: source_id.clone(),
                handle: handle.clone(),
                account_id: account_id.clone(),
                profile_root: profile_root.to_string_lossy().to_string(),
                post_id,
                image_count,
            });
        }
        if totals.profiles.len() < PROFILE_SUMMARY_LIMIT {
            totals.profiles.push(SlideshowAudioRepairProfileSummary {
                source_id: source_id.clone(),
                handle: handle.clone(),
                slideshows: profile_slideshows,
                missing_audio: profile_missing,
                already_have_audio: profile_have,
            });
        }
    }

    if let Some(on_progress) = on_progress {
        on_progress(SlideshowAudioRepairProgress {
            phase: "scanning".to_string(),
            message: format!(
                "Scan complete · {} slideshow(s), {} missing audio",
                totals.slideshows_found,
                totals.missing.len()
            ),
            profiles_total,
            profiles_done: profiles_total,
            slideshows_found: totals.slideshows_found,
            missing_audio: totals.missing.len(),
            already_have_audio: totals.already_have_audio,
            ..Default::default()
        });
    }

    Ok(totals)
}

pub fn preview_slideshow_audio_repair_full(
    app: Option<&AppHandle>,
) -> Result<SlideshowAudioRepairPreview, String> {
    with_workspace(|connection, layout| {
        let dismissed = is_slideshow_audio_repair_dismissed(connection)?;
        let previous = load_state(layout);
        let on_progress: Option<Box<ProgressFn>> = app.map(|app| {
            let app = app.clone();
            Box::new(move |progress| emit_progress(Some(&app), progress)) as Box<ProgressFn>
        });
        let totals = collect_missing_slideshow_audio_detailed(
            connection,
            layout,
            on_progress.as_deref(),
        )?;

        // Keep inaccessible ledger across scans; only clear if audio already exists.
        let unavailable = previous.unavailable;
        // Remove unavailable entries that now have audio on disk.
        let mut still_unavail = HashMap::new();
        for (key, entry) in unavailable {
            // Find profile root from missing of previous or skip check via disk if we can find path
            // We only drop when post is no longer "missing" in new scan AND has audio —
            // if still missing, keep. If not in new missing at all (has audio or gone), drop.
            let still_missing = totals.missing.iter().any(|m| {
                m.source_id == entry.source_id && m.post_id == entry.post_id
            });
            if still_missing {
                still_unavail.insert(key, entry);
            }
        }

        let state = RepairPersistedState {
            scanned_at: Some(Utc::now().to_rfc3339()),
            profiles_scanned: totals.profiles_scanned,
            profiles_with_slideshows: totals.profiles_with_slideshows,
            slideshows_found: totals.slideshows_found,
            already_have_audio: totals.already_have_audio,
            missing: totals.missing,
            profiles: totals.profiles,
            unavailable: still_unavail,
        };
        save_state(layout, &state)?;

        if let Some(app) = app {
            emit_progress(
                Some(app),
                SlideshowAudioRepairProgress {
                    phase: "done".to_string(),
                    message: format!(
                        "Scan saved · {} missing actionable · {} unavailable (skipped) · reopening panel reuses this scan",
                        state
                            .missing
                            .iter()
                            .filter(|m| !state
                                .unavailable
                                .contains_key(&unavailable_key(&m.source_id, &m.post_id)))
                            .count(),
                        state.unavailable.len()
                    ),
                    profiles_total: state.profiles_scanned,
                    profiles_done: state.profiles_scanned,
                    slideshows_found: state.slideshows_found,
                    missing_audio: state.missing.len(),
                    already_have_audio: state.already_have_audio,
                    ..Default::default()
                },
            );
        }

        Ok(preview_from_state(dismissed, &state))
    })
}

fn scan_profile_slideshows(profile_root: &Path) -> Result<Vec<(String, usize)>, String> {
    let mut grouped: HashMap<String, (bool, usize)> = HashMap::new();
    for path in collect_media_file_paths(profile_root)? {
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if is_profile_image_file(file_name) {
            continue;
        }
        let relative = path
            .strip_prefix(profile_root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let top = relative
            .split('/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if top == "settings" || top == "cover" {
            continue;
        }
        let Some(derived) = derive_post_metadata("tiktok", file_name, None) else {
            continue;
        };
        if derived.media_type != "image" {
            continue;
        }
        let Some(post_id) = derived.post_id else {
            continue;
        };
        let entry = grouped.entry(post_id).or_insert((false, 0));
        entry.1 += 1;
        if derived.index.is_some() {
            entry.0 = true;
        }
    }
    let mut out: Vec<(String, usize)> = grouped
        .into_iter()
        .filter(|(_, (has_index, count))| *has_index || *count > 1)
        .map(|(post_id, (_, count))| (post_id, count))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Checagem O(exts) por path fixo — evita read_dir no hot path do download.
fn has_audio_file_fast(profile_root: &Path, post_id: &str) -> bool {
    for ext in GALLERY_AUDIO_EXTS {
        let path = profile_root.join(format!("{post_id}_audio.{ext}"));
        if atomic_file::is_nonempty_file(&path) {
            return true;
        }
    }
    false
}

/// Failure taxonomy. The yt-dlp error string alone cannot distinguish a post
/// deleted by the creator from a real IP block — TikTok answers both with
/// "Your IP address is blocked from accessing this post" — so that string maps
/// to the explicitly *ambiguous* class and is resolved per post via TikTok
/// oEmbed (+ profile embed) before anything is marked unavailable.
fn classify_tiktok_audio_error(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    if lower.contains("ip address is blocked") {
        "ambiguous_ip_block"
    } else if lower.contains("rate-limit") || lower.contains("rate limit") || lower.contains("429")
    {
        "rate_limit"
    } else if lower.contains("permission to view")
        || lower.contains("log into an account")
        || lower.contains("private")
    {
        "private_or_auth"
    } else if lower.contains("no video formats") || lower.contains("no audio formats") {
        "photo_no_av_format"
    } else if lower.contains("unsupported url")
        || lower.contains("unable to extract")
        || lower.contains("unexpected response")
    {
        "extractor_error"
    } else if lower.contains("not found")
        || lower.contains("status code 404")
        || lower.contains("does not exist")
    {
        "post_gone"
    } else {
        "unknown"
    }
}

/// Classes where the yt-dlp error alone cannot tell "post is gone" from a
/// block/limit/extractor hiccup — these get the per-post oEmbed confirmation
/// instead of being marked unavailable on the string alone.
fn is_ambiguous_availability_class(class: &str) -> bool {
    matches!(
        class,
        "ambiguous_ip_block" | "rate_limit" | "extractor_error" | "unknown"
    )
}

/// Failures that must not write the ledger without an oEmbed (or control)
/// check. Includes `photo_no_av_format`: "No video formats found" is also how
/// private / deleted posts sometimes present; on live posts it means the
/// soundtrack is not extractable *yet* — requeue for later repair, never
/// abandon the slideshow without proof the post is gone.
fn needs_availability_confirmation(class: &str) -> bool {
    is_ambiguous_availability_class(class) || class == "photo_no_av_format"
}

/// Result of the per-post oEmbed availability check.
/// Validated 2026-07-19 from this codebase: a live post answers HTTP 200 with
/// oEmbed JSON; posts reported by yt-dlp as "IP blocked" answered HTTP 400
/// `{"code":400}` while the live control answered 200 from the same IP.
#[derive(Clone, Debug, PartialEq)]
enum PostOembedStatus {
    Alive,
    Gone,
    Unreachable(String),
}

fn classify_oembed_response(http_status: u16, body: &str) -> PostOembedStatus {
    match http_status {
        // WAF/challenge pages can answer 200 with HTML; require JSON markers.
        200 => {
            if body.contains("\"html\"") || body.contains("\"title\"") {
                PostOembedStatus::Alive
            } else {
                PostOembedStatus::Unreachable("HTTP 200 without oEmbed JSON".to_string())
            }
        }
        400 => PostOembedStatus::Gone,
        other => PostOembedStatus::Unreachable(format!("HTTP {other}")),
    }
}

/// Profile embed page status — same markers the TikTok connector's
/// `classify_embed_profile_status` relies on (`10222` private, `10221` gone).
#[derive(Clone, Debug, PartialEq)]
enum ProfileEmbedStatus {
    Public,
    Private,
    Gone,
    Inconclusive(String),
}

impl ProfileEmbedStatus {
    fn as_log_str(&self) -> &str {
        match self {
            ProfileEmbedStatus::Public => "public",
            ProfileEmbedStatus::Private => "private",
            ProfileEmbedStatus::Gone => "gone",
            ProfileEmbedStatus::Inconclusive(_) => "inconclusive",
        }
    }
}

fn classify_profile_embed_response(body: &str) -> ProfileEmbedStatus {
    if body.contains("\"errorCode\":10222") || body.contains("\"privateAccount\":true") {
        ProfileEmbedStatus::Private
    } else if body.contains("\"errorCode\":10221") {
        ProfileEmbedStatus::Gone
    } else if body.contains("\"privateAccount\":false") {
        ProfileEmbedStatus::Public
    } else {
        ProfileEmbedStatus::Inconclusive("no profile markers in embed body".to_string())
    }
}

fn build_availability_probe_client() -> Option<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(AVAILABILITY_PROBE_TIMEOUT)
        .user_agent(AVAILABILITY_PROBE_USER_AGENT)
        .build()
        .ok()
}

fn probe_oembed_for_url(client: &reqwest::blocking::Client, post_url: &str) -> PostOembedStatus {
    match client
        .get("https://www.tiktok.com/oembed")
        .query(&[("url", post_url)])
        .send()
    {
        Ok(response) => {
            let status = response.status().as_u16();
            let body = response.text().unwrap_or_default();
            classify_oembed_response(status, &body)
        }
        Err(error) => PostOembedStatus::Unreachable(error.to_string()),
    }
}

fn probe_post_oembed(
    client: &reqwest::blocking::Client,
    handle: &str,
    post_id: &str,
) -> PostOembedStatus {
    let post_url = format!(
        "https://www.tiktok.com/@{}/video/{post_id}",
        handle.trim_start_matches('@')
    );
    probe_oembed_for_url(client, &post_url)
}

fn probe_profile_embed(client: &reqwest::blocking::Client, handle: &str) -> ProfileEmbedStatus {
    let url = format!(
        "https://www.tiktok.com/embed/@{}",
        handle.trim_start_matches('@')
    );
    match client.get(&url).send() {
        Ok(response) => {
            let body = response.text().unwrap_or_default();
            classify_profile_embed_response(&body)
        }
        Err(error) => ProfileEmbedStatus::Inconclusive(error.to_string()),
    }
}

/// Proves the oEmbed endpoint itself is reachable: prefers the last post
/// recovered in this run (known-alive moments ago; it may itself have just
/// been deleted, so a non-alive answer falls through), else the static
/// control post.
fn control_post_alive(
    client: &reqwest::blocking::Client,
    last_recovered: Option<&(String, String)>,
) -> bool {
    if let Some((handle, post_id)) = last_recovered {
        if probe_post_oembed(client, handle, post_id) == PostOembedStatus::Alive {
            return true;
        }
    }
    probe_oembed_for_url(client, CONTROL_POST_URL) == PostOembedStatus::Alive
}

enum PathHealthAction {
    Continue,
    Cooldown,
    Abort,
}

/// Pure health tracker for the download path. Suspect failures = downloads
/// failing for posts that are provably still online (or availability checks
/// unreachable while the control is alive). Confirmed content answers reset
/// the streak; a recovered download also restores the cooldown budget.
#[derive(Default)]
struct DownloadPathHealth {
    consecutive_suspect: usize,
    cooldowns_used: usize,
}

impl DownloadPathHealth {
    fn on_recovered(&mut self) {
        self.consecutive_suspect = 0;
        self.cooldowns_used = 0;
    }

    fn on_content_answer(&mut self) {
        self.consecutive_suspect = 0;
    }

    fn on_suspect(&mut self) -> PathHealthAction {
        self.consecutive_suspect += 1;
        if self.consecutive_suspect < PATH_SUSPECT_THRESHOLD {
            return PathHealthAction::Continue;
        }
        self.consecutive_suspect = 0;
        if self.cooldowns_used < PATH_COOLDOWN_CYCLES {
            self.cooldowns_used += 1;
            PathHealthAction::Cooldown
        } else {
            PathHealthAction::Abort
        }
    }
}

/// Resolution of one failed download after the availability decision tree.
enum FailureOutcome {
    /// Evidence says the post is not retrievable: write it to the ledger.
    Mark {
        final_class: String,
        confirm_note: String,
    },
    /// Leave the post in the missing queue (never ledger-mark). Used when the
    /// post is still online or the check was inconclusive — repair is not done.
    /// `path_suspect` = true only when downloads fail for posts that are
    /// provably online (blocked/limited path); false for content gaps such as
    /// missing soundtrack formats that should not trigger cooldowns/abort.
    Requeue {
        confirm_note: String,
        path_suspect: bool,
    },
    /// The account is private and the session's cookies got the audio.
    Recovered {
        path: PathBuf,
        confirm_note: String,
    },
    /// Target AND live control unreachable: TikTok is down/blocked for us.
    AbortNetwork { detail: String },
}

/// Uma tentativa, espelhando single-video photo audio + flags anti-bot do sync.
/// URL photo (carrossel) com `-f ba`; sem multi-retry que multiplica rate-limit.
fn download_slideshow_audio_track_once(
    yt_dlp: &Path,
    handle: &str,
    post_id: &str,
    dest_dir: &Path,
    cookie_file: Option<&Path>,
    user_agent: Option<&str>,
) -> Result<PathBuf, String> {
    fs::create_dir_all(dest_dir).map_err(|error| error.to_string())?;
    let handle = handle.trim_start_matches('@');
    // yt-dlp only supports the /video/ URL form for TikTok posts (photo-mode
    // included); /photo/ URLs fail with "Unsupported URL".
    let url = format!("https://www.tiktok.com/@{handle}/video/{post_id}");
    let output_template = format!(
        "{}/{}_audio.%(ext)s",
        dest_dir.to_string_lossy().replace('\\', "/"),
        post_id
    );

    let saved_audio = |dest_dir: &Path| -> Option<PathBuf> {
        find_slideshow_audio(dest_dir, &[], Some(post_id))
            .1
            .map(PathBuf::from)
            .filter(|path| atomic_file::is_nonempty_file(path))
    };

    // Attempt 1: no cookies, mirroring the working Single Videos path — the
    // saved session cookies break yt-dlp's "universal data" extraction on
    // many public posts.
    let first = run_yt_dlp_audio_once(yt_dlp, &url, &output_template, None, None, Some("ba"));
    if let Some(path) = saved_audio(dest_dir) {
        return Ok(path);
    }
    let first_error =
        first.err().unwrap_or_else(|| format!("yt-dlp finished without writing audio for {url}"));
    let first_class = classify_tiktok_audio_error(&first_error);

    // Attempt 2: cookie + UA fallback. Private posts often surface as plain
    // "no video formats found" (not private_or_auth), so cookies are worth
    // one try for format/auth/extractor classes — not for IP-block strings
    // (those are usually deleted posts and just double request volume).
    let should_retry_with_cookies = cookie_file.is_some()
        && matches!(
            first_class,
            "private_or_auth" | "photo_no_av_format" | "extractor_error" | "unknown"
        );
    let after_cookies = if should_retry_with_cookies {
        let second = run_yt_dlp_audio_once(
            yt_dlp,
            &url,
            &output_template,
            cookie_file,
            user_agent,
            Some("ba"),
        );
        if let Some(path) = saved_audio(dest_dir) {
            return Ok(path);
        }
        second
            .err()
            .unwrap_or_else(|| first_error.clone())
    } else {
        first_error.clone()
    };
    let after_cookies_class = classify_tiktok_audio_error(&after_cookies);

    // Attempt 3: no format selector. Some photo posts expose only a default
    // audio stream that `-f ba` rejects even though a plain download works.
    if matches!(after_cookies_class, "photo_no_av_format" | "extractor_error") {
        let third = run_yt_dlp_audio_once(
            yt_dlp,
            &url,
            &output_template,
            // Prefer the same cookie posture that got us here.
            if should_retry_with_cookies {
                cookie_file
            } else {
                None
            },
            if should_retry_with_cookies {
                user_agent
            } else {
                None
            },
            None,
        );
        if let Some(path) = saved_audio(dest_dir) {
            return Ok(path);
        }
        return Err(third.err().unwrap_or(after_cookies));
    }

    Err(after_cookies)
}

/// One authenticated attempt with the account cookies/UA — used when the
/// profile embed says the account is private (TikTok masks those posts behind
/// the same misleading "IP blocked" error, so the classifier-gated cookie
/// fallback inside `download_slideshow_audio_track_once` never fires).
fn retry_download_with_cookies(
    yt_dlp: &Path,
    handle: &str,
    post_id: &str,
    dest_dir: &Path,
    cookie_file: &Path,
    user_agent: Option<&str>,
) -> Result<PathBuf, String> {
    let handle = handle.trim_start_matches('@');
    let url = format!("https://www.tiktok.com/@{handle}/video/{post_id}");
    let output_template = format!(
        "{}/{}_audio.%(ext)s",
        dest_dir.to_string_lossy().replace('\\', "/"),
        post_id
    );
    let attempt = run_yt_dlp_audio_once(
        yt_dlp,
        &url,
        &output_template,
        Some(cookie_file),
        user_agent,
        Some("ba"),
    );
    if let Some(path) = find_slideshow_audio(dest_dir, &[], Some(post_id))
        .1
        .map(PathBuf::from)
        .filter(|path| atomic_file::is_nonempty_file(path))
    {
        return Ok(path);
    }
    Err(attempt
        .err()
        .unwrap_or_else(|| format!("yt-dlp finished without writing audio for {url}")))
}

fn run_yt_dlp_audio_once(
    yt_dlp: &Path,
    url: &str,
    output_template: &str,
    cookie_file: Option<&Path>,
    user_agent: Option<&str>,
    format: Option<&str>,
) -> Result<(), String> {
    let mut command = Command::new(yt_dlp);
    configure_background_command(&mut command);
    // Flags = tiktok_connector::download_batch + single photo audio (-f ba).
    command
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .arg("--ignore-errors")
        .arg("--no-warnings")
        .arg("--no-playlist")
        .arg("--no-simulate")
        .arg("--no-mtime")
        .arg("--socket-timeout")
        .arg("30")
        .arg("--extractor-retries")
        .arg(YT_DLP_EXTRACTOR_RETRIES)
        .arg("--retries")
        .arg(YT_DLP_RETRIES)
        .arg("--sleep-requests")
        .arg(YT_DLP_SLEEP_REQUESTS)
        .arg("--impersonate")
        .arg(YT_DLP_IMPERSONATE)
        .arg("--no-cookies-from-browser")
        .arg("-o")
        .arg(output_template);
    if let Some(format) = format {
        command.arg("-f").arg(format);
    }
    if let Some(cookie_file) = cookie_file {
        command.arg("--cookies").arg(cookie_file);
    }
    if let Some(user_agent) = user_agent.map(str::trim).filter(|value| !value.is_empty()) {
        command.arg("--user-agent").arg(user_agent);
    }
    command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = command
        .output()
        .map_err(|error| format!("Failed to run yt-dlp: {error}"))?;
    yt_dlp_outcome_from_output(&output)
}

/// Success/error extraction shared by the download and probe invocations:
/// ERROR lines beat exit code, and the last few lines become the detail.
fn yt_dlp_outcome_from_output(output: &std::process::Output) -> Result<(), String> {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut detail_lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if detail_lines.is_empty() {
        detail_lines = stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
    }
    if output.status.success() && detail_lines.iter().all(|line| !line.starts_with("ERROR:")) {
        return Ok(());
    }
    if detail_lines.iter().any(|line| line.starts_with("ERROR:")) {
        let detail = detail_lines
            .iter()
            .filter(|line| line.starts_with("ERROR:") || line.contains("ERROR"))
            .rev()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join(" | ");
        return Err(if detail.is_empty() {
            detail_lines.iter().rev().take(3).cloned().collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ")
        } else {
            detail
        });
    }
    if output.status.success() {
        return Ok(());
    }
    Err(if detail_lines.is_empty() {
        format!("yt-dlp failed (exit={})", output.status)
    } else {
        detail_lines
            .iter()
            .rev()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join(" | ")
    })
}

pub fn open_app_log_file(path: String) -> Result<(), String> {
    with_workspace(|_connection, layout| {
        let path = PathBuf::from(path.trim());
        if path.as_os_str().is_empty() {
            return Err("Log path is empty.".to_string());
        }
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("Log file not found: {error}"))?;
        let logs_root = layout
            .logs_dir
            .canonicalize()
            .unwrap_or_else(|_| layout.logs_dir.clone());
        if !canonical.starts_with(&logs_root) {
            return Err(format!(
                "Refusing to open path outside app logs dir ({}).",
                layout.logs_dir.display()
            ));
        }
        open_path_with_os_shell(&canonical)
    })
}

pub fn reveal_app_log_file(path: String) -> Result<(), String> {
    with_workspace(|_connection, layout| {
        let path = PathBuf::from(path.trim());
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("Log file not found: {error}"))?;
        let logs_root = layout
            .logs_dir
            .canonicalize()
            .unwrap_or_else(|_| layout.logs_dir.clone());
        if !canonical.starts_with(&logs_root) {
            return Err(format!(
                "Refusing to reveal path outside app logs dir ({}).",
                layout.logs_dir.display()
            ));
        }
        reveal_path_with_os_shell(&canonical)
    })
}

fn open_path_with_os_shell(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        Command::new("cmd")
            .args(["/C", "start", "", &path.to_string_lossy()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("Failed to open log: {error}"))?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|error| format!("Failed to open log: {error}"))?;
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|error| format!("Failed to open log: {error}"))?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err("Opening logs is not supported on this platform.".to_string())
}

fn reveal_path_with_os_shell(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn()
            .map_err(|error| format!("Failed to reveal log: {error}"))?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .args(["-R"])
            .arg(path)
            .spawn()
            .map_err(|error| format!("Failed to reveal log: {error}"))?;
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(parent) = path.parent() {
            Command::new("xdg-open")
                .arg(parent)
                .spawn()
                .map_err(|error| format!("Failed to reveal log: {error}"))?;
            return Ok(());
        }
    }
    #[allow(unreachable_code)]
    Err("Revealing logs is not supported on this platform.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_profile_slideshows_groups_index_photos() {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path();
        fs::write(
            root.join("h_1700000000_7658099397019831573_index_0_2.jpeg"),
            b"one",
        )
        .expect("write");
        fs::write(
            root.join("h_1700000000_7658099397019831573_index_1_2.jpeg"),
            b"two",
        )
        .expect("write");
        fs::write(root.join("h_1700000000_9999999999999999999.mp4"), b"vid").expect("video");

        let slideshows = scan_profile_slideshows(root).expect("scan");
        assert_eq!(slideshows.len(), 1);
        assert_eq!(slideshows[0].0, "7658099397019831573");
        assert_eq!(slideshows[0].1, 2);
    }

    #[test]
    fn classify_ip_block_as_ambiguous() {
        // Real string from slideshow-audio-repair-20260719-094813.log — this is
        // what TikTok returns for posts deleted by the creator too, so it must
        // NOT be trusted as a network-level fact.
        assert_eq!(
            classify_tiktok_audio_error(
                "ERROR: [TikTok] 7588998641923230997: Your IP address is blocked from accessing this post"
            ),
            "ambiguous_ip_block"
        );
    }

    #[test]
    fn classify_permission_error_as_private_or_auth() {
        assert_eq!(
            classify_tiktok_audio_error(
                "ERROR: You do not have permission to view this post. Log into an account that has access."
            ),
            "private_or_auth"
        );
    }

    #[test]
    fn classify_not_found_as_post_gone() {
        assert_eq!(
            classify_tiktok_audio_error("ERROR: [TikTok] 123: HTTP Error 404: Not Found"),
            "post_gone"
        );
    }

    #[test]
    fn classify_429_as_rate_limit() {
        assert_eq!(
            classify_tiktok_audio_error("ERROR: HTTP Error 429: Too Many Requests"),
            "rate_limit"
        );
    }

    #[test]
    fn classify_photo_no_av_as_content_terminal() {
        assert_eq!(
            classify_tiktok_audio_error("ERROR: [TikTok] 1: No video formats found"),
            "photo_no_av_format"
        );
    }

    #[test]
    fn ambiguous_classes_include_ip_block_rate_limit_extractor_unknown() {
        assert!(is_ambiguous_availability_class("ambiguous_ip_block"));
        assert!(is_ambiguous_availability_class("rate_limit"));
        assert!(is_ambiguous_availability_class("extractor_error"));
        assert!(is_ambiguous_availability_class("unknown"));
        assert!(!is_ambiguous_availability_class("post_gone"));
        assert!(!is_ambiguous_availability_class("private_or_auth"));
        assert!(!is_ambiguous_availability_class("photo_no_av_format"));
        assert!(!is_ambiguous_availability_class("account_private"));
    }

    #[test]
    fn photo_no_av_requires_oembed_confirmation() {
        // Empirically: @2julinda/7223126054590762245 is oEmbed-alive with empty
        // music.playUrl; yt-dlp says "No video formats found". Without
        // confirmation we would either mis-label private posts or permanently
        // abandon slideshows that still need repair when audio becomes
        // extractable again.
        assert!(needs_availability_confirmation("photo_no_av_format"));
        assert!(needs_availability_confirmation("ambiguous_ip_block"));
        assert!(!needs_availability_confirmation("post_gone"));
        assert!(!needs_availability_confirmation("private_or_auth"));
    }

    #[test]
    fn classify_no_video_formats_as_photo_no_av() {
        assert_eq!(
            classify_tiktok_audio_error(
                "ERROR: [TikTok] 7223126054590762245: No video formats found!; please report this issue on  https://github.com/yt-dlp/yt-dlp/issues?q="
            ),
            "photo_no_av_format"
        );
    }

    // --- oEmbed response classification (validated 2026-07-19) ---

    #[test]
    fn oembed_200_with_json_markers_is_alive() {
        assert_eq!(
            classify_oembed_response(200, r#"{"title":"x","author_name":"y","html":"<blockquote>"}"#),
            PostOembedStatus::Alive
        );
    }

    #[test]
    fn oembed_200_without_json_markers_is_unreachable() {
        // WAF/challenge pages can answer 200 with HTML; must not count as alive.
        let status = classify_oembed_response(200, "<html><body>challenge</body></html>");
        assert!(matches!(status, PostOembedStatus::Unreachable(_)));
    }

    #[test]
    fn oembed_400_is_gone() {
        // Real body shape from the validation run for deleted/private-masked posts.
        assert_eq!(
            classify_oembed_response(400, r#"{"message":"Something went wrong","code":400}"#),
            PostOembedStatus::Gone
        );
    }

    #[test]
    fn oembed_non_200_400_is_unreachable() {
        for code in [403u16, 429, 500, 503] {
            let status = classify_oembed_response(code, "nope");
            assert!(
                matches!(status, PostOembedStatus::Unreachable(_)),
                "HTTP {code} should be Unreachable"
            );
        }
    }

    // --- Profile embed classification (same markers as TikTok connector) ---

    #[test]
    fn profile_embed_10222_or_private_flag_is_private() {
        assert_eq!(
            classify_profile_embed_response(r#"{"errorCode":10222,"msg":"private"}"#),
            ProfileEmbedStatus::Private
        );
        assert_eq!(
            classify_profile_embed_response(r#"{"privateAccount":true}"#),
            ProfileEmbedStatus::Private
        );
    }

    #[test]
    fn profile_embed_10221_is_gone() {
        assert_eq!(
            classify_profile_embed_response(r#"{"errorCode":10221}"#),
            ProfileEmbedStatus::Gone
        );
    }

    #[test]
    fn profile_embed_public_flag_is_public() {
        assert_eq!(
            classify_profile_embed_response(r#"{"privateAccount":false,"uniqueId":"x"}"#),
            ProfileEmbedStatus::Public
        );
    }

    #[test]
    fn profile_embed_without_markers_is_inconclusive() {
        let status = classify_profile_embed_response("<html></html>");
        assert!(matches!(status, ProfileEmbedStatus::Inconclusive(_)));
    }

    // --- Download-path health (suspect streak → cooldown → abort) ---

    #[test]
    fn path_health_continues_before_suspect_threshold() {
        let mut health = DownloadPathHealth::default();
        for _ in 0..(PATH_SUSPECT_THRESHOLD - 1) {
            assert!(matches!(health.on_suspect(), PathHealthAction::Continue));
        }
        assert_eq!(health.consecutive_suspect, PATH_SUSPECT_THRESHOLD - 1);
        assert_eq!(health.cooldowns_used, 0);
    }

    #[test]
    fn path_health_triggers_cooldown_at_threshold_then_abort_after_budget() {
        let mut health = DownloadPathHealth::default();

        // First cooldown cycle.
        for _ in 0..(PATH_SUSPECT_THRESHOLD - 1) {
            assert!(matches!(health.on_suspect(), PathHealthAction::Continue));
        }
        assert!(matches!(health.on_suspect(), PathHealthAction::Cooldown));
        assert_eq!(health.cooldowns_used, 1);
        assert_eq!(health.consecutive_suspect, 0);

        // Second cooldown cycle exhausts the budget.
        for _ in 0..(PATH_SUSPECT_THRESHOLD - 1) {
            assert!(matches!(health.on_suspect(), PathHealthAction::Continue));
        }
        assert!(matches!(health.on_suspect(), PathHealthAction::Cooldown));
        assert_eq!(health.cooldowns_used, 2);

        // Next threshold hit aborts (no cooldowns left).
        for _ in 0..(PATH_SUSPECT_THRESHOLD - 1) {
            assert!(matches!(health.on_suspect(), PathHealthAction::Continue));
        }
        assert!(matches!(health.on_suspect(), PathHealthAction::Abort));
    }

    #[test]
    fn path_health_recovered_resets_suspect_and_cooldown_budget() {
        let mut health = DownloadPathHealth::default();
        for _ in 0..PATH_SUSPECT_THRESHOLD {
            let _ = health.on_suspect();
        }
        assert_eq!(health.cooldowns_used, 1);

        health.on_recovered();
        assert_eq!(health.consecutive_suspect, 0);
        assert_eq!(health.cooldowns_used, 0);
    }

    #[test]
    fn path_health_content_answer_resets_suspect_but_keeps_cooldown_budget() {
        let mut health = DownloadPathHealth::default();
        for _ in 0..PATH_SUSPECT_THRESHOLD {
            let _ = health.on_suspect();
        }
        assert_eq!(health.cooldowns_used, 1);

        // A few more suspects, then a confirmed content answer (post_gone).
        let _ = health.on_suspect();
        let _ = health.on_suspect();
        health.on_content_answer();
        assert_eq!(health.consecutive_suspect, 0);
        assert_eq!(health.cooldowns_used, 1); // budget not restored
    }
}
