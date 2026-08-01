-- Canonical media catalog. Until now every media view was derived from disk on
-- demand (`load_source_media_gallery` walks the profile root on each call), so
-- nothing could answer questions across profiles: an aggregated timeline,
-- collections that survive a folder rename, library-wide counters, or duplicate
-- detection between two profiles of the same person.
--
-- `id` is an opaque UUID and is the stable handle other tables reference. The
-- natural key `(source_id, relative_path)` moves when a profile folder is
-- renamed or the media root changes; the row keeps its `id`, so anything
-- pointing at it (collections, variant groups) survives.
--
-- `normalized_path` is the absolute path in the same shape `media_dedupe_catalog`
-- stores it (backslashes, lowercased on Windows), which lets the index inherit
-- already-computed hashes from the dedupe catalog instead of re-hashing the
-- whole library.
CREATE TABLE IF NOT EXISTS media_index (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    source_id TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    normalized_path TEXT NOT NULL,
    media_type TEXT NOT NULL,
    media_section TEXT NOT NULL DEFAULT '',
    provider_media_key TEXT,
    provider_post_key TEXT,
    -- Unix seconds of the original post, mirroring the ledger `captured_at`.
    captured_at INTEGER,
    -- Unix seconds of the first time this app saw the file.
    downloaded_at INTEGER,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    modified_at_ms INTEGER NOT NULL DEFAULT 0,
    width INTEGER,
    height INTEGER,
    duration_ms INTEGER,
    sha256 TEXT,
    ahash64 TEXT,
    dhash64 TEXT,
    -- JSON array of sampled-frame hashes for video similarity (filled in F5).
    video_signature TEXT,
    -- pending | complete | failed | skipped
    fingerprint_status TEXT NOT NULL DEFAULT 'pending',
    -- Populated by the variant grouping in F5; NULL means "not grouped".
    variant_group_id TEXT,
    is_canonical INTEGER NOT NULL DEFAULT 1,
    -- Two independent axes, deliberately kept in separate columns: a file can
    -- be gone from disk while still online, and archived here while removed
    -- from the provider.
    -- present | missing_on_disk
    local_state TEXT NOT NULL DEFAULT 'present',
    -- present | missing — projected from the post ledger by the upstream
    -- presence evaluation.
    upstream_state TEXT NOT NULL DEFAULT 'present',
    indexed_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (source_id, relative_path),
    FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE
);

-- Keyset pagination for the aggregated timeline (captured_at DESC, id).
CREATE INDEX IF NOT EXISTS idx_media_index_timeline
    ON media_index(captured_at DESC, id);

CREATE INDEX IF NOT EXISTS idx_media_index_source
    ON media_index(source_id, captured_at DESC);

CREATE INDEX IF NOT EXISTS idx_media_index_provider
    ON media_index(provider, captured_at DESC);

CREATE INDEX IF NOT EXISTS idx_media_index_sha256
    ON media_index(sha256);

-- Drives the fingerprint backlog queue.
CREATE INDEX IF NOT EXISTS idx_media_index_fingerprint
    ON media_index(fingerprint_status, updated_at);

CREATE INDEX IF NOT EXISTS idx_media_index_variant
    ON media_index(variant_group_id);

-- Join key against media_dedupe_catalog and reconciliation lookups.
CREATE INDEX IF NOT EXISTS idx_media_index_normalized
    ON media_index(normalized_path);

CREATE INDEX IF NOT EXISTS idx_media_index_post
    ON media_index(source_id, provider_post_key);

-- Progress and resumability for the background indexing runs.
CREATE TABLE IF NOT EXISTS media_index_runs (
    id TEXT PRIMARY KEY,
    -- queued | running | completed | failed | cancelled
    status TEXT NOT NULL,
    -- inventory | inherit | reconcile | done
    stage TEXT NOT NULL,
    scope_source_id TEXT REFERENCES source_profiles(id) ON DELETE SET NULL,
    sources_total INTEGER NOT NULL DEFAULT 0,
    sources_processed INTEGER NOT NULL DEFAULT 0,
    files_indexed INTEGER NOT NULL DEFAULT 0,
    files_updated INTEGER NOT NULL DEFAULT 0,
    files_missing INTEGER NOT NULL DEFAULT 0,
    hashes_inherited INTEGER NOT NULL DEFAULT 0,
    -- Fingerprint backlog: the long stage. Tracked separately so progress and a
    -- finish estimate can be reported instead of a profile count stuck at 100%.
    fingerprints_total INTEGER NOT NULL DEFAULT 0,
    fingerprints_done INTEGER NOT NULL DEFAULT 0,
    fingerprint_started_at TEXT,
    -- quiet | balanced | fast — how much of the machine the operator allows.
    resource_profile TEXT NOT NULL DEFAULT 'balanced',
    current_source_handle TEXT,
    error TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_media_index_runs_status
    ON media_index_runs(status, updated_at DESC);
