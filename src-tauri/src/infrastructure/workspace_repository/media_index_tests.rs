use super::tests::{create_test_layout, sample_account, sample_source};
use super::*;

fn downloaded_media_fixture(
    profile_root: &Path,
    relative_name: &str,
    contents: &str,
) -> twitter_connector::DownloadedTwitterMedia {
    let file_path = profile_root.join(relative_name);
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).expect("media folder");
    }
    fs::write(&file_path, contents).expect("media file");
    twitter_connector::DownloadedTwitterMedia {
        file_path,
        media_type: "video".to_string(),
        media_section: "timeline".to_string(),
        provider_media_key: "media-key-1".to_string(),
        provider_post_key: "post-key-1".to_string(),
        captured_at_timestamp: Some(1_760_000_000),
        final_file_name: relative_name.to_string(),
    }
}

struct IndexedRow {
    id: String,
    relative_path: String,
    normalized_path: String,
    size_bytes: i64,
    modified_at_ms: i64,
    sha256: Option<String>,
    fingerprint_status: String,
}

fn media_index_row(connection: &Connection, source_id: &str) -> IndexedRow {
    connection
        .query_row(
            "SELECT id, relative_path, normalized_path, size_bytes, modified_at_ms,
                    sha256, fingerprint_status
             FROM media_index WHERE source_id = ?1",
            params![source_id],
            |row| {
                Ok(IndexedRow {
                    id: row.get(0)?,
                    relative_path: row.get(1)?,
                    normalized_path: row.get(2)?,
                    size_bytes: row.get(3)?,
                    modified_at_ms: row.get(4)?,
                    sha256: row.get(5)?,
                    fingerprint_status: row.get(6)?,
                })
            },
        )
        .expect("indexed media row")
}

fn index_downloaded_media(
    connection: &Connection,
    profile_root: &Path,
    media: &twitter_connector::DownloadedTwitterMedia,
    timestamp: &str,
) -> Result<(), String> {
    upsert_provider_sync_media_ledger_entries(
        connection,
        &ProviderSyncMediaScope {
            provider: "twitter",
            source_id: "source-1",
            account_id: "account-1",
            source_handle: "@source-1",
            profile_root,
            timestamp,
        },
        std::slice::from_ref(media),
    )
}

fn seed_indexable_source(connection: &Connection, layout: &StorageLayout) -> Result<(), String> {
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

#[test]
fn downloaded_media_is_registered_in_the_canonical_index() {
    let (temp_dir, layout) = create_test_layout();
    let profile_root = temp_dir
        .path()
        .join("media")
        .join("twitter")
        .join("source-1");

    with_workspace_layout(layout, |connection, test_layout| {
        seed_indexable_source(connection, test_layout)?;
        let media = downloaded_media_fixture(&profile_root, "video.mp4", "first-revision");
        index_downloaded_media(connection, &profile_root, &media, "2026-03-10T00:00:00Z")?;

        let row = media_index_row(connection, "source-1");
        assert!(!row.id.is_empty(), "index rows carry an opaque stable id");
        assert_eq!(row.relative_path, "video.mp4");
        assert_eq!(
            row.normalized_path,
            normalize_absolute_media_path(&media.file_path),
            "normalized path must match the dedupe catalog shape so hashes can be inherited"
        );
        assert_eq!(row.size_bytes, "first-revision".len() as i64);
        assert!(
            row.sha256.is_none(),
            "the ingest hook never hashes inline; that is the indexing run's job"
        );
        assert_eq!(row.fingerprint_status, "pending");

        let counts = media_index_counts(connection)?;
        assert_eq!(counts.total_files, 1);
        assert_eq!(counts.pending_fingerprints, 1);
        assert_eq!(counts.indexed_sources, 1);
        Ok(())
    })
    .expect("indexing downloaded media should succeed");
}

#[test]
fn reindexing_an_unchanged_file_keeps_its_id_and_fingerprint() {
    let (temp_dir, layout) = create_test_layout();
    let profile_root = temp_dir
        .path()
        .join("media")
        .join("twitter")
        .join("source-1");

    with_workspace_layout(layout, |connection, test_layout| {
        seed_indexable_source(connection, test_layout)?;
        let media = downloaded_media_fixture(&profile_root, "video.mp4", "stable-bytes");
        index_downloaded_media(connection, &profile_root, &media, "2026-03-10T00:00:00Z")?;

        let original = media_index_row(connection, "source-1");
        connection
            .execute(
                "UPDATE media_index SET sha256 = 'abc', fingerprint_status = 'complete'
                 WHERE id = ?1",
                params![original.id],
            )
            .map_err(|error| error.to_string())?;

        index_downloaded_media(connection, &profile_root, &media, "2026-03-11T00:00:00Z")?;

        let row = media_index_row(connection, "source-1");
        assert_eq!(
            row.id, original.id,
            "the id is what collections and variant groups reference; it must survive a re-sync"
        );
        assert_eq!(row.sha256.as_deref(), Some("abc"));
        assert_eq!(row.fingerprint_status, "complete");
        Ok(())
    })
    .expect("reindexing should succeed");
}

#[test]
fn reindexing_a_changed_file_drops_the_stale_fingerprint() {
    let (temp_dir, layout) = create_test_layout();
    let profile_root = temp_dir
        .path()
        .join("media")
        .join("twitter")
        .join("source-1");

    with_workspace_layout(layout, |connection, test_layout| {
        seed_indexable_source(connection, test_layout)?;
        let media = downloaded_media_fixture(&profile_root, "video.mp4", "first-revision");
        index_downloaded_media(connection, &profile_root, &media, "2026-03-10T00:00:00Z")?;

        let original = media_index_row(connection, "source-1");
        connection
            .execute(
                "UPDATE media_index
                 SET sha256 = 'abc', ahash64 = 'aa', dhash64 = 'dd',
                     fingerprint_status = 'complete', variant_group_id = 'group-1',
                     is_canonical = 0
                 WHERE id = ?1",
                params![original.id],
            )
            .map_err(|error| error.to_string())?;

        // Same path, different bytes: a re-download that replaced the file.
        let replaced =
            downloaded_media_fixture(&profile_root, "video.mp4", "second-revision-is-longer");
        index_downloaded_media(connection, &profile_root, &replaced, "2026-03-11T00:00:00Z")?;

        let row = media_index_row(connection, "source-1");
        assert_eq!(row.id, original.id, "identity survives a content change");
        assert_eq!(row.size_bytes, "second-revision-is-longer".len() as i64);
        assert!(
            row.sha256.is_none(),
            "a hash of the previous bytes would be worse than no hash at all"
        );
        assert_eq!(row.fingerprint_status, "pending");

        let (variant_group_id, is_canonical) = connection
            .query_row(
                "SELECT variant_group_id, is_canonical FROM media_index WHERE id = ?1",
                params![row.id],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?)),
            )
            .map_err(|error| error.to_string())?;
        assert!(
            variant_group_id.is_none(),
            "variant grouping was derived from bytes that no longer exist"
        );
        assert_eq!(is_canonical, 1);
        Ok(())
    })
    .expect("reindexing a replaced file should succeed");
}

/// Reconciliation is the only path that reaches media this app never
/// downloaded, which is the bulk of an imported SCrawler/4K Stogram library.
#[test]
fn reconciliation_indexes_files_that_no_sync_ever_registered() {
    let (_temp_dir, layout) = create_test_layout();

    with_workspace_layout(layout, |connection, test_layout| {
        seed_indexable_source(connection, test_layout)?;
        let source = load_sources(connection)?
            .into_iter()
            .find(|candidate| candidate.id == "source-1")
            .expect("seeded source");
        let root =
            resolved_source_media_output_root_with_connection(connection, test_layout, &source)?;
        fs::create_dir_all(root.join("stories")).expect("profile folders");
        fs::write(root.join("imported.jpg"), "image-bytes").expect("image");
        fs::write(root.join("stories").join("story.mp4"), "video-bytes").expect("video");
        // Auxiliary slideshow track: part of another post, not an item of its own.
        fs::write(root.join("post_audio.mp3"), "audio-bytes").expect("audio");
        // App-managed folder, never user media.
        fs::create_dir_all(root.join(".thumbs")).expect("cache folder");
        fs::write(root.join(".thumbs").join("cached.jpg"), "thumb").expect("thumb");

        let outcome = reconcile_source_media_index_with_connection(
            connection,
            test_layout,
            &source,
            "2026-03-10T00:00:00Z",
        )?;
        assert_eq!(outcome.indexed, 2);
        assert_eq!(outcome.updated, 0);
        assert_eq!(outcome.missing, 0);

        let mut indexed: Vec<(String, String)> = connection
            .prepare("SELECT relative_path, media_type FROM media_index WHERE source_id = 'source-1'")
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                    .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            })
            .map_err(|error| error.to_string())?;
        indexed.sort();
        assert_eq!(
            indexed,
            vec![
                ("imported.jpg".to_string(), "image".to_string()),
                ("stories/story.mp4".to_string(), "video".to_string()),
            ]
        );
        Ok(())
    })
    .expect("reconciliation should succeed");
}

#[test]
fn reconciliation_flags_files_that_vanished_from_disk_and_clears_the_flag_when_they_return() {
    let (_temp_dir, layout) = create_test_layout();

    with_workspace_layout(layout, |connection, test_layout| {
        seed_indexable_source(connection, test_layout)?;
        let source = load_sources(connection)?
            .into_iter()
            .find(|candidate| candidate.id == "source-1")
            .expect("seeded source");
        let root =
            resolved_source_media_output_root_with_connection(connection, test_layout, &source)?;
        fs::create_dir_all(&root).expect("profile folder");
        let file_path = root.join("photo.jpg");
        fs::write(&file_path, "image-bytes").expect("image");

        reconcile_source_media_index_with_connection(
            connection,
            test_layout,
            &source,
            "2026-03-10T00:00:00Z",
        )?;
        let indexed_id: String = connection
            .query_row(
                "SELECT id FROM media_index WHERE source_id = 'source-1'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;

        fs::remove_file(&file_path).expect("remove media");
        let outcome = reconcile_source_media_index_with_connection(
            connection,
            test_layout,
            &source,
            "2026-03-11T00:00:00Z",
        )?;
        assert_eq!(outcome.missing, 1);
        assert_eq!(
            media_index_counts(connection)?.missing_on_disk,
            1,
            "a file gone from disk is flagged, not deleted from the index"
        );

        fs::write(&file_path, "image-bytes").expect("restore media");
        reconcile_source_media_index_with_connection(
            connection,
            test_layout,
            &source,
            "2026-03-12T00:00:00Z",
        )?;
        assert_eq!(media_index_counts(connection)?.missing_on_disk, 0);
        let restored_id: String = connection
            .query_row(
                "SELECT id FROM media_index WHERE source_id = 'source-1'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        assert_eq!(
            restored_id, indexed_id,
            "a restored file keeps the identity anything else references"
        );
        Ok(())
    })
    .expect("missing-file reconciliation should succeed");
}

/// Files a sync did register keep their provider identity when the row is
/// rebuilt from disk — otherwise reconciliation would quietly strip post keys.
#[test]
fn reconciliation_recovers_provider_identity_from_the_sync_ledger() {
    let (_temp_dir, layout) = create_test_layout();

    with_workspace_layout(layout, |connection, test_layout| {
        seed_indexable_source(connection, test_layout)?;
        let source = load_sources(connection)?
            .into_iter()
            .find(|candidate| candidate.id == "source-1")
            .expect("seeded source");
        let root =
            resolved_source_media_output_root_with_connection(connection, test_layout, &source)?;
        let media = downloaded_media_fixture(&root, "video.mp4", "synced-bytes");
        index_downloaded_media(connection, &root, &media, "2026-03-10T00:00:00Z")?;
        connection
            .execute("DELETE FROM media_index", [])
            .map_err(|error| error.to_string())?;

        reconcile_source_media_index_with_connection(
            connection,
            test_layout,
            &source,
            "2026-03-11T00:00:00Z",
        )?;

        let (media_key, post_key, section, captured_at) = connection
            .query_row(
                "SELECT provider_media_key, provider_post_key, media_section, captured_at
                 FROM media_index WHERE source_id = 'source-1'",
                [],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .map_err(|error| error.to_string())?;
        assert_eq!(media_key.as_deref(), Some("media-key-1"));
        assert_eq!(post_key.as_deref(), Some("post-key-1"));
        assert_eq!(section, "timeline");
        assert_eq!(captured_at, Some(1_760_000_000));
        Ok(())
    })
    .expect("ledger identity recovery should succeed");
}

#[test]
fn pending_fingerprints_are_inherited_from_the_dedupe_catalog() {
    let (temp_dir, layout) = create_test_layout();
    let profile_root = temp_dir
        .path()
        .join("media")
        .join("twitter")
        .join("source-1");

    with_workspace_layout(layout, |connection, test_layout| {
        seed_indexable_source(connection, test_layout)?;
        let media = downloaded_media_fixture(&profile_root, "video.mp4", "already-hashed");
        index_downloaded_media(connection, &profile_root, &media, "2026-03-10T00:00:00Z")?;

        let row = media_index_row(connection, "source-1");
        connection
            .execute(
                "INSERT INTO media_dedupe_catalog (
                    normalized_path, path, source_id, provider, root_path, volume_key,
                    media_type, size_bytes, modified_at_ms, sha256, width, height,
                    duration_ms, ahash64, dhash64, hash_status, last_seen_scan_id, updated_at
                 ) VALUES (?1, ?2, 'source-1', 'twitter', ?3, 'C:', 'video', ?4, ?5,
                           'sha-from-dedupe', 1920, 1080, 15000, 'aaaa', 'dddd',
                           'complete', 'scan-1', '2026-03-09T00:00:00Z')",
                params![
                    row.normalized_path,
                    media.file_path.to_string_lossy(),
                    profile_root.to_string_lossy(),
                    row.size_bytes,
                    row.modified_at_ms,
                ],
            )
            .map_err(|error| error.to_string())?;

        let inherited =
            inherit_media_index_fingerprints(connection, Some("source-1"), "2026-03-12T00:00:00Z")?;
        assert_eq!(
            inherited, 1,
            "a library the dedupe scan already read must not be hashed twice"
        );

        let refreshed = media_index_row(connection, "source-1");
        assert_eq!(refreshed.sha256.as_deref(), Some("sha-from-dedupe"));
        assert_eq!(refreshed.fingerprint_status, "complete");

        let (width, duration_ms) = connection
            .query_row(
                "SELECT width, duration_ms FROM media_index WHERE source_id = 'source-1'",
                [],
                |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<i64>>(1)?)),
            )
            .map_err(|error| error.to_string())?;
        assert_eq!(width, Some(1920));
        assert_eq!(duration_ms, Some(15000));
        Ok(())
    })
    .expect("inheriting fingerprints should succeed");
}

#[test]
fn fingerprints_are_not_inherited_from_a_different_file_revision() {
    let (temp_dir, layout) = create_test_layout();
    let profile_root = temp_dir
        .path()
        .join("media")
        .join("twitter")
        .join("source-1");

    with_workspace_layout(layout, |connection, test_layout| {
        seed_indexable_source(connection, test_layout)?;
        let media = downloaded_media_fixture(&profile_root, "video.mp4", "current-bytes");
        index_downloaded_media(connection, &profile_root, &media, "2026-03-10T00:00:00Z")?;

        let row = media_index_row(connection, "source-1");
        connection
            .execute(
                "INSERT INTO media_dedupe_catalog (
                    normalized_path, path, source_id, provider, root_path, volume_key,
                    media_type, size_bytes, modified_at_ms, sha256, hash_status,
                    last_seen_scan_id, updated_at
                 ) VALUES (?1, ?2, 'source-1', 'twitter', ?3, 'C:', 'video', ?4, ?5,
                           'stale-sha', 'complete', 'scan-1', '2026-03-09T00:00:00Z')",
                params![
                    row.normalized_path,
                    media.file_path.to_string_lossy(),
                    profile_root.to_string_lossy(),
                    row.size_bytes + 4096,
                    row.modified_at_ms,
                ],
            )
            .map_err(|error| error.to_string())?;

        let inherited =
            inherit_media_index_fingerprints(connection, Some("source-1"), "2026-03-12T00:00:00Z")?;
        assert_eq!(
            inherited, 0,
            "the catalog entry describes a different revision of the file"
        );

        let refreshed = media_index_row(connection, "source-1");
        assert!(refreshed.sha256.is_none());
        assert_eq!(refreshed.fingerprint_status, "pending");
        Ok(())
    })
    .expect("inheritance guard should succeed");
}

#[test]
fn fingerprint_planner_only_queues_eligible_missing_results() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_indexable_source(connection, test_layout)?;
        let source = load_sources(connection)?
            .into_iter()
            .find(|source| source.id == "source-1")
            .expect("source");
        let root = resolved_source_media_output_root_with_connection(connection, test_layout, &source)?;
        fs::create_dir_all(root.join("stories")).expect("stories");
        for (relative, section, contents) in [
            ("stories/story.jpg", "stories", "story-bytes"),
            ("feed.jpg", "timeline", "different-feed-bytes"),
        ] {
            let path = root.join(relative);
            fs::write(&path, contents).expect("media");
            upsert_media_index_entry(
                connection,
                "twitter",
                "source-1",
                &MediaIndexEntry {
                    relative_path: relative,
                    absolute_path: &path,
                    media_type: "image",
                    media_section: section,
                    provider_media_key: Some(relative),
                    provider_post_key: Some(relative),
                    captured_at: Some(1_000),
                },
                "2026-03-10T00:00:00Z",
            )?;
        }

        // One existing perceptual result must be preserved and represented by
        // a complete job; only the missing peer is pending heavy work.
        connection
            .execute(
                "UPDATE media_index SET ahash64 = 'aaaaaaaaaaaaaaaa', dhash64 = 'aaaaaaaaaaaaaaaa'
                 WHERE relative_path = 'feed.jpg'",
                [],
            )
            .map_err(|error| error.to_string())?;
        let planned = plan_media_fingerprint_jobs_with_connection(connection)?;
        assert_eq!(planned.perceptual_image, 1);
        assert_eq!(planned.perceptual_video, 0);

        let complete_images: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM media_fingerprint_jobs
                 WHERE kind = 'perceptual_image' AND status = 'complete'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        assert_eq!(complete_images, 1, "backfill records existing hashes without rehashing");

        // Once both rows are in the same section they are no longer eligible;
        // stale jobs are removed instead of silently widening the policy.
        connection
            .execute(
                "UPDATE media_index SET media_section = 'stories' WHERE relative_path = 'feed.jpg'",
                [],
            )
            .map_err(|error| error.to_string())?;
        let replanned = plan_media_fingerprint_jobs_with_connection(connection)?;
        assert_eq!(replanned.pending(), 0);
        let jobs: i64 = connection
            .query_row("SELECT COUNT(*) FROM media_fingerprint_jobs", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        assert_eq!(jobs, 0, "section changes invalidate candidate jobs");
        Ok(())
    })
    .expect("candidate planning should succeed");
}

#[test]
fn changed_files_cannot_publish_a_leased_fingerprint() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_indexable_source(connection, test_layout)?;
        let source = load_sources(connection)?
            .into_iter()
            .find(|source| source.id == "source-1")
            .expect("source");
        let root = resolved_source_media_output_root_with_connection(connection, test_layout, &source)?;
        fs::create_dir_all(root.join("stories")).expect("stories");
        for (relative, section) in [("stories/a.jpg", "stories"), ("b.jpg", "timeline")] {
            let path = root.join(relative);
            fs::write(&path, "same-size").expect("media");
            upsert_media_index_entry(
                connection,
                "twitter",
                "source-1",
                &MediaIndexEntry {
                    relative_path: relative,
                    absolute_path: &path,
                    media_type: "image",
                    media_section: section,
                    provider_media_key: Some(relative),
                    provider_post_key: Some(relative),
                    captured_at: Some(1_000),
                },
                "2026-03-10T00:00:00Z",
            )?;
        }
        plan_media_fingerprint_jobs_with_connection(connection)?;
        let mut leased = lease_pending_fingerprints_with_connection(
            connection,
            test_layout,
            "perceptual_image",
            1,
            "test-run",
        )?;
        let item = leased.pop().expect("leased candidate");
        connection
            .execute(
                "UPDATE media_index SET modified_at_ms = modified_at_ms + 1 WHERE id = ?1",
                params![item.id],
            )
            .map_err(|error| error.to_string())?;
        let stored = complete_media_fingerprint_job_with_connection(
            connection,
            &item,
            Some("sha"),
            Some("aaaa"),
            Some("dddd"),
            None,
            Some(100),
            Some(100),
        )?;
        assert!(!stored, "size/mtime guard rejects results for replaced bytes");
        let status: String = connection
            .query_row(
                "SELECT status FROM media_fingerprint_jobs WHERE media_id = ?1 AND kind = ?2",
                params![item.id, item.kind],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        assert_eq!(status, "queued");
        Ok(())
    })
    .expect("lease guard should succeed");
}
