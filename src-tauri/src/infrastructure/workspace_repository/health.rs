use super::*;
use crate::domain::models::{
    AccountHealthItem, SourceHealthItem, StorageRootHealth, StorageVolumeHealth,
    WorkspaceHealthCounts, WorkspaceHealthIncident, WorkspaceHealthSnapshot,
};
use std::collections::BTreeMap;

const GIB: u64 = 1024 * 1024 * 1024;
const CRITICAL_FREE_BYTES: u64 = 5 * GIB;
const ATTENTION_FREE_BYTES: u64 = 20 * GIB;

fn source_incident_actions() -> Vec<String> {
    [
        "retry_sync",
        "open_profile",
        "open_folder",
        "open_filtered_log",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn display_handle(handle: &str) -> String {
    if handle.starts_with('@') {
        handle.to_string()
    } else {
        format!("@{handle}")
    }
}

fn source_severity(
    recurring_failure: bool,
    has_problem: bool,
    latest_status: Option<&str>,
) -> &'static str {
    if recurring_failure || has_problem || latest_status == Some("failed") {
        "attention"
    } else {
        "healthy"
    }
}

fn source_incident_kind(
    problem_code: Option<&str>,
    recurring_failure: bool,
    latest_status: Option<&str>,
) -> Option<&'static str> {
    if problem_code.is_some() {
        Some("source_problem")
    } else if recurring_failure {
        Some("source_recurring_failure")
    } else if latest_status == Some("failed") {
        Some("source_latest_failure")
    } else {
        None
    }
}

pub fn load_workspace_health() -> Result<WorkspaceHealthSnapshot, String> {
    with_workspace(|connection, layout| {
        let snapshot = load_snapshot(connection, layout)?;
        Ok(build_workspace_health(snapshot, Utc::now()))
    })
}

fn build_workspace_health(
    snapshot: WorkspaceSnapshot,
    now: DateTime<Utc>,
) -> WorkspaceHealthSnapshot {
    let mut incidents = Vec::new();
    let mut runs_by_source = HashMap::<String, Vec<SourceSyncRun>>::new();
    for run in &snapshot.source_sync_runs {
        runs_by_source
            .entry(run.source_id.clone())
            .or_default()
            .push(run.clone());
    }

    let mut sources = Vec::with_capacity(snapshot.sources.len());
    for source in &snapshot.sources {
        let recent_runs = runs_by_source.get(&source.id).cloned().unwrap_or_default();
        let consecutive_failures =
            consecutive_failure_count(recent_runs.iter().map(|run| run.status.as_str()));
        let recurring_failure = consecutive_failures >= 3;
        let latest_status = recent_runs.first().map(|run| run.status.clone());
        let freshness = source_freshness(source.last_synced_at.as_deref(), now);
        let severity = source_severity(
            recurring_failure,
            source.sync_problem_code.is_some(),
            latest_status.as_deref(),
        );

        if let Some(kind) = source_incident_kind(
            source.sync_problem_code.as_deref(),
            recurring_failure,
            latest_status.as_deref(),
        ) {
            let problem_code = source.sync_problem_code.as_deref();
            let title = if problem_code.is_some() {
                format!(
                    "{} has a blocking sync problem",
                    display_handle(&source.handle)
                )
            } else if recurring_failure {
                format!(
                    "{} failed {} consecutive syncs",
                    display_handle(&source.handle),
                    consecutive_failures
                )
            } else {
                format!("{} latest sync failed", display_handle(&source.handle))
            };
            let detail = source
                .sync_problem_message
                .clone()
                .or_else(|| recent_runs.first().map(|run| run.summary.clone()))
                .or_else(|| problem_code.map(str::to_string))
                .unwrap_or_else(|| "The latest source sync failed.".to_string());
            let mut evidence = Vec::new();
            if let Some(problem_code) = problem_code {
                if let Some(recorded_at) = source.sync_problem_at.as_deref() {
                    evidence.push(format!("Recorded at {recorded_at}"));
                }
                evidence.push(format!("Problem code: {problem_code}"));
            }
            evidence.extend(
                recent_runs
                    .iter()
                    .take(if recurring_failure {
                        consecutive_failures as usize
                    } else {
                        1
                    })
                    .map(|run| format!("{} · {}", run.finished_at, run.summary)),
            );
            incidents.push(WorkspaceHealthIncident {
                id: format!("source:{}", source.id),
                severity: "attention".to_string(),
                kind: kind.to_string(),
                title,
                detail,
                source_id: Some(source.id.clone()),
                account_id: source.account_id.clone(),
                volume_key: None,
                evidence,
                available_actions: source_incident_actions(),
            });
        }

        sources.push(SourceHealthItem {
            source_id: source.id.clone(),
            provider: source.provider.clone(),
            handle: source.handle.clone(),
            display_name: source.display_name.clone(),
            account_id: source.account_id.clone(),
            last_synced_at: source.last_synced_at.clone(),
            latest_status,
            consecutive_failures,
            recurring_failure,
            freshness,
            severity: severity.to_string(),
            problem_code: source.sync_problem_code.clone(),
            problem_message: source.sync_problem_message.clone(),
            recent_runs: recent_runs.into_iter().take(5).collect(),
        });
    }

    let sessions_by_account = snapshot
        .account_sessions
        .iter()
        .map(|session| (session.account_id.as_str(), session))
        .collect::<HashMap<_, _>>();
    let mut accounts = Vec::with_capacity(snapshot.accounts.len());
    for account in &snapshot.accounts {
        let session = sessions_by_account.get(account.id.as_str()).copied();
        let has_session = session.is_some();
        let has_secret = session.is_some_and(|value| value.has_secret);
        let impacted_source_count = snapshot
            .sources
            .iter()
            .filter(|source| source.account_id.as_deref() == Some(account.id.as_str()))
            .count() as u32;
        let severity = account_severity(&account.auth_state, has_session, has_secret);
        let validation_error = session.and_then(|value| value.last_validation_error.clone());
        if severity != "healthy" {
            let reason = if !has_session {
                "No stored session is available.".to_string()
            } else if !has_secret {
                "The stored session secret is unavailable.".to_string()
            } else {
                validation_error
                    .clone()
                    .unwrap_or_else(|| format!("Account state is {}.", account.auth_state))
            };
            incidents.push(WorkspaceHealthIncident {
                id: format!("account:{}", account.id),
                severity: severity.to_string(),
                kind: "account_session".to_string(),
                title: if !has_session {
                    format!("{} has no stored session", account.display_name)
                } else if !has_secret {
                    format!("{} session secret is missing", account.display_name)
                } else {
                    format!("{} session is {}", account.display_name, account.auth_state)
                },
                detail: format!("{reason} {impacted_source_count} source(s) use this account."),
                source_id: None,
                account_id: Some(account.id.clone()),
                volume_key: None,
                evidence: vec![
                    format!("Provider: {}", account.provider),
                    format!("Last validated: {}", account.last_validated_at),
                ],
                available_actions: vec![
                    "validate_account".to_string(),
                    "open_account".to_string(),
                    "open_filtered_log".to_string(),
                ],
            });
        }
        accounts.push(AccountHealthItem {
            account_id: account.id.clone(),
            provider: account.provider.clone(),
            display_name: account.display_name.clone(),
            auth_state: account.auth_state.clone(),
            has_session,
            has_secret,
            last_validated_at: session
                .and_then(|value| value.last_validated_at.clone())
                .or_else(|| Some(account.last_validated_at.clone())),
            last_validation_error: validation_error,
            impacted_source_count,
            severity: severity.to_string(),
        });
    }

    let volumes = build_storage_health(&snapshot, &mut incidents);
    incidents.sort_by(|left, right| {
        severity_rank(&right.severity)
            .cmp(&severity_rank(&left.severity))
            .then_with(|| left.title.cmp(&right.title))
    });
    sources.sort_by(|left, right| {
        severity_rank(&right.severity)
            .cmp(&severity_rank(&left.severity))
            .then_with(|| {
                left.handle
                    .to_ascii_lowercase()
                    .cmp(&right.handle.to_ascii_lowercase())
            })
    });
    accounts.sort_by(|left, right| {
        severity_rank(&right.severity)
            .cmp(&severity_rank(&left.severity))
            .then_with(|| left.display_name.cmp(&right.display_name))
    });

    let critical_issue_count = incidents
        .iter()
        .filter(|item| item.severity == "critical")
        .count() as u32;
    let attention_issue_count = incidents
        .iter()
        .filter(|item| item.severity == "attention")
        .count() as u32;
    let overall_status = if critical_issue_count > 0 {
        "critical"
    } else if attention_issue_count > 0 {
        "attention"
    } else {
        "healthy"
    };
    let counts = WorkspaceHealthCounts {
        source_count: sources.len() as u32,
        affected_source_count: sources
            .iter()
            .filter(|item| item.severity != "healthy")
            .count() as u32,
        recurring_failure_count: sources.iter().filter(|item| item.recurring_failure).count()
            as u32,
        degraded_account_count: accounts
            .iter()
            .filter(|item| item.severity == "attention")
            .count() as u32,
        critical_account_count: accounts
            .iter()
            .filter(|item| item.severity == "critical")
            .count() as u32,
        storage_attention_count: volumes
            .iter()
            .filter(|item| item.severity != "healthy")
            .count() as u32,
        critical_issue_count,
        attention_issue_count,
    };

    WorkspaceHealthSnapshot {
        overall_status: overall_status.to_string(),
        generated_at: now.to_rfc3339(),
        counts,
        incidents,
        sources,
        accounts,
        volumes,
    }
}

fn consecutive_failure_count<'a>(statuses: impl Iterator<Item = &'a str>) -> u32 {
    statuses.take_while(|status| *status == "failed").count() as u32
}

fn account_severity(auth_state: &str, has_session: bool, has_secret: bool) -> &'static str {
    if auth_state == "expired" || !has_session || !has_secret {
        "critical"
    } else if auth_state == "degraded" {
        "attention"
    } else {
        "healthy"
    }
}

fn source_freshness(last_synced_at: Option<&str>, now: DateTime<Utc>) -> String {
    let Some(value) = last_synced_at else {
        return "never".to_string();
    };
    let Ok(timestamp) = DateTime::parse_from_rfc3339(value) else {
        return "never".to_string();
    };
    let age = now.signed_duration_since(timestamp.with_timezone(&Utc));
    if age < Duration::hours(24) {
        "fresh"
    } else if age < Duration::days(7) {
        "stale"
    } else if age < Duration::days(30) {
        "old"
    } else {
        "ancient"
    }
    .to_string()
}

fn build_storage_health(
    snapshot: &WorkspaceSnapshot,
    incidents: &mut Vec<WorkspaceHealthIncident>,
) -> Vec<StorageVolumeHealth> {
    #[derive(Default)]
    struct RootAccumulator {
        path: String,
        source_count: u32,
        primary: bool,
    }

    let mut roots = BTreeMap::<String, RootAccumulator>::new();
    let primary_key = normalize_path_key(Path::new(&snapshot.media_root));
    roots.insert(
        primary_key,
        RootAccumulator {
            path: snapshot.media_root.clone(),
            source_count: 0,
            primary: true,
        },
    );
    for path in snapshot.source_media_paths.values() {
        let source_path = Path::new(path);
        let root_path = source_storage_root(source_path);
        let root_path = root_path.to_string_lossy().to_string();
        let key = normalize_path_key(Path::new(&root_path));
        let entry = roots.entry(key).or_insert_with(|| RootAccumulator {
            path: root_path,
            source_count: 0,
            primary: false,
        });
        entry.source_count = entry.source_count.saturating_add(1);
    }

    let mut by_volume = BTreeMap::<String, Vec<StorageRootHealth>>::new();
    for root in roots.into_values() {
        let path = PathBuf::from(&root.path);
        let accessible = storage_path_accessible(&path);
        let volume_key = volume_key(&path);
        by_volume
            .entry(volume_key)
            .or_default()
            .push(StorageRootHealth {
                path: root.path,
                source_count: root.source_count,
                primary: root.primary,
                accessible,
            });
    }

    let mut volumes = Vec::new();
    for (volume_key, mut volume_roots) in by_volume {
        volume_roots.sort_by(|left, right| {
            right
                .primary
                .cmp(&left.primary)
                .then_with(|| left.path.cmp(&right.path))
        });
        let representative = volume_roots
            .iter()
            .map(|root| PathBuf::from(&root.path))
            .find(|path| path.exists())
            .or_else(|| nearest_existing_ancestor(Path::new(&volume_roots[0].path)));
        let (total_bytes, available_bytes) = representative
            .as_ref()
            .and_then(|path| {
                let total = fs2::total_space(path).ok()?;
                let available = fs2::available_space(path).ok()?;
                Some((total, available))
            })
            .unwrap_or((0, 0));
        let available_percent = if total_bytes == 0 {
            0.0
        } else {
            available_bytes as f64 * 100.0 / total_bytes as f64
        };
        let primary_inaccessible = volume_roots
            .iter()
            .any(|root| root.primary && !root.accessible);
        let unavailable_associated_roots = volume_roots
            .iter()
            .filter(|root| !root.primary && !root.accessible)
            .collect::<Vec<_>>();
        let severity = storage_severity(
            primary_inaccessible,
            !unavailable_associated_roots.is_empty(),
            total_bytes,
            available_bytes,
        );
        if severity != "healthy" {
            let detail = if primary_inaccessible {
                "The primary media root cannot be accessed.".to_string()
            } else if !unavailable_associated_roots.is_empty() {
                format!(
                    "{} associated media destination(s) cannot be accessed. {} remains free on this volume.",
                    unavailable_associated_roots.len(),
                    format_bytes(available_bytes)
                )
            } else {
                format!(
                    "{} available of {} ({:.1}% free).",
                    format_bytes(available_bytes),
                    format_bytes(total_bytes),
                    available_percent
                )
            };
            incidents.push(WorkspaceHealthIncident {
                id: format!("storage:{volume_key}"),
                severity: severity.to_string(),
                kind: "storage".to_string(),
                title: format!("Media storage {volume_key} needs attention"),
                detail,
                source_id: None,
                account_id: None,
                volume_key: Some(volume_key.clone()),
                evidence: storage_incident_evidence(
                    &volume_roots,
                    total_bytes,
                    available_bytes,
                    available_percent,
                ),
                available_actions: vec!["open_storage_cleanup".to_string()],
            });
        }
        volumes.push(StorageVolumeHealth {
            volume_key,
            total_bytes,
            available_bytes,
            used_bytes: total_bytes.saturating_sub(available_bytes),
            available_percent,
            severity: severity.to_string(),
            roots: volume_roots,
        });
    }
    volumes.sort_by(|left, right| {
        severity_rank(&right.severity)
            .cmp(&severity_rank(&left.severity))
            .then_with(|| left.volume_key.cmp(&right.volume_key))
    });
    volumes
}

fn storage_severity(
    primary_inaccessible: bool,
    associated_inaccessible: bool,
    total_bytes: u64,
    available_bytes: u64,
) -> &'static str {
    if primary_inaccessible || total_bytes == 0 || available_bytes < CRITICAL_FREE_BYTES {
        "critical"
    } else if associated_inaccessible || available_bytes < ATTENTION_FREE_BYTES {
        "attention"
    } else {
        "healthy"
    }
}

fn source_storage_root(path: &Path) -> &Path {
    path.parent().unwrap_or(path)
}

fn storage_path_accessible(path: &Path) -> bool {
    let candidate = if path.exists() {
        Some(path.to_path_buf())
    } else {
        nearest_existing_ancestor(path)
    };
    candidate.is_some_and(|value| value.is_dir() && fs::read_dir(value).is_ok())
}

fn storage_incident_evidence(
    roots: &[StorageRootHealth],
    total_bytes: u64,
    available_bytes: u64,
    available_percent: f64,
) -> Vec<String> {
    let mut evidence = vec![format!(
        "{} free of {} ({available_percent:.1}%)",
        format_bytes(available_bytes),
        format_bytes(total_bytes)
    )];
    let unavailable = roots
        .iter()
        .filter(|root| !root.accessible)
        .collect::<Vec<_>>();
    evidence.extend(
        unavailable
            .iter()
            .take(12)
            .map(|root| format!("Unavailable: {}", root.path)),
    );
    if unavailable.len() > 12 {
        evidence.push(format!(
            "{} additional unavailable destination(s)",
            unavailable.len() - 12
        ));
    }
    evidence
}

pub(super) fn normalize_path_key(path: &Path) -> String {
    let value = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('/', "\\");
    if cfg!(windows) {
        value.trim_end_matches('\\').to_ascii_lowercase()
    } else {
        value.trim_end_matches('\\').to_string()
    }
}

pub(super) fn volume_key(path: &Path) -> String {
    let value = path.to_string_lossy().replace('/', "\\");
    let bytes = value.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        return value[..2].to_ascii_uppercase();
    }
    if value.starts_with("\\\\") {
        let parts = value
            .trim_start_matches('\\')
            .split('\\')
            .collect::<Vec<_>>();
        if parts.len() >= 2 {
            return format!("\\\\{}\\{}", parts[0], parts[1]).to_ascii_lowercase();
        }
    }
    "/".to_string()
}

fn nearest_existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        if candidate.exists() {
            return Some(candidate.to_path_buf());
        }
        current = candidate.parent();
    }
    None
}

fn severity_rank(value: &str) -> u8 {
    match value {
        "critical" => 2,
        "attention" => 1,
        _ => 0,
    }
}

fn format_bytes(value: u64) -> String {
    if value >= GIB {
        format!("{:.1} GB", value as f64 / GIB as f64)
    } else {
        format!("{:.1} MB", value as f64 / (1024 * 1024) as f64)
    }
}

#[cfg(test)]
mod health_tests {
    use super::*;

    #[test]
    fn freshness_uses_workspace_thresholds() {
        let now = Utc::now();
        assert_eq!(source_freshness(None, now), "never");
        assert_eq!(
            source_freshness(Some(&(now - Duration::hours(25)).to_rfc3339()), now),
            "stale"
        );
        assert_eq!(
            source_freshness(Some(&(now - Duration::days(8)).to_rfc3339()), now),
            "old"
        );
    }

    #[test]
    fn volume_key_groups_windows_drive_paths_case_insensitively() {
        assert_eq!(volume_key(Path::new(r"s:\NinjaCrawler\one")), "S:");
        assert_eq!(volume_key(Path::new(r"S:\NinjaCrawler\two")), "S:");
    }

    #[test]
    fn source_destinations_are_grouped_by_their_configured_parent_root() {
        assert_eq!(
            source_storage_root(Path::new(r"S:\NinjaCrawler\TikTok\profile-one")),
            Path::new(r"S:\NinjaCrawler\TikTok")
        );
        assert_eq!(
            source_storage_root(Path::new(r"S:\NinjaCrawler\TikTok\profile-two")),
            Path::new(r"S:\NinjaCrawler\TikTok")
        );
    }

    #[test]
    fn missing_profile_folder_is_reachable_when_its_parent_is_accessible() {
        let temp = tempfile::tempdir().expect("tempdir");
        assert!(storage_path_accessible(
            &temp.path().join("profile-not-created-yet")
        ));
    }

    #[test]
    fn recurring_failures_reset_after_success_or_warning() {
        assert_eq!(
            consecutive_failure_count(["failed", "failed", "failed"].into_iter()),
            3
        );
        assert_eq!(
            consecutive_failure_count(["failed", "warning", "failed"].into_iter()),
            1
        );
        assert_eq!(
            consecutive_failure_count(["succeeded", "failed", "failed"].into_iter()),
            0
        );
    }

    #[test]
    fn storage_thresholds_use_the_most_severe_condition() {
        assert_eq!(
            storage_severity(false, false, 100 * GIB, 4 * GIB),
            "critical"
        );
        assert_eq!(
            storage_severity(false, false, 100 * GIB, 19 * GIB),
            "attention"
        );
        assert_eq!(
            storage_severity(false, false, 1_863 * GIB, 236 * GIB),
            "healthy"
        );
        assert_eq!(
            storage_severity(false, false, 7_452 * GIB, 490 * GIB),
            "healthy"
        );
        assert_eq!(
            storage_severity(false, true, 500 * GIB, 100 * GIB),
            "attention"
        );
        assert_eq!(
            storage_severity(true, false, 500 * GIB, 100 * GIB),
            "critical"
        );
    }

    #[test]
    fn freshness_is_a_filter_and_does_not_degrade_source_health() {
        assert_eq!(source_severity(false, false, Some("succeeded")), "healthy");
        assert_eq!(source_severity(false, false, None), "healthy");
        assert_eq!(source_severity(false, false, Some("failed")), "attention");
    }

    #[test]
    fn source_incidents_are_coalesced_to_the_most_actionable_reason() {
        assert_eq!(
            source_incident_kind(Some("username_unresolvable"), true, Some("failed")),
            Some("source_problem")
        );
        assert_eq!(
            source_incident_kind(None, true, Some("failed")),
            Some("source_recurring_failure")
        );
        assert_eq!(
            source_incident_kind(None, false, Some("failed")),
            Some("source_latest_failure")
        );
        assert_eq!(source_incident_kind(None, false, Some("succeeded")), None);
    }

    #[test]
    fn account_sessions_distinguish_degraded_and_unusable() {
        assert_eq!(account_severity("degraded", true, true), "attention");
        assert_eq!(account_severity("expired", true, true), "critical");
        assert_eq!(account_severity("ready", false, false), "critical");
        assert_eq!(account_severity("ready", true, false), "critical");
        assert_eq!(account_severity("ready", true, true), "healthy");
    }
}
