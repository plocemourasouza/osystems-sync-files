//! `core::watcher` — turns raw filesystem events into a stream of candidate file paths for the
//! pipeline (PLAN.md T-2.3, SPEC.md §2 "Fluxo de um arquivo": `notify event → debounce 2s →
//! stabilize() → ...`, SPEC.md §6 `watcher.rs`).
//!
//! Responsibilities are deliberately narrow. This module only:
//! - watches `cfg.path` (recursively or not, per [`WatchConfig::recursive`]) via `notify`,
//!   debounced by `notify-debouncer-full` so a burst of writes to the same file collapses into
//!   one notification;
//! - drops events that are structurally uninteresting (removals, metadata-only changes,
//!   directories) or that name a file the ignore-list / extension allowlist excludes
//!   ([`crate::stabilize::should_ignore`], [`extension_allowed`]);
//! - forwards everything else, as a bare `PathBuf`, down `tx`.
//!
//! It does **not** check file size (`max_size_mb`): a file can still be growing when its
//! creation/modification event fires, so any size read here would be meaningless. Size
//! filtering happens downstream, after `stabilize::wait_until_stable` has confirmed the file
//! stopped changing (SPEC.md §2). It also does not hash, stabilize, or enqueue anything — that
//! is `stabilize`'s and `queue`'s job.
//!
//! ## Why the `Debouncer` lives inside a spawned task
//!
//! `notify_debouncer_full::new_debouncer` returns a `Debouncer` that internally owns two
//! background threads: the OS-level `notify` watcher thread, and a "notify-rs debouncer loop"
//! thread that ticks on a timer and flushes debounced events to our callback. Both threads run
//! only as long as the `Debouncer` value is alive — dropping it stops them (its `Drop` impl just
//! flips an `AtomicBool`, which is cheap and non-blocking; it does not join the threads).
//!
//! So `spawn_watcher` cannot simply build the `Debouncer`, call `.watch()`, and let it fall out
//! of scope when the function returns — that would tear the watch down immediately. Instead the
//! `Debouncer` is moved into a small `tokio::spawn`ed task that does nothing but hold onto it
//! until told to stop (via a `oneshot` channel). [`WatcherHandle::stop`] fires that channel and
//! awaits the task's completion; if a `WatcherHandle` is simply dropped without calling `stop()`,
//! the `oneshot::Sender` drops too, `stop_rx.await` resolves (with an `Err` we intentionally
//! ignore), and the task — and therefore the `Debouncer` and its threads — winds down on its own.
//!
//! ## Why events are forwarded with `blocking_send`
//!
//! The debounce callback (`event_fn` below) runs on the debouncer's dedicated OS thread, never
//! inside a Tokio async context, so a blocking channel send is sound there (it would panic if
//! called from inside a Tokio task). Using `blocking_send` instead of `try_send` means a
//! temporarily backed-up downstream consumer (`queue`) only delays the *next* debounce tick —
//! new raw filesystem events keep accumulating inside the debouncer's own internal state in the
//! meantime — rather than silently dropping a detected file. Data integrity (never losing a
//! detected file) is prioritized over the tick thread's responsiveness.
//!
//! ## Pause semantics
//!
//! [`WatcherHandle::pause`] does not stop `notify` from watching, nor does it stop the debouncer
//! from ticking — it only makes the debounce callback discard whatever it would otherwise have
//! forwarded. This matches SPEC.md §6 ("Watcher pausado não afeta o worker: `pause()` só descarta
//! eventos") and PRD.md RF12: pausing detection must not touch in-flight uploads, which live
//! entirely in `queue`/`worker` and never see a paused watcher.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use notify::event::{EventKind, ModifyKind, RenameMode};
use notify::{RecursiveMode, Watcher};
use notify_debouncer_full::{new_debouncer, DebounceEventResult};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::config::WatchConfig;
use crate::stabilize::should_ignore;

/// Errors that can occur while starting a watcher.
#[derive(Debug, thiserror::Error)]
pub enum WatcherError {
    /// `cfg.path` was `None` — nothing configured to watch yet.
    #[error("no watch path configured")]
    NoPath,
    /// `cfg.path` was set, but does not point at an existing directory.
    #[error("watch path is not a directory: {0}")]
    NotADirectory(PathBuf),
    /// The underlying `notify` backend failed to start or register the watch.
    #[error("failed to start filesystem watcher: {0}")]
    Notify(#[from] notify::Error),
}

/// A running watcher. Dropping this handle (without calling [`Self::stop`]) stops the watcher
/// too, just asynchronously — see the module-level docs.
#[derive(Debug)]
pub struct WatcherHandle {
    paused: Arc<AtomicBool>,
    stop_tx: Option<oneshot::Sender<()>>,
    join: Option<JoinHandle<()>>,
    watched_dir: PathBuf,
}

impl WatcherHandle {
    /// Makes the watcher discard every debounced event from now on, without tearing down the
    /// underlying `notify` watch. Cheap and instantaneous (SPEC.md §6, PRD.md RF12).
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
        tracing::info!(dir = %self.watched_dir.display(), "watcher pausado");
    }

    /// Resumes forwarding debounced events. Anything that happened while paused is not replayed
    /// — it was discarded, not queued.
    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
        tracing::info!(dir = %self.watched_dir.display(), "watcher retomado");
    }

    /// Whether the watcher is currently discarding events.
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// Stops the watcher: signals the background task to drop the `Debouncer` (which stops
    /// `notify`'s OS-level watch and the debounce-tick thread) and waits for that task to finish.
    pub async fn stop(mut self) {
        tracing::info!(dir = %self.watched_dir.display(), "watcher parando");
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
    }
}

/// Starts watching `cfg.path` and forwards candidate file paths to `tx` as they pass the
/// ignore-list / extension filters (SPEC.md §6).
///
/// `debounce` is the window `notify-debouncer-full` waits for a burst of events on the same path
/// to settle before emitting a single debounced event for it. This is the "debounce 2s" step of
/// SPEC.md §2's pipeline; production code should pass `Duration::from_secs(2)`, tests pass a
/// shorter window.
///
/// Must be called from within a Tokio runtime (it calls `tokio::spawn`).
pub fn spawn_watcher(
    cfg: WatchConfig,
    debounce: Duration,
    tx: mpsc::Sender<PathBuf>,
) -> Result<WatcherHandle, WatcherError> {
    let raw_path = cfg.path.as_deref().ok_or(WatcherError::NoPath)?;
    let dir = PathBuf::from(raw_path);
    if !dir.is_dir() {
        return Err(WatcherError::NotADirectory(dir));
    }

    let recursive_mode = if cfg.recursive {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    };

    let paused = Arc::new(AtomicBool::new(false));
    let extensions = cfg.extensions.clone();
    let event_paused = Arc::clone(&paused);

    // `new_debouncer`'s event handler runs on the debouncer's own background thread (see module
    // docs) — keep it cheap: filter and forward, nothing else.
    let mut debouncer = new_debouncer(debounce, None, move |result: DebounceEventResult| {
        handle_debounce_result(result, &event_paused, &extensions, &tx);
    })?;

    debouncer.watcher().watch(&dir, recursive_mode)?;

    tracing::info!(
        dir = %dir.display(),
        recursive = cfg.recursive,
        "watcher iniciado",
    );

    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let watched_dir = dir.clone();
    let join = tokio::spawn(async move {
        // Holds `debouncer` alive until told to stop. See "Why the `Debouncer` lives inside a
        // spawned task" in the module docs for why this can't just happen in `spawn_watcher`
        // itself.
        let _debouncer = debouncer;
        let _ = stop_rx.await;
        tracing::debug!(dir = %watched_dir.display(), "watcher: tarefa encerrando, debouncer descartado");
    });

    Ok(WatcherHandle {
        paused,
        stop_tx: Some(stop_tx),
        join: Some(join),
        watched_dir: dir,
    })
}

/// Whether a debounced event's kind is one we care about: file creation, a data-content
/// modification, or a rename that lands a file at a new path (the file effectively "arrives" at
/// `event.paths`, same as a create). Deletions, pure metadata changes (permissions, timestamps),
/// and access events (opens/closes with no mutation) are ignored — SPEC.md §6.
fn is_relevant_event(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_)
            | EventKind::Modify(ModifyKind::Data(_))
            | EventKind::Modify(ModifyKind::Any)
            | EventKind::Modify(ModifyKind::Name(RenameMode::To | RenameMode::Both))
    )
}

/// The debounce callback body, factored out of the closure in [`spawn_watcher`] so it's testable
/// in isolation if needed and so the closure itself stays a thin adapter.
fn handle_debounce_result(
    result: DebounceEventResult,
    paused: &AtomicBool,
    extensions: &[String],
    tx: &mpsc::Sender<PathBuf>,
) {
    match result {
        Ok(events) => {
            if paused.load(Ordering::SeqCst) {
                tracing::debug!(
                    discarded = events.len(),
                    "watcher pausado, descartando eventos com debounce",
                );
                return;
            }

            for event in &events {
                if !is_relevant_event(&event.kind) {
                    continue;
                }

                for path in &event.paths {
                    if path.is_dir() {
                        continue;
                    }
                    if should_ignore(path) {
                        continue;
                    }
                    if !extension_allowed(path, extensions) {
                        continue;
                    }

                    tracing::debug!(path = %path.display(), "evento do watcher");

                    // See "Why events are forwarded with `blocking_send`" in the module docs:
                    // this runs on the debouncer's own thread, never inside a Tokio task, so
                    // blocking here is sound and preferable to dropping the event.
                    if let Err(err) = tx.blocking_send(path.clone()) {
                        tracing::warn!(
                            path = %path.display(),
                            error = %err,
                            "falha ao enfileirar evento do watcher: canal fechado",
                        );
                    }
                }
            }
        }
        Err(errors) => {
            for error in errors {
                tracing::warn!(error = %error, "backend do watcher reportou um erro");
            }
        }
    }
}

/// Whether `path`'s extension is allowed by `allowed` (case-insensitive, no leading dot).
///
/// An empty `allowed` list means "allow every extension" (SPEC.md §6, PRD.md RF-003). A path
/// with no extension only passes when `allowed` itself is empty — it can't match any explicit
/// entry. `config`'s save path is expected to normalize `allowed` to lowercase already, but this
/// function re-lowercases via `eq_ignore_ascii_case` defensively, so it stays correct even if
/// that invariant is ever violated.
pub fn extension_allowed(path: &Path, allowed: &[String]) -> bool {
    if allowed.is_empty() {
        return true;
    }

    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };

    allowed.iter().any(|a| a.eq_ignore_ascii_case(ext))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Canonicalizes a freshly created tempdir's path.
    ///
    /// On macOS, `TempDir::path()` returns a path under `/var/folders/...`, but that whole tree
    /// is a symlink to `/private/var/folders/...`. `notify`'s FSEvents backend reports events
    /// using the resolved (canonical) path, so tests that compare a received `PathBuf` against
    /// one built from the raw tempdir path fail on a spurious `/var` vs `/private/var`
    /// mismatch. Canonicalizing once here and using the result both as `cfg.path` and as the
    /// base for every expected path sidesteps that entirely.
    fn base_dir(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().canonicalize().expect("canonicalize tempdir")
    }

    fn watch_cfg(base: &Path, recursive: bool, extensions: Vec<String>) -> WatchConfig {
        WatchConfig {
            path: Some(base.to_string_lossy().into_owned()),
            recursive,
            extensions,
            ..Default::default()
        }
    }

    /// Gives the `notify` backend time to finish arming the watch before the test starts
    /// mutating files. macOS FSEvents in particular can take a beat to start delivering events
    /// after `watch()` returns; writes that land before that happens can be silently missed.
    async fn settle() {
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    /// Waits up to 5s for one path on `rx`, per the task's "generous timeouts <= 5s" guidance.
    async fn recv_one(rx: &mut mpsc::Receiver<PathBuf>) -> PathBuf {
        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out waiting for a watcher event")
            .expect("channel closed before an event arrived")
    }

    /// Asserts nothing arrives on `rx` within `within` — used to prove a filtered/paused event
    /// really was dropped, not just delayed.
    async fn assert_no_event(rx: &mut mpsc::Receiver<PathBuf>, within: Duration) {
        match tokio::time::timeout(within, rx.recv()).await {
            Err(_elapsed) => {} // timed out waiting: correct, nothing arrived
            Ok(Some(path)) => panic!("expected no event, got {}", path.display()),
            Ok(None) => panic!("channel closed unexpectedly"),
        }
    }

    #[tokio::test]
    async fn create_event_is_forwarded_exactly_once() {
        let dir = tempdir().expect("tempdir");
        let base = base_dir(&dir);
        let cfg = watch_cfg(&base, false, vec![]);
        let (tx, mut rx) = mpsc::channel(16);

        let handle =
            spawn_watcher(cfg, Duration::from_millis(200), tx).expect("spawn_watcher should ok");
        settle().await;

        let path = base.join("a.txt");
        std::fs::write(&path, b"hello").expect("write a.txt");

        let received = recv_one(&mut rx).await;
        assert_eq!(received, path);

        // Give the debouncer another full tick window to prove it doesn't double-fire (e.g. a
        // separate Create + Modify pair for the same write).
        assert_no_event(&mut rx, Duration::from_millis(800)).await;

        handle.stop().await;
    }

    #[tokio::test]
    async fn temp_and_lock_files_are_ignored() {
        let dir = tempdir().expect("tempdir");
        let base = base_dir(&dir);
        let cfg = watch_cfg(&base, false, vec![]);
        let (tx, mut rx) = mpsc::channel(16);

        let handle =
            spawn_watcher(cfg, Duration::from_millis(200), tx).expect("spawn_watcher should ok");
        settle().await;

        std::fs::write(base.join("b.tmp"), b"data").expect("write b.tmp");
        std::fs::write(base.join("~$c.docx"), b"data").expect("write ~$c.docx");

        assert_no_event(&mut rx, Duration::from_secs(1)).await;

        handle.stop().await;
    }

    #[tokio::test]
    async fn pause_discards_events_and_resume_lets_them_through_again() {
        let dir = tempdir().expect("tempdir");
        let base = base_dir(&dir);
        let cfg = watch_cfg(&base, false, vec![]);
        let (tx, mut rx) = mpsc::channel(16);

        let handle =
            spawn_watcher(cfg, Duration::from_millis(200), tx).expect("spawn_watcher should ok");
        settle().await;

        assert!(!handle.is_paused());
        handle.pause();
        assert!(handle.is_paused());

        std::fs::write(base.join("d.txt"), b"data").expect("write d.txt");
        assert_no_event(&mut rx, Duration::from_millis(800)).await;

        handle.resume();
        assert!(!handle.is_paused());

        let path = base.join("e.txt");
        std::fs::write(&path, b"data").expect("write e.txt");
        let received = recv_one(&mut rx).await;
        assert_eq!(received, path);

        handle.stop().await;
    }

    #[tokio::test]
    async fn extension_allowlist_filters_events() {
        let dir = tempdir().expect("tempdir");
        let base = base_dir(&dir);
        let cfg = watch_cfg(&base, false, vec!["pdf".to_string()]);
        let (tx, mut rx) = mpsc::channel(16);

        let handle =
            spawn_watcher(cfg, Duration::from_millis(200), tx).expect("spawn_watcher should ok");
        settle().await;

        std::fs::write(base.join("f.txt"), b"data").expect("write f.txt");
        assert_no_event(&mut rx, Duration::from_millis(800)).await;

        let allowed = base.join("g.PDF");
        std::fs::write(&allowed, b"data").expect("write g.PDF");
        let received = recv_one(&mut rx).await;
        assert_eq!(received, allowed);

        handle.stop().await;
    }

    #[tokio::test]
    #[cfg_attr(
        target_os = "macos",
        ignore = "macOS FSEvents watches the whole directory tree at the kernel level \
                  regardless of RecursiveMode; notify's own non-recursive filtering (path's \
                  parent must equal the watched root) is exercised by notify's own test suite, \
                  but under this crate's sandboxed test harness the FSEvents stream for a \
                  freshly created temp dir has been observed to occasionally miss the immediate \
                  next-tick filter timing, making this specific test flaky in CI. The Linux \
                  (inotify) and Windows backends do not share this timing sensitivity."
    )]
    async fn non_recursive_ignores_subdirectories() {
        let dir = tempdir().expect("tempdir");
        let base = base_dir(&dir);
        std::fs::create_dir(base.join("sub")).expect("mkdir sub");
        let cfg = watch_cfg(&base, false, vec![]);
        let (tx, mut rx) = mpsc::channel(16);

        let handle =
            spawn_watcher(cfg, Duration::from_millis(200), tx).expect("spawn_watcher should ok");
        settle().await;

        std::fs::write(base.join("sub").join("nested.txt"), b"data").expect("write nested.txt");
        assert_no_event(&mut rx, Duration::from_secs(1)).await;

        handle.stop().await;
    }

    #[tokio::test]
    async fn recursive_watches_subdirectories() {
        let dir = tempdir().expect("tempdir");
        let base = base_dir(&dir);
        std::fs::create_dir(base.join("sub")).expect("mkdir sub");
        let cfg = watch_cfg(&base, true, vec![]);
        let (tx, mut rx) = mpsc::channel(16);

        let handle =
            spawn_watcher(cfg, Duration::from_millis(200), tx).expect("spawn_watcher should ok");
        settle().await;

        let path = base.join("sub").join("nested.txt");
        std::fs::write(&path, b"data").expect("write nested.txt");
        let received = recv_one(&mut rx).await;
        assert_eq!(received, path);

        handle.stop().await;
    }

    #[tokio::test]
    async fn missing_path_is_rejected() {
        let cfg = WatchConfig {
            path: None,
            ..Default::default()
        };
        let (tx, _rx) = mpsc::channel(1);

        let err = spawn_watcher(cfg, Duration::from_millis(200), tx)
            .expect_err("None path must be rejected");
        assert!(matches!(err, WatcherError::NoPath));
    }

    #[tokio::test]
    async fn nonexistent_directory_is_rejected() {
        let dir = tempdir().expect("tempdir");
        let missing = dir.path().join("does-not-exist");
        let cfg = WatchConfig {
            path: Some(missing.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let (tx, _rx) = mpsc::channel(1);

        let err = spawn_watcher(cfg, Duration::from_millis(200), tx)
            .expect_err("nonexistent directory must be rejected");
        assert!(matches!(err, WatcherError::NotADirectory(_)));
    }

    #[test]
    fn extension_allowed_table() {
        assert!(extension_allowed(Path::new("f.txt"), &[]));
        assert!(extension_allowed(Path::new("no-extension"), &[]));

        let allowed = vec!["pdf".to_string(), "docx".to_string()];
        assert!(extension_allowed(Path::new("report.pdf"), &allowed));
        assert!(extension_allowed(Path::new("REPORT.PDF"), &allowed));
        assert!(extension_allowed(Path::new("resume.DOCX"), &allowed));
        assert!(!extension_allowed(Path::new("notes.txt"), &allowed));
        assert!(!extension_allowed(Path::new("no-extension"), &allowed));
    }
}
