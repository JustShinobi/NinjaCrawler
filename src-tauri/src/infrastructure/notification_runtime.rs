//! Native system notifications when a source sync batch finishes.
//!
//! The hook point is the batch boundary of the queue worker in
//! `source_sync_runtime` (when a provider queue drains): we aggregate the
//! results of jobs from that batch into a single notification, avoiding spam
//! (one per batch, never one per downloaded item).
//!
//! The notification respects the "silent mode" already modeled in
//! `DesktopRuntimeState.silent_mode` (tray menu): silent = no native
//! notification. This is the authoritative gate; the `notification_mode`
//! per-plan (`SchedulerPlanNotifications`) governs the style per plan, but
//! since queued jobs don't carry the plan identity (the scheduler only
//! enqueues with trigger "scheduler"), the aggregated batch uses a neutral summary.

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::infrastructure::workspace_repository;

/// Result of a single job within a batch, accumulated by the worker.
#[derive(Clone)]
pub struct SyncBatchItem {
    pub handle: String,
    /// "failed" | "cancelled" | "warning" | anything else = success.
    pub status: String,
    pub downloaded_items: u32,
    pub summary: String,
}

/// Accumulator of a sync batch for a provider (one worker run until the
/// queue drains). Empty = nothing to notify.
#[derive(Clone, Default)]
pub struct SyncBatch {
    pub provider: String,
    pub items: Vec<SyncBatchItem>,
}

impl SyncBatch {
    pub fn new(provider: String) -> Self {
        Self {
            provider,
            items: Vec::new(),
        }
    }

    pub fn push(&mut self, item: SyncBatchItem) {
        self.items.push(item);
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn completed(&self) -> usize {
        self.items
            .iter()
            .filter(|item| !is_failure_status(&item.status))
            .count()
    }

    fn failed(&self) -> usize {
        self.items
            .iter()
            .filter(|item| is_failure_status(&item.status))
            .count()
    }

    fn downloaded_items(&self) -> u32 {
        self.items
            .iter()
            .map(|item| item.downloaded_items)
            .fold(0u32, |acc, value| acc.saturating_add(value))
    }

    fn first_failure(&self) -> Option<&SyncBatchItem> {
        self.items
            .iter()
            .find(|item| is_failure_status(&item.status))
    }
}

fn is_failure_status(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "failed" | "cancelled" | "error"
    )
}

fn provider_label(provider: &str) -> String {
    let trimmed = provider.trim();
    if trimmed.is_empty() {
        return "Sources".to_string();
    }
    let mut chars = trimmed.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => trimmed.to_string(),
    }
}

/// Builds the (title, body) of the notification from the aggregated batch.
fn compose_message(batch: &SyncBatch) -> (String, String) {
    let provider = provider_label(&batch.provider);
    let completed = batch.completed();
    let failed = batch.failed();
    let downloaded = batch.downloaded_items();

    let source_word = |count: usize| if count == 1 { "source" } else { "sources" };

    if failed > 0 {
        let title = if completed == 0 {
            format!("{provider} sync failed")
        } else {
            format!("{provider} sync finished with errors")
        };
        let mut body = if completed > 0 {
            format!(
                "{completed} {} synced, {failed} failed",
                source_word(completed)
            )
        } else {
            format!("{failed} {} failed", source_word(failed))
        };
        if downloaded > 0 {
            body.push_str(&format!("; {downloaded} new item(s) downloaded"));
        }
        if let Some(failure) = batch.first_failure() {
            let detail = failure.summary.trim();
            if !detail.is_empty() {
                body.push_str(&format!("\n{}: {}", failure.handle, truncate(detail, 120)));
            }
        }
        (title, body)
    } else {
        let title = format!("{provider} sync complete");
        let body = if downloaded > 0 {
            format!(
                "{completed} {} synced, {downloaded} new item(s) downloaded",
                source_word(completed)
            )
        } else {
            format!("{completed} {} synced, no new items", source_word(completed))
        };
        (title, body)
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Emits an aggregated native notification for the batch, respecting silent
/// mode. No-op when the batch is empty, when the app is in silent mode, or
/// when the plugin/state fails (never propagates error to the sync worker).
pub fn notify_sync_batch(app: &AppHandle, batch: &SyncBatch) {
    if batch.is_empty() {
        return;
    }

    // Silent mode is the authoritative gate (tray menu "Silent mode").
    let silent = workspace_repository::desktop_runtime_state()
        .map(|state| state.silent_mode)
        .unwrap_or(false);
    if silent {
        return;
    }

    let (title, body) = compose_message(batch);

    if let Err(error) = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show()
    {
        eprintln!("failed to show sync notification: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(handle: &str, status: &str, downloaded: u32) -> SyncBatchItem {
        SyncBatchItem {
            handle: handle.to_string(),
            status: status.to_string(),
            downloaded_items: downloaded,
            summary: "detail".to_string(),
        }
    }

    #[test]
    fn success_batch_message_reports_counts() {
        let mut batch = SyncBatch::new("instagram".to_string());
        batch.push(item("alice", "succeeded", 3));
        batch.push(item("bob", "succeeded", 2));
        let (title, body) = compose_message(&batch);
        assert_eq!(title, "Instagram sync complete");
        assert_eq!(body, "2 sources synced, 5 new item(s) downloaded");
    }

    #[test]
    fn failure_batch_message_flags_errors() {
        let mut batch = SyncBatch::new("twitter".to_string());
        batch.push(item("alice", "succeeded", 1));
        batch.push(item("bob", "failed", 0));
        let (title, body) = compose_message(&batch);
        assert_eq!(title, "Twitter sync finished with errors");
        assert!(body.starts_with("1 source synced, 1 failed"));
        assert!(body.contains("bob:"));
    }

    #[test]
    fn all_failed_batch_uses_failed_title() {
        let mut batch = SyncBatch::new("tiktok".to_string());
        batch.push(item("alice", "failed", 0));
        let (title, _) = compose_message(&batch);
        assert_eq!(title, "Tiktok sync failed");
    }
}
