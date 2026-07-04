CREATE TABLE IF NOT EXISTS account_sync_scope_state (
    provider TEXT NOT NULL,
    account_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    sync_scope TEXT NOT NULL,
    full_scan_completed_at TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (provider, account_id, source_id, sync_scope),
    FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE,
    FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE
);
