-- Persistência da fila manual de source sync: jobs pendentes sobrevivem ao
-- fechamento do app e são restaurados no próximo start.
CREATE TABLE IF NOT EXISTS source_sync_queue_jobs (
    source_id TEXT PRIMARY KEY,
    trigger TEXT NOT NULL,
    run_mode TEXT,
    sync_options_override_json TEXT,
    queued_at TEXT NOT NULL
);
