//! Shared error type returned by all Tauri commands (SPEC.md §7).
//!
//! Renderer never receives internal error details beyond `code` + `message` —
//! anything sensitive must be logged server-side (core) instead.

use serde::Serialize;

/// Typed, serializable error returned by every `#[tauri::command]`.
#[derive(Debug, Serialize)]
pub struct AppError {
    pub code: String,
    pub message: String,
}

impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

impl From<tauri::Error> for AppError {
    fn from(err: tauri::Error) -> Self {
        AppError::new("tauri_error", err.to_string())
    }
}

impl From<tauri_plugin_autostart::Error> for AppError {
    fn from(err: tauri_plugin_autostart::Error) -> Self {
        AppError::new("autostart_error", err.to_string())
    }
}

impl From<osystems_sync_core::config::ConfigError> for AppError {
    fn from(err: osystems_sync_core::config::ConfigError) -> Self {
        use osystems_sync_core::config::ConfigError;
        match err {
            ConfigError::Io(e) => AppError::new("config.io", e.to_string()),
            ConfigError::Parse(e) => AppError::new("config.parse", e.to_string()),
            ConfigError::NoHome => AppError::new(
                "config.no_home",
                "could not determine the user's home/config directory",
            ),
        }
    }
}

impl From<osystems_sync_core::logging::LoggingError> for AppError {
    fn from(err: osystems_sync_core::logging::LoggingError) -> Self {
        AppError::new("logging.init", err.to_string())
    }
}

impl From<osystems_sync_core::state::StateError> for AppError {
    fn from(err: osystems_sync_core::state::StateError) -> Self {
        AppError::new("state.db", err.to_string())
    }
}

impl From<osystems_sync_core::queue::QueueError> for AppError {
    fn from(err: osystems_sync_core::queue::QueueError) -> Self {
        AppError::new("queue.error", err.to_string())
    }
}

impl From<osystems_sync_core::rescan::RescanError> for AppError {
    fn from(err: osystems_sync_core::rescan::RescanError) -> Self {
        AppError::new("rescan.error", err.to_string())
    }
}

impl From<osystems_sync_core::watcher::WatcherError> for AppError {
    fn from(err: osystems_sync_core::watcher::WatcherError) -> Self {
        AppError::new("watcher.error", err.to_string())
    }
}

impl From<tauri_plugin_opener::Error> for AppError {
    fn from(err: tauri_plugin_opener::Error) -> Self {
        AppError::new("opener_error", err.to_string())
    }
}

impl From<osystems_sync_core::credentials::CredError> for AppError {
    fn from(err: osystems_sync_core::credentials::CredError) -> Self {
        // `CredError`'s `Display` is already scrubbed of secret values (module docs on
        // `credentials.rs`: only the backend's own error message, never a stored
        // credential) so it's safe to surface directly, same as every other `From` here.
        AppError::new("credentials.error", err.to_string())
    }
}

impl From<osystems_sync_core::uploaders::UploadError> for AppError {
    fn from(err: osystems_sync_core::uploaders::UploadError) -> Self {
        // `UploadError::code()` is already the stable machine-readable tag
        // (`"auth"`/`"transient"`/`"permanent"`/`"io"`/`"cancelled"`) — reused verbatim
        // as the `AppError.code` suffix so `test_connection`/uploader-rebuild failures
        // are just as inspectable client-side as any other command error.
        AppError::new(format!("upload.{}", err.code()), err.to_string())
    }
}

impl From<osystems_sync_core::worker::WorkerError> for AppError {
    fn from(err: osystems_sync_core::worker::WorkerError) -> Self {
        use osystems_sync_core::worker::WorkerError;
        match err {
            WorkerError::NotFound(_) => AppError::new("job.not_found", err.to_string()),
            WorkerError::InvalidTransition(..) => {
                AppError::new("job.invalid_transition", err.to_string())
            }
            WorkerError::State(inner) => AppError::from(inner),
        }
    }
}
