//! `core::rescan` — startup/resume/manual re-scan of the watched folder
//! (PLAN.md T-2.5, SPEC.md §2 "Fluxo de um arquivo", §6 "rescan()";
//! PRD.md RF-004, RF-094, RNF-007).
//!
//! `rescan()` exists because the filesystem watcher (`core::watcher`) can
//! miss events entirely — the app was closed, `notify` dropped an event
//! during a very large copy, or the watcher was paused. It is the
//! "reconciliation" half of SPEC.md §2's pipeline: it walks `watch.path`
//! on disk and compares it against the `files` table, enqueueing anything
//! new or changed via the exact same [`crate::queue::intake`] used by the
//! live watcher, so the two paths can never disagree about what "changed"
//! means.
//!
//! Per SPEC.md §5 "Regras de escrita", a rescan must not re-hash every file
//! on every run (RF-094: ≤ 60 s for 500 files) — that would make it as
//! expensive as a full initial scan every time. So for each candidate file,
//! [`Repo::file_by_path`] is checked first: if a row already exists with the
//! *same* `size` **and** `mtime` as what's on disk right now, the file is
//! reported [`RescanReport::unchanged`] without touching
//! [`crate::stabilize::wait_until_stable`] or [`crate::hash`] at all — just
//! the one stat done during the directory walk. Hashing (and the
//! stabilization wait that must precede it, since a rescan can catch a file
//! mid-copy just like the live watcher can) only happens when that fast
//! check fails to match: a brand-new file, or an existing one whose
//! `size`/`mtime` moved.
//!
//! Concurrency is bounded to [`CHUNK_SIZE`] simultaneous
//! stabilize/hash/intake operations (manual chunking via `tokio::join!` —
//! this crate does not depend on `futures`, and nothing here needs the
//! `'static` bound `tokio::spawn`/`JoinSet` would require, since `wakers` is
//! borrowed for the whole call) so that scanning a folder with hundreds of
//! files does not attempt to hash all of them at once.
//!
//! A per-file failure (I/O, stabilize, or queue error) is logged via
//! `tracing::warn!` and counted in [`RescanReport::errors`] — it never
//! aborts the scan; the remaining files are still processed.
//!
//! Process-wide, [`rescan_inner`] also enforces that **at most one scan runs
//! at a time** (PLAN.md T-2.5, [`RescanGuard`]): there are six independent
//! call sites (boot, the manual "Atualizar Lista" button, the periodic
//! reconcile loop, tray, resume-from-pause and resume-from-sleep) and
//! nothing else coordinates between them. A second concurrent call returns
//! [`RescanError::AlreadyInProgress`] immediately rather than queuing behind
//! the first — two overlapping full-tree walks would double the disk reads
//! competing with the uploaders for no benefit, since reconciliation is
//! idempotent and the next scan (whichever one runs next) simply picks up
//! whatever the in-flight one missed.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::time::Instant as TokioInstant;
use ts_rs::TS;

use crate::config::WatchConfig;
use crate::queue::{intake, passes_filters, with_repo, QueueError, Wakers};
use crate::stabilize::{should_ignore_meta, wait_until_stable, StabilizeConfig};
use crate::state::{Repo, UpsertOutcome};

/// Number of files processed concurrently during the hash/intake stage of a
/// [`rescan`] (see the module-level docs for why this is manual chunking
/// rather than a `Semaphore` + spawned tasks).
const CHUNK_SIZE: usize = 4;

/// Progress sink for [`rescan_with_progress`]'s hash/intake phase — the same
/// "closure the core crate can call without depending on `tauri`" shape as
/// `uploaders::s3::StateSink`/`uploaders::gdrive::StateSink`. Called with
/// `(processed, total)` candidates already stat'ed and filtered — `total` is
/// known up front (the walk collects every candidate before this phase
/// starts) and is *not* [`RescanReport::scanned`], which also counts entries
/// `passes_filters` rejected before ever reaching here. Calls are throttled
/// to [`PROGRESS_MIN_INTERVAL`] by [`rescan_inner`]; the sink itself just
/// renders/emits whatever it's given.
pub type RescanProgressSink = Arc<dyn Fn(u32, u32) + Send + Sync>;

/// Minimum spacing between [`RescanProgressSink`] calls, plus always a final
/// call at `processed == total` — mirrors `worker.rs`'s `forward_progress`
/// 500 ms throttle for `upload-progress`, so the two progress-style events
/// share one cadence. Without this, a chunk of only [`CHUNK_SIZE`] files
/// finishing against a fast local SSD would fire the sink hundreds of times
/// a second.
const PROGRESS_MIN_INTERVAL: Duration = Duration::from_millis(500);

/// Summary counters for one [`rescan`] run.
///
/// The `rescan` IPC command (PLAN.md T-2.6) returns just the enqueued count
/// per SPEC.md §7, but this full breakdown is useful for structured logs
/// and a future UI summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, TS)]
#[ts(export)]
pub struct RescanReport {
    /// Total non-directory entries encountered under `watch.path` (before
    /// any filtering).
    pub scanned: u32,
    /// Files that were new or had changed content: hashed, (re)enqueued,
    /// and every worker woken.
    pub enqueued: u32,
    /// Files whose `files` row already matched on `size`+`mtime` (fast
    /// path, no hash), or whose hash matched despite a `size`/`mtime`
    /// mismatch (slow path, `UpsertOutcome::Unchanged`): no jobs touched.
    pub unchanged: u32,
    /// Files dropped by `should_ignore_meta`/`passes_filters` before ever
    /// reaching the database.
    pub skipped_filtered: u32,
    /// Symlinks (file or directory) encountered under `watch.path` and
    /// skipped without following them (VULN-003): a symlink inside the
    /// watched folder could otherwise be used to read or enqueue a file
    /// outside it. Logged via `tracing::warn!` when it happens.
    pub skipped_symlink: u32,
    /// Directory entries the walk could not read: a subdirectory whose
    /// `read_dir`/`next_entry` failed (permission denied, e.g. Windows'
    /// `System Volume Information`), an entry whose `symlink_metadata`
    /// failed (deleted between `read_dir` and the `stat`, or locked by
    /// another process), or a Windows system path skipped by
    /// [`crate::paths::is_system_path`]. Never aborts the scan — logged via
    /// `tracing::warn!` and counted, except when the ROOT itself cannot be
    /// read, which is a genuine [`RescanError::Io`].
    pub skipped_unreadable: u32,
    /// Per-file failures (I/O, stabilize, or queue errors). Logged via
    /// `tracing::warn!` when they happen; the scan continues regardless.
    pub errors: u32,
    /// Files whose queued jobs were archived by the reconciliation pass
    /// because they no longer pass the current `watch` filters, or are gone
    /// from disk (RF-004).
    pub archived: u32,
    /// Files whose previously archived jobs came back because they pass the
    /// filters again — the other half of the reconciliation, without which
    /// widening a filter would be a one-way door.
    pub restored: u32,
}

/// Errors that abort an entire [`rescan`] call (as opposed to a single
/// file — see [`RescanReport::errors`] for those).
#[derive(Debug, thiserror::Error)]
pub enum RescanError {
    /// `watch.path` is not configured (RF-001: no folder chosen yet).
    #[error("rescan requires watch.path to be configured")]
    NoPath,
    /// Walking `watch.path` itself failed (e.g. the folder is gone).
    #[error("io error while scanning: {0}")]
    Io(#[from] std::io::Error),
    /// A `core::queue` operation failed outside the per-file loop.
    #[error("queue error: {0}")]
    Queue(#[from] QueueError),
    /// Another `rescan()`/`rescan_with_progress()` call is already running
    /// process-wide (PLAN.md T-2.5). Returned immediately by
    /// [`RescanGuard::try_acquire`] rather than queuing behind the in-flight
    /// scan — see the module docs for why. The manual "Atualizar Lista"
    /// button (`commands::queue::rescan`) surfaces this distinctly to the
    /// user rather than showing "0 arquivos enfileirados", which would look
    /// exactly like the bug this guard exists to prevent; every other
    /// (background) call site treats it as a normal no-op via
    /// `events::notify_rescan_failure`.
    #[error("uma varredura já está em andamento")]
    AlreadyInProgress,
}

/// Process-wide "at most one rescan running" flag (PLAN.md T-2.5). Never
/// read or written directly outside [`RescanGuard`] — that's what keeps the
/// invariant "`true` iff a `RescanGuard` is currently alive" from being
/// broken by a stray access elsewhere in this module.
static RESCAN_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// RAII handle on [`RESCAN_IN_FLIGHT`], held for the lifetime of one
/// [`rescan_inner`] call.
///
/// Chosen over a hand-rolled "clear the flag before every return" because
/// `rescan_inner` has several early-return sites (`NoPath`, the initial
/// `walk_dir` `?`, the `reconcile` `?`) *and* one early-exit path that is
/// not a `return` statement at all: `runtime.rs`'s `run_reconcile_loop`
/// wraps its `on_tick()` call (which runs a full `rescan()`) in a
/// `tokio::select!` against `cancel`, and drops the `on_tick()` future
/// outright, mid-scan, when the app is shutting down and `cancel` wins the
/// race. A bare flag cleared only at the bottom of the function would leak
/// forever on that path — silently disabling every future rescan for the
/// rest of the process's life, a worse bug than the one this guard fixes.
/// `Drop` runs on every one of these exits alike (normal return, `?`
/// propagation, or the future simply being dropped), so it is the one
/// guard shape that cannot miss one of them.
struct RescanGuard;

impl RescanGuard {
    /// Attempts to become the one in-flight scan. `None` means another
    /// `rescan()`/`rescan_with_progress()` call already holds it — the
    /// caller must return [`RescanError::AlreadyInProgress`] immediately
    /// rather than block, since queuing behind it would just run the same
    /// full-tree walk twice back-to-back.
    fn try_acquire() -> Option<Self> {
        RESCAN_IN_FLIGHT
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| RescanGuard)
    }
}

impl Drop for RescanGuard {
    fn drop(&mut self) {
        RESCAN_IN_FLIGHT.store(false, Ordering::Release);
    }
}

/// Walks `watch.path` honoring `watch.recursive`, skipping filters, and
/// enqueueing every file that is new or whose content changed since the
/// last scan (SPEC.md §6). See the module docs for the fast-path/slow-path
/// split and the bounded concurrency used for the slow path.
///
/// Reports no progress along the way — used by the boot, resume-from-pause,
/// resume-from-sleep and tray call sites, none of which have anywhere to
/// show it today. See [`rescan_with_progress`] for the one call site that
/// does (the manual "Atualizar Lista" command).
pub async fn rescan(
    repo: Arc<Mutex<Repo>>,
    wakers: &Wakers,
    watch: &WatchConfig,
    stabilize: &StabilizeConfig,
) -> Result<RescanReport, RescanError> {
    rescan_inner(repo, wakers, watch, stabilize, None).await
}

/// Same as [`rescan`], but calls `progress` with `(processed, total)` as the
/// hash/intake phase advances (throttled to [`PROGRESS_MIN_INTERVAL`] — see
/// [`RescanProgressSink`]). Wired into the manual rescan command so the UI
/// can show a live count instead of an indeterminate spinner during a scan
/// that can take minutes over a large folder (SPEC.md §6, PLAN.md T-2.4).
pub async fn rescan_with_progress(
    repo: Arc<Mutex<Repo>>,
    wakers: &Wakers,
    watch: &WatchConfig,
    stabilize: &StabilizeConfig,
    progress: &RescanProgressSink,
) -> Result<RescanReport, RescanError> {
    rescan_inner(repo, wakers, watch, stabilize, Some(progress)).await
}

/// Shared implementation behind [`rescan`]/[`rescan_with_progress`] — see
/// their docs; `progress` is `None` for every call site that has nowhere to
/// show it.
async fn rescan_inner(
    repo: Arc<Mutex<Repo>>,
    wakers: &Wakers,
    watch: &WatchConfig,
    stabilize: &StabilizeConfig,
    progress: Option<&RescanProgressSink>,
) -> Result<RescanReport, RescanError> {
    // Must be the very first thing this function does, before any `.await`:
    // that is what makes acquisition synchronous and race-free with respect
    // to any other in-flight call, whatever executor/poll order drives them.
    // `_guard`'s `Drop` releases the flag on every exit below, including the
    // `?` early-returns and a cancelled/dropped future (see `RescanGuard`'s
    // docs).
    let Some(_guard) = RescanGuard::try_acquire() else {
        return Err(RescanError::AlreadyInProgress);
    };

    let root = watch.path.as_deref().ok_or(RescanError::NoPath)?;
    // Same canonical form the watcher reports (see `queue::intake`), so the
    // `file_by_path` short-circuit below hits instead of re-hashing every run.
    let root_buf = crate::paths::canonicalize_clean(std::path::Path::new(root))
        .await
        .unwrap_or_else(|_| std::path::PathBuf::from(root));
    let root = root_buf;

    let mut report = RescanReport::default();
    let (entries, skipped_symlink, skipped_unreadable) = walk_dir(&root, watch.recursive).await?;
    report.skipped_symlink = skipped_symlink;
    report.skipped_unreadable = skipped_unreadable;

    // Every path the walk saw, filtered or not — the reconciliation pass below
    // uses it to tell "gone from disk" from "still there but now excluded".
    let mut on_disk = HashSet::with_capacity(entries.len());
    let mut candidates = Vec::with_capacity(entries.len());
    for (path, metadata) in entries {
        report.scanned += 1;
        on_disk.insert(path.clone());
        if should_ignore_meta(&path, Some(&metadata))
            || !passes_filters(&path, metadata.len(), watch)
        {
            report.skipped_filtered += 1;
            continue;
        }
        candidates.push((path, metadata));
    }

    // Known up front — the walk above already collected every candidate — so
    // the first progress emission already carries a meaningful denominator
    // instead of the UI having to guess at one.
    let total_candidates = candidates.len() as u32;
    let mut processed_candidates: u32 = 0;
    let mut last_progress_emit: Option<TokioInstant> = None;

    for chunk in candidates.chunks(CHUNK_SIZE) {
        let chunk_len = chunk.len() as u32;
        for outcome in run_chunk(&repo, wakers, &root, stabilize, chunk).await {
            match outcome {
                CandidateOutcome::Enqueued => report.enqueued += 1,
                CandidateOutcome::Unchanged => report.unchanged += 1,
                CandidateOutcome::Error => report.errors += 1,
            }
        }
        processed_candidates += chunk_len;

        if let Some(sink) = progress {
            let is_final = processed_candidates >= total_candidates;
            let should_emit = is_final
                || last_progress_emit
                    .is_none_or(|emitted_at| emitted_at.elapsed() >= PROGRESS_MIN_INTERVAL);
            if should_emit {
                sink(processed_candidates, total_candidates);
                last_progress_emit = Some(TokioInstant::now());
            }
        }
    }

    // Order is load-bearing: the enqueue pass above has already refreshed
    // `files.size` for anything whose size moved on disk, so the sweep can
    // judge every row from the database alone and needs no `stat` of its own
    // beyond the existence check below.
    let (archived, restored) = reconcile(&repo, watch, &root, &on_disk).await?;
    report.archived = archived;
    report.restored = restored;

    tracing::info!(?report, "varredura concluída");
    Ok(report)
}

/// Brings already-queued jobs back in line with `watch` (RF-004).
///
/// A queued file is dropped from the visible queue when it is any of:
///
/// - **out of scope** — outside `root`, or nested when `recursive` is now
///   `false`. Scope is computed from the path, not from what the walk found:
///   a file that fell out of scope is precisely one the walk no longer visits,
///   so "the walk did not see it" cannot distinguish out-of-scope from gone,
///   and treating existence as scope would strand it in the queue forever.
/// - **gone from disk** — checked with one `stat`, and only for candidates the
///   walk did not just see. The walk's paths and `files.path` are both
///   canonical, but an existence check is authoritative and costs nothing for
///   the majority, which keeps a path-normalisation mismatch from silently
///   emptying the queue.
/// - **rejected by the filters** — `passes_filters` against the stored size,
///   which the enqueue pass above has already refreshed for anything that
///   moved on disk. That ordering is why no extra `stat` is needed here.
///
/// All of the I/O happens in this function, outside `with_repo`, so the
/// blocking closure that runs the transaction never waits on a disk.
///
/// A watched folder that is a network share and mounts *empty* would archive
/// everything — but nothing is lost: `archived_at` is reversible and the
/// restore half of [`Repo::reconcile_jobs_with_filters`] brings every row back
/// on the first rescan after the share returns.
async fn reconcile(
    repo: &Arc<Mutex<Repo>>,
    watch: &WatchConfig,
    root: &Path,
    on_disk: &HashSet<PathBuf>,
) -> Result<(u32, u32), RescanError> {
    let candidates = with_repo(repo.clone(), |repo| repo.files_with_sweepable_jobs()).await?;

    let mut rejected = HashSet::new();
    for candidate in &candidates {
        let path = PathBuf::from(&candidate.path);

        let in_scope = path.starts_with(root) && (watch.recursive || path.parent() == Some(root));
        if !in_scope {
            rejected.insert(candidate.path.clone());
            continue;
        }

        if on_disk.contains(&path) {
            continue;
        }
        if tokio::fs::symlink_metadata(&path).await.is_err() {
            rejected.insert(candidate.path.clone());
        }
    }

    let watch = watch.clone();
    let now = Utc::now().to_rfc3339();
    let (archived, restored) = with_repo(repo.clone(), move |repo| {
        repo.reconcile_jobs_with_filters(&now, |path, size| {
            if rejected.contains(path) {
                return false;
            }
            let size = u64::try_from(size).unwrap_or(u64::MAX);
            passes_filters(std::path::Path::new(path), size, &watch)
        })
    })
    .await?;

    Ok((archived, restored))
}

/// Recursively (when `recursive`) lists every non-directory entry under
/// `root`, paired with the `metadata` already read for it — the caller
/// reuses that `metadata` for both filtering and the fast-path `size`
/// comparison, so each file is `stat`-ed exactly once per scan. Also
/// returns the number of symlinks (file or directory) skipped along the
/// way, and the number of unreadable/system entries skipped (see
/// [`RescanReport::skipped_unreadable`]).
///
/// VULN-003: uses `symlink_metadata` (never follows the final component) so
/// a symlink is detected *before* deciding what to do with it — a symlinked
/// file or directory anywhere under `root` is skipped entirely (not
/// descended into, not hashed/enqueued) rather than followed, since it
/// could otherwise be used to read or enqueue a file outside the watched
/// folder. Each skip is logged via `tracing::warn!` and counted.
///
/// Fault tolerance (the #1 field bug — `E:\System Volume Information`
/// denies access to everyone, os error 5): a `read_dir`/`next_entry`/
/// `symlink_metadata` failure on anything *except* `root` itself is logged
/// and skipped rather than aborting the whole walk. `root` is the one
/// directory pushed onto `dirs` before the loop starts, so it is
/// necessarily the first one popped — its `read_dir` error is the only one
/// still propagated, via `?`, becoming [`RescanError::Io`] in the caller.
///
/// Manual `tokio::fs::read_dir` + an explicit stack, on purpose: this crate
/// does not depend on `walkdir`.
async fn walk_dir(
    root: &Path,
    recursive: bool,
) -> std::io::Result<(Vec<(PathBuf, std::fs::Metadata)>, u32, u32)> {
    let mut files = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    let mut skipped_symlink: u32 = 0;
    let mut skipped_unreadable: u32 = 0;
    let mut is_root = true;

    while let Some(dir) = dirs.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(entries) => entries,
            Err(err) => {
                if is_root {
                    return Err(err);
                }
                tracing::warn!(path = %dir.display(), error = %err, "varredura: diretório ilegível, pulando");
                skipped_unreadable += 1;
                continue;
            }
        };
        is_root = false;

        loop {
            let entry = match entries.next_entry().await {
                Ok(Some(entry)) => entry,
                Ok(None) => break,
                Err(err) => {
                    tracing::warn!(path = %dir.display(), error = %err, "varredura: diretório ilegível, pulando");
                    skipped_unreadable += 1;
                    break;
                }
            };
            let path = entry.path();

            if crate::paths::is_system_path(&path) {
                tracing::warn!(path = %path.display(), "varredura: ignorando caminho de sistema");
                skipped_unreadable += 1;
                continue;
            }

            let metadata = match tokio::fs::symlink_metadata(&path).await {
                Ok(metadata) => metadata,
                Err(err) => {
                    tracing::warn!(path = %path.display(), error = %err, "varredura: entrada ilegível, pulando");
                    skipped_unreadable += 1;
                    continue;
                }
            };
            if metadata.is_symlink() {
                tracing::warn!(path = %path.display(), "varredura: ignorando link simbólico");
                skipped_symlink += 1;
                continue;
            }
            if metadata.is_dir() {
                if recursive {
                    dirs.push(path);
                }
                continue;
            }
            files.push((path, metadata));
        }
    }

    Ok((files, skipped_symlink, skipped_unreadable))
}

/// Outcome of processing one already-filtered candidate file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidateOutcome {
    Enqueued,
    Unchanged,
    Error,
}

/// Runs up to [`CHUNK_SIZE`] candidates concurrently via `tokio::join!`.
///
/// `chunk` always comes from `candidates.chunks(CHUNK_SIZE)`, so it never
/// holds more than [`CHUNK_SIZE`] items; the match below is exhaustive for
/// that invariant.
async fn run_chunk(
    repo: &Arc<Mutex<Repo>>,
    wakers: &Wakers,
    root: &Path,
    stabilize: &StabilizeConfig,
    chunk: &[(PathBuf, std::fs::Metadata)],
) -> Vec<CandidateOutcome> {
    match chunk {
        [] => Vec::new(),
        [a] => {
            vec![
                process_candidate(
                    repo.clone(),
                    wakers,
                    root,
                    stabilize,
                    a.0.clone(),
                    a.1.clone(),
                )
                .await,
            ]
        }
        [a, b] => {
            let (ra, rb) = tokio::join!(
                process_candidate(
                    repo.clone(),
                    wakers,
                    root,
                    stabilize,
                    a.0.clone(),
                    a.1.clone()
                ),
                process_candidate(
                    repo.clone(),
                    wakers,
                    root,
                    stabilize,
                    b.0.clone(),
                    b.1.clone()
                ),
            );
            vec![ra, rb]
        }
        [a, b, c] => {
            let (ra, rb, rc) = tokio::join!(
                process_candidate(
                    repo.clone(),
                    wakers,
                    root,
                    stabilize,
                    a.0.clone(),
                    a.1.clone()
                ),
                process_candidate(
                    repo.clone(),
                    wakers,
                    root,
                    stabilize,
                    b.0.clone(),
                    b.1.clone()
                ),
                process_candidate(
                    repo.clone(),
                    wakers,
                    root,
                    stabilize,
                    c.0.clone(),
                    c.1.clone()
                ),
            );
            vec![ra, rb, rc]
        }
        [a, b, c, d] => {
            let (ra, rb, rc, rd) = tokio::join!(
                process_candidate(
                    repo.clone(),
                    wakers,
                    root,
                    stabilize,
                    a.0.clone(),
                    a.1.clone()
                ),
                process_candidate(
                    repo.clone(),
                    wakers,
                    root,
                    stabilize,
                    b.0.clone(),
                    b.1.clone()
                ),
                process_candidate(
                    repo.clone(),
                    wakers,
                    root,
                    stabilize,
                    c.0.clone(),
                    c.1.clone()
                ),
                process_candidate(
                    repo.clone(),
                    wakers,
                    root,
                    stabilize,
                    d.0.clone(),
                    d.1.clone()
                ),
            );
            vec![ra, rb, rc, rd]
        }
        _ => unreachable!("chunks(CHUNK_SIZE) never yields more than CHUNK_SIZE items"),
    }
}

/// The fast-path/slow-path split described in the module docs, for one
/// already-filtered file. Never returns an `Err` — every failure is logged
/// and folded into [`CandidateOutcome::Error`] so [`run_chunk`]'s caller
/// never has to decide whether to abort the rest of the scan.
async fn process_candidate(
    repo: Arc<Mutex<Repo>>,
    wakers: &Wakers,
    root: &Path,
    stabilize: &StabilizeConfig,
    path: PathBuf,
    metadata: std::fs::Metadata,
) -> CandidateOutcome {
    let path_str = path.to_string_lossy().into_owned();
    let size = metadata.len();
    let mtime_str = mtime_of(&metadata).to_rfc3339();

    let existing = {
        let lookup_path = path_str.clone();
        with_repo(repo.clone(), move |repo| repo.file_by_path(&lookup_path)).await
    };

    match existing {
        Ok(Some(row)) if row.size == size as i64 && row.mtime == mtime_str => {
            tracing::debug!(
                path = %path_str,
                "varredura: tamanho+mtime coincidem com registro existente, hash ignorado"
            );
            return CandidateOutcome::Unchanged;
        }
        Ok(_) => {}
        Err(err) => {
            tracing::warn!(path = %path_str, error = %err, "varredura: falha ao consultar registro de arquivo existente");
            return CandidateOutcome::Error;
        }
    }

    // Fast path missed (new file, or size/mtime moved): a rescan can catch a
    // file mid-copy just like the live watcher can, so it must wait for
    // stability before hashing too.
    let stable = match wait_until_stable(&path, stabilize).await {
        Ok(stable) => stable,
        Err(err) => {
            tracing::warn!(path = %path_str, error = %err, "varredura: falha ao aguardar estabilização do arquivo");
            return CandidateOutcome::Error;
        }
    };

    match intake(repo, wakers, root, path, stable.size, stable.mtime).await {
        Ok(outcome) => match outcome.outcome {
            UpsertOutcome::Created | UpsertOutcome::Rehashed => CandidateOutcome::Enqueued,
            UpsertOutcome::Unchanged => CandidateOutcome::Unchanged,
        },
        Err(err) => {
            tracing::warn!(path = %path_str, error = %err, "varredura: entrada de arquivo falhou");
            CandidateOutcome::Error
        }
    }
}

fn mtime_of(metadata: &std::fs::Metadata) -> DateTime<Utc> {
    metadata
        .modified()
        .ok()
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(Utc::now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use tempfile::tempdir;

    use crate::state::Destination;

    /// Serializes this module's test *bodies* against each other (NOT against the
    /// concurrency exercised *inside* a single test body via `tokio::join!`/
    /// `tokio::time::timeout`). Needed because `RESCAN_IN_FLIGHT` is a single
    /// process-wide static (T-2.5) and the default test harness runs
    /// `#[tokio::test]` functions concurrently on separate threads -- without this,
    /// two unrelated tests racing to call `rescan()` at the same wall-clock moment
    /// could spuriously observe `AlreadyInProgress` from each other. Every test
    /// function below takes this lock as its first statement, except the
    /// `visible_jobs` helper (not itself a test, never calls `rescan()`).
    static TEST_SERIAL: Mutex<()> = Mutex::new(());

    /// Acquires [`TEST_SERIAL`], recovering from a poisoned lock (a prior test
    /// panicking mid-body must not permanently deadlock every test after it).
    fn serial_guard() -> std::sync::MutexGuard<'static, ()> {
        TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn shared_repo() -> Arc<Mutex<Repo>> {
        Arc::new(Mutex::new(
            Repo::open_in_memory().expect("open in-memory repo"),
        ))
    }

    fn watch_cfg(path: &Path, recursive: bool) -> WatchConfig {
        WatchConfig {
            path: Some(path.to_string_lossy().into_owned()),
            recursive,
            extensions: Vec::new(),
            min_size_mb: 0,
            max_size_mb: 5000,
            stabilize_seconds: 3,
        }
    }

    fn fast_stabilize() -> StabilizeConfig {
        StabilizeConfig {
            stable_reads: 1,
            interval: Duration::from_millis(5),
            timeout: Duration::from_millis(500),
        }
    }

    // --- happy path: enqueue then no duplicates on rerun --------------------

    #[tokio::test]
    async fn rescan_enqueues_new_files_and_reruns_without_duplicates() {
        let _serial = serial_guard();
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.txt"), b"aaa").expect("write a");
        std::fs::write(dir.path().join("b.txt"), b"bbbb").expect("write b");
        std::fs::write(dir.path().join("c.txt"), b"ccccc").expect("write c");

        let repo = shared_repo();
        let wakers = Wakers::new();
        let watch = watch_cfg(dir.path(), false);
        let stabilize = fast_stabilize();

        let first = rescan(repo.clone(), &wakers, &watch, &stabilize)
            .await
            .expect("first rescan");
        assert_eq!(first.scanned, 3);
        assert_eq!(first.enqueued, 3);
        assert_eq!(first.unchanged, 0);
        assert_eq!(first.skipped_filtered, 0);
        assert_eq!(first.errors, 0);

        let counts = with_repo(repo.clone(), |repo| repo.status_counts())
            .await
            .expect("status_counts");
        assert_eq!(counts.pending, 6, "3 files * 2 destinations");

        // "App closed" between scans: rerun rescan directly, exactly as the
        // task's done-when criterion describes.
        let second = rescan(repo.clone(), &wakers, &watch, &stabilize)
            .await
            .expect("second rescan");
        assert_eq!(second.scanned, 3);
        assert_eq!(second.enqueued, 0, "no duplicates on rerun");
        assert_eq!(
            second.unchanged, 3,
            "all 3 files detected via the size+mtime fast path, no rehash"
        );
        assert_eq!(second.errors, 0);

        let counts_after = with_repo(repo.clone(), |repo| repo.status_counts())
            .await
            .expect("status_counts after rerun");
        assert_eq!(counts_after.pending, 6, "rerun must not create extra jobs");
    }

    // --- content change: rehash + reset ------------------------------------

    #[tokio::test]
    async fn rescan_rehashes_a_file_whose_content_and_mtime_changed() {
        let _serial = serial_guard();
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"hello").expect("initial write");

        let repo = shared_repo();
        let wakers = Wakers::new();
        let watch = watch_cfg(dir.path(), false);
        let stabilize = fast_stabilize();

        rescan(repo.clone(), &wakers, &watch, &stabilize)
            .await
            .expect("first rescan");

        // Advance the s3 job to `done` so the reset caused by the rehash is
        // observable (mirrors queue::intake's own rehash test).
        with_repo(repo.clone(), |repo| {
            let job = repo
                .claim_next(Destination::S3, "2026-01-01T00:00:00Z")?
                .expect("s3 job should be pending");
            repo.mark_done(&job.id, "remote-1", None)
        })
        .await
        .expect("advance s3 job to done");

        // Rewrite with different content (and therefore a different size,
        // and — modulo filesystem timestamp resolution — a bumped mtime).
        std::fs::write(&path, b"hello world, now much longer").expect("rewrite content");

        let second = rescan(repo.clone(), &wakers, &watch, &stabilize)
            .await
            .expect("second rescan");
        assert_eq!(second.enqueued, 1, "changed content re-enqueues the file");
        assert_eq!(second.unchanged, 0);
        assert_eq!(second.errors, 0);

        let counts = with_repo(repo.clone(), |repo| repo.status_counts())
            .await
            .expect("status_counts");
        assert_eq!(
            counts.pending, 2,
            "both jobs reset to pending by the rehash"
        );
        assert_eq!(counts.done, 0, "the previously-done job was reset too");
    }

    // --- filters -------------------------------------------------------------

    #[tokio::test]
    async fn rescan_skips_filtered_files() {
        let _serial = serial_guard();
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("ok.txt"), b"fits").expect("write ok file");
        std::fs::write(dir.path().join("locked.tmp"), b"ignored temp file").expect("write tmp");
        std::fs::write(dir.path().join("huge.bin"), vec![0u8; 2 * 1024 * 1024])
            .expect("write oversize file");

        let repo = shared_repo();
        let wakers = Wakers::new();
        let mut watch = watch_cfg(dir.path(), false);
        watch.max_size_mb = 1; // 1 MB cap: `huge.bin` (2 MB) exceeds it, `ok.txt` doesn't.
        let stabilize = fast_stabilize();

        let report = rescan(repo.clone(), &wakers, &watch, &stabilize)
            .await
            .expect("rescan");

        assert_eq!(report.scanned, 3);
        assert_eq!(
            report.skipped_filtered, 2,
            "the .tmp file and the oversize file"
        );
        assert_eq!(report.enqueued, 1, "only ok.txt passes the filters");
        assert_eq!(report.errors, 0);
    }

    // --- filter reconciliation (RF-004) ----------------------------------------

    /// How many jobs the dashboard would show — `list_jobs` hides archived
    /// rows, so this is the count the user actually perceives.
    async fn visible_jobs(repo: &Arc<Mutex<Repo>>) -> i64 {
        with_repo(repo.clone(), |repo| {
            repo.list_jobs(&crate::state::ListJobsQuery {
                statuses: None,
                destination: None,
                include_archived: false,
                limit: 100,
                offset: 0,
            })
        })
        .await
        .expect("list_jobs")
        .total
    }

    // The reported bug: tighten the size filter, hit "Atualizar Lista", and
    // nothing changed on screen because rescan only ever added.
    #[tokio::test]
    async fn rescan_archives_files_that_no_longer_pass_the_size_filter() {
        let _serial = serial_guard();
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("small.txt"), b"tiny").expect("write small");
        std::fs::write(dir.path().join("big.txt"), vec![0u8; 2 * 1024 * 1024]).expect("write big");

        let repo = shared_repo();
        let wakers = Wakers::new();
        let stabilize = fast_stabilize();
        let watch = watch_cfg(dir.path(), false);

        let first = rescan(repo.clone(), &wakers, &watch, &stabilize)
            .await
            .expect("first rescan");
        assert_eq!(first.enqueued, 2);
        assert_eq!(first.archived, 0);
        assert_eq!(visible_jobs(&repo).await, 2);

        let mut tightened = watch.clone();
        tightened.min_size_mb = 1; // `small.txt` no longer qualifies.

        let second = rescan(repo.clone(), &wakers, &tightened, &stabilize)
            .await
            .expect("second rescan");

        assert_eq!(second.archived, 1);
        assert_eq!(second.restored, 0);
        assert_eq!(visible_jobs(&repo).await, 1, "small.txt leaves the list");
    }

    #[tokio::test]
    async fn rescan_archives_files_excluded_by_the_extension_allowlist() {
        let _serial = serial_guard();
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("keep.pdf"), b"pdf").expect("write pdf");
        std::fs::write(dir.path().join("drop.txt"), b"txt").expect("write txt");

        let repo = shared_repo();
        let wakers = Wakers::new();
        let stabilize = fast_stabilize();
        let watch = watch_cfg(dir.path(), false);

        rescan(repo.clone(), &wakers, &watch, &stabilize)
            .await
            .expect("first rescan");
        assert_eq!(visible_jobs(&repo).await, 2);

        let mut only_pdf = watch.clone();
        only_pdf.extensions = vec!["pdf".to_string()];

        let report = rescan(repo.clone(), &wakers, &only_pdf, &stabilize)
            .await
            .expect("second rescan");

        assert_eq!(report.archived, 1);
        assert_eq!(visible_jobs(&repo).await, 1);
    }

    // Widening a filter must be reversible. `rescan`'s size+mtime fast path
    // returns `Unchanged` for a file that did not move on disk, so only the
    // restore half of the reconciliation can bring it back.
    #[tokio::test]
    async fn rescan_restores_files_when_the_filter_widens_again() {
        let _serial = serial_guard();
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("small.txt"), b"tiny").expect("write small");

        let repo = shared_repo();
        let wakers = Wakers::new();
        let stabilize = fast_stabilize();
        let watch = watch_cfg(dir.path(), false);

        rescan(repo.clone(), &wakers, &watch, &stabilize)
            .await
            .expect("first rescan");

        let mut tightened = watch.clone();
        tightened.min_size_mb = 1;
        let tight = rescan(repo.clone(), &wakers, &tightened, &stabilize)
            .await
            .expect("tightening rescan");
        assert_eq!(tight.archived, 1);
        assert_eq!(visible_jobs(&repo).await, 0);

        let widened = rescan(repo.clone(), &wakers, &watch, &stabilize)
            .await
            .expect("widening rescan");

        assert_eq!(widened.restored, 1);
        assert_eq!(widened.enqueued, 0, "the file did not change on disk");
        assert_eq!(visible_jobs(&repo).await, 1, "and it comes back");
    }

    #[tokio::test]
    async fn rescan_archives_files_that_are_gone_from_disk() {
        let _serial = serial_guard();
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("doomed.txt");
        std::fs::write(&path, b"here for now").expect("write");

        let repo = shared_repo();
        let wakers = Wakers::new();
        let stabilize = fast_stabilize();
        let watch = watch_cfg(dir.path(), false);

        rescan(repo.clone(), &wakers, &watch, &stabilize)
            .await
            .expect("first rescan");
        assert_eq!(visible_jobs(&repo).await, 1);

        std::fs::remove_file(&path).expect("delete the file");

        let report = rescan(repo.clone(), &wakers, &watch, &stabilize)
            .await
            .expect("second rescan");

        assert_eq!(report.archived, 1);
        assert_eq!(visible_jobs(&repo).await, 0);
    }

    // A file that dropped out of scope because `recursive` went true → false is
    // never walked again — so a sweep limited to what the walk found would
    // strand it in the queue forever.
    #[tokio::test]
    async fn rescan_reconciles_files_the_walk_can_no_longer_reach() {
        let _serial = serial_guard();
        let dir = tempdir().expect("tempdir");
        let nested = dir.path().join("sub");
        std::fs::create_dir(&nested).expect("mkdir");
        std::fs::write(nested.join("deep.txt"), b"nested").expect("write nested");

        let repo = shared_repo();
        let wakers = Wakers::new();
        let stabilize = fast_stabilize();

        let recursive = watch_cfg(dir.path(), true);
        rescan(repo.clone(), &wakers, &recursive, &stabilize)
            .await
            .expect("recursive rescan");
        assert_eq!(visible_jobs(&repo).await, 1);

        let flat = watch_cfg(dir.path(), false);
        let report = rescan(repo.clone(), &wakers, &flat, &stabilize)
            .await
            .expect("flat rescan");

        assert_eq!(report.archived, 1);
        assert_eq!(visible_jobs(&repo).await, 0);
    }

    // A transfer in flight is never interrupted by a filter change.
    #[tokio::test]
    async fn rescan_never_archives_an_uploading_job() {
        let _serial = serial_guard();
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("small.txt"), b"tiny").expect("write small");

        let repo = shared_repo();
        let wakers = Wakers::new();
        let stabilize = fast_stabilize();
        let watch = watch_cfg(dir.path(), false);

        rescan(repo.clone(), &wakers, &watch, &stabilize)
            .await
            .expect("first rescan");

        // Through the production path: `claim_next` is what a worker calls, and
        // it is the transition that makes a job in-flight.
        let claimed = with_repo(repo.clone(), |repo| {
            repo.claim_next(crate::state::Destination::S3, &Utc::now().to_rfc3339())
        })
        .await
        .expect("claim_next");
        assert!(claimed.is_some(), "a pending S3 job must be claimable");

        let mut tightened = watch.clone();
        tightened.min_size_mb = 1;
        rescan(repo.clone(), &wakers, &tightened, &stabilize)
            .await
            .expect("tightening rescan");

        // `status_counts` already excludes archived rows, so a surviving
        // `uploading` count is exactly "in flight and not swept".
        let counts = with_repo(repo.clone(), |repo| repo.status_counts())
            .await
            .expect("status_counts");

        assert_eq!(
            counts.uploading, 1,
            "the in-flight job must survive the sweep"
        );
    }

    // --- recursion -------------------------------------------------------------

    #[tokio::test]
    async fn rescan_recursion_follows_the_recursive_flag() {
        let _serial = serial_guard();
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("top.txt"), b"top-level").expect("write top-level file");
        std::fs::create_dir(dir.path().join("sub")).expect("create sub dir");
        std::fs::write(dir.path().join("sub").join("x.txt"), b"nested").expect("write nested file");

        let repo = shared_repo();
        let wakers = Wakers::new();
        let stabilize = fast_stabilize();

        let non_recursive_watch = watch_cfg(dir.path(), false);
        let non_recursive = rescan(repo.clone(), &wakers, &non_recursive_watch, &stabilize)
            .await
            .expect("non-recursive rescan");
        assert_eq!(non_recursive.scanned, 1, "sub/x.txt is not visited");
        assert_eq!(non_recursive.enqueued, 1);

        let recursive_watch = watch_cfg(dir.path(), true);
        let recursive = rescan(repo.clone(), &wakers, &recursive_watch, &stabilize)
            .await
            .expect("recursive rescan");
        assert_eq!(
            recursive.scanned, 2,
            "top.txt (already known) + sub/x.txt (new)"
        );
        assert_eq!(recursive.enqueued, 1, "only sub/x.txt is new");
        assert_eq!(
            recursive.unchanged, 1,
            "top.txt already known from the first (non-recursive) pass"
        );
    }

    // --- no path configured -----------------------------------------------

    #[tokio::test]
    async fn rescan_without_a_configured_path_errors() {
        let _serial = serial_guard();
        let repo = shared_repo();
        let wakers = Wakers::new();
        let watch = WatchConfig {
            path: None,
            ..watch_cfg(Path::new("unused"), false)
        };
        let stabilize = fast_stabilize();

        let err = rescan(repo, &wakers, &watch, &stabilize)
            .await
            .expect_err("missing watch.path should error");
        assert!(matches!(err, RescanError::NoPath));
    }
    /// Regression (Fase 2 acceptance): the watcher reports canonical paths
    /// (`/private/tmp/...` on macOS) while `watch.path` may be the raw
    /// user-supplied form (`/tmp/...`). Both must map to ONE `files` row.
    #[tokio::test]
    async fn rescan_with_non_canonical_root_does_not_duplicate_watcher_intakes() {
        let _serial = serial_guard();
        let dir = tempfile::tempdir().unwrap();
        let raw_root = dir.path().to_path_buf();
        let canonical_root = crate::paths::canonicalize_clean_sync(&raw_root).unwrap();
        if canonical_root == raw_root {
            // Nothing to prove on filesystems without symlinked temp dirs.
            return;
        }
        let file = canonical_root.join("a.txt");
        std::fs::write(&file, b"hello").unwrap();
        let repo = Arc::new(Mutex::new(Repo::open_in_memory().unwrap()));
        let wakers = Wakers::default();
        let meta = std::fs::metadata(&file).unwrap();
        let mtime: chrono::DateTime<chrono::Utc> = meta.modified().unwrap().into();
        // 1) watcher-style intake with the canonical path
        crate::queue::intake(
            repo.clone(),
            &wakers,
            &canonical_root,
            file.clone(),
            meta.len(),
            mtime,
        )
        .await
        .unwrap();
        // 2) rescan with the raw (non-canonical) root
        let watch = WatchConfig {
            path: Some(raw_root.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let report = rescan(repo.clone(), &wakers, &watch, &fast_stabilize())
            .await
            .unwrap();
        assert_eq!(report.enqueued, 0, "raw-root rescan must not enqueue again");
        assert_eq!(report.unchanged, 1);
        let counts = repo.lock().unwrap().status_counts().unwrap();
        assert_eq!(counts.pending, 2, "still exactly one file = two jobs");
    }

    // --- symlink containment (VULN-003) -------------------------------------

    /// A symlinked file and a symlinked directory inside the watched root
    /// must both be skipped (never followed, never hashed/enqueued) and
    /// counted in `skipped_symlink` -- only the one real file is enqueued.
    #[cfg(unix)]
    #[tokio::test]
    async fn rescan_skips_symlinked_file_and_symlinked_dir_and_counts_them() {
        let _serial = serial_guard();
        use std::os::unix::fs::symlink;

        let root = tempdir().expect("root tempdir");
        let outside = tempdir().expect("outside tempdir");

        // Real file, must be enqueued normally.
        std::fs::write(root.path().join("a.txt"), b"real file").expect("write a.txt");

        // Symlink to a file outside root -- must be skipped, not followed.
        std::fs::write(outside.path().join("secret.txt"), b"outside secret")
            .expect("write outside file");
        symlink(
            outside.path().join("secret.txt"),
            root.path().join("link_to_secret.txt"),
        )
        .expect("create symlinked file");

        // Symlink to a directory outside root -- must be skipped, never
        // descended into (even though it contains a matching-extension file).
        std::fs::write(outside.path().join("inner.txt"), b"outside dir file")
            .expect("write outside dir file");
        symlink(outside.path(), root.path().join("link_to_dir")).expect("create symlinked dir");

        let repo = shared_repo();
        let wakers = Wakers::default();
        let watch = watch_cfg(root.path(), true);
        let stabilize = fast_stabilize();

        let report = rescan(repo.clone(), &wakers, &watch, &stabilize)
            .await
            .expect("rescan with symlinks present");

        assert_eq!(report.scanned, 1, "only a.txt is a real, non-symlink entry");
        assert_eq!(report.enqueued, 1, "only a.txt is enqueued");
        assert_eq!(
            report.skipped_symlink, 2,
            "the symlinked file and the symlinked dir are both counted"
        );

        let counts = repo.lock().unwrap().status_counts().unwrap();
        assert_eq!(counts.pending, 2, "exactly one file = two jobs");
    }

    // --- fault tolerance (T-1.1: `System Volume Information`-style errors) --

    /// An unreadable subdirectory (permission denied, e.g. Windows'
    /// `System Volume Information`) must not abort the rescan: files outside
    /// it are still enqueued and the unreadable one is counted, not treated
    /// as an error that stops the whole walk.
    ///
    /// Named something other than a known system path on purpose: the point
    /// here is `walk_dir`'s per-entry error tolerance on `read_dir`, a
    /// distinct code path from the `is_system_path` short-circuit (which is
    /// unit-tested directly in `paths.rs` and would otherwise skip this
    /// directory before ever calling `read_dir` on it).
    #[cfg(unix)]
    #[tokio::test]
    async fn rescan_skips_unreadable_subdir_and_still_enqueues_the_rest() {
        let _serial = serial_guard();
        use std::os::unix::fs::PermissionsExt;

        let root = tempdir().expect("root tempdir");
        std::fs::write(root.path().join("ok.txt"), b"readable file").expect("write ok.txt");

        let locked = root.path().join("dados_privados");
        std::fs::create_dir(&locked).expect("create locked dir");
        std::fs::write(locked.join("hidden.txt"), b"never seen").expect("write hidden.txt");
        // Deny all access, the way NTFS denies everyone but SYSTEM on the
        // real `System Volume Information`.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000))
            .expect("lock down permissions");

        let repo = shared_repo();
        let wakers = Wakers::default();
        let watch = watch_cfg(root.path(), true);
        let stabilize = fast_stabilize();

        let result = rescan(repo.clone(), &wakers, &watch, &stabilize).await;

        // Restore permissions unconditionally so the tempdir can be dropped,
        // even if an assertion below fails.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755))
            .expect("restore permissions");

        let report = result.expect("rescan must tolerate an unreadable subdirectory");
        assert_eq!(
            report.enqueued, 1,
            "ok.txt outside the locked dir is enqueued"
        );
        assert!(
            report.skipped_unreadable >= 1,
            "the locked subdirectory must be counted as skipped-unreadable"
        );

        let counts = repo.lock().unwrap().status_counts().unwrap();
        assert_eq!(counts.pending, 2, "exactly one visible file = two jobs");
    }

    /// The ROOT itself being unreadable/missing is a genuine failure, not a
    /// per-entry one: it must still surface as `RescanError::Io` rather than
    /// being swallowed like a descendant's error would be.
    #[tokio::test]
    async fn rescan_with_missing_root_still_returns_io_error() {
        let _serial = serial_guard();
        let repo = shared_repo();
        let wakers = Wakers::default();
        let missing_root = Path::new("/definitely/does/not/exist/at/all/rescan-root-test");
        let watch = watch_cfg(missing_root, true);
        let stabilize = fast_stabilize();

        let err = rescan(repo, &wakers, &watch, &stabilize)
            .await
            .expect_err("a nonexistent root must not silently report zero files");

        assert!(
            matches!(err, RescanError::Io(_)),
            "expected RescanError::Io, got {err:?}"
        );
    }

    // --- T-2.4: rescan_with_progress ----------------------------------------

    #[tokio::test]
    async fn rescan_with_progress_emits_throttled_progress_coherent_with_the_report() {
        let _serial = serial_guard();
        let dir = tempdir().expect("tempdir");
        // More candidates than one CHUNK_SIZE (4) so the loop crosses several
        // chunk boundaries -- without the 500ms throttle this would produce
        // more than one sink call per chunk boundary crossed.
        for i in 0..10 {
            std::fs::write(
                dir.path().join(format!("file-{i}.txt")),
                format!("contents {i}"),
            )
            .expect("write file");
        }

        let repo = shared_repo();
        let wakers = Wakers::new();
        let watch = watch_cfg(dir.path(), false);
        let stabilize = fast_stabilize();

        let calls: Arc<Mutex<Vec<(u32, u32)>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_calls = calls.clone();
        let sink: RescanProgressSink = Arc::new(move |processed, total| {
            sink_calls.lock().unwrap().push((processed, total));
        });

        let report = rescan_with_progress(repo.clone(), &wakers, &watch, &stabilize, &sink)
            .await
            .expect("rescan_with_progress");

        assert_eq!(report.scanned, 10);
        assert_eq!(report.enqueued, 10);

        let recorded = calls.lock().unwrap().clone();
        assert!(!recorded.is_empty(), "sink must be called at least once");
        assert!(
            recorded.len() < 10,
            "500ms throttle must coalesce well below one call per file, got {recorded:?}"
        );

        let (last_processed, last_total) = *recorded.last().expect("at least one call");
        assert_eq!(
            last_processed, last_total,
            "final call must report processed == total"
        );
        assert_eq!(
            last_total,
            report.enqueued + report.unchanged + report.errors,
            "progress total must be coherent with what the report actually processed"
        );

        for &(processed, total) in &recorded {
            assert_eq!(total, last_total, "total must stay constant across calls");
            assert!(processed <= total, "processed must never exceed total");
        }
    }

    #[tokio::test]
    async fn rescan_with_progress_calls_the_sink_even_for_a_single_small_batch() {
        let _serial = serial_guard();
        // A folder smaller than one CHUNK_SIZE must still get the final
        // "done" callback -- the throttle's is_final branch, not just its
        // time-based one, has to fire.
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("only.txt"), b"just one file").expect("write file");

        let repo = shared_repo();
        let wakers = Wakers::new();
        let watch = watch_cfg(dir.path(), false);
        let stabilize = fast_stabilize();

        let calls: Arc<Mutex<Vec<(u32, u32)>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_calls = calls.clone();
        let sink: RescanProgressSink = Arc::new(move |processed, total| {
            sink_calls.lock().unwrap().push((processed, total));
        });

        rescan_with_progress(repo, &wakers, &watch, &stabilize, &sink)
            .await
            .expect("rescan_with_progress");

        let recorded = calls.lock().unwrap().clone();
        assert_eq!(recorded, vec![(1, 1)]);
    }

    // --- T-2.5: single-flight rescan guard ----------------------------------

    /// A slower-than-usual `StabilizeConfig` -- `wait_until_stable` needs
    /// several *stable* reads spaced `interval` apart, so this keeps a scan
    /// genuinely in flight for a predictable stretch, the same trick
    /// `runtime.rs`'s own tests use to simulate a slow scan without writing
    /// gigabytes of test data.
    fn slow_stabilize() -> StabilizeConfig {
        StabilizeConfig {
            stable_reads: 6,
            interval: Duration::from_millis(50),
            timeout: Duration::from_secs(5),
        }
    }

    #[tokio::test]
    async fn a_second_concurrent_rescan_is_skipped_and_does_not_wait_for_the_first() {
        let _serial = serial_guard();
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.txt"), b"aaa").expect("write a");

        let repo = shared_repo();
        let wakers = Wakers::new();
        let watch = watch_cfg(dir.path(), false);
        let slow = slow_stabilize();
        let fast = fast_stabilize();

        let first = rescan(repo.clone(), &wakers, &watch, &slow);
        let second = async {
            // Cede control once so the runtime has a chance to poll `first`
            // up through its (synchronous) guard acquisition and into its
            // first real `.await` before this one ever calls `rescan()`.
            // Not load-bearing for correctness -- guard acquisition happens
            // before any `.await` in `rescan_inner`, so `first` would hold
            // it after its very first poll regardless -- just documents the
            // ordering this test relies on.
            tokio::task::yield_now().await;
            let started = TokioInstant::now();
            let result = rescan(repo.clone(), &wakers, &watch, &fast).await;
            (result, started.elapsed())
        };

        let (first_result, (second_result, second_elapsed)) = tokio::join!(first, second);

        assert!(
            first_result.is_ok(),
            "the first rescan should complete normally, got {first_result:?}"
        );
        assert!(
            matches!(second_result, Err(RescanError::AlreadyInProgress)),
            "the second concurrent rescan must report AlreadyInProgress, got {second_result:?}"
        );
        assert!(
            second_elapsed < Duration::from_millis(50),
            "the second rescan must return immediately rather than queuing behind \
             the first (which needs several 50ms-spaced stable reads to finish), \
             took {second_elapsed:?}"
        );
    }

    #[tokio::test]
    async fn guard_is_released_after_a_successful_scan_so_a_later_scan_runs_normally() {
        let _serial = serial_guard();
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.txt"), b"aaa").expect("write a");

        let repo = shared_repo();
        let wakers = Wakers::new();
        let watch = watch_cfg(dir.path(), false);
        let stabilize = fast_stabilize();

        rescan(repo.clone(), &wakers, &watch, &stabilize)
            .await
            .expect("first rescan should succeed");

        let second = rescan(repo, &wakers, &watch, &stabilize).await;
        assert!(
            matches!(second, Ok(_)),
            "the guard must be released after a successful scan, got {second:?}"
        );
    }

    #[tokio::test]
    async fn guard_is_released_after_an_error_return() {
        let _serial = serial_guard();
        let repo = shared_repo();
        let wakers = Wakers::new();
        let stabilize = fast_stabilize();

        // A nonexistent root fails past the guard acquisition (`walk_dir`'s
        // `?`), exercising the `RescanError::Io` early-return path.
        let missing_root = Path::new("/definitely/does/not/exist/at/all/rescan-guard-release");
        let missing_watch = watch_cfg(missing_root, true);
        let err = rescan(repo.clone(), &wakers, &missing_watch, &stabilize)
            .await
            .expect_err("a nonexistent root must error");
        assert!(matches!(err, RescanError::Io(_)));

        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.txt"), b"aaa").expect("write a");
        let watch = watch_cfg(dir.path(), false);
        let after = rescan(repo, &wakers, &watch, &stabilize).await;
        assert!(
            matches!(after, Ok(_)),
            "the guard must be released after an Err return, got {after:?}"
        );
    }

    #[tokio::test]
    async fn guard_is_released_when_the_in_flight_future_is_dropped_mid_scan() {
        let _serial = serial_guard();
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.txt"), b"aaa").expect("write a");

        let repo = shared_repo();
        let wakers = Wakers::new();
        let watch = watch_cfg(dir.path(), false);
        let slow = slow_stabilize();

        // Mirrors exactly what `run_reconcile_loop`'s outer `select!` does on
        // shutdown: race the scan against something else and drop it, still
        // in flight, when that something else wins. `tokio::time::timeout`
        // drops its inner future the moment it fires if that future hasn't
        // resolved yet.
        let timed_out = tokio::time::timeout(
            Duration::from_millis(20),
            rescan(repo.clone(), &wakers, &watch, &slow),
        )
        .await;
        assert!(
            timed_out.is_err(),
            "the scan must still be in flight when the timeout fires (test setup issue if not)"
        );

        // The timed-out future -- and the `RescanGuard` held inside it -- has
        // now been dropped. A fresh rescan must be able to acquire the guard.
        let fast = fast_stabilize();
        let after = rescan(repo, &wakers, &watch, &fast).await;
        assert!(
            matches!(after, Ok(_)),
            "the guard must be released when the in-flight future is dropped, got {after:?}"
        );
    }
}
