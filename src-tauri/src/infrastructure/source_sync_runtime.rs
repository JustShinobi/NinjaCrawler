use chrono::Utc;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use crate::domain::models::{
    RunSourceSyncInput, SourceSyncOptions, SourceSyncQueueItem, SourceSyncQueueProviderStatus,
    SourceSyncQueueRecentResult, SourceSyncQueueStatus, WorkspaceSnapshot,
};
use crate::infrastructure::runtime_log::RuntimeLogAnchor;
use crate::infrastructure::{
    desktop_runtime, media_thumbnail_runtime, runtime_log, workspace_repository,
};
use crate::providers;

const SCHEDULER_TICK_EVENT: &str = "runtime://scheduler-tick";
pub const SOURCE_SYNC_QUEUE_CHANGED_EVENT: &str = "runtime://source-sync-queue-changed";
const RECENT_RESULTS_LIMIT: usize = 80;
const ACCOUNT_HOLD_CHECK_INTERVAL_SECS: u64 = 15;

fn is_force_imported_backfill(run_mode: Option<&str>) -> bool {
    run_mode.is_some_and(|value| value.eq_ignore_ascii_case("force_imported_backfill"))
}

fn is_queue_promotion_run_mode(run_mode: Option<&str>) -> bool {
    is_force_imported_backfill(run_mode)
        || run_mode.is_some_and(|value| {
            value
                .eq_ignore_ascii_case(workspace_repository::TWITTER_FULL_TIMELINE_BACKFILL_RUN_MODE)
        })
}

fn source_sync_job_key(
    source_id: &str,
    sync_options_override: Option<&SourceSyncOptions>,
) -> String {
    if let Some(story_id) = sync_options_override
        .and_then(|options| options.instagram.as_ref())
        .and_then(|options| options.target_story_media_id.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return format!("{source_id}:instagram-story:{story_id}");
    }

    if let Some(target_url) = sync_options_override
        .and_then(|options| options.tiktok.as_ref())
        .and_then(|options| options.target_video_url.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return format!("{source_id}:tiktok-story:{target_url}");
    }

    source_id.to_string()
}

#[derive(Clone)]
struct SourceSyncQueueJob {
    job_key: String,
    source_id: String,
    provider: String,
    handle: String,
    account_id: Option<String>,
    trigger: String,
    run_mode: Option<String>,
    sync_options_override: Option<SourceSyncOptions>,
    queued_at: String,
    started_at: Option<String>,
    progress_percent: Option<u32>,
    progress_label: Option<String>,
    progress_detail: Option<String>,
    progress_indeterminate: bool,
    downloaded_items: Option<u32>,
    hold_until: Option<String>,
}

enum DequeueOutcome {
    Job(SourceSyncQueueJob),
    WaitingForAccount,
    Finished,
}

#[derive(Clone)]
struct SourceSyncQueueJobResult {
    source_id: String,
    provider: String,
    handle: String,
    account_id: Option<String>,
    status: String,
    summary: String,
    finished_at: String,
}

#[derive(Default)]
struct SourceSyncQueueProviderCounters {
    completed: u32,
    failed: u32,
}

#[derive(Default)]
struct SourceSyncQueueState {
    /// Uma sub-fila por provider: cada provider tem seu próprio worker, então
    /// providers diferentes baixam em paralelo (ex.: TikTok e Instagram ao mesmo
    /// tempo), enquanto dentro de um provider segue sequencial.
    queues: HashMap<String, VecDeque<SourceSyncQueueJob>>,
    queued_keys: HashSet<String>,
    /// Job ativo por provider (no máximo um por provider).
    active_jobs: HashMap<String, SourceSyncQueueJob>,
    /// Providers com worker vivo.
    workers_running: HashSet<String>,
    /// Providers pausados: os jobs em fila não iniciam até retomar. O job que já
    /// está rodando continua até o fim.
    paused_providers: HashSet<String>,
    completed_count: u32,
    failed_count: u32,
    provider_counters: HashMap<String, SourceSyncQueueProviderCounters>,
    recent_results: VecDeque<SourceSyncQueueJobResult>,
}

impl SourceSyncQueueState {
    fn active_job_for_source_mut(&mut self, source_id: &str) -> Option<&mut SourceSyncQueueJob> {
        self.active_jobs
            .values_mut()
            .find(|job| job.source_id == source_id)
    }
}

type SharedQueueState = Arc<Mutex<SourceSyncQueueState>>;
type SharedQueueAppHandle = Arc<Mutex<Option<AppHandle>>>;

#[derive(Default)]
struct QueueEnqueueResult {
    should_spawn_worker: bool,
    queued_now: bool,
    promoted_existing_job: bool,
}

fn queue_state() -> &'static SharedQueueState {
    static STATE: OnceLock<SharedQueueState> = OnceLock::new();
    STATE.get_or_init(|| Arc::new(Mutex::new(SourceSyncQueueState::default())))
}

fn queue_app_handle() -> &'static SharedQueueAppHandle {
    static APP_HANDLE: OnceLock<SharedQueueAppHandle> = OnceLock::new();
    APP_HANDLE.get_or_init(|| Arc::new(Mutex::new(None)))
}

fn register_queue_app_handle(app: &AppHandle) {
    if let Ok(mut holder) = queue_app_handle().lock() {
        *holder = Some(app.clone());
    }
}

pub fn registered_app_handle() -> Result<AppHandle, String> {
    queue_app_handle()
        .lock()
        .map_err(|_| "Source sync queue app handle lock is poisoned.".to_string())?
        .as_ref()
        .cloned()
        .ok_or_else(|| "Source sync queue is not attached to the desktop runtime.".to_string())
}

fn publish_queue_status_event_from_registered_app() {
    let app = queue_app_handle()
        .lock()
        .ok()
        .and_then(|holder| holder.as_ref().cloned());

    if let Some(app) = app {
        publish_queue_status_event(&app);
    }
}

fn log_source_sync_event(
    scope: &str,
    level: &str,
    context: RuntimeLogAnchor<'_>,
    message: impl Into<String>,
    detail: Option<String>,
) {
    let _ = runtime_log::append_workspace(scope, level, context, message, detail);
}

fn enqueue_job(state: &mut SourceSyncQueueState, job: SourceSyncQueueJob) -> QueueEnqueueResult {
    let job_key = job.job_key.clone();
    let provider = job.provider.clone();
    let force_imported_backfill = is_force_imported_backfill(job.run_mode.as_deref());
    let promote_existing = is_queue_promotion_run_mode(job.run_mode.as_deref());
    let already_active = state
        .active_jobs
        .get(&provider)
        .is_some_and(|active| active.job_key == job_key);
    let mut promoted_existing_job = false;

    if let Some(existing_job) = state
        .queues
        .get_mut(&provider)
        .and_then(|queue| queue.iter_mut().find(|queued| queued.job_key == job_key))
    {
        if promote_existing {
            existing_job.trigger = job.trigger.clone();
            existing_job.run_mode = job.run_mode.clone();
            existing_job.sync_options_override = job.sync_options_override.clone();
            promoted_existing_job = true;
        }
    }

    let already_queued = state.queued_keys.contains(&job_key);
    let queued_now = if !already_queued && (!already_active || force_imported_backfill) {
        state
            .queues
            .entry(provider.clone())
            .or_default()
            .push_back(job);
        state.queued_keys.insert(job_key);
        true
    } else {
        false
    };

    // Um worker por provider: só pede spawn se ainda não houver worker desse
    // provider rodando.
    let should_spawn_worker = if state.workers_running.contains(&provider) {
        false
    } else {
        state.workers_running.insert(provider);
        true
    };

    QueueEnqueueResult {
        should_spawn_worker,
        queued_now,
        promoted_existing_job,
    }
}

pub fn enqueue_source_sync(
    app: &AppHandle,
    input: RunSourceSyncInput,
) -> Result<WorkspaceSnapshot, String> {
    let source_id = input.id.trim().to_string();
    if source_id.is_empty() {
        return Err("Source id is required.".to_string());
    }

    let trigger = input
        .trigger
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("manual")
        .to_string();

    register_queue_app_handle(app);
    let seed = workspace_repository::source_sync_queue_item_seed(
        source_id.clone(),
        input.sync_options_override.clone(),
    )?;
    let snapshot = workspace_repository::queue_source_sync(
        source_id.clone(),
        input.sync_options_override.clone(),
    )?;

    let job = SourceSyncQueueJob {
        job_key: source_sync_job_key(&seed.source_id, input.sync_options_override.as_ref()),
        source_id: seed.source_id.clone(),
        provider: seed.provider.clone(),
        handle: seed.handle.clone(),
        account_id: seed.account_id.clone(),
        trigger: trigger.clone(),
        run_mode: input.run_mode.clone(),
        sync_options_override: input.sync_options_override.clone(),
        queued_at: Utc::now().to_rfc3339(),
        started_at: None,
        progress_percent: None,
        progress_label: None,
        progress_detail: None,
        progress_indeterminate: false,
        downloaded_items: None,
        hold_until: None,
    };

    let queued_at = job.queued_at.clone();
    let job_key = job.job_key.clone();
    let queue_result = {
        let mut state = queue_state()
            .lock()
            .map_err(|_| "Source sync queue lock is poisoned.".to_string())?;
        enqueue_job(&mut state, job)
    };

    // Persiste o job para que a fila sobreviva ao fechamento do app; o registro
    // é removido quando o job termina ou é cancelado.
    if queue_result.queued_now || queue_result.promoted_existing_job {
        let _ = workspace_repository::persist_source_sync_queue_job(
            &workspace_repository::PersistedSourceSyncQueueJob {
                job_key,
                source_id: seed.source_id.clone(),
                trigger: trigger.clone(),
                run_mode: input.run_mode.clone(),
                sync_options_override: input.sync_options_override.clone(),
                queued_at,
            },
        );
    }

    if queue_result.queued_now {
        log_source_sync_event(
            "sync.queue",
            "info",
            RuntimeLogAnchor {
                source_id: Some(&seed.source_id),
                provider: Some(&seed.provider),
                source_handle: Some(&seed.handle),
                account_id: seed.account_id.as_deref(),
            },
            format!("Queued source sync for '{}'.", seed.handle),
            None,
        );
    }

    if queue_result.promoted_existing_job {
        let promotion_label = if input.run_mode.as_deref().is_some_and(|value| {
            value
                .eq_ignore_ascii_case(workspace_repository::TWITTER_FULL_TIMELINE_BACKFILL_RUN_MODE)
        }) {
            "dedicated Twitter full-timeline backfill"
        } else {
            "force legacy backfill"
        };
        log_source_sync_event(
            "sync.queue",
            "info",
            RuntimeLogAnchor {
                source_id: Some(&seed.source_id),
                provider: Some(&seed.provider),
                source_handle: Some(&seed.handle),
                account_id: seed.account_id.as_deref(),
            },
            format!(
                "Promoted queued source sync for '{}' to {promotion_label}.",
                seed.handle
            ),
            Some("The existing queued run was updated instead of duplicated.".to_string()),
        );
    }

    publish_queue_status_event(app);

    if queue_result.should_spawn_worker {
        spawn_worker(app.clone(), seed.provider.clone());
    }

    Ok(snapshot)
}

/// Restaura no boot os jobs de sync que ficaram pendentes quando o app foi
/// fechado (inclusive o que estava ativo, que será re-executado do zero — o
/// ledger garante que nada já baixado se repita). Jobs cujo perfil não existe
/// mais são descartados da persistência.
pub fn restore_persisted_queue(app: &AppHandle) {
    register_queue_app_handle(app);

    let jobs = match workspace_repository::load_persisted_source_sync_queue_jobs() {
        Ok(jobs) if !jobs.is_empty() => jobs,
        Ok(_) => return,
        Err(error) => {
            eprintln!("failed to load persisted source-sync queue: {error}");
            return;
        }
    };

    let mut restored = 0usize;
    let mut providers_to_spawn: HashSet<String> = HashSet::new();
    for persisted in jobs {
        let seed = match workspace_repository::source_sync_queue_item_seed(
            persisted.source_id.clone(),
            persisted.sync_options_override.clone(),
        ) {
            Ok(seed) => seed,
            Err(_) => {
                // Perfil removido/inválido desde o fechamento: descarta o job.
                let _ = workspace_repository::remove_source_sync_queue_job(&persisted.job_key);
                continue;
            }
        };

        let job = SourceSyncQueueJob {
            job_key: persisted.job_key.clone(),
            source_id: seed.source_id.clone(),
            provider: seed.provider.clone(),
            handle: seed.handle.clone(),
            account_id: seed.account_id.clone(),
            trigger: persisted.trigger.clone(),
            run_mode: persisted.run_mode.clone(),
            sync_options_override: persisted.sync_options_override.clone(),
            queued_at: persisted.queued_at.clone(),
            started_at: None,
            progress_percent: None,
            progress_label: None,
            progress_detail: None,
            progress_indeterminate: false,
            downloaded_items: None,
            hold_until: None,
        };

        let queue_result = match queue_state().lock() {
            Ok(mut state) => enqueue_job(&mut state, job),
            Err(_) => break,
        };
        if queue_result.queued_now {
            restored += 1;
            log_source_sync_event(
                "sync.queue",
                "info",
                RuntimeLogAnchor {
                    source_id: Some(&seed.source_id),
                    provider: Some(&seed.provider),
                    source_handle: Some(&seed.handle),
                    account_id: seed.account_id.as_deref(),
                },
                format!(
                    "Restored queued source sync for '{}' from the previous session.",
                    seed.handle
                ),
                None,
            );
        }
        if queue_result.should_spawn_worker {
            providers_to_spawn.insert(seed.provider.clone());
        }
    }

    if restored > 0 {
        publish_queue_status_event(app);
    }
    for provider in providers_to_spawn {
        spawn_worker(app.clone(), provider);
    }
}

fn provider_has_pending_jobs(provider: &str) -> bool {
    queue_state()
        .lock()
        .map(|state| {
            state
                .queues
                .get(provider)
                .is_some_and(|queue| !queue.is_empty())
        })
        .unwrap_or(false)
}

fn spawn_worker(app: AppHandle, provider: String) {
    thread::spawn(move || loop {
        let job = match dequeue_next(&provider) {
            Ok(DequeueOutcome::Job(job)) => job,
            Ok(DequeueOutcome::WaitingForAccount) => {
                publish_queue_status_event(&app);
                thread::sleep(Duration::from_secs(ACCOUNT_HOLD_CHECK_INTERVAL_SECS));
                continue;
            }
            Ok(DequeueOutcome::Finished) => {
                publish_queue_status_event(&app);
                break;
            }
            Err(error) => {
                eprintln!("manual source-sync worker failed to dequeue: {error}");
                publish_queue_status_event(&app);
                break;
            }
        };
        publish_queue_status_event(&app);

        let sync_result = workspace_repository::run_source_sync(
            job.source_id.clone(),
            Some(job.trigger.clone()),
            job.run_mode.clone(),
            job.sync_options_override.clone(),
        );
        if sync_result.is_ok() {
            let _ = media_thumbnail_runtime::enqueue(vec![job.source_id.clone()]);
        }
        let hold_until = job.account_id.as_deref().and_then(|account_id| {
            workspace_repository::active_source_sync_account_hold_until(account_id, &job.provider)
                .ok()
                .flatten()
        });
        if let Some(hold_until) = hold_until {
            let (_, summary) = summarize_sync_result(&job.source_id, &sync_result);
            requeue_active_on_account_hold(&job, &hold_until.to_rfc3339(), &summary);
            publish_queue_status_event(&app);
            match sync_result {
                Ok(snapshot) => emit_runtime_refresh(&app, &snapshot),
                Err(_) => {
                    if let Ok(snapshot) = workspace_repository::bootstrap_workspace() {
                        emit_runtime_refresh(&app, &snapshot);
                    }
                }
            }
            continue;
        }
        let (final_status, final_summary) = summarize_sync_result(&job.source_id, &sync_result);
        finish_active(&job, &final_status, &final_summary);
        publish_queue_status_event(&app);

        // Throttle configurável entre downloads. Cada conta/cookie tem seu
        // próprio rate limit, então o delay é por conta
        // (<provider>.account.delayBetweenDownloadsSecs) com fallback no padrão
        // global (policy.sync.delayBetweenProfilesSecs). Só dorme se ainda
        // houver job pendente DESTE provider, em passos de 1s.
        let delay_secs =
            workspace_repository::sync_delay_for_account(job.account_id.as_deref(), &job.provider);
        if delay_secs > 0 && provider_has_pending_jobs(&provider) {
            log_source_sync_event(
                "sync.queue",
                "debug",
                RuntimeLogAnchor {
                    source_id: Some(&job.source_id),
                    provider: Some(&job.provider),
                    source_handle: Some(&job.handle),
                    account_id: job.account_id.as_deref(),
                },
                "Provider cooldown is delaying the next queued sync.",
                Some(format!(
                    "Waiting {delay_secs} seconds before starting the next {} job.",
                    job.provider
                )),
            );
            for _ in 0..delay_secs {
                thread::sleep(Duration::from_secs(1));
            }
            log_source_sync_event(
                "sync.queue",
                "debug",
                RuntimeLogAnchor {
                    source_id: Some(&job.source_id),
                    provider: Some(&job.provider),
                    source_handle: Some(&job.handle),
                    account_id: job.account_id.as_deref(),
                },
                "Provider cooldown finished.",
                Some(format!("The next {} job can now start.", job.provider)),
            );
        }

        match sync_result {
            Ok(snapshot) => emit_runtime_refresh(&app, &snapshot),
            Err(error) => {
                eprintln!(
                    "manual source-sync worker failed for '{}': {error}",
                    job.source_id
                );
                if let Ok(snapshot) = workspace_repository::bootstrap_workspace() {
                    emit_runtime_refresh(&app, &snapshot);
                }
            }
        }
    });
}

fn dequeue_next(provider: &str) -> Result<DequeueOutcome, String> {
    let mut state = queue_state()
        .lock()
        .map_err(|_| "Source sync queue lock is poisoned.".to_string())?;

    // Provider pausado: para o worker sem tocar na fila (os jobs ficam à espera).
    if state.paused_providers.contains(provider) {
        state.active_jobs.remove(provider);
        state.workers_running.remove(provider);
        return Ok(DequeueOutcome::Finished);
    }

    let queue_len = state.queues.get(provider).map(VecDeque::len).unwrap_or(0);
    let mut next = None;
    for _ in 0..queue_len {
        let Some(mut candidate) = state
            .queues
            .get_mut(provider)
            .and_then(|queue| queue.pop_front())
        else {
            break;
        };
        let hold_until = candidate.account_id.as_deref().and_then(|account_id| {
            workspace_repository::active_source_sync_account_hold_until(account_id, provider)
                .ok()
                .flatten()
        });
        if let Some(until) = hold_until {
            candidate.hold_until = Some(until.to_rfc3339());
            candidate.progress_label = Some("On hold".to_string());
            candidate.progress_detail = Some(format!(
                "Twitter Account rate limit; automatic retry after {}.",
                until.to_rfc3339()
            ));
            candidate.progress_indeterminate = false;
            state
                .queues
                .entry(provider.to_string())
                .or_default()
                .push_back(candidate);
            continue;
        }
        next = Some(candidate);
        break;
    }
    match next {
        Some(mut job) => {
            state.queued_keys.remove(&job.job_key);
            job.started_at = Some(Utc::now().to_rfc3339());
            job.progress_label = Some("Starting download".to_string());
            job.progress_detail = Some("Connector runtime is preparing source sync.".to_string());
            job.progress_indeterminate = true;
            job.progress_percent = Some(0);
            job.downloaded_items = Some(0);
            job.hold_until = None;
            state.active_jobs.insert(provider.to_string(), job.clone());
            log_source_sync_event(
                "sync.run",
                "info",
                RuntimeLogAnchor {
                    source_id: Some(&job.source_id),
                    provider: Some(&job.provider),
                    source_handle: Some(&job.handle),
                    account_id: job.account_id.as_deref(),
                },
                format!("Started source sync for '{}'.", job.handle),
                job.account_id
                    .as_ref()
                    .map(|account_id| format!("Using provider account '{}'.", account_id)),
            );
            Ok(DequeueOutcome::Job(job))
        }
        None if queue_len > 0 => Ok(DequeueOutcome::WaitingForAccount),
        None => {
            state.active_jobs.remove(provider);
            state.workers_running.remove(provider);
            if let Some(queue) = state.queues.get(provider) {
                if queue.is_empty() {
                    state.queues.remove(provider);
                }
            }
            Ok(DequeueOutcome::Finished)
        }
    }
}

fn requeue_active_on_account_hold(job: &SourceSyncQueueJob, hold_until: &str, summary: &str) {
    if let Ok(mut state) = queue_state().lock() {
        state.active_jobs.remove(&job.provider);
        let mut held_job = job.clone();
        held_job.started_at = None;
        held_job.progress_percent = None;
        held_job.progress_label = Some("On hold".to_string());
        held_job.progress_detail = Some(format!(
            "Twitter Account rate limit; automatic retry after {hold_until}."
        ));
        held_job.progress_indeterminate = false;
        held_job.hold_until = Some(hold_until.to_string());
        if state.queued_keys.insert(job.job_key.clone()) {
            state
                .queues
                .entry(job.provider.clone())
                .or_default()
                .push_back(held_job);
        }
    }
    log_source_sync_event(
        "sync.queue",
        "warning",
        RuntimeLogAnchor {
            source_id: Some(&job.source_id),
            provider: Some(&job.provider),
            source_handle: Some(&job.handle),
            account_id: job.account_id.as_deref(),
        },
        format!("Source sync for '{}' is on Account hold.", job.handle),
        Some(format!("{summary} Automatic retry after {hold_until}.")),
    );
}

fn finish_active(job: &SourceSyncQueueJob, status: &str, summary: &str) {
    // O job terminou (sucesso ou falha): sai da persistência da fila.
    let _ = workspace_repository::remove_source_sync_queue_job(&job.job_key);
    if let Ok(mut state) = queue_state().lock() {
        if state
            .active_jobs
            .get(&job.provider)
            .is_some_and(|active| active.job_key == job.job_key)
        {
            state.active_jobs.remove(&job.provider);
        }

        match status {
            "failed" => {
                state.failed_count = state.failed_count.saturating_add(1);
                let provider_counters = state
                    .provider_counters
                    .entry(job.provider.clone())
                    .or_insert_with(SourceSyncQueueProviderCounters::default);
                provider_counters.failed = provider_counters.failed.saturating_add(1);
            }
            _ => {
                state.completed_count = state.completed_count.saturating_add(1);
                let provider_counters = state
                    .provider_counters
                    .entry(job.provider.clone())
                    .or_insert_with(SourceSyncQueueProviderCounters::default);
                provider_counters.completed = provider_counters.completed.saturating_add(1);
            }
        }

        state.recent_results.push_front(SourceSyncQueueJobResult {
            source_id: job.source_id.clone(),
            provider: job.provider.clone(),
            handle: job.handle.clone(),
            account_id: job.account_id.clone(),
            status: status.to_string(),
            summary: summary.to_string(),
            finished_at: Utc::now().to_rfc3339(),
        });
        while state.recent_results.len() > RECENT_RESULTS_LIMIT {
            state.recent_results.pop_back();
        }
    }

    let (level, message) = match status {
        "failed" => ("error", format!("Source sync failed for '{}'.", job.handle)),
        "cancelled" => (
            "warning",
            format!("Source sync cancelled for '{}'.", job.handle),
        ),
        _ => (
            "info",
            format!("Source sync finished for '{}'.", job.handle),
        ),
    };
    log_source_sync_event(
        "sync.run",
        level,
        RuntimeLogAnchor {
            source_id: Some(&job.source_id),
            provider: Some(&job.provider),
            source_handle: Some(&job.handle),
            account_id: job.account_id.as_deref(),
        },
        message,
        Some(summary.to_string()),
    );
}

fn summarize_sync_result(
    source_id: &str,
    sync_result: &Result<WorkspaceSnapshot, String>,
) -> (String, String) {
    match sync_result {
        Ok(snapshot) => {
            let matching_run = snapshot
                .source_sync_runs
                .iter()
                .find(|run| run.source_id == source_id);

            if let Some(run) = matching_run {
                (run.status.clone(), run.summary.clone())
            } else {
                (
                    "succeeded".to_string(),
                    "Connector sync finished successfully.".to_string(),
                )
            }
        }
        Err(error) => ("failed".to_string(), error.clone()),
    }
}

fn emit_runtime_refresh(app: &AppHandle, snapshot: &WorkspaceSnapshot) {
    let _ = desktop_runtime::publish_workspace_runtime(app, snapshot);
    let _ = app.emit(SCHEDULER_TICK_EVENT, ());
}

fn publish_queue_status_event(app: &AppHandle) {
    if let Ok(status) = source_sync_queue_status() {
        let _ = app.emit(SOURCE_SYNC_QUEUE_CHANGED_EVENT, status);
    }
}

pub fn source_sync_queue_status() -> Result<SourceSyncQueueStatus, String> {
    let state = queue_state()
        .lock()
        .map_err(|_| "Source sync queue lock is poisoned.".to_string())?;
    Ok(build_queue_status(&state))
}

pub fn report_source_sync_progress(
    source_id: &str,
    progress_percent: Option<u32>,
    progress_label: Option<String>,
    progress_detail: Option<String>,
    progress_indeterminate: bool,
    downloaded_items: Option<u32>,
) {
    let log_context = if let Ok(mut state) = queue_state().lock() {
        let Some(active_job) = state.active_job_for_source_mut(source_id) else {
            return;
        };

        let normalized_percent = progress_percent.map(|value| value.min(100));
        let changed = active_job.progress_percent != normalized_percent
            || active_job.progress_label != progress_label
            || active_job.progress_detail != progress_detail
            || active_job.progress_indeterminate != progress_indeterminate
            || active_job.downloaded_items != downloaded_items;

        if !changed {
            return;
        }

        active_job.progress_percent = normalized_percent;
        active_job.progress_label = progress_label.clone();
        active_job.progress_detail = progress_detail.clone();
        active_job.progress_indeterminate = progress_indeterminate;
        active_job.downloaded_items = downloaded_items;

        Some((
            active_job.source_id.clone(),
            active_job.provider.clone(),
            active_job.handle.clone(),
            active_job.account_id.clone(),
        ))
    } else {
        None
    };

    if let Some((source_id, provider, handle, account_id)) = log_context {
        let mut detail_parts = Vec::new();
        if let Some(detail) = progress_detail {
            detail_parts.push(detail);
        }
        if let Some(percent) = progress_percent {
            detail_parts.push(format!("Progress: {}%.", percent.min(100)));
        } else if progress_indeterminate {
            detail_parts.push("Progress: indeterminate.".to_string());
        }
        if let Some(downloaded) = downloaded_items {
            detail_parts.push(format!("Downloaded items: {downloaded}."));
        }

        log_source_sync_event(
            "sync.progress",
            "debug",
            RuntimeLogAnchor {
                source_id: Some(&source_id),
                provider: Some(&provider),
                source_handle: Some(&handle),
                account_id: account_id.as_deref(),
            },
            progress_label.unwrap_or_else(|| "Source sync progress updated.".to_string()),
            (!detail_parts.is_empty()).then(|| detail_parts.join(" ")),
        );
    }

    publish_queue_status_event_from_registered_app();
}

pub fn cancel_source_sync_profile(
    app: &AppHandle,
    source_id: String,
) -> Result<WorkspaceSnapshot, String> {
    register_queue_app_handle(app);

    let mut removed_jobs = Vec::new();
    let mut active_job_to_cancel: Option<SourceSyncQueueJob> = None;
    let mut should_request_active_cancel = false;
    {
        let mut state = queue_state()
            .lock()
            .map_err(|_| "Source sync queue lock is poisoned.".to_string())?;

        for queue in state.queues.values_mut() {
            let mut retained = VecDeque::new();
            while let Some(job) = queue.pop_front() {
                if job.source_id == source_id {
                    removed_jobs.push(job);
                } else {
                    retained.push_back(job);
                }
            }
            *queue = retained;
        }
        for job in &removed_jobs {
            state.queued_keys.remove(&job.job_key);
        }

        if let Some(active_job) = state.active_job_for_source_mut(&source_id) {
            active_job_to_cancel = Some(active_job.clone());
            should_request_active_cancel = true;
        }
    }

    for job in removed_jobs {
        let _ = workspace_repository::remove_source_sync_queue_job(&job.job_key);
        log_source_sync_event(
            "sync.queue",
            "warning",
            RuntimeLogAnchor {
                source_id: Some(&job.source_id),
                provider: Some(&job.provider),
                source_handle: Some(&job.handle),
                account_id: job.account_id.as_deref(),
            },
            format!("Cancelled queued source sync for '{}'.", job.handle),
            Some("Removed from the queue before execution.".to_string()),
        );
    }

    if should_request_active_cancel {
        if let Some(job) = active_job_to_cancel.as_ref() {
            log_source_sync_event(
                "sync.run",
                "warning",
                RuntimeLogAnchor {
                    source_id: Some(&job.source_id),
                    provider: Some(&job.provider),
                    source_handle: Some(&job.handle),
                    account_id: job.account_id.as_deref(),
                },
                format!("Cancellation requested for '{}'.", job.handle),
                Some("User requested cancellation for the active source sync.".to_string()),
            );
        }
        let _ = workspace_repository::request_source_sync_cancel(&source_id);
        report_source_sync_progress(
            &source_id,
            None,
            Some("Cancelling".to_string()),
            Some("Cancellation requested by user.".to_string()),
            true,
            None,
        );
    }

    publish_queue_status_event(app);
    workspace_repository::bootstrap_workspace()
}

pub fn cancel_source_sync_provider(
    app: &AppHandle,
    provider: String,
) -> Result<WorkspaceSnapshot, String> {
    register_queue_app_handle(app);
    let normalized_provider = provider.trim().to_ascii_lowercase();
    if normalized_provider.is_empty() {
        return Err("Provider key is required to cancel source sync jobs.".to_string());
    }

    let mut removed_jobs = Vec::new();
    let active_job_to_cancel: Option<SourceSyncQueueJob>;
    {
        let mut state = queue_state()
            .lock()
            .map_err(|_| "Source sync queue lock is poisoned.".to_string())?;

        for queue in state.queues.values_mut() {
            let mut retained = VecDeque::new();
            while let Some(job) = queue.pop_front() {
                if job.provider.eq_ignore_ascii_case(&normalized_provider) {
                    removed_jobs.push(job);
                } else {
                    retained.push_back(job);
                }
            }
            *queue = retained;
        }
        for job in &removed_jobs {
            state.queued_keys.remove(&job.job_key);
        }

        active_job_to_cancel = state
            .active_jobs
            .values()
            .find(|active| active.provider.eq_ignore_ascii_case(&normalized_provider))
            .cloned();
    }

    for job in removed_jobs {
        let _ = workspace_repository::remove_source_sync_queue_job(&job.job_key);
        log_source_sync_event(
            "sync.queue",
            "warning",
            RuntimeLogAnchor {
                source_id: Some(&job.source_id),
                provider: Some(&job.provider),
                source_handle: Some(&job.handle),
                account_id: job.account_id.as_deref(),
            },
            format!("Cancelled queued source sync for '{}'.", job.handle),
            Some("Provider cancellation removed the job before execution.".to_string()),
        );
    }

    if let Some(job) = active_job_to_cancel {
        log_source_sync_event(
            "sync.run",
            "warning",
            RuntimeLogAnchor {
                source_id: Some(&job.source_id),
                provider: Some(&job.provider),
                source_handle: Some(&job.handle),
                account_id: job.account_id.as_deref(),
            },
            format!("Cancellation requested for '{}'.", job.handle),
            Some("Provider cancellation requested stop for the active source sync.".to_string()),
        );
        let _ = workspace_repository::request_source_sync_cancel(&job.source_id);
        report_source_sync_progress(
            &job.source_id,
            None,
            Some("Cancelling".to_string()),
            Some("Provider cancellation requested by user.".to_string()),
            true,
            None,
        );
    }

    publish_queue_status_event(app);
    workspace_repository::bootstrap_workspace()
}

/// Pausa um provider: os jobs em fila param de iniciar (o que já roda termina).
pub fn pause_source_sync_provider(
    app: &AppHandle,
    provider: String,
) -> Result<WorkspaceSnapshot, String> {
    register_queue_app_handle(app);
    let normalized_provider = provider.trim().to_ascii_lowercase();
    if normalized_provider.is_empty() {
        return Err("Provider key is required to pause source sync jobs.".to_string());
    }
    if let Ok(mut state) = queue_state().lock() {
        state.paused_providers.insert(normalized_provider);
    }
    publish_queue_status_event(app);
    workspace_repository::bootstrap_workspace()
}

/// Retoma um provider pausado e religa o worker se houver jobs em fila.
pub fn resume_source_sync_provider(
    app: &AppHandle,
    provider: String,
) -> Result<WorkspaceSnapshot, String> {
    register_queue_app_handle(app);
    let normalized_provider = provider.trim().to_ascii_lowercase();
    if normalized_provider.is_empty() {
        return Err("Provider key is required to resume source sync jobs.".to_string());
    }
    let should_spawn = {
        let mut state = queue_state()
            .lock()
            .map_err(|_| "Source sync queue lock is poisoned.".to_string())?;
        state.paused_providers.remove(&normalized_provider);
        let has_pending = state
            .queues
            .get(&normalized_provider)
            .is_some_and(|queue| !queue.is_empty());
        if has_pending && !state.workers_running.contains(&normalized_provider) {
            state.workers_running.insert(normalized_provider.clone());
            true
        } else {
            false
        }
    };
    if should_spawn {
        spawn_worker(app.clone(), normalized_provider);
    }
    publish_queue_status_event(app);
    workspace_repository::bootstrap_workspace()
}

/// Reordena a fila (apenas os jobs aguardando) de um provider conforme a ordem
/// de `ordered_job_keys` vinda do drag-and-drop. Chaves não presentes na fila são
/// ignorados; jobs da fila ausentes da lista ficam ao final, na ordem original.
pub fn reorder_source_sync_provider_queue(
    app: &AppHandle,
    provider: String,
    ordered_job_keys: Vec<String>,
) -> Result<WorkspaceSnapshot, String> {
    register_queue_app_handle(app);
    let normalized_provider = provider.trim().to_ascii_lowercase();
    if normalized_provider.is_empty() {
        return Err("Provider key is required to reorder the queue.".to_string());
    }
    let persisted_order = {
        let mut state = queue_state()
            .lock()
            .map_err(|_| "Source sync queue lock is poisoned.".to_string())?;
        if let Some(queue) = state.queues.get_mut(&normalized_provider) {
            let rank: HashMap<&str, usize> = ordered_job_keys
                .iter()
                .enumerate()
                .map(|(index, id)| (id.as_str(), index))
                .collect();
            let mut jobs: Vec<SourceSyncQueueJob> = queue.drain(..).collect();
            // sort_by_key é estável: jobs fora da lista (usize::MAX) preservam a
            // ordem relativa original ao final.
            jobs.sort_by_key(|job| {
                rank.get(job.job_key.as_str())
                    .copied()
                    .unwrap_or(usize::MAX)
            });
            *queue = jobs.into_iter().collect();
        }
        // Ordem a persistir: o job ativo do provider primeiro (restaura antes),
        // depois a nova ordem da fila.
        let mut order: Vec<String> = Vec::new();
        if let Some(active) = state.active_jobs.get(&normalized_provider) {
            order.push(active.job_key.clone());
        }
        if let Some(queue) = state.queues.get(&normalized_provider) {
            order.extend(queue.iter().map(|job| job.job_key.clone()));
        }
        order
    };

    // Persiste a ordem manual para sobreviver ao restart (best-effort).
    let _ = workspace_repository::persist_source_sync_queue_order(&persisted_order);

    publish_queue_status_event(app);
    workspace_repository::bootstrap_workspace()
}

fn build_queue_status(state: &SourceSyncQueueState) -> SourceSyncQueueStatus {
    let provider_catalog = providers::provider_catalog();
    let mut provider_display_names = HashMap::new();
    let mut provider_order = Vec::new();
    for descriptor in provider_catalog {
        provider_display_names.insert(descriptor.key.clone(), descriptor.display_name.clone());
        provider_order.push(descriptor.key);
    }

    let mut queued_by_provider: HashMap<String, u32> = HashMap::new();
    for queue in state.queues.values() {
        for job in queue {
            let entry = queued_by_provider.entry(job.provider.clone()).or_default();
            *entry = entry.saturating_add(1);
        }
    }

    let mut running_by_provider: HashMap<String, u32> = HashMap::new();
    for active_job in state.active_jobs.values() {
        let entry = running_by_provider
            .entry(active_job.provider.clone())
            .or_default();
        *entry = entry.saturating_add(1);
    }

    for key in queued_by_provider.keys() {
        if !provider_display_names.contains_key(key) {
            provider_display_names.insert(key.clone(), key.clone());
            provider_order.push(key.clone());
        }
    }

    for key in running_by_provider.keys() {
        if !provider_display_names.contains_key(key) {
            provider_display_names.insert(key.clone(), key.clone());
            provider_order.push(key.clone());
        }
    }

    for key in state.provider_counters.keys() {
        if !provider_display_names.contains_key(key) {
            provider_display_names.insert(key.clone(), key.clone());
            provider_order.push(key.clone());
        }
    }

    let providers = provider_order
        .into_iter()
        .map(|provider| {
            let queued = queued_by_provider.get(&provider).copied().unwrap_or(0);
            let running = running_by_provider.get(&provider).copied().unwrap_or(0);
            let counters = state.provider_counters.get(&provider);
            let completed = counters.map(|item| item.completed).unwrap_or(0);
            let failed = counters.map(|item| item.failed).unwrap_or(0);
            let active_progress_percent = state
                .active_jobs
                .get(&provider)
                .and_then(|job| job.progress_percent.filter(|_| !job.progress_indeterminate));
            SourceSyncQueueProviderStatus {
                provider: provider.clone(),
                display_name: provider_display_names
                    .get(&provider)
                    .cloned()
                    .unwrap_or(provider.clone()),
                queued,
                running,
                completed,
                failed,
                total: queued
                    .saturating_add(running)
                    .saturating_add(completed)
                    .saturating_add(failed),
                active_progress_percent,
                paused: state.paused_providers.contains(&provider),
            }
        })
        .collect::<Vec<_>>();

    // A ordem interna de cada sub-fila é autoritativa: reflete o drag manual
    // e o restore por order_index. Reordenar aqui (ex.: por queued_at) fazia a
    // UI perder a ordem manual a cada evento da fila.
    let mut provider_keys: Vec<&String> = state.queues.keys().collect();
    provider_keys.sort();
    let mut queued_items = Vec::new();
    for provider in provider_keys {
        let Some(queue) = state.queues.get(provider) else {
            continue;
        };
        queued_items.extend(queue.iter().map(|job| SourceSyncQueueItem {
            job_key: job.job_key.clone(),
            source_id: job.source_id.clone(),
            provider: job.provider.clone(),
            handle: job.handle.clone(),
            account_id: job.account_id.clone(),
            state: if job.hold_until.is_some() {
                "held".to_string()
            } else {
                "queued".to_string()
            },
            queued_at: job.queued_at.clone(),
            started_at: None,
            progress_percent: job.progress_percent,
            progress_label: job.progress_label.clone(),
            progress_detail: job.progress_detail.clone(),
            progress_indeterminate: job.progress_indeterminate,
            downloaded_items: job.downloaded_items,
            hold_until: job.hold_until.clone(),
        }));
    }

    let mut running_items = state
        .active_jobs
        .values()
        .map(|job| SourceSyncQueueItem {
            job_key: job.job_key.clone(),
            source_id: job.source_id.clone(),
            provider: job.provider.clone(),
            handle: job.handle.clone(),
            account_id: job.account_id.clone(),
            state: "running".to_string(),
            queued_at: job.queued_at.clone(),
            started_at: job.started_at.clone(),
            progress_percent: job.progress_percent,
            progress_label: job.progress_label.clone(),
            progress_detail: job.progress_detail.clone(),
            progress_indeterminate: job.progress_indeterminate,
            downloaded_items: job.downloaded_items,
            hold_until: job.hold_until.clone(),
        })
        .collect::<Vec<_>>();
    running_items.sort_by(|a, b| a.provider.cmp(&b.provider));

    let recent_results = state
        .recent_results
        .iter()
        .map(|entry| SourceSyncQueueRecentResult {
            source_id: entry.source_id.clone(),
            provider: entry.provider.clone(),
            handle: entry.handle.clone(),
            account_id: entry.account_id.clone(),
            status: entry.status.clone(),
            summary: entry.summary.clone(),
            finished_at: entry.finished_at.clone(),
        })
        .collect::<Vec<_>>();

    let queued_count = queued_items.len() as u32;
    let running_count = running_items.len() as u32;
    let total_count = queued_count
        .saturating_add(running_count)
        .saturating_add(state.completed_count)
        .saturating_add(state.failed_count);

    // Campos `active_*` (legados, singular) representam o primeiro job em
    // execução; o detalhe completo por provider vem em `running_items`/`providers`.
    let representative_active = running_items.first();
    SourceSyncQueueStatus {
        queued_count,
        running_count,
        completed_count: state.completed_count,
        failed_count: state.failed_count,
        total_count,
        active_source_id: representative_active.map(|job| job.source_id.clone()),
        active_handle: representative_active.map(|job| job.handle.clone()),
        active_provider: representative_active.map(|job| job.provider.clone()),
        active_started_at: representative_active.and_then(|job| job.started_at.clone()),
        providers,
        queued_items,
        running_items,
        recent_results,
        updated_at: Utc::now().to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_queue_status, enqueue_job, source_sync_job_key, SourceSyncQueueJob,
        SourceSyncQueueJobResult, SourceSyncQueueState,
    };
    use crate::domain::models::{
        InstagramSourceSyncOptions, SourceSyncOptions, TikTokSourceSyncOptions,
    };

    fn sample_job(
        source_id: &str,
        provider: &str,
        handle: &str,
        queued_at: &str,
    ) -> SourceSyncQueueJob {
        SourceSyncQueueJob {
            job_key: source_id.to_string(),
            source_id: source_id.to_string(),
            provider: provider.to_string(),
            handle: handle.to_string(),
            account_id: Some("account-1".to_string()),
            trigger: "manual".to_string(),
            run_mode: None,
            sync_options_override: None,
            queued_at: queued_at.to_string(),
            started_at: None,
            progress_percent: None,
            progress_label: None,
            progress_detail: None,
            progress_indeterminate: false,
            downloaded_items: None,
            hold_until: None,
        }
    }

    #[test]
    fn queue_status_exposes_account_held_jobs_without_marking_them_running() {
        let mut state = SourceSyncQueueState::default();
        let mut job = sample_job(
            "source-held",
            "twitter",
            "held-user",
            "2026-07-11T00:00:00Z",
        );
        job.hold_until = Some("2026-07-11T00:15:30Z".to_string());
        job.progress_label = Some("On hold".to_string());
        state.queued_keys.insert(job.job_key.clone());
        state
            .queues
            .entry("twitter".to_string())
            .or_default()
            .push_back(job);

        let status = build_queue_status(&state);
        assert_eq!(status.queued_count, 1);
        assert_eq!(status.running_count, 0);
        assert_eq!(status.queued_items[0].state, "held");
        assert_eq!(
            status.queued_items[0].hold_until.as_deref(),
            Some("2026-07-11T00:15:30Z")
        );
    }

    #[test]
    fn queue_status_reports_counts_for_active_and_queued_items() {
        let mut state = SourceSyncQueueState::default();
        state
            .queues
            .entry("instagram".to_string())
            .or_default()
            .push_back(sample_job(
                "source-queued",
                "instagram",
                "@queued_handle",
                "2026-03-11T17:00:00Z",
            ));
        state.active_jobs.insert(
            "instagram".to_string(),
            SourceSyncQueueJob {
                started_at: Some("2026-03-11T17:01:00Z".to_string()),
                ..sample_job(
                    "source-running",
                    "instagram",
                    "@running_handle",
                    "2026-03-11T16:59:00Z",
                )
            },
        );
        state.completed_count = 2;
        state.failed_count = 1;
        state.recent_results.push_back(SourceSyncQueueJobResult {
            source_id: "source-finished".to_string(),
            provider: "instagram".to_string(),
            handle: "@finished".to_string(),
            account_id: Some("account-1".to_string()),
            status: "succeeded".to_string(),
            summary: "ok".to_string(),
            finished_at: "2026-03-11T16:58:00Z".to_string(),
        });

        let status = build_queue_status(&state);

        assert_eq!(status.queued_count, 1);
        assert_eq!(status.running_count, 1);
        assert_eq!(status.completed_count, 2);
        assert_eq!(status.failed_count, 1);
        assert_eq!(status.total_count, 5);
        assert_eq!(status.active_source_id.as_deref(), Some("source-running"));
        assert_eq!(status.queued_items.len(), 1);
        assert_eq!(status.running_items.len(), 1);
        assert_eq!(status.recent_results.len(), 1);
    }

    #[test]
    fn enqueue_job_promotes_existing_queued_job_to_force_backfill() {
        let mut state = SourceSyncQueueState::default();
        state
            .queues
            .entry("instagram".to_string())
            .or_default()
            .push_back(sample_job(
                "source-1",
                "instagram",
                "@queued_handle",
                "2026-03-11T17:00:00Z",
            ));
        state.queued_keys.insert("source-1".to_string());
        state.workers_running.insert("instagram".to_string());

        let result = enqueue_job(
            &mut state,
            SourceSyncQueueJob {
                trigger: "manual_force_imported_backfill".to_string(),
                run_mode: Some("force_imported_backfill".to_string()),
                ..sample_job(
                    "source-1",
                    "instagram",
                    "@queued_handle",
                    "2026-03-11T17:05:00Z",
                )
            },
        );

        assert!(!result.queued_now);
        assert!(result.promoted_existing_job);
        let queue = state.queues.get("instagram").expect("instagram queue");
        assert_eq!(queue.len(), 1);
        let queued = queue.front().expect("queued job");
        assert_eq!(queued.trigger, "manual_force_imported_backfill");
        assert_eq!(queued.run_mode.as_deref(), Some("force_imported_backfill"));
    }

    #[test]
    fn enqueue_job_allows_force_backfill_follow_up_while_source_is_active() {
        let mut state = SourceSyncQueueState::default();
        state.active_jobs.insert(
            "instagram".to_string(),
            SourceSyncQueueJob {
                started_at: Some("2026-03-11T17:01:00Z".to_string()),
                ..sample_job(
                    "source-1",
                    "instagram",
                    "@running_handle",
                    "2026-03-11T16:59:00Z",
                )
            },
        );
        state.workers_running.insert("instagram".to_string());

        let result = enqueue_job(
            &mut state,
            SourceSyncQueueJob {
                trigger: "manual_force_imported_backfill".to_string(),
                run_mode: Some("force_imported_backfill".to_string()),
                ..sample_job(
                    "source-1",
                    "instagram",
                    "@running_handle",
                    "2026-03-11T17:05:00Z",
                )
            },
        );

        assert!(result.queued_now);
        assert!(!result.promoted_existing_job);
        let queue = state.queues.get("instagram").expect("instagram queue");
        assert_eq!(queue.len(), 1);
        assert!(state.queued_keys.contains("source-1"));
        let queued = queue.front().expect("queued follow-up");
        assert_eq!(queued.run_mode.as_deref(), Some("force_imported_backfill"));
    }

    #[test]
    fn enqueue_job_allows_distinct_story_targets_for_the_same_source() {
        let mut state = SourceSyncQueueState::default();
        state.workers_running.insert("instagram".to_string());

        let mut first = sample_job(
            "source-1",
            "instagram",
            "@story_handle",
            "2026-03-11T17:00:00Z",
        );
        first.job_key = "source-1:instagram-story:111".to_string();
        let mut second = sample_job(
            "source-1",
            "instagram",
            "@story_handle",
            "2026-03-11T17:01:00Z",
        );
        second.job_key = "source-1:instagram-story:222".to_string();

        assert!(enqueue_job(&mut state, first.clone()).queued_now);
        assert!(enqueue_job(&mut state, second).queued_now);
        assert!(!enqueue_job(&mut state, first).queued_now);
        assert_eq!(
            state.queues.get("instagram").map(|queue| queue.len()),
            Some(2)
        );
    }

    #[test]
    fn story_job_keys_include_the_target_identity() {
        let instagram = SourceSyncOptions {
            instagram: Some(InstagramSourceSyncOptions {
                target_story_media_id: Some("123456".to_string()),
                ..InstagramSourceSyncOptions::default()
            }),
            ..SourceSyncOptions::default()
        };
        let tiktok = SourceSyncOptions {
            tiktok: Some(TikTokSourceSyncOptions {
                target_video_url: Some("https://www.tiktok.com/@user/video/987".to_string()),
                ..TikTokSourceSyncOptions::default()
            }),
            ..SourceSyncOptions::default()
        };

        assert_eq!(
            source_sync_job_key("source-1", Some(&instagram)),
            "source-1:instagram-story:123456"
        );
        assert_eq!(
            source_sync_job_key("source-1", Some(&tiktok)),
            "source-1:tiktok-story:https://www.tiktok.com/@user/video/987"
        );
        assert_eq!(source_sync_job_key("source-1", None), "source-1");
    }

    #[test]
    fn enqueue_runs_distinct_providers_in_parallel() {
        let mut state = SourceSyncQueueState::default();
        // Primeiro um TikTok ativo, depois enfileira um Instagram: o Instagram
        // deve pedir spawn do seu proprio worker (nao fica preso atras do TikTok).
        state.active_jobs.insert(
            "tiktok".to_string(),
            sample_job("tt-1", "tiktok", "@tt", "2026-03-11T17:00:00Z"),
        );
        state.workers_running.insert("tiktok".to_string());

        let result = enqueue_job(
            &mut state,
            sample_job("ig-1", "instagram", "@ig", "2026-03-11T17:01:00Z"),
        );

        assert!(result.queued_now);
        assert!(result.should_spawn_worker);
        assert!(state.workers_running.contains("instagram"));
        assert_eq!(state.queues.get("instagram").map(|q| q.len()), Some(1));
        // A fila do TikTok nao foi tocada.
        assert!(!state.queues.contains_key("tiktok"));
    }
}
