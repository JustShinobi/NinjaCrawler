//! Internal VSCO connector.
//!
//! Mirrors the Twitter connector contract: gallery-dl is used only as a *parser*
//! (`--no-download --no-skip --write-pages`) to obtain the JSON pages of VSCO's
//! media API; the media download, naming and ledger cataloguing stay under
//! NinjaCrawler control (reqwest + SQLite).
//!
//! VSCO is photo-first (`vsco.co/<user>/gallery`) and normally works without
//! cookies, so authentication is optional. Each VSCO media item is its own
//! post, so `provider_post_key == provider_media_key == _id`.
//!
//! Pagination: the gallery-dl VSCO extractor paginates its API internally and is
//! not rate-limited like the X/Twitter timeline, so there is no resume cursor.
//! Every sync re-enumerates the whole gallery (like the TikTok/YouTube
//! connectors) and relies on the ledgers/disk state to skip what already exists.

use chrono::{Local, TimeZone};
use reqwest::blocking::Client;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use crate::infrastructure::{atomic_file, connector_debug};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const DEFAULT_DOWNLOAD_TIMEOUT_SECS: u64 = 120;
const GALLERY_DL_TIMEOUT_SECS: u64 = 600;
const VSCO_REQUEST_SLEEP_RANGE: &str = "1.0-2.5";

#[derive(Clone, Copy, Default)]
pub struct VscoSectionSelection {
    pub gallery: bool,
    pub journal: bool,
}

#[derive(Clone)]
pub struct VscoConnectorRequest {
    pub handle: String,
    pub gallery_dl_executable: PathBuf,
    /// Netscape cookie file written by the caller. Passed to gallery-dl only when
    /// it exists and is non-empty (VSCO usually works without authentication).
    pub cookie_file: PathBuf,
    pub user_agent: Option<String>,
    pub profile_root: PathBuf,
    /// Working directory for the config and the temporary parser pages.
    pub cache_root: PathBuf,
    pub sections: VscoSectionSelection,
    pub download_images: bool,
    pub download_videos: bool,
    /// Routes videos into the `Video` subfolder (SCrawler SeparateVideoFolder).
    pub separate_video_folder: bool,
    /// Discards byte-identical downloads comparing the sha256 (UseMD5Comparison).
    pub use_md5_comparison: bool,
    pub ledger_post_keys: HashSet<String>,
    pub ledger_media_keys: HashSet<String>,
    pub existing_relative_paths: HashSet<String>,
    /// Stable numeric user id (`site_id`), when already known.
    pub user_id_hint: Option<String>,
}

#[derive(Clone)]
pub struct ObservedVscoPost {
    pub provider_post_key: String,
    pub media_section: String,
}

#[derive(Clone)]
pub struct DownloadedVscoMedia {
    pub file_path: PathBuf,
    pub media_type: String,
    pub media_section: String,
    pub provider_media_key: String,
    pub provider_post_key: String,
    pub captured_at_timestamp: Option<i64>,
    pub final_file_name: String,
}

#[derive(Clone, Default)]
pub struct VscoManifestSummary {
    pub parsed_page_count: u32,
    pub normalized_post_count: u32,
    pub discovered_asset_count: u32,
    pub queued_asset_count: u32,
    pub skipped_existing_post_count: u32,
    pub skipped_existing_asset_count: u32,
    pub skipped_disabled_asset_count: u32,
    pub skipped_duplicate_asset_count: u32,
    pub downloaded_asset_count: u32,
    pub downloaded_by_section: BTreeMap<String, u32>,
}

pub struct VscoConnectorResult {
    pub observed_posts: Vec<ObservedVscoPost>,
    pub downloaded_media: Vec<DownloadedVscoMedia>,
    /// New media keys whose content already existed in another file. Persisted as
    /// aliases in the ledger, but not counted as downloads.
    pub deduplicated_media_aliases: Vec<DownloadedVscoMedia>,
    pub section_errors: Vec<String>,
    /// Stable numeric user id (`site_id`) resolved from the pages, when present.
    pub resolved_user_id: Option<String>,
    /// Avatar URL (`responsive_url` of the profile image) when present. The
    /// caller downloads and persists it as the profile picture.
    pub resolved_avatar_url: Option<String>,
    /// Filled when `is_duplicate_user_id` reported that the user id already
    /// belongs to another profile; in that case the download was cancelled.
    pub duplicate_user_id: Option<String>,
    /// `true` when the profile could not be resolved (non-existent or private).
    /// The caller turns this into a blocking sync problem instead of reporting a
    /// successful empty sync.
    pub profile_unavailable: bool,
    pub manifest_summary: VscoManifestSummary,
}

pub struct VscoProgress {
    pub label: String,
    pub detail: String,
    pub downloaded_items: Option<u32>,
    pub progress_percent: Option<u32>,
    pub indeterminate: bool,
}

#[derive(Clone)]
struct ParsedVscoMedia {
    /// VSCO media id, used both as the media key and the post key.
    media_id: String,
    media_type: String,
    file_url: String,
    file_name: String,
    captured_at_timestamp: Option<i64>,
}

struct SectionRun {
    media_section: &'static str,
    url: String,
}

fn vsco_section_label(section: &str) -> &'static str {
    match section {
        "gallery" => "profile gallery",
        "journal" => "profile journal",
        _ => "VSCO media",
    }
}

pub fn run_profile_sync<F, C, D>(
    request: &VscoConnectorRequest,
    mut report_progress: F,
    is_cancelled: C,
    is_duplicate_user_id: D,
) -> Result<VscoConnectorResult, String>
where
    F: FnMut(VscoProgress),
    C: Fn() -> bool,
    D: Fn(&str) -> bool,
{
    let handle = request.handle.trim().trim_start_matches('@').to_string();
    if handle.is_empty() {
        return Err("VSCO handle is required.".to_string());
    }

    fs::create_dir_all(&request.cache_root).map_err(|error| error.to_string())?;
    fs::create_dir_all(&request.profile_root).map_err(|error| error.to_string())?;
    let config_path = write_gallery_dl_config(request)?;

    let mut runs: Vec<SectionRun> = Vec::new();
    if request.sections.gallery {
        runs.push(SectionRun {
            media_section: "gallery",
            url: format!("https://vsco.co/{handle}/gallery"),
        });
    }
    if request.sections.journal {
        runs.push(SectionRun {
            media_section: "journal",
            url: format!("https://vsco.co/{handle}/journal"),
        });
    }
    if runs.is_empty() {
        let _ = fs::remove_file(&config_path);
        return Err("No VSCO download section is enabled for this profile.".to_string());
    }

    let mut summary = VscoManifestSummary::default();
    let mut section_errors: Vec<String> = Vec::new();
    let mut planned_downloads: Vec<DownloadPlanEntry> = Vec::new();
    let mut pending_posts: Vec<PendingVscoPost> = Vec::new();
    let mut seen_media_ids: HashSet<String> = HashSet::new();
    let mut available_media_keys = request.ledger_media_keys.clone();
    let mut resolved_user_id: Option<String> = None;
    let mut resolved_avatar_url: Option<String> = None;
    let mut any_unavailable = false;
    let mut any_parsed = false;
    let total_runs = runs.len();

    for (run_index, run) in runs.iter().enumerate() {
        if is_cancelled() {
            let _ = fs::remove_file(&config_path);
            return Err("source sync cancelled by user".to_string());
        }

        report_progress(VscoProgress {
            label: format!("Parsing {}", vsco_section_label(run.media_section)),
            detail: format!(
                "gallery-dl is fetching the {} ({}/{}).",
                vsco_section_label(run.media_section),
                run_index + 1,
                total_runs
            ),
            downloaded_items: None,
            progress_percent: None,
            indeterminate: true,
        });

        let pages_dir = request
            .cache_root
            .join(format!("pages-{}", run.media_section));
        let _ = fs::remove_dir_all(&pages_dir);
        fs::create_dir_all(&pages_dir).map_err(|error| error.to_string())?;

        let parse_outcome =
            run_gallery_dl_parser(request, &config_path, &run.url, &pages_dir, &is_cancelled);
        let unavailable = match parse_outcome {
            Ok(page_unavailable) => page_unavailable,
            Err(error) => {
                if is_cancelled() || error.contains("cancelled by user") {
                    let _ = fs::remove_file(&config_path);
                    return Err(error);
                }
                section_errors.push(format!("{}: {}", run.media_section, error));
                let _ = fs::remove_dir_all(&pages_dir);
                continue;
            }
        };
        any_unavailable = any_unavailable || unavailable;

        let (media_items, page_avatar, page_user_id) =
            parse_media_from_pages(&pages_dir, &mut summary)?;
        any_parsed = true;
        if resolved_user_id.is_none() {
            resolved_user_id = page_user_id;
        }
        if resolved_avatar_url.is_none() {
            resolved_avatar_url = page_avatar;
        }
        // First sync: let the caller decide whether the user id is a duplicate
        // before any media is downloaded.
        if let Some(uid) = resolved_user_id.clone() {
            if is_duplicate_user_id(&uid) {
                let _ = fs::remove_dir_all(&pages_dir);
                let _ = fs::remove_file(&config_path);
                return Ok(VscoConnectorResult {
                    observed_posts: Vec::new(),
                    downloaded_media: Vec::new(),
                    deduplicated_media_aliases: Vec::new(),
                    section_errors,
                    resolved_user_id,
                    resolved_avatar_url,
                    duplicate_user_id: Some(uid),
                    profile_unavailable: false,
                    manifest_summary: summary,
                });
            }
        }

        for media in media_items {
            if !seen_media_ids.insert(media.media_id.clone()) {
                continue;
            }
            summary.normalized_post_count += 1;
            summary.discovered_asset_count += 1;

            let allowed = match media.media_type.as_str() {
                "video" => request.download_videos,
                _ => request.download_images,
            };
            if !allowed {
                summary.skipped_disabled_asset_count += 1;
                continue;
            }

            let was_known_post = request.ledger_post_keys.contains(&media.media_id);
            let mut had_missing_asset = false;
            if request.ledger_media_keys.contains(&media.media_id)
                || asset_exists_on_disk(request, &media)
            {
                summary.skipped_existing_asset_count += 1;
                available_media_keys.insert(media.media_id.clone());
            } else {
                had_missing_asset = true;
                summary.queued_asset_count += 1;
                planned_downloads.push(DownloadPlanEntry {
                    media: media.clone(),
                    media_section: run.media_section.to_string(),
                });
            }
            pending_posts.push(PendingVscoPost {
                provider_post_key: media.media_id,
                media_section: run.media_section.to_string(),
                was_known_post,
                had_missing_asset,
            });
        }

        let _ = fs::remove_dir_all(&pages_dir);
    }

    // Profile unresolvable: nothing parsed and gallery-dl reported the profile is
    // unavailable. Fail-open when there is known history (empty gallery).
    let profile_unavailable = !any_parsed
        && any_unavailable
        && !has_known_vsco_history(request)
        && pending_posts.is_empty();
    if profile_unavailable {
        let _ = fs::remove_file(&config_path);
        return Ok(VscoConnectorResult {
            observed_posts: Vec::new(),
            downloaded_media: Vec::new(),
            deduplicated_media_aliases: Vec::new(),
            section_errors,
            resolved_user_id,
            resolved_avatar_url,
            duplicate_user_id: None,
            profile_unavailable: true,
            manifest_summary: summary,
        });
    }

    let mut downloaded_media: Vec<DownloadedVscoMedia> = Vec::new();
    let mut deduplicated_media_aliases: Vec<DownloadedVscoMedia> = Vec::new();
    let client = build_download_client(request)?;
    let mut known_hashes = if request.use_md5_comparison {
        seed_existing_hashes(&request.profile_root)
    } else {
        HashMap::new()
    };
    let total = planned_downloads.len();
    for (index, entry) in planned_downloads.iter().enumerate() {
        if is_cancelled() {
            let _ = fs::remove_file(&config_path);
            return Err("source sync cancelled by user".to_string());
        }
        report_progress(VscoProgress {
            label: format!("Downloading {}", vsco_section_label(&entry.media_section)),
            detail: format!(
                "{}: {} ({}/{})",
                vsco_section_label(&entry.media_section),
                entry.media.file_name,
                index + 1,
                total
            ),
            downloaded_items: Some(downloaded_media.len() as u32),
            progress_percent: Some(((index * 100) / total.max(1)) as u32),
            indeterminate: false,
        });

        match download_asset(&client, request, entry) {
            Ok(media) => {
                if request.use_md5_comparison {
                    if let Ok(hash) = file_sha256(&media.file_path) {
                        if let Some(canonical_path) = known_hashes.get(&hash).cloned() {
                            let _ = fs::remove_file(&media.file_path);
                            summary.skipped_duplicate_asset_count += 1;
                            available_media_keys.insert(entry.media.media_id.clone());
                            let mut alias = media;
                            alias.final_file_name = canonical_path
                                .file_name()
                                .and_then(|value| value.to_str())
                                .unwrap_or_default()
                                .to_string();
                            alias.file_path = canonical_path;
                            deduplicated_media_aliases.push(alias);
                            continue;
                        }
                        known_hashes.insert(hash, media.file_path.clone());
                    }
                }
                summary.downloaded_asset_count += 1;
                *summary
                    .downloaded_by_section
                    .entry(entry.media_section.clone())
                    .or_insert(0) += 1;
                available_media_keys.insert(entry.media.media_id.clone());
                downloaded_media.push(media);
            }
            Err(error) => {
                section_errors.push(format!(
                    "{}: download failed for '{}': {}",
                    entry.media_section, entry.media.file_name, error
                ));
            }
        }
    }

    let _ = fs::remove_file(&config_path);

    let observed_posts =
        completed_observed_posts(pending_posts, &available_media_keys, &mut summary);

    report_progress(VscoProgress {
        label: "Finishing".to_string(),
        detail: format!("Downloaded {} media files.", downloaded_media.len()),
        downloaded_items: Some(downloaded_media.len() as u32),
        progress_percent: Some(100),
        indeterminate: false,
    });

    Ok(VscoConnectorResult {
        observed_posts,
        downloaded_media,
        deduplicated_media_aliases,
        section_errors,
        resolved_user_id,
        resolved_avatar_url,
        duplicate_user_id: None,
        profile_unavailable: false,
        manifest_summary: summary,
    })
}

struct DownloadPlanEntry {
    media: ParsedVscoMedia,
    media_section: String,
}

#[derive(Clone)]
struct PendingVscoPost {
    provider_post_key: String,
    media_section: String,
    was_known_post: bool,
    had_missing_asset: bool,
}

fn completed_observed_posts(
    pending_posts: Vec<PendingVscoPost>,
    available_media_keys: &HashSet<String>,
    summary: &mut VscoManifestSummary,
) -> Vec<ObservedVscoPost> {
    pending_posts
        .into_iter()
        .filter_map(|post| {
            if available_media_keys.contains(&post.provider_post_key) {
                if post.was_known_post && !post.had_missing_asset {
                    summary.skipped_existing_post_count += 1;
                }
                Some(ObservedVscoPost {
                    provider_post_key: post.provider_post_key,
                    media_section: post.media_section,
                })
            } else {
                None
            }
        })
        .collect()
}

fn has_known_vsco_history(request: &VscoConnectorRequest) -> bool {
    !request.ledger_post_keys.is_empty()
        || !request.ledger_media_keys.is_empty()
        || !request.existing_relative_paths.is_empty()
}

fn write_gallery_dl_config(request: &VscoConnectorRequest) -> Result<PathBuf, String> {
    let config_path = request.cache_root.join("vsco-gdl-config.json");
    let mut extractor = serde_json::Map::new();
    // Cookies are optional for VSCO; only reference the file when it has content.
    if atomic_file::is_nonempty_file(&request.cookie_file) {
        extractor.insert(
            "cookies".to_string(),
            Value::String(request.cookie_file.display().to_string()),
        );
    }
    if let Some(user_agent) = request
        .user_agent
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        extractor.insert(
            "user-agent".to_string(),
            Value::String(user_agent.to_string()),
        );
    }

    let config = Value::Object(
        [("extractor".to_string(), Value::Object(extractor))]
            .into_iter()
            .collect(),
    );
    let serialized = serde_json::to_string_pretty(&config).map_err(|error| error.to_string())?;
    let mut file = fs::File::create(&config_path).map_err(|error| error.to_string())?;
    file.write_all(serialized.as_bytes())
        .map_err(|error| error.to_string())?;
    Ok(config_path)
}

fn stream_debug_file(path: &Path, offset: &mut usize, event_type: &str) {
    let Ok(bytes) = fs::read(path) else {
        return;
    };
    if bytes.len() <= *offset {
        return;
    }
    let chunk = String::from_utf8_lossy(&bytes[*offset..]).to_string();
    *offset = bytes.len();
    if !chunk.trim().is_empty() {
        connector_debug::append_current("gallery-dl", event_type, "parser.output", chunk);
    }
}

/// Runs gallery-dl as a parser for one VSCO section. Returns `true` when the
/// output indicates the profile/section is unavailable (non-existent/private).
fn run_gallery_dl_parser<C>(
    request: &VscoConnectorRequest,
    config_path: &Path,
    url: &str,
    pages_dir: &Path,
    is_cancelled: &C,
) -> Result<bool, String>
where
    C: Fn() -> bool,
{
    let stdout_log = request.cache_root.join("gdl-stdout.log");
    let stderr_log = request.cache_root.join("gdl-stderr.log");
    let stdout_file = fs::File::create(&stdout_log).map_err(|error| error.to_string())?;
    let stderr_file = fs::File::create(&stderr_log).map_err(|error| error.to_string())?;

    let mut command = Command::new(&request.gallery_dl_executable);
    command
        .arg("--verbose")
        .arg("--no-download")
        .arg("--no-skip")
        .arg("--sleep-request")
        .arg(VSCO_REQUEST_SLEEP_RANGE)
        .arg("--config")
        .arg(config_path)
        .arg("--write-pages")
        .arg(url)
        .current_dir(pages_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file));
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let command_line = std::iter::once(command.get_program().to_string_lossy().to_string())
        .chain(
            command
                .get_args()
                .map(|arg| arg.to_string_lossy().to_string()),
        )
        .collect::<Vec<_>>()
        .join(" ");
    connector_debug::append_current("gallery-dl", "call", "parser.spawn", command_line);
    let mut child = command.spawn().map_err(|error| {
        connector_debug::append_current("gallery-dl", "error", "parser.spawn", error.to_string());
        format!("Failed to start gallery-dl: {}", error)
    })?;

    let started = std::time::Instant::now();
    let mut stdout_offset = 0usize;
    let mut stderr_offset = 0usize;
    let status = loop {
        if is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err("source sync cancelled by user".to_string());
        }
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(status) => {
                stream_debug_file(&stdout_log, &mut stdout_offset, "stdout");
                stream_debug_file(&stderr_log, &mut stderr_offset, "stderr");
                break status;
            }
            None => {
                stream_debug_file(&stdout_log, &mut stdout_offset, "stdout");
                stream_debug_file(&stderr_log, &mut stderr_offset, "stderr");
                if started.elapsed() > Duration::from_secs(GALLERY_DL_TIMEOUT_SECS) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("gallery-dl parser timed out.".to_string());
                }
                thread::sleep(Duration::from_millis(250));
            }
        }
    };

    let stderr = fs::read_to_string(&stderr_log).unwrap_or_default();
    connector_debug::append_current(
        "gallery-dl",
        "response",
        "parser.exit",
        format!(
            "exit_code={}",
            status
                .code()
                .map_or_else(|| "terminated".to_string(), |code| code.to_string())
        ),
    );
    let _ = fs::remove_file(&stdout_log);
    let _ = fs::remove_file(&stderr_log);

    let unavailable = output_is_profile_unavailable(&stderr);
    let produced_pages = fs::read_dir(pages_dir)
        .map(|entries| entries.flatten().next().is_some())
        .unwrap_or(false);
    if !status.success() && !produced_pages && !unavailable {
        let detail = stderr
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("no error detail")
            .trim()
            .to_string();
        return Err(format!(
            "gallery-dl exited with status {:?}: {}",
            status.code(),
            detail
        ));
    }

    Ok(unavailable)
}

/// Detects gallery-dl stderr markers that indicate the VSCO profile (or the
/// requested section) does not exist or is otherwise unavailable/private.
fn output_is_profile_unavailable(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    lowered.contains("http error 404")
        || lowered.contains("404: not found")
        || lowered.contains("does not exist")
        || lowered.contains("not found")
        || lowered.contains("no results")
        || lowered.contains("unable to fetch")
}

/// Parses every JSON page in `pages_dir`. Returns the media items plus the
/// resolved avatar URL and numeric user id, when present.
fn parse_media_from_pages(
    pages_dir: &Path,
    summary: &mut VscoManifestSummary,
) -> Result<(Vec<ParsedVscoMedia>, Option<String>, Option<String>), String> {
    let mut media: Vec<ParsedVscoMedia> = Vec::new();
    let mut avatar_url: Option<String> = None;
    let mut user_id: Option<String> = None;
    let entries = fs::read_dir(pages_dir).map_err(|error| error.to_string())?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        summary.parsed_page_count += 1;
        collect_media(&value, &mut media);
        if avatar_url.is_none() {
            avatar_url = find_avatar_url(&value);
        }
        if user_id.is_none() {
            user_id = find_user_id(&value);
        }
    }
    Ok((media, avatar_url, user_id))
}

/// Recursive traversal resilient to VSCO's API shape: any object carrying an
/// `_id` and a media URL (`responsive_url`/`video_url`) is treated as a media
/// item, regardless of how the surrounding `media`/`image` wrapper is nested.
fn collect_media(value: &Value, media: &mut Vec<ParsedVscoMedia>) {
    match value {
        Value::Object(map) => {
            if let Some(item) = extract_media_from_object(map) {
                media.push(item);
            }
            for child in map.values() {
                collect_media(child, media);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_media(child, media);
            }
        }
        _ => {}
    }
}

fn value_to_id_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn extract_media_from_object(map: &serde_json::Map<String, Value>) -> Option<ParsedVscoMedia> {
    let media_id = map.get("_id").and_then(value_to_id_string)?;
    let is_video = map.get("is_video").and_then(Value::as_bool).unwrap_or(false);

    let raw_url = if is_video {
        map.get("video_url")
            .and_then(Value::as_str)
            .or_else(|| map.get("responsive_url").and_then(Value::as_str))
    } else {
        map.get("responsive_url").and_then(Value::as_str)
    }?;
    let file_url = normalize_media_url(raw_url);
    let file_name = url_file_name(&file_url)?;

    // Upload/capture dates are epoch milliseconds.
    let captured_at_timestamp = map
        .get("upload_date")
        .and_then(Value::as_i64)
        .or_else(|| map.get("capture_date").and_then(Value::as_i64))
        .map(|value| value / 1000)
        .filter(|value| *value > 0);

    Some(ParsedVscoMedia {
        media_id,
        media_type: if is_video {
            "video".to_string()
        } else {
            "image".to_string()
        },
        file_url,
        file_name,
        captured_at_timestamp,
    })
}

/// VSCO URLs come without a scheme (e.g. `im.vsco.co/aws/.../file.jpg`).
fn normalize_media_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    }
}

/// Best-effort avatar lookup: VSCO profile objects expose the avatar under
/// `profile_image` / `responsive_url` with a `_id`-less shape.
fn find_avatar_url(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            if let Some(url) = map
                .get("profile_image")
                .and_then(Value::as_str)
                .or_else(|| map.get("profile_image_url").and_then(Value::as_str))
            {
                let trimmed = url.trim();
                if !trimmed.is_empty() {
                    return Some(normalize_media_url(trimmed));
                }
            }
            map.values().find_map(find_avatar_url)
        }
        Value::Array(items) => items.iter().find_map(find_avatar_url),
        _ => None,
    }
}

/// Resolves the stable numeric user id from a VSCO profile/media object.
fn find_user_id(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            if let Some(id) = map
                .get("site_id")
                .and_then(value_to_id_string)
                .or_else(|| map.get("grid_id").and_then(value_to_id_string))
            {
                return Some(id);
            }
            map.values().find_map(find_user_id)
        }
        Value::Array(items) => items.iter().find_map(find_user_id),
        _ => None,
    }
}

fn url_file_name(url: &str) -> Option<String> {
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    let name = without_query.rsplit('/').next()?.trim();
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

/// Stable disk identity for VSCO file names. Ignores the NinjaCrawler date
/// prefix, casing and extension so previous layouts match current downloads.
pub fn vsco_disk_asset_key(file_name: &str) -> Option<String> {
    let stem = file_name
        .rsplit_once('.')
        .map(|(value, _)| value)
        .unwrap_or(file_name)
        .trim();
    let without_date = if stem.len() > 20
        && stem.as_bytes().get(4) == Some(&b'-')
        && stem.as_bytes().get(7) == Some(&b'-')
        && stem.as_bytes().get(10) == Some(&b' ')
        && stem.as_bytes().get(13) == Some(&b'.')
        && stem.as_bytes().get(16) == Some(&b'.')
        && stem.as_bytes().get(19) == Some(&b' ')
    {
        &stem[20..]
    } else {
        stem
    };
    let normalized = without_date.trim().to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

fn asset_exists_on_disk(request: &VscoConnectorRequest, media: &ParsedVscoMedia) -> bool {
    vsco_disk_asset_key(&media.file_name)
        .is_some_and(|key| request.existing_relative_paths.contains(&key))
}

fn build_download_client(request: &VscoConnectorRequest) -> Result<Client, String> {
    let mut builder = Client::builder().timeout(Duration::from_secs(DEFAULT_DOWNLOAD_TIMEOUT_SECS));
    if let Some(user_agent) = request
        .user_agent
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        builder = builder.user_agent(user_agent.to_string());
    } else {
        builder = builder.user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
        );
    }
    builder.build().map_err(|error| error.to_string())
}

fn download_asset(
    client: &Client,
    request: &VscoConnectorRequest,
    entry: &DownloadPlanEntry,
) -> Result<DownloadedVscoMedia, String> {
    let final_file_name =
        timestamped_file_name(entry.media.captured_at_timestamp, &entry.media.file_name);

    let target_dir = if entry.media.media_type == "video" && request.separate_video_folder {
        request.profile_root.join("Video")
    } else {
        request.profile_root.clone()
    };
    let destination = target_dir.join(&final_file_name);

    connector_debug::append_current(
        "vsco-http",
        "call",
        "GET media",
        format!("GET {}", entry.media.file_url),
    );
    let response = client.get(&entry.media.file_url).send().map_err(|error| {
        connector_debug::append_current("vsco-http", "error", "GET media", error.to_string());
        error.to_string()
    })?;
    connector_debug::append_current(
        "vsco-http",
        "response",
        "GET media",
        format!("HTTP {}", response.status()),
    );
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    let bytes = response.bytes().map_err(|error| error.to_string())?;
    if bytes.is_empty() {
        return Err("empty response body".to_string());
    }

    write_download_atomically(&destination, &bytes)?;

    Ok(DownloadedVscoMedia {
        file_path: destination,
        media_type: entry.media.media_type.clone(),
        media_section: entry.media_section.clone(),
        provider_media_key: entry.media.media_id.clone(),
        provider_post_key: entry.media.media_id.clone(),
        captured_at_timestamp: entry.media.captured_at_timestamp,
        final_file_name,
    })
}

fn write_download_atomically(destination: &Path, bytes: &[u8]) -> Result<(), String> {
    atomic_file::write_bytes_replacing_empty(destination, bytes)
}

/// Prefixes the file name with the media's local date/time (`YYYY-MM-DD
/// HH.MM.SS `), like the other providers, for chronological ordering on disk.
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

fn file_sha256(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read =
            std::io::Read::read(&mut file, &mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn seed_existing_hashes(profile_root: &Path) -> HashMap<String, PathBuf> {
    let mut hashes = HashMap::new();
    let mut pending = vec![profile_root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if let Ok(hash) = file_sha256(&path) {
                hashes.entry(hash).or_insert(path);
            }
        }
    }
    hashes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gallery_page_json() -> Value {
        // Mirrors the VSCO `medias/profile` API response shape: a `media` array
        // whose items wrap the media object, plus a profile block with the id.
        serde_json::json!({
            "site": {
                "id": 987654,
                "name": "testuser",
                "profile_image": "im.vsco.co/aws-us-west-2/avatars/1/avatar.jpg"
            },
            "media": [
                {
                    "type": "image",
                    "image": {
                        "_id": "5f0000000000000000000001",
                        "site_id": 987654,
                        "grid_name": "testuser",
                        "is_video": false,
                        "upload_date": 1539202764000_i64,
                        "capture_date": 1539202700000_i64,
                        "responsive_url": "im.vsco.co/aws-us-west-2/abc/123/5f0000000000000000000001.jpg",
                        "permalink": "https://vsco.co/testuser/media/5f0000000000000000000001"
                    }
                },
                {
                    "type": "video",
                    "video": {
                        "_id": "5f0000000000000000000002",
                        "site_id": 987654,
                        "grid_name": "testuser",
                        "is_video": true,
                        "upload_date": 1600000000000_i64,
                        "responsive_url": "im.vsco.co/aws-us-west-2/abc/123/poster.jpg",
                        "video_url": "im.vsco.co/aws-us-west-2/abc/123/5f0000000000000000000002.mp4",
                        "permalink": "https://vsco.co/testuser/media/5f0000000000000000000002"
                    }
                }
            ]
        })
    }

    #[test]
    fn collect_media_extracts_image_and_video_from_api_page() {
        let mut media = Vec::new();
        collect_media(&gallery_page_json(), &mut media);

        assert_eq!(media.len(), 2);

        let image = &media[0];
        assert_eq!(image.media_id, "5f0000000000000000000001");
        assert_eq!(image.media_type, "image");
        assert_eq!(image.file_name, "5f0000000000000000000001.jpg");
        assert_eq!(
            image.file_url,
            "https://im.vsco.co/aws-us-west-2/abc/123/5f0000000000000000000001.jpg"
        );
        // upload_date epoch ms -> seconds.
        assert_eq!(image.captured_at_timestamp, Some(1539202764));

        // The video prefers `video_url` over the poster `responsive_url`.
        let video = &media[1];
        assert_eq!(video.media_id, "5f0000000000000000000002");
        assert_eq!(video.media_type, "video");
        assert_eq!(video.file_name, "5f0000000000000000000002.mp4");
        assert_eq!(
            video.file_url,
            "https://im.vsco.co/aws-us-west-2/abc/123/5f0000000000000000000002.mp4"
        );
    }

    #[test]
    fn numeric_media_id_is_stringified() {
        let object = serde_json::json!({
            "_id": 1234567890_i64,
            "is_video": false,
            "responsive_url": "im.vsco.co/x/1234567890.jpg"
        });
        let media = extract_media_from_object(object.as_object().unwrap()).expect("media");
        assert_eq!(media.media_id, "1234567890");
        assert_eq!(media.file_name, "1234567890.jpg");
    }

    #[test]
    fn find_user_id_and_avatar_from_profile_block() {
        let page = gallery_page_json();
        assert_eq!(find_user_id(&page).as_deref(), Some("987654"));
        assert_eq!(
            find_avatar_url(&page).as_deref(),
            Some("https://im.vsco.co/aws-us-west-2/avatars/1/avatar.jpg")
        );
    }

    #[test]
    fn normalize_media_url_adds_scheme_when_missing() {
        assert_eq!(
            normalize_media_url("im.vsco.co/x/file.jpg"),
            "https://im.vsco.co/x/file.jpg"
        );
        assert_eq!(
            normalize_media_url("https://im.vsco.co/x/file.jpg"),
            "https://im.vsco.co/x/file.jpg"
        );
    }

    #[test]
    fn url_file_name_strips_query_and_fragment() {
        assert_eq!(
            url_file_name("https://im.vsco.co/x/file.jpg?w=1200"),
            Some("file.jpg".to_string())
        );
        assert_eq!(url_file_name("https://vsco.co/user/gallery/"), None);
    }

    #[test]
    fn timestamped_file_name_prepends_local_datetime() {
        let named = timestamped_file_name(Some(1539202764), "abc.jpg");
        assert!(named.ends_with(" abc.jpg"));
        let prefix = &named[..named.len() - " abc.jpg".len()];
        assert_eq!(prefix.len(), 19);
        assert_eq!(&prefix[4..5], "-");
        assert_eq!(&prefix[13..14], ".");
    }

    #[test]
    fn timestamped_file_name_keeps_raw_name_without_timestamp() {
        assert_eq!(timestamped_file_name(None, "abc.mp4"), "abc.mp4");
    }

    #[test]
    fn vsco_disk_asset_key_matches_current_and_dated_names() {
        assert_eq!(
            vsco_disk_asset_key("5f0000000000000000000001.jpg").as_deref(),
            Some("5f0000000000000000000001")
        );
        assert_eq!(
            vsco_disk_asset_key("2018-10-10 20.19.24 5F0000000000000000000001.JPG").as_deref(),
            Some("5f0000000000000000000001")
        );
    }

    #[test]
    fn profile_unavailable_detection_matches_markers() {
        assert!(output_is_profile_unavailable(
            "[vsco][error] HTTP Error 404: Not Found"
        ));
        assert!(output_is_profile_unavailable(
            "ERROR: Unable to fetch data for this user"
        ));
        assert!(!output_is_profile_unavailable("[vsco][info] Downloaded 10"));
    }

    #[test]
    fn completed_posts_only_count_available_media() {
        let pending = vec![
            PendingVscoPost {
                provider_post_key: "known".to_string(),
                media_section: "gallery".to_string(),
                was_known_post: true,
                had_missing_asset: false,
            },
            PendingVscoPost {
                provider_post_key: "missing".to_string(),
                media_section: "gallery".to_string(),
                was_known_post: false,
                had_missing_asset: true,
            },
        ];
        let available = HashSet::from(["known".to_string()]);
        let mut summary = VscoManifestSummary::default();

        let completed = completed_observed_posts(pending, &available, &mut summary);

        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].provider_post_key, "known");
        assert_eq!(summary.skipped_existing_post_count, 1);
    }
}
