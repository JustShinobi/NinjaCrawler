-- Development builds may have recorded migration 42 before the media cleanup
-- schema was present. Re-applying the idempotent schema repairs those databases.
CREATE TABLE IF NOT EXISTS media_dedupe_scans (
    id TEXT PRIMARY KEY,
    status TEXT NOT NULL,
    stage TEXT NOT NULL,
    files_scanned INTEGER NOT NULL DEFAULT 0,
    files_total INTEGER NOT NULL DEFAULT 0,
    bytes_scanned INTEGER NOT NULL DEFAULT 0,
    bytes_total INTEGER NOT NULL DEFAULT 0,
    exact_group_count INTEGER NOT NULL DEFAULT 0,
    similar_group_count INTEGER NOT NULL DEFAULT 0,
    reclaimable_bytes INTEGER NOT NULL DEFAULT 0,
    skipped_video_similarity_count INTEGER NOT NULL DEFAULT 0,
    current_path TEXT,
    error TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS media_dedupe_files (
    scan_id TEXT NOT NULL,
    path TEXT NOT NULL,
    normalized_path TEXT NOT NULL,
    source_id TEXT,
    provider TEXT,
    root_path TEXT NOT NULL,
    volume_key TEXT NOT NULL,
    media_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    modified_at_ms INTEGER NOT NULL,
    sha256 TEXT NOT NULL,
    width INTEGER,
    height INTEGER,
    duration_ms INTEGER,
    ahash64 TEXT,
    dhash64 TEXT,
    video_hashes_json TEXT,
    PRIMARY KEY (scan_id, normalized_path),
    FOREIGN KEY (scan_id) REFERENCES media_dedupe_scans(id) ON DELETE CASCADE,
    FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_media_dedupe_files_exact
    ON media_dedupe_files(scan_id, volume_key, size_bytes, sha256);

CREATE INDEX IF NOT EXISTS idx_media_dedupe_files_similar
    ON media_dedupe_files(scan_id, source_id, media_type, width, height, ahash64, dhash64);

CREATE TABLE IF NOT EXISTS media_dedupe_jobs (
    id TEXT PRIMARY KEY,
    scan_id TEXT NOT NULL,
    job_kind TEXT NOT NULL,
    status TEXT NOT NULL,
    stage TEXT NOT NULL,
    files_processed INTEGER NOT NULL DEFAULT 0,
    files_total INTEGER NOT NULL DEFAULT 0,
    bytes_processed INTEGER NOT NULL DEFAULT 0,
    bytes_total INTEGER NOT NULL DEFAULT 0,
    current_path TEXT,
    current_root TEXT,
    error TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (scan_id) REFERENCES media_dedupe_scans(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_media_dedupe_jobs_status
    ON media_dedupe_jobs(status, updated_at DESC);

CREATE TABLE IF NOT EXISTS media_dedupe_actions (
    id TEXT PRIMARY KEY,
    scan_id TEXT NOT NULL,
    action_kind TEXT NOT NULL,
    status TEXT NOT NULL,
    canonical_path TEXT,
    target_path TEXT,
    bytes_reclaimed INTEGER NOT NULL DEFAULT 0,
    error TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    FOREIGN KEY (scan_id) REFERENCES media_dedupe_scans(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_media_dedupe_actions_scan
    ON media_dedupe_actions(scan_id, started_at DESC);
