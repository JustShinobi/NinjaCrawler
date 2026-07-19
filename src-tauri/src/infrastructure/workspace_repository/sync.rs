use super::*;

pub(super) const INSTAGRAM_SYNC_RETRY_AFTER_FALLBACK_SECS: i64 = 10 * 60;
pub(super) const INSTAGRAM_SYNC_COOLDOWN_UNTIL_SETTING_KEY: &str = "instagram.sync.cooldownUntil";
pub const TWITTER_FULL_TIMELINE_BACKFILL_RUN_MODE: &str = "twitter_full_timeline_backfill";
const TWITTER_RATE_LIMIT_HOLD_SECS: i64 = 15 * 60 + 30;
const TWITTER_INCREMENTAL_OVERLAP_SECS: i64 = 7 * 24 * 60 * 60;
const TWITTER_PERIODIC_FULL_SCAN_SECS: i64 = 7 * 24 * 60 * 60;
pub(super) const SOURCE_SYNC_PROGRESS_POLL_MS: u64 = 900;
/// `run_mode` da ação pontual "Refresh media stats" (TikTok): roda um sync
/// normal com re-coleta de estatísticas da mídia existente, sem persistir nada
/// nas opções do perfil.
pub(super) const TIKTOK_REFRESH_MEDIA_STATS_RUN_MODE: &str = "refresh_media_stats";

/// Lê uma setting em lista separada por vírgulas (trim, sem vazios).
pub(super) fn parse_csv_provider_setting(
    settings: &HashMap<String, String>,
    key: &str,
) -> Vec<String> {
    settings
        .get(key)
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Campos de tolerância a erros do connector Instagram, lidos das settings da
/// conta (skip list, gate de log e limite de aviso da seção Tagged).
pub(super) fn instagram_error_policy_settings(
    settings: &HashMap<String, String>,
) -> (Vec<String>, bool, u32) {
    let exclude = parse_csv_provider_setting(settings, "instagram.errors.skipErrorsExclude");
    let log_skipped = parse_bool_setting(
        settings
            .get("instagram.errors.addSkippedErrorsToLog")
            .map(String::as_str),
        true,
    );
    let tagged_limit =
        parse_u64_provider_setting(settings, "instagram.errors.taggedNotifyLimit", 25)
            .min(u64::from(u32::MAX)) as u32;
    (exclude, log_skipped, tagged_limit)
}

/// Result of a bidirectional ledger ↔ disk reconcile for a TikTok source.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct TikTokLedgerReconcileStats {
    pub recovered_from_disk: usize,
    pub purged_media_rows: usize,
    pub purged_post_rows: usize,
}

/// Two-way ledger reconcile before TikTok sync (and after partial delete-with-media):
/// 1. purge media/post ledger rows whose files no longer exist on disk
/// 2. recover on-disk files that are missing from the ledger
///
/// Without (1), a failed delete-with-media (disk wiped, profile still in DB) leaves
/// post keys in the ledger and the connector skips every post as "already have it".
pub(super) fn reconcile_tiktok_provider_ledgers(
    connection: &Connection,
    profile_root: &Path,
    account_id: &str,
    source_id: &str,
    source_handle: &str,
    timestamp: &str,
) -> Result<TikTokLedgerReconcileStats, String> {
    // Purge liked media that earlier builds wrongly reconciled as "timeline".
    // Those rows share the physical file (and therefore the gallery's relative
    // path key) with the likes runtime's own "likes" row, so leaving them behind
    // keeps leaking liked media into the Timeline tab even after this reconcile
    // stopped producing them. The likes runtime's row (media_section = "likes")
    // is untouched; if no likes row exists the gallery falls back to the on-disk
    // `Liked/` folder and still classifies it correctly.
    purge_reconciled_tiktok_liked_timeline_rows(connection, source_id)?;

    let (purged_media_rows, purged_post_rows) =
        purge_provider_ledgers_missing_on_disk(connection, "tiktok", source_id, profile_root)?;

    let files = collect_media_file_paths(profile_root)?;
    let mut observed_by_post: HashMap<String, twitter_connector::ObservedTwitterPost> =
        HashMap::new();
    let mut recovered_media = Vec::new();
    let mut seen_media_keys = HashSet::new();

    for file_path in files {
        let Some(file_name) = file_path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let file_name = file_name.to_string();
        if !is_recoverable_tiktok_media_path(profile_root, &file_path, &file_name) {
            continue;
        }
        let Some(post_id) = tiktok_post_id_from_media_file_name(&file_name) else {
            continue;
        };
        let Some(media_type) = tiktok_media_type_from_file_name(&file_name) else {
            continue;
        };
        let provider_media_key = file_name.to_ascii_lowercase();
        if !seen_media_keys.insert((provider_media_key.clone(), media_type.clone())) {
            continue;
        }

        observed_by_post.entry(post_id.clone()).or_insert_with(|| {
            twitter_connector::ObservedTwitterPost {
                provider_post_key: post_id.clone(),
                media_section: "timeline".to_string(),
            }
        });
        recovered_media.push(twitter_connector::DownloadedTwitterMedia {
            file_path,
            media_type,
            media_section: "timeline".to_string(),
            provider_media_key,
            provider_post_key: post_id.clone(),
            captured_at_timestamp: tiktok_timestamp_from_post_id(&post_id),
            final_file_name: file_name,
        });
    }

    let mut recovered_from_disk = 0usize;
    if !recovered_media.is_empty() {
        let mut observed_posts = observed_by_post.into_values().collect::<Vec<_>>();
        observed_posts.sort_by(|left, right| left.provider_post_key.cmp(&right.provider_post_key));
        upsert_provider_sync_post_ledger_entries(
            connection,
            "tiktok",
            source_id,
            account_id,
            source_handle,
            &observed_posts,
            timestamp,
        )?;
        upsert_provider_sync_media_ledger_entries(
            connection,
            &ProviderSyncMediaScope {
                provider: "tiktok",
                source_id,
                account_id,
                source_handle,
                profile_root,
                timestamp,
            },
            &recovered_media,
        )?;
        recovered_from_disk = recovered_media.len();
    }

    Ok(TikTokLedgerReconcileStats {
        recovered_from_disk,
        purged_media_rows,
        purged_post_rows,
    })
}

/// Drop media-ledger rows whose relative_path is gone from disk, then drop post-
/// ledger keys that no longer have any media-ledger rows for this source.
/// Returns `(purged_media_rows, purged_post_rows)`.
pub(super) fn purge_provider_ledgers_missing_on_disk(
    connection: &Connection,
    provider: &str,
    source_id: &str,
    profile_root: &Path,
) -> Result<(usize, usize), String> {
    let provider = provider.to_ascii_lowercase();
    let mut statement = connection
        .prepare(
            "SELECT relative_path, provider_post_key FROM provider_sync_media_ledger
             WHERE provider = ?1 AND source_id = ?2",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![&provider, source_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|error| error.to_string())?;

    let mut stale_paths = Vec::new();
    for row in rows {
        let (relative_path, _post_key) = row.map_err(|error| error.to_string())?;
        let normalized = relative_path.replace('\\', "/");
        let abs = profile_root.join(normalized.trim_start_matches('/'));
        if !abs.is_file() {
            stale_paths.push(relative_path);
        }
    }

    let mut purged_media = 0usize;
    for relative_path in &stale_paths {
        purged_media += connection
            .execute(
                "DELETE FROM provider_sync_media_ledger
                 WHERE provider = ?1 AND source_id = ?2 AND relative_path = ?3",
                params![&provider, source_id, relative_path],
            )
            .map_err(|error| error.to_string())? as usize;
    }

    // Post keys that still have at least one media-ledger row remain "known".
    // Keys that lost every media row are stale (typical after partial with-media
    // delete) and would block re-download forever if left behind.
    let remaining_post_keys: HashSet<String> = {
        let mut statement = connection
            .prepare(
                "SELECT DISTINCT provider_post_key FROM provider_sync_media_ledger
                 WHERE provider = ?1 AND source_id = ?2
                   AND provider_post_key IS NOT NULL
                   AND TRIM(provider_post_key) != ''",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![&provider, source_id], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        let mut keys = HashSet::new();
        for row in rows {
            let key = row.map_err(|error| error.to_string())?;
            keys.insert(key);
        }
        keys
    };

    let all_post_keys: Vec<String> = {
        let mut statement = connection
            .prepare(
                "SELECT provider_post_key FROM provider_sync_post_ledger
                 WHERE provider = ?1 AND source_id = ?2",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![&provider, source_id], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        let mut keys = Vec::new();
        for row in rows {
            keys.push(row.map_err(|error| error.to_string())?);
        }
        keys
    };

    let mut purged_posts = 0usize;
    for post_key in all_post_keys {
        if remaining_post_keys.contains(&post_key) {
            continue;
        }
        // If the post id still has a matching file on disk under a path that
        // was never ledgered, keep the post key (disk recovery runs next).
        // Otherwise purge so the next sync re-downloads.
        let keep_for_disk = post_key_has_media_on_disk(profile_root, &post_key);
        if keep_for_disk {
            continue;
        }
        purged_posts += connection
            .execute(
                "DELETE FROM provider_sync_post_ledger
                 WHERE provider = ?1 AND source_id = ?2 AND provider_post_key = ?3",
                params![&provider, source_id, post_key],
            )
            .map_err(|error| error.to_string())? as usize;
    }

    Ok((purged_media, purged_posts))
}

fn post_key_has_media_on_disk(profile_root: &Path, post_key: &str) -> bool {
    let needle = post_key.trim();
    if needle.is_empty() || !profile_root.is_dir() {
        return false;
    }
    collect_media_file_paths(profile_root)
        .unwrap_or_default()
        .into_iter()
        .any(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(needle))
        })
}

/// Removes the bogus `media_section = "timeline"` rows that older builds wrote
/// for liked media (keyed by file name, with a lowercased `liked/...` relative
/// path). SQLite's `LIKE` is ASCII case-insensitive, so `'liked/%'` also matches
/// the likes runtime's own `Liked/...` path — the `media_section = 'timeline'`
/// filter is what keeps that legitimate `"likes"` row intact.
fn purge_reconciled_tiktok_liked_timeline_rows(
    connection: &Connection,
    source_id: &str,
) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM provider_sync_media_ledger
             WHERE provider = 'tiktok'
               AND source_id = ?1
               AND media_section = 'timeline'
               AND (relative_path LIKE 'liked/%' OR relative_path LIKE 'likes/%')",
            params![source_id],
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn is_recoverable_tiktok_media_path(profile_root: &Path, path: &Path, file_name: &str) -> bool {
    if is_profile_image_file(file_name) {
        return false;
    }
    let relative = path.strip_prefix(profile_root).unwrap_or(path);
    // This reconcile only recovers timeline media that is on disk but missing
    // from the ledger, so it hard-codes `media_section = "timeline"`. Liked
    // videos live under `Liked/` (new) or `Likes/` (legacy) and are owned by
    // the likes runtime, which already writes them with `media_section =
    // "likes"`. Recovering them here would emit a competing ledger row (keyed by
    // file name instead of `liked_<id>`) that shadows the "likes" section on the
    // same `relative_path`, leaking liked media into the Timeline tab.
    if relative.components().any(|component| {
        let segment = component.as_os_str().to_string_lossy();
        segment.eq_ignore_ascii_case(".thumbs")
            || segment.eq_ignore_ascii_case("cover")
            || segment.eq_ignore_ascii_case("settings")
            || segment.eq_ignore_ascii_case("liked")
            || segment.eq_ignore_ascii_case("likes")
    }) {
        return false;
    }
    tiktok_media_type_from_file_name(file_name).is_some()
}

fn tiktok_media_type_from_file_name(file_name: &str) -> Option<String> {
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())?
        .to_ascii_lowercase();
    match extension.as_str() {
        "mp4" | "webm" | "mkv" | "mov" | "m4v" => Some("video".to_string()),
        "jpg" | "jpeg" | "png" | "webp" | "heic" | "gif" => Some("image".to_string()),
        _ => None,
    }
}

pub(super) fn tiktok_post_id_from_media_file_name(file_name: &str) -> Option<String> {
    let mut current = String::new();
    for character in file_name.chars() {
        if character.is_ascii_digit() {
            current.push(character);
            continue;
        }
        if is_plausible_tiktok_post_id(&current) {
            return Some(current);
        }
        current.clear();
    }
    is_plausible_tiktok_post_id(&current).then_some(current)
}

fn is_plausible_tiktok_post_id(value: &str) -> bool {
    (18..=20).contains(&value.len()) && value.chars().all(|character| character.is_ascii_digit())
}

fn tiktok_timestamp_from_post_id(post_id: &str) -> Option<i64> {
    let parsed = post_id.parse::<u64>().ok()?;
    let timestamp = (parsed >> 32) as i64;
    (timestamp > 0).then_some(timestamp)
}

/// Monta o pacing de requests do Instagram a partir das settings da conta
/// (timers espelhados do SCrawler; defaults idem).
pub(super) fn instagram_request_pacing(
    settings: &HashMap<String, String>,
) -> instagram_connector::InstagramPacing {
    instagram_connector::InstagramPacing {
        base_delay_ms: parse_u64_provider_setting(settings, "instagram.timers.requestAnyMs", 1500),
        extra_delay_ms: parse_u64_provider_setting(settings, "instagram.timers.requestMs", 1000),
        counter_threshold: parse_u64_provider_setting(
            settings,
            "instagram.timers.requestCounter",
            10,
        )
        .min(u64::from(u32::MAX)) as u32,
        page_delay_ms: parse_u64_provider_setting(settings, "instagram.timers.postsLimitMs", 3000),
    }
}
/// Grava a identidade estável do Instagram diretamente no perfil. O histórico
/// de sync continua sendo uma fonte de recuperação para instalações antigas,
/// mas não deve ser a única âncora porque o schema dos resumos evolui.
pub(super) fn persist_instagram_user_id_hint(
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
pub(super) fn persist_twitter_user_id_hint(
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
pub(super) fn persist_tiktok_user_id_hint(
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
pub(super) fn source_sync_cancel_registry() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}
pub(super) fn register_source_sync_cancel_token(source_id: &str) -> Arc<AtomicBool> {
    let token = Arc::new(AtomicBool::new(false));
    if let Ok(mut registry) = source_sync_cancel_registry().lock() {
        registry.insert(source_id.to_string(), Arc::clone(&token));
    }
    token
}
pub(super) fn clear_source_sync_cancel_token(source_id: &str) {
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
        let mut tally = AvailabilityCheckTally::default();
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
                tally.failed += 1;
                tally.items.push(SourceAvailabilityCheckItem {
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
                tally.skipped += 1;
                tally.items.push(SourceAvailabilityCheckItem {
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
                tally.failed += 1;
                tally.items.push(SourceAvailabilityCheckItem {
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
                tally.failed += 1;
                tally.items.push(SourceAvailabilityCheckItem {
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
            let history_user_id_hint = load_latest_instagram_profile_user_id_hint(connection, &id)
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
                tally.failed += 1;
                tally.items.push(SourceAvailabilityCheckItem {
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
                    tally.failed += 1;
                    tally.items.push(SourceAvailabilityCheckItem {
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
                        tally.skipped += 1;
                        tally.items.push(build_availability_rate_limit_skipped_item(
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
                ) => Some(instagram_connector::resolve_profile_identity(
                    &request,
                    Some(hint),
                )),
                (
                    Some(InstagramIdentityErrorClassification::UsernameUnresolvable),
                    Some(hint),
                    false,
                ) => Some(instagram_connector::resolve_profile_identity(
                    &request,
                    Some(hint),
                )),
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
                        &mut tally,
                    )?;
                    if let Some(last) = tally.items.last_mut() {
                        last.message = format!(
                            "{} Also aborted batch due to Instagram rate limiting (429) during hint fallback: {}",
                            last.message, rate_limit_error
                        );
                    }

                    for remaining_source_id in unique_source_ids.iter().skip(source_index + 1) {
                        tally.skipped += 1;
                        tally.items.push(build_availability_rate_limit_skipped_item(
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
                &mut tally,
            )?;
        }

        let snapshot = load_snapshot(connection, layout)?;
        Ok(SourceAvailabilityCheckResult {
            snapshot,
            requested: unique_source_ids.len() as u32,
            processed: tally.items.len() as u32,
            unchanged: tally.unchanged,
            updated_handle: tally.updated_handle,
            marked_problem: tally.marked_problem,
            skipped: tally.skipped,
            failed: tally.failed,
            items: tally.items,
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
                   (job_key, source_id, trigger, run_mode, sync_options_override_json, queued_at, order_index)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6,
                          (SELECT COALESCE(MAX(order_index), 0) + 1 FROM source_sync_queue_jobs))
                 ON CONFLICT(job_key) DO UPDATE SET
                   trigger = excluded.trigger,
                   run_mode = excluded.run_mode,
                   sync_options_override_json = excluded.sync_options_override_json,
                   queued_at = excluded.queued_at",
                params![
                    job.job_key,
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
/// conforme a posição em `ordered_job_keys`. Só afeta os jobs informados; a
/// ordem relativa por provider é o que importa no restore.
pub fn persist_source_sync_queue_order(ordered_job_keys: &[String]) -> Result<(), String> {
    if ordered_job_keys.is_empty() {
        return Ok(());
    }
    with_workspace(|connection, _| {
        for (index, job_key) in ordered_job_keys.iter().enumerate() {
            connection
                .execute(
                    "UPDATE source_sync_queue_jobs SET order_index = ?2 WHERE job_key = ?1",
                    params![job_key, index as i64],
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    })
}
pub fn remove_source_sync_queue_job(job_key: &str) -> Result<(), String> {
    with_workspace(|connection, _| {
        connection
            .execute(
                "DELETE FROM source_sync_queue_jobs WHERE job_key = ?1",
                params![job_key],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}
pub fn load_persisted_source_sync_queue_jobs() -> Result<Vec<PersistedSourceSyncQueueJob>, String> {
    with_workspace(|connection, _| {
        let mut statement = connection
            .prepare(
                "SELECT job_key, source_id, trigger, run_mode, sync_options_override_json, queued_at
                 FROM source_sync_queue_jobs
                 ORDER BY order_index ASC, queued_at ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })
            .map_err(|error| error.to_string())?;

        let mut jobs = Vec::new();
        for row in rows {
            let (job_key, source_id, trigger, run_mode, override_json, queued_at) =
                row.map_err(|error| error.to_string())?;
            let sync_options_override = override_json
                .as_deref()
                .and_then(|json| serde_json::from_str::<SourceSyncOptions>(json).ok());
            jobs.push(PersistedSourceSyncQueueJob {
                job_key,
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
pub(super) fn run_source_sync_with_connection(
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
pub(super) fn execute_source_sync_with_connection(
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
            run_mode,
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
            run_mode,
            sync_options_override,
        );
    }
    if context.source.provider.eq_ignore_ascii_case("youtube") {
        let account_settings = load_provider_account_settings_map(connection, &context.account.id)?;
        return execute_youtube_source_sync_with_connection(
            connection,
            layout,
            &context,
            &account_settings,
            trigger,
            run_mode,
            sync_options_override,
        );
    }
    if context.source.provider.eq_ignore_ascii_case("vsco") {
        let account_settings = load_provider_account_settings_map(connection, &context.account.id)?;
        return execute_vsco_source_sync_with_connection(
            connection,
            layout,
            &context,
            &account_settings,
            trigger,
            run_mode,
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
                            RuntimeLogAnchor {
                                account_id: Some(&context.account.id),
                                provider: Some(&context.source.provider),
                                source_id: Some(&context.source.id),
                                source_handle: Some(&context.source.handle),
                            },
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
                        layout,
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
pub(super) fn execute_twitter_source_sync_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    context: &SourceSyncContext,
    settings: &HashMap<String, String>,
    trigger: &str,
    run_mode: Option<&str>,
    sync_options_override: Option<&SourceSyncOptions>,
) -> Result<SourceSyncOutcome, String> {
    let options = source_twitter_sync_options_with_override(&context.source, sync_options_override);
    let started_at = now_timestamp();

    let mut handle = sanitize_source_handle("twitter", &context.source.handle)
        .trim_start_matches('@')
        .to_string();
    if handle.is_empty() {
        return Err("Twitter source handle is empty.".to_string());
    }

    let profile_root =
        resolved_source_media_output_root_with_connection(connection, layout, &context.source)?;
    fs::create_dir_all(&profile_root).map_err(|error| error.to_string())?;
    cleanup_empty_media_artifacts(&profile_root)?;
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

    let mut ledger_post_keys =
        load_provider_sync_post_ledger_keys(connection, "twitter", &context.source.id)?;
    ledger_post_keys.extend(
        load_provider_deleted_post_keys(connection, "twitter", &context.source.id)
            .unwrap_or_default(),
    );
    let ledger_media_keys =
        load_provider_sync_media_ledger_keys(connection, "twitter", &context.source.id)?;
    let existing_relative_paths = load_existing_relative_media_paths(&profile_root)
        .into_iter()
        .filter_map(|file_name| twitter_connector::twitter_disk_asset_key(&file_name))
        .collect();

    let phase_scope = if run_mode
        .is_some_and(|value| value.eq_ignore_ascii_case(TWITTER_FULL_TIMELINE_BACKFILL_RUN_MODE))
    {
        "full_timeline_backfill"
    } else {
        "normal"
    };
    let (resume_cursors, completed_phase_sections) = load_provider_sync_resume_state(
        connection,
        "twitter",
        &context.source.id,
        &context.account.id,
        phase_scope,
    )?;
    let mut models =
        twitter_model_selection_for_phase(&options, run_mode, &completed_phase_sections);
    if !models.media && !models.profile && !models.search && !models.likes {
        clear_provider_sync_resume_scope(
            connection,
            "twitter",
            &context.source.id,
            &context.account.id,
            phase_scope,
        )?;
        models = twitter_model_selection_for_run(&options, run_mode);
    }
    let media_selection_signature = twitter_media_selection_signature(&options);
    let incremental_state = if phase_scope == "normal"
        && models.media
        && resume_cursors.is_empty()
        && completed_phase_sections.is_empty()
    {
        load_twitter_incremental_state(connection, &context.source.id, &media_selection_signature)?
    } else {
        None
    };
    let mut request = twitter_connector::TwitterConnectorRequest {
        handle: handle.clone(),
        gallery_dl_executable: PathBuf::from(&executable),
        cookie_file,
        user_agent,
        profile_root: profile_root.clone(),
        cache_root,
        models,
        resume_cursors,
        incremental_cutoff_timestamp: incremental_state
            .as_ref()
            .map(|state| state.cutoff_timestamp),
        ledger_post_keys,
        ledger_media_keys,
        existing_relative_paths,
        user_id_hint: options
            .user_id_hint
            .clone()
            .filter(|value| !value.trim().is_empty()),
        // Politica central: o connector devolve o controle ao NinjaCrawler no
        // primeiro 429; a fila persiste cursor, hold e retomada.
        abort_on_limit: true,
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
    let run_connector = |request: &twitter_connector::TwitterConnectorRequest| {
        twitter_connector::run_profile_sync(
            request,
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
        )
    };
    let mut execution = run_connector(&request);
    let mut redirected_from_handle = None;
    let redirect_handle = execution.as_ref().ok().and_then(|result| {
        twitter_handle_redirect(
            &handle,
            result.resolved_handle.as_deref(),
            result.rate_limited,
        )
    });
    if let Some(new_handle) = redirect_handle {
        let handle_updated_at = now_timestamp();
        if let Err(error) = update_twitter_source_handle_after_sync(
            connection,
            &context.source.id,
            &new_handle,
            &handle_updated_at,
        ) {
            execution = Err(format!(
                "Twitter handle change detected (@{handle} -> @{new_handle}) but updating the source failed: {error}"
            ));
        } else {
            log_runtime_event(
                layout,
                "sync.profile",
                "info",
                RuntimeLogAnchor {
                    account_id: Some(&context.account.id),
                    provider: Some(&context.source.provider),
                    source_id: Some(&context.source.id),
                    source_handle: Some(&context.source.handle),
                },
                format!(
                    "Twitter handle changed from '@{handle}' to '@{new_handle}'. Continuing the same sync automatically."
                ),
                None,
            );
            source_sync_runtime::report_source_sync_progress(
                &context.source.id,
                Some(0),
                Some("Handle updated".to_string()),
                Some(format!(
                    "Continuing automatically with the current handle @{new_handle}."
                )),
                true,
                Some(0),
            );
            let resume_clear = clear_provider_sync_resume_scope(
                connection,
                "twitter",
                &context.source.id,
                &context.account.id,
                phase_scope,
            );
            if let Err(error) = resume_clear {
                execution = Err(format!(
                    "Twitter handle was updated to @{new_handle}, but clearing the previous handle resume state failed: {error}"
                ));
            } else {
                request.resume_cursors.clear();
                request.models = twitter_model_selection_for_run(&options, run_mode);
                request.incremental_cutoff_timestamp = None;
                redirected_from_handle = Some(handle.clone());
                handle = new_handle;
                request.handle.clone_from(&handle);
                execution = run_connector(&request);
            }
        }
    }
    clear_source_sync_cancel_token(&context.source.id);
    let finished_at = now_timestamp();

    let enabled_models = [
        (request.models.media, "profile-media"),
        (request.models.profile, "full-timeline"),
        (request.models.search, "search"),
        (request.models.likes, "likes"),
    ]
    .into_iter()
    .filter_map(|(enabled, label)| enabled.then_some(label))
    .collect::<Vec<_>>()
    .join(",");
    let command_preview = format!(
        "internal.twitter models={} profile {} -> {}",
        enabled_models,
        handle,
        profile_root.display()
    );

    let outcome = match execution {
        Ok(mut result) => {
            if request.models.media {
                result.manifest_summary.selection_signature =
                    Some(media_selection_signature.clone());
                if result.manifest_summary.incremental_scan {
                    result.manifest_summary.full_scan_at = incremental_state
                        .as_ref()
                        .map(|state| state.full_scan_at.clone());
                } else if !result.rate_limited
                    && result.section_errors.is_empty()
                    && result.manifest_summary.incomplete_post_count == 0
                    && result
                        .completed_sections
                        .iter()
                        .any(|section| section == "media")
                {
                    result.manifest_summary.full_scan_at = Some(finished_at.clone());
                    result.manifest_summary.incremental_cutoff_timestamp = result
                        .manifest_summary
                        .newest_post_timestamp
                        .map(|timestamp| {
                            timestamp.saturating_sub(TWITTER_INCREMENTAL_OVERLAP_SECS)
                        });
                }
            }
            if result.rate_limited {
                for section in &result.completed_sections {
                    upsert_provider_sync_resume_state(
                        connection,
                        "twitter",
                        &context.source.id,
                        &context.account.id,
                        phase_scope,
                        section,
                        "completed",
                        None,
                        &finished_at,
                    )?;
                }
                for (section, cursor) in &result.resume_cursors {
                    upsert_provider_sync_resume_state(
                        connection,
                        "twitter",
                        &context.source.id,
                        &context.account.id,
                        phase_scope,
                        section,
                        "pending",
                        Some(cursor.as_str()),
                        &finished_at,
                    )?;
                }
            } else {
                clear_provider_sync_resume_scope(
                    connection,
                    "twitter",
                    &context.source.id,
                    &context.account.id,
                    phase_scope,
                )?;
            }
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
                &ProviderSyncMediaScope {
                    provider: "twitter",
                    source_id: &context.source.id,
                    account_id: &context.account.id,
                    source_handle: &handle,
                    profile_root: &profile_root,
                    timestamp: &finished_at,
                },
                &result.downloaded_media,
            )?;
            upsert_provider_sync_media_ledger_entries(
                connection,
                &ProviderSyncMediaScope {
                    provider: "twitter",
                    source_id: &context.source.id,
                    account_id: &context.account.id,
                    source_handle: &handle,
                    profile_root: &profile_root,
                    timestamp: &finished_at,
                },
                &result.deduplicated_media_aliases,
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
                                RuntimeLogAnchor {
                                    account_id: Some(&context.account.id),
                                    provider: Some(&context.source.provider),
                                    source_id: Some(&context.source.id),
                                    source_handle: Some(&context.source.handle),
                                },
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
                let resolved_avatar = provider_avatar.or_else(|| find_source_avatar(&profile_root));
                if let Some(avatar_path) = resolved_avatar {
                    let _ = update_source_profile_image(
                        connection,
                        layout,
                        &context.source.id,
                        &avatar_path,
                        &finished_at,
                    );
                }
            }

            let downloaded = result.downloaded_media.len();
            // Detalhamento técnico → realtime debugger (o resumo fica amigável).
            connector_debug::append_current(
                "internal.twitter",
                "summary",
                "manifest",
                format!(
                    "scan_mode={} cutoff={} parsed_pages={} queued_assets={} downloaded_assets={} skipped_existing_posts={} skipped_existing_assets={} skipped_duplicates={}",
                    if result.manifest_summary.incremental_scan {
                        "incremental"
                    } else {
                        "full"
                    },
                    result
                        .manifest_summary
                        .incremental_cutoff_timestamp
                        .map_or_else(|| "none".to_string(), |value| value.to_string()),
                    result.manifest_summary.parsed_page_count,
                    result.manifest_summary.queued_asset_count,
                    downloaded,
                    result.manifest_summary.skipped_existing_post_count,
                    result.manifest_summary.skipped_existing_asset_count,
                    result.manifest_summary.skipped_duplicate_asset_count,
                ),
            );
            let account_can_view_sensitive_media = parse_bool_setting(
                settings
                    .get("twitter.account.canViewSensitiveMedia")
                    .map(String::as_str),
                true,
            );
            if let Some(warning) = twitter_sensitive_media_account_warning(
                account_can_view_sensitive_media,
                result.manifest_summary.profile_sensitive_media,
            ) {
                result.section_errors.push(warning);
            }
            let completed_with_warnings = twitter_sync_completed_with_warnings(
                result.rate_limited || result.limit_aborted,
                &result.section_errors,
            );
            // "warning" = sync parcial útil (mídia/ledger ok) com aviso operacional
            // (ex.: conta sem acesso a NSFW). Não é failed; a fila conta como completed.
            let outcome_status = if completed_with_warnings {
                "warning".to_string()
            } else {
                "succeeded".to_string()
            };
            let mut summary = format_download_success_summary(
                if completed_with_warnings {
                    "Twitter sync completed with warnings."
                } else {
                    "Twitter sync succeeded."
                },
                downloaded,
            );
            if let Some(previous_handle) = redirected_from_handle.as_deref() {
                summary.push_str(&format!(
                    " Profile handle updated automatically: @{previous_handle} -> @{handle}; sync continued in the same run."
                ));
            }
            if let Some(breakdown) =
                format_twitter_download_breakdown(&result.manifest_summary.downloaded_by_section)
            {
                summary.push_str(&breakdown);
            }
            summary.push_str(&format_already_up_to_date_suffix(
                result.manifest_summary.skipped_existing_post_count,
            ));
            summary.push_str(&format_twitter_disabled_media_suffix(
                &result.manifest_summary.skipped_disabled_by_type,
            ));
            if result.manifest_summary.skipped_duplicate_asset_count > 0 {
                let count = result.manifest_summary.skipped_duplicate_asset_count;
                summary.push_str(&format!(
                    " {count} duplicate media {} reused from existing files.",
                    if count == 1 { "item was" } else { "items were" }
                ));
            }
            if result.manifest_summary.incomplete_post_count > 0 {
                summary.push_str(&format!(
                    " {} posts remain pending because enabled media could not be downloaded.",
                    result.manifest_summary.incomplete_post_count
                ));
            }
            if result.rate_limited {
                let hold_until = set_twitter_sync_hold(
                    connection,
                    &context.account.id,
                    Duration::seconds(TWITTER_RATE_LIMIT_HOLD_SECS),
                    &finished_at,
                )?;
                summary.push_str(&format!(
                    " Twitter rate limit reached; this Account is on hold until {}. The profile remains queued and will resume automatically.",
                    hold_until.to_rfc3339()
                ));
            } else {
                clear_twitter_sync_hold(connection, &context.account.id)?;
            }
            if !result.section_errors.is_empty() {
                summary.push_str(" Warnings: ");
                summary.push_str(&result.section_errors.join(" | "));
            }

            let manifest_summary_json = serde_json::to_string(&result.manifest_summary)
                .map_err(|error| error.to_string())?;
            SourceSyncOutcome {
                tool: "internal.twitter".to_string(),
                status: outcome_status,
                summary,
                command_preview,
                manifest_summary_json: Some(manifest_summary_json),
                degraded_capabilities: Vec::new(),
                validation_error: None,
            }
        }
        Err(error) => {
            let cancelled_by_user = error
                .trim()
                .to_ascii_lowercase()
                .contains("cancelled by user");
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
        Some(match outcome.status.as_str() {
            "succeeded" => "Download complete".to_string(),
            "warning" => "Download complete with warnings".to_string(),
            "skipped" => "Download skipped".to_string(),
            _ => "Download failed".to_string(),
        }),
        Some(outcome.summary.clone()),
        false,
        None,
    );
    Ok(outcome)
}

pub(super) fn load_provider_sync_resume_state(
    connection: &Connection,
    provider: &str,
    source_id: &str,
    account_id: &str,
    scope: &str,
) -> Result<(HashMap<String, String>, HashSet<String>), String> {
    let mut statement = connection
        .prepare(
            "SELECT section, state, cursor
             FROM provider_sync_resume_cursors
             WHERE provider = ?1 AND source_id = ?2 AND account_id = ?3 AND scope = ?4",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![provider, source_id, account_id, scope], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut cursors = HashMap::new();
    let mut completed = HashSet::new();
    for row in rows {
        let (section, state, cursor) = row.map_err(|error| error.to_string())?;
        if state == "completed" {
            completed.insert(section);
        } else if let Some(cursor) = cursor.filter(|value| !value.trim().is_empty()) {
            cursors.insert(section, cursor);
        }
    }
    Ok((cursors, completed))
}

pub(super) fn upsert_provider_sync_resume_state(
    connection: &Connection,
    provider: &str,
    source_id: &str,
    account_id: &str,
    scope: &str,
    section: &str,
    state: &str,
    cursor: Option<&str>,
    now: &str,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO provider_sync_resume_cursors
                (provider, source_id, account_id, scope, section, state, cursor, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(provider, source_id, account_id, scope, section)
             DO UPDATE SET state = excluded.state,
                           cursor = excluded.cursor,
                           updated_at = excluded.updated_at",
            params![provider, source_id, account_id, scope, section, state, cursor, now],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn clear_provider_sync_resume_scope(
    connection: &Connection,
    provider: &str,
    source_id: &str,
    account_id: &str,
    scope: &str,
) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM provider_sync_resume_cursors
             WHERE provider = ?1 AND source_id = ?2 AND account_id = ?3 AND scope = ?4",
            params![provider, source_id, account_id, scope],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn set_twitter_sync_hold(
    connection: &Connection,
    account_id: &str,
    retry_after: Duration,
    now: &str,
) -> Result<DateTime<Utc>, String> {
    let until = parse_rfc3339_utc(now).unwrap_or_else(Utc::now) + retry_after;
    connection
        .execute(
            "INSERT INTO provider_sync_account_holds
                (provider, account_id, hold_until, reason, updated_at)
             VALUES ('twitter', ?1, ?2, 'rate_limit', ?3)
             ON CONFLICT(provider, account_id)
             DO UPDATE SET hold_until = excluded.hold_until,
                           reason = excluded.reason,
                           updated_at = excluded.updated_at",
            params![account_id, until.to_rfc3339(), now],
        )
        .map_err(|error| error.to_string())?;
    Ok(until)
}

pub(super) fn clear_twitter_sync_hold(
    connection: &Connection,
    account_id: &str,
) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM provider_sync_account_holds
             WHERE provider = 'twitter' AND account_id = ?1",
            params![account_id],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Hold interno consultado pelo worker. Expiracoes sao limpas aqui para que
/// reiniciar o app nao deixe uma Account permanentemente bloqueada.
pub fn active_source_sync_account_hold_until(
    account_id: &str,
    provider: &str,
) -> Result<Option<DateTime<Utc>>, String> {
    if !provider.eq_ignore_ascii_case("twitter") {
        return Ok(None);
    }
    with_workspace(|connection, _| {
        let until = connection
            .query_row(
                "SELECT hold_until FROM provider_sync_account_holds
                 WHERE provider = 'twitter' AND account_id = ?1",
                params![account_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .as_deref()
            .and_then(parse_rfc3339_utc);
        if until.is_some_and(|value| value <= Utc::now()) {
            clear_twitter_sync_hold(connection, account_id)?;
            return Ok(None);
        }
        Ok(until)
    })
}

pub(super) fn twitter_sync_completed_with_warnings(
    limit_aborted: bool,
    section_errors: &[String],
) -> bool {
    limit_aborted || !section_errors.is_empty()
}

/// Syncs that finished productively enough to refresh `last_synced_at` and feed
/// incremental state: full success or success-with-warnings.
pub(super) fn source_sync_status_is_productive(status: &str) -> bool {
    matches!(status, "succeeded" | "warning")
}

/// Warning when the operator marked the Account as unable to view sensitive
/// media and the target profile is NSFW-flagged by X
/// (`profile_interstitial_type: sensitive_media`).
pub(super) fn twitter_sensitive_media_account_warning(
    account_can_view_sensitive_media: bool,
    profile_sensitive_media: bool,
) -> Option<String> {
    if account_can_view_sensitive_media || !profile_sensitive_media {
        return None;
    }
    Some(
        "This Account is marked as unable to view sensitive media (NSFW), and the profile is flagged as potentially sensitive on X. Media downloads may be incomplete. Enable sensitive media for this Account on X, open the profile and accept the warning, re-import cookies — or use an Account with sensitive media access and turn on “Can view sensitive media (NSFW)” in Account settings."
            .to_string(),
    )
}

pub(super) fn twitter_handle_redirect(
    current_handle: &str,
    resolved_handle: Option<&str>,
    rate_limited: bool,
) -> Option<String> {
    if rate_limited {
        return None;
    }
    let resolved = sanitize_source_handle("twitter", resolved_handle?)
        .trim_start_matches('@')
        .to_string();
    (!resolved.is_empty() && !current_handle.eq_ignore_ascii_case(&resolved)).then_some(resolved)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TwitterIncrementalState {
    pub(super) cutoff_timestamp: i64,
    pub(super) full_scan_at: String,
}

pub(super) fn twitter_media_selection_signature(options: &TwitterSourceSyncOptions) -> String {
    format!(
        "v1:images={};videos={};gifs={};non-user={}",
        options.download_images.unwrap_or(true),
        options.download_videos.unwrap_or(true),
        options.download_gifs.unwrap_or(true),
        options.allow_non_user_tweets.unwrap_or(false),
    )
}

pub(super) fn select_twitter_incremental_state(
    summaries: &[twitter_connector::TwitterManifestSummary],
    selection_signature: &str,
    now: DateTime<Utc>,
) -> Option<TwitterIncrementalState> {
    summaries.iter().find_map(|summary| {
        if summary.rate_limited
            || summary.incomplete_post_count > 0
            || summary.attempted_model_count == 0
            || summary.completed_model_count != summary.attempted_model_count
            || summary.selection_signature.as_deref() != Some(selection_signature)
        {
            return None;
        }
        let full_scan_at = summary.full_scan_at.as_deref()?;
        let parsed = DateTime::parse_from_rfc3339(full_scan_at)
            .ok()?
            .with_timezone(&Utc);
        let age = now.signed_duration_since(parsed).num_seconds();
        if !(0..TWITTER_PERIODIC_FULL_SCAN_SECS).contains(&age) {
            return None;
        }
        Some(TwitterIncrementalState {
            cutoff_timestamp: summary.incremental_cutoff_timestamp?,
            full_scan_at: full_scan_at.to_string(),
        })
    })
}

fn load_twitter_incremental_state(
    connection: &Connection,
    source_id: &str,
    selection_signature: &str,
) -> Result<Option<TwitterIncrementalState>, String> {
    let mut statement = connection
        .prepare(
            "SELECT manifest_summary_json
             FROM source_sync_runs
             WHERE source_id = ?1 AND provider = 'twitter'
               AND status IN ('succeeded', 'warning')
               AND manifest_summary_json IS NOT NULL
             ORDER BY finished_at DESC
             LIMIT 20",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([source_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    let mut summaries = Vec::new();
    for row in rows {
        if let Ok(summary) = serde_json::from_str::<twitter_connector::TwitterManifestSummary>(
            &row.map_err(|error| error.to_string())?,
        ) {
            summaries.push(summary);
        }
    }
    Ok(select_twitter_incremental_state(
        &summaries,
        selection_signature,
        Utc::now(),
    ))
}

pub(super) fn format_twitter_download_breakdown(
    downloaded_by_section: &std::collections::BTreeMap<String, u32>,
) -> Option<String> {
    if downloaded_by_section.len() < 2 {
        return None;
    }
    let breakdown = downloaded_by_section
        .iter()
        .map(|(section, count)| {
            let label = match section.as_str() {
                "media" => "profile posts",
                "timeline" => "full timeline backfill",
                "search" => "search results",
                "likes" => "liked posts",
                _ => section.as_str(),
            };
            format!("{count} from {label}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(" Breakdown: {breakdown}."))
}

pub(super) fn format_twitter_disabled_media_suffix(
    skipped_by_type: &std::collections::BTreeMap<String, u32>,
) -> String {
    let entries = skipped_by_type
        .iter()
        .filter(|(_, count)| **count > 0)
        .map(|(media_type, count)| (media_type.as_str(), *count))
        .collect::<Vec<_>>();
    let format_part = |media_type: &str, count: u32| {
        let label = match (media_type, count) {
            ("image", 1) => "image",
            ("image", _) => "images",
            ("video", 1) => "video",
            ("video", _) => "videos",
            ("gif", 1) => "GIF",
            ("gif", _) => "GIFs",
            _ => media_type,
        };
        format!("{count} {label}")
    };
    match entries.as_slice() {
        [] => String::new(),
        [(media_type, count)] => {
            let item = format_part(media_type, *count);
            let setting = match *media_type {
                "image" => "image",
                "video" => "video",
                "gif" => "GIF",
                other => other,
            };
            format!(
                " {item} {} skipped because {setting} downloads are disabled.",
                if *count == 1 { "was" } else { "were" }
            )
        }
        _ => {
            let parts = entries
                .iter()
                .map(|(media_type, count)| format_part(media_type, *count))
                .collect::<Vec<_>>();
            format!(
                " {} were skipped because these media types are disabled.",
                parts.join(", ")
            )
        }
    }
}
/// Sync interno do TikTok: yt-dlp baixa os vídeos da timeline e o gallery-dl
/// parseia os posts de fotos (slideshow), persistindo nos ledgers
/// provider-neutral. Espelha o branch do Twitter.
/// Persists the resolved YouTube `channel_id` into the source's sync options,
/// without overwriting an already-known hint.
pub(super) fn persist_youtube_user_id_hint(
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
            youtube: Some(normalize_youtube_source_sync_options(None)),
            ..Default::default()
        });
    let youtube = options
        .youtube
        .get_or_insert_with(|| normalize_youtube_source_sync_options(None));
    if youtube
        .user_id_hint
        .as_deref()
        .map(str::trim)
        .map(|value| !value.is_empty())
        .unwrap_or(false)
    {
        return Ok(());
    }
    youtube.user_id_hint = Some(user_id.to_string());
    let serialized = serialize_source_sync_options("youtube", &options)?;
    connection
        .execute(
            "UPDATE source_profiles SET sync_options_json = ?2, updated_at = ?3
             WHERE id = ?1 AND deleted_at IS NULL",
            params![source_id, serialized, timestamp],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Updates the YouTube post ledger with fresh view counts.
pub(super) fn upsert_youtube_post_stats(
    connection: &Connection,
    source_id: &str,
    observed_posts: &[youtube_connector::ObservedYouTubePost],
    timestamp: &str,
) -> Result<(), String> {
    for post in observed_posts {
        if post.view_count.is_none() {
            continue;
        }
        connection
            .execute(
                "UPDATE provider_sync_post_ledger
                 SET view_count = ?1, stats_updated_at = ?2
                 WHERE provider = 'youtube' AND source_id = ?3
                   AND provider_post_key = ?4",
                params![
                    post.view_count,
                    timestamp,
                    source_id,
                    &post.provider_post_key,
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Writes the captured `title`/`duration_seconds` onto the freshly upserted
/// media ledger rows (the shared media ledger upsert does not carry them).
pub(super) fn upsert_youtube_media_metadata(
    connection: &Connection,
    source_id: &str,
    downloaded_media: &[youtube_connector::DownloadedYouTubeMedia],
    timestamp: &str,
) -> Result<(), String> {
    for media in downloaded_media {
        if media.title.is_none() && media.duration_seconds.is_none() {
            continue;
        }
        connection
            .execute(
                "UPDATE provider_sync_media_ledger
                 SET title = COALESCE(?1, title),
                     duration_seconds = COALESCE(?2, duration_seconds),
                     last_seen_at = ?3
                 WHERE provider = 'youtube' AND source_id = ?4
                   AND provider_media_key = ?5",
                params![
                    media.title,
                    media.duration_seconds,
                    timestamp,
                    source_id,
                    media.provider_media_key.to_ascii_lowercase(),
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(super) fn execute_youtube_source_sync_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    context: &SourceSyncContext,
    settings: &HashMap<String, String>,
    trigger: &str,
    _run_mode: Option<&str>,
    sync_options_override: Option<&SourceSyncOptions>,
) -> Result<SourceSyncOutcome, String> {
    let options = source_youtube_sync_options_with_override(&context.source, sync_options_override);
    let started_at = now_timestamp();

    let videos_enabled = options.get_videos.unwrap_or(true);
    let shorts_enabled = options.get_shorts.unwrap_or(false);
    if !videos_enabled && !shorts_enabled {
        return Err(
            "No YouTube sync section is enabled. Select Videos or Shorts.".to_string(),
        );
    }
    let collect_media_stats = options.collect_media_stats.unwrap_or(true);

    let handle = sanitize_source_handle("youtube", &context.source.handle)
        .trim_start_matches('@')
        .to_string();
    if handle.is_empty() {
        return Err("YouTube source handle is empty.".to_string());
    }

    let profile_root =
        resolved_source_media_output_root_with_connection(connection, layout, &context.source)?;
    fs::create_dir_all(&profile_root).map_err(|error| error.to_string())?;
    cleanup_empty_media_artifacts(&profile_root)?;
    let cache_root = layout
        .cache_root
        .join(format!("youtube-sync-{}", context.source.id));
    fs::create_dir_all(&cache_root).map_err(|error| error.to_string())?;

    // YouTube usually works without authentication; write a cookie file only if
    // the session carries cookies (age-restricted / members content).
    let cookie_file = cache_root.join("cookies.txt");
    let parsed_session = parse_session_payload(&context.session_payload);
    let user_agent = match &parsed_session {
        Ok(session) => {
            if !session.cookies.is_empty() {
                write_netscape_cookie_file(&cookie_file, &session.cookies)?;
            }
            session.metadata.user_agent.clone()
        }
        Err(_) => None,
    };
    let user_agent = settings
        .get("youtube.auth.userAgent")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or(user_agent);

    let yt_dlp_executable =
        connector_runtime::resolve_connector_executable(connection, layout, "yt-dlp")?;

    let ledger_post_keys =
        load_provider_sync_post_ledger_keys(connection, "youtube", &context.source.id)?;
    let ledger_media_keys =
        load_provider_sync_media_ledger_keys(connection, "youtube", &context.source.id)?;
    let existing_relative_paths = load_existing_relative_media_paths(&profile_root);

    let request = youtube_connector::YouTubeConnectorRequest {
        handle: handle.clone(),
        yt_dlp_executable: PathBuf::from(&yt_dlp_executable),
        cookie_file,
        user_agent,
        profile_root: profile_root.clone(),
        cache_root,
        sections: youtube_connector::YouTubeSectionSelection {
            videos: videos_enabled,
            shorts: shorts_enabled,
        },
        download_videos: options.download_videos.unwrap_or(true),
        separate_video_folder: options.separate_video_folder.unwrap_or(false),
        use_parsed_video_date: options.use_parsed_video_date.unwrap_or(true),
        sleep_timer_secs: options.sleep_timer_secs.unwrap_or(-1),
        abort_on_limit: options.abort_on_limit.unwrap_or(true),
        collect_media_stats,
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
        Some("YouTube connector is preparing source sync.".to_string()),
        true,
        Some(0),
    );

    let is_first_sync = context.source.last_synced_at.is_none();
    let dup_source_id = context.source.id.clone();
    let execution = youtube_connector::run_profile_sync(
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
                && find_source_with_same_user_id(connection, "youtube", user_id, &dup_source_id)
                    .ok()
                    .flatten()
                    .is_some()
        },
    );
    clear_source_sync_cancel_token(&context.source.id);
    let finished_at = now_timestamp();

    let command_preview = format!(
        "internal.youtube channel {} -> {}",
        handle,
        profile_root.display()
    );

    let outcome = match execution {
        Ok(result) => {
            if let Some(user_id) = result.duplicate_user_id.as_deref() {
                if let Some(dup_outcome) = detect_duplicate_user_id_on_first_sync(
                    connection,
                    layout,
                    context,
                    user_id,
                    "internal.youtube",
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

            if result.profile_unavailable {
                let problem_code = "youtube_profile_unavailable";
                let problem_message = format!(
                    "YouTube channel '@{}' (source id {}) is unavailable. The channel may have been renamed, deleted, terminated, or made private.",
                    handle, context.source.id
                );
                let mark_error = set_source_sync_problem(
                    connection,
                    &context.source.id,
                    problem_code,
                    &problem_message,
                    &finished_at,
                    true,
                );
                let mut summary = format!("YouTube sync blocked: {problem_message}");
                if let Err(mark_failure) = mark_error {
                    summary.push_str(&format!(
                        " Failed to persist source problem marker: {mark_failure}."
                    ));
                } else {
                    log_runtime_event(
                        layout,
                        "sync.profile",
                        "warning",
                        RuntimeLogAnchor {
                            account_id: Some(&context.account.id),
                            provider: Some(&context.source.provider),
                            source_id: Some(&context.source.id),
                            source_handle: Some(&context.source.handle),
                        },
                        format!(
                            "Marked source '{}' as '{}': {}",
                            context.source.handle, problem_code, problem_message
                        ),
                        None,
                    );
                }
                let outcome = SourceSyncOutcome {
                    tool: "internal.youtube".to_string(),
                    status: "failed".to_string(),
                    summary: summary.clone(),
                    command_preview: command_preview.clone(),
                    manifest_summary_json: None,
                    degraded_capabilities: Vec::new(),
                    validation_error: Some(summary),
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
                    Some("Channel unavailable".to_string()),
                    Some(outcome.summary.clone()),
                    false,
                    None,
                );
                return Ok(outcome);
            }

            // The ledgers are provider-neutral; reuse the Twitter connector
            // structs (same fields) just for the base insertion.
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
                "youtube",
                &context.source.id,
                &context.account.id,
                &handle,
                &observed_posts,
                &finished_at,
            )?;
            if collect_media_stats {
                upsert_youtube_post_stats(
                    connection,
                    &context.source.id,
                    &result.observed_posts,
                    &finished_at,
                )?;
            }
            upsert_provider_sync_media_ledger_entries(
                connection,
                &ProviderSyncMediaScope {
                    provider: "youtube",
                    source_id: &context.source.id,
                    account_id: &context.account.id,
                    source_handle: &handle,
                    profile_root: &profile_root,
                    timestamp: &finished_at,
                },
                &downloaded_media,
            )?;
            upsert_youtube_media_metadata(
                connection,
                &context.source.id,
                &result.downloaded_media,
                &finished_at,
            )?;

            if let Some(user_id) = result.resolved_user_id.as_deref() {
                let _ = persist_youtube_user_id_hint(
                    connection,
                    &context.source.id,
                    user_id,
                    &finished_at,
                );
            }

            if !context.source.profile_image_custom {
                if let Some(avatar_path) = find_source_avatar(&profile_root) {
                    let _ = update_source_profile_image(
                        connection,
                        layout,
                        &context.source.id,
                        &avatar_path,
                        &finished_at,
                    );
                }
            }

            let downloaded = result.downloaded_media.len();
            let stats_updated = result
                .observed_posts
                .iter()
                .filter(|post| post.view_count.is_some())
                .count();
            connector_debug::append_current(
                "internal.youtube",
                "summary",
                "manifest",
                format!(
                    "scanned_posts={} discovered_assets={} queued_assets={} downloaded_assets={} skipped_existing_posts={} skipped_existing_assets={} stats_updated={}",
                    result.manifest_summary.normalized_post_count,
                    result.manifest_summary.discovered_asset_count,
                    result.manifest_summary.queued_asset_count,
                    downloaded,
                    result.manifest_summary.skipped_existing_post_count,
                    result.manifest_summary.skipped_existing_asset_count,
                    stats_updated,
                ),
            );
            let mut summary =
                format_download_success_summary("YouTube sync succeeded.", downloaded);
            summary.push_str(&format_already_up_to_date_suffix(
                result.manifest_summary.skipped_existing_post_count,
            ));
            if collect_media_stats && stats_updated > 0 {
                summary.push_str(&format!(" Stats updated for {stats_updated} video(s)."));
            }
            if result.limit_aborted {
                summary.push_str(" Rate limit reached; remaining videos were skipped.");
            }
            if !result.section_errors.is_empty() {
                summary.push_str(" Warnings: ");
                summary.push_str(&result.section_errors.join(" | "));
            }

            SourceSyncOutcome {
                tool: "internal.youtube".to_string(),
                status: "succeeded".to_string(),
                summary,
                command_preview,
                manifest_summary_json: None,
                degraded_capabilities: Vec::new(),
                validation_error: None,
            }
        }
        Err(error) => {
            let cancelled_by_user = error
                .trim()
                .to_ascii_lowercase()
                .contains("cancelled by user");
            SourceSyncOutcome {
                tool: "internal.youtube".to_string(),
                status: if cancelled_by_user {
                    "skipped".to_string()
                } else {
                    "failed".to_string()
                },
                summary: if cancelled_by_user {
                    "YouTube sync cancelled by user.".to_string()
                } else {
                    format!("YouTube sync failed: {}", error)
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
    if outcome.status == "succeeded" {
        if let Err(error) = clear_source_sync_problem(connection, &context.source.id, &finished_at)
        {
            log_runtime_event(
                layout,
                "sync.profile",
                "warning",
                RuntimeLogAnchor {
                    account_id: Some(&context.account.id),
                    provider: Some(&context.source.provider),
                    source_id: Some(&context.source.id),
                    source_handle: Some(&context.source.handle),
                },
                format!(
                    "YouTube sync succeeded, but failed to clear source sync problem marker: {error}"
                ),
                Some(error),
            );
        }
    }
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

pub(super) fn persist_vsco_user_id_hint(
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
            vsco: Some(normalize_vsco_source_sync_options(None)),
            ..Default::default()
        });
    let vsco = options
        .vsco
        .get_or_insert_with(|| normalize_vsco_source_sync_options(None));
    if vsco
        .user_id_hint
        .as_deref()
        .map(str::trim)
        .map(|value| !value.is_empty())
        .unwrap_or(false)
    {
        return Ok(());
    }
    vsco.user_id_hint = Some(user_id.to_string());
    let serialized = serialize_source_sync_options("vsco", &options)?;
    connection
        .execute(
            "UPDATE source_profiles SET sync_options_json = ?2, updated_at = ?3
             WHERE id = ?1 AND deleted_at IS NULL",
            params![source_id, serialized, timestamp],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Executes the internal VSCO sync: gallery-dl parses the gallery/journal pages
/// and NinjaCrawler downloads and catalogs the media, persisting posts/media in
/// the provider-neutral ledgers. VSCO re-enumerates the whole gallery on every
/// sync (no resume cursor), relying on the ledgers/disk to skip existing media.
pub(super) fn execute_vsco_source_sync_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    context: &SourceSyncContext,
    settings: &HashMap<String, String>,
    trigger: &str,
    _run_mode: Option<&str>,
    sync_options_override: Option<&SourceSyncOptions>,
) -> Result<SourceSyncOutcome, String> {
    let options = source_vsco_sync_options_with_override(&context.source, sync_options_override);
    let started_at = now_timestamp();

    let gallery_enabled = options.get_gallery.unwrap_or(true);
    let journal_enabled = options.get_journal.unwrap_or(false);
    if !gallery_enabled && !journal_enabled {
        return Err("No VSCO sync section is enabled. Select Gallery or Journal.".to_string());
    }

    let handle = sanitize_source_handle("vsco", &context.source.handle)
        .trim_start_matches('@')
        .to_string();
    if handle.is_empty() {
        return Err("VSCO source handle is empty.".to_string());
    }

    let profile_root =
        resolved_source_media_output_root_with_connection(connection, layout, &context.source)?;
    fs::create_dir_all(&profile_root).map_err(|error| error.to_string())?;
    cleanup_empty_media_artifacts(&profile_root)?;
    let cache_root = layout
        .cache_root
        .join(format!("vsco-sync-{}", context.source.id));
    fs::create_dir_all(&cache_root).map_err(|error| error.to_string())?;

    // VSCO usually works without authentication; write a cookie file only if the
    // session carries cookies (private profiles).
    let cookie_file = cache_root.join("cookies.txt");
    let parsed_session = parse_session_payload(&context.session_payload);
    let user_agent = match &parsed_session {
        Ok(session) => {
            if !session.cookies.is_empty() {
                write_netscape_cookie_file(&cookie_file, &session.cookies)?;
            }
            session.metadata.user_agent.clone()
        }
        Err(_) => None,
    };
    let user_agent = settings
        .get("vsco.auth.userAgent")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or(user_agent);

    let executable =
        connector_runtime::resolve_connector_executable(connection, layout, "gallery-dl")?;

    let ledger_post_keys =
        load_provider_sync_post_ledger_keys(connection, "vsco", &context.source.id)?;
    let ledger_media_keys =
        load_provider_sync_media_ledger_keys(connection, "vsco", &context.source.id)?;
    let existing_relative_paths = load_existing_relative_media_paths(&profile_root)
        .into_iter()
        .filter_map(|file_name| vsco_connector::vsco_disk_asset_key(&file_name))
        .collect();

    let request = vsco_connector::VscoConnectorRequest {
        handle: handle.clone(),
        gallery_dl_executable: PathBuf::from(&executable),
        cookie_file,
        user_agent,
        profile_root: profile_root.clone(),
        cache_root,
        sections: vsco_connector::VscoSectionSelection {
            gallery: gallery_enabled,
            journal: journal_enabled,
        },
        download_images: options.download_images.unwrap_or(true),
        download_videos: options.download_videos.unwrap_or(true),
        separate_video_folder: options.separate_video_folder.unwrap_or(true),
        use_md5_comparison: options.use_md5_comparison.unwrap_or(false),
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
        Some("VSCO connector is preparing source sync.".to_string()),
        true,
        Some(0),
    );

    let is_first_sync = context.source.last_synced_at.is_none();
    let dup_source_id = context.source.id.clone();
    let execution = vsco_connector::run_profile_sync(
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
                && find_source_with_same_user_id(connection, "vsco", user_id, &dup_source_id)
                    .ok()
                    .flatten()
                    .is_some()
        },
    );
    clear_source_sync_cancel_token(&context.source.id);
    let finished_at = now_timestamp();

    let command_preview = format!(
        "internal.vsco gallery={} journal={} profile {} -> {}",
        gallery_enabled,
        journal_enabled,
        handle,
        profile_root.display()
    );

    let outcome = match execution {
        Ok(result) => {
            if let Some(user_id) = result.duplicate_user_id.as_deref() {
                if let Some(dup_outcome) = detect_duplicate_user_id_on_first_sync(
                    connection,
                    layout,
                    context,
                    user_id,
                    "internal.vsco",
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

            if result.profile_unavailable {
                let problem_code = "vsco_profile_unavailable";
                let problem_message = format!(
                    "VSCO profile '{}' (source id {}) is unavailable. The profile may have been renamed, deleted, or made private.",
                    handle, context.source.id
                );
                let mark_error = set_source_sync_problem(
                    connection,
                    &context.source.id,
                    problem_code,
                    &problem_message,
                    &finished_at,
                    true,
                );
                let mut summary = format!("VSCO sync blocked: {problem_message}");
                if let Err(mark_failure) = mark_error {
                    summary.push_str(&format!(
                        " Failed to persist source problem marker: {mark_failure}."
                    ));
                } else {
                    log_runtime_event(
                        layout,
                        "sync.profile",
                        "warning",
                        RuntimeLogAnchor {
                            account_id: Some(&context.account.id),
                            provider: Some(&context.source.provider),
                            source_id: Some(&context.source.id),
                            source_handle: Some(&context.source.handle),
                        },
                        format!(
                            "Marked source '{}' as '{}': {}",
                            context.source.handle, problem_code, problem_message
                        ),
                        None,
                    );
                }
                let outcome = SourceSyncOutcome {
                    tool: "internal.vsco".to_string(),
                    status: "failed".to_string(),
                    summary: summary.clone(),
                    command_preview: command_preview.clone(),
                    manifest_summary_json: None,
                    degraded_capabilities: Vec::new(),
                    validation_error: Some(summary),
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
                    Some("Profile unavailable".to_string()),
                    Some(outcome.summary.clone()),
                    false,
                    None,
                );
                return Ok(outcome);
            }

            // The ledgers are provider-neutral; reuse the Twitter connector
            // structs (same fields) just for the base insertion.
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
                .chain(result.deduplicated_media_aliases.iter())
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
                "vsco",
                &context.source.id,
                &context.account.id,
                &handle,
                &observed_posts,
                &finished_at,
            )?;
            upsert_provider_sync_media_ledger_entries(
                connection,
                &ProviderSyncMediaScope {
                    provider: "vsco",
                    source_id: &context.source.id,
                    account_id: &context.account.id,
                    source_handle: &handle,
                    profile_root: &profile_root,
                    timestamp: &finished_at,
                },
                &downloaded_media,
            )?;

            if let Some(user_id) = result.resolved_user_id.as_deref() {
                let _ =
                    persist_vsco_user_id_hint(connection, &context.source.id, user_id, &finished_at);
            }

            if !context.source.profile_image_custom {
                if let Some(avatar_path) = find_source_avatar(&profile_root) {
                    let _ = update_source_profile_image(
                        connection,
                        layout,
                        &context.source.id,
                        &avatar_path,
                        &finished_at,
                    );
                }
            }

            let downloaded = result.downloaded_media.len();
            connector_debug::append_current(
                "internal.vsco",
                "summary",
                "manifest",
                format!(
                    "parsed_pages={} normalized_posts={} discovered_assets={} queued_assets={} downloaded_assets={} skipped_existing_posts={} skipped_existing_assets={} skipped_duplicates={}",
                    result.manifest_summary.parsed_page_count,
                    result.manifest_summary.normalized_post_count,
                    result.manifest_summary.discovered_asset_count,
                    result.manifest_summary.queued_asset_count,
                    downloaded,
                    result.manifest_summary.skipped_existing_post_count,
                    result.manifest_summary.skipped_existing_asset_count,
                    result.manifest_summary.skipped_duplicate_asset_count,
                ),
            );
            let mut summary = format_download_success_summary("VSCO sync succeeded.", downloaded);
            summary.push_str(&format_already_up_to_date_suffix(
                result.manifest_summary.skipped_existing_post_count,
            ));
            if result.manifest_summary.skipped_duplicate_asset_count > 0 {
                let count = result.manifest_summary.skipped_duplicate_asset_count;
                summary.push_str(&format!(
                    " {count} duplicate media {} reused from existing files.",
                    if count == 1 { "item was" } else { "items were" }
                ));
            }
            if !result.section_errors.is_empty() {
                summary.push_str(" Warnings: ");
                summary.push_str(&result.section_errors.join(" | "));
            }

            SourceSyncOutcome {
                tool: "internal.vsco".to_string(),
                status: "succeeded".to_string(),
                summary,
                command_preview,
                manifest_summary_json: None,
                degraded_capabilities: Vec::new(),
                validation_error: None,
            }
        }
        Err(error) => {
            let cancelled_by_user = error
                .trim()
                .to_ascii_lowercase()
                .contains("cancelled by user");
            SourceSyncOutcome {
                tool: "internal.vsco".to_string(),
                status: if cancelled_by_user {
                    "skipped".to_string()
                } else {
                    "failed".to_string()
                },
                summary: if cancelled_by_user {
                    "VSCO sync cancelled by user.".to_string()
                } else {
                    format!("VSCO sync failed: {}", error)
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
    if outcome.status == "succeeded" {
        if let Err(error) = clear_source_sync_problem(connection, &context.source.id, &finished_at) {
            log_runtime_event(
                layout,
                "sync.profile",
                "warning",
                RuntimeLogAnchor {
                    account_id: Some(&context.account.id),
                    provider: Some(&context.source.provider),
                    source_id: Some(&context.source.id),
                    source_handle: Some(&context.source.handle),
                },
                format!(
                    "VSCO sync succeeded, but failed to clear source sync problem marker: {error}"
                ),
                Some(error),
            );
        }
    }
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

pub(super) fn execute_tiktok_source_sync_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    context: &SourceSyncContext,
    settings: &HashMap<String, String>,
    trigger: &str,
    run_mode: Option<&str>,
    sync_options_override: Option<&SourceSyncOptions>,
) -> Result<SourceSyncOutcome, String> {
    let options = source_tiktok_sync_options_with_override(&context.source, sync_options_override);
    let started_at = now_timestamp();
    // Ação pontual "Refresh media stats": um sync normal que também re-coleta
    // as estatísticas da mídia já baixada, sem alterar as opções persistidas.
    let stats_refresh_run = run_mode
        .is_some_and(|value| value.eq_ignore_ascii_case(TIKTOK_REFRESH_MEDIA_STATS_RUN_MODE));
    let timeline_enabled = options.get_timeline.unwrap_or(true);
    let stories_enabled = options.get_stories_user.unwrap_or(false);
    let reposts_enabled = options.get_reposts.unwrap_or(false);
    let liked_videos_enabled = options.get_liked_videos.unwrap_or(false);
    let liked_videos_limit = options.liked_videos_limit.unwrap_or(100);
    let liked_videos_incremental = options.liked_videos_incremental.unwrap_or(true);
    let liked_videos_known_page_threshold = options.liked_videos_known_page_threshold.unwrap_or(3);
    let collect_media_stats = stats_refresh_run || options.collect_media_stats.unwrap_or(true);
    let refresh_existing_media_stats = collect_media_stats
        && (stats_refresh_run || options.refresh_existing_media_stats.unwrap_or(false));
    if liked_videos_limit < 0 {
        return Err("TikTok liked videos limit cannot be negative.".to_string());
    }
    if liked_videos_known_page_threshold < 1 {
        return Err("TikTok liked videos known-page threshold must be at least 1.".to_string());
    }
    let target_video_url = options
        .target_video_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let profile_sections_enabled =
        timeline_enabled || stories_enabled || reposts_enabled || target_video_url.is_some();
    if !profile_sections_enabled && !liked_videos_enabled {
        return Err(
            "No TikTok sync section is enabled. Select Timeline, User Stories, Reposts, or Liked videos."
                .to_string(),
        );
    }

    let handle = sanitize_source_handle("tiktok", &context.source.handle)
        .trim_start_matches('@')
        .to_string();
    if handle.is_empty() {
        return Err("TikTok source handle is empty.".to_string());
    }

    let profile_root =
        resolved_source_media_output_root_with_connection(connection, layout, &context.source)?;
    fs::create_dir_all(&profile_root).map_err(|error| error.to_string())?;
    cleanup_empty_media_artifacts(&profile_root)?;
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
    let gallery_dl_executable = if stories_enabled || reposts_enabled {
        connector_runtime::resolve_connector_executable(connection, layout, "gallery-dl")?
    } else {
        String::new()
    };

    let reconcile = reconcile_tiktok_provider_ledgers(
        connection,
        &profile_root,
        &context.account.id,
        &context.source.id,
        &handle,
        &started_at,
    )?;
    if reconcile.recovered_from_disk > 0
        || reconcile.purged_media_rows > 0
        || reconcile.purged_post_rows > 0
    {
        log_runtime_event(
            layout,
            "sync.ledger",
            "info",
            RuntimeLogAnchor {
                account_id: Some(&context.account.id),
                provider: Some(&context.source.provider),
                source_id: Some(&context.source.id),
                source_handle: Some(&context.source.handle),
            },
            format!(
                "TikTok ledger reconcile: recovered {} from disk, purged {} missing media row(s) and {} stale post key(s).",
                reconcile.recovered_from_disk,
                reconcile.purged_media_rows,
                reconcile.purged_post_rows
            ),
            None,
        );
    }

    let mut ledger_post_keys =
        load_provider_sync_post_ledger_keys(connection, "tiktok", &context.source.id)?;
    ledger_post_keys.extend(
        load_provider_deleted_post_keys(connection, "tiktok", &context.source.id)
            .unwrap_or_default(),
    );
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
        sections: tiktok_section_selection(&options),
        target_video_url,
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
        collect_media_stats,
        refresh_existing_media_stats,
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

    let likes_execution = if liked_videos_enabled {
        match source_sync_runtime::registered_app_handle() {
            Ok(app) => tiktok_likes_runtime::run_source_sync(
                &app,
                tiktok_likes_runtime::TikTokLikesSourceRequest {
                    account_id: context.account.id.clone(),
                    source_id: context.source.id.clone(),
                    source_handle: handle.clone(),
                    profile_root: profile_root.clone(),
                    item_limit: liked_videos_limit as usize,
                    incremental: liked_videos_incremental,
                    known_page_threshold: liked_videos_known_page_threshold as usize,
                    collect_media_stats,
                    refresh_existing_media_stats,
                },
                |percent, label, detail, indeterminate, downloaded| {
                    let queue_percent = percent.map(|value| {
                        if profile_sections_enabled {
                            value / 2
                        } else {
                            value
                        }
                    });
                    source_sync_runtime::report_source_sync_progress(
                        &context.source.id,
                        queue_percent,
                        Some(label),
                        Some(detail),
                        indeterminate,
                        downloaded,
                    );
                },
                || cancel_token.load(Ordering::SeqCst),
            ),
            Err(error) => Err(error),
        }
    } else {
        Ok(tiktok_likes_runtime::TikTokLikesSyncResult::default())
    };
    let is_first_sync = context.source.last_synced_at.is_none();
    let dup_source_id = context.source.id.clone();
    let combined_execution = match likes_execution {
        Err(error) => Err(error),
        Ok(likes) => {
            let profile_execution = if profile_sections_enabled {
                tiktok_connector::run_profile_sync(
                    &request,
                    |progress| {
                        let queue_percent = progress.progress_percent.map(|value| {
                            if liked_videos_enabled {
                                50 + (value / 2)
                            } else {
                                value
                            }
                        });
                        let downloaded_items = progress
                            .downloaded_items
                            .map(|value| value + likes.downloaded as u32);
                        source_sync_runtime::report_source_sync_progress(
                            &context.source.id,
                            queue_percent,
                            Some(progress.label),
                            Some(progress.detail),
                            progress.indeterminate,
                            downloaded_items,
                        );
                    },
                    || cancel_token.load(Ordering::SeqCst),
                    |user_id| {
                        is_first_sync
                            && find_source_with_same_user_id(
                                connection,
                                "tiktok",
                                user_id,
                                &dup_source_id,
                            )
                            .ok()
                            .flatten()
                            .is_some()
                    },
                )
            } else {
                Ok(tiktok_connector::TikTokConnectorResult {
                    observed_posts: Vec::new(),
                    downloaded_media: Vec::new(),
                    section_errors: Vec::new(),
                    rate_limited: false,
                    limit_aborted: false,
                    resolved_user_id: None,
                    resolved_avatar_url: None,
                    duplicate_user_id: None,
                    resolved_handle: None,
                    profile_unavailable: false,
                    profile_private: false,
                    manifest_summary: tiktok_connector::TikTokManifestSummary::default(),
                })
            };
            profile_execution.map(|result| (result, likes))
        }
    };
    clear_source_sync_cancel_token(&context.source.id);
    let finished_at = now_timestamp();

    let command_preview = format!(
        "internal.tiktok profile {} -> {}",
        handle,
        profile_root.display()
    );

    let outcome = match combined_execution {
        Ok((result, likes)) => {
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
        RuntimeLogAnchor {
            account_id: Some(&context.account.id),
            provider: Some(&context.source.provider),
            source_id: Some(&context.source.id),
            source_handle: Some(&context.source.handle),
        },
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
        RuntimeLogAnchor {
            account_id: Some(&context.account.id),
            provider: Some(&context.source.provider),
            source_id: Some(&context.source.id),
            source_handle: Some(&context.source.handle),
        },
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
                    propagate_source_sync_account_health(
                        connection,
                        context,
                        &outcome,
                        &finished_at,
                    )?;
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

            // Perfil indisponível: o yt-dlp não resolveu o dono do perfil e não
            // houve renomeação a recuperar. Marca a fonte e reporta "bloqueado"
            // em vez de um sync bem-sucedido com zero posts (paridade com o
            // Instagram, que sinaliza `instagram_username_unresolvable`).
            if result.profile_unavailable {
                let problem_code = "tiktok_profile_unavailable";
                let problem_message = format!(
                    "TikTok profile '@{}' (source id {}) is unavailable. The account may have been renamed, made private, deactivated, or banned.",
                    handle, context.source.id
                );
                let mark_error = set_source_sync_problem(
                    connection,
                    &context.source.id,
                    problem_code,
                    &problem_message,
                    &finished_at,
                    true,
                );
                let mut summary = format!("TikTok sync blocked: {problem_message}");
                if let Err(mark_failure) = mark_error {
                    summary.push_str(&format!(
                        " Failed to persist source problem marker: {mark_failure}."
                    ));
                } else {
                    log_runtime_event(
                        layout,
                        "sync.profile",
                        "warning",
                        RuntimeLogAnchor {
                            account_id: Some(&context.account.id),
                            provider: Some(&context.source.provider),
                            source_id: Some(&context.source.id),
                            source_handle: Some(&context.source.handle),
                        },
                        format!(
                            "Marked source '{}' as '{}': {}",
                            context.source.handle, problem_code, problem_message
                        ),
                        None,
                    );
                }
                let outcome = SourceSyncOutcome {
                    tool: "internal.tiktok".to_string(),
                    status: "failed".to_string(),
                    summary: summary.clone(),
                    command_preview: command_preview.clone(),
                    manifest_summary_json: None,
                    degraded_capabilities: Vec::new(),
                    validation_error: Some(summary),
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
                    Some("Profile unavailable".to_string()),
                    Some(outcome.summary.clone()),
                    false,
                    None,
                );
                return Ok(outcome);
            }

            // Perfil privado não seguido: existe, mas não há mídia acessível.
            // Marca "perfil privado" e desliga `ready_for_download` (não há o que
            // baixar enquanto a conta não seguir o perfil); reporta "skipped".
            if result.profile_private {
                let problem_code = "tiktok_profile_private_or_restricted";
                let problem_message = format!(
                    "TikTok profile '@{}' (source id {}) is private and the signed-in account does not follow it, so no media is accessible.",
                    handle, context.source.id
                );
                let mark_error = set_source_sync_problem(
                    connection,
                    &context.source.id,
                    problem_code,
                    &problem_message,
                    &finished_at,
                    true,
                );
                let mut summary = format!("TikTok sync skipped: {problem_message}");
                if let Err(mark_failure) = mark_error {
                    summary.push_str(&format!(
                        " Failed to persist source problem marker: {mark_failure}."
                    ));
                } else {
                    log_runtime_event(
                        layout,
                        "sync.profile",
                        "info",
                        RuntimeLogAnchor {
                            account_id: Some(&context.account.id),
                            provider: Some(&context.source.provider),
                            source_id: Some(&context.source.id),
                            source_handle: Some(&context.source.handle),
                        },
                        format!(
                            "Marked source '{}' as '{}': {}",
                            context.source.handle, problem_code, problem_message
                        ),
                        None,
                    );
                }
                let outcome = SourceSyncOutcome {
                    tool: "internal.tiktok".to_string(),
                    status: "skipped".to_string(),
                    summary: summary.clone(),
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
                    Some("Private profile".to_string()),
                    Some(outcome.summary.clone()),
                    false,
                    None,
                );
                return Ok(outcome);
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
            if collect_media_stats {
                upsert_tiktok_post_stats(
                    connection,
                    &context.source.id,
                    &result.observed_posts,
                    &finished_at,
                )?;
            }
            upsert_provider_sync_media_ledger_entries(
                connection,
                &ProviderSyncMediaScope {
                    provider: "tiktok",
                    source_id: &context.source.id,
                    account_id: &context.account.id,
                    source_handle: &handle,
                    profile_root: &profile_root,
                    timestamp: &finished_at,
                },
                &downloaded_media,
            )?;

            if let Some(user_id) = result.resolved_user_id.as_deref() {
                let _ = persist_tiktok_user_id_hint(
                    connection,
                    &context.source.id,
                    user_id,
                    &finished_at,
                );
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
                                RuntimeLogAnchor {
                                    account_id: Some(&context.account.id),
                                    provider: Some(&context.source.provider),
                                    source_id: Some(&context.source.id),
                                    source_handle: Some(&context.source.handle),
                                },
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
                        layout,
                        &context.source.id,
                        &avatar_path,
                        &finished_at,
                    );
                }
            }

            let downloaded = result.downloaded_media.len() + likes.downloaded;
            let stats_updated = likes.stats_updated
                + result
                    .observed_posts
                    .iter()
                    .filter(|post| {
                        post.view_count.is_some()
                            || post.like_count.is_some()
                            || post.comment_count.is_some()
                            || post.share_count.is_some()
                    })
                    .count();
            // Detalhamento técnico (scan, páginas, liked videos) → realtime
            // debugger; o resumo mostrado ao usuário fica curto e amigável.
            connector_debug::append_current(
                "internal.tiktok",
                "summary",
                "manifest",
                format!(
                    "scanned_posts={} parsed_pages={} discovered_assets={} queued_assets={} downloaded_assets={} skipped_existing_posts={} skipped_existing_assets={} stats_updated={} liked(pages={}, discovered={}, downloaded={}, skipped_existing={}, failed={}, stopped_incrementally={})",
                    result.manifest_summary.normalized_post_count,
                    result.manifest_summary.parsed_page_count,
                    result.manifest_summary.discovered_asset_count,
                    result.manifest_summary.queued_asset_count,
                    downloaded,
                    result.manifest_summary.skipped_existing_post_count,
                    result.manifest_summary.skipped_existing_asset_count,
                    stats_updated,
                    likes.pages_read,
                    likes.discovered,
                    likes.downloaded,
                    likes.skipped_existing,
                    likes.failed,
                    likes.stopped_incrementally,
                ),
            );
            let mut summary = format_download_success_summary("TikTok sync succeeded.", downloaded);
            summary.push_str(&format_already_up_to_date_suffix(
                result.manifest_summary.skipped_existing_post_count,
            ));
            if collect_media_stats && stats_updated > 0 {
                summary.push_str(&format!(" Stats updated for {stats_updated} post(s)."));
            }
            if liked_videos_enabled && likes.failed > 0 {
                summary.push_str(&format!(
                    " {} liked video(s) could not be downloaded.",
                    likes.failed
                ));
            }
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
            let cancelled_by_user = error
                .trim()
                .to_ascii_lowercase()
                .contains("cancelled by user");
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

    persist_source_sync_run(
        connection,
        context,
        &outcome,
        trigger,
        &started_at,
        &finished_at,
    )?;
    propagate_source_sync_account_health(connection, context, &outcome, &finished_at)?;
    // Sync bem-sucedido limpa qualquer marcador anterior (ex.: perfil que voltou
    // a ficar disponível deixa de exibir o badge "Profile unavailable").
    if outcome.status == "succeeded" {
        if let Err(error) = clear_source_sync_problem(connection, &context.source.id, &finished_at)
        {
            log_runtime_event(
                layout,
                "sync.profile",
                "warning",
                RuntimeLogAnchor {
                    account_id: Some(&context.account.id),
                    provider: Some(&context.source.provider),
                    source_id: Some(&context.source.id),
                    source_handle: Some(&context.source.handle),
                },
                format!(
                    "TikTok sync succeeded, but failed to clear source sync problem marker: {error}"
                ),
                Some(error),
            );
        }
    }
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
pub(super) struct SourceSyncContext {
    pub(super) source: SourceProfile,
    pub(super) account: ProviderAccount,
    pub(super) session_payload: String,
}
pub(super) struct SourceSyncOutcome {
    pub(super) tool: String,
    pub(super) status: String,
    pub(super) summary: String,
    pub(super) command_preview: String,
    pub(super) manifest_summary_json: Option<String>,
    pub(super) degraded_capabilities: Vec<String>,
    pub(super) validation_error: Option<String>,
}
pub(super) fn effective_instagram_sections_enabled(
    sections: &instagram_connector::InstagramSectionSelection,
) -> bool {
    sections.timeline
        || sections.reels
        || sections.stories
        || sections.stories_user
        || sections.tagged
}
pub(super) fn instagram_request_has_base_auth(
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
pub(super) fn read_instagram_sync_cooldown_until(
    settings: &HashMap<String, String>,
) -> Option<DateTime<Utc>> {
    settings
        .get(INSTAGRAM_SYNC_COOLDOWN_UNTIL_SETTING_KEY)
        .and_then(|value| parse_rfc3339_utc(value))
}
pub(super) fn set_instagram_sync_cooldown(
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
pub(super) fn clear_instagram_sync_cooldown(
    connection: &Connection,
    account_id: &str,
) -> Result<(), String> {
    delete_provider_account_setting(
        connection,
        account_id,
        INSTAGRAM_SYNC_COOLDOWN_UNTIL_SETTING_KEY,
    )
}
pub(super) fn instagram_error_indicates_availability_abort_rate_limit(error: &str) -> bool {
    instagram_error_indicates_rate_limit(error)
        && !instagram_error_is_inconclusive_identity_probe(error)
}
pub(super) fn blocked_instagram_source_sync_outcome(
    request: &instagram_connector::InstagramConnectorRequest,
    status: &str,
    summary: String,
    validation_error: Option<String>,
) -> Box<SourceSyncOutcome> {
    // Boxed: este outcome viaja como variante `Err` pelos preflights, e o
    // clippy aponta que carregar os 168+ bytes inline encarece cada `Result`.
    Box::new(SourceSyncOutcome {
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
    })
}
pub(super) fn validate_instagram_source_sync_preflight(
    connection: &Connection,
    context: &SourceSyncContext,
    request: &instagram_connector::InstagramConnectorRequest,
    settings: &HashMap<String, String>,
    now: &str,
) -> Result<(), Box<SourceSyncOutcome>> {
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
pub(super) fn source_user_id_hint_from_json(
    provider: &str,
    sync_options_json: &str,
) -> Option<String> {
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
/// Detecta, no primeiro sync de um perfil, se o `user_id` resolvido já pertence
/// a outro perfil. Em caso afirmativo, remove o recém-adicionado (soft-delete,
/// mantém mídia), registra no log e devolve um outcome explicativo a ser
/// reportado. Só age no primeiro sync (`last_synced_at` vazio) para não mexer em
/// perfis que já vinham sincronizando.
pub(super) fn detect_duplicate_user_id_on_first_sync(
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
    log_runtime_event(
        layout,
        "sync.profile",
        "warning",
        RuntimeLogAnchor {
            account_id: Some(&context.account.id),
            provider: Some(&context.source.provider),
            source_id: Some(&context.source.id),
            source_handle: Some(&context.source.handle),
        },
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
pub(super) fn resolve_instagram_source_identity_preflight(
    connection: &Connection,
    layout: &StorageLayout,
    context: &SourceSyncContext,
    source_options: &InstagramSourceSyncOptions,
    request: &mut instagram_connector::InstagramConnectorRequest,
    timestamp: &str,
) -> Result<Option<String>, Box<SourceSyncOutcome>> {
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
                    log_runtime_event(
                        layout,
                        "sync.profile",
                        "warning",
                        RuntimeLogAnchor {
                            account_id: Some(&context.account.id),
                            provider: Some(&context.source.provider),
                            source_id: Some(&context.source.id),
                            source_handle: Some(&context.source.handle),
                        },
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
                    true,
                );
                let mut summary = format!("Instagram sync skipped: {problem_message}");
                if let Err(mark_failure) = mark_error {
                    summary.push_str(&format!(
                        " Failed to persist source problem marker: {mark_failure}."
                    ));
                } else {
                    log_runtime_event(
                        layout,
                        "sync.profile",
                        "info",
                        RuntimeLogAnchor {
                            account_id: Some(&context.account.id),
                            provider: Some(&context.source.provider),
                            source_id: Some(&context.source.id),
                            source_handle: Some(&context.source.handle),
                        },
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
            return Err(Box::new(outcome));
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
                log_runtime_event(
                    layout,
                    "sync.profile",
                    "info",
                    RuntimeLogAnchor {
                        account_id: Some(&context.account.id),
                        provider: Some(&context.source.provider),
                        source_id: Some(&context.source.id),
                        source_handle: Some(&context.source.handle),
                    },
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
                log_runtime_event(
                    layout,
                    "sync.profile",
                    "warning",
                    RuntimeLogAnchor {
                        account_id: Some(&context.account.id),
                        provider: Some(&context.source.provider),
                        source_id: Some(&context.source.id),
                        source_handle: Some(&context.source.handle),
                    },
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
        log_runtime_event(
            layout,
            "sync.profile",
            "info",
            RuntimeLogAnchor {
                account_id: Some(&context.account.id),
                provider: Some(&context.source.provider),
                source_id: Some(&context.source.id),
                source_handle: Some(&context.source.handle),
            },
            message,
            None,
        );
        Ok(Some(format!(
            " Username changed from '{}' to '{}' (auto-update disabled).",
            context.source.handle, resolved_handle
        )))
    }
}
pub(super) fn set_source_sync_problem(
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum InstagramAvailabilityAction {
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
pub(super) fn decide_instagram_availability_action(
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
/// Acumulador de contadores e itens por perfil durante um availability check.
#[derive(Default)]
pub(super) struct AvailabilityCheckTally {
    pub(super) unchanged: u32,
    pub(super) updated_handle: u32,
    pub(super) marked_problem: u32,
    pub(super) skipped: u32,
    pub(super) failed: u32,
    pub(super) items: Vec<SourceAvailabilityCheckItem>,
}
pub(super) fn apply_instagram_availability_action(
    connection: &Connection,
    source_id: &str,
    provider: &str,
    previous_handle: &str,
    now: &str,
    action: InstagramAvailabilityAction,
    tally: &mut AvailabilityCheckTally,
) -> Result<(), String> {
    match action {
        InstagramAvailabilityAction::AbortedRateLimited(error) => {
            tally.failed += 1;
            tally.items.push(SourceAvailabilityCheckItem {
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
                tally.failed += 1;
                tally.items.push(SourceAvailabilityCheckItem {
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
                tally.unchanged += 1;
                tally.items.push(SourceAvailabilityCheckItem {
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
                    tally.updated_handle += 1;
                    tally.items.push(SourceAvailabilityCheckItem {
                        source_id: source_id.to_string(),
                        provider: provider.to_string(),
                        previous_handle: previous_handle.to_string(),
                        current_handle: Some(resolved_handle),
                        status: "updated_handle".to_string(),
                        message: "Handle was updated using current provider identity.".to_string(),
                    });
                }
                Err(error) => {
                    tally.failed += 1;
                    tally.items.push(SourceAvailabilityCheckItem {
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
                "Instagram profile '{}' (source id {}) appears to be private or temporarily restricted, so no media is accessible and download readiness is paused.",
                previous_handle, source_id
            );
            let marker = set_source_sync_problem(
                connection,
                source_id,
                "instagram_profile_private_or_restricted",
                &problem_message,
                now,
                true,
            );
            tally.marked_problem += 1;

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
                            tally.updated_handle += 1;
                        }
                        Err(error) => handle_update_error = Some(error),
                    }
                }
            }

            tally.items.push(SourceAvailabilityCheckItem {
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
            tally.marked_problem += 1;
            tally.items.push(SourceAvailabilityCheckItem {
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
            tally.failed += 1;
            tally.items.push(SourceAvailabilityCheckItem {
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
pub(super) fn build_availability_rate_limit_skipped_item(
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
pub(super) fn clear_source_sync_problem(
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
pub(super) fn load_latest_instagram_profile_user_id_hint(
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
pub(super) struct AccountSyncOutcome {
    pub(super) tool: String,
    pub(super) status: String,
    pub(super) summary: String,
    pub(super) command_preview: String,
}
pub(super) fn load_account_sync_context(
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
pub(super) fn execute_instagram_source_sync_with_connection(
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
        return Ok(*outcome);
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
            return Ok(*outcome);
        }
    }

    let cancel_token = register_source_sync_cancel_token(&context.source.id);

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
    let finished_at = now_timestamp();

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
                        RuntimeLogAnchor {
                            account_id: Some(&context.account.id),
                            provider: Some(&context.source.provider),
                            source_id: Some(&context.source.id),
                            source_handle: Some(&context.source.handle),
                        },
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
                                    RuntimeLogAnchor {
                                        account_id: Some(&context.account.id),
                                        provider: Some(&context.source.provider),
                                        source_id: Some(&context.source.id),
                                        source_handle: Some(&context.source.handle),
                                    },
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
                                    RuntimeLogAnchor {
                                        account_id: Some(&context.account.id),
                                        provider: Some(&context.source.provider),
                                        source_id: Some(&context.source.id),
                                        source_handle: Some(&context.source.handle),
                                    },
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
                            RuntimeLogAnchor {
                                account_id: Some(&context.account.id),
                                provider: Some(&context.source.provider),
                                source_id: Some(&context.source.id),
                                source_handle: Some(&context.source.handle),
                            },
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
                            RuntimeLogAnchor {
                                account_id: Some(&context.account.id),
                                provider: Some(&context.source.provider),
                                source_id: Some(&context.source.id),
                                source_handle: Some(&context.source.handle),
                            },
                            format!(
                                "Failed to persist Instagram biography for '{}': {}",
                                context.source.handle, error
                            ),
                            Some(error),
                        );
                    }
                }
            } else if let Some(error) = result.profile_description_error.as_deref() {
                log_runtime_event(
                    layout,
                    "sync.profile",
                    "warning",
                    RuntimeLogAnchor {
                        account_id: Some(&context.account.id),
                        provider: Some(&context.source.provider),
                        source_id: Some(&context.source.id),
                        source_handle: Some(&context.source.handle),
                    },
                    format!(
                        "Instagram biography was not refreshed for '{}'. Media sync can still complete.",
                        context.source.handle
                    ),
                    Some(error.to_string()),
                );
            }

            // Contadores estruturados do perfil (independem da nota de bio acima):
            // complementares, então um erro aqui nunca derruba o sync.
            if let Err(error) = update_source_profile_stats(
                connection,
                &context.source.id,
                result.profile_description.as_deref(),
                result.profile_counts.follower_count,
                result.profile_counts.following_count,
                result.profile_counts.media_count,
                result.profile_counts.is_verified,
                &finished_at,
            ) {
                log_runtime_event(
                    layout,
                    "sync.profile",
                    "warning",
                    RuntimeLogAnchor {
                        account_id: Some(&context.account.id),
                        provider: Some(&context.source.provider),
                        source_id: Some(&context.source.id),
                        source_handle: Some(&context.source.handle),
                    },
                    format!(
                        "Failed to persist Instagram profile stats for '{}'.",
                        context.source.handle
                    ),
                    Some(error),
                );
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
                            RuntimeLogAnchor {
                                account_id: Some(&context.account.id),
                                provider: Some(&context.source.provider),
                                source_id: Some(&context.source.id),
                                source_handle: Some(&context.source.handle),
                            },
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
                            RuntimeLogAnchor {
                                account_id: Some(&context.account.id),
                                provider: Some(&context.source.provider),
                                source_id: Some(&context.source.id),
                                source_handle: Some(&context.source.handle),
                            },
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
                        layout,
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
            let mut status = "failed".to_string();
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
                // Erros de identidade podem vazar do sync principal (o preflight
                // nem sempre roda; a timeline falha depois com o marcador do
                // probe embutido no erro). Classifica-os para pausar a fonte e
                // exibir o badge, como já faz o preflight de identidade.
                let marked = match classify_instagram_identity_error(&error) {
                    InstagramIdentityErrorClassification::UsernameUnresolvable => Some((
                        "instagram_username_unresolvable",
                        format!(
                            "Instagram profile '{}' (source id {}) could not be resolved. The account may have been renamed, disabled, or banned.",
                            context.source.handle, context.source.id
                        ),
                    )),
                    InstagramIdentityErrorClassification::PrivateOrRestricted => {
                        status = "skipped".to_string();
                        Some((
                            "instagram_profile_private_or_restricted",
                            format!(
                                "Instagram profile '{}' (source id {}) is private or restricted, so no media is accessible.",
                                context.source.handle, context.source.id
                            ),
                        ))
                    }
                    InstagramIdentityErrorClassification::Other => None,
                };
                if let Some((problem_code, problem_message)) = marked {
                    if let Err(mark_failure) = set_source_sync_problem(
                        connection,
                        &context.source.id,
                        problem_code,
                        &problem_message,
                        &finished_at,
                        true,
                    ) {
                        summary.push_str(&format!(
                            " Failed to persist source problem marker: {mark_failure}."
                        ));
                    } else {
                        log_runtime_event(
                            layout,
                            "sync.profile",
                            if status == "skipped" {
                                "info"
                            } else {
                                "warning"
                            },
                            RuntimeLogAnchor {
                                account_id: Some(&context.account.id),
                                provider: Some(&context.source.provider),
                                source_id: Some(&context.source.id),
                                source_handle: Some(&context.source.handle),
                            },
                            format!(
                                "Marked source '{}' as '{}': {}",
                                context.source.handle, problem_code, problem_message
                            ),
                            None,
                        );
                    }
                }
            }
            let validation_error = if cancelled_by_user || status == "skipped" {
                None
            } else {
                Some(error)
            };
            SourceSyncOutcome {
                tool: "internal.instagram".to_string(),
                status,
                summary,
                command_preview: format!(
                    "internal.instagram profile {} -> {}",
                    context.source.handle,
                    request.profile_root.display()
                ),
                manifest_summary_json: None,
                degraded_capabilities: Vec::new(),
                validation_error,
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
                RuntimeLogAnchor {
                    account_id: Some(&context.account.id),
                    provider: Some(&context.source.provider),
                    source_id: Some(&context.source.id),
                    source_handle: Some(&context.source.handle),
                },
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
pub(super) fn execute_instagram_saved_posts_sync_with_connection(
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
pub(super) fn build_instagram_profile_sync_request(
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
    cleanup_empty_media_artifacts(&profile_root)?;
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
        .unwrap_or_default();
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
        skip_errors_exclude: instagram_error_policy_settings(settings).0,
        log_skipped_errors: instagram_error_policy_settings(settings).1,
        tagged_notify_limit: instagram_error_policy_settings(settings).2,
        ignore_stories_560_errors: parse_bool_setting(
            settings
                .get("instagram.errors.ignoreStories560")
                .map(String::as_str),
            false,
        ),
        pacing: instagram_request_pacing(settings),
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
        full_scan: instagram_full_scan_enabled(&source_options),
        date_from_timestamp: explicit_date_from_timestamp
            .or_else(|| implicit_instagram_imported_cutoff_timestamp(&context.source, run_mode)),
        date_to_timestamp: instagram_date_to_timestamp(&source_options),
        media_file_naming_mode,
        media_file_naming_template,
        target_story_media_id: source_options.target_story_media_id.clone(),
    })
}
pub(super) fn build_instagram_section_selection(
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
pub(super) fn run_profile_post_sync_script(
    script_pattern: &str,
    profile_root: &Path,
) -> Result<(), String> {
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
pub(super) fn load_source_sync_context(
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
pub(super) fn build_source_sync_invocation(
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
pub(super) fn persist_source_sync_run(
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
    connector_debug::append_current(&outcome.tool, event_type, "sync.result", raw);

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

    if source_sync_status_is_productive(&outcome.status) {
        connection
            .execute(
                "UPDATE source_profiles
                 SET last_synced_at = ?2,
                     updated_at = ?2
                 WHERE id = ?1",
                params![&context.source.id, finished_at],
            )
            .map_err(|error| error.to_string())?;
    } else {
        connection
            .execute(
                "UPDATE source_profiles
                 SET updated_at = ?2
                 WHERE id = ?1",
                params![&context.source.id, finished_at],
            )
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}
pub(super) fn persist_account_sync_run(
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
pub(super) fn propagate_source_sync_account_health(
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
pub(super) fn ensure_instagram_sync_post_ledger_table(
    connection: &Connection,
) -> Result<(), String> {
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
/// Máximo de runs de histórico por entidade (source/account/plan) carregadas
/// no snapshot. As tabelas crescem sem poda e todo snapshot ia inteiro para o
/// webview a cada publicação; a UI mostra no máximo ~10 runs por entidade. O
/// cap é POR ENTIDADE (window function) — um LIMIT global apagaria o histórico
/// de perfis raramente sincronizados.
pub(super) const SYNC_RUN_HISTORY_CAP_PER_ENTITY: u32 = 20;

pub(super) fn load_source_sync_runs(connection: &Connection) -> Result<Vec<SourceSyncRun>, String> {
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
             FROM (
                SELECT
                    r.*,
                    ROW_NUMBER() OVER (
                        PARTITION BY source_id
                        ORDER BY finished_at DESC, created_at DESC
                    ) AS recency_rank
                FROM source_sync_runs r
             )
             WHERE recency_rank <= ?1
             ORDER BY finished_at DESC, created_at DESC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![SYNC_RUN_HISTORY_CAP_PER_ENTITY], |row| {
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
pub(super) fn load_account_sync_runs(
    connection: &Connection,
) -> Result<Vec<AccountSyncRun>, String> {
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
             FROM (
                SELECT
                    r.*,
                    ROW_NUMBER() OVER (
                        PARTITION BY account_id
                        ORDER BY finished_at DESC, created_at DESC
                    ) AS recency_rank
                FROM account_sync_runs r
             )
             WHERE recency_rank <= ?1
             ORDER BY finished_at DESC, created_at DESC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![SYNC_RUN_HISTORY_CAP_PER_ENTITY], |row| {
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
