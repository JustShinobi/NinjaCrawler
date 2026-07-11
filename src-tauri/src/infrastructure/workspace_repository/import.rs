use super::*;

use base64::Engine as _;

#[derive(Clone)]
pub(super) struct LegacyInstagramProfileXml {
    pub(super) account_name: Option<String>,
    pub(super) user_id: Option<String>,
    pub(super) user_name: Option<String>,
    pub(super) true_name: Option<String>,
    pub(super) friendly_name: Option<String>,
    pub(super) user_site_name: Option<String>,
    pub(super) description: Option<String>,
    pub(super) ready_for_download: bool,
    pub(super) get_timeline: bool,
    pub(super) get_reels: bool,
    pub(super) get_stories: bool,
    pub(super) get_stories_user: bool,
    pub(super) get_tagged_data: bool,
}
#[derive(Clone)]
pub(super) struct ImportCandidateProfile {
    pub(super) profile_root: PathBuf,
    pub(super) user_xml_path: PathBuf,
    pub(super) folder_name: String,
    pub(super) profile: LegacyInstagramProfileXml,
}
#[derive(Clone)]
pub(super) struct LegacyInstagramMediaXmlEntry {
    pub(super) file_name: String,
    pub(super) provider_post_key: String,
    pub(super) media_url: String,
    pub(super) special_folder: Option<String>,
    pub(super) post_permalink: Option<String>,
}
#[derive(Clone)]
pub(super) struct LegacyInstagramReconciliationRecord {
    pub(super) file_path: PathBuf,
    pub(super) legacy_file_name: String,
    pub(super) provider_media_key: String,
    pub(super) alias_keys: Vec<(String, String)>,
    pub(super) file_sha256: Option<String>,
    pub(super) provider_post_key: String,
    /// Normalized (lowercased) shortcode used for dedupe/aliases.
    pub(super) provider_post_code: Option<String>,
    /// Shortcode preserving original casing, used to rebuild the post URL
    /// (Instagram shortcodes are case-sensitive).
    pub(super) provider_post_code_cased: Option<String>,
    pub(super) media_type: String,
    pub(super) media_section: String,
}
#[derive(Default)]
pub(super) struct LegacyInstagramReconciliationStats {
    pub(super) seeded_media_entries: u32,
    pub(super) seeded_post_entries: u32,
}
pub(super) fn implicit_instagram_imported_cutoff_timestamp(
    source: &SourceProfile,
    run_mode: Option<&str>,
) -> Option<i64> {
    if run_mode.is_some_and(|value| value.eq_ignore_ascii_case("force_imported_backfill")) {
        return None;
    }

    if !source
        .importer_id
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case(INSTAGRAM_SCRAWLER_IMPORTER_ID))
    {
        return None;
    }

    source
        .imported_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.timestamp())
}
pub fn import_provider_account_cookies(
    input: ProviderAccountCookieImport,
) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        import_provider_account_cookies_with_connection(connection, layout, input)
    })
}
pub fn preview_companion_account(
    capture: CompanionAccountCapture,
) -> Result<CompanionAccountPreview, String> {
    with_workspace(|connection, layout| {
        preview_companion_account_with_connection(connection, layout, &capture)
    })
}
pub fn import_companion_account(
    input: CompanionAccountImportInput,
) -> Result<CompanionAccountImportResult, String> {
    with_workspace(|connection, layout| {
        import_companion_account_with_connection(connection, layout, input)
    })
}
pub fn revert_provider_account_import(account_id: String) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        revert_provider_account_import_with_connection(connection, layout, &account_id)
    })
}
pub fn list_import_providers() -> Result<Vec<ImportProviderDescriptor>, String> {
    Ok(vec![ImportProviderDescriptor {
        key: "instagram".to_string(),
        display_name: "Instagram".to_string(),
        description:
            "Import legacy SCrawler profile folders into NinjaCrawler sources and media catalog."
                .to_string(),
    }])
}
pub fn list_import_methods(provider: String) -> Result<Vec<ImportMethodDescriptor>, String> {
    if !provider.eq_ignore_ascii_case("instagram") {
        return Ok(Vec::new());
    }

    Ok(vec![ImportMethodDescriptor {
        importer_id: INSTAGRAM_SCRAWLER_IMPORTER_ID.to_string(),
        provider: "instagram".to_string(),
        label: "SCrawler".to_string(),
        description: "Scan SCrawler Instagram profile folders, preview matches, and import the media already on disk.".to_string(),
    }])
}
pub fn list_import_roots(
    importer_id: String,
    manual_roots: Vec<String>,
    disabled_roots: Vec<String>,
) -> Result<Vec<ImportRootDescriptor>, String> {
    with_workspace(|connection, layout| match importer_id.as_str() {
        INSTAGRAM_SCRAWLER_IMPORTER_ID => list_instagram_scrawler_import_roots_with_connection(
            connection,
            layout,
            &manual_roots,
            &disabled_roots,
        ),
        _ => Err(format!("Unsupported importer '{importer_id}'.")),
    })
}
pub fn preview_import_method(
    importer_id: String,
    options: ImportPreviewOptions,
) -> Result<ImportPreview, String> {
    with_workspace(|connection, layout| match importer_id.as_str() {
        INSTAGRAM_SCRAWLER_IMPORTER_ID => {
            preview_instagram_scrawler_import_with_connection(connection, layout, options)
        }
        _ => Err(format!("Unsupported importer '{importer_id}'.")),
    })
}
pub fn run_import_method(
    importer_id: String,
    input: ImportRunRequest,
) -> Result<ImportRunResult, String> {
    with_workspace(|connection, layout| match importer_id.as_str() {
        INSTAGRAM_SCRAWLER_IMPORTER_ID => {
            run_instagram_scrawler_import_with_connection(connection, layout, input)
        }
        _ => Err(format!("Unsupported importer '{importer_id}'.")),
    })
}
pub fn pick_import_root_folder() -> Result<Option<String>, String> {
    let initial_directory = storage::ensure_workspace_layout()
        .map(|layout| layout.media_root)
        .unwrap_or_else(|_| PathBuf::from("."));

    let picked = rfd::FileDialog::new()
        .set_title("Choose SCrawler import folder")
        .set_directory(initial_directory)
        .pick_folder();

    Ok(picked.map(|path| path.to_string_lossy().into_owned()))
}
pub(super) fn load_provider_account_import_state(
    connection: &Connection,
    account_id: &str,
) -> Result<Option<ProviderAccountImportState>, String> {
    connection
        .query_row(
            "SELECT account_id, provider_user_id, provider_username, last_imported_at,
                    backup_secret_ref, backup_imported_at
             FROM provider_account_import_state
             WHERE account_id = ?1",
            params![account_id],
            |row| {
                Ok(ProviderAccountImportState {
                    account_id: row.get(0)?,
                    provider_user_id: row.get(1)?,
                    provider_username: row.get(2)?,
                    last_imported_at: row.get(3)?,
                    can_revert: row.get::<_, Option<String>>(4)?.is_some(),
                    backup_imported_at: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())
}
pub(super) fn preview_instagram_scrawler_import_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    options: ImportPreviewOptions,
) -> Result<ImportPreview, String> {
    let roots = collect_instagram_import_roots(
        connection,
        layout,
        &options.manual_roots,
        &options.disabled_roots,
    )?;
    let root_labels = roots
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let accounts = load_accounts(connection)?;
    let sources = load_sources(connection)?;
    let imported_roots =
        load_external_imported_entity_keys(connection, INSTAGRAM_SCRAWLER_IMPORTER_ID)?;
    let candidates = collect_scrawler_instagram_candidates(&roots)?;
    let duplicate_handles = collect_duplicate_import_handles(&candidates);

    let profiles = candidates
        .iter()
        .map(|candidate| {
            build_instagram_scrawler_preview_profile(
                candidate,
                &accounts,
                &sources,
                &imported_roots,
                &duplicate_handles,
                options.force_reimport,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    let summary = ImportPreviewSummary {
        detected_profiles: profiles.len() as u32,
        ready_profiles: profiles
            .iter()
            .filter(|profile| profile.import_state == "ready")
            .count() as u32,
        blocked_profiles: profiles
            .iter()
            .filter(|profile| {
                matches!(
                    profile.import_state.as_str(),
                    "needs_account_link" | "duplicate_conflict" | "no_media"
                )
            })
            .count() as u32,
        already_imported_profiles: profiles
            .iter()
            .filter(|profile| profile.already_imported)
            .count() as u32,
        importable_files: profiles.iter().map(|profile| profile.file_count).sum(),
    };

    Ok(ImportPreview {
        importer_id: INSTAGRAM_SCRAWLER_IMPORTER_ID.to_string(),
        provider: "instagram".to_string(),
        method_label: "SCrawler".to_string(),
        force_reimport: options.force_reimport,
        roots: root_labels,
        profiles,
        summary,
    })
}
pub(super) fn list_instagram_scrawler_import_roots_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    manual_roots: &[String],
    disabled_roots: &[String],
) -> Result<Vec<ImportRootDescriptor>, String> {
    collect_instagram_import_root_descriptors(connection, layout, manual_roots, disabled_roots)
}
pub(super) fn merge_import_root_descriptors(
    existing: &mut ImportRootDescriptor,
    incoming: ImportRootDescriptor,
) {
    if existing.source == "manual" && incoming.source != "manual" {
        existing.source = incoming.source;
        existing.label = incoming.label;
        existing.removable = incoming.removable;
    }
}
pub(super) fn run_instagram_scrawler_import_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    input: ImportRunRequest,
) -> Result<ImportRunResult, String> {
    let preview = preview_instagram_scrawler_import_with_connection(
        connection,
        layout,
        ImportPreviewOptions {
            force_reimport: input.force_reimport,
            manual_roots: input.manual_roots.clone(),
            disabled_roots: input.disabled_roots.clone(),
        },
    )?;
    let roots = collect_instagram_import_roots(
        connection,
        layout,
        &input.manual_roots,
        &input.disabled_roots,
    )?;
    let candidates = collect_scrawler_instagram_candidates(&roots)?;
    let candidate_by_root = candidates
        .into_iter()
        .map(|candidate| {
            (
                normalize_import_entity_key(&candidate.profile_root),
                candidate,
            )
        })
        .collect::<HashMap<_, _>>();
    let resolution_by_root = input
        .resolutions
        .into_iter()
        .map(|resolution| {
            (
                normalize_import_entity_key(Path::new(&resolution.profile_root)),
                resolution,
            )
        })
        .collect::<HashMap<_, _>>();

    let mut imported_profiles = 0u32;
    let mut skipped_profiles = 0u32;
    let mut failed_profiles = 0u32;
    let mut imported_media_count = 0u32;
    let mut already_cataloged_count = 0u32;
    let mut profiles = Vec::new();

    for preview_profile in &preview.profiles {
        let profile_key = normalize_import_entity_key(Path::new(&preview_profile.profile_root));
        let resolution = resolution_by_root.get(&profile_key);
        let action = resolution
            .map(|entry| entry.action.as_str())
            .unwrap_or_else(|| {
                if preview_profile.import_state == "already_imported" && !input.force_reimport {
                    "skip"
                } else {
                    "import"
                }
            });

        if action.eq_ignore_ascii_case("skip") {
            skipped_profiles += 1;
            profiles.push(ImportRunProfileResult {
                profile_root: preview_profile.profile_root.clone(),
                handle: preview_profile.handle.clone(),
                status: "skipped".to_string(),
                source_id: preview_profile.source_id.clone(),
                imported_media_count: 0,
                already_cataloged_count: 0,
                message: "Skipped by import review.".to_string(),
            });
            continue;
        }

        let resolved_account_id = resolution
            .and_then(|entry| entry.account_id.clone())
            .or_else(|| preview_profile.account_id.clone());

        let can_proceed = match preview_profile.import_state.as_str() {
            "ready" | "already_imported" => true,
            "needs_account_link" => resolved_account_id.is_some(),
            _ => false,
        };

        if !can_proceed {
            failed_profiles += 1;
            profiles.push(ImportRunProfileResult {
                profile_root: preview_profile.profile_root.clone(),
                handle: preview_profile.handle.clone(),
                status: "failed".to_string(),
                source_id: preview_profile.source_id.clone(),
                imported_media_count: 0,
                already_cataloged_count: 0,
                message: format!(
                    "Profile is blocked in review state '{}'.",
                    preview_profile.import_state
                ),
            });
            continue;
        }

        if preview_profile.import_state == "already_imported" && !input.force_reimport {
            skipped_profiles += 1;
            profiles.push(ImportRunProfileResult {
                profile_root: preview_profile.profile_root.clone(),
                handle: preview_profile.handle.clone(),
                status: "skipped".to_string(),
                source_id: preview_profile.source_id.clone(),
                imported_media_count: 0,
                already_cataloged_count: 0,
                message:
                    "Profile was already imported. Enable force re-import to process it again."
                        .to_string(),
            });
            continue;
        }

        let Some(candidate) = candidate_by_root.get(&profile_key) else {
            failed_profiles += 1;
            profiles.push(ImportRunProfileResult {
                profile_root: preview_profile.profile_root.clone(),
                handle: preview_profile.handle.clone(),
                status: "failed".to_string(),
                source_id: preview_profile.source_id.clone(),
                imported_media_count: 0,
                already_cataloged_count: 0,
                message: "The legacy profile root is no longer available.".to_string(),
            });
            continue;
        };

        match import_instagram_scrawler_profile_with_connection(
            connection,
            layout,
            candidate,
            preview_profile,
            resolved_account_id,
            input.force_reimport,
        ) {
            Ok(result) => {
                imported_profiles += 1;
                imported_media_count += result.imported_media_count;
                already_cataloged_count += result.already_cataloged_count;
                profiles.push(result);
            }
            Err(error) => {
                failed_profiles += 1;
                profiles.push(ImportRunProfileResult {
                    profile_root: preview_profile.profile_root.clone(),
                    handle: preview_profile.handle.clone(),
                    status: "failed".to_string(),
                    source_id: preview_profile.source_id.clone(),
                    imported_media_count: 0,
                    already_cataloged_count: 0,
                    message: error,
                });
            }
        }
    }

    Ok(ImportRunResult {
        importer_id: INSTAGRAM_SCRAWLER_IMPORTER_ID.to_string(),
        imported_profiles,
        skipped_profiles,
        failed_profiles,
        imported_media_count,
        already_cataloged_count,
        profiles,
    })
}
pub(super) fn import_instagram_scrawler_profile_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    candidate: &ImportCandidateProfile,
    preview_profile: &ImportPreviewProfile,
    resolved_account_id: Option<String>,
    force_reimport: bool,
) -> Result<ImportRunProfileResult, String> {
    connection
        .execute("BEGIN IMMEDIATE TRANSACTION", [])
        .map_err(|error| error.to_string())?;

    let operation = (|| -> Result<ImportRunProfileResult, String> {
        let source = if let Some(source_id) = preview_profile.source_id.as_deref() {
            load_sources(connection)?
                .into_iter()
                .find(|entry| entry.id == source_id)
                .ok_or_else(|| format!("Source '{}' is no longer available.", source_id))?
        } else {
            let account_id = resolved_account_id.ok_or_else(|| {
                "This profile requires an explicit Instagram account link before import."
                    .to_string()
            })?;
            let source_id = new_id();
            let handle =
                legacy_instagram_profile_handle(&candidate.profile, &candidate.folder_name);
            let display_name = legacy_instagram_profile_display_name(&candidate.profile, &handle);
            let mut instagram_sync_options = default_instagram_source_sync_options();
            instagram_sync_options.timeline = candidate.profile.get_timeline;
            instagram_sync_options.reels = candidate.profile.get_reels;
            instagram_sync_options.stories = candidate.profile.get_stories;
            instagram_sync_options.stories_user = candidate.profile.get_stories_user;
            instagram_sync_options.tagged = candidate.profile.get_tagged_data;
            // Always store the absolute profile root as special_path for imported profiles
            // so media resolution works regardless of the storage.media_root setting.
            instagram_sync_options.special_path =
                Some(candidate.profile_root.display().to_string());
            instagram_sync_options.user_id_hint = candidate.profile.user_id.clone();
            // O SCrawler guarda o nome legado em UserName e o nome atual em
            // TrueName (usado como handle). Quando diferem, registra o UserName
            // como nome anterior para que a busca encontre pelo nome antigo.
            if let Some(legacy_username) = candidate.profile.user_name.as_deref() {
                instagram_sync_options.previous_handles = push_previous_instagram_handle(
                    instagram_sync_options.previous_handles.take(),
                    legacy_username,
                    &handle,
                );
            }
            let sync_options = SourceSyncOptions {
                instagram: Some(instagram_sync_options),
                ..Default::default()
            };

            let _ = upsert_source_profile_with_connection(
                connection,
                layout,
                SourceProfileUpsert {
                    id: Some(source_id.clone()),
                    provider: "instagram".to_string(),
                    source_kind: "profile".to_string(),
                    handle,
                    display_name,
                    account_id: Some(account_id),
                    group_id: None,
                    labels: Vec::new(),
                    ready_for_download: candidate.profile.ready_for_download,
                    sync_options,
                    remote_state: None,
                    is_subscription: None,
                },
            )?;

            let source = load_sources(connection)?
                .into_iter()
                .find(|entry| entry.id == source_id)
                .ok_or_else(|| "Imported source was not persisted.".to_string())?;

            if !source.profile_image_custom {
                let avatar_path = candidate.profile_root.join(PROFILE_PICTURE_FILE_NAME);
                if avatar_path.exists() {
                    let normalized_avatar_path = normalize_media_file_path(&avatar_path)?;
                    update_source_profile_image(
                        connection,
                        &source.id,
                        &normalized_avatar_path,
                        &now_timestamp(),
                    )?;
                }
            }

            source
        };

        let imported_at = now_timestamp();
        if let Some(profile_description) = candidate.profile.description.as_deref() {
            update_instagram_source_description_after_sync(
                connection,
                &source,
                profile_description,
                false,
                &imported_at,
            )?;
        }

        let importable_media = collect_legacy_instagram_media_candidates(&candidate.profile_root)?;
        if importable_media.is_empty() {
            return Err(
                "No importable image or video files were found in this legacy profile.".to_string(),
            );
        }

        if preview_profile.import_state == "already_imported" && !force_reimport {
            return Err(
                "This legacy profile was already imported. Enable force re-import to run it again."
                    .to_string(),
            );
        }

        let file_count = importable_media.len() as u32;
        let (imported_count, already_count) = if preview_profile.already_imported {
            (0, file_count)
        } else {
            (file_count, 0)
        };
        let account_id = source.account_id.clone().ok_or_else(|| {
            "Imported source is missing its explicit provider account binding.".to_string()
        })?;
        record_external_import_ledger(
            connection,
            ExternalImportLedgerRecord {
                importer_id: INSTAGRAM_SCRAWLER_IMPORTER_ID,
                profile_root: &candidate.profile_root,
                provider: "instagram",
                handle: &source.handle,
                source_id: &source.id,
                account_id: &account_id,
                timestamp: &imported_at,
            },
        )?;
        let reconciliation = reconcile_instagram_scrawler_profile_ledgers_with_connection(
            connection,
            &candidate.profile_root,
            &source.id,
            &account_id,
            &source.handle,
            &imported_at,
        )?;

        Ok(ImportRunProfileResult {
            profile_root: candidate.profile_root.to_string_lossy().into_owned(),
            handle: source.handle.clone(),
            status: "imported".to_string(),
            source_id: Some(source.id.clone()),
            imported_media_count: imported_count,
            already_cataloged_count: already_count + reconciliation.seeded_post_entries,
            message: if imported_count > 0 {
                format!(
                    "Imported {} file(s); {} file(s) were already present; reconciled {} media entrie(s) and {} post entrie(s).",
                    imported_count,
                    already_count,
                    reconciliation.seeded_media_entries,
                    reconciliation.seeded_post_entries
                )
            } else {
                format!(
                    "No new files were added; {} file(s) were already present; reconciled {} media entrie(s) and {} post entrie(s).",
                    already_count,
                    reconciliation.seeded_media_entries,
                    reconciliation.seeded_post_entries
                )
            },
        })
    })();

    match operation {
        Ok(result) => {
            connection
                .execute("COMMIT", [])
                .map_err(|error| error.to_string())?;
            Ok(result)
        }
        Err(error) => {
            let _ = connection.execute("ROLLBACK", []);
            Err(error)
        }
    }
}
pub(super) fn build_instagram_scrawler_preview_profile(
    candidate: &ImportCandidateProfile,
    accounts: &[ProviderAccount],
    sources: &[SourceProfile],
    imported_roots: &HashSet<String>,
    duplicate_handles: &HashSet<String>,
    force_reimport: bool,
) -> Result<ImportPreviewProfile, String> {
    let handle = legacy_instagram_profile_handle(&candidate.profile, &candidate.folder_name);
    let display_name = legacy_instagram_profile_display_name(&candidate.profile, &handle);
    let profile_root_key = normalize_import_entity_key(&candidate.profile_root);
    let already_imported = imported_roots.contains(&profile_root_key);
    let source_match = sources.iter().find(|source| {
        source.provider.eq_ignore_ascii_case("instagram")
            && source.handle.eq_ignore_ascii_case(&handle)
    });
    let account_matches = accounts
        .iter()
        .filter(|account| {
            account.provider.eq_ignore_ascii_case("instagram")
                && candidate
                    .profile
                    .account_name
                    .as_deref()
                    .is_some_and(|account_name| {
                        account.display_name.eq_ignore_ascii_case(account_name)
                    })
        })
        .collect::<Vec<_>>();
    let resolved_account = if source_match.is_none() && account_matches.len() == 1 {
        account_matches.first().copied()
    } else {
        None
    };
    let media_candidates = collect_legacy_instagram_media_candidates(&candidate.profile_root)?;
    let file_count = media_candidates.len() as u32;
    let already_cataloged_count = if already_imported && !force_reimport {
        file_count
    } else {
        0
    };
    let new_file_count = file_count.saturating_sub(already_cataloged_count);
    let avatar_path = candidate.profile_root.join(PROFILE_PICTURE_FILE_NAME);
    let duplicate_handle = duplicate_handles.contains(&handle.to_ascii_lowercase());
    let mut problems = Vec::new();

    if sanitize_source_handle("instagram", &candidate.folder_name)
        != sanitize_source_handle("instagram", &handle)
    {
        problems.push(ImportProblem {
            severity: "warning".to_string(),
            code: "folder-handle-mismatch".to_string(),
            message: format!(
                "Folder '{}' differs from the XML handle '{}'. The XML handle will be used.",
                candidate.folder_name, handle
            ),
        });
    }

    if duplicate_handle {
        problems.push(ImportProblem {
            severity: "error".to_string(),
            code: "duplicate-handle".to_string(),
            message: format!(
                "More than one legacy folder resolved to the Instagram handle '{}'. Mark one of them as skip before importing.",
                handle
            ),
        });
    }

    if file_count == 0 {
        problems.push(ImportProblem {
            severity: "error".to_string(),
            code: "no-media".to_string(),
            message: "No importable image or video files were found in this legacy profile root."
                .to_string(),
        });
    }

    if source_match.is_none() && resolved_account.is_none() {
        match account_matches.len() {
            0 => problems.push(ImportProblem {
                severity: "error".to_string(),
                code: "account-match-missing".to_string(),
                message: if let Some(account_name) = candidate.profile.account_name.as_deref() {
                    format!(
                        "No Instagram account named '{}' was found. Link an account manually to import this folder.",
                        account_name
                    )
                } else {
                    "The legacy XML does not expose a usable AccountName. Link an account manually to import this folder.".to_string()
                },
            }),
            _ => problems.push(ImportProblem {
                severity: "error".to_string(),
                code: "account-match-ambiguous".to_string(),
                message: "More than one Instagram account matched the legacy AccountName. Choose the target account manually.".to_string(),
            }),
        }
    }

    if already_imported && !force_reimport {
        problems.push(ImportProblem {
            severity: "warning".to_string(),
            code: "already-imported".to_string(),
            message: "This legacy folder was already imported. Enable force re-import to process it again.".to_string(),
        });
    }

    let import_state = if duplicate_handle {
        "duplicate_conflict"
    } else if file_count == 0 {
        "no_media"
    } else if already_imported && !force_reimport {
        "already_imported"
    } else if source_match.is_none() && resolved_account.is_none() {
        "needs_account_link"
    } else {
        "ready"
    };

    Ok(ImportPreviewProfile {
        profile_root: candidate.profile_root.to_string_lossy().into_owned(),
        user_xml_path: candidate.user_xml_path.to_string_lossy().into_owned(),
        handle,
        display_name,
        account_name: candidate.profile.account_name.clone(),
        source_id: source_match.map(|source| source.id.clone()),
        source_display_name: source_match.map(|source| source.display_name.clone()),
        source_handle: source_match.map(|source| source.handle.clone()),
        account_id: source_match
            .and_then(|source| source.account_id.clone())
            .or_else(|| resolved_account.map(|account| account.id.clone())),
        account_display_name: source_match
            .and_then(|source| source.account_id.as_ref())
            .and_then(|source_account_id| {
                accounts
                    .iter()
                    .find(|account| account.id == *source_account_id)
                    .map(|account| account.display_name.clone())
            })
            .or_else(|| resolved_account.map(|account| account.display_name.clone())),
        avatar_path: avatar_path
            .exists()
            .then(|| avatar_path.to_string_lossy().into_owned()),
        already_imported,
        import_state: import_state.to_string(),
        file_count,
        already_cataloged_count,
        new_file_count,
        problems,
    })
}
pub(super) fn collect_instagram_import_root_descriptors(
    connection: &Connection,
    layout: &StorageLayout,
    manual_roots: &[String],
    disabled_roots: &[String],
) -> Result<Vec<ImportRootDescriptor>, String> {
    let mut roots = Vec::new();
    roots.push(ImportRootDescriptor {
        path: layout
            .media_root
            .join("instagram")
            .to_string_lossy()
            .into_owned(),
        source: "default".to_string(),
        label: "Media root".to_string(),
        removable: false,
    });

    for account in load_accounts(connection)?
        .into_iter()
        .filter(|entry| entry.provider.eq_ignore_ascii_case("instagram"))
    {
        let settings = load_provider_account_settings_map(connection, &account.id)?;
        if let Some(media_path) = setting_value(&settings, "instagram.account.mediaPath") {
            roots.push(ImportRootDescriptor {
                path: media_path,
                source: "account".to_string(),
                label: format!("Account path · {}", account.display_name),
                removable: false,
            });
        }
    }

    for manual_root in manual_roots {
        let trimmed = manual_root.trim();
        if !trimmed.is_empty() {
            roots.push(ImportRootDescriptor {
                path: trimmed.to_string(),
                source: "manual".to_string(),
                label: "Manual root".to_string(),
                removable: true,
            });
        }
    }

    let mut unique_roots = HashMap::new();
    for root in roots {
        let key = normalize_import_entity_key(Path::new(&root.path));
        match unique_roots.entry(key) {
            std::collections::hash_map::Entry::Occupied(mut occupied) => {
                merge_import_root_descriptors(occupied.get_mut(), root);
            }
            std::collections::hash_map::Entry::Vacant(vacant) => {
                vacant.insert(root);
            }
        }
    }

    let disabled_keys = disabled_roots
        .iter()
        .filter_map(|root| {
            let trimmed = root.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(normalize_import_entity_key(Path::new(trimmed)))
            }
        })
        .collect::<HashSet<_>>();

    let mut deduped = unique_roots
        .into_iter()
        .filter_map(|(key, root)| (!disabled_keys.contains(&key)).then_some(root))
        .collect::<Vec<_>>();
    deduped.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(deduped)
}
pub(super) fn collect_instagram_import_roots(
    connection: &Connection,
    layout: &StorageLayout,
    manual_roots: &[String],
    disabled_roots: &[String],
) -> Result<Vec<PathBuf>, String> {
    collect_instagram_import_root_descriptors(connection, layout, manual_roots, disabled_roots).map(
        |roots| {
            roots
                .into_iter()
                .map(|entry| PathBuf::from(entry.path))
                .collect::<Vec<_>>()
        },
    )
}
pub(super) fn collect_scrawler_instagram_candidates(
    roots: &[PathBuf],
) -> Result<Vec<ImportCandidateProfile>, String> {
    let mut candidates = HashMap::new();

    for root in roots {
        if !root.exists() {
            continue;
        }

        let mut pending = vec![root.clone()];
        while let Some(current) = pending.pop() {
            if let Some(user_xml_path) = find_user_instagram_xml(&current)? {
                let key = normalize_import_entity_key(&current);
                candidates.entry(key).or_insert(ImportCandidateProfile {
                    profile_root: current.clone(),
                    user_xml_path: user_xml_path.clone(),
                    folder_name: current
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                        .to_string(),
                    profile: parse_legacy_instagram_profile_xml(&user_xml_path)?,
                });
            }

            for entry in fs::read_dir(&current).map_err(|error| error.to_string())? {
                let entry = entry.map_err(|error| error.to_string())?;
                let path = entry.path();
                if entry
                    .file_type()
                    .map_err(|error| error.to_string())?
                    .is_dir()
                {
                    pending.push(path);
                }
            }
        }
    }

    let mut collected = candidates.into_values().collect::<Vec<_>>();
    collected.sort_by(|left, right| {
        left.profile_root
            .to_string_lossy()
            .cmp(&right.profile_root.to_string_lossy())
    });
    Ok(collected)
}
pub(super) fn parse_legacy_instagram_profile_xml(
    path: &Path,
) -> Result<LegacyInstagramProfileXml, String> {
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let document = roxmltree::Document::parse(&raw)
        .map_err(|error| format!("Failed to parse '{}': {error}", path.display()))?;

    Ok(LegacyInstagramProfileXml {
        account_name: xml_text(&document, "AccountName"),
        user_id: xml_text(&document, "UserID"),
        user_name: xml_text(&document, "UserName"),
        true_name: xml_text(&document, "TrueName"),
        friendly_name: xml_text(&document, "FriendlyName"),
        user_site_name: xml_text(&document, "UserSiteName"),
        description: xml_text(&document, "Description"),
        ready_for_download: xml_bool(&document, "ReadyForDownload"),
        get_timeline: xml_bool(&document, "GetTimeline"),
        get_reels: xml_bool(&document, "GetReels"),
        get_stories: xml_bool(&document, "GetStories"),
        get_stories_user: xml_bool(&document, "GetStoriesUser"),
        get_tagged_data: xml_bool(&document, "GetTaggedData"),
    })
}
pub(super) fn parse_legacy_instagram_data_xml(
    path: &Path,
) -> Result<Vec<LegacyInstagramMediaXmlEntry>, String> {
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let document = roxmltree::Document::parse(&raw)
        .map_err(|error| format!("Failed to parse '{}': {error}", path.display()))?;

    let mut entries = Vec::new();
    for node in document
        .descendants()
        .filter(|node| node.has_tag_name("MediaData"))
    {
        let file_name = node
            .attribute("File")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        let provider_post_key = node
            .attribute("ID")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());

        let (Some(file_name), Some(provider_post_key)) = (file_name, provider_post_key) else {
            continue;
        };

        entries.push(LegacyInstagramMediaXmlEntry {
            file_name,
            provider_post_key,
            media_url: node
                .attribute("URL")
                .map(str::trim)
                .unwrap_or_default()
                .to_string(),
            special_folder: node
                .attribute("SpecialFolder")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string()),
            post_permalink: node
                .text()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string()),
        });
    }

    Ok(entries)
}
pub(super) fn normalize_legacy_instagram_relative_path(value: &str) -> String {
    value
        .replace('\\', "/")
        .trim()
        .trim_matches('/')
        .to_ascii_lowercase()
}
pub(super) fn legacy_instagram_candidate_relative_path(
    file_name: &str,
    special_folder: Option<&str>,
) -> String {
    let normalized_file_name = file_name.trim().replace('\\', "/");
    let normalized_file_name = normalized_file_name.trim_matches('/').to_ascii_lowercase();
    let normalized_folder = special_folder
        .map(normalize_legacy_instagram_relative_path)
        .unwrap_or_default();

    if normalized_folder.is_empty() {
        normalized_file_name
    } else {
        format!("{normalized_folder}/{normalized_file_name}")
    }
}
pub(super) fn legacy_instagram_post_permalink(entry: &LegacyInstagramMediaXmlEntry) -> &str {
    entry
        .post_permalink
        .as_deref()
        .filter(|value| value.contains("instagram.com/"))
        .unwrap_or(&entry.media_url)
}
/// Lightweight relative-path → (cased shortcode, section) map read straight from
/// the legacy SCrawler `User_Instagram*_Data.xml`, used by ProfileView to rebuild
/// post links for media imported BEFORE the shortcode was persisted in the media
/// ledger. Unlike full reconciliation this skips per-file hashing/IO, so it is
/// cheap enough to run on gallery load. Keys are lowercased to match the gallery.
pub(super) fn load_legacy_instagram_post_codes(
    profile_root: &Path,
) -> HashMap<String, (Option<String>, Option<String>)> {
    let mut map = HashMap::new();
    let Ok(Some(data_xml_path)) = find_user_instagram_data_xml(profile_root) else {
        return map;
    };
    let Ok(entries) = parse_legacy_instagram_data_xml(&data_xml_path) else {
        return map;
    };
    for entry in entries {
        let relative_path = legacy_instagram_candidate_relative_path(
            &entry.file_name,
            entry.special_folder.as_deref(),
        );
        if relative_path.is_empty() {
            continue;
        }
        let permalink = legacy_instagram_post_permalink(&entry);
        let post_code = extract_instagram_post_code_from_permalink_cased(permalink);
        let section = Some(infer_legacy_instagram_media_section(
            entry.special_folder.as_deref(),
            permalink,
            Some(entry.media_url.as_str()),
        ));
        map.entry(relative_path).or_insert((post_code, section));
    }
    map
}
/// `media_key -> tweet status id` read from the legacy SCrawler Twitter XML.
/// Twitter file names never carry the status id (only the media key), so this is
/// the only local source of the post link for media imported before the status
/// id was persisted in the media ledger. Cheap (single XML parse, no file IO).
pub(super) fn load_legacy_twitter_post_keys(profile_root: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(Some(data_xml_path)) = find_user_twitter_data_xml(profile_root) else {
        return map;
    };
    let Ok(entries) = parse_legacy_instagram_data_xml(&data_xml_path) else {
        return map;
    };
    for entry in entries {
        let status_id = entry.provider_post_key.trim();
        if status_id.is_empty() || !status_id.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if let Some(media_key) = twitter_media_key_from_file_name(&entry.file_name) {
            map.entry(media_key)
                .or_insert_with(|| status_id.to_string());
        }
    }
    map
}
pub(super) fn infer_legacy_instagram_media_section(
    special_folder: Option<&str>,
    permalink: &str,
    media_url: Option<&str>,
) -> String {
    let normalized_folder = special_folder
        .map(normalize_legacy_instagram_relative_path)
        .unwrap_or_default();
    let normalized_permalink = permalink.trim().to_ascii_lowercase();

    if normalized_folder.starts_with("stories (user)") {
        "stories_user".to_string()
    } else if normalized_folder.starts_with("stories") {
        "stories".to_string()
    } else if normalized_folder.contains("tag") {
        "tagged".to_string()
    } else if normalized_folder.contains("reel")
        || normalized_permalink.contains("/reel/")
        // SCrawler stores reels with inconsistent permalinks: some as
        // `/reel/<code>`, others as `/p/<code>` (which also opens reels).
        // The reliable signal is the video CDN URL, whose `xpv_encode_tag`
        // (inside the base64 `efg` query param) carries `INSTAGRAM.CLIPS` for
        // reels and `INSTAGRAM.FEED`/`STORY` for the rest.
        || media_url.is_some_and(legacy_media_url_is_clip)
    {
        "reels".to_string()
    } else {
        "timeline".to_string()
    }
}

/// `true` when the SCrawler media URL belongs to a reel, detected via the
/// `xpv_encode_tag` (`…INSTAGRAM.CLIPS…`) embedded — as base64 — in the `efg`
/// query param of the CDN URL. Feed videos carry `INSTAGRAM.FEED` and stories
/// `INSTAGRAM.STORY`; images have no `efg`.
fn legacy_media_url_is_clip(media_url: &str) -> bool {
    let Some(efg) = extract_url_query_param(media_url, "efg") else {
        return false;
    };
    let decoded_param = percent_decode_ascii(&efg);
    // Strip the padding so both `…=` and the padding-less form decode.
    let unpadded = decoded_param.trim_end_matches('=');
    let Ok(bytes) = base64::engine::general_purpose::STANDARD_NO_PAD.decode(unpadded) else {
        return false;
    };
    let decoded = String::from_utf8_lossy(&bytes);
    decoded.contains(".CLIPS.")
}

/// Extracts the raw value of a query parameter (`name=<value>`) from a URL
/// without a full parser. Returns the slice up to the next `&`.
fn extract_url_query_param(url: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=");
    let start = url.find(&needle)? + needle.len();
    let rest = &url[start..];
    let end = rest.find('&').unwrap_or(rest.len());
    let value = &rest[..end];
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// Decodes percent-encoded (`%XX`) sequences of an ASCII string. Invalid or
/// truncated bytes are kept as-is (best-effort).
fn percent_decode_ascii(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hi = (bytes[index + 1] as char).to_digit(16);
            let lo = (bytes[index + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
pub(super) fn collect_legacy_instagram_reconciliation_records(
    profile_root: &Path,
) -> Result<Vec<LegacyInstagramReconciliationRecord>, String> {
    let Some(data_xml_path) = find_user_instagram_data_xml(profile_root)? else {
        return Ok(Vec::new());
    };

    let entries = parse_legacy_instagram_data_xml(&data_xml_path)?;
    if entries.is_empty() {
        return Ok(Vec::new());
    }

    let files = collect_legacy_instagram_media_candidates(profile_root)?;
    let mut files_by_relative_path = HashMap::<String, PathBuf>::new();
    let mut files_by_name = HashMap::<String, Vec<String>>::new();
    for path in files {
        let relative_path = normalize_instagram_relative_media_path(profile_root, &path);
        files_by_relative_path.insert(relative_path.clone(), path);

        if let Some(file_name) = Path::new(&relative_path)
            .file_name()
            .and_then(|value| value.to_str())
        {
            files_by_name
                .entry(file_name.to_ascii_lowercase())
                .or_default()
                .push(relative_path);
        }
    }

    let mut reconciled = Vec::new();
    for entry in entries {
        let direct_relative_path = legacy_instagram_candidate_relative_path(
            &entry.file_name,
            entry.special_folder.as_deref(),
        );
        let resolved_relative_path = if files_by_relative_path.contains_key(&direct_relative_path) {
            Some(direct_relative_path)
        } else {
            let file_name_key = Path::new(&entry.file_name)
                .file_name()
                .and_then(|value| value.to_str())
                .map(|value| value.to_ascii_lowercase());
            let Some(file_name_key) = file_name_key else {
                continue;
            };

            let Some(relative_matches) = files_by_name.get(&file_name_key) else {
                continue;
            };

            if let Some(special_folder) = entry.special_folder.as_deref() {
                let expected_prefix = normalize_legacy_instagram_relative_path(special_folder);
                relative_matches
                    .iter()
                    .find(|relative_path| {
                        relative_path.starts_with(&format!("{expected_prefix}/"))
                            || Path::new(relative_path)
                                .parent()
                                .and_then(|value| value.to_str())
                                .is_some_and(|parent| parent.eq_ignore_ascii_case(&expected_prefix))
                    })
                    .cloned()
            } else if relative_matches.len() == 1 {
                relative_matches.first().cloned()
            } else {
                None
            }
        };

        let Some(resolved_relative_path) = resolved_relative_path else {
            continue;
        };
        let Some(file_path) = files_by_relative_path.get(&resolved_relative_path).cloned() else {
            continue;
        };
        let Some(provider_media_key) = derive_instagram_media_identity_key_from_path(&file_path)
        else {
            continue;
        };
        let Some(media_type) = infer_media_type(&file_path) else {
            continue;
        };

        let permalink = legacy_instagram_post_permalink(&entry);
        let provider_post_key = normalize_instagram_post_ledger_key(&entry.provider_post_key);
        let provider_post_code = extract_instagram_post_code_from_permalink(permalink);
        let provider_post_code_cased = extract_instagram_post_code_from_permalink_cased(permalink);
        let mut alias_keys = Vec::new();
        alias_keys.push((provider_media_key.clone(), "legacy_file_path".to_string()));
        for candidate in
            extract_instagram_media_identity_candidates_from_file_name(&entry.file_name)
        {
            alias_keys.push((candidate, "legacy_xml_file_name".to_string()));
        }
        if let Some(raw_file_name) = entry.media_url.split('?').next() {
            for candidate in
                extract_instagram_media_identity_candidates_from_file_name(raw_file_name)
            {
                alias_keys.push((candidate, "legacy_media_url".to_string()));
            }
        }
        if !provider_post_key.is_empty() {
            alias_keys.push((provider_post_key.clone(), "legacy_post_id".to_string()));
        }
        if let Some(provider_post_code) = provider_post_code.clone() {
            alias_keys.push((provider_post_code, "legacy_post_code".to_string()));
        }
        let file_sha256 = compute_file_sha256(&file_path).ok();

        reconciled.push(LegacyInstagramReconciliationRecord {
            file_path,
            legacy_file_name: entry.file_name.clone(),
            provider_media_key,
            alias_keys,
            file_sha256,
            provider_post_key,
            provider_post_code,
            provider_post_code_cased,
            media_type: media_type.to_string(),
            media_section: infer_legacy_instagram_media_section(
                entry.special_folder.as_deref(),
                permalink,
                Some(entry.media_url.as_str()),
            ),
        });
    }

    Ok(reconciled)
}
pub(super) fn collect_duplicate_import_handles(
    candidates: &[ImportCandidateProfile],
) -> HashSet<String> {
    let mut seen = HashSet::new();
    let mut duplicates = HashSet::new();

    for candidate in candidates {
        let handle = legacy_instagram_profile_handle(&candidate.profile, &candidate.folder_name)
            .to_ascii_lowercase();

        if !seen.insert(handle.clone()) {
            duplicates.insert(handle);
        }
    }

    duplicates
}
pub(super) fn collect_legacy_instagram_media_candidates(
    profile_root: &Path,
) -> Result<Vec<PathBuf>, String> {
    let mut collected = Vec::new();
    let mut pending = vec![profile_root.to_path_buf()];

    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|error| error.to_string())?;

            if file_type.is_dir() {
                if path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case("Settings"))
                {
                    continue;
                }
                pending.push(path);
                continue;
            }

            if !file_type.is_file() {
                continue;
            }

            if is_profile_picture_file(&path) {
                continue;
            }

            if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| {
                    value.eq_ignore_ascii_case("xml") || value.eq_ignore_ascii_case("txt")
                })
            {
                continue;
            }

            if infer_media_type(&path).is_none() {
                continue;
            }

            collected.push(path.clone());
        }
    }

    collected.sort();
    Ok(collected)
}
pub(super) fn load_external_imported_entity_keys(
    connection: &Connection,
    importer_id: &str,
) -> Result<HashSet<String>, String> {
    ensure_external_import_ledger_table(connection)?;
    let mut statement = connection
        .prepare(
            "SELECT entity_key
             FROM external_import_ledger
             WHERE importer_id = ?1",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![importer_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;

    let mut keys = HashSet::new();
    for row in rows {
        keys.insert(row.map_err(|error| error.to_string())?);
    }
    Ok(keys)
}
pub(super) fn normalize_import_entity_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}
pub(super) fn legacy_instagram_profile_handle(
    profile: &LegacyInstagramProfileXml,
    folder_name: &str,
) -> String {
    sanitize_source_handle(
        "instagram",
        profile
            .true_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or(profile.user_name.as_deref())
            .unwrap_or(folder_name),
    )
}
pub(super) fn legacy_instagram_profile_display_name(
    profile: &LegacyInstagramProfileXml,
    handle: &str,
) -> String {
    profile
        .friendly_name
        .as_deref()
        .or(profile.true_name.as_deref())
        .or(profile.user_site_name.as_deref())
        .unwrap_or(handle)
        .trim()
        .to_string()
}
/// Registro de um import externo (SCrawler, Tokkit, ...) para o ledger.
pub(super) struct ExternalImportLedgerRecord<'a> {
    pub(super) importer_id: &'a str,
    pub(super) profile_root: &'a Path,
    pub(super) provider: &'a str,
    pub(super) handle: &'a str,
    pub(super) source_id: &'a str,
    pub(super) account_id: &'a str,
    pub(super) timestamp: &'a str,
}
pub(super) fn record_external_import_ledger(
    connection: &Connection,
    record: ExternalImportLedgerRecord<'_>,
) -> Result<(), String> {
    let ExternalImportLedgerRecord {
        importer_id,
        profile_root,
        provider,
        handle,
        source_id,
        account_id,
        timestamp,
    } = record;
    ensure_external_import_ledger_table(connection)?;
    connection
        .execute(
            "INSERT INTO external_import_ledger (
                importer_id,
                entity_key,
                provider,
                handle,
                source_id,
                account_id,
                imported_at,
                updated_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT(importer_id, entity_key) DO UPDATE SET
               provider = excluded.provider,
               handle = excluded.handle,
               source_id = excluded.source_id,
               account_id = excluded.account_id,
               imported_at = excluded.imported_at,
               updated_at = excluded.updated_at",
            params![
                importer_id,
                normalize_import_entity_key(profile_root),
                provider,
                handle,
                source_id,
                account_id,
                timestamp,
            ],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "UPDATE source_profiles
             SET importer_id = ?1,
                 imported_at = ?2
             WHERE id = ?3",
            params![importer_id, timestamp, source_id],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}
pub(super) fn ensure_external_import_ledger_table(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS external_import_ledger (
                importer_id TEXT NOT NULL,
                entity_key TEXT NOT NULL,
                provider TEXT NOT NULL,
                handle TEXT NOT NULL,
                source_id TEXT,
                account_id TEXT,
                imported_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (importer_id, entity_key)
            );",
        )
        .map_err(|error| error.to_string())
}
#[derive(Default)]
pub(super) struct CompanionImportRecord {
    pub(super) provider_user_id: Option<String>,
    pub(super) provider_username: Option<String>,
    pub(super) backup_secret_ref: Option<String>,
    pub(super) backup_provider_user_id: Option<String>,
    pub(super) backup_provider_username: Option<String>,
    pub(super) backup_imported_at: Option<String>,
}
pub(super) fn normalize_companion_provider(provider: &str) -> Result<String, String> {
    let provider = provider.trim().to_ascii_lowercase();
    match provider.as_str() {
        "instagram" | "twitter" | "tiktok" => Ok(provider),
        _ => Err("This provider does not support Companion account import.".to_string()),
    }
}
pub(super) fn companion_username(value: &str) -> String {
    value.trim().trim_start_matches('@').to_ascii_lowercase()
}
pub(super) fn load_companion_import_record(
    connection: &Connection,
    account_id: &str,
) -> Result<Option<CompanionImportRecord>, String> {
    connection
        .query_row(
            "SELECT provider_user_id, provider_username, backup_secret_ref, backup_provider_user_id,
                    backup_provider_username, backup_imported_at
             FROM provider_account_import_state WHERE account_id = ?1",
            params![account_id],
            |row| Ok(CompanionImportRecord {
                provider_user_id: row.get(0)?,
                provider_username: row.get(1)?,
                backup_secret_ref: row.get(2)?,
                backup_provider_user_id: row.get(3)?,
                backup_provider_username: row.get(4)?,
                backup_imported_at: row.get(5)?,
            }),
        )
        .optional()
        .map_err(|error| error.to_string())
}
pub(super) fn validate_companion_capture(
    provider: &str,
    capture: &CompanionAccountCapture,
) -> Result<Vec<String>, String> {
    if capture.cookies.is_empty() || capture.cookies.len() > 300 {
        return Err("The captured cookie count is outside the supported range.".to_string());
    }
    validate_captured_cookies(&convert_provider_cookies_to_captured_cookies(
        capture.cookies.clone(),
    ))?;
    let allowed_domains: &[&str] = match provider {
        "instagram" => &["instagram.com"],
        "twitter" => &["x.com", "twitter.com"],
        "tiktok" => &["tiktok.com"],
        _ => &[],
    };
    let cookie_names = capture
        .cookies
        .iter()
        .filter(|cookie| !cookie.value.trim().is_empty())
        .map(|cookie| cookie.name.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    for cookie in &capture.cookies {
        if cookie.value.len() > 16 * 1024
            || !allowed_domains
                .iter()
                .any(|domain| domain_matches_allowed(&cookie.domain, domain))
        {
            return Err("The capture contains an invalid provider cookie.".to_string());
        }
    }
    let allowed_auth = [
        "csrfToken",
        "appId",
        "asbdId",
        "igWwwClaim",
        "userAgent",
        "secChUa",
        "secChUaFullVersionList",
        "secChUaPlatformVersion",
        "lsd",
        "dtsg",
    ];
    for (key, value) in &capture.authorization {
        if !allowed_auth.contains(&key.as_str()) || value.len() > 16 * 1024 {
            return Err(format!("Authorization field '{key}' is not supported."));
        }
    }
    let mut missing = Vec::new();
    if companion_username(&capture.identity.username).is_empty() {
        missing.push("identity.username".to_string());
    }
    let required: &[&[&str]] = match provider {
        "instagram" => &[&["sessionid"], &["csrftoken"]],
        "twitter" => &[&["auth_token"], &["ct0"]],
        "tiktok" => &[&["sessionid", "sessionid_ss"]],
        _ => &[],
    };
    for group in required {
        if !group.iter().any(|name| cookie_names.contains(*name)) {
            missing.push(format!("cookie:{}", group.join("|")));
        }
    }
    Ok(missing)
}
pub(super) fn companion_metadata(
    provider: &str,
    capture: &CompanionAccountCapture,
) -> CapturedBrowserMetadata {
    let value = |key: &str| {
        capture
            .authorization
            .get(key)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let cookie = |name: &str| {
        capture
            .cookies
            .iter()
            .find(|cookie| cookie.name.eq_ignore_ascii_case(name))
            .map(|cookie| cookie.value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    CapturedBrowserMetadata {
        csrf_token: (provider == "instagram")
            .then(|| value("csrfToken").or_else(|| cookie("csrftoken")))
            .flatten(),
        app_id: (provider == "instagram").then(|| value("appId")).flatten(),
        asbd_id: (provider == "instagram").then(|| value("asbdId")).flatten(),
        ig_www_claim: (provider == "instagram")
            .then(|| value("igWwwClaim"))
            .flatten(),
        user_agent: value("userAgent"),
        // Client-hints só alimentam o connector do Instagram; para os demais
        // providers eles nunca são aplicados no download, então não os
        // persistimos mesmo que um Companion antigo ainda os envie.
        sec_ch_ua: (provider == "instagram").then(|| value("secChUa")).flatten(),
        sec_ch_ua_full_version_list: (provider == "instagram")
            .then(|| value("secChUaFullVersionList"))
            .flatten(),
        sec_ch_ua_platform_version: (provider == "instagram")
            .then(|| value("secChUaPlatformVersion"))
            .flatten(),
        lsd: (provider == "instagram").then(|| value("lsd")).flatten(),
        dtsg: (provider == "instagram").then(|| value("dtsg")).flatten(),
    }
}
pub(super) fn preview_companion_account_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    capture: &CompanionAccountCapture,
) -> Result<CompanionAccountPreview, String> {
    let provider = normalize_companion_provider(&capture.provider)?;
    let missing_required_fields = validate_companion_capture(&provider, capture)?;
    let username = companion_username(&capture.identity.username);
    let captured_id = capture
        .identity
        .provider_user_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut candidates = Vec::new();
    for account in load_accounts(connection)?
        .into_iter()
        .filter(|account| account.provider.eq_ignore_ascii_case(&provider))
    {
        let state = load_companion_import_record(connection, &account.id)?;
        let match_kind = state.as_ref().and_then(|state| {
            if captured_id.is_some() && captured_id == state.provider_user_id.as_deref() {
                Some("provider_user_id".to_string())
            } else if !username.is_empty()
                && state
                    .provider_username
                    .as_deref()
                    .map(companion_username)
                    .as_deref()
                    == Some(username.as_str())
            {
                Some("username".to_string())
            } else {
                None
            }
        });
        let has_session = load_account_session_record(connection, &account.id)?
            .and_then(|session| session_secret_store::has_secret(layout, &session.secret_ref).ok())
            .unwrap_or(false);
        candidates.push(CompanionAccountCandidate {
            account_id: account.id,
            display_name: account.display_name,
            match_kind,
            has_session,
        });
    }
    candidates.sort_by(|left, right| {
        right
            .match_kind
            .is_some()
            .cmp(&left.match_kind.is_some())
            .then_with(|| left.display_name.cmp(&right.display_name))
    });
    let suggested_account_id = candidates
        .iter()
        .find(|candidate| candidate.match_kind.as_deref() == Some("provider_user_id"))
        .map(|candidate| candidate.account_id.clone());
    let metadata = companion_metadata(&provider, capture);
    let authorization_fields = [
        ("csrfToken", metadata.csrf_token.as_ref()),
        ("appId", metadata.app_id.as_ref()),
        ("asbdId", metadata.asbd_id.as_ref()),
        ("igWwwClaim", metadata.ig_www_claim.as_ref()),
        ("userAgent", metadata.user_agent.as_ref()),
        ("secChUa", metadata.sec_ch_ua.as_ref()),
        (
            "secChUaFullVersionList",
            metadata.sec_ch_ua_full_version_list.as_ref(),
        ),
        (
            "secChUaPlatformVersion",
            metadata.sec_ch_ua_platform_version.as_ref(),
        ),
        ("lsd", metadata.lsd.as_ref()),
        ("dtsg", metadata.dtsg.as_ref()),
    ]
    .into_iter()
    .filter_map(|(key, value)| value.map(|_| key.to_string()))
    .collect();
    Ok(CompanionAccountPreview {
        provider,
        username,
        cookie_count: capture.cookies.len(),
        authorization_fields,
        missing_required_fields,
        candidates,
        suggested_account_id,
    })
}
pub(super) fn clear_plaintext_companion_authorization(
    connection: &Connection,
    account_id: &str,
    provider: &str,
) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM provider_account_settings WHERE account_id = ?1 AND setting_key LIKE ?2",
            params![account_id, format!("{provider}.auth.%")],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}
pub(super) fn import_companion_account_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    input: CompanionAccountImportInput,
) -> Result<CompanionAccountImportResult, String> {
    let preview = preview_companion_account_with_connection(connection, layout, &input.capture)?;
    if !preview.missing_required_fields.is_empty() {
        return Err(format!(
            "The browser session is incomplete: {}.",
            preview.missing_required_fields.join(", ")
        ));
    }
    let provider = preview.provider;
    let username = preview.username;
    let (account_id, created) = match input
        .target_account_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(id) => {
            let account = load_provider_account_by_id(connection, id)?;
            if !account.provider.eq_ignore_ascii_case(&provider) {
                return Err("The selected account belongs to another provider.".to_string());
            }
            (id.to_string(), false)
        }
        None => (new_id(), true),
    };
    let old_session = load_account_session_record(connection, &account_id)?;
    let old_state = load_companion_import_record(connection, &account_id)?;
    let metadata = companion_metadata(&provider, &input.capture);
    let payload = serialize_session_payload_for_storage(
        &convert_provider_cookies_to_captured_cookies(input.capture.cookies.clone()),
        Some(input.capture.current_url.trim()),
        Some(&metadata),
    )?;
    let new_ref = format!("companion-{}-{}", account_id, Uuid::new_v4());
    session_secret_store::store_secret(layout, &new_ref, &payload)?;
    let imported_at = now_timestamp();
    connection
        .execute_batch("BEGIN IMMEDIATE TRANSACTION")
        .map_err(|error| error.to_string())?;
    let persisted = (|| {
        if created {
            let descriptor = providers::provider_runtime(&provider)
                .ok_or_else(|| "Provider runtime is unavailable.".to_string())?
                .descriptor();
            upsert_provider_account_with_connection(
                connection,
                layout,
                ProviderAccountUpsert {
                    id: Some(account_id.clone()),
                    provider: provider.clone(),
                    display_name: input
                        .create_display_name
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .unwrap_or(&username)
                        .to_string(),
                    auth_mode: "imported_session".to_string(),
                    auth_state: "ready".to_string(),
                    capabilities: descriptor.default_capabilities,
                    last_validated_at: None,
                },
            )?;
        }
        write_provider_account_session_record(
            connection,
            &account_id,
            &new_ref,
            &payload,
            &imported_at,
        )?;
        clear_plaintext_companion_authorization(connection, &account_id, &provider)?;
        connection.execute(
            "INSERT INTO provider_account_import_state (
                account_id, provider_user_id, provider_username, last_imported_at,
                backup_secret_ref, backup_provider_user_id, backup_provider_username, backup_imported_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(account_id) DO UPDATE SET provider_user_id=excluded.provider_user_id,
                provider_username=excluded.provider_username, last_imported_at=excluded.last_imported_at,
                backup_secret_ref=excluded.backup_secret_ref,
                backup_provider_user_id=excluded.backup_provider_user_id,
                backup_provider_username=excluded.backup_provider_username,
                backup_imported_at=excluded.backup_imported_at",
            params![
                account_id,
                input.capture.identity.provider_user_id.as_deref().map(str::trim).filter(|v| !v.is_empty()),
                username, imported_at,
                old_session.as_ref().map(|session| session.secret_ref.clone()),
                old_state.as_ref().and_then(|state| state.provider_user_id.clone()),
                old_state.as_ref().and_then(|state| state.provider_username.clone()),
                old_session.as_ref().map(|session| session.imported_at.clone()),
            ],
        ).map_err(|error| error.to_string())?;
        Ok::<(), String>(())
    })();
    if let Err(error) = persisted {
        let _ = connection.execute_batch("ROLLBACK");
        let _ = session_secret_store::delete_secret(layout, &new_ref);
        return Err(error);
    }
    if let Err(error) = connection.execute_batch("COMMIT") {
        let _ = session_secret_store::delete_secret(layout, &new_ref);
        return Err(error.to_string());
    }
    if let Some(previous_backup) = old_state.and_then(|state| state.backup_secret_ref) {
        if old_session
            .as_ref()
            .map(|session| session.secret_ref.as_str())
            != Some(previous_backup.as_str())
        {
            let _ = session_secret_store::delete_secret(layout, &previous_backup);
        }
    }
    let snapshot =
        validate_provider_account_with_connection(connection, layout, account_id.clone())?;
    let account = snapshot
        .accounts
        .iter()
        .find(|account| account.id == account_id)
        .ok_or_else(|| "Imported account disappeared after validation.".to_string())?;
    let validation_error = snapshot
        .account_sessions
        .iter()
        .find(|session| session.account_id == account_id)
        .and_then(|session| session.last_validation_error.clone());
    Ok(CompanionAccountImportResult {
        account_id,
        created,
        auth_state: account.auth_state.clone(),
        validation_error,
        can_revert: old_session.is_some(),
    })
}
pub(super) fn revert_provider_account_import_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    account_id: &str,
) -> Result<WorkspaceSnapshot, String> {
    let account = load_provider_account_by_id(connection, account_id)?;
    let current = load_account_session_record(connection, account_id)?
        .ok_or_else(|| "The account does not have a current session.".to_string())?;
    let state = load_companion_import_record(connection, account_id)?
        .ok_or_else(|| "The account does not have a Companion import backup.".to_string())?;
    let backup_ref = state
        .backup_secret_ref
        .clone()
        .ok_or_else(|| "The account does not have a previous import to restore.".to_string())?;
    let backup_payload = session_secret_store::load_secret(layout, &backup_ref)?;
    let restored_at = state
        .backup_imported_at
        .clone()
        .unwrap_or_else(now_timestamp);
    connection
        .execute_batch("BEGIN IMMEDIATE TRANSACTION")
        .map_err(|error| error.to_string())?;
    let reverted = (|| {
        write_provider_account_session_record(
            connection,
            account_id,
            &backup_ref,
            &backup_payload,
            &restored_at,
        )?;
        clear_plaintext_companion_authorization(connection, account_id, &account.provider)?;
        connection.execute(
            "UPDATE provider_account_import_state SET provider_user_id=?2, provider_username=?3,
                last_imported_at=?4, backup_secret_ref=?5, backup_provider_user_id=?6,
                backup_provider_username=?7, backup_imported_at=?8 WHERE account_id=?1",
            params![account_id, state.backup_provider_user_id, state.backup_provider_username,
                restored_at, current.secret_ref, state.provider_user_id,
                state.provider_username, current.imported_at],
        ).map_err(|error| error.to_string())?;
        Ok::<(), String>(())
    })();
    if let Err(error) = reverted {
        let _ = connection.execute_batch("ROLLBACK");
        return Err(error);
    }
    connection
        .execute_batch("COMMIT")
        .map_err(|error| error.to_string())?;
    validate_provider_account_with_connection(connection, layout, account_id.to_string())
}
pub(super) fn import_provider_account_cookies_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    input: ProviderAccountCookieImport,
) -> Result<WorkspaceSnapshot, String> {
    let account_id = input.account_id.trim().to_string();
    ensure_provider_account_exists(connection, &account_id)?;

    let import_format = input.import_format.trim().to_ascii_lowercase();
    if import_format.is_empty() {
        return Err("Cookie import format cannot be empty.".to_string());
    }

    if input.content.trim().is_empty() {
        return Err("Cookie import content cannot be empty.".to_string());
    }

    let parsed = parse_cookie_import_content(&import_format, &input.content)?;
    validate_captured_cookies(&parsed.cookies)?;
    let secret_payload = serialize_session_payload_for_storage(
        &parsed.cookies,
        parsed.current_url.as_deref(),
        Some(&parsed.metadata),
    )?;

    persist_provider_account_session_payload(connection, layout, &account_id, &secret_payload)?;
    apply_instagram_auth_settings_from_session_metadata(
        connection,
        layout,
        &account_id,
        &parsed.metadata,
        &parsed.cookies,
    )?;
    validate_provider_account_with_connection(connection, layout, account_id)
}
pub(super) fn load_account_import_backup_secret_ref(
    connection: &Connection,
    account_id: &str,
) -> Result<Option<String>, String> {
    connection
        .query_row(
            "SELECT backup_secret_ref FROM provider_account_import_state WHERE account_id = ?1",
            params![account_id],
            |row| row.get(0),
        )
        .optional()
        .map(|value| value.flatten())
        .map_err(|error| error.to_string())
}
/// Converts legacy reconciliation records into `DownloadedInstagramMedia`
/// (the shape the ledger upserts consume). Shared between the import
/// reconciliation and the recategorization backfill.
pub(super) fn legacy_reconciliation_records_to_downloaded_media(
    records: &[LegacyInstagramReconciliationRecord],
) -> Vec<instagram_connector::DownloadedInstagramMedia> {
    records
        .iter()
        .map(|record| instagram_connector::DownloadedInstagramMedia {
            file_path: record.file_path.clone(),
            media_type: record.media_type.clone(),
            media_section: record.media_section.clone(),
            provider_media_key: record.provider_media_key.clone(),
            provider_post_code: record.provider_post_code_cased.clone(),
            captured_at_timestamp: None,
            final_file_name: record
                .file_path
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or_default()
                .to_string(),
            legacy_raw_file_name: Some(record.legacy_file_name.clone()),
            extension: record
                .file_path
                .extension()
                .and_then(|value| value.to_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_ascii_lowercase())
                .unwrap_or_else(|| match record.media_type.as_str() {
                    "video" => "mp4".to_string(),
                    _ => "jpg".to_string(),
                }),
            pattern_mode: "legacy_backfill".to_string(),
            pattern_template: None,
        })
        .collect::<Vec<_>>()
}

/// Derives the observed posts (deduplicated by `provider_post_key`) from the
/// legacy reconciliation records.
pub(super) fn legacy_reconciliation_records_to_observed_posts(
    records: &[LegacyInstagramReconciliationRecord],
) -> Vec<instagram_connector::ObservedInstagramPost> {
    let mut observed_posts_by_key =
        HashMap::<String, instagram_connector::ObservedInstagramPost>::new();
    for record in records {
        if record.provider_post_key.trim().is_empty() {
            continue;
        }

        observed_posts_by_key
            .entry(record.provider_post_key.clone())
            .or_insert_with(|| instagram_connector::ObservedInstagramPost {
                provider_post_key: record.provider_post_key.clone(),
                provider_post_code: record.provider_post_code.clone(),
                media_section: record.media_section.clone(),
            });
    }
    observed_posts_by_key.into_values().collect::<Vec<_>>()
}

pub(super) fn reconcile_instagram_scrawler_profile_ledgers_with_connection(
    connection: &Connection,
    profile_root: &Path,
    source_id: &str,
    account_id: &str,
    source_handle: &str,
    timestamp: &str,
) -> Result<LegacyInstagramReconciliationStats, String> {
    let records = collect_legacy_instagram_reconciliation_records(profile_root)?;
    if records.is_empty() {
        return Ok(LegacyInstagramReconciliationStats::default());
    }

    let downloaded_media = legacy_reconciliation_records_to_downloaded_media(&records);
    let observed_posts = legacy_reconciliation_records_to_observed_posts(&records);

    upsert_instagram_media_ledger_entries(
        connection,
        source_id,
        account_id,
        source_handle,
        profile_root,
        &downloaded_media,
        timestamp,
    )?;
    upsert_instagram_media_alias_entries(
        connection,
        source_id,
        account_id,
        profile_root,
        &downloaded_media,
        timestamp,
    )?;
    upsert_instagram_media_fingerprint_entries(
        connection,
        source_id,
        account_id,
        profile_root,
        &downloaded_media,
        timestamp,
    )?;
    upsert_instagram_legacy_media_alias_entries(
        connection,
        source_id,
        account_id,
        profile_root,
        &records,
        timestamp,
    )?;
    upsert_instagram_legacy_media_fingerprint_entries(
        connection,
        source_id,
        account_id,
        profile_root,
        &records,
        timestamp,
    )?;
    upsert_instagram_post_ledger_entries(
        connection,
        source_id,
        account_id,
        source_handle,
        &observed_posts,
        timestamp,
    )?;

    Ok(LegacyInstagramReconciliationStats {
        seeded_media_entries: downloaded_media.len() as u32,
        seeded_post_entries: observed_posts.len() as u32,
    })
}
#[derive(Default)]
pub(super) struct ParsedCookieImportContent {
    pub(super) current_url: Option<String>,
    pub(super) metadata: CapturedBrowserMetadata,
    pub(super) cookies: Vec<CapturedBrowserCookie>,
}
pub(super) fn parse_cookie_import_content(
    import_format: &str,
    content: &str,
) -> Result<ParsedCookieImportContent, String> {
    match import_format {
        "json" => parse_session_payload(content).map(|payload| ParsedCookieImportContent {
            current_url: payload.current_url,
            metadata: payload.metadata,
            cookies: payload.cookies,
        }),
        "netscape" => {
            parse_netscape_cookie_text(content).map(|cookies| ParsedCookieImportContent {
                current_url: None,
                metadata: CapturedBrowserMetadata::default(),
                cookies,
            })
        }
        other => Err(format!(
            "Cookie import format '{}' is not supported.",
            other
        )),
    }
}

#[cfg(test)]
mod reels_section_inference_tests {
    use super::{infer_legacy_instagram_media_section, legacy_media_url_is_clip};
    use base64::Engine as _;

    /// Builds an Instagram video CDN URL with the given `xpv_encode_tag`
    /// embedded — as base64 — in the `efg` param, the way SCrawler stores it.
    /// `url_encode_padding` reproduces the `%3D` the XML usually carries.
    fn media_url_with_encode_tag(encode_tag: &str, url_encode_padding: bool) -> String {
        let payload = format!(
            "{{\"xpv_encode_tag\":\"{encode_tag}\",\"xpv_asset_id\":123,\"duration_s\":14}}"
        );
        let mut efg = base64::engine::general_purpose::STANDARD.encode(payload.as_bytes());
        if url_encode_padding {
            efg = efg.replace('=', "%3D");
        }
        format!("https://instagram.fxxx-1.fna.fbcdn.net/o1/v/t2/f2/m367/AQxxx.mp4?_nc_cat=101&efg={efg}&ccb=17-1&oh=00_deadbeef&oe=696E87EA")
    }

    #[test]
    fn detects_clip_from_encode_tag_in_efg() {
        let clip_url = media_url_with_encode_tag(
            "xpv_progressive.INSTAGRAM.CLIPS.C3.720.dash_baseline_1_v1",
            false,
        );
        assert!(legacy_media_url_is_clip(&clip_url));

        // Still detected when the padding comes url-encoded (`%3D`), as in the XML.
        let clip_url_encoded = media_url_with_encode_tag(
            "xpv_progressive.INSTAGRAM.CLIPS.C3.720.dash_baseline_1_v1",
            true,
        );
        assert!(legacy_media_url_is_clip(&clip_url_encoded));
    }

    #[test]
    fn feed_and_story_videos_are_not_clips() {
        let feed_url = media_url_with_encode_tag(
            "xpv_progressive.INSTAGRAM.FEED.C3.720.dash_baseline_1_v1",
            false,
        );
        assert!(!legacy_media_url_is_clip(&feed_url));

        let story_url = media_url_with_encode_tag("xpv_progressive.INSTAGRAM.STORY.C3.720", false);
        assert!(!legacy_media_url_is_clip(&story_url));

        // Images have no `efg`.
        assert!(!legacy_media_url_is_clip(
            "https://instagram.example/652760881_n.jpg?stp=dst-jpg&_nc_cat=1"
        ));
    }

    #[test]
    fn clip_url_reclassifies_p_permalink_as_reels() {
        // The core SCrawler case: reel stored with a `/p/<code>/` permalink
        // (not `/reel/`). Without the URL signal it would fall into `timeline`.
        let clip_url = media_url_with_encode_tag(
            "xpv_progressive.INSTAGRAM.CLIPS.C3.720.dash_baseline_1_v1",
            false,
        );
        assert_eq!(
            infer_legacy_instagram_media_section(
                None,
                "https://www.instagram.com/p/C_3zLt7PsrI/",
                Some(&clip_url),
            ),
            "reels"
        );
    }

    #[test]
    fn feed_video_with_p_permalink_stays_timeline() {
        let feed_url = media_url_with_encode_tag(
            "xpv_progressive.INSTAGRAM.FEED.C3.720.dash_baseline_1_v1",
            false,
        );
        assert_eq!(
            infer_legacy_instagram_media_section(
                None,
                "https://www.instagram.com/p/DLGaeghuNLN/",
                Some(&feed_url),
            ),
            "timeline"
        );
    }

    #[test]
    fn reel_permalink_still_wins_without_url() {
        assert_eq!(
            infer_legacy_instagram_media_section(
                None,
                "https://www.instagram.com/nynf4_/reel/DPkG4KtjqN6",
                None,
            ),
            "reels"
        );
    }

    #[test]
    fn highlight_folder_keeps_precedence_over_clip_url() {
        // A reel saved inside a highlight stays `stories` (highlight); it must
        // not be downgraded to `reels` by the URL signal.
        let clip_url = media_url_with_encode_tag(
            "xpv_progressive.INSTAGRAM.CLIPS.C3.720.dash_baseline_1_v1",
            false,
        );
        assert_eq!(
            infer_legacy_instagram_media_section(
                Some("Stories/Outfit yes"),
                "https://www.instagram.com/p/C_3zLt7PsrI/",
                Some(&clip_url),
            ),
            "stories"
        );
    }
}
