-- Tracks posts that disappeared from the provider. This is what separates an
-- archive from a mirror: media that only exists here is the part worth keeping.
--
-- Absence is NOT evidence of removal. Normal syncs stop early once they
-- recognize known posts (see the Instagram incremental discovery), sections can
-- be deselected, date filters narrow the window, and rate limits truncate a
-- listing. Marking any of those as "removed" would flag most of the library.
-- Only a scan that enumerated a whole section, finished cleanly, and repeated
-- the same absence `missing_confirmations` times flips the state.
ALTER TABLE provider_sync_post_ledger ADD COLUMN upstream_state TEXT NOT NULL DEFAULT 'present';
ALTER TABLE provider_sync_post_ledger ADD COLUMN missing_confirmations INTEGER NOT NULL DEFAULT 0;
ALTER TABLE provider_sync_post_ledger ADD COLUMN missing_since TEXT;
ALTER TABLE provider_sync_post_ledger ADD COLUMN last_full_scan_at TEXT;

CREATE INDEX IF NOT EXISTS idx_provider_sync_post_ledger_upstream
    ON provider_sync_post_ledger(provider, source_id, upstream_state);

-- Audit trail of the scans that were allowed to judge absence. Kept separate
-- from `source_sync_runs` because most runs never qualify, and because a wrong
-- "removed" badge is only debuggable with the qualifying scan in hand.
CREATE TABLE IF NOT EXISTS source_full_scan_runs (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES source_profiles(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    -- Comma-separated sections this scan enumerated in full.
    sections TEXT NOT NULL DEFAULT '',
    posts_seen INTEGER NOT NULL DEFAULT 0,
    posts_flagged INTEGER NOT NULL DEFAULT 0,
    posts_recovered INTEGER NOT NULL DEFAULT 0,
    evaluated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_source_full_scan_runs_source
    ON source_full_scan_runs(source_id, evaluated_at DESC);
