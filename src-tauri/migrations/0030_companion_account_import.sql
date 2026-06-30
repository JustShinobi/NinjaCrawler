CREATE TABLE IF NOT EXISTS provider_account_import_state (
    account_id TEXT PRIMARY KEY,
    provider_user_id TEXT,
    provider_username TEXT,
    last_imported_at TEXT NOT NULL,
    backup_secret_ref TEXT,
    backup_provider_user_id TEXT,
    backup_provider_username TEXT,
    backup_imported_at TEXT,
    FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_provider_account_import_identity
    ON provider_account_import_state(provider_user_id, provider_username);
