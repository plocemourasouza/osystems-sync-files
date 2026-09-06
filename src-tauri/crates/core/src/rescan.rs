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

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use serde::Serialize;
use ts_rs::TS;

use crate::config::WatchConfig;
use crate::queue::{intake, passes_filters, with_repo, QueueError, Wakers};
use crate::stabilize::{should_ignore_meta, wait_until_stable, StabilizeConfig};
use crate::state::{Repo, UpsertOutcome};

/// Number of files processed concurrently during the hash/intake stage of a
/// [`rescan`] (see the module-level docs for why this is manual chunking
/// rather than a `Semaphore` + spawned tasks).
const CHUNK_SIZE: usize = 4;

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
}

/// Walks `watch.path` honoring `watch.recursive`, skipping filters, and
/// enqueueing every file that is new or whose content changed since the
/// last scan (SPEC.md §6). See the module docs for the fast-path/slow-path
/// split and the bounded concurrency used for the slow path.
pub async fn rescan(
    repo: Arc<Mutex<Repo>>,
    wakers: &Wakers,
    watch: &WatchConfig,
    stabilize: &StabilizeConfig,
) -> Result<RescanReport, RescanError> {
    let root = watch.path.as_deref().ok_or(RescanError::NoPath)?;
    // Same canonical form the watcher reports (see `queue::intake`), so the
    // `file_by_path` short-circuit below hits instead of re-hashing every run.
    let root_buf = crate::paths::canonicalize_clean(std::path::Path::new(root))
        .await
        .unwrap_or_else(|_| std::path::PathBuf::from(root));
    let root = root_buf;

    let mut report = RescanReport::default();
    let (entries, skipped_symlink) = walk_dir(&root, watch.recursive).await?;
    report.skipped_symlink = skipped_symlink;

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

    for chunk in candidates.chunks(CHUNK_SIZE) {
        for outcome in run_chunk(&repo, wakers, &root, stabilize, chunk).await {
            match outcome {
                CandidateOutcome::Enqueued => report.enqueued += 1,
                CandidateOutcome::Unchanged => report.unchanged += 1,
                CandidateOutcome::Error => report.errors += 1,
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
/// way.
///
/// VULN-003: uses `symlink_metadata` (never follows the final component) so
/// a symlink is detected *before* deciding what to do with it — a symlinked
/// file or directory anywhere under `root` is skipped entirely (not
/// descended into, not hashed/enqueued) rather than followed, since it
/// could otherwise be used to read or enqueue a file outside the watched
/// folder. Each skip is logged via `tracing::warn!` and counted.
///
/// Manual `tokio::fs::read_dir` + an explicit stack, on purpose: this crate
/// does not depend on `walkdir`.
async fn walk_dir(
    root: &Path,
    recursive: bool,
) -> std::io::Result<(Vec<(PathBuf, std::fs::Metadata)>, u32)> {
    let mut files = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    let mut skipped_symlink: u32 = 0;

    while let Some(dir) = dirs.pop() {
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let metadata = tokio::fs::symlink_metadata(&path).await?;
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

    Ok((files, skipped_symlink))
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
}
