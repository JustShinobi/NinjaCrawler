-- Permite múltiplos jobs direcionados (por exemplo, stories diferentes) do
-- mesmo source sem remover a deduplicação do sync normal.
ALTER TABLE source_sync_queue_jobs RENAME TO source_sync_queue_jobs_legacy;

CREATE TABLE source_sync_queue_jobs (
    job_key TEXT PRIMARY KEY,
    source_id TEXT NOT NULL,
    trigger TEXT NOT NULL,
    run_mode TEXT,
    sync_options_override_json TEXT,
    queued_at TEXT NOT NULL,
    order_index INTEGER NOT NULL DEFAULT 0
);

INSERT INTO source_sync_queue_jobs
    (job_key, source_id, trigger, run_mode, sync_options_override_json, queued_at, order_index)
SELECT source_id, source_id, trigger, run_mode, sync_options_override_json, queued_at, order_index
FROM source_sync_queue_jobs_legacy;

DROP TABLE source_sync_queue_jobs_legacy;

CREATE INDEX idx_source_sync_queue_jobs_source_id
    ON source_sync_queue_jobs(source_id);
