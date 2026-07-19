use super::*;
use crate::domain::models::{
    MediaDedupeFile, MediaDedupeGroup, MediaDedupeScanResult, MediaDedupeSourceJobStatus,
};
use rusqlite::OptionalExtension;
use std::collections::{BTreeMap, HashSet};

#[derive(Clone)]
pub(crate) struct MediaDedupeSourceRoot {
    pub source_id: String,
    pub provider: String,
    pub path: PathBuf,
}

#[derive(Clone)]
pub(crate) struct MediaDedupeScanContext {
    pub roots: Vec<PathBuf>,
    pub source_roots: Vec<MediaDedupeSourceRoot>,
}

pub(crate) struct MediaDedupeIndexedFile<'a> {
    pub scan_id: &'a str,
    pub path: &'a Path,
    pub normalized_path: &'a str,
    pub source_id: Option<&'a str>,
    pub provider: Option<&'a str>,
    pub root_path: &'a Path,
    pub volume_key: &'a str,
    pub media_type: &'a str,
    pub size_bytes: u64,
    pub modified_at_ms: i64,
    pub sha256: &'a str,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
    pub ahash64: Option<&'a str>,
    pub dhash64: Option<&'a str>,
    pub video_hashes_json: Option<&'a str>,
}

#[derive(Clone)]
pub(crate) struct MediaDedupeIndexedFileOwned {
    pub scan_id: String,
    pub path: PathBuf,
    pub normalized_path: String,
    pub source_id: Option<String>,
    pub provider: Option<String>,
    pub root_path: PathBuf,
    pub volume_key: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub modified_at_ms: i64,
    pub sha256: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
    pub ahash64: Option<String>,
    pub dhash64: Option<String>,
    pub video_hashes_json: Option<String>,
}

#[derive(Clone)]
pub(crate) struct MediaDedupeCatalogEntryOwned {
    pub path: PathBuf,
    pub normalized_path: String,
    pub source_id: Option<String>,
    pub provider: Option<String>,
    pub root_path: PathBuf,
    pub volume_key: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub modified_at_ms: i64,
}

#[derive(Clone)]
pub(crate) struct MediaDedupeCatalogCandidate {
    pub path: PathBuf,
    pub normalized_path: String,
    pub source_id: Option<String>,
    pub provider: Option<String>,
    pub root_path: PathBuf,
    pub volume_key: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub modified_at_ms: i64,
    pub sha256: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
    pub ahash64: Option<String>,
    pub dhash64: Option<String>,
}

pub(crate) struct MediaDedupeCatalogStats {
    pub files: u64,
    pub bytes: u64,
    pub candidate_files: u64,
    pub candidate_bytes: u64,
}

pub(crate) struct MediaDedupeSourceVideoInventory {
    pub fingerprint: String,
    pub video_count: u32,
}

#[derive(Clone)]
pub(crate) struct MediaDedupeVdfCandidateOwned {
    pub group_id: String,
    pub path: String,
    pub normalized_path: String,
    pub similarity_percent: f64,
    pub size_bytes: u64,
    pub duration_ms: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl MediaDedupeIndexedFileOwned {
    fn borrowed(&self) -> MediaDedupeIndexedFile<'_> {
        MediaDedupeIndexedFile {
            scan_id: &self.scan_id,
            path: &self.path,
            normalized_path: &self.normalized_path,
            source_id: self.source_id.as_deref(),
            provider: self.provider.as_deref(),
            root_path: &self.root_path,
            volume_key: &self.volume_key,
            media_type: &self.media_type,
            size_bytes: self.size_bytes,
            modified_at_ms: self.modified_at_ms,
            sha256: &self.sha256,
            width: self.width,
            height: self.height,
            duration_ms: self.duration_ms,
            ahash64: self.ahash64.as_deref(),
            dhash64: self.dhash64.as_deref(),
            video_hashes_json: self.video_hashes_json.as_deref(),
        }
    }
}

#[derive(Clone)]
struct IndexedFileRow {
    path: String,
    source_id: Option<String>,
    provider: Option<String>,
    volume_key: String,
    media_type: String,
    size_bytes: u64,
    width: Option<u32>,
    height: Option<u32>,
    duration_ms: Option<u64>,
    sha256: String,
    ahash64: Option<String>,
    dhash64: Option<String>,
    video_hashes: Vec<(String, String)>,
}

pub(crate) fn media_dedupe_scan_context(
    provider_scope: Option<&str>,
    source_scope: Option<&str>,
) -> Result<MediaDedupeScanContext, String> {
    with_workspace(|connection, layout| {
        let sources = load_sources(connection)?;
        let paths = compute_source_media_paths(connection, layout, &sources);
        let mut roots = Vec::new();
        let mut source_roots = Vec::new();
        if provider_scope.is_none() && source_scope.is_none() {
            roots.push(layout.media_root.clone());
        }
        for source in sources {
            if source_scope.is_some_and(|source_id| source.id != source_id) {
                continue;
            }
            if provider_scope
                .is_some_and(|provider| !source.provider.eq_ignore_ascii_case(provider))
            {
                continue;
            }
            let Some(path) = paths.get(&source.id) else {
                continue;
            };
            let path = PathBuf::from(path);
            roots.push(path.clone());
            source_roots.push(MediaDedupeSourceRoot {
                source_id: source.id,
                provider: source.provider,
                path,
            });
        }
        if roots.is_empty() {
            let scope = source_scope
                .map(|source_id| format!("profile '{source_id}'"))
                .or_else(|| provider_scope.map(|provider| provider.to_string()))
                .unwrap_or_else(|| "the selected scope".to_string());
            return Err(format!("No configured media roots were found for {scope}."));
        }
        roots.sort_by(|left, right| {
            left.components()
                .count()
                .cmp(&right.components().count())
                .then_with(|| normalize_path_key(left).cmp(&normalize_path_key(right)))
        });
        let mut disjoint_roots = Vec::<PathBuf>::new();
        for root in roots {
            if disjoint_roots.iter().any(|known| {
                path_key_starts_with(&normalize_path_key(&root), &normalize_path_key(known))
            }) {
                continue;
            }
            disjoint_roots.push(root);
        }
        source_roots.sort_by(|left, right| {
            right
                .path
                .components()
                .count()
                .cmp(&left.path.components().count())
        });
        Ok(MediaDedupeScanContext {
            roots: disjoint_roots,
            source_roots,
        })
    })
}

pub(crate) fn media_dedupe_source_relative_path(
    source_id: &str,
    path: &Path,
) -> Result<String, String> {
    let context = media_dedupe_scan_context(None, Some(source_id))?;
    let source = context
        .source_roots
        .first()
        .ok_or_else(|| format!("Profile '{source_id}' has no configured media root."))?;
    let relative = path.strip_prefix(&source.path).map_err(|_| {
        format!(
            "'{}' is outside the media root for profile '{source_id}'.",
            path.display()
        )
    })?;
    let value = relative.to_string_lossy().replace('\\', "/");
    if value.is_empty() {
        return Err("The selected media path is not a file inside the profile root.".to_string());
    }
    Ok(value)
}

fn path_key_starts_with(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

pub(crate) fn begin_media_dedupe_scan(
    scan_id: &str,
    started_at: &str,
    provider_scope: Option<&str>,
    source_scope: Option<&str>,
    resource_profile: &str,
) -> Result<(), String> {
    with_workspace(|connection, _| {
        connection
            .execute(
                "INSERT INTO media_dedupe_scans (
                    id, status, stage, provider_scope, source_scope, resource_profile, started_at, updated_at
                 ) VALUES (?1, 'running', 'inventory', ?2, ?3, ?4, ?5, ?5)",
                params![scan_id, provider_scope, source_scope, resource_profile, started_at],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

fn dedupe_files_match(left: &Path, right: &Path) -> bool {
    let (Ok(left_meta), Ok(right_meta)) = (fs::metadata(left), fs::metadata(right)) else {
        return false;
    };
    if left_meta.len() != right_meta.len() {
        return false;
    }
    dedupe_file_sha(left)
        .zip(dedupe_file_sha(right))
        .is_some_and(|(left, right)| left == right)
}

fn dedupe_file_sha(path: &Path) -> Option<Vec<u8>> {
    let mut file = fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Some(hasher.finalize().to_vec())
}

pub(crate) fn begin_media_dedupe_job(
    job_id: &str,
    scan_id: &str,
    job_kind: &str,
    stage: &str,
    files_total: u64,
    bytes_total: u64,
) -> Result<(), String> {
    with_workspace(|connection, _| {
        let now = Utc::now().to_rfc3339();
        connection
            .execute(
                "INSERT INTO media_dedupe_jobs (
                    id, scan_id, job_kind, status, stage, files_total, bytes_total,
                    started_at, updated_at
                 ) VALUES (?1, ?2, ?3, 'queued', ?4, ?5, ?6, ?7, ?7)",
                params![
                    job_id,
                    scan_id,
                    job_kind,
                    stage,
                    files_total as i64,
                    bytes_total as i64,
                    now
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

pub(crate) fn update_media_dedupe_job(
    job_id: &str,
    status: &str,
    stage: &str,
    files_processed: u64,
    files_total: u64,
    bytes_processed: u64,
    bytes_total: u64,
    current_path: Option<&str>,
    current_root: Option<&str>,
    error: Option<&str>,
) -> Result<(), String> {
    with_workspace(|connection, _| {
        let terminal = matches!(status, "completed" | "failed" | "cancelled" | "interrupted");
        let now = Utc::now().to_rfc3339();
        connection
            .execute(
                "UPDATE media_dedupe_jobs
                 SET status = ?2, stage = ?3, files_processed = ?4, files_total = ?5,
                     bytes_processed = ?6, bytes_total = ?7, current_path = ?8,
                     current_root = ?9, error = ?10,
                     finished_at = CASE WHEN ?11 THEN ?12 ELSE finished_at END,
                     updated_at = ?12
                 WHERE id = ?1",
                params![
                    job_id,
                    status,
                    stage,
                    files_processed as i64,
                    files_total as i64,
                    bytes_processed as i64,
                    bytes_total as i64,
                    current_path,
                    current_root,
                    error,
                    terminal,
                    now,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

pub(crate) fn update_media_dedupe_scan_progress(
    scan_id: &str,
    stage: &str,
    files_scanned: u64,
    files_total: u64,
    bytes_scanned: u64,
    bytes_total: u64,
    current_path: Option<&str>,
) -> Result<(), String> {
    with_workspace(|connection, _| {
        connection
            .execute(
                "UPDATE media_dedupe_scans
                 SET stage = ?2,
                     files_scanned = ?3,
                     files_total = ?4,
                     bytes_scanned = ?5,
                     bytes_total = ?6,
                     current_path = ?7,
                     updated_at = ?8
                 WHERE id = ?1",
                params![
                    scan_id,
                    stage,
                    files_scanned as i64,
                    files_total as i64,
                    bytes_scanned as i64,
                    bytes_total as i64,
                    current_path,
                    Utc::now().to_rfc3339(),
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

pub(crate) fn persist_media_dedupe_catalog_inventory(
    scan_id: &str,
    files: &[MediaDedupeCatalogEntryOwned],
) -> Result<(), String> {
    if files.is_empty() {
        return Ok(());
    }
    with_workspace(|connection, _| {
        let now = Utc::now().to_rfc3339();
        connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|error| error.to_string())?;
        for file in files {
            let result = connection.execute(
                "INSERT INTO media_dedupe_catalog (
                    normalized_path, path, source_id, provider, root_path, volume_key,
                    media_type, size_bytes, modified_at_ms, hash_status,
                    last_seen_scan_id, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'pending', ?10, ?11)
                 ON CONFLICT(normalized_path) DO UPDATE SET
                    path = excluded.path,
                    source_id = excluded.source_id,
                    provider = excluded.provider,
                    root_path = excluded.root_path,
                    volume_key = excluded.volume_key,
                    media_type = excluded.media_type,
                    sha256 = CASE
                        WHEN media_dedupe_catalog.size_bytes = excluded.size_bytes
                         AND media_dedupe_catalog.modified_at_ms = excluded.modified_at_ms
                        THEN media_dedupe_catalog.sha256 ELSE NULL END,
                    width = CASE
                        WHEN media_dedupe_catalog.size_bytes = excluded.size_bytes
                         AND media_dedupe_catalog.modified_at_ms = excluded.modified_at_ms
                        THEN media_dedupe_catalog.width ELSE NULL END,
                    height = CASE
                        WHEN media_dedupe_catalog.size_bytes = excluded.size_bytes
                         AND media_dedupe_catalog.modified_at_ms = excluded.modified_at_ms
                        THEN media_dedupe_catalog.height ELSE NULL END,
                    duration_ms = CASE
                        WHEN media_dedupe_catalog.size_bytes = excluded.size_bytes
                         AND media_dedupe_catalog.modified_at_ms = excluded.modified_at_ms
                        THEN media_dedupe_catalog.duration_ms ELSE NULL END,
                    ahash64 = CASE
                        WHEN media_dedupe_catalog.size_bytes = excluded.size_bytes
                         AND media_dedupe_catalog.modified_at_ms = excluded.modified_at_ms
                        THEN media_dedupe_catalog.ahash64 ELSE NULL END,
                    dhash64 = CASE
                        WHEN media_dedupe_catalog.size_bytes = excluded.size_bytes
                         AND media_dedupe_catalog.modified_at_ms = excluded.modified_at_ms
                        THEN media_dedupe_catalog.dhash64 ELSE NULL END,
                    hash_status = CASE
                        WHEN media_dedupe_catalog.size_bytes = excluded.size_bytes
                         AND media_dedupe_catalog.modified_at_ms = excluded.modified_at_ms
                         AND media_dedupe_catalog.sha256 IS NOT NULL
                        THEN 'complete' ELSE 'pending' END,
                    size_bytes = excluded.size_bytes,
                    modified_at_ms = excluded.modified_at_ms,
                    last_seen_scan_id = excluded.last_seen_scan_id,
                    updated_at = excluded.updated_at",
                params![
                    file.normalized_path,
                    file.path.to_string_lossy(),
                    file.source_id,
                    file.provider,
                    file.root_path.to_string_lossy(),
                    file.volume_key,
                    file.media_type,
                    file.size_bytes as i64,
                    file.modified_at_ms,
                    scan_id,
                    now,
                ],
            );
            if let Err(error) = result {
                let _ = connection.execute_batch("ROLLBACK");
                return Err(error.to_string());
            }
        }
        connection
            .execute_batch("COMMIT")
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

pub(crate) fn media_dedupe_catalog_stats(scan_id: &str) -> Result<MediaDedupeCatalogStats, String> {
    with_workspace(|connection, _| {
        let (files, bytes) = connection
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0)
                 FROM media_dedupe_catalog WHERE last_seen_scan_id = ?1",
                params![scan_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .map_err(|error| error.to_string())?;
        let (candidate_files, candidate_bytes) = connection
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(candidate.size_bytes), 0)
                 FROM media_dedupe_catalog candidate
                 WHERE candidate.last_seen_scan_id = ?1
                   AND EXISTS (
                     SELECT 1 FROM media_dedupe_catalog other
                     WHERE other.last_seen_scan_id = ?1
                       AND other.volume_key = candidate.volume_key
                       AND other.size_bytes = candidate.size_bytes
                       AND other.normalized_path <> candidate.normalized_path
                   )",
                params![scan_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .map_err(|error| error.to_string())?;
        Ok(MediaDedupeCatalogStats {
            files: files.max(0) as u64,
            bytes: bytes.max(0) as u64,
            candidate_files: candidate_files.max(0) as u64,
            candidate_bytes: candidate_bytes.max(0) as u64,
        })
    })
}

pub(crate) fn media_dedupe_video_count(scan_id: &str) -> Result<u32, String> {
    with_workspace(|connection, _| {
        connection
            .query_row(
                "SELECT COUNT(*) FROM media_dedupe_catalog
                 WHERE last_seen_scan_id = ?1 AND media_type = 'video'",
                params![scan_id],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value.max(0).min(i64::from(u32::MAX)) as u32)
            .map_err(|error| error.to_string())
    })
}

pub(crate) fn media_dedupe_source_video_inventory(
    scan_id: &str,
    source_id: &str,
) -> Result<MediaDedupeSourceVideoInventory, String> {
    with_workspace(|connection, _| {
        let mut statement = connection
            .prepare(
                "SELECT normalized_path, size_bytes, modified_at_ms
                 FROM media_dedupe_catalog
                 WHERE last_seen_scan_id = ?1 AND source_id = ?2 AND media_type = 'video'
                 ORDER BY normalized_path",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![scan_id, source_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?.max(0) as u64,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        let mut hasher = Sha256::new();
        let mut video_count = 0u32;
        for row in rows {
            let (path, size, modified_at_ms) = row.map_err(|error| error.to_string())?;
            hasher.update(path.as_bytes());
            hasher.update([0]);
            hasher.update(size.to_le_bytes());
            hasher.update(modified_at_ms.to_le_bytes());
            video_count = video_count.saturating_add(1);
        }
        Ok(MediaDedupeSourceVideoInventory {
            fingerprint: format!("{:x}", hasher.finalize()),
            video_count,
        })
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn begin_media_dedupe_source_job(
    id: &str,
    scan_id: &str,
    source_id: &str,
    provider: &str,
    source_path: &Path,
    runtime_version: &str,
    runtime_digest: &str,
    settings_fingerprint: &str,
    inventory_fingerprint: &str,
    database_path: &Path,
    result_path: &Path,
) -> Result<(), String> {
    with_workspace(|connection, _| {
        let now = Utc::now().to_rfc3339();
        connection
            .execute(
                "INSERT INTO media_dedupe_source_jobs (
                    id, scan_id, source_id, provider, source_path, status, stage,
                    runtime_version, runtime_digest, settings_fingerprint,
                    inventory_fingerprint, database_path, result_path, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 'queued', 'queued', ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(scan_id, source_id) DO UPDATE SET
                    status = 'queued', stage = 'queued', progress_percent = NULL,
                    files_processed = 0, files_total = 0, current_path = NULL,
                    runtime_version = excluded.runtime_version,
                    runtime_digest = excluded.runtime_digest,
                    settings_fingerprint = excluded.settings_fingerprint,
                    inventory_fingerprint = excluded.inventory_fingerprint,
                    database_path = excluded.database_path,
                    result_path = excluded.result_path,
                    cached_from_scan_id = NULL, error = NULL,
                    started_at = NULL, finished_at = NULL,
                    updated_at = excluded.updated_at",
                params![
                    id,
                    scan_id,
                    source_id,
                    provider,
                    source_path.to_string_lossy(),
                    runtime_version,
                    runtime_digest,
                    settings_fingerprint,
                    inventory_fingerprint,
                    database_path.to_string_lossy(),
                    result_path.to_string_lossy(),
                    now,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

pub(crate) fn find_reusable_media_dedupe_source_scan(
    scan_id: &str,
    source_id: &str,
    runtime_digest: &str,
    settings_fingerprint: &str,
    inventory_fingerprint: &str,
) -> Result<Option<String>, String> {
    with_workspace(|connection, _| {
        connection
            .query_row(
                "SELECT job.scan_id
                 FROM media_dedupe_source_jobs job
                 JOIN media_dedupe_scans scan ON scan.id = job.scan_id
                 WHERE job.scan_id <> ?1
                   AND job.source_id = ?2
                   AND job.status = 'completed'
                   AND scan.status = 'completed'
                   AND job.runtime_digest = ?3
                   AND job.settings_fingerprint = ?4
                   AND job.inventory_fingerprint = ?5
                 ORDER BY COALESCE(job.finished_at, job.updated_at) DESC
                 LIMIT 1",
                params![
                    scan_id,
                    source_id,
                    runtime_digest,
                    settings_fingerprint,
                    inventory_fingerprint,
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())
    })
}

pub(crate) fn reuse_media_dedupe_vdf_candidates(
    scan_id: &str,
    source_id: &str,
    previous_scan_id: &str,
) -> Result<(), String> {
    with_workspace(|connection, _| {
        let now = Utc::now().to_rfc3339();
        connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|error| error.to_string())?;
        let copy = connection.execute(
            "INSERT INTO media_dedupe_vdf_candidates (
                scan_id, source_id, group_id, path, normalized_path,
                similarity_percent, size_bytes, duration_ms, width, height,
                runtime_version, runtime_digest, settings_fingerprint, imported_at
             )
             SELECT ?1, source_id, group_id, path, normalized_path,
                    similarity_percent, size_bytes, duration_ms, width, height,
                    runtime_version, runtime_digest, settings_fingerprint, ?4
             FROM media_dedupe_vdf_candidates
             WHERE scan_id = ?3 AND source_id = ?2",
            params![scan_id, source_id, previous_scan_id, now],
        );
        if let Err(error) = copy {
            let _ = connection.execute_batch("ROLLBACK");
            return Err(error.to_string());
        }
        if let Err(error) = connection.execute(
            "UPDATE media_dedupe_source_jobs
             SET cached_from_scan_id = ?3
             WHERE scan_id = ?1 AND source_id = ?2",
            params![scan_id, source_id, previous_scan_id],
        ) {
            let _ = connection.execute_batch("ROLLBACK");
            return Err(error.to_string());
        }
        connection
            .execute_batch("COMMIT")
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_media_dedupe_source_job(
    scan_id: &str,
    source_id: &str,
    status: &str,
    stage: &str,
    progress_percent: Option<u32>,
    files_processed: u64,
    files_total: u64,
    current_path: Option<&str>,
    error: Option<&str>,
) -> Result<(), String> {
    with_workspace(|connection, _| {
        let now = Utc::now().to_rfc3339();
        let terminal = matches!(status, "completed" | "failed" | "cancelled");
        connection
            .execute(
                "UPDATE media_dedupe_source_jobs
                 SET status = ?3, stage = ?4, progress_percent = ?5,
                     files_processed = ?6, files_total = ?7, current_path = ?8,
                     error = ?9,
                     started_at = CASE WHEN started_at IS NULL AND ?3 = 'running' THEN ?10 ELSE started_at END,
                     finished_at = CASE WHEN ?11 THEN ?10 ELSE finished_at END,
                     updated_at = ?10
                 WHERE scan_id = ?1 AND source_id = ?2",
                params![
                    scan_id,
                    source_id,
                    status,
                    stage,
                    progress_percent.map(i64::from),
                    files_processed as i64,
                    files_total as i64,
                    current_path,
                    error,
                    now,
                    terminal,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

pub(crate) fn load_media_dedupe_source_jobs(
    scan_id: &str,
) -> Result<Vec<MediaDedupeSourceJobStatus>, String> {
    with_workspace(|connection, _| {
        let mut statement = connection
            .prepare(
                "SELECT source_id, provider, source_path, status, stage,
                        progress_percent, files_processed, files_total, current_path, error
                 FROM media_dedupe_source_jobs
                 WHERE scan_id = ?1
                 ORDER BY CASE status
                    WHEN 'running' THEN 0 WHEN 'failed' THEN 1 WHEN 'queued' THEN 2 ELSE 3 END,
                    updated_at DESC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![scan_id], |row| {
                Ok(MediaDedupeSourceJobStatus {
                    source_id: row.get(0)?,
                    provider: row.get(1)?,
                    source_path: row.get(2)?,
                    status: row.get(3)?,
                    stage: row.get(4)?,
                    progress_percent: row
                        .get::<_, Option<i64>>(5)?
                        .map(|value| value.clamp(0, 100) as u32),
                    files_processed: row.get::<_, i64>(6)?.max(0) as u64,
                    files_total: row.get::<_, i64>(7)?.max(0) as u64,
                    current_path: row.get(8)?,
                    error: row.get(9)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn replace_media_dedupe_vdf_candidates(
    scan_id: &str,
    source_id: &str,
    runtime_version: &str,
    runtime_digest: &str,
    settings_fingerprint: &str,
    candidates: &[MediaDedupeVdfCandidateOwned],
) -> Result<(), String> {
    with_workspace(|connection, _| {
        connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|error| error.to_string())?;
        if let Err(error) = connection.execute(
            "DELETE FROM media_dedupe_vdf_candidates WHERE scan_id = ?1 AND source_id = ?2",
            params![scan_id, source_id],
        ) {
            let _ = connection.execute_batch("ROLLBACK");
            return Err(error.to_string());
        }
        let now = Utc::now().to_rfc3339();
        for candidate in candidates {
            if let Err(error) = connection.execute(
                "INSERT INTO media_dedupe_vdf_candidates (
                    scan_id, source_id, group_id, path, normalized_path,
                    similarity_percent, size_bytes, duration_ms, width, height,
                    runtime_version, runtime_digest, settings_fingerprint, imported_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    scan_id,
                    source_id,
                    candidate.group_id,
                    candidate.path,
                    candidate.normalized_path,
                    candidate.similarity_percent,
                    candidate.size_bytes as i64,
                    candidate.duration_ms.map(|value| value as i64),
                    candidate.width.map(i64::from),
                    candidate.height.map(i64::from),
                    runtime_version,
                    runtime_digest,
                    settings_fingerprint,
                    now,
                ],
            ) {
                let _ = connection.execute_batch("ROLLBACK");
                return Err(error.to_string());
            }
        }
        connection
            .execute_batch("COMMIT")
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

pub(crate) fn load_media_dedupe_catalog_candidates(
    scan_id: &str,
    after_path: Option<&str>,
    limit: u32,
) -> Result<Vec<MediaDedupeCatalogCandidate>, String> {
    with_workspace(|connection, _| {
        let mut statement = connection
            .prepare(
                "SELECT candidate.path, candidate.normalized_path, candidate.source_id,
                        candidate.provider, candidate.root_path, candidate.volume_key,
                        candidate.media_type, candidate.size_bytes, candidate.modified_at_ms,
                        candidate.sha256, candidate.width, candidate.height,
                        candidate.duration_ms, candidate.ahash64, candidate.dhash64
                 FROM media_dedupe_catalog candidate
                 WHERE candidate.last_seen_scan_id = ?1
                   AND candidate.normalized_path > COALESCE(?2, '')
                   AND EXISTS (
                     SELECT 1 FROM media_dedupe_catalog other
                     WHERE other.last_seen_scan_id = ?1
                       AND other.volume_key = candidate.volume_key
                       AND other.size_bytes = candidate.size_bytes
                       AND other.normalized_path <> candidate.normalized_path
                   )
                 ORDER BY candidate.normalized_path
                 LIMIT ?3",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![scan_id, after_path, i64::from(limit)], |row| {
                Ok(MediaDedupeCatalogCandidate {
                    path: PathBuf::from(row.get::<_, String>(0)?),
                    normalized_path: row.get(1)?,
                    source_id: row.get(2)?,
                    provider: row.get(3)?,
                    root_path: PathBuf::from(row.get::<_, String>(4)?),
                    volume_key: row.get(5)?,
                    media_type: row.get(6)?,
                    size_bytes: row.get::<_, i64>(7)?.max(0) as u64,
                    modified_at_ms: row.get(8)?,
                    sha256: row.get(9)?,
                    width: row
                        .get::<_, Option<i64>>(10)?
                        .map(|value| value.max(0) as u32),
                    height: row
                        .get::<_, Option<i64>>(11)?
                        .map(|value| value.max(0) as u32),
                    duration_ms: row
                        .get::<_, Option<i64>>(12)?
                        .map(|value| value.max(0) as u64),
                    ahash64: row.get(13)?,
                    dhash64: row.get(14)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    })
}

pub(crate) fn update_media_dedupe_catalog_hash(
    scan_id: &str,
    file: &MediaDedupeIndexedFileOwned,
) -> Result<(), String> {
    with_workspace(|connection, _| {
        let now = Utc::now().to_rfc3339();
        connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|error| error.to_string())?;
        let updated = connection.execute(
            "UPDATE media_dedupe_catalog
             SET sha256 = ?3, width = ?4, height = ?5, duration_ms = ?6,
                 ahash64 = ?7, dhash64 = ?8, hash_status = 'complete',
                 last_hashed_at = ?9, updated_at = ?9
             WHERE normalized_path = ?1 AND last_seen_scan_id = ?2
               AND size_bytes = ?10 AND modified_at_ms = ?11",
            params![
                file.normalized_path,
                scan_id,
                file.sha256,
                file.width.map(i64::from),
                file.height.map(i64::from),
                file.duration_ms.map(|value| value as i64),
                file.ahash64,
                file.dhash64,
                now,
                file.size_bytes as i64,
                file.modified_at_ms,
            ],
        );
        let updated = match updated {
            Ok(value) => value,
            Err(error) => {
                let _ = connection.execute_batch("ROLLBACK");
                return Err(error.to_string());
            }
        };
        if updated == 0 {
            let _ = connection.execute_batch("ROLLBACK");
            return Err(
                "Media changed after inventory; it will be retried on the next scan.".to_string(),
            );
        }
        if let Err(error) = insert_media_dedupe_file(connection, &file.borrowed()) {
            let _ = connection.execute_batch("ROLLBACK");
            return Err(error);
        }
        connection
            .execute_batch("COMMIT")
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

fn insert_media_dedupe_file(
    connection: &Connection,
    file: &MediaDedupeIndexedFile<'_>,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO media_dedupe_files (
                    scan_id, path, normalized_path, source_id, provider, root_path,
                    volume_key, media_type, size_bytes, modified_at_ms, sha256,
                    width, height, duration_ms, ahash64, dhash64, video_hashes_json
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                    ?12, ?13, ?14, ?15, ?16, ?17
                 )
                 ON CONFLICT(scan_id, normalized_path) DO UPDATE SET
                    source_id = excluded.source_id,
                    provider = excluded.provider,
                    root_path = excluded.root_path,
                    volume_key = excluded.volume_key,
                    media_type = excluded.media_type,
                    size_bytes = excluded.size_bytes,
                    modified_at_ms = excluded.modified_at_ms,
                    sha256 = excluded.sha256,
                    width = excluded.width,
                    height = excluded.height,
                    duration_ms = excluded.duration_ms,
                    ahash64 = excluded.ahash64,
                    dhash64 = excluded.dhash64,
                    video_hashes_json = excluded.video_hashes_json",
            params![
                file.scan_id,
                file.path.to_string_lossy(),
                file.normalized_path,
                file.source_id,
                file.provider,
                file.root_path.to_string_lossy(),
                file.volume_key,
                file.media_type,
                file.size_bytes as i64,
                file.modified_at_ms,
                file.sha256,
                file.width.map(i64::from),
                file.height.map(i64::from),
                file.duration_ms.map(|value| value as i64),
                file.ahash64,
                file.dhash64,
                file.video_hashes_json,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn complete_media_dedupe_scan(
    scan_id: &str,
    skipped_video_similarity_count: u32,
) -> Result<MediaDedupeScanResult, String> {
    let result = load_media_dedupe_scan_result(scan_id)?;
    with_workspace(|connection, _| {
        let now = Utc::now().to_rfc3339();
        connection
            .execute(
                "UPDATE media_dedupe_scans
                 SET status = 'completed',
                     stage = 'completed',
                     exact_group_count = ?2,
                     similar_group_count = ?3,
                     reclaimable_bytes = ?4,
                     skipped_video_similarity_count = ?5,
                     current_path = NULL,
                     finished_at = ?6,
                     updated_at = ?6
                 WHERE id = ?1",
                params![
                    scan_id,
                    result.exact_group_count as i64,
                    result.similar_group_count as i64,
                    result.reclaimable_bytes as i64,
                    skipped_video_similarity_count as i64,
                    now,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })?;
    load_media_dedupe_scan_result(scan_id)
}

pub(crate) fn set_media_dedupe_inventory_totals(
    scan_id: &str,
    files_scanned: u64,
    bytes_scanned: u64,
) -> Result<MediaDedupeScanResult, String> {
    with_workspace(|connection, _| {
        connection
            .execute(
                "UPDATE media_dedupe_scans
                 SET files_scanned = ?2, bytes_scanned = ?3, updated_at = ?4
                 WHERE id = ?1",
                params![
                    scan_id,
                    files_scanned as i64,
                    bytes_scanned as i64,
                    Utc::now().to_rfc3339(),
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })?;
    load_media_dedupe_scan_result(scan_id)
}

pub(crate) fn finish_media_dedupe_scan_with_error(
    scan_id: &str,
    status: &str,
    error: Option<&str>,
) -> Result<(), String> {
    with_workspace(|connection, _| {
        let now = Utc::now().to_rfc3339();
        connection
            .execute(
                "UPDATE media_dedupe_scans
                 SET status = ?2,
                     stage = ?2,
                     error = ?3,
                     current_path = NULL,
                     finished_at = ?4,
                     updated_at = ?4
                 WHERE id = ?1",
                params![scan_id, status, error, now],
            )
            .map_err(|value| value.to_string())?;
        Ok(())
    })
}

pub(crate) fn load_latest_media_dedupe_scan() -> Result<Option<MediaDedupeScanResult>, String> {
    let scan_id = with_workspace(|connection, _| {
        connection
            .query_row(
                "SELECT id FROM media_dedupe_scans
                 WHERE status = 'completed'
                 ORDER BY COALESCE(finished_at, started_at) DESC
                 LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())
    })?;
    scan_id
        .map(|value| load_media_dedupe_scan_result(&value))
        .transpose()
}

pub(crate) fn load_media_dedupe_scan_result(
    scan_id: &str,
) -> Result<MediaDedupeScanResult, String> {
    with_workspace(|connection, _| {
        let (
            status,
            provider_scope,
            source_scope,
            resource_profile,
            files_scanned,
            bytes_scanned,
            skipped_video_similarity_count,
            started_at,
            finished_at,
        ) = connection
            .query_row(
                "SELECT status, provider_scope, source_scope, resource_profile, files_scanned, bytes_scanned,
                        skipped_video_similarity_count, started_at, finished_at
                 FROM media_dedupe_scans WHERE id = ?1",
                params![scan_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?.max(0) as u64,
                        row.get::<_, i64>(5)?.max(0) as u64,
                        row.get::<_, i64>(6)?.max(0) as u32,
                        row.get::<_, String>(7)?,
                        row.get::<_, Option<String>>(8)?,
                    ))
                },
            )
            .map_err(|error| error.to_string())?;
        let files = load_indexed_files(connection, scan_id)?;
        let exact_groups = exact_groups(&files);
        let mut similar_groups = similar_groups(&files);
        similar_groups.extend(vdf_similar_groups(connection, scan_id)?);
        similar_groups.sort_by(|left, right| right.reclaimable_bytes.cmp(&left.reclaimable_bytes));
        let reclaimable_bytes = exact_groups
            .iter()
            .map(|group| group.reclaimable_bytes)
            .sum();
        Ok(MediaDedupeScanResult {
            scan_id: scan_id.to_string(),
            provider_scope,
            source_scope,
            resource_profile,
            similarity_scope: "source".to_string(),
            status,
            files_scanned,
            bytes_scanned,
            exact_group_count: exact_groups.len() as u32,
            similar_group_count: similar_groups.len() as u32,
            reclaimable_bytes,
            skipped_video_similarity_count,
            started_at,
            finished_at,
            exact_groups,
            similar_groups,
        })
    })
}

fn vdf_similar_groups(
    connection: &Connection,
    scan_id: &str,
) -> Result<Vec<MediaDedupeGroup>, String> {
    let mut statement = connection
        .prepare(
            "SELECT candidate.source_id, source.provider, candidate.group_id,
                    candidate.path, candidate.similarity_percent, candidate.size_bytes,
                    candidate.duration_ms, candidate.width, candidate.height
             FROM media_dedupe_vdf_candidates candidate
             JOIN source_profiles source ON source.id = candidate.source_id
             WHERE candidate.scan_id = ?1
             ORDER BY candidate.source_id, candidate.group_id, candidate.normalized_path",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![scan_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, i64>(5)?.max(0) as u64,
                row.get::<_, Option<i64>>(6)?
                    .map(|value| value.max(0) as u64),
                row.get::<_, Option<i64>>(7)?
                    .map(|value| value.max(0) as u32),
                row.get::<_, Option<i64>>(8)?
                    .map(|value| value.max(0) as u32),
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut grouped = BTreeMap::<
        (String, String, String),
        Vec<(String, f64, u64, Option<u64>, Option<u32>, Option<u32>)>,
    >::new();
    for row in rows {
        let (source_id, provider, group_id, path, similarity, size, duration, width, height) =
            row.map_err(|error| error.to_string())?;
        grouped
            .entry((source_id, provider, group_id))
            .or_default()
            .push((path, similarity, size, duration, width, height));
    }
    let mut output = Vec::new();
    for ((source_id, provider, group_id), items) in grouped {
        if items.len() < 2 {
            continue;
        }
        let reclaimable = items
            .iter()
            .map(|item| item.2)
            .sum::<u64>()
            .saturating_sub(items.iter().map(|item| item.2).max().unwrap_or(0));
        let confidence = items
            .iter()
            .map(|item| item.1.clamp(0.0, 100.0).round() as u32)
            .min();
        let files = items
            .into_iter()
            .map(|(path, _, size_bytes, duration_ms, width, height)| {
                let media_type = Path::new(&path)
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| {
                        matches!(
                            value.to_ascii_lowercase().as_str(),
                            "jpg" | "jpeg" | "png" | "webp"
                        )
                    })
                    .then_some("image")
                    .unwrap_or("video")
                    .to_string();
                MediaDedupeFile {
                    path,
                    source_id: Some(source_id.clone()),
                    provider: Some(provider.clone()),
                    media_type,
                    size_bytes,
                    width,
                    height,
                    duration_ms,
                }
            })
            .collect();
        output.push(MediaDedupeGroup {
            id: format!("vdf:{source_id}:{group_id}"),
            kind: "similar".to_string(),
            confidence_percent: confidence,
            reclaimable_bytes: reclaimable,
            consolidatable: false,
            reason: Some(
                "Video Duplicate Finder candidate. Review before moving files to the Recycle Bin."
                    .to_string(),
            ),
            files,
        });
    }
    Ok(output)
}

pub(crate) fn media_dedupe_file_signature(
    scan_id: &str,
    path: &str,
) -> Result<Option<(u64, String)>, String> {
    with_workspace(|connection, _| {
        connection
            .query_row(
                "SELECT size_bytes, sha256
                 FROM media_dedupe_files
                 WHERE scan_id = ?1 AND path = ?2",
                params![scan_id, path],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?.max(0) as u64,
                        row.get::<_, String>(1)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())
    })
}

pub(crate) fn recover_interrupted_media_dedupe_jobs() -> Result<(), String> {
    with_workspace(|connection, _| {
        let now = Utc::now().to_rfc3339();
        connection
            .execute(
                "UPDATE media_dedupe_scans
                 SET status = 'interrupted', stage = 'interrupted',
                     error = 'The app closed before this scan completed.',
                     finished_at = ?1, updated_at = ?1
                 WHERE status = 'running'",
                params![now],
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE media_dedupe_jobs
                 SET status = 'interrupted', stage = 'interrupted',
                     error = 'The app closed before this cleanup job completed.',
                     finished_at = ?1, updated_at = ?1
                 WHERE status IN ('queued', 'scanning', 'applying')",
                params![now],
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE media_dedupe_actions
                 SET status = 'interrupted',
                     error = 'The app closed before this action completed.',
                     finished_at = ?1
                 WHERE status = 'running'",
                params![now],
            )
            .map_err(|error| error.to_string())?;
        let interrupted_hardlinks = {
            let mut statement = connection
                .prepare(
                    "SELECT id, target_path FROM media_dedupe_actions
                     WHERE action_kind = 'hardlink' AND status = 'interrupted'
                       AND target_path IS NOT NULL",
                )
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|error| error.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?
        };
        for (action_id, target_value) in interrupted_hardlinks {
            let target = PathBuf::from(&target_value);
            let Some(parent) = target.parent() else {
                continue;
            };
            let Some(file_name) = target.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let prefix = format!(".{file_name}.ninjacrawler-dedupe-");
            let backup = fs::read_dir(parent)
                .ok()
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path())
                .find(|path| {
                    path.file_name()
                        .and_then(|value| value.to_str())
                        .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".backup"))
                });
            let Some(backup) = backup else {
                continue;
            };
            let backup_size = fs::metadata(&backup).map(|value| value.len()).unwrap_or(0);
            let recovery = if !target.exists() {
                fs::rename(&backup, &target).map(|_| "rolled_back")
            } else if dedupe_files_match(&backup, &target) {
                fs::remove_file(&backup).map(|_| "succeeded")
            } else {
                continue;
            };
            if let Ok(status) = recovery {
                connection
                    .execute(
                        "UPDATE media_dedupe_actions
                         SET status = ?2,
                             bytes_reclaimed = CASE WHEN ?2 = 'succeeded' THEN ?3 ELSE 0 END,
                             error = NULL, finished_at = ?4
                         WHERE id = ?1",
                        params![action_id, status, backup_size as i64, now],
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    })
}

pub(crate) fn begin_media_dedupe_action(
    action_id: &str,
    scan_id: &str,
    action_kind: &str,
    canonical_path: Option<&str>,
    target_path: Option<&str>,
) -> Result<(), String> {
    with_workspace(|connection, _| {
        connection
            .execute(
                "INSERT INTO media_dedupe_actions (
                    id, scan_id, action_kind, status, canonical_path,
                    target_path, started_at
                 ) VALUES (?1, ?2, ?3, 'running', ?4, ?5, ?6)",
                params![
                    action_id,
                    scan_id,
                    action_kind,
                    canonical_path,
                    target_path,
                    Utc::now().to_rfc3339(),
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

pub(crate) fn finish_media_dedupe_action(
    action_id: &str,
    status: &str,
    bytes_reclaimed: u64,
    error: Option<&str>,
) -> Result<(), String> {
    with_workspace(|connection, _| {
        connection
            .execute(
                "UPDATE media_dedupe_actions
                 SET status = ?2, bytes_reclaimed = ?3, error = ?4, finished_at = ?5
                 WHERE id = ?1",
                params![
                    action_id,
                    status,
                    bytes_reclaimed as i64,
                    error,
                    Utc::now().to_rfc3339(),
                ],
            )
            .map_err(|value| value.to_string())?;
        Ok(())
    })
}

fn load_indexed_files(
    connection: &Connection,
    scan_id: &str,
) -> Result<Vec<IndexedFileRow>, String> {
    let mut statement = connection
        .prepare(
            "SELECT path, source_id, provider, volume_key, media_type, size_bytes,
                    width, height, duration_ms, sha256, ahash64, dhash64, video_hashes_json
             FROM media_dedupe_files
             WHERE scan_id = ?1
             ORDER BY normalized_path",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![scan_id], |row| {
            let video_hashes_json = row.get::<_, Option<String>>(12)?;
            Ok(IndexedFileRow {
                path: row.get(0)?,
                source_id: row.get(1)?,
                provider: row.get(2)?,
                volume_key: row.get(3)?,
                media_type: row.get(4)?,
                size_bytes: row.get::<_, i64>(5)?.max(0) as u64,
                width: row
                    .get::<_, Option<i64>>(6)?
                    .map(|value| value.max(0) as u32),
                height: row
                    .get::<_, Option<i64>>(7)?
                    .map(|value| value.max(0) as u32),
                duration_ms: row
                    .get::<_, Option<i64>>(8)?
                    .map(|value| value.max(0) as u64),
                sha256: row.get(9)?,
                ahash64: row.get(10)?,
                dhash64: row.get(11)?,
                video_hashes: video_hashes_json
                    .as_deref()
                    .and_then(|value| serde_json::from_str(value).ok())
                    .unwrap_or_default(),
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn exact_groups(files: &[IndexedFileRow]) -> Vec<MediaDedupeGroup> {
    let mut grouped = BTreeMap::<(String, u64, String), Vec<&IndexedFileRow>>::new();
    for file in files {
        grouped
            .entry((
                file.volume_key.clone(),
                file.size_bytes,
                file.sha256.clone(),
            ))
            .or_default()
            .push(file);
    }
    let mut output = Vec::new();
    for ((volume, size, sha), group) in grouped {
        if group.len() < 2 {
            continue;
        }
        let mut unique = Vec::<&IndexedFileRow>::new();
        for file in group {
            let is_existing_link = unique
                .iter()
                .any(|known| same_file::is_same_file(&known.path, &file.path).unwrap_or(false));
            if !is_existing_link {
                unique.push(file);
            }
        }
        if unique.len() < 2 {
            continue;
        }
        let consolidatable = hardlinks_likely_supported(&volume);
        output.push(MediaDedupeGroup {
            id: format!("exact:{volume}:{sha}"),
            kind: "exact".to_string(),
            confidence_percent: Some(100),
            reclaimable_bytes: if consolidatable {
                size.saturating_mul((unique.len() - 1) as u64)
            } else {
                0
            },
            consolidatable,
            reason: (!consolidatable).then(|| {
                "This volume does not report hardlink support; all paths will be left unchanged."
                    .to_string()
            }),
            files: unique.into_iter().map(media_file).collect(),
        });
    }
    output.sort_by(|left, right| right.reclaimable_bytes.cmp(&left.reclaimable_bytes));
    output
}

fn hardlinks_likely_supported(volume: &str) -> bool {
    #[cfg(windows)]
    {
        if volume.starts_with("\\\\") {
            return false;
        }
        static SUPPORT: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
        let cache = SUPPORT.get_or_init(|| Mutex::new(HashMap::new()));
        if let Ok(cache) = cache.lock() {
            if let Some(supported) = cache.get(volume) {
                return *supported;
            }
        }
        let drive_letter = volume.trim_end_matches(':');
        let script = format!("(Get-Volume -DriveLetter '{drive_letter}').FileSystem");
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        command.creation_flags(0x08000000);
        let supported = command
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .to_ascii_uppercase()
            })
            .is_some_and(|file_system| file_system == "NTFS" || file_system == "REFS");
        if let Ok(mut cache) = cache.lock() {
            cache.insert(volume.to_string(), supported);
        }
        return supported;
    }
    #[cfg(not(windows))]
    {
        let _ = volume;
        true
    }
}

fn similar_groups(files: &[IndexedFileRow]) -> Vec<MediaDedupeGroup> {
    let candidates = files
        .iter()
        .filter(|file| {
            file.source_id.is_some()
                && ((file.media_type == "image"
                    && file.ahash64.is_some()
                    && file.dhash64.is_some())
                    || (file.media_type == "video" && !file.video_hashes.is_empty()))
        })
        .collect::<Vec<_>>();
    let mut consumed = HashSet::<String>::new();
    let mut output = Vec::new();
    for file in &candidates {
        if consumed.contains(&file.path) {
            continue;
        }
        let mut group = vec![*file];
        for other in &candidates {
            if file.path == other.path
                || consumed.contains(&other.path)
                || file.source_id != other.source_id
                || file.media_type != other.media_type
                || file.sha256 == other.sha256
            {
                continue;
            }
            if perceptually_similar(file, other) {
                group.push(*other);
            }
        }
        if group.len() < 2 {
            continue;
        }
        for item in &group {
            consumed.insert(item.path.clone());
        }
        let confidence = group
            .iter()
            .skip(1)
            .filter_map(|item| similarity_confidence(file, item))
            .min()
            .unwrap_or(80);
        let reclaimable = group
            .iter()
            .map(|item| item.size_bytes)
            .sum::<u64>()
            .saturating_sub(group.iter().map(|item| item.size_bytes).max().unwrap_or(0));
        output.push(MediaDedupeGroup {
            id: format!(
                "similar:{}:{}",
                file.source_id.as_deref().unwrap_or("unknown"),
                output.len()
            ),
            kind: "similar".to_string(),
            confidence_percent: Some(confidence),
            reclaimable_bytes: reclaimable,
            consolidatable: false,
            reason: Some("Review required before moving files to the Recycle Bin.".to_string()),
            files: group.into_iter().map(media_file).collect(),
        });
    }
    output.sort_by(|left, right| right.reclaimable_bytes.cmp(&left.reclaimable_bytes));
    output
}

fn media_file(file: &IndexedFileRow) -> MediaDedupeFile {
    MediaDedupeFile {
        path: file.path.clone(),
        source_id: file.source_id.clone(),
        provider: file.provider.clone(),
        media_type: file.media_type.clone(),
        size_bytes: file.size_bytes,
        width: file.width,
        height: file.height,
        duration_ms: file.duration_ms,
    }
}

fn perceptually_similar(left: &IndexedFileRow, right: &IndexedFileRow) -> bool {
    if left.media_type == "image" {
        let aspect_close = match (left.width, left.height, right.width, right.height) {
            (Some(lw), Some(lh), Some(rw), Some(rh)) if lh > 0 && rh > 0 => {
                let left_aspect = lw as f64 / lh as f64;
                let right_aspect = rw as f64 / rh as f64;
                ((left_aspect - right_aspect).abs() / left_aspect.max(right_aspect)) <= 0.01
            }
            _ => false,
        };
        return aspect_close
            && hash_distance(left.ahash64.as_deref(), right.ahash64.as_deref()) <= 6
            && hash_distance(left.dhash64.as_deref(), right.dhash64.as_deref()) <= 6;
    }
    let duration_close = match (left.duration_ms, right.duration_ms) {
        (Some(left), Some(right)) if left.max(right) > 0 => {
            left.abs_diff(right) as f64 / left.max(right) as f64 <= 0.02
        }
        _ => false,
    };
    duration_close
        && left
            .video_hashes
            .iter()
            .zip(&right.video_hashes)
            .filter(|((la, ld), (ra, rd))| {
                hash_distance(Some(la), Some(ra)) <= 6 && hash_distance(Some(ld), Some(rd)) <= 6
            })
            .count()
            >= 2
}

fn similarity_confidence(left: &IndexedFileRow, right: &IndexedFileRow) -> Option<u32> {
    if left.media_type == "image" {
        let distance = hash_distance(left.ahash64.as_deref(), right.ahash64.as_deref())
            + hash_distance(left.dhash64.as_deref(), right.dhash64.as_deref());
        return Some(100u32.saturating_sub(distance.saturating_mul(3)));
    }
    let matched = left
        .video_hashes
        .iter()
        .zip(&right.video_hashes)
        .filter(|((la, ld), (ra, rd))| {
            hash_distance(Some(la), Some(ra)) <= 6 && hash_distance(Some(ld), Some(rd)) <= 6
        })
        .count() as u32;
    Some(70 + matched.saturating_mul(10).min(30))
}

fn hash_distance(left: Option<&str>, right: Option<&str>) -> u32 {
    let (Ok(left), Ok(right)) = (
        u64::from_str_radix(left.unwrap_or_default(), 16),
        u64::from_str_radix(right.unwrap_or_default(), 16),
    ) else {
        return 64;
    };
    (left ^ right).count_ones()
}

#[cfg(test)]
mod dedupe_repository_tests {
    use super::*;

    #[test]
    fn perceptual_hash_distance_is_hamming_distance() {
        assert_eq!(
            hash_distance(Some("0000000000000000"), Some("0000000000000003")),
            2
        );
        assert_eq!(hash_distance(None, Some("0")), 64);
    }

    #[test]
    fn overlapping_roots_require_a_path_component_boundary() {
        assert!(path_key_starts_with(r"c:\media\profile", r"c:\media"));
        assert!(!path_key_starts_with(
            r"c:\media\profile-two",
            r"c:\media\profile"
        ));
    }
}
