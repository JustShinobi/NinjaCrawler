//! Connector interno do TikTok.
//!
//! O gallery-dl não consegue mais parsear o TikTok (cai num desafio JavaScript
//! e entra em loop de retries), então usamos **apenas o yt-dlp**, que resolve
//! tanto vídeos quanto posts de foto (slideshow):
//! 1. `--flat-playlist` enumera os posts do perfil (leve e rápido);
//! 2. filtramos contra o ledger e pelo tipo (vídeo/foto);
//! 3. baixamos os posts novos em lotes, e o naming/catálogo ficam sob controle
//!    do NinjaCrawler (prefixo de data + ledger), como nos demais providers.

use chrono::{Local, TimeZone};
use reqwest::blocking::Client;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use std::time::Instant;

use crate::infrastructure::connector_debug;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const YT_DLP_LIST_TIMEOUT_SECS: u64 = 600;
const YT_DLP_DOWNLOAD_TIMEOUT_SECS: u64 = 3600;
const PHOTO_PAGE_TIMEOUT_SECS: u64 = 120;
const STORIES_TIMEOUT_SECS: u64 = 300;
const DOWNLOAD_BATCH_SIZE: usize = 40;
/// Alvo de impersonation do yt-dlp (curl_cffi). Sem isto o extractor do TikTok
/// falha com "Unable to extract webpage video data" (anti-bot por TLS).
const YT_DLP_IMPERSONATE: &str = "chrome";
const VIDEO_EXTENSIONS: [&str; 5] = ["mp4", "webm", "mkv", "mov", "m4v"];
const IMAGE_EXTENSIONS: [&str; 6] = ["jpg", "jpeg", "png", "webp", "heic", "gif"];
const AUDIO_EXTENSIONS: [&str; 5] = ["mp3", "m4a", "wav", "opus", "aac"];

#[derive(Clone, Copy, Default)]
pub struct TikTokSectionSelection {
    pub timeline: bool,
    pub stories: bool,
    pub reposts: bool,
}

#[derive(Clone)]
pub struct TikTokConnectorRequest {
    pub handle: String,
    pub yt_dlp_executable: PathBuf,
    /// Usado só para os Stories: o gallery-dl tem o extractor `/stories` (que,
    /// ao contrário do `/photo/`, não toma 403) e o yt-dlp não os suporta.
    pub gallery_dl_executable: PathBuf,
    /// Arquivo de cookies no formato Netscape já gravado pelo caller.
    pub cookie_file: PathBuf,
    pub user_agent: Option<String>,
    pub profile_root: PathBuf,
    /// Diretório de trabalho para os downloads temporários.
    pub cache_root: PathBuf,
    pub sections: TikTokSectionSelection,
    /// Quando presente, o sync baixa APENAS este vídeo (URL `/video/<id>`) na
    /// pasta `Stories/` do perfil — usado pela captura de story do Companion.
    pub target_video_url: Option<String>,
    pub download_videos: bool,
    pub download_photos: bool,
    /// Vídeos vão para a subpasta `Video` (SeparateVideoFolder do SCrawler).
    pub separate_video_folder: bool,
    /// Ajusta a data do arquivo para a data do post (yt-dlp --mtime).
    pub use_parsed_video_date: bool,
    /// Usa o título nativo do vídeo no nome do arquivo (SCrawler UseNativeTitle).
    pub use_native_title: bool,
    /// Anexa o id do vídeo ao título (SCrawler AddVideoIDToTitle). Só se aplica
    /// quando `use_native_title`.
    pub add_video_id_to_title: bool,
    /// Remove hashtags (`#tag`) do título antes de nomear (RemoveLastSymbols /
    /// RemoveTags do SCrawler). Só se aplica quando `use_native_title`.
    pub remove_tags_from_title: bool,
    /// Range de download (unix seconds), espelhando o 4K Tokkit. Posts fora de
    /// `[from, to]` são descartados na seleção (data derivada do id). `None` ou
    /// `0` desabilita o respectivo limite.
    pub download_from_date: Option<i64>,
    pub download_to_date: Option<i64>,
    /// Nomeia os arquivos no padrão do 4K Tokkit: `<handle>_<unix>_<post_id>.ext`
    /// (vídeo) e `<handle>_<unix>_<post_id>_index_<i>_<n>.jpeg` (foto), sem o
    /// prefixo de data. Tem precedência sobre `use_native_title`.
    pub tokkit_naming: bool,
    pub abort_on_limit: bool,
    /// Segundos entre lotes; `-1` desabilita (default SCrawler).
    pub sleep_timer_secs: i64,
    pub ledger_post_keys: HashSet<String>,
    pub ledger_media_keys: HashSet<String>,
    pub existing_relative_paths: HashSet<String>,
    /// Id numérico estável do dono do perfil (`userIdHint`), quando já conhecido.
    /// Usado para validar a identidade ao recuperar o handle após renomeação.
    pub user_id_hint: Option<String>,
}

#[derive(Clone)]
pub struct ObservedTikTokPost {
    pub provider_post_key: String,
    pub media_section: String,
}

#[derive(Clone)]
pub struct DownloadedTikTokMedia {
    pub file_path: PathBuf,
    pub media_type: String,
    pub media_section: String,
    pub provider_media_key: String,
    pub provider_post_key: String,
    pub captured_at_timestamp: Option<i64>,
    pub final_file_name: String,
}

#[derive(Clone, Default)]
pub struct TikTokManifestSummary {
    pub parsed_page_count: u32,
    pub normalized_post_count: u32,
    pub discovered_asset_count: u32,
    pub queued_asset_count: u32,
    pub skipped_existing_post_count: u32,
    pub skipped_existing_asset_count: u32,
    pub downloaded_asset_count: u32,
}

pub struct TikTokConnectorResult {
    pub observed_posts: Vec<ObservedTikTokPost>,
    pub downloaded_media: Vec<DownloadedTikTokMedia>,
    pub section_errors: Vec<String>,
    pub rate_limited: bool,
    pub limit_aborted: bool,
    /// uploader_id estável do TikTok, quando resolvido.
    pub resolved_user_id: Option<String>,
    /// URL do avatar do dono do perfil (não resolvido via yt-dlp hoje).
    pub resolved_avatar_url: Option<String>,
    /// Preenchido quando `is_duplicate_user_id` apontou que o user id já
    /// pertence a outro perfil; nesse caso o download foi cancelado.
    pub duplicate_user_id: Option<String>,
    /// Handle atual descoberto quando o perfil foi renomeado (o handle salvo
    /// parou de listar posts, mas um post conhecido resolveu para outro
    /// `uniqueId` com o mesmo `author.id`). O chamador atualiza o perfil.
    pub resolved_handle: Option<String>,
    pub manifest_summary: TikTokManifestSummary,
}

pub struct TikTokProgress {
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
}

pub fn run_profile_sync<F, C, D>(
    request: &TikTokConnectorRequest,
    mut report_progress: F,
    is_cancelled: C,
    is_duplicate_user_id: D,
) -> Result<TikTokConnectorResult, String>
where
    F: FnMut(TikTokProgress),
    C: Fn() -> bool,
    D: Fn(&str) -> bool,
{
    fs::create_dir_all(&request.cache_root).map_err(|error| error.to_string())?;
    fs::create_dir_all(&request.profile_root).map_err(|error| error.to_string())?;

    let handle = request.handle.trim().trim_start_matches('@').to_string();
    let profile_url = format!("https://www.tiktok.com/@{handle}");

    let mut summary = TikTokManifestSummary::default();

    // Story capturado pelo Companion: baixa só este vídeo na pasta Stories/ do
    // perfil (com os cookies da conta), sem enumerar a timeline.
    if let Some(target_url) = request
        .target_video_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let stories_dir = request.profile_root.join("Stories");
        fs::create_dir_all(&stories_dir).map_err(|error| error.to_string())?;
        report_progress(TikTokProgress {
            label: "Downloading story".to_string(),
            detail: format!("Downloading the selected story for '{handle}'."),
            downloaded_items: Some(0),
            progress_percent: None,
            indeterminate: true,
        });
        let downloaded = download_target_story_video(request, target_url, &stories_dir, &is_cancelled)?;
        let mut observed_posts = Vec::new();
        let mut downloaded_media = Vec::new();
        if let Some(media) = downloaded {
            summary.downloaded_asset_count += 1;
            observed_posts.push(ObservedTikTokPost {
                provider_post_key: media.provider_post_key.clone(),
                media_section: media.media_section.clone(),
            });
            downloaded_media.push(media);
        }
        return Ok(TikTokConnectorResult {
            observed_posts,
            downloaded_media,
            section_errors: Vec::new(),
            rate_limited: false,
            limit_aborted: false,
            resolved_user_id: None,
            resolved_avatar_url: None,
            duplicate_user_id: None,
            resolved_handle: None,
            manifest_summary: summary,
        });
    }
    let mut observed_posts: Vec<ObservedTikTokPost> = Vec::new();
    let mut downloaded_media: Vec<DownloadedTikTokMedia> = Vec::new();
    let mut section_errors: Vec<String> = Vec::new();
    let mut rate_limited = false;
    let mut limit_aborted = false;
    let mut duplicate_user_id: Option<String> = None;
    let mut resolved_handle: Option<String> = None;

    if is_cancelled() {
        return Err("source sync cancelled by user".to_string());
    }

    report_progress(TikTokProgress {
        label: "Parsing profile".to_string(),
        detail: format!("Listing TikTok posts for '{handle}'."),
        downloaded_items: Some(0),
        progress_percent: Some(0),
        indeterminate: true,
    });

    let listed = enumerate_posts(request, &profile_url, &is_cancelled)?;
    connector_debug::append_current(
        "internal.tiktok",
        "system",
        "listing.complete",
        format!(
            "posts_received={}\nuploader_id={}\nrate_limited={}",
            listed.posts.len(),
            listed.uploader_id.as_deref().unwrap_or("unknown"),
            listed.rate_limited
        ),
    );
    rate_limited = rate_limited || listed.rate_limited;
    let resolved_user_id = listed.uploader_id.clone();

    // Duplicata no primeiro sync: cancela antes de baixar qualquer coisa.
    if let Some(uid) = resolved_user_id.as_deref() {
        if is_duplicate_user_id(uid) {
            duplicate_user_id = Some(uid.to_string());
        }
    }

    // Recuperação de handle: se o perfil deixou de listar posts (sem rate
    // limit), o usuário pode ter renomeado a conta. Pegamos um post que já
    // conhecemos do ledger e abrimos sua página (o handle no path é ignorado);
    // se o `uniqueId` atual difere do salvo — e o `author.id` confirma a
    // identidade — devolvemos o novo handle para o chamador atualizar o perfil.
    if duplicate_user_id.is_none() && listed.posts.is_empty() && !listed.rate_limited {
        if let Some(known_post_id) = newest_known_timeline_post_id(&request.ledger_post_keys) {
            if let Some(author) = fetch_post_author(request, &known_post_id, &is_cancelled) {
                let identity_ok = match (request.user_id_hint.as_deref(), author.author_id.as_deref()) {
                    (Some(hint), Some(found)) => hint == found,
                    // Sem hint salvo, confiamos no post (já pertencia a este perfil).
                    (None, _) => true,
                    // Temos hint mas a página não trouxe o id: não arrisca renomear.
                    (Some(_), None) => false,
                };
                if let Some(current) = author.unique_id.as_deref() {
                    if identity_ok && !current.eq_ignore_ascii_case(&handle) {
                        resolved_handle = Some(current.to_string());
                    }
                }
            }
        }
    }

    // Avatar: o yt-dlp não expõe a foto do canal, então buscamos a página de um
    // post (write-pages + impersonate) e extraímos `author.avatarLarger`.
    let resolved_avatar_url = match (duplicate_user_id.is_none(), listed.posts.first()) {
        (true, Some(first_post)) => fetch_avatar(request, &first_post.post_id, &is_cancelled),
        _ => None,
    };

    if duplicate_user_id.is_none() && resolved_handle.is_none() {
        // Seleciona os posts novos (dedup por ledger). O tipo (vídeo/foto) só é
        // conhecido no download: o yt-dlp baixa o vídeo; posts de foto rendem
        // áudio-only e são roteados para o gallery-dl. Por isso a filtragem por
        // download_videos/download_photos acontece no download, não aqui.
        let from_date = request.download_from_date.filter(|value| *value > 0);
        let to_date = request.download_to_date.filter(|value| *value > 0);
        let mut seen: HashSet<String> = HashSet::new();
        let mut selected: Vec<EnumeratedPost> = Vec::new();
        for post in listed.posts {
            summary.normalized_post_count += 1;
            if !seen.insert(post.post_id.clone()) {
                continue;
            }
            // Range de data (4K Tokkit): a data de criação vem do id do post.
            // Posts sem timestamp legível não são filtrados (fail-open).
            if from_date.is_some() || to_date.is_some() {
                if let Some(created) = timestamp_from_tiktok_id(&post.post_id) {
                    if from_date.is_some_and(|from| created < from)
                        || to_date.is_some_and(|to| created > to)
                    {
                        continue;
                    }
                }
            }
            if request.ledger_post_keys.contains(&post.post_id) {
                summary.skipped_existing_post_count += 1;
                continue;
            }
            summary.discovered_asset_count += 1;
            selected.push(post);
        }
        summary.queued_asset_count = selected.len() as u32;
        connector_debug::append_current(
            "internal.tiktok",
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
            processed += batch.len();
            let done = processed.min(total);
            let percent = if total > 0 {
                ((done as f64 / total as f64) * 100.0).round() as u32
            } else {
                0
            };
            report_progress(TikTokProgress {
                label: "Downloading posts".to_string(),
                detail: format!("Post {done} of {total}"),
                downloaded_items: Some(summary.downloaded_asset_count),
                progress_percent: Some(percent.min(100)),
                indeterminate: false,
            });

            let batch_started = Instant::now();
            connector_debug::append_current(
                "internal.tiktok",
                "call",
                "batch.download",
                format!(
                    "batch_start={}\nbatch_end={done}\nbatch_size={}\ntotal_posts={total}\npost_ids={}",
                    done.saturating_sub(batch.len()).saturating_add(1),
                    batch.len(),
                    batch
                        .iter()
                        .map(|post| post.post_id.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                ),
            );
            match download_batch(request, batch, &is_cancelled) {
                Ok(outcome) => {
                    connector_debug::append_current(
                        "internal.tiktok",
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
                                "TikTok rate limit reached; remaining posts were skipped."
                                    .to_string(),
                            );
                            break;
                        }
                    }
                }
                Err(error) => {
                    connector_debug::append_current(
                        "internal.tiktok",
                        "error",
                        "batch.download",
                        format!(
                            "elapsed_ms={}\nerror={error}",
                            batch_started.elapsed().as_millis()
                        ),
                    );
                    let lowered = error.to_ascii_lowercase();
                    if lowered.contains("cancelled by user") {
                        return Err(error);
                    }
                    section_errors.push(format!("download batch failed: {error}"));
                }
            }

            if request.sleep_timer_secs > 0 && processed < total {
                thread::sleep(Duration::from_secs(request.sleep_timer_secs as u64));
            }
        }

        for post_id in downloaded_post_ids {
            observed_posts.push(ObservedTikTokPost {
                provider_post_key: post_id,
                media_section: "timeline".to_string(),
            });
        }

        // Stories (efêmeros, 24h) e Reposts: via gallery-dl.
        let extra_sections = [
            (request.sections.stories, GalleryDlSection::Stories, "stories"),
            (request.sections.reposts, GalleryDlSection::Reposts, "reposts"),
        ];
        for (enabled, section, label) in extra_sections {
            if !enabled {
                continue;
            }
            if is_cancelled() {
                return Err("source sync cancelled by user".to_string());
            }
            report_progress(TikTokProgress {
                label: format!("Downloading {label}"),
                detail: format!("{label} for '{handle}'."),
                downloaded_items: Some(summary.downloaded_asset_count),
                progress_percent: None,
                indeterminate: true,
            });
            match download_gallery_dl_section(request, section, &is_cancelled) {
                Ok(section_media) => {
                    let mut seen_posts: HashSet<String> = HashSet::new();
                    for media in section_media {
                        let media_section = media.media_section.clone();
                        if seen_posts.insert(media.provider_post_key.clone()) {
                            observed_posts.push(ObservedTikTokPost {
                                provider_post_key: media.provider_post_key.clone(),
                                media_section,
                            });
                        }
                        summary.downloaded_asset_count += 1;
                        downloaded_media.push(media);
                    }
                }
                Err(error) => {
                    if error.to_ascii_lowercase().contains("cancelled by user") {
                        return Err(error);
                    }
                    section_errors.push(format!("{label} failed: {error}"));
                }
            }
        }
    }

    report_progress(TikTokProgress {
        label: "Finished".to_string(),
        detail: format!("Downloaded {} media items.", summary.downloaded_asset_count),
        downloaded_items: Some(summary.downloaded_asset_count),
        progress_percent: Some(100),
        indeterminate: false,
    });

    Ok(TikTokConnectorResult {
        observed_posts,
        downloaded_media,
        section_errors,
        rate_limited,
        limit_aborted,
        resolved_user_id,
        resolved_avatar_url,
        duplicate_user_id,
        resolved_handle,
        manifest_summary: summary,
    })
}

/// Escolhe um post de timeline conhecido (id puramente numérico) do ledger para
/// usar na recuperação de handle. Ignora chaves `story_`/`repost_`. Retorna o
/// id "maior" (mais recente, já que o id cresce com o tempo de criação).
fn newest_known_timeline_post_id(ledger_post_keys: &HashSet<String>) -> Option<String> {
    ledger_post_keys
        .iter()
        .filter(|key| key.chars().all(|c| c.is_ascii_digit()) && !key.is_empty())
        .max_by_key(|key| (key.len(), key.as_str()))
        .cloned()
}

/// Dados do autor extraídos da página de um post (rehydration). `unique_id` é o
/// handle ATUAL (muda quando o usuário renomeia a conta); `author_id` é o id
/// numérico estável (bate com o `userIdHint`); `avatar_url` é a foto do perfil.
#[derive(Default)]
struct PostAuthor {
    unique_id: Option<String>,
    author_id: Option<String>,
    avatar_url: Option<String>,
}

/// Conveniência: só a URL do avatar do autor de um post.
fn fetch_avatar<C>(request: &TikTokConnectorRequest, post_id: &str, is_cancelled: &C) -> Option<String>
where
    C: Fn() -> bool,
{
    fetch_post_author(request, post_id, is_cancelled).and_then(|author| author.avatar_url)
}

/// Busca a página de um post conhecido com `yt-dlp --impersonate --write-pages`
/// (que passa pelo 403 que bloqueia o fetch direto) e extrai os dados do autor.
/// O handle no path da URL é ignorado pelo TikTok — só o id do vídeo importa —,
/// então isto funciona mesmo quando o handle do perfil mudou. Best-effort:
/// qualquer falha retorna `None`.
fn fetch_post_author<C>(
    request: &TikTokConnectorRequest,
    post_id: &str,
    is_cancelled: &C,
) -> Option<PostAuthor>
where
    C: Fn() -> bool,
{
    let post_id = post_id.trim();
    if post_id.is_empty() {
        return None;
    }
    let dir = request.cache_root.join(format!("author-{post_id}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).ok()?;
    let url = format!(
        "https://www.tiktok.com/@{}/video/{post_id}",
        request.handle.trim_start_matches('@')
    );

    let mut command = Command::new(&request.yt_dlp_executable);
    command
        .arg("--ignore-errors")
        .arg("--no-warnings")
        .arg("--impersonate")
        .arg(YT_DLP_IMPERSONATE)
        .arg("--skip-download")
        .arg("--write-pages")
        .arg("--no-cookies-from-browser")
        .arg("--cookies")
        .arg(&request.cookie_file)
        .arg(&url)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let _ = run_capturing(command, PHOTO_PAGE_TIMEOUT_SECS, is_cancelled, "yt-dlp (author page)");

    let author = fs::read_dir(&dir)
        .ok()
        .and_then(|entries| {
            entries.flatten().map(|entry| entry.path()).find(|path| {
                path.extension()
                    .and_then(|value| value.to_str())
                    .map(|ext| ext.eq_ignore_ascii_case("dump") || ext.eq_ignore_ascii_case("html"))
                    .unwrap_or(false)
            })
        })
        .and_then(|dump| fs::read_to_string(dump).ok())
        .and_then(|html| extract_rehydration_json(&html))
        .and_then(|json| -> Option<PostAuthor> {
            let author = json
                .get("__DEFAULT_SCOPE__")?
                .get("webapp.video-detail")?
                .get("itemInfo")?
                .get("itemStruct")?
                .get("author")?;
            let string_field = |key: &str| {
                author
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            };
            Some(PostAuthor {
                unique_id: string_field("uniqueId"),
                author_id: string_field("id"),
                avatar_url: string_field("avatarLarger")
                    .or_else(|| string_field("avatarMedium"))
                    .or_else(|| string_field("avatarThumb")),
            })
        });

    let _ = fs::remove_dir_all(&dir);
    author
}

/// Extrai o JSON do `<script id="__UNIVERSAL_DATA_FOR_REHYDRATION__">` da página.
fn extract_rehydration_json(body: &str) -> Option<Value> {
    let marker = "__UNIVERSAL_DATA_FOR_REHYDRATION__\"";
    let start = body.find(marker)? + marker.len();
    let rest = &body[start..];
    let json_start = rest.find('{')?;
    let json_slice = &rest[json_start..];
    // Encontra o `}` que fecha o objeto, balanceando chaves (ignorando as que
    // estiverem dentro de strings).
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
    let json_text = &json_slice[..end?];
    serde_json::from_str(json_text).ok()
}

/// Cliente reqwest para baixar as imagens dos posts de foto (o CDN de imagem
/// aceita GET simples, sem impersonation).
fn build_download_client(request: &TikTokConnectorRequest) -> Result<Client, String> {
    let mut builder = Client::builder().timeout(Duration::from_secs(120));
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

struct EnumeratedPosts {
    posts: Vec<EnumeratedPost>,
    uploader_id: Option<String>,
    rate_limited: bool,
}

/// Lista os posts do perfil (`--flat-playlist`), distinguindo vídeo de foto
/// pela URL (`/video/` vs `/photo/`).
fn enumerate_posts<C>(
    request: &TikTokConnectorRequest,
    profile_url: &str,
    is_cancelled: &C,
) -> Result<EnumeratedPosts, String>
where
    C: Fn() -> bool,
{
    let mut command = Command::new(&request.yt_dlp_executable);
    command
        .arg("--ignore-errors")
        .arg("--no-warnings")
        .arg("--impersonate")
        .arg(YT_DLP_IMPERSONATE)
        .arg("--flat-playlist")
        .arg("--print")
        .arg("%(id)s\t%(webpage_url)s\t%(uploader_id)s")
        .arg("--no-cookies-from-browser")
        .arg("--cookies")
        .arg(&request.cookie_file);
    apply_user_agent(&mut command, request);
    command
        .arg(profile_url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let (stdout, stderr) = run_capturing(
        command,
        YT_DLP_LIST_TIMEOUT_SECS,
        is_cancelled,
        "yt-dlp (listing)",
    )?;
    let rate_limited = output_is_rate_limited(&stderr);

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
        let uploader = parts.next().unwrap_or("").trim();
        if post_id.is_empty() || post_id == "NA" {
            continue;
        }
        if uploader_id.is_none() && !uploader.is_empty() && uploader != "NA" {
            uploader_id = Some(uploader.to_string());
        }
        let url = if webpage_url.is_empty() || webpage_url == "NA" {
            format!("https://www.tiktok.com/@{}/video/{post_id}", request.handle.trim_start_matches('@'))
        } else {
            webpage_url.to_string()
        };
        posts.push(EnumeratedPost {
            post_id: post_id.to_string(),
            webpage_url: url,
        });
    }

    Ok(EnumeratedPosts {
        posts,
        uploader_id,
        rate_limited,
    })
}

struct BatchOutcome {
    media: Vec<DownloadedTikTokMedia>,
    rate_limited: bool,
    errors: Vec<String>,
}

/// Baixa um lote de posts (vídeos e/ou fotos) numa única invocação do yt-dlp.
/// O `--print after_move` informa o timestamp, o id do post e o caminho de cada
/// arquivo produzido; movemos cada um para a pasta final com o prefixo de data.
/// Baixa um único vídeo (story capturado pelo Companion) na pasta `Stories/` do
/// perfil, com os cookies da conta e impersonation — mesmo caminho de download
/// da timeline, mas sem enumerar.
fn download_target_story_video<C>(
    request: &TikTokConnectorRequest,
    url: &str,
    stories_dir: &std::path::Path,
    is_cancelled: &C,
) -> Result<Option<DownloadedTikTokMedia>, String>
where
    C: Fn() -> bool,
{
    let output_template = format!(
        "{}/%(uploader)s_%(timestamp)s_%(id)s.%(ext)s",
        stories_dir.to_string_lossy().replace('\\', "/")
    );
    let mut command = Command::new(&request.yt_dlp_executable);
    command
        .arg("--ignore-errors")
        .arg("--no-warnings")
        .arg("--impersonate")
        .arg(YT_DLP_IMPERSONATE)
        .arg("--no-playlist")
        .arg("--no-simulate")
        .arg("--extractor-retries")
        .arg("3")
        .arg("--retries")
        .arg("5")
        .arg("--sleep-requests")
        .arg("1")
        .arg("--no-cookies-from-browser")
        .arg("--cookies")
        .arg(&request.cookie_file)
        .arg("--no-mtime")
        .arg("-o")
        .arg(&output_template)
        .arg("--print")
        .arg("after_move:%(timestamp)s\t%(id)s\t%(filepath)s");
    apply_user_agent(&mut command, request);
    command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let (stdout, _stderr) =
        run_capturing(command, STORIES_TIMEOUT_SECS, is_cancelled, "yt-dlp (story)")?;

    // Extrai o vídeo baixado (metadados via --print) para registrá-lo no ledger,
    // fazendo o story aparecer na seção Stories do perfil e contar no resumo.
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let timestamp = parts.next().unwrap_or("").trim();
        let post_id = parts.next().unwrap_or("").trim();
        let file_path = parts.next().unwrap_or("").trim();
        if post_id.is_empty() || file_path.is_empty() {
            continue;
        }
        let source_path = PathBuf::from(file_path);
        if !source_path.exists() {
            continue;
        }
        let captured_at_timestamp = timestamp.parse::<i64>().ok().filter(|value| *value > 0);
        let final_file_name = source_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();
        // Chaves prefixadas com `story_` (mesma convenção efêmera usada no ledger),
        // garantindo dedup estável e mantendo o story fora da recuperação de handle.
        let key = format!("story_{post_id}");
        return Ok(Some(DownloadedTikTokMedia {
            file_path: source_path,
            media_type: "video".to_string(),
            media_section: "stories".to_string(),
            provider_media_key: key.clone(),
            provider_post_key: key,
            captured_at_timestamp,
            final_file_name,
        }));
    }

    Ok(None)
}

fn download_batch<C>(
    request: &TikTokConnectorRequest,
    batch: &[EnumeratedPost],
    is_cancelled: &C,
) -> Result<BatchOutcome, String>
where
    C: Fn() -> bool,
{
    let download_dir = request.cache_root.join("dl");
    let _ = fs::remove_dir_all(&download_dir);
    fs::create_dir_all(&download_dir).map_err(|error| error.to_string())?;

    let mut command = Command::new(&request.yt_dlp_executable);
    command
        .arg("--ignore-errors")
        .arg("--no-warnings")
        // Impersonation (curl_cffi) é obrigatório: sem ele o extractor do TikTok
        // falha com "Unable to extract webpage video data" (anti-bot por TLS).
        .arg("--impersonate")
        .arg(YT_DLP_IMPERSONATE)
        .arg("--no-playlist")
        .arg("--no-simulate")
        // TikTok bloqueia requisições intermitentemente (anti-bot); retries e um
        // pequeno sleep entre requisições aumentam a taxa de sucesso.
        .arg("--extractor-retries")
        .arg("3")
        .arg("--retries")
        .arg("5")
        .arg("--sleep-requests")
        .arg("1")
        .arg("--no-cookies-from-browser")
        .arg("--cookies")
        .arg(&request.cookie_file);
    if request.use_parsed_video_date {
        command.arg("--mtime");
    } else {
        command.arg("--no-mtime");
    }
    apply_user_agent(&mut command, request);
    // Nome do arquivo: por padrão `<id>_<autonumber>`. Com `use_native_title`,
    // usa o título nativo do vídeo (SCrawler UseNativeTitle), opcionalmente com o
    // id anexado e/ou sem hashtags. Trunca por bytes (`.NB`) para não estourar o
    // limite de path; `%(title,id)s` cai no id quando o título fica vazio (ex.:
    // post só de hashtags com remoção ligada), evitando colisão de nomes.
    let output_template = if request.tokkit_naming {
        // Padrão 4K Tokkit: `<uploader>_<unix>_<id>.<ext>`. O nome já sai pronto
        // do yt-dlp; o finalize não acrescenta prefixo de data.
        "%(uploader)s_%(timestamp)s_%(id)s.%(ext)s"
    } else if request.use_native_title {
        if request.remove_tags_from_title {
            command
                .arg("--replace-in-metadata")
                .arg("title")
                .arg("#\\S+")
                .arg("")
                .arg("--replace-in-metadata")
                .arg("title")
                .arg("^\\s+|\\s+$")
                .arg("");
        }
        if request.add_video_id_to_title {
            "%(title).80B_%(id)s.%(ext)s"
        } else {
            "%(title,id).100B.%(ext)s"
        }
    } else {
        "%(id)s_%(autonumber)03d.%(ext)s"
    };
    command
        .arg("-P")
        .arg(&download_dir)
        .arg("-o")
        .arg(output_template)
        .arg("--print")
        .arg("after_move:%(timestamp)s\t%(id)s\t%(filepath)s");
    for post in batch {
        command.arg(&post.webpage_url);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let (stdout, stderr) = run_capturing(
        command,
        YT_DLP_DOWNLOAD_TIMEOUT_SECS,
        is_cancelled,
        "yt-dlp (download)",
    )?;
    let rate_limited = output_is_rate_limited(&stderr);

    let mut media = Vec::new();
    let mut errors = Vec::new();
    // Posts que o yt-dlp entregou como vídeo (ou imagem direta). Os demais do
    // lote — fotos de slideshow, que o yt-dlp não suporta — vão para o
    // gallery-dl. (Não dá para depender do áudio: sem ffmpeg no PATH, o yt-dlp
    // pula o áudio do slideshow silenciosamente com --ignore-errors.)
    let mut produced: HashSet<String> = HashSet::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let timestamp = parts.next().unwrap_or("").trim();
        let post_id = parts.next().unwrap_or("").trim();
        let file_path = parts.next().unwrap_or("").trim();
        if post_id.is_empty() || file_path.is_empty() {
            continue;
        }
        let source_path = PathBuf::from(file_path);
        if !source_path.exists() {
            continue;
        }
        let captured_at_timestamp = timestamp.parse::<i64>().ok().filter(|value| *value > 0);
        let extension = source_path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if AUDIO_EXTENSIONS.contains(&extension.as_str()) {
            // Áudio do slideshow (quando o yt-dlp consegue): descarta — as
            // imagens vêm do gallery-dl.
            let _ = fs::remove_file(&source_path);
            continue;
        }
        if VIDEO_EXTENSIONS.contains(&extension.as_str()) && !request.download_videos {
            let _ = fs::remove_file(&source_path);
            produced.insert(post_id.to_string());
            continue;
        }
        match finalize_media_file(request, &source_path, post_id, captured_at_timestamp) {
            Ok(item) => {
                produced.insert(post_id.to_string());
                media.push(item);
            }
            Err(_) => {
                let _ = fs::remove_file(&source_path);
            }
        }
    }
    let _ = fs::remove_dir_all(&download_dir);

    // Fotos via gallery-dl: todo post do lote que não virou vídeo é candidato.
    // Pulamos quando o lote bateu rate limit (os "sem vídeo" são provavelmente
    // vídeos throttled, não fotos; serão tentados de novo no próximo sync).
    if request.download_photos && !rate_limited {
        let mut seen_photo: HashSet<String> = HashSet::new();
        for post in batch {
            if produced.contains(&post.post_id) || !seen_photo.insert(post.post_id.clone()) {
                continue;
            }
            if is_cancelled() {
                return Err("source sync cancelled by user".to_string());
            }
            match download_post_photos(request, &post.post_id, is_cancelled) {
                Ok(mut images) => media.append(&mut images),
                Err(error) => {
                    if error.to_ascii_lowercase().contains("cancelled by user") {
                        return Err(error);
                    }
                    errors.push(format!("photo {} failed: {error}", post.post_id));
                }
            }
        }
    }

    Ok(BatchOutcome {
        media,
        rate_limited,
        errors,
    })
}

/// Baixa as imagens de um post de foto (slideshow). O yt-dlp não suporta o
/// download direto de `/photo/`, mas com `--impersonate --write-pages` ele
/// busca a página do post (passando pelo 403 que bloqueia o gallery-dl) e nós
/// extraímos as URLs das imagens do JSON de rehydration e baixamos via reqwest.
fn download_post_photos<C>(
    request: &TikTokConnectorRequest,
    post_id: &str,
    is_cancelled: &C,
) -> Result<Vec<DownloadedTikTokMedia>, String>
where
    C: Fn() -> bool,
{
    let photo_dir = request.cache_root.join(format!("photo-{post_id}"));
    let _ = fs::remove_dir_all(&photo_dir);
    fs::create_dir_all(&photo_dir).map_err(|error| error.to_string())?;
    let url = format!(
        "https://www.tiktok.com/@{}/video/{post_id}",
        request.handle.trim_start_matches('@')
    );

    // yt-dlp escreve a página (`*.dump`) no diretório de trabalho.
    let mut command = Command::new(&request.yt_dlp_executable);
    command
        .arg("--ignore-errors")
        .arg("--no-warnings")
        .arg("--impersonate")
        .arg(YT_DLP_IMPERSONATE)
        .arg("--skip-download")
        .arg("--write-pages")
        .arg("--no-cookies-from-browser")
        .arg("--cookies")
        .arg(&request.cookie_file)
        .arg(&url)
        .current_dir(&photo_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let (_stdout, stderr) =
        run_capturing(command, PHOTO_PAGE_TIMEOUT_SECS, is_cancelled, "yt-dlp (photo page)")?;

    let dump_path = fs::read_dir(&photo_dir)
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
        let _ = fs::remove_dir_all(&photo_dir);
        let detail = stderr
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("no page written")
            .trim();
        return Err(format!("page not fetched ({detail})"));
    };
    let html = fs::read_to_string(&dump_path).map_err(|error| error.to_string())?;
    let json = extract_rehydration_json(&html)
        .ok_or_else(|| "rehydration data not found in page".to_string())?;
    let item = json
        .get("__DEFAULT_SCOPE__")
        .and_then(|scope| scope.get("webapp.video-detail"))
        .and_then(|detail| detail.get("itemInfo"))
        .and_then(|info| info.get("itemStruct"));
    let captured_at_timestamp = item
        .and_then(|item| item.get("createTime"))
        .and_then(parse_unix_timestamp)
        .filter(|value| *value > 0);
    let image_urls: Vec<String> = item
        .and_then(|item| item.get("imagePost"))
        .and_then(|image_post| image_post.get("images"))
        .and_then(Value::as_array)
        .map(|images| {
            images
                .iter()
                .filter_map(|image| {
                    image
                        .get("imageURL")
                        .and_then(|node| node.get("urlList"))
                        .and_then(Value::as_array)
                        .and_then(|list| list.first())
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    let _ = fs::remove_dir_all(&photo_dir);

    if image_urls.is_empty() {
        // Não é um post de foto (provavelmente um vídeo que falhou no yt-dlp).
        return Ok(Vec::new());
    }

    let client = build_download_client(request)?;
    let count = image_urls.len();
    let pad = count.to_string().len().max(3);
    let mut media = Vec::new();
    for (index, image_url) in image_urls.iter().enumerate() {
        if is_cancelled() {
            return Err("source sync cancelled by user".to_string());
        }
        let final_file_name = if request.tokkit_naming {
            // Padrão 4K Tokkit: `<handle>_<unix>_<post_id>_index_<i>_<n>.jpeg`
            // (i 0-based, n total), sem prefixo de data.
            let ts = captured_at_timestamp
                .or_else(|| timestamp_from_tiktok_id(post_id))
                .unwrap_or(0);
            let handle = request.handle.trim().trim_start_matches('@');
            format!("{handle}_{ts}_{post_id}_index_{index}_{count}.jpeg")
        } else {
            let raw_name = if count > 1 {
                format!("{post_id}_{:0width$}.jpg", index + 1, width = pad)
            } else {
                format!("{post_id}.jpg")
            };
            timestamped_file_name(captured_at_timestamp, &raw_name)
        };
        let destination = request.profile_root.join(&final_file_name);
        if destination.exists() || request.existing_relative_paths.contains(&final_file_name) {
            continue;
        }
        connector_debug::append_current(
            "tiktok-http",
            "call",
            "GET photo",
            format!("GET {image_url}"),
        );
        let response = match client.get(image_url).send() {
            Ok(response) if response.status().is_success() => {
                connector_debug::append_current(
                    "tiktok-http",
                    "response",
                    "GET photo",
                    format!("HTTP {}", response.status()),
                );
                response
            }
            Ok(response) => {
                connector_debug::append_current(
                    "tiktok-http",
                    "error",
                    "GET photo",
                    format!("HTTP {}", response.status()),
                );
                continue;
            }
            Err(error) => {
                connector_debug::append_current(
                    "tiktok-http",
                    "error",
                    "GET photo",
                    error.to_string(),
                );
                continue;
            }
        };
        let Ok(bytes) = response.bytes() else { continue };
        if bytes.is_empty() {
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        if fs::write(&destination, &bytes).is_err() {
            continue;
        }
        media.push(DownloadedTikTokMedia {
            file_path: destination,
            media_type: "image".to_string(),
            media_section: "timeline".to_string(),
            provider_media_key: final_file_name.clone(),
            provider_post_key: post_id.to_string(),
            captured_at_timestamp,
            final_file_name,
        });
    }

    Ok(media)
}

/// Seção do perfil baixada pelo gallery-dl (extractors `/stories` e `/reposts`,
/// que — ao contrário do `/photo/` — não tomam 403).
#[derive(Clone, Copy)]
enum GalleryDlSection {
    Stories,
    Reposts,
}

impl GalleryDlSection {
    /// (sufixo da URL, subpasta no perfil, prefixo da chave de ledger, seção).
    fn parts(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            GalleryDlSection::Stories => ("stories", "Stories", "story", "stories"),
            GalleryDlSection::Reposts => ("reposts", "Reposts", "repost", "reposts"),
        }
    }
}

/// Baixa uma seção (Stories/Reposts) via gallery-dl. As mídias vão para uma
/// subpasta do perfil; o ledger usa a chave `<prefixo>_<id>` para deduplicar.
fn download_gallery_dl_section<C>(
    request: &TikTokConnectorRequest,
    section: GalleryDlSection,
    is_cancelled: &C,
) -> Result<Vec<DownloadedTikTokMedia>, String>
where
    C: Fn() -> bool,
{
    let (url_suffix, subfolder, key_prefix, media_section) = section.parts();
    let work_dir = request.cache_root.join(url_suffix);
    let _ = fs::remove_dir_all(&work_dir);
    fs::create_dir_all(&work_dir).map_err(|error| error.to_string())?;
    let url = format!(
        "https://www.tiktok.com/@{}/{url_suffix}",
        request.handle.trim_start_matches('@')
    );

    let mut command = Command::new(&request.gallery_dl_executable);
    command
        .arg("--no-mtime")
        .arg("-D")
        .arg(&work_dir)
        .arg("--cookies")
        .arg(&request.cookie_file)
        .arg("-o")
        .arg("cookies-update=false")
        .arg(&url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    run_capturing(command, STORIES_TIMEOUT_SECS, is_cancelled, "gallery-dl (section)")?;

    let stories_dir = work_dir;
    let target_dir = request.profile_root.join(subfolder);
    let mut media = Vec::new();
    let entries = fs::read_dir(&stories_dir).map_err(|error| error.to_string())?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let is_video = VIDEO_EXTENSIONS.contains(&extension.as_str());
        let is_image = IMAGE_EXTENSIONS.contains(&extension.as_str());
        if !is_video && !is_image {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        // gallery-dl nomeia "<id> TikTok video #<id>.mp4"; o id é o token inicial.
        let post_id: String = file_name.chars().take_while(|c| c.is_ascii_digit()).collect();
        if post_id.is_empty() {
            continue;
        }
        let post_key = format!("{key_prefix}_{post_id}");
        if request.ledger_post_keys.contains(&post_key) {
            continue;
        }
        let captured_at_timestamp = timestamp_from_tiktok_id(&post_id);
        let raw_name = format!("{post_id}.{extension}");
        let final_file_name = timestamped_file_name(captured_at_timestamp, &raw_name);
        if request.ledger_media_keys.contains(&final_file_name)
            || request.existing_relative_paths.contains(&final_file_name)
        {
            continue;
        }
        fs::create_dir_all(&target_dir).map_err(|error| error.to_string())?;
        let destination = target_dir.join(&final_file_name);
        if destination.exists() {
            continue;
        }
        if fs::rename(&path, &destination).is_err() {
            if fs::copy(&path, &destination).is_err() {
                continue;
            }
            let _ = fs::remove_file(&path);
        }
        media.push(DownloadedTikTokMedia {
            file_path: destination,
            media_type: if is_video { "video" } else { "image" }.to_string(),
            media_section: media_section.to_string(),
            provider_media_key: final_file_name.clone(),
            provider_post_key: post_key,
            captured_at_timestamp,
            final_file_name,
        });
    }

    let _ = fs::remove_dir_all(&stories_dir);
    Ok(media)
}

/// Os IDs do TikTok codificam o timestamp de criação nos bits altos
/// (`id >> 32` = unix seconds). Usado para datar Stories, cujo nome do
/// gallery-dl não traz a data.
fn timestamp_from_tiktok_id(post_id: &str) -> Option<i64> {
    let id = post_id.trim().parse::<u64>().ok()?;
    let seconds = (id >> 32) as i64;
    // Sanidade: TikTok existe desde ~2016; rejeita valores absurdos.
    if (1_400_000_000..4_000_000_000).contains(&seconds) {
        Some(seconds)
    } else {
        None
    }
}

/// Move um arquivo baixado para a pasta final com o prefixo de data, roteando
/// vídeos para a subpasta `Video` quando configurado.
fn finalize_media_file(
    request: &TikTokConnectorRequest,
    source_path: &Path,
    post_id: &str,
    captured_at_timestamp: Option<i64>,
) -> Result<DownloadedTikTokMedia, String> {
    let extension = source_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_video = VIDEO_EXTENSIONS.contains(&extension.as_str());
    let is_image = IMAGE_EXTENSIONS.contains(&extension.as_str());
    if !is_video && !is_image {
        // Áudio-only (mp3/m4a de slideshow sem imagens) ou formato inesperado:
        // descarta, o post não conta como baixado e será tentado de novo.
        let error = format!("unsupported media extension '{extension}'");
        connector_debug::append_current(
            "internal.tiktok",
            "error",
            "media.finalize",
            format!("source={}\npost_id={post_id}\nerror={error}", source_path.display()),
        );
        return Err(error);
    }
    let media_type = if is_video { "video" } else { "image" };

    let raw_name = source_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "invalid file name".to_string())?
        .to_string();
    // No modo 4K Tokkit o nome já vem completo do yt-dlp (`<handle>_<unix>_<id>`);
    // os demais modos recebem o prefixo de data.
    let final_file_name = if request.tokkit_naming {
        raw_name.clone()
    } else {
        timestamped_file_name(captured_at_timestamp, &raw_name)
    };

    let target_dir = if is_video && request.separate_video_folder {
        request.profile_root.join("Video")
    } else {
        request.profile_root.clone()
    };
    fs::create_dir_all(&target_dir).map_err(|error| error.to_string())?;
    let destination = target_dir.join(&final_file_name);
    if destination.exists() {
        connector_debug::append_current(
            "internal.tiktok",
            "system",
            "media.skip",
            format!(
                "post_id={post_id}\nreason=destination already exists\ndestination={}",
                destination.display()
            ),
        );
        return Err("destination already exists".to_string());
    }
    connector_debug::append_current(
        "internal.tiktok",
        "call",
        "media.finalize",
        format!(
            "post_id={post_id}\nmedia_type={media_type}\nsource={}\ndestination={}",
            source_path.display(),
            destination.display()
        ),
    );
    let transfer_mode = if fs::rename(source_path, &destination).is_err() {
        fs::copy(source_path, &destination).map_err(|error| error.to_string())?;
        let _ = fs::remove_file(source_path);
        "copy"
    } else {
        "rename"
    };
    let file_size = fs::metadata(&destination).map(|value| value.len()).unwrap_or(0);
    connector_debug::append_current(
        "internal.tiktok",
        "response",
        "media.finalize",
        format!(
            "post_id={post_id}\ntransfer={transfer_mode}\nbytes={file_size}\ndestination={}",
            destination.display()
        ),
    );

    Ok(DownloadedTikTokMedia {
        file_path: destination,
        media_type: media_type.to_string(),
        media_section: "timeline".to_string(),
        provider_media_key: final_file_name.clone(),
        provider_post_key: post_id.to_string(),
        captured_at_timestamp,
        final_file_name,
    })
}

fn apply_user_agent(command: &mut Command, request: &TikTokConnectorRequest) {
    if let Some(user_agent) = request
        .user_agent
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        command.arg("--user-agent").arg(user_agent);
    }
}

/// Roda um processo capturando stdout/stderr, com polling de cancel/timeout.
/// As saídas são drenadas em threads concorrentes para evitar deadlock quando o
/// yt-dlp produz muita saída.
fn run_capturing<C>(
    mut command: Command,
    timeout_secs: u64,
    is_cancelled: &C,
    label: &str,
) -> Result<(String, String), String>
where
    C: Fn() -> bool,
{
    let context = connector_debug::current_context();
    let command_line = std::iter::once(command.get_program().to_string_lossy().to_string())
        .chain(command.get_args().map(|arg| arg.to_string_lossy().to_string()))
        .collect::<Vec<_>>()
        .join(" ");
    connector_debug::append_with_context(
        context.clone(),
        label,
        "call",
        "process.spawn",
        command_line,
    );
    let mut child = command
        .spawn()
        .map_err(|error| {
            connector_debug::append_with_context(
                context.clone(),
                label,
                "error",
                "process.spawn",
                error.to_string(),
            );
            format!("Failed to start {label}: {error}")
        })?;
    connector_debug::append_with_context(
        context.clone(),
        label,
        "system",
        "process.started",
        format!("pid={}\ntimeout_seconds={timeout_secs}", child.id()),
    );

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
    let mut last_heartbeat = std::time::Instant::now();
    let mut cancelled = false;
    let mut timed_out = false;
    loop {
        if is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            cancelled = true;
            break;
        }
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(status) => {
                connector_debug::append_with_context(
                    context.clone(),
                    label,
                    "response",
                    "process.exit",
                    format!(
                        "exit_code={}",
                        status
                            .code()
                            .map_or_else(|| "terminated".to_string(), |code| code.to_string())
                    ),
                );
                break;
            }
            None => {
                if last_heartbeat.elapsed() >= Duration::from_secs(5) {
                    connector_debug::append_with_context(
                        context.clone(),
                        label,
                        "system",
                        "process.heartbeat",
                        format!(
                            "pid={}\nelapsed_seconds={}\nstate=running",
                            child.id(),
                            started.elapsed().as_secs()
                        ),
                    );
                    last_heartbeat = std::time::Instant::now();
                }
                if started.elapsed() > Duration::from_secs(timeout_secs) {
                    let _ = child.kill();
                    let _ = child.wait();
                    connector_debug::append_with_context(
                        context.clone(),
                        label,
                        "error",
                        "process.timeout",
                        format!("pid={}\ntimeout_seconds={timeout_secs}", child.id()),
                    );
                    timed_out = true;
                    break;
                }
                thread::sleep(Duration::from_millis(250));
            }
        }
    }

    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
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

fn timestamped_file_name(captured_at_timestamp: Option<i64>, raw_file_name: &str) -> String {
    match captured_at_timestamp.and_then(|value| Local.timestamp_opt(value, 0).single()) {
        Some(local_time) => {
            format!("{} {}", local_time.format("%Y-%m-%d %H.%M.%S"), raw_file_name)
        }
        None => raw_file_name.to_string(),
    }
}

/// Lê um timestamp unix do JSON, aceitando número ou string (o `createTime` do
/// TikTok vem como string).
fn parse_unix_timestamp(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamped_file_name_prefixes_local_date() {
        let named = timestamped_file_name(Some(1_700_000_000), "7300000000000000001_001.mp4");
        assert!(named.ends_with("7300000000000000001_001.mp4"));
        assert!(named.len() > "7300000000000000001_001.mp4".len());
    }

    #[test]
    fn timestamped_file_name_without_timestamp_is_raw() {
        assert_eq!(timestamped_file_name(None, "abc_001.jpg"), "abc_001.jpg");
    }

    #[test]
    fn rate_limit_detection_matches_common_markers() {
        assert!(output_is_rate_limited("HTTP Error 429: Too Many Requests"));
        assert!(output_is_rate_limited("rate-limit reached"));
        assert!(!output_is_rate_limited("downloaded 10 files"));
    }

    #[test]
    fn timestamp_from_tiktok_id_decodes_creation_time() {
        // Id real (2026); `id >> 32` ≈ unix seconds de 2026.
        let ts = timestamp_from_tiktok_id("7655134518160018695").expect("timestamp");
        assert!((1_700_000_000..1_900_000_000).contains(&ts), "got {ts}");
        assert_eq!(timestamp_from_tiktok_id("abc"), None);
        assert_eq!(timestamp_from_tiktok_id("123"), None); // muito pequeno
    }

    #[test]
    fn extract_rehydration_json_finds_avatar() {
        let body = r#"<html><head>
<script id="__UNIVERSAL_DATA_FOR_REHYDRATION__" type="application/json">{"__DEFAULT_SCOPE__":{"webapp.user-detail":{"userInfo":{"user":{"id":"123","avatarLarger":"https://p16.tiktok.com/avatar_larger.jpg"}}}}}</script>
</head></html>"#;
        let json = extract_rehydration_json(body).expect("json");
        let avatar = json
            .get("__DEFAULT_SCOPE__")
            .and_then(|s| s.get("webapp.user-detail"))
            .and_then(|d| d.get("userInfo"))
            .and_then(|i| i.get("user"))
            .and_then(|u| u.get("avatarLarger"))
            .and_then(|v| v.as_str());
        assert_eq!(avatar, Some("https://p16.tiktok.com/avatar_larger.jpg"));
    }

    #[test]
    fn extract_rehydration_json_handles_braces_in_strings() {
        let body = r#"<script id="__UNIVERSAL_DATA_FOR_REHYDRATION__" type="application/json">{"a":"x{y}z","b":{"c":1}}</script>"#;
        let json = extract_rehydration_json(body).expect("json");
        assert_eq!(json.get("a").and_then(|v| v.as_str()), Some("x{y}z"));
        assert_eq!(
            json.get("b").and_then(|b| b.get("c")).and_then(|v| v.as_i64()),
            Some(1)
        );
    }
}
