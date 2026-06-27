-- Add group_id column to source_profiles for direct profile-to-group assignment.
ALTER TABLE source_profiles ADD COLUMN group_id TEXT;
