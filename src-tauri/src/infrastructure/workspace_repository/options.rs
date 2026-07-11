use super::*;

pub(super) fn default_instagram_source_sync_options() -> InstagramSourceSyncOptions {
    InstagramSourceSyncOptions::default()
}
pub(super) fn normalize_instagram_source_sync_options(
    options: Option<InstagramSourceSyncOptions>,
) -> InstagramSourceSyncOptions {
    let mut normalized = options.unwrap_or_else(default_instagram_source_sync_options);
    normalized.temporary = Some(normalized.temporary.unwrap_or(false));
    normalized.favorite = Some(normalized.favorite.unwrap_or(false));
    normalized.download_images = Some(normalized.download_images.unwrap_or(true));
    normalized.download_videos = Some(normalized.download_videos.unwrap_or(true));
    normalized.get_user_media_only = Some(normalized.get_user_media_only.unwrap_or(false));
    normalized.missing_only = Some(normalized.missing_only.unwrap_or(false));
    normalized.full_scan = Some(normalized.full_scan.unwrap_or(false));
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
            .unwrap_or_default(),
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
pub(crate) fn default_source_sync_options(provider: &str) -> SourceSyncOptions {
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
pub(super) fn normalize_source_sync_options(
    provider: &str,
    options: &SourceSyncOptions,
) -> SourceSyncOptions {
    if provider.eq_ignore_ascii_case("instagram") {
        SourceSyncOptions {
            instagram: Some(normalize_instagram_source_sync_options(
                options.instagram.clone(),
            )),
            ..Default::default()
        }
    } else if provider.eq_ignore_ascii_case("twitter") {
        SourceSyncOptions {
            twitter: Some(normalize_twitter_source_sync_options(
                options.twitter.clone(),
            )),
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
pub(super) fn normalize_tiktok_source_sync_options(
    options: Option<TikTokSourceSyncOptions>,
) -> TikTokSourceSyncOptions {
    let defaults = default_tiktok_source_sync_options();
    let mut merged = options.unwrap_or_else(default_tiktok_source_sync_options);
    merged.get_timeline = merged.get_timeline.or(defaults.get_timeline);
    merged.get_stories_user = merged.get_stories_user.or(defaults.get_stories_user);
    merged.get_reposts = merged.get_reposts.or(defaults.get_reposts);
    merged.get_liked_videos = merged.get_liked_videos.or(defaults.get_liked_videos);
    merged.liked_videos_limit = merged.liked_videos_limit.or(defaults.liked_videos_limit);
    merged.liked_videos_incremental = merged
        .liked_videos_incremental
        .or(defaults.liked_videos_incremental);
    merged.liked_videos_known_page_threshold = merged
        .liked_videos_known_page_threshold
        .or(defaults.liked_videos_known_page_threshold);
    merged.collect_media_stats = merged.collect_media_stats.or(defaults.collect_media_stats);
    merged.refresh_existing_media_stats = merged
        .refresh_existing_media_stats
        .or(defaults.refresh_existing_media_stats);
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
pub(super) fn normalize_twitter_source_sync_options(
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
pub(super) fn source_twitter_sync_options(source: &SourceProfile) -> TwitterSourceSyncOptions {
    normalize_source_sync_options(&source.provider, &source.sync_options)
        .twitter
        .unwrap_or_else(|| normalize_twitter_source_sync_options(None))
}
pub(super) fn source_tiktok_sync_options(source: &SourceProfile) -> TikTokSourceSyncOptions {
    normalize_source_sync_options(&source.provider, &source.sync_options)
        .tiktok
        .unwrap_or_else(|| normalize_tiktok_source_sync_options(None))
}
/// Preserva metadados internos do Twitter (`user_id_hint` e `special_path`) que
/// a UI não reenviar em todo upsert, evitando que edições os apaguem do perfil.
pub(super) fn preserve_persisted_twitter_metadata(
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
    // `special_path` é editável na UI: um payload que TRAZ o campo (mesmo
    // vazio) expressa intenção — vazio limpa o override. Só payloads que nem
    // mencionam o campo (presets, fluxos internos) preservam o persistido.
    let incoming_special_sent = incoming
        .map(|twitter| twitter.special_path.is_some())
        .unwrap_or(false);
    if incoming_hint_present && incoming_special_sent {
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
        if let Some(hint) = persisted
            .user_id_hint
            .filter(|value| !value.trim().is_empty())
        {
            twitter.user_id_hint = Some(hint);
        }
    }
    if !incoming_special_sent {
        if let Some(special) = persisted
            .special_path
            .filter(|value| !value.trim().is_empty())
        {
            twitter.special_path = Some(special);
        }
    }
}
/// Igual ao `preserve_persisted_twitter_metadata`, mas para o TikTok.
pub(super) fn preserve_persisted_tiktok_metadata(
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
    // Mesmo contrato do Twitter: campo enviado (mesmo vazio) expressa
    // intenção; ausência preserva o valor persistido.
    let incoming_special_sent = incoming
        .map(|tiktok| tiktok.special_path.is_some())
        .unwrap_or(false);
    if incoming_hint_present && incoming_special_sent {
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
        if let Some(hint) = persisted
            .user_id_hint
            .filter(|value| !value.trim().is_empty())
        {
            tiktok.user_id_hint = Some(hint);
        }
    }
    if !incoming_special_sent {
        if let Some(special) = persisted
            .special_path
            .filter(|value| !value.trim().is_empty())
        {
            tiktok.special_path = Some(special);
        }
    }
}
pub(super) fn serialize_source_sync_options(
    provider: &str,
    options: &SourceSyncOptions,
) -> Result<String, String> {
    serde_json::to_string(&normalize_source_sync_options(provider, options))
        .map_err(|error| error.to_string())
}
pub(super) fn deserialize_source_sync_options(provider: &str, raw: &str) -> SourceSyncOptions {
    serde_json::from_str::<SourceSyncOptions>(raw)
        .map(|value| normalize_source_sync_options(provider, &value))
        .unwrap_or_else(|_| default_source_sync_options(provider))
}
pub(super) fn source_instagram_sync_options(source: &SourceProfile) -> InstagramSourceSyncOptions {
    normalize_source_sync_options(&source.provider, &source.sync_options)
        .instagram
        .unwrap_or_else(default_instagram_source_sync_options)
}
pub(super) fn instagram_handles_present(handles: Option<&Vec<String>>) -> bool {
    handles.map(|list| !list.is_empty()).unwrap_or(false)
}
/// Acrescenta `old_handle` à lista de handles anteriores, normalizando e
/// evitando duplicatas ou o próprio handle atual. Usado quando um perfil do
/// Instagram é renomeado ou importado com um nome legado.
pub(super) fn push_previous_instagram_handle(
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
pub(super) fn preserve_persisted_instagram_metadata(
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
    let incoming_prev_present = instagram_handles_present(
        incoming.and_then(|instagram| instagram.previous_handles.as_ref()),
    );
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
pub(super) fn source_instagram_sync_options_with_override(
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
pub(super) fn instagram_force_update_user_name_enabled(
    options: &InstagramSourceSyncOptions,
) -> bool {
    options.force_update_user_name.unwrap_or(true)
}
pub(super) fn instagram_force_update_user_information_enabled(
    options: &InstagramSourceSyncOptions,
) -> bool {
    options.force_update_user_information.unwrap_or(false)
}
pub(super) fn instagram_profile_script_pattern(
    options: &InstagramSourceSyncOptions,
) -> Option<String> {
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
pub(super) fn instagram_user_id_hint(options: &InstagramSourceSyncOptions) -> Option<&str> {
    options
        .user_id_hint
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}
pub(super) fn preferred_instagram_user_id_hint(
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
pub(super) fn instagram_special_path(options: &InstagramSourceSyncOptions) -> Option<&str> {
    options
        .special_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}
pub(super) fn instagram_username_override(options: &InstagramSourceSyncOptions) -> Option<&str> {
    options
        .username_override
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}
pub(super) fn instagram_missing_only_enabled(options: &InstagramSourceSyncOptions) -> bool {
    options.missing_only.unwrap_or(false)
}
pub(super) fn instagram_full_scan_enabled(options: &InstagramSourceSyncOptions) -> bool {
    options.full_scan.unwrap_or(false)
}
pub(super) fn parse_instagram_sync_date_boundary(
    raw: Option<&str>,
    end_of_day: bool,
) -> Option<i64> {
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
pub(super) fn instagram_date_from_timestamp(options: &InstagramSourceSyncOptions) -> Option<i64> {
    parse_instagram_sync_date_boundary(options.date_from.as_deref(), false)
}
pub(super) fn instagram_date_to_timestamp(options: &InstagramSourceSyncOptions) -> Option<i64> {
    parse_instagram_sync_date_boundary(options.date_to.as_deref(), true)
}
pub(super) fn validate_source_sync_override(
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
pub(super) fn source_twitter_sync_options_with_override(
    source: &SourceProfile,
    sync_options_override: Option<&SourceSyncOptions>,
) -> TwitterSourceSyncOptions {
    if let Some(override_options) =
        sync_options_override.and_then(|options| options.twitter.clone())
    {
        return normalize_twitter_source_sync_options(Some(override_options));
    }
    source_twitter_sync_options(source)
}
pub(super) fn source_tiktok_sync_options_with_override(
    source: &SourceProfile,
    sync_options_override: Option<&SourceSyncOptions>,
) -> TikTokSourceSyncOptions {
    if let Some(override_options) = sync_options_override.and_then(|options| options.tiktok.clone())
    {
        return normalize_tiktok_source_sync_options(Some(override_options));
    }
    source_tiktok_sync_options(source)
}
