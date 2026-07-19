use super::*;
use crate::domain::models::{BatchSourceSyncOptionsPatch, ImportResolution};
use serde_json::json;
use tempfile::TempDir;

#[test]
fn provider_section_selections_preserve_explicit_false_values() {
    let mut twitter = default_twitter_source_sync_options();
    twitter.media_model = Some(false);
    twitter.profile_model = Some(false);
    twitter.search_model = Some(false);
    twitter.likes_model = Some(false);
    let twitter = twitter_model_selection(&normalize_twitter_source_sync_options(Some(twitter)));
    assert!(!twitter.media);
    assert!(!twitter.profile);
    assert!(!twitter.search);
    assert!(!twitter.likes);

    let mut tiktok = default_tiktok_source_sync_options();
    tiktok.get_timeline = Some(false);
    tiktok.get_stories_user = Some(false);
    tiktok.get_reposts = Some(false);
    tiktok.get_liked_videos = Some(false);
    tiktok.liked_videos_limit = Some(0);
    tiktok.liked_videos_incremental = Some(false);
    tiktok.liked_videos_known_page_threshold = Some(5);
    let tiktok = normalize_tiktok_source_sync_options(Some(tiktok));
    let sections = tiktok_section_selection(&tiktok);
    assert!(!sections.timeline);
    assert!(!sections.stories);
    assert!(!sections.reposts);
    assert_eq!(tiktok.get_liked_videos, Some(false));
    assert_eq!(tiktok.liked_videos_limit, Some(0));
    assert_eq!(tiktok.liked_videos_incremental, Some(false));
    assert_eq!(tiktok.liked_videos_known_page_threshold, Some(5));
}

#[test]
fn twitter_full_timeline_only_runs_in_dedicated_backfill_mode() {
    let options = default_twitter_source_sync_options();
    let normal = twitter_model_selection_for_run(&options, None);
    assert!(normal.media);
    assert!(!normal.profile);

    let backfill =
        twitter_model_selection_for_run(&options, Some(TWITTER_FULL_TIMELINE_BACKFILL_RUN_MODE));
    assert!(!backfill.media);
    assert!(backfill.profile);
    assert!(!backfill.search);
    assert!(!backfill.likes);
}

#[test]
fn twitter_phase_skips_sections_completed_before_an_account_hold() {
    let mut options = default_twitter_source_sync_options();
    options.likes_model = Some(true);
    let completed = HashSet::from(["media".to_string()]);

    let resumed = twitter_model_selection_for_phase(&options, None, &completed);
    assert!(!resumed.media);
    assert!(!resumed.profile);
    assert!(resumed.likes);
}

#[test]
fn twitter_resume_cursor_is_scoped_by_account_profile_and_section() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "twitter"),
        )?;
        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-1", "twitter", Some("account-1")),
        )?;
        upsert_provider_sync_resume_state(
            connection,
            "twitter",
            "source-1",
            "account-1",
            "normal",
            "media",
            "pending",
            Some("3_123/opaque"),
            "2026-07-11T00:00:00Z",
        )?;
        upsert_provider_sync_resume_state(
            connection,
            "twitter",
            "source-1",
            "account-1",
            "normal",
            "search",
            "completed",
            None,
            "2026-07-11T00:00:00Z",
        )?;
        let (cursors, completed) = load_provider_sync_resume_state(
            connection,
            "twitter",
            "source-1",
            "account-1",
            "normal",
        )?;
        assert_eq!(
            cursors.get("media").map(String::as_str),
            Some("3_123/opaque")
        );
        assert!(!cursors.contains_key("timeline"));
        assert!(completed.contains("search"));

        clear_provider_sync_resume_scope(connection, "twitter", "source-1", "account-1", "normal")?;
        let (cursors, completed) = load_provider_sync_resume_state(
            connection,
            "twitter",
            "source-1",
            "account-1",
            "normal",
        )?;
        assert!(cursors.is_empty());
        assert!(completed.is_empty());
        let hold_until = set_twitter_sync_hold(
            connection,
            "account-1",
            Duration::minutes(15),
            "2026-07-11T00:00:00Z",
        )?;
        assert_eq!(hold_until.to_rfc3339(), "2026-07-11T00:15:00+00:00");
        let stored_hold: String = connection
            .query_row(
                "SELECT hold_until FROM provider_sync_account_holds
                 WHERE provider = 'twitter' AND account_id = 'account-1'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        assert_eq!(stored_hold, "2026-07-11T00:15:00+00:00");
        clear_twitter_sync_hold(connection, "account-1")?;
        Ok(())
    })
    .expect("cursor lifecycle");
}

#[test]
fn twitter_sync_reports_partial_completion_as_warnings() {
    assert!(!twitter_sync_completed_with_warnings(false, &[]));
    assert!(twitter_sync_completed_with_warnings(true, &[]));
    assert!(twitter_sync_completed_with_warnings(
        false,
        &["media download failed".to_string()]
    ));
}

#[test]
fn twitter_sensitive_media_warning_requires_restricted_account_and_nsfw_profile() {
    assert!(twitter_sensitive_media_account_warning(true, true).is_none());
    assert!(twitter_sensitive_media_account_warning(true, false).is_none());
    assert!(twitter_sensitive_media_account_warning(false, false).is_none());
    let warning = twitter_sensitive_media_account_warning(false, true).expect("warning");
    assert!(warning.contains("unable to view sensitive media"));
    assert!(warning.contains("potentially sensitive"));
}

#[test]
fn source_sync_status_is_productive_includes_warning() {
    assert!(source_sync_status_is_productive("succeeded"));
    assert!(source_sync_status_is_productive("warning"));
    assert!(!source_sync_status_is_productive("failed"));
    assert!(!source_sync_status_is_productive("skipped"));
}

#[test]
fn existing_media_scan_ignores_zero_byte_download_placeholders() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("empty.mp4"), []).expect("placeholder");
    std::fs::write(temp.path().join("valid.mp4"), b"media").expect("media");

    let paths = load_existing_relative_media_paths(temp.path());

    assert!(!paths.contains("empty.mp4"));
    assert!(paths.contains("valid.mp4"));
}

#[test]
fn instagram_profile_sections_can_disable_every_account_enabled_section() {
    let mut source = sample_source_profile_model();
    source.provider = "instagram".to_string();
    source.sync_options = SourceSyncOptions {
        instagram: Some(InstagramSourceSyncOptions {
            timeline: false,
            reels: false,
            stories: false,
            stories_user: false,
            tagged: false,
            ..InstagramSourceSyncOptions::default()
        }),
        ..SourceSyncOptions::default()
    };
    let settings = HashMap::from([
        (
            "instagram.download.timeline".to_string(),
            "true".to_string(),
        ),
        ("instagram.download.reels".to_string(), "true".to_string()),
        ("instagram.download.stories".to_string(), "true".to_string()),
        (
            "instagram.download.storiesUser".to_string(),
            "true".to_string(),
        ),
        (
            "instagram.download.taggedPosts".to_string(),
            "true".to_string(),
        ),
    ]);

    let sections = build_instagram_section_selection(&source, &settings, None);

    assert!(!sections.timeline);
    assert!(!sections.reels);
    assert!(!sections.stories);
    assert!(!sections.stories_user);
    assert!(!sections.tagged);
}

#[test]
fn derive_post_metadata_tiktok_tokkit_video() {
    let d = derive_post_metadata(
        "tiktok",
        "gaaby.tls_1775147243_7624199329925958920.mp4",
        None,
    )
    .expect("derived");
    assert_eq!(d.post_id.as_deref(), Some("7624199329925958920"));
    assert_eq!(d.media_type, "video");
    assert!(d.captured_at.is_some());
}

#[test]
fn derive_post_metadata_tiktok_slideshow_groups_by_post() {
    let d = derive_post_metadata(
        "tiktok",
        "reeh_dmris_1703197051_7315175620856581381_index_0_2.jpeg",
        None,
    )
    .expect("derived");
    assert_eq!(d.post_id.as_deref(), Some("7315175620856581381"));
    assert_eq!(d.index, Some(0));
    assert_eq!(d.group_key, "7315175620856581381");
    assert_eq!(d.media_type, "image");
}

#[test]
fn build_post_url_tiktok_video_vs_photo_and_profile() {
    assert_eq!(
        build_post_url(
            "tiktok",
            "reeh_dmris",
            Some("7252779904704564486"),
            true,
            None
        )
        .as_deref(),
        Some("https://www.tiktok.com/@reeh_dmris/video/7252779904704564486")
    );
    assert_eq!(
        build_post_url(
            "tiktok",
            "@reeh_dmris",
            Some("7315175620856581381"),
            false,
            None
        )
        .as_deref(),
        Some("https://www.tiktok.com/@reeh_dmris/photo/7315175620856581381")
    );
    assert_eq!(
        source_target_url("tiktok", "reeh_dmris"),
        "https://www.tiktok.com/@reeh_dmris"
    );
}

#[test]
fn twitter_media_key_strips_date_gif_and_extension() {
    assert_eq!(
        twitter_media_key_from_file_name("2026-06-19 16.44.17 hlm3jgqxsaajvu-.jpg").as_deref(),
        Some("hlm3jgqxsaajvu-")
    );
    // GIF_ prefix (and casing) is normalized to match the XML File basename.
    assert_eq!(
        twitter_media_key_from_file_name("2025-11-10 15.11.32 GIF_G5aakG1WoAA2yHs.mp4").as_deref(),
        Some("g5aakg1woaa2yhs")
    );
    // Raw SCrawler name without a date prefix.
    assert_eq!(
        twitter_media_key_from_file_name("Ghmf7p4asAA3qXa.jpg").as_deref(),
        Some("ghmf7p4asaa3qxa")
    );
}

#[test]
fn build_post_url_twitter_uses_status_id() {
    assert_eq!(
        build_post_url(
            "twitter",
            "@someone",
            Some("1700000000000000001"),
            false,
            None
        )
        .as_deref(),
        Some("https://x.com/someone/status/1700000000000000001")
    );
    // Sem id não há link.
    assert_eq!(
        build_post_url("twitter", "someone", None, false, None),
        None
    );
}

#[test]
fn single_video_url_kind_routes_tiktok_photo_posts() {
    assert_eq!(
        single_video_url_kind(
            "tiktok",
            "https://www.tiktok.com/@rar1dade_/photo/7658099397019831573?lang=en"
        ),
        SingleVideoUrlKind::TikTokPhoto {
            handle: "rar1dade_".to_string(),
            post_id: "7658099397019831573".to_string(),
        }
    );
    assert_eq!(
        single_video_url_kind(
            "tiktok",
            "https://www.tiktok.com/@rar1dade_/video/7658099397019831573"
        ),
        SingleVideoUrlKind::Video
    );
}

#[test]
fn extract_tiktok_rehydration_json_reads_photo_post_images() {
    let html = r#"
        <script id="__UNIVERSAL_DATA_FOR_REHYDRATION__" type="application/json">
        {
          "__DEFAULT_SCOPE__": {
            "webapp.video-detail": {
              "itemInfo": {
                "itemStruct": {
                  "id": "7658099397019831573",
                  "desc": "photo post",
                  "createTime": "1783546503",
                  "author": { "uniqueId": "rar1dade_" },
                  "imagePost": {
                    "images": [
                      { "imageURL": { "urlList": ["https://example.test/one.jpeg"] } },
                      { "imageURL": { "urlList": ["https://example.test/two.jpeg"] } }
                    ]
                  }
                }
              }
            }
          }
        }
        </script>
    "#;

    let value = extract_tiktok_rehydration_json(html).expect("rehydration json");
    let item = tiktok_item_from_rehydration(&value).expect("item struct");
    let images = item
        .get("imagePost")
        .and_then(|image_post| image_post.get("images"))
        .and_then(serde_json::Value::as_array)
        .expect("images");

    assert_eq!(
        json_string_field(item, "desc").as_deref(),
        Some("photo post")
    );
    assert_eq!(
        item.get("createTime").and_then(parse_json_unix_timestamp),
        Some(1_783_546_503)
    );
    assert_eq!(images.len(), 2);
    assert_eq!(
        tiktok_photo_file_name("7658099397019831573", 0, 2),
        "7658099397019831573_001.jpg"
    );
}

#[test]
fn requested_tiktok_image_index_is_one_based_and_bounded() {
    assert_eq!(
        requested_tiktok_image_index(
            "https://www.tiktok.com/@rar1dade_/photo/7658099397019831573?image_index=2",
            8
        ),
        1
    );
    assert_eq!(
        requested_tiktok_image_index(
            "https://www.tiktok.com/@rar1dade_/photo/7658099397019831573?image_index=999",
            8
        ),
        0
    );
    assert_eq!(
        requested_tiktok_image_index(
            "https://www.tiktok.com/@rar1dade_/photo/7658099397019831573",
            8
        ),
        0
    );
}

#[test]
fn single_video_display_path_uses_requested_tiktok_photo_index() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let root = temp_dir.path();
    fs::write(root.join("7658099397019831573_001.jpg"), b"one").expect("first image");
    fs::write(root.join("7658099397019831573_002.jpg"), b"two").expect("second image");

    assert_eq!(
        single_video_display_relative_path(
            root,
            "7658099397019831573_001.jpg",
            "slideshow",
            "https://www.tiktok.com/@rar1dade_/photo/7658099397019831573?image_index=2",
            Some("7658099397019831573"),
        ),
        "7658099397019831573_002.jpg"
    );
}

#[test]
fn single_video_slideshow_paths_exclude_audio_but_audio_is_discovered() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let root = temp_dir.path();
    fs::write(root.join("7658099397019831573_001.jpg"), b"one").expect("first image");
    fs::write(root.join("7658099397019831573_002.jpg"), b"two").expect("second image");
    fs::write(root.join("7658099397019831573_audio.m4a"), b"audio").expect("audio");

    let image_names: Vec<String> = single_video_slideshow_paths(root, "7658099397019831573")
        .into_iter()
        .filter_map(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .map(str::to_string)
        })
        .collect();
    assert_eq!(
        image_names,
        vec![
            "7658099397019831573_001.jpg".to_string(),
            "7658099397019831573_002.jpg".to_string(),
        ]
    );

    assert_eq!(
        single_video_audio_relative_path(
            root,
            single_video_audio_path(root, Some("7658099397019831573")).as_deref(),
        )
        .as_deref(),
        Some("7658099397019831573_audio.m4a")
    );
}

#[test]
fn backfill_twitter_post_keys_fills_only_missing() {
    let conn = rusqlite::Connection::open_in_memory().expect("db");
    conn.execute_batch(
        "CREATE TABLE provider_sync_media_ledger (
                provider TEXT, source_id TEXT, account_id TEXT, source_handle TEXT,
                provider_media_key TEXT, media_type TEXT, media_section TEXT, relative_path TEXT,
                provider_post_key TEXT, captured_at INTEGER, first_seen_at TEXT, last_seen_at TEXT,
                PRIMARY KEY (provider, source_id, provider_media_key, media_type));
             INSERT INTO provider_sync_media_ledger VALUES
                ('twitter','s1','a','h','2068','image','media','2026 x.jpg', NULL, NULL, 't0','t0'),
                ('twitter','s1','a','h','9999','image','media','y.jpg', 'KEEP', NULL, 't0','t0');",
    )
    .expect("seed");

    let links = vec![
        twitter_connector::TwitterMediaPostLink {
            provider_media_key: "2068".into(),
            provider_post_key: "111".into(),
            media_section: "media".into(),
            captured_at_timestamp: Some(123),
        },
        twitter_connector::TwitterMediaPostLink {
            provider_media_key: "9999".into(),
            provider_post_key: "222".into(),
            media_section: "media".into(),
            captured_at_timestamp: Some(456),
        },
    ];
    backfill_provider_sync_media_ledger_post_keys(&conn, "twitter", "s1", &links, "t1")
        .expect("backfill");

    // Missing key gets filled (with captured_at); existing key is preserved.
    let filled: (Option<String>, Option<i64>) = conn
        .query_row(
            "SELECT provider_post_key, captured_at FROM provider_sync_media_ledger WHERE provider_media_key='2068'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(filled.0.as_deref(), Some("111"));
    assert_eq!(filled.1, Some(123));
    let kept: Option<String> = conn
        .query_row(
            "SELECT provider_post_key FROM provider_sync_media_ledger WHERE provider_media_key='9999'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kept.as_deref(), Some("KEEP"));
}

#[test]
fn twitter_media_ledger_accepts_multiple_media_keys_for_one_canonical_file() {
    let conn = rusqlite::Connection::open_in_memory().expect("db");
    conn.execute_batch(
        "CREATE TABLE provider_sync_media_ledger (
            provider TEXT, source_id TEXT, account_id TEXT, source_handle TEXT,
            provider_media_key TEXT, media_type TEXT, media_section TEXT, relative_path TEXT,
            provider_post_key TEXT, captured_at INTEGER, first_seen_at TEXT, last_seen_at TEXT,
            PRIMARY KEY (provider, source_id, provider_media_key, media_type));",
    )
    .expect("schema");
    let temp = tempfile::tempdir().expect("tempdir");
    let canonical = temp.path().join("canonical.jpg");
    fs::write(&canonical, b"same bytes").expect("canonical");
    let rows = vec![
        twitter_connector::DownloadedTwitterMedia {
            file_path: canonical.clone(),
            media_type: "image".to_string(),
            media_section: "media".to_string(),
            provider_media_key: "original-key".to_string(),
            provider_post_key: "post-1".to_string(),
            captured_at_timestamp: Some(1),
            final_file_name: "canonical.jpg".to_string(),
        },
        twitter_connector::DownloadedTwitterMedia {
            file_path: canonical,
            media_type: "image".to_string(),
            media_section: "media".to_string(),
            provider_media_key: "duplicate-key".to_string(),
            provider_post_key: "post-2".to_string(),
            captured_at_timestamp: Some(2),
            final_file_name: "canonical.jpg".to_string(),
        },
    ];

    upsert_provider_sync_media_ledger_entries(
        &conn,
        &ProviderSyncMediaScope {
            provider: "twitter",
            source_id: "source-1",
            account_id: "account-1",
            source_handle: "profile",
            profile_root: temp.path(),
            timestamp: "2026-07-11T00:00:00Z",
        },
        &rows,
    )
    .expect("aliases");

    let (count, paths): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), COUNT(DISTINCT relative_path)
             FROM provider_sync_media_ledger",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("counts");
    assert_eq!(count, 2);
    assert_eq!(paths, 1);
}

#[test]
fn build_post_url_instagram_uses_case_sensitive_shortcode() {
    // O shortcode mantém o casing original (case-sensitive).
    assert_eq!(
        build_post_url("instagram", "someone", None, false, Some("CyAbC-1_x")).as_deref(),
        Some("https://www.instagram.com/p/CyAbC-1_x/")
    );
    // Sem shortcode, sem link de post (cai para o perfil no front).
    assert_eq!(
        build_post_url("instagram", "someone", Some("123"), false, None),
        None
    );
}

#[test]
fn extract_post_tombstone_keys_per_provider() {
    let post = |url: Option<&str>, id: Option<&str>| MediaGalleryPost {
        post_id: id.map(str::to_string),
        post_url: url.map(str::to_string),
        captured_at: None,
        downloaded_at: None,
        author: None,
        media_type: "image".to_string(),
        section: "timeline".to_string(),
        albums: Vec::new(),
        poster_path: None,
        view_count: None,
        like_count: None,
        comment_count: None,
        share_count: None,
        stats_updated_at: None,
        files: Vec::new(),
    };
    // TikTok: usa o post id.
    assert_eq!(
        extract_post_tombstone_keys(
            "tiktok",
            &post(Some("https://www.tiktok.com/@h/video/123"), Some("123")),
        ),
        (Some("123".to_string()), None)
    );
    // Twitter: status id do post_url.
    assert_eq!(
        extract_post_tombstone_keys("twitter", &post(Some("https://x.com/h/status/999"), None)),
        (Some("999".to_string()), None)
    );
    // Instagram: shortcode (case-sensitive) do post_url.
    assert_eq!(
        extract_post_tombstone_keys(
            "instagram",
            &post(Some("https://www.instagram.com/p/CyAbC-1_x/"), None),
        ),
        (None, Some("CyAbC-1_x".to_string()))
    );
    // Sem URL: nada a tombstonar via post ledger.
    assert_eq!(
        extract_post_tombstone_keys("twitter", &post(None, None)),
        (None, None)
    );
}

#[test]
fn deleted_post_keys_suppress_future_syncs_for_every_provider() {
    let connection = Connection::open_in_memory().expect("in-memory database");
    connection
        .execute_batch(
            "CREATE TABLE source_profiles (id TEXT PRIMARY KEY);
             INSERT INTO source_profiles(id) VALUES ('source-1');",
        )
        .expect("source table");
    ensure_provider_deleted_media_table(&connection).expect("deleted media table");
    for (provider, post_key, post_code) in [
        ("twitter", Some("12345"), None),
        ("tiktok", Some("67890"), None),
        ("instagram", None, Some("AbC_12")),
    ] {
        connection
            .execute(
                "INSERT INTO provider_deleted_media (
                    provider, source_id, relative_path, media_section,
                    provider_post_key, provider_post_code, deleted_at
                 ) VALUES (?1, 'source-1', ?2, 'timeline', ?3, ?4, 'now')",
                params![provider, format!("{provider}.mp4"), post_key, post_code],
            )
            .expect("tombstone");
    }

    assert!(load_provider_deleted_post_keys(&connection, "twitter", "source-1")
        .expect("twitter keys")
        .contains("12345"));
    assert!(load_provider_deleted_post_keys(&connection, "tiktok", "source-1")
        .expect("tiktok keys")
        .contains("67890"));
    assert!(load_instagram_deleted_post_keys(&connection, "source-1")
        .expect("instagram keys")
        .contains("abc_12"));
}

#[test]
fn extract_instagram_post_code_preserves_casing_for_url() {
    let permalink = "https://www.instagram.com/p/CyAbC-1_x/";
    assert_eq!(
        extract_instagram_post_code_from_permalink_cased(permalink).as_deref(),
        Some("CyAbC-1_x")
    );
    // A variante normalizada (dedupe) continua lowercased.
    assert_eq!(
        extract_instagram_post_code_from_permalink(permalink).as_deref(),
        Some("cyabc-1_x")
    );
}

#[test]
fn derive_like_author_extracts_uploader_prefix() {
    // yt-dlp: `<uploader>_<videoId>.<ext>` — o autor é o prefixo antes do id.
    assert_eq!(
        derive_like_author(
            "gui3kkk_7624199329925958920.mp4",
            Some("7624199329925958920")
        )
        .as_deref(),
        Some("gui3kkk")
    );
    // Uploader com underscore é preservado por inteiro.
    assert_eq!(
        derive_like_author(
            "some_user_7624199329925958920.mp4",
            Some("7624199329925958920")
        )
        .as_deref(),
        Some("some_user")
    );
    // 4K Tokkit: `<uploader>_<unix>_<videoId>` — o token unix é descartado.
    assert_eq!(
        derive_like_author(
            "01.kessia_1773541356_7617302081459784980.mp4",
            Some("7617302081459784980"),
        )
        .as_deref(),
        Some("01.kessia")
    );
    // Slideshow do Tokkit: sufixo `_index_<i>_<n>` depois do id.
    assert_eq!(
        derive_like_author(
            "abcvickxyz_1768619691_7596163701175094536_index_0_5.jpeg",
            Some("7596163701175094536"),
        )
        .as_deref(),
        Some("abcvickxyz")
    );
    // Sem id derivado não há como separar o autor.
    assert_eq!(derive_like_author("whatever.mp4", None), None);
    // Só o id (sem autor no nome) não vira autor.
    assert_eq!(
        derive_like_author("7624199329925958920.mp4", Some("7624199329925958920")),
        None
    );
}

#[test]
fn video_thumbnail_lives_beside_media_and_preserves_dot_name() {
    let source = Path::new(r"S:\4K Tokkit\gui3kkk\Liked\.alice_123.mp4");
    assert_eq!(
        video_thumbnail_path(source),
        Some(PathBuf::from(
            r"S:\4K Tokkit\gui3kkk\Liked\.thumbs\.alice_123.mp4.jpg"
        ))
    );
}

#[test]
fn media_collection_ignores_generated_thumbs_directory() {
    let temp = tempfile::tempdir().expect("temp dir");
    let media = temp.path().join("Liked").join(".alice_123.mp4");
    let thumb = temp
        .path()
        .join("Liked")
        .join(".thumbs")
        .join(".alice_123.mp4.jpg");
    fs::create_dir_all(thumb.parent().expect("thumb parent")).expect("create dirs");
    fs::write(&media, b"video").expect("write media");
    fs::write(&thumb, b"jpeg").expect("write thumb");

    assert_eq!(
        collect_media_file_paths(temp.path()).expect("collect media"),
        vec![media]
    );
}

#[test]
fn media_collection_ignores_zero_byte_files() {
    let temp = tempfile::tempdir().expect("temp dir");
    let empty = temp.path().join("empty.mp4");
    let media = temp.path().join("valid.mp4");
    fs::write(&empty, []).expect("write placeholder");
    fs::write(&media, b"video").expect("write media");

    assert_eq!(
        collect_media_file_paths(temp.path()).expect("collect media"),
        vec![media]
    );
}

#[test]
fn empty_media_cleanup_removes_media_and_avatar_download_placeholders_only() {
    let temp = tempfile::tempdir().expect("temp dir");
    let empty_media = temp.path().join("empty.mp4");
    let empty_avatar_download = temp.path().join("ProfilePicture.jpg.download");
    let unrelated_empty = temp.path().join("notes.txt");
    let valid_media = temp.path().join("valid.mp4");
    fs::write(&empty_media, []).expect("write media placeholder");
    fs::write(&empty_avatar_download, []).expect("write avatar placeholder");
    fs::write(&unrelated_empty, []).expect("write unrelated placeholder");
    fs::write(&valid_media, b"video").expect("write valid media");

    assert_eq!(cleanup_empty_media_artifacts(temp.path()).unwrap(), 2);
    assert!(!empty_media.exists());
    assert!(!empty_avatar_download.exists());
    assert!(unrelated_empty.exists());
    assert!(valid_media.exists());
}

#[test]
fn parse_rfc3339_unix_parses_ledger_timestamps() {
    assert_eq!(parse_rfc3339_unix("1970-01-01T00:00:00Z"), Some(0));
    // Espaços em volta são tolerados; valor inválido retorna None.
    assert_eq!(parse_rfc3339_unix("  1970-01-01T00:00:10Z  "), Some(10));
    assert_eq!(parse_rfc3339_unix("not-a-date"), None);
}

#[test]
fn is_profile_image_file_excludes_avatar_and_profile_picture() {
    assert!(is_profile_image_file(
        "reeh_dmris_0_7318182511312371717_avatar.jpeg"
    ));
    assert!(is_profile_image_file("ProfilePicture.jpg"));
    assert!(!is_profile_image_file(
        "reeh_dmris_1703197051_7315175620856581381_index_0_2.jpeg"
    ));
}

#[test]
fn reconcile_tiktok_provider_ledgers_from_disk_seeds_existing_files() {
    let (_temp_dir, layout) = create_test_layout();

    let (recovered, post_count, media_count, thumbnail_count, liked_timeline_count, liked_likes_count) =
        with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "tiktok"),
            )?;
            let mut source = sample_source("source-1", "tiktok", Some("account-1"));
            source.handle = "@archangelszxx".to_string();
            upsert_source_profile_with_connection(connection, test_layout, source)?;

            let profile_root = test_layout.media_root.join("tiktok").join("@archangelszxx");
            fs::create_dir_all(profile_root.join(".thumbs")).map_err(|error| error.to_string())?;
            fs::create_dir_all(profile_root.join("Settings")).map_err(|error| error.to_string())?;
            fs::write(
                profile_root.join("2026-07-07 12.04.09 7659802061789302034_001.jpg"),
                b"image",
            )
            .map_err(|error| error.to_string())?;
            fs::write(
                profile_root.join("archangelszxx_1775147243_7624199329925958920.mp4"),
                b"video",
            )
            .map_err(|error| error.to_string())?;
            fs::write(profile_root.join("ProfilePicture.jpg"), b"avatar")
                .map_err(|error| error.to_string())?;
            fs::write(
                profile_root
                    .join(".thumbs")
                    .join("2026-07-07 12.04.09 7659802061789302034_001.jpg"),
                b"thumb",
            )
            .map_err(|error| error.to_string())?;
            fs::write(
                profile_root
                    .join("Settings")
                    .join("7659802061789302034_001.jpg"),
                b"settings",
            )
            .map_err(|error| error.to_string())?;
            // Liked videos are owned by the likes runtime (section "likes"); the
            // timeline reconcile must skip them so it never shadows that section
            // with a competing "timeline" row on the same relative path.
            fs::create_dir_all(profile_root.join("Liked")).map_err(|error| error.to_string())?;
            fs::write(
                profile_root
                    .join("Liked")
                    .join("eujhulys_1743950164_7490208900055190789.mp4"),
                b"liked video",
            )
            .map_err(|error| error.to_string())?;
            // Simulate a database left behind by an older build: the same liked
            // file has both the likes runtime's legitimate "likes" row and the
            // bogus "timeline" row a previous reconcile wrote (keyed by file name,
            // lowercased path). The reconcile must purge the latter and keep the
            // former.
            connection
                .execute_batch(
                    "INSERT INTO provider_sync_media_ledger
                        (provider, source_id, account_id, source_handle,
                         provider_media_key, media_type, media_section, relative_path,
                         first_seen_at, last_seen_at)
                     VALUES
                        ('tiktok','source-1','account-1','@archangelszxx',
                         'liked_7490208900055190789','video','likes',
                         'Liked/eujhulys_1743950164_7490208900055190789.mp4','t0','t0'),
                        ('tiktok','source-1','account-1','@archangelszxx',
                         'eujhulys_1743950164_7490208900055190789.mp4','video','timeline',
                         'liked/eujhulys_1743950164_7490208900055190789.mp4','t0','t0');",
                )
                .map_err(|error| error.to_string())?;

            let recovered = reconcile_tiktok_provider_ledgers_from_disk(
                connection,
                &profile_root,
                "account-1",
                "source-1",
                "@archangelszxx",
                "2026-07-08T00:00:00Z",
            )?;
            let post_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM provider_sync_post_ledger
                     WHERE provider = 'tiktok' AND source_id = 'source-1'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            let media_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM provider_sync_media_ledger
                     WHERE provider = 'tiktok' AND source_id = 'source-1'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            let thumbnail_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM provider_sync_media_ledger
                     WHERE provider = 'tiktok' AND source_id = 'source-1'
                       AND relative_path LIKE '%.thumbs%'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            // Bogus reconciled row (lowercased `liked/...`, section "timeline").
            let liked_timeline_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM provider_sync_media_ledger
                     WHERE provider = 'tiktok' AND source_id = 'source-1'
                       AND media_section = 'timeline'
                       AND relative_path LIKE 'liked/%'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            // Legitimate likes runtime row (keyed by `liked_<id>`, section "likes").
            let liked_likes_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM provider_sync_media_ledger
                     WHERE provider = 'tiktok' AND source_id = 'source-1'
                       AND media_section = 'likes'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            Ok((
                recovered,
                post_count,
                media_count,
                thumbnail_count,
                liked_timeline_count,
                liked_likes_count,
            ))
        })
        .expect("existing TikTok files should seed provider ledgers");

    assert_eq!(recovered, 2);
    assert_eq!(post_count, 2);
    // Two reconciled timeline files plus the preserved likes runtime row.
    assert_eq!(media_count, 3);
    assert_eq!(thumbnail_count, 0);
    // The bogus liked "timeline" row must be purged, and the real likes row kept.
    assert_eq!(liked_timeline_count, 0);
    assert_eq!(liked_likes_count, 1);
}

fn create_test_layout() -> (TempDir, StorageLayout) {
    // Layout montado na mão para o suite ser HERMÉTICO: evita depender de
    // ensure_workspace_layout()/preferred_media_root, que resolvem paths reais
    // do usuário (LOCALAPPDATA/USERPROFILE) em vez do diretório temporário.
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let root = temp_dir.path().join("localappdata").join("NinjaCrawler");
    let data_dir = root.join("data");
    let logs_dir = root.join("logs");
    let cache_root = root.join("cache");
    let connectors_root = data_dir.join("connectors");
    let media_root = temp_dir
        .path()
        .join("userprofile")
        .join("Pictures")
        .join("NinjaCrawler");
    let db_path = data_dir.join("ninjacrawler.db");
    for dir in [
        &data_dir,
        &logs_dir,
        &cache_root,
        &connectors_root,
        &media_root,
    ] {
        fs::create_dir_all(dir).expect("test layout dir");
    }
    let layout = StorageLayout {
        root,
        data_dir,
        logs_dir,
        db_path,
        media_root,
        cache_root,
        connectors_root,
    };
    (temp_dir, layout)
}

fn sample_account(id: &str, provider: &str) -> ProviderAccountUpsert {
    ProviderAccountUpsert {
        id: Some(id.to_string()),
        provider: provider.to_string(),
        display_name: format!("{provider}-account"),
        auth_mode: "imported_session".to_string(),
        auth_state: "ready".to_string(),
        capabilities: vec!["posts".to_string()],
        last_validated_at: Some("2026-03-10T00:00:00Z".to_string()),
    }
}

fn sample_source(id: &str, provider: &str, account_id: Option<&str>) -> SourceProfileUpsert {
    SourceProfileUpsert {
        id: Some(id.to_string()),
        provider: provider.to_string(),
        source_kind: "profile".to_string(),
        handle: format!("@{id}"),
        display_name: id.to_string(),
        account_id: account_id.map(|value| value.to_string()),
        group_id: None,
        labels: vec!["priority".to_string()],
        ready_for_download: true,
        sync_options: default_source_sync_options(provider),
        remote_state: None,
        is_subscription: None,
    }
}

fn sample_source_profile_model() -> SourceProfile {
    SourceProfile {
        id: "source-1".to_string(),
        provider: "instagram".to_string(),
        source_kind: "profile".to_string(),
        handle: "@source-1".to_string(),
        display_name: "source-1".to_string(),
        account_id: Some("account-1".to_string()),
        group_id: None,
        labels: vec![],
        ready_for_download: true,
        sync_options: default_source_sync_options("instagram"),
        profile_image_path: None,
        profile_image_custom: false,
        remote_state: "exists".to_string(),
        is_subscription: false,
        last_synced_at: None,
        sync_problem_code: None,
        sync_problem_message: None,
        sync_problem_at: None,
        created_at: Some("2026-03-10T00:00:00Z".to_string()),
        importer_id: None,
        imported_at: None,
    }
}

fn sample_instagram_manifest_summary() -> instagram_connector::InstagramManifestSummary {
    instagram_connector::InstagramManifestSummary {
        section_count: 4,
        discovered_item_count: 4,
        normalized_post_count: 0,
        discovered_asset_count: 0,
        queued_asset_count: 0,
        skipped_existing_post_count: 4,
        skipped_duplicate_post_count: 0,
        skipped_unavailable_post_count: 0,
        skipped_existing_asset_count: 0,
        skipped_duplicate_asset_count: 0,
        downloaded_asset_count: 0,
        profile_user_id: None,
        sections: vec![],
    }
}

fn create_legacy_instagram_profile_root(
    profile_root: &Path,
    account_name: &str,
    user_name: &str,
    description: Option<&str>,
) -> Result<PathBuf, String> {
    create_legacy_instagram_profile_root_full(
        profile_root,
        account_name,
        user_name,
        None,
        None,
        description,
    )
}

fn create_legacy_instagram_profile_root_full(
    profile_root: &Path,
    account_name: &str,
    user_name: &str,
    true_name: Option<&str>,
    user_id: Option<&str>,
    description: Option<&str>,
) -> Result<PathBuf, String> {
    let settings_dir = profile_root.join("Settings");
    fs::create_dir_all(&settings_dir).map_err(|error| error.to_string())?;

    let true_name_value = true_name.unwrap_or(user_name);
    let description_tag = description
        .map(|value| format!("\n  <Description>{value}</Description>"))
        .unwrap_or_default();
    let user_id_tag = user_id
        .map(|value| format!("\n  <UserID>{value}</UserID>"))
        .unwrap_or_default();
    let user_xml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
             <UserData>\n\
               <AccountName>{account_name}</AccountName>{user_id_tag}\n\
               <UserName>{user_name}</UserName>\n\
               <TrueName>{true_name_value}</TrueName>\n\
               <FriendlyName>{user_name}</FriendlyName>\n\
               <UserSiteName>{user_name}</UserSiteName>{description_tag}\n\
               <ReadyForDownload>true</ReadyForDownload>\n\
               <GetTimeline>true</GetTimeline>\n\
               <GetReels>false</GetReels>\n\
               <GetStories>false</GetStories>\n\
               <GetStoriesUser>false</GetStoriesUser>\n\
               <GetTaggedData>false</GetTaggedData>\n\
             </UserData>\n"
    );

    let user_xml_path = settings_dir.join("User_Instagram.xml");
    fs::write(&user_xml_path, user_xml).map_err(|error| error.to_string())?;
    fs::write(profile_root.join("first.jpg"), b"image").map_err(|error| error.to_string())?;
    Ok(user_xml_path)
}

fn create_legacy_instagram_data_xml(
    profile_root: &Path,
    file_name: &str,
    post_id: &str,
    special_folder: Option<&str>,
    media_url: &str,
    post_permalink: &str,
) -> Result<PathBuf, String> {
    let settings_dir = profile_root.join("Settings");
    fs::create_dir_all(&settings_dir).map_err(|error| error.to_string())?;

    let data_xml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"yes\"?>\n\
             <Data>\n\
               <MediaData Attempts=\"0\" Date=\"2025-02-10 04:31:48\" File=\"{file_name}\" ID=\"{post_id}\" SpecialFolder=\"{}\" State=\"2\" Type=\"{}\" URL=\"{media_url}\">{post_permalink}</MediaData>\n\
             </Data>\n",
        special_folder.unwrap_or_default(),
        if file_name.to_ascii_lowercase().ends_with(".mp4") { "2" } else { "1" },
    );

    let file_stem = Path::new(file_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("legacy");
    let data_xml_path = settings_dir.join(format!("User_Instagram_{file_stem}_Data.xml"));
    fs::write(&data_xml_path, data_xml).map_err(|error| error.to_string())?;
    Ok(data_xml_path)
}

#[test]
fn implicit_instagram_imported_cutoff_uses_materialized_metadata_and_force_mode_bypasses_it() {
    let mut source = sample_source_profile_model();
    source.importer_id = Some("instagram.scrawler".to_string());
    source.imported_at = Some("2026-03-20T12:00:00Z".to_string());

    // 2026-03-20T12:00:00Z em unix seconds.
    let cutoff = implicit_instagram_imported_cutoff_timestamp(&source, None);
    assert_eq!(cutoff, Some(1_774_008_000));

    let bypassed =
        implicit_instagram_imported_cutoff_timestamp(&source, Some("force_imported_backfill"));
    assert_eq!(bypassed, None);
}

fn load_source_profile_by_id(
    connection: &Connection,
    source_id: &str,
) -> Result<SourceProfile, String> {
    connection
        .query_row(
        "SELECT provider, source_kind, handle, display_name, account_id, labels_json, ready_for_download, sync_options_json, profile_image_path, profile_image_custom, remote_state, is_subscription, last_synced_at, sync_problem_code, sync_problem_message, sync_problem_at, created_at, group_id, importer_id, imported_at
             FROM source_profiles
             WHERE id = ?1
               AND deleted_at IS NULL
             LIMIT 1",
            params![source_id],
            |row| {
                let provider = row.get::<_, String>(0)?;
                let labels_json = row.get::<_, String>(5)?;
                let sync_options_json = row.get::<_, String>(7)?;
                Ok(SourceProfile {
                    id: source_id.to_string(),
                    provider: provider.clone(),
                    source_kind: row.get(1)?,
                    handle: row.get(2)?,
                    display_name: row.get(3)?,
                    account_id: row.get(4)?,
                    group_id: row.get(17)?,
                    labels: serde_json::from_str(&labels_json).unwrap_or_default(),
                ready_for_download: row.get::<_, i64>(6).unwrap_or(0) != 0,
                sync_options: deserialize_source_sync_options(&provider, &sync_options_json),
                profile_image_path: row.get(8)?,
                profile_image_custom: row.get::<_, i64>(9).unwrap_or(0) != 0,
                remote_state: row.get::<_, String>(10).unwrap_or_else(|_| "exists".to_string()),
                is_subscription: row.get::<_, i64>(11).unwrap_or(0) != 0,
                last_synced_at: row.get(12).ok(),
                sync_problem_code: row.get(13).ok(),
                sync_problem_message: row.get(14).ok(),
                sync_problem_at: row.get(15).ok(),
                created_at: row.get(16).ok(),
                importer_id: row.get(18).ok(),
                imported_at: row.get(19).ok(),
            })
        },
    )
        .map_err(|error| error.to_string())
}

#[test]
fn record_external_import_ledger_updates_materialized_source_import_metadata() {
    let (_temp_dir, layout) = create_test_layout();
    let connection = database::open_connection(&layout.db_path).expect("connection");

    upsert_provider_account_with_connection(
        &connection,
        &layout,
        sample_account("account-1", "instagram"),
    )
    .expect("account should upsert");
    upsert_source_profile_with_connection(
        &connection,
        &layout,
        sample_source("source-1", "instagram", Some("account-1")),
    )
    .expect("source should upsert");

    record_external_import_ledger(
        &connection,
        ExternalImportLedgerRecord {
            importer_id: INSTAGRAM_SCRAWLER_IMPORTER_ID,
            profile_root: Path::new("D:/legacy/source-a"),
            provider: "instagram",
            handle: "@source-1",
            source_id: "source-1",
            account_id: "account-1",
            timestamp: "2026-03-20T12:00:00Z",
        },
    )
    .expect("first import metadata should persist");
    record_external_import_ledger(
        &connection,
        ExternalImportLedgerRecord {
            importer_id: INSTAGRAM_SCRAWLER_IMPORTER_ID,
            profile_root: Path::new("D:/legacy/source-b"),
            provider: "instagram",
            handle: "@source-1",
            source_id: "source-1",
            account_id: "account-1",
            timestamp: "2026-03-22T15:30:00Z",
        },
    )
    .expect("latest import metadata should persist");

    let source = load_source_profile_by_id(&connection, "source-1").expect("source should load");
    assert_eq!(
        source.importer_id.as_deref(),
        Some(INSTAGRAM_SCRAWLER_IMPORTER_ID)
    );
    assert_eq!(source.imported_at.as_deref(), Some("2026-03-22T15:30:00Z"));
}

#[test]
fn special_path_is_cleared_by_sent_empty_value_and_preserved_when_absent() {
    let (_temp_dir, layout) = create_test_layout();
    let connection = database::open_connection(&layout.db_path).expect("connection");

    upsert_provider_account_with_connection(
        &connection,
        &layout,
        sample_account("account-tt", "tiktok"),
    )
    .expect("account should upsert");

    // Perfil com special_path definido (como um import legado deixaria).
    let mut with_special = sample_source("source-tt", "tiktok", Some("account-tt"));
    if let Some(tiktok) = with_special.sync_options.tiktok.as_mut() {
        tiktok.special_path = Some("D:/legacy/tiktok/source-tt".to_string());
    }
    upsert_source_profile_with_connection(&connection, &layout, with_special)
        .expect("source should upsert");

    // Upsert SEM o campo (fluxo interno): o valor persistido é preservado.
    let mut absent = sample_source("source-tt", "tiktok", Some("account-tt"));
    if let Some(tiktok) = absent.sync_options.tiktok.as_mut() {
        tiktok.special_path = None;
    }
    upsert_source_profile_with_connection(&connection, &layout, absent)
        .expect("absent special path should upsert");
    let source = load_source_profile_by_id(&connection, "source-tt").expect("source should load");
    assert_eq!(
        source
            .sync_options
            .tiktok
            .as_ref()
            .and_then(|tiktok| tiktok.special_path.as_deref()),
        Some("D:/legacy/tiktok/source-tt"),
        "absent special path must preserve the persisted override"
    );

    // Upsert COM o campo vazio (edição na UI): o override é limpo.
    let mut cleared = sample_source("source-tt", "tiktok", Some("account-tt"));
    if let Some(tiktok) = cleared.sync_options.tiktok.as_mut() {
        tiktok.special_path = Some(String::new());
    }
    upsert_source_profile_with_connection(&connection, &layout, cleared)
        .expect("cleared special path should upsert");
    let source = load_source_profile_by_id(&connection, "source-tt").expect("source should load");
    let cleared_value = source
        .sync_options
        .tiktok
        .as_ref()
        .and_then(|tiktok| tiktok.special_path.as_deref())
        .unwrap_or("");
    assert_eq!(
        cleared_value, "",
        "a sent-but-empty special path must clear the persisted override"
    );
}

#[test]
fn manual_handle_change_is_supported_for_every_provider() {
    let (_temp_dir, layout) = create_test_layout();
    let connection = database::open_connection(&layout.db_path).expect("connection");

    let runs = |conn: &Connection, source_id: &str| -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM source_sync_runs WHERE source_id = ?1 AND trigger = 'manual_handle_edit'",
            params![source_id],
            |row| row.get(0),
        )
        .expect("count")
    };

    for provider in ["instagram", "tiktok", "twitter"] {
        let account_id = format!("account-{provider}");
        let source_id = format!("source-{provider}");
        upsert_provider_account_with_connection(
            &connection,
            &layout,
            sample_account(&account_id, provider),
        )
        .expect("account should upsert");
        upsert_source_profile_with_connection(
            &connection,
            &layout,
            sample_source(&source_id, provider, Some(&account_id)),
        )
        .expect("source should upsert");

        assert_eq!(runs(&connection, &source_id), 0, "{provider}");

        let mut renamed = sample_source(&source_id, provider, Some(&account_id));
        renamed.handle = format!("@renamed-{provider}");
        upsert_source_profile_with_connection(&connection, &layout, renamed.clone())
            .expect("handle change should upsert");
        assert_eq!(runs(&connection, &source_id), 1, "{provider}");

        upsert_source_profile_with_connection(&connection, &layout, renamed)
            .expect("no-op resave should upsert");
        assert_eq!(runs(&connection, &source_id), 1, "{provider}");
    }

    let instagram_source =
        load_source_profile_by_id(&connection, "source-instagram").expect("Instagram source");
    let previous_handles = instagram_source
        .sync_options
        .instagram
        .and_then(|options| options.previous_handles)
        .unwrap_or_default();
    assert!(
        previous_handles
            .iter()
            .any(|handle| handle == "source-instagram"),
        "manual Instagram rename should preserve the previous handle"
    );
}

#[test]
fn load_source_sync_runs_caps_history_per_source() {
    let (_temp_dir, layout) = create_test_layout();
    let connection = database::open_connection(&layout.db_path).expect("connection");
    upsert_provider_account_with_connection(
        &connection,
        &layout,
        sample_account("account-1", "instagram"),
    )
    .expect("account should upsert");
    upsert_source_profile_with_connection(
        &connection,
        &layout,
        sample_source("source-1", "instagram", Some("account-1")),
    )
    .expect("source should upsert");
    upsert_source_profile_with_connection(
        &connection,
        &layout,
        sample_source("source-2", "instagram", Some("account-1")),
    )
    .expect("source should upsert");

    let insert_run = |run_id: &str, source_id: &str, minute: u32| {
        let timestamp = format!("2026-07-01T10:{minute:02}:00Z");
        connection
            .execute(
                "INSERT INTO source_sync_runs (
                    id, source_id, account_id, provider, tool, trigger, status,
                    summary, command_preview, manifest_summary_json,
                    degraded_capabilities_json, started_at, finished_at, created_at
                 ) VALUES (
                    ?1, ?2, 'account-1', 'instagram', 'internal.instagram',
                    'manual', 'succeeded', 'ok', 'test', NULL, '[]', ?3, ?3, ?3
                 )",
                params![run_id, source_id, timestamp],
            )
            .expect("run should insert");
    };
    for minute in 0..25 {
        insert_run(&format!("run-a-{minute:02}"), "source-1", minute);
    }
    for minute in 0..3 {
        insert_run(&format!("run-b-{minute:02}"), "source-2", minute);
    }

    let runs = load_source_sync_runs(&connection).expect("runs should load");
    let source_1_runs: Vec<_> = runs
        .iter()
        .filter(|run| run.source_id == "source-1")
        .collect();
    let source_2_runs: Vec<_> = runs
        .iter()
        .filter(|run| run.source_id == "source-2")
        .collect();
    assert_eq!(
        source_1_runs.len(),
        SYNC_RUN_HISTORY_CAP_PER_ENTITY as usize,
        "history above the cap should be trimmed per source"
    );
    assert_eq!(
        source_2_runs.len(),
        3,
        "sources below the cap keep their full history"
    );
    assert!(
        source_1_runs
            .iter()
            .all(|run| run.finished_at.as_str() >= "2026-07-01T10:05:00Z"),
        "the most recent runs are the ones kept"
    );
}

#[test]
fn legacy_instagram_manifest_keeps_identity_hint_recoverable() {
    let (_temp_dir, layout) = create_test_layout();
    let connection = database::open_connection(&layout.db_path).expect("connection");
    upsert_provider_account_with_connection(
        &connection,
        &layout,
        sample_account("account-1", "instagram"),
    )
    .expect("account should upsert");
    upsert_source_profile_with_connection(
        &connection,
        &layout,
        sample_source("source-1", "instagram", Some("account-1")),
    )
    .expect("source should upsert");

    let legacy_summary = json!({
        "profileUserId": "80735443629",
        "sectionCount": 1,
        "discoveredItemCount": 1,
        "normalizedPostCount": 1,
        "discoveredAssetCount": 1,
        "queuedAssetCount": 1,
        "skippedExistingPostCount": 0,
        "skippedDuplicatePostCount": 0,
        "skippedUnavailablePostCount": 0,
        "skippedExistingAssetCount": 0,
        "skippedDuplicateAssetCount": 0,
        "downloadedAssetCount": 1,
        "sections": [{
            "section": "timeline",
            "label": "Timeline",
            "itemCount": 1,
            "normalizedPostCount": 1,
            "discoveredAssetCount": 1,
            "queuedAssetCount": 1,
            "skippedExistingPostCount": 0,
            "skippedDuplicatePostCount": 0,
            "skippedUnavailablePostCount": 0,
            "skippedExistingAssetCount": 0,
            "skippedDuplicateAssetCount": 0
        }]
    })
    .to_string();
    connection
        .execute(
            "INSERT INTO source_sync_runs (
                    id, source_id, account_id, provider, tool, trigger, status,
                    summary, command_preview, manifest_summary_json,
                    degraded_capabilities_json, started_at, finished_at, created_at
                 ) VALUES (
                    'run-legacy', 'source-1', 'account-1', 'instagram',
                    'internal.instagram', 'manual', 'succeeded', 'ok', 'test',
                    ?1, '[]', '2026-06-21T13:40:44Z',
                    '2026-06-21T13:41:01Z', '2026-06-21T13:41:01Z'
                 )",
            params![legacy_summary],
        )
        .expect("legacy run should insert");
    set_source_sync_problem(
        &connection,
        "source-1",
        "instagram_username_unresolvable",
        "legacy resolver could not recover the renamed profile",
        "2026-07-01T17:52:10Z",
        true,
    )
    .expect("legacy problem marker");

    let parsed =
        serde_json::from_str::<instagram_connector::InstagramManifestSummary>(&legacy_summary)
            .expect("new summary fields must default when reading legacy history");
    assert_eq!(parsed.sections[0].skipped_out_of_range_item_count, 0);
    assert_eq!(
        load_latest_instagram_profile_user_id_hint(&connection, "source-1")
            .expect("history lookup"),
        Some("80735443629".to_string())
    );

    connection
        .execute_batch(include_str!(
            "../../../migrations/0031_instagram_identity_hint_backfill.sql"
        ))
        .expect("identity backfill migration should be idempotent");
    let recovered_source = connection
        .query_row(
            "SELECT
                    json_extract(sync_options_json, '$.instagram.userIdHint'),
                    ready_for_download,
                    sync_problem_code
                 FROM source_profiles WHERE id = 'source-1'",
            [],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .expect("recovered source");
    assert_eq!(recovered_source.0.as_deref(), Some("80735443629"));
    assert_eq!(recovered_source.1, 1);
    assert_eq!(recovered_source.2, None);

    connection
        .execute(
            "UPDATE source_profiles
                 SET sync_options_json = json_set(
                        sync_options_json,
                        '$.instagram.userIdHint',
                        '59617797093'
                     ),
                     ready_for_download = 0,
                     sync_problem_code = 'instagram_username_unresolvable',
                     sync_problem_message = 'blocked by stale imported hint',
                     sync_problem_at = '2026-07-03T07:50:13Z'
                 WHERE id = 'source-1'",
            [],
        )
        .expect("stale imported identity should be simulated");
    connection
        .execute_batch(include_str!(
            "../../../migrations/0033_instagram_identity_hint_reconcile.sql"
        ))
        .expect("identity reconciliation migration should run");
    let reconciled_source = connection
        .query_row(
            "SELECT
                    json_extract(sync_options_json, '$.instagram.userIdHint'),
                    ready_for_download,
                    sync_problem_code
                 FROM source_profiles WHERE id = 'source-1'",
            [],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .expect("reconciled source");
    assert_eq!(reconciled_source.0.as_deref(), Some("80735443629"));
    assert_eq!(reconciled_source.1, 1);
    assert_eq!(reconciled_source.2, None);
}

#[test]
fn instagram_identity_hint_is_persisted_once_and_cannot_drift() {
    let (_temp_dir, layout) = create_test_layout();
    let connection = database::open_connection(&layout.db_path).expect("connection");
    upsert_provider_account_with_connection(
        &connection,
        &layout,
        sample_account("account-1", "instagram"),
    )
    .expect("account should upsert");
    upsert_source_profile_with_connection(
        &connection,
        &layout,
        sample_source("source-1", "instagram", Some("account-1")),
    )
    .expect("source should upsert");

    persist_instagram_user_id_hint(
        &connection,
        "source-1",
        "80735443629",
        "2026-07-02T00:00:00Z",
    )
    .expect("identity should persist");
    persist_instagram_user_id_hint(
        &connection,
        "source-1",
        "80735443629",
        "2026-07-02T00:01:00Z",
    )
    .expect("same identity should be idempotent");

    let mismatch =
        persist_instagram_user_id_hint(&connection, "source-1", "999999", "2026-07-02T00:02:00Z")
            .expect_err("identity drift must be rejected");
    assert!(mismatch.contains("identity mismatch"));

    let source = load_source_profile_by_id(&connection, "source-1").expect("source");
    assert_eq!(
        source
            .sync_options
            .instagram
            .and_then(|options| options.user_id_hint)
            .as_deref(),
        Some("80735443629")
    );
}

#[test]
fn instagram_identity_hint_prefers_confirmed_history_over_imported_hint() {
    assert_eq!(
        preferred_instagram_user_id_hint(Some("59617797093"), Some("74818949106")).as_deref(),
        Some("74818949106")
    );
    assert_eq!(
        preferred_instagram_user_id_hint(Some("59617797093"), None).as_deref(),
        Some("59617797093")
    );
}

#[test]
fn instagram_identity_hint_can_repair_imported_mismatch_confirmed_by_history() {
    let (_temp_dir, layout) = create_test_layout();
    let connection = database::open_connection(&layout.db_path).expect("connection");
    upsert_provider_account_with_connection(
        &connection,
        &layout,
        sample_account("account-1", "instagram"),
    )
    .expect("account should upsert");
    let mut source = sample_source("source-1", "instagram", Some("account-1"));
    source
        .sync_options
        .instagram
        .get_or_insert_with(default_instagram_source_sync_options)
        .user_id_hint = Some("59617797093".to_string());
    upsert_source_profile_with_connection(&connection, &layout, source)
        .expect("source should upsert");
    connection
        .execute(
            "INSERT INTO source_sync_runs (
                    id, source_id, account_id, provider, tool, trigger, status,
                    summary, command_preview, manifest_summary_json,
                    degraded_capabilities_json, started_at, finished_at, created_at
                 ) VALUES (
                    'run-confirmed', 'source-1', 'account-1', 'instagram',
                    'internal.instagram', 'manual', 'succeeded', 'ok', 'test',
                    '{\"profileUserId\":\"74818949106\"}', '[]',
                    '2026-06-30T04:04:26Z', '2026-06-30T04:04:39Z',
                    '2026-06-30T04:04:39Z'
                 )",
            [],
        )
        .expect("confirmed history should insert");

    persist_instagram_user_id_hint(
        &connection,
        "source-1",
        "74818949106",
        "2026-07-03T08:00:00Z",
    )
    .expect("confirmed history should repair the imported hint");

    let repaired = load_source_profile_by_id(&connection, "source-1")
        .expect("source")
        .sync_options
        .instagram
        .and_then(|options| options.user_id_hint);
    assert_eq!(repaired.as_deref(), Some("74818949106"));
}

fn sample_instagram_cookies() -> Vec<ProviderAccountCookie> {
    vec![
        ProviderAccountCookie {
            domain: ".instagram.com".to_string(),
            name: "sessionid".to_string(),
            value: "abc123".to_string(),
            path: "/".to_string(),
            expires_at: Some("2030-01-01T00:00:00Z".to_string()),
            secure: true,
            http_only: true,
        },
        ProviderAccountCookie {
            domain: ".instagram.com".to_string(),
            name: "csrftoken".to_string(),
            value: "csrf123".to_string(),
            path: "/".to_string(),
            expires_at: Some("2030-01-01T00:00:00Z".to_string()),
            secure: true,
            http_only: false,
        },
    ]
}

#[test]
fn batch_update_source_profiles_rolls_back_when_any_source_is_invalid() {
    let (_temp_dir, layout) = create_test_layout();

    with_workspace_layout(layout.clone(), |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;
        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-1", "instagram", Some("account-1")),
        )
    })
    .expect("source setup");

    let result = with_workspace_layout(layout.clone(), |connection, test_layout| {
        batch_update_source_profiles_with_connection(
            connection,
            test_layout,
            BatchSourceProfilePatch {
                source_ids: vec!["source-1".to_string(), "missing-source".to_string()],
                labels_to_add: vec!["batch-updated".to_string()],
                labels_to_remove: Vec::new(),
                ready_for_download: Some(false),
                sync_options_patch: None,
                set_group_id: None,
            },
        )
    });

    assert!(
        result.is_err(),
        "batch update should fail for missing source"
    );

    let source = with_workspace_layout(layout, |connection, _| {
        load_source_profile_by_id(connection, "source-1")
    })
    .expect("source should remain available");

    assert!(
        !source.labels.iter().any(|label| label == "batch-updated"),
        "labels should not be partially applied after rollback"
    );
    assert!(
        source.ready_for_download,
        "ready-for-download should remain unchanged after rollback"
    );
}

#[test]
fn batch_update_source_profiles_rejects_unknown_group_id_without_partial_changes() {
    let (_temp_dir, layout) = create_test_layout();

    with_workspace_layout(layout.clone(), |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;
        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-1", "instagram", Some("account-1")),
        )
    })
    .expect("source setup");

    let result = with_workspace_layout(layout.clone(), |connection, test_layout| {
        batch_update_source_profiles_with_connection(
            connection,
            test_layout,
            BatchSourceProfilePatch {
                source_ids: vec!["source-1".to_string()],
                labels_to_add: vec!["batch-updated".to_string()],
                labels_to_remove: Vec::new(),
                ready_for_download: None,
                sync_options_patch: None,
                set_group_id: Some(Some("missing-group".to_string())),
            },
        )
    });

    assert!(
        matches!(result, Err(message) if message.contains("Scheduler group not found")),
        "missing group should fail fast with a clear error"
    );

    let (group_id, labels): (Option<String>, Vec<String>) =
        with_workspace_layout(layout, |connection, _| {
            connection
                .query_row(
                    "SELECT group_id, labels_json FROM source_profiles WHERE id = ?1",
                    params!["source-1"],
                    |row| {
                        let group_id: Option<String> = row.get(0)?;
                        let labels_json: String = row.get(1)?;
                        Ok((
                            group_id,
                            serde_json::from_str(&labels_json).unwrap_or_default(),
                        ))
                    },
                )
                .map_err(|error| error.to_string())
        })
        .expect("source should remain unchanged");

    assert!(group_id.is_none(), "group assignment should not be applied");
    assert!(
        !labels.iter().any(|label| label == "batch-updated"),
        "labels should remain unchanged after validation failure"
    );
}

#[test]
fn batch_update_source_profiles_applies_group_and_sync_patch_when_inputs_are_valid() {
    let (_temp_dir, layout) = create_test_layout();

    with_workspace_layout(layout.clone(), |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;
        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-1", "instagram", Some("account-1")),
        )?;
        upsert_scheduler_group_with_connection(
            connection,
            SchedulerGroupUpsert {
                id: Some("group-1".to_string()),
                name: "Batch group".to_string(),
                sort_index: Some(1),
                criteria: SchedulerPlanCriteria::default(),
            },
        )
    })
    .expect("setup source and group");

    with_workspace_layout(layout.clone(), |connection, test_layout| {
        batch_update_source_profiles_with_connection(
            connection,
            test_layout,
            BatchSourceProfilePatch {
                source_ids: vec!["source-1".to_string()],
                labels_to_add: Vec::new(),
                labels_to_remove: Vec::new(),
                ready_for_download: None,
                sync_options_patch: Some(BatchSourceSyncOptionsPatch {
                    instagram: Some(serde_json::json!({
                        "temporary": false,
                        "downloadImages": false,
                        "getUserMediaOnly": true,
                    })),
                    ..Default::default()
                }),
                set_group_id: Some(Some("group-1".to_string())),
            },
        )
    })
    .expect("batch update should succeed");

    let source = with_workspace_layout(layout, |connection, _| {
        connection
            .query_row(
                "SELECT group_id, sync_options_json FROM source_profiles WHERE id = ?1",
                params!["source-1"],
                |row| {
                    let group_id: Option<String> = row.get(0)?;
                    let sync_options_json: String = row.get(1)?;
                    Ok((group_id, sync_options_json))
                },
            )
            .map_err(|error| error.to_string())
    })
    .expect("source should persist updates");

    assert_eq!(source.0.as_deref(), Some("group-1"));
    let sync_options = deserialize_source_sync_options("instagram", &source.1);
    let instagram = sync_options
        .instagram
        .expect("instagram sync options should exist after patch");
    assert_eq!(instagram.temporary, Some(false));
    assert_eq!(instagram.download_images, Some(false));
    assert_eq!(instagram.get_user_media_only, Some(true));
}

#[test]
fn batch_update_source_profiles_applies_only_the_matching_provider_patch() {
    let (_temp_dir, layout) = create_test_layout();

    with_workspace_layout(layout.clone(), |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("twitter-account", "twitter"),
        )?;
        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("twitter-source", "twitter", Some("twitter-account")),
        )
    })
    .expect("setup twitter source");

    with_workspace_layout(layout.clone(), |connection, test_layout| {
        batch_update_source_profiles_with_connection(
            connection,
            test_layout,
            BatchSourceProfilePatch {
                source_ids: vec!["twitter-source".to_string()],
                labels_to_add: Vec::new(),
                labels_to_remove: Vec::new(),
                ready_for_download: None,
                sync_options_patch: Some(BatchSourceSyncOptionsPatch {
                    instagram: Some(serde_json::json!({ "downloadImages": false })),
                    twitter: Some(serde_json::json!({
                        "downloadGifs": false,
                        "gifsPrefix": "ANIM_",
                    })),
                    tiktok: None,
                }),
                set_group_id: None,
            },
        )
    })
    .expect("twitter batch update should succeed");

    let persisted: serde_json::Value = with_workspace_layout(layout, |connection, _| {
        connection
            .query_row(
                "SELECT sync_options_json FROM source_profiles WHERE id = ?1",
                params!["twitter-source"],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| error.to_string())
    })
    .map(|json| serde_json::from_str(&json).expect("valid sync options json"))
    .expect("load twitter source");

    assert_eq!(persisted["twitter"]["downloadGifs"], false);
    assert_eq!(persisted["twitter"]["gifsPrefix"], "ANIM_");
    assert!(persisted.get("instagram").is_none());
}

#[test]
fn download_success_summary_uses_short_copy_for_zero_items() {
    assert_eq!(
        format_download_success_summary("Instagram sync succeeded.", 0),
        "Instagram sync succeeded. No new media downloaded."
    );
    assert_eq!(
        format_download_success_summary("Saved posts sync succeeded.", 0),
        "Saved posts sync succeeded. No new media downloaded."
    );
    assert_eq!(
        format_download_success_summary("Instagram sync succeeded.", 3),
        "Instagram sync succeeded. Downloaded 3 media items."
    );
}

#[test]
fn twitter_breakdown_is_only_shown_for_multiple_download_sources() {
    let one = std::collections::BTreeMap::from([("media".to_string(), 2)]);
    assert_eq!(format_twitter_download_breakdown(&one), None);

    let multiple =
        std::collections::BTreeMap::from([("likes".to_string(), 3), ("media".to_string(), 2)]);
    assert_eq!(
        format_twitter_download_breakdown(&multiple).as_deref(),
        Some(" Breakdown: 3 from liked posts, 2 from profile posts.")
    );
}

#[test]
fn twitter_handle_redirect_is_single_run_safe_and_ignores_rate_limits() {
    assert_eq!(
        twitter_handle_redirect("samaissc", Some("@ruivinhasv"), false).as_deref(),
        Some("ruivinhasv")
    );
    assert_eq!(
        twitter_handle_redirect("ruivinhasv", Some("@RUIVINHASV"), false),
        None
    );
    assert_eq!(
        twitter_handle_redirect("samaissc", Some("@ruivinhasv"), true),
        None
    );
    assert_eq!(twitter_handle_redirect("samaissc", Some("@"), false), None);
}

#[test]
fn twitter_incremental_state_requires_fresh_full_scan_and_same_selection() {
    let now = Utc
        .with_ymd_and_hms(2026, 7, 11, 20, 0, 0)
        .single()
        .expect("now");
    let signature = "v1:images=true;videos=true;gifs=true;non-user=false";
    let eligible = twitter_connector::TwitterManifestSummary {
        attempted_model_count: 1,
        completed_model_count: 1,
        selection_signature: Some(signature.to_string()),
        full_scan_at: Some("2026-07-10T20:00:00Z".to_string()),
        incremental_cutoff_timestamp: Some(1_720_000_000),
        ..Default::default()
    };

    assert_eq!(
        select_twitter_incremental_state(&[eligible.clone()], signature, now),
        Some(TwitterIncrementalState {
            cutoff_timestamp: 1_720_000_000,
            full_scan_at: "2026-07-10T20:00:00Z".to_string(),
        })
    );
    assert_eq!(
        select_twitter_incremental_state(&[eligible.clone()], "changed", now),
        None
    );

    let mut stale = eligible.clone();
    stale.full_scan_at = Some("2026-07-01T20:00:00Z".to_string());
    assert_eq!(
        select_twitter_incremental_state(&[stale], signature, now),
        None
    );

    let mut limited = eligible.clone();
    limited.rate_limited = true;
    assert_eq!(
        select_twitter_incremental_state(&[limited], signature, now),
        None
    );
}

#[test]
fn twitter_media_selection_signature_changes_with_download_policy() {
    let mut options = TwitterSourceSyncOptions {
        download_images: Some(true),
        download_videos: Some(true),
        download_gifs: Some(true),
        allow_non_user_tweets: Some(false),
        ..Default::default()
    };
    let initial = twitter_media_selection_signature(&options);
    options.download_images = Some(false);

    assert_ne!(twitter_media_selection_signature(&options), initial);
}

#[test]
fn twitter_disabled_media_copy_explains_intentional_skips() {
    let images = std::collections::BTreeMap::from([("image".to_string(), 8)]);
    assert_eq!(
        format_twitter_disabled_media_suffix(&images),
        " 8 images were skipped because image downloads are disabled."
    );

    let mixed =
        std::collections::BTreeMap::from([("gif".to_string(), 1), ("video".to_string(), 2)]);
    assert_eq!(
        format_twitter_disabled_media_suffix(&mixed),
        " 1 GIF, 2 videos were skipped because these media types are disabled."
    );
}

#[test]
fn connector_sync_summary_preserves_degraded_capabilities() {
    assert_eq!(
        format_connector_sync_success_summary(0, &[]),
        "Connector sync succeeded. No new media downloaded."
    );
    assert_eq!(
        format_connector_sync_success_summary(0, &["stories".to_string()]),
        "Connector sync succeeded. No new media downloaded. Degraded capabilities: stories."
    );
    assert_eq!(
        format_connector_sync_success_summary(2, &["stories".to_string()]),
        "Connector sync succeeded. Downloaded 2 media items with degraded capabilities: stories."
    );
}

#[test]
fn instagram_manifest_suffix_is_omitted_for_zero_download_success() {
    let manifest_summary = sample_instagram_manifest_summary();
    assert_eq!(
        format_instagram_manifest_suffix(Some(&manifest_summary), false),
        ""
    );
    assert_eq!(
        format_instagram_manifest_suffix(Some(&manifest_summary), true),
        " 4 posts already up to date."
    );
}

#[test]
fn instagram_manifest_suffix_is_empty_when_nothing_was_already_synced() {
    // Tudo novo (nenhum post reconhecido como já sincronizado) → sem sufixo; o
    // resumo base ("Downloaded N media items.") já basta.
    let manifest_summary = instagram_connector::InstagramManifestSummary {
        section_count: 4,
        discovered_item_count: 0,
        normalized_post_count: 12,
        discovered_asset_count: 0,
        queued_asset_count: 13,
        skipped_existing_post_count: 0,
        skipped_duplicate_post_count: 0,
        skipped_unavailable_post_count: 0,
        skipped_existing_asset_count: 0,
        skipped_duplicate_asset_count: 0,
        downloaded_asset_count: 0,
        profile_user_id: None,
        sections: vec![],
    };

    assert_eq!(
        format_instagram_manifest_suffix(Some(&manifest_summary), true),
        ""
    );

    // Singular quando só 1 post já estava em dia.
    let single = instagram_connector::InstagramManifestSummary {
        skipped_existing_post_count: 1,
        ..manifest_summary
    };
    assert_eq!(
        format_instagram_manifest_suffix(Some(&single), true),
        " 1 post already up to date."
    );
}

#[test]
fn failed_source_sync_run_does_not_advance_last_synced_at() {
    let (_temp_dir, layout) = create_test_layout();

    let (after_failed, after_success) = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "tiktok"),
        )?;
        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-1", "tiktok", Some("account-1")),
        )?;
        let source = load_source_profile_by_id(connection, "source-1")?;
        let context = SourceSyncContext {
            source,
            account: ProviderAccount {
                id: "account-1".to_string(),
                provider: "tiktok".to_string(),
                display_name: "tiktok-account".to_string(),
                auth_mode: "imported_session".to_string(),
                auth_state: "ready".to_string(),
                capabilities: vec!["posts".to_string()],
                last_validated_at: "2026-03-10T00:00:00Z".to_string(),
            },
            session_payload: "{}".to_string(),
        };
        let failed = SourceSyncOutcome {
            tool: "internal.tiktok".to_string(),
            status: "failed".to_string(),
            summary: "TikTok sync failed: transient listing failure".to_string(),
            command_preview: "internal.tiktok profile @source-1".to_string(),
            manifest_summary_json: None,
            degraded_capabilities: Vec::new(),
            validation_error: Some("transient listing failure".to_string()),
        };
        persist_source_sync_run(
            connection,
            &context,
            &failed,
            "manual",
            "2026-07-08T00:00:00Z",
            "2026-07-08T00:00:01Z",
        )?;
        let after_failed: Option<String> = connection
            .query_row(
                "SELECT last_synced_at FROM source_profiles WHERE id = 'source-1'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;

        let succeeded = SourceSyncOutcome {
            status: "succeeded".to_string(),
            summary: "TikTok sync succeeded.".to_string(),
            validation_error: None,
            ..failed
        };
        persist_source_sync_run(
            connection,
            &context,
            &succeeded,
            "manual",
            "2026-07-08T00:01:00Z",
            "2026-07-08T00:01:01Z",
        )?;
        let after_success: Option<String> = connection
            .query_row(
                "SELECT last_synced_at FROM source_profiles WHERE id = 'source-1'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        Ok((after_failed, after_success))
    })
    .expect("source sync run persistence should work");

    assert_eq!(after_failed, None);
    assert_eq!(after_success.as_deref(), Some("2026-07-08T00:01:01Z"));
}

fn seed_instagram_auth_settings(
    connection: &Connection,
    layout: &StorageLayout,
    account_id: &str,
) -> Result<(), String> {
    save_provider_account_settings_with_connection(
        connection,
        layout,
        account_id.to_string(),
        vec![ProviderAccountSettingValue {
            setting_key: "instagram.auth.appId".to_string(),
            value_kind: ProviderAccountSettingValueKind::String,
            string_value: Some("936619743392459".to_string()),
            json_value: None,
        }],
    )?;
    Ok(())
}

fn seed_instagram_session(
    connection: &Connection,
    layout: &StorageLayout,
    account_id: &str,
) -> Result<(), String> {
    seed_instagram_auth_settings(connection, layout, account_id)?;
    let _ = save_provider_account_cookies_with_connection(
        connection,
        layout,
        account_id,
        sample_instagram_cookies(),
    )?;
    Ok(())
}

fn string_provider_setting(key: &str, value: &str) -> ProviderAccountSettingValue {
    ProviderAccountSettingValue {
        setting_key: key.to_string(),
        value_kind: ProviderAccountSettingValueKind::String,
        string_value: Some(value.to_string()),
        json_value: None,
    }
}

fn sample_scheduler_set(id: &str, active: bool) -> SchedulerSetUpsert {
    SchedulerSetUpsert {
        id: Some(id.to_string()),
        name: format!("set-{id}"),
        active,
    }
}

fn sample_sync_plan(
    id: &str,
    scheduler_set_id: &str,
    mode: &str,
    interval_minutes: u32,
    startup_delay_minutes: u32,
) -> SyncPlanUpsert {
    SyncPlanUpsert {
        id: Some(id.to_string()),
        scheduler_set_id: scheduler_set_id.to_string(),
        name: format!("plan-{id}"),
        enabled: true,
        mode: mode.to_string(),
        interval_minutes,
        startup_delay_minutes,
        notification_mode: "summary".to_string(),
        target_filter: "label = priority".to_string(),
        sort_index: None,
        pause_mode: None,
        pause_until: None,
        notifications: SchedulerPlanNotifications::default(),
        criteria: SchedulerPlanCriteria::default(),
    }
}

#[test]
fn push_previous_instagram_handle_dedupes_and_ignores_current() {
    // Adiciona o nome antigo normalizado.
    let list = push_previous_instagram_handle(None, "@OldName", "newname");
    assert_eq!(list, Some(vec!["OldName".to_string()]));
    // Não duplica (case/@ insensitive) e mantém o existente.
    let list = push_previous_instagram_handle(list, "oldname", "newname");
    assert_eq!(list, Some(vec!["OldName".to_string()]));
    // Acrescenta um segundo nome antigo distinto.
    let list = push_previous_instagram_handle(list, "older_one", "newname");
    assert_eq!(
        list,
        Some(vec!["OldName".to_string(), "older_one".to_string()])
    );
    // O handle atual nunca entra na lista.
    assert_eq!(
        push_previous_instagram_handle(None, "@newname", "newname"),
        None
    );
}

#[test]
fn source_dedupe_key_normalizes_at_prefix_and_case() {
    assert_eq!(
        source_dedupe_key("instagram", "@Poliana"),
        source_dedupe_key("instagram", "poliana")
    );
    assert_eq!(source_dedupe_key("instagram", "  @Foo/ "), "foo");
    // TikTok mantém o '@' canônico mas continua case-insensitive.
    assert_eq!(
        source_dedupe_key("tiktok", "Bar"),
        source_dedupe_key("tiktok", "@bar")
    );
}

#[test]
fn find_conflicting_source_handle_detects_at_prefix_duplicates() {
    let connection = Connection::open_in_memory().expect("in-memory connection");
    connection
        .execute_batch(
            "CREATE TABLE source_profiles (
                    id TEXT PRIMARY KEY,
                    provider TEXT NOT NULL,
                    handle TEXT NOT NULL,
                    deleted_at TEXT
                 );
                 INSERT INTO source_profiles (id, provider, handle, deleted_at)
                 VALUES ('keep', 'instagram', 'polianaarapiraca', NULL),
                        ('gone', 'instagram', 'removed_one', '2026-01-01T00:00:00Z'),
                        ('tt',   'tiktok',    '@polianaarapiraca', NULL);",
        )
        .expect("seed source_profiles");

    // '@polianaarapiraca' colide com 'polianaarapiraca' do mesmo provider.
    assert_eq!(
        find_conflicting_source_handle(&connection, "instagram", "@polianaarapiraca", "new-id")
            .expect("query ok"),
        Some("polianaarapiraca".to_string())
    );
    // O próprio registro nunca conflita consigo mesmo.
    assert_eq!(
        find_conflicting_source_handle(&connection, "instagram", "polianaarapiraca", "keep")
            .expect("query ok"),
        None
    );
    // Perfis excluídos (deleted_at) e outros providers não contam.
    assert_eq!(
        find_conflicting_source_handle(&connection, "instagram", "removed_one", "new-id")
            .expect("query ok"),
        None
    );
    // Handle novo e único não acusa conflito.
    assert_eq!(
        find_conflicting_source_handle(&connection, "instagram", "@brand_new", "new-id")
            .expect("query ok"),
        None
    );
}

#[test]
fn instagram_post_ledger_snapshot_round_trips_keys_and_codes() {
    let connection = Connection::open_in_memory().expect("in-memory connection");
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .expect("enable foreign keys");
    connection
        .execute_batch(
            "CREATE TABLE source_profiles (id TEXT PRIMARY KEY);
                 CREATE TABLE provider_accounts (id TEXT PRIMARY KEY);",
        )
        .expect("create minimal foreign-key tables");
    connection
        .execute(
            "INSERT INTO source_profiles (id) VALUES (?1)",
            params!["source-1"],
        )
        .expect("insert source");
    connection
        .execute(
            "INSERT INTO provider_accounts (id) VALUES (?1)",
            params!["account-1"],
        )
        .expect("insert account");

    upsert_instagram_post_ledger_entries(
        &connection,
        "source-1",
        "account-1",
        "@handle",
        &[instagram_connector::ObservedInstagramPost {
            provider_post_key: "Post-1".to_string(),
            provider_post_code: Some("ABC123".to_string()),
            media_section: "timeline".to_string(),
        }],
        "2026-03-14T12:00:00Z",
    )
    .expect("upsert post ledger");

    let snapshot = load_instagram_post_ledger_snapshot_for_source(&connection, "source-1")
        .expect("load post ledger snapshot");

    assert!(snapshot.keys.contains("post-1"));
    assert!(snapshot.keys.contains("abc123"));
}

#[test]
fn instagram_media_identity_candidates_strip_default_datetime_prefix() {
    let candidates = extract_instagram_media_identity_candidates_from_file_name(
        "2026-03-21 10.11.12 631495592_18384355651158098_6314965943446164250_n.jpg",
    );

    assert!(
        candidates.contains(
            &"2026-03-21 10.11.12 631495592_18384355651158098_6314965943446164250_n".to_string()
        ),
        "expected the full filename stem to remain a valid lookup candidate"
    );
    assert!(
        candidates.contains(&"631495592_18384355651158098_6314965943446164250_n".to_string()),
        "expected the provider media key suffix to be extracted from the new default naming pattern"
    );
}

#[test]
fn merged_import_roots_prefer_managed_origin_for_duplicate_paths() {
    let mut descriptor = ImportRootDescriptor {
        path: r"F:\SCrawler\Data\instagram".to_string(),
        source: "default".to_string(),
        label: "Media root".to_string(),
        removable: false,
    };

    merge_import_root_descriptors(
        &mut descriptor,
        ImportRootDescriptor {
            path: r"F:\SCrawler\Data\instagram".to_string(),
            source: "manual".to_string(),
            label: "Manual root".to_string(),
            removable: true,
        },
    );

    assert!(
        !descriptor.removable,
        "manual duplicate should collapse into managed root"
    );
    assert_eq!(descriptor.source, "default");
    assert_eq!(descriptor.label, "Media root");
}

#[test]
fn bootstrap_workspace_starts_without_demo_records() {
    let (_temp_dir, layout) = create_test_layout();

    let snapshot = with_workspace_layout(layout, load_snapshot).expect("bootstrap snapshot");

    assert!(
        snapshot.accounts.is_empty(),
        "fresh workspace should not seed accounts"
    );
    assert!(
        snapshot.sources.is_empty(),
        "fresh workspace should not seed sources"
    );
    assert!(
        snapshot.scheduler_sets.is_empty(),
        "fresh workspace should not seed scheduler sets"
    );
}

#[test]
fn bootstrap_workspace_seeds_default_settings_only() {
    let (_temp_dir, layout) = create_test_layout();

    let snapshot = with_workspace_layout(layout, load_snapshot).expect("bootstrap snapshot");

    assert!(
        snapshot
            .app_settings
            .iter()
            .any(|setting| setting.key == "tool.yt-dlp.path"),
        "default tool settings should be present"
    );
    assert!(
        snapshot
            .app_settings
            .iter()
            .any(|setting| setting.key == "policy.notifications.default"),
        "default policy settings should be present"
    );
    assert!(
        snapshot.desktop_runtime.close_to_tray,
        "desktop runtime should default to close-to-tray"
    );
    assert!(
        !snapshot.desktop_runtime.silent_mode,
        "desktop runtime should default to non-silent mode"
    );
}

#[test]
fn desktop_runtime_settings_roundtrip_into_snapshot_state() {
    let (_temp_dir, layout) = create_test_layout();

    let snapshot = with_workspace_layout(layout, |connection, test_layout| {
        upsert_app_setting_value(connection, DESKTOP_CLOSE_TO_TRAY_SETTING_KEY, "false")?;
        upsert_app_setting_value(connection, DESKTOP_SILENT_MODE_SETTING_KEY, "true")?;
        load_snapshot(connection, test_layout)
    })
    .expect("desktop runtime settings should load into snapshot");

    assert!(
        !snapshot.desktop_runtime.close_to_tray,
        "close-to-tray should reflect persisted app settings"
    );
    assert!(
        snapshot.desktop_runtime.silent_mode,
        "silent mode should reflect persisted app settings"
    );
}

#[test]
fn source_upsert_requires_explicit_account_binding() {
    let (_temp_dir, layout) = create_test_layout();

    let error = with_workspace_layout(layout, |connection, test_layout| {
        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-1", "instagram", None),
        )
    })
    .err()
    .expect("source binding without account should fail");

    assert!(
        error.contains("explicit provider account"),
        "unexpected error: {error}"
    );
}

#[test]
fn source_upsert_requires_matching_account_provider() {
    let (_temp_dir, layout) = create_test_layout();

    let error = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;

        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-1", "tiktok", Some("account-1")),
        )
    })
    .err()
    .expect("cross-provider binding should fail");

    assert!(error.contains("cannot bind"), "unexpected error: {error}");
}

#[test]
fn source_upsert_persists_matching_account_binding() {
    let (_temp_dir, layout) = create_test_layout();

    let snapshot = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;

        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-1", "instagram", Some("account-1")),
        )
    })
    .expect("source binding with matching provider should succeed");

    assert_eq!(snapshot.sources.len(), 1, "expected one persisted source");
    assert_eq!(
        snapshot.sources[0].account_id.as_deref(),
        Some("account-1"),
        "source should persist the explicit account binding"
    );
}

#[test]
fn instagram_saved_posts_request_honors_account_defaults() {
    let (_temp_dir, layout) = create_test_layout();

    let request = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;
        seed_instagram_session(connection, test_layout, "account-1")?;
        save_provider_account_settings_with_connection(
            connection,
            test_layout,
            "account-1".to_string(),
            vec![
                string_provider_setting(
                    "instagram.account.extractSavedPostsImageFromVideo",
                    "false",
                ),
                string_provider_setting("instagram.defaults.downloadText", "true"),
                string_provider_setting("instagram.defaults.downloadTextPosts", "true"),
                string_provider_setting("instagram.defaults.textSpecialFolder", "false"),
                string_provider_setting(
                    "instagram.defaults.placeExtractedImageIntoVideoFolder",
                    "true",
                ),
            ],
        )?;

        let context = load_account_sync_context(connection, test_layout, "account-1")?;
        build_instagram_saved_posts_request(test_layout, &context)
    })
    .expect("saved-post request should build");

    assert!(
        !request.extract_image_from_video.timeline
            && !request.extract_image_from_video.reels
            && !request.extract_image_from_video.stories
            && !request.extract_image_from_video.stories_user
            && !request.extract_image_from_video.tagged,
        "saved-post extract-image flags should honor the account setting"
    );
    assert!(
        request.place_extracted_image_into_video_folder,
        "saved-post request should honor the account default for extracted image placement"
    );
    assert!(
        request.download_text,
        "saved-post request should honor the account default for text downloads"
    );
    assert!(
        request.download_text_posts,
        "saved-post request should honor the account default for text-post downloads"
    );
    assert!(
        !request.text_special_folder,
        "saved-post request should honor the account default for text special folder"
    );
}

#[test]
fn resolved_source_media_output_root_uses_instagram_account_media_path_setting() {
    let (temp_dir, layout) = create_test_layout();
    let custom_media_root = temp_dir.path().join("custom-instagram-media");

    let resolved_root = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;
        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-1", "instagram", Some("account-1")),
        )?;
        save_provider_account_settings_with_connection(
            connection,
            test_layout,
            "account-1".to_string(),
            vec![ProviderAccountSettingValue {
                setting_key: "instagram.account.mediaPath".to_string(),
                value_kind: ProviderAccountSettingValueKind::String,
                string_value: Some(custom_media_root.display().to_string()),
                json_value: None,
            }],
        )?;

        let source = load_sources(connection)?
            .into_iter()
            .find(|item| item.id == "source-1")
            .ok_or_else(|| "source should exist".to_string())?;
        resolved_source_media_output_root_with_connection(connection, test_layout, &source)
    })
    .expect("source root should resolve");

    assert_eq!(
        resolved_root,
        custom_media_root.join("source-1"),
        "instagram sources should honor account-level mediaPath when resolving root"
    );
}

#[test]
fn snapshot_exposes_save_paths_for_all_supported_providers() {
    let (temp_dir, layout) = create_test_layout();
    let instagram_base = temp_dir.path().join("instagram-account");
    let twitter_base = temp_dir.path().join("twitter-account");
    let tiktok_special_path = temp_dir.path().join("tiktok-special");

    let paths = with_workspace_layout(layout, |connection, test_layout| {
        for (id, provider) in [
            ("instagram-account", "instagram"),
            ("twitter-account", "twitter"),
            ("tiktok-account", "tiktok"),
        ] {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account(id, provider),
            )?;
        }

        save_provider_account_settings_with_connection(
            connection,
            test_layout,
            "instagram-account".to_string(),
            vec![ProviderAccountSettingValue {
                setting_key: "instagram.account.mediaPath".to_string(),
                value_kind: ProviderAccountSettingValueKind::String,
                string_value: Some(instagram_base.display().to_string()),
                json_value: None,
            }],
        )?;
        save_provider_account_settings_with_connection(
            connection,
            test_layout,
            "twitter-account".to_string(),
            vec![ProviderAccountSettingValue {
                setting_key: "twitter.account.mediaPath".to_string(),
                value_kind: ProviderAccountSettingValueKind::String,
                string_value: Some(twitter_base.display().to_string()),
                json_value: None,
            }],
        )?;

        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("instagram-source", "instagram", Some("instagram-account")),
        )?;
        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("twitter-source", "twitter", Some("twitter-account")),
        )?;
        let mut tiktok_source = sample_source("tiktok-source", "tiktok", Some("tiktok-account"));
        tiktok_source
            .sync_options
            .tiktok
            .as_mut()
            .expect("TikTok source should have default options")
            .special_path = Some(tiktok_special_path.display().to_string());
        upsert_source_profile_with_connection(connection, test_layout, tiktok_source)?;

        Ok(load_snapshot(connection, test_layout)?.source_media_paths)
    })
    .expect("snapshot should expose every provider save path");

    assert_eq!(
        paths.get("instagram-source"),
        Some(
            &instagram_base
                .join("instagram-source")
                .display()
                .to_string()
        )
    );
    assert_eq!(
        paths.get("twitter-source"),
        Some(&twitter_base.join("twitter-source").display().to_string())
    );
    assert_eq!(
        paths.get("tiktok-source"),
        Some(&tiktok_special_path.display().to_string()),
        "a per-profile special path must take precedence over the account and global roots"
    );
}

#[test]
fn instagram_media_identity_keys_backfill_from_legacy_file_names() {
    let (_temp_dir, layout) = create_test_layout();

    let keys = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;

        let mut source = sample_source("source-1", "instagram", Some("account-1"));
        source.handle = "@_theecat".to_string();
        upsert_source_profile_with_connection(connection, test_layout, source)?;

        let profile_root = test_layout.media_root.join("instagram").join("_theecat");
        fs::create_dir_all(&profile_root).map_err(|error| error.to_string())?;
        let legacy_file =
            profile_root.join("631495592_18384355651158098_6314965943446164250_n.jpg");
        fs::write(&legacy_file, b"legacy").map_err(|error| error.to_string())?;
        // A varredura de identity keys lê o DISCO (o arquivo acima); a antiga
        // tabela media_items não existe mais no schema.
        let source = load_source_profile_by_id(connection, "source-1")?;
        let settings = load_app_settings_map(connection)?;
        load_existing_instagram_media_identity_keys_for_source(test_layout, &source, &settings)
    })
    .expect("legacy media keys should load");

    assert!(
        keys.contains("631495592_18384355651158098_6314965943446164250_n"),
        "expected imported legacy filename stem to be used as instagram media identity"
    );
}

#[test]
fn provider_account_cannot_change_provider_while_sources_are_bound() {
    let (_temp_dir, layout) = create_test_layout();

    with_workspace_layout(layout.clone(), |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;

        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-1", "instagram", Some("account-1")),
        )?;

        Ok(())
    })
    .expect("initial account and source setup");

    let error = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "tiktok"),
        )
    })
    .err()
    .expect("provider change should fail while sources remain bound");

    assert!(error.contains("bound source"), "unexpected error: {error}");
}

#[test]
fn imported_session_updates_account_state_and_persists_session_metadata() {
    let (_temp_dir, layout) = create_test_layout();

    let snapshot = with_workspace_layout(layout.clone(), |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;

        seed_instagram_auth_settings(connection, test_layout, "account-1")?;

        import_provider_account_cookies_with_connection(
            connection,
            test_layout,
            ProviderAccountCookieImport {
                account_id: "account-1".to_string(),
                import_format: "json".to_string(),
                content: serde_json::to_string(&sample_instagram_cookies())
                    .expect("cookie import json"),
            },
        )
    })
    .expect("session import should succeed");

    assert_eq!(snapshot.accounts.len(), 1, "expected one account");
    assert_eq!(snapshot.accounts[0].auth_mode, "imported_session");
    assert_eq!(
        snapshot.accounts[0].auth_state,
        "ready",
        "validation error: {:?}",
        snapshot
            .account_sessions
            .first()
            .and_then(|session| session.last_validation_error.clone())
    );
    assert_eq!(
        snapshot.account_sessions.len(),
        1,
        "expected one stored session"
    );
    assert!(
        snapshot.account_sessions[0].has_secret,
        "secret should exist in secure storage"
    );
    assert_eq!(snapshot.account_sessions[0].session_format, "cookie_json");
    assert!(
        !snapshot.account_sessions[0].fingerprint.is_empty(),
        "session fingerprint should be persisted"
    );

    // O secret_ref é gerado como "session-<account>-<uuid>"; busca o real.
    let secret_ref = with_workspace_layout(layout.clone(), |connection, _| {
        load_account_session_secret_ref(connection, "account-1")
    })
    .expect("load session secret ref")
    .expect("session secret ref should be stored");
    let restored_secret =
        session_secret_store::load_secret(&layout, &secret_ref).expect("session secret roundtrip");
    let restored_cookies =
        parse_session_cookies(&restored_secret).expect("restored session cookies");
    assert_eq!(
        restored_cookies.len(),
        2,
        "expected canonical cookie storage"
    );
    assert!(
        restored_cookies
            .iter()
            .any(|cookie| cookie.name == "sessionid" && cookie.value == "abc123"),
        "sessionid cookie should roundtrip through secure storage"
    );
}

#[test]
fn validate_provider_account_marks_expired_sessions() {
    let (_temp_dir, layout) = create_test_layout();

    let snapshot = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;

        seed_instagram_session(connection, test_layout, "account-1")?;
        connection
            .execute(
                "UPDATE provider_account_sessions
                     SET expires_at = ?2
                     WHERE account_id = ?1",
                params!["account-1", "2020-01-01T00:00:00Z"],
            )
            .map_err(|error| error.to_string())?;

        validate_provider_account_with_connection(connection, test_layout, "account-1".to_string())
    })
    .expect("validation should return snapshot");

    assert_eq!(snapshot.accounts[0].auth_state, "expired");
    assert_eq!(
        snapshot.account_sessions[0]
            .last_validation_error
            .as_deref(),
        Some("Stored session has expired.")
    );
}

#[test]
fn deleting_provider_account_removes_persisted_session_secret() {
    let (_temp_dir, layout) = create_test_layout();

    with_workspace_layout(layout.clone(), |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;

        seed_instagram_session(connection, test_layout, "account-1")?;

        Ok(())
    })
    .expect("initial account and session setup");

    let secret_ref = with_workspace_layout(layout.clone(), |connection, _| {
        load_account_session_secret_ref(connection, "account-1")
    })
    .expect("load session secret ref")
    .expect("session secret ref should be stored");
    assert!(session_secret_store::has_secret(&layout, &secret_ref).expect("secret before delete"));

    with_workspace_layout(layout.clone(), |connection, test_layout| {
        delete_provider_account_with_connection(connection, test_layout, "account-1".to_string())
    })
    .expect("delete account with session");

    assert!(
        !session_secret_store::has_secret(&layout, &secret_ref).expect("secret after delete"),
        "deleting the account should remove the secure session secret"
    );
}

#[test]
fn deleting_provider_account_ignores_soft_deleted_sources() {
    let (_temp_dir, layout) = create_test_layout();

    let snapshot = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;
        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-1", "instagram", Some("account-1")),
        )?;
        delete_source_profile_with_connection(
            connection,
            test_layout,
            "source-1".to_string(),
            SourceProfileDeleteMode::UserOnly,
        )?;
        delete_provider_account_with_connection(connection, test_layout, "account-1".to_string())
    })
    .expect("provider account deletion should ignore soft-deleted sources");

    assert!(snapshot.accounts.is_empty());
    assert!(snapshot.sources.is_empty());
}

#[test]
fn provider_account_settings_roundtrip_through_editor_load() {
    let (_temp_dir, layout) = create_test_layout();

    let editor = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;

        save_provider_account_settings_with_connection(
            connection,
            test_layout,
            "account-1".to_string(),
            vec![
                ProviderAccountSettingValue {
                    setting_key: "request.user_agent".to_string(),
                    value_kind: ProviderAccountSettingValueKind::String,
                    string_value: Some("Instagram 321.0".to_string()),
                    json_value: None,
                },
                ProviderAccountSettingValue {
                    setting_key: "sync.window".to_string(),
                    value_kind: ProviderAccountSettingValueKind::Json,
                    string_value: None,
                    json_value: Some(json!({
                        "includeStories": true,
                        "maxItems": 25
                    })),
                },
            ],
        )?;

        load_provider_account_editor_with_connection(
            connection,
            test_layout,
            "account-1".to_string(),
        )
    })
    .expect("editor should load persisted provider settings");

    assert_eq!(editor.account.id, "account-1");
    assert!(
        editor.session.is_none(),
        "account should not have session metadata"
    );
    assert_eq!(editor.settings.len(), 2, "expected two persisted settings");
    assert_eq!(
        editor.settings[0],
        ProviderAccountSettingValue {
            setting_key: "request.user_agent".to_string(),
            value_kind: ProviderAccountSettingValueKind::String,
            string_value: Some("Instagram 321.0".to_string()),
            json_value: None,
        }
    );
    assert_eq!(
        editor.settings[1],
        ProviderAccountSettingValue {
            setting_key: "sync.window".to_string(),
            value_kind: ProviderAccountSettingValueKind::Json,
            string_value: None,
            json_value: Some(json!({
                "includeStories": true,
                "maxItems": 25
            })),
        }
    );
}

#[test]
fn instagram_avatar_cooldown_persists_in_provider_settings() {
    let (_temp_dir, layout) = create_test_layout();

    with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;

        let until = set_instagram_avatar_cooldown(
            connection,
            "account-1",
            StdDuration::from_secs(90),
            "2026-03-20T06:00:00Z",
        )?;
        assert_eq!(until.to_rfc3339(), "2026-03-20T06:01:30+00:00");

        let settings = load_provider_account_settings_map(connection, "account-1")?;
        assert_eq!(
            read_instagram_avatar_cooldown_until(&settings).map(|value| value.to_rfc3339()),
            Some("2026-03-20T06:01:30+00:00".to_string())
        );

        clear_instagram_avatar_cooldown(connection, "account-1")?;

        let cleared_settings = load_provider_account_settings_map(connection, "account-1")?;
        assert_eq!(
            read_instagram_avatar_cooldown_until(&cleared_settings),
            None
        );

        Ok::<(), String>(())
    })
    .expect("avatar cooldown should persist through provider settings");
}

#[test]
fn clone_provider_account_copies_settings_without_session_material() {
    let (_temp_dir, layout) = create_test_layout();

    let (snapshot, cloned_account_id) =
        with_workspace_layout(layout.clone(), |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;

            seed_instagram_session(connection, test_layout, "account-1")?;

            save_provider_account_settings_with_connection(
                connection,
                test_layout,
                "account-1".to_string(),
                vec![
                    ProviderAccountSettingValue {
                        setting_key: "request.user_agent".to_string(),
                        value_kind: ProviderAccountSettingValueKind::String,
                        string_value: Some("Instagram 321.0".to_string()),
                        json_value: None,
                    },
                    ProviderAccountSettingValue {
                        setting_key: "sync.window".to_string(),
                        value_kind: ProviderAccountSettingValueKind::Json,
                        string_value: None,
                        json_value: Some(json!({ "maxItems": 25 })),
                    },
                ],
            )?;

            let snapshot = clone_provider_account_with_connection(
                connection,
                test_layout,
                "account-1".to_string(),
            )?;

            let cloned_account_id = snapshot
                .accounts
                .iter()
                .find(|account| account.id != "account-1")
                .map(|account| account.id.clone())
                .expect("cloned account id");

            Ok((snapshot, cloned_account_id))
        })
        .expect("clone provider account should succeed");

    assert_eq!(snapshot.accounts.len(), 2, "expected original plus clone");
    assert_eq!(
        snapshot.account_sessions.len(),
        1,
        "clone must not duplicate session metadata entries"
    );
    assert_eq!(
        snapshot.account_sessions[0].account_id, "account-1",
        "original session metadata should remain bound to the source account"
    );
    assert!(
        !session_secret_store::has_secret(&layout, &cloned_account_id)
            .expect("clone should not have a session secret"),
        "clone must not receive secret material"
    );

    let cloned_editor = with_workspace_layout(layout, |connection, test_layout| {
        load_provider_account_editor_with_connection(connection, test_layout, cloned_account_id)
    })
    .expect("cloned editor should load");

    assert!(
        cloned_editor.session.is_none(),
        "clone should not carry session metadata"
    );
    assert_eq!(
        cloned_editor.settings,
        vec![
            ProviderAccountSettingValue {
                setting_key: "request.user_agent".to_string(),
                value_kind: ProviderAccountSettingValueKind::String,
                string_value: Some("Instagram 321.0".to_string()),
                json_value: None,
            },
            ProviderAccountSettingValue {
                setting_key: "sync.window".to_string(),
                value_kind: ProviderAccountSettingValueKind::Json,
                string_value: None,
                json_value: Some(json!({ "maxItems": 25 })),
            },
        ],
        "clone should inherit advanced settings"
    );
}

#[test]
fn deleting_provider_account_cascades_provider_settings() {
    let (_temp_dir, layout) = create_test_layout();

    with_workspace_layout(layout.clone(), |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;

        save_provider_account_settings_with_connection(
            connection,
            test_layout,
            "account-1".to_string(),
            vec![ProviderAccountSettingValue {
                setting_key: "request.user_agent".to_string(),
                value_kind: ProviderAccountSettingValueKind::String,
                string_value: Some("Instagram 321.0".to_string()),
                json_value: None,
            }],
        )?;

        Ok(())
    })
    .expect("account settings setup");

    with_workspace_layout(layout.clone(), |connection, test_layout| {
        delete_provider_account_with_connection(connection, test_layout, "account-1".to_string())
    })
    .expect("delete account with advanced settings");

    let remaining_settings = with_workspace_layout(layout, |connection, _| {
        connection
            .query_row(
                "SELECT COUNT(*) FROM provider_account_settings WHERE account_id = ?1",
                params!["account-1"],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| error.to_string())
    })
    .expect("settings count after delete");

    assert_eq!(
        remaining_settings, 0,
        "provider account settings should cascade on delete"
    );
}

#[test]
#[ignore = "pre-existing: written for the generic ToolExecutor path, but instagram syncs now run the internal connector (real HTTP); needs an injectable HTTP client to test"]
fn running_source_sync_persists_successful_run_history() {
    struct SuccessfulExecutor;

    impl ToolExecutor for SuccessfulExecutor {
        fn execute(&self, _invocation: &ToolInvocation) -> Result<ToolExecutionResult, String> {
            Ok(ToolExecutionResult {
                status: "succeeded".to_string(),
            })
        }
    }

    let (_temp_dir, layout) = create_test_layout();

    let snapshot = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;
        seed_instagram_session(connection, test_layout, "account-1")?;
        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-1", "instagram", Some("account-1")),
        )?;

        run_source_sync_with_connection(
            connection,
            test_layout,
            "source-1".to_string(),
            "manual",
            None,
            None,
            &SuccessfulExecutor,
        )
    })
    .expect("source sync should succeed");

    assert_eq!(
        snapshot.source_sync_runs.len(),
        1,
        "expected one persisted sync run"
    );
    assert_eq!(snapshot.source_sync_runs[0].status, "succeeded");
    assert_eq!(snapshot.source_sync_runs[0].provider, "instagram");
    assert_eq!(snapshot.accounts[0].auth_state, "ready");
    assert!(
        snapshot.source_sync_runs[0]
            .command_preview
            .contains("gallery-dl"),
        "expected connector command preview to be persisted"
    );
}

#[test]
#[ignore = "pre-existing: written for the generic ToolExecutor path, but instagram syncs now run the internal connector (real HTTP); needs an injectable HTTP client to test"]
fn running_source_sync_persists_failed_run_and_degrades_account() {
    struct FailingExecutor;

    impl ToolExecutor for FailingExecutor {
        fn execute(&self, _invocation: &ToolInvocation) -> Result<ToolExecutionResult, String> {
            Err("gallery-dl exited with failure".to_string())
        }
    }

    let (_temp_dir, layout) = create_test_layout();

    let snapshot = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;
        seed_instagram_session(connection, test_layout, "account-1")?;
        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-1", "instagram", Some("account-1")),
        )?;

        run_source_sync_with_connection(
            connection,
            test_layout,
            "source-1".to_string(),
            "manual",
            None,
            None,
            &FailingExecutor,
        )
    })
    .expect("failed sync should still persist run history");

    assert_eq!(
        snapshot.source_sync_runs.len(),
        1,
        "expected failed run history"
    );
    assert_eq!(snapshot.source_sync_runs[0].status, "failed");
    assert_eq!(snapshot.accounts[0].auth_state, "degraded");
    assert!(
        snapshot.account_sessions[0]
            .last_validation_error
            .as_deref()
            .is_some_and(|value| value.contains("gallery-dl exited with failure")),
        "expected connector failure to propagate into account validation state"
    );
}

#[test]
fn running_instagram_source_sync_blocks_when_base_auth_is_missing() {
    struct SuccessfulExecutor;

    impl ToolExecutor for SuccessfulExecutor {
        fn execute(&self, _invocation: &ToolInvocation) -> Result<ToolExecutionResult, String> {
            Ok(ToolExecutionResult {
                status: "succeeded".to_string(),
            })
        }
    }

    let (_temp_dir, layout) = create_test_layout();

    let snapshot = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;
        let _ = save_provider_account_cookies_with_connection(
            connection,
            test_layout,
            "account-1",
            sample_instagram_cookies(),
        )?;
        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-1", "instagram", Some("account-1")),
        )?;

        run_source_sync_with_connection(
            connection,
            test_layout,
            "source-1".to_string(),
            "manual",
            None,
            None,
            &SuccessfulExecutor,
        )
    })
    .expect("preflight failure should still persist run history");

    assert_eq!(snapshot.source_sync_runs.len(), 1);
    assert_eq!(snapshot.source_sync_runs[0].status, "failed");
    assert!(
        snapshot.source_sync_runs[0]
            .summary
            .contains("required base auth"),
        "preflight summary should explain the missing base auth"
    );
    assert_eq!(snapshot.accounts[0].auth_state, "degraded");
    assert!(
        snapshot.account_sessions[0]
            .last_validation_error
            .as_deref()
            .is_some_and(|value| value.contains("required base auth")),
        "missing base auth should degrade the stored session state"
    );
}

#[test]
fn running_instagram_source_sync_skips_when_provider_cooldown_is_active() {
    struct SuccessfulExecutor;

    impl ToolExecutor for SuccessfulExecutor {
        fn execute(&self, _invocation: &ToolInvocation) -> Result<ToolExecutionResult, String> {
            Ok(ToolExecutionResult {
                status: "succeeded".to_string(),
            })
        }
    }

    let (_temp_dir, layout) = create_test_layout();

    let snapshot = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;
        seed_instagram_session(connection, test_layout, "account-1")?;
        save_provider_account_settings_with_connection(
            connection,
            test_layout,
            "account-1".to_string(),
            vec![ProviderAccountSettingValue {
                setting_key: INSTAGRAM_SYNC_COOLDOWN_UNTIL_SETTING_KEY.to_string(),
                value_kind: ProviderAccountSettingValueKind::String,
                string_value: Some("2030-01-01T00:00:00Z".to_string()),
                json_value: None,
            }],
        )?;
        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-1", "instagram", Some("account-1")),
        )?;

        run_source_sync_with_connection(
            connection,
            test_layout,
            "source-1".to_string(),
            "manual",
            None,
            None,
            &SuccessfulExecutor,
        )
    })
    .expect("cooldown skip should persist run history");

    assert_eq!(snapshot.source_sync_runs.len(), 1);
    assert_eq!(snapshot.source_sync_runs[0].status, "skipped");
    assert!(
        snapshot.source_sync_runs[0]
            .summary
            .contains("provider cooldown is active until 2030-01-01T00:00:00+00:00"),
        "skip summary should expose the cooldown deadline"
    );
    assert_eq!(snapshot.accounts[0].auth_state, "ready");
    assert!(
        snapshot.account_sessions[0].last_validation_error.is_none(),
        "cooldown skips should not degrade account health"
    );
}

#[test]
#[ignore = "pre-existing: written for the generic ToolExecutor path, but instagram syncs now run the internal connector (real HTTP); needs an injectable HTTP client to test"]
fn running_source_sync_cancellation_preserves_account_health() {
    struct CancelledExecutor;

    impl ToolExecutor for CancelledExecutor {
        fn execute(&self, _invocation: &ToolInvocation) -> Result<ToolExecutionResult, String> {
            Err("source sync cancelled by user".to_string())
        }
    }

    let (_temp_dir, layout) = create_test_layout();

    let snapshot = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;
        seed_instagram_session(connection, test_layout, "account-1")?;
        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-1", "instagram", Some("account-1")),
        )?;

        run_source_sync_with_connection(
            connection,
            test_layout,
            "source-1".to_string(),
            "manual",
            None,
            None,
            &CancelledExecutor,
        )
    })
    .expect("cancelled sync should persist run history");

    assert_eq!(snapshot.source_sync_runs.len(), 1);
    assert_eq!(snapshot.source_sync_runs[0].status, "failed");
    assert!(snapshot.source_sync_runs[0]
        .summary
        .to_ascii_lowercase()
        .contains("cancelled by user"));
    assert_eq!(snapshot.accounts[0].auth_state, "ready");
    assert!(
        snapshot.account_sessions[0].last_validation_error.is_none(),
        "manual cancellation should not mark account session as degraded"
    );
}

#[test]
fn ensure_avatar_thumbnail_generates_and_invalidates_by_mtime() {
    let (_temp_dir, layout) = create_test_layout();
    let original_path = layout.media_root.join(PROFILE_PICTURE_FILE_NAME);
    // PNG RGBA gravado com extensão .jpg de propósito: o decoder deve
    // adivinhar o formato pelo conteúdo e o encoder achatar o canal alfa.
    let original = image::RgbaImage::from_pixel(800, 600, image::Rgba([200, 30, 30, 128]));
    original
        .save_with_format(&original_path, image::ImageFormat::Png)
        .expect("original avatar");

    let thumbs_dir = layout.cache_root.join("avatar-thumbs");
    let count_thumbs = || {
        fs::read_dir(&thumbs_dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| {
                        entry.file_name().to_string_lossy().starts_with("source-1.")
                    })
                    .count()
            })
            .unwrap_or(0)
    };

    let thumb_path = ensure_avatar_thumbnail(&layout, "source-1", &original_path)
        .expect("thumbnail should generate");
    let thumb_name = Path::new(&thumb_path)
        .file_name()
        .expect("thumb file name")
        .to_string_lossy()
        .to_string();
    assert_eq!(Path::new(&thumb_path).parent(), Some(thumbs_dir.as_path()));
    // O nome carrega o mtime (cache-buster no path, não em query string).
    assert!(
        thumb_name.starts_with("source-1.") && thumb_name.ends_with(".jpg"),
        "thumb name should be source-1.<mtime>.jpg, got {thumb_name}"
    );
    let decoded = image::open(&thumb_path).expect("thumb should decode as jpeg");
    assert_eq!(
        (decoded.width(), decoded.height()),
        (AVATAR_THUMB_MAX_DIMENSION, 192),
        "800x600 should downscale preserving aspect ratio"
    );

    // Original inalterado: a segunda chamada reaproveita o mesmo jpg.
    let reused = ensure_avatar_thumbnail(&layout, "source-1", &original_path)
        .expect("thumbnail should be reused");
    assert_eq!(reused, thumb_path);
    assert_eq!(count_thumbs(), 1);

    // Original substituído (mtime mais novo) → novo path e o antigo é removido.
    std::thread::sleep(StdDuration::from_millis(50));
    let replacement = image::RgbaImage::from_pixel(300, 300, image::Rgba([10, 10, 200, 255]));
    replacement
        .save_with_format(&original_path, image::ImageFormat::Png)
        .expect("replacement avatar");
    let regenerated = ensure_avatar_thumbnail(&layout, "source-1", &original_path)
        .expect("thumbnail should regenerate");
    assert_ne!(regenerated, thumb_path, "a changed avatar yields a new thumb path");
    assert!(!Path::new(&thumb_path).exists(), "the stale thumb is removed");
    assert_eq!(count_thumbs(), 1, "old versions must not accumulate");
    let decoded = image::open(&regenerated).expect("regenerated thumb should decode");
    assert_eq!(
        (decoded.width(), decoded.height()),
        (AVATAR_THUMB_MAX_DIMENSION, AVATAR_THUMB_MAX_DIMENSION)
    );

    remove_avatar_thumbnail(&layout, "source-1");
    assert!(!Path::new(&regenerated).exists());
    assert_eq!(count_thumbs(), 0);
}

#[test]
fn ensure_image_thumbnail_generates_beside_media_and_invalidates_by_mtime() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let media_dir = temp_dir.path().join("instagram").join("someone");
    fs::create_dir_all(&media_dir).expect("media dir");
    let photo = media_dir.join("2026-05-19_photo.jpg");
    // PNG salvo com extensão .jpg de propósito: decode adivinha o formato e o
    // encoder achata o alfa. 1440x1800 espelha uma foto grande do Instagram (a
    // decisão de gerar é por dimensão, não por tamanho de arquivo).
    image::RgbaImage::from_pixel(1440, 1800, image::Rgba([40, 160, 220, 200]))
        .save_with_format(&photo, image::ImageFormat::Png)
        .expect("source photo");

    let thumb = ensure_image_thumbnail(&photo).expect("thumbnail should generate");
    let thumb_path = PathBuf::from(&thumb);
    // Convenção `.thumbs/<arquivo>.jpg` ao lado da mídia (mesma dos vídeos).
    assert_eq!(thumb_path.parent(), Some(media_dir.join(".thumbs").as_path()));
    assert_eq!(
        thumb_path.file_name().and_then(|n| n.to_str()),
        Some("2026-05-19_photo.jpg.jpg")
    );
    let decoded = image::open(&thumb_path).expect("thumb decodes as jpeg");
    assert_eq!(
        (decoded.width(), decoded.height()),
        (384, 480),
        "1440x1800 deve reduzir para caber em 480px preservando o aspecto"
    );

    // Reaproveita quando o original não mudou.
    let first_mtime = fs::metadata(&thumb_path)
        .and_then(|m| m.modified())
        .expect("mtime");
    assert_eq!(ensure_image_thumbnail(&photo).as_deref(), Some(thumb.as_str()));
    assert_eq!(
        fs::metadata(&thumb_path).and_then(|m| m.modified()).unwrap(),
        first_mtime,
        "thumb atual não deve ser reescrito"
    );

    // Original substituído (mtime maior) → regenera com o novo aspecto (prova a
    // invalidação por mtime).
    std::thread::sleep(StdDuration::from_millis(50));
    image::RgbaImage::from_pixel(800, 800, image::Rgba([10, 10, 10, 255]))
        .save_with_format(&photo, image::ImageFormat::Png)
        .expect("replacement photo");
    let regenerated = ensure_image_thumbnail(&photo).expect("regenerates");
    let decoded = image::open(&regenerated).expect("regenerated decodes");
    assert_eq!(
        (decoded.width(), decoded.height()),
        (480, 480),
        "800x800 deve reduzir para 480x480"
    );

    // Caso-chave: dimensões grandes mas arquivo minúsculo (cor sólida comprime
    // a poucos KB, como um JPEG bem comprimido do Instagram) AINDA gera thumb —
    // a decisão é por dimensão, não por bytes.
    let compressed = media_dir.join("compressed.jpg");
    image::RgbaImage::from_pixel(2000, 2000, image::Rgba([7, 7, 7, 255]))
        .save_with_format(&compressed, image::ImageFormat::Png)
        .expect("compressed photo");
    assert!(
        fs::metadata(&compressed).unwrap().len() < 150 * 1024,
        "cor sólida deve comprimir a poucos KB"
    );
    assert!(
        ensure_image_thumbnail(&compressed).is_some(),
        "dimensão grande gera thumb mesmo com arquivo pequeno"
    );

    // Dimensões pequenas (≤480px): sem thumb (o `thumbnail()` faria upscale); o
    // front usa o original, que já é leve.
    let small = media_dir.join("small.png");
    image::RgbaImage::from_pixel(320, 240, image::Rgba([1, 2, 3, 255]))
        .save_with_format(&small, image::ImageFormat::Png)
        .expect("small photo");
    assert!(
        ensure_image_thumbnail(&small).is_none(),
        "imagem ≤480px não deve gerar thumb (upscale)"
    );
    assert!(!video_thumbnail_path(&small).unwrap().exists());
}

#[test]
fn is_thumbnailable_image_covers_supported_formats_only() {
    assert!(is_thumbnailable_image(Path::new("a/b/photo.JPG")));
    assert!(is_thumbnailable_image(Path::new("x.jpeg")));
    assert!(is_thumbnailable_image(Path::new("x.png")));
    assert!(is_thumbnailable_image(Path::new("x.webp")));
    assert!(!is_thumbnailable_image(Path::new("x.gif")));
    assert!(!is_thumbnailable_image(Path::new("x.bmp")));
    assert!(!is_thumbnailable_image(Path::new("x.mp4")));
}

#[test]
fn queued_media_thumbnail_generation_dispatches_images_without_ffmpeg() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let large = temp_dir.path().join("large.jpg");
    image::RgbaImage::from_pixel(900, 1200, image::Rgba([20, 40, 60, 255]))
        .save_with_format(&large, image::ImageFormat::Png)
        .expect("large source image");
    assert_eq!(
        generate_media_thumbnail(&large),
        MediaThumbnailGenerationOutcome::Generated
    );
    assert!(video_thumbnail_path(&large).unwrap().is_file());

    let small = temp_dir.path().join("small.png");
    image::RgbaImage::from_pixel(320, 240, image::Rgba([1, 2, 3, 255]))
        .save_with_format(&small, image::ImageFormat::Png)
        .expect("small source image");
    assert_eq!(
        generate_media_thumbnail(&small),
        MediaThumbnailGenerationOutcome::NotNeeded
    );

    let invalid = temp_dir.path().join("invalid.webp");
    fs::write(&invalid, b"not an image").expect("invalid source image");
    assert_eq!(
        generate_media_thumbnail(&invalid),
        MediaThumbnailGenerationOutcome::Failed
    );
}

#[test]
fn ensure_avatar_thumbnail_returns_none_for_undecodable_input() {
    let (_temp_dir, layout) = create_test_layout();
    let original_path = layout.media_root.join(PROFILE_PICTURE_FILE_NAME);
    fs::write(&original_path, b"not an image at all").expect("bogus avatar");

    let result = ensure_avatar_thumbnail(&layout, "source-1", &original_path);
    assert!(result.is_none(), "undecodable input should not produce a thumb");
    let thumbs_dir = layout.cache_root.join("avatar-thumbs");
    let generated = fs::read_dir(&thumbs_dir)
        .map(|entries| {
            entries
                .flatten()
                .any(|entry| entry.file_name().to_string_lossy().starts_with("source-1."))
        })
        .unwrap_or(false);
    assert!(!generated, "no thumb file should be written for undecodable input");
}

#[test]
fn find_source_avatar_prefers_profile_picture_file() {
    let temp_dir = tempfile::tempdir().expect("temp directory");
    let root = temp_dir.path();
    fs::write(root.join("avatar.jpg"), b"legacy-avatar").expect("legacy avatar");
    fs::write(
        root.join(PROFILE_PICTURE_FILE_NAME),
        b"profile-picture-avatar",
    )
    .expect("profile picture");

    let resolved = find_source_avatar(root).expect("avatar path should resolve");
    assert!(
        resolved
            .to_ascii_lowercase()
            .ends_with(&PROFILE_PICTURE_FILE_NAME.to_ascii_lowercase()),
        "ProfilePicture.jpg should take priority over heuristic avatar names"
    );
}

#[test]
fn find_source_avatar_uses_scrawler_user_picture_layout() {
    let temp_dir = tempfile::tempdir().expect("temp directory");
    let root = temp_dir.path();
    let pictures_dir = root.join(PROFILE_SETTINGS_DIR_NAME).join("Pictures");
    fs::create_dir_all(&pictures_dir).expect("pictures dir");
    fs::write(pictures_dir.join("UserPicture.jpg"), b"imported-avatar").expect("user picture");

    let resolved = find_source_avatar(root).expect("avatar path should resolve");
    assert!(
        resolved.to_ascii_lowercase().ends_with("userpicture.jpg"),
        "imported SCrawler avatar (Settings/Pictures/UserPicture.jpg) should resolve, got {resolved}"
    );
}

#[test]
fn upgrade_twitter_avatar_url_strips_size_suffixes() {
    assert_eq!(
        upgrade_twitter_avatar_url("https://pbs.twimg.com/profile_images/123/avatar_normal.jpg"),
        "https://pbs.twimg.com/profile_images/123/avatar.jpg"
    );
    assert_eq!(
        upgrade_twitter_avatar_url(
            "https://pbs.twimg.com/profile_images/123/avatar_400x400.png?foo=bar"
        ),
        "https://pbs.twimg.com/profile_images/123/avatar.png?foo=bar"
    );
    // Sem sufixo de tamanho conhecido: mantém a URL original.
    assert_eq!(
        upgrade_twitter_avatar_url("https://pbs.twimg.com/profile_images/123/avatar.jpg"),
        "https://pbs.twimg.com/profile_images/123/avatar.jpg"
    );
}

#[test]
fn find_source_avatar_ignores_nested_gallery_dl_profile_picture() {
    let temp_dir = tempfile::tempdir().expect("temp directory");
    let root = temp_dir.path();

    // Simulate gallery-dl structure: instagram/{handle}/ProfilePicture.jpg
    let gallery_dl_dir = root.join("instagram").join("beeaa0_0");
    fs::create_dir_all(&gallery_dl_dir).expect("gallery-dl dir");
    fs::write(
        gallery_dl_dir.join(PROFILE_PICTURE_FILE_NAME),
        b"gallery-dl-avatar",
    )
    .expect("gallery-dl avatar");

    // No ProfilePicture.jpg at root or Settings/
    let resolved = find_source_avatar(root);
    assert!(
        resolved.is_none(),
        "should not pick up ProfilePicture.jpg from nested gallery-dl directory, got: {:?}",
        resolved
    );
}

#[test]
fn find_source_avatar_finds_root_avatar_even_with_nested_gallery_dl() {
    let temp_dir = tempfile::tempdir().expect("temp directory");
    let root = temp_dir.path();

    // Gallery-dl nested structure
    let gallery_dl_dir = root.join("instagram").join("beeaa0_0");
    fs::create_dir_all(&gallery_dl_dir).expect("gallery-dl dir");
    fs::write(
        gallery_dl_dir.join(PROFILE_PICTURE_FILE_NAME),
        b"gallery-dl-avatar",
    )
    .expect("gallery-dl avatar");

    // Root-level avatar.jpg (legacy heuristic match)
    fs::write(root.join("avatar.jpg"), b"root-avatar").expect("root avatar");

    let resolved = find_source_avatar(root).expect("avatar path should resolve");
    assert!(
        resolved.to_ascii_lowercase().ends_with("avatar.jpg"),
        "should find root-level avatar.jpg, not nested gallery-dl file, got: {:?}",
        resolved
    );
}

#[test]
fn ensure_profile_picture_at_root_promotes_nested_profile_picture() {
    let temp_dir = tempfile::tempdir().expect("temp directory");
    let root = temp_dir.path();
    let nested_root = root.join("instagram").join("lolxz_maria");
    fs::create_dir_all(&nested_root).expect("nested root");

    let nested_profile_picture = nested_root.join(PROFILE_PICTURE_FILE_NAME);
    fs::write(&nested_profile_picture, b"nested-profile-picture").expect("nested avatar");

    let promoted = match ensure_profile_picture_at_root(root, &nested_profile_picture) {
        Ok(path) => path,
        Err(error) => panic!("promote avatar failed: {}", error.message),
    };
    assert_eq!(
        promoted,
        root.join(PROFILE_SETTINGS_DIR_NAME)
            .join(PROFILE_PICTURE_FILE_NAME),
        "avatar should be normalized to Settings/ProfilePicture.jpg"
    );
    assert!(
        promoted.exists(),
        "normalized profile picture should exist in Settings"
    );
    assert_eq!(
        fs::read(&promoted).expect("promoted bytes"),
        b"nested-profile-picture",
        "normalized file should preserve avatar content"
    );
    assert!(
        !nested_profile_picture.exists(),
        "nested gallery-dl avatar should be removed after promotion"
    );
    assert!(
        !nested_root.exists(),
        "empty nested gallery-dl directories should be cleaned up after promotion"
    );
}

#[test]
fn ensure_profile_picture_at_root_syncs_root_avatar_to_settings() {
    let temp_dir = tempfile::tempdir().expect("temp directory");
    let root = temp_dir.path();
    fs::create_dir_all(root).expect("profile root");

    let root_profile_picture = root.join(PROFILE_PICTURE_FILE_NAME);
    let image = image::RgbImage::from_fn(24, 24, |x, y| {
        image::Rgb([(x * 10) as u8, (y * 10) as u8, 120])
    });
    image
        .save(&root_profile_picture)
        .expect("write valid root profile picture");

    let promoted = match ensure_profile_picture_at_root(root, &root_profile_picture) {
        Ok(path) => path,
        Err(error) => panic!("sync root avatar failed: {}", error.message),
    };

    let expected_settings_picture = root
        .join(PROFILE_SETTINGS_DIR_NAME)
        .join(PROFILE_PICTURE_FILE_NAME);
    assert_eq!(
        promoted, expected_settings_picture,
        "root avatar should resolve to Settings/ProfilePicture.jpg"
    );
    assert!(
        expected_settings_picture.exists(),
        "Settings profile picture should exist after sync"
    );
}

#[test]
fn update_instagram_source_handle_after_sync_updates_source_and_media_rows() {
    let (_temp_dir, layout) = create_test_layout();

    let (source_handle, previous_handles) =
        with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;
            upsert_source_profile_with_connection(
                connection,
                test_layout,
                sample_source("source-1", "instagram", Some("account-1")),
            )?;

            update_instagram_source_handle_after_sync(
                connection,
                "source-1",
                "new_profile",
                "2026-03-12T03:01:00Z",
            )?;

            let source_handle = connection
                .query_row(
                    "SELECT handle FROM source_profiles WHERE id = ?1",
                    params!["source-1"],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|error| error.to_string())?;
            // O rename guarda o handle antigo em previous_handles para a busca
            // continuar encontrando o perfil pelo nome anterior.
            let source = load_source_profile_by_id(connection, "source-1")?;
            let previous_handles = source
                .sync_options
                .instagram
                .and_then(|options| options.previous_handles)
                .unwrap_or_default();

            Ok((source_handle, previous_handles))
        })
        .expect("source handle should update");

    assert_eq!(source_handle, "new_profile");
    assert!(
        previous_handles
            .iter()
            .any(|handle| handle.eq_ignore_ascii_case("source-1")),
        "previous handle should be recorded, got {previous_handles:?}"
    );
}

#[test]
fn update_instagram_source_description_after_sync_populates_empty_profile_note() {
    let (_temp_dir, layout) = create_test_layout();

    let saved_description = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;
        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-1", "instagram", Some("account-1")),
        )?;

        let source = load_source_profile_by_id(connection, "source-1")?;
        update_instagram_source_description_after_sync(
            connection,
            &source,
            "Imported biography",
            false,
            "2026-03-13T03:01:00Z",
        )?;

        let raw_sync_options = connection
            .query_row(
                "SELECT sync_options_json FROM source_profiles WHERE id = ?1",
                params!["source-1"],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| error.to_string())?;
        let sync_options = deserialize_source_sync_options("instagram", &raw_sync_options);
        Ok(sync_options
            .instagram
            .and_then(|instagram| instagram.description)
            .unwrap_or_default())
    })
    .expect("description should persist");

    assert_eq!(saved_description, "Imported biography");
}

#[test]
fn update_source_profile_stats_persists_and_coalesces_partial_updates() {
    let (_temp_dir, layout) = create_test_layout();

    with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;
        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-1", "instagram", Some("account-1")),
        )?;

        // Primeira captura grava todos os campos.
        let changed = update_source_profile_stats(
            connection,
            "source-1",
            Some("Pole & flexibility"),
            Some(18_432),
            Some(642),
            Some(577),
            Some(true),
            "2026-07-17T00:00:00Z",
        )?;
        assert!(changed);

        // Segunda captura só traz bio + seguidores; o resto é preservado (COALESCE).
        update_source_profile_stats(
            connection,
            "source-1",
            None,
            Some(18_500),
            None,
            None,
            None,
            "2026-07-17T01:00:00Z",
        )?;

        let row: (
            Option<String>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<String>,
        ) = connection
            .query_row(
                "SELECT profile_biography, profile_follower_count, profile_following_count,
                        profile_media_count, profile_is_verified, profile_stats_updated_at
                 FROM source_profiles WHERE id = ?1",
                params!["source-1"],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .map_err(|error| error.to_string())?;
        assert_eq!(row.0.as_deref(), Some("Pole & flexibility"));
        assert_eq!(row.1, Some(18_500));
        assert_eq!(row.2, Some(642));
        assert_eq!(row.3, Some(577));
        assert_eq!(row.4, Some(1));
        assert_eq!(row.5.as_deref(), Some("2026-07-17T01:00:00Z"));

        // Sem nenhum dado novo é no-op (não sobrescreve o timestamp).
        let changed = update_source_profile_stats(
            connection,
            "source-1",
            None,
            None,
            None,
            None,
            None,
            "2026-07-17T02:00:00Z",
        )?;
        assert!(!changed);

        Ok(())
    })
    .expect("profile stats should persist");
}

#[test]
fn update_instagram_source_description_after_sync_preserves_existing_note_without_force() {
    let (_temp_dir, layout) = create_test_layout();

    let saved_description = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;
        let mut source = sample_source("source-1", "instagram", Some("account-1"));
        source.sync_options = SourceSyncOptions {
            instagram: Some(InstagramSourceSyncOptions {
                description: Some("Operator note".to_string()),
                ..default_instagram_source_sync_options()
            }),
            ..Default::default()
        };
        upsert_source_profile_with_connection(connection, test_layout, source)?;

        let source = load_source_profile_by_id(connection, "source-1")?;
        update_instagram_source_description_after_sync(
            connection,
            &source,
            "Imported biography",
            false,
            "2026-03-13T03:01:00Z",
        )?;

        let raw_sync_options = connection
            .query_row(
                "SELECT sync_options_json FROM source_profiles WHERE id = ?1",
                params!["source-1"],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| error.to_string())?;
        let sync_options = deserialize_source_sync_options("instagram", &raw_sync_options);
        Ok(sync_options
            .instagram
            .and_then(|instagram| instagram.description)
            .unwrap_or_default())
    })
    .expect("existing note should remain");

    assert_eq!(saved_description, "Operator note");
}

#[test]
fn update_instagram_source_description_after_sync_appends_history_with_force() {
    let (_temp_dir, layout) = create_test_layout();

    let saved_description = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;
        let mut source = sample_source("source-1", "instagram", Some("account-1"));
        source.sync_options = SourceSyncOptions {
            instagram: Some(InstagramSourceSyncOptions {
                description: Some("Operator note".to_string()),
                ..default_instagram_source_sync_options()
            }),
            ..Default::default()
        };
        upsert_source_profile_with_connection(connection, test_layout, source)?;

        let source = load_source_profile_by_id(connection, "source-1")?;
        update_instagram_source_description_after_sync(
            connection,
            &source,
            "Imported biography",
            true,
            "2026-03-13T03:01:00Z",
        )?;

        let raw_sync_options = connection
            .query_row(
                "SELECT sync_options_json FROM source_profiles WHERE id = ?1",
                params!["source-1"],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| error.to_string())?;
        let sync_options = deserialize_source_sync_options("instagram", &raw_sync_options);
        Ok(sync_options
            .instagram
            .and_then(|instagram| instagram.description)
            .unwrap_or_default())
    })
    .expect("history should append");

    assert_eq!(saved_description, "Operator note\n----\nImported biography");
}

#[test]
fn update_instagram_source_description_after_sync_avoids_duplicate_history_entries() {
    let (_temp_dir, layout) = create_test_layout();

    let saved_description = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;
        let mut source = sample_source("source-1", "instagram", Some("account-1"));
        source.sync_options = SourceSyncOptions {
            instagram: Some(InstagramSourceSyncOptions {
                description: Some("Operator note\n----\nImported biography".to_string()),
                ..default_instagram_source_sync_options()
            }),
            ..Default::default()
        };
        upsert_source_profile_with_connection(connection, test_layout, source)?;

        let source = load_source_profile_by_id(connection, "source-1")?;
        update_instagram_source_description_after_sync(
            connection,
            &source,
            "Imported biography",
            true,
            "2026-03-13T03:01:00Z",
        )?;

        let raw_sync_options = connection
            .query_row(
                "SELECT sync_options_json FROM source_profiles WHERE id = ?1",
                params!["source-1"],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| error.to_string())?;
        let sync_options = deserialize_source_sync_options("instagram", &raw_sync_options);
        Ok(sync_options
            .instagram
            .and_then(|instagram| instagram.description)
            .unwrap_or_default())
    })
    .expect("duplicate history should be avoided");

    assert_eq!(saved_description, "Operator note\n----\nImported biography");
}

#[test]
fn parse_legacy_instagram_profile_xml_reads_description() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let profile_root = temp_dir.path().join("legacy-profile");
    let user_xml_path = create_legacy_instagram_profile_root(
        &profile_root,
        "instagram-account",
        "legacy.user",
        Some("Imported biography"),
    )
    .expect("legacy profile fixture");

    let profile =
        parse_legacy_instagram_profile_xml(&user_xml_path).expect("legacy xml should parse");

    assert_eq!(profile.description.as_deref(), Some("Imported biography"));
}

#[test]
fn run_instagram_scrawler_import_populates_profile_note_from_legacy_description() {
    let (_temp_dir, layout) = create_test_layout();
    let legacy_root = layout.media_root.join("legacy-import").join("legacy.user");
    create_legacy_instagram_profile_root(
        &legacy_root,
        "instagram-account",
        "legacy.user",
        Some("Imported biography"),
    )
    .expect("legacy profile fixture");

    let saved_description = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;

        let manual_root = legacy_root.display().to_string();
        let preview = preview_instagram_scrawler_import_with_connection(
            connection,
            test_layout,
            ImportPreviewOptions {
                force_reimport: false,
                manual_roots: vec![manual_root.clone()],
                disabled_roots: Vec::new(),
            },
        )?;

        assert_eq!(
            preview.profiles.len(),
            1,
            "expected imported legacy profile"
        );

        let result = run_instagram_scrawler_import_with_connection(
            connection,
            test_layout,
            ImportRunRequest {
                force_reimport: false,
                manual_roots: vec![manual_root],
                disabled_roots: Vec::new(),
                resolutions: preview
                    .profiles
                    .iter()
                    .map(|profile| ImportResolution {
                        profile_root: profile.profile_root.clone(),
                        action: "import".to_string(),
                        account_id: profile.account_id.clone(),
                    })
                    .collect(),
            },
        )?;

        assert_eq!(result.imported_profiles, 1, "legacy profile should import");

        let source_id = result
            .profiles
            .first()
            .and_then(|profile| profile.source_id.as_deref())
            .ok_or_else(|| "imported source id missing".to_string())?;
        let source = load_source_profile_by_id(connection, source_id)?;
        Ok(source
            .sync_options
            .instagram
            .and_then(|instagram| instagram.description)
            .unwrap_or_default())
    })
    .expect("legacy import should persist note");

    assert_eq!(saved_description, "Imported biography");
}

#[test]
fn run_instagram_scrawler_import_seeds_ledgers_from_legacy_data_xml() {
    let (_temp_dir, layout) = create_test_layout();
    let legacy_root = layout.media_root.join("legacy-import").join("ledger.user");
    create_legacy_instagram_profile_root(&legacy_root, "instagram-account", "ledger.user", None)
        .expect("legacy profile fixture");
    let legacy_file_name = "471328806_18026404109545583_7067156219508743506_n.jpg";
    fs::write(legacy_root.join(legacy_file_name), b"image").expect("legacy media");
    create_legacy_instagram_data_xml(
        &legacy_root,
        legacy_file_name,
        "3528946119357054415_46332873582",
        None,
        "https://instagram.example/media.jpg",
        "https://www.instagram.com/p/DD5VcxjxT3P/",
    )
    .expect("legacy data xml");

    let (media_snapshot, post_snapshot, media_section) =
        with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;

            let manual_root = legacy_root.display().to_string();
            let preview = preview_instagram_scrawler_import_with_connection(
                connection,
                test_layout,
                ImportPreviewOptions {
                    force_reimport: false,
                    manual_roots: vec![manual_root.clone()],
                    disabled_roots: Vec::new(),
                },
            )?;
            let result = run_instagram_scrawler_import_with_connection(
                connection,
                test_layout,
                ImportRunRequest {
                    force_reimport: false,
                    manual_roots: vec![manual_root],
                    disabled_roots: Vec::new(),
                    resolutions: preview
                        .profiles
                        .iter()
                        .map(|profile| ImportResolution {
                            profile_root: profile.profile_root.clone(),
                            action: "import".to_string(),
                            account_id: profile.account_id.clone(),
                        })
                        .collect(),
                },
            )?;

            let source_id = result
                .profiles
                .first()
                .and_then(|profile| profile.source_id.as_deref())
                .ok_or_else(|| "imported source id missing".to_string())?;
            let media_snapshot =
                load_instagram_media_ledger_snapshot_for_source(connection, source_id)?;
            let post_snapshot =
                load_instagram_post_ledger_snapshot_for_source(connection, source_id)?;
            let media_section = connection
                .query_row(
                    "SELECT media_section
                     FROM instagram_sync_post_ledger
                     WHERE source_id = ?1
                     LIMIT 1",
                    params![source_id],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|error| error.to_string())?;
            Ok((media_snapshot, post_snapshot, media_section))
        })
        .expect("legacy import should seed ledgers");

    assert!(
        media_snapshot
            .media_keys
            .contains("471328806_18026404109545583_7067156219508743506_n"),
        "expected media ledger to include the legacy file stem"
    );
    assert!(
        post_snapshot
            .keys
            .contains("3528946119357054415_46332873582"),
        "expected post ledger to include the legacy post id"
    );
    assert!(
        post_snapshot.keys.contains("dd5vcxjxt3p"),
        "expected post ledger to include the permalink code"
    );
    assert_eq!(media_section, "timeline");
}

#[test]
fn import_backfill_recategorizes_legacy_reels_mislabeled_as_timeline() {
    use base64::Engine as _;

    let (_temp_dir, layout) = create_test_layout();
    let legacy_root = layout.media_root.join("legacy-import").join("reel.user");
    create_legacy_instagram_profile_root(&legacy_root, "instagram-account", "reel.user", None)
        .expect("legacy profile fixture");
    let legacy_file_name = "AQreelclipabcdefghijklmnopqrstuvwxyz0123456789.mp4";
    fs::write(legacy_root.join(legacy_file_name), b"video").expect("legacy media");

    // CDN URL with the `xpv_encode_tag` INSTAGRAM.CLIPS embedded in the `efg`
    // (base64), the way SCrawler stores reels. The permalink uses `/p/` (not
    // `/reel/`), reproducing the case that wrongly fell into `timeline`.
    let payload =
        "{\"xpv_encode_tag\":\"xpv_progressive.INSTAGRAM.CLIPS.C3.720.dash_baseline_1_v1\"}";
    let efg = base64::engine::general_purpose::STANDARD.encode(payload.as_bytes());
    // In the XML the query-string `&` come escaped as `&amp;` (roxmltree
    // unescapes them back to `&` when reading the attribute, as in production).
    let media_url = format!("https://cdn.example/AQreel.mp4?_nc_cat=1&amp;efg={efg}&amp;ccb=1");

    create_legacy_instagram_data_xml(
        &legacy_root,
        legacy_file_name,
        "3827569610392183305_46332873582",
        None,
        &media_url,
        "https://www.instagram.com/p/DUeQkgEgaIJ/",
    )
    .expect("legacy data xml");

    let (media_section, post_section) =
        with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;

            let manual_root = legacy_root.display().to_string();
            let preview = preview_instagram_scrawler_import_with_connection(
                connection,
                test_layout,
                ImportPreviewOptions {
                    force_reimport: false,
                    manual_roots: vec![manual_root.clone()],
                    disabled_roots: Vec::new(),
                },
            )?;
            let result = run_instagram_scrawler_import_with_connection(
                connection,
                test_layout,
                ImportRunRequest {
                    force_reimport: false,
                    manual_roots: vec![manual_root],
                    disabled_roots: Vec::new(),
                    resolutions: preview
                        .profiles
                        .iter()
                        .map(|profile| ImportResolution {
                            profile_root: profile.profile_root.clone(),
                            action: "import".to_string(),
                            account_id: profile.account_id.clone(),
                        })
                        .collect(),
                },
            )?;
            let source_id = result
                .profiles
                .first()
                .and_then(|profile| profile.source_id.as_deref())
                .ok_or_else(|| "imported source id missing".to_string())?
                .to_string();

            // Simulate the WRONG pre-fix state: the reel stored as `timeline`
            // in both ledgers (legacy import via a `/p/` permalink).
            connection
                .execute(
                    "UPDATE instagram_sync_media_ledger SET media_section = 'timeline' WHERE source_id = ?1",
                    params![source_id],
                )
                .map_err(|error| error.to_string())?;
            connection
                .execute(
                    "UPDATE instagram_sync_post_ledger SET media_section = 'timeline' WHERE source_id = ?1",
                    params![source_id],
                )
                .map_err(|error| error.to_string())?;

            // The backfill must recategorize timeline -> reels via the URL signal.
            let mut noop = |_progress: InstagramNamingLedgerBackfillProgress| {};
            run_instagram_media_naming_ledger_backfill_with_connection(
                connection,
                test_layout,
                &mut noop,
            )?;

            let media_section = connection
                .query_row(
                    "SELECT media_section FROM instagram_sync_media_ledger WHERE source_id = ?1 LIMIT 1",
                    params![source_id],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|error| error.to_string())?;
            let post_section = connection
                .query_row(
                    "SELECT media_section FROM instagram_sync_post_ledger WHERE source_id = ?1 LIMIT 1",
                    params![source_id],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|error| error.to_string())?;
            Ok((media_section, post_section))
        })
        .expect("backfill should recategorize the mislabeled reel");

    assert_eq!(
        media_section, "reels",
        "media ledger should be recategorized"
    );
    assert_eq!(post_section, "reels", "post ledger should be recategorized");
}

#[test]
fn run_instagram_scrawler_import_seeds_media_aliases_from_legacy_url() {
    let (_temp_dir, layout) = create_test_layout();
    let legacy_root = layout.media_root.join("legacy-import").join("alias.user");
    create_legacy_instagram_profile_root(&legacy_root, "instagram-account", "alias.user", None)
        .expect("legacy profile fixture");
    let legacy_file_name = "471328806_18026404109545583_7067156219508743506_n.jpg";
    fs::write(legacy_root.join(legacy_file_name), b"image").expect("legacy media");
    create_legacy_instagram_data_xml(
        &legacy_root,
        legacy_file_name,
        "3528946119357054415_46332873582",
        None,
        "https://instagram.example/media/API_ALIAS_01.jpg?stp=dst-jpg_e35",
        "https://www.instagram.com/p/DD5VcxjxT3P/",
    )
    .expect("legacy data xml");

    let (alias_snapshot, hashed_alias_count) =
        with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;

            let manual_root = legacy_root.display().to_string();
            let preview = preview_instagram_scrawler_import_with_connection(
                connection,
                test_layout,
                ImportPreviewOptions {
                    force_reimport: false,
                    manual_roots: vec![manual_root.clone()],
                    disabled_roots: Vec::new(),
                },
            )?;
            let result = run_instagram_scrawler_import_with_connection(
                connection,
                test_layout,
                ImportRunRequest {
                    force_reimport: false,
                    manual_roots: vec![manual_root],
                    disabled_roots: Vec::new(),
                    resolutions: preview
                        .profiles
                        .iter()
                        .map(|profile| ImportResolution {
                            profile_root: profile.profile_root.clone(),
                            action: "import".to_string(),
                            account_id: profile.account_id.clone(),
                        })
                        .collect(),
                },
            )?;

            let source_id = result
                .profiles
                .first()
                .and_then(|profile| profile.source_id.as_deref())
                .ok_or_else(|| "imported source id missing".to_string())?;
            let alias_snapshot =
                load_instagram_media_alias_snapshot_for_source(connection, source_id)?;
            let hashed_alias_count = connection
                .query_row(
                    "SELECT COUNT(*)
                         FROM instagram_media_key_aliases
                         WHERE source_id = ?1
                           AND file_sha256 IS NOT NULL
                           AND file_sha256 <> ''",
                    params![source_id],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| error.to_string())?;
            Ok((alias_snapshot, hashed_alias_count))
        })
        .expect("legacy import should seed aliases");

    assert!(
        alias_snapshot.keys.contains("api_alias_01"),
        "expected media alias snapshot to include the basename from the legacy media URL"
    );
    assert!(
        alias_snapshot
            .keys
            .contains("3528946119357054415_46332873582"),
        "expected media alias snapshot to include the legacy post id"
    );
    assert!(
        alias_snapshot.keys.contains("dd5vcxjxt3p"),
        "expected media alias snapshot to include the legacy post code"
    );
    assert!(
        hashed_alias_count > 0,
        "expected imported aliases to persist a file SHA256 fingerprint"
    );
}

#[test]
fn scrawler_import_prefers_true_name_over_user_name_for_handle() {
    let (_temp_dir, layout) = create_test_layout();
    let legacy_root = layout.media_root.join("legacy-import").join("_franjudaaa_");
    create_legacy_instagram_profile_root_full(
        &legacy_root,
        "instagram-account",
        "_franjudaaa_",
        Some("franjuda"),
        Some("17443084061"),
        None,
    )
    .expect("legacy profile fixture");

    let (handle, user_id_hint) = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;

        let manual_root = legacy_root.display().to_string();
        let preview = preview_instagram_scrawler_import_with_connection(
            connection,
            test_layout,
            ImportPreviewOptions {
                force_reimport: false,
                manual_roots: vec![manual_root.clone()],
                disabled_roots: Vec::new(),
            },
        )?;

        assert_eq!(
            preview.profiles.len(),
            1,
            "expected one legacy profile in preview; first roots: {:?}",
            preview
                .profiles
                .iter()
                .take(3)
                .map(|profile| profile.profile_root.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            preview.profiles[0].handle, "franjuda",
            "preview handle should use TrueName"
        );

        let result = run_instagram_scrawler_import_with_connection(
            connection,
            test_layout,
            ImportRunRequest {
                force_reimport: false,
                manual_roots: vec![manual_root],
                disabled_roots: Vec::new(),
                resolutions: preview
                    .profiles
                    .iter()
                    .map(|profile| ImportResolution {
                        profile_root: profile.profile_root.clone(),
                        action: "import".to_string(),
                        account_id: profile.account_id.clone(),
                    })
                    .collect(),
            },
        )?;

        let source_id = result
            .profiles
            .first()
            .and_then(|profile| profile.source_id.as_deref())
            .ok_or_else(|| "imported source id missing".to_string())?;
        let source = load_source_profile_by_id(connection, source_id)?;
        let hint = source
            .sync_options
            .instagram
            .and_then(|instagram| instagram.user_id_hint);
        Ok((source.handle, hint))
    })
    .expect("legacy import should succeed");

    assert_eq!(handle, "franjuda", "imported handle should use TrueName");
    assert_eq!(
        user_id_hint.as_deref(),
        Some("17443084061"),
        "imported sync options should store UserID as user_id_hint"
    );
}

#[test]
fn scrawler_import_falls_back_to_user_name_when_true_name_is_empty() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let profile_root = temp_dir.path().join("legacy-profile");
    let user_xml_path = create_legacy_instagram_profile_root_full(
        &profile_root,
        "instagram-account",
        "original_handle",
        Some(""),
        None,
        None,
    )
    .expect("legacy profile fixture");

    let profile =
        parse_legacy_instagram_profile_xml(&user_xml_path).expect("legacy xml should parse");
    let handle = legacy_instagram_profile_handle(&profile, "folder_name");

    assert_eq!(
        handle, "original_handle",
        "handle should fall back to UserName when TrueName is empty"
    );
}

#[test]
fn parse_retry_after_duration_supports_seconds() {
    let value = reqwest::header::HeaderValue::from_static("120");
    let parsed = parse_retry_after_duration(Some(&value)).expect("retry-after seconds");
    assert_eq!(parsed.as_secs(), 120);
}

#[test]
fn parse_retry_after_duration_supports_http_date() {
    let future = (Utc::now() + Duration::seconds(90)).to_rfc2822();
    let value = reqwest::header::HeaderValue::from_str(&future).expect("header value");
    let parsed = parse_retry_after_duration(Some(&value)).expect("retry-after date");
    assert!(
        parsed.as_secs() >= 1,
        "retry-after date parsing should return a positive delay"
    );
}

#[test]
fn classify_instagram_identity_error_detects_private_or_restricted() {
    let error = "Instagram request 'https://www.instagram.com/api/v1/feed/user/demo/username/?count=30' returned 403: {\"message\":\"login required for private account\"}";
    assert_eq!(
        classify_instagram_identity_error(error),
        InstagramIdentityErrorClassification::PrivateOrRestricted
    );
}

#[test]
fn classify_instagram_identity_error_detects_unresolvable_profiles() {
    let error = "Instagram request 'https://www.instagram.com/api/v1/feed/user/missing-user/username/?count=30' returned 404: Not Found";
    assert_eq!(
        classify_instagram_identity_error(error),
        InstagramIdentityErrorClassification::UsernameUnresolvable
    );
}

#[test]
fn classify_instagram_identity_error_uses_probe_marker_for_private_profiles() {
    let error = "Instagram timeline response is missing user data. [identity_probe=instagram_profile_private_or_restricted] Profile accessibility probe confirmed `web_profile_info.data.user.is_private=true`.";
    assert_eq!(
        classify_instagram_identity_error(error),
        InstagramIdentityErrorClassification::PrivateOrRestricted
    );
}

#[test]
fn classify_instagram_identity_error_uses_probe_marker_for_unresolvable_profiles() {
    let error = "Instagram timeline response is missing user data. [identity_probe=instagram_username_unresolvable] Profile accessibility probe returned no user object.";
    assert_eq!(
        classify_instagram_identity_error(error),
        InstagramIdentityErrorClassification::UsernameUnresolvable
    );
}

#[test]
fn availability_rate_limit_abort_ignores_inconclusive_probe_429() {
    let error = "Instagram timeline response is missing user data. Profile accessibility probe returned 429 Too Many Requests.";
    assert!(
        !instagram_error_indicates_availability_abort_rate_limit(error),
        "inconclusive probe 429 should not abort the full availability batch"
    );
}

#[test]
fn availability_rate_limit_abort_keeps_explicit_endpoint_429() {
    let error =
        "Instagram request 'https://www.instagram.com/api/v1/feed/user/demo/username/?count=30' returned 429: Too Many Requests";
    assert!(
        instagram_error_indicates_availability_abort_rate_limit(error),
        "explicit endpoint 429 should still abort the availability batch"
    );
}

#[test]
fn decide_instagram_availability_action_keeps_private_marker_even_when_hint_fallback_resolves() {
    let previous = "demo_user";
    let primary = Err("Instagram timeline response is missing user data. [identity_probe=instagram_profile_private_or_restricted] Profile accessibility probe confirmed `web_profile_info.data.user.is_private=true`.".to_string());
    let fallback = Ok(instagram_connector::InstagramProfileIdentity {
        username: "demo_user".to_string(),
        user_id: "123".to_string(),
    });

    assert_eq!(
        decide_instagram_availability_action(previous, &primary, Some(&fallback)),
        InstagramAvailabilityAction::MarkPrivateOrRestricted {
            resolved_handle: Some("demo_user".to_string()),
            handle_changed: false
        }
    );
}

#[test]
fn decide_instagram_availability_action_clears_unresolvable_when_hint_fallback_resolves_username() {
    let previous = "old_name";
    let primary = Err("Instagram request 'https://www.instagram.com/api/v1/feed/user/old_name/username/?count=30' returned 404: Not Found".to_string());
    let fallback = Ok(instagram_connector::InstagramProfileIdentity {
        username: "new_name".to_string(),
        user_id: "999".to_string(),
    });

    assert_eq!(
        decide_instagram_availability_action(previous, &primary, Some(&fallback)),
        InstagramAvailabilityAction::Resolved {
            resolved_handle: "new_name".to_string(),
            handle_changed: true
        }
    );
}

#[test]
fn decide_instagram_availability_action_prefers_anchored_identity_after_handle_reuse() {
    let primary = Ok(instagram_connector::InstagramProfileIdentity {
        username: "old_name".to_string(),
        user_id: "new-owner-id".to_string(),
    });
    let fallback = Ok(instagram_connector::InstagramProfileIdentity {
        username: "renamed_original".to_string(),
        user_id: "stable-owner-id".to_string(),
    });

    assert_eq!(
        decide_instagram_availability_action("old_name", &primary, Some(&fallback)),
        InstagramAvailabilityAction::Resolved {
            resolved_handle: "renamed_original".to_string(),
            handle_changed: true
        }
    );
}

#[test]
fn decide_instagram_availability_action_rejects_reused_handle_when_anchor_lookup_fails() {
    let primary = Ok(instagram_connector::InstagramProfileIdentity {
        username: "old_name".to_string(),
        user_id: "new-owner-id".to_string(),
    });
    let fallback = Err("stable identity lookup failed".to_string());

    assert!(matches!(
        decide_instagram_availability_action("old_name", &primary, Some(&fallback)),
        InstagramAvailabilityAction::Failed(message)
            if message.contains("different Instagram account")
    ));
}

#[test]
fn set_source_sync_problem_can_preserve_ready_for_download_state() {
    let (_temp_dir, layout) = create_test_layout();

    let (ready_for_download, sync_problem_code) =
        with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;
            let source = sample_source("source-1", "instagram", Some("account-1"));
            upsert_source_profile_with_connection(connection, test_layout, source)?;
            set_source_sync_problem(
                connection,
                "source-1",
                "instagram_profile_private_or_restricted",
                "private profile",
                "2026-03-20T00:00:00Z",
                false,
            )?;

            let result = connection
                .query_row(
                    "SELECT ready_for_download, sync_problem_code
                         FROM source_profiles
                         WHERE id = ?1",
                    params!["source-1"],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
                )
                .map_err(|error| error.to_string())?;
            Ok(result)
        })
        .expect("non-blocking sync problem should persist");

    assert_eq!(
        ready_for_download, 1,
        "non-blocking profile marker must not pause source readiness"
    );
    assert_eq!(
        sync_problem_code.as_deref(),
        Some("instagram_profile_private_or_restricted")
    );
}

#[test]
fn clear_source_sync_problem_restores_ready_for_download() {
    let (_temp_dir, layout) = create_test_layout();

    let (ready_for_download, sync_problem_code) =
        with_workspace_layout(layout, |connection, test_layout| {
            upsert_provider_account_with_connection(
                connection,
                test_layout,
                sample_account("account-1", "instagram"),
            )?;
            let source = sample_source("source-1", "instagram", Some("account-1"));
            upsert_source_profile_with_connection(connection, test_layout, source)?;

            // Mark a blocking sync problem (disables ready_for_download).
            set_source_sync_problem(
                connection,
                "source-1",
                "instagram_username_unresolvable",
                "profile unavailable",
                "2026-03-20T00:00:00Z",
                true,
            )?;

            // Verify ready_for_download is now 0.
            let before: i64 = connection
                .query_row(
                    "SELECT ready_for_download FROM source_profiles WHERE id = ?1",
                    params!["source-1"],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            assert_eq!(
                before, 0,
                "blocking problem should disable ready_for_download"
            );

            // Clear the problem — should restore ready_for_download.
            clear_source_sync_problem(connection, "source-1", "2026-03-20T01:00:00Z")?;

            let result = connection
                .query_row(
                    "SELECT ready_for_download, sync_problem_code
                         FROM source_profiles
                         WHERE id = ?1",
                    params!["source-1"],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
                )
                .map_err(|error| error.to_string())?;
            Ok(result)
        })
        .expect("clear sync problem should succeed");

    assert_eq!(
        ready_for_download, 1,
        "clearing sync problem must restore ready_for_download"
    );
    assert_eq!(sync_problem_code, None, "sync problem code must be cleared");
}

#[test]
fn running_sync_plan_now_persists_runtime_history_and_notification() {
    let (_temp_dir, layout) = create_test_layout();

    let (snapshot, source_ids) = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;
        seed_instagram_session(connection, test_layout, "account-1")?;
        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-1", "instagram", Some("account-1")),
        )?;
        upsert_scheduler_set_with_connection(connection, sample_scheduler_set("set-1", true))?;
        upsert_sync_plan_with_connection(
            connection,
            sample_sync_plan("plan-1", "set-1", "automatic", 30, 0),
        )?;

        let source_ids = run_sync_plan_now_with_connection(
            connection,
            test_layout,
            "plan-1",
            "manual",
            "2026-03-10T00:15:00Z",
        )?;
        Ok((load_snapshot(connection, test_layout)?, source_ids))
    })
    .expect("manual sync-plan run should succeed");

    // O plano resolve as fontes e devolve os ids a enfileirar; não roda
    // o sync inline.
    assert_eq!(source_ids, vec!["source-1".to_string()]);
    assert_eq!(
        snapshot.sync_plan_runs.len(),
        1,
        "expected persisted plan run history"
    );
    assert_eq!(snapshot.sync_plan_runs[0].status, "succeeded");
    assert_eq!(snapshot.sync_plan_runs[0].source_count, 1);
    assert_eq!(
        snapshot.scheduler_sets[0].plans[0].last_run_status,
        "succeeded"
    );
    assert!(
        snapshot.scheduler_sets[0].plans[0]
            .last_run_summary
            .as_deref()
            .is_some_and(|value| value.contains("Queued 1 source syncs")),
        "expected last run summary to report the queued count"
    );
}

#[test]
fn running_sync_plan_now_queues_sources_even_when_in_cooldown() {
    // O plano só resolve e enfileira; o skip por cooldown da conta acontece
    // depois, quando o worker da fila executa a fonte (não mais inline).
    let (_temp_dir, layout) = create_test_layout();

    let (snapshot, source_ids) = with_workspace_layout(layout, |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;
        seed_instagram_session(connection, test_layout, "account-1")?;
        save_provider_account_settings_with_connection(
            connection,
            test_layout,
            "account-1".to_string(),
            vec![ProviderAccountSettingValue {
                setting_key: INSTAGRAM_SYNC_COOLDOWN_UNTIL_SETTING_KEY.to_string(),
                value_kind: ProviderAccountSettingValueKind::String,
                string_value: Some("2030-01-01T00:00:00Z".to_string()),
                json_value: None,
            }],
        )?;
        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-1", "instagram", Some("account-1")),
        )?;
        upsert_scheduler_set_with_connection(connection, sample_scheduler_set("set-1", true))?;
        upsert_sync_plan_with_connection(
            connection,
            sample_sync_plan("plan-1", "set-1", "automatic", 30, 0),
        )?;

        let source_ids = run_sync_plan_now_with_connection(
            connection,
            test_layout,
            "plan-1",
            "manual",
            "2026-03-10T00:15:00Z",
        )?;
        Ok((load_snapshot(connection, test_layout)?, source_ids))
    })
    .expect("manual sync-plan run should queue gracefully");

    // A fonte é resolvida/enfileirada (nada roda inline), então não há run
    // de sync ainda; o registro do plano marca quantas foram enfileiradas.
    assert_eq!(source_ids, vec!["source-1".to_string()]);
    assert_eq!(snapshot.source_sync_runs.len(), 0);
    assert_eq!(snapshot.sync_plan_runs.len(), 1);
    assert_eq!(snapshot.sync_plan_runs[0].status, "succeeded");
    assert!(
        snapshot.scheduler_sets[0].plans[0]
            .last_run_summary
            .as_deref()
            .is_some_and(|value| value.contains("Queued 1 source syncs")),
        "expected the sync-plan summary to report the queued count"
    );
}

#[test]
fn scheduler_tick_respects_startup_delay_across_restarts() {
    let (_temp_dir, layout) = create_test_layout();

    with_workspace_layout(layout.clone(), |connection, test_layout| {
        upsert_provider_account_with_connection(
            connection,
            test_layout,
            sample_account("account-1", "instagram"),
        )?;
        seed_instagram_session(connection, test_layout, "account-1")?;
        upsert_source_profile_with_connection(
            connection,
            test_layout,
            sample_source("source-1", "instagram", Some("account-1")),
        )?;
        upsert_scheduler_set_with_connection(connection, sample_scheduler_set("set-1", true))?;
        upsert_sync_plan_with_connection(
            connection,
            sample_sync_plan("plan-1", "set-1", "automatic", 30, 30),
        )?;
        record_scheduler_launch_with_connection(connection, "2026-03-10T00:00:00Z")
    })
    .expect("seed scheduler state");

    let before_due = with_workspace_layout(layout.clone(), |connection, test_layout| {
        process_scheduler_tick_with_connection(connection, test_layout, "2026-03-10T00:10:00Z")?;
        load_snapshot(connection, test_layout)
    })
    .expect("tick before startup delay");

    assert_eq!(
        before_due.sync_plan_runs.len(),
        0,
        "plan should not run before startup delay"
    );
    assert_eq!(
        before_due.scheduler_sets[0].plans[0].next_due_at.as_deref(),
        Some("2026-03-10T00:30:00+00:00")
    );

    let after_due = with_workspace_layout(layout, |connection, test_layout| {
        process_scheduler_tick_with_connection(connection, test_layout, "2026-03-10T00:31:00Z")?;
        load_snapshot(connection, test_layout)
    })
    .expect("tick after restart should still honor persisted launch state");

    assert_eq!(
        after_due.sync_plan_runs.len(),
        1,
        "plan should run once after startup delay"
    );
    assert_eq!(after_due.sync_plan_runs[0].status, "succeeded");
    assert_eq!(
        after_due.scheduler_sets[0].plans[0].last_run_status,
        "succeeded"
    );
}

#[test]
fn pause_resume_and_skip_sync_plan_update_runtime_state() {
    let (_temp_dir, layout) = create_test_layout();

    let snapshot = with_workspace_layout(layout, |connection, test_layout| {
        upsert_scheduler_set_with_connection(connection, sample_scheduler_set("set-1", true))?;
        upsert_sync_plan_with_connection(
            connection,
            sample_sync_plan("plan-1", "set-1", "automatic", 30, 0),
        )?;
        record_scheduler_launch_with_connection(connection, "2026-03-10T00:00:00Z")?;

        set_sync_plan_pause_with_connection(
            connection,
            &SetSyncPlanPauseInput {
                id: "plan-1".to_string(),
                pause_mode: "indefinite".to_string(),
                pause_until: None,
            },
            "2026-03-10T00:01:00Z",
        )?;
        clear_sync_plan_pause_with_connection(connection, "plan-1", "2026-03-10T00:05:00Z")?;
        skip_sync_plan_with_connection(
            connection,
            &SkipSyncPlanInput {
                id: "plan-1".to_string(),
                mode: "next".to_string(),
                minutes: None,
                until: None,
            },
            "2026-03-10T00:05:00Z",
        )?;
        load_snapshot(connection, test_layout)
    })
    .expect("pause/resume/skip should succeed");

    let plan = &snapshot.scheduler_sets[0].plans[0];
    assert!(!plan.paused, "plan should be resumed");
    assert_eq!(plan.last_run_status, "skipped");
    assert!(
        plan.skip_until.is_some(),
        "skip should persist a skip-until timestamp"
    );
    assert!(
        plan.last_run_summary
            .as_deref()
            .is_some_and(|summary| summary.contains("Skipped")),
        "skip should still leave an operator-visible runtime summary"
    );
}
