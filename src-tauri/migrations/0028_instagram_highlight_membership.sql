-- Archival membership of Instagram posts in highlight albums.
--
-- A highlight item often reshares a feed post that was already downloaded; the
-- sync skips re-downloading the bytes (known_in_post_ledger), so on disk the
-- media lives only under the feed. This table records, append-only, that the
-- post belongs to a highlight album so ProfileView can show it under that album
-- without duplicating bytes. It is a snapshot: rows are never removed by sync,
-- so an item stays in the album even if later removed from the highlight online.
CREATE TABLE IF NOT EXISTS instagram_highlight_membership (
    source_id TEXT NOT NULL,
    provider_post_key TEXT NOT NULL,
    provider_post_code TEXT,
    album TEXT NOT NULL,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    PRIMARY KEY (source_id, provider_post_key, album),
    FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_instagram_highlight_membership_source
    ON instagram_highlight_membership(source_id);
