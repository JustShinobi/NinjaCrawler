use super::*;

pub fn upsert_source_profile(input: SourceProfileUpsert) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        upsert_source_profile_with_connection(connection, layout, input)
    })
}
pub fn batch_update_source_profiles(
    patch: BatchSourceProfilePatch,
) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        batch_update_source_profiles_with_connection(connection, layout, patch)
    })
}

pub(super) fn batch_update_source_profiles_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    patch: BatchSourceProfilePatch,
) -> Result<WorkspaceSnapshot, String> {
    {
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
                        "SELECT provider, labels_json, ready_for_download, sync_options_json, group_id FROM source_profiles WHERE id = ?1 AND deleted_at IS NULL",
                        params![source_id],
                        |row| {
                            let provider: String = row.get(0)?;
                            let labels_json: String = row.get(1)?;
                            let ready_for_download: bool = row.get(2)?;
                            let sync_options_json: String = row.get(3)?;
                            let group_id: Option<String> = row.get(4)?;
                            Ok((provider, labels_json, ready_for_download, sync_options_json, group_id))
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

            for (
                source_id,
                (provider, labels_json, current_ready, sync_options_json, current_group_id),
            ) in loaded_sources
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

                let mut sync_options: serde_json::Value = serde_json::from_str(&sync_options_json)
                    .unwrap_or_else(|_| serde_json::json!({}));
                if !sync_options.is_object() {
                    sync_options = serde_json::json!({});
                }
                if let Some(provider_patch) = patch
                    .sync_options_patch
                    .as_ref()
                    .and_then(|sync_patch| sync_patch.for_provider(&provider))
                {
                    let provider_options = sync_options
                        .as_object_mut()
                        .expect("sync options were normalized to an object")
                        .entry(provider.clone())
                        .or_insert_with(|| serde_json::json!({}));
                    merge_json_patch(provider_options, provider_patch);
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
    }
}

fn merge_json_patch(target: &mut serde_json::Value, patch: &serde_json::Value) {
    match (target, patch) {
        (serde_json::Value::Object(target), serde_json::Value::Object(patch)) => {
            for (key, value) in patch {
                merge_json_patch(
                    target.entry(key.clone()).or_insert(serde_json::Value::Null),
                    value,
                );
            }
        }
        (target, patch) => *target = patch.clone(),
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
pub(super) fn delete_source_profile_with_connection(
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
pub(super) fn delete_source_profile_with_connection_and_progress<F>(
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
pub(super) fn remove_source_media_directories(
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
#[derive(Clone)]
pub struct SourceSyncQueueItemSeed {
    pub source_id: String,
    pub provider: String,
    pub handle: String,
    pub account_id: Option<String>,
}
/// Job pendente da fila manual de sync, persistido para sobreviver ao
/// fechamento do app.
#[derive(Clone)]
pub struct PersistedSourceSyncQueueJob {
    pub job_key: String,
    pub source_id: String,
    pub trigger: String,
    pub run_mode: Option<String>,
    pub sync_options_override: Option<SourceSyncOptions>,
    pub queued_at: String,
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
/// Pedido de enfileiramento gerado por um plano: a fonte a sincronizar e o
/// trigger a registrar. A camada de runtime (que tem o AppHandle) enfileira.
pub struct PlanSyncEnqueueRequest {
    pub source_id: String,
    pub trigger: String,
}
pub fn open_source_folder(source_id: String) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        open_source_folder_with_connection(connection, layout, &source_id)?;
        load_snapshot(connection, layout)
    })
}
pub(super) fn open_source_folder_with_connection(
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
pub(super) fn upsert_source_profile_with_connection(
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
pub(super) fn sanitize_source_handle(provider: &str, handle: &str) -> String {
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
pub(super) fn source_dedupe_key(provider: &str, handle: &str) -> String {
    sanitize_source_handle(provider, handle).to_lowercase()
}
/// Procura outro perfil ativo (`deleted_at IS NULL`) do mesmo provider cujo handle
/// normalizado colida com o handle informado, ignorando o próprio `self_id`.
/// Retorna o handle do perfil conflitante, se houver.
pub(super) fn find_conflicting_source_handle(
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
pub(super) fn source_target_url(provider: &str, handle: &str) -> String {
    let handle = handle.trim().trim_start_matches('@');
    match provider {
        "instagram" => format!("https://www.instagram.com/{}/", handle),
        // O TikTok exige o `@` no path do perfil.
        "tiktok" => format!("https://www.tiktok.com/@{}", handle),
        "twitter" => format!("https://x.com/{}", handle),
        _ => handle.to_string(),
    }
}
/// Move o conteúdo de uma pasta para outra: tenta `rename` (rápido, mesmo
/// volume) e recorre a cópia recursiva + remoção quando o destino está em outro
/// volume. Os relative_path dos ledgers são relativos à pasta do perfil, então
/// mover a pasta mantém o histórico de downloads consistente.
pub(super) fn move_media_directory(from: &Path, to: &Path) -> Result<(), String> {
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
pub(super) fn copy_dir_recursive(from: &Path, to: &Path) -> Result<(), String> {
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
            let options = source_instagram_sync_options(source);
            let old_root = resolve_instagram_profile_root_with_options(
                layout,
                source,
                account_settings.as_ref(),
                Some(&options),
            );

            let folder = sanitize_path_segment(
                sanitize_source_handle("instagram", &source.handle).trim_start_matches('@'),
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
pub(super) fn compute_source_media_paths(
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
pub(super) fn load_sources(connection: &Connection) -> Result<Vec<SourceProfile>, String> {
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
