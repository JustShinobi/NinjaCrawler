-- Mirrors 0049 on the Instagram-specific ledger: the post caption and the
-- capture timestamp. The native connector leaves both NULL; they are filled by
-- legacy imports whose catalog carries richer metadata than the file name (the
-- 4K Stogram migration stores the caption in `photos.title` and the capture time
-- in `photos.created_time`).
--
-- `instagram_sync_media_ledger` is created lazily at runtime by
-- `ensure_instagram_sync_media_ledger_table`, so it may not exist yet when this
-- runs. The statements below are therefore applied from `apply_migration` via
-- `add_column_if_missing`, which no-ops on a missing table; this file is kept
-- for documentation and for the schema history.
ALTER TABLE instagram_sync_media_ledger ADD COLUMN title TEXT;
ALTER TABLE instagram_sync_media_ledger ADD COLUMN captured_at INTEGER;
