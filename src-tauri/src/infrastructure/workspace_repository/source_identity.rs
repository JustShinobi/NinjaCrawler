use super::*;

/// What the sync learned about who a profile actually is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceIdentityOutcome {
    /// Same person, same handle — nothing to report.
    Unchanged,
    /// First time this profile resolved to a provider user id.
    Adopted,
    /// Same user id, new handle: the person renamed their profile.
    Renamed { previous_handle: String },
    /// Same handle, different user id: somebody else now owns the handle.
    ///
    /// This is the dangerous one. Without detection the sync would keep
    /// downloading a stranger's media into the folder of the profile that was
    /// archived, silently mixing two people in one archive.
    HandleRecycled {
        previous_user_id: String,
        current_user_id: String,
    },
}

fn stored_identity(
    connection: &Connection,
    source_id: &str,
) -> Result<(Option<String>, String), String> {
    connection
        .query_row(
            "SELECT provider_user_id, handle FROM source_profiles WHERE id = ?1",
            params![source_id],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|error| error.to_string())
}

fn record_handle(
    connection: &Connection,
    source_id: &str,
    handle: &str,
    user_id: &str,
    timestamp: &str,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO source_handle_history (
                source_id, handle, provider_user_id, first_seen_at, last_seen_at
             ) VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(source_id, handle) DO UPDATE SET
                provider_user_id = COALESCE(excluded.provider_user_id, provider_user_id),
                last_seen_at = excluded.last_seen_at",
            params![source_id, handle.trim(), user_id, timestamp],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Records the provider user id a sync resolved, keeps the handle history, and
/// classifies what changed.
///
/// The caller decides what to do with the verdict: a rename is applied, a
/// recycled handle aborts the run. This function only writes what is safe to
/// write in both cases — it never rewrites the handle itself.
pub(crate) fn record_source_identity(
    connection: &Connection,
    source_id: &str,
    user_id: &str,
    observed_handle: &str,
    timestamp: &str,
) -> Result<SourceIdentityOutcome, String> {
    let user_id = user_id.trim();
    if user_id.is_empty() {
        return Ok(SourceIdentityOutcome::Unchanged);
    }
    let (stored_user_id, stored_handle) = stored_identity(connection, source_id)?;
    let observed_handle = observed_handle.trim().trim_start_matches('@');
    let stored_handle_bare = stored_handle.trim().trim_start_matches('@').to_string();

    let outcome = match stored_user_id.as_deref().map(str::trim) {
        None | Some("") => SourceIdentityOutcome::Adopted,
        Some(previous) if previous == user_id => {
            if observed_handle.is_empty()
                || observed_handle.eq_ignore_ascii_case(&stored_handle_bare)
            {
                SourceIdentityOutcome::Unchanged
            } else {
                SourceIdentityOutcome::Renamed {
                    previous_handle: stored_handle_bare.clone(),
                }
            }
        }
        Some(previous) => SourceIdentityOutcome::HandleRecycled {
            previous_user_id: previous.to_string(),
            current_user_id: user_id.to_string(),
        },
    };

    // A recycled handle must not overwrite the stored id: the profile in the
    // workspace still refers to the original person, whose archive is on disk.
    if !matches!(outcome, SourceIdentityOutcome::HandleRecycled { .. }) {
        connection
            .execute(
                "UPDATE source_profiles SET provider_user_id = ?2 WHERE id = ?1",
                params![source_id, user_id],
            )
            .map_err(|error| error.to_string())?;
        if !stored_handle_bare.is_empty() {
            record_handle(
                connection,
                source_id,
                &stored_handle_bare,
                user_id,
                timestamp,
            )?;
        }
        if !observed_handle.is_empty() {
            record_handle(connection, source_id, observed_handle, user_id, timestamp)?;
        }
    }

    Ok(outcome)
}

pub(crate) const HANDLE_RECYCLED_PROBLEM_CODE: &str = "handle_recycled";

/// Records what a sync learned about a profile's identity and acts on it.
///
/// A recycled handle stops the profile: `ready_for_download` is cleared so the
/// next scheduled run cannot pour a stranger's media into an archive that
/// belongs to someone else. Recovering is a deliberate operator action, which is
/// the point — this is unrecoverable corruption if it happens silently.
///
/// Best-effort: identity bookkeeping must not fail a sync that already worked.
pub(super) fn apply_source_identity_verdict(
    connection: &Connection,
    layout: &StorageLayout,
    context: &SourceSyncContext,
    user_id: &str,
    observed_handle: &str,
    timestamp: &str,
) {
    let outcome = match record_source_identity(
        connection,
        &context.source.id,
        user_id,
        observed_handle,
        timestamp,
    ) {
        Ok(outcome) => outcome,
        Err(error) => {
            let _ = runtime_log::append_workspace(
                "source_identity",
                "warn",
                RuntimeLogAnchor {
                    account_id: Some(&context.account.id),
                    provider: Some(&context.source.provider),
                    source_id: Some(&context.source.id),
                    source_handle: Some(&context.source.handle),
                },
                "Failed to record the profile identity resolved by this sync.",
                Some(error),
            );
            return;
        }
    };

    match outcome {
        SourceIdentityOutcome::Unchanged | SourceIdentityOutcome::Adopted => {}
        SourceIdentityOutcome::Renamed { previous_handle } => {
            log_runtime_event(
                layout,
                "sync.profile",
                "info",
                RuntimeLogAnchor {
                    account_id: Some(&context.account.id),
                    provider: Some(&context.source.provider),
                    source_id: Some(&context.source.id),
                    source_handle: Some(&context.source.handle),
                },
                format!(
                    "'{}' was previously known as '{}' (same provider user id).",
                    context.source.handle, previous_handle
                ),
                None,
            );
        }
        SourceIdentityOutcome::HandleRecycled {
            previous_user_id,
            current_user_id,
        } => {
            let message = format!(
                "The handle '{}' now resolves to a different provider account (was user id {}, now {}). Downloads were paused so media from the new owner is not mixed into this archive. Review the profile and either point it at the new account or keep it paused.",
                context.source.handle, previous_user_id, current_user_id
            );
            let _ = set_source_sync_problem(
                connection,
                &context.source.id,
                HANDLE_RECYCLED_PROBLEM_CODE,
                &message,
                timestamp,
                true,
            );
            log_runtime_event(
                layout,
                "sync.profile",
                "warning",
                RuntimeLogAnchor {
                    account_id: Some(&context.account.id),
                    provider: Some(&context.source.provider),
                    source_id: Some(&context.source.id),
                    source_handle: Some(&context.source.handle),
                },
                message,
                None,
            );
        }
    }
}

/// Looks a profile up by provider user id, preferring the normalized column and
/// falling back to the legacy hint inside `sync_options_json` for profiles that
/// have not synced since the column was introduced.
pub(crate) fn find_source_by_provider_user_id(
    connection: &Connection,
    provider: &str,
    user_id: &str,
    self_id: &str,
) -> Result<Option<(String, String)>, String> {
    let user_id = user_id.trim();
    if user_id.is_empty() {
        return Ok(None);
    }
    let indexed = connection
        .query_row(
            "SELECT id, handle FROM source_profiles
             WHERE provider = ?1 AND provider_user_id = ?2
               AND deleted_at IS NULL AND id != ?3
             LIMIT 1",
            params![provider, user_id, self_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if indexed.is_some() {
        return Ok(indexed);
    }

    let mut statement = connection
        .prepare(
            "SELECT id, handle, sync_options_json FROM source_profiles
             WHERE provider = ?1 AND deleted_at IS NULL AND id != ?2
               AND (provider_user_id IS NULL OR provider_user_id = '')",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![provider, self_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (id, handle, json) = row.map_err(|error| error.to_string())?;
        if source_user_id_hint_from_json(provider, &json).as_deref() == Some(user_id) {
            return Ok(Some((id, handle)));
        }
    }
    Ok(None)
}

pub(crate) fn list_identities() -> Result<Vec<Identity>, String> {
    with_workspace(|connection, _| {
        let mut statement = connection
            .prepare(
                "SELECT id, display_name, notes, avatar_source_id, created_at, updated_at
                 FROM identities ORDER BY display_name COLLATE NOCASE",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(Identity {
                    id: row.get(0)?,
                    display_name: row.get(1)?,
                    notes: row.get(2)?,
                    avatar_source_id: row.get(3)?,
                    source_ids: Vec::new(),
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })
            .map_err(|error| error.to_string())?;
        let mut identities = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;

        let mut members = connection
            .prepare(
                "SELECT identity_id, id FROM source_profiles
                 WHERE identity_id IS NOT NULL AND deleted_at IS NULL",
            )
            .map_err(|error| error.to_string())?;
        let mut by_identity: HashMap<String, Vec<String>> = HashMap::new();
        let member_rows = members
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        for row in member_rows {
            let (identity_id, source_id) = row.map_err(|error| error.to_string())?;
            by_identity.entry(identity_id).or_default().push(source_id);
        }
        for identity in &mut identities {
            identity.source_ids = by_identity.remove(&identity.id).unwrap_or_default();
        }
        Ok(identities)
    })
}

pub(crate) fn create_identity(display_name: String, notes: Option<String>) -> Result<Identity, String> {
    let display_name = display_name.trim().to_string();
    if display_name.is_empty() {
        return Err("An identity needs a name.".to_string());
    }
    let now = Utc::now().to_rfc3339();
    let identity = Identity {
        id: Uuid::new_v4().to_string(),
        display_name,
        notes: notes.map(|value| value.trim().to_string()).filter(|value| !value.is_empty()),
        avatar_source_id: None,
        source_ids: Vec::new(),
        created_at: now.clone(),
        updated_at: now,
    };
    with_workspace(|connection, _| {
        connection
            .execute(
                "INSERT INTO identities (id, display_name, notes, avatar_source_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, NULL, ?4, ?4)",
                params![
                    identity.id,
                    identity.display_name,
                    identity.notes,
                    identity.created_at
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })?;
    Ok(identity)
}

pub(crate) fn delete_identity(identity_id: String) -> Result<(), String> {
    with_workspace(|connection, _| {
        // The FK clears `identity_id` on the members; the profiles themselves
        // and their media are untouched.
        connection
            .execute("DELETE FROM identities WHERE id = ?1", params![identity_id])
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

pub(crate) fn link_source_to_identity(
    source_id: String,
    identity_id: Option<String>,
) -> Result<(), String> {
    with_workspace(|connection, _| {
        connection
            .execute(
                "UPDATE source_profiles SET identity_id = ?2, updated_at = ?3
                 WHERE id = ?1 AND deleted_at IS NULL",
                params![source_id, identity_id, Utc::now().to_rfc3339()],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

/// Profiles on different providers sharing the exact same handle. A weak signal
/// on its own, which is why it is only ever a suggestion for the operator to
/// confirm.
pub(crate) fn suggest_identity_links() -> Result<Vec<IdentityLinkSuggestion>, String> {
    with_workspace(|connection, _| {
        let sources = load_sources(connection)?;
        let mut suggestions = Vec::new();
        for candidate in &sources {
            if candidate.identity_id.is_some() {
                continue;
            }
            let handle = candidate.handle.trim().trim_start_matches('@').to_ascii_lowercase();
            if handle.is_empty() {
                continue;
            }
            let matched = sources.iter().find(|other| {
                other.id != candidate.id
                    && !other.provider.eq_ignore_ascii_case(&candidate.provider)
                    && other
                        .handle
                        .trim()
                        .trim_start_matches('@')
                        .eq_ignore_ascii_case(&handle)
            });
            if let Some(matched) = matched {
                suggestions.push(IdentityLinkSuggestion {
                    source_id: candidate.id.clone(),
                    provider: candidate.provider.clone(),
                    handle: candidate.handle.clone(),
                    reason: "same_handle".to_string(),
                    matched_source_id: matched.id.clone(),
                    matched_provider: matched.provider.clone(),
                });
            }
        }
        Ok(suggestions)
    })
}

pub(crate) fn load_source_handle_history_for(
    source_id: String,
) -> Result<Vec<SourceHandleHistoryEntry>, String> {
    with_workspace(|connection, _| load_source_handle_history(connection, &source_id))
}

/// Every handle a profile has answered to, most recent first.
pub(crate) fn load_source_handle_history(
    connection: &Connection,
    source_id: &str,
) -> Result<Vec<SourceHandleHistoryEntry>, String> {
    let mut statement = connection
        .prepare(
            "SELECT handle, provider_user_id, first_seen_at, last_seen_at
             FROM source_handle_history
             WHERE source_id = ?1
             ORDER BY last_seen_at DESC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![source_id], |row| {
            Ok(SourceHandleHistoryEntry {
                handle: row.get(0)?,
                provider_user_id: row.get(1)?,
                first_seen_at: row.get(2)?,
                last_seen_at: row.get(3)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}
