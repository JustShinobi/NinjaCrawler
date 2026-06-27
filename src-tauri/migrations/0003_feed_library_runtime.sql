ALTER TABLE media_items ADD COLUMN missing_at TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_media_items_file_path
    ON media_items(file_path);

CREATE TABLE IF NOT EXISTS feed_collection_items (
    collection_id TEXT NOT NULL,
    media_item_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (collection_id, media_item_id),
    FOREIGN KEY (collection_id) REFERENCES feed_collections(id) ON DELETE CASCADE,
    FOREIGN KEY (media_item_id) REFERENCES media_items(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_feed_collection_items_media_item_id
    ON feed_collection_items(media_item_id);
