CREATE TABLE IF NOT EXISTS instagram_sync_media_ledger (
    source_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    source_handle TEXT NOT NULL,
    provider_media_key TEXT NOT NULL,
    media_type TEXT NOT NULL,
    media_section TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    PRIMARY KEY (source_id, provider_media_key, media_type),
    FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE,
    FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_instagram_sync_media_ledger_source_path
    ON instagram_sync_media_ledger(source_id, relative_path);

CREATE INDEX IF NOT EXISTS idx_instagram_sync_media_ledger_account_key
    ON instagram_sync_media_ledger(account_id, provider_media_key);

ALTER TABLE source_sync_runs ADD COLUMN manifest_summary_json TEXT;
