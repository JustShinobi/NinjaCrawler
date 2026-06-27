use chrono::{Local, TimeZone};
use reqwest::blocking::Client;
use reqwest::header::HeaderMap;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

/// App id público da web do Instagram, usado em consultas anônimas de identidade.
const INSTAGRAM_PUBLIC_APP_ID: &str = "936619743392459";
const INSTAGRAM_PUBLIC_ASBD_ID: &str = "129477";

#[derive(Clone)]
pub struct SessionCookie {
    pub domain: String,
    pub name: String,
    pub value: String,
}

#[derive(Clone, Default)]
pub struct InstagramAuthHeaders {
    pub csrf_token: Option<String>,
    pub app_id: Option<String>,
    pub asbd_id: Option<String>,
    pub ig_www_claim: Option<String>,
    pub lsd: Option<String>,
    pub dtsg: Option<String>,
    pub sec_ch_ua: Option<String>,
    pub sec_ch_ua_full_version_list: Option<String>,
    pub sec_ch_ua_platform_version: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Clone, Copy, Default)]
pub struct InstagramSectionSelection {
    pub timeline: bool,
    pub reels: bool,
    pub stories: bool,
    pub stories_user: bool,
    pub tagged: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InstagramMediaFileNamingMode {
    PresetNewDefault,
    PresetLegacyUrlBasename,
    Custom,
}

impl InstagramMediaFileNamingMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PresetNewDefault => "preset_new_default",
            Self::PresetLegacyUrlBasename => "preset_legacy_url_basename",
            Self::Custom => "custom",
        }
    }
}

#[derive(Clone)]
pub struct InstagramConnectorRequest {
    pub username: String,
    pub cookies: Vec<SessionCookie>,
    pub headers: InstagramAuthHeaders,
    pub profile_root: PathBuf,
    pub saved_posts_root: PathBuf,
    pub ledger_post_keys: HashSet<String>,
    pub existing_media_keys: HashSet<String>,
    pub ledger_media_keys: HashSet<String>,
    pub existing_relative_paths: HashSet<String>,
    pub ledger_relative_paths: HashSet<String>,
    pub sections: InstagramSectionSelection,
    pub use_gql: bool,
    pub download_saved_posts: bool,
    pub post_page_size: u32,
    pub skip_errors: bool,
    pub ignore_stories_560_errors: bool,
    pub request_delay_ms: u64,
    pub timeout_secs: u64,
    pub download_images: bool,
    pub download_videos: bool,
    pub extract_image_from_video: InstagramSectionSelection,
    pub place_extracted_image_into_video_folder: bool,
    pub download_text: bool,
    pub download_text_posts: bool,
    pub text_special_folder: bool,
    pub get_user_media_only: bool,
    pub missing_only: bool,
    pub date_from_timestamp: Option<i64>,
    pub date_to_timestamp: Option<i64>,
    pub media_file_naming_mode: InstagramMediaFileNamingMode,
    pub media_file_naming_template: Option<String>,
    pub target_story_media_id: Option<String>,
}

#[derive(Clone)]
pub struct DownloadedInstagramMedia {
    pub file_path: PathBuf,
    pub media_type: String,
    pub media_section: String,
    pub provider_media_key: String,
    /// Post shortcode preserving original casing (Instagram shortcodes are
    /// case-sensitive), used to rebuild the `instagram.com/p/<code>/` link.
    pub provider_post_code: Option<String>,
    pub captured_at_timestamp: Option<i64>,
    pub final_file_name: String,
    pub legacy_raw_file_name: Option<String>,
    pub extension: String,
    pub pattern_mode: String,
    pub pattern_template: Option<String>,
}

#[derive(Clone)]
pub struct ObservedInstagramPost {
    pub provider_post_key: String,
    pub provider_post_code: Option<String>,
    pub media_section: String,
}

pub struct InstagramConnectorResult {
    pub observed_posts: Vec<ObservedInstagramPost>,
    pub downloaded_media: Vec<DownloadedInstagramMedia>,
    pub section_errors: Vec<String>,
    pub validation_error: Option<String>,
    pub auth_disabled_sections: Vec<String>,
    pub resolved_username: Option<String>,
    pub profile_description: Option<String>,
    pub manifest_summary: Option<InstagramManifestSummary>,
    pub updated_headers: InstagramAuthHeaders,
    pub rate_limited: bool,
}

#[derive(Clone)]
pub struct InstagramProfileIdentity {
    pub username: String,
    pub user_id: String,
}

pub struct InstagramProgress {
    pub label: String,
    pub detail: String,
    pub downloaded_items: Option<u32>,
    pub progress_percent: Option<u32>,
    pub indeterminate: bool,
}

struct InstagramClient {
    client: Client,
    cookie_header: String,
    headers: InstagramAuthHeaders,
    header_mode: InstagramHeaderMode,
    request_delay: Duration,
    last_request_at: Option<Instant>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InstagramHeaderMode {
    BrowserLike,
    Relaxed,
}

#[derive(Clone)]
struct UserProfile {
    username: String,
    user_id: String,
    description: Option<String>,
    timeline_items: Vec<Value>,
    timeline_next_max_id: Option<String>,
    reel_items: Vec<Value>,
    tagged_items: Vec<Value>,
    highlight_tray: Vec<HighlightTrayItem>,
}

#[derive(Clone)]
struct HighlightTrayItem {
    id: String,
    title: String,
}

#[derive(Clone)]
struct MediaAsset {
    file_url: String,
    media_type: String,
    extracted_from_video: bool,
    file_name: String,
    provider_media_key: String,
    /// Owning post shortcode (original casing) for link reconstruction.
    provider_post_code: Option<String>,
    captured_at_timestamp: Option<i64>,
    legacy_raw_file_name: Option<String>,
    extension: String,
}

#[derive(Clone)]
struct PlannedMediaAsset {
    asset: MediaAsset,
    destination_path: PathBuf,
}

#[derive(Clone)]
struct InstagramManifestPost {
    item: Value,
    provider_post_key: String,
    provider_post_code: Option<String>,
    planned_assets: Vec<PlannedMediaAsset>,
    write_text_sidecar: bool,
}

#[derive(Clone)]
struct InstagramManifestSection {
    media_section: String,
    display_label: String,
    section_root: PathBuf,
    items: Vec<Value>,
    profile_user_id: Option<String>,
    discovered_asset_count: usize,
    normalized_post_count: usize,
    skipped_existing_post_count: usize,
    skipped_duplicate_post_count: usize,
    skipped_unavailable_post_count: usize,
    skipped_existing_asset_count: usize,
    skipped_duplicate_asset_count: usize,
    posts: Vec<InstagramManifestPost>,
}

#[derive(Clone, Default)]
struct InstagramSyncManifest {
    sections: Vec<InstagramManifestSection>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstagramManifestSectionSummary {
    pub section: String,
    pub label: String,
    pub item_count: u32,
    pub normalized_post_count: u32,
    pub discovered_asset_count: u32,
    pub queued_asset_count: u32,
    pub skipped_existing_post_count: u32,
    pub skipped_duplicate_post_count: u32,
    pub skipped_unavailable_post_count: u32,
    pub skipped_existing_asset_count: u32,
    pub skipped_duplicate_asset_count: u32,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstagramManifestSummary {
    pub profile_user_id: Option<String>,
    pub section_count: u32,
    pub discovered_item_count: u32,
    pub normalized_post_count: u32,
    pub discovered_asset_count: u32,
    pub queued_asset_count: u32,
    pub skipped_existing_post_count: u32,
    pub skipped_duplicate_post_count: u32,
    pub skipped_unavailable_post_count: u32,
    pub skipped_existing_asset_count: u32,
    pub skipped_duplicate_asset_count: u32,
    pub downloaded_asset_count: u32,
    pub sections: Vec<InstagramManifestSectionSummary>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SectionErrorDisposition {
    Generic,
    AlwaysWarn,
    ForceFail,
    AuthInvalid,
}

impl InstagramClient {
    fn new(
        cookies: &[SessionCookie],
        headers: InstagramAuthHeaders,
        timeout_secs: u64,
        request_delay_ms: u64,
    ) -> Result<Self, String> {
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs.max(10)))
            .build()
            .map_err(|error| error.to_string())?;

        Ok(Self {
            client,
            cookie_header: build_cookie_header(cookies),
            headers,
            header_mode: InstagramHeaderMode::BrowserLike,
            request_delay: Duration::from_millis(request_delay_ms),
            last_request_at: None,
        })
    }

    fn get_json(&mut self, url: &str, referer: Option<&str>) -> Result<Value, String> {
        self.get_json_with_extra_headers(url, referer, &[])
    }

    fn get_json_with_extra_headers(
        &mut self,
        url: &str,
        referer: Option<&str>,
        extra_headers: &[(&str, String)],
    ) -> Result<Value, String> {
        let (mut status, mut body) =
            self.send_text_request(url, referer, self.header_mode, extra_headers)?;

        if status == reqwest::StatusCode::BAD_REQUEST
            && body.to_ascii_lowercase().contains("useragent mismatch")
            && self.header_mode != InstagramHeaderMode::Relaxed
        {
            self.header_mode = InstagramHeaderMode::Relaxed;
            let (retry_status, retry_body) =
                self.send_text_request(url, referer, self.header_mode, extra_headers)?;
            status = retry_status;
            body = retry_body;
        }

        if !status.is_success() {
            return Err(format!(
                "Instagram request '{url}' returned {}: {}",
                status,
                truncate_for_error(&body)
            ));
        }

        serde_json::from_str(&body)
            .map_err(|error| format!("Instagram JSON decode failed for '{url}': {error}"))
    }

    fn download_file(
        &mut self,
        url: &str,
        path: &Path,
        referer: Option<&str>,
    ) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }

        self.wait_for_pacing();
        let mut request = self.client.get(url);
        request = self.apply_headers(request, referer, self.header_mode);

        let response = request
            .send()
            .map_err(|error| format!("Instagram media download failed for '{url}': {error}"))?;
        self.absorb_response_headers(response.headers());
        let status = response.status();
        let bytes = response
            .bytes()
            .map_err(|error| format!("Instagram media payload read failed for '{url}': {error}"))?;
        self.last_request_at = Some(Instant::now());

        if !status.is_success() {
            return Err(format!("Instagram media request '{url}' returned {status}"));
        }

        fs::write(path, &bytes).map_err(|error| error.to_string())
    }

    fn apply_headers(
        &self,
        mut request: reqwest::blocking::RequestBuilder,
        referer: Option<&str>,
        header_mode: InstagramHeaderMode,
    ) -> reqwest::blocking::RequestBuilder {
        if !self.cookie_header.is_empty() {
            request = request.header("cookie", self.cookie_header.clone());
        }

        request = request.header(
            "user-agent",
            self.headers.user_agent.clone().unwrap_or_else(|| {
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36".to_string()
            }),
        );
        request = request.header("accept", "*/*");
        request = request.header("accept-language", "en-US,en;q=0.9");

        if header_mode == InstagramHeaderMode::BrowserLike {
            request = request.header("origin", "https://www.instagram.com");
            request = request.header("dnt", "1");
            request = request.header("sec-ch-ua-mobile", "?0");
            request = request.header("sec-ch-ua-model", "\"\"");
            request = request.header("sec-ch-ua-platform", "\"Windows\"");
            request = request.header("sec-fetch-dest", "empty");
            request = request.header("sec-fetch-mode", "cors");
            request = request.header("sec-fetch-site", "same-origin");
            request = request.header("x-requested-with", "XMLHttpRequest");
        }

        if let Some(value) = self
            .headers
            .app_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            request = request.header("x-ig-app-id", value);
        }
        if let Some(value) = self
            .headers
            .asbd_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            request = request.header("x-asbd-id", value);
        }
        let ig_www_claim = self
            .headers
            .ig_www_claim
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("0");
        request = request.header("x-ig-www-claim", ig_www_claim);
        if let Some(value) = self
            .headers
            .csrf_token
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            request = request.header("x-csrftoken", value);
        }
        if header_mode == InstagramHeaderMode::BrowserLike {
            if let Some(value) = self
                .headers
                .sec_ch_ua
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                request = request.header("sec-ch-ua", value);
            }
            if let Some(value) = self
                .headers
                .sec_ch_ua_full_version_list
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                request = request.header("sec-ch-ua-full-version-list", value);
            }
            if let Some(value) = self
                .headers
                .sec_ch_ua_platform_version
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                request = request.header("sec-ch-ua-platform-version", value);
            }
        }
        if let Some(value) = referer {
            request = request.header("referer", value);
        }

        request
    }

    fn send_text_request(
        &mut self,
        url: &str,
        referer: Option<&str>,
        header_mode: InstagramHeaderMode,
        extra_headers: &[(&str, String)],
    ) -> Result<(reqwest::StatusCode, String), String> {
        self.wait_for_pacing();

        let mut request = self.client.get(url);
        request = self.apply_headers(request, referer, header_mode);
        for (name, value) in extra_headers {
            request = request.header(*name, value);
        }

        let response = request
            .send()
            .map_err(|error| format!("Instagram request failed for '{url}': {error}"))?;
        self.absorb_response_headers(response.headers());
        let status = response.status();
        let body = response
            .text()
            .map_err(|error| format!("Instagram response read failed for '{url}': {error}"))?;
        self.last_request_at = Some(Instant::now());
        Ok((status, body))
    }

    fn absorb_response_headers(&mut self, headers: &HeaderMap) {
        if let Some(value) = header_text(headers, "x-ig-www-claim") {
            self.headers.ig_www_claim = Some(value);
        }
        if let Some(value) = header_text(headers, "x-csrftoken") {
            self.headers.csrf_token = Some(value);
        }
    }

    fn wait_for_pacing(&self) {
        if self.request_delay.is_zero() {
            return;
        }

        if let Some(last_request_at) = self.last_request_at {
            let elapsed = last_request_at.elapsed();
            if elapsed < self.request_delay {
                thread::sleep(self.request_delay - elapsed);
            }
        }
    }
}

pub fn run_profile_sync<F, C>(
    request: &InstagramConnectorRequest,
    mut progress: F,
    should_cancel: C,
) -> Result<InstagramConnectorResult, String>
where
    F: FnMut(InstagramProgress),
    C: Fn() -> bool,
{
    ensure_sync_not_cancelled(&should_cancel)?;
    let mut client = InstagramClient::new(
        &request.cookies,
        request.headers.clone(),
        request.timeout_secs,
        request.request_delay_ms,
    )?;
    let profile = load_profile(&mut client, &request.username)?;
    let profile_description =
        load_profile_description_by_user_id(&mut client, &profile.username, &profile.user_id)
            .or_else(|| profile.description.clone())
            .or_else(|| {
                load_profile_description_gql(&mut client, &profile.username, &profile.user_id)
            })
            .or_else(|| load_profile_description(&mut client, &profile.username));
    let mut effective_request = request.clone();
    effective_request.username = profile.username.clone();
    let mut section_errors = Vec::new();
    let mut validation_error = None;
    let mut auth_disabled_sections = Vec::new();
    let mut rate_limited = false;
    let mut manifest = InstagramSyncManifest::default();
    let total_sections = enabled_section_count(&effective_request.sections);
    let mut completed_discovery_sections = 0usize;

    if let Some(target_story_media_id) = effective_request
        .target_story_media_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    {
        ensure_sync_not_cancelled(&should_cancel)?;
        report_profile_phase_progress(
            &mut progress,
            "Discovering story",
            "Selected story: loading item".to_string(),
            Some(0),
            Some(0),
            true,
        );
        match discover_target_story_manifest_section(
            &mut client,
            &effective_request,
            &profile,
            &target_story_media_id,
            &mut manifest,
        ) {
            Ok(()) => {
                completed_discovery_sections += 1;
                report_profile_phase_progress(
                    &mut progress,
                    "Discovering story",
                    "Selected story: discovery complete".to_string(),
                    Some(0),
                    discovery_progress_percent(completed_discovery_sections, 1),
                    false,
                );
            }
            Err(error) => {
                handle_section_error(
                    error,
                    effective_request.skip_errors,
                    effective_request.ignore_stories_560_errors,
                    effective_request.use_gql,
                    "stories_user",
                    &mut section_errors,
                    &mut validation_error,
                    &mut auth_disabled_sections,
                    &mut rate_limited,
                )?;
            }
        }
    } else if effective_request.sections.timeline {
        ensure_sync_not_cancelled(&should_cancel)?;
        report_profile_phase_progress(
            &mut progress,
            "Discovering posts",
            format!("{}: loading items", section_label("timeline")),
            Some(0),
            discovery_progress_percent(completed_discovery_sections, total_sections),
            true,
        );
        match load_timeline_items(
            &mut client,
            &profile.username,
            &profile,
            effective_request.post_page_size,
            effective_request.use_gql,
        ) {
            Ok(timeline_items) => {
                completed_discovery_sections += 1;
                manifest.sections.push(build_manifest_section(
                    "timeline",
                    section_label("timeline").to_string(),
                    effective_request.profile_root.clone(),
                    timeline_items,
                    Some(&profile.user_id),
                ));
                report_profile_phase_progress(
                    &mut progress,
                    "Discovering posts",
                    format!(
                        "{}: discovered {} items",
                        section_label("timeline"),
                        manifest
                            .sections
                            .last()
                            .map(|section| section.items.len())
                            .unwrap_or_default()
                    ),
                    Some(0),
                    discovery_progress_percent(completed_discovery_sections, total_sections),
                    false,
                );
            }
            Err(error) => {
                handle_section_error(
                    error,
                    effective_request.skip_errors,
                    effective_request.ignore_stories_560_errors,
                    effective_request.use_gql,
                    "timeline",
                    &mut section_errors,
                    &mut validation_error,
                    &mut auth_disabled_sections,
                    &mut rate_limited,
                )?;
            }
        }
    }

    if effective_request.target_story_media_id.is_none() && effective_request.sections.reels {
        ensure_sync_not_cancelled(&should_cancel)?;
        report_profile_phase_progress(
            &mut progress,
            "Discovering posts",
            format!("{}: loading items", section_label("reels")),
            Some(0),
            discovery_progress_percent(completed_discovery_sections, total_sections),
            true,
        );
        match load_reel_items(
            &mut client,
            &profile,
            effective_request.post_page_size,
            effective_request.use_gql,
        ) {
            Ok(reel_items) => {
                completed_discovery_sections += 1;
                manifest.sections.push(build_manifest_section(
                    "reels",
                    section_label("reels").to_string(),
                    effective_request.profile_root.clone(),
                    reel_items,
                    Some(&profile.user_id),
                ));
                report_profile_phase_progress(
                    &mut progress,
                    "Discovering posts",
                    format!(
                        "{}: discovered {} items",
                        section_label("reels"),
                        manifest
                            .sections
                            .last()
                            .map(|section| section.items.len())
                            .unwrap_or_default()
                    ),
                    Some(0),
                    discovery_progress_percent(completed_discovery_sections, total_sections),
                    false,
                );
            }
            Err(error) => {
                handle_section_error(
                    error,
                    effective_request.skip_errors,
                    effective_request.ignore_stories_560_errors,
                    effective_request.use_gql,
                    "reels",
                    &mut section_errors,
                    &mut validation_error,
                    &mut auth_disabled_sections,
                    &mut rate_limited,
                )?;
            }
        }
    }

    if effective_request.target_story_media_id.is_none() && effective_request.sections.stories {
        ensure_sync_not_cancelled(&should_cancel)?;
        report_profile_phase_progress(
            &mut progress,
            "Discovering posts",
            format!("{}: loading items", section_label("stories")),
            Some(0),
            discovery_progress_percent(completed_discovery_sections, total_sections),
            true,
        );
        if let Err(error) = discover_highlights_manifest_sections(
            &mut client,
            &effective_request,
            &profile,
            &mut manifest,
        ) {
            handle_section_error(
                error,
                effective_request.skip_errors,
                effective_request.ignore_stories_560_errors,
                effective_request.use_gql,
                "stories",
                &mut section_errors,
                &mut validation_error,
                &mut auth_disabled_sections,
                &mut rate_limited,
            )?;
        } else {
            completed_discovery_sections += 1;
            report_profile_phase_progress(
                &mut progress,
                "Discovering posts",
                format!("{}: discovery complete", section_label("stories")),
                Some(0),
                discovery_progress_percent(completed_discovery_sections, total_sections),
                false,
            );
        }
    }

    if effective_request.target_story_media_id.is_none() && effective_request.sections.stories_user
    {
        ensure_sync_not_cancelled(&should_cancel)?;
        report_profile_phase_progress(
            &mut progress,
            "Discovering posts",
            format!("{}: loading items", section_label("stories_user")),
            Some(0),
            discovery_progress_percent(completed_discovery_sections, total_sections),
            true,
        );
        if let Err(error) = discover_user_stories_manifest_section(
            &mut client,
            &effective_request,
            &profile,
            &mut manifest,
        ) {
            handle_section_error(
                error,
                effective_request.skip_errors,
                effective_request.ignore_stories_560_errors,
                effective_request.use_gql,
                "stories_user",
                &mut section_errors,
                &mut validation_error,
                &mut auth_disabled_sections,
                &mut rate_limited,
            )?;
        } else {
            completed_discovery_sections += 1;
            report_profile_phase_progress(
                &mut progress,
                "Discovering posts",
                format!("{}: discovery complete", section_label("stories_user")),
                Some(0),
                discovery_progress_percent(completed_discovery_sections, total_sections),
                false,
            );
        }
    }

    if effective_request.target_story_media_id.is_none() && effective_request.sections.tagged {
        ensure_sync_not_cancelled(&should_cancel)?;
        report_profile_phase_progress(
            &mut progress,
            "Discovering posts",
            format!("{}: loading items", section_label("tagged")),
            Some(0),
            discovery_progress_percent(completed_discovery_sections, total_sections),
            true,
        );
        match load_tagged_items(
            &mut client,
            &profile,
            effective_request.post_page_size,
            effective_request.use_gql,
        ) {
            Ok(tagged_items) => {
                completed_discovery_sections += 1;
                manifest.sections.push(build_manifest_section(
                    "tagged",
                    section_label("tagged").to_string(),
                    effective_request.profile_root.join("Tagged"),
                    tagged_items,
                    Some(&profile.user_id),
                ));
                report_profile_phase_progress(
                    &mut progress,
                    "Discovering posts",
                    format!(
                        "{}: discovered {} items",
                        section_label("tagged"),
                        manifest
                            .sections
                            .last()
                            .map(|section| section.items.len())
                            .unwrap_or_default()
                    ),
                    Some(0),
                    discovery_progress_percent(completed_discovery_sections, total_sections),
                    false,
                );
            }
            Err(error) => {
                handle_section_error(
                    error,
                    effective_request.skip_errors,
                    effective_request.ignore_stories_560_errors,
                    effective_request.use_gql,
                    "tagged",
                    &mut section_errors,
                    &mut validation_error,
                    &mut auth_disabled_sections,
                    &mut rate_limited,
                )?;
            }
        }
    }

    report_profile_phase_progress(
        &mut progress,
        "Preparing manifest",
        format!("Discovered {} section(s)", manifest.sections.len()),
        Some(0),
        Some(0),
        true,
    );
    normalize_profile_sync_manifest(
        &effective_request,
        &mut manifest,
        &mut progress,
        &should_cancel,
    )?;

    let mut downloaded_media = Vec::new();
    for section in &manifest.sections {
        ensure_sync_not_cancelled(&should_cancel)?;
        if let Err(error) = execute_manifest_section(
            &mut client,
            &effective_request,
            section,
            manifest_total_queued_assets(&manifest),
            &mut downloaded_media,
            &mut progress,
            &should_cancel,
        ) {
            handle_section_error(
                error,
                effective_request.skip_errors,
                effective_request.ignore_stories_560_errors,
                effective_request.use_gql,
                &section.media_section,
                &mut section_errors,
                &mut validation_error,
                &mut auth_disabled_sections,
                &mut rate_limited,
            )?;
        }
    }

    if !effective_request.skip_errors && downloaded_media.is_empty() && !section_errors.is_empty() {
        return Err(section_errors.join(" | "));
    }

    let downloaded_asset_count = downloaded_media.len() as u32;

    Ok(InstagramConnectorResult {
        observed_posts: manifest_observed_posts(&manifest),
        downloaded_media,
        section_errors,
        validation_error,
        auth_disabled_sections,
        resolved_username: Some(profile.username.clone()),
        profile_description,
        manifest_summary: Some(summarize_profile_sync_manifest(
            &manifest,
            downloaded_asset_count,
            Some(&profile.user_id),
        )),
        updated_headers: client.headers.clone(),
        rate_limited,
    })
}

pub fn run_saved_posts_sync<F, C>(
    request: &InstagramConnectorRequest,
    mut progress: F,
    should_cancel: C,
) -> Result<InstagramConnectorResult, String>
where
    F: FnMut(InstagramProgress),
    C: Fn() -> bool,
{
    if !request.download_saved_posts {
        return Ok(InstagramConnectorResult {
            observed_posts: Vec::new(),
            downloaded_media: Vec::new(),
            section_errors: Vec::new(),
            validation_error: None,
            auth_disabled_sections: Vec::new(),
            resolved_username: None,
            profile_description: None,
            manifest_summary: None,
            updated_headers: request.headers.clone(),
            rate_limited: false,
        });
    }

    let mut client = InstagramClient::new(
        &request.cookies,
        request.headers.clone(),
        request.timeout_secs,
        request.request_delay_ms,
    )?;
    let items = load_saved_posts_items(&mut client, request.post_page_size)?;
    let mut downloaded_media = Vec::new();
    let mut section_errors = Vec::new();
    let mut validation_error = None;
    let mut auth_disabled_sections = Vec::new();
    let mut rate_limited = false;

    ensure_sync_not_cancelled(&should_cancel)?;
    if let Err(error) = download_items_section(
        &mut client,
        request,
        "saved_posts",
        &request.saved_posts_root,
        items,
        None,
        &mut downloaded_media,
        &mut progress,
    ) {
        handle_section_error(
            error,
            request.skip_errors,
            request.ignore_stories_560_errors,
            request.use_gql,
            "saved_posts",
            &mut section_errors,
            &mut validation_error,
            &mut auth_disabled_sections,
            &mut rate_limited,
        )?;
    }

    if !request.skip_errors && downloaded_media.is_empty() && !section_errors.is_empty() {
        return Err(section_errors.join(" | "));
    }

    Ok(InstagramConnectorResult {
        observed_posts: Vec::new(),
        downloaded_media,
        section_errors,
        validation_error,
        auth_disabled_sections,
        resolved_username: None,
        profile_description: None,
        manifest_summary: None,
        updated_headers: client.headers.clone(),
        rate_limited,
    })
}

fn build_manifest_section(
    media_section: &str,
    display_label: String,
    section_root: PathBuf,
    items: Vec<Value>,
    profile_user_id: Option<&str>,
) -> InstagramManifestSection {
    InstagramManifestSection {
        media_section: media_section.to_string(),
        display_label,
        section_root,
        items,
        profile_user_id: profile_user_id.map(str::to_string),
        discovered_asset_count: 0,
        normalized_post_count: 0,
        skipped_existing_post_count: 0,
        skipped_duplicate_post_count: 0,
        skipped_unavailable_post_count: 0,
        skipped_existing_asset_count: 0,
        skipped_duplicate_asset_count: 0,
        posts: Vec::new(),
    }
}

fn enabled_section_count(selection: &InstagramSectionSelection) -> usize {
    [
        selection.timeline,
        selection.reels,
        selection.stories,
        selection.stories_user,
        selection.tagged,
    ]
    .into_iter()
    .filter(|enabled| *enabled)
    .count()
}

fn discovery_progress_percent(completed_sections: usize, total_sections: usize) -> Option<u32> {
    if total_sections == 0 {
        None
    } else {
        Some(((completed_sections * 100) / total_sections) as u32)
    }
}

fn report_profile_phase_progress<F>(
    progress: &mut F,
    label: &str,
    detail: String,
    downloaded_items: Option<u32>,
    progress_percent: Option<u32>,
    indeterminate: bool,
) where
    F: FnMut(InstagramProgress),
{
    progress(InstagramProgress {
        label: label.to_string(),
        detail,
        downloaded_items,
        progress_percent,
        indeterminate,
    });
}

struct InstagramPostIdentity {
    provider_post_key: String,
    provider_post_code: Option<String>,
}

fn normalize_profile_sync_manifest<F>(
    request: &InstagramConnectorRequest,
    manifest: &mut InstagramSyncManifest,
    progress: &mut F,
    should_cancel: &impl Fn() -> bool,
) -> Result<(), String>
where
    F: FnMut(InstagramProgress),
{
    let total_sections = manifest.sections.len().max(1);
    let mut seen_destination_paths = HashSet::new();
    let mut seen_post_keys = HashSet::new();
    let mut seen_story_post_keys = HashSet::new();

    for (index, section) in manifest.sections.iter_mut().enumerate() {
        ensure_sync_not_cancelled(should_cancel)?;
        report_profile_phase_progress(
            progress,
            "Filtering duplicates",
            format!("{}: resolving normalized posts", section.display_label),
            Some(0),
            Some((index * 100 / total_sections) as u32),
            true,
        );

        let filtered_items = section
            .items
            .iter()
            .filter(|item| item_matches_requested_date_range(item, request))
            .cloned()
            .collect::<Vec<_>>();

        for item in filtered_items {
            ensure_sync_not_cancelled(should_cancel)?;
            if request.get_user_media_only
                && section
                    .profile_user_id
                    .as_deref()
                    .is_some_and(|user_id| !item_belongs_to_user(&item, user_id))
            {
                continue;
            }

            let identity = media_item_post_identity(&item)?;
            let known_in_post_ledger = request
                .ledger_post_keys
                .contains(&identity.provider_post_key)
                || identity
                    .provider_post_code
                    .as_ref()
                    .is_some_and(|code| request.ledger_post_keys.contains(code));

            if !request.missing_only && known_in_post_ledger {
                section.skipped_existing_post_count += 1;
                continue;
            }

            if uses_contextual_story_post_dedupe(section) {
                let scoped_key = format!(
                    "{}::{}",
                    story_post_dedupe_scope(section),
                    identity.provider_post_key
                );
                if !seen_story_post_keys.insert(scoped_key) {
                    section.skipped_duplicate_post_count += 1;
                    continue;
                }
            } else if !seen_post_keys.insert(identity.provider_post_key.clone()) {
                section.skipped_duplicate_post_count += 1;
                continue;
            }

            let assets = collect_media_assets(
                std::slice::from_ref(&item),
                request,
                &section.media_section,
                section.profile_user_id.as_deref(),
            )?;
            let discovered_asset_count = assets.len();
            section.discovered_asset_count += discovered_asset_count;

            let mut planned_assets = Vec::new();
            for asset in assets {
                let base_destination_path =
                    build_destination_base_path(&section.section_root, &asset, request);
                let base_relative_path_key =
                    profile_relative_path_key(&request.profile_root, &base_destination_path);

                let known_in_filesystem = request
                    .existing_media_keys
                    .contains(&asset.provider_media_key)
                    || request
                        .existing_relative_paths
                        .contains(&base_relative_path_key);
                let known_in_ledger = request
                    .ledger_media_keys
                    .contains(&asset.provider_media_key)
                    || request
                        .ledger_relative_paths
                        .contains(&base_relative_path_key);
                let should_skip_existing = if request.missing_only {
                    known_in_filesystem
                } else {
                    known_in_filesystem || known_in_ledger
                };

                if should_skip_existing {
                    section.skipped_existing_asset_count += 1;
                    continue;
                }

                let destination_path = resolve_destination_path(
                    &section.section_root,
                    &asset,
                    request,
                    &seen_destination_paths,
                );
                let destination_key = planned_destination_key(&destination_path);
                let resolved_relative_path_key =
                    profile_relative_path_key(&request.profile_root, &destination_path);

                let resolved_known_in_filesystem = request
                    .existing_relative_paths
                    .contains(&resolved_relative_path_key);
                let resolved_known_in_ledger = request
                    .ledger_relative_paths
                    .contains(&resolved_relative_path_key);
                let should_skip_resolved_destination = if request.missing_only {
                    resolved_known_in_filesystem
                } else {
                    resolved_known_in_filesystem || resolved_known_in_ledger
                };

                if should_skip_resolved_destination {
                    section.skipped_existing_asset_count += 1;
                    continue;
                }

                seen_destination_paths.insert(destination_key);

                planned_assets.push(PlannedMediaAsset {
                    asset,
                    destination_path,
                });
            }

            let write_text_sidecar = should_write_text_sidecar(request, &section.media_section)
                && media_item_text(&item).is_some()
                && (!planned_assets.is_empty() || discovered_asset_count == 0);

            if planned_assets.is_empty() && !write_text_sidecar {
                section.skipped_unavailable_post_count += 1;
                continue;
            }

            section.posts.push(InstagramManifestPost {
                item,
                provider_post_key: identity.provider_post_key,
                provider_post_code: identity.provider_post_code,
                planned_assets,
                write_text_sidecar,
            });
        }

        section.normalized_post_count = section.posts.len();

        report_profile_phase_progress(
            progress,
            "Filtering duplicates",
            format!(
                "{}: retained {} posts and queued {} of {} assets",
                section.display_label,
                section.normalized_post_count,
                section
                    .posts
                    .iter()
                    .map(|post| post.planned_assets.len())
                    .sum::<usize>(),
                section.discovered_asset_count
            ),
            Some(0),
            Some(((index + 1) * 100 / total_sections) as u32),
            false,
        );
    }

    Ok(())
}

fn media_item_post_identity(item: &Value) -> Result<InstagramPostIdentity, String> {
    let provider_post_key = normalize_instagram_post_identity_key(
        &string_from_value(item.get("id"))
            .or_else(|| string_from_value(item.get("pk")))
            .ok_or_else(|| "Instagram media item is missing an identifier.".to_string())?,
    );
    let provider_post_code = string_from_value(item.get("code"))
        .or_else(|| string_from_value(item.get("shortcode")))
        .map(|value| normalize_instagram_post_identity_key(&value))
        .filter(|value| !value.is_empty());

    Ok(InstagramPostIdentity {
        provider_post_key,
        provider_post_code,
    })
}

fn normalize_instagram_post_identity_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn manifest_post_requires_execution(post: &InstagramManifestPost) -> bool {
    post.write_text_sidecar || !post.planned_assets.is_empty()
}

fn manifest_total_queued_assets(manifest: &InstagramSyncManifest) -> usize {
    manifest
        .sections
        .iter()
        .flat_map(|section| section.posts.iter())
        .map(|post| post.planned_assets.len())
        .sum()
}

fn manifest_observed_posts(manifest: &InstagramSyncManifest) -> Vec<ObservedInstagramPost> {
    manifest
        .sections
        .iter()
        .flat_map(|section| {
            section.posts.iter().map(|post| ObservedInstagramPost {
                provider_post_key: post.provider_post_key.clone(),
                provider_post_code: post.provider_post_code.clone(),
                media_section: section.media_section.clone(),
            })
        })
        .collect()
}

pub fn resolve_profile_identity(
    request: &InstagramConnectorRequest,
    user_id_hint: Option<&str>,
) -> Result<InstagramProfileIdentity, String> {
    let mut client = InstagramClient::new(
        &request.cookies,
        request.headers.clone(),
        request.timeout_secs,
        request.request_delay_ms,
    )?;
    let username = request.username.trim();
    let primary_error;

    match load_profile_identity_by_username(&mut client, username) {
        Ok(identity) => return Ok(identity),
        Err(error) => primary_error = error,
    }

    let Some(user_id) = user_id_hint
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Err(primary_error);
    };

    // Resolver `user_id -> username` é uma consulta pública. Sessões importadas
    // degradadas (cookies expirados) fazem o endpoint retornar 400, então
    // tentamos primeiro com um cliente anônimo (sem cookies) e só recorremos à
    // sessão autenticada se a consulta pública falhar. Isso mantém os perfis
    // renomeados resolvíveis mesmo quando a sessão da conta está degradada.
    let mut public_client = InstagramClient::new(
        &[],
        public_identity_headers(&request.headers),
        request.timeout_secs,
        request.request_delay_ms,
    )?;
    match load_profile_identity_by_user_id(&mut public_client, username, user_id) {
        Ok(identity) => Ok(identity),
        Err(public_error) => load_profile_identity_by_user_id(&mut client, username, user_id)
            .map_err(|auth_error| {
                format!(
                    "{primary_error} | fallback by user id '{user_id}' failed \
                     (public lookup: {public_error}; authenticated lookup: {auth_error})"
                )
            }),
    }
}

/// Headers mínimos para uma consulta de identidade anônima: preserva o app id e
/// o user agent da sessão (com defaults públicos) e descarta cookies/CSRF, que
/// são justamente o que quebra o endpoint quando a sessão está degradada.
fn public_identity_headers(source: &InstagramAuthHeaders) -> InstagramAuthHeaders {
    InstagramAuthHeaders {
        app_id: source
            .app_id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| Some(INSTAGRAM_PUBLIC_APP_ID.to_string())),
        asbd_id: source
            .asbd_id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| Some(INSTAGRAM_PUBLIC_ASBD_ID.to_string())),
        user_agent: source.user_agent.clone(),
        ..Default::default()
    }
}

fn load_profile_identity_by_username(
    client: &mut InstagramClient,
    username: &str,
) -> Result<InstagramProfileIdentity, String> {
    let profile = load_profile(client, username)?;
    Ok(InstagramProfileIdentity {
        username: profile.username,
        user_id: profile.user_id,
    })
}

fn load_profile_identity_by_user_id(
    client: &mut InstagramClient,
    current_username: &str,
    user_id: &str,
) -> Result<InstagramProfileIdentity, String> {
    let referer = format!("https://www.instagram.com/{current_username}/");
    let payload = client.get_json(
        &format!("https://i.instagram.com/api/v1/users/{user_id}/info/"),
        Some(&referer),
    )?;
    let user = payload
        .get("user")
        .ok_or_else(|| "Instagram user info response is missing user data.".to_string())?;
    let username = string_from_value(user.get("username"))
        .map(|value| normalize_instagram_username(&value))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Instagram user info response is missing username.".to_string())?;
    let resolved_user_id = string_from_value(user.get("pk"))
        .or_else(|| string_from_value(user.get("id")))
        .unwrap_or_else(|| user_id.to_string());

    Ok(InstagramProfileIdentity {
        username,
        user_id: resolved_user_id,
    })
}

fn discover_user_stories_manifest_section(
    client: &mut InstagramClient,
    request: &InstagramConnectorRequest,
    profile: &UserProfile,
    manifest: &mut InstagramSyncManifest,
) -> Result<(), String> {
    let payload = client.get_json(
        &format!(
            "https://www.instagram.com/api/v1/feed/reels_media/?reel_ids={}",
            profile.user_id
        ),
        Some(&format!("https://www.instagram.com/{}/", request.username)),
    )?;
    let items = hydrate_story_items_if_needed(client, extract_reels_payload_items(&payload));

    manifest.sections.push(build_manifest_section(
        "stories_user",
        section_label("stories_user").to_string(),
        request.profile_root.join("Stories (user)"),
        items,
        Some(&profile.user_id),
    ));

    Ok(())
}

fn discover_target_story_manifest_section(
    client: &mut InstagramClient,
    request: &InstagramConnectorRequest,
    profile: &UserProfile,
    story_media_id: &str,
    manifest: &mut InstagramSyncManifest,
) -> Result<(), String> {
    let media_id = story_media_id.trim();
    if media_id.is_empty() || !media_id.chars().all(|value| value.is_ascii_digit()) {
        return Err("Selected Instagram story id is invalid.".to_string());
    }

    let payload = client.get_json(
        &format!("https://i.instagram.com/api/v1/media/{media_id}/info/"),
        Some(&format!(
            "https://www.instagram.com/stories/{}/{media_id}/",
            request.username
        )),
    )?;
    let item = payload
        .get("items")
        .and_then(Value::as_array)
        .and_then(|entries| entries.first())
        .cloned()
        .or_else(|| payload.get("item").cloned())
        .ok_or_else(|| "Selected Instagram story was not found.".to_string())?;

    if let Some(owner_id) = item_owner_user_id(&item) {
        if owner_id != profile.user_id {
            return Err("Selected Instagram story belongs to a different profile.".to_string());
        }
    }

    let items = hydrate_story_items_if_needed(client, vec![item]);
    manifest.sections.push(build_manifest_section(
        "stories_user",
        "Selected story".to_string(),
        request.profile_root.join("Stories (user)"),
        items,
        Some(&profile.user_id),
    ));

    Ok(())
}

fn discover_highlights_manifest_sections(
    client: &mut InstagramClient,
    request: &InstagramConnectorRequest,
    profile: &UserProfile,
    manifest: &mut InstagramSyncManifest,
) -> Result<(), String> {
    if profile.highlight_tray.is_empty() {
        return Ok(());
    }

    for highlight in &profile.highlight_tray {
        let payload = client.get_json(
            &format!(
                "https://i.instagram.com/api/v1/feed/reels_media/?reel_ids=highlight:{}",
                highlight.id
            ),
            Some(&format!("https://www.instagram.com/{}/", request.username)),
        )?;
        let items = hydrate_story_items_if_needed(client, extract_reels_payload_items(&payload));
        let title = sanitize_path_segment(&highlight.title);
        let section_name = if title.is_empty() {
            format!("Story_{}", highlight.id)
        } else {
            title
        };
        let section_root = request.profile_root.join("Stories").join(&section_name);

        manifest.sections.push(build_manifest_section(
            "stories",
            format!("{} / {}", section_label("stories"), section_name),
            section_root,
            items,
            Some(&profile.user_id),
        ));
    }

    Ok(())
}

fn execute_manifest_section<F>(
    client: &mut InstagramClient,
    request: &InstagramConnectorRequest,
    section: &InstagramManifestSection,
    total_queued_assets: usize,
    downloaded_media: &mut Vec<DownloadedInstagramMedia>,
    progress: &mut F,
    should_cancel: &impl Fn() -> bool,
) -> Result<(), String>
where
    F: FnMut(InstagramProgress),
{
    ensure_sync_not_cancelled(should_cancel)?;
    let mut processed_assets = downloaded_media.len();
    let referer = format!("https://www.instagram.com/{}/", request.username);
    let executable_post_count = section
        .posts
        .iter()
        .filter(|post| manifest_post_requires_execution(post))
        .count();
    let queued_asset_count = section
        .posts
        .iter()
        .map(|post| post.planned_assets.len())
        .sum::<usize>();

    report_profile_phase_progress(
        progress,
        "Downloading media",
        format!(
            "{}: processing {} queued assets across {} executable posts",
            section.display_label, queued_asset_count, executable_post_count
        ),
        Some(downloaded_media.len() as u32),
        if total_queued_assets == 0 {
            Some(100)
        } else {
            Some(((processed_assets * 100) / total_queued_assets) as u32)
        },
        total_queued_assets == 0,
    );

    for post in &section.posts {
        ensure_sync_not_cancelled(should_cancel)?;
        if post.write_text_sidecar {
            write_text_sidecar_for_item(
                request,
                &section.media_section,
                &section.section_root,
                &post.item,
            )?;
        }

        for planned_asset in &post.planned_assets {
            ensure_sync_not_cancelled(should_cancel)?;
            let mut asset_available = planned_asset.destination_path.exists();
            if !asset_available {
                match client.download_file(
                    &planned_asset.asset.file_url,
                    &planned_asset.destination_path,
                    Some(&referer),
                ) {
                    Ok(()) => {
                        asset_available = true;
                    }
                    Err(error)
                        if should_ignore_media_download_error(&section.media_section, &error) => {}
                    Err(error) => return Err(error),
                }
            }

            if asset_available {
                downloaded_media.push(DownloadedInstagramMedia {
                    file_path: planned_asset.destination_path.clone(),
                    media_type: planned_asset.asset.media_type.clone(),
                    media_section: section.media_section.clone(),
                    provider_media_key: planned_asset.asset.provider_media_key.clone(),
                    provider_post_code: planned_asset.asset.provider_post_code.clone(),
                    captured_at_timestamp: planned_asset.asset.captured_at_timestamp,
                    final_file_name: planned_asset
                        .destination_path
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                        .to_string(),
                    legacy_raw_file_name: planned_asset.asset.legacy_raw_file_name.clone(),
                    extension: planned_asset.asset.extension.clone(),
                    pattern_mode: request.media_file_naming_mode.as_str().to_string(),
                    pattern_template: request.media_file_naming_template.clone(),
                });
            }

            processed_assets += 1;
            report_profile_phase_progress(
                progress,
                "Downloading media",
                format!(
                    "{}: processed {}/{} queued assets",
                    section.display_label,
                    processed_assets,
                    total_queued_assets.max(1)
                ),
                Some(downloaded_media.len() as u32),
                Some(((processed_assets * 100) / total_queued_assets.max(1)) as u32),
                false,
            );
        }
    }

    Ok(())
}

fn ensure_sync_not_cancelled(should_cancel: &impl Fn() -> bool) -> Result<(), String> {
    if should_cancel() {
        Err("source sync cancelled by user".to_string())
    } else {
        Ok(())
    }
}

fn planned_destination_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

fn build_destination_base_path(
    section_root: &Path,
    asset: &MediaAsset,
    request: &InstagramConnectorRequest,
) -> PathBuf {
    let target_root = if asset.media_type == "video"
        || (asset.extracted_from_video && request.place_extracted_image_into_video_folder)
    {
        section_root.join("Video")
    } else {
        section_root.to_path_buf()
    };
    target_root.join(&asset.file_name)
}

fn profile_relative_path_key(profile_root: &Path, path: &Path) -> String {
    let relative = path
        .strip_prefix(profile_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    relative.trim().trim_start_matches('/').to_ascii_lowercase()
}

fn item_matches_requested_date_range(item: &Value, request: &InstagramConnectorRequest) -> bool {
    let Some(timestamp) = media_item_timestamp(item) else {
        return true;
    };

    if let Some(date_from_timestamp) = request.date_from_timestamp {
        if timestamp < date_from_timestamp {
            return false;
        }
    }

    if let Some(date_to_timestamp) = request.date_to_timestamp {
        if timestamp > date_to_timestamp {
            return false;
        }
    }

    true
}

fn uses_contextual_story_post_dedupe(section: &InstagramManifestSection) -> bool {
    matches!(section.media_section.as_str(), "stories" | "stories_user")
}

fn story_post_dedupe_scope(section: &InstagramManifestSection) -> String {
    match section.media_section.as_str() {
        "stories" => format!(
            "stories:{}",
            section
                .section_root
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase()
        ),
        "stories_user" => "stories_user".to_string(),
        _ => section.media_section.clone(),
    }
}

fn extract_reels_payload_items(payload: &Value) -> Vec<Value> {
    let mut items = Vec::new();
    let mut seen_item_keys = HashSet::new();

    for field in ["reels_media", "reels"] {
        if let Some(values) = payload.get(field).and_then(Value::as_array) {
            for reel in values {
                append_reel_items(reel, &mut items, &mut seen_item_keys);
            }
        } else if let Some(values) = payload.get(field).and_then(Value::as_object) {
            for reel in values.values() {
                append_reel_items(reel, &mut items, &mut seen_item_keys);
            }
        }
    }

    items
}

fn append_reel_items(reel: &Value, output: &mut Vec<Value>, seen_item_keys: &mut HashSet<String>) {
    if let Some(items) = reel.get("items").and_then(Value::as_array) {
        for item in items {
            let key = string_from_value(item.get("id"))
                .or_else(|| string_from_value(item.get("pk")))
                .map(|value| normalize_instagram_post_identity_key(&value));

            if let Some(ref item_key) = key {
                if !seen_item_keys.insert(item_key.clone()) {
                    continue;
                }
            }

            output.push(item.clone());
        }
    }
}

fn hydrate_story_items_if_needed(client: &mut InstagramClient, items: Vec<Value>) -> Vec<Value> {
    items
        .into_iter()
        .map(|item| {
            if media_item_has_downloadable_media(&item) {
                return item;
            }

            let Some(media_id) =
                string_from_value(item.get("id")).or_else(|| string_from_value(item.get("pk")))
            else {
                return item;
            };

            let Ok(payload) = client.get_json(
                &format!("https://i.instagram.com/api/v1/media/{media_id}/info/"),
                Some("https://www.instagram.com/"),
            ) else {
                return item;
            };

            payload
                .get("items")
                .and_then(Value::as_array)
                .and_then(|entries| entries.first())
                .cloned()
                .or_else(|| payload.get("item").cloned())
                .filter(media_item_has_downloadable_media)
                .unwrap_or(item)
        })
        .collect()
}

fn item_owner_user_id(item: &Value) -> Option<String> {
    string_from_value(item.pointer("/user/pk"))
        .or_else(|| string_from_value(item.pointer("/user/id")))
        .or_else(|| string_from_value(item.pointer("/owner/pk")))
        .or_else(|| string_from_value(item.pointer("/owner/id")))
}

fn media_item_has_downloadable_media(item: &Value) -> bool {
    best_video_url(item).is_some() || best_image_url(item).is_some()
}

fn media_item_timestamp(item: &Value) -> Option<i64> {
    item.get("taken_at")
        .and_then(Value::as_i64)
        .or_else(|| item.get("taken_at_timestamp").and_then(Value::as_i64))
}

fn summarize_profile_sync_manifest(
    manifest: &InstagramSyncManifest,
    downloaded_asset_count: u32,
    profile_user_id: Option<&str>,
) -> InstagramManifestSummary {
    let section_summaries = manifest
        .sections
        .iter()
        .map(|section| InstagramManifestSectionSummary {
            section: section.media_section.clone(),
            label: section.display_label.clone(),
            item_count: section.items.len() as u32,
            normalized_post_count: section.normalized_post_count as u32,
            discovered_asset_count: section.discovered_asset_count as u32,
            queued_asset_count: section
                .posts
                .iter()
                .map(|post| post.planned_assets.len() as u32)
                .sum(),
            skipped_existing_post_count: section.skipped_existing_post_count as u32,
            skipped_duplicate_post_count: section.skipped_duplicate_post_count as u32,
            skipped_unavailable_post_count: section.skipped_unavailable_post_count as u32,
            skipped_existing_asset_count: section.skipped_existing_asset_count as u32,
            skipped_duplicate_asset_count: section.skipped_duplicate_asset_count as u32,
        })
        .collect::<Vec<_>>();

    InstagramManifestSummary {
        profile_user_id: profile_user_id.map(str::to_string),
        section_count: manifest.sections.len() as u32,
        discovered_item_count: manifest
            .sections
            .iter()
            .map(|section| section.items.len() as u32)
            .sum(),
        normalized_post_count: manifest
            .sections
            .iter()
            .map(|section| section.normalized_post_count as u32)
            .sum(),
        discovered_asset_count: manifest
            .sections
            .iter()
            .map(|section| section.discovered_asset_count as u32)
            .sum(),
        queued_asset_count: manifest
            .sections
            .iter()
            .flat_map(|section| section.posts.iter())
            .map(|post| post.planned_assets.len() as u32)
            .sum(),
        skipped_existing_post_count: manifest
            .sections
            .iter()
            .map(|section| section.skipped_existing_post_count as u32)
            .sum(),
        skipped_duplicate_post_count: manifest
            .sections
            .iter()
            .map(|section| section.skipped_duplicate_post_count as u32)
            .sum(),
        skipped_unavailable_post_count: manifest
            .sections
            .iter()
            .map(|section| section.skipped_unavailable_post_count as u32)
            .sum(),
        skipped_existing_asset_count: manifest
            .sections
            .iter()
            .map(|section| section.skipped_existing_asset_count as u32)
            .sum(),
        skipped_duplicate_asset_count: manifest
            .sections
            .iter()
            .map(|section| section.skipped_duplicate_asset_count as u32)
            .sum(),
        downloaded_asset_count,
        sections: section_summaries,
    }
}

fn load_profile(client: &mut InstagramClient, username: &str) -> Result<UserProfile, String> {
    let referer = format!("https://www.instagram.com/{username}/");
    // Match SCrawler's initial profile/timeline discovery route.
    let timeline_payload = client.get_json(
        &format!("https://www.instagram.com/api/v1/feed/user/{username}/username/?count=30"),
        Some(&referer),
    )?;

    let user = timeline_payload
        .get("user")
        .ok_or_else(|| infer_missing_timeline_user_data_error(client, username, &referer))?;
    let canonical_username = string_from_value(user.get("username"))
        .map(|value| normalize_instagram_username(&value))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| normalize_instagram_username(username));
    let user_id = string_from_value(user.get("pk"))
        .or_else(|| string_from_value(user.get("id")))
        .ok_or_else(|| "Instagram timeline response is missing the user identifier.".to_string())?;
    let description = parse_profile_description_from_user(user);

    let timeline_items = timeline_payload
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let timeline_next_max_id = next_max_id(&timeline_payload);
    let reel_items = timeline_items
        .iter()
        .filter(|item| is_clip_product(item))
        .cloned()
        .collect::<Vec<_>>();
    let highlight_tray = load_highlight_tray(client, &user_id, username).unwrap_or_default();

    Ok(UserProfile {
        username: canonical_username,
        user_id,
        description,
        timeline_items,
        timeline_next_max_id,
        reel_items,
        tagged_items: Vec::new(),
        highlight_tray,
    })
}

fn infer_missing_timeline_user_data_error(
    client: &mut InstagramClient,
    username: &str,
    referer: &str,
) -> String {
    let marker_for_private = "[identity_probe=instagram_profile_private_or_restricted]";
    let marker_for_unresolvable = "[identity_probe=instagram_username_unresolvable]";
    let probe_url =
        format!("https://i.instagram.com/api/v1/users/web_profile_info/?username={username}");

    match client.get_json(&probe_url, Some(referer)) {
        Ok(payload) => {
            let maybe_user = payload
                .pointer("/data/user")
                .or_else(|| payload.get("user"));

            match maybe_user {
                Some(user) => {
                    if user
                        .get("is_private")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        format!(
                            "Instagram timeline response is missing user data. \
                             {marker_for_private} Profile accessibility probe confirmed \
                             `web_profile_info.data.user.is_private=true`."
                        )
                    } else {
                        "Instagram timeline response is missing user data.".to_string()
                    }
                }
                None => format!(
                    "Instagram timeline response is missing user data. \
                     {marker_for_unresolvable} Profile accessibility probe returned no user object."
                ),
            }
        }
        Err(error) => match extract_http_status_code(&error) {
            Some(429) => infer_missing_timeline_user_data_from_html_probe(
                client,
                username,
                marker_for_private,
                marker_for_unresolvable,
            ),
            Some(404) => format!(
                "Instagram timeline response is missing user data. \
                 {marker_for_unresolvable} Profile accessibility probe returned 404."
            ),
            Some(400) | Some(401) | Some(403) => format!(
                "Instagram timeline response is missing user data. \
                 {marker_for_private} Profile accessibility probe returned auth status {}.",
                extract_http_status_code(&error).unwrap_or_default()
            ),
            _ => infer_missing_timeline_user_data_from_html_probe(
                client,
                username,
                marker_for_private,
                marker_for_unresolvable,
            ),
        },
    }
}

fn infer_missing_timeline_user_data_from_html_probe(
    client: &mut InstagramClient,
    username: &str,
    marker_for_private: &str,
    marker_for_unresolvable: &str,
) -> String {
    let profile_url = format!("https://www.instagram.com/{username}/");
    let (status, body) =
        match client.send_text_request(&profile_url, Some(&profile_url), client.header_mode, &[]) {
            Ok(result) => result,
            Err(_) => return "Instagram timeline response is missing user data.".to_string(),
        };
    let lower = body.to_ascii_lowercase();

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        // Inconclusive. Do not add identity probe markers.
        return "Instagram timeline response is missing user data. HTML profile probe returned 429 Too Many Requests.".to_string();
    }

    if status == reqwest::StatusCode::NOT_FOUND
        || lower.contains("comet.igweb.polariserrorroute")
        || lower.contains("page isn't available")
        || lower.contains("page isnt available")
        || lower.contains("the link you followed may be broken")
    {
        return format!(
            "Instagram timeline response is missing user data. \
             {marker_for_unresolvable} HTML profile probe indicates unavailable profile."
        );
    }

    if matches!(
        status,
        reqwest::StatusCode::BAD_REQUEST
            | reqwest::StatusCode::UNAUTHORIZED
            | reqwest::StatusCode::FORBIDDEN
    ) || lower.contains("\"is_private\":true")
        || lower.contains("\"is_private\": true")
        || lower.contains("this account is private")
        || lower.contains("this account is private.")
    {
        return format!(
            "Instagram timeline response is missing user data. \
             {marker_for_private} HTML profile probe indicates private/restricted profile."
        );
    }

    "Instagram timeline response is missing user data.".to_string()
}

fn load_profile_description(client: &mut InstagramClient, username: &str) -> Option<String> {
    let referer = format!("https://www.instagram.com/{username}/");
    let payload = client
        .get_json(
            &format!("https://i.instagram.com/api/v1/users/web_profile_info/?username={username}"),
            Some(&referer),
        )
        .ok()?;

    parse_profile_description(&payload)
}

fn load_profile_description_by_user_id(
    client: &mut InstagramClient,
    username: &str,
    user_id: &str,
) -> Option<String> {
    let referer = format!("https://www.instagram.com/{username}/");
    let payload = client
        .get_json(
            &format!("https://i.instagram.com/api/v1/users/{user_id}/info/"),
            Some(&referer),
        )
        .ok()?;

    parse_profile_description(&payload)
}

fn load_profile_description_gql(
    client: &mut InstagramClient,
    username: &str,
    user_id: &str,
) -> Option<String> {
    let (lsd, dtsg) = gql_tokens(&client.headers)?;
    let friendly_name = "PolarisProfilePageContentQuery";
    let variables = format!(
        "{{\"id\":\"{}\",\"relay_header\":false,\"render_surface\":\"PROFILE\"}}",
        escape_json(user_id)
    );
    let referer = format!("https://www.instagram.com/{username}/");
    let url = build_graphql_url("7381344031985950", &lsd, &dtsg, friendly_name, &variables);
    let payload = client
        .get_json_with_extra_headers(
            &url,
            Some(&referer),
            &[
                ("x-fb-friendly-name", friendly_name.to_string()),
                ("x-fb-lsd", lsd),
            ],
        )
        .ok()?;

    parse_profile_description(&payload)
}

fn parse_profile_description(payload: &Value) -> Option<String> {
    payload
        .pointer("/data/user")
        .or_else(|| payload.pointer("/user"))
        .and_then(parse_profile_description_from_user)
}

fn parse_profile_description_from_user(user: &Value) -> Option<String> {
    let mut description = string_from_value(user.get("biography"))
        .or_else(|| string_from_value(user.pointer("/biography_with_entities/raw_text")))
        .or_else(|| string_from_value(user.pointer("/biography_with_entities/text")))
        .unwrap_or_default();
    let mut bio_links = collect_profile_link_values(user.get("bio_links"));
    bio_links.extend(collect_profile_link_values(
        user.pointer("/biography_with_entities/entities"),
    ));
    bio_links.dedup();

    if !bio_links.is_empty() {
        if !description.trim().is_empty() {
            description.push('\n');
        }
        description.push_str(&bio_links.join("\n"));
    }

    if let Some(external_url) = string_from_value(user.get("external_url"))
        .or_else(|| string_from_value(user.get("external_url_linkshimmed")))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        if description.is_empty() || !description.contains(&external_url) {
            if !description.trim().is_empty() {
                description.push('\n');
            }
            description.push_str(&external_url);
        }
    }

    let description = description.trim();
    if description.is_empty() {
        None
    } else {
        Some(description.to_string())
    }
}

fn collect_profile_link_values(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    string_from_value(item.get("url"))
                        .or_else(|| string_from_value(item.get("link_url")))
                        .or_else(|| string_from_value(item.get("lynx_url")))
                })
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn load_timeline_items(
    client: &mut InstagramClient,
    username: &str,
    profile: &UserProfile,
    page_size: u32,
    use_gql: bool,
) -> Result<Vec<Value>, String> {
    if use_gql {
        if let Some((lsd, dtsg)) = gql_tokens(&client.headers) {
            let mut items = Vec::new();
            let mut cursor = None::<String>;
            loop {
                let (doc_id, friendly_name) = if cursor.is_none() {
                    ("7268577773270422", "PolarisProfilePostsQuery")
                } else {
                    (
                        "7286316061475375",
                        "PolarisProfilePostsTabContentQuery_connection",
                    )
                };
                let variables =
                    build_timeline_gql_variables(username, page_size, cursor.as_deref());
                let url = build_graphql_url(doc_id, &lsd, &dtsg, friendly_name, &variables);
                let payload = match client.get_json_with_extra_headers(
                    &url,
                    Some(&format!("https://www.instagram.com/{username}/")),
                    &[
                        ("x-fb-friendly-name", friendly_name.to_string()),
                        ("x-fb-lsd", lsd.clone()),
                    ],
                ) {
                    Ok(payload) => payload,
                    Err(error) => {
                        if is_auth_error_status(extract_http_status_code(&error), "timeline") {
                            return Err(error);
                        }
                        break;
                    }
                };

                let page_items = payload
                    .pointer("/data/xdt_api__v1__feed__user_timeline_graphql_connection/edges")
                    .and_then(Value::as_array)
                    .map(|edges| {
                        edges
                            .iter()
                            .filter_map(|edge| edge.get("node").cloned())
                            .filter(|item| !is_clip_product(item))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                if page_items.is_empty() {
                    return Ok(items);
                }

                items.extend(page_items);
                let has_next_page = payload
                    .pointer(
                        "/data/xdt_api__v1__feed__user_timeline_graphql_connection/page_info/has_next_page",
                    )
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let next_cursor = payload
                    .pointer(
                        "/data/xdt_api__v1__feed__user_timeline_graphql_connection/page_info/end_cursor",
                    )
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if !has_next_page || next_cursor.as_deref().unwrap_or("").trim().is_empty() {
                    return Ok(items);
                }
                cursor = next_cursor;
            }
        }
    }

    let mut items = profile.timeline_items.clone();
    let mut max_id = profile.timeline_next_max_id.clone();
    if items.is_empty() || max_id.is_some() {
        loop {
            let url = match max_id.as_deref() {
                Some(cursor) => format!(
                    "https://www.instagram.com/api/v1/feed/user/{}/username/?count={}&max_id={}",
                    username, page_size, cursor
                ),
                None => format!(
                    "https://www.instagram.com/api/v1/feed/user/{}/username/?count={}",
                    username, page_size
                ),
            };
            let payload = match client.get_json(
                &url,
                Some(&format!("https://www.instagram.com/{username}/")),
            ) {
                Ok(payload) => payload,
                Err(error) => {
                    if max_id.is_some() {
                        return Err(format!("Timeline pagination failed: {error}"));
                    }
                    return Err(error);
                }
            };
            let page_items = payload
                .get("items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();

            if page_items.is_empty() {
                break;
            }

            items.extend(page_items);
            let next = next_max_id(&payload);
            if next.is_none() {
                break;
            }
            max_id = next;
        }
    }

    Ok(items
        .into_iter()
        .filter(|item| !is_clip_product(item))
        .collect())
}

fn load_highlight_tray(
    client: &mut InstagramClient,
    user_id: &str,
    username: &str,
) -> Result<Vec<HighlightTrayItem>, String> {
    let payload = client.get_json(
        &format!(
            "https://i.instagram.com/api/v1/highlights/{}/highlights_tray/",
            user_id
        ),
        Some(&format!("https://www.instagram.com/{username}/")),
    )?;
    Ok(payload
        .get("tray")
        .and_then(Value::as_array)
        .map(|tray| {
            tray.iter()
                .filter_map(|item| {
                    let raw_id = string_from_value(item.get("id"))?;
                    let id = raw_id
                        .strip_prefix("highlight:")
                        .unwrap_or(raw_id.as_str())
                        .to_string();
                    let title = string_from_value(item.get("title"))
                        .unwrap_or_else(|| format!("Story_{id}"));
                    Some(HighlightTrayItem { id, title })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default())
}

fn load_reel_items(
    client: &mut InstagramClient,
    profile: &UserProfile,
    page_size: u32,
    use_gql: bool,
) -> Result<Vec<Value>, String> {
    let mut items = profile
        .reel_items
        .clone()
        .into_iter()
        .filter(|item| is_clip_product(item))
        .collect::<Vec<_>>();
    let mut max_id = None;

    if use_gql {
        if let Some((lsd, dtsg)) = gql_tokens(&client.headers) {
            let mut gql_cursor = None::<String>;
            loop {
                let variables = build_reels_gql_variables(
                    profile.user_id.as_str(),
                    page_size,
                    gql_cursor.as_deref(),
                );
                let url = build_graphql_url(
                    "7191572580905225",
                    &lsd,
                    &dtsg,
                    "PolarisProfileReelsTabContentQuery",
                    &variables,
                );
                let payload = match client.get_json_with_extra_headers(
                    &url,
                    Some("https://www.instagram.com/"),
                    &[
                        (
                            "x-fb-friendly-name",
                            "PolarisProfileReelsTabContentQuery".to_string(),
                        ),
                        ("x-fb-lsd", lsd.clone()),
                    ],
                ) {
                    Ok(payload) => payload,
                    Err(error) => {
                        if is_auth_error_status(extract_http_status_code(&error), "reels") {
                            return Err(error);
                        }
                        break;
                    }
                };

                let page_items = payload
                    .pointer("/data/xdt_api__v1__clips__user__connection_v2/edges")
                    .and_then(Value::as_array)
                    .map(|edges| {
                        edges
                            .iter()
                            .filter_map(|edge| {
                                edge.pointer("/node/media")
                                    .cloned()
                                    .or_else(|| edge.get("node").cloned())
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                if page_items.is_empty() {
                    return Ok(items);
                }

                items.extend(page_items);
                let has_next_page = payload
                    .pointer(
                        "/data/xdt_api__v1__clips__user__connection_v2/page_info/has_next_page",
                    )
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let next_cursor = payload
                    .pointer("/data/xdt_api__v1__clips__user__connection_v2/page_info/end_cursor")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if !has_next_page || next_cursor.as_deref().unwrap_or("").trim().is_empty() {
                    return Ok(items);
                }
                gql_cursor = next_cursor;
            }
        }
    }

    loop {
        let url = match max_id.as_deref() {
            Some(cursor) => format!(
                "https://i.instagram.com/api/v1/clips/user/?target_user_id={}&page_size={}&max_id={}",
                profile.user_id, page_size, cursor
            ),
            None => format!(
                "https://i.instagram.com/api/v1/clips/user/?target_user_id={}&page_size={}",
                profile.user_id, page_size
            ),
        };
        let payload = match client.get_json(&url, Some("https://www.instagram.com/")) {
            Ok(payload) => payload,
            Err(error) if is_method_not_allowed_error(&error) => break,
            Err(error) => return Err(error),
        };
        let page_items = payload
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .or_else(|| payload.get("clips").and_then(Value::as_array).cloned())
            .unwrap_or_default();

        if page_items.is_empty() {
            break;
        }

        items.extend(page_items);
        let next = next_max_id(&payload);
        if next.is_none() {
            break;
        }
        max_id = next;
    }

    Ok(items)
}

fn load_tagged_items(
    client: &mut InstagramClient,
    profile: &UserProfile,
    page_size: u32,
    use_gql: bool,
) -> Result<Vec<Value>, String> {
    let mut items = profile.tagged_items.clone();
    let mut max_id = None;

    if use_gql {
        if let Some((lsd, dtsg)) = gql_tokens(&client.headers) {
            let mut gql_cursor = None::<String>;
            loop {
                let variables = build_tagged_gql_variables(
                    profile.user_id.as_str(),
                    page_size,
                    gql_cursor.as_deref(),
                );
                let url = build_graphql_url(
                    "7289408964443685",
                    &lsd,
                    &dtsg,
                    "PolarisProfileTaggedTabContentQuery",
                    &variables,
                );
                let payload = match client.get_json_with_extra_headers(
                    &url,
                    Some("https://www.instagram.com/"),
                    &[
                        (
                            "x-fb-friendly-name",
                            "PolarisProfileTaggedTabContentQuery".to_string(),
                        ),
                        ("x-fb-lsd", lsd.clone()),
                    ],
                ) {
                    Ok(payload) => payload,
                    Err(error) => {
                        if is_auth_error_status(extract_http_status_code(&error), "tagged") {
                            return Err(error);
                        }
                        break;
                    }
                };

                let page_items = payload
                    .pointer("/data/xdt_api__v1__usertags__user_id__feed_connection/edges")
                    .and_then(Value::as_array)
                    .map(|edges| {
                        edges
                            .iter()
                            .filter_map(|edge| {
                                edge.get("node")
                                    .cloned()
                                    .or_else(|| edge.get("media").cloned())
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                if page_items.is_empty() {
                    return hydrate_tagged_items(client, items);
                }

                items.extend(page_items);
                let has_next_page = payload
                    .pointer(
                        "/data/xdt_api__v1__usertags__user_id__feed_connection/page_info/has_next_page",
                    )
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let next_cursor = payload
                    .pointer(
                        "/data/xdt_api__v1__usertags__user_id__feed_connection/page_info/end_cursor",
                    )
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if !has_next_page || next_cursor.as_deref().unwrap_or("").trim().is_empty() {
                    return hydrate_tagged_items(client, items);
                }
                gql_cursor = next_cursor;
            }
        }
    }

    loop {
        let url = match max_id.as_deref() {
            Some(cursor) => format!(
                "https://i.instagram.com/api/v1/usertags/{}/feed/?count={}&max_id={}",
                profile.user_id, page_size, cursor
            ),
            None => format!(
                "https://i.instagram.com/api/v1/usertags/{}/feed/?count={}",
                profile.user_id, page_size
            ),
        };
        let payload = match client.get_json(&url, Some("https://www.instagram.com/")) {
            Ok(payload) => payload,
            Err(error) => {
                if max_id.is_some() {
                    return Err(format!("Tagged pagination failed: {error}"));
                }
                return Err(error);
            }
        };
        let page_items = payload
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        if page_items.is_empty() {
            break;
        }

        items.extend(page_items);
        let next = next_max_id(&payload);
        if next.is_none() {
            break;
        }
        max_id = next;
    }

    hydrate_tagged_items(client, items)
}

fn load_saved_posts_items(
    client: &mut InstagramClient,
    page_size: u32,
) -> Result<Vec<Value>, String> {
    let mut items = Vec::new();
    let mut max_id = None;

    loop {
        let url = match max_id.as_deref() {
            Some(cursor) => format!(
                "https://i.instagram.com/api/v1/feed/saved/posts/?count={}&max_id={}",
                page_size, cursor
            ),
            None => format!(
                "https://i.instagram.com/api/v1/feed/saved/posts/?count={}",
                page_size
            ),
        };
        let payload = client.get_json(&url, Some("https://www.instagram.com/"))?;
        let page_items = payload
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        if page_items.is_empty() {
            break;
        }

        items.extend(page_items);
        let next = next_max_id(&payload);
        if next.is_none() {
            break;
        }
        max_id = next;
    }

    Ok(items)
}

fn download_items_section<F>(
    client: &mut InstagramClient,
    request: &InstagramConnectorRequest,
    media_section: &str,
    section_root: &Path,
    items: Vec<Value>,
    profile_user_id: Option<&str>,
    downloaded_media: &mut Vec<DownloadedInstagramMedia>,
    progress: &mut F,
) -> Result<(), String>
where
    F: FnMut(InstagramProgress),
{
    let referer = format!("https://www.instagram.com/{}/", request.username);
    let assets = collect_media_assets(&items, request, media_section, profile_user_id)?;
    write_text_sidecars_for_items(request, media_section, section_root, &items)?;
    let mut reserved_paths = HashSet::new();

    progress(InstagramProgress {
        label: section_label(media_section).to_string(),
        detail: format!("Preparing {} assets", assets.len()),
        downloaded_items: Some(downloaded_media.len() as u32),
        progress_percent: None,
        indeterminate: true,
    });

    for (index, asset) in assets.iter().enumerate() {
        if request
            .existing_media_keys
            .contains(&asset.provider_media_key)
        {
            progress(InstagramProgress {
                label: section_label(media_section).to_string(),
                detail: format!("Processed {}/{} assets", index + 1, assets.len()),
                downloaded_items: Some(downloaded_media.len() as u32),
                progress_percent: Some(((index + 1) * 100 / assets.len().max(1)) as u32),
                indeterminate: false,
            });
            continue;
        }

        let destination_path =
            resolve_destination_path(section_root, asset, request, &reserved_paths);
        reserved_paths.insert(planned_destination_key(&destination_path));
        let mut asset_available = destination_path.exists();
        if !asset_available {
            match client.download_file(&asset.file_url, &destination_path, Some(&referer)) {
                Ok(()) => {
                    asset_available = true;
                }
                Err(error) if should_ignore_media_download_error(media_section, &error) => {}
                Err(error) => return Err(error),
            }
        }

        if asset_available {
            downloaded_media.push(DownloadedInstagramMedia {
                file_path: destination_path.clone(),
                media_type: asset.media_type.clone(),
                media_section: media_section.to_string(),
                provider_media_key: asset.provider_media_key.clone(),
                provider_post_code: asset.provider_post_code.clone(),
                captured_at_timestamp: asset.captured_at_timestamp,
                final_file_name: destination_path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_string(),
                legacy_raw_file_name: asset.legacy_raw_file_name.clone(),
                extension: asset.extension.clone(),
                pattern_mode: request.media_file_naming_mode.as_str().to_string(),
                pattern_template: request.media_file_naming_template.clone(),
            });
        }

        progress(InstagramProgress {
            label: section_label(media_section).to_string(),
            detail: format!("Processed {}/{} assets", index + 1, assets.len()),
            downloaded_items: Some(downloaded_media.len() as u32),
            progress_percent: Some(((index + 1) * 100 / assets.len().max(1)) as u32),
            indeterminate: false,
        });
    }

    Ok(())
}

fn collect_media_assets(
    items: &[Value],
    request: &InstagramConnectorRequest,
    media_section: &str,
    profile_user_id: Option<&str>,
) -> Result<Vec<MediaAsset>, String> {
    let mut assets = Vec::new();
    for item in items {
        if request.get_user_media_only
            && profile_user_id.is_some_and(|user_id| !item_belongs_to_user(item, user_id))
        {
            continue;
        }

        append_assets_from_item(item, &mut assets, request, media_section)?;
    }
    Ok(assets)
}

/// Raw post shortcode kept with its original casing. Unlike
/// `media_item_post_identity`, this is NOT lowercased because Instagram
/// shortcodes are case-sensitive and feed the public post URL.
fn raw_post_code(item: &Value) -> Option<String> {
    string_from_value(item.get("code"))
        .or_else(|| string_from_value(item.get("shortcode")))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn append_assets_from_item(
    item: &Value,
    assets: &mut Vec<MediaAsset>,
    request: &InstagramConnectorRequest,
    media_section: &str,
) -> Result<(), String> {
    // The shortcode lives on the parent post; carousel children inherit it.
    let post_code = raw_post_code(item);

    if let Some(edges) = item
        .pointer("/edge_sidecar_to_children/edges")
        .and_then(Value::as_array)
    {
        for (index, edge) in edges.iter().enumerate() {
            let child = edge.get("node").unwrap_or(edge);
            append_single_asset(child, assets, index, request, media_section, post_code.as_deref())?;
        }
        return Ok(());
    }

    if let Some(children) = item.get("carousel_media").and_then(Value::as_array) {
        for (index, child) in children.iter().enumerate() {
            append_single_asset(child, assets, index, request, media_section, post_code.as_deref())?;
        }
        return Ok(());
    }

    append_single_asset(item, assets, 0, request, media_section, post_code.as_deref())
}

fn append_single_asset(
    item: &Value,
    assets: &mut Vec<MediaAsset>,
    variant_index: usize,
    request: &InstagramConnectorRequest,
    media_section: &str,
    post_code: Option<&str>,
) -> Result<(), String> {
    let item_id = string_from_value(item.get("id"))
        .or_else(|| string_from_value(item.get("pk")))
        .ok_or_else(|| "Instagram media item is missing an identifier.".to_string())?;

    let extract_image_from_video = extract_image_from_video_enabled(request, media_section);
    let captured_at_timestamp = media_item_timestamp(item);

    if let Some(url) = best_video_url(item) {
        if request.download_videos {
            let (provider_media_key, legacy_raw_file_name) =
                provider_media_identity_from_url(url, &item_id, variant_index);
            let file_name = build_media_file_name(
                request,
                captured_at_timestamp,
                &provider_media_key,
                "mp4",
                legacy_raw_file_name.as_deref(),
            );
            assets.push(MediaAsset {
                file_url: url.to_string(),
                media_type: "video".to_string(),
                extracted_from_video: false,
                file_name,
                provider_media_key,
                provider_post_code: post_code.map(str::to_string),
                captured_at_timestamp,
                legacy_raw_file_name,
                extension: "mp4".to_string(),
            });
        }

        if extract_image_from_video {
            if let Some(image_url) = best_image_url(item) {
                let (provider_media_key, legacy_raw_file_name) =
                    provider_media_identity_from_url(image_url, &item_id, variant_index);
                let file_name = build_media_file_name(
                    request,
                    captured_at_timestamp,
                    &provider_media_key,
                    "jpg",
                    legacy_raw_file_name.as_deref(),
                );
                assets.push(MediaAsset {
                    file_url: image_url.to_string(),
                    media_type: "image".to_string(),
                    extracted_from_video: true,
                    file_name,
                    provider_media_key,
                    provider_post_code: post_code.map(str::to_string),
                    captured_at_timestamp,
                    legacy_raw_file_name,
                    extension: "jpg".to_string(),
                });
            }
        }
        return Ok(());
    }

    if request.download_images {
        if let Some(url) = best_image_url(item) {
            let (provider_media_key, legacy_raw_file_name) =
                provider_media_identity_from_url(url, &item_id, variant_index);
            let file_name = build_media_file_name(
                request,
                captured_at_timestamp,
                &provider_media_key,
                "jpg",
                legacy_raw_file_name.as_deref(),
            );
            assets.push(MediaAsset {
                file_url: url.to_string(),
                media_type: "image".to_string(),
                extracted_from_video: false,
                file_name,
                provider_media_key,
                provider_post_code: post_code.map(str::to_string),
                captured_at_timestamp,
                legacy_raw_file_name,
                extension: "jpg".to_string(),
            });
        }
    }

    Ok(())
}

fn resolve_destination_path(
    section_root: &Path,
    asset: &MediaAsset,
    request: &InstagramConnectorRequest,
    reserved_paths: &HashSet<String>,
) -> PathBuf {
    let base_path = build_destination_base_path(section_root, asset, request);
    let target_root = base_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| section_root.to_path_buf());
    let base_key = planned_destination_key(&base_path);
    if !reserved_paths.contains(&base_key) && !base_path.exists() {
        return base_path;
    }

    let fallback_stem = sanitize_path_segment(&asset.provider_media_key);
    let stem = Path::new(&asset.file_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&fallback_stem)
        .to_string();
    let extension = Path::new(&asset.file_name)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| asset.extension.to_ascii_lowercase());

    let mut index = 2usize;
    loop {
        let candidate_name = if extension.is_empty() {
            format!("{stem}_{index}")
        } else {
            format!("{stem}_{index}.{}", extension)
        };
        let candidate = target_root.join(candidate_name);
        let candidate_key = planned_destination_key(&candidate);
        if !reserved_paths.contains(&candidate_key) && !candidate.exists() {
            return candidate;
        }
        index += 1;
    }
}

fn write_text_sidecars_for_items(
    request: &InstagramConnectorRequest,
    media_section: &str,
    section_root: &Path,
    items: &[Value],
) -> Result<(), String> {
    if !should_write_text_sidecar(request, media_section) {
        return Ok(());
    }

    for item in items {
        write_text_sidecar_for_item(request, media_section, section_root, item)?;
    }

    Ok(())
}

fn write_text_sidecar_for_item(
    request: &InstagramConnectorRequest,
    media_section: &str,
    section_root: &Path,
    item: &Value,
) -> Result<(), String> {
    if !should_write_text_sidecar(request, media_section) {
        return Ok(());
    }

    let text_root = if request.text_special_folder {
        section_root.join("Text")
    } else {
        section_root.to_path_buf()
    };
    fs::create_dir_all(&text_root).map_err(|error| error.to_string())?;

    let Some(text) = media_item_text(item) else {
        return Ok(());
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(());
    }

    let item_id = string_from_value(item.get("id"))
        .or_else(|| string_from_value(item.get("pk")))
        .unwrap_or_else(|| "unknown".to_string());
    let timestamp = item
        .get("taken_at")
        .and_then(Value::as_i64)
        .or_else(|| item.get("taken_at_timestamp").and_then(Value::as_i64))
        .unwrap_or(0);
    let file_stem = format!("{}_{}", timestamp, sanitize_path_segment(&item_id));
    let destination = text_root.join(format!("{file_stem}.txt"));
    fs::write(destination, trimmed).map_err(|error| error.to_string())?;

    Ok(())
}

fn media_item_text(item: &Value) -> Option<String> {
    item.get("caption")
        .and_then(|caption| caption.get("text"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            item.pointer("/edge_media_to_caption/edges/0/node/text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .or_else(|| {
            item.get("accessibility_caption")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
}

fn should_write_text_sidecar(request: &InstagramConnectorRequest, media_section: &str) -> bool {
    if request.download_text {
        return true;
    }

    request.download_text_posts
        && matches!(
            media_section,
            "timeline" | "reels" | "tagged" | "saved_posts"
        )
}

fn item_belongs_to_user(item: &Value, user_id: &str) -> bool {
    item.pointer("/user/pk")
        .or_else(|| item.pointer("/user/id"))
        .and_then(|value| {
            value
                .as_i64()
                .map(|raw| raw.to_string())
                .or_else(|| value.as_str().map(str::to_string))
        })
        .map(|value| value == user_id)
        .or_else(|| {
            item.pointer("/owner/id")
                .or_else(|| item.pointer("/owner/pk"))
                .and_then(Value::as_str)
                .map(|value| value == user_id)
        })
        .unwrap_or(true)
}

fn extract_image_from_video_enabled(
    request: &InstagramConnectorRequest,
    media_section: &str,
) -> bool {
    match media_section {
        "timeline" => request.extract_image_from_video.timeline,
        "reels" => request.extract_image_from_video.reels,
        "stories" => request.extract_image_from_video.stories,
        "stories_user" => request.extract_image_from_video.stories_user,
        "tagged" => request.extract_image_from_video.tagged,
        "saved_posts" => request.extract_image_from_video.timeline,
        _ => false,
    }
}

fn best_video_url(item: &Value) -> Option<&str> {
    item.get("video_versions")
        .and_then(Value::as_array)
        .and_then(|versions| {
            versions
                .iter()
                .filter(|version| {
                    version
                        .get("url")
                        .and_then(Value::as_str)
                        .is_some_and(is_downloadable_media_url)
                })
                .max_by(|left, right| compare_numeric_field(left, right, "width"))
        })
        .and_then(|version| version.get("url"))
        .and_then(Value::as_str)
        .filter(|url| is_downloadable_media_url(url))
        .or_else(|| item.get("video_url").and_then(Value::as_str))
        .filter(|url| is_downloadable_media_url(url))
}

fn best_image_url(item: &Value) -> Option<&str> {
    item.pointer("/image_versions2/candidates")
        .and_then(Value::as_array)
        .and_then(|candidates| {
            candidates
                .iter()
                .filter(|candidate| {
                    candidate
                        .get("url")
                        .and_then(Value::as_str)
                        .is_some_and(is_downloadable_media_url)
                })
                .max_by(|left, right| compare_numeric_field(left, right, "width"))
        })
        .and_then(|candidate| candidate.get("url"))
        .and_then(Value::as_str)
        .filter(|url| is_downloadable_media_url(url))
        .or_else(|| item.get("display_url").and_then(Value::as_str))
        .filter(|url| is_downloadable_media_url(url))
        .or_else(|| {
            item.get("display_resources")
                .and_then(Value::as_array)
                .and_then(|resources| {
                    resources
                        .iter()
                        .filter(|resource| {
                            resource
                                .get("src")
                                .and_then(Value::as_str)
                                .is_some_and(is_downloadable_media_url)
                        })
                        .max_by(|left, right| compare_numeric_field(left, right, "config_width"))
                })
                .and_then(|resource| resource.get("src"))
                .and_then(Value::as_str)
        })
        .filter(|url| is_downloadable_media_url(url))
}

fn compare_numeric_field(left: &Value, right: &Value, field: &str) -> Ordering {
    left.get(field)
        .and_then(Value::as_i64)
        .unwrap_or_default()
        .cmp(&right.get(field).and_then(Value::as_i64).unwrap_or_default())
}

fn is_downloadable_media_url(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    if parsed.host_str().is_none() {
        return false;
    }

    !parsed
        .path_segments()
        .and_then(|mut segments| segments.next_back().map(|value| value.to_ascii_lowercase()))
        .is_some_and(|value| {
            matches!(
                value.as_str(),
                "null.jpg" | "null.jpeg" | "null.png" | "null.webp" | "null.mp4"
            )
        })
}

fn should_ignore_media_download_error(media_section: &str, error: &str) -> bool {
    matches!(media_section, "stories" | "stories_user")
        && error
            .to_ascii_lowercase()
            .contains("static.cdninstagram.com/rsrc.php/null.")
}

fn handle_section_error(
    error: String,
    skip_errors: bool,
    ignore_stories_560_errors: bool,
    use_gql: bool,
    section: &str,
    section_errors: &mut Vec<String>,
    validation_error: &mut Option<String>,
    auth_disabled_sections: &mut Vec<String>,
    rate_limited: &mut bool,
) -> Result<(), String> {
    let message = format!("{}: {}", section_label(section), error);
    if extract_http_status_code(&error) == Some(429) {
        *rate_limited = true;
    }
    match classify_section_error(section, &error, ignore_stories_560_errors) {
        SectionErrorDisposition::AuthInvalid => {
            if validation_error.is_none() {
                *validation_error = Some(message.clone());
            }
            section_errors.push(message.clone());
            for section_name in sections_to_disable_for_auth_error(section, use_gql) {
                if !auth_disabled_sections
                    .iter()
                    .any(|value| value.eq_ignore_ascii_case(section_name))
                {
                    auth_disabled_sections.push(section_name.to_string());
                }
            }
            Err(message)
        }
        SectionErrorDisposition::AlwaysWarn => {
            section_errors.push(message);
            Ok(())
        }
        SectionErrorDisposition::ForceFail => Err(message),
        SectionErrorDisposition::Generic => {
            if skip_errors {
                section_errors.push(message);
                Ok(())
            } else {
                Err(message)
            }
        }
    }
}

fn build_cookie_header(cookies: &[SessionCookie]) -> String {
    cookies
        .iter()
        .filter(|cookie| !cookie.name.trim().is_empty() && !cookie.value.trim().is_empty())
        .map(|cookie| format!("{}={}", cookie.name.trim(), cookie.value.trim()))
        .collect::<Vec<_>>()
        .join("; ")
}

fn next_max_id(payload: &Value) -> Option<String> {
    string_from_value(payload.get("next_max_id"))
        .or_else(|| string_from_value(payload.pointer("/paging_info/max_id")))
        .filter(|value| !value.trim().is_empty())
}

fn classify_section_error(
    section: &str,
    error: &str,
    ignore_stories_560_errors: bool,
) -> SectionErrorDisposition {
    let status = extract_http_status_code(error);
    if is_auth_error_status(status, section) {
        return SectionErrorDisposition::AuthInvalid;
    }

    if status == Some(429)
        || (section == "reels" && status == Some(405))
        || (section == "tagged" && status == Some(403))
        || (section == "stories" && status == Some(560) && ignore_stories_560_errors)
    {
        return SectionErrorDisposition::AlwaysWarn;
    }

    if (section == "timeline" || section == "tagged")
        && error.to_ascii_lowercase().contains("pagination failed")
    {
        return SectionErrorDisposition::ForceFail;
    }

    SectionErrorDisposition::Generic
}

fn sections_to_disable_for_auth_error(section: &str, use_gql: bool) -> Vec<&'static str> {
    if section == "reels" && !use_gql {
        return vec!["reels"];
    }
    if section == "tagged" {
        return vec!["tagged"];
    }
    vec!["timeline", "reels", "stories", "stories_user", "tagged"]
}

fn is_auth_error_status(status: Option<u16>, section: &str) -> bool {
    match status {
        Some(400) | Some(401) => true,
        Some(403) => section != "tagged",
        _ => false,
    }
}

fn extract_http_status_code(error: &str) -> Option<u16> {
    let lower = error.to_ascii_lowercase();
    let marker_index = lower.find("returned")?;
    let after = &error[marker_index + "returned".len()..];
    let digits = after
        .chars()
        .skip_while(|value| !value.is_ascii_digit())
        .take_while(|value| value.is_ascii_digit())
        .collect::<String>();
    digits.parse::<u16>().ok()
}

fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn gql_tokens(headers: &InstagramAuthHeaders) -> Option<(String, String)> {
    let lsd = headers.lsd.as_deref()?.trim();
    let dtsg = headers.dtsg.as_deref()?.trim();
    if lsd.is_empty() || dtsg.is_empty() {
        None
    } else {
        Some((lsd.to_string(), dtsg.to_string()))
    }
}

fn build_reels_gql_variables(user_id: &str, page_size: u32, cursor: Option<&str>) -> String {
    match cursor {
        Some(cursor_value) if !cursor_value.trim().is_empty() => format!(
            "{{\"after\":\"{}\",\"before\":null,\"data\":{{\"include_feed_video\":true,\"page_size\":{},\"target_user_id\":\"{}\"}},\"first\":4,\"last\":null}}",
            escape_json(cursor_value),
            page_size,
            escape_json(user_id)
        ),
        _ => format!(
            "{{\"data\":{{\"include_feed_video\":true,\"page_size\":{},\"target_user_id\":\"{}\"}}}}",
            page_size,
            escape_json(user_id)
        ),
    }
}

fn build_tagged_gql_variables(user_id: &str, page_size: u32, cursor: Option<&str>) -> String {
    match cursor {
        Some(cursor_value) if !cursor_value.trim().is_empty() => format!(
            "{{\"after\":\"{}\",\"before\":null,\"count\":{},\"first\":{},\"last\":null,\"user_id\":\"{}\"}}",
            escape_json(cursor_value),
            page_size,
            page_size,
            escape_json(user_id)
        ),
        _ => format!(
            "{{\"count\":{},\"user_id\":\"{}\"}}",
            page_size,
            escape_json(user_id)
        ),
    }
}

fn build_timeline_gql_variables(username: &str, page_size: u32, cursor: Option<&str>) -> String {
    match cursor {
        Some(cursor_value) if !cursor_value.trim().is_empty() => format!(
            "{{\"after\":\"{}\",\"before\":null,\"data\":{{\"count\":{},\"include_relationship_info\":true,\"latest_besties_reel_media\":true,\"latest_reel_media\":true}},\"first\":{},\"last\":null,\"username\":\"{}\",\"__relay_internal__pv__PolarisShareMenurelayprovider\":false}}",
            escape_json(cursor_value),
            page_size,
            page_size,
            escape_json(username),
        ),
        _ => format!(
            "{{\"data\":{{\"count\":{},\"include_relationship_info\":true,\"latest_besties_reel_media\":true,\"latest_reel_media\":true}},\"username\":\"{}\",\"__relay_internal__pv__PolarisShareMenurelayprovider\":false}}",
            page_size,
            escape_json(username),
        ),
    }
}

fn hydrate_tagged_items(
    client: &mut InstagramClient,
    items: Vec<Value>,
) -> Result<Vec<Value>, String> {
    if items.is_empty() {
        return Ok(items);
    }

    let mut hydrated_items = Vec::with_capacity(items.len());
    for item in items {
        let media_id =
            string_from_value(item.get("id")).or_else(|| string_from_value(item.get("pk")));
        let Some(media_id) = media_id else {
            hydrated_items.push(item);
            continue;
        };

        let payload = match client.get_json(
            &format!("https://i.instagram.com/api/v1/media/{}/info/", media_id),
            Some("https://www.instagram.com/"),
        ) {
            Ok(payload) => payload,
            Err(error) => {
                if is_auth_error_status(extract_http_status_code(&error), "tagged") {
                    return Err(error);
                }
                hydrated_items.push(item);
                continue;
            }
        };

        let hydrated = payload
            .get("items")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .cloned()
            .or_else(|| payload.get("item").cloned());

        if let Some(value) = hydrated {
            hydrated_items.push(value);
        } else {
            hydrated_items.push(item);
        }
    }

    Ok(hydrated_items)
}

fn build_graphql_url(
    doc_id: &str,
    lsd: &str,
    dtsg: &str,
    friendly_name: &str,
    variables_json: &str,
) -> String {
    format!(
        "https://www.instagram.com/api/graphql?doc_id={}&lsd={}&fb_dtsg={}&fb_api_req_friendly_name={}&variables={}",
        percent_encode_component(doc_id),
        percent_encode_component(lsd),
        percent_encode_component(dtsg),
        percent_encode_component(friendly_name),
        percent_encode_component(variables_json),
    )
}

fn escape_json(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn percent_encode_component(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(byte as char);
        } else {
            output.push('%');
            output.push_str(&format!("{:02X}", byte));
        }
    }
    output
}

fn is_method_not_allowed_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("405 method not allowed") || normalized.contains(" returned 405")
}

fn string_from_value(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(raw)) => Some(raw.trim().to_string()),
        Some(Value::Number(number)) => Some(number.to_string()),
        _ => None,
    }
}

fn normalize_instagram_username(value: &str) -> String {
    value
        .trim()
        .trim_matches('/')
        .trim_start_matches('@')
        .to_string()
}

fn sanitize_path_segment(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            value if value.is_control() => '_',
            value => value,
        })
        .collect::<String>()
        .trim_matches('.')
        .trim()
        .to_string()
}

fn provider_media_identity_from_url(
    url: &str,
    fallback_item_id: &str,
    variant_index: usize,
) -> (String, Option<String>) {
    if let Some(raw_file_name) = media_file_name_from_url(url) {
        let file_name = sanitize_media_file_name(&raw_file_name);
        if let Some(provider_media_key) = media_identity_key_from_file_name(&file_name) {
            return (provider_media_key, Some(file_name));
        }
    }

    let fallback_stem = fallback_media_stem(fallback_item_id, variant_index);
    (fallback_stem, None)
}

fn build_media_file_name(
    request: &InstagramConnectorRequest,
    captured_at_timestamp: Option<i64>,
    provider_media_key: &str,
    fallback_extension: &str,
    legacy_raw_file_name: Option<&str>,
) -> String {
    let extension = fallback_extension.trim().to_ascii_lowercase();
    let sanitized_key = sanitize_path_segment(provider_media_key);
    let legacy_name = legacy_raw_file_name
        .map(sanitize_media_file_name)
        .filter(|value| !value.trim().is_empty());
    let datetime = format_media_timestamp(captured_at_timestamp);

    let raw_name = match request.media_file_naming_mode {
        InstagramMediaFileNamingMode::PresetLegacyUrlBasename => {
            legacy_name.unwrap_or_else(|| format!("{sanitized_key}.{extension}"))
        }
        InstagramMediaFileNamingMode::PresetNewDefault => {
            format!("{datetime} {sanitized_key}.{extension}")
        }
        InstagramMediaFileNamingMode::Custom => {
            let template = request
                .media_file_naming_template
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty());
            match template {
                Some(value) => {
                    let rendered = render_media_name_template(
                        value,
                        &datetime,
                        &sanitized_key,
                        &extension,
                        legacy_name.as_deref().unwrap_or(""),
                    );
                    if rendered.trim().is_empty() {
                        format!("{datetime} {sanitized_key}.{extension}")
                    } else {
                        rendered
                    }
                }
                None => format!("{datetime} {sanitized_key}.{extension}"),
            }
        }
    };

    let mut sanitized = sanitize_media_file_name(&raw_name);
    if Path::new(&sanitized).extension().is_none() && !extension.is_empty() {
        sanitized.push('.');
        sanitized.push_str(&extension);
    }

    sanitized
}

fn render_media_name_template(
    template: &str,
    datetime: &str,
    provider_media_key: &str,
    extension: &str,
    raw_file_name: &str,
) -> String {
    template
        .replace("{datetime}", datetime)
        .replace("{provider_media_key}", provider_media_key)
        .replace("{ext}", extension)
        .replace("{raw_file_name}", raw_file_name)
}

fn format_media_timestamp(timestamp: Option<i64>) -> String {
    let local_time = timestamp
        .and_then(|value| Local.timestamp_opt(value, 0).single())
        .unwrap_or_else(Local::now);
    local_time.format("%Y-%m-%d %H.%M.%S").to_string()
}

fn media_file_name_from_url(url: &str) -> Option<String> {
    let path = url.split('?').next().unwrap_or(url);
    let candidate = path.rsplit('/').next()?.trim();
    if candidate.is_empty() {
        None
    } else {
        Some(candidate.to_string())
    }
}

fn sanitize_media_file_name(file_name: &str) -> String {
    let sanitized = file_name
        .trim()
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            value if value.is_control() => '_',
            value => value,
        })
        .collect::<String>();

    if sanitized.is_empty() {
        "unknown.bin".to_string()
    } else {
        sanitized
    }
}

fn media_identity_key_from_file_name(file_name: &str) -> Option<String> {
    Path::new(file_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

fn fallback_media_stem(item_id: &str, variant_index: usize) -> String {
    let base = sanitize_path_segment(item_id);
    if variant_index > 0 {
        format!("{}_{}", base, variant_index + 1).to_ascii_lowercase()
    } else {
        base.to_ascii_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        append_single_asset, best_image_url, build_manifest_section, build_media_file_name,
        collect_media_assets, execute_manifest_section, extract_reels_payload_items,
        normalize_profile_sync_manifest,
        parse_profile_description, parse_profile_description_from_user,
        provider_media_identity_from_url, public_identity_headers, resolve_destination_path,
        should_ignore_media_download_error, DownloadedInstagramMedia, InstagramAuthHeaders,
        InstagramClient, InstagramConnectorRequest, InstagramManifestPost,
        InstagramMediaFileNamingMode, InstagramSectionSelection, InstagramSyncManifest, MediaAsset,
        PlannedMediaAsset,
    };
    use serde_json::json;
    use std::collections::HashSet;
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn public_identity_headers_drop_session_and_keep_app_id() {
        let session = InstagramAuthHeaders {
            csrf_token: Some("secret-csrf".to_string()),
            app_id: Some("111222333".to_string()),
            asbd_id: Some("359341".to_string()),
            user_agent: Some("CustomUA/1.0".to_string()),
            ig_www_claim: Some("claim".to_string()),
            ..Default::default()
        };
        let public = public_identity_headers(&session);
        // App id, asbd id e user agent são preservados.
        assert_eq!(public.app_id.as_deref(), Some("111222333"));
        assert_eq!(public.asbd_id.as_deref(), Some("359341"));
        assert_eq!(public.user_agent.as_deref(), Some("CustomUA/1.0"));
        // CSRF e demais sinais de sessão são descartados.
        assert!(public.csrf_token.is_none());
        assert!(public.ig_www_claim.is_none());

        // Sem app id/asbd na sessão, cai nos defaults públicos.
        let empty = InstagramAuthHeaders {
            app_id: Some("   ".to_string()),
            ..Default::default()
        };
        let fallback = public_identity_headers(&empty);
        assert_eq!(
            fallback.app_id.as_deref(),
            Some(super::INSTAGRAM_PUBLIC_APP_ID)
        );
        assert_eq!(
            fallback.asbd_id.as_deref(),
            Some(super::INSTAGRAM_PUBLIC_ASBD_ID)
        );
    }

    fn sample_request() -> InstagramConnectorRequest {
        InstagramConnectorRequest {
            username: "stories".to_string(),
            cookies: Vec::new(),
            headers: InstagramAuthHeaders::default(),
            profile_root: Path::new(r"C:\Media\Stories").to_path_buf(),
            saved_posts_root: Path::new(r"C:\Media\Saved").to_path_buf(),
            ledger_post_keys: HashSet::new(),
            existing_media_keys: HashSet::new(),
            ledger_media_keys: HashSet::new(),
            existing_relative_paths: HashSet::new(),
            ledger_relative_paths: HashSet::new(),
            sections: InstagramSectionSelection::default(),
            use_gql: false,
            download_saved_posts: false,
            post_page_size: 30,
            skip_errors: true,
            ignore_stories_560_errors: false,
            request_delay_ms: 0,
            timeout_secs: 30,
            download_images: true,
            download_videos: true,
            extract_image_from_video: InstagramSectionSelection::default(),
            place_extracted_image_into_video_folder: false,
            download_text: false,
            download_text_posts: false,
            text_special_folder: false,
            get_user_media_only: false,
            missing_only: false,
            date_from_timestamp: None,
            date_to_timestamp: None,
            media_file_naming_mode: InstagramMediaFileNamingMode::PresetNewDefault,
            media_file_naming_template: None,
            target_story_media_id: None,
        }
    }

    #[test]
    fn parse_profile_description_combines_bio_links_and_external_url() {
        let payload = json!({
            "data": {
                "user": {
                    "biography": "Main biography",
                    "bio_links": [
                        { "url": "https://example.com/alpha" },
                        { "url": "https://example.com/beta" }
                    ],
                    "external_url": "https://example.com/root"
                }
            }
        });

        let description = parse_profile_description(&payload).expect("description should parse");

        assert_eq!(
            description,
            "Main biography\nhttps://example.com/alpha\nhttps://example.com/beta\nhttps://example.com/root"
        );
    }

    #[test]
    fn parse_profile_description_avoids_duplicate_external_url() {
        let payload = json!({
            "data": {
                "user": {
                    "biography": "Main biography\nhttps://example.com/root",
                    "bio_links": [],
                    "external_url": "https://example.com/root"
                }
            }
        });

        let description = parse_profile_description(&payload).expect("description should parse");

        assert_eq!(description, "Main biography\nhttps://example.com/root");
    }

    #[test]
    fn parse_profile_description_supports_top_level_user_payload() {
        let payload = json!({
            "user": {
                "biography": "ID biography",
                "bio_links": [
                    { "url": "https://example.com/id" }
                ]
            }
        });

        let description = parse_profile_description(&payload).expect("description should parse");

        assert_eq!(description, "ID biography\nhttps://example.com/id");
    }

    #[test]
    fn parse_profile_description_from_user_supports_timeline_user_payload() {
        let user = json!({
            "biography": "Timeline biography",
            "bio_links": [
                { "url": "https://example.com/timeline" }
            ],
            "external_url": "https://example.com/root"
        });

        let description =
            parse_profile_description_from_user(&user).expect("description should parse");

        assert_eq!(
            description,
            "Timeline biography\nhttps://example.com/timeline\nhttps://example.com/root"
        );
    }

    #[test]
    fn parse_profile_description_from_user_supports_gql_biography_entities() {
        let user = json!({
            "biography_with_entities": {
                "raw_text": "GQL biography",
                "entities": [
                    { "url": "https://example.com/gql" }
                ]
            },
            "external_url_linkshimmed": "https://example.com/root"
        });

        let description =
            parse_profile_description_from_user(&user).expect("description should parse");

        assert_eq!(
            description,
            "GQL biography\nhttps://example.com/gql\nhttps://example.com/root"
        );
    }

    #[test]
    fn best_image_url_ignores_null_placeholder_candidates() {
        let item = json!({
            "image_versions2": {
                "candidates": [
                    { "url": "http://static.cdninstagram.com/rsrc.php/null.jpg", "width": 1080 },
                    { "url": "https://cdninstagram.example/path/real.jpg", "width": 720 }
                ]
            }
        });

        assert_eq!(
            best_image_url(&item),
            Some("https://cdninstagram.example/path/real.jpg")
        );
    }

    #[test]
    fn append_single_asset_skips_placeholder_story_extract_images() {
        let item = json!({
            "id": "story-1",
            "video_versions": [
                { "url": "https://cdninstagram.example/path/story.mp4", "width": 720 }
            ],
            "image_versions2": {
                "candidates": [
                    { "url": "http://static.cdninstagram.com/rsrc.php/null.jpg", "width": 1080 }
                ]
            }
        });
        let request = sample_request();
        let mut assets = Vec::new();

        append_single_asset(&item, &mut assets, 0, &request, "stories", None)
            .expect("story asset extraction should succeed");

        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].media_type, "video");
        assert!(!assets[0].extracted_from_video);
    }

    #[test]
    fn collect_media_assets_propagates_cased_shortcode_to_carousel_children() {
        // O shortcode fica no post pai; os filhos do carrossel herdam (com casing).
        let item = json!({
            "id": "123_456",
            "code": "CyAbC-1_x",
            "edge_sidecar_to_children": {
                "edges": [
                    { "node": { "id": "child-1", "display_url": "https://cdninstagram.example/a/aaa111.jpg" } },
                    { "node": { "id": "child-2", "display_url": "https://cdninstagram.example/b/bbb222.jpg" } }
                ]
            }
        });
        let request = sample_request();
        let assets = collect_media_assets(std::slice::from_ref(&item), &request, "timeline", None)
            .expect("asset extraction should succeed");

        assert_eq!(assets.len(), 2);
        for asset in &assets {
            assert_eq!(asset.provider_post_code.as_deref(), Some("CyAbC-1_x"));
        }
    }

    #[test]
    fn placeholder_story_download_errors_are_ignored() {
        assert!(should_ignore_media_download_error(
            "stories",
            "Instagram media request 'http://static.cdninstagram.com/rsrc.php/null.jpg' returned 400 Bad Request"
        ));
        assert!(!should_ignore_media_download_error(
            "timeline",
            "Instagram media request 'http://static.cdninstagram.com/rsrc.php/null.jpg' returned 400 Bad Request"
        ));
    }

    #[test]
    fn media_identity_uses_url_basename_and_ignores_query_string() {
        let (provider_media_key, legacy_file_name) = provider_media_identity_from_url(
            "https://cdninstagram.example/path/631495592_18384355651158098_6314965943446164250_n.webp?stp=dst-jpg_e35&foo=bar",
            "fallback-id",
            0,
        );

        assert_eq!(
            legacy_file_name.as_deref(),
            Some("631495592_18384355651158098_6314965943446164250_n.webp")
        );
        assert_eq!(
            provider_media_key,
            "631495592_18384355651158098_6314965943446164250_n"
        );
    }

    #[test]
    fn build_media_file_name_uses_new_default_pattern() {
        let request = sample_request();
        let file_name = build_media_file_name(
            &request,
            Some(1_711_800_191),
            "3339838382976122123_46124578107",
            "mp4",
            None,
        );
        assert_eq!(
            file_name,
            "2024-04-05 20.29.51 3339838382976122123_46124578107.mp4"
        );
    }

    #[test]
    fn resolve_destination_path_uses_file_name_layout() {
        let asset = MediaAsset {
            file_url: "https://cdninstagram.example/path/631495592_18384355651158098_6314965943446164250_n.jpg".to_string(),
            media_type: "video".to_string(),
            extracted_from_video: false,
            file_name: "631495592_18384355651158098_6314965943446164250_n.jpg".to_string(),
            provider_media_key: "631495592_18384355651158098_6314965943446164250_n".to_string(),
            provider_post_code: None,
            captured_at_timestamp: Some(1_700_000_000),
            legacy_raw_file_name: Some("631495592_18384355651158098_6314965943446164250_n.jpg".to_string()),
            extension: "jpg".to_string(),
        };

        let request = sample_request();
        let resolved = resolve_destination_path(
            Path::new(r"C:\Media\Stories"),
            &asset,
            &request,
            &HashSet::new(),
        );
        assert_eq!(
            resolved,
            Path::new(
                r"C:\Media\Stories\Video\631495592_18384355651158098_6314965943446164250_n.jpg"
            )
        );
    }

    #[test]
    fn normalize_manifest_skips_ledger_hits_by_default_but_missing_only_restores_them() {
        let item = json!({
            "id": "media-1",
            "taken_at": 1_700_000_000_i64,
            "image_versions2": {
                "candidates": [
                    { "url": "https://cdninstagram.example/path/ledger-hit.jpg", "width": 720 }
                ]
            }
        });
        let mut default_request = sample_request();
        default_request
            .ledger_media_keys
            .insert("ledger-hit".to_string());
        let mut default_manifest = InstagramSyncManifest {
            sections: vec![build_manifest_section(
                "timeline",
                "Timeline".to_string(),
                default_request.profile_root.clone(),
                vec![item.clone()],
                None,
            )],
        };

        normalize_profile_sync_manifest(
            &default_request,
            &mut default_manifest,
            &mut |_| {},
            &|| false,
        )
        .expect("default normalization should succeed");

        assert_eq!(default_manifest.sections[0].posts.len(), 1);
        assert_eq!(
            default_manifest.sections[0].posts[0].planned_assets.len(),
            0
        );
        assert_eq!(default_manifest.sections[0].skipped_existing_asset_count, 1);

        let mut missing_only_request = sample_request();
        missing_only_request.missing_only = true;
        missing_only_request
            .ledger_media_keys
            .insert("ledger-hit".to_string());
        let mut missing_only_manifest = InstagramSyncManifest {
            sections: vec![build_manifest_section(
                "timeline",
                "Timeline".to_string(),
                missing_only_request.profile_root.clone(),
                vec![item],
                None,
            )],
        };

        normalize_profile_sync_manifest(
            &missing_only_request,
            &mut missing_only_manifest,
            &mut |_| {},
            &|| false,
        )
        .expect("missing-only normalization should succeed");

        assert_eq!(missing_only_manifest.sections[0].posts.len(), 1);
        assert_eq!(
            missing_only_manifest.sections[0].posts[0]
                .planned_assets
                .len(),
            1
        );
        assert_eq!(
            missing_only_manifest.sections[0].skipped_existing_asset_count,
            0
        );
    }

    #[test]
    fn normalize_manifest_skips_existing_base_path_before_generating_suffix() {
        let temp_root = std::env::temp_dir().join(format!(
            "ninjacrawler-instagram-existing-path-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time should be monotonic")
                .as_nanos()
        ));
        fs::create_dir_all(&temp_root).expect("temp root should exist");

        let existing_path = temp_root.join("existing.jpg");
        fs::write(&existing_path, b"already-here").expect("existing file should be created");

        let item = json!({
            "id": "media-1",
            "taken_at": 1_700_000_000_i64,
            "image_versions2": {
                "candidates": [
                    { "url": "https://cdninstagram.example/path/existing.jpg", "width": 720 }
                ]
            }
        });

        let mut request = sample_request();
        request.profile_root = temp_root.clone();
        request.media_file_naming_mode = InstagramMediaFileNamingMode::PresetLegacyUrlBasename;
        request
            .existing_relative_paths
            .insert("existing.jpg".to_string());

        let mut manifest = InstagramSyncManifest {
            sections: vec![build_manifest_section(
                "timeline",
                "Timeline".to_string(),
                request.profile_root.clone(),
                vec![item],
                None,
            )],
        };

        normalize_profile_sync_manifest(&request, &mut manifest, &mut |_| {}, &|| false)
            .expect("normalization should succeed");

        assert_eq!(manifest.sections[0].posts.len(), 1);
        assert_eq!(manifest.sections[0].posts[0].planned_assets.len(), 0);
        assert_eq!(manifest.sections[0].skipped_existing_asset_count, 1);

        fs::remove_dir_all(&temp_root).expect("temp root should be removed");
    }

    #[test]
    fn normalize_manifest_skips_post_ledger_hits_before_asset_planning() {
        let item = json!({
            "id": "post-ledger-1",
            "code": "ABC123",
            "taken_at": 1_700_000_000_i64,
            "image_versions2": {
                "candidates": [
                    { "url": "https://cdninstagram.example/path/post-ledger-1.jpg", "width": 720 }
                ]
            }
        });
        let mut request = sample_request();
        request.ledger_post_keys.insert("post-ledger-1".to_string());
        let mut manifest = InstagramSyncManifest {
            sections: vec![build_manifest_section(
                "timeline",
                "Timeline".to_string(),
                request.profile_root.clone(),
                vec![item],
                None,
            )],
        };

        normalize_profile_sync_manifest(&request, &mut manifest, &mut |_| {}, &|| false)
            .expect("post-ledger normalization should succeed");

        assert_eq!(manifest.sections[0].posts.len(), 0);
        assert_eq!(manifest.sections[0].discovered_asset_count, 0);
        assert_eq!(manifest.sections[0].skipped_existing_post_count, 1);
    }

    #[test]
    fn normalize_manifest_dedupes_posts_across_sections() {
        let item = json!({
            "id": "shared-post-1",
            "taken_at": 1_700_000_000_i64,
            "image_versions2": {
                "candidates": [
                    { "url": "https://cdninstagram.example/path/shared-post-1.jpg", "width": 720 }
                ]
            }
        });
        let request = sample_request();
        let mut manifest = InstagramSyncManifest {
            sections: vec![
                build_manifest_section(
                    "timeline",
                    "Timeline".to_string(),
                    request.profile_root.clone(),
                    vec![item.clone()],
                    None,
                ),
                build_manifest_section(
                    "reels",
                    "Reels".to_string(),
                    request.profile_root.join("Reels"),
                    vec![item],
                    None,
                ),
            ],
        };

        normalize_profile_sync_manifest(&request, &mut manifest, &mut |_| {}, &|| false)
            .expect("cross-section post dedupe should succeed");

        assert_eq!(manifest.sections[0].posts.len(), 1);
        assert_eq!(manifest.sections[1].posts.len(), 0);
        assert_eq!(manifest.sections[1].skipped_duplicate_post_count, 1);
    }

    #[test]
    fn normalize_manifest_keeps_story_posts_across_story_contexts() {
        let item = json!({
            "id": "shared-story-1",
            "taken_at": 1_700_000_000_i64,
            "image_versions2": {
                "candidates": [
                    { "url": "https://cdninstagram.example/path/shared-story-1.jpg", "width": 720 }
                ]
            }
        });
        let request = sample_request();
        let mut manifest = InstagramSyncManifest {
            sections: vec![
                build_manifest_section(
                    "stories",
                    "Stories / Highlight A".to_string(),
                    request.profile_root.join("Stories").join("Highlight A"),
                    vec![item.clone()],
                    None,
                ),
                build_manifest_section(
                    "stories_user",
                    "Stories (user)".to_string(),
                    request.profile_root.join("Stories (user)"),
                    vec![item],
                    None,
                ),
            ],
        };

        normalize_profile_sync_manifest(&request, &mut manifest, &mut |_| {}, &|| false)
            .expect("story normalization should keep contextual duplicates");

        assert_eq!(manifest.sections[0].posts.len(), 1);
        assert_eq!(manifest.sections[1].posts.len(), 1);
        assert_eq!(manifest.sections[1].skipped_duplicate_post_count, 0);
    }

    #[test]
    fn extract_reels_payload_items_supports_reels_object_shape() {
        let payload = json!({
            "reels": {
                "123": {
                    "items": [
                        { "id": "story-a" },
                        { "id": "story-b" }
                    ]
                },
                "456": {
                    "items": [
                        { "id": "story-c" }
                    ]
                }
            }
        });

        let items = extract_reels_payload_items(&payload);
        assert_eq!(items.len(), 3);
        assert_eq!(
            items[0].get("id").and_then(|value| value.as_str()),
            Some("story-a")
        );
        assert_eq!(
            items[2].get("id").and_then(|value| value.as_str()),
            Some("story-c")
        );
    }

    #[test]
    fn normalize_manifest_filters_items_outside_requested_date_range() {
        let older_item = json!({
            "id": "media-old",
            "taken_at": 1_700_000_000_i64,
            "image_versions2": {
                "candidates": [
                    { "url": "https://cdninstagram.example/path/old.jpg", "width": 720 }
                ]
            }
        });
        let newer_item = json!({
            "id": "media-new",
            "taken_at": 1_710_000_000_i64,
            "image_versions2": {
                "candidates": [
                    { "url": "https://cdninstagram.example/path/new.jpg", "width": 720 }
                ]
            }
        });
        let mut request = sample_request();
        request.date_from_timestamp = Some(1_705_000_000_i64);
        let mut manifest = InstagramSyncManifest {
            sections: vec![build_manifest_section(
                "timeline",
                "Timeline".to_string(),
                request.profile_root.clone(),
                vec![older_item, newer_item],
                None,
            )],
        };

        normalize_profile_sync_manifest(&request, &mut manifest, &mut |_| {}, &|| false)
            .expect("date-filter normalization should succeed");

        assert_eq!(manifest.sections[0].discovered_asset_count, 1);
        assert_eq!(manifest.sections[0].posts.len(), 1);
        assert_eq!(manifest.sections[0].posts[0].planned_assets.len(), 1);
        assert_eq!(
            manifest.sections[0].posts[0].planned_assets[0]
                .asset
                .provider_media_key,
            "new"
        );
    }

    #[test]
    fn normalize_manifest_aborts_when_sync_cancelled() {
        let item = json!({
            "id": "media-1",
            "taken_at": 1_700_000_000_i64,
            "image_versions2": {
                "candidates": [
                    { "url": "https://cdninstagram.example/path/cancelled.jpg", "width": 720 }
                ]
            }
        });
        let request = sample_request();
        let mut manifest = InstagramSyncManifest {
            sections: vec![build_manifest_section(
                "timeline",
                "Timeline".to_string(),
                request.profile_root.clone(),
                vec![item],
                None,
            )],
        };

        let error = normalize_profile_sync_manifest(&request, &mut manifest, &mut |_| {}, &|| true)
            .expect_err("cancelled normalization should abort");

        assert_eq!(error, "source sync cancelled by user");
    }

    #[test]
    fn execute_manifest_section_aborts_when_sync_cancelled() {
        let request = sample_request();
        let destination_path = request.profile_root.join("cancelled.jpg");
        let section = super::InstagramManifestSection {
            media_section: "timeline".to_string(),
            display_label: "Timeline".to_string(),
            section_root: request.profile_root.clone(),
            items: Vec::new(),
            profile_user_id: None,
            discovered_asset_count: 1,
            normalized_post_count: 1,
            skipped_existing_post_count: 0,
            skipped_duplicate_post_count: 0,
            skipped_unavailable_post_count: 0,
            skipped_existing_asset_count: 0,
            skipped_duplicate_asset_count: 0,
            posts: vec![InstagramManifestPost {
                item: json!({ "id": "media-1" }),
                provider_post_key: "media-1".to_string(),
                provider_post_code: None,
                planned_assets: vec![PlannedMediaAsset {
                    asset: MediaAsset {
                        file_url: "https://cdninstagram.example/path/cancelled.jpg".to_string(),
                        media_type: "image".to_string(),
                        extracted_from_video: false,
                        file_name: "cancelled.jpg".to_string(),
                        provider_media_key: "cancelled".to_string(),
                        provider_post_code: None,
                        captured_at_timestamp: Some(1_700_000_000),
                        legacy_raw_file_name: Some("cancelled.jpg".to_string()),
                        extension: "jpg".to_string(),
                    },
                    destination_path,
                }],
                write_text_sidecar: false,
            }],
        };
        let mut client = InstagramClient::new(&[], InstagramAuthHeaders::default(), 1, 0)
            .expect("client should build");
        let mut downloaded_media = Vec::<DownloadedInstagramMedia>::new();

        let error = execute_manifest_section(
            &mut client,
            &request,
            &section,
            1,
            &mut downloaded_media,
            &mut |_| {},
            &|| true,
        )
        .expect_err("cancelled execution should abort");

        assert_eq!(error, "source sync cancelled by user");
        assert!(downloaded_media.is_empty());
    }
}

fn truncate_for_error(input: &str) -> String {
    const MAX_LEN: usize = 220;
    let trimmed = input.trim();
    if trimmed.len() <= MAX_LEN {
        trimmed.to_string()
    } else {
        format!("{}...", &trimmed[..MAX_LEN])
    }
}

fn section_label(section: &str) -> &'static str {
    match section {
        "timeline" => "Timeline",
        "reels" => "Reels",
        "stories" => "Stories",
        "stories_user" => "Stories (user)",
        "tagged" => "Tagged",
        "saved_posts" => "Saved posts",
        _ => "Instagram",
    }
}

fn is_clip_product(item: &Value) -> bool {
    item.get("product_type")
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("clips"))
        || item
            .get("media_type")
            .and_then(Value::as_i64)
            .is_some_and(|value| value == 2 && item.get("clips_metadata").is_some())
}
