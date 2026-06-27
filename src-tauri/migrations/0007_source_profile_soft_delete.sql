ALTER TABLE source_profiles
    ADD COLUMN deleted_at TEXT;

CREATE INDEX IF NOT EXISTS idx_source_profiles_deleted_at
    ON source_profiles (deleted_at);
