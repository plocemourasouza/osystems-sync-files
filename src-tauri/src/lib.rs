mod commands;
mod error;
mod events;
mod runtime;
mod state;
mod tray;

use tauri::{Manager, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;

/// Boots the Tauri application.
///
/// ## Close-to-tray behaviour (RF-091)
///
/// The main window's `CloseRequested` event is intercepted: `api.prevent_close()`
/// stops Tauri from destroying the window and `window.hide()` takes it out of
/// sight instead, while the process (and any in-flight watcher/upload work)
/// keeps running in the background. The only way to actually terminate the
/// process today is the tray menu's `Sair`, which calls `app.exit(0)` directly
/// (see `tray::handle_menu_event`) — `exit` does not emit `CloseRequested`, so
/// it is never intercepted by this handler. Clicking the tray icon (or its
/// `Abrir` item) calls `tray::show_main_window` to bring the window back.
///
/// `app.exit(0)` here is an immediate exit, not a graceful one: waiting for
/// in-flight uploads to finish (or cancel) before quitting is RF-095 and
/// lands with Fase 5's shutdown-sequencing work, not in this task.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let state = state::bootstrap()?;
            // Grab the `Arc`s this closure and the boot/runtime-start tasks need before
            // `app.manage(state)` moves `state` away — all cheap `Arc`/`CancellationToken`
            // clones.
            let logging = state.logging.clone();
            let sync_runtime = state.runtime.clone();
            let repo = state.repo.clone();
            let config = state.config.clone();
            let credentials = state.credentials.clone();
            let uploaders = state.uploaders.clone();
            let throttles = state.throttles.clone();
            let pool_cell = state.pool.clone();
            let keep_awake = state.keep_awake.clone();
            let night_mode = state.night_mode.clone();
            let resume_cancel = state.resume_cancel.clone();
            let night_mode_cancel = state.night_mode_cancel.clone();
            app.manage(state);
            tray::build_tray(app.handle())?;
            events::spawn_log_forwarder(app.handle().clone(), logging);
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // `boot_workers` (health monitor + crash recovery + worker pool) runs
                // before `start` (watcher/intake + startup rescan) — see PLAN.md T-3.9
                // and `SyncRuntime::boot_workers`'s module docs for why health is
                // spawned ahead of the pool it's watching, deliberately reordered from
                // the task's literal bootstrap pseudocode.
                sync_runtime
                    .boot_workers(
                        app_handle.clone(),
                        repo.clone(),
                        config.clone(),
                        credentials,
                        uploaders,
                        throttles,
                        pool_cell,
                        keep_awake,
                        night_mode,
                        resume_cancel,
                        night_mode_cancel,
                    )
                    .await;
                sync_runtime.start(app_handle, repo, config).await;
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::system::set_autostart,
            commands::system::open_in_explorer,
            commands::system::open_author_link,
            commands::config::get_config,
            commands::config::save_config,
            commands::queue::pick_folder,
            commands::queue::rescan,
            commands::queue::pause_watcher,
            commands::queue::resume_watcher,
            commands::queue::list_jobs,
            commands::queue::get_status,
            commands::queue::retry_job,
            commands::queue::retry_all_failed,
            commands::queue::cancel_job,
            commands::queue::clear_completed,
            commands::queue::test_connection,
            commands::queue::open_remote,
            commands::queue::pause_job,
            commands::queue::resume_job,
            commands::credentials::set_credential,
            commands::credentials::get_credential_status,
            commands::credentials::clear_credential,
            commands::credentials::pick_service_account_file,
            commands::qos::set_qos,
            commands::logs::get_recent_logs,
            commands::logs::open_logs_folder,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // Graceful shutdown (RF-095, PLAN.md T-3.9): `Sair` (tray) and the OS
            // asking the whole app to quit both raise `ExitRequested` — give in-flight
            // uploads up to 30s to finish/cancel and stop the health monitor's probe
            // loop before the process actually goes away. The window's own
            // `CloseRequested` (handled above) is a different, earlier event — it
            // never reaches here because `api.prevent_close()` stops the close, not
            // the app, from proceeding.
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let state = app_handle.state::<state::AppState>();
                let runtime = state.runtime.clone();
                runtime.cancel_health();
                state.resume_cancel.cancel();
                state.night_mode_cancel.cancel();
                if let Some(pool) = state.pool.get().cloned() {
                    tauri::async_runtime::block_on(
                        pool.shutdown(std::time::Duration::from_secs(30)),
                    );
                }
            }
        });
}
