//! `SyncRuntime` — owns the watcher → stabilize → intake pipeline for the app's whole
//! lifetime (PLAN.md T-2.6, SPEC.md §2 "Fluxo de um arquivo", §6 watcher/queue
//! contracts).
//!
//! # Pieces
//!
//! - [`Wakers`] is owned here for the app's lifetime — it is cheap, `Send + Sync`, and
//!   every intake loop (initial + every restart) shares the same one so a worker parked
//!   on `Wakers::waiter` never misses a wake because a restart handed it a fresh, empty
//!   `Notify`. [`StabilizeConfig`] used to share that sentence, but the reasoning never
//!   applied to it: nothing parks on a `Copy` value struct. It is now *derived* from
//!   `watch.stabilize_seconds` on every watcher generation, which is what makes that
//!   setting mean anything at all — it was previously validated, persisted, shown in
//!   Settings and hardwired to `StabilizeConfig::default()` at runtime.
//! - `watcher` / `intake_task` are the *current* watcher handle and the task running
//!   [`run_intake_loop`] against its channel. Both are replaced wholesale by
//!   [`SyncRuntime::restart_watcher`] (`save_config` on a `watch.path`/`recursive`
//!   change, or `pick_folder`) — the old watcher is stopped and the old loop aborted
//!   before the new ones start, so there is never more than one of each alive.
//! - `paused` mirrors the current [`WatcherHandle::is_paused`] flag outside of the
//!   `Option` so [`SyncRuntime::build_app_status`] can report it even for the brief
//!   window right after a restart, and so a restart triggered while paused (e.g. the
//!   user changes the folder without resuming first) can carry the pause state
//!   forward onto the new watcher instead of silently un-pausing it.
//!
//! # Deviations from the task's literal pseudocode
//!
//! - `start`/`restart_watcher` take `self: &Arc<Self>` rather than `&self`: both spawn
//!   `'static` tasks (the initial rescan, and — per intake — the `job-updated` /
//!   `status-changed` emitters) that need to call back into `build_app_status`, which
//!   reads `self.paused`. An `Arc<SyncRuntime>` clone is the only way to hand a
//!   `'static` task a live reference to `self`. `pause`/`resume`/`build_app_status`
//!   stay on plain `&self` since nothing they do outlives the call.
//! - `start`/`restart_watcher` take explicit `app: AppHandle`, `repo: Arc<Mutex<Repo>>`
//!   parameters (the task's pseudocode left this as `state_parts…`) plus, for `start`
//!   only, `config: Arc<async_runtime::RwLock<AppConfig>>` to read the boot-time
//!   `watch` section. `resume` additionally takes `repo`/`watch` for its rescan.
//! - [`run_intake_loop`] takes an extra `watch: WatchConfig` parameter beyond the
//!   task's example signature — `queue::passes_filters` requires it for the
//!   extension/size gate, so there is no way to filter without it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use osystems_sync_core::config::{AppConfig, WatchConfig};
use osystems_sync_core::credentials::{Credentials, KeyringStore, SecretStore};
use osystems_sync_core::health::{self, HealthHandle, UploaderProvider};
use osystems_sync_core::power::{self, KeepAwake, ResumeSink};
use osystems_sync_core::queue::{self, IntakeOutcome, QueueError, Wakers};
use osystems_sync_core::rescan::{self, RescanError, RescanReport};
use osystems_sync_core::stabilize::{self, StabilizeConfig};
use osystems_sync_core::state::{
    AppStatus, Destination, DestinationHealth, DestinationsHealth, Repo, UpsertOutcome,
};
use osystems_sync_core::throttle::NightModeScheduler;
use osystems_sync_core::uploaders::gdrive::auth::{from_credentials, TokenProvider};
use osystems_sync_core::uploaders::gdrive::{GDriveOptions, GDriveUploader};
use osystems_sync_core::uploaders::s3::{S3Options, S3Uploader, StateSink};
use osystems_sync_core::uploaders::Uploader;
use osystems_sync_core::watcher::{self, WatcherHandle};
use osystems_sync_core::worker::{Throttles, Uploaders, WorkerDeps, WorkerPool};
use tauri::async_runtime::{JoinHandle, Mutex, RwLock};
use tauri::AppHandle;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::events;

/// SPEC.md §2: "notify event → debounce 2s → stabilize() → ...".
const WATCH_DEBOUNCE: Duration = Duration::from_secs(2);
/// Bounded so a pathological burst of filesystem events applies backpressure to the
/// watcher's debouncer thread instead of growing an unbounded in-memory queue.
const WATCH_CHANNEL_CAPACITY: usize = 1024;

/// Owns the watcher/intake pipeline. Constructed once at boot ([`SyncRuntime::new`]),
/// stored as `AppState.runtime: Arc<SyncRuntime>`, started from `lib.rs`'s `.setup()`.
pub struct SyncRuntime {
    wakers: Arc<Wakers>,
    watcher: Mutex<Option<WatcherHandle>>,
    intake_task: Mutex<Option<JoinHandle<()>>>,
    paused: AtomicBool,
    /// Derived from `watch.stabilize_seconds` on every watcher generation
    /// (`spawn_watcher_and_intake`), so the live intake loop and every rescan
    /// caller can never disagree about it. A plain `std` mutex: it is only
    /// ever set-or-copied, never held across an `.await` — same reasoning as
    /// `health_cancel` below.
    stabilize: StdMutex<StabilizeConfig>,
    /// Shared with `AppState::health` (literally the same `Arc<OnceLock<..>>`, set
    /// exactly once by the async boot task after [`boot_workers`] runs `health::spawn`)
    /// — T-3.9. Storing the cell here, instead of threading a `health` parameter
    /// through `start`/`restart_watcher`/`spawn_watcher_and_intake` down to every
    /// `build_app_status` call site, means `build_app_status` just reads `self.health`
    /// directly and none of those signatures need to change.
    health: Arc<OnceLock<HealthHandle>>,
    /// The `CancellationToken` passed to `health::spawn` in `SyncRuntime::boot_workers`,
    /// kept around so `SyncRuntime::cancel_health` (`lib.rs`'s `RunEvent::ExitRequested`
    /// handler) can stop the periodic probe loop on the way out. `None` until
    /// `boot_workers` has run -- a plain `std::sync::Mutex` since every access is a
    /// same-thread set-or-cancel, never held across an `.await`.
    health_cancel: StdMutex<Option<CancellationToken>>,
}

/// Decides whether `OSYSTEMS_SYNC_S3_ENDPOINT` may override the S3 endpoint
/// (VULN-001): only in debug builds (`debug == true`, i.e.
/// `cfg!(debug_assertions)`), and only when the value looks like a local/dev
/// endpoint (`http://127.0.0.1...`, `http://localhost...`) or an explicit
/// `https://...` URL -- never a bare `http://` pointed at an arbitrary host,
/// which would let an attacker-controlled environment variable redirect
/// uploads (and the AWS credentials used to make them) to a server they
/// control. Release builds ignore the variable unconditionally, regardless of
/// its value. Pure so it is unit-testable without touching real env vars.
fn endpoint_override(raw: Option<String>, debug: bool) -> Option<String> {
    if !debug {
        return None;
    }
    raw.filter(|s| !s.is_empty()).filter(|s| {
        s.starts_with("http://127.0.0.1")
            || s.starts_with("http://localhost")
            || s.starts_with("https://")
    })
}

impl Default for SyncRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncRuntime {
    pub fn new() -> Self {
        Self::with_health(Arc::new(OnceLock::new()))
    }

    /// Like [`SyncRuntime::new`], but sharing a pre-existing health cell —
    /// `state::bootstrap_in` uses this so `AppState.health` and this runtime's
    /// internal `health` field are the same `Arc`, kept in sync for free.
    pub fn with_health(health: Arc<OnceLock<HealthHandle>>) -> Self {
        Self {
            wakers: Arc::new(Wakers::new()),
            watcher: Mutex::new(None),
            intake_task: Mutex::new(None),
            paused: AtomicBool::new(false),
            stabilize: StdMutex::new(StabilizeConfig::default()),
            health,
            health_cancel: StdMutex::new(None),
        }
    }

    /// Starts the pipeline for the current `config.watch` (if `path` is set and exists)
    /// and kicks off the startup rescan (SPEC.md §2/§6) in a spawned task, emitting
    /// `status-changed` once it (and the follow-up status snapshot) complete.
    pub async fn start(
        self: &Arc<Self>,
        app: AppHandle,
        repo: Arc<StdMutex<Repo>>,
        config: Arc<RwLock<AppConfig>>,
    ) {
        let watch = config.read().await.watch.clone();
        self.spawn_watcher_and_intake(app.clone(), repo.clone(), watch.clone())
            .await;

        let runtime = self.clone();
        tauri::async_runtime::spawn(async move {
            if watch.path.is_some() {
                // `rescan()` itself logs `tracing::info!(?report, "varredura concluída")`
                // (core::rescan module docs) — that is what the manual verify step
                // greps for in app.log.
                if let Err(err) = rescan::rescan(
                    repo.clone(),
                    &runtime.wakers,
                    &watch,
                    &runtime.stabilize_config(),
                )
                .await
                {
                    tracing::warn!(error = %err, "varredura de inicialização falhou");
                }
            }

            match runtime.build_app_status(&repo).await {
                Ok(status) => events::emit_status_changed(&app, &status),
                Err(err) => {
                    tracing::warn!(error = %err, "falha ao construir o status do app após a inicialização")
                }
            }
        });
    }

    /// Stops the current watcher/intake loop (if any) and starts fresh ones against
    /// `new_watch`. Used by `save_config` when `watch.path`/`recursive`/`extensions`/
    /// `stabilize_seconds` changed, and by `pick_folder` right after persisting the
    /// newly chosen folder.
    pub async fn restart_watcher(
        self: &Arc<Self>,
        app: AppHandle,
        repo: Arc<StdMutex<Repo>>,
        new_watch: WatchConfig,
    ) {
        self.spawn_watcher_and_intake(app, repo, new_watch).await;
    }

    /// Makes the watcher discard events without tearing down the OS-level watch
    /// (SPEC.md §6, RF12). Never touches in-flight worker uploads.
    pub async fn pause(&self) {
        if let Some(handle) = self.watcher.lock().await.as_ref() {
            handle.pause();
        }
        self.paused.store(true, Ordering::SeqCst);
    }

    /// Resumes the watcher and — per SPEC.md §6 / RF-005 — runs a `rescan()` to catch
    /// anything that landed in the folder while paused (events were discarded, not
    /// queued, so this is the only way to recover them).
    pub async fn resume(
        &self,
        repo: Arc<StdMutex<Repo>>,
        watch: WatchConfig,
    ) -> Result<RescanReport, RescanError> {
        if let Some(handle) = self.watcher.lock().await.as_ref() {
            handle.resume();
        }
        self.paused.store(false, Ordering::SeqCst);
        rescan::rescan(repo, &self.wakers, &watch, &self.stabilize_config()).await
    }

    /// Whether the watcher is currently paused (mirrors [`WatcherHandle::is_paused`];
    /// `false` when no watcher is running at all).
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// The shared [`Wakers`] used by every intake loop and rescan — exposed so
    /// `commands::queue::rescan` can drive a manual rescan through the same instance.
    pub fn wakers(&self) -> &Arc<Wakers> {
        &self.wakers
    }

    /// The [`StabilizeConfig`] the current watcher generation was spawned with
    /// — exposed so a manual rescan (`commands::queue::rescan`) uses the same
    /// stabilization settings as the live watcher/intake pipeline. Returned by
    /// value; `StabilizeConfig` is `Copy`.
    pub fn stabilize_config(&self) -> StabilizeConfig {
        match self.stabilize.lock() {
            Ok(guard) => *guard,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }

    /// Single-query `AppStatus` snapshot for `get_status` / `status-changed`.
    /// `destinations` comes from `self.health`'s live [`HealthHandle::snapshot`] once
    /// `boot_workers` has run; until then (the brief window between `app.manage(state)`
    /// and that async boot task completing) it falls back to an all-offline snapshot —
    /// never blocks/panics waiting for health to exist.
    pub async fn build_app_status(
        &self,
        repo: &Arc<StdMutex<Repo>>,
    ) -> Result<AppStatus, QueueError> {
        let counts = queue::with_repo(repo.clone(), |r| r.status_counts()).await?;

        let destinations = match self.health.get() {
            Some(handle) => handle.snapshot().await,
            None => offline_destinations_health(),
        };

        Ok(AppStatus {
            watcher_paused: self.is_paused(),
            destinations,
            counts_by_status: counts,
            core_version: osystems_sync_core::version().to_string(),
            build_target: format!(
                "Tauri {} • {} {}",
                tauri::VERSION,
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
        })
    }

    /// Tears down the current watcher + intake loop (if any) and, if `watch.path` is
    /// set and points at an existing directory, starts new ones. A missing/unset path
    /// is not an error — it just means "not watching yet" (RF-001: no folder chosen).
    async fn spawn_watcher_and_intake(
        self: &Arc<Self>,
        app: AppHandle,
        repo: Arc<StdMutex<Repo>>,
        watch: WatchConfig,
    ) {
        if let Some(old) = self.watcher.lock().await.take() {
            old.stop().await;
        }
        if let Some(old_task) = self.intake_task.lock().await.take() {
            old_task.abort();
        }

        // Stored before the `points_at_dir` bail below: a user who sets
        // "seconds of stabilization" *before* picking a folder must still get
        // that value once they pick one.
        let stabilize = StabilizeConfig::from_watch(&watch);
        match self.stabilize.lock() {
            Ok(mut guard) => *guard = stabilize,
            Err(poisoned) => *poisoned.into_inner() = stabilize,
        }

        let points_at_dir = watch
            .path
            .as_deref()
            .map(|p| Path::new(p).is_dir())
            .unwrap_or(false);
        if !points_at_dir {
            tracing::info!("watch.path não configurado ou ausente; watcher não iniciado");
            return;
        }

        let (tx, rx) = mpsc::channel::<PathBuf>(WATCH_CHANNEL_CAPACITY);
        let handle = match watcher::spawn_watcher(watch.clone(), WATCH_DEBOUNCE, tx) {
            Ok(handle) => handle,
            Err(err) => {
                tracing::warn!(error = %err, "falha ao iniciar o watcher do sistema de arquivos");
                return;
            }
        };
        // Carry a paused state forward across a restart (e.g. folder changed while
        // paused) instead of silently resuming.
        if self.is_paused() {
            handle.pause();
        }
        *self.watcher.lock().await = Some(handle);

        let runtime = self.clone();
        let wakers = self.wakers.clone();
        let app_for_cb = app.clone();
        let repo_for_cb = repo.clone();

        let join = tauri::async_runtime::spawn(run_intake_loop(
            rx,
            repo,
            wakers,
            watch,
            stabilize,
            move |outcome: IntakeOutcome| {
                if !matches!(
                    outcome.outcome,
                    UpsertOutcome::Created | UpsertOutcome::Rehashed
                ) {
                    return;
                }
                let runtime = runtime.clone();
                let app = app_for_cb.clone();
                let repo = repo_for_cb.clone();
                tauri::async_runtime::spawn(async move {
                    emit_job_and_status_after_intake(&runtime, &app, &repo, outcome.file_id).await;
                });
            },
        ));
        *self.intake_task.lock().await = Some(join);
    }

    /// Read-only access to the health monitor once [`SyncRuntime::boot_workers`] has
    /// run — used by `events::AppEvents::auth_required` to forward a worker-observed
    /// auth failure into `HealthHandle::set_auth_required` (which is what actually
    /// flips `AppStatus.destinations`; `AppEvents` itself never mutates health state
    /// directly, only reads/subscribes to it). `None` during the brief startup window
    /// before `boot_workers` completes -- callers must treat that as "not tracked yet",
    /// not as an error.
    pub(crate) fn health_handle(&self) -> Option<&HealthHandle> {
        self.health.get()
    }

    /// Stops the health monitor's periodic probe loop (SPEC.md §7 graceful shutdown,
    /// PLAN.md T-3.9) -- paired with `AppState.pool`'s own `WorkerPool::shutdown` in
    /// `lib.rs`'s `RunEvent::ExitRequested` handler. A no-op if `boot_workers` never ran
    /// (health never started); safe to call more than once (`CancellationToken::cancel`
    /// is idempotent).
    pub fn cancel_health(&self) {
        let guard = self
            .health_cancel
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(cancel) = guard.as_ref() {
            cancel.cancel();
        }
    }

    /// (Re)builds the S3 uploader from the current `config.s3` plus whatever AWS
    /// credentials are stored, and swaps it into `uploaders.write().await.s3` --
    /// `None` when S3 is disabled or no complete credential pair is stored yet (SPEC.md
    /// §6: a worker with no uploader for its destination just idles instead of
    /// claiming). Called once at boot ([`SyncRuntime::boot_workers`]) and again, at
    /// runtime, by `commands::credentials::set_credential` (once both AWS parts are
    /// present) and `commands::config::save_config` (whenever `config.s3` itself
    /// changed) -- both share this one code path so the uploader is always built the
    /// same way, regardless of what triggered the rebuild.
    ///
    /// GDrive is deliberately out of scope: its uploader wiring is a separate task
    /// (PLAN.md T-4.x); `uploaders.gdrive` is left untouched here.
    ///
    /// Generic over `S: SecretStore` (rather than pinned to [`KeyringStore`]) purely so
    /// it is unit-testable against [`osystems_sync_core::credentials::MemoryStore`]
    /// without touching the OS keyring — every real caller still passes
    /// `Arc<Credentials<KeyringStore>>`, inferred from `AppState::credentials`.
    pub async fn rebuild_s3_uploader<S: SecretStore>(
        &self,
        repo: &Arc<StdMutex<Repo>>,
        config: &Arc<RwLock<AppConfig>>,
        credentials: &Arc<Credentials<S>>,
        throttles: &Throttles,
        uploaders: &Arc<RwLock<Uploaders>>,
    ) -> Result<(), AppError> {
        let cfg = config.read().await.clone();
        let aws = credentials.get_aws()?;

        let built: Option<Arc<dyn Uploader>> = match (cfg.s3.enabled, aws) {
            (true, Some(creds)) => {
                // `OSYSTEMS_SYNC_S3_ENDPOINT`: dev/test override to point at a local
                // S3-compatible server (e.g. MinIO) instead of real AWS. Unset in
                // production, where `endpoint_url: None` makes the AWS SDK use its
                // normal `s3.{region}.amazonaws.com` endpoint and virtual-hosted-style
                // addressing. VULN-001: only honored in debug builds and only for
                // local/https-looking values (`endpoint_override`) -- a release
                // build must never let an env var redirect uploads (and the AWS
                // credentials used to make them) to an attacker-controlled host.
                let raw_endpoint = std::env::var("OSYSTEMS_SYNC_S3_ENDPOINT").ok();
                if !cfg!(debug_assertions) && raw_endpoint.as_deref().is_some_and(|s| !s.is_empty())
                {
                    tracing::warn!(
                        "OSYSTEMS_SYNC_S3_ENDPOINT está definida mas é ignorada em builds de release"
                    );
                }
                let endpoint_url = endpoint_override(raw_endpoint, cfg!(debug_assertions));
                let force_path_style = endpoint_url.is_some();
                let opts = S3Options {
                    endpoint_url,
                    force_path_style,
                    skip_head_check: false,
                };

                let sink = state_sink_for(repo.clone());
                let uploader = S3Uploader::new(cfg.s3.clone(), creds, throttles.s3.clone(), opts)
                    .await?
                    .with_state_sink(sink);
                Some(Arc::new(uploader))
            }
            _ => None,
        };

        uploaders.write().await.s3 = built;
        Ok(())
    }

    /// Mirrors [`Self::rebuild_s3_uploader`] for Google Drive (PLAN.md T-4.6/4.7):
    /// rebuilds `uploaders.gdrive` from the current config + stored Service Account
    /// JSON, or clears it when GDrive is disabled or has no credentials yet. Called
    /// once at boot ([`Self::boot_workers`]) and again from
    /// `commands::credentials::pick_service_account_file` and
    /// `commands::config::save_config` (whenever `config.gdrive` changed).
    ///
    /// Generic over `S: SecretStore` for the same reason as `rebuild_s3_uploader`:
    /// unit-testable against `MemoryStore` without touching the OS keyring.
    pub async fn rebuild_gdrive_uploader<S: SecretStore>(
        &self,
        repo: &Arc<StdMutex<Repo>>,
        config: &Arc<RwLock<AppConfig>>,
        credentials: &Arc<Credentials<S>>,
        throttles: &Throttles,
        uploaders: &Arc<RwLock<Uploaders>>,
    ) -> Result<(), AppError> {
        let cfg = config.read().await.clone();
        let sa = from_credentials(credentials.as_ref())?;

        let built: Option<Arc<dyn Uploader>> = match (cfg.gdrive.enabled, sa) {
            (true, Some(sa)) => {
                let client_email = sa.client_email.clone();
                let tokens = TokenProvider::new(sa, None).await?;
                let sink = state_sink_for(repo.clone());
                let uploader = GDriveUploader::new(
                    cfg.gdrive.clone(),
                    tokens,
                    client_email,
                    throttles.gdrive.clone(),
                    GDriveOptions {
                        state_sink: Some(sink),
                        ..Default::default()
                    },
                );
                Some(Arc::new(uploader))
            }
            _ => None,
        };

        uploaders.write().await.gdrive = built;
        Ok(())
    }

    /// Boots the worker pool + health monitor (PLAN.md T-3.9). Must run exactly once,
    /// from `lib.rs`'s `.setup()` async boot task, after an `AppHandle` exists (which is
    /// why this isn't part of `state::bootstrap_in` -- see that function's module
    /// docs). Idempotency is enforced by the two `OnceLock`s it fills (`self.health`,
    /// `pool_cell`): a second call logs an error and leaves the existing pool/health
    /// running rather than panicking or silently replacing them out from under
    /// in-flight uploads.
    ///
    /// # Deviation from PLAN.md T-3.9's literal bootstrap order
    ///
    /// The task lists `recover_on_boot -> WorkerPool::start -> health::spawn`; this
    /// runs `health::spawn` *first*, right after `Uploaders`/`AppEvents` are built.
    /// Starting the pool before health exists would leave a window where a worker's
    /// very first `auth_required` callback (`events::AppEvents::auth_required` calling
    /// `HealthHandle::set_auth_required`) finds `self.health` empty and silently drops
    /// the state update -- the `auth-required` *event* would still fire (that part
    /// doesn't depend on `self.health`), but `AppStatus.destinations.*.auth_required`
    /// would stay stale until the next 60s probe. Spawning health first closes that
    /// window; the monitor's own probe cadence is unaffected either way.
    ///
    /// `clippy::too_many_arguments`: this is `lib.rs`'s one-shot boot wiring — every
    /// parameter is a distinct `Arc` the boot task already holds from `AppState`, none
    /// of them naturally group into a smaller type, and it is called from exactly one
    /// call site. A bespoke "boot args" struct would only move the same eight fields
    /// one level down for no readability gain.
    #[allow(clippy::too_many_arguments)]
    pub async fn boot_workers(
        self: &Arc<Self>,
        app: AppHandle,
        repo: Arc<StdMutex<Repo>>,
        config: Arc<RwLock<AppConfig>>,
        credentials: Arc<Credentials<KeyringStore>>,
        uploaders: Arc<RwLock<Uploaders>>,
        throttles: Throttles,
        pool_cell: Arc<OnceLock<Arc<WorkerPool>>>,
        keep_awake: Arc<KeepAwake>,
        night_mode: Arc<NightModeScheduler>,
        resume_cancel: CancellationToken,
        night_mode_cancel: CancellationToken,
    ) {
        if let Err(err) = self
            .rebuild_s3_uploader(&repo, &config, &credentials, &throttles, &uploaders)
            .await
        {
            tracing::warn!(
                error = %err,
                "boot: falha ao construir o uploader do S3; uploads do S3 ficam offline até que credenciais/configuração sejam corrigidas"
            );
        }

        if let Err(err) = self
            .rebuild_gdrive_uploader(&repo, &config, &credentials, &throttles, &uploaders)
            .await
        {
            tracing::warn!(
                error = %err,
                "boot: falha ao construir o uploader do GDrive; uploads do GDrive ficam offline até que credenciais/configuração sejam corrigidas"
            );
        }

        // Keep-awake + night mode (PLAN.md T-5.5, SPEC.md §6, RF-093/095/096): applied
        // from the config snapshot at boot, then kept in sync at runtime by
        // `commands::config::save_config`/`commands::qos::set_qos`.
        let cfg_snapshot = config.read().await.clone();
        keep_awake.set(cfg_snapshot.keep_awake);
        if let Err(err) = night_mode.configure(
            cfg_snapshot.qos.night_mode.enabled,
            &cfg_snapshot.qos.night_mode.start,
            &cfg_snapshot.qos.night_mode.end,
        ) {
            tracing::warn!(
                error = %err,
                "boot: janela do modo noturno inválida na configuração; modo noturno deixado desabilitado"
            );
        }
        night_mode.spawn(night_mode_cancel, || chrono::Local::now().time());

        // Resume detection (RF-094): rescans + refreshes `AppStatus` once the OS wakes
        // from sleep, so jobs that would otherwise wait for the next watcher event or
        // manual rescan pick up immediately.
        let resume_sink: Arc<dyn ResumeSink> = Arc::new(ResumeDetectorSink {
            app: app.clone(),
            repo: repo.clone(),
            config: config.clone(),
            runtime: self.clone(),
        });
        power::spawn_resume_detector(Duration::from_secs(60), resume_sink, resume_cancel);

        let events = crate::events::AppEvents::new(app, repo.clone(), self.clone());

        let provider: Arc<dyn UploaderProvider> = Arc::new(UploaderProviderImpl {
            uploaders: uploaders.clone(),
        });
        let cancel = CancellationToken::new();
        let health_handle = health::spawn(
            provider,
            events.clone() as Arc<dyn health::HealthSink>,
            Duration::from_secs(60),
            cancel.clone(),
        );
        if self.health.set(health_handle).is_err() {
            tracing::error!(
                "boot_workers chamado mais de uma vez; mantendo o monitor de saúde existente"
            );
        }
        *self
            .health_cancel
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(cancel);

        let deps = WorkerDeps {
            repo,
            wakers: self.wakers.clone(),
            config,
            throttles,
            uploaders,
            events: events as Arc<dyn osystems_sync_core::worker::WorkerEvents>,
        };

        let recovered = WorkerPool::recover_on_boot(deps.clone()).await;
        if recovered > 0 {
            tracing::info!(
                recovered,
                "recuperação de falha: jobs órfãos em upload voltaram para pendente"
            );
        }

        let pool = WorkerPool::start(deps).await;
        if pool_cell.set(pool).is_err() {
            tracing::error!(
                "boot_workers chamado mais de uma vez; mantendo o pool de workers existente"
            );
        }
    }
}

/// [`health::UploaderProvider`] over the live `Arc<RwLock<Uploaders>>` -- reads via
/// `try_read` rather than the (async) `.read().await` a sync trait method can't call:
/// the lock is only ever held write-side for a plain `Arc` swap
/// ([`SyncRuntime::rebuild_s3_uploader`]), never across an `.await`, so a contended
/// `try_read` is a sub-microsecond fluke, not a sign of a stuck writer -- worth
/// returning `None` for this probe cycle rather than blocking the health monitor's task
/// (which `RwLock::blocking_read` would do, and which `tokio::sync::RwLock` documents as
/// panicking if called from inside an async runtime context).
struct UploaderProviderImpl {
    uploaders: Arc<RwLock<Uploaders>>,
}

impl UploaderProvider for UploaderProviderImpl {
    fn uploader(&self, dest: Destination) -> Option<Arc<dyn Uploader>> {
        match self.uploaders.try_read() {
            Ok(guard) => match dest {
                Destination::S3 => guard.s3.clone(),
                Destination::GDrive => guard.gdrive.clone(),
            },
            Err(_) => {
                tracing::warn!(
                    ?dest,
                    "lock de uploaders disputado durante uma sonda de saúde; ignorando este ciclo"
                );
                None
            }
        }
    }
}

/// [`ResumeSink`] wired into [`power::spawn_resume_detector`] by
/// [`SyncRuntime::boot_workers`] (PLAN.md T-5.5, RF-094). On a detected resume-from-sleep
/// gap, runs the same manual rescan `tray.rs`'s `rescan_from_tray` triggers and then
/// refreshes `AppStatus` — jobs blocked on a stale watcher state pick back up
/// immediately instead of waiting for the next filesystem event.
struct ResumeDetectorSink {
    app: AppHandle,
    repo: Arc<StdMutex<Repo>>,
    config: Arc<RwLock<AppConfig>>,
    runtime: Arc<SyncRuntime>,
}

impl ResumeSink for ResumeDetectorSink {
    fn resumed(&self, gap: Duration) {
        let app = self.app.clone();
        let repo = self.repo.clone();
        let config = self.config.clone();
        let runtime = self.runtime.clone();
        tauri::async_runtime::spawn(async move {
            tracing::info!(
                gap_secs = gap.as_secs(),
                "retomada do sistema detectada; revarrendo"
            );

            let watch = config.read().await.watch.clone();
            match rescan::rescan(
                repo.clone(),
                &runtime.wakers,
                &watch,
                &runtime.stabilize_config(),
            )
            .await
            {
                Ok(report) => tracing::info!(
                    enqueued = report.enqueued,
                    "varredura de retomada concluída"
                ),
                Err(err) => tracing::warn!(error = %err, "varredura de retomada falhou"),
            }

            match runtime.build_app_status(&repo).await {
                Ok(status) => events::emit_status_changed(&app, &status),
                Err(err) => {
                    tracing::warn!(error = %err, "falha ao construir o status do app após a retomada")
                }
            }
        });
    }
}

/// Builds the `S3Uploader::with_state_sink` callback that persists multipart progress
/// into `jobs.remote_state` (SPEC.md §5) as it happens, so a crash mid-upload can
/// resume via `WorkerPool::recover_on_boot` next launch instead of restarting the whole
/// file. The sink itself is sync (`s3.rs`'s [`StateSink`] type), but resolving
/// `remote_name` to a `job_id` and persisting the state are both async (SQLite via
/// `queue::with_repo`), so each call spawns a fire-and-forget task -- the uploader keeps
/// making progress even if a particular write is slow or fails; worst case is a
/// slightly stale `remote_state`, which `recover_on_boot` already tolerates (it
/// re-derives current parts via a fresh `ListParts` call rather than trusting the
/// persisted list blindly).
fn state_sink_for(repo: Arc<StdMutex<Repo>>) -> StateSink {
    Arc::new(move |remote_name: &str, state: serde_json::Value| {
        let repo = repo.clone();
        let remote_name = remote_name.to_string();
        tauri::async_runtime::spawn(async move {
            let lookup_name = remote_name.clone();
            let job_id = match queue::with_repo(repo.clone(), move |r| {
                r.uploading_job_id_for_name(Destination::S3, &lookup_name)
            })
            .await
            {
                Ok(Some(id)) => id,
                Ok(None) => {
                    tracing::warn!(
                        remote_name = %remote_name,
                        "S3 state_sink: nenhum job em upload correspondeu a este nome remoto"
                    );
                    return;
                }
                Err(err) => {
                    tracing::warn!(
                        remote_name = %remote_name,
                        error = %err,
                        "S3 state_sink: falha ao resolver o job id para este nome remoto"
                    );
                    return;
                }
            };

            let json = state.to_string();
            if let Err(err) =
                queue::with_repo(repo, move |r| r.set_remote_state(&job_id, &json)).await
            {
                tracing::warn!(error = %err, "S3 state_sink: falha ao persistir remote_state");
            }
        });
    })
}

/// All-offline placeholder used by [`SyncRuntime::build_app_status`] during the brief
/// window before [`SyncRuntime::boot_workers`] has run `health::spawn` -- never blocks
/// or errors just because health tracking hasn't started yet.
fn offline_destinations_health() -> DestinationsHealth {
    DestinationsHealth {
        gdrive: DestinationHealth {
            online: false,
            auth_required: false,
            latency_ms: None,
        },
        s3: DestinationHealth {
            online: false,
            auth_required: false,
            latency_ms: None,
        },
    }
}

/// After a `Created`/`Rehashed` intake: fetches the fresh [`JobView`](osystems_sync_core::state::JobView)
/// for `file_id` and emits `job-updated`, then emits `status-changed` with the latest
/// counts. Both are best-effort — a failure here is logged and otherwise ignored
/// (CLAUDE.md / events.rs: a missed UI update is not worth taking the app down over).
async fn emit_job_and_status_after_intake(
    runtime: &Arc<SyncRuntime>,
    app: &AppHandle,
    repo: &Arc<StdMutex<Repo>>,
    file_id: String,
) {
    let lookup_id = file_id.clone();
    match queue::with_repo(repo.clone(), move |r| r.job_view_for_file(&lookup_id)).await {
        Ok(Some(job)) => events::emit_job_updated(app, &job),
        Ok(None) => {
            tracing::warn!(
                file_id,
                "job_view_for_file não encontrou nada logo após a entrada de arquivo"
            )
        }
        Err(err) => {
            tracing::warn!(file_id, error = %err, "job_view_for_file falhou após a entrada de arquivo")
        }
    }

    match runtime.build_app_status(repo).await {
        Ok(status) => events::emit_status_changed(app, &status),
        Err(err) => {
            tracing::warn!(error = %err, "falha ao construir o status do app após a entrada de arquivo")
        }
    }
}

/// The `stabilize → filter → intake` half of the pipeline (SPEC.md §2), factored out of
/// [`SyncRuntime`] so it can be unit-tested without Tauri: an in-memory [`Repo`] plus a
/// plain [`mpsc::channel`] stand in for the real watcher.
///
/// For every `path` received: a cheap `stat`-based prefilter runs first (size may still
/// be mid-write and wrong, but this drops obviously-ignored / wrong-extension paths
/// before paying for `wait_until_stable`'s polling loop), then `wait_until_stable`, then
/// the filters are re-checked against the *stabilized* size (the authoritative check —
/// SPEC.md §2's `stabilize() → ...` step), then `intake`. Every error (stat, stabilize,
/// intake) is `tracing::warn!`-logged and the loop continues with the next path — one
/// bad file must never take down the whole pipeline.
pub async fn run_intake_loop<F>(
    mut rx: mpsc::Receiver<PathBuf>,
    repo: Arc<StdMutex<Repo>>,
    wakers: Arc<Wakers>,
    watch: WatchConfig,
    stabilize: StabilizeConfig,
    on_intake: F,
) where
    F: Fn(IntakeOutcome) + Send + Sync + 'static,
{
    // VULN-003: canonicalized once per loop (not once per file) and passed
    // to every `queue::intake` call as the containment boundary -- a
    // symlink under `watch.path` that resolves outside it must be rejected
    // rather than hashed/enqueued. Falls back to the raw configured path
    // (matching `queue::intake`'s own canonicalize-with-fallback) if the
    // folder is unreadable right now; an empty/unset `watch.path` (should
    // not happen once the watcher is running -- `spawn_watcher_and_intake`
    // only starts it when `watch.path` points at an existing directory)
    // falls back to an empty root, so containment simply rejects everything
    // rather than silently trusting an unconfigured boundary.
    let root: PathBuf = match watch.path.as_deref() {
        Some(p) => osystems_sync_core::paths::canonicalize_clean(Path::new(p))
            .await
            .unwrap_or_else(|_| PathBuf::from(p)),
        None => PathBuf::new(),
    };

    while let Some(path) = rx.recv().await {
        let quick_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if !queue::passes_filters(&path, quick_size, &watch) {
            continue;
        }

        let stable = match stabilize::wait_until_stable(&path, &stabilize).await {
            Ok(stable) => stable,
            Err(err) => {
                tracing::warn!(path = %path.display(), error = %err, "estabilização falhou, ignorando arquivo");
                continue;
            }
        };

        if !queue::passes_filters(&path, stable.size, &watch) {
            continue;
        }

        match queue::intake(
            repo.clone(),
            &wakers,
            &root,
            path.clone(),
            stable.size,
            stable.mtime,
        )
        .await
        {
            Ok(outcome) => on_intake(outcome),
            Err(err) => {
                tracing::warn!(path = %path.display(), error = %err, "entrada de arquivo falhou, ignorando arquivo")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::AtomicU32;

    use tempfile::tempdir;

    fn shared_repo() -> Arc<StdMutex<Repo>> {
        Arc::new(StdMutex::new(
            Repo::open_in_memory().expect("open in-memory repo"),
        ))
    }

    fn fast_stabilize() -> StabilizeConfig {
        StabilizeConfig {
            stable_reads: 1,
            interval: Duration::from_millis(5),
            timeout: Duration::from_secs(5),
        }
    }

    #[tokio::test]
    async fn run_intake_loop_enqueues_a_valid_file_and_calls_back_once() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("report.pdf");
        std::fs::write(&file_path, b"hello world").expect("write file");

        let repo = shared_repo();
        let wakers = Arc::new(Wakers::new());
        let (tx, rx) = mpsc::channel::<PathBuf>(8);
        let watch = WatchConfig {
            path: Some(dir.path().to_string_lossy().into_owned()),
            ..WatchConfig::default()
        };

        let calls = Arc::new(AtomicU32::new(0));
        let calls_cb = calls.clone();

        let loop_handle = tokio::spawn(run_intake_loop(
            rx,
            repo.clone(),
            wakers,
            watch,
            fast_stabilize(),
            move |outcome: IntakeOutcome| {
                assert_eq!(outcome.outcome, UpsertOutcome::Created);
                calls_cb.fetch_add(1, Ordering::SeqCst);
            },
        ));

        tx.send(file_path.clone()).await.expect("send path");
        drop(tx); // closes rx once the one message is processed, ending the loop

        tokio::time::timeout(Duration::from_secs(5), loop_handle)
            .await
            .expect("run_intake_loop should finish once rx closes")
            .expect("run_intake_loop task should not panic");

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "callback fires exactly once"
        );

        let counts = queue::with_repo(repo, |r| r.status_counts())
            .await
            .expect("status_counts");
        assert_eq!(counts.pending, 2, "one pending job per destination");
    }

    #[tokio::test]
    async fn run_intake_loop_skips_a_tmp_file_and_never_calls_back() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("download.tmp");
        let mut f = std::fs::File::create(&file_path).expect("create tmp file");
        f.write_all(b"partial").expect("write tmp file");
        drop(f);

        let repo = shared_repo();
        let wakers = Arc::new(Wakers::new());
        let (tx, rx) = mpsc::channel::<PathBuf>(8);
        let watch = WatchConfig {
            path: Some(dir.path().to_string_lossy().into_owned()),
            ..WatchConfig::default()
        };

        let calls = Arc::new(AtomicU32::new(0));
        let calls_cb = calls.clone();

        let loop_handle = tokio::spawn(run_intake_loop(
            rx,
            repo.clone(),
            wakers,
            watch,
            fast_stabilize(),
            move |_outcome: IntakeOutcome| {
                calls_cb.fetch_add(1, Ordering::SeqCst);
            },
        ));

        tx.send(file_path).await.expect("send path");
        drop(tx);

        tokio::time::timeout(Duration::from_secs(5), loop_handle)
            .await
            .expect("run_intake_loop should finish once rx closes")
            .expect("run_intake_loop task should not panic");

        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "a .tmp path is never enqueued"
        );

        let counts = queue::with_repo(repo, |r| r.status_counts())
            .await
            .expect("status_counts");
        assert_eq!(counts.pending, 0);
    }

    #[tokio::test]
    async fn build_app_status_populates_core_version_and_counts() {
        let runtime = Arc::new(SyncRuntime::new());
        let repo = shared_repo();

        // A fixed RFC3339 literal (not `chrono::Utc::now()`): the app crate doesn't
        // depend on `chrono` directly and this task's constraints forbid touching
        // `Cargo.toml` to add it — any valid RFC3339 string works equally well here,
        // since this test only cares about `status_counts()` afterwards.
        queue::with_repo(repo.clone(), |r| {
            r.upsert_file_and_enqueue("C:/x/a.txt", "sha-1", 100, "2024-01-01T00:00:00Z")
        })
        .await
        .expect("seed one file");

        let status = runtime
            .build_app_status(&repo)
            .await
            .expect("build_app_status should succeed");

        assert_eq!(status.core_version, osystems_sync_core::version());
        assert_eq!(status.counts_by_status.pending, 2);
        assert!(!status.watcher_paused);
        assert!(!status.destinations.s3.online);
        assert!(!status.destinations.gdrive.online);
    }

    #[tokio::test]
    async fn pause_and_resume_toggle_is_paused() {
        let runtime = SyncRuntime::new();
        assert!(!runtime.is_paused());

        runtime.pause().await;
        assert!(runtime.is_paused());

        let repo = shared_repo();
        let watch = WatchConfig::default(); // no path configured
        let err = runtime
            .resume(repo, watch)
            .await
            .expect_err("resume's rescan should fail fast with no watch.path configured");
        assert!(matches!(err, RescanError::NoPath));
        // `resume` flips the flag before attempting the rescan, regardless of its
        // outcome — the watcher itself is what actually starts discarding/forwarding
        // events again, this flag is only the status-reporting mirror of that.
        assert!(!runtime.is_paused());
    }

    /// `rebuild_s3_uploader` generic over `S: SecretStore` (T-3.9) so it can be driven
    /// here by [`osystems_sync_core::credentials::MemoryStore`] instead of the real
    /// OS keyring. Only the two branches that never touch the network are exercised —
    /// `s3.enabled == false` and "enabled but no AWS credentials yet" — both of which
    /// must leave `uploaders.s3` untouched (`None`). The `(enabled, Some(creds))`
    /// branch does construct a real `S3Uploader` (a `HeadBucket` call against AWS),
    /// which core's own `uploaders::s3` test suite already covers with a mocked
    /// client; re-exercising it here would make this test depend on the network.
    #[tokio::test]
    async fn rebuild_s3_uploader_from_memory_store_stays_none_when_disabled_or_uncredentialed() {
        use osystems_sync_core::credentials::MemoryStore;
        use osystems_sync_core::throttle::Throttle;

        let runtime = SyncRuntime::new();
        let repo = shared_repo();
        let throttles = Throttles {
            s3: Throttle::new(0),
            gdrive: Throttle::new(0),
        };
        let uploaders = Arc::new(RwLock::new(Uploaders::default()));

        // s3.enabled == false, even with credentials present: stays None.
        let credentials = Arc::new(Credentials::new(MemoryStore::default()));
        credentials
            .set_aws("AKIAEXAMPLE", "secret-example")
            .expect("set_aws against MemoryStore");
        let config = Arc::new(RwLock::new(AppConfig::default()));
        assert!(
            !config.read().await.s3.enabled,
            "default S3Config is disabled"
        );

        runtime
            .rebuild_s3_uploader(&repo, &config, &credentials, &throttles, &uploaders)
            .await
            .expect("rebuild_s3_uploader must not error when s3 is disabled");
        assert!(uploaders.read().await.s3.is_none());

        // s3.enabled == true but no AWS credentials in the store: stays None.
        let uncredentialed = Arc::new(Credentials::new(MemoryStore::default()));
        let mut enabled_cfg = AppConfig::default();
        enabled_cfg.s3.enabled = true;
        enabled_cfg.s3.bucket = "example-bucket".to_string();
        let config = Arc::new(RwLock::new(enabled_cfg));

        runtime
            .rebuild_s3_uploader(&repo, &config, &uncredentialed, &throttles, &uploaders)
            .await
            .expect("rebuild_s3_uploader must not error when credentials are missing");
        assert!(uploaders.read().await.s3.is_none());
    }

    /// Throwaway 2048-bit RSA key (PKCS8 PEM), copied from
    /// `crates/core/src/uploaders/gdrive/auth.rs`'s own test fixture per PLAN.md
    /// T-4.6's test instructions — generated once for tests only, never used outside
    /// them.
    const TEST_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7Oyt3p8SoGEJP
bIsMFnQeQmYlR0NasbR3t6yCBKF6w/hzTMYKmWiBhxMoAtDWSpbNBwW0YoyEb9+m
hikW5GxE8wIBA0Xf1bBJuG50YA6wueZg/lDzcnKfEYGeD9VLoQC2fdN3AWvD1C33
UOowbMhZgslWxjHbRfICz989ZhZYB+RO6afajtdi2Fijju9sj+0v9GzhGZpL8RAa
NVjo5Dh6tVDJEOypTBqcx9IkeydD/eOQQA4ruhmsalHQWMfQUPJwyxEoj9dkvuj3
dSQcd4grxhniNPcPEKJ3xNp8JgbGxVVflJhU1Uu3GOMRtApwFuMwuPNQ7Fao0dYv
3DM4Q5CDAgMBAAECggEAGpEK72eyQKnEivGGCgPg8RmPKzNL7CwH7+7I1GAPgP8a
m1L694O3WkhjdaDEp4tzj18I4QS/bIaqmj3cybSaukPYS8go8Nmo3HpbeI7YDB0s
rSRDh0T83POby+TyPnt3bELkBQsCErTiMClejtrN/jQlGSIhQ+b5BTYgC93Bm4ww
7WZir/1E+5T8rqO3Ms3wmdC4dWTOQuDDWK0ljaLOqG0bW+t4eS6XHgn9CPS3DuJt
8wcmVlwPE2UXDkdv9r0DUk3D6FMcxU+2SdXehH/PqwzGk+NHrvn9lBlknF3vscs4
DvwATqvHoystxgI9LC52xIQbO+Uw0i82qUMUShrUTQKBgQDlIixIhRQpTCQGwiKc
/ZiDhO2ALMd5VxGY9xzsM+m2bTNSBy6ZgYEitZvy8txg3C0ExyyaBcqcRLeQ9gN7
DRvx9JEqsapjATfdqBt3JCv8AtjCxky44hkl3dFvjl5zLhu9O44Lcw0+2Y9KmrLz
+psqYkdCjoRUrYXA3pqzUOh9pwKBgQDRLzokZaKzBUGVgewLVp9DBgtDG09cw3m0
xbrVEUig0vt3EtpC/KcRAsRCLoVXlCzZssK+9qlKs63lUIMl29Uqftkm1t9LbDDI
Zgjj51wvM6DekTffx5XK8DEgzISsiBZtLPKyvmwUeVz9jVNb2U0K7n3iOG06T2Ok
ukYRbZAJxQKBgCU/vu8zIynrhNfMa5AV8ds/mtSBcxQYwXWahosnjVDow7UMEdlG
olWgLG/8ZzMf1/m0311Sn7NzwFvCgqJYaTiWR5snMsnRguF32K8vpC7dz5sqXYKY
zvnG66s0+8nBryS+L8NQutCC0baRG5JqJRtoyqjZPk39v4axKXkJKCJ1AoGBAKAD
lGJLLM3sc2K+Y6W4uVM3yF2pAmhfTzYtGuHpurjrK1jGnxcm1VV53E8T7wQzYKuW
xsn1PULbd2Y21Fudcc50AgBn1Z+IPzjMdHiBfk7NG32lcCxKLBd07N++Eq832o/h
FjYM2/g9bhi2htF3xCtcjAcESumT2RElPHwQZ2JRAoGAJMpVd75YW4YrgVdfV1Fl
AY61Vt5BYt7cXfwinvI5mc2BdT3Y7bzVuozxBmqbeo2ZyeCS+JEw3SeF5ckeBazT
Eqz5sfZq1hAiWB5WE9lc7DIUDxHEahm+RQwe4i939Sb5IRD9nvLRfvVFUL1OD/gu
h5sWg6OzGIQ6XQbasKq+W/8=
-----END PRIVATE KEY-----";

    fn valid_sa_json() -> String {
        serde_json::json!({
            "type": "service_account",
            "project_id": "my-project",
            "private_key_id": "abc123",
            "private_key": TEST_PRIVATE_KEY,
            "client_email": "sync@my-project.iam.gserviceaccount.com",
            "client_id": "123456789",
            "token_uri": "https://oauth2.googleapis.com/token",
        })
        .to_string()
    }

    /// `rebuild_gdrive_uploader` (T-4.6), same shape as the `rebuild_s3_uploader` test
    /// above: `gdrive.enabled == false` and "enabled but no Service Account stored yet"
    /// both leave `uploaders.gdrive` untouched, then a valid stored Service Account
    /// JSON does build a real `GDriveUploader` — `TokenProvider::new` only builds the
    /// authenticator (no network call happens until `.token()` is actually awaited),
    /// so this stays offline like the S3 test.
    #[tokio::test]
    async fn rebuild_gdrive_uploader_from_memory_store() {
        use osystems_sync_core::credentials::MemoryStore;
        use osystems_sync_core::throttle::Throttle;

        let runtime = SyncRuntime::new();
        let repo = shared_repo();
        let throttles = Throttles {
            s3: Throttle::new(0),
            gdrive: Throttle::new(0),
        };
        let uploaders = Arc::new(RwLock::new(Uploaders::default()));

        // gdrive.enabled == false, even with a stored Service Account: stays None.
        let credentials = Arc::new(Credentials::new(MemoryStore::default()));
        credentials
            .set_service_account_json(&valid_sa_json())
            .expect("set_service_account_json against MemoryStore");
        let config = Arc::new(RwLock::new(AppConfig::default()));
        assert!(
            !config.read().await.gdrive.enabled,
            "default GDriveConfig is disabled"
        );

        runtime
            .rebuild_gdrive_uploader(&repo, &config, &credentials, &throttles, &uploaders)
            .await
            .expect("rebuild_gdrive_uploader must not error when gdrive is disabled");
        assert!(uploaders.read().await.gdrive.is_none());

        // gdrive.enabled == true but no Service Account in the store: stays None.
        let uncredentialed = Arc::new(Credentials::new(MemoryStore::default()));
        let mut enabled_cfg = AppConfig::default();
        enabled_cfg.gdrive.enabled = true;
        let config = Arc::new(RwLock::new(enabled_cfg.clone()));

        runtime
            .rebuild_gdrive_uploader(&repo, &config, &uncredentialed, &throttles, &uploaders)
            .await
            .expect("rebuild_gdrive_uploader must not error when credentials are missing");
        assert!(uploaders.read().await.gdrive.is_none());

        // gdrive.enabled == true and a valid Service Account is stored: builds Some.
        let config = Arc::new(RwLock::new(enabled_cfg));
        runtime
            .rebuild_gdrive_uploader(&repo, &config, &credentials, &throttles, &uploaders)
            .await
            .expect("rebuild_gdrive_uploader must not error for a valid Service Account");
        assert!(
            uploaders.read().await.gdrive.is_some(),
            "enabled + a valid Service Account must build an uploader"
        );
    }

    // VULN-001: `endpoint_override` must only ever honor
    // `OSYSTEMS_SYNC_S3_ENDPOINT` in debug builds, and only for local/https
    // looking values -- release builds ignore it unconditionally.
    #[test]
    fn endpoint_override_in_debug_with_localhost_url_is_honored() {
        assert_eq!(
            endpoint_override(Some("http://localhost:9000".to_string()), true),
            Some("http://localhost:9000".to_string())
        );
        assert_eq!(
            endpoint_override(Some("http://127.0.0.1:9000".to_string()), true),
            Some("http://127.0.0.1:9000".to_string())
        );
        assert_eq!(
            endpoint_override(Some("https://minio.example.com".to_string()), true),
            Some("https://minio.example.com".to_string())
        );
    }

    #[test]
    fn endpoint_override_in_release_is_always_none() {
        assert_eq!(
            endpoint_override(Some("http://localhost:9000".to_string()), false),
            None
        );
        assert_eq!(
            endpoint_override(Some("https://minio.example.com".to_string()), false),
            None
        );
    }

    #[test]
    fn endpoint_override_rejects_arbitrary_http_host_even_in_debug() {
        assert_eq!(
            endpoint_override(Some("http://evil.example.com".to_string()), true),
            None
        );
    }

    #[test]
    fn endpoint_override_with_none_or_empty_is_none_in_debug_or_release() {
        assert_eq!(endpoint_override(None, true), None);
        assert_eq!(endpoint_override(Some(String::new()), true), None);
        assert_eq!(endpoint_override(None, false), None);
    }
}
