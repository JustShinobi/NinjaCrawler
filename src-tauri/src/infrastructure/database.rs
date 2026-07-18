use rusqlite::{Connection, OptionalExtension};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

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
    (
        40,
        include_str!("../../migrations/0040_media_path_migration_queue.sql"),
    ),
    (
        41,
        include_str!("../../migrations/0041_connector_runtime_asset_digest.sql"),
    ),
    (
        42,
        include_str!("../../migrations/0042_source_profile_stats.sql"),
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

    // As migrations são versionadas e idempotentes, mas revalidá-las (1 SELECT
    // por migration) em TODA abertura de conexão é puro overhead — cada comando
    // e cada tick de 5s reabrem conexão. Roda uma vez por arquivo de banco por
    // processo. A chave por path preserva o suite hermético dos testes (um
    // banco temporário por teste, cada um migrado na 1ª abertura).
    ensure_migrations(&mut connection, path)?;

    // Invariante operacional: migrations de branches de desenvolvimento podem
    // colidir por versao. A fila nao deve falhar só porque o ledger diz que a
    // migration passou enquanto as tabelas requeridas estao ausentes. Reparo
    // barato, mantido por-abertura de propósito.
    ensure_provider_sync_resume_schema(&connection)?;

    Ok(connection)
}

fn migrated_db_paths() -> &'static Mutex<HashSet<PathBuf>> {
    static APPLIED: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    APPLIED.get_or_init(|| Mutex::new(HashSet::new()))
}

fn ensure_migrations(connection: &mut Connection, path: &Path) -> rusqlite::Result<()> {
    let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    // Segura o lock durante a corrida: só a primeira abertura de cada banco
    // paga as migrations; aberturas concorrentes do mesmo banco esperam em vez
    // de rodar o loop duas vezes.
    let mut applied = migrated_db_paths()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if applied.contains(&key) {
        return Ok(());
    }

    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );",
    )?;

    // Rede de segurança: antes de aplicar QUALQUER migration pendente num banco
    // que já tem schema, grava um snapshot consistente. Migrations não têm
    // "down" e podem ser destrutivas (DROP/rebuild), então um snapshot por salto
    // de versão dá um ponto de restauração manual. Bancos novos (nenhuma
    // migration aplicada) não têm dados a proteger e são pulados.
    let applied_versions: HashSet<i64> = {
        let mut statement = connection.prepare("SELECT version FROM schema_migrations")?;
        let rows = statement.query_map([], |row| row.get::<_, i64>(0))?;
        rows.collect::<rusqlite::Result<HashSet<i64>>>()?
    };
    let has_pending = MIGRATIONS
        .iter()
        .any(|(version, _)| !applied_versions.contains(version));
    if has_pending {
        if let Some(applied_max) = applied_versions.iter().copied().max() {
            backup_database_before_migrations(connection, path, applied_max)?;
        }
    }

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

    applied.insert(key);
    Ok(())
}

/// Quantos snapshots pré-migration manter (os mais antigos são podados).
const MIGRATION_BACKUP_RETENTION: usize = 3;

/// Erro genérico de backup como `rusqlite::Error`, para propagar via `?` e
/// abortar a abertura do banco quando o snapshot não pôde ser gravado.
fn backup_error(message: String) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
        Some(message),
    )
}

/// Grava um snapshot consistente do banco (`VACUUM INTO`, que consolida o WAL)
/// em `<data_dir>/backups/<stem>.pre-v<N>.<timestamp>.db` e mantém só os últimos
/// [`MIGRATION_BACKUP_RETENTION`]. Um erro aqui é fatal de propósito: sem o
/// backup, as migrations não devem rodar.
fn backup_database_before_migrations(
    connection: &Connection,
    db_path: &Path,
    applied_max: i64,
) -> rusqlite::Result<()> {
    let parent = db_path.parent().unwrap_or_else(|| Path::new("."));
    let backups_dir = parent.join("backups");
    std::fs::create_dir_all(&backups_dir).map_err(|error| {
        backup_error(format!(
            "failed to create migration backups directory '{}': {error}",
            backups_dir.display()
        ))
    })?;

    let stem = db_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("database");
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let backup_path = backups_dir.join(format!("{stem}.pre-v{applied_max}.{timestamp}.db"));
    // `VACUUM INTO` exige que o destino ainda não exista.
    if backup_path.exists() {
        let _ = std::fs::remove_file(&backup_path);
    }
    // O caminho vai como literal SQL; aspas simples precisam ser dobradas.
    let target = backup_path.to_string_lossy().replace('\'', "''");
    connection
        .execute(&format!("VACUUM INTO '{target}'"), [])
        .map_err(|error| {
            backup_error(format!(
                "failed to snapshot database into '{}': {error}",
                backup_path.display()
            ))
        })?;

    prune_migration_backups(&backups_dir, stem);
    Ok(())
}

/// Mantém só os [`MIGRATION_BACKUP_RETENTION`] snapshots mais recentes de um
/// mesmo banco, apagando os mais antigos (best-effort — erros são ignorados).
fn prune_migration_backups(backups_dir: &Path, stem: &str) {
    let prefix = format!("{stem}.pre-v");
    let Ok(entries) = std::fs::read_dir(backups_dir) else {
        return;
    };
    let mut backups: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if !name.starts_with(&prefix) || !name.ends_with(".db") {
                return None;
            }
            let modified = entry.metadata().and_then(|meta| meta.modified()).ok()?;
            Some((modified, path))
        })
        .collect();
    if backups.len() <= MIGRATION_BACKUP_RETENTION {
        return;
    }
    // Mais novo primeiro (desempate por nome, que embute a versão/timestamp);
    // tudo além dos N mais recentes é podado.
    backups.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    for (_, path) in backups.into_iter().skip(MIGRATION_BACKUP_RETENTION) {
        let _ = std::fs::remove_file(path);
    }
}

/// Abre uma conexão SEM rodar migrations (as pragmas espelham `open_connection`).
/// Usada pela pré-checagem e pelo runner com progresso — que controlam o momento
/// das migrations explicitamente, fora do caminho automático.
fn open_connection_raw(path: &Path) -> Result<Connection, String> {
    let connection = Connection::open(path).map_err(|error| error.to_string())?;
    let _ = connection.pragma_update(None, "foreign_keys", "ON");
    connection
        .busy_timeout(std::time::Duration::from_secs(15))
        .map_err(|error| error.to_string())?;
    let _ = connection.pragma_update(None, "journal_mode", "WAL");
    let _ = connection.pragma_update(None, "synchronous", "NORMAL");
    Ok(connection)
}

/// Pré-checagem read-only: há migrations pendentes num banco que JÁ tem schema?
/// Não roda nada. `None` para banco novo/sem schema ou já atualizado — nesses
/// casos o boot segue direto (migrations de banco novo são triviais/rápidas).
pub fn migration_precheck(
    db_path: &Path,
) -> Result<Option<crate::domain::models::MigrationStatus>, String> {
    if !db_path.exists() {
        return Ok(None);
    }
    let connection = Connection::open(db_path).map_err(|error| error.to_string())?;
    let has_ledger = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_migrations' LIMIT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .is_some();
    if !has_ledger {
        return Ok(None);
    }
    let applied = load_applied_versions(&connection)?;
    let applied_max = applied.iter().copied().max().unwrap_or(0);
    if applied_max == 0 {
        return Ok(None);
    }
    let pending_count = MIGRATIONS
        .iter()
        .filter(|(version, _)| !applied.contains(version))
        .count();
    if pending_count == 0 {
        return Ok(None);
    }
    let to_version = MIGRATIONS
        .iter()
        .map(|(version, _)| *version)
        .max()
        .unwrap_or(applied_max);
    let db_size_bytes = std::fs::metadata(db_path).map(|meta| meta.len()).unwrap_or(0);
    Ok(Some(crate::domain::models::MigrationStatus {
        from_version: applied_max,
        to_version,
        pending_count,
        db_size_bytes,
    }))
}

fn load_applied_versions(connection: &Connection) -> Result<HashSet<i64>, String> {
    let mut statement = connection
        .prepare("SELECT version FROM schema_migrations")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<HashSet<i64>>>()
        .map_err(|error| error.to_string())
}

/// Aplica as migrations pendentes com backup prévio (snapshot online do rusqlite,
/// que reporta progresso por página) e emite progresso via `on_progress`
/// (fase `backup` e depois `migrate` X/N). É o caminho usado pela tela de
/// migração da janela principal.
pub fn run_pending_migrations_with_progress<F>(
    db_path: &Path,
    mut on_progress: F,
) -> Result<(), String>
where
    F: FnMut(crate::domain::models::MigrationProgress),
{
    let mut connection = open_connection_raw(db_path)?;
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
             );",
        )
        .map_err(|error| error.to_string())?;

    let applied = load_applied_versions(&connection)?;
    let pending: Vec<&(i64, &str)> = MIGRATIONS
        .iter()
        .filter(|(version, _)| !applied.contains(version))
        .collect();
    if pending.is_empty() {
        return Ok(());
    }

    let applied_max = applied.iter().copied().max().unwrap_or(0);
    if applied_max > 0 {
        backup_with_progress(&connection, db_path, applied_max, &mut on_progress)?;
    }

    let total = pending.len() as u64;
    for (index, (version, sql)) in pending.iter().enumerate() {
        on_progress(crate::domain::models::MigrationProgress {
            phase: "migrate".to_string(),
            current: index as u64,
            total,
            label: format!("Applying update {version}"),
        });
        let transaction = connection.transaction().map_err(|error| error.to_string())?;
        transaction
            .execute_batch(sql)
            .map_err(|error| error.to_string())?;
        transaction
            .execute("INSERT INTO schema_migrations (version) VALUES (?1)", [version])
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
    }
    on_progress(crate::domain::models::MigrationProgress {
        phase: "migrate".to_string(),
        current: total,
        total,
        label: "Finishing up".to_string(),
    });
    Ok(())
}

/// Snapshot consistente do banco via a API de backup online do SQLite, copiando
/// em passos de páginas para reportar progresso (diferente do `VACUUM INTO`, que
/// é atômico mas sem progresso). Mesma nomenclatura/retentção do backup silencioso.
fn backup_with_progress<F>(
    source: &Connection,
    db_path: &Path,
    applied_max: i64,
    on_progress: &mut F,
) -> Result<(), String>
where
    F: FnMut(crate::domain::models::MigrationProgress),
{
    let parent = db_path.parent().unwrap_or_else(|| Path::new("."));
    let backups_dir = parent.join("backups");
    std::fs::create_dir_all(&backups_dir)
        .map_err(|error| format!("failed to create migration backups directory: {error}"))?;
    let stem = db_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("database");
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let backup_path = backups_dir.join(format!("{stem}.pre-v{applied_max}.{timestamp}.db"));
    if backup_path.exists() {
        let _ = std::fs::remove_file(&backup_path);
    }

    let mut destination = Connection::open(&backup_path).map_err(|error| error.to_string())?;
    {
        let backup = rusqlite::backup::Backup::new(source, &mut destination)
            .map_err(|error| error.to_string())?;
        // ~512 páginas por passo: progresso suave mesmo num banco de ~1GB.
        loop {
            let result = backup.step(512).map_err(|error| error.to_string())?;
            let progress = backup.progress();
            let total = progress.pagecount.max(0) as u64;
            let done = (progress.pagecount - progress.remaining).max(0) as u64;
            on_progress(crate::domain::models::MigrationProgress {
                phase: "backup".to_string(),
                current: done,
                total,
                label: "Backing up your database".to_string(),
            });
            match result {
                rusqlite::backup::StepResult::Done => break,
                rusqlite::backup::StepResult::Busy | rusqlite::backup::StepResult::Locked => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                // `More` e quaisquer variantes futuras: continua copiando.
                _ => {}
            }
        }
    }
    prune_migration_backups(&backups_dir, stem);
    Ok(())
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
        // Registra TODAS as migrações como aplicadas (sem rodá-las): o teste
        // simula um ledger cheio com as tabelas de runtime ausentes. Iterar a
        // constante em vez de um número fixo mantém o teste válido conforme
        // novas migrações são adicionadas (algumas alteram tabelas antigas e
        // falhariam contra este DB esqueleto se fossem re-executadas).
        for &(version, _) in MIGRATIONS {
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
    fn migration_precheck_and_progress_runner_backup_and_apply() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("ninjacrawler.db");
        let max_version = MIGRATIONS.iter().map(|(v, _)| *v).max().expect("migrations");
        {
            let connection = Connection::open(&path).expect("open");
            connection
                .execute_batch(
                    "CREATE TABLE schema_migrations (
                        version INTEGER PRIMARY KEY,
                        applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                     );
                     CREATE TABLE source_profiles (id TEXT PRIMARY KEY);
                     INSERT INTO source_profiles(id) VALUES ('seed');",
                )
                .expect("seed schema");
            // Marca todas menos a ÚLTIMA como aplicadas → só a última fica pendente.
            for (version, _) in MIGRATIONS.iter().filter(|(v, _)| *v < max_version) {
                connection
                    .execute("INSERT INTO schema_migrations(version) VALUES (?1)", [version])
                    .expect("seed version");
            }
        }

        // Pré-checagem detecta a pendência (sem rodar nada).
        let status = migration_precheck(&path)
            .expect("precheck ok")
            .expect("should be pending");
        assert_eq!(status.pending_count, 1);
        assert_eq!(status.to_version, max_version);
        assert!(status.db_size_bytes > 0);

        // Aplica com progresso (backup online + migrations).
        let mut phases: Vec<String> = Vec::new();
        run_pending_migrations_with_progress(&path, |progress| phases.push(progress.phase))
            .expect("migrate ok");
        assert!(phases.iter().any(|p| p == "backup"), "reports backup progress");
        assert!(phases.iter().any(|p| p == "migrate"), "reports migrate progress");

        // A última migration foi registrada e um snapshot foi gravado.
        let connection = Connection::open(&path).expect("reopen");
        let applied_last: Option<i64> = connection
            .query_row(
                "SELECT 1 FROM schema_migrations WHERE version = ?1",
                [max_version],
                |row| row.get(0),
            )
            .optional()
            .expect("query");
        assert!(applied_last.is_some(), "last migration should be applied");
        let backups = std::fs::read_dir(temp.path().join("backups"))
            .expect("backups dir")
            .flatten()
            .count();
        assert!(backups >= 1, "a backup snapshot should exist");

        // Já atualizado: a pré-checagem não acusa mais pendência.
        assert!(migration_precheck(&path).expect("precheck2").is_none());
    }

    #[test]
    fn migration_precheck_is_none_for_fresh_or_updated_db() {
        let temp = tempfile::tempdir().expect("tempdir");
        // Banco novo (sem arquivo) → None.
        let missing = temp.path().join("missing.db");
        assert!(migration_precheck(&missing).expect("precheck").is_none());
        // Banco totalmente migrado (open_connection aplica tudo) → None.
        let path = temp.path().join("fresh.db");
        let _ = open_connection(&path).expect("open");
        assert!(migration_precheck(&path).expect("precheck").is_none());
    }

    #[test]
    fn migration_backup_snapshots_db_and_retains_last_three() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("ninjacrawler.db");
        let connection = Connection::open(&path).expect("open");
        connection
            .execute_batch("CREATE TABLE t (v INTEGER); INSERT INTO t(v) VALUES (7);")
            .expect("seed data");

        // Quatro "saltos de versão" → 4 snapshots, mas a retenção mantém 3.
        for version in 1..=4 {
            backup_database_before_migrations(&connection, &path, version).expect("backup ok");
        }

        let backups_dir = temp.path().join("backups");
        let mut files: Vec<String> = std::fs::read_dir(&backups_dir)
            .expect("backups dir exists")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.starts_with("ninjacrawler.pre-v") && name.ends_with(".db"))
            .collect();
        files.sort();
        assert_eq!(files.len(), 3, "keeps only the newest three backups: {files:?}");
        assert!(
            !files.iter().any(|name| name.contains(".pre-v1.")),
            "the oldest backup (v1) should have been pruned: {files:?}"
        );

        // O snapshot é uma cópia consistente e legível, com os dados originais.
        let newest = backups_dir.join(files.last().expect("a backup file"));
        let restored = Connection::open(&newest).expect("open backup");
        let value: i64 = restored
            .query_row("SELECT v FROM t", [], |row| row.get(0))
            .expect("data present in snapshot");
        assert_eq!(value, 7);
    }

    #[test]
    fn open_connection_migrates_once_per_path_and_serves_reopens() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("once.db");

        // 1ª abertura roda as migrations.
        {
            let connection = open_connection(&path).expect("first open");
            let applied: i64 = connection
                .query_row("SELECT count(*) FROM schema_migrations", [], |row| {
                    row.get(0)
                })
                .expect("migration ledger populated");
            assert!(applied >= 1, "migrations should run on the first open");
        }

        // 2ª abertura do MESMO path pula o loop (guard), mas o schema migrado
        // persiste no arquivo — a conexão continua funcional.
        {
            let connection = open_connection(&path).expect("reopen same path");
            connection
                .query_row("SELECT count(*) FROM source_profiles", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("migrated table still queryable after reopen");
        }

        // Um path NOVO migra de forma independente — o guard é por-arquivo, não
        // global (garante que o suite hermético, com um banco por teste, não
        // fica sem migrations).
        let other = temp.path().join("other.db");
        let connection = open_connection(&other).expect("second path open");
        connection
            .query_row("SELECT count(*) FROM source_profiles", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("a fresh db path migrates on its own first open");
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
        for &(version, _) in MIGRATIONS {
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
