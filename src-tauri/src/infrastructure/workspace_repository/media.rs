use super::*;

#[derive(Default)]
pub(super) struct InstagramMediaAliasSnapshot {
    pub(super) keys: HashSet<String>,
}
/// Loads the relative-path → link map used to rebuild post URLs and resolve the
/// feed/reels section. Instagram keeps its own ledger (with the case-sensitive
/// shortcode); TikTok/Twitter share the provider-neutral ledger (with the post
/// key and capture time). Returns an empty map for legacy media without ledger
/// rows — the gallery then falls back to file-name derivation.
pub(super) fn load_gallery_media_ledger_links(
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
                            downloaded_at: None,
                            title: None,
                            duration_seconds: None,
                        },
                    );
                }
            }
        }
        // Fallback para imports legados (SCrawler) baixados ANTES do shortcode ser
        // persistido no ledger: lê o código (casing original) direto do XML.
        for (relative_path, (post_code, section)) in load_legacy_instagram_post_codes(profile_root)
        {
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
        "SELECT relative_path, media_section, provider_post_key, captured_at, first_seen_at,
                title, duration_seconds
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
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<i64>>(6)?,
        ))
    });
    if let Ok(rows) = rows {
        for row in rows.flatten() {
            let (relative_path, section, post_key, captured_at, first_seen_at, title, duration_seconds) =
                row;
            links.insert(
                relative_path.to_ascii_lowercase(),
                GalleryMediaLedgerLink {
                    post_key,
                    post_code: None,
                    section,
                    captured_at,
                    downloaded_at: first_seen_at.as_deref().and_then(parse_rfc3339_unix),
                    title,
                    duration_seconds,
                },
            );
        }
    }
    links
}
/// Resolves either the system tools or NinjaCrawler's private FFmpeg runtime.
pub(super) fn ffmpeg_available() -> bool {
    media_tool_runtime::resolve().is_some()
}
/// Gera (ou reaproveita) thumbnails jpg para os vídeos pedidos. O
/// grid do Profile View usa `<img>` no lugar de `<video preload=metadata>` —
/// milhares de media elements montados/desmontados no scroll travam o webview
/// (limite de ~75 por página no Chromium). Cada jpg fica em `.thumbs/` ao lado
/// da mídia de origem, no mesmo volume — nunca ocupa silenciosamente o disco C.
pub fn load_media_thumbnails(paths: Vec<String>) -> Result<MediaThumbnailBatch, String> {
    // Remove o cache experimental da implementação anterior. É totalmente
    // regenerável e o usuário não deve ficar com uma cópia órfã no disco C.
    if let Ok(layout) = storage::ensure_workspace_layout() {
        let _ = fs::remove_dir_all(layout.cache_root.join("video-thumbs"));
    }
    let ffmpeg = ffmpeg_available();
    // Fotos usam o crate `image` (não dependem de ffmpeg); vídeos usam ffmpeg.
    let (images, videos): (Vec<String>, Vec<String>) = paths
        .into_iter()
        .partition(|path| is_thumbnailable_image(Path::new(path)));

    let mut thumbs = HashMap::new();
    // Fotos: sempre geram (thumb ~480px ao lado da mídia).
    for (path, thumb) in generate_media_thumbnails_parallel(&images, ensure_image_thumbnail) {
        thumbs.insert(path, thumb);
    }
    // Vídeos: só quando o ffmpeg está disponível; senão o front cai no <video>.
    if ffmpeg {
        for (path, thumb) in generate_media_thumbnails_parallel(&videos, ensure_video_thumbnail) {
            thumbs.insert(path, thumb);
        }
    }

    // `available` reflete apenas a disponibilidade de thumbs de VÍDEO (ffmpeg);
    // o front usa isso para o fallback de <video>. As fotos não afetam o flag.
    Ok(MediaThumbnailBatch {
        available: ffmpeg,
        thumbs,
    })
}

/// Reparte o lote entre alguns workers para o primeiro paint da viewport não
/// esperar a fila inteira (o gargalo é o decode/ffmpeg por arquivo).
fn generate_media_thumbnails_parallel(
    paths: &[String],
    generate: fn(&Path) -> Option<String>,
) -> Vec<(String, String)> {
    if paths.is_empty() {
        return Vec::new();
    }
    const THUMBNAIL_WORKERS: usize = 4;
    let chunk_size = paths.len().div_ceil(THUMBNAIL_WORKERS).max(1);
    let mut results = Vec::new();
    std::thread::scope(|scope| {
        let workers: Vec<_> = paths
            .chunks(chunk_size)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .filter_map(|path| {
                            generate(Path::new(path)).map(|thumb| (path.clone(), thumb))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for worker in workers {
            if let Ok(chunk_results) = worker.join() {
                results.extend(chunk_results);
            }
        }
    });
    results
}
/// Backfill assíncrono usado depois de um sync: novos downloads já saem com
/// thumbnail, e instalações existentes migram gradualmente sem ocupar o C:.
pub fn prewarm_source_media_thumbnails(source_id: String) -> Result<usize, String> {
    static PREWARM_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = PREWARM_LOCK
        .lock()
        .map_err(|_| "Media thumbnail prewarm lock is poisoned.".to_string())?;
    let mut paths = media_thumbnail_source_paths(&source_id)?;
    paths.sort();
    paths.dedup();
    let requested = paths.len();
    let _ = load_media_thumbnails(paths)?;
    Ok(requested)
}

/// Extensões de imagem que o crate `image` (features jpeg/png/webp) decodifica.
/// gif/bmp ficam de fora e caem no arquivo original no front.
pub(super) fn is_thumbnailable_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("jpg" | "jpeg" | "png" | "webp")
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MediaThumbnailGenerationOutcome {
    Generated,
    /// Small image that already fits the thumb budget — not an error.
    NotNeeded,
    /// Media has no usable visual content (audio-only / empty / corrupt container).
    /// Surfaces as a queue **warning** with manual review + recycle-bin option.
    InvalidMedia { reason: String },
    /// Tooling/IO failure while generating a thumb for otherwise valid media.
    Failed { reason: String },
}

#[derive(Debug)]
enum VideoThumbnailEnsureResult {
    Ready(String),
    /// Sem stream de vídeo / mídia sem conteúdo visual utilizável.
    InvalidMedia { reason: String },
    Failed { detail: String },
}

/// Generate (or classify) a media thumbnail beside the source file.
/// Returns `InvalidMedia` for non-visual/corrupt content (warning + review path)
/// and `Failed` for tooling/IO errors.
pub(crate) fn generate_media_thumbnail(source: &Path) -> MediaThumbnailGenerationOutcome {
    if is_thumbnailable_image(source) {
        return match generate_image_thumbnail(source) {
            Some(ImageThumbnailGeneration::Generated(_)) => {
                MediaThumbnailGenerationOutcome::Generated
            }
            Some(ImageThumbnailGeneration::NotNeeded) => MediaThumbnailGenerationOutcome::NotNeeded,
            None => {
                let reason =
                    "Image is undecodable or corrupt (thumbnail could not be generated).".to_string();
                log_thumbnail_issue(source, "warn", "invalid_media", &reason);
                MediaThumbnailGenerationOutcome::InvalidMedia { reason }
            }
        };
    }

    match ensure_video_thumbnail_result(source) {
        VideoThumbnailEnsureResult::Ready(_) => MediaThumbnailGenerationOutcome::Generated,
        VideoThumbnailEnsureResult::InvalidMedia { reason } => {
            log_thumbnail_issue(source, "warn", "invalid_media", &reason);
            MediaThumbnailGenerationOutcome::InvalidMedia { reason }
        }
        VideoThumbnailEnsureResult::Failed { detail } => {
            log_thumbnail_issue(source, "warn", "generation_failed", &detail);
            MediaThumbnailGenerationOutcome::Failed { reason: detail }
        }
    }
}

fn log_thumbnail_issue(source: &Path, level: &str, kind: &str, detail: &str) {
    let _ = runtime_log::append_workspace(
        "media.thumbnail",
        level,
        RuntimeLogAnchor::default(),
        format!("Thumbnail {kind} for '{}'.", source.display()),
        Some(detail.to_string()),
    );
}

/// Absolute media root for a source profile — used by the thumbnail queue to
/// derive `relative_path` for recycle-bin + ledger deletion.
pub fn media_thumbnail_source_root(source_id: &str) -> Result<PathBuf, String> {
    with_workspace(|connection, layout| {
        let row = connection
            .query_row(
                "SELECT provider, handle, account_id, sync_options_json FROM source_profiles
                 WHERE id = ?1 AND deleted_at IS NULL LIMIT 1",
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
            .ok_or_else(|| format!("Source '{source_id}' does not exist."))?;
        let (provider, handle, account_id, sync_options_json) = row;
        let source = SourceProfile {
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
        resolved_source_media_output_root_with_connection(connection, layout, &source)
    })
}

pub fn media_thumbnail_source_seed(source_id: &str) -> Result<(String, String), String> {
    with_workspace(|connection, _| {
        connection
            .query_row(
                "SELECT provider, handle FROM source_profiles
                 WHERE id = ?1 AND deleted_at IS NULL LIMIT 1",
                params![source_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Source '{source_id}' does not exist."))
    })
}
pub fn media_thumbnail_source_paths(source_id: &str) -> Result<Vec<String>, String> {
    let gallery = load_source_media_gallery(source_id.to_string())?;
    let mut paths: Vec<String> = gallery
        .posts
        .into_iter()
        .flat_map(|post| post.files)
        .filter(|file| {
            file.media_type == "video" || is_thumbnailable_image(Path::new(&file.absolute_path))
        })
        .map(|file| file.absolute_path)
        .collect();
    paths.sort();
    paths.dedup();
    Ok(paths)
}
pub(super) fn video_thumbnail_path(source: &Path) -> Option<PathBuf> {
    let media_dir = source.parent()?;
    let source_name = source.file_name()?.to_string_lossy();
    Some(media_dir.join(".thumbs").join(format!("{source_name}.jpg")))
}
/// Devolve `<media-dir>/.thumbs/<arquivo>.<ext>.jpg`, gerando-o com ffmpeg se
/// necessário. O mtime invalida o jpg quando o vídeo de origem é substituído.
/// Retorna `None` quando o arquivo sumiu ou o ffmpeg não conseguiu decodificar.
pub(crate) fn media_thumbnail_is_current(source: &Path) -> bool {
    let Some(output) = video_thumbnail_path(source) else {
        return false;
    };
    let source_mtime = fs::metadata(source).and_then(|meta| meta.modified()).ok();
    fs::metadata(output)
        .ok()
        .filter(|thumb| thumb.len() > 0)
        .and_then(|thumb| thumb.modified().ok())
        .zip(source_mtime)
        .is_some_and(|(thumb_mtime, media_mtime)| thumb_mtime >= media_mtime)
}
pub(crate) fn ensure_video_thumbnail(source: &Path) -> Option<String> {
    match ensure_video_thumbnail_result(source) {
        VideoThumbnailEnsureResult::Ready(path) => Some(path),
        VideoThumbnailEnsureResult::InvalidMedia { .. }
        | VideoThumbnailEnsureResult::Failed { .. } => None,
    }
}

/// Gera thumb de vídeo e classifica o resultado: pronto, mídia inválida
/// (sem stream visual) ou falha de ferramenta com detalhe (stderr/ffmpeg).
fn ensure_video_thumbnail_result(source: &Path) -> VideoThumbnailEnsureResult {
    if fs::metadata(source).is_err() {
        return VideoThumbnailEnsureResult::Failed {
            detail: format!("Source media is missing: {}", source.display()),
        };
    }
    let Some(output) = video_thumbnail_path(source) else {
        return VideoThumbnailEnsureResult::Failed {
            detail: format!(
                "Could not derive thumbnail path for '{}'.",
                source.display()
            ),
        };
    };
    let Some(thumbs_dir) = output.parent().map(Path::to_path_buf) else {
        return VideoThumbnailEnsureResult::Failed {
            detail: format!(
                "Could not resolve .thumbs directory for '{}'.",
                source.display()
            ),
        };
    };
    if let Err(error) = fs::create_dir_all(&thumbs_dir) {
        return VideoThumbnailEnsureResult::Failed {
            detail: format!(
                "Failed to create thumbnail directory '{}': {error}",
                thumbs_dir.display()
            ),
        };
    }
    let source_name = source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "media".to_string());
    if media_thumbnail_is_current(source) {
        return VideoThumbnailEnsureResult::Ready(output.to_string_lossy().to_string());
    }
    let _ = fs::remove_file(&output);

    // Sem stream de vídeo: mídia online/local inválida ou só áudio — warning +
    // revisão manual (não conta como falha de pipeline).
    if media_lacks_video_stream(source) {
        return VideoThumbnailEnsureResult::InvalidMedia {
            reason: "No video stream (invalid or non-visual media). Manual check recommended."
                .to_string(),
        };
    }

    let Some(ffmpeg) = media_tool_runtime::ffmpeg_executable() else {
        return VideoThumbnailEnsureResult::Failed {
            detail: "FFmpeg is not available on PATH or as a managed runtime.".to_string(),
        };
    };

    static TEMP_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp_output = thumbs_dir.join(format!(
        ".{source_name}.{}.{}.tmp.jpg",
        std::process::id(),
        sequence
    ));
    // -ss antes do -i (seek rápido); vídeos mais curtos que o seek não emitem
    // frame algum, então tenta 1s e cai para o 1º frame.
    let mut last_detail = String::from("FFmpeg did not produce a thumbnail frame.");
    for seek in ["1", "0"] {
        let mut command = Command::new(&ffmpeg);
        configure_background_command(&mut command);
        media_tool_runtime::configure_tool_path(&mut command);
        let result = command
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error")
            .arg("-y")
            .arg("-ss")
            .arg(seek)
            .arg("-i")
            .arg(source)
            .arg("-frames:v")
            .arg("1")
            .arg("-vf")
            .arg("scale=min(480\\,iw):-2")
            .arg("-q:v")
            .arg("4")
            .arg(&temp_output)
            .output();
        match result {
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let ok = out.status.success()
                    && fs::metadata(&temp_output)
                        .map(|meta| meta.len() > 0)
                        .unwrap_or(false);
                if ok {
                    // Outra geração concorrente pode ter vencido; qualquer jpg completo
                    // é equivalente para a mesma mídia.
                    if output.is_file() || fs::rename(&temp_output, &output).is_err() {
                        let _ = fs::remove_file(&temp_output);
                    }
                    if output.is_file() {
                        return VideoThumbnailEnsureResult::Ready(
                            output.to_string_lossy().to_string(),
                        );
                    }
                    last_detail = format!(
                        "FFmpeg wrote a temp frame (seek={seek}s) but the final thumb could not be published."
                    );
                } else {
                    last_detail = if stderr.is_empty() {
                        format!(
                            "FFmpeg failed extracting frame at seek={seek}s (exit={}).",
                            out.status
                        )
                    } else {
                        format!("FFmpeg seek={seek}s: {stderr}")
                    };
                    // stderr típico de mídia sem stream de vídeo
                    if is_no_video_stream_message(&last_detail) {
                        let _ = fs::remove_file(&temp_output);
                        return VideoThumbnailEnsureResult::InvalidMedia {
                            reason: format!(
                                "No extractable video frame ({last_detail}). Manual check recommended."
                            ),
                        };
                    }
                }
            }
            Err(error) => {
                last_detail = format!("Failed to spawn FFmpeg: {error}");
            }
        }
        let _ = fs::remove_file(&temp_output);
    }
    // Não deixa um jpg vazio/parcial envenenar o cache.
    let _ = fs::remove_file(&temp_output);
    VideoThumbnailEnsureResult::Failed {
        detail: last_detail,
    }
}

/// `true` quando o arquivo existe e o ffprobe confirma **ausência** de stream de
/// vídeo. `false` quando há stream de vídeo **ou** o probe não pôde decidir
/// (falha de ferramenta) — nesse caso o caller ainda tenta o ffmpeg.
fn media_lacks_video_stream(source: &Path) -> bool {
    let Some(ffprobe) = media_tool_runtime::ffprobe_executable() else {
        return false;
    };
    let mut command = Command::new(&ffprobe);
    configure_background_command(&mut command);
    media_tool_runtime::configure_tool_path(&mut command);
    let Ok(output) = command
        .args([
            "-v",
            "error",
            "-select_streams",
            "v",
            "-show_entries",
            "stream=codec_type",
            "-of",
            "csv=p=0",
        ])
        .arg(source)
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    !stdout
        .lines()
        .any(|line| line.trim().eq_ignore_ascii_case("video"))
}

fn is_no_video_stream_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("does not contain any stream")
        || lower.contains("output file does not contain any stream")
        || lower.contains("matches no streams")
}
const MEDIA_IMAGE_THUMB_MAX_DIMENSION: u32 = 480;
const MEDIA_IMAGE_THUMB_JPEG_QUALITY: u8 = 80;
enum ImageThumbnailGeneration {
    Generated(String),
    NotNeeded,
}

/// Gera (ou reaproveita) um thumb jpg ~480px de uma FOTO, em `.thumbs/` ao lado
/// da mídia — mesma convenção dos thumbs de vídeo, mas via crate `image` (fotos
/// não precisam de ffmpeg). Sem os thumbs, o webview decodifica cada foto em
/// resolução original (5-64 MB de bitmap).
///
/// A decisão de gerar é por DIMENSÃO, não por tamanho de arquivo: o custo de
/// RAM é largura×altura×4 no webview, e JPEGs bem comprimidos (Instagram) têm
/// dimensões grandes com poucos KB — ex.: 1440×1800 em ~120KB decodifica a
/// ~10MB, então PRECISA de thumb. Pula só quando ambos os lados já cabem em
/// 480px (senão `thumbnail()` faria upscale, gerando um thumb maior que o
/// original). Invalida por mtime; `None` quando o arquivo sumiu ou o formato
/// não é decodificável (gif/bmp → o front cai no arquivo original).
fn generate_image_thumbnail(source: &Path) -> Option<ImageThumbnailGeneration> {
    fs::metadata(source).ok()?;
    let output = video_thumbnail_path(source)?;
    let thumbs_dir = output.parent()?;
    fs::create_dir_all(thumbs_dir).ok()?;
    let source_name = source.file_name()?.to_string_lossy();
    if media_thumbnail_is_current(source) {
        return Some(ImageThumbnailGeneration::Generated(
            output.to_string_lossy().to_string(),
        ));
    }
    let _ = fs::remove_file(&output);

    // A extensão pode mentir; adivinha o formato pelo conteúdo.
    let decoded = image::ImageReader::open(source)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    // `thumbnail` faz upscale; se a imagem já cabe no limite, um thumb só
    // gastaria disco (e seria maior que o original). Devolve None → o front usa
    // o arquivo original, que já é pequeno.
    if decoded.width() <= MEDIA_IMAGE_THUMB_MAX_DIMENSION
        && decoded.height() <= MEDIA_IMAGE_THUMB_MAX_DIMENSION
    {
        return Some(ImageThumbnailGeneration::NotNeeded);
    }
    let thumbnail = decoded.thumbnail(
        MEDIA_IMAGE_THUMB_MAX_DIMENSION,
        MEDIA_IMAGE_THUMB_MAX_DIMENSION,
    );
    // JPEG não tem canal alfa; encodar RGBA falha em vez de degradar.
    let rgb = thumbnail.to_rgb8();
    let mut encoded: Vec<u8> = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
        &mut encoded,
        MEDIA_IMAGE_THUMB_JPEG_QUALITY,
    );
    rgb.write_with_encoder(encoder).ok()?;
    if encoded.is_empty() {
        return None;
    }

    static TEMP_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp_output = thumbs_dir.join(format!(
        ".{source_name}.{}.{}.img.tmp.jpg",
        std::process::id(),
        sequence
    ));
    if fs::write(&temp_output, &encoded).is_err() {
        let _ = fs::remove_file(&temp_output);
        return None;
    }
    // Geração concorrente pode ter vencido; qualquer jpg completo serve.
    if output.is_file() || fs::rename(&temp_output, &output).is_err() {
        let _ = fs::remove_file(&temp_output);
    }
    if output.is_file() {
        Some(ImageThumbnailGeneration::Generated(
            output.to_string_lossy().to_string(),
        ))
    } else {
        None
    }
}

pub(crate) fn ensure_image_thumbnail(source: &Path) -> Option<String> {
    match generate_image_thumbnail(source)? {
        ImageThumbnailGeneration::Generated(path) => Some(path),
        ImageThumbnailGeneration::NotNeeded => None,
    }
}
pub(super) fn ensure_provider_deleted_media_table(connection: &Connection) -> Result<(), String> {
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
pub(super) fn run_instagram_media_naming_ledger_backfill_with_connection<F>(
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

            if result.scanned_files.is_multiple_of(200) {
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

        // Recategorize in the sync ledger (the source of truth for Profile View
        // and sync dedupe) the media/posts whose `media_section` was recomputed
        // by the legacy inference — chiefly SCrawler reels previously marked as
        // `timeline` because their permalink was `/p/` instead of `/reel/`. The
        // upserts do `DO UPDATE`, so they reclassify existing rows without
        // duplicating, reusing the already-collected records (no extra hash/IO).
        if !legacy_by_key.is_empty() {
            let legacy_records = legacy_by_key.values().cloned().collect::<Vec<_>>();
            let downloaded_media =
                legacy_reconciliation_records_to_downloaded_media(&legacy_records);
            let observed_posts = legacy_reconciliation_records_to_observed_posts(&legacy_records);
            upsert_instagram_media_ledger_entries(
                connection,
                &source.id,
                account_id,
                &source.handle,
                &profile_root,
                &downloaded_media,
                &now,
            )?;
            upsert_instagram_post_ledger_entries(
                connection,
                &source.id,
                account_id,
                &source.handle,
                &observed_posts,
                &now,
            )?;
        }
    }

    upsert_app_setting_value(
        connection,
        INSTAGRAM_NAMING_LEDGER_BACKFILL_SETTING_KEY,
        "true",
    )?;

    on_progress(progress);
    Ok(result)
}
pub(super) fn load_provider_sync_media_ledger_keys(
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
/// Identidade do perfil/conta dona das entradas gravadas no media ledger de
/// um sync (mesma para todos os arquivos do lote).
pub(super) struct ProviderSyncMediaScope<'a> {
    pub(super) provider: &'a str,
    pub(super) source_id: &'a str,
    pub(super) account_id: &'a str,
    pub(super) source_handle: &'a str,
    pub(super) profile_root: &'a Path,
    pub(super) timestamp: &'a str,
}
pub(super) fn upsert_provider_sync_media_ledger_entries(
    connection: &Connection,
    scope: &ProviderSyncMediaScope<'_>,
    downloaded_media: &[twitter_connector::DownloadedTwitterMedia],
) -> Result<(), String> {
    let &ProviderSyncMediaScope {
        provider,
        source_id,
        account_id,
        source_handle,
        profile_root,
        timestamp,
    } = scope;
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
pub(super) fn backfill_provider_sync_media_ledger_post_keys(
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
pub(super) fn ensure_instagram_sync_media_ledger_table(
    connection: &Connection,
) -> Result<(), String> {
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
pub(super) fn ensure_instagram_media_key_aliases_table(
    connection: &Connection,
) -> Result<(), String> {
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
pub(super) fn load_instagram_media_alias_snapshot_for_source(
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
pub(super) fn load_instagram_media_ledger_snapshot_for_source(
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
pub(super) fn upsert_instagram_media_ledger_entries(
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
pub(super) fn compute_file_sha256(path: &Path) -> Result<String, String> {
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
pub(super) fn ensure_instagram_media_fingerprints_table(
    connection: &Connection,
) -> Result<(), String> {
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
pub(super) fn compute_instagram_media_fingerprint(
    path: &Path,
) -> Option<(u32, u32, String, String)> {
    if infer_media_type(path) != Some("image") {
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
/// Identidade e arquivo de uma mídia para a tabela de fingerprints. O escopo
/// (source/account/root/timestamp) fica em `InstagramFingerprintScope` porque
/// se repete para todos os arquivos do mesmo lote.
pub(super) struct InstagramFingerprintScope<'a> {
    pub(super) source_id: &'a str,
    pub(super) account_id: &'a str,
    pub(super) profile_root: &'a Path,
    pub(super) timestamp: &'a str,
}
pub(super) struct InstagramFingerprintMedia<'a> {
    pub(super) provider_media_key: &'a str,
    pub(super) media_type: &'a str,
    pub(super) media_section: &'a str,
    pub(super) file_path: &'a Path,
    pub(super) file_sha256: Option<&'a str>,
}
pub(super) fn upsert_instagram_media_fingerprint_row(
    connection: &Connection,
    scope: &InstagramFingerprintScope<'_>,
    media: &InstagramFingerprintMedia<'_>,
) -> Result<(), String> {
    let &InstagramFingerprintScope {
        source_id,
        account_id,
        profile_root,
        timestamp,
    } = scope;
    let &InstagramFingerprintMedia {
        provider_media_key,
        media_type,
        media_section,
        file_path,
        file_sha256,
    } = media;
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
pub(super) fn collect_instagram_media_alias_rows(
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
pub(super) fn upsert_instagram_media_alias_entries(
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
pub(super) fn upsert_instagram_media_fingerprint_entries(
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
            &InstagramFingerprintScope {
                source_id,
                account_id,
                profile_root,
                timestamp,
            },
            &InstagramFingerprintMedia {
                provider_media_key: &media.provider_media_key,
                media_type: &media.media_type,
                media_section: &media.media_section,
                file_path: &media.file_path,
                file_sha256: file_sha256.as_deref(),
            },
        )?;
    }

    Ok(())
}
pub(super) fn upsert_instagram_legacy_media_alias_entries(
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
pub(super) fn upsert_instagram_legacy_media_fingerprint_entries(
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
            &InstagramFingerprintScope {
                source_id,
                account_id,
                profile_root,
                timestamp,
            },
            &InstagramFingerprintMedia {
                provider_media_key: &record.provider_media_key,
                media_type: &record.media_type,
                media_section: &record.media_section,
                file_path: &record.file_path,
                file_sha256: record.file_sha256.as_deref(),
            },
        )?;
    }

    Ok(())
}
pub(super) fn ensure_instagram_media_naming_ledger_table(
    connection: &Connection,
) -> Result<(), String> {
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
pub(super) fn upsert_instagram_media_naming_ledger_entries(
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
pub(super) fn infer_media_type(path: &Path) -> Option<&'static str> {
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
