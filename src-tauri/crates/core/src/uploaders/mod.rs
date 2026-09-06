//! `core::uploaders` — the `Uploader` trait both destinations implement
//! (SPEC.md §6 "uploaders/mod.rs"; PRD.md RF-032).
//!
//! This module only defines the *contract*: request/result/error shapes
//! and the provider-agnostic error classifier. `worker.rs` (T-3.5) drives
//! uploads through it without knowing whether `id()` is `S3` or `GDrive`;
//! `s3.rs` (T-3.3) and `gdrive/` (T-3.4) are the two implementations and
//! land in later tasks — nothing here depends on `aws-sdk-s3`,
//! `yup-oauth2` or `reqwest`.

mod error;
pub mod gdrive;
pub mod s3;

pub use error::{
    classify, classify_transport, Classified, ClassifyContext, ErrorBody, UploadError,
};
// Re-exported for provider modules that need to build a body from a raw
// HTTP response before calling `classify` (e.g. `s3.rs`, `gdrive/upload.rs`).
pub use error::parse_error_body;

use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

use crate::state::Destination;

/// One upload, from a stable local file to a named remote object/file.
///
/// `resume_state` carries whatever `remote_state` a previous, interrupted
/// attempt at the *same* job left in the `jobs` table (SPEC.md §5) — an S3
/// multipart `UploadId` with completed part numbers, or a Drive resumable
/// `session_uri`. `None` means "first attempt" or "resume state expired,
/// start over" (SPEC.md §6 gdrive: a 404/410 on the stored `session_uri`
/// clears it before retrying).
#[derive(Debug)]
pub struct UploadRequest {
    pub local_path: PathBuf,
    pub remote_name: String,
    pub size: u64,
    pub sha256: String,
    /// Bytes-sent notifications. Best-effort like `hash.rs`'s progress
    /// channel — an uploader must never block or fail the upload because
    /// this channel is full or closed.
    pub progress: mpsc::Sender<ProgressUpdate>,
    /// Cooperative cancellation, checked by the uploader between chunks/parts.
    /// An upload that observes cancellation returns `Err(UploadError::Cancelled)`.
    pub cancel: CancellationToken,
    pub resume_state: Option<serde_json::Value>,
}

/// Bytes sent so far vs. the total (`UploadRequest::size`), for the
/// throughput/progress UI (SPEC.md §6 `throttle.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressUpdate {
    pub sent: u64,
    pub total: u64,
}

/// Outcome of a successful [`Uploader::upload`].
///
/// `remote_state` is opaque to the worker: it's whatever the provider
/// wants persisted in `jobs.remote_state` for later use — Drive's
/// `web_view_link` (used by `open_remote`, SPEC.md §6) or, mid-upload, an
/// in-progress multipart/resumable session for [`UploadRequest::resume_state`]
/// to pick back up.
#[derive(Debug, Clone)]
pub struct UploadResult {
    pub remote_id: String,
    pub remote_state: Option<serde_json::Value>,
}

/// Result of [`Uploader::test_connection`] — the "Testar conexão" button
/// (SPEC.md §7 IPC `test_connection`). Crosses IPC, hence `Serialize`/`TS`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TestResult {
    pub ok: bool,
    pub message: String,
    pub latency_ms: u32,
}

/// What every upload destination implements (SPEC.md §6). Implementations
/// land in `s3.rs` (T-3.3) and `gdrive/` (T-3.4); `worker.rs` (T-3.5)
/// depends only on this trait, never on a concrete provider type.
#[async_trait]
pub trait Uploader: Send + Sync {
    /// Which destination this is (`S3` or `GDrive`) — used to route
    /// `jobs.destination` rows and health/status events to the right
    /// implementation.
    fn id(&self) -> Destination;

    /// "Testar conexão" (SPEC.md §7): a cheap, side-effect-limited probe
    /// that also verifies write access, not just read (S3 `head_bucket`;
    /// Drive creates+deletes a zero-byte probe file, since a read-only
    /// service account can't be told apart from a healthy one otherwise).
    async fn test_connection(&self) -> Result<TestResult, UploadError>;

    /// Uploads one file. Idempotent by contract: implementations check for
    /// an existing remote object with a matching hash before transferring
    /// any bytes (SPEC.md §6 "Idempotência").
    async fn upload(&self, req: UploadRequest) -> Result<UploadResult, UploadError>;

    /// Cleans up an in-progress upload that will not be resumed — S3's
    /// `AbortMultipartUpload` on crash recovery (SPEC.md §5 "Crash
    /// recovery no boot"). Default no-op: destinations without a
    /// server-side partial-upload concept (Drive's resumable session just
    /// expires on its own) don't need to override it.
    async fn abort(&self, _remote_state: &serde_json::Value) -> Result<(), UploadError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `TestResult`'s JSON keys are part of the IPC contract with the
    /// frontend (SPEC.md §7 `test_connection` -> `TestResult`); this locks
    /// the exact field names so a rename doesn't silently break `invoke()`
    /// callers on the TS side without a compile error here.
    #[test]
    fn test_result_serde_keys() {
        let result = TestResult {
            ok: true,
            message: "Connected".to_string(),
            latency_ms: 42,
        };
        let value = serde_json::to_value(&result).unwrap();
        let obj = value.as_object().unwrap();
        assert_eq!(obj.len(), 3, "unexpected extra/missing fields: {obj:?}");
        assert_eq!(obj.get("ok").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(
            obj.get("message").and_then(|v| v.as_str()),
            Some("Connected")
        );
        assert_eq!(obj.get("latency_ms").and_then(|v| v.as_u64()), Some(42));
    }

    #[test]
    fn test_result_round_trips() {
        let raw = r#"{"ok":false,"message":"timeout","latency_ms":5000}"#;
        let result: TestResult = serde_json::from_str(raw).unwrap();
        assert!(!result.ok);
        assert_eq!(result.message, "timeout");
        assert_eq!(result.latency_ms, 5000);
    }

    /// `Uploader` must be object-safe / usable behind `Arc<dyn Uploader>`
    /// (the worker holds one uploader per destination) — this is a
    /// compile-time check disguised as a test: it only needs to compile.
    struct NoopUploader;

    #[async_trait]
    impl Uploader for NoopUploader {
        fn id(&self) -> Destination {
            Destination::S3
        }

        async fn test_connection(&self) -> Result<TestResult, UploadError> {
            Ok(TestResult {
                ok: true,
                message: "ok".to_string(),
                latency_ms: 1,
            })
        }

        async fn upload(&self, _req: UploadRequest) -> Result<UploadResult, UploadError> {
            Ok(UploadResult {
                remote_id: "id".to_string(),
                remote_state: None,
            })
        }
        // `abort` intentionally left at its default no-op impl.
    }

    #[tokio::test]
    async fn uploader_trait_object_is_usable() {
        let uploader: std::sync::Arc<dyn Uploader> = std::sync::Arc::new(NoopUploader);
        assert_eq!(uploader.id(), Destination::S3);

        let (tx, _rx) = mpsc::channel(1);
        let req = UploadRequest {
            local_path: PathBuf::from("/tmp/does-not-matter"),
            remote_name: "file.bin".to_string(),
            size: 0,
            sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
            progress: tx,
            cancel: CancellationToken::new(),
            resume_state: None,
        };
        let result = uploader.upload(req).await.unwrap();
        assert_eq!(result.remote_id, "id");

        // Default `abort` is a no-op `Ok(())`.
        uploader.abort(&serde_json::json!({})).await.unwrap();
    }
}
