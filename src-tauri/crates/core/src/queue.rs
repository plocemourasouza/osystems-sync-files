//! `core::queue` — file intake pipeline (PLAN.md T-2.4, SPEC.md §2 "Fluxo de
//! um arquivo", §6 "queue.rs / worker.rs"; PRD.md RF-030, RF-039, RNF-007).
//!
//! This module is the single sanctioned bridge between the async world
//! (watcher, rescan, future `worker.rs`) and the synchronous `rusqlite`
//! [`Repo`] (CLAUDE.md: "Não bloquear o runtime Tokio com I/O síncrono
//! pesado"). Everything here goes through [`with_repo`], which runs on the
//! blocking thread pool.
//!
//! Per SPEC.md §6, the queue *is* the `jobs` table — there is no in-memory
//! job list. The only in-memory piece is [`Wakers`]: a pair of
//! `tokio::sync::Notify` (one per [`Destination`]) that tell idle workers
//! "something new is pending" without polling the database. Pausing the
//! watcher never touches `Wakers` or the worker pool (SPEC.md §6): that is
//! enforced simply by `intake` being the only thing that calls
//! `notify_all`, and callers (the watcher) are the ones responsible for not
//! invoking `intake` while paused.
//!
//! [`intake`] implements the `stabilize → hash → upsert → wake` half of the
//! SPEC.md §2 pipeline (stabilization itself is `core::stabilize`'s job —
//! callers pass in the already-stabilized `size`/`mtime`). [`passes_filters`]
//! is the shared extension/size/ignore gate used by both the watcher (per
//! filesystem event) and `rescan()` (T-2.5, per file on disk).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use tokio::sync::Notify;

use crate::config::WatchConfig;
use crate::hash::{self, HashError};
use crate::stabilize;
use crate::state::{Destination, Repo, StateError, UpsertOutcome};

/// Errors surfaced by `core::queue`.
#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("hashing failed: {0}")]
    Hash(#[from] HashError),

    #[error("state error: {0}")]
    State(#[from] StateError),

    #[error("blocking task panicked or was cancelled: {0}")]
    Join(#[from] tokio::task::JoinError),

    /// VULN-003: the canonicalized path escaped `root` (e.g. a symlink
    /// resolving outside the watched folder, or a stale/forged path handed
    /// to `intake` directly) — refused rather than hashed/enqueued, since
    /// `root` is the only boundary the rest of the app trusts.
    #[error("path escapes the watch root: {0}")]
    OutsideWatchRoot(PathBuf),
}

/// Runs `f` against the shared [`Repo`] on the blocking thread pool.
///
/// This is the **only** sanctioned way to touch SQLite from async code —
/// `Repo`'s methods are synchronous `rusqlite` on purpose (CLAUDE.md), so
/// every caller (watcher, rescan, worker) must route through here instead
/// of calling `Repo` methods directly from an async context.
///
/// The lock is acquired *inside* the blocking closure (not before spawning
/// it), so an async task waiting to run `with_repo` never blocks a Tokio
/// worker thread on `Mutex::lock`. A poisoned mutex (a previous holder
/// panicked while holding the lock) is recovered rather than propagated —
/// losing one caller's SQLite handle should not take down every future
/// caller too.
pub async fn with_repo<T, F>(repo: Arc<Mutex<Repo>>, f: F) -> Result<T, QueueError>
where
    T: Send + 'static,
    F: FnOnce(&Repo) -> Result<T, StateError> + Send + 'static,
{
    let joined = tokio::task::spawn_blocking(move || {
        let guard = match repo.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&guard)
    })
    .await;

    match joined {
        Ok(inner) => inner.map_err(QueueError::from),
        Err(join_err) => Err(QueueError::from(join_err)),
    }
}

/// Thin newtype around the shared, mutex-guarded [`Repo`] handle other
/// modules pass around. Equivalent to using `Arc<Mutex<Repo>>` directly —
/// [`with_repo`] accepts either — this just gives call sites a named type
/// and a method form.
#[derive(Clone)]
pub struct SharedRepo(pub Arc<Mutex<Repo>>);

impl SharedRepo {
    pub fn new(repo: Repo) -> Self {
        Self(Arc::new(Mutex::new(repo)))
    }

    pub async fn with_repo<T, F>(&self, f: F) -> Result<T, QueueError>
    where
        T: Send + 'static,
        F: FnOnce(&Repo) -> Result<T, StateError> + Send + 'static,
    {
        with_repo(self.0.clone(), f).await
    }
}

/// Per-destination `Notify` handles used to wake idle workers when a job
/// becomes available (SPEC.md §6: "em memória apenas `Notify` para acordar
/// workers"). Cheap to clone the individual waiters out via [`Wakers::waiter`]
/// and hand them to a `worker_loop(dest)` task.
pub struct Wakers {
    s3: Arc<Notify>,
    gdrive: Arc<Notify>,
}

impl Default for Wakers {
    fn default() -> Self {
        Self::new()
    }
}

impl Wakers {
    pub fn new() -> Self {
        Self {
            s3: Arc::new(Notify::new()),
            gdrive: Arc::new(Notify::new()),
        }
    }

    /// Wakes every worker currently waiting on `dest`'s queue.
    pub fn notify(&self, dest: Destination) {
        self.waiter(dest).notify_waiters();
    }

    /// Wakes every worker on every destination. Used after [`intake`] enqueues
    /// a new pair of jobs, since a fresh file always produces one job per
    /// destination (RF-030).
    pub fn notify_all(&self) {
        self.s3.notify_waiters();
        self.gdrive.notify_waiters();
    }

    /// Returns the shared `Notify` for `dest`, to be awaited (`.notified()`)
    /// by that destination's worker loop.
    pub fn waiter(&self, dest: Destination) -> Arc<Notify> {
        match dest {
            Destination::S3 => self.s3.clone(),
            Destination::GDrive => self.gdrive.clone(),
        }
    }
}

/// Result of a successful [`intake`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntakeOutcome {
    /// `files.id` of the row `path` maps to (existing or newly created).
    pub file_id: String,
    /// What [`Repo::upsert_file_and_enqueue`] did with it.
    pub outcome: UpsertOutcome,
    /// Freshly computed SHA-256 hex digest.
    pub sha256: String,
    /// Size in bytes, as passed in by the caller (already-stabilized).
    pub size: u64,
}

/// Runs one file through the `hash → upsert → wake` half of the SPEC.md §2
/// pipeline: `stabilize::wait_until_stable` has already run by the time this
/// is called — `size`/`mtime` are its output.
///
/// - `Created` / `Rehashed` (a new file, or an existing path whose content
///   changed): logs an `info` event into the `events` table via
///   `insert_event` and wakes every worker (`wakers.notify_all()`) — SPEC.md
///   §2 always creates exactly 2 jobs (`s3`, `gdrive`) per file (RF-030).
/// - `Unchanged` (duplicate detection of an already-known, unmodified file):
///   only a `tracing::debug!` — no event row, no wake, since no job was
///   touched.
pub async fn intake(
    repo: Arc<Mutex<Repo>>,
    wakers: &Wakers,
    root: &Path,
    path: PathBuf,
    size: u64,
    mtime: DateTime<Utc>,
) -> Result<IntakeOutcome, QueueError> {
    // Canonicalize once, at the single choke point into `files.path`: the
    // watcher (notify) reports canonical paths (`/private/tmp/...` on macOS)
    // while `rescan` walks the user-supplied `watch.path` (`/tmp/...`). Without
    // this the same file would be inserted twice (RF-039 regression found in
    // the Fase 2 acceptance run). Falls back to the given path if the file
    // vanished between detection and intake.
    let path = crate::paths::canonicalize_clean(&path)
        .await
        .unwrap_or(path);

    // VULN-003: containment check against `root` (canonicalized once by the
    // caller -- `rescan()`/`run_intake_loop`). A symlink inside the watched
    // folder that resolves outside it (or any other forged/stale path) must
    // never be hashed, uploaded, or recorded in `files` -- that would let an
    // attacker who controls a symlink inside the watch folder exfiltrate
    // arbitrary files reachable by this process.
    if !path.starts_with(root) {
        return Err(QueueError::OutsideWatchRoot(path));
    }

    let digest = hash::sha256_file(path.clone()).await?;
    let sha256 = digest.to_string();

    let path_str = path.to_string_lossy().into_owned();
    let mtime_str = mtime.to_rfc3339();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path_str.clone());

    let outcome = {
        let upsert_path = path_str.clone();
        let upsert_sha = sha256.clone();
        with_repo(repo.clone(), move |repo| {
            repo.upsert_file_and_enqueue(&upsert_path, &upsert_sha, size as i64, &mtime_str)
        })
        .await?
    };

    // `Repo::upsert_file_and_enqueue` reports only what happened, not the
    // `files.id` it happened to — `files.path` is UNIQUE (schema.sql), so
    // `Repo::file_id_for_path` (T-2.5) resolves it directly.
    let file_id = find_file_id(repo.clone(), path_str.clone()).await?;

    match outcome {
        UpsertOutcome::Created | UpsertOutcome::Rehashed => {
            let message = format!("{name} enfileirado");
            with_repo(repo.clone(), move |repo| {
                repo.insert_event("info", None, &message)
            })
            .await?;

            tracing::info!(
                file_id = %file_id,
                path = %path_str,
                size,
                outcome = ?outcome,
                "queue: entrada de arquivo enfileirou jobs"
            );
            wakers.notify_all();
        }
        UpsertOutcome::Unchanged => {
            tracing::debug!(
                file_id = %file_id,
                path = %path_str,
                size,
                "queue: entrada de arquivo sem alteração, nenhum job enfileirado"
            );
        }
    }

    Ok(IntakeOutcome {
        file_id,
        outcome,
        sha256,
        size,
    })
}

/// Best-effort `files.id` lookup by `path`, via `Repo::file_id_for_path`.
/// Returns an empty string if the row can't be found — which should not
/// happen in practice since this always runs immediately after an
/// `upsert_file_and_enqueue` that just created or touched that exact path.
async fn find_file_id(repo: Arc<Mutex<Repo>>, path: String) -> Result<String, QueueError> {
    let found = with_repo(repo, move |repo| repo.file_id_for_path(&path)).await?;
    Ok(found.unwrap_or_default())
}

/// Shared filter gate for whether a file should be handed to [`intake`] at
/// all — used by both the watcher (per filesystem event) and `rescan()`
/// (T-2.5, per file already on disk). Order matches SPEC.md §5
/// `WatchConfig`: ignored-name/temp-file check first (cheapest, no config
/// needed), then extension allowlist, then the size cap.
///
/// - `stabilize::should_ignore`: editor lock files (`~$*`), dotfiles, and
///   known temp extensions (`.tmp`, `.crdownload`, `.part`, `.partial`,
///   `.download`) are dropped regardless of config.
/// - `watch.extensions`: case-insensitive allowlist; an empty list means
///   "no filter, allow every extension" (SPEC.md §5 default `[]`).
/// - `watch.min_size_mb`: files strictly smaller than this are dropped;
///   `0` (the default) disables the floor.
/// - `watch.max_size_mb`: files strictly larger than this are dropped;
///   `0` (the default) means no ceiling, leaving only the destination's
///   own limit (`config::MAX_FILE_SIZE_MB`, enforced by the uploaders).
pub fn passes_filters(path: &Path, size: u64, watch: &WatchConfig) -> bool {
    if stabilize::should_ignore(path) {
        return false;
    }

    if !watch.extensions.is_empty() {
        let ext_allowed = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| {
                watch
                    .extensions
                    .iter()
                    .any(|allowed| allowed.eq_ignore_ascii_case(ext))
            })
            .unwrap_or(false);
        if !ext_allowed {
            return false;
        }
    }

    if watch.min_size_mb > 0 {
        let min_bytes = u64::from(watch.min_size_mb) * 1024 * 1024;
        if size < min_bytes {
            return false;
        }
    }

    if watch.max_size_mb > 0 {
        let max_bytes = u64::from(watch.max_size_mb) * 1024 * 1024;
        if size > max_bytes {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration;

    use tempfile::NamedTempFile;

    fn shared_repo() -> Arc<Mutex<Repo>> {
        Arc::new(Mutex::new(
            Repo::open_in_memory().expect("open in-memory repo"),
        ))
    }

    fn write_temp(contents: &[u8]) -> NamedTempFile {
        let mut file = NamedTempFile::new().expect("create temp file");
        file.write_all(contents).expect("write temp file");
        file.flush().expect("flush temp file");
        file
    }

    /// Canonicalized parent dir of a `write_temp` file, usable as the
    /// `root` argument to `intake` in tests that don't care about
    /// containment (the file always lives directly inside it).
    fn temp_root(file: &NamedTempFile) -> PathBuf {
        crate::paths::canonicalize_clean_sync(
            file.path().parent().expect("temp file has a parent dir"),
        )
        .expect("canonicalize temp dir")
    }

    fn fixed_mtime() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    // --- intake: Created -----------------------------------------------

    #[tokio::test]
    async fn intake_on_new_file_creates_two_pending_jobs_and_wakes_both_workers() {
        let repo = shared_repo();
        let wakers = Wakers::new();
        let file = write_temp(b"hello world");

        let s3_waiter = wakers.waiter(Destination::S3);
        let gdrive_waiter = wakers.waiter(Destination::GDrive);
        let s3_notified = s3_waiter.notified();
        let gdrive_notified = gdrive_waiter.notified();
        tokio::pin!(s3_notified, gdrive_notified);

        let outcome = intake(
            repo.clone(),
            &wakers,
            &temp_root(&file),
            file.path().to_path_buf(),
            11,
            fixed_mtime(),
        )
        .await
        .expect("intake should succeed");

        assert_eq!(outcome.outcome, UpsertOutcome::Created);
        assert_eq!(outcome.size, 11);
        assert!(!outcome.file_id.is_empty());
        assert_eq!(outcome.sha256.len(), 64);

        let counts = with_repo(repo.clone(), |repo| repo.status_counts())
            .await
            .expect("status_counts");
        assert_eq!(counts.pending, 2, "one pending job per destination");

        tokio::time::timeout(Duration::from_millis(50), s3_notified)
            .await
            .expect("s3 worker should have been woken");
        tokio::time::timeout(Duration::from_millis(50), gdrive_notified)
            .await
            .expect("gdrive worker should have been woken");
    }

    // --- intake: Unchanged ------------------------------------------------

    #[tokio::test]
    async fn intake_on_same_file_again_is_unchanged_and_does_not_wake_workers() {
        let repo = shared_repo();
        let wakers = Wakers::new();
        let file = write_temp(b"hello world");

        intake(
            repo.clone(),
            &wakers,
            &temp_root(&file),
            file.path().to_path_buf(),
            11,
            fixed_mtime(),
        )
        .await
        .expect("first intake should succeed");

        let s3_waiter = wakers.waiter(Destination::S3);
        let notified = s3_waiter.notified();
        tokio::pin!(notified);

        let outcome = intake(
            repo.clone(),
            &wakers,
            &temp_root(&file),
            file.path().to_path_buf(),
            11,
            fixed_mtime(),
        )
        .await
        .expect("second intake should succeed");

        assert_eq!(outcome.outcome, UpsertOutcome::Unchanged);

        let counts = with_repo(repo.clone(), |repo| repo.status_counts())
            .await
            .expect("status_counts");
        assert_eq!(
            counts.pending, 2,
            "no new jobs created for an unchanged file"
        );

        // No notification should arrive: waiting for it should time out.
        let result = tokio::time::timeout(Duration::from_millis(50), notified).await;
        assert!(
            result.is_err(),
            "worker should NOT be woken for an unchanged file"
        );
    }

    // --- intake: Rehashed ---------------------------------------------------

    #[tokio::test]
    async fn intake_after_content_changes_is_rehashed_and_resets_jobs() {
        let repo = shared_repo();
        let wakers = Wakers::new();
        let mut file = write_temp(b"hello world");

        let first = intake(
            repo.clone(),
            &wakers,
            &temp_root(&file),
            file.path().to_path_buf(),
            11,
            fixed_mtime(),
        )
        .await
        .expect("first intake should succeed");

        // Move at least one job forward so we can observe the reset.
        with_repo(repo.clone(), {
            let file_id = first.file_id.clone();
            move |repo| {
                let now = "2026-01-01T00:00:00Z";
                let job = repo
                    .claim_next(Destination::S3, now)?
                    .expect("s3 job should be pending");
                assert_eq!(job.file_id, file_id);
                repo.mark_done(&job.id, "remote-1", None)
            }
        })
        .await
        .expect("advance s3 job to done");

        let s3_waiter = wakers.waiter(Destination::S3);
        let notified = s3_waiter.notified();
        tokio::pin!(notified);

        file.as_file_mut()
            .set_len(0)
            .expect("truncate before rewrite");
        std::io::Write::write_all(file.as_file_mut(), b"goodbye world, longer now")
            .expect("rewrite file contents");
        file.as_file_mut().flush().expect("flush rewrite");

        let rehashed = intake(
            repo.clone(),
            &wakers,
            &temp_root(&file),
            file.path().to_path_buf(),
            26,
            fixed_mtime(),
        )
        .await
        .expect("rehash intake should succeed");

        assert_eq!(rehashed.outcome, UpsertOutcome::Rehashed);
        assert_eq!(rehashed.file_id, first.file_id, "same path, same file_id");
        assert_ne!(rehashed.sha256, first.sha256);

        let counts = with_repo(repo.clone(), |repo| repo.status_counts())
            .await
            .expect("status_counts");
        assert_eq!(
            counts.pending, 2,
            "both jobs reset to pending, no extra pair"
        );
        assert_eq!(counts.done, 0, "the previously-done job was reset too");

        tokio::time::timeout(Duration::from_millis(50), notified)
            .await
            .expect("worker should be woken again after a rehash");
    }

    // --- intake: containment (VULN-003) ---------------------------------

    #[tokio::test]
    async fn intake_rejects_a_path_outside_the_given_root() {
        let repo = shared_repo();
        let wakers = Wakers::new();

        // `root` is one temp dir; `file` lives in a different one (a stand-in
        // for a symlink inside `root` resolving to somewhere else entirely).
        let root_dir = tempfile::tempdir().expect("create root tempdir");
        let root =
            crate::paths::canonicalize_clean_sync(root_dir.path()).expect("canonicalize root");
        let outside_file = write_temp(b"outside root");

        let err = intake(
            repo.clone(),
            &wakers,
            &root,
            outside_file.path().to_path_buf(),
            12,
            fixed_mtime(),
        )
        .await
        .expect_err("intake must reject a path outside root");

        assert!(
            matches!(err, QueueError::OutsideWatchRoot(_)),
            "expected OutsideWatchRoot, got {err:?}"
        );

        let counts = with_repo(repo.clone(), |repo| repo.status_counts())
            .await
            .expect("status_counts");
        assert_eq!(counts.pending, 0, "no jobs created for a rejected path");
    }

    // --- intake: Windows verbatim-path normalization ---------------------

    /// On Windows, `std::fs::canonicalize` (what `intake` calls internally) returns
    /// the verbatim form (`\\?\C:\...`). Without `paths::canonicalize_clean`, that
    /// verbatim string would land straight in `files.path` and get shown as-is in
    /// the UI. This asserts the row `intake` writes never carries that prefix.
    #[cfg(windows)]
    #[tokio::test]
    async fn intake_never_stores_a_windows_verbatim_prefix_in_files_path() {
        let repo = shared_repo();
        let wakers = Wakers::new();
        let file = write_temp(b"hello world");
        let root = temp_root(&file); // already clean, per `canonicalize_clean_sync`.
        let expected_path = root.join(file.path().file_name().expect("temp file has a name"));

        intake(
            repo.clone(),
            &wakers,
            &root,
            file.path().to_path_buf(),
            11,
            fixed_mtime(),
        )
        .await
        .expect("intake should succeed");

        let expected_path_str = expected_path.to_string_lossy().into_owned();
        let stored = with_repo(repo.clone(), move |repo| {
            repo.file_by_path(&expected_path_str)
        })
        .await
        .expect("file_by_path query should succeed")
        .expect("row should exist under the clean (non-verbatim) path");

        assert!(
            !stored.path.contains(r"\\?\"),
            "files.path must never carry a verbatim prefix, got {:?}",
            stored.path
        );
    }

    /// Non-Windows twin of the test above: there is no OS-level verbatim prefix to
    /// provoke through a real `canonicalize` call here, so this exercises
    /// `paths::canonicalize_clean` directly on a real tempdir and asserts the
    /// stripping is a no-op that never introduces (or leaves) a `\\?\` marker —
    /// the same guarantee `intake` relies on for its `files.path` value.
    #[cfg(not(windows))]
    #[tokio::test]
    async fn intake_never_stores_a_windows_verbatim_prefix_in_files_path() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let cleaned = crate::paths::canonicalize_clean(dir.path())
            .await
            .expect("canonicalize_clean should succeed on an existing dir");

        assert!(
            !cleaned.to_string_lossy().contains(r"\\?\"),
            "canonicalize_clean must never leak a verbatim prefix, got {cleaned:?}"
        );
    }

    // --- with_repo / SharedRepo ----------------------------------------

    #[tokio::test]
    async fn with_repo_runs_closure_against_the_shared_repo() {
        let repo = shared_repo();
        let counts = with_repo(repo, |repo| repo.status_counts())
            .await
            .expect("status_counts via with_repo");
        assert_eq!(counts.pending, 0);
    }

    #[tokio::test]
    async fn shared_repo_wrapper_delegates_to_with_repo() {
        let shared = SharedRepo::new(Repo::open_in_memory().expect("open in-memory repo"));
        let counts = shared
            .with_repo(|repo| repo.status_counts())
            .await
            .expect("status_counts via SharedRepo");
        assert_eq!(counts.pending, 0);
    }

    // --- passes_filters --------------------------------------------------

    fn watch_cfg(extensions: &[&str], max_size_mb: u32) -> WatchConfig {
        WatchConfig {
            path: None,
            recursive: false,
            extensions: extensions.iter().map(|s| s.to_string()).collect(),
            min_size_mb: 0,
            max_size_mb,
            stabilize_seconds: 3,
        }
    }

    #[test]
    fn passes_filters_table() {
        let one_mb = 1024 * 1024;

        struct Case {
            name: &'static str,
            path: &'static str,
            size: u64,
            extensions: &'static [&'static str],
            max_size_mb: u32,
            expected: bool,
        }

        let cases = [
            Case {
                name: "no extension filter allows anything under the size cap",
                path: "report.pdf",
                size: 100,
                extensions: &[],
                max_size_mb: 10,
                expected: true,
            },
            Case {
                name: "allowlisted extension passes",
                path: "report.pdf",
                size: 100,
                extensions: &["pdf", "csv"],
                max_size_mb: 10,
                expected: true,
            },
            Case {
                name: "extension not in allowlist is rejected",
                path: "video.mp4",
                size: 100,
                extensions: &["pdf", "csv"],
                max_size_mb: 10,
                expected: false,
            },
            Case {
                name: "allowlist match is case-insensitive",
                path: "REPORT.PDF",
                size: 100,
                extensions: &["pdf"],
                max_size_mb: 10,
                expected: true,
            },
            Case {
                name: "file exactly at the size cap passes",
                path: "big.pdf",
                size: 10 * one_mb,
                extensions: &[],
                max_size_mb: 10,
                expected: true,
            },
            Case {
                name: "file over the size cap is rejected",
                path: "big.pdf",
                size: 10 * one_mb + 1,
                extensions: &[],
                max_size_mb: 10,
                expected: false,
            },
            Case {
                name: "editor lock temp name is always rejected",
                path: "~$document.docx",
                size: 100,
                extensions: &[],
                max_size_mb: 10,
                expected: false,
            },
            Case {
                name: "known temp extension is always rejected even if allowlisted",
                path: "download.tmp",
                size: 100,
                extensions: &["tmp"],
                max_size_mb: 10,
                expected: false,
            },
            Case {
                name: "dotfile is always rejected",
                path: ".hidden",
                size: 100,
                extensions: &[],
                max_size_mb: 10,
                expected: false,
            },
        ];

        for case in cases {
            let cfg = watch_cfg(case.extensions, case.max_size_mb);
            let actual = passes_filters(Path::new(case.path), case.size, &cfg);
            assert_eq!(actual, case.expected, "case failed: {}", case.name);
        }
    }

    // `max_size_mb: 0` is the default and must mean "no ceiling", not
    // "reject everything" -- the whole point of the change that introduced
    // it was letting very large files through.
    #[test]
    fn passes_filters_treats_max_size_zero_as_no_ceiling() {
        let cfg = watch_cfg(&[], 0);
        let huge = 900 * 1024 * 1024 * 1024; // 900 GiB

        assert!(passes_filters(Path::new("export.bak"), huge, &cfg));
    }

    // The floor drops anything strictly smaller; the boundary itself passes.
    #[test]
    fn passes_filters_enforces_the_minimum_size() {
        let one_mb = 1024 * 1024;
        let mut cfg = watch_cfg(&[], 0);
        cfg.min_size_mb = 10;

        assert!(!passes_filters(Path::new("small.csv"), 9 * one_mb, &cfg));
        assert!(passes_filters(Path::new("exact.csv"), 10 * one_mb, &cfg));
        assert!(passes_filters(Path::new("big.csv"), 11 * one_mb, &cfg));
    }

    #[test]
    fn passes_filters_minimum_is_off_by_default() {
        let cfg = watch_cfg(&[], 0);

        assert!(passes_filters(Path::new("tiny.csv"), 1, &cfg));
    }

    // Floor and ceiling together define a closed band, both ends inclusive.
    #[test]
    fn passes_filters_applies_both_bounds_together() {
        let one_mb = 1024 * 1024;
        let mut cfg = watch_cfg(&[], 20);
        cfg.min_size_mb = 10;

        assert!(!passes_filters(Path::new("under.csv"), 9 * one_mb, &cfg));
        assert!(passes_filters(Path::new("inside.csv"), 15 * one_mb, &cfg));
        assert!(!passes_filters(Path::new("over.csv"), 21 * one_mb, &cfg));
    }
}
