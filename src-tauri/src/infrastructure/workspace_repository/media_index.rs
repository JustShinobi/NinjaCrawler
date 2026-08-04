use super::*;

/// Absolute-path shape shared with `media_dedupe_catalog`: backslash separators,
/// lowercased on Windows. The two catalogs must agree byte for byte — the
/// fingerprint inheritance below joins on this column, and a mismatched shape
/// would match nothing while looking perfectly healthy.
pub(crate) fn normalize_absolute_media_path(path: &Path) -> String {
    let value = path.to_string_lossy().replace('/', "\\");
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value
    }
}

/// One media file being registered in the index. `relative_path` is the ledger
/// shape (forward slashes, lowercased, relative to the profile root) so index
/// rows can be paired with `provider_sync_media_ledger` entries.
pub(crate) struct MediaIndexEntry<'a> {
    pub(crate) relative_path: &'a str,
    pub(crate) absolute_path: &'a Path,
    pub(crate) media_type: &'a str,
    pub(crate) media_section: &'a str,
    pub(crate) provider_media_key: Option<&'a str>,
    pub(crate) provider_post_key: Option<&'a str>,
    pub(crate) captured_at: Option<i64>,
}

/// Size and mtime of the file as it is right now, or `None` when the path is
/// gone. Callers treat a missing file as "index what we know, leave the
/// fingerprint pending" instead of failing the surrounding sync.
fn file_stat(path: &Path) -> Option<(i64, i64)> {
    let metadata = fs::metadata(path).ok()?;
    let size = metadata.len().min(i64::MAX as u64) as i64;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0);
    Some((size, modified))
}

/// Registers (or refreshes) a single media file in the canonical index.
///
/// The row keeps its `id` across updates — that is the whole point of the
/// table, since collections and variant groups reference it. When the bytes on
/// disk changed (size or mtime differ), previously computed fingerprints
/// describe a file that no longer exists, so they are dropped and the row goes
/// back to `pending`.
pub(crate) fn upsert_media_index_entry(
    connection: &Connection,
    provider: &str,
    source_id: &str,
    entry: &MediaIndexEntry<'_>,
    timestamp: &str,
) -> Result<(), String> {
    let (size_bytes, modified_at_ms) = file_stat(entry.absolute_path).unwrap_or((0, 0));
    let downloaded_at = Utc::now().timestamp();
    connection
        .execute(
            "INSERT INTO media_index (
                id, provider, source_id, relative_path, normalized_path,
                media_type, media_section, provider_media_key, provider_post_key,
                captured_at, downloaded_at, size_bytes, modified_at_ms,
                fingerprint_status, indexed_at, updated_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 'pending', ?14, ?14)
             ON CONFLICT(source_id, relative_path) DO UPDATE SET
                provider = excluded.provider,
                normalized_path = excluded.normalized_path,
                media_type = excluded.media_type,
                media_section = excluded.media_section,
                provider_media_key =
                    COALESCE(excluded.provider_media_key, media_index.provider_media_key),
                provider_post_key =
                    COALESCE(excluded.provider_post_key, media_index.provider_post_key),
                captured_at = COALESCE(excluded.captured_at, media_index.captured_at),
                downloaded_at = COALESCE(media_index.downloaded_at, excluded.downloaded_at),
                size_bytes = excluded.size_bytes,
                modified_at_ms = excluded.modified_at_ms,
                local_state = 'present',
                sha256 = CASE WHEN media_index.size_bytes = excluded.size_bytes
                               AND media_index.modified_at_ms = excluded.modified_at_ms
                              THEN media_index.sha256 ELSE NULL END,
                ahash64 = CASE WHEN media_index.size_bytes = excluded.size_bytes
                                AND media_index.modified_at_ms = excluded.modified_at_ms
                               THEN media_index.ahash64 ELSE NULL END,
                dhash64 = CASE WHEN media_index.size_bytes = excluded.size_bytes
                                AND media_index.modified_at_ms = excluded.modified_at_ms
                               THEN media_index.dhash64 ELSE NULL END,
                video_signature = CASE WHEN media_index.size_bytes = excluded.size_bytes
                                        AND media_index.modified_at_ms = excluded.modified_at_ms
                                       THEN media_index.video_signature ELSE NULL END,
                variant_group_id = CASE WHEN media_index.size_bytes = excluded.size_bytes
                                         AND media_index.modified_at_ms = excluded.modified_at_ms
                                        THEN media_index.variant_group_id ELSE NULL END,
                is_canonical = CASE WHEN media_index.size_bytes = excluded.size_bytes
                                     AND media_index.modified_at_ms = excluded.modified_at_ms
                                    THEN media_index.is_canonical ELSE 1 END,
                fingerprint_status = CASE WHEN media_index.size_bytes = excluded.size_bytes
                                           AND media_index.modified_at_ms = excluded.modified_at_ms
                                          THEN media_index.fingerprint_status ELSE 'pending' END,
                updated_at = excluded.updated_at",
            params![
                Uuid::new_v4().to_string(),
                provider,
                source_id,
                entry.relative_path,
                normalize_absolute_media_path(entry.absolute_path),
                entry.media_type.to_ascii_lowercase(),
                entry.media_section,
                entry.provider_media_key,
                entry.provider_post_key,
                entry.captured_at,
                downloaded_at,
                size_bytes,
                modified_at_ms,
                timestamp,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Copies fingerprints the dedupe catalog already computed into pending index
/// rows. A full library would otherwise pay for hashing twice — the dedupe scan
/// has usually already read every byte of it.
///
/// Only rows whose size and mtime still match the catalog entry are inherited;
/// a hash taken from a different revision of the file would be worse than no
/// hash at all.
pub(crate) fn inherit_media_index_fingerprints(
    connection: &Connection,
    source_id: Option<&str>,
    timestamp: &str,
) -> Result<usize, String> {
    let updated = connection
        .execute(
            "UPDATE media_index
             SET sha256 = (SELECT catalog.sha256 FROM media_dedupe_catalog catalog
                            WHERE catalog.normalized_path = media_index.normalized_path),
                 ahash64 = (SELECT catalog.ahash64 FROM media_dedupe_catalog catalog
                             WHERE catalog.normalized_path = media_index.normalized_path),
                 dhash64 = (SELECT catalog.dhash64 FROM media_dedupe_catalog catalog
                             WHERE catalog.normalized_path = media_index.normalized_path),
                 width = COALESCE((SELECT catalog.width FROM media_dedupe_catalog catalog
                                    WHERE catalog.normalized_path = media_index.normalized_path),
                                  media_index.width),
                 height = COALESCE((SELECT catalog.height FROM media_dedupe_catalog catalog
                                     WHERE catalog.normalized_path = media_index.normalized_path),
                                   media_index.height),
                 duration_ms = COALESCE((SELECT catalog.duration_ms FROM media_dedupe_catalog catalog
                                          WHERE catalog.normalized_path = media_index.normalized_path),
                                        media_index.duration_ms),
                 fingerprint_status = 'complete',
                 updated_at = ?1
             WHERE fingerprint_status = 'pending'
               AND (?2 IS NULL OR source_id = ?2)
               AND EXISTS (
                   SELECT 1 FROM media_dedupe_catalog catalog
                    WHERE catalog.normalized_path = media_index.normalized_path
                      AND catalog.hash_status = 'complete'
                      AND catalog.sha256 IS NOT NULL
                      AND catalog.size_bytes = media_index.size_bytes
                      AND catalog.modified_at_ms = media_index.modified_at_ms
               )",
            params![timestamp, source_id],
        )
        .map_err(|error| error.to_string())?;
    Ok(updated)
}

/// What one reconciliation pass over a profile changed.
#[derive(Clone, Copy, Default)]
pub(crate) struct MediaIndexReconcileOutcome {
    pub(crate) indexed: usize,
    pub(crate) updated: usize,
    pub(crate) missing: usize,
    pub(crate) inherited: usize,
}

fn is_indexable_media_file(file_name: &str) -> bool {
    // The slideshow soundtrack is an auxiliary track of another post, not a
    // media item of its own — the gallery already presents it that way.
    if is_slideshow_audio_file(file_name) {
        return false;
    }
    let Some((_, extension)) = file_name.rsplit_once('.') else {
        return false;
    };
    let extension = extension.to_ascii_lowercase();
    GALLERY_VIDEO_EXTS.contains(&extension.as_str())
        || GALLERY_IMAGE_EXTS.contains(&extension.as_str())
}

fn media_type_for_extension(file_name: &str) -> &'static str {
    let extension = file_name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    if GALLERY_VIDEO_EXTS.contains(&extension.as_str()) {
        "video"
    } else {
        "image"
    }
}

/// Depth-first walk of a profile folder. Dot-directories are skipped: they hold
/// app-managed material (thumbnail caches, quarantined duplicates), not media
/// the user downloaded.
fn collect_media_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            match entry.file_type() {
                Ok(file_type) if file_type.is_dir() => pending.push(path),
                Ok(file_type) if file_type.is_file() && is_indexable_media_file(name) => {
                    files.push(path)
                }
                _ => {}
            }
        }
    }
    files
}

/// Post/media keys the sync ledger already knows for a given file, so files
/// that were downloaded by a sync keep their provider identity even when the
/// index row is (re)built from disk.
fn ledger_identity(
    connection: &Connection,
    provider: &str,
    source_id: &str,
    relative_path: &str,
) -> (Option<String>, Option<String>, Option<String>, Option<i64>) {
    connection
        .query_row(
            "SELECT provider_media_key, provider_post_key, media_section, captured_at
             FROM provider_sync_media_ledger
             WHERE provider = ?1 AND source_id = ?2 AND relative_path = ?3",
            params![provider, source_id, relative_path],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            },
        )
        .optional()
        .ok()
        .flatten()
        .unwrap_or((None, None, None, None))
}

/// Brings the index in line with what is actually on disk for one profile.
///
/// This is what keeps the index honest for everything the ingest hook cannot
/// see: libraries imported from SCrawler/4K Stogram, files moved or deleted
/// outside the app, and media downloaded before the index existed.
pub(crate) fn reconcile_source_media_index_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    source: &SourceProfile,
    timestamp: &str,
) -> Result<MediaIndexReconcileOutcome, String> {
    let root = resolved_source_media_output_root_with_connection(connection, layout, source)?;
    let mut known: HashSet<String> = HashSet::new();
    {
        let mut statement = connection
            .prepare("SELECT relative_path FROM media_index WHERE source_id = ?1")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![source.id], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        for row in rows {
            known.insert(row.map_err(|error| error.to_string())?);
        }
    }

    let mut outcome = MediaIndexReconcileOutcome::default();
    let mut seen: HashSet<String> = HashSet::new();
    for path in collect_media_files(&root) {
        let relative_path = normalize_instagram_relative_media_path(&root, &path);
        if relative_path.is_empty() {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let (media_key, post_key, section, captured_at) =
            ledger_identity(connection, &source.provider, &source.id, &relative_path);
        upsert_media_index_entry(
            connection,
            &source.provider,
            &source.id,
            &MediaIndexEntry {
                relative_path: &relative_path,
                absolute_path: &path,
                media_type: media_type_for_extension(file_name),
                media_section: section.as_deref().unwrap_or_default(),
                provider_media_key: media_key.as_deref(),
                provider_post_key: post_key.as_deref(),
                captured_at,
            },
            timestamp,
        )?;
        if known.contains(&relative_path) {
            outcome.updated += 1;
        } else {
            outcome.indexed += 1;
        }
        seen.insert(relative_path);
    }

    for relative_path in known.difference(&seen) {
        let changed = connection
            .execute(
                "UPDATE media_index
                 SET local_state = 'missing_on_disk', updated_at = ?3
                 WHERE source_id = ?1 AND relative_path = ?2
                   AND local_state <> 'missing_on_disk'",
                params![source.id, relative_path, timestamp],
            )
            .map_err(|error| error.to_string())?;
        outcome.missing += changed;
    }

    outcome.inherited = inherit_media_index_fingerprints(connection, Some(&source.id), timestamp)?;
    Ok(outcome)
}

/// Connection-owning wrapper used by the indexing runtime thread.
pub(crate) fn reconcile_source_media_index(
    source_id: &str,
) -> Result<MediaIndexReconcileOutcome, String> {
    with_workspace(|connection, layout| {
        let source = load_sources(connection)?
            .into_iter()
            .find(|candidate| candidate.id == source_id)
            .ok_or_else(|| format!("Profile {source_id} is no longer in the workspace."))?;
        reconcile_source_media_index_with_connection(
            connection,
            layout,
            &source,
            &Utc::now().to_rfc3339(),
        )
    })
}

/// Profiles the indexing runtime will walk, in a stable order.
pub(crate) fn media_index_reconcile_targets(
    scope_source_id: Option<&str>,
) -> Result<Vec<(String, String)>, String> {
    with_workspace(|connection, _| {
        Ok(load_sources(connection)?
            .into_iter()
            .filter(|source| {
                scope_source_id.is_none_or(|scope| scope == source.id)
            })
            .map(|source| (source.id, source.handle))
            .collect())
    })
}

/// A file still waiting for its fingerprint, with what the runtime needs to
/// compute one.
pub(crate) struct PendingFingerprint {
    pub(crate) id: String,
    pub(crate) absolute_path: PathBuf,
    pub(crate) kind: String,
    pub(crate) also_needs_exact: bool,
    pub(crate) duration_ms: Option<i64>,
    pub(crate) size_bytes: i64,
    pub(crate) modified_at_ms: i64,
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct FingerprintJobCounts {
    pub(crate) exact: i64,
    pub(crate) perceptual_image: i64,
    pub(crate) perceptual_video: i64,
    pub(crate) failed: i64,
}

impl FingerprintJobCounts {
    pub(crate) fn pending(self) -> i64 {
        self.exact + self.perceptual_image + self.perceptual_video
    }
}

const FINGERPRINT_POLICY_VERSION: i64 = 1;
const CROSS_SOURCE_WINDOW_SECONDS: i64 = 7 * 24 * 60 * 60;

/// Rebuilds the missing-work queue from comparison eligibility, not from the
/// number of catalog rows. A fingerprint that cannot participate in a variant
/// group is deliberately not scheduled.
pub(super) fn plan_media_fingerprint_jobs_with_connection(
    connection: &Connection,
) -> Result<FingerprintJobCounts, String> {
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        let timestamp = Utc::now().to_rfc3339();
        transaction
            .execute(
                "UPDATE media_fingerprint_jobs
                 SET status = 'queued', lease_owner = NULL, lease_expires_at = NULL,
                     updated_at = ?1
                 WHERE status = 'running'",
                params![timestamp],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute("UPDATE media_fingerprint_jobs SET priority = -1", [])
            .map_err(|error| error.to_string())?;
        transaction
            .execute_batch(&format!(
                "DROP TABLE IF EXISTS temp.planned_fingerprint_jobs;
                 CREATE TEMP TABLE planned_fingerprint_jobs (
                    media_id TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    PRIMARY KEY (media_id, kind)
                 ) WITHOUT ROWID;

                 INSERT OR IGNORE INTO planned_fingerprint_jobs(media_id, kind)
                 SELECT media_index.id, 'exact'
                 FROM media_index
                 JOIN (
                    SELECT source_id, size_bytes
                    FROM media_index
                    WHERE local_state = 'present'
                    GROUP BY source_id, size_bytes
                    HAVING COUNT(DISTINCT LOWER(TRIM(media_section))) > 1
                 ) eligible
                   ON eligible.source_id = media_index.source_id
                  AND eligible.size_bytes = media_index.size_bytes
                 WHERE media_index.local_state = 'present';

                 INSERT OR IGNORE INTO planned_fingerprint_jobs(media_id, kind)
                 SELECT media_index.id,
                        CASE WHEN media_index.media_type = 'video'
                             THEN 'perceptual_video' ELSE 'perceptual_image' END
                 FROM media_index
                 JOIN (
                    SELECT source_id, media_type
                    FROM media_index
                    WHERE local_state = 'present'
                    GROUP BY source_id, media_type
                    HAVING COUNT(DISTINCT LOWER(TRIM(media_section))) > 1
                 ) eligible
                   ON eligible.source_id = media_index.source_id
                  AND eligible.media_type = media_index.media_type
                 WHERE media_index.local_state = 'present';

                 INSERT OR IGNORE INTO planned_fingerprint_jobs(media_id, kind)
                 SELECT left_media.id, 'exact'
                 FROM media_index left_media
                 JOIN source_profiles left_source ON left_source.id = left_media.source_id
                 JOIN source_profiles right_source
                   ON right_source.identity_id = left_source.identity_id
                  AND right_source.id <> left_source.id
                 JOIN media_index right_media
                   ON right_media.source_id = right_source.id
                  AND right_media.local_state = 'present'
                  AND right_media.size_bytes = left_media.size_bytes
                 WHERE left_media.local_state = 'present'
                   AND left_source.identity_id IS NOT NULL
                   AND (left_media.captured_at IS NULL OR right_media.captured_at IS NULL
                        OR ABS(left_media.captured_at - right_media.captured_at) <= {CROSS_SOURCE_WINDOW_SECONDS});

                 INSERT OR IGNORE INTO planned_fingerprint_jobs(media_id, kind)
                 SELECT left_media.id,
                        CASE WHEN left_media.media_type = 'video'
                             THEN 'perceptual_video' ELSE 'perceptual_image' END
                 FROM media_index left_media
                 JOIN source_profiles left_source ON left_source.id = left_media.source_id
                 JOIN source_profiles right_source
                   ON right_source.identity_id = left_source.identity_id
                  AND right_source.id <> left_source.id
                 JOIN media_index right_media
                   ON right_media.source_id = right_source.id
                  AND right_media.local_state = 'present'
                  AND right_media.media_type = left_media.media_type
                 WHERE left_media.local_state = 'present'
                   AND left_source.identity_id IS NOT NULL
                   AND (left_media.captured_at IS NULL OR right_media.captured_at IS NULL
                        OR ABS(left_media.captured_at - right_media.captured_at) <= {CROSS_SOURCE_WINDOW_SECONDS});"
            ))
            .map_err(|error| error.to_string())?;

        for (kind, media_predicate, output_predicate, priority) in [
            (
                "exact",
                "1 = 1",
                "media_index.sha256 IS NULL",
                300_i64,
            ),
            (
                "perceptual_image",
                "media_index.media_type <> 'video'",
                "(media_index.ahash64 IS NULL OR media_index.dhash64 IS NULL)",
                200_i64,
            ),
            (
                "perceptual_video",
                "media_index.media_type = 'video'",
                "media_index.video_signature IS NULL",
                100_i64,
            ),
        ] {
            let sql = format!(
                "INSERT INTO media_fingerprint_jobs (
                    media_id, kind, status, priority, attempts, policy_version,
                    candidate_context, expected_size_bytes, expected_modified_at_ms,
                    created_at, updated_at
                 )
                 SELECT media_index.id, ?1,
                        CASE WHEN {output_predicate} THEN 'queued' ELSE 'complete' END,
                        ?2, 0, ?3,
                        media_index.source_id || '|' ||
                        LOWER(TRIM(media_index.media_section)) || '|' ||
                        COALESCE(source_profiles.identity_id, '-') || '|' || ?3,
                        media_index.size_bytes, media_index.modified_at_ms, ?4, ?4
                 FROM media_index
                 JOIN source_profiles ON source_profiles.id = media_index.source_id
                 WHERE media_index.local_state = 'present'
                   AND {media_predicate}
                   AND EXISTS (SELECT 1 FROM planned_fingerprint_jobs planned
                               WHERE planned.media_id = media_index.id
                                 AND planned.kind = ?1)
                 ON CONFLICT(media_id, kind) DO UPDATE SET
                    status = CASE
                        WHEN excluded.status = 'complete' THEN 'complete'
                        WHEN media_fingerprint_jobs.expected_size_bytes <> excluded.expected_size_bytes
                          OR media_fingerprint_jobs.expected_modified_at_ms <> excluded.expected_modified_at_ms
                          OR media_fingerprint_jobs.policy_version <> excluded.policy_version
                          OR media_fingerprint_jobs.candidate_context <> excluded.candidate_context
                        THEN excluded.status
                        WHEN media_fingerprint_jobs.status = 'failed'
                             AND media_fingerprint_jobs.attempts >= 2 THEN 'failed'
                        WHEN media_fingerprint_jobs.status = 'complete' THEN 'queued'
                        ELSE 'queued'
                    END,
                    priority = excluded.priority,
                    policy_version = excluded.policy_version,
                    candidate_context = excluded.candidate_context,
                    attempts = CASE
                        WHEN media_fingerprint_jobs.expected_size_bytes <> excluded.expected_size_bytes
                          OR media_fingerprint_jobs.expected_modified_at_ms <> excluded.expected_modified_at_ms
                          OR media_fingerprint_jobs.policy_version <> excluded.policy_version
                          OR media_fingerprint_jobs.candidate_context <> excluded.candidate_context
                        THEN 0 ELSE media_fingerprint_jobs.attempts END,
                    expected_size_bytes = excluded.expected_size_bytes,
                    expected_modified_at_ms = excluded.expected_modified_at_ms,
                    lease_owner = NULL,
                    lease_expires_at = NULL,
                    error = CASE
                        WHEN media_fingerprint_jobs.attempts >= 2 THEN media_fingerprint_jobs.error
                        ELSE NULL
                    END,
                    updated_at = excluded.updated_at"
            );
            transaction
                .execute(
                    &sql,
                    params![kind, priority, FINGERPRINT_POLICY_VERSION, timestamp],
                )
                .map_err(|error| error.to_string())?;
        }

        transaction
            .execute("DELETE FROM media_fingerprint_jobs WHERE priority = -1", [])
            .map_err(|error| error.to_string())?;
        transaction
            .execute("DROP TABLE temp.planned_fingerprint_jobs", [])
            .map_err(|error| error.to_string())?;
        refresh_media_index_fingerprint_status_with_connection(&transaction, &timestamp)?;
        let counts = fingerprint_job_counts_with_connection(&transaction)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(counts)
}

pub(crate) fn plan_media_fingerprint_jobs() -> Result<FingerprintJobCounts, String> {
    with_workspace(|connection, _| plan_media_fingerprint_jobs_with_connection(connection))
}

fn fingerprint_job_counts_with_connection(
    connection: &Connection,
) -> Result<FingerprintJobCounts, String> {
    connection
        .query_row(
            "SELECT
                COALESCE(SUM(CASE WHEN kind = 'exact' AND status IN ('queued', 'running') THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN kind = 'perceptual_image' AND status IN ('queued', 'running') THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN kind = 'perceptual_video' AND status IN ('queued', 'running') THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END), 0)
             FROM media_fingerprint_jobs",
            [],
            |row| {
                Ok(FingerprintJobCounts {
                    exact: row.get(0)?,
                    perceptual_image: row.get(1)?,
                    perceptual_video: row.get(2)?,
                    failed: row.get(3)?,
                })
            },
        )
        .map_err(|error| error.to_string())
}

/// Leases the next slice of one phase. Paths are rebuilt from the profile root
/// so the runtime never has to trust the lowercased `normalized_path`.
pub(super) fn lease_pending_fingerprints_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    kind: &str,
    limit: u32,
    lease_owner: &str,
) -> Result<Vec<PendingFingerprint>, String> {
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        let now = Utc::now();
        let expires = (now + Duration::minutes(2)).to_rfc3339();
        let now = now.to_rfc3339();
        transaction
            .execute(
                "UPDATE media_fingerprint_jobs
                 SET status = 'running', lease_owner = ?3, lease_expires_at = ?4,
                     attempts = attempts + 1, updated_at = ?5
                 WHERE (media_id, kind) IN (
                     SELECT media_id, kind FROM media_fingerprint_jobs
                     WHERE kind = ?1 AND status = 'queued'
                     ORDER BY priority DESC, updated_at, media_id
                     LIMIT ?2
                 )",
                params![kind, limit, lease_owner, expires, now],
            )
            .map_err(|error| error.to_string())?;

        let mut statement = transaction
            .prepare(
                "SELECT media_index.id, media_index.source_id, media_index.relative_path,
                        media_index.duration_ms,
                        media_index.size_bytes, media_index.modified_at_ms,
                        jobs.kind,
                        EXISTS (
                            SELECT 1 FROM media_fingerprint_jobs exact_job
                            WHERE exact_job.media_id = media_index.id
                              AND exact_job.kind = 'exact'
                              AND exact_job.status IN ('queued', 'running')
                        )
                 FROM media_fingerprint_jobs jobs
                 JOIN media_index ON media_index.id = jobs.media_id
                 WHERE jobs.kind = ?1 AND jobs.status = 'running'
                   AND jobs.lease_owner = ?2
                 ORDER BY jobs.priority DESC, jobs.updated_at, jobs.media_id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![kind, lease_owner], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)? == 1,
                ))
            })
            .map_err(|error| error.to_string())?;

        let sources = load_sources(&transaction)?
            .into_iter()
            .map(|source| (source.id.clone(), source))
            .collect::<HashMap<_, _>>();
        let mut roots: HashMap<String, PathBuf> = HashMap::new();
        let mut pending = Vec::new();
        for row in rows {
            let (
                id,
                source_id,
                relative_path,
                duration_ms,
                size_bytes,
                modified_at_ms,
                kind,
                also_needs_exact,
            ) = row.map_err(|error| error.to_string())?;
            let root = match roots.get(&source_id) {
                Some(root) => root.clone(),
                None => {
                    let Some(source) = sources.get(&source_id) else {
                        continue;
                    };
                    let root = resolved_source_media_output_root_with_connection(
                        &transaction, layout, source,
                    )?;
                    roots.insert(source_id.clone(), root.clone());
                    root
                }
            };
            pending.push(PendingFingerprint {
                id,
                absolute_path: root.join(&relative_path),
                kind,
                also_needs_exact,
                duration_ms,
                size_bytes,
                modified_at_ms,
            });
        }
        drop(statement);
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(pending)
}

pub(crate) fn lease_pending_fingerprints(
    kind: &str,
    limit: u32,
    lease_owner: &str,
) -> Result<Vec<PendingFingerprint>, String> {
    with_workspace(|connection, layout| {
        lease_pending_fingerprints_with_connection(connection, layout, kind, limit, lease_owner)
    })
}

/// Stores a computed fingerprint. The size/mtime guard means a file replaced
/// while the runtime was hashing it stays `pending` instead of being recorded
/// with a hash of bytes that no longer exist.
pub(super) fn complete_media_fingerprint_job_with_connection(
    connection: &Connection,
    item: &PendingFingerprint,
    sha256: Option<&str>,
    ahash64: Option<&str>,
    dhash64: Option<&str>,
    video_signature: Option<&str>,
    width: Option<i64>,
    height: Option<i64>,
) -> Result<bool, String> {
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        let timestamp = Utc::now().to_rfc3339();
        let updated = transaction
            .execute(
                "UPDATE media_index
                 SET sha256 = COALESCE(?2, sha256),
                     ahash64 = COALESCE(?3, ahash64),
                     dhash64 = COALESCE(?4, dhash64),
                     video_signature = COALESCE(?5, video_signature),
                     width = COALESCE(?6, width), height = COALESCE(?7, height),
                     fingerprint_status = 'complete', updated_at = ?8
                 WHERE id = ?1 AND size_bytes = ?9 AND modified_at_ms = ?10",
                params![
                    item.id,
                    sha256,
                    ahash64,
                    dhash64,
                    video_signature,
                    width,
                    height,
                    timestamp,
                    item.size_bytes,
                    item.modified_at_ms,
                ],
            )
            .map_err(|error| error.to_string())?;
        if updated > 0 {
            transaction
                .execute(
                    "UPDATE media_fingerprint_jobs
                     SET status = 'complete', lease_owner = NULL, lease_expires_at = NULL,
                         error = NULL, updated_at = ?3
                     WHERE media_id = ?1 AND kind = ?2",
                    params![item.id, item.kind, timestamp],
                )
                .map_err(|error| error.to_string())?;
            if item.also_needs_exact && sha256.is_some() {
                transaction
                    .execute(
                        "UPDATE media_fingerprint_jobs
                         SET status = 'complete', lease_owner = NULL, lease_expires_at = NULL,
                             error = NULL, updated_at = ?2
                         WHERE media_id = ?1 AND kind = 'exact'",
                        params![item.id, timestamp],
                    )
                    .map_err(|error| error.to_string())?;
            }
        } else {
            transaction
                .execute(
                    "UPDATE media_fingerprint_jobs
                     SET status = 'queued', lease_owner = NULL, lease_expires_at = NULL,
                         error = 'file_changed', updated_at = ?2
                     WHERE media_id = ?1 AND kind = ?3",
                    params![item.id, timestamp, item.kind],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(updated > 0)
}

pub(crate) fn complete_media_fingerprint_job(
    item: &PendingFingerprint,
    sha256: Option<&str>,
    ahash64: Option<&str>,
    dhash64: Option<&str>,
    video_signature: Option<&str>,
    width: Option<i64>,
    height: Option<i64>,
) -> Result<bool, String> {
    with_workspace(|connection, _| {
        complete_media_fingerprint_job_with_connection(
            connection,
            item,
            sha256,
            ahash64,
            dhash64,
            video_signature,
            width,
            height,
        )
    })
}

pub(crate) fn mark_fingerprint_job_failed(item: &PendingFingerprint, error_code: &str) -> Result<bool, String> {
    with_workspace(|connection, _| {
        let updated = connection
            .execute(
                "UPDATE media_fingerprint_jobs
                 SET status = CASE WHEN attempts >= 2 THEN 'failed' ELSE 'queued' END,
                     lease_owner = NULL, lease_expires_at = NULL, error = ?3, updated_at = ?4
                 WHERE media_id = ?1 AND kind = ?2",
                params![item.id, item.kind, error_code, Utc::now().to_rfc3339()],
            )
            .map_err(|error| error.to_string())?;
        let failed = connection
            .query_row(
                "SELECT status = 'failed' FROM media_fingerprint_jobs
                 WHERE media_id = ?1 AND kind = ?2",
                params![item.id, item.kind],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .unwrap_or(0)
            == 1;
        Ok(updated > 0 && failed)
    })
}

fn refresh_media_index_fingerprint_status_with_connection(
    connection: &Connection,
    timestamp: &str,
) -> Result<(), String> {
    connection
        .execute(
            "UPDATE media_index
             SET fingerprint_status = CASE
                 WHEN EXISTS (SELECT 1 FROM media_fingerprint_jobs jobs
                              WHERE jobs.media_id = media_index.id
                                AND jobs.status IN ('queued', 'running')) THEN 'pending'
                 WHEN EXISTS (SELECT 1 FROM media_fingerprint_jobs jobs
                              WHERE jobs.media_id = media_index.id
                                AND jobs.status = 'failed') THEN 'failed'
                 ELSE 'complete'
             END,
             updated_at = ?1
             WHERE local_state = 'present'",
            params![timestamp],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn finalize_media_fingerprint_jobs() -> Result<FingerprintJobCounts, String> {
    with_workspace(|connection, _| {
        refresh_media_index_fingerprint_status_with_connection(
            connection,
            &Utc::now().to_rfc3339(),
        )?;
        fingerprint_job_counts_with_connection(connection)
    })
}

pub(crate) fn retry_failed_media_fingerprint_jobs() -> Result<i64, String> {
    with_workspace(|connection, _| {
        connection
            .execute(
                "UPDATE media_fingerprint_jobs
                 SET status = 'queued', attempts = 0, error = NULL,
                     lease_owner = NULL, lease_expires_at = NULL, updated_at = ?1
                 WHERE status = 'failed'",
                params![Utc::now().to_rfc3339()],
            )
            .map(|value| value as i64)
            .map_err(|error| error.to_string())
    })
}

pub(crate) fn release_media_fingerprint_leases(lease_owner: &str) -> Result<(), String> {
    with_workspace(|connection, _| {
        connection
            .execute(
                "UPDATE media_fingerprint_jobs
                 SET status = 'queued', lease_owner = NULL, lease_expires_at = NULL,
                     updated_at = ?2
                 WHERE status = 'running' AND lease_owner = ?1",
                params![lease_owner, Utc::now().to_rfc3339()],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

pub(crate) fn insert_media_index_run(run: &MediaIndexRun) -> Result<(), String> {
    with_workspace(|connection, _| {
        connection
            .execute(
                "INSERT INTO media_index_runs (
                    id, status, stage, scope_source_id, sources_total, sources_processed,
                    files_indexed, files_updated, files_missing, hashes_inherited,
                    current_source_handle, error, started_at, finished_at,
                    fingerprints_total, fingerprints_done, fingerprint_started_at,
                    resource_profile, phase_total, phase_done, phase_failed,
                    bytes_processed, last_progress_at, rate_per_second, eta_seconds,
                    updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                           ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)",
                params![
                    run.id,
                    run.status,
                    run.stage,
                    run.scope_source_id,
                    run.sources_total,
                    run.sources_processed,
                    run.files_indexed,
                    run.files_updated,
                    run.files_missing,
                    run.hashes_inherited,
                    run.current_source_handle,
                    run.error,
                    run.started_at,
                    run.finished_at,
                    run.fingerprints_total,
                    run.fingerprints_done,
                    run.fingerprint_started_at,
                    run.resource_profile,
                    run.phase_total,
                    run.phase_done,
                    run.phase_failed,
                    run.bytes_processed,
                    run.last_progress_at,
                    run.rate_per_second,
                    run.eta_seconds,
                    Utc::now().to_rfc3339(),
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

pub(crate) fn persist_media_index_run(run: &MediaIndexRun) -> Result<(), String> {
    with_workspace(|connection, _| {
        connection
            .execute(
                "UPDATE media_index_runs
                 SET status = ?2, stage = ?3, sources_total = ?4, sources_processed = ?5,
                     files_indexed = ?6, files_updated = ?7, files_missing = ?8,
                     hashes_inherited = ?9, current_source_handle = ?10, error = ?11,
                     finished_at = ?12, updated_at = ?13,
                     fingerprints_total = ?14, fingerprints_done = ?15,
                     fingerprint_started_at = ?16, resource_profile = ?17,
                     phase_total = ?18, phase_done = ?19, phase_failed = ?20,
                     bytes_processed = ?21, last_progress_at = ?22,
                     rate_per_second = ?23, eta_seconds = ?24
                 WHERE id = ?1",
                params![
                    run.id,
                    run.status,
                    run.stage,
                    run.sources_total,
                    run.sources_processed,
                    run.files_indexed,
                    run.files_updated,
                    run.files_missing,
                    run.hashes_inherited,
                    run.current_source_handle,
                    run.error,
                    run.finished_at,
                    Utc::now().to_rfc3339(),
                    run.fingerprints_total,
                    run.fingerprints_done,
                    run.fingerprint_started_at,
                    run.resource_profile,
                    run.phase_total,
                    run.phase_done,
                    run.phase_failed,
                    run.bytes_processed,
                    run.last_progress_at,
                    run.rate_per_second,
                    run.eta_seconds,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

fn media_index_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MediaIndexRun> {
    Ok(MediaIndexRun {
        id: row.get(0)?,
        status: row.get(1)?,
        stage: row.get(2)?,
        scope_source_id: row.get(3)?,
        sources_total: row.get(4)?,
        sources_processed: row.get(5)?,
        files_indexed: row.get(6)?,
        files_updated: row.get(7)?,
        files_missing: row.get(8)?,
        hashes_inherited: row.get(9)?,
        fingerprints_total: row.get(14).unwrap_or(0),
        fingerprints_done: row.get(15).unwrap_or(0),
        fingerprint_started_at: row.get(16).unwrap_or(None),
        resource_profile: row.get(17).unwrap_or_else(|_| "balanced".to_string()),
        phase_total: row.get(18).unwrap_or(0),
        phase_done: row.get(19).unwrap_or(0),
        phase_failed: row.get(20).unwrap_or(0),
        bytes_processed: row.get(21).unwrap_or(0),
        last_progress_at: row.get(22).unwrap_or(None),
        rate_per_second: row.get(23).unwrap_or(0.0),
        eta_seconds: row.get(24).unwrap_or(None),
        current_source_handle: row.get(10)?,
        error: row.get(11)?,
        started_at: row.get(12)?,
        finished_at: row.get(13)?,
    })
}

pub(crate) fn load_latest_media_index_run(
    connection: &Connection,
) -> Result<Option<MediaIndexRun>, String> {
    connection
        .query_row(
            "SELECT id, status, stage, scope_source_id, sources_total, sources_processed,
                    files_indexed, files_updated, files_missing, hashes_inherited,
                    current_source_handle, error, started_at, finished_at,
                    fingerprints_total, fingerprints_done, fingerprint_started_at,
                    resource_profile, phase_total, phase_done, phase_failed,
                    bytes_processed, last_progress_at, rate_per_second, eta_seconds
             FROM media_index_runs
             ORDER BY started_at DESC, updated_at DESC
             LIMIT 1",
            [],
            media_index_run_from_row,
        )
        .optional()
        .map_err(|error| error.to_string())
}

/// Interrupted runs become explicitly resumable and their leases return to the
/// queue. They are not auto-started because a fast run may intentionally have
/// been stopped before the next application launch.
pub(crate) fn recover_interrupted_media_index_runs() -> Result<(), String> {
    with_workspace(|connection, _| {
        let timestamp = Utc::now().to_rfc3339();
        connection
            .execute(
                "UPDATE media_index_runs
                 SET status = 'paused',
                     error = COALESCE(error, 'Interrupted when the app closed.'),
                     updated_at = ?1
                 WHERE status IN ('queued', 'running', 'pausing')",
                params![timestamp],
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE media_fingerprint_jobs
                 SET status = 'queued', lease_owner = NULL, lease_expires_at = NULL,
                     updated_at = ?1
                 WHERE status = 'running'",
                params![timestamp],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

pub(crate) fn load_media_index_status() -> Result<MediaIndexStatus, String> {
    with_workspace(|connection, _| {
        Ok(MediaIndexStatus {
            counts: media_index_counts(connection)?,
            run: load_latest_media_index_run(connection)?,
        })
    })
}

/// Aggregate counters backing the index status card and, later, the library
/// dashboard. Kept as one pass over the table so the caller never needs to fan
/// out several queries for a status poll.
pub(crate) fn media_index_counts(connection: &Connection) -> Result<MediaIndexCounts, String> {
    connection
        .query_row(
            "SELECT
                COUNT(*),
                COALESCE(SUM(size_bytes), 0),
                COALESCE(SUM(CASE WHEN fingerprint_status = 'pending' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN fingerprint_status = 'failed' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN local_state = 'missing_on_disk' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN upstream_state = 'missing' THEN 1 ELSE 0 END), 0),
                COUNT(DISTINCT source_id)
             FROM media_index",
            [],
            |row| {
                Ok(MediaIndexCounts {
                    total_files: row.get::<_, i64>(0)?,
                    total_bytes: row.get::<_, i64>(1)?,
                    pending_fingerprints: row.get::<_, i64>(2)?,
                    failed_fingerprints: row.get::<_, i64>(3)?,
                    missing_on_disk: row.get::<_, i64>(4)?,
                    upstream_missing: row.get::<_, i64>(5)?,
                    indexed_sources: row.get::<_, i64>(6)?,
                })
            },
        )
        .map_err(|error| error.to_string())
}
