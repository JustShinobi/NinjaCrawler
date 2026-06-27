ALTER TABLE sync_plans ADD COLUMN paused INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sync_plans ADD COLUMN skip_until TEXT;
ALTER TABLE sync_plans ADD COLUMN last_run_at TEXT;
ALTER TABLE sync_plans ADD COLUMN last_run_status TEXT NOT NULL DEFAULT 'idle';
ALTER TABLE sync_plans ADD COLUMN last_run_summary TEXT;
ALTER TABLE sync_plans ADD COLUMN next_due_at TEXT;

CREATE TABLE IF NOT EXISTS sync_plan_runs (
    id TEXT PRIMARY KEY,
    plan_id TEXT NOT NULL,
    scheduler_set_id TEXT NOT NULL,
    trigger TEXT NOT NULL,
    status TEXT NOT NULL,
    summary TEXT NOT NULL,
    source_count INTEGER NOT NULL DEFAULT 0,
    started_at TEXT NOT NULL,
    finished_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (plan_id) REFERENCES sync_plans(id) ON DELETE CASCADE,
    FOREIGN KEY (scheduler_set_id) REFERENCES scheduler_sets(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS notification_events (
    id TEXT PRIMARY KEY,
    scope TEXT NOT NULL,
    title TEXT NOT NULL,
    message TEXT NOT NULL,
    level TEXT NOT NULL,
    action_route TEXT,
    created_at TEXT NOT NULL,
    read_at TEXT
);
