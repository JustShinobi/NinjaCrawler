-- Ledgers de sync provider-neutral. O Twitter é o segundo provider interno;
-- em vez de clonar os esquemas instagram_*, novos providers registram posts e
-- mídia aqui, discriminados pela coluna provider.
CREATE TABLE IF NOT EXISTS provider_sync_post_ledger (
    provider TEXT NOT NULL,
    source_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    source_handle TEXT NOT NULL,
    provider_post_key TEXT NOT NULL,
    provider_post_code TEXT NOT NULL DEFAULT '',
    media_section TEXT NOT NULL,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    PRIMARY KEY (provider, source_id, provider_post_key),
    FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE,
    FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_provider_sync_post_ledger_account_key
    ON provider_sync_post_ledger(provider, account_id, provider_post_key);

CREATE TABLE IF NOT EXISTS provider_sync_media_ledger (
    provider TEXT NOT NULL,
    source_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    source_handle TEXT NOT NULL,
    provider_media_key TEXT NOT NULL,
    media_type TEXT NOT NULL,
    media_section TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    PRIMARY KEY (provider, source_id, provider_media_key, media_type),
    FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE,
    FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_provider_sync_media_ledger_source_path
    ON provider_sync_media_ledger(provider, source_id, relative_path);
