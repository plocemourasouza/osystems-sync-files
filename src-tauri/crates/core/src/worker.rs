//! `core::worker` — per-destination worker pool: retry/backoff, progress
//! reporting, pause/resume on auth failure, hot-reloadable concurrency and
//! crash recovery (PLAN.md T-3.7; SPEC.md §2 "Fluxo de um arquivo", §6
//! `queue.rs` / `worker.rs`, §7 IPC events; PRD.md RF-030/031/032/040,
//! RF-052, RNF-006/008).
//!
//! One [`worker_loop`] per `(destination, worker_idx)` claims the oldest due
//! `pending` job for its destination (`Repo::claim_next`), drives it through
//! an [`Uploader`], and reacts to the result:
//!
//! - success -> `mark_done`
//! - [`UploadError::Cancelled`] -> leave status as already set by whatever
//!   command triggered the cancel/pause; just log + notify
//! - [`UploadError::Auth`] -> back to `pending` *without* burning an
//!   attempt, destination paused, `auth_required` emitted
//! - [`UploadError::Transient`] / [`UploadError::Io`] -> `attempts += 1`;
//!   `mark_failed` at `retry.max_attempts`, otherwise `mark_retry` with
//!   [`backoff_delay`]
//! - [`UploadError::Permanent`] -> `mark_failed` immediately
//!
//! [`WorkerPool`] owns the spawned loops, a per-destination pause set and a
//! [`CancellationToken`] tree for graceful shutdown (SPEC.md "Sair", 30s
//! grace).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;
use tokio::time::Instant as TokioInstant;
use tokio_util::sync::CancellationToken;

use crate::config::AppConfig;
use crate::queue::{with_repo, QueueError, Wakers};
use crate::state::{
    Destination, JobRow, JobSide, JobStatus, JobView, Repo, Throughput, UploadProgress,
};
use crate::throttle::{Throttle, ThroughputMeter};
use crate::uploaders::{ProgressUpdate, UploadError, UploadRequest, Uploader};

// ---------------------------------------------------------------------
// Dependencies & events
// ---------------------------------------------------------------------

/// Per-destination throttles, shared by every worker of that destination
/// (SPEC.md §6 `throttle.rs`). Concrete `Uploader` implementations pace
/// their own body reads against these; `worker.rs` only reads
/// [`Throttle::limit_bps`] for the periodic `throughput` event.
#[derive(Clone)]
pub struct Throttles {
    pub s3: Arc<Throttle>,
    pub gdrive: Arc<Throttle>,
}

impl Throttles {
    fn get(&self, dest: Destination) -> Arc<Throttle> {
        match dest {
            Destination::S3 => self.s3.clone(),
            Destination::GDrive => self.gdrive.clone(),
        }
    }
}

/// The active uploader per destination, `None` until credentials are
/// configured/valid (SPEC.md §6: a worker with no uploader for its
/// destination idles instead of claiming). Swapped out at runtime when
/// credentials change (T-3.9 commands layer) — hence the outer `RwLock` in
/// [`WorkerDeps::uploaders`].
#[derive(Clone, Default)]
pub struct Uploaders {
    pub s3: Option<Arc<dyn Uploader>>,
    pub gdrive: Option<Arc<dyn Uploader>>,
}

impl Uploaders {
    fn get(&self, dest: Destination) -> Option<Arc<dyn Uploader>> {
        match dest {
            Destination::S3 => self.s3.clone(),
            Destination::GDrive => self.gdrive.clone(),
        }
    }
}

/// Event sink the worker pool reports to (SPEC.md §7 IPC): the Tauri app
/// implements this to forward each call to `app_handle.emit(...)`. Kept
/// trait-object-based so `core` never depends on `tauri`.
pub trait WorkerEvents: Send + Sync {
    /// A job's status/attempts/last_error changed — the receiver should
    /// re-fetch (or the caller already has) the row via `job_view_for_file`.
    fn job_updated(&self, job_id: &str);
    /// Bytes-sent progress for one in-flight job, throttled to at most
    /// once per 500ms (plus a final 100% update).
    fn upload_progress(&self, p: UploadProgress);
    /// Aggregate transfer rate, emitted ~once per second.
    fn throughput(&self, t: Throughput);
    /// A destination just failed with `UploadError::Auth` and was paused.
    fn auth_required(&self, dest: Destination, hint: String);
}

/// Everything a worker loop needs, cloned into every spawned task. Built
/// once by the Tauri app (T-3.9) and handed to [`WorkerPool::start`].
#[derive(Clone)]
pub struct WorkerDeps {
    pub repo: Arc<Mutex<Repo>>,
    pub wakers: Arc<Wakers>,
    pub config: Arc<RwLock<AppConfig>>,
    pub throttles: Throttles,
    pub uploaders: Arc<RwLock<Uploaders>>,
    pub events: Arc<dyn WorkerEvents>,
}

// ---------------------------------------------------------------------
// Backoff
// ---------------------------------------------------------------------

/// `min(base_secs * 2^attempt, 600s) * jitter_factor`, `jitter_factor` in
/// `[0.8, 1.2]` linearly mapped from `jitter` (clamped to `[0, 1]`) — SPEC.md
/// §6: "backoff exponencial com jitter de ±20%, teto de 10 minutos". Pure
/// and deterministic so it's unit-testable without a clock; production
/// callers supply `jitter` from [`random_jitter`].
pub fn backoff_delay(base_secs: u64, attempt: u32, jitter: f64) -> Duration {
    const CAP_SECS: u64 = 600;

    let exp = 2u64.checked_pow(attempt).unwrap_or(u64::MAX);
    let raw_secs = base_secs.saturating_mul(exp).min(CAP_SECS);

    let jitter = jitter.clamp(0.0, 1.0);
    let factor = 0.8 + jitter * 0.4;

    Duration::from_secs_f64(raw_secs as f64 * factor)
}

/// Cheap, dependency-free (no `rand` crate in `Cargo.toml`) pseudo-random
/// value in `[0, 1)` for [`backoff_delay`]'s jitter. Not cryptographic —
/// just needs to avoid every worker retrying in lockstep.
fn random_jitter() -> f64 {
    use std::hash::{Hash, Hasher};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (nanos, n, std::process::id()).hash(&mut hasher);
    (hasher.finish() as f64) / (u64::MAX as f64)
}

// ---------------------------------------------------------------------
// Clock (testable under `#[tokio::test(start_paused = true)]`)
// ---------------------------------------------------------------------

/// `next_attempt_at` is a wall-clock RFC3339 string compared lexicographically
/// by `Repo::claim_next`'s SQL. Anchoring it to `chrono::Utc::now()` directly
/// would make the retry/backoff tests wait for real wall-clock seconds even
/// under `#[tokio::test(start_paused = true)]` (`tokio::time::pause`/`advance`
/// only affect `tokio::time::Instant`, never `chrono`/`SystemTime`).
///
/// Instead we anchor once to `(Utc::now(), tokio::time::Instant::now())` and
/// derive "now" from the *tokio* clock's elapsed time. In production this
/// tracks the wall clock just as accurately as `Utc::now()` would (the tokio
/// clock free-runs in real time whenever it isn't paused); in tests it lets
/// `tokio::time::advance(...)` fast-forward retry scheduling deterministically.
static CLOCK_EPOCH: OnceLock<(DateTime<Utc>, TokioInstant)> = OnceLock::new();

fn worker_now() -> DateTime<Utc> {
    let (epoch_dt, epoch_instant) = *CLOCK_EPOCH.get_or_init(|| (Utc::now(), TokioInstant::now()));
    let elapsed = TokioInstant::now().saturating_duration_since(epoch_instant);
    epoch_dt + chrono::Duration::from_std(elapsed).unwrap_or_else(|_| chrono::Duration::zero())
}

// ---------------------------------------------------------------------
// Pause set (DashMap-free: a plain Mutex<HashSet<Destination>>, shared)
// ---------------------------------------------------------------------

/// Deviation from the literal spec text: `paused_dest` is stored as
/// `Arc<Mutex<HashSet<Destination>>>` (wrapped here) rather than an
/// unwrapped `Mutex<HashSet<Destination>>` field directly on
/// [`WorkerPool`], so [`worker_loop`]/`run_job` (spawned as free-standing
/// tasks, not methods) can share it without `WorkerPool` needing a
/// `Weak<Self>`/`Arc::new_cyclic` self-reference. `WorkerPool` still exposes
/// `pause_destination`/`resume_destination`/`is_paused` as its own methods
/// over the same shared set — no `DashMap`, no extra crate.
#[derive(Clone, Default)]
pub struct PausedSet(Arc<Mutex<HashSet<Destination>>>);

impl PausedSet {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(HashSet::new())))
    }

    fn pause(&self, dest: Destination) {
        self.0
            .lock()
            .expect("paused-set mutex poisoned")
            .insert(dest);
    }

    fn resume(&self, dest: Destination) {
        self.0
            .lock()
            .expect("paused-set mutex poisoned")
            .remove(&dest);
    }

    fn is_paused(&self, dest: Destination) -> bool {
        self.0
            .lock()
            .expect("paused-set mutex poisoned")
            .contains(&dest)
    }
}

// ---------------------------------------------------------------------
// In-flight job registry (T-5.2: pause/resume a single job)
// ---------------------------------------------------------------------

/// `job_id -> its per-run CancellationToken`, shared across a destination's
/// workers so [`WorkerPool::pause_job`] can cancel a currently-`uploading`
/// job's transfer immediately, rather than waiting up to 1s for `run_job`'s
/// watchdog (`should_abort`) to notice a status flip to `paused` on its own.
///
/// Same `Arc<Mutex<..>>`-wrapped-newtype shape as [`PausedSet`] and for the
/// same reason: `run_job` is a free-standing spawned task (not a
/// `WorkerPool` method), so this has to be `Clone` and threaded through
/// `worker_loop`/`run_job` as a parameter instead of living unwrapped on
/// `WorkerPool`.
#[derive(Clone, Default)]
pub struct Inflight(Arc<Mutex<HashMap<String, CancellationToken>>>);

impl Inflight {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(HashMap::new())))
    }

    /// Registers `job_id`'s cancellation token. Called by `run_job` right
    /// before it hands control to the `Uploader`.
    fn insert(&self, job_id: String, token: CancellationToken) {
        self.0
            .lock()
            .expect("inflight mutex poisoned")
            .insert(job_id, token);
    }

    /// Deregisters `job_id`. Called by `run_job` right after `upload()`
    /// returns, whatever the outcome — a job is only "in-flight" while its
    /// `Uploader::upload` call is actually running.
    fn remove(&self, job_id: &str) {
        self.0
            .lock()
            .expect("inflight mutex poisoned")
            .remove(job_id);
    }

    /// Cancels `job_id`'s token if it is currently in-flight. Returns
    /// whether a token was found (i.e. the job *was* in-flight).
    fn cancel(&self, job_id: &str) -> bool {
        match self.0.lock().expect("inflight mutex poisoned").get(job_id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }
}

/// Per-destination throughput meters, shared across a destination's workers
/// so `throughput_ticker` reports one aggregate rate per destination
/// regardless of `workers_per_destination`. Not part of [`WorkerDeps`] (the
/// spec's public dependency bag) — it's pool-lifecycle state, threaded
/// through `worker_loop`/`run_job` parameters like [`PausedSet`].
#[derive(Clone, Default)]
pub struct Meters {
    s3: Arc<ThroughputMeter>,
    gdrive: Arc<ThroughputMeter>,
}

impl Meters {
    fn new() -> Self {
        Self {
            s3: Arc::new(ThroughputMeter::new()),
            gdrive: Arc::new(ThroughputMeter::new()),
        }
    }

    fn get(&self, dest: Destination) -> Arc<ThroughputMeter> {
        match dest {
            Destination::S3 => self.s3.clone(),
            Destination::GDrive => self.gdrive.clone(),
        }
    }
}

// ---------------------------------------------------------------------
// WorkerPool
// ---------------------------------------------------------------------

/// Owns every spawned worker loop plus the shared cancellation/pause state.
/// `DashMap`-free by design (task instructions): plain `Mutex`-guarded
/// collections, since none of them are hot enough to need lock-free maps.
pub struct WorkerPool {
    handles: Mutex<HashMap<Destination, Vec<JoinHandle<()>>>>,
    cancel: CancellationToken,
    paused_dest: PausedSet,
    inflight: Inflight,
    deps: WorkerDeps,
    meters: Meters,
    throughput_handle: Mutex<Option<JoinHandle<()>>>,
}

/// Errors from [`WorkerPool::pause_job`] / [`WorkerPool::resume_job`]
/// (T-5.2; PLAN.md RF-037/RF-063).
#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    /// No `jobs` row has this id.
    #[error("job not found: {0}")]
    NotFound(String),
    /// The job exists but isn't in a status the requested transition
    /// accepts (e.g. `resume` on a `done`/`failed`/`cancelled` job, or
    /// `pause` on one already terminal).
    #[error("job {0} cannot transition from status {1:?}")]
    InvalidTransition(String, JobStatus),
    /// The underlying `state.db` query failed.
    #[error("state error: {0}")]
    State(#[from] QueueError),
}

/// Resolves which side (`s3`/`gdrive`) of a `JobView` a given `jobs.id`
/// belongs to — `job_view_for_job` returns the whole file's pair, but
/// `pause_job`/`resume_job` only ever care about (and mutate) the one job
/// the caller named. Returns `None` if `job_id` matches neither side, which
/// should be unreachable given `job_view_for_job` just resolved this same
/// id, but is treated as "not found" rather than panicking.
fn side_and_dest_for_job<'a>(
    view: &'a JobView,
    job_id: &str,
) -> Option<(&'a JobSide, Destination)> {
    if view.s3.job_id == job_id {
        Some((&view.s3, Destination::S3))
    } else if view.gdrive.job_id == job_id {
        Some((&view.gdrive, Destination::GDrive))
    } else {
        None
    }
}

impl WorkerPool {
    /// Spawns `config.workers_per_destination` worker loops for each of
    /// `S3`/`GDrive`, plus the 1s throughput ticker (SPEC.md §7
    /// `throughput`).
    pub async fn start(deps: WorkerDeps) -> Arc<WorkerPool> {
        let pool = Arc::new(WorkerPool {
            handles: Mutex::new(HashMap::new()),
            cancel: CancellationToken::new(),
            paused_dest: PausedSet::new(),
            inflight: Inflight::new(),
            meters: Meters::new(),
            deps,
            throughput_handle: Mutex::new(None),
        });

        let n = pool.deps.config.read().await.workers_per_destination;
        pool.resize(n).await;

        let ticker = tokio::spawn(throughput_ticker(
            pool.deps.clone(),
            pool.meters.clone(),
            pool.cancel.clone(),
        ));
        *pool
            .throughput_handle
            .lock()
            .expect("throughput-handle mutex poisoned") = Some(ticker);

        pool
    }

    /// Hot-reloads the worker count for *both* destinations to `n`
    /// (`AppConfig::workers_per_destination`, validated to `1..=4` upstream
    /// in `config.rs`). Spawns more loops or aborts the extras.
    pub async fn resize(&self, n: u8) {
        for dest in [Destination::S3, Destination::GDrive] {
            self.resize_dest(dest, n);
        }
    }

    fn resize_dest(&self, dest: Destination, n: u8) {
        let mut handles = self.handles.lock().expect("handles mutex poisoned");
        let list = handles.entry(dest).or_default();

        while list.len() < n as usize {
            let idx = list.len();
            let handle = tokio::spawn(worker_loop(
                dest,
                self.deps.clone(),
                self.paused_dest.clone(),
                self.inflight.clone(),
                self.meters.clone(),
                self.cancel.clone(),
                idx,
            ));
            list.push(handle);
        }

        while list.len() > n as usize {
            if let Some(handle) = list.pop() {
                handle.abort();
            }
        }
    }

    /// Pauses claiming for `dest` (SPEC.md §6: entered on `UploadError::Auth`,
    /// left by the user re-authenticating).
    pub fn pause_destination(&self, dest: Destination) {
        self.paused_dest.pause(dest);
    }

    /// Resumes claiming for `dest` and wakes any worker parked waiting for
    /// work so it doesn't sit idle for up to 5s.
    pub fn resume_destination(&self, dest: Destination) {
        self.paused_dest.resume(dest);
        self.deps.wakers.notify(dest);
    }

    pub fn is_paused(&self, dest: Destination) -> bool {
        self.paused_dest.is_paused(dest)
    }

    /// Pauses one job (RF-037/RF-063), independent of the per-destination
    /// pause above. A `pending` job is simply flipped to `paused` so
    /// `claim_next` skips it. An `uploading` (in-flight) job has its
    /// transfer cancelled immediately via the [`Inflight`] registry — the
    /// uploader's `upload()` then returns [`UploadError::Cancelled`], whose
    /// arm in `run_job` deliberately makes no further status write, leaving
    /// the `paused` we set here (and whatever `remote_state` the
    /// uploader/`state_sink` last persisted, e.g. an S3 multipart id) intact
    /// for [`WorkerPool::resume_job`] to pick back up. `attempts` is never
    /// touched by either path.
    ///
    /// No separate `WorkerDeps` parameter (unlike the task note's sketch):
    /// `WorkerPool` already owns one (`self.deps`), matching
    /// `pause_destination`/`resume_destination`'s existing convention of
    /// reaching through `self` rather than taking it again from the caller.
    pub async fn pause_job(&self, job_id: &str) -> Result<(), WorkerError> {
        let view = with_repo(self.deps.repo.clone(), {
            let job_id = job_id.to_string();
            move |repo| repo.job_view_for_job(&job_id)
        })
        .await?;
        let view = view.ok_or_else(|| WorkerError::NotFound(job_id.to_string()))?;
        let (side, _dest) = side_and_dest_for_job(&view, job_id)
            .ok_or_else(|| WorkerError::NotFound(job_id.to_string()))?;

        match side.status {
            // Idempotent: already where the caller wants it.
            JobStatus::Paused => return Ok(()),
            JobStatus::Pending | JobStatus::Uploading => {}
            other => return Err(WorkerError::InvalidTransition(job_id.to_string(), other)),
        }

        // Cancel the in-flight upload (if any) *before* persisting `paused`
        // so a job that finishes/fails between our status check and here
        // can't have its terminal status clobbered by the write below.
        self.inflight.cancel(job_id);

        with_repo(self.deps.repo.clone(), {
            let job_id = job_id.to_string();
            move |repo| repo.set_status(&job_id, JobStatus::Paused)
        })
        .await?;

        Ok(())
    }

    /// Resumes a `paused` job back to `pending` (RF-037/RF-063): `attempts`
    /// and `remote_state` are left exactly as they were, so a resumed S3
    /// multipart upload or Google Drive resumable session continues instead
    /// of restarting. Wakes an idle worker for the job's destination so it
    /// doesn't sit until the 5s fallback poll in [`wait_for_wake_or_cancel`].
    pub async fn resume_job(&self, job_id: &str) -> Result<(), WorkerError> {
        let view = with_repo(self.deps.repo.clone(), {
            let job_id = job_id.to_string();
            move |repo| repo.job_view_for_job(&job_id)
        })
        .await?;
        let view = view.ok_or_else(|| WorkerError::NotFound(job_id.to_string()))?;
        let (side, dest) = side_and_dest_for_job(&view, job_id)
            .ok_or_else(|| WorkerError::NotFound(job_id.to_string()))?;

        if side.status != JobStatus::Paused {
            return Err(WorkerError::InvalidTransition(
                job_id.to_string(),
                side.status,
            ));
        }

        with_repo(self.deps.repo.clone(), {
            let job_id = job_id.to_string();
            move |repo| repo.set_status(&job_id, JobStatus::Pending)
        })
        .await?;

        self.deps.wakers.notify(dest);
        Ok(())
    }

    /// Cancels every worker + the throughput ticker and waits up to `grace`
    /// for them to exit cleanly (SPEC.md "Sair": 30s grace before the
    /// process is killed regardless).
    pub async fn shutdown(&self, grace: Duration) {
        self.cancel.cancel();

        let handles: Vec<JoinHandle<()>> = {
            let mut map = self.handles.lock().expect("handles mutex poisoned");
            map.drain().flat_map(|(_, v)| v).collect()
        };
        let ticker = self
            .throughput_handle
            .lock()
            .expect("throughput-handle mutex poisoned")
            .take();

        let wait_all = async move {
            for handle in handles {
                let _ = handle.await;
            }
            if let Some(t) = ticker {
                let _ = t.await;
            }
        };

        if tokio::time::timeout(grace, wait_all).await.is_err() {
            tracing::warn!(
                ?grace,
                "pool de workers: desligamento excedeu o período de tolerância"
            );
        }
    }

    /// RNF-006 boot crash recovery: resets every `uploading` job back to
    /// `pending` (`Repo::recover_on_boot`) and, best-effort, aborts the
    /// remote side of any job that has an `upload_id` in its `remote_state`
    /// (S3 multipart) so nothing is left billing/orphaned. Returns how many
    /// jobs were recovered.
    pub async fn recover_on_boot(deps: WorkerDeps) -> u32 {
        let recovered = with_repo(deps.repo.clone(), |repo| repo.recover_on_boot())
            .await
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "worker: consulta do recover_on_boot falhou");
                Vec::new()
            });

        let count = recovered.len() as u32;

        for job in recovered {
            let Some(remote_state) = job.remote_state.as_deref() else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(remote_state) else {
                continue;
            };
            if value.get("upload_id").is_none() {
                continue;
            }

            let uploader = deps.uploaders.read().await.get(job.destination);
            let Some(uploader) = uploader else { continue };

            if let Err(e) = uploader.abort(&value).await {
                tracing::warn!(
                    job_id = %job.job_id,
                    error = %e,
                    "worker: melhor esforço de aborto na recuperação de boot falhou"
                );
            }
        }

        count
    }
}

// ---------------------------------------------------------------------
// Worker loop
// ---------------------------------------------------------------------

/// One worker of one destination: claims the next due job, runs it, repeats.
/// Idles (waiting on `wakers.waiter(dest)` with a 5s fallback timeout) while
/// `dest` is paused, has no uploader configured, or there is no due job.
pub async fn worker_loop(
    dest: Destination,
    deps: WorkerDeps,
    paused: PausedSet,
    inflight: Inflight,
    meters: Meters,
    cancel: CancellationToken,
    worker_idx: usize,
) {
    loop {
        if cancel.is_cancelled() {
            return;
        }

        let uploader = deps.uploaders.read().await.get(dest);
        let uploader = match uploader {
            Some(u) if !paused.is_paused(dest) => u,
            _ => {
                if wait_for_wake_or_cancel(&deps.wakers, dest, &cancel).await {
                    return;
                }
                continue;
            }
        };

        let now = worker_now().to_rfc3339();
        let claimed = with_repo(deps.repo.clone(), move |repo| repo.claim_next(dest, &now)).await;

        match claimed {
            Ok(Some(job)) => {
                let meter = meters.get(dest);
                let child_cancel = cancel.child_token();
                run_job(
                    job,
                    dest,
                    uploader,
                    deps.clone(),
                    paused.clone(),
                    inflight.clone(),
                    meter,
                    child_cancel,
                )
                .await;
            }
            Ok(None) => {
                if wait_for_wake_or_cancel(&deps.wakers, dest, &cancel).await {
                    return;
                }
            }
            Err(e) => {
                tracing::error!(?dest, worker_idx, error = %e, "worker: claim_next falhou");
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                }
            }
        }
    }
}

/// Waits for either a wakeup notification, a fixed 5s timeout (so a paused
/// destination re-checks periodically without an explicit resume signal
/// ever being missed for long), or cancellation. Returns `true` if the
/// caller should stop (cancelled).
async fn wait_for_wake_or_cancel(
    wakers: &Wakers,
    dest: Destination,
    cancel: &CancellationToken,
) -> bool {
    let waiter = wakers.waiter(dest);
    let notified = waiter.notified();
    tokio::pin!(notified);

    tokio::select! {
        _ = cancel.cancelled() => true,
        _ = &mut notified => false,
        _ = tokio::time::sleep(Duration::from_secs(5)) => false,
    }
}

// ---------------------------------------------------------------------
// Running a single job
// ---------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn run_job(
    job: JobRow,
    dest: Destination,
    uploader: Arc<dyn Uploader>,
    deps: WorkerDeps,
    paused: PausedSet,
    inflight: Inflight,
    meter: Arc<ThroughputMeter>,
    cancel: CancellationToken,
) {
    let job_id = job.id.clone();
    let file_id = job.file_id.clone();

    let view = with_repo(deps.repo.clone(), {
        let file_id = file_id.clone();
        move |repo| repo.job_view_for_file(&file_id)
    })
    .await;

    let view = match view {
        Ok(Some(v)) => v,
        Ok(None) => {
            tracing::error!(%job_id, %file_id, "worker: job reivindicado não tem registro de arquivo correspondente");
            let _ = with_repo(deps.repo.clone(), {
                let job_id = job_id.clone();
                move |repo| repo.mark_failed(&job_id, "internal error: file row missing")
            })
            .await;
            deps.events.job_updated(&job_id);
            return;
        }
        Err(e) => {
            tracing::error!(%job_id, error = %e, "worker: job_view_for_file falhou");
            return;
        }
    };

    let resume_state: Option<serde_json::Value> = job
        .remote_state
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    let (progress_tx, progress_rx) = mpsc::channel::<ProgressUpdate>(64);

    let req = UploadRequest {
        local_path: PathBuf::from(&view.path),
        remote_name: view.name.clone(),
        size: view.size as u64,
        sha256: view.sha256.clone(),
        progress: progress_tx,
        cancel: cancel.clone(),
        resume_state: resume_state.clone(),
    };

    let progress_handle = tokio::spawn(forward_progress(
        job_id.clone(),
        progress_rx,
        meter,
        deps.events.clone(),
    ));

    // Watchdog: an external command (cancel/pause) flips this job's status
    // while it's `uploading`; nothing else re-checks that once `upload()`
    // is running, so poll for it here and cancel the child token the
    // uploader was given.
    let watchdog_cancel = cancel.clone();
    let watchdog_repo = deps.repo.clone();
    let watchdog_file_id = file_id.clone();
    let watchdog_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = watchdog_cancel.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
            if should_abort(watchdog_repo.clone(), watchdog_file_id.clone(), dest).await {
                watchdog_cancel.cancel();
                return;
            }
        }
    });

    // T-5.2: register this job's cancellation token so `WorkerPool::pause_job`
    // can cancel it immediately (instead of waiting up to 1s for the
    // watchdog above to notice a status flip to `paused`) while it's
    // actually uploading.
    inflight.insert(job_id.clone(), cancel.clone());
    let result = uploader.upload(req).await;
    inflight.remove(&job_id);

    watchdog_handle.abort();
    let _ = progress_handle.await;

    match result {
        Ok(upload_result) => {
            let remote_state_json = upload_result.remote_state.as_ref().map(|v| v.to_string());
            let _ = with_repo(deps.repo.clone(), {
                let job_id = job_id.clone();
                let remote_id = upload_result.remote_id.clone();
                move |repo| repo.mark_done(&job_id, &remote_id, remote_state_json.as_deref())
            })
            .await;
            let _ = with_repo(deps.repo.clone(), {
                let job_id = job_id.clone();
                move |repo| repo.insert_event("info", Some(&job_id), "upload concluído")
            })
            .await;
            deps.events.job_updated(&job_id);
        }

        Err(UploadError::Cancelled) => {
            // Status was already set to `cancelled`/`paused` by whatever
            // command triggered this (RF-035); the uploader's best-effort
            // multipart abort, if any, already happened inside `upload()`.
            let _ = with_repo(deps.repo.clone(), {
                let job_id = job_id.clone();
                move |repo| repo.insert_event("info", Some(&job_id), "upload cancelado")
            })
            .await;
            deps.events.job_updated(&job_id);
        }

        Err(UploadError::Auth(msg)) => {
            let hint = UploadError::Auth(msg.clone())
                .hint()
                .map(str::to_string)
                .unwrap_or_else(|| default_auth_hint(dest));

            let next_at = worker_now().to_rfc3339();
            let attempts = job.attempts; // unchanged: auth failures never burn an attempt.
            let _ = with_repo(deps.repo.clone(), {
                let job_id = job_id.clone();
                let msg = msg.clone();
                move |repo| repo.mark_retry(&job_id, attempts, &next_at, &msg)
            })
            .await;

            paused.pause(dest);
            deps.events.job_updated(&job_id);
            deps.events.auth_required(dest, hint);
        }

        Err(UploadError::Permanent(msg)) => {
            // VULN-004: a permanent failure is not retried, so if a
            // multipart upload was in flight (`resume_state` carries an
            // `upload_id`) its parts would otherwise sit on S3 forever,
            // accruing storage cost with nothing left to complete or clean
            // them up. Best-effort: a failed abort is logged and does not
            // change the outcome for the job itself.
            abort_if_resumable(&uploader, &resume_state, &job_id).await;

            let _ = with_repo(deps.repo.clone(), {
                let job_id = job_id.clone();
                move |repo| repo.mark_failed(&job_id, &msg)
            })
            .await;
            deps.events.job_updated(&job_id);
        }

        // Remaining variants (`Transient`, `Io`) are exactly the retryable
        // ones per `UploadError::is_retryable()`.
        Err(e) => {
            let attempts = job.attempts + 1;
            let message = e.to_string();
            let max_attempts = i64::from(deps.config.read().await.retry.max_attempts);

            if attempts >= max_attempts {
                // VULN-004: retries are exhausted -- same best-effort abort
                // as the `Permanent` arm above, for the same reason (no
                // further attempt will ever complete or clean up this
                // multipart upload).
                abort_if_resumable(&uploader, &resume_state, &job_id).await;

                let _ = with_repo(deps.repo.clone(), {
                    let job_id = job_id.clone();
                    move |repo| repo.mark_failed(&job_id, &message)
                })
                .await;
            } else {
                let base_secs = u64::from(deps.config.read().await.retry.base_delay_seconds);
                let exponent = job.attempts as u32;
                let delay = backoff_delay(base_secs, exponent, random_jitter());
                let next_at = (worker_now()
                    + chrono::Duration::from_std(delay).unwrap_or_default())
                .to_rfc3339();

                let _ = with_repo(deps.repo.clone(), {
                    let job_id = job_id.clone();
                    move |repo| repo.mark_retry(&job_id, attempts, &next_at, &message)
                })
                .await;
            }
            deps.events.job_updated(&job_id);
        }
    }
}

/// VULN-004: best-effort cleanup for a job that will not be retried
/// (`Permanent`, or retry exhaustion) but may have left a multipart upload
/// in progress. Mirrors `WorkerPool::recover_on_boot`'s crash-recovery
/// abort: a no-op unless `remote_state` carries an `upload_id`, and any
/// abort failure is only logged -- it must never change the outcome
/// already decided for the job itself. See SPEC.md §9 for the S3 bucket
/// lifecycle rule (`AbortIncompleteMultipartUpload`) that backstops this in
/// case the abort call itself fails or is never reached (e.g. the process
/// crashes before it runs).
async fn abort_if_resumable(
    uploader: &Arc<dyn Uploader>,
    remote_state: &Option<serde_json::Value>,
    job_id: &str,
) {
    let Some(value) = remote_state else {
        return;
    };
    if value.get("upload_id").is_none() {
        return;
    }
    if let Err(e) = uploader.abort(value).await {
        tracing::warn!(job_id = %job_id, error = %e, "worker: melhor esforço de aborto em falha não retentada falhou");
    }
}

/// `UploadError::hint()` only extracts a hint embedded by `classify()`
/// (currently just the clock-skew case) — most `Auth` failures (missing IAM
/// permission, unshared Drive folder) carry none. This is a generic,
/// destination-flavored fallback; see the deviations note in the task
/// report for why a more specific hint isn't wired up here.
fn default_auth_hint(dest: Destination) -> String {
    match dest {
        Destination::S3 => {
            "Verifique as credenciais e a política IAM do bucket configurado.".to_string()
        }
        Destination::GDrive => {
            "Verifique a conta de serviço e o compartilhamento da pasta do Google Drive."
                .to_string()
        }
    }
}

/// Polls whether the job for `(file_id, dest)` was externally moved to a
/// terminal "stop this" status while it was uploading.
async fn should_abort(repo: Arc<Mutex<Repo>>, file_id: String, dest: Destination) -> bool {
    let view = with_repo(repo, move |repo| repo.job_view_for_file(&file_id)).await;
    match view {
        Ok(Some(view)) => {
            let side = match dest {
                Destination::S3 => view.s3,
                Destination::GDrive => view.gdrive,
            };
            matches!(side.status, JobStatus::Cancelled | JobStatus::Paused)
        }
        _ => false,
    }
}

/// Forwards `UploadRequest::progress` updates to `WorkerEvents::upload_progress`,
/// throttled to at most once per 500ms (always emitting the final 100%
/// update), and feeds the byte deltas into `meter` for the throughput ticker.
async fn forward_progress(
    job_id: String,
    mut rx: mpsc::Receiver<ProgressUpdate>,
    meter: Arc<ThroughputMeter>,
    events: Arc<dyn WorkerEvents>,
) {
    const MIN_INTERVAL: Duration = Duration::from_millis(500);

    let mut last_emit: Option<TokioInstant> = None;
    let mut last_sent: u64 = 0;

    while let Some(update) = rx.recv().await {
        let delta = update.sent.saturating_sub(last_sent);
        if delta > 0 {
            meter.record(delta);
        }
        last_sent = update.sent;

        let should_emit = last_emit.is_none_or(|t| t.elapsed() >= MIN_INTERVAL);
        let is_final = update.total > 0 && update.sent >= update.total;

        if should_emit || is_final {
            events.upload_progress(UploadProgress {
                job_id: job_id.clone(),
                sent: update.sent,
                total: update.total,
                rate_bps: meter.bps() as u64,
            });
            last_emit = Some(TokioInstant::now());
        }
    }
}

/// Emits `Throughput` once per second: aggregate `sent bytes/s` (from the
/// two per-destination [`ThroughputMeter`]s) plus the currently configured
/// QoS ceilings (SPEC.md §7 `throughput`, `None` while a destination is
/// unlimited).
async fn throughput_ticker(deps: WorkerDeps, meters: Meters, cancel: CancellationToken) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = interval.tick() => {}
        }

        let s3_bps = meters.s3.bps() as u64;
        let gdrive_bps = meters.gdrive.bps() as u64;
        let limit_s3 = deps.throttles.get(Destination::S3).limit_bps();
        let limit_gdrive = deps.throttles.get(Destination::GDrive).limit_bps();

        deps.events.throughput(Throughput {
            total_bps: s3_bps + gdrive_bps,
            gdrive_bps,
            s3_bps,
            limit_gdrive_bps: if limit_gdrive == 0 {
                None
            } else {
                Some(limit_gdrive)
            },
            limit_s3_bps: if limit_s3 == 0 { None } else { Some(limit_s3) },
        });
    }
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicUsize;

    use crate::state::Repo;

    // -- backoff_delay -------------------------------------------------

    #[test]
    fn backoff_delay_grows_exponentially_at_neutral_jitter() {
        assert_eq!(backoff_delay(5, 0, 0.5), Duration::from_secs_f64(5.0));
        assert_eq!(backoff_delay(5, 1, 0.5), Duration::from_secs_f64(10.0));
        assert_eq!(backoff_delay(5, 2, 0.5), Duration::from_secs_f64(20.0));
        assert_eq!(backoff_delay(5, 3, 0.5), Duration::from_secs_f64(40.0));
    }

    #[test]
    fn backoff_delay_jitter_bounds_are_plus_minus_20_percent() {
        assert_eq!(backoff_delay(5, 0, 0.0), Duration::from_secs_f64(4.0));
        assert_eq!(backoff_delay(5, 0, 1.0), Duration::from_secs_f64(6.0));
    }

    #[test]
    fn backoff_delay_caps_at_600s_before_jitter() {
        // 5 * 2^20 would overflow the useful range long before hitting u64
        // limits; the cap must apply before jitter is multiplied in.
        assert_eq!(backoff_delay(5, 20, 0.5), Duration::from_secs_f64(600.0));
        assert_eq!(backoff_delay(5, 20, 1.0), Duration::from_secs_f64(720.0));
        assert_eq!(backoff_delay(5, 20, 0.0), Duration::from_secs_f64(480.0));
    }

    #[test]
    fn backoff_delay_does_not_panic_on_extreme_attempt() {
        let d = backoff_delay(60, u32::MAX, 0.5);
        assert_eq!(d, Duration::from_secs_f64(600.0));
    }

    // -- mocks -----------------------------------------------------------

    #[derive(Clone, Debug)]
    enum Step {
        Succeed,
        Progress(Vec<(u64, u64)>),
        FailAuth(String),
        FailTransient(String),
        FailPermanent(String),
        /// T-5.2: loops emitting progress every 50ms, honoring
        /// `req.cancel`, until cancelled — used to simulate an upload
        /// that's genuinely in-flight when `WorkerPool::pause_job` cancels
        /// it. Notifies `MockUploader::started` the moment it begins so
        /// tests can await "the upload has actually started" instead of
        /// guessing with a sleep.
        SlowCancellable,
    }

    struct MockUploader {
        dest: Destination,
        script: Mutex<VecDeque<Step>>,
        calls: AtomicUsize,
        aborts: Mutex<Vec<serde_json::Value>>,
        started: Arc<tokio::sync::Notify>,
        /// `UploadRequest::resume_state` seen on each call, in order — lets
        /// pause/resume tests assert `remote_state` survived the round trip
        /// without needing raw SQL access to the `jobs` table.
        resume_states: Mutex<Vec<Option<serde_json::Value>>>,
    }

    impl MockUploader {
        fn new(dest: Destination, steps: Vec<Step>) -> Arc<Self> {
            Arc::new(Self {
                dest,
                script: Mutex::new(steps.into()),
                calls: AtomicUsize::new(0),
                aborts: Mutex::new(Vec::new()),
                started: Arc::new(tokio::sync::Notify::new()),
                resume_states: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait::async_trait]
    impl Uploader for MockUploader {
        fn id(&self) -> Destination {
            self.dest
        }

        async fn test_connection(&self) -> Result<crate::uploaders::TestResult, UploadError> {
            Ok(crate::uploaders::TestResult {
                ok: true,
                message: "ok".into(),
                latency_ms: 1,
            })
        }

        async fn upload(
            &self,
            req: UploadRequest,
        ) -> Result<crate::uploaders::UploadResult, UploadError> {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            self.resume_states
                .lock()
                .unwrap()
                .push(req.resume_state.clone());
            let step = {
                let mut script = self.script.lock().unwrap();
                script.pop_front().unwrap_or(Step::Succeed)
            };

            match step {
                Step::Succeed => Ok(crate::uploaders::UploadResult {
                    remote_id: "remote-1".into(),
                    remote_state: None,
                }),
                Step::Progress(updates) => {
                    for (sent, total) in updates {
                        let _ = req.progress.send(ProgressUpdate { sent, total }).await;
                    }
                    Ok(crate::uploaders::UploadResult {
                        remote_id: "remote-1".into(),
                        remote_state: None,
                    })
                }
                Step::FailAuth(msg) => Err(UploadError::Auth(msg)),
                Step::FailTransient(msg) => Err(UploadError::Transient(msg)),
                Step::FailPermanent(msg) => Err(UploadError::Permanent(msg)),
                Step::SlowCancellable => {
                    self.started.notify_one();
                    loop {
                        tokio::select! {
                            _ = req.cancel.cancelled() => return Err(UploadError::Cancelled),
                            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                                let _ = req
                                    .progress
                                    .send(ProgressUpdate { sent: 10, total: 100 })
                                    .await;
                            }
                        }
                    }
                }
            }
        }

        async fn abort(&self, remote_state: &serde_json::Value) -> Result<(), UploadError> {
            self.aborts.lock().unwrap().push(remote_state.clone());
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingEvents {
        job_updated: Mutex<Vec<String>>,
        progress: Mutex<Vec<UploadProgress>>,
        #[allow(dead_code)]
        throughput: Mutex<Vec<Throughput>>,
        auth_required: Mutex<Vec<(Destination, String)>>,
    }

    impl WorkerEvents for RecordingEvents {
        fn job_updated(&self, job_id: &str) {
            self.job_updated.lock().unwrap().push(job_id.to_string());
        }
        fn upload_progress(&self, p: UploadProgress) {
            self.progress.lock().unwrap().push(p);
        }
        fn throughput(&self, t: Throughput) {
            self.throughput.lock().unwrap().push(t);
        }
        fn auth_required(&self, dest: Destination, hint: String) {
            self.auth_required.lock().unwrap().push((dest, hint));
        }
    }

    fn test_deps(events: Arc<RecordingEvents>) -> WorkerDeps {
        WorkerDeps {
            repo: Arc::new(Mutex::new(Repo::open_in_memory().unwrap())),
            wakers: Arc::new(Wakers::new()),
            config: Arc::new(RwLock::new(AppConfig::default())),
            throttles: Throttles {
                s3: Throttle::new(0),
                gdrive: Throttle::new(0),
            },
            uploaders: Arc::new(RwLock::new(Uploaders::default())),
            events,
        }
    }

    /// Seeds one file via `upsert_file_and_enqueue` and resolves its `s3`
    /// job id — shared setup for the `pause_job`/`resume_job` tests below.
    async fn seed_s3_job(deps: &WorkerDeps, path: &str) -> String {
        with_repo(deps.repo.clone(), {
            let path = path.to_string();
            move |repo| repo.upsert_file_and_enqueue(&path, "sha-a", 100, "2026-01-01T00:00:00Z")
        })
        .await
        .unwrap();

        let file_id = with_repo(deps.repo.clone(), {
            let path = path.to_string();
            move |repo| repo.file_id_for_path(&path)
        })
        .await
        .unwrap()
        .expect("file row was just created");

        with_repo(deps.repo.clone(), move |repo| {
            repo.job_view_for_file(&file_id)
        })
        .await
        .unwrap()
        .expect("job view was just created")
        .s3
        .job_id
    }

    // -- run_job: success -------------------------------------------------

    #[tokio::test(start_paused = true)]
    async fn run_job_success_marks_done_and_reports_progress() {
        let events = Arc::new(RecordingEvents::default());
        let deps = test_deps(events.clone());

        with_repo(deps.repo.clone(), |repo| {
            repo.upsert_file_and_enqueue("/tmp/a.txt", "sha-a", 100, "2026-01-01T00:00:00Z")
        })
        .await
        .unwrap();

        let now = worker_now().to_rfc3339();
        let job = with_repo(deps.repo.clone(), move |repo| {
            repo.claim_next(Destination::S3, &now)
        })
        .await
        .unwrap()
        .expect("a freshly-enqueued file has a due s3 job");
        let file_id = job.file_id.clone();

        let uploader = MockUploader::new(
            Destination::S3,
            vec![Step::Progress(vec![(50, 100), (100, 100)])],
        );

        run_job(
            job,
            Destination::S3,
            uploader.clone(),
            deps.clone(),
            PausedSet::new(),
            Inflight::default(),
            Arc::new(ThroughputMeter::new()),
            CancellationToken::new(),
        )
        .await;

        assert_eq!(uploader.calls.load(AtomicOrdering::SeqCst), 1);
        assert!(!events.progress.lock().unwrap().is_empty());
        assert_eq!(events.job_updated.lock().unwrap().len(), 1);

        let view = with_repo(deps.repo.clone(), move |repo| {
            repo.job_view_for_file(&file_id)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(view.s3.status, JobStatus::Done);
        assert_eq!(view.s3.remote_id.as_deref(), Some("remote-1"));
    }

    // -- run_job: transient x2 then success --------------------------------

    #[tokio::test(start_paused = true)]
    async fn transient_failures_retry_with_backoff_then_succeed() {
        let events = Arc::new(RecordingEvents::default());
        let deps = test_deps(events.clone());

        with_repo(deps.repo.clone(), |repo| {
            repo.upsert_file_and_enqueue("/tmp/a.txt", "sha-a", 100, "2026-01-01T00:00:00Z")
        })
        .await
        .unwrap();

        let uploader = MockUploader::new(
            Destination::S3,
            vec![
                Step::FailTransient("boom-1".into()),
                Step::FailTransient("boom-2".into()),
                Step::Succeed,
            ],
        );

        let mut file_id = String::new();
        for _ in 0..3 {
            let now = worker_now().to_rfc3339();
            let job = with_repo(deps.repo.clone(), move |repo| {
                repo.claim_next(Destination::S3, &now)
            })
            .await
            .unwrap()
            .expect("job should be due for this attempt");
            file_id = job.file_id.clone();

            run_job(
                job,
                Destination::S3,
                uploader.clone(),
                deps.clone(),
                PausedSet::new(),
                Inflight::default(),
                Arc::new(ThroughputMeter::new()),
                CancellationToken::new(),
            )
            .await;

            // Comfortably clears the worst-case (jittered) backoff for any
            // of the first few attempts (base 5s: ~4-6s, ~8-12s, ...).
            tokio::time::advance(Duration::from_secs(60)).await;
        }

        assert_eq!(uploader.calls.load(AtomicOrdering::SeqCst), 3);

        let view = with_repo(deps.repo.clone(), move |repo| {
            repo.job_view_for_file(&file_id)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(view.s3.status, JobStatus::Done);
        assert_eq!(
            view.s3.attempts, 2,
            "2 failed attempts were recorded before success"
        );
    }

    // -- run_job: 5 transient failures -> failed ---------------------------

    #[tokio::test(start_paused = true)]
    async fn five_transient_failures_mark_job_failed() {
        let events = Arc::new(RecordingEvents::default());
        let deps = test_deps(events.clone());

        with_repo(deps.repo.clone(), |repo| {
            repo.upsert_file_and_enqueue("/tmp/a.txt", "sha-a", 100, "2026-01-01T00:00:00Z")
        })
        .await
        .unwrap();

        let steps = (0..5)
            .map(|i| Step::FailTransient(format!("boom-{i}")))
            .collect();
        let uploader = MockUploader::new(Destination::S3, steps);

        let mut file_id = String::new();
        for _ in 0..5 {
            let now = worker_now().to_rfc3339();
            let job = with_repo(deps.repo.clone(), move |repo| {
                repo.claim_next(Destination::S3, &now)
            })
            .await
            .unwrap()
            .expect("job should be due for this attempt");
            file_id = job.file_id.clone();

            run_job(
                job,
                Destination::S3,
                uploader.clone(),
                deps.clone(),
                PausedSet::new(),
                Inflight::default(),
                Arc::new(ThroughputMeter::new()),
                CancellationToken::new(),
            )
            .await;

            tokio::time::advance(Duration::from_secs(600 * 2)).await;
        }

        assert_eq!(uploader.calls.load(AtomicOrdering::SeqCst), 5);

        let view = with_repo(deps.repo.clone(), move |repo| {
            repo.job_view_for_file(&file_id)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(view.s3.status, JobStatus::Failed);
        assert!(view.s3.last_error.unwrap().contains("boom-4"));

        // The 5th (final) mark_failed doesn't reschedule, so no 6th claim
        // should ever become available.
        let now = worker_now().to_rfc3339();
        let none = with_repo(deps.repo.clone(), move |repo| {
            repo.claim_next(Destination::S3, &now)
        })
        .await
        .unwrap();
        assert!(none.is_none());
    }

    // -- run_job: retry exhaustion aborts a dangling multipart (VULN-004) --

    #[tokio::test(start_paused = true)]
    async fn retry_exhaustion_with_seeded_remote_state_calls_abort_once() {
        let events = Arc::new(RecordingEvents::default());
        let deps = test_deps(events.clone());

        with_repo(deps.repo.clone(), |repo| {
            repo.upsert_file_and_enqueue("/tmp/a.txt", "sha-a", 100, "2026-01-01T00:00:00Z")
        })
        .await
        .unwrap();
        let file_id = with_repo(deps.repo.clone(), |repo| {
            repo.file_id_for_path("/tmp/a.txt")
        })
        .await
        .unwrap()
        .expect("file row should exist after upsert_file_and_enqueue");

        // Seed a dangling multipart upload_id on the job's remote_state, as
        // if a previous attempt had gotten partway through before failing
        // transiently -- `mark_retry` never touches `remote_state`, so this
        // stays in place across every subsequent attempt.
        let job_id = with_repo(deps.repo.clone(), {
            let file_id = file_id.clone();
            move |repo| repo.job_view_for_file(&file_id)
        })
        .await
        .unwrap()
        .unwrap()
        .s3
        .job_id;
        with_repo(deps.repo.clone(), {
            let job_id = job_id.clone();
            move |repo| {
                repo.set_remote_state(&job_id, r#"{"upload_id":"dangling-upload","key":"a.txt"}"#)
            }
        })
        .await
        .unwrap();

        let steps = (0..5)
            .map(|i| Step::FailTransient(format!("boom-{i}")))
            .collect();
        let uploader = MockUploader::new(Destination::S3, steps);

        for _ in 0..5 {
            let now = worker_now().to_rfc3339();
            let job = with_repo(deps.repo.clone(), move |repo| {
                repo.claim_next(Destination::S3, &now)
            })
            .await
            .unwrap()
            .expect("job should be due for this attempt");

            run_job(
                job,
                Destination::S3,
                uploader.clone(),
                deps.clone(),
                PausedSet::new(),
                Inflight::default(),
                Arc::new(ThroughputMeter::new()),
                CancellationToken::new(),
            )
            .await;

            tokio::time::advance(Duration::from_secs(600 * 2)).await;
        }

        assert_eq!(uploader.calls.load(AtomicOrdering::SeqCst), 5);

        let view = with_repo(deps.repo.clone(), move |repo| {
            repo.job_view_for_file(&file_id)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(view.s3.status, JobStatus::Failed);

        let aborts = uploader.aborts.lock().unwrap();
        assert_eq!(
            aborts.len(),
            1,
            "abort must be called exactly once, on the final (non-retried) failure"
        );
        assert_eq!(aborts[0]["upload_id"], "dangling-upload");
    }

    // -- run_job: Auth -> pause, no attempts burned ------------------------

    #[tokio::test(start_paused = true)]
    async fn auth_failure_pauses_destination_and_does_not_burn_an_attempt() {
        let events = Arc::new(RecordingEvents::default());
        let deps = test_deps(events.clone());

        with_repo(deps.repo.clone(), |repo| {
            repo.upsert_file_and_enqueue("/tmp/a.txt", "sha-a", 100, "2026-01-01T00:00:00Z")
        })
        .await
        .unwrap();

        let now = worker_now().to_rfc3339();
        let job = with_repo(deps.repo.clone(), move |repo| {
            repo.claim_next(Destination::S3, &now)
        })
        .await
        .unwrap()
        .unwrap();
        let file_id = job.file_id.clone();

        let uploader = MockUploader::new(
            Destination::S3,
            vec![Step::FailAuth("403 forbidden".into())],
        );
        let paused = PausedSet::new();

        run_job(
            job,
            Destination::S3,
            uploader,
            deps.clone(),
            paused.clone(),
            Inflight::default(),
            Arc::new(ThroughputMeter::new()),
            CancellationToken::new(),
        )
        .await;

        assert!(paused.is_paused(Destination::S3));
        assert_eq!(events.auth_required.lock().unwrap().len(), 1);

        let view = with_repo(deps.repo.clone(), move |repo| {
            repo.job_view_for_file(&file_id)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(view.s3.status, JobStatus::Pending);
        assert_eq!(
            view.s3.attempts, 0,
            "an Auth failure must not burn a retry attempt"
        );
    }

    // -- run_job: Permanent -> failed immediately --------------------------

    #[tokio::test(start_paused = true)]
    async fn permanent_failure_marks_failed_on_first_attempt() {
        let events = Arc::new(RecordingEvents::default());
        let deps = test_deps(events.clone());

        with_repo(deps.repo.clone(), |repo| {
            repo.upsert_file_and_enqueue("/tmp/a.txt", "sha-a", 100, "2026-01-01T00:00:00Z")
        })
        .await
        .unwrap();

        let now = worker_now().to_rfc3339();
        let job = with_repo(deps.repo.clone(), move |repo| {
            repo.claim_next(Destination::S3, &now)
        })
        .await
        .unwrap()
        .unwrap();
        let file_id = job.file_id.clone();

        let uploader = MockUploader::new(
            Destination::S3,
            vec![Step::FailPermanent("400 bad request".into())],
        );

        run_job(
            job,
            Destination::S3,
            uploader.clone(),
            deps.clone(),
            PausedSet::new(),
            Inflight::default(),
            Arc::new(ThroughputMeter::new()),
            CancellationToken::new(),
        )
        .await;

        assert_eq!(uploader.calls.load(AtomicOrdering::SeqCst), 1);

        let view = with_repo(deps.repo.clone(), move |repo| {
            repo.job_view_for_file(&file_id)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(view.s3.status, JobStatus::Failed);
    }

    // -- recover_on_boot ----------------------------------------------------

    #[tokio::test]
    async fn recover_on_boot_resets_uploading_jobs_and_aborts_orphans() {
        let events = Arc::new(RecordingEvents::default());
        let deps = test_deps(events.clone());

        with_repo(deps.repo.clone(), |repo| {
            repo.upsert_file_and_enqueue("/tmp/a.txt", "sha-a", 100, "2026-01-01T00:00:00Z")
        })
        .await
        .unwrap();

        let now = worker_now().to_rfc3339();
        let job = with_repo(deps.repo.clone(), move |repo| {
            repo.claim_next(Destination::S3, &now)
        })
        .await
        .unwrap()
        .unwrap();
        let job_id = job.id.clone();
        let file_id = job.file_id.clone();

        with_repo(deps.repo.clone(), {
            let job_id = job_id.clone();
            move |repo| repo.set_remote_state(&job_id, r#"{"upload_id":"abc-123"}"#)
        })
        .await
        .unwrap();

        let uploader = MockUploader::new(Destination::S3, vec![]);
        deps.uploaders.write().await.s3 = Some(uploader.clone());

        let n = WorkerPool::recover_on_boot(deps.clone()).await;
        assert_eq!(n, 1);
        assert_eq!(uploader.aborts.lock().unwrap().len(), 1);
        assert_eq!(
            uploader.aborts.lock().unwrap()[0],
            serde_json::json!({"upload_id": "abc-123"})
        );

        let view = with_repo(deps.repo.clone(), move |repo| {
            repo.job_view_for_file(&file_id)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(view.s3.status, JobStatus::Pending);
    }

    // -- pause/resume gates claiming -----------------------------------------

    #[tokio::test(start_paused = true)]
    async fn pause_destination_gates_claiming_until_resumed() {
        let events = Arc::new(RecordingEvents::default());
        let deps = test_deps(events.clone());

        with_repo(deps.repo.clone(), |repo| {
            repo.upsert_file_and_enqueue("/tmp/a.txt", "sha-a", 100, "2026-01-01T00:00:00Z")
        })
        .await
        .unwrap();

        let uploader = MockUploader::new(Destination::S3, vec![Step::Succeed]);
        deps.uploaders.write().await.s3 = Some(uploader.clone());
        deps.config.write().await.workers_per_destination = 1;

        let pool = WorkerPool::start(deps.clone()).await;
        pool.pause_destination(Destination::S3);

        tokio::time::sleep(Duration::from_secs(6)).await;
        assert_eq!(
            uploader.calls.load(AtomicOrdering::SeqCst),
            0,
            "a paused destination must not claim jobs"
        );

        pool.resume_destination(Destination::S3);
        tokio::time::sleep(Duration::from_secs(6)).await;
        assert_eq!(
            uploader.calls.load(AtomicOrdering::SeqCst),
            1,
            "resuming must let the worker claim and process the pending job"
        );

        pool.shutdown(Duration::from_secs(1)).await;
    }

    // -- resize -----------------------------------------------------------

    #[tokio::test(start_paused = true)]
    async fn resize_spawns_additional_workers_per_destination() {
        let events = Arc::new(RecordingEvents::default());
        let deps = test_deps(events.clone());

        let pool = WorkerPool::start(deps.clone()).await;

        {
            let handles = pool.handles.lock().unwrap();
            assert_eq!(handles.get(&Destination::S3).unwrap().len(), 2);
            assert_eq!(handles.get(&Destination::GDrive).unwrap().len(), 2);
        }

        pool.resize(4).await;

        {
            let handles = pool.handles.lock().unwrap();
            assert_eq!(handles.get(&Destination::S3).unwrap().len(), 4);
            assert_eq!(handles.get(&Destination::GDrive).unwrap().len(), 4);
        }

        pool.resize(1).await;
        {
            let handles = pool.handles.lock().unwrap();
            assert_eq!(handles.get(&Destination::S3).unwrap().len(), 1);
            assert_eq!(handles.get(&Destination::GDrive).unwrap().len(), 1);
        }

        pool.shutdown(Duration::from_secs(1)).await;
    }

    // -- pause_job / resume_job (T-5.2) ------------------------------------

    #[tokio::test]
    async fn claim_next_never_returns_a_paused_job() {
        // Direct assertion of the "worker loop ignores paused" contract at
        // the query level, independent of WorkerPool/pause_job: `claim_next`
        // only selects `status = 'pending'`, so a job flipped straight to
        // `paused` in the DB must never be returned.
        let deps = test_deps(Arc::new(RecordingEvents::default()));
        let job_id = seed_s3_job(&deps, "/tmp/a.txt").await;

        with_repo(deps.repo.clone(), {
            let job_id = job_id.clone();
            move |repo| repo.set_status(&job_id, JobStatus::Paused)
        })
        .await
        .unwrap();

        let now = worker_now().to_rfc3339();
        let claimed = with_repo(deps.repo.clone(), move |repo| {
            repo.claim_next(Destination::S3, &now)
        })
        .await
        .unwrap();
        assert!(claimed.is_none(), "claim_next must skip paused jobs");
    }

    #[tokio::test(start_paused = true)]
    async fn pause_job_on_pending_job_blocks_claiming_until_resumed() {
        let events = Arc::new(RecordingEvents::default());
        let deps = test_deps(events.clone());
        let job_id = seed_s3_job(&deps, "/tmp/a.txt").await;

        let uploader = MockUploader::new(Destination::S3, vec![Step::Succeed]);
        deps.uploaders.write().await.s3 = Some(uploader.clone());
        // Starts with 0 workers so pausing happens before any worker exists
        // to race against — resize(1) below is what actually lets a worker
        // start claiming.
        deps.config.write().await.workers_per_destination = 0;

        let pool = WorkerPool::start(deps.clone()).await;
        pool.pause_job(&job_id).await.unwrap();
        pool.resize(1).await;

        tokio::time::sleep(Duration::from_secs(6)).await;
        assert_eq!(
            uploader.calls.load(AtomicOrdering::SeqCst),
            0,
            "a paused job must never be claimed"
        );

        let view = with_repo(deps.repo.clone(), {
            let job_id = job_id.clone();
            move |repo| repo.job_view_for_job(&job_id)
        })
        .await
        .unwrap()
        .expect("job still exists");
        assert_eq!(view.s3.status, JobStatus::Paused);

        pool.resume_job(&job_id).await.unwrap();
        tokio::time::sleep(Duration::from_secs(6)).await;
        assert_eq!(
            uploader.calls.load(AtomicOrdering::SeqCst),
            1,
            "resuming a pending job must let a worker claim and run it"
        );

        pool.shutdown(Duration::from_secs(1)).await;
    }

    #[tokio::test(start_paused = true)]
    async fn pause_job_cancels_in_flight_upload_and_resume_completes_it() {
        let events = Arc::new(RecordingEvents::default());
        let deps = test_deps(events.clone());
        let job_id = seed_s3_job(&deps, "/tmp/a.txt").await;

        // Seed remote_state as if a prior attempt had already opened an S3
        // multipart upload — pause_job/resume_job must leave it untouched
        // (only `run_job`'s eventual `mark_done` clears it, on success).
        with_repo(deps.repo.clone(), {
            let job_id = job_id.clone();
            move |repo| repo.set_remote_state(&job_id, r#"{"upload_id":"u1","parts":[]}"#)
        })
        .await
        .unwrap();

        let uploader =
            MockUploader::new(Destination::S3, vec![Step::SlowCancellable, Step::Succeed]);
        let started = uploader.started.clone();
        deps.uploaders.write().await.s3 = Some(uploader.clone());
        deps.config.write().await.workers_per_destination = 1;

        let pool = WorkerPool::start(deps.clone()).await;
        // Deterministic instead of a guessed sleep: SlowCancellable notifies
        // this the instant `upload()` starts, which `run_job` only calls
        // after registering the job in `Inflight` — so by the time this
        // resolves, `pause_job` is guaranteed to find it in-flight.
        started.notified().await;

        pool.pause_job(&job_id).await.unwrap();

        // Let run_job's `Err(UploadError::Cancelled)` arm run to completion.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let view = with_repo(deps.repo.clone(), {
            let job_id = job_id.clone();
            move |repo| repo.job_view_for_job(&job_id)
        })
        .await
        .unwrap()
        .expect("job still exists");
        assert_eq!(view.s3.status, JobStatus::Paused);
        assert_eq!(
            view.s3.attempts, 0,
            "cancelling an in-flight job for pause must not burn a retry attempt"
        );
        assert_eq!(
            uploader.calls.load(AtomicOrdering::SeqCst),
            1,
            "must not have been reclaimed while paused"
        );

        pool.resume_job(&job_id).await.unwrap();
        tokio::time::sleep(Duration::from_secs(6)).await;

        let view = with_repo(deps.repo.clone(), {
            let job_id = job_id.clone();
            move |repo| repo.job_view_for_job(&job_id)
        })
        .await
        .unwrap()
        .expect("job still exists");
        assert_eq!(view.s3.status, JobStatus::Done);
        assert_eq!(uploader.calls.load(AtomicOrdering::SeqCst), 2);

        let resume_states = uploader.resume_states.lock().unwrap().clone();
        assert_eq!(
            resume_states[1],
            Some(serde_json::json!({"upload_id": "u1", "parts": []})),
            "remote_state persisted before the pause must reach the resumed upload attempt"
        );

        pool.shutdown(Duration::from_secs(1)).await;
    }

    #[tokio::test(start_paused = true)]
    async fn resume_job_on_a_done_job_is_an_invalid_transition() {
        let deps = test_deps(Arc::new(RecordingEvents::default()));
        deps.config.write().await.workers_per_destination = 0;
        let job_id = seed_s3_job(&deps, "/tmp/a.txt").await;

        with_repo(deps.repo.clone(), {
            let job_id = job_id.clone();
            move |repo| repo.mark_done(&job_id, "remote-1", None)
        })
        .await
        .unwrap();

        let pool = WorkerPool::start(deps.clone()).await;
        let err = pool.resume_job(&job_id).await.unwrap_err();
        assert!(
            matches!(&err, WorkerError::InvalidTransition(id, JobStatus::Done) if id == &job_id),
            "unexpected error: {err:?}"
        );

        pool.shutdown(Duration::from_millis(10)).await;
    }

    #[tokio::test(start_paused = true)]
    async fn pause_job_unknown_id_is_not_found() {
        let deps = test_deps(Arc::new(RecordingEvents::default()));
        deps.config.write().await.workers_per_destination = 0;

        let pool = WorkerPool::start(deps.clone()).await;
        let err = pool.pause_job("does-not-exist").await.unwrap_err();
        assert!(
            matches!(&err, WorkerError::NotFound(id) if id == "does-not-exist"),
            "unexpected error: {err:?}"
        );

        pool.shutdown(Duration::from_millis(10)).await;
    }

    #[tokio::test(start_paused = true)]
    async fn resume_job_unknown_id_is_not_found() {
        let deps = test_deps(Arc::new(RecordingEvents::default()));
        deps.config.write().await.workers_per_destination = 0;

        let pool = WorkerPool::start(deps.clone()).await;
        let err = pool.resume_job("does-not-exist").await.unwrap_err();
        assert!(
            matches!(&err, WorkerError::NotFound(id) if id == "does-not-exist"),
            "unexpected error: {err:?}"
        );

        pool.shutdown(Duration::from_millis(10)).await;
    }
}
