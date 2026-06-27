ALTER TABLE media_items ADD COLUMN provider_media_key TEXT;

CREATE INDEX IF NOT EXISTS idx_media_items_source_provider_media_key
    ON media_items(provider, source_id, provider_media_key);

CREATE INDEX IF NOT EXISTS idx_media_items_account_provider_media_key
    ON media_items(provider, account_id, source_handle, provider_media_key);
