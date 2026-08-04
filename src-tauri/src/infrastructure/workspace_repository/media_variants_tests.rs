use super::tests::{create_test_layout, sample_account, sample_source};
use super::*;

fn seed_profiles(
    connection: &Connection,
    layout: &StorageLayout,
    profiles: &[(&str, &str)],
) -> Result<(), String> {
    for (id, provider) in profiles {
        upsert_provider_account_with_connection(
            connection,
            layout,
            sample_account(&format!("account-{id}"), provider),
        )?;
        upsert_source_profile_with_connection(
            connection,
            layout,
            sample_source(id, provider, Some(&format!("account-{id}"))),
        )?;
    }
    Ok(())
}

/// Indexes a file and writes the fingerprint directly, standing in for what the
/// hashing backlog produces.
#[allow(clippy::too_many_arguments)]
fn index_with_fingerprint(
    connection: &Connection,
    layout: &StorageLayout,
    source_id: &str,
    provider: &str,
    relative_path: &str,
    section: &str,
    media_type: &str,
    dhash: Option<&str>,
    video_signature: Option<&str>,
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
        provider,
        source_id,
        &MediaIndexEntry {
            relative_path,
            absolute_path: &path,
            media_type,
            media_section: section,
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
            "UPDATE media_index
             SET fingerprint_status = 'complete', dhash64 = ?2, ahash64 = ?2,
                 video_signature = ?3, width = 1080, height = 1920, duration_ms = 15000,
                 size_bytes = ?4
             WHERE id = ?1",
            params![id, dhash, video_signature, size_bytes],
        )
        .map_err(|error| error.to_string())?;
    Ok(id)
}

fn signature(frames: &[&str]) -> String {
    serde_json::to_string(frames).expect("signature json")
}

/// The reported case: a profile posts a video to its story and then to the feed.
/// Different provider keys, different encodes — only the visual content pairs them.
#[test]
fn a_story_reposted_to_the_feed_is_grouped_within_the_same_profile() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_profiles(connection, test_layout, &[("source-1", "instagram")])?;
        let frames = signature(&["aaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbb", "cccccccccccccccc"]);
        index_with_fingerprint(
            connection, test_layout, "source-1", "instagram", "story.mp4", "stories",
            "video", None, Some(&frames), 1_000, 5_000,
        )?;
        index_with_fingerprint(
            connection, test_layout, "source-1", "instagram", "feed.mp4", "timeline",
            "video", None, Some(&frames), 2_000, 9_000,
        )?;

        let outcome = detect_variants_with_connection(
            connection,
            &["source-1".to_string()],
            None,
            "2026-03-12T00:00:00Z",
        )?;
        assert_eq!(outcome.groups_created, 1);
        assert_eq!(outcome.media_grouped, 2);

        let groups = load_variant_groups_with_connection(connection, test_layout, 10)?;
        assert_eq!(groups[0].scope, "intra_source");
        assert_eq!(groups[0].match_kind, "perceptual_video");
        assert_eq!(groups[0].policy_applied, "link_only", "nothing is deleted by default");

        // The bigger copy (the clean feed upload) becomes the canonical one.
        let canonical = groups[0]
            .members
            .iter()
            .find(|member| member.role == "canonical")
            .expect("canonical member");
        assert_eq!(canonical.relative_path, "feed.mp4");
        Ok(())
    })
    .expect("story/feed grouping should work");
}

/// Two posts of the same profile in the same section are just two posts, even
/// if they look alike — grouping them would hide real content.
#[test]
fn two_posts_in_the_same_section_are_never_grouped() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_profiles(connection, test_layout, &[("source-1", "instagram")])?;
        let frames = signature(&["aaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbb"]);
        index_with_fingerprint(
            connection, test_layout, "source-1", "instagram", "one.mp4", "timeline",
            "video", None, Some(&frames), 1_000, 5_000,
        )?;
        index_with_fingerprint(
            connection, test_layout, "source-1", "instagram", "two.mp4", "timeline",
            "video", None, Some(&frames), 2_000, 5_000,
        )?;

        let outcome = detect_variants_with_connection(
            connection,
            &["source-1".to_string()],
            None,
            "2026-03-12T00:00:00Z",
        )?;
        assert_eq!(outcome.groups_created, 0);
        Ok(())
    })
    .expect("same-section media should be left alone");
}

/// The other reported case: the same person on Instagram and TikTok posting the
/// same video. Only reachable because the two profiles share an identity.
#[test]
fn the_same_upload_on_two_providers_is_grouped_through_the_identity() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_profiles(
            connection,
            test_layout,
            &[("source-1", "instagram"), ("source-2", "tiktok")],
        )?;
        connection
            .execute(
                "INSERT INTO identities (id, display_name, created_at, updated_at)
                 VALUES ('identity-1', 'Creator', '2026-03-01T00:00:00Z', '2026-03-01T00:00:00Z')",
                [],
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE source_profiles SET identity_id = 'identity-1'
                 WHERE id IN ('source-1', 'source-2')",
                [],
            )
            .map_err(|error| error.to_string())?;

        let frames = signature(&["aaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbb", "cccccccccccccccc"]);
        index_with_fingerprint(
            connection, test_layout, "source-1", "instagram", "ig.mp4", "timeline",
            "video", None, Some(&frames), 1_000, 8_000,
        )?;
        index_with_fingerprint(
            connection, test_layout, "source-2", "tiktok", "tt.mp4", "timeline",
            "video", None, Some(&frames), 4_000, 6_000,
        )?;

        let outcome = detect_variants_with_connection(
            connection,
            &["source-1".to_string(), "source-2".to_string()],
            Some("identity-1"),
            "2026-03-12T00:00:00Z",
        )?;
        assert_eq!(outcome.groups_created, 1);

        let groups = load_variant_groups_with_connection(connection, test_layout, 10)?;
        assert_eq!(groups[0].scope, "cross_source");
        assert_eq!(groups[0].identity_id.as_deref(), Some("identity-1"));
        assert_eq!(groups[0].members.len(), 2);
        Ok(())
    })
    .expect("cross-provider grouping should work");
}

/// A cross-post happens within hours. Two uploads months apart are separate
/// events even when the content matches.
#[test]
fn cross_provider_matching_respects_the_time_window() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_profiles(
            connection,
            test_layout,
            &[("source-1", "instagram"), ("source-2", "tiktok")],
        )?;
        let frames = signature(&["aaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbb"]);
        index_with_fingerprint(
            connection, test_layout, "source-1", "instagram", "ig.mp4", "timeline",
            "video", None, Some(&frames), 1_000_000, 8_000,
        )?;
        index_with_fingerprint(
            connection, test_layout, "source-2", "tiktok", "tt.mp4", "timeline",
            "video", None, Some(&frames), 9_000_000, 6_000,
        )?;

        let outcome = detect_variants_with_connection(
            connection,
            &["source-1".to_string(), "source-2".to_string()],
            None,
            "2026-03-12T00:00:00Z",
        )?;
        assert_eq!(outcome.groups_created, 0);
        Ok(())
    })
    .expect("time window should apply");
}

#[test]
fn images_are_grouped_by_perceptual_hash_across_sections() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_profiles(connection, test_layout, &[("source-1", "instagram")])?;
        index_with_fingerprint(
            connection, test_layout, "source-1", "instagram", "story.jpg", "stories",
            "image", Some("ffffffffffffffff"), None, 1_000, 3_000,
        )?;
        index_with_fingerprint(
            connection, test_layout, "source-1", "instagram", "feed.jpg", "timeline",
            "image", Some("fffffffffffffffe"), None, 2_000, 7_000,
        )?;
        // Visually unrelated: must not join the group.
        index_with_fingerprint(
            connection, test_layout, "source-1", "instagram", "other.jpg", "timeline",
            "image", Some("0000000000000000"), None, 3_000, 3_000,
        )?;

        let outcome = detect_variants_with_connection(
            connection,
            &["source-1".to_string()],
            None,
            "2026-03-12T00:00:00Z",
        )?;
        assert_eq!(outcome.groups_created, 1);
        assert_eq!(outcome.media_grouped, 2);
        Ok(())
    })
    .expect("image grouping should work");
}

#[test]
fn grouping_collapses_variants_in_the_timeline_and_dismissing_restores_them() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_profiles(connection, test_layout, &[("source-1", "instagram")])?;
        let frames = signature(&["aaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbb", "cccccccccccccccc"]);
        index_with_fingerprint(
            connection, test_layout, "source-1", "instagram", "story.mp4", "stories",
            "video", None, Some(&frames), 1_000, 5_000,
        )?;
        index_with_fingerprint(
            connection, test_layout, "source-1", "instagram", "feed.mp4", "timeline",
            "video", None, Some(&frames), 2_000, 9_000,
        )?;
        detect_variants_with_connection(
            connection,
            &["source-1".to_string()],
            None,
            "2026-03-12T00:00:00Z",
        )?;

        let page = load_media_timeline_with_connection(
            connection,
            test_layout,
            MediaTimelineRequest {
                filter: MediaTimelineFilter::default(),
                cursor: None,
                limit: None,
            },
        )?;
        assert_eq!(
            page.items.len(),
            1,
            "the repost is collapsed into the canonical copy"
        );
        assert_eq!(page.items[0].relative_path, "feed.mp4");

        let group_id = load_variant_groups_with_connection(connection, test_layout, 1)?[0].id.clone();
        connection
            .execute(
                "UPDATE media_index SET variant_group_id = NULL, is_canonical = 1
                 WHERE variant_group_id = ?1",
                params![group_id],
            )
            .map_err(|error| error.to_string())?;

        let restored = load_media_timeline_with_connection(
            connection,
            test_layout,
            MediaTimelineRequest {
                filter: MediaTimelineFilter::default(),
                cursor: None,
                limit: None,
            },
        )?;
        assert_eq!(restored.items.len(), 2, "dismissing brings both copies back");
        Ok(())
    })
    .expect("collapse and restore should work");
}

#[test]
fn media_without_a_fingerprint_is_never_matched() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_profiles(connection, test_layout, &[("source-1", "instagram")])?;
        let frames = signature(&["aaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbb"]);
        index_with_fingerprint(
            connection, test_layout, "source-1", "instagram", "story.mp4", "stories",
            "video", None, Some(&frames), 1_000, 5_000,
        )?;
        index_with_fingerprint(
            connection, test_layout, "source-1", "instagram", "feed.mp4", "timeline",
            "video", None, Some(&frames), 2_000, 9_000,
        )?;
        connection
            .execute(
                "UPDATE media_index SET fingerprint_status = 'pending'
                 WHERE relative_path = 'feed.mp4'",
                [],
            )
            .map_err(|error| error.to_string())?;

        let outcome = detect_variants_with_connection(
            connection,
            &["source-1".to_string()],
            None,
            "2026-03-12T00:00:00Z",
        )?;
        assert_eq!(
            outcome.groups_created, 0,
            "an un-hashed file is not comparable yet; guessing would be worse than waiting"
        );
        Ok(())
    })
    .expect("pending fingerprints should be skipped");
}
