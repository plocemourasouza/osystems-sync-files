//! `core::health` — periodic destination health monitor (SPEC.md §6
//! "health.rs"; SPEC.md §7 `AppStatus.destinations` / `status-changed` /
//! `auth-required`; PRD.md RF-068, RF-069, RF-032; PLAN.md T-3.8).
//!
//! Every `interval` (60 s per SPEC.md §6), [`probe`] calls
//! [`crate::uploaders::Uploader::test_connection`] for each configured
//! destination — Drive's `about` / S3's `head_bucket`, abstracted away
//! behind the `Uploader` trait this module doesn't otherwise depend on —
//! and only reports a change to the [`HealthSink`] when the resulting
//! [`DestinationsHealth`] snapshot actually differs from the previous one
//! (RF-068 AC: state change visible within one probe interval of the
//! underlying network loss, not spammed every tick).
//!
//! This module intentionally does not depend on `worker.rs` (written
//! concurrently, T-3.5): instead of borrowing its `Uploaders` type, it
//! defines its own minimal [`UploaderProvider`] trait that the app layer
//! implements over whatever holds the real uploaders.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use tokio::{
    sync::{Notify, RwLock},
    task::JoinHandle,
    time::timeout,
};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::{
    state::{Destination, DestinationHealth, DestinationsHealth},
    uploaders::{UploadError, Uploader},
};

/// Per-destination probe timeout (RNF-007: health checks must stay cheap
/// and never block the monitor loop indefinitely on a hung connection).
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Fallback `auth-required` hint (SPEC.md §7) used when the uploader's
/// `Auth` error carried no message of its own.
const DEFAULT_AUTH_HINT: &str = "Verifique credenciais/permissões";

/// Supplies the uploader for a destination, if one is currently configured
/// (RNF-007: a destination without credentials/an uploader is skipped —
/// reported simply as offline — rather than probed and logged as an
/// error). Implemented by the app layer over `worker::Uploaders` to avoid
/// `core::health` depending on `worker.rs`.
pub trait UploaderProvider: Send + Sync {
    fn uploader(&self, dest: Destination) -> Option<Arc<dyn Uploader>>;
}

/// Notified of health changes worth surfacing to the renderer (SPEC.md §7
/// events `status-changed` / `auth-required`).
pub trait HealthSink: Send + Sync {
    /// The full destinations snapshot, whenever it actually changed.
    fn changed(&self, health: DestinationsHealth);
    /// A destination's *offline/online → auth-required* transition, fired
    /// once per transition (never once per probe while it stays
    /// auth-required).
    fn auth_required(&self, dest: Destination, hint: String);
}

/// `DestinationHealth`/`DestinationsHealth` (`state::model`) derive
/// `Serialize` but not `PartialEq` — this crate must not edit that module
/// (owned by a concurrent task), so equality is checked structurally via
/// their JSON representation instead.
fn health_eq(a: &DestinationsHealth, b: &DestinationsHealth) -> bool {
    serde_json::to_value(a).expect("DestinationsHealth always serializes")
        == serde_json::to_value(b).expect("DestinationsHealth always serializes")
}

fn default_destination_health() -> DestinationHealth {
    DestinationHealth {
        online: false,
        auth_required: false,
        latency_ms: None,
    }
}

fn default_health() -> DestinationsHealth {
    DestinationsHealth {
        gdrive: default_destination_health(),
        s3: default_destination_health(),
    }
}

fn dest_field_mut(health: &mut DestinationsHealth, dest: Destination) -> &mut DestinationHealth {
    match dest {
        Destination::GDrive => &mut health.gdrive,
        Destination::S3 => &mut health.s3,
    }
}

/// Probes one destination (SPEC.md §6): no uploader configured is offline
/// by definition; a successful [`Uploader::test_connection`] reports the
/// reachability/latency it measured; an `Auth` error means the destination
/// is reachable but rejecting credentials, so it's reported as
/// `auth_required` (with `online: false` — a destination needing
/// reauthentication cannot serve uploads either way, and this keeps
/// "online" a simple "usable right now" signal for the statusbar rather
/// than a three-way state crammed into a bool); every other error
/// (`Transient`, `Permanent`, `Io`, `Cancelled`, or a probe timeout) is
/// offline. Runs with a [`PROBE_TIMEOUT`] ceiling so a hung connection
/// never stalls the monitor loop.
pub async fn probe(dest: Destination, provider: &dyn UploaderProvider) -> DestinationHealth {
    probe_with_hint(dest, provider).await.0
}

/// [`probe`]'s implementation, additionally returning the `Auth` error's
/// message (if any) for use as the `auth-required` hint — kept separate so
/// `probe`'s public signature stays exactly the plain-`DestinationHealth`
/// contract the rest of the crate (and its tests) depend on.
async fn probe_with_hint(
    dest: Destination,
    provider: &dyn UploaderProvider,
) -> (DestinationHealth, Option<String>) {
    let Some(uploader) = provider.uploader(dest) else {
        return (default_destination_health(), None);
    };

    match timeout(PROBE_TIMEOUT, uploader.test_connection()).await {
        Ok(Ok(result)) => (
            DestinationHealth {
                online: result.ok,
                auth_required: false,
                latency_ms: Some(result.latency_ms),
            },
            None,
        ),
        Ok(Err(UploadError::Auth(message))) => (
            DestinationHealth {
                online: false,
                auth_required: true,
                latency_ms: None,
            },
            Some(message).filter(|m| !m.is_empty()),
        ),
        Ok(Err(UploadError::Transient(message))) => {
            warn!(destination = ?dest, error = %message, "sonda de saúde: erro transitório, reportando offline");
            (default_destination_health(), None)
        }
        Ok(Err(UploadError::Io(err))) => {
            warn!(destination = ?dest, error = %err, "sonda de saúde: erro de I/O, reportando offline");
            (default_destination_health(), None)
        }
        Ok(Err(UploadError::Permanent(message))) => {
            warn!(destination = ?dest, error = %message, "sonda de saúde: erro permanente do uploader, reportando offline");
            (default_destination_health(), None)
        }
        Ok(Err(UploadError::Cancelled)) => (default_destination_health(), None),
        Err(_elapsed) => {
            warn!(destination = ?dest, timeout = ?PROBE_TIMEOUT, "sonda de saúde: tempo esgotado, reportando offline");
            (default_destination_health(), None)
        }
    }
}

/// Handle to a running [`spawn`]ned health monitor.
pub struct HealthHandle {
    state: Arc<RwLock<DestinationsHealth>>,
    /// Forces the very first state write (from either the loop or
    /// `set_auth_required`/`mark_online`) to be reported even though it
    /// happens to equal the all-offline `default_health()` the monitor
    /// starts from — otherwise a destination that never changes from
    /// offline would never be announced at all.
    has_emitted: Arc<AtomicBool>,
    sink: Arc<dyn HealthSink>,
    notify: Arc<Notify>,
    join: JoinHandle<()>,
}

impl HealthHandle {
    /// Current health snapshot (e.g. for `get_status`, SPEC.md §7).
    pub async fn snapshot(&self) -> DestinationsHealth {
        self.state.read().await.clone()
    }

    /// Requests an immediate probe instead of waiting for the next
    /// interval tick — used by the "Testar conexão" flow and right after
    /// new credentials are saved. Safe to call before the monitor loop has
    /// started running; the permit is held until it's ready to consume it.
    pub fn probe_now(&self) {
        self.notify.notify_one();
    }

    /// Sets (or clears) `auth_required` for `dest` directly, bypassing a
    /// probe — for the worker to call the moment it observes an
    /// `UploadError::Auth` mid-upload (RF-032), or for commands to clear it
    /// right after new credentials are saved, without waiting up to
    /// `interval` for the next scheduled probe to notice.
    pub async fn set_auth_required(&self, dest: Destination, required: bool) {
        let mut guard = self.state.write().await;
        let previous = guard.clone();
        let field = dest_field_mut(&mut guard, dest);
        let was_auth_required = field.auth_required;
        field.auth_required = required;
        if required {
            field.online = false;
        }

        let first = !self.has_emitted.swap(true, Ordering::SeqCst);
        if !first && health_eq(&previous, &guard) {
            return;
        }
        let snapshot = guard.clone();
        drop(guard);

        self.sink.changed(snapshot);
        if required && !was_auth_required {
            self.sink.auth_required(dest, DEFAULT_AUTH_HINT.to_string());
        }
    }

    /// Marks `dest` online and clears any `auth_required` flag — for the
    /// worker to call right after a successful upload, short-circuiting
    /// recovery instead of waiting for the next scheduled probe.
    pub async fn mark_online(&self, dest: Destination) {
        let mut guard = self.state.write().await;
        let previous = guard.clone();
        let field = dest_field_mut(&mut guard, dest);
        field.online = true;
        field.auth_required = false;

        let first = !self.has_emitted.swap(true, Ordering::SeqCst);
        if !first && health_eq(&previous, &guard) {
            return;
        }
        let snapshot = guard.clone();
        drop(guard);
        self.sink.changed(snapshot);
    }

    /// Waits for the background loop to exit. Used by callers that need to
    /// confirm shutdown completed (and by this module's own tests to
    /// confirm `cancel` actually stops the loop).
    pub async fn wait(self) -> Result<(), tokio::task::JoinError> {
        self.join.await
    }
}

/// Starts the periodic health monitor: probes both destinations through
/// `provider` every `interval`, reporting changes to `sink`, until
/// `cancel` fires.
pub fn spawn(
    provider: Arc<dyn UploaderProvider>,
    sink: Arc<dyn HealthSink>,
    interval: Duration,
    cancel: CancellationToken,
) -> HealthHandle {
    let state = Arc::new(RwLock::new(default_health()));
    let has_emitted = Arc::new(AtomicBool::new(false));
    let notify = Arc::new(Notify::new());

    let loop_state = state.clone();
    let loop_has_emitted = has_emitted.clone();
    let loop_sink = sink.clone();
    let loop_notify = notify.clone();
    let loop_cancel = cancel.clone();
    let join = tokio::spawn(async move {
        run_loop(
            provider,
            loop_sink,
            loop_state,
            loop_has_emitted,
            loop_notify,
            interval,
            loop_cancel,
        )
        .await;
    });

    HealthHandle {
        state,
        has_emitted,
        sink,
        notify,
        join,
    }
}

/// The monitor's main loop: wait for whichever of "interval elapsed",
/// "probe requested" (`probe_now`) or "cancelled" comes first, then — the
/// two branches (1) and (2) don't cancel the loop — run one [`tick`], and
/// repeat.
async fn run_loop(
    provider: Arc<dyn UploaderProvider>,
    sink: Arc<dyn HealthSink>,
    state: Arc<RwLock<DestinationsHealth>>,
    has_emitted: Arc<AtomicBool>,
    notify: Arc<Notify>,
    interval: Duration,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(interval) => {},
            _ = notify.notified() => {},
        }
        if cancel.is_cancelled() {
            return;
        }

        tick(&provider, &sink, &state, &has_emitted).await;
    }
}

/// Probes both destinations concurrently, and — only when the resulting
/// snapshot actually differs from the previous one — reports it to `sink`
/// (once) followed by any `auth-required` transitions it contains (SPEC.md
/// §7: `status-changed` then `auth-required`).
async fn tick(
    provider: &Arc<dyn UploaderProvider>,
    sink: &Arc<dyn HealthSink>,
    state: &Arc<RwLock<DestinationsHealth>>,
    has_emitted: &Arc<AtomicBool>,
) {
    let ((gdrive, gdrive_hint), (s3, s3_hint)) = tokio::join!(
        probe_with_hint(Destination::GDrive, provider.as_ref()),
        probe_with_hint(Destination::S3, provider.as_ref()),
    );

    let mut guard = state.write().await;
    let previous = guard.clone();
    guard.gdrive = gdrive;
    guard.s3 = s3;

    let first = !has_emitted.swap(true, Ordering::SeqCst);
    if !first && health_eq(&previous, &guard) {
        return;
    }
    let snapshot = guard.clone();
    drop(guard);

    sink.changed(snapshot.clone());

    if snapshot.gdrive.auth_required && !previous.gdrive.auth_required {
        sink.auth_required(
            Destination::GDrive,
            gdrive_hint.unwrap_or_else(|| DEFAULT_AUTH_HINT.to_string()),
        );
    }
    if snapshot.s3.auth_required && !previous.s3.auth_required {
        sink.auth_required(
            Destination::S3,
            s3_hint.unwrap_or_else(|| DEFAULT_AUTH_HINT.to_string()),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, VecDeque},
        sync::Mutex,
    };

    use async_trait::async_trait;

    use super::*;
    use crate::uploaders::{TestResult, UploadRequest, UploadResult};

    /// Returns a canned sequence of `test_connection` results, one per
    /// call; once exhausted, keeps returning a healthy default so a test
    /// that ticks more times than it scripted doesn't panic.
    struct ScriptedUploader {
        dest: Destination,
        script: Mutex<VecDeque<Result<TestResult, UploadError>>>,
    }

    impl ScriptedUploader {
        fn new(dest: Destination, script: Vec<Result<TestResult, UploadError>>) -> Self {
            Self {
                dest,
                script: Mutex::new(script.into()),
            }
        }
    }

    #[async_trait]
    impl Uploader for ScriptedUploader {
        fn id(&self) -> Destination {
            self.dest
        }

        async fn test_connection(&self) -> Result<TestResult, UploadError> {
            self.script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(TestResult {
                    ok: true,
                    message: "default".to_string(),
                    latency_ms: 1,
                }))
        }

        async fn upload(&self, _req: UploadRequest) -> Result<UploadResult, UploadError> {
            unimplemented!("core::health only calls test_connection")
        }
    }

    struct MapProvider {
        uploaders: HashMap<Destination, Arc<dyn Uploader>>,
    }

    impl UploaderProvider for MapProvider {
        fn uploader(&self, dest: Destination) -> Option<Arc<dyn Uploader>> {
            self.uploaders.get(&dest).cloned()
        }
    }

    #[derive(Default)]
    struct RecordingSink {
        changed: Mutex<Vec<DestinationsHealth>>,
        auth_required: Mutex<Vec<(Destination, String)>>,
    }

    impl HealthSink for RecordingSink {
        fn changed(&self, health: DestinationsHealth) {
            self.changed.lock().unwrap().push(health);
        }

        fn auth_required(&self, dest: Destination, hint: String) {
            self.auth_required.lock().unwrap().push((dest, hint));
        }
    }

    fn ok(latency_ms: u32) -> Result<TestResult, UploadError> {
        Ok(TestResult {
            ok: true,
            message: "ok".to_string(),
            latency_ms,
        })
    }

    // -- direct `probe()` tests -------------------------------------------------

    #[tokio::test]
    async fn probe_without_an_uploader_is_offline() {
        let provider = MapProvider {
            uploaders: HashMap::new(),
        };
        let health = probe(Destination::GDrive, &provider).await;
        assert!(!health.online);
        assert!(!health.auth_required);
        assert_eq!(health.latency_ms, None);
    }

    #[tokio::test]
    async fn probe_reports_ok_and_latency_from_a_successful_test_connection() {
        let uploader: Arc<dyn Uploader> =
            Arc::new(ScriptedUploader::new(Destination::S3, vec![ok(37)]));
        let provider = MapProvider {
            uploaders: HashMap::from([(Destination::S3, uploader)]),
        };
        let health = probe(Destination::S3, &provider).await;
        assert!(health.online);
        assert!(!health.auth_required);
        assert_eq!(health.latency_ms, Some(37));
    }

    #[tokio::test]
    async fn probe_reports_latency_even_when_test_connection_says_not_ok() {
        let uploader: Arc<dyn Uploader> = Arc::new(ScriptedUploader::new(
            Destination::S3,
            vec![Ok(TestResult {
                ok: false,
                message: "degraded".to_string(),
                latency_ms: 999,
            })],
        ));
        let provider = MapProvider {
            uploaders: HashMap::from([(Destination::S3, uploader)]),
        };
        let health = probe(Destination::S3, &provider).await;
        assert!(!health.online);
        assert!(!health.auth_required);
        assert_eq!(health.latency_ms, Some(999));
    }

    #[tokio::test]
    async fn probe_maps_auth_error_to_auth_required_and_offline() {
        let uploader: Arc<dyn Uploader> = Arc::new(ScriptedUploader::new(
            Destination::GDrive,
            vec![Err(UploadError::Auth("invalid_grant".to_string()))],
        ));
        let provider = MapProvider {
            uploaders: HashMap::from([(Destination::GDrive, uploader)]),
        };
        let health = probe(Destination::GDrive, &provider).await;
        assert!(!health.online);
        assert!(health.auth_required);
        assert_eq!(health.latency_ms, None);
    }

    #[tokio::test]
    async fn probe_treats_transient_permanent_io_and_cancelled_as_plain_offline() {
        let errors = vec![
            UploadError::Transient("timeout".to_string()),
            UploadError::Permanent("bad request".to_string()),
            UploadError::Io(std::io::Error::other("disk lock")),
            UploadError::Cancelled,
        ];
        for err in errors {
            let uploader: Arc<dyn Uploader> =
                Arc::new(ScriptedUploader::new(Destination::S3, vec![Err(err)]));
            let provider = MapProvider {
                uploaders: HashMap::from([(Destination::S3, uploader)]),
            };
            let health = probe(Destination::S3, &provider).await;
            assert!(!health.online);
            assert!(!health.auth_required);
            assert_eq!(health.latency_ms, None);
        }
    }

    // -- `spawn`ned monitor loop tests -------------------------------------------

    /// Yields to the scheduler until `cond` holds (or panics after a
    /// generous budget). None of this crosses real time — every future
    /// involved in a tick resolves without its own timer — so this is
    /// purely "give the woken loop task enough turns to finish," robust to
    /// exactly how many polls that happens to take.
    async fn wait_until(mut cond: impl FnMut() -> bool) {
        for _ in 0..10_000 {
            if cond() {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("condition was not met within the polling budget");
    }

    /// Requests an immediate probe and waits for `sink` to have recorded at
    /// least `expect_changed_len` `changed` calls. These tests use
    /// `probe_now` (a plain `Notify`, resolved directly by the executor)
    /// rather than letting `interval` elapse under a paused clock: a
    /// `tokio::time::sleep` only becomes ready once the runtime's timer
    /// driver gets a turn, which — unlike a `Notify` wakeup — isn't
    /// guaranteed by a bounded number of `yield_now` calls, making
    /// interval-elapsed ticks an unreliable way to drive *these*
    /// assertions. [`periodic_interval_triggers_a_probe_without_probe_now`]
    /// below separately covers that the interval timer path works at all.
    async fn trigger_tick(handle: &HealthHandle, sink: &RecordingSink, expect_changed_len: usize) {
        handle.probe_now();
        wait_until(|| sink.changed.lock().unwrap().len() >= expect_changed_len).await;
    }

    /// Requests an immediate probe and gives the woken loop task a
    /// generous, unconditional number of scheduler turns to finish it —
    /// for steps that (correctly) expect *no* new `changed` call, so there
    /// is no "count reached" condition to wait on.
    async fn trigger_tick_expecting_no_change(handle: &HealthHandle) {
        handle.probe_now();
        for _ in 0..200 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn online_offline_auth_required_transitions_emit_three_changed_and_one_auth_required() {
        let script = vec![
            ok(10),
            Ok(TestResult {
                ok: false,
                message: "down".to_string(),
                latency_ms: 5000,
            }),
            Err(UploadError::Auth("token expired".to_string())),
        ];
        let uploader: Arc<dyn Uploader> = Arc::new(ScriptedUploader::new(Destination::S3, script));
        let provider: Arc<dyn UploaderProvider> = Arc::new(MapProvider {
            uploaders: HashMap::from([(Destination::S3, uploader)]),
        });
        let sink = Arc::new(RecordingSink::default());
        let cancel = CancellationToken::new();
        // Large and irrelevant: every tick below is driven by `probe_now`,
        // not by this interval elapsing.
        let handle = spawn(
            provider,
            sink.clone(),
            Duration::from_secs(3600),
            cancel.clone(),
        );

        trigger_tick(&handle, &sink, 1).await; // online
        trigger_tick(&handle, &sink, 2).await; // offline
        trigger_tick(&handle, &sink, 3).await; // auth-required

        cancel.cancel();
        handle.wait().await.expect("loop task must not panic");

        let changed = sink.changed.lock().unwrap();
        assert_eq!(
            changed.len(),
            3,
            "expected exactly 3 `changed` calls: {changed:?}"
        );
        assert!(changed[0].s3.online && !changed[0].s3.auth_required);
        assert!(!changed[1].s3.online && !changed[1].s3.auth_required);
        assert!(!changed[2].s3.online && changed[2].s3.auth_required);

        let auth = sink.auth_required.lock().unwrap();
        assert_eq!(
            auth.len(),
            1,
            "expected exactly 1 `auth_required` call: {auth:?}"
        );
        assert_eq!(auth[0], (Destination::S3, "token expired".to_string()));
    }

    #[tokio::test(start_paused = true)]
    async fn unchanged_consecutive_probes_emit_nothing_after_the_first() {
        let script = vec![ok(10), ok(10), ok(10)];
        let uploader: Arc<dyn Uploader> = Arc::new(ScriptedUploader::new(Destination::S3, script));
        let provider: Arc<dyn UploaderProvider> = Arc::new(MapProvider {
            uploaders: HashMap::from([(Destination::S3, uploader)]),
        });
        let sink = Arc::new(RecordingSink::default());
        let cancel = CancellationToken::new();
        let handle = spawn(
            provider,
            sink.clone(),
            Duration::from_secs(3600),
            cancel.clone(),
        );

        trigger_tick(&handle, &sink, 1).await;
        trigger_tick_expecting_no_change(&handle).await;
        trigger_tick_expecting_no_change(&handle).await;

        cancel.cancel();
        handle.wait().await.expect("loop task must not panic");

        assert_eq!(sink.changed.lock().unwrap().len(), 1);
        assert!(sink.auth_required.lock().unwrap().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn no_uploader_configured_is_offline_and_emits_only_the_first_change() {
        let provider: Arc<dyn UploaderProvider> = Arc::new(MapProvider {
            uploaders: HashMap::new(),
        });
        let sink = Arc::new(RecordingSink::default());
        let cancel = CancellationToken::new();
        let handle = spawn(
            provider,
            sink.clone(),
            Duration::from_secs(3600),
            cancel.clone(),
        );

        trigger_tick(&handle, &sink, 1).await;
        trigger_tick_expecting_no_change(&handle).await;
        trigger_tick_expecting_no_change(&handle).await;

        cancel.cancel();
        handle.wait().await.expect("loop task must not panic");

        let changed = sink.changed.lock().unwrap();
        assert_eq!(changed.len(), 1);
        assert!(!changed[0].gdrive.online && !changed[0].gdrive.auth_required);
        assert_eq!(changed[0].gdrive.latency_ms, None);
        assert!(!changed[0].s3.online && !changed[0].s3.auth_required);
        assert_eq!(changed[0].s3.latency_ms, None);
        assert!(sink.auth_required.lock().unwrap().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn probe_now_triggers_a_probe_without_waiting_for_the_interval() {
        let uploader: Arc<dyn Uploader> =
            Arc::new(ScriptedUploader::new(Destination::GDrive, vec![ok(5)]));
        let provider: Arc<dyn UploaderProvider> = Arc::new(MapProvider {
            uploaders: HashMap::from([(Destination::GDrive, uploader)]),
        });
        let sink = Arc::new(RecordingSink::default());
        let cancel = CancellationToken::new();
        // An interval long enough that only `probe_now` (not time passing)
        // could explain a probe having happened.
        let handle = spawn(
            provider,
            sink.clone(),
            Duration::from_secs(3600),
            cancel.clone(),
        );

        handle.probe_now();
        wait_until(|| !sink.changed.lock().unwrap().is_empty()).await;

        {
            let changed = sink.changed.lock().unwrap();
            assert_eq!(changed.len(), 1);
            assert!(changed[0].gdrive.online);
            assert_eq!(changed[0].gdrive.latency_ms, Some(5));
        }

        cancel.cancel();
        handle.wait().await.expect("loop task must not panic");
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_stops_the_background_loop_promptly() {
        let provider: Arc<dyn UploaderProvider> = Arc::new(MapProvider {
            uploaders: HashMap::new(),
        });
        let sink = Arc::new(RecordingSink::default());
        let cancel = CancellationToken::new();
        let handle = spawn(provider, sink, Duration::from_secs(3600), cancel.clone());

        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(5), handle.wait())
            .await
            .expect("loop task should exit promptly after cancellation")
            .expect("loop task should not panic");
    }

    /// Confirms the interval itself (not just `probe_now`) drives a probe —
    /// RF-068's AC that a destination's state is visible within one probe
    /// interval of the underlying event. Advances the paused clock in many
    /// small steps rather than one big jump: each `tokio::time::advance`
    /// call yields once internally, and the runtime's timer driver needs
    /// one of those turns to notice the elapsed `interval` and wake the
    /// loop task — a single big jump isn't reliably enough turns for that
    /// wakeup to be observed by this test's assertions.
    #[tokio::test(start_paused = true)]
    async fn periodic_interval_triggers_a_probe_without_probe_now() {
        let uploader: Arc<dyn Uploader> =
            Arc::new(ScriptedUploader::new(Destination::S3, vec![ok(9)]));
        let provider: Arc<dyn UploaderProvider> = Arc::new(MapProvider {
            uploaders: HashMap::from([(Destination::S3, uploader)]),
        });
        let sink = Arc::new(RecordingSink::default());
        let cancel = CancellationToken::new();
        let interval = Duration::from_secs(60);
        let handle = spawn(provider, sink.clone(), interval, cancel.clone());

        let step = Duration::from_millis(500);
        let budget = interval * 4; // generous: several intervals' worth of steps
        let mut waited = Duration::ZERO;
        while sink.changed.lock().unwrap().is_empty() && waited < budget {
            tokio::time::advance(step).await;
            tokio::task::yield_now().await;
            waited += step;
        }

        {
            let changed = sink.changed.lock().unwrap();
            assert!(
                !changed.is_empty(),
                "expected the periodic loop to have probed within {budget:?} of virtual time"
            );
            assert!(changed[0].s3.online);
            assert_eq!(changed[0].s3.latency_ms, Some(9));
        }

        cancel.cancel();
        handle.wait().await.expect("loop task must not panic");
    }

    #[tokio::test]
    async fn snapshot_defaults_to_offline_before_any_probe() {
        let provider: Arc<dyn UploaderProvider> = Arc::new(MapProvider {
            uploaders: HashMap::new(),
        });
        let sink = Arc::new(RecordingSink::default());
        let cancel = CancellationToken::new();
        let handle = spawn(provider, sink, Duration::from_secs(3600), cancel.clone());

        let snapshot = handle.snapshot().await;
        assert!(!snapshot.gdrive.online);
        assert!(!snapshot.s3.online);

        cancel.cancel();
        handle.wait().await.expect("loop task must not panic");
    }

    #[tokio::test(start_paused = true)]
    async fn set_auth_required_emits_once_per_transition_and_ignores_repeats() {
        let provider: Arc<dyn UploaderProvider> = Arc::new(MapProvider {
            uploaders: HashMap::new(),
        });
        let sink = Arc::new(RecordingSink::default());
        let cancel = CancellationToken::new();
        let handle = spawn(
            provider,
            sink.clone(),
            Duration::from_secs(3600),
            cancel.clone(),
        );

        handle.set_auth_required(Destination::S3, true).await;
        handle.set_auth_required(Destination::S3, true).await; // repeat: no new emit
        handle.set_auth_required(Destination::S3, false).await;

        cancel.cancel();
        handle.wait().await.expect("loop task must not panic");

        let auth = sink.auth_required.lock().unwrap();
        assert_eq!(auth.len(), 1);
        assert_eq!(auth[0].0, Destination::S3);

        let changed = sink.changed.lock().unwrap();
        assert_eq!(changed.len(), 2, "true, then false; the repeat is a no-op");
        assert!(changed[0].s3.auth_required);
        assert!(!changed[1].s3.auth_required);
    }

    #[tokio::test(start_paused = true)]
    async fn mark_online_clears_auth_required_and_emits_once() {
        let provider: Arc<dyn UploaderProvider> = Arc::new(MapProvider {
            uploaders: HashMap::new(),
        });
        let sink = Arc::new(RecordingSink::default());
        let cancel = CancellationToken::new();
        let handle = spawn(
            provider,
            sink.clone(),
            Duration::from_secs(3600),
            cancel.clone(),
        );

        handle.set_auth_required(Destination::GDrive, true).await;
        handle.mark_online(Destination::GDrive).await;
        handle.mark_online(Destination::GDrive).await; // repeat: no new emit

        cancel.cancel();
        handle.wait().await.expect("loop task must not panic");

        let changed = sink.changed.lock().unwrap();
        assert_eq!(changed.len(), 2);
        assert!(changed[1].gdrive.online);
        assert!(!changed[1].gdrive.auth_required);
    }
}
