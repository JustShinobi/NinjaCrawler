//! Connector interno do X/Twitter.
//!
//! Espelha o contrato do módulo Twitter do SCrawler legado: o gallery-dl é
//! usado apenas como *parser* (`--no-download --no-skip --write-pages`) para
//! obter as páginas JSON da timeline; o download da mídia, o naming e o
//! catálogo no ledger ficam sob controle do NinjaCrawler (reqwest + SQLite).

use chrono::{DateTime, Local, TimeZone};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
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
const TWITTER_REQUEST_SLEEP_RANGE: &str = "1.5-3.5";

#[derive(Clone, Copy, Default)]
pub struct TwitterModelSelection {
    pub media: bool,
    pub profile: bool,
    pub search: bool,
    pub likes: bool,
}

#[derive(Clone)]
pub struct TwitterConnectorRequest {
    pub handle: String,
    pub gallery_dl_executable: PathBuf,
    /// Arquivo de cookies no formato Netscape já gravado pelo caller. O
    /// gallery-dl lê e (com cookies-update) regrava este arquivo.
    pub cookie_file: PathBuf,
    pub user_agent: Option<String>,
    pub profile_root: PathBuf,
    /// Diretório de trabalho para config + páginas temporárias do parser.
    pub cache_root: PathBuf,
    pub models: TwitterModelSelection,
    /// Cursor opaco do gallery-dl por modelo/timeline. Nunca deve ser
    /// compartilhado entre secoes.
    pub resume_cursors: HashMap<String, String>,
    /// Cutoff Unix do sync incremental do modelo `media`. `None` força full
    /// scan; backfills e modelos que podem trazer posts antigos não o usam.
    pub incremental_cutoff_timestamp: Option<i64>,
    pub ledger_post_keys: HashSet<String>,
    pub ledger_media_keys: HashSet<String>,
    pub existing_relative_paths: HashSet<String>,
    /// Id numérico estável do dono do perfil (`userIdHint`), quando já conhecido.
    /// Usado para recuperar o handle atual via `x.com/i/user/<id>` quando o
    /// handle salvo deixa de listar tweets (conta renomeada).
    pub user_id_hint: Option<String>,
    pub abort_on_limit: bool,
    pub download_already_parsed: bool,
    /// Segundos entre invocações do parser; `-1` desabilita (default SCrawler).
    pub sleep_timer_secs: i64,
    /// Sleep antes da primeira invocação; `-1` desabilita, `-2` usa o valor de
    /// `sleep_timer_secs` (default SCrawler).
    pub sleep_timer_before_first_secs: i64,
    pub download_images: bool,
    pub download_videos: bool,
    pub download_gifs: bool,
    /// Roteia vídeos para a subpasta `Video` (SeparateVideoFolder do SCrawler).
    pub separate_video_folder: bool,
    /// Subpasta (relativa ao profile_root) para os GIFs; vazio = junto da mídia.
    pub gifs_special_folder: String,
    /// Prefixo aplicado ao nome dos arquivos de GIF (default `GIF_`).
    pub gifs_prefix: String,
    /// Permite tweets de terceiros no modelo media (MediaModelAllowNonUserTweets).
    pub allow_non_user_tweets: bool,
    /// Descarta downloads byte-idênticos comparando o sha256 (UseMD5Comparison).
    pub use_md5_comparison: bool,
    /// Usa `-o search-endpoint=graphql` no modelo de search (UseNewEndPointSearch).
    pub search_use_graphql_endpoint: bool,
    /// Usa `-o search-endpoint=graphql` nos modelos de profile (UseNewEndPointProfiles).
    pub profile_use_graphql_endpoint: bool,
}

#[derive(Clone)]
pub struct ObservedTwitterPost {
    pub provider_post_key: String,
    pub media_section: String,
}

/// Media→post link observed in the fetched timeline, used to backfill the post
/// key on media that is already on disk (downloaded before the key was stored).
/// Captured for every observed tweet, including ones skipped from download.
#[derive(Clone)]
pub struct TwitterMediaPostLink {
    pub provider_media_key: String,
    pub provider_post_key: String,
    pub media_section: String,
    pub captured_at_timestamp: Option<i64>,
}

#[derive(Clone)]
pub struct DownloadedTwitterMedia {
    pub file_path: PathBuf,
    pub media_type: String,
    pub media_section: String,
    pub provider_media_key: String,
    pub provider_post_key: String,
    pub captured_at_timestamp: Option<i64>,
    pub final_file_name: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TwitterManifestSummary {
    pub parsed_page_count: u32,
    pub normalized_post_count: u32,
    pub discovered_asset_count: u32,
    pub queued_asset_count: u32,
    pub skipped_existing_post_count: u32,
    pub skipped_existing_asset_count: u32,
    pub skipped_disabled_asset_count: u32,
    pub skipped_duplicate_asset_count: u32,
    pub downloaded_asset_count: u32,
    pub completed_post_count: u32,
    pub incomplete_post_count: u32,
    pub attempted_model_count: u32,
    pub completed_model_count: u32,
    pub rate_limited: bool,
    pub resume_cursor_count: u32,
    pub downloaded_by_section: BTreeMap<String, u32>,
    pub skipped_disabled_by_type: BTreeMap<String, u32>,
    pub newest_post_timestamp: Option<i64>,
    pub incremental_scan: bool,
    pub incremental_cutoff_timestamp: Option<i64>,
    pub selection_signature: Option<String>,
    pub full_scan_at: Option<String>,
}

pub struct TwitterConnectorResult {
    pub observed_posts: Vec<ObservedTwitterPost>,
    pub downloaded_media: Vec<DownloadedTwitterMedia>,
    /// Novos media keys cujo conteúdo já existia em outro arquivo. São
    /// persistidos como aliases no ledger, mas não contam como downloads.
    pub deduplicated_media_aliases: Vec<DownloadedTwitterMedia>,
    /// Media→post links from the fetched timeline (for backfilling the post key
    /// on already-downloaded media). Includes posts skipped from download.
    pub media_post_links: Vec<TwitterMediaPostLink>,
    pub section_errors: Vec<String>,
    pub rate_limited: bool,
    /// O sync interrompeu modelos restantes por limite (AbortOnLimit).
    pub limit_aborted: bool,
    /// Cursores que ainda precisam ser retomados, indexados por media_section.
    pub resume_cursors: BTreeMap<String, String>,
    /// Modelos que chegaram ao fim e podem ter qualquer cursor anterior limpo.
    pub completed_sections: Vec<String>,
    /// Id numérico do usuário resolvido das páginas (rest_id), quando disponível.
    pub resolved_user_id: Option<String>,
    /// URL do avatar (profile_image_url_https) do dono do perfil, quando
    /// presente nas páginas. O caller baixa e persiste como ProfilePicture.
    pub resolved_avatar_url: Option<String>,
    /// Preenchido quando o `is_duplicate_user_id` apontou que o user id já
    /// pertence a outro perfil; nesse caso o download foi cancelado.
    pub duplicate_user_id: Option<String>,
    /// Handle (screen_name) atual descoberto quando a conta foi renomeada: o
    /// handle salvo parou de listar tweets, mas `x.com/i/user/<userIdHint>`
    /// resolveu para outro screen_name. O chamador atualiza o perfil.
    pub resolved_handle: Option<String>,
    pub manifest_summary: TwitterManifestSummary,
}

pub struct TwitterProgress {
    pub label: String,
    pub detail: String,
    pub downloaded_items: Option<u32>,
    pub progress_percent: Option<u32>,
    pub indeterminate: bool,
}

#[derive(Clone)]
struct ParsedTweetAsset {
    provider_media_key: String,
    media_type: String,
    file_url: String,
    file_name: String,
}

#[derive(Clone)]
struct ParsedTweet {
    post_key: String,
    author_screen_name: Option<String>,
    author_user_id: Option<String>,
    author_avatar_url: Option<String>,
    captured_at_timestamp: Option<i64>,
    assets: Vec<ParsedTweetAsset>,
}

struct ModelRun {
    media_section: &'static str,
    url: String,
    /// Argumentos extra do gallery-dl para este modelo (ex.: endpoint graphql
    /// do search).
    extra_args: Vec<String>,
}

fn twitter_section_label(section: &str) -> &'static str {
    match section {
        "media" => "profile posts with media",
        "timeline" => "full profile timeline",
        "search" => "search results",
        "likes" => "liked posts",
        _ => "Twitter media",
    }
}

fn twitter_incremental_post_filter(cutoff_timestamp: i64) -> String {
    format!("date.timestamp() >= {cutoff_timestamp} or abort()")
}

fn tweet_belongs_to_owner(tweet: &ParsedTweet, handle: &str, user_id_hint: Option<&str>) -> bool {
    if let Some(hint) = user_id_hint
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return tweet.author_user_id.as_deref().is_some_and(|id| id == hint);
    }

    tweet
        .author_screen_name
        .as_deref()
        .is_some_and(|name| name.eq_ignore_ascii_case(handle))
}

pub fn run_profile_sync<F, C, D>(
    request: &TwitterConnectorRequest,
    mut report_progress: F,
    is_cancelled: C,
    is_duplicate_user_id: D,
) -> Result<TwitterConnectorResult, String>
where
    F: FnMut(TwitterProgress),
    C: Fn() -> bool,
    D: Fn(&str) -> bool,
{
    let handle = request.handle.trim().trim_start_matches('@').to_string();
    if handle.is_empty() {
        return Err("Twitter handle is required.".to_string());
    }

    fs::create_dir_all(&request.cache_root).map_err(|error| error.to_string())?;
    fs::create_dir_all(&request.profile_root).map_err(|error| error.to_string())?;
    let config_path = write_gallery_dl_config(request)?;

    let graphql_args = |enabled: bool| -> Vec<String> {
        if enabled {
            vec!["-o".to_string(), "search-endpoint=graphql".to_string()]
        } else {
            Vec::new()
        }
    };

    let mut runs: Vec<ModelRun> = Vec::new();
    if request.models.media {
        runs.push(ModelRun {
            media_section: "media",
            url: format!("https://x.com/{handle}/media"),
            extra_args: graphql_args(request.profile_use_graphql_endpoint),
        });
    }
    if request.models.profile {
        runs.push(ModelRun {
            media_section: "timeline",
            url: format!("https://x.com/{handle}"),
            extra_args: graphql_args(request.profile_use_graphql_endpoint),
        });
    }
    if request.models.search {
        runs.push(ModelRun {
            media_section: "search",
            url: format!("https://x.com/search?q=from%3A{handle}+include%3Anativeretweets&f=live"),
            extra_args: graphql_args(request.search_use_graphql_endpoint),
        });
    }
    if request.models.likes {
        runs.push(ModelRun {
            media_section: "likes",
            url: format!("https://x.com/{handle}/likes"),
            extra_args: Vec::new(),
        });
    }
    if runs.is_empty() {
        return Err("No Twitter download model is enabled for this profile.".to_string());
    }

    let mut summary = TwitterManifestSummary::default();
    summary.incremental_scan = request.incremental_cutoff_timestamp.is_some();
    summary.incremental_cutoff_timestamp = request.incremental_cutoff_timestamp;
    let mut section_errors: Vec<String> = Vec::new();
    let mut rate_limited = false;
    let mut limit_aborted = false;
    let mut resume_cursors = BTreeMap::new();
    let mut completed_sections = Vec::new();
    let mut media_post_links: Vec<TwitterMediaPostLink> = Vec::new();
    let mut planned_downloads: Vec<DownloadPlanEntry> = Vec::new();
    let mut pending_posts: Vec<PendingTwitterPost> = Vec::new();
    let mut seen_post_keys: HashSet<String> = HashSet::new();
    let mut seen_media_keys: HashSet<String> = HashSet::new();
    let mut available_media_keys = request.ledger_media_keys.clone();
    let mut resolved_user_id: Option<String> = None;
    let mut resolved_avatar_url: Option<String> = None;
    let mut duplicate_user_id: Option<String> = None;
    let mut resolved_handle: Option<String> = None;
    let total_runs = runs.len();
    summary.attempted_model_count = total_runs as u32;

    for (run_index, run) in runs.iter().enumerate() {
        if is_cancelled() {
            return Err("source sync cancelled by user".to_string());
        }

        apply_sleep_timer(request, run_index, &is_cancelled);

        report_progress(TwitterProgress {
            label: format!("Parsing {}", twitter_section_label(run.media_section)),
            detail: format!(
                "gallery-dl is fetching {} ({}/{}).",
                twitter_section_label(run.media_section),
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

        let mut parser_args = run.extra_args.clone();
        if run.media_section == "media" {
            if let Some(cutoff) = request.incremental_cutoff_timestamp {
                parser_args.push("--post-filter".to_string());
                parser_args.push(twitter_incremental_post_filter(cutoff));
            }
        }
        if let Some(cursor) = request
            .resume_cursors
            .get(run.media_section)
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            parser_args.push("-o".to_string());
            parser_args.push(format!("cursor={cursor}"));
        }
        let parse_outcome = run_gallery_dl_parser(
            request,
            &config_path,
            &run.url,
            &parser_args,
            &pages_dir,
            &is_cancelled,
        );
        let page_output = match parse_outcome {
            Ok(output) => output,
            Err(error) => {
                // Cancelamento nunca é "erro de seção" a engolir — aborta o sync.
                if is_cancelled() || error.contains("cancelled by user") {
                    return Err(error);
                }
                section_errors.push(format!("{}: {}", run.media_section, error));
                continue;
            }
        };
        summary.completed_model_count += 1;
        if page_output.rate_limited {
            rate_limited = true;
            if let Some(cursor) = page_output.continuation_cursor {
                resume_cursors.insert(run.media_section.to_string(), cursor);
            }
        } else {
            completed_sections.push(run.media_section.to_string());
        }

        let tweets = parse_tweets_from_pages(&pages_dir, &mut summary)?;

        // Resolve o user id (rest_id) do dono do perfil a partir de um tweet
        // dele e, no primeiro sync, deixa o caller decidir se é duplicado —
        // cancelando antes de baixar qualquer mídia.
        if resolved_user_id.is_none() {
            let owner_tweet = tweets.iter().find(|tweet| {
                tweet_belongs_to_owner(tweet, &handle, request.user_id_hint.as_deref())
            });
            resolved_user_id = owner_tweet.and_then(|tweet| tweet.author_user_id.clone());
            if let Some(current_handle) = owner_tweet
                .and_then(|tweet| tweet.author_screen_name.as_deref())
                .filter(|name| !name.eq_ignore_ascii_case(&handle))
            {
                resolved_handle = Some(current_handle.to_string());
            }
            if let Some(uid) = resolved_user_id.as_deref() {
                if is_duplicate_user_id(uid) {
                    duplicate_user_id = Some(uid.to_string());
                }
            }
        }
        // Avatar: prioriza um tweet do próprio dono do perfil; só guarda o
        // primeiro que aparecer para evitar pegar avatar de terceiros (likes).
        if resolved_avatar_url.is_none() {
            resolved_avatar_url = tweets
                .iter()
                .find(|tweet| {
                    tweet_belongs_to_owner(tweet, &handle, request.user_id_hint.as_deref())
                })
                .and_then(|tweet| tweet.author_avatar_url.clone());
        }
        if duplicate_user_id.is_some() {
            let _ = fs::remove_dir_all(&pages_dir);
            break;
        }
        if resolved_handle.is_some() {
            // A identidade estável confirmou que esta URL pertence agora a
            // outro handle. Não planeje downloads nesta passagem: o caller
            // atualiza o perfil e repete uma única vez com o handle canônico,
            // evitando bytes sem ledger ou colisões entre as duas passagens.
            let _ = fs::remove_dir_all(&pages_dir);
            break;
        }

        for tweet in tweets {
            // Modelo media restrito a tweets do próprio usuário, salvo se
            // allow_non_user_tweets (espelho do MediaModelAllowNonUserTweets).
            // Likes inclui posts de terceiros por natureza.
            if run.media_section == "media" && !request.allow_non_user_tweets {
                if !tweet_belongs_to_owner(&tweet, &handle, request.user_id_hint.as_deref()) {
                    continue;
                }
            }

            if !seen_post_keys.insert(tweet.post_key.clone()) {
                continue;
            }
            summary.normalized_post_count += 1;
            if let Some(timestamp) = tweet.captured_at_timestamp {
                summary.newest_post_timestamp = Some(
                    summary
                        .newest_post_timestamp
                        .map_or(timestamp, |current| current.max(timestamp)),
                );
            }
            // Vínculo media→post de TODO tweet visto (inclusive os pulados abaixo):
            // permite preencher o post key na mídia já no disco, baixada antes de
            // o key ser persistido. Usa só os dados já buscados (sem download).
            for asset in &tweet.assets {
                media_post_links.push(TwitterMediaPostLink {
                    provider_media_key: asset.provider_media_key.clone(),
                    provider_post_key: tweet.post_key.clone(),
                    media_section: run.media_section.to_string(),
                    captured_at_timestamp: tweet.captured_at_timestamp,
                });
            }
            // O post ledger sozinho não prova que todos os assets chegaram ao
            // disco: versões anteriores registravam o post mesmo após falha de
            // download. Reavalia os assets e deixa o media ledger decidir o que
            // já existe, recuperando também backlog histórico sem migração.
            let was_known_post = request.ledger_post_keys.contains(&tweet.post_key);
            let mut asset_keys = Vec::with_capacity(tweet.assets.len());
            let mut had_missing_assets = false;
            for asset in &tweet.assets {
                summary.discovered_asset_count += 1;
                let allowed = match asset.media_type.as_str() {
                    "image" => request.download_images,
                    "gif" => request.download_gifs,
                    _ => request.download_videos,
                };
                if !allowed {
                    summary.skipped_disabled_asset_count += 1;
                    *summary
                        .skipped_disabled_by_type
                        .entry(asset.media_type.clone())
                        .or_insert(0) += 1;
                    continue;
                }
                // Somente assets habilitados participam da completude atual do
                // post. O connector reavalia todos os assets em cada sync, logo
                // habilitar este tipo no futuro ainda agenda o download.
                asset_keys.push(asset.provider_media_key.clone());
                if !available_media_keys.contains(&asset.provider_media_key)
                    && !asset_exists_on_disk(request, asset)
                {
                    had_missing_assets = true;
                }
                if !seen_media_keys.insert(asset.provider_media_key.clone()) {
                    continue;
                }
                if request
                    .ledger_media_keys
                    .contains(&asset.provider_media_key)
                    || asset_exists_on_disk(request, asset)
                {
                    summary.skipped_existing_asset_count += 1;
                    available_media_keys.insert(asset.provider_media_key.clone());
                    continue;
                }
                summary.queued_asset_count += 1;
                planned_downloads.push(DownloadPlanEntry {
                    asset: asset.clone(),
                    post_key: tweet.post_key.clone(),
                    media_section: run.media_section.to_string(),
                    captured_at_timestamp: tweet.captured_at_timestamp,
                });
            }
            pending_posts.push(PendingTwitterPost {
                provider_post_key: tweet.post_key,
                media_section: run.media_section.to_string(),
                asset_keys,
                was_known_post,
                had_missing_assets,
            });
        }

        let _ = fs::remove_dir_all(&pages_dir);

        if page_output.rate_limited && request.abort_on_limit {
            limit_aborted = run_index + 1 < total_runs;
            break;
        }
    }

    if summary.completed_model_count == 0 {
        let _ = fs::remove_file(&config_path);
        return Err(format!(
            "All enabled Twitter download models failed: {}",
            section_errors.join(" | ")
        ));
    }

    let mut downloaded_media: Vec<DownloadedTwitterMedia> = Vec::new();
    let mut deduplicated_media_aliases: Vec<DownloadedTwitterMedia> = Vec::new();
    // Duplicado de outro perfil: cancela o download (o caller remove o perfil).
    let should_download =
        duplicate_user_id.is_none() && (!rate_limited || request.download_already_parsed);
    if should_download {
        let client = build_download_client(request)?;
        // Hashes do conteúdo já presente no disco ANTES desta rodada, para o
        // dedupe por conteúdo (MD5 comparison) descartar baixados idênticos.
        let mut known_hashes = if request.use_md5_comparison {
            seed_existing_hashes(&request.profile_root)
        } else {
            HashMap::new()
        };
        let total = planned_downloads.len();
        for (index, entry) in planned_downloads.iter().enumerate() {
            if is_cancelled() {
                return Err("source sync cancelled by user".to_string());
            }
            report_progress(TwitterProgress {
                label: format!(
                    "Downloading {}",
                    twitter_section_label(&entry.media_section)
                ),
                detail: format!(
                    "{}: {} ({}/{})",
                    twitter_section_label(&entry.media_section),
                    entry.asset.file_name,
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
                                // Conteúdo idêntico já presente: remove os bytes
                                // repetidos, mas persiste este novo media key como
                                // alias do arquivo canônico para não baixá-lo de
                                // novo em toda sincronização.
                                let _ = fs::remove_file(&media.file_path);
                                summary.skipped_duplicate_asset_count += 1;
                                available_media_keys.insert(entry.asset.provider_media_key.clone());
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
                    available_media_keys.insert(entry.asset.provider_media_key.clone());
                    downloaded_media.push(media);
                }
                Err(error) => {
                    section_errors.push(format!(
                        "{}: download failed for '{}': {}",
                        entry.media_section, entry.asset.file_name, error
                    ));
                }
            }
        }
    } else {
        section_errors.push(
            "Twitter rate limit reached and 'download already parsed' is disabled; parsed media was not downloaded."
                .to_string(),
        );
    }

    // Recuperação de handle: nenhum tweet veio das páginas (sem rate limit nem
    // duplicata) — a conta pode ter sido renomeada. Resolve o handle atual via
    // `x.com/i/user/<userIdHint>`, cujos tweets trazem o screen_name corrente.
    if pending_posts.is_empty()
        && downloaded_media.is_empty()
        && !rate_limited
        && duplicate_user_id.is_none()
    {
        if resolved_handle.is_none() {
            if let Some(hint) = request
                .user_id_hint
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if let Some(current) =
                    resolve_handle_via_user_id(request, &config_path, hint, &is_cancelled)
                {
                    if !current.eq_ignore_ascii_case(&handle) {
                        resolved_handle = Some(current);
                    }
                }
            }
        }
    }
    let _ = fs::remove_file(&config_path);

    let observed_posts =
        completed_observed_posts(pending_posts, &available_media_keys, &mut summary);
    summary.rate_limited = rate_limited;
    summary.resume_cursor_count = resume_cursors.len() as u32;

    report_progress(TwitterProgress {
        label: "Finishing".to_string(),
        detail: format!("Downloaded {} media files.", downloaded_media.len()),
        downloaded_items: Some(downloaded_media.len() as u32),
        progress_percent: Some(100),
        indeterminate: false,
    });

    Ok(TwitterConnectorResult {
        observed_posts,
        downloaded_media,
        deduplicated_media_aliases,
        media_post_links,
        section_errors,
        rate_limited,
        limit_aborted,
        resume_cursors,
        completed_sections,
        resolved_user_id,
        resolved_avatar_url,
        duplicate_user_id,
        resolved_handle,
        manifest_summary: summary,
    })
}

/// Resolve o screen_name atual de um usuário a partir do seu id numérico,
/// usando o gallery-dl em `x.com/i/user/<id>` (o extractor resolve o id para a
/// timeline e cada tweet traz `author[name]` = screen_name atual). Confirma a
/// identidade pelo `author_user_id` antes de aceitar. Best-effort: `None` em
/// qualquer falha (inclusive conta sem tweets).
fn resolve_handle_via_user_id<C>(
    request: &TwitterConnectorRequest,
    config_path: &Path,
    user_id: &str,
    is_cancelled: &C,
) -> Option<String>
where
    C: Fn() -> bool,
{
    if is_cancelled() {
        return None;
    }
    let url = format!("https://x.com/i/user/{user_id}");
    let pages_dir = request.cache_root.join("pages-handle-recovery");
    let _ = fs::remove_dir_all(&pages_dir);
    fs::create_dir_all(&pages_dir).ok()?;

    let parsed = run_gallery_dl_parser(request, config_path, &url, &[], &pages_dir, is_cancelled);
    let resolved = parsed.ok().and_then(|_| {
        let mut throwaway = TwitterManifestSummary::default();
        let tweets = parse_tweets_from_pages(&pages_dir, &mut throwaway).ok()?;
        // Prioriza o tweet cujo author bate com o id buscado.
        tweets
            .iter()
            .find(|tweet| {
                tweet
                    .author_user_id
                    .as_deref()
                    .map(|id| id == user_id)
                    .unwrap_or(false)
            })
            .or_else(|| tweets.first())
            .and_then(|tweet| tweet.author_screen_name.clone())
            .map(|name| name.trim().trim_start_matches('@').to_string())
            .filter(|name| !name.is_empty())
    });
    let _ = fs::remove_dir_all(&pages_dir);
    resolved
}

struct DownloadPlanEntry {
    asset: ParsedTweetAsset,
    post_key: String,
    media_section: String,
    captured_at_timestamp: Option<i64>,
}

#[derive(Clone)]
struct PendingTwitterPost {
    provider_post_key: String,
    media_section: String,
    asset_keys: Vec<String>,
    was_known_post: bool,
    had_missing_assets: bool,
}

fn completed_observed_posts(
    pending_posts: Vec<PendingTwitterPost>,
    available_media_keys: &HashSet<String>,
    summary: &mut TwitterManifestSummary,
) -> Vec<ObservedTwitterPost> {
    pending_posts
        .into_iter()
        .filter_map(|post| {
            let complete = post
                .asset_keys
                .iter()
                .all(|key| available_media_keys.contains(key));
            if complete {
                summary.completed_post_count += 1;
                if post.was_known_post && !post.had_missing_assets {
                    summary.skipped_existing_post_count += 1;
                }
                Some(ObservedTwitterPost {
                    provider_post_key: post.provider_post_key,
                    media_section: post.media_section,
                })
            } else {
                summary.incomplete_post_count += 1;
                None
            }
        })
        .collect()
}

/// Dorme em passos curtos, abortando assim que o cancelamento é solicitado —
/// evita ficar preso no sleep timer (potencialmente longo) entre modelos.
fn interruptible_sleep(total: Duration, is_cancelled: &dyn Fn() -> bool) {
    const STEP: Duration = Duration::from_millis(200);
    let mut remaining = total;
    while !remaining.is_zero() {
        if is_cancelled() {
            return;
        }
        let chunk = STEP.min(remaining);
        thread::sleep(chunk);
        remaining -= chunk;
    }
}

fn apply_sleep_timer(
    request: &TwitterConnectorRequest,
    run_index: usize,
    is_cancelled: &dyn Fn() -> bool,
) {
    let seconds = if run_index == 0 {
        match request.sleep_timer_before_first_secs {
            -2 => request.sleep_timer_secs,
            value => value,
        }
    } else {
        request.sleep_timer_secs
    };
    if seconds > 0 {
        interruptible_sleep(Duration::from_secs(seconds as u64), is_cancelled);
    }
}

fn write_gallery_dl_config(request: &TwitterConnectorRequest) -> Result<PathBuf, String> {
    let config_path = request.cache_root.join("twitter-gdl-config.json");
    let mut extractor = serde_json::Map::new();
    extractor.insert(
        "cookies".to_string(),
        Value::String(request.cookie_file.display().to_string()),
    );
    // Espelho do CookiesUpdate do SCrawler: o gallery-dl regrava o arquivo de
    // cookies quando o Twitter os rotaciona durante a sessão.
    extractor.insert("cookies-update".to_string(), Value::Bool(true));
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

struct ParserRunOutput {
    rate_limited: bool,
    continuation_cursor: Option<String>,
}

fn parse_continuation_cursor(output: &str) -> Option<String> {
    for line in output.lines().rev() {
        if let Some(rest) = line.split("cursor=").nth(1) {
            let cursor = rest
                .trim_start()
                .trim_start_matches(|character| matches!(character, '\'' | '"'))
                .split(|character: char| {
                    character.is_whitespace() || matches!(character, '\'' | '"')
                })
                .next()
                .unwrap_or_default()
                .trim_end_matches([')', '.']);
            if !cursor.is_empty() && !cursor.eq_ignore_ascii_case("none") {
                return Some(cursor.to_string());
            }
        }
        if let Some((_, rest)) = line.rsplit_once("Cursor:") {
            let cursor = rest.trim();
            if cursor.eq_ignore_ascii_case("none") {
                // O ultimo marcador do extractor e autoritativo. Continuar a
                // busca ressuscitaria um cursor de uma pagina anterior mesmo
                // depois de a timeline ter chegado normalmente ao fim.
                return None;
            }
            if !cursor.is_empty() {
                return Some(cursor.to_string());
            }
        }
    }
    None
}

fn output_indicates_rate_limit(output: &str) -> bool {
    output.lines().any(|line| {
        let line = line.to_ascii_lowercase();
        // Nunca procure apenas por `429`: IDs de tweets, timestamps e nomes de
        // arquivos frequentemente contem essa sequencia. Os formatos abaixo
        // exigem um status/mensagem HTTP ou uma mensagem explicita do extractor.
        line.contains("\" 429 ")
            || line.contains("429 too many requests")
            || line.contains("http error 429")
            || line.contains("http status 429")
            || line.contains("status code 429")
            || line.contains("status: 429")
            || line.contains("rate limit")
            || line.contains("rate-limit")
    })
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

fn run_gallery_dl_parser<C>(
    request: &TwitterConnectorRequest,
    config_path: &Path,
    url: &str,
    extra_args: &[String],
    pages_dir: &Path,
    is_cancelled: &C,
) -> Result<ParserRunOutput, String>
where
    C: Fn() -> bool,
{
    // O gallery-dl em modo --verbose gera muita saída. Se mantivéssemos os
    // pipes sem drená-los enquanto aguardamos o término, o buffer do SO encheria
    // e o processo travaria escrevendo (deadlock). Redirecionamos stdout/stderr
    // para arquivos no cache, lidos depois, mantendo o polling de cancel/timeout.
    let stdout_log = request.cache_root.join("gdl-stdout.log");
    let stderr_log = request.cache_root.join("gdl-stderr.log");
    let stdout_file = fs::File::create(&stdout_log).map_err(|error| error.to_string())?;
    let stderr_file = fs::File::create(&stderr_log).map_err(|error| error.to_string())?;

    let mut command = Command::new(&request.gallery_dl_executable);
    command
        .arg("--verbose")
        .arg("--no-download")
        .arg("--no-skip")
        // O NinjaCrawler controla espera e retomada. O extractor deve devolver
        // o controle assim que receber 429, preservando o cursor emitido.
        .arg("-o")
        .arg("ratelimit=abort")
        // Espalha as requisicoes dentro de cada pagina para reduzir rajadas.
        .arg("--sleep-request")
        .arg(TWITTER_REQUEST_SLEEP_RANGE)
        .arg("--config")
        .arg(config_path)
        .arg("--write-pages")
        .args(extra_args)
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
    let stdout = fs::read_to_string(&stdout_log).unwrap_or_default();
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
    let combined = format!("{stdout}\n{stderr}");
    let rate_limited = output_indicates_rate_limit(&combined);

    let produced_pages = fs::read_dir(pages_dir)
        .map(|entries| entries.flatten().next().is_some())
        .unwrap_or(false);
    if !status.success() && !produced_pages && !rate_limited {
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

    let continuation_cursor = rate_limited
        .then(|| parse_continuation_cursor(&combined))
        .flatten();

    Ok(ParserRunOutput {
        rate_limited,
        continuation_cursor,
    })
}

fn parse_tweets_from_pages(
    pages_dir: &Path,
    summary: &mut TwitterManifestSummary,
) -> Result<Vec<ParsedTweet>, String> {
    let mut tweets: Vec<ParsedTweet> = Vec::new();
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
        collect_tweets(&value, &mut tweets);
    }
    Ok(tweets)
}

/// Travessia recursiva resiliente ao formato GraphQL do Twitter: qualquer
/// objeto com `legacy.id_str` + mídia em `extended_entities` é tratado como
/// tweet, independentemente da instrução/módulo de timeline que o envolve.
fn collect_tweets(value: &Value, tweets: &mut Vec<ParsedTweet>) {
    match value {
        Value::Object(map) => {
            if let Some(tweet) = extract_tweet_from_object(map) {
                tweets.push(tweet);
            }
            for child in map.values() {
                collect_tweets(child, tweets);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_tweets(child, tweets);
            }
        }
        _ => {}
    }
}

fn extract_tweet_from_object(map: &serde_json::Map<String, Value>) -> Option<ParsedTweet> {
    let legacy = map.get("legacy")?.as_object()?;
    let post_key = legacy.get("id_str")?.as_str()?.to_string();
    let media_entries = legacy
        .get("extended_entities")
        .and_then(|value| value.get("media"))
        .and_then(Value::as_array)?;

    let author_result = map
        .get("core")
        .and_then(|core| core.get("user_results"))
        .and_then(|results| results.get("result"));
    let author_screen_name = author_result
        .and_then(|result| {
            result
                .get("legacy")
                .and_then(|user_legacy| user_legacy.get("screen_name"))
                .or_else(|| result.get("core").and_then(|c| c.get("screen_name")))
        })
        .and_then(Value::as_str)
        .map(str::to_string);
    // rest_id é o id numérico estável do usuário (igual ao UserID do SCrawler).
    let author_user_id = author_result
        .and_then(|result| result.get("rest_id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    // Avatar do usuário: `legacy.profile_image_url_https` (API clássica) ou
    // `avatar.image_url` (API nova do X). O caller faz o upgrade para o
    // tamanho original e baixa.
    let author_avatar_url = author_result
        .and_then(|result| {
            result
                .get("legacy")
                .and_then(|user_legacy| user_legacy.get("profile_image_url_https"))
                .or_else(|| {
                    result
                        .get("avatar")
                        .and_then(|avatar| avatar.get("image_url"))
                })
        })
        .and_then(Value::as_str)
        .map(str::to_string);

    let captured_at_timestamp = legacy
        .get("created_at")
        .and_then(Value::as_str)
        .and_then(parse_twitter_timestamp);

    let mut assets = Vec::new();
    for media in media_entries {
        let Some(asset) = extract_asset_from_media(media) else {
            continue;
        };
        assets.push(asset);
    }
    if assets.is_empty() {
        return None;
    }

    Some(ParsedTweet {
        post_key,
        author_screen_name,
        author_user_id,
        author_avatar_url,
        captured_at_timestamp,
        assets,
    })
}

fn extract_asset_from_media(media: &Value) -> Option<ParsedTweetAsset> {
    let media_type = media.get("type").and_then(Value::as_str)?;
    let media_key = media
        .get("id_str")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            media
                .get("media_key")
                .and_then(Value::as_str)
                .map(str::to_string)
        })?;

    match media_type {
        "photo" => {
            let base_url = media.get("media_url_https").and_then(Value::as_str)?;
            let file_name = url_file_name(base_url)?;
            Some(ParsedTweetAsset {
                provider_media_key: media_key,
                media_type: "image".to_string(),
                file_url: format!("{base_url}?name=orig"),
                file_name,
            })
        }
        "video" | "animated_gif" => {
            let variants = media
                .get("video_info")
                .and_then(|info| info.get("variants"))
                .and_then(Value::as_array)?;
            let best = variants
                .iter()
                .filter(|variant| {
                    variant
                        .get("content_type")
                        .and_then(Value::as_str)
                        .is_some_and(|value| value.eq_ignore_ascii_case("video/mp4"))
                })
                .max_by_key(|variant| {
                    variant.get("bitrate").and_then(Value::as_i64).unwrap_or(0)
                })?;
            let url = best.get("url").and_then(Value::as_str)?;
            let file_name = url_file_name(url)?;
            Some(ParsedTweetAsset {
                provider_media_key: media_key,
                media_type: if media_type == "animated_gif" {
                    "gif".to_string()
                } else {
                    "video".to_string()
                },
                file_url: url.to_string(),
                file_name,
            })
        }
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

/// Stable disk identity for Twitter CDN file names. It intentionally ignores
/// the NinjaCrawler date prefix, the legacy `GIF_` prefix, casing and extension
/// so SCrawler layouts such as `Video/GIF_<key>.mp4` match current downloads.
pub fn twitter_disk_asset_key(file_name: &str) -> Option<String> {
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
    let mut normalized = without_date.trim().to_ascii_lowercase();
    if let Some(stripped) = normalized.strip_prefix("gif_") {
        normalized = stripped.to_string();
    }
    (!normalized.is_empty()).then_some(normalized)
}

fn asset_exists_on_disk(request: &TwitterConnectorRequest, asset: &ParsedTweetAsset) -> bool {
    twitter_disk_asset_key(&asset.file_name)
        .is_some_and(|key| request.existing_relative_paths.contains(&key))
}

/// Formato legado do Twitter: "Wed Oct 10 20:19:24 +0000 2018".
fn parse_twitter_timestamp(raw: &str) -> Option<i64> {
    DateTime::parse_from_str(raw, "%a %b %d %H:%M:%S %z %Y")
        .ok()
        .map(|value| value.timestamp())
}

fn build_download_client(request: &TwitterConnectorRequest) -> Result<Client, String> {
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
    request: &TwitterConnectorRequest,
    entry: &DownloadPlanEntry,
) -> Result<DownloadedTwitterMedia, String> {
    let is_gif = entry.asset.media_type == "gif";
    let raw_name = if is_gif && !request.gifs_prefix.is_empty() {
        format!("{}{}", request.gifs_prefix, entry.asset.file_name)
    } else {
        entry.asset.file_name.clone()
    };
    let final_file_name = timestamped_file_name(entry.captured_at_timestamp, &raw_name);

    // Estrutura de pastas espelhando o SCrawler: GIFs na pasta especial (se
    // configurada), vídeos na subpasta `Video` (se separate_video_folder),
    // imagens e demais na raiz do perfil.
    let target_dir = if is_gif && !request.gifs_special_folder.trim().is_empty() {
        request
            .profile_root
            .join(request.gifs_special_folder.trim())
    } else if entry.asset.media_type == "video" && request.separate_video_folder {
        request.profile_root.join("Video")
    } else {
        request.profile_root.clone()
    };
    let destination = target_dir.join(&final_file_name);

    connector_debug::append_current(
        "twitter-http",
        "call",
        "GET media",
        format!("GET {}", entry.asset.file_url),
    );
    let response = client.get(&entry.asset.file_url).send().map_err(|error| {
        connector_debug::append_current("twitter-http", "error", "GET media", error.to_string());
        error.to_string()
    })?;
    connector_debug::append_current(
        "twitter-http",
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

    Ok(DownloadedTwitterMedia {
        file_path: destination,
        media_type: entry.asset.media_type.clone(),
        media_section: entry.media_section.clone(),
        provider_media_key: entry.asset.provider_media_key.clone(),
        provider_post_key: entry.post_key.clone(),
        captured_at_timestamp: entry.captured_at_timestamp,
        final_file_name,
    })
}

fn write_download_atomically(destination: &Path, bytes: &[u8]) -> Result<(), String> {
    atomic_file::write_bytes_replacing_empty(destination, bytes)
}

/// Prefixa o nome do arquivo com a data/hora local do tweet, no mesmo formato
/// do Instagram (`YYYY-MM-DD HH.MM.SS `), para ordenação cronológica no disco.
/// Sem timestamp, mantém o nome cru.
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

/// Calcula o sha256 de todos os arquivos já presentes na pasta do perfil, para
/// o dedupe por conteúdo (MD5 comparison) descartar baixados idênticos.
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

    fn tweet_page_json() -> Value {
        serde_json::json!({
            "data": {
                "user": {
                    "result": {
                        "timeline_v2": {
                            "timeline": {
                                "instructions": [{
                                    "type": "TimelineAddEntries",
                                    "entries": [{
                                        "content": {
                                            "itemContent": {
                                                "tweet_results": {
                                                    "result": {
                                                        "rest_id": "1700000000000000001",
                                                        "core": {
                                                            "user_results": {
                                                                "result": {
                                                                    "rest_id": "1513311701554372616",
                                                                    "legacy": {
                                                                        "screen_name": "testuser",
                                                                        "profile_image_url_https": "https://pbs.twimg.com/profile_images/123/avatar_normal.jpg"
                                                                    }
                                                                }
                                                            }
                                                        },
                                                        "legacy": {
                                                            "id_str": "1700000000000000001",
                                                            "created_at": "Wed Oct 10 20:19:24 +0000 2018",
                                                            "extended_entities": {
                                                                "media": [
                                                                    {
                                                                        "type": "photo",
                                                                        "id_str": "9000000000000000001",
                                                                        "media_url_https": "https://pbs.twimg.com/media/AbCdEf123.jpg"
                                                                    },
                                                                    {
                                                                        "type": "video",
                                                                        "id_str": "9000000000000000002",
                                                                        "video_info": {
                                                                            "variants": [
                                                                                {"content_type": "application/x-mpegURL", "url": "https://video.twimg.com/pl/playlist.m3u8"},
                                                                                {"content_type": "video/mp4", "bitrate": 832000, "url": "https://video.twimg.com/vid/640x360/low.mp4"},
                                                                                {"content_type": "video/mp4", "bitrate": 2176000, "url": "https://video.twimg.com/vid/1280x720/best.mp4?tag=12"}
                                                                            ]
                                                                        }
                                                                    }
                                                                ]
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }]
                                }]
                            }
                        }
                    }
                }
            }
        })
    }

    #[test]
    fn collect_tweets_extracts_posts_and_assets_from_graphql_page() {
        let mut tweets = Vec::new();
        collect_tweets(&tweet_page_json(), &mut tweets);

        assert_eq!(tweets.len(), 1);
        let tweet = &tweets[0];
        assert_eq!(tweet.post_key, "1700000000000000001");
        assert_eq!(tweet.author_screen_name.as_deref(), Some("testuser"));
        assert_eq!(tweet.author_user_id.as_deref(), Some("1513311701554372616"));
        assert_eq!(
            tweet.author_avatar_url.as_deref(),
            Some("https://pbs.twimg.com/profile_images/123/avatar_normal.jpg")
        );
        assert!(tweet.captured_at_timestamp.is_some());
        assert_eq!(tweet.assets.len(), 2);

        let photo = &tweet.assets[0];
        assert_eq!(photo.media_type, "image");
        assert_eq!(photo.file_name, "AbCdEf123.jpg");
        assert_eq!(
            photo.file_url,
            "https://pbs.twimg.com/media/AbCdEf123.jpg?name=orig"
        );

        // O vídeo escolhe a variant mp4 de maior bitrate e ignora a m3u8.
        let video = &tweet.assets[1];
        assert_eq!(video.media_type, "video");
        assert_eq!(video.file_name, "best.mp4");
        assert_eq!(
            video.file_url,
            "https://video.twimg.com/vid/1280x720/best.mp4?tag=12"
        );
    }

    #[test]
    fn parse_twitter_timestamp_handles_legacy_format() {
        let timestamp = parse_twitter_timestamp("Wed Oct 10 20:19:24 +0000 2018");
        assert_eq!(timestamp, Some(1539202764));
    }

    #[test]
    fn timestamped_file_name_prepends_local_datetime() {
        // 2018-10-10 20:19:24 UTC -> prefixo no horário local; valida só o formato.
        let named = timestamped_file_name(Some(1539202764), "AbCdEf123.jpg");
        assert!(named.ends_with(" AbCdEf123.jpg"));
        let prefix = &named[..named.len() - " AbCdEf123.jpg".len()];
        // "YYYY-MM-DD HH.MM.SS" tem 19 caracteres.
        assert_eq!(prefix.len(), 19);
        assert_eq!(&prefix[4..5], "-");
        assert_eq!(&prefix[13..14], ".");
    }

    #[test]
    fn timestamped_file_name_keeps_raw_name_without_timestamp() {
        assert_eq!(timestamped_file_name(None, "best.mp4"), "best.mp4");
    }

    #[test]
    fn url_file_name_strips_query_and_fragment() {
        assert_eq!(
            url_file_name("https://video.twimg.com/vid/best.mp4?tag=12"),
            Some("best.mp4".to_string())
        );
        assert_eq!(url_file_name("https://x.com/path/"), None);
    }

    #[test]
    fn twitter_disk_asset_key_matches_current_and_legacy_names() {
        assert_eq!(
            twitter_disk_asset_key("G972MR2XoAA_QJr.mp4").as_deref(),
            Some("g972mr2xoaa_qjr")
        );
        assert_eq!(
            twitter_disk_asset_key("2026-01-05 20.02.32 GIF_G972MR2XoAA_QJr.mp4").as_deref(),
            Some("g972mr2xoaa_qjr")
        );
        assert_eq!(
            twitter_disk_asset_key("Gif_G972MR2XoAA_QJr.MP4").as_deref(),
            Some("g972mr2xoaa_qjr")
        );
    }

    #[test]
    fn atomic_download_replaces_zero_byte_placeholder_without_leaving_part_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let destination = temp.path().join("media.mp4");
        fs::write(&destination, []).expect("placeholder");

        write_download_atomically(&destination, b"complete media").expect("atomic write");

        assert_eq!(fs::read(&destination).expect("download"), b"complete media");
        assert!(fs::read_dir(temp.path())
            .expect("entries")
            .all(|entry| !entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .ends_with(".part")));
    }

    #[test]
    fn atomic_download_preserves_a_nonempty_existing_destination() {
        let temp = tempfile::tempdir().expect("tempdir");
        let destination = temp.path().join("media.mp4");
        fs::write(&destination, b"existing media").expect("existing");

        let error = write_download_atomically(&destination, b"replacement").expect_err("blocked");

        assert_eq!(error, "destination file already exists");
        assert_eq!(fs::read(&destination).expect("existing"), b"existing media");
        assert!(fs::read_dir(temp.path())
            .expect("entries")
            .all(|entry| !entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .ends_with(".part")));
    }

    #[test]
    fn animated_gif_maps_to_gif_media_type() {
        let media = serde_json::json!({
            "type": "animated_gif",
            "id_str": "42",
            "video_info": {
                "variants": [
                    {"content_type": "video/mp4", "bitrate": 0, "url": "https://video.twimg.com/tweet_video/loop.mp4"}
                ]
            }
        });
        let asset = extract_asset_from_media(&media).expect("gif asset");
        assert_eq!(asset.media_type, "gif");
        assert_eq!(asset.file_name, "loop.mp4");
    }

    #[test]
    fn likes_author_is_not_treated_as_profile_owner_without_an_identity_match() {
        let tweet = ParsedTweet {
            post_key: "post-1".to_string(),
            author_screen_name: Some("someone_else".to_string()),
            author_user_id: Some("other-id".to_string()),
            author_avatar_url: None,
            captured_at_timestamp: None,
            assets: Vec::new(),
        };

        assert!(!tweet_belongs_to_owner(&tweet, "profile_owner", None));
        assert!(!tweet_belongs_to_owner(
            &tweet,
            "profile_owner",
            Some("profile-id")
        ));
        assert!(tweet_belongs_to_owner(
            &tweet,
            "renamed_profile",
            Some("other-id")
        ));
    }

    #[test]
    fn only_posts_missing_enabled_media_remain_out_of_the_completed_post_ledger() {
        let pending = vec![
            PendingTwitterPost {
                provider_post_key: "complete".to_string(),
                media_section: "media".to_string(),
                asset_keys: vec!["downloaded".to_string()],
                was_known_post: true,
                had_missing_assets: false,
            },
            PendingTwitterPost {
                provider_post_key: "failed".to_string(),
                media_section: "likes".to_string(),
                asset_keys: vec!["missing".to_string()],
                was_known_post: true,
                had_missing_assets: true,
            },
            PendingTwitterPost {
                provider_post_key: "recovered-history".to_string(),
                media_section: "likes".to_string(),
                asset_keys: vec!["recovered".to_string()],
                was_known_post: true,
                had_missing_assets: true,
            },
            PendingTwitterPost {
                provider_post_key: "disabled-only".to_string(),
                media_section: "timeline".to_string(),
                asset_keys: Vec::new(),
                was_known_post: true,
                had_missing_assets: false,
            },
        ];
        let available = HashSet::from(["downloaded".to_string(), "recovered".to_string()]);
        let mut summary = TwitterManifestSummary::default();

        let completed = completed_observed_posts(pending, &available, &mut summary);

        assert_eq!(completed.len(), 3);
        assert_eq!(completed[0].provider_post_key, "complete");
        assert_eq!(completed[1].provider_post_key, "recovered-history");
        assert_eq!(completed[2].provider_post_key, "disabled-only");
        assert_eq!(summary.completed_post_count, 3);
        assert_eq!(summary.incomplete_post_count, 1);
        assert_eq!(summary.skipped_existing_post_count, 2);
    }

    #[test]
    fn continuation_cursor_is_extracted_from_gallery_dl_resume_hint() {
        let output = "[twitter][debug] Cursor: older\n[twitter][info] Use '-o cursor=3_1599855479290634240/DAADabc' to continue downloading from the current position";
        assert_eq!(
            parse_continuation_cursor(output).as_deref(),
            Some("3_1599855479290634240/DAADabc")
        );
    }

    #[test]
    fn continuation_cursor_ignores_completed_cursor_marker() {
        assert_eq!(
            parse_continuation_cursor("[twitter][debug] Cursor: None"),
            None
        );
    }

    #[test]
    fn completed_cursor_does_not_fall_back_to_an_older_page_cursor() {
        let output = "[twitter][debug] Cursor: DAABCgABHM91IoZ_previous\n\
                      .\\gallery-dl\\twitter\\profile\\1693020454921867429_1.mp4\n\
                      [twitter][debug] Cursor: None";

        assert_eq!(parse_continuation_cursor(output), None);
    }

    #[test]
    fn tweet_ids_containing_429_do_not_trigger_rate_limit() {
        let output = "gallery-dl --no-download -o ratelimit=abort\n\
                      [urllib3.connectionpool][debug] GET /UserMedia HTTP/1.1\" 200 5073\n\
                      .\\gallery-dl\\twitter\\profile\\1693020454921867429_1.mp4\n\
                      .\\gallery-dl\\twitter\\profile\\1602842910675943425_1.jpg\n\
                      [twitter][debug] Cursor: None";

        assert!(!output_indicates_rate_limit(output));
    }

    #[test]
    fn structured_http_429_and_explicit_messages_trigger_rate_limit() {
        assert!(output_indicates_rate_limit(
            "[urllib3.connectionpool][debug] GET /UserMedia HTTP/1.1\" 429 0"
        ));
        assert!(output_indicates_rate_limit(
            "[twitter][warning] API rate limit exceeded"
        ));
        assert!(output_indicates_rate_limit(
            "HttpError: 429 Too Many Requests"
        ));
    }

    #[test]
    fn incremental_filter_stops_when_twitter_reaches_the_overlap_cutoff() {
        assert_eq!(
            twitter_incremental_post_filter(1_700_000_000),
            "date.timestamp() >= 1700000000 or abort()"
        );
    }

    #[test]
    fn legacy_manifest_deserializes_with_new_telemetry_defaults() {
        let summary: TwitterManifestSummary = serde_json::from_str(
            r#"{"parsedPageCount":3,"normalizedPostCount":10,"rateLimited":false}"#,
        )
        .expect("legacy manifest");

        assert_eq!(summary.parsed_page_count, 3);
        assert_eq!(summary.normalized_post_count, 10);
        assert_eq!(summary.skipped_duplicate_asset_count, 0);
        assert!(!summary.incremental_scan);
        assert_eq!(summary.full_scan_at, None);
    }

    #[test]
    fn existing_hash_index_keeps_the_canonical_file_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let canonical = temp.path().join("canonical.jpg");
        fs::write(&canonical, b"same bytes").expect("media");
        let hash = file_sha256(&canonical).expect("hash");

        let hashes = seed_existing_hashes(temp.path());

        assert_eq!(hashes.get(&hash), Some(&canonical));
    }
}
