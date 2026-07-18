-- Structured profile metadata captured during sync (Instagram today; other
-- providers as they gain support). Distinct from the free-text profile note
-- stored in the source's sync options — these power the enriched ProfileView
-- header (bio + follower/following/post counts + verified badge).
ALTER TABLE source_profiles ADD COLUMN profile_biography TEXT;
ALTER TABLE source_profiles ADD COLUMN profile_follower_count INTEGER;
ALTER TABLE source_profiles ADD COLUMN profile_following_count INTEGER;
ALTER TABLE source_profiles ADD COLUMN profile_media_count INTEGER;
ALTER TABLE source_profiles ADD COLUMN profile_is_verified INTEGER;
ALTER TABLE source_profiles ADD COLUMN profile_stats_updated_at TEXT;
