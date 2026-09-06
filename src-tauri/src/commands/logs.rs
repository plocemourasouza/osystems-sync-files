//! Log-related IPC commands (SPEC.md §7): `get_recent_logs`, `open_logs_folder`.

use tauri::AppHandle;
use tauri::State;
use tauri_plugin_opener::OpenerExt;

use osystems_sync_core::logging::LogLine;

use crate::error::AppError;
use crate::state::AppState;

/// Absolute ceiling on `get_recent_logs`' `limit`, mirroring `LoggingHandle`'s own
/// 500-line ring buffer — asking for more than the buffer holds can never return more
/// than this anyway, so cap here defensively rather than trust the caller.
const MAX_LOG_LINES: usize = 500;

/// Returns up to `limit` (capped at 500) of the most recent log lines, oldest first.
#[tauri::command]
pub async fn get_recent_logs(
    state: State<'_, AppState>,
    limit: usize,
) -> Result<Vec<LogLine>, AppError> {
    Ok(state.logging.recent(clamp_limit(limit)))
}

/// Reveals the `logs/` directory under the app's data dir in the OS file explorer
/// (RF-098 — "abrir pasta de logs").
#[tauri::command]
pub async fn open_logs_folder(app: AppHandle, state: State<'_, AppState>) -> Result<(), AppError> {
    let logs_dir = state.data_dir.join("logs");
    let path = logs_dir.to_string_lossy().into_owned();

    app.opener()
        .open_path(path, None::<String>)
        .map_err(AppError::from)
}

/// Caps a caller-supplied `get_recent_logs` limit at [`MAX_LOG_LINES`].
fn clamp_limit(limit: usize) -> usize {
    limit.min(MAX_LOG_LINES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_limit_passes_through_values_within_range() {
        assert_eq!(clamp_limit(0), 0);
        assert_eq!(clamp_limit(50), 50);
        assert_eq!(clamp_limit(MAX_LOG_LINES), MAX_LOG_LINES);
    }

    #[test]
    fn clamp_limit_caps_values_above_the_max() {
        assert_eq!(clamp_limit(MAX_LOG_LINES + 1), MAX_LOG_LINES);
        assert_eq!(clamp_limit(usize::MAX), MAX_LOG_LINES);
    }
}
