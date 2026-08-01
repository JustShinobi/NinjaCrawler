-- Curated groupings of media, with two orthogonal dimensions.
--
-- `scope` decides where the collection shows up: inside one profile, on an
-- identity (the same person across providers), or library-wide. `kind` decides
-- how membership is computed: `manual` holds explicit items, `smart` stores a
-- saved timeline filter in `rule_json` and is evaluated on read.
--
-- Keeping both in one table is what makes promotion cheap: turning a profile
-- collection into a global one is an UPDATE of two columns, and the items stay
-- put. It also means the timeline filter engine serves both kinds — a manual
-- collection is a filter by membership, a smart one is a filter by rule.
CREATE TABLE IF NOT EXISTS collections (
    id TEXT PRIMARY KEY,
    -- manual | smart
    kind TEXT NOT NULL DEFAULT 'manual',
    -- global | source | identity
    scope TEXT NOT NULL DEFAULT 'global',
    -- source_profiles.id or identities.id; NULL for global collections.
    scope_ref_id TEXT,
    name TEXT NOT NULL,
    description TEXT,
    color TEXT,
    rule_json TEXT,
    cover_media_id TEXT REFERENCES media_index(id) ON DELETE SET NULL,
    pinned INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_collections_scope
    ON collections(scope, scope_ref_id, pinned DESC, name);

-- Membership of a manual collection. The media stays exactly where it is on
-- disk: a file in five collections is still one file.
CREATE TABLE IF NOT EXISTS collection_items (
    collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    media_id TEXT NOT NULL REFERENCES media_index(id) ON DELETE CASCADE,
    position INTEGER,
    note TEXT,
    added_at TEXT NOT NULL,
    PRIMARY KEY (collection_id, media_id)
);

CREATE INDEX IF NOT EXISTS idx_collection_items_media
    ON collection_items(media_id);

CREATE INDEX IF NOT EXISTS idx_collection_items_added
    ON collection_items(collection_id, added_at DESC);
