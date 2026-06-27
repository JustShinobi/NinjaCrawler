-- Stores the Instagram post shortcode (case-sensitive, used in instagram.com/p/<code>/)
-- alongside each downloaded media file, so ProfileView can rebuild the original
-- post link. Instagram shortcodes are case-sensitive, so this is kept with its
-- original casing (unlike the normalized, lowercased identity keys in the post
-- ledger which exist only for dedupe).
ALTER TABLE instagram_sync_media_ledger ADD COLUMN provider_post_code TEXT;
