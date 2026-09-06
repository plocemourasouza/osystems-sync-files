//! `core::throttle` — Fase 3 (T-3.6). Bandwidth limiting for uploaders.
//!
//! Implements the token-bucket contract from `SPEC.md §6` (`throttle.rs`,
//! RF11) and the accuracy/hot-reload/sharing requirements from `PRD.md`
//! RF-050/RF-051/RF-052:
//!
//! - [`Throttle`] is a single shared token bucket, one `Arc<Throttle>` per
//!   *destination*, shared by every worker of that destination (RF-052) so
//!   that concurrent workers never together exceed the configured cap.
//! - [`Throttle::set_limit`] hot-reloads the limit (RF-051): a change is
//!   observed by every in-flight [`Throttle::acquire`] call within at most
//!   100 ms (bucket refill / recheck interval), well under the 2 s budget.
//! - [`ThrottledReader`] wraps an [`tokio::io::AsyncRead`] body (the upload
//!   stream) so the limit applies transparently to simple and multipart /
//!   resumable uploads alike.
//! - [`ThroughputMeter`] is the *measurement* side (separate from limiting):
//!   a 5 s sliding-window byte counter used to report `rate_bps` in the
//!   `upload-progress` event.
//!
//! All internal timing uses [`tokio::time::Instant`]/`sleep` (not
//! `std::time::Instant`) so that behaviour is deterministic and instant
//! under `#[tokio::test(start_paused = true)]`.

use std::collections::VecDeque;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::task::{Context, Poll};

use chrono::NaiveTime;
use thiserror::Error;
use tokio::io::{AsyncRead, ReadBuf};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// Internal token-bucket state. Guarded by [`Throttle::state`] behind a
/// plain (synchronous) `std::sync::Mutex` — the lock is only ever held for
/// the duration of a small arithmetic update, never across an `.await>`, so
/// [`Throttle::acquire`] stays cancel-safe (dropping the future mid-wait
/// leaves the bucket untouched: tokens are only ever debited immediately
/// before returning).
struct Bucket {
    /// Tokens currently available, in bytes. Starts at `0.0` (not full) so
    /// that a freshly created `Throttle` paces from the first byte instead
    /// of allowing an initial one-second burst — this is what makes "10 MB
    /// at a 1 MB/s cap takes ~10 s" hold from `t = 0`.
    tokens: f64,
    /// Instant of the last refill, used to compute how many tokens have
    /// accrued since.
    last: Instant,
}

/// A shared, hot-reloadable token bucket bandwidth limiter.
///
/// One instance is created per destination (S3, Google Drive, …) and its
/// `Arc` handle is cloned into every worker task for that destination, so
/// the limit is enforced on the *combined* throughput of all of them
/// (RF-052) rather than per-worker.
pub struct Throttle {
    /// Configured limit in bytes/second. `0` means unlimited. `Atomic` so
    /// [`Throttle::set_limit`] can be called from the config/save-config
    /// path without needing to reach into the bucket lock.
    limit_bps: AtomicU64,
    /// Token-bucket bookkeeping (tokens available, last refill instant).
    state: Mutex<Bucket>,
    /// Wakes any task parked inside [`Throttle::acquire`] as soon as
    /// [`Throttle::set_limit`] changes the rate, so the new slope is picked
    /// up immediately instead of only after the current sleep elapses.
    notify: Notify,
}

impl Throttle {
    /// Creates a new throttle with the given limit (bytes/second, `0` =
    /// unlimited), ready to be shared across workers.
    pub fn new(limit_bps: u64) -> Arc<Self> {
        Arc::new(Self {
            limit_bps: AtomicU64::new(limit_bps),
            state: Mutex::new(Bucket {
                tokens: 0.0,
                last: Instant::now(),
            }),
            notify: Notify::new(),
        })
    }

    /// Current limit in bytes/second (`0` = unlimited).
    pub fn limit_bps(&self) -> u64 {
        self.limit_bps.load(Ordering::Relaxed)
    }

    /// Hot-reloads the limit (RF-051). `0` disables throttling. Wakes every
    /// task currently blocked in [`Throttle::acquire`] so the new rate
    /// applies immediately rather than after the in-flight sleep elapses.
    pub fn set_limit(&self, limit_bps: u64) {
        let previous = self.limit_bps.swap(limit_bps, Ordering::Relaxed);
        if previous != limit_bps {
            tracing::debug!(
                previous_bps = previous,
                new_bps = limit_bps,
                "limite de throttle atualizado"
            );
        }
        self.notify.notify_waiters();
    }

    /// Blocks (asynchronously) until `bytes` worth of tokens are available,
    /// then debits them. Returns immediately when unlimited (`limit_bps ==
    /// 0`).
    ///
    /// Token bucket: tokens refill continuously at `limit_bps` bytes/second,
    /// capped at `limit_bps` tokens (i.e. burst = 1 second worth of the
    /// configured rate). The wait is re-evaluated at least every 100 ms (or
    /// sooner, if [`Throttle::set_limit`] wakes it), which is also the
    /// refill granularity implied by `SPEC.md`.
    ///
    /// **Oversized chunks:** if `bytes` is larger than the burst capacity
    /// (one second of the current rate), it can never be paid for exactly —
    /// the bucket would have to hold more than its cap. In that case the
    /// whole chunk is released once the bucket has fully refilled (reached
    /// its cap), consuming it entirely. This keeps the long-run *average*
    /// rate correct (which is what RF-050's ±10% accuracy target measures)
    /// even though that single oversized chunk is not metered byte-for-byte
    /// against the cap. Callers that care about tight accuracy (e.g.
    /// [`ThrottledReader`]) should keep `bytes` at or below the burst size.
    pub async fn acquire(&self, bytes: usize) {
        if bytes == 0 {
            return;
        }

        loop {
            let limit = self.limit_bps.load(Ordering::Relaxed);
            if limit == 0 {
                return;
            }

            let rate = limit as f64;
            let cap = rate; // burst = 1 second worth of the current rate
            let bytes_f = bytes as f64;

            let wait = {
                let mut bucket = self.state.lock().expect("throttle bucket mutex poisoned");

                let now = Instant::now();
                let elapsed = now.saturating_duration_since(bucket.last).as_secs_f64();
                bucket.tokens = (bucket.tokens + elapsed * rate).min(cap);
                bucket.last = now;

                if bytes_f <= cap {
                    if bucket.tokens >= bytes_f {
                        bucket.tokens -= bytes_f;
                        return;
                    }
                    Duration::from_secs_f64(((bytes_f - bucket.tokens) / rate).max(0.0))
                } else if bucket.tokens >= cap {
                    // Oversized chunk, bucket is full: release the whole
                    // chunk as documented above.
                    bucket.tokens = 0.0;
                    return;
                } else {
                    Duration::from_secs_f64(((cap - bucket.tokens) / rate).max(0.0))
                }
            };

            let wait = wait.min(Duration::from_millis(100));
            let notified = self.notify.notified();
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = notified => {}
            }
        }
    }
}

/// Converts a MB/s figure (the QoS slider unit, RF-050/RF-082) into
/// bytes/second for [`Throttle::new`]/[`Throttle::set_limit`].
///
/// Uses the *decimal* megabyte (1 MB = 1_000_000 B) — the convention used
/// for network throughput (ISPs, `iperf`, cloud provider transfer graphs) —
/// rather than the *binary* mebibyte (1 MiB = 1_048_576 B) used elsewhere in
/// this crate for file/storage sizes. This keeps the on-screen "2.5 MB/s"
/// label numerically consistent with the throughput a destination console
/// (S3/Drive) would report for the same transfer.
pub fn mbps_to_bps(mbps: f64) -> u64 {
    (mbps * 1_000_000.0).round() as u64
}

/// Maximum size of a single internal read from the wrapped reader before
/// handing it to [`Throttle::acquire`]. Keeping this at or below the 1
/// second burst cap for any realistic configured rate (minimum slider value
/// is 0.5 MB/s = 500_000 B/s, RF-082) is what keeps pacing accurate: a
/// chunk larger than the burst would fall into `acquire`'s oversized-chunk
/// path and be released in one lump instead of being paced smoothly.
const READ_CHUNK: usize = 64 * 1024;

/// Wraps an [`AsyncRead`] body so every byte read through it is paced by a
/// shared [`Throttle`] (RF11/RF-050..052). Used to wrap upload bodies
/// (`reqwest::Body::wrap_stream` / `ByteStream`) for both simple and
/// multipart/resumable uploads.
///
/// # Design (acquire-before-deliver)
///
/// Rather than reserving tokens speculatively before knowing how many bytes
/// will actually be available (which can overcharge the bucket if the
/// inner reader yields fewer bytes than requested, or is not ready yet),
/// `ThrottledReader` reads a chunk from the inner reader **first** (capped
/// at [`READ_CHUNK`], independent of the caller-provided buffer size), then
/// pays for exactly the `n` bytes it actually got via
/// [`Throttle::acquire`], and only *after that completes* copies bytes into
/// the caller's buffer. Read bytes are staged in an internal queue so a
/// caller-provided buffer smaller than the chunk still gets fed correctly
/// across multiple `poll_read` calls, without ever re-acquiring tokens for
/// bytes that have already been paid for. This keeps metering exact
/// (no over- or under-charging) and is what keeps the effective throughput
/// within the ±10% target (RF-050) for arbitrary caller-side buffer sizes.
pub struct ThrottledReader<R> {
    inner: R,
    throttle: Arc<Throttle>,
    /// Bytes read from `inner` but not yet handed to the caller (already
    /// paid for, or about to be once `pending` resolves — see `scratch`).
    paid: VecDeque<u8>,
    /// Scratch space for a chunk read from `inner` while its `acquire` call
    /// is still pending (not yet in `paid`).
    scratch: Vec<u8>,
    scratch_len: usize,
    /// In-flight `throttle.acquire(scratch_len)` call for the bytes
    /// currently sitting in `scratch`.
    pending: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl<R> ThrottledReader<R> {
    /// Wraps `inner`, pacing reads through the given shared throttle.
    pub fn new(inner: R, throttle: Arc<Throttle>) -> Self {
        Self {
            inner,
            throttle,
            paid: VecDeque::new(),
            scratch: Vec::new(),
            scratch_len: 0,
            pending: None,
        }
    }

    /// Unwraps the reader, discarding the throttle (and any staged, unread
    /// bytes — callers should drain the reader with `AsyncReadExt::read*`
    /// before calling this if that data matters).
    pub fn into_inner(self) -> R {
        self.inner
    }
}

impl<R> AsyncRead for ThrottledReader<R>
where
    R: AsyncRead + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // `ThrottledReader<R>` is `Unpin` whenever `R` is: `paid`/`scratch`
        // are plain owned buffers and `pending` is already pinned via
        // `Pin<Box<_>>` (which is `Unpin` regardless of the boxed future).
        let this = self.get_mut();

        loop {
            if buf.remaining() == 0 {
                return Poll::Ready(Ok(()));
            }

            if !this.paid.is_empty() {
                let n = this.paid.len().min(buf.remaining());
                let (front, back) = this.paid.as_slices();
                let from_front = front.len().min(n);
                buf.put_slice(&front[..from_front]);
                if from_front < n {
                    buf.put_slice(&back[..n - from_front]);
                }
                this.paid.drain(..n);
                return Poll::Ready(Ok(()));
            }

            if let Some(fut) = this.pending.as_mut() {
                match fut.as_mut().poll(cx) {
                    Poll::Ready(()) => {
                        this.paid
                            .extend(this.scratch[..this.scratch_len].iter().copied());
                        this.scratch_len = 0;
                        this.pending = None;
                        // Loop back around: delivers from `paid` above.
                        continue;
                    }
                    Poll::Pending => return Poll::Pending,
                }
            }

            if this.scratch.len() < READ_CHUNK {
                this.scratch.resize(READ_CHUNK, 0);
            }
            let mut read_buf = ReadBuf::new(&mut this.scratch[..READ_CHUNK]);
            match Pin::new(&mut this.inner).poll_read(cx, &mut read_buf) {
                Poll::Ready(Ok(())) => {
                    let n = read_buf.filled().len();
                    if n == 0 {
                        // EOF.
                        return Poll::Ready(Ok(()));
                    }
                    this.scratch_len = n;
                    let throttle = this.throttle.clone();
                    this.pending = Some(Box::pin(async move { throttle.acquire(n).await }));
                    // Loop back around: polls the freshly-created future.
                }
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// 5 second sliding-window throughput meter (bytes/second), independent of
/// [`Throttle`]. Used by `core::worker` to report `rate_bps` in the
/// `upload-progress` event (`SPEC.md §6`: "Throughput medido ... janela
/// deslizante de 5 s").
pub struct ThroughputMeter {
    window: Duration,
    samples: Mutex<VecDeque<(Instant, u64)>>,
}

impl ThroughputMeter {
    const WINDOW: Duration = Duration::from_secs(5);

    /// Creates a meter with the standard 5 second window.
    pub fn new() -> Self {
        Self {
            window: Self::WINDOW,
            samples: Mutex::new(VecDeque::new()),
        }
    }

    /// Records that `bytes` were transferred just now.
    pub fn record(&self, bytes: u64) {
        let now = Instant::now();
        let mut samples = self
            .samples
            .lock()
            .expect("throughput meter mutex poisoned");
        samples.push_back((now, bytes));
        Self::prune(&mut samples, now, self.window);
    }

    /// Current throughput in bytes/second, averaged over the last window
    /// (samples older than the window are dropped first).
    pub fn bps(&self) -> f64 {
        let now = Instant::now();
        let mut samples = self
            .samples
            .lock()
            .expect("throughput meter mutex poisoned");
        Self::prune(&mut samples, now, self.window);
        let total: u64 = samples.iter().map(|(_, bytes)| *bytes).sum();
        total as f64 / self.window.as_secs_f64()
    }

    fn prune(samples: &mut VecDeque<(Instant, u64)>, now: Instant, window: Duration) {
        while let Some(&(t, _)) = samples.front() {
            if now.saturating_duration_since(t) > window {
                samples.pop_front();
            } else {
                break;
            }
        }
    }
}

impl Default for ThroughputMeter {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors raised by the `throttle` module's parsing/configuration APIs.
#[derive(Debug, Error)]
pub enum ThrottleError {
    /// A `night_mode.start`/`night_mode.end` string wasn't a valid `HH:MM`
    /// 24-hour time (`config::is_valid_hh_mm` already rejects malformed
    /// strings at `save_config` time — this is a second, defensive parse at
    /// the point the scheduler actually consumes the value).
    #[error("invalid time \"{0}\", expected HH:MM")]
    InvalidTime(String),
}

/// Strict, always-two-digit `HH:MM` shape check (mirrors
/// `config::is_valid_hh_mm`, which already gates this format at
/// `save_config` time — duplicated here, rather than shared, because
/// that helper is private to `config` and this module is intentionally
/// self-contained). `chrono::NaiveTime::parse_from_str("%H:%M")` alone
/// would accept single-digit fields like `"6:00"`; this rejects those.
fn is_strict_hh_mm(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 5
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2] == b':'
        && bytes[3].is_ascii_digit()
        && bytes[4].is_ascii_digit()
}

/// A `[start, end)` daily time-of-day window (`PRD.md` RF-053, `SPEC.md §6`
/// "Modo Noturno"), used to decide when [`NightModeScheduler`] suspends
/// uploads.
///
/// `start` is inclusive, `end` is exclusive. When `start > end` the window
/// wraps past midnight (e.g. the `23:00`-`06:00` default): a time-of-day
/// `t` is inside it when `t >= start || t < end`. When `start <= end` it's a
/// same-day window (e.g. `13:00`-`14:00`): `t` is inside it when
/// `start <= t < end`. `start == end` is [`NightWindow::is_empty`] — never
/// contains any time, matching a disabled/zero-length window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NightWindow {
    start: NaiveTime,
    end: NaiveTime,
}

impl NightWindow {
    /// Parses `start`/`end` as `HH:MM` 24-hour times (the format persisted
    /// in `config::NightModeConfig`).
    pub fn parse(start: &str, end: &str) -> Result<Self, ThrottleError> {
        let parse_one = |value: &str| {
            // `chrono`'s `%H`/`%M` parsing accepts single-digit fields
            // (e.g. "6:00"), which is looser than the strict, always
            // two-digit `HH:MM` this type promises (matching
            // `config::is_valid_hh_mm`'s own strictness) — so reject those
            // before handing off to `NaiveTime::parse_from_str`.
            if !is_strict_hh_mm(value) {
                return Err(ThrottleError::InvalidTime(value.to_string()));
            }
            NaiveTime::parse_from_str(value, "%H:%M")
                .map_err(|_| ThrottleError::InvalidTime(value.to_string()))
        };
        Ok(Self {
            start: parse_one(start)?,
            end: parse_one(end)?,
        })
    }

    /// Whether time-of-day `t` falls inside this window (start inclusive,
    /// end exclusive), handling both same-day and midnight-wrapping windows.
    pub fn contains(&self, t: NaiveTime) -> bool {
        if self.is_empty() {
            return false;
        }
        if self.start < self.end {
            t >= self.start && t < self.end
        } else {
            // Wraps past midnight: "inside" is everything from `start`
            // through end-of-day, plus everything from start-of-day up to
            // (not including) `end`.
            t >= self.start || t < self.end
        }
    }

    /// `true` when `start == end` — a zero-length window that never
    /// contains any time.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// Suspends a set of [`Throttle`]s (one per destination) to `0 bps` for the
/// duration of a configured [`NightWindow`], restoring each to its prior
/// ("daytime") limit once the window ends (`PRD.md` RF-053, `SPEC.md §6`).
///
/// One `Arc<NightModeScheduler>` is created per app session, wrapping every
/// destination's `Arc<Throttle>`. [`NightModeScheduler::spawn`] drives it
/// off a 60 s clock tick; [`NightModeScheduler::tick`] itself is pure and
/// synchronous, so it's directly unit-testable without a running task.
pub struct NightModeScheduler {
    /// One `(throttle, remembered daytime limit in bytes/second)` pair per
    /// destination, in the order passed to [`NightModeScheduler::new`].
    throttles: Vec<(Arc<Throttle>, AtomicU64)>,
    /// `None` while night mode is disabled. `Some(window)` — possibly
    /// [`NightWindow::is_empty`] — while enabled.
    window: RwLock<Option<NightWindow>>,
    /// Whether the scheduler currently believes "now" is inside the window
    /// (i.e. has zeroed the throttles). Tracked so [`NightModeScheduler::tick`]
    /// and [`NightModeScheduler::configure`] only act on actual transitions.
    in_night: AtomicBool,
}

impl NightModeScheduler {
    /// Wraps `throttles` (one per destination) for night-mode scheduling.
    /// Each throttle's *current* [`Throttle::limit_bps`] is captured as its
    /// initial remembered daytime limit.
    pub fn new(throttles: Vec<Arc<Throttle>>) -> Arc<Self> {
        let throttles = throttles
            .into_iter()
            .map(|throttle| {
                let daytime = AtomicU64::new(throttle.limit_bps());
                (throttle, daytime)
            })
            .collect();
        Arc::new(Self {
            throttles,
            window: RwLock::new(None),
            in_night: AtomicBool::new(false),
        })
    }

    /// Hot-reloads the night-mode window from `save_config`
    /// (`config::NightModeConfig`). Parses `start`/`end` even when
    /// `enabled` is `false` — a malformed pair should surface as an error
    /// either way rather than being silently ignored.
    ///
    /// Disabling while a night window is currently active restores every
    /// throttle to its remembered daytime limit immediately, rather than
    /// waiting for the next [`NightModeScheduler::tick`].
    pub fn configure(&self, enabled: bool, start: &str, end: &str) -> Result<(), ThrottleError> {
        let parsed = NightWindow::parse(start, end)?;
        let new_window = if enabled { Some(parsed) } else { None };

        {
            let mut guard = self
                .window
                .write()
                .expect("night mode window lock poisoned");
            *guard = new_window;
        }

        if !enabled && self.in_night.swap(false, Ordering::Relaxed) {
            tracing::debug!(
                "modo noturno: desativado no meio da janela, restaurando limites diurnos"
            );
            self.restore_daytime();
        }
        Ok(())
    }

    /// Updates destination `idx`'s remembered daytime limit (called by
    /// `set_qos` whenever the user changes a QoS slider), without touching
    /// the throttle's *live* limit. During the day the caller is expected
    /// to also apply the new limit live via [`Throttle::set_limit`]; during
    /// the night the live limit must stay `0` until
    /// [`NightModeScheduler::tick`] restores it, so this only updates the
    /// value that restore will use.
    pub fn set_daytime_limit(&self, idx: usize, bps: u64) {
        if let Some((_, daytime)) = self.throttles.get(idx) {
            daytime.store(bps, Ordering::Relaxed);
        }
    }

    /// Whether the scheduler currently considers "now" inside the night window.
    pub fn is_night(&self) -> bool {
        self.in_night.load(Ordering::Relaxed)
    }

    /// Pure transition logic, given the caller's notion of "now" as a
    /// time-of-day: enters the night window (remembering each throttle's
    /// current limit, then zeroing it) or leaves it (restoring the
    /// remembered limits), only when `now_local` actually crosses the
    /// configured [`NightWindow`] boundary. Idempotent — calling it
    /// repeatedly while on the same side of the boundary does nothing.
    pub fn tick(&self, now_local: NaiveTime) {
        let should_be_night = self
            .window
            .read()
            .expect("night mode window lock poisoned")
            .is_some_and(|window| window.contains(now_local));

        if should_be_night && !self.in_night.swap(true, Ordering::Relaxed) {
            tracing::debug!("modo noturno: entrando na janela, suspendendo destinos com limite");
            self.enter_night();
        } else if !should_be_night && self.in_night.swap(false, Ordering::Relaxed) {
            tracing::debug!("modo noturno: saindo da janela, restaurando limites diurnos");
            self.restore_daytime();
        }
    }

    /// Remembers each throttle's current limit as its daytime limit, then
    /// suspends it (`set_limit(0)`).
    fn enter_night(&self) {
        for (throttle, daytime) in &self.throttles {
            daytime.store(throttle.limit_bps(), Ordering::Relaxed);
            throttle.set_limit(0);
        }
    }

    /// Restores each throttle to its remembered daytime limit.
    fn restore_daytime(&self) {
        for (throttle, daytime) in &self.throttles {
            throttle.set_limit(daytime.load(Ordering::Relaxed));
        }
    }

    /// Spawns the 60 s scheduling loop, calling
    /// [`NightModeScheduler::tick`] with `clock()`'s current time-of-day
    /// until `cancel` fires. In production `clock` is
    /// `|| chrono::Local::now().time()`; tests inject a fake clock (e.g.
    /// backed by `Arc<Mutex<NaiveTime>>`) so `tokio::time::advance` under
    /// `#[tokio::test(start_paused = true)]` drives deterministic
    /// transitions.
    pub fn spawn(
        self: &Arc<Self>,
        cancel: CancellationToken,
        clock: impl Fn() -> NaiveTime + Send + Sync + 'static,
    ) -> JoinHandle<()> {
        let scheduler = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_secs(60)) => {}
                }
                scheduler.tick(clock());
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use tokio::io::AsyncReadExt;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn unlimited_acquire_never_sleeps() {
        let throttle = Throttle::new(0);
        let start = Instant::now();
        throttle.acquire(10 * 1024 * 1024).await;
        assert_eq!(Instant::now(), start, "unlimited acquire must not wait");
    }

    #[tokio::test(start_paused = true)]
    async fn limited_acquire_paces_to_the_expected_virtual_time() {
        let throttle = Throttle::new(1_000_000); // 1 MB/s (decimal)
        let start = Instant::now();

        let total = 10 * 1_000_000usize; // 10 MB
        let mut remaining = total;
        while remaining > 0 {
            let n = remaining.min(64 * 1024);
            throttle.acquire(n).await;
            remaining -= n;
        }

        let elapsed = Instant::now()
            .saturating_duration_since(start)
            .as_secs_f64();
        let expected = total as f64 / 1_000_000.0; // 10 s
        assert!(
            (elapsed - expected).abs() / expected <= 0.05,
            "elapsed {elapsed}s expected ~{expected}s (±5%)"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_readers_share_a_single_cap() {
        let throttle = Throttle::new(1_000_000); // 1 MB/s combined cap
        let start = Instant::now();

        let per_task = 5 * 1_000_000usize; // 5 MB each, 10 MB combined
        let make_task = |t: Arc<Throttle>| async move {
            let mut remaining = per_task;
            while remaining > 0 {
                let n = remaining.min(64 * 1024);
                t.acquire(n).await;
                remaining -= n;
            }
        };
        let h1 = tokio::spawn(make_task(throttle.clone()));
        let h2 = tokio::spawn(make_task(throttle.clone()));
        h1.await.unwrap();
        h2.await.unwrap();

        // 10 MB combined at a 1 MB/s shared cap must take ~10s: neither
        // reader escapes the shared bucket to add extra throughput.
        let elapsed = Instant::now()
            .saturating_duration_since(start)
            .as_secs_f64();
        let expected = 10.0;
        assert!(
            (elapsed - expected).abs() / expected <= 0.05,
            "elapsed {elapsed}s expected ~{expected}s (±5%)"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn set_limit_applies_to_waiters_within_100ms() {
        let throttle = Throttle::new(1); // ~nothing gets through
        let waiter = tokio::spawn({
            let throttle = throttle.clone();
            async move {
                // Larger than the (tiny) burst cap, so it can only be
                // released once the bucket is full for the *current* rate.
                throttle.acquire(1000).await;
            }
        });

        // Let the waiter task register itself in `acquire`'s first sleep
        // before we raise the limit — no virtual time has elapsed yet.
        tokio::task::yield_now().await;

        throttle.set_limit(1_000_000_000); // effectively unlimited now

        tokio::time::timeout(Duration::from_millis(100), waiter)
            .await
            .expect("set_limit should wake and satisfy the waiter within 100ms")
            .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn throttled_reader_paces_and_preserves_bytes() {
        let data: Vec<u8> = (0..1_048_576u32).map(|i| (i % 256) as u8).collect();
        let throttle = Throttle::new(524_288); // 512 KiB/s
        let mut reader = ThrottledReader::new(Cursor::new(data.clone()), throttle);

        let start = Instant::now();
        let mut out = Vec::new();
        reader.read_to_end(&mut out).await.expect("read_to_end");
        let elapsed = Instant::now()
            .saturating_duration_since(start)
            .as_secs_f64();

        assert_eq!(out, data, "ThrottledReader must not corrupt the stream");
        assert!(
            (elapsed - 2.0).abs() <= 0.1,
            "elapsed {elapsed}s expected ~2.0s"
        );
    }

    #[tokio::test]
    async fn throttled_reader_into_inner_returns_wrapped_reader() {
        let throttle = Throttle::new(0);
        let reader = ThrottledReader::new(Cursor::new(vec![1u8, 2, 3]), throttle);
        assert_eq!(reader.into_inner().into_inner(), vec![1u8, 2, 3]);
    }

    #[tokio::test(start_paused = true)]
    async fn throughput_meter_computes_sliding_window_rate() {
        let meter = ThroughputMeter::new();

        meter.record(1_000_000);
        tokio::time::advance(Duration::from_secs(1)).await;
        meter.record(1_000_000);
        // 2 MB total inside the 5s window.
        assert_eq!(meter.bps(), 400_000.0);

        tokio::time::advance(Duration::from_secs(5)).await; // t = 6s
                                                            // First sample (t=0) is now 6s old (> 5s window) and pruned; the
                                                            // second (t=1s) is exactly 5s old and still kept.
        assert_eq!(meter.bps(), 200_000.0);

        tokio::time::advance(Duration::from_millis(1)).await;
        // Second sample now just over the window too.
        assert_eq!(meter.bps(), 0.0);
    }

    #[test]
    fn mbps_to_bps_uses_decimal_megabytes() {
        assert_eq!(mbps_to_bps(1.0), 1_000_000);
        assert_eq!(mbps_to_bps(2.5), 2_500_000);
        assert_eq!(mbps_to_bps(0.5), 500_000);
        assert_eq!(mbps_to_bps(10.0), 10_000_000);
    }

    #[tokio::test]
    async fn limit_bps_reflects_set_limit() {
        let throttle = Throttle::new(0);
        assert_eq!(throttle.limit_bps(), 0);
        throttle.set_limit(2_500_000);
        assert_eq!(throttle.limit_bps(), 2_500_000);
    }

    fn hms(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).expect("valid HH:MM")
    }

    #[test]
    fn night_window_parse_rejects_malformed_time() {
        assert!(matches!(
            NightWindow::parse("23h00", "06:00"),
            Err(ThrottleError::InvalidTime(_))
        ));
        assert!(matches!(
            NightWindow::parse("23:00", "6:00"),
            Err(ThrottleError::InvalidTime(_))
        ));
    }

    #[test]
    fn night_window_contains_handles_wrap_and_same_day_windows() {
        // Wraps past midnight (the RF-053 default): start inclusive, end
        // exclusive, "inside" spans both sides of midnight.
        let wrap = NightWindow::parse("23:00", "06:00").expect("valid window");
        assert!(wrap.contains(hms(23, 0)), "start boundary is inclusive");
        assert!(wrap.contains(hms(23, 30)));
        assert!(wrap.contains(hms(0, 0)));
        assert!(wrap.contains(hms(5, 59)));
        assert!(!wrap.contains(hms(6, 0)), "end boundary is exclusive");
        assert!(!wrap.contains(hms(22, 59)));
        assert!(!wrap.contains(hms(12, 0)));

        // Same-day window: no wrap needed.
        let same_day = NightWindow::parse("13:00", "14:00").expect("valid window");
        assert!(same_day.contains(hms(13, 0)), "start boundary is inclusive");
        assert!(same_day.contains(hms(13, 30)));
        assert!(!same_day.contains(hms(14, 0)), "end boundary is exclusive");
        assert!(!same_day.contains(hms(12, 59)));
        assert!(!same_day.contains(hms(20, 0)));

        // Zero-length window: never contains anything.
        let empty = NightWindow::parse("07:00", "07:00").expect("valid window");
        assert!(empty.is_empty());
        assert!(!empty.contains(hms(7, 0)));
    }

    #[tokio::test(start_paused = true)]
    async fn night_mode_scheduler_enters_and_leaves_the_window() {
        let gdrive_daytime = mbps_to_bps(2.0);
        let s3_daytime = mbps_to_bps(5.0);
        let gdrive = Throttle::new(gdrive_daytime);
        let s3 = Throttle::new(s3_daytime);
        let scheduler = NightModeScheduler::new(vec![gdrive.clone(), s3.clone()]);
        scheduler
            .configure(true, "23:00", "06:00")
            .expect("valid window");

        // Just before the window: unchanged.
        scheduler.tick(hms(22, 59));
        assert_eq!(gdrive.limit_bps(), gdrive_daytime);
        assert_eq!(s3.limit_bps(), s3_daytime);

        // Entering the window: both throttles suspended.
        scheduler.tick(hms(23, 0));
        assert_eq!(gdrive.limit_bps(), 0);
        assert_eq!(s3.limit_bps(), 0);

        // Idempotent: ticking again while still inside the window is a no-op.
        scheduler.tick(hms(23, 30));
        assert_eq!(gdrive.limit_bps(), 0);
        assert_eq!(s3.limit_bps(), 0);

        // A QoS change during the night updates the remembered daytime
        // limit but must not un-suspend the throttle before the window ends.
        let gdrive_new_daytime = mbps_to_bps(3.0);
        scheduler.set_daytime_limit(0, gdrive_new_daytime);
        assert_eq!(
            gdrive.limit_bps(),
            0,
            "set_daytime_limit must not restore mid-window"
        );

        // Leaving the window: restores each throttle's (possibly updated)
        // daytime limit.
        scheduler.tick(hms(6, 0));
        assert_eq!(gdrive.limit_bps(), gdrive_new_daytime);
        assert_eq!(s3.limit_bps(), s3_daytime);

        // Idempotent: ticking again while still outside the window is a no-op.
        scheduler.tick(hms(12, 0));
        assert_eq!(gdrive.limit_bps(), gdrive_new_daytime);
        assert_eq!(s3.limit_bps(), s3_daytime);
    }

    #[tokio::test(start_paused = true)]
    async fn night_mode_scheduler_configure_disable_mid_window_restores_immediately() {
        let daytime = mbps_to_bps(2.0);
        let throttle = Throttle::new(daytime);
        let scheduler = NightModeScheduler::new(vec![throttle.clone()]);
        scheduler
            .configure(true, "23:00", "06:00")
            .expect("valid window");

        scheduler.tick(hms(23, 0));
        assert_eq!(throttle.limit_bps(), 0, "should be suspended for the night");

        scheduler
            .configure(false, "23:00", "06:00")
            .expect("valid window");
        assert_eq!(
            throttle.limit_bps(),
            daytime,
            "disabling mid-window restores immediately, without waiting for a tick"
        );

        // Once disabled, a night-hours tick must not re-suspend.
        scheduler.tick(hms(23, 30));
        assert_eq!(throttle.limit_bps(), daytime);
    }

    #[tokio::test(start_paused = true)]
    async fn night_mode_scheduler_spawn_advances_and_transitions_once_per_interval() {
        let daytime = mbps_to_bps(2.0);
        let throttle = Throttle::new(daytime);
        let scheduler = NightModeScheduler::new(vec![throttle.clone()]);
        scheduler
            .configure(true, "23:00", "06:00")
            .expect("valid window");

        let clock_time = Arc::new(Mutex::new(hms(22, 59)));
        let clock_for_task = clock_time.clone();
        let cancel = CancellationToken::new();
        let handle = scheduler.spawn(cancel.clone(), move || {
            *clock_for_task.lock().expect("fake clock mutex poisoned")
        });

        // Nothing happens until the first 60s interval elapses.
        tokio::task::yield_now().await;
        assert_eq!(throttle.limit_bps(), daytime);

        // Move the fake clock into the window, then advance virtual time
        // past one interval in small steps: like `health.rs`'s equivalent
        // loop test, a single big `advance` isn't reliably enough runtime
        // turns for the timer driver to notice and wake the loop task.
        *clock_time.lock().expect("fake clock mutex poisoned") = hms(23, 0);
        let step = Duration::from_millis(500);
        let budget = Duration::from_secs(60) * 2; // generous: 2 intervals' worth
        let mut waited = Duration::ZERO;
        while throttle.limit_bps() != 0 && waited < budget {
            tokio::time::advance(step).await;
            tokio::task::yield_now().await;
            waited += step;
        }
        assert_eq!(
            throttle.limit_bps(),
            0,
            "interval tick should have entered the window within {budget:?} of virtual time"
        );

        // A further interval at the same (still in-window) clock time must
        // not do anything beyond the already-suspended state.
        tokio::time::advance(Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        assert_eq!(throttle.limit_bps(), 0);

        cancel.cancel();
        handle
            .await
            .expect("scheduler task should exit cleanly on cancellation");
    }
}
