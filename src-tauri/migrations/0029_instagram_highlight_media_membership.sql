-- Archival membership of Instagram MEDIA in highlight albums, keyed by the CDN
-- media key (the on-disk filename stem). Highlight items usually reshare media
-- already downloaded under the feed; the join key that reliably matches the
-- existing file is the media key (the post shortcode is often NULL for legacy
-- media). Resolving membership -> media ledger -> relative_path lets ProfileView
-- show the media under its album without duplicating bytes.
--
-- Replaces the post-key-based table from migration 0028 (which never matched,
-- since legacy feed media carries no post code).
DROP TABLE IF EXISTS instagram_highlight_membership;

CREATE TABLE IF NOT EXISTS instagram_highlight_media_membership (
    source_id TEXT NOT NULL,
    provider_media_key TEXT NOT NULL,
    album TEXT NOT NULL,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    PRIMARY KEY (source_id, provider_media_key, album),
    FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_instagram_highlight_media_membership_source
    ON instagram_highlight_media_membership(source_id);
