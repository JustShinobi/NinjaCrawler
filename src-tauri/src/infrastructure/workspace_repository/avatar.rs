use super::*;

pub(super) const PROFILE_PICTURE_FILE_NAME: &str = "ProfilePicture.jpg";
pub(super) const PROFILE_SETTINGS_DIR_NAME: &str = "Settings";
#[derive(Clone, Copy)]
pub(super) enum ProfilePictureRefreshLogLevel {
    Info,
    Warning,
}
impl ProfilePictureRefreshLogLevel {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            ProfilePictureRefreshLogLevel::Info => "info",
            ProfilePictureRefreshLogLevel::Warning => "warning",
        }
    }
}
pub(super) struct ProfilePictureRefreshError {
    pub(super) level: ProfilePictureRefreshLogLevel,
    pub(super) message: String,
    pub(super) detail: Option<String>,
}
impl ProfilePictureRefreshError {
    pub(super) fn info(message: impl Into<String>) -> Self {
        Self {
            level: ProfilePictureRefreshLogLevel::Info,
            message: message.into(),
            detail: None,
        }
    }

    pub(super) fn warning(message: impl Into<String>) -> Self {
        Self {
            level: ProfilePictureRefreshLogLevel::Warning,
            message: message.into(),
            detail: None,
        }
    }

    pub(super) fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}
pub(super) fn parse_retry_after_duration(
    value: Option<&reqwest::header::HeaderValue>,
) -> Option<StdDuration> {
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
        // Drop the stale thumb before regenerating: the picked file can be
        // older than the previous thumb, which would defeat the mtime check.
        remove_avatar_thumbnail(layout, &source_id);
        let _ = ensure_avatar_thumbnail(layout, &source_id, &dest_path);
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

        remove_avatar_thumbnail(layout, &source_id);
        if let Some(avatar_path) = &auto_avatar {
            let _ = ensure_avatar_thumbnail(layout, &source_id, Path::new(avatar_path));
        }

        load_snapshot(connection, layout)
    })
}
pub(super) fn remove_source_custom_profile_images(
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
pub(super) fn is_profile_picture_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(PROFILE_PICTURE_FILE_NAME))
}
pub(super) fn profile_picture_path(output_root: &Path) -> PathBuf {
    output_root.join(PROFILE_PICTURE_FILE_NAME)
}
pub(super) fn ensure_profile_picture_at_root(
    output_root: &Path,
    candidate_path: &Path,
) -> Result<PathBuf, ProfilePictureRefreshError> {
    let target_path = profile_picture_path(output_root);
    if candidate_path != target_path {
        let temporary_path = output_root.join(format!("{PROFILE_PICTURE_FILE_NAME}.download"));
        if let Err(error) = fs::copy(candidate_path, &temporary_path) {
            let _ = fs::remove_file(&temporary_path);
            return Err(ProfilePictureRefreshError::warning(error.to_string()));
        }
        if target_path.exists() {
            let _ = fs::remove_file(&target_path);
        }

        if let Err(rename_error) = fs::rename(&temporary_path, &target_path) {
            if let Err(copy_error) = fs::copy(&temporary_path, &target_path) {
                let _ = fs::remove_file(&temporary_path);
                let _ = fs::remove_file(&target_path);
                return Err(ProfilePictureRefreshError::warning(format!(
                    "Failed to persist profile picture: {copy_error}"
                )));
            }
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
pub(super) fn cleanup_promoted_profile_picture_candidate(
    output_root: &Path,
    candidate_path: &Path,
) {
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
pub(super) fn settings_profile_picture_path(output_root: &Path) -> PathBuf {
    output_root
        .join(PROFILE_SETTINGS_DIR_NAME)
        .join(PROFILE_PICTURE_FILE_NAME)
}
pub(super) fn sync_profile_picture_to_settings(output_root: &Path) -> Result<PathBuf, String> {
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
pub(super) fn archive_profile_picture(existing_path: &Path) {
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
pub(super) fn find_source_avatar(output_root: &Path) -> Option<String> {
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
pub(super) fn refresh_profile_picture_from_provider(
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
pub(super) fn upgrade_twitter_avatar_url(url: &str) -> String {
    const SIZE_SUFFIXES: [&str; 7] = [
        "_normal",
        "_bigger",
        "_mini",
        "_200x200",
        "_400x400",
        "_x96",
        "_reasonably_small",
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
#[allow(clippy::too_many_arguments)]
pub(super) fn try_instagram_graphql_avatar(
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
    // Instagram's /api/graphql only accepts POST with a form-urlencoded body; a
    // GET with the same parameters in the query string is rejected with 400 (see
    // post_graphql_json in instagram_connector.rs). `av` (acting user id from the
    // `ds_user_id` cookie) and `jazoest` (a checksum of fb_dtsg) are required too.
    let mut body = String::new();
    if let Some(av) = avatar_cookie_value(cookie_header, "ds_user_id") {
        body.push_str("av=");
        body.push_str(&avatar_percent_encode(&av));
        body.push('&');
    }
    body.push_str("__comet_req=7&fb_dtsg=");
    body.push_str(&avatar_percent_encode(dtsg));
    body.push_str("&jazoest=");
    body.push_str(&avatar_percent_encode(&avatar_jazoest(dtsg)));
    body.push_str("&lsd=");
    body.push_str(&avatar_percent_encode(lsd));
    body.push_str("&fb_api_caller_class=RelayModern&fb_api_req_friendly_name=");
    body.push_str(&avatar_percent_encode(friendly_name));
    body.push_str("&doc_id=");
    body.push_str(&avatar_percent_encode(doc_id));
    body.push_str("&variables=");
    body.push_str(&avatar_percent_encode(&variables));
    body.push_str("&server_timestamps=true");

    let mut request = client
        .post("https://www.instagram.com/api/graphql")
        .header(reqwest::header::ACCEPT, "*/*")
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
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
    request = request.body(body);
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
pub(super) fn avatar_error_detail(
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
pub(super) fn avatar_percent_encode(value: &str) -> String {
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
pub(super) fn avatar_cookie_value(cookie_header: &str, name: &str) -> Option<String> {
    cookie_header
        .split(';')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
/// Facebook/Instagram anti-CSRF checksum derived from `fb_dtsg`: the literal `2`
/// followed by the sum of the token's byte values.
pub(super) fn avatar_jazoest(token: &str) -> String {
    let sum: u32 = token.bytes().map(u32::from).sum();
    format!("2{sum}")
}
pub(super) struct TopSearchUserResult {
    pub(super) user_id: String,
    pub(super) profile_pic_url: Option<String>,
}
pub(super) fn update_source_profile_image(
    connection: &Connection,
    layout: &StorageLayout,
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
    // The avatar was just downloaded, so the file is hot in the page cache —
    // refresh the local thumb now instead of on the next batch request.
    let _ = ensure_avatar_thumbnail(layout, source_id, Path::new(image_path));
    Ok(())
}

pub(super) const AVATAR_THUMB_MAX_DIMENSION: u32 = 256;
const AVATAR_THUMB_JPEG_QUALITY: u8 = 80;

pub(super) fn avatar_thumbs_dir(layout: &StorageLayout) -> PathBuf {
    layout.cache_root.join("avatar-thumbs")
}

/// Diretório dos thumbs de avatar do layout corrente. Exposto para o setup do
/// asset protocol liberar explicitamente esta pasta (o escopo estático de
/// `tauri.conf.json` também a cobre via `$LOCALAPPDATA/NinjaCrawler/**`, mas
/// liberar em runtime não depende da resolução daquela variável).
pub fn avatar_thumbnail_dir() -> Result<PathBuf, String> {
    let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    Ok(avatar_thumbs_dir(&layout))
}

/// mtime do original em milissegundos — compõe o nome do thumb (`{id}.{mtime}
/// .jpg`) para servir de cache-buster: quando o avatar troca, o path muda e o
/// webview não serve o jpg antigo do cache. Query strings (`?v=`) não servem:
/// o asset protocol do Windows (`http://asset.localhost`) não as ignora e
/// tenta abrir um arquivo inexistente.
fn original_mtime_millis(original: &Path) -> Option<u64> {
    fs::metadata(original)
        .and_then(|meta| meta.modified())
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}

fn avatar_thumb_path(layout: &StorageLayout, source_id: &str, version: u64) -> PathBuf {
    avatar_thumbs_dir(layout).join(format!("{source_id}.{version}.jpg"))
}

/// Devolve `<cache-root>/avatar-thumbs/<source-id>.<mtime>.jpg`, gerando-o se
/// preciso. Os originais moram no volume de mídia (possivelmente HD externo); a
/// lista de perfis exibe estes jpgs pequenos no disco local em vez de
/// decodificar cada `ProfilePicture.jpg` em resolução original. O mtime no nome
/// invalida o thumb quando o original é substituído. Retorna `None` quando o
/// original sumiu ou o formato não é decodificável (gif/bmp ficam fora do crate
/// `image` — o front cai no arquivo original nesses casos).
pub(super) fn ensure_avatar_thumbnail(
    layout: &StorageLayout,
    source_id: &str,
    original: &Path,
) -> Option<String> {
    let version = original_mtime_millis(original)?;
    let thumbs_dir = avatar_thumbs_dir(layout);
    fs::create_dir_all(&thumbs_dir).ok()?;
    let output = avatar_thumb_path(layout, source_id, version);
    if fs::metadata(&output).is_ok_and(|meta| meta.len() > 0) {
        return Some(output.to_string_lossy().to_string());
    }
    // Versão obsoleta (ou nenhuma): limpa qualquer `{id}.*.jpg` antigo antes de
    // escrever a nova para o cache não acumular um jpg por sync.
    remove_avatar_thumbnail(layout, source_id);

    // A extensão mente com frequência (avatares baixados são sempre salvos
    // como ProfilePicture.jpg, seja qual for o encoding real).
    let decoded = image::ImageReader::open(original)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    let thumbnail = decoded.thumbnail(AVATAR_THUMB_MAX_DIMENSION, AVATAR_THUMB_MAX_DIMENSION);
    // JPEG não tem canal alfa; encodar RGBA falha em vez de degradar.
    let rgb = thumbnail.to_rgb8();
    let mut encoded: Vec<u8> = Vec::new();
    let encoder =
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, AVATAR_THUMB_JPEG_QUALITY);
    rgb.write_with_encoder(encoder).ok()?;
    if encoded.is_empty() {
        return None;
    }

    static TEMP_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp_output = thumbs_dir.join(format!(
        ".{source_id}.{}.{}.tmp.jpg",
        std::process::id(),
        sequence
    ));
    if fs::write(&temp_output, &encoded).is_err() {
        let _ = fs::remove_file(&temp_output);
        return None;
    }
    // Outra geração concorrente pode ter vencido; qualquer jpg completo é
    // equivalente para o mesmo original.
    if output.is_file() || fs::rename(&temp_output, &output).is_err() {
        let _ = fs::remove_file(&temp_output);
    }
    if output.is_file() {
        Some(output.to_string_lossy().to_string())
    } else {
        None
    }
}

/// Remove todos os thumbs versionados de um source (`{id}.*.jpg`). Os arquivos
/// temporários (`.{id}.…tmp.jpg`) começam com ponto e não casam com o prefixo.
pub(super) fn remove_avatar_thumbnail(layout: &StorageLayout, source_id: &str) {
    let Ok(entries) = fs::read_dir(avatar_thumbs_dir(layout)) else {
        return;
    };
    let prefix = format!("{source_id}.");
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(&prefix) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Gera (ou reaproveita) os thumbs de avatar dos perfis pedidos — todos quando
/// `source_ids` é `None`. Lê os paths do DB e solta a conexão ANTES de tocar
/// nas imagens: os originais podem morar em volume lento e `with_workspace` é
/// o gate global do banco.
pub fn load_avatar_thumbnails(
    source_ids: Option<Vec<String>>,
) -> Result<AvatarThumbnailBatch, String> {
    let (layout, entries) = with_workspace(|connection, layout| {
        let mut statement = connection
            .prepare(
                "SELECT id, profile_image_path FROM source_profiles
                 WHERE deleted_at IS NULL AND profile_image_path IS NOT NULL",
            )
            .map_err(|error| error.to_string())?;
        let rows: Vec<(String, String)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|error| error.to_string())?
            .filter_map(|row| row.ok())
            .collect();
        Ok((layout.clone(), rows))
    })?;

    let requested: Option<HashSet<String>> = source_ids.map(|ids| ids.into_iter().collect());
    let entries: Vec<(String, String)> = entries
        .into_iter()
        .filter(|(id, _)| {
            requested
                .as_ref()
                .is_none_or(|wanted| wanted.contains(id))
        })
        .collect();

    // O decode do original é o gargalo (I/O + jpeg full-res): reparte o lote
    // entre alguns workers para o primeiro paint não esperar a fila inteira.
    const AVATAR_THUMB_WORKERS: usize = 4;
    let chunk_size = entries.len().div_ceil(AVATAR_THUMB_WORKERS).max(1);
    let mut thumbs: Vec<AvatarThumbnail> = Vec::with_capacity(entries.len());
    std::thread::scope(|scope| {
        let workers: Vec<_> = entries
            .chunks(chunk_size)
            .map(|chunk| {
                let layout = &layout;
                scope.spawn(move || {
                    chunk
                        .iter()
                        .filter_map(|(source_id, original)| {
                            let original_path = Path::new(original);
                            let version = original_mtime_millis(original_path).unwrap_or(0);
                            ensure_avatar_thumbnail(layout, source_id, original_path).map(|path| {
                                AvatarThumbnail {
                                    source_id: source_id.clone(),
                                    path,
                                    version,
                                }
                            })
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for worker in workers {
            if let Ok(results) = worker.join() {
                thumbs.extend(results);
            }
        }
    });

    Ok(AvatarThumbnailBatch { thumbs })
}
