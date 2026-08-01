use super::*;

/// Marks where the operator last caught up with the library.
pub(crate) const TIMELINE_LAST_SEEN_SETTING_KEY: &str = "library.timeline.lastSeenAt";

const DEFAULT_PAGE_SIZE: u32 = 60;
const MAX_PAGE_SIZE: u32 = 200;

/// Builds the `WHERE` fragment and its bindings from the filter.
///
/// Values are bound, never interpolated; only the number of placeholders varies
/// with the filter, so a handle containing quotes can never change the query.
fn filter_clause(
    filter: &MediaTimelineFilter,
    last_seen_unix: Option<i64>,
) -> (String, Vec<Box<dyn rusqlite::ToSql>>) {
    let mut clauses = vec![
        // Files that vanished from disk stay in the index for the health
        // report, but they have nothing to render in a gallery.
        "media_index.local_state = 'present'".to_string(),
        // Non-canonical variants are collapsed into their canonical sibling
        // (F5); until then every row is canonical.
        "media_index.is_canonical = 1".to_string(),
    ];
    let mut bindings: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    let mut push_in = |column: &str, values: &[String], clauses: &mut Vec<String>| {
        if values.is_empty() {
            return;
        }
        let placeholders = values.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        clauses.push(format!("{column} IN ({placeholders})"));
        for value in values {
            bindings.push(Box::new(value.clone()));
        }
    };

    push_in("media_index.provider", &filter.providers, &mut clauses);
    push_in("media_index.source_id", &filter.source_ids, &mut clauses);
    push_in(
        "source_profiles.identity_id",
        &filter.identity_ids,
        &mut clauses,
    );
    push_in(
        "LOWER(TRIM(media_index.media_section))",
        &filter
            .sections
            .iter()
            .map(|section| section.trim().to_ascii_lowercase())
            .collect::<Vec<_>>(),
        &mut clauses,
    );

    if let Some(media_type) = filter
        .media_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("all"))
    {
        clauses.push("media_index.media_type = ?".to_string());
        bindings.push(Box::new(media_type.to_ascii_lowercase()));
    }
    if let Some(from) = filter.captured_from {
        clauses.push("media_index.captured_at >= ?".to_string());
        bindings.push(Box::new(from));
    }
    if let Some(to) = filter.captured_to {
        clauses.push("media_index.captured_at <= ?".to_string());
        bindings.push(Box::new(to));
    }
    if filter.upstream_missing_only {
        clauses.push("media_index.upstream_state = 'missing'".to_string());
    }
    if filter.unseen_only {
        // No "seen" mark yet means everything is new.
        if let Some(last_seen) = last_seen_unix {
            clauses.push("media_index.downloaded_at > ?".to_string());
            bindings.push(Box::new(last_seen));
        }
    }
    if let Some(collection_id) = filter
        .collection_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        clauses.push(
            "EXISTS (SELECT 1 FROM collection_items
                      WHERE collection_items.collection_id = ?
                        AND collection_items.media_id = media_index.id)"
                .to_string(),
        );
        bindings.push(Box::new(collection_id.to_string()));
    }

    (clauses.join(" AND "), bindings)
}

fn last_seen_timestamp(connection: &Connection) -> Option<String> {
    load_app_setting_value(connection, TIMELINE_LAST_SEEN_SETTING_KEY)
        .ok()
        .flatten()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn to_unix(timestamp: Option<&str>) -> Option<i64> {
    timestamp
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.timestamp())
}

/// One page of the aggregated timeline.
///
/// Rows are grouped by post so a carousel is one card, matching the profile
/// grid. Posts with no provider post key (imported libraries, mostly) group by
/// their own row, which is the safe fallback: worst case a post shows as
/// several cards, never two posts merged into one.
pub(crate) fn load_media_timeline(
    request: MediaTimelineRequest,
) -> Result<MediaTimelinePage, String> {
    with_workspace(|connection, layout| {
        load_media_timeline_with_connection(connection, layout, request)
    })
}

pub(super) fn load_media_timeline_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    request: MediaTimelineRequest,
) -> Result<MediaTimelinePage, String> {
    {
        let limit = request
            .limit
            .unwrap_or(DEFAULT_PAGE_SIZE)
            .clamp(1, MAX_PAGE_SIZE);
        let last_seen_at = last_seen_timestamp(connection);
        let last_seen_unix = to_unix(last_seen_at.as_deref());
        let (where_clause, mut bindings) = filter_clause(&request.filter, last_seen_unix);

        let mut cursor_clause = String::new();
        if let Some(cursor) = request.cursor.as_ref() {
            // NULL `captured_at` sorts last, so the cursor comparison has to
            // treat it as the smallest value rather than dropping the rows.
            cursor_clause = " HAVING (COALESCE(MAX(media_index.captured_at), -9223372036854775808), MIN(media_index.id)) < (?, ?)".to_string();
            bindings.push(Box::new(cursor.captured_at.unwrap_or(i64::MIN)));
            bindings.push(Box::new(cursor.id.clone()));
        }
        bindings.push(Box::new(i64::from(limit)));

        let statement = format!(
            "SELECT
                MIN(media_index.id) AS group_id,
                media_index.source_id,
                media_index.provider,
                source_profiles.handle,
                source_profiles.identity_id,
                media_index.provider_post_key,
                MIN(media_index.relative_path) AS relative_path,
                COUNT(*) AS file_count,
                COALESCE(SUM(media_index.size_bytes), 0) AS size_bytes,
                MAX(media_index.captured_at) AS captured_at,
                MIN(media_index.downloaded_at) AS downloaded_at,
                MAX(CASE WHEN media_index.media_type = 'video' THEN 1 ELSE 0 END) AS has_video,
                MIN(media_index.media_section) AS media_section,
                MAX(CASE WHEN media_index.upstream_state = 'missing' THEN 1 ELSE 0 END) AS upstream_missing
             FROM media_index
             JOIN source_profiles ON source_profiles.id = media_index.source_id
             WHERE {where_clause} AND source_profiles.deleted_at IS NULL
             GROUP BY media_index.source_id,
                      COALESCE(media_index.provider_post_key, media_index.id)
             {cursor_clause}
             ORDER BY captured_at DESC, group_id DESC
             LIMIT ?"
        );

        let mut prepared = connection
            .prepare(&statement)
            .map_err(|error| error.to_string())?;
        let bound: Vec<&dyn rusqlite::ToSql> =
            bindings.iter().map(|value| value.as_ref()).collect();
        let rows = prepared
            .query_map(bound.as_slice(), |row| {
                Ok(TimelineRow {
                    id: row.get(0)?,
                    source_id: row.get(1)?,
                    provider: row.get(2)?,
                    handle: row.get(3)?,
                    identity_id: row.get(4)?,
                    post_key: row.get(5)?,
                    relative_path: row.get(6)?,
                    file_count: row.get(7)?,
                    size_bytes: row.get(8)?,
                    captured_at: row.get(9)?,
                    downloaded_at: row.get(10)?,
                    has_video: row.get::<_, i64>(11)? == 1,
                    media_section: row.get::<_, Option<String>>(12)?.unwrap_or_default(),
                    upstream_missing: row.get::<_, i64>(13)? == 1,
                })
            })
            .map_err(|error| error.to_string())?;

        // Profile roots are resolved once per source, not once per row: the
        // lookup reads account settings and would dominate the page otherwise.
        let mut roots: HashMap<String, PathBuf> = HashMap::new();
        let sources = load_sources(connection)?
            .into_iter()
            .map(|source| (source.id.clone(), source))
            .collect::<HashMap<_, _>>();

        let mut items = Vec::new();
        for row in rows {
            let row = row.map_err(|error| error.to_string())?;
            let root = match roots.get(&row.source_id) {
                Some(root) => root.clone(),
                None => {
                    let Some(source) = sources.get(&row.source_id) else {
                        continue;
                    };
                    let root =
                        resolved_source_media_output_root_with_connection(connection, layout, source)?;
                    roots.insert(row.source_id.clone(), root.clone());
                    root
                }
            };
            let absolute_path = root.join(&row.relative_path);
            items.push(MediaTimelineItem {
                id: row.id,
                source_id: row.source_id,
                provider: row.provider,
                handle: row.handle,
                identity_id: row.identity_id,
                post_key: row.post_key,
                media_type: if row.has_video {
                    "video".to_string()
                } else if row.file_count > 1 {
                    "slideshow".to_string()
                } else {
                    "image".to_string()
                },
                media_section: row.media_section,
                captured_at: row.captured_at,
                downloaded_at: row.downloaded_at,
                absolute_path: absolute_path.to_string_lossy().to_string(),
                relative_path: row.relative_path,
                file_count: row.file_count,
                size_bytes: row.size_bytes,
                upstream_missing: row.upstream_missing,
            });
        }

        let next_cursor = (items.len() as u32 == limit)
            .then(|| items.last())
            .flatten()
            .map(|item| MediaTimelineCursor {
                captured_at: item.captured_at,
                id: item.id.clone(),
            });

        let new_since_last_visit = match last_seen_unix {
            Some(last_seen) => connection
                .query_row(
                    "SELECT COUNT(*) FROM media_index
                     WHERE local_state = 'present' AND downloaded_at > ?1",
                    params![last_seen],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0),
            None => 0,
        };

        Ok(MediaTimelinePage {
            items,
            next_cursor,
            new_since_last_visit,
            last_seen_at,
        })
    }
}

struct TimelineRow {
    id: String,
    source_id: String,
    provider: String,
    handle: String,
    identity_id: Option<String>,
    post_key: Option<String>,
    relative_path: String,
    file_count: i64,
    size_bytes: i64,
    captured_at: Option<i64>,
    downloaded_at: Option<i64>,
    has_video: bool,
    media_section: String,
    upstream_missing: bool,
}

/// Records that the operator caught up, so the next visit only highlights what
/// arrived after this moment.
pub(crate) fn mark_timeline_seen() -> Result<String, String> {
    with_workspace(|connection, _| mark_timeline_seen_with_connection(connection))
}

pub(super) fn mark_timeline_seen_with_connection(
    connection: &Connection,
) -> Result<String, String> {
    let now = Utc::now().to_rfc3339();
    upsert_app_setting_value(connection, TIMELINE_LAST_SEEN_SETTING_KEY, &now)?;
    Ok(now)
}
