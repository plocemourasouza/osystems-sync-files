//! `core::hash` — Fase 2 (T-2.1).
//!
//! SHA-256 file hashing (SPEC.md §2, §12 C5; PRD.md RF-039, RNF-004, RNF-007).
//!
//! **Why SHA-256 (C5):** the integrity checksum is compared against
//! provider-native metadata on both destinations — S3's `x-amz-meta-sha256`
//! object metadata and Google Drive's `sha256Checksum` field (SPEC.md §6) —
//! so the algorithm is fixed by what the destinations already expose, not a
//! local preference (BLAKE3 was decorative in the mockup only).
//!
//! **Why streaming:** files can be up to 5 GB (RNF-004). Reading a whole
//! file into memory before hashing would spike RSS proportionally to file
//! size and stall the idle-CPU/idle-I/O budget (RNF-007) for large files.
//! Instead every function here reads through a fixed 1 MiB buffer, so peak
//! memory stays constant regardless of file size.
//!
//! Hashing is CPU/disk-bound synchronous work, so the async entry points
//! (`sha256_file`, `sha256_file_with_progress`) always run the blocking
//! implementation inside `tokio::task::spawn_blocking` — the Tokio runtime
//! is never blocked directly (CLAUDE.md: "Não bloquear o runtime Tokio com
//! I/O síncrono pesado").

use std::fmt;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio::sync::mpsc::Sender;

/// Size of the read buffer used while streaming a file through the hasher.
const BUFFER_SIZE: usize = 1024 * 1024; // 1 MiB

/// Minimum amount of newly-hashed bytes between progress notifications.
///
/// Progress is best-effort (RNF-008 UI responsiveness) — the hasher never
/// blocks on a full channel, it just skips that update.
const PROGRESS_STEP_BYTES: u64 = 8 * 1024 * 1024; // 8 MiB

/// Lowercase hex-encoded SHA-256 digest of a file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sha256Digest(pub String);

impl fmt::Display for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for Sha256Digest {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Errors surfaced by `core::hash`.
#[derive(Debug, thiserror::Error)]
pub enum HashError {
    #[error("io error hashing file: {0}")]
    Io(#[from] io::Error),

    #[error("hashing task panicked or was cancelled: {0}")]
    Join(#[from] tokio::task::JoinError),
}

/// Streams `path` through SHA-256 using a 1 MiB buffer and returns the
/// lowercase hex digest.
///
/// Synchronous and blocking on purpose — callers on the Tokio runtime must
/// use [`sha256_file`] or [`sha256_file_with_progress`] instead, which wrap
/// this in `spawn_blocking`.
pub fn sha256_file_blocking(path: &Path) -> Result<Sha256Digest, HashError> {
    hash_with_progress(path, |_| {})
}

/// Async wrapper around [`sha256_file_blocking`] that runs the streaming
/// hash on the blocking thread pool so it never stalls the Tokio runtime.
pub async fn sha256_file(path: PathBuf) -> Result<Sha256Digest, HashError> {
    tokio::task::spawn_blocking(move || sha256_file_blocking(&path)).await?
}

/// Same as [`sha256_file`], but reports cumulative bytes hashed on
/// `progress` roughly every [`PROGRESS_STEP_BYTES`], plus a final message
/// equal to the total file size.
///
/// Sending is best-effort (`try_send`): a full or closed channel never
/// blocks or fails the hash — it only means the caller misses a progress
/// tick (RNF-008: progress is a UI nicety, not a correctness requirement).
pub async fn sha256_file_with_progress(
    path: PathBuf,
    progress: Option<Sender<u64>>,
) -> Result<Sha256Digest, HashError> {
    tokio::task::spawn_blocking(move || {
        hash_with_progress(&path, |bytes_read| {
            if let Some(tx) = &progress {
                let _ = tx.try_send(bytes_read);
            }
        })
    })
    .await?
}

/// Core streaming loop shared by every entry point above. Calls
/// `on_progress(cumulative_bytes_read)` at least once every
/// [`PROGRESS_STEP_BYTES`] and once more with the final total.
fn hash_with_progress(
    path: &Path,
    mut on_progress: impl FnMut(u64),
) -> Result<Sha256Digest, HashError> {
    let file = File::open(path)?;
    let mut reader = BufReader::with_capacity(BUFFER_SIZE, file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; BUFFER_SIZE];

    let mut total_read: u64 = 0;
    let mut since_last_progress: u64 = 0;

    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        total_read += n as u64;
        since_last_progress += n as u64;

        if since_last_progress >= PROGRESS_STEP_BYTES {
            on_progress(total_read);
            since_last_progress = 0;
        }
    }

    on_progress(total_read);

    let digest = hasher.finalize();
    Ok(Sha256Digest(hex_encode(&digest)))
}

/// Lowercase hex encoding without pulling in an extra crate dependency.
fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;
    use tokio::sync::mpsc;

    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn write_temp(contents: &[u8]) -> NamedTempFile {
        let mut file = NamedTempFile::new().expect("create temp file");
        file.write_all(contents).expect("write temp file");
        file.flush().expect("flush temp file");
        file
    }

    #[test]
    fn blocking_hashes_empty_file_to_known_vector() {
        let file = write_temp(b"");
        let digest = sha256_file_blocking(file.path()).expect("hash empty file");
        assert_eq!(digest.0, EMPTY_SHA256);
        assert_eq!(digest.to_string(), EMPTY_SHA256);
        assert_eq!(digest.as_ref(), EMPTY_SHA256);
    }

    #[test]
    fn blocking_hashes_abc_to_known_vector() {
        let file = write_temp(b"abc");
        let digest = sha256_file_blocking(file.path()).expect("hash abc file");
        assert_eq!(digest.0, ABC_SHA256);
    }

    #[test]
    fn blocking_missing_file_returns_io_error() {
        let missing = PathBuf::from("/definitely/does/not/exist/osystems-sync-hash-test");
        let result = sha256_file_blocking(&missing);
        assert!(matches!(result, Err(HashError::Io(_))));
    }

    #[tokio::test]
    async fn async_missing_file_returns_io_error() {
        let missing = PathBuf::from("/definitely/does/not/exist/osystems-sync-hash-test");
        let result = sha256_file(missing).await;
        assert!(matches!(result, Err(HashError::Io(_))));
    }

    #[tokio::test]
    async fn blocking_and_async_paths_agree_with_reference_one_shot_hash() {
        // 3 MiB of pseudo-random bytes, generated with a cheap xorshift so
        // the test has no extra dependency on `rand`.
        let mut data = vec![0u8; 3 * 1024 * 1024];
        let mut state: u64 = 0x243F_6A88_85A3_08D3; // arbitrary nonzero seed
        for chunk in data.chunks_mut(8) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let bytes = state.to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }

        let file = write_temp(&data);

        let reference = {
            let mut hasher = Sha256::new();
            hasher.update(&data);
            hex_encode(&hasher.finalize())
        };

        let blocking_digest = sha256_file_blocking(file.path()).expect("blocking hash");
        let async_digest = sha256_file(file.path().to_path_buf())
            .await
            .expect("async hash");

        assert_eq!(blocking_digest.0, reference);
        assert_eq!(async_digest.0, reference);
        assert_eq!(blocking_digest, async_digest);
    }

    #[tokio::test]
    async fn progress_channel_receives_updates_ending_at_file_size() {
        let size: u64 = 20 * 1024 * 1024; // 20 MiB, zeroed (sparse-ish, fast to write)
        let data = vec![0u8; size as usize];
        let file = write_temp(&data);

        let (tx, mut rx) = mpsc::channel::<u64>(64);
        let digest = sha256_file_with_progress(file.path().to_path_buf(), Some(tx))
            .await
            .expect("hash with progress");

        // Sanity: digest matches a direct one-shot hash of the same buffer.
        let mut hasher = Sha256::new();
        hasher.update(&data);
        assert_eq!(digest.0, hex_encode(&hasher.finalize()));

        let mut updates = Vec::new();
        while let Ok(value) = rx.try_recv() {
            updates.push(value);
        }

        assert!(
            !updates.is_empty(),
            "expected at least one progress update for a 20 MiB file"
        );
        assert_eq!(*updates.last().expect("last progress update"), size);
    }

    #[tokio::test]
    async fn progress_is_none_by_default_and_still_hashes() {
        let file = write_temp(b"abc");
        let digest = sha256_file_with_progress(file.path().to_path_buf(), None)
            .await
            .expect("hash without progress sender");
        assert_eq!(digest.0, ABC_SHA256);
    }
}
