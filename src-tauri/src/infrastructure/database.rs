use rusqlite::{Connection, OptionalExtension};
use std::collections::HashSet;
use std::path::Path;

const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../../migrations/0001_initial.sql")),
    (
        2,
        include_str!("../../migrations/0002_scheduler_runtime.sql"),
    ),
    (
        3,
        include_str!("../../migrations/0003_feed_library_runtime.sql"),
    ),
    (
        4,
        include_str!("../../migrations/0004_provider_account_settings.sql"),
    ),
    (
        5,
        include_str!("../../migrations/0005_source_profile_image.sql"),
    ),
    (
        6,
        include_str!("../../migrations/0006_connector_runtimes.sql"),
    ),
    (
        7,
        include_str!("../../migrations/0007_source_profile_soft_delete.sql"),
    ),
    (
        8,
        include_str!("../../migrations/0008_instagram_internal_sync.sql"),
    ),
    (
        9,
        include_str!("../../migrations/0009_connector_runtimes_nullable_active.sql"),
    ),
    (
        10,
        include_str!("../../migrations/0010_external_import_ledger.sql"),
    ),
    (
        11,
        include_str!("../../migrations/0011_instagram_media_identity.sql"),
    ),
    (
        12,
        include_str!("../../migrations/0012_remove_feed_library.sql"),
    ),
    (
        13,
        include_str!("../../migrations/0013_instagram_sync_ledger.sql"),
    ),
    (
        14,
        include_str!("../../migrations/0014_instagram_post_ledger.sql"),
    ),
    (
        15,
        include_str!("../../migrations/0015_scheduler_parity.sql"),
    ),
    (
        16,
        include_str!("../../migrations/0016_source_profile_group.sql"),
    ),
    (
        17,
        include_str!("../../migrations/0017_source_sync_problem.sql"),
    ),
    (
        18,
        include_str!("../../migrations/0018_instagram_media_naming_ledger.sql"),
    ),
    (
        19,
        include_str!("../../migrations/0019_instagram_media_key_aliases.sql"),
    ),
    (
        20,
        include_str!("../../migrations/0020_instagram_media_fingerprints.sql"),
    ),
    (
        21,
        include_str!("../../migrations/0021_source_profile_import_metadata.sql"),
    ),
    (
        22,
        include_str!("../../migrations/0022_source_sync_queue_persistence.sql"),
    ),
    (
        23,
        include_str!("../../migrations/0023_provider_sync_ledgers.sql"),
    ),
    (
        24,
        include_str!("../../migrations/0024_source_sync_queue_order.sql"),
    ),
    (
        25,
        include_str!("../../migrations/0025_media_ledger_post_link.sql"),
    ),
    (
        26,
        include_str!("../../migrations/0026_instagram_media_post_code.sql"),
    ),
    (27, include_str!("../../migrations/0027_deleted_media.sql")),
    (
        28,
        include_str!("../../migrations/0028_instagram_highlight_membership.sql"),
    ),
    (
        29,
        include_str!("../../migrations/0029_instagram_highlight_media_membership.sql"),
    ),
    (
        30,
        include_str!("../../migrations/0030_companion_account_import.sql"),
    ),
    (
        31,
        include_str!("../../migrations/0031_instagram_identity_hint_backfill.sql"),
    ),
    (32, include_str!("../../migrations/0032_single_videos.sql")),
    (
        33,
        include_str!("../../migrations/0033_instagram_identity_hint_reconcile.sql"),
    ),
    (
        34,
        include_str!("../../migrations/0034_account_sync_media_ledger.sql"),
    ),
    (
        35,
        include_str!("../../migrations/0035_account_sync_scope_state.sql"),
    ),
    (
        36,
        include_str!("../../migrations/0036_provider_post_stats.sql"),
    ),
    (
        37,
        include_str!("../../migrations/0037_source_sync_queue_job_keys.sql"),
    ),
    (
        38,
        include_str!("../../migrations/0038_provider_sync_resume_cursor.sql"),
    ),
    (
        39,
        include_str!("../../migrations/0039_provider_sync_resume_schema_repair.sql"),
    ),
];

const PROVIDER_SYNC_RESUME_SCHEMA: &str =
    include_str!("../../migrations/0039_provider_sync_resume_schema_repair.sql");

fn table_columns(connection: &Connection, table: &str) -> rusqlite::Result<HashSet<String>> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    rows.collect()
}

fn ensure_provider_sync_resume_schema(connection: &Connection) -> rusqlite::Result<()> {
    let resume_columns = table_columns(connection, "provider_sync_resume_cursors")?;
    let required_resume_columns = [
        "provider",
        "source_id",
        "account_id",
        "scope",
        "section",
        "state",
        "cursor",
        "updated_at",
    ];
    if !resume_columns.is_empty()
        && !required_resume_columns
            .iter()
            .all(|column| resume_columns.contains(*column))
    {
        // Essas tabelas guardam apenas checkpoints temporarios. Um draft
        // incompatível nao pode impedir toda a fila de sincronizar.
        connection.execute_batch(
            "DROP TABLE IF EXISTS provider_sync_resume_cursors;
             DROP INDEX IF EXISTS idx_provider_sync_resume_account;",
        )?;
    }

    let hold_columns = table_columns(connection, "provider_sync_account_holds")?;
    let required_hold_columns = [
        "provider",
        "account_id",
        "hold_until",
        "reason",
        "updated_at",
    ];
    if !hold_columns.is_empty()
        && !required_hold_columns
            .iter()
            .all(|column| hold_columns.contains(*column))
    {
        connection.execute_batch("DROP TABLE IF EXISTS provider_sync_account_holds;")?;
    }

    connection.execute_batch(PROVIDER_SYNC_RESUME_SCHEMA)?;
    connection.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_provider_sync_resume_account
         ON provider_sync_resume_cursors(provider, account_id, scope, updated_at);",
    )
}

pub fn open_connection(path: &Path) -> rusqlite::Result<Connection> {
    let mut connection = Connection::open(path)?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    // Cada sync (por provider) abre sua própria conexão e roda em paralelo. WAL
    // deixa leitores e o escritor coexistirem; busy_timeout faz escritas
    // concorrentes esperarem em vez de falhar com SQLITE_BUSY.
    connection.busy_timeout(std::time::Duration::from_secs(15))?;
    let _ = connection.pragma_update(None, "journal_mode", "WAL");
    let _ = connection.pragma_update(None, "synchronous", "NORMAL");
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );",
    )?;

    for (version, sql) in MIGRATIONS {
        let already_applied = connection
            .query_row(
                "SELECT 1 FROM schema_migrations WHERE version = ?1 LIMIT 1",
                [version],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .is_some();

        if already_applied {
            continue;
        }

        let transaction = connection.transaction()?;
        transaction.execute_batch(sql)?;
        transaction.execute(
            "INSERT INTO schema_migrations (version) VALUES (?1)",
            [version],
        )?;
        transaction.commit()?;
    }

    // Invariante operacional: migrations de branches de desenvolvimento podem
    // colidir por versao. A fila nao deve falhar só porque o ledger diz que a
    // migration passou enquanto as tabelas requeridas estao ausentes.
    ensure_provider_sync_resume_schema(&connection)?;

    Ok(connection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    #[test]
    fn open_connection_repairs_registered_migration_with_missing_runtime_tables() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("repair.db");
        let raw = Connection::open(&path).expect("raw connection");
        raw.execute_batch(
            "CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
             );",
        )
        .expect("migration ledger");
        for version in 1..=38 {
            raw.execute(
                "INSERT INTO schema_migrations(version) VALUES (?1)",
                [version],
            )
            .expect("registered migration");
        }
        drop(raw);

        let repaired = open_connection(&path).expect("repaired connection");
        let resume = table_columns(&repaired, "provider_sync_resume_cursors").expect("resume");
        let holds = table_columns(&repaired, "provider_sync_account_holds").expect("holds");
        assert!(resume.contains("scope"));
        assert!(resume.contains("state"));
        assert!(holds.contains("hold_until"));
    }

    #[test]
    fn open_connection_replaces_incompatible_draft_resume_table() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("draft.db");
        let raw = Connection::open(&path).expect("raw connection");
        raw.execute_batch(
            "CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE provider_sync_resume_cursors (
                provider TEXT, source_id TEXT, account_id TEXT,
                section TEXT, cursor TEXT, updated_at TEXT
             );",
        )
        .expect("draft schema");
        for version in 1..=39 {
            raw.execute(
                "INSERT INTO schema_migrations(version) VALUES (?1)",
                [version],
            )
            .expect("registered migration");
        }
        drop(raw);

        let repaired = open_connection(&path).expect("repaired connection");
        let columns = table_columns(&repaired, "provider_sync_resume_cursors").expect("columns");
        assert!(columns.contains("scope"));
        assert!(columns.contains("state"));
    }

    #[test]
    fn story_job_key_migration_preserves_old_jobs_and_allows_same_source_twice() {
        let connection = Connection::open_in_memory().expect("in-memory database");
        connection
            .execute_batch(include_str!(
                "../../migrations/0022_source_sync_queue_persistence.sql"
            ))
            .expect("legacy queue table");
        connection
            .execute_batch(include_str!(
                "../../migrations/0024_source_sync_queue_order.sql"
            ))
            .expect("legacy queue order");
        connection
            .execute(
                "INSERT INTO source_sync_queue_jobs
                    (source_id, trigger, queued_at, order_index)
                 VALUES ('source-1', 'manual', '2026-07-11T00:00:00Z', 1)",
                [],
            )
            .expect("legacy job");

        connection
            .execute_batch(include_str!(
                "../../migrations/0037_source_sync_queue_job_keys.sql"
            ))
            .expect("job key migration");

        let preserved: (String, String) = connection
            .query_row(
                "SELECT job_key, source_id FROM source_sync_queue_jobs",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("preserved job");
        assert_eq!(preserved, ("source-1".to_string(), "source-1".to_string()));

        connection
            .execute(
                "INSERT INTO source_sync_queue_jobs
                    (job_key, source_id, trigger, queued_at, order_index)
                 VALUES (?1, ?2, 'companion_story', '2026-07-11T00:01:00Z', 2)",
                params!["source-1:instagram-story:222", "source-1"],
            )
            .expect("second targeted job for same source");
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM source_sync_queue_jobs WHERE source_id = 'source-1'",
                [],
                |row| row.get(0),
            )
            .expect("queue count");
        assert_eq!(count, 2);
    }
}
