//! System tray icon + menu (SPEC.md §8 "Tray").
//!
//! `toggle_watcher` and `rescan` are wired to the real watcher/runtime (T-3.9):
//! `toggle_watcher` flips [`crate::runtime::SyncRuntime::pause`]/`resume` (mirroring
//! `commands::queue::pause_watcher`/`resume_watcher`) and its label follows
//! [`SyncRuntime::is_paused`]; `rescan` runs the same `rescan()` the manual
//! "Rescan" button in the UI triggers. Both run on a spawned task since
//! `on_menu_event`'s callback is synchronous.
//! Handlers log via `tracing` (CLAUDE.md) — no direct stdout writes.

use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

use osystems_sync_core::state::AppStatus;

use crate::state::AppState;

/// The tray icon's visual state. Swapped via [`set_tray_state`], driven off real
/// `AppStatus` snapshots by [`tray_state_for`] (T-5.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    Ok,
    Working,
    Error,
}

impl TrayState {
    fn icon_bytes(self) -> &'static [u8] {
        match self {
            TrayState::Ok => include_bytes!("../icons/tray-ok.png"),
            TrayState::Working => include_bytes!("../icons/tray-working.png"),
            TrayState::Error => include_bytes!("../icons/tray-error.png"),
        }
    }
}

/// The tray icon's stable id, used to look it up later via [`AppHandle::tray_by_id`].
pub const TRAY_ID: &str = "main";

const PAUSE_LABEL: &str = "Pausar watcher";
const RESUME_LABEL: &str = "Retomar watcher";

/// The tray's menu items that need mutating after the tray is built (label/enabled
/// state) — `TrayIcon` itself exposes no getter back to its `MenuItem`s once built,
/// so these are `app.manage()`d here and looked up from `handle_menu_event`.
struct TrayMenuItems {
    toggle_watcher_item: MenuItem<tauri::Wry>,
}

/// Builds and registers the tray icon + its menu.
///
/// Menu items: `open` ("Abrir"), `toggle_watcher` ("Pausar watcher" / "Retomar
/// watcher"), `rescan` ("Rescan"), `quit` ("Sair"). Left-click on the icon shows
/// the main window; the `open` menu item does the same.
pub fn build_tray(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let open_item = MenuItem::with_id(app, "open", "Abrir", true, None::<&str>)?;
    let toggle_watcher_item =
        MenuItem::with_id(app, "toggle_watcher", PAUSE_LABEL, true, None::<&str>)?;
    let rescan_item = MenuItem::with_id(app, "rescan", "Rescan", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &open_item,
            &toggle_watcher_item,
            &rescan_item,
            &separator,
            &quit_item,
        ],
    )?;

    app.manage(TrayMenuItems {
        toggle_watcher_item: toggle_watcher_item.clone(),
    });

    let icon = Image::from_bytes(TrayState::Ok.icon_bytes())?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip("oSystems Sync")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(handle_menu_event)
        .on_tray_icon_event(handle_tray_icon_event)
        .build(app)
}

fn handle_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    match event.id().as_ref() {
        "open" => show_main_window(app),
        "quit" => app.exit(0),
        "toggle_watcher" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { toggle_watcher(&app).await });
        }
        "rescan" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { rescan_from_tray(&app).await });
        }
        other => tracing::warn!("[tray] evento de menu não tratado: {other}"),
    }
}

/// Pauses (or resumes + rescans) the watcher — the tray equivalent of
/// `commands::queue::pause_watcher`/`resume_watcher` — then syncs the menu item's
/// label and the app's `status-changed` event.
async fn toggle_watcher(app: &AppHandle) {
    let state = app.state::<AppState>();

    if state.runtime.is_paused() {
        let watch = state.config.read().await.watch.clone();
        if let Err(err) = state.runtime.resume(state.repo.clone(), watch).await {
            if !matches!(err, osystems_sync_core::rescan::RescanError::NoPath) {
                tracing::warn!(error = %err, "[tray] varredura do resume falhou");
            }
        }
    } else {
        state.runtime.pause().await;
    }

    if let Some(items) = app.try_state::<TrayMenuItems>() {
        let label = if state.runtime.is_paused() {
            RESUME_LABEL
        } else {
            PAUSE_LABEL
        };
        if let Err(err) = items.toggle_watcher_item.set_text(label) {
            tracing::warn!(error = %err, "[tray] falha ao atualizar o rótulo do menu toggle_watcher");
        }
    }

    match state.runtime.build_app_status(&state.repo).await {
        Ok(status) => crate::events::emit_status_changed(app, &status),
        Err(err) => {
            tracing::warn!(error = %err, "[tray] falha ao construir o status do app após alternar o watcher")
        }
    }
}

/// Runs the same manual rescan `commands::queue::rescan` exposes to the UI.
async fn rescan_from_tray(app: &AppHandle) {
    let state = app.state::<AppState>();
    let watch = state.config.read().await.watch.clone();

    match osystems_sync_core::rescan::rescan(
        state.repo.clone(),
        state.runtime.wakers(),
        &watch,
        &state.runtime.stabilize_config(),
    )
    .await
    {
        Ok(report) => tracing::info!(enqueued = report.enqueued, "[tray] varredura concluída"),
        Err(err) => tracing::warn!(error = %err, "[tray] varredura falhou"),
    }

    match state.runtime.build_app_status(&state.repo).await {
        Ok(status) => crate::events::emit_status_changed(app, &status),
        Err(err) => {
            tracing::warn!(error = %err, "[tray] falha ao construir o status do app após a varredura")
        }
    }
}

fn handle_tray_icon_event(tray: &TrayIcon, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
    } = event
    {
        show_main_window(tray.app_handle());
    }
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Swaps the tray icon to reflect `state` (ok / working / error). Called from
/// `events.rs` on every `AppStatus` rebuild (see `apply_tray_state`), which caches the
/// last-applied state itself so this only actually runs `set_icon` on a real change.
pub fn set_tray_state(app: &AppHandle, state: TrayState) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let icon = Image::from_bytes(state.icon_bytes())?;
        tray.set_icon(Some(icon))?;
    }
    Ok(())
}

/// The tray icon state that should reflect `status` right now (T-5.10, PLAN.md):
/// `Working` while anything is actively uploading, `Error` if anything failed or
/// either destination needs re-authentication, `Ok` otherwise. Pure so it is
/// unit-testable without a real tray icon or `AppHandle`.
pub fn tray_state_for(status: &AppStatus) -> TrayState {
    let counts = &status.counts_by_status;
    if counts.uploading > 0 {
        TrayState::Working
    } else if counts.failed > 0
        || status.destinations.gdrive.auth_required
        || status.destinations.s3.auth_required
    {
        TrayState::Error
    } else {
        TrayState::Ok
    }
}

/// The tray tooltip text for `status` (T-5.10): `"oSystems Sync — N na fila, M
/// enviando"`, `N`/`M` from the same `counts_by_status` the Dashboard KPIs use.
pub fn tray_tooltip_for(status: &AppStatus) -> String {
    format!(
        "oSystems Sync — {} na fila, {} enviando",
        status.counts_by_status.pending, status.counts_by_status.uploading
    )
}

#[cfg(test)]
mod tray_state_tests {
    use super::*;
    use osystems_sync_core::state::{DestinationHealth, DestinationsHealth, StatusCounts};

    fn status_with(counts: StatusCounts, gdrive_auth: bool, s3_auth: bool) -> AppStatus {
        AppStatus {
            watcher_paused: false,
            destinations: DestinationsHealth {
                gdrive: DestinationHealth {
                    online: true,
                    auth_required: gdrive_auth,
                    latency_ms: None,
                },
                s3: DestinationHealth {
                    online: true,
                    auth_required: s3_auth,
                    latency_ms: None,
                },
            },
            counts_by_status: counts,
            core_version: "test".to_string(),
            build_target: "test".to_string(),
        }
    }

    #[test]
    fn ok_when_nothing_uploading_failed_or_auth_required() {
        let status = status_with(StatusCounts::default(), false, false);
        assert_eq!(tray_state_for(&status), TrayState::Ok);
    }

    #[test]
    fn working_takes_priority_when_uploading_even_if_something_also_failed() {
        let counts = StatusCounts {
            uploading: 1,
            failed: 3,
            ..StatusCounts::default()
        };
        let status = status_with(counts, false, false);
        assert_eq!(tray_state_for(&status), TrayState::Working);
    }

    #[test]
    fn error_when_something_failed() {
        let counts = StatusCounts {
            failed: 1,
            ..StatusCounts::default()
        };
        let status = status_with(counts, false, false);
        assert_eq!(tray_state_for(&status), TrayState::Error);
    }

    #[test]
    fn error_when_either_destination_needs_auth() {
        let status = status_with(StatusCounts::default(), true, false);
        assert_eq!(tray_state_for(&status), TrayState::Error);

        let status = status_with(StatusCounts::default(), false, true);
        assert_eq!(tray_state_for(&status), TrayState::Error);
    }

    #[test]
    fn tooltip_reports_pending_and_uploading_counts() {
        let counts = StatusCounts {
            pending: 5,
            uploading: 2,
            ..StatusCounts::default()
        };
        let status = status_with(counts, false, false);
        assert_eq!(
            tray_tooltip_for(&status),
            "oSystems Sync — 5 na fila, 2 enviando"
        );
    }
}
