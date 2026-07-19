//! Internal YouTube connector.
//!
//! Mirrors the TikTok connector template, but YouTube is considerably simpler:
//! there is no photo/slideshow backend, no gallery-dl fallback, and no TLS
//! impersonation is required. We use **yt-dlp** end to end:
//! 1. `--flat-playlist --print` enumerates the channel's uploads (fast, light);
//! 2. we filter the enumerated ids against the ledgers;
//! 3. new videos are downloaded in batches, with the naming/catalog kept under
//!    NinjaCrawler control (date prefix + ledger), like the other providers.
//!
//! The `videos` and (optionally) `shorts` channel tabs are enumerated as
//! separate sections; `%(title)s`, `%(duration)s` and `%(view_count)s` are
//! captured on download and persisted to the media/post ledgers.

use chrono::{Local, TimeZone};
use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use std::time::Instant;

use crate::infrastructure::{atomic_file, connector_debug};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const YT_DLP_LIST_TIMEOUT_SECS: u64 = 600;
const YT_DLP_DOWNLOAD_TIMEOUT_SECS: u64 = 3600;
const DOWNLOAD_BATCH_SIZE: usize = 20;
const VIDEO_EXTENSIONS: [&str; 5] = ["mp4", "webm", "mkv", "mov", "m4v"];
const IMAGE_EXTENSIONS: [&str; 6] = ["jpg", "jpeg", "png", "webp", "heic", "gif"];

#[derive(Clone, Copy, Default)]
pub struct YouTubeSectionSelection {
    pub videos: bool,
    pub shorts: bool,
}

#[derive(Clone)]
pub struct YouTubeConnectorRequest {
    pub handle: String,
    pub yt_dlp_executable: PathBuf,
    /// Netscape cookie file written by the caller. Passed to yt-dlp only when it
    /// exists and is non-empty (YouTube usually works without authentication).
    pub cookie_file: PathBuf,
    pub user_agent: Option<String>,
    pub profile_root: PathBuf,
    /// Working directory for temporary downloads.
    pub cache_root: PathBuf,
    pub sections: YouTubeSectionSelection,
    pub download_videos: bool,
    /// Videos go to a `Video` subfolder (SCrawler SeparateVideoFolder).
    pub separate_video_folder: bool,
    /// Adjusts the file mtime to the upload date (yt-dlp `--mtime`).
    pub use_parsed_video_date: bool,
    /// Seconds between download batches; `-1` disables.
    pub sleep_timer_secs: i64,
    pub abort_on_limit: bool,
    pub collect_media_stats: bool,
    pub ledger_post_keys: HashSet<String>,
    pub ledger_media_keys: HashSet<String>,
    pub existing_relative_paths: HashSet<String>,
    /// Stable channel id (`userIdHint`), when already known.
    pub user_id_hint: Option<String>,
}

#[derive(Clone)]
pub struct ObservedYouTubePost {
    pub provider_post_key: String,
    pub media_section: String,
    pub view_count: Option<i64>,
}

#[derive(Clone)]
pub struct DownloadedYouTubeMedia {
    pub file_path: PathBuf,
    pub media_type: String,
    pub media_section: String,
    pub provider_media_key: String,
    pub provider_post_key: String,
    pub captured_at_timestamp: Option<i64>,
    pub final_file_name: String,
    pub title: Option<String>,
    pub duration_seconds: Option<i64>,
}

#[derive(Clone, Default)]
pub struct YouTubeManifestSummary {
    pub normalized_post_count: u32,
    pub discovered_asset_count: u32,
    pub queued_asset_count: u32,
    pub skipped_existing_post_count: u32,
    pub skipped_existing_asset_count: u32,
    pub downloaded_asset_count: u32,
}

pub struct YouTubeConnectorResult {
    pub observed_posts: Vec<ObservedYouTubePost>,
    pub downloaded_media: Vec<DownloadedYouTubeMedia>,
    pub section_errors: Vec<String>,
    pub rate_limited: bool,
    pub limit_aborted: bool,
    /// Stable channel id (`uploader_id`/`channel_id`), when resolved.
    pub resolved_user_id: Option<String>,
    /// Preenchido quando `is_duplicate_user_id` apontou que o channel id já
    /// pertence a outro perfil; nesse caso o download foi cancelado.
    pub duplicate_user_id: Option<String>,
    /// `true` when the channel could not be resolved (non-existent, terminated,
    /// or unavailable). The caller turns this into a "profile unavailable"
    /// sync problem instead of reporting a successful empty sync.
    pub profile_unavailable: bool,
    pub manifest_summary: YouTubeManifestSummary,
}

pub struct YouTubeProgress {
    pub label: String,
    pub detail: String,
    pub downloaded_items: Option<u32>,
    pub progress_percent: Option<u32>,
    pub indeterminate: bool,
}

#[derive(Clone)]
struct EnumeratedPost {
    post_id: String,
    webpage_url: String,
    media_section: String,
    view_count: Option<i64>,
}

struct EnumeratedPosts {
    posts: Vec<EnumeratedPost>,
    uploader_id: Option<String>,
    rate_limited: bool,
    /// stderr indicated the channel/tab does not exist or is unavailable.
    unavailable: bool,
}

pub fn run_profile_sync<F, C, D>(
    request: &YouTubeConnectorRequest,
    mut report_progress: F,
    is_cancelled: C,
    is_duplicate_user_id: D,
) -> Result<YouTubeConnectorResult, String>
where
    F: FnMut(YouTubeProgress),
    C: Fn() -> bool,
    D: Fn(&str) -> bool,
{
    fs::create_dir_all(&request.cache_root).map_err(|error| error.to_string())?;
    fs::create_dir_all(&request.profile_root).map_err(|error| error.to_string())?;

    let handle = request.handle.trim().trim_start_matches('@').to_string();

    let mut observed_posts: Vec<ObservedYouTubePost> = Vec::new();
    let mut downloaded_media: Vec<DownloadedYouTubeMedia> = Vec::new();
    let mut section_errors: Vec<String> = Vec::new();
    let mut rate_limited = false;
    let mut limit_aborted = false;
    let mut duplicate_user_id: Option<String> = None;
    let mut summary = YouTubeManifestSummary::default();

    if is_cancelled() {
        return Err("source sync cancelled by user".to_string());
    }

    // Enumerate the enabled channel tabs. Each tab is a separate flat-playlist
    // listing; a post's `media_section` is `videos` or `shorts` accordingly.
    let sections: Vec<(&str, &str)> = {
        let mut list = Vec::new();
        if request.sections.videos {
            list.push(("videos", "videos"));
        }
        if request.sections.shorts {
            list.push(("shorts", "shorts"));
        }
        list
    };

    let mut resolved_user_id: Option<String> = None;
    let mut selected: Vec<EnumeratedPost> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut any_unavailable = false;
    let mut any_listed = false;

    for (url_suffix, media_section) in &sections {
        if is_cancelled() {
            return Err("source sync cancelled by user".to_string());
        }
        let profile_url = format!("https://www.youtube.com/@{handle}/{url_suffix}");
        report_progress(YouTubeProgress {
            label: "Parsing channel".to_string(),
            detail: format!("Listing YouTube {media_section} for '{handle}'."),
            downloaded_items: Some(0),
            progress_percent: Some(0),
            indeterminate: true,
        });
        let mut listed_count: u32 = 0;
        let listed = enumerate_posts(
            request,
            &profile_url,
            media_section,
            &is_cancelled,
            &mut |line| {
                if line.trim().is_empty() {
                    return;
                }
                listed_count += 1;
                if listed_count.is_multiple_of(10) {
                    report_progress(YouTubeProgress {
                        label: "Parsing channel".to_string(),
                        detail: format!(
                            "Listed {listed_count} {media_section} item(s) so far for '{handle}'."
                        ),
                        downloaded_items: Some(0),
                        progress_percent: None,
                        indeterminate: true,
                    });
                }
            },
        )?;
        connector_debug::append_current(
            "internal.youtube",
            "system",
            "listing.complete",
            format!(
                "section={media_section}\nposts_received={}\nuploader_id={}\nrate_limited={}\nunavailable={}",
                listed.posts.len(),
                listed.uploader_id.as_deref().unwrap_or("unknown"),
                listed.rate_limited,
                listed.unavailable
            ),
        );
        rate_limited = rate_limited || listed.rate_limited;
        any_unavailable = any_unavailable || listed.unavailable;
        if !listed.posts.is_empty() {
            any_listed = true;
        }
        if resolved_user_id.is_none() {
            resolved_user_id = listed.uploader_id.clone();
        }
        for post in listed.posts {
            summary.normalized_post_count += 1;
            if !seen.insert(post.post_id.clone()) {
                continue;
            }
            if request.ledger_post_keys.contains(&post.post_id) {
                summary.skipped_existing_post_count += 1;
                continue;
            }
            summary.discovered_asset_count += 1;
            selected.push(post);
        }
    }

    // Duplicate on first sync: cancel before downloading anything.
    if let Some(uid) = resolved_user_id.as_deref() {
        if is_duplicate_user_id(uid) {
            duplicate_user_id = Some(uid.to_string());
        }
    }

    // Channel unresolvable: nothing was listed and yt-dlp signalled that the
    // channel/tab is unavailable. We only mark the profile unavailable if there
    // is no known history (empty channel that once had posts is fail-open).
    let profile_unavailable = duplicate_user_id.is_none()
        && !any_listed
        && any_unavailable
        && !has_known_youtube_history(request);

    if duplicate_user_id.is_some() || profile_unavailable {
        return Ok(YouTubeConnectorResult {
            observed_posts,
            downloaded_media,
            section_errors,
            rate_limited,
            limit_aborted,
            resolved_user_id,
            duplicate_user_id,
            profile_unavailable,
            manifest_summary: summary,
        });
    }

    summary.queued_asset_count = selected.len() as u32;
    connector_debug::append_current(
        "internal.youtube",
        "system",
        "selection.complete",
        format!(
            "normalized_posts={}\nselected_posts={}\nskipped_existing_posts={}\ndownload_batch_size={DOWNLOAD_BATCH_SIZE}",
            summary.normalized_post_count,
            selected.len(),
            summary.skipped_existing_post_count
        ),
    );

    let total = selected.len();
    let mut processed = 0_usize;
    let mut downloaded_post_ids: HashSet<String> = HashSet::new();
    for batch in selected.chunks(DOWNLOAD_BATCH_SIZE) {
        if is_cancelled() {
            return Err("source sync cancelled by user".to_string());
        }
        let batch_base = processed;
        processed += batch.len();
        let percent_for = |completed: usize| -> u32 {
            if total > 0 {
                (((completed.min(total)) as f64 / total as f64) * 100.0).round() as u32
            } else {
                0
            }
        };
        report_progress(YouTubeProgress {
            label: "Downloading videos".to_string(),
            detail: format!("Video {} of {total}", (batch_base + 1).min(total.max(1))),
            downloaded_items: Some(summary.downloaded_asset_count),
            progress_percent: Some(percent_for(batch_base).min(100)),
            indeterminate: false,
        });

        let batch_started = Instant::now();
        let downloaded_before_batch = summary.downloaded_asset_count;
        let mut batch_completed = 0_usize;
        let batch_result = download_batch(request, batch, &is_cancelled, &mut |line| {
            if line.trim().is_empty() {
                return;
            }
            batch_completed = (batch_completed + 1).min(batch.len());
            let done_overall = batch_base + batch_completed;
            report_progress(YouTubeProgress {
                label: "Downloading videos".to_string(),
                detail: format!("Video {done_overall} of {total}"),
                downloaded_items: Some(downloaded_before_batch + batch_completed as u32),
                progress_percent: Some(percent_for(done_overall).min(100)),
                indeterminate: false,
            });
        });
        match batch_result {
            Ok(outcome) => {
                connector_debug::append_current(
                    "internal.youtube",
                    "response",
                    "batch.download",
                    format!(
                        "elapsed_ms={}\nmedia_produced={}\nerrors={}\nrate_limited={}",
                        batch_started.elapsed().as_millis(),
                        outcome.media.len(),
                        outcome.errors.len(),
                        outcome.rate_limited
                    ),
                );
                if outcome.rate_limited {
                    rate_limited = true;
                }
                section_errors.extend(outcome.errors);
                for media in outcome.media {
                    if request.ledger_media_keys.contains(&media.provider_media_key)
                        || request
                            .existing_relative_paths
                            .contains(&media.final_file_name)
                    {
                        summary.skipped_existing_asset_count += 1;
                        continue;
                    }
                    downloaded_post_ids.insert(media.provider_post_key.clone());
                    summary.downloaded_asset_count += 1;
                    downloaded_media.push(media);
                }
                if outcome.rate_limited && request.abort_on_limit {
                    limit_aborted = processed < total;
                    if limit_aborted {
                        section_errors.push(
                            "YouTube rate limit reached; remaining videos were skipped."
                                .to_string(),
                        );
                        break;
                    }
                }
            }
            Err(error) => {
                let lowered = error.to_ascii_lowercase();
                if lowered.contains("cancelled by user") {
                    return Err(error);
                }
                section_errors.push(format!("download batch failed: {error}"));
            }
        }

        if request.sleep_timer_secs > 0 && processed < total {
            interruptible_sleep(
                Duration::from_secs(request.sleep_timer_secs as u64),
                &is_cancelled,
            );
        }
    }

    for post in &selected {
        if downloaded_post_ids.contains(&post.post_id) {
            observed_posts.push(ObservedYouTubePost {
                provider_post_key: post.post_id.clone(),
                media_section: post.media_section.clone(),
                view_count: if request.collect_media_stats {
                    post.view_count
                } else {
                    None
                },
            });
        }
    }

    report_progress(YouTubeProgress {
        label: "Finished".to_string(),
        detail: format!("Downloaded {} media items.", summary.downloaded_asset_count),
        downloaded_items: Some(summary.downloaded_asset_count),
        progress_percent: Some(100),
        indeterminate: false,
    });

    Ok(YouTubeConnectorResult {
        observed_posts,
        downloaded_media,
        section_errors,
        rate_limited,
        limit_aborted,
        resolved_user_id,
        duplicate_user_id,
        profile_unavailable: false,
        manifest_summary: summary,
    })
}

/// Enumerates a channel tab (`/videos` or `/shorts`) with `--flat-playlist`.
fn enumerate_posts<C>(
    request: &YouTubeConnectorRequest,
    profile_url: &str,
    media_section: &str,
    is_cancelled: &C,
    on_listed_line: &mut dyn FnMut(&str),
) -> Result<EnumeratedPosts, String>
where
    C: Fn() -> bool,
{
    let mut command = Command::new(&request.yt_dlp_executable);
    command
        .arg("--ignore-errors")
        .arg("--no-warnings")
        .arg("--flat-playlist")
        .arg("--print")
        .arg("%(id)s\t%(webpage_url)s\t%(channel_id)s\t%(view_count)s");
    apply_cookies(&mut command, request);
    apply_user_agent(&mut command, request);
    command
        .arg(profile_url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let (stdout, stderr) = run_capturing_streaming(
        command,
        YT_DLP_LIST_TIMEOUT_SECS,
        is_cancelled,
        "yt-dlp (listing)",
        on_listed_line,
    )?;
    let rate_limited = output_is_rate_limited(&stderr);
    let unavailable = output_is_channel_unavailable(&stderr);

    let mut posts = Vec::new();
    let mut uploader_id = None;
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let post_id = parts.next().unwrap_or("").trim();
        let webpage_url = parts.next().unwrap_or("").trim();
        let channel_id = parts.next().unwrap_or("").trim();
        let view_count = parse_optional_count(parts.next());
        if post_id.is_empty() || post_id == "NA" {
            continue;
        }
        if uploader_id.is_none() && !channel_id.is_empty() && channel_id != "NA" {
            uploader_id = Some(channel_id.to_string());
        }
        let url = if webpage_url.is_empty() || webpage_url == "NA" {
            youtube_post_url(post_id, media_section)
        } else {
            webpage_url.to_string()
        };
        posts.push(EnumeratedPost {
            post_id: post_id.to_string(),
            webpage_url: url,
            media_section: media_section.to_string(),
            view_count,
        });
    }

    Ok(EnumeratedPosts {
        posts,
        uploader_id,
        rate_limited,
        unavailable,
    })
}

/// Canonical watch/shorts URL for a video id.
fn youtube_post_url(post_id: &str, media_section: &str) -> String {
    if media_section == "shorts" {
        format!("https://www.youtube.com/shorts/{post_id}")
    } else {
        format!("https://www.youtube.com/watch?v={post_id}")
    }
}

fn parse_optional_count(value: Option<&str>) -> Option<i64> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "NA")
        .and_then(|value| value.parse::<i64>().ok())
}

struct BatchOutcome {
    media: Vec<DownloadedYouTubeMedia>,
    rate_limited: bool,
    errors: Vec<String>,
}

/// Downloads a batch of videos in a single yt-dlp invocation. `after_move`
/// reports the timestamp, id, path, title, duration and view count for every
/// produced file (tab-separated); each is moved to the final folder with the
/// date prefix.
fn download_batch<C>(
    request: &YouTubeConnectorRequest,
    batch: &[EnumeratedPost],
    is_cancelled: &C,
    on_stdout_line: &mut dyn FnMut(&str),
) -> Result<BatchOutcome, String>
where
    C: Fn() -> bool,
{
    let download_dir = request.cache_root.join("dl");
    let _ = fs::remove_dir_all(&download_dir);
    fs::create_dir_all(&download_dir).map_err(|error| error.to_string())?;

    // Map post id -> section so produced files land in the right media section.
    let section_by_id: std::collections::HashMap<String, String> = batch
        .iter()
        .map(|post| (post.post_id.clone(), post.media_section.clone()))
        .collect();

    let mut command = Command::new(&request.yt_dlp_executable);
    command
        .arg("--ignore-errors")
        .arg("--no-warnings")
        .arg("--no-playlist")
        .arg("--no-simulate")
        .arg("--extractor-retries")
        .arg("3")
        .arg("--retries")
        .arg("5");
    if request.use_parsed_video_date {
        command.arg("--mtime");
    } else {
        command.arg("--no-mtime");
    }
    apply_cookies(&mut command, request);
    apply_user_agent(&mut command, request);
    command
        .arg("-P")
        .arg(&download_dir)
        .arg("-o")
        .arg("%(id)s.%(ext)s")
        .arg("--print")
        .arg("after_move:%(timestamp)s\t%(id)s\t%(filepath)s\t%(title)s\t%(duration)s\t%(view_count)s");
    for post in batch {
        command.arg(&post.webpage_url);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let (stdout, stderr) = run_capturing_streaming(
        command,
        YT_DLP_DOWNLOAD_TIMEOUT_SECS,
        is_cancelled,
        "yt-dlp (download)",
        on_stdout_line,
    )?;
    let rate_limited = output_is_rate_limited(&stderr);

    let mut media = Vec::new();
    let errors = Vec::new();
    for line in stdout.lines() {
        if let Some(parsed) = parse_after_move_line(line) {
            if !request.download_videos {
                let _ = fs::remove_file(&parsed.file_path);
                continue;
            }
            let source_path = parsed.file_path.clone();
            if !source_path.exists() {
                continue;
            }
            let section = section_by_id
                .get(&parsed.post_id)
                .cloned()
                .unwrap_or_else(|| "videos".to_string());
            match finalize_media_file(request, &source_path, &parsed, &section) {
                Ok(item) => media.push(item),
                Err(_) => {
                    let _ = fs::remove_file(&source_path);
                }
            }
        }
    }
    let _ = fs::remove_dir_all(&download_dir);

    Ok(BatchOutcome {
        media,
        rate_limited,
        errors,
    })
}

/// One parsed `after_move` line from the download invocation.
struct AfterMove {
    captured_at_timestamp: Option<i64>,
    post_id: String,
    file_path: PathBuf,
    title: Option<String>,
    duration_seconds: Option<i64>,
}

fn parse_after_move_line(line: &str) -> Option<AfterMove> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let mut parts = line.split('\t');
    let timestamp = parts.next().unwrap_or("").trim();
    let post_id = parts.next().unwrap_or("").trim();
    let file_path = parts.next().unwrap_or("").trim();
    let title = parts.next().unwrap_or("").trim();
    let duration = parts.next().unwrap_or("").trim();
    // The trailing `%(view_count)s` column is captured for the debug log but not
    // stored here (enumeration already supplies stats); ignore it.
    if post_id.is_empty() || file_path.is_empty() {
        return None;
    }
    Some(AfterMove {
        captured_at_timestamp: timestamp.parse::<i64>().ok().filter(|value| *value > 0),
        post_id: post_id.to_string(),
        file_path: PathBuf::from(file_path),
        title: normalize_optional_text(title),
        // yt-dlp prints duration as a float (e.g. `61.0`); take the whole part.
        duration_seconds: normalize_optional_text(duration)
            .and_then(|value| value.split('.').next().map(str::to_string))
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|value| *value >= 0),
    })
}

fn normalize_optional_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "NA" {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Moves a downloaded file to the final folder with the date prefix, routing
/// videos into the `Video` subfolder when configured.
fn finalize_media_file(
    request: &YouTubeConnectorRequest,
    source_path: &Path,
    parsed: &AfterMove,
    media_section: &str,
) -> Result<DownloadedYouTubeMedia, String> {
    let extension = source_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_video = VIDEO_EXTENSIONS.contains(&extension.as_str());
    let is_image = IMAGE_EXTENSIONS.contains(&extension.as_str());
    if !is_video && !is_image {
        return Err(format!("unsupported media extension '{extension}'"));
    }
    let media_type = if is_video { "video" } else { "image" };

    let raw_name = source_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "invalid file name".to_string())?
        .to_string();
    let final_file_name = timestamped_file_name(parsed.captured_at_timestamp, &raw_name);

    let target_dir = if is_video && request.separate_video_folder {
        request.profile_root.join("Video")
    } else {
        request.profile_root.clone()
    };
    fs::create_dir_all(&target_dir).map_err(|error| error.to_string())?;
    let destination = target_dir.join(&final_file_name);
    if atomic_file::is_nonempty_file(&destination) {
        return Err("destination already exists".to_string());
    }
    if !atomic_file::is_nonempty_file(source_path) {
        return Err("downloaded source file is empty".to_string());
    }
    if destination.exists() {
        fs::remove_file(&destination).map_err(|error| error.to_string())?;
    }
    if fs::rename(source_path, &destination).is_err() {
        atomic_file::copy_file_replacing_empty(source_path, &destination)?;
        let _ = fs::remove_file(source_path);
    }

    Ok(DownloadedYouTubeMedia {
        file_path: destination,
        media_type: media_type.to_string(),
        media_section: media_section.to_string(),
        provider_media_key: final_file_name.clone(),
        provider_post_key: parsed.post_id.clone(),
        captured_at_timestamp: parsed.captured_at_timestamp,
        final_file_name,
        title: parsed.title.clone(),
        duration_seconds: parsed.duration_seconds,
    })
}

fn apply_cookies(command: &mut Command, request: &YouTubeConnectorRequest) {
    if atomic_file::is_nonempty_file(&request.cookie_file) {
        command
            .arg("--no-cookies-from-browser")
            .arg("--cookies")
            .arg(&request.cookie_file);
    }
}

fn apply_user_agent(command: &mut Command, request: &YouTubeConnectorRequest) {
    if let Some(user_agent) = request
        .user_agent
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        command.arg("--user-agent").arg(user_agent);
    }
}

fn has_known_youtube_history(request: &YouTubeConnectorRequest) -> bool {
    !request.ledger_post_keys.is_empty()
        || !request.ledger_media_keys.is_empty()
        || !request.existing_relative_paths.is_empty()
}

fn interruptible_sleep(total: Duration, is_cancelled: &dyn Fn() -> bool) {
    const STEP: Duration = Duration::from_millis(200);
    let mut remaining = total;
    while !remaining.is_zero() {
        if is_cancelled() {
            return;
        }
        let chunk = STEP.min(remaining);
        thread::sleep(chunk);
        remaining -= chunk;
    }
}

fn run_capturing_streaming<C>(
    mut command: Command,
    timeout_secs: u64,
    is_cancelled: &C,
    label: &str,
    on_stdout_line: &mut dyn FnMut(&str),
) -> Result<(String, String), String>
where
    C: Fn() -> bool,
{
    let (line_sender, line_receiver) = std::sync::mpsc::channel::<String>();
    let context = connector_debug::current_context();
    let command_line = std::iter::once(command.get_program().to_string_lossy().to_string())
        .chain(
            command
                .get_args()
                .map(|arg| arg.to_string_lossy().to_string()),
        )
        .collect::<Vec<_>>()
        .join(" ");
    connector_debug::append_with_context(
        context.clone(),
        label,
        "call",
        "process.spawn",
        command_line,
    );
    let mut child = command.spawn().map_err(|error| {
        connector_debug::append_with_context(
            context.clone(),
            label,
            "error",
            "process.spawn",
            error.to_string(),
        );
        format!("Failed to start {label}: {error}")
    })?;

    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();
    let stdout_context = context.clone();
    let stdout_label = label.to_string();
    let stdout_reader = thread::spawn(move || {
        let mut lines = Vec::new();
        if let Some(handle) = stdout_handle {
            for line in BufReader::new(handle).lines().map_while(Result::ok) {
                connector_debug::append_with_context(
                    stdout_context.clone(),
                    &stdout_label,
                    "stdout",
                    "process.output",
                    line.clone(),
                );
                let _ = line_sender.send(line.clone());
                lines.push(line);
            }
        }
        lines.join("\n")
    });
    let stderr_context = context.clone();
    let stderr_label = label.to_string();
    let stderr_reader = thread::spawn(move || {
        let mut lines = Vec::new();
        if let Some(handle) = stderr_handle {
            for line in BufReader::new(handle).lines().map_while(Result::ok) {
                connector_debug::append_with_context(
                    stderr_context.clone(),
                    &stderr_label,
                    "stderr",
                    "process.output",
                    line.clone(),
                );
                lines.push(line);
            }
        }
        lines.join("\n")
    });

    let started = std::time::Instant::now();
    let mut cancelled = false;
    let mut timed_out = false;
    loop {
        while let Ok(line) = line_receiver.try_recv() {
            on_stdout_line(&line);
        }
        if is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            cancelled = true;
            break;
        }
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(_status) => break,
            None => {
                if started.elapsed() > Duration::from_secs(timeout_secs) {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break;
                }
                thread::sleep(Duration::from_millis(250));
            }
        }
    }

    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    while let Ok(line) = line_receiver.try_recv() {
        on_stdout_line(&line);
    }
    if cancelled {
        return Err("source sync cancelled by user".to_string());
    }
    if timed_out {
        return Err(format!("{label} timed out."));
    }
    Ok((stdout, stderr))
}

fn output_is_rate_limited(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    lowered.contains("429") || lowered.contains("rate limit") || lowered.contains("rate-limit")
}

/// Detects yt-dlp stderr markers that indicate the channel (or the requested
/// tab) does not exist or is otherwise unavailable, so an empty listing can be
/// reported as a blocking problem instead of a successful zero-post sync.
fn output_is_channel_unavailable(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    lowered.contains("does not exist")
        || lowered.contains("this channel does not have")
        || lowered.contains("not available")
        || lowered.contains("has been terminated")
        || lowered.contains("account associated with this")
        || lowered.contains("http error 404")
        || lowered.contains("unable to recognize tab")
}

fn timestamped_file_name(captured_at_timestamp: Option<i64>, raw_file_name: &str) -> String {
    match captured_at_timestamp.and_then(|value| Local.timestamp_opt(value, 0).single()) {
        Some(local_time) => {
            format!(
                "{} {}",
                local_time.format("%Y-%m-%d %H.%M.%S"),
                raw_file_name
            )
        }
        None => raw_file_name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamped_file_name_prefixes_local_date() {
        let named = timestamped_file_name(Some(1_700_000_000), "dQw4w9WgXcQ.mp4");
        assert!(named.ends_with("dQw4w9WgXcQ.mp4"));
        assert!(named.len() > "dQw4w9WgXcQ.mp4".len());
    }

    #[test]
    fn timestamped_file_name_without_timestamp_is_raw() {
        assert_eq!(timestamped_file_name(None, "dQw4w9WgXcQ.mp4"), "dQw4w9WgXcQ.mp4");
    }

    #[test]
    fn youtube_post_url_distinguishes_shorts() {
        assert_eq!(
            youtube_post_url("dQw4w9WgXcQ", "videos"),
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
        );
        assert_eq!(
            youtube_post_url("abc123", "shorts"),
            "https://www.youtube.com/shorts/abc123"
        );
    }

    #[test]
    fn parse_after_move_line_extracts_metadata() {
        let parsed = parse_after_move_line(
            "1700000000\tdQw4w9WgXcQ\t/tmp/dl/dQw4w9WgXcQ.mp4\tNever Gonna Give You Up\t212.0\t1600000000",
        )
        .expect("parsed");
        assert_eq!(parsed.post_id, "dQw4w9WgXcQ");
        assert_eq!(parsed.captured_at_timestamp, Some(1_700_000_000));
        assert_eq!(parsed.title.as_deref(), Some("Never Gonna Give You Up"));
        assert_eq!(parsed.duration_seconds, Some(212));
    }

    #[test]
    fn parse_after_move_line_handles_na_fields() {
        let parsed =
            parse_after_move_line("NA\tabc123\t/tmp/dl/abc123.webm\tNA\tNA\tNA").expect("parsed");
        assert_eq!(parsed.post_id, "abc123");
        assert_eq!(parsed.captured_at_timestamp, None);
        assert_eq!(parsed.title, None);
        assert_eq!(parsed.duration_seconds, None);
    }

    #[test]
    fn parse_after_move_line_rejects_missing_path() {
        assert!(parse_after_move_line("1700000000\tabc123\t").is_none());
        assert!(parse_after_move_line("").is_none());
    }

    #[test]
    fn rate_limit_detection_matches_common_markers() {
        assert!(output_is_rate_limited("HTTP Error 429: Too Many Requests"));
        assert!(output_is_rate_limited("rate-limit reached"));
        assert!(!output_is_rate_limited("downloaded 10 files"));
    }

    #[test]
    fn channel_unavailable_detection_matches_markers() {
        assert!(output_is_channel_unavailable(
            "ERROR: [youtube:tab] @nope: This channel does not exist."
        ));
        assert!(output_is_channel_unavailable(
            "ERROR: The account associated with this channel has been terminated"
        ));
        assert!(output_is_channel_unavailable("HTTP Error 404: Not Found"));
        assert!(!output_is_channel_unavailable("downloaded 10 files"));
    }
}
