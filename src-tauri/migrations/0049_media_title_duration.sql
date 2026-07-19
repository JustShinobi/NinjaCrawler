-- Adds optional media metadata captured by the YouTube connector (and available
-- to any future provider): the native media title and its duration in seconds.
-- SQLite only allows a single column change per ALTER TABLE statement.
ALTER TABLE provider_sync_media_ledger ADD COLUMN title TEXT;
ALTER TABLE provider_sync_media_ledger ADD COLUMN duration_seconds INTEGER;
