use super::*;

pub fn upsert_scheduler_set(input: SchedulerSetUpsert) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        upsert_scheduler_set_with_connection(connection, input)?;
        load_snapshot(connection, layout)
    })
}
pub fn delete_scheduler_set(id: String) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        connection
            .execute("DELETE FROM scheduler_sets WHERE id = ?1", params![id])
            .map_err(|error| error.to_string())?;
        load_snapshot(connection, layout)
    })
}
pub fn upsert_scheduler_group(input: SchedulerGroupUpsert) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        upsert_scheduler_group_with_connection(connection, input)?;
        load_snapshot(connection, layout)
    })
}
pub fn delete_scheduler_group(id: String) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        connection
            .execute("DELETE FROM scheduler_groups WHERE id = ?1", params![id])
            .map_err(|error| error.to_string())?;
        load_snapshot(connection, layout)
    })
}
pub fn upsert_sync_plan(input: SyncPlanUpsert) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        upsert_sync_plan_with_connection(connection, input)?;
        load_snapshot(connection, layout)
    })
}
pub fn preview_sync_plan_target(
    input: SyncPlanTargetPreviewInput,
) -> Result<SyncPlanTargetPreview, String> {
    with_workspace(|connection, _layout| {
        preview_sync_plan_target_with_connection(connection, input)
    })
}
pub fn delete_sync_plan(id: String) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        connection
            .execute("DELETE FROM sync_plans WHERE id = ?1", params![id])
            .map_err(|error| error.to_string())?;
        load_snapshot(connection, layout)
    })
}
pub fn run_sync_plan_now(
    input: RunSyncPlanNowInput,
) -> Result<(WorkspaceSnapshot, Vec<PlanSyncEnqueueRequest>), String> {
    let trigger = if input.force.unwrap_or(false) {
        "manual_force"
    } else {
        "manual"
    };
    with_workspace(|connection, layout| {
        let source_ids = run_sync_plan_now_with_connection(
            connection,
            layout,
            &input.id,
            trigger,
            &now_timestamp(),
        )?;
        let snapshot = load_snapshot(connection, layout)?;
        let requests = source_ids
            .into_iter()
            .map(|source_id| PlanSyncEnqueueRequest {
                source_id,
                trigger: trigger.to_string(),
            })
            .collect();
        Ok((snapshot, requests))
    })
}
pub fn pause_sync_plan(id: String) -> Result<WorkspaceSnapshot, String> {
    set_sync_plan_pause(SetSyncPlanPauseInput {
        id,
        pause_mode: "unlimited".to_string(),
        pause_until: None,
    })
}
pub fn resume_sync_plan(id: String) -> Result<WorkspaceSnapshot, String> {
    clear_sync_plan_pause(id)
}
pub fn skip_sync_plan(id: String) -> Result<WorkspaceSnapshot, String> {
    skip_sync_plan_with_input(SkipSyncPlanInput {
        id,
        mode: "default".to_string(),
        minutes: None,
        until: None,
    })
}
pub fn set_sync_plan_pause(input: SetSyncPlanPauseInput) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        set_sync_plan_pause_with_connection(connection, &input, &now_timestamp())?;
        load_snapshot(connection, layout)
    })
}
pub fn clear_sync_plan_pause(id: String) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        clear_sync_plan_pause_with_connection(connection, &id, &now_timestamp())?;
        load_snapshot(connection, layout)
    })
}
pub fn skip_sync_plan_with_input(input: SkipSyncPlanInput) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        skip_sync_plan_with_connection(connection, &input, &now_timestamp())?;
        load_snapshot(connection, layout)
    })
}
pub fn move_sync_plan(input: MoveSyncPlanInput) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        move_sync_plan_with_connection(connection, &input)?;
        load_snapshot(connection, layout)
    })
}
pub fn clone_sync_plan(input: CloneSyncPlanInput) -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        clone_sync_plan_with_connection(connection, &input)?;
        load_snapshot(connection, layout)
    })
}
pub fn process_scheduler_tick(
) -> Result<(Option<WorkspaceSnapshot>, Vec<PlanSyncEnqueueRequest>), String> {
    with_workspace(|connection, layout| {
        let (requests, changed) =
            process_scheduler_tick_with_connection(connection, layout, &now_timestamp())?;
        // Só remonta o snapshot quando o tick mexeu em algo — em ociosidade
        // nada muda e o `load_snapshot` (caro: canonicalize por perfil) é pulado.
        let snapshot = if changed {
            Some(load_snapshot(connection, layout)?)
        } else {
            None
        };
        Ok((snapshot, requests))
    })
}
pub fn record_scheduler_launch() -> Result<WorkspaceSnapshot, String> {
    with_workspace(|connection, layout| {
        record_scheduler_launch_with_connection(connection, &now_timestamp())?;
        load_snapshot(connection, layout)
    })
}
pub(super) fn scheduler_notifications_for_mode(mode: &str) -> SchedulerPlanNotifications {
    match mode {
        "detailed" => SchedulerPlanNotifications {
            enabled: true,
            simple: false,
            show_image: true,
            show_user_icon: true,
        },
        "summary" => SchedulerPlanNotifications {
            enabled: true,
            simple: true,
            show_image: false,
            show_user_icon: false,
        },
        _ => SchedulerPlanNotifications::default(),
    }
}
pub(super) fn scheduler_notification_mode_for_struct(value: &SchedulerPlanNotifications) -> String {
    if !value.enabled || value.simple {
        "summary".to_string()
    } else {
        "detailed".to_string()
    }
}
pub(super) fn normalize_scheduler_criteria(
    mut criteria: SchedulerPlanCriteria,
    target_filter: &str,
) -> SchedulerPlanCriteria {
    if criteria.sites_included.is_empty() {
        criteria.sites_included = Vec::new();
    }
    if criteria.sites_excluded.is_empty() {
        criteria.sites_excluded = Vec::new();
    }
    if criteria
        .advanced_expression
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
        && !target_filter.trim().is_empty()
    {
        criteria.advanced_expression = Some(target_filter.trim().to_string());
    }
    criteria
}
pub(super) fn parse_scheduler_notifications(
    value: &str,
    notification_mode: &str,
) -> SchedulerPlanNotifications {
    serde_json::from_str(value)
        .unwrap_or_else(|_| scheduler_notifications_for_mode(notification_mode))
}
pub(super) fn parse_scheduler_criteria(value: &str, target_filter: &str) -> SchedulerPlanCriteria {
    let parsed = serde_json::from_str(value).unwrap_or_default();
    normalize_scheduler_criteria(parsed, target_filter)
}
pub(super) fn serialize_scheduler_notifications(
    value: &SchedulerPlanNotifications,
) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| error.to_string())
}
pub(super) fn serialize_scheduler_criteria(
    value: &SchedulerPlanCriteria,
) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| error.to_string())
}
#[allow(dead_code)]
pub(super) fn load_scheduler_group_by_id(
    connection: &Connection,
    group_id: &str,
) -> Result<Option<SchedulerGroup>, String> {
    connection
        .query_row(
            "SELECT id, name, sort_index, criteria_json
             FROM scheduler_groups
             WHERE id = ?1
             LIMIT 1",
            params![group_id],
            |row| {
                let criteria_json = row.get::<_, String>(3)?;
                Ok(SchedulerGroup {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    sort_index: row.get(2)?,
                    criteria: serde_json::from_str(&criteria_json).unwrap_or_default(),
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())
}
pub(super) fn upsert_scheduler_set_with_connection(
    connection: &Connection,
    input: SchedulerSetUpsert,
) -> Result<(), String> {
    let id = input.id.unwrap_or_else(new_id);
    let now = now_timestamp();

    if input.active {
        connection
            .execute(
                "UPDATE scheduler_sets SET is_active = 0, updated_at = ?1",
                params![now.clone()],
            )
            .map_err(|error| error.to_string())?;
    }

    connection
        .execute(
            "INSERT INTO scheduler_sets (id, name, is_active, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               is_active = excluded.is_active,
               updated_at = excluded.updated_at",
            params![id, input.name, bool_to_int(input.active), now],
        )
        .map_err(|error| error.to_string())?;

    Ok(())
}
pub(super) fn upsert_scheduler_group_with_connection(
    connection: &Connection,
    input: SchedulerGroupUpsert,
) -> Result<(), String> {
    let id = input.id.unwrap_or_else(new_id);
    let now = now_timestamp();
    let criteria_json = serialize_scheduler_criteria(&input.criteria)?;
    connection
        .execute(
            "INSERT INTO scheduler_groups (id, name, sort_index, criteria_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               sort_index = excluded.sort_index,
               criteria_json = excluded.criteria_json,
               updated_at = excluded.updated_at",
            params![
                id,
                input.name.trim(),
                input.sort_index.unwrap_or(0),
                criteria_json,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}
pub(super) fn upsert_sync_plan_with_connection(
    connection: &Connection,
    input: SyncPlanUpsert,
) -> Result<(), String> {
    let id = input.id.unwrap_or_else(new_id);
    let now = now_timestamp();
    let notifications = if input.notifications == SchedulerPlanNotifications::default() {
        scheduler_notifications_for_mode(&input.notification_mode)
    } else {
        input.notifications.clone()
    };
    let criteria = normalize_scheduler_criteria(input.criteria.clone(), &input.target_filter);
    let notifications_json = serialize_scheduler_notifications(&notifications)?;
    let criteria_json = serialize_scheduler_criteria(&criteria)?;
    connection
        .execute(
            "INSERT INTO sync_plans (
                id,
                scheduler_set_id,
                name,
                enabled,
                mode,
                interval_minutes,
                startup_delay_minutes,
                notification_mode,
                target_filter,
                sort_index,
                pause_mode,
                pause_until,
                notifications_json,
                criteria_json,
                created_at,
                updated_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)
             ON CONFLICT(id) DO UPDATE SET
               scheduler_set_id = excluded.scheduler_set_id,
               name = excluded.name,
               enabled = excluded.enabled,
               mode = excluded.mode,
               interval_minutes = excluded.interval_minutes,
               startup_delay_minutes = excluded.startup_delay_minutes,
               notification_mode = excluded.notification_mode,
               target_filter = excluded.target_filter,
               sort_index = excluded.sort_index,
               pause_mode = excluded.pause_mode,
               pause_until = excluded.pause_until,
               notifications_json = excluded.notifications_json,
               criteria_json = excluded.criteria_json,
               updated_at = excluded.updated_at",
            params![
                id,
                input.scheduler_set_id,
                input.name,
                bool_to_int(input.enabled),
                input.mode,
                i64::from(input.interval_minutes),
                i64::from(input.startup_delay_minutes),
                scheduler_notification_mode_for_struct(&notifications),
                criteria
                    .advanced_expression
                    .clone()
                    .unwrap_or_else(|| input.target_filter.clone()),
                input.sort_index.unwrap_or(0),
                input.pause_mode.unwrap_or_else(|| "disabled".to_string()),
                input.pause_until,
                notifications_json,
                criteria_json,
                now
            ],
        )
        .map_err(|error| error.to_string())?;

    Ok(())
}
pub(super) fn record_scheduler_launch_with_connection(
    connection: &Connection,
    launched_at: &str,
) -> Result<(), String> {
    upsert_app_setting_value(connection, "runtime.scheduler.launch_at", launched_at)
}
/// Devolve `(requests, changed)`. `changed` indica se o tick alterou algo no
/// banco (um plano rodou, ou o estado de runtime de algum plano mudou) — o
/// chamador só remonta/publica o snapshot quando `changed`, evitando refazer
/// `load_snapshot` (com `canonicalize()` por perfil) a cada 5s em ociosidade.
pub(super) fn process_scheduler_tick_with_connection(
    connection: &Connection,
    layout: &StorageLayout,
    now: &str,
) -> Result<(Vec<PlanSyncEnqueueRequest>, bool), String> {
    let launch_at = ensure_scheduler_launch_at(connection, now)?;
    let plans = load_active_automatic_sync_plans(connection)?;

    let mut requests = Vec::new();
    let mut changed = false;
    for plan in plans {
        let next_due_at = compute_sync_plan_next_due_at(&plan, &launch_at, now)?;
        if let Some(next_due_at_value) = next_due_at.as_deref() {
            if is_timestamp_due(next_due_at_value, now)? {
                // Um plano venceu: roda (grava run + last_run_at) → estado mudou.
                changed = true;
                let source_ids = run_sync_plan_now_with_connection(
                    connection,
                    layout,
                    &plan.id,
                    "scheduler",
                    now,
                )?;
                requests.extend(
                    source_ids
                        .into_iter()
                        .map(|source_id| PlanSyncEnqueueRequest {
                            source_id,
                            trigger: "scheduler".to_string(),
                        }),
                );
            } else {
                // O else-branch só pode alterar next_due_at e paused; o UPDATE
                // é no-op quando ambos coincidem (ver update_sync_plan_runtime_state).
                let will_change = next_due_at.as_deref() != plan.next_due_at.as_deref()
                    || is_sync_plan_paused(&plan, now) != plan.paused;
                changed = changed || will_change;
                update_sync_plan_runtime_state(
                    connection,
                    &plan.id,
                    SyncPlanRuntimeStatePatch {
                        next_due_at: next_due_at.as_deref(),
                        pause_mode: Some(plan.pause_mode.clone()),
                        pause_until: plan.pause_until.as_deref(),
                        paused: Some(is_sync_plan_paused(&plan, now)),
                        ..Default::default()
                    },
                )?;
            }
        }
    }

    Ok((requests, changed))
}
/// Resolve as fontes do plano e devolve os ids a enfileirar — NÃO executa o
/// sync aqui. Rodar os downloads inline (gallery-dl + reqwest + sleeps de rate
/// limit) segurava a conexão/lock do workspace por todo o lote, congelando o
/// app. Os downloads passam pela fila sequencial (source_sync_runtime), que já
/// respeita o delay por conta; o registro do plano apenas marca que N fontes
/// foram enfileiradas.
pub(super) fn run_sync_plan_now_with_connection(
    connection: &Connection,
    _layout: &StorageLayout,
    plan_id: &str,
    trigger: &str,
    now: &str,
) -> Result<Vec<String>, String> {
    let plan = load_sync_plan(connection, plan_id)?
        .ok_or_else(|| format!("Sync plan '{}' does not exist.", plan_id))?;
    let sources = resolve_sync_plan_sources(connection, &plan)?;
    let started_at = now.to_string();
    let finished_at = now.to_string();
    let source_count = sources.len() as u32;
    let source_ids: Vec<String> = sources.iter().map(|source| source.id.clone()).collect();

    let (status, summary) = if source_count == 0 {
        (
            "skipped".to_string(),
            "No eligible sources matched this plan.".to_string(),
        )
    } else {
        (
            "succeeded".to_string(),
            format!("Queued {} source syncs.", source_count),
        )
    };

    let next_due_at =
        if plan.mode == "automatic" && plan.enabled && !is_sync_plan_paused(&plan, now) {
            compute_sync_plan_next_due_at(
                &SyncPlan {
                    last_run_at: Some(finished_at.clone()),
                    skip_until: None,
                    ..plan.clone()
                },
                &ensure_scheduler_launch_at(connection, now)?,
                now,
            )?
        } else {
            None
        };

    update_sync_plan_runtime_state(
        connection,
        &plan.id,
        SyncPlanRuntimeStatePatch {
            last_run_at: Some(&finished_at),
            last_run_status: Some(&status),
            last_run_summary: Some(&summary),
            next_due_at: next_due_at.as_deref(),
            pause_mode: Some(plan.pause_mode.clone()),
            pause_until: plan.pause_until.as_deref(),
            paused: Some(false),
            ..Default::default()
        },
    )?;

    persist_sync_plan_run(
        connection,
        &plan,
        SyncPlanRunRecord {
            trigger,
            status: &status,
            summary: &summary,
            source_count,
            started_at: &started_at,
            finished_at: &finished_at,
        },
    )?;

    Ok(source_ids)
}
pub(super) fn set_sync_plan_pause_with_connection(
    connection: &Connection,
    input: &SetSyncPlanPauseInput,
    now: &str,
) -> Result<(), String> {
    let plan = load_sync_plan(connection, &input.id)?
        .ok_or_else(|| format!("Sync plan '{}' does not exist.", input.id))?;
    let pause_until = resolve_pause_until(now, &input.pause_mode, input.pause_until.as_deref())?;
    let launch_at = ensure_scheduler_launch_at(connection, now)?;
    let paused_plan = SyncPlan {
        pause_mode: input.pause_mode.clone(),
        pause_until: pause_until.clone(),
        paused: input.pause_mode != "disabled",
        ..plan.clone()
    };
    update_sync_plan_runtime_state(
        connection,
        &input.id,
        SyncPlanRuntimeStatePatch {
            last_run_status: Some("idle"),
            last_run_summary: Some("Plan paused."),
            next_due_at: compute_sync_plan_next_due_at(&paused_plan, &launch_at, now)?.as_deref(),
            pause_mode: Some(input.pause_mode.clone()),
            pause_until: pause_until.as_deref(),
            paused: Some(true),
            ..Default::default()
        },
    )?;
    Ok(())
}
pub(super) fn clear_sync_plan_pause_with_connection(
    connection: &Connection,
    plan_id: &str,
    now: &str,
) -> Result<(), String> {
    let plan = load_sync_plan(connection, plan_id)?
        .ok_or_else(|| format!("Sync plan '{}' does not exist.", plan_id))?;
    let launch_at = ensure_scheduler_launch_at(connection, now)?;
    let next_due_at = compute_sync_plan_next_due_at(
        &SyncPlan {
            pause_mode: "disabled".to_string(),
            pause_until: None,
            paused: false,
            ..plan.clone()
        },
        &launch_at,
        now,
    )?;
    update_sync_plan_runtime_state(
        connection,
        &plan.id,
        SyncPlanRuntimeStatePatch {
            last_run_status: Some("idle"),
            last_run_summary: Some("Plan resumed."),
            next_due_at: next_due_at.as_deref(),
            pause_mode: Some("disabled".to_string()),
            paused: Some(false),
            ..Default::default()
        },
    )?;
    Ok(())
}
pub(super) fn skip_sync_plan_with_connection(
    connection: &Connection,
    input: &SkipSyncPlanInput,
    now: &str,
) -> Result<(), String> {
    let plan = load_sync_plan(connection, &input.id)?
        .ok_or_else(|| format!("Sync plan '{}' does not exist.", input.id))?;
    let launch_at = ensure_scheduler_launch_at(connection, now)?;
    let current_due =
        compute_sync_plan_next_due_at(&plan, &launch_at, now)?.unwrap_or_else(|| now.to_string());
    let skip_until = match input.mode.as_str() {
        "reset" => None,
        "minutes" => Some(add_minutes_to_timestamp(
            now,
            i64::from(input.minutes.unwrap_or(0).max(1)),
        )?),
        "until" => input.until.clone(),
        _ => Some(add_minutes_to_timestamp(
            &current_due,
            i64::from(plan.interval_minutes.max(1)),
        )?),
    };
    let summary = match input.mode.as_str() {
        "reset" => "Cleared pending skip.".to_string(),
        "minutes" => format!(
            "Skipped automatic execution for {} minutes.",
            input.minutes.unwrap_or(0).max(1)
        ),
        "until" => "Skipped automatic execution until the chosen time.".to_string(),
        _ => "Skipped the next scheduled execution.".to_string(),
    };
    let next_due = if input.mode == "reset" {
        compute_sync_plan_next_due_at(&plan, &launch_at, now)?
    } else {
        skip_until.clone()
    };
    update_sync_plan_runtime_state(
        connection,
        &plan.id,
        SyncPlanRuntimeStatePatch {
            last_run_status: Some(if input.mode == "reset" {
                "idle"
            } else {
                "skipped"
            }),
            last_run_summary: Some(&summary),
            skip_until: skip_until.as_deref(),
            next_due_at: next_due.as_deref(),
            pause_mode: Some(plan.pause_mode.clone()),
            pause_until: plan.pause_until.as_deref(),
            paused: Some(plan.paused),
            ..Default::default()
        },
    )?;
    Ok(())
}
pub(super) fn move_sync_plan_with_connection(
    connection: &Connection,
    input: &MoveSyncPlanInput,
) -> Result<(), String> {
    let plan = load_sync_plan(connection, &input.id)?
        .ok_or_else(|| format!("Sync plan '{}' does not exist.", input.id))?;
    let sibling_plans = load_sync_plans(connection, &plan.scheduler_set_id)?;
    let Some(index) = sibling_plans.iter().position(|entry| entry.id == input.id) else {
        return Ok(());
    };
    let swap_index = match input.direction.as_str() {
        "up" if index > 0 => Some(index - 1),
        "down" if index + 1 < sibling_plans.len() => Some(index + 1),
        _ => None,
    };
    let Some(target_index) = swap_index else {
        return Ok(());
    };
    let target = &sibling_plans[target_index];
    let now = now_timestamp();
    connection
        .execute(
            "UPDATE sync_plans SET sort_index = ?2, updated_at = ?3 WHERE id = ?1",
            params![&plan.id, target.sort_index, &now],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "UPDATE sync_plans SET sort_index = ?2, updated_at = ?3 WHERE id = ?1",
            params![&target.id, plan.sort_index, &now],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}
pub(super) fn clone_sync_plan_with_connection(
    connection: &Connection,
    input: &CloneSyncPlanInput,
) -> Result<(), String> {
    let plan = load_sync_plan(connection, &input.id)?
        .ok_or_else(|| format!("Sync plan '{}' does not exist.", input.id))?;
    let max_sort_index = load_sync_plans(connection, &plan.scheduler_set_id)?
        .into_iter()
        .map(|entry| entry.sort_index)
        .max()
        .unwrap_or(0);
    upsert_sync_plan_with_connection(
        connection,
        SyncPlanUpsert {
            id: None,
            scheduler_set_id: plan.scheduler_set_id,
            name: format!("{} Copy", plan.name),
            enabled: plan.enabled,
            mode: plan.mode,
            interval_minutes: plan.interval_minutes,
            startup_delay_minutes: plan.startup_delay_minutes,
            notification_mode: plan.notification_mode,
            target_filter: plan.target_filter,
            sort_index: Some(max_sort_index + 1),
            pause_mode: Some("disabled".to_string()),
            pause_until: None,
            notifications: plan.notifications,
            criteria: plan.criteria,
        },
    )
}
pub(super) fn ensure_scheduler_launch_at(
    connection: &Connection,
    fallback_now: &str,
) -> Result<String, String> {
    let existing = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1 LIMIT 1",
            params!["runtime.scheduler.launch_at"],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;

    if let Some(value) = existing {
        return Ok(value);
    }

    record_scheduler_launch_with_connection(connection, fallback_now)?;
    Ok(fallback_now.to_string())
}
pub(super) fn load_active_automatic_sync_plans(
    connection: &Connection,
) -> Result<Vec<SyncPlan>, String> {
    let mut statement = connection
        .prepare(
            "SELECT
                p.id,
                p.scheduler_set_id,
                p.name,
                p.enabled,
                p.mode,
                p.interval_minutes,
                p.startup_delay_minutes,
                p.notification_mode,
                p.target_filter,
                p.sort_index,
                p.paused,
                p.pause_mode,
                p.pause_until,
                p.skip_until,
                p.last_run_at,
                p.last_run_status,
                p.last_run_summary,
                p.next_due_at,
                p.notifications_json,
                p.criteria_json
             FROM sync_plans p
             INNER JOIN scheduler_sets s ON s.id = p.scheduler_set_id
             WHERE s.is_active = 1 AND p.enabled = 1 AND p.mode = 'automatic'
             ORDER BY p.sort_index, p.name",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], map_sync_plan_row)
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}
pub(super) fn load_sync_plan(
    connection: &Connection,
    plan_id: &str,
) -> Result<Option<SyncPlan>, String> {
    connection
        .query_row(
            "SELECT
                id,
                scheduler_set_id,
                name,
                enabled,
                mode,
                interval_minutes,
                startup_delay_minutes,
                notification_mode,
                target_filter,
                sort_index,
                paused,
                pause_mode,
                pause_until,
                skip_until,
                last_run_at,
                last_run_status,
                last_run_summary,
                next_due_at,
                notifications_json,
                criteria_json
             FROM sync_plans
             WHERE id = ?1
             LIMIT 1",
            params![plan_id],
            map_sync_plan_row,
        )
        .optional()
        .map_err(|error| error.to_string())
}
pub(super) fn resolve_sync_plan_sources(
    connection: &Connection,
    plan: &SyncPlan,
) -> Result<Vec<SourceProfile>, String> {
    resolve_sources_for_criteria(connection, &plan.criteria)
}
pub(super) fn preview_sync_plan_target_with_connection(
    connection: &Connection,
    input: SyncPlanTargetPreviewInput,
) -> Result<SyncPlanTargetPreview, String> {
    let criteria = input.criteria;
    let sources = resolve_sources_for_criteria(connection, &criteria)?;
    Ok(SyncPlanTargetPreview {
        source_count: sources.len() as u32,
        sources: sources
            .into_iter()
            .take(120)
            .map(|source| SyncPlanTargetPreviewSource {
                id: source.id,
                handle: source.handle,
                provider: source.provider,
                labels: source.labels,
                ready_for_download: source.ready_for_download,
                remote_state: source.remote_state,
                subscription: source.is_subscription,
                last_synced_at: source.last_synced_at,
            })
            .collect(),
    })
}
pub(super) fn resolve_sources_for_criteria(
    connection: &Connection,
    criteria: &SchedulerPlanCriteria,
) -> Result<Vec<SourceProfile>, String> {
    let sources = load_sources(connection)?;

    // Os grupos do scheduler funcionam como filtro de pertencimento (membership
    // estática via `source_profiles.group_id`), e NÃO como uma criteria salva
    // que é reavaliada. Eles são interseccionados com os demais filtros do
    // plano (provider, labels, etc.). Incluir um ou mais grupos restringe o
    // resultado às fontes que pertencem a pelo menos um deles; excluir grupos
    // remove suas fontes.
    let included_groups: HashSet<&str> = criteria
        .group_ids_included
        .iter()
        .map(String::as_str)
        .collect();
    let excluded_groups: HashSet<&str> = criteria
        .group_ids_excluded
        .iter()
        .map(String::as_str)
        .collect();

    let mut resolved = sources
        .into_iter()
        .filter(|source| source_matches_scheduler_criteria(source, criteria))
        .filter(|source| {
            if included_groups.is_empty() {
                return true;
            }
            source
                .group_id
                .as_deref()
                .map(|group_id| included_groups.contains(group_id))
                .unwrap_or(false)
        })
        .filter(|source| {
            if excluded_groups.is_empty() {
                return true;
            }
            !source
                .group_id
                .as_deref()
                .map(|group_id| excluded_groups.contains(group_id))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();

    resolved.sort_by(|left, right| left.handle.cmp(&right.handle));
    if let Some(limit) = criteria.users_count {
        resolved.truncate(limit as usize);
    }
    Ok(resolved)
}
pub(super) fn split_filter_clauses(expression: &str) -> Vec<String> {
    let mut clauses = Vec::new();
    let mut current = Vec::new();

    for token in expression.split_whitespace() {
        if token.eq_ignore_ascii_case("AND") {
            if !current.is_empty() {
                clauses.push(current.join(" "));
                current.clear();
            }
        } else {
            current.push(token.to_string());
        }
    }

    if !current.is_empty() {
        clauses.push(current.join(" "));
    }

    clauses
}
pub(super) fn source_matches_scheduler_criteria(
    source: &SourceProfile,
    criteria: &SchedulerPlanCriteria,
) -> bool {
    if !criteria.ignore_ready_for_download
        && criteria.ready_for_download
        && !source.ready_for_download
    {
        return false;
    }

    let selected_categories = [
        (criteria.regular, "regular"),
        (criteria.temporary, "temporary"),
        (criteria.favorite, "favorite"),
    ];
    if selected_categories.iter().any(|(enabled, _)| *enabled)
        && !selected_categories
            .iter()
            .any(|(enabled, category)| *enabled && source_profile_category(source) == *category)
    {
        return false;
    }

    if criteria.download_users == criteria.download_subscriptions {
    } else if criteria.download_subscriptions {
        if !source.is_subscription {
            return false;
        }
    } else if source.is_subscription {
        return false;
    }

    let selected_states = [
        (criteria.user_exists, "exists"),
        (criteria.user_suspended, "suspended"),
        (criteria.user_deleted, "deleted"),
    ];
    if selected_states.iter().any(|(enabled, _)| *enabled)
        && !selected_states
            .iter()
            .any(|(enabled, state)| *enabled && source.remote_state.eq_ignore_ascii_case(state))
    {
        return false;
    }

    if criteria.labels_no && !source.labels.is_empty() {
        return false;
    }

    if !criteria.labels_included.is_empty()
        && !criteria.labels_included.iter().all(|label| {
            source
                .labels
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(label))
        })
    {
        return false;
    }

    if !criteria.ignore_excluded_labels
        && criteria.labels_excluded.iter().any(|label| {
            source
                .labels
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(label))
        })
    {
        return false;
    }

    if !criteria.sites_included.is_empty()
        && !criteria
            .sites_included
            .iter()
            .any(|site| source.provider.eq_ignore_ascii_case(site))
    {
        return false;
    }

    if criteria
        .sites_excluded
        .iter()
        .any(|site| source.provider.eq_ignore_ascii_case(site))
    {
        return false;
    }

    if let Some(days_number) = criteria.days_number {
        let cutoff = Utc::now() - Duration::days(i64::from(days_number));
        let is_downloaded_recently = source
            .last_synced_at
            .as_deref()
            .and_then(|last_synced_at| parse_timestamp(last_synced_at).ok())
            .map(|timestamp| timestamp >= cutoff)
            .unwrap_or(false);
        if is_downloaded_recently != criteria.days_is_downloaded {
            return false;
        }
    }

    if criteria.date_from.is_some() || criteria.date_to.is_some() {
        let Some(last_synced_at) = source.last_synced_at.as_deref() else {
            return false;
        };
        let parsed_date = parse_timestamp(last_synced_at)
            .map(|value| value.date_naive())
            .ok();
        let Some(parsed_date) = parsed_date else {
            return false;
        };
        let in_range = criteria
            .date_from
            .as_deref()
            .and_then(parse_date_input)
            .map(|date_from| parsed_date >= date_from)
            .unwrap_or(true)
            && criteria
                .date_to
                .as_deref()
                .and_then(parse_date_input)
                .map(|date_to| parsed_date <= date_to)
                .unwrap_or(true);
        if in_range != criteria.date_in_range {
            return false;
        }
    }

    if let Some(expression) = criteria.advanced_expression.as_deref() {
        let expression = expression.trim();
        if !expression.is_empty()
            && !split_filter_clauses(expression)
                .into_iter()
                .all(|clause| source_matches_clause(source, &clause))
        {
            return false;
        }
    }

    true
}
pub(super) fn source_matches_clause(source: &SourceProfile, clause: &str) -> bool {
    let Some((field, raw_value)) = clause.split_once('=') else {
        return false;
    };

    let field = field.trim().to_ascii_lowercase();
    let value = raw_value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_ascii_lowercase();

    match field.as_str() {
        "provider" => source.provider.eq_ignore_ascii_case(&value),
        "label" => source
            .labels
            .iter()
            .any(|label| label.eq_ignore_ascii_case(&value)),
        "ready" | "ready_for_download" => {
            let desired = matches!(value.as_str(), "true" | "1" | "yes");
            source.ready_for_download == desired
        }
        "handle" | "source" => source.handle.eq_ignore_ascii_case(&value),
        "account" | "account_id" => source
            .account_id
            .as_deref()
            .is_some_and(|account_id| account_id.eq_ignore_ascii_case(&value)),
        "kind" | "source_kind" => source.source_kind.eq_ignore_ascii_case(&value),
        "state" | "remote_state" => source.remote_state.eq_ignore_ascii_case(&value),
        "subscription" | "is_subscription" => {
            let desired = matches!(value.as_str(), "true" | "1" | "yes");
            source.is_subscription == desired
        }
        _ => false,
    }
}
pub(super) fn is_sync_plan_paused(plan: &SyncPlan, now: &str) -> bool {
    if plan.pause_mode == "disabled" {
        return false;
    }
    if plan.pause_mode == "until" {
        return plan
            .pause_until
            .as_deref()
            .map(|until| !is_timestamp_due(until, now).unwrap_or(false))
            .unwrap_or(false);
    }
    true
}
pub(super) fn resolve_pause_until(
    now: &str,
    pause_mode: &str,
    explicit_until: Option<&str>,
) -> Result<Option<String>, String> {
    let duration_minutes = match pause_mode {
        "1h" => Some(60),
        "2h" => Some(120),
        "3h" => Some(180),
        "4h" => Some(240),
        "6h" => Some(360),
        "12h" => Some(720),
        "until" => None,
        _ => None,
    };
    if let Some(minutes) = duration_minutes {
        return Ok(Some(add_minutes_to_timestamp(now, minutes)?));
    }
    Ok(explicit_until.map(str::to_string))
}
pub(super) fn compute_sync_plan_next_due_at(
    plan: &SyncPlan,
    launch_at: &str,
    now: &str,
) -> Result<Option<String>, String> {
    if !plan.enabled || plan.mode != "automatic" {
        return Ok(None);
    }

    if is_sync_plan_paused(plan, now) {
        if plan.pause_mode == "until" {
            return Ok(plan.pause_until.clone());
        }
        return Ok(None);
    }

    let startup_due_at =
        add_minutes_to_timestamp(launch_at, i64::from(plan.startup_delay_minutes))?;

    let interval_due_at = if let Some(last_run_at) = plan.last_run_at.as_deref() {
        Some(add_minutes_to_timestamp(
            last_run_at,
            i64::from(plan.interval_minutes),
        )?)
    } else {
        None
    };

    let mut candidates = vec![startup_due_at];
    if let Some(interval_due_at_value) = interval_due_at {
        candidates.push(interval_due_at_value);
    }
    if let Some(skip_until) = plan.skip_until.as_deref() {
        candidates.push(skip_until.to_string());
    }

    let next_due = latest_timestamp(candidates)?;
    Ok(Some(next_due))
}
/// Patch parcial do estado de runtime de um plano: campos `None` preservam o
/// valor persistido atual.
#[derive(Default)]
pub(super) struct SyncPlanRuntimeStatePatch<'a> {
    last_run_at: Option<&'a str>,
    last_run_status: Option<&'a str>,
    last_run_summary: Option<&'a str>,
    skip_until: Option<&'a str>,
    next_due_at: Option<&'a str>,
    pause_mode: Option<String>,
    pause_until: Option<&'a str>,
    paused: Option<bool>,
}
pub(super) fn update_sync_plan_runtime_state(
    connection: &Connection,
    plan_id: &str,
    patch: SyncPlanRuntimeStatePatch<'_>,
) -> Result<(), String> {
    let current = load_sync_plan(connection, plan_id)?
        .ok_or_else(|| format!("Sync plan '{}' does not exist.", plan_id))?;

    let next_last_run_at = patch.last_run_at.or(current.last_run_at.as_deref());
    let next_last_run_status = patch.last_run_status.unwrap_or(&current.last_run_status);
    let next_last_run_summary = patch
        .last_run_summary
        .or(current.last_run_summary.as_deref());
    let next_skip_until = patch.skip_until.or(current.skip_until.as_deref());
    let next_due_at = patch.next_due_at.or(current.next_due_at.as_deref());
    let next_pause_mode = patch.pause_mode.unwrap_or_else(|| current.pause_mode.clone());
    let next_pause_until = patch.pause_until.or(current.pause_until.as_deref());
    let next_paused = patch.paused.unwrap_or(current.paused);

    // O tick do scheduler chama isto para cada plano a cada 5s; sem este
    // guard, cada tick gravava WAL mesmo quando nada mudou.
    let unchanged = next_last_run_at == current.last_run_at.as_deref()
        && next_last_run_status == current.last_run_status
        && next_last_run_summary == current.last_run_summary.as_deref()
        && next_skip_until == current.skip_until.as_deref()
        && next_due_at == current.next_due_at.as_deref()
        && next_pause_mode == current.pause_mode
        && next_pause_until == current.pause_until.as_deref()
        && next_paused == current.paused;
    if unchanged {
        return Ok(());
    }

    let now = now_timestamp();
    connection
        .execute(
            "UPDATE sync_plans
             SET last_run_at = ?2,
                 last_run_status = ?3,
                 last_run_summary = ?4,
                 skip_until = ?5,
                 next_due_at = ?6,
                 pause_mode = ?7,
                 pause_until = ?8,
                 paused = ?9,
                 updated_at = ?10
             WHERE id = ?1",
            params![
                plan_id,
                next_last_run_at,
                next_last_run_status,
                next_last_run_summary,
                next_skip_until,
                next_due_at,
                next_pause_mode,
                next_pause_until,
                bool_to_int(next_paused),
                now,
            ],
        )
        .map_err(|error| error.to_string())?;

    Ok(())
}
/// Dados de uma execução concluída de plano, gravados em `sync_plan_runs`.
pub(super) struct SyncPlanRunRecord<'a> {
    trigger: &'a str,
    status: &'a str,
    summary: &'a str,
    source_count: u32,
    started_at: &'a str,
    finished_at: &'a str,
}
pub(super) fn persist_sync_plan_run(
    connection: &Connection,
    plan: &SyncPlan,
    run: SyncPlanRunRecord<'_>,
) -> Result<SyncPlanRun, String> {
    let SyncPlanRunRecord {
        trigger,
        status,
        summary,
        source_count,
        started_at,
        finished_at,
    } = run;
    let id = new_id();
    connection
        .execute(
            "INSERT INTO sync_plan_runs (
                id,
                plan_id,
                scheduler_set_id,
                trigger,
                status,
                summary,
                source_count,
                started_at,
                finished_at,
                created_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![
                &id,
                &plan.id,
                &plan.scheduler_set_id,
                trigger,
                status,
                summary,
                i64::from(source_count),
                started_at,
                finished_at,
            ],
        )
        .map_err(|error| error.to_string())?;

    Ok(SyncPlanRun {
        id,
        plan_id: plan.id.clone(),
        scheduler_set_id: plan.scheduler_set_id.clone(),
        trigger: trigger.to_string(),
        status: status.to_string(),
        summary: summary.to_string(),
        source_count,
        started_at: started_at.to_string(),
        finished_at: finished_at.to_string(),
    })
}
pub(super) fn load_scheduler_sets(connection: &Connection) -> Result<Vec<SchedulerSet>, String> {
    let mut set_statement = connection
        .prepare("SELECT id, name, is_active FROM scheduler_sets ORDER BY is_active DESC, name")
        .map_err(|error| error.to_string())?;
    let set_rows = set_statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? == 1,
            ))
        })
        .map_err(|error| error.to_string())?;

    let mut scheduler_sets = Vec::new();
    for row in set_rows {
        let (id, name, active) = row.map_err(|error| error.to_string())?;
        scheduler_sets.push(SchedulerSet {
            id: id.clone(),
            name,
            active,
            plans: load_sync_plans(connection, &id)?,
        });
    }
    Ok(scheduler_sets)
}
pub(super) fn load_sync_plans(
    connection: &Connection,
    scheduler_set_id: &str,
) -> Result<Vec<SyncPlan>, String> {
    let mut statement = connection.prepare("SELECT id, scheduler_set_id, name, enabled, mode, interval_minutes, startup_delay_minutes, notification_mode, target_filter, sort_index, paused, pause_mode, pause_until, skip_until, last_run_at, last_run_status, last_run_summary, next_due_at, notifications_json, criteria_json FROM sync_plans WHERE scheduler_set_id = ?1 ORDER BY sort_index, name").map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![scheduler_set_id], map_sync_plan_row)
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}
pub(super) fn map_sync_plan_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SyncPlan> {
    let notification_mode = row.get::<_, String>(7)?;
    let target_filter = row.get::<_, String>(8)?;
    let notifications_json = row.get::<_, String>(18)?;
    let criteria_json = row.get::<_, String>(19)?;
    Ok(SyncPlan {
        id: row.get(0)?,
        scheduler_set_id: row.get(1)?,
        name: row.get(2)?,
        enabled: row.get::<_, i64>(3)? == 1,
        mode: row.get(4)?,
        interval_minutes: row.get::<_, i64>(5)? as u32,
        startup_delay_minutes: row.get::<_, i64>(6)? as u32,
        notification_mode: notification_mode.clone(),
        target_filter: target_filter.clone(),
        sort_index: row.get::<_, i64>(9).unwrap_or(0),
        paused: row.get::<_, i64>(10)? == 1,
        pause_mode: row
            .get::<_, String>(11)
            .unwrap_or_else(|_| "disabled".to_string()),
        pause_until: row.get(12).ok(),
        skip_until: row.get(13)?,
        last_run_at: row.get(14)?,
        last_run_status: row.get(15)?,
        last_run_summary: row.get(16)?,
        next_due_at: row.get(17)?,
        notifications: parse_scheduler_notifications(&notifications_json, &notification_mode),
        criteria: parse_scheduler_criteria(&criteria_json, &target_filter),
    })
}
pub(super) fn load_scheduler_groups(
    connection: &Connection,
) -> Result<Vec<SchedulerGroup>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id, name, sort_index, criteria_json
             FROM scheduler_groups
             ORDER BY sort_index, name",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let criteria_json = row.get::<_, String>(3)?;
            Ok(SchedulerGroup {
                id: row.get(0)?,
                name: row.get(1)?,
                sort_index: row.get(2)?,
                criteria: serde_json::from_str(&criteria_json).unwrap_or_default(),
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}
pub(super) fn load_sync_plan_runs(connection: &Connection) -> Result<Vec<SyncPlanRun>, String> {
    let mut statement = connection
        .prepare(
            "SELECT
                id,
                plan_id,
                scheduler_set_id,
                trigger,
                status,
                summary,
                source_count,
                started_at,
                finished_at
             FROM (
                SELECT
                    r.*,
                    ROW_NUMBER() OVER (
                        PARTITION BY plan_id
                        ORDER BY finished_at DESC, created_at DESC
                    ) AS recency_rank
                FROM sync_plan_runs r
             )
             WHERE recency_rank <= ?1
             ORDER BY finished_at DESC, created_at DESC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![SYNC_RUN_HISTORY_CAP_PER_ENTITY], |row| {
            Ok(SyncPlanRun {
                id: row.get(0)?,
                plan_id: row.get(1)?,
                scheduler_set_id: row.get(2)?,
                trigger: row.get(3)?,
                status: row.get(4)?,
                summary: row.get(5)?,
                source_count: row.get::<_, i64>(6)? as u32,
                started_at: row.get(7)?,
                finished_at: row.get(8)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}
