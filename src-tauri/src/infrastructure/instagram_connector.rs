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

use crate::infrastructure::{atomic_file, connector_debug};

/// App id público da web do Instagram, usado em consultas anônimas de identidade.
const INSTAGRAM_PUBLIC_APP_ID: &str = "936619743392459";
const INSTAGRAM_PUBLIC_ASBD_ID: &str = "129477";
const INSTAGRAM_WEB_ORIGIN: &str = "https://www.instagram.com";
/// Instagram's Relay GraphQL endpoint. It only accepts POST requests with the
/// query parameters in a form-urlencoded body; a GET with the same parameters in
/// the query string is rejected with `400 Bad Request` (an HTML error page).
const INSTAGRAM_GRAPHQL_ENDPOINT: &str = "https://www.instagram.com/api/graphql";

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
    /// Posts explicitamente deletados pelo usuário (tombstone). Mesmo nas seções
    /// que ignoram o post-ledger (highlights, para baixar o que falta), estes
    /// continuam suprimidos — deleção é intenção do usuário e deve ser honrada.
    pub deleted_post_keys: HashSet<String>,
    pub existing_media_keys: HashSet<String>,
    pub ledger_media_keys: HashSet<String>,
    pub existing_relative_paths: HashSet<String>,
    pub ledger_relative_paths: HashSet<String>,
    pub sections: InstagramSectionSelection,
    pub use_gql: bool,
    pub download_saved_posts: bool,
    pub post_page_size: u32,
    pub skip_errors: bool,
    /// Termos que anulam o `skip_errors`: um erro genérico contendo qualquer
    /// um deles falha a seção mesmo com skip ligado (SCrawler "skip errors
    /// exclude").
    pub skip_errors_exclude: Vec<String>,
    /// `false` silencia (no resumo/log) os erros de seção que foram pulados.
    pub log_skipped_errors: bool,
    /// Avisa quando a seção Tagged devolve mais que N posts (0 = desligado).
    pub tagged_notify_limit: u32,
    pub ignore_stories_560_errors: bool,
    pub pacing: InstagramPacing,
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
    /// Quando `true`, desliga a parada incremental da descoberta (timeline/reels)
    /// e re-percorre o perfil inteiro. Útil quando o perfil reexpõe mídias
    /// antigas que estavam ocultas — cenário pontual, não o padrão.
    pub full_scan: bool,
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
    pub profile_description_error: Option<String>,
    pub manifest_summary: Option<InstagramManifestSummary>,
    /// Associação post→álbum de highlight para TODOS os itens descobertos nos
    /// destaques (inclusive os já existentes no ledger, que não rebaixam bytes).
    pub highlight_memberships: Vec<InstagramHighlightMembership>,
    pub updated_headers: InstagramAuthHeaders,
    pub rate_limited: bool,
}

#[derive(Clone)]
pub struct InstagramHighlightMembership {
    pub album: String,
    /// Chave de mídia do CDN (stem do arquivo) — junta com o arquivo já em disco.
    pub provider_media_key: String,
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

/// Pacing dos requests do Instagram, espelhando os timers do SCrawler:
/// um delay base em todo request (`requestAnyMs`), um delay extra a cada
/// `requestCounter` requests (`requestMs`) e uma pausa entre páginas na
/// listagem de posts (`postsLimitMs`).
#[derive(Clone, Copy)]
pub struct InstagramPacing {
    pub base_delay_ms: u64,
    pub extra_delay_ms: u64,
    pub counter_threshold: u32,
    pub page_delay_ms: u64,
}

impl InstagramPacing {
    /// Sem espera alguma (probes e testes).
    pub fn none() -> Self {
        Self {
            base_delay_ms: 0,
            extra_delay_ms: 0,
            counter_threshold: 0,
            page_delay_ms: 0,
        }
    }
}

struct InstagramClient {
    client: Client,
    cookie_header: String,
    headers: InstagramAuthHeaders,
    header_mode: InstagramHeaderMode,
    pacing: InstagramPacing,
    request_count: u64,
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
    /// Itens descartados pelo filtro de data (cutoff/date range) antes do loop.
    skipped_out_of_range_item_count: usize,
    skipped_existing_post_count: usize,
    skipped_duplicate_post_count: usize,
    skipped_unavailable_post_count: usize,
    skipped_existing_asset_count: usize,
    skipped_duplicate_asset_count: usize,
    /// Media keys de TODOS os assets descobertos (highlights), p/ associação ao
    /// álbum mesmo quando o asset é pulado por já existir em disco.
    highlight_media_keys: Vec<String>,
    posts: Vec<InstagramManifestPost>,
    /// Posts cuja mídia já estava toda em disco (nada a baixar). Não entram em
    /// `posts` (não há execução), mas são registrados como observados para que o
    /// post-ledger vire um índice COMPLETO de "post já visto" — chaveado pelo
    /// id/shortcode estável, imune às fragilidades de matching de mídia
    /// (timezone no nome, candidatos de resolução, clips). Sem isso, esses posts
    /// nunca entram no ledger e re-syncs os re-processam para sempre.
    observed_existing_posts: Vec<InstagramPostIdentity>,
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
    #[serde(default)]
    pub skipped_out_of_range_item_count: u32,
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
        pacing: InstagramPacing,
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
            pacing,
            request_count: 0,
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

    /// POSTs a persisted (`doc_id`) GraphQL query to Instagram's Relay endpoint.
    /// The parameters go in a form-urlencoded body — the endpoint rejects the GET
    /// equivalent with `400 Bad Request` — mirroring what the web client sends.
    fn post_graphql_json(
        &mut self,
        doc_id: &str,
        lsd: &str,
        dtsg: &str,
        friendly_name: &str,
        variables: &str,
        referer: Option<&str>,
    ) -> Result<Value, String> {
        let acting_user_id = cookie_value(&self.cookie_header, "ds_user_id");
        let body = build_graphql_body(
            doc_id,
            lsd,
            dtsg,
            friendly_name,
            variables,
            acting_user_id.as_deref(),
        );
        let extra_headers = [
            ("x-fb-friendly-name", friendly_name.to_string()),
            ("x-fb-lsd", lsd.to_string()),
        ];

        let (mut status, mut response_body) =
            self.send_graphql_post(&body, referer, self.header_mode, &extra_headers)?;

        if status == reqwest::StatusCode::BAD_REQUEST
            && response_body
                .to_ascii_lowercase()
                .contains("useragent mismatch")
            && self.header_mode != InstagramHeaderMode::Relaxed
        {
            self.header_mode = InstagramHeaderMode::Relaxed;
            let (retry_status, retry_body) =
                self.send_graphql_post(&body, referer, self.header_mode, &extra_headers)?;
            status = retry_status;
            response_body = retry_body;
        }

        if !status.is_success() {
            return Err(format!(
                "Instagram request '{INSTAGRAM_GRAPHQL_ENDPOINT}' returned {}: {}",
                status,
                truncate_for_error(&response_body)
            ));
        }

        serde_json::from_str(&response_body).map_err(|error| {
            format!("Instagram JSON decode failed for '{INSTAGRAM_GRAPHQL_ENDPOINT}': {error}")
        })
    }

    fn send_graphql_post(
        &mut self,
        body: &str,
        referer: Option<&str>,
        header_mode: InstagramHeaderMode,
        extra_headers: &[(&str, String)],
    ) -> Result<(reqwest::StatusCode, String), String> {
        self.wait_for_pacing();
        connector_debug::append_current(
            "instagram-http",
            "call",
            "POST graphql",
            format!(
                "POST {INSTAGRAM_GRAPHQL_ENDPOINT}\nReferer: {}\nHeader-Mode: {}\n\n{body}",
                referer.unwrap_or("-"),
                if header_mode == InstagramHeaderMode::BrowserLike {
                    "browser-like"
                } else {
                    "relaxed"
                }
            ),
        );

        let mut request = self.client.post(INSTAGRAM_GRAPHQL_ENDPOINT);
        request = self.apply_headers(request, referer, header_mode);
        request = request.header("content-type", "application/x-www-form-urlencoded");
        for (name, value) in extra_headers {
            request = request.header(*name, value);
        }
        request = request.body(body.to_string());

        let response = request.send().map_err(|error| {
            connector_debug::append_current(
                "instagram-http",
                "error",
                "POST graphql",
                error.to_string(),
            );
            format!("Instagram request failed for '{INSTAGRAM_GRAPHQL_ENDPOINT}': {error}")
        })?;
        self.absorb_response_headers(response.headers());
        let status = response.status();
        let body = response.text().map_err(|error| {
            format!("Instagram response read failed for '{INSTAGRAM_GRAPHQL_ENDPOINT}': {error}")
        })?;
        self.last_request_at = Some(Instant::now());
        connector_debug::append_current(
            "instagram-http",
            "response",
            "POST graphql",
            format!("HTTP {status}\n\n{body}"),
        );
        Ok((status, body))
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
        connector_debug::append_current(
            "instagram-http",
            "call",
            "GET media",
            format!("GET {url}\nReferer: {}", referer.unwrap_or("-")),
        );
        let mut request = self.client.get(url);
        request = self.apply_headers(request, referer, self.header_mode);

        let response = request.send().map_err(|error| {
            connector_debug::append_current(
                "instagram-http",
                "error",
                "GET media",
                error.to_string(),
            );
            format!("Instagram media download failed for '{url}': {error}")
        })?;
        self.absorb_response_headers(response.headers());
        let status = response.status();
        let bytes = response
            .bytes()
            .map_err(|error| format!("Instagram media payload read failed for '{url}': {error}"))?;
        self.last_request_at = Some(Instant::now());
        connector_debug::append_current(
            "instagram-http",
            "response",
            "GET media",
            format!("HTTP {status}\nContent-Length: {}", bytes.len()),
        );

        if !status.is_success() {
            return Err(format!("Instagram media request '{url}' returned {status}"));
        }

        atomic_file::write_bytes_replacing_empty(path, bytes.as_ref())
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
        connector_debug::append_current(
            "instagram-http",
            "call",
            "GET json",
            format!(
                "GET {url}\nReferer: {}\nHeader-Mode: {}",
                referer.unwrap_or("-"),
                if header_mode == InstagramHeaderMode::BrowserLike {
                    "browser-like"
                } else {
                    "relaxed"
                }
            ),
        );

        let mut request = self.client.get(url);
        request = self.apply_headers(request, referer, header_mode);
        for (name, value) in extra_headers {
            request = request.header(*name, value);
        }

        let response = request.send().map_err(|error| {
            connector_debug::append_current(
                "instagram-http",
                "error",
                "GET json",
                error.to_string(),
            );
            format!("Instagram request failed for '{url}': {error}")
        })?;
        self.absorb_response_headers(response.headers());
        let status = response.status();
        let body = response
            .text()
            .map_err(|error| format!("Instagram response read failed for '{url}': {error}"))?;
        self.last_request_at = Some(Instant::now());
        connector_debug::append_current(
            "instagram-http",
            "response",
            "GET json",
            format!("HTTP {status}\n\n{body}"),
        );
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

    fn wait_for_pacing(&mut self) {
        self.request_count += 1;
        let mut delay = Duration::from_millis(self.pacing.base_delay_ms);
        // A cada `counter_threshold` requests, soma o delay extra (requestMs),
        // espelhando o "request timer counter" do SCrawler.
        if self.pacing.counter_threshold > 0
            && self.pacing.extra_delay_ms > 0
            && self
                .request_count
                .is_multiple_of(u64::from(self.pacing.counter_threshold))
        {
            delay += Duration::from_millis(self.pacing.extra_delay_ms);
        }
        if delay.is_zero() {
            return;
        }

        if let Some(last_request_at) = self.last_request_at {
            let elapsed = last_request_at.elapsed();
            if elapsed < delay {
                thread::sleep(delay - elapsed);
            }
        }
    }

    /// Pausa entre páginas na listagem de posts (`postsLimitMs`). Interrompível:
    /// aborta a espera assim que o cancelamento é solicitado.
    fn wait_between_post_pages(&self, should_cancel: &dyn Fn() -> bool) {
        if self.pacing.page_delay_ms > 0 {
            interruptible_sleep(
                Duration::from_millis(self.pacing.page_delay_ms),
                should_cancel,
            );
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
        request.pacing,
    )?;
    let profile = load_profile(&mut client, &request.username)?;
    let (profile_description, profile_description_error) =
        resolve_profile_description(&mut client, &profile);
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
                    SectionErrorPolicy {
                        skip_errors: effective_request.skip_errors,
                        force_fail_terms: &effective_request.skip_errors_exclude,
                        log_skipped_errors: effective_request.log_skipped_errors,
                        ignore_stories_560_errors: effective_request.ignore_stories_560_errors,
                        use_gql: effective_request.use_gql,
                    },
                    "stories_user",
                    SectionErrorSink {
                        section_errors: &mut section_errors,
                        validation_error: &mut validation_error,
                        auth_disabled_sections: &mut auth_disabled_sections,
                        rate_limited: &mut rate_limited,
                    },
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
        let discovery_stop = IncrementalDiscoveryStop::new(
            &effective_request,
            "timeline",
            &effective_request.profile_root,
            Some(&profile.user_id),
        );
        match load_timeline_items(
            &mut client,
            &profile.username,
            &profile,
            effective_request.post_page_size,
            effective_request.use_gql,
            &discovery_stop,
            &should_cancel,
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
                    SectionErrorPolicy {
                        skip_errors: effective_request.skip_errors,
                        force_fail_terms: &effective_request.skip_errors_exclude,
                        log_skipped_errors: effective_request.log_skipped_errors,
                        ignore_stories_560_errors: effective_request.ignore_stories_560_errors,
                        use_gql: effective_request.use_gql,
                    },
                    "timeline",
                    SectionErrorSink {
                        section_errors: &mut section_errors,
                        validation_error: &mut validation_error,
                        auth_disabled_sections: &mut auth_disabled_sections,
                        rate_limited: &mut rate_limited,
                    },
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
        let discovery_stop = IncrementalDiscoveryStop::new(
            &effective_request,
            "reels",
            &effective_request.profile_root,
            Some(&profile.user_id),
        );
        match load_reel_items(
            &mut client,
            &profile,
            effective_request.post_page_size,
            effective_request.use_gql,
            &discovery_stop,
            &should_cancel,
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
                    SectionErrorPolicy {
                        skip_errors: effective_request.skip_errors,
                        force_fail_terms: &effective_request.skip_errors_exclude,
                        log_skipped_errors: effective_request.log_skipped_errors,
                        ignore_stories_560_errors: effective_request.ignore_stories_560_errors,
                        use_gql: effective_request.use_gql,
                    },
                    "reels",
                    SectionErrorSink {
                        section_errors: &mut section_errors,
                        validation_error: &mut validation_error,
                        auth_disabled_sections: &mut auth_disabled_sections,
                        rate_limited: &mut rate_limited,
                    },
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
                SectionErrorPolicy {
                    skip_errors: effective_request.skip_errors,
                    force_fail_terms: &effective_request.skip_errors_exclude,
                    log_skipped_errors: effective_request.log_skipped_errors,
                    ignore_stories_560_errors: effective_request.ignore_stories_560_errors,
                    use_gql: effective_request.use_gql,
                },
                "stories",
                SectionErrorSink {
                    section_errors: &mut section_errors,
                    validation_error: &mut validation_error,
                    auth_disabled_sections: &mut auth_disabled_sections,
                    rate_limited: &mut rate_limited,
                },
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
                SectionErrorPolicy {
                    skip_errors: effective_request.skip_errors,
                    force_fail_terms: &effective_request.skip_errors_exclude,
                    log_skipped_errors: effective_request.log_skipped_errors,
                    ignore_stories_560_errors: effective_request.ignore_stories_560_errors,
                    use_gql: effective_request.use_gql,
                },
                "stories_user",
                SectionErrorSink {
                    section_errors: &mut section_errors,
                    validation_error: &mut validation_error,
                    auth_disabled_sections: &mut auth_disabled_sections,
                    rate_limited: &mut rate_limited,
                },
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
            &should_cancel,
        ) {
            Ok(tagged_items) => {
                completed_discovery_sections += 1;
                // Aviso operacional: contagem alta de tagged costuma indicar
                // spam de marcações; o limite vem das settings da conta.
                if effective_request.tagged_notify_limit > 0
                    && tagged_items.len() > effective_request.tagged_notify_limit as usize
                {
                    section_errors.push(format!(
                        "Tagged: {} post(s) exceed the notify limit of {}.",
                        tagged_items.len(),
                        effective_request.tagged_notify_limit
                    ));
                }
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
                    SectionErrorPolicy {
                        skip_errors: effective_request.skip_errors,
                        force_fail_terms: &effective_request.skip_errors_exclude,
                        log_skipped_errors: effective_request.log_skipped_errors,
                        ignore_stories_560_errors: effective_request.ignore_stories_560_errors,
                        use_gql: effective_request.use_gql,
                    },
                    "tagged",
                    SectionErrorSink {
                        section_errors: &mut section_errors,
                        validation_error: &mut validation_error,
                        auth_disabled_sections: &mut auth_disabled_sections,
                        rate_limited: &mut rate_limited,
                    },
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
                SectionErrorPolicy {
                    skip_errors: effective_request.skip_errors,
                    force_fail_terms: &effective_request.skip_errors_exclude,
                    log_skipped_errors: effective_request.log_skipped_errors,
                    ignore_stories_560_errors: effective_request.ignore_stories_560_errors,
                    use_gql: effective_request.use_gql,
                },
                &section.media_section,
                SectionErrorSink {
                    section_errors: &mut section_errors,
                    validation_error: &mut validation_error,
                    auth_disabled_sections: &mut auth_disabled_sections,
                    rate_limited: &mut rate_limited,
                },
            )?;
        }
    }

    if !effective_request.skip_errors && downloaded_media.is_empty() && !section_errors.is_empty() {
        return Err(section_errors.join(" | "));
    }

    let downloaded_asset_count = downloaded_media.len() as u32;
    let manifest_summary =
        summarize_profile_sync_manifest(&manifest, downloaded_asset_count, Some(&profile.user_id));
    // O detalhamento técnico (contadores por seção) vai para o realtime debugger;
    // o resumo persistido/mostrado ao usuário fica curto e amigável.
    connector_debug::append_current(
        "internal.instagram",
        "summary",
        "manifest",
        format_instagram_manifest_debug(&manifest_summary),
    );

    Ok(InstagramConnectorResult {
        observed_posts: manifest_observed_posts(&manifest),
        downloaded_media,
        section_errors,
        validation_error,
        auth_disabled_sections,
        resolved_username: Some(profile.username.clone()),
        profile_description,
        profile_description_error,
        manifest_summary: Some(manifest_summary),
        highlight_memberships: collect_highlight_memberships(&manifest),
        updated_headers: client.headers.clone(),
        rate_limited,
    })
}

/// Bloco técnico legível (contadores globais + por seção) para o realtime
/// debugger. O resumo mostrado ao usuário não repete esses números.
fn format_instagram_manifest_debug(summary: &InstagramManifestSummary) -> String {
    let mut lines = vec![format!(
        "sections={} discovered_items={} normalized_posts={} discovered_assets={} queued_assets={} downloaded_assets={}",
        summary.section_count,
        summary.discovered_item_count,
        summary.normalized_post_count,
        summary.discovered_asset_count,
        summary.queued_asset_count,
        summary.downloaded_asset_count,
    )];
    lines.push(format!(
        "skipped: existing_posts={} duplicate_posts={} unavailable_posts={} existing_assets={} duplicate_assets={}",
        summary.skipped_existing_post_count,
        summary.skipped_duplicate_post_count,
        summary.skipped_unavailable_post_count,
        summary.skipped_existing_asset_count,
        summary.skipped_duplicate_asset_count,
    ));
    for section in &summary.sections {
        lines.push(format!(
            "[{}] items={} posts={} queued={} skipped(existing_post={}, dup_post={}, unavail_post={}, existing_asset={}, dup_asset={}, out_of_range={})",
            section.section,
            section.item_count,
            section.normalized_post_count,
            section.queued_asset_count,
            section.skipped_existing_post_count,
            section.skipped_duplicate_post_count,
            section.skipped_unavailable_post_count,
            section.skipped_existing_asset_count,
            section.skipped_duplicate_asset_count,
            section.skipped_out_of_range_item_count,
        ));
    }
    lines.join("\n")
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
            profile_description_error: None,
            manifest_summary: None,
            highlight_memberships: Vec::new(),
            updated_headers: request.headers.clone(),
            rate_limited: false,
        });
    }

    let mut client = InstagramClient::new(
        &request.cookies,
        request.headers.clone(),
        request.timeout_secs,
        request.pacing,
    )?;
    let items = load_saved_posts_items(&mut client, request.post_page_size, &should_cancel)?;
    let mut downloaded_media = Vec::new();
    let mut section_errors = Vec::new();
    let mut validation_error = None;
    let mut auth_disabled_sections = Vec::new();
    let mut rate_limited = false;

    ensure_sync_not_cancelled(&should_cancel)?;
    if let Err(error) = download_items_section(
        &mut client,
        request,
        SectionDownloadInput {
            media_section: "saved_posts",
            section_root: &request.saved_posts_root,
            items,
            profile_user_id: None,
        },
        &mut downloaded_media,
        &mut progress,
    ) {
        handle_section_error(
            error,
            SectionErrorPolicy {
                skip_errors: request.skip_errors,
                force_fail_terms: &request.skip_errors_exclude,
                log_skipped_errors: request.log_skipped_errors,
                ignore_stories_560_errors: request.ignore_stories_560_errors,
                use_gql: request.use_gql,
            },
            "saved_posts",
            SectionErrorSink {
                section_errors: &mut section_errors,
                validation_error: &mut validation_error,
                auth_disabled_sections: &mut auth_disabled_sections,
                rate_limited: &mut rate_limited,
            },
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
        profile_description_error: None,
        manifest_summary: None,
        highlight_memberships: Vec::new(),
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
        skipped_out_of_range_item_count: 0,
        skipped_existing_post_count: 0,
        skipped_duplicate_post_count: 0,
        skipped_unavailable_post_count: 0,
        skipped_existing_asset_count: 0,
        skipped_duplicate_asset_count: 0,
        highlight_media_keys: Vec::new(),
        posts: Vec::new(),
        observed_existing_posts: Vec::new(),
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

#[derive(Clone)]
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

        let mut out_of_range_count = 0usize;
        let filtered_items = section
            .items
            .iter()
            .filter(|item| {
                let in_range = item_matches_requested_date_range(item, request);
                if !in_range {
                    out_of_range_count += 1;
                }
                in_range
            })
            .cloned()
            .collect::<Vec<_>>();
        section.skipped_out_of_range_item_count = out_of_range_count;

        // Highlights (`stories`) costumam ter posts já registrados no post-ledger
        // (observados em syncs antigas) cuja mídia nunca foi baixada — o skip por
        // post-ledger os bloquearia para sempre. Para essas seções, decidimos a
        // re-descida pela presença do arquivo em disco (como `missing_only`), mas
        // ainda honrando as tombstones de deleção.
        let effective_missing_only = request.missing_only || section.media_section == "stories";

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
            let known_as_deleted = request
                .deleted_post_keys
                .contains(&identity.provider_post_key)
                || identity
                    .provider_post_code
                    .as_ref()
                    .is_some_and(|code| request.deleted_post_keys.contains(code));

            // Em highlights ignoramos o post-ledger (mídia faltante deve baixar),
            // mas a deleção explícita sempre suprime. Nas demais seções, o
            // post-ledger já cobre as tombstones (a deleção escreve nele).
            let post_suppressed = if effective_missing_only {
                known_as_deleted
            } else {
                known_in_post_ledger
            };
            if post_suppressed {
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

            // Registra a media key de TODO asset de highlight (baixado ou não),
            // para associar o álbum ao arquivo já existente em disco (no Feed).
            if section.media_section == "stories" {
                for asset in &assets {
                    section
                        .highlight_media_keys
                        .push(asset.provider_media_key.clone());
                }
            }

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
                let should_skip_existing = if effective_missing_only {
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
                let should_skip_resolved_destination = if effective_missing_only {
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
                if discovered_asset_count > 0 {
                    // Mídia toda já presente (disco/ledger): post completo, nada
                    // a baixar. Registra como observado para o post-ledger virar
                    // um índice completo de posts vistos — re-syncs o reconhecem
                    // pela chave estável em vez de re-processar sempre.
                    section.skipped_existing_post_count += 1;
                    section.observed_existing_posts.push(identity);
                } else {
                    // Nenhuma mídia baixável resolvida — realmente indisponível.
                    section.skipped_unavailable_post_count += 1;
                }
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
            let from_posts = section.posts.iter().map(|post| ObservedInstagramPost {
                provider_post_key: post.provider_post_key.clone(),
                provider_post_code: post.provider_post_code.clone(),
                media_section: section.media_section.clone(),
            });
            // Posts já completos em disco também são "vistos" e precisam entrar
            // no post-ledger (senão re-syncs nunca os reconhecem).
            let from_existing =
                section
                    .observed_existing_posts
                    .iter()
                    .map(|identity| ObservedInstagramPost {
                        provider_post_key: identity.provider_post_key.clone(),
                        provider_post_code: identity.provider_post_code.clone(),
                        media_section: section.media_section.clone(),
                    });
            from_posts.chain(from_existing)
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
        request.pacing,
    )?;
    let username = request.username.trim();
    let normalized_user_id_hint = user_id_hint
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let primary_error = match load_profile_identity_by_username(&mut client, username) {
        Ok(identity) => {
            let Some(expected_user_id) = normalized_user_id_hint else {
                return Ok(identity);
            };
            if identity.user_id.trim() == expected_user_id {
                return Ok(identity);
            }

            // A username can be claimed by another account after a rename. The
            // persisted numeric id is the identity anchor, so never accept the
            // newly resolved account merely because it owns the old handle.
            format!(
                "Instagram username '{username}' resolved to user id '{}', but this source is \
                 anchored to user id '{expected_user_id}'.",
                identity.user_id.trim()
            )
        }
        Err(error) => error,
    };

    let Some(user_id) = normalized_user_id_hint else {
        return Err(primary_error);
    };

    resolve_profile_identity_by_user_id_fallback(
        request,
        &mut client,
        username,
        user_id,
        &primary_error,
    )
}

fn resolve_profile_identity_by_user_id_fallback(
    request: &InstagramConnectorRequest,
    authenticated_client: &mut InstagramClient,
    username: &str,
    user_id: &str,
    primary_error: &str,
) -> Result<InstagramProfileIdentity, String> {
    // Resolver `user_id -> username` é uma consulta pública. Sessões importadas
    // degradadas (cookies expirados) fazem o endpoint retornar 400, então
    // tentamos primeiro com um cliente anônimo (sem cookies) e só recorremos à
    // sessão autenticada se a consulta pública falhar. Isso mantém os perfis
    // renomeados resolvíveis mesmo quando a sessão da conta está degradada.
    let mut public_client = InstagramClient::new(
        &[],
        public_identity_headers(&request.headers),
        request.timeout_secs,
        request.pacing,
    )?;
    match load_profile_identity_by_user_id(&mut public_client, username, user_id) {
        Ok(identity) => Ok(identity),
        Err(public_error) => {
            load_profile_identity_by_user_id(authenticated_client, username, user_id).map_err(
                |auth_error| {
                    format!(
                        "{primary_error} | fallback by user id '{user_id}' failed \
                     (public lookup: {public_error}; authenticated lookup: {auth_error})"
                    )
                },
            )
        }
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
    let payload = client.get_json(&instagram_user_info_url(user_id), Some(&referer))?;
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

const SYNC_CANCELLED_MESSAGE: &str = "source sync cancelled by user";

fn ensure_sync_not_cancelled(should_cancel: &impl Fn() -> bool) -> Result<(), String> {
    if should_cancel() {
        Err(SYNC_CANCELLED_MESSAGE.to_string())
    } else {
        Ok(())
    }
}

fn is_sync_cancelled_error(error: &str) -> bool {
    error.contains(SYNC_CANCELLED_MESSAGE)
}

/// Dorme em passos curtos, abortando assim que o cancelamento é solicitado.
/// Evita ficar preso num `thread::sleep` longo (ex.: 30s entre páginas) enquanto
/// o usuário já pediu para cancelar.
fn interruptible_sleep(total: Duration, should_cancel: &dyn Fn() -> bool) {
    const STEP: Duration = Duration::from_millis(200);
    let mut remaining = total;
    while !remaining.is_zero() {
        if should_cancel() {
            return;
        }
        let chunk = STEP.min(remaining);
        thread::sleep(chunk);
        remaining -= chunk;
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

/// Associação post→álbum para todos os itens descobertos nas seções de highlight
/// (`stories`), independentemente de terem sido baixados ou pulados por já
/// existirem no ledger. O álbum é o nome da subpasta sob `Stories/`.
fn collect_highlight_memberships(
    manifest: &InstagramSyncManifest,
) -> Vec<InstagramHighlightMembership> {
    let mut memberships = Vec::new();
    for section in &manifest.sections {
        if section.media_section != "stories" {
            continue;
        }
        let album = section
            .section_root
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        if album.is_empty() {
            continue;
        }
        for media_key in &section.highlight_media_keys {
            if media_key.trim().is_empty() {
                continue;
            }
            memberships.push(InstagramHighlightMembership {
                album: album.clone(),
                provider_media_key: media_key.clone(),
            });
        }
    }
    memberships
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
            skipped_out_of_range_item_count: section.skipped_out_of_range_item_count as u32,
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

fn instagram_user_info_url(user_id: &str) -> String {
    format!("{INSTAGRAM_WEB_ORIGIN}/api/v1/users/{user_id}/info/")
}

fn resolve_profile_description(
    client: &mut InstagramClient,
    profile: &UserProfile,
) -> (Option<String>, Option<String>) {
    let mut errors = Vec::new();

    match load_profile_description_by_user_id(client, &profile.username, &profile.user_id) {
        Ok(description) => return (description, None),
        Err(error) => errors.push(format!("user info: {error}")),
    }

    if let Some(description) = profile.description.clone() {
        return (Some(description), None);
    }

    match load_profile_description_gql(client, &profile.username, &profile.user_id) {
        Ok(description) => return (description, None),
        Err(error) => errors.push(format!("GraphQL: {error}")),
    }

    match load_profile_description(client, &profile.username) {
        Ok(description) => (description, None),
        Err(error) => {
            errors.push(format!("web profile: {error}"));
            (
                None,
                Some(format!(
                    "Instagram profile description lookup failed: {}",
                    errors.join(" | ")
                )),
            )
        }
    }
}

fn load_profile_description(
    client: &mut InstagramClient,
    username: &str,
) -> Result<Option<String>, String> {
    let referer = format!("https://www.instagram.com/{username}/");
    let payload = client.get_json(
        &format!("https://i.instagram.com/api/v1/users/web_profile_info/?username={username}"),
        Some(&referer),
    )?;

    parse_profile_description_response(&payload)
}

fn load_profile_description_by_user_id(
    client: &mut InstagramClient,
    username: &str,
    user_id: &str,
) -> Result<Option<String>, String> {
    let referer = format!("https://www.instagram.com/{username}/");
    let payload = client.get_json(&instagram_user_info_url(user_id), Some(&referer))?;

    parse_profile_description_response(&payload)
}

fn load_profile_description_gql(
    client: &mut InstagramClient,
    username: &str,
    user_id: &str,
) -> Result<Option<String>, String> {
    let (lsd, dtsg) = gql_tokens(&client.headers)
        .ok_or_else(|| "Instagram GraphQL tokens are unavailable.".to_string())?;
    let friendly_name = "PolarisProfilePageContentQuery";
    let variables = format!(
        "{{\"id\":\"{}\",\"relay_header\":false,\"render_surface\":\"PROFILE\"}}",
        escape_json(user_id)
    );
    let referer = format!("https://www.instagram.com/{username}/");
    let payload = client.post_graphql_json(
        "7381344031985950",
        &lsd,
        &dtsg,
        friendly_name,
        &variables,
        Some(&referer),
    )?;

    parse_profile_description_response(&payload)
}

#[cfg(test)]
fn parse_profile_description(payload: &Value) -> Option<String> {
    parse_profile_description_response(payload).ok().flatten()
}

fn parse_profile_description_response(payload: &Value) -> Result<Option<String>, String> {
    let user = payload
        .pointer("/data/user")
        .or_else(|| payload.pointer("/user"))
        .ok_or_else(|| "Instagram profile response is missing user data.".to_string())?;
    Ok(parse_profile_description_from_user(user))
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

/// Limite (em páginas totalmente conhecidas consecutivas) para encerrar a
/// descoberta incremental do feed. Tolera até 3 páginas de posts fixados ou já
/// vistos no topo antes de parar, espelhando o `known_page_threshold` do TikTok.
const INSTAGRAM_INCREMENTAL_KNOWN_PAGE_THRESHOLD: usize = 3;

/// Contexto de parada incremental da descoberta. O feed do Instagram vem em
/// ordem cronológica reversa, então quando encontramos páginas cheias de posts
/// já sincronizados, tudo o que vem depois também já está e podemos parar de
/// paginar em vez de re-descer o perfil inteiro.
///
/// Um post conta como "conhecido" pelo MESMO critério que a fase de download usa
/// para pular: post no post-ledger/tombstone OU toda a sua mídia já presente
/// (em disco ou no media-ledger). Ancorar só no post-ledger não basta — ele só
/// registra posts que baixaram algo, então posts com mídia já em disco (import
/// antigo, sync anterior) nunca entram nele e apareceriam como "novos" para
/// sempre.
struct IncrementalDiscoveryStop<'a> {
    active: bool,
    known_page_threshold: usize,
    request: &'a InstagramConnectorRequest,
    media_section: &'a str,
    section_root: &'a Path,
    profile_user_id: Option<&'a str>,
}

impl<'a> IncrementalDiscoveryStop<'a> {
    fn new(
        request: &'a InstagramConnectorRequest,
        media_section: &'a str,
        section_root: &'a Path,
        profile_user_id: Option<&'a str>,
    ) -> Self {
        // Full scan explícito ou `missing_only` (que re-desce o perfil para
        // preencher lacunas) desligam a parada. Sem nenhuma evidência de sync
        // anterior (1º sync) não há o que reconhecer — mantém desligado para
        // evitar trabalho inútil por item.
        let has_prior_evidence = !request.ledger_post_keys.is_empty()
            || !request.existing_media_keys.is_empty()
            || !request.ledger_media_keys.is_empty()
            || !request.existing_relative_paths.is_empty()
            || !request.ledger_relative_paths.is_empty();
        let active = !request.full_scan && !request.missing_only && has_prior_evidence;
        Self {
            active,
            known_page_threshold: INSTAGRAM_INCREMENTAL_KNOWN_PAGE_THRESHOLD,
            request,
            media_section,
            section_root,
            profile_user_id,
        }
    }

    fn identity_known(&self, item: &Value) -> bool {
        let Ok(identity) = media_item_post_identity(item) else {
            return false;
        };
        let known_in = |set: &HashSet<String>| {
            set.contains(&identity.provider_post_key)
                || identity
                    .provider_post_code
                    .as_ref()
                    .is_some_and(|code| set.contains(code))
        };
        known_in(&self.request.ledger_post_keys) || known_in(&self.request.deleted_post_keys)
    }

    /// Espelha o skip do download (caminho não-`missing_only`): a mídia já está
    /// resolvida se a chave OU o caminho relativo estão em disco ou no ledger.
    fn all_media_present(&self, item: &Value) -> bool {
        let assets = match collect_media_assets(
            std::slice::from_ref(item),
            self.request,
            self.media_section,
            self.profile_user_id,
        ) {
            Ok(assets) => assets,
            Err(_) => return false,
        };
        if assets.is_empty() {
            return false;
        }
        assets.iter().all(|asset| {
            if self
                .request
                .existing_media_keys
                .contains(&asset.provider_media_key)
                || self
                    .request
                    .ledger_media_keys
                    .contains(&asset.provider_media_key)
            {
                return true;
            }
            let base = build_destination_base_path(self.section_root, asset, self.request);
            let key = profile_relative_path_key(&self.request.profile_root, &base);
            self.request.existing_relative_paths.contains(&key)
                || self.request.ledger_relative_paths.contains(&key)
        })
    }

    /// Um item só conta para a parada da seção que de fato o processa. A
    /// timeline descarta clips (`load_timeline_items` os filtra no fim; são
    /// baixados na seção reels) e a seção reels só olha clips. Assim, clips
    /// misturados no feed de timeline não impedem a página de ser reconhecida.
    fn item_relevant(&self, item: &Value) -> bool {
        match self.media_section {
            "timeline" => !is_clip_product(item),
            "reels" => is_clip_product(item),
            _ => true,
        }
    }

    /// Diagnóstico por página (só itens relevantes à seção): conhecidos via
    /// post-ledger, via mídia em disco, desconhecidos, e amostra do 1º
    /// desconhecido (id/code/product_type).
    fn page_known_breakdown(
        &self,
        page_items: &[Value],
    ) -> (usize, usize, usize, usize, Option<String>) {
        let mut relevant = 0;
        let mut via_ledger = 0;
        let mut via_disk = 0;
        let mut unknown = 0;
        let mut first_unknown = None;
        for item in page_items {
            if !self.item_relevant(item) {
                continue;
            }
            relevant += 1;
            if self.identity_known(item) {
                via_ledger += 1;
            } else if self.all_media_present(item) {
                via_disk += 1;
            } else {
                unknown += 1;
                if first_unknown.is_none() {
                    let id = string_from_value(item.get("id"))
                        .or_else(|| string_from_value(item.get("pk")))
                        .unwrap_or_default();
                    let code = string_from_value(item.get("code")).unwrap_or_default();
                    let product_type =
                        string_from_value(item.get("product_type")).unwrap_or_default();
                    first_unknown =
                        Some(format!("id={id} code={code} product_type={product_type}"));
                }
            }
        }
        (relevant, via_ledger, via_disk, unknown, first_unknown)
    }

    /// Avalia a página E registra o diagnóstico no realtime debugger. "Totalmente
    /// conhecida" = há ≥1 item relevante à seção e todos já estão sincronizados.
    /// Página sem itens relevantes (ex.: só clips na timeline) não conta como
    /// conhecida — conservador de propósito.
    fn page_fully_known(&self, page_items: &[Value], consecutive_known_pages: usize) -> bool {
        let (relevant, via_ledger, via_disk, unknown, first_unknown) =
            self.page_known_breakdown(page_items);
        let fully_known = self.active && relevant > 0 && unknown == 0;
        connector_debug::append_current(
            "internal.instagram",
            "discovery",
            format!("{}.page", self.media_section),
            format!(
                "active={} items={} relevant={} known_ledger={} known_disk={} unknown={} fully_known={} consecutive_known={} first_unknown=[{}]",
                self.active,
                page_items.len(),
                relevant,
                via_ledger,
                via_disk,
                unknown,
                fully_known,
                consecutive_known_pages,
                first_unknown.unwrap_or_default(),
            ),
        );
        fully_known
    }
}

fn load_timeline_items(
    client: &mut InstagramClient,
    username: &str,
    profile: &UserProfile,
    page_size: u32,
    use_gql: bool,
    stop: &IncrementalDiscoveryStop<'_>,
    should_cancel: &impl Fn() -> bool,
) -> Result<Vec<Value>, String> {
    if use_gql {
        if let Some((lsd, dtsg)) = gql_tokens(&client.headers) {
            let mut items = Vec::new();
            let mut cursor = None::<String>;
            let mut consecutive_known_pages = 0usize;
            loop {
                ensure_sync_not_cancelled(should_cancel)?;
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
                let payload = match client.post_graphql_json(
                    doc_id,
                    &lsd,
                    &dtsg,
                    friendly_name,
                    &variables,
                    Some(&format!("https://www.instagram.com/{username}/")),
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

                let page_fully_known = stop.page_fully_known(&page_items, consecutive_known_pages);
                items.extend(page_items);
                if stop.active {
                    if page_fully_known {
                        consecutive_known_pages += 1;
                        if consecutive_known_pages >= stop.known_page_threshold {
                            return Ok(items);
                        }
                    } else {
                        consecutive_known_pages = 0;
                    }
                }
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
    let mut consecutive_known_pages = 0usize;
    if items.is_empty() || max_id.is_some() {
        loop {
            ensure_sync_not_cancelled(should_cancel)?;
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

            let page_fully_known = stop.page_fully_known(&page_items, consecutive_known_pages);
            items.extend(page_items);
            if stop.active {
                if page_fully_known {
                    consecutive_known_pages += 1;
                    if consecutive_known_pages >= stop.known_page_threshold {
                        break;
                    }
                } else {
                    consecutive_known_pages = 0;
                }
            }
            let next = next_max_id(&payload);
            if next.is_none() {
                break;
            }
            client.wait_between_post_pages(should_cancel);
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
    stop: &IncrementalDiscoveryStop<'_>,
    should_cancel: &impl Fn() -> bool,
) -> Result<Vec<Value>, String> {
    let mut items = profile
        .reel_items
        .clone()
        .into_iter()
        .filter(is_clip_product)
        .collect::<Vec<_>>();
    let mut max_id = None;
    let mut consecutive_known_pages = 0usize;

    if use_gql {
        if let Some((lsd, dtsg)) = gql_tokens(&client.headers) {
            let mut gql_cursor = None::<String>;
            loop {
                ensure_sync_not_cancelled(should_cancel)?;
                let variables = build_reels_gql_variables(
                    profile.user_id.as_str(),
                    page_size,
                    gql_cursor.as_deref(),
                );
                let payload = match client.post_graphql_json(
                    "7191572580905225",
                    &lsd,
                    &dtsg,
                    "PolarisProfileReelsTabContentQuery",
                    &variables,
                    Some("https://www.instagram.com/"),
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

                let page_fully_known = stop.page_fully_known(&page_items, consecutive_known_pages);
                items.extend(page_items);
                if stop.active {
                    if page_fully_known {
                        consecutive_known_pages += 1;
                        if consecutive_known_pages >= stop.known_page_threshold {
                            return Ok(items);
                        }
                    } else {
                        consecutive_known_pages = 0;
                    }
                }
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
        ensure_sync_not_cancelled(should_cancel)?;
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

        let page_fully_known = stop.page_fully_known(&page_items, consecutive_known_pages);
        items.extend(page_items);
        if stop.active {
            if page_fully_known {
                consecutive_known_pages += 1;
                if consecutive_known_pages >= stop.known_page_threshold {
                    break;
                }
            } else {
                consecutive_known_pages = 0;
            }
        }
        let next = next_max_id(&payload);
        if next.is_none() {
            break;
        }
        client.wait_between_post_pages(should_cancel);
        max_id = next;
    }

    Ok(items)
}

fn load_tagged_items(
    client: &mut InstagramClient,
    profile: &UserProfile,
    page_size: u32,
    use_gql: bool,
    should_cancel: &impl Fn() -> bool,
) -> Result<Vec<Value>, String> {
    let mut items = profile.tagged_items.clone();
    let mut max_id = None;

    if use_gql {
        if let Some((lsd, dtsg)) = gql_tokens(&client.headers) {
            let mut gql_cursor = None::<String>;
            loop {
                ensure_sync_not_cancelled(should_cancel)?;
                let variables = build_tagged_gql_variables(
                    profile.user_id.as_str(),
                    page_size,
                    gql_cursor.as_deref(),
                );
                let payload = match client.post_graphql_json(
                    "7289408964443685",
                    &lsd,
                    &dtsg,
                    "PolarisProfileTaggedTabContentQuery",
                    &variables,
                    Some("https://www.instagram.com/"),
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
        ensure_sync_not_cancelled(should_cancel)?;
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
        client.wait_between_post_pages(should_cancel);
        max_id = next;
    }

    hydrate_tagged_items(client, items)
}

fn load_saved_posts_items(
    client: &mut InstagramClient,
    page_size: u32,
    should_cancel: &impl Fn() -> bool,
) -> Result<Vec<Value>, String> {
    let mut items = Vec::new();
    let mut max_id = None;

    loop {
        ensure_sync_not_cancelled(should_cancel)?;
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
        client.wait_between_post_pages(should_cancel);
        max_id = next;
    }

    Ok(items)
}

/// Seção resolvida pronta para download: nome, pasta destino, itens da API e
/// (quando conhecido) o user id do perfil.
struct SectionDownloadInput<'a> {
    media_section: &'a str,
    section_root: &'a Path,
    items: Vec<Value>,
    profile_user_id: Option<&'a str>,
}

fn download_items_section<F>(
    client: &mut InstagramClient,
    request: &InstagramConnectorRequest,
    section: SectionDownloadInput<'_>,
    downloaded_media: &mut Vec<DownloadedInstagramMedia>,
    progress: &mut F,
) -> Result<(), String>
where
    F: FnMut(InstagramProgress),
{
    let SectionDownloadInput {
        media_section,
        section_root,
        items,
        profile_user_id,
    } = section;
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
        let mut asset_available = atomic_file::is_nonempty_file(&destination_path);
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
            append_single_asset(
                child,
                assets,
                index,
                request,
                media_section,
                post_code.as_deref(),
            )?;
        }
        return Ok(());
    }

    if let Some(children) = item.get("carousel_media").and_then(Value::as_array) {
        for (index, child) in children.iter().enumerate() {
            append_single_asset(
                child,
                assets,
                index,
                request,
                media_section,
                post_code.as_deref(),
            )?;
        }
        return Ok(());
    }

    append_single_asset(
        item,
        assets,
        0,
        request,
        media_section,
        post_code.as_deref(),
    )
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

/// Política de tolerância a erros de seção, derivada do request.
#[derive(Clone, Copy)]
struct SectionErrorPolicy<'a> {
    skip_errors: bool,
    /// Erros genéricos contendo qualquer um destes termos falham mesmo com
    /// `skip_errors` ligado.
    force_fail_terms: &'a [String],
    /// `false` pula silenciosamente (sem registrar no resumo/log).
    log_skipped_errors: bool,
    ignore_stories_560_errors: bool,
    use_gql: bool,
}

/// Acumuladores de erro compartilhados por todas as seções de um mesmo sync.
struct SectionErrorSink<'a> {
    section_errors: &'a mut Vec<String>,
    validation_error: &'a mut Option<String>,
    auth_disabled_sections: &'a mut Vec<String>,
    rate_limited: &'a mut bool,
}

fn handle_section_error(
    error: String,
    policy: SectionErrorPolicy<'_>,
    section: &str,
    sink: SectionErrorSink<'_>,
) -> Result<(), String> {
    let SectionErrorPolicy {
        skip_errors,
        force_fail_terms,
        log_skipped_errors,
        ignore_stories_560_errors,
        use_gql,
    } = policy;
    let SectionErrorSink {
        section_errors,
        validation_error,
        auth_disabled_sections,
        rate_limited,
    } = sink;
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
            if log_skipped_errors {
                section_errors.push(message);
            }
            Ok(())
        }
        SectionErrorDisposition::ForceFail => Err(message),
        SectionErrorDisposition::Generic => {
            let error_lowered = error.to_ascii_lowercase();
            let force_fail = force_fail_terms.iter().any(|term| {
                let term = term.trim();
                !term.is_empty() && error_lowered.contains(&term.to_ascii_lowercase())
            });
            if skip_errors && !force_fail {
                if log_skipped_errors {
                    section_errors.push(message);
                }
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
    // Cancelamento nunca é "erro de seção" a ser engolido por skip_errors —
    // precisa abortar o sync inteiro imediatamente.
    if is_sync_cancelled_error(error) {
        return SectionErrorDisposition::ForceFail;
    }

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

/// Builds the form-urlencoded body for a persisted GraphQL query, matching the
/// fields Instagram's web client posts. `av` (the acting user id, read from the
/// `ds_user_id` cookie) and `jazoest` (a checksum of `fb_dtsg`) are required by
/// the endpoint alongside the query itself.
fn build_graphql_body(
    doc_id: &str,
    lsd: &str,
    dtsg: &str,
    friendly_name: &str,
    variables_json: &str,
    acting_user_id: Option<&str>,
) -> String {
    let mut body = String::new();
    if let Some(av) = acting_user_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        body.push_str("av=");
        body.push_str(&percent_encode_component(av));
        body.push('&');
    }
    body.push_str("__comet_req=7&fb_dtsg=");
    body.push_str(&percent_encode_component(dtsg));
    body.push_str("&jazoest=");
    body.push_str(&percent_encode_component(&compute_jazoest(dtsg)));
    body.push_str("&lsd=");
    body.push_str(&percent_encode_component(lsd));
    body.push_str("&fb_api_caller_class=RelayModern&fb_api_req_friendly_name=");
    body.push_str(&percent_encode_component(friendly_name));
    body.push_str("&doc_id=");
    body.push_str(&percent_encode_component(doc_id));
    body.push_str("&variables=");
    body.push_str(&percent_encode_component(variables_json));
    body.push_str("&server_timestamps=true");
    body
}

/// Facebook/Instagram anti-CSRF checksum derived from `fb_dtsg`: the literal
/// `2` followed by the sum of the token's byte values.
fn compute_jazoest(token: &str) -> String {
    let sum: u32 = token.bytes().map(u32::from).sum();
    format!("2{sum}")
}

fn cookie_value(cookie_header: &str, name: &str) -> Option<String> {
    cookie_header
        .split(';')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty())
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

#[cfg(test)]
mod tests {
    use super::{
        append_single_asset, best_image_url, build_graphql_body, build_manifest_section,
        build_media_file_name, classify_section_error, collect_media_assets, compute_jazoest,
        cookie_value, execute_manifest_section, extract_reels_payload_items, interruptible_sleep,
        instagram_user_info_url, manifest_observed_posts, normalize_profile_sync_manifest,
        parse_profile_description, parse_profile_description_from_user,
        parse_profile_description_response, provider_media_identity_from_url,
        public_identity_headers, resolve_destination_path, should_ignore_media_download_error,
        DownloadedInstagramMedia, IncrementalDiscoveryStop, InstagramAuthHeaders, InstagramClient,
        InstagramConnectorRequest, InstagramManifestPost, InstagramMediaFileNamingMode,
        InstagramPacing, InstagramSectionSelection, InstagramSyncManifest, MediaAsset,
        PlannedMediaAsset, SectionErrorDisposition, SYNC_CANCELLED_MESSAGE,
    };

    #[test]
    fn compute_jazoest_sums_token_bytes_with_prefix() {
        // '1'..'5' → 49+50+51+52+53 = 255.
        assert_eq!(compute_jazoest("12345"), "2255");
    }

    #[test]
    fn cookie_value_reads_named_cookie() {
        let header = "csrftoken=abc; ds_user_id=17841400000000000; sessionid=xyz";
        assert_eq!(
            cookie_value(header, "ds_user_id").as_deref(),
            Some("17841400000000000")
        );
        assert_eq!(cookie_value(header, "missing"), None);
    }

    #[test]
    fn build_graphql_body_posts_query_fields() {
        let body = build_graphql_body(
            "123",
            "the-lsd",
            "the-dtsg",
            "PolarisProfileReelsTabContentQuery",
            "{\"target_user_id\":\"42\"}",
            Some("17841400000000000"),
        );

        assert!(body.contains("doc_id=123"));
        assert!(body.contains("fb_dtsg=the-dtsg"));
        assert!(body.contains("lsd=the-lsd"));
        assert!(body.contains("av=17841400000000000"));
        assert!(body.contains("fb_api_req_friendly_name=PolarisProfileReelsTabContentQuery"));
        // `variables` must be percent-encoded so the JSON braces survive transport.
        assert!(body.contains("variables=%7B%22target_user_id%22%3A%2242%22%7D"));
        assert!(body.contains(&format!("jazoest={}", compute_jazoest("the-dtsg"))));
    }

    #[test]
    fn build_graphql_body_omits_av_without_user_cookie() {
        let body = build_graphql_body("123", "l", "d", "Friendly", "{}", None);
        assert!(!body.contains("av="));
    }
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
            deleted_post_keys: HashSet::new(),
            existing_media_keys: HashSet::new(),
            ledger_media_keys: HashSet::new(),
            existing_relative_paths: HashSet::new(),
            ledger_relative_paths: HashSet::new(),
            sections: InstagramSectionSelection::default(),
            use_gql: false,
            download_saved_posts: false,
            post_page_size: 30,
            skip_errors: true,
            skip_errors_exclude: Vec::new(),
            log_skipped_errors: true,
            tagged_notify_limit: 0,
            ignore_stories_560_errors: false,
            pacing: InstagramPacing::none(),
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
            full_scan: false,
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
    fn incremental_stop_flags_page_fully_known_only_when_every_item_is_known() {
        let mut request = sample_request();
        request.ledger_post_keys = ["100", "200", "abc"]
            .iter()
            .map(|v| v.to_string())
            .collect();
        let root = request.profile_root.clone();

        let stop = IncrementalDiscoveryStop::new(&request, "timeline", &root, None);
        assert!(stop.active);

        // Todos conhecidos pelo post-ledger → página totalmente conhecida.
        let known_page = vec![json!({ "id": "100" }), json!({ "id": "200" })];
        assert!(stop.page_fully_known(&known_page, 0));

        // Um item novo (sem ledger e sem mídia em disco) derruba a condição.
        let mixed_page = vec![json!({ "id": "100" }), json!({ "id": "999" })];
        assert!(!stop.page_fully_known(&mixed_page, 0));

        // Casa por shortcode também (code entra na comparação).
        let code_page = vec![json!({ "id": "555", "code": "abc" })];
        assert!(stop.page_fully_known(&code_page, 0));

        // Página vazia nunca conta como conhecida (evita parada prematura).
        assert!(!stop.page_fully_known(&[], 0));
    }

    #[test]
    fn cancellation_error_always_force_fails_section_handling() {
        // Cancelamento NUNCA pode ser tratado como erro genérico de seção
        // (que skip_errors engoliria) — precisa abortar o sync inteiro.
        assert!(matches!(
            classify_section_error("timeline", SYNC_CANCELLED_MESSAGE, false),
            SectionErrorDisposition::ForceFail
        ));
        assert!(matches!(
            classify_section_error(
                "stories",
                &format!("Timeline pagination failed: {SYNC_CANCELLED_MESSAGE}"),
                true,
            ),
            SectionErrorDisposition::ForceFail
        ));
    }

    #[test]
    fn interruptible_sleep_aborts_promptly_when_cancelled() {
        let start = std::time::Instant::now();
        interruptible_sleep(std::time::Duration::from_secs(30), &|| true);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(1),
            "sleep deveria abortar quase imediatamente ao cancelar"
        );
    }

    #[test]
    fn timeline_stop_ignores_clips_mixed_into_the_feed() {
        let mut request = sample_request();
        request.ledger_post_keys = ["100", "200"].iter().map(|v| v.to_string()).collect();
        let root = request.profile_root.clone();
        let stop = IncrementalDiscoveryStop::new(&request, "timeline", &root, None);

        // Posts de timeline todos conhecidos + um clip (reel) desconhecido: o
        // clip é processado na seção reels, não deve impedir a parada da timeline.
        let page = vec![
            json!({ "id": "100" }),
            json!({ "id": "200" }),
            json!({ "id": "999", "product_type": "clips" }),
        ];
        assert!(stop.page_fully_known(&page, 0));

        // Já a seção reels só considera clips: o clip desconhecido derruba.
        let reels_stop = IncrementalDiscoveryStop::new(&request, "reels", &root, None);
        assert!(!reels_stop.page_fully_known(&page, 0));

        // Página só de clips na timeline: 0 itens relevantes → não conta como
        // conhecida (conservador).
        let only_clips = vec![json!({ "id": "999", "product_type": "clips" })];
        assert!(!stop.page_fully_known(&only_clips, 0));
    }

    #[test]
    fn on_disk_posts_are_recorded_as_observed_for_the_post_ledger() {
        let item = json!({
            "id": "1001_2002",
            "code": "AbCдEf1",
            "image_versions2": {
                "candidates": [
                    { "url": "https://cdninstagram.example/path/already-here.jpg", "width": 720 }
                ]
            }
        });

        let mut request = sample_request();
        // Descobre a media key real do item e finge que já está em disco.
        let assets = collect_media_assets(std::slice::from_ref(&item), &request, "timeline", None)
            .expect("assets should resolve");
        assert_eq!(assets.len(), 1, "item deve resolver exatamente 1 asset");
        request
            .existing_media_keys
            .insert(assets[0].provider_media_key.clone());

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
            .expect("normalize should succeed");

        let section = &manifest.sections[0];
        // Nada a baixar (mídia já em disco), mas o post foi registrado como visto.
        assert!(section.posts.is_empty(), "não deve enfileirar download");
        assert_eq!(section.observed_existing_posts.len(), 1);
        assert_eq!(section.skipped_existing_post_count, 1);
        assert_eq!(section.skipped_unavailable_post_count, 0);

        // E entra em observed_posts (→ post-ledger) pela chave estável do post.
        let observed = manifest_observed_posts(&manifest);
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].provider_post_key, "1001_2002");
    }

    #[test]
    fn genuinely_unavailable_posts_are_not_recorded_as_observed() {
        // Item sem nenhuma mídia baixável (discovered_asset_count == 0).
        let item = json!({ "id": "3003_4004", "code": "NoMedia" });

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
        normalize_profile_sync_manifest(&request, &mut manifest, &mut |_| {}, &|| false)
            .expect("normalize should succeed");

        let section = &manifest.sections[0];
        assert!(section.observed_existing_posts.is_empty());
        assert_eq!(section.skipped_unavailable_post_count, 1);
        assert!(manifest_observed_posts(&manifest).is_empty());
    }

    #[test]
    fn incremental_stop_is_inactive_for_full_scan_and_first_sync() {
        // Full scan explícito desliga a parada mesmo com ledger populado.
        let mut full = sample_request();
        full.ledger_post_keys = ["100"].iter().map(|v| v.to_string()).collect();
        full.full_scan = true;
        let root = full.profile_root.clone();
        let stop_full = IncrementalDiscoveryStop::new(&full, "timeline", &root, None);
        assert!(!stop_full.active);
        assert!(!stop_full.page_fully_known(&[json!({ "id": "100" })], 0));

        // Primeiro sync (sem nenhuma evidência) também fica inativo.
        let first = sample_request();
        let first_root = first.profile_root.clone();
        let stop_first = IncrementalDiscoveryStop::new(&first, "timeline", &first_root, None);
        assert!(!stop_first.active);

        // `missing_only` desliga (precisa re-descer para preencher lacunas).
        let mut missing = sample_request();
        missing.ledger_post_keys = ["100"].iter().map(|v| v.to_string()).collect();
        missing.missing_only = true;
        let missing_root = missing.profile_root.clone();
        let stop_missing = IncrementalDiscoveryStop::new(&missing, "timeline", &missing_root, None);
        assert!(!stop_missing.active);
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
    fn instagram_user_info_uses_authenticated_web_origin() {
        assert_eq!(
            instagram_user_info_url("2069513520"),
            "https://www.instagram.com/api/v1/users/2069513520/info/"
        );
    }

    #[test]
    fn profile_description_response_rejects_payload_without_user_data() {
        let error = parse_profile_description_response(&json!({
            "message": "Please wait a few minutes before you try again.",
            "status": "fail"
        }))
        .expect_err("an error payload must not look like an empty biography");

        assert_eq!(error, "Instagram profile response is missing user data.");
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
        // O prefixo usa o fuso local; calcula o esperado da mesma forma para o
        // teste não depender do timezone da máquina.
        let expected_prefix = chrono::TimeZone::timestamp_opt(&chrono::Local, 1_711_800_191, 0)
            .single()
            .expect("timestamp should resolve")
            .format("%Y-%m-%d %H.%M.%S")
            .to_string();
        assert_eq!(
            file_name,
            format!("{expected_prefix} 3339838382976122123_46124578107.mp4")
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

        // Um post cujos assets caíram todos no ledger não enfileira download,
        // mas conta como post existente e é registrado como observado para o
        // post-ledger (não fica um post vazio em `posts`).
        assert_eq!(default_manifest.sections[0].posts.len(), 0);
        assert_eq!(default_manifest.sections[0].skipped_existing_asset_count, 1);
        assert_eq!(default_manifest.sections[0].skipped_existing_post_count, 1);
        assert_eq!(
            default_manifest.sections[0].skipped_unavailable_post_count,
            0
        );
        assert_eq!(
            default_manifest.sections[0].observed_existing_posts.len(),
            1
        );

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

        assert_eq!(manifest.sections[0].posts.len(), 0);
        assert_eq!(manifest.sections[0].skipped_existing_asset_count, 1);
        // Post com mídia já em disco: registrado como existente/observado, não
        // como indisponível.
        assert_eq!(manifest.sections[0].skipped_existing_post_count, 1);
        assert_eq!(manifest.sections[0].skipped_unavailable_post_count, 0);
        assert_eq!(manifest.sections[0].observed_existing_posts.len(), 1);

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
            skipped_out_of_range_item_count: 0,
            skipped_existing_post_count: 0,
            skipped_duplicate_post_count: 0,
            skipped_unavailable_post_count: 0,
            skipped_existing_asset_count: 0,
            skipped_duplicate_asset_count: 0,
            highlight_media_keys: Vec::new(),
            observed_existing_posts: Vec::new(),
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
        let mut client = InstagramClient::new(
            &[],
            InstagramAuthHeaders::default(),
            1,
            InstagramPacing::none(),
        )
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
