DELETE FROM app_settings
WHERE key = 'policy.feed.archived_session_retention_limit';

DROP TABLE IF EXISTS feed_collection_items;
DROP TABLE IF EXISTS saved_views;
DROP TABLE IF EXISTS saved_filters;
DROP TABLE IF EXISTS media_items;
DROP TABLE IF EXISTS feed_collections;
DROP TABLE IF EXISTS feed_sessions;
