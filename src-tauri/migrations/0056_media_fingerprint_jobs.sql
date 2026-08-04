-- Candidate-driven fingerprint queue for the aggregated Library.
--
-- A media row only needs an expensive fingerprint when another row is eligible
-- to be compared with it. Keeping that work in a dedicated queue avoids using
-- `media_index.fingerprint_status` as both catalog state and scheduler state.
CREATE TABLE IF NOT EXISTS media_fingerprint_jobs (
    media_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('exact', 'perceptual_image', 'perceptual_video')),
    status TEXT NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'running', 'complete', 'failed')),
    priority INTEGER NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0,
    policy_version INTEGER NOT NULL DEFAULT 1,
    candidate_context TEXT NOT NULL DEFAULT '',
    lease_owner TEXT,
    lease_expires_at TEXT,
    error TEXT,
    expected_size_bytes INTEGER NOT NULL,
    expected_modified_at_ms INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (media_id, kind),
    FOREIGN KEY (media_id) REFERENCES media_index(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_media_fingerprint_jobs_queue
    ON media_fingerprint_jobs(status, priority DESC, updated_at, media_id, kind);

CREATE INDEX IF NOT EXISTS idx_media_fingerprint_jobs_lease
    ON media_fingerprint_jobs(status, lease_expires_at);

-- Candidate planning follows the same source/section/identity boundaries as
-- variant detection. This index keeps those EXISTS probes bounded.
CREATE INDEX IF NOT EXISTS idx_media_index_variant_candidates
    ON media_index(source_id, media_type, media_section, size_bytes, captured_at);

ALTER TABLE media_index_runs ADD COLUMN phase_total INTEGER NOT NULL DEFAULT 0;
ALTER TABLE media_index_runs ADD COLUMN phase_done INTEGER NOT NULL DEFAULT 0;
ALTER TABLE media_index_runs ADD COLUMN phase_failed INTEGER NOT NULL DEFAULT 0;
ALTER TABLE media_index_runs ADD COLUMN bytes_processed INTEGER NOT NULL DEFAULT 0;
ALTER TABLE media_index_runs ADD COLUMN last_progress_at TEXT;
ALTER TABLE media_index_runs ADD COLUMN rate_per_second REAL NOT NULL DEFAULT 0;
ALTER TABLE media_index_runs ADD COLUMN eta_seconds INTEGER;
