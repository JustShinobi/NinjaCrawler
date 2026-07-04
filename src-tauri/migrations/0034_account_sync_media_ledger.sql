CREATE TABLE IF NOT EXISTS account_sync_media_ledger (
    provider TEXT NOT NULL,
    account_id TEXT NOT NULL,
    sync_scope TEXT NOT NULL,
    provider_item_key TEXT NOT NULL,
    source_handle TEXT NOT NULL DEFAULT '',
    source_url TEXT NOT NULL DEFAULT '',
    relative_path TEXT NOT NULL,
    media_type TEXT NOT NULL,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    PRIMARY KEY (provider, account_id, sync_scope, provider_item_key),
    FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_account_sync_media_ledger_path
    ON account_sync_media_ledger(provider, account_id, sync_scope, relative_path);
