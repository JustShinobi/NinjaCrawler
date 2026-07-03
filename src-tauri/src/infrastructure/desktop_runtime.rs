use chrono::Utc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{Emitter, Manager, PhysicalPosition, Runtime, Window, WindowEvent};
use tauri_plugin_window_state::{StateFlags, WindowExt};

use crate::domain::models::{
    AccountsWindowIntent, DesktopRuntimeState, PlanEditorWindowIntent, RuntimeLogWindowStatus,
    SourceEditorWindowIntent, WorkspaceSnapshot,
};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchEditorIntent {
    pub source_ids: Vec<String>,
}
use crate::infrastructure::{connector_debug, runtime_log, workspace_repository};
#[cfg(windows)]
use winreg::enums::HKEY_CURRENT_USER;
#[cfg(windows)]
use winreg::RegKey;

pub const DESKTOP_STATE_CHANGED_EVENT: &str = "runtime://desktop-state-changed";
pub const WORKSPACE_SNAPSHOT_CHANGED_EVENT: &str = "runtime://workspace-snapshot-changed";
pub const FOREGROUND_ROUTE_EVENT: &str = "runtime://foreground-route";
pub const RUNTIME_LOG_WINDOW_READY_EVENT: &str = "runtime://runtime-log-window-ready";
pub const RUNTIME_LOG_WINDOW_FAILED_EVENT: &str = "runtime://runtime-log-window-failed";
pub const ACCOUNTS_WINDOW_INTENT_EVENT: &str = "runtime://accounts-window-intent";
pub const PROFILE_VIEW_WINDOW_SOURCE_EVENT: &str = "runtime://profile-view-source";
pub const SOURCE_EDITOR_WINDOW_INTENT_EVENT: &str = "runtime://source-editor-window-intent";
pub const PROFILE_EDITOR_WINDOW_INTENT_EVENT: &str = "runtime://profile-editor-window-intent";
pub const PLANS_WINDOW_INTENT_EVENT: &str = "runtime://plans-window-intent";
pub const BATCH_EDITOR_WINDOW_INTENT_EVENT: &str = "runtime://batch-editor-window-intent";

const MAIN_WINDOW_LABEL: &str = "main";
const ACCOUNTS_WINDOW_LABEL: &str = "accounts";
const PROFILE_EDITOR_WINDOW_LABEL: &str = "profile-editor";
const PLANS_WINDOW_LABEL: &str = "plans";
const RUNTIME_LOG_WINDOW_LABEL: &str = "runtime-log";
const CONNECTOR_DEBUG_WINDOW_LABEL: &str = "connector-debug";
const SCHEDULER_WINDOW_LABEL: &str = "scheduler-plans";
const SOURCE_SYNC_QUEUE_WINDOW_LABEL: &str = "source-sync-queue";
const CONNECTOR_RUNTIMES_WINDOW_LABEL: &str = "connector-runtimes";
const SINGLE_VIDEOS_WINDOW_LABEL: &str = "single-videos";
const IMPORT_WINDOW_LABEL: &str = "import";
const BATCH_EDITOR_WINDOW_LABEL: &str = "batch-editor";
const PROFILE_VIEW_WINDOW_LABEL: &str = "profile-view";
const MANAGED_STANDALONE_WINDOW_LABELS: &[&str] = &[
    RUNTIME_LOG_WINDOW_LABEL,
    CONNECTOR_DEBUG_WINDOW_LABEL,
    SCHEDULER_WINDOW_LABEL,
    SOURCE_SYNC_QUEUE_WINDOW_LABEL,
    CONNECTOR_RUNTIMES_WINDOW_LABEL,
    ACCOUNTS_WINDOW_LABEL,
    PROFILE_EDITOR_WINDOW_LABEL,
    PLANS_WINDOW_LABEL,
    IMPORT_WINDOW_LABEL,
    BATCH_EDITOR_WINDOW_LABEL,
    PROFILE_VIEW_WINDOW_LABEL,
];
#[cfg(desktop)]
const TRAY_ID: &str = "main-runtime-tray";
#[cfg(desktop)]
const TRAY_MENU_SHOW_ID: &str = "tray.show";
#[cfg(desktop)]
const TRAY_MENU_SILENT_MODE_ID: &str = "tray.silent-mode";
#[cfg(desktop)]
const TRAY_MENU_CLOSE_TO_TRAY_ID: &str = "tray.close-to-tray";
#[cfg(desktop)]
const TRAY_MENU_QUIT_ID: &str = "tray.quit";

pub struct DesktopRuntimeController {
    quitting: AtomicBool,
    last_emitted_state: Mutex<Option<DesktopRuntimeState>>,
    runtime_log_lifecycle: Mutex<RuntimeLogWindowLifecycle>,
    #[cfg(desktop)]
    tray_handles: Option<TrayMenuHandles>,
}

#[cfg(desktop)]
struct TrayMenuHandles {
    silent_mode: tauri::menu::CheckMenuItem<tauri::Wry>,
    close_to_tray: tauri::menu::CheckMenuItem<tauri::Wry>,
}

#[derive(Default)]
struct RuntimeLogWindowLifecycle {
    open_requests: u64,
    ready_signals: u64,
    last_ready_at: Option<String>,
    last_failure: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ForegroundRouteEvent {
    route: Option<String>,
    source: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeLogWindowReadyEvent {
    open_requests: u64,
    ready_signals: u64,
    reported_at: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeLogWindowFailedEvent {
    open_requests: u64,
    ready_signals: u64,
    message: String,
}

pub fn setup(app: &tauri::AppHandle) -> Result<(), String> {
    runtime_log::register_app_handle(app);
    connector_debug::register_app_handle(app);
    let snapshot = workspace_repository::bootstrap_workspace()?;
    apply_asset_scope(app)?;
    let controller = DesktopRuntimeController::new(app, &snapshot)?;
    app.manage(controller);
    publish_workspace_runtime(app, &snapshot)
}

pub fn apply_asset_scope(app: &tauri::AppHandle) -> Result<(), String> {
    let paths = workspace_repository::load_all_asset_media_paths()?;
    let scope = app.asset_protocol_scope();
    for path in &paths {
        scope
            .allow_directory(path, true)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn window_state_plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    // A janela principal é gerenciada pelo plugin (lembra tamanho/posição). As
    // janelas auxiliares também, mas restauram o estado manualmente ao abrir
    // (skip_initial_state), enquanto a principal restaura no boot.
    let mut builder = tauri_plugin_window_state::Builder::new()
        .with_filter(is_state_managed_window_label)
        .with_state_flags(managed_window_state_flags());

    for label in MANAGED_STANDALONE_WINDOW_LABELS {
        builder = builder.skip_initial_state(label);
    }

    builder.build()
}

pub fn publish_workspace_runtime(
    app: &tauri::AppHandle,
    snapshot: &WorkspaceSnapshot,
) -> Result<(), String> {
    let Some(controller) = app.try_state::<DesktopRuntimeController>() else {
        return Ok(());
    };

    controller.sync_tray_state(app, &snapshot.desktop_runtime)?;

    app.emit(WORKSPACE_SNAPSHOT_CHANGED_EVENT, snapshot.clone())
        .map_err(|error| error.to_string())?;

    if controller.should_emit_state(&snapshot.desktop_runtime)? {
        app.emit(
            DESKTOP_STATE_CHANGED_EVENT,
            snapshot.desktop_runtime.clone(),
        )
        .map_err(|error| error.to_string())?;
    }

    Ok(())
}

pub fn set_close_to_tray(
    app: &tauri::AppHandle,
    enabled: bool,
) -> Result<WorkspaceSnapshot, String> {
    let snapshot = workspace_repository::set_desktop_close_to_tray(enabled)?;
    publish_workspace_runtime(app, &snapshot)?;
    Ok(snapshot)
}

pub fn set_silent_mode(app: &tauri::AppHandle, enabled: bool) -> Result<WorkspaceSnapshot, String> {
    let snapshot = workspace_repository::set_desktop_silent_mode(enabled)?;
    publish_workspace_runtime(app, &snapshot)?;
    Ok(snapshot)
}

pub fn activate_main_window(
    app: &tauri::AppHandle,
    route: Option<String>,
    source: &str,
) -> Result<DesktopRuntimeState, String> {
    activate_main_window_with_route(app, route.as_deref(), source)?;
    workspace_repository::desktop_runtime_state()
}

pub fn hide_main_window(app: &tauri::AppHandle) -> Result<DesktopRuntimeState, String> {
    let window = main_window(app)?;
    window.hide().map_err(|error| error.to_string())?;
    workspace_repository::desktop_runtime_state()
}

pub fn system_short_date_pattern() -> Result<String, String> {
    #[cfg(windows)]
    {
        let current_user = RegKey::predef(HKEY_CURRENT_USER);
        let international = current_user
            .open_subkey("Control Panel\\International")
            .map_err(|error| error.to_string())?;
        let pattern = international
            .get_value::<String, _>("sShortDate")
            .map_err(|error| error.to_string())?;
        let normalized = pattern.trim();
        if normalized.is_empty() {
            return Err("Windows regional short date pattern is empty.".to_string());
        }
        return Ok(normalized.to_string());
    }

    #[cfg(not(windows))]
    {
        Ok("yyyy-MM-dd".to_string())
    }
}

pub fn open_runtime_log_window(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(controller) = app.try_state::<DesktopRuntimeController>() {
        controller.register_runtime_log_open_request()?;
    }

    if let Some(window) = app.get_webview_window(RUNTIME_LOG_WINDOW_LABEL) {
        window.show().map_err(|error| error.to_string())?;
        window.unminimize().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }

    // Spawn window creation on a separate thread to avoid deadlocking the
    // UI thread.  Tauri command handlers run on a thread-pool while
    // WebviewWindowBuilder::build() dispatches to the main/UI thread.
    // If we call build() synchronously from the command handler the UI
    // thread is still blocked waiting for the IPC response → deadlock.
    let app_handle = app.clone();
    std::thread::spawn(move || {
        if let Err(error) = create_runtime_log_window(&app_handle) {
            eprintln!("[runtime-log] failed to create window: {error}");
        }
    });

    Ok(())
}

pub fn open_connector_debug_window(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(CONNECTOR_DEBUG_WINDOW_LABEL) {
        window.show().map_err(|error| error.to_string())?;
        window.unminimize().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }

    let app_handle = app.clone();
    std::thread::spawn(move || {
        if let Err(error) = create_connector_debug_window(&app_handle) {
            eprintln!("[connector-debug] failed to create window: {error}");
        }
    });
    Ok(())
}

pub fn open_scheduler_window(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(SCHEDULER_WINDOW_LABEL) {
        window.show().map_err(|error| error.to_string())?;
        window.unminimize().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }

    let app_handle = app.clone();
    std::thread::spawn(move || {
        if let Err(error) = create_scheduler_window(&app_handle) {
            eprintln!("[scheduler-plans] failed to create window: {error}");
        }
    });

    Ok(())
}

pub fn open_plans_window(
    app: &tauri::AppHandle,
    intent: Option<PlanEditorWindowIntent>,
) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(PLANS_WINDOW_LABEL) {
        apply_plans_window_constraints(&window)?;
        window.show().map_err(|error| error.to_string())?;
        window.unminimize().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        if let Some(intent_payload) = intent {
            window
                .emit(PLANS_WINDOW_INTENT_EVENT, intent_payload)
                .map_err(|error| error.to_string())?;
        }
        return Ok(());
    }

    let app_handle = app.clone();
    std::thread::spawn(move || {
        if let Err(error) = create_plans_window(&app_handle, intent) {
            eprintln!("[plans] failed to create window: {error}");
        }
    });

    Ok(())
}

pub fn open_source_sync_queue_window(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(SOURCE_SYNC_QUEUE_WINDOW_LABEL) {
        window.show().map_err(|error| error.to_string())?;
        window.unminimize().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }

    let app_handle = app.clone();
    std::thread::spawn(move || {
        if let Err(error) = create_source_sync_queue_window(&app_handle) {
            eprintln!("[source-sync-queue] failed to create window: {error}");
        }
    });

    Ok(())
}

pub fn open_profile_view_window(
    app: &tauri::AppHandle,
    source_id: String,
) -> Result<(), String> {
    // Janela única reutilizada: ao reabrir com outro perfil, emite o novo
    // sourceId para a página recarregar.
    if let Some(window) = app.get_webview_window(PROFILE_VIEW_WINDOW_LABEL) {
        window.show().map_err(|error| error.to_string())?;
        window.unminimize().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        window
            .emit(PROFILE_VIEW_WINDOW_SOURCE_EVENT, source_id)
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    let app_handle = app.clone();
    std::thread::spawn(move || {
        if let Err(error) = create_profile_view_window(&app_handle, &source_id) {
            eprintln!("[profile-view] failed to create window: {error}");
        }
    });

    Ok(())
}

pub fn open_single_videos_window(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(SINGLE_VIDEOS_WINDOW_LABEL) {
        window.show().map_err(|error| error.to_string())?;
        window.unminimize().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }

    let app_handle = app.clone();
    std::thread::spawn(move || {
        if let Err(error) = create_single_videos_window(&app_handle) {
            eprintln!("[single-videos] failed to create window: {error}");
        }
    });

    Ok(())
}

pub fn open_connector_runtimes_window(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(CONNECTOR_RUNTIMES_WINDOW_LABEL) {
        window.show().map_err(|error| error.to_string())?;
        window.unminimize().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }

    let app_handle = app.clone();
    std::thread::spawn(move || {
        if let Err(error) = create_connector_runtimes_window(&app_handle) {
            eprintln!("[connector-runtimes] failed to create window: {error}");
        }
    });

    Ok(())
}

pub fn open_accounts_window(
    app: &tauri::AppHandle,
    intent: Option<AccountsWindowIntent>,
) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(ACCOUNTS_WINDOW_LABEL) {
        window.show().map_err(|error| error.to_string())?;
        window.unminimize().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        if let Some(intent_payload) = intent {
            window
                .emit(ACCOUNTS_WINDOW_INTENT_EVENT, intent_payload)
                .map_err(|error| error.to_string())?;
        }
        return Ok(());
    }

    let app_handle = app.clone();
    std::thread::spawn(move || {
        if let Err(error) = create_accounts_window(&app_handle, intent) {
            eprintln!("[accounts] failed to create window: {error}");
        }
    });

    Ok(())
}

pub fn open_import_window(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(IMPORT_WINDOW_LABEL) {
        window.show().map_err(|error| error.to_string())?;
        window.unminimize().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }

    let app_handle = app.clone();
    std::thread::spawn(move || {
        if let Err(error) = create_import_window(&app_handle) {
            eprintln!("[import] failed to create window: {error}");
        }
    });

    Ok(())
}

pub fn open_source_editor_window(
    app: &tauri::AppHandle,
    intent: Option<SourceEditorWindowIntent>,
) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(PROFILE_EDITOR_WINDOW_LABEL) {
        apply_profile_editor_window_constraints(&window)?;
        window.show().map_err(|error| error.to_string())?;
        window.unminimize().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        if let Some(intent_payload) = intent {
            window
                .emit(SOURCE_EDITOR_WINDOW_INTENT_EVENT, intent_payload.clone())
                .map_err(|error| error.to_string())?;
            window
                .emit(PROFILE_EDITOR_WINDOW_INTENT_EVENT, intent_payload)
                .map_err(|error| error.to_string())?;
        }
        return Ok(());
    }

    let app_handle = app.clone();
    std::thread::spawn(move || {
        if let Err(error) = create_profile_editor_window(&app_handle, intent) {
            eprintln!("[profile-editor] failed to create window: {error}");
        }
    });

    Ok(())
}

pub fn open_profile_editor_window(
    app: &tauri::AppHandle,
    intent: Option<SourceEditorWindowIntent>,
) -> Result<(), String> {
    open_source_editor_window(app, intent)
}

pub fn close_profile_editor_window(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(PROFILE_EDITOR_WINDOW_LABEL) {
        window.close().map_err(|error| error.to_string())?;
    }

    Ok(())
}

pub fn open_batch_editor_window(
    app: &tauri::AppHandle,
    source_ids: Vec<String>,
) -> Result<(), String> {
    let intent = BatchEditorIntent {
        source_ids: source_ids.clone(),
    };

    if let Some(window) = app.get_webview_window(BATCH_EDITOR_WINDOW_LABEL) {
        window.show().map_err(|error| error.to_string())?;
        window.unminimize().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        window
            .emit(BATCH_EDITOR_WINDOW_INTENT_EVENT, intent)
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    let app_handle = app.clone();
    std::thread::spawn(move || {
        if let Err(error) = create_batch_editor_window(&app_handle, source_ids) {
            eprintln!("[batch-editor] failed to create window: {error}");
        }
    });

    Ok(())
}

fn create_batch_editor_window(
    app: &tauri::AppHandle,
    source_ids: Vec<String>,
) -> Result<(), String> {
    let window = tauri::WebviewWindowBuilder::new(
        app,
        BATCH_EDITOR_WINDOW_LABEL,
        tauri::WebviewUrl::App(batch_editor_entrypoint(&source_ids).into()),
    )
    .title("Change Parameters")
    .inner_size(900.0, 700.0)
    .min_inner_size(700.0, 500.0)
    .resizable(true)
    .maximizable(false)
    .closable(true)
    .visible(false)
    .build()
    .map_err(|error| error.to_string())?;

    show_new_standalone_window(
        app,
        &window,
        WindowSizeSpec {
            width: 900,
            height: 700,
        },
        apply_batch_editor_window_constraints,
    )
}

fn apply_batch_editor_window_constraints(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.is_maximized().map_err(|error| error.to_string())? {
        window.unmaximize().map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn create_runtime_log_window(app: &tauri::AppHandle) -> Result<(), String> {
    let window = tauri::WebviewWindowBuilder::new(
        app,
        RUNTIME_LOG_WINDOW_LABEL,
        tauri::WebviewUrl::App(runtime_log_entrypoint().into()),
    )
    .title("Runtime Log")
    .inner_size(1100.0, 760.0)
    .min_inner_size(860.0, 540.0)
    .closable(true)
    .visible(false)
    .build()
    .map_err(|error| error.to_string())?;

    show_new_standalone_window(
        app,
        &window,
        WindowSizeSpec {
            width: 1100,
            height: 760,
        },
        |_| Ok(()),
    )
}

fn create_connector_debug_window(app: &tauri::AppHandle) -> Result<(), String> {
    let window = tauri::WebviewWindowBuilder::new(
        app,
        CONNECTOR_DEBUG_WINDOW_LABEL,
        tauri::WebviewUrl::App("connector-debug.html".into()),
    )
    .title("Realtime Connector Debugger")
    .inner_size(1280.0, 820.0)
    .min_inner_size(780.0, 520.0)
    .closable(true)
    .visible(false)
    .build()
    .map_err(|error| error.to_string())?;

    show_new_standalone_window(
        app,
        &window,
        WindowSizeSpec {
            width: 1280,
            height: 820,
        },
        |_| Ok(()),
    )
}

fn create_scheduler_window(app: &tauri::AppHandle) -> Result<(), String> {
    let window = tauri::WebviewWindowBuilder::new(
        app,
        SCHEDULER_WINDOW_LABEL,
        tauri::WebviewUrl::App(scheduler_entrypoint().into()),
    )
    .title("Scheduler")
    .inner_size(1120.0, 720.0)
    .min_inner_size(760.0, 480.0)
    .closable(true)
    .build()
    .map_err(|error| error.to_string())?;

    window.show().map_err(|error| error.to_string())?;
    window.unminimize().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())?;
    Ok(())
}

fn create_source_sync_queue_window(app: &tauri::AppHandle) -> Result<(), String> {
    let window = tauri::WebviewWindowBuilder::new(
        app,
        SOURCE_SYNC_QUEUE_WINDOW_LABEL,
        tauri::WebviewUrl::App(source_sync_queue_entrypoint().into()),
    )
    .title("Queue Status")
    .inner_size(1180.0, 780.0)
    .min_inner_size(920.0, 560.0)
    .closable(true)
    .visible(false)
    .build()
    .map_err(|error| error.to_string())?;

    show_new_standalone_window(
        app,
        &window,
        WindowSizeSpec {
            width: 1180,
            height: 780,
        },
        |_| Ok(()),
    )
}

fn create_profile_view_window(app: &tauri::AppHandle, source_id: &str) -> Result<(), String> {
    let window = tauri::WebviewWindowBuilder::new(
        app,
        PROFILE_VIEW_WINDOW_LABEL,
        tauri::WebviewUrl::App(profile_view_entrypoint(source_id).into()),
    )
    .title("Profile View")
    .inner_size(1280.0, 860.0)
    .min_inner_size(940.0, 600.0)
    .resizable(true)
    .closable(true)
    .visible(false)
    .build()
    .map_err(|error| error.to_string())?;

    show_new_standalone_window(
        app,
        &window,
        WindowSizeSpec {
            width: 1280,
            height: 860,
        },
        |_| Ok(()),
    )
}

fn create_plans_window(
    app: &tauri::AppHandle,
    intent: Option<PlanEditorWindowIntent>,
) -> Result<(), String> {
    let window = tauri::WebviewWindowBuilder::new(
        app,
        PLANS_WINDOW_LABEL,
        tauri::WebviewUrl::App(plans_entrypoint(intent.as_ref()).into()),
    )
    .title("Plans")
    .inner_size(960.0, 900.0)
    .min_inner_size(960.0, 720.0)
    .max_inner_size(960.0, 4096.0)
    .resizable(true)
    .maximizable(false)
    .closable(true)
    .visible(false)
    .build()
    .map_err(|error| error.to_string())?;

    show_new_standalone_window(
        app,
        &window,
        WindowSizeSpec {
            width: 960,
            height: 900,
        },
        apply_plans_window_constraints,
    )
}

fn create_single_videos_window(app: &tauri::AppHandle) -> Result<(), String> {
    let window = tauri::WebviewWindowBuilder::new(
        app,
        SINGLE_VIDEOS_WINDOW_LABEL,
        tauri::WebviewUrl::App(single_videos_entrypoint().into()),
    )
    .title("Single Videos")
    .inner_size(1280.0, 860.0)
    .min_inner_size(940.0, 600.0)
    .resizable(true)
    .closable(true)
    .visible(false)
    .build()
    .map_err(|error| error.to_string())?;

    show_new_standalone_window(
        app,
        &window,
        WindowSizeSpec {
            width: 1280,
            height: 860,
        },
        |_| Ok(()),
    )
}

fn create_connector_runtimes_window(app: &tauri::AppHandle) -> Result<(), String> {
    let window = tauri::WebviewWindowBuilder::new(
        app,
        CONNECTOR_RUNTIMES_WINDOW_LABEL,
        tauri::WebviewUrl::App(connector_runtimes_entrypoint().into()),
    )
    .title("Connector Runtimes")
    .inner_size(1120.0, 760.0)
    .min_inner_size(900.0, 560.0)
    .closable(true)
    .visible(false)
    .build()
    .map_err(|error| error.to_string())?;

    show_new_standalone_window(
        app,
        &window,
        WindowSizeSpec {
            width: 1120,
            height: 760,
        },
        |_| Ok(()),
    )
}

fn create_accounts_window(
    app: &tauri::AppHandle,
    intent: Option<AccountsWindowIntent>,
) -> Result<(), String> {
    let window = tauri::WebviewWindowBuilder::new(
        app,
        ACCOUNTS_WINDOW_LABEL,
        tauri::WebviewUrl::App(accounts_entrypoint(intent.as_ref()).into()),
    )
    .title("Accounts")
    .inner_size(920.0, 820.0)
    .min_inner_size(920.0, 620.0)
    .max_inner_size(920.0, 4096.0)
    .closable(true)
    .visible(false)
    .build()
    .map_err(|error| error.to_string())?;

    show_new_standalone_window(
        app,
        &window,
        WindowSizeSpec {
            width: 920,
            height: 820,
        },
        apply_accounts_window_constraints,
    )
}

fn create_profile_editor_window(
    app: &tauri::AppHandle,
    intent: Option<SourceEditorWindowIntent>,
) -> Result<(), String> {
    let window = tauri::WebviewWindowBuilder::new(
        app,
        PROFILE_EDITOR_WINDOW_LABEL,
        tauri::WebviewUrl::App(profile_editor_entrypoint(intent.as_ref()).into()),
    )
    .title("Profile editor")
    .inner_size(960.0, 900.0)
    .min_inner_size(960.0, 900.0)
    .max_inner_size(960.0, 4096.0)
    .resizable(true)
    .maximizable(false)
    .closable(true)
    .visible(false)
    .build()
    .map_err(|error| error.to_string())?;

    show_new_standalone_window(
        app,
        &window,
        WindowSizeSpec {
            width: 960,
            height: 900,
        },
        apply_profile_editor_window_constraints,
    )
}

fn apply_profile_editor_window_constraints(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.is_maximized().map_err(|error| error.to_string())? {
        window.unmaximize().map_err(|error| error.to_string())?;
    }
    let scale_factor = window.scale_factor().map_err(|error| error.to_string())?;
    let current_height = window
        .inner_size()
        .map_err(|error| error.to_string())?
        .to_logical::<f64>(scale_factor)
        .height
        .max(900.0);
    window
        .set_size(tauri::LogicalSize::new(960.0, current_height))
        .map_err(|error| error.to_string())?;
    window
        .set_min_size(Some(tauri::LogicalSize::new(960.0, 900.0)))
        .map_err(|error| error.to_string())?;
    window
        .set_max_size(Some(tauri::LogicalSize::new(960.0, 4096.0)))
        .map_err(|error| error.to_string())?;
    window
        .set_resizable(true)
        .map_err(|error| error.to_string())?;
    window
        .set_maximizable(false)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn apply_accounts_window_constraints(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.is_maximized().map_err(|error| error.to_string())? {
        window.unmaximize().map_err(|error| error.to_string())?;
    }

    let scale_factor = window.scale_factor().map_err(|error| error.to_string())?;
    let current_height = window
        .inner_size()
        .map_err(|error| error.to_string())?
        .to_logical::<f64>(scale_factor)
        .height
        .max(620.0);

    window
        .set_size(tauri::LogicalSize::new(920.0, current_height))
        .map_err(|error| error.to_string())?;
    window
        .set_min_size(Some(tauri::LogicalSize::new(920.0, 620.0)))
        .map_err(|error| error.to_string())?;
    window
        .set_max_size(Some(tauri::LogicalSize::new(920.0, 4096.0)))
        .map_err(|error| error.to_string())?;

    Ok(())
}

fn apply_plans_window_constraints(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.is_maximized().map_err(|error| error.to_string())? {
        window.unmaximize().map_err(|error| error.to_string())?;
    }

    let scale_factor = window.scale_factor().map_err(|error| error.to_string())?;
    let current_height = window
        .inner_size()
        .map_err(|error| error.to_string())?
        .to_logical::<f64>(scale_factor)
        .height
        .max(720.0);

    window
        .set_size(tauri::LogicalSize::new(960.0, current_height))
        .map_err(|error| error.to_string())?;
    window
        .set_min_size(Some(tauri::LogicalSize::new(960.0, 720.0)))
        .map_err(|error| error.to_string())?;
    window
        .set_max_size(Some(tauri::LogicalSize::new(960.0, 4096.0)))
        .map_err(|error| error.to_string())?;
    window
        .set_resizable(true)
        .map_err(|error| error.to_string())?;
    window
        .set_maximizable(false)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn create_import_window(app: &tauri::AppHandle) -> Result<(), String> {
    let window = tauri::WebviewWindowBuilder::new(
        app,
        IMPORT_WINDOW_LABEL,
        tauri::WebviewUrl::App(import_entrypoint().into()),
    )
    .title("Import")
    .inner_size(1180.0, 820.0)
    .min_inner_size(980.0, 620.0)
    .closable(true)
    .visible(false)
    .build()
    .map_err(|error| error.to_string())?;

    show_new_standalone_window(
        app,
        &window,
        WindowSizeSpec {
            width: 1180,
            height: 820,
        },
        |_| Ok(()),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WindowFrame {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WindowPosition {
    x: i32,
    y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WindowSizeSpec {
    width: i32,
    height: i32,
}

impl WindowFrame {
    fn from_parts(x: i32, y: i32, width: u32, height: u32) -> Option<Self> {
        Some(Self {
            x,
            y,
            width: i32::try_from(width).ok()?,
            height: i32::try_from(height).ok()?,
        })
    }
}

impl WindowSizeSpec {
    fn from_outer_size(size: tauri::PhysicalSize<u32>) -> Option<Self> {
        Some(Self {
            width: i32::try_from(size.width).ok()?,
            height: i32::try_from(size.height).ok()?,
        })
    }
}

fn managed_window_state_flags() -> StateFlags {
    StateFlags::POSITION | StateFlags::SIZE | StateFlags::MAXIMIZED
}

fn is_managed_standalone_window_label(label: &str) -> bool {
    MANAGED_STANDALONE_WINDOW_LABELS.contains(&label)
}

/// Janelas cujo tamanho/posição o plugin de window-state persiste: a principal
/// e as auxiliares gerenciadas.
fn is_state_managed_window_label(label: &str) -> bool {
    label == MAIN_WINDOW_LABEL || is_managed_standalone_window_label(label)
}

fn show_new_standalone_window<F>(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    fallback_size: WindowSizeSpec,
    post_restore: F,
) -> Result<(), String>
where
    F: FnOnce(&tauri::WebviewWindow) -> Result<(), String>,
{
    anchor_window_over_main(app, window, fallback_size)?;
    window
        .restore_state(managed_window_state_flags())
        .map_err(|error| error.to_string())?;
    post_restore(window)?;
    window.show().map_err(|error| error.to_string())?;
    window.unminimize().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())?;
    Ok(())
}

fn anchor_window_over_main(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    fallback_size: WindowSizeSpec,
) -> Result<(), String> {
    if let Some(position) = calculate_centered_child_position(app, window, fallback_size) {
        window
            .set_position(PhysicalPosition::new(position.x, position.y))
            .map_err(|error| error.to_string())?;
    } else {
        window.center().map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn calculate_centered_child_position(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    fallback_size: WindowSizeSpec,
) -> Option<WindowPosition> {
    let parent = main_window(app).ok()?;
    let parent_position = parent.outer_position().ok()?;
    let parent_size = parent.outer_size().ok()?;
    let parent_frame = WindowFrame::from_parts(
        parent_position.x,
        parent_position.y,
        parent_size.width,
        parent_size.height,
    )?;

    let monitor = parent
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| parent.primary_monitor().ok().flatten())?;
    let monitor_position = monitor.position();
    let monitor_size = monitor.size();
    let monitor_frame = WindowFrame::from_parts(
        monitor_position.x,
        monitor_position.y,
        monitor_size.width,
        monitor_size.height,
    )?;

    let child_size = window
        .outer_size()
        .ok()
        .and_then(WindowSizeSpec::from_outer_size)
        .unwrap_or(fallback_size);

    centered_child_position(parent_frame, child_size, monitor_frame)
}

fn centered_child_position(
    parent: WindowFrame,
    child_size: WindowSizeSpec,
    monitor: WindowFrame,
) -> Option<WindowPosition> {
    if parent.width <= 0
        || parent.height <= 0
        || child_size.width <= 0
        || child_size.height <= 0
        || monitor.width <= 0
        || monitor.height <= 0
    {
        return None;
    }

    let desired_x = parent.x + (parent.width - child_size.width) / 2;
    let desired_y = parent.y + (parent.height - child_size.height) / 2;

    Some(WindowPosition {
        x: clamp_child_axis(desired_x, child_size.width, monitor.x, monitor.width),
        y: clamp_child_axis(desired_y, child_size.height, monitor.y, monitor.height),
    })
}

fn clamp_child_axis(desired: i32, child_size: i32, monitor_origin: i32, monitor_size: i32) -> i32 {
    if child_size >= monitor_size {
        return monitor_origin;
    }

    let max_position = monitor_origin + monitor_size - child_size;
    desired.clamp(monitor_origin, max_position)
}

fn runtime_log_entrypoint() -> String {
    "runtime-log.html".to_string()
}

fn scheduler_entrypoint() -> String {
    "scheduler.html".to_string()
}

fn plans_entrypoint(intent: Option<&PlanEditorWindowIntent>) -> String {
    let mut query_parts = Vec::new();
    if let Some(intent) = intent {
        if let Some(mode) = intent.mode.as_deref() {
            let normalized_mode = match mode.trim().to_ascii_lowercase().as_str() {
                "new" => Some("new"),
                "edit" => Some("edit"),
                "clone" => Some("clone"),
                _ => None,
            };
            if let Some(mode_value) = normalized_mode {
                query_parts.push(format!("mode={}", encode_query_component(mode_value)));
            }
        }

        if let Some(plan_id) = intent.plan_id.as_deref() {
            let sanitized = plan_id.trim();
            if !sanitized.is_empty() {
                query_parts.push(format!("planId={}", encode_query_component(sanitized)));
            }
        }

        if let Some(scheduler_set_id) = intent.scheduler_set_id.as_deref() {
            let sanitized = scheduler_set_id.trim();
            if !sanitized.is_empty() {
                query_parts.push(format!(
                    "schedulerSetId={}",
                    encode_query_component(sanitized)
                ));
            }
        }
    }

    if query_parts.is_empty() {
        "plans.html".to_string()
    } else {
        format!("plans.html?{}", query_parts.join("&"))
    }
}

fn accounts_entrypoint(intent: Option<&AccountsWindowIntent>) -> String {
    let mut query_parts = Vec::new();
    if let Some(intent) = intent {
        if let Some(account_id) = intent.initial_account_id.as_deref() {
            let sanitized = account_id.trim();
            if !sanitized.is_empty() {
                query_parts.push(format!(
                    "initialAccountId={}",
                    encode_query_component(sanitized)
                ));
            }
        }

        if let Some(provider) = intent.initial_provider.as_deref() {
            let sanitized = provider.trim();
            if !sanitized.is_empty() {
                query_parts.push(format!(
                    "initialProvider={}",
                    encode_query_component(sanitized)
                ));
            }
        }

        if let Some(mode) = intent.initial_mode.as_deref() {
            let normalized_mode = match mode.trim().to_ascii_lowercase().as_str() {
                "create" => Some("create"),
                "edit" => Some("edit"),
                _ => None,
            };
            if let Some(mode_value) = normalized_mode {
                query_parts.push(format!(
                    "initialMode={}",
                    encode_query_component(mode_value)
                ));
            }
        }
    }

    if query_parts.is_empty() {
        "accounts.html".to_string()
    } else {
        format!("accounts.html?{}", query_parts.join("&"))
    }
}

fn source_sync_queue_entrypoint() -> String {
    "queue-status.html".to_string()
}

fn profile_view_entrypoint(source_id: &str) -> String {
    let sanitized = source_id.trim();
    if sanitized.is_empty() {
        "profile-view.html".to_string()
    } else {
        format!(
            "profile-view.html?sourceId={}",
            encode_query_component(sanitized)
        )
    }
}

fn connector_runtimes_entrypoint() -> String {
    "connector-runtimes.html".to_string()
}

fn single_videos_entrypoint() -> String {
    "single-videos.html".to_string()
}

fn profile_editor_entrypoint(intent: Option<&SourceEditorWindowIntent>) -> String {
    let mut query_parts = Vec::new();
    if let Some(intent) = intent {
        if let Some(source_id) = intent.source_id.as_deref() {
            let sanitized = source_id.trim();
            if !sanitized.is_empty() {
                query_parts.push(format!("sourceId={}", encode_query_component(sanitized)));
            }
        }

        if let Some(provider) = intent.preferred_provider.as_deref() {
            let sanitized = provider.trim();
            if !sanitized.is_empty() {
                query_parts.push(format!(
                    "preferredProvider={}",
                    encode_query_component(sanitized)
                ));
            }
        }

        if let Some(account_id) = intent.preferred_account_id.as_deref() {
            let sanitized = account_id.trim();
            if !sanitized.is_empty() {
                query_parts.push(format!(
                    "preferredAccountId={}",
                    encode_query_component(sanitized)
                ));
            }
        }

        if let Some(seed) = intent.seed.as_ref() {
            let provider = seed.provider.trim();
            let handle = seed.handle.trim();
            if !provider.is_empty() && !handle.is_empty() {
                query_parts.push(format!("seedProvider={}", encode_query_component(provider)));
                query_parts.push(format!("seedHandle={}", encode_query_component(handle)));
                let display_name = seed.display_name.trim();
                if !display_name.is_empty() {
                    query_parts.push(format!(
                        "seedDisplayName={}",
                        encode_query_component(display_name)
                    ));
                }
            }
        }
    }

    if query_parts.is_empty() {
        "profile-editor.html".to_string()
    } else {
        format!("profile-editor.html?{}", query_parts.join("&"))
    }
}

fn encode_query_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push_str(&format!("{byte:02X}"));
        }
    }
    encoded
}

fn import_entrypoint() -> String {
    "import.html".to_string()
}

fn batch_editor_entrypoint(source_ids: &[String]) -> String {
    if source_ids.is_empty() {
        return "batch-editor.html".to_string();
    }
    let ids_param = source_ids
        .iter()
        .map(|id| encode_query_component(id.trim()))
        .collect::<Vec<_>>()
        .join(",");
    format!("batch-editor.html?ids={}", ids_param)
}

pub fn report_runtime_log_window_ready(
    app: &tauri::AppHandle,
) -> Result<RuntimeLogWindowStatus, String> {
    let Some(controller) = app.try_state::<DesktopRuntimeController>() else {
        return runtime_log_window_status(app);
    };

    let update = controller.mark_runtime_log_ready()?;
    app.emit(
        RUNTIME_LOG_WINDOW_READY_EVENT,
        RuntimeLogWindowReadyEvent {
            open_requests: update.open_requests,
            ready_signals: update.ready_signals,
            reported_at: update
                .last_ready_at
                .clone()
                .unwrap_or_else(|| Utc::now().to_rfc3339()),
        },
    )
    .map_err(|error| error.to_string())?;

    runtime_log_window_status(app)
}

pub fn report_runtime_log_window_bootstrap_failure(
    app: &tauri::AppHandle,
    message: String,
) -> Result<RuntimeLogWindowStatus, String> {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return runtime_log_window_status(app);
    }

    let Some(controller) = app.try_state::<DesktopRuntimeController>() else {
        return runtime_log_window_status(app);
    };

    let sanitized = trimmed.chars().take(2048).collect::<String>();
    let update = controller.mark_runtime_log_failure(sanitized.clone())?;
    app.emit(
        RUNTIME_LOG_WINDOW_FAILED_EVENT,
        RuntimeLogWindowFailedEvent {
            open_requests: update.open_requests,
            ready_signals: update.ready_signals,
            message: sanitized,
        },
    )
    .map_err(|error| error.to_string())?;

    runtime_log_window_status(app)
}

pub fn runtime_log_window_status(app: &tauri::AppHandle) -> Result<RuntimeLogWindowStatus, String> {
    let Some(controller) = app.try_state::<DesktopRuntimeController>() else {
        return Ok(RuntimeLogWindowStatus {
            window_open: app.get_webview_window(RUNTIME_LOG_WINDOW_LABEL).is_some(),
            open_requests: 0,
            ready_signals: 0,
            last_ready_at: None,
            last_failure: None,
        });
    };

    let state = controller.runtime_log_snapshot()?;
    Ok(RuntimeLogWindowStatus {
        window_open: app.get_webview_window(RUNTIME_LOG_WINDOW_LABEL).is_some(),
        open_requests: state.open_requests,
        ready_signals: state.ready_signals,
        last_ready_at: state.last_ready_at.clone(),
        last_failure: state.last_failure.clone(),
    })
}

pub fn route_notification_action(
    app: &tauri::AppHandle,
    route: String,
) -> Result<DesktopRuntimeState, String> {
    activate_main_window(app, Some(route), "notification")
}

pub fn handle_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != MAIN_WINDOW_LABEL {
        return;
    }

    let WindowEvent::CloseRequested { api, .. } = event else {
        return;
    };

    let app = window.app_handle();
    let should_intercept = app
        .try_state::<DesktopRuntimeController>()
        .is_some_and(|controller| !controller.is_quitting())
        && workspace_repository::desktop_runtime_state()
            .map(|state| state.close_to_tray && state.tray_available)
            .unwrap_or(false);

    if should_intercept {
        api.prevent_close();
        let _ = window.hide();
    }
}

impl DesktopRuntimeController {
    fn new(app: &tauri::AppHandle, snapshot: &WorkspaceSnapshot) -> Result<Self, String> {
        let mut controller = Self {
            quitting: AtomicBool::new(false),
            last_emitted_state: Mutex::new(None),
            runtime_log_lifecycle: Mutex::new(RuntimeLogWindowLifecycle::default()),
            #[cfg(desktop)]
            tray_handles: None,
        };
        #[cfg(desktop)]
        {
            controller.tray_handles = create_tray(app, &snapshot.desktop_runtime)?;
        }
        Ok(controller)
    }

    fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::SeqCst)
    }

    fn should_emit_state(&self, next_state: &DesktopRuntimeState) -> Result<bool, String> {
        let mut state_guard = self
            .last_emitted_state
            .lock()
            .map_err(|_| "Desktop runtime state lock was poisoned.".to_string())?;
        if state_guard.as_ref() == Some(next_state) {
            return Ok(false);
        }

        *state_guard = Some(next_state.clone());
        Ok(true)
    }

    fn sync_tray_state(
        &self,
        app: &tauri::AppHandle,
        runtime_state: &DesktopRuntimeState,
    ) -> Result<(), String> {
        #[cfg(desktop)]
        if let Some(handles) = self.tray_handles.as_ref() {
            handles
                .silent_mode
                .set_checked(runtime_state.silent_mode)
                .map_err(|error| error.to_string())?;
            handles
                .close_to_tray
                .set_checked(runtime_state.close_to_tray)
                .map_err(|error| error.to_string())?;

            if let Some(tray) = app.tray_by_id(TRAY_ID) {
                tray.set_tooltip(Some(tray_tooltip(runtime_state)))
                    .map_err(|error| error.to_string())?;
            }
        }

        Ok(())
    }

    fn register_runtime_log_open_request(&self) -> Result<(), String> {
        let mut state = self
            .runtime_log_lifecycle
            .lock()
            .map_err(|_| "Runtime log window lifecycle lock was poisoned.".to_string())?;
        state.open_requests = state.open_requests.saturating_add(1);
        state.last_failure = None;
        Ok(())
    }

    fn mark_runtime_log_ready(&self) -> Result<RuntimeLogWindowLifecycle, String> {
        let mut state = self
            .runtime_log_lifecycle
            .lock()
            .map_err(|_| "Runtime log window lifecycle lock was poisoned.".to_string())?;
        state.ready_signals = state.ready_signals.saturating_add(1);
        state.last_ready_at = Some(Utc::now().to_rfc3339());
        state.last_failure = None;
        Ok(RuntimeLogWindowLifecycle {
            open_requests: state.open_requests,
            ready_signals: state.ready_signals,
            last_ready_at: state.last_ready_at.clone(),
            last_failure: state.last_failure.clone(),
        })
    }

    fn mark_runtime_log_failure(
        &self,
        message: String,
    ) -> Result<RuntimeLogWindowLifecycle, String> {
        let mut state = self
            .runtime_log_lifecycle
            .lock()
            .map_err(|_| "Runtime log window lifecycle lock was poisoned.".to_string())?;
        state.last_failure = Some(message);
        Ok(RuntimeLogWindowLifecycle {
            open_requests: state.open_requests,
            ready_signals: state.ready_signals,
            last_ready_at: state.last_ready_at.clone(),
            last_failure: state.last_failure.clone(),
        })
    }

    fn runtime_log_snapshot(&self) -> Result<RuntimeLogWindowLifecycle, String> {
        let state = self
            .runtime_log_lifecycle
            .lock()
            .map_err(|_| "Runtime log window lifecycle lock was poisoned.".to_string())?;
        Ok(RuntimeLogWindowLifecycle {
            open_requests: state.open_requests,
            ready_signals: state.ready_signals,
            last_ready_at: state.last_ready_at.clone(),
            last_failure: state.last_failure.clone(),
        })
    }
}

fn main_window(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, String> {
    app.get_webview_window(MAIN_WINDOW_LABEL)
        .ok_or_else(|| format!("Main window '{}' is not available.", MAIN_WINDOW_LABEL))
}

fn activate_main_window_with_route(
    app: &tauri::AppHandle,
    route: Option<&str>,
    source: &str,
) -> Result<(), String> {
    let window = main_window(app)?;
    window.show().map_err(|error| error.to_string())?;
    window.unminimize().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())?;
    app.emit(
        FOREGROUND_ROUTE_EVENT,
        ForegroundRouteEvent {
            route: route.map(str::to_string),
            source: source.to_string(),
        },
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn tray_tooltip(runtime_state: &DesktopRuntimeState) -> String {
    let mut tooltip = "NinjaCrawler".to_string();
    if runtime_state.silent_mode {
        tooltip.push_str(" (silent)");
    }
    tooltip
}

#[cfg(desktop)]
fn create_tray(
    app: &tauri::AppHandle,
    runtime_state: &DesktopRuntimeState,
) -> Result<Option<TrayMenuHandles>, String> {
    if !runtime_state.tray_available {
        return Ok(None);
    }

    let show_item = tauri::menu::MenuItem::with_id(
        app,
        TRAY_MENU_SHOW_ID,
        "Show NinjaCrawler",
        true,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;
    let silent_mode =
        tauri::menu::CheckMenuItemBuilder::with_id(TRAY_MENU_SILENT_MODE_ID, "Silent mode")
            .checked(runtime_state.silent_mode)
            .build(app)
            .map_err(|error| error.to_string())?;
    let close_to_tray =
        tauri::menu::CheckMenuItemBuilder::with_id(TRAY_MENU_CLOSE_TO_TRAY_ID, "Close to tray")
            .checked(runtime_state.close_to_tray)
            .build(app)
            .map_err(|error| error.to_string())?;
    let quit_item =
        tauri::menu::MenuItem::with_id(app, TRAY_MENU_QUIT_ID, "Quit", true, None::<&str>)
            .map_err(|error| error.to_string())?;

    let menu = tauri::menu::MenuBuilder::new(app)
        .item(&show_item)
        .separator()
        .item(&silent_mode)
        .item(&close_to_tray)
        .separator()
        .item(&quit_item)
        .build()
        .map_err(|error| error.to_string())?;

    let tray_icon = tauri::tray::TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip(tray_tooltip(runtime_state))
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                tauri::tray::TrayIconEvent::Click {
                    button: tauri::tray::MouseButton::Left,
                    button_state: tauri::tray::MouseButtonState::Up,
                    ..
                } | tauri::tray::TrayIconEvent::DoubleClick {
                    button: tauri::tray::MouseButton::Left,
                    ..
                }
            ) {
                let _ = activate_main_window_with_route(&tray.app_handle(), None, "tray");
            }
        });

    let tray_icon = if let Some(icon) = app.default_window_icon().cloned() {
        tray_icon.icon(icon)
    } else {
        tray_icon
    };

    tray_icon
        .on_menu_event(|app, event| match event.id().as_ref() {
            TRAY_MENU_SHOW_ID => {
                let _ = activate_main_window_with_route(app, None, "tray");
            }
            TRAY_MENU_SILENT_MODE_ID => {
                if let Ok(state) = workspace_repository::desktop_runtime_state() {
                    let _ = set_silent_mode(app, !state.silent_mode);
                }
            }
            TRAY_MENU_CLOSE_TO_TRAY_ID => {
                if let Ok(state) = workspace_repository::desktop_runtime_state() {
                    let _ = set_close_to_tray(app, !state.close_to_tray);
                }
            }
            TRAY_MENU_QUIT_ID => {
                if let Some(controller) = app.try_state::<DesktopRuntimeController>() {
                    controller.quitting.store(true, Ordering::SeqCst);
                }
                app.exit(0);
            }
            _ => {}
        })
        .build(app)
        .map_err(|error| error.to_string())?;

    Ok(Some(TrayMenuHandles {
        silent_mode,
        close_to_tray,
    }))
}

#[cfg(test)]
mod tests {
    use super::{
        centered_child_position, managed_window_state_flags, tray_tooltip, DesktopRuntimeState,
        WindowFrame, WindowPosition, WindowSizeSpec,
    };
    use tauri_plugin_window_state::StateFlags;

    #[test]
    fn tray_tooltip_reflects_silent_mode() {
        let active = DesktopRuntimeState {
            close_to_tray: true,
            silent_mode: false,
            tray_available: true,
        };
        let silent = DesktopRuntimeState {
            close_to_tray: true,
            silent_mode: true,
            tray_available: true,
        };

        assert_eq!(tray_tooltip(&active), "NinjaCrawler");
        assert_eq!(tray_tooltip(&silent), "NinjaCrawler (silent)");
    }

    #[test]
    fn centered_child_position_centers_over_parent() {
        let parent = WindowFrame {
            x: 140,
            y: 80,
            width: 1200,
            height: 900,
        };
        let child = WindowSizeSpec {
            width: 600,
            height: 400,
        };
        let monitor = WindowFrame {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };

        assert_eq!(
            centered_child_position(parent, child, monitor),
            Some(WindowPosition { x: 440, y: 330 })
        );
    }

    #[test]
    fn centered_child_position_clamps_to_monitor_bounds() {
        let parent = WindowFrame {
            x: 1500,
            y: 900,
            width: 700,
            height: 500,
        };
        let child = WindowSizeSpec {
            width: 900,
            height: 700,
        };
        let monitor = WindowFrame {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };

        assert_eq!(
            centered_child_position(parent, child, monitor),
            Some(WindowPosition { x: 1020, y: 380 })
        );
    }

    #[test]
    fn centered_child_position_returns_none_for_invalid_geometry() {
        let parent = WindowFrame {
            x: 120,
            y: 80,
            width: 0,
            height: 900,
        };
        let child = WindowSizeSpec {
            width: 600,
            height: 400,
        };
        let monitor = WindowFrame {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };

        assert_eq!(centered_child_position(parent, child, monitor), None);
    }

    #[test]
    fn centered_child_position_supports_negative_monitor_coordinates() {
        let parent = WindowFrame {
            x: -1510,
            y: 140,
            width: 1200,
            height: 900,
        };
        let child = WindowSizeSpec {
            width: 800,
            height: 520,
        };
        let monitor = WindowFrame {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };

        assert_eq!(
            centered_child_position(parent, child, monitor),
            Some(WindowPosition { x: -1310, y: 330 })
        );
    }

    #[test]
    fn managed_window_state_flags_exclude_visibility_and_fullscreen() {
        let flags = managed_window_state_flags();

        assert!(flags.contains(StateFlags::POSITION));
        assert!(flags.contains(StateFlags::SIZE));
        assert!(flags.contains(StateFlags::MAXIMIZED));
        assert!(!flags.contains(StateFlags::VISIBLE));
        assert!(!flags.contains(StateFlags::FULLSCREEN));
    }

    #[test]
    fn state_managed_window_label_includes_main_and_standalone() {
        assert!(super::is_state_managed_window_label("main"));
        for label in super::MANAGED_STANDALONE_WINDOW_LABELS {
            assert!(super::is_state_managed_window_label(label));
        }
        assert!(!super::is_state_managed_window_label("unknown-window"));
    }
}
