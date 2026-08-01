use super::tests::{create_test_layout, sample_account, sample_source};
use super::*;

fn seed(connection: &Connection, layout: &StorageLayout, id: &str) -> Result<(), String> {
    upsert_provider_account_with_connection(
        connection,
        layout,
        sample_account("account-1", "twitter"),
    )?;
    upsert_source_profile_with_connection(
        connection,
        layout,
        sample_source(id, "twitter", Some("account-1")),
    )?;
    Ok(())
}

fn stored_user_id(connection: &Connection, source_id: &str) -> Option<String> {
    connection
        .query_row(
            "SELECT provider_user_id FROM source_profiles WHERE id = ?1",
            params![source_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .expect("profile row")
}

#[test]
fn the_first_resolved_user_id_is_adopted_and_the_handle_recorded() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed(connection, test_layout, "source-1")?;

        let outcome = record_source_identity(
            connection,
            "source-1",
            "17841400000000001",
            "@source-1",
            "2026-03-10T00:00:00Z",
        )?;
        assert_eq!(outcome, SourceIdentityOutcome::Adopted);
        assert_eq!(
            stored_user_id(connection, "source-1").as_deref(),
            Some("17841400000000001")
        );

        let history = load_source_handle_history(connection, "source-1")?;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].handle, "source-1");
        Ok(())
    })
    .expect("adoption should succeed");
}

#[test]
fn a_renamed_profile_keeps_its_id_and_gains_a_handle_history_entry() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed(connection, test_layout, "source-1")?;
        record_source_identity(
            connection,
            "source-1",
            "user-1",
            "@source-1",
            "2026-03-10T00:00:00Z",
        )?;

        let outcome = record_source_identity(
            connection,
            "source-1",
            "user-1",
            "@brand-new-handle",
            "2026-03-11T00:00:00Z",
        )?;
        assert_eq!(
            outcome,
            SourceIdentityOutcome::Renamed {
                previous_handle: "source-1".to_string()
            }
        );

        let history = load_source_handle_history(connection, "source-1")?;
        let handles: Vec<&str> = history.iter().map(|entry| entry.handle.as_str()).collect();
        assert!(handles.contains(&"source-1"));
        assert!(handles.contains(&"brand-new-handle"));
        Ok(())
    })
    .expect("rename should be classified");
}

/// The case that silently corrupts an archive: the tracked person abandoned the
/// handle, somebody else claimed it, and the sync would download a stranger's
/// media into the original person's folder.
#[test]
fn a_recycled_handle_is_flagged_and_never_overwrites_the_stored_identity() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed(connection, test_layout, "source-1")?;
        record_source_identity(
            connection,
            "source-1",
            "original-owner",
            "@source-1",
            "2026-03-10T00:00:00Z",
        )?;

        let outcome = record_source_identity(
            connection,
            "source-1",
            "someone-else",
            "@source-1",
            "2026-03-11T00:00:00Z",
        )?;
        assert_eq!(
            outcome,
            SourceIdentityOutcome::HandleRecycled {
                previous_user_id: "original-owner".to_string(),
                current_user_id: "someone-else".to_string(),
            }
        );
        assert_eq!(
            stored_user_id(connection, "source-1").as_deref(),
            Some("original-owner"),
            "the profile still refers to the person whose archive is on disk"
        );
        Ok(())
    })
    .expect("recycled handle should be classified");
}

#[test]
fn an_unchanged_profile_reports_nothing_but_refreshes_the_history() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed(connection, test_layout, "source-1")?;
        record_source_identity(
            connection,
            "source-1",
            "user-1",
            "@source-1",
            "2026-03-10T00:00:00Z",
        )?;

        let outcome = record_source_identity(
            connection,
            "source-1",
            "user-1",
            "@source-1",
            "2026-03-12T00:00:00Z",
        )?;
        assert_eq!(outcome, SourceIdentityOutcome::Unchanged);

        let history = load_source_handle_history(connection, "source-1")?;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].last_seen_at, "2026-03-12T00:00:00Z");
        Ok(())
    })
    .expect("unchanged identity should refresh the history");
}

#[test]
fn lookup_by_user_id_uses_the_column_and_still_finds_legacy_json_hints() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed(connection, test_layout, "source-1")?;
        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-2", "twitter", Some("account-1")),
        )?;

        record_source_identity(
            connection,
            "source-1",
            "user-1",
            "@source-1",
            "2026-03-10T00:00:00Z",
        )?;
        let found = find_source_by_provider_user_id(connection, "twitter", "user-1", "source-2")?;
        assert_eq!(found.map(|(id, _)| id), Some("source-1".to_string()));

        // A profile that has not synced since the column was introduced only
        // carries the id inside its sync options JSON.
        let mut options = default_source_sync_options("twitter");
        if let Some(twitter) = options.twitter.as_mut() {
            twitter.user_id_hint = Some("legacy-user".to_string());
        }
        connection
            .execute(
                "UPDATE source_profiles SET sync_options_json = ?2, provider_user_id = NULL
                 WHERE id = ?1",
                params![
                    "source-2",
                    serde_json::to_string(&options).expect("options json")
                ],
            )
            .map_err(|error| error.to_string())?;

        let legacy =
            find_source_by_provider_user_id(connection, "twitter", "legacy-user", "source-1")?;
        assert_eq!(
            legacy.map(|(id, _)| id),
            Some("source-2".to_string()),
            "profiles synced before the column must stay discoverable"
        );
        Ok(())
    })
    .expect("lookup should succeed");
}
