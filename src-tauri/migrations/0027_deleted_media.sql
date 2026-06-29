-- Tombstone for media deleted from ProfileView. Records the deletion so the
-- file is not shown again and, together with the post key written back into the
-- per-provider post ledger, is NOT re-downloaded on the next sync.
CREATE TABLE IF NOT EXISTS provider_deleted_media (
    provider TEXT NOT NULL,
    source_id TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    media_section TEXT NOT NULL DEFAULT '',
    provider_post_key TEXT,
    provider_post_code TEXT,
    provider_media_key TEXT,
    deleted_at TEXT NOT NULL,
    PRIMARY KEY (provider, source_id, relative_path),
    FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_provider_deleted_media_source
    ON provider_deleted_media(provider, source_id);
