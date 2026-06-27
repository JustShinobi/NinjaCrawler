ALTER TABLE source_profiles
    ADD COLUMN sync_options_json TEXT NOT NULL DEFAULT '{}';

CREATE TABLE IF NOT EXISTS account_sync_runs (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    sync_scope TEXT NOT NULL,
    tool TEXT NOT NULL,
    trigger TEXT NOT NULL,
    status TEXT NOT NULL,
    summary TEXT NOT NULL,
    command_preview TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE CASCADE
);

PRAGMA foreign_keys = OFF;

DROP INDEX IF EXISTS idx_media_items_file_path;
DROP INDEX IF EXISTS idx_feed_collection_items_media_item_id;

ALTER TABLE media_items RENAME TO media_items_legacy;
ALTER TABLE feed_collection_items RENAME TO feed_collection_items_legacy;

CREATE TABLE media_items (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    source_id TEXT,
    account_id TEXT,
    session_id TEXT,
    source_handle TEXT NOT NULL,
    media_section TEXT,
    media_type TEXT NOT NULL,
    captured_at TEXT NOT NULL,
    file_path TEXT NOT NULL,
    missing_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE SET NULL,
    FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE SET NULL,
    FOREIGN KEY (session_id) REFERENCES feed_sessions(id) ON DELETE SET NULL
);

INSERT INTO media_items (
    id,
    provider,
    source_id,
    account_id,
    session_id,
    source_handle,
    media_section,
    media_type,
    captured_at,
    file_path,
    missing_at,
    created_at,
    updated_at
)
SELECT
    id,
    provider,
    source_id,
    NULL,
    session_id,
    source_handle,
    NULL,
    media_type,
    captured_at,
    file_path,
    missing_at,
    created_at,
    updated_at
FROM media_items_legacy;

CREATE UNIQUE INDEX IF NOT EXISTS idx_media_items_file_path
    ON media_items(file_path);

CREATE TABLE feed_collection_items (
    collection_id TEXT NOT NULL,
    media_item_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (collection_id, media_item_id),
    FOREIGN KEY (collection_id) REFERENCES feed_collections(id) ON DELETE CASCADE,
    FOREIGN KEY (media_item_id) REFERENCES media_items(id) ON DELETE CASCADE
);

INSERT INTO feed_collection_items (collection_id, media_item_id, created_at)
SELECT collection_id, media_item_id, created_at
FROM feed_collection_items_legacy;

CREATE INDEX IF NOT EXISTS idx_feed_collection_items_media_item_id
    ON feed_collection_items(media_item_id);

DROP TABLE feed_collection_items_legacy;
DROP TABLE media_items_legacy;

PRAGMA foreign_keys = ON;
