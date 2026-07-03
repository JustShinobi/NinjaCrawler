use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use crate::domain::models::{ConnectorDebugEntry, ConnectorDebugQuery};

pub const CONNECTOR_DEBUG_APPENDED_EVENT: &str = "runtime://connector-debug-appended";
const MAX_ENTRIES: usize = 5_000;
const MAX_RAW_CHARS: usize = 128_000;
const MAX_TOTAL_RAW_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Default)]
pub struct ConnectorDebugContext {
    pub source_id: Option<String>,
    pub provider: Option<String>,
    pub handle: Option<String>,
}

thread_local! {
    static CURRENT_CONTEXT: RefCell<Option<ConnectorDebugContext>> = const { RefCell::new(None) };
}

#[derive(Default)]
struct ConnectorDebugBuffer {
    entries: VecDeque<ConnectorDebugEntry>,
    raw_bytes: usize,
}

fn entries() -> &'static Mutex<ConnectorDebugBuffer> {
    static ENTRIES: OnceLock<Mutex<ConnectorDebugBuffer>> = OnceLock::new();
    ENTRIES.get_or_init(|| Mutex::new(ConnectorDebugBuffer::default()))
}

fn app_handle() -> &'static Mutex<Option<AppHandle>> {
    static APP_HANDLE: OnceLock<Mutex<Option<AppHandle>>> = OnceLock::new();
    APP_HANDLE.get_or_init(|| Mutex::new(None))
}

pub fn register_app_handle(app: &AppHandle) {
    if let Ok(mut slot) = app_handle().lock() {
        *slot = Some(app.clone());
    }
}

pub struct ConnectorDebugContextGuard {
    previous: Option<ConnectorDebugContext>,
}

impl Drop for ConnectorDebugContextGuard {
    fn drop(&mut self) {
        CURRENT_CONTEXT.with(|slot| *slot.borrow_mut() = self.previous.take());
    }
}

pub fn enter(
    source_id: impl Into<String>,
    provider: impl Into<String>,
    handle: impl Into<String>,
) -> ConnectorDebugContextGuard {
    let context = ConnectorDebugContext {
        source_id: Some(source_id.into()),
        provider: Some(provider.into()),
        handle: Some(handle.into()),
    };
    let previous = CURRENT_CONTEXT.with(|slot| slot.borrow_mut().replace(context));
    ConnectorDebugContextGuard { previous }
}

pub fn current_context() -> ConnectorDebugContext {
    CURRENT_CONTEXT.with(|slot| slot.borrow().clone().unwrap_or_default())
}

pub fn append_current(
    connector: &str,
    event_type: &str,
    operation: impl Into<String>,
    raw: impl Into<String>,
) {
    append_with_context(current_context(), connector, event_type, operation, raw);
}

pub fn append_with_context(
    context: ConnectorDebugContext,
    connector: &str,
    event_type: &str,
    operation: impl Into<String>,
    raw: impl Into<String>,
) {
    let raw = redact_sensitive_text(&raw.into());
    let raw = if raw.chars().count() > MAX_RAW_CHARS {
        format!(
            "{}\n… [truncated by connector debugger]",
            raw.chars().take(MAX_RAW_CHARS).collect::<String>()
        )
    } else {
        raw
    };
    let entry = ConnectorDebugEntry {
        id: Uuid::new_v4().to_string(),
        timestamp: Utc::now().to_rfc3339(),
        source_id: context.source_id,
        provider: context.provider,
        source_handle: context.handle,
        connector: connector.trim().to_string(),
        event_type: event_type.trim().to_ascii_lowercase(),
        operation: operation.into(),
        raw,
    };

    if let Ok(mut buffer) = entries().lock() {
        buffer.raw_bytes = buffer.raw_bytes.saturating_add(entry.raw.len());
        buffer.entries.push_back(entry.clone());
        while buffer.entries.len() > MAX_ENTRIES || buffer.raw_bytes > MAX_TOTAL_RAW_BYTES {
            if let Some(removed) = buffer.entries.pop_front() {
                buffer.raw_bytes = buffer.raw_bytes.saturating_sub(removed.raw.len());
            } else {
                break;
            }
        }
    }

    let app = app_handle()
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().cloned());
    if let Some(app) = app {
        let _ = app.emit(CONNECTOR_DEBUG_APPENDED_EVENT, &entry);
    }
}

pub fn query(filter: ConnectorDebugQuery) -> Vec<ConnectorDebugEntry> {
    let Ok(buffer) = entries().lock() else {
        return Vec::new();
    };
    let limit = filter.limit.unwrap_or(1_000).clamp(1, MAX_ENTRIES as u32) as usize;
    buffer
        .entries
        .iter()
        .rev()
        .filter(|entry| {
            filter
                .provider
                .as_deref()
                .is_none_or(|value| entry.provider.as_deref() == Some(value))
        })
        .filter(|entry| {
            filter
                .source_id
                .as_deref()
                .is_none_or(|value| entry.source_id.as_deref() == Some(value))
        })
        .filter(|entry| {
            filter
                .event_type
                .as_deref()
                .is_none_or(|value| entry.event_type.eq_ignore_ascii_case(value))
        })
        .take(limit)
        .cloned()
        .collect()
}

pub fn clear() {
    if let Ok(mut buffer) = entries().lock() {
        buffer.entries.clear();
        buffer.raw_bytes = 0;
    }
}

fn redact_json_value(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let normalized = key.to_ascii_lowercase();
                if normalized.contains("cookie")
                    || normalized.contains("authorization")
                    || normalized.contains("password")
                    || normalized.contains("session")
                    || normalized.contains("csrf")
                    || normalized == "token"
                    || normalized.ends_with("_token")
                {
                    *child = Value::String("[REDACTED]".to_string());
                } else {
                    redact_json_value(child);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact_json_value),
        _ => {}
    }
}

pub fn redact_sensitive_text(raw: &str) -> String {
    if let Ok(mut json) = serde_json::from_str::<Value>(raw) {
        redact_json_value(&mut json);
        return serde_json::to_string_pretty(&json).unwrap_or_else(|_| raw.to_string());
    }

    raw.lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if ["authorization:", "cookie:", "set-cookie:"]
                .iter()
                .any(|name| lower.trim_start().starts_with(name))
            {
                let prefix = line
                    .split_once(':')
                    .map(|(prefix, _)| prefix)
                    .unwrap_or(line);
                return format!("{prefix}: [REDACTED]");
            }

            let mut sanitized = line.to_string();
            for key in [
                "sessionid=",
                "csrftoken=",
                "auth_token=",
                "authorization=",
                "authorization:",
                "cookie:",
                "x-csrftoken:",
                "--password ",
            ] {
                let lower_sanitized = sanitized.to_ascii_lowercase();
                if let Some(start) = lower_sanitized.find(key) {
                    let value_start = start + key.len();
                    let value_end = sanitized[value_start..]
                        .find(|character: char| {
                            character.is_whitespace() || character == ';' || character == '&'
                        })
                        .map(|offset| value_start + offset)
                        .unwrap_or(sanitized.len());
                    sanitized.replace_range(value_start..value_end, "[REDACTED]");
                }
            }
            sanitized
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::redact_sensitive_text;

    #[test]
    fn redacts_secrets_without_hiding_raw_payload() {
        let json = redact_sensitive_text(r#"{"status":"ok","sessionid":"secret","items":[1,2]}"#);
        assert!(json.contains("\"status\": \"ok\""));
        assert!(json.contains("[REDACTED]"));
        assert!(!json.contains("secret"));

        let headers = redact_sensitive_text("HTTP 200\nCookie: sessionid=secret\nbody");
        assert!(headers.contains("HTTP 200"));
        assert!(headers.contains("Cookie: [REDACTED]"));
        assert!(headers.contains("body"));
    }
}
