use super::*;

/// A media row as 4K Stogram stores it, so tests read like the real catalog.
struct CatalogRow {
    instagram_id: &'static str,
    web_url: &'static str,
    media_url: &'static str,
    title: Option<&'static str>,
    created_time: i64,
    /// Catalog-relative path, with the backslashes 4K Stogram writes.
    file: &'static str,
    is_video: i64,
    state: i64,
}

/// Builds a `.stogram.sqlite` with one subscription plus its rows, and writes
/// every referenced file (unless listed in `skip_files`, to exercise the
/// missing-file path).
fn write_catalog(
    root: &Path,
    handle: &str,
    user_id: &str,
    rows: &[CatalogRow],
    skip_files: &[&str],
) {
    fs::create_dir_all(root).expect("stogram root");
    let connection = Connection::open(root.join(".stogram.sqlite")).expect("catalog");
    connection
        .execute_batch(
            "CREATE TABLE subscriptions (id BLOB PRIMARY KEY, query TEXT, type INTEGER,
                finished INTEGER, private INTEGER, date_added TEXT, instagram_id TEXT,
                overflow_behavior INTEGER, from_date_time INTEGER, to_date_time INTEGER,
                downloadPhotos INTEGER, downloadVideos INTEGER, downloadFeed INTEGER,
                downloadStories INTEGER, downloadHighlights INTEGER, downloadTaggedPosts INTEGER,
                downloadReels INTEGER, attributes TEXT, display_name TEXT,
                last_update_time INTEGER, initialized INTEGER, filterVersion INTEGER);
             CREATE TABLE photos (id INTEGER PRIMARY KEY AUTOINCREMENT, subscriptionId BLOB,
                instagram_id TEXT, web_url TEXT, thumbnail_url TEXT, media_url TEXT, title TEXT,
                is_video INTEGER, created_time INTEGER, thumbnail_file TEXT, file TEXT,
                state INTEGER, locationId TEXT, ownerName TEXT, ownerId TEXT, locationName TEXT);",
        )
        .expect("schema");

    let subscription_id = vec![1u8, 2, 3, 4];
    connection
        .execute(
            "INSERT INTO subscriptions (id, query, instagram_id, display_name,
                downloadFeed, downloadStories, downloadHighlights, downloadReels,
                downloadTaggedPosts)
             VALUES (?1, ?2, ?3, ?4, 1, 1, 1, 1, 0)",
            params![subscription_id, handle, user_id, handle],
        )
        .expect("subscription");

    for row in rows {
        connection
            .execute(
                "INSERT INTO photos (subscriptionId, instagram_id, web_url, media_url, title,
                    is_video, created_time, file, state)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    subscription_id,
                    row.instagram_id,
                    row.web_url,
                    row.media_url,
                    row.title,
                    row.is_video,
                    row.created_time,
                    row.file,
                    row.state,
                ],
            )
            .expect("photo");

        if skip_files.contains(&row.file) {
            continue;
        }
        let path = root.join(row.file.replace('\\', "/"));
        fs::create_dir_all(path.parent().expect("parent")).expect("media dir");
        // Distinct content per row so SHA-256 dedupe is meaningful, and padded
        // past the degraded-placeholder threshold so real media is never
        // mistaken for a thumbnail.
        let body = format!("content-of-{}", row.instagram_id);
        let padding = "x".repeat(STOGRAM_DEGRADED_MAX_BYTES as usize);
        fs::write(&path, format!("{body}{padding}")).expect("media file");
    }
}

fn seed_account(connection: &Connection, layout: &StorageLayout, id: &str) {
    upsert_provider_account_with_connection(
        connection,
        layout,
        ProviderAccountUpsert {
            id: Some(id.to_string()),
            provider: "instagram".to_string(),
            display_name: "tester".to_string(),
            auth_mode: "imported_session".to_string(),
            auth_state: "ready".to_string(),
            capabilities: vec!["posts".to_string()],
            last_validated_at: None,
        },
    )
    .expect("account");
}

fn seed_source(
    connection: &Connection,
    layout: &StorageLayout,
    id: &str,
    handle: &str,
    account_id: &str,
    user_id_hint: Option<&str>,
    special_path: &Path,
) -> SourceProfile {
    let mut instagram = default_instagram_source_sync_options();
    instagram.special_path = Some(special_path.display().to_string());
    instagram.user_id_hint = user_id_hint.map(|value| value.to_string());
    upsert_source_profile_with_connection(
        connection,
        layout,
        SourceProfileUpsert {
            id: Some(id.to_string()),
            provider: "instagram".to_string(),
            source_kind: "profile".to_string(),
            handle: handle.to_string(),
            display_name: handle.to_string(),
            account_id: Some(account_id.to_string()),
            group_id: None,
            labels: Vec::new(),
            ready_for_download: true,
            sync_options: SourceSyncOptions {
                instagram: Some(instagram),
                ..Default::default()
            },
            remote_state: None,
            is_subscription: None,
        },
    )
    .expect("source");
    load_sources(connection)
        .expect("sources")
        .into_iter()
        .find(|entry| entry.id == id)
        .expect("persisted source")
}

fn options(root: &Path, dry_run: bool) -> StogramMigrationOptions {
    StogramMigrationOptions {
        stogram_root: root.to_path_buf(),
        account: Some("tester".to_string()),
        handles: Vec::new(),
        limit: None,
        dry_run,
    }
}

fn migrate(
    connection: &Connection,
    layout: &StorageLayout,
    options: &StogramMigrationOptions,
) -> StogramMigrationReport {
    run_stogram_migration_with_connection(connection, layout, options, &mut |_| {})
        .expect("migration")
}

fn ledger_rows(connection: &Connection, source_id: &str) -> Vec<(String, String, String)> {
    let mut statement = connection
        .prepare(
            "SELECT provider_media_key, media_section, relative_path
             FROM instagram_sync_media_ledger WHERE source_id = ?1 ORDER BY relative_path",
        )
        .expect("prepare");
    let rows = statement
        .query_map(params![source_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .expect("query");
    rows.map(|row| row.expect("row")).collect()
}

#[test]
fn is_video_bitfield_maps_to_sections_and_avatars() {
    for (is_video, expected) in [
        (0, "timeline"),
        (2, "timeline"),
        (3, "timeline"),
        (4, "stories_user"),
        (5, "stories_user"),
        (16, "stories"),
        (17, "stories"),
        (65, "reels"),
    ] {
        match classify_stogram_media(is_video) {
            StogramMediaKind::Section(section) => assert_eq!(section, expected, "for {is_video}"),
            StogramMediaKind::Avatar => panic!("{is_video} should not be an avatar"),
        }
    }
    assert!(matches!(
        classify_stogram_media(8),
        StogramMediaKind::Avatar
    ));
}

#[test]
fn migrates_a_new_profile_with_sections_caption_and_avatar() {
    let temp = tempfile::tempdir().expect("temp dir");
    let stogram_root = temp.path().join("4K Stogram");
    write_catalog(
        &stogram_root,
        "newbie",
        "555",
        &[
            CatalogRow {
                instagram_id: "111_555",
                web_url: "https://www.instagram.com/p/AbCdEf/",
                media_url: "https://cdn.example/v/t51/1_2_3_n.jpg?oe=1",
                title: Some("hello world"),
                created_time: 1_600_000_000,
                file: "newbie\\2020-09-13 09.06.40 111_555.jpg",
                is_video: 0,
                state: 4,
            },
            CatalogRow {
                instagram_id: "222_555",
                web_url: "https://www.instagram.com/p/GhIjKl/",
                media_url: "https://cdn.example/o1/v/t16/video_dashinit.mp4",
                title: None,
                created_time: 1_600_000_100,
                file: "newbie\\reels\\2020-09-13 09.08.20 222_555.mp4",
                is_video: 65,
                state: 4,
            },
            CatalogRow {
                instagram_id: "333_555",
                web_url: "https://www.instagram.com/stories/newbie/333/",
                media_url: "https://cdn.example/v/t51/4_5_6_n.jpg",
                title: None,
                created_time: 1_600_000_200,
                file: "newbie\\story\\2020-09-13 09.10.00 333_555.jpg",
                is_video: 4,
                state: 4,
            },
            CatalogRow {
                instagram_id: "444_555",
                web_url: "https://www.instagram.com/newbie",
                media_url: "https://cdn.example/v/t51/7_8_9_n.jpg",
                title: None,
                created_time: 0,
                file: "newbie\\1969-12-31 21.00.00 444_555.jpg",
                is_video: 8,
                state: 4,
            },
            // Not downloaded: must be ignored entirely.
            CatalogRow {
                instagram_id: "999_555",
                web_url: "https://www.instagram.com/p/ZzZzZz/",
                media_url: "https://cdn.example/v/t51/9_9_9_n.jpg",
                title: None,
                created_time: 1_600_000_300,
                file: "newbie\\2020-09-13 09.20.00 999_555.jpg",
                is_video: 0,
                state: 6,
            },
        ],
        &[],
    );

    let layout = storage::workspace_layout_from_roots(
        temp.path().join("localappdata"),
        temp.path().join("userprofile"),
    )
    .expect("layout");
    with_workspace_layout(layout, |connection, test_layout| {
        seed_account(connection, test_layout, "account-1");
        let report = migrate(connection, test_layout, &options(&stogram_root, false));

        assert_eq!(report.profiles_created, 1);
        assert_eq!(report.profiles_merged, 0);
        assert_eq!(report.profiles_failed, 0);
        assert_eq!(report.media_copied, 3, "the state=6 row must be skipped");
        assert_eq!(report.avatars_promoted, 1);
        assert_eq!(report.avatars_archived, 0);

        let source = load_sources(connection)?
            .into_iter()
            .find(|entry| entry.handle == "newbie")
            .expect("migrated source");
        assert_eq!(
            source_instagram_sync_options(&source).user_id_hint.as_deref(),
            Some("555"),
            "a brand-new profile stores the 4K Stogram user id"
        );

        let rows = ledger_rows(connection, &source.id);
        assert_eq!(rows.len(), 3);
        let sections = rows
            .iter()
            .map(|(_, section, path)| (section.as_str(), path.as_str()))
            .collect::<HashMap<_, _>>();
        assert_eq!(
            sections.get("timeline"),
            Some(&"2020-09-13 09.06.40 111_555.jpg")
        );
        assert_eq!(
            sections.get("reels"),
            Some(&"video/2020-09-13 09.08.20 222_555.mp4")
        );
        assert_eq!(
            sections.get("stories_user"),
            Some(&"stories (user)/2020-09-13 09.10.00 333_555.jpg")
        );

        let (title, captured_at, post_code) = connection
            .query_row(
                "SELECT title, captured_at, provider_post_code FROM instagram_sync_media_ledger
                 WHERE source_id = ?1 AND relative_path = ?2",
                params![source.id, "2020-09-13 09.06.40 111_555.jpg"],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .map_err(|error| error.to_string())?;
        assert_eq!(title.as_deref(), Some("hello world"));
        assert_eq!(captured_at, Some(1_600_000_000));
        assert_eq!(post_code.as_deref(), Some("AbCdEf"), "casing is preserved");

        // The media key comes from the file name (as in the SCrawler import),
        // so the bare media pk reaches the ledgers as an alias. That alias is
        // what makes a second run recognise this file instead of re-copying it.
        let alias_count = connection
            .query_row(
                "SELECT COUNT(*) FROM instagram_media_key_aliases
                 WHERE source_id = ?1 AND alias_key = '111_555'",
                params![source.id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| error.to_string())?;
        assert!(alias_count > 0, "the media pk must be seeded as an alias");

        // The avatar is not gallery media.
        assert!(
            !rows
                .iter()
                .any(|(_, _, path)| path.contains("444_555")),
            "the profile picture must stay out of the media ledger"
        );
        let profile_root = test_layout.media_root.join("instagram").join("newbie");
        assert!(settings_profile_picture_path(&profile_root).is_file());
        Ok(())
    })
    .expect("workspace");
}

#[test]
fn merges_by_user_id_when_the_handle_changed() {
    let temp = tempfile::tempdir().expect("temp dir");
    let stogram_root = temp.path().join("4K Stogram");
    write_catalog(
        &stogram_root,
        "old_handle",
        "777",
        &[CatalogRow {
            instagram_id: "111_777",
            web_url: "https://www.instagram.com/p/AbCdEf/",
            media_url: "https://cdn.example/v/t51/1_2_3_n.jpg",
            title: None,
            created_time: 1_600_000_000,
            file: "old_handle\\2020-09-13 09.06.40 111_777.jpg",
            is_video: 0,
            state: 4,
        }],
        &[],
    );

    let layout = storage::workspace_layout_from_roots(
        temp.path().join("localappdata"),
        temp.path().join("userprofile"),
    )
    .expect("layout");
    with_workspace_layout(layout, |connection, test_layout| {
        seed_account(connection, test_layout, "account-1");
        let special_path = test_layout.media_root.join("instagram").join("new_handle");
        let existing = seed_source(
            connection,
            test_layout,
            "source-1",
            "new_handle",
            "account-1",
            Some("777"),
            &special_path,
        );

        let report = migrate(connection, test_layout, &options(&stogram_root, false));
        assert_eq!(report.profiles_merged, 1);
        assert_eq!(report.profiles_created, 0);
        assert_eq!(report.profiles[0].matched_by, "user_id");

        let sources = load_sources(connection)?;
        assert_eq!(sources.len(), 1, "a rebrand must not create a second source");
        let source = &sources[0];
        assert_eq!(source.handle, "new_handle", "the current handle wins");

        let options = source_instagram_sync_options(source);
        assert_eq!(
            options.previous_handles.as_deref(),
            Some(["old_handle".to_string()].as_slice()),
            "the 4K Stogram handle is kept as a previous handle"
        );
        assert_eq!(
            options.special_path.as_deref(),
            Some(existing_special_path(&existing).as_str()),
            "merging must not move the profile root"
        );
        assert!(special_path
            .join("2020-09-13 09.06.40 111_777.jpg")
            .is_file());
        Ok(())
    })
    .expect("workspace");
}

fn existing_special_path(source: &SourceProfile) -> String {
    source_instagram_sync_options(source)
        .special_path
        .unwrap_or_default()
}

#[test]
fn handle_match_never_overwrites_the_workspace_user_id() {
    let temp = tempfile::tempdir().expect("temp dir");
    let stogram_root = temp.path().join("4K Stogram");
    write_catalog(
        &stogram_root,
        "recycled",
        "111111",
        &[CatalogRow {
            instagram_id: "111_111111",
            web_url: "https://www.instagram.com/p/AbCdEf/",
            media_url: "https://cdn.example/v/t51/1_2_3_n.jpg",
            title: None,
            created_time: 1_600_000_000,
            file: "recycled\\2020-09-13 09.06.40 111_111111.jpg",
            is_video: 0,
            state: 4,
        }],
        &[],
    );

    let layout = storage::workspace_layout_from_roots(
        temp.path().join("localappdata"),
        temp.path().join("userprofile"),
    )
    .expect("layout");
    with_workspace_layout(layout, |connection, test_layout| {
        seed_account(connection, test_layout, "account-1");
        let special_path = test_layout.media_root.join("instagram").join("recycled");
        seed_source(
            connection,
            test_layout,
            "source-1",
            "recycled",
            "account-1",
            // A different id: the account was recreated. The workspace is
            // authoritative, so this value must survive the migration.
            Some("999999"),
            &special_path,
        );

        let report = migrate(connection, test_layout, &options(&stogram_root, false));
        assert_eq!(report.profiles_merged, 1);
        assert_eq!(report.profiles[0].matched_by, "handle");

        let source = load_sources(connection)?.remove(0);
        assert_eq!(
            source_instagram_sync_options(&source).user_id_hint.as_deref(),
            Some("999999"),
            "the 4K Stogram id must never overwrite the workspace one"
        );
        Ok(())
    })
    .expect("workspace");
}

#[test]
fn handle_match_leaves_a_missing_user_id_empty() {
    let temp = tempfile::tempdir().expect("temp dir");
    let stogram_root = temp.path().join("4K Stogram");
    write_catalog(
        &stogram_root,
        "no_id_here",
        "424242",
        &[CatalogRow {
            instagram_id: "111_424242",
            web_url: "https://www.instagram.com/p/AbCdEf/",
            media_url: "https://cdn.example/v/t51/1_2_3_n.jpg",
            title: None,
            created_time: 1_600_000_000,
            file: "no_id_here\\2020-09-13 09.06.40 111_424242.jpg",
            is_video: 0,
            state: 4,
        }],
        &[],
    );

    let layout = storage::workspace_layout_from_roots(
        temp.path().join("localappdata"),
        temp.path().join("userprofile"),
    )
    .expect("layout");
    with_workspace_layout(layout, |connection, test_layout| {
        seed_account(connection, test_layout, "account-1");
        let special_path = test_layout.media_root.join("instagram").join("no_id_here");
        seed_source(
            connection,
            test_layout,
            "source-1",
            "no_id_here",
            "account-1",
            None,
            &special_path,
        );

        migrate(connection, test_layout, &options(&stogram_root, false));

        let source = load_sources(connection)?.remove(0);
        assert_eq!(
            source_instagram_sync_options(&source).user_id_hint,
            None,
            "an unconfirmed handle match must not backfill the id"
        );
        Ok(())
    })
    .expect("workspace");
}

#[test]
fn skips_media_already_cataloged_by_cdn_key_or_content_hash() {
    let temp = tempfile::tempdir().expect("temp dir");
    let stogram_root = temp.path().join("4K Stogram");
    write_catalog(
        &stogram_root,
        "dupes",
        "888",
        &[
            // Already known through the CDN file name embedded in media_url.
            CatalogRow {
                instagram_id: "111_888",
                web_url: "https://www.instagram.com/p/AbCdEf/",
                media_url: "https://cdn.example/v/t51/100_200_300_n.jpg?oe=7",
                title: None,
                created_time: 1_600_000_000,
                file: "dupes\\2020-09-13 09.06.40 111_888.jpg",
                is_video: 0,
                state: 4,
            },
            // A video has no CDN key, so only the content hash can catch it.
            CatalogRow {
                instagram_id: "222_888",
                web_url: "https://www.instagram.com/p/GhIjKl/",
                media_url: "https://cdn.example/o1/v/t16/video_dashinit.mp4",
                title: None,
                created_time: 1_600_000_100,
                file: "dupes\\reels\\2020-09-13 09.08.20 222_888.mp4",
                is_video: 65,
                state: 4,
            },
            CatalogRow {
                instagram_id: "333_888",
                web_url: "https://www.instagram.com/p/MnOpQr/",
                media_url: "https://cdn.example/v/t51/400_500_600_n.jpg",
                title: None,
                created_time: 1_600_000_200,
                file: "dupes\\2020-09-13 09.10.00 333_888.jpg",
                is_video: 0,
                state: 4,
            },
        ],
        &[],
    );

    let layout = storage::workspace_layout_from_roots(
        temp.path().join("localappdata"),
        temp.path().join("userprofile"),
    )
    .expect("layout");
    with_workspace_layout(layout, |connection, test_layout| {
        seed_account(connection, test_layout, "account-1");
        let special_path = test_layout.media_root.join("instagram").join("dupes");
        let source = seed_source(
            connection,
            test_layout,
            "source-1",
            "dupes",
            "account-1",
            Some("888"),
            &special_path,
        );

        let now = now_timestamp();
        ensure_instagram_sync_media_ledger_table(connection)?;
        ensure_instagram_media_fingerprints_table(connection)?;
        // Row 1 is known by its CDN key…
        connection
            .execute(
                "INSERT INTO instagram_sync_media_ledger (source_id, account_id, source_handle,
                    provider_media_key, media_type, media_section, relative_path,
                    first_seen_at, last_seen_at)
                 VALUES (?1, 'account-1', 'dupes', '100_200_300_n', 'image', 'timeline',
                         'existing.jpg', ?2, ?2)",
                params![source.id, now],
            )
            .map_err(|error| error.to_string())?;
        // …and row 2 by the SHA-256 of the bytes `write_catalog` produced.
        let video_sha = compute_file_sha256(
            &stogram_root.join("dupes/reels/2020-09-13 09.08.20 222_888.mp4"),
        )?;
        connection
            .execute(
                "INSERT INTO instagram_media_fingerprints (source_id, account_id,
                    provider_media_key, media_type, media_section, file_sha256,
                    first_seen_at, last_seen_at)
                 VALUES (?1, 'account-1', 'whatever', 'video', 'reels', ?2, ?3, ?3)",
                params![source.id, video_sha, now],
            )
            .map_err(|error| error.to_string())?;

        let report = migrate(connection, test_layout, &options(&stogram_root, false));
        assert_eq!(report.media_already_cataloged, 2);
        assert_eq!(report.media_copied, 1, "only the unknown photo is copied");
        assert!(special_path
            .join("2020-09-13 09.10.00 333_888.jpg")
            .is_file());
        assert!(!special_path
            .join("2020-09-13 09.06.40 111_888.jpg")
            .exists());
        Ok(())
    })
    .expect("workspace");
}

/// An interrupted run leaves files at the destination that no ledger knows
/// about. Re-running must re-register them instead of skipping them as
/// "already there", or they stay invisible to the gallery forever.
#[test]
fn recovers_files_left_behind_by_an_interrupted_run() {
    let temp = tempfile::tempdir().expect("temp dir");
    let stogram_root = temp.path().join("4K Stogram");
    write_catalog(
        &stogram_root,
        "interrupted",
        "606",
        &[CatalogRow {
            instagram_id: "111_606",
            web_url: "https://www.instagram.com/p/AbCdEf/",
            media_url: "https://cdn.example/v/t51/1_2_3_n.jpg",
            title: None,
            created_time: 1_600_000_000,
            file: "interrupted\\2020-09-13 09.06.40 111_606.jpg",
            is_video: 0,
            state: 4,
        }],
        &[],
    );

    let layout = storage::workspace_layout_from_roots(
        temp.path().join("localappdata"),
        temp.path().join("userprofile"),
    )
    .expect("layout");
    with_workspace_layout(layout, |connection, test_layout| {
        seed_account(connection, test_layout, "account-1");
        let special_path = test_layout.media_root.join("instagram").join("interrupted");
        seed_source(
            connection,
            test_layout,
            "source-1",
            "interrupted",
            "account-1",
            Some("606"),
            &special_path,
        );

        // Simulate the leftover: the exact file already copied, nothing in the
        // ledgers.
        fs::create_dir_all(&special_path).expect("profile root");
        fs::copy(
            stogram_root.join("interrupted/2020-09-13 09.06.40 111_606.jpg"),
            special_path.join("2020-09-13 09.06.40 111_606.jpg"),
        )
        .expect("leftover file");

        let report = migrate(connection, test_layout, &options(&stogram_root, false));
        assert_eq!(report.media_copied, 0, "the file is already in place");
        assert_eq!(report.media_recovered, 1);
        assert_eq!(
            ledger_rows(connection, "source-1").len(),
            1,
            "the leftover must reach the ledger"
        );
        Ok(())
    })
    .expect("workspace");
}

/// The gallery reads a highlight's album from the SECOND path segment
/// (`stories/<album>/file`), so the album has to be a real directory — dropping
/// files straight into `stories/` makes every file name look like an album.
#[test]
fn highlights_land_inside_an_album_directory() {
    let temp = tempfile::tempdir().expect("temp dir");
    let stogram_root = temp.path().join("4K Stogram");
    write_catalog(
        &stogram_root,
        "albumdir",
        "909",
        &[CatalogRow {
            instagram_id: "111_909",
            web_url: "https://www.instagram.com/p/AbCdEf/",
            media_url: "https://cdn.example/v/t51/1_2_3_n.jpg",
            title: None,
            created_time: 1_600_000_000,
            file: "albumdir\\highlights\\2020-09-13 09.06.40 111_909.jpg",
            is_video: 16,
            state: 4,
        }],
        &[],
    );

    let layout = storage::workspace_layout_from_roots(
        temp.path().join("localappdata"),
        temp.path().join("userprofile"),
    )
    .expect("layout");
    with_workspace_layout(layout, |connection, test_layout| {
        seed_account(connection, test_layout, "account-1");
        migrate(connection, test_layout, &options(&stogram_root, false));

        let source = load_sources(connection)?.remove(0);
        let rows = ledger_rows(connection, &source.id);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].2, "stories/legacy/2020-09-13 09.06.40 111_909.jpg",
            "a highlight must sit under an album directory, not loose in stories/"
        );
        let profile_root = test_layout.media_root.join("instagram").join("albumdir");
        assert!(profile_root
            .join("stories")
            .join("Legacy")
            .join("2020-09-13 09.06.40 111_909.jpg")
            .is_file());
        Ok(())
    })
    .expect("workspace");
}

/// The 4K Stogram library contains highlight rows flagged as downloaded whose
/// file is a thumbnail-sized placeholder. They must not reach the gallery.
#[test]
fn skips_thumbnail_sized_placeholders() {
    let temp = tempfile::tempdir().expect("temp dir");
    let stogram_root = temp.path().join("4K Stogram");
    write_catalog(
        &stogram_root,
        "degraded",
        "1010",
        &[
            CatalogRow {
                instagram_id: "111_1010",
                web_url: "https://www.instagram.com/p/AbCdEf/",
                media_url: "https://cdn.example/v/t51/1_2_3_n.jpg",
                title: None,
                created_time: 1_600_000_000,
                file: "degraded\\highlights\\2020-09-13 09.06.40 111_1010.jpg",
                is_video: 16,
                state: 4,
            },
            // Raw CDN name, written tiny below — the real library's signature
            // for a failed highlight download.
            CatalogRow {
                instagram_id: "222_1010",
                web_url: "https://www.instagram.com/p/GhIjKl/",
                media_url: "https://cdn.example/v/t51/4_5_6_n.jpg",
                title: None,
                created_time: 1_600_000_100,
                file: "degraded\\highlights\\460133787_3519631311515746_5487665385260616830_n.jpg",
                is_video: 16,
                state: 4,
            },
        ],
        &[],
    );
    // Shrink the CDN-named one to thumbnail size.
    fs::write(
        stogram_root.join("degraded/highlights/460133787_3519631311515746_5487665385260616830_n.jpg"),
        "tiny",
    )
    .expect("degraded file");

    let layout = storage::workspace_layout_from_roots(
        temp.path().join("localappdata"),
        temp.path().join("userprofile"),
    )
    .expect("layout");
    with_workspace_layout(layout, |connection, test_layout| {
        seed_account(connection, test_layout, "account-1");
        let report = migrate(connection, test_layout, &options(&stogram_root, false));

        assert_eq!(report.media_skipped_degraded, 1);
        assert_eq!(report.media_copied, 1, "only the real highlight is migrated");
        let source = load_sources(connection)?.remove(0);
        assert_eq!(ledger_rows(connection, &source.id).len(), 1);
        Ok(())
    })
    .expect("workspace");
}

/// A migrated highlight keeps its real album when the workspace already knows
/// one for that media; otherwise it goes to `Legacy`.
#[test]
fn files_migrated_highlights_under_legacy_or_their_matched_album() {
    let temp = tempfile::tempdir().expect("temp dir");
    let stogram_root = temp.path().join("4K Stogram");
    write_catalog(
        &stogram_root,
        "albums",
        "707",
        &[
            // Already downloaded by a real highlight sync (see the membership
            // seeded below, keyed by the CDN name inside media_url).
            CatalogRow {
                instagram_id: "111_707",
                web_url: "https://www.instagram.com/p/AbCdEf/",
                media_url: "https://cdn.example/v/t51/900_800_700_n.jpg",
                title: None,
                created_time: 1_600_000_000,
                file: "albums\\highlights\\2020-09-13 09.06.40 111_707.jpg",
                is_video: 16,
                state: 4,
            },
            // Unknown to the workspace → Legacy.
            CatalogRow {
                instagram_id: "222_707",
                web_url: "https://www.instagram.com/p/GhIjKl/",
                media_url: "https://cdn.example/v/t51/1_2_3_n.jpg",
                title: None,
                created_time: 1_600_000_100,
                file: "albums\\highlights\\2020-09-13 09.08.20 222_707.jpg",
                is_video: 16,
                state: 4,
            },
        ],
        &[],
    );

    let layout = storage::workspace_layout_from_roots(
        temp.path().join("localappdata"),
        temp.path().join("userprofile"),
    )
    .expect("layout");
    with_workspace_layout(layout, |connection, test_layout| {
        seed_account(connection, test_layout, "account-1");
        let special_path = test_layout.media_root.join("instagram").join("albums");
        seed_source(
            connection,
            test_layout,
            "source-1",
            "albums",
            "account-1",
            Some("707"),
            &special_path,
        );
        upsert_instagram_highlight_memberships(
            connection,
            "source-1",
            &[instagram_connector::InstagramHighlightMembership {
                provider_media_key: "900_800_700_n".to_string(),
                album: "Verão".to_string(),
            }],
            &now_timestamp(),
        )?;

        let report = migrate(connection, test_layout, &options(&stogram_root, false));
        assert_eq!(report.highlight_albums_matched, 1);

        let albums = load_instagram_highlight_membership(connection, "source-1");
        let album_of = |key: &str| {
            albums
                .get(key)
                .map(|set| set.iter().cloned().collect::<Vec<_>>())
                .unwrap_or_default()
        };
        assert_eq!(
            album_of("2020-09-13 09.06.40 111_707"),
            vec!["Verão".to_string()],
            "a highlight already known to an album keeps that album"
        );
        assert_eq!(
            album_of("2020-09-13 09.08.20 222_707"),
            vec!["Legacy".to_string()],
            "an untraceable highlight goes to Legacy"
        );
        Ok(())
    })
    .expect("workspace");
}

#[test]
fn dry_run_touches_neither_disk_nor_database() {
    let temp = tempfile::tempdir().expect("temp dir");
    let stogram_root = temp.path().join("4K Stogram");
    write_catalog(
        &stogram_root,
        "preview",
        "321",
        &[
            CatalogRow {
                instagram_id: "111_321",
                web_url: "https://www.instagram.com/p/AbCdEf/",
                media_url: "https://cdn.example/v/t51/1_2_3_n.jpg",
                title: None,
                created_time: 1_600_000_000,
                file: "preview\\2020-09-13 09.06.40 111_321.jpg",
                is_video: 0,
                state: 4,
            },
            // Catalogued but absent from disk.
            CatalogRow {
                instagram_id: "222_321",
                web_url: "https://www.instagram.com/p/GhIjKl/",
                media_url: "https://cdn.example/v/t51/4_5_6_n.jpg",
                title: None,
                created_time: 1_600_000_100,
                file: "preview\\2020-09-13 09.08.20 222_321.jpg",
                is_video: 0,
                state: 4,
            },
        ],
        &["preview\\2020-09-13 09.08.20 222_321.jpg"],
    );

    let layout = storage::workspace_layout_from_roots(
        temp.path().join("localappdata"),
        temp.path().join("userprofile"),
    )
    .expect("layout");
    with_workspace_layout(layout, |connection, test_layout| {
        seed_account(connection, test_layout, "account-1");
        let report = migrate(connection, test_layout, &options(&stogram_root, true));

        assert!(report.dry_run);
        assert_eq!(report.media_copied, 1);
        assert_eq!(report.media_missing_files, 1);
        assert!(
            load_sources(connection)?.is_empty(),
            "a dry run must not create sources"
        );
        assert!(
            !test_layout.media_root.join("instagram").join("preview").exists(),
            "a dry run must not create the profile root"
        );
        Ok(())
    })
    .expect("workspace");
}
