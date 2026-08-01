use super::tests::{create_test_layout, sample_account, sample_source};
use super::*;

fn seed(connection: &Connection, layout: &StorageLayout, ids: &[&str]) -> Result<(), String> {
    upsert_provider_account_with_connection(
        connection,
        layout,
        sample_account("account-1", "twitter"),
    )?;
    for id in ids {
        upsert_source_profile_with_connection(
            connection,
            layout,
            sample_source(id, "twitter", Some("account-1")),
        )?;
    }
    Ok(())
}

fn index(
    connection: &Connection,
    layout: &StorageLayout,
    source_id: &str,
    relative_path: &str,
    captured_at: i64,
    size_bytes: i64,
) -> Result<String, String> {
    let source = load_sources(connection)?
        .into_iter()
        .find(|candidate| candidate.id == source_id)
        .expect("seeded source");
    let root = resolved_source_media_output_root_with_connection(connection, layout, &source)?;
    let path = root.join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("media folder");
    }
    fs::write(&path, "bytes").expect("media file");
    upsert_media_index_entry(
        connection,
        "twitter",
        source_id,
        &MediaIndexEntry {
            relative_path,
            absolute_path: &path,
            media_type: "image",
            media_section: "timeline",
            provider_media_key: Some(relative_path),
            provider_post_key: Some(relative_path),
            captured_at: Some(captured_at),
        },
        "2026-03-10T00:00:00Z",
    )?;
    let id: String = connection
        .query_row(
            "SELECT id FROM media_index WHERE source_id = ?1 AND relative_path = ?2",
            params![source_id, relative_path],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "UPDATE media_index SET size_bytes = ?2 WHERE id = ?1",
            params![id, size_bytes],
        )
        .map_err(|error| error.to_string())?;
    Ok(id)
}

#[test]
fn the_dashboard_totals_the_library_and_ranks_profiles_by_disk_use() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed(connection, test_layout, &["source-1", "source-2"])?;
        index(connection, test_layout, "source-1", "a.jpg", 1_760_000_000, 5_000)?;
        index(connection, test_layout, "source-1", "b.jpg", 1_760_100_000, 7_000)?;
        index(connection, test_layout, "source-2", "c.jpg", 1_760_200_000, 1_000)?;

        let dashboard = load_library_dashboard_with_connection(connection)?;
        assert_eq!(dashboard.total_files, 3);
        assert_eq!(dashboard.total_bytes, 13_000);
        assert_eq!(dashboard.total_sources, 2);
        assert_eq!(dashboard.providers.len(), 1);
        assert_eq!(dashboard.providers[0].provider, "twitter");
        assert_eq!(dashboard.top_profiles[0].source_id, "source-1");
        assert_eq!(dashboard.top_profiles[0].bytes, 12_000);
        Ok(())
    })
    .expect("dashboard should load");
}

/// The distinction the existing health view does not make: a profile whose sync
/// is broken needs attention, one that simply stopped posting does not.
#[test]
fn stalled_profiles_separate_a_broken_sync_from_a_profile_that_stopped_posting() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed(connection, test_layout, &["failing", "quiet", "healthy"])?;
        let now = Utc::now().timestamp();
        index(connection, test_layout, "failing", "a.jpg", now - 86_400, 1_000)?;
        // No media for well over the stall threshold.
        index(connection, test_layout, "quiet", "b.jpg", now - 200 * 86_400, 1_000)?;
        index(connection, test_layout, "healthy", "c.jpg", now - 86_400, 1_000)?;
        connection
            .execute(
                "UPDATE source_profiles
                 SET sync_problem_code = 'auth_required', sync_problem_message = 'Session expired'
                 WHERE id = 'failing'",
                [],
            )
            .map_err(|error| error.to_string())?;

        let dashboard = load_library_dashboard_with_connection(connection)?;
        let by_id = |id: &str| {
            dashboard
                .stalled_profiles
                .iter()
                .find(|profile| profile.source_id == id)
                .cloned()
        };

        let failing = by_id("failing").expect("failing profile is listed");
        assert_eq!(failing.reason, "sync_failing");
        assert_eq!(failing.sync_problem_code.as_deref(), Some("auth_required"));

        let quiet = by_id("quiet").expect("quiet profile is listed");
        assert_eq!(quiet.reason, "not_posting");
        assert!(quiet.days_since_last_media.is_some_and(|days| days > 60));

        assert!(
            by_id("healthy").is_none(),
            "a profile posting normally is not stalled"
        );
        Ok(())
    })
    .expect("stall detection should work");
}

#[test]
fn the_dashboard_reports_what_variant_grouping_could_reclaim() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed(connection, test_layout, &["source-1"])?;
        let canonical = index(connection, test_layout, "source-1", "feed.jpg", 1_000, 9_000)?;
        let variant = index(connection, test_layout, "source-1", "story.jpg", 1_000, 4_000)?;
        connection
            .execute(
                "INSERT INTO media_variant_groups (
                    id, scope, canonical_media_id, match_kind, confidence, policy_applied,
                    reviewed, created_at, updated_at
                 ) VALUES ('group-1', 'intra_source', ?1, 'perceptual_image', 0.9,
                           'link_only', 0, '2026-03-10T00:00:00Z', '2026-03-10T00:00:00Z')",
                params![canonical],
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE media_index SET variant_group_id = 'group-1', is_canonical = 0
                 WHERE id = ?1",
                params![variant],
            )
            .map_err(|error| error.to_string())?;

        let dashboard = load_library_dashboard_with_connection(connection)?;
        assert_eq!(dashboard.variant_groups, 1);
        assert_eq!(
            dashboard.variant_reclaimable_bytes, 4_000,
            "only the non-canonical copies count as reclaimable"
        );
        Ok(())
    })
    .expect("variant reporting should work");
}

#[test]
fn growth_is_reported_per_month_oldest_first() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed(connection, test_layout, &["source-1"])?;
        // 2026-01 and 2026-03.
        index(connection, test_layout, "source-1", "jan.jpg", 1_767_225_600, 1_000)?;
        index(connection, test_layout, "source-1", "mar.jpg", 1_772_496_000, 2_000)?;

        let dashboard = load_library_dashboard_with_connection(connection)?;
        assert_eq!(dashboard.growth.len(), 2);
        assert!(
            dashboard.growth[0].month < dashboard.growth[1].month,
            "the chart reads left to right"
        );
        Ok(())
    })
    .expect("growth should be reported");
}
