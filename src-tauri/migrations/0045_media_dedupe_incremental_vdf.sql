CREATE TABLE IF NOT EXISTS media_dedupe_catalog (
    normalized_path TEXT PRIMARY KEY,
    path TEXT NOT NULL,
    source_id TEXT,
    provider TEXT,
    root_path TEXT NOT NULL,
    volume_key TEXT NOT NULL,
    media_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    modified_at_ms INTEGER NOT NULL,
    sha256 TEXT,
    width INTEGER,
    height INTEGER,
    duration_ms INTEGER,
    ahash64 TEXT,
    dhash64 TEXT,
    hash_status TEXT NOT NULL DEFAULT 'pending',
    last_seen_scan_id TEXT NOT NULL,
    last_hashed_at TEXT,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_media_dedupe_catalog_seen
    ON media_dedupe_catalog(last_seen_scan_id, volume_key, size_bytes);

CREATE INDEX IF NOT EXISTS idx_media_dedupe_catalog_source
    ON media_dedupe_catalog(last_seen_scan_id, source_id, media_type);

CREATE TABLE IF NOT EXISTS media_dedupe_source_jobs (
    id TEXT PRIMARY KEY,
    scan_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    source_path TEXT NOT NULL,
    status TEXT NOT NULL,
    stage TEXT NOT NULL,
    progress_percent INTEGER,
    files_processed INTEGER NOT NULL DEFAULT 0,
    files_total INTEGER NOT NULL DEFAULT 0,
    current_path TEXT,
    runtime_version TEXT,
    runtime_digest TEXT,
    settings_fingerprint TEXT,
    database_path TEXT,
    result_path TEXT,
    error TEXT,
    started_at TEXT,
    finished_at TEXT,
    updated_at TEXT NOT NULL,
    UNIQUE (scan_id, source_id),
    FOREIGN KEY (scan_id) REFERENCES media_dedupe_scans(id) ON DELETE CASCADE,
    FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_media_dedupe_source_jobs_queue
    ON media_dedupe_source_jobs(scan_id, status, updated_at);

CREATE TABLE IF NOT EXISTS media_dedupe_vdf_candidates (
    scan_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    group_id TEXT NOT NULL,
    path TEXT NOT NULL,
    normalized_path TEXT NOT NULL,
    similarity_percent REAL NOT NULL,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    duration_ms INTEGER,
    width INTEGER,
    height INTEGER,
    runtime_version TEXT NOT NULL,
    runtime_digest TEXT NOT NULL,
    settings_fingerprint TEXT NOT NULL,
    imported_at TEXT NOT NULL,
    PRIMARY KEY (scan_id, source_id, group_id, normalized_path),
    FOREIGN KEY (scan_id) REFERENCES media_dedupe_scans(id) ON DELETE CASCADE,
    FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_media_dedupe_vdf_candidates_group
    ON media_dedupe_vdf_candidates(scan_id, source_id, group_id);
