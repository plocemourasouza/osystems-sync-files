//! `get_config` / `save_config` IPC commands (SPEC.md §7, PRD.md RF-083/085/086).
//!
//! `save_config` applies at runtime (T-3.9): the in-memory config swap below takes
//! effect immediately for anything that reads `state.config` on demand (`retry`,
//! autostart, ...). Three sections need an extra push beyond the swap because their
//! consumer cached something at boot instead of re-reading `state.config` every time:
//! `watch` restarts the watcher (T-2.6), `workers_per_destination` calls
//! `WorkerPool::resize`, and `qos`/`s3` push straight into the live `Throttle`s /
//! rebuild the S3 uploader.

use tauri::{AppHandle, State};

use osystems_sync_core::config::{self as core_config, AppConfig, ValidationIssue, WatchConfig};
use osystems_sync_core::throttle::mbps_to_bps;

use crate::error::AppError;
use crate::state::AppState;

/// Returns the current in-memory config (already loaded/validated at boot by
/// [`crate::state::bootstrap`]).
#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<AppConfig, AppError> {
    Ok(state.config.read().await.clone())
}

/// Validates, persists, and applies `config` at runtime.
///
/// Order matters: validation happens before anything touches disk or the in-memory
/// state, so a rejected save leaves both untouched. Persisting to `config.json` runs
/// inside `spawn_blocking` — it is synchronous file I/O and must not block the Tokio
/// runtime (CLAUDE.md).
#[tauri::command]
pub async fn save_config(
    app: AppHandle,
    state: State<'_, AppState>,
    config: AppConfig,
) -> Result<(), AppError> {
    if let Err(issues) = core_config::validate(&config) {
        return Err(issues_to_error(issues));
    }

    let dir = state.data_dir.clone();
    let to_persist = config.clone();
    let save_result =
        tauri::async_runtime::spawn_blocking(move || core_config::save(&dir, &to_persist))
            .await
            .map_err(|join_err| {
                AppError::new(
                    "config.io",
                    format!("save_config background task panicked: {join_err}"),
                )
            })?;
    save_result?;

    let previous = state.config.read().await.clone();
    *state.config.write().await = config.clone();

    if watch_changed(&previous.watch, &config.watch) {
        state
            .runtime
            .restart_watcher(app.clone(), state.repo.clone(), config.watch.clone())
            .await;
    }

    // `retry` needs no explicit propagation: `worker.rs` reads `deps.config` (the very
    // same `Arc<RwLock<AppConfig>>` this function just wrote into) fresh on every
    // retry decision, so the new policy applies to the next failure without a restart.
    if previous.workers_per_destination != config.workers_per_destination {
        match state.pool.get() {
            Some(pool) => pool.resize(config.workers_per_destination).await,
            None => tracing::warn!(
                "workers_per_destination alterado antes do pool de workers terminar de iniciar; \
                 a nova contagem se aplica assim que ele iniciar"
            ),
        }
    }

    if previous.qos != config.qos {
        state
            .throttles
            .s3
            .set_limit(config.qos.s3_limit_mbps.map(mbps_to_bps).unwrap_or(0));
        state
            .throttles
            .gdrive
            .set_limit(config.qos.gdrive_limit_mbps.map(mbps_to_bps).unwrap_or(0));
    }

    if previous.qos.night_mode != config.qos.night_mode {
        if let Err(err) = state.night_mode.configure(
            config.qos.night_mode.enabled,
            &config.qos.night_mode.start,
            &config.qos.night_mode.end,
        ) {
            // `core_config::validate` above already rejects malformed HH:MM times, so
            // this is unreachable in practice — logged rather than propagated so a
            // hypothetical future validation gap degrades to "night mode unchanged"
            // instead of failing the whole config save.
            tracing::warn!(error = %err, "falha ao reconfigurar o modo noturno; mantendo a janela anterior");
        }
    }

    if previous.s3 != config.s3 {
        state
            .runtime
            .rebuild_s3_uploader(
                &state.repo,
                &state.config,
                &state.credentials,
                &state.throttles,
                &state.uploaders,
            )
            .await?;
    }

    if previous.gdrive != config.gdrive {
        state
            .runtime
            .rebuild_gdrive_uploader(
                &state.repo,
                &state.config,
                &state.credentials,
                &state.throttles,
                &state.uploaders,
            )
            .await?;
    }

    if previous.keep_awake != config.keep_awake {
        state.keep_awake.set(config.keep_awake);
    }

    tracing::info!(changed_fields = ?changed_top_level_fields(&previous, &config), "configuração salva");

    Ok(())
}

/// Whether a saved `WatchConfig` change requires restarting the watcher.
///
/// Every field is consumed by the pipeline: `path`/`recursive` by
/// `watcher::spawn_watcher`, `extensions`/`min_size_mb`/`max_size_mb` by the two
/// `queue::passes_filters` calls in `run_intake_loop`, and `stabilize_seconds` by the
/// `StabilizeConfig` that `spawn_watcher_and_intake` derives. So there is no field this
/// could correctly ignore, and a hand-maintained list of the ones that "matter" only
/// gets forgotten — which is exactly what happened: `max_size_mb` was never listed, and
/// `min_size_mb` (ADR-017) was not added either, so saving a size filter wrote
/// `config.json` while the running intake loop kept the values from the last restart.
///
/// A spurious restart costs one sub-millisecond window where `notify` events are
/// dropped, on an explicit Settings save. A missed one costs a filter that silently
/// does not apply. Compare the whole struct.
fn watch_changed(old: &WatchConfig, new: &WatchConfig) -> bool {
    old != new
}

/// Maps a batch of validation failures into the single `AppError` the renderer expects:
/// code `config.invalid`, message = a JSON array of `{ field, message }` so the
/// frontend can highlight every offending field at once (`errors.config.invalid`).
fn issues_to_error(issues: Vec<ValidationIssue>) -> AppError {
    let message = serde_json::to_string(&issues).unwrap_or_else(|_| "[]".to_string());
    AppError::new("config.invalid", message)
}

/// Names of the top-level `AppConfig` sections that differ between `old` and `new`,
/// for the structured `"configuração salva"` log line. Field *names* only, never values —
/// RNF-015 forbids logging anything that could carry a secret, and while `AppConfig`
/// itself never holds one (see the `no_secret_like_keys_in_app_config_json` test
/// below), logging names instead of values keeps that guarantee robust to future
/// fields too.
fn changed_top_level_fields(old: &AppConfig, new: &AppConfig) -> Vec<&'static str> {
    let mut changed = Vec::new();
    if old.watch != new.watch {
        changed.push("watch");
    }
    if old.s3 != new.s3 {
        changed.push("s3");
    }
    if old.gdrive != new.gdrive {
        changed.push("gdrive");
    }
    if old.qos != new.qos {
        changed.push("qos");
    }
    if old.retry != new.retry {
        changed.push("retry");
    }
    if old.workers_per_destination != new.workers_per_destination {
        changed.push("workers_per_destination");
    }
    if old.autostart != new.autostart {
        changed.push("autostart");
    }
    if old.keep_awake != new.keep_awake {
        changed.push("keep_awake");
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issues_to_error_uses_stable_code_and_serializes_every_issue() {
        let issues = vec![
            ValidationIssue {
                field: "s3.bucket".to_string(),
                message: "bucket must not be empty when S3 is enabled".to_string(),
            },
            ValidationIssue {
                field: "workers_per_destination".to_string(),
                message: "must be between 1 and 4".to_string(),
            },
        ];

        let error = issues_to_error(issues.clone());

        assert_eq!(error.code, "config.invalid");
        let decoded: Vec<ValidationIssue> =
            serde_json::from_str(&error.message).expect("message must be a JSON array");
        assert_eq!(decoded, issues);
    }

    #[test]
    fn issues_to_error_on_empty_issues_still_serializes() {
        let error = issues_to_error(Vec::new());
        assert_eq!(error.code, "config.invalid");
        assert_eq!(error.message, "[]");
    }

    #[test]
    fn changed_top_level_fields_reports_only_differing_sections() {
        let old = AppConfig::default();
        let new = AppConfig {
            workers_per_destination: 4,
            autostart: !old.autostart,
            ..AppConfig::default()
        };

        let changed = changed_top_level_fields(&old, &new);

        assert_eq!(changed, vec!["workers_per_destination", "autostart"]);
    }

    #[test]
    fn changed_top_level_fields_is_empty_for_identical_configs() {
        let cfg = AppConfig::default();
        assert!(changed_top_level_fields(&cfg, &cfg).is_empty());
    }

    #[test]
    fn watch_changed_is_false_for_identical_watch_configs() {
        let watch = WatchConfig::default();
        assert!(!watch_changed(&watch, &watch));
    }

    // Every field, one mutation each. The predecessor of this test listed only
    // four of the six, and a sibling test actively asserted that `max_size_mb`
    // must NOT restart the watcher — which is the bug this replaced: saving a
    // size filter left the running intake loop on the old values.
    #[test]
    fn watch_changed_is_true_for_every_watch_field() {
        let base = WatchConfig::default();

        let mut path = base.clone();
        path.path = Some("C:/Sync".to_string());

        let mut recursive = base.clone();
        recursive.recursive = !base.recursive;

        let mut extensions = base.clone();
        extensions.extensions = vec!["pdf".to_string()];

        let mut min_size = base.clone();
        min_size.min_size_mb = base.min_size_mb + 10;

        let mut max_size = base.clone();
        max_size.max_size_mb = base.max_size_mb + 100;

        let mut stabilize = base.clone();
        stabilize.stabilize_seconds = base.stabilize_seconds + 1;

        for (name, changed) in [
            ("path", path),
            ("recursive", recursive),
            ("extensions", extensions),
            ("min_size_mb", min_size),
            ("max_size_mb", max_size),
            ("stabilize_seconds", stabilize),
        ] {
            assert!(
                watch_changed(&base, &changed),
                "changing {name} must restart the watcher"
            );
        }
    }

    /// RNF-003 / RNF-015 / CLAUDE.md: `AppConfig` must never carry a secret — those
    /// live exclusively in the OS keyring (see `SPEC.md` §5 "Credenciais"). This
    /// walks the *serialized JSON's keys* (not Rust field names, since `serde`
    /// renaming could otherwise hide a mismatch) recursively so a future field added
    /// under any nested struct is caught too.
    #[test]
    fn no_secret_like_keys_in_app_config_json() {
        let value = serde_json::to_value(AppConfig::default()).expect("AppConfig must serialize");
        let mut offending = Vec::new();
        collect_secret_like_keys(&value, "$", &mut offending);
        assert!(
            offending.is_empty(),
            "AppConfig JSON must never contain secret-shaped keys, found: {offending:?}"
        );
    }

    const FORBIDDEN_KEY_SUBSTRINGS: &[&str] = &["secret", "access_key", "private_key", "token"];

    fn collect_secret_like_keys(
        value: &serde_json::Value,
        path: &str,
        offending: &mut Vec<String>,
    ) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, nested) in map {
                    let lower = key.to_lowercase();
                    if FORBIDDEN_KEY_SUBSTRINGS
                        .iter()
                        .any(|forbidden| lower.contains(forbidden))
                    {
                        offending.push(format!("{path}.{key}"));
                    }
                    collect_secret_like_keys(nested, &format!("{path}.{key}"), offending);
                }
            }
            serde_json::Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    collect_secret_like_keys(item, &format!("{path}[{i}]"), offending);
                }
            }
            _ => {}
        }
    }
}
