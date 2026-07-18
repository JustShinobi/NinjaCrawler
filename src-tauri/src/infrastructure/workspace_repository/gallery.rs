use super::*;

pub(super) const GALLERY_VIDEO_EXTS: [&str; 5] = ["mp4", "webm", "mkv", "mov", "m4v"];
pub(super) const GALLERY_IMAGE_EXTS: [&str; 6] = ["jpg", "jpeg", "png", "webp", "heic", "gif"];
/// Converte o prefixo de data dos nomes (`YYYY-MM-DD HH.MM.SS`, hora local) em
/// unix. Retorna (unix, resto_do_nome_sem_prefixo).
pub(super) fn strip_gallery_date_prefix(stem: &str) -> (Option<i64>, String) {
    // Formato fixo: 19 chars + espaço.
    if stem.len() > 20 {
        let (prefix, rest) = stem.split_at(19);
        if let Some(name) = rest.strip_prefix(' ') {
            if let Ok(naive) = NaiveDateTime::parse_from_str(prefix, "%Y-%m-%d %H.%M.%S") {
                let unix = Local
                    .from_local_datetime(&naive)
                    .single()
                    .map(|dt| dt.timestamp());
                return (unix, name.to_string());
            }
        }
    }
    (None, stem.to_string())
}
pub(super) struct DerivedPost {
    pub(super) post_id: Option<String>,
    pub(super) captured_at: Option<i64>,
    pub(super) media_type: &'static str,
    /// Chave de agrupamento (post_id quando houver, senão o nome).
    pub(super) group_key: String,
    /// Índice da imagem no slideshow (`_index_<i>_<n>`), 0-based.
    pub(super) index: Option<usize>,
}
/// Arquivo de avatar/foto de perfil que NÃO deve aparecer na galeria.
pub(super) fn is_profile_image_file(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    lower.contains("_avatar.") || lower.starts_with("profilepicture")
}
/// Monta o link original do post (nível do post — depende do tipo final).
pub(super) fn build_post_url(
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
pub(super) fn derive_post_metadata(
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
        if token.len() >= 10 && token.chars().all(|c| c.is_ascii_digit()) && token.len() >= best_len
        {
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
            t.parse::<i64>()
                .ok()
                .filter(|v| (1_400_000_000..4_000_000_000).contains(v))
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
pub(super) struct GalleryPostAcc {
    pub(super) post_id: Option<String>,
    pub(super) captured_at: Option<i64>,
    /// Menor download time (unix) entre os arquivos do post — o momento em que
    /// o post começou a ser baixado. Preferido do ledger, com fallback mtime.
    pub(super) downloaded_at: Option<i64>,
    /// Autor original (só nos Likes do TikTok); ver `derive_like_author`.
    pub(super) author: Option<String>,
    pub(super) media_type: String,
    pub(super) section: String,
    /// Highlight album (subpasta sob `Stories/`), quando o post for um highlight.
    pub(super) album: Option<String>,
    /// Álbuns de highlight resolvidos por associação (mídia que mora no Feed mas
    /// pertence a um destaque), casados pela media key dos arquivos.
    pub(super) membership_albums: BTreeSet<String>,
    pub(super) files: Vec<(Option<usize>, MediaGalleryFile)>,
    /// Authoritative metadata joined from the sync ledger by relative path
    /// (preferred over the values derived from the file name).
    pub(super) ledger_post_key: Option<String>,
    pub(super) ledger_post_code: Option<String>,
    pub(super) ledger_section: Option<String>,
    pub(super) ledger_captured_at: Option<i64>,
}
/// Post link metadata joined from the per-provider media ledger, keyed by the
/// (lowercased) relative path of each downloaded file.
#[derive(Default, Clone)]
pub(super) struct GalleryMediaLedgerLink {
    pub(super) post_key: Option<String>,
    pub(super) post_code: Option<String>,
    pub(super) section: Option<String>,
    pub(super) captured_at: Option<i64>,
    /// `first_seen_at` do ledger em unix — quando o app baixou/viu a mídia
    /// pela 1ª vez. Serve de eixo "Download Date" na ordenação.
    pub(super) downloaded_at: Option<i64>,
}
#[derive(Default, Clone)]
pub(super) struct GalleryPostStats {
    pub(super) view_count: Option<i64>,
    pub(super) like_count: Option<i64>,
    pub(super) comment_count: Option<i64>,
    pub(super) share_count: Option<i64>,
    pub(super) updated_at: Option<String>,
}
pub(super) fn load_gallery_post_stats(
    connection: &Connection,
    provider: &str,
    source_id: &str,
) -> HashMap<String, GalleryPostStats> {
    let Ok(mut statement) = connection.prepare(
        "SELECT provider_post_key, view_count, like_count, comment_count,
                share_count, stats_updated_at
         FROM provider_sync_post_ledger
         WHERE provider = ?1 AND source_id = ?2 AND stats_updated_at IS NOT NULL",
    ) else {
        return HashMap::new();
    };
    let Ok(rows) = statement.query_map(params![provider, source_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            GalleryPostStats {
                view_count: row.get(1)?,
                like_count: row.get(2)?,
                comment_count: row.get(3)?,
                share_count: row.get(4)?,
                updated_at: row.get(5)?,
            },
        ))
    }) else {
        return HashMap::new();
    };
    rows.flatten().collect()
}
/// Converte um timestamp RFC3339 (como os `first_seen_at`/`last_seen_at` dos
/// ledgers) em unix seconds. Retorna `None` quando o valor não é parseável.
pub(super) fn parse_rfc3339_unix(value: &str) -> Option<i64> {
    parse_rfc3339_utc(value.trim()).map(|dt| dt.timestamp())
}
/// Autor de um like/favorito a partir do nome do arquivo — fallback quando o
/// `account_sync_media_ledger` não tem a linha (import antigo / slideshow).
/// Cobre o naming do yt-dlp (`<uploader>_<videoId>.<ext>`) e o do 4K Tokkit
/// (`<uploader>_<unix>_<videoId>[_index_<i>_<n>].<ext>`): o autor são os
/// tokens antes do id do post, descartando o token unix (9-11 dígitos).
pub(super) fn derive_like_author(file_name: &str, post_id: Option<&str>) -> Option<String> {
    let stem = file_name
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(file_name);
    let post_id = post_id?;
    let tokens: Vec<&str> = stem.split('_').collect();
    let id_index = tokens.iter().position(|token| *token == post_id)?;
    let mut author_tokens = &tokens[..id_index];
    // Descarta o token unix do Tokkit imediatamente antes do id.
    if let Some((last, rest)) = author_tokens.split_last() {
        if (9..=11).contains(&last.len()) && last.chars().all(|c| c.is_ascii_digit()) {
            author_tokens = rest;
        }
    }
    let author = author_tokens.join("_");
    let author = author.trim();
    (!author.is_empty()).then(|| author.to_string())
}
/// Lista a mídia baixada de um perfil agrupada por post, com o link original
/// reconstruído. O front agrupa por dia (via `captured_at`).
pub fn load_source_media_gallery(source_id: String) -> Result<SourceMediaGallery, String> {
    with_workspace(|connection, layout| {
        let row = connection
            .query_row(
                "SELECT provider, handle, account_id, sync_options_json,
                        profile_biography, profile_follower_count, profile_following_count,
                        profile_media_count, profile_is_verified, profile_stats_updated_at
                 FROM source_profiles
                 WHERE id = ?1 AND deleted_at IS NULL LIMIT 1",
                params![&source_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, Option<i64>>(7)?,
                        row.get::<_, Option<i64>>(8)?,
                        row.get::<_, Option<String>>(9)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Source '{}' does not exist.", source_id))?;
        let (
            provider,
            handle,
            account_id,
            sync_options_json,
            profile_biography,
            profile_follower_count,
            profile_following_count,
            profile_media_count,
            profile_is_verified,
            profile_stats_updated_at,
        ) = row;

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
        // Autor real dos likes do TikTok (basename → @autor). Vazio nos demais.
        let like_authors = load_tiktok_like_authors(connection, &provider);
        let post_stats = load_gallery_post_stats(connection, &provider, &source_id);
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
            // A pasta física dos likes do TikTok é `Liked/` (nova) ou `Likes/`
            // (legacy); sem linha no ledger (reimport/mídia antiga) o link não
            // é resolvido, então classifica direto aqui para o filtro "Likes"
            // não vazar em "Timeline". Com ledger, `ledger_section` prevalece.
            // `Favorites/` (4K Tokkit) também é conteúdo de terceiros — seção
            // própria em vez de inflar a Timeline.
            let section = match top_segment.as_str() {
                "stories" | "reposts" | "video" | "favorites" => top_segment.clone(),
                "liked" | "likes" => "likes".to_string(),
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
            let link = ledger_links
                .get(&relative_path.to_ascii_lowercase())
                .cloned();
            // Download time do arquivo: `first_seen_at` do ledger, senão o mtime.
            let file_downloaded_at = link
                .as_ref()
                .and_then(|entry| entry.downloaded_at)
                .or(mtime_unix);
            // Autor real do like (map por basename do `account_sync_media_ledger`);
            // fallback no nome do arquivo quando não há linha no ledger. Vale
            // também para Favorites — igualmente conteúdo de outros autores.
            let author = like_authors
                .get(&file_name.to_ascii_lowercase())
                .cloned()
                .or_else(|| {
                    if section == "likes" || section == "favorites" {
                        derive_like_author(file_name, derived.post_id.as_deref())
                    } else {
                        None
                    }
                });
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
                    downloaded_at: file_downloaded_at,
                    author: author.clone(),
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
            // Download time do post = o mais antigo entre seus arquivos.
            if let Some(dl) = file_downloaded_at {
                entry.downloaded_at =
                    Some(entry.downloaded_at.map_or(dl, |current| current.min(dl)));
            }
            if entry.author.is_none() {
                entry.author = author;
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
                    if let Some(post_code) = link.post_code.filter(|value| !value.trim().is_empty())
                    {
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
                if let Some(status_id) = twitter_media_key_from_file_name(file_name)
                    .and_then(|key| twitter_post_keys.get(&key))
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
                let stats = post_id_for_url
                    .and_then(|post_id| post_stats.get(post_id))
                    .cloned()
                    .unwrap_or_default();
                // Seção e data preferem o ledger; caem para o derivado do nome.
                let section = acc
                    .ledger_section
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or(acc.section);
                let captured_at = acc.ledger_captured_at.or(acc.captured_at);
                // Poster = capa DISTINTA (cover de vídeo / highlight), quando
                // houver. Para foto NÃO usamos o próprio arquivo como poster: o
                // grid gera um thumb (.thumbs) e cai no original só enquanto ele
                // não chega. Preencher com o arquivo original aqui contornava o
                // sistema de thumb de foto — o front pulava a geração porque o
                // `posterPath` já vinha setado (== o próprio arquivo).
                let poster_path = acc
                    .post_id
                    .as_deref()
                    .and_then(|id| posters.get(id).cloned());
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
                    downloaded_at: acc.downloaded_at,
                    author: acc.author,
                    media_type,
                    section,
                    albums,
                    poster_path,
                    view_count: stats.view_count,
                    like_count: stats.like_count,
                    comment_count: stats.comment_count,
                    share_count: stats.share_count,
                    stats_updated_at: stats.updated_at,
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
            biography: profile_biography
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            follower_count: profile_follower_count,
            following_count: profile_following_count,
            media_count: profile_media_count,
            is_verified: profile_is_verified.map(|value| value != 0),
            stats_updated_at: profile_stats_updated_at,
        })
    })
}
/// Reads the substring of `url` right after `marker` up to the next path/query
/// separator. Used to recover a post key/shortcode from the gallery's post URL.
pub(super) fn url_segment_after(url: &str, marker: &str) -> Option<String> {
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
pub(super) fn extract_post_tombstone_keys(
    provider: &str,
    post: &MediaGalleryPost,
) -> (Option<String>, Option<String>) {
    match provider {
        "tiktok" => (
            post.post_id.clone().or_else(|| {
                post.post_url.as_deref().and_then(|url| {
                    url_segment_after(url, "/video/").or_else(|| url_segment_after(url, "/photo/"))
                })
            }),
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
        let canonical_root =
            fs::canonicalize(&profile_root).unwrap_or_else(|_| profile_root.clone());

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
            // A thumbnail é derivada e não precisa ir para a Lixeira junto com
            // a mídia. Removê-la evita lixo órfão em `.thumbs`.
            if let Some(thumbnail) = video_thumbnail_path(&abs) {
                let _ = fs::remove_file(thumbnail);
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
pub(super) fn catalog_source_media_output(
    _connection: &Connection,
    _context: &SourceSyncContext,
    output_root: &Path,
    _captured_at: &str,
) -> Result<usize, String> {
    Ok(count_downloaded_media_items(output_root) as usize)
}
pub(super) fn collect_media_file_paths(root: &Path) -> Result<Vec<PathBuf>, String> {
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
                // Thumbnails derivadas não são mídia da galeria. Sem esta guarda,
                // os jpgs em `<media-dir>/.thumbs` virariam posts duplicados.
                if entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(".thumbs")
                {
                    continue;
                }
                pending.push(path);
                continue;
            }

            if file_type.is_file()
                && entry
                    .metadata()
                    .map(|metadata| metadata.len() > 0)
                    .unwrap_or(false)
            {
                collected.push(path);
            }
        }
    }

    collected.sort();
    Ok(collected)
}

pub(super) fn cleanup_empty_media_artifacts(root: &Path) -> Result<usize, String> {
    if !root.exists() {
        return Ok(0);
    }

    let mut removed = 0;
    let mut pending = vec![root.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|error| error.to_string())?;
            if file_type.is_dir() {
                if !entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(".thumbs")
                {
                    pending.push(path);
                }
                continue;
            }
            if !file_type.is_file()
                || entry
                    .metadata()
                    .map(|metadata| metadata.len() > 0)
                    .unwrap_or(true)
            {
                continue;
            }

            let is_avatar_download = entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(&format!("{PROFILE_PICTURE_FILE_NAME}.download"));
            if infer_media_type(&path).is_some() || is_avatar_download {
                fs::remove_file(&path).map_err(|error| {
                    format!(
                        "Failed to remove empty media artifact '{}': {error}",
                        path.display()
                    )
                })?;
                removed += 1;
            }
        }
    }
    Ok(removed)
}
pub(super) fn normalize_media_file_path(path: &Path) -> Result<String, String> {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    Ok(resolved.to_string_lossy().into_owned())
}
pub(super) fn average_hash_64(image: &image::DynamicImage) -> String {
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
pub(super) fn difference_hash_64(image: &image::DynamicImage) -> String {
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
pub(super) fn count_downloaded_media_items(root: &Path) -> u32 {
    collect_media_file_paths(root)
        .map(|paths| {
            paths
                .into_iter()
                .filter(|path| !is_profile_picture_file(path) && infer_media_type(path).is_some())
                .count() as u32
        })
        .unwrap_or(0)
}
