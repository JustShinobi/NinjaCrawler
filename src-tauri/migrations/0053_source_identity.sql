-- Stable provider identity for tracked profiles.
--
-- Until now the provider user id lived inside `sync_options_json`, and only for
-- Instagram and X. Two consequences: looking a profile up by user id meant
-- scanning every row of the provider and parsing JSON per row, and the
-- duplicate-profile guard silently did nothing on TikTok, YouTube and VSCO
-- because there was no hint to compare against.
--
-- The column is the normalized home for that id. The JSON hint stays readable
-- as a fallback so profiles synced before this migration keep working until
-- their next sync fills the column in.
-- Cross-provider person. A profile belongs to at most one identity; an identity
-- groups the same person's profiles across providers, which is what scopes
-- cross-provider duplicate detection to a tractable set. Created before the
-- column that references it.
CREATE TABLE IF NOT EXISTS identities (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    notes TEXT,
    avatar_source_id TEXT REFERENCES source_profiles(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

ALTER TABLE source_profiles ADD COLUMN provider_user_id TEXT;
ALTER TABLE source_profiles ADD COLUMN identity_id TEXT REFERENCES identities(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_source_profiles_provider_user
    ON source_profiles(provider, provider_user_id);

CREATE INDEX IF NOT EXISTS idx_source_profiles_identity
    ON source_profiles(identity_id);

-- Every handle a profile has been known by. A renamed profile keeps its media,
-- its ledger and its id; only the handle moves, and the old one still has to be
-- recognizable (old links, old folder names, the operator's memory).
CREATE TABLE IF NOT EXISTS source_handle_history (
    source_id TEXT NOT NULL REFERENCES source_profiles(id) ON DELETE CASCADE,
    handle TEXT NOT NULL,
    provider_user_id TEXT,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    PRIMARY KEY (source_id, handle)
);

CREATE INDEX IF NOT EXISTS idx_source_handle_history_source
    ON source_handle_history(source_id, last_seen_at DESC);
