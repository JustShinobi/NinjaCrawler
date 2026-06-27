CREATE TABLE IF NOT EXISTS instagram_media_naming_ledger (
    source_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    source_handle TEXT NOT NULL,
    provider_media_key TEXT NOT NULL,
    media_type TEXT NOT NULL,
    media_section TEXT NOT NULL,
    captured_at INTEGER,
    extension TEXT NOT NULL,
    final_file_name TEXT NOT NULL,
    legacy_raw_file_name TEXT,
    relative_path TEXT NOT NULL,
    pattern_mode TEXT NOT NULL,
    pattern_template TEXT,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    PRIMARY KEY (source_id, provider_media_key, media_type),
    FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE,
    FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_instagram_media_naming_ledger_source_path
    ON instagram_media_naming_ledger(source_id, relative_path);

CREATE INDEX IF NOT EXISTS idx_instagram_media_naming_ledger_account_key
    ON instagram_media_naming_ledger(account_id, provider_media_key);
