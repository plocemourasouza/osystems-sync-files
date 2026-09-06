//! `core::stabilize` — waits for a file to stop changing before the pipeline hashes and
//! enqueues it (PLAN.md T-2.2, SPEC.md §2 "Fluxo de um arquivo", §6 `wait_until_stable`).
//!
//! Debouncing filesystem events (2 s) is `watcher`'s job, not this module's. `stabilize` picks
//! up *after* the debounce: it treats a file as stable once its size has been identical across
//! [`StabilizeConfig::stable_reads`] consecutive polls **and** an exclusive open succeeds (the
//! writer, if any, has closed its handle). This two-part check exists because size-only
//! stability is not enough: many editors and downloaders write in bursts with quiet gaps,
//! and antivirus / cloud-sync tools can hold a sharing lock on a file whose size already looks
//! final (PRD.md §8 risk "antivírus corporativo trava arquivo").
//!
//! Exclusive-open detection is a Windows-only guarantee. `OpenOptions::share_mode(0)` asks the
//! OS to fail the open if *any* other handle (read, write, or delete) is currently open on the
//! file — that is how we detect "some other process still has this file open" without needing
//! a crate beyond `std`. On macOS and Linux there is no equivalent sharing-lock concept in the
//! POSIX open() semantics: a plain `File::open` almost always succeeds even while another
//! process is actively writing to the same file (advisory locks like `flock` only matter if the
//! writer opts into them, which most producers of these files do not). So on non-Windows
//! platforms this module still performs the size-stability check faithfully, but the "exclusive
//! open" half of the guarantee degrades to "the path exists and is readable" — it cannot catch a
//! concurrent writer. This is a documented, accepted gap (the project ships Windows-first; see
//! SPEC.md §2 and PRD.md RF-002/RF-006).

use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};

/// Tuning knobs for [`wait_until_stable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StabilizeConfig {
    /// Number of consecutive equal-size polls required before the file is considered stable.
    pub stable_reads: u32,
    /// Delay between polls.
    pub interval: Duration,
    /// Maximum time to wait before giving up and proceeding anyway (logged as a warning).
    pub timeout: Duration,
}

impl Default for StabilizeConfig {
    fn default() -> Self {
        Self {
            stable_reads: 3,
            interval: Duration::from_secs(1),
            timeout: Duration::from_secs(30 * 60),
        }
    }
}

impl StabilizeConfig {
    /// The config the user actually asked for in Settings.
    ///
    /// `watch.stabilize_seconds` is labelled "seconds of stabilization", and
    /// with a one-second [`interval`](Self::interval) that is exactly
    /// [`stable_reads`](Self::stable_reads): N consecutive equal-size polls,
    /// one second apart, is N seconds of quiet. The default `3` therefore
    /// maps onto [`StabilizeConfig::default`] unchanged.
    ///
    /// Before this existed the field was validated, persisted, shown in the
    /// UI and restarted the watcher — while `SyncRuntime` used
    /// `StabilizeConfig::default()` regardless, so changing it did nothing.
    ///
    /// `0` is rejected by `config::validate`, but is clamped to `1` here
    /// anyway: `wait_until_stable` with `stable_reads: 0` would treat every
    /// file as instantly stable, which is the one outcome this module exists
    /// to prevent.
    pub fn from_watch(watch: &crate::config::WatchConfig) -> Self {
        Self {
            stable_reads: watch.stabilize_seconds.max(1),
            ..Self::default()
        }
    }
}

/// Outcome of a successful [`wait_until_stable`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StableFile {
    /// File size in bytes at the moment stability was confirmed (or last observed, if forced).
    pub size: u64,
    /// Last-modified time at the moment stability was confirmed (or last observed, if forced).
    pub mtime: DateTime<Utc>,
    /// `true` if [`StabilizeConfig::timeout`] elapsed before the file actually stabilized —
    /// the caller proceeded anyway per PLAN.md T-2.2 ("timeout de 30 min loga `warn` e segue
    /// mesmo assim").
    pub forced: bool,
}

/// Errors that can occur while waiting for a file to stabilize.
#[derive(Debug, thiserror::Error)]
pub enum StabilizeError {
    /// The file disappeared (was deleted/moved away) while we were waiting on it.
    #[error("file vanished while waiting for it to stabilize: {0}")]
    Vanished(std::path::PathBuf),
    /// Reading metadata failed for a reason other than the file being gone.
    #[error("failed to read metadata for {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Waits until `path` stops changing and is not held open exclusively by another process.
///
/// Algorithm (SPEC.md §2, §6):
/// 1. Every [`StabilizeConfig::interval`], read `metadata().len()`.
///    - Missing file → [`StabilizeError::Vanished`].
///    - Size equal to the previous read → bump a consecutive-equal counter; size changed →
///      reset the counter to 1 (this read itself counts as the first of a new streak).
/// 2. Once the counter reaches [`StabilizeConfig::stable_reads`], attempt an exclusive open
///    ([`try_open_exclusive`]). Success → return `Ok(StableFile { forced: false, .. })`.
///    A sharing violation resets the counter and the wait continues (a writer — or an
///    antivirus scanner — still has the file open).
/// 3. If [`StabilizeConfig::timeout`] elapses before step 2 succeeds, log a `tracing::warn!`
///    and return `Ok(StableFile { forced: true, .. })` with the last observed size/mtime —
///    the pipeline proceeds anyway rather than stalling forever.
pub async fn wait_until_stable(
    path: &Path,
    cfg: &StabilizeConfig,
) -> Result<StableFile, StabilizeError> {
    let start = tokio::time::Instant::now();
    let mut previous_len: Option<u64> = None;
    let mut consecutive_equal: u32 = 0;

    loop {
        let metadata = read_metadata(path)?;
        let len = metadata.len();

        consecutive_equal = match previous_len {
            Some(prev) if prev == len => consecutive_equal + 1,
            _ => 1,
        };
        previous_len = Some(len);

        if consecutive_equal >= cfg.stable_reads {
            match try_open_exclusive(path) {
                Ok(()) => {
                    return Ok(StableFile {
                        size: metadata.len(),
                        mtime: mtime_of(&metadata),
                        forced: false,
                    });
                }
                Err(_sharing_violation) => {
                    // Someone (writer, AV scanner) still has the file open. Reset and keep
                    // waiting — the size being unchanged is not enough on its own.
                    consecutive_equal = 0;
                }
            }
        }

        if start.elapsed() >= cfg.timeout {
            tracing::warn!(
                path = %path.display(),
                timeout_secs = cfg.timeout.as_secs(),
                "wait_until_stable: tempo esgotado, prosseguindo mesmo assim",
            );
            return Ok(StableFile {
                size: metadata.len(),
                mtime: mtime_of(&metadata),
                forced: true,
            });
        }

        tokio::time::sleep(cfg.interval).await;
    }
}

fn read_metadata(path: &Path) -> Result<std::fs::Metadata, StabilizeError> {
    std::fs::metadata(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            StabilizeError::Vanished(path.to_path_buf())
        } else {
            StabilizeError::Io {
                path: path.to_path_buf(),
                source,
            }
        }
    })
}

fn mtime_of(metadata: &std::fs::Metadata) -> DateTime<Utc> {
    metadata
        .modified()
        .ok()
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(Utc::now)
}

/// Attempts to open `path` in a mode that fails if any other handle is currently open on it.
///
/// Windows: `share_mode(0)` denies the open while *any* other process holds a handle (read,
/// write, or delete) on the file — this is the actual "is someone still writing to this?"
/// check.
///
/// Non-Windows: there is no POSIX equivalent of a sharing lock, so this degrades to a plain
/// `File::open` — it only proves the path is readable, not that no writer is attached. See the
/// module-level docs for why this gap is accepted.
fn try_open_exclusive(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::fs::OpenOptions;
        use std::os::windows::fs::OpenOptionsExt;

        OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(path)
            .map(|_file| ())
    }

    #[cfg(not(windows))]
    {
        std::fs::File::open(path).map(|_file| ())
    }
}

/// Returns whether `path` should be ignored by the watcher/stabilize pipeline entirely
/// (SPEC.md §2, §6): editor lock files (`~$*`), hidden/dotfiles, known temp-file extensions,
/// and directories. Metadata is not consulted here — see [`should_ignore_meta`] when directory
/// detection or the Windows hidden attribute must be checked too.
pub fn should_ignore(path: &Path) -> bool {
    should_ignore_meta(path, None)
}

/// Same as [`should_ignore`], but also considers filesystem metadata when available: directory
/// status and, on Windows, the hidden file attribute (`FILE_ATTRIBUTE_HIDDEN = 0x2`). Passing
/// `None` for `metadata` skips those two checks (useful when the caller only has a path, e.g. a
/// `notify` event for a file that may have already vanished).
pub fn should_ignore_meta(path: &Path, metadata: Option<&std::fs::Metadata>) -> bool {
    if let Some(metadata) = metadata {
        if metadata.is_dir() {
            return true;
        }
        if is_windows_hidden(metadata) {
            return true;
        }
    }

    let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
        // Non-UTF-8 or missing name: fail closed and process it rather than silently dropping
        // a file we can't classify.
        return false;
    };

    if file_name.starts_with("~$") || file_name.starts_with('.') {
        return true;
    }

    const IGNORED_EXTENSIONS: &[&str] = &["tmp", "crdownload", "part", "partial", "download"];
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => IGNORED_EXTENSIONS
            .iter()
            .any(|ignored| ext.eq_ignore_ascii_case(ignored)),
        None => false,
    }
}

#[cfg(windows)]
fn is_windows_hidden(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
}

#[cfg(not(windows))]
fn is_windows_hidden(_metadata: &std::fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration as StdDuration;

    use tempfile::tempdir;

    // NOTE (deviation from the task's ask): the task asked for `tokio::time::pause()` +
    // `advance()` for deterministic virtual-clock tests. That API is gated behind tokio's
    // `test-util` feature, which is not enabled in this crate's `Cargo.toml` — and per this
    // task's scope, `Cargo.toml` must not be touched (other agents own it / it's shared
    // dependency wiring). So these tests drive `wait_until_stable` against short *real*
    // intervals/timeouts with generous margins between the "must not stabilize yet" and "must
    // have stabilized by now" windows, which keeps them deterministic without the extra
    // dependency feature.

    #[tokio::test]
    async fn stabilizes_only_after_the_last_write_chunk() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("growing.bin");
        std::fs::write(&path, b"aaaa").expect("initial chunk");

        // interval=40ms, stable_reads=3 => needs ~120ms of unchanged size to pass the size
        // check. Chunks land 80ms apart, i.e. before 3 consecutive equal reads can land, so the
        // counter keeps resetting until the final chunk is written.
        let cfg = StabilizeConfig {
            stable_reads: 3,
            interval: Duration::from_millis(40),
            timeout: Duration::from_secs(10),
        };

        let wait = tokio::spawn({
            let path = path.clone();
            async move { wait_until_stable(&path, &cfg).await }
        });

        tokio::time::sleep(Duration::from_millis(80)).await;
        std::fs::write(&path, b"aaaabbbb").expect("second chunk");

        tokio::time::sleep(Duration::from_millis(80)).await;
        std::fs::write(&path, b"aaaabbbbcccc").expect("third (final) chunk");

        let result = tokio::time::timeout(StdDuration::from_secs(2), wait)
            .await
            .expect("wait_until_stable should finish well within 2s")
            .expect("task should not panic")
            .expect("should stabilize, not error");

        assert_eq!(result.size, 12); // "aaaabbbbcccc".len()
        assert!(!result.forced);
    }

    #[tokio::test]
    async fn never_stops_growing_hits_timeout_and_forces() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("forever.bin");
        std::fs::write(&path, b"a").expect("initial write");

        // Timeout (150ms) is much shorter than how long the background writer keeps mutating
        // the file (500ms), so `wait_until_stable` must give up and force-return.
        let cfg = StabilizeConfig {
            stable_reads: 3,
            interval: Duration::from_millis(30),
            timeout: Duration::from_millis(150),
        };

        let writer_path = path.clone();
        let writer = std::thread::spawn(move || {
            for i in 0..25u8 {
                std::thread::sleep(StdDuration::from_millis(20));
                let mut file = std::fs::OpenOptions::new()
                    .append(true)
                    .open(&writer_path)
                    .expect("reopen for append");
                file.write_all(&[b'a' + (i % 26)]).expect("append byte");
                file.flush().expect("flush");
            }
        });

        let result =
            tokio::time::timeout(StdDuration::from_secs(2), wait_until_stable(&path, &cfg))
                .await
                .expect("wait_until_stable should finish well within 2s")
                .expect("timeout path returns Ok, not Err");

        assert!(result.forced);
        writer.join().expect("writer thread should not panic");
    }

    #[tokio::test]
    async fn vanished_file_is_reported_mid_wait() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("disappearing.bin");
        std::fs::write(&path, b"hello").expect("initial write");

        // interval=30ms, stable_reads=3 => stability would take ~90ms; delete the file at 45ms,
        // well before that, so the deletion is what ends the wait.
        let cfg = StabilizeConfig {
            stable_reads: 3,
            interval: Duration::from_millis(30),
            timeout: Duration::from_secs(10),
        };

        let wait = tokio::spawn({
            let path = path.clone();
            async move { wait_until_stable(&path, &cfg).await }
        });

        tokio::time::sleep(Duration::from_millis(45)).await;
        std::fs::remove_file(&path).expect("remove fixture file");

        let result = tokio::time::timeout(StdDuration::from_secs(2), wait)
            .await
            .expect("wait_until_stable should finish well within 2s")
            .expect("task should not panic");

        match result {
            Err(StabilizeError::Vanished(reported_path)) => assert_eq!(reported_path, path),
            other => panic!("expected Vanished, got {other:?}"),
        }
    }

    #[test]
    fn should_ignore_table() {
        let cases: &[(&str, bool)] = &[
            ("~$document.docx", true),
            (".hidden-file", true),
            ("download.tmp", true),
            ("video.crdownload", true),
            ("archive.part", true),
            ("archive.partial", true),
            ("file.download", true),
            ("FILE.TMP", true), // extension match is case-insensitive
            ("report.pdf", false),
            ("notes.txt", false),
            ("archive.tar.gz", false),
            ("no-extension-file", false),
        ];

        for (name, expected) in cases {
            let path = Path::new(name);
            assert_eq!(
                should_ignore(path),
                *expected,
                "should_ignore({name:?}) expected {expected}",
            );
        }
    }

    #[test]
    fn should_ignore_meta_flags_directories() {
        let dir = tempdir().expect("tempdir");
        let metadata = std::fs::metadata(dir.path()).expect("read dir metadata");
        assert!(should_ignore_meta(dir.path(), Some(&metadata)));
    }

    // Windows-specific behaviors (share_mode sharing violations, the hidden attribute) require
    // real Win32 semantics that a non-Windows CI runner cannot exercise. They only compile and
    // run under `#[cfg(windows)]`, so on this (non-Windows) machine they are skipped entirely —
    // noted per the task's ask, not a coverage gap we're hiding.
    #[cfg(windows)]
    #[tokio::test]
    async fn windows_exclusive_open_blocks_on_open_writer() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("locked.bin");
        std::fs::write(&path, b"data").expect("initial write");

        // Hold the file open with a sharing mode that denies others, simulating a writer (or
        // antivirus scanner) still attached to it.
        let _held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .expect("hold exclusive handle");

        assert!(try_open_exclusive(&path).is_err());
    }

    // `watch.stabilize_seconds` was dead config: validated, persisted, shown
    // in Settings, and ignored at runtime. These pin the mapping that makes
    // it mean something.
    #[test]
    fn from_watch_maps_stabilize_seconds_onto_stable_reads() {
        let watch = crate::config::WatchConfig {
            stabilize_seconds: 10,
            ..Default::default()
        };

        let cfg = StabilizeConfig::from_watch(&watch);

        assert_eq!(cfg.stable_reads, 10);
        // One second per read is what makes "N reads" == "N seconds".
        assert_eq!(cfg.interval, Duration::from_secs(1));
        assert_eq!(cfg.timeout, StabilizeConfig::default().timeout);
    }

    #[test]
    fn from_watch_preserves_the_default_for_the_default_watch_config() {
        let watch = crate::config::WatchConfig::default();

        assert_eq!(
            StabilizeConfig::from_watch(&watch),
            StabilizeConfig::default()
        );
    }

    // `wait_until_stable` with `stable_reads: 0` would call every file
    // instantly stable — the exact failure this module exists to prevent.
    // `validate` rejects 0, but defence in depth is cheap here.
    #[test]
    fn from_watch_clamps_zero_to_one_read() {
        let watch = crate::config::WatchConfig {
            stabilize_seconds: 0,
            ..Default::default()
        };

        assert_eq!(StabilizeConfig::from_watch(&watch).stable_reads, 1);
    }

    #[cfg(windows)]
    #[test]
    fn windows_hidden_attribute_is_ignored() {
        use std::os::windows::fs::MetadataExt;
        use std::process::Command;

        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("hidden.bin");
        std::fs::write(&path, b"data").expect("initial write");
        Command::new("attrib")
            .args(["+h", path.to_str().unwrap()])
            .status()
            .expect("run attrib");

        let metadata = std::fs::metadata(&path).expect("read metadata");
        assert!(metadata.file_attributes() & 0x2 != 0);
        assert!(should_ignore_meta(&path, Some(&metadata)));
    }
}
