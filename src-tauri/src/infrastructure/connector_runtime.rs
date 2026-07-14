use chrono::{Duration, Utc};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::{Cursor, Read, Write};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter};
use zip::ZipArchive;

use crate::domain::models::ConnectorRuntimeStatus;
use crate::infrastructure::storage::StorageLayout;
use crate::infrastructure::{database, runtime_log, storage};

pub const CONNECTOR_RUNTIME_CHANGED_EVENT: &str = "runtime://connector-runtime-changed";
const UPDATE_CHECK_INTERVAL_HOURS: i64 = 12;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectorManifest {
    connectors: Vec<ConnectorCatalogEntry>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectorCatalogEntry {
    key: String,
    display_name: String,
    tool_setting_key: String,
    default_command: String,
    bundled_version: String,
    #[serde(default)]
    release_tag: Option<String>,
    executable_name: String,
    version_args: Vec<String>,
    release_api_url: String,
    asset_name: String,
    #[serde(default)]
    asset_prefix: Option<String>,
    #[serde(default)]
    asset_suffix: Option<String>,
    archive_member_name: Option<String>,
}

impl ConnectorCatalogEntry {
    /// Checks whether a release asset name matches the catalog entry.
    /// Prefix and suffix matching supports versioned archive names.
    fn asset_matches(&self, name: &str) -> bool {
        if self.asset_prefix.is_some() || self.asset_suffix.is_some() {
            let lower = name.to_ascii_lowercase();
            let prefix_ok = self
                .asset_prefix
                .as_ref()
                .map(|prefix| lower.starts_with(&prefix.to_ascii_lowercase()))
                .unwrap_or(true);
            let suffix_ok = self
                .asset_suffix
                .as_ref()
                .map(|suffix| lower.ends_with(&suffix.to_ascii_lowercase()))
                .unwrap_or(true);
            prefix_ok && suffix_ok
        } else {
            name.eq_ignore_ascii_case(&self.asset_name)
        }
    }

    /// Returns a readable asset pattern for error messages.
    fn asset_descriptor(&self) -> String {
        match (&self.asset_prefix, &self.asset_suffix) {
            (None, None) => self.asset_name.clone(),
            (prefix, suffix) => format!(
                "{}*{}",
                prefix.as_deref().unwrap_or(""),
                suffix.as_deref().unwrap_or("")
            ),
        }
    }
}

#[derive(Clone)]
struct ConnectorRuntimeRecord {
    key: String,
    display_name: String,
    management_mode: String,
    bundled_version: String,
    active_version: Option<String>,
    active_path: Option<String>,
    custom_path: Option<String>,
    latest_version: Option<String>,
    latest_asset_url: Option<String>,
    latest_asset_digest: Option<String>,
    latest_checked_at: Option<String>,
    update_status: String,
    pending_version: Option<String>,
    pending_path: Option<String>,
    progress_percent: Option<u32>,
    progress_detail: Option<String>,
    last_error: Option<String>,
    updated_at: String,
}

#[derive(Clone)]
struct LatestRelease {
    version: String,
    asset_url: String,
    asset_digest: String,
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<GitHubReleaseAsset>,
}

#[derive(Deserialize)]
struct GitHubReleaseAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

#[derive(Default)]
struct ConnectorUsageState {
    counts: HashMap<String, usize>,
}

pub struct ConnectorUsageGuard {
    key: String,
}

impl Drop for ConnectorUsageGuard {
    fn drop(&mut self) {
        let should_activate = {
            let Ok(mut state) = usage_state().lock() else {
                return;
            };
            let Some(count) = state.counts.get_mut(&self.key) else {
                return;
            };
            if *count > 1 {
                *count -= 1;
                false
            } else {
                state.counts.remove(&self.key);
                true
            }
        };

        if should_activate {
            let _ = activate_pending_if_idle(&self.key);
        }
    }
}

fn catalog() -> &'static [ConnectorCatalogEntry] {
    static CATALOG: OnceLock<Vec<ConnectorCatalogEntry>> = OnceLock::new();
    CATALOG
        .get_or_init(|| {
            let manifest: ConnectorManifest =
                serde_json::from_str(include_str!("../../../connectors/manifest.json"))
                    .expect("connector manifest should be valid JSON");
            manifest.connectors
        })
        .as_slice()
}

fn catalog_entry(key: &str) -> Result<&'static ConnectorCatalogEntry, String> {
    catalog()
        .iter()
        .find(|entry| entry.key.eq_ignore_ascii_case(key))
        .ok_or_else(|| format!("Connector runtime '{}' is not registered.", key))
}

fn app_handle_registry() -> &'static Mutex<Option<AppHandle>> {
    static HANDLE: OnceLock<Mutex<Option<AppHandle>>> = OnceLock::new();
    HANDLE.get_or_init(|| Mutex::new(None))
}

fn usage_state() -> &'static Mutex<ConnectorUsageState> {
    static STATE: OnceLock<Mutex<ConnectorUsageState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(ConnectorUsageState::default()))
}

pub fn register_app_handle(app: &AppHandle) {
    if let Ok(mut handle) = app_handle_registry().lock() {
        *handle = Some(app.clone());
    }
}

fn log_connector_runtime_event(
    entry: &ConnectorCatalogEntry,
    level: &str,
    message: impl Into<String>,
    detail: Option<String>,
) {
    let _ = runtime_log::append_workspace(
        "connector.runtime",
        level,
        runtime_log::RuntimeLogAnchor::default(),
        message,
        detail.or_else(|| Some(format!("Connector: {} ({})", entry.display_name, entry.key))),
    );
}

pub fn claim_connector_usage(key: &str) -> ConnectorUsageGuard {
    if let Ok(mut state) = usage_state().lock() {
        let count = state.counts.entry(key.to_string()).or_default();
        *count += 1;
    }

    ConnectorUsageGuard {
        key: key.to_string(),
    }
}

pub fn ensure_catalog_state(connection: &Connection, layout: &StorageLayout) -> Result<(), String> {
    let settings = load_app_settings_map(connection)?;

    for entry in catalog() {
        let record = load_record(connection, &entry.key)?;
        let normalized = normalize_record(layout, entry, record, &settings)?;
        save_record(connection, &normalized)?;
    }

    Ok(())
}

pub fn load_connector_runtime_statuses(
    connection: &Connection,
) -> Result<Vec<ConnectorRuntimeStatus>, String> {
    let mut statuses = Vec::new();

    for entry in catalog() {
        if let Some(record) = load_record(connection, &entry.key)? {
            statuses.push(to_status(&record));
        }
    }

    Ok(statuses)
}

pub fn prepare_connector_runtimes() -> Result<Vec<ConnectorRuntimeStatus>, String> {
    let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    let connection =
        database::open_connection(&layout.db_path).map_err(|error| error.to_string())?;
    ensure_catalog_state(&connection, &layout)?;

    let mut failures = Vec::new();
    for entry in catalog() {
        let record = load_record(&connection, &entry.key)?
            .ok_or_else(|| format!("Connector runtime '{}' is not initialized.", entry.key))?;
        let active_exists = record
            .active_path
            .as_deref()
            .map(Path::new)
            .is_some_and(Path::exists);
        if active_exists {
            continue;
        }
        if record.management_mode == "custom" {
            failures.push(format!(
                "{}: configured custom executable does not exist",
                entry.display_name
            ));
            continue;
        }

        if let Err(error) = prepare_managed_connector(&connection, &layout, entry) {
            let _ = update_progress(
                &connection,
                &entry.key,
                "error",
                None,
                Some("Connector preparation failed.".to_string()),
                Some(error.clone()),
            );
            log_connector_runtime_event(
                entry,
                "error",
                format!("Failed to prepare '{}'.", entry.display_name),
                Some(error.clone()),
            );
            emit_runtime_changed();
            failures.push(format!("{}: {}", entry.display_name, error));
        }
    }

    if failures.is_empty() {
        load_connector_runtime_statuses(&connection)
    } else {
        Err(format!(
            "Connector preparation did not finish: {}",
            failures.join("; ")
        ))
    }
}

fn prepare_managed_connector(
    connection: &Connection,
    layout: &StorageLayout,
    entry: &ConnectorCatalogEntry,
) -> Result<(), String> {
    update_progress(
        connection,
        &entry.key,
        "checking",
        Some(5),
        Some("Resolving the pinned connector release.".to_string()),
        None,
    )?;
    emit_runtime_changed();

    let release = lookup_pinned_release(entry)?;
    update_progress(
        connection,
        &entry.key,
        "downloading",
        Some(20),
        Some(format!(
            "Downloading {} {}.",
            entry.display_name, release.version
        )),
        None,
    )?;
    emit_runtime_changed();

    let download = download_release_asset(entry, &release.asset_url, &layout.connectors_root)?;
    let bytes = fs::read(download.path()).map_err(|error| {
        format!(
            "Failed to read the temporary '{}' download: {}",
            entry.display_name, error
        )
    })?;
    verify_asset_digest(entry, &bytes, &release.asset_digest)?;
    update_progress(
        connection,
        &entry.key,
        "downloading",
        Some(75),
        Some("Digest verified. Installing connector runtime.".to_string()),
        None,
    )?;
    emit_runtime_changed();

    let installed_path = install_release_asset(layout, entry, &release.version, &bytes)?;
    let mut record = load_record(connection, &entry.key)?
        .ok_or_else(|| format!("Connector runtime '{}' is not initialized.", entry.key))?;
    record.active_version = Some(release.version.clone());
    record.active_path = Some(installed_path.display().to_string());
    record.latest_version = Some(release.version);
    record.latest_asset_url = Some(release.asset_url);
    record.latest_asset_digest = Some(release.asset_digest);
    record.latest_checked_at = Some(now_timestamp());
    record.update_status = "up_to_date".to_string();
    record.progress_percent = Some(100);
    record.progress_detail = Some("Managed runtime installed and verified.".to_string());
    record.last_error = None;
    record.updated_at = now_timestamp();
    save_record(connection, &record)?;
    emit_runtime_changed();
    Ok(())
}

pub fn resolve_connector_executable(
    connection: &Connection,
    layout: &StorageLayout,
    key: &str,
) -> Result<String, String> {
    ensure_catalog_state(connection, layout)?;
    activate_pending_if_idle(key)?;
    let record = load_record(connection, key)?
        .ok_or_else(|| format!("Connector runtime '{}' is not initialized.", key))?;
    record.active_path.ok_or_else(|| {
        format!(
            "Connector '{}' is not installed. Complete connector preparation before continuing.",
            key
        )
    })
}

pub fn check_connector_updates(key: Option<&str>) -> Result<Vec<ConnectorRuntimeStatus>, String> {
    let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    let connection =
        database::open_connection(&layout.db_path).map_err(|error| error.to_string())?;
    ensure_catalog_state(&connection, &layout)?;

    let target_keys = match key {
        Some(value) => vec![catalog_entry(value)?.key.clone()],
        None => catalog().iter().map(|entry| entry.key.clone()).collect(),
    };

    for target_key in target_keys {
        let entry = catalog_entry(&target_key)?;
        update_progress(
            &connection,
            &entry.key,
            "checking",
            None,
            Some("Checking upstream release metadata.".to_string()),
            None,
        )?;

        match lookup_latest_release(entry) {
            Ok(release) => {
                let now = now_timestamp();
                let mut record = load_record(&connection, &entry.key)?.ok_or_else(|| {
                    format!("Connector runtime '{}' is not initialized.", entry.key)
                })?;
                let update_available = record.active_version.as_deref() != Some(&release.version);
                record.latest_version = Some(release.version);
                record.latest_asset_url = Some(release.asset_url);
                record.latest_asset_digest = Some(release.asset_digest);
                record.latest_checked_at = Some(now.clone());
                record.last_error = None;
                record.progress_percent = None;
                record.progress_detail = None;
                record.update_status = derive_status(
                    &record.management_mode,
                    record.active_version.as_deref(),
                    record.latest_version.as_deref(),
                    record.pending_version.as_deref(),
                );
                record.updated_at = now;
                save_record(&connection, &record)?;
                if update_available {
                    log_connector_runtime_event(
                        entry,
                        "info",
                        format!(
                            "Update available for '{}' ({}, active {}).",
                            entry.display_name,
                            record.latest_version.as_deref().unwrap_or("unknown"),
                            record.active_version.as_deref().unwrap_or("not installed")
                        ),
                        None,
                    );
                } else {
                    log_connector_runtime_event(
                        entry,
                        "info",
                        format!(
                            "'{}' is up to date at {}.",
                            entry.display_name,
                            record.active_version.as_deref().unwrap_or("not installed")
                        ),
                        None,
                    );
                }
            }
            Err(error) => {
                let mut record = load_record(&connection, &entry.key)?.ok_or_else(|| {
                    format!("Connector runtime '{}' is not initialized.", entry.key)
                })?;
                record.update_status = if record.management_mode == "custom" {
                    "custom_override".to_string()
                } else {
                    "error".to_string()
                };
                record.last_error = Some(error);
                record.progress_percent = None;
                record.progress_detail = None;
                record.latest_checked_at = Some(now_timestamp());
                record.updated_at = now_timestamp();
                let detail = record.last_error.clone();
                save_record(&connection, &record)?;
                log_connector_runtime_event(
                    entry,
                    "error",
                    format!("Failed to check updates for '{}'.", entry.display_name),
                    detail,
                );
            }
        }
    }

    emit_runtime_changed();
    load_connector_runtime_statuses(&connection)
}

pub fn update_connector_runtime(key: &str) -> Result<Vec<ConnectorRuntimeStatus>, String> {
    let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    let connection =
        database::open_connection(&layout.db_path).map_err(|error| error.to_string())?;
    ensure_catalog_state(&connection, &layout)?;

    let entry = catalog_entry(key)?;
    let mut record = load_record(&connection, &entry.key)?
        .ok_or_else(|| format!("Connector runtime '{}' is not initialized.", entry.key))?;

    if record.management_mode == "custom" {
        log_connector_runtime_event(
            entry,
            "warning",
            format!(
                "Skipped managed update for '{}' because it is using a custom executable.",
                entry.display_name
            ),
            None,
        );
        return Err(format!(
            "Connector '{}' is using a custom executable. Revert it to managed mode before updating.",
            entry.display_name
        ));
    }

    let needs_refresh = record
        .latest_checked_at
        .as_deref()
        .and_then(parse_timestamp)
        .map(|checked| {
            Utc::now().signed_duration_since(checked)
                >= Duration::hours(UPDATE_CHECK_INTERVAL_HOURS)
        })
        .unwrap_or(true)
        || record.latest_version.is_none()
        || record.latest_asset_url.is_none()
        || record.latest_asset_digest.is_none();

    if needs_refresh {
        let _ = check_connector_updates(Some(&entry.key))?;
        record = load_record(&connection, &entry.key)?
            .ok_or_else(|| format!("Connector runtime '{}' is not initialized.", entry.key))?;
    }

    let latest_version = record.latest_version.clone().ok_or_else(|| {
        format!(
            "Unable to determine the latest version for '{}'.",
            entry.display_name
        )
    })?;
    let asset_url = record.latest_asset_url.clone().ok_or_else(|| {
        format!(
            "Unable to determine the latest download asset for '{}'.",
            entry.display_name
        )
    })?;
    let asset_digest = record.latest_asset_digest.clone().ok_or_else(|| {
        format!(
            "Unable to determine the verified download digest for '{}'.",
            entry.display_name
        )
    })?;

    if record.pending_version.as_deref() == Some(latest_version.as_str()) {
        return load_connector_runtime_statuses(&connection);
    }
    if record.active_version.as_deref() == Some(latest_version.as_str()) {
        record.update_status = "up_to_date".to_string();
        record.progress_percent = None;
        record.progress_detail = None;
        record.last_error = None;
        record.updated_at = now_timestamp();
        save_record(&connection, &record)?;
        emit_runtime_changed();
        log_connector_runtime_event(
            entry,
            "info",
            format!(
                "'{}' is already running the latest version ({}).",
                entry.display_name, latest_version
            ),
            None,
        );
        return load_connector_runtime_statuses(&connection);
    }

    log_connector_runtime_event(
        entry,
        "info",
        format!(
            "Starting managed runtime update for '{}'.",
            entry.display_name
        ),
        Some(format!(
            "Active version: {}. Target version: {}.",
            record.active_version.as_deref().unwrap_or("not installed"),
            latest_version
        )),
    );

    update_progress(
        &connection,
        &entry.key,
        "downloading",
        Some(5),
        Some(format!(
            "Downloading {} {}.",
            entry.display_name, latest_version
        )),
        None,
    )?;
    emit_runtime_changed();

    let download = match download_release_asset(entry, &asset_url, &layout.connectors_root) {
        Ok(download) => download,
        Err(error) => {
            log_connector_runtime_event(
                entry,
                "error",
                format!(
                    "Failed to download the update for '{}'.",
                    entry.display_name
                ),
                Some(error.clone()),
            );
            return Err(error);
        }
    };
    let bytes = fs::read(download.path()).map_err(|error| {
        format!(
            "Failed to read the temporary '{}' download: {}",
            entry.display_name, error
        )
    })?;
    if let Err(error) = verify_asset_digest(entry, &bytes, &asset_digest) {
        update_progress(
            &connection,
            &entry.key,
            "error",
            None,
            Some("Downloaded asset failed integrity verification.".to_string()),
            Some(error.clone()),
        )?;
        emit_runtime_changed();
        return Err(error);
    }

    update_progress(
        &connection,
        &entry.key,
        "downloading",
        Some(65),
        Some("Installing downloaded connector runtime.".to_string()),
        None,
    )?;
    emit_runtime_changed();

    let installed_path = match install_release_asset(&layout, entry, &latest_version, &bytes) {
        Ok(path) => path,
        Err(error) => {
            update_progress(
                &connection,
                &entry.key,
                "error",
                None,
                Some("Downloaded asset could not be installed.".to_string()),
                Some(error.clone()),
            )?;
            emit_runtime_changed();
            log_connector_runtime_event(
                entry,
                "error",
                format!("Failed to install the update for '{}'.", entry.display_name),
                Some(error.clone()),
            );
            return Err(error);
        }
    };
    let installed_path_string = installed_path.display().to_string();
    let connector_busy = match usage_state().lock() {
        Ok(state) => state.counts.get(&entry.key).copied().unwrap_or(0) > 0,
        Err(_) => false,
    };

    let mut record = load_record(&connection, &entry.key)?
        .ok_or_else(|| format!("Connector runtime '{}' is not initialized.", entry.key))?;
    record.last_error = None;
    record.progress_percent = Some(100);

    if connector_busy {
        record.pending_version = Some(latest_version.clone());
        record.pending_path = Some(installed_path_string);
        record.progress_detail =
            Some("Downloaded. Activation will happen after the current job finishes.".to_string());
        record.update_status = "pending_activation".to_string();
    } else {
        record.active_version = Some(latest_version.clone());
        record.active_path = Some(installed_path_string);
        record.pending_version = None;
        record.pending_path = None;
        record.progress_detail = Some("Connector runtime updated successfully.".to_string());
        record.update_status = "up_to_date".to_string();
    }

    record.updated_at = now_timestamp();
    save_record(&connection, &record)?;
    if connector_busy {
        log_connector_runtime_event(
            entry,
            "warning",
            format!(
                "Downloaded '{}' {}. Activation will happen when the current job finishes.",
                entry.display_name, latest_version
            ),
            None,
        );
    } else {
        log_connector_runtime_event(
            entry,
            "info",
            format!(
                "Updated '{}' to {} and activated it immediately.",
                entry.display_name, latest_version
            ),
            None,
        );
    }
    emit_runtime_changed();

    load_connector_runtime_statuses(&connection)
}

pub fn set_connector_custom_override(
    key: &str,
    custom_path: &str,
) -> Result<Vec<ConnectorRuntimeStatus>, String> {
    let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    let connection =
        database::open_connection(&layout.db_path).map_err(|error| error.to_string())?;
    ensure_catalog_state(&connection, &layout)?;

    let entry = catalog_entry(key)?;
    let path = PathBuf::from(custom_path.trim());
    if custom_path.trim().is_empty() {
        return Err("Custom connector path must not be empty.".to_string());
    }
    if !path.exists() {
        return Err(format!(
            "Custom connector executable does not exist: '{}'.",
            custom_path
        ));
    }

    let version = probe_connector_version(entry, &path)?;
    let mut record = load_record(&connection, &entry.key)?
        .ok_or_else(|| format!("Connector runtime '{}' is not initialized.", entry.key))?;
    record.management_mode = "custom".to_string();
    record.custom_path = Some(path.display().to_string());
    record.active_path = Some(path.display().to_string());
    record.active_version = Some(version);
    record.pending_version = None;
    record.pending_path = None;
    record.update_status = "custom_override".to_string();
    record.progress_percent = None;
    record.progress_detail = Some("Using a user-supplied executable.".to_string());
    record.last_error = None;
    record.updated_at = now_timestamp();
    save_record(&connection, &record)?;
    emit_runtime_changed();

    load_connector_runtime_statuses(&connection)
}

pub fn clear_connector_custom_override(key: &str) -> Result<Vec<ConnectorRuntimeStatus>, String> {
    let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    let connection =
        database::open_connection(&layout.db_path).map_err(|error| error.to_string())?;
    ensure_catalog_state(&connection, &layout)?;

    let entry = catalog_entry(key)?;
    let install_path = managed_install_path(&layout, entry, &entry.bundled_version);
    let managed_ready = install_path.exists()
        && probe_connector_version(entry, &install_path).as_deref()
            == Ok(entry.bundled_version.as_str());
    let (active_version, active_path, status, detail) = if managed_ready {
        (
            Some(entry.bundled_version.clone()),
            Some(install_path.display().to_string()),
            "up_to_date",
            "Managed runtime restored.",
        )
    } else {
        (
            None,
            None,
            "not_installed",
            "Managed runtime must be downloaded before the workspace can open.",
        )
    };
    let mut record = load_record(&connection, &entry.key)?
        .ok_or_else(|| format!("Connector runtime '{}' is not initialized.", entry.key))?;
    record.management_mode = "managed".to_string();
    record.custom_path = None;
    record.active_version = active_version;
    record.active_path = active_path;
    record.pending_version = None;
    record.pending_path = None;
    record.progress_percent = None;
    record.progress_detail = Some(detail.to_string());
    record.last_error = None;
    record.update_status = status.to_string();
    record.updated_at = now_timestamp();
    save_record(&connection, &record)?;
    emit_runtime_changed();

    load_connector_runtime_statuses(&connection)
}

fn activate_pending_if_idle(key: &str) -> Result<(), String> {
    let busy = {
        let state = usage_state()
            .lock()
            .map_err(|_| "Connector usage state lock is poisoned.".to_string())?;
        state.counts.get(key).copied().unwrap_or(0) > 0
    };
    if busy {
        return Ok(());
    }

    let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    let connection =
        database::open_connection(&layout.db_path).map_err(|error| error.to_string())?;
    ensure_catalog_state(&connection, &layout)?;
    let Some(mut record) = load_record(&connection, key)? else {
        return Ok(());
    };
    let Some(pending_version) = record.pending_version.clone() else {
        return Ok(());
    };
    let Some(pending_path) = record.pending_path.clone() else {
        return Ok(());
    };

    record.active_version = Some(pending_version);
    record.active_path = Some(pending_path);
    record.pending_version = None;
    record.pending_path = None;
    record.progress_percent = None;
    record.progress_detail = Some("Pending connector runtime activated.".to_string());
    record.update_status = derive_status(
        &record.management_mode,
        record.active_version.as_deref(),
        record.latest_version.as_deref(),
        record.pending_version.as_deref(),
    );
    record.updated_at = now_timestamp();
    save_record(&connection, &record)?;
    if let Ok(entry) = catalog_entry(key) {
        log_connector_runtime_event(
            entry,
            "info",
            format!(
                "Activated pending runtime for '{}' at version {}.",
                entry.display_name,
                record.active_version.as_deref().unwrap_or("unknown")
            ),
            None,
        );
    }
    emit_runtime_changed();
    Ok(())
}

fn normalize_record(
    layout: &StorageLayout,
    entry: &ConnectorCatalogEntry,
    existing: Option<ConnectorRuntimeRecord>,
    settings: &HashMap<String, String>,
) -> Result<ConnectorRuntimeRecord, String> {
    if let Some(mut record) = existing {
        if record.management_mode == "custom" {
            if let Some(custom_path) = record.custom_path.clone() {
                let custom = PathBuf::from(&custom_path);
                if custom.exists() {
                    match probe_connector_version(entry, &custom) {
                        Ok(version) => {
                            record.active_path = Some(custom_path);
                            record.active_version = Some(version);
                            record.last_error = None;
                        }
                        Err(error) => {
                            record.active_path = None;
                            record.active_version = None;
                            record.last_error = Some(error);
                            record.update_status = "error".to_string();
                        }
                    }
                } else {
                    record.active_path = None;
                    record.active_version = None;
                    record.last_error =
                        Some("Configured custom executable no longer exists.".to_string());
                    record.update_status = "error".to_string();
                }
            }
        } else {
            let active_exists = record
                .active_path
                .as_deref()
                .map(Path::new)
                .is_some_and(Path::exists);
            if !active_exists {
                let bundled = managed_install_path(layout, entry, &entry.bundled_version);
                if bundled.exists()
                    && probe_connector_version(entry, &bundled).as_deref()
                        == Ok(entry.bundled_version.as_str())
                {
                    record.active_path = Some(bundled.display().to_string());
                    record.active_version = Some(entry.bundled_version.clone());
                    record.last_error = None;
                } else {
                    record.active_path = None;
                    record.active_version = None;
                    record.last_error = None;
                }
            }
        }

        if let Some(pending_path) = record.pending_path.clone() {
            if !Path::new(&pending_path).exists() {
                record.pending_path = None;
                record.pending_version = None;
            }
        }

        record.update_status = if record.management_mode == "custom" && record.active_path.is_none()
        {
            "error".to_string()
        } else {
            derive_status(
                &record.management_mode,
                record.active_version.as_deref(),
                record.latest_version.as_deref(),
                record.pending_version.as_deref(),
            )
        };
        if record.updated_at.trim().is_empty() {
            record.updated_at = now_timestamp();
        }
        return Ok(record);
    }

    let legacy_value = settings
        .get(&entry.tool_setting_key)
        .cloned()
        .unwrap_or_else(|| entry.default_command.clone());
    let is_custom = is_custom_connector_path(entry, &legacy_value);

    if is_custom {
        let custom = PathBuf::from(legacy_value.trim());
        let probe = if custom.exists() {
            probe_connector_version(entry, &custom)
        } else {
            Err("Configured custom executable does not exist.".to_string())
        };
        let active_version = probe.as_ref().ok().cloned();
        let active_path = probe.as_ref().ok().map(|_| custom.display().to_string());
        let last_error = probe.err();
        let custom_ready = active_version.is_some();
        Ok(ConnectorRuntimeRecord {
            key: entry.key.clone(),
            display_name: entry.display_name.clone(),
            management_mode: "custom".to_string(),
            bundled_version: entry.bundled_version.clone(),
            active_version,
            active_path,
            custom_path: Some(custom.display().to_string()),
            latest_version: None,
            latest_asset_url: None,
            latest_asset_digest: None,
            latest_checked_at: None,
            update_status: if custom_ready {
                "custom_override"
            } else {
                "error"
            }
            .to_string(),
            pending_version: None,
            pending_path: None,
            progress_percent: None,
            progress_detail: Some("Using a user-supplied executable.".to_string()),
            last_error,
            updated_at: now_timestamp(),
        })
    } else {
        let path = managed_install_path(layout, entry, &entry.bundled_version);
        let installed = path.exists()
            && probe_connector_version(entry, &path).as_deref()
                == Ok(entry.bundled_version.as_str());
        Ok(ConnectorRuntimeRecord {
            key: entry.key.clone(),
            display_name: entry.display_name.clone(),
            management_mode: "managed".to_string(),
            bundled_version: entry.bundled_version.clone(),
            active_version: installed.then(|| entry.bundled_version.clone()),
            active_path: installed.then(|| path.display().to_string()),
            custom_path: None,
            latest_version: None,
            latest_asset_url: None,
            latest_asset_digest: None,
            latest_checked_at: None,
            update_status: if installed {
                "up_to_date"
            } else {
                "not_installed"
            }
            .to_string(),
            pending_version: None,
            pending_path: None,
            progress_percent: None,
            progress_detail: Some(
                if installed {
                    "Managed runtime is ready."
                } else {
                    "Managed runtime must be downloaded before the workspace can open."
                }
                .to_string(),
            ),
            last_error: None,
            updated_at: now_timestamp(),
        })
    }
}

fn to_status(record: &ConnectorRuntimeRecord) -> ConnectorRuntimeStatus {
    ConnectorRuntimeStatus {
        key: record.key.clone(),
        display_name: record.display_name.clone(),
        management_mode: record.management_mode.clone(),
        active_version: record.active_version.clone(),
        bundled_version: record.bundled_version.clone(),
        latest_version: record.latest_version.clone(),
        update_available: record.management_mode == "managed"
            && record.pending_version.is_none()
            && record
                .latest_version
                .as_deref()
                .is_some_and(|latest| record.active_version.as_deref() != Some(latest)),
        status: record.update_status.clone(),
        last_checked_at: record.latest_checked_at.clone(),
        last_error: record.last_error.clone(),
        pending_version: record.pending_version.clone(),
        progress_percent: record.progress_percent,
        progress_detail: record.progress_detail.clone(),
        active_path: record.active_path.clone(),
        custom_path: record.custom_path.clone(),
    }
}

fn load_record(
    connection: &Connection,
    key: &str,
) -> Result<Option<ConnectorRuntimeRecord>, String> {
    connection
        .query_row(
            "SELECT
                key,
                display_name,
                management_mode,
                bundled_version,
                active_version,
                active_path,
                custom_path,
                latest_version,
                latest_asset_url,
                latest_asset_digest,
                latest_checked_at,
                update_status,
                pending_version,
                pending_path,
                progress_percent,
                progress_detail,
                last_error,
                updated_at
             FROM connector_runtimes
             WHERE key = ?1
             LIMIT 1",
            params![key],
            |row| {
                Ok(ConnectorRuntimeRecord {
                    key: row.get(0)?,
                    display_name: row.get(1)?,
                    management_mode: row.get(2)?,
                    bundled_version: row.get(3)?,
                    active_version: row.get(4)?,
                    active_path: row.get(5)?,
                    custom_path: row.get(6)?,
                    latest_version: row.get(7)?,
                    latest_asset_url: row.get(8)?,
                    latest_asset_digest: row.get(9)?,
                    latest_checked_at: row.get(10)?,
                    update_status: row.get(11)?,
                    pending_version: row.get(12)?,
                    pending_path: row.get(13)?,
                    progress_percent: row.get(14)?,
                    progress_detail: row.get(15)?,
                    last_error: row.get(16)?,
                    updated_at: row.get(17)?,
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn save_record(connection: &Connection, record: &ConnectorRuntimeRecord) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO connector_runtimes (
                key,
                display_name,
                management_mode,
                bundled_version,
                active_version,
                active_path,
                custom_path,
                latest_version,
                latest_asset_url,
                latest_asset_digest,
                latest_checked_at,
                update_status,
                pending_version,
                pending_path,
                progress_percent,
                progress_detail,
                last_error,
                updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18
             )
             ON CONFLICT(key) DO UPDATE SET
                display_name = excluded.display_name,
                management_mode = excluded.management_mode,
                bundled_version = excluded.bundled_version,
                active_version = excluded.active_version,
                active_path = excluded.active_path,
                custom_path = excluded.custom_path,
                latest_version = excluded.latest_version,
                latest_asset_url = excluded.latest_asset_url,
                latest_asset_digest = excluded.latest_asset_digest,
                latest_checked_at = excluded.latest_checked_at,
                update_status = excluded.update_status,
                pending_version = excluded.pending_version,
                pending_path = excluded.pending_path,
                progress_percent = excluded.progress_percent,
                progress_detail = excluded.progress_detail,
                last_error = excluded.last_error,
                updated_at = excluded.updated_at",
            params![
                record.key,
                record.display_name,
                record.management_mode,
                record.bundled_version,
                record.active_version,
                record.active_path,
                record.custom_path,
                record.latest_version,
                record.latest_asset_url,
                record.latest_asset_digest,
                record.latest_checked_at,
                record.update_status,
                record.pending_version,
                record.pending_path,
                record.progress_percent,
                record.progress_detail,
                record.last_error,
                record.updated_at,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn update_progress(
    connection: &Connection,
    key: &str,
    status: &str,
    progress_percent: Option<u32>,
    progress_detail: Option<String>,
    last_error: Option<String>,
) -> Result<(), String> {
    let mut record = load_record(connection, key)?
        .ok_or_else(|| format!("Connector runtime '{}' is not initialized.", key))?;
    record.update_status = status.to_string();
    record.progress_percent = progress_percent;
    record.progress_detail = progress_detail;
    record.last_error = last_error;
    record.updated_at = now_timestamp();
    save_record(connection, &record)
}

fn load_app_settings_map(connection: &Connection) -> Result<HashMap<String, String>, String> {
    let mut statement = connection
        .prepare("SELECT key, value FROM app_settings")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?;

    let mut settings = HashMap::new();
    for row in rows {
        let (key, value) = row.map_err(|error| error.to_string())?;
        settings.insert(key, value);
    }

    Ok(settings)
}

fn managed_install_path(
    layout: &StorageLayout,
    entry: &ConnectorCatalogEntry,
    version: &str,
) -> PathBuf {
    layout
        .connectors_root
        .join(&entry.key)
        .join(version)
        .join(&entry.executable_name)
}

fn lookup_pinned_release(entry: &ConnectorCatalogEntry) -> Result<LatestRelease, String> {
    let tag = entry
        .release_tag
        .as_deref()
        .unwrap_or(&entry.bundled_version);
    let url = format!(
        "{}/tags/{}",
        entry.release_api_url.trim_end_matches('/'),
        tag
    );
    let release: GitHubRelease = github_client()?
        .get(&url)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| {
            format!(
                "Failed to resolve pinned release '{}' for '{}': {}",
                tag, entry.display_name, error
            )
        })?
        .json()
        .map_err(|error| {
            format!(
                "Failed to parse pinned release '{}' for '{}': {}",
                tag, entry.display_name, error
            )
        })?;
    release_from_asset(entry, &release, Some(&entry.bundled_version))
}

fn lookup_latest_release(entry: &ConnectorCatalogEntry) -> Result<LatestRelease, String> {
    let client = github_client()?;
    let releases: Vec<GitHubRelease> = client
        .get(&entry.release_api_url)
        .query(&[("per_page", "20")])
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| {
            format!(
                "Failed to query releases for '{}': {}",
                entry.display_name, error
            )
        })?
        .json()
        .map_err(|error| {
            format!(
                "Failed to parse releases response for '{}': {}",
                entry.display_name, error
            )
        })?;

    // GitHub returns newest releases first. Some upstream releases publish
    // Windows assets asynchronously, so select the newest stable release that
    // actually contains the expected verified asset.
    for release in &releases {
        if release.draft || release.prerelease {
            continue;
        }
        if release
            .assets
            .iter()
            .any(|item| entry.asset_matches(&item.name))
        {
            return release_from_asset(entry, release, None);
        }
    }

    Err(format!(
        "No recent release for '{}' exposes the expected Windows asset '{}'.",
        entry.display_name,
        entry.asset_descriptor()
    ))
}

fn release_from_asset(
    entry: &ConnectorCatalogEntry,
    release: &GitHubRelease,
    version_override: Option<&str>,
) -> Result<LatestRelease, String> {
    let asset = release
        .assets
        .iter()
        .find(|item| entry.asset_matches(&item.name))
        .ok_or_else(|| {
            format!(
                "Release '{}' for '{}' does not contain the expected Windows asset '{}'.",
                release.tag_name,
                entry.display_name,
                entry.asset_descriptor()
            )
        })?;
    let digest = required_sha256(asset.digest.as_deref()).map_err(|error| {
        format!(
            "Release asset '{}' for '{}' cannot be trusted: {}",
            asset.name, entry.display_name, error
        )
    })?;
    Ok(LatestRelease {
        version: version_override
            .map(str::to_string)
            .unwrap_or_else(|| release.tag_name.trim_start_matches('v').to_string()),
        asset_url: asset.browser_download_url.clone(),
        asset_digest: digest,
    })
}

fn required_sha256(digest: Option<&str>) -> Result<String, String> {
    let value = digest.ok_or_else(|| "GitHub did not provide an asset digest.".to_string())?;
    let hash = value
        .strip_prefix("sha256:")
        .ok_or_else(|| format!("Unsupported asset digest '{}'.", value))?;
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("Invalid SHA-256 asset digest '{}'.", value));
    }
    Ok(value.to_ascii_lowercase())
}

fn verify_asset_digest(
    entry: &ConnectorCatalogEntry,
    bytes: &[u8],
    expected_digest: &str,
) -> Result<(), String> {
    let expected = required_sha256(Some(expected_digest))?;
    let actual = format!("sha256:{:x}", Sha256::digest(bytes));
    if actual != expected {
        return Err(format!(
            "SHA-256 mismatch for '{}': expected {}, received {}.",
            entry.display_name, expected, actual
        ));
    }
    Ok(())
}

fn download_release_asset(
    entry: &ConnectorCatalogEntry,
    asset_url: &str,
    connectors_root: &Path,
) -> Result<tempfile::NamedTempFile, String> {
    let client = github_client()?;
    let mut response = client
        .get(asset_url)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| {
            format!(
                "Failed to download '{}' update asset: {}",
                entry.display_name, error
            )
        })?;

    stage_download(entry, connectors_root, &mut response)
}

fn stage_download(
    entry: &ConnectorCatalogEntry,
    connectors_root: &Path,
    reader: &mut impl Read,
) -> Result<tempfile::NamedTempFile, String> {
    let download_root = connectors_root.join(".downloads");
    fs::create_dir_all(&download_root).map_err(|error| error.to_string())?;
    let mut download = tempfile::Builder::new()
        .prefix(&format!("{}-", entry.key))
        .suffix(".download")
        .tempfile_in(&download_root)
        .map_err(|error| error.to_string())?;
    std::io::copy(reader, download.as_file_mut()).map_err(|error| {
        format!(
            "Failed to write the temporary '{}' download: {}",
            entry.display_name, error
        )
    })?;
    download.as_file_mut().flush().map_err(|error| {
        format!(
            "Failed to flush the temporary '{}' download: {}",
            entry.display_name, error
        )
    })?;
    Ok(download)
}

fn install_release_asset(
    layout: &StorageLayout,
    entry: &ConnectorCatalogEntry,
    version: &str,
    bytes: &[u8],
) -> Result<PathBuf, String> {
    let install_path = managed_install_path(layout, entry, version);
    if install_path.exists() {
        if probe_connector_version(entry, &install_path).as_deref() == Ok(version) {
            return Ok(install_path);
        }
        fs::remove_file(&install_path).map_err(|error| {
            format!(
                "Failed to replace invalid '{}' runtime at '{}': {}",
                entry.display_name,
                install_path.display(),
                error
            )
        })?;
    }

    let parent = install_path
        .parent()
        .ok_or_else(|| "Connector install path has no parent directory.".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary_path = parent.join(format!(
        ".install-{}-{}",
        uuid::Uuid::new_v4(),
        entry.executable_name
    ));

    let result = (|| -> Result<(), String> {
        if let Some(member_name) = entry.archive_member_name.as_deref() {
            let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|error| {
                format!(
                    "Failed to open '{}' update archive: {}",
                    entry.display_name, error
                )
            })?;
            let mut member = archive.by_name(member_name).map_err(|error| {
                format!(
                    "Failed to locate '{}' inside '{}' update archive: {}",
                    member_name, entry.display_name, error
                )
            })?;
            let mut file = fs::File::create(&temporary_path).map_err(|error| error.to_string())?;
            std::io::copy(&mut member, &mut file).map_err(|error| error.to_string())?;
            file.flush().map_err(|error| error.to_string())?;
        } else {
            fs::write(&temporary_path, bytes).map_err(|error| {
                format!(
                    "Failed to stage '{}' at '{}': {}",
                    entry.display_name,
                    temporary_path.display(),
                    error
                )
            })?;
        }

        let observed_version = probe_connector_version(entry, &temporary_path)?;
        if observed_version != version {
            return Err(format!(
                "'{}' reported version '{}', expected '{}'.",
                entry.display_name, observed_version, version
            ));
        }
        fs::rename(&temporary_path, &install_path).map_err(|error| {
            format!(
                "Failed to activate '{}' at '{}': {}",
                entry.display_name,
                install_path.display(),
                error
            )
        })?;
        Ok(())
    })();

    if let Err(error) = result {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }
    Ok(install_path)
}

fn probe_connector_version(
    entry: &ConnectorCatalogEntry,
    executable_path: &Path,
) -> Result<String, String> {
    let mut command = Command::new(executable_path);
    configure_background_command(&mut command);
    let output = command
        .args(&entry.version_args)
        .output()
        .map_err(|error| {
            format!(
                "Failed to probe '{}' version: {}",
                entry.display_name, error
            )
        })?;

    if !output.status.success() {
        return Err(format!(
            "'{}' version probe exited with status {:?}.",
            entry.display_name,
            output.status.code()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let raw = if stdout.is_empty() { stderr } else { stdout };
    let normalized = raw
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .trim_start_matches('v');
    if normalized.is_empty() {
        return Err(format!(
            "'{}' version probe returned an empty value.",
            entry.display_name
        ));
    }

    Ok(normalized.to_string())
}

fn configure_background_command(command: &mut Command) {
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

fn is_custom_connector_path(entry: &ConnectorCatalogEntry, value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }

    let normalized = trimmed.replace('\\', "/").to_ascii_lowercase();
    let executable_name = entry.executable_name.to_ascii_lowercase();
    let default_command = entry.default_command.to_ascii_lowercase();
    normalized != executable_name && normalized != default_command
}

fn derive_status(
    management_mode: &str,
    active_version: Option<&str>,
    latest_version: Option<&str>,
    pending_version: Option<&str>,
) -> String {
    if management_mode == "custom" {
        return "custom_override".to_string();
    }
    if pending_version.is_some() {
        return "pending_activation".to_string();
    }
    let Some(active_version) = active_version else {
        return "not_installed".to_string();
    };
    if latest_version.is_some_and(|latest| latest != active_version) {
        return "update_available".to_string();
    }
    "up_to_date".to_string()
}

fn github_client() -> Result<Client, String> {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("NinjaCrawler/0.1.0 connector-runtime"),
    );
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );

    Client::builder()
        .default_headers(headers)
        .build()
        .map_err(|error| {
            format!(
                "Failed to build HTTP client for connector updates: {}",
                error
            )
        })
}

fn parse_timestamp(value: &str) -> Option<chrono::DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn emit_runtime_changed() {
    let app = app_handle_registry()
        .lock()
        .ok()
        .and_then(|handle| handle.as_ref().cloned());

    if let Some(app) = app {
        let _ = app.emit(CONNECTOR_RUNTIME_CHANGED_EVENT, ());
    }
}

fn now_timestamp() -> String {
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_with_asset(
        asset_name: &str,
        prefix: Option<&str>,
        suffix: Option<&str>,
    ) -> ConnectorCatalogEntry {
        ConnectorCatalogEntry {
            key: "test".to_string(),
            display_name: "Test".to_string(),
            tool_setting_key: "tool.test.path".to_string(),
            default_command: "test".to_string(),
            bundled_version: "1.0".to_string(),
            release_tag: Some("v1.0".to_string()),
            executable_name: "test.exe".to_string(),
            version_args: vec!["--version".to_string()],
            release_api_url: "https://example.invalid/releases".to_string(),
            asset_name: asset_name.to_string(),
            asset_prefix: prefix.map(str::to_string),
            asset_suffix: suffix.map(str::to_string),
            archive_member_name: None,
        }
    }

    #[test]
    fn exact_asset_name_matches_case_insensitively() {
        let entry = entry_with_asset("gallery-dl.exe", None, None);
        assert!(entry.asset_matches("gallery-dl.exe"));
        assert!(entry.asset_matches("Gallery-DL.EXE"));
        assert!(!entry.asset_matches("gallery-dl_x86.exe"));
        assert!(!entry.asset_matches("gallery-dl.bin"));
    }

    #[test]
    fn prefix_suffix_matches_versioned_asset_names() {
        // Instaloader includes its version in the asset name.
        let entry = entry_with_asset(
            "instaloader-windows-standalone.zip",
            Some("instaloader-"),
            Some("-windows-standalone.zip"),
        );
        assert!(entry.asset_matches("instaloader-v4.15-windows-standalone.zip"));
        assert!(entry.asset_matches("instaloader-v4.15.1-windows-standalone.zip"));
        assert!(entry.asset_matches("instaloader-v5.0-windows-standalone.zip"));
        assert!(!entry.asset_matches("instaloader-v4.15-linux.tar.gz"));
        assert!(!entry.asset_matches("something-else.zip"));
    }

    #[test]
    fn required_sha256_rejects_missing_and_malformed_digests() {
        assert!(required_sha256(None).is_err());
        assert!(required_sha256(Some("sha512:abc")).is_err());
        assert!(required_sha256(Some("sha256:abc")).is_err());
    }

    #[test]
    fn asset_digest_verification_is_fail_closed() {
        let entry = entry_with_asset("test.exe", None, None);
        let bytes = b"verified connector";
        let digest = format!("sha256:{:x}", Sha256::digest(bytes));
        assert!(verify_asset_digest(&entry, bytes, &digest).is_ok());
        assert!(verify_asset_digest(&entry, b"tampered connector", &digest).is_err());
    }

    #[test]
    fn managed_status_is_not_installed_without_an_active_version() {
        assert_eq!(derive_status("managed", None, None, None), "not_installed");
        assert_eq!(
            derive_status("managed", Some("1.0"), Some("2.0"), None),
            "update_available"
        );
    }

    #[test]
    fn invalid_archive_never_creates_an_active_runtime() {
        let temporary = tempfile::tempdir().expect("temporary connector root");
        let root = temporary.path().to_path_buf();
        let layout = StorageLayout {
            root: root.clone(),
            data_dir: root.join("data"),
            logs_dir: root.join("logs"),
            db_path: root.join("data/ninjacrawler.db"),
            media_root: root.join("media"),
            cache_root: root.join("cache"),
            connectors_root: root.join("connectors"),
        };
        let mut entry = entry_with_asset("test.zip", None, None);
        entry.archive_member_name = Some("test.exe".to_string());

        assert!(install_release_asset(&layout, &entry, "1.0", b"not a zip").is_err());
        let install_dir = layout.connectors_root.join("test/1.0");
        assert!(!install_dir.join("test.exe").exists());
        assert_eq!(
            fs::read_dir(install_dir)
                .map(|entries| entries.count())
                .unwrap_or(0),
            0
        );
    }

    #[test]
    fn interrupted_download_removes_its_temporary_file() {
        struct InterruptedReader(bool);
        impl Read for InterruptedReader {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                if self.0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::ConnectionReset,
                        "interrupted",
                    ));
                }
                self.0 = true;
                let partial = b"partial";
                buffer[..partial.len()].copy_from_slice(partial);
                Ok(partial.len())
            }
        }

        let temporary = tempfile::tempdir().expect("temporary connector root");
        let entry = entry_with_asset("test.exe", None, None);
        let mut reader = InterruptedReader(false);
        assert!(stage_download(&entry, temporary.path(), &mut reader).is_err());
        let download_root = temporary.path().join(".downloads");
        assert_eq!(
            fs::read_dir(download_root)
                .map(|entries| entries.count())
                .unwrap_or(0),
            0
        );
    }

    #[cfg(windows)]
    #[test]
    fn version_probe_requires_a_successful_executable() {
        let mut entry = entry_with_asset("cmd.exe", None, None);
        entry.version_args = vec!["/c".to_string(), "echo 1.0".to_string()];
        assert_eq!(
            probe_connector_version(&entry, Path::new(r"C:\Windows\System32\cmd.exe"))
                .expect("cmd version probe"),
            "1.0"
        );
        assert!(probe_connector_version(&entry, Path::new(r"C:\missing\connector.exe")).is_err());
    }
}
