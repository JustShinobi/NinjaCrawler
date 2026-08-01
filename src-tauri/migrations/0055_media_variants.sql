-- Groups of media that are the same content in more than one place.
--
-- Two cases the existing dedupe cannot see, both reported from real use:
--   * the same person posts a video to Instagram and to TikTok — different
--     providers, different profiles, so a per-profile scan never compares them;
--   * a profile posts a video to their story and then to the feed — same
--     profile, different sections, different provider keys, and the encodes
--     differ enough that a sha256 comparison finds nothing.
--
-- Grouping is deliberately non-destructive by default: the members stay on disk
-- and the gallery collapses them into one card. Reclaiming space stays an
-- explicit, reviewed action in the existing media cleanup flow.
CREATE TABLE IF NOT EXISTS media_variant_groups (
    id TEXT PRIMARY KEY,
    -- intra_source (story vs feed) | cross_source (same person, two providers)
    scope TEXT NOT NULL,
    identity_id TEXT REFERENCES identities(id) ON DELETE SET NULL,
    canonical_media_id TEXT REFERENCES media_index(id) ON DELETE SET NULL,
    -- exact_sha256 | perceptual_image | perceptual_video
    match_kind TEXT NOT NULL,
    confidence REAL NOT NULL DEFAULT 1.0,
    -- link_only | kept_best | kept_first | keep_all
    policy_applied TEXT NOT NULL DEFAULT 'link_only',
    reviewed INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_media_variant_groups_review
    ON media_variant_groups(reviewed, updated_at DESC);

CREATE TABLE IF NOT EXISTS media_variant_members (
    group_id TEXT NOT NULL REFERENCES media_variant_groups(id) ON DELETE CASCADE,
    media_id TEXT NOT NULL REFERENCES media_index(id) ON DELETE CASCADE,
    similarity REAL NOT NULL DEFAULT 1.0,
    -- canonical | variant
    role TEXT NOT NULL DEFAULT 'variant',
    PRIMARY KEY (group_id, media_id)
);

CREATE INDEX IF NOT EXISTS idx_media_variant_members_media
    ON media_variant_members(media_id);
