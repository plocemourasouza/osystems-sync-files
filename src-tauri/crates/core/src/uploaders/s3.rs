//! `uploaders::s3` — `S3Uploader` (T-3.3/3.4/3.5).
//!
//! T-3.3 implements the simple `PutObject` path for files under
//! [`SIMPLE_UPLOAD_MAX`] plus `test_connection` (SPEC.md §6 "s3.rs").
//! `upload()` grew additively on top of it, not a rewrite:
//!
//! - T-3.4 (multipart, files `>= SIMPLE_UPLOAD_MAX`) adds
//!   `CreateMultipartUpload`/`UploadPart`/`CompleteMultipartUpload`
//!   (`upload_multipart`, [`PART_SIZE`] parts, [`PART_CONCURRENCY`]
//!   concurrent) — it does not touch `put_simple`. A previous attempt's
//!   `upload_id`/parts (`UploadRequest::resume_state`) are verified via
//!   `ListParts` before being trusted (`start_or_resume_multipart`); a
//!   cancel or a non-retryable part error aborts the multipart upload
//!   (`abort_multipart`), a retryable one leaves it open for a future
//!   resume. `S3Uploader::with_state_sink` lets the caller persist
//!   `remote_state` after every part completes, since `UploadRequest`
//!   carries no per-job callback of its own.
//! - T-3.5 (idempotency) adds a `head_object` probe at the top of
//!   `upload()` (`check_existing`), before either branch, comparing
//!   `x-amz-meta-sha256` on the existing object to `req.sha256` and
//!   returning `Ok` without transferring bytes on a match — it does not
//!   touch `put_simple` or the multipart branch either.
//!
//! Every provider error (`HeadBucket`/`PutObject`/`DeleteObject`/the
//! multipart operations) funnels through [`map_sdk_error`] into
//! [`super::classify`]/[`super::classify_transport`], so `S3Uploader`
//! never invents its own error taxonomy — it just extracts the HTTP
//! status/body the SDK saw and lets the shared classifier decide
//! Auth/Transient/Permanent (SPEC.md §6 "Classificação de erros").

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use aws_sdk_s3::config::{BehaviorVersion, Builder as S3ConfigBuilder, Credentials, Region};
use aws_sdk_s3::error::{ProvideErrorMetadata, SdkError};
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart, StorageClass as S3StorageClass};
use aws_smithy_types::retry::RetryConfig;
use aws_smithy_types::timeout::TimeoutConfig;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::config::{S3Config, StorageClass};
use crate::credentials::AwsCredentials;
use crate::state::Destination;
use crate::throttle::{Throttle, ThrottledReader};

use super::{
    classify, classify_transport, parse_error_body, ClassifyContext, ProgressUpdate, TestResult,
    UploadError, UploadRequest, UploadResult, Uploader,
};

/// Files smaller than this go through [`S3Uploader::put_simple`]
/// (`PutObject`, one request). Files at or above it go through
/// `upload_multipart` (SPEC.md §6: "`>= 8 MB`: multipart, parte de 16 MB,
/// 4 partes concorrentes").
pub const SIMPLE_UPLOAD_MAX: u64 = 8 * 1024 * 1024;

/// Baseline multipart part size (SPEC.md §6: "parte de 16 MB"). S3 requires
/// every part but the last to be at least 5 MiB; 16 MiB keeps the part
/// count (and therefore `CompleteMultipartUpload`'s XML body) small for the
/// common case while still letting [`PART_CONCURRENCY`] parts make
/// meaningful progress under the RNF-015 bandwidth cap. Files larger than
/// `PART_SIZE * MAX_PARTS` (160 GiB) get a proportionally bigger part --
/// see [`part_size_for`].
pub const PART_SIZE: u64 = 16 * 1024 * 1024;

/// S3's own limit on how many parts one multipart upload may have.
pub const MAX_PARTS: u64 = 10_000;

/// S3's own limit on a single part (5 GiB). `MAX_PARTS * MAX_PART_SIZE` is
/// 50 TiB, well past the 5 TiB object limit, so [`MAX_OBJECT_SIZE`] is
/// always the binding constraint.
pub const MAX_PART_SIZE: u64 = 5 * 1024 * 1024 * 1024;

/// S3's own limit on a single object (5 TiB). Mirrors
/// `config::MAX_FILE_SIZE_MB`.
pub const MAX_OBJECT_SIZE: u64 = 5 * 1024 * 1024 * 1024 * 1024;

/// How many parts upload concurrently (SPEC.md §6: "4 partes
/// concorrentes"). This is a ceiling, not a fixed value: [`part_concurrency`]
/// lowers it for large parts so in-flight buffers stay inside
/// [`PART_MEMORY_BUDGET`].
pub const PART_CONCURRENCY: usize = 4;

/// Cap on bytes held in memory by in-flight parts. [`read_part`] buffers a
/// whole part before sending it, so `part_size * concurrency` is the real
/// peak; without this bound a 512 MiB part times 4 workers would be 2 GiB
/// of RSS and break RNF-001.
pub const PART_MEMORY_BUDGET: u64 = 256 * 1024 * 1024;

/// Part size for a `total_size`-byte object: [`PART_SIZE`] until that would
/// need more than [`MAX_PARTS`] parts, then the smallest MiB-aligned size
/// that fits the file into `MAX_PARTS`.
///
/// Deterministic in `total_size` alone, which is what makes resume safe --
/// a session persisted under one run recomputes the identical part layout
/// on the next, so `part_bounds` still lines up with the parts S3 already
/// has.
pub fn part_size_for(total_size: u64) -> u64 {
    const MIB: u64 = 1024 * 1024;
    if total_size <= PART_SIZE * MAX_PARTS {
        return PART_SIZE;
    }
    total_size.div_ceil(MAX_PARTS).div_ceil(MIB) * MIB
}

/// How many parts of `part_size` may be in flight at once: [`PART_CONCURRENCY`]
/// shrunk to fit [`PART_MEMORY_BUDGET`], never below 1.
pub fn part_concurrency(part_size: u64) -> usize {
    let by_budget = (PART_MEMORY_BUDGET / part_size).max(1);
    usize::try_from(by_budget)
        .unwrap_or(PART_CONCURRENCY)
        .min(PART_CONCURRENCY)
}

/// Callback an uploader owner (the worker, T-3.9) supplies via
/// [`S3Uploader::with_state_sink`] to persist `jobs.remote_state`
/// (SPEC.md §5) as a multipart upload progresses. `UploadRequest` carries
/// no such callback of its own — it's a `mod.rs` type shared with
/// `gdrive/`, and adding an S3-only field there would leak this
/// provider's persistence mechanics into the provider-agnostic contract.
///
/// Called with `(remote_name, remote_state)` after `CreateMultipartUpload`
/// (or a verified resume) and again after every completed part, so a
/// crash before `CompleteMultipartUpload` loses at most the in-flight
/// parts, not the whole upload.
pub type StateSink = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

/// Zero-byte object `test_connection` creates then deletes to prove write
/// (not just read) access, named so it never collides with a real upload
/// (SPEC.md §9: "apenas para o probe de `test_connection`").
const PROBE_OBJECT_NAME: &str = ".osystems-sync-probe";

/// How long to wait for the TCP connect (SPEC.md RNF-012: fail fast on a
/// dead network rather than hang).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Ceiling for one S3 operation end-to-end. Generous because the throttle
/// (RNF-015, 2.5 MB/s cap) can legitimately stretch a large `PutObject`
/// well past typical HTTP client defaults.
const OPERATION_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Test/deployment-time overrides that production config never sets.
/// `endpoint_url` points the SDK at a `wiremock`/MinIO endpoint instead of
/// real S3; `force_path_style` is required for both (they don't own a
/// `<bucket>.<host>` virtual-hosted DNS entry). `skip_head_check` bypasses
/// T-3.5's idempotency probe entirely — for tests that don't want to mock
/// `HeadObject` on every call; production always leaves it `false`.
#[derive(Debug, Clone, Default)]
pub struct S3Options {
    pub endpoint_url: Option<String>,
    pub force_path_style: bool,
    pub skip_head_check: bool,
}

/// `Uploader` implementation backed by `aws-sdk-s3`. Holds one configured
/// client per destination (credentials/region/endpoint baked in at
/// construction) plus the shared bandwidth [`Throttle`] every upload reads
/// through.
pub struct S3Uploader {
    client: aws_sdk_s3::Client,
    cfg: S3Config,
    throttle: Arc<Throttle>,
    skip_head_check: bool,
    state_sink: Option<StateSink>,
}

impl S3Uploader {
    /// Builds the SDK client with static keyring credentials
    /// (`Credentials::new`, provider name `"keyring"` — never a real AWS
    /// profile/env lookup, so a misconfigured host environment can't
    /// silently supply different credentials than the ones the user
    /// entered) and our own timeouts/retry policy.
    ///
    /// Retries are disabled at the SDK level (`RetryConfig::disabled()`):
    /// the worker (T-3.5, `worker.rs`) owns backoff/attempt-count policy
    /// for every destination uniformly, so the SDK retrying underneath it
    /// would double the effective attempt budget and hide transient
    /// failures the worker is supposed to observe and log.
    ///
    /// Never logs `creds.secret_access_key` — `AwsCredentials`'s `Debug`
    /// impl already masks it, and this function never formats the field
    /// directly.
    pub async fn new(
        cfg: S3Config,
        creds: AwsCredentials,
        throttle: Arc<Throttle>,
        opts: S3Options,
    ) -> Result<Self, UploadError> {
        let credentials = Credentials::new(
            creds.access_key_id,
            creds.secret_access_key,
            None,
            None,
            "keyring",
        );

        let timeout_config = TimeoutConfig::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .operation_timeout(OPERATION_TIMEOUT)
            .build();

        // Explicit HTTP client: rustls + `ring` (no `aws-lc-sys` C toolchain, so the
        // core cross-compiles to `x86_64-pc-windows-msvc`). The rustls provider
        // accepts plain `http://` too (`enforce_http(false)`), which MinIO/wiremock need.
        let http_client = aws_smithy_http_client::Builder::new()
            .tls_provider(aws_smithy_http_client::tls::Provider::Rustls(
                aws_smithy_http_client::tls::rustls_provider::CryptoMode::Ring,
            ))
            .build_https();

        let mut builder = S3ConfigBuilder::new()
            .http_client(http_client)
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(cfg.region.clone()))
            .credentials_provider(credentials)
            .timeout_config(timeout_config)
            .retry_config(RetryConfig::disabled());

        if let Some(endpoint_url) = opts.endpoint_url {
            builder = builder.endpoint_url(endpoint_url);
        }
        if opts.force_path_style {
            builder = builder.force_path_style(true);
        }

        let client = aws_sdk_s3::Client::from_conf(builder.build());

        Ok(Self {
            client,
            cfg,
            throttle,
            skip_head_check: opts.skip_head_check,
            state_sink: None,
        })
    }

    /// Registers a [`StateSink`] to persist `remote_state` as a multipart
    /// upload progresses. Consumes and returns `self` so construction
    /// reads as `S3Uploader::new(..).await?.with_state_sink(sink)` at the
    /// call site (T-3.9's worker wiring) without an extra `let mut`.
    #[must_use]
    pub fn with_state_sink(mut self, sink: StateSink) -> Self {
        self.state_sink = Some(sink);
        self
    }

    /// Final object key: `{prefix}{remote_name}`, with any run of `/`
    /// produced by a prefix/name boundary collapsed to one (SPEC.md §6:
    /// "Key final: `{prefix}{remote_name}`").
    fn object_key(&self, remote_name: &str) -> String {
        collapse_slashes(&format!("{}{}", self.cfg.prefix, remote_name))
    }

    /// Maps our config-level [`StorageClass`] to the SDK's generated enum.
    fn storage_class(&self) -> S3StorageClass {
        match self.cfg.storage_class {
            StorageClass::Standard => S3StorageClass::Standard,
            StorageClass::IntelligentTiering => S3StorageClass::IntelligentTiering,
            StorageClass::GlacierIr => S3StorageClass::GlacierIr,
        }
    }

    /// The `< SIMPLE_UPLOAD_MAX` path: throttle-read the whole file into
    /// memory, then one `PutObject`. Reading fully (rather than streaming
    /// the `ThrottledReader` straight into the request body) is a
    /// deliberate simplification for files this small — at most 8 MiB
    /// resident, and it lets us set `content_length` up front instead of
    /// relying on chunked transfer encoding, which several S3-compatible
    /// backends (MinIO included) handle less predictably. T-3.4's
    /// multipart path streams per-part instead, since parts are 16 MiB and
    /// concurrent.
    async fn put_simple(
        &self,
        req: &UploadRequest,
        key: &str,
    ) -> Result<UploadResult, UploadError> {
        let _ = req.progress.try_send(ProgressUpdate {
            sent: 0,
            total: req.size,
        });

        let file = tokio::fs::File::open(&req.local_path).await?;
        let mut reader = ThrottledReader::new(file, self.throttle.clone());
        let mut buf = Vec::with_capacity(req.size as usize);
        reader.read_to_end(&mut buf).await?;
        let content_length = buf.len() as i64;

        let output = self
            .client
            .put_object()
            .bucket(&self.cfg.bucket)
            .key(key)
            .body(ByteStream::from(buf))
            .content_length(content_length)
            .metadata("sha256", req.sha256.as_str())
            .storage_class(self.storage_class())
            .send()
            .await
            .map_err(map_sdk_error)?;

        let _ = req.progress.try_send(ProgressUpdate {
            sent: req.size,
            total: req.size,
        });

        Ok(UploadResult {
            remote_id: key.to_string(),
            remote_state: Some(json!({ "etag": output.e_tag() })),
        })
    }

    /// T-3.5 idempotency (SPEC.md §6: "antes de enviar, `head_object`; se
    /// existe com mesmo sha256 na metadata, retorna `Ok` sem reenviar").
    /// `Ok(Some(_))` means the caller should skip the transfer entirely;
    /// `Ok(None)` covers every case that should fall through to a normal
    /// upload — no object, a mismatched hash, or `skip_head_check`.
    async fn check_existing(
        &self,
        req: &UploadRequest,
        key: &str,
    ) -> Result<Option<UploadResult>, UploadError> {
        if self.skip_head_check {
            return Ok(None);
        }

        let output = match self
            .client
            .head_object()
            .bucket(&self.cfg.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(output) => output,
            Err(SdkError::ServiceError(service_err)) if service_err.err().is_not_found() => {
                return Ok(None);
            }
            Err(err) => return Err(map_sdk_error(err)),
        };

        let existing_sha256 = output.metadata().and_then(|m| m.get("sha256"));
        if existing_sha256 != Some(&req.sha256) {
            return Ok(None);
        }

        Ok(Some(UploadResult {
            remote_id: key.to_string(),
            remote_state: Some(json!({ "etag": output.e_tag() })),
        }))
    }

    /// Forwards the current multipart state to [`Self::state_sink`], if
    /// one was registered. `key` rides along in `remote_state` itself
    /// (SPEC.md §5's `{upload_id, parts[]}` plus `key`) so [`Self::abort`]
    /// is self-contained on crash recovery — it doesn't need to
    /// recompute `object_key` from a `remote_name` it isn't given.
    fn persist_state(
        &self,
        remote_name: &str,
        key: &str,
        upload_id: &str,
        parts: &[(i32, String)],
    ) {
        let Some(sink) = &self.state_sink else {
            return;
        };
        let parts_json: Vec<serde_json::Value> = parts
            .iter()
            .map(|(part_number, etag)| json!({ "part_number": part_number, "etag": etag }))
            .collect();
        sink(
            remote_name,
            json!({ "upload_id": upload_id, "key": key, "parts": parts_json }),
        );
    }

    /// Starts a fresh multipart upload, or verifies and resumes one
    /// recorded in `req.resume_state` (SPEC.md §5 `remote_state`:
    /// `{upload_id, key, parts[]}`). A resume candidate is only trusted
    /// after `ListParts` confirms S3 still knows the `upload_id` — it may
    /// have been aborted by a previous crash-recovery pass (SPEC.md §5
    /// "Crash recovery no boot") or expired via a bucket lifecycle rule;
    /// either way we fall back to starting over rather than failing the
    /// job outright.
    async fn start_or_resume_multipart(
        &self,
        req: &UploadRequest,
        key: &str,
    ) -> Result<(String, Vec<(i32, String)>), UploadError> {
        if let Some(state) = &req.resume_state {
            let upload_id = state.get("upload_id").and_then(|v| v.as_str());
            let same_key = state.get("key").and_then(|v| v.as_str()) == Some(key);

            if let Some(upload_id) = upload_id.filter(|_| same_key) {
                match self
                    .client
                    .list_parts()
                    .bucket(&self.cfg.bucket)
                    .key(key)
                    .upload_id(upload_id)
                    .send()
                    .await
                {
                    Ok(output) => {
                        let parts = output
                            .parts()
                            .iter()
                            .filter_map(|p| Some((p.part_number()?, p.e_tag()?.to_string())))
                            .collect();
                        return Ok((upload_id.to_string(), parts));
                    }
                    Err(SdkError::ServiceError(service_err))
                        if service_err.err().code() == Some("NoSuchUpload") =>
                    {
                        // Stale/aborted upload_id — fall through and start over.
                    }
                    Err(err) => return Err(map_sdk_error(err)),
                }
            }
        }

        let output = self
            .client
            .create_multipart_upload()
            .bucket(&self.cfg.bucket)
            .key(key)
            .metadata("sha256", req.sha256.as_str())
            .storage_class(self.storage_class())
            .send()
            .await
            .map_err(map_sdk_error)?;

        let upload_id = output
            .upload_id()
            .ok_or_else(|| {
                UploadError::Transient("CreateMultipartUpload returned no upload_id".to_string())
            })?
            .to_string();

        Ok((upload_id, Vec::new()))
    }

    /// `>= SIMPLE_UPLOAD_MAX`: `CreateMultipartUpload`/`UploadPart`
    /// ([`PART_SIZE`] parts, [`PART_CONCURRENCY`] concurrent)
    /// `/CompleteMultipartUpload` (SPEC.md §6 "s3.rs"). Resumes from
    /// `req.resume_state` when present and still valid on S3's side
    /// (`start_or_resume_multipart`); persists progress after every
    /// completed part via `self.state_sink` so a crash mid-upload resumes
    /// from the last confirmed part instead of byte zero.
    ///
    /// Cancellation and part failures race in the `select!` loop below,
    /// `biased` toward cancellation so it always wins a tie. A
    /// non-retryable outcome (`Cancelled`, `Auth`, `Permanent`) aborts the
    /// multipart upload immediately — there is no reason to keep paying
    /// for storage on an upload that will not be retried as-is. A
    /// retryable outcome (`Transient`, `Io`) is returned to the caller
    /// *without* aborting: the persisted `upload_id`/parts let a retry
    /// resume instead of restarting a possibly-large file from scratch.
    async fn upload_multipart(
        &self,
        req: &UploadRequest,
        key: &str,
    ) -> Result<UploadResult, UploadError> {
        // VULN-005: S3 caps a multipart upload at 10,000 parts and a single
        // object at 5 TiB. `part_size_for` grows the part so the part count
        // stays inside `MAX_PARTS` for any file up to that object limit, so
        // what remains to check up front is the object limit itself --
        // before any request is made, so an oversized file fails fast with a
        // clear message instead of uploading thousands of parts only to have
        // `CompleteMultipartUpload` reject it from S3's side.
        if req.size > MAX_OBJECT_SIZE {
            return Err(UploadError::Permanent(
                "arquivo excede o limite de 5 TiB por objeto do S3".to_string(),
            ));
        }

        let part_size = part_size_for(req.size);
        debug_assert!(part_size <= MAX_PART_SIZE);
        debug_assert!(req.size.div_ceil(part_size) <= MAX_PARTS);

        let (upload_id, mut completed) = self.start_or_resume_multipart(req, key).await?;
        self.persist_state(&req.remote_name, key, &upload_id, &completed);

        let total_parts = req.size.div_ceil(part_size) as i32;
        let done: std::collections::HashSet<i32> = completed
            .iter()
            .map(|(part_number, _)| *part_number)
            .collect();

        let sent_so_far: u64 = completed
            .iter()
            .map(|(part_number, _)| part_bounds(req.size, *part_number).1)
            .sum();
        let sent = Arc::new(std::sync::atomic::AtomicU64::new(sent_so_far));
        let _ = req.progress.try_send(ProgressUpdate {
            sent: sent.load(std::sync::atomic::Ordering::Relaxed),
            total: req.size,
        });

        let semaphore = Arc::new(Semaphore::new(part_concurrency(part_size)));
        let mut join_set: JoinSet<Result<(i32, String, u64), UploadError>> = JoinSet::new();

        for part_number in 1..=total_parts {
            if done.contains(&part_number) {
                continue;
            }
            let (offset, len) = part_bounds(req.size, part_number);
            let client = self.client.clone();
            let bucket = self.cfg.bucket.clone();
            let key = key.to_string();
            let upload_id_clone = upload_id.clone();
            let path = req.local_path.clone();
            let throttle = self.throttle.clone();
            let semaphore = semaphore.clone();

            join_set.spawn(async move {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .expect("semaphore is never closed");
                let buf = read_part(&path, offset, len, throttle).await?;
                let content_length = buf.len() as i64;
                let output = client
                    .upload_part()
                    .bucket(bucket)
                    .key(key)
                    .upload_id(upload_id_clone)
                    .part_number(part_number)
                    .body(ByteStream::from(buf))
                    .content_length(content_length)
                    .send()
                    .await
                    .map_err(map_sdk_error)?;
                let etag = output
                    .e_tag()
                    .ok_or_else(|| {
                        UploadError::Transient("UploadPart returned no ETag".to_string())
                    })?
                    .to_string();
                Ok((part_number, etag, len))
            });
        }

        let outcome: Result<(), UploadError> = loop {
            if join_set.is_empty() {
                break Ok(());
            }
            tokio::select! {
                biased;
                () = req.cancel.cancelled() => break Err(UploadError::Cancelled),
                next = join_set.join_next() => {
                    match next {
                        None => break Ok(()),
                        Some(Ok(Ok((part_number, etag, len)))) => {
                            completed.push((part_number, etag));
                            sent.fetch_add(len, std::sync::atomic::Ordering::Relaxed);
                            let _ = req.progress.try_send(ProgressUpdate {
                                sent: sent.load(std::sync::atomic::Ordering::Relaxed),
                                total: req.size,
                            });
                            self.persist_state(&req.remote_name, key, &upload_id, &completed);
                        }
                        Some(Ok(Err(part_err))) => break Err(part_err),
                        Some(Err(join_err)) => {
                            break Err(UploadError::Permanent(format!(
                                "part upload task panicked or was aborted: {join_err}"
                            )));
                        }
                    }
                }
            }
        };

        if let Err(err) = outcome {
            join_set.abort_all();
            if !err.is_retryable() {
                if let Err(abort_err) = self.abort_multipart(key, &upload_id).await {
                    tracing::warn!(
                        error = %abort_err,
                        upload_id = %upload_id,
                        "falha ao abortar upload multipart após uma falha não retentável"
                    );
                }
            }
            return Err(err);
        }

        completed.sort_by_key(|(part_number, _)| *part_number);
        let completed_parts: Vec<CompletedPart> = completed
            .iter()
            .map(|(part_number, etag)| {
                CompletedPart::builder()
                    .part_number(*part_number)
                    .e_tag(etag)
                    .build()
            })
            .collect();

        let output = self
            .client
            .complete_multipart_upload()
            .bucket(&self.cfg.bucket)
            .key(key)
            .upload_id(&upload_id)
            .multipart_upload(
                CompletedMultipartUpload::builder()
                    .set_parts(Some(completed_parts))
                    .build(),
            )
            .send()
            .await
            .map_err(map_sdk_error)?;

        Ok(UploadResult {
            remote_id: key.to_string(),
            remote_state: Some(json!({ "etag": output.e_tag() })),
        })
    }

    /// `AbortMultipartUpload`, tolerant of an upload that is already gone
    /// (`NoSuchUpload` — already completed, already aborted, or expired
    /// via a lifecycle rule): crash recovery (SPEC.md §5) and
    /// `upload_multipart`'s own cancel/failure path both call this
    /// expecting cleanup to be a no-op when there is nothing left to
    /// clean up, not a hard failure.
    async fn abort_multipart(&self, key: &str, upload_id: &str) -> Result<(), UploadError> {
        match self
            .client
            .abort_multipart_upload()
            .bucket(&self.cfg.bucket)
            .key(key)
            .upload_id(upload_id)
            .send()
            .await
        {
            Ok(_) => Ok(()),
            Err(SdkError::ServiceError(service_err))
                if service_err.err().code() == Some("NoSuchUpload") =>
            {
                Ok(())
            }
            Err(err) => Err(map_sdk_error(err)),
        }
    }
}

#[async_trait]
impl Uploader for S3Uploader {
    fn id(&self) -> Destination {
        Destination::S3
    }

    /// `head_bucket` (read access) then a zero-byte `put_object` +
    /// `delete_object` round-trip on a dedicated probe key (write access —
    /// SPEC.md §6: "`test_connection`: `head_bucket`"; §9 justifies the
    /// probe's narrow `s3:DeleteObject` grant). A read-only credential
    /// fails at the `put_object` step with `Auth`, which is exactly the
    /// distinction the "Testar conexão" button exists to surface.
    async fn test_connection(&self) -> Result<TestResult, UploadError> {
        let started = Instant::now();
        let probe_key = self.object_key(PROBE_OBJECT_NAME);

        self.client
            .head_bucket()
            .bucket(&self.cfg.bucket)
            .send()
            .await
            .map_err(map_sdk_error)?;

        self.client
            .put_object()
            .bucket(&self.cfg.bucket)
            .key(&probe_key)
            .body(ByteStream::from(Vec::new()))
            .content_length(0)
            .send()
            .await
            .map_err(map_sdk_error)?;

        // Cleaning up the probe is best-effort: `s3:DeleteObject` is NOT required
        // for the app to work (it never deletes user data), and write-only /
        // append-only IAM policies are common for backup buckets. A denied
        // delete must not fail the whole connection test (found in the field:
        // a `PutObject`+`ListBucket`-only policy reported "auth-required").
        let message = match self
            .client
            .delete_object()
            .bucket(&self.cfg.bucket)
            .key(&probe_key)
            .send()
            .await
            .map_err(map_sdk_error)
        {
            Ok(_) => "Bucket válido (Put/List OK)".to_string(),
            Err(UploadError::Auth(_)) | Err(UploadError::Permanent(_)) => {
                tracing::warn!(
                    key = %probe_key,
                    "s3: objeto de sonda não pôde ser excluído (sem s3:DeleteObject); mantendo-o no lugar"
                );
                format!(
                    "Bucket válido (Put/List OK; sem permissão DeleteObject — o arquivo de teste {probe_key} permanece no bucket)"
                )
            }
            Err(other) => return Err(other),
        };

        Ok(TestResult {
            ok: true,
            message,
            latency_ms: started.elapsed().as_millis() as u32,
        })
    }

    async fn upload(&self, req: UploadRequest) -> Result<UploadResult, UploadError> {
        if req.cancel.is_cancelled() {
            return Err(UploadError::Cancelled);
        }

        let key = self.object_key(&req.remote_name);

        if let Some(result) = self.check_existing(&req, &key).await? {
            return Ok(result);
        }

        if req.size >= SIMPLE_UPLOAD_MAX {
            return self.upload_multipart(&req, &key).await;
        }

        self.put_simple(&req, &key).await
    }

    /// Cleans up an interrupted multipart upload that will not be resumed
    /// (SPEC.md §5 "Crash recovery no boot": every recovered job whose
    /// `remote_state` carries an `upload_id` gets `AbortMultipartUpload`'d
    /// on boot). `remote_state.key` makes the call self-contained — no
    /// need to recompute `object_key` from a `remote_name` this method
    /// isn't given. Missing/malformed `remote_state` is a no-op: nothing
    /// to abort.
    async fn abort(&self, remote_state: &serde_json::Value) -> Result<(), UploadError> {
        let (Some(upload_id), Some(key)) = (
            remote_state.get("upload_id").and_then(|v| v.as_str()),
            remote_state.get("key").and_then(|v| v.as_str()),
        ) else {
            return Ok(());
        };
        self.abort_multipart(key, upload_id).await
    }
}

/// `[offset, offset + len)` for `part_number` (1-based) of a `total_size`
/// file split into [`part_size_for`] parts — the last part is whatever
/// remains, which is `<= part_size_for(total_size)` and, per S3's own rule,
/// may be smaller than 5 MiB only when it's the sole/last part.
fn part_bounds(total_size: u64, part_number: i32) -> (u64, u64) {
    let part_size = part_size_for(total_size);
    let offset = (part_number as u64 - 1) * part_size;
    let len = (total_size - offset).min(part_size);
    (offset, len)
}

/// Reads one part's bytes from `path` at `[offset, offset + len)`,
/// throttled through the shared bandwidth limiter like `put_simple`'s
/// whole-file read (SPEC.md §6 `throttle.rs`). Parts are read on demand
/// rather than the whole file up front — multipart exists specifically
/// for files too large to comfortably hold twice in memory.
async fn read_part(
    path: &std::path::Path,
    offset: u64,
    len: u64,
    throttle: Arc<Throttle>,
) -> Result<Vec<u8>, UploadError> {
    let mut file = tokio::fs::File::open(path).await?;
    file.seek(std::io::SeekFrom::Start(offset)).await?;
    let mut reader = ThrottledReader::new(file.take(len), throttle);
    let mut buf = Vec::with_capacity(len as usize);
    reader.read_to_end(&mut buf).await?;
    Ok(buf)
}

/// Collapses any run of `/` into a single `/` — guards against a
/// double slash when `prefix` ends with `/` and `remote_name` doesn't (or
/// vice versa), which S3 would otherwise treat as an empty path segment.
fn collapse_slashes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_slash = false;
    for c in s.chars() {
        if c == '/' {
            if !last_was_slash {
                out.push(c);
            }
            last_was_slash = true;
        } else {
            out.push(c);
            last_was_slash = false;
        }
    }
    out
}

/// Funnels every `SdkError` shape into [`super::classify`]/
/// [`super::classify_transport`], so `S3Uploader` shares one error
/// taxonomy with `gdrive/` instead of inventing an S3-specific one.
///
/// - `ServiceError`: the only case with an actual HTTP response — pulls
///   the status and body off `raw()`, parses the body as S3's XML error
///   shape via [`parse_error_body`], and falls back to the SDK's own
///   `ProvideErrorMetadata::code()/message()` when the body didn't parse
///   (e.g. a truncated response) so classification never regresses to
///   "unrecognised" just because the XML was empty.
/// - `TimeoutError`: the request timed out client-side.
/// - `DispatchFailure`: the request never got a response (connection
///   refused, DNS failure, socket reset).
/// - `ResponseError`: a response arrived but couldn't be parsed —
///   treated as transient, same as any other transport hiccup.
/// - `ConstructionFailure`: a bug in how *we* built the request (bad
///   header value, etc.) — never transient, never the server's fault.
/// - Any other, future variant (the enum is `#[non_exhaustive]`): treated
///   as `Transient` rather than panicking or silently discarding it —
///   fail-secure means the worker gets a chance to retry rather than the
///   job disappearing into an unmatched arm.
fn map_sdk_error<E>(err: SdkError<E>) -> UploadError
where
    E: ProvideErrorMetadata,
{
    match err {
        SdkError::ServiceError(service_err) => {
            let status = service_err.raw().status().as_u16();
            let body_bytes = service_err.raw().body().bytes();
            let mut body = body_bytes.map(parse_error_body).unwrap_or_default();
            if body.code.is_none() {
                body.code = service_err.err().code().map(str::to_string);
            }
            if body.message.is_none() {
                body.message = service_err.err().message().map(str::to_string);
            }
            let retry_after = service_err
                .raw()
                .headers()
                .get("retry-after")
                .and_then(|v| v.parse::<u64>().ok());

            classify(status, Some(&body), ClassifyContext::Api, retry_after).error
        }
        SdkError::TimeoutError(_) => classify_transport(true, false),
        SdkError::DispatchFailure(dispatch) => {
            classify_transport(dispatch.is_timeout(), !dispatch.is_timeout())
        }
        SdkError::ResponseError(_) => classify_transport(false, false),
        SdkError::ConstructionFailure(context) => {
            UploadError::Permanent(format!("request construction failed: {context:?}"))
        }
        other => UploadError::Transient(format!("unrecognised S3 SDK error: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;
    use wiremock::matchers::{body_string_contains, header, method, path, query_param};
    use wiremock::{Mock, MockBuilder, MockServer, Request, ResponseTemplate};

    fn test_cfg(bucket: &str, prefix: &str) -> S3Config {
        S3Config {
            enabled: true,
            region: "us-east-1".to_string(),
            bucket: bucket.to_string(),
            prefix: prefix.to_string(),
            storage_class: StorageClass::Standard,
        }
    }

    fn test_creds() -> AwsCredentials {
        AwsCredentials {
            access_key_id: "AKIATESTACCESSKEY".to_string(),
            secret_access_key: "test-secret-access-key-do-not-log".to_string(),
        }
    }

    async fn uploader_for(server: &MockServer, cfg: S3Config) -> S3Uploader {
        S3Uploader::new(
            cfg,
            test_creds(),
            Throttle::new(0),
            S3Options {
                endpoint_url: Some(server.uri()),
                force_path_style: true,
                skip_head_check: false,
            },
        )
        .await
        .expect("uploader construction never fails without I/O")
    }

    fn s3_error_xml(code: &str, message: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
             <Error><Code>{code}</Code><Message>{message}</Message></Error>"
        )
    }

    fn noop_request(size: u64, sha256: &str) -> (UploadRequest, mpsc::Receiver<ProgressUpdate>) {
        let (tx, rx) = mpsc::channel(8);
        let req = UploadRequest {
            local_path: "/nonexistent/should-not-be-read".into(),
            remote_name: "name.bin".to_string(),
            size,
            sha256: sha256.to_string(),
            progress: tx,
            cancel: CancellationToken::new(),
            resume_state: None,
        };
        (req, rx)
    }

    #[tokio::test]
    async fn test_connection_happy_path_reports_ok_and_latency() {
        let server = MockServer::start().await;
        Mock::given(method("HEAD"))
            .and(path("/bucket/"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/bucket/.osystems-sync-probe"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/bucket/.osystems-sync-probe"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let uploader = uploader_for(&server, test_cfg("bucket", "")).await;
        let result = uploader.test_connection().await.expect("should succeed");

        assert!(result.ok);
        assert_eq!(result.message, "Bucket válido (Put/List OK)");
    }

    #[tokio::test]
    async fn test_connection_head_403_access_denied_is_auth() {
        let server = MockServer::start().await;
        Mock::given(method("HEAD"))
            .and(path("/bucket/"))
            .respond_with(ResponseTemplate::new(403).set_body_raw(
                s3_error_xml("AccessDenied", "Access denied"),
                "application/xml",
            ))
            .mount(&server)
            .await;

        let uploader = uploader_for(&server, test_cfg("bucket", "")).await;
        let err = uploader.test_connection().await.expect_err("should fail");

        assert!(matches!(err, UploadError::Auth(_)), "got {err:?}");
    }

    /// Field regression: a write-only IAM policy (Put/List, no DeleteObject)
    /// must still pass the connection test — the probe cleanup is best-effort.
    #[tokio::test]
    async fn test_connection_without_delete_object_permission_is_still_ok() {
        let server = MockServer::start().await;
        Mock::given(method("HEAD"))
            .and(path("/bucket/"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/bucket/.osystems-sync-probe"))
            .respond_with(ResponseTemplate::new(200).insert_header("ETag", "\"d41d8cd9\""))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/bucket/.osystems-sync-probe"))
            .respond_with(ResponseTemplate::new(403).set_body_raw(
                b"<?xml version=\"1.0\"?><Error><Code>AccessDenied</Code><Message>User is not authorized to perform: s3:DeleteObject</Message></Error>".to_vec(),
                "application/xml",
            ))
            .mount(&server)
            .await;

        let uploader = uploader_for(&server, test_cfg("bucket", "")).await;
        let result = uploader
            .test_connection()
            .await
            .expect("missing DeleteObject must not fail the connection test");

        assert!(result.ok);
        assert!(
            result.message.contains("sem permissão DeleteObject"),
            "got {}",
            result.message
        );
    }

    #[tokio::test]
    async fn test_connection_head_404_no_such_bucket_is_permanent() {
        let server = MockServer::start().await;
        Mock::given(method("HEAD"))
            .and(path("/bucket/"))
            .respond_with(ResponseTemplate::new(404).set_body_raw(
                s3_error_xml("NoSuchBucket", "The specified bucket does not exist"),
                "application/xml",
            ))
            .mount(&server)
            .await;

        let uploader = uploader_for(&server, test_cfg("bucket", "")).await;
        let err = uploader.test_connection().await.expect_err("should fail");

        assert!(matches!(err, UploadError::Permanent(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn test_connection_head_500_is_transient() {
        let server = MockServer::start().await;
        Mock::given(method("HEAD"))
            .and(path("/bucket/"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let uploader = uploader_for(&server, test_cfg("bucket", "")).await;
        let err = uploader.test_connection().await.expect_err("should fail");

        assert!(matches!(err, UploadError::Transient(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn test_connection_connection_refused_is_transient() {
        // Bind then immediately drop a listener to obtain a port nothing is
        // listening on, so the connect fails fast (no need to wait out the
        // 10s connect timeout).
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral port");
        let port = listener.local_addr().expect("local addr").port();
        drop(listener);

        let uploader = S3Uploader::new(
            test_cfg("bucket", ""),
            test_creds(),
            Throttle::new(0),
            S3Options {
                endpoint_url: Some(format!("http://127.0.0.1:{port}")),
                force_path_style: true,
                skip_head_check: false,
            },
        )
        .await
        .expect("uploader construction never fails without I/O");

        let err = uploader.test_connection().await.expect_err("should fail");

        assert!(matches!(err, UploadError::Transient(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn upload_simple_put_object_succeeds_and_reports_progress() {
        let server = MockServer::start().await;
        let data = vec![0xABu8; 1024 * 1024];
        let sha256 = "a".repeat(64);

        let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
        std::io::Write::write_all(&mut tmp, &data).expect("write tempfile");

        Mock::given(method("PUT"))
            .and(path("/bucket/prefix/name.bin"))
            .and(header("x-amz-meta-sha256", sha256.as_str()))
            .respond_with(ResponseTemplate::new(200).insert_header("ETag", "\"deadbeef\""))
            .mount(&server)
            .await;

        let uploader = uploader_for(&server, test_cfg("bucket", "prefix/")).await;
        let (mut req, mut rx) = noop_request(data.len() as u64, &sha256);
        req.local_path = tmp.path().to_path_buf();

        let result = uploader.upload(req).await.expect("upload should succeed");

        assert_eq!(result.remote_id, "prefix/name.bin");

        let mut last = None;
        while let Ok(update) = rx.try_recv() {
            last = Some(update);
        }
        let last = last.expect("at least one progress update");
        assert_eq!(last.sent, data.len() as u64);
        assert_eq!(last.total, data.len() as u64);
    }

    #[tokio::test]
    async fn upload_sets_storage_class_header_when_non_standard() {
        let server = MockServer::start().await;
        let data = vec![0x11u8; 128];
        let sha256 = "b".repeat(64);

        let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
        std::io::Write::write_all(&mut tmp, &data).expect("write tempfile");

        Mock::given(method("PUT"))
            .and(path("/bucket/name.bin"))
            .and(header("x-amz-storage-class", "INTELLIGENT_TIERING"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let mut cfg = test_cfg("bucket", "");
        cfg.storage_class = StorageClass::IntelligentTiering;
        let uploader = uploader_for(&server, cfg).await;
        let (mut req, _rx) = noop_request(data.len() as u64, &sha256);
        req.local_path = tmp.path().to_path_buf();

        uploader
            .upload(req)
            .await
            .expect("upload should succeed (mock only matches the expected storage class header)");
    }

    #[tokio::test]
    async fn upload_cancelled_returns_cancelled_without_making_a_request() {
        let server = MockServer::start().await;
        let uploader = uploader_for(&server, test_cfg("bucket", "")).await;

        let (req, _rx) = noop_request(128, &"c".repeat(64));
        req.cancel.cancel();

        let err = uploader.upload(req).await.expect_err("should be cancelled");
        assert!(matches!(err, UploadError::Cancelled), "got {err:?}");

        assert!(server
            .received_requests()
            .await
            .expect("mock server tracks requests")
            .is_empty());
    }

    /// `check_existing` runs before the multipart dispatch (`upload()`
    /// calls it unconditionally), so every multipart test below mocks a
    /// `HEAD` 404 to fall through — this is that mock, reused everywhere.
    fn mount_head_not_found(_server: &MockServer, bucket: &str, key: &str) -> MockBuilder {
        Mock::given(method("HEAD")).and(path(format!("/{bucket}/{key}")))
    }

    fn sparse_tempfile(size: u64) -> tempfile::NamedTempFile {
        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        tmp.as_file().set_len(size).expect("set_len sparse file");
        tmp
    }

    fn initiate_multipart_xml(bucket: &str, key: &str, upload_id: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
             <InitiateMultipartUploadResult><Bucket>{bucket}</Bucket><Key>{key}</Key><UploadId>{upload_id}</UploadId></InitiateMultipartUploadResult>"
        )
    }

    fn complete_multipart_xml(bucket: &str, key: &str, etag: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
             <CompleteMultipartUploadResult><Bucket>{bucket}</Bucket><Key>{key}</Key><ETag>{etag}</ETag></CompleteMultipartUploadResult>"
        )
    }

    fn list_parts_xml(bucket: &str, key: &str, upload_id: &str, parts: &[(i32, &str)]) -> String {
        let parts_xml: String = parts
            .iter()
            .map(|(number, etag)| {
                format!("<Part><PartNumber>{number}</PartNumber><ETag>{etag}</ETag><Size>{}</Size></Part>", PART_SIZE)
            })
            .collect();
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
             <ListPartsResult><Bucket>{bucket}</Bucket><Key>{key}</Key><UploadId>{upload_id}</UploadId>{parts_xml}</ListPartsResult>"
        )
    }

    // VULN-005: a file past S3's 5 TiB object limit must be rejected up
    // front (`UploadError::Permanent`), before any S3 request is made --
    // `skip_head_check: true` and no mounted mocks prove nothing was called.
    // Below that limit `part_size_for` keeps the part count inside
    // `MAX_PARTS`, so the part count itself can no longer be the trigger.
    #[tokio::test]
    async fn upload_multipart_rejects_files_exceeding_object_limit() {
        let uploader = S3Uploader::new(
            test_cfg("bucket", ""),
            test_creds(),
            Throttle::new(0),
            S3Options {
                endpoint_url: None,
                force_path_style: false,
                skip_head_check: true,
            },
        )
        .await
        .expect("uploader construction never fails without I/O");

        let size = MAX_OBJECT_SIZE + 1; // one byte over S3's 5 TiB object limit
        let (req, _rx) = noop_request(size, &"a".repeat(64));

        let err = uploader
            .upload(req)
            .await
            .expect_err("must reject before starting the multipart upload");
        assert!(
            matches!(err, UploadError::Permanent(_)),
            "expected Permanent, got {err:?}"
        );
    }

    // A file just past `PART_SIZE * MAX_PARTS` (160 GiB) used to be rejected
    // outright; it now scales the part instead, so the layout must stay
    // inside both S3 limits.
    #[test]
    fn part_size_grows_past_the_10_000_part_boundary() {
        assert_eq!(part_size_for(0), PART_SIZE);
        assert_eq!(part_size_for(PART_SIZE * MAX_PARTS), PART_SIZE);

        let over = PART_SIZE * MAX_PARTS + 1;
        let scaled = part_size_for(over);
        assert!(scaled > PART_SIZE, "expected a larger part, got {scaled}");
        assert!(over.div_ceil(scaled) <= MAX_PARTS);
    }

    // Every size up to the object limit must produce a layout S3 accepts:
    // at most `MAX_PARTS` parts, no part above `MAX_PART_SIZE`, and (except
    // for a single-part upload) no part below S3's 5 MiB floor.
    #[test]
    fn part_layout_is_valid_across_the_whole_size_range() {
        const MIB: u64 = 1024 * 1024;
        for size in [
            SIMPLE_UPLOAD_MAX,
            20 * MIB,
            5 * 1024 * MIB,
            PART_SIZE * MAX_PARTS,
            PART_SIZE * MAX_PARTS + 1,
            1024 * 1024 * MIB,
            MAX_OBJECT_SIZE,
        ] {
            let part_size = part_size_for(size);
            assert!(part_size >= 5 * MIB, "part below S3 floor for {size}");
            assert!(part_size <= MAX_PART_SIZE, "part above S3 cap for {size}");
            assert!(
                size.div_ceil(part_size) <= MAX_PARTS,
                "too many parts for {size}"
            );
        }
    }

    // `part_bounds` must tile the file exactly: contiguous, no gap, no
    // overlap, and summing to the total.
    #[test]
    fn part_bounds_tile_a_large_file_exactly() {
        let size = PART_SIZE * MAX_PARTS + 12_345;
        let part_size = part_size_for(size);
        let total_parts = size.div_ceil(part_size) as i32;

        let mut cursor = 0u64;
        for part_number in 1..=total_parts {
            let (offset, len) = part_bounds(size, part_number);
            assert_eq!(offset, cursor, "gap or overlap at part {part_number}");
            cursor += len;
        }
        assert_eq!(cursor, size);
    }

    // RNF-001: `read_part` buffers a whole part, so concurrency must shrink
    // as the part grows or peak RSS would scale without bound.
    #[test]
    fn part_concurrency_respects_the_memory_budget() {
        assert_eq!(part_concurrency(PART_SIZE), PART_CONCURRENCY);
        for part_size in [
            PART_SIZE,
            64 * 1024 * 1024,
            512 * 1024 * 1024,
            MAX_PART_SIZE,
        ] {
            let concurrency = part_concurrency(part_size);
            assert!((1..=PART_CONCURRENCY).contains(&concurrency));
            assert!(
                concurrency == 1 || part_size * concurrency as u64 <= PART_MEMORY_BUDGET,
                "budget blown at part_size={part_size}"
            );
        }
    }

    #[tokio::test]
    async fn upload_multipart_uploads_two_parts_and_reports_progress() {
        let server = MockServer::start().await;
        let size = 20 * 1024 * 1024u64; // 16 MiB + 4 MiB => 2 parts.
        let sha256 = "f".repeat(64);
        let tmp = sparse_tempfile(size);

        mount_head_not_found(&server, "bucket", "name.bin")
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/bucket/name.bin"))
            .and(query_param("uploads", ""))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                initiate_multipart_xml("bucket", "name.bin", "test-upload-id"),
                "application/xml",
            ))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("PUT"))
            .and(path("/bucket/name.bin"))
            .and(query_param("partNumber", "1"))
            .and(query_param("uploadId", "test-upload-id"))
            .respond_with(ResponseTemplate::new(200).insert_header("ETag", "\"part1etag\""))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("PUT"))
            .and(path("/bucket/name.bin"))
            .and(query_param("partNumber", "2"))
            .and(query_param("uploadId", "test-upload-id"))
            .respond_with(ResponseTemplate::new(200).insert_header("ETag", "\"part2etag\""))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/bucket/name.bin"))
            .and(query_param("uploadId", "test-upload-id"))
            .and(body_string_contains("part1etag"))
            .and(body_string_contains("part2etag"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                complete_multipart_xml("bucket", "name.bin", "\"final-etag\""),
                "application/xml",
            ))
            .expect(1)
            .mount(&server)
            .await;

        let sink_calls: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_calls_clone = sink_calls.clone();
        let uploader = uploader_for(&server, test_cfg("bucket", ""))
            .await
            .with_state_sink(Arc::new(move |_remote_name, state| {
                sink_calls_clone.lock().unwrap().push(state);
            }));

        let (mut req, mut rx) = noop_request(size, &sha256);
        req.local_path = tmp.path().to_path_buf();

        let result = uploader
            .upload(req)
            .await
            .expect("multipart upload should succeed");
        assert_eq!(result.remote_id, "name.bin");

        let mut updates = Vec::new();
        while let Ok(update) = rx.try_recv() {
            updates.push(update);
        }
        assert!(
            updates.windows(2).all(|w| w[0].sent <= w[1].sent),
            "progress must be monotonic: {updates:?}"
        );
        let last = updates
            .last()
            .copied()
            .expect("at least one progress update");
        assert_eq!(last.sent, size);
        assert_eq!(last.total, size);

        let calls = sink_calls.lock().unwrap();
        assert!(
            calls.len() >= 3,
            "expected create + 2 part completions, got {}: {calls:?}",
            calls.len()
        );
        let last_state = calls.last().expect("at least one state_sink call");
        assert_eq!(last_state["upload_id"], "test-upload-id");
        assert_eq!(last_state["key"], "name.bin");
        assert_eq!(
            last_state["parts"].as_array().expect("parts array").len(),
            2
        );
    }

    #[tokio::test]
    async fn upload_multipart_cancelled_after_first_part_aborts_and_returns_cancelled() {
        let server = MockServer::start().await;
        let size = 20 * 1024 * 1024u64;
        let sha256 = "1".repeat(64);
        let tmp = sparse_tempfile(size);

        mount_head_not_found(&server, "bucket", "name.bin")
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/bucket/name.bin"))
            .and(query_param("uploads", ""))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                initiate_multipart_xml("bucket", "name.bin", "cancel-upload-id"),
                "application/xml",
            ))
            .mount(&server)
            .await;

        let (req, _rx) = noop_request(size, &sha256);
        let cancel = req.cancel.clone();

        // Cancelling from inside the responder guarantees the token is
        // already set by the time the client task observes part 1's
        // response, so the cancellation branch of `upload_multipart`'s
        // `select!` wins deterministically instead of racing part 2. Part
        // 2 is mocked with a long delay (rather than left unmocked) so it
        // can never resolve — matched or not — before the cancellation
        // branch is polled; without this, an unmocked part 2 request
        // returns a fast unmatched-request 404 that can race the
        // cancellation and get misclassified as a real part failure.
        Mock::given(method("PUT"))
            .and(path("/bucket/name.bin"))
            .and(query_param("partNumber", "1"))
            .respond_with(move |_req: &Request| {
                cancel.cancel();
                ResponseTemplate::new(200).insert_header("ETag", "\"part1etag\"")
            })
            .mount(&server)
            .await;

        Mock::given(method("PUT"))
            .and(path("/bucket/name.bin"))
            .and(query_param("partNumber", "2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("ETag", "\"part2etag\"")
                    .set_delay(Duration::from_secs(5)),
            )
            .mount(&server)
            .await;

        Mock::given(method("DELETE"))
            .and(path("/bucket/name.bin"))
            .and(query_param("uploadId", "cancel-upload-id"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let uploader = uploader_for(&server, test_cfg("bucket", "")).await;
        let mut req = req;
        req.local_path = tmp.path().to_path_buf();

        let err = uploader.upload(req).await.expect_err("should be cancelled");
        assert!(matches!(err, UploadError::Cancelled), "got {err:?}");
    }

    #[tokio::test]
    async fn upload_multipart_resumes_via_list_parts_and_only_uploads_missing_part() {
        let server = MockServer::start().await;
        let size = 20 * 1024 * 1024u64;
        let sha256 = "2".repeat(64);
        let tmp = sparse_tempfile(size);

        mount_head_not_found(&server, "bucket", "name.bin")
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/bucket/name.bin"))
            .and(query_param("uploadId", "resume-upload-id"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                list_parts_xml(
                    "bucket",
                    "name.bin",
                    "resume-upload-id",
                    &[(1, "\"part1etag\"")],
                ),
                "application/xml",
            ))
            .expect(1)
            .mount(&server)
            .await;

        // Part 1 is already done per `ListParts` above — only part 2 may
        // be uploaded. Any request for part 1 would mean the resume
        // verification was ignored.
        Mock::given(method("PUT"))
            .and(path("/bucket/name.bin"))
            .and(query_param("partNumber", "2"))
            .and(query_param("uploadId", "resume-upload-id"))
            .respond_with(ResponseTemplate::new(200).insert_header("ETag", "\"part2etag\""))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/bucket/name.bin"))
            .and(query_param("uploadId", "resume-upload-id"))
            .and(body_string_contains("part1etag"))
            .and(body_string_contains("part2etag"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                complete_multipart_xml("bucket", "name.bin", "\"final-etag\""),
                "application/xml",
            ))
            .expect(1)
            .mount(&server)
            .await;

        let uploader = uploader_for(&server, test_cfg("bucket", "")).await;
        let (mut req, _rx) = noop_request(size, &sha256);
        req.local_path = tmp.path().to_path_buf();
        req.resume_state = Some(json!({
            "upload_id": "resume-upload-id",
            "key": "name.bin",
            "parts": [{"part_number": 1, "etag": "\"part1etag\""}],
        }));

        let result = uploader
            .upload(req)
            .await
            .expect("resumed upload should succeed");
        assert_eq!(result.remote_id, "name.bin");

        // Assert the (never-mounted-for-part-1) requests wiremock did see
        // don't include a duplicate part 1 upload.
        let requests = server
            .received_requests()
            .await
            .expect("mock server tracks requests");
        let part1_uploads = requests
            .iter()
            .filter(|r| {
                r.method == wiremock::http::Method::PUT
                    && r.url
                        .query_pairs()
                        .any(|(k, v)| k == "partNumber" && v == "1")
            })
            .count();
        assert_eq!(
            part1_uploads, 0,
            "part 1 should not be re-uploaded after a verified resume"
        );
    }

    #[tokio::test]
    async fn check_existing_skips_upload_when_head_object_sha256_matches() {
        let server = MockServer::start().await;
        let sha256 = "3".repeat(64);

        Mock::given(method("HEAD"))
            .and(path("/bucket/name.bin"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-amz-meta-sha256", sha256.as_str())
                    .insert_header("ETag", "\"existing-etag\""),
            )
            .mount(&server)
            .await;

        let uploader = uploader_for(&server, test_cfg("bucket", "")).await;
        let (req, _rx) = noop_request(1024, &sha256);

        let result = uploader
            .upload(req)
            .await
            .expect("idempotent hit should succeed");
        assert_eq!(result.remote_id, "name.bin");

        // No PutObject/multipart request should ever have been made.
        let requests = server
            .received_requests()
            .await
            .expect("mock server tracks requests");
        assert!(
            requests
                .iter()
                .all(|r| r.method == wiremock::http::Method::HEAD),
            "expected only the HEAD probe, got {requests:?}"
        );
    }

    #[tokio::test]
    async fn check_existing_uploads_when_head_object_sha256_mismatches() {
        let server = MockServer::start().await;
        let data = vec![0x22u8; 128];
        let sha256 = "4".repeat(64);
        let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
        std::io::Write::write_all(&mut tmp, &data).expect("write tempfile");

        Mock::given(method("HEAD"))
            .and(path("/bucket/name.bin"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-amz-meta-sha256", "0".repeat(64).as_str()),
            )
            .mount(&server)
            .await;

        Mock::given(method("PUT"))
            .and(path("/bucket/name.bin"))
            .and(header("x-amz-meta-sha256", sha256.as_str()))
            .respond_with(ResponseTemplate::new(200).insert_header("ETag", "\"deadbeef\""))
            .expect(1)
            .mount(&server)
            .await;

        let uploader = uploader_for(&server, test_cfg("bucket", "")).await;
        let (mut req, _rx) = noop_request(data.len() as u64, &sha256);
        req.local_path = tmp.path().to_path_buf();

        uploader
            .upload(req)
            .await
            .expect("mismatched hash should fall through to a real upload");
    }

    #[tokio::test]
    async fn check_existing_uploads_when_object_does_not_exist() {
        let server = MockServer::start().await;
        let data = vec![0x33u8; 128];
        let sha256 = "5".repeat(64);
        let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
        std::io::Write::write_all(&mut tmp, &data).expect("write tempfile");

        Mock::given(method("HEAD"))
            .and(path("/bucket/name.bin"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        Mock::given(method("PUT"))
            .and(path("/bucket/name.bin"))
            .respond_with(ResponseTemplate::new(200).insert_header("ETag", "\"deadbeef\""))
            .expect(1)
            .mount(&server)
            .await;

        let uploader = uploader_for(&server, test_cfg("bucket", "")).await;
        let (mut req, _rx) = noop_request(data.len() as u64, &sha256);
        req.local_path = tmp.path().to_path_buf();

        uploader
            .upload(req)
            .await
            .expect("no existing object should fall through to a real upload");
    }

    #[tokio::test]
    async fn abort_tolerates_no_such_upload() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/bucket/name.bin"))
            .and(query_param("uploadId", "gone-upload-id"))
            .respond_with(ResponseTemplate::new(404).set_body_raw(
                s3_error_xml("NoSuchUpload", "The specified upload does not exist"),
                "application/xml",
            ))
            .mount(&server)
            .await;

        let uploader = uploader_for(&server, test_cfg("bucket", "")).await;
        uploader
            .abort(&json!({ "upload_id": "gone-upload-id", "key": "name.bin" }))
            .await
            .expect("abort of an already-gone upload should be Ok");
    }

    #[tokio::test]
    async fn upload_multipart_part_5xx_is_transient_and_does_not_abort() {
        let server = MockServer::start().await;
        let size = 20 * 1024 * 1024u64;
        let sha256 = "6".repeat(64);
        let tmp = sparse_tempfile(size);

        mount_head_not_found(&server, "bucket", "name.bin")
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/bucket/name.bin"))
            .and(query_param("uploads", ""))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                initiate_multipart_xml("bucket", "name.bin", "flaky-upload-id"),
                "application/xml",
            ))
            .mount(&server)
            .await;

        Mock::given(method("PUT"))
            .and(path("/bucket/name.bin"))
            .and(query_param("uploadId", "flaky-upload-id"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let sink_calls: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_calls_clone = sink_calls.clone();
        let uploader = uploader_for(&server, test_cfg("bucket", ""))
            .await
            .with_state_sink(Arc::new(move |_remote_name, state| {
                sink_calls_clone.lock().unwrap().push(state);
            }));
        let (mut req, _rx) = noop_request(size, &sha256);
        req.local_path = tmp.path().to_path_buf();

        let err = uploader
            .upload(req)
            .await
            .expect_err("5xx on UploadPart should fail");
        assert!(matches!(err, UploadError::Transient(_)), "got {err:?}");

        let requests = server
            .received_requests()
            .await
            .expect("mock server tracks requests");
        assert!(
            requests
                .iter()
                .all(|r| r.method != wiremock::http::Method::DELETE),
            "a transient part failure must not abort the multipart upload: {requests:?}"
        );

        // A transient part failure must not wipe the persisted resume
        // state — the caller's `jobs.remote_state` should still carry the
        // upload_id from `CreateMultipartUpload` so a retry can resume via
        // `ListParts` instead of starting over.
        let calls = sink_calls.lock().unwrap();
        let last_state = calls
            .last()
            .expect("at least one state_sink call before the failure");
        assert_eq!(last_state["upload_id"], "flaky-upload-id");
        assert_eq!(last_state["key"], "name.bin");
    }

    #[test]
    fn collapse_slashes_collapses_runs_of_slashes() {
        assert_eq!(collapse_slashes("prefix//name.bin"), "prefix/name.bin");
        assert_eq!(collapse_slashes("prefix/name.bin"), "prefix/name.bin");
        assert_eq!(collapse_slashes("name.bin"), "name.bin");
        assert_eq!(collapse_slashes("a///b"), "a/b");
    }

    /// Exercises `S3Uploader` against a real S3-compatible server (MinIO).
    /// Skipped unless `OSYSTEMS_SYNC_S3_ENDPOINT` is set — CI and everyday
    /// `cargo test` runs never depend on Docker being available; a
    /// dedicated job that starts MinIO sets the env and passes `--ignored`.
    #[tokio::test]
    #[ignore = "requires a running MinIO/S3-compatible server; set OSYSTEMS_SYNC_S3_ENDPOINT + OSYSTEMS_SYNC_S3_{ACCESS_KEY,SECRET_KEY,BUCKET}"]
    async fn upload_and_test_connection_against_real_minio() {
        let Ok(endpoint) = std::env::var("OSYSTEMS_SYNC_S3_ENDPOINT") else {
            return;
        };
        let access_key = std::env::var("OSYSTEMS_SYNC_S3_ACCESS_KEY")
            .expect("OSYSTEMS_SYNC_S3_ACCESS_KEY required when OSYSTEMS_SYNC_S3_ENDPOINT is set");
        let secret_key = std::env::var("OSYSTEMS_SYNC_S3_SECRET_KEY")
            .expect("OSYSTEMS_SYNC_S3_SECRET_KEY required when OSYSTEMS_SYNC_S3_ENDPOINT is set");
        let bucket = std::env::var("OSYSTEMS_SYNC_S3_BUCKET")
            .expect("OSYSTEMS_SYNC_S3_BUCKET required when OSYSTEMS_SYNC_S3_ENDPOINT is set");

        let cfg = test_cfg(&bucket, "osystems-sync-test/");
        let creds = AwsCredentials {
            access_key_id: access_key,
            secret_access_key: secret_key,
        };
        let uploader = S3Uploader::new(
            cfg,
            creds,
            Throttle::new(0),
            S3Options {
                endpoint_url: Some(endpoint),
                force_path_style: true,
                skip_head_check: false,
            },
        )
        .await
        .expect("uploader construction");

        let result = uploader
            .test_connection()
            .await
            .expect("test_connection against a real bucket should succeed");
        assert!(result.ok);

        let data = vec![0x42u8; 4096];
        let sha256 = "e".repeat(64);
        let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
        std::io::Write::write_all(&mut tmp, &data).expect("write tempfile");

        let (mut req, _rx) = noop_request(data.len() as u64, &sha256);
        req.local_path = tmp.path().to_path_buf();

        let upload_result = uploader.upload(req).await.expect("upload should succeed");
        assert_eq!(upload_result.remote_id, "osystems-sync-test/name.bin");
    }
}
