use std::{env, thread, time::Duration};

pub mod application;
pub mod domain;
pub mod infrastructure;
pub mod providers;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(infrastructure::desktop_runtime::window_state_plugin())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            infrastructure::desktop_runtime::setup(app.handle())
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
            infrastructure::scheduler_runtime::start(app.handle().clone())
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
            infrastructure::companion_api::start(app.handle().clone());

            // Restaura jobs de sync que ficaram na fila quando o app foi
            // fechado. Fora do caminho crítico do boot (thread + atraso curto)
            // para não competir com o bootstrap do workspace.
            {
                let app_handle = app.handle().clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(2000));
                    infrastructure::media_path_migration_runtime::restore_persisted_queue(
                        &app_handle,
                    );
                    infrastructure::source_sync_runtime::restore_persisted_queue(&app_handle);
                });
            }

            if env::var_os("NINJACRAWLER_DEBUG_OPEN_RUNTIME_LOG").is_some() {
                let app_handle = app.handle().clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(1500));
                    let _ = application::commands::open_runtime_log_window(app_handle);
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            infrastructure::desktop_runtime::handle_window_event(window, event);
        })
        .invoke_handler(tauri::generate_handler![
            application::commands::get_app_build_info,
            application::commands::check_app_update,
            application::commands::get_companion_install_status,
            application::commands::install_companion,
            application::commands::bootstrap_workspace,
            application::commands::prepare_connector_runtimes,
            application::commands::check_connector_updates,
            application::commands::update_connector_runtime,
            application::commands::set_connector_custom_override,
            application::commands::clear_connector_custom_override,
            application::commands::upsert_provider_account,
            application::commands::delete_provider_account,
            application::commands::load_provider_account_cookies,
            application::commands::save_provider_account_cookies,
            application::commands::import_provider_account_cookies,
            application::commands::clear_provider_account_cookies,
            application::commands::validate_provider_account,
            application::commands::revert_provider_account_import,
            application::commands::query_runtime_logs,
            application::commands::load_runtime_log_context,
            application::commands::query_connector_debug,
            application::commands::clear_connector_debug,
            application::commands::list_import_providers,
            application::commands::list_import_methods,
            application::commands::list_import_roots,
            application::commands::preview_import_method,
            application::commands::run_import_method,
            application::commands::pick_import_root_folder,
            application::commands::enqueue_import_preview,
            application::commands::enqueue_import_run,
            application::commands::enqueue_import_backfill,
            application::commands::import_queue_status,
            application::commands::load_provider_account_editor,
            application::commands::save_provider_account_settings,
            application::commands::clone_provider_account,
            application::commands::upsert_source_profile,
            application::commands::batch_update_source_profiles,
            application::commands::change_source_media_path,
            application::commands::enqueue_source_media_path_migration,
            application::commands::media_path_migration_queue_status,
            application::commands::cancel_media_path_migrations,
            application::commands::open_batch_editor_window,
            application::commands::delete_source_profile,
            application::commands::enqueue_source_delete,
            application::commands::check_source_availability,
            application::commands::run_source_sync,
            application::commands::run_instagram_saved_posts_sync,
            application::commands::cancel_source_sync_profile,
            application::commands::cancel_source_sync_provider,
            application::commands::pause_source_sync_provider,
            application::commands::resume_source_sync_provider,
            application::commands::reorder_source_sync_provider_queue,
            application::commands::source_sync_queue_status,
            application::commands::source_delete_queue_status,
            application::commands::pick_source_profile_image,
            application::commands::reset_source_profile_image,
            application::commands::upsert_scheduler_set,
            application::commands::delete_scheduler_set,
            application::commands::upsert_scheduler_group,
            application::commands::delete_scheduler_group,
            application::commands::upsert_sync_plan,
            application::commands::preview_sync_plan_target,
            application::commands::delete_sync_plan,
            application::commands::run_sync_plan_now,
            application::commands::pause_sync_plan,
            application::commands::resume_sync_plan,
            application::commands::skip_sync_plan,
            application::commands::set_sync_plan_pause,
            application::commands::clear_sync_plan_pause,
            application::commands::apply_sync_plan_skip,
            application::commands::move_sync_plan,
            application::commands::clone_sync_plan,
            application::commands::open_source_folder,
            application::commands::load_source_media_gallery,
            application::commands::load_media_thumbnails,
            application::commands::load_avatar_thumbnails,
            application::commands::enqueue_media_thumbnail_generation,
            application::commands::media_thumbnail_queue_status,
            application::commands::enqueue_single_video_download,
            application::commands::single_video_queue_status,
            application::commands::list_single_videos,
            application::commands::delete_single_video,
            application::commands::delete_source_media,
            application::commands::upsert_app_setting,
            application::commands::prepare_connector_runtimes,
            application::commands::check_connector_updates,
            application::commands::update_connector_runtime,
            application::commands::set_connector_custom_override,
            application::commands::clear_connector_custom_override,
            application::commands::desktop_runtime_state,
            application::commands::system_short_date_pattern,
            application::commands::set_close_to_tray,
            application::commands::set_silent_mode,
            application::commands::open_runtime_log_window,
            application::commands::open_connector_debug_window,
            application::commands::open_scheduler_window,
            application::commands::open_plans_window,
            application::commands::open_source_sync_queue_window,
            application::commands::open_profile_view_window,
            application::commands::open_connector_runtimes_window,
            application::commands::open_single_videos_window,
            application::commands::open_accounts_window,
            application::commands::open_source_editor_window,
            application::commands::open_profile_editor_window,
            application::commands::close_profile_editor_window,
            application::commands::open_import_window,
            application::commands::report_runtime_log_window_ready,
            application::commands::report_runtime_log_window_bootstrap_failure,
            application::commands::runtime_log_window_status,
            application::commands::activate_main_window,
            application::commands::hide_main_window,
            application::commands::route_notification_action
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application")
}
