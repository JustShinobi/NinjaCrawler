ALTER TABLE media_dedupe_scans
    ADD COLUMN resource_profile TEXT NOT NULL DEFAULT 'balanced';

ALTER TABLE media_dedupe_source_jobs
    ADD COLUMN inventory_fingerprint TEXT;

ALTER TABLE media_dedupe_source_jobs
    ADD COLUMN cached_from_scan_id TEXT;

CREATE INDEX IF NOT EXISTS idx_media_dedupe_source_jobs_cache
    ON media_dedupe_source_jobs(
        source_id,
        status,
        runtime_digest,
        settings_fingerprint,
        inventory_fingerprint,
        finished_at DESC
    );
