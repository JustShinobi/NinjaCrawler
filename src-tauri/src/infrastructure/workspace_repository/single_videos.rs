use super::*;

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
pub(super) enum SingleVideoUrlKind {
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

pub(super) fn single_video_url_kind(provider: &str, url: &str) -> SingleVideoUrlKind {
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

pub(super) fn extract_tiktok_rehydration_json(body: &str) -> Option<serde_json::Value> {
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

pub(super) fn tiktok_item_from_rehydration(value: &serde_json::Value) -> Option<&serde_json::Value> {
    value
        .get("__DEFAULT_SCOPE__")
        .and_then(|scope| scope.get("webapp.video-detail"))
        .and_then(|detail| detail.get("itemInfo"))
        .and_then(|info| info.get("itemStruct"))
}

pub(super) fn json_string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn parse_json_unix_timestamp(value: &serde_json::Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        value
            .as_str()
            .and_then(|raw| raw.trim().parse::<i64>().ok())
    })
}

pub(super) fn tiktok_photo_file_name(post_id: &str, index: usize, count: usize) -> String {
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

pub(super) fn requested_tiktok_image_index(url: &str, count: usize) -> usize {
    query_param_value(url, "image_index")
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .map(|value| value - 1)
        .filter(|index| *index < count)
        .unwrap_or(0)
}

pub(super) fn single_video_slideshow_paths(root: &Path, post_id: &str) -> Vec<PathBuf> {
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

pub(super) fn single_video_display_relative_path(
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

pub(super) fn single_video_audio_path(root: &Path, provider_video_id: Option<&str>) -> Option<PathBuf> {
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

pub(super) fn single_video_audio_relative_path(root: &Path, audio_path: Option<&Path>) -> Option<String> {
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
