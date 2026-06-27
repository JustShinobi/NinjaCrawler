CREATE TABLE IF NOT EXISTS instagram_media_fingerprints (
    source_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    provider_media_key TEXT NOT NULL,
    media_type TEXT NOT NULL,
    media_section TEXT NOT NULL,
    width INTEGER,
    height INTEGER,
    file_sha256 TEXT,
    ahash64 TEXT,
    dhash64 TEXT,
    relative_path TEXT,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    PRIMARY KEY (source_id, provider_media_key, media_type),
    FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE,
    FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_instagram_media_fingerprints_sha256
    ON instagram_media_fingerprints(source_id, file_sha256);

CREATE INDEX IF NOT EXISTS idx_instagram_media_fingerprints_perceptual
    ON instagram_media_fingerprints(source_id, media_section, width, height, ahash64, dhash64);
