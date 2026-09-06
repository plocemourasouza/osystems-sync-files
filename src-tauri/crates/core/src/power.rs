//! `core::power` — keep-awake + sleep/resume detection (SPEC.md §6
//! "power.rs"; PRD.md RF-093, RF-094; PLAN.md T-5.1).
//!
//! Two independent pieces:
//!
//! - [`KeepAwake`]: while enabled, tells Windows not to let the machine
//!   sleep (never forces the display on) via `SetThreadExecutionState`
//!   (RF-093). That flag is per-*thread* — only the last call made from a
//!   given thread is in effect — so `KeepAwake` owns one dedicated
//!   `std::thread` for its whole lifetime and funnels every `set()` call,
//!   whichever async task makes it, through that single thread over an
//!   `mpsc` channel.
//! - [`spawn_resume_detector`]: a Tokio loop that ticks every `interval`
//!   and notices when far more wall-clock time passed than the loop was
//!   actually asleep-in-`await` for (SPEC.md: `elapsed > 2×interval`) —
//!   the signature of the OS having suspended the whole process — and
//!   reports that gap to a [`ResumeSink`]. The app layer wires the sink to
//!   `rescan()` (RF-094: pending files processed within 60 s of waking),
//!   the same way `core::health` decouples itself from `worker.rs` via its
//!   own sink trait rather than depending on it directly.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};

use tokio::{task::JoinHandle, time::Instant};
use tokio_util::sync::CancellationToken;
use tracing::info;

#[cfg(windows)]
use windows::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED};

/// Keeps the system from sleeping while enabled, without forcing the
/// display on (RF-093, SPEC.md §6: `ES_CONTINUOUS | ES_SYSTEM_REQUIRED`,
/// no `ES_DISPLAY_REQUIRED`).
///
/// `SetThreadExecutionState` is a per-thread flag: it only stays in effect
/// for as long as the thread that last called it stays alive and doesn't
/// call it again with a different value. Calling it from whichever async
/// task happens to invoke [`KeepAwake::set`] (a different OS thread on
/// every call, under Tokio's multi-thread scheduler) would make the state
/// flicker unpredictably. Instead, `KeepAwake` spawns one dedicated
/// `std::thread` for its entire lifetime and serializes every `set()` call
/// through it via an `mpsc` channel — `SetThreadExecutionState` is always
/// called from that same thread.
pub struct KeepAwake {
    active: AtomicBool,
    tx: Option<mpsc::Sender<bool>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl KeepAwake {
    /// Spawns the dedicated worker thread. If the OS refuses to create it
    /// (an exceptional, effectively unrecoverable condition), `set()`
    /// silently becomes a no-op rather than panicking — keep-awake is a
    /// best-effort convenience (RF-093 is a "Should", not required for
    /// correctness of uploads themselves).
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<bool>();
        let worker = match std::thread::Builder::new()
            .name("osystems-sync-keep-awake".into())
            .spawn(move || keep_awake_thread(rx))
        {
            Ok(handle) => Some(handle),
            Err(error) => {
                tracing::warn!(%error, "falha ao iniciar thread de keep-awake; keep-awake desabilitado");
                None
            }
        };

        Self {
            active: AtomicBool::new(false),
            tx: worker.as_ref().map(|_| tx),
            worker,
        }
    }

    /// Enables or disables keep-awake. Re-asserting the same value is
    /// intentionally not deduplicated: forwarding it to the dedicated
    /// thread every time is exactly what re-arms the OS's execution-state
    /// timeout before it can lapse.
    pub fn set(&self, enabled: bool) {
        self.active.store(enabled, Ordering::SeqCst);
        if let Some(tx) = &self.tx {
            // The channel can only be disconnected if the worker thread
            // panicked; either way there is nothing more `set` can do.
            let _ = tx.send(enabled);
        }
    }

    /// Whether keep-awake is currently enabled, per the last [`Self::set`]
    /// call (reflects intent, not confirmation that the OS applied it).
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }
}

impl Default for KeepAwake {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        // Clear the execution state before exiting: drop the sender after
        // asking the worker to apply `false`, so its `for enabled in rx`
        // loop sees the disconnect and returns right after handling it,
        // instead of blocking on `recv` forever.
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(false);
            drop(tx);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The dedicated thread's body: applies every command it receives, in
/// order, until the channel disconnects (the [`KeepAwake`] was dropped).
fn keep_awake_thread(rx: mpsc::Receiver<bool>) {
    for enabled in rx {
        apply_execution_state(enabled);
    }
}

#[cfg(windows)]
fn apply_execution_state(enabled: bool) {
    let flags = if enabled {
        ES_CONTINUOUS | ES_SYSTEM_REQUIRED
    } else {
        ES_CONTINUOUS
    };
    // Safety: `SetThreadExecutionState` takes a plain flags value and
    // touches no memory this process owns; it has no other preconditions.
    unsafe {
        SetThreadExecutionState(flags);
    }
}

#[cfg(not(windows))]
fn apply_execution_state(enabled: bool) {
    tracing::debug!(enabled, "keep-awake não tem efeito fora do Windows");
}

/// Notified when [`spawn_resume_detector`] concludes the machine just woke
/// from sleep. Implemented by the app layer to trigger `rescan()`
/// (RF-094), the same seam `core::health`'s `HealthSink` uses to avoid
/// `core::power` depending on `rescan.rs` directly.
pub trait ResumeSink: Send + Sync {
    fn resumed(&self, gap: Duration);
}

/// Pure check: `None` unless more than `2×interval` elapsed between `last`
/// and `now` (SPEC.md §6) — a gap a live, un-suspended timer loop could
/// never produce on its own, since each of its ticks is at most `interval`
/// apart. Kept free of `spawn_resume_detector`'s loop/channel machinery so
/// it can be exercised directly with plain instants.
pub fn detect_gap(last: Instant, now: Instant, interval: Duration) -> Option<Duration> {
    let elapsed = now.saturating_duration_since(last);
    if elapsed > interval * 2 {
        Some(elapsed)
    } else {
        None
    }
}

/// Starts the resume-detection loop (RF-094): every `interval`, checks
/// whether more than `2×interval` passed since the previous tick and, if
/// so, reports the gap to `sink`. Runs until `cancel` fires.
pub fn spawn_resume_detector(
    interval: Duration,
    sink: Arc<dyn ResumeSink>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // `interval()` fires immediately on creation; consume that tick so
        // `last` below is a real baseline, not a fake tick at startup.
        ticker.tick().await;
        let mut last = Instant::now();

        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = ticker.tick() => {
                    // Read the clock ourselves instead of trusting the
                    // tick's own return value: under `MissedTickBehavior::
                    // Delay` after a long pause, the returned instant can
                    // reflect the original (skipped) deadline rather than
                    // how much wall-clock time actually passed.
                    let now = Instant::now();
                    if let Some(gap) = detect_gap(last, now, interval) {
                        info!(gap_secs = gap.as_secs(), "retomada do sistema detectada");
                        sink.resumed(gap);
                    }
                    last = now;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // --- detect_gap -----------------------------------------------------

    #[test]
    fn detect_gap_table() {
        let interval = Duration::from_secs(60);
        let base = Instant::now();

        let cases: &[(u64, Option<u64>)] = &[
            (0, None),
            (60, None),
            (119, None),
            (120, None), // boundary: not strictly greater than 2x interval
            (121, Some(121)),
            (200, Some(200)),
        ];

        for &(elapsed_secs, expected_secs) in cases {
            let now = base + Duration::from_secs(elapsed_secs);
            let got = detect_gap(base, now, interval);
            let expected = expected_secs.map(Duration::from_secs);
            assert_eq!(
                got, expected,
                "elapsed={elapsed_secs}s interval={interval:?}"
            );
        }
    }

    #[test]
    fn detect_gap_never_goes_backwards_in_time() {
        let interval = Duration::from_secs(60);
        let now = Instant::now();
        let later = now + Duration::from_secs(10);
        // `last` after `now`: `saturating_duration_since` clamps to zero
        // instead of panicking/overflowing.
        assert_eq!(detect_gap(later, now, interval), None);
    }

    // --- spawn_resume_detector -------------------------------------------

    #[derive(Default)]
    struct RecordingSink {
        gaps: Mutex<Vec<Duration>>,
    }

    impl ResumeSink for RecordingSink {
        fn resumed(&self, gap: Duration) {
            self.gaps.lock().unwrap().push(gap);
        }
    }

    /// Lets every currently-runnable task (in particular the detector
    /// loop, once its timer future is ready) actually run, without
    /// advancing the paused clock any further.
    async fn drain_ready_tasks() {
        for _ in 0..200 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn normal_tick_does_not_report_a_gap() {
        let interval = Duration::from_secs(60);
        let sink = Arc::new(RecordingSink::default());
        let cancel = CancellationToken::new();
        let _handle = spawn_resume_detector(interval, sink.clone(), cancel.clone());

        tokio::time::advance(Duration::from_secs(60)).await;
        drain_ready_tasks().await;

        assert!(
            sink.gaps.lock().unwrap().is_empty(),
            "a single on-schedule tick must not be treated as a resume"
        );

        cancel.cancel();
    }

    #[tokio::test(start_paused = true)]
    async fn large_jump_reports_exactly_one_resume_with_the_full_gap() {
        let interval = Duration::from_secs(60);
        let sink = Arc::new(RecordingSink::default());
        let cancel = CancellationToken::new();
        let handle = spawn_resume_detector(interval, sink.clone(), cancel.clone());

        // One normal tick first, matching how the detector is actually
        // used (it has been running a while before the machine sleeps).
        tokio::time::advance(Duration::from_secs(60)).await;
        drain_ready_tasks().await;
        assert!(sink.gaps.lock().unwrap().is_empty());

        // Simulate the machine sleeping for ~200s: jump the clock in one
        // shot rather than ticking through it.
        tokio::time::advance(Duration::from_secs(200)).await;
        drain_ready_tasks().await;

        {
            let gaps = sink.gaps.lock().unwrap();
            assert_eq!(gaps.len(), 1, "expected exactly one resume: {gaps:?}");
            let gap = gaps[0];
            assert!(
                gap >= Duration::from_secs(200) && gap < Duration::from_secs(210),
                "gap {gap:?} should be ~200s"
            );
        }

        cancel.cancel();
        handle.await.expect("detector task must not panic");
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_stops_the_loop() {
        let interval = Duration::from_secs(60);
        let sink = Arc::new(RecordingSink::default());
        let cancel = CancellationToken::new();
        let handle = spawn_resume_detector(interval, sink.clone(), cancel.clone());

        cancel.cancel();
        handle
            .await
            .expect("detector task must exit cleanly on cancel");

        // Advancing well past the interval after cancellation must not
        // resurrect the loop or call the sink.
        tokio::time::advance(Duration::from_secs(600)).await;
        drain_ready_tasks().await;
        assert!(sink.gaps.lock().unwrap().is_empty());
    }

    // --- KeepAwake ---------------------------------------------------------

    #[cfg(not(windows))]
    #[test]
    fn keep_awake_set_is_a_no_op_off_windows_and_does_not_panic() {
        let keep_awake = KeepAwake::new();
        assert!(!keep_awake.is_active());

        keep_awake.set(true);
        assert!(keep_awake.is_active());

        keep_awake.set(false);
        assert!(!keep_awake.is_active());

        // Dropping must join the worker thread rather than hang.
        drop(keep_awake);
    }

    #[cfg(windows)]
    #[test]
    fn keep_awake_set_toggles_execution_state_on_windows_without_panicking() {
        let keep_awake = KeepAwake::new();

        keep_awake.set(true);
        assert!(keep_awake.is_active());

        keep_awake.set(false);
        assert!(!keep_awake.is_active());

        drop(keep_awake);
    }
}
