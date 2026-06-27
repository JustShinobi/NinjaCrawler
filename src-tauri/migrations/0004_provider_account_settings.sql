CREATE TABLE IF NOT EXISTS provider_account_settings (
    account_id TEXT NOT NULL,
    setting_key TEXT NOT NULL,
    value_kind TEXT NOT NULL,
    value_text TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (account_id, setting_key),
    FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE,
    CHECK (value_kind IN ('string', 'json'))
);
