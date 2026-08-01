use super::tests::{create_test_layout, sample_account, sample_source};
use super::*;

fn seed_source(connection: &Connection, layout: &StorageLayout) -> Result<(), String> {
    upsert_provider_account_with_connection(
        connection,
        layout,
        sample_account("account-1", "twitter"),
    )?;
    upsert_source_profile_with_connection(
        connection,
        layout,
        sample_source("source-1", "twitter", Some("account-1")),
    )?;
    Ok(())
}

fn seed_post(connection: &Connection, post_key: &str, section: &str) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO provider_sync_post_ledger (
                provider, source_id, account_id, source_handle, provider_post_key,
                provider_post_code, media_section, first_seen_at, last_seen_at
             ) VALUES ('twitter', 'source-1', 'account-1', '@source-1', ?1, '', ?2,
                       '2026-03-01T00:00:00Z', '2026-03-01T00:00:00Z')",
            params![post_key, section],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn upstream_state(connection: &Connection, post_key: &str) -> (String, i64) {
    connection
        .query_row(
            "SELECT upstream_state, missing_confirmations FROM provider_sync_post_ledger
             WHERE provider = 'twitter' AND source_id = 'source-1' AND provider_post_key = ?1",
            params![post_key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("ledger row")
}

fn qualification<'a>(
    sections: &'a [String],
    observed: &'a HashSet<String>,
) -> UpstreamScanQualification<'a> {
    UpstreamScanQualification {
        provider: "twitter",
        source_id: "source-1",
        sections_scanned: sections,
        observed_post_keys: observed,
        enumerated_in_full: true,
        filtered: false,
        truncated: false,
    }
}

#[test]
fn a_post_is_flagged_only_after_repeated_qualifying_absences() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_source(connection, test_layout)?;
        seed_post(connection, "post-kept", "timeline")?;
        seed_post(connection, "post-gone", "timeline")?;

        let sections = vec!["timeline".to_string()];
        let observed = HashSet::from(["post-kept".to_string()]);

        let first = evaluate_upstream_presence(
            connection,
            &qualification(&sections, &observed),
            "2026-03-10T00:00:00Z",
        )?;
        assert_eq!(first.posts_seen, 2);
        assert_eq!(
            first.flagged, 0,
            "one short listing is not evidence of removal"
        );
        assert_eq!(upstream_state(connection, "post-gone"), ("present".to_string(), 1));

        let second = evaluate_upstream_presence(
            connection,
            &qualification(&sections, &observed),
            "2026-03-11T00:00:00Z",
        )?;
        assert_eq!(second.flagged, 1);
        assert_eq!(upstream_state(connection, "post-gone"), ("missing".to_string(), 2));
        assert_eq!(
            upstream_state(connection, "post-kept"),
            ("present".to_string(), 0)
        );
        Ok(())
    })
    .expect("evaluation should succeed");
}

#[test]
fn a_post_that_comes_back_clears_the_flag() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_source(connection, test_layout)?;
        seed_post(connection, "post-1", "timeline")?;

        let sections = vec!["timeline".to_string()];
        let empty = HashSet::new();
        for timestamp in ["2026-03-10T00:00:00Z", "2026-03-11T00:00:00Z"] {
            evaluate_upstream_presence(connection, &qualification(&sections, &empty), timestamp)?;
        }
        assert_eq!(upstream_state(connection, "post-1").0, "missing");

        let observed = HashSet::from(["post-1".to_string()]);
        let outcome = evaluate_upstream_presence(
            connection,
            &qualification(&sections, &observed),
            "2026-03-12T00:00:00Z",
        )?;
        assert_eq!(outcome.recovered, 1);
        assert_eq!(upstream_state(connection, "post-1"), ("present".to_string(), 0));
        Ok(())
    })
    .expect("recovery should succeed");
}

/// The whole reason this evaluation is gated: an incremental sync stops as soon
/// as it recognizes known posts, so nearly every post is legitimately absent
/// from the listing. Judging absence there would flag most of the library.
#[test]
fn a_scan_that_stopped_early_never_judges_absence() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_source(connection, test_layout)?;
        seed_post(connection, "post-1", "timeline")?;

        let sections = vec!["timeline".to_string()];
        let empty = HashSet::new();
        let mut incremental = qualification(&sections, &empty);
        incremental.enumerated_in_full = false;

        let outcome =
            evaluate_upstream_presence(connection, &incremental, "2026-03-10T00:00:00Z")?;
        assert_eq!(outcome, UpstreamPresenceOutcome::default());
        assert_eq!(upstream_state(connection, "post-1"), ("present".to_string(), 0));
        Ok(())
    })
    .expect("incremental scan should be ignored");
}

#[test]
fn filtered_and_truncated_scans_never_judge_absence() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_source(connection, test_layout)?;
        seed_post(connection, "post-1", "timeline")?;

        let sections = vec!["timeline".to_string()];
        let empty = HashSet::new();

        // A date window narrows the listing below the whole section.
        let mut filtered = qualification(&sections, &empty);
        filtered.filtered = true;
        evaluate_upstream_presence(connection, &filtered, "2026-03-10T00:00:00Z")?;

        // Rate limiting truncates it while the sync still reports success.
        let mut truncated = qualification(&sections, &empty);
        truncated.truncated = true;
        evaluate_upstream_presence(connection, &truncated, "2026-03-11T00:00:00Z")?;

        assert_eq!(upstream_state(connection, "post-1"), ("present".to_string(), 0));
        Ok(())
    })
    .expect("partial scans should be ignored");
}

/// Stories expire on their own and TikTok likes vanish when the owner un-likes
/// them. Neither says anything about the author removing a post.
#[test]
fn ephemeral_sections_are_out_of_scope() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_source(connection, test_layout)?;
        seed_post(connection, "story-1", "stories")?;
        seed_post(connection, "like-1", "likes")?;

        let sections = vec![
            "stories".to_string(),
            "likes".to_string(),
            "timeline".to_string(),
        ];
        let empty = HashSet::new();
        for timestamp in ["2026-03-10T00:00:00Z", "2026-03-11T00:00:00Z"] {
            evaluate_upstream_presence(connection, &qualification(&sections, &empty), timestamp)?;
        }

        assert_eq!(upstream_state(connection, "story-1"), ("present".to_string(), 0));
        assert_eq!(upstream_state(connection, "like-1"), ("present".to_string(), 0));
        Ok(())
    })
    .expect("ephemeral sections should be skipped");
}

/// Media the operator deleted on purpose is deliberately not re-downloaded, so
/// it is always absent from the listing. That is a local decision, not a
/// provider removal.
#[test]
fn locally_deleted_media_is_not_reported_as_removed_upstream() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_source(connection, test_layout)?;
        seed_post(connection, "post-deleted", "timeline")?;
        connection
            .execute(
                "INSERT INTO provider_deleted_media (
                    provider, source_id, relative_path, media_section,
                    provider_post_key, deleted_at
                 ) VALUES ('twitter', 'source-1', 'gone.mp4', 'timeline',
                           'post-deleted', '2026-03-05T00:00:00Z')",
                [],
            )
            .map_err(|error| error.to_string())?;

        let sections = vec!["timeline".to_string()];
        let empty = HashSet::new();
        for timestamp in ["2026-03-10T00:00:00Z", "2026-03-11T00:00:00Z"] {
            evaluate_upstream_presence(connection, &qualification(&sections, &empty), timestamp)?;
        }

        assert_eq!(
            upstream_state(connection, "post-deleted"),
            ("present".to_string(), 0)
        );
        Ok(())
    })
    .expect("locally deleted media should be skipped");
}

#[test]
fn the_verdict_is_projected_onto_the_media_index() {
    let (temp_dir, layout) = create_test_layout();
    let profile_root = temp_dir
        .path()
        .join("media")
        .join("twitter")
        .join("source-1");

    with_workspace_layout(layout, |connection, test_layout| {
        seed_source(connection, test_layout)?;
        seed_post(connection, "post-gone", "timeline")?;
        fs::create_dir_all(&profile_root).expect("profile folder");
        let file_path = profile_root.join("video.mp4");
        fs::write(&file_path, "bytes").expect("media file");
        upsert_media_index_entry(
            connection,
            "twitter",
            "source-1",
            &MediaIndexEntry {
                relative_path: "video.mp4",
                absolute_path: &file_path,
                media_type: "video",
                media_section: "timeline",
                provider_media_key: Some("media-1"),
                provider_post_key: Some("post-gone"),
                captured_at: Some(1_760_000_000),
            },
            "2026-03-01T00:00:00Z",
        )?;

        let sections = vec!["timeline".to_string()];
        let empty = HashSet::new();
        for timestamp in ["2026-03-10T00:00:00Z", "2026-03-11T00:00:00Z"] {
            evaluate_upstream_presence(connection, &qualification(&sections, &empty), timestamp)?;
        }

        let state: String = connection
            .query_row(
                "SELECT upstream_state FROM media_index WHERE source_id = 'source-1'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        assert_eq!(state, "missing");
        assert_eq!(media_index_counts(connection)?.upstream_missing, 1);

        let flagged =
            load_upstream_missing_post_keys(connection, "twitter", "source-1")?;
        assert!(flagged.contains("post-gone"));
        Ok(())
    })
    .expect("projection should succeed");
}

#[test]
fn qualifying_scans_are_recorded_for_audit() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_source(connection, test_layout)?;
        seed_post(connection, "post-1", "timeline")?;

        let sections = vec!["timeline".to_string(), "stories".to_string()];
        let observed = HashSet::from(["post-1".to_string()]);
        evaluate_upstream_presence(
            connection,
            &qualification(&sections, &observed),
            "2026-03-10T00:00:00Z",
        )?;

        let (recorded_sections, posts_seen) = connection
            .query_row(
                "SELECT sections, posts_seen FROM source_full_scan_runs WHERE source_id = 'source-1'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .map_err(|error| error.to_string())?;
        assert_eq!(
            recorded_sections, "timeline",
            "only sections that can report removal are recorded"
        );
        assert_eq!(posts_seen, 1);
        Ok(())
    })
    .expect("audit trail should be written");
}
