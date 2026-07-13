CREATE TABLE IF NOT EXISTS media_path_migration_queue_jobs (
    job_id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL UNIQUE,
    target_base_path TEXT NOT NULL,
    queued_at TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'queued',
    started_at TEXT,
    staging_path TEXT
);
