-- Standalone videos captured by URL (via the Companion) that do NOT belong to a
-- tracked profile. Files live flat under a single "Single videos" root on disk;
-- this table holds the metadata so the media view can filter by provider,
-- uploader and date without a per-profile folder structure.
CREATE TABLE IF NOT EXISTS single_videos (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    source_url TEXT NOT NULL,
    -- Provider-native id (e.g. TikTok aweme id). Together with `provider` it
    -- dedupes re-captures of the same video.
    provider_video_id TEXT,
    uploader TEXT,
    title TEXT,
    -- Path of the downloaded file relative to the Single videos root.
    relative_path TEXT NOT NULL,
    media_type TEXT NOT NULL DEFAULT 'video',
    -- Original post timestamp (unix seconds), when resolvable.
    captured_at INTEGER,
    downloaded_at TEXT NOT NULL,
    UNIQUE (provider, provider_video_id)
);

CREATE INDEX IF NOT EXISTS idx_single_videos_provider ON single_videos(provider);
CREATE INDEX IF NOT EXISTS idx_single_videos_uploader ON single_videos(uploader);
