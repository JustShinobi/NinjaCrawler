ALTER TABLE source_profiles ADD COLUMN remote_state TEXT NOT NULL DEFAULT 'exists';
ALTER TABLE source_profiles ADD COLUMN is_subscription INTEGER NOT NULL DEFAULT 0;
ALTER TABLE source_profiles ADD COLUMN last_synced_at TEXT;

ALTER TABLE sync_plans ADD COLUMN sort_index INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sync_plans ADD COLUMN pause_mode TEXT NOT NULL DEFAULT 'disabled';
ALTER TABLE sync_plans ADD COLUMN pause_until TEXT;
ALTER TABLE sync_plans ADD COLUMN notifications_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE sync_plans ADD COLUMN criteria_json TEXT NOT NULL DEFAULT '{}';

CREATE TABLE IF NOT EXISTS scheduler_groups (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    criteria_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_scheduler_groups_sort_index
    ON scheduler_groups(sort_index, name);
