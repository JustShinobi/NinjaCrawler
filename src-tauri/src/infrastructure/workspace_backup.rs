//! Export / import of the workspace metadata (NOT media).
//!
//! A backup is a single ZIP archive:
//!   manifest.json   describes the archive (format, version, app version, flags)
//!   workspace.db    a consistent SQLite snapshot taken with `VACUUM INTO`
//!                   (the live database runs in WAL mode, so a plain file copy
//!                   would be inconsistent)
//!   secrets.enc     present only when the user opted to include account
//!                   secrets; an Argon2id + AES-256-GCM sealed JSON map of
//!                   `secret_ref -> plaintext` (see [`backup_crypto`])
//!
//! Downloaded media, caches, logs and connector binaries are intentionally left
//! out — they are large and re-downloadable.
//!
//! Account secrets live on disk as DPAPI blobs (see [`session_secret_store`]),
//! which are bound to the current Windows user and cannot be moved to another
//! machine. The default export therefore strips them; the opt-in path decrypts
//! them with DPAPI and re-seals them under a user password so the backup stays
//! portable. On import the reverse happens: the password unseals the material
//! and DPAPI re-protects it for the local user.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::infrastructure::storage::StorageLayout;
use crate::infrastructure::{backup_crypto, database, session_secret_store, storage};

const MANIFEST_ENTRY: &str = "manifest.json";
const DATABASE_ENTRY: &str = "workspace.db";
const SECRETS_ENTRY: &str = "secrets.enc";
const BACKUP_FORMAT: &str = "ninjacrawler-backup";
const BACKUP_FORMAT_VERSION: u32 = 1;
const SESSION_SECRET_EXTENSION: &str = "bin";

#[derive(Serialize, Deserialize)]
struct BackupManifest {
    format: String,
    version: u32,
    created_at: String,
    app_version: String,
    includes_secrets: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackupExportResult {
    pub cancelled: bool,
    pub path: Option<String>,
    pub includes_secrets: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackupInspection {
    pub cancelled: bool,
    pub path: Option<String>,
    pub includes_secrets: bool,
    pub app_version: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackupImportResult {
    pub includes_secrets: bool,
    pub secrets_restored: u32,
    pub restart_required: bool,
    pub pre_restore_path: Option<String>,
}

fn default_backup_file_name() -> String {
    let date = chrono::Local::now().format("%Y%m%d-%H%M%S");
    format!("ninjacrawler-backup-{date}.zip")
}

fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// Opens a save dialog and writes the backup. When `include_secrets` is set a
/// non-empty `password` is required.
pub fn export_workspace_backup(
    include_secrets: bool,
    password: Option<String>,
) -> Result<BackupExportResult, String> {
    if include_secrets {
        let has_password = password.as_deref().map(str::is_empty) == Some(false);
        if !has_password {
            return Err("A password is required to include account secrets.".to_string());
        }
    }

    let dialog = rfd::FileDialog::new()
        .set_title("Export workspace backup")
        .set_file_name(default_backup_file_name())
        .add_filter("NinjaCrawler backup", &["zip"]);

    let Some(dest) = dialog.save_file() else {
        return Ok(BackupExportResult {
            cancelled: true,
            path: None,
            includes_secrets: include_secrets,
        });
    };

    let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    write_export(&layout, &dest, include_secrets, password.as_deref())?;

    Ok(BackupExportResult {
        cancelled: false,
        path: Some(dest.to_string_lossy().into_owned()),
        includes_secrets: include_secrets,
    })
}

fn write_export(
    layout: &StorageLayout,
    dest: &Path,
    include_secrets: bool,
    password: Option<&str>,
) -> Result<(), String> {
    let db_bytes = snapshot_database(layout)?;

    let secrets_blob = if include_secrets {
        let password = password.ok_or("A password is required to include account secrets.")?;
        let secrets = collect_session_secrets(layout)?;
        let json = serde_json::to_vec(&secrets).map_err(|error| error.to_string())?;
        let blob = backup_crypto::encrypt(&json, password)?;
        Some(blob)
    } else {
        None
    };

    let archive = build_archive(&db_bytes, secrets_blob.as_deref(), include_secrets)?;
    fs::write(dest, archive).map_err(|error| format!("Failed to write backup file: {error}"))
}

/// Takes a consistent snapshot of the live WAL database via `VACUUM INTO` and
/// returns the raw bytes of the resulting single-file database.
fn snapshot_database(layout: &StorageLayout) -> Result<Vec<u8>, String> {
    let temp = tempfile::Builder::new()
        .prefix("ninjacrawler-backup-")
        .suffix(".db")
        .tempfile()
        .map_err(|error| error.to_string())?;
    let snapshot_path = temp.path().to_path_buf();
    // VACUUM INTO refuses to overwrite an existing file.
    drop(temp);

    let connection =
        database::open_connection(&layout.db_path).map_err(|error| error.to_string())?;
    connection
        .execute("VACUUM INTO ?1", [snapshot_path.to_string_lossy().as_ref()])
        .map_err(|error| format!("Failed to snapshot the database: {error}"))?;
    drop(connection);

    let bytes = fs::read(&snapshot_path).map_err(|error| error.to_string())?;
    let _ = fs::remove_file(&snapshot_path);
    Ok(bytes)
}

/// DPAPI-decrypts every `*.bin` session secret into a `secret_ref -> plaintext`
/// map. The `secret_ref` is the file stem, matching the values stored in the
/// database (`provider_account_sessions.secret_ref`, companion backups, ...).
fn collect_session_secrets(layout: &StorageLayout) -> Result<BTreeMap<String, String>, String> {
    let mut secrets = BTreeMap::new();
    let root = layout.data_dir.join("sessions");
    if !root.exists() {
        return Ok(secrets);
    }

    for entry in fs::read_dir(&root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some(SESSION_SECRET_EXTENSION) {
            continue;
        }
        let Some(secret_ref) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let payload = session_secret_store::load_secret(layout, secret_ref)?;
        secrets.insert(secret_ref.to_string(), payload);
    }

    Ok(secrets)
}

fn build_archive(
    db_bytes: &[u8],
    secrets_blob: Option<&[u8]>,
    includes_secrets: bool,
) -> Result<Vec<u8>, String> {
    let mut buffer = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buffer);
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

        let manifest = BackupManifest {
            format: BACKUP_FORMAT.to_string(),
            version: BACKUP_FORMAT_VERSION,
            created_at: chrono::Utc::now().to_rfc3339(),
            app_version: app_version(),
            includes_secrets,
        };
        let manifest_json =
            serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;

        zip.start_file(MANIFEST_ENTRY, deflated)
            .map_err(|error| error.to_string())?;
        zip.write_all(&manifest_json)
            .map_err(|error| error.to_string())?;

        zip.start_file(DATABASE_ENTRY, deflated)
            .map_err(|error| error.to_string())?;
        zip.write_all(db_bytes).map_err(|error| error.to_string())?;

        if let Some(blob) = secrets_blob {
            // Already AES-GCM ciphertext — no point deflating incompressible data.
            zip.start_file(SECRETS_ENTRY, stored)
                .map_err(|error| error.to_string())?;
            zip.write_all(blob).map_err(|error| error.to_string())?;
        }

        zip.finish().map_err(|error| error.to_string())?;
    }
    Ok(buffer.into_inner())
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Opens a file dialog and reads only the manifest so the UI can decide whether
/// to prompt for a password before applying the restore.
pub fn inspect_workspace_backup() -> Result<BackupInspection, String> {
    let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    let dialog = rfd::FileDialog::new()
        .set_title("Import workspace backup")
        .set_directory(layout.root.clone())
        .add_filter("NinjaCrawler backup", &["zip"]);

    let Some(path) = dialog.pick_file() else {
        return Ok(BackupInspection {
            cancelled: true,
            path: None,
            includes_secrets: false,
            app_version: None,
            created_at: None,
        });
    };

    let bytes = fs::read(&path).map_err(|error| error.to_string())?;
    let manifest = read_manifest(&bytes)?;

    Ok(BackupInspection {
        cancelled: false,
        path: Some(path.to_string_lossy().into_owned()),
        includes_secrets: manifest.includes_secrets,
        app_version: Some(manifest.app_version),
        created_at: Some(manifest.created_at),
    })
}

fn read_manifest(archive_bytes: &[u8]) -> Result<BackupManifest, String> {
    let mut archive =
        ZipArchive::new(Cursor::new(archive_bytes)).map_err(|_| "Not a valid backup file.")?;
    let mut manifest_file = archive
        .by_name(MANIFEST_ENTRY)
        .map_err(|_| "Backup file is missing its manifest.".to_string())?;
    let mut json = String::new();
    manifest_file
        .read_to_string(&mut json)
        .map_err(|error| error.to_string())?;
    let manifest: BackupManifest =
        serde_json::from_str(&json).map_err(|_| "Backup manifest is corrupted.".to_string())?;

    if manifest.format != BACKUP_FORMAT {
        return Err("This file is not a NinjaCrawler backup.".to_string());
    }
    if manifest.version > BACKUP_FORMAT_VERSION {
        return Err(format!(
            "This backup was made by a newer version (format {}). Update the app first.",
            manifest.version
        ));
    }
    Ok(manifest)
}

/// Restores a backup from `path`. The current database is preserved as
/// `ninjacrawler.db.pre-restore-<timestamp>` before being replaced. A restart
/// is required afterwards because the process caches migration/bootstrap state
/// per database path.
pub fn import_workspace_backup(
    path: String,
    password: Option<String>,
) -> Result<BackupImportResult, String> {
    let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    let archive_bytes = fs::read(&path).map_err(|error| error.to_string())?;
    let manifest = read_manifest(&archive_bytes)?;

    // Unseal secrets before touching the live database, so a wrong password
    // aborts the restore without side effects.
    let secrets = if manifest.includes_secrets {
        let password = password
            .as_deref()
            .filter(|value| !value.is_empty())
            .ok_or("This backup contains account secrets. A password is required.")?;
        let blob = read_secrets_blob(&archive_bytes)?;
        let json = backup_crypto::decrypt(&blob, password)?;
        let map: BTreeMap<String, String> =
            serde_json::from_slice(&json).map_err(|_| "Backup secrets are corrupted.".to_string())?;
        Some(map)
    } else {
        None
    };

    let db_bytes = read_database_bytes(&archive_bytes)?;
    validate_snapshot_database(&db_bytes)?;

    let pre_restore = swap_in_database(&layout, &db_bytes)?;

    let mut secrets_restored = 0u32;
    if let Some(secrets) = secrets {
        for (secret_ref, payload) in secrets {
            session_secret_store::store_secret(&layout, &secret_ref, &payload)?;
            secrets_restored += 1;
        }
    }

    Ok(BackupImportResult {
        includes_secrets: manifest.includes_secrets,
        secrets_restored,
        restart_required: true,
        pre_restore_path: Some(pre_restore.to_string_lossy().into_owned()),
    })
}

fn read_entry(archive_bytes: &[u8], name: &str) -> Result<Vec<u8>, String> {
    let mut archive =
        ZipArchive::new(Cursor::new(archive_bytes)).map_err(|_| "Not a valid backup file.")?;
    let mut file = archive
        .by_name(name)
        .map_err(|_| format!("Backup file is missing '{name}'."))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

fn read_database_bytes(archive_bytes: &[u8]) -> Result<Vec<u8>, String> {
    read_entry(archive_bytes, DATABASE_ENTRY)
}

fn read_secrets_blob(archive_bytes: &[u8]) -> Result<Vec<u8>, String> {
    read_entry(archive_bytes, SECRETS_ENTRY)
}

/// Confirms the extracted snapshot is a usable SQLite database (integrity check
/// passes and the migration ledger is present) before it replaces the live one.
fn validate_snapshot_database(db_bytes: &[u8]) -> Result<(), String> {
    let temp = tempfile::Builder::new()
        .prefix("ninjacrawler-restore-")
        .suffix(".db")
        .tempfile()
        .map_err(|error| error.to_string())?;
    fs::write(temp.path(), db_bytes).map_err(|error| error.to_string())?;

    let connection = rusqlite::Connection::open(temp.path())
        .map_err(|_| "Backup database could not be opened.".to_string())?;
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|_| "Backup database failed the integrity check.".to_string())?;
    if integrity != "ok" {
        return Err("Backup database failed the integrity check.".to_string());
    }
    let has_ledger: bool = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if !has_ledger {
        return Err("Backup database is missing the schema ledger.".to_string());
    }
    Ok(())
}

/// Checkpoints and preserves the current database, then installs the snapshot.
/// Returns the path of the preserved copy.
fn swap_in_database(layout: &StorageLayout, db_bytes: &[u8]) -> Result<PathBuf, String> {
    let db_path = &layout.db_path;

    if db_path.exists() {
        // Fold any pending WAL back into the main file so the preserved copy is
        // self-contained, then drop the connection before moving the file.
        if let Ok(connection) = database::open_connection(db_path) {
            let _ = connection.pragma_update(None, "wal_checkpoint", "TRUNCATE");
        }
    }

    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let pre_restore = sibling_with_suffix(db_path, &format!("pre-restore-{timestamp}"));

    if db_path.exists() {
        fs::rename(db_path, &pre_restore)
            .map_err(|error| format!("Failed to preserve the current database: {error}"))?;
    }

    // The old WAL/SHM belong to the pre-restore database; remove them so SQLite
    // does not fold stale WAL frames into the freshly restored file.
    let _ = fs::remove_file(sibling_with_extra(db_path, "-wal"));
    let _ = fs::remove_file(sibling_with_extra(db_path, "-shm"));

    fs::write(db_path, db_bytes).map_err(|error| {
        format!("Failed to install the restored database: {error}")
    })?;

    Ok(pre_restore)
}

fn sibling_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "ninjacrawler.db".to_string());
    name.push('.');
    name.push_str(suffix);
    path.with_file_name(name)
}

fn sibling_with_extra(path: &Path, extra: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.push_str(extra);
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_db(path: &Path) {
        let connection = rusqlite::Connection::open(path).expect("open db");
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY);
                 INSERT INTO schema_migrations(version) VALUES (1);
                 CREATE TABLE demo (id INTEGER PRIMARY KEY, value TEXT);
                 INSERT INTO demo(value) VALUES ('hello');",
            )
            .expect("seed db");
    }

    #[test]
    fn archive_without_secrets_strips_secret_material() {
        let db = b"SQLite format 3\0 not-a-real-db-but-fine-for-membership".to_vec();
        let archive = build_archive(&db, None, false).expect("archive");

        let mut zip = ZipArchive::new(Cursor::new(archive)).expect("zip");
        assert!(zip.by_name(DATABASE_ENTRY).is_ok(), "database must be present");
        assert!(
            zip.by_name(SECRETS_ENTRY).is_err(),
            "no secrets entry when stripped"
        );

        let mut manifest_json = String::new();
        zip.by_name(MANIFEST_ENTRY)
            .expect("manifest")
            .read_to_string(&mut manifest_json)
            .expect("read manifest");
        let manifest: BackupManifest = serde_json::from_str(&manifest_json).expect("manifest");
        assert!(!manifest.includes_secrets);
    }

    #[test]
    fn archive_with_secrets_round_trips_through_password() {
        let db = b"db-bytes".to_vec();
        let mut secrets = BTreeMap::new();
        secrets.insert("account-1".to_string(), "{\"cookie\":\"v\"}".to_string());
        let json = serde_json::to_vec(&secrets).expect("json");
        let blob = backup_crypto::encrypt(&json, "pw").expect("encrypt");

        let archive = build_archive(&db, Some(&blob), true).expect("archive");

        let read_back = read_secrets_blob(&archive).expect("secrets entry present");
        let decrypted = backup_crypto::decrypt(&read_back, "pw").expect("decrypt");
        let restored: BTreeMap<String, String> =
            serde_json::from_slice(&decrypted).expect("map");
        assert_eq!(restored, secrets);

        // A wrong password must not surface the material.
        assert!(backup_crypto::decrypt(&read_back, "nope").is_err());
    }

    #[test]
    fn manifest_rejects_foreign_archives() {
        let db = b"db".to_vec();
        let archive = build_archive(&db, None, false).expect("archive");
        // Corrupt the format marker by rebuilding a bogus manifest archive.
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut buffer);
            let opts = SimpleFileOptions::default();
            zip.start_file(MANIFEST_ENTRY, opts).unwrap();
            zip.write_all(br#"{"format":"something-else","version":1,"createdAt":"","appVersion":"","includesSecrets":false}"#)
                .unwrap();
            zip.finish().unwrap();
        }
        assert!(read_manifest(&buffer.into_inner()).is_err());
        // Sanity: the genuine archive parses.
        assert!(read_manifest(&archive).is_ok());
    }

    #[test]
    fn validate_snapshot_accepts_real_db_and_rejects_garbage() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db_path = temp.path().join("valid.db");
        make_test_db(&db_path);
        let bytes = fs::read(&db_path).expect("read db");
        assert!(validate_snapshot_database(&bytes).is_ok());

        assert!(validate_snapshot_database(b"definitely not sqlite").is_err());
    }

    #[test]
    fn swap_in_database_preserves_previous_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let layout = storage::workspace_layout_from_roots(
            temp.path().join("localappdata"),
            temp.path().join("userprofile"),
        )
        .expect("layout");

        // Seed a "current" database.
        make_test_db(&layout.db_path);

        let new_db_dir = temp.path().join("snap");
        fs::create_dir_all(&new_db_dir).unwrap();
        let snapshot_path = new_db_dir.join("snap.db");
        make_test_db(&snapshot_path);
        let snapshot_bytes = fs::read(&snapshot_path).unwrap();

        let pre_restore = swap_in_database(&layout, &snapshot_bytes).expect("swap");
        assert!(pre_restore.exists(), "previous db must be preserved");
        assert!(layout.db_path.exists(), "restored db must exist");
    }
}
