ALTER TABLE source_profiles ADD COLUMN profile_image_path TEXT;
ALTER TABLE source_profiles ADD COLUMN profile_image_custom INTEGER NOT NULL DEFAULT 0;
