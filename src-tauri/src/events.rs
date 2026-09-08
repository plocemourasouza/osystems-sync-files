//! Typed event emitters + the `log-line` forwarder (SPEC.md §7 "Events", PLAN.md
//! T-2.7).
//!
//! Every payload type (`AppStatus`, `JobView`, `UploadProgress`, `Throughput`,
//! `LogLine`, `AuthRequired`) lives in `core::state`/`core::logging` and is
//! `#[ts(export)]`, so the frontend's `src/types/generated.ts` mirrors these shapes
//! exactly — see `crates/core/src/state/model.rs`.
//!
//! `emit_*` never panics on failure: a failed `app.emit` (e.g. no window currently
//! exists) is logged via `tracing::warn!` and otherwise ignored, since a missed UI
//! update is not worth taking the whole app down over (RF-066/RF-068 want the *next*
//! event to still arrive, not a crash).

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use osystems_sync_core::health::HealthSink;
use osystems_sync_core::logging::{redact, LogLine, LoggingHandle};
use osystems_sync_core::queue;
use osystems_sync_core::rescan::{RescanError, RescanProgressSink};
use osystems_sync_core::state::{
    AppStatus, AuthRequired, Destination, DestinationsHealth, JobSide, JobStatus, JobView, Repo,
    RescanFailed, RescanProgress, Throughput, UploadProgress,
};
use osystems_sync_core::worker::WorkerEvents;
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::broadcast;

use crate::runtime::SyncRuntime;
use crate::tray::{self, TrayState};

// `EV_LOG_LINE`/`EV_STATUS_CHANGED`/`EV_JOB_UPDATED` are used (the log forwarder below,
// and `runtime.rs`'s intake/watcher pipeline as of T-2.6, respectively). The remaining
// three aren't referenced anywhere yet because the worker/health call sites that will
// use them land in later tasks (T-3.x, T-4.9). Kept `pub` and defined now regardless,
// same as `AppState::repo`/`logging` in `state.rs`, so those tasks only need to add
// call sites, not plumbing.
/// `status-changed` event name (SPEC.md §7). Payload: [`AppStatus`].
pub const EV_STATUS_CHANGED: &str = "status-changed";
/// `job-updated` event name (SPEC.md §7). Payload: [`JobView`].
pub const EV_JOB_UPDATED: &str = "job-updated";
#[allow(dead_code)]
/// `upload-progress` event name (SPEC.md §7), throttled by the caller to 500 ms per
/// job. Payload: [`UploadProgress`].
pub const EV_UPLOAD_PROGRESS: &str = "upload-progress";
#[allow(dead_code)]
/// `throughput` event name (SPEC.md §7), emitted by the caller once per second.
/// Payload: [`Throughput`].
pub const EV_THROUGHPUT: &str = "throughput";
/// `log-line` event name (SPEC.md §7, RF-066). Payload: [`LogLine`]. Forwarded
/// automatically by [`spawn_log_forwarder`] — no other call site should emit this one.
pub const EV_LOG_LINE: &str = "log-line";
#[allow(dead_code)]
/// `auth-required` event name (SPEC.md §7). Payload: [`AuthRequired`].
pub const EV_AUTH_REQUIRED: &str = "auth-required";
/// `rescan-failed` event name: a background reconciliation scan aborted with a
/// `RescanError` other than `NoPath` (PLAN.md T-1.4 follow-up). Payload:
/// [`RescanFailed`]. Emitted via [`notify_rescan_failure`] — new rescan call sites
/// should go through that helper rather than calling `emit_rescan_failed` directly,
/// so the `NoPath`-is-not-an-error rule can't be forgotten at a new site.
pub const EV_RESCAN_FAILED: &str = "rescan-failed";
/// `rescan-progress` event name (PLAN.md T-2.4): emitted while the manual
/// rescan command's hash/intake phase is running, so the UI can show a live
/// count instead of an indeterminate spinner over a folder that can take
/// minutes to scan. Payload: [`RescanProgress`]. Emitted via
/// [`rescan_progress_sink`] — new call sites that want this should build
/// their [`RescanProgressSink`] through that helper rather than constructing
/// the closure by hand.
pub const EV_RESCAN_PROGRESS: &str = "rescan-progress";

/// Emits `event` with a clone of `payload`, logging (never panicking) if the frontend
/// isn't there to receive it.
fn emit_event<S: serde::Serialize + Clone>(app: &AppHandle, event: &'static str, payload: &S) {
    if let Err(error) = app.emit(event, payload.clone()) {
        tracing::warn!(event, %error, "falha ao emitir evento para o frontend");
    }
}

/// Emits `status-changed` with the latest [`AppStatus`] snapshot.
pub fn emit_status_changed(app: &AppHandle, status: &AppStatus) {
    emit_event(app, EV_STATUS_CHANGED, status);
}

/// Emits `job-updated` for the one file/job pair that changed.
pub fn emit_job_updated(app: &AppHandle, job: &JobView) {
    emit_event(app, EV_JOB_UPDATED, job);
}

/// Emits `upload-progress` for one in-flight job. Callers are responsible for the
/// 500 ms throttle (SPEC.md §7) — this function emits unconditionally.
pub fn emit_upload_progress(app: &AppHandle, progress: &UploadProgress) {
    emit_event(app, EV_UPLOAD_PROGRESS, progress);
}

/// Emits `throughput` with the current aggregate + per-destination transfer rates.
/// Callers are responsible for the 1 s cadence (SPEC.md §7) — this function emits
/// unconditionally.
pub fn emit_throughput(app: &AppHandle, throughput: &Throughput) {
    emit_event(app, EV_THROUGHPUT, throughput);
}

/// Emits `log-line` for one structured log line. Only [`spawn_log_forwarder`] should
/// call this in practice — see its module docs on avoiding feedback loops.
pub fn emit_log_line(app: &AppHandle, line: &LogLine) {
    emit_event(app, EV_LOG_LINE, line);
}

/// Emits `auth-required` when a destination starts needing re-authentication.
pub fn emit_auth_required(app: &AppHandle, auth: &AuthRequired) {
    emit_event(app, EV_AUTH_REQUIRED, auth);
}

/// Emits `rescan-failed` with `err`'s message. Prefer [`notify_rescan_failure`] at
/// call sites — it also applies the `NoPath` exclusion this raw emitter does not.
pub fn emit_rescan_failed(app: &AppHandle, err: &RescanError) {
    emit_event(
        app,
        EV_RESCAN_FAILED,
        &RescanFailed {
            message: err.to_string(),
        },
    );
}

/// Pure decision for [`notify_rescan_failure`]: is `err` worth telling the user
/// about? Split out (no `AppHandle`) so it is unit-testable without a real Tauri
/// app, the same way [`should_notify`] is for the OS-notification rate limiter.
/// `false` for [`RescanError::NoPath`] (no folder chosen yet is the legitimate
/// initial state, not a failure) and for [`RescanError::AlreadyInProgress`]
/// (PLAN.md T-2.5: another rescan is already running — normal/expected under the
/// six independent call sites, not a failure either).
fn should_notify_rescan_failure(err: &RescanError) -> bool {
    !matches!(err, RescanError::NoPath | RescanError::AlreadyInProgress)
}

/// Logs and emits `rescan-failed` for a background rescan failure, unless
/// [`should_notify_rescan_failure`] says `err` doesn't warrant it (mirrors the
/// exclusion `SyncRuntime::start`/`resume` already log around).
/// [`RescanError::AlreadyInProgress`] gets its own `tracing::debug!` line (with
/// `context`, since a skip is routine, not worth `warn!`) rather than falling
/// through silently — everything else excluded by
/// `should_notify_rescan_failure` (just `NoPath`) stays silent as before.
/// `context` is a short pt-BR label identifying the call site (boot, resume,
/// tray, ...) for the log line only; the emitted event carries just `err`'s
/// message, with no call-site context, since the renderer doesn't need to
/// distinguish which rescan failed.
///
/// Centralizing this (rather than repeating the `NoPath`/`AlreadyInProgress`
/// guard at every one of the 5 background call sites — `runtime::start`,
/// `runtime`'s periodic reconcile loop and `ResumeDetectorSink`, `tray.rs`'s
/// `toggle_watcher`/`rescan_from_tray`, `commands::queue::resume_watcher`) is
/// what keeps a future 6th call site from silently swallowing the error again
/// the same way this one had to be fixed. The manual "Atualizar Lista" button
/// (`commands::queue::rescan`) does NOT go through here — it must surface
/// `AlreadyInProgress` to the user distinctly rather than no-op, so it handles
/// that variant itself before this function would ever see it.
pub fn notify_rescan_failure(app: &AppHandle, context: &str, err: RescanError) {
    if matches!(err, RescanError::AlreadyInProgress) {
        tracing::debug!(
            context,
            "varredura pulada: já existe uma varredura em andamento"
        );
        return;
    }
    if !should_notify_rescan_failure(&err) {
        return;
    }
    tracing::warn!(error = %err, context, "varredura de reconciliação falhou");
    emit_rescan_failed(app, &err);
}

/// Emits `rescan-progress` with `scanned`/`total` candidates processed so far.
pub fn emit_rescan_progress(app: &AppHandle, progress: &RescanProgress) {
    emit_event(app, EV_RESCAN_PROGRESS, progress);
}

/// Builds a [`RescanProgressSink`] that emits `rescan-progress` on `app` —
/// `core::rescan` never depends on `tauri` itself (this crate's cardinal
/// rule), so the manual rescan command builds this closure and hands it to
/// [`osystems_sync_core::rescan::rescan_with_progress`] the same way
/// `runtime.rs` builds `S3Uploader::with_state_sink`'s closure.
pub fn rescan_progress_sink(app: &AppHandle) -> RescanProgressSink {
    let app = app.clone();
    Arc::new(move |scanned, total| {
        emit_rescan_progress(&app, &RescanProgress { scanned, total });
    })
}

/// SPEC.md §7: `status-changed` must not fire more than ~twice a second even under a
/// burst of `job_updated` calls (many jobs finishing near-simultaneously) — every
/// emission pays for a fresh `build_app_status` (a `status_counts` SQL query plus
/// `HealthHandle::snapshot`), so bursts are coalesced instead of emitting once per job.
const STATUS_EMIT_MIN_INTERVAL: Duration = Duration::from_millis(500);

/// T-5.7: native OS notifications ("Falha no upload" / "Autenticação necessária") are
/// rate-limited per *kind* (not per job/destination) to at most one every 60 s, so a
/// burst of failures (e.g. the network dropping mid-batch) doesn't spam the OS
/// notification center.
const NOTIFY_MIN_INTERVAL: Duration = Duration::from_secs(60);

/// Notification body text is truncated to this many characters (PLAN.md T-5.7) — OS
/// notification centers clip long bodies anyway, and `last_error` can be an entire
/// wrapped SDK error string.
const NOTIFY_BODY_MAX_LEN: usize = 120;

/// Truncates `s` to at most `max` chars (not bytes — safe on multi-byte UTF-8),
/// appending `…` when it was cut.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut truncated: String = s.chars().take(max).collect();
    truncated.push('…');
    truncated
}

/// Builds the "Falha no upload" notification body for one failed job side.
///
/// `last_error` is a wrapped SDK/IO error string (T-6.3 audit MINOR #11) that can
/// itself embed a secret — e.g. an S3 SDK error echoing a signed request URL, or an
/// AWS credential scope. It is passed through
/// [`osystems_sync_core::logging::redact`] before ever reaching a native OS
/// notification, exactly as it would before hitting a log line, then truncated to
/// [`NOTIFY_BODY_MAX_LEN`].
fn failure_notification_body(
    job_name: &str,
    dest: Destination,
    last_error: Option<&str>,
) -> String {
    let error_text = last_error.unwrap_or("erro desconhecido");
    let redacted_error = redact(error_text);
    format!(
        "{} → {}: {}",
        job_name,
        destination_label(dest),
        truncate(&redacted_error, NOTIFY_BODY_MAX_LEN)
    )
}

/// Human-readable destination name for notification bodies (pt-BR, matches SPEC.md's
/// own UI copy conventions).
fn destination_label(dest: Destination) -> &'static str {
    match dest {
        Destination::S3 => "S3",
        Destination::GDrive => "Google Drive",
    }
}

/// Shows a native OS notification, logging (never panicking) on failure — same
/// never-crash-the-app policy as [`emit_event`].
fn notify(app: &AppHandle, title: &str, body: String) {
    if let Err(error) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!(%error, title, "falha ao exibir notificação nativa");
    }
}

/// Pure rate-limit decision for [`notify_kind`]: may a notification of `kind` fire at
/// `now`, given `last` (kind -> when it last fired)? Records `now` into `last` when it
/// does, exactly like the real call site — split out as a plain function (no
/// `AppHandle`, no wall-clock `Instant::now()`) so [`NOTIFY_MIN_INTERVAL`]'s behaviour
/// is unit-testable with synthetic timestamps instead of real sleeps.
fn should_notify(
    last: &mut HashMap<&'static str, Instant>,
    kind: &'static str,
    now: Instant,
) -> bool {
    let allowed = last
        .get(kind)
        .map(|prev| now.duration_since(*prev) >= NOTIFY_MIN_INTERVAL)
        .unwrap_or(true);
    if allowed {
        last.insert(kind, now);
    }
    allowed
}

/// Rate-limits (via [`should_notify`]) and shows a notification of `kind` (`"failed"`
/// or `"auth"`). Takes the shared map by reference rather than as an `&AppEvents`
/// method so [`WorkerEvents::job_updated`]'s spawned lookup task — which only carries
/// cloned `Arc`s, not `&self` — can call it too.
fn notify_kind(
    notify_last: &StdMutex<HashMap<&'static str, Instant>>,
    app: &AppHandle,
    kind: &'static str,
    title: &str,
    body: String,
) {
    let fire = {
        let mut last = notify_last
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        should_notify(&mut last, kind, Instant::now())
    };
    if fire {
        notify(app, title, body);
    }
}

/// Applies `status`'s tray icon state and tooltip (T-5.10). The icon is only
/// re-`set_icon`'d when [`tray::tray_state_for`] actually differs from
/// `last_tray_state` (a real OS call, best kept off the hot path); the tooltip is
/// cheap enough to just set every time a status is rebuilt.
fn apply_tray_state(
    app: &AppHandle,
    status: &AppStatus,
    last_tray_state: &StdMutex<Option<TrayState>>,
) {
    let desired = tray::tray_state_for(status);
    let changed = {
        let mut last = last_tray_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let changed = *last != Some(desired);
        *last = Some(desired);
        changed
    };
    if changed {
        if let Err(error) = tray::set_tray_state(app, desired) {
            tracing::warn!(%error, "falha ao atualizar o estado do ícone da bandeja");
        }
    }
    if let Some(tray_icon) = app.tray_by_id(tray::TRAY_ID) {
        let tooltip = tray::tray_tooltip_for(status);
        if let Err(error) = tray_icon.set_tooltip(Some(tooltip)) {
            tracing::warn!(%error, "falha ao atualizar a dica de contexto da bandeja");
        }
    }
}

/// Which [`JobView`] side (`s3` or `gdrive`) `job_id` (a `JobSide.job_id`, per
/// [`WorkerEvents::job_updated`]'s doc comment) refers to, if either.
fn matching_side<'a>(job: &'a JobView, job_id: &str) -> Option<(Destination, &'a JobSide)> {
    if job.gdrive.job_id == job_id {
        Some((Destination::GDrive, &job.gdrive))
    } else if job.s3.job_id == job_id {
        Some((Destination::S3, &job.s3))
    } else {
        None
    }
}

/// Bridges `core::worker::WorkerEvents` and `core::health::HealthSink` to this crate's
/// typed `emit_*` functions (PLAN.md T-3.9) — the one concrete type both
/// `WorkerDeps::events` and `health::spawn`'s `sink` are built from in
/// `SyncRuntime::boot_workers`, so a worker-observed state change and the health
/// monitor's own periodic probe both end up on the same notification path instead of
/// needing two separate sink implementations.
///
/// # The `auth_required` double-emit (documented, not a bug)
///
/// [`WorkerEvents::auth_required`] emits `auth-required` immediately with the
/// uploader's own message, then hands the transition to
/// [`HealthHandle::set_auth_required`](osystems_sync_core::health::HealthHandle::set_auth_required)
/// so `AppStatus.destinations` reflects it too. That call, being unaware it was
/// triggered by the worker rather than its own periodic probe, invokes
/// [`HealthSink::auth_required`] on this *same* instance again — this time with
/// `core::health`'s hardcoded generic hint rather than the uploader's specific
/// message. The renderer may therefore see two `auth-required` events in quick
/// succession for one worker-triggered failure (same destination, specific hint then
/// generic hint) — harmless, and not fixable from this crate without editing
/// `core::health` (owned by a different task, frozen for this one).
pub struct AppEvents {
    app: AppHandle,
    repo: Arc<StdMutex<Repo>>,
    runtime: Arc<SyncRuntime>,
    last_status_emit: Arc<StdMutex<Instant>>,
    /// Last time a notification of each kind (`"failed"`, `"auth"`) was shown — backs
    /// the [`NOTIFY_MIN_INTERVAL`] rate limit (T-5.7).
    notify_last: Arc<StdMutex<HashMap<&'static str, Instant>>>,
    /// The tray icon state last applied via [`tray::set_tray_state`] — `None` before
    /// the first `AppStatus` build. Lets `apply_tray_state` skip `set_icon` (a real
    /// OS call) when the status rebuild didn't actually change which icon should show
    /// (T-5.10).
    last_tray_state: Arc<StdMutex<Option<TrayState>>>,
}

impl AppEvents {
    /// Builds a fresh `AppEvents` wired to `app`/`repo` and — for `changed`/
    /// `auth_required`'s health-state updates and every `status-changed` rebuild —
    /// `runtime`. Returned as an `Arc` since callers hand the *same* instance to both
    /// `WorkerDeps::events` (as `Arc<dyn WorkerEvents>`) and `health::spawn` (as
    /// `Arc<dyn HealthSink>`).
    pub fn new(app: AppHandle, repo: Arc<StdMutex<Repo>>, runtime: Arc<SyncRuntime>) -> Arc<Self> {
        Arc::new(Self {
            app,
            repo,
            runtime,
            // Backdated so the very first `job_updated` after boot emits immediately
            // instead of waiting out a full interval it never actually used.
            last_status_emit: Arc::new(StdMutex::new(Instant::now() - STATUS_EMIT_MIN_INTERVAL)),
            notify_last: Arc::new(StdMutex::new(HashMap::new())),
            last_tray_state: Arc::new(StdMutex::new(None)),
        })
    }

    /// [`notify_kind`] bound to this instance's `app`/`notify_last` — used by the two
    /// synchronous `auth_required` impls below (they hold `&self` already, unlike
    /// `job_updated`'s spawned lookup task).
    fn notify_rate_limited(&self, kind: &'static str, title: &str, body: String) {
        notify_kind(&self.notify_last, &self.app, kind, title, body);
    }

    /// Rebuilds `AppStatus` from scratch and emits it, unconditionally. Per T-3.9:
    /// "changed -> emit_status_changed with a fresh build_app_status that now reads
    /// health.snapshot()" — the `DestinationsHealth` a caller may already be holding
    /// (e.g. `HealthSink::changed`'s parameter) is deliberately ignored in favor of
    /// this, since `AppStatus` needs the rest of the snapshot (counts, watcher state)
    /// too, not just the destinations' health.
    fn emit_status_now(&self) {
        let app = self.app.clone();
        let repo = self.repo.clone();
        let runtime = self.runtime.clone();
        let last_tray_state = self.last_tray_state.clone();
        tauri::async_runtime::spawn(async move {
            match runtime.build_app_status(&repo).await {
                Ok(status) => {
                    apply_tray_state(&app, &status, &last_tray_state);
                    emit_status_changed(&app, &status);
                }
                Err(err) => {
                    tracing::warn!(error = %err, "AppEvents: falha ao construir o status do app")
                }
            }
        });
    }

    /// [`AppEvents::emit_status_now`], coalesced to at most once per
    /// [`STATUS_EMIT_MIN_INTERVAL`] — used by [`WorkerEvents::job_updated`], by far the
    /// highest-frequency call site (once per completed multipart part, at minimum).
    fn emit_status_throttled(&self) {
        let should_emit = {
            let mut last = self
                .last_status_emit
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if last.elapsed() >= STATUS_EMIT_MIN_INTERVAL {
                *last = Instant::now();
                true
            } else {
                false
            }
        };
        if should_emit {
            self.emit_status_now();
        }
    }
}

impl WorkerEvents for AppEvents {
    fn job_updated(&self, job_id: &str) {
        let app = self.app.clone();
        let repo = self.repo.clone();
        let notify_last = self.notify_last.clone();
        let owned_job_id = job_id.to_string();
        tauri::async_runtime::spawn(async move {
            let lookup_id = owned_job_id.clone();
            match queue::with_repo(repo, move |r| r.job_view_for_job(&lookup_id)).await {
                Ok(Some(job)) => {
                    // T-5.7: notify on the side this event is actually about, not
                    // "either side is failed" — a long-failed `s3` job must not
                    // re-trigger a notification every time its sibling `gdrive` job
                    // merely progresses.
                    if let Some((dest, side)) = matching_side(&job, &owned_job_id) {
                        if side.status == JobStatus::Failed {
                            let body = failure_notification_body(
                                &job.name,
                                dest,
                                side.last_error.as_deref(),
                            );
                            notify_kind(&notify_last, &app, "failed", "Falha no upload", body);
                        }
                    }
                    emit_job_updated(&app, &job);
                }
                Ok(None) => tracing::warn!(
                    job_id = %owned_job_id,
                    "job_view_for_job não encontrou nada para um evento job_updated"
                ),
                Err(err) => tracing::warn!(
                    job_id = %owned_job_id,
                    error = %err,
                    "job_view_for_job falhou para um evento job_updated"
                ),
            }
        });
        self.emit_status_throttled();
    }

    fn upload_progress(&self, p: UploadProgress) {
        emit_upload_progress(&self.app, &p);
    }

    fn throughput(&self, t: Throughput) {
        emit_throughput(&self.app, &t);
    }

    fn auth_required(&self, dest: Destination, hint: String) {
        emit_auth_required(
            &self.app,
            &AuthRequired {
                destination: dest,
                hint: hint.clone(),
            },
        );
        self.notify_rate_limited("auth", "Autenticação necessária", hint);

        let runtime = self.runtime.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(health) = runtime.health_handle() {
                // Also flips `AppStatus.destinations` — triggers this same `AppEvents`'
                // `HealthSink::changed` (and, on first transition, `HealthSink::auth_required`
                // again with the generic hint; see the module-level doc comment above).
                health.set_auth_required(dest, true).await;
            } else {
                tracing::warn!(
                    destination = ?dest,
                    "worker observou uma falha de autenticação antes de health::spawn ser executado; \
                     AppStatus.destinations não vai refletir isso até a próxima sonda"
                );
            }
        });
    }
}

impl HealthSink for AppEvents {
    fn changed(&self, _health: DestinationsHealth) {
        self.emit_status_now();
    }

    fn auth_required(&self, dest: Destination, hint: String) {
        emit_auth_required(
            &self.app,
            &AuthRequired {
                destination: dest,
                hint: hint.clone(),
            },
        );
        self.notify_rate_limited("auth", "Autenticação necessária", hint);
    }
}

/// What [`spawn_log_forwarder`]'s loop should do with one
/// `broadcast::Receiver::recv()` result. Factored out of the loop as a pure decision
/// function (see [`should_forward`]) so the `Ok`/`Lagged`/`Closed` branches are
/// unit-testable without a real broadcast channel or `AppHandle`.
#[derive(Debug, PartialEq)]
pub enum Forward {
    /// A line was received; the caller should emit it as `log-line`.
    Emit(LogLine),
    /// The receiver fell behind the ring and lost some lines; nothing to emit this
    /// tick, but the loop keeps running.
    Skip,
    /// The sender (the `LoggingHandle` the app owns for its whole lifetime) was
    /// dropped; the loop should exit.
    Stop,
}

/// Decides what to do with one `rx.recv().await` result.
///
/// Deliberately emits **no** `tracing` event for the common `Ok` case: this forwarder
/// is itself fed by `tracing` (via `core::logging`'s broadcast channel), so logging on
/// every successfully forwarded line would immediately re-enter the same channel and
/// spin forever. `Lagged` and `Closed` are rare, one-off conditions rather than
/// per-line noise, so those two are logged here.
pub fn should_forward(res: Result<LogLine, broadcast::error::RecvError>) -> Forward {
    match res {
        Ok(line) => Forward::Emit(line),
        Err(broadcast::error::RecvError::Lagged(skipped)) => {
            tracing::warn!(
                skipped,
                "log forwarder atrasou em relação ao canal broadcast do core::logging; alguns \
                 eventos log-line foram descartados"
            );
            Forward::Skip
        }
        Err(broadcast::error::RecvError::Closed) => {
            tracing::debug!("canal broadcast do log forwarder fechado; parando");
            Forward::Stop
        }
    }
}

/// Spawns the background task that subscribes to `logging`'s broadcast channel and
/// re-emits every line as a `log-line` event (SPEC.md §7, RF-066: UI latency ≤ 300 ms).
///
/// Call once, after `app.manage(state)`, with a clone of the same `Arc<LoggingHandle>`
/// stored in `AppState` — `logging` must outlive the task, which is why this takes an
/// owned `Arc` rather than a borrow.
pub fn spawn_log_forwarder(app: AppHandle, logging: Arc<LoggingHandle>) {
    let mut rx = logging.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match should_forward(rx.recv().await) {
                Forward::Emit(line) => emit_log_line(&app, &line),
                Forward::Skip => {}
                Forward::Stop => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_line() -> LogLine {
        LogLine {
            ts: "2026-01-01T00:00:00.000Z".to_string(),
            level: "INFO".to_string(),
            target: "osystems_sync_lib::events::tests".to_string(),
            job_id: None,
            destination: None,
            message: "sample".to_string(),
            error: None,
            path: None,
        }
    }

    #[test]
    fn should_forward_ok_emits_the_line() {
        let line = sample_line();
        assert_eq!(should_forward(Ok(line.clone())), Forward::Emit(line));
    }

    #[test]
    fn should_forward_lagged_skips_without_stopping() {
        assert_eq!(
            should_forward(Err(broadcast::error::RecvError::Lagged(7))),
            Forward::Skip
        );
    }

    #[test]
    fn should_forward_closed_stops() {
        assert_eq!(
            should_forward(Err(broadcast::error::RecvError::Closed)),
            Forward::Stop
        );
    }

    // ---- T-5.7: notification rate limiter --------------------------------------

    #[test]
    fn should_notify_fires_the_first_time_for_a_kind() {
        let mut last = HashMap::new();
        assert!(should_notify(&mut last, "failed", Instant::now()));
    }

    #[test]
    fn should_notify_suppresses_a_second_call_within_the_window() {
        let mut last = HashMap::new();
        let t0 = Instant::now();
        assert!(should_notify(&mut last, "failed", t0));
        assert!(!should_notify(
            &mut last,
            "failed",
            t0 + Duration::from_secs(30)
        ));
    }

    #[test]
    fn should_notify_fires_again_once_the_window_elapses() {
        let mut last = HashMap::new();
        let t0 = Instant::now();
        assert!(should_notify(&mut last, "failed", t0));
        assert!(should_notify(&mut last, "failed", t0 + NOTIFY_MIN_INTERVAL));
    }

    #[test]
    fn should_notify_tracks_kinds_independently() {
        let mut last = HashMap::new();
        let t0 = Instant::now();
        assert!(should_notify(&mut last, "failed", t0));
        // A different kind at the same instant is unaffected by "failed"'s state.
        assert!(should_notify(&mut last, "auth", t0));
        assert!(!should_notify(
            &mut last,
            "auth",
            t0 + Duration::from_secs(1)
        ));
    }

    // ---- T-1.4 follow-up: rescan-failed notification gate -----------------------

    #[test]
    fn should_notify_rescan_failure_is_false_for_no_path() {
        // `NoPath` (no folder configured yet) must stay silent — it is not a failure
        // an initial-run boot scan should surface to the user.
        assert!(!should_notify_rescan_failure(
            &osystems_sync_core::rescan::RescanError::NoPath
        ));
    }

    #[test]
    fn should_notify_rescan_failure_is_true_for_an_io_error() {
        let err = osystems_sync_core::rescan::RescanError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "watch folder is gone",
        ));
        assert!(should_notify_rescan_failure(&err));
    }

    #[test]
    fn should_notify_rescan_failure_is_false_for_already_in_progress() {
        // T-2.5: another rescan already running is expected under 6 independent
        // call sites, not a failure — background initiators must no-op silently.
        assert!(!should_notify_rescan_failure(
            &osystems_sync_core::rescan::RescanError::AlreadyInProgress
        ));
    }

    // ---- T-5.7: notification body helpers ---------------------------------------

    #[test]
    fn truncate_leaves_short_strings_untouched() {
        assert_eq!(truncate("short", 120), "short");
    }

    #[test]
    fn truncate_cuts_long_strings_and_appends_an_ellipsis() {
        let long = "a".repeat(200);
        let truncated = truncate(&long, 120);
        assert_eq!(truncated.chars().count(), 121); // 120 chars + '…'
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn destination_label_matches_expected_pt_br_copy() {
        assert_eq!(destination_label(Destination::S3), "S3");
        assert_eq!(destination_label(Destination::GDrive), "Google Drive");
    }

    /// T-6.3 audit MINOR #11: a `last_error` embedding an AWS access key must not
    /// reach the notification body verbatim — this is the exact shape a wrapped S3
    /// SDK error can take (e.g. echoing a signed request's `Credential=` value).
    #[test]
    fn failure_notification_body_redacts_a_secret_inside_last_error() {
        let body = failure_notification_body(
            "photo.png",
            Destination::S3,
            Some("AccessDenied for AKIAABCDEFGHIJKLMNOP: permission denied"),
        );
        assert!(
            !body.contains("AKIAABCDEFGHIJKLMNOP"),
            "body must not leak the access key: {body}"
        );
        assert!(
            body.contains("[REDACTED]"),
            "body should show the redaction marker: {body}"
        );
        assert!(body.starts_with("photo.png → S3: "));
    }

    #[test]
    fn failure_notification_body_falls_back_when_last_error_is_absent() {
        let body = failure_notification_body("photo.png", Destination::GDrive, None);
        assert_eq!(body, "photo.png → Google Drive: erro desconhecido");
    }

    #[test]
    fn failure_notification_body_still_truncates_after_redaction() {
        let long_secret_free = "e".repeat(200);
        let body = failure_notification_body("photo.png", Destination::S3, Some(&long_secret_free));
        assert!(body.ends_with('…'));
    }

    fn side(job_id: &str, status: JobStatus) -> JobSide {
        JobSide {
            job_id: job_id.to_string(),
            status,
            attempts: 0,
            next_attempt_at: None,
            remote_id: None,
            last_error: None,
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn sample_job() -> JobView {
        JobView {
            file_id: "file-1".to_string(),
            path: "C:/x/a.txt".to_string(),
            name: "a.txt".to_string(),
            size: 10,
            sha256: "sha".to_string(),
            detected_at: "2026-01-01T00:00:00Z".to_string(),
            gdrive: side("job-gdrive", JobStatus::Failed),
            s3: side("job-s3", JobStatus::Pending),
        }
    }

    #[test]
    fn matching_side_finds_the_gdrive_side_by_job_id() {
        let job = sample_job();
        let (dest, found) = matching_side(&job, "job-gdrive").expect("should match gdrive");
        assert_eq!(dest, Destination::GDrive);
        assert_eq!(found.status, JobStatus::Failed);
    }

    #[test]
    fn matching_side_finds_the_s3_side_by_job_id() {
        let job = sample_job();
        let (dest, found) = matching_side(&job, "job-s3").expect("should match s3");
        assert_eq!(dest, Destination::S3);
        assert_eq!(found.status, JobStatus::Pending);
    }

    #[test]
    fn matching_side_is_none_for_an_unrelated_job_id() {
        let job = sample_job();
        assert!(matching_side(&job, "job-unrelated").is_none());
    }
}
