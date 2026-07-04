use rusqlite::{Connection, OptionalExtension};
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
];

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

    Ok(connection)
}
