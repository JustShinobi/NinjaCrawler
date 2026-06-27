CREATE TABLE IF NOT EXISTS instagram_media_key_aliases (
    source_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    provider_media_key TEXT NOT NULL,
    alias_key TEXT NOT NULL,
    alias_kind TEXT NOT NULL,
    file_sha256 TEXT,
    relative_path TEXT,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    PRIMARY KEY (source_id, provider_media_key, alias_key),
    FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE,
    FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_instagram_media_key_aliases_source_alias
    ON instagram_media_key_aliases(source_id, alias_key);

CREATE INDEX IF NOT EXISTS idx_instagram_media_key_aliases_provider_key
    ON instagram_media_key_aliases(source_id, provider_media_key);

CREATE INDEX IF NOT EXISTS idx_instagram_media_key_aliases_sha256
    ON instagram_media_key_aliases(source_id, file_sha256);
