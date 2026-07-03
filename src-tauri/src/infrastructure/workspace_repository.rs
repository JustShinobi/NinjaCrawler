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
    AccountSyncRun, AppSetting, AppSettingUpsert, BatchSourceProfilePatch, CloneSyncPlanInput,
    CompanionAccountCandidate, CompanionAccountCapture, CompanionAccountImportInput,
    CompanionAccountImportResult, CompanionAccountPreview, DesktopRuntimeState,
    ImportMethodDescriptor, ImportPreview, ImportPreviewOptions,
    ImportPreviewProfile, ImportPreviewSummary, ImportProblem, ImportProviderDescriptor,
    ImportRootDescriptor, ImportRunProfileResult, ImportRunRequest, ImportRunResult,
    InstagramExtractImageFromVideoSections, InstagramNamingLedgerBackfillResult,
    InstagramSourceSyncOptions, InstagramSyncOptionsPatch, MediaGalleryFile, MediaGalleryPost,
    MoveSyncPlanInput, ProviderAccount,
    ProviderAccountCookie, ProviderAccountCookieImport, ProviderAccountEditor,
    ProviderAccountImportState,
    ProviderAccountSession, ProviderAccountSettingValue, ProviderAccountSettingValueKind,
    ProviderAccountUpsert, RunSyncPlanNowInput, RuntimeLogContext, RuntimeLogEntry,
    RuntimeLogQuery, SchedulerGroup, SchedulerGroupUpsert, SchedulerPlanCriteria,
    SchedulerPlanNotifications, SchedulerSet, SchedulerSetUpsert, SetSyncPlanPauseInput,
    SkipSyncPlanInput, SourceAvailabilityCheckItem, SourceAvailabilityCheckResult,
    SingleVideo, SourceMediaGallery, SourceProfile,
    SourceProfileDeleteMode, SourceProfileUpsert, SourceSyncOptions, SourceSyncRun, SyncPlan,
    SyncPlanRun, SyncPlanTargetPreview, SyncPlanTargetPreviewInput, SyncPlanTargetPreviewSource,
    SyncPlanUpsert, TikTokSourceSyncOptions, TwitterSourceSyncOptions, WorkspaceSnapshot,
};
use crate::domain::models::{
    default_tiktok_source_sync_options, default_twitter_source_sync_options,
};
use crate::infrastructure::storage::StorageLayout;
use crate::infrastructure::{
    connector_debug, connector_runtime, database, instagram_connector, runtime_log, session_secret_store,
    source_sync_runtime, storage, tiktok_connector, twitter_connector,
};
use crate::providers;

pub const DESKTOP_CLOSE_TO_TRAY_SETTING_KEY: &str = "policy.desktop.close_to_tray";
pub const DESKTOP_SILENT_MODE_SETTING_KEY: &str = "policy.desktop.silent_mode";
const PROFILE_PICTURE_FILE_NAME: &str = "ProfilePicture.jpg";
const PROFILE_SETTINGS_DIR_NAME: &str = "Settings";
const INSTAGRAM_AVATAR_RETRY_AFTER_FALLBACK_SECS: u64 = 20 * 60;
const INSTAGRAM_AVATAR_COOLDOWN_UNTIL_SETTING_KEY: &str = "instagram.avatar.cooldownUntil";
const INSTAGRAM_SYNC_RETRY_AFTER_FALLBACK_SECS: i64 = 10 * 60;
const INSTAGRAM_SYNC_COOLDOWN_UNTIL_SETTING_KEY: &str = "instagram.sync.cooldownUntil";
const INSTAGRAM_PUBLIC_APP_ID: &str = "936619743392459";
const INSTAGRAM_PUBLIC_ASBD_ID: &str = "129477";
const INSTAGRAM_PUBLIC_IG_CLAIM: &str = "0";
const INSTAGRAM_PUBLIC_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";
const INSTAGRAM_NAMING_LEDGER_BACKFILL_SETTING_KEY: &str =
    "runtime.instagram.naming_ledger_backfilled_v1";
const SOURCE_SYNC_PROGRESS_POLL_MS: u64 = 900;
const INSTAGRAM_SCRAWLER_IMPORTER_ID: &str = "instagram.scrawler";
/// Toggle (Settings) que liga/desliga o cancelamento+remoção automática quando
/// um perfil novo resolve, no primeiro sync, para um user id já cadastrado.
const DUPLICATE_USER_ID_BLOCK_SETTING_KEY: &str = "policy.sync.blockDuplicateUserId";
/// Segundos de espera entre cada download da fila (throttle global). Protege
/// provedores com rate limit baixo (ex.: Twitter). 0 = sem espera.
const SYNC_DELAY_BETWEEN_PROFILES_SETTING_KEY: &str = "policy.sync.delayBetweenProfilesSecs";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Clone)]
struct LegacyInstagramProfileXml {
    account_name: Option<String>,
    user_id: Option<String>,
    user_name: Option<String>,
    true_name: Option<String>,
    friendly_name: Option<String>,
    user_site_name: Option<String>,
    description: Option<String>,
    ready_for_download: bool,
    get_timeline: bool,
    get_reels: bool,
    get_stories: bool,
    get_stories_user: bool,
    get_tagged_data: bool,
}

#[derive(Clone)]
struct ImportCandidateProfile {
    profile_root: PathBuf,
    user_xml_path: PathBuf,
    folder_name: String,
    profile: LegacyInstagramProfileXml,
}

#[derive(Clone)]
struct LegacyInstagramMediaXmlEntry {
    file_name: String,
    provider_post_key: String,
    media_url: String,
    special_folder: Option<String>,
    post_permalink: Option<String>,
}

#[derive(Clone)]
struct LegacyInstagramReconciliationRecord {
    file_path: PathBuf,
    legacy_file_name: String,
    provider_media_key: String,
    alias_keys: Vec<(String, String)>,
    file_sha256: Option<String>,
    provider_post_key: String,
    /// Normalized (lowercased) shortcode used for dedupe/aliases.
    provider_post_code: Option<String>,
    /// Shortcode preserving original casing, used to rebuild the post URL
    /// (Instagram shortcodes are case-sensitive).
    provider_post_code_cased: Option<String>,
    media_type: String,
    media_section: String,
}

#[derive(Default)]
struct LegacyInstagramReconciliationStats {
    seeded_media_entries: u32,
    seeded_post_entries: u32,
}

#[derive(Default)]
struct InstagramMediaLedgerSnapshot {
    media_keys: HashSet<String>,
    relative_paths: HashSet<String>,
}

#[derive(Default)]
struct InstagramMediaAliasSnapshot {
    keys: HashSet<String>,
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

fn default_instagram_source_sync_options() -> InstagramSourceSyncOptions {
    InstagramSourceSyncOptions::default()
}

fn normalize_instagram_source_sync_options(
    options: Option<InstagramSourceSyncOptions>,
) -> InstagramSourceSyncOptions {
    let mut normalized = options.unwrap_or_else(default_instagram_source_sync_options);
    normalized.temporary = Some(normalized.temporary.unwrap_or(false));
    normalized.favorite = Some(normalized.favorite.unwrap_or(false));
    normalized.download_images = Some(normalized.download_images.unwrap_or(true));
    normalized.download_videos = Some(normalized.download_videos.unwrap_or(true));
    normalized.get_user_media_only = Some(normalized.get_user_media_only.unwrap_or(false));
    normalized.missing_only = Some(normalized.missing_only.unwrap_or(false));
    normalized.date_from = Some(
        normalized
            .date_from
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_string(),
    );
    normalized.date_to = Some(
        normalized
            .date_to
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_string(),
    );
    normalized.verified_profile = Some(normalized.verified_profile.unwrap_or(true));
    normalized.force_update_user_name = Some(normalized.force_update_user_name.unwrap_or(true));
    normalized.force_update_user_information =
        Some(normalized.force_update_user_information.unwrap_or(false));
    normalized.extract_image_from_video = Some(
        normalized
            .extract_image_from_video
            .clone()
            .unwrap_or_else(InstagramExtractImageFromVideoSections::default),
    );
    normalized.place_extracted_image_into_video_folder = Some(
        normalized
            .place_extracted_image_into_video_folder
            .unwrap_or(false),
    );
    normalized.download_text = Some(normalized.download_text.unwrap_or(false));
    normalized.download_text_posts = Some(normalized.download_text_posts.unwrap_or(false));
    normalized.target_story_media_id = normalized
        .target_story_media_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    normalized.text_special_folder = Some(normalized.text_special_folder.unwrap_or(true));
    normalized.special_path = Some(
        normalized
            .special_path
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_string(),
    );
    normalized.username_override = Some(
        normalized
            .username_override
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_string(),
    );
    normalized.script_enabled = Some(normalized.script_enabled.unwrap_or(false));
    normalized.script = Some(normalized.script.unwrap_or_default());
    normalized.description = Some(normalized.description.unwrap_or_default());
    normalized.color = Some(normalized.color.unwrap_or_default());
    normalized
}

fn default_source_sync_options(provider: &str) -> SourceSyncOptions {
    if provider.eq_ignore_ascii_case("instagram") {
        SourceSyncOptions {
            instagram: Some(normalize_instagram_source_sync_options(None)),
            ..Default::default()
        }
    } else if provider.eq_ignore_ascii_case("twitter") {
        SourceSyncOptions {
            twitter: Some(normalize_twitter_source_sync_options(None)),
            ..Default::default()
        }
    } else if provider.eq_ignore_ascii_case("tiktok") {
        SourceSyncOptions {
            tiktok: Some(normalize_tiktok_source_sync_options(None)),
            ..Default::default()
        }
    } else {
        SourceSyncOptions::default()
    }
}

fn normalize_source_sync_options(provider: &str, options: &SourceSyncOptions) -> SourceSyncOptions {
    if provider.eq_ignore_ascii_case("instagram") {
        SourceSyncOptions {
            instagram: Some(normalize_instagram_source_sync_options(
                options.instagram.clone(),
            )),
            ..Default::default()
        }
    } else if provider.eq_ignore_ascii_case("twitter") {
        SourceSyncOptions {
            twitter: Some(normalize_twitter_source_sync_options(options.twitter.clone())),
            ..Default::default()
        }
    } else if provider.eq_ignore_ascii_case("tiktok") {
        SourceSyncOptions {
            tiktok: Some(normalize_tiktok_source_sync_options(options.tiktok.clone())),
            ..Default::default()
        }
    } else {
        SourceSyncOptions::default()
    }
}

/// Preenche campos ausentes com os defaults do TikTok (espelho do SCrawler),
/// preservando os valores já persistidos.
fn normalize_tiktok_source_sync_options(
    options: Option<TikTokSourceSyncOptions>,
) -> TikTokSourceSyncOptions {
    let defaults = default_tiktok_source_sync_options();
    let mut merged = options.unwrap_or_else(default_tiktok_source_sync_options);
    merged.get_timeline = merged.get_timeline.or(defaults.get_timeline);
    merged.get_stories_user = merged.get_stories_user.or(defaults.get_stories_user);
    merged.get_reposts = merged.get_reposts.or(defaults.get_reposts);
    merged.download_videos = merged.download_videos.or(defaults.download_videos);
    merged.download_photos = merged.download_photos.or(defaults.download_photos);
    merged.use_native_title = merged.use_native_title.or(defaults.use_native_title);
    merged.add_video_id_to_title = merged
        .add_video_id_to_title
        .or(defaults.add_video_id_to_title);
    merged.remove_tags_from_title = merged
        .remove_tags_from_title
        .or(defaults.remove_tags_from_title);
    merged.tokkit_file_naming = merged.tokkit_file_naming.or(defaults.tokkit_file_naming);
    merged.use_parsed_video_date = merged
        .use_parsed_video_date
        .or(defaults.use_parsed_video_date);
    merged.separate_video_folder = merged
        .separate_video_folder
        .or(defaults.separate_video_folder);
    merged.abort_on_limit = merged.abort_on_limit.or(defaults.abort_on_limit);
    merged.sleep_timer_secs = merged.sleep_timer_secs.or(defaults.sleep_timer_secs);
    merged.temporary = merged.temporary.or(defaults.temporary);
    merged.special_path = merged.special_path.or(defaults.special_path);
    merged.description = merged.description.or(defaults.description);
    merged.color = merged.color.or(defaults.color);
    merged.user_id_hint = merged.user_id_hint.or(defaults.user_id_hint);
    merged
}

/// Preenche campos ausentes com os defaults do provider (espelho dos defaults
/// do SCrawler), preservando os valores já persistidos.
fn normalize_twitter_source_sync_options(
    options: Option<TwitterSourceSyncOptions>,
) -> TwitterSourceSyncOptions {
    let defaults = default_twitter_source_sync_options();
    let mut merged = options.unwrap_or_else(default_twitter_source_sync_options);
    merged.media_model = merged.media_model.or(defaults.media_model);
    merged.profile_model = merged.profile_model.or(defaults.profile_model);
    merged.search_model = merged.search_model.or(defaults.search_model);
    merged.likes_model = merged.likes_model.or(defaults.likes_model);
    merged.search_use_graphql_endpoint = merged
        .search_use_graphql_endpoint
        .or(defaults.search_use_graphql_endpoint);
    merged.profile_use_graphql_endpoint = merged
        .profile_use_graphql_endpoint
        .or(defaults.profile_use_graphql_endpoint);
    merged.allow_non_user_tweets = merged
        .allow_non_user_tweets
        .or(defaults.allow_non_user_tweets);
    merged.abort_on_limit = merged.abort_on_limit.or(defaults.abort_on_limit);
    merged.download_already_parsed = merged
        .download_already_parsed
        .or(defaults.download_already_parsed);
    merged.sleep_timer_secs = merged.sleep_timer_secs.or(defaults.sleep_timer_secs);
    merged.sleep_timer_before_first_secs = merged
        .sleep_timer_before_first_secs
        .or(defaults.sleep_timer_before_first_secs);
    merged.download_images = merged.download_images.or(defaults.download_images);
    merged.download_videos = merged.download_videos.or(defaults.download_videos);
    merged.download_gifs = merged.download_gifs.or(defaults.download_gifs);
    merged.separate_video_folder = merged
        .separate_video_folder
        .or(defaults.separate_video_folder);
    merged.gifs_special_folder = merged.gifs_special_folder.or(defaults.gifs_special_folder);
    merged.gifs_prefix = merged.gifs_prefix.or(defaults.gifs_prefix);
    merged.use_md5_comparison = merged.use_md5_comparison.or(defaults.use_md5_comparison);
    merged.temporary = merged.temporary.or(defaults.temporary);
    merged.special_path = merged.special_path.or(defaults.special_path);
    merged.description = merged.description.or(defaults.description);
    merged.color = merged.color.or(defaults.color);
    merged.user_id_hint = merged.user_id_hint.or(defaults.user_id_hint);
    merged
}

fn source_twitter_sync_options(source: &SourceProfile) -> TwitterSourceSyncOptions {
    normalize_source_sync_options(&source.provider, &source.sync_options)
        .twitter
        .unwrap_or_else(|| normalize_twitter_source_sync_options(None))
}

fn source_tiktok_sync_options(source: &SourceProfile) -> TikTokSourceSyncOptions {
    normalize_source_sync_options(&source.provider, &source.sync_options)
        .tiktok
        .unwrap_or_else(|| normalize_tiktok_source_sync_options(None))
}

/// Grava a identidade estável do Instagram diretamente no perfil. O histórico
/// de sync continua sendo uma fonte de recuperação para instalações antigas,
/// mas não deve ser a única âncora porque o schema dos resumos evolui.
fn persist_instagram_user_id_hint(
    connection: &Connection,
    source_id: &str,
    user_id: &str,
    timestamp: &str,
) -> Result<(), String> {
    let user_id = user_id.trim();
    if user_id.is_empty() {
        return Ok(());
    }
    let Some(json) = connection
        .query_row(
            "SELECT sync_options_json FROM source_profiles WHERE id = ?1 AND deleted_at IS NULL",
            params![source_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
    else {
        return Ok(());
    };
    let mut options = deserialize_source_sync_options("instagram", &json);
    let instagram = options
        .instagram
        .get_or_insert_with(default_instagram_source_sync_options);
    if let Some(existing_user_id) = instagram
        .user_id_hint
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if existing_user_id != user_id {
            let history_user_id =
                load_latest_instagram_profile_user_id_hint(connection, source_id)?;
            if history_user_id.as_deref() != Some(user_id) {
                return Err(format!(
                    "Instagram identity mismatch for source '{source_id}': stored user id \
                     '{existing_user_id}', resolved user id '{user_id}'."
                ));
            }
        } else {
            return Ok(());
        }
    }

    instagram.user_id_hint = Some(user_id.to_string());
    let serialized = serialize_source_sync_options("instagram", &options)?;
    connection
        .execute(
            "UPDATE source_profiles SET sync_options_json = ?2, updated_at = ?3
             WHERE id = ?1 AND deleted_at IS NULL",
            params![source_id, serialized, timestamp],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Grava o `userIdHint` do Twitter no perfil após o primeiro sync bem-sucedido,
/// quando ainda não havia um. Permite detectar renames e duplicatas futuras.
fn persist_twitter_user_id_hint(
    connection: &Connection,
    source_id: &str,
    user_id: &str,
    timestamp: &str,
) -> Result<(), String> {
    let user_id = user_id.trim();
    if user_id.is_empty() {
        return Ok(());
    }
    let Some(json) = connection
        .query_row(
            "SELECT sync_options_json FROM source_profiles WHERE id = ?1 AND deleted_at IS NULL",
            params![source_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
    else {
        return Ok(());
    };
    let mut options =
        serde_json::from_str::<SourceSyncOptions>(&json).unwrap_or_else(|_| SourceSyncOptions {
            twitter: Some(normalize_twitter_source_sync_options(None)),
            ..Default::default()
        });
    let twitter = options
        .twitter
        .get_or_insert_with(|| normalize_twitter_source_sync_options(None));
    if twitter
        .user_id_hint
        .as_deref()
        .map(str::trim)
        .map(|value| !value.is_empty())
        .unwrap_or(false)
    {
        return Ok(()); // já preenchido, não sobrescreve
    }
    twitter.user_id_hint = Some(user_id.to_string());
    let serialized = serialize_source_sync_options("twitter", &options)?;
    connection
        .execute(
            "UPDATE source_profiles SET sync_options_json = ?2, updated_at = ?3
             WHERE id = ?1 AND deleted_at IS NULL",
            params![source_id, serialized, timestamp],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Grava o `userIdHint` do TikTok (uploader_id) após o primeiro sync, quando
/// ainda não havia um. Permite detectar renames e duplicatas futuras.
fn persist_tiktok_user_id_hint(
    connection: &Connection,
    source_id: &str,
    user_id: &str,
    timestamp: &str,
) -> Result<(), String> {
    let user_id = user_id.trim();
    if user_id.is_empty() {
        return Ok(());
    }
    let Some(json) = connection
        .query_row(
            "SELECT sync_options_json FROM source_profiles WHERE id = ?1 AND deleted_at IS NULL",
            params![source_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
    else {
        return Ok(());
    };
    let mut options =
        serde_json::from_str::<SourceSyncOptions>(&json).unwrap_or_else(|_| SourceSyncOptions {
            tiktok: Some(normalize_tiktok_source_sync_options(None)),
            ..Default::default()
        });
    let tiktok = options
        .tiktok
        .get_or_insert_with(|| normalize_tiktok_source_sync_options(None));
    if tiktok
        .user_id_hint
        .as_deref()
        .map(str::trim)
        .map(|value| !value.is_empty())
        .unwrap_or(false)
    {
        return Ok(());
    }
    tiktok.user_id_hint = Some(user_id.to_string());
    let serialized = serialize_source_sync_options("tiktok", &options)?;
    connection
        .execute(
            "UPDATE source_profiles SET sync_options_json = ?2, updated_at = ?3
             WHERE id = ?1 AND deleted_at IS NULL",
            params![source_id, serialized, timestamp],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Preserva metadados internos do Twitter (`user_id_hint` e `special_path`) que
/// a UI não reenviar em todo upsert, evitando que edições os apaguem do perfil.
fn preserve_persisted_twitter_metadata(
    connection: &Connection,
    id: &str,
    input: &mut SourceProfileUpsert,
) {
    if !input.provider.eq_ignore_ascii_case("twitter") {
        return;
    }

    let incoming = input.sync_options.twitter.as_ref();
    let incoming_hint_present = incoming
        .and_then(|twitter| twitter.user_id_hint.as_deref())
        .map(str::trim)
        .map(|value| !value.is_empty())
        .unwrap_or(false);
    let incoming_special_present = incoming
        .and_then(|twitter| twitter.special_path.as_deref())
        .map(str::trim)
        .map(|value| !value.is_empty())
        .unwrap_or(false);
    if incoming_hint_present && incoming_special_present {
        return;
    }

    let persisted = connection
        .query_row(
            "SELECT sync_options_json FROM source_profiles WHERE id = ?1 AND deleted_at IS NULL",
            params![id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str::<SourceSyncOptions>(&json).ok())
        .and_then(|options| options.twitter);
    let Some(persisted) = persisted else {
        return;
    };

    let twitter = input
        .sync_options
        .twitter
        .get_or_insert_with(default_twitter_source_sync_options);
    if !incoming_hint_present {
        if let Some(hint) = persisted.user_id_hint.filter(|value| !value.trim().is_empty()) {
            twitter.user_id_hint = Some(hint);
        }
    }
    if !incoming_special_present {
        if let Some(special) = persisted.special_path.filter(|value| !value.trim().is_empty()) {
            twitter.special_path = Some(special);
        }
    }
}

/// Igual ao `preserve_persisted_twitter_metadata`, mas para o TikTok.
fn preserve_persisted_tiktok_metadata(
    connection: &Connection,
    id: &str,
    input: &mut SourceProfileUpsert,
) {
    if !input.provider.eq_ignore_ascii_case("tiktok") {
        return;
    }

    let incoming = input.sync_options.tiktok.as_ref();
    let incoming_hint_present = incoming
        .and_then(|tiktok| tiktok.user_id_hint.as_deref())
        .map(str::trim)
        .map(|value| !value.is_empty())
        .unwrap_or(false);
    let incoming_special_present = incoming
        .and_then(|tiktok| tiktok.special_path.as_deref())
        .map(str::trim)
        .map(|value| !value.is_empty())
        .unwrap_or(false);
    if incoming_hint_present && incoming_special_present {
        return;
    }

    let persisted = connection
        .query_row(
            "SELECT sync_options_json FROM source_profiles WHERE id = ?1 AND deleted_at IS NULL",
            params![id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str::<SourceSyncOptions>(&json).ok())
        .and_then(|options| options.tiktok);
    let Some(persisted) = persisted else {
        return;
    };

    let tiktok = input
        .sync_options
        .tiktok
        .get_or_insert_with(default_tiktok_source_sync_options);
    if !incoming_hint_present {
        if let Some(hint) = persisted.user_id_hint.filter(|value| !value.trim().is_empty()) {
            tiktok.user_id_hint = Some(hint);
        }
    }
    if !incoming_special_present {
        if let Some(special) = persisted.special_path.filter(|value| !value.trim().is_empty()) {
            tiktok.special_path = Some(special);
        }
    }
}

fn serialize_source_sync_options(
    provider: &str,
    options: &SourceSyncOptions,
) -> Result<String, String> {
    serde_json::to_string(&normalize_source_sync_options(provider, options))
        .map_err(|error| error.to_string())
}

fn deserialize_source_sync_options(provider: &str, raw: &str) -> SourceSyncOptions {
    serde_json::from_str::<SourceSyncOptions>(raw)
        .map(|value| normalize_source_sync_options(provider, &value))
        .unwrap_or_else(|_| default_source_sync_options(provider))
}

fn source_instagram_sync_options(source: &SourceProfile) -> InstagramSourceSyncOptions {
    normalize_source_sync_options(&source.provider, &source.sync_options)
        .instagram
        .unwrap_or_else(default_instagram_source_sync_options)
}

fn instagram_handles_present(handles: Option<&Vec<String>>) -> bool {
    handles.map(|list| !list.is_empty()).unwrap_or(false)
}

/// Acrescenta `old_handle` à lista de handles anteriores, normalizando e
/// evitando duplicatas ou o próprio handle atual. Usado quando um perfil do
/// Instagram é renomeado ou importado com um nome legado.
fn push_previous_instagram_handle(
    existing: Option<Vec<String>>,
    old_handle: &str,
    current_handle: &str,
) -> Option<Vec<String>> {
    let normalized_old = sanitize_source_handle("instagram", old_handle);
    let normalized_current = sanitize_source_handle("instagram", current_handle);
    if normalized_old.is_empty() || normalized_old.eq_ignore_ascii_case(&normalized_current) {
        return existing;
    }

    let mut list = existing.unwrap_or_default();
    let already = list.iter().any(|handle| {
        sanitize_source_handle("instagram", handle).eq_ignore_ascii_case(&normalized_old)
    });
    if !already {
        list.push(normalized_old);
    }
    Some(list)
}

/// Garante que metadados internos do Instagram (user id e handles anteriores)
/// não sejam perdidos quando o payload de upsert (vindo da UI ou de um sync com
/// override) não os inclui. São controlados internamente, então só um valor
/// novo não-vazio os sobrescreve.
fn preserve_persisted_instagram_metadata(
    connection: &Connection,
    id: &str,
    input: &mut SourceProfileUpsert,
) {
    if !input.provider.eq_ignore_ascii_case("instagram") {
        return;
    }

    let incoming = input.sync_options.instagram.as_ref();
    let incoming_hint_present = incoming
        .and_then(|instagram| instagram.user_id_hint.as_deref())
        .map(str::trim)
        .map(|value| !value.is_empty())
        .unwrap_or(false);
    let incoming_prev_present =
        instagram_handles_present(incoming.and_then(|instagram| instagram.previous_handles.as_ref()));
    if incoming_hint_present && incoming_prev_present {
        return;
    }

    let persisted = connection
        .query_row(
            "SELECT sync_options_json FROM source_profiles WHERE id = ?1 AND deleted_at IS NULL",
            params![id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str::<SourceSyncOptions>(&json).ok())
        .and_then(|options| options.instagram);
    let Some(persisted) = persisted else {
        return;
    };

    let needs_hint = !incoming_hint_present
        && persisted
            .user_id_hint
            .as_deref()
            .map(str::trim)
            .map(|value| !value.is_empty())
            .unwrap_or(false);
    let needs_prev =
        !incoming_prev_present && instagram_handles_present(persisted.previous_handles.as_ref());
    if !needs_hint && !needs_prev {
        return;
    }

    let instagram = input
        .sync_options
        .instagram
        .get_or_insert_with(default_instagram_source_sync_options);
    if needs_hint {
        instagram.user_id_hint = persisted.user_id_hint;
    }
    if needs_prev {
        instagram.previous_handles = persisted.previous_handles;
    }
}

fn source_instagram_sync_options_with_override(
    source: &SourceProfile,
    sync_options_override: Option<&SourceSyncOptions>,
) -> InstagramSourceSyncOptions {
    let persisted = source_instagram_sync_options(source);
    if let Some(override_options) = sync_options_override {
        let mut merged = normalize_source_sync_options(&source.provider, override_options)
            .instagram
            .unwrap_or_else(default_instagram_source_sync_options);
        // `user_id_hint`, `special_path` e `previous_handles` são metadados
        // internos que a UI não controla; o override (ex.: presets) não os
        // envia. Sem preservá-los do perfil persistido, perderíamos o user id
        // que resolve perfis renomeados, o caminho de mídia importada e os
        // nomes antigos usados na busca.
        if merged
            .user_id_hint
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
        {
            merged.user_id_hint = persisted.user_id_hint.clone();
        }
        if merged
            .special_path
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
        {
            merged.special_path = persisted.special_path.clone();
        }
        if !instagram_handles_present(merged.previous_handles.as_ref()) {
            merged.previous_handles = persisted.previous_handles.clone();
        }
        return merged;
    }

    persisted
}

fn instagram_force_update_user_name_enabled(options: &InstagramSourceSyncOptions) -> bool {
    options.force_update_user_name.unwrap_or(true)
}

fn instagram_force_update_user_information_enabled(options: &InstagramSourceSyncOptions) -> bool {
    options.force_update_user_information.unwrap_or(false)
}

fn instagram_profile_script_pattern(options: &InstagramSourceSyncOptions) -> Option<String> {
    if !options.script_enabled.unwrap_or(false) {
        return None;
    }

    options
        .script
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn instagram_user_id_hint(options: &InstagramSourceSyncOptions) -> Option<&str> {
    options
        .user_id_hint
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn preferred_instagram_user_id_hint(
    persisted_hint: Option<&str>,
    latest_successful_hint: Option<&str>,
) -> Option<String> {
    // Um sync concluído prova qual conta produziu a mídia deste source. O hint
    // persistido normalmente é igual, mas imports legados podem trazer UserID
    // obsoleto ou incorreto; nesse conflito, o histórico confirmado é a âncora
    // mais forte. Sem histórico, preservamos o hint persistido para recuperar
    // contas renomeadas antes do primeiro sync no NinjaCrawler.
    latest_successful_hint
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            persisted_hint
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .map(str::to_string)
}

fn instagram_special_path(options: &InstagramSourceSyncOptions) -> Option<&str> {
    options
        .special_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn instagram_username_override(options: &InstagramSourceSyncOptions) -> Option<&str> {
    options
        .username_override
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn instagram_missing_only_enabled(options: &InstagramSourceSyncOptions) -> bool {
    options.missing_only.unwrap_or(false)
}

fn parse_instagram_sync_date_boundary(raw: Option<&str>, end_of_day: bool) -> Option<i64> {
    let value = raw?.trim();
    if value.is_empty() {
        return None;
    }

    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
    let date_time = if end_of_day {
        date.and_hms_opt(23, 59, 59)?
    } else {
        date.and_hms_opt(0, 0, 0)?
    };
    Some(date_time.and_utc().timestamp())
}

fn instagram_date_from_timestamp(options: &InstagramSourceSyncOptions) -> Option<i64> {
    parse_instagram_sync_date_boundary(options.date_from.as_deref(), false)
}

fn instagram_date_to_timestamp(options: &InstagramSourceSyncOptions) -> Option<i64> {
    parse_instagram_sync_date_boundary(options.date_to.as_deref(), true)
}

fn implicit_instagram_imported_cutoff_timestamp(
    source: &SourceProfile,
    run_mode: Option<&str>,
) -> Option<i64> {
    if run_mode.is_some_and(|value| value.eq_ignore_ascii_case("force_imported_backfill")) {
        return None;
    }

    if !source
        .importer_id
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case(INSTAGRAM_SCRAWLER_IMPORTER_ID))
    {
        return None;
    }

    source
        .imported_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.timestamp())
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

fn validate_source_sync_override(
    source: &SourceProfile,
    sync_options_override: Option<&SourceSyncOptions>,
) -> Result<(), String> {
    let Some(override_options) = sync_options_override else {
        return Ok(());
    };

    if source.provider.eq_ignore_ascii_case("instagram") {
        if override_options.instagram.is_none() {
            return Err(
                "Instagram sync override must include instagram section options.".to_string(),
            );
        }
        return Ok(());
    }

    if source.provider.eq_ignore_ascii_case("twitter") {
        if override_options.twitter.is_none() {
            return Err("Twitter sync override must include twitter section options.".to_string());
        }
        return Ok(());
    }

    if override_options.instagram.is_some() || override_options.twitter.is_some() {
        return Err(format!(
            "Sync overrides are only supported for instagram and twitter sources. Source '{}' uses '{}'.",
            source.handle, source.provider
        ));
    }

    Ok(())
}

fn source_sync_cancel_registry() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_source_sync_cancel_token(source_id: &str) -> Arc<AtomicBool> {
    let token = Arc::new(AtomicBool::new(false));
    if let Ok(mut registry) = source_sync_cancel_registry().lock() {
        registry.insert(source_id.to_string(), Arc::clone(&token));
    }
    token
}

fn clear_source_sync_cancel_token(source_id: &str) {
    if let Ok(mut registry) = source_sync_cancel_registry().lock() {
        registry.remove(source_id);
    }
}

pub fn request_source_sync_cancel(source_id: &str) -> bool {
    if let Ok(registry) = source_sync_cancel_registry().lock() {
        if let Some(token) = registry.get(source_id) {
            token.store(true, Ordering::SeqCst);
            return true;
        }
    }

    false
}

#[derive(Clone, Copy)]
enum ProfilePictureRefreshLogLevel {
    Info,
    Warning,
}

impl ProfilePictureRefreshLogLevel {
    fn as_str(self) -> &'static str {
        match self {
            ProfilePictureRefreshLogLevel::Info => "info",
            ProfilePictureRefreshLogLevel::Warning => "warning",
        }
    }
}

struct ProfilePictureRefreshError {
    level: ProfilePictureRefreshLogLevel,
    message: String,
    detail: Option<String>,
}

impl ProfilePictureRefreshError {
    fn info(message: impl Into<String>) -> Self {
        Self {
            level: ProfilePictureRefreshLogLevel::Info,
            message: message.into(),
            detail: None,
        }
    }

    fn warning(message: impl Into<String>) -> Self {
        Self {
            level: ProfilePictureRefreshLogLevel::Warning,
            message: message.into(),
            detail: None,
        }
    }

    fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

fn parse_retry_after_duration(value: Option<&reqwest::header::HeaderValue>) -> Option<StdDuration> {
    let raw = value?.to_str().ok()?.trim();
    if raw.is_empty() {
        return None;
    }

    if let Ok(seconds) = raw.parse::<u64>() {
        return Some(StdDuration::from_secs(seconds.max(1)));
    }

    let parsed = DateTime::parse_from_rfc2822(raw)
        .or_else(|_| DateTime::parse_from_rfc3339(raw))
        .ok()?
        .with_timezone(&Utc);
    let remaining = parsed.signed_duration_since(Utc::now()).num_seconds();
    if remaining <= 0 {
        return None;
    }

    Some(StdDuration::from_secs(
        u64::try_from(remaining).unwrap_or(1),
    ))
}

fn log_runtime_event(
    layout: &StorageLayout,
    scope: &str,
    level: &str,
    account_id: Option<&str>,
    provider: Option<&str>,
    source_id: Option<&str>,
    source_handle: Option<&str>,
    message: impl Into<String>,
    detail: Option<String>,
) {
    let _ = runtime_log::append(
        layout,
        scope,
        level,
        account_id,
        provider,
        source_id,
        source_handle,
        message,
        detail,
    );
}

pub fn bootstrap_workspace() -> Result<WorkspaceSnapshot, String> {
    with_workspace(load_snapshot)
}

pub fn migrate_profile_pictures_to_settings() -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        let mut statement = connection
            .prepare(
                "SELECT id, profile_image_path FROM source_profiles WHERE profile_image_path IS NOT NULL AND profile_image_custom = 0",
            )
            .map_err(|e| e.to_string())?;

        let rows: Vec<(String, String)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        let now = chrono::Utc::now().to_rfc3339();

        for (source_id, image_path) in &rows {
            let path = Path::new(image_path);
            // Skip if already in Settings/
            if path
                .parent()
                .and_then(|p| p.file_name())
                .is_some_and(|name| name.eq_ignore_ascii_case(PROFILE_SETTINGS_DIR_NAME))
            {
                continue;
            }

            // Derive output_root from image path (parent of ProfilePicture.jpg)
            let output_root = match path.parent() {
                Some(root) => root,
                None => continue,
            };

            match sync_profile_picture_to_settings(output_root) {
                Ok(settings_path) => {
                    if let Ok(normalized) = normalize_media_file_path(&settings_path) {
                        let _ = connection.execute(
                            "UPDATE source_profiles SET profile_image_path = ?1, updated_at = ?2 WHERE id = ?3",
                            rusqlite::params![normalized, now, source_id],
                        );
                    }
                }
                Err(_) => continue,
            }
        }

        load_snapshot(connection, layout)
    })
}

pub fn load_all_asset_media_paths() -> Result<Vec<PathBuf>, String> {
    with_workspace(|connection, layout| {
        let mut paths: Vec<PathBuf> = Vec::new();
        paths.push(layout.media_root.clone());

        // mediaPath de cada conta (todos os providers usam `<provider>.account.mediaPath`).
        for account in load_accounts(connection)? {
            let settings = load_provider_account_settings_map(connection, &account.id)?;
            let key = format!("{}.account.mediaPath", account.provider.to_ascii_lowercase());
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

pub fn upsert_provider_account(input: ProviderAccountUpsert) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        upsert_provider_account_with_connection(connection, layout, input)
    })
}

pub fn delete_provider_account(id: String) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        delete_provider_account_with_connection(connection, layout, id)
    })
}

pub fn load_provider_account_cookies(
    account_id: String,
) -> Result<Vec<ProviderAccountCookie>, String> {
    with_workspace(|connection, layout| {
        load_provider_account_cookies_with_connection(connection, layout, &account_id)
    })
}

pub fn save_provider_account_cookies(
    account_id: String,
    cookies: Vec<ProviderAccountCookie>,
) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        save_provider_account_cookies_with_connection(connection, layout, &account_id, cookies)
    })
}

pub fn import_provider_account_cookies(
    input: ProviderAccountCookieImport,
) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        import_provider_account_cookies_with_connection(connection, layout, input)
    })
}

pub fn clear_provider_account_cookies(account_id: String) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        clear_provider_account_cookies_with_connection(connection, layout, &account_id)
    })
}

pub fn validate_provider_account(id: String) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        validate_provider_account_with_connection(connection, layout, id)
    })
}

pub fn preview_companion_account(
    capture: CompanionAccountCapture,
) -> Result<CompanionAccountPreview, String> {
    with_workspace(|connection, layout| {
        preview_companion_account_with_connection(connection, layout, &capture)
    })
}

pub fn import_companion_account(
    input: CompanionAccountImportInput,
) -> Result<CompanionAccountImportResult, String> {
    with_workspace(|connection, layout| {
        import_companion_account_with_connection(connection, layout, input)
    })
}

pub fn revert_provider_account_import(account_id: String) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        revert_provider_account_import_with_connection(connection, layout, &account_id)
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

pub fn list_import_providers() -> Result<Vec<ImportProviderDescriptor>, String> {
    Ok(vec![ImportProviderDescriptor {
        key: "instagram".to_string(),
        display_name: "Instagram".to_string(),
        description:
            "Import legacy SCrawler profile folders into NinjaCrawler sources and media catalog."
                .to_string(),
    }])
}

pub fn list_import_methods(provider: String) -> Result<Vec<ImportMethodDescriptor>, String> {
    if !provider.eq_ignore_ascii_case("instagram") {
        return Ok(Vec::new());
    }

    Ok(vec![ImportMethodDescriptor {
        importer_id: INSTAGRAM_SCRAWLER_IMPORTER_ID.to_string(),
        provider: "instagram".to_string(),
        label: "SCrawler".to_string(),
        description: "Scan SCrawler Instagram profile folders, preview matches, and import the media already on disk.".to_string(),
    }])
}

pub fn list_import_roots(
    importer_id: String,
    manual_roots: Vec<String>,
    disabled_roots: Vec<String>,
) -> Result<Vec<ImportRootDescriptor>, String> {
    with_workspace(|connection, layout| match importer_id.as_str() {
        INSTAGRAM_SCRAWLER_IMPORTER_ID => list_instagram_scrawler_import_roots_with_connection(
            connection,
            layout,
            &manual_roots,
            &disabled_roots,
        ),
        _ => Err(format!("Unsupported importer '{importer_id}'.")),
    })
}

pub fn preview_import_method(
    importer_id: String,
    options: ImportPreviewOptions,
) -> Result<ImportPreview, String> {
    with_workspace(|connection, layout| match importer_id.as_str() {
        INSTAGRAM_SCRAWLER_IMPORTER_ID => {
            preview_instagram_scrawler_import_with_connection(connection, layout, options)
        }
        _ => Err(format!("Unsupported importer '{importer_id}'.")),
    })
}

pub fn run_import_method(
    importer_id: String,
    input: ImportRunRequest,
) -> Result<ImportRunResult, String> {
    with_workspace(|connection, layout| match importer_id.as_str() {
        INSTAGRAM_SCRAWLER_IMPORTER_ID => {
            run_instagram_scrawler_import_with_connection(connection, layout, input)
        }
        _ => Err(format!("Unsupported importer '{importer_id}'.")),
    })
}

pub fn pick_import_root_folder() -> Result<Option<String>, String> {
    let initial_directory = storage::ensure_workspace_layout()
        .map(|layout| layout.media_root)
        .unwrap_or_else(|_| PathBuf::from("."));

    let picked = rfd::FileDialog::new()
        .set_title("Choose SCrawler import folder")
        .set_directory(initial_directory)
        .pick_folder();

    Ok(picked.map(|path| path.to_string_lossy().into_owned()))
}

pub fn load_provider_account_editor(account_id: String) -> Result<ProviderAccountEditor, String> {
    with_workspace(|connection, layout| {
        load_provider_account_editor_with_connection(connection, layout, account_id)
    })
}

pub fn save_provider_account_settings(
    account_id: String,
    values: Vec<ProviderAccountSettingValue>,
) -> Result<ProviderAccountEditor, String> {
    with_workspace(|connection, layout| {
        save_provider_account_settings_with_connection(connection, layout, account_id, values)
    })
}

pub fn clone_provider_account(account_id: String) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        clone_provider_account_with_connection(connection, layout, account_id)
    })
}

pub fn upsert_source_profile(input: SourceProfileUpsert) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        upsert_source_profile_with_connection(connection, layout, input)
    })
}

pub fn batch_update_source_profiles(
    patch: BatchSourceProfilePatch,
) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        connection
            .execute_batch("BEGIN IMMEDIATE TRANSACTION")
            .map_err(|error| format!("Failed to start batch update transaction: {error}"))?;

        let result = (|| {
            if let Some(Some(group_id)) = &patch.set_group_id {
                let group_exists = connection
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM scheduler_groups WHERE id = ?1)",
                        params![group_id],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(|error| {
                        format!("Failed to validate scheduler group {group_id}: {error}")
                    })?
                    != 0;

                if !group_exists {
                    return Err(format!("Scheduler group not found: {group_id}"));
                }
            }

            let mut loaded_sources = Vec::with_capacity(patch.source_ids.len());
            for source_id in &patch.source_ids {
                let row = connection
                    .query_row(
                        "SELECT labels_json, ready_for_download, sync_options_json, group_id FROM source_profiles WHERE id = ?1 AND deleted_at IS NULL",
                        params![source_id],
                        |row| {
                            let labels_json: String = row.get(0)?;
                            let ready_for_download: bool = row.get(1)?;
                            let sync_options_json: String = row.get(2)?;
                            let group_id: Option<String> = row.get(3)?;
                            Ok((labels_json, ready_for_download, sync_options_json, group_id))
                        },
                    )
                    .map_err(|error| format!("Failed to load source {source_id}: {error}"))?;
                loaded_sources.push((source_id, row));
            }

            let remove_set: std::collections::HashSet<String> = patch
                .labels_to_remove
                .iter()
                .map(|label| label.trim().to_lowercase())
                .collect();
            let now = chrono::Utc::now().to_rfc3339();

            for (source_id, (labels_json, current_ready, sync_options_json, current_group_id)) in
                loaded_sources
            {
                let mut labels: Vec<String> =
                    serde_json::from_str(&labels_json).unwrap_or_default();
                for label in &patch.labels_to_add {
                    let normalized = label.trim().to_lowercase();
                    if !labels
                        .iter()
                        .any(|existing| existing.trim().to_lowercase() == normalized)
                    {
                        labels.push(label.trim().to_string());
                    }
                }
                if !remove_set.is_empty() {
                    labels.retain(|label| !remove_set.contains(&label.trim().to_lowercase()));
                }

                let ready_for_download = patch.ready_for_download.unwrap_or(current_ready);
                let group_id = match &patch.set_group_id {
                    Some(new_group_id) => new_group_id.clone(),
                    None => current_group_id,
                };

                let mut sync_options: SourceSyncOptions =
                    serde_json::from_str(&sync_options_json).unwrap_or_default();
                if let Some(ref ig_patch) = patch.sync_options_patch {
                    let ig = sync_options.instagram.get_or_insert_with(Default::default);
                    apply_instagram_patch(ig, ig_patch);
                }

                let new_labels_json =
                    serde_json::to_string(&labels).unwrap_or_else(|_| "[]".to_string());
                let new_sync_json =
                    serde_json::to_string(&sync_options).unwrap_or_else(|_| "{}".to_string());

                connection
                    .execute(
                        "UPDATE source_profiles SET labels_json = ?1, ready_for_download = ?2, sync_options_json = ?3, updated_at = ?4, group_id = ?6 WHERE id = ?5",
                        params![new_labels_json, ready_for_download, new_sync_json, now, source_id, group_id],
                    )
                    .map_err(|error| format!("Failed to update source {source_id}: {error}"))?;
            }

            connection
                .execute_batch("COMMIT")
                .map_err(|error| format!("Failed to commit batch update transaction: {error}"))?;

            load_snapshot(connection, layout)
        })();

        if result.is_err() {
            let _ = connection.execute_batch("ROLLBACK");
        }

        result
    })
}

fn apply_instagram_patch(
    options: &mut InstagramSourceSyncOptions,
    patch: &InstagramSyncOptionsPatch,
) {
    if let Some(v) = patch.timeline {
        options.timeline = v;
    }
    if let Some(v) = patch.reels {
        options.reels = v;
    }
    if let Some(v) = patch.stories {
        options.stories = v;
    }
    if let Some(v) = patch.stories_user {
        options.stories_user = v;
    }
    if let Some(v) = patch.tagged {
        options.tagged = v;
    }
    if let Some(v) = patch.temporary {
        options.temporary = Some(v);
    }
    if let Some(v) = patch.favorite {
        options.favorite = Some(v);
    }
    if let Some(v) = patch.download_images {
        options.download_images = Some(v);
    }
    if let Some(v) = patch.download_videos {
        options.download_videos = Some(v);
    }
    if let Some(v) = patch.place_extracted_image_into_video_folder {
        options.place_extracted_image_into_video_folder = Some(v);
    }
    if let Some(ref extract_patch) = patch.extract_image_from_video {
        let extract = options
            .extract_image_from_video
            .get_or_insert_with(Default::default);
        if let Some(v) = extract_patch.timeline {
            extract.timeline = v;
        }
        if let Some(v) = extract_patch.reels {
            extract.reels = v;
        }
        if let Some(v) = extract_patch.stories {
            extract.stories = v;
        }
        if let Some(v) = extract_patch.stories_user {
            extract.stories_user = v;
        }
        if let Some(v) = extract_patch.tagged {
            extract.tagged = v;
        }
    }
    if let Some(v) = patch.get_user_media_only {
        options.get_user_media_only = Some(v);
    }
    if let Some(v) = patch.missing_only {
        options.missing_only = Some(v);
    }
    if let Some(v) = patch.verified_profile {
        options.verified_profile = Some(v);
    }
    if let Some(v) = patch.force_update_user_name {
        options.force_update_user_name = Some(v);
    }
    if let Some(v) = patch.force_update_user_information {
        options.force_update_user_information = Some(v);
    }
    if let Some(v) = patch.download_text {
        options.download_text = Some(v);
    }
    if let Some(v) = patch.download_text_posts {
        options.download_text_posts = Some(v);
    }
}

pub fn delete_source_profile(
    id: String,
    mode: SourceProfileDeleteMode,
) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        delete_source_profile_with_connection(connection, layout, id, mode)
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDeleteProgressUpdate {
    pub progress_percent: Option<u32>,
    pub progress_label: Option<String>,
    pub progress_detail: Option<String>,
    pub progress_indeterminate: bool,
    pub files_processed: Option<u32>,
    pub files_total: Option<u32>,
}

pub fn delete_source_profile_with_progress<F>(
    id: String,
    mode: SourceProfileDeleteMode,
    mut on_progress: F,
) -> Result<WorkspaceSnapshot, String>
where
    F: FnMut(SourceDeleteProgressUpdate) -> Result<(), String>,
{
    with_workspace(|connection, layout| {
        delete_source_profile_with_connection_and_progress(
            connection,
            layout,
            id,
            mode,
            &mut on_progress,
        )
    })
}

fn delete_source_profile_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    id: String,
    mode: SourceProfileDeleteMode,
) -> Result<WorkspaceSnapshot, String> {
    let mut on_progress = |_| Ok(());
    delete_source_profile_with_connection_and_progress(
        connection,
        layout,
        id,
        mode,
        &mut on_progress,
    )
}

fn delete_source_profile_with_connection_and_progress<F>(
    connection: &Connection,
    layout: &StorageLayout,
    id: String,
    mode: SourceProfileDeleteMode,
    on_progress: &mut F,
) -> Result<WorkspaceSnapshot, String>
where
    F: FnMut(SourceDeleteProgressUpdate) -> Result<(), String>,
{
    match mode {
        SourceProfileDeleteMode::UserOnly => {
            on_progress(SourceDeleteProgressUpdate {
                progress_percent: Some(5),
                progress_label: Some("Preparing delete".to_string()),
                progress_detail: Some("Loading source and validating delete request.".to_string()),
                progress_indeterminate: true,
                files_processed: None,
                files_total: None,
            })?;

            let timestamp = now_timestamp();
            let updated = connection
                .execute(
                    "UPDATE source_profiles
                     SET deleted_at = ?2,
                         account_id = NULL,
                         ready_for_download = 0,
                         updated_at = ?2
                     WHERE id = ?1
                       AND deleted_at IS NULL",
                    params![&id, &timestamp],
                )
                .map_err(|error| error.to_string())?;

            if updated == 0 {
                return Err(format!("Source '{}' does not exist.", id));
            }

            on_progress(SourceDeleteProgressUpdate {
                progress_percent: Some(70),
                progress_label: Some("Soft deleting profile".to_string()),
                progress_detail: Some(
                    "Keeping existing media files and folders on disk.".to_string(),
                ),
                progress_indeterminate: false,
                files_processed: None,
                files_total: None,
            })?;

            on_progress(SourceDeleteProgressUpdate {
                progress_percent: Some(95),
                progress_label: Some("Finalizing snapshot".to_string()),
                progress_detail: Some("Refreshing workspace after profile delete.".to_string()),
                progress_indeterminate: false,
                files_processed: None,
                files_total: None,
            })?;

            load_snapshot(connection, layout)
        }
        SourceProfileDeleteMode::WithMedia => {
            on_progress(SourceDeleteProgressUpdate {
                progress_percent: Some(5),
                progress_label: Some("Loading source".to_string()),
                progress_detail: Some("Collecting source metadata before delete.".to_string()),
                progress_indeterminate: true,
                files_processed: None,
                files_total: None,
            })?;

            let source = connection
                .query_row(
                    "SELECT id, provider, source_kind, handle, display_name, account_id, labels_json, ready_for_download, sync_options_json, profile_image_path, profile_image_custom, remote_state, is_subscription, last_synced_at, sync_problem_code, sync_problem_message, sync_problem_at, created_at, group_id, importer_id, imported_at
                     FROM source_profiles
                     WHERE id = ?1
                       AND deleted_at IS NULL
                     LIMIT 1",
                    params![&id],
                    |row| {
                        let provider: String = row.get(1)?;
                        Ok(SourceProfile {
                            id: row.get(0)?,
                            provider: provider.clone(),
                            source_kind: row.get(2)?,
                            handle: row.get(3)?,
                            display_name: row.get(4)?,
                            account_id: row.get(5)?,
                            group_id: row.get(18)?,
                            labels: from_json_array(row.get::<_, String>(6)?),
                            ready_for_download: row.get::<_, i64>(7)? != 0,
                            sync_options: deserialize_source_sync_options(
                                &provider,
                                &row.get::<_, String>(8)?,
                            ),
                            profile_image_path: row.get(9)?,
                            profile_image_custom: row.get::<_, i64>(10).unwrap_or(0) != 0,
                            remote_state: row.get::<_, String>(11).unwrap_or_else(|_| "exists".to_string()),
                            is_subscription: row.get::<_, i64>(12).unwrap_or(0) != 0,
                            last_synced_at: row.get(13).ok(),
                            sync_problem_code: row.get(14).ok(),
                            sync_problem_message: row.get(15).ok(),
                            sync_problem_at: row.get(16).ok(),
                            created_at: row.get(17).ok(),
                            importer_id: row.get(19).ok(),
                            imported_at: row.get(20).ok(),
                        })
                    },
                )
                .optional()
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("Source '{}' does not exist.", id))?;

            on_progress(SourceDeleteProgressUpdate {
                progress_percent: Some(12),
                progress_label: Some("Loading profile settings".to_string()),
                progress_detail: Some("Resolving source folders before delete.".to_string()),
                progress_indeterminate: true,
                files_processed: None,
                files_total: None,
            })?;
            let account_settings = source
                .account_id
                .as_deref()
                .filter(|_| source.provider.eq_ignore_ascii_case("instagram"))
                .map(|account_id| load_provider_account_settings_map(connection, account_id))
                .transpose()?
                .unwrap_or_default();

            on_progress(SourceDeleteProgressUpdate {
                progress_percent: Some(45),
                progress_label: Some("Removing source folders".to_string()),
                progress_detail: Some("Deleting profile media directories on disk.".to_string()),
                progress_indeterminate: false,
                files_processed: None,
                files_total: None,
            })?;
            remove_source_media_directories(layout, &source, &account_settings)?;

            on_progress(SourceDeleteProgressUpdate {
                progress_percent: Some(72),
                progress_label: Some("Removing custom profile image".to_string()),
                progress_detail: Some(
                    "Deleting any custom image bound to this profile.".to_string(),
                ),
                progress_indeterminate: false,
                files_processed: None,
                files_total: None,
            })?;
            remove_source_custom_profile_images(layout, &source.id)?;

            on_progress(SourceDeleteProgressUpdate {
                progress_percent: Some(88),
                progress_label: Some("Deleting profile".to_string()),
                progress_detail: Some("Removing the source profile record.".to_string()),
                progress_indeterminate: false,
                files_processed: None,
                files_total: None,
            })?;
            connection
                .execute(
                    "DELETE FROM source_profiles WHERE id = ?1",
                    params![&source.id],
                )
                .map_err(|error| error.to_string())?;

            on_progress(SourceDeleteProgressUpdate {
                progress_percent: Some(97),
                progress_label: Some("Finalizing snapshot".to_string()),
                progress_detail: Some("Refreshing workspace after profile delete.".to_string()),
                progress_indeterminate: false,
                files_processed: None,
                files_total: None,
            })?;
            load_snapshot(connection, layout)
        }
    }
}

fn remove_source_media_directories(
    layout: &StorageLayout,
    source: &SourceProfile,
    account_settings: &HashMap<String, String>,
) -> Result<(), String> {
    let normalized_handle = sanitize_source_handle(&source.provider, &source.handle);
    let at_handle = format!("@{}", normalized_handle.trim_start_matches('@'));
    let mut directories = HashSet::new();

    directories.insert(resolved_source_media_output_root(
        layout,
        source,
        Some(account_settings),
    ));
    directories.insert(source_media_output_root(layout, source));
    directories.insert(
        layout
            .media_root
            .join(sanitize_path_segment(&source.provider))
            .join(sanitize_path_segment(&normalized_handle)),
    );
    directories.insert(
        layout
            .media_root
            .join(sanitize_path_segment(&source.provider))
            .join(sanitize_path_segment(&at_handle)),
    );

    if source.provider.eq_ignore_ascii_case("instagram") {
        let instagram_base = instagram_media_base_root(layout, Some(account_settings));
        directories.insert(instagram_base.join(sanitize_path_segment(&normalized_handle)));
        directories.insert(instagram_base.join(sanitize_path_segment(&at_handle)));
    }

    for directory in directories {
        if directory.exists() {
            fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
        }
    }

    Ok(())
}

const GALLERY_VIDEO_EXTS: [&str; 5] = ["mp4", "webm", "mkv", "mov", "m4v"];
const GALLERY_IMAGE_EXTS: [&str; 6] = ["jpg", "jpeg", "png", "webp", "heic", "gif"];

/// TikTok codifica o unix de criação nos bits altos do id (`id >> 32`).
fn gallery_timestamp_from_tiktok_id(post_id: &str) -> Option<i64> {
    let id = post_id.trim().parse::<u64>().ok()?;
    let seconds = (id >> 32) as i64;
    (1_400_000_000..4_000_000_000).contains(&seconds).then_some(seconds)
}

/// Converte o prefixo de data dos nomes (`YYYY-MM-DD HH.MM.SS`, hora local) em
/// unix. Retorna (unix, resto_do_nome_sem_prefixo).
fn strip_gallery_date_prefix(stem: &str) -> (Option<i64>, String) {
    // Formato fixo: 19 chars + espaço.
    if stem.len() > 20 {
        let (prefix, rest) = stem.split_at(19);
        if rest.starts_with(' ') {
            if let Ok(naive) = NaiveDateTime::parse_from_str(prefix, "%Y-%m-%d %H.%M.%S") {
                let unix = Local
                    .from_local_datetime(&naive)
                    .single()
                    .map(|dt| dt.timestamp());
                return (unix, rest[1..].to_string());
            }
        }
    }
    (None, stem.to_string())
}

struct DerivedPost {
    post_id: Option<String>,
    captured_at: Option<i64>,
    media_type: &'static str,
    /// Chave de agrupamento (post_id quando houver, senão o nome).
    group_key: String,
    /// Índice da imagem no slideshow (`_index_<i>_<n>`), 0-based.
    index: Option<usize>,
}

/// Arquivo de avatar/foto de perfil que NÃO deve aparecer na galeria.
fn is_profile_image_file(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    lower.contains("_avatar.") || lower.starts_with("profilepicture")
}

/// Monta o link original do post (nível do post — depende do tipo final).
fn build_post_url(
    provider: &str,
    handle: &str,
    post_id: Option<&str>,
    is_video: bool,
    post_code: Option<&str>,
) -> Option<String> {
    let handle = handle.trim().trim_start_matches('@');
    match provider {
        // TikTok separa vídeo (`/video/`) de foto-slideshow (`/photo/`).
        "tiktok" => post_id.map(|post_id| {
            format!(
                "https://www.tiktok.com/@{handle}/{}/{post_id}",
                if is_video { "video" } else { "photo" }
            )
        }),
        "twitter" => post_id.map(|post_id| format!("https://x.com/{handle}/status/{post_id}")),
        // Instagram usa o shortcode (case-sensitive) reconstruído pelo ledger;
        // sem ele o link cai para o perfil.
        "instagram" => post_code
            .map(str::trim)
            .filter(|code| !code.is_empty())
            .map(|code| format!("https://www.instagram.com/p/{code}/")),
        _ => None,
    }
}

/// Deriva o id/data do post a partir do NOME do arquivo (cobre imports 4K Tokkit
/// e os naming do connector — ambos guardam o post id no nome).
fn derive_post_metadata(
    provider: &str,
    file_name: &str,
    mtime_unix: Option<i64>,
) -> Option<DerivedPost> {
    let (stem, ext) = match file_name.rsplit_once('.') {
        Some((s, e)) => (s, e.to_ascii_lowercase()),
        None => return None,
    };
    let media_type = if GALLERY_VIDEO_EXTS.contains(&ext.as_str()) {
        "video"
    } else if GALLERY_IMAGE_EXTS.contains(&ext.as_str()) {
        "image"
    } else {
        return None;
    };

    let (date_prefix_unix, rest) = strip_gallery_date_prefix(stem);

    // Tokens separados por '_'. O id do post é o token numérico mais LONGO
    // (>=10 dígitos): ids do TikTok/Twitter têm 18-19, unix tem 10, autonumber 3,
    // e handles com dígitos curtos (027_araujo) não confundem.
    let tokens: Vec<&str> = rest.split('_').collect();
    let mut post_id: Option<String> = None;
    let mut best_len = 0usize;
    for token in &tokens {
        if token.len() >= 10 && token.chars().all(|c| c.is_ascii_digit()) && token.len() >= best_len {
            best_len = token.len();
            post_id = Some((*token).to_string());
        }
    }

    // Slideshow: `..._<postid>_index_<i>_<n>`.
    let mut index: Option<usize> = None;
    if let Some(pos) = tokens.iter().position(|t| *t == "index") {
        if let Some(i) = tokens.get(pos + 1).and_then(|t| t.parse::<usize>().ok()) {
            index = Some(i);
        }
        // o id costuma estar imediatamente antes de "index"
        if let Some(candidate) = tokens.get(pos.wrapping_sub(1)) {
            if candidate.len() >= 10 && candidate.chars().all(|c| c.is_ascii_digit()) {
                post_id = Some((*candidate).to_string());
            }
        }
    }

    // unix token (tokkit `<handle>_<unix>_<postid>`): 9-11 dígitos e != post_id.
    let tokkit_unix = tokens.iter().find_map(|t| {
        if (9..=11).contains(&t.len())
            && t.chars().all(|c| c.is_ascii_digit())
            && Some(t.to_string()) != post_id
        {
            t.parse::<i64>().ok().filter(|v| (1_400_000_000..4_000_000_000).contains(v))
        } else {
            None
        }
    });

    // Data: TikTok a deriva do próprio id; os demais usam o token unix / prefixo
    // de data / mtime.
    let captured_at = if provider == "tiktok" {
        post_id
            .as_deref()
            .and_then(gallery_timestamp_from_tiktok_id)
            .or(tokkit_unix)
            .or(date_prefix_unix)
            .or(mtime_unix)
    } else {
        date_prefix_unix.or(tokkit_unix).or(mtime_unix)
    };

    let group_key = post_id.clone().unwrap_or_else(|| stem.to_string());
    Some(DerivedPost {
        post_id,
        captured_at,
        media_type,
        group_key,
        index,
    })
}

struct GalleryPostAcc {
    post_id: Option<String>,
    captured_at: Option<i64>,
    media_type: String,
    section: String,
    /// Highlight album (subpasta sob `Stories/`), quando o post for um highlight.
    album: Option<String>,
    /// Álbuns de highlight resolvidos por associação (mídia que mora no Feed mas
    /// pertence a um destaque), casados pela media key dos arquivos.
    membership_albums: BTreeSet<String>,
    files: Vec<(Option<usize>, MediaGalleryFile)>,
    /// Authoritative metadata joined from the sync ledger by relative path
    /// (preferred over the values derived from the file name).
    ledger_post_key: Option<String>,
    ledger_post_code: Option<String>,
    ledger_section: Option<String>,
    ledger_captured_at: Option<i64>,
}

/// Post link metadata joined from the per-provider media ledger, keyed by the
/// (lowercased) relative path of each downloaded file.
#[derive(Default, Clone)]
struct GalleryMediaLedgerLink {
    post_key: Option<String>,
    post_code: Option<String>,
    section: Option<String>,
    captured_at: Option<i64>,
}

/// Loads the relative-path → link map used to rebuild post URLs and resolve the
/// feed/reels section. Instagram keeps its own ledger (with the case-sensitive
/// shortcode); TikTok/Twitter share the provider-neutral ledger (with the post
/// key and capture time). Returns an empty map for legacy media without ledger
/// rows — the gallery then falls back to file-name derivation.
fn load_gallery_media_ledger_links(
    connection: &Connection,
    provider: &str,
    source_id: &str,
    profile_root: &Path,
) -> HashMap<String, GalleryMediaLedgerLink> {
    let mut links = HashMap::new();
    if provider.eq_ignore_ascii_case("instagram") {
        if let Ok(mut statement) = connection.prepare(
            "SELECT relative_path, media_section, provider_post_code
             FROM instagram_sync_media_ledger WHERE source_id = ?1",
        ) {
            let rows = statement.query_map(params![source_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            });
            if let Ok(rows) = rows {
                for row in rows.flatten() {
                    let (relative_path, section, post_code) = row;
                    links.insert(
                        relative_path.to_ascii_lowercase(),
                        GalleryMediaLedgerLink {
                            post_key: None,
                            post_code,
                            section,
                            captured_at: None,
                        },
                    );
                }
            }
        }
        // Fallback para imports legados (SCrawler) baixados ANTES do shortcode ser
        // persistido no ledger: lê o código (casing original) direto do XML.
        for (relative_path, (post_code, section)) in load_legacy_instagram_post_codes(profile_root) {
            let entry = links.entry(relative_path).or_default();
            if entry.post_code.is_none() {
                entry.post_code = post_code;
            }
            if entry.section.is_none() {
                entry.section = section;
            }
        }
        return links;
    }

    let Ok(mut statement) = connection.prepare(
        "SELECT relative_path, media_section, provider_post_key, captured_at
         FROM provider_sync_media_ledger WHERE provider = ?1 AND source_id = ?2",
    ) else {
        return links;
    };
    let rows = statement.query_map(params![provider, source_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<i64>>(3)?,
        ))
    });
    if let Ok(rows) = rows {
        for row in rows.flatten() {
            let (relative_path, section, post_key, captured_at) = row;
            links.insert(
                relative_path.to_ascii_lowercase(),
                GalleryMediaLedgerLink {
                    post_key,
                    post_code: None,
                    section,
                    captured_at,
                },
            );
        }
    }
    links
}

/// Lista a mídia baixada de um perfil agrupada por post, com o link original
/// reconstruído. O front agrupa por dia (via `captured_at`).
pub fn load_source_media_gallery(source_id: String) -> Result<SourceMediaGallery, String> {
    with_workspace(|connection, layout| {
        let row = connection
            .query_row(
                "SELECT provider, handle, account_id, sync_options_json FROM source_profiles
                 WHERE id = ?1 AND deleted_at IS NULL LIMIT 1",
                params![&source_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Source '{}' does not exist.", source_id))?;
        let (provider, handle, account_id, sync_options_json) = row;

        // SourceProfile mínimo, mas COM sync_options (TikTok lê specialPath dele).
        let source_profile = SourceProfile {
            id: source_id.clone(),
            provider: provider.clone(),
            source_kind: "profile".to_string(),
            handle: handle.clone(),
            display_name: String::new(),
            account_id,
            group_id: None,
            labels: Vec::new(),
            ready_for_download: false,
            sync_options: deserialize_source_sync_options(&provider, &sync_options_json),
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

        // Post link/section metadata from the sync ledger, keyed by relative path
        // (lowercased to match the gallery's own key). For Instagram this also
        // merges shortcodes read from the legacy SCrawler XML.
        let ledger_links =
            load_gallery_media_ledger_links(connection, &provider, &source_id, &profile_root);
        // Twitter has no status id in the file name and older media ledger rows
        // predate the post-key column, so pair files with their tweet id via the
        // legacy SCrawler XML (keyed by media key). Empty for other providers.
        let twitter_post_keys = if provider.eq_ignore_ascii_case("twitter") {
            load_legacy_twitter_post_keys(&profile_root)
        } else {
            HashMap::new()
        };
        // Associações de álbum de highlight (Instagram), media key → álbuns.
        // Vazio para outros providers / quando não há associação.
        let highlight_membership = if provider.eq_ignore_ascii_case("instagram") {
            load_instagram_highlight_membership(connection, &source_id)
        } else {
            HashMap::new()
        };

        let mut grouped: HashMap<String, GalleryPostAcc> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        // Posters por post id (cover do 4K Tokkit na subpasta `cover/`).
        let mut posters: HashMap<String, String> = HashMap::new();
        for path in collect_media_file_paths(&profile_root)? {
            let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            // Avatares e a foto de perfil não são posts.
            if is_profile_image_file(file_name) {
                continue;
            }
            let relative_path = path
                .strip_prefix(&profile_root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let top_segment = if relative_path.contains('/') {
                relative_path
                    .split('/')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase()
            } else {
                String::new()
            };
            // `Settings/` etc. são ignorados; `cover/` vira poster do post.
            if top_segment == "settings" {
                continue;
            }
            if top_segment == "cover" {
                if let Some(post_id) = derive_post_metadata(&provider, file_name, None)
                    .and_then(|derived| derived.post_id)
                {
                    posters
                        .entry(post_id)
                        .or_insert_with(|| path.to_string_lossy().to_string());
                }
                continue;
            }

            let mtime_unix = fs::metadata(&path)
                .ok()
                .and_then(|meta| meta.modified().ok())
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|dur| dur.as_secs() as i64);
            let Some(derived) = derive_post_metadata(&provider, file_name, mtime_unix) else {
                continue;
            };
            // Seção: subpasta conhecida (Stories/Reposts/Video) ou "timeline".
            let section = match top_segment.as_str() {
                "stories" | "reposts" | "video" => top_segment.clone(),
                _ => "timeline".to_string(),
            };
            // Highlights ficam em `Stories/<álbum>/arquivo` — o 2º segmento é o
            // título do álbum (preserva casing/emoji do nome da pasta original).
            let album = if top_segment == "stories" {
                relative_path
                    .split('/')
                    .nth(1)
                    .map(str::trim)
                    .filter(|segment| !segment.is_empty())
                    .map(str::to_string)
            } else {
                None
            };
            // Liga ao ledger pelo relative_path (que no banco é lowercased).
            let link = ledger_links.get(&relative_path.to_ascii_lowercase()).cloned();
            // Instagram: agrupa o carrossel pelo shortcode — todas as fotos do
            // post compartilham o mesmo code, então viram UM card (slideshow).
            // Sem code (mídia antiga sem link), cai para o id derivado do nome.
            let group_key = if provider.eq_ignore_ascii_case("instagram") {
                link.as_ref()
                    .and_then(|entry| entry.post_code.as_deref())
                    .map(str::trim)
                    .filter(|code| !code.is_empty())
                    .map(|code| format!("ig-code:{}", code.to_ascii_lowercase()))
                    .unwrap_or_else(|| derived.group_key.clone())
            } else {
                derived.group_key.clone()
            };
            let file = MediaGalleryFile {
                relative_path,
                absolute_path: path.to_string_lossy().to_string(),
                media_type: derived.media_type.to_string(),
            };
            let entry = grouped.entry(group_key.clone()).or_insert_with(|| {
                order.push(group_key.clone());
                GalleryPostAcc {
                    post_id: derived.post_id.clone(),
                    captured_at: derived.captured_at,
                    media_type: derived.media_type.to_string(),
                    section,
                    album,
                    membership_albums: BTreeSet::new(),
                    files: Vec::new(),
                    ledger_post_key: None,
                    ledger_post_code: None,
                    ledger_section: None,
                    ledger_captured_at: None,
                }
            });
            entry.files.push((derived.index, file));
            // Resolve a participação em álbuns de highlight pela media key deste
            // arquivo (mesmo método de `existing_media_keys`), cobrindo a mídia
            // que mora no Feed mas pertence a um destaque.
            if !highlight_membership.is_empty() {
                for candidate in extract_instagram_media_identity_candidates_from_path(&path) {
                    if let Some(member_albums) = highlight_membership.get(&candidate) {
                        entry
                            .membership_albums
                            .extend(member_albums.iter().cloned());
                    }
                }
            }
            if entry.captured_at.is_none() {
                entry.captured_at = derived.captured_at;
            }
            if entry.media_type == "image" && derived.media_type == "video" {
                entry.media_type = "video".to_string();
            }
            // Primeiro arquivo do post que tiver dado de ledger define o link/seção.
            if let Some(link) = link {
                if entry.ledger_post_key.is_none() {
                    if let Some(post_key) = link.post_key.filter(|value| !value.trim().is_empty()) {
                        entry.ledger_post_key = Some(post_key);
                    }
                }
                if entry.ledger_post_code.is_none() {
                    if let Some(post_code) = link.post_code.filter(|value| !value.trim().is_empty()) {
                        entry.ledger_post_code = Some(post_code);
                    }
                }
                if entry.ledger_section.is_none() {
                    if let Some(section) = link.section.filter(|value| !value.trim().is_empty()) {
                        entry.ledger_section = Some(section);
                    }
                }
                if entry.ledger_captured_at.is_none() {
                    entry.ledger_captured_at = link.captured_at;
                }
            }
            // Twitter: o status id não está no nome nem (para mídia antiga) no
            // ledger; recupera do XML do SCrawler casando pelo media key.
            if entry.ledger_post_key.is_none() && !twitter_post_keys.is_empty() {
                if let Some(status_id) =
                    twitter_media_key_from_file_name(file_name).and_then(|key| twitter_post_keys.get(&key))
                {
                    entry.ledger_post_key = Some(status_id.clone());
                }
            }
        }

        let mut posts: Vec<MediaGalleryPost> = Vec::with_capacity(order.len());
        for key in order {
            if let Some(mut acc) = grouped.remove(&key) {
                acc.files.sort_by_key(|(index, _)| index.unwrap_or(0));
                let files: Vec<MediaGalleryFile> =
                    acc.files.into_iter().map(|(_, file)| file).collect();
                let is_video = acc.media_type == "video";
                let media_type = if !is_video && files.len() > 1 {
                    "slideshow".to_string()
                } else {
                    acc.media_type
                };
                // O id do ledger (autoridade do connector) tem prioridade sobre o
                // derivado do nome; o shortcode do IG só vem do ledger. No Twitter
                // o nome do arquivo NUNCA carrega o status id (só o media key/
                // autonumber), então usar o id derivado do nome geraria um link
                // ERRADO — ali só o status id real (ledger/XML) vale.
                let post_id_for_url = if provider.eq_ignore_ascii_case("twitter") {
                    acc.ledger_post_key.as_deref()
                } else {
                    acc.ledger_post_key.as_deref().or(acc.post_id.as_deref())
                };
                let post_url = build_post_url(
                    &provider,
                    &handle,
                    post_id_for_url,
                    is_video,
                    acc.ledger_post_code.as_deref(),
                );
                // Seção e data preferem o ledger; caem para o derivado do nome.
                let section = acc
                    .ledger_section
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or(acc.section);
                let captured_at = acc.ledger_captured_at.or(acc.captured_at);
                // Poster: o cover (vídeo) ou a 1ª imagem (slideshow).
                let poster_path = acc
                    .post_id
                    .as_deref()
                    .and_then(|id| posters.get(id).cloned())
                    .or_else(|| {
                        if !is_video {
                            files.first().map(|file| file.absolute_path.clone())
                        } else {
                            None
                        }
                    });
                // Álbuns: o da subpasta física (`Stories/<álbum>/`) unido aos das
                // associações de highlight (mídia que mora no Feed mas pertence a
                // um destaque), já resolvidas por media key no loop acima.
                let mut albums: Vec<String> = Vec::new();
                let mut seen_albums = BTreeSet::new();
                if let Some(path_album) = acc.album {
                    if seen_albums.insert(path_album.clone()) {
                        albums.push(path_album);
                    }
                }
                for album in acc.membership_albums {
                    if seen_albums.insert(album.clone()) {
                        albums.push(album);
                    }
                }
                posts.push(MediaGalleryPost {
                    post_id: acc.post_id,
                    post_url,
                    captured_at,
                    media_type,
                    section,
                    albums,
                    poster_path,
                    files,
                });
            }
        }
        // Mais recentes primeiro (sem data vão ao fim).
        posts.sort_by(|a, b| b.captured_at.unwrap_or(0).cmp(&a.captured_at.unwrap_or(0)));

        Ok(SourceMediaGallery {
            source_id,
            provider: provider.clone(),
            handle: handle.clone(),
            profile_url: source_target_url(&provider, &handle),
            posts,
        })
    })
}

fn ensure_provider_deleted_media_table(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS provider_deleted_media (
                provider TEXT NOT NULL,
                source_id TEXT NOT NULL,
                relative_path TEXT NOT NULL,
                media_section TEXT NOT NULL DEFAULT '',
                provider_post_key TEXT,
                provider_post_code TEXT,
                provider_media_key TEXT,
                deleted_at TEXT NOT NULL,
                PRIMARY KEY (provider, source_id, relative_path),
                FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_provider_deleted_media_source
                ON provider_deleted_media(provider, source_id);",
        )
        .map_err(|error| error.to_string())
}

/// Reads the substring of `url` right after `marker` up to the next path/query
/// separator. Used to recover a post key/shortcode from the gallery's post URL.
fn url_segment_after(url: &str, marker: &str) -> Option<String> {
    let tail = url.split_once(marker)?.1;
    tail.split(['/', '?', '&', '#'])
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Resolves the (post_key, post_code) used to tombstone a deleted post in the
/// per-provider post ledger, from what the gallery already resolved. TikTok uses
/// the numeric post id; Twitter the status id; Instagram the case-sensitive
/// shortcode (the connector's skip set accepts the code as a key).
fn extract_post_tombstone_keys(
    provider: &str,
    post: &MediaGalleryPost,
) -> (Option<String>, Option<String>) {
    match provider {
        "tiktok" => (
            post.post_id
                .clone()
                .or_else(|| post.post_url.as_deref().and_then(|url| {
                    url_segment_after(url, "/video/").or_else(|| url_segment_after(url, "/photo/"))
                })),
            None,
        ),
        "twitter" => (
            post.post_url
                .as_deref()
                .and_then(|url| url_segment_after(url, "/status/")),
            None,
        ),
        "instagram" => (
            None,
            post.post_url
                .as_deref()
                .and_then(|url| url_segment_after(url, "/p/")),
        ),
        _ => (None, None),
    }
}

/// Moves the given media files (paths relative to the source's profile root) to
/// the OS recycle bin and records a deletion tombstone, so they are neither
/// shown again nor re-downloaded on the next sync. The post key/code is written
/// back into the per-provider post ledger — which every connector already
/// consults to skip known posts — so no connector changes are needed. Returns
/// the refreshed gallery.
pub fn delete_source_media(
    source_id: String,
    relative_paths: Vec<String>,
) -> Result<SourceMediaGallery, String> {
    // Resolve each requested file to its post first, reusing the gallery's own
    // link/section resolution (ledger + legacy XML + file-name derivation).
    let gallery = load_source_media_gallery(source_id.clone())?;
    let mut post_by_rel: HashMap<String, MediaGalleryPost> = HashMap::new();
    for post in &gallery.posts {
        for file in &post.files {
            post_by_rel.insert(file.relative_path.to_ascii_lowercase(), post.clone());
        }
    }

    with_workspace(|connection, layout| {
        let row = connection
            .query_row(
                "SELECT provider, handle, account_id, sync_options_json FROM source_profiles
                 WHERE id = ?1 AND deleted_at IS NULL LIMIT 1",
                params![&source_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Source '{}' does not exist.", source_id))?;
        let (provider, handle, account_id, sync_options_json) = row;
        let account_id_for_ledger = account_id.clone();
        let source_profile = SourceProfile {
            id: source_id.clone(),
            provider: provider.clone(),
            source_kind: "profile".to_string(),
            handle: handle.clone(),
            display_name: String::new(),
            account_id,
            group_id: None,
            labels: Vec::new(),
            ready_for_download: false,
            sync_options: deserialize_source_sync_options(&provider, &sync_options_json),
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
        let canonical_root = fs::canonicalize(&profile_root).unwrap_or_else(|_| profile_root.clone());

        ensure_provider_deleted_media_table(connection)?;
        let now = now_timestamp();

        let mut tw_posts: Vec<twitter_connector::ObservedTwitterPost> = Vec::new();
        let mut ig_posts: Vec<instagram_connector::ObservedInstagramPost> = Vec::new();
        let mut seen_post: HashSet<String> = HashSet::new();

        for raw_rel in &relative_paths {
            let rel = raw_rel.replace('\\', "/");
            let rel = rel.trim_start_matches('/').to_string();
            if rel.is_empty() {
                continue;
            }
            let abs = profile_root.join(&rel);
            // Containment guard: never touch anything outside the profile root.
            let abs_canon = fs::canonicalize(&abs).unwrap_or_else(|_| abs.clone());
            if !abs_canon.starts_with(&canonical_root) {
                continue;
            }

            let post = post_by_rel.get(&rel.to_ascii_lowercase());
            let section = post
                .map(|entry| entry.section.clone())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "timeline".to_string());
            let (post_key, post_code) = post
                .map(|entry| extract_post_tombstone_keys(&provider, entry))
                .unwrap_or((None, None));

            if abs.exists() {
                trash::delete(&abs)
                    .map_err(|error| format!("Failed to delete '{}': {error}", abs.display()))?;
            }

            connection
                .execute(
                    "INSERT INTO provider_deleted_media (
                        provider, source_id, relative_path, media_section,
                        provider_post_key, provider_post_code, deleted_at
                     )
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(provider, source_id, relative_path) DO UPDATE SET
                        media_section = excluded.media_section,
                        provider_post_key = COALESCE(excluded.provider_post_key, provider_deleted_media.provider_post_key),
                        provider_post_code = COALESCE(excluded.provider_post_code, provider_deleted_media.provider_post_code),
                        deleted_at = excluded.deleted_at",
                    params![
                        provider,
                        source_id,
                        rel.to_ascii_lowercase(),
                        section,
                        post_key,
                        post_code,
                        now,
                    ],
                )
                .map_err(|error| error.to_string())?;

            // Tombstone the post key/code so the next sync skips re-downloading it.
            if provider.eq_ignore_ascii_case("instagram") {
                if let Some(code) = post_code.clone() {
                    if seen_post.insert(code.to_ascii_lowercase()) {
                        ig_posts.push(instagram_connector::ObservedInstagramPost {
                            provider_post_key: post_key.clone().unwrap_or_else(|| code.clone()),
                            provider_post_code: Some(code),
                            media_section: section.clone(),
                        });
                    }
                }
            } else if let Some(key) = post_key.clone() {
                if seen_post.insert(key.to_ascii_lowercase()) {
                    tw_posts.push(twitter_connector::ObservedTwitterPost {
                        provider_post_key: key,
                        media_section: section.clone(),
                    });
                }
            }
        }

        // The post ledger has a FK to provider_accounts, so only write tombstones
        // there when the source is account-linked (the deletion is always
        // recorded in provider_deleted_media regardless).
        if let Some(account) = account_id_for_ledger
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if provider.eq_ignore_ascii_case("instagram") {
                if !ig_posts.is_empty() {
                    upsert_instagram_post_ledger_entries(
                        connection, &source_id, account, &handle, &ig_posts, &now,
                    )?;
                }
            } else if !tw_posts.is_empty() {
                upsert_provider_sync_post_ledger_entries(
                    connection,
                    &provider.to_ascii_lowercase(),
                    &source_id,
                    account,
                    &handle,
                    &tw_posts,
                    &now,
                )?;
            }
        }

        Ok(())
    })?;

    load_source_media_gallery(source_id)
}

pub fn pick_source_profile_image(source_id: String) -> Result<WorkspaceSnapshot, String> {
    let initial_directory = with_workspace(|connection, layout| {
        let source = connection
            .query_row(
                "SELECT provider, handle, account_id
                 FROM source_profiles
                 WHERE id = ?1
                   AND deleted_at IS NULL
                 LIMIT 1",
                params![&source_id],
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
            .ok_or_else(|| format!("Source '{}' does not exist.", source_id))?;

        let source_profile = SourceProfile {
            id: source_id.clone(),
            provider: source.0,
            source_kind: String::new(),
            handle: source.1,
            display_name: String::new(),
            account_id: source.2,
            group_id: None,
            labels: Vec::new(),
            ready_for_download: false,
            sync_options: Default::default(),
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
        let source_media_dir =
            resolved_source_media_output_root_with_connection(connection, layout, &source_profile)?;
        fs::create_dir_all(&source_media_dir).map_err(|error| error.to_string())?;
        Ok(source_media_dir)
    })?;

    let dialog = rfd::FileDialog::new()
        .set_title("Choose profile image")
        .set_directory(initial_directory)
        .add_filter("Images", &["jpg", "jpeg", "png", "webp", "gif", "bmp"]);
    let picked = dialog.pick_file();

    let file_path = match picked {
        Some(path) => path,
        None => return with_workspace(load_snapshot),
    };

    with_workspace(|connection, layout| {
        let extension = file_path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("jpg");
        let dest_dir = layout.data_dir.join("profile-images");
        fs::create_dir_all(&dest_dir).map_err(|error| error.to_string())?;
        let dest_path = dest_dir.join(format!("{}.{}", source_id, extension));
        fs::copy(&file_path, &dest_path).map_err(|error| error.to_string())?;
        let normalized = normalize_media_file_path(&dest_path)?;
        let timestamp = now_timestamp();
        connection
            .execute(
                "UPDATE source_profiles
                 SET profile_image_path = ?2, profile_image_custom = 1, updated_at = ?3
                 WHERE id = ?1",
                params![source_id, normalized, timestamp],
            )
            .map_err(|error| error.to_string())?;
        load_snapshot(connection, layout)
    })
}

pub fn reset_source_profile_image(source_id: String) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        let source = connection
            .query_row(
                "SELECT provider, handle, account_id
                 FROM source_profiles
                 WHERE id = ?1
                   AND deleted_at IS NULL
                 LIMIT 1",
                params![&source_id],
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
            .ok_or_else(|| format!("Source '{}' does not exist.", source_id))?;

        let source_profile = SourceProfile {
            id: source_id.clone(),
            provider: source.0,
            source_kind: String::new(),
            handle: source.1,
            display_name: String::new(),
            account_id: source.2,
            group_id: None,
            labels: Vec::new(),
            ready_for_download: false,
            sync_options: Default::default(),
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
        let output_root =
            resolved_source_media_output_root_with_connection(connection, layout, &source_profile)?;
        let auto_avatar = find_source_avatar(&output_root);

        let timestamp = now_timestamp();
        connection
            .execute(
                "UPDATE source_profiles
                 SET profile_image_path = ?2, profile_image_custom = 0, updated_at = ?3
                 WHERE id = ?1",
                params![&source_id, auto_avatar, timestamp],
            )
            .map_err(|error| error.to_string())?;

        remove_source_custom_profile_images(layout, &source_id)?;

        load_snapshot(connection, layout)
    })
}

fn remove_source_custom_profile_images(
    layout: &StorageLayout,
    source_id: &str,
) -> Result<(), String> {
    let custom_dir = layout.data_dir.join("profile-images");
    if !custom_dir.exists() {
        return Ok(());
    }

    let source_prefix = format!("{source_id}.");
    for entry in fs::read_dir(&custom_dir).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name == source_id || name.starts_with(&source_prefix) {
            let _ = fs::remove_file(entry.path());
        }
    }

    Ok(())
}

pub fn run_source_sync(
    source_id: String,
    trigger: Option<String>,
    run_mode: Option<String>,
    sync_options_override: Option<SourceSyncOptions>,
) -> Result<WorkspaceSnapshot, String> {
    let trigger_value = trigger
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("manual")
        .to_string();

    with_workspace(|connection, layout| {
        run_source_sync_with_connection(
            connection,
            layout,
            source_id,
            &trigger_value,
            run_mode.as_deref(),
            sync_options_override.as_ref(),
            &CommandToolExecutor,
        )
    })
}

pub fn check_source_availability(
    source_ids: Vec<String>,
    account_id_override: Option<String>,
) -> Result<SourceAvailabilityCheckResult, String> {
    with_workspace(|connection, layout| {
        let unique_source_ids: Vec<String> = source_ids
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let mut items = Vec::<SourceAvailabilityCheckItem>::new();
        let mut unchanged = 0u32;
        let mut updated_handle = 0u32;
        let mut marked_problem = 0u32;
        let mut skipped = 0u32;
        let mut failed = 0u32;
        let now = now_timestamp();
        let normalized_account_id_override = account_id_override
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let account_override_error = normalized_account_id_override
            .as_ref()
            .map(|account_id| -> Result<Option<String>, String> {
                let provider = connection
                    .query_row(
                        "SELECT provider FROM provider_accounts WHERE id = ?1 LIMIT 1",
                        params![account_id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(|error| error.to_string())?;

                let Some(provider) = provider else {
                    return Ok(Some(format!(
                        "Selected availability account '{account_id}' was not found."
                    )));
                };

                if !provider.eq_ignore_ascii_case("instagram") {
                    return Ok(Some(format!(
                        "Selected availability account '{account_id}' is not an Instagram account."
                    )));
                }

                Ok(None)
            })
            .transpose()?
            .flatten();

        let mut session_cache: HashMap<String, (ParsedSessionPayload, HashMap<String, String>)> =
            HashMap::new();

        for (source_index, source_id) in unique_source_ids.iter().enumerate() {
            let source_row = connection
                .query_row(
                    "SELECT id, provider, handle, sync_options_json, account_id
                     FROM source_profiles
                     WHERE id = ?1
                       AND deleted_at IS NULL
                     LIMIT 1",
                    params![source_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, Option<String>>(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(|error| error.to_string())?;

            let Some((id, provider, handle, sync_options_json, account_id)) = source_row else {
                failed += 1;
                items.push(SourceAvailabilityCheckItem {
                    source_id: source_id.clone(),
                    provider: "unknown".to_string(),
                    previous_handle: String::new(),
                    current_handle: None,
                    status: "failed".to_string(),
                    message: "Profile was not found in the workspace.".to_string(),
                });
                continue;
            };

            if !provider.eq_ignore_ascii_case("instagram") {
                skipped += 1;
                items.push(SourceAvailabilityCheckItem {
                    source_id: id,
                    provider,
                    previous_handle: handle,
                    current_handle: None,
                    status: "skipped".to_string(),
                    message:
                        "Availability check is currently supported only for Instagram profiles."
                            .to_string(),
                });
                continue;
            }

            let previous_handle = sanitize_source_handle("instagram", &handle);
            if previous_handle.is_empty() {
                failed += 1;
                items.push(SourceAvailabilityCheckItem {
                    source_id: id,
                    provider,
                    previous_handle: handle,
                    current_handle: None,
                    status: "failed".to_string(),
                    message: "Profile handle is empty or invalid.".to_string(),
                });
                continue;
            }

            if let Some(message) = account_override_error.as_ref() {
                failed += 1;
                items.push(SourceAvailabilityCheckItem {
                    source_id: id,
                    provider,
                    previous_handle,
                    current_handle: None,
                    status: "failed".to_string(),
                    message: message.clone(),
                });
                continue;
            }

            let sync_options = deserialize_source_sync_options("instagram", &sync_options_json);
            let source_user_id_hint = sync_options
                .instagram
                .as_ref()
                .and_then(|instagram| instagram.user_id_hint.clone())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            let history_user_id_hint =
                load_latest_instagram_profile_user_id_hint(connection, &id)
                    .ok()
                    .flatten();
            let user_id_hint = preferred_instagram_user_id_hint(
                source_user_id_hint.as_deref(),
                history_user_id_hint.as_deref(),
            );

            let selected_account_id = normalized_account_id_override
                .as_deref()
                .or(account_id.as_deref());
            let auth_context_error = if let Some(acct_id) = selected_account_id {
                if !session_cache.contains_key(acct_id) {
                    match (|| -> Result<(ParsedSessionPayload, HashMap<String, String>), String> {
                        let session = load_account_session_record(connection, acct_id)?
                            .ok_or_else(|| "No session record".to_string())?;
                        let secret =
                            session_secret_store::load_secret(layout, &session.secret_ref)?;
                        let parsed = parse_session_payload(&secret)?;
                        let settings = load_provider_account_settings_map(connection, acct_id)?;
                        Ok((parsed, settings))
                    })() {
                        Ok(ctx) => {
                            session_cache.insert(acct_id.to_string(), ctx);
                            None
                        }
                        Err(error) => {
                            if normalized_account_id_override.is_some() {
                                Some(format!(
                                    "Selected availability account '{acct_id}' is not ready for authenticated checks: {error}"
                                ))
                            } else {
                                None
                            }
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(message) = auth_context_error {
                failed += 1;
                items.push(SourceAvailabilityCheckItem {
                    source_id: id,
                    provider,
                    previous_handle,
                    current_handle: None,
                    status: "failed".to_string(),
                    message,
                });
                continue;
            }

            let auth_context = if let Some(acct_id) = selected_account_id {
                if !session_cache.contains_key(acct_id) {
                    let loaded =
                        (|| -> Result<(ParsedSessionPayload, HashMap<String, String>), String> {
                            let session = load_account_session_record(connection, acct_id)?
                                .ok_or_else(|| "No session record".to_string())?;
                            let secret =
                                session_secret_store::load_secret(layout, &session.secret_ref)?;
                            let parsed = parse_session_payload(&secret)?;
                            let settings = load_provider_account_settings_map(connection, acct_id)?;
                            Ok((parsed, settings))
                        })();
                    if let Ok(ctx) = loaded {
                        session_cache.insert(acct_id.to_string(), ctx);
                    }
                }
                session_cache.get(acct_id)
            } else {
                None
            };

            let request = match auth_context {
                Some((parsed_session, settings)) => {
                    build_instagram_authenticated_identity_probe_request(
                        &previous_handle,
                        &parsed_session.cookies,
                        settings,
                        Some(&parsed_session.metadata),
                    )
                }
                None => build_instagram_identity_probe_request(&previous_handle),
            };

            let primary = instagram_connector::resolve_profile_identity(&request, None);
            if let Err(error) = &primary {
                if instagram_error_indicates_availability_abort_rate_limit(error) {
                    failed += 1;
                    items.push(SourceAvailabilityCheckItem {
                        source_id: id.clone(),
                        provider: provider.clone(),
                        previous_handle: previous_handle.clone(),
                        current_handle: None,
                        status: "failed".to_string(),
                        message: format!(
                            "Availability check aborted due to Instagram rate limiting (429): {error}"
                        ),
                    });
                    for remaining_source_id in unique_source_ids.iter().skip(source_index + 1) {
                        skipped += 1;
                        items.push(build_availability_rate_limit_skipped_item(
                            connection,
                            remaining_source_id,
                        ));
                    }
                    break;
                }
            }

            let primary_classification = primary
                .as_ref()
                .err()
                .map(|error| classify_instagram_identity_error(error));
            let normalized_user_id_hint = user_id_hint
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let primary_identity_mismatch = match (&primary, normalized_user_id_hint) {
                (Ok(identity), Some(expected_user_id)) => {
                    identity.user_id.trim() != expected_user_id
                }
                _ => false,
            };
            let fallback = match (
                primary_classification,
                normalized_user_id_hint,
                primary_identity_mismatch,
            ) {
                (_, Some(hint), true) => Some(instagram_connector::resolve_profile_identity(
                    &request,
                    Some(hint),
                )),
                (
                    Some(InstagramIdentityErrorClassification::PrivateOrRestricted),
                    Some(hint),
                    false,
                ) => {
                    Some(instagram_connector::resolve_profile_identity(
                        &request,
                        Some(hint),
                    ))
                }
                (
                    Some(InstagramIdentityErrorClassification::UsernameUnresolvable),
                    Some(hint),
                    false,
                ) => {
                    Some(instagram_connector::resolve_profile_identity(
                        &request,
                        Some(hint),
                    ))
                }
                _ => None,
            };

            if let Some(Err(error)) = fallback.as_ref() {
                if instagram_error_indicates_availability_abort_rate_limit(error) {
                    let rate_limit_error = error.clone();
                    let action = decide_instagram_availability_action(
                        &previous_handle,
                        &primary,
                        fallback.as_ref(),
                    );
                    apply_instagram_availability_action(
                        connection,
                        &id,
                        &provider,
                        &previous_handle,
                        &now,
                        action,
                        &mut unchanged,
                        &mut updated_handle,
                        &mut marked_problem,
                        &mut failed,
                        &mut items,
                    )?;
                    if let Some(last) = items.last_mut() {
                        last.message = format!(
                            "{} Also aborted batch due to Instagram rate limiting (429) during hint fallback: {}",
                            last.message, rate_limit_error
                        );
                    }

                    for remaining_source_id in unique_source_ids.iter().skip(source_index + 1) {
                        skipped += 1;
                        items.push(build_availability_rate_limit_skipped_item(
                            connection,
                            remaining_source_id,
                        ));
                    }
                    break;
                }
            }

            let action =
                decide_instagram_availability_action(&previous_handle, &primary, fallback.as_ref());
            apply_instagram_availability_action(
                connection,
                &id,
                &provider,
                &previous_handle,
                &now,
                action,
                &mut unchanged,
                &mut updated_handle,
                &mut marked_problem,
                &mut failed,
                &mut items,
            )?;
        }

        let snapshot = load_snapshot(connection, layout)?;
        Ok(SourceAvailabilityCheckResult {
            snapshot,
            requested: unique_source_ids.len() as u32,
            processed: items.len() as u32,
            unchanged,
            updated_handle,
            marked_problem,
            skipped,
            failed,
            items,
        })
    })
}
pub fn run_instagram_saved_posts_sync(account_id: String) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        execute_instagram_saved_posts_sync_with_connection(
            connection, layout, account_id, "manual",
        )?;
        load_snapshot(connection, layout)
    })
}

pub fn queue_source_sync(
    source_id: String,
    sync_options_override: Option<SourceSyncOptions>,
) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        let context = load_source_sync_context(connection, layout, &source_id)?;
        validate_source_sync_override(&context.source, sync_options_override.as_ref())?;
        load_snapshot(connection, layout)
    })
}

#[derive(Clone)]
pub struct SourceSyncQueueItemSeed {
    pub source_id: String,
    pub provider: String,
    pub handle: String,
    pub account_id: Option<String>,
}

pub fn source_sync_queue_item_seed(
    source_id: String,
    sync_options_override: Option<SourceSyncOptions>,
) -> Result<SourceSyncQueueItemSeed, String> {
    with_workspace(|connection, layout| {
        let context = load_source_sync_context(connection, layout, &source_id)?;
        validate_source_sync_override(&context.source, sync_options_override.as_ref())?;
        Ok(SourceSyncQueueItemSeed {
            source_id: context.source.id,
            provider: context.source.provider,
            handle: context.source.handle,
            account_id: Some(context.account.id),
        })
    })
}

/// Job pendente da fila manual de sync, persistido para sobreviver ao
/// fechamento do app.
#[derive(Clone)]
pub struct PersistedSourceSyncQueueJob {
    pub source_id: String,
    pub trigger: String,
    pub run_mode: Option<String>,
    pub sync_options_override: Option<SourceSyncOptions>,
    pub queued_at: String,
}

pub fn persist_source_sync_queue_job(job: &PersistedSourceSyncQueueJob) -> Result<(), String> {
    with_workspace(|connection, _| {
        let override_json = job
            .sync_options_override
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        // Novos jobs entram ao final (maior order_index). O ON CONFLICT NÃO
        // altera order_index para preservar a ordem manual de um job já na fila
        // (ex.: promoção para force-backfill).
        connection
            .execute(
                "INSERT INTO source_sync_queue_jobs
                   (source_id, trigger, run_mode, sync_options_override_json, queued_at, order_index)
                 VALUES (?1, ?2, ?3, ?4, ?5,
                         (SELECT COALESCE(MAX(order_index), 0) + 1 FROM source_sync_queue_jobs))
                 ON CONFLICT(source_id) DO UPDATE SET
                   trigger = excluded.trigger,
                   run_mode = excluded.run_mode,
                   sync_options_override_json = excluded.sync_options_override_json,
                   queued_at = excluded.queued_at",
                params![
                    job.source_id,
                    job.trigger,
                    job.run_mode,
                    override_json,
                    job.queued_at
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

/// Persiste a ordem manual (drag-and-drop) atribuindo `order_index` crescente
/// conforme a posição em `ordered_source_ids`. Só afeta os jobs informados; a
/// ordem relativa por provider é o que importa no restore.
pub fn persist_source_sync_queue_order(ordered_source_ids: &[String]) -> Result<(), String> {
    if ordered_source_ids.is_empty() {
        return Ok(());
    }
    with_workspace(|connection, _| {
        for (index, source_id) in ordered_source_ids.iter().enumerate() {
            connection
                .execute(
                    "UPDATE source_sync_queue_jobs SET order_index = ?2 WHERE source_id = ?1",
                    params![source_id, index as i64],
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    })
}

pub fn remove_source_sync_queue_job(source_id: &str) -> Result<(), String> {
    with_workspace(|connection, _| {
        connection
            .execute(
                "DELETE FROM source_sync_queue_jobs WHERE source_id = ?1",
                params![source_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

pub fn load_persisted_source_sync_queue_jobs() -> Result<Vec<PersistedSourceSyncQueueJob>, String> {
    with_workspace(|connection, _| {
        let mut statement = connection
            .prepare(
                "SELECT source_id, trigger, run_mode, sync_options_override_json, queued_at
                 FROM source_sync_queue_jobs
                 ORDER BY order_index ASC, queued_at ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|error| error.to_string())?;

        let mut jobs = Vec::new();
        for row in rows {
            let (source_id, trigger, run_mode, override_json, queued_at) =
                row.map_err(|error| error.to_string())?;
            let sync_options_override = override_json
                .as_deref()
                .and_then(|json| serde_json::from_str::<SourceSyncOptions>(json).ok());
            jobs.push(PersistedSourceSyncQueueJob {
                source_id,
                trigger,
                run_mode,
                sync_options_override,
                queued_at,
            });
        }
        Ok(jobs)
    })
}

#[derive(Clone)]
pub struct SourceDeleteQueueItemSeed {
    pub source_id: String,
    pub provider: String,
    pub handle: String,
}

pub fn source_delete_queue_item_seed(
    source_id: String,
) -> Result<SourceDeleteQueueItemSeed, String> {
    with_workspace(|connection, _layout| {
        let source = connection
            .query_row(
                "SELECT id, provider, handle
                 FROM source_profiles
                 WHERE id = ?1
                   AND deleted_at IS NULL
                 LIMIT 1",
                params![&source_id],
                |row| {
                    Ok(SourceDeleteQueueItemSeed {
                        source_id: row.get(0)?,
                        provider: row.get(1)?,
                        handle: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;

        source.ok_or_else(|| format!("Source '{}' does not exist.", source_id))
    })
}

pub fn upsert_scheduler_set(input: SchedulerSetUpsert) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        upsert_scheduler_set_with_connection(connection, input)?;
        load_snapshot(connection, layout)
    })
}

pub fn delete_scheduler_set(id: String) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        connection
            .execute("DELETE FROM scheduler_sets WHERE id = ?1", params![id])
            .map_err(|error| error.to_string())?;
        load_snapshot(connection, layout)
    })
}

pub fn upsert_scheduler_group(input: SchedulerGroupUpsert) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        upsert_scheduler_group_with_connection(connection, input)?;
        load_snapshot(connection, layout)
    })
}

pub fn delete_scheduler_group(id: String) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        connection
            .execute("DELETE FROM scheduler_groups WHERE id = ?1", params![id])
            .map_err(|error| error.to_string())?;
        load_snapshot(connection, layout)
    })
}

pub fn upsert_sync_plan(input: SyncPlanUpsert) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        upsert_sync_plan_with_connection(connection, input)?;
        load_snapshot(connection, layout)
    })
}

pub fn preview_sync_plan_target(
    input: SyncPlanTargetPreviewInput,
) -> Result<SyncPlanTargetPreview, String> {
    with_workspace(|connection, _layout| {
        preview_sync_plan_target_with_connection(connection, input)
    })
}

pub fn delete_sync_plan(id: String) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        connection
            .execute("DELETE FROM sync_plans WHERE id = ?1", params![id])
            .map_err(|error| error.to_string())?;
        load_snapshot(connection, layout)
    })
}

/// Pedido de enfileiramento gerado por um plano: a fonte a sincronizar e o
/// trigger a registrar. A camada de runtime (que tem o AppHandle) enfileira.
pub struct PlanSyncEnqueueRequest {
    pub source_id: String,
    pub trigger: String,
}

pub fn run_sync_plan_now(
    input: RunSyncPlanNowInput,
) -> Result<(WorkspaceSnapshot, Vec<PlanSyncEnqueueRequest>), String> {
    let trigger = if input.force.unwrap_or(false) {
        "manual_force"
    } else {
        "manual"
    };
    with_workspace(|connection, layout| {
        let source_ids =
            run_sync_plan_now_with_connection(connection, layout, &input.id, trigger, &now_timestamp())?;
        let snapshot = load_snapshot(connection, layout)?;
        let requests = source_ids
            .into_iter()
            .map(|source_id| PlanSyncEnqueueRequest {
                source_id,
                trigger: trigger.to_string(),
            })
            .collect();
        Ok((snapshot, requests))
    })
}

pub fn pause_sync_plan(id: String) -> Result<WorkspaceSnapshot, String> {
    set_sync_plan_pause(SetSyncPlanPauseInput {
        id,
        pause_mode: "unlimited".to_string(),
        pause_until: None,
    })
}

pub fn resume_sync_plan(id: String) -> Result<WorkspaceSnapshot, String> {
    clear_sync_plan_pause(id)
}

pub fn skip_sync_plan(id: String) -> Result<WorkspaceSnapshot, String> {
    skip_sync_plan_with_input(SkipSyncPlanInput {
        id,
        mode: "default".to_string(),
        minutes: None,
        until: None,
    })
}

pub fn set_sync_plan_pause(input: SetSyncPlanPauseInput) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        set_sync_plan_pause_with_connection(connection, &input, &now_timestamp())?;
        load_snapshot(connection, layout)
    })
}

pub fn clear_sync_plan_pause(id: String) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        clear_sync_plan_pause_with_connection(connection, &id, &now_timestamp())?;
        load_snapshot(connection, layout)
    })
}

pub fn skip_sync_plan_with_input(input: SkipSyncPlanInput) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        skip_sync_plan_with_connection(connection, &input, &now_timestamp())?;
        load_snapshot(connection, layout)
    })
}

pub fn move_sync_plan(input: MoveSyncPlanInput) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        move_sync_plan_with_connection(connection, &input)?;
        load_snapshot(connection, layout)
    })
}

pub fn clone_sync_plan(input: CloneSyncPlanInput) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        clone_sync_plan_with_connection(connection, &input)?;
        load_snapshot(connection, layout)
    })
}

pub fn process_scheduler_tick(
) -> Result<(WorkspaceSnapshot, Vec<PlanSyncEnqueueRequest>), String> {
    with_workspace(|connection, layout| {
        let requests =
            process_scheduler_tick_with_connection(connection, layout, &now_timestamp())?;
        let snapshot = load_snapshot(connection, layout)?;
        Ok((snapshot, requests))
    })
}

pub fn record_scheduler_launch() -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        record_scheduler_launch_with_connection(connection, &now_timestamp())?;
        load_snapshot(connection, layout)
    })
}

pub fn open_source_folder(source_id: String) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        open_source_folder_with_connection(connection, layout, &source_id)?;
        load_snapshot(connection, layout)
    })
}

fn open_source_folder_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    source_id: &str,
) -> Result<(), String> {
    let (provider, handle, account_id, sync_options_json) = connection
        .query_row(
            "SELECT provider, handle, account_id, sync_options_json
             FROM source_profiles
             WHERE id = ?1
               AND deleted_at IS NULL
             LIMIT 1",
            params![source_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Source '{}' does not exist.", source_id))?;

    let account_settings = account_id
        .as_deref()
        .map(|id| load_provider_account_settings_map(connection, id))
        .transpose()?;
    let source_profile = SourceProfile {
        id: source_id.to_string(),
        provider: provider.clone(),
        source_kind: "profile".to_string(),
        handle,
        display_name: String::new(),
        account_id,
        group_id: None,
        labels: Vec::new(),
        ready_for_download: false,
        sync_options: deserialize_source_sync_options(&provider, &sync_options_json),
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
    let root =
        resolved_source_media_output_root(layout, &source_profile, account_settings.as_ref());
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let root_display = root.display().to_string();
    run_windows_command("explorer", &[&root_display])
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

/// Lê o throttle (segundos) entre downloads consecutivos da fila. Tolerante a
/// falhas: qualquer erro/ausência retorna 0 (sem espera).
/// Delay (em segundos) a aplicar depois de baixar um perfil desta conta antes
/// do próximo job da fila. Como cada cookie/conta tem seu próprio rate limit,
/// a configuração é por conta (`<provider>.account.delayBetweenDownloadsSecs`);
/// quando a conta não define (ou define 0), recai no padrão global
/// (`policy.sync.delayBetweenProfilesSecs`).
pub fn sync_delay_for_account(account_id: Option<&str>, provider: &str) -> u64 {
    with_workspace(|connection, _| {
        if let Some(account_id) = account_id {
            let key = format!(
                "{}.account.delayBetweenDownloadsSecs",
                provider.to_ascii_lowercase()
            );
            let account_settings = load_provider_account_settings_map(connection, account_id)?;
            if let Some(secs) = account_settings
                .get(&key)
                .and_then(|value| value.trim().parse::<u64>().ok())
            {
                if secs > 0 {
                    return Ok(secs);
                }
            }
        }

        Ok(load_app_setting_value(connection, SYNC_DELAY_BETWEEN_PROFILES_SETTING_KEY)?
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(0))
    })
    .unwrap_or(0)
    .min(3600)
}

pub fn upsert_app_setting(input: AppSettingUpsert) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        let now = now_timestamp();
        connection
            .execute(
                "INSERT INTO app_settings (key, value, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET
                   value = excluded.value,
                   updated_at = excluded.updated_at",
                params![input.key, input.value, now],
            )
            .map_err(|error| error.to_string())?;
        load_snapshot(connection, layout)
    })
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
    captured_at: Option<i64>,
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
    command.env("PYTHONUTF8", "1").env("PYTHONIOENCODING", "utf-8");
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
        return Err(format!("Downloaded file was not found on disk: {file_path}"));
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
        captured_at,
    })
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

        let result = run_yt_dlp_video_download(connection, layout, &url, provider, &root)?;
        let absolute = result.absolute_path;
        let provider_video_id = result.provider_video_id;
        let uploader = result.uploader;
        let title = result.title;
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
                    "video",
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

        Ok(SingleVideo {
            id: canonical_id,
            provider: provider.to_string(),
            source_url: url,
            provider_video_id,
            uploader,
            title,
            absolute_path: absolute.to_string_lossy().to_string(),
            relative_path,
            media_type: "video".to_string(),
            captured_at,
            downloaded_at: now,
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
                let relative_path: String = row.get(6)?;
                Ok(SingleVideo {
                    id: row.get(0)?,
                    provider: row.get(1)?,
                    source_url: row.get(2)?,
                    provider_video_id: row.get(3)?,
                    uploader: row.get(4)?,
                    title: row.get(5)?,
                    absolute_path: root.join(&relative_path).to_string_lossy().to_string(),
                    relative_path,
                    media_type: row.get(7)?,
                    captured_at: row.get(8)?,
                    downloaded_at: row.get(9)?,
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
        if let Some(relative) = connection
            .query_row(
                "SELECT relative_path FROM single_videos WHERE id = ?1",
                params![id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
        {
            let absolute = root.join(&relative);
            if absolute.exists() {
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

pub fn run_instagram_media_naming_ledger_backfill<F>(
    mut on_progress: F,
) -> Result<InstagramNamingLedgerBackfillResult, String>
where
    F: FnMut(InstagramNamingLedgerBackfillProgress),
{
    with_workspace(|connection, layout| {
        run_instagram_media_naming_ledger_backfill_with_connection(
            connection,
            layout,
            &mut on_progress,
        )
    })
}

fn run_instagram_media_naming_ledger_backfill_with_connection<F>(
    connection: &Connection,
    layout: &StorageLayout,
    on_progress: &mut F,
) -> Result<InstagramNamingLedgerBackfillResult, String>
where
    F: FnMut(InstagramNamingLedgerBackfillProgress),
{
    ensure_instagram_media_naming_ledger_table(connection)?;

    let global_settings = load_app_settings_map(connection)?;
    let now = now_timestamp();
    let sources = load_sources(connection)?
        .into_iter()
        .filter(|entry| entry.provider.eq_ignore_ascii_case("instagram"))
        .filter(|entry| entry.account_id.as_deref().is_some())
        .collect::<Vec<_>>();
    let naming_mode = parse_instagram_media_file_naming_mode(&global_settings);
    let naming_template = parse_instagram_media_file_naming_template(&global_settings);

    let mut result = InstagramNamingLedgerBackfillResult {
        scanned_sources: sources.len() as u32,
        backfilled_at: now.clone(),
        ..InstagramNamingLedgerBackfillResult::default()
    };

    let mut progress = InstagramNamingLedgerBackfillProgress {
        total_sources: result.scanned_sources,
        ..InstagramNamingLedgerBackfillProgress::default()
    };
    on_progress(progress.clone());

    for source in sources.into_iter() {
        let Some(account_id) = source.account_id.as_deref() else {
            continue;
        };
        result.scanned_profiles += 1;
        progress.processed_sources = result.scanned_profiles;
        progress.source_id = Some(source.id.clone());
        progress.source_handle = Some(source.handle.clone());
        on_progress(progress.clone());

        let account_settings = load_provider_account_settings_map(connection, account_id)?;
        let source_options = source_instagram_sync_options(&source);
        let profile_root = resolve_instagram_profile_root_with_options(
            layout,
            &source,
            Some(&account_settings),
            Some(&source_options),
        );
        if !profile_root.exists() {
            continue;
        }

        let legacy_records =
            collect_legacy_instagram_reconciliation_records(&profile_root).unwrap_or_default();
        let mut legacy_by_key = HashMap::<String, LegacyInstagramReconciliationRecord>::new();
        for record in legacy_records {
            legacy_by_key
                .entry(record.provider_media_key.clone())
                .or_insert(record);
        }
        result.legacy_records_total += legacy_by_key.len() as u32;
        progress.legacy_records_total = result.legacy_records_total;

        let mut matched_legacy_keys = HashSet::new();

        for path in collect_media_file_paths(&profile_root)? {
            result.scanned_files += 1;
            progress.scanned_files = result.scanned_files;
            if is_profile_picture_file(&path) {
                result.skipped_files += 1;
                progress.skipped_files = result.skipped_files;
                continue;
            }

            let Some(media_type) = infer_media_type(&path) else {
                result.skipped_files += 1;
                progress.skipped_files = result.skipped_files;
                continue;
            };
            let Some(provider_media_key) = derive_instagram_media_identity_key_from_path(&path)
            else {
                result.skipped_files += 1;
                progress.skipped_files = result.skipped_files;
                continue;
            };
            let final_file_name = path
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or_default()
                .to_string();
            if final_file_name.is_empty() {
                result.skipped_files += 1;
                progress.skipped_files = result.skipped_files;
                continue;
            }
            let relative_path = normalize_instagram_relative_media_path(&profile_root, &path);
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_ascii_lowercase())
                .unwrap_or_else(|| media_type.to_string());
            let legacy_match = legacy_by_key.get(&provider_media_key);
            let existed = connection
                .query_row(
                    "SELECT 1
                     FROM instagram_media_naming_ledger
                     WHERE source_id = ?1
                       AND provider_media_key = ?2
                       AND media_type = ?3
                     LIMIT 1",
                    params![&source.id, &provider_media_key, media_type],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(|error| error.to_string())?
                .is_some();

            connection
                .execute(
                    "INSERT INTO instagram_media_naming_ledger (
                        source_id,
                        account_id,
                        source_handle,
                        provider_media_key,
                        media_type,
                        media_section,
                        captured_at,
                        extension,
                        final_file_name,
                        legacy_raw_file_name,
                        relative_path,
                        pattern_mode,
                        pattern_template,
                        first_seen_at,
                        last_seen_at
                     )
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)
                     ON CONFLICT(source_id, provider_media_key, media_type)
                     DO UPDATE SET
                        account_id = excluded.account_id,
                        source_handle = excluded.source_handle,
                        media_section = excluded.media_section,
                        extension = excluded.extension,
                        final_file_name = excluded.final_file_name,
                        legacy_raw_file_name = excluded.legacy_raw_file_name,
                        relative_path = excluded.relative_path,
                        pattern_mode = excluded.pattern_mode,
                        pattern_template = excluded.pattern_template,
                        last_seen_at = excluded.last_seen_at",
                    params![
                        &source.id,
                        account_id,
                        &source.handle,
                        provider_media_key,
                        media_type,
                        legacy_match
                            .map(|record| record.media_section.as_str())
                            .unwrap_or("timeline"),
                        extension,
                        final_file_name,
                        legacy_match.map(|record| record.legacy_file_name.as_str()),
                        relative_path,
                        naming_mode.as_str(),
                        naming_template.as_deref(),
                        &now,
                    ],
                )
                .map_err(|error| error.to_string())?;

            if existed {
                result.updated_entries += 1;
                progress.updated_entries = result.updated_entries;
            } else {
                result.inserted_entries += 1;
                progress.inserted_entries = result.inserted_entries;
            }
            if legacy_match.is_some() {
                matched_legacy_keys.insert(provider_media_key);
            }

            if result.scanned_files % 200 == 0 {
                on_progress(progress.clone());
            }
        }

        result.legacy_records_matched += matched_legacy_keys.len() as u32;
        result.legacy_records_missing_files += legacy_by_key
            .len()
            .saturating_sub(matched_legacy_keys.len())
            as u32;
        progress.legacy_records_matched = result.legacy_records_matched;
        on_progress(progress.clone());
    }

    upsert_app_setting_value(
        connection,
        INSTAGRAM_NAMING_LEDGER_BACKFILL_SETTING_KEY,
        "true",
    )?;

    on_progress(progress);
    Ok(result)
}

fn load_app_setting_value(connection: &Connection, key: &str) -> Result<Option<String>, String> {
    connection
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1 LIMIT 1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn migrate_media_root_setting_to_scrawler_pattern(connection: &Connection) -> Result<(), String> {
    let scrawler_media_root = PathBuf::from(r"F:\SCrawler\Data");
    if !scrawler_media_root.exists() {
        return Ok(());
    }

    let Some(current_value) = load_app_setting_value(connection, "storage.media_root")? else {
        return Ok(());
    };
    let current_trimmed = current_value.trim();
    if current_trimmed.is_empty() {
        return Ok(());
    }

    let Some(user_profile) = std::env::var_os("USERPROFILE").map(PathBuf::from) else {
        return Ok(());
    };
    let legacy_default = user_profile.join("Pictures").join("NinjaCrawler");
    if !paths_match_case_insensitive(&PathBuf::from(current_trimmed), &legacy_default) {
        return Ok(());
    }

    connection
        .execute(
            "UPDATE app_settings
             SET value = ?2, updated_at = ?3
             WHERE key = ?1",
            params![
                "storage.media_root",
                scrawler_media_root.display().to_string(),
                now_timestamp()
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn paths_match_case_insensitive(left: &Path, right: &Path) -> bool {
    normalize_path_for_compare(left) == normalize_path_for_compare(right)
}

fn normalize_path_for_compare(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('/', "\\");
    raw.trim_end_matches('\\').to_ascii_lowercase()
}

fn scheduler_notifications_for_mode(mode: &str) -> SchedulerPlanNotifications {
    match mode {
        "detailed" => SchedulerPlanNotifications {
            enabled: true,
            simple: false,
            show_image: true,
            show_user_icon: true,
        },
        "summary" => SchedulerPlanNotifications {
            enabled: true,
            simple: true,
            show_image: false,
            show_user_icon: false,
        },
        _ => SchedulerPlanNotifications::default(),
    }
}

fn scheduler_notification_mode_for_struct(value: &SchedulerPlanNotifications) -> String {
    if !value.enabled || value.simple {
        "summary".to_string()
    } else {
        "detailed".to_string()
    }
}

fn normalize_scheduler_criteria(
    mut criteria: SchedulerPlanCriteria,
    target_filter: &str,
) -> SchedulerPlanCriteria {
    if criteria.sites_included.is_empty() {
        criteria.sites_included = Vec::new();
    }
    if criteria.sites_excluded.is_empty() {
        criteria.sites_excluded = Vec::new();
    }
    if criteria
        .advanced_expression
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
        && !target_filter.trim().is_empty()
    {
        criteria.advanced_expression = Some(target_filter.trim().to_string());
    }
    criteria
}

fn parse_scheduler_notifications(
    value: &str,
    notification_mode: &str,
) -> SchedulerPlanNotifications {
    serde_json::from_str(value)
        .unwrap_or_else(|_| scheduler_notifications_for_mode(notification_mode))
}

fn parse_scheduler_criteria(value: &str, target_filter: &str) -> SchedulerPlanCriteria {
    let parsed = serde_json::from_str(value).unwrap_or_default();
    normalize_scheduler_criteria(parsed, target_filter)
}

fn serialize_scheduler_notifications(value: &SchedulerPlanNotifications) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| error.to_string())
}

fn serialize_scheduler_criteria(value: &SchedulerPlanCriteria) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| error.to_string())
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

#[allow(dead_code)]
fn load_scheduler_group_by_id(
    connection: &Connection,
    group_id: &str,
) -> Result<Option<SchedulerGroup>, String> {
    connection
        .query_row(
            "SELECT id, name, sort_index, criteria_json
             FROM scheduler_groups
             WHERE id = ?1
             LIMIT 1",
            params![group_id],
            |row| {
                let criteria_json = row.get::<_, String>(3)?;
                Ok(SchedulerGroup {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    sort_index: row.get(2)?,
                    criteria: serde_json::from_str(&criteria_json).unwrap_or_default(),
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn upsert_scheduler_set_with_connection(
    connection: &Connection,
    input: SchedulerSetUpsert,
) -> Result<(), String> {
    let id = input.id.unwrap_or_else(new_id);
    let now = now_timestamp();

    if input.active {
        connection
            .execute(
                "UPDATE scheduler_sets SET is_active = 0, updated_at = ?1",
                params![now.clone()],
            )
            .map_err(|error| error.to_string())?;
    }

    connection
        .execute(
            "INSERT INTO scheduler_sets (id, name, is_active, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               is_active = excluded.is_active,
               updated_at = excluded.updated_at",
            params![id, input.name, bool_to_int(input.active), now],
        )
        .map_err(|error| error.to_string())?;

    Ok(())
}

fn upsert_scheduler_group_with_connection(
    connection: &Connection,
    input: SchedulerGroupUpsert,
) -> Result<(), String> {
    let id = input.id.unwrap_or_else(new_id);
    let now = now_timestamp();
    let criteria_json = serialize_scheduler_criteria(&input.criteria)?;
    connection
        .execute(
            "INSERT INTO scheduler_groups (id, name, sort_index, criteria_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               sort_index = excluded.sort_index,
               criteria_json = excluded.criteria_json,
               updated_at = excluded.updated_at",
            params![
                id,
                input.name.trim(),
                input.sort_index.unwrap_or(0),
                criteria_json,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn upsert_sync_plan_with_connection(
    connection: &Connection,
    input: SyncPlanUpsert,
) -> Result<(), String> {
    let id = input.id.unwrap_or_else(new_id);
    let now = now_timestamp();
    let notifications = if input.notifications == SchedulerPlanNotifications::default() {
        scheduler_notifications_for_mode(&input.notification_mode)
    } else {
        input.notifications.clone()
    };
    let criteria = normalize_scheduler_criteria(input.criteria.clone(), &input.target_filter);
    let notifications_json = serialize_scheduler_notifications(&notifications)?;
    let criteria_json = serialize_scheduler_criteria(&criteria)?;
    connection
        .execute(
            "INSERT INTO sync_plans (
                id,
                scheduler_set_id,
                name,
                enabled,
                mode,
                interval_minutes,
                startup_delay_minutes,
                notification_mode,
                target_filter,
                sort_index,
                pause_mode,
                pause_until,
                notifications_json,
                criteria_json,
                created_at,
                updated_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)
             ON CONFLICT(id) DO UPDATE SET
               scheduler_set_id = excluded.scheduler_set_id,
               name = excluded.name,
               enabled = excluded.enabled,
               mode = excluded.mode,
               interval_minutes = excluded.interval_minutes,
               startup_delay_minutes = excluded.startup_delay_minutes,
               notification_mode = excluded.notification_mode,
               target_filter = excluded.target_filter,
               sort_index = excluded.sort_index,
               pause_mode = excluded.pause_mode,
               pause_until = excluded.pause_until,
               notifications_json = excluded.notifications_json,
               criteria_json = excluded.criteria_json,
               updated_at = excluded.updated_at",
            params![
                id,
                input.scheduler_set_id,
                input.name,
                bool_to_int(input.enabled),
                input.mode,
                i64::from(input.interval_minutes),
                i64::from(input.startup_delay_minutes),
                scheduler_notification_mode_for_struct(&notifications),
                criteria
                    .advanced_expression
                    .clone()
                    .unwrap_or_else(|| input.target_filter.clone()),
                input.sort_index.unwrap_or(0),
                input.pause_mode.unwrap_or_else(|| "disabled".to_string()),
                input.pause_until,
                notifications_json,
                criteria_json,
                now
            ],
        )
        .map_err(|error| error.to_string())?;

    Ok(())
}

fn upsert_provider_account_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    input: ProviderAccountUpsert,
) -> Result<WorkspaceSnapshot, String> {
    let id = input.id.unwrap_or_else(new_id);
    validate_bound_source_provider_integrity(connection, &id, &input.provider)?;

    let now = now_timestamp();
    let capabilities = to_json_array(&input.capabilities)?;
    let last_validated_at = input.last_validated_at.unwrap_or_else(now_timestamp);
    connection
        .execute(
            "INSERT INTO provider_accounts (id, provider, display_name, auth_mode, auth_state, capabilities_json, last_validated_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
             ON CONFLICT(id) DO UPDATE SET
               provider = excluded.provider,
               display_name = excluded.display_name,
               auth_mode = excluded.auth_mode,
               auth_state = excluded.auth_state,
               capabilities_json = excluded.capabilities_json,
               last_validated_at = excluded.last_validated_at,
               updated_at = excluded.updated_at",
            params![
                id,
                input.provider,
                input.display_name,
                input.auth_mode,
                input.auth_state,
                capabilities,
                last_validated_at,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
    load_snapshot(connection, layout)
}

fn delete_provider_account_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    id: String,
) -> Result<WorkspaceSnapshot, String> {
    let bound_source = connection
        .query_row(
            "SELECT handle
             FROM source_profiles
             WHERE account_id = ?1
               AND deleted_at IS NULL
             LIMIT 1",
            params![&id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;

    if let Some(handle) = bound_source {
        return Err(format!(
            "Cannot delete provider account while source '{}' is bound to it.",
            handle
        ));
    }

    if let Some(secret_ref) = load_account_session_secret_ref(connection, &id)? {
        session_secret_store::delete_secret(layout, &secret_ref)?;
    }
    if let Some(secret_ref) = load_account_import_backup_secret_ref(connection, &id)? {
        if load_account_session_secret_ref(connection, &id)?.as_deref() != Some(secret_ref.as_str()) {
            session_secret_store::delete_secret(layout, &secret_ref)?;
        }
    }

    connection
        .execute("DELETE FROM provider_accounts WHERE id = ?1", params![id])
        .map_err(|error| error.to_string())?;
    load_snapshot(connection, layout)
}

fn load_provider_account_editor_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    account_id: String,
) -> Result<ProviderAccountEditor, String> {
    let account = load_provider_account_by_id(connection, &account_id)?;
    let session = load_account_session(connection, layout, &account_id)?;
    let mut settings = load_provider_account_settings(connection, &account_id)?;
    if let Some(secret_ref) = load_account_session_secret_ref(connection, &account_id)? {
        if let Ok(secret) = session_secret_store::load_secret(layout, &secret_ref) {
            if let Ok(parsed) = parse_session_payload(&secret) {
                merge_protected_authorization_settings(
                    &mut settings,
                    &account.provider,
                    &parsed.metadata,
                );
            }
        }
    }
    Ok(ProviderAccountEditor {
        account,
        session,
        settings,
        import_state: load_provider_account_import_state(connection, &account_id)?,
    })
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
            ("instagram.auth.secChUaFullVersionList", metadata.sec_ch_ua_full_version_list.clone()),
            ("instagram.auth.secChUaPlatformVersion", metadata.sec_ch_ua_platform_version.clone()),
        ],
        "twitter" => vec![
            ("twitter.auth.useUserAgent", metadata.user_agent.as_ref().map(|_| "true".to_string())),
            ("twitter.auth.userAgent", metadata.user_agent.clone()),
        ],
        "tiktok" => vec![
            ("tiktok.auth.useUserAgent", metadata.user_agent.as_ref().map(|_| "true".to_string())),
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

fn load_provider_account_import_state(
    connection: &Connection,
    account_id: &str,
) -> Result<Option<ProviderAccountImportState>, String> {
    connection
        .query_row(
            "SELECT account_id, provider_user_id, provider_username, last_imported_at,
                    backup_secret_ref, backup_imported_at
             FROM provider_account_import_state
             WHERE account_id = ?1",
            params![account_id],
            |row| {
                Ok(ProviderAccountImportState {
                    account_id: row.get(0)?,
                    provider_user_id: row.get(1)?,
                    provider_username: row.get(2)?,
                    last_imported_at: row.get(3)?,
                    can_revert: row.get::<_, Option<String>>(4)?.is_some(),
                    backup_imported_at: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn save_provider_account_settings_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    account_id: String,
    values: Vec<ProviderAccountSettingValue>,
) -> Result<ProviderAccountEditor, String> {
    ensure_provider_account_exists(connection, &account_id)?;
    let account = load_provider_account_by_id(connection, &account_id)?;
    let protect_authorization = load_provider_account_import_state(connection, &account_id)?.is_some();

    let mut seen_keys = HashSet::new();
    let mut serialized_values = Vec::new();
    let mut protected_values = HashMap::new();
    for value in values {
        let setting_key = value.setting_key.trim();
        if setting_key.is_empty() {
            return Err("Provider account setting key cannot be empty.".to_string());
        }

        if !seen_keys.insert(setting_key.to_string()) {
            return Err(format!(
                "Provider account setting '{}' was provided more than once.",
                setting_key
            ));
        }

        let (value_kind, value_text) = serialize_provider_account_setting_value(&value)?;
        if protect_authorization
            && is_protected_authorization_setting(&account.provider, setting_key)
        {
            protected_values.insert(setting_key.to_string(), value_text);
            continue;
        }
        serialized_values.push((setting_key.to_string(), value_kind, value_text));
    }

    if !protected_values.is_empty() {
        update_protected_authorization_metadata(
            connection,
            layout,
            &account_id,
            &account.provider,
            &protected_values,
        )?;
    }

    connection
        .execute(
            "DELETE FROM provider_account_settings WHERE account_id = ?1",
            params![&account_id],
        )
        .map_err(|error| error.to_string())?;

    let now = now_timestamp();
    for (setting_key, value_kind, value_text) in serialized_values {
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
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![&account_id, setting_key, value_kind, value_text, now],
            )
            .map_err(|error| error.to_string())?;
    }

    load_provider_account_editor_with_connection(connection, layout, account_id)
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
            params![account_id, session_fingerprint(&updated_payload), now_timestamp()],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn clone_provider_account_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    account_id: String,
) -> Result<WorkspaceSnapshot, String> {
    let source_account = load_provider_account_by_id(connection, &account_id)?;
    let source_settings = load_provider_account_settings(connection, &account_id)?;
    let cloned_account_id = new_id();
    let cloned_display_name =
        next_cloned_account_display_name(connection, &source_account.display_name)?;
    let now = now_timestamp();

    connection
        .execute(
            "INSERT INTO provider_accounts (
                id,
                provider,
                display_name,
                auth_mode,
                auth_state,
                capabilities_json,
                last_validated_at,
                created_at,
                updated_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![
                &cloned_account_id,
                &source_account.provider,
                cloned_display_name,
                &source_account.auth_mode,
                &source_account.auth_state,
                to_json_array(&source_account.capabilities)?,
                &source_account.last_validated_at,
                now
            ],
        )
        .map_err(|error| error.to_string())?;

    for value in source_settings {
        let (value_kind, value_text) = serialize_provider_account_setting_value(&value)?;
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
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![
                    &cloned_account_id,
                    value.setting_key,
                    value_kind,
                    value_text,
                    now
                ],
            )
            .map_err(|error| error.to_string())?;
    }

    load_snapshot(connection, layout)
}

fn next_cloned_account_display_name(
    connection: &Connection,
    display_name: &str,
) -> Result<String, String> {
    let base_name = if display_name.trim().is_empty() {
        "Cloned account".to_string()
    } else {
        format!("{} Copy", display_name.trim())
    };

    if !provider_account_display_name_exists(connection, &base_name)? {
        return Ok(base_name);
    }

    let mut counter = 2_u32;
    loop {
        let candidate = format!("{base_name} {counter}");
        if !provider_account_display_name_exists(connection, &candidate)? {
            return Ok(candidate);
        }
        counter += 1;
    }
}

fn provider_account_display_name_exists(
    connection: &Connection,
    display_name: &str,
) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT 1 FROM provider_accounts WHERE display_name = ?1 LIMIT 1",
            params![display_name],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| error.to_string())
        .map(|result| result.is_some())
}

fn preview_instagram_scrawler_import_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    options: ImportPreviewOptions,
) -> Result<ImportPreview, String> {
    let roots = collect_instagram_import_roots(
        connection,
        layout,
        &options.manual_roots,
        &options.disabled_roots,
    )?;
    let root_labels = roots
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let accounts = load_accounts(connection)?;
    let sources = load_sources(connection)?;
    let imported_roots =
        load_external_imported_entity_keys(connection, INSTAGRAM_SCRAWLER_IMPORTER_ID)?;
    let candidates = collect_scrawler_instagram_candidates(&roots)?;
    let duplicate_handles = collect_duplicate_import_handles(&candidates);

    let profiles = candidates
        .iter()
        .map(|candidate| {
            build_instagram_scrawler_preview_profile(
                candidate,
                &accounts,
                &sources,
                &imported_roots,
                &duplicate_handles,
                options.force_reimport,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    let summary = ImportPreviewSummary {
        detected_profiles: profiles.len() as u32,
        ready_profiles: profiles
            .iter()
            .filter(|profile| profile.import_state == "ready")
            .count() as u32,
        blocked_profiles: profiles
            .iter()
            .filter(|profile| {
                matches!(
                    profile.import_state.as_str(),
                    "needs_account_link" | "duplicate_conflict" | "no_media"
                )
            })
            .count() as u32,
        already_imported_profiles: profiles
            .iter()
            .filter(|profile| profile.already_imported)
            .count() as u32,
        importable_files: profiles.iter().map(|profile| profile.file_count).sum(),
    };

    Ok(ImportPreview {
        importer_id: INSTAGRAM_SCRAWLER_IMPORTER_ID.to_string(),
        provider: "instagram".to_string(),
        method_label: "SCrawler".to_string(),
        force_reimport: options.force_reimport,
        roots: root_labels,
        profiles,
        summary,
    })
}

fn list_instagram_scrawler_import_roots_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    manual_roots: &[String],
    disabled_roots: &[String],
) -> Result<Vec<ImportRootDescriptor>, String> {
    collect_instagram_import_root_descriptors(connection, layout, manual_roots, disabled_roots)
}

fn merge_import_root_descriptors(
    existing: &mut ImportRootDescriptor,
    incoming: ImportRootDescriptor,
) {
    if existing.source == "manual" && incoming.source != "manual" {
        existing.source = incoming.source;
        existing.label = incoming.label;
        existing.removable = incoming.removable;
    }
}

fn run_instagram_scrawler_import_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    input: ImportRunRequest,
) -> Result<ImportRunResult, String> {
    let preview = preview_instagram_scrawler_import_with_connection(
        connection,
        layout,
        ImportPreviewOptions {
            force_reimport: input.force_reimport,
            manual_roots: input.manual_roots.clone(),
            disabled_roots: input.disabled_roots.clone(),
        },
    )?;
    let roots = collect_instagram_import_roots(
        connection,
        layout,
        &input.manual_roots,
        &input.disabled_roots,
    )?;
    let candidates = collect_scrawler_instagram_candidates(&roots)?;
    let candidate_by_root = candidates
        .into_iter()
        .map(|candidate| {
            (
                normalize_import_entity_key(&candidate.profile_root),
                candidate,
            )
        })
        .collect::<HashMap<_, _>>();
    let resolution_by_root = input
        .resolutions
        .into_iter()
        .map(|resolution| {
            (
                normalize_import_entity_key(Path::new(&resolution.profile_root)),
                resolution,
            )
        })
        .collect::<HashMap<_, _>>();

    let mut imported_profiles = 0u32;
    let mut skipped_profiles = 0u32;
    let mut failed_profiles = 0u32;
    let mut imported_media_count = 0u32;
    let mut already_cataloged_count = 0u32;
    let mut profiles = Vec::new();

    for preview_profile in &preview.profiles {
        let profile_key = normalize_import_entity_key(Path::new(&preview_profile.profile_root));
        let resolution = resolution_by_root.get(&profile_key);
        let action = resolution
            .map(|entry| entry.action.as_str())
            .unwrap_or_else(|| {
                if preview_profile.import_state == "already_imported" && !input.force_reimport {
                    "skip"
                } else {
                    "import"
                }
            });

        if action.eq_ignore_ascii_case("skip") {
            skipped_profiles += 1;
            profiles.push(ImportRunProfileResult {
                profile_root: preview_profile.profile_root.clone(),
                handle: preview_profile.handle.clone(),
                status: "skipped".to_string(),
                source_id: preview_profile.source_id.clone(),
                imported_media_count: 0,
                already_cataloged_count: 0,
                message: "Skipped by import review.".to_string(),
            });
            continue;
        }

        let resolved_account_id = resolution
            .and_then(|entry| entry.account_id.clone())
            .or_else(|| preview_profile.account_id.clone());

        let can_proceed = match preview_profile.import_state.as_str() {
            "ready" | "already_imported" => true,
            "needs_account_link" => resolved_account_id.is_some(),
            _ => false,
        };

        if !can_proceed {
            failed_profiles += 1;
            profiles.push(ImportRunProfileResult {
                profile_root: preview_profile.profile_root.clone(),
                handle: preview_profile.handle.clone(),
                status: "failed".to_string(),
                source_id: preview_profile.source_id.clone(),
                imported_media_count: 0,
                already_cataloged_count: 0,
                message: format!(
                    "Profile is blocked in review state '{}'.",
                    preview_profile.import_state
                ),
            });
            continue;
        }

        if preview_profile.import_state == "already_imported" && !input.force_reimport {
            skipped_profiles += 1;
            profiles.push(ImportRunProfileResult {
                profile_root: preview_profile.profile_root.clone(),
                handle: preview_profile.handle.clone(),
                status: "skipped".to_string(),
                source_id: preview_profile.source_id.clone(),
                imported_media_count: 0,
                already_cataloged_count: 0,
                message:
                    "Profile was already imported. Enable force re-import to process it again."
                        .to_string(),
            });
            continue;
        }

        let Some(candidate) = candidate_by_root.get(&profile_key) else {
            failed_profiles += 1;
            profiles.push(ImportRunProfileResult {
                profile_root: preview_profile.profile_root.clone(),
                handle: preview_profile.handle.clone(),
                status: "failed".to_string(),
                source_id: preview_profile.source_id.clone(),
                imported_media_count: 0,
                already_cataloged_count: 0,
                message: "The legacy profile root is no longer available.".to_string(),
            });
            continue;
        };

        match import_instagram_scrawler_profile_with_connection(
            connection,
            layout,
            candidate,
            preview_profile,
            resolved_account_id,
            input.force_reimport,
        ) {
            Ok(result) => {
                imported_profiles += 1;
                imported_media_count += result.imported_media_count;
                already_cataloged_count += result.already_cataloged_count;
                profiles.push(result);
            }
            Err(error) => {
                failed_profiles += 1;
                profiles.push(ImportRunProfileResult {
                    profile_root: preview_profile.profile_root.clone(),
                    handle: preview_profile.handle.clone(),
                    status: "failed".to_string(),
                    source_id: preview_profile.source_id.clone(),
                    imported_media_count: 0,
                    already_cataloged_count: 0,
                    message: error,
                });
            }
        }
    }

    Ok(ImportRunResult {
        importer_id: INSTAGRAM_SCRAWLER_IMPORTER_ID.to_string(),
        imported_profiles,
        skipped_profiles,
        failed_profiles,
        imported_media_count,
        already_cataloged_count,
        profiles,
    })
}

fn import_instagram_scrawler_profile_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    candidate: &ImportCandidateProfile,
    preview_profile: &ImportPreviewProfile,
    resolved_account_id: Option<String>,
    force_reimport: bool,
) -> Result<ImportRunProfileResult, String> {
    connection
        .execute("BEGIN IMMEDIATE TRANSACTION", [])
        .map_err(|error| error.to_string())?;

    let operation = (|| -> Result<ImportRunProfileResult, String> {
        let source = if let Some(source_id) = preview_profile.source_id.as_deref() {
            load_sources(connection)?
                .into_iter()
                .find(|entry| entry.id == source_id)
                .ok_or_else(|| format!("Source '{}' is no longer available.", source_id))?
        } else {
            let account_id = resolved_account_id.ok_or_else(|| {
                "This profile requires an explicit Instagram account link before import."
                    .to_string()
            })?;
            let source_id = new_id();
            let handle =
                legacy_instagram_profile_handle(&candidate.profile, &candidate.folder_name);
            let display_name = legacy_instagram_profile_display_name(&candidate.profile, &handle);
            let mut instagram_sync_options = default_instagram_source_sync_options();
            instagram_sync_options.timeline = candidate.profile.get_timeline;
            instagram_sync_options.reels = candidate.profile.get_reels;
            instagram_sync_options.stories = candidate.profile.get_stories;
            instagram_sync_options.stories_user = candidate.profile.get_stories_user;
            instagram_sync_options.tagged = candidate.profile.get_tagged_data;
            // Always store the absolute profile root as special_path for imported profiles
            // so media resolution works regardless of the storage.media_root setting.
            instagram_sync_options.special_path =
                Some(candidate.profile_root.display().to_string());
            instagram_sync_options.user_id_hint = candidate.profile.user_id.clone();
            // O SCrawler guarda o nome legado em UserName e o nome atual em
            // TrueName (usado como handle). Quando diferem, registra o UserName
            // como nome anterior para que a busca encontre pelo nome antigo.
            if let Some(legacy_username) = candidate.profile.user_name.as_deref() {
                instagram_sync_options.previous_handles = push_previous_instagram_handle(
                    instagram_sync_options.previous_handles.take(),
                    legacy_username,
                    &handle,
                );
            }
            let sync_options = SourceSyncOptions {
                instagram: Some(instagram_sync_options),
                ..Default::default()
            };

            let _ = upsert_source_profile_with_connection(
                connection,
                layout,
                SourceProfileUpsert {
                    id: Some(source_id.clone()),
                    provider: "instagram".to_string(),
                    source_kind: "profile".to_string(),
                    handle,
                    display_name,
                    account_id: Some(account_id),
                    group_id: None,
                    labels: Vec::new(),
                    ready_for_download: candidate.profile.ready_for_download,
                    sync_options,
                    remote_state: None,
                    is_subscription: None,
                },
            )?;

            let source = load_sources(connection)?
                .into_iter()
                .find(|entry| entry.id == source_id)
                .ok_or_else(|| "Imported source was not persisted.".to_string())?;

            if !source.profile_image_custom {
                let avatar_path = candidate.profile_root.join(PROFILE_PICTURE_FILE_NAME);
                if avatar_path.exists() {
                    let normalized_avatar_path = normalize_media_file_path(&avatar_path)?;
                    update_source_profile_image(
                        connection,
                        &source.id,
                        &normalized_avatar_path,
                        &now_timestamp(),
                    )?;
                }
            }

            source
        };

        let imported_at = now_timestamp();
        if let Some(profile_description) = candidate.profile.description.as_deref() {
            update_instagram_source_description_after_sync(
                connection,
                &source,
                profile_description,
                false,
                &imported_at,
            )?;
        }

        let importable_media = collect_legacy_instagram_media_candidates(&candidate.profile_root)?;
        if importable_media.is_empty() {
            return Err(
                "No importable image or video files were found in this legacy profile.".to_string(),
            );
        }

        if preview_profile.import_state == "already_imported" && !force_reimport {
            return Err(
                "This legacy profile was already imported. Enable force re-import to run it again."
                    .to_string(),
            );
        }

        let file_count = importable_media.len() as u32;
        let (imported_count, already_count) = if preview_profile.already_imported {
            (0, file_count)
        } else {
            (file_count, 0)
        };
        let account_id = source.account_id.clone().ok_or_else(|| {
            "Imported source is missing its explicit provider account binding.".to_string()
        })?;
        record_external_import_ledger(
            connection,
            INSTAGRAM_SCRAWLER_IMPORTER_ID,
            &candidate.profile_root,
            "instagram",
            &source.handle,
            &source.id,
            &account_id,
            &imported_at,
        )?;
        let reconciliation = reconcile_instagram_scrawler_profile_ledgers_with_connection(
            connection,
            &candidate.profile_root,
            &source.id,
            &account_id,
            &source.handle,
            &imported_at,
        )?;

        Ok(ImportRunProfileResult {
            profile_root: candidate.profile_root.to_string_lossy().into_owned(),
            handle: source.handle.clone(),
            status: "imported".to_string(),
            source_id: Some(source.id.clone()),
            imported_media_count: imported_count,
            already_cataloged_count: already_count + reconciliation.seeded_post_entries,
            message: if imported_count > 0 {
                format!(
                    "Imported {} file(s); {} file(s) were already present; reconciled {} media entrie(s) and {} post entrie(s).",
                    imported_count,
                    already_count,
                    reconciliation.seeded_media_entries,
                    reconciliation.seeded_post_entries
                )
            } else {
                format!(
                    "No new files were added; {} file(s) were already present; reconciled {} media entrie(s) and {} post entrie(s).",
                    already_count,
                    reconciliation.seeded_media_entries,
                    reconciliation.seeded_post_entries
                )
            },
        })
    })();

    match operation {
        Ok(result) => {
            connection
                .execute("COMMIT", [])
                .map_err(|error| error.to_string())?;
            Ok(result)
        }
        Err(error) => {
            let _ = connection.execute("ROLLBACK", []);
            Err(error)
        }
    }
}

fn build_instagram_scrawler_preview_profile(
    candidate: &ImportCandidateProfile,
    accounts: &[ProviderAccount],
    sources: &[SourceProfile],
    imported_roots: &HashSet<String>,
    duplicate_handles: &HashSet<String>,
    force_reimport: bool,
) -> Result<ImportPreviewProfile, String> {
    let handle = legacy_instagram_profile_handle(&candidate.profile, &candidate.folder_name);
    let display_name = legacy_instagram_profile_display_name(&candidate.profile, &handle);
    let profile_root_key = normalize_import_entity_key(&candidate.profile_root);
    let already_imported = imported_roots.contains(&profile_root_key);
    let source_match = sources.iter().find(|source| {
        source.provider.eq_ignore_ascii_case("instagram")
            && source.handle.eq_ignore_ascii_case(&handle)
    });
    let account_matches = accounts
        .iter()
        .filter(|account| {
            account.provider.eq_ignore_ascii_case("instagram")
                && candidate
                    .profile
                    .account_name
                    .as_deref()
                    .is_some_and(|account_name| {
                        account.display_name.eq_ignore_ascii_case(account_name)
                    })
        })
        .collect::<Vec<_>>();
    let resolved_account = if source_match.is_none() && account_matches.len() == 1 {
        account_matches.first().copied()
    } else {
        None
    };
    let media_candidates = collect_legacy_instagram_media_candidates(&candidate.profile_root)?;
    let file_count = media_candidates.len() as u32;
    let already_cataloged_count = if already_imported && !force_reimport {
        file_count
    } else {
        0
    };
    let new_file_count = file_count.saturating_sub(already_cataloged_count);
    let avatar_path = candidate.profile_root.join(PROFILE_PICTURE_FILE_NAME);
    let duplicate_handle = duplicate_handles.contains(&handle.to_ascii_lowercase());
    let mut problems = Vec::new();

    if sanitize_source_handle("instagram", &candidate.folder_name)
        != sanitize_source_handle("instagram", &handle)
    {
        problems.push(ImportProblem {
            severity: "warning".to_string(),
            code: "folder-handle-mismatch".to_string(),
            message: format!(
                "Folder '{}' differs from the XML handle '{}'. The XML handle will be used.",
                candidate.folder_name, handle
            ),
        });
    }

    if duplicate_handle {
        problems.push(ImportProblem {
            severity: "error".to_string(),
            code: "duplicate-handle".to_string(),
            message: format!(
                "More than one legacy folder resolved to the Instagram handle '{}'. Mark one of them as skip before importing.",
                handle
            ),
        });
    }

    if file_count == 0 {
        problems.push(ImportProblem {
            severity: "error".to_string(),
            code: "no-media".to_string(),
            message: "No importable image or video files were found in this legacy profile root."
                .to_string(),
        });
    }

    if source_match.is_none() && resolved_account.is_none() {
        match account_matches.len() {
            0 => problems.push(ImportProblem {
                severity: "error".to_string(),
                code: "account-match-missing".to_string(),
                message: if let Some(account_name) = candidate.profile.account_name.as_deref() {
                    format!(
                        "No Instagram account named '{}' was found. Link an account manually to import this folder.",
                        account_name
                    )
                } else {
                    "The legacy XML does not expose a usable AccountName. Link an account manually to import this folder.".to_string()
                },
            }),
            _ => problems.push(ImportProblem {
                severity: "error".to_string(),
                code: "account-match-ambiguous".to_string(),
                message: "More than one Instagram account matched the legacy AccountName. Choose the target account manually.".to_string(),
            }),
        }
    }

    if already_imported && !force_reimport {
        problems.push(ImportProblem {
            severity: "warning".to_string(),
            code: "already-imported".to_string(),
            message: "This legacy folder was already imported. Enable force re-import to process it again.".to_string(),
        });
    }

    let import_state = if duplicate_handle {
        "duplicate_conflict"
    } else if file_count == 0 {
        "no_media"
    } else if already_imported && !force_reimport {
        "already_imported"
    } else if source_match.is_none() && resolved_account.is_none() {
        "needs_account_link"
    } else {
        "ready"
    };

    Ok(ImportPreviewProfile {
        profile_root: candidate.profile_root.to_string_lossy().into_owned(),
        user_xml_path: candidate.user_xml_path.to_string_lossy().into_owned(),
        handle,
        display_name,
        account_name: candidate.profile.account_name.clone(),
        source_id: source_match.map(|source| source.id.clone()),
        source_display_name: source_match.map(|source| source.display_name.clone()),
        source_handle: source_match.map(|source| source.handle.clone()),
        account_id: source_match
            .and_then(|source| source.account_id.clone())
            .or_else(|| resolved_account.map(|account| account.id.clone())),
        account_display_name: source_match
            .and_then(|source| source.account_id.as_ref())
            .and_then(|source_account_id| {
                accounts
                    .iter()
                    .find(|account| account.id == *source_account_id)
                    .map(|account| account.display_name.clone())
            })
            .or_else(|| resolved_account.map(|account| account.display_name.clone())),
        avatar_path: avatar_path
            .exists()
            .then(|| avatar_path.to_string_lossy().into_owned()),
        already_imported,
        import_state: import_state.to_string(),
        file_count,
        already_cataloged_count,
        new_file_count,
        problems,
    })
}

fn collect_instagram_import_root_descriptors(
    connection: &Connection,
    layout: &StorageLayout,
    manual_roots: &[String],
    disabled_roots: &[String],
) -> Result<Vec<ImportRootDescriptor>, String> {
    let mut roots = Vec::new();
    roots.push(ImportRootDescriptor {
        path: layout
            .media_root
            .join("instagram")
            .to_string_lossy()
            .into_owned(),
        source: "default".to_string(),
        label: "Media root".to_string(),
        removable: false,
    });

    for account in load_accounts(connection)?
        .into_iter()
        .filter(|entry| entry.provider.eq_ignore_ascii_case("instagram"))
    {
        let settings = load_provider_account_settings_map(connection, &account.id)?;
        if let Some(media_path) = setting_value(&settings, "instagram.account.mediaPath") {
            roots.push(ImportRootDescriptor {
                path: media_path,
                source: "account".to_string(),
                label: format!("Account path · {}", account.display_name),
                removable: false,
            });
        }
    }

    for manual_root in manual_roots {
        let trimmed = manual_root.trim();
        if !trimmed.is_empty() {
            roots.push(ImportRootDescriptor {
                path: trimmed.to_string(),
                source: "manual".to_string(),
                label: "Manual root".to_string(),
                removable: true,
            });
        }
    }

    let mut unique_roots = HashMap::new();
    for root in roots {
        let key = normalize_import_entity_key(Path::new(&root.path));
        match unique_roots.entry(key) {
            std::collections::hash_map::Entry::Occupied(mut occupied) => {
                merge_import_root_descriptors(occupied.get_mut(), root);
            }
            std::collections::hash_map::Entry::Vacant(vacant) => {
                vacant.insert(root);
            }
        }
    }

    let disabled_keys = disabled_roots
        .iter()
        .filter_map(|root| {
            let trimmed = root.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(normalize_import_entity_key(Path::new(trimmed)))
            }
        })
        .collect::<HashSet<_>>();

    let mut deduped = unique_roots
        .into_iter()
        .filter_map(|(key, root)| (!disabled_keys.contains(&key)).then_some(root))
        .collect::<Vec<_>>();
    deduped.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(deduped)
}

fn collect_instagram_import_roots(
    connection: &Connection,
    layout: &StorageLayout,
    manual_roots: &[String],
    disabled_roots: &[String],
) -> Result<Vec<PathBuf>, String> {
    collect_instagram_import_root_descriptors(connection, layout, manual_roots, disabled_roots).map(
        |roots| {
            roots
                .into_iter()
                .map(|entry| PathBuf::from(entry.path))
                .collect::<Vec<_>>()
        },
    )
}

fn collect_scrawler_instagram_candidates(
    roots: &[PathBuf],
) -> Result<Vec<ImportCandidateProfile>, String> {
    let mut candidates = HashMap::new();

    for root in roots {
        if !root.exists() {
            continue;
        }

        let mut pending = vec![root.clone()];
        while let Some(current) = pending.pop() {
            if let Some(user_xml_path) = find_user_instagram_xml(&current)? {
                let key = normalize_import_entity_key(&current);
                candidates.entry(key).or_insert(ImportCandidateProfile {
                    profile_root: current.clone(),
                    user_xml_path: user_xml_path.clone(),
                    folder_name: current
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                        .to_string(),
                    profile: parse_legacy_instagram_profile_xml(&user_xml_path)?,
                });
            }

            for entry in fs::read_dir(&current).map_err(|error| error.to_string())? {
                let entry = entry.map_err(|error| error.to_string())?;
                let path = entry.path();
                if entry
                    .file_type()
                    .map_err(|error| error.to_string())?
                    .is_dir()
                {
                    pending.push(path);
                }
            }
        }
    }

    let mut collected = candidates.into_values().collect::<Vec<_>>();
    collected.sort_by(|left, right| {
        left.profile_root
            .to_string_lossy()
            .cmp(&right.profile_root.to_string_lossy())
    });
    Ok(collected)
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

fn parse_legacy_instagram_profile_xml(path: &Path) -> Result<LegacyInstagramProfileXml, String> {
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let document = roxmltree::Document::parse(&raw)
        .map_err(|error| format!("Failed to parse '{}': {error}", path.display()))?;

    Ok(LegacyInstagramProfileXml {
        account_name: xml_text(&document, "AccountName"),
        user_id: xml_text(&document, "UserID"),
        user_name: xml_text(&document, "UserName"),
        true_name: xml_text(&document, "TrueName"),
        friendly_name: xml_text(&document, "FriendlyName"),
        user_site_name: xml_text(&document, "UserSiteName"),
        description: xml_text(&document, "Description"),
        ready_for_download: xml_bool(&document, "ReadyForDownload"),
        get_timeline: xml_bool(&document, "GetTimeline"),
        get_reels: xml_bool(&document, "GetReels"),
        get_stories: xml_bool(&document, "GetStories"),
        get_stories_user: xml_bool(&document, "GetStoriesUser"),
        get_tagged_data: xml_bool(&document, "GetTaggedData"),
    })
}

fn parse_legacy_instagram_data_xml(
    path: &Path,
) -> Result<Vec<LegacyInstagramMediaXmlEntry>, String> {
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let document = roxmltree::Document::parse(&raw)
        .map_err(|error| format!("Failed to parse '{}': {error}", path.display()))?;

    let mut entries = Vec::new();
    for node in document
        .descendants()
        .filter(|node| node.has_tag_name("MediaData"))
    {
        let file_name = node
            .attribute("File")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        let provider_post_key = node
            .attribute("ID")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());

        let (Some(file_name), Some(provider_post_key)) = (file_name, provider_post_key) else {
            continue;
        };

        entries.push(LegacyInstagramMediaXmlEntry {
            file_name,
            provider_post_key,
            media_url: node
                .attribute("URL")
                .map(str::trim)
                .unwrap_or_default()
                .to_string(),
            special_folder: node
                .attribute("SpecialFolder")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string()),
            post_permalink: node
                .text()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string()),
        });
    }

    Ok(entries)
}

fn normalize_legacy_instagram_relative_path(value: &str) -> String {
    value
        .replace('\\', "/")
        .trim()
        .trim_matches('/')
        .to_ascii_lowercase()
}

fn legacy_instagram_candidate_relative_path(
    file_name: &str,
    special_folder: Option<&str>,
) -> String {
    let normalized_file_name = file_name.trim().replace('\\', "/");
    let normalized_file_name = normalized_file_name.trim_matches('/').to_ascii_lowercase();
    let normalized_folder = special_folder
        .map(normalize_legacy_instagram_relative_path)
        .unwrap_or_default();

    if normalized_folder.is_empty() {
        normalized_file_name
    } else {
        format!("{normalized_folder}/{normalized_file_name}")
    }
}

fn legacy_instagram_post_permalink(entry: &LegacyInstagramMediaXmlEntry) -> &str {
    entry
        .post_permalink
        .as_deref()
        .filter(|value| value.contains("instagram.com/"))
        .unwrap_or(&entry.media_url)
}

/// Lightweight relative-path → (cased shortcode, section) map read straight from
/// the legacy SCrawler `User_Instagram*_Data.xml`, used by ProfileView to rebuild
/// post links for media imported BEFORE the shortcode was persisted in the media
/// ledger. Unlike full reconciliation this skips per-file hashing/IO, so it is
/// cheap enough to run on gallery load. Keys are lowercased to match the gallery.
fn load_legacy_instagram_post_codes(
    profile_root: &Path,
) -> HashMap<String, (Option<String>, Option<String>)> {
    let mut map = HashMap::new();
    let Ok(Some(data_xml_path)) = find_user_instagram_data_xml(profile_root) else {
        return map;
    };
    let Ok(entries) = parse_legacy_instagram_data_xml(&data_xml_path) else {
        return map;
    };
    for entry in entries {
        let relative_path =
            legacy_instagram_candidate_relative_path(&entry.file_name, entry.special_folder.as_deref());
        if relative_path.is_empty() {
            continue;
        }
        let permalink = legacy_instagram_post_permalink(&entry);
        let post_code = extract_instagram_post_code_from_permalink_cased(permalink);
        let section = Some(infer_legacy_instagram_media_section(
            entry.special_folder.as_deref(),
            permalink,
        ));
        map.entry(relative_path).or_insert((post_code, section));
    }
    map
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
    let stem = file_name.rsplit_once('.').map(|(s, _)| s).unwrap_or(file_name);
    let (_, rest) = strip_gallery_date_prefix(stem);
    let mut key = rest.trim().to_ascii_lowercase();
    if let Some(stripped) = key.strip_prefix("gif_") {
        key = stripped.to_string();
    }
    let key = key.trim().to_string();
    (!key.is_empty()).then_some(key)
}

/// `media_key -> tweet status id` read from the legacy SCrawler Twitter XML.
/// Twitter file names never carry the status id (only the media key), so this is
/// the only local source of the post link for media imported before the status
/// id was persisted in the media ledger. Cheap (single XML parse, no file IO).
fn load_legacy_twitter_post_keys(profile_root: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(Some(data_xml_path)) = find_user_twitter_data_xml(profile_root) else {
        return map;
    };
    let Ok(entries) = parse_legacy_instagram_data_xml(&data_xml_path) else {
        return map;
    };
    for entry in entries {
        let status_id = entry.provider_post_key.trim();
        if status_id.is_empty() || !status_id.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if let Some(media_key) = twitter_media_key_from_file_name(&entry.file_name) {
            map.entry(media_key).or_insert_with(|| status_id.to_string());
        }
    }
    map
}

fn infer_legacy_instagram_media_section(special_folder: Option<&str>, permalink: &str) -> String {
    let normalized_folder = special_folder
        .map(normalize_legacy_instagram_relative_path)
        .unwrap_or_default();
    let normalized_permalink = permalink.trim().to_ascii_lowercase();

    if normalized_folder.starts_with("stories (user)") {
        "stories_user".to_string()
    } else if normalized_folder.starts_with("stories") {
        "stories".to_string()
    } else if normalized_folder.contains("tag") {
        "tagged".to_string()
    } else if normalized_folder.contains("reel") || normalized_permalink.contains("/reel/") {
        "reels".to_string()
    } else {
        "timeline".to_string()
    }
}

fn collect_legacy_instagram_reconciliation_records(
    profile_root: &Path,
) -> Result<Vec<LegacyInstagramReconciliationRecord>, String> {
    let Some(data_xml_path) = find_user_instagram_data_xml(profile_root)? else {
        return Ok(Vec::new());
    };

    let entries = parse_legacy_instagram_data_xml(&data_xml_path)?;
    if entries.is_empty() {
        return Ok(Vec::new());
    }

    let files = collect_legacy_instagram_media_candidates(profile_root)?;
    let mut files_by_relative_path = HashMap::<String, PathBuf>::new();
    let mut files_by_name = HashMap::<String, Vec<String>>::new();
    for path in files {
        let relative_path = normalize_instagram_relative_media_path(profile_root, &path);
        files_by_relative_path.insert(relative_path.clone(), path);

        if let Some(file_name) = Path::new(&relative_path)
            .file_name()
            .and_then(|value| value.to_str())
        {
            files_by_name
                .entry(file_name.to_ascii_lowercase())
                .or_default()
                .push(relative_path);
        }
    }

    let mut reconciled = Vec::new();
    for entry in entries {
        let direct_relative_path = legacy_instagram_candidate_relative_path(
            &entry.file_name,
            entry.special_folder.as_deref(),
        );
        let resolved_relative_path = if files_by_relative_path.contains_key(&direct_relative_path) {
            Some(direct_relative_path)
        } else {
            let file_name_key = Path::new(&entry.file_name)
                .file_name()
                .and_then(|value| value.to_str())
                .map(|value| value.to_ascii_lowercase());
            let Some(file_name_key) = file_name_key else {
                continue;
            };

            let Some(relative_matches) = files_by_name.get(&file_name_key) else {
                continue;
            };

            if let Some(special_folder) = entry.special_folder.as_deref() {
                let expected_prefix = normalize_legacy_instagram_relative_path(special_folder);
                relative_matches
                    .iter()
                    .find(|relative_path| {
                        relative_path.starts_with(&format!("{expected_prefix}/"))
                            || Path::new(relative_path)
                                .parent()
                                .and_then(|value| value.to_str())
                                .is_some_and(|parent| parent.eq_ignore_ascii_case(&expected_prefix))
                    })
                    .cloned()
            } else if relative_matches.len() == 1 {
                relative_matches.first().cloned()
            } else {
                None
            }
        };

        let Some(resolved_relative_path) = resolved_relative_path else {
            continue;
        };
        let Some(file_path) = files_by_relative_path.get(&resolved_relative_path).cloned() else {
            continue;
        };
        let Some(provider_media_key) = derive_instagram_media_identity_key_from_path(&file_path)
        else {
            continue;
        };
        let Some(media_type) = infer_media_type(&file_path) else {
            continue;
        };

        let permalink = legacy_instagram_post_permalink(&entry);
        let provider_post_key = normalize_instagram_post_ledger_key(&entry.provider_post_key);
        let provider_post_code = extract_instagram_post_code_from_permalink(permalink);
        let provider_post_code_cased = extract_instagram_post_code_from_permalink_cased(permalink);
        let mut alias_keys = Vec::new();
        alias_keys.push((provider_media_key.clone(), "legacy_file_path".to_string()));
        for candidate in
            extract_instagram_media_identity_candidates_from_file_name(&entry.file_name)
        {
            alias_keys.push((candidate, "legacy_xml_file_name".to_string()));
        }
        if let Some(raw_file_name) = entry.media_url.split('?').next() {
            for candidate in
                extract_instagram_media_identity_candidates_from_file_name(raw_file_name)
            {
                alias_keys.push((candidate, "legacy_media_url".to_string()));
            }
        }
        if !provider_post_key.is_empty() {
            alias_keys.push((provider_post_key.clone(), "legacy_post_id".to_string()));
        }
        if let Some(provider_post_code) = provider_post_code.clone() {
            alias_keys.push((provider_post_code, "legacy_post_code".to_string()));
        }
        let file_sha256 = compute_file_sha256(&file_path).ok();

        reconciled.push(LegacyInstagramReconciliationRecord {
            file_path,
            legacy_file_name: entry.file_name.clone(),
            provider_media_key,
            alias_keys,
            file_sha256,
            provider_post_key,
            provider_post_code,
            provider_post_code_cased,
            media_type: media_type.to_string(),
            media_section: infer_legacy_instagram_media_section(
                entry.special_folder.as_deref(),
                permalink,
            ),
        });
    }

    Ok(reconciled)
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

fn collect_duplicate_import_handles(candidates: &[ImportCandidateProfile]) -> HashSet<String> {
    let mut seen = HashSet::new();
    let mut duplicates = HashSet::new();

    for candidate in candidates {
        let handle = legacy_instagram_profile_handle(&candidate.profile, &candidate.folder_name)
            .to_ascii_lowercase();

        if !seen.insert(handle.clone()) {
            duplicates.insert(handle);
        }
    }

    duplicates
}

fn collect_legacy_instagram_media_candidates(profile_root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut collected = Vec::new();
    let mut pending = vec![profile_root.to_path_buf()];

    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|error| error.to_string())?;

            if file_type.is_dir() {
                if path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case("Settings"))
                {
                    continue;
                }
                pending.push(path);
                continue;
            }

            if !file_type.is_file() {
                continue;
            }

            if is_profile_picture_file(&path) {
                continue;
            }

            if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| {
                    value.eq_ignore_ascii_case("xml") || value.eq_ignore_ascii_case("txt")
                })
            {
                continue;
            }

            if infer_media_type(&path).is_none() {
                continue;
            }

            collected.push(path.clone());
        }
    }

    collected.sort();
    Ok(collected)
}

fn load_external_imported_entity_keys(
    connection: &Connection,
    importer_id: &str,
) -> Result<HashSet<String>, String> {
    ensure_external_import_ledger_table(connection)?;
    let mut statement = connection
        .prepare(
            "SELECT entity_key
             FROM external_import_ledger
             WHERE importer_id = ?1",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![importer_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;

    let mut keys = HashSet::new();
    for row in rows {
        keys.insert(row.map_err(|error| error.to_string())?);
    }
    Ok(keys)
}

fn normalize_import_entity_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

fn legacy_instagram_profile_handle(
    profile: &LegacyInstagramProfileXml,
    folder_name: &str,
) -> String {
    sanitize_source_handle(
        "instagram",
        profile
            .true_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or(profile.user_name.as_deref())
            .unwrap_or(folder_name),
    )
}

fn legacy_instagram_profile_display_name(
    profile: &LegacyInstagramProfileXml,
    handle: &str,
) -> String {
    profile
        .friendly_name
        .as_deref()
        .or(profile.true_name.as_deref())
        .or(profile.user_site_name.as_deref())
        .unwrap_or(handle)
        .trim()
        .to_string()
}

fn record_external_import_ledger(
    connection: &Connection,
    importer_id: &str,
    profile_root: &Path,
    provider: &str,
    handle: &str,
    source_id: &str,
    account_id: &str,
    timestamp: &str,
) -> Result<(), String> {
    ensure_external_import_ledger_table(connection)?;
    connection
        .execute(
            "INSERT INTO external_import_ledger (
                importer_id,
                entity_key,
                provider,
                handle,
                source_id,
                account_id,
                imported_at,
                updated_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT(importer_id, entity_key) DO UPDATE SET
               provider = excluded.provider,
               handle = excluded.handle,
               source_id = excluded.source_id,
               account_id = excluded.account_id,
               imported_at = excluded.imported_at,
               updated_at = excluded.updated_at",
            params![
                importer_id,
                normalize_import_entity_key(profile_root),
                provider,
                handle,
                source_id,
                account_id,
                timestamp,
            ],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "UPDATE source_profiles
             SET importer_id = ?1,
                 imported_at = ?2
             WHERE id = ?3",
            params![importer_id, timestamp, source_id],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn ensure_external_import_ledger_table(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS external_import_ledger (
                importer_id TEXT NOT NULL,
                entity_key TEXT NOT NULL,
                provider TEXT NOT NULL,
                handle TEXT NOT NULL,
                source_id TEXT,
                account_id TEXT,
                imported_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (importer_id, entity_key)
            );",
        )
        .map_err(|error| error.to_string())
}

fn upsert_source_profile_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    mut input: SourceProfileUpsert,
) -> Result<WorkspaceSnapshot, String> {
    let id = input.id.clone().unwrap_or_else(new_id);
    let now = now_timestamp();
    // user id e handles anteriores são metadados internos (resolvem renames e
    // alimentam a busca por nome antigo) e a UI não os reenviar em todo upsert.
    // Preserva os valores persistidos quando o payload não os traz, evitando que
    // edições/sync os apaguem do perfil.
    preserve_persisted_instagram_metadata(connection, &id, &mut input);
    preserve_persisted_twitter_metadata(connection, &id, &mut input);
    preserve_persisted_tiktok_metadata(connection, &id, &mut input);
    // Handle anterior (se o perfil já existe) para registrar uma troca MANUAL
    // no histórico, espelhando o que o auto-update (via user id) já registra.
    let existing_source_state = connection
        .query_row(
            "SELECT handle, account_id FROM source_profiles WHERE id = ?1 AND deleted_at IS NULL",
            params![id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;

    // A troca manual é suportada por todos os providers. Para Instagram,
    // preserve também o nome anterior usado pela busca e pela auditoria de
    // identidade, igual ao caminho de rename automático.
    if input.provider.eq_ignore_ascii_case("instagram") {
        if let Some((old_handle, _)) = existing_source_state.as_ref() {
            let old_normalized = sanitize_source_handle("instagram", old_handle);
            let new_normalized = sanitize_source_handle("instagram", &input.handle);
            if !old_normalized.is_empty()
                && !new_normalized.is_empty()
                && !old_normalized.eq_ignore_ascii_case(&new_normalized)
            {
                let instagram = input
                    .sync_options
                    .instagram
                    .get_or_insert_with(default_instagram_source_sync_options);
                instagram.previous_handles = push_previous_instagram_handle(
                    instagram.previous_handles.take(),
                    old_handle,
                    &new_normalized,
                );
            }
        }
    }

    let labels = to_json_array(&input.labels)?;
    let sync_options = serialize_source_sync_options(&input.provider, &input.sync_options)?;
    let account_id = validate_explicit_source_account_binding(
        connection,
        &input.provider,
        input.account_id.as_deref(),
    )?;

    if let Some(existing_handle) =
        find_conflicting_source_handle(connection, &input.provider, &input.handle, &id)?
    {
        return Err(format!(
            "O perfil \"{}\" já existe nesta lista (equivalente a \"{}\"). Abra o perfil existente em vez de criar outro.",
            input.handle.trim(),
            existing_handle
        ));
    }

    connection
        .execute(
            "INSERT INTO source_profiles (
                id,
                provider,
                source_kind,
                handle,
                display_name,
                account_id,
                 group_id,
                labels_json,
                ready_for_download,
                sync_options_json,
                remote_state,
                is_subscription,
                created_at,
                updated_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)
             ON CONFLICT(id) DO UPDATE SET
               provider = excluded.provider,
               source_kind = excluded.source_kind,
               handle = excluded.handle,
               display_name = excluded.display_name,
               account_id = excluded.account_id,
                group_id = excluded.group_id,
               labels_json = excluded.labels_json,
               ready_for_download = excluded.ready_for_download,
               sync_options_json = excluded.sync_options_json,
               remote_state = excluded.remote_state,
               is_subscription = excluded.is_subscription,
               deleted_at = NULL,
               updated_at = excluded.updated_at",
            params![
                id,
                input.provider,
                input.source_kind,
                input.handle,
                input.display_name,
                account_id,
                input.group_id,
                labels,
                bool_to_int(input.ready_for_download),
                sync_options,
                input.remote_state.unwrap_or_else(|| "exists".to_string()),
                bool_to_int(input.is_subscription.unwrap_or(false)),
                now
            ],
        )
        .map_err(|error| error.to_string())?;

    // Se um perfil existente teve o handle alterado (edição manual), registra no
    // histórico para ficar rastreável, como o auto-update faz.
    if let Some((old_handle, existing_account_id)) = existing_source_state {
        let old_norm = sanitize_source_handle(&input.provider, &old_handle);
        let new_norm = sanitize_source_handle(&input.provider, &input.handle);
        if !old_norm.trim().is_empty()
            && !new_norm.trim().is_empty()
            && !old_norm.eq_ignore_ascii_case(&new_norm)
        {
            if let Some(run_account_id) = existing_account_id
                .or_else(|| input.account_id.clone())
                .filter(|value| !value.trim().is_empty())
            {
                record_manual_handle_change_run(
                    connection,
                    &id,
                    &run_account_id,
                    &input.provider,
                    old_norm.trim_start_matches('@'),
                    new_norm.trim_start_matches('@'),
                    &now,
                )?;
            }
        }
    }

    load_snapshot(connection, layout)
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

fn validate_explicit_source_account_binding(
    connection: &Connection,
    source_provider: &str,
    account_id: Option<&str>,
) -> Result<String, String> {
    let account_id = account_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Sources must bind to an explicit provider account.".to_string())?;

    let account_provider = connection
        .query_row(
            "SELECT provider FROM provider_accounts WHERE id = ?1 LIMIT 1",
            params![account_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;

    let Some(account_provider) = account_provider else {
        return Err(format!(
            "Provider account '{}' does not exist for source binding.",
            account_id
        ));
    };

    if account_provider != source_provider {
        return Err(format!(
            "Source provider '{}' cannot bind to provider account '{}' owned by provider '{}'.",
            source_provider, account_id, account_provider
        ));
    }

    Ok(account_id.to_string())
}

fn run_source_sync_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    source_id: String,
    trigger: &str,
    run_mode: Option<&str>,
    sync_options_override: Option<&SourceSyncOptions>,
    executor: &dyn ToolExecutor,
) -> Result<WorkspaceSnapshot, String> {
    execute_source_sync_with_connection(
        connection,
        layout,
        source_id,
        trigger,
        run_mode,
        sync_options_override,
        executor,
    )?;
    load_snapshot(connection, layout)
}

fn execute_source_sync_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    source_id: String,
    trigger: &str,
    run_mode: Option<&str>,
    sync_options_override: Option<&SourceSyncOptions>,
    executor: &dyn ToolExecutor,
) -> Result<SourceSyncOutcome, String> {
    let context = load_source_sync_context(connection, layout, &source_id)?;
    let _connector_debug_context = connector_debug::enter(
        context.source.id.clone(),
        context.source.provider.clone(),
        context.source.handle.clone(),
    );
    connector_debug::append_current(
        "backend",
        "system",
        "sync.begin",
        format!(
            "source_id={}\nprovider={}\nhandle={}\ntrigger={trigger}\nrun_mode={}",
            context.source.id,
            context.source.provider,
            context.source.handle,
            run_mode.unwrap_or("default")
        ),
    );
    validate_source_sync_override(&context.source, sync_options_override)?;
    if context.source.provider.eq_ignore_ascii_case("instagram") {
        let account_settings = load_provider_account_settings_map(connection, &context.account.id)?;
        return execute_instagram_source_sync_with_connection(
            connection,
            layout,
            &context,
            &account_settings,
            trigger,
            run_mode,
            sync_options_override,
        );
    }
    if context.source.provider.eq_ignore_ascii_case("twitter") {
        let account_settings = load_provider_account_settings_map(connection, &context.account.id)?;
        return execute_twitter_source_sync_with_connection(
            connection,
            layout,
            &context,
            &account_settings,
            trigger,
            sync_options_override,
        );
    }
    if context.source.provider.eq_ignore_ascii_case("tiktok") {
        let account_settings = load_provider_account_settings_map(connection, &context.account.id)?;
        return execute_tiktok_source_sync_with_connection(
            connection,
            layout,
            &context,
            &account_settings,
            trigger,
            sync_options_override,
        );
    }
    let app_settings = load_app_settings_map(connection)?;

    let invocation =
        build_source_sync_invocation(connection, &context, layout, sync_options_override)?;
    let started_at = now_timestamp();
    let execution = executor.execute(&invocation);
    clear_source_sync_cancel_token(&context.source.id);
    let finished_at = now_timestamp();
    let degraded_capabilities =
        connector_degraded_capabilities(&context.source.provider, &context.account.capabilities);

    let outcome = match execution {
        Ok(result) => {
            let validation_error = if degraded_capabilities.is_empty() {
                None
            } else {
                Some(format!(
                    "Connector runtime degraded capabilities: {}",
                    degraded_capabilities.join(", ")
                ))
            };

            let ingested_media_count = catalog_source_media_output(
                connection,
                &context,
                &invocation.output_root,
                &finished_at,
            )?;

            if !context.source.profile_image_custom {
                let provider_avatar = match refresh_profile_picture_from_provider(
                    connection,
                    layout,
                    &context,
                    &invocation.output_root,
                    &app_settings,
                ) {
                    Ok(path) => path,
                    Err(error) => {
                        let message = match error.level {
                            ProfilePictureRefreshLogLevel::Info => format!(
                                "Profile picture refresh skipped for '{}': {}",
                                context.source.handle, error.message
                            ),
                            ProfilePictureRefreshLogLevel::Warning => format!(
                                "Failed to refresh profile picture for '{}': {}",
                                context.source.handle, error.message
                            ),
                        };
                        log_runtime_event(
                            layout,
                            "sync.avatar",
                            error.level.as_str(),
                            Some(&context.account.id),
                            Some(&context.source.provider),
                            Some(&context.source.id),
                            Some(&context.source.handle),
                            message,
                            error.detail,
                        );
                        None
                    }
                };

                let resolved_avatar =
                    provider_avatar.or_else(|| find_source_avatar(&invocation.output_root));
                if let Some(avatar_path) = resolved_avatar {
                    let _ = update_source_profile_image(
                        connection,
                        &context.source.id,
                        &avatar_path,
                        &finished_at,
                    );
                }
            }

            SourceSyncOutcome {
                tool: invocation.executable.clone(),
                status: result.status,
                summary: format_connector_sync_success_summary(
                    ingested_media_count,
                    &degraded_capabilities,
                ),
                command_preview: invocation.command_preview.clone(),
                manifest_summary_json: None,
                degraded_capabilities,
                validation_error,
            }
        }
        Err(error) => {
            let cancelled_by_user = error
                .trim()
                .to_ascii_lowercase()
                .contains("cancelled by user");

            SourceSyncOutcome {
                tool: invocation.executable.clone(),
                status: "failed".to_string(),
                summary: if cancelled_by_user {
                    "Connector sync cancelled by user.".to_string()
                } else {
                    format!("Connector sync failed: {}", error)
                },
                command_preview: invocation.command_preview.clone(),
                manifest_summary_json: None,
                degraded_capabilities: Vec::new(),
                validation_error: if cancelled_by_user { None } else { Some(error) },
            }
        }
    };

    persist_source_sync_run(
        connection,
        &context,
        &outcome,
        trigger,
        &started_at,
        &finished_at,
    )?;
    propagate_source_sync_account_health(connection, &context, &outcome, &finished_at)?;
    Ok(outcome)
}

/// Executa o sync interno do X/Twitter, espelhando o contrato do SCrawler: o
/// gallery-dl parseia a timeline e o NinjaCrawler baixa e cataloga a mídia,
/// persistindo posts/mídia nos ledgers provider-neutral.
fn execute_twitter_source_sync_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    context: &SourceSyncContext,
    settings: &HashMap<String, String>,
    trigger: &str,
    sync_options_override: Option<&SourceSyncOptions>,
) -> Result<SourceSyncOutcome, String> {
    let options = source_twitter_sync_options_with_override(&context.source, sync_options_override);
    let started_at = now_timestamp();

    let handle = sanitize_source_handle("twitter", &context.source.handle)
        .trim_start_matches('@')
        .to_string();
    if handle.is_empty() {
        return Err("Twitter source handle is empty.".to_string());
    }

    let profile_root =
        resolved_source_media_output_root_with_connection(connection, layout, &context.source)?;
    fs::create_dir_all(&profile_root).map_err(|error| error.to_string())?;
    let cache_root = layout
        .cache_root
        .join(format!("twitter-sync-{}", context.source.id));

    let parsed_session = parse_session_payload(&context.session_payload)?;
    let cookies = parsed_session.cookies;
    let cookie_file = cache_root.join("cookies.txt");
    fs::create_dir_all(&cache_root).map_err(|error| error.to_string())?;
    write_netscape_cookie_file(&cookie_file, &cookies)?;
    let use_user_agent = settings
        .get("twitter.auth.useUserAgent")
        .map(|value| value.trim().eq_ignore_ascii_case("true"))
        .unwrap_or(true);
    let user_agent = if use_user_agent {
        settings
            .get("twitter.auth.userAgent")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| parsed_session.metadata.user_agent.clone())
    } else {
        None
    };
    // UA usado para baixar o avatar (request consome `user_agent` por move).
    let avatar_user_agent = user_agent
        .clone()
        .unwrap_or_else(|| "Mozilla/5.0".to_string());

    let executable =
        connector_runtime::resolve_connector_executable(connection, layout, "gallery-dl")?;

    let ledger_post_keys =
        load_provider_sync_post_ledger_keys(connection, "twitter", &context.source.id)?;
    let ledger_media_keys =
        load_provider_sync_media_ledger_keys(connection, "twitter", &context.source.id)?;
    let existing_relative_paths = load_existing_relative_media_paths(&profile_root);

    let request = twitter_connector::TwitterConnectorRequest {
        handle: handle.clone(),
        gallery_dl_executable: PathBuf::from(&executable),
        cookie_file,
        user_agent,
        profile_root: profile_root.clone(),
        cache_root,
        models: twitter_connector::TwitterModelSelection {
            media: options.media_model.unwrap_or(true),
            profile: options.profile_model.unwrap_or(true),
            search: options.search_model.unwrap_or(false),
            likes: options.likes_model.unwrap_or(false),
        },
        ledger_post_keys,
        ledger_media_keys,
        existing_relative_paths,
        user_id_hint: options
            .user_id_hint
            .clone()
            .filter(|value| !value.trim().is_empty()),
        abort_on_limit: options.abort_on_limit.unwrap_or(true),
        download_already_parsed: options.download_already_parsed.unwrap_or(true),
        sleep_timer_secs: options.sleep_timer_secs.unwrap_or(-1),
        sleep_timer_before_first_secs: options.sleep_timer_before_first_secs.unwrap_or(-2),
        download_images: options.download_images.unwrap_or(true),
        download_videos: options.download_videos.unwrap_or(true),
        download_gifs: options.download_gifs.unwrap_or(true),
        separate_video_folder: options.separate_video_folder.unwrap_or(true),
        gifs_special_folder: options.gifs_special_folder.clone().unwrap_or_default(),
        gifs_prefix: options
            .gifs_prefix
            .clone()
            .unwrap_or_else(|| "GIF_".to_string()),
        allow_non_user_tweets: options.allow_non_user_tweets.unwrap_or(false),
        use_md5_comparison: options.use_md5_comparison.unwrap_or(false),
        search_use_graphql_endpoint: options.search_use_graphql_endpoint.unwrap_or(true),
        profile_use_graphql_endpoint: options.profile_use_graphql_endpoint.unwrap_or(true),
    };

    let cancel_token = register_source_sync_cancel_token(&context.source.id);
    if cancel_token.load(Ordering::SeqCst) {
        clear_source_sync_cancel_token(&context.source.id);
        return Err("source sync cancelled by user".to_string());
    }

    source_sync_runtime::report_source_sync_progress(
        &context.source.id,
        Some(0),
        Some("Starting download".to_string()),
        Some("Twitter connector is preparing source sync.".to_string()),
        true,
        Some(0),
    );

    // Primeiro sync: o connector resolve o user id das páginas e consulta este
    // closure antes de baixar. Se o id já pertence a outro perfil, cancela.
    let is_first_sync = context.source.last_synced_at.is_none();
    let dup_source_id = context.source.id.clone();
    let execution = twitter_connector::run_profile_sync(
        &request,
        |progress| {
            source_sync_runtime::report_source_sync_progress(
                &context.source.id,
                progress.progress_percent,
                Some(progress.label),
                Some(progress.detail),
                progress.indeterminate,
                progress.downloaded_items,
            );
        },
        || cancel_token.load(Ordering::SeqCst),
        |user_id| {
            is_first_sync
                && find_source_with_same_user_id(connection, "twitter", user_id, &dup_source_id)
                    .ok()
                    .flatten()
                    .is_some()
        },
    );
    clear_source_sync_cancel_token(&context.source.id);
    let finished_at = now_timestamp();

    let command_preview = format!(
        "internal.twitter profile {} -> {}",
        handle,
        profile_root.display()
    );

    let outcome = match execution {
        Ok(result) => {
            // Duplicado detectado no primeiro sync: remove o perfil recém-
            // adicionado, informa e cancela (nada foi baixado).
            if let Some(user_id) = result.duplicate_user_id.as_deref() {
                if let Some(dup_outcome) = detect_duplicate_user_id_on_first_sync(
                    connection,
                    layout,
                    context,
                    user_id,
                    "internal.twitter",
                    command_preview.clone(),
                ) {
                    persist_source_sync_run(
                        connection,
                        context,
                        &dup_outcome,
                        trigger,
                        &started_at,
                        &finished_at,
                    )?;
                    source_sync_runtime::report_source_sync_progress(
                        &context.source.id,
                        Some(100),
                        Some("Download cancelled".to_string()),
                        Some(dup_outcome.summary.clone()),
                        false,
                        None,
                    );
                    return Ok(dup_outcome);
                }
            }

            // Renomeação de conta: nenhum tweet veio sob o handle salvo, mas
            // `x.com/i/user/<userIdHint>` resolveu para outro screen_name.
            // Atualiza o perfil; o próximo sync baixa as mídias sob o novo handle.
            if let Some(new_handle) = result.resolved_handle.as_deref() {
                let new_handle = sanitize_source_handle("twitter", new_handle)
                    .trim_start_matches('@')
                    .to_string();
                if !new_handle.is_empty() && !handle.eq_ignore_ascii_case(&new_handle) {
                    let summary = match update_twitter_source_handle_after_sync(
                        connection,
                        &context.source.id,
                        &new_handle,
                        &finished_at,
                    ) {
                        Ok(()) => {
                            log_runtime_event(
                                layout,
                                "sync.profile",
                                "info",
                                Some(&context.account.id),
                                Some(&context.source.provider),
                                Some(&context.source.id),
                                Some(&context.source.handle),
                                format!(
                                    "Twitter handle changed from '@{handle}' to '@{new_handle}'. Source handle updated automatically."
                                ),
                                None,
                            );
                            format!(
                                "Twitter handle changed: @{handle} → @{new_handle}. Profile updated; run the sync again to download media under the new handle."
                            )
                        }
                        Err(error) => {
                            log_runtime_event(
                                layout,
                                "sync.profile",
                                "warning",
                                Some(&context.account.id),
                                Some(&context.source.provider),
                                Some(&context.source.id),
                                Some(&context.source.handle),
                                format!(
                                    "Twitter handle change detected (@{handle} → @{new_handle}) but updating the source failed: {error}"
                                ),
                                Some(error),
                            );
                            format!(
                                "Twitter handle change detected (@{handle} → @{new_handle}) but the source update failed."
                            )
                        }
                    };
                    let outcome = SourceSyncOutcome {
                        tool: "internal.twitter".to_string(),
                        status: "succeeded".to_string(),
                        summary,
                        command_preview: command_preview.clone(),
                        manifest_summary_json: None,
                        degraded_capabilities: Vec::new(),
                        validation_error: None,
                    };
                    persist_source_sync_run(
                        connection,
                        context,
                        &outcome,
                        trigger,
                        &started_at,
                        &finished_at,
                    )?;
                    propagate_source_sync_account_health(connection, context, &outcome, &finished_at)?;
                    source_sync_runtime::report_source_sync_progress(
                        &context.source.id,
                        Some(100),
                        Some("Handle updated".to_string()),
                        Some(outcome.summary.clone()),
                        false,
                        None,
                    );
                    return Ok(outcome);
                }
            }

            upsert_provider_sync_post_ledger_entries(
                connection,
                "twitter",
                &context.source.id,
                &context.account.id,
                &handle,
                &result.observed_posts,
                &finished_at,
            )?;
            upsert_provider_sync_media_ledger_entries(
                connection,
                "twitter",
                &context.source.id,
                &context.account.id,
                &handle,
                &profile_root,
                &result.downloaded_media,
                &finished_at,
            )?;
            // Preenche o post key na mídia já no disco (baixada antes de o key ser
            // gravado): casa pelo provider_media_key, só onde está vazio.
            backfill_provider_sync_media_ledger_post_keys(
                connection,
                "twitter",
                &context.source.id,
                &result.media_post_links,
                &finished_at,
            )?;

            // Persiste o user id resolvido para detectar renames/duplicatas depois.
            if let Some(user_id) = result.resolved_user_id.as_deref() {
                let _ = persist_twitter_user_id_hint(
                    connection,
                    &context.source.id,
                    user_id,
                    &finished_at,
                );
            }

            // Avatar: baixa/atualiza a foto de perfil quando o usuário não
            // definiu uma imagem custom. Usa a URL resolvida das páginas; se
            // falhar (ou não houver URL), recorre a um avatar já presente na
            // pasta, incluindo o UserPicture.jpg dos perfis importados.
            if !context.source.profile_image_custom {
                let provider_avatar = result.resolved_avatar_url.as_deref().and_then(|url| {
                    match refresh_twitter_profile_picture(&profile_root, url, &avatar_user_agent) {
                        Ok(path) => path,
                        Err(error) => {
                            log_runtime_event(
                                layout,
                                "sync.avatar",
                                error.level.as_str(),
                                Some(&context.account.id),
                                Some(&context.source.provider),
                                Some(&context.source.id),
                                Some(&context.source.handle),
                                format!(
                                    "Failed to refresh Twitter profile picture for '{}': {}",
                                    context.source.handle, error.message
                                ),
                                error.detail,
                            );
                            None
                        }
                    }
                });
                let resolved_avatar =
                    provider_avatar.or_else(|| find_source_avatar(&profile_root));
                if let Some(avatar_path) = resolved_avatar {
                    let _ = update_source_profile_image(
                        connection,
                        &context.source.id,
                        &avatar_path,
                        &finished_at,
                    );
                }
            }

            let downloaded = result.downloaded_media.len();
            let mut summary = format!(
                "Twitter sync succeeded. Downloaded {} media items. Manifest parsed {} pages and queued {} assets.",
                downloaded,
                result.manifest_summary.parsed_page_count,
                result.manifest_summary.queued_asset_count
            );
            if result.limit_aborted {
                summary.push_str(" Rate limit reached; remaining models were skipped.");
            }
            if !result.section_errors.is_empty() {
                summary.push_str(" Warnings: ");
                summary.push_str(&result.section_errors.join(" | "));
            }

            SourceSyncOutcome {
                tool: "internal.twitter".to_string(),
                status: "succeeded".to_string(),
                summary,
                command_preview,
                manifest_summary_json: None,
                degraded_capabilities: Vec::new(),
                validation_error: None,
            }
        }
        Err(error) => {
            let cancelled_by_user = error.trim().to_ascii_lowercase().contains("cancelled by user");
            SourceSyncOutcome {
                tool: "internal.twitter".to_string(),
                status: if cancelled_by_user {
                    "skipped".to_string()
                } else {
                    "failed".to_string()
                },
                summary: if cancelled_by_user {
                    "Twitter sync cancelled by user.".to_string()
                } else {
                    format!("Twitter sync failed: {}", error)
                },
                command_preview,
                manifest_summary_json: None,
                degraded_capabilities: Vec::new(),
                validation_error: if cancelled_by_user { None } else { Some(error) },
            }
        }
    };

    persist_source_sync_run(
        connection,
        context,
        &outcome,
        trigger,
        &started_at,
        &finished_at,
    )?;
    propagate_source_sync_account_health(connection, context, &outcome, &finished_at)?;
    source_sync_runtime::report_source_sync_progress(
        &context.source.id,
        Some(100),
        Some(if outcome.status == "succeeded" {
            "Download complete".to_string()
        } else if outcome.status == "skipped" {
            "Download skipped".to_string()
        } else {
            "Download failed".to_string()
        }),
        Some(outcome.summary.clone()),
        false,
        None,
    );
    Ok(outcome)
}

fn source_twitter_sync_options_with_override(
    source: &SourceProfile,
    sync_options_override: Option<&SourceSyncOptions>,
) -> TwitterSourceSyncOptions {
    if let Some(override_options) = sync_options_override.and_then(|options| options.twitter.clone())
    {
        return normalize_twitter_source_sync_options(Some(override_options));
    }
    source_twitter_sync_options(source)
}

fn source_tiktok_sync_options_with_override(
    source: &SourceProfile,
    sync_options_override: Option<&SourceSyncOptions>,
) -> TikTokSourceSyncOptions {
    if let Some(override_options) = sync_options_override.and_then(|options| options.tiktok.clone()) {
        return normalize_tiktok_source_sync_options(Some(override_options));
    }
    source_tiktok_sync_options(source)
}

/// Sync interno do TikTok: yt-dlp baixa os vídeos da timeline e o gallery-dl
/// parseia os posts de fotos (slideshow), persistindo nos ledgers
/// provider-neutral. Espelha o branch do Twitter.
fn execute_tiktok_source_sync_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    context: &SourceSyncContext,
    settings: &HashMap<String, String>,
    trigger: &str,
    sync_options_override: Option<&SourceSyncOptions>,
) -> Result<SourceSyncOutcome, String> {
    let options = source_tiktok_sync_options_with_override(&context.source, sync_options_override);
    let started_at = now_timestamp();

    let handle = sanitize_source_handle("tiktok", &context.source.handle)
        .trim_start_matches('@')
        .to_string();
    if handle.is_empty() {
        return Err("TikTok source handle is empty.".to_string());
    }

    let profile_root =
        resolved_source_media_output_root_with_connection(connection, layout, &context.source)?;
    fs::create_dir_all(&profile_root).map_err(|error| error.to_string())?;
    let cache_root = layout
        .cache_root
        .join(format!("tiktok-sync-{}", context.source.id));
    fs::create_dir_all(&cache_root).map_err(|error| error.to_string())?;

    let parsed_session = parse_session_payload(&context.session_payload)?;
    let cookies = parsed_session.cookies;
    let cookie_file = cache_root.join("cookies.txt");
    write_netscape_cookie_file(&cookie_file, &cookies)?;
    let use_user_agent = settings
        .get("tiktok.auth.useUserAgent")
        .map(|value| value.trim().eq_ignore_ascii_case("true"))
        .unwrap_or(true);
    let user_agent = if use_user_agent {
        settings
            .get("tiktok.auth.userAgent")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| parsed_session.metadata.user_agent.clone())
    } else {
        None
    };
    let avatar_user_agent = user_agent
        .clone()
        .unwrap_or_else(|| "Mozilla/5.0".to_string());

    let yt_dlp_executable =
        connector_runtime::resolve_connector_executable(connection, layout, "yt-dlp")?;
    // Usado só para os Stories (extractor `/stories` do gallery-dl).
    let gallery_dl_executable =
        connector_runtime::resolve_connector_executable(connection, layout, "gallery-dl")?;

    let ledger_post_keys =
        load_provider_sync_post_ledger_keys(connection, "tiktok", &context.source.id)?;
    let ledger_media_keys =
        load_provider_sync_media_ledger_keys(connection, "tiktok", &context.source.id)?;
    let existing_relative_paths = load_existing_relative_media_paths(&profile_root);

    let request = tiktok_connector::TikTokConnectorRequest {
        handle: handle.clone(),
        yt_dlp_executable: PathBuf::from(&yt_dlp_executable),
        gallery_dl_executable: PathBuf::from(&gallery_dl_executable),
        cookie_file,
        user_agent,
        profile_root: profile_root.clone(),
        cache_root,
        sections: tiktok_connector::TikTokSectionSelection {
            timeline: options.get_timeline.unwrap_or(true),
            stories: options.get_stories_user.unwrap_or(false),
            reposts: options.get_reposts.unwrap_or(false),
        },
        target_video_url: options
            .target_video_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        download_videos: options.download_videos.unwrap_or(true),
        download_photos: options.download_photos.unwrap_or(true),
        separate_video_folder: options.separate_video_folder.unwrap_or(false),
        use_parsed_video_date: options.use_parsed_video_date.unwrap_or(true),
        use_native_title: options.use_native_title.unwrap_or(false),
        add_video_id_to_title: options.add_video_id_to_title.unwrap_or(true),
        remove_tags_from_title: options.remove_tags_from_title.unwrap_or(false),
        tokkit_naming: options.tokkit_file_naming.unwrap_or(false),
        download_from_date: options.download_from_date.filter(|value| *value > 0),
        download_to_date: options.download_to_date.filter(|value| *value > 0),
        abort_on_limit: options.abort_on_limit.unwrap_or(true),
        sleep_timer_secs: options.sleep_timer_secs.unwrap_or(-1),
        ledger_post_keys,
        ledger_media_keys,
        existing_relative_paths,
        user_id_hint: options
            .user_id_hint
            .clone()
            .filter(|value| !value.trim().is_empty()),
    };

    let cancel_token = register_source_sync_cancel_token(&context.source.id);
    if cancel_token.load(Ordering::SeqCst) {
        clear_source_sync_cancel_token(&context.source.id);
        return Err("source sync cancelled by user".to_string());
    }

    source_sync_runtime::report_source_sync_progress(
        &context.source.id,
        Some(0),
        Some("Starting download".to_string()),
        Some("TikTok connector is preparing source sync.".to_string()),
        true,
        Some(0),
    );

    let is_first_sync = context.source.last_synced_at.is_none();
    let dup_source_id = context.source.id.clone();
    let execution = tiktok_connector::run_profile_sync(
        &request,
        |progress| {
            source_sync_runtime::report_source_sync_progress(
                &context.source.id,
                progress.progress_percent,
                Some(progress.label),
                Some(progress.detail),
                progress.indeterminate,
                progress.downloaded_items,
            );
        },
        || cancel_token.load(Ordering::SeqCst),
        |user_id| {
            is_first_sync
                && find_source_with_same_user_id(connection, "tiktok", user_id, &dup_source_id)
                    .ok()
                    .flatten()
                    .is_some()
        },
    );
    clear_source_sync_cancel_token(&context.source.id);
    let finished_at = now_timestamp();

    let command_preview = format!("internal.tiktok profile {} -> {}", handle, profile_root.display());

    let outcome = match execution {
        Ok(result) => {
            if let Some(user_id) = result.duplicate_user_id.as_deref() {
                if let Some(dup_outcome) = detect_duplicate_user_id_on_first_sync(
                    connection,
                    layout,
                    context,
                    user_id,
                    "internal.tiktok",
                    command_preview.clone(),
                ) {
                    persist_source_sync_run(
                        connection,
                        context,
                        &dup_outcome,
                        trigger,
                        &started_at,
                        &finished_at,
                    )?;
                    source_sync_runtime::report_source_sync_progress(
                        &context.source.id,
                        Some(100),
                        Some("Download cancelled".to_string()),
                        Some(dup_outcome.summary.clone()),
                        false,
                        None,
                    );
                    return Ok(dup_outcome);
                }
            }

            // Renomeação de conta: o connector descobriu o handle atual a partir
            // de um post conhecido. Atualiza o perfil e encerra; o próximo sync
            // baixa as mídias sob o novo handle (todas as rotas usam o handle).
            if let Some(new_handle) = result.resolved_handle.as_deref() {
                let new_handle = sanitize_source_handle("tiktok", new_handle)
                    .trim_start_matches('@')
                    .to_string();
                if !new_handle.is_empty() && !handle.eq_ignore_ascii_case(&new_handle) {
                    let summary = match update_tiktok_source_handle_after_sync(
                        connection,
                        &context.source.id,
                        &new_handle,
                        &finished_at,
                    ) {
                        Ok(()) => {
                            log_runtime_event(
                                layout,
                                "sync.profile",
                                "info",
                                Some(&context.account.id),
                                Some(&context.source.provider),
                                Some(&context.source.id),
                                Some(&context.source.handle),
                                format!(
                                    "TikTok handle changed from '@{handle}' to '@{new_handle}'. Source handle updated automatically."
                                ),
                                None,
                            );
                            format!(
                                "TikTok handle changed: @{handle} → @{new_handle}. Profile updated; run the sync again to download media under the new handle."
                            )
                        }
                        Err(error) => {
                            log_runtime_event(
                                layout,
                                "sync.profile",
                                "warning",
                                Some(&context.account.id),
                                Some(&context.source.provider),
                                Some(&context.source.id),
                                Some(&context.source.handle),
                                format!(
                                    "TikTok handle change detected (@{handle} → @{new_handle}) but updating the source failed: {error}"
                                ),
                                Some(error),
                            );
                            format!(
                                "TikTok handle change detected (@{handle} → @{new_handle}) but the source update failed."
                            )
                        }
                    };
                    let outcome = SourceSyncOutcome {
                        tool: "internal.tiktok".to_string(),
                        status: "succeeded".to_string(),
                        summary,
                        command_preview: command_preview.clone(),
                        manifest_summary_json: None,
                        degraded_capabilities: Vec::new(),
                        validation_error: None,
                    };
                    persist_source_sync_run(
                        connection,
                        context,
                        &outcome,
                        trigger,
                        &started_at,
                        &finished_at,
                    )?;
                    propagate_source_sync_account_health(connection, context, &outcome, &finished_at)?;
                    source_sync_runtime::report_source_sync_progress(
                        &context.source.id,
                        Some(100),
                        Some("Handle updated".to_string()),
                        Some(outcome.summary.clone()),
                        false,
                        None,
                    );
                    return Ok(outcome);
                }
            }

            // Os ledgers são provider-neutral no banco; reusamos os structs do
            // connector do Twitter (mesmos campos) só para a inserção.
            let observed_posts: Vec<twitter_connector::ObservedTwitterPost> = result
                .observed_posts
                .iter()
                .map(|post| twitter_connector::ObservedTwitterPost {
                    provider_post_key: post.provider_post_key.clone(),
                    media_section: post.media_section.clone(),
                })
                .collect();
            let downloaded_media: Vec<twitter_connector::DownloadedTwitterMedia> = result
                .downloaded_media
                .iter()
                .map(|media| twitter_connector::DownloadedTwitterMedia {
                    file_path: media.file_path.clone(),
                    media_type: media.media_type.clone(),
                    media_section: media.media_section.clone(),
                    provider_media_key: media.provider_media_key.clone(),
                    provider_post_key: media.provider_post_key.clone(),
                    captured_at_timestamp: media.captured_at_timestamp,
                    final_file_name: media.final_file_name.clone(),
                })
                .collect();
            upsert_provider_sync_post_ledger_entries(
                connection,
                "tiktok",
                &context.source.id,
                &context.account.id,
                &handle,
                &observed_posts,
                &finished_at,
            )?;
            upsert_provider_sync_media_ledger_entries(
                connection,
                "tiktok",
                &context.source.id,
                &context.account.id,
                &handle,
                &profile_root,
                &downloaded_media,
                &finished_at,
            )?;

            if let Some(user_id) = result.resolved_user_id.as_deref() {
                let _ =
                    persist_tiktok_user_id_hint(connection, &context.source.id, user_id, &finished_at);
            }

            if !context.source.profile_image_custom {
                let provider_avatar = result.resolved_avatar_url.as_deref().and_then(|url| {
                    match refresh_twitter_profile_picture(&profile_root, url, &avatar_user_agent) {
                        Ok(path) => path,
                        Err(error) => {
                            log_runtime_event(
                                layout,
                                "sync.avatar",
                                error.level.as_str(),
                                Some(&context.account.id),
                                Some(&context.source.provider),
                                Some(&context.source.id),
                                Some(&context.source.handle),
                                format!(
                                    "Failed to refresh TikTok profile picture for '{}': {}",
                                    context.source.handle, error.message
                                ),
                                error.detail,
                            );
                            None
                        }
                    }
                });
                let resolved_avatar = provider_avatar.or_else(|| find_source_avatar(&profile_root));
                if let Some(avatar_path) = resolved_avatar {
                    let _ = update_source_profile_image(
                        connection,
                        &context.source.id,
                        &avatar_path,
                        &finished_at,
                    );
                }
            }

            let downloaded = result.downloaded_media.len();
            let mut summary = format!(
                "TikTok sync succeeded. Downloaded {} media item(s) from {} new post(s) (queued {}).",
                downloaded,
                result.observed_posts.len(),
                result.manifest_summary.queued_asset_count
            );
            if result.limit_aborted {
                summary.push_str(" Rate limit reached; remaining posts were skipped.");
            }
            if !result.section_errors.is_empty() {
                summary.push_str(" Warnings: ");
                summary.push_str(&result.section_errors.join(" | "));
            }

            SourceSyncOutcome {
                tool: "internal.tiktok".to_string(),
                status: "succeeded".to_string(),
                summary,
                command_preview,
                manifest_summary_json: None,
                degraded_capabilities: Vec::new(),
                validation_error: None,
            }
        }
        Err(error) => {
            let cancelled_by_user = error.trim().to_ascii_lowercase().contains("cancelled by user");
            SourceSyncOutcome {
                tool: "internal.tiktok".to_string(),
                status: if cancelled_by_user {
                    "skipped".to_string()
                } else {
                    "failed".to_string()
                },
                summary: if cancelled_by_user {
                    "TikTok sync cancelled by user.".to_string()
                } else {
                    format!("TikTok sync failed: {}", error)
                },
                command_preview,
                manifest_summary_json: None,
                degraded_capabilities: Vec::new(),
                validation_error: if cancelled_by_user { None } else { Some(error) },
            }
        }
    };

    persist_source_sync_run(connection, context, &outcome, trigger, &started_at, &finished_at)?;
    propagate_source_sync_account_health(connection, context, &outcome, &finished_at)?;
    source_sync_runtime::report_source_sync_progress(
        &context.source.id,
        Some(100),
        Some(if outcome.status == "succeeded" {
            "Download complete".to_string()
        } else if outcome.status == "skipped" {
            "Download skipped".to_string()
        } else {
            "Download failed".to_string()
        }),
        Some(outcome.summary.clone()),
        false,
        None,
    );
    Ok(outcome)
}

fn load_existing_relative_media_paths(profile_root: &Path) -> HashSet<String> {
    let mut paths = HashSet::new();
    let Ok(files) = collect_media_file_paths(profile_root) else {
        return paths;
    };
    for file in files {
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

fn load_provider_sync_media_ledger_keys(
    connection: &Connection,
    provider: &str,
    source_id: &str,
) -> Result<HashSet<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT provider_media_key FROM provider_sync_media_ledger
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

fn upsert_provider_sync_media_ledger_entries(
    connection: &Connection,
    provider: &str,
    source_id: &str,
    account_id: &str,
    source_handle: &str,
    profile_root: &Path,
    downloaded_media: &[twitter_connector::DownloadedTwitterMedia],
    timestamp: &str,
) -> Result<(), String> {
    for media in downloaded_media {
        let relative_path = normalize_instagram_relative_media_path(profile_root, &media.file_path);
        connection
            .execute(
                "INSERT INTO provider_sync_media_ledger (
                    provider, source_id, account_id, source_handle,
                    provider_media_key, media_type, media_section, relative_path,
                    provider_post_key, captured_at, first_seen_at, last_seen_at
                 )
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
                 ON CONFLICT(provider, source_id, provider_media_key, media_type)
                 DO UPDATE SET
                    account_id = excluded.account_id,
                    source_handle = excluded.source_handle,
                    media_section = excluded.media_section,
                    relative_path = excluded.relative_path,
                    provider_post_key = excluded.provider_post_key,
                    captured_at = excluded.captured_at,
                    last_seen_at = excluded.last_seen_at",
                params![
                    provider,
                    source_id,
                    account_id,
                    source_handle,
                    media.provider_media_key.to_ascii_lowercase(),
                    &media.media_type,
                    &media.media_section,
                    relative_path,
                    media.provider_post_key,
                    media.captured_at_timestamp,
                    timestamp,
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Fills `provider_post_key` (and `captured_at`) on media ledger rows that are
/// already on disk but lack the post key — paired by `provider_media_key` from
/// the freshly fetched timeline. UPDATE-only: never inserts and never overwrites
/// a key that is already set, so it is safe to run on every sync.
fn backfill_provider_sync_media_ledger_post_keys(
    connection: &Connection,
    provider: &str,
    source_id: &str,
    links: &[twitter_connector::TwitterMediaPostLink],
    timestamp: &str,
) -> Result<(), String> {
    for link in links {
        let media_key = link.provider_media_key.trim();
        let post_key = link.provider_post_key.trim();
        if media_key.is_empty() || post_key.is_empty() {
            continue;
        }
        connection
            .execute(
                "UPDATE provider_sync_media_ledger
                 SET provider_post_key = ?4,
                     captured_at = COALESCE(captured_at, ?5),
                     last_seen_at = ?6
                 WHERE provider = ?1
                   AND source_id = ?2
                   AND provider_media_key = ?3
                   AND (provider_post_key IS NULL OR provider_post_key = '')",
                params![
                    provider,
                    source_id,
                    media_key.to_ascii_lowercase(),
                    post_key,
                    link.captured_at_timestamp,
                    timestamp,
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

struct AccountSyncContext {
    account: ProviderAccount,
    settings: HashMap<String, String>,
    session_payload: String,
}

#[derive(Clone)]
struct ProviderAccountSessionRecord {
    account_id: String,
    auth_mode: String,
    session_format: String,
    fingerprint: String,
    secret_ref: String,
    expires_at: Option<String>,
    imported_at: String,
    last_validated_at: Option<String>,
    last_validation_error: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CapturedBrowserCookie {
    pub domain: String,
    pub name: String,
    pub value: String,
    pub path: String,
    pub expires_at: Option<String>,
    pub secure: bool,
    pub http_only: bool,
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

struct SourceSyncContext {
    source: SourceProfile,
    account: ProviderAccount,
    session_payload: String,
}

struct SourceSyncOutcome {
    tool: String,
    status: String,
    summary: String,
    command_preview: String,
    manifest_summary_json: Option<String>,
    degraded_capabilities: Vec<String>,
    validation_error: Option<String>,
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

fn format_instagram_manifest_suffix(
    manifest_summary: Option<&instagram_connector::InstagramManifestSummary>,
    include_in_summary: bool,
) -> String {
    if !include_in_summary {
        return String::new();
    }

    manifest_summary
        .map(|summary| {
            let mut filtered_reasons = Vec::new();
            if summary.skipped_existing_post_count > 0 {
                filtered_reasons.push(format!(
                    "{} existing posts",
                    summary.skipped_existing_post_count
                ));
            }
            if summary.skipped_duplicate_post_count > 0 {
                filtered_reasons.push(format!(
                    "{} duplicate posts",
                    summary.skipped_duplicate_post_count
                ));
            }
            if summary.skipped_unavailable_post_count > 0 {
                filtered_reasons.push(format!(
                    "{} posts without downloadable media",
                    summary.skipped_unavailable_post_count
                ));
            }
            if summary.skipped_existing_asset_count > 0 {
                filtered_reasons.push(format!(
                    "{} existing assets",
                    summary.skipped_existing_asset_count
                ));
            }
            if summary.skipped_duplicate_asset_count > 0 {
                filtered_reasons.push(format!(
                    "{} duplicate assets",
                    summary.skipped_duplicate_asset_count
                ));
            }

            if filtered_reasons.is_empty() {
                format!(
                    " Manifest retained {} posts and queued {} assets across {} sections.",
                    summary.normalized_post_count,
                    summary.queued_asset_count,
                    summary.section_count
                )
            } else {
                format!(
                    " Manifest retained {} posts and queued {} assets across {} sections after filtering {}.",
                    summary.normalized_post_count,
                    summary.queued_asset_count,
                    summary.section_count,
                    filtered_reasons.join(", ")
                )
            }
        })
        .unwrap_or_default()
}

fn upsert_provider_account_string_setting(
    connection: &Connection,
    account_id: &str,
    setting_key: &str,
    value: &str,
    now: &str,
) -> Result<(), String> {
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
             VALUES (?1, ?2, 'string', ?3, ?4, ?4)
             ON CONFLICT(account_id, setting_key)
             DO UPDATE SET
                value_kind = excluded.value_kind,
                value_text = excluded.value_text,
                updated_at = excluded.updated_at",
            params![account_id, setting_key, value, now],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn delete_provider_account_setting(
    connection: &Connection,
    account_id: &str,
    setting_key: &str,
) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM provider_account_settings
             WHERE account_id = ?1
               AND setting_key = ?2",
            params![account_id, setting_key],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn effective_instagram_sections_enabled(
    sections: &instagram_connector::InstagramSectionSelection,
) -> bool {
    sections.timeline
        || sections.reels
        || sections.stories
        || sections.stories_user
        || sections.tagged
}

fn instagram_request_has_base_auth(
    request: &instagram_connector::InstagramConnectorRequest,
) -> bool {
    let has_cookie = request
        .cookies
        .iter()
        .any(|cookie| !cookie.name.trim().is_empty() && !cookie.value.trim().is_empty());
    let has_app_id = request
        .headers
        .app_id
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let has_csrf = request
        .headers
        .csrf_token
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    has_cookie && has_app_id && has_csrf
}

fn parse_rfc3339_utc(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

fn read_instagram_sync_cooldown_until(settings: &HashMap<String, String>) -> Option<DateTime<Utc>> {
    settings
        .get(INSTAGRAM_SYNC_COOLDOWN_UNTIL_SETTING_KEY)
        .and_then(|value| parse_rfc3339_utc(value))
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
    let seconds = retry_after.as_secs().max(1).min(24 * 60 * 60);
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

fn set_instagram_sync_cooldown(
    connection: &Connection,
    account_id: &str,
    retry_after: Duration,
    now: &str,
) -> Result<DateTime<Utc>, String> {
    let base_time = parse_rfc3339_utc(now).unwrap_or_else(Utc::now);
    let until = base_time + retry_after;
    upsert_provider_account_string_setting(
        connection,
        account_id,
        INSTAGRAM_SYNC_COOLDOWN_UNTIL_SETTING_KEY,
        &until.to_rfc3339(),
        now,
    )?;
    Ok(until)
}

fn clear_instagram_sync_cooldown(connection: &Connection, account_id: &str) -> Result<(), String> {
    delete_provider_account_setting(
        connection,
        account_id,
        INSTAGRAM_SYNC_COOLDOWN_UNTIL_SETTING_KEY,
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

fn instagram_error_indicates_availability_abort_rate_limit(error: &str) -> bool {
    instagram_error_indicates_rate_limit(error)
        && !instagram_error_is_inconclusive_identity_probe(error)
}

fn blocked_instagram_source_sync_outcome(
    request: &instagram_connector::InstagramConnectorRequest,
    status: &str,
    summary: String,
    validation_error: Option<String>,
) -> SourceSyncOutcome {
    SourceSyncOutcome {
        tool: "internal.instagram".to_string(),
        status: status.to_string(),
        summary,
        command_preview: format!(
            "internal.instagram profile {} -> {}",
            request.username,
            request.profile_root.display()
        ),
        manifest_summary_json: None,
        degraded_capabilities: Vec::new(),
        validation_error,
    }
}

fn validate_instagram_source_sync_preflight(
    connection: &Connection,
    context: &SourceSyncContext,
    request: &instagram_connector::InstagramConnectorRequest,
    settings: &HashMap<String, String>,
    now: &str,
) -> Result<(), SourceSyncOutcome> {
    let current_time = parse_rfc3339_utc(now).unwrap_or_else(Utc::now);
    if let Some(until) = read_instagram_sync_cooldown_until(settings) {
        if until > current_time {
            return Err(blocked_instagram_source_sync_outcome(
                request,
                "skipped",
                format!(
                    "Instagram sync skipped: provider cooldown is active until {}.",
                    until.to_rfc3339()
                ),
                None,
            ));
        }
        let _ = clear_instagram_sync_cooldown(connection, &context.account.id);
    }

    if !instagram_request_has_base_auth(request) {
        let reason = "Instagram sync blocked: imported session is missing required base authentication data (cookies, app id, or csrf token).".to_string();
        return Err(blocked_instagram_source_sync_outcome(
            request,
            "failed",
            reason.clone(),
            Some(reason),
        ));
    }

    if !effective_instagram_sections_enabled(&request.sections) {
        return Err(blocked_instagram_source_sync_outcome(
            request,
            "skipped",
            "Instagram sync skipped: no enabled sections remain after account and source settings were applied.".to_string(),
            None,
        ));
    }

    Ok(())
}

/// Lê o `userIdHint` persistido nas opções de sync de um perfil, por provider.
fn source_user_id_hint_from_json(provider: &str, sync_options_json: &str) -> Option<String> {
    let options = serde_json::from_str::<SourceSyncOptions>(sync_options_json).ok()?;
    let hint = if provider.eq_ignore_ascii_case("instagram") {
        options.instagram.and_then(|value| value.user_id_hint)
    } else if provider.eq_ignore_ascii_case("twitter") {
        options.twitter.and_then(|value| value.user_id_hint)
    } else {
        None
    };
    hint.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
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

/// Detecta, no primeiro sync de um perfil, se o `user_id` resolvido já pertence
/// a outro perfil. Em caso afirmativo, remove o recém-adicionado (soft-delete,
/// mantém mídia), registra no log e devolve um outcome explicativo a ser
/// reportado. Só age no primeiro sync (`last_synced_at` vazio) para não mexer em
/// perfis que já vinham sincronizando.
fn detect_duplicate_user_id_on_first_sync(
    connection: &Connection,
    layout: &StorageLayout,
    context: &SourceSyncContext,
    user_id: &str,
    tool: &str,
    command_preview: String,
) -> Option<SourceSyncOutcome> {
    // Configurável em Settings (policy.sync.blockDuplicateUserId). Desligado, a
    // detecção de duplicados por user id no primeiro sync não age.
    let enabled = load_app_setting_value(connection, DUPLICATE_USER_ID_BLOCK_SETTING_KEY)
        .ok()
        .flatten()
        .map(|value| value.trim().eq_ignore_ascii_case("true"))
        .unwrap_or(true);
    if !enabled {
        return None;
    }
    if context.source.last_synced_at.is_some() {
        return None;
    }
    let existing = find_source_with_same_user_id(
        connection,
        &context.source.provider,
        user_id,
        &context.source.id,
    )
    .ok()
    .flatten()?;
    let (_existing_id, existing_handle) = existing;

    // Remove o perfil duplicado recém-adicionado (mantém a mídia no disco).
    let _ = delete_source_profile_with_connection(
        connection,
        layout,
        context.source.id.clone(),
        SourceProfileDeleteMode::UserOnly,
    );

    let summary = format!(
        "'{}' is already registered as '{}' (same user id {}). The newly added duplicate profile was removed and the sync was cancelled.",
        context.source.handle, existing_handle, user_id
    );
    let _ = log_runtime_event(
        layout,
        "sync.profile",
        "warning",
        Some(&context.account.id),
        Some(&context.source.provider),
        Some(&context.source.id),
        Some(&context.source.handle),
        summary.clone(),
        None,
    );

    Some(SourceSyncOutcome {
        tool: tool.to_string(),
        status: "skipped".to_string(),
        summary,
        command_preview,
        manifest_summary_json: None,
        degraded_capabilities: Vec::new(),
        validation_error: None,
    })
}

fn resolve_instagram_source_identity_preflight(
    connection: &Connection,
    layout: &StorageLayout,
    context: &SourceSyncContext,
    source_options: &InstagramSourceSyncOptions,
    request: &mut instagram_connector::InstagramConnectorRequest,
    timestamp: &str,
) -> Result<Option<String>, SourceSyncOutcome> {
    let history_user_id_hint =
        load_latest_instagram_profile_user_id_hint(connection, &context.source.id)
            .ok()
            .flatten();
    let user_id_hint = preferred_instagram_user_id_hint(
        instagram_user_id_hint(source_options),
        history_user_id_hint.as_deref(),
    );
    let identity = match instagram_connector::resolve_profile_identity(
        request,
        user_id_hint.as_deref(),
    ) {
        Ok(identity) => identity,
        Err(error) => match classify_instagram_identity_error(&error) {
            InstagramIdentityErrorClassification::UsernameUnresolvable => {
                let problem_code = "instagram_username_unresolvable";
                let problem_message = format!(
                        "Instagram username could not be resolved from '{}' (source id {}). This can indicate a renamed, disabled, or banned account.",
                        context.source.handle, context.source.id
                    );
                let mark_error = set_source_sync_problem(
                    connection,
                    &context.source.id,
                    problem_code,
                    &problem_message,
                    timestamp,
                    true,
                );
                let mut summary = format!("Instagram sync blocked: {problem_message}");
                if let Err(mark_failure) = mark_error {
                    summary.push_str(&format!(
                        " Failed to persist source problem marker: {mark_failure}."
                    ));
                } else {
                    let _ = log_runtime_event(
                        layout,
                        "sync.profile",
                        "warning",
                        Some(&context.account.id),
                        Some(&context.source.provider),
                        Some(&context.source.id),
                        Some(&context.source.handle),
                        format!(
                            "Marked source '{}' as '{}': {}",
                            context.source.handle, problem_code, problem_message
                        ),
                        // Preserva o erro técnico real do resolver de identidade para
                        // diagnóstico (qual rota/endpoint falhou e com qual status).
                        Some(format!("Identity resolver error: {error}")),
                    );
                }
                return Err(blocked_instagram_source_sync_outcome(
                    request,
                    "failed",
                    summary.clone(),
                    Some(format!("{summary} (identity resolver error: {error})")),
                ));
            }
            InstagramIdentityErrorClassification::PrivateOrRestricted => {
                let problem_code = "instagram_profile_private_or_restricted";
                let problem_message = format!(
                    "Instagram profile '{}' (source id {}) appears private or restricted during identity preflight.",
                    context.source.handle, context.source.id
                );
                let mark_error = set_source_sync_problem(
                    connection,
                    &context.source.id,
                    problem_code,
                    &problem_message,
                    timestamp,
                    false,
                );
                let mut summary = format!("Instagram sync skipped: {problem_message}");
                if let Err(mark_failure) = mark_error {
                    summary.push_str(&format!(
                        " Failed to persist source problem marker: {mark_failure}."
                    ));
                } else {
                    let _ = log_runtime_event(
                        layout,
                        "sync.profile",
                        "info",
                        Some(&context.account.id),
                        Some(&context.source.provider),
                        Some(&context.source.id),
                        Some(&context.source.handle),
                        format!(
                            "Marked source '{}' as '{}': {}",
                            context.source.handle, problem_code, problem_message
                        ),
                        None,
                    );
                }
                return Err(blocked_instagram_source_sync_outcome(
                    request, "skipped", summary, None,
                ));
            }
            InstagramIdentityErrorClassification::Other => {
                return Err(blocked_instagram_source_sync_outcome(
                    request,
                    "failed",
                    format!("Instagram sync failed during username validation: {error}"),
                    Some(error),
                ));
            }
        },
    };

    let resolved_handle = sanitize_source_handle("instagram", &identity.username);
    if resolved_handle.is_empty() {
        return Err(blocked_instagram_source_sync_outcome(
            request,
            "failed",
            "Instagram sync failed during username validation: resolved username is empty."
                .to_string(),
            Some(
                "Instagram sync failed during username validation: resolved username is empty."
                    .to_string(),
            ),
        ));
    }

    // Primeiro sync: se o user id resolvido já pertence a outro perfil, este é
    // um duplicado (handle novo de um usuário já cadastrado) — remove e cancela.
    let resolved_user_id = identity.user_id.trim();
    if !resolved_user_id.is_empty() {
        if let Err(error) = persist_instagram_user_id_hint(
            connection,
            &context.source.id,
            resolved_user_id,
            timestamp,
        ) {
            let summary = format!(
                "Instagram sync failed while persisting the stable profile identity: {error}"
            );
            return Err(blocked_instagram_source_sync_outcome(
                request,
                "failed",
                summary.clone(),
                Some(summary),
            ));
        }
        if let Some(outcome) = detect_duplicate_user_id_on_first_sync(
            connection,
            layout,
            context,
            resolved_user_id,
            "internal.instagram",
            format!(
                "internal.instagram profile {} -> identity preflight",
                context.source.handle
            ),
        ) {
            return Err(outcome);
        }
    }

    // Identity resolved successfully — clear any previous sync problem marker
    // (e.g. instagram_username_unresolvable from a prior availability check).
    if context.source.sync_problem_code.is_some() {
        let _ = clear_source_sync_problem(connection, &context.source.id, timestamp);
    }

    request.username = resolved_handle.clone();
    let current_handle = sanitize_source_handle("instagram", &context.source.handle);
    if current_handle.eq_ignore_ascii_case(&resolved_handle) {
        return Ok(None);
    }

    if instagram_force_update_user_name_enabled(source_options) {
        match update_instagram_source_handle_after_sync(
            connection,
            &context.source.id,
            &resolved_handle,
            timestamp,
        ) {
            Ok(()) => {
                let message = format!(
                    "Instagram username changed from '{}' to '{}'. Source handle updated before sync.",
                    context.source.handle, resolved_handle
                );
                let _ = log_runtime_event(
                    layout,
                    "sync.profile",
                    "info",
                    Some(&context.account.id),
                    Some(&context.source.provider),
                    Some(&context.source.id),
                    Some(&context.source.handle),
                    message,
                    None,
                );
                Ok(Some(format!(
                    " Username changed from '{}' to '{}'.",
                    context.source.handle, resolved_handle
                )))
            }
            Err(error) => {
                let message = format!(
                    "Instagram username change detected ({} -> {}), but source handle preflight update failed: {}",
                    context.source.handle, resolved_handle, error
                );
                let _ = log_runtime_event(
                    layout,
                    "sync.profile",
                    "warning",
                    Some(&context.account.id),
                    Some(&context.source.provider),
                    Some(&context.source.id),
                    Some(&context.source.handle),
                    message,
                    Some(error),
                );
                Ok(Some(format!(
                    " Username changed from '{}' to '{}' (auto-update failed).",
                    context.source.handle, resolved_handle
                )))
            }
        }
    } else {
        let message = format!(
            "Instagram username change detected ({} -> {}), but source auto-update is disabled.",
            context.source.handle, resolved_handle
        );
        let _ = log_runtime_event(
            layout,
            "sync.profile",
            "info",
            Some(&context.account.id),
            Some(&context.source.provider),
            Some(&context.source.id),
            Some(&context.source.handle),
            message,
            None,
        );
        Ok(Some(format!(
            " Username changed from '{}' to '{}' (auto-update disabled).",
            context.source.handle, resolved_handle
        )))
    }
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
        ignore_stories_560_errors: true,
        request_delay_ms: 1000,
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
        ignore_stories_560_errors: true,
        request_delay_ms: 0,
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
        date_from_timestamp: None,
        date_to_timestamp: None,
        media_file_naming_mode: instagram_connector::InstagramMediaFileNamingMode::PresetNewDefault,
        media_file_naming_template: None,
        target_story_media_id: None,
    }
}

fn set_source_sync_problem(
    connection: &Connection,
    source_id: &str,
    code: &str,
    message: &str,
    timestamp: &str,
    disable_ready_for_download: bool,
) -> Result<(), String> {
    if disable_ready_for_download {
        connection
            .execute(
                "UPDATE source_profiles
                 SET sync_problem_code = ?2,
                     sync_problem_message = ?3,
                     sync_problem_at = ?4,
                     ready_for_download = 0,
                     updated_at = ?4
                 WHERE id = ?1
                   AND deleted_at IS NULL",
                params![source_id, code, message, timestamp],
            )
            .map_err(|error| error.to_string())?;
    } else {
        connection
            .execute(
                "UPDATE source_profiles
                 SET sync_problem_code = ?2,
                     sync_problem_message = ?3,
                     sync_problem_at = ?4,
                     updated_at = ?4
                 WHERE id = ?1
                   AND deleted_at IS NULL",
                params![source_id, code, message, timestamp],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InstagramIdentityErrorClassification {
    UsernameUnresolvable,
    PrivateOrRestricted,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum InstagramAvailabilityAction {
    AbortedRateLimited(String),
    Resolved {
        resolved_handle: String,
        handle_changed: bool,
    },
    MarkPrivateOrRestricted {
        resolved_handle: Option<String>,
        handle_changed: bool,
    },
    MarkUsernameUnresolvable,
    Failed(String),
}

fn decide_instagram_availability_action(
    previous_handle: &str,
    primary: &Result<instagram_connector::InstagramProfileIdentity, String>,
    fallback: Option<&Result<instagram_connector::InstagramProfileIdentity, String>>,
) -> InstagramAvailabilityAction {
    match primary {
        Ok(identity) => {
            let identity = match fallback {
                Some(Ok(identity)) => identity,
                Some(Err(error)) => {
                    return InstagramAvailabilityAction::Failed(format!(
                        "The current username resolved to a different Instagram account, and the \
                         stored identity could not be resolved: {error}"
                    ));
                }
                None => identity,
            };
            let resolved_handle = sanitize_source_handle("instagram", &identity.username);
            let handle_changed = !resolved_handle.eq_ignore_ascii_case(previous_handle);
            InstagramAvailabilityAction::Resolved {
                resolved_handle,
                handle_changed,
            }
        }
        Err(primary_error) => {
            if instagram_error_indicates_availability_abort_rate_limit(primary_error) {
                return InstagramAvailabilityAction::AbortedRateLimited(primary_error.clone());
            }
            match classify_instagram_identity_error(primary_error) {
                InstagramIdentityErrorClassification::PrivateOrRestricted => {
                    let resolved_handle = fallback.and_then(|result| {
                        result
                            .as_ref()
                            .ok()
                            .map(|identity| sanitize_source_handle("instagram", &identity.username))
                            .filter(|value| !value.is_empty())
                    });
                    let handle_changed = resolved_handle
                        .as_deref()
                        .map(|value| !value.eq_ignore_ascii_case(previous_handle))
                        .unwrap_or(false);
                    InstagramAvailabilityAction::MarkPrivateOrRestricted {
                        resolved_handle,
                        handle_changed,
                    }
                }
                InstagramIdentityErrorClassification::UsernameUnresolvable => {
                    if let Some(Ok(identity)) = fallback {
                        let resolved_handle =
                            sanitize_source_handle("instagram", &identity.username);
                        let handle_changed = !resolved_handle.eq_ignore_ascii_case(previous_handle);
                        return InstagramAvailabilityAction::Resolved {
                            resolved_handle,
                            handle_changed,
                        };
                    }
                    InstagramAvailabilityAction::MarkUsernameUnresolvable
                }
                InstagramIdentityErrorClassification::Other => {
                    InstagramAvailabilityAction::Failed(primary_error.clone())
                }
            }
        }
    }
}

fn apply_instagram_availability_action(
    connection: &Connection,
    source_id: &str,
    provider: &str,
    previous_handle: &str,
    now: &str,
    action: InstagramAvailabilityAction,
    unchanged: &mut u32,
    updated_handle: &mut u32,
    marked_problem: &mut u32,
    failed: &mut u32,
    items: &mut Vec<SourceAvailabilityCheckItem>,
) -> Result<(), String> {
    match action {
        InstagramAvailabilityAction::AbortedRateLimited(error) => {
            *failed += 1;
            items.push(SourceAvailabilityCheckItem {
                source_id: source_id.to_string(),
                provider: provider.to_string(),
                previous_handle: previous_handle.to_string(),
                current_handle: None,
                status: "failed".to_string(),
                message: format!(
                    "Availability check aborted due to Instagram rate limiting (429): {error}"
                ),
            });
            Ok(())
        }
        InstagramAvailabilityAction::Resolved {
            resolved_handle,
            handle_changed,
        } => {
            if resolved_handle.trim().is_empty() {
                *failed += 1;
                items.push(SourceAvailabilityCheckItem {
                    source_id: source_id.to_string(),
                    provider: provider.to_string(),
                    previous_handle: previous_handle.to_string(),
                    current_handle: None,
                    status: "failed".to_string(),
                    message: "Resolved username is empty.".to_string(),
                });
                return Ok(());
            }

            if !handle_changed {
                let _ = clear_source_sync_problem(connection, source_id, now);
                *unchanged += 1;
                items.push(SourceAvailabilityCheckItem {
                    source_id: source_id.to_string(),
                    provider: provider.to_string(),
                    previous_handle: previous_handle.to_string(),
                    current_handle: Some(resolved_handle),
                    status: "unchanged".to_string(),
                    message: "Profile is still available with the same handle.".to_string(),
                });
                return Ok(());
            }

            match update_instagram_source_handle_after_sync(
                connection,
                source_id,
                &resolved_handle,
                now,
            ) {
                Ok(()) => {
                    let _ = clear_source_sync_problem(connection, source_id, now);
                    *updated_handle += 1;
                    items.push(SourceAvailabilityCheckItem {
                        source_id: source_id.to_string(),
                        provider: provider.to_string(),
                        previous_handle: previous_handle.to_string(),
                        current_handle: Some(resolved_handle),
                        status: "updated_handle".to_string(),
                        message: "Handle was updated using current provider identity.".to_string(),
                    });
                }
                Err(error) => {
                    *failed += 1;
                    items.push(SourceAvailabilityCheckItem {
                        source_id: source_id.to_string(),
                        provider: provider.to_string(),
                        previous_handle: previous_handle.to_string(),
                        current_handle: Some(resolved_handle),
                        status: "failed".to_string(),
                        message: format!(
                            "Handle change was detected, but local update failed: {error}"
                        ),
                    });
                }
            }

            Ok(())
        }
        InstagramAvailabilityAction::MarkPrivateOrRestricted {
            resolved_handle,
            handle_changed,
        } => {
            let problem_message = format!(
                "Instagram profile '{}' (source id {}) appears to be private or temporarily restricted. This is informative and does not disable download readiness.",
                previous_handle, source_id
            );
            let marker = set_source_sync_problem(
                connection,
                source_id,
                "instagram_profile_private_or_restricted",
                &problem_message,
                now,
                false,
            );
            *marked_problem += 1;

            let mut handle_update_error: Option<String> = None;
            let mut handle_updated = false;
            if let (Some(resolved_handle), true) = (resolved_handle.clone(), handle_changed) {
                if !resolved_handle.trim().is_empty() {
                    match update_instagram_source_handle_after_sync(
                        connection,
                        source_id,
                        &resolved_handle,
                        now,
                    ) {
                        Ok(()) => {
                            handle_updated = true;
                            *updated_handle += 1;
                        }
                        Err(error) => handle_update_error = Some(error),
                    }
                }
            }

            items.push(SourceAvailabilityCheckItem {
                source_id: source_id.to_string(),
                provider: provider.to_string(),
                previous_handle: previous_handle.to_string(),
                current_handle: resolved_handle.clone(),
                status: if handle_updated {
                    "updated_handle".to_string()
                } else {
                    "marked_problem".to_string()
                },
                message: if let Err(marker_error) = marker {
                    format!("{problem_message} Failed to persist problem marker: {marker_error}")
                } else if let Some(error) = handle_update_error {
                    format!(
                        "{problem_message} Handle update attempt failed: {error}"
                    )
                } else if handle_updated {
                    "Handle was updated using current provider identity; profile still appears private/restricted.".to_string()
                } else {
                    problem_message
                },
            });

            Ok(())
        }
        InstagramAvailabilityAction::MarkUsernameUnresolvable => {
            let problem_message = format!(
                "Instagram username could not be resolved from '{}' (source id {}). This can indicate a renamed, disabled, or banned account.",
                previous_handle, source_id
            );
            let marker = set_source_sync_problem(
                connection,
                source_id,
                "instagram_username_unresolvable",
                &problem_message,
                now,
                true,
            );
            *marked_problem += 1;
            items.push(SourceAvailabilityCheckItem {
                source_id: source_id.to_string(),
                provider: provider.to_string(),
                previous_handle: previous_handle.to_string(),
                current_handle: None,
                status: "marked_problem".to_string(),
                message: if let Err(marker_error) = marker {
                    format!("{problem_message} Failed to persist problem marker: {marker_error}")
                } else {
                    problem_message
                },
            });
            Ok(())
        }
        InstagramAvailabilityAction::Failed(error) => {
            *failed += 1;
            items.push(SourceAvailabilityCheckItem {
                source_id: source_id.to_string(),
                provider: provider.to_string(),
                previous_handle: previous_handle.to_string(),
                current_handle: None,
                status: "failed".to_string(),
                message: format!("Availability check failed: {error}"),
            });
            Ok(())
        }
    }
}

fn build_availability_rate_limit_skipped_item(
    connection: &Connection,
    source_id: &str,
) -> SourceAvailabilityCheckItem {
    let row = connection
        .query_row(
            "SELECT provider, handle
             FROM source_profiles
             WHERE id = ?1
               AND deleted_at IS NULL
             LIMIT 1",
            params![source_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional();

    let (provider, previous_handle) = match row {
        Ok(Some((provider, handle))) => (provider, handle),
        _ => ("unknown".to_string(), source_id.to_string()),
    };

    SourceAvailabilityCheckItem {
        source_id: source_id.to_string(),
        provider,
        previous_handle,
        current_handle: None,
        status: "skipped".to_string(),
        message: "Skipped because availability check was aborted after a 429 Too Many Requests."
            .to_string(),
    }
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

fn clear_source_sync_problem(
    connection: &Connection,
    source_id: &str,
    timestamp: &str,
) -> Result<(), String> {
    connection
        .execute(
            "UPDATE source_profiles
             SET sync_problem_code = NULL,
                 sync_problem_message = NULL,
                 sync_problem_at = NULL,
                 ready_for_download = 1,
                 updated_at = ?2
             WHERE id = ?1
               AND deleted_at IS NULL",
            params![source_id, timestamp],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
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

fn load_latest_instagram_profile_user_id_hint(
    connection: &Connection,
    source_id: &str,
) -> Result<Option<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT manifest_summary_json
             FROM source_sync_runs
             WHERE source_id = ?1
               AND provider = 'instagram'
               AND status = 'succeeded'
               AND manifest_summary_json IS NOT NULL
             ORDER BY finished_at DESC
             LIMIT 25",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![source_id], |row| row.get::<_, Option<String>>(0))
        .map_err(|error| error.to_string())?;
    for row in rows {
        let Some(raw_json) = row.map_err(|error| error.to_string())? else {
            continue;
        };
        let Ok(summary) = serde_json::from_str::<serde_json::Value>(&raw_json) else {
            continue;
        };
        let Some(user_id) = summary
            .get("profileUserId")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        return Ok(Some(user_id.to_string()));
    }

    Ok(None)
}

struct AccountSyncOutcome {
    tool: String,
    status: String,
    summary: String,
    command_preview: String,
}

fn load_account_sync_context(
    connection: &Connection,
    layout: &StorageLayout,
    account_id: &str,
) -> Result<AccountSyncContext, String> {
    let account = load_provider_account_by_id(connection, account_id)?;
    let session = load_account_session_record(connection, account_id)?
        .ok_or_else(|| format!("Provider account '{}' has no stored session.", account_id))?;
    let session_payload = session_secret_store::load_secret(layout, &session.secret_ref)?;
    let settings = load_provider_account_settings_map(connection, account_id)?;

    Ok(AccountSyncContext {
        account,
        settings,
        session_payload,
    })
}

fn execute_instagram_source_sync_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    context: &SourceSyncContext,
    settings: &HashMap<String, String>,
    trigger: &str,
    run_mode: Option<&str>,
    sync_options_override: Option<&SourceSyncOptions>,
) -> Result<SourceSyncOutcome, String> {
    let source_options =
        source_instagram_sync_options_with_override(&context.source, sync_options_override);
    let started_at = now_timestamp();
    let mut request = build_instagram_profile_sync_request(
        connection,
        context,
        layout,
        settings,
        run_mode,
        sync_options_override,
    )?;
    let mut preflight_handle_change_suffix = String::new();
    if let Err(outcome) = validate_instagram_source_sync_preflight(
        connection,
        context,
        &request,
        settings,
        &started_at,
    ) {
        persist_source_sync_run(
            connection,
            context,
            &outcome,
            trigger,
            &started_at,
            &started_at,
        )?;
        propagate_source_sync_account_health(connection, context, &outcome, &started_at)?;
        source_sync_runtime::report_source_sync_progress(
            &context.source.id,
            Some(0),
            Some(if outcome.status == "skipped" {
                "Download skipped".to_string()
            } else {
                "Download failed".to_string()
            }),
            Some(outcome.summary.clone()),
            false,
            None,
        );
        return Ok(outcome);
    }

    let identity_preflight = resolve_instagram_source_identity_preflight(
        connection,
        layout,
        context,
        &source_options,
        &mut request,
        &started_at,
    );
    match identity_preflight {
        Ok(Some(suffix)) => preflight_handle_change_suffix = suffix,
        Ok(None) => {}
        Err(outcome) => {
            persist_source_sync_run(
                connection,
                context,
                &outcome,
                trigger,
                &started_at,
                &started_at,
            )?;
            propagate_source_sync_account_health(connection, context, &outcome, &started_at)?;
            source_sync_runtime::report_source_sync_progress(
                &context.source.id,
                Some(0),
                Some("Download failed".to_string()),
                Some(outcome.summary.clone()),
                false,
                None,
            );
            return Ok(outcome);
        }
    }

    let cancel_token = register_source_sync_cancel_token(&context.source.id);
    let finished_at;

    if cancel_token.load(Ordering::SeqCst) {
        clear_source_sync_cancel_token(&context.source.id);
        return Err("source sync cancelled by user".to_string());
    }

    source_sync_runtime::report_source_sync_progress(
        &context.source.id,
        Some(0),
        Some("Starting download".to_string()),
        Some("Instagram connector is preparing source sync.".to_string()),
        true,
        Some(0),
    );

    let execution = instagram_connector::run_profile_sync(
        &request,
        |progress| {
            source_sync_runtime::report_source_sync_progress(
                &context.source.id,
                progress.progress_percent,
                Some(progress.label),
                Some(progress.detail),
                progress.indeterminate,
                progress.downloaded_items,
            );
        },
        || cancel_token.load(Ordering::SeqCst),
    );
    clear_source_sync_cancel_token(&context.source.id);
    finished_at = now_timestamp();

    let outcome = match execution {
        Ok(result) => {
            let mut source_handle =
                sanitize_source_handle(&context.source.provider, &request.username);
            let mut handle_change_suffix = preflight_handle_change_suffix.clone();
            let mut description_update_suffix = String::new();
            let mut cooldown_suffix = String::new();
            let force_update_user_name = instagram_force_update_user_name_enabled(&source_options);
            let force_update_user_information =
                instagram_force_update_user_information_enabled(&source_options);

            persist_instagram_runtime_auth_headers(
                connection,
                &context.account.id,
                &result.updated_headers,
                &finished_at,
            )?;

            // Diagnóstico (nível debug): por seção de stories/highlights, registra
            // onde os itens descobertos somem (filtro de data vs dedupe vs já
            // existente). Útil para investigar highlights subdimensionados.
            if let Some(summary) = result.manifest_summary.as_ref() {
                for section in &summary.sections {
                    if section.section != "stories" && section.section != "stories_user" {
                        continue;
                    }
                    let message = format!(
                        "Highlights diagnostic — {}: discovered={}, skipped_out_of_range_date={}, skipped_existing_post={}, skipped_duplicate_post={}, skipped_unavailable_post={}, skipped_existing_asset={}, queued_assets={}",
                        section.label,
                        section.item_count,
                        section.skipped_out_of_range_item_count,
                        section.skipped_existing_post_count,
                        section.skipped_duplicate_post_count,
                        section.skipped_unavailable_post_count,
                        section.skipped_existing_asset_count,
                        section.queued_asset_count,
                    );
                    log_runtime_event(
                        layout,
                        "sync.highlights.diagnostic",
                        "debug",
                        Some(&context.account.id),
                        Some(&context.source.provider),
                        Some(&context.source.id),
                        Some(&context.source.handle),
                        message,
                        None,
                    );
                }
            }

            // Snapshot append-only da participação de posts em álbuns de highlight.
            // Cobre os itens "já existentes" (pulados no download) para que a
            // galeria os mostre sob o destaque sem rebaixar bytes.
            upsert_instagram_highlight_memberships(
                connection,
                &context.source.id,
                &result.highlight_memberships,
                &finished_at,
            )?;

            if let Some(resolved_username) = result.resolved_username.as_deref() {
                let resolved_handle = sanitize_source_handle("instagram", resolved_username);
                if !resolved_handle.is_empty()
                    && !source_handle.eq_ignore_ascii_case(&resolved_handle)
                {
                    if force_update_user_name {
                        match update_instagram_source_handle_after_sync(
                            connection,
                            &context.source.id,
                            &resolved_handle,
                            &finished_at,
                        ) {
                            Ok(()) => {
                                let message = format!(
                                    "Instagram username changed from '{}' to '{}'. Source handle updated automatically.",
                                    context.source.handle, resolved_handle
                                );
                                log_runtime_event(
                                    layout,
                                    "sync.profile",
                                    "info",
                                    Some(&context.account.id),
                                    Some(&context.source.provider),
                                    Some(&context.source.id),
                                    Some(&context.source.handle),
                                    message,
                                    None,
                                );
                                handle_change_suffix = format!(
                                    " Username changed from '{}' to '{}'.",
                                    context.source.handle, resolved_handle
                                );
                                source_handle = resolved_handle;
                            }
                            Err(error) => {
                                let message = format!(
                                    "Instagram username change detected ({} -> {}), but source handle update failed: {}",
                                    context.source.handle, resolved_handle, error
                                );
                                log_runtime_event(
                                    layout,
                                    "sync.profile",
                                    "warning",
                                    Some(&context.account.id),
                                    Some(&context.source.provider),
                                    Some(&context.source.id),
                                    Some(&context.source.handle),
                                    message,
                                    Some(error),
                                );
                            }
                        }
                    } else {
                        let message = format!(
                            "Instagram username change detected ({} -> {}), but source auto-update is disabled.",
                            context.source.handle, resolved_handle
                        );
                        log_runtime_event(
                            layout,
                            "sync.profile",
                            "info",
                            Some(&context.account.id),
                            Some(&context.source.provider),
                            Some(&context.source.id),
                            Some(&context.source.handle),
                            message,
                            None,
                        );
                        handle_change_suffix = format!(
                            " Username changed from '{}' to '{}' (auto-update disabled).",
                            context.source.handle, resolved_handle
                        );
                    }
                }
            }

            if let Some(profile_description) = result.profile_description.as_deref() {
                match update_instagram_source_description_after_sync(
                    connection,
                    &context.source,
                    profile_description,
                    force_update_user_information,
                    &finished_at,
                ) {
                    Ok(true) => {
                        description_update_suffix = if force_update_user_information {
                            " Profile note updated from Instagram biography.".to_string()
                        } else {
                            " Profile note populated from Instagram biography.".to_string()
                        };
                    }
                    Ok(false) => {}
                    Err(error) => {
                        log_runtime_event(
                            layout,
                            "sync.profile",
                            "warning",
                            Some(&context.account.id),
                            Some(&context.source.provider),
                            Some(&context.source.id),
                            Some(&context.source.handle),
                            format!(
                                "Failed to persist Instagram biography for '{}': {}",
                                context.source.handle, error
                            ),
                            Some(error),
                        );
                    }
                }
            }

            source_sync_runtime::report_source_sync_progress(
                &context.source.id,
                Some(100),
                Some("Committing results".to_string()),
                Some("Persisting Instagram sync history plus post and media ledgers.".to_string()),
                true,
                Some(result.downloaded_media.len() as u32),
            );

            let ingested_media_count = catalog_instagram_downloads(
                connection,
                &context.account.id,
                Some(&context.source.id),
                &source_handle,
                &finished_at,
                &result.downloaded_media,
            )?;
            upsert_instagram_media_ledger_entries(
                connection,
                &context.source.id,
                &context.account.id,
                &source_handle,
                &request.profile_root,
                &result.downloaded_media,
                &finished_at,
            )?;
            upsert_instagram_media_alias_entries(
                connection,
                &context.source.id,
                &context.account.id,
                &request.profile_root,
                &result.downloaded_media,
                &finished_at,
            )?;
            upsert_instagram_media_fingerprint_entries(
                connection,
                &context.source.id,
                &context.account.id,
                &request.profile_root,
                &result.downloaded_media,
                &finished_at,
            )?;
            upsert_instagram_media_naming_ledger_entries(
                connection,
                &context.source.id,
                &context.account.id,
                &source_handle,
                &request.profile_root,
                &result.downloaded_media,
                &finished_at,
            )?;
            upsert_instagram_post_ledger_entries(
                connection,
                &context.source.id,
                &context.account.id,
                &source_handle,
                &result.observed_posts,
                &finished_at,
            )?;
            let mut script_suffix = String::new();
            if let Some(script_pattern) = instagram_profile_script_pattern(&source_options) {
                if ingested_media_count > 0 {
                    if let Err(error) =
                        run_profile_post_sync_script(&script_pattern, &request.profile_root)
                    {
                        log_runtime_event(
                            layout,
                            "sync.script",
                            "warning",
                            Some(&context.account.id),
                            Some(&context.source.provider),
                            Some(&context.source.id),
                            Some(&context.source.handle),
                            format!(
                                "Profile post-sync script failed for '{}': {}",
                                context.source.handle, error
                            ),
                            Some(error),
                        );
                        script_suffix =
                            " Post-sync script execution failed (see runtime log).".to_string();
                    }
                }
            }

            if !context.source.profile_image_custom {
                let provider_avatar = match refresh_profile_picture_from_provider(
                    connection,
                    layout,
                    context,
                    &request.profile_root,
                    settings,
                ) {
                    Ok(path) => path,
                    Err(error) => {
                        let message = match error.level {
                            ProfilePictureRefreshLogLevel::Info => format!(
                                "Profile picture refresh skipped for '{}': {}",
                                context.source.handle, error.message
                            ),
                            ProfilePictureRefreshLogLevel::Warning => format!(
                                "Failed to refresh profile picture for '{}': {}",
                                context.source.handle, error.message
                            ),
                        };
                        log_runtime_event(
                            layout,
                            "sync.avatar",
                            error.level.as_str(),
                            Some(&context.account.id),
                            Some(&context.source.provider),
                            Some(&context.source.id),
                            Some(&context.source.handle),
                            message,
                            error.detail,
                        );
                        None
                    }
                };

                let resolved_avatar =
                    provider_avatar.or_else(|| find_source_avatar(&request.profile_root));
                if let Some(avatar_path) = resolved_avatar {
                    let _ = update_source_profile_image(
                        connection,
                        &context.source.id,
                        &avatar_path,
                        &finished_at,
                    );
                }
            }

            if result.validation_error.is_some() && !result.auth_disabled_sections.is_empty() {
                disable_instagram_sections_after_auth_failure(
                    connection,
                    &context.account.id,
                    &result.auth_disabled_sections,
                    &finished_at,
                )?;
            }

            if result.rate_limited {
                let cooldown_until = set_instagram_sync_cooldown(
                    connection,
                    &context.account.id,
                    Duration::seconds(INSTAGRAM_SYNC_RETRY_AFTER_FALLBACK_SECS),
                    &finished_at,
                )?;
                cooldown_suffix = format!(
                    " Provider cooldown active until {} after Instagram rate limiting.",
                    cooldown_until.to_rfc3339()
                );
            } else {
                clear_instagram_sync_cooldown(connection, &context.account.id)?;
            }

            let auth_invalid = result.validation_error.is_some();
            let manifest_summary_json = result
                .manifest_summary
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|error| error.to_string())?;
            let summary = if result.section_errors.is_empty() {
                if auth_invalid {
                    format!(
                            "Instagram sync failed: credentials appear invalid. Downloaded {} media items before failure.",
                            ingested_media_count
                        )
                } else {
                    format_download_success_summary(
                        "Instagram sync succeeded.",
                        ingested_media_count,
                    )
                }
            } else {
                format!(
                    "Instagram sync completed with warnings. Downloaded {} media items. {}",
                    ingested_media_count,
                    result.section_errors.join(" | ")
                )
            };
            let manifest_suffix = format_instagram_manifest_suffix(
                result.manifest_summary.as_ref(),
                auth_invalid || !result.section_errors.is_empty() || ingested_media_count > 0,
            );
            SourceSyncOutcome {
                tool: "internal.instagram".to_string(),
                status: if auth_invalid {
                    "failed".to_string()
                } else {
                    "succeeded".to_string()
                },
                summary: format!(
                    "{summary}{manifest_suffix}{handle_change_suffix}{description_update_suffix}{script_suffix}{cooldown_suffix}"
                ),
                command_preview: format!(
                    "internal.instagram profile {} -> {}",
                    source_handle,
                    request.profile_root.display()
                ),
                manifest_summary_json,
                degraded_capabilities: Vec::new(),
                validation_error: result.validation_error,
            }
        }
        Err(error) => {
            let cancelled_by_user = error
                .trim()
                .to_ascii_lowercase()
                .contains("cancelled by user");
            let mut summary = if cancelled_by_user {
                "Instagram sync cancelled by user.".to_string()
            } else {
                format!("Instagram sync failed: {}", error)
            };
            if instagram_error_indicates_rate_limit(&error) {
                let cooldown_until = set_instagram_sync_cooldown(
                    connection,
                    &context.account.id,
                    Duration::seconds(INSTAGRAM_SYNC_RETRY_AFTER_FALLBACK_SECS),
                    &finished_at,
                )?;
                summary.push_str(&format!(
                    " Provider cooldown active until {} after Instagram rate limiting.",
                    cooldown_until.to_rfc3339()
                ));
            } else if !cancelled_by_user {
                clear_instagram_sync_cooldown(connection, &context.account.id)?;
            }
            SourceSyncOutcome {
                tool: "internal.instagram".to_string(),
                status: "failed".to_string(),
                summary,
                command_preview: format!(
                    "internal.instagram profile {} -> {}",
                    context.source.handle,
                    request.profile_root.display()
                ),
                manifest_summary_json: None,
                degraded_capabilities: Vec::new(),
                validation_error: if cancelled_by_user { None } else { Some(error) },
            }
        }
    };

    if outcome.status == "succeeded" {
        if let Err(error) = clear_source_sync_problem(connection, &context.source.id, &finished_at)
        {
            log_runtime_event(
                layout,
                "sync.profile",
                "warning",
                Some(&context.account.id),
                Some(&context.source.provider),
                Some(&context.source.id),
                Some(&context.source.handle),
                format!(
                    "Instagram sync succeeded, but failed to clear source sync problem marker: {}",
                    error
                ),
                Some(error),
            );
        }
    }

    persist_source_sync_run(
        connection,
        context,
        &outcome,
        trigger,
        &started_at,
        &finished_at,
    )?;
    propagate_source_sync_account_health(connection, context, &outcome, &finished_at)?;

    source_sync_runtime::report_source_sync_progress(
        &context.source.id,
        Some(if outcome.status == "succeeded" {
            100
        } else {
            0
        }),
        Some(match outcome.status.as_str() {
            "succeeded" => "Download finished".to_string(),
            "skipped" => "Download skipped".to_string(),
            _ => "Download failed".to_string(),
        }),
        Some(outcome.summary.clone()),
        false,
        None,
    );

    Ok(outcome)
}

fn execute_instagram_saved_posts_sync_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    account_id: String,
    trigger: &str,
) -> Result<AccountSyncOutcome, String> {
    let context = load_account_sync_context(connection, layout, &account_id)?;
    if !context.account.provider.eq_ignore_ascii_case("instagram") {
        return Err(format!(
            "Provider account '{}' does not belong to Instagram.",
            account_id
        ));
    }

    let request = build_instagram_saved_posts_request(layout, &context)?;
    let started_at = now_timestamp();
    let execution = instagram_connector::run_saved_posts_sync(&request, |_| {}, || false);
    let finished_at = now_timestamp();

    let outcome = match execution {
        Ok(result) => {
            let ingested_media_count = catalog_instagram_downloads(
                connection,
                &context.account.id,
                None,
                &context.account.display_name,
                &finished_at,
                &result.downloaded_media,
            )?;
            AccountSyncOutcome {
                tool: "internal.instagram".to_string(),
                status: "succeeded".to_string(),
                summary: if result.section_errors.is_empty() {
                    format_download_success_summary(
                        "Saved posts sync succeeded.",
                        ingested_media_count,
                    )
                } else {
                    format!(
                        "Saved posts sync completed with warnings. Downloaded {} media items. {}",
                        ingested_media_count,
                        result.section_errors.join(" | ")
                    )
                },
                command_preview: format!(
                    "internal.instagram saved_posts {} -> {}",
                    context.account.display_name,
                    request.saved_posts_root.display()
                ),
            }
        }
        Err(error) => AccountSyncOutcome {
            tool: "internal.instagram".to_string(),
            status: "failed".to_string(),
            summary: format!("Saved posts sync failed: {}", error),
            command_preview: format!(
                "internal.instagram saved_posts {} -> {}",
                context.account.display_name,
                request.saved_posts_root.display()
            ),
        },
    };

    persist_account_sync_run(
        connection,
        &context.account,
        "saved_posts",
        &outcome,
        trigger,
        &started_at,
        &finished_at,
    )?;

    Ok(outcome)
}

fn build_instagram_profile_sync_request(
    connection: &Connection,
    context: &SourceSyncContext,
    layout: &StorageLayout,
    settings: &HashMap<String, String>,
    run_mode: Option<&str>,
    sync_options_override: Option<&SourceSyncOptions>,
) -> Result<instagram_connector::InstagramConnectorRequest, String> {
    let parsed_session = parse_session_payload(&context.session_payload)?;
    let cookies = parsed_session.cookies;
    let metadata = parsed_session.metadata;
    let source_options =
        source_instagram_sync_options_with_override(&context.source, sync_options_override);
    let profile_root = resolve_instagram_profile_root_with_options(
        layout,
        &context.source,
        Some(settings),
        Some(&source_options),
    );
    let saved_posts_root = resolve_instagram_saved_posts_root(layout, Some(settings));
    let existing_media_keys =
        load_existing_instagram_media_identity_keys_for_source(layout, &context.source, settings)?;
    let existing_relative_paths =
        load_existing_instagram_relative_media_paths_for_source(layout, &context.source, settings)?;
    let media_ledger_snapshot =
        load_instagram_media_ledger_snapshot_for_source(connection, &context.source.id)?;
    let media_alias_snapshot =
        load_instagram_media_alias_snapshot_for_source(connection, &context.source.id)?;
    let post_ledger_snapshot =
        load_instagram_post_ledger_snapshot_for_source(connection, &context.source.id)?;
    let username = instagram_username_override(&source_options)
        .map(|value| {
            sanitize_source_handle("instagram", value)
                .trim_start_matches('@')
                .to_string()
        })
        .unwrap_or_else(|| {
            sanitize_source_handle("instagram", &context.source.handle)
                .trim_start_matches('@')
                .to_string()
        });
    if username.is_empty() {
        return Err("Instagram source handle is empty.".to_string());
    }
    let extract_from_video = source_options
        .extract_image_from_video
        .clone()
        .unwrap_or_else(InstagramExtractImageFromVideoSections::default);
    let verified_profile = source_options.verified_profile.unwrap_or(true);
    let media_file_naming_mode = parse_instagram_media_file_naming_mode(settings);
    let media_file_naming_template = parse_instagram_media_file_naming_template(settings);
    let mut ledger_media_keys = media_ledger_snapshot.media_keys;
    ledger_media_keys.extend(media_alias_snapshot.keys);
    let explicit_date_from_timestamp = instagram_date_from_timestamp(&source_options);
    let deleted_post_keys =
        load_instagram_deleted_post_keys(connection, &context.source.id).unwrap_or_default();

    Ok(instagram_connector::InstagramConnectorRequest {
        username,
        cookies: cookies
            .iter()
            .map(|cookie| instagram_connector::SessionCookie {
                domain: cookie.domain.clone(),
                name: cookie.name.clone(),
                value: cookie.value.clone(),
            })
            .collect(),
        headers: build_instagram_auth_headers(settings, &cookies, Some(&metadata)),
        profile_root,
        saved_posts_root,
        ledger_post_keys: post_ledger_snapshot.keys,
        deleted_post_keys,
        existing_media_keys,
        ledger_media_keys,
        existing_relative_paths,
        ledger_relative_paths: media_ledger_snapshot.relative_paths,
        sections: build_instagram_section_selection(
            &context.source,
            settings,
            sync_options_override,
        ),
        use_gql: parse_instagram_use_gql_setting(settings),
        download_saved_posts: parse_bool_setting(
            settings
                .get("instagram.account.downloadSavedPosts")
                .map(String::as_str),
            false,
        ),
        post_page_size: parse_instagram_post_page_size(settings, verified_profile),
        skip_errors: parse_bool_setting(
            settings
                .get("instagram.errors.skipErrors")
                .map(String::as_str),
            true,
        ),
        ignore_stories_560_errors: parse_bool_setting(
            settings
                .get("instagram.errors.ignoreStories560")
                .map(String::as_str),
            false,
        ),
        request_delay_ms: parse_u64_provider_setting(settings, "instagram.timers.requestMs", 1000),
        timeout_secs: 45,
        download_images: source_options.download_images.unwrap_or(true),
        download_videos: source_options.download_videos.unwrap_or(true),
        extract_image_from_video: instagram_connector::InstagramSectionSelection {
            timeline: extract_from_video.timeline,
            reels: extract_from_video.reels,
            stories: extract_from_video.stories,
            stories_user: extract_from_video.stories_user,
            tagged: extract_from_video.tagged,
        },
        place_extracted_image_into_video_folder: source_options
            .place_extracted_image_into_video_folder
            .unwrap_or(false),
        download_text: source_options.download_text.unwrap_or(false),
        download_text_posts: source_options.download_text_posts.unwrap_or(false),
        text_special_folder: source_options.text_special_folder.unwrap_or(true),
        get_user_media_only: source_options.get_user_media_only.unwrap_or(false),
        missing_only: instagram_missing_only_enabled(&source_options),
        date_from_timestamp: explicit_date_from_timestamp
            .or_else(|| implicit_instagram_imported_cutoff_timestamp(&context.source, run_mode)),
        date_to_timestamp: instagram_date_to_timestamp(&source_options),
        media_file_naming_mode,
        media_file_naming_template,
        target_story_media_id: source_options.target_story_media_id.clone(),
    })
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
        ignore_stories_560_errors: parse_bool_setting(
            context
                .settings
                .get("instagram.errors.ignoreStories560")
                .map(String::as_str),
            false,
        ),
        request_delay_ms: parse_u64_provider_setting(
            &context.settings,
            "instagram.timers.requestMs",
            1000,
        ),
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

fn build_instagram_section_selection(
    source: &SourceProfile,
    settings: &HashMap<String, String>,
    sync_options_override: Option<&SourceSyncOptions>,
) -> instagram_connector::InstagramSectionSelection {
    let options = source_instagram_sync_options_with_override(source, sync_options_override);
    instagram_connector::InstagramSectionSelection {
        timeline: parse_bool_setting(
            settings
                .get("instagram.download.timeline")
                .map(String::as_str),
            true,
        ) && options.timeline,
        reels: parse_bool_setting(
            settings.get("instagram.download.reels").map(String::as_str),
            false,
        ) && options.reels,
        stories: parse_bool_setting(
            settings
                .get("instagram.download.stories")
                .map(String::as_str),
            true,
        ) && options.stories,
        stories_user: parse_bool_setting(
            settings
                .get("instagram.download.storiesUser")
                .map(String::as_str),
            true,
        ) && options.stories_user,
        tagged: parse_bool_setting(
            settings
                .get("instagram.download.taggedPosts")
                .map(String::as_str),
            false,
        ) && options.tagged,
    }
}

fn parse_bool_setting_from_keys(
    settings: &HashMap<String, String>,
    keys: &[&str],
    default: bool,
) -> bool {
    for key in keys {
        if let Some(raw) = settings.get(*key) {
            return parse_bool_setting(Some(raw.as_str()), default);
        }
    }

    default
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

fn parse_u64_provider_setting(settings: &HashMap<String, String>, key: &str, default: u64) -> u64 {
    settings
        .get(key)
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn run_profile_post_sync_script(script_pattern: &str, profile_root: &Path) -> Result<(), String> {
    let profile_root_value = profile_root.to_string_lossy();
    let escaped_profile_root = profile_root_value.replace('"', "\\\"");
    let command_line = if script_pattern.contains("{0}") {
        script_pattern.replace("{0}", &escaped_profile_root)
    } else {
        format!(r#"{} "{}""#, script_pattern, escaped_profile_root)
    };

    let mut command = Command::new("cmd");
    configure_background_command(&mut command);
    let status = command
        .arg("/C")
        .arg(&command_line)
        .status()
        .map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Script exited with status {:?}: {}",
            status.code(),
            command_line
        ))
    }
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

fn record_scheduler_launch_with_connection(
    connection: &Connection,
    launched_at: &str,
) -> Result<(), String> {
    upsert_app_setting_value(connection, "runtime.scheduler.launch_at", launched_at)
}

fn process_scheduler_tick_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    now: &str,
) -> Result<Vec<PlanSyncEnqueueRequest>, String> {
    let launch_at = ensure_scheduler_launch_at(connection, now)?;
    let plans = load_active_automatic_sync_plans(connection)?;

    let mut requests = Vec::new();
    for plan in plans {
        let next_due_at = compute_sync_plan_next_due_at(&plan, &launch_at, now)?;
        if let Some(next_due_at_value) = next_due_at.as_deref() {
            if is_timestamp_due(next_due_at_value, now)? {
                let source_ids = run_sync_plan_now_with_connection(
                    connection,
                    layout,
                    &plan.id,
                    "scheduler",
                    now,
                )?;
                requests.extend(source_ids.into_iter().map(|source_id| {
                    PlanSyncEnqueueRequest {
                        source_id,
                        trigger: "scheduler".to_string(),
                    }
                }));
            } else {
                update_sync_plan_runtime_state(
                    connection,
                    &plan.id,
                    None,
                    None,
                    None,
                    None,
                    next_due_at.as_deref(),
                    Some(plan.pause_mode.clone()),
                    plan.pause_until.as_deref(),
                    Some(is_sync_plan_paused(&plan, now)),
                )?;
            }
        }
    }

    Ok(requests)
}

/// Resolve as fontes do plano e devolve os ids a enfileirar — NÃO executa o
/// sync aqui. Rodar os downloads inline (gallery-dl + reqwest + sleeps de rate
/// limit) segurava a conexão/lock do workspace por todo o lote, congelando o
/// app. Os downloads passam pela fila sequencial (source_sync_runtime), que já
/// respeita o delay por conta; o registro do plano apenas marca que N fontes
/// foram enfileiradas.
fn run_sync_plan_now_with_connection(
    connection: &Connection,
    _layout: &StorageLayout,
    plan_id: &str,
    trigger: &str,
    now: &str,
) -> Result<Vec<String>, String> {
    let plan = load_sync_plan(connection, plan_id)?
        .ok_or_else(|| format!("Sync plan '{}' does not exist.", plan_id))?;
    let sources = resolve_sync_plan_sources(connection, &plan)?;
    let started_at = now.to_string();
    let finished_at = now.to_string();
    let source_count = sources.len() as u32;
    let source_ids: Vec<String> = sources.iter().map(|source| source.id.clone()).collect();

    let (status, summary) = if source_count == 0 {
        (
            "skipped".to_string(),
            "No eligible sources matched this plan.".to_string(),
        )
    } else {
        (
            "succeeded".to_string(),
            format!("Queued {} source syncs.", source_count),
        )
    };

    let next_due_at =
        if plan.mode == "automatic" && plan.enabled && !is_sync_plan_paused(&plan, now) {
            compute_sync_plan_next_due_at(
                &SyncPlan {
                    last_run_at: Some(finished_at.clone()),
                    skip_until: None,
                    ..plan.clone()
                },
                &ensure_scheduler_launch_at(connection, now)?,
                now,
            )?
        } else {
            None
        };

    update_sync_plan_runtime_state(
        connection,
        &plan.id,
        Some(&finished_at),
        Some(&status),
        Some(&summary),
        None,
        next_due_at.as_ref().map(String::as_str),
        Some(plan.pause_mode.clone()),
        plan.pause_until.as_deref(),
        Some(false),
    )?;

    persist_sync_plan_run(
        connection,
        &plan,
        trigger,
        &status,
        &summary,
        source_count,
        &started_at,
        &finished_at,
    )?;

    Ok(source_ids)
}

fn set_sync_plan_pause_with_connection(
    connection: &Connection,
    input: &SetSyncPlanPauseInput,
    now: &str,
) -> Result<(), String> {
    let plan = load_sync_plan(connection, &input.id)?
        .ok_or_else(|| format!("Sync plan '{}' does not exist.", input.id))?;
    let pause_until = resolve_pause_until(now, &input.pause_mode, input.pause_until.as_deref())?;
    let launch_at = ensure_scheduler_launch_at(connection, now)?;
    let paused_plan = SyncPlan {
        pause_mode: input.pause_mode.clone(),
        pause_until: pause_until.clone(),
        paused: input.pause_mode != "disabled",
        ..plan.clone()
    };
    update_sync_plan_runtime_state(
        connection,
        &input.id,
        None,
        Some("idle"),
        Some("Plan paused."),
        None,
        compute_sync_plan_next_due_at(&paused_plan, &launch_at, now)?.as_deref(),
        Some(input.pause_mode.clone()),
        pause_until.as_deref(),
        Some(true),
    )?;
    Ok(())
}

fn clear_sync_plan_pause_with_connection(
    connection: &Connection,
    plan_id: &str,
    now: &str,
) -> Result<(), String> {
    let plan = load_sync_plan(connection, plan_id)?
        .ok_or_else(|| format!("Sync plan '{}' does not exist.", plan_id))?;
    let launch_at = ensure_scheduler_launch_at(connection, now)?;
    let next_due_at = compute_sync_plan_next_due_at(
        &SyncPlan {
            pause_mode: "disabled".to_string(),
            pause_until: None,
            paused: false,
            ..plan.clone()
        },
        &launch_at,
        now,
    )?;
    update_sync_plan_runtime_state(
        connection,
        &plan.id,
        None,
        Some("idle"),
        Some("Plan resumed."),
        None,
        next_due_at.as_ref().map(String::as_str),
        Some("disabled".to_string()),
        None,
        Some(false),
    )?;
    Ok(())
}

fn skip_sync_plan_with_connection(
    connection: &Connection,
    input: &SkipSyncPlanInput,
    now: &str,
) -> Result<(), String> {
    let plan = load_sync_plan(connection, &input.id)?
        .ok_or_else(|| format!("Sync plan '{}' does not exist.", input.id))?;
    let launch_at = ensure_scheduler_launch_at(connection, now)?;
    let current_due =
        compute_sync_plan_next_due_at(&plan, &launch_at, now)?.unwrap_or_else(|| now.to_string());
    let skip_until = match input.mode.as_str() {
        "reset" => None,
        "minutes" => Some(add_minutes_to_timestamp(
            now,
            i64::from(input.minutes.unwrap_or(0).max(1)),
        )?),
        "until" => input.until.clone(),
        _ => Some(add_minutes_to_timestamp(
            &current_due,
            i64::from(plan.interval_minutes.max(1)),
        )?),
    };
    let summary = match input.mode.as_str() {
        "reset" => "Cleared pending skip.".to_string(),
        "minutes" => format!(
            "Skipped automatic execution for {} minutes.",
            input.minutes.unwrap_or(0).max(1)
        ),
        "until" => "Skipped automatic execution until the chosen time.".to_string(),
        _ => "Skipped the next scheduled execution.".to_string(),
    };
    let next_due = if input.mode == "reset" {
        compute_sync_plan_next_due_at(&plan, &launch_at, now)?
    } else {
        skip_until.clone()
    };
    update_sync_plan_runtime_state(
        connection,
        &plan.id,
        None,
        Some(if input.mode == "reset" {
            "idle"
        } else {
            "skipped"
        }),
        Some(&summary),
        skip_until.as_deref(),
        next_due.as_deref(),
        Some(plan.pause_mode.clone()),
        plan.pause_until.as_deref(),
        Some(plan.paused),
    )?;
    Ok(())
}

fn move_sync_plan_with_connection(
    connection: &Connection,
    input: &MoveSyncPlanInput,
) -> Result<(), String> {
    let plan = load_sync_plan(connection, &input.id)?
        .ok_or_else(|| format!("Sync plan '{}' does not exist.", input.id))?;
    let sibling_plans = load_sync_plans(connection, &plan.scheduler_set_id)?;
    let Some(index) = sibling_plans.iter().position(|entry| entry.id == input.id) else {
        return Ok(());
    };
    let swap_index = match input.direction.as_str() {
        "up" if index > 0 => Some(index - 1),
        "down" if index + 1 < sibling_plans.len() => Some(index + 1),
        _ => None,
    };
    let Some(target_index) = swap_index else {
        return Ok(());
    };
    let target = &sibling_plans[target_index];
    let now = now_timestamp();
    connection
        .execute(
            "UPDATE sync_plans SET sort_index = ?2, updated_at = ?3 WHERE id = ?1",
            params![&plan.id, target.sort_index, &now],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "UPDATE sync_plans SET sort_index = ?2, updated_at = ?3 WHERE id = ?1",
            params![&target.id, plan.sort_index, &now],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn clone_sync_plan_with_connection(
    connection: &Connection,
    input: &CloneSyncPlanInput,
) -> Result<(), String> {
    let plan = load_sync_plan(connection, &input.id)?
        .ok_or_else(|| format!("Sync plan '{}' does not exist.", input.id))?;
    let max_sort_index = load_sync_plans(connection, &plan.scheduler_set_id)?
        .into_iter()
        .map(|entry| entry.sort_index)
        .max()
        .unwrap_or(0);
    upsert_sync_plan_with_connection(
        connection,
        SyncPlanUpsert {
            id: None,
            scheduler_set_id: plan.scheduler_set_id,
            name: format!("{} Copy", plan.name),
            enabled: plan.enabled,
            mode: plan.mode,
            interval_minutes: plan.interval_minutes,
            startup_delay_minutes: plan.startup_delay_minutes,
            notification_mode: plan.notification_mode,
            target_filter: plan.target_filter,
            sort_index: Some(max_sort_index + 1),
            pause_mode: Some("disabled".to_string()),
            pause_until: None,
            notifications: plan.notifications,
            criteria: plan.criteria,
        },
    )
}

fn ensure_scheduler_launch_at(
    connection: &Connection,
    fallback_now: &str,
) -> Result<String, String> {
    let existing = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1 LIMIT 1",
            params!["runtime.scheduler.launch_at"],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;

    if let Some(value) = existing {
        return Ok(value);
    }

    record_scheduler_launch_with_connection(connection, fallback_now)?;
    Ok(fallback_now.to_string())
}

fn load_active_automatic_sync_plans(connection: &Connection) -> Result<Vec<SyncPlan>, String> {
    let mut statement = connection
        .prepare(
            "SELECT
                p.id,
                p.scheduler_set_id,
                p.name,
                p.enabled,
                p.mode,
                p.interval_minutes,
                p.startup_delay_minutes,
                p.notification_mode,
                p.target_filter,
                p.sort_index,
                p.paused,
                p.pause_mode,
                p.pause_until,
                p.skip_until,
                p.last_run_at,
                p.last_run_status,
                p.last_run_summary,
                p.next_due_at,
                p.notifications_json,
                p.criteria_json
             FROM sync_plans p
             INNER JOIN scheduler_sets s ON s.id = p.scheduler_set_id
             WHERE s.is_active = 1 AND p.enabled = 1 AND p.mode = 'automatic'
             ORDER BY p.sort_index, p.name",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| map_sync_plan_row(row))
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn load_sync_plan(connection: &Connection, plan_id: &str) -> Result<Option<SyncPlan>, String> {
    connection
        .query_row(
            "SELECT
                id,
                scheduler_set_id,
                name,
                enabled,
                mode,
                interval_minutes,
                startup_delay_minutes,
                notification_mode,
                target_filter,
                sort_index,
                paused,
                pause_mode,
                pause_until,
                skip_until,
                last_run_at,
                last_run_status,
                last_run_summary,
                next_due_at,
                notifications_json,
                criteria_json
             FROM sync_plans
             WHERE id = ?1
             LIMIT 1",
            params![plan_id],
            |row| map_sync_plan_row(row),
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn resolve_sync_plan_sources(
    connection: &Connection,
    plan: &SyncPlan,
) -> Result<Vec<SourceProfile>, String> {
    resolve_sources_for_criteria(connection, &plan.criteria)
}

fn preview_sync_plan_target_with_connection(
    connection: &Connection,
    input: SyncPlanTargetPreviewInput,
) -> Result<SyncPlanTargetPreview, String> {
    let criteria = input.criteria;
    let sources = resolve_sources_for_criteria(connection, &criteria)?;
    Ok(SyncPlanTargetPreview {
        source_count: sources.len() as u32,
        sources: sources
            .into_iter()
            .take(120)
            .map(|source| SyncPlanTargetPreviewSource {
                id: source.id,
                handle: source.handle,
                provider: source.provider,
                labels: source.labels,
                ready_for_download: source.ready_for_download,
                remote_state: source.remote_state,
                subscription: source.is_subscription,
                last_synced_at: source.last_synced_at,
            })
            .collect(),
    })
}

fn resolve_sources_for_criteria(
    connection: &Connection,
    criteria: &SchedulerPlanCriteria,
) -> Result<Vec<SourceProfile>, String> {
    let sources = load_sources(connection)?;

    // Os grupos do scheduler funcionam como filtro de pertencimento (membership
    // estática via `source_profiles.group_id`), e NÃO como uma criteria salva
    // que é reavaliada. Eles são interseccionados com os demais filtros do
    // plano (provider, labels, etc.). Incluir um ou mais grupos restringe o
    // resultado às fontes que pertencem a pelo menos um deles; excluir grupos
    // remove suas fontes.
    let included_groups: HashSet<&str> = criteria
        .group_ids_included
        .iter()
        .map(String::as_str)
        .collect();
    let excluded_groups: HashSet<&str> = criteria
        .group_ids_excluded
        .iter()
        .map(String::as_str)
        .collect();

    let mut resolved = sources
        .into_iter()
        .filter(|source| source_matches_scheduler_criteria(source, criteria))
        .filter(|source| {
            if included_groups.is_empty() {
                return true;
            }
            source
                .group_id
                .as_deref()
                .map(|group_id| included_groups.contains(group_id))
                .unwrap_or(false)
        })
        .filter(|source| {
            if excluded_groups.is_empty() {
                return true;
            }
            !source
                .group_id
                .as_deref()
                .map(|group_id| excluded_groups.contains(group_id))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();

    resolved.sort_by(|left, right| left.handle.cmp(&right.handle));
    if let Some(limit) = criteria.users_count {
        resolved.truncate(limit as usize);
    }
    Ok(resolved)
}

fn split_filter_clauses(expression: &str) -> Vec<String> {
    let mut clauses = Vec::new();
    let mut current = Vec::new();

    for token in expression.split_whitespace() {
        if token.eq_ignore_ascii_case("AND") {
            if !current.is_empty() {
                clauses.push(current.join(" "));
                current.clear();
            }
        } else {
            current.push(token.to_string());
        }
    }

    if !current.is_empty() {
        clauses.push(current.join(" "));
    }

    clauses
}

fn source_matches_scheduler_criteria(
    source: &SourceProfile,
    criteria: &SchedulerPlanCriteria,
) -> bool {
    if !criteria.ignore_ready_for_download
        && criteria.ready_for_download
        && !source.ready_for_download
    {
        return false;
    }

    let selected_categories = [
        (criteria.regular, "regular"),
        (criteria.temporary, "temporary"),
        (criteria.favorite, "favorite"),
    ];
    if selected_categories.iter().any(|(enabled, _)| *enabled)
        && !selected_categories
            .iter()
            .any(|(enabled, category)| *enabled && source_profile_category(source) == *category)
    {
        return false;
    }

    if criteria.download_users == criteria.download_subscriptions {
    } else if criteria.download_subscriptions {
        if !source.is_subscription {
            return false;
        }
    } else if source.is_subscription {
        return false;
    }

    let selected_states = [
        (criteria.user_exists, "exists"),
        (criteria.user_suspended, "suspended"),
        (criteria.user_deleted, "deleted"),
    ];
    if selected_states.iter().any(|(enabled, _)| *enabled)
        && !selected_states
            .iter()
            .any(|(enabled, state)| *enabled && source.remote_state.eq_ignore_ascii_case(state))
    {
        return false;
    }

    if criteria.labels_no && !source.labels.is_empty() {
        return false;
    }

    if !criteria.labels_included.is_empty()
        && !criteria.labels_included.iter().all(|label| {
            source
                .labels
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(label))
        })
    {
        return false;
    }

    if !criteria.ignore_excluded_labels
        && criteria.labels_excluded.iter().any(|label| {
            source
                .labels
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(label))
        })
    {
        return false;
    }

    if !criteria.sites_included.is_empty()
        && !criteria
            .sites_included
            .iter()
            .any(|site| source.provider.eq_ignore_ascii_case(site))
    {
        return false;
    }

    if criteria
        .sites_excluded
        .iter()
        .any(|site| source.provider.eq_ignore_ascii_case(site))
    {
        return false;
    }

    if let Some(days_number) = criteria.days_number {
        let cutoff = Utc::now() - Duration::days(i64::from(days_number));
        let is_downloaded_recently = source
            .last_synced_at
            .as_deref()
            .and_then(|last_synced_at| parse_timestamp(last_synced_at).ok())
            .map(|timestamp| timestamp >= cutoff)
            .unwrap_or(false);
        if is_downloaded_recently != criteria.days_is_downloaded {
            return false;
        }
    }

    if criteria.date_from.is_some() || criteria.date_to.is_some() {
        let Some(last_synced_at) = source.last_synced_at.as_deref() else {
            return false;
        };
        let parsed_date = parse_timestamp(last_synced_at)
            .map(|value| value.date_naive())
            .ok();
        let Some(parsed_date) = parsed_date else {
            return false;
        };
        let in_range = criteria
            .date_from
            .as_deref()
            .and_then(parse_date_input)
            .map(|date_from| parsed_date >= date_from)
            .unwrap_or(true)
            && criteria
                .date_to
                .as_deref()
                .and_then(parse_date_input)
                .map(|date_to| parsed_date <= date_to)
                .unwrap_or(true);
        if in_range != criteria.date_in_range {
            return false;
        }
    }

    if let Some(expression) = criteria.advanced_expression.as_deref() {
        let expression = expression.trim();
        if !expression.is_empty()
            && !split_filter_clauses(expression)
                .into_iter()
                .all(|clause| source_matches_clause(source, &clause))
        {
            return false;
        }
    }

    true
}

fn source_matches_clause(source: &SourceProfile, clause: &str) -> bool {
    let Some((field, raw_value)) = clause.split_once('=') else {
        return false;
    };

    let field = field.trim().to_ascii_lowercase();
    let value = raw_value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_ascii_lowercase();

    match field.as_str() {
        "provider" => source.provider.eq_ignore_ascii_case(&value),
        "label" => source
            .labels
            .iter()
            .any(|label| label.eq_ignore_ascii_case(&value)),
        "ready" | "ready_for_download" => {
            let desired = matches!(value.as_str(), "true" | "1" | "yes");
            source.ready_for_download == desired
        }
        "handle" | "source" => source.handle.eq_ignore_ascii_case(&value),
        "account" | "account_id" => source
            .account_id
            .as_deref()
            .is_some_and(|account_id| account_id.eq_ignore_ascii_case(&value)),
        "kind" | "source_kind" => source.source_kind.eq_ignore_ascii_case(&value),
        "state" | "remote_state" => source.remote_state.eq_ignore_ascii_case(&value),
        "subscription" | "is_subscription" => {
            let desired = matches!(value.as_str(), "true" | "1" | "yes");
            source.is_subscription == desired
        }
        _ => false,
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

fn is_sync_plan_paused(plan: &SyncPlan, now: &str) -> bool {
    if plan.pause_mode == "disabled" {
        return false;
    }
    if plan.pause_mode == "until" {
        return plan
            .pause_until
            .as_deref()
            .map(|until| !is_timestamp_due(until, now).unwrap_or(false))
            .unwrap_or(false);
    }
    true
}

fn resolve_pause_until(
    now: &str,
    pause_mode: &str,
    explicit_until: Option<&str>,
) -> Result<Option<String>, String> {
    let duration_minutes = match pause_mode {
        "1h" => Some(60),
        "2h" => Some(120),
        "3h" => Some(180),
        "4h" => Some(240),
        "6h" => Some(360),
        "12h" => Some(720),
        "until" => None,
        _ => None,
    };
    if let Some(minutes) = duration_minutes {
        return Ok(Some(add_minutes_to_timestamp(now, minutes)?));
    }
    Ok(explicit_until.map(str::to_string))
}

fn compute_sync_plan_next_due_at(
    plan: &SyncPlan,
    launch_at: &str,
    now: &str,
) -> Result<Option<String>, String> {
    if !plan.enabled || plan.mode != "automatic" {
        return Ok(None);
    }

    if is_sync_plan_paused(plan, now) {
        if plan.pause_mode == "until" {
            return Ok(plan.pause_until.clone());
        }
        return Ok(None);
    }

    let startup_due_at =
        add_minutes_to_timestamp(launch_at, i64::from(plan.startup_delay_minutes))?;

    let interval_due_at = if let Some(last_run_at) = plan.last_run_at.as_deref() {
        Some(add_minutes_to_timestamp(
            last_run_at,
            i64::from(plan.interval_minutes),
        )?)
    } else {
        None
    };

    let mut candidates = vec![startup_due_at];
    if let Some(interval_due_at_value) = interval_due_at {
        candidates.push(interval_due_at_value);
    }
    if let Some(skip_until) = plan.skip_until.as_deref() {
        candidates.push(skip_until.to_string());
    }

    let next_due = latest_timestamp(candidates)?;
    if is_timestamp_due(&next_due, now)? {
        Ok(Some(next_due))
    } else {
        Ok(Some(next_due))
    }
}

fn update_sync_plan_runtime_state(
    connection: &Connection,
    plan_id: &str,
    last_run_at: Option<&str>,
    last_run_status: Option<&str>,
    last_run_summary: Option<&str>,
    skip_until: Option<&str>,
    next_due_at: Option<&str>,
    pause_mode: Option<String>,
    pause_until: Option<&str>,
    paused: Option<bool>,
) -> Result<(), String> {
    let current = load_sync_plan(connection, plan_id)?
        .ok_or_else(|| format!("Sync plan '{}' does not exist.", plan_id))?;
    let now = now_timestamp();

    connection
        .execute(
            "UPDATE sync_plans
             SET last_run_at = ?2,
                 last_run_status = ?3,
                 last_run_summary = ?4,
                 skip_until = ?5,
                 next_due_at = ?6,
                 pause_mode = ?7,
                 pause_until = ?8,
                 paused = ?9,
                 updated_at = ?10
             WHERE id = ?1",
            params![
                plan_id,
                last_run_at.or(current.last_run_at.as_deref()),
                last_run_status.unwrap_or(&current.last_run_status),
                last_run_summary.or(current.last_run_summary.as_deref()),
                skip_until.or(current.skip_until.as_deref()),
                next_due_at.or(current.next_due_at.as_deref()),
                pause_mode.unwrap_or(current.pause_mode),
                pause_until.or(current.pause_until.as_deref()),
                bool_to_int(paused.unwrap_or(current.paused)),
                now,
            ],
        )
        .map_err(|error| error.to_string())?;

    Ok(())
}

fn persist_sync_plan_run(
    connection: &Connection,
    plan: &SyncPlan,
    trigger: &str,
    status: &str,
    summary: &str,
    source_count: u32,
    started_at: &str,
    finished_at: &str,
) -> Result<SyncPlanRun, String> {
    let id = new_id();
    connection
        .execute(
            "INSERT INTO sync_plan_runs (
                id,
                plan_id,
                scheduler_set_id,
                trigger,
                status,
                summary,
                source_count,
                started_at,
                finished_at,
                created_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![
                &id,
                &plan.id,
                &plan.scheduler_set_id,
                trigger,
                status,
                summary,
                i64::from(source_count),
                started_at,
                finished_at,
            ],
        )
        .map_err(|error| error.to_string())?;

    Ok(SyncPlanRun {
        id,
        plan_id: plan.id.clone(),
        scheduler_set_id: plan.scheduler_set_id.clone(),
        trigger: trigger.to_string(),
        status: status.to_string(),
        summary: summary.to_string(),
        source_count,
        started_at: started_at.to_string(),
        finished_at: finished_at.to_string(),
    })
}

fn upsert_app_setting_value(connection: &Connection, key: &str, value: &str) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO app_settings (key, value, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET
               value = excluded.value,
               updated_at = excluded.updated_at",
            params![key, value, now_timestamp()],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[derive(Default)]
struct CompanionImportRecord {
    provider_user_id: Option<String>,
    provider_username: Option<String>,
    backup_secret_ref: Option<String>,
    backup_provider_user_id: Option<String>,
    backup_provider_username: Option<String>,
    backup_imported_at: Option<String>,
}

fn normalize_companion_provider(provider: &str) -> Result<String, String> {
    let provider = provider.trim().to_ascii_lowercase();
    match provider.as_str() {
        "instagram" | "twitter" | "tiktok" => Ok(provider),
        _ => Err("This provider does not support Companion account import.".to_string()),
    }
}

fn companion_username(value: &str) -> String {
    value.trim().trim_start_matches('@').to_ascii_lowercase()
}

fn load_companion_import_record(
    connection: &Connection,
    account_id: &str,
) -> Result<Option<CompanionImportRecord>, String> {
    connection
        .query_row(
            "SELECT provider_user_id, provider_username, backup_secret_ref, backup_provider_user_id,
                    backup_provider_username, backup_imported_at
             FROM provider_account_import_state WHERE account_id = ?1",
            params![account_id],
            |row| Ok(CompanionImportRecord {
                provider_user_id: row.get(0)?,
                provider_username: row.get(1)?,
                backup_secret_ref: row.get(2)?,
                backup_provider_user_id: row.get(3)?,
                backup_provider_username: row.get(4)?,
                backup_imported_at: row.get(5)?,
            }),
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn validate_companion_capture(
    provider: &str,
    capture: &CompanionAccountCapture,
) -> Result<Vec<String>, String> {
    if capture.cookies.is_empty() || capture.cookies.len() > 300 {
        return Err("The captured cookie count is outside the supported range.".to_string());
    }
    validate_captured_cookies(&convert_provider_cookies_to_captured_cookies(
        capture.cookies.clone(),
    ))?;
    let allowed_domains: &[&str] = match provider {
        "instagram" => &["instagram.com"],
        "twitter" => &["x.com", "twitter.com"],
        "tiktok" => &["tiktok.com"],
        _ => &[],
    };
    let cookie_names = capture.cookies.iter()
        .filter(|cookie| !cookie.value.trim().is_empty())
        .map(|cookie| cookie.name.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    for cookie in &capture.cookies {
        if cookie.value.len() > 16 * 1024
            || !allowed_domains.iter().any(|domain| domain_matches_allowed(&cookie.domain, domain))
        {
            return Err("The capture contains an invalid provider cookie.".to_string());
        }
    }
    let allowed_auth = [
        "csrfToken", "appId", "asbdId", "igWwwClaim", "userAgent", "secChUa",
        "secChUaFullVersionList", "secChUaPlatformVersion", "lsd", "dtsg",
    ];
    for (key, value) in &capture.authorization {
        if !allowed_auth.contains(&key.as_str()) || value.len() > 16 * 1024 {
            return Err(format!("Authorization field '{key}' is not supported."));
        }
    }
    let mut missing = Vec::new();
    if companion_username(&capture.identity.username).is_empty() {
        missing.push("identity.username".to_string());
    }
    let required: &[&[&str]] = match provider {
        "instagram" => &[&["sessionid"], &["csrftoken"]],
        "twitter" => &[&["auth_token"], &["ct0"]],
        "tiktok" => &[&["sessionid", "sessionid_ss"]],
        _ => &[],
    };
    for group in required {
        if !group.iter().any(|name| cookie_names.contains(*name)) {
            missing.push(format!("cookie:{}", group.join("|")));
        }
    }
    Ok(missing)
}

fn companion_metadata(
    provider: &str,
    capture: &CompanionAccountCapture,
) -> CapturedBrowserMetadata {
    let value = |key: &str| capture.authorization.get(key)
        .map(|value| value.trim().to_string()).filter(|value| !value.is_empty());
    let cookie = |name: &str| capture.cookies.iter()
        .find(|cookie| cookie.name.eq_ignore_ascii_case(name))
        .map(|cookie| cookie.value.trim().to_string()).filter(|value| !value.is_empty());
    CapturedBrowserMetadata {
        csrf_token: (provider == "instagram").then(|| value("csrfToken").or_else(|| cookie("csrftoken"))).flatten(),
        app_id: (provider == "instagram").then(|| value("appId")).flatten(),
        asbd_id: (provider == "instagram").then(|| value("asbdId")).flatten(),
        ig_www_claim: (provider == "instagram").then(|| value("igWwwClaim")).flatten(),
        user_agent: value("userAgent"),
        sec_ch_ua: value("secChUa"),
        sec_ch_ua_full_version_list: value("secChUaFullVersionList"),
        sec_ch_ua_platform_version: value("secChUaPlatformVersion"),
        lsd: (provider == "instagram").then(|| value("lsd")).flatten(),
        dtsg: (provider == "instagram").then(|| value("dtsg")).flatten(),
    }
}

fn preview_companion_account_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    capture: &CompanionAccountCapture,
) -> Result<CompanionAccountPreview, String> {
    let provider = normalize_companion_provider(&capture.provider)?;
    let missing_required_fields = validate_companion_capture(&provider, capture)?;
    let username = companion_username(&capture.identity.username);
    let captured_id = capture.identity.provider_user_id.as_deref()
        .map(str::trim).filter(|value| !value.is_empty());
    let mut candidates = Vec::new();
    for account in load_accounts(connection)?.into_iter()
        .filter(|account| account.provider.eq_ignore_ascii_case(&provider))
    {
        let state = load_companion_import_record(connection, &account.id)?;
        let match_kind = state.as_ref().and_then(|state| {
            if captured_id.is_some() && captured_id == state.provider_user_id.as_deref() {
                Some("provider_user_id".to_string())
            } else if !username.is_empty()
                && state.provider_username.as_deref().map(companion_username).as_deref()
                    == Some(username.as_str())
            {
                Some("username".to_string())
            } else {
                None
            }
        });
        let has_session = load_account_session_record(connection, &account.id)?
            .and_then(|session| session_secret_store::has_secret(layout, &session.secret_ref).ok())
            .unwrap_or(false);
        candidates.push(CompanionAccountCandidate {
            account_id: account.id,
            display_name: account.display_name,
            match_kind,
            has_session,
        });
    }
    candidates.sort_by(|left, right| right.match_kind.is_some()
        .cmp(&left.match_kind.is_some()).then_with(|| left.display_name.cmp(&right.display_name)));
    let suggested_account_id = candidates.iter()
        .find(|candidate| candidate.match_kind.as_deref() == Some("provider_user_id"))
        .map(|candidate| candidate.account_id.clone());
    let metadata = companion_metadata(&provider, capture);
    let authorization_fields = [
        ("csrfToken", metadata.csrf_token.as_ref()), ("appId", metadata.app_id.as_ref()),
        ("asbdId", metadata.asbd_id.as_ref()), ("igWwwClaim", metadata.ig_www_claim.as_ref()),
        ("userAgent", metadata.user_agent.as_ref()), ("secChUa", metadata.sec_ch_ua.as_ref()),
        ("secChUaFullVersionList", metadata.sec_ch_ua_full_version_list.as_ref()),
        ("secChUaPlatformVersion", metadata.sec_ch_ua_platform_version.as_ref()),
        ("lsd", metadata.lsd.as_ref()), ("dtsg", metadata.dtsg.as_ref()),
    ].into_iter().filter_map(|(key, value)| value.map(|_| key.to_string())).collect();
    Ok(CompanionAccountPreview {
        provider, username, cookie_count: capture.cookies.len(), authorization_fields,
        missing_required_fields, candidates, suggested_account_id,
    })
}

fn write_provider_account_session_record(
    connection: &Connection,
    account_id: &str,
    secret_ref: &str,
    payload: &str,
    imported_at: &str,
) -> Result<(), String> {
    connection.execute(
        "INSERT INTO provider_account_sessions (
            account_id, auth_mode, session_format, session_hint, fingerprint, secret_ref,
            expires_at, imported_at, last_validated_at, last_validation_error, created_at, updated_at
         ) VALUES (?1, 'imported_session', 'cookie_json', '', ?2, ?3, NULL, ?4, NULL, NULL, ?4, ?4)
         ON CONFLICT(account_id) DO UPDATE SET
            auth_mode = excluded.auth_mode, session_format = excluded.session_format,
            session_hint = '', fingerprint = excluded.fingerprint, secret_ref = excluded.secret_ref,
            expires_at = NULL, imported_at = excluded.imported_at, last_validated_at = NULL,
            last_validation_error = NULL, updated_at = excluded.updated_at",
        params![account_id, session_fingerprint(payload), secret_ref, imported_at],
    ).map_err(|error| error.to_string())?;
    connection.execute(
        "UPDATE provider_accounts SET auth_mode = 'imported_session', updated_at = ?2 WHERE id = ?1",
        params![account_id, imported_at],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

fn clear_plaintext_companion_authorization(
    connection: &Connection,
    account_id: &str,
    provider: &str,
) -> Result<(), String> {
    connection.execute(
        "DELETE FROM provider_account_settings WHERE account_id = ?1 AND setting_key LIKE ?2",
        params![account_id, format!("{provider}.auth.%")],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

fn import_companion_account_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    input: CompanionAccountImportInput,
) -> Result<CompanionAccountImportResult, String> {
    let preview = preview_companion_account_with_connection(connection, layout, &input.capture)?;
    if !preview.missing_required_fields.is_empty() {
        return Err(format!("The browser session is incomplete: {}.",
            preview.missing_required_fields.join(", ")));
    }
    let provider = preview.provider;
    let username = preview.username;
    let (account_id, created) = match input.target_account_id.as_deref()
        .map(str::trim).filter(|value| !value.is_empty())
    {
        Some(id) => {
            let account = load_provider_account_by_id(connection, id)?;
            if !account.provider.eq_ignore_ascii_case(&provider) {
                return Err("The selected account belongs to another provider.".to_string());
            }
            (id.to_string(), false)
        }
        None => (new_id(), true),
    };
    let old_session = load_account_session_record(connection, &account_id)?;
    let old_state = load_companion_import_record(connection, &account_id)?;
    let metadata = companion_metadata(&provider, &input.capture);
    let payload = serialize_session_payload_for_storage(
        &convert_provider_cookies_to_captured_cookies(input.capture.cookies.clone()),
        Some(input.capture.current_url.trim()),
        Some(&metadata),
    )?;
    let new_ref = format!("companion-{}-{}", account_id, Uuid::new_v4());
    session_secret_store::store_secret(layout, &new_ref, &payload)?;
    let imported_at = now_timestamp();
    connection.execute_batch("BEGIN IMMEDIATE TRANSACTION").map_err(|error| error.to_string())?;
    let persisted = (|| {
        if created {
            let descriptor = providers::provider_runtime(&provider)
                .ok_or_else(|| "Provider runtime is unavailable.".to_string())?.descriptor();
            upsert_provider_account_with_connection(connection, layout, ProviderAccountUpsert {
                id: Some(account_id.clone()), provider: provider.clone(),
                display_name: input.create_display_name.as_deref().map(str::trim)
                    .filter(|value| !value.is_empty()).unwrap_or(&username).to_string(),
                auth_mode: "imported_session".to_string(), auth_state: "ready".to_string(),
                capabilities: descriptor.default_capabilities, last_validated_at: None,
            })?;
        }
        write_provider_account_session_record(connection, &account_id, &new_ref, &payload, &imported_at)?;
        clear_plaintext_companion_authorization(connection, &account_id, &provider)?;
        connection.execute(
            "INSERT INTO provider_account_import_state (
                account_id, provider_user_id, provider_username, last_imported_at,
                backup_secret_ref, backup_provider_user_id, backup_provider_username, backup_imported_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(account_id) DO UPDATE SET provider_user_id=excluded.provider_user_id,
                provider_username=excluded.provider_username, last_imported_at=excluded.last_imported_at,
                backup_secret_ref=excluded.backup_secret_ref,
                backup_provider_user_id=excluded.backup_provider_user_id,
                backup_provider_username=excluded.backup_provider_username,
                backup_imported_at=excluded.backup_imported_at",
            params![
                account_id,
                input.capture.identity.provider_user_id.as_deref().map(str::trim).filter(|v| !v.is_empty()),
                username, imported_at,
                old_session.as_ref().map(|session| session.secret_ref.clone()),
                old_state.as_ref().and_then(|state| state.provider_user_id.clone()),
                old_state.as_ref().and_then(|state| state.provider_username.clone()),
                old_session.as_ref().map(|session| session.imported_at.clone()),
            ],
        ).map_err(|error| error.to_string())?;
        Ok::<(), String>(())
    })();
    if let Err(error) = persisted {
        let _ = connection.execute_batch("ROLLBACK");
        let _ = session_secret_store::delete_secret(layout, &new_ref);
        return Err(error);
    }
    if let Err(error) = connection.execute_batch("COMMIT") {
        let _ = session_secret_store::delete_secret(layout, &new_ref);
        return Err(error.to_string());
    }
    if let Some(previous_backup) = old_state.and_then(|state| state.backup_secret_ref) {
        if old_session.as_ref().map(|session| session.secret_ref.as_str()) != Some(previous_backup.as_str()) {
            let _ = session_secret_store::delete_secret(layout, &previous_backup);
        }
    }
    let snapshot = validate_provider_account_with_connection(connection, layout, account_id.clone())?;
    let account = snapshot.accounts.iter().find(|account| account.id == account_id)
        .ok_or_else(|| "Imported account disappeared after validation.".to_string())?;
    let validation_error = snapshot.account_sessions.iter()
        .find(|session| session.account_id == account_id)
        .and_then(|session| session.last_validation_error.clone());
    Ok(CompanionAccountImportResult {
        account_id, created, auth_state: account.auth_state.clone(),
        validation_error, can_revert: old_session.is_some(),
    })
}

fn revert_provider_account_import_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    account_id: &str,
) -> Result<WorkspaceSnapshot, String> {
    let account = load_provider_account_by_id(connection, account_id)?;
    let current = load_account_session_record(connection, account_id)?
        .ok_or_else(|| "The account does not have a current session.".to_string())?;
    let state = load_companion_import_record(connection, account_id)?
        .ok_or_else(|| "The account does not have a Companion import backup.".to_string())?;
    let backup_ref = state.backup_secret_ref.clone()
        .ok_or_else(|| "The account does not have a previous import to restore.".to_string())?;
    let backup_payload = session_secret_store::load_secret(layout, &backup_ref)?;
    let restored_at = state.backup_imported_at.clone().unwrap_or_else(now_timestamp);
    connection.execute_batch("BEGIN IMMEDIATE TRANSACTION").map_err(|error| error.to_string())?;
    let reverted = (|| {
        write_provider_account_session_record(connection, account_id, &backup_ref, &backup_payload, &restored_at)?;
        clear_plaintext_companion_authorization(connection, account_id, &account.provider)?;
        connection.execute(
            "UPDATE provider_account_import_state SET provider_user_id=?2, provider_username=?3,
                last_imported_at=?4, backup_secret_ref=?5, backup_provider_user_id=?6,
                backup_provider_username=?7, backup_imported_at=?8 WHERE account_id=?1",
            params![account_id, state.backup_provider_user_id, state.backup_provider_username,
                restored_at, current.secret_ref, state.provider_user_id,
                state.provider_username, current.imported_at],
        ).map_err(|error| error.to_string())?;
        Ok::<(), String>(())
    })();
    if let Err(error) = reverted {
        let _ = connection.execute_batch("ROLLBACK");
        return Err(error);
    }
    connection.execute_batch("COMMIT").map_err(|error| error.to_string())?;
    validate_provider_account_with_connection(connection, layout, account_id.to_string())
}

#[cfg(test)]
mod companion_account_import_tests {
    use super::*;

    fn cookie(name: &str, value: &str) -> ProviderAccountCookie {
        ProviderAccountCookie {
            domain: ".x.com".to_string(), name: name.to_string(), value: value.to_string(),
            path: "/".to_string(), expires_at: None, secure: true, http_only: true,
        }
    }

    fn capture(token: &str) -> CompanionAccountCapture {
        CompanionAccountCapture {
            provider: "twitter".to_string(),
            current_url: "https://x.com/home".to_string(),
            identity: crate::domain::models::CompanionAccountIdentity {
                provider_user_id: Some("42".to_string()), username: "ninja".to_string(),
            },
            cookies: vec![cookie("auth_token", token), cookie("ct0", &format!("csrf-{token}"))],
            authorization: HashMap::from([("userAgent".to_string(), "Mozilla/5.0 Test".to_string())]),
        }
    }

    #[test]
    fn import_and_revert_swap_the_protected_session() {
        let temp = tempfile::tempdir().expect("temp dir");
        let layout = storage::workspace_layout_from_roots(
            temp.path().join("localappdata"), temp.path().join("userprofile"),
        ).expect("layout");
        with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(connection, test_layout, ProviderAccountUpsert {
                id: Some("account-1".to_string()), provider: "twitter".to_string(),
                display_name: "Existing".to_string(), auth_mode: "imported_session".to_string(),
                auth_state: "ready".to_string(), capabilities: vec!["posts".to_string()],
                last_validated_at: None,
            })?;
            save_provider_account_cookies_with_connection(
                connection, test_layout, "account-1",
                vec![cookie("auth_token", "old"), cookie("ct0", "old-csrf")],
            )?;
            let result = import_companion_account_with_connection(
                connection, test_layout, CompanionAccountImportInput {
                    capture: capture("new"), target_account_id: Some("account-1".to_string()),
                    create_display_name: None,
                },
            )?;
            assert!(result.can_revert);
            assert!(load_provider_account_cookies_with_connection(connection, test_layout, "account-1")?
                .iter().any(|item| item.name == "auth_token" && item.value == "new"));
            let plaintext = connection.query_row(
                "SELECT COUNT(*) FROM provider_account_settings
                 WHERE account_id='account-1' AND setting_key='twitter.auth.userAgent'",
                [], |row| row.get::<_, i64>(0),
            ).map_err(|error| error.to_string())?;
            assert_eq!(plaintext, 0);
            revert_provider_account_import_with_connection(connection, test_layout, "account-1")?;
            assert!(load_provider_account_cookies_with_connection(connection, test_layout, "account-1")?
                .iter().any(|item| item.name == "auth_token" && item.value == "old"));
            Ok(())
        }).expect("import and revert");
    }

    #[test]
    fn preview_is_redacted() {
        let temp = tempfile::tempdir().expect("temp dir");
        let layout = storage::workspace_layout_from_roots(
            temp.path().join("localappdata"), temp.path().join("userprofile"),
        ).expect("layout");
        with_workspace_layout(layout, |connection, test_layout| {
            let preview = preview_companion_account_with_connection(
                connection, test_layout, &capture("super-secret"),
            )?;
            assert_eq!(preview.cookie_count, 2);
            assert!(!serde_json::to_string(&preview).map_err(|error| error.to_string())?
                .contains("super-secret"));
            Ok(())
        }).expect("preview");
    }
}

fn load_provider_account_cookies_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    account_id: &str,
) -> Result<Vec<ProviderAccountCookie>, String> {
    ensure_provider_account_exists(connection, account_id)?;

    let Some(secret_ref) = load_account_session_secret_ref(connection, account_id)? else {
        return Ok(Vec::new());
    };

    if !session_secret_store::has_secret(layout, &secret_ref)? {
        return Ok(Vec::new());
    }

    let secret_payload = session_secret_store::load_secret(layout, &secret_ref)?;
    parse_session_cookies(&secret_payload).map(convert_captured_cookies_to_provider_cookies)
}

fn save_provider_account_cookies_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    account_id: &str,
    cookies: Vec<ProviderAccountCookie>,
) -> Result<WorkspaceSnapshot, String> {
    ensure_provider_account_exists(connection, account_id)?;

    if cookies.is_empty() {
        return Err("Cookie collection cannot be empty.".to_string());
    }

    let captured_cookies = convert_provider_cookies_to_captured_cookies(cookies);
    validate_captured_cookies(&captured_cookies)?;
    let secret_payload = serialize_session_payload_for_storage(&captured_cookies, None, None)?;

    persist_provider_account_session_payload(connection, layout, account_id, &secret_payload)?;
    validate_provider_account_with_connection(connection, layout, account_id.to_string())
}

fn import_provider_account_cookies_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    input: ProviderAccountCookieImport,
) -> Result<WorkspaceSnapshot, String> {
    let account_id = input.account_id.trim().to_string();
    ensure_provider_account_exists(connection, &account_id)?;

    let import_format = input.import_format.trim().to_ascii_lowercase();
    if import_format.is_empty() {
        return Err("Cookie import format cannot be empty.".to_string());
    }

    if input.content.trim().is_empty() {
        return Err("Cookie import content cannot be empty.".to_string());
    }

    let parsed = parse_cookie_import_content(&import_format, &input.content)?;
    validate_captured_cookies(&parsed.cookies)?;
    let secret_payload = serialize_session_payload_for_storage(
        &parsed.cookies,
        parsed.current_url.as_deref(),
        Some(&parsed.metadata),
    )?;

    persist_provider_account_session_payload(connection, layout, &account_id, &secret_payload)?;
    apply_instagram_auth_settings_from_session_metadata(
        connection,
        layout,
        &account_id,
        &parsed.metadata,
        &parsed.cookies,
    )?;
    validate_provider_account_with_connection(connection, layout, account_id)
}

fn clear_provider_account_cookies_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    account_id: &str,
) -> Result<WorkspaceSnapshot, String> {
    ensure_provider_account_exists(connection, account_id)?;

    if let Some(secret_ref) = load_account_session_secret_ref(connection, account_id)? {
        let _ = session_secret_store::delete_secret(layout, &secret_ref);
    }
    if let Some(secret_ref) = load_account_import_backup_secret_ref(connection, account_id)? {
        let _ = session_secret_store::delete_secret(layout, &secret_ref);
    }

    connection
        .execute(
            "DELETE FROM provider_account_sessions WHERE account_id = ?1",
            params![account_id],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "DELETE FROM provider_account_import_state WHERE account_id = ?1",
            params![account_id],
        )
        .map_err(|error| error.to_string())?;

    validate_provider_account_with_connection(connection, layout, account_id.to_string())
}

fn apply_instagram_auth_settings_from_session_metadata(
    connection: &Connection,
    layout: &StorageLayout,
    account_id: &str,
    metadata: &CapturedBrowserMetadata,
    cookies: &[CapturedBrowserCookie],
) -> Result<(), String> {
    let account = load_provider_account_by_id(connection, account_id)?;
    if !account.provider.eq_ignore_ascii_case("instagram") {
        return Ok(());
    }

    let trim_non_empty = |value: Option<&str>| {
        value
            .map(str::trim)
            .filter(|raw| !raw.is_empty())
            .map(str::to_string)
    };

    let csrf_token = trim_non_empty(metadata.csrf_token.as_deref()).or_else(|| {
        cookies
            .iter()
            .find(|cookie| cookie.name.eq_ignore_ascii_case("csrftoken"))
            .and_then(|cookie| trim_non_empty(Some(cookie.value.as_str())))
    });

    let settings_candidates = [
        ("instagram.auth.csrfToken", csrf_token),
        (
            "instagram.auth.appId",
            trim_non_empty(metadata.app_id.as_deref()),
        ),
        (
            "instagram.auth.asbdId",
            trim_non_empty(metadata.asbd_id.as_deref()),
        ),
        (
            "instagram.auth.igWwwClaim",
            trim_non_empty(metadata.ig_www_claim.as_deref()),
        ),
        (
            "instagram.auth.userAgent",
            trim_non_empty(metadata.user_agent.as_deref()),
        ),
        (
            "instagram.auth.secChUa",
            trim_non_empty(metadata.sec_ch_ua.as_deref()),
        ),
        (
            "instagram.auth.secChUaFullVersionList",
            trim_non_empty(metadata.sec_ch_ua_full_version_list.as_deref()),
        ),
        (
            "instagram.auth.secChUaPlatformVersion",
            trim_non_empty(metadata.sec_ch_ua_platform_version.as_deref()),
        ),
        (
            "instagram.auth.lsd",
            trim_non_empty(metadata.lsd.as_deref()),
        ),
        (
            "instagram.auth.dtsg",
            trim_non_empty(metadata.dtsg.as_deref()),
        ),
    ];

    let values = settings_candidates
        .into_iter()
        .filter_map(|(setting_key, string_value)| {
            string_value.map(|value| ProviderAccountSettingValue {
                setting_key: setting_key.to_string(),
                value_kind: ProviderAccountSettingValueKind::String,
                string_value: Some(value),
                json_value: None,
            })
        })
        .collect::<Vec<_>>();

    if values.is_empty() {
        return Ok(());
    }

    let _ = save_provider_account_settings_with_connection(
        connection,
        layout,
        account_id.to_string(),
        values,
    )?;
    Ok(())
}

fn validate_provider_account_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    account_id: String,
) -> Result<WorkspaceSnapshot, String> {
    let account = load_provider_account_by_id(connection, &account_id)?;

    let session = load_account_session_record(connection, &account_id)?;
    let now = now_timestamp();

    let (auth_state, validation_error) = match session.as_ref() {
        None => (
            "expired".to_string(),
            Some("No session is stored for this provider account.".to_string()),
        ),
        Some(record) => {
            let secret_exists = session_secret_store::has_secret(layout, &record.secret_ref)?;

            if !secret_exists {
                (
                    "degraded".to_string(),
                    Some("Session secret is missing from secure storage.".to_string()),
                )
            } else if record
                .expires_at
                .as_deref()
                .is_some_and(is_expired_timestamp)
            {
                (
                    "expired".to_string(),
                    Some("Stored session has expired.".to_string()),
                )
            } else {
                match session_secret_store::load_secret(layout, &record.secret_ref) {
                    Ok(secret) if secret.trim().is_empty() => (
                        "degraded".to_string(),
                        Some("Stored session payload is empty.".to_string()),
                    ),
                    Ok(secret) => match validate_session_payload_for_account(
                        connection,
                        &account_id,
                        &account.provider,
                        &record.auth_mode,
                        &record.session_format,
                        &secret,
                    ) {
                        Ok(()) => ("ready".to_string(), None),
                        Err(error) => ("degraded".to_string(), Some(error)),
                    },
                    Err(error) => (
                        "degraded".to_string(),
                        Some(format!("Secure session secret could not be read: {error}")),
                    ),
                }
            }
        }
    };

    if session.is_some() {
        connection
            .execute(
                "UPDATE provider_account_sessions
                 SET last_validated_at = ?2,
                     last_validation_error = ?3,
                     updated_at = ?2
                 WHERE account_id = ?1",
                params![account_id, now, validation_error],
            )
            .map_err(|error| error.to_string())?;
    }

    connection
        .execute(
            "UPDATE provider_accounts
             SET auth_state = ?2,
                 last_validated_at = ?3,
                 updated_at = ?3
             WHERE id = ?1",
            params![account_id, auth_state, now],
        )
        .map_err(|error| error.to_string())?;

    log_runtime_event(
        layout,
        "auth.validation",
        if auth_state == "ready" {
            "info"
        } else {
            "warning"
        },
        Some(&account_id),
        Some(&account.provider),
        None,
        None,
        format!(
            "Validated provider account '{}' as '{}'.",
            account.provider, auth_state
        ),
        validation_error.clone(),
    );

    load_snapshot(connection, layout)
}

fn ensure_provider_account_exists(connection: &Connection, account_id: &str) -> Result<(), String> {
    let exists = connection
        .query_row(
            "SELECT 1 FROM provider_accounts WHERE id = ?1 LIMIT 1",
            params![account_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .is_some();

    if !exists {
        return Err(format!("Provider account '{}' does not exist.", account_id));
    }

    Ok(())
}

fn load_provider_account_by_id(
    connection: &Connection,
    account_id: &str,
) -> Result<ProviderAccount, String> {
    connection
        .query_row(
            "SELECT
                id,
                provider,
                display_name,
                auth_mode,
                auth_state,
                capabilities_json,
                last_validated_at
             FROM provider_accounts
             WHERE id = ?1
             LIMIT 1",
            params![account_id],
            |row| {
                Ok(ProviderAccount {
                    id: row.get(0)?,
                    provider: row.get(1)?,
                    display_name: row.get(2)?,
                    auth_mode: row.get(3)?,
                    auth_state: row.get(4)?,
                    capabilities: from_json_array(row.get::<_, String>(5)?),
                    last_validated_at: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Provider account '{}' does not exist.", account_id))
}

fn load_account_session(
    connection: &Connection,
    layout: &StorageLayout,
    account_id: &str,
) -> Result<Option<ProviderAccountSession>, String> {
    load_account_session_record(connection, account_id)?
        .map(|record| hydrate_account_session(layout, record))
        .transpose()
}

fn hydrate_account_session(
    layout: &StorageLayout,
    record: ProviderAccountSessionRecord,
) -> Result<ProviderAccountSession, String> {
    let has_secret = session_secret_store::has_secret(layout, &record.secret_ref)?;
    let cookie_count = if has_secret {
        session_secret_store::load_secret(layout, &record.secret_ref)
            .ok()
            .and_then(|secret| parse_session_cookies(&secret).ok())
            .map(|cookies| cookies.len() as u32)
            .unwrap_or(0)
    } else {
        0
    };

    Ok(ProviderAccountSession {
        account_id: record.account_id,
        auth_mode: record.auth_mode,
        session_format: record.session_format,
        fingerprint: record.fingerprint,
        cookie_count,
        imported_at: record.imported_at,
        last_validated_at: record.last_validated_at,
        last_validation_error: record.last_validation_error,
        has_secret,
    })
}

fn load_provider_account_settings(
    connection: &Connection,
    account_id: &str,
) -> Result<Vec<ProviderAccountSettingValue>, String> {
    let mut statement = connection
        .prepare(
            "SELECT setting_key, value_kind, value_text
             FROM provider_account_settings
             WHERE account_id = ?1
             ORDER BY setting_key",
        )
        .map_err(|error| error.to_string())?;

    let mut rows = statement
        .query(params![account_id])
        .map_err(|error| error.to_string())?;

    let mut settings = Vec::new();
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        let setting_key = row.get::<_, String>(0).map_err(|error| error.to_string())?;
        let value_kind = row.get::<_, String>(1).map_err(|error| error.to_string())?;
        let value_text = row.get::<_, String>(2).map_err(|error| error.to_string())?;
        settings.push(map_provider_account_setting_value(
            setting_key,
            &value_kind,
            value_text,
        )?);
    }

    Ok(settings)
}

fn load_provider_account_settings_map(
    connection: &Connection,
    account_id: &str,
) -> Result<HashMap<String, String>, String> {
    Ok(load_provider_account_settings(connection, account_id)?
        .into_iter()
        .filter_map(|setting| {
            setting
                .string_value
                .map(|value| (setting.setting_key, value))
        })
        .collect())
}

fn map_provider_account_setting_value(
    setting_key: String,
    value_kind: &str,
    value_text: String,
) -> Result<ProviderAccountSettingValue, String> {
    match parse_provider_account_setting_value_kind(value_kind)? {
        ProviderAccountSettingValueKind::String => Ok(ProviderAccountSettingValue {
            setting_key,
            value_kind: ProviderAccountSettingValueKind::String,
            string_value: Some(value_text),
            json_value: None,
        }),
        ProviderAccountSettingValueKind::Json => Ok(ProviderAccountSettingValue {
            setting_key,
            value_kind: ProviderAccountSettingValueKind::Json,
            string_value: None,
            json_value: Some(
                serde_json::from_str::<serde_json::Value>(&value_text)
                    .map_err(|error| error.to_string())?,
            ),
        }),
    }
}

fn serialize_provider_account_setting_value(
    value: &ProviderAccountSettingValue,
) -> Result<(&'static str, String), String> {
    match value.value_kind {
        ProviderAccountSettingValueKind::String => {
            if value.json_value.is_some() {
                return Err(format!(
                    "Provider account setting '{}' cannot carry a JSON value when stored as string.",
                    value.setting_key
                ));
            }

            let string_value = value.string_value.clone().ok_or_else(|| {
                format!(
                    "Provider account setting '{}' is missing its string value.",
                    value.setting_key
                )
            })?;

            Ok(("string", string_value))
        }
        ProviderAccountSettingValueKind::Json => {
            if value.string_value.is_some() {
                return Err(format!(
                    "Provider account setting '{}' cannot carry a string value when stored as JSON.",
                    value.setting_key
                ));
            }

            let json_value = value.json_value.clone().ok_or_else(|| {
                format!(
                    "Provider account setting '{}' is missing its JSON value.",
                    value.setting_key
                )
            })?;

            Ok((
                "json",
                serde_json::to_string(&json_value).map_err(|error| error.to_string())?,
            ))
        }
    }
}

fn parse_provider_account_setting_value_kind(
    value: &str,
) -> Result<ProviderAccountSettingValueKind, String> {
    match value {
        "string" => Ok(ProviderAccountSettingValueKind::String),
        "json" => Ok(ProviderAccountSettingValueKind::Json),
        other => Err(format!(
            "Provider account setting kind '{}' is not supported.",
            other
        )),
    }
}

fn load_account_session_record(
    connection: &Connection,
    account_id: &str,
) -> Result<Option<ProviderAccountSessionRecord>, String> {
    connection
        .query_row(
            "SELECT
                account_id,
                auth_mode,
                session_format,
                session_hint,
                fingerprint,
                secret_ref,
                expires_at,
                imported_at,
                last_validated_at,
                last_validation_error
             FROM provider_account_sessions
             WHERE account_id = ?1
             LIMIT 1",
            params![account_id],
            |row| {
                let _: String = row.get(3)?;
                Ok(ProviderAccountSessionRecord {
                    account_id: row.get(0)?,
                    auth_mode: row.get(1)?,
                    session_format: row.get(2)?,
                    fingerprint: row.get(4)?,
                    secret_ref: row.get(5)?,
                    expires_at: row.get(6)?,
                    imported_at: row.get(7)?,
                    last_validated_at: row.get(8)?,
                    last_validation_error: row.get(9)?,
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn load_account_session_secret_ref(
    connection: &Connection,
    account_id: &str,
) -> Result<Option<String>, String> {
    connection
        .query_row(
            "SELECT secret_ref FROM provider_account_sessions WHERE account_id = ?1 LIMIT 1",
            params![account_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn load_account_import_backup_secret_ref(
    connection: &Connection,
    account_id: &str,
) -> Result<Option<String>, String> {
    connection
        .query_row(
            "SELECT backup_secret_ref FROM provider_account_import_state WHERE account_id = ?1",
            params![account_id],
            |row| row.get(0),
        )
        .optional()
        .map(|value| value.flatten())
        .map_err(|error| error.to_string())
}

fn load_source_sync_context(
    connection: &Connection,
    layout: &StorageLayout,
    source_id: &str,
) -> Result<SourceSyncContext, String> {
    let source = connection
        .query_row(
            "SELECT id, provider, source_kind, handle, display_name, account_id, labels_json, ready_for_download, sync_options_json, profile_image_path, profile_image_custom, remote_state, is_subscription, last_synced_at, sync_problem_code, sync_problem_message, sync_problem_at, created_at, group_id, importer_id, imported_at
             FROM source_profiles
             WHERE id = ?1
               AND deleted_at IS NULL
             LIMIT 1",
            params![source_id],
            |row| {
                let provider: String = row.get(1)?;
                Ok(SourceProfile {
                    id: row.get(0)?,
                    provider: provider.clone(),
                    source_kind: row.get(2)?,
                    handle: row.get(3)?,
                    display_name: row.get(4)?,
                    account_id: row.get(5)?,
                    group_id: row.get(18)?,
                    labels: from_json_array(row.get::<_, String>(6)?),
                    ready_for_download: row.get::<_, i64>(7)? != 0,
                    sync_options: deserialize_source_sync_options(
                        &provider,
                        &row.get::<_, String>(8)?,
                    ),
                    profile_image_path: row.get(9)?,
                    profile_image_custom: row.get::<_, i64>(10).unwrap_or(0) != 0,
                    remote_state: row.get::<_, String>(11).unwrap_or_else(|_| "exists".to_string()),
                    is_subscription: row.get::<_, i64>(12).unwrap_or(0) != 0,
                    last_synced_at: row.get(13).ok(),
                    sync_problem_code: row.get(14).ok(),
                    sync_problem_message: row.get(15).ok(),
                    sync_problem_at: row.get(16).ok(),
                    created_at: row.get(17).ok(),
                    importer_id: row.get(19).ok(),
                    imported_at: row.get(20).ok(),
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Source '{}' does not exist.", source_id))?;

    let account_id = source.account_id.clone().ok_or_else(|| {
        format!(
            "Source '{}' is missing a bound provider account.",
            source.handle
        )
    })?;

    let account = connection
        .query_row(
            "SELECT id, provider, display_name, auth_mode, auth_state, capabilities_json, last_validated_at
             FROM provider_accounts
             WHERE id = ?1
             LIMIT 1",
            params![&account_id],
            |row| {
                Ok(ProviderAccount {
                    id: row.get(0)?,
                    provider: row.get(1)?,
                    display_name: row.get(2)?,
                    auth_mode: row.get(3)?,
                    auth_state: row.get(4)?,
                    capabilities: from_json_array(row.get::<_, String>(5)?),
                    last_validated_at: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Provider account '{}' does not exist.", account_id))?;

    let session = load_account_session_record(connection, &account_id)?
        .ok_or_else(|| format!("Provider account '{}' has no stored session.", account_id))?;
    let session_payload = session_secret_store::load_secret(layout, &session.secret_ref)?;

    Ok(SourceSyncContext {
        source,
        account,
        session_payload,
    })
}

fn load_app_settings_map(connection: &Connection) -> Result<HashMap<String, String>, String> {
    Ok(load_app_settings(connection)?
        .into_iter()
        .map(|setting| (setting.key, setting.value))
        .collect())
}

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

fn parse_bool_setting(value: Option<&str>, default: bool) -> bool {
    value
        .map(|raw| match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => default,
        })
        .unwrap_or(default)
}

fn bool_setting_value(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn build_source_sync_invocation(
    connection: &Connection,
    context: &SourceSyncContext,
    layout: &StorageLayout,
    _sync_options_override: Option<&SourceSyncOptions>,
) -> Result<ToolInvocation, String> {
    let cookies = parse_session_cookies(&context.session_payload)?;
    let cookie_file_path = layout
        .cache_root
        .join(format!("source-sync-{}.cookies.txt", context.source.id));
    write_netscape_cookie_file(&cookie_file_path, &cookies)?;
    let output_root =
        resolved_source_media_output_root_with_connection(connection, layout, &context.source)?;
    fs::create_dir_all(&output_root).map_err(|error| error.to_string())?;

    let sanitized_handle = sanitize_source_handle(&context.source.provider, &context.source.handle);
    let target_url = source_target_url(&context.source.provider, &sanitized_handle);

    let runtime = providers::source_sync_runtime(&context.source.provider).ok_or_else(|| {
        format!(
            "Provider '{}' does not have a connector runtime implementation.",
            context.source.provider
        )
    })?;

    let connector_key = runtime
        .tool_setting_key
        .trim()
        .trim_start_matches("tool.")
        .trim_end_matches(".path")
        .to_string();
    let executable =
        connector_runtime::resolve_connector_executable(connection, layout, &connector_key)?;

    let output_root_value = output_root.display().to_string();
    let args = match runtime.argument_mode {
        providers::SourceSyncArgumentMode::YtDlpDirectory => vec![
            "--cookies".to_string(),
            cookie_file_path.display().to_string(),
            "-P".to_string(),
            output_root_value.clone(),
            target_url.clone(),
        ],
        providers::SourceSyncArgumentMode::GalleryDlDirectory => vec![
            "-d".to_string(),
            output_root_value.clone(),
            "--cookies".to_string(),
            cookie_file_path.display().to_string(),
            target_url.clone(),
        ],
    };

    let command_preview = format!(
        "{} {}",
        runtime.default_executable,
        args.iter()
            .map(|argument| {
                if argument == &cookie_file_path.display().to_string() {
                    "<session-cookie-file>".to_string()
                } else {
                    argument.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    );

    Ok(ToolInvocation {
        source_id: context.source.id.clone(),
        handle: context.source.handle.clone(),
        connector_key,
        executable,
        args,
        command_preview,
        working_directory: Some(layout.cache_root.clone()),
        output_root,
        cancel_token: register_source_sync_cancel_token(&context.source.id),
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

fn persist_source_sync_run(
    connection: &Connection,
    context: &SourceSyncContext,
    outcome: &SourceSyncOutcome,
    trigger: &str,
    started_at: &str,
    finished_at: &str,
) -> Result<(), String> {
    let event_type = if outcome.status == "failed" {
        "error"
    } else {
        "response"
    };
    let mut raw = format!(
        "status={}\nstarted_at={started_at}\nfinished_at={finished_at}\ncommand={}\nsummary={}",
        outcome.status, outcome.command_preview, outcome.summary
    );
    if let Some(error) = outcome.validation_error.as_deref() {
        raw.push_str("\nerror=");
        raw.push_str(error);
    }
    if let Some(manifest) = outcome.manifest_summary_json.as_deref() {
        raw.push_str("\nmanifest=");
        raw.push_str(manifest);
    }
    connector_debug::append_current(
        &outcome.tool,
        event_type,
        "sync.result",
        raw,
    );

    connection
        .execute(
            "INSERT INTO source_sync_runs (
                id,
                source_id,
                account_id,
                provider,
                tool,
                trigger,
                status,
                summary,
                command_preview,
                manifest_summary_json,
                degraded_capabilities_json,
                started_at,
                finished_at,
                created_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)",
            params![
                new_id(),
                &context.source.id,
                &context.account.id,
                &context.source.provider,
                &outcome.tool,
                trigger,
                &outcome.status,
                &outcome.summary,
                &outcome.command_preview,
                &outcome.manifest_summary_json,
                to_json_array(&outcome.degraded_capabilities)?,
                started_at,
                finished_at
            ],
        )
        .map_err(|error| error.to_string())?;

    connection
        .execute(
            "UPDATE source_profiles
             SET last_synced_at = ?2,
                 updated_at = ?2
             WHERE id = ?1",
            params![&context.source.id, finished_at],
        )
        .map_err(|error| error.to_string())?;

    Ok(())
}

fn persist_account_sync_run(
    connection: &Connection,
    account: &ProviderAccount,
    sync_scope: &str,
    outcome: &AccountSyncOutcome,
    trigger: &str,
    started_at: &str,
    finished_at: &str,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO account_sync_runs (
                id,
                account_id,
                provider,
                sync_scope,
                tool,
                trigger,
                status,
                summary,
                command_preview,
                started_at,
                finished_at,
                created_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
            params![
                new_id(),
                &account.id,
                &account.provider,
                sync_scope,
                &outcome.tool,
                trigger,
                &outcome.status,
                &outcome.summary,
                &outcome.command_preview,
                started_at,
                finished_at
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

fn propagate_source_sync_account_health(
    connection: &Connection,
    context: &SourceSyncContext,
    outcome: &SourceSyncOutcome,
    finished_at: &str,
) -> Result<(), String> {
    let auth_state = if outcome.validation_error.is_none() {
        "ready"
    } else {
        "degraded"
    };

    connection
        .execute(
            "UPDATE provider_account_sessions
             SET last_validated_at = ?2,
                 last_validation_error = ?3,
                 updated_at = ?2
             WHERE account_id = ?1",
            params![
                &context.account.id,
                finished_at,
                outcome.validation_error.clone()
            ],
        )
        .map_err(|error| error.to_string())?;

    connection
        .execute(
            "UPDATE provider_accounts
             SET auth_state = ?2,
                 last_validated_at = ?3,
                 updated_at = ?3
             WHERE id = ?1",
            params![&context.account.id, auth_state, finished_at],
        )
        .map_err(|error| error.to_string())?;

    Ok(())
}

fn sanitize_source_handle(provider: &str, handle: &str) -> String {
    let trimmed = handle.trim().trim_matches('/');
    match provider {
        "tiktok" => {
            let without_prefix = trimmed.strip_prefix('@').unwrap_or(trimmed);
            format!("@{}", without_prefix)
        }
        _ => trimmed.strip_prefix('@').unwrap_or(trimmed).to_string(),
    }
}

/// Chave canônica de deduplicação: handle sanitizado (sem `@`, sem `/`) em minúsculas.
/// Handles de Instagram/TikTok/Twitter são case-insensitive, então o
/// lowercasing evita que `@Perfil` e `perfil` sejam tratados como distintos.
fn source_dedupe_key(provider: &str, handle: &str) -> String {
    sanitize_source_handle(provider, handle).to_lowercase()
}

/// Procura outro perfil ativo (`deleted_at IS NULL`) do mesmo provider cujo handle
/// normalizado colida com o handle informado, ignorando o próprio `self_id`.
/// Retorna o handle do perfil conflitante, se houver.
fn find_conflicting_source_handle(
    connection: &Connection,
    provider: &str,
    handle: &str,
    self_id: &str,
) -> Result<Option<String>, String> {
    let target_key = source_dedupe_key(provider, handle);
    if target_key.is_empty() {
        return Ok(None);
    }

    let mut statement = connection
        .prepare(
            "SELECT handle
             FROM source_profiles
             WHERE provider = ?1
               AND deleted_at IS NULL
               AND id <> ?2",
        )
        .map_err(|error| error.to_string())?;
    let mut rows = statement
        .query(params![provider, self_id])
        .map_err(|error| error.to_string())?;
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        let existing_handle: String = row.get(0).map_err(|error| error.to_string())?;
        if source_dedupe_key(provider, &existing_handle) == target_key {
            return Ok(Some(existing_handle));
        }
    }

    Ok(None)
}

fn source_target_url(provider: &str, handle: &str) -> String {
    let handle = handle.trim().trim_start_matches('@');
    match provider {
        "instagram" => format!("https://www.instagram.com/{}/", handle),
        // O TikTok exige o `@` no path do perfil.
        "tiktok" => format!("https://www.tiktok.com/@{}", handle),
        "twitter" => format!("https://x.com/{}", handle),
        _ => handle.to_string(),
    }
}

fn source_media_output_root(layout: &StorageLayout, source: &SourceProfile) -> PathBuf {
    resolved_source_media_output_root_for_provider(layout, &source.provider, &source.handle, None)
}

fn resolved_source_media_output_root_for_provider(
    layout: &StorageLayout,
    provider: &str,
    handle: &str,
    settings: Option<&HashMap<String, String>>,
) -> PathBuf {
    if provider.eq_ignore_ascii_case("instagram") {
        return resolve_instagram_profile_root_for_account(
            layout,
            &sanitize_source_handle("instagram", handle),
            settings,
        );
    }

    layout
        .media_root
        .join(sanitize_path_segment(provider))
        .join(sanitize_path_segment(handle))
}

fn resolved_source_media_output_root(
    layout: &StorageLayout,
    source: &SourceProfile,
    settings: Option<&HashMap<String, String>>,
) -> PathBuf {
    if source.provider.eq_ignore_ascii_case("instagram") {
        let options = source_instagram_sync_options(source);
        return resolve_instagram_profile_root_with_options(
            layout,
            source,
            settings,
            Some(&options),
        );
    }

    if source.provider.eq_ignore_ascii_case("twitter") {
        return resolve_twitter_profile_root(layout, source, settings);
    }

    if source.provider.eq_ignore_ascii_case("tiktok") {
        return resolve_tiktok_profile_root(layout, source, settings);
    }

    resolved_source_media_output_root_for_provider(
        layout,
        &source.provider,
        &source.handle,
        settings,
    )
}

/// Resolve a pasta de mídia de um perfil TikTok: specialPath (absoluto, por
/// perfil) > `tiktok.account.mediaPath` da conta + handle > media_root global.
fn resolve_tiktok_profile_root(
    layout: &StorageLayout,
    source: &SourceProfile,
    settings: Option<&HashMap<String, String>>,
) -> PathBuf {
    let options = source_tiktok_sync_options(source);
    if let Some(special) = options
        .special_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return PathBuf::from(special);
    }

    let sanitized_handle = sanitize_source_handle("tiktok", &source.handle);
    if let Some(media_path) =
        settings.and_then(|map| setting_value(map, "tiktok.account.mediaPath"))
    {
        return PathBuf::from(media_path).join(&sanitized_handle);
    }

    layout
        .media_root
        .join(sanitize_path_segment("tiktok"))
        .join(sanitize_path_segment(&sanitized_handle))
}

/// Resolve a pasta de mídia de um perfil Twitter: specialPath (absoluto, por
/// perfil) > `twitter.account.mediaPath` da conta + handle > media_root global
/// + twitter + handle.
fn resolve_twitter_profile_root(
    layout: &StorageLayout,
    source: &SourceProfile,
    settings: Option<&HashMap<String, String>>,
) -> PathBuf {
    let options = source_twitter_sync_options(source);
    if let Some(special) = options
        .special_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return PathBuf::from(special);
    }

    let sanitized_handle = sanitize_source_handle("twitter", &source.handle);
    if let Some(media_path) =
        settings.and_then(|map| setting_value(map, "twitter.account.mediaPath"))
    {
        return PathBuf::from(media_path).join(&sanitized_handle);
    }

    layout
        .media_root
        .join(sanitize_path_segment("twitter"))
        .join(sanitize_path_segment(&sanitized_handle))
}

fn resolved_source_media_output_root_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    source: &SourceProfile,
) -> Result<PathBuf, String> {
    let account_settings = source
        .account_id
        .as_deref()
        .map(|id| load_provider_account_settings_map(connection, id))
        .transpose()?;
    Ok(resolved_source_media_output_root(
        layout,
        source,
        account_settings.as_ref(),
    ))
}

fn instagram_media_base_root(
    layout: &StorageLayout,
    settings: Option<&HashMap<String, String>>,
) -> PathBuf {
    settings
        .and_then(|values| setting_value(values, "instagram.account.mediaPath"))
        .map(PathBuf::from)
        .unwrap_or_else(|| layout.media_root.join("instagram"))
}

fn resolve_instagram_profile_root_with_options(
    layout: &StorageLayout,
    source: &SourceProfile,
    settings: Option<&HashMap<String, String>>,
    options: Option<&InstagramSourceSyncOptions>,
) -> PathBuf {
    if let Some(path_override) = options.and_then(instagram_special_path) {
        let override_path = PathBuf::from(path_override);
        if override_path.is_absolute() {
            return override_path;
        }

        return instagram_media_base_root(layout, settings).join(override_path);
    }

    resolve_instagram_profile_root_for_account(
        layout,
        &sanitize_source_handle("instagram", &source.handle),
        settings,
    )
}

fn resolve_instagram_profile_root_for_account(
    layout: &StorageLayout,
    handle: &str,
    settings: Option<&HashMap<String, String>>,
) -> PathBuf {
    let sanitized_handle = sanitize_path_segment(handle.trim().trim_start_matches('@'));
    instagram_media_base_root(layout, settings).join(sanitized_handle)
}

fn resolve_instagram_saved_posts_root(
    layout: &StorageLayout,
    settings: Option<&HashMap<String, String>>,
) -> PathBuf {
    settings
        .and_then(|values| setting_value(values, "instagram.account.savedPostsPath"))
        .map(PathBuf::from)
        .unwrap_or_else(|| instagram_media_base_root(layout, settings).join("!Saved"))
}

fn sanitize_path_segment(value: &str) -> String {
    let sanitized = value
        .trim()
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            value if value.is_control() => '_',
            value => value,
        })
        .collect::<String>();

    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}

fn catalog_source_media_output(
    _connection: &Connection,
    _context: &SourceSyncContext,
    output_root: &Path,
    _captured_at: &str,
) -> Result<usize, String> {
    Ok(count_downloaded_media_items(output_root) as usize)
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

fn collect_media_file_paths(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut collected = Vec::new();

    if !root.exists() {
        return Ok(collected);
    }

    let mut pending = vec![root.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|error| error.to_string())?;

            if file_type.is_dir() {
                pending.push(path);
                continue;
            }

            if file_type.is_file() {
                collected.push(path);
            }
        }
    }

    collected.sort();
    Ok(collected)
}

fn normalize_media_file_path(path: &Path) -> Result<String, String> {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    Ok(resolved.to_string_lossy().into_owned())
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

fn normalize_instagram_relative_media_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
        .trim()
        .trim_start_matches('/')
        .to_ascii_lowercase()
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

fn ensure_instagram_sync_media_ledger_table(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS instagram_sync_media_ledger (
                source_id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                source_handle TEXT NOT NULL,
                provider_media_key TEXT NOT NULL,
                media_type TEXT NOT NULL,
                media_section TEXT NOT NULL,
                relative_path TEXT NOT NULL,
                provider_post_code TEXT,
                first_seen_at TEXT NOT NULL,
                last_seen_at TEXT NOT NULL,
                PRIMARY KEY (source_id, provider_media_key, media_type),
                FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE,
                FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_instagram_sync_media_ledger_source_path
                ON instagram_sync_media_ledger(source_id, relative_path);

            CREATE INDEX IF NOT EXISTS idx_instagram_sync_media_ledger_account_key
                ON instagram_sync_media_ledger(account_id, provider_media_key);",
        )
        .map_err(|error| error.to_string())
}

fn ensure_instagram_media_key_aliases_table(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS instagram_media_key_aliases (
                source_id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                provider_media_key TEXT NOT NULL,
                alias_key TEXT NOT NULL,
                alias_kind TEXT NOT NULL,
                file_sha256 TEXT,
                relative_path TEXT,
                first_seen_at TEXT NOT NULL,
                last_seen_at TEXT NOT NULL,
                PRIMARY KEY (source_id, provider_media_key, alias_key),
                FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE,
                FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_instagram_media_key_aliases_source_alias
                ON instagram_media_key_aliases(source_id, alias_key);

            CREATE INDEX IF NOT EXISTS idx_instagram_media_key_aliases_provider_key
                ON instagram_media_key_aliases(source_id, provider_media_key);

            CREATE INDEX IF NOT EXISTS idx_instagram_media_key_aliases_sha256
                ON instagram_media_key_aliases(source_id, file_sha256);",
        )
        .map_err(|error| error.to_string())
}

fn load_instagram_media_alias_snapshot_for_source(
    connection: &Connection,
    source_id: &str,
) -> Result<InstagramMediaAliasSnapshot, String> {
    ensure_instagram_media_key_aliases_table(connection)?;
    let mut statement = connection
        .prepare(
            "SELECT provider_media_key, alias_key
             FROM instagram_media_key_aliases
             WHERE source_id = ?1",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![source_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?;

    let mut snapshot = InstagramMediaAliasSnapshot::default();
    for row in rows {
        let (provider_media_key, alias_key) = row.map_err(|error| error.to_string())?;
        snapshot.keys.insert(provider_media_key);
        snapshot.keys.insert(alias_key);
    }

    Ok(snapshot)
}

fn load_instagram_media_ledger_snapshot_for_source(
    connection: &Connection,
    source_id: &str,
) -> Result<InstagramMediaLedgerSnapshot, String> {
    ensure_instagram_sync_media_ledger_table(connection)?;
    let mut statement = connection
        .prepare(
            "SELECT provider_media_key, relative_path
             FROM instagram_sync_media_ledger
             WHERE source_id = ?1",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![source_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?;

    let mut snapshot = InstagramMediaLedgerSnapshot::default();
    for row in rows {
        let (provider_media_key, relative_path) = row.map_err(|error| error.to_string())?;
        snapshot.media_keys.insert(provider_media_key);
        snapshot.relative_paths.insert(relative_path);
    }

    Ok(snapshot)
}

fn upsert_instagram_media_ledger_entries(
    connection: &Connection,
    source_id: &str,
    account_id: &str,
    source_handle: &str,
    profile_root: &Path,
    downloaded_media: &[instagram_connector::DownloadedInstagramMedia],
    timestamp: &str,
) -> Result<(), String> {
    ensure_instagram_sync_media_ledger_table(connection)?;

    for media in downloaded_media {
        if is_profile_picture_file(&media.file_path) {
            continue;
        }

        let relative_path = normalize_instagram_relative_media_path(profile_root, &media.file_path);
        let provider_post_code = media
            .provider_post_code
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        connection
            .execute(
                "INSERT INTO instagram_sync_media_ledger (
                    source_id,
                    account_id,
                    source_handle,
                    provider_media_key,
                    media_type,
                    media_section,
                    relative_path,
                    provider_post_code,
                    first_seen_at,
                    last_seen_at
                 )
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
                 ON CONFLICT(source_id, provider_media_key, media_type)
                 DO UPDATE SET
                    account_id = excluded.account_id,
                    source_handle = excluded.source_handle,
                    media_section = excluded.media_section,
                    relative_path = excluded.relative_path,
                    provider_post_code = COALESCE(excluded.provider_post_code, instagram_sync_media_ledger.provider_post_code),
                    last_seen_at = excluded.last_seen_at",
                params![
                    source_id,
                    account_id,
                    source_handle,
                    media.provider_media_key.to_ascii_lowercase(),
                    &media.media_type,
                    &media.media_section,
                    relative_path,
                    provider_post_code,
                    timestamp,
                ],
            )
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn compute_file_sha256(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

fn ensure_instagram_media_fingerprints_table(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS instagram_media_fingerprints (
                source_id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                provider_media_key TEXT NOT NULL,
                media_type TEXT NOT NULL,
                media_section TEXT NOT NULL,
                width INTEGER,
                height INTEGER,
                file_sha256 TEXT,
                ahash64 TEXT,
                dhash64 TEXT,
                relative_path TEXT,
                first_seen_at TEXT NOT NULL,
                last_seen_at TEXT NOT NULL,
                PRIMARY KEY (source_id, provider_media_key, media_type),
                FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE,
                FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_instagram_media_fingerprints_sha256
                ON instagram_media_fingerprints(source_id, file_sha256);

            CREATE INDEX IF NOT EXISTS idx_instagram_media_fingerprints_perceptual
                ON instagram_media_fingerprints(source_id, media_section, width, height, ahash64, dhash64);",
        )
        .map_err(|error| error.to_string())
}

fn average_hash_64(image: &image::DynamicImage) -> String {
    let resized = image.resize_exact(8, 8, FilterType::Triangle).grayscale();
    let pixels = resized
        .to_luma8()
        .pixels()
        .map(|pixel| pixel[0])
        .collect::<Vec<_>>();
    let average = pixels.iter().map(|value| u64::from(*value)).sum::<u64>() / 64;
    let mut hash = 0u64;
    for (index, value) in pixels.iter().enumerate() {
        if u64::from(*value) >= average {
            hash |= 1u64 << index;
        }
    }
    format!("{hash:016x}")
}

fn difference_hash_64(image: &image::DynamicImage) -> String {
    let resized = image.resize_exact(9, 8, FilterType::Triangle).grayscale();
    let pixels = resized.to_luma8();
    let mut hash = 0u64;
    let mut bit_index = 0usize;
    for y in 0..8 {
        for x in 0..8 {
            let left = pixels.get_pixel(x, y)[0];
            let right = pixels.get_pixel(x + 1, y)[0];
            if left >= right {
                hash |= 1u64 << bit_index;
            }
            bit_index += 1;
        }
    }
    format!("{hash:016x}")
}

fn compute_instagram_media_fingerprint(path: &Path) -> Option<(u32, u32, String, String)> {
    if infer_media_type(path).as_deref() != Some("image") {
        return None;
    }

    let image = image::open(path).ok()?;
    let (width, height) = image.dimensions();
    Some((
        width,
        height,
        average_hash_64(&image),
        difference_hash_64(&image),
    ))
}

fn upsert_instagram_media_fingerprint_row(
    connection: &Connection,
    source_id: &str,
    account_id: &str,
    provider_media_key: &str,
    media_type: &str,
    media_section: &str,
    file_path: &Path,
    profile_root: &Path,
    file_sha256: Option<&str>,
    timestamp: &str,
) -> Result<(), String> {
    let relative_path = normalize_instagram_relative_media_path(profile_root, file_path);
    let fingerprint = compute_instagram_media_fingerprint(file_path);
    let (width, height, ahash64, dhash64) = match fingerprint {
        Some((width, height, ahash64, dhash64)) => (
            Some(i64::from(width)),
            Some(i64::from(height)),
            Some(ahash64),
            Some(dhash64),
        ),
        None => (None, None, None, None),
    };

    connection
        .execute(
            "INSERT INTO instagram_media_fingerprints (
                source_id,
                account_id,
                provider_media_key,
                media_type,
                media_section,
                width,
                height,
                file_sha256,
                ahash64,
                dhash64,
                relative_path,
                first_seen_at,
                last_seen_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)
             ON CONFLICT(source_id, provider_media_key, media_type)
             DO UPDATE SET
                account_id = excluded.account_id,
                media_section = excluded.media_section,
                width = COALESCE(excluded.width, instagram_media_fingerprints.width),
                height = COALESCE(excluded.height, instagram_media_fingerprints.height),
                file_sha256 = COALESCE(excluded.file_sha256, instagram_media_fingerprints.file_sha256),
                ahash64 = COALESCE(excluded.ahash64, instagram_media_fingerprints.ahash64),
                dhash64 = COALESCE(excluded.dhash64, instagram_media_fingerprints.dhash64),
                relative_path = COALESCE(excluded.relative_path, instagram_media_fingerprints.relative_path),
                last_seen_at = excluded.last_seen_at",
            params![
                source_id,
                account_id,
                provider_media_key.to_ascii_lowercase(),
                media_type,
                media_section,
                width,
                height,
                file_sha256,
                ahash64.as_deref(),
                dhash64.as_deref(),
                Some(relative_path.as_str()),
                timestamp,
            ],
        )
        .map_err(|error| error.to_string())?;

    Ok(())
}

fn collect_instagram_media_alias_rows(
    provider_media_key: &str,
    final_file_name: &str,
    legacy_raw_file_name: Option<&str>,
) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let mut rows = Vec::new();

    let mut push_alias = |alias_kind: &str, value: &str| {
        if let Some(alias_key) = normalize_instagram_media_identity_key(value) {
            if seen.insert(alias_key.clone()) {
                rows.push((alias_key, alias_kind.to_string()));
            }
        }
    };

    push_alias("provider_media_key", provider_media_key);
    for candidate in extract_instagram_media_identity_candidates_from_file_name(final_file_name) {
        push_alias("final_file_name", &candidate);
    }
    if let Some(raw_file_name) = legacy_raw_file_name {
        for candidate in extract_instagram_media_identity_candidates_from_file_name(raw_file_name) {
            push_alias("legacy_raw_file_name", &candidate);
        }
    }

    rows
}

fn upsert_instagram_media_alias_entries(
    connection: &Connection,
    source_id: &str,
    account_id: &str,
    profile_root: &Path,
    downloaded_media: &[instagram_connector::DownloadedInstagramMedia],
    timestamp: &str,
) -> Result<(), String> {
    ensure_instagram_media_key_aliases_table(connection)?;

    for media in downloaded_media {
        if is_profile_picture_file(&media.file_path) {
            continue;
        }

        let file_sha256 = compute_file_sha256(&media.file_path).ok();
        let relative_path = normalize_instagram_relative_media_path(profile_root, &media.file_path);
        let aliases = collect_instagram_media_alias_rows(
            &media.provider_media_key,
            &media.final_file_name,
            media.legacy_raw_file_name.as_deref(),
        );

        for (alias_key, alias_kind) in aliases {
            connection
                .execute(
                    "INSERT INTO instagram_media_key_aliases (
                        source_id,
                        account_id,
                        provider_media_key,
                        alias_key,
                        alias_kind,
                        file_sha256,
                        relative_path,
                        first_seen_at,
                        last_seen_at
                     )
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
                     ON CONFLICT(source_id, provider_media_key, alias_key)
                     DO UPDATE SET
                        account_id = excluded.account_id,
                        alias_kind = excluded.alias_kind,
                        file_sha256 = COALESCE(excluded.file_sha256, instagram_media_key_aliases.file_sha256),
                        relative_path = COALESCE(excluded.relative_path, instagram_media_key_aliases.relative_path),
                        last_seen_at = excluded.last_seen_at",
                    params![
                        source_id,
                        account_id,
                        media.provider_media_key.to_ascii_lowercase(),
                        alias_key,
                        alias_kind,
                        file_sha256.as_deref(),
                        Some(relative_path.as_str()),
                        timestamp,
                    ],
                )
                .map_err(|error| error.to_string())?;
        }
    }

    Ok(())
}

fn upsert_instagram_media_fingerprint_entries(
    connection: &Connection,
    source_id: &str,
    account_id: &str,
    profile_root: &Path,
    downloaded_media: &[instagram_connector::DownloadedInstagramMedia],
    timestamp: &str,
) -> Result<(), String> {
    ensure_instagram_media_fingerprints_table(connection)?;
    for media in downloaded_media {
        if is_profile_picture_file(&media.file_path) {
            continue;
        }

        let file_sha256 = compute_file_sha256(&media.file_path).ok();
        upsert_instagram_media_fingerprint_row(
            connection,
            source_id,
            account_id,
            &media.provider_media_key,
            &media.media_type,
            &media.media_section,
            &media.file_path,
            profile_root,
            file_sha256.as_deref(),
            timestamp,
        )?;
    }

    Ok(())
}

fn upsert_instagram_legacy_media_alias_entries(
    connection: &Connection,
    source_id: &str,
    account_id: &str,
    profile_root: &Path,
    records: &[LegacyInstagramReconciliationRecord],
    timestamp: &str,
) -> Result<(), String> {
    ensure_instagram_media_key_aliases_table(connection)?;

    for record in records {
        let relative_path =
            normalize_instagram_relative_media_path(profile_root, &record.file_path);
        let mut seen = HashSet::new();
        for (alias_key, alias_kind) in &record.alias_keys {
            let Some(normalized_alias_key) = normalize_instagram_media_identity_key(alias_key)
            else {
                continue;
            };
            if !seen.insert(normalized_alias_key.clone()) {
                continue;
            }

            connection
                .execute(
                    "INSERT INTO instagram_media_key_aliases (
                        source_id,
                        account_id,
                        provider_media_key,
                        alias_key,
                        alias_kind,
                        file_sha256,
                        relative_path,
                        first_seen_at,
                        last_seen_at
                     )
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
                     ON CONFLICT(source_id, provider_media_key, alias_key)
                     DO UPDATE SET
                        account_id = excluded.account_id,
                        alias_kind = excluded.alias_kind,
                        file_sha256 = COALESCE(excluded.file_sha256, instagram_media_key_aliases.file_sha256),
                        relative_path = COALESCE(excluded.relative_path, instagram_media_key_aliases.relative_path),
                        last_seen_at = excluded.last_seen_at",
                    params![
                        source_id,
                        account_id,
                        record.provider_media_key.as_str(),
                        normalized_alias_key,
                        alias_kind.as_str(),
                        record.file_sha256.as_deref(),
                        Some(relative_path.as_str()),
                        timestamp,
                    ],
                )
                .map_err(|error| error.to_string())?;
        }
    }

    Ok(())
}

fn upsert_instagram_legacy_media_fingerprint_entries(
    connection: &Connection,
    source_id: &str,
    account_id: &str,
    profile_root: &Path,
    records: &[LegacyInstagramReconciliationRecord],
    timestamp: &str,
) -> Result<(), String> {
    ensure_instagram_media_fingerprints_table(connection)?;
    for record in records {
        upsert_instagram_media_fingerprint_row(
            connection,
            source_id,
            account_id,
            &record.provider_media_key,
            &record.media_type,
            &record.media_section,
            &record.file_path,
            profile_root,
            record.file_sha256.as_deref(),
            timestamp,
        )?;
    }

    Ok(())
}

fn ensure_instagram_media_naming_ledger_table(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS instagram_media_naming_ledger (
                source_id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                source_handle TEXT NOT NULL,
                provider_media_key TEXT NOT NULL,
                media_type TEXT NOT NULL,
                media_section TEXT NOT NULL,
                captured_at INTEGER,
                extension TEXT NOT NULL,
                final_file_name TEXT NOT NULL,
                legacy_raw_file_name TEXT,
                relative_path TEXT NOT NULL,
                pattern_mode TEXT NOT NULL,
                pattern_template TEXT,
                first_seen_at TEXT NOT NULL,
                last_seen_at TEXT NOT NULL,
                PRIMARY KEY (source_id, provider_media_key, media_type),
                FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE,
                FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_instagram_media_naming_ledger_source_path
                ON instagram_media_naming_ledger(source_id, relative_path);

            CREATE INDEX IF NOT EXISTS idx_instagram_media_naming_ledger_account_key
                ON instagram_media_naming_ledger(account_id, provider_media_key);",
        )
        .map_err(|error| error.to_string())
}

fn upsert_instagram_media_naming_ledger_entries(
    connection: &Connection,
    source_id: &str,
    account_id: &str,
    source_handle: &str,
    profile_root: &Path,
    downloaded_media: &[instagram_connector::DownloadedInstagramMedia],
    timestamp: &str,
) -> Result<(), String> {
    ensure_instagram_media_naming_ledger_table(connection)?;

    for media in downloaded_media {
        if is_profile_picture_file(&media.file_path) {
            continue;
        }

        let final_file_name = if media.final_file_name.trim().is_empty() {
            media
                .file_path
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or_default()
                .to_string()
        } else {
            media.final_file_name.clone()
        };
        let relative_path = normalize_instagram_relative_media_path(profile_root, &media.file_path);

        connection
            .execute(
                "INSERT INTO instagram_media_naming_ledger (
                    source_id,
                    account_id,
                    source_handle,
                    provider_media_key,
                    media_type,
                    media_section,
                    captured_at,
                    extension,
                    final_file_name,
                    legacy_raw_file_name,
                    relative_path,
                    pattern_mode,
                    pattern_template,
                    first_seen_at,
                    last_seen_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14)
                ON CONFLICT(source_id, provider_media_key, media_type)
                DO UPDATE SET
                    account_id = excluded.account_id,
                    source_handle = excluded.source_handle,
                    media_section = excluded.media_section,
                    captured_at = excluded.captured_at,
                    extension = excluded.extension,
                    final_file_name = excluded.final_file_name,
                    legacy_raw_file_name = excluded.legacy_raw_file_name,
                    relative_path = excluded.relative_path,
                    pattern_mode = excluded.pattern_mode,
                    pattern_template = excluded.pattern_template,
                    last_seen_at = excluded.last_seen_at",
                params![
                    source_id,
                    account_id,
                    source_handle,
                    media.provider_media_key.to_ascii_lowercase(),
                    &media.media_type,
                    &media.media_section,
                    media.captured_at_timestamp,
                    &media.extension,
                    final_file_name,
                    media.legacy_raw_file_name.as_deref(),
                    relative_path,
                    &media.pattern_mode,
                    media.pattern_template.as_deref(),
                    timestamp,
                ],
            )
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn ensure_instagram_sync_post_ledger_table(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS instagram_sync_post_ledger (
                source_id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                source_handle TEXT NOT NULL,
                provider_post_key TEXT NOT NULL,
                provider_post_code TEXT NOT NULL DEFAULT '',
                media_section TEXT NOT NULL,
                first_seen_at TEXT NOT NULL,
                last_seen_at TEXT NOT NULL,
                PRIMARY KEY (source_id, provider_post_key),
                FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE,
                FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_instagram_sync_post_ledger_account_key
                ON instagram_sync_post_ledger(account_id, provider_post_key);

            CREATE INDEX IF NOT EXISTS idx_instagram_sync_post_ledger_source_code
                ON instagram_sync_post_ledger(source_id, provider_post_code);",
        )
        .map_err(|error| error.to_string())
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

fn reconcile_instagram_scrawler_profile_ledgers_with_connection(
    connection: &Connection,
    profile_root: &Path,
    source_id: &str,
    account_id: &str,
    source_handle: &str,
    timestamp: &str,
) -> Result<LegacyInstagramReconciliationStats, String> {
    let records = collect_legacy_instagram_reconciliation_records(profile_root)?;
    if records.is_empty() {
        return Ok(LegacyInstagramReconciliationStats::default());
    }

    let downloaded_media = records
        .iter()
        .map(|record| instagram_connector::DownloadedInstagramMedia {
            file_path: record.file_path.clone(),
            media_type: record.media_type.clone(),
            media_section: record.media_section.clone(),
            provider_media_key: record.provider_media_key.clone(),
            provider_post_code: record.provider_post_code_cased.clone(),
            captured_at_timestamp: None,
            final_file_name: record
                .file_path
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or_default()
                .to_string(),
            legacy_raw_file_name: Some(record.legacy_file_name.clone()),
            extension: record
                .file_path
                .extension()
                .and_then(|value| value.to_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_ascii_lowercase())
                .unwrap_or_else(|| match record.media_type.as_str() {
                    "video" => "mp4".to_string(),
                    _ => "jpg".to_string(),
                }),
            pattern_mode: "legacy_backfill".to_string(),
            pattern_template: None,
        })
        .collect::<Vec<_>>();

    let mut observed_posts_by_key =
        HashMap::<String, instagram_connector::ObservedInstagramPost>::new();
    for record in &records {
        if record.provider_post_key.trim().is_empty() {
            continue;
        }

        observed_posts_by_key
            .entry(record.provider_post_key.clone())
            .or_insert_with(|| instagram_connector::ObservedInstagramPost {
                provider_post_key: record.provider_post_key.clone(),
                provider_post_code: record.provider_post_code.clone(),
                media_section: record.media_section.clone(),
            });
    }
    let observed_posts = observed_posts_by_key.into_values().collect::<Vec<_>>();

    upsert_instagram_media_ledger_entries(
        connection,
        source_id,
        account_id,
        source_handle,
        profile_root,
        &downloaded_media,
        timestamp,
    )?;
    upsert_instagram_media_alias_entries(
        connection,
        source_id,
        account_id,
        profile_root,
        &downloaded_media,
        timestamp,
    )?;
    upsert_instagram_media_fingerprint_entries(
        connection,
        source_id,
        account_id,
        profile_root,
        &downloaded_media,
        timestamp,
    )?;
    upsert_instagram_legacy_media_alias_entries(
        connection,
        source_id,
        account_id,
        profile_root,
        &records,
        timestamp,
    )?;
    upsert_instagram_legacy_media_fingerprint_entries(
        connection,
        source_id,
        account_id,
        profile_root,
        &records,
        timestamp,
    )?;
    upsert_instagram_post_ledger_entries(
        connection,
        source_id,
        account_id,
        source_handle,
        &observed_posts,
        timestamp,
    )?;

    Ok(LegacyInstagramReconciliationStats {
        seeded_media_entries: downloaded_media.len() as u32,
        seeded_post_entries: observed_posts.len() as u32,
    })
}

fn normalize_instagram_post_ledger_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn count_downloaded_media_items(root: &Path) -> u32 {
    collect_media_file_paths(root)
        .map(|paths| {
            paths
                .into_iter()
                .filter(|path| !is_profile_picture_file(path) && infer_media_type(path).is_some())
                .count() as u32
        })
        .unwrap_or(0)
}

fn infer_media_type(path: &Path) -> Option<&'static str> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())?;

    match extension.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" => Some("image"),
        "mp4" | "mkv" | "mov" | "webm" | "avi" | "m4v" => Some("video"),
        _ => None,
    }
}

fn is_profile_picture_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(PROFILE_PICTURE_FILE_NAME))
}

fn profile_picture_path(output_root: &Path) -> PathBuf {
    output_root.join(PROFILE_PICTURE_FILE_NAME)
}

fn ensure_profile_picture_at_root(
    output_root: &Path,
    candidate_path: &Path,
) -> Result<PathBuf, ProfilePictureRefreshError> {
    let target_path = profile_picture_path(output_root);
    if candidate_path != target_path {
        let temporary_path = output_root.join(format!("{PROFILE_PICTURE_FILE_NAME}.download"));
        fs::copy(candidate_path, &temporary_path)
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

        cleanup_promoted_profile_picture_candidate(output_root, candidate_path);
    }

    // Sync to Settings/ for NinjaCrawler
    if let Ok(settings_path) = sync_profile_picture_to_settings(output_root) {
        return Ok(settings_path);
    }

    Ok(target_path)
}

fn cleanup_promoted_profile_picture_candidate(output_root: &Path, candidate_path: &Path) {
    if !candidate_path.starts_with(output_root) {
        return;
    }

    let _ = fs::remove_file(candidate_path);
    let mut current = candidate_path.parent();
    while let Some(directory) = current {
        if directory == output_root {
            break;
        }

        match fs::remove_dir(directory) {
            Ok(()) => current = directory.parent(),
            Err(_) => break,
        }
    }
}

fn settings_profile_picture_path(output_root: &Path) -> PathBuf {
    output_root
        .join(PROFILE_SETTINGS_DIR_NAME)
        .join(PROFILE_PICTURE_FILE_NAME)
}

fn sync_profile_picture_to_settings(output_root: &Path) -> Result<PathBuf, String> {
    let root_picture = profile_picture_path(output_root);
    if !root_picture.exists() {
        return Err("No profile picture at root".to_string());
    }

    let settings_dir = output_root.join(PROFILE_SETTINGS_DIR_NAME);
    fs::create_dir_all(&settings_dir)
        .map_err(|e| format!("Failed to create Settings directory: {e}"))?;

    let settings_picture = settings_profile_picture_path(output_root);

    // Archive existing Settings/ProfilePicture.jpg if it differs from root
    if settings_picture.exists() {
        let root_size = fs::metadata(&root_picture).map(|m| m.len()).unwrap_or(0);
        let settings_size = fs::metadata(&settings_picture)
            .map(|m| m.len())
            .unwrap_or(0);
        if root_size != settings_size {
            archive_profile_picture(&settings_picture);
        }
    }

    fs::copy(&root_picture, &settings_picture)
        .map_err(|e| format!("Failed to copy profile picture to Settings: {e}"))?;

    Ok(settings_picture)
}

fn archive_profile_picture(existing_path: &Path) {
    let parent = match existing_path.parent() {
        Some(p) => p,
        None => return,
    };
    let date_str = chrono::Local::now().format("%Y-%m-%d").to_string();
    let base_name = format!("ProfilePicture_{date_str}.jpg");
    let mut target = parent.join(&base_name);
    let mut suffix = 2u32;
    while target.exists() {
        target = parent.join(format!("ProfilePicture_{date_str}_{suffix}.jpg"));
        suffix += 1;
    }
    let _ = fs::rename(existing_path, &target);
}

fn find_source_avatar(output_root: &Path) -> Option<String> {
    // Check Settings/ first (NinjaCrawler path)
    let settings_picture = settings_profile_picture_path(output_root);
    if settings_picture.exists() {
        return normalize_media_file_path(&settings_picture).ok();
    }

    // Fallback to root ProfilePicture.jpg
    let profile_picture = profile_picture_path(output_root);
    if profile_picture.exists() {
        return normalize_media_file_path(&profile_picture).ok();
    }

    // Layout legado do SCrawler (perfis importados): Settings/Pictures/UserPicture.jpg
    let scrawler_picture = output_root
        .join(PROFILE_SETTINGS_DIR_NAME)
        .join("Pictures")
        .join("UserPicture.jpg");
    if scrawler_picture.exists() {
        return normalize_media_file_path(&scrawler_picture).ok();
    }

    // Heuristic search for avatar files only in the root directory (not subdirectories).
    // Gallery-dl creates nested instagram/{handle}/ directories containing ProfilePicture.jpg
    // which must be ignored — only Settings/ is the canonical avatar location.
    let image_extensions = ["jpg", "jpeg", "png", "gif", "webp", "bmp"];
    let mut candidates: Vec<PathBuf> = Vec::new();

    let entries = fs::read_dir(output_root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let file_name = path.file_name()?.to_str()?.to_ascii_lowercase();
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .unwrap_or_default();

        let matches_avatar_name = file_name.contains("avatar")
            || file_name.contains("pfp")
            || (file_name.contains("profile")
                && (file_name.contains("pic")
                    || file_name.contains("image")
                    || file_name.contains("photo")));

        if matches_avatar_name && image_extensions.contains(&extension.as_str()) {
            candidates.push(path);
        }
    }

    candidates.sort_by(|a, b| {
        let meta_a = fs::metadata(a).and_then(|m| m.modified()).ok();
        let meta_b = fs::metadata(b).and_then(|m| m.modified()).ok();
        meta_b.cmp(&meta_a)
    });

    candidates
        .first()
        .and_then(|path| normalize_media_file_path(path).ok())
}

fn refresh_profile_picture_from_provider(
    connection: &Connection,
    _layout: &StorageLayout,
    context: &SourceSyncContext,
    output_root: &Path,
    settings: &HashMap<String, String>,
) -> Result<Option<String>, ProfilePictureRefreshError> {
    if cfg!(test) {
        return Ok(None);
    }

    if context.source.provider.eq_ignore_ascii_case("instagram") {
        return refresh_instagram_profile_picture(connection, context, output_root, settings);
    }

    Ok(None)
}

/// Faz o upgrade da URL do avatar do Twitter para o tamanho original,
/// removendo o sufixo de tamanho antes da extensão (`_normal`, `_bigger`,
/// `_400x400`, etc.), espelhando o que o SCrawler faz com o UserPicture.
fn upgrade_twitter_avatar_url(url: &str) -> String {
    const SIZE_SUFFIXES: [&str; 7] = [
        "_normal", "_bigger", "_mini", "_200x200", "_400x400", "_x96", "_reasonably_small",
    ];
    let (base, query) = match url.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (url, None),
    };
    let upgraded_base = match base.rsplit_once('.') {
        Some((stem, ext)) => {
            let mut stem = stem.to_string();
            for suffix in SIZE_SUFFIXES {
                if let Some(trimmed) = stem.strip_suffix(suffix) {
                    stem = trimmed.to_string();
                    break;
                }
            }
            format!("{stem}.{ext}")
        }
        None => base.to_string(),
    };
    match query {
        Some(query) => format!("{upgraded_base}?{query}"),
        None => upgraded_base,
    }
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

#[allow(clippy::too_many_arguments)]
fn try_instagram_graphql_avatar(
    client: &reqwest::blocking::Client,
    user_id: &str,
    lsd: &str,
    dtsg: &str,
    cookie_header: &str,
    referer: &str,
    user_agent: &str,
    csrf_token: &str,
    ig_www_claim: &str,
    ig_app_id: Option<&str>,
    ig_asbd_id: Option<&str>,
) -> Result<String, ProfilePictureRefreshError> {
    let variables = serde_json::json!({
        "id": user_id,
        "render_surface": "PROFILE",
        "__relay_internal__pv__PolarisCannesGuardianExperienceEnabledrelayprovider": true,
        "__relay_internal__pv__PolarisCASB976ProfileEnabledrelayprovider": false,
        "__relay_internal__pv__PolarisRepostsConsumptionEnabledrelayprovider": false
    })
    .to_string();
    let doc_id = "25980296051578533";
    let friendly_name = "PolarisProfilePageContentQuery";
    let url = format!(
        "https://www.instagram.com/api/graphql?doc_id={}&lsd={}&fb_dtsg={}&fb_api_req_friendly_name={}&variables={}",
        avatar_percent_encode(doc_id),
        avatar_percent_encode(lsd),
        avatar_percent_encode(dtsg),
        avatar_percent_encode(friendly_name),
        avatar_percent_encode(&variables),
    );
    let mut request = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "*/*")
        .header(reqwest::header::COOKIE, cookie_header)
        .header(reqwest::header::REFERER, referer)
        .header(reqwest::header::USER_AGENT, user_agent)
        .header("x-csrftoken", csrf_token)
        .header("x-ig-www-claim", ig_www_claim)
        .header("x-fb-friendly-name", friendly_name)
        .header("x-fb-lsd", lsd);
    if let Some(value) = ig_app_id {
        request = request.header("x-ig-app-id", value);
    }
    if let Some(value) = ig_asbd_id {
        request = request.header("x-asbd-id", value);
    }
    let response = request.send().map_err(|error| {
        ProfilePictureRefreshError::warning(format!(
            "Failed to fetch Instagram profile metadata via GraphQL: {error}"
        ))
    })?;
    let status = response.status();
    let body_text = response.text().unwrap_or_default();

    if !status.is_success() {
        let detail =
            avatar_error_detail(&body_text, ig_app_id, ig_asbd_id, ig_www_claim, csrf_token);
        return Err(ProfilePictureRefreshError::warning(format!(
            "Instagram profile GraphQL request failed with status {status}."
        ))
        .with_detail(detail));
    }

    let payload: serde_json::Value = serde_json::from_str(&body_text).map_err(|error| {
        ProfilePictureRefreshError::warning(format!(
            "Failed to parse Instagram profile GraphQL payload: {error}"
        ))
    })?;
    parse_instagram_profile_picture_url(&payload).ok_or_else(|| {
        ProfilePictureRefreshError::warning(
            "Instagram profile GraphQL response did not include a profile picture URL.",
        )
    })
}

fn avatar_error_detail(
    response_body: &str,
    sent_app_id: Option<&str>,
    sent_asbd_id: Option<&str>,
    sent_ig_www_claim: &str,
    sent_csrf_token: &str,
) -> String {
    let truncated_body: String = response_body.chars().take(2000).collect();
    serde_json::json!({
        "response_body": truncated_body,
        "sent_app_id": sent_app_id.unwrap_or("(none)"),
        "sent_asbd_id": sent_asbd_id.unwrap_or("(none)"),
        "sent_ig_www_claim": sent_ig_www_claim,
        "sent_csrf_token_len": sent_csrf_token.len(),
    })
    .to_string()
}

fn avatar_percent_encode(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
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

struct TopSearchUserResult {
    user_id: String,
    profile_pic_url: Option<String>,
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

fn build_cookie_header(cookies: &[CapturedBrowserCookie]) -> String {
    cookies
        .iter()
        .filter_map(|cookie| {
            let name = cookie.name.trim();
            let value = cookie.value.trim();
            if name.is_empty() || value.is_empty() {
                return None;
            }

            Some(format!("{name}={value}"))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn setting_value(settings: &HashMap<String, String>, key: &str) -> Option<String> {
    settings
        .get(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn configure_background_command(command: &mut Command) {
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

fn update_source_profile_image(
    connection: &Connection,
    source_id: &str,
    image_path: &str,
    timestamp: &str,
) -> Result<(), String> {
    connection
        .execute(
            "UPDATE source_profiles
             SET profile_image_path = ?2, updated_at = ?3
             WHERE id = ?1",
            params![source_id, image_path, timestamp],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
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
        if sanitize_source_handle("instagram", &old_handle).eq_ignore_ascii_case(&normalized_handle) {
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

fn convert_captured_cookies_to_provider_cookies(
    cookies: Vec<CapturedBrowserCookie>,
) -> Vec<ProviderAccountCookie> {
    cookies
        .into_iter()
        .map(|cookie| ProviderAccountCookie {
            domain: cookie.domain,
            name: cookie.name,
            value: cookie.value,
            path: cookie.path,
            expires_at: cookie.expires_at,
            secure: cookie.secure,
            http_only: cookie.http_only,
        })
        .collect()
}

fn convert_provider_cookies_to_captured_cookies(
    cookies: Vec<ProviderAccountCookie>,
) -> Vec<CapturedBrowserCookie> {
    cookies
        .into_iter()
        .map(|cookie| CapturedBrowserCookie {
            domain: cookie.domain.trim().to_string(),
            name: cookie.name.trim().to_string(),
            value: cookie.value,
            path: normalize_cookie_path(&cookie.path),
            expires_at: cookie
                .expires_at
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            secure: cookie.secure,
            http_only: cookie.http_only,
        })
        .collect()
}

fn persist_provider_account_session_payload(
    connection: &Connection,
    layout: &StorageLayout,
    account_id: &str,
    secret_payload: &str,
) -> Result<(), String> {
    let previous_secret_ref = load_account_session_secret_ref(connection, account_id)?;
    let backup_secret_ref = load_account_import_backup_secret_ref(connection, account_id)?;
    let secret_ref = format!("session-{}-{}", account_id, Uuid::new_v4());
    let imported_at = now_timestamp();
    session_secret_store::store_secret(layout, &secret_ref, secret_payload)?;
    if let Err(error) = write_provider_account_session_record(
        connection, account_id, &secret_ref, secret_payload, &imported_at,
    ) {
        let _ = session_secret_store::delete_secret(layout, &secret_ref);
        return Err(error);
    }
    if let Some(previous) = previous_secret_ref {
        if Some(previous.as_str()) != backup_secret_ref.as_deref() {
            let _ = session_secret_store::delete_secret(layout, &previous);
        }
    }
    Ok(())
}

fn serialize_session_payload_for_storage(
    cookies: &[CapturedBrowserCookie],
    current_url: Option<&str>,
    metadata: Option<&CapturedBrowserMetadata>,
) -> Result<String, String> {
    if cookies.is_empty() {
        return Err("Cookie collection cannot be empty.".to_string());
    }

    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct CookieEnvelope {
        #[serde(skip_serializing_if = "Option::is_none")]
        current_url: Option<String>,
        #[serde(default)]
        metadata: CapturedBrowserMetadata,
        cookies: Vec<CapturedBrowserCookie>,
    }

    serde_json::to_string(&CookieEnvelope {
        current_url: current_url.map(str::to_string),
        metadata: metadata.cloned().unwrap_or_default(),
        cookies: cookies.to_vec(),
    })
    .map_err(|error| error.to_string())
}

#[derive(Default)]
struct ParsedCookieImportContent {
    current_url: Option<String>,
    metadata: CapturedBrowserMetadata,
    cookies: Vec<CapturedBrowserCookie>,
}

fn parse_cookie_import_content(
    import_format: &str,
    content: &str,
) -> Result<ParsedCookieImportContent, String> {
    match import_format {
        "json" => parse_session_payload(content).map(|payload| ParsedCookieImportContent {
            current_url: payload.current_url,
            metadata: payload.metadata,
            cookies: payload.cookies,
        }),
        "netscape" => {
            parse_netscape_cookie_text(content).map(|cookies| ParsedCookieImportContent {
                current_url: None,
                metadata: CapturedBrowserMetadata::default(),
                cookies,
            })
        }
        other => Err(format!(
            "Cookie import format '{}' is not supported.",
            other
        )),
    }
}

fn parse_netscape_cookie_text(content: &str) -> Result<Vec<CapturedBrowserCookie>, String> {
    let mut cookies = Vec::new();

    for (index, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed == "# Netscape HTTP Cookie File" {
            continue;
        }

        let http_only = trimmed.starts_with("#HttpOnly_");
        if trimmed.starts_with('#') && !http_only {
            continue;
        }

        let normalized = if http_only {
            trimmed.replacen("#HttpOnly_", "", 1)
        } else {
            trimmed.to_string()
        };
        let parts = normalized.split('\t').collect::<Vec<_>>();
        if parts.len() < 7 {
            return Err(format!("Netscape cookie line {} is malformed.", index + 1));
        }

        cookies.push(CapturedBrowserCookie {
            domain: parts[0].trim().to_string(),
            name: parts[5].trim().to_string(),
            value: parts[6].to_string(),
            path: normalize_cookie_path(parts[2]),
            expires_at: parse_netscape_expiry(parts[4].trim()),
            secure: parts[3].trim().eq_ignore_ascii_case("TRUE"),
            http_only,
        });
    }

    if cookies.is_empty() {
        return Err("Imported cookie file does not contain any cookies.".to_string());
    }

    Ok(cookies)
}

fn parse_netscape_expiry(value: &str) -> Option<String> {
    if value.is_empty() || value == "0" {
        return None;
    }

    value
        .parse::<i64>()
        .ok()
        .and_then(|timestamp| DateTime::<Utc>::from_timestamp(timestamp, 0))
        .map(|timestamp| timestamp.to_rfc3339())
}

fn normalize_cookie_path(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

fn validate_captured_cookies(cookies: &[CapturedBrowserCookie]) -> Result<(), String> {
    for cookie in cookies {
        if cookie.domain.trim().is_empty() {
            return Err("Cookie domain cannot be empty.".to_string());
        }
        if cookie.name.trim().is_empty() {
            return Err("Cookie name cannot be empty.".to_string());
        }
    }

    Ok(())
}

fn write_netscape_cookie_file(
    path: &PathBuf,
    cookies: &[CapturedBrowserCookie],
) -> Result<(), String> {
    let mut lines = vec!["# Netscape HTTP Cookie File".to_string()];

    for cookie in cookies {
        let domain = if cookie.http_only {
            format!("#HttpOnly_{}", cookie.domain)
        } else {
            cookie.domain.clone()
        };
        let include_subdomains = if cookie.domain.starts_with('.') {
            "TRUE"
        } else {
            "FALSE"
        };
        let secure = if cookie.secure { "TRUE" } else { "FALSE" };
        let expires = cookie
            .expires_at
            .as_deref()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|timestamp| timestamp.timestamp().to_string())
            .unwrap_or_else(|| "0".to_string());
        let path_value = if cookie.path.trim().is_empty() {
            "/".to_string()
        } else {
            cookie.path.clone()
        };

        lines.push(format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            domain, include_subdomains, path_value, secure, expires, cookie.name, cookie.value
        ));
    }

    fs::write(path, lines.join("\n")).map_err(|error| error.to_string())
}

#[derive(Default)]
struct ParsedSessionPayload {
    current_url: Option<String>,
    metadata: CapturedBrowserMetadata,
    cookies: Vec<CapturedBrowserCookie>,
}

fn parse_session_payload(secret_payload: &str) -> Result<ParsedSessionPayload, String> {
    #[derive(serde::Deserialize)]
    struct SessionCookiePayload {
        domain: String,
        name: String,
        value: String,
        #[serde(default = "default_cookie_path")]
        path: String,
        #[serde(alias = "expiresAt", alias = "expires_at")]
        expires_at: Option<String>,
        #[serde(default)]
        secure: bool,
        #[serde(default, alias = "httpOnly", alias = "http_only")]
        http_only: bool,
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct SessionCookieEnvelope {
        #[serde(default)]
        current_url: Option<String>,
        #[serde(default)]
        metadata: CapturedBrowserMetadata,
        cookies: Vec<SessionCookiePayload>,
    }

    fn convert_cookie(cookie: SessionCookiePayload) -> CapturedBrowserCookie {
        CapturedBrowserCookie {
            domain: cookie.domain,
            name: cookie.name,
            value: cookie.value,
            path: cookie.path,
            expires_at: cookie.expires_at,
            secure: cookie.secure,
            http_only: cookie.http_only,
        }
    }

    fn parse_browser_session_payload(secret_payload: &str) -> Result<ParsedSessionPayload, String> {
        if let Ok(envelope) = serde_json::from_str::<SessionCookieEnvelope>(secret_payload) {
            return Ok(ParsedSessionPayload {
                current_url: envelope.current_url,
                metadata: envelope.metadata,
                cookies: envelope
                    .cookies
                    .into_iter()
                    .map(convert_cookie)
                    .collect::<Vec<_>>(),
            });
        }

        if let Ok(items) = serde_json::from_str::<Vec<SessionCookiePayload>>(secret_payload) {
            return Ok(ParsedSessionPayload {
                current_url: None,
                metadata: CapturedBrowserMetadata::default(),
                cookies: items.into_iter().map(convert_cookie).collect::<Vec<_>>(),
            });
        }

        Err("Stored session payload is not a supported cookie JSON shape.".to_string())
    }

    let payload = parse_browser_session_payload(secret_payload)?;
    if payload.cookies.is_empty() {
        return Err("Stored session payload does not contain any cookies.".to_string());
    }

    Ok(payload)
}

fn parse_session_cookies(secret_payload: &str) -> Result<Vec<CapturedBrowserCookie>, String> {
    parse_session_payload(secret_payload).map(|payload| payload.cookies)
}

fn default_cookie_path() -> String {
    "/".to_string()
}

fn domain_matches_allowed(domain: &str, allowed: &str) -> bool {
    let normalized_domain = domain.trim().trim_matches('.').to_ascii_lowercase();
    let normalized_allowed = allowed.trim().trim_matches('.').to_ascii_lowercase();
    normalized_domain == normalized_allowed
        || normalized_domain.ends_with(&format!(".{normalized_allowed}"))
}

fn validate_session_payload_for_account(
    connection: &Connection,
    account_id: &str,
    provider: &str,
    _auth_mode: &str,
    _session_format: &str,
    secret_payload: &str,
) -> Result<(), String> {
    if provider.eq_ignore_ascii_case("instagram") {
        return validate_instagram_manual_session_payload(connection, account_id, secret_payload);
    }

    Ok(())
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

fn session_fingerprint(secret_payload: &str) -> String {
    let mut hasher = DefaultHasher::new();
    secret_payload.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn is_expired_timestamp(value: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp < Utc::now())
        .unwrap_or(false)
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

/// Move o conteúdo de uma pasta para outra: tenta `rename` (rápido, mesmo
/// volume) e recorre a cópia recursiva + remoção quando o destino está em outro
/// volume. Os relative_path dos ledgers são relativos à pasta do perfil, então
/// mover a pasta mantém o histórico de downloads consistente.
fn move_media_directory(from: &Path, to: &Path) -> Result<(), String> {
    if to.exists() {
        return Err(format!(
            "Destino de mídia já existe: {}. Mova ou remova-o antes.",
            to.display()
        ));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    if fs::rename(from, to).is_ok() {
        return Ok(());
    }

    copy_dir_recursive(from, to)?;
    fs::remove_dir_all(from).map_err(|error| {
        format!(
            "Mídia copiada para '{}', mas falhou ao remover a pasta antiga '{}': {}",
            to.display(),
            from.display(),
            error
        )
    })
}

fn copy_dir_recursive(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(from).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let source_path = entry.path();
        let target_path = to.join(entry.file_name());
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path).map_err(|error| {
                format!("Falha ao copiar '{}': {}", source_path.display(), error)
            })?;
        }
    }
    Ok(())
}

/// Muda o path de salvamento de um ou mais perfis do Instagram para
/// `target_base_path/<handle>`, opcionalmente movendo a mídia já baixada.
pub fn change_source_media_path(
    source_ids: Vec<String>,
    target_base_path: String,
    move_media: bool,
) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        let base = PathBuf::from(target_base_path.trim());
        if base.as_os_str().is_empty() || !base.is_absolute() {
            return Err("O novo caminho de salvamento deve ser absoluto.".to_string());
        }

        let now = now_timestamp();
        let sources_by_id: HashMap<String, SourceProfile> = load_sources(connection)?
            .into_iter()
            .map(|source| (source.id.clone(), source))
            .collect();
        for source_id in &source_ids {
            let Some(source) = sources_by_id.get(source_id) else {
                continue;
            };
            if !source.provider.eq_ignore_ascii_case("instagram") {
                continue;
            }

            let account_settings = source
                .account_id
                .as_ref()
                .map(|account_id| load_provider_account_settings_map(connection, account_id))
                .transpose()?;
            let options = source_instagram_sync_options(&source);
            let old_root = resolve_instagram_profile_root_with_options(
                layout,
                &source,
                account_settings.as_ref(),
                Some(&options),
            );

            let folder = sanitize_path_segment(
                sanitize_source_handle("instagram", &source.handle)
                    .trim_start_matches('@'),
            );
            let new_root = base.join(&folder);
            if new_root == old_root {
                continue;
            }

            // No Windows os paths são case-insensitive: se origem e destino
            // canonicalizam para a mesma pasta física (ex.: `instagram` vs
            // `Instagram`), não há nada a mover — só o specialPath é atualizado
            // para a grafia nova.
            let same_physical_dir = match (old_root.canonicalize(), new_root.canonicalize()) {
                (Ok(old_canonical), Ok(new_canonical)) => old_canonical == new_canonical,
                _ => false,
            };

            if move_media && !same_physical_dir && old_root.exists() {
                move_media_directory(&old_root, &new_root)?;
            }

            let mut sync_options = source.sync_options.clone();
            let instagram = sync_options
                .instagram
                .get_or_insert_with(default_instagram_source_sync_options);
            instagram.special_path = Some(new_root.display().to_string());
            let serialized = serialize_source_sync_options("instagram", &sync_options)?;

            // A foto de perfil mora dentro da pasta movida (Settings/...); o
            // path absoluto persistido precisa acompanhar a mudança, senão a UI
            // perde o thumbnail.
            let updated_image_path = source.profile_image_path.as_deref().and_then(|image| {
                let normalized = image.trim_start_matches(r"\\?\");
                let old_prefix = old_root.display().to_string();
                let relative = Path::new(normalized).strip_prefix(&old_prefix).ok()?;
                let candidate = new_root.join(relative);
                candidate
                    .exists()
                    .then(|| format!(r"\\?\{}", candidate.display()))
            });

            match updated_image_path {
                Some(image_path) => {
                    connection
                        .execute(
                            "UPDATE source_profiles
                             SET sync_options_json = ?2,
                                 profile_image_path = ?4,
                                 updated_at = ?3
                             WHERE id = ?1
                               AND deleted_at IS NULL",
                            params![source_id, serialized, now, image_path],
                        )
                        .map_err(|error| error.to_string())?;
                }
                None => {
                    connection
                        .execute(
                            "UPDATE source_profiles
                             SET sync_options_json = ?2,
                                 updated_at = ?3
                             WHERE id = ?1
                               AND deleted_at IS NULL",
                            params![source_id, serialized, now],
                        )
                        .map_err(|error| error.to_string())?;
                }
            }
        }

        load_snapshot(connection, layout)
    })
}

/// Resolve o path absoluto de salvamento de mídia de cada perfil do Instagram,
/// reusando a mesma lógica do sync (specialPath > mediaPath da conta > media_root
/// global + handle). Erros de leitura de settings degradam para o root global.
fn compute_source_media_paths(
    connection: &Connection,
    layout: &StorageLayout,
    sources: &[SourceProfile],
) -> HashMap<String, String> {
    let mut account_settings_cache: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut result = HashMap::new();

    for source in sources {
        if !source.provider.eq_ignore_ascii_case("instagram") {
            continue;
        }

        let settings = source.account_id.as_ref().map(|account_id| {
            account_settings_cache
                .entry(account_id.clone())
                .or_insert_with(|| {
                    load_provider_account_settings_map(connection, account_id).unwrap_or_default()
                })
                .clone()
        });

        let options = source_instagram_sync_options(source);
        let root = resolve_instagram_profile_root_with_options(
            layout,
            source,
            settings.as_ref(),
            Some(&options),
        );
        // Canonicaliza para a grafia real do disco (Windows é case-insensitive),
        // unificando variações como `instagram` vs `Instagram` no filtro da UI.
        let display = root
            .canonicalize()
            .map(|canonical| {
                canonical
                    .display()
                    .to_string()
                    .trim_start_matches(r"\\?\")
                    .to_string()
            })
            .unwrap_or_else(|_| root.display().to_string());
        result.insert(source.id.clone(), display);
    }

    result
}

fn migrate_legacy_setting_keys(connection: &Connection) -> Result<(), String> {
    let mappings = [
        ("yt_dlp_path", "tool.yt-dlp.path"),
        ("gallery_dl_path", "tool.gallery-dl.path"),
        ("media_root", "storage.media_root"),
        ("notification_mode", "policy.notifications.default"),
    ];

    for (legacy_key, canonical_key) in mappings {
        let legacy_value = connection
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1 LIMIT 1",
                params![legacy_key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;

        let Some(value) = legacy_value else {
            continue;
        };

        let now = now_timestamp();
        connection
            .execute(
                "INSERT INTO app_settings (key, value, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET
                   value = excluded.value,
                   updated_at = excluded.updated_at",
                params![canonical_key, value, now],
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "DELETE FROM app_settings WHERE key = ?1",
                params![legacy_key],
            )
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn seed_missing_app_settings(
    connection: &Connection,
    layout: &StorageLayout,
) -> Result<(), String> {
    let now = now_timestamp();
    let defaults = [
        ("tool.yt-dlp.path", "yt-dlp".to_string()),
        ("tool.gallery-dl.path", "gallery-dl".to_string()),
        ("tool.instaloader.path", "instaloader".to_string()),
        ("policy.notifications.default", "summary".to_string()),
        (
            "naming.instagram.media_file_pattern_mode",
            "preset_new_default".to_string(),
        ),
        (
            "naming.instagram.media_file_pattern_template",
            "{datetime} {provider_media_key}.{ext}".to_string(),
        ),
        (DESKTOP_CLOSE_TO_TRAY_SETTING_KEY, "true".to_string()),
        (DESKTOP_SILENT_MODE_SETTING_KEY, "false".to_string()),
        ("policy.session_import.enabled", "true".to_string()),
        (DUPLICATE_USER_ID_BLOCK_SETTING_KEY, "true".to_string()),
        (SYNC_DELAY_BETWEEN_PROFILES_SETTING_KEY, "0".to_string()),
        (
            "instagram.sync.globalPreset1",
            r#"{"enabled":false,"label":"Preset 1","sections":{"timeline":true,"reels":false,"stories":false,"storiesUser":false,"tagged":false}}"#
                .to_string(),
        ),
        (
            "instagram.sync.globalPreset2",
            r#"{"enabled":false,"label":"Preset 2","sections":{"timeline":true,"reels":false,"stories":false,"storiesUser":false,"tagged":false}}"#
                .to_string(),
        ),
        (
            "storage.media_root",
            layout.media_root.display().to_string(),
        ),
    ];

    for (key, value) in defaults {
        connection
            .execute(
                "INSERT INTO app_settings (key, value, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO NOTHING",
                params![key, value, now.clone()],
            )
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn load_accounts(connection: &Connection) -> Result<Vec<ProviderAccount>, String> {
    let mut statement = connection.prepare("SELECT id, provider, display_name, auth_mode, auth_state, capabilities_json, last_validated_at FROM provider_accounts ORDER BY provider, display_name").map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(ProviderAccount {
                id: row.get(0)?,
                provider: row.get(1)?,
                display_name: row.get(2)?,
                auth_mode: row.get(3)?,
                auth_state: row.get(4)?,
                capabilities: from_json_array(row.get::<_, String>(5)?),
                last_validated_at: row.get(6)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn load_account_sessions(
    connection: &Connection,
    layout: &StorageLayout,
) -> Result<Vec<ProviderAccountSession>, String> {
    let mut statement = connection
        .prepare(
            "SELECT
                account_id,
                auth_mode,
                session_format,
                session_hint,
                fingerprint,
                secret_ref,
                expires_at,
                imported_at,
                last_validated_at,
                last_validation_error
             FROM provider_account_sessions
             ORDER BY imported_at DESC",
        )
        .map_err(|error| error.to_string())?;

    let rows = statement
        .query_map([], |row| {
            let _: String = row.get(3)?;
            Ok(ProviderAccountSessionRecord {
                account_id: row.get(0)?,
                auth_mode: row.get(1)?,
                session_format: row.get(2)?,
                fingerprint: row.get(4)?,
                secret_ref: row.get(5)?,
                expires_at: row.get(6)?,
                imported_at: row.get(7)?,
                last_validated_at: row.get(8)?,
                last_validation_error: row.get(9)?,
            })
        })
        .map_err(|error| error.to_string())?;

    let mut sessions = Vec::new();
    for row in rows {
        let record = row.map_err(|error| error.to_string())?;
        sessions.push(hydrate_account_session(layout, record)?);
    }

    Ok(sessions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::{ImportResolution, InstagramExtractImageFromVideoPatch};
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn derive_post_metadata_tiktok_tokkit_video() {
        let d = derive_post_metadata("tiktok", "gaaby.tls_1775147243_7624199329925958920.mp4", None)
            .expect("derived");
        assert_eq!(d.post_id.as_deref(), Some("7624199329925958920"));
        assert_eq!(d.media_type, "video");
        assert!(d.captured_at.is_some());
    }

    #[test]
    fn derive_post_metadata_tiktok_slideshow_groups_by_post() {
        let d = derive_post_metadata(
            "tiktok",
            "reeh_dmris_1703197051_7315175620856581381_index_0_2.jpeg",
            None,
        )
        .expect("derived");
        assert_eq!(d.post_id.as_deref(), Some("7315175620856581381"));
        assert_eq!(d.index, Some(0));
        assert_eq!(d.group_key, "7315175620856581381");
        assert_eq!(d.media_type, "image");
    }

    #[test]
    fn build_post_url_tiktok_video_vs_photo_and_profile() {
        assert_eq!(
            build_post_url("tiktok", "reeh_dmris", Some("7252779904704564486"), true, None)
                .as_deref(),
            Some("https://www.tiktok.com/@reeh_dmris/video/7252779904704564486")
        );
        assert_eq!(
            build_post_url("tiktok", "@reeh_dmris", Some("7315175620856581381"), false, None)
                .as_deref(),
            Some("https://www.tiktok.com/@reeh_dmris/photo/7315175620856581381")
        );
        assert_eq!(source_target_url("tiktok", "reeh_dmris"), "https://www.tiktok.com/@reeh_dmris");
    }

    #[test]
    fn twitter_media_key_strips_date_gif_and_extension() {
        assert_eq!(
            twitter_media_key_from_file_name("2026-06-19 16.44.17 hlm3jgqxsaajvu-.jpg").as_deref(),
            Some("hlm3jgqxsaajvu-")
        );
        // GIF_ prefix (and casing) is normalized to match the XML File basename.
        assert_eq!(
            twitter_media_key_from_file_name("2025-11-10 15.11.32 GIF_G5aakG1WoAA2yHs.mp4").as_deref(),
            Some("g5aakg1woaa2yhs")
        );
        // Raw SCrawler name without a date prefix.
        assert_eq!(
            twitter_media_key_from_file_name("Ghmf7p4asAA3qXa.jpg").as_deref(),
            Some("ghmf7p4asaa3qxa")
        );
    }

    #[test]
    fn build_post_url_twitter_uses_status_id() {
        assert_eq!(
            build_post_url("twitter", "@someone", Some("1700000000000000001"), false, None)
                .as_deref(),
            Some("https://x.com/someone/status/1700000000000000001")
        );
        // Sem id não há link.
        assert_eq!(build_post_url("twitter", "someone", None, false, None), None);
    }

    #[test]
    fn backfill_twitter_post_keys_fills_only_missing() {
        let conn = rusqlite::Connection::open_in_memory().expect("db");
        conn.execute_batch(
            "CREATE TABLE provider_sync_media_ledger (
                provider TEXT, source_id TEXT, account_id TEXT, source_handle TEXT,
                provider_media_key TEXT, media_type TEXT, media_section TEXT, relative_path TEXT,
                provider_post_key TEXT, captured_at INTEGER, first_seen_at TEXT, last_seen_at TEXT,
                PRIMARY KEY (provider, source_id, provider_media_key, media_type));
             INSERT INTO provider_sync_media_ledger VALUES
                ('twitter','s1','a','h','2068','image','media','2026 x.jpg', NULL, NULL, 't0','t0'),
                ('twitter','s1','a','h','9999','image','media','y.jpg', 'KEEP', NULL, 't0','t0');",
        )
        .expect("seed");

        let links = vec![
            twitter_connector::TwitterMediaPostLink {
                provider_media_key: "2068".into(),
                provider_post_key: "111".into(),
                media_section: "media".into(),
                captured_at_timestamp: Some(123),
            },
            twitter_connector::TwitterMediaPostLink {
                provider_media_key: "9999".into(),
                provider_post_key: "222".into(),
                media_section: "media".into(),
                captured_at_timestamp: Some(456),
            },
        ];
        backfill_provider_sync_media_ledger_post_keys(&conn, "twitter", "s1", &links, "t1")
            .expect("backfill");

        // Missing key gets filled (with captured_at); existing key is preserved.
        let filled: (Option<String>, Option<i64>) = conn
            .query_row(
                "SELECT provider_post_key, captured_at FROM provider_sync_media_ledger WHERE provider_media_key='2068'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(filled.0.as_deref(), Some("111"));
        assert_eq!(filled.1, Some(123));
        let kept: Option<String> = conn
            .query_row(
                "SELECT provider_post_key FROM provider_sync_media_ledger WHERE provider_media_key='9999'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept.as_deref(), Some("KEEP"));
    }

    #[test]
    fn build_post_url_instagram_uses_case_sensitive_shortcode() {
        // O shortcode mantém o casing original (case-sensitive).
        assert_eq!(
            build_post_url("instagram", "someone", None, false, Some("CyAbC-1_x")).as_deref(),
            Some("https://www.instagram.com/p/CyAbC-1_x/")
        );
        // Sem shortcode, sem link de post (cai para o perfil no front).
        assert_eq!(
            build_post_url("instagram", "someone", Some("123"), false, None),
            None
        );
    }

    #[test]
    fn extract_post_tombstone_keys_per_provider() {
        let post = |url: Option<&str>, id: Option<&str>| MediaGalleryPost {
            post_id: id.map(str::to_string),
            post_url: url.map(str::to_string),
            captured_at: None,
            media_type: "image".to_string(),
            section: "timeline".to_string(),
            albums: Vec::new(),
            poster_path: None,
            files: Vec::new(),
        };
        // TikTok: usa o post id.
        assert_eq!(
            extract_post_tombstone_keys(
                "tiktok",
                &post(Some("https://www.tiktok.com/@h/video/123"), Some("123")),
            ),
            (Some("123".to_string()), None)
        );
        // Twitter: status id do post_url.
        assert_eq!(
            extract_post_tombstone_keys("twitter", &post(Some("https://x.com/h/status/999"), None)),
            (Some("999".to_string()), None)
        );
        // Instagram: shortcode (case-sensitive) do post_url.
        assert_eq!(
            extract_post_tombstone_keys(
                "instagram",
                &post(Some("https://www.instagram.com/p/CyAbC-1_x/"), None),
            ),
            (None, Some("CyAbC-1_x".to_string()))
        );
        // Sem URL: nada a tombstonar via post ledger.
        assert_eq!(
            extract_post_tombstone_keys("twitter", &post(None, None)),
            (None, None)
        );
    }

    #[test]
    fn extract_instagram_post_code_preserves_casing_for_url() {
        let permalink = "https://www.instagram.com/p/CyAbC-1_x/";
        assert_eq!(
            extract_instagram_post_code_from_permalink_cased(permalink).as_deref(),
            Some("CyAbC-1_x")
        );
        // A variante normalizada (dedupe) continua lowercased.
        assert_eq!(
            extract_instagram_post_code_from_permalink(permalink).as_deref(),
            Some("cyabc-1_x")
        );
    }

    #[test]
    fn is_profile_image_file_excludes_avatar_and_profile_picture() {
        assert!(is_profile_image_file(
            "reeh_dmris_0_7318182511312371717_avatar.jpeg"
        ));
        assert!(is_profile_image_file("ProfilePicture.jpg"));
        assert!(!is_profile_image_file(
            "reeh_dmris_1703197051_7315175620856581381_index_0_2.jpeg"
        ));
    }

    fn create_test_layout() -> (TempDir, StorageLayout) {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let local_app_data = temp_dir.path().join("localappdata");
        let user_profile = temp_dir.path().join("userprofile");
        let layout =
            storage::workspace_layout_from_roots(local_app_data, user_profile).expect("layout");
        (temp_dir, layout)
    }

    fn sample_account(id: &str, provider: &str) -> ProviderAccountUpsert {
        ProviderAccountUpsert {
            id: Some(id.to_string()),
            provider: provider.to_string(),
            display_name: format!("{provider}-account"),
            auth_mode: "imported_session".to_string(),
            auth_state: "ready".to_string(),
            capabilities: vec!["posts".to_string()],
            last_validated_at: Some("2026-03-10T00:00:00Z".to_string()),
        }
    }

    fn sample_source(id: &str, provider: &str, account_id: Option<&str>) -> SourceProfileUpsert {
        SourceProfileUpsert {
            id: Some(id.to_string()),
            provider: provider.to_string(),
            source_kind: "profile".to_string(),
            handle: format!("@{id}"),
            display_name: id.to_string(),
            account_id: account_id.map(|value| value.to_string()),
            group_id: None,
            labels: vec!["priority".to_string()],
            ready_for_download: true,
            sync_options: default_source_sync_options(provider),
            remote_state: None,
            is_subscription: None,
        }
    }

    fn sample_source_profile_model() -> SourceProfile {
        SourceProfile {
            id: "source-1".to_string(),
            provider: "instagram".to_string(),
            source_kind: "profile".to_string(),
            handle: "@source-1".to_string(),
            display_name: "source-1".to_string(),
            account_id: Some("account-1".to_string()),
            group_id: None,
            labels: vec![],
            ready_for_download: true,
            sync_options: default_source_sync_options("instagram"),
            profile_image_path: None,
            profile_image_custom: false,
            remote_state: "exists".to_string(),
            is_subscription: false,
            last_synced_at: None,
            sync_problem_code: None,
            sync_problem_message: None,
            sync_problem_at: None,
            created_at: Some("2026-03-10T00:00:00Z".to_string()),
            importer_id: None,
            imported_at: None,
        }
    }

    fn sample_instagram_manifest_summary() -> instagram_connector::InstagramManifestSummary {
        instagram_connector::InstagramManifestSummary {
            section_count: 4,
            discovered_item_count: 4,
            normalized_post_count: 0,
            discovered_asset_count: 0,
            queued_asset_count: 0,
            skipped_existing_post_count: 4,
            skipped_duplicate_post_count: 0,
            skipped_unavailable_post_count: 0,
            skipped_existing_asset_count: 0,
            skipped_duplicate_asset_count: 0,
            downloaded_asset_count: 0,
            profile_user_id: None,
            sections: vec![],
        }
    }

    fn create_legacy_instagram_profile_root(
        profile_root: &Path,
        account_name: &str,
        user_name: &str,
        description: Option<&str>,
    ) -> Result<PathBuf, String> {
        create_legacy_instagram_profile_root_full(
            profile_root,
            account_name,
            user_name,
            None,
            None,
            description,
        )
    }

    fn create_legacy_instagram_profile_root_full(
        profile_root: &Path,
        account_name: &str,
        user_name: &str,
        true_name: Option<&str>,
        user_id: Option<&str>,
        description: Option<&str>,
    ) -> Result<PathBuf, String> {
        let settings_dir = profile_root.join("Settings");
        fs::create_dir_all(&settings_dir).map_err(|error| error.to_string())?;

        let true_name_value = true_name.unwrap_or(user_name);
        let description_tag = description
            .map(|value| format!("\n  <Description>{value}</Description>"))
            .unwrap_or_default();
        let user_id_tag = user_id
            .map(|value| format!("\n  <UserID>{value}</UserID>"))
            .unwrap_or_default();
        let user_xml = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
             <UserData>\n\
               <AccountName>{account_name}</AccountName>{user_id_tag}\n\
               <UserName>{user_name}</UserName>\n\
               <TrueName>{true_name_value}</TrueName>\n\
               <FriendlyName>{user_name}</FriendlyName>\n\
               <UserSiteName>{user_name}</UserSiteName>{description_tag}\n\
               <ReadyForDownload>true</ReadyForDownload>\n\
               <GetTimeline>true</GetTimeline>\n\
               <GetReels>false</GetReels>\n\
               <GetStories>false</GetStories>\n\
               <GetStoriesUser>false</GetStoriesUser>\n\
               <GetTaggedData>false</GetTaggedData>\n\
             </UserData>\n"
        );

        let user_xml_path = settings_dir.join("User_Instagram.xml");
        fs::write(&user_xml_path, user_xml).map_err(|error| error.to_string())?;
        fs::write(profile_root.join("first.jpg"), b"image").map_err(|error| error.to_string())?;
        Ok(user_xml_path)
    }

    fn create_legacy_instagram_data_xml(
        profile_root: &Path,
        file_name: &str,
        post_id: &str,
        special_folder: Option<&str>,
        media_url: &str,
        post_permalink: &str,
    ) -> Result<PathBuf, String> {
        let settings_dir = profile_root.join("Settings");
        fs::create_dir_all(&settings_dir).map_err(|error| error.to_string())?;

        let data_xml = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"yes\"?>\n\
             <Data>\n\
               <MediaData Attempts=\"0\" Date=\"2025-02-10 04:31:48\" File=\"{file_name}\" ID=\"{post_id}\" SpecialFolder=\"{}\" State=\"2\" Type=\"{}\" URL=\"{media_url}\">{post_permalink}</MediaData>\n\
             </Data>\n",
            special_folder.unwrap_or_default(),
            if file_name.to_ascii_lowercase().ends_with(".mp4") { "2" } else { "1" },
        );

        let file_stem = Path::new(file_name)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("legacy");
        let data_xml_path = settings_dir.join(format!("User_Instagram_{file_stem}_Data.xml"));
        fs::write(&data_xml_path, data_xml).map_err(|error| error.to_string())?;
        Ok(data_xml_path)
    }

    #[test]
    fn implicit_instagram_imported_cutoff_uses_materialized_metadata_and_force_mode_bypasses_it() {
        let mut source = sample_source_profile_model();
        source.importer_id = Some("instagram.scrawler".to_string());
        source.imported_at = Some("2026-03-20T12:00:00Z".to_string());

        let cutoff = implicit_instagram_imported_cutoff_timestamp(&source, None);
        assert_eq!(cutoff, Some(1_742_473_600));

        let bypassed =
            implicit_instagram_imported_cutoff_timestamp(&source, Some("force_imported_backfill"));
        assert_eq!(bypassed, None);
    }

    fn load_source_profile_by_id(
        connection: &Connection,
        source_id: &str,
    ) -> Result<SourceProfile, String> {
        connection
            .query_row(
            "SELECT provider, source_kind, handle, display_name, account_id, labels_json, ready_for_download, sync_options_json, profile_image_path, profile_image_custom, remote_state, is_subscription, last_synced_at, sync_problem_code, sync_problem_message, sync_problem_at, created_at, group_id, importer_id, imported_at
             FROM source_profiles
             WHERE id = ?1
               AND deleted_at IS NULL
             LIMIT 1",
                params![source_id],
                |row| {
                    let provider = row.get::<_, String>(0)?;
                    let labels_json = row.get::<_, String>(5)?;
                    let sync_options_json = row.get::<_, String>(7)?;
                    Ok(SourceProfile {
                        id: source_id.to_string(),
                        provider: provider.clone(),
                        source_kind: row.get(1)?,
                        handle: row.get(2)?,
                        display_name: row.get(3)?,
                        account_id: row.get(4)?,
                        group_id: row.get(17)?,
                        labels: serde_json::from_str(&labels_json).unwrap_or_default(),
                    ready_for_download: row.get::<_, i64>(6).unwrap_or(0) != 0,
                    sync_options: deserialize_source_sync_options(&provider, &sync_options_json),
                    profile_image_path: row.get(8)?,
                    profile_image_custom: row.get::<_, i64>(9).unwrap_or(0) != 0,
                    remote_state: row.get::<_, String>(10).unwrap_or_else(|_| "exists".to_string()),
                    is_subscription: row.get::<_, i64>(11).unwrap_or(0) != 0,
                    last_synced_at: row.get(12).ok(),
                    sync_problem_code: row.get(13).ok(),
                    sync_problem_message: row.get(14).ok(),
                    sync_problem_at: row.get(15).ok(),
                    created_at: row.get(16).ok(),
                    importer_id: row.get(18).ok(),
                    imported_at: row.get(19).ok(),
                })
            },
        )
            .map_err(|error| error.to_string())
    }

    #[test]
    fn record_external_import_ledger_updates_materialized_source_import_metadata() {
        let (_temp_dir, layout) = create_test_layout();
        let connection = database::open_connection(&layout.db_path).expect("connection");

        upsert_provider_account_with_connection(
            &connection,
            &layout,
            sample_account("account-1", "instagram"),
        )
        .expect("account should upsert");
        upsert_source_profile_with_connection(
            &connection,
            &layout,
            sample_source("source-1", "instagram", Some("account-1")),
        )
        .expect("source should upsert");

        record_external_import_ledger(
            &connection,
            INSTAGRAM_SCRAWLER_IMPORTER_ID,
            Path::new("D:/legacy/source-a"),
            "instagram",
            "@source-1",
            "source-1",
            "account-1",
            "2026-03-20T12:00:00Z",
        )
        .expect("first import metadata should persist");
        record_external_import_ledger(
            &connection,
            INSTAGRAM_SCRAWLER_IMPORTER_ID,
            Path::new("D:/legacy/source-b"),
            "instagram",
            "@source-1",
            "source-1",
            "account-1",
            "2026-03-22T15:30:00Z",
        )
        .expect("latest import metadata should persist");

        let source =
            load_source_profile_by_id(&connection, "source-1").expect("source should load");
        assert_eq!(
            source.importer_id.as_deref(),
            Some(INSTAGRAM_SCRAWLER_IMPORTER_ID)
        );
        assert_eq!(source.imported_at.as_deref(), Some("2026-03-22T15:30:00Z"));
    }

    #[test]
    fn manual_handle_change_is_supported_for_every_provider() {
        let (_temp_dir, layout) = create_test_layout();
        let connection = database::open_connection(&layout.db_path).expect("connection");

        let runs = |conn: &Connection, source_id: &str| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM source_sync_runs WHERE source_id = ?1 AND trigger = 'manual_handle_edit'",
                params![source_id],
                |row| row.get(0),
            )
            .expect("count")
        };

        for provider in ["instagram", "tiktok", "twitter"] {
            let account_id = format!("account-{provider}");
            let source_id = format!("source-{provider}");
            upsert_provider_account_with_connection(
                &connection,
                &layout,
                sample_account(&account_id, provider),
            )
            .expect("account should upsert");
            upsert_source_profile_with_connection(
                &connection,
                &layout,
                sample_source(&source_id, provider, Some(&account_id)),
            )
            .expect("source should upsert");

            assert_eq!(runs(&connection, &source_id), 0, "{provider}");

            let mut renamed = sample_source(&source_id, provider, Some(&account_id));
            renamed.handle = format!("@renamed-{provider}");
            upsert_source_profile_with_connection(&connection, &layout, renamed.clone())
                .expect("handle change should upsert");
            assert_eq!(runs(&connection, &source_id), 1, "{provider}");

            upsert_source_profile_with_connection(&connection, &layout, renamed)
                .expect("no-op resave should upsert");
            assert_eq!(runs(&connection, &source_id), 1, "{provider}");
        }

        let instagram_source =
            load_source_profile_by_id(&connection, "source-instagram").expect("Instagram source");
        let previous_handles = instagram_source
            .sync_options
            .instagram
            .and_then(|options| options.previous_handles)
            .unwrap_or_default();
        assert!(
            previous_handles
                .iter()
                .any(|handle| handle == "source-instagram"),
            "manual Instagram rename should preserve the previous handle"
        );
    }

    #[test]
    fn legacy_instagram_manifest_keeps_identity_hint_recoverable() {
        let (_temp_dir, layout) = create_test_layout();
        let connection = database::open_connection(&layout.db_path).expect("connection");
        upsert_provider_account_with_connection(
            &connection,
            &layout,
            sample_account("account-1", "instagram"),
        )
        .expect("account should upsert");
        upsert_source_profile_with_connection(
            &connection,
            &layout,
            sample_source("source-1", "instagram", Some("account-1")),
        )
        .expect("source should upsert");

        let legacy_summary = json!({
            "profileUserId": "80735443629",
            "sectionCount": 1,
            "discoveredItemCount": 1,
            "normalizedPostCount": 1,
            "discoveredAssetCount": 1,
            "queuedAssetCount": 1,
            "skippedExistingPostCount": 0,
            "skippedDuplicatePostCount": 0,
            "skippedUnavailablePostCount": 0,
            "skippedExistingAssetCount": 0,
            "skippedDuplicateAssetCount": 0,
            "downloadedAssetCount": 1,
            "sections": [{
                "section": "timeline",
                "label": "Timeline",
                "itemCount": 1,
                "normalizedPostCount": 1,
                "discoveredAssetCount": 1,
                "queuedAssetCount": 1,
                "skippedExistingPostCount": 0,
                "skippedDuplicatePostCount": 0,
                "skippedUnavailablePostCount": 0,
                "skippedExistingAssetCount": 0,
                "skippedDuplicateAssetCount": 0
            }]
        })
        .to_string();
        connection
            .execute(
                "INSERT INTO source_sync_runs (
                    id, source_id, account_id, provider, tool, trigger, status,
                    summary, command_preview, manifest_summary_json,
                    degraded_capabilities_json, started_at, finished_at, created_at
                 ) VALUES (
                    'run-legacy', 'source-1', 'account-1', 'instagram',
                    'internal.instagram', 'manual', 'succeeded', 'ok', 'test',
                    ?1, '[]', '2026-06-21T13:40:44Z',
                    '2026-06-21T13:41:01Z', '2026-06-21T13:41:01Z'
                 )",
                params![legacy_summary],
            )
            .expect("legacy run should insert");
        set_source_sync_problem(
            &connection,
            "source-1",
            "instagram_username_unresolvable",
            "legacy resolver could not recover the renamed profile",
            "2026-07-01T17:52:10Z",
            true,
        )
        .expect("legacy problem marker");

        let parsed = serde_json::from_str::<instagram_connector::InstagramManifestSummary>(
            &legacy_summary,
        )
        .expect("new summary fields must default when reading legacy history");
        assert_eq!(parsed.sections[0].skipped_out_of_range_item_count, 0);
        assert_eq!(
            load_latest_instagram_profile_user_id_hint(&connection, "source-1")
                .expect("history lookup"),
            Some("80735443629".to_string())
        );

        connection
            .execute_batch(include_str!(
                "../../migrations/0031_instagram_identity_hint_backfill.sql"
            ))
            .expect("identity backfill migration should be idempotent");
        let recovered_source = connection
            .query_row(
                "SELECT
                    json_extract(sync_options_json, '$.instagram.userIdHint'),
                    ready_for_download,
                    sync_problem_code
                 FROM source_profiles WHERE id = 'source-1'",
                [],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .expect("recovered source");
        assert_eq!(recovered_source.0.as_deref(), Some("80735443629"));
        assert_eq!(recovered_source.1, 1);
        assert_eq!(recovered_source.2, None);

        connection
            .execute(
                "UPDATE source_profiles
                 SET sync_options_json = json_set(
                        sync_options_json,
                        '$.instagram.userIdHint',
                        '59617797093'
                     ),
                     ready_for_download = 0,
                     sync_problem_code = 'instagram_username_unresolvable',
                     sync_problem_message = 'blocked by stale imported hint',
                     sync_problem_at = '2026-07-03T07:50:13Z'
                 WHERE id = 'source-1'",
                [],
            )
            .expect("stale imported identity should be simulated");
        connection
            .execute_batch(include_str!(
                "../../migrations/0033_instagram_identity_hint_reconcile.sql"
            ))
            .expect("identity reconciliation migration should run");
        let reconciled_source = connection
            .query_row(
                "SELECT
                    json_extract(sync_options_json, '$.instagram.userIdHint'),
                    ready_for_download,
                    sync_problem_code
                 FROM source_profiles WHERE id = 'source-1'",
                [],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .expect("reconciled source");
        assert_eq!(reconciled_source.0.as_deref(), Some("80735443629"));
        assert_eq!(reconciled_source.1, 1);
        assert_eq!(reconciled_source.2, None);
    }

    #[test]
    fn instagram_identity_hint_is_persisted_once_and_cannot_drift() {
        let (_temp_dir, layout) = create_test_layout();
        let connection = database::open_connection(&layout.db_path).expect("connection");
        upsert_provider_account_with_connection(
            &connection,
            &layout,
            sample_account("account-1", "instagram"),
        )
        .expect("account should upsert");
        upsert_source_profile_with_connection(
            &connection,
            &layout,
            sample_source("source-1", "instagram", Some("account-1")),
        )
        .expect("source should upsert");

        persist_instagram_user_id_hint(
            &connection,
            "source-1",
            "80735443629",
            "2026-07-02T00:00:00Z",
        )
        .expect("identity should persist");
        persist_instagram_user_id_hint(
            &connection,
            "source-1",
            "80735443629",
            "2026-07-02T00:01:00Z",
        )
        .expect("same identity should be idempotent");

        let mismatch = persist_instagram_user_id_hint(
            &connection,
            "source-1",
            "999999",
            "2026-07-02T00:02:00Z",
        )
        .expect_err("identity drift must be rejected");
        assert!(mismatch.contains("identity mismatch"));

        let source = load_source_profile_by_id(&connection, "source-1").expect("source");
        assert_eq!(
            source
                .sync_options
                .instagram
                .and_then(|options| options.user_id_hint)
                .as_deref(),
            Some("80735443629")
        );
    }

    #[test]
    fn instagram_identity_hint_prefers_confirmed_history_over_imported_hint() {
        assert_eq!(
            preferred_instagram_user_id_hint(Some("59617797093"), Some("74818949106"))
                .as_deref(),
            Some("74818949106")
        );
        assert_eq!(
            preferred_instagram_user_id_hint(Some("59617797093"), None).as_deref(),
            Some("59617797093")
        );
    }

    #[test]
    fn instagram_identity_hint_can_repair_imported_mismatch_confirmed_by_history() {
        let (_temp_dir, layout) = create_test_layout();
        let connection = database::open_connection(&layout.db_path).expect("connection");
        upsert_provider_account_with_connection(
            &connection,
            &layout,
            sample_account("account-1", "instagram"),
        )
        .expect("account should upsert");
        let mut source = sample_source("source-1", "instagram", Some("account-1"));
        source
            .sync_options
            .instagram
            .get_or_insert_with(default_instagram_source_sync_options)
            .user_id_hint = Some("59617797093".to_string());
        upsert_source_profile_with_connection(&connection, &layout, source)
            .expect("source should upsert");
        connection
            .execute(
                "INSERT INTO source_sync_runs (
                    id, source_id, account_id, provider, tool, trigger, status,
                    summary, command_preview, manifest_summary_json,
                    degraded_capabilities_json, started_at, finished_at, created_at
                 ) VALUES (
                    'run-confirmed', 'source-1', 'account-1', 'instagram',
                    'internal.instagram', 'manual', 'succeeded', 'ok', 'test',
                    '{\"profileUserId\":\"74818949106\"}', '[]',
                    '2026-06-30T04:04:26Z', '2026-06-30T04:04:39Z',
                    '2026-06-30T04:04:39Z'
                 )",
                [],
            )
            .expect("confirmed history should insert");

        persist_instagram_user_id_hint(
            &connection,
            "source-1",
            "74818949106",
            "2026-07-03T08:00:00Z",
        )
        .expect("confirmed history should repair the imported hint");

        let repaired = load_source_profile_by_id(&connection, "source-1")
            .expect("source")
            .sync_options
            .instagram
            .and_then(|options| options.user_id_hint);
        assert_eq!(repaired.as_deref(), Some("74818949106"));
    }

    fn sample_instagram_cookies() -> Vec<ProviderAccountCookie> {
        vec![
            ProviderAccountCookie {
                domain: ".instagram.com".to_string(),
                name: "sessionid".to_string(),
                value: "abc123".to_string(),
                path: "/".to_string(),
                expires_at: Some("2030-01-01T00:00:00Z".to_string()),
                secure: true,
                http_only: true,
            },
            ProviderAccountCookie {
                domain: ".instagram.com".to_string(),
                name: "csrftoken".to_string(),
                value: "csrf123".to_string(),
                path: "/".to_string(),
                expires_at: Some("2030-01-01T00:00:00Z".to_string()),
                secure: true,
                http_only: false,
            },
        ]
    }

    #[test]
    fn batch_update_source_profiles_rolls_back_when_any_source_is_invalid() {
        let (_temp_dir, layout) = create_test_layout();

        with_workspace_layout(layout.clone(), |connection, test_layout| {
            upsert_source_profile_with_connection(
                connection,
                test_layout,
                sample_source("source-1", "instagram", None),
            )
        })
        .expect("source setup");

        let result = with_workspace_layout(layout.clone(), |_connection, _| {
            batch_update_source_profiles(BatchSourceProfilePatch {
                source_ids: vec!["source-1".to_string(), "missing-source".to_string()],
                labels_to_add: vec!["batch-updated".to_string()],
                labels_to_remove: Vec::new(),
                ready_for_download: Some(false),
                sync_options_patch: None,
                set_group_id: None,
            })
        });

        assert!(
            result.is_err(),
            "batch update should fail for missing source"
        );

        let source = with_workspace_layout(layout, |connection, _| {
            load_source_profile_by_id(connection, "source-1")
        })
        .expect("source should remain available");

        assert!(
            !source.labels.iter().any(|label| label == "batch-updated"),
            "labels should not be partially applied after rollback"
        );
        assert!(
            source.ready_for_download,
            "ready-for-download should remain unchanged after rollback"
        );
    }

    #[test]
    fn batch_update_source_profiles_rejects_unknown_group_id_without_partial_changes() {
        let (_temp_dir, layout) = create_test_layout();

        with_workspace_layout(layout.clone(), |connection, test_layout| {
            upsert_source_profile_with_connection(
                connection,
                test_layout,
                sample_source("source-1", "instagram", None),
            )
        })
        .expect("source setup");

        let result = with_workspace_layout(layout.clone(), |_connection, _| {
            batch_update_source_profiles(BatchSourceProfilePatch {
                source_ids: vec!["source-1".to_string()],
                labels_to_add: vec!["batch-updated".to_string()],
                labels_to_remove: Vec::new(),
                ready_for_download: None,
                sync_options_patch: None,
                set_group_id: Some(Some("missing-group".to_string())),
            })
        });

        assert!(
            matches!(result, Err(message) if message.contains("Scheduler group not found")),
            "missing group should fail fast with a clear error"
        );

        let (group_id, labels): (Option<String>, Vec<String>) =
            with_workspace_layout(layout, |connection, _| {
                connection
                    .query_row(
                        "SELECT group_id, labels_json FROM source_profiles WHERE id = ?1",
                        params!["source-1"],
                        |row| {
                            let group_id: Option<String> = row.get(0)?;
                            let labels_json: String = row.get(1)?;
                            Ok((
                                group_id,
                                serde_json::from_str(&labels_json).unwrap_or_default(),
                            ))
                        },
                    )
                    .map_err(|error| error.to_string())
            })
            .expect("source should remain unchanged");

        assert!(group_id.is_none(), "group assignment should not be applied");
        assert!(
            !labels.iter().any(|label| label == "batch-updated"),
            "labels should remain unchanged after validation failure"
        );
    }

    #[test]
    fn batch_update_source_profiles_applies_group_and_sync_patch_when_inputs_are_valid() {
        let (_temp_dir, layout) = create_test_layout();

        with_workspace_layout(layout.clone(), |connection, test_layout| {
            upsert_source_profile_with_connection(
                connection,
                test_layout,
                sample_source("source-1", "instagram", None),
            )?;
            upsert_scheduler_group_with_connection(
                connection,
                SchedulerGroupUpsert {
                    id: Some("group-1".to_string()),
                    name: "Batch group".to_string(),
                    sort_index: Some(1),
                    criteria: SchedulerPlanCriteria::default(),
                },
            )
        })
        .expect("setup source and group");

        with_workspace_layout(layout.clone(), |_connection, _| {
            batch_update_source_profiles(BatchSourceProfilePatch {
                source_ids: vec!["source-1".to_string()],
                labels_to_add: Vec::new(),
                labels_to_remove: Vec::new(),
                ready_for_download: None,
                sync_options_patch: Some(InstagramSyncOptionsPatch {
                    timeline: None,
                    reels: None,
                    stories: None,
                    stories_user: None,
                    tagged: None,
                    temporary: Some(false),
                    favorite: None,
                    download_images: Some(false),
                    download_videos: None,
                    place_extracted_image_into_video_folder: None,
                    extract_image_from_video: None,
                    get_user_media_only: Some(true),
                    missing_only: None,
                    verified_profile: None,
                    force_update_user_name: None,
                    force_update_user_information: None,
                    download_text: None,
                    download_text_posts: None,
                }),
                set_group_id: Some(Some("group-1".to_string())),
            })
        })
        .expect("batch update should succeed");

        let source = with_workspace_layout(layout, |connection, _| {
            connection
                .query_row(
                    "SELECT group_id, sync_options_json FROM source_profiles WHERE id = ?1",
                    params!["source-1"],
                    |row| {
                        let group_id: Option<String> = row.get(0)?;
                        let sync_options_json: String = row.get(1)?;
                        Ok((group_id, sync_options_json))
                    },
                )
                .map_err(|error| error.to_string())
        })
        .expect("source should persist updates");

        assert_eq!(source.0.as_deref(), Some("group-1"));
        let sync_options = deserialize_source_sync_options("instagram", &source.1);
        let instagram = sync_options
            .instagram
            .expect("instagram sync options should exist after patch");
        assert_eq!(instagram.temporary, Some(false));
        assert_eq!(instagram.download_images, Some(false));
        assert_eq!(instagram.get_user_media_only, Some(true));
    }

    #[test]
    fn apply_instagram_patch_updates_media_agnostic_fields() {
        let mut options = InstagramSourceSyncOptions::default();
        let patch = InstagramSyncOptionsPatch {
            timeline: None,
            reels: None,
            stories: None,
            stories_user: None,
            tagged: None,
            temporary: None,
            favorite: None,
            download_images: None,
            download_videos: None,
            place_extracted_image_into_video_folder: Some(true),
            extract_image_from_video: Some(InstagramExtractImageFromVideoPatch {
                timeline: Some(false),
                reels: Some(false),
                stories: None,
                stories_user: Some(false),
                tagged: Some(true),
            }),
            get_user_media_only: None,
            missing_only: None,
            verified_profile: None,
            force_update_user_name: None,
            force_update_user_information: None,
            download_text: None,
            download_text_posts: None,
        };

        apply_instagram_patch(&mut options, &patch);

        assert_eq!(options.place_extracted_image_into_video_folder, Some(true));
        let extract = options.extract_image_from_video.expect("extract patch");
        assert!(!extract.timeline);
        assert!(!extract.reels);
        assert!(!extract.stories_user);
        assert!(extract.tagged);
        assert!(extract.stories);
    }

    #[test]
    fn download_success_summary_uses_short_copy_for_zero_items() {
        assert_eq!(
            format_download_success_summary("Instagram sync succeeded.", 0),
            "Instagram sync succeeded. No new media downloaded."
        );
        assert_eq!(
            format_download_success_summary("Saved posts sync succeeded.", 0),
            "Saved posts sync succeeded. No new media downloaded."
        );
        assert_eq!(
            format_download_success_summary("Instagram sync succeeded.", 3),
            "Instagram sync succeeded. Downloaded 3 media items."
        );
    }

    #[test]
    fn connector_sync_summary_preserves_degraded_capabilities() {
        assert_eq!(
            format_connector_sync_success_summary(0, &[]),
            "Connector sync succeeded. No new media downloaded."
        );
        assert_eq!(
            format_connector_sync_success_summary(0, &["stories".to_string()]),
            "Connector sync succeeded. No new media downloaded. Degraded capabilities: stories."
        );
        assert_eq!(
            format_connector_sync_success_summary(2, &["stories".to_string()]),
            "Connector sync succeeded. Downloaded 2 media items with degraded capabilities: stories."
        );
    }

    #[test]
    fn instagram_manifest_suffix_is_omitted_for_zero_download_success() {
        let manifest_summary = sample_instagram_manifest_summary();
        assert_eq!(
            format_instagram_manifest_suffix(Some(&manifest_summary), false),
            ""
        );
        assert_eq!(
            format_instagram_manifest_suffix(Some(&manifest_summary), true),
            " Manifest retained 0 posts and queued 0 assets across 4 sections after filtering 4 existing posts."
        );
    }

    #[test]
    fn instagram_manifest_suffix_omits_zero_value_filter_counts() {
        let manifest_summary = instagram_connector::InstagramManifestSummary {
            section_count: 4,
            discovered_item_count: 0,
            normalized_post_count: 12,
            discovered_asset_count: 0,
            queued_asset_count: 13,
            skipped_existing_post_count: 0,
            skipped_duplicate_post_count: 0,
            skipped_unavailable_post_count: 0,
            skipped_existing_asset_count: 0,
            skipped_duplicate_asset_count: 0,
            downloaded_asset_count: 0,
            profile_user_id: None,
            sections: vec![],
        };

        assert_eq!(
            format_instagram_manifest_suffix(Some(&manifest_summary), true),
            " Manifest retained 12 posts and queued 13 assets across 4 sections."
        );
    }

    fn seed_instagram_auth_settings(
        connection: &Connection,
        layout: &StorageLayout,
        account_id: &str,
    ) -> Result<(), String> {
        save_provider_account_settings_with_connection(
            connection,
            layout,
            account_id.to_string(),
            vec![ProviderAccountSettingValue {
                setting_key: "instagram.auth.appId".to_string(),
                value_kind: ProviderAccountSettingValueKind::String,
                string_value: Some("936619743392459".to_string()),
                json_value: None,
            }],
        )?;
        Ok(())
    }

    fn seed_instagram_session(
        connection: &Connection,
        layout: &StorageLayout,
        account_id: &str,
    ) -> Result<(), String> {
        seed_instagram_auth_settings(connection, layout, account_id)?;
        let _ = save_provider_account_cookies_with_connection(
            connection,
            layout,
            account_id,
            sample_instagram_cookies(),
        )?;
        Ok(())
    }

    fn string_provider_setting(key: &str, value: &str) -> ProviderAccountSettingValue {
        ProviderAccountSettingValue {
            setting_key: key.to_string(),
            value_kind: ProviderAccountSettingValueKind::String,
            string_value: Some(value.to_string()),
            json_value: None,
        }
    }

    fn sample_scheduler_set(id: &str, active: bool) -> SchedulerSetUpsert {
        SchedulerSetUpsert {
            id: Some(id.to_string()),
            name: format!("set-{id}"),
            active,
        }
    }

    fn sample_sync_plan(
        id: &str,
        scheduler_set_id: &str,
        mode: &str,
        interval_minutes: u32,
        startup_delay_minutes: u32,
    ) -> SyncPlanUpsert {
        SyncPlanUpsert {
            id: Some(id.to_string()),
            scheduler_set_id: scheduler_set_id.to_string(),
            name: format!("plan-{id}"),
            enabled: true,
            mode: mode.to_string(),
            interval_minutes,
            startup_delay_minutes,
            notification_mode: "summary".to_string(),
            target_filter: "label = priority".to_string(),
            sort_index: None,
            pause_mode: None,
            pause_until: None,
            notifications: SchedulerPlanNotifications::default(),
            criteria: SchedulerPlanCriteria::default(),
        }
    }

    #[test]
    fn push_previous_instagram_handle_dedupes_and_ignores_current() {
        // Adiciona o nome antigo normalizado.
        let list = push_previous_instagram_handle(None, "@OldName", "newname");
        assert_eq!(list, Some(vec!["OldName".to_string()]));
        // Não duplica (case/@ insensitive) e mantém o existente.
        let list = push_previous_instagram_handle(list, "oldname", "newname");
        assert_eq!(list, Some(vec!["OldName".to_string()]));
        // Acrescenta um segundo nome antigo distinto.
        let list = push_previous_instagram_handle(list, "older_one", "newname");
        assert_eq!(list, Some(vec!["OldName".to_string(), "older_one".to_string()]));
        // O handle atual nunca entra na lista.
        assert_eq!(
            push_previous_instagram_handle(None, "@newname", "newname"),
            None
        );
    }

    #[test]
    fn source_dedupe_key_normalizes_at_prefix_and_case() {
        assert_eq!(
            source_dedupe_key("instagram", "@Poliana"),
            source_dedupe_key("instagram", "poliana")
        );
        assert_eq!(source_dedupe_key("instagram", "  @Foo/ "), "foo");
        // TikTok mantém o '@' canônico mas continua case-insensitive.
        assert_eq!(
            source_dedupe_key("tiktok", "Bar"),
            source_dedupe_key("tiktok", "@bar")
        );
    }

    #[test]
    fn find_conflicting_source_handle_detects_at_prefix_duplicates() {
        let connection = Connection::open_in_memory().expect("in-memory connection");
        connection
            .execute_batch(
                "CREATE TABLE source_profiles (
                    id TEXT PRIMARY KEY,
                    provider TEXT NOT NULL,
                    handle TEXT NOT NULL,
                    deleted_at TEXT
                 );
                 INSERT INTO source_profiles (id, provider, handle, deleted_at)
                 VALUES ('keep', 'instagram', 'polianaarapiraca', NULL),
                        ('gone', 'instagram', 'removed_one', '2026-01-01T00:00:00Z'),
                        ('tt',   'tiktok',    '@polianaarapiraca', NULL);",
            )
            .expect("seed source_profiles");

        // '@polianaarapiraca' colide com 'polianaarapiraca' do mesmo provider.
        assert_eq!(
            find_conflicting_source_handle(&connection, "instagram", "@polianaarapiraca", "new-id")
                .expect("query ok"),
            Some("polianaarapiraca".to_string())
        );
        // O próprio registro nunca conflita consigo mesmo.
        assert_eq!(
            find_conflicting_source_handle(&connection, "instagram", "polianaarapiraca", "keep")
                .expect("query ok"),
            None
        );
        // Perfis excluídos (deleted_at) e outros providers não contam.
        assert_eq!(
            find_conflicting_source_handle(&connection, "instagram", "removed_one", "new-id")
                .expect("query ok"),
            None
        );
        // Handle novo e único não acusa conflito.
        assert_eq!(
            find_conflicting_source_handle(&connection, "instagram", "@brand_new", "new-id")
                .expect("query ok"),
            None
        );
    }

    #[test]
    fn instagram_post_ledger_snapshot_round_trips_keys_and_codes() {
        let connection = Connection::open_in_memory().expect("in-memory connection");
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .expect("enable foreign keys");
        connection
            .execute_batch(
                "CREATE TABLE source_profiles (id TEXT PRIMARY KEY);
                 CREATE TABLE provider_accounts (id TEXT PRIMARY KEY);",
            )
            .expect("create minimal foreign-key tables");
        connection
            .execute(
                "INSERT INTO source_profiles (id) VALUES (?1)",
                params!["source-1"],
            )
            .expect("insert source");
        connection
            .execute(
                "INSERT INTO provider_accounts (id) VALUES (?1)",
                params!["account-1"],
            )
            .expect("insert account");

        upsert_instagram_post_ledger_entries(
            &connection,
            "source-1",
            "account-1",
            "@handle",
            &[instagram_connector::ObservedInstagramPost {
                provider_post_key: "Post-1".to_string(),
                provider_post_code: Some("ABC123".to_string()),
                media_section: "timeline".to_string(),
            }],
            "2026-03-14T12:00:00Z",
        )
        .expect("upsert post ledger");

        let snapshot = load_instagram_post_ledger_snapshot_for_source(&connection, "source-1")
            .expect("load post ledger snapshot");

        assert!(snapshot.keys.contains("post-1"));
        assert!(snapshot.keys.contains("abc123"));
    }

    #[test]
    fn instagram_media_identity_candidates_strip_default_datetime_prefix() {
        let candidates = extract_instagram_media_identity_candidates_from_file_name(
            "2026-03-21 10.11.12 631495592_18384355651158098_6314965943446164250_n.jpg",
        );

        assert!(
            candidates.contains(
                &"2026-03-21 10.11.12 631495592_18384355651158098_6314965943446164250_n"
                    .to_string()
            ),
            "expected the full filename stem to remain a valid lookup candidate"
        );
        assert!(
            candidates.contains(&"631495592_18384355651158098_6314965943446164250_n".to_string()),
            "expected the provider media key suffix to be extracted from the new default naming pattern"
        );
    }

    #[test]
    fn merged_import_roots_prefer_managed_origin_for_duplicate_paths() {
        let mut descriptor = ImportRootDescriptor {
            path: r"F:\SCrawler\Data\instagram".to_string(),
            source: "default".to_string(),
            label: "Media root".to_string(),
            removable: false,
        };

        merge_import_root_descriptors(
            &mut descriptor,
            ImportRootDescriptor {
                path: r"F:\SCrawler\Data\instagram".to_string(),
                source: "manual".to_string(),
                label: "Manual root".to_string(),
                removable: true,
            },
        );

        assert!(
            !descriptor.removable,
            "manual duplicate should collapse into managed root"
        );
        assert_eq!(descriptor.source, "default");
        assert_eq!(descriptor.label, "Media root");
    }

    #[test]
    fn bootstrap_workspace_starts_without_demo_records() {
        let (_temp_dir, layout) = create_test_layout();

        let snapshot = with_workspace_layout(layout, load_snapshot).expect("bootstrap snapshot");

        assert!(
            snapshot.accounts.is_empty(),
            "fresh workspace should not seed accounts"
        );
        assert!(
            snapshot.sources.is_empty(),
            "fresh workspace should not seed sources"
        );
        assert!(
            snapshot.scheduler_sets.is_empty(),
            "fresh workspace should not seed scheduler sets"
        );
    }

    #[test]
    fn bootstrap_workspace_seeds_default_settings_only() {
        let (_temp_dir, layout) = create_test_layout();

        let snapshot = with_workspace_layout(layout, load_snapshot).expect("bootstrap snapshot");

        assert!(
            snapshot
                .app_settings
                .iter()
                .any(|setting| setting.key == "tool.yt-dlp.path"),
            "default tool settings should be present"
        );
        assert!(
            snapshot
                .app_settings
                .iter()
                .any(|setting| setting.key == "policy.notifications.default"),
            "default policy settings should be present"
        );
        assert!(
            snapshot.desktop_runtime.close_to_tray,
            "desktop runtime should default to close-to-tray"
        );
        assert!(
            !snapshot.desktop_runtime.silent_mode,
            "desktop runtime should default to non-silent mode"
        );
    }

    #[test]
    fn desktop_runtime_settings_roundtrip_into_snapshot_state() {
        let (_temp_dir, layout) = create_test_layout();

        let snapshot = with_workspace_layout(layout, |connection, test_layout| {
            upsert_app_setting_value(connection, DESKTOP_CLOSE_TO_TRAY_SETTING_KEY, "false")?;
            upsert_app_setting_value(connection, DESKTOP_SILENT_MODE_SETTING_KEY, "true")?;
            load_snapshot(connection, test_layout)
        })
        .expect("desktop runtime settings should load into snapshot");

        assert!(
            !snapshot.desktop_runtime.close_to_tray,
            "close-to-tray should reflect persisted app settings"
        );
        assert!(
            snapshot.desktop_runtime.silent_mode,
            "silent mode should reflect persisted app settings"
        );
    }

    #[test]
    fn source_upsert_requires_explicit_account_binding() {
        let (_temp_dir, layout) = create_test_layout();

        let error = with_workspace_layout(layout, |connection, test_layout| {
            upsert_source_profile_with_connection(
                connection,
                test_layout,
                sample_source("source-1", "instagram", None),
            )
        })
        .err()
        .expect("source binding without account should fail");

        assert!(
            error.contains("explicit provider account"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn source_upsert_requires_matching_account_provider() {
        let (_temp_dir, layout) = create_test_layout();

        let error = with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;

            upsert_source_profile_with_connection(
                connection,
                test_layout,
                sample_source("source-1", "tiktok", Some("account-1")),
            )
        })
        .err()
        .expect("cross-provider binding should fail");

        assert!(error.contains("cannot bind"), "unexpected error: {error}");
    }

    #[test]
    fn source_upsert_persists_matching_account_binding() {
        let (_temp_dir, layout) = create_test_layout();

        let snapshot = with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;

            upsert_source_profile_with_connection(
                connection,
                test_layout,
                sample_source("source-1", "instagram", Some("account-1")),
            )
        })
        .expect("source binding with matching provider should succeed");

        assert_eq!(snapshot.sources.len(), 1, "expected one persisted source");
        assert_eq!(
            snapshot.sources[0].account_id.as_deref(),
            Some("account-1"),
            "source should persist the explicit account binding"
        );
    }

    #[test]
    fn instagram_saved_posts_request_honors_account_defaults() {
        let (_temp_dir, layout) = create_test_layout();

        let request = with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;
            seed_instagram_session(connection, test_layout, "account-1")?;
            save_provider_account_settings_with_connection(
                connection,
                test_layout,
                "account-1".to_string(),
                vec![
                    string_provider_setting(
                        "instagram.account.extractSavedPostsImageFromVideo",
                        "false",
                    ),
                    string_provider_setting("instagram.defaults.downloadText", "true"),
                    string_provider_setting("instagram.defaults.downloadTextPosts", "true"),
                    string_provider_setting("instagram.defaults.textSpecialFolder", "false"),
                    string_provider_setting(
                        "instagram.defaults.placeExtractedImageIntoVideoFolder",
                        "true",
                    ),
                ],
            )?;

            let context = load_account_sync_context(connection, test_layout, "account-1")?;
            build_instagram_saved_posts_request(test_layout, &context)
        })
        .expect("saved-post request should build");

        assert!(
            !request.extract_image_from_video.timeline
                && !request.extract_image_from_video.reels
                && !request.extract_image_from_video.stories
                && !request.extract_image_from_video.stories_user
                && !request.extract_image_from_video.tagged,
            "saved-post extract-image flags should honor the account setting"
        );
        assert!(
            request.place_extracted_image_into_video_folder,
            "saved-post request should honor the account default for extracted image placement"
        );
        assert!(
            request.download_text,
            "saved-post request should honor the account default for text downloads"
        );
        assert!(
            request.download_text_posts,
            "saved-post request should honor the account default for text-post downloads"
        );
        assert!(
            !request.text_special_folder,
            "saved-post request should honor the account default for text special folder"
        );
    }

    #[test]
    fn resolved_source_media_output_root_uses_instagram_account_media_path_setting() {
        let (temp_dir, layout) = create_test_layout();
        let custom_media_root = temp_dir.path().join("custom-instagram-media");

        let resolved_root = with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;
            upsert_source_profile_with_connection(
                connection,
                test_layout,
                sample_source("source-1", "instagram", Some("account-1")),
            )?;
            save_provider_account_settings_with_connection(
                connection,
                test_layout,
                "account-1".to_string(),
                vec![ProviderAccountSettingValue {
                    setting_key: "instagram.account.mediaPath".to_string(),
                    value_kind: ProviderAccountSettingValueKind::String,
                    string_value: Some(custom_media_root.display().to_string()),
                    json_value: None,
                }],
            )?;

            let source = load_sources(connection)?
                .into_iter()
                .find(|item| item.id == "source-1")
                .ok_or_else(|| "source should exist".to_string())?;
            resolved_source_media_output_root_with_connection(connection, test_layout, &source)
        })
        .expect("source root should resolve");

        assert_eq!(
            resolved_root,
            custom_media_root.join("source-1"),
            "instagram sources should honor account-level mediaPath when resolving root"
        );
    }

    #[test]
    fn instagram_media_identity_keys_backfill_from_legacy_file_names() {
        let (_temp_dir, layout) = create_test_layout();

        let keys = with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;

            let mut source = sample_source("source-1", "instagram", Some("account-1"));
            source.handle = "@_theecat".to_string();
            upsert_source_profile_with_connection(connection, test_layout, source)?;

            let profile_root = test_layout.media_root.join("instagram").join("_theecat");
            fs::create_dir_all(&profile_root).map_err(|error| error.to_string())?;
            let legacy_file =
                profile_root.join("631495592_18384355651158098_6314965943446164250_n.jpg");
            fs::write(&legacy_file, b"legacy").map_err(|error| error.to_string())?;
            let normalized_path = normalize_media_file_path(&legacy_file)?;

            connection
                .execute(
                    "INSERT INTO media_items (
                        id,
                        provider,
                        source_id,
                        account_id,
                        session_id,
                        source_handle,
                        media_section,
                        media_type,
                        captured_at,
                        file_path,
                        missing_at,
                        created_at,
                        updated_at
                    )
                    VALUES (?1, 'instagram', ?2, ?3, NULL, ?4, NULL, 'image', ?5, ?6, NULL, ?5, ?5)",
                    params![
                        "media-1",
                        "source-1",
                        "account-1",
                        "_theecat",
                        "2026-03-13T00:00:00Z",
                        normalized_path,
                    ],
                )
                .map_err(|error| error.to_string())?;

            let source = load_source_profile_by_id(connection, "source-1")?;
            let settings = load_app_settings_map(connection)?;
            load_existing_instagram_media_identity_keys_for_source(test_layout, &source, &settings)
        })
        .expect("legacy media keys should load");

        assert!(
            keys.contains("631495592_18384355651158098_6314965943446164250_n"),
            "expected imported legacy filename stem to be used as instagram media identity"
        );
    }

    #[test]
    fn provider_account_cannot_change_provider_while_sources_are_bound() {
        let (_temp_dir, layout) = create_test_layout();

        with_workspace_layout(layout.clone(), |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;

            upsert_source_profile_with_connection(
                connection,
                test_layout,
                sample_source("source-1", "instagram", Some("account-1")),
            )?;

            Ok(())
        })
        .expect("initial account and source setup");

        let error = with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "tiktok"),
            )
        })
        .err()
        .expect("provider change should fail while sources remain bound");

        assert!(error.contains("bound source"), "unexpected error: {error}");
    }

    #[test]
    fn imported_session_updates_account_state_and_persists_session_metadata() {
        let (_temp_dir, layout) = create_test_layout();

        let snapshot = with_workspace_layout(layout.clone(), |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;

            seed_instagram_auth_settings(connection, test_layout, "account-1")?;

            import_provider_account_cookies_with_connection(
                connection,
                test_layout,
                ProviderAccountCookieImport {
                    account_id: "account-1".to_string(),
                    import_format: "json".to_string(),
                    content: serde_json::to_string(&sample_instagram_cookies())
                        .expect("cookie import json"),
                },
            )
        })
        .expect("session import should succeed");

        assert_eq!(snapshot.accounts.len(), 1, "expected one account");
        assert_eq!(snapshot.accounts[0].auth_mode, "imported_session");
        assert_eq!(snapshot.accounts[0].auth_state, "ready");
        assert_eq!(
            snapshot.account_sessions.len(),
            1,
            "expected one stored session"
        );
        assert!(
            snapshot.account_sessions[0].has_secret,
            "secret should exist in secure storage"
        );
        assert_eq!(snapshot.account_sessions[0].session_format, "cookie_json");
        assert!(
            !snapshot.account_sessions[0].fingerprint.is_empty(),
            "session fingerprint should be persisted"
        );

        let restored_secret = session_secret_store::load_secret(&layout, "account-1")
            .expect("session secret roundtrip");
        let restored_cookies =
            parse_session_cookies(&restored_secret).expect("restored session cookies");
        assert_eq!(
            restored_cookies.len(),
            2,
            "expected canonical cookie storage"
        );
        assert!(
            restored_cookies
                .iter()
                .any(|cookie| cookie.name == "sessionid" && cookie.value == "abc123"),
            "sessionid cookie should roundtrip through secure storage"
        );
    }

    #[test]
    fn validate_provider_account_marks_expired_sessions() {
        let (_temp_dir, layout) = create_test_layout();

        let snapshot = with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;

            seed_instagram_session(connection, test_layout, "account-1")?;
            connection
                .execute(
                    "UPDATE provider_account_sessions
                     SET expires_at = ?2
                     WHERE account_id = ?1",
                    params!["account-1", "2020-01-01T00:00:00Z"],
                )
                .map_err(|error| error.to_string())?;

            validate_provider_account_with_connection(
                connection,
                test_layout,
                "account-1".to_string(),
            )
        })
        .expect("validation should return snapshot");

        assert_eq!(snapshot.accounts[0].auth_state, "expired");
        assert_eq!(
            snapshot.account_sessions[0]
                .last_validation_error
                .as_deref(),
            Some("Stored session has expired.")
        );
    }

    #[test]
    fn deleting_provider_account_removes_persisted_session_secret() {
        let (_temp_dir, layout) = create_test_layout();

        with_workspace_layout(layout.clone(), |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;

            seed_instagram_session(connection, test_layout, "account-1")?;

            Ok(())
        })
        .expect("initial account and session setup");

        assert!(
            session_secret_store::has_secret(&layout, "account-1").expect("secret before delete")
        );

        with_workspace_layout(layout.clone(), |connection, test_layout| {
            delete_provider_account_with_connection(
                connection,
                test_layout,
                "account-1".to_string(),
            )
        })
        .expect("delete account with session");

        assert!(
            !session_secret_store::has_secret(&layout, "account-1").expect("secret after delete"),
            "deleting the account should remove the secure session secret"
        );
    }

    #[test]
    fn deleting_provider_account_ignores_soft_deleted_sources() {
        let (_temp_dir, layout) = create_test_layout();

        let snapshot = with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;
            upsert_source_profile_with_connection(
                connection,
                test_layout,
                sample_source("source-1", "instagram", Some("account-1")),
            )?;
            delete_source_profile_with_connection(
                connection,
                test_layout,
                "source-1".to_string(),
                SourceProfileDeleteMode::UserOnly,
            )?;
            delete_provider_account_with_connection(
                connection,
                test_layout,
                "account-1".to_string(),
            )
        })
        .expect("provider account deletion should ignore soft-deleted sources");

        assert!(snapshot.accounts.is_empty());
        assert!(snapshot.sources.is_empty());
    }

    #[test]
    fn provider_account_settings_roundtrip_through_editor_load() {
        let (_temp_dir, layout) = create_test_layout();

        let editor = with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;

            save_provider_account_settings_with_connection(
                connection,
                test_layout,
                "account-1".to_string(),
                vec![
                    ProviderAccountSettingValue {
                        setting_key: "request.user_agent".to_string(),
                        value_kind: ProviderAccountSettingValueKind::String,
                        string_value: Some("Instagram 321.0".to_string()),
                        json_value: None,
                    },
                    ProviderAccountSettingValue {
                        setting_key: "sync.window".to_string(),
                        value_kind: ProviderAccountSettingValueKind::Json,
                        string_value: None,
                        json_value: Some(json!({
                            "includeStories": true,
                            "maxItems": 25
                        })),
                    },
                ],
            )?;

            load_provider_account_editor_with_connection(
                connection,
                test_layout,
                "account-1".to_string(),
            )
        })
        .expect("editor should load persisted provider settings");

        assert_eq!(editor.account.id, "account-1");
        assert!(
            editor.session.is_none(),
            "account should not have session metadata"
        );
        assert_eq!(editor.settings.len(), 2, "expected two persisted settings");
        assert_eq!(
            editor.settings[0],
            ProviderAccountSettingValue {
                setting_key: "request.user_agent".to_string(),
                value_kind: ProviderAccountSettingValueKind::String,
                string_value: Some("Instagram 321.0".to_string()),
                json_value: None,
            }
        );
        assert_eq!(
            editor.settings[1],
            ProviderAccountSettingValue {
                setting_key: "sync.window".to_string(),
                value_kind: ProviderAccountSettingValueKind::Json,
                string_value: None,
                json_value: Some(json!({
                    "includeStories": true,
                    "maxItems": 25
                })),
            }
        );
    }

    #[test]
    fn instagram_avatar_cooldown_persists_in_provider_settings() {
        let (_temp_dir, layout) = create_test_layout();

        with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;

            let until = set_instagram_avatar_cooldown(
                connection,
                "account-1",
                StdDuration::from_secs(90),
                "2026-03-20T06:00:00Z",
            )?;
            assert_eq!(until.to_rfc3339(), "2026-03-20T06:01:30+00:00");

            let settings = load_provider_account_settings_map(connection, "account-1")?;
            assert_eq!(
                read_instagram_avatar_cooldown_until(&settings).map(|value| value.to_rfc3339()),
                Some("2026-03-20T06:01:30+00:00".to_string())
            );

            clear_instagram_avatar_cooldown(connection, "account-1")?;

            let cleared_settings = load_provider_account_settings_map(connection, "account-1")?;
            assert_eq!(
                read_instagram_avatar_cooldown_until(&cleared_settings),
                None
            );

            Ok::<(), String>(())
        })
        .expect("avatar cooldown should persist through provider settings");
    }

    #[test]
    fn clone_provider_account_copies_settings_without_session_material() {
        let (_temp_dir, layout) = create_test_layout();

        let (snapshot, cloned_account_id) =
            with_workspace_layout(layout.clone(), |connection, test_layout| {
                upsert_provider_account_with_connection(
                    connection,
                    test_layout,
                    sample_account("account-1", "instagram"),
                )?;

                seed_instagram_session(connection, test_layout, "account-1")?;

                save_provider_account_settings_with_connection(
                    connection,
                    test_layout,
                    "account-1".to_string(),
                    vec![
                        ProviderAccountSettingValue {
                            setting_key: "request.user_agent".to_string(),
                            value_kind: ProviderAccountSettingValueKind::String,
                            string_value: Some("Instagram 321.0".to_string()),
                            json_value: None,
                        },
                        ProviderAccountSettingValue {
                            setting_key: "sync.window".to_string(),
                            value_kind: ProviderAccountSettingValueKind::Json,
                            string_value: None,
                            json_value: Some(json!({ "maxItems": 25 })),
                        },
                    ],
                )?;

                let snapshot = clone_provider_account_with_connection(
                    connection,
                    test_layout,
                    "account-1".to_string(),
                )?;

                let cloned_account_id = snapshot
                    .accounts
                    .iter()
                    .find(|account| account.id != "account-1")
                    .map(|account| account.id.clone())
                    .expect("cloned account id");

                Ok((snapshot, cloned_account_id))
            })
            .expect("clone provider account should succeed");

        assert_eq!(snapshot.accounts.len(), 2, "expected original plus clone");
        assert_eq!(
            snapshot.account_sessions.len(),
            1,
            "clone must not duplicate session metadata entries"
        );
        assert_eq!(
            snapshot.account_sessions[0].account_id, "account-1",
            "original session metadata should remain bound to the source account"
        );
        assert!(
            !session_secret_store::has_secret(&layout, &cloned_account_id)
                .expect("clone should not have a session secret"),
            "clone must not receive secret material"
        );

        let cloned_editor = with_workspace_layout(layout, |connection, test_layout| {
            load_provider_account_editor_with_connection(connection, test_layout, cloned_account_id)
        })
        .expect("cloned editor should load");

        assert!(
            cloned_editor.session.is_none(),
            "clone should not carry session metadata"
        );
        assert_eq!(
            cloned_editor.settings,
            vec![
                ProviderAccountSettingValue {
                    setting_key: "request.user_agent".to_string(),
                    value_kind: ProviderAccountSettingValueKind::String,
                    string_value: Some("Instagram 321.0".to_string()),
                    json_value: None,
                },
                ProviderAccountSettingValue {
                    setting_key: "sync.window".to_string(),
                    value_kind: ProviderAccountSettingValueKind::Json,
                    string_value: None,
                    json_value: Some(json!({ "maxItems": 25 })),
                },
            ],
            "clone should inherit advanced settings"
        );
    }

    #[test]
    fn deleting_provider_account_cascades_provider_settings() {
        let (_temp_dir, layout) = create_test_layout();

        with_workspace_layout(layout.clone(), |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;

            save_provider_account_settings_with_connection(
                connection,
                test_layout,
                "account-1".to_string(),
                vec![ProviderAccountSettingValue {
                    setting_key: "request.user_agent".to_string(),
                    value_kind: ProviderAccountSettingValueKind::String,
                    string_value: Some("Instagram 321.0".to_string()),
                    json_value: None,
                }],
            )?;

            Ok(())
        })
        .expect("account settings setup");

        with_workspace_layout(layout.clone(), |connection, test_layout| {
            delete_provider_account_with_connection(
                connection,
                test_layout,
                "account-1".to_string(),
            )
        })
        .expect("delete account with advanced settings");

        let remaining_settings = with_workspace_layout(layout, |connection, _| {
            connection
                .query_row(
                    "SELECT COUNT(*) FROM provider_account_settings WHERE account_id = ?1",
                    params!["account-1"],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| error.to_string())
        })
        .expect("settings count after delete");

        assert_eq!(
            remaining_settings, 0,
            "provider account settings should cascade on delete"
        );
    }

    #[test]
    fn running_source_sync_persists_successful_run_history() {
        struct SuccessfulExecutor;

        impl ToolExecutor for SuccessfulExecutor {
            fn execute(&self, _invocation: &ToolInvocation) -> Result<ToolExecutionResult, String> {
                Ok(ToolExecutionResult {
                    status: "succeeded".to_string(),
                })
            }
        }

        let (_temp_dir, layout) = create_test_layout();

        let snapshot = with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;
            seed_instagram_session(connection, test_layout, "account-1")?;
            upsert_source_profile_with_connection(
                connection,
                test_layout,
                sample_source("source-1", "instagram", Some("account-1")),
            )?;

            run_source_sync_with_connection(
                connection,
                test_layout,
                "source-1".to_string(),
                "manual",
                None,
                None,
                &SuccessfulExecutor,
            )
        })
        .expect("source sync should succeed");

        assert_eq!(
            snapshot.source_sync_runs.len(),
            1,
            "expected one persisted sync run"
        );
        assert_eq!(snapshot.source_sync_runs[0].status, "succeeded");
        assert_eq!(snapshot.source_sync_runs[0].provider, "instagram");
        assert_eq!(snapshot.accounts[0].auth_state, "ready");
        assert!(
            snapshot.source_sync_runs[0]
                .command_preview
                .contains("gallery-dl"),
            "expected connector command preview to be persisted"
        );
    }

    #[test]
    fn running_source_sync_persists_failed_run_and_degrades_account() {
        struct FailingExecutor;

        impl ToolExecutor for FailingExecutor {
            fn execute(&self, _invocation: &ToolInvocation) -> Result<ToolExecutionResult, String> {
                Err("gallery-dl exited with failure".to_string())
            }
        }

        let (_temp_dir, layout) = create_test_layout();

        let snapshot = with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;
            seed_instagram_session(connection, test_layout, "account-1")?;
            upsert_source_profile_with_connection(
                connection,
                test_layout,
                sample_source("source-1", "instagram", Some("account-1")),
            )?;

            run_source_sync_with_connection(
                connection,
                test_layout,
                "source-1".to_string(),
                "manual",
                None,
                None,
                &FailingExecutor,
            )
        })
        .expect("failed sync should still persist run history");

        assert_eq!(
            snapshot.source_sync_runs.len(),
            1,
            "expected failed run history"
        );
        assert_eq!(snapshot.source_sync_runs[0].status, "failed");
        assert_eq!(snapshot.accounts[0].auth_state, "degraded");
        assert!(
            snapshot.account_sessions[0]
                .last_validation_error
                .as_deref()
                .is_some_and(|value| value.contains("gallery-dl exited with failure")),
            "expected connector failure to propagate into account validation state"
        );
    }

    #[test]
    fn running_instagram_source_sync_blocks_when_base_auth_is_missing() {
        struct SuccessfulExecutor;

        impl ToolExecutor for SuccessfulExecutor {
            fn execute(&self, _invocation: &ToolInvocation) -> Result<ToolExecutionResult, String> {
                Ok(ToolExecutionResult {
                    status: "succeeded".to_string(),
                })
            }
        }

        let (_temp_dir, layout) = create_test_layout();

        let snapshot = with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;
            let _ = save_provider_account_cookies_with_connection(
                connection,
                test_layout,
                "account-1",
                sample_instagram_cookies(),
            )?;
            upsert_source_profile_with_connection(
                connection,
                test_layout,
                sample_source("source-1", "instagram", Some("account-1")),
            )?;

            run_source_sync_with_connection(
                connection,
                test_layout,
                "source-1".to_string(),
                "manual",
                None,
                None,
                &SuccessfulExecutor,
            )
        })
        .expect("preflight failure should still persist run history");

        assert_eq!(snapshot.source_sync_runs.len(), 1);
        assert_eq!(snapshot.source_sync_runs[0].status, "failed");
        assert!(
            snapshot.source_sync_runs[0]
                .summary
                .contains("required base auth"),
            "preflight summary should explain the missing base auth"
        );
        assert_eq!(snapshot.accounts[0].auth_state, "degraded");
        assert!(
            snapshot.account_sessions[0]
                .last_validation_error
                .as_deref()
                .is_some_and(|value| value.contains("required base auth")),
            "missing base auth should degrade the stored session state"
        );
    }

    #[test]
    fn running_instagram_source_sync_skips_when_provider_cooldown_is_active() {
        struct SuccessfulExecutor;

        impl ToolExecutor for SuccessfulExecutor {
            fn execute(&self, _invocation: &ToolInvocation) -> Result<ToolExecutionResult, String> {
                Ok(ToolExecutionResult {
                    status: "succeeded".to_string(),
                })
            }
        }

        let (_temp_dir, layout) = create_test_layout();

        let snapshot = with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;
            seed_instagram_session(connection, test_layout, "account-1")?;
            save_provider_account_settings_with_connection(
                connection,
                test_layout,
                "account-1".to_string(),
                vec![ProviderAccountSettingValue {
                    setting_key: INSTAGRAM_SYNC_COOLDOWN_UNTIL_SETTING_KEY.to_string(),
                    value_kind: ProviderAccountSettingValueKind::String,
                    string_value: Some("2030-01-01T00:00:00Z".to_string()),
                    json_value: None,
                }],
            )?;
            upsert_source_profile_with_connection(
                connection,
                test_layout,
                sample_source("source-1", "instagram", Some("account-1")),
            )?;

            run_source_sync_with_connection(
                connection,
                test_layout,
                "source-1".to_string(),
                "manual",
                None,
                None,
                &SuccessfulExecutor,
            )
        })
        .expect("cooldown skip should persist run history");

        assert_eq!(snapshot.source_sync_runs.len(), 1);
        assert_eq!(snapshot.source_sync_runs[0].status, "skipped");
        assert!(
            snapshot.source_sync_runs[0]
                .summary
                .contains("provider cooldown is active until 2030-01-01T00:00:00+00:00"),
            "skip summary should expose the cooldown deadline"
        );
        assert_eq!(snapshot.accounts[0].auth_state, "ready");
        assert!(
            snapshot.account_sessions[0].last_validation_error.is_none(),
            "cooldown skips should not degrade account health"
        );
    }

    #[test]
    fn running_source_sync_cancellation_preserves_account_health() {
        struct CancelledExecutor;

        impl ToolExecutor for CancelledExecutor {
            fn execute(&self, _invocation: &ToolInvocation) -> Result<ToolExecutionResult, String> {
                Err("source sync cancelled by user".to_string())
            }
        }

        let (_temp_dir, layout) = create_test_layout();

        let snapshot = with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;
            seed_instagram_session(connection, test_layout, "account-1")?;
            upsert_source_profile_with_connection(
                connection,
                test_layout,
                sample_source("source-1", "instagram", Some("account-1")),
            )?;

            run_source_sync_with_connection(
                connection,
                test_layout,
                "source-1".to_string(),
                "manual",
                None,
                None,
                &CancelledExecutor,
            )
        })
        .expect("cancelled sync should persist run history");

        assert_eq!(snapshot.source_sync_runs.len(), 1);
        assert_eq!(snapshot.source_sync_runs[0].status, "failed");
        assert!(snapshot.source_sync_runs[0]
            .summary
            .to_ascii_lowercase()
            .contains("cancelled by user"));
        assert_eq!(snapshot.accounts[0].auth_state, "ready");
        assert!(
            snapshot.account_sessions[0].last_validation_error.is_none(),
            "manual cancellation should not mark account session as degraded"
        );
    }

    #[test]
    fn find_source_avatar_prefers_profile_picture_file() {
        let temp_dir = tempfile::tempdir().expect("temp directory");
        let root = temp_dir.path();
        fs::write(root.join("avatar.jpg"), b"legacy-avatar").expect("legacy avatar");
        fs::write(
            root.join(PROFILE_PICTURE_FILE_NAME),
            b"profile-picture-avatar",
        )
        .expect("profile picture");

        let resolved = find_source_avatar(root).expect("avatar path should resolve");
        assert!(
            resolved
                .to_ascii_lowercase()
                .ends_with(&PROFILE_PICTURE_FILE_NAME.to_ascii_lowercase()),
            "ProfilePicture.jpg should take priority over heuristic avatar names"
        );
    }

    #[test]
    fn find_source_avatar_uses_scrawler_user_picture_layout() {
        let temp_dir = tempfile::tempdir().expect("temp directory");
        let root = temp_dir.path();
        let pictures_dir = root.join(PROFILE_SETTINGS_DIR_NAME).join("Pictures");
        fs::create_dir_all(&pictures_dir).expect("pictures dir");
        fs::write(pictures_dir.join("UserPicture.jpg"), b"imported-avatar")
            .expect("user picture");

        let resolved = find_source_avatar(root).expect("avatar path should resolve");
        assert!(
            resolved.to_ascii_lowercase().ends_with("userpicture.jpg"),
            "imported SCrawler avatar (Settings/Pictures/UserPicture.jpg) should resolve, got {resolved}"
        );
    }

    #[test]
    fn upgrade_twitter_avatar_url_strips_size_suffixes() {
        assert_eq!(
            upgrade_twitter_avatar_url(
                "https://pbs.twimg.com/profile_images/123/avatar_normal.jpg"
            ),
            "https://pbs.twimg.com/profile_images/123/avatar.jpg"
        );
        assert_eq!(
            upgrade_twitter_avatar_url(
                "https://pbs.twimg.com/profile_images/123/avatar_400x400.png?foo=bar"
            ),
            "https://pbs.twimg.com/profile_images/123/avatar.png?foo=bar"
        );
        // Sem sufixo de tamanho conhecido: mantém a URL original.
        assert_eq!(
            upgrade_twitter_avatar_url("https://pbs.twimg.com/profile_images/123/avatar.jpg"),
            "https://pbs.twimg.com/profile_images/123/avatar.jpg"
        );
    }

    #[test]
    fn find_source_avatar_ignores_nested_gallery_dl_profile_picture() {
        let temp_dir = tempfile::tempdir().expect("temp directory");
        let root = temp_dir.path();

        // Simulate gallery-dl structure: instagram/{handle}/ProfilePicture.jpg
        let gallery_dl_dir = root.join("instagram").join("beeaa0_0");
        fs::create_dir_all(&gallery_dl_dir).expect("gallery-dl dir");
        fs::write(
            gallery_dl_dir.join(PROFILE_PICTURE_FILE_NAME),
            b"gallery-dl-avatar",
        )
        .expect("gallery-dl avatar");

        // No ProfilePicture.jpg at root or Settings/
        let resolved = find_source_avatar(root);
        assert!(
            resolved.is_none(),
            "should not pick up ProfilePicture.jpg from nested gallery-dl directory, got: {:?}",
            resolved
        );
    }

    #[test]
    fn find_source_avatar_finds_root_avatar_even_with_nested_gallery_dl() {
        let temp_dir = tempfile::tempdir().expect("temp directory");
        let root = temp_dir.path();

        // Gallery-dl nested structure
        let gallery_dl_dir = root.join("instagram").join("beeaa0_0");
        fs::create_dir_all(&gallery_dl_dir).expect("gallery-dl dir");
        fs::write(
            gallery_dl_dir.join(PROFILE_PICTURE_FILE_NAME),
            b"gallery-dl-avatar",
        )
        .expect("gallery-dl avatar");

        // Root-level avatar.jpg (legacy heuristic match)
        fs::write(root.join("avatar.jpg"), b"root-avatar").expect("root avatar");

        let resolved = find_source_avatar(root).expect("avatar path should resolve");
        assert!(
            resolved.to_ascii_lowercase().ends_with("avatar.jpg"),
            "should find root-level avatar.jpg, not nested gallery-dl file, got: {:?}",
            resolved
        );
    }

    #[test]
    fn ensure_profile_picture_at_root_promotes_nested_profile_picture() {
        let temp_dir = tempfile::tempdir().expect("temp directory");
        let root = temp_dir.path();
        let nested_root = root.join("instagram").join("lolxz_maria");
        fs::create_dir_all(&nested_root).expect("nested root");

        let nested_profile_picture = nested_root.join(PROFILE_PICTURE_FILE_NAME);
        fs::write(&nested_profile_picture, b"nested-profile-picture").expect("nested avatar");

        let promoted = match ensure_profile_picture_at_root(root, &nested_profile_picture) {
            Ok(path) => path,
            Err(error) => panic!("promote avatar failed: {}", error.message),
        };
        assert_eq!(
            promoted,
            root.join(PROFILE_SETTINGS_DIR_NAME)
                .join(PROFILE_PICTURE_FILE_NAME),
            "avatar should be normalized to Settings/ProfilePicture.jpg"
        );
        assert!(
            promoted.exists(),
            "normalized profile picture should exist in Settings"
        );
        assert_eq!(
            fs::read(&promoted).expect("promoted bytes"),
            b"nested-profile-picture",
            "normalized file should preserve avatar content"
        );
        assert!(
            !nested_profile_picture.exists(),
            "nested gallery-dl avatar should be removed after promotion"
        );
        assert!(
            !nested_root.exists(),
            "empty nested gallery-dl directories should be cleaned up after promotion"
        );
    }

    #[test]
    fn ensure_profile_picture_at_root_syncs_root_avatar_to_settings() {
        let temp_dir = tempfile::tempdir().expect("temp directory");
        let root = temp_dir.path();
        fs::create_dir_all(root).expect("profile root");

        let root_profile_picture = root.join(PROFILE_PICTURE_FILE_NAME);
        let image = image::RgbImage::from_fn(24, 24, |x, y| {
            image::Rgb([(x * 10) as u8, (y * 10) as u8, 120])
        });
        image
            .save(&root_profile_picture)
            .expect("write valid root profile picture");

        let promoted = match ensure_profile_picture_at_root(root, &root_profile_picture) {
            Ok(path) => path,
            Err(error) => panic!("sync root avatar failed: {}", error.message),
        };

        let expected_settings_picture = root
            .join(PROFILE_SETTINGS_DIR_NAME)
            .join(PROFILE_PICTURE_FILE_NAME);
        assert_eq!(
            promoted, expected_settings_picture,
            "root avatar should resolve to Settings/ProfilePicture.jpg"
        );
        assert!(
            expected_settings_picture.exists(),
            "Settings profile picture should exist after sync"
        );
    }

    #[test]
    fn update_instagram_source_handle_after_sync_updates_source_and_media_rows() {
        let (_temp_dir, layout) = create_test_layout();

        let (source_handle, media_handle) = with_workspace_layout(layout, |connection, test_layout| {
            upsert_source_profile_with_connection(
                connection,
                test_layout,
                sample_source("source-1", "instagram", None),
            )?;

            let captured_at = "2026-03-12T03:00:00Z";
            let media_path = test_layout.media_root.join("legacy-handle.jpg");
            connection
                .execute(
                    "INSERT INTO media_items (
                        id,
                        provider,
                        source_id,
                        account_id,
                        session_id,
                        source_handle,
                        media_section,
                        media_type,
                        captured_at,
                        file_path,
                        missing_at,
                        created_at,
                        updated_at
                     )
                     VALUES (?1, 'instagram', ?2, NULL, NULL, ?3, 'timeline', 'image', ?4, ?5, NULL, ?4, ?4)",
                    params![
                        new_id(),
                        "source-1",
                        "source-1",
                        captured_at,
                        media_path.display().to_string(),
                    ],
                )
                .map_err(|error| error.to_string())?;

            update_instagram_source_handle_after_sync(
                connection,
                "source-1",
                "new_profile",
                "2026-03-12T03:01:00Z",
            )?;

            let source_handle = connection
                .query_row(
                    "SELECT handle FROM source_profiles WHERE id = ?1",
                    params!["source-1"],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|error| error.to_string())?;
            let media_handle = connection
                .query_row(
                    "SELECT source_handle FROM media_items WHERE source_id = ?1 LIMIT 1",
                    params!["source-1"],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|error| error.to_string())?;

            Ok((source_handle, media_handle))
        })
        .expect("source/media handles should update");

        assert_eq!(source_handle, "new_profile");
        assert_eq!(media_handle, "new_profile");
    }

    #[test]
    fn update_instagram_source_description_after_sync_populates_empty_profile_note() {
        let (_temp_dir, layout) = create_test_layout();

        let saved_description = with_workspace_layout(layout, |connection, test_layout| {
            upsert_source_profile_with_connection(
                connection,
                test_layout,
                sample_source("source-1", "instagram", None),
            )?;

            let source = load_source_profile_by_id(connection, "source-1")?;
            update_instagram_source_description_after_sync(
                connection,
                &source,
                "Imported biography",
                false,
                "2026-03-13T03:01:00Z",
            )?;

            let raw_sync_options = connection
                .query_row(
                    "SELECT sync_options_json FROM source_profiles WHERE id = ?1",
                    params!["source-1"],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|error| error.to_string())?;
            let sync_options = deserialize_source_sync_options("instagram", &raw_sync_options);
            Ok(sync_options
                .instagram
                .and_then(|instagram| instagram.description)
                .unwrap_or_default())
        })
        .expect("description should persist");

        assert_eq!(saved_description, "Imported biography");
    }

    #[test]
    fn update_instagram_source_description_after_sync_preserves_existing_note_without_force() {
        let (_temp_dir, layout) = create_test_layout();

        let saved_description = with_workspace_layout(layout, |connection, test_layout| {
            let mut source = sample_source("source-1", "instagram", None);
            source.sync_options = SourceSyncOptions {
                instagram: Some(InstagramSourceSyncOptions {
                    description: Some("Operator note".to_string()),
                    ..default_instagram_source_sync_options()
                }),
                ..Default::default()
            };
            upsert_source_profile_with_connection(connection, test_layout, source)?;

            let source = load_source_profile_by_id(connection, "source-1")?;
            update_instagram_source_description_after_sync(
                connection,
                &source,
                "Imported biography",
                false,
                "2026-03-13T03:01:00Z",
            )?;

            let raw_sync_options = connection
                .query_row(
                    "SELECT sync_options_json FROM source_profiles WHERE id = ?1",
                    params!["source-1"],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|error| error.to_string())?;
            let sync_options = deserialize_source_sync_options("instagram", &raw_sync_options);
            Ok(sync_options
                .instagram
                .and_then(|instagram| instagram.description)
                .unwrap_or_default())
        })
        .expect("existing note should remain");

        assert_eq!(saved_description, "Operator note");
    }

    #[test]
    fn update_instagram_source_description_after_sync_appends_history_with_force() {
        let (_temp_dir, layout) = create_test_layout();

        let saved_description = with_workspace_layout(layout, |connection, test_layout| {
            let mut source = sample_source("source-1", "instagram", None);
            source.sync_options = SourceSyncOptions {
                instagram: Some(InstagramSourceSyncOptions {
                    description: Some("Operator note".to_string()),
                    ..default_instagram_source_sync_options()
                }),
                ..Default::default()
            };
            upsert_source_profile_with_connection(connection, test_layout, source)?;

            let source = load_source_profile_by_id(connection, "source-1")?;
            update_instagram_source_description_after_sync(
                connection,
                &source,
                "Imported biography",
                true,
                "2026-03-13T03:01:00Z",
            )?;

            let raw_sync_options = connection
                .query_row(
                    "SELECT sync_options_json FROM source_profiles WHERE id = ?1",
                    params!["source-1"],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|error| error.to_string())?;
            let sync_options = deserialize_source_sync_options("instagram", &raw_sync_options);
            Ok(sync_options
                .instagram
                .and_then(|instagram| instagram.description)
                .unwrap_or_default())
        })
        .expect("history should append");

        assert_eq!(saved_description, "Operator note\n----\nImported biography");
    }

    #[test]
    fn update_instagram_source_description_after_sync_avoids_duplicate_history_entries() {
        let (_temp_dir, layout) = create_test_layout();

        let saved_description = with_workspace_layout(layout, |connection, test_layout| {
            let mut source = sample_source("source-1", "instagram", None);
            source.sync_options = SourceSyncOptions {
                instagram: Some(InstagramSourceSyncOptions {
                    description: Some("Operator note\n----\nImported biography".to_string()),
                    ..default_instagram_source_sync_options()
                }),
                ..Default::default()
            };
            upsert_source_profile_with_connection(connection, test_layout, source)?;

            let source = load_source_profile_by_id(connection, "source-1")?;
            update_instagram_source_description_after_sync(
                connection,
                &source,
                "Imported biography",
                true,
                "2026-03-13T03:01:00Z",
            )?;

            let raw_sync_options = connection
                .query_row(
                    "SELECT sync_options_json FROM source_profiles WHERE id = ?1",
                    params!["source-1"],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|error| error.to_string())?;
            let sync_options = deserialize_source_sync_options("instagram", &raw_sync_options);
            Ok(sync_options
                .instagram
                .and_then(|instagram| instagram.description)
                .unwrap_or_default())
        })
        .expect("duplicate history should be avoided");

        assert_eq!(saved_description, "Operator note\n----\nImported biography");
    }

    #[test]
    fn parse_legacy_instagram_profile_xml_reads_description() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let profile_root = temp_dir.path().join("legacy-profile");
        let user_xml_path = create_legacy_instagram_profile_root(
            &profile_root,
            "instagram-account",
            "legacy.user",
            Some("Imported biography"),
        )
        .expect("legacy profile fixture");

        let profile =
            parse_legacy_instagram_profile_xml(&user_xml_path).expect("legacy xml should parse");

        assert_eq!(profile.description.as_deref(), Some("Imported biography"));
    }

    #[test]
    fn run_instagram_scrawler_import_populates_profile_note_from_legacy_description() {
        let (_temp_dir, layout) = create_test_layout();
        let legacy_root = layout.media_root.join("legacy-import").join("legacy.user");
        create_legacy_instagram_profile_root(
            &legacy_root,
            "instagram-account",
            "legacy.user",
            Some("Imported biography"),
        )
        .expect("legacy profile fixture");

        let saved_description = with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;

            let manual_root = legacy_root.display().to_string();
            let preview = preview_instagram_scrawler_import_with_connection(
                connection,
                test_layout,
                ImportPreviewOptions {
                    force_reimport: false,
                    manual_roots: vec![manual_root.clone()],
                    disabled_roots: Vec::new(),
                },
            )?;

            assert_eq!(
                preview.profiles.len(),
                1,
                "expected imported legacy profile"
            );

            let result = run_instagram_scrawler_import_with_connection(
                connection,
                test_layout,
                ImportRunRequest {
                    force_reimport: false,
                    manual_roots: vec![manual_root],
                    disabled_roots: Vec::new(),
                    resolutions: preview
                        .profiles
                        .iter()
                        .map(|profile| ImportResolution {
                            profile_root: profile.profile_root.clone(),
                            action: "import".to_string(),
                            account_id: profile.account_id.clone(),
                        })
                        .collect(),
                },
            )?;

            assert_eq!(result.imported_profiles, 1, "legacy profile should import");

            let source_id = result
                .profiles
                .first()
                .and_then(|profile| profile.source_id.as_deref())
                .ok_or_else(|| "imported source id missing".to_string())?;
            let source = load_source_profile_by_id(connection, source_id)?;
            Ok(source
                .sync_options
                .instagram
                .and_then(|instagram| instagram.description)
                .unwrap_or_default())
        })
        .expect("legacy import should persist note");

        assert_eq!(saved_description, "Imported biography");
    }

    #[test]
    fn run_instagram_scrawler_import_seeds_ledgers_from_legacy_data_xml() {
        let (_temp_dir, layout) = create_test_layout();
        let legacy_root = layout.media_root.join("legacy-import").join("ledger.user");
        create_legacy_instagram_profile_root(
            &legacy_root,
            "instagram-account",
            "ledger.user",
            None,
        )
        .expect("legacy profile fixture");
        let legacy_file_name = "471328806_18026404109545583_7067156219508743506_n.jpg";
        fs::write(legacy_root.join(legacy_file_name), b"image").expect("legacy media");
        create_legacy_instagram_data_xml(
            &legacy_root,
            legacy_file_name,
            "3528946119357054415_46332873582",
            None,
            "https://instagram.example/media.jpg",
            "https://www.instagram.com/p/DD5VcxjxT3P/",
        )
        .expect("legacy data xml");

        let (media_snapshot, post_snapshot, media_section) =
            with_workspace_layout(layout, |connection, test_layout| {
                upsert_provider_account_with_connection(
                    connection,
                    test_layout,
                    sample_account("account-1", "instagram"),
                )?;

                let manual_root = legacy_root.display().to_string();
                let preview = preview_instagram_scrawler_import_with_connection(
                    connection,
                    test_layout,
                    ImportPreviewOptions {
                        force_reimport: false,
                        manual_roots: vec![manual_root.clone()],
                        disabled_roots: Vec::new(),
                    },
                )?;
                let result = run_instagram_scrawler_import_with_connection(
                    connection,
                    test_layout,
                    ImportRunRequest {
                        force_reimport: false,
                        manual_roots: vec![manual_root],
                        disabled_roots: Vec::new(),
                        resolutions: preview
                            .profiles
                            .iter()
                            .map(|profile| ImportResolution {
                                profile_root: profile.profile_root.clone(),
                                action: "import".to_string(),
                                account_id: profile.account_id.clone(),
                            })
                            .collect(),
                    },
                )?;

                let source_id = result
                    .profiles
                    .first()
                    .and_then(|profile| profile.source_id.as_deref())
                    .ok_or_else(|| "imported source id missing".to_string())?;
                let media_snapshot =
                    load_instagram_media_ledger_snapshot_for_source(connection, source_id)?;
                let post_snapshot =
                    load_instagram_post_ledger_snapshot_for_source(connection, source_id)?;
                let media_section = connection
                    .query_row(
                        "SELECT media_section
                     FROM instagram_sync_post_ledger
                     WHERE source_id = ?1
                     LIMIT 1",
                        params![source_id],
                        |row| row.get::<_, String>(0),
                    )
                    .map_err(|error| error.to_string())?;
                Ok((media_snapshot, post_snapshot, media_section))
            })
            .expect("legacy import should seed ledgers");

        assert!(
            media_snapshot
                .media_keys
                .contains("471328806_18026404109545583_7067156219508743506_n"),
            "expected media ledger to include the legacy file stem"
        );
        assert!(
            post_snapshot
                .keys
                .contains("3528946119357054415_46332873582"),
            "expected post ledger to include the legacy post id"
        );
        assert!(
            post_snapshot.keys.contains("dd5vcxjxt3p"),
            "expected post ledger to include the permalink code"
        );
        assert_eq!(media_section, "timeline");
    }

    #[test]
    fn run_instagram_scrawler_import_seeds_media_aliases_from_legacy_url() {
        let (_temp_dir, layout) = create_test_layout();
        let legacy_root = layout.media_root.join("legacy-import").join("alias.user");
        create_legacy_instagram_profile_root(&legacy_root, "instagram-account", "alias.user", None)
            .expect("legacy profile fixture");
        let legacy_file_name = "471328806_18026404109545583_7067156219508743506_n.jpg";
        fs::write(legacy_root.join(legacy_file_name), b"image").expect("legacy media");
        create_legacy_instagram_data_xml(
            &legacy_root,
            legacy_file_name,
            "3528946119357054415_46332873582",
            None,
            "https://instagram.example/media/API_ALIAS_01.jpg?stp=dst-jpg_e35",
            "https://www.instagram.com/p/DD5VcxjxT3P/",
        )
        .expect("legacy data xml");

        let (alias_snapshot, hashed_alias_count) =
            with_workspace_layout(layout, |connection, test_layout| {
                upsert_provider_account_with_connection(
                    connection,
                    test_layout,
                    sample_account("account-1", "instagram"),
                )?;

                let manual_root = legacy_root.display().to_string();
                let preview = preview_instagram_scrawler_import_with_connection(
                    connection,
                    test_layout,
                    ImportPreviewOptions {
                        force_reimport: false,
                        manual_roots: vec![manual_root.clone()],
                        disabled_roots: Vec::new(),
                    },
                )?;
                let result = run_instagram_scrawler_import_with_connection(
                    connection,
                    test_layout,
                    ImportRunRequest {
                        force_reimport: false,
                        manual_roots: vec![manual_root],
                        disabled_roots: Vec::new(),
                        resolutions: preview
                            .profiles
                            .iter()
                            .map(|profile| ImportResolution {
                                profile_root: profile.profile_root.clone(),
                                action: "import".to_string(),
                                account_id: profile.account_id.clone(),
                            })
                            .collect(),
                    },
                )?;

                let source_id = result
                    .profiles
                    .first()
                    .and_then(|profile| profile.source_id.as_deref())
                    .ok_or_else(|| "imported source id missing".to_string())?;
                let alias_snapshot =
                    load_instagram_media_alias_snapshot_for_source(connection, source_id)?;
                let hashed_alias_count = connection
                    .query_row(
                        "SELECT COUNT(*)
                         FROM instagram_media_key_aliases
                         WHERE source_id = ?1
                           AND file_sha256 IS NOT NULL
                           AND file_sha256 <> ''",
                        params![source_id],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(|error| error.to_string())?;
                Ok((alias_snapshot, hashed_alias_count))
            })
            .expect("legacy import should seed aliases");

        assert!(
            alias_snapshot.keys.contains("api_alias_01"),
            "expected media alias snapshot to include the basename from the legacy media URL"
        );
        assert!(
            alias_snapshot
                .keys
                .contains("3528946119357054415_46332873582"),
            "expected media alias snapshot to include the legacy post id"
        );
        assert!(
            alias_snapshot.keys.contains("dd5vcxjxt3p"),
            "expected media alias snapshot to include the legacy post code"
        );
        assert!(
            hashed_alias_count > 0,
            "expected imported aliases to persist a file SHA256 fingerprint"
        );
    }

    #[test]
    fn scrawler_import_prefers_true_name_over_user_name_for_handle() {
        let (_temp_dir, layout) = create_test_layout();
        let legacy_root = layout.media_root.join("legacy-import").join("_franjudaaa_");
        create_legacy_instagram_profile_root_full(
            &legacy_root,
            "instagram-account",
            "_franjudaaa_",
            Some("franjuda"),
            Some("17443084061"),
            None,
        )
        .expect("legacy profile fixture");

        let (handle, user_id_hint) = with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;

            let manual_root = legacy_root.display().to_string();
            let preview = preview_instagram_scrawler_import_with_connection(
                connection,
                test_layout,
                ImportPreviewOptions {
                    force_reimport: false,
                    manual_roots: vec![manual_root.clone()],
                    disabled_roots: Vec::new(),
                },
            )?;

            assert_eq!(
                preview.profiles.len(),
                1,
                "expected one legacy profile in preview"
            );
            assert_eq!(
                preview.profiles[0].handle, "franjuda",
                "preview handle should use TrueName"
            );

            let result = run_instagram_scrawler_import_with_connection(
                connection,
                test_layout,
                ImportRunRequest {
                    force_reimport: false,
                    manual_roots: vec![manual_root],
                    disabled_roots: Vec::new(),
                    resolutions: preview
                        .profiles
                        .iter()
                        .map(|profile| ImportResolution {
                            profile_root: profile.profile_root.clone(),
                            action: "import".to_string(),
                            account_id: profile.account_id.clone(),
                        })
                        .collect(),
                },
            )?;

            let source_id = result
                .profiles
                .first()
                .and_then(|profile| profile.source_id.as_deref())
                .ok_or_else(|| "imported source id missing".to_string())?;
            let source = load_source_profile_by_id(connection, source_id)?;
            let hint = source
                .sync_options
                .instagram
                .and_then(|instagram| instagram.user_id_hint);
            Ok((source.handle, hint))
        })
        .expect("legacy import should succeed");

        assert_eq!(handle, "franjuda", "imported handle should use TrueName");
        assert_eq!(
            user_id_hint.as_deref(),
            Some("17443084061"),
            "imported sync options should store UserID as user_id_hint"
        );
    }

    #[test]
    fn scrawler_import_falls_back_to_user_name_when_true_name_is_empty() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let profile_root = temp_dir.path().join("legacy-profile");
        let user_xml_path = create_legacy_instagram_profile_root_full(
            &profile_root,
            "instagram-account",
            "original_handle",
            Some(""),
            None,
            None,
        )
        .expect("legacy profile fixture");

        let profile =
            parse_legacy_instagram_profile_xml(&user_xml_path).expect("legacy xml should parse");
        let handle = legacy_instagram_profile_handle(&profile, "folder_name");

        assert_eq!(
            handle, "original_handle",
            "handle should fall back to UserName when TrueName is empty"
        );
    }

    #[test]
    fn parse_retry_after_duration_supports_seconds() {
        let value = reqwest::header::HeaderValue::from_static("120");
        let parsed = parse_retry_after_duration(Some(&value)).expect("retry-after seconds");
        assert_eq!(parsed.as_secs(), 120);
    }

    #[test]
    fn parse_retry_after_duration_supports_http_date() {
        let future = (Utc::now() + Duration::seconds(90)).to_rfc2822();
        let value = reqwest::header::HeaderValue::from_str(&future).expect("header value");
        let parsed = parse_retry_after_duration(Some(&value)).expect("retry-after date");
        assert!(
            parsed.as_secs() >= 1,
            "retry-after date parsing should return a positive delay"
        );
    }

    #[test]
    fn classify_instagram_identity_error_detects_private_or_restricted() {
        let error = "Instagram request 'https://www.instagram.com/api/v1/feed/user/demo/username/?count=30' returned 403: {\"message\":\"login required for private account\"}";
        assert_eq!(
            classify_instagram_identity_error(error),
            InstagramIdentityErrorClassification::PrivateOrRestricted
        );
    }

    #[test]
    fn classify_instagram_identity_error_detects_unresolvable_profiles() {
        let error = "Instagram request 'https://www.instagram.com/api/v1/feed/user/missing-user/username/?count=30' returned 404: Not Found";
        assert_eq!(
            classify_instagram_identity_error(error),
            InstagramIdentityErrorClassification::UsernameUnresolvable
        );
    }

    #[test]
    fn classify_instagram_identity_error_uses_probe_marker_for_private_profiles() {
        let error = "Instagram timeline response is missing user data. [identity_probe=instagram_profile_private_or_restricted] Profile accessibility probe confirmed `web_profile_info.data.user.is_private=true`.";
        assert_eq!(
            classify_instagram_identity_error(error),
            InstagramIdentityErrorClassification::PrivateOrRestricted
        );
    }

    #[test]
    fn classify_instagram_identity_error_uses_probe_marker_for_unresolvable_profiles() {
        let error = "Instagram timeline response is missing user data. [identity_probe=instagram_username_unresolvable] Profile accessibility probe returned no user object.";
        assert_eq!(
            classify_instagram_identity_error(error),
            InstagramIdentityErrorClassification::UsernameUnresolvable
        );
    }

    #[test]
    fn availability_rate_limit_abort_ignores_inconclusive_probe_429() {
        let error = "Instagram timeline response is missing user data. Profile accessibility probe returned 429 Too Many Requests.";
        assert!(
            !instagram_error_indicates_availability_abort_rate_limit(error),
            "inconclusive probe 429 should not abort the full availability batch"
        );
    }

    #[test]
    fn availability_rate_limit_abort_keeps_explicit_endpoint_429() {
        let error =
            "Instagram request 'https://www.instagram.com/api/v1/feed/user/demo/username/?count=30' returned 429: Too Many Requests";
        assert!(
            instagram_error_indicates_availability_abort_rate_limit(error),
            "explicit endpoint 429 should still abort the availability batch"
        );
    }

    #[test]
    fn decide_instagram_availability_action_keeps_private_marker_even_when_hint_fallback_resolves()
    {
        let previous = "demo_user";
        let primary = Err("Instagram timeline response is missing user data. [identity_probe=instagram_profile_private_or_restricted] Profile accessibility probe confirmed `web_profile_info.data.user.is_private=true`.".to_string());
        let fallback = Ok(instagram_connector::InstagramProfileIdentity {
            username: "demo_user".to_string(),
            user_id: "123".to_string(),
        });

        assert_eq!(
            decide_instagram_availability_action(previous, &primary, Some(&fallback)),
            InstagramAvailabilityAction::MarkPrivateOrRestricted {
                resolved_handle: Some("demo_user".to_string()),
                handle_changed: false
            }
        );
    }

    #[test]
    fn decide_instagram_availability_action_clears_unresolvable_when_hint_fallback_resolves_username(
    ) {
        let previous = "old_name";
        let primary = Err("Instagram request 'https://www.instagram.com/api/v1/feed/user/old_name/username/?count=30' returned 404: Not Found".to_string());
        let fallback = Ok(instagram_connector::InstagramProfileIdentity {
            username: "new_name".to_string(),
            user_id: "999".to_string(),
        });

        assert_eq!(
            decide_instagram_availability_action(previous, &primary, Some(&fallback)),
            InstagramAvailabilityAction::Resolved {
                resolved_handle: "new_name".to_string(),
                handle_changed: true
            }
        );
    }

    #[test]
    fn decide_instagram_availability_action_prefers_anchored_identity_after_handle_reuse() {
        let primary = Ok(instagram_connector::InstagramProfileIdentity {
            username: "old_name".to_string(),
            user_id: "new-owner-id".to_string(),
        });
        let fallback = Ok(instagram_connector::InstagramProfileIdentity {
            username: "renamed_original".to_string(),
            user_id: "stable-owner-id".to_string(),
        });

        assert_eq!(
            decide_instagram_availability_action("old_name", &primary, Some(&fallback)),
            InstagramAvailabilityAction::Resolved {
                resolved_handle: "renamed_original".to_string(),
                handle_changed: true
            }
        );
    }

    #[test]
    fn decide_instagram_availability_action_rejects_reused_handle_when_anchor_lookup_fails() {
        let primary = Ok(instagram_connector::InstagramProfileIdentity {
            username: "old_name".to_string(),
            user_id: "new-owner-id".to_string(),
        });
        let fallback = Err("stable identity lookup failed".to_string());

        assert!(matches!(
            decide_instagram_availability_action("old_name", &primary, Some(&fallback)),
            InstagramAvailabilityAction::Failed(message)
                if message.contains("different Instagram account")
        ));
    }

    #[test]
    fn set_source_sync_problem_can_preserve_ready_for_download_state() {
        let (_temp_dir, layout) = create_test_layout();

        let (ready_for_download, sync_problem_code) =
            with_workspace_layout(layout, |connection, test_layout| {
                let source = sample_source("source-1", "instagram", None);
                upsert_source_profile_with_connection(connection, test_layout, source)?;
                set_source_sync_problem(
                    connection,
                    "source-1",
                    "instagram_profile_private_or_restricted",
                    "private profile",
                    "2026-03-20T00:00:00Z",
                    false,
                )?;

                let result = connection
                    .query_row(
                        "SELECT ready_for_download, sync_problem_code
                         FROM source_profiles
                         WHERE id = ?1",
                        params!["source-1"],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
                    )
                    .map_err(|error| error.to_string())?;
                Ok(result)
            })
            .expect("non-blocking sync problem should persist");

        assert_eq!(
            ready_for_download, 1,
            "non-blocking profile marker must not pause source readiness"
        );
        assert_eq!(
            sync_problem_code.as_deref(),
            Some("instagram_profile_private_or_restricted")
        );
    }

    #[test]
    fn clear_source_sync_problem_restores_ready_for_download() {
        let (_temp_dir, layout) = create_test_layout();

        let (ready_for_download, sync_problem_code) =
            with_workspace_layout(layout, |connection, test_layout| {
                let source = sample_source("source-1", "instagram", None);
                upsert_source_profile_with_connection(connection, test_layout, source)?;

                // Mark a blocking sync problem (disables ready_for_download).
                set_source_sync_problem(
                    connection,
                    "source-1",
                    "instagram_username_unresolvable",
                    "profile unavailable",
                    "2026-03-20T00:00:00Z",
                    true,
                )?;

                // Verify ready_for_download is now 0.
                let before: i64 = connection
                    .query_row(
                        "SELECT ready_for_download FROM source_profiles WHERE id = ?1",
                        params!["source-1"],
                        |row| row.get(0),
                    )
                    .map_err(|error| error.to_string())?;
                assert_eq!(
                    before, 0,
                    "blocking problem should disable ready_for_download"
                );

                // Clear the problem — should restore ready_for_download.
                clear_source_sync_problem(connection, "source-1", "2026-03-20T01:00:00Z")?;

                let result = connection
                    .query_row(
                        "SELECT ready_for_download, sync_problem_code
                         FROM source_profiles
                         WHERE id = ?1",
                        params!["source-1"],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
                    )
                    .map_err(|error| error.to_string())?;
                Ok(result)
            })
            .expect("clear sync problem should succeed");

        assert_eq!(
            ready_for_download, 1,
            "clearing sync problem must restore ready_for_download"
        );
        assert_eq!(sync_problem_code, None, "sync problem code must be cleared");
    }

    #[test]
    fn running_sync_plan_now_persists_runtime_history_and_notification() {
        let (_temp_dir, layout) = create_test_layout();

        let (snapshot, source_ids) = with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;
            seed_instagram_session(connection, test_layout, "account-1")?;
            upsert_source_profile_with_connection(
                connection,
                test_layout,
                sample_source("source-1", "instagram", Some("account-1")),
            )?;
            upsert_scheduler_set_with_connection(connection, sample_scheduler_set("set-1", true))?;
            upsert_sync_plan_with_connection(
                connection,
                sample_sync_plan("plan-1", "set-1", "automatic", 30, 0),
            )?;

            let source_ids = run_sync_plan_now_with_connection(
                connection,
                test_layout,
                "plan-1",
                "manual",
                "2026-03-10T00:15:00Z",
            )?;
            Ok((load_snapshot(connection, test_layout)?, source_ids))
        })
        .expect("manual sync-plan run should succeed");

        // O plano resolve as fontes e devolve os ids a enfileirar; não roda
        // o sync inline.
        assert_eq!(source_ids, vec!["source-1".to_string()]);
        assert_eq!(
            snapshot.sync_plan_runs.len(),
            1,
            "expected persisted plan run history"
        );
        assert_eq!(snapshot.sync_plan_runs[0].status, "succeeded");
        assert_eq!(snapshot.sync_plan_runs[0].source_count, 1);
        assert_eq!(
            snapshot.scheduler_sets[0].plans[0].last_run_status,
            "succeeded"
        );
        assert!(
            snapshot.scheduler_sets[0].plans[0]
                .last_run_summary
                .as_deref()
                .is_some_and(|value| value.contains("Queued 1 source syncs")),
            "expected last run summary to report the queued count"
        );
    }

    #[test]
    fn running_sync_plan_now_queues_sources_even_when_in_cooldown() {
        // O plano só resolve e enfileira; o skip por cooldown da conta acontece
        // depois, quando o worker da fila executa a fonte (não mais inline).
        let (_temp_dir, layout) = create_test_layout();

        let (snapshot, source_ids) = with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;
            seed_instagram_session(connection, test_layout, "account-1")?;
            save_provider_account_settings_with_connection(
                connection,
                test_layout,
                "account-1".to_string(),
                vec![ProviderAccountSettingValue {
                    setting_key: INSTAGRAM_SYNC_COOLDOWN_UNTIL_SETTING_KEY.to_string(),
                    value_kind: ProviderAccountSettingValueKind::String,
                    string_value: Some("2030-01-01T00:00:00Z".to_string()),
                    json_value: None,
                }],
            )?;
            upsert_source_profile_with_connection(
                connection,
                test_layout,
                sample_source("source-1", "instagram", Some("account-1")),
            )?;
            upsert_scheduler_set_with_connection(connection, sample_scheduler_set("set-1", true))?;
            upsert_sync_plan_with_connection(
                connection,
                sample_sync_plan("plan-1", "set-1", "automatic", 30, 0),
            )?;

            let source_ids = run_sync_plan_now_with_connection(
                connection,
                test_layout,
                "plan-1",
                "manual",
                "2026-03-10T00:15:00Z",
            )?;
            Ok((load_snapshot(connection, test_layout)?, source_ids))
        })
        .expect("manual sync-plan run should queue gracefully");

        // A fonte é resolvida/enfileirada (nada roda inline), então não há run
        // de sync ainda; o registro do plano marca quantas foram enfileiradas.
        assert_eq!(source_ids, vec!["source-1".to_string()]);
        assert_eq!(snapshot.source_sync_runs.len(), 0);
        assert_eq!(snapshot.sync_plan_runs.len(), 1);
        assert_eq!(snapshot.sync_plan_runs[0].status, "succeeded");
        assert!(
            snapshot.scheduler_sets[0].plans[0]
                .last_run_summary
                .as_deref()
                .is_some_and(|value| value.contains("Queued 1 source syncs")),
            "expected the sync-plan summary to report the queued count"
        );
    }

    #[test]
    fn scheduler_tick_respects_startup_delay_across_restarts() {
        let (_temp_dir, layout) = create_test_layout();

        with_workspace_layout(layout.clone(), |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;
            seed_instagram_session(connection, test_layout, "account-1")?;
            upsert_source_profile_with_connection(
                connection,
                test_layout,
                sample_source("source-1", "instagram", Some("account-1")),
            )?;
            upsert_scheduler_set_with_connection(connection, sample_scheduler_set("set-1", true))?;
            upsert_sync_plan_with_connection(
                connection,
                sample_sync_plan("plan-1", "set-1", "automatic", 30, 30),
            )?;
            record_scheduler_launch_with_connection(connection, "2026-03-10T00:00:00Z")
        })
        .expect("seed scheduler state");

        let before_due = with_workspace_layout(layout.clone(), |connection, test_layout| {
            process_scheduler_tick_with_connection(connection, test_layout, "2026-03-10T00:10:00Z")?;
            load_snapshot(connection, test_layout)
        })
        .expect("tick before startup delay");

        assert_eq!(
            before_due.sync_plan_runs.len(),
            0,
            "plan should not run before startup delay"
        );
        assert_eq!(
            before_due.scheduler_sets[0].plans[0].next_due_at.as_deref(),
            Some("2026-03-10T00:30:00+00:00")
        );

        let after_due = with_workspace_layout(layout, |connection, test_layout| {
            process_scheduler_tick_with_connection(connection, test_layout, "2026-03-10T00:31:00Z")?;
            load_snapshot(connection, test_layout)
        })
        .expect("tick after restart should still honor persisted launch state");

        assert_eq!(
            after_due.sync_plan_runs.len(),
            1,
            "plan should run once after startup delay"
        );
        assert_eq!(after_due.sync_plan_runs[0].status, "succeeded");
        assert_eq!(
            after_due.scheduler_sets[0].plans[0].last_run_status,
            "succeeded"
        );
    }

    #[test]
    fn pause_resume_and_skip_sync_plan_update_runtime_state() {
        let (_temp_dir, layout) = create_test_layout();

        let snapshot = with_workspace_layout(layout, |connection, test_layout| {
            upsert_scheduler_set_with_connection(connection, sample_scheduler_set("set-1", true))?;
            upsert_sync_plan_with_connection(
                connection,
                sample_sync_plan("plan-1", "set-1", "automatic", 30, 0),
            )?;
            record_scheduler_launch_with_connection(connection, "2026-03-10T00:00:00Z")?;

            set_sync_plan_pause_with_connection(
                connection,
                &SetSyncPlanPauseInput {
                    id: "plan-1".to_string(),
                    pause_mode: "indefinite".to_string(),
                    pause_until: None,
                },
                "2026-03-10T00:01:00Z",
            )?;
            clear_sync_plan_pause_with_connection(connection, "plan-1", "2026-03-10T00:05:00Z")?;
            skip_sync_plan_with_connection(
                connection,
                &SkipSyncPlanInput {
                    id: "plan-1".to_string(),
                    mode: "next".to_string(),
                    minutes: None,
                    until: None,
                },
                "2026-03-10T00:05:00Z",
            )?;
            load_snapshot(connection, test_layout)
        })
        .expect("pause/resume/skip should succeed");

        let plan = &snapshot.scheduler_sets[0].plans[0];
        assert!(!plan.paused, "plan should be resumed");
        assert_eq!(plan.last_run_status, "skipped");
        assert!(
            plan.skip_until.is_some(),
            "skip should persist a skip-until timestamp"
        );
        assert!(
            plan.last_run_summary
                .as_deref()
                .is_some_and(|summary| summary.contains("Skipped")),
            "skip should still leave an operator-visible runtime summary"
        );
    }
}

fn load_sources(connection: &Connection) -> Result<Vec<SourceProfile>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id, provider, source_kind, handle, display_name, account_id, labels_json, ready_for_download, sync_options_json, profile_image_path, profile_image_custom, remote_state, is_subscription, last_synced_at, sync_problem_code, sync_problem_message, sync_problem_at, created_at, group_id, importer_id, imported_at
             FROM source_profiles
             WHERE deleted_at IS NULL
             ORDER BY provider, display_name",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let provider: String = row.get(1)?;
            Ok(SourceProfile {
                id: row.get(0)?,
                provider: provider.clone(),
                source_kind: row.get(2)?,
                handle: row.get(3)?,
                display_name: row.get(4)?,
                account_id: row.get(5)?,
                group_id: row.get(18)?,
                labels: from_json_array(row.get::<_, String>(6)?),
                ready_for_download: row.get::<_, i64>(7)? == 1,
                sync_options: deserialize_source_sync_options(&provider, &row.get::<_, String>(8)?),
                profile_image_path: row.get(9)?,
                profile_image_custom: row.get::<_, i64>(10).unwrap_or(0) != 0,
                remote_state: row
                    .get::<_, String>(11)
                    .unwrap_or_else(|_| "exists".to_string()),
                is_subscription: row.get::<_, i64>(12).unwrap_or(0) != 0,
                last_synced_at: row.get(13).ok(),
                sync_problem_code: row.get(14).ok(),
                sync_problem_message: row.get(15).ok(),
                sync_problem_at: row.get(16).ok(),
                created_at: row.get(17).ok(),
                importer_id: row.get(19).ok(),
                imported_at: row.get(20).ok(),
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn load_source_sync_runs(connection: &Connection) -> Result<Vec<SourceSyncRun>, String> {
    let mut statement = connection
        .prepare(
            "SELECT
                id,
                source_id,
                account_id,
                provider,
                tool,
                trigger,
                status,
                summary,
                command_preview,
                manifest_summary_json,
                degraded_capabilities_json,
                started_at,
                finished_at
             FROM source_sync_runs
             ORDER BY finished_at DESC, created_at DESC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(SourceSyncRun {
                id: row.get(0)?,
                source_id: row.get(1)?,
                account_id: row.get(2)?,
                provider: row.get(3)?,
                tool: row.get(4)?,
                trigger: row.get(5)?,
                status: row.get(6)?,
                summary: row.get(7)?,
                command_preview: row.get(8)?,
                manifest_summary_json: row.get(9)?,
                degraded_capabilities: from_json_array(row.get::<_, String>(10)?),
                started_at: row.get(11)?,
                finished_at: row.get(12)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn load_account_sync_runs(connection: &Connection) -> Result<Vec<AccountSyncRun>, String> {
    let mut statement = connection
        .prepare(
            "SELECT
                id,
                account_id,
                provider,
                sync_scope,
                tool,
                trigger,
                status,
                summary,
                command_preview,
                started_at,
                finished_at
             FROM account_sync_runs
             ORDER BY finished_at DESC, created_at DESC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(AccountSyncRun {
                id: row.get(0)?,
                account_id: row.get(1)?,
                provider: row.get(2)?,
                sync_scope: row.get(3)?,
                tool: row.get(4)?,
                trigger: row.get(5)?,
                status: row.get(6)?,
                summary: row.get(7)?,
                command_preview: row.get(8)?,
                started_at: row.get(9)?,
                finished_at: row.get(10)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn load_scheduler_sets(connection: &Connection) -> Result<Vec<SchedulerSet>, String> {
    let mut set_statement = connection
        .prepare("SELECT id, name, is_active FROM scheduler_sets ORDER BY is_active DESC, name")
        .map_err(|error| error.to_string())?;
    let set_rows = set_statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? == 1,
            ))
        })
        .map_err(|error| error.to_string())?;

    let mut scheduler_sets = Vec::new();
    for row in set_rows {
        let (id, name, active) = row.map_err(|error| error.to_string())?;
        scheduler_sets.push(SchedulerSet {
            id: id.clone(),
            name,
            active,
            plans: load_sync_plans(connection, &id)?,
        });
    }
    Ok(scheduler_sets)
}

fn load_sync_plans(
    connection: &Connection,
    scheduler_set_id: &str,
) -> Result<Vec<SyncPlan>, String> {
    let mut statement = connection.prepare("SELECT id, scheduler_set_id, name, enabled, mode, interval_minutes, startup_delay_minutes, notification_mode, target_filter, sort_index, paused, pause_mode, pause_until, skip_until, last_run_at, last_run_status, last_run_summary, next_due_at, notifications_json, criteria_json FROM sync_plans WHERE scheduler_set_id = ?1 ORDER BY sort_index, name").map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![scheduler_set_id], |row| map_sync_plan_row(row))
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn map_sync_plan_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SyncPlan> {
    let notification_mode = row.get::<_, String>(7)?;
    let target_filter = row.get::<_, String>(8)?;
    let notifications_json = row.get::<_, String>(18)?;
    let criteria_json = row.get::<_, String>(19)?;
    Ok(SyncPlan {
        id: row.get(0)?,
        scheduler_set_id: row.get(1)?,
        name: row.get(2)?,
        enabled: row.get::<_, i64>(3)? == 1,
        mode: row.get(4)?,
        interval_minutes: row.get::<_, i64>(5)? as u32,
        startup_delay_minutes: row.get::<_, i64>(6)? as u32,
        notification_mode: notification_mode.clone(),
        target_filter: target_filter.clone(),
        sort_index: row.get::<_, i64>(9).unwrap_or(0),
        paused: row.get::<_, i64>(10)? == 1,
        pause_mode: row
            .get::<_, String>(11)
            .unwrap_or_else(|_| "disabled".to_string()),
        pause_until: row.get(12).ok(),
        skip_until: row.get(13)?,
        last_run_at: row.get(14)?,
        last_run_status: row.get(15)?,
        last_run_summary: row.get(16)?,
        next_due_at: row.get(17)?,
        notifications: parse_scheduler_notifications(&notifications_json, &notification_mode),
        criteria: parse_scheduler_criteria(&criteria_json, &target_filter),
    })
}

fn load_scheduler_groups(connection: &Connection) -> Result<Vec<SchedulerGroup>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id, name, sort_index, criteria_json
             FROM scheduler_groups
             ORDER BY sort_index, name",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let criteria_json = row.get::<_, String>(3)?;
            Ok(SchedulerGroup {
                id: row.get(0)?,
                name: row.get(1)?,
                sort_index: row.get(2)?,
                criteria: serde_json::from_str(&criteria_json).unwrap_or_default(),
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn load_sync_plan_runs(connection: &Connection) -> Result<Vec<SyncPlanRun>, String> {
    let mut statement = connection
        .prepare(
            "SELECT
                id,
                plan_id,
                scheduler_set_id,
                trigger,
                status,
                summary,
                source_count,
                started_at,
                finished_at
             FROM sync_plan_runs
             ORDER BY finished_at DESC, created_at DESC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(SyncPlanRun {
                id: row.get(0)?,
                plan_id: row.get(1)?,
                scheduler_set_id: row.get(2)?,
                trigger: row.get(3)?,
                status: row.get(4)?,
                summary: row.get(5)?,
                source_count: row.get::<_, i64>(6)? as u32,
                started_at: row.get(7)?,
                finished_at: row.get(8)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn load_app_settings(connection: &Connection) -> Result<Vec<AppSetting>, String> {
    let mut statement = connection
        .prepare("SELECT key, value FROM app_settings ORDER BY key")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(AppSetting {
                key: row.get(0)?,
                value: row.get(1)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

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
