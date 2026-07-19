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
    match mode {
        SourceProfileDeleteMode::UserOnly => with_workspace(|connection, layout| {
            delete_source_profile_with_connection_and_progress(
                connection,
                layout,
                id,
                SourceProfileDeleteMode::UserOnly,
                &mut on_progress,
            )
        }),
        // With-media: do heavy disk I/O *outside* with_workspace so other UI
        // commands are not stuck behind an open SQLite connection for minutes.
        SourceProfileDeleteMode::WithMedia => {
            delete_source_profile_with_media_progress(id, &mut on_progress)
        }
    }
}

struct PreparedWithMediaDelete {
    source: SourceProfile,
    primary_root: PathBuf,
    media_directories: HashSet<PathBuf>,
}

fn delete_source_profile_with_media_progress<F>(
    id: String,
    on_progress: &mut F,
) -> Result<WorkspaceSnapshot, String>
where
    F: FnMut(SourceDeleteProgressUpdate) -> Result<(), String>,
{
    on_progress(SourceDeleteProgressUpdate {
        progress_percent: Some(4),
        progress_label: Some("Loading source".to_string()),
        progress_detail: Some("Reading profile metadata and media paths.".to_string()),
        progress_indeterminate: false,
        files_processed: None,
        files_total: None,
    })?;

    let prepared = with_workspace(|connection, layout| {
        prepare_with_media_delete(connection, layout, &id)
    })?;

    on_progress(SourceDeleteProgressUpdate {
        progress_percent: Some(8),
        progress_label: Some("Inventorying media".to_string()),
        progress_detail: Some(format!(
            "Counting files under {} media root(s) for {}.",
            prepared.media_directories.len(),
            prepared.source.handle
        )),
        progress_indeterminate: true,
        files_processed: None,
        files_total: None,
    })?;

    let files_total = count_entries_under_roots(&prepared.media_directories);
    on_progress(SourceDeleteProgressUpdate {
        progress_percent: Some(12),
        progress_label: Some("Inventory complete".to_string()),
        progress_detail: Some(format!(
            "Found {files_total} file(s)/folder(s) to remove for {}.",
            prepared.source.handle
        )),
        progress_indeterminate: false,
        files_processed: Some(0),
        files_total: Some(files_total),
    })?;

    // Disk phase — no DB connection held. Progress maps into 12%..78%.
    let mut files_processed = 0_u32;
    let mut last_emit = std::time::Instant::now()
        .checked_sub(std::time::Duration::from_secs(1))
        .unwrap_or_else(std::time::Instant::now);
    let disk_error = {
        let mut disk_err: Option<String> = None;
        for (index, directory) in prepared.media_directories.iter().enumerate() {
            let root_label = directory
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| directory.display().to_string());
            if let Err(error) = remove_directory_resilient_with_progress(directory, &mut |path| {
                files_processed = files_processed.saturating_add(1);
                let now = std::time::Instant::now();
                // Throttle UI events; always emit on the last item.
                let is_last = files_total > 0 && files_processed >= files_total;
                if !is_last && now.duration_since(last_emit) < std::time::Duration::from_millis(120)
                {
                    return Ok(());
                }
                last_emit = now;
                let disk_fraction = if files_total == 0 {
                    1.0
                } else {
                    f64::from(files_processed) / f64::from(files_total.max(1))
                };
                let percent = 12 + ((disk_fraction * 66.0).round() as u32).min(66);
                let name = path
                    .file_name()
                    .map(|value| value.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                on_progress(SourceDeleteProgressUpdate {
                    progress_percent: Some(percent.min(78)),
                    progress_label: Some(format!(
                        "Deleting media ({}/{})",
                        index + 1,
                        prepared.media_directories.len().max(1)
                    )),
                    progress_detail: Some(format!(
                        "Root · {root_label} · removing {name} ({files_processed}/{files_total})"
                    )),
                    progress_indeterminate: false,
                    files_processed: Some(files_processed),
                    files_total: Some(files_total),
                })
            }) {
                disk_err = Some(error);
                break;
            }
        }
        disk_err
    };

    if disk_error.is_none() {
        on_progress(SourceDeleteProgressUpdate {
            progress_percent: Some(80),
            progress_label: Some("Media removed".to_string()),
            progress_detail: Some(format!(
                "Removed {files_processed} item(s) from disk. Updating database…"
            )),
            progress_indeterminate: false,
            files_processed: Some(files_processed),
            files_total: Some(files_total.max(files_processed)),
        })?;
    }

    on_progress(SourceDeleteProgressUpdate {
        progress_percent: Some(84),
        progress_label: Some("Updating database".to_string()),
        progress_detail: Some("Opening workspace database to remove the profile record.".to_string()),
        progress_indeterminate: false,
        files_processed: Some(files_processed),
        files_total: Some(files_total.max(files_processed)),
    })?;

    let snapshot = with_workspace(|connection, layout| {
        if let Some(disk_error) = disk_error.as_ref() {
            on_progress(SourceDeleteProgressUpdate {
                progress_percent: Some(85),
                progress_label: Some("Reconciling ledgers".to_string()),
                progress_detail: Some(
                    "Disk wipe hit a lock; purging ledger rows for missing files.".to_string(),
                ),
                progress_indeterminate: false,
                files_processed: Some(files_processed),
                files_total: Some(files_total.max(files_processed)),
            })?;
            let _ = purge_provider_ledgers_missing_on_disk(
                connection,
                &prepared.source.provider,
                &prepared.source.id,
                &prepared.primary_root,
            );
            let still_has_media = !media_tree_is_effectively_empty(&prepared.primary_root)
                || prepared
                    .media_directories
                    .iter()
                    .any(|dir| !media_tree_is_effectively_empty(dir));
            if still_has_media {
                return Err(format!(
                    "{disk_error} Ledger rows for missing files were purged so the next sync can re-download them. Retry delete-with-media after closing open files."
                ));
            }
            // Media effectively gone — finish the profile delete.
            let _ = remove_prepared_media_directories(&prepared.media_directories);
        }

        on_progress(SourceDeleteProgressUpdate {
            progress_percent: Some(90),
            progress_label: Some("Removing profile image cache".to_string()),
            progress_detail: Some("Deleting custom avatar and thumbnail cache.".to_string()),
            progress_indeterminate: false,
            files_processed: Some(files_processed),
            files_total: Some(files_total.max(files_processed)),
        })?;
        remove_source_custom_profile_images(layout, &prepared.source.id)?;
        remove_avatar_thumbnail(layout, &prepared.source.id);

        on_progress(SourceDeleteProgressUpdate {
            progress_percent: Some(94),
            progress_label: Some("Deleting profile record".to_string()),
            progress_detail: Some(
                "Hard-deleting source_profiles (cascades ledgers so re-add starts clean).".to_string(),
            ),
            progress_indeterminate: false,
            files_processed: Some(files_processed),
            files_total: Some(files_total.max(files_processed)),
        })?;
        connection
            .execute(
                "DELETE FROM source_profiles WHERE id = ?1",
                params![&prepared.source.id],
            )
            .map_err(|error| error.to_string())?;

        on_progress(SourceDeleteProgressUpdate {
            progress_percent: Some(98),
            progress_label: Some("Building workspace snapshot".to_string()),
            progress_detail: Some("Refreshing library state after profile removal.".to_string()),
            progress_indeterminate: false,
            files_processed: Some(files_processed),
            files_total: Some(files_total.max(files_processed)),
        })?;
        load_snapshot(connection, layout)
    })?;

    // 100% only after the snapshot is ready — never mark the queue job done earlier.
    on_progress(SourceDeleteProgressUpdate {
        progress_percent: Some(100),
        progress_label: Some("Delete complete".to_string()),
        progress_detail: Some(format!(
            "Removed profile {} and {files_processed} on-disk item(s).",
            prepared.source.handle
        )),
        progress_indeterminate: false,
        files_processed: Some(files_processed),
        files_total: Some(files_total.max(files_processed)),
    })?;

    Ok(snapshot)
}

fn prepare_with_media_delete(
    connection: &Connection,
    layout: &StorageLayout,
    id: &str,
) -> Result<PreparedWithMediaDelete, String> {
    let source = connection
        .query_row(
            "SELECT id, provider, source_kind, handle, display_name, account_id, labels_json, ready_for_download, sync_options_json, profile_image_path, profile_image_custom, remote_state, is_subscription, last_synced_at, sync_problem_code, sync_problem_message, sync_problem_at, created_at, group_id, importer_id, imported_at
             FROM source_profiles
             WHERE id = ?1
               AND deleted_at IS NULL
             LIMIT 1",
            params![id],
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
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Source '{id}' does not exist."))?;

    let account_settings = source
        .account_id
        .as_deref()
        .filter(|_| source.provider.eq_ignore_ascii_case("instagram"))
        .map(|account_id| load_provider_account_settings_map(connection, account_id))
        .transpose()?
        .unwrap_or_default();

    let primary_root =
        resolved_source_media_output_root(layout, &source, Some(&account_settings));
    let media_directories =
        collect_source_media_directories(layout, &source, &account_settings);

    Ok(PreparedWithMediaDelete {
        source,
        primary_root,
        media_directories,
    })
}

fn remove_prepared_media_directories(directories: &HashSet<PathBuf>) -> Result<(), String> {
    let mut errors = Vec::new();
    for directory in directories {
        if let Err(error) = remove_directory_resilient(directory) {
            errors.push(error);
        }
    }
    if let Some(error) = errors.into_iter().next() {
        return Err(error);
    }
    Ok(())
}

fn count_entries_under_roots(directories: &HashSet<PathBuf>) -> u32 {
    let mut total = 0_u32;
    for directory in directories {
        total = total.saturating_add(count_tree_entries(directory));
    }
    total
}

fn count_tree_entries(path: &Path) -> u32 {
    if !path.exists() {
        return 0;
    }
    let mut total = 1_u32; // the root itself
    let mut stack = vec![path.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            total = total.saturating_add(1);
            let child = entry.path();
            if child.is_dir() {
                stack.push(child);
            }
        }
    }
    total
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

            // Resolve roots *before* any mutation so specialPath / account
            // mediaPath still resolve while the profile row exists.
            let primary_root = resolved_source_media_output_root(
                layout,
                &source,
                Some(&account_settings),
            );
            let disk_result =
                remove_source_media_directories(layout, &source, &account_settings);

            match disk_result {
                Ok(()) => {}
                Err(disk_error) => {
                    // Partial wipe is the dangerous case: files gone, ledger still
                    // claims every post is known → next sync downloads nothing.
                    // Reconcile ledger against what remains, then decide.
                    let _ = purge_provider_ledgers_missing_on_disk(
                        connection,
                        &source.provider,
                        &source.id,
                        &primary_root,
                    );

                    let media_directories =
                        collect_source_media_directories(layout, &source, &account_settings);
                    if media_tree_is_effectively_empty(&primary_root)
                        && media_directories
                            .iter()
                            .all(|dir| media_tree_is_effectively_empty(dir))
                    {
                        // User asked for with-media; media is gone. Finish the DB
                        // delete so CASCADE clears ledgers and the ghost profile.
                        let _ = remove_source_media_directories(layout, &source, &account_settings);
                    } else {
                        return Err(format!(
                            "{disk_error} Ledger rows for missing files were purged so the next sync can re-download them. Retry delete-with-media after closing open files."
                        ));
                    }
                }
            }

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
            remove_avatar_thumbnail(layout, &source.id);

            on_progress(SourceDeleteProgressUpdate {
                progress_percent: Some(88),
                progress_label: Some("Deleting profile".to_string()),
                progress_detail: Some(
                    "Removing the source profile record and cascading ledgers.".to_string(),
                ),
                progress_indeterminate: false,
                files_processed: None,
                files_total: None,
            })?;
            // Hard delete cascades provider_sync_*_ledger, provider_deleted_media,
            // account_sync_scope_state, etc. Re-adding the same handle starts clean.
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
/// Delete every known media root for a source (resolved specialPath, default
/// layout, @handle variants, Instagram account base). Used by delete-with-media.
pub(super) fn remove_source_media_directories(
    layout: &StorageLayout,
    source: &SourceProfile,
    account_settings: &HashMap<String, String>,
) -> Result<(), String> {
    let directories = collect_source_media_directories(layout, source, account_settings);
    let mut errors = Vec::new();
    for directory in &directories {
        if let Err(error) = remove_directory_resilient(directory) {
            errors.push(error);
        }
    }
    if let Some(error) = errors.into_iter().next() {
        return Err(error);
    }
    Ok(())
}

fn collect_source_media_directories(
    layout: &StorageLayout,
    source: &SourceProfile,
    account_settings: &HashMap<String, String>,
) -> HashSet<PathBuf> {
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

    directories
}

/// True when the path is missing, or only has empty dirs / `.thumbs` leftovers
/// (no real media). Used to finish with-media delete after a partial wipe.
fn media_tree_is_effectively_empty(path: &Path) -> bool {
    if !path.exists() {
        return true;
    }
    let Ok(files) = collect_media_file_paths(path) else {
        return false;
    };
    files.is_empty()
}

/// Windows often returns ERROR_DIR_NOT_EMPTY (145) from `remove_dir_all` when a
/// child is briefly locked (thumbnail workers, Explorer, AV). Walk bottom-up,
/// clear read-only, and retry with backoff until the tree is gone.
pub(super) fn remove_directory_resilient(path: &Path) -> Result<(), String> {
    remove_directory_resilient_with_progress(path, &mut |_| Ok(()))
}

/// Same as [`remove_directory_resilient`], but invokes `on_entry` after each
/// file/directory is removed so the delete queue can show live progress.
pub(super) fn remove_directory_resilient_with_progress<F>(
    path: &Path,
    on_entry: &mut F,
) -> Result<(), String>
where
    F: FnMut(&Path) -> Result<(), String>,
{
    if !path.exists() {
        return Ok(());
    }

    const ATTEMPTS: u32 = 10;
    let mut last_error: Option<String> = None;

    for attempt in 0..ATTEMPTS {
        // Prefer a walk with progress. Fast `remove_dir_all` is only used when
        // no progress callback work is needed on the first attempt and the tree
        // is small — for queue UX we always walk so the UI can update.
        match remove_path_tree_with_progress(path, on_entry) {
            Ok(()) if !path.exists() => return Ok(()),
            Ok(()) => {
                last_error = Some(format!(
                    "Path still exists after delete attempt: {}",
                    path.display()
                ));
            }
            Err(error) => last_error = Some(error),
        }

        if !path.exists() {
            return Ok(());
        }

        std::thread::sleep(std::time::Duration::from_millis(
            40u64.saturating_mul(u64::from(attempt + 1)),
        ));
    }

    if !path.exists() {
        return Ok(());
    }

    Err(format!(
        "Failed to delete media directory '{}': {}. \
Close any open files from this folder (thumbnails, Explorer preview) and retry.",
        path.display(),
        last_error.unwrap_or_else(|| "unknown error".to_string())
    ))
}

fn clear_readonly_attribute(path: &Path) {
    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        if permissions.readonly() {
            permissions.set_readonly(false);
            let _ = fs::set_permissions(path, permissions);
        }
    }
}

fn remove_path_tree_with_progress<F>(path: &Path, on_entry: &mut F) -> Result<(), String>
where
    F: FnMut(&Path) -> Result<(), String>,
{
    if !path.exists() {
        return Ok(());
    }

    clear_readonly_attribute(path);
    if path.is_file() {
        fs::remove_file(path).map_err(|error| {
            format!("Failed to delete file '{}': {error}", path.display())
        })?;
        on_entry(path)?;
        return Ok(());
    }

    if path.is_dir() {
        let entries = fs::read_dir(path).map_err(|error| {
            format!(
                "Failed to list directory '{}' while deleting: {error}",
                path.display()
            )
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!(
                    "Failed to read entry under '{}' while deleting: {error}",
                    path.display()
                )
            })?;
            remove_path_tree_with_progress(&entry.path(), on_entry)?;
        }
        clear_readonly_attribute(path);
        match fs::remove_dir(path) {
            Ok(()) => {
                on_entry(path)?;
                Ok(())
            }
            Err(_) if !path.exists() => {
                on_entry(path)?;
                Ok(())
            }
            Err(error) => Err(format!(
                "Failed to remove directory '{}': {error}",
                path.display()
            )),
        }
    } else {
        clear_readonly_attribute(path);
        fs::remove_file(path)
            .or_else(|_| fs::remove_dir_all(path))
            .map_err(|error| format!("Failed to delete '{}': {error}", path.display()))?;
        on_entry(path)?;
        Ok(())
    }
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
        // YouTube handles are stored without the leading `@`, but the channel
        // URL requires it.
        "youtube" => format!("https://www.youtube.com/@{}", handle),
        "vsco" => format!("https://vsco.co/{}", handle),
        _ => handle.to_string(),
    }
}
/// Move o conteúdo de uma pasta para outra: tenta `rename` (rápido, mesmo
/// volume) e recorre a cópia recursiva + remoção quando o destino está em outro
/// volume. Os relative_path dos ledgers são relativos à pasta do perfil, então
/// mover a pasta mantém o histórico de downloads consistente.
pub(super) fn move_media_directory(from: &Path, to: &Path) -> Result<(), String> {
    move_media_directory_with_progress(from, to, &mut |_, _| Ok(()))
}

pub struct MediaMoveProgress {
    pub files_processed: u64,
    pub bytes_processed: u64,
    pub current_file: String,
}

pub fn move_media_directory_with_progress(
    from: &Path,
    to: &Path,
    progress: &mut dyn FnMut(MediaMoveProgress, bool) -> Result<(), String>,
) -> Result<(), String> {
    if to.exists() {
        return Err(format!(
            "Destino de mídia já existe: {}. Mova ou remova-o antes.",
            to.display()
        ));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    progress(
        MediaMoveProgress {
            files_processed: 0,
            bytes_processed: 0,
            current_file: String::new(),
        },
        true,
    )?;
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }

    let mut files_processed = 0;
    let mut bytes_processed = 0;
    copy_dir_recursive_with_progress(from, to, &mut |path, bytes| {
        files_processed += 1;
        bytes_processed += bytes;
        progress(
            MediaMoveProgress {
                files_processed,
                bytes_processed,
                current_file: path.display().to_string(),
            },
            false,
        )?;
        Ok(())
    })?;
    fs::remove_dir_all(from).map_err(|error| {
        format!(
            "Mídia copiada para '{}', mas falhou ao remover a pasta antiga '{}': {}",
            to.display(),
            from.display(),
            error
        )
    })
}

fn move_media_directory_for_migration(
    from: &Path,
    staging: &Path,
    to: &Path,
    progress: &mut dyn FnMut(MediaMoveProgress, bool) -> Result<(), String>,
) -> Result<(), String> {
    if to.exists() {
        return Err(format!("Destino de mídia já existe: {}", to.display()));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    progress(
        MediaMoveProgress {
            files_processed: 0,
            bytes_processed: 0,
            current_file: String::new(),
        },
        true,
    )?;
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }

    if staging.exists() {
        fs::remove_dir_all(staging).map_err(|error| error.to_string())?;
    }
    let mut files_processed = 0;
    let mut bytes_processed = 0;
    copy_dir_recursive_with_progress(from, staging, &mut |path, bytes| {
        files_processed += 1;
        bytes_processed += bytes;
        progress(
            MediaMoveProgress {
                files_processed,
                bytes_processed,
                current_file: path.display().to_string(),
            },
            false,
        )
    })?;
    progress(
        MediaMoveProgress {
            files_processed,
            bytes_processed,
            current_file: String::new(),
        },
        true,
    )?;
    fs::rename(staging, to).map_err(|error| {
        format!("Falha ao promover staging '{}': {}", staging.display(), error)
    })?;
    fs::remove_dir_all(from).map_err(|error| {
        format!(
            "Mídia movida para '{}', mas falhou ao remover a origem '{}': {}",
            to.display(),
            from.display(),
            error
        )
    })
}
fn copy_dir_recursive_with_progress(
    from: &Path,
    to: &Path,
    progress: &mut dyn FnMut(&Path, u64) -> Result<(), String>,
) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(from).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let source_path = entry.path();
        let target_path = to.join(entry.file_name());
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if file_type.is_dir() {
            copy_dir_recursive_with_progress(&source_path, &target_path, progress)?;
        } else {
            let bytes = fs::copy(&source_path, &target_path).map_err(|error| {
                format!("Falha ao copiar '{}': {}", source_path.display(), error)
            })?;
            progress(&source_path, bytes)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod media_move_progress_tests {
    use super::*;

    #[test]
    fn recursive_copy_reports_each_completed_file() {
        let root =
            std::env::temp_dir().join(format!("ninjacrawler-progress-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("one.bin"), [1_u8, 2, 3]).unwrap();
        fs::write(source.join("nested").join("two.bin"), [4_u8, 5]).unwrap();
        let mut updates = Vec::new();

        copy_dir_recursive_with_progress(&source, &target, &mut |path, bytes| {
            updates.push((
                path.file_name().unwrap().to_string_lossy().to_string(),
                bytes,
            ));
            Ok(())
        })
        .unwrap();

        assert_eq!(updates.len(), 2);
        assert_eq!(updates.iter().map(|(_, bytes)| bytes).sum::<u64>(), 5);
        assert_eq!(
            fs::read(target.join("nested").join("two.bin")).unwrap(),
            [4_u8, 5]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recursive_copy_stops_when_progress_requests_cancellation() {
        let root =
            std::env::temp_dir().join(format!("ninjacrawler-cancel-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("one.bin"), [1_u8]).unwrap();
        fs::write(source.join("two.bin"), [2_u8]).unwrap();
        let mut copied = 0;

        let result = copy_dir_recursive_with_progress(&source, &target, &mut |_, _| {
            copied += 1;
            Err("cancelled".to_string())
        });

        assert_eq!(result, Err("cancelled".to_string()));
        assert_eq!(copied, 1);
        assert!(source.exists(), "the original folder must be preserved");
        fs::remove_dir_all(root).unwrap();
    }

}
/// Changes the save path for one or more supported profiles to
/// `target_base_path/<handle>`, optionally moving already-downloaded media.
pub fn change_source_media_path(
    source_ids: Vec<String>,
    target_base_path: String,
    move_media: bool,
) -> Result<WorkspaceSnapshot, String> {
    change_source_media_path_internal(source_ids, target_base_path, move_media, None)
}

pub fn change_source_media_path_migration(
    source_id: String,
    target_base_path: String,
    job_id: &str,
    mut progress: impl FnMut(MediaMoveProgress, bool) -> Result<(), String>,
) -> Result<WorkspaceSnapshot, String> {
    change_source_media_path_internal(
        vec![source_id],
        target_base_path,
        true,
        Some((job_id, &mut progress)),
    )
}

fn change_source_media_path_internal(
    source_ids: Vec<String>,
    target_base_path: String,
    move_media: bool,
    mut migration: Option<(&str, &mut dyn FnMut(MediaMoveProgress, bool) -> Result<(), String>)>,
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
            let account_settings = source
                .account_id
                .as_ref()
                .map(|account_id| load_provider_account_settings_map(connection, account_id))
                .transpose()?;
            let old_root = resolved_source_media_output_root(layout, source, account_settings.as_ref());

            let folder = sanitize_path_segment(
                sanitize_source_handle(&source.provider, &source.handle).trim_start_matches('@'),
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

            if move_media && !same_physical_dir {
                if let Some((job_id, progress)) = migration.as_mut() {
                    // A staging directory is owned by this durable job. It makes a
                    // cross-volume copy resumable: on restart, promote the staging
                    // directory instead of mistaking it for a user's destination.
                    let staging_root = base.join(format!(".ninjacrawler-moving-{job_id}"));
                    if staging_root.exists() {
                        if new_root.exists() {
                            return Err(format!(
                                "Destino de mídia já existe: {}",
                                new_root.display()
                            ));
                        }
                        if old_root.exists() {
                            move_media_directory_for_migration(
                                &old_root,
                                &staging_root,
                                &new_root,
                                *progress,
                            )?;
                        } else {
                            // A legacy/recovered staging folder may already be the
                            // only complete copy. Promotion is atomic and must finish.
                            fs::rename(&staging_root, &new_root).map_err(|error| error.to_string())?;
                        }
                    } else if old_root.exists() {
                        move_media_directory_for_migration(
                            &old_root,
                            &staging_root,
                            &new_root,
                            *progress,
                        )?;
                    }
                } else if old_root.exists() {
                    move_media_directory(&old_root, &new_root)?;
                }
            }

            let mut sync_options = source.sync_options.clone();
            let special_path = Some(new_root.display().to_string());
            if source.provider.eq_ignore_ascii_case("instagram") {
                sync_options.instagram.get_or_insert_with(default_instagram_source_sync_options).special_path = special_path;
            } else if source.provider.eq_ignore_ascii_case("twitter") {
                sync_options.twitter.get_or_insert_with(default_twitter_source_sync_options).special_path = special_path;
            } else if source.provider.eq_ignore_ascii_case("tiktok") {
                sync_options.tiktok.get_or_insert_with(default_tiktok_source_sync_options).special_path = special_path;
            } else {
                return Err(format!("Changing the save path is not supported for {} profiles.", source.provider));
            }
            let serialized = serialize_source_sync_options(&source.provider, &sync_options)?;

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
/// Minimal immutable data needed by the asynchronous media-path worker.
pub fn media_path_migration_seed(source_id: String) -> Result<(String, String, String), String> {
    let snapshot = bootstrap_workspace()?;
    let source = snapshot
        .sources
        .into_iter()
        .find(|source| source.id == source_id)
        .ok_or_else(|| "Profile no longer exists.".to_string())?;
    if !matches!(source.provider.to_ascii_lowercase().as_str(), "instagram" | "twitter" | "tiktok") {
        return Err(format!("Changing the save path is not supported for {} profiles.", source.provider));
    }
    let source_path = snapshot
        .source_media_paths
        .get(&source.id)
        .cloned()
        .ok_or_else(|| "Could not resolve the current media path.".to_string())?;
    Ok((source.provider, source.handle, source_path))
}

pub fn persist_media_path_migration_job(
    job_id: &str,
    source_id: &str,
    target_base_path: &str,
    queued_at: &str,
) -> Result<(), String> {
    with_workspace(|connection, _| {
        connection.execute(
            "INSERT INTO media_path_migration_queue_jobs(job_id, source_id, target_base_path, queued_at, state)
             VALUES (?1, ?2, ?3, ?4, 'queued')
             ON CONFLICT(source_id) DO NOTHING",
            params![job_id, source_id, target_base_path, queued_at],
        ).map_err(|error| error.to_string())?;
        Ok(())
    })
}

pub fn load_media_path_migration_jobs() -> Result<Vec<(String, String, String, String)>, String> {
    with_workspace(|connection, _| {
        connection.execute("UPDATE media_path_migration_queue_jobs SET state = 'queued', started_at = NULL WHERE state = 'running'", [])
            .map_err(|error| error.to_string())?;
        let mut statement = connection.prepare(
            "SELECT job_id, source_id, target_base_path, queued_at FROM media_path_migration_queue_jobs WHERE state = 'queued' ORDER BY queued_at"
        ).map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(rows)
    })
}

pub fn set_media_path_migration_job_running(job_id: &str, started_at: &str) -> Result<(), String> {
    with_workspace(|connection, _| {
        connection.execute(
        "UPDATE media_path_migration_queue_jobs SET state = 'running', started_at = ?2 WHERE job_id = ?1",
        params![job_id, started_at],
    ).map(|_| ()).map_err(|error| error.to_string())
    })
}

pub fn remove_media_path_migration_job(job_id: &str) -> Result<(), String> {
    with_workspace(|connection, _| {
        connection
            .execute(
                "DELETE FROM media_path_migration_queue_jobs WHERE job_id = ?1",
                params![job_id],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    })
}
/// Resolves the absolute media output path for every profile using the same
/// precedence as sync. Setting read failures fall back to the provider's global
/// media root.
pub(super) fn compute_source_media_paths(
    connection: &Connection,
    layout: &StorageLayout,
    sources: &[SourceProfile],
) -> HashMap<String, String> {
    let mut account_settings_cache: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut result = HashMap::new();

    for source in sources {
        let settings = source.account_id.as_ref().map(|account_id| {
            account_settings_cache
                .entry(account_id.clone())
                .or_insert_with(|| {
                    load_provider_account_settings_map(connection, account_id).unwrap_or_default()
                })
                .clone()
        });

        let root = resolved_source_media_output_root(layout, source, settings.as_ref());
        // Use the on-disk spelling on case-insensitive Windows filesystems so the
        // filter merges variants such as `instagram` and `Instagram`.
        let display = canonicalized_media_root_display(&root);
        result.insert(source.id.clone(), display);
    }

    result
}

/// Cache do `canonicalize()` por root de mídia. `load_snapshot` (todo comando)
/// chamava um stat de disco por perfil do Instagram; o resultado é estável para
/// um dado root, então memoizamos. Só cacheia sucessos: se a pasta ainda não
/// existe, cai no display sem canonizar e tenta de novo numa próxima vez.
fn canonicalized_media_root_display(root: &Path) -> String {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(hit) = cache.lock().ok().and_then(|guard| guard.get(root).cloned()) {
        return hit;
    }
    match root.canonicalize() {
        Ok(canonical) => {
            let display = canonical
                .display()
                .to_string()
                .trim_start_matches(r"\\?\")
                .to_string();
            if let Ok(mut guard) = cache.lock() {
                guard.insert(root.to_path_buf(), display.clone());
            }
            display
        }
        Err(_) => root.display().to_string(),
    }
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
