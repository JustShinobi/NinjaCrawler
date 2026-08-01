use super::*;

/// A profile with no new media for this long is considered stalled.
const STALLED_AFTER_DAYS: i64 = 60;

pub(crate) fn load_library_dashboard() -> Result<LibraryDashboard, String> {
    with_workspace(|connection, _| load_library_dashboard_with_connection(connection))
}

pub(super) fn load_library_dashboard_with_connection(
    connection: &Connection,
) -> Result<LibraryDashboard, String> {
    let counts = media_index_counts(connection)?;

    let mut providers = Vec::new();
    {
        let mut statement = connection
            .prepare(
                "SELECT provider, COUNT(*), COALESCE(SUM(size_bytes), 0), COUNT(DISTINCT source_id)
                 FROM media_index
                 WHERE local_state = 'present'
                 GROUP BY provider
                 ORDER BY SUM(size_bytes) DESC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(LibraryProviderBreakdown {
                    provider: row.get(0)?,
                    files: row.get(1)?,
                    bytes: row.get(2)?,
                    sources: row.get(3)?,
                })
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            providers.push(row.map_err(|error| error.to_string())?);
        }
    }

    let mut top_profiles = Vec::new();
    {
        let mut statement = connection
            .prepare(
                "SELECT media_index.source_id, media_index.provider, source_profiles.handle,
                        COUNT(*), COALESCE(SUM(media_index.size_bytes), 0),
                        MAX(media_index.captured_at)
                 FROM media_index
                 JOIN source_profiles ON source_profiles.id = media_index.source_id
                 WHERE media_index.local_state = 'present' AND source_profiles.deleted_at IS NULL
                 GROUP BY media_index.source_id
                 ORDER BY SUM(media_index.size_bytes) DESC
                 LIMIT 20",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(LibraryProfileUsage {
                    source_id: row.get(0)?,
                    provider: row.get(1)?,
                    handle: row.get(2)?,
                    files: row.get(3)?,
                    bytes: row.get(4)?,
                    last_captured_at: row.get(5)?,
                })
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            top_profiles.push(row.map_err(|error| error.to_string())?);
        }
    }

    let mut growth = Vec::new();
    {
        // Grouped on the capture date, not the download date: the shape of the
        // archive follows when the content was posted.
        let mut statement = connection
            .prepare(
                "SELECT strftime('%Y-%m', captured_at, 'unixepoch') AS month,
                        COUNT(*), COALESCE(SUM(size_bytes), 0)
                 FROM media_index
                 WHERE local_state = 'present' AND captured_at IS NOT NULL
                 GROUP BY month
                 ORDER BY month DESC
                 LIMIT 24",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(LibraryGrowthPoint {
                    month: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    files: row.get(1)?,
                    bytes: row.get(2)?,
                })
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            growth.push(row.map_err(|error| error.to_string())?);
        }
        growth.reverse();
    }

    let (variant_groups, variant_reclaimable_bytes) = connection
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM media_variant_groups),
                COALESCE((
                    SELECT SUM(media_index.size_bytes)
                    FROM media_index
                    WHERE media_index.variant_group_id IS NOT NULL
                      AND media_index.is_canonical = 0
                ), 0)",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .unwrap_or((0, 0));

    let stalled_profiles = load_stalled_profiles(connection)?;

    Ok(LibraryDashboard {
        total_files: counts.total_files,
        total_bytes: counts.total_bytes,
        total_sources: counts.indexed_sources,
        upstream_missing: counts.upstream_missing,
        pending_fingerprints: counts.pending_fingerprints,
        variant_groups,
        variant_reclaimable_bytes,
        providers,
        top_profiles,
        growth,
        stalled_profiles,
    })
}

/// Profiles with nothing new for a while, split by whether the sync is broken
/// or the person simply stopped posting. The two need opposite actions, and
/// conflating them is why "silent profile" lists get ignored.
fn load_stalled_profiles(
    connection: &Connection,
) -> Result<Vec<LibraryStalledProfile>, String> {
    let now = Utc::now().timestamp();
    let mut statement = connection
        .prepare(
            "SELECT source_profiles.id, source_profiles.provider, source_profiles.handle,
                    source_profiles.sync_problem_code,
                    (SELECT MAX(media_index.captured_at) FROM media_index
                      WHERE media_index.source_id = source_profiles.id),
                    (SELECT source_sync_runs.status FROM source_sync_runs
                      WHERE source_sync_runs.source_id = source_profiles.id
                      ORDER BY source_sync_runs.started_at DESC LIMIT 1)
             FROM source_profiles
             WHERE source_profiles.deleted_at IS NULL",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(|error| error.to_string())?;

    let mut stalled = Vec::new();
    for row in rows {
        let (source_id, provider, handle, problem_code, last_captured, last_status) =
            row.map_err(|error| error.to_string())?;
        let days = last_captured.map(|value| (now - value) / 86_400);
        let sync_failing = problem_code.is_some()
            || last_status
                .as_deref()
                .is_some_and(|status| status.eq_ignore_ascii_case("failed"));
        let quiet = days.is_none_or(|value| value >= STALLED_AFTER_DAYS);
        if !sync_failing && !quiet {
            continue;
        }
        stalled.push(LibraryStalledProfile {
            source_id,
            provider,
            handle,
            reason: if sync_failing {
                "sync_failing".to_string()
            } else {
                "not_posting".to_string()
            },
            days_since_last_media: days,
            last_sync_status: last_status,
            sync_problem_code: problem_code,
        });
    }
    // Broken syncs first: those are the ones an operator can act on.
    stalled.sort_by(|left, right| {
        right
            .reason
            .cmp(&left.reason)
            .then(right.days_since_last_media.cmp(&left.days_since_last_media))
    });
    stalled.truncate(30);
    Ok(stalled)
}
