-- Imported SCrawler profiles can contain a stale or incorrect UserID even when
-- this application has later completed syncs for the correct account. In that
-- conflict, the latest successful manifest is stronger evidence: it records
-- the identity that actually produced this source's downloaded media.
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
    ),
    sync_problem_code = CASE
        WHEN sync_problem_code = 'instagram_username_unresolvable' THEN NULL
        ELSE sync_problem_code
    END,
    sync_problem_message = CASE
        WHEN sync_problem_code = 'instagram_username_unresolvable' THEN NULL
        ELSE sync_problem_message
    END,
    sync_problem_at = CASE
        WHEN sync_problem_code = 'instagram_username_unresolvable' THEN NULL
        ELSE sync_problem_at
    END,
    ready_for_download = CASE
        WHEN sync_problem_code = 'instagram_username_unresolvable' THEN 1
        ELSE ready_for_download
    END
WHERE provider = 'instagram'
  AND deleted_at IS NULL
  AND json_valid(sync_options_json)
  AND NULLIF(
        TRIM(json_extract(sync_options_json, '$.instagram.userIdHint')),
        ''
      ) IS NOT NULL
  AND TRIM(json_extract(sync_options_json, '$.instagram.userIdHint')) <> (
        SELECT TRIM(json_extract(run.manifest_summary_json, '$.profileUserId'))
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
    );
