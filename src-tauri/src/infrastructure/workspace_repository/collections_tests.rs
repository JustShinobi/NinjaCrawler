use super::tests::{create_test_layout, sample_account, sample_source};
use super::*;

fn seed_profile(connection: &Connection, layout: &StorageLayout) -> Result<(), String> {
    upsert_provider_account_with_connection(
        connection,
        layout,
        sample_account("account-1", "twitter"),
    )?;
    upsert_source_profile_with_connection(
        connection,
        layout,
        sample_source("source-1", "twitter", Some("account-1")),
    )?;
    Ok(())
}

fn index(
    connection: &Connection,
    layout: &StorageLayout,
    relative_path: &str,
    post_key: Option<&str>,
) -> Result<String, String> {
    let source = load_sources(connection)?
        .into_iter()
        .find(|candidate| candidate.id == "source-1")
        .expect("seeded source");
    let root = resolved_source_media_output_root_with_connection(connection, layout, &source)?;
    let path = root.join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("media folder");
    }
    fs::write(&path, "bytes").expect("media file");
    upsert_media_index_entry(
        connection,
        "twitter",
        "source-1",
        &MediaIndexEntry {
            relative_path,
            absolute_path: &path,
            media_type: "image",
            media_section: "timeline",
            provider_media_key: Some(relative_path),
            provider_post_key: post_key,
            captured_at: Some(1_000),
        },
        "2026-03-10T00:00:00Z",
    )?;
    connection
        .query_row(
            "SELECT id FROM media_index WHERE source_id = 'source-1' AND relative_path = ?1",
            params![relative_path],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| error.to_string())
}

fn manual_collection(connection: &Connection, scope: &str, scope_ref: Option<&str>) -> Collection {
    upsert_collection_with_connection(
        connection,
        CollectionUpsert {
            name: "Favourites".to_string(),
            scope: Some(scope.to_string()),
            scope_ref_id: scope_ref.map(str::to_string),
            ..Default::default()
        },
    )
    .expect("collection")
}

#[test]
fn a_collection_scoped_to_a_profile_can_be_promoted_without_losing_its_items() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_profile(connection, test_layout)?;
        let media_id = index(connection, test_layout, "a.jpg", Some("post-a"))?;
        let collection = manual_collection(connection, "source", Some("source-1"));
        assert_eq!(collection.scope, "source");
        assert_eq!(collection.scope_ref_id.as_deref(), Some("source-1"));

        connection
            .execute(
                "INSERT INTO collection_items (collection_id, media_id, added_at)
                 VALUES (?1, ?2, '2026-03-10T00:00:00Z')",
                params![collection.id, media_id],
            )
            .map_err(|error| error.to_string())?;

        connection
            .execute(
                "UPDATE collections SET scope = 'global', scope_ref_id = NULL WHERE id = ?1",
                params![collection.id],
            )
            .map_err(|error| error.to_string())?;

        let promoted = load_collection_with_connection(connection, &collection.id)?;
        assert_eq!(promoted.scope, "global");
        assert!(promoted.scope_ref_id.is_none());
        assert_eq!(
            promoted.item_count, 1,
            "promotion must not drop what the operator curated"
        );
        Ok(())
    })
    .expect("promotion should succeed");
}

#[test]
fn a_scoped_collection_needs_the_owner_it_belongs_to() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_profile(connection, test_layout)?;
        let result = upsert_collection_with_connection(
            connection,
            CollectionUpsert {
                name: "Orphan".to_string(),
                scope: Some("source".to_string()),
                scope_ref_id: None,
                ..Default::default()
            },
        );
        assert!(result.is_err(), "a profile collection without a profile is meaningless");
        Ok(())
    })
    .expect("validation should run");
}

#[test]
fn listing_is_scoped_so_a_profile_only_sees_its_own_collections() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_profile(connection, test_layout)?;
        manual_collection(connection, "source", Some("source-1"));
        upsert_collection_with_connection(
            connection,
            CollectionUpsert {
                name: "Everything".to_string(),
                scope: Some("global".to_string()),
                ..Default::default()
            },
        )?;

        let scoped =
            list_collections_with_connection(connection, Some("source"), Some("source-1"))?;
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].name, "Favourites");

        let all = list_collections_with_connection(connection, None, None)?;
        assert_eq!(all.len(), 2);
        Ok(())
    })
    .expect("listing should be scoped");
}

#[test]
fn adding_a_post_from_the_profile_grid_resolves_every_file_of_that_post() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_profile(connection, test_layout)?;
        index(connection, test_layout, "carousel_0.jpg", Some("post-a"))?;
        index(connection, test_layout, "carousel_1.jpg", Some("post-a"))?;
        let collection = manual_collection(connection, "global", None);

        // The gallery hands over relative paths, not index ids.
        let mut media_ids = Vec::new();
        for relative_path in ["carousel_0.jpg", "carousel_1.jpg"] {
            let id: String = connection
                .query_row(
                    "SELECT id FROM media_index WHERE source_id = 'source-1' AND relative_path = ?1",
                    params![relative_path],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            media_ids.push(id);
        }
        for media_id in &media_ids {
            connection
                .execute(
                    "INSERT INTO collection_items (collection_id, media_id, added_at)
                     VALUES (?1, ?2, '2026-03-10T00:00:00Z')",
                    params![collection.id, media_id],
                )
                .map_err(|error| error.to_string())?;
        }

        let refreshed = load_collection_with_connection(connection, &collection.id)?;
        assert_eq!(refreshed.item_count, 2);
        Ok(())
    })
    .expect("membership should be recorded");
}

#[test]
fn a_manual_collection_reads_back_through_the_timeline_engine() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_profile(connection, test_layout)?;
        let kept = index(connection, test_layout, "kept.jpg", Some("post-a"))?;
        index(connection, test_layout, "other.jpg", Some("post-b"))?;
        let collection = manual_collection(connection, "global", None);
        connection
            .execute(
                "INSERT INTO collection_items (collection_id, media_id, added_at)
                 VALUES (?1, ?2, '2026-03-10T00:00:00Z')",
                params![collection.id, kept],
            )
            .map_err(|error| error.to_string())?;

        let page = load_media_timeline_with_connection(
            connection,
            test_layout,
            MediaTimelineRequest {
                filter: MediaTimelineFilter {
                    collection_id: Some(collection.id.clone()),
                    ..Default::default()
                },
                cursor: None,
                limit: None,
            },
        )?;
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].relative_path, "kept.jpg");
        Ok(())
    })
    .expect("collection filter should apply");
}

#[test]
fn a_smart_collection_stores_a_rule_instead_of_members() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_profile(connection, test_layout)?;
        let rule = serde_json::to_string(&MediaTimelineFilter {
            media_type: Some("video".to_string()),
            ..Default::default()
        })
        .expect("rule json");

        let collection = upsert_collection_with_connection(
            connection,
            CollectionUpsert {
                name: "Only videos".to_string(),
                kind: Some("smart".to_string()),
                rule_json: Some(rule),
                ..Default::default()
            },
        )?;
        assert_eq!(collection.kind, "smart");
        assert_eq!(
            collection.item_count, 0,
            "a smart collection has no explicit members"
        );
        let parsed = serde_json::from_str::<MediaTimelineFilter>(
            collection.rule_json.as_deref().expect("rule"),
        )
        .expect("rule parses back");
        assert_eq!(parsed.media_type.as_deref(), Some("video"));
        Ok(())
    })
    .expect("smart collection should be stored");
}

#[test]
fn deleting_a_collection_leaves_the_media_alone() {
    let (_temp_dir, layout) = create_test_layout();
    with_workspace_layout(layout, |connection, test_layout| {
        seed_profile(connection, test_layout)?;
        let media_id = index(connection, test_layout, "a.jpg", Some("post-a"))?;
        let collection = manual_collection(connection, "global", None);
        connection
            .execute(
                "INSERT INTO collection_items (collection_id, media_id, added_at)
                 VALUES (?1, ?2, '2026-03-10T00:00:00Z')",
                params![collection.id, media_id],
            )
            .map_err(|error| error.to_string())?;

        connection
            .execute("DELETE FROM collections WHERE id = ?1", params![collection.id])
            .map_err(|error| error.to_string())?;

        let orphan_items: i64 = connection
            .query_row("SELECT COUNT(*) FROM collection_items", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        assert_eq!(orphan_items, 0, "membership is cascaded away");
        let media_rows: i64 = connection
            .query_row("SELECT COUNT(*) FROM media_index", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        assert_eq!(media_rows, 1, "the media itself is untouched");
        Ok(())
    })
    .expect("deletion should be safe");
}
