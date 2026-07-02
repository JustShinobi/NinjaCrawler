-- Persist the stable Instagram identity already present in successful sync
-- manifests. Older profiles only kept this value inside source_sync_runs,
-- which made rename recovery depend on deserializing an evolving report schema.
UPDATE source_profiles
SET sync_options_json = json_set(
        sync_options_json,
        '$.instagram.userIdHint',
        (
            SELECT json_extract(run.manifest_summary_json, '$.profileUserId')
            FROM source_sync_runs AS run
            WHERE run.source_id = source_profiles.id
              AND run.provider = 'instagram'
              AND run.status = 'succeeded'
              AND run.manifest_summary_json IS NOT NULL
              AND json_valid(run.manifest_summary_json)
              AND NULLIF(
                    TRIM(json_extract(run.manifest_summary_json, '$.profileUserId')),
                    ''
                  ) IS NOT NULL
            ORDER BY run.finished_at DESC
            LIMIT 1
        )
    )
WHERE provider = 'instagram'
  AND deleted_at IS NULL
  AND json_valid(sync_options_json)
  AND NULLIF(
        TRIM(json_extract(sync_options_json, '$.instagram.userIdHint')),
        ''
      ) IS NULL
  AND EXISTS (
        SELECT 1
        FROM source_sync_runs AS run
        WHERE run.source_id = source_profiles.id
          AND run.provider = 'instagram'
          AND run.status = 'succeeded'
          AND run.manifest_summary_json IS NOT NULL
          AND json_valid(run.manifest_summary_json)
          AND NULLIF(
                TRIM(json_extract(run.manifest_summary_json, '$.profileUserId')),
                ''
              ) IS NOT NULL
    );

-- Profiles blocked by the old failure mode need one retry after their stable
-- identity is restored. A genuinely removed account will be marked again by
-- the normal preflight; a renamed account can now resolve and update its handle.
UPDATE source_profiles
SET sync_problem_code = NULL,
    sync_problem_message = NULL,
    sync_problem_at = NULL,
    ready_for_download = 1
WHERE provider = 'instagram'
  AND deleted_at IS NULL
  AND sync_problem_code = 'instagram_username_unresolvable'
  AND json_valid(sync_options_json)
  AND NULLIF(
        TRIM(json_extract(sync_options_json, '$.instagram.userIdHint')),
        ''
      ) IS NOT NULL;
