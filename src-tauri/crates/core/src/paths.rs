//! `core::paths` — Windows verbatim (`\\?\`) path normalization.
//!
//! `std::fs::canonicalize` on Windows returns the "verbatim" (a.k.a. extended-length,
//! a.k.a. UNC-prefixed) form of a path: `C:\monitoramento\file.bin` comes back as
//! `\\?\C:\monitoramento\file.bin`, and a network share comes back as
//! `\\?\UNC\server\share\file.bin`. That form is correct and necessary for paths
//! longer than `MAX_PATH`, but it causes three concrete problems in this app:
//!
//! 1. **Display**: `files.path` is shown verbatim in the UI. Nobody wants to see
//!    `\\?\C:\monitoramento` (and on non-Windows-aware renderers it can even show up
//!    slash-mangled, e.g. `//?/C:/monitoramento`, since some tools treat `\\?\` as if
//!    it were a regular path and normalize its backslashes).
//! 2. **Containment (`Path::starts_with`)**: `queue::intake`'s VULN-003 containment
//!    check compares the canonicalized file path against `root`. If one side went
//!    through `canonicalize` (verbatim) and the other didn't (or vice versa),
//!    `starts_with` fails even though the two paths refer to the same location —
//!    either wrongly rejecting a legitimate file or (worse) failing open in some
//!    refactor down the line.
//! 3. **Explorer "reveal"**: shelling out to Explorer with a verbatim path does not
//!    always behave the same as with the plain form.
//!
//! The `dunce` crate solves exactly this, but it is a single-purpose dependency for
//! a handful of lines of string manipulation, so we implement it ourselves instead
//! (per SPEC.md §4, prefer no new dependency over a one-function crate).
//!
//! This module's stripping logic is pure string manipulation and runs on every OS
//! (a POSIX path never starts with `\\?\`, so [`strip_verbatim`] is a no-op there),
//! which is what makes it unit-testable without `#[cfg(windows)]` gates on the table
//! test itself.

use std::io;
use std::path::{Path, PathBuf};

/// Verbatim-disk prefix: `\\?\C:\...` → strip this, keep `C:\...`.
const VERBATIM_PREFIX: &str = r"\\?\";
/// Verbatim-UNC prefix: `\\?\UNC\server\share\...` → strip this, restore the
/// leading `\\`, keep `server\share\...`.
const VERBATIM_UNC_PREFIX: &str = r"\\?\UNC\";

/// Strips a Windows verbatim (`\\?\`) prefix from `p`, if present.
///
/// - `\\?\C:\x` → `C:\x`
/// - `\\?\UNC\server\share\x` → `\\server\share\x`
/// - anything else (including every POSIX path) is returned unchanged.
///
/// Pure string logic — no filesystem access, no `#[cfg(windows)]` — so it can (and
/// must) be exercised on every OS in CI.
pub fn strip_verbatim(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();

    if let Some(rest) = s.strip_prefix(VERBATIM_UNC_PREFIX) {
        return PathBuf::from(format!(r"\\{rest}"));
    }

    if let Some(rest) = s.strip_prefix(VERBATIM_PREFIX) {
        return PathBuf::from(rest);
    }

    p.to_path_buf()
}

/// `tokio::fs::canonicalize` + [`strip_verbatim`]: the async variant every async
/// call site (`queue::intake`, `rescan()`, `run_intake_loop`) should use instead of
/// calling `tokio::fs::canonicalize` directly, so the stored/compared path is always
/// in the clean (non-verbatim) form.
pub async fn canonicalize_clean(p: &Path) -> io::Result<PathBuf> {
    let canonical = tokio::fs::canonicalize(p).await?;
    Ok(strip_verbatim(&canonical))
}

/// Synchronous twin of [`canonicalize_clean`], for call sites (mainly tests) that
/// don't run inside a Tokio task.
pub fn canonicalize_clean_sync(p: &Path) -> io::Result<PathBuf> {
    let canonical: PathBuf = std::fs::canonicalize(p)?;
    Ok(strip_verbatim(&canonical))
}

/// Windows system entries a scan must never touch: not descended into (when
/// `p` is a directory) and not `stat`-ed (when `p` is a file). Matched on the
/// final path component only, case-insensitively.
///
/// `System Volume Information` and `$RECYCLE.BIN` deny access to everyone but
/// `SYSTEM` on every NTFS volume (os error 5 — `PermissionDenied`), so a scan
/// that reaches them fails on every single run, not intermittently; skipping
/// them by name avoids that permission round trip entirely instead of
/// depending on `rescan::walk_dir`'s per-entry error tolerance to paper over
/// it. `pagefile.sys` / `hiberfil.sys` / `swapfile.sys` are locked by the OS
/// while it runs, and `DumpStack.log*` is written by the crash-dump service —
/// none of them are ever a file this app is meant to sync.
///
/// Pure string logic — no filesystem access, no `#[cfg(windows)]` — so, like
/// [`strip_verbatim`], it is unit-testable on every OS in CI even though the
/// names it matches only ever occur on Windows.
pub fn is_system_path(p: &Path) -> bool {
    // Deliberately not `p.file_name()`: `std::path::Path` only splits on `/`
    // on a non-Windows host, so a Windows-style `E:\System Volume
    // Information` test string would come back as one single (wrong)
    // component when this test runs on macOS/Linux CI. Manual splitting on
    // both separators, same as `strip_verbatim` above, makes the result
    // identical regardless of the host OS the test happens to run on.
    let full = p.to_string_lossy();
    let trimmed = full.trim_end_matches(['/', '\\']);
    let name = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);

    const EXACT: &[&str] = &[
        "System Volume Information",
        "$RECYCLE.BIN",
        "$Extend",
        "Config.Msi",
        "Recovery",
        "$WinREAgent",
        "pagefile.sys",
        "hiberfil.sys",
        "swapfile.sys",
    ];
    if EXACT.iter().any(|exact| exact.eq_ignore_ascii_case(name)) {
        return true;
    }

    name.to_ascii_lowercase().starts_with("dumpstack.log")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Case {
        name: &'static str,
        input: &'static str,
        expected: &'static str,
    }

    #[test]
    fn strip_verbatim_table() {
        let cases = [
            Case {
                name: "verbatim disk prefix is stripped",
                input: r"\\?\C:\monitoramento\file.bin",
                expected: r"C:\monitoramento\file.bin",
            },
            Case {
                name: "verbatim UNC prefix is stripped and \\\\ restored",
                input: r"\\?\UNC\server\share\file.bin",
                expected: r"\\server\share\file.bin",
            },
            Case {
                name: "verbatim disk prefix with nested dirs",
                input: r"\\?\D:\a\b\c\d.txt",
                expected: r"D:\a\b\c\d.txt",
            },
            Case {
                name: "already-clean Windows path is unchanged",
                input: r"C:\monitoramento\file.bin",
                expected: r"C:\monitoramento\file.bin",
            },
            Case {
                name: "POSIX path is unchanged",
                input: "/private/tmp/monitoramento/file.bin",
                expected: "/private/tmp/monitoramento/file.bin",
            },
            Case {
                name: "relative POSIX path is unchanged",
                input: "monitoramento/file.bin",
                expected: "monitoramento/file.bin",
            },
            Case {
                name: "plain UNC path without the verbatim marker is unchanged",
                input: r"\\server\share\file.bin",
                expected: r"\\server\share\file.bin",
            },
            Case {
                name: "verbatim UNC root with no trailing path",
                input: r"\\?\UNC\server\share",
                expected: r"\\server\share",
            },
        ];

        for case in cases {
            let actual = strip_verbatim(Path::new(case.input));
            assert_eq!(
                actual,
                PathBuf::from(case.expected),
                "case failed: {}",
                case.name
            );
        }
    }

    #[tokio::test]
    async fn canonicalize_clean_strips_verbatim_prefix_if_platform_adds_one() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let result = canonicalize_clean(dir.path())
            .await
            .expect("canonicalize_clean should succeed on an existing dir");

        assert!(
            !result.to_string_lossy().contains(r"\\?\"),
            "canonicalize_clean must never leak a verbatim prefix, got {result:?}"
        );
    }

    #[test]
    fn canonicalize_clean_sync_strips_verbatim_prefix_if_platform_adds_one() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let result = canonicalize_clean_sync(dir.path())
            .expect("canonicalize_clean_sync should succeed on an existing dir");

        assert!(
            !result.to_string_lossy().contains(r"\\?\"),
            "canonicalize_clean_sync must never leak a verbatim prefix, got {result:?}"
        );
    }

    #[tokio::test]
    async fn canonicalize_clean_propagates_io_errors() {
        let missing = Path::new("/definitely/does/not/exist/at/all/paths-test");
        let err = canonicalize_clean(missing)
            .await
            .expect_err("canonicalizing a missing path must error");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn is_system_path_table() {
        let cases = [
            (r"E:\System Volume Information", true),
            (r"E:\system volume information", true),
            (r"E:\$RECYCLE.BIN", true),
            (r"E:\$recycle.bin", true),
            (r"C:\$Extend", true),
            (r"C:\Config.Msi", true),
            (r"C:\Recovery", true),
            (r"C:\$WinREAgent", true),
            (r"C:\pagefile.sys", true),
            (r"C:\hiberfil.sys", true),
            (r"C:\swapfile.sys", true),
            (r"C:\DumpStack.log", true),
            (r"C:\DumpStack.log.tmp", true),
            (r"C:\dumpstack.log.tmp", true),
            (r"E:\gravacoes\video.mp4", false),
            (r"E:\gravacoes\Recovery Plan.docx", false),
        ];

        for (input, expected) in cases {
            let path = Path::new(input);
            assert_eq!(
                is_system_path(path),
                expected,
                "is_system_path({input:?}) should be {expected}"
            );
        }
    }
}
