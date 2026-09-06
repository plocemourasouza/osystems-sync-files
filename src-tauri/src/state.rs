//! Shared application state, managed via [`tauri::Manager::manage`] (SPEC.md §5/§7,
//! PLAN.md T-1.5).
//!
//! Bootstrapping order matters: logging must be initialized before anything else logs,
//! `config` is loaded (falling back to defaults on a malformed file — see
//! [`bootstrap_in`]) before it is handed to any command, and the SQLite [`Repo`] is
//! opened last since it is the most likely thing to fail with an I/O error.
//!
//! ## Locking strategy
//!
//! - `config` uses `tauri::async_runtime::RwLock` (a re-export of `tokio::sync::RwLock`)
//!   because commands read/write it from `async fn` contexts and only ever hold the
//!   lock across cheap in-memory work (clone / field replace) — never across blocking
//!   I/O — so an async-aware lock that lets other tasks make progress while a reader is
//!   parked is the right tool.
//! - `repo` uses `std::sync::Mutex` because [`Repo`] wraps a synchronous
//!   `rusqlite::Connection` (CLAUDE.md: "Não bloquear o runtime Tokio com I/O síncrono
//!   pesado"). Every access MUST happen inside `tauri::async_runtime::spawn_blocking`,
//!   where the lock is acquired and released entirely within a blocking-pool thread and
//!   never held across an `.await` — exactly the case a `std::sync::Mutex` is meant for
//!   (no `Send` requirement on the guard, cheaper than an async mutex).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use osystems_sync_core::config::{self, AppConfig};
use osystems_sync_core::credentials::{Credentials, KeyringStore};
use osystems_sync_core::health::HealthHandle;
use osystems_sync_core::logging::{self, LoggingHandle};
use osystems_sync_core::power::KeepAwake;
use osystems_sync_core::state::Repo;
use osystems_sync_core::throttle::{mbps_to_bps, NightModeScheduler, Throttle};
use osystems_sync_core::worker::{Throttles, Uploaders, WorkerPool};
use tauri::async_runtime::RwLock;
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::runtime::SyncRuntime;

/// Everything a command needs, managed once at startup via `app.manage(state)`.
pub struct AppState {
    /// Per-OS application data directory (`config.json`, `logs/`, `state.db` all live
    /// under here). Kept around so commands (e.g. `save_config`) don't need to
    /// re-resolve it.
    pub data_dir: PathBuf,
    /// In-memory mirror of `config.json`, kept in sync by `save_config`.
    pub config: Arc<RwLock<AppConfig>>,
    /// The `state.db` connection. See module docs for why this is a `std::sync::Mutex`.
    ///
    pub repo: Arc<Mutex<Repo>>,
    /// Keeps the logging subsystem (ring buffer, broadcast channel, file writer) alive
    /// for the app's lifetime; also read by `commands::logs::get_recent_logs`.
    pub logging: Arc<LoggingHandle>,
    /// Owns the watcher → stabilize → intake pipeline (T-2.6). Constructed here but
    /// only actually started (`SyncRuntime::start`) from `lib.rs`'s `.setup()`, once
    /// an `AppHandle` exists to emit events with.
    pub runtime: Arc<SyncRuntime>,
    /// OS-keychain-backed secret storage (T-3.9, SPEC.md §5 "Credenciais"). Never
    /// logs/serializes the values it reads/writes — see `commands::credentials`.
    pub credentials: Arc<Credentials<KeyringStore>>,
    /// Per-destination bandwidth throttles (SPEC.md §6 `throttle.rs`), built once from
    /// `config.qos` at boot and live-updated by `commands::qos::set_qos` and
    /// `commands::config::save_config` via `Throttle::set_limit` — never rebuilt, so
    /// every `Arc<Throttle>` handed to a worker/uploader at boot stays valid for the
    /// app's whole lifetime.
    pub throttles: Throttles,
    /// The active uploader per destination (`None` until credentials are configured),
    /// shared with the worker pool's [`WorkerDeps::uploaders`](osystems_sync_core::worker::WorkerDeps).
    /// Starts empty; `SyncRuntime::boot_workers` populates it once at boot, and
    /// `commands::credentials`/`commands::config` swap it (write lock) whenever AWS
    /// credentials or `config.s3` change at runtime.
    pub uploaders: Arc<RwLock<Uploaders>>,
    /// Set exactly once, from the async boot task in `lib.rs`'s `.setup()` (needs an
    /// `AppHandle` to build `WorkerDeps::events`, which isn't available yet in
    /// [`bootstrap_in`]). `None` for the brief window between `app.manage(state)` and
    /// that task completing — callers fall back to treating the pool as "not ready
    /// yet" rather than blocking or panicking.
    /// `Arc`-wrapped (unlike a bare `OnceLock` field) so it can be cloned into the
    /// `'static` tasks `SyncRuntime`/`lib.rs` spawn (the boot task, and the graceful
    /// `Sair`/`ExitRequested` shutdown task) without borrowing `AppState` itself.
    pub pool: Arc<OnceLock<Arc<WorkerPool>>>,
    /// Set exactly once, alongside `pool` — same "not available until boot completes"
    /// contract, same `Arc`-wrapping reason. [`SyncRuntime::build_app_status`] reads it
    /// (falling back to an all-offline snapshot beforehand) so `get_status`/
    /// `status-changed` never fail just because health hasn't started yet.
    ///
    /// Not read through this field directly today — every current reader goes through
    /// `SyncRuntime::health_handle()`, the literally-identical `Arc` clone `runtime`
    /// holds — but it is kept `pub` here for a future command that wants health
    /// without going through `runtime` (e.g. a dedicated `get_health_status`).
    #[allow(dead_code)]
    pub health: Arc<OnceLock<HealthHandle>>,
    /// Prevents the OS from sleeping while enabled (PLAN.md T-5.5, SPEC.md §6,
    /// RF-093). Constructed once here; `set(config.keep_awake)` is applied at boot
    /// (`SyncRuntime::boot_workers`) and again by `commands::config::save_config`
    /// whenever `config.keep_awake` changes.
    pub keep_awake: Arc<KeepAwake>,
    /// Drops the configured destinations' bandwidth to 0 during the configured night
    /// window (PLAN.md T-5.5, SPEC.md §6, RF-095/096). Built once from `throttles` so
    /// its `Arc<Throttle>`s are the exact same instances every uploader/worker holds;
    /// `configure`d at boot and by `save_config`, ticked by its own `spawn`ed task
    /// (`SyncRuntime::boot_workers`). Index `0` is GDrive, `1` is S3 — see
    /// `commands::qos::set_qos`.
    pub night_mode: Arc<NightModeScheduler>,
    /// Cancels the resume-detector task ([`osystems_sync_core::power::spawn_resume_detector`])
    /// started by `SyncRuntime::boot_workers`. Cancelled from `lib.rs`'s
    /// `RunEvent::ExitRequested` handler alongside `runtime.cancel_health()`.
    pub resume_cancel: CancellationToken,
    /// Cancels the night-mode scheduler's ticker task (`NightModeScheduler::spawn`)
    /// started by `SyncRuntime::boot_workers`. Cancelled from `lib.rs`'s
    /// `RunEvent::ExitRequested` handler alongside `runtime.cancel_health()`.
    pub night_mode_cancel: CancellationToken,
}

/// Boots [`AppState`] against the real per-OS data directory
/// ([`osystems_sync_core::config::data_dir`]). The only entry point used by the running
/// app (`lib.rs`'s `.setup()`); tests call [`bootstrap_in`] directly against a tempdir.
pub fn bootstrap() -> Result<AppState, AppError> {
    let data_dir = config::data_dir()?;
    bootstrap_in(data_dir)
}

/// Boots [`AppState`] against an arbitrary `data_dir`, so tests never touch the real
/// `%APPDATA%`/`~/Library/Application Support`.
///
/// `logging::init` installs a process-wide global `tracing` subscriber and can only
/// succeed once per process ([`LoggingError::AlreadyInitialized`] on a second call) —
/// fine for the one real call `bootstrap()` makes, but fatal for a test suite that
/// calls this function many times. Under `cfg(test)` this uses
/// [`logging::init_for_tests`] instead, which installs a thread-scoped default and is
/// safe to call repeatedly.
pub fn bootstrap_in(data_dir: PathBuf) -> Result<AppState, AppError> {
    std::fs::create_dir_all(&data_dir).map_err(|e| AppError::new("config.io", e.to_string()))?;

    #[cfg(not(test))]
    let logging = {
        let logs_dir = data_dir.join("logs");
        logging::init(&logs_dir, "info")?
    };
    #[cfg(test)]
    let logging = logging::init_for_tests();

    tracing::info!(version = osystems_sync_core::version(), "core iniciado");

    let cfg = match config::load(&data_dir) {
        Ok(cfg) => cfg,
        Err(config::ConfigError::Parse(err)) => {
            tracing::warn!(
                error = %err,
                "config.json malformado; usando padrões em memória sem sobrescrever o arquivo"
            );
            AppConfig::default()
        }
        Err(err) => return Err(err.into()),
    };

    // Crash recovery (`recover_on_boot`, RNF-006) runs from the async boot task in
    // `lib.rs`'s `.setup()` instead of here: `WorkerPool::recover_on_boot` best-effort
    // aborts orphaned multipart uploads through `WorkerDeps::uploaders`, so it must run
    // *after* `Uploaders` is built (see `SyncRuntime::boot_workers`'s module docs for
    // why this is a deliberate reordering of PLAN.md T-3.9's literal bootstrap
    // pseudocode) — plain DB access here would run it too early to actually recover
    // anything.
    let repo = Repo::open(&data_dir.join("state.db"))?;

    let throttles = throttles_from_qos(&cfg);

    // Shared with `SyncRuntime` (via `with_health`) so `AppState.health` and
    // `SyncRuntime`'s own `health` field are literally the same `Arc<OnceLock<..>>` —
    // whichever one `boot_workers` sets, both readers see it. `pool` has no such
    // counterpart inside `SyncRuntime`; it only ever lives here.
    let health: Arc<OnceLock<HealthHandle>> = Arc::new(OnceLock::new());

    let keep_awake = Arc::new(KeepAwake::new());
    // Index 0 = GDrive, 1 = S3 — `commands::qos::set_qos` relies on this exact order.
    let night_mode = NightModeScheduler::new(vec![throttles.gdrive.clone(), throttles.s3.clone()]);

    Ok(AppState {
        data_dir,
        config: Arc::new(RwLock::new(cfg)),
        repo: Arc::new(Mutex::new(repo)),
        logging: Arc::new(logging),
        runtime: Arc::new(SyncRuntime::with_health(health.clone())),
        credentials: Arc::new(Credentials::new(KeyringStore::default())),
        throttles,
        uploaders: Arc::new(RwLock::new(Uploaders::default())),
        pool: Arc::new(OnceLock::new()),
        health,
        keep_awake,
        night_mode,
        resume_cancel: CancellationToken::new(),
        night_mode_cancel: CancellationToken::new(),
    })
}

/// Builds the boot-time [`Throttles`] from `config.qos`'s configured `*_limit_mbps`
/// (`None`/absent means unlimited, i.e. `0` bytes/sec — `Throttle`'s own convention).
/// Night mode (`config.qos.night_mode`) is not applied here: it is a time-of-day
/// override PLAN.md schedules for a later task, not a boot-time constant.
fn throttles_from_qos(cfg: &AppConfig) -> Throttles {
    let s3_bps = cfg.qos.s3_limit_mbps.map(mbps_to_bps).unwrap_or(0);
    let gdrive_bps = cfg.qos.gdrive_limit_mbps.map(mbps_to_bps).unwrap_or(0);
    Throttles {
        s3: Throttle::new(s3_bps),
        gdrive: Throttle::new(gdrive_bps),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_in_creates_dirs_default_config_and_db() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let data_dir = tmp.path().to_path_buf();

        let state = bootstrap_in(data_dir.clone()).expect("bootstrap_in should succeed");

        assert_eq!(state.data_dir, data_dir);
        assert!(data_dir.join("state.db").exists());
        // No config.json existed yet, so `load()` must return defaults without ever
        // creating the file (that's `save_config`'s job, not boot's).
        assert!(!data_dir.join("config.json").exists());

        let cfg = tauri::async_runtime::block_on(async { state.config.read().await.clone() });
        assert_eq!(cfg, AppConfig::default());
    }

    #[test]
    fn bootstrap_in_falls_back_to_defaults_on_malformed_config_without_overwriting() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let data_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(&data_dir).expect("create data dir");
        let config_path = data_dir.join("config.json");
        std::fs::write(&config_path, b"{ not valid json").expect("write malformed config");

        let state = bootstrap_in(data_dir.clone())
            .expect("bootstrap_in should recover from a malformed config.json");

        let cfg = tauri::async_runtime::block_on(async { state.config.read().await.clone() });
        assert_eq!(cfg, AppConfig::default());

        let raw = std::fs::read_to_string(&config_path).expect("config.json must still exist");
        assert_eq!(
            raw, "{ not valid json",
            "a parse error must never overwrite the file on disk"
        );
    }

    #[test]
    fn bootstrap_in_reloads_a_previously_saved_config() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let data_dir = tmp.path().to_path_buf();
        let cfg = AppConfig {
            workers_per_destination: 3,
            ..AppConfig::default()
        };
        config::save(&data_dir, &cfg).expect("seed config.json");

        let state = bootstrap_in(data_dir.clone()).expect("bootstrap_in should succeed");

        let loaded = tauri::async_runtime::block_on(async { state.config.read().await.clone() });
        assert_eq!(loaded.workers_per_destination, 3);
    }
}
