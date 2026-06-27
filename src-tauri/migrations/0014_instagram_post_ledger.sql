CREATE TABLE IF NOT EXISTS instagram_sync_post_ledger (
    source_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    source_handle TEXT NOT NULL,
    provider_post_key TEXT NOT NULL,
    provider_post_code TEXT NOT NULL DEFAULT '',
    media_section TEXT NOT NULL,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    PRIMARY KEY (source_id, provider_post_key),
    FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE,
    FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_instagram_sync_post_ledger_account_key
    ON instagram_sync_post_ledger(account_id, provider_post_key);

CREATE INDEX IF NOT EXISTS idx_instagram_sync_post_ledger_source_code
    ON instagram_sync_post_ledger(source_id, provider_post_code);
