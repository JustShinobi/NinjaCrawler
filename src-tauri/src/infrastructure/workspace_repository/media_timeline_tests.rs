use super::tests::{create_test_layout, sample_account, sample_source};
use super::*;

/// Seeds two profiles and indexes media for them, all inside the hermetic test
/// layout — the timeline reads through the same connection the test owns.
fn index_media(
    connection: &Connection,
    layout: &StorageLayout,
    source_id: &str,
    relative_path: &str,
    media_type: &str,
    post_key: Option<&str>,
    captured_at: i64,
) -> Result<(), String> {
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
            media_type,
            media_section: "timeline",
            provider_media_key: Some(relative_path),
            provider_post_key: post_key,
            captured_at: Some(captured_at),
        },
        "2026-03-10T00:00:00Z",
    )
}

fn seed_two_profiles(connection: &Connection, layout: &StorageLayout) -> Result<(), String> {
    upsert_provider_account_with_connection(
        connection,
        layout,
        sample_account("account-1", "twitter"),
    )?;
    for id in ["source-1", "source-2"] {
        upsert_source_profile_with_connection(
            connection,
            layout,
            sample_source(id, "twitter", Some("account-1")),
        )?;
    }
    Ok(())
}

fn request(filter: MediaTimelineFilter, limit: Option<u32>) -> MediaTimelineRequest {
    MediaTimelineRequest {
        filter,
        cursor: None,
        limit,
    }
}

#[test]
fn the_timeline_merges_profiles_newest_first_and_collapses_a_post_into_one_card() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_two_profiles(connection, test_layout)?;
        // A two-file carousel on one profile and a single video on the other.
        index_media(connection, test_layout, "source-1", "a_0.jpg", "image", Some("post-a"), 1_000)?;
        index_media(connection, test_layout, "source-1", "a_1.jpg", "image", Some("post-a"), 1_000)?;
        index_media(connection, test_layout, "source-2", "b.mp4", "video", Some("post-b"), 2_000)?;

        let page = load_media_timeline_with_connection(
            connection,
            test_layout,
            request(MediaTimelineFilter::default(), None),
        )?;
        assert_eq!(page.items.len(), 2, "the carousel is a single card");
        assert_eq!(page.items[0].source_id, "source-2", "newest first");
        assert_eq!(page.items[0].media_type, "video");
        assert_eq!(page.items[1].source_id, "source-1");
        assert_eq!(page.items[1].file_count, 2);
        assert_eq!(page.items[1].media_type, "slideshow");
        assert!(
            page.items[1].absolute_path.ends_with("a_0.jpg"),
            "the card points at a real file on disk"
        );
        Ok(())
    })
    .expect("timeline should load");
}

#[test]
fn keyset_pagination_walks_the_whole_library_without_repeating_an_item() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_two_profiles(connection, test_layout)?;
        for index in 0..5 {
            index_media(
                connection,
                test_layout,
                "source-1",
                &format!("post-{index}.jpg"),
                "image",
                Some(&format!("post-{index}")),
                1_000 + i64::from(index),
            )?;
        }

        let mut seen: Vec<String> = Vec::new();
        let mut cursor = None;
        for _ in 0..10 {
            let page = load_media_timeline_with_connection(
                connection,
                test_layout,
                MediaTimelineRequest {
                    filter: MediaTimelineFilter::default(),
                    cursor: cursor.clone(),
                    limit: Some(2),
                },
            )?;
            seen.extend(page.items.iter().map(|item| item.id.clone()));
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }

        assert_eq!(seen.len(), 5, "every post is visited exactly once");
        let unique: HashSet<&String> = seen.iter().collect();
        assert_eq!(unique.len(), 5, "no post is returned twice");
        Ok(())
    })
    .expect("pagination should terminate");
}

#[test]
fn filters_narrow_the_timeline_by_profile_media_type_and_upstream_state() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_two_profiles(connection, test_layout)?;
        index_media(connection, test_layout, "source-1", "photo.jpg", "image", Some("p-1"), 1_000)?;
        index_media(connection, test_layout, "source-1", "clip.mp4", "video", Some("p-2"), 2_000)?;
        index_media(connection, test_layout, "source-2", "other.jpg", "image", Some("p-3"), 3_000)?;
        connection
            .execute(
                "UPDATE media_index SET upstream_state = 'missing' WHERE relative_path = 'photo.jpg'",
                [],
            )
            .map_err(|error| error.to_string())?;

        let by_source = load_media_timeline_with_connection(
            connection,
            test_layout,
            request(
                MediaTimelineFilter {
                    source_ids: vec!["source-1".to_string()],
                    ..Default::default()
                },
                None,
            ),
        )?;
        assert_eq!(by_source.items.len(), 2);

        let videos = load_media_timeline_with_connection(
            connection,
            test_layout,
            request(
                MediaTimelineFilter {
                    media_type: Some("video".to_string()),
                    ..Default::default()
                },
                None,
            ),
        )?;
        assert_eq!(videos.items.len(), 1);
        assert_eq!(videos.items[0].relative_path, "clip.mp4");

        let archived = load_media_timeline_with_connection(
            connection,
            test_layout,
            request(
                MediaTimelineFilter {
                    upstream_missing_only: true,
                    ..Default::default()
                },
                None,
            ),
        )?;
        assert_eq!(archived.items.len(), 1);
        assert!(archived.items[0].upstream_missing);
        Ok(())
    })
    .expect("filters should apply");
}

#[test]
fn media_gone_from_disk_stays_out_of_the_timeline() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_two_profiles(connection, test_layout)?;
        index_media(connection, test_layout, "source-1", "kept.jpg", "image", Some("p-1"), 1_000)?;
        index_media(connection, test_layout, "source-1", "gone.jpg", "image", Some("p-2"), 2_000)?;
        connection
            .execute(
                "UPDATE media_index SET local_state = 'missing_on_disk'
                 WHERE relative_path = 'gone.jpg'",
                [],
            )
            .map_err(|error| error.to_string())?;

        let page = load_media_timeline_with_connection(
            connection,
            test_layout,
            request(MediaTimelineFilter::default(), None),
        )?;
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].relative_path, "kept.jpg");
        Ok(())
    })
    .expect("missing media should be hidden");
}

#[test]
fn the_new_arrivals_counter_only_counts_media_downloaded_after_the_last_visit() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_two_profiles(connection, test_layout)?;
        index_media(connection, test_layout, "source-1", "old.jpg", "image", Some("p-1"), 1_000)?;

        // Never marked as seen: the counter stays at zero instead of claiming
        // the whole existing library is new.
        let before = load_media_timeline_with_connection(
            connection,
            test_layout,
            request(MediaTimelineFilter::default(), None),
        )?;
        assert_eq!(before.new_since_last_visit, 0);
        assert!(before.last_seen_at.is_none());

        mark_timeline_seen_with_connection(connection)?;
        let after = load_media_timeline_with_connection(
            connection,
            test_layout,
            request(MediaTimelineFilter::default(), None),
        )?;
        assert!(after.last_seen_at.is_some());
        assert_eq!(
            after.new_since_last_visit, 0,
            "media downloaded before the mark is not new"
        );

        // Media that lands after the mark is what the badge is for.
        connection
            .execute(
                "UPDATE media_index SET downloaded_at = ?1 WHERE relative_path = 'old.jpg'",
                params![Utc::now().timestamp() + 60],
            )
            .map_err(|error| error.to_string())?;
        let refreshed = load_media_timeline_with_connection(
            connection,
            test_layout,
            request(MediaTimelineFilter::default(), None),
        )?;
        assert_eq!(refreshed.new_since_last_visit, 1);
        Ok(())
    })
    .expect("new arrivals counter should work");
}
