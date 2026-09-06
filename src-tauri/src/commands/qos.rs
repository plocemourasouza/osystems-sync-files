//! Bandwidth-limit IPC command (SPEC.md §6 `throttle.rs`, §7; PRD.md RF-051):
//! `set_qos`.

use tauri::{AppHandle, State};

use osystems_sync_core::config;
use osystems_sync_core::state::{Destination, Throughput};
use osystems_sync_core::throttle::mbps_to_bps;

use crate::error::AppError;
use crate::state::AppState;

/// Valid non-`None` range for a QoS limit, in MB/s (SPEC.md §7). `None` means
/// "unlimited" and is always allowed regardless of this range.
const MIN_LIMIT_MBPS: f64 = 0.5;
const MAX_LIMIT_MBPS: f64 = 10.0;

/// Validates a caller-supplied limit: `None` (unlimited) is always fine; `Some(v)`
/// must be finite and within `[MIN_LIMIT_MBPS, MAX_LIMIT_MBPS]`.
fn validate_limit(limit_mbps: Option<f64>) -> Result<(), AppError> {
    match limit_mbps {
        None => Ok(()),
        Some(v) if v.is_finite() && (MIN_LIMIT_MBPS..=MAX_LIMIT_MBPS).contains(&v) => Ok(()),
        Some(v) => Err(AppError::new(
            "qos.invalid_limit",
            format!(
                "limit_mbps must be between {MIN_LIMIT_MBPS} and {MAX_LIMIT_MBPS}, or omitted for unlimited (got {v})"
            ),
        )),
    }
}

/// Sets (or clears) the bandwidth cap for one destination. Applies to the live
/// `Arc<Throttle>` immediately (so in-flight transfers feel it on their very next
/// `acquire()`, no restart needed — RF-051) and persists it into `config.qos` so it
/// survives a relaunch.
///
/// This does not itself emit a `throughput` event: `WorkerPool`'s own 1 Hz ticker
/// reads the exact same `Arc<Throttle>` this command just updated
/// (`AppState::throttles`), so the new limit is reflected within at most one second
/// through the existing ticker — an extra emission here would either race that one or
/// have to fabricate the current in-flight bytes/s figure, which only the worker
/// pool's private `Meters` track.
#[tauri::command]
pub async fn set_qos(
    app: AppHandle,
    state: State<'_, AppState>,
    destination: Destination,
    limit_mbps: Option<f64>,
) -> Result<(), AppError> {
    validate_limit(limit_mbps)?;

    let limit_bps = limit_mbps.map(mbps_to_bps).unwrap_or(0);
    let throttle = match destination {
        Destination::S3 => &state.throttles.s3,
        Destination::GDrive => &state.throttles.gdrive,
    };

    // `NightModeScheduler` (index 0 = GDrive, 1 = S3 — `state::bootstrap_in`'s
    // construction order) tracks its own "daytime" limit per throttle so it can
    // restore it when the night window ends; keep that in sync regardless of
    // whether we're currently inside the window. The live `Throttle` itself is only
    // touched here while outside the window -- during the window it must stay at 0
    // (night mode's whole point), and `NightModeScheduler::tick`'s next run will
    // reapply this new daytime figure the moment the window closes.
    let night_idx = match destination {
        Destination::GDrive => 0,
        Destination::S3 => 1,
    };
    state.night_mode.set_daytime_limit(night_idx, limit_bps);
    if !state.night_mode.is_night() {
        throttle.set_limit(limit_bps);
    }

    let mut new_config = state.config.read().await.clone();
    match destination {
        Destination::S3 => new_config.qos.s3_limit_mbps = limit_mbps,
        Destination::GDrive => new_config.qos.gdrive_limit_mbps = limit_mbps,
    }

    let dir = state.data_dir.clone();
    let to_persist = new_config.clone();
    tauri::async_runtime::spawn_blocking(move || config::save(&dir, &to_persist))
        .await
        .map_err(|join_err| {
            tracing::error!(error = %join_err, "tarefa de salvamento de configuração do set_qos entrou em panic");
            AppError::new("config.io", "failed to persist the new bandwidth limit")
        })??;

    *state.config.write().await = new_config;

    // Best-effort immediate feedback: this is 0 sent-bytes/s if nothing is
    // in-flight right now (which is also what the real ticker would show for an idle
    // destination), and gets superseded by the real ticker's figure within 1s once a
    // transfer is active.
    crate::events::emit_throughput(
        &app,
        &Throughput {
            total_bps: 0,
            gdrive_bps: 0,
            s3_bps: 0,
            limit_gdrive_bps: non_zero(state.throttles.gdrive.limit_bps()),
            limit_s3_bps: non_zero(state.throttles.s3.limit_bps()),
        },
    );

    Ok(())
}

/// `Throttle::limit_bps()` uses `0` for "unlimited"; `Throughput.limit_*_bps` is
/// `Option<u64>` with the same convention at the JSON boundary (`None` -> `null`) —
/// this bridges the two so the emitted event matches what `worker.rs`'s own ticker
/// would produce for the same state.
fn non_zero(bps: u64) -> Option<u64> {
    if bps == 0 {
        None
    } else {
        Some(bps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_limit_accepts_none() {
        assert!(validate_limit(None).is_ok());
    }

    #[test]
    fn validate_limit_accepts_the_boundaries() {
        assert!(validate_limit(Some(MIN_LIMIT_MBPS)).is_ok());
        assert!(validate_limit(Some(MAX_LIMIT_MBPS)).is_ok());
        assert!(validate_limit(Some(3.5)).is_ok());
    }

    #[test]
    fn validate_limit_rejects_out_of_range_or_non_finite() {
        assert!(validate_limit(Some(0.0)).is_err());
        assert!(validate_limit(Some(0.49)).is_err());
        assert!(validate_limit(Some(10.01)).is_err());
        assert!(validate_limit(Some(f64::NAN)).is_err());
        assert!(validate_limit(Some(f64::INFINITY)).is_err());
    }

    #[test]
    fn non_zero_maps_zero_to_none() {
        assert_eq!(non_zero(0), None);
        assert_eq!(non_zero(42), Some(42));
    }
}
