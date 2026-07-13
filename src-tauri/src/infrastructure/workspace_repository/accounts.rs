use super::*;

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

        Ok(
            load_app_setting_value(connection, SYNC_DELAY_BETWEEN_PROFILES_SETTING_KEY)?
                .and_then(|value| value.trim().parse::<u64>().ok())
                .unwrap_or(0),
        )
    })
    .unwrap_or(0)
    .min(3600)
}
pub(super) fn upsert_provider_account_with_connection(
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
pub(super) fn delete_provider_account_with_connection(
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
        if load_account_session_secret_ref(connection, &id)?.as_deref() != Some(secret_ref.as_str())
        {
            session_secret_store::delete_secret(layout, &secret_ref)?;
        }
    }

    connection
        .execute("DELETE FROM provider_accounts WHERE id = ?1", params![id])
        .map_err(|error| error.to_string())?;
    load_snapshot(connection, layout)
}
pub(super) fn load_provider_account_editor_with_connection(
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
pub(super) fn save_provider_account_settings_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    account_id: String,
    values: Vec<ProviderAccountSettingValue>,
) -> Result<ProviderAccountEditor, String> {
    ensure_provider_account_exists(connection, &account_id)?;
    let account = load_provider_account_by_id(connection, &account_id)?;
    let protect_authorization =
        load_provider_account_import_state(connection, &account_id)?.is_some();
    let existing_settings = load_provider_account_settings_map(connection, &account_id)?;

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

    preserve_existing_source_media_paths_for_account_setting_change(
        connection,
        layout,
        &account,
        &account_id,
        &existing_settings,
        &serialized_values,
    )?;

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

/// An account media path is the default for new profiles, not a relocation of
/// every profile that already uses it.  Before replacing an account's settings,
/// pin those existing profiles to the root they currently resolve to.  Moving
/// existing media remains the explicit "Change save path" workflow.
fn preserve_existing_source_media_paths_for_account_setting_change(
    connection: &Connection,
    layout: &StorageLayout,
    account: &ProviderAccount,
    account_id: &str,
    existing_settings: &HashMap<String, String>,
    incoming_settings: &[(String, &'static str, String)],
) -> Result<(), String> {
    let media_path_key = format!(
        "{}.account.mediaPath",
        account.provider.to_ascii_lowercase()
    );
    let Some((_, _, next_value)) = incoming_settings
        .iter()
        .find(|(key, _, _)| key == &media_path_key)
    else {
        return Ok(());
    };

    let current_media_path = setting_value(existing_settings, &media_path_key);
    let current_value = current_media_path.as_deref().unwrap_or("").trim();
    if current_value == next_value.trim() {
        return Ok(());
    }

    let now = now_timestamp();
    for source in load_sources(connection)?
        .into_iter()
        .filter(|source| source.account_id.as_deref() == Some(account_id))
    {
        let has_special_path = if source.provider.eq_ignore_ascii_case("instagram") {
            source_instagram_sync_options(&source)
                .special_path
                .as_deref()
                .is_some_and(|path| !path.trim().is_empty())
        } else if source.provider.eq_ignore_ascii_case("twitter") {
            source_twitter_sync_options(&source)
                .special_path
                .as_deref()
                .is_some_and(|path| !path.trim().is_empty())
        } else if source.provider.eq_ignore_ascii_case("tiktok") {
            source_tiktok_sync_options(&source)
                .special_path
                .as_deref()
                .is_some_and(|path| !path.trim().is_empty())
        } else {
            true
        };
        if has_special_path {
            continue;
        }

        let root = resolved_source_media_output_root(layout, &source, Some(existing_settings));
        let mut options = source.sync_options.clone();
        let special_path = Some(root.display().to_string());
        if source.provider.eq_ignore_ascii_case("instagram") {
            options
                .instagram
                .get_or_insert_with(default_instagram_source_sync_options)
                .special_path = special_path;
        } else if source.provider.eq_ignore_ascii_case("twitter") {
            options
                .twitter
                .get_or_insert_with(default_twitter_source_sync_options)
                .special_path = special_path;
        } else if source.provider.eq_ignore_ascii_case("tiktok") {
            options
                .tiktok
                .get_or_insert_with(default_tiktok_source_sync_options)
                .special_path = special_path;
        }

        let serialized = serialize_source_sync_options(&source.provider, &options)?;
        connection
            .execute(
                "UPDATE source_profiles
                 SET sync_options_json = ?2, updated_at = ?3
                 WHERE id = ?1 AND deleted_at IS NULL",
                params![source.id, serialized, now],
            )
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}
pub(super) fn clone_provider_account_with_connection(
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
pub(super) fn next_cloned_account_display_name(
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
pub(super) fn provider_account_display_name_exists(
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
pub(super) fn validate_explicit_source_account_binding(
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
pub(super) struct AccountSyncContext {
    pub(super) account: ProviderAccount,
    pub(super) settings: HashMap<String, String>,
    pub(super) session_payload: String,
}
#[derive(Clone)]
pub(super) struct ProviderAccountSessionRecord {
    pub(super) account_id: String,
    pub(super) auth_mode: String,
    pub(super) session_format: String,
    pub(super) fingerprint: String,
    pub(super) secret_ref: String,
    pub(super) expires_at: Option<String>,
    pub(super) imported_at: String,
    pub(super) last_validated_at: Option<String>,
    pub(super) last_validation_error: Option<String>,
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
pub(super) fn upsert_provider_account_string_setting(
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
pub(super) fn delete_provider_account_setting(
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
pub(super) fn write_provider_account_session_record(
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
pub(super) fn load_provider_account_cookies_with_connection(
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
pub(super) fn save_provider_account_cookies_with_connection(
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
pub(super) fn clear_provider_account_cookies_with_connection(
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
pub(super) fn apply_instagram_auth_settings_from_session_metadata(
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

    // O save substitui o conjunto INTEIRO de settings da conta; sem repor as
    // existentes, um import cookie-only (sem metadata) apagava appId, timers,
    // mediaPath e todo o resto da conta. Merge: preserva o que já existe e
    // sobrepõe apenas as chaves trazidas pela sessão.
    let incoming_keys: HashSet<String> = values
        .iter()
        .map(|value| value.setting_key.clone())
        .collect();
    let mut merged: Vec<ProviderAccountSettingValue> =
        load_provider_account_settings(connection, account_id)?
            .into_iter()
            .filter(|setting| !incoming_keys.contains(&setting.setting_key))
            .collect();
    merged.extend(values);

    let _ = save_provider_account_settings_with_connection(
        connection,
        layout,
        account_id.to_string(),
        merged,
    )?;
    Ok(())
}
pub(super) fn validate_provider_account_with_connection(
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
        RuntimeLogAnchor {
            account_id: Some(&account_id),
            provider: Some(&account.provider),
            source_id: None,
            source_handle: None,
        },
        format!(
            "Validated provider account '{}' as '{}'.",
            account.provider, auth_state
        ),
        validation_error.clone(),
    );

    load_snapshot(connection, layout)
}
pub(super) fn ensure_provider_account_exists(
    connection: &Connection,
    account_id: &str,
) -> Result<(), String> {
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
pub(super) fn load_provider_account_by_id(
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
pub(super) fn load_account_session(
    connection: &Connection,
    layout: &StorageLayout,
    account_id: &str,
) -> Result<Option<ProviderAccountSession>, String> {
    load_account_session_record(connection, account_id)?
        .map(|record| hydrate_account_session(layout, record))
        .transpose()
}
pub(super) fn hydrate_account_session(
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
pub(super) fn load_provider_account_settings(
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
pub(super) fn load_provider_account_settings_map(
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
pub(super) fn map_provider_account_setting_value(
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
pub(super) fn serialize_provider_account_setting_value(
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
pub(super) fn parse_provider_account_setting_value_kind(
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
pub(super) fn load_account_session_record(
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
pub(super) fn load_account_session_secret_ref(
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
pub(super) fn build_cookie_header(cookies: &[CapturedBrowserCookie]) -> String {
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
pub(super) fn convert_captured_cookies_to_provider_cookies(
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
pub(super) fn convert_provider_cookies_to_captured_cookies(
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
pub(super) fn persist_provider_account_session_payload(
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
        connection,
        account_id,
        &secret_ref,
        secret_payload,
        &imported_at,
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
pub(super) fn serialize_session_payload_for_storage(
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
pub(super) fn parse_netscape_cookie_text(
    content: &str,
) -> Result<Vec<CapturedBrowserCookie>, String> {
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
pub(super) fn parse_netscape_expiry(value: &str) -> Option<String> {
    if value.is_empty() || value == "0" {
        return None;
    }

    value
        .parse::<i64>()
        .ok()
        .and_then(|timestamp| DateTime::<Utc>::from_timestamp(timestamp, 0))
        .map(|timestamp| timestamp.to_rfc3339())
}
pub(super) fn normalize_cookie_path(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}
pub(super) fn validate_captured_cookies(cookies: &[CapturedBrowserCookie]) -> Result<(), String> {
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
pub(super) fn write_netscape_cookie_file(
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
pub(super) struct ParsedSessionPayload {
    pub(super) current_url: Option<String>,
    pub(super) metadata: CapturedBrowserMetadata,
    pub(super) cookies: Vec<CapturedBrowserCookie>,
}
pub(super) fn parse_session_payload(secret_payload: &str) -> Result<ParsedSessionPayload, String> {
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
pub(super) fn parse_session_cookies(
    secret_payload: &str,
) -> Result<Vec<CapturedBrowserCookie>, String> {
    parse_session_payload(secret_payload).map(|payload| payload.cookies)
}
pub(super) fn default_cookie_path() -> String {
    "/".to_string()
}
pub(super) fn domain_matches_allowed(domain: &str, allowed: &str) -> bool {
    let normalized_domain = domain.trim().trim_matches('.').to_ascii_lowercase();
    let normalized_allowed = allowed.trim().trim_matches('.').to_ascii_lowercase();
    normalized_domain == normalized_allowed
        || normalized_domain.ends_with(&format!(".{normalized_allowed}"))
}
pub(super) fn validate_session_payload_for_account(
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
pub(super) fn session_fingerprint(secret_payload: &str) -> String {
    let mut hasher = DefaultHasher::new();
    secret_payload.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
pub(super) fn is_expired_timestamp(value: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp < Utc::now())
        .unwrap_or(false)
}
pub(super) fn load_accounts(connection: &Connection) -> Result<Vec<ProviderAccount>, String> {
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
pub(super) fn load_account_sessions(
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
