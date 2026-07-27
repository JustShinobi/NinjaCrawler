//! One-shot migration of a 4K Stogram library into the NinjaCrawler workspace.
//!
//! 4K Stogram was discontinued; this reads its `.stogram.sqlite` catalog, copies
//! the media into the canonical `<media_root>/instagram/<handle>` layout and
//! seeds the Instagram ledgers so the gallery resolves sections, captions and
//! post links exactly like natively synced media.
//!
//! This is deliberately NOT wired into the import UI (`list_import_methods`):
//! it runs once, from `stogram_migration_cli`, and is meant to be deleted
//! afterwards.
//!
//! Source of truth is the catalog, never the directory tree — every media row
//! carries its section, permalink, caption and capture time, which a filesystem
//! walk cannot recover. The per-profile `.thumb.stogram/` directories are
//! derived thumbnails and are never touched.

use super::*;

use rusqlite::OpenFlags;

/// Marks rows the 4K Stogram downloader completed. Anything else (5, 6, 7) is a
/// failed or pending download whose file is absent or truncated.
const STOGRAM_STATE_DOWNLOADED: i64 = 4;
const STOGRAM_IMPORTER_ID: &str = "instagram.stogram";
const STOGRAM_CATALOG_FILE_NAME: &str = ".stogram.sqlite";

#[derive(Clone)]
pub struct StogramMigrationOptions {
    pub stogram_root: PathBuf,
    /// Provider account bound to profiles created from scratch. Accepts the
    /// account id or its display name. Existing profiles keep their own.
    pub account: Option<String>,
    /// Restrict the run to these 4K Stogram handles (empty = every profile).
    pub handles: Vec<String>,
    pub limit: Option<usize>,
    pub dry_run: bool,
}

#[derive(Clone, Default)]
pub struct StogramMigrationReport {
    pub dry_run: bool,
    pub profiles_total: u32,
    pub profiles_created: u32,
    pub profiles_merged: u32,
    pub profiles_failed: u32,
    pub media_copied: u32,
    pub media_already_cataloged: u32,
    /// Files found at the destination but missing from the ledgers (leftovers of
    /// an interrupted run) that this run re-registered.
    pub media_recovered: u32,
    /// Thumbnail-sized placeholders skipped (see `is_degraded_stogram_media`).
    pub media_skipped_degraded: u32,
    pub media_missing_files: u32,
    pub avatars_promoted: u32,
    pub avatars_archived: u32,
    /// Migrated highlights whose real album was recovered from the workspace;
    /// the rest go to the `Legacy` album.
    pub highlight_albums_matched: u32,
    pub bytes_copied: u64,
    pub profiles: Vec<StogramProfileOutcome>,
}

#[derive(Clone)]
pub struct StogramProfileOutcome {
    pub stogram_handle: String,
    pub user_id: String,
    /// `created`, `merged` or `failed`.
    pub status: String,
    /// How the existing profile was found: `user_id`, `handle` or `none`.
    pub matched_by: String,
    pub source_id: Option<String>,
    pub source_handle: Option<String>,
    pub profile_root: Option<String>,
    pub media_copied: u32,
    pub media_already_cataloged: u32,
    pub media_recovered: u32,
    pub media_skipped_degraded: u32,
    pub media_missing_files: u32,
    pub avatars_promoted: u32,
    pub avatars_archived: u32,
    pub highlight_albums_matched: u32,
    pub bytes_copied: u64,
    pub message: String,
}

/// A `subscriptions` row plus its downloaded `photos`.
struct StogramProfile {
    handle: String,
    user_id: String,
    display_name: String,
    download_timeline: bool,
    download_reels: bool,
    download_stories: bool,
    download_highlights: bool,
    download_tagged: bool,
    media: Vec<StogramMedia>,
}

struct StogramMedia {
    /// `<media_pk>_<owner_id>` — the same namespace as
    /// `instagram_sync_post_ledger.provider_post_key`.
    instagram_id: String,
    web_url: String,
    media_url: String,
    title: Option<String>,
    created_time: i64,
    /// Catalog-relative path, e.g. `handle\reels\2024-01-01 10.00.00 <id>.mp4`.
    file: String,
    is_video: i64,
}

/// What a `photos.is_video` bitfield says the row actually is.
pub(super) enum StogramMediaKind {
    /// Gallery media, carrying the NinjaCrawler section name.
    Section(&'static str),
    /// Profile picture (`created_time` is 0, hence the 1969-12-31 file name).
    Avatar,
}

/// `is_video` is a bitfield, not a boolean: bit 0 flags video, the upper bits
/// carry the section. Observed values are 0/2/3 (feed), 4/5 (story), 16/17
/// (highlight), 65 (reel) and 8 (profile picture). This is strictly better than
/// the SCrawler importer's URL heuristic, which has to guess reels from the CDN
/// `xpv_encode_tag`.
pub(super) fn classify_stogram_media(is_video: i64) -> StogramMediaKind {
    if is_video == 8 {
        return StogramMediaKind::Avatar;
    }
    if is_video & 64 != 0 {
        return StogramMediaKind::Section("reels");
    }
    if is_video & 16 != 0 {
        // 4K Stogram stores highlights flat, with no album name, so they land in
        // `stories/` without an `instagram_highlight_media_membership` row.
        return StogramMediaKind::Section("stories");
    }
    if is_video & 4 != 0 {
        return StogramMediaKind::Section("stories_user");
    }
    StogramMediaKind::Section("timeline")
}

/// Sub-directory each section lives in, relative to the profile root. Mirrors
/// the layout the SCrawler import and the native sync already produce.
///
/// Highlights are special: the gallery reads the album from the SECOND path
/// segment (`stories/<album>/file`), so a highlight dropped straight into
/// `stories/` makes the gallery treat the file name itself as an album. The
/// album is therefore part of the directory and resolved per media.
fn section_relative_dir(section: &str) -> &'static str {
    match section {
        "reels" => "video",
        "stories" => "stories",
        "stories_user" => "stories (user)",
        "tagged" => "tagged",
        _ => "",
    }
}

/// Anything smaller than this that also breaks the `<date> <pk>_<owner>` naming
/// is a degraded placeholder, not real media (see
/// [`is_degraded_stogram_media`]). Real media never comes close: across 4.570
/// correctly named files in the library, none is under 20 KB.
pub(super) const STOGRAM_DEGRADED_MAX_BYTES: u64 = 20 * 1024;

/// `true` for catalog rows whose file is a thumbnail-sized placeholder rather
/// than the real media.
///
/// 4K Stogram left 362 such rows, all highlights, all flagged `state = 4` as if
/// the download had succeeded: they average 4 KB against 2.6 MB for real
/// highlights, and their name is the raw CDN one instead of the usual
/// `<date> <pk>_<owner>`. Both signals must agree before skipping, so a legit
/// small file is never dropped.
fn is_degraded_stogram_media(source_path: &Path, file_size: u64) -> bool {
    if file_size >= STOGRAM_DEGRADED_MAX_BYTES {
        return false;
    }
    source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .is_some_and(|stem| strip_instagram_timestamp_prefix(stem).is_none())
}

pub fn run_stogram_migration(
    options: StogramMigrationOptions,
    on_progress: &mut dyn FnMut(&StogramProfileOutcome),
) -> Result<StogramMigrationReport, String> {
    with_workspace(|connection, layout| {
        run_stogram_migration_with_connection(connection, layout, &options, on_progress)
    })
}

pub(super) fn run_stogram_migration_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    options: &StogramMigrationOptions,
    on_progress: &mut dyn FnMut(&StogramProfileOutcome),
) -> Result<StogramMigrationReport, String> {
    let catalog_path = options.stogram_root.join(STOGRAM_CATALOG_FILE_NAME);
    if !catalog_path.exists() {
        return Err(format!(
            "No 4K Stogram catalog at '{}'.",
            catalog_path.display()
        ));
    }

    let profiles = load_stogram_profiles(&catalog_path, &options.handles, options.limit)?;
    let fallback_account_id = resolve_migration_account_id(connection, options.account.as_deref())?;

    let mut report = StogramMigrationReport {
        dry_run: options.dry_run,
        profiles_total: profiles.len() as u32,
        ..StogramMigrationReport::default()
    };

    for profile in &profiles {
        let outcome = match migrate_stogram_profile(
            connection,
            layout,
            options,
            profile,
            fallback_account_id.as_deref(),
        ) {
            Ok(outcome) => outcome,
            Err(error) => StogramProfileOutcome {
                stogram_handle: profile.handle.clone(),
                user_id: profile.user_id.clone(),
                status: "failed".to_string(),
                matched_by: "none".to_string(),
                source_id: None,
                source_handle: None,
                profile_root: None,
                media_copied: 0,
                media_already_cataloged: 0,
                media_recovered: 0,
                media_skipped_degraded: 0,
                media_missing_files: 0,
                avatars_promoted: 0,
                avatars_archived: 0,
                highlight_albums_matched: 0,
                bytes_copied: 0,
                message: error,
            },
        };

        match outcome.status.as_str() {
            "created" => report.profiles_created += 1,
            "merged" => report.profiles_merged += 1,
            _ => report.profiles_failed += 1,
        }
        report.media_copied += outcome.media_copied;
        report.media_already_cataloged += outcome.media_already_cataloged;
        report.media_recovered += outcome.media_recovered;
        report.media_skipped_degraded += outcome.media_skipped_degraded;
        report.highlight_albums_matched += outcome.highlight_albums_matched;
        report.media_missing_files += outcome.media_missing_files;
        report.avatars_promoted += outcome.avatars_promoted;
        report.avatars_archived += outcome.avatars_archived;
        report.bytes_copied += outcome.bytes_copied;

        on_progress(&outcome);
        report.profiles.push(outcome);
    }

    Ok(report)
}

/// Reads the catalog read-only — the 4K Stogram library is treated as an
/// immutable backup and is never written to.
fn load_stogram_profiles(
    catalog_path: &Path,
    handle_filter: &[String],
    limit: Option<usize>,
) -> Result<Vec<StogramProfile>, String> {
    let connection = Connection::open_with_flags(
        catalog_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|error| format!("Failed to open '{}': {error}", catalog_path.display()))?;

    let wanted = handle_filter
        .iter()
        .map(|handle| sanitize_source_handle("instagram", handle).to_ascii_lowercase())
        .filter(|handle| !handle.is_empty())
        .collect::<HashSet<_>>();

    let mut statement = connection
        .prepare(
            "SELECT id, query, instagram_id, display_name,
                    downloadFeed, downloadReels, downloadStories,
                    downloadHighlights, downloadTaggedPosts
             FROM subscriptions
             ORDER BY query",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                row.get::<_, Option<i64>>(5)?.unwrap_or(0),
                row.get::<_, Option<i64>>(6)?.unwrap_or(0),
                row.get::<_, Option<i64>>(7)?.unwrap_or(0),
                row.get::<_, Option<i64>>(8)?.unwrap_or(0),
            ))
        })
        .map_err(|error| error.to_string())?;

    let mut profiles = Vec::new();
    for row in rows {
        let (id, query, user_id, display_name, feed, reels, stories, highlights, tagged) =
            row.map_err(|error| error.to_string())?;
        let handle = query.trim().to_string();
        if handle.is_empty() {
            continue;
        }
        if !wanted.is_empty()
            && !wanted.contains(&sanitize_source_handle("instagram", &handle).to_ascii_lowercase())
        {
            continue;
        }

        let media = load_stogram_media(&connection, &id)?;
        if media.is_empty() {
            continue;
        }

        profiles.push(StogramProfile {
            handle,
            user_id: user_id.trim().to_string(),
            display_name: display_name.trim().to_string(),
            download_timeline: feed != 0,
            download_reels: reels != 0,
            download_stories: stories != 0,
            download_highlights: highlights != 0,
            download_tagged: tagged != 0,
            media,
        });

        if limit.is_some_and(|limit| profiles.len() >= limit) {
            break;
        }
    }

    Ok(profiles)
}

fn load_stogram_media(
    connection: &Connection,
    subscription_id: &[u8],
) -> Result<Vec<StogramMedia>, String> {
    let mut statement = connection
        .prepare(
            "SELECT instagram_id, web_url, media_url, title, created_time, file, is_video
             FROM photos
             WHERE subscriptionId = ?1 AND state = ?2 AND file IS NOT NULL AND file <> ''
             ORDER BY created_time, id",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![subscription_id, STOGRAM_STATE_DOWNLOADED], |row| {
            Ok(StogramMedia {
                instagram_id: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                web_url: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                media_url: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                title: row
                    .get::<_, Option<String>>(3)?
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
                created_time: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                file: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                is_video: row.get::<_, Option<i64>>(6)?.unwrap_or(0),
            })
        })
        .map_err(|error| error.to_string())?;

    let mut media = Vec::new();
    for row in rows {
        let entry = row.map_err(|error| error.to_string())?;
        if entry.file.trim().is_empty() {
            continue;
        }
        media.push(entry);
    }
    Ok(media)
}

fn resolve_migration_account_id(
    connection: &Connection,
    account: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(account) = account.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    let accounts = load_accounts(connection)?
        .into_iter()
        .filter(|entry| entry.provider.eq_ignore_ascii_case("instagram"))
        .collect::<Vec<_>>();
    let matched = accounts
        .iter()
        .find(|entry| entry.id == account)
        .or_else(|| {
            accounts
                .iter()
                .find(|entry| entry.display_name.eq_ignore_ascii_case(account))
        })
        .ok_or_else(|| format!("No Instagram account matched '{account}'."))?;
    Ok(Some(matched.id.clone()))
}

/// How an existing NinjaCrawler profile was matched, if at all.
enum SourceMatch {
    /// `userIdHint` equals the 4K Stogram user id — the reliable case.
    ByUserId(SourceProfile),
    /// Handles match but the ids could not confirm it, either because the
    /// NinjaCrawler profile has no `userIdHint` or because it holds a different
    /// one (an account that was deleted and recreated). Still a merge, but the
    /// stored id is left exactly as it is.
    ByHandle(SourceProfile),
    None,
}

/// Runs `operation` inside a short `BEGIN IMMEDIATE` and commits, rolling back
/// on error. Deliberately narrow: only database work belongs in here, never file
/// copying, or the write lock is held for minutes.
fn in_transaction<T>(
    connection: &Connection,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    connection
        .execute("BEGIN IMMEDIATE TRANSACTION", [])
        .map_err(|error| error.to_string())?;
    match operation() {
        Ok(value) => {
            connection
                .execute("COMMIT", [])
                .map_err(|error| error.to_string())?;
            Ok(value)
        }
        Err(error) => {
            let _ = connection.execute("ROLLBACK", []);
            Err(error)
        }
    }
}

/// Album given to migrated highlights that cannot be traced to a real one.
const STOGRAM_LEGACY_HIGHLIGHT_ALBUM: &str = "Legacy";

/// Album a migrated highlight belongs to.
///
/// 4K Stogram stores highlights flat, with no album name, so the real album can
/// only come from the workspace: if the same media was already downloaded by a
/// highlight sync, `instagram_highlight_media_membership` names it. The two
/// systems key media differently (4K Stogram by media pk, the native sync by CDN
/// file name), hence trying every identity the catalog row yields. Untraceable
/// highlights go to `Legacy`.
fn resolve_highlight_album(
    known_albums: &HashMap<String, BTreeSet<String>>,
    media: &StogramMedia,
) -> String {
    if known_albums.is_empty() {
        return STOGRAM_LEGACY_HIGHLIGHT_ALBUM.to_string();
    }
    for candidate in identity_candidates(media) {
        if let Some(albums) = known_albums.get(&candidate) {
            if let Some(album) = albums.iter().next() {
                return sanitize_album_directory_name(album);
            }
        }
    }
    STOGRAM_LEGACY_HIGHLIGHT_ALBUM.to_string()
}

/// Album names come from Instagram and may carry characters Windows rejects in
/// a directory name (`\ / : * ? " < > |`). Emoji and accents are kept — the
/// existing albums on disk already use them.
fn sanitize_album_directory_name(album: &str) -> String {
    let sanitized = album
        .trim()
        .chars()
        .map(|character| match character {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect::<String>();
    let sanitized = sanitized.trim_end_matches([' ', '.']).to_string();
    if sanitized.is_empty() {
        STOGRAM_LEGACY_HIGHLIGHT_ALBUM.to_string()
    } else {
        sanitized
    }
}

/// Records the album membership of every migrated highlight, mirroring the album
/// directory the file was copied into (`stories/<album>/…`, decided by
/// [`resolve_highlight_album`]). The gallery already derives the album from that
/// path; the membership rows keep the association explicit and survive a later
/// move of the file. Returns how many were traced to a real album rather than
/// falling back to `Legacy`.
fn assign_stogram_highlight_albums(
    connection: &Connection,
    source_id: &str,
    records: &[LegacyInstagramReconciliationRecord],
    profile_root: &Path,
    timestamp: &str,
) -> Result<u32, String> {
    let mut memberships = Vec::new();
    let mut matched = 0u32;
    for record in records
        .iter()
        .filter(|record| record.media_section == "stories")
    {
        // The album is the file's own parent directory (`stories/<album>/file`),
        // read straight from the path so the original casing and emoji survive —
        // the gallery derives it the same way, and a lowercased copy here would
        // surface as a second, duplicate album.
        let Some(album) = record
            .file_path
            .parent()
            .filter(|parent| parent != &profile_root.join("stories"))
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
        else {
            continue;
        };
        if !album.eq_ignore_ascii_case(STOGRAM_LEGACY_HIGHLIGHT_ALBUM) {
            matched += 1;
        }
        memberships.push(instagram_connector::InstagramHighlightMembership {
            provider_media_key: record.provider_media_key.clone(),
            album: album.to_string(),
        });
    }

    if memberships.is_empty() {
        return Ok(0);
    }
    upsert_instagram_highlight_memberships(connection, source_id, &memberships, timestamp)?;
    Ok(matched)
}

fn match_existing_source(
    sources: &[SourceProfile],
    profile: &StogramProfile,
) -> Result<SourceMatch, String> {
    if !profile.user_id.is_empty() {
        let by_user_id = sources.iter().find(|source| {
            source.provider.eq_ignore_ascii_case("instagram")
                && source_instagram_sync_options(source)
                    .user_id_hint
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|hint| hint == profile.user_id)
        });
        if let Some(source) = by_user_id {
            return Ok(SourceMatch::ByUserId(source.clone()));
        }
    }

    let handle = sanitize_source_handle("instagram", &profile.handle);
    let by_handle = sources.iter().find(|source| {
        source.provider.eq_ignore_ascii_case("instagram")
            && sanitize_source_handle("instagram", &source.handle).eq_ignore_ascii_case(&handle)
    });
    if let Some(source) = by_handle {
        return Ok(SourceMatch::ByHandle(source.clone()));
    }

    Ok(SourceMatch::None)
}

fn migrate_stogram_profile(
    connection: &Connection,
    layout: &StorageLayout,
    options: &StogramMigrationOptions,
    profile: &StogramProfile,
    fallback_account_id: Option<&str>,
) -> Result<StogramProfileOutcome, String> {
    let sources = load_sources(connection)?;
    let matched = match_existing_source(&sources, profile)?;
    let (existing_source, matched_by) = match matched {
        SourceMatch::ByUserId(source) => (Some(source), "user_id"),
        SourceMatch::ByHandle(source) => (Some(source), "handle"),
        SourceMatch::None => (None, "none"),
    };

    if options.dry_run {
        return dry_run_outcome(
            connection,
            layout,
            options,
            profile,
            existing_source.as_ref(),
            matched_by,
            fallback_account_id,
        );
    }

    // Three phases, so the SQLite write lock is only ever held for the database
    // work. Copying gigabytes inside `BEGIN IMMEDIATE` blocks every other writer
    // — including the app's own startup DDL, which then dies on SQLITE_BUSY.
    //
    // Phase 1 (short transaction): resolve or create the source.
    let created = existing_source.is_none();
    let source = match existing_source {
        Some(source) => {
            // A rebrand: keep the current NinjaCrawler handle and record the 4K
            // Stogram one as a previous handle so search still finds it. The
            // stored `userIdHint` is never touched — where the two catalogs
            // disagree the workspace is authoritative, since the 4K Stogram
            // database has been frozen since it was discontinued.
            in_transaction(connection, || {
                register_previous_handle_if_needed(connection, &source, &profile.handle)
            })?;
            source
        }
        None => {
            let account_id = fallback_account_id.ok_or_else(|| {
                format!(
                    "Profile '{}' is new and needs an Instagram account (pass --account).",
                    profile.handle
                )
            })?;
            in_transaction(connection, || {
                create_source_for_stogram_profile(connection, layout, profile, account_id)
            })?
        }
    };

    let account_id = source
        .account_id
        .clone()
        .ok_or_else(|| format!("Source '{}' has no provider account bound.", source.handle))?;
    let account_settings = load_provider_account_settings_map(connection, &account_id)?;
    let source_options = source_instagram_sync_options(&source);
    let profile_root = resolve_instagram_profile_root_with_options(
        layout,
        &source,
        Some(&account_settings),
        Some(&source_options),
    );
    fs::create_dir_all(&profile_root)
        .map_err(|error| format!("Failed to create '{}': {error}", profile_root.display()))?;

    // Phase 2 (no transaction): the long part — hashing and copying files.
    let staged = stage_stogram_media(connection, options, profile, &source, &profile_root)?;
    let avatars = stage_stogram_avatars(
        connection,
        layout,
        options,
        profile,
        &source,
        &profile_root,
        created,
    )?;

    // Phase 3 (short transaction): everything the ledgers need, in one commit.
    let timestamp = now_timestamp();
    let (reconciliation, albums_matched) = in_transaction(connection, || {
        let reconciliation = reconcile_instagram_ledgers_from_records(
            connection,
            &staged.records,
            &profile_root,
            &source.id,
            &account_id,
            &source.handle,
            &timestamp,
        )?;
        let albums_matched =
            assign_stogram_highlight_albums(
                connection,
                &source.id,
                &staged.records,
                &profile_root,
                &timestamp,
            )?;
        record_external_import_ledger(
            connection,
            ExternalImportLedgerRecord {
                importer_id: STOGRAM_IMPORTER_ID,
                profile_root: &profile_root,
                provider: "instagram",
                handle: &source.handle,
                source_id: &source.id,
                account_id: &account_id,
                timestamp: &timestamp,
            },
        )?;
        Ok((reconciliation, albums_matched))
    })?;

    Ok(StogramProfileOutcome {
        stogram_handle: profile.handle.clone(),
        user_id: profile.user_id.clone(),
        status: if created { "created" } else { "merged" }.to_string(),
        matched_by: matched_by.to_string(),
        source_id: Some(source.id.clone()),
        source_handle: Some(source.handle.clone()),
        profile_root: Some(profile_root.display().to_string()),
        media_copied: staged.copied,
        media_already_cataloged: staged.already_cataloged,
        media_recovered: staged.already_on_disk,
        media_skipped_degraded: staged.skipped_degraded,
        media_missing_files: staged.missing_files,
        avatars_promoted: avatars.promoted,
        avatars_archived: avatars.archived,
        highlight_albums_matched: albums_matched,
        bytes_copied: staged.bytes_copied,
        message: format!(
            "{} copied, {} already cataloged, {} recovered from disk, {} missing; \
             reconciled {} media and {} post entrie(s); {} highlight album(s) matched.",
            staged.copied,
            staged.already_cataloged,
            staged.already_on_disk,
            staged.missing_files,
            reconciliation.seeded_media_entries,
            reconciliation.seeded_post_entries,
            albums_matched
        ),
    })
}

fn register_previous_handle_if_needed(
    connection: &Connection,
    source: &SourceProfile,
    stogram_handle: &str,
) -> Result<(), String> {
    let mut instagram = source_instagram_sync_options(source);
    let updated = push_previous_instagram_handle(
        instagram.previous_handles.take(),
        stogram_handle,
        &source.handle,
    );
    let unchanged = updated
        .as_ref()
        .map(|list| list.len())
        .unwrap_or_default()
        == source
            .sync_options
            .instagram
            .as_ref()
            .and_then(|options| options.previous_handles.as_ref())
            .map(|list| list.len())
            .unwrap_or_default();
    instagram.previous_handles = updated;
    if unchanged {
        return Ok(());
    }

    let options = SourceSyncOptions {
        instagram: Some(instagram),
        ..source.sync_options.clone()
    };
    let serialized = serialize_source_sync_options("instagram", &options)?;
    connection
        .execute(
            "UPDATE source_profiles SET sync_options_json = ?2, updated_at = ?3
             WHERE id = ?1 AND deleted_at IS NULL",
            params![source.id, serialized, now_timestamp()],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn create_source_for_stogram_profile(
    connection: &Connection,
    layout: &StorageLayout,
    profile: &StogramProfile,
    account_id: &str,
) -> Result<SourceProfile, String> {
    let handle = sanitize_source_handle("instagram", &profile.handle);
    let display_name = if profile.display_name.is_empty() {
        handle.clone()
    } else {
        profile.display_name.clone()
    };
    let source_id = new_id();

    let mut instagram = default_instagram_source_sync_options();
    instagram.timeline = profile.download_timeline;
    instagram.reels = profile.download_reels;
    // 4K Stogram splits "stories" (ephemeral) from "highlights"; NinjaCrawler
    // calls the latter `stories` and the former `stories_user`.
    instagram.stories = profile.download_highlights;
    instagram.stories_user = profile.download_stories;
    instagram.tagged = profile.download_tagged;
    instagram.special_path = Some(
        layout
            .media_root
            .join("instagram")
            .join(&handle)
            .display()
            .to_string(),
    );
    instagram.user_id_hint = Some(profile.user_id.clone()).filter(|value| !value.is_empty());

    upsert_source_profile_with_connection(
        connection,
        layout,
        SourceProfileUpsert {
            id: Some(source_id.clone()),
            provider: "instagram".to_string(),
            source_kind: "profile".to_string(),
            handle,
            display_name,
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
    )?;

    load_sources(connection)?
        .into_iter()
        .find(|entry| entry.id == source_id)
        .ok_or_else(|| "The migrated source was not persisted.".to_string())
}

#[derive(Default)]
struct StagedMedia {
    records: Vec<LegacyInstagramReconciliationRecord>,
    copied: u32,
    /// Known to the ledgers already — skipped entirely.
    already_cataloged: u32,
    /// Present at the destination but absent from the ledgers (leftover of an
    /// interrupted run): not copied again, but re-registered.
    already_on_disk: u32,
    missing_files: u32,
    /// Thumbnail-sized placeholders the 4K Stogram downloader left behind while
    /// still flagging them as complete.
    skipped_degraded: u32,
    bytes_copied: u64,
}

/// Copies the catalog's media into the profile root, skipping anything the
/// workspace already holds, and builds the reconciliation records for what
/// ends up on disk.
fn stage_stogram_media(
    connection: &Connection,
    options: &StogramMigrationOptions,
    profile: &StogramProfile,
    source: &SourceProfile,
    profile_root: &Path,
) -> Result<StagedMedia, String> {
    let known_keys = load_known_media_keys(connection, &source.id)?;
    let known_hashes = load_known_media_hashes(connection, &source.id)?;
    // Albums already known for this profile, used to file each migrated
    // highlight under its real album instead of `Legacy`.
    let known_albums = load_instagram_highlight_membership(connection, &source.id);
    let mut staged = StagedMedia::default();

    for media in &profile.media {
        let section = match classify_stogram_media(media.is_video) {
            StogramMediaKind::Section(section) => section,
            StogramMediaKind::Avatar => continue,
        };

        let source_path = options.stogram_root.join(media.file.replace('\\', "/"));
        let Ok(metadata) = fs::metadata(&source_path) else {
            staged.missing_files += 1;
            continue;
        };
        if !metadata.is_file() {
            staged.missing_files += 1;
            continue;
        }
        if is_degraded_stogram_media(&source_path, metadata.len()) {
            staged.skipped_degraded += 1;
            continue;
        }

        // Cheap identity check first: the 4K Stogram media pk and, for photos,
        // the CDN file name embedded in `media_url` — which is exactly the key
        // the native sync and the SCrawler import store.
        if identity_candidates(media)
            .iter()
            .any(|key| known_keys.contains(key))
        {
            staged.already_cataloged += 1;
            continue;
        }

        // Videos carry no CDN key (`..._video_dashinit.mp4`), so content hashing
        // is the only way to recognise media already downloaded under a
        // different name. The hash is needed for the fingerprint ledger anyway.
        let file_sha256 = compute_file_sha256(&source_path)?;
        if known_hashes.contains(&file_sha256) {
            staged.already_cataloged += 1;
            continue;
        }

        let Some(file_name) = source_path.file_name().and_then(|value| value.to_str()) else {
            staged.missing_files += 1;
            continue;
        };
        let target_dir = {
            let relative = section_relative_dir(section);
            let base = if relative.is_empty() {
                profile_root.to_path_buf()
            } else {
                profile_root.join(relative)
            };
            // Highlights need the album as a directory, or the gallery reads the
            // file name as the album name.
            if section == "stories" {
                base.join(resolve_highlight_album(&known_albums, media))
            } else {
                base
            }
        };
        let (target_path, already_there) = resolve_copy_target(&target_dir, file_name, &file_sha256)?;
        if already_there {
            // The identical file is already at the destination but was not in
            // the ledgers — an interrupted run copied it and never committed.
            // It still needs its record, otherwise it stays invisible to the
            // gallery forever.
            staged.already_on_disk += 1;
        } else {
            let bytes = copy_media_file(&source_path, &target_path)?;
            staged.copied += 1;
            staged.bytes_copied += bytes;
        }

        // Decoding the image for its perceptual hashes is the single most
        // expensive step, so it happens here — outside any transaction.
        let image_fingerprint = compute_instagram_media_fingerprint(&target_path);
        let Some(record) =
            build_reconciliation_record(media, &target_path, section, file_sha256, image_fingerprint)
        else {
            continue;
        };
        staged.records.push(record);
    }

    Ok(staged)
}

/// Every key under which this media may already be cataloged: the 4K Stogram
/// media pk and whatever identity the CDN URL yields.
fn identity_candidates(media: &StogramMedia) -> Vec<String> {
    let mut candidates = Vec::new();
    let pk = media.instagram_id.trim().to_ascii_lowercase();
    if !pk.is_empty() {
        candidates.push(pk);
    }
    if let Some(raw_file_name) = media.media_url.split('?').next() {
        for candidate in extract_instagram_media_identity_candidates_from_file_name(raw_file_name) {
            candidates.push(candidate);
        }
    }
    candidates
}

/// Destination for a file, plus whether an identical file is already sitting
/// there (in which case the copy is skipped but the record is still built, so an
/// interrupted run is recoverable).
///
/// Name collisions across the two systems are unlikely (4K Stogram names by
/// media pk, the SCrawler import by CDN key), so a differing file gets a
/// suffix rather than overwriting anything.
fn resolve_copy_target(
    target_dir: &Path,
    file_name: &str,
    file_sha256: &str,
) -> Result<(PathBuf, bool), String> {
    let direct = target_dir.join(file_name);
    if !direct.exists() {
        return Ok((direct, false));
    }
    if compute_file_sha256(&direct).is_ok_and(|existing| existing == file_sha256) {
        return Ok((direct, true));
    }

    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(file_name);
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    for suffix in 2..100u32 {
        let candidate = if extension.is_empty() {
            target_dir.join(format!("{stem} ({suffix})"))
        } else {
            target_dir.join(format!("{stem} ({suffix}).{extension}"))
        };
        if !candidate.exists() {
            return Ok((candidate, false));
        }
        if compute_file_sha256(&candidate).is_ok_and(|existing| existing == file_sha256) {
            return Ok((candidate, true));
        }
    }
    Err(format!(
        "Could not find a free name for '{file_name}' in '{}'.",
        target_dir.display()
    ))
}

/// Copies and verifies the size. The 4K Stogram library is never modified, so a
/// failure here only leaves a partial file in the destination, which the
/// enclosing transaction rollback plus a re-run will replace.
fn copy_media_file(source_path: &Path, target_path: &Path) -> Result<u64, String> {
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create '{}': {error}", parent.display()))?;
    }
    let copied = fs::copy(source_path, target_path).map_err(|error| {
        format!(
            "Failed to copy '{}' to '{}': {error}",
            source_path.display(),
            target_path.display()
        )
    })?;
    let expected = fs::metadata(source_path)
        .map(|metadata| metadata.len())
        .map_err(|error| error.to_string())?;
    if copied != expected {
        let _ = fs::remove_file(target_path);
        return Err(format!(
            "Short copy of '{}': {copied} of {expected} byte(s).",
            source_path.display()
        ));
    }
    Ok(copied)
}

fn build_reconciliation_record(
    media: &StogramMedia,
    target_path: &Path,
    section: &str,
    file_sha256: String,
    image_fingerprint: Option<ImageFingerprint>,
) -> Option<LegacyInstagramReconciliationRecord> {
    let provider_media_key = derive_instagram_media_identity_key_from_path(target_path)?;
    let media_type = infer_media_type(target_path)?;
    let provider_post_key = normalize_instagram_post_ledger_key(&media.instagram_id);
    let provider_post_code = extract_instagram_post_code_from_permalink(&media.web_url);
    let provider_post_code_cased = extract_instagram_post_code_from_permalink_cased(&media.web_url);

    // Same alias set the SCrawler import seeds, so later syncs recognise this
    // media whichever identity they derive: the file name, the CDN URL, the post
    // id or the shortcode.
    let mut alias_keys = vec![(provider_media_key.clone(), "legacy_file_path".to_string())];
    if let Some(raw_file_name) = media.media_url.split('?').next() {
        for candidate in extract_instagram_media_identity_candidates_from_file_name(raw_file_name) {
            alias_keys.push((candidate, "legacy_media_url".to_string()));
        }
    }
    if !provider_post_key.is_empty() {
        alias_keys.push((provider_post_key.clone(), "legacy_post_id".to_string()));
    }
    if let Some(post_code) = provider_post_code.clone() {
        alias_keys.push((post_code, "legacy_post_code".to_string()));
    }

    Some(LegacyInstagramReconciliationRecord {
        file_path: target_path.to_path_buf(),
        legacy_file_name: media.file.clone(),
        provider_media_key,
        alias_keys,
        file_sha256: Some(file_sha256),
        provider_post_key,
        provider_post_code,
        provider_post_code_cased,
        media_type: media_type.to_string(),
        media_section: section.to_string(),
        title: media.title.clone(),
        captured_at_timestamp: Some(media.created_time).filter(|value| *value > 0),
        image_fingerprint,
    })
}

fn load_known_media_keys(
    connection: &Connection,
    source_id: &str,
) -> Result<HashSet<String>, String> {
    ensure_instagram_sync_media_ledger_table(connection)?;
    ensure_instagram_media_key_aliases_table(connection)?;

    let mut keys = HashSet::new();
    for (sql, label) in [
        (
            "SELECT provider_media_key FROM instagram_sync_media_ledger WHERE source_id = ?1",
            "media ledger",
        ),
        (
            "SELECT alias_key FROM instagram_media_key_aliases WHERE source_id = ?1",
            "media aliases",
        ),
    ] {
        let mut statement = connection
            .prepare(sql)
            .map_err(|error| format!("Failed to read {label}: {error}"))?;
        let rows = statement
            .query_map(params![source_id], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        for row in rows.flatten() {
            keys.insert(row.trim().to_ascii_lowercase());
        }
    }
    Ok(keys)
}

fn load_known_media_hashes(
    connection: &Connection,
    source_id: &str,
) -> Result<HashSet<String>, String> {
    ensure_instagram_media_fingerprints_table(connection)?;
    ensure_instagram_media_key_aliases_table(connection)?;

    let mut hashes = HashSet::new();
    for sql in [
        "SELECT file_sha256 FROM instagram_media_fingerprints
         WHERE source_id = ?1 AND file_sha256 IS NOT NULL",
        "SELECT file_sha256 FROM instagram_media_key_aliases
         WHERE source_id = ?1 AND file_sha256 IS NOT NULL",
    ] {
        let mut statement = connection.prepare(sql).map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![source_id], |row| row.get::<_, Option<String>>(0))
            .map_err(|error| error.to_string())?;
        for row in rows.flatten().flatten() {
            hashes.insert(row);
        }
    }
    Ok(hashes)
}

#[derive(Default)]
struct StagedAvatars {
    promoted: u32,
    archived: u32,
}

/// Profile pictures live in `Settings/`: the newest becomes the canonical
/// `ProfilePicture.jpg` for a brand-new profile, and everything else is archived
/// as `ProfilePicture_<date>.jpg`. A merged profile keeps whatever avatar the
/// workspace already shows — the 4K Stogram copy is always older.
fn stage_stogram_avatars(
    connection: &Connection,
    layout: &StorageLayout,
    options: &StogramMigrationOptions,
    profile: &StogramProfile,
    source: &SourceProfile,
    profile_root: &Path,
    created: bool,
) -> Result<StagedAvatars, String> {
    let mut avatars = profile
        .media
        .iter()
        .filter(|media| matches!(classify_stogram_media(media.is_video), StogramMediaKind::Avatar))
        .filter_map(|media| {
            let path = options.stogram_root.join(media.file.replace('\\', "/"));
            let modified = fs::metadata(&path).ok()?.modified().ok()?;
            path.is_file().then_some((path, modified))
        })
        .collect::<Vec<_>>();
    if avatars.is_empty() {
        return Ok(StagedAvatars::default());
    }
    avatars.sort_by_key(|(_, modified)| *modified);

    let settings_dir = profile_root.join(PROFILE_SETTINGS_DIR_NAME);
    fs::create_dir_all(&settings_dir)
        .map_err(|error| format!("Failed to create '{}': {error}", settings_dir.display()))?;

    let mut staged = StagedAvatars::default();
    // Only a brand-new profile promotes an avatar; the newest one is last after
    // the sort.
    let promote = created && !source.profile_image_custom;
    let promoted = if promote { avatars.pop() } else { None };

    for (path, modified) in &avatars {
        let date = DateTime::<Local>::from(*modified)
            .format("%Y-%m-%d")
            .to_string();
        let target = archived_profile_picture_path(&settings_dir, &date);
        copy_media_file(path, &target)?;
        staged.archived += 1;
    }

    if let Some((path, _)) = promoted {
        let target = settings_profile_picture_path(profile_root);
        copy_media_file(&path, &target)?;
        let normalized = normalize_media_file_path(&target)?;
        update_source_profile_image(connection, layout, &source.id, &normalized, &now_timestamp())?;
        staged.promoted += 1;
    }

    Ok(staged)
}

/// Walks the same decisions as a real run but touches nothing, so the counts can
/// be reviewed before committing 68 GB of copies.
fn dry_run_outcome(
    connection: &Connection,
    layout: &StorageLayout,
    options: &StogramMigrationOptions,
    profile: &StogramProfile,
    existing_source: Option<&SourceProfile>,
    matched_by: &str,
    fallback_account_id: Option<&str>,
) -> Result<StogramProfileOutcome, String> {
    let (profile_root, known_keys) = match existing_source {
        Some(source) => {
            let account_id = source.account_id.clone().unwrap_or_default();
            let account_settings = load_provider_account_settings_map(connection, &account_id)?;
            let source_options = source_instagram_sync_options(source);
            (
                resolve_instagram_profile_root_with_options(
                    layout,
                    source,
                    Some(&account_settings),
                    Some(&source_options),
                ),
                load_known_media_keys(connection, &source.id)?,
            )
        }
        None => {
            if fallback_account_id.is_none() {
                return Err(format!(
                    "Profile '{}' is new and needs an Instagram account (pass --account).",
                    profile.handle
                ));
            }
            let handle = sanitize_source_handle("instagram", &profile.handle);
            (
                layout.media_root.join("instagram").join(&handle),
                HashSet::new(),
            )
        }
    };

    let mut copied = 0u32;
    let mut already_cataloged = 0u32;
    let mut missing_files = 0u32;
    let mut skipped_degraded = 0u32;
    let mut avatars = 0u32;
    for media in &profile.media {
        match classify_stogram_media(media.is_video) {
            StogramMediaKind::Section(_) => {}
            StogramMediaKind::Avatar => {
                avatars += 1;
                continue;
            }
        }

        let source_path = options.stogram_root.join(media.file.replace('\\', "/"));
        let Ok(metadata) = fs::metadata(&source_path) else {
            missing_files += 1;
            continue;
        };
        if !metadata.is_file() {
            missing_files += 1;
            continue;
        }
        if is_degraded_stogram_media(&source_path, metadata.len()) {
            skipped_degraded += 1;
            continue;
        }
        // Only the cheap key check runs here: hashing 68 GB for a preview is not
        // worth it, so the real run may report a higher `already_cataloged` once
        // content hashes are compared.
        if identity_candidates(media)
            .iter()
            .any(|key| known_keys.contains(key))
        {
            already_cataloged += 1;
        } else {
            copied += 1;
        }
    }

    let created = existing_source.is_none();
    Ok(StogramProfileOutcome {
        stogram_handle: profile.handle.clone(),
        user_id: profile.user_id.clone(),
        status: if created { "created" } else { "merged" }.to_string(),
        matched_by: matched_by.to_string(),
        source_id: existing_source.map(|source| source.id.clone()),
        source_handle: existing_source.map(|source| source.handle.clone()),
        profile_root: Some(profile_root.display().to_string()),
        media_copied: copied,
        media_already_cataloged: already_cataloged,
        media_recovered: 0,
        media_skipped_degraded: skipped_degraded,
        media_missing_files: missing_files,
        // A new profile promotes the newest picture; a merge archives them all.
        avatars_promoted: if created && avatars > 0 { 1 } else { 0 },
        avatars_archived: if created { avatars.saturating_sub(1) } else { avatars },
        highlight_albums_matched: 0,
        bytes_copied: 0,
        message: format!(
            "dry-run: would copy {copied} file(s), {already_cataloged} already cataloged, \
             {skipped_degraded} degraded skipped, {missing_files} missing, {avatars} avatar(s)."
        ),
    })
}
