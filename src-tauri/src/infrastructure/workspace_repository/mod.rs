use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use image::{imageops::FilterType, GenericImageView};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::{hash_map::DefaultHasher, BTreeSet, HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Read};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration as StdDuration;
use uuid::Uuid;

use crate::domain::models::{
    default_tiktok_source_sync_options, default_twitter_source_sync_options,
};
use crate::domain::models::{
    AccountSyncRun, AppSetting, AppSettingUpsert, BatchSourceProfilePatch,
    CloneSyncPlanInput,
    CompanionAccountCandidate, CompanionAccountCapture, CompanionAccountImportInput,
    CompanionAccountImportResult, CompanionAccountPreview, DesktopRuntimeState,
    ImportMethodDescriptor, ImportPreview, ImportPreviewOptions, ImportPreviewProfile,
    ImportPreviewSummary, ImportProblem, ImportProviderDescriptor, ImportRootDescriptor,
    ImportRunProfileResult, ImportRunRequest, ImportRunResult, InstagramNamingLedgerBackfillResult,
    InstagramSourceSyncOptions, MediaGalleryFile, MediaGalleryPost,
    MediaThumbnailBatch, MoveSyncPlanInput, ProviderAccount, ProviderAccountCookie,
    ProviderAccountCookieImport, ProviderAccountEditor, ProviderAccountImportState,
    ProviderAccountSession, ProviderAccountSettingValue, ProviderAccountSettingValueKind,
    ProviderAccountUpsert, RunSyncPlanNowInput, RuntimeLogContext, RuntimeLogEntry,
    RuntimeLogQuery, SchedulerGroup, SchedulerGroupUpsert, SchedulerPlanCriteria,
    SchedulerPlanNotifications, SchedulerSet, SchedulerSetUpsert, SetSyncPlanPauseInput,
    SingleVideo, SingleVideoFile, SkipSyncPlanInput, SourceAvailabilityCheckItem,
    SourceAvailabilityCheckResult, SourceMediaGallery, SourceProfile, SourceProfileDeleteMode,
    SourceProfileUpsert, SourceSyncOptions, SourceSyncRun, SyncPlan, SyncPlanRun,
    SyncPlanTargetPreview, SyncPlanTargetPreviewInput, SyncPlanTargetPreviewSource, SyncPlanUpsert,
    TikTokSourceSyncOptions, TwitterSourceSyncOptions, WorkspaceSnapshot,
};
use crate::infrastructure::runtime_log::RuntimeLogAnchor;
use crate::infrastructure::storage::StorageLayout;
use crate::infrastructure::{
    connector_debug, connector_runtime, database, instagram_connector, runtime_log,
    session_secret_store, source_sync_runtime, storage, tiktok_connector, tiktok_likes_runtime,
    twitter_connector,
};
use crate::providers;

// Submódulos extraídos mecanicamente do antigo arquivo único de 21k linhas.
// Cada um faz `use super::*;` e devolve seus nomes ao namespace do módulo via
// o glob re-export abaixo, então as chamadas internas não mudam.
mod accounts;
mod avatar;
mod gallery;
mod import;
mod media;
mod options;
mod paths;
mod scheduler;
mod settings;
mod sources;
mod sync;
pub use accounts::*;
pub use avatar::*;
pub use gallery::*;
pub use import::*;
pub use media::*;
pub(crate) use options::*;
use paths::*;
pub use scheduler::*;
pub use settings::*;
pub use sources::*;
pub use sync::*;

pub const DESKTOP_CLOSE_TO_TRAY_SETTING_KEY: &str = "policy.desktop.close_to_tray";
pub const DESKTOP_SILENT_MODE_SETTING_KEY: &str = "policy.desktop.silent_mode";
const INSTAGRAM_AVATAR_RETRY_AFTER_FALLBACK_SECS: u64 = 20 * 60;
const INSTAGRAM_AVATAR_COOLDOWN_UNTIL_SETTING_KEY: &str = "instagram.avatar.cooldownUntil";
const INSTAGRAM_PUBLIC_APP_ID: &str = "936619743392459";
const INSTAGRAM_PUBLIC_ASBD_ID: &str = "129477";
const INSTAGRAM_PUBLIC_IG_CLAIM: &str = "0";
const INSTAGRAM_PUBLIC_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";
const INSTAGRAM_NAMING_LEDGER_BACKFILL_SETTING_KEY: &str =
    "runtime.instagram.naming_ledger_backfilled_v1";
const INSTAGRAM_SCRAWLER_IMPORTER_ID: &str = "instagram.scrawler";
/// Toggle (Settings) que liga/desliga o cancelamento+remoção automática quando
/// um perfil novo resolve, no primeiro sync, para um user id já cadastrado.
const DUPLICATE_USER_ID_BLOCK_SETTING_KEY: &str = "policy.sync.blockDuplicateUserId";
/// Segundos de espera entre cada download da fila (throttle global). Protege
/// provedores com rate limit baixo (ex.: Twitter). 0 = sem espera.
const SYNC_DELAY_BETWEEN_PROFILES_SETTING_KEY: &str = "policy.sync.delayBetweenProfilesSecs";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Default)]
struct InstagramMediaLedgerSnapshot {
    media_keys: HashSet<String>,
    relative_paths: HashSet<String>,
}

#[derive(Default)]
struct InstagramPostLedgerSnapshot {
    keys: HashSet<String>,
}

#[derive(Clone, Default)]
pub struct InstagramNamingLedgerBackfillProgress {
    pub processed_sources: u32,
    pub total_sources: u32,
    pub source_id: Option<String>,
    pub source_handle: Option<String>,
    pub scanned_files: u32,
    pub inserted_entries: u32,
    pub updated_entries: u32,
    pub skipped_files: u32,
    pub legacy_records_total: u32,
    pub legacy_records_matched: u32,
}

fn twitter_model_selection(
    options: &TwitterSourceSyncOptions,
) -> twitter_connector::TwitterModelSelection {
    twitter_connector::TwitterModelSelection {
        media: options.media_model.unwrap_or(true),
        profile: options.profile_model.unwrap_or(true),
        search: options.search_model.unwrap_or(false),
        likes: options.likes_model.unwrap_or(false),
    }
}

fn twitter_model_selection_for_run(
    options: &TwitterSourceSyncOptions,
    run_mode: Option<&str>,
) -> twitter_connector::TwitterModelSelection {
    if run_mode
        .is_some_and(|value| value.eq_ignore_ascii_case(TWITTER_FULL_TIMELINE_BACKFILL_RUN_MODE))
    {
        return twitter_connector::TwitterModelSelection {
            media: false,
            profile: true,
            search: false,
            likes: false,
        };
    }

    let mut selection = twitter_model_selection(options);
    // A timeline completa sobrepoe o modelo media e portanto nunca participa
    // do sync normal. Ela e executada somente no run mode de backfill acima.
    selection.profile = false;
    selection
}

fn twitter_model_selection_for_phase(
    options: &TwitterSourceSyncOptions,
    run_mode: Option<&str>,
    completed_sections: &HashSet<String>,
) -> twitter_connector::TwitterModelSelection {
    let mut selection = twitter_model_selection_for_run(options, run_mode);
    selection.media &= !completed_sections.contains("media");
    selection.profile &= !completed_sections.contains("timeline");
    selection.search &= !completed_sections.contains("search");
    selection.likes &= !completed_sections.contains("likes");
    selection
}

fn tiktok_section_selection(
    options: &TikTokSourceSyncOptions,
) -> tiktok_connector::TikTokSectionSelection {
    tiktok_connector::TikTokSectionSelection {
        timeline: options.get_timeline.unwrap_or(true),
        stories: options.get_stories_user.unwrap_or(false),
        reposts: options.get_reposts.unwrap_or(false),
    }
}

/// Grava (append-only) a participação de posts em álbuns de highlight. Mantém
/// `first_seen_at` no conflito e nunca remove linhas — é um snapshot: o item
/// permanece no álbum mesmo se for retirado do destaque online depois.
fn upsert_instagram_highlight_memberships(
    connection: &Connection,
    source_id: &str,
    memberships: &[instagram_connector::InstagramHighlightMembership],
    seen_at: &str,
) -> Result<(), String> {
    if memberships.is_empty() {
        return Ok(());
    }
    let mut statement = connection
        .prepare(
            "INSERT INTO instagram_highlight_media_membership
                 (source_id, provider_media_key, album, first_seen_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(source_id, provider_media_key, album) DO UPDATE SET
                 last_seen_at = excluded.last_seen_at",
        )
        .map_err(|error| error.to_string())?;
    for membership in memberships {
        let media_key = membership.provider_media_key.trim();
        let album = membership.album.trim();
        if media_key.is_empty() || album.is_empty() {
            continue;
        }
        statement
            .execute(rusqlite::params![source_id, media_key, album, seen_at])
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Mapa media key → álbuns de highlight. A galeria resolve o álbum casando a
/// media key de cada arquivo em disco (mesmo método de `existing_media_keys`),
/// então a mídia que vive no Feed aparece sob o destaque. Best-effort: tabela
/// ausente ou query falha → vazio (sem regressão).
fn load_instagram_highlight_membership(
    connection: &Connection,
    source_id: &str,
) -> HashMap<String, BTreeSet<String>> {
    let mut map: HashMap<String, BTreeSet<String>> = HashMap::new();
    let Ok(mut statement) = connection.prepare(
        "SELECT provider_media_key, album
         FROM instagram_highlight_media_membership WHERE source_id = ?1",
    ) else {
        return map;
    };
    let Ok(rows) = statement.query_map([source_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return map;
    };
    for (media_key, album) in rows.flatten() {
        let media_key = media_key.trim();
        let album = album.trim();
        if media_key.is_empty() || album.is_empty() {
            continue;
        }
        map.entry(media_key.to_string())
            .or_default()
            .insert(album.to_string());
    }
    map
}

fn log_runtime_event(
    layout: &StorageLayout,
    scope: &str,
    level: &str,
    context: RuntimeLogAnchor<'_>,
    message: impl Into<String>,
    detail: Option<String>,
) {
    let _ = runtime_log::append(layout, scope, level, context, message, detail);
}

pub fn bootstrap_workspace() -> Result<WorkspaceSnapshot, String> {
    with_workspace(load_snapshot)
}

pub fn load_all_asset_media_paths() -> Result<Vec<PathBuf>, String> {
    with_workspace(|connection, layout| {
        let mut paths: Vec<PathBuf> = Vec::new();
        paths.push(layout.media_root.clone());

        // mediaPath de cada conta (todos os providers usam `<provider>.account.mediaPath`).
        for account in load_accounts(connection)? {
            let settings = load_provider_account_settings_map(connection, &account.id)?;
            let key = format!(
                "{}.account.mediaPath",
                account.provider.to_ascii_lowercase()
            );
            if let Some(media_path) = setting_value(&settings, &key) {
                let trimmed = media_path.trim();
                if !trimmed.is_empty() {
                    paths.push(PathBuf::from(trimmed));
                }
            }
        }

        // `specialPath` por perfil: aponta a mídia para fora do media root (ex.:
        // perfis importados do 4K Tokkit em `S:\4K Tokkit\<handle>`). Sem isto, o
        // protocolo de asset bloqueia o avatar/preview desses perfis.
        let mut statement = connection
            .prepare(
                "SELECT sync_options_json FROM source_profiles
                 WHERE deleted_at IS NULL AND sync_options_json IS NOT NULL AND sync_options_json != ''",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        for row in rows {
            let json = row.map_err(|error| error.to_string())?;
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) else {
                continue;
            };
            for provider in ["instagram", "twitter", "tiktok"] {
                if let Some(special_path) = value
                    .get(provider)
                    .and_then(|provider_value| provider_value.get("specialPath"))
                    .and_then(|special| special.as_str())
                {
                    let trimmed = special_path.trim();
                    if !trimmed.is_empty() {
                        paths.push(PathBuf::from(trimmed));
                    }
                }
            }
        }

        paths.sort();
        paths.dedup();
        Ok(paths)
    })
}

pub fn desktop_runtime_state() -> Result<DesktopRuntimeState, String> {
    with_workspace(|connection, _| load_desktop_runtime_state(connection))
}

pub fn set_desktop_close_to_tray(enabled: bool) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        upsert_app_setting_value(
            connection,
            DESKTOP_CLOSE_TO_TRAY_SETTING_KEY,
            bool_setting_value(enabled),
        )?;
        load_snapshot(connection, layout)
    })
}

pub fn set_desktop_silent_mode(enabled: bool) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        upsert_app_setting_value(
            connection,
            DESKTOP_SILENT_MODE_SETTING_KEY,
            bool_setting_value(enabled),
        )?;
        load_snapshot(connection, layout)
    })
}

pub fn query_runtime_logs(input: RuntimeLogQuery) -> Result<Vec<RuntimeLogEntry>, String> {
    let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    runtime_log::query(&layout, input)
}

pub fn load_runtime_log_context() -> Result<RuntimeLogContext, String> {
    with_workspace(|connection, _layout| {
        Ok(RuntimeLogContext {
            provider_catalog: providers::provider_catalog(),
            accounts: load_accounts(connection)?,
        })
    })
}

/// TikTok codifica o unix de criação nos bits altos do id (`id >> 32`).
fn gallery_timestamp_from_tiktok_id(post_id: &str) -> Option<i64> {
    let id = post_id.trim().parse::<u64>().ok()?;
    let seconds = (id >> 32) as i64;
    (1_400_000_000..4_000_000_000)
        .contains(&seconds)
        .then_some(seconds)
}

/// Map `basename (lowercased) → autor` dos likes do TikTok. A pasta de likes
/// guarda o vídeo de outra pessoa (`<uploader>_<videoId>.<ext>`); o autor real
/// vive no `account_sync_media_ledger` (sync_scope `liked_videos`), cujo
/// `relative_path` é só o nome do arquivo. Vazio para outros providers.
fn load_tiktok_like_authors(connection: &Connection, provider: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if !provider.eq_ignore_ascii_case("tiktok") {
        return map;
    }
    let Ok(mut statement) = connection.prepare(
        "SELECT relative_path, source_handle FROM account_sync_media_ledger
         WHERE provider = 'tiktok' AND sync_scope = 'liked_videos'",
    ) else {
        return map;
    };
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    });
    if let Ok(rows) = rows {
        for (relative_path, handle) in rows.flatten() {
            let handle = handle.trim();
            if handle.is_empty() {
                continue;
            }
            let basename = relative_path
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&relative_path);
            map.insert(basename.to_ascii_lowercase(), handle.to_string());
        }
    }
    map
}

fn run_windows_command(executable: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(executable)
        .args(args)
        .status()
        .map_err(|error| format!("Failed to launch '{}': {}", executable, error))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "'{}' exited with status {}.",
            executable,
            status
                .code()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        ))
    }
}

fn with_workspace<T, F>(operation: F) -> Result<T, String>
where
    F: FnOnce(&Connection, &StorageLayout) -> Result<T, String>,
{
    let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    with_workspace_layout(layout, operation)
}

fn with_workspace_layout<T, F>(layout: StorageLayout, operation: F) -> Result<T, String>
where
    F: FnOnce(&Connection, &StorageLayout) -> Result<T, String>,
{
    let connection =
        database::open_connection(&layout.db_path).map_err(|error| error.to_string())?;
    migrate_legacy_setting_keys(&connection)?;
    seed_missing_app_settings(&connection, &layout)?;
    migrate_media_root_setting_to_scrawler_pattern(&connection)?;
    let effective_layout = resolve_effective_storage_layout(&connection, &layout)?;
    operation(&connection, &effective_layout)
}

fn resolve_effective_storage_layout(
    connection: &Connection,
    layout: &StorageLayout,
) -> Result<StorageLayout, String> {
    let mut effective_layout = layout.clone();
    if let Some(media_root_setting) = load_app_setting_value(connection, "storage.media_root")? {
        let trimmed = media_root_setting.trim();
        if !trimmed.is_empty() {
            effective_layout.media_root = PathBuf::from(trimmed);
        }
    }

    fs::create_dir_all(&effective_layout.media_root).map_err(|error| error.to_string())?;
    Ok(effective_layout)
}

const SINGLE_VIDEOS_ROOT_SETTING_KEY: &str = "storage.single_videos_root";

fn single_video_url_host(url: &str) -> String {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let host = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    host.trim().trim_start_matches("www.").to_ascii_lowercase()
}

/// Provider suportado na captura de vídeo por URL (detectado pelo host).
fn detect_single_video_provider(url: &str) -> Option<&'static str> {
    let host = single_video_url_host(url);
    if host == "tiktok.com" || host.ends_with(".tiktok.com") {
        Some("tiktok")
    } else if host == "instagram.com" || host.ends_with(".instagram.com") {
        Some("instagram")
    } else if host == "x.com"
        || host.ends_with(".x.com")
        || host == "twitter.com"
        || host.ends_with(".twitter.com")
    {
        Some("twitter")
    } else if host == "youtube.com" || host.ends_with(".youtube.com") || host == "youtu.be" {
        Some("youtube")
    } else {
        None
    }
}

/// Raiz "Single videos" (setting `storage.single_videos_root`; default
/// `<media_root>/Single videos`). Garante a pasta criada.
fn single_videos_root(connection: &Connection, layout: &StorageLayout) -> Result<PathBuf, String> {
    if let Some(setting) = load_app_setting_value(connection, SINGLE_VIDEOS_ROOT_SETTING_KEY)? {
        let trimmed = setting.trim();
        if !trimmed.is_empty() {
            let root = PathBuf::from(trimmed);
            fs::create_dir_all(&root).map_err(|error| error.to_string())?;
            return Ok(root);
        }
    }
    let effective = resolve_effective_storage_layout(connection, layout)?;
    let root = effective.media_root.join("Single videos");
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    Ok(root)
}

fn single_video_meta_field(fields: &mut std::str::Split<'_, char>) -> Option<String> {
    fields
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "NA")
        .map(str::to_string)
}

struct SingleVideoDownloadResult {
    absolute_path: PathBuf,
    provider_video_id: Option<String>,
    uploader: Option<String>,
    title: Option<String>,
    media_type: String,
    audio_path: Option<PathBuf>,
    captured_at: Option<i64>,
}

#[derive(Debug, PartialEq, Eq)]
enum SingleVideoUrlKind {
    Video,
    TikTokPhoto { handle: String, post_id: String },
}

fn url_segment_after<'a>(url: &'a str, marker: &str) -> Option<&'a str> {
    url.split_once(marker)
        .map(|(_, rest)| rest)
        .and_then(|rest| rest.split(['?', '#', '/']).next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn single_video_url_kind(provider: &str, url: &str) -> SingleVideoUrlKind {
    if provider == "tiktok" {
        if let (Some(handle), Some(post_id)) = (
            url_segment_after(url, "/@").and_then(|value| value.split('/').next()),
            url_segment_after(url, "/photo/"),
        ) {
            return SingleVideoUrlKind::TikTokPhoto {
                handle: handle.trim_start_matches('@').to_string(),
                post_id: post_id.to_string(),
            };
        }
    }
    SingleVideoUrlKind::Video
}

/// Baixa UM vídeo por URL via yt-dlp (`--impersonate` para TikTok) para `dest_dir`
/// e devolve o caminho final + metadados. Usado pelos vídeos avulsos e pelo
/// download direcionado de story num perfil.
fn run_yt_dlp_video_download(
    connection: &Connection,
    layout: &StorageLayout,
    url: &str,
    provider: &str,
    dest_dir: &Path,
) -> Result<SingleVideoDownloadResult, String> {
    fs::create_dir_all(dest_dir).map_err(|error| error.to_string())?;
    let yt_dlp = connector_runtime::resolve_connector_executable(connection, layout, "yt-dlp")?;

    let output_template = format!(
        "{}/%(uploader,uploader_id,id)s_%(id)s.%(ext)s",
        dest_dir.to_string_lossy().replace('\\', "/")
    );
    let mut command = Command::new(&yt_dlp);
    configure_background_command(&mut command);
    command
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8");
    command
        .arg("--no-playlist")
        .arg("--no-simulate")
        .arg("--no-warnings")
        .arg("--ignore-errors")
        .arg("--no-cookies-from-browser")
        .arg("--no-mtime")
        .arg("--socket-timeout")
        .arg("30")
        .arg("--retries")
        .arg("5")
        .arg("--extractor-retries")
        .arg("3");
    // TikTok exige impersonation de TLS (curl_cffi); os demais não precisam.
    if provider == "tiktok" {
        command.arg("--impersonate").arg("chrome");
    }
    command
        .arg("-o")
        .arg(&output_template)
        .arg("--print")
        .arg("SVMETA\t%(id)s\t%(uploader,uploader_id)s\t%(title)s\t%(timestamp)s")
        .arg("--print")
        .arg("after_move:SVPATH\t%(filepath)s")
        .arg(url);

    let output = command
        .output()
        .map_err(|error| format!("Failed to run yt-dlp: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let mut meta_line: Option<String> = None;
    let mut file_path: Option<String> = None;
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("SVMETA\t") {
            meta_line = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("SVPATH\t") {
            file_path = Some(rest.trim().to_string());
        }
    }

    let file_path = file_path.filter(|value| !value.is_empty()).ok_or_else(|| {
        let detail = stderr.trim();
        if detail.is_empty() {
            "yt-dlp did not download the video.".to_string()
        } else {
            format!("yt-dlp could not download the video: {detail}")
        }
    })?;
    let absolute_path = PathBuf::from(&file_path);
    if !absolute_path.exists() {
        return Err(format!(
            "Downloaded file was not found on disk: {file_path}"
        ));
    }

    let mut fields = meta_line.as_deref().unwrap_or("").split('\t');
    let provider_video_id = single_video_meta_field(&mut fields);
    let uploader = single_video_meta_field(&mut fields);
    let title = single_video_meta_field(&mut fields);
    let captured_at = fields
        .next()
        .and_then(|value| value.trim().parse::<i64>().ok());

    Ok(SingleVideoDownloadResult {
        absolute_path,
        provider_video_id,
        uploader,
        title,
        media_type: "video".to_string(),
        audio_path: None,
        captured_at,
    })
}

fn extract_tiktok_rehydration_json(body: &str) -> Option<serde_json::Value> {
    let marker = "__UNIVERSAL_DATA_FOR_REHYDRATION__\"";
    let start = body.find(marker)? + marker.len();
    let rest = &body[start..];
    let json_start = rest.find('{')?;
    let json_slice = &rest[json_start..];
    let mut depth = 0_i32;
    let mut in_string = false;
    let mut escaped = false;
    let mut end = None;
    for (index, ch) in json_slice.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(index + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    serde_json::from_str(&json_slice[..end?]).ok()
}

fn tiktok_item_from_rehydration(value: &serde_json::Value) -> Option<&serde_json::Value> {
    value
        .get("__DEFAULT_SCOPE__")
        .and_then(|scope| scope.get("webapp.video-detail"))
        .and_then(|detail| detail.get("itemInfo"))
        .and_then(|info| info.get("itemStruct"))
}

fn json_string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn parse_json_unix_timestamp(value: &serde_json::Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        value
            .as_str()
            .and_then(|raw| raw.trim().parse::<i64>().ok())
    })
}

fn tiktok_photo_file_name(post_id: &str, index: usize, count: usize) -> String {
    let pad = count.to_string().len().max(3);
    if count > 1 {
        format!("{post_id}_{:0width$}.jpg", index + 1, width = pad)
    } else {
        format!("{post_id}.jpg")
    }
}

fn is_single_video_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "webp" | "heic" | "gif"
            )
        })
        .unwrap_or(false)
}

fn query_param_value<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    let query = url.split_once('?')?.1.split('#').next().unwrap_or("");
    query.split('&').find_map(|part| {
        let (candidate, value) = part.split_once('=')?;
        (candidate == key).then_some(value)
    })
}

fn requested_tiktok_image_index(url: &str, count: usize) -> usize {
    query_param_value(url, "image_index")
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .map(|value| value - 1)
        .filter(|index| *index < count)
        .unwrap_or(0)
}

fn single_video_slideshow_paths(root: &Path, post_id: &str) -> Vec<PathBuf> {
    let prefix = format!("{post_id}_");
    let single_name = format!("{post_id}.jpg");
    let mut paths: Vec<PathBuf> = fs::read_dir(root)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.flatten())
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| is_single_video_image_path(path))
        .filter(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|file_name| file_name == single_name || file_name.starts_with(&prefix))
        })
        .collect();
    paths.sort();
    paths
}

fn single_video_display_relative_path(
    root: &Path,
    relative_path: &str,
    media_type: &str,
    source_url: &str,
    provider_video_id: Option<&str>,
) -> String {
    if media_type == "slideshow" {
        if let Some(post_id) = provider_video_id {
            let paths = single_video_slideshow_paths(root, post_id);
            if !paths.is_empty() {
                let index = requested_tiktok_image_index(source_url, paths.len());
                if let Some(path) = paths.get(index) {
                    return path
                        .strip_prefix(root)
                        .unwrap_or(path)
                        .to_string_lossy()
                        .replace('\\', "/");
                }
            }
        }
    }
    relative_path.to_string()
}

fn single_video_files(
    root: &Path,
    relative_path: &str,
    media_type: &str,
    provider_video_id: Option<&str>,
) -> Vec<SingleVideoFile> {
    let paths = if media_type == "slideshow" {
        provider_video_id
            .map(|post_id| single_video_slideshow_paths(root, post_id))
            .filter(|paths| !paths.is_empty())
            .unwrap_or_else(|| vec![root.join(relative_path)])
    } else {
        vec![root.join(relative_path)]
    };

    paths
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            SingleVideoFile {
                relative_path: relative,
                absolute_path: path.to_string_lossy().to_string(),
                media_type: if media_type == "video" {
                    "video".to_string()
                } else {
                    "image".to_string()
                },
            }
        })
        .collect()
}

fn single_video_audio_path(root: &Path, provider_video_id: Option<&str>) -> Option<PathBuf> {
    let post_id = provider_video_id?;
    let prefix = format!("{post_id}_audio.");
    let mut paths: Vec<PathBuf> = fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|file_name| file_name.starts_with(&prefix))
        })
        .collect();
    paths.sort();
    paths.into_iter().next()
}

fn single_video_audio_relative_path(root: &Path, audio_path: Option<&Path>) -> Option<String> {
    audio_path.map(|path| {
        path.strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    })
}

fn run_tiktok_photo_download(
    connection: &Connection,
    layout: &StorageLayout,
    url: &str,
    dest_dir: &Path,
    handle: &str,
    post_id: &str,
) -> Result<SingleVideoDownloadResult, String> {
    fs::create_dir_all(dest_dir).map_err(|error| error.to_string())?;
    let yt_dlp = connector_runtime::resolve_connector_executable(connection, layout, "yt-dlp")?;
    let cache_dir = layout
        .cache_root
        .join(format!("single-video-photo-{post_id}"));
    let _ = fs::remove_dir_all(&cache_dir);
    fs::create_dir_all(&cache_dir).map_err(|error| error.to_string())?;

    let fetch_url = format!(
        "https://www.tiktok.com/@{}/video/{post_id}",
        handle.trim_start_matches('@')
    );
    let mut command = Command::new(&yt_dlp);
    configure_background_command(&mut command);
    command
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .arg("--ignore-errors")
        .arg("--no-warnings")
        .arg("--impersonate")
        .arg("chrome")
        .arg("--skip-download")
        .arg("--write-pages")
        .arg("--no-cookies-from-browser")
        .arg(&fetch_url)
        .current_dir(&cache_dir);

    let output = command
        .output()
        .map_err(|error| format!("Failed to run yt-dlp: {error}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let dump_path = fs::read_dir(&cache_dir)
        .map_err(|error| error.to_string())?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(|ext| ext.eq_ignore_ascii_case("dump") || ext.eq_ignore_ascii_case("html"))
                    .unwrap_or(false)
        });
    let Some(dump_path) = dump_path else {
        let _ = fs::remove_dir_all(&cache_dir);
        let detail = stderr
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("no page written")
            .trim();
        return Err(format!(
            "TikTok photo post page could not be fetched for {url}: {detail}"
        ));
    };

    let html = fs::read_to_string(&dump_path).map_err(|error| error.to_string())?;
    let json = extract_tiktok_rehydration_json(&html)
        .ok_or_else(|| "TikTok photo post rehydration data was not found.".to_string())?;
    let item = tiktok_item_from_rehydration(&json)
        .ok_or_else(|| "TikTok photo post metadata was not found.".to_string())?;
    let captured_at = item
        .get("createTime")
        .and_then(parse_json_unix_timestamp)
        .filter(|value| *value > 0)
        .or_else(|| gallery_timestamp_from_tiktok_id(post_id));
    let title = json_string_field(item, "desc");
    let uploader = item
        .get("author")
        .and_then(|author| json_string_field(author, "uniqueId"))
        .or_else(|| Some(handle.trim_start_matches('@').to_string()));
    let image_urls: Vec<String> = item
        .get("imagePost")
        .and_then(|image_post| image_post.get("images"))
        .and_then(serde_json::Value::as_array)
        .map(|images| {
            images
                .iter()
                .filter_map(|image| {
                    image
                        .get("imageURL")
                        .and_then(|node| node.get("urlList"))
                        .and_then(serde_json::Value::as_array)
                        .and_then(|list| list.first())
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    let _ = fs::remove_dir_all(&cache_dir);

    if image_urls.is_empty() {
        return Err("TikTok photo post did not expose any image URLs.".to_string());
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(StdDuration::from_secs(120))
        .user_agent(INSTAGRAM_PUBLIC_USER_AGENT)
        .build()
        .map_err(|error| error.to_string())?;
    let mut first_path: Option<PathBuf> = None;
    let mut saved_paths: Vec<PathBuf> = Vec::new();
    let count = image_urls.len();
    for (index, image_url) in image_urls.iter().enumerate() {
        let response = client
            .get(image_url)
            .send()
            .map_err(|error| format!("Failed to download TikTok photo image: {error}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "TikTok photo image request failed with HTTP {}.",
                response.status()
            ));
        }
        let bytes = response.bytes().map_err(|error| error.to_string())?;
        if bytes.is_empty() {
            return Err("TikTok photo image response was empty.".to_string());
        }
        let file_name = tiktok_photo_file_name(post_id, index, count);
        let destination = dest_dir.join(file_name);
        fs::write(&destination, &bytes).map_err(|error| error.to_string())?;
        if first_path.is_none() {
            first_path = Some(destination.clone());
        }
        saved_paths.push(destination);
    }
    let selected_path = saved_paths
        .get(requested_tiktok_image_index(url, saved_paths.len()))
        .cloned()
        .or(first_path)
        .ok_or_else(|| "TikTok photo post did not save any images.".to_string())?;
    let audio_path =
        run_tiktok_photo_audio_download(Path::new(&yt_dlp), &fetch_url, dest_dir, post_id);

    Ok(SingleVideoDownloadResult {
        absolute_path: selected_path,
        provider_video_id: Some(post_id.to_string()),
        uploader,
        title,
        media_type: "slideshow".to_string(),
        audio_path,
        captured_at,
    })
}

fn run_tiktok_photo_audio_download(
    yt_dlp: &Path,
    fetch_url: &str,
    dest_dir: &Path,
    post_id: &str,
) -> Option<PathBuf> {
    let output_template = format!(
        "{}/{}_audio.%(ext)s",
        dest_dir.to_string_lossy().replace('\\', "/"),
        post_id
    );
    let mut command = Command::new(yt_dlp);
    configure_background_command(&mut command);
    command
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .arg("--ignore-errors")
        .arg("--no-warnings")
        .arg("--impersonate")
        .arg("chrome")
        .arg("--no-cookies-from-browser")
        .arg("--no-playlist")
        .arg("-f")
        .arg("ba")
        .arg("-o")
        .arg(&output_template)
        .arg(fetch_url);

    if command.output().is_err() {
        return None;
    }
    single_video_audio_path(dest_dir, Some(post_id))
}

/// Baixa um vídeo avulso por URL (yt-dlp; `--impersonate` para TikTok), salva na
/// raiz plana "Single videos" e cataloga em `single_videos` (dedup por provider+id).
pub fn download_single_video(url: String) -> Result<SingleVideo, String> {
    with_workspace(|connection, layout| {
        let url = url.trim().to_string();
        if url.is_empty() {
            return Err("A video URL is required.".to_string());
        }
        let provider = detect_single_video_provider(&url).ok_or_else(|| {
            "Unsupported URL — only TikTok, Instagram, Twitter/X and YouTube video links are supported."
                .to_string()
        })?;
        let root = single_videos_root(connection, layout)?;

        let result = match single_video_url_kind(provider, &url) {
            SingleVideoUrlKind::Video => {
                run_yt_dlp_video_download(connection, layout, &url, provider, &root)?
            }
            SingleVideoUrlKind::TikTokPhoto { handle, post_id } => {
                run_tiktok_photo_download(connection, layout, &url, &root, &handle, &post_id)?
            }
        };
        let absolute = result.absolute_path;
        let provider_video_id = result.provider_video_id;
        let uploader = result.uploader;
        let title = result.title;
        let media_type = result.media_type;
        let audio_path = result.audio_path;
        let captured_at = result.captured_at;

        let relative_path = absolute
            .strip_prefix(&root)
            .unwrap_or(&absolute)
            .to_string_lossy()
            .replace('\\', "/");
        let now = now_timestamp();
        let id = new_id();

        connection
            .execute(
                "INSERT INTO single_videos (
                    id, provider, source_url, provider_video_id, uploader, title,
                    relative_path, media_type, captured_at, downloaded_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(provider, provider_video_id) DO UPDATE SET
                    source_url = excluded.source_url,
                    uploader = COALESCE(excluded.uploader, uploader),
                    title = COALESCE(excluded.title, title),
                    relative_path = excluded.relative_path,
                    media_type = excluded.media_type,
                    captured_at = COALESCE(excluded.captured_at, captured_at),
                    downloaded_at = excluded.downloaded_at",
                params![
                    id,
                    provider,
                    &url,
                    provider_video_id,
                    uploader,
                    title,
                    relative_path,
                    media_type,
                    captured_at,
                    now
                ],
            )
            .map_err(|error| error.to_string())?;

        // Em conflito o `id` persistido é o antigo; recupera o canônico.
        let canonical_id = match provider_video_id.as_deref() {
            Some(video_id) => connection
                .query_row(
                    "SELECT id FROM single_videos WHERE provider = ?1 AND provider_video_id = ?2",
                    params![provider, video_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| error.to_string())?
                .unwrap_or_else(|| id.clone()),
            None => id.clone(),
        };

        let files = single_video_files(
            &root,
            &relative_path,
            &media_type,
            provider_video_id.as_deref(),
        );
        let audio_relative_path = single_video_audio_relative_path(&root, audio_path.as_deref());
        let audio_absolute_path = audio_path
            .as_deref()
            .map(|path| path.to_string_lossy().to_string());

        Ok(SingleVideo {
            id: canonical_id,
            provider: provider.to_string(),
            source_url: url,
            provider_video_id,
            uploader,
            title,
            absolute_path: absolute.to_string_lossy().to_string(),
            files,
            relative_path,
            media_type,
            captured_at,
            downloaded_at: now,
            audio_relative_path,
            audio_absolute_path,
        })
    })
}

/// Lista os vídeos avulsos catalogados (mais recentes primeiro).
pub fn list_single_videos() -> Result<Vec<SingleVideo>, String> {
    with_workspace(|connection, layout| {
        let root = single_videos_root(connection, layout)?;
        let mut statement = connection
            .prepare(
                "SELECT id, provider, source_url, provider_video_id, uploader, title,
                        relative_path, media_type, captured_at, downloaded_at
                 FROM single_videos
                 ORDER BY downloaded_at DESC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                let source_url: String = row.get(2)?;
                let provider_video_id: Option<String> = row.get(3)?;
                let relative_path: String = row.get(6)?;
                let media_type: String = row.get(7)?;
                let display_relative_path = single_video_display_relative_path(
                    &root,
                    &relative_path,
                    &media_type,
                    &source_url,
                    provider_video_id.as_deref(),
                );
                let audio_path = single_video_audio_path(&root, provider_video_id.as_deref());
                let audio_relative_path =
                    single_video_audio_relative_path(&root, audio_path.as_deref());
                let audio_absolute_path = audio_path
                    .as_deref()
                    .map(|path| path.to_string_lossy().to_string());
                Ok(SingleVideo {
                    id: row.get(0)?,
                    provider: row.get(1)?,
                    source_url,
                    provider_video_id: provider_video_id.clone(),
                    uploader: row.get(4)?,
                    title: row.get(5)?,
                    absolute_path: root
                        .join(&display_relative_path)
                        .to_string_lossy()
                        .to_string(),
                    files: single_video_files(
                        &root,
                        &relative_path,
                        &media_type,
                        provider_video_id.as_deref(),
                    ),
                    relative_path: display_relative_path,
                    media_type,
                    captured_at: row.get(8)?,
                    downloaded_at: row.get(9)?,
                    audio_relative_path,
                    audio_absolute_path,
                })
            })
            .map_err(|error| error.to_string())?;
        let mut videos = Vec::new();
        for row in rows {
            videos.push(row.map_err(|error| error.to_string())?);
        }
        Ok(videos)
    })
}

/// Remove um vídeo avulso: manda o arquivo para a Lixeira e apaga a linha do
/// catálogo. Devolve a lista atualizada.
pub fn delete_single_video(id: String) -> Result<Vec<SingleVideo>, String> {
    with_workspace(|connection, layout| {
        let root = single_videos_root(connection, layout)?;
        if let Some((relative, media_type, provider_video_id)) = connection
            .query_row(
                "SELECT relative_path, media_type, provider_video_id FROM single_videos WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?
        {
            let absolute = root.join(&relative);
            if media_type == "slideshow" {
                if let Some(post_id) = provider_video_id.as_deref() {
                    delete_single_video_slideshow_files(&root, post_id)?;
                } else if absolute.exists() {
                    let _ = trash::delete(&absolute);
                }
            } else if absolute.exists() {
                let _ = trash::delete(&absolute);
            }
        }
        connection
            .execute("DELETE FROM single_videos WHERE id = ?1", params![id])
            .map_err(|error| error.to_string())?;
        Ok(())
    })?;
    list_single_videos()
}

fn delete_single_video_slideshow_files(root: &Path, post_id: &str) -> Result<(), String> {
    let prefix = format!("{post_id}_");
    let single_name = format!("{post_id}.jpg");
    for entry in fs::read_dir(root)
        .map_err(|error| error.to_string())?
        .flatten()
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if file_name == single_name || file_name.starts_with(&prefix) {
            let _ = trash::delete(&path);
        }
    }
    Ok(())
}

fn paths_match_case_insensitive(left: &Path, right: &Path) -> bool {
    normalize_path_for_compare(left) == normalize_path_for_compare(right)
}

fn normalize_path_for_compare(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('/', "\\");
    raw.trim_end_matches('\\').to_ascii_lowercase()
}

fn source_profile_category(source: &SourceProfile) -> &'static str {
    if source
        .sync_options
        .instagram
        .as_ref()
        .and_then(|options| options.favorite)
        .unwrap_or(false)
    {
        return "favorite";
    }

    if source
        .sync_options
        .instagram
        .as_ref()
        .and_then(|options| options.temporary)
        .unwrap_or(false)
    {
        return "temporary";
    }

    "regular"
}

fn merge_protected_authorization_settings(
    settings: &mut Vec<ProviderAccountSettingValue>,
    provider: &str,
    metadata: &CapturedBrowserMetadata,
) {
    let values: Vec<(&str, Option<String>)> = match provider {
        "instagram" => vec![
            ("instagram.auth.csrfToken", metadata.csrf_token.clone()),
            ("instagram.auth.appId", metadata.app_id.clone()),
            ("instagram.auth.asbdId", metadata.asbd_id.clone()),
            ("instagram.auth.igWwwClaim", metadata.ig_www_claim.clone()),
            ("instagram.auth.userAgent", metadata.user_agent.clone()),
            ("instagram.auth.secChUa", metadata.sec_ch_ua.clone()),
            (
                "instagram.auth.secChUaFullVersionList",
                metadata.sec_ch_ua_full_version_list.clone(),
            ),
            (
                "instagram.auth.secChUaPlatformVersion",
                metadata.sec_ch_ua_platform_version.clone(),
            ),
        ],
        "twitter" => vec![
            (
                "twitter.auth.useUserAgent",
                metadata.user_agent.as_ref().map(|_| "true".to_string()),
            ),
            ("twitter.auth.userAgent", metadata.user_agent.clone()),
        ],
        "tiktok" => vec![
            (
                "tiktok.auth.useUserAgent",
                metadata.user_agent.as_ref().map(|_| "true".to_string()),
            ),
            ("tiktok.auth.userAgent", metadata.user_agent.clone()),
        ],
        _ => Vec::new(),
    };
    for (setting_key, value) in values {
        let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
            continue;
        };
        settings.retain(|setting| setting.setting_key != setting_key);
        settings.push(ProviderAccountSettingValue {
            setting_key: setting_key.to_string(),
            value_kind: ProviderAccountSettingValueKind::String,
            string_value: Some(value),
            json_value: None,
        });
    }
}

fn is_protected_authorization_setting(provider: &str, setting_key: &str) -> bool {
    match provider {
        "instagram" => matches!(
            setting_key,
            "instagram.auth.csrfToken"
                | "instagram.auth.appId"
                | "instagram.auth.asbdId"
                | "instagram.auth.igWwwClaim"
                | "instagram.auth.userAgent"
                | "instagram.auth.secChUa"
                | "instagram.auth.secChUaFullVersionList"
                | "instagram.auth.secChUaPlatformVersion"
        ),
        "twitter" => setting_key == "twitter.auth.userAgent",
        "tiktok" => setting_key == "tiktok.auth.userAgent",
        _ => false,
    }
}

fn update_protected_authorization_metadata(
    connection: &Connection,
    layout: &StorageLayout,
    account_id: &str,
    provider: &str,
    values: &HashMap<String, String>,
) -> Result<(), String> {
    let secret_ref = load_account_session_secret_ref(connection, account_id)?
        .ok_or_else(|| "The account session secret is missing.".to_string())?;
    let secret_payload = session_secret_store::load_secret(layout, &secret_ref)?;
    let mut parsed = parse_session_payload(&secret_payload)?;
    let optional = |key: &str| {
        values
            .get(key)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    match provider {
        "instagram" => {
            parsed.metadata.csrf_token = optional("instagram.auth.csrfToken");
            parsed.metadata.app_id = optional("instagram.auth.appId");
            parsed.metadata.asbd_id = optional("instagram.auth.asbdId");
            parsed.metadata.ig_www_claim = optional("instagram.auth.igWwwClaim");
            parsed.metadata.user_agent = optional("instagram.auth.userAgent");
            parsed.metadata.sec_ch_ua = optional("instagram.auth.secChUa");
            parsed.metadata.sec_ch_ua_full_version_list =
                optional("instagram.auth.secChUaFullVersionList");
            parsed.metadata.sec_ch_ua_platform_version =
                optional("instagram.auth.secChUaPlatformVersion");
        }
        "twitter" => parsed.metadata.user_agent = optional("twitter.auth.userAgent"),
        "tiktok" => parsed.metadata.user_agent = optional("tiktok.auth.userAgent"),
        _ => {}
    }
    let updated_payload = serialize_session_payload_for_storage(
        &parsed.cookies,
        parsed.current_url.as_deref(),
        Some(&parsed.metadata),
    )?;
    session_secret_store::store_secret(layout, &secret_ref, &updated_payload)?;
    connection
        .execute(
            "UPDATE provider_account_sessions
             SET fingerprint = ?2, updated_at = ?3
             WHERE account_id = ?1",
            params![
                account_id,
                session_fingerprint(&updated_payload),
                now_timestamp()
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn find_user_instagram_xml(profile_root: &Path) -> Result<Option<PathBuf>, String> {
    let settings_dir = profile_root.join("Settings");
    if !settings_dir.is_dir() {
        return Ok(None);
    }

    let mut matches = fs::read_dir(&settings_dir)
        .map_err(|error| error.to_string())?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(is_user_instagram_xml_file_name)
        })
        .collect::<Vec<_>>();
    matches.sort();
    Ok(matches.into_iter().next())
}

fn find_user_instagram_data_xml(profile_root: &Path) -> Result<Option<PathBuf>, String> {
    let settings_dir = profile_root.join("Settings");
    if !settings_dir.is_dir() {
        return Ok(None);
    }

    let mut matches = fs::read_dir(&settings_dir)
        .map_err(|error| error.to_string())?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(is_user_instagram_data_xml_file_name)
        })
        .collect::<Vec<_>>();
    matches.sort();
    Ok(matches.into_iter().next())
}

fn is_user_instagram_xml_file_name(value: &str) -> bool {
    let lowercase = value.to_ascii_lowercase();
    lowercase.starts_with("user_instagram")
        && lowercase.ends_with(".xml")
        && !lowercase.ends_with("_data.xml")
}

fn is_user_instagram_data_xml_file_name(value: &str) -> bool {
    let lowercase = value.to_ascii_lowercase();
    lowercase.starts_with("user_instagram") && lowercase.ends_with("_data.xml")
}

/// Extracts the post shortcode from a permalink preserving its original casing.
/// Instagram shortcodes are case-sensitive, so the cased form is what feeds the
/// reconstructed `instagram.com/p/<code>/` URL.
fn extract_instagram_post_code_from_permalink_cased(value: &str) -> Option<String> {
    let normalized = value.trim();
    let marker = ["/p/", "/reel/", "/tv/"]
        .into_iter()
        .find(|marker| normalized.contains(marker))?;
    let tail = normalized.split_once(marker)?.1;
    tail.split(['/', '?', '&', '#'])
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn extract_instagram_post_code_from_permalink(value: &str) -> Option<String> {
    extract_instagram_post_code_from_permalink_cased(value).map(|code| code.to_ascii_lowercase())
}

fn is_user_twitter_data_xml_file_name(value: &str) -> bool {
    let lowercase = value.to_ascii_lowercase();
    lowercase.starts_with("user_twitter") && lowercase.ends_with("_data.xml")
}

fn find_user_twitter_data_xml(profile_root: &Path) -> Result<Option<PathBuf>, String> {
    let settings_dir = profile_root.join("Settings");
    if !settings_dir.is_dir() {
        return Ok(None);
    }
    let mut matches = fs::read_dir(&settings_dir)
        .map_err(|error| error.to_string())?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(is_user_twitter_data_xml_file_name)
        })
        .collect::<Vec<_>>();
    matches.sort();
    Ok(matches.into_iter().next())
}

/// Twitter media key from a downloaded file name: drops the date prefix, an
/// optional `GIF_` prefix and the extension, lowercased. This matches the
/// basename of the `File` attribute in the SCrawler `User_Twitter_*_Data.xml`,
/// letting us pair a file on disk with its tweet status id.
fn twitter_media_key_from_file_name(file_name: &str) -> Option<String> {
    let stem = file_name
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(file_name);
    let (_, rest) = strip_gallery_date_prefix(stem);
    let mut key = rest.trim().to_ascii_lowercase();
    if let Some(stripped) = key.strip_prefix("gif_") {
        key = stripped.to_string();
    }
    let key = key.trim().to_string();
    (!key.is_empty()).then_some(key)
}

fn xml_text(document: &roxmltree::Document<'_>, tag_name: &str) -> Option<String> {
    document
        .descendants()
        .find(|node| node.has_tag_name(tag_name))
        .and_then(|node| node.text())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}

fn xml_bool(document: &roxmltree::Document<'_>, tag_name: &str) -> bool {
    xml_text(document, tag_name)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
        .unwrap_or(false)
}

fn validate_bound_source_provider_integrity(
    connection: &Connection,
    account_id: &str,
    provider: &str,
) -> Result<(), String> {
    let mismatch = connection
        .query_row(
            "SELECT handle, provider
             FROM source_profiles
             WHERE account_id = ?1
               AND provider <> ?2
               AND deleted_at IS NULL
             LIMIT 1",
            params![account_id, provider],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;

    if let Some((handle, bound_provider)) = mismatch {
        return Err(format!(
            "Cannot set provider account '{}' to provider '{}' while bound source '{}' uses provider '{}'.",
            account_id, provider, handle, bound_provider
        ));
    }

    Ok(())
}

fn load_existing_relative_media_paths(profile_root: &Path) -> HashSet<String> {
    let mut paths = HashSet::new();
    let Ok(files) = collect_media_file_paths(profile_root) else {
        return paths;
    };
    for file in files {
        if file
            .metadata()
            .map(|metadata| metadata.len() == 0)
            .unwrap_or(true)
        {
            continue;
        }
        if let Some(name) = file.file_name().and_then(|value| value.to_str()) {
            paths.insert(name.to_string());
        }
    }
    paths
}

fn load_provider_sync_post_ledger_keys(
    connection: &Connection,
    provider: &str,
    source_id: &str,
) -> Result<HashSet<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT provider_post_key FROM provider_sync_post_ledger
             WHERE provider = ?1 AND source_id = ?2",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![provider, source_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    let mut keys = HashSet::new();
    for row in rows {
        keys.insert(row.map_err(|error| error.to_string())?);
    }
    Ok(keys)
}

fn upsert_provider_sync_post_ledger_entries(
    connection: &Connection,
    provider: &str,
    source_id: &str,
    account_id: &str,
    source_handle: &str,
    observed_posts: &[twitter_connector::ObservedTwitterPost],
    timestamp: &str,
) -> Result<(), String> {
    for post in observed_posts {
        let provider_post_key = post.provider_post_key.trim();
        if provider_post_key.is_empty() {
            continue;
        }
        connection
            .execute(
                "INSERT INTO provider_sync_post_ledger (
                    provider, source_id, account_id, source_handle,
                    provider_post_key, provider_post_code, media_section,
                    first_seen_at, last_seen_at
                 )
                 VALUES (?1, ?2, ?3, ?4, ?5, '', ?6, ?7, ?7)
                 ON CONFLICT(provider, source_id, provider_post_key)
                 DO UPDATE SET
                    account_id = excluded.account_id,
                    source_handle = excluded.source_handle,
                    media_section = excluded.media_section,
                    last_seen_at = excluded.last_seen_at",
                params![
                    provider,
                    source_id,
                    account_id,
                    source_handle,
                    provider_post_key,
                    &post.media_section,
                    timestamp,
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn upsert_tiktok_post_stats(
    connection: &Connection,
    source_id: &str,
    observed_posts: &[tiktok_connector::ObservedTikTokPost],
    timestamp: &str,
) -> Result<(), String> {
    for post in observed_posts {
        if post.view_count.is_none()
            && post.like_count.is_none()
            && post.comment_count.is_none()
            && post.share_count.is_none()
        {
            continue;
        }
        connection
            .execute(
                "UPDATE provider_sync_post_ledger
                 SET view_count = ?1, like_count = ?2, comment_count = ?3,
                     share_count = ?4, stats_updated_at = ?5
                 WHERE provider = 'tiktok' AND source_id = ?6
                   AND provider_post_key = ?7",
                params![
                    post.view_count,
                    post.like_count,
                    post.comment_count,
                    post.share_count,
                    timestamp,
                    source_id,
                    &post.provider_post_key,
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CapturedBrowserMetadata {
    #[serde(default)]
    pub csrf_token: Option<String>,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub asbd_id: Option<String>,
    #[serde(default)]
    pub ig_www_claim: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub sec_ch_ua: Option<String>,
    #[serde(default)]
    pub sec_ch_ua_full_version_list: Option<String>,
    #[serde(default)]
    pub sec_ch_ua_platform_version: Option<String>,
    #[serde(default)]
    pub lsd: Option<String>,
    #[serde(default)]
    pub dtsg: Option<String>,
}

fn format_download_success_summary(prefix: &str, ingested_media_count: usize) -> String {
    if ingested_media_count == 0 {
        format!("{prefix} No new media downloaded.")
    } else {
        format!("{prefix} Downloaded {} media items.", ingested_media_count)
    }
}

fn format_connector_sync_success_summary(
    ingested_media_count: usize,
    degraded_capabilities: &[String],
) -> String {
    if degraded_capabilities.is_empty() {
        return format_download_success_summary("Connector sync succeeded.", ingested_media_count);
    }

    if ingested_media_count == 0 {
        format!(
            "Connector sync succeeded. No new media downloaded. Degraded capabilities: {}.",
            degraded_capabilities.join(", ")
        )
    } else {
        format!(
            "Connector sync succeeded. Downloaded {} media items with degraded capabilities: {}.",
            ingested_media_count,
            degraded_capabilities.join(", ")
        )
    }
}

/// Sufixo curto e amigável comum aos providers: "N posts already up to date."
/// (vazio quando nada estava sincronizado). Os contadores técnicos ficam no
/// realtime debugger, não no resumo mostrado ao usuário.
pub(super) fn format_already_up_to_date_suffix(already_up_to_date: u32) -> String {
    if already_up_to_date == 0 {
        return String::new();
    }
    let post_word = if already_up_to_date == 1 {
        "post"
    } else {
        "posts"
    };
    format!(" {already_up_to_date} {post_word} already up to date.")
}

/// Sufixo amigável do Instagram para o resumo mostrado ao usuário. O
/// detalhamento técnico (posts/assets pulados por seção) vai para o realtime
/// debugger — aqui só dizemos, em linguagem simples, o que já estava em dia.
fn format_instagram_manifest_suffix(
    manifest_summary: Option<&instagram_connector::InstagramManifestSummary>,
    include_in_summary: bool,
) -> String {
    if !include_in_summary {
        return String::new();
    }

    manifest_summary
        .map(|summary| {
            // "Já em dia" = posts reconhecidos como sincronizados (no ledger ou
            // com a mídia já em disco) + duplicados colapsados.
            format_already_up_to_date_suffix(
                summary.skipped_existing_post_count + summary.skipped_duplicate_post_count,
            )
        })
        .unwrap_or_default()
}

fn parse_rfc3339_utc(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

fn read_instagram_avatar_cooldown_until(
    settings: &HashMap<String, String>,
) -> Option<DateTime<Utc>> {
    settings
        .get(INSTAGRAM_AVATAR_COOLDOWN_UNTIL_SETTING_KEY)
        .and_then(|value| parse_rfc3339_utc(value))
}

fn set_instagram_avatar_cooldown(
    connection: &Connection,
    account_id: &str,
    retry_after: StdDuration,
    now: &str,
) -> Result<DateTime<Utc>, String> {
    let base_time = parse_rfc3339_utc(now).unwrap_or_else(Utc::now);
    let seconds = retry_after.as_secs().clamp(1, 24 * 60 * 60);
    let until = base_time + Duration::seconds(i64::try_from(seconds).unwrap_or(24 * 60 * 60));
    upsert_provider_account_string_setting(
        connection,
        account_id,
        INSTAGRAM_AVATAR_COOLDOWN_UNTIL_SETTING_KEY,
        &until.to_rfc3339(),
        now,
    )?;
    Ok(until)
}

fn clear_instagram_avatar_cooldown(
    connection: &Connection,
    account_id: &str,
) -> Result<(), String> {
    delete_provider_account_setting(
        connection,
        account_id,
        INSTAGRAM_AVATAR_COOLDOWN_UNTIL_SETTING_KEY,
    )
}

fn persist_instagram_runtime_auth_headers(
    connection: &Connection,
    account_id: &str,
    headers: &instagram_connector::InstagramAuthHeaders,
    now: &str,
) -> Result<(), String> {
    for (setting_key, value) in [
        ("instagram.auth.csrfToken", headers.csrf_token.as_deref()),
        ("instagram.auth.igWwwClaim", headers.ig_www_claim.as_deref()),
    ] {
        if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
            upsert_provider_account_string_setting(
                connection,
                account_id,
                setting_key,
                value,
                now,
            )?;
        }
    }
    Ok(())
}

fn error_message_http_status(error: &str) -> Option<u16> {
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

fn instagram_error_indicates_rate_limit(error: &str) -> bool {
    error_message_http_status(error) == Some(429)
        || error.to_ascii_lowercase().contains("too many requests")
}

fn instagram_error_is_inconclusive_identity_probe(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("timeline response is missing user data")
        && (lower.contains("profile accessibility probe returned 429")
            || lower.contains("html profile probe returned 429"))
}

/// Procura outro perfil ativo do mesmo provider cujo `userIdHint` coincide com
/// `user_id`, ignorando `self_id`. Usado para detectar que um perfil recém-
/// adicionado, ao resolver sua identidade no primeiro sync, é na verdade um
/// usuário já cadastrado (handle antigo vs novo). Retorna (id, handle).
fn find_source_with_same_user_id(
    connection: &Connection,
    provider: &str,
    user_id: &str,
    self_id: &str,
) -> Result<Option<(String, String)>, String> {
    let user_id = user_id.trim();
    if user_id.is_empty() {
        return Ok(None);
    }
    let mut statement = connection
        .prepare(
            "SELECT id, handle, sync_options_json FROM source_profiles
             WHERE provider = ?1 AND deleted_at IS NULL AND id != ?2",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![provider, self_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (id, handle, json) = row.map_err(|error| error.to_string())?;
        if source_user_id_hint_from_json(provider, &json).as_deref() == Some(user_id) {
            return Ok(Some((id, handle)));
        }
    }
    Ok(None)
}

fn build_instagram_identity_probe_request(
    username: &str,
) -> instagram_connector::InstagramConnectorRequest {
    instagram_connector::InstagramConnectorRequest {
        username: username.to_string(),
        cookies: Vec::new(),
        headers: instagram_connector::InstagramAuthHeaders {
            app_id: Some(INSTAGRAM_PUBLIC_APP_ID.to_string()),
            asbd_id: Some(INSTAGRAM_PUBLIC_ASBD_ID.to_string()),
            ig_www_claim: Some(INSTAGRAM_PUBLIC_IG_CLAIM.to_string()),
            user_agent: Some(INSTAGRAM_PUBLIC_USER_AGENT.to_string()),
            ..Default::default()
        },
        profile_root: PathBuf::new(),
        saved_posts_root: PathBuf::new(),
        ledger_post_keys: HashSet::new(),
        deleted_post_keys: HashSet::new(),
        existing_media_keys: HashSet::new(),
        ledger_media_keys: HashSet::new(),
        existing_relative_paths: HashSet::new(),
        ledger_relative_paths: HashSet::new(),
        sections: instagram_connector::InstagramSectionSelection::default(),
        use_gql: true,
        download_saved_posts: false,
        post_page_size: 12,
        skip_errors: true,
        skip_errors_exclude: Vec::new(),
        log_skipped_errors: true,
        tagged_notify_limit: 0,
        ignore_stories_560_errors: true,
        pacing: instagram_connector::InstagramPacing {
            base_delay_ms: 1000,
            extra_delay_ms: 0,
            counter_threshold: 0,
            page_delay_ms: 0,
        },
        timeout_secs: 20,
        download_images: false,
        download_videos: false,
        extract_image_from_video: instagram_connector::InstagramSectionSelection::default(),
        place_extracted_image_into_video_folder: false,
        download_text: false,
        download_text_posts: false,
        text_special_folder: false,
        get_user_media_only: false,
        missing_only: false,
        full_scan: false,
        date_from_timestamp: None,
        date_to_timestamp: None,
        media_file_naming_mode: instagram_connector::InstagramMediaFileNamingMode::PresetNewDefault,
        media_file_naming_template: None,
        target_story_media_id: None,
    }
}

fn build_instagram_authenticated_identity_probe_request(
    username: &str,
    cookies: &[CapturedBrowserCookie],
    settings: &HashMap<String, String>,
    metadata: Option<&CapturedBrowserMetadata>,
) -> instagram_connector::InstagramConnectorRequest {
    instagram_connector::InstagramConnectorRequest {
        username: username.to_string(),
        cookies: cookies
            .iter()
            .map(|cookie| instagram_connector::SessionCookie {
                domain: cookie.domain.clone(),
                name: cookie.name.clone(),
                value: cookie.value.clone(),
            })
            .collect(),
        headers: build_instagram_auth_headers(settings, cookies, metadata),
        profile_root: PathBuf::new(),
        saved_posts_root: PathBuf::new(),
        ledger_post_keys: HashSet::new(),
        deleted_post_keys: HashSet::new(),
        existing_media_keys: HashSet::new(),
        ledger_media_keys: HashSet::new(),
        existing_relative_paths: HashSet::new(),
        ledger_relative_paths: HashSet::new(),
        sections: instagram_connector::InstagramSectionSelection::default(),
        use_gql: true,
        download_saved_posts: false,
        post_page_size: 12,
        skip_errors: true,
        skip_errors_exclude: Vec::new(),
        log_skipped_errors: true,
        tagged_notify_limit: 0,
        ignore_stories_560_errors: true,
        pacing: instagram_connector::InstagramPacing::none(),
        timeout_secs: 20,
        download_images: false,
        download_videos: false,
        extract_image_from_video: instagram_connector::InstagramSectionSelection::default(),
        place_extracted_image_into_video_folder: false,
        download_text: false,
        download_text_posts: false,
        text_special_folder: false,
        get_user_media_only: false,
        missing_only: false,
        full_scan: false,
        date_from_timestamp: None,
        date_to_timestamp: None,
        media_file_naming_mode: instagram_connector::InstagramMediaFileNamingMode::PresetNewDefault,
        media_file_naming_template: None,
        target_story_media_id: None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InstagramIdentityErrorClassification {
    UsernameUnresolvable,
    PrivateOrRestricted,
    Other,
}

fn classify_instagram_identity_error(error: &str) -> InstagramIdentityErrorClassification {
    let status = extract_http_status_code_from_message(error);
    let lower = error.to_ascii_lowercase();
    let has_private_probe_marker =
        lower.contains("[identity_probe=instagram_profile_private_or_restricted]");
    let has_unresolvable_probe_marker =
        lower.contains("[identity_probe=instagram_username_unresolvable]");

    if has_unresolvable_probe_marker {
        return InstagramIdentityErrorClassification::UsernameUnresolvable;
    }

    if has_private_probe_marker {
        return InstagramIdentityErrorClassification::PrivateOrRestricted;
    }

    if status == Some(404) || lower.contains("user not found") {
        return InstagramIdentityErrorClassification::UsernameUnresolvable;
    }

    let indicates_private_or_restricted = lower.contains("private")
        || lower.contains("restricted")
        || lower.contains("not authorized")
        || lower.contains("unauthorized")
        || lower.contains("login required")
        || lower.contains("checkpoint_required")
        || lower.contains("consent_required")
        || lower.contains("challenge_required");

    if indicates_private_or_restricted && matches!(status, Some(400) | Some(401) | Some(403) | None)
    {
        return InstagramIdentityErrorClassification::PrivateOrRestricted;
    }

    InstagramIdentityErrorClassification::Other
}

fn extract_http_status_code_from_message(error: &str) -> Option<u16> {
    let bytes = error.as_bytes();
    for index in 0..bytes.len().saturating_sub(2) {
        if !bytes[index].is_ascii_digit()
            || !bytes[index + 1].is_ascii_digit()
            || !bytes[index + 2].is_ascii_digit()
        {
            continue;
        }

        let has_left_boundary = index == 0 || !bytes[index - 1].is_ascii_digit();
        let has_right_boundary = index + 3 >= bytes.len() || !bytes[index + 3].is_ascii_digit();
        if !has_left_boundary || !has_right_boundary {
            continue;
        }

        let code = std::str::from_utf8(&bytes[index..index + 3])
            .ok()
            .and_then(|value| value.parse::<u16>().ok())?;
        if (100..=599).contains(&code) {
            return Some(code);
        }
    }
    None
}

fn build_instagram_saved_posts_request(
    layout: &StorageLayout,
    context: &AccountSyncContext,
) -> Result<instagram_connector::InstagramConnectorRequest, String> {
    let parsed_session = parse_session_payload(&context.session_payload)?;
    let cookies = parsed_session.cookies;
    let metadata = parsed_session.metadata;
    let extract_saved_posts_image_from_video = parse_bool_setting(
        context
            .settings
            .get("instagram.account.extractSavedPostsImageFromVideo")
            .map(String::as_str),
        true,
    );
    let download_text = parse_bool_setting(
        context
            .settings
            .get("instagram.defaults.downloadText")
            .map(String::as_str),
        false,
    );
    let download_text_posts = parse_bool_setting(
        context
            .settings
            .get("instagram.defaults.downloadTextPosts")
            .map(String::as_str),
        false,
    );
    let text_special_folder = parse_bool_setting(
        context
            .settings
            .get("instagram.defaults.textSpecialFolder")
            .map(String::as_str),
        true,
    );
    let place_extracted_image_into_video_folder = parse_bool_setting(
        context
            .settings
            .get("instagram.defaults.placeExtractedImageIntoVideoFolder")
            .map(String::as_str),
        false,
    );
    let media_file_naming_mode = parse_instagram_media_file_naming_mode(&context.settings);
    let media_file_naming_template = parse_instagram_media_file_naming_template(&context.settings);
    Ok(instagram_connector::InstagramConnectorRequest {
        username: "saved".to_string(),
        cookies: cookies
            .iter()
            .map(|cookie| instagram_connector::SessionCookie {
                domain: cookie.domain.clone(),
                name: cookie.name.clone(),
                value: cookie.value.clone(),
            })
            .collect(),
        headers: build_instagram_auth_headers(&context.settings, &cookies, Some(&metadata)),
        profile_root: resolve_instagram_profile_root_for_account(
            layout,
            &context.account.display_name,
            Some(&context.settings),
        ),
        saved_posts_root: resolve_instagram_saved_posts_root(layout, Some(&context.settings)),
        ledger_post_keys: HashSet::new(),
        deleted_post_keys: HashSet::new(),
        existing_media_keys: HashSet::new(),
        ledger_media_keys: HashSet::new(),
        existing_relative_paths: HashSet::new(),
        ledger_relative_paths: HashSet::new(),
        sections: instagram_connector::InstagramSectionSelection::default(),
        use_gql: parse_instagram_use_gql_setting(&context.settings),
        download_saved_posts: true,
        post_page_size: parse_instagram_post_page_size(&context.settings, true),
        skip_errors: parse_bool_setting(
            context
                .settings
                .get("instagram.errors.skipErrors")
                .map(String::as_str),
            true,
        ),
        skip_errors_exclude: instagram_error_policy_settings(&context.settings).0,
        log_skipped_errors: instagram_error_policy_settings(&context.settings).1,
        tagged_notify_limit: instagram_error_policy_settings(&context.settings).2,
        ignore_stories_560_errors: parse_bool_setting(
            context
                .settings
                .get("instagram.errors.ignoreStories560")
                .map(String::as_str),
            false,
        ),
        pacing: instagram_request_pacing(&context.settings),
        timeout_secs: 45,
        download_images: true,
        download_videos: true,
        extract_image_from_video: instagram_connector::InstagramSectionSelection {
            timeline: extract_saved_posts_image_from_video,
            reels: extract_saved_posts_image_from_video,
            stories: extract_saved_posts_image_from_video,
            stories_user: extract_saved_posts_image_from_video,
            tagged: extract_saved_posts_image_from_video,
        },
        place_extracted_image_into_video_folder,
        download_text,
        download_text_posts,
        text_special_folder,
        get_user_media_only: false,
        missing_only: false,
        full_scan: false,
        date_from_timestamp: None,
        date_to_timestamp: None,
        media_file_naming_mode,
        media_file_naming_template,
        target_story_media_id: None,
    })
}

fn build_instagram_auth_headers(
    settings: &HashMap<String, String>,
    cookies: &[CapturedBrowserCookie],
    metadata: Option<&CapturedBrowserMetadata>,
) -> instagram_connector::InstagramAuthHeaders {
    let metadata_value = |selector: fn(&CapturedBrowserMetadata) -> Option<&String>| {
        metadata
            .and_then(selector)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };

    instagram_connector::InstagramAuthHeaders {
        csrf_token: setting_value(settings, "instagram.auth.csrfToken")
            .or_else(|| metadata_value(|meta| meta.csrf_token.as_ref()))
            .or_else(|| {
                cookies
                    .iter()
                    .find(|cookie| cookie.name.eq_ignore_ascii_case("csrftoken"))
                    .map(|cookie| cookie.value.trim().to_string())
                    .filter(|value| !value.is_empty())
            }),
        app_id: setting_value(settings, "instagram.auth.appId")
            .or_else(|| metadata_value(|meta| meta.app_id.as_ref())),
        asbd_id: setting_value(settings, "instagram.auth.asbdId")
            .or_else(|| metadata_value(|meta| meta.asbd_id.as_ref())),
        ig_www_claim: setting_value(settings, "instagram.auth.igWwwClaim")
            .or_else(|| metadata_value(|meta| meta.ig_www_claim.as_ref())),
        lsd: setting_value(settings, "instagram.auth.lsd")
            .or_else(|| metadata_value(|meta| meta.lsd.as_ref())),
        dtsg: setting_value(settings, "instagram.auth.dtsg")
            .or_else(|| metadata_value(|meta| meta.dtsg.as_ref())),
        sec_ch_ua: setting_value(settings, "instagram.auth.secChUa")
            .or_else(|| metadata_value(|meta| meta.sec_ch_ua.as_ref())),
        sec_ch_ua_full_version_list: setting_value(
            settings,
            "instagram.auth.secChUaFullVersionList",
        )
        .or_else(|| metadata_value(|meta| meta.sec_ch_ua_full_version_list.as_ref())),
        sec_ch_ua_platform_version: setting_value(
            settings,
            "instagram.auth.secChUaPlatformVersion",
        )
        .or_else(|| metadata_value(|meta| meta.sec_ch_ua_platform_version.as_ref())),
        user_agent: setting_value(settings, "instagram.auth.userAgent")
            .or_else(|| metadata_value(|meta| meta.user_agent.as_ref())),
    }
}

fn parse_instagram_use_gql_setting(settings: &HashMap<String, String>) -> bool {
    parse_bool_setting_from_keys(
        settings,
        &["instagram.download.graphQlPrimary", "instagram.api.useGql"],
        true,
    )
}

fn parse_instagram_media_file_naming_mode(
    settings: &HashMap<String, String>,
) -> instagram_connector::InstagramMediaFileNamingMode {
    match settings
        .get("naming.instagram.media_file_pattern_mode")
        .map(String::as_str)
        .map(str::trim)
        .unwrap_or("preset_new_default")
    {
        "preset_legacy_url_basename" => {
            instagram_connector::InstagramMediaFileNamingMode::PresetLegacyUrlBasename
        }
        "custom" => instagram_connector::InstagramMediaFileNamingMode::Custom,
        _ => instagram_connector::InstagramMediaFileNamingMode::PresetNewDefault,
    }
}

fn parse_instagram_media_file_naming_template(
    settings: &HashMap<String, String>,
) -> Option<String> {
    settings
        .get("naming.instagram.media_file_pattern_template")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn parse_instagram_post_page_size(
    settings: &HashMap<String, String>,
    verified_profile: bool,
) -> u32 {
    let key = if verified_profile {
        "instagram.download.postCountVerified"
    } else {
        "instagram.download.postCountUnverified"
    };
    parse_u32_provider_setting(settings, key, 24)
}

fn parse_u32_provider_setting(settings: &HashMap<String, String>, key: &str, default: u32) -> u32 {
    settings
        .get(key)
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

trait ToolExecutor {
    fn execute(&self, invocation: &ToolInvocation) -> Result<ToolExecutionResult, String>;
}

struct CommandToolExecutor;

struct ToolInvocation {
    source_id: String,
    handle: String,
    connector_key: String,
    executable: String,
    args: Vec<String>,
    command_preview: String,
    working_directory: Option<PathBuf>,
    output_root: PathBuf,
    cancel_token: Arc<AtomicBool>,
}

struct ToolExecutionResult {
    status: String,
}

impl ToolExecutor for CommandToolExecutor {
    fn execute(&self, invocation: &ToolInvocation) -> Result<ToolExecutionResult, String> {
        let _connector_usage = connector_runtime::claim_connector_usage(&invocation.connector_key);
        let mut command = Command::new(&invocation.executable);
        configure_background_command(&mut command);
        command
            .args(&invocation.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(working_directory) = invocation.working_directory.as_ref() {
            command.current_dir(working_directory);
        }

        if invocation.cancel_token.load(Ordering::SeqCst) {
            return Err("source sync cancelled by user".to_string());
        }

        source_sync_runtime::report_source_sync_progress(
            &invocation.source_id,
            None,
            Some("Starting download".to_string()),
            Some("Launching connector runtime".to_string()),
            true,
            Some(0),
        );

        connector_debug::append_current(
            &invocation.connector_key,
            "call",
            "process.spawn",
            std::iter::once(invocation.executable.clone())
                .chain(invocation.args.iter().cloned())
                .collect::<Vec<_>>()
                .join(" "),
        );
        let mut child = command.spawn().map_err(|error| {
            connector_debug::append_current(
                &invocation.connector_key,
                "error",
                "process.spawn",
                error.to_string(),
            );
            format!("Failed to launch '{}': {}", invocation.executable, error)
        })?;

        source_sync_runtime::report_source_sync_progress(
            &invocation.source_id,
            None,
            Some("Connector process started".to_string()),
            Some(format!(
                "{} is running with process id {}.",
                invocation.connector_key,
                child.id()
            )),
            true,
            Some(0),
        );

        let debug_context = connector_debug::current_context();
        let stdout_context = debug_context.clone();
        let stdout_connector = invocation.connector_key.clone();
        let stdout = child.stdout.take();
        let stdout_reader = std::thread::spawn(move || {
            let mut lines = Vec::new();
            if let Some(handle) = stdout {
                for line in BufReader::new(handle).lines().map_while(Result::ok) {
                    connector_debug::append_with_context(
                        stdout_context.clone(),
                        &stdout_connector,
                        "stdout",
                        "process.output",
                        line.clone(),
                    );
                    lines.push(line);
                }
            }
            lines.join("\n")
        });
        let stderr_context = debug_context.clone();
        let stderr_connector = invocation.connector_key.clone();
        let stderr = child.stderr.take();
        let stderr_reader = std::thread::spawn(move || {
            let mut lines = Vec::new();
            if let Some(handle) = stderr {
                for line in BufReader::new(handle).lines().map_while(Result::ok) {
                    connector_debug::append_with_context(
                        stderr_context.clone(),
                        &stderr_connector,
                        "stderr",
                        "process.output",
                        line.clone(),
                    );
                    lines.push(line);
                }
            }
            lines.join("\n")
        });

        let mut last_reported_count = 0_u32;
        let status = loop {
            if invocation.cancel_token.load(Ordering::SeqCst) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                source_sync_runtime::report_source_sync_progress(
                    &invocation.source_id,
                    None,
                    Some("Cancelled".to_string()),
                    Some("Cancellation requested by user".to_string()),
                    false,
                    Some(100),
                );
                connector_debug::append_current(
                    &invocation.connector_key,
                    "system",
                    "process.cancel",
                    "Process killed after cancellation request.",
                );
                return Err("source sync cancelled by user".to_string());
            }

            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    let downloaded_count = count_downloaded_media_items(&invocation.output_root);
                    if downloaded_count != last_reported_count {
                        last_reported_count = downloaded_count;
                    }

                    let detail = if downloaded_count > 0 {
                        format!(
                            "{} · {} files downloaded",
                            invocation.handle, downloaded_count
                        )
                    } else {
                        format!("{} · preparing download stream", invocation.handle)
                    };

                    source_sync_runtime::report_source_sync_progress(
                        &invocation.source_id,
                        None,
                        Some("Downloading profile".to_string()),
                        Some(detail),
                        true,
                        None,
                    );

                    std::thread::sleep(StdDuration::from_millis(SOURCE_SYNC_PROGRESS_POLL_MS));
                }
                Err(error) => {
                    connector_debug::append_current(
                        &invocation.connector_key,
                        "error",
                        "process.wait",
                        error.to_string(),
                    );
                    return Err(format!(
                        "Failed while waiting for '{}': {}",
                        invocation.executable, error
                    ));
                }
            }
        };
        let _stdout = stdout_reader.join().unwrap_or_default();
        let stderr = stderr_reader.join().unwrap_or_default().trim().to_string();
        connector_debug::append_current(
            &invocation.connector_key,
            "response",
            "process.exit",
            format!(
                "exit_code={}",
                status
                    .code()
                    .map_or_else(|| "terminated".to_string(), |code| code.to_string())
            ),
        );

        if status.success() {
            let downloaded_count = count_downloaded_media_items(&invocation.output_root);
            source_sync_runtime::report_source_sync_progress(
                &invocation.source_id,
                Some(100),
                Some("Download finished".to_string()),
                Some(format!(
                    "{} · {} files downloaded",
                    invocation.handle, downloaded_count
                )),
                false,
                Some(downloaded_count),
            );
            Ok(ToolExecutionResult {
                status: "succeeded".to_string(),
            })
        } else {
            let exit_code = status
                .code()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "terminated".to_string());
            let message = if stderr.is_empty() {
                format!("{} exited with code {}", invocation.executable, exit_code)
            } else {
                format!(
                    "{} exited with code {}: {}",
                    invocation.executable, exit_code, stderr
                )
            };
            Err(message)
        }
    }
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|error| error.to_string())
}

fn parse_date_input(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").ok()
}

#[cfg(test)]
mod companion_account_import_tests;

fn load_desktop_runtime_state(connection: &Connection) -> Result<DesktopRuntimeState, String> {
    let settings = load_app_settings_map(connection)?;
    Ok(DesktopRuntimeState {
        close_to_tray: parse_bool_setting(
            settings
                .get(DESKTOP_CLOSE_TO_TRAY_SETTING_KEY)
                .map(String::as_str),
            true,
        ),
        silent_mode: parse_bool_setting(
            settings
                .get(DESKTOP_SILENT_MODE_SETTING_KEY)
                .map(String::as_str),
            false,
        ),
        tray_available: cfg!(desktop),
    })
}

fn connector_degraded_capabilities(
    provider: &str,
    configured_capabilities: &[String],
) -> Vec<String> {
    if provider.eq_ignore_ascii_case("instagram") {
        return Vec::new();
    }

    providers::source_sync_runtime(provider)
        .map(|runtime| {
            runtime
                .degraded_capabilities
                .iter()
                .map(|capability| (*capability).to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
        .into_iter()
        .filter(|capability| configured_capabilities.contains(capability))
        .collect()
}

/// Registra no histórico (`source_sync_runs`) uma troca MANUAL de handle, para
/// aparecer na aba History do editor — espelhando o rastro que o auto-update
/// (resolução via user id) deixa no summary do sync.
fn record_manual_handle_change_run(
    connection: &Connection,
    source_id: &str,
    account_id: &str,
    provider: &str,
    old_handle: &str,
    new_handle: &str,
    timestamp: &str,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO source_sync_runs (
                id, source_id, account_id, provider, tool, trigger, status,
                summary, command_preview, manifest_summary_json,
                degraded_capabilities_json, started_at, finished_at, created_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12, ?12)",
            params![
                new_id(),
                source_id,
                account_id,
                provider,
                "manual",
                "manual_handle_edit",
                "succeeded",
                format!("Handle changed manually: @{old_handle} → @{new_handle}."),
                format!("manual handle edit: @{old_handle} -> @{new_handle}"),
                Option::<String>::None,
                "[]",
                timestamp,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn disable_instagram_sections_after_auth_failure(
    connection: &Connection,
    account_id: &str,
    sections: &[String],
    now: &str,
) -> Result<(), String> {
    let mut setting_keys = HashSet::new();
    for section in sections {
        let key = match section.as_str() {
            "timeline" => Some("instagram.download.timeline"),
            "reels" => Some("instagram.download.reels"),
            "stories" => Some("instagram.download.stories"),
            "stories_user" => Some("instagram.download.storiesUser"),
            "tagged" => Some("instagram.download.taggedPosts"),
            _ => None,
        };
        if let Some(value) = key {
            setting_keys.insert(value);
        }
    }

    for setting_key in setting_keys {
        connection
            .execute(
                "INSERT INTO provider_account_settings (
                    account_id,
                    setting_key,
                    value_kind,
                    value_text,
                    created_at,
                    updated_at
                 )
                 VALUES (?1, ?2, 'string', 'false', ?3, ?3)
                 ON CONFLICT(account_id, setting_key)
                 DO UPDATE SET
                    value_kind = excluded.value_kind,
                    value_text = excluded.value_text,
                    updated_at = excluded.updated_at",
                params![account_id, setting_key, now],
            )
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn catalog_instagram_downloads(
    _connection: &Connection,
    _account_id: &str,
    _source_id: Option<&str>,
    _source_handle: &str,
    _captured_at: &str,
    downloaded_media: &[instagram_connector::DownloadedInstagramMedia],
) -> Result<usize, String> {
    Ok(downloaded_media
        .iter()
        .filter(|media| !is_profile_picture_file(&media.file_path))
        .count())
}

fn normalize_instagram_media_identity_key(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn strip_instagram_timestamp_prefix(value: &str) -> Option<&str> {
    let bytes = value.as_bytes();
    if bytes.len() <= 20 {
        return None;
    }

    let expected_digit_positions = [0usize, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18];
    if expected_digit_positions
        .iter()
        .any(|index| !bytes[*index].is_ascii_digit())
    {
        return None;
    }

    let expected_literals = [
        (4usize, b'-'),
        (7usize, b'-'),
        (10usize, b' '),
        (13usize, b'.'),
        (16usize, b'.'),
        (19usize, b' '),
    ];
    if expected_literals
        .iter()
        .any(|(index, expected)| bytes[*index] != *expected)
    {
        return None;
    }

    Some(&value[20..])
}

fn extract_instagram_media_identity_candidates_from_file_name(file_name: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();

    let Some(stem) = Path::new(file_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return candidates;
    };

    let mut push_candidate = |value: &str| {
        if let Some(candidate) = normalize_instagram_media_identity_key(value) {
            if seen.insert(candidate.clone()) {
                candidates.push(candidate);
            }
        }
    };

    push_candidate(stem);
    if let Some(trimmed) = strip_instagram_timestamp_prefix(stem) {
        push_candidate(trimmed);
    }

    candidates
}

fn extract_instagram_media_identity_candidates_from_path(path: &Path) -> Vec<String> {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(extract_instagram_media_identity_candidates_from_file_name)
        .unwrap_or_default()
}

fn derive_instagram_media_identity_key_from_path(path: &Path) -> Option<String> {
    extract_instagram_media_identity_candidates_from_path(path)
        .into_iter()
        .next()
}

fn load_existing_instagram_media_identity_keys_for_source(
    layout: &StorageLayout,
    source: &SourceProfile,
    settings: &HashMap<String, String>,
) -> Result<HashSet<String>, String> {
    let mut keys = HashSet::new();
    let source_options = source_instagram_sync_options(source);
    let profile_root = resolve_instagram_profile_root_with_options(
        layout,
        source,
        Some(settings),
        Some(&source_options),
    );
    let saved_posts_root = resolve_instagram_saved_posts_root(layout, Some(settings));

    for root in [profile_root, saved_posts_root] {
        if !root.exists() {
            continue;
        }

        for path in collect_media_file_paths(&root)? {
            if is_profile_picture_file(&path) || infer_media_type(&path).is_none() {
                continue;
            }

            for provider_media_key in extract_instagram_media_identity_candidates_from_path(&path) {
                keys.insert(provider_media_key);
            }
        }
    }

    Ok(keys)
}

fn load_existing_instagram_relative_media_paths_for_source(
    layout: &StorageLayout,
    source: &SourceProfile,
    settings: &HashMap<String, String>,
) -> Result<HashSet<String>, String> {
    let mut paths = HashSet::new();
    let source_options = source_instagram_sync_options(source);
    let profile_root = resolve_instagram_profile_root_with_options(
        layout,
        source,
        Some(settings),
        Some(&source_options),
    );

    if !profile_root.exists() {
        return Ok(paths);
    }

    for path in collect_media_file_paths(&profile_root)? {
        if is_profile_picture_file(&path) || infer_media_type(&path).is_none() {
            continue;
        }

        paths.insert(normalize_instagram_relative_media_path(
            &profile_root,
            &path,
        ));
    }

    Ok(paths)
}

fn load_instagram_post_ledger_snapshot_for_source(
    connection: &Connection,
    source_id: &str,
) -> Result<InstagramPostLedgerSnapshot, String> {
    ensure_instagram_sync_post_ledger_table(connection)?;
    let mut statement = connection
        .prepare(
            "SELECT provider_post_key, provider_post_code
             FROM instagram_sync_post_ledger
             WHERE source_id = ?1",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![source_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?;

    let mut snapshot = InstagramPostLedgerSnapshot::default();
    for row in rows {
        let (provider_post_key, provider_post_code) = row.map_err(|error| error.to_string())?;
        if !provider_post_key.trim().is_empty() {
            snapshot
                .keys
                .insert(normalize_instagram_post_ledger_key(&provider_post_key));
        }
        if !provider_post_code.trim().is_empty() {
            snapshot
                .keys
                .insert(normalize_instagram_post_ledger_key(&provider_post_code));
        }
    }

    Ok(snapshot)
}

/// Chaves de posts deletados pelo usuário (tombstone) para Instagram, na mesma
/// normalização do post-ledger. Usadas para suprimir re-download mesmo nas
/// seções que ignoram o post-ledger (highlights).
fn load_instagram_deleted_post_keys(
    connection: &Connection,
    source_id: &str,
) -> Result<HashSet<String>, String> {
    ensure_provider_deleted_media_table(connection)?;
    let mut statement = connection
        .prepare(
            "SELECT provider_post_key, provider_post_code
             FROM provider_deleted_media
             WHERE provider = 'instagram' AND source_id = ?1",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![source_id], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut keys = HashSet::new();
    for row in rows {
        let (post_key, post_code) = row.map_err(|error| error.to_string())?;
        for value in [post_key, post_code].into_iter().flatten() {
            let normalized = normalize_instagram_post_ledger_key(&value);
            if !normalized.is_empty() {
                keys.insert(normalized);
            }
        }
    }
    Ok(keys)
}

fn upsert_instagram_post_ledger_entries(
    connection: &Connection,
    source_id: &str,
    account_id: &str,
    source_handle: &str,
    observed_posts: &[instagram_connector::ObservedInstagramPost],
    timestamp: &str,
) -> Result<(), String> {
    ensure_instagram_sync_post_ledger_table(connection)?;

    for post in observed_posts {
        let provider_post_key = normalize_instagram_post_ledger_key(&post.provider_post_key);
        if provider_post_key.is_empty() {
            continue;
        }

        let provider_post_code = post
            .provider_post_code
            .as_deref()
            .map(normalize_instagram_post_ledger_key)
            .unwrap_or_default();

        connection
            .execute(
                "INSERT INTO instagram_sync_post_ledger (
                    source_id,
                    account_id,
                    source_handle,
                    provider_post_key,
                    provider_post_code,
                    media_section,
                    first_seen_at,
                    last_seen_at
                 )
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
                 ON CONFLICT(source_id, provider_post_key)
                 DO UPDATE SET
                    account_id = excluded.account_id,
                    source_handle = excluded.source_handle,
                    provider_post_code = excluded.provider_post_code,
                    media_section = excluded.media_section,
                    last_seen_at = excluded.last_seen_at",
                params![
                    source_id,
                    account_id,
                    source_handle,
                    provider_post_key,
                    provider_post_code,
                    &post.media_section,
                    timestamp,
                ],
            )
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn normalize_instagram_post_ledger_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

/// Baixa o avatar resolvido pelo connector do Twitter e o persiste como
/// ProfilePicture (raiz + Settings), retornando o caminho normalizado.
fn refresh_twitter_profile_picture(
    output_root: &Path,
    avatar_url: &str,
    user_agent: &str,
) -> Result<Option<String>, ProfilePictureRefreshError> {
    let trimmed = avatar_url.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let download_url = upgrade_twitter_avatar_url(trimmed);

    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|error| ProfilePictureRefreshError::warning(error.to_string()))?;
    let response = client
        .get(&download_url)
        .header(reqwest::header::USER_AGENT, user_agent)
        .send()
        .map_err(|error| {
            ProfilePictureRefreshError::warning(format!(
                "Failed to download Twitter profile picture: {error}"
            ))
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(ProfilePictureRefreshError::warning(format!(
            "Twitter profile picture download failed with status {status}."
        )));
    }
    let bytes = response.bytes().map_err(|error| {
        ProfilePictureRefreshError::warning(format!(
            "Failed to read Twitter profile picture bytes: {error}"
        ))
    })?;
    if bytes.is_empty() {
        return Err(ProfilePictureRefreshError::warning(
            "Twitter profile picture download returned an empty response.",
        ));
    }

    fs::create_dir_all(output_root)
        .map_err(|error| ProfilePictureRefreshError::warning(error.to_string()))?;
    let target_path = profile_picture_path(output_root);
    let temporary_path = output_root.join(format!("{PROFILE_PICTURE_FILE_NAME}.download"));
    fs::write(&temporary_path, bytes.as_ref())
        .map_err(|error| ProfilePictureRefreshError::warning(error.to_string()))?;
    if target_path.exists() {
        let _ = fs::remove_file(&target_path);
    }
    if let Err(rename_error) = fs::rename(&temporary_path, &target_path) {
        fs::copy(&temporary_path, &target_path).map_err(|copy_error| {
            ProfilePictureRefreshError::warning(format!(
                "Failed to persist profile picture: {copy_error}"
            ))
        })?;
        let _ = fs::remove_file(&temporary_path);
        if !target_path.exists() {
            return Err(ProfilePictureRefreshError::warning(format!(
                "Failed to persist profile picture after rename error: {rename_error}"
            )));
        }
    }

    let canonical_path = ensure_profile_picture_at_root(output_root, &target_path)?;
    normalize_media_file_path(&canonical_path)
        .map(Some)
        .map_err(ProfilePictureRefreshError::warning)
}

fn refresh_instagram_profile_picture(
    connection: &Connection,
    context: &SourceSyncContext,
    output_root: &Path,
    settings: &HashMap<String, String>,
) -> Result<Option<String>, ProfilePictureRefreshError> {
    if read_instagram_avatar_cooldown_until(settings).is_some_and(|until| until > Utc::now()) {
        return Ok(None);
    }

    let now = now_timestamp();
    let username = sanitize_source_handle("instagram", &context.source.handle)
        .trim_start_matches('@')
        .to_string();
    if username.is_empty() {
        return Err(ProfilePictureRefreshError::warning(
            "Instagram source handle is empty.",
        ));
    }

    let parsed_session = parse_session_payload(&context.session_payload)
        .map_err(ProfilePictureRefreshError::warning)?;
    let cookies = parsed_session.cookies;
    let cookie_header = build_cookie_header(&cookies);
    if cookie_header.is_empty() {
        return Err(ProfilePictureRefreshError::warning(
            "Instagram session payload does not include usable cookies.",
        ));
    }

    let auth_headers =
        build_instagram_auth_headers(settings, &cookies, Some(&parsed_session.metadata));
    let user_agent = auth_headers.user_agent.clone().unwrap_or_else(|| {
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36".to_string()
    });
    let referer = format!("https://www.instagram.com/{username}/");
    let csrf_token = auth_headers.csrf_token.as_deref().ok_or_else(|| {
        ProfilePictureRefreshError::warning(
            "Instagram session payload does not include a usable csrf token.",
        )
    })?;
    let ig_app_id = auth_headers
        .app_id
        .as_deref()
        .filter(|v| !v.trim().is_empty());
    let ig_asbd_id = auth_headers
        .asbd_id
        .as_deref()
        .filter(|v| !v.trim().is_empty());
    let ig_www_claim = auth_headers
        .ig_www_claim
        .as_deref()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or("0");
    let ig_lsd = auth_headers
        .lsd
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let ig_dtsg = auth_headers
        .dtsg
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());

    let client = reqwest::blocking::Client::builder()
        .timeout(StdDuration::from_secs(25))
        .build()
        .map_err(|error| {
            ProfilePictureRefreshError::warning(format!(
                "Failed to initialize HTTP client: {error}"
            ))
        })?;

    let mut topsearch_request = client
        .get("https://www.instagram.com/web/search/topsearch/")
        .query(&[("query", username.as_str())])
        .header(reqwest::header::ACCEPT, "application/json, text/plain, */*")
        .header(reqwest::header::COOKIE, cookie_header.clone())
        .header(reqwest::header::REFERER, referer.clone())
        .header(reqwest::header::USER_AGENT, user_agent.clone())
        .header("x-csrftoken", csrf_token)
        .header("x-ig-www-claim", ig_www_claim)
        .header("X-Requested-With", "XMLHttpRequest");
    if let Some(value) = ig_app_id {
        topsearch_request = topsearch_request.header("x-ig-app-id", value);
    }
    if let Some(value) = ig_asbd_id {
        topsearch_request = topsearch_request.header("x-asbd-id", value);
    }
    let topsearch_response = topsearch_request.send().map_err(|error| {
        ProfilePictureRefreshError::warning(format!(
            "Failed to resolve Instagram user id via topsearch: {error}"
        ))
    })?;
    let topsearch_status = topsearch_response.status();
    let topsearch_retry_after = parse_retry_after_duration(
        topsearch_response
            .headers()
            .get(reqwest::header::RETRY_AFTER),
    );
    let topsearch_body_text = topsearch_response.text().unwrap_or_default();

    if topsearch_status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let retry_after = topsearch_retry_after
            .unwrap_or_else(|| StdDuration::from_secs(INSTAGRAM_AVATAR_RETRY_AFTER_FALLBACK_SECS));
        let until =
            set_instagram_avatar_cooldown(connection, &context.account.id, retry_after, &now)
                .map_err(ProfilePictureRefreshError::warning)?;
        let detail = avatar_error_detail(
            &topsearch_body_text,
            ig_app_id,
            ig_asbd_id,
            ig_www_claim,
            csrf_token,
        );
        return Err(ProfilePictureRefreshError::info(format!(
            "Instagram topsearch request received 429 Too Many Requests. Next retry after {}.",
            until.to_rfc3339()
        ))
        .with_detail(detail));
    }

    if !topsearch_status.is_success() {
        let detail = avatar_error_detail(
            &topsearch_body_text,
            ig_app_id,
            ig_asbd_id,
            ig_www_claim,
            csrf_token,
        );
        return Err(ProfilePictureRefreshError::warning(format!(
            "Instagram topsearch request failed with status {topsearch_status}."
        ))
        .with_detail(detail));
    }

    let topsearch_payload: serde_json::Value =
        serde_json::from_str(&topsearch_body_text).map_err(|error| {
            ProfilePictureRefreshError::warning(format!(
                "Failed to parse Instagram topsearch response: {error}"
            ))
        })?;
    let topsearch_user =
        parse_instagram_topsearch_user(&topsearch_payload, &username).ok_or_else(|| {
            ProfilePictureRefreshError::warning(
                "Instagram topsearch response did not include a matching user.",
            )
        })?;

    let avatar_url = match (ig_lsd, ig_dtsg) {
        (Some(lsd), Some(dtsg)) => {
            match try_instagram_graphql_avatar(
                &client,
                &topsearch_user.user_id,
                lsd,
                dtsg,
                &cookie_header,
                &referer,
                &user_agent,
                csrf_token,
                ig_www_claim,
                ig_app_id,
                ig_asbd_id,
            ) {
                Ok(url) => url,
                Err(gql_error) => {
                    if let Some(fallback) = &topsearch_user.profile_pic_url {
                        fallback.clone()
                    } else {
                        return Err(gql_error);
                    }
                }
            }
        }
        _ => topsearch_user.profile_pic_url.ok_or_else(|| {
            ProfilePictureRefreshError::warning(
                "Instagram session does not include lsd/dtsg tokens and topsearch did not provide a profile picture URL.",
            )
        })?,
    };

    let avatar_response = client
        .get(&avatar_url)
        .header(reqwest::header::COOKIE, cookie_header)
        .header(reqwest::header::REFERER, referer)
        .header(reqwest::header::USER_AGENT, user_agent)
        .send()
        .map_err(|error| {
            ProfilePictureRefreshError::warning(format!(
                "Failed to download Instagram profile picture: {error}"
            ))
        })?;
    let avatar_status = avatar_response.status();
    if avatar_status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let retry_after =
            parse_retry_after_duration(avatar_response.headers().get(reqwest::header::RETRY_AFTER))
                .unwrap_or_else(|| {
                    StdDuration::from_secs(INSTAGRAM_AVATAR_RETRY_AFTER_FALLBACK_SECS)
                });
        let until =
            set_instagram_avatar_cooldown(connection, &context.account.id, retry_after, &now)
                .map_err(ProfilePictureRefreshError::warning)?;
        return Err(ProfilePictureRefreshError::info(format!(
            "Instagram profile picture download received 429 Too Many Requests. Next retry after {}.",
            until.to_rfc3339()
        )));
    }

    if !avatar_status.is_success() {
        return Err(ProfilePictureRefreshError::warning(format!(
            "Instagram profile picture download failed with status {avatar_status}."
        )));
    }

    let avatar_bytes = avatar_response.bytes().map_err(|error| {
        ProfilePictureRefreshError::warning(format!(
            "Failed to read Instagram profile picture bytes: {error}"
        ))
    })?;
    if avatar_bytes.is_empty() {
        return Err(ProfilePictureRefreshError::warning(
            "Instagram profile picture download returned an empty response.",
        ));
    }

    fs::create_dir_all(output_root)
        .map_err(|error| ProfilePictureRefreshError::warning(error.to_string()))?;
    let target_path = profile_picture_path(output_root);
    let temporary_path = output_root.join(format!("{PROFILE_PICTURE_FILE_NAME}.download"));

    fs::write(&temporary_path, avatar_bytes.as_ref())
        .map_err(|error| ProfilePictureRefreshError::warning(error.to_string()))?;
    if target_path.exists() {
        let _ = fs::remove_file(&target_path);
    }

    if let Err(rename_error) = fs::rename(&temporary_path, &target_path) {
        fs::copy(&temporary_path, &target_path).map_err(|copy_error| {
            ProfilePictureRefreshError::warning(format!(
                "Failed to persist profile picture: {copy_error}"
            ))
        })?;
        let _ = fs::remove_file(&temporary_path);
        if !target_path.exists() {
            return Err(ProfilePictureRefreshError::warning(format!(
                "Failed to persist profile picture after rename error: {rename_error}"
            )));
        }
    }

    clear_instagram_avatar_cooldown(connection, &context.account.id)
        .map_err(ProfilePictureRefreshError::warning)?;
    let canonical_path = ensure_profile_picture_at_root(output_root, &target_path)?;
    normalize_media_file_path(&canonical_path)
        .map(Some)
        .map_err(ProfilePictureRefreshError::warning)
}

fn parse_instagram_profile_picture_url(payload: &serde_json::Value) -> Option<String> {
    [
        "/data/user/hd_profile_pic_url_info/url",
        "/data/user/profile_pic_url_hd",
        "/data/user/profile_pic_url",
    ]
    .iter()
    .find_map(|pointer| payload.pointer(pointer).and_then(|value| value.as_str()))
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .map(ToOwned::to_owned)
}

fn parse_instagram_topsearch_user(
    payload: &serde_json::Value,
    username: &str,
) -> Option<TopSearchUserResult> {
    let target_username = username.trim();
    payload
        .pointer("/users")
        .and_then(|value| value.as_array())
        .and_then(|users| {
            users.iter().find_map(|entry| {
                let user = entry.get("user")?;
                let entry_username = user.get("username")?.as_str()?.trim();
                if !entry_username.eq_ignore_ascii_case(target_username) {
                    return None;
                }

                let json_value_to_id_string = |value: &serde_json::Value| {
                    value
                        .as_str()
                        .map(ToOwned::to_owned)
                        .or_else(|| value.as_u64().map(|n| n.to_string()))
                        .or_else(|| value.as_i64().map(|n| n.to_string()))
                        .filter(|s| !s.is_empty())
                };
                let user_id = user
                    .get("pk")
                    .and_then(json_value_to_id_string)
                    .or_else(|| user.get("pk_id").and_then(json_value_to_id_string))
                    .or_else(|| user.get("id").and_then(json_value_to_id_string))?;
                let profile_pic_url = user
                    .get("profile_pic_url")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(ToOwned::to_owned);
                Some(TopSearchUserResult {
                    user_id,
                    profile_pic_url,
                })
            })
        })
}

fn configure_background_command(command: &mut Command) {
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

fn update_instagram_source_handle_after_sync(
    connection: &Connection,
    source_id: &str,
    new_handle: &str,
    timestamp: &str,
) -> Result<(), String> {
    let normalized_handle = sanitize_source_handle("instagram", new_handle);
    if normalized_handle.is_empty() {
        return Err("Instagram source handle cannot be empty.".to_string());
    }

    // Registra o handle antigo na lista de nomes anteriores para que a busca
    // continue encontrando o perfil pelo nome de antes do rename.
    let existing: Option<(String, String)> = connection
        .query_row(
            "SELECT handle, sync_options_json FROM source_profiles WHERE id = ?1 AND deleted_at IS NULL",
            params![source_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;

    let updated_sync_options_json = existing.and_then(|(old_handle, sync_options_json)| {
        if sanitize_source_handle("instagram", &old_handle).eq_ignore_ascii_case(&normalized_handle)
        {
            return None;
        }
        let mut options = deserialize_source_sync_options("instagram", &sync_options_json);
        let instagram = options
            .instagram
            .get_or_insert_with(default_instagram_source_sync_options);
        instagram.previous_handles = push_previous_instagram_handle(
            instagram.previous_handles.take(),
            &old_handle,
            &normalized_handle,
        );
        serialize_source_sync_options("instagram", &options).ok()
    });

    match updated_sync_options_json {
        Some(sync_options_json) => {
            connection
                .execute(
                    "UPDATE source_profiles
                     SET handle = ?2,
                         sync_options_json = ?4,
                         updated_at = ?3
                     WHERE id = ?1
                       AND deleted_at IS NULL",
                    params![source_id, &normalized_handle, timestamp, sync_options_json],
                )
                .map_err(|error| error.to_string())?;
        }
        None => {
            connection
                .execute(
                    "UPDATE source_profiles
                     SET handle = ?2,
                         updated_at = ?3
                     WHERE id = ?1
                       AND deleted_at IS NULL",
                    params![source_id, &normalized_handle, timestamp],
                )
                .map_err(|error| error.to_string())?;
        }
    }

    Ok(())
}

/// Atualiza o handle de um perfil TikTok após detectarmos uma renomeação de
/// conta (o handle salvo parou de listar posts e um post conhecido resolveu
/// para outro `uniqueId` com o mesmo `author.id`). Diferente do Instagram, a
/// recuperação não depende de busca por nome — usamos o `userIdHint`/post id —,
/// então basta atualizar a coluna `handle`.
fn update_tiktok_source_handle_after_sync(
    connection: &Connection,
    source_id: &str,
    new_handle: &str,
    timestamp: &str,
) -> Result<(), String> {
    let normalized_handle = sanitize_source_handle("tiktok", new_handle)
        .trim_start_matches('@')
        .to_string();
    if normalized_handle.is_empty() {
        return Err("TikTok source handle cannot be empty.".to_string());
    }
    connection
        .execute(
            "UPDATE source_profiles
             SET handle = ?2,
                 updated_at = ?3
             WHERE id = ?1
               AND deleted_at IS NULL",
            params![source_id, &normalized_handle, timestamp],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Igual a [`update_tiktok_source_handle_after_sync`], para o Twitter: atualiza
/// a coluna `handle` após detectarmos um rename (resolvido via `userIdHint`).
fn update_twitter_source_handle_after_sync(
    connection: &Connection,
    source_id: &str,
    new_handle: &str,
    timestamp: &str,
) -> Result<(), String> {
    let normalized_handle = sanitize_source_handle("twitter", new_handle)
        .trim_start_matches('@')
        .to_string();
    if normalized_handle.is_empty() {
        return Err("Twitter source handle cannot be empty.".to_string());
    }
    connection
        .execute(
            "UPDATE source_profiles
             SET handle = ?2,
                 updated_at = ?3
             WHERE id = ?1
               AND deleted_at IS NULL",
            params![source_id, &normalized_handle, timestamp],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn update_instagram_source_description_after_sync(
    connection: &Connection,
    source: &SourceProfile,
    profile_description: &str,
    force_update: bool,
    timestamp: &str,
) -> Result<bool, String> {
    if source.provider != "instagram" {
        return Ok(false);
    }

    let next_description = profile_description.trim();
    if next_description.is_empty() {
        return Ok(false);
    }

    let mut options = source_instagram_sync_options(source);
    let current_description = options
        .description
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    let next_description_value = if current_description.is_empty() {
        next_description.to_string()
    } else if current_description.contains(next_description) {
        return Ok(false);
    } else if force_update {
        format!("{current_description}\n----\n{next_description}")
    } else {
        return Ok(false);
    };

    options.description = Some(next_description_value);
    let sync_options = SourceSyncOptions {
        instagram: Some(options),
        ..Default::default()
    };
    let serialized = serialize_source_sync_options(&source.provider, &sync_options)?;

    connection
        .execute(
            "UPDATE source_profiles
             SET sync_options_json = ?2,
                 updated_at = ?3
             WHERE id = ?1
               AND deleted_at IS NULL",
            params![&source.id, serialized, timestamp],
        )
        .map_err(|error| error.to_string())?;

    Ok(true)
}

fn validate_instagram_manual_session_payload(
    connection: &Connection,
    account_id: &str,
    secret_payload: &str,
) -> Result<(), String> {
    let parsed_session = parse_session_payload(secret_payload)?;
    let cookies = parsed_session.cookies;
    let metadata = parsed_session.metadata;
    if !cookies.iter().any(|cookie| {
        domain_matches_allowed(&cookie.domain, "instagram.com") && !cookie.value.trim().is_empty()
    }) {
        return Err("Instagram manual session is missing provider-owned cookies.".to_string());
    }

    let settings = load_provider_account_settings_map(connection, account_id)?;
    let csrf_token = settings
        .get("instagram.auth.csrfToken")
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            metadata
                .csrf_token
                .as_deref()
                .filter(|value| !value.trim().is_empty())
        })
        .or_else(|| {
            cookies
                .iter()
                .find(|cookie| cookie.name.eq_ignore_ascii_case("csrftoken"))
                .map(|cookie| cookie.value.as_str())
                .filter(|value| !value.trim().is_empty())
        });
    if csrf_token.is_none() {
        return Err("Instagram manual session is missing x-csrftoken.".to_string());
    }

    if settings
        .get("instagram.auth.appId")
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            metadata
                .app_id
                .as_deref()
                .filter(|value| !value.trim().is_empty())
        })
        .is_none()
    {
        return Err("Instagram manual session is missing x-ig-app-id.".to_string());
    }

    Ok(())
}

fn load_snapshot(
    connection: &Connection,
    layout: &StorageLayout,
) -> Result<WorkspaceSnapshot, String> {
    connector_runtime::ensure_catalog_state(connection, layout)?;
    let sources = load_sources(connection)?;
    let source_media_paths = compute_source_media_paths(connection, layout, &sources);
    Ok(WorkspaceSnapshot {
        workspace_root: layout.root.display().to_string(),
        db_path: layout.db_path.display().to_string(),
        media_root: layout.media_root.display().to_string(),
        provider_catalog: providers::provider_catalog(),
        accounts: load_accounts(connection)?,
        account_sessions: load_account_sessions(connection, layout)?,
        sources,
        source_sync_runs: load_source_sync_runs(connection)?,
        account_sync_runs: load_account_sync_runs(connection)?,
        scheduler_sets: load_scheduler_sets(connection)?,
        scheduler_groups: load_scheduler_groups(connection)?,
        sync_plan_runs: load_sync_plan_runs(connection)?,
        app_settings: load_app_settings(connection)?,
        connector_runtimes: connector_runtime::load_connector_runtime_statuses(connection)?,
        desktop_runtime: load_desktop_runtime_state(connection)?,
        source_media_paths,
    })
}

#[cfg(test)]
mod tests;

fn add_minutes_to_timestamp(value: &str, minutes: i64) -> Result<String, String> {
    Ok((parse_timestamp(value)? + Duration::minutes(minutes)).to_rfc3339())
}

fn latest_timestamp(values: Vec<String>) -> Result<String, String> {
    let mut timestamps = values
        .into_iter()
        .map(|value| parse_timestamp(&value).map(|timestamp| (value, timestamp)))
        .collect::<Result<Vec<_>, _>>()?;
    timestamps.sort_by(|left, right| left.1.cmp(&right.1));
    timestamps
        .pop()
        .map(|entry| entry.0)
        .ok_or_else(|| "At least one timestamp candidate is required.".to_string())
}

fn is_timestamp_due(candidate: &str, now: &str) -> Result<bool, String> {
    Ok(parse_timestamp(candidate)? <= parse_timestamp(now)?)
}

fn now_timestamp() -> String {
    Utc::now().to_rfc3339()
}

fn new_id() -> String {
    Uuid::new_v4().to_string()
}

fn to_json_array(values: &[String]) -> Result<String, String> {
    serde_json::to_string(values).map_err(|error| error.to_string())
}

fn from_json_array(value: String) -> Vec<String> {
    serde_json::from_str(&value).unwrap_or_default()
}

fn bool_to_int(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}
