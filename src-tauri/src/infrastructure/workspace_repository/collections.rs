use super::*;

fn normalized_kind(kind: Option<&str>) -> String {
    match kind.map(str::trim).unwrap_or("manual") {
        "smart" => "smart".to_string(),
        _ => "manual".to_string(),
    }
}

fn normalized_scope(scope: Option<&str>) -> String {
    match scope.map(str::trim).unwrap_or("global") {
        "source" => "source".to_string(),
        "identity" => "identity".to_string(),
        _ => "global".to_string(),
    }
}

fn collection_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Collection> {
    Ok(Collection {
        id: row.get(0)?,
        kind: row.get(1)?,
        scope: row.get(2)?,
        scope_ref_id: row.get(3)?,
        name: row.get(4)?,
        description: row.get(5)?,
        color: row.get(6)?,
        rule_json: row.get(7)?,
        cover_media_id: row.get(8)?,
        pinned: row.get::<_, i64>(9)? != 0,
        item_count: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

const COLLECTION_COLUMNS: &str = "collections.id, collections.kind, collections.scope,
     collections.scope_ref_id, collections.name, collections.description, collections.color,
     collections.rule_json, collections.cover_media_id, collections.pinned,
     (SELECT COUNT(*) FROM collection_items WHERE collection_items.collection_id = collections.id),
     collections.created_at, collections.updated_at";

/// Collections visible in a given place. `scope=None` lists everything, which is
/// what the library window shows.
pub(crate) fn list_collections(
    scope: Option<String>,
    scope_ref_id: Option<String>,
) -> Result<Vec<Collection>, String> {
    with_workspace(|connection, _| {
        list_collections_with_connection(connection, scope.as_deref(), scope_ref_id.as_deref())
    })
}

pub(super) fn list_collections_with_connection(
    connection: &Connection,
    scope: Option<&str>,
    scope_ref_id: Option<&str>,
) -> Result<Vec<Collection>, String> {
    let statement = format!(
        "SELECT {COLLECTION_COLUMNS}
         FROM collections
         WHERE (?1 IS NULL OR collections.scope = ?1)
           AND (?2 IS NULL OR collections.scope_ref_id = ?2)
         ORDER BY collections.pinned DESC, collections.name COLLATE NOCASE"
    );
    let mut prepared = connection
        .prepare(&statement)
        .map_err(|error| error.to_string())?;
    let rows = prepared
        .query_map(params![scope, scope_ref_id], collection_from_row)
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

pub(crate) fn upsert_collection(input: CollectionUpsert) -> Result<Collection, String> {
    with_workspace(|connection, _| upsert_collection_with_connection(connection, input.clone()))
}

pub(super) fn upsert_collection_with_connection(
    connection: &Connection,
    input: CollectionUpsert,
) -> Result<Collection, String> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("A collection needs a name.".to_string());
    }
    let scope = normalized_scope(input.scope.as_deref());
    if scope != "global" && input.scope_ref_id.as_deref().unwrap_or("").trim().is_empty() {
        return Err("A profile or identity collection needs the owner it belongs to.".to_string());
    }

    let now = Utc::now().to_rfc3339();
    let id = input
        .id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    connection
        .execute(
            "INSERT INTO collections (
                id, kind, scope, scope_ref_id, name, description, color, rule_json,
                pinned, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
             ON CONFLICT(id) DO UPDATE SET
                kind = excluded.kind,
                scope = excluded.scope,
                scope_ref_id = excluded.scope_ref_id,
                name = excluded.name,
                description = excluded.description,
                color = excluded.color,
                rule_json = excluded.rule_json,
                pinned = excluded.pinned,
                updated_at = excluded.updated_at",
            params![
                id,
                normalized_kind(input.kind.as_deref()),
                scope,
                input
                    .scope_ref_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty()),
                name,
                input.description.as_deref().map(str::trim),
                input.color.as_deref().map(str::trim),
                input.rule_json.as_deref(),
                i64::from(input.pinned.unwrap_or(false)),
                now,
            ],
        )
        .map_err(|error| error.to_string())?;

    load_collection_with_connection(connection, &id)
}

pub(super) fn load_collection_with_connection(
    connection: &Connection,
    collection_id: &str,
) -> Result<Collection, String> {
    let statement = format!("SELECT {COLLECTION_COLUMNS} FROM collections WHERE collections.id = ?1");
    connection
        .query_row(&statement, params![collection_id], collection_from_row)
        .map_err(|error| error.to_string())
}

pub(crate) fn delete_collection(collection_id: String) -> Result<(), String> {
    with_workspace(|connection, _| {
        // Members are removed by cascade; the media on disk is untouched.
        connection
            .execute(
                "DELETE FROM collections WHERE id = ?1",
                params![collection_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

/// Turns a profile (or identity) collection into a library-wide one without
/// losing its items — the whole reason scope is a column and not a table.
pub(crate) fn promote_collection_to_global(collection_id: String) -> Result<Collection, String> {
    with_workspace(|connection, _| {
        connection
            .execute(
                "UPDATE collections
                 SET scope = 'global', scope_ref_id = NULL, updated_at = ?2
                 WHERE id = ?1",
                params![collection_id, Utc::now().to_rfc3339()],
            )
            .map_err(|error| error.to_string())?;
        load_collection_with_connection(connection, &collection_id)
    })
}

fn insert_members(
    connection: &Connection,
    collection_id: &str,
    media_ids: &[String],
) -> Result<usize, String> {
    let now = Utc::now().to_rfc3339();
    let mut added = 0;
    for media_id in media_ids {
        added += connection
            .execute(
                "INSERT INTO collection_items (collection_id, media_id, added_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(collection_id, media_id) DO NOTHING",
                params![collection_id, media_id, now],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(added)
}

/// Adds media the profile gallery is showing. The gallery works in relative
/// paths (it is derived from disk), so the index ids are resolved here instead
/// of leaking them into the UI.
pub(crate) fn add_profile_media_to_collection(
    collection_id: String,
    source_id: String,
    relative_paths: Vec<String>,
) -> Result<i64, String> {
    with_workspace(|connection, _| {
        let mut media_ids = Vec::new();
        for relative_path in &relative_paths {
            let normalized = relative_path.trim().replace('\\', "/").to_ascii_lowercase();
            if normalized.is_empty() {
                continue;
            }
            let found = connection
                .query_row(
                    "SELECT id FROM media_index WHERE source_id = ?1 AND relative_path = ?2",
                    params![source_id, normalized],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            if let Some(id) = found {
                media_ids.push(id);
            }
        }
        let added = insert_members(connection, &collection_id, &media_ids)?;
        Ok(added as i64)
    })
}

/// Adds whole posts picked from the timeline. A timeline card stands for every
/// file of its post, so the post key is expanded back into its files.
pub(crate) fn add_timeline_items_to_collection(
    collection_id: String,
    item_ids: Vec<String>,
) -> Result<i64, String> {
    with_workspace(|connection, _| {
        let mut media_ids: Vec<String> = Vec::new();
        for item_id in &item_ids {
            let anchor = connection
                .query_row(
                    "SELECT source_id, provider_post_key FROM media_index WHERE id = ?1",
                    params![item_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                        ))
                    },
                )
                .optional()
                .map_err(|error| error.to_string())?;
            let Some((source_id, post_key)) = anchor else {
                continue;
            };
            match post_key.filter(|value| !value.trim().is_empty()) {
                Some(post_key) => {
                    let mut statement = connection
                        .prepare(
                            "SELECT id FROM media_index
                             WHERE source_id = ?1 AND provider_post_key = ?2",
                        )
                        .map_err(|error| error.to_string())?;
                    let rows = statement
                        .query_map(params![source_id, post_key], |row| row.get::<_, String>(0))
                        .map_err(|error| error.to_string())?;
                    for row in rows {
                        media_ids.push(row.map_err(|error| error.to_string())?);
                    }
                }
                // No post key: the card stands for a single file.
                None => media_ids.push(item_id.clone()),
            }
        }
        let added = insert_members(connection, &collection_id, &media_ids)?;
        Ok(added as i64)
    })
}

pub(crate) fn remove_timeline_items_from_collection(
    collection_id: String,
    item_ids: Vec<String>,
) -> Result<i64, String> {
    with_workspace(|connection, _| {
        let mut removed = 0;
        for item_id in &item_ids {
            // Remove the whole post, mirroring how it was added.
            removed += connection
                .execute(
                    "DELETE FROM collection_items
                     WHERE collection_id = ?1
                       AND media_id IN (
                           SELECT sibling.id FROM media_index sibling
                            JOIN media_index anchor ON anchor.id = ?2
                            WHERE sibling.source_id = anchor.source_id
                              AND (
                                  (anchor.provider_post_key IS NOT NULL
                                   AND sibling.provider_post_key = anchor.provider_post_key)
                                  OR sibling.id = anchor.id
                              )
                       )",
                    params![collection_id, item_id],
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(removed as i64)
    })
}

/// Relative paths of a collection's members that belong to one profile.
///
/// The profile gallery is derived from disk and works in relative paths, so it
/// filters by membership without ever seeing an index id.
pub(crate) fn load_collection_relative_paths(
    collection_id: String,
    source_id: String,
) -> Result<Vec<String>, String> {
    with_workspace(|connection, _| {
        let mut statement = connection
            .prepare(
                "SELECT media_index.relative_path
                 FROM collection_items
                 JOIN media_index ON media_index.id = collection_items.media_id
                 WHERE collection_items.collection_id = ?1 AND media_index.source_id = ?2",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![collection_id, source_id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    })
}

/// Resolves a collection into a timeline page.
///
/// Manual collections filter by membership, smart collections expand their
/// stored rule. Both end up in the same keyset-paginated query, so a collection
/// scrolls exactly like the timeline it came from.
pub(crate) fn load_collection_timeline(
    collection_id: String,
    cursor: Option<MediaTimelineCursor>,
    limit: Option<u32>,
) -> Result<MediaTimelinePage, String> {
    with_workspace(|connection, layout| {
        let collection = load_collection_with_connection(connection, &collection_id)?;
        let mut filter = if collection.kind == "smart" {
            collection
                .rule_json
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(|value| serde_json::from_str::<MediaTimelineFilter>(value))
                .transpose()
                .map_err(|error| format!("This smart collection has an unreadable rule: {error}"))?
                .unwrap_or_default()
        } else {
            MediaTimelineFilter {
                collection_id: Some(collection.id.clone()),
                ..Default::default()
            }
        };

        // A scoped collection never leaks media from outside its owner, even if
        // a saved rule says otherwise.
        if collection.scope == "source" {
            if let Some(source_id) = collection.scope_ref_id.clone() {
                filter.source_ids = vec![source_id];
            }
        } else if collection.scope == "identity" {
            if let Some(identity_id) = collection.scope_ref_id.clone() {
                filter.identity_ids = vec![identity_id];
            }
        }

        load_media_timeline_with_connection(
            connection,
            layout,
            MediaTimelineRequest {
                filter,
                cursor,
                limit,
            },
        )
    })
}
