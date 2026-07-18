use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use crate::domain::models::{RuntimeLogEntry, RuntimeLogQuery};
use crate::infrastructure::{storage, storage::StorageLayout};

pub const RUNTIME_LOG_APPENDED_EVENT: &str = "runtime://runtime-log-appended";
const MAX_LOG_LINES: usize = 20_000;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedRuntimeLogEntry {
    id: Option<String>,
    timestamp: String,
    scope: String,
    level: String,
    account_id: Option<String>,
    provider: Option<String>,
    source_id: Option<String>,
    source_handle: Option<String>,
    message: String,
    detail: Option<String>,
}

fn app_handle_registry() -> &'static Mutex<Option<AppHandle>> {
    static HANDLE: OnceLock<Mutex<Option<AppHandle>>> = OnceLock::new();
    HANDLE.get_or_init(|| Mutex::new(None))
}

pub fn register_app_handle(app: &AppHandle) {
    if let Ok(mut handle) = app_handle_registry().lock() {
        *handle = Some(app.clone());
    }
}

fn create_runtime_log_id() -> String {
    Uuid::new_v4().to_string()
}

fn to_runtime_log_entry(entry: PersistedRuntimeLogEntry) -> RuntimeLogEntry {
    RuntimeLogEntry {
        id: entry.id.unwrap_or_else(create_runtime_log_id),
        timestamp: entry.timestamp,
        scope: entry.scope,
        level: entry.level,
        account_id: entry.account_id,
        provider: entry.provider,
        source_id: entry.source_id,
        source_handle: entry.source_handle,
        message: entry.message,
        detail: entry.detail,
    }
}

fn emit_appended(entry: &RuntimeLogEntry) {
    let app = app_handle_registry()
        .lock()
        .ok()
        .and_then(|handle| handle.as_ref().cloned());

    if let Some(app) = app {
        let _ = app.emit(RUNTIME_LOG_APPENDED_EVENT, entry);
    }
}

/// Identificadores opcionais que ancoram uma entrada de log à conta e ao
/// perfil de origem. Agrupados num struct para não carregar quatro
/// `Option<&str>` posicionais (e trocáveis) em toda assinatura de log.
#[derive(Clone, Copy, Default)]
pub struct RuntimeLogAnchor<'a> {
    pub account_id: Option<&'a str>,
    pub provider: Option<&'a str>,
    pub source_id: Option<&'a str>,
    pub source_handle: Option<&'a str>,
}

pub fn append_workspace(
    scope: &str,
    level: &str,
    context: RuntimeLogAnchor<'_>,
    message: impl Into<String>,
    detail: Option<String>,
) -> Result<RuntimeLogEntry, String> {
    let layout = storage::ensure_workspace_layout().map_err(|error| error.to_string())?;
    append(&layout, scope, level, context, message, detail)
}

pub fn append(
    layout: &StorageLayout,
    scope: &str,
    level: &str,
    context: RuntimeLogAnchor<'_>,
    message: impl Into<String>,
    detail: Option<String>,
) -> Result<RuntimeLogEntry, String> {
    fs::create_dir_all(&layout.logs_dir).map_err(|error| error.to_string())?;
    let path = runtime_log_path(layout);
    let persisted_entry = PersistedRuntimeLogEntry {
        id: Some(create_runtime_log_id()),
        timestamp: Utc::now().to_rfc3339(),
        scope: scope.to_string(),
        level: level.to_string(),
        account_id: context.account_id.map(str::to_string),
        provider: context.provider.map(str::to_string),
        source_id: context.source_id.map(str::to_string),
        source_handle: context.source_handle.map(str::to_string),
        message: message.into(),
        detail,
    };
    let serialized = serde_json::to_string(&persisted_entry).map_err(|error| error.to_string())?;

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    writeln!(file, "{serialized}").map_err(|error| error.to_string())?;

    let entry = to_runtime_log_entry(persisted_entry);
    emit_appended(&entry);
    Ok(entry)
}

pub fn tail(layout: &StorageLayout, limit: usize) -> Result<Vec<RuntimeLogEntry>, String> {
    query(
        layout,
        RuntimeLogQuery {
            limit: Some(limit as u32),
            level: None,
            scope: None,
            provider: None,
            account_id: None,
            source_id: None,
        },
    )
}

pub fn query(
    layout: &StorageLayout,
    filter: RuntimeLogQuery,
) -> Result<Vec<RuntimeLogEntry>, String> {
    let path = runtime_log_path(layout);
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    let reader = BufReader::new(file);
    let mut lines = VecDeque::with_capacity(MAX_LOG_LINES);

    for line in reader.lines() {
        let line = line.map_err(|error| error.to_string())?;
        if lines.len() == MAX_LOG_LINES {
            lines.pop_front();
        }
        lines.push_back(line);
    }

    let limit = filter.limit.unwrap_or(200) as usize;
    let entries = lines
        .into_iter()
        .rev()
        .filter_map(|line| serde_json::from_str::<PersistedRuntimeLogEntry>(&line).ok())
        .filter(|entry| {
            filter
                .level
                .as_deref()
                .is_none_or(|value| entry.level.eq_ignore_ascii_case(value))
        })
        .filter(|entry| {
            filter
                .scope
                .as_deref()
                .is_none_or(|value| entry.scope.eq_ignore_ascii_case(value))
        })
        .filter(|entry| {
            filter.provider.as_deref().is_none_or(|value| {
                entry
                    .provider
                    .as_deref()
                    .is_some_and(|provider| provider.eq_ignore_ascii_case(value))
            })
        })
        .filter(|entry| {
            filter.account_id.as_deref().is_none_or(|value| {
                entry
                    .account_id
                    .as_deref()
                    .is_some_and(|account_id| account_id == value)
            })
        })
        .filter(|entry| {
            filter.source_id.as_deref().is_none_or(|value| {
                entry
                    .source_id
                    .as_deref()
                    .is_some_and(|source_id| source_id == value)
            })
        })
        .take(limit)
        .map(to_runtime_log_entry)
        .collect::<Vec<_>>();

    Ok(entries)
}

fn runtime_log_path(layout: &StorageLayout) -> PathBuf {
    layout.logs_dir.join("runtime.log")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_layout() -> (TempDir, StorageLayout) {
        let temp_dir = TempDir::new().expect("temp dir");
        let root = temp_dir.path().to_path_buf();
        let data_dir = root.join("data");
        let logs_dir = root.join("logs");
        let db_path = data_dir.join("ninjacrawler.db");
        let media_root = root.join("media");
        let cache_root = root.join("cache");
        let connectors_root = data_dir.join("connectors");

        fs::create_dir_all(&data_dir).expect("data dir");
        fs::create_dir_all(&logs_dir).expect("logs dir");
        fs::create_dir_all(&media_root).expect("media dir");
        fs::create_dir_all(&cache_root).expect("cache dir");
        fs::create_dir_all(&connectors_root).expect("connectors dir");

        (
            temp_dir,
            StorageLayout {
                root,
                data_dir,
                logs_dir,
                db_path,
                media_root,
                cache_root,
                connectors_root,
            },
        )
    }

    #[test]
    fn query_reads_only_recent_log_lines() {
        let (_temp_dir, layout) = create_test_layout();

        for index in 0..(MAX_LOG_LINES + 250) {
            append(
                &layout,
                "runtime.test",
                "info",
                RuntimeLogAnchor::default(),
                format!("entry-{index}"),
                None,
            )
            .expect("append log");
        }

        let entries = query(
            &layout,
            RuntimeLogQuery {
                limit: Some(20),
                level: None,
                scope: None,
                provider: None,
                account_id: None,
                source_id: None,
            },
        )
        .expect("query log");

        assert_eq!(entries.len(), 20);
        assert_eq!(
            entries.first().map(|entry| entry.message.as_str()),
            Some("entry-20249")
        );
        assert_eq!(
            entries.last().map(|entry| entry.message.as_str()),
            Some("entry-20230")
        );
        assert!(entries.iter().all(|entry| !entry.id.is_empty()));
    }
}
