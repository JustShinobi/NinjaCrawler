use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use crate::domain::models::RunSourceSyncInput;
use crate::infrastructure::{desktop_runtime, source_sync_runtime, workspace_repository};

pub fn start(app: AppHandle) -> Result<(), String> {
    let launch_snapshot = workspace_repository::record_scheduler_launch()?;
    desktop_runtime::publish_workspace_runtime(&app, &launch_snapshot)?;

    thread::spawn(move || loop {
        match workspace_repository::process_scheduler_tick() {
            Ok((snapshot, requests)) => {
                let _ = desktop_runtime::publish_workspace_runtime(&app, &snapshot);
                // Os planos vencidos enfileiram suas fontes na fila sequencial
                // de sync; nada roda inline aqui (evita travar a UI).
                for request in requests {
                    if let Err(error) = source_sync_runtime::enqueue_source_sync(
                        &app,
                        RunSourceSyncInput {
                            id: request.source_id,
                            trigger: Some(request.trigger),
                            run_mode: None,
                            sync_options_override: None,
                        },
                    ) {
                        eprintln!("scheduler failed to enqueue source sync: {error}");
                    }
                }
                let _ = app.emit("runtime://scheduler-tick", ());
            }
            Err(error) => {
                eprintln!("scheduler runtime tick failed: {error}");
            }
        }

        thread::sleep(Duration::from_secs(5));
    });

    Ok(())
}
