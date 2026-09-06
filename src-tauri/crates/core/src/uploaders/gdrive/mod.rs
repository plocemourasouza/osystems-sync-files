//! `uploaders::gdrive` — `GDriveUploader` (T-4.2/4.3/4.4; SPEC.md §6
//! "gdrive/"; PRD.md RF-011/012/013/014/016, RNF-004/015).
//!
//! Mirrors `s3.rs`'s shape (SPEC.md §6): this module owns the
//! [`Uploader`] impl — `id`/`test_connection`/`upload`/`abort` — plus the
//! three read-only Drive calls that decide *which* upload path to take
//! ([`GDriveUploader::check_folder_access`],
//! [`GDriveUploader::probe_write_access`], [`GDriveUploader::find_existing`]);
//! `upload.rs` (T-4.2/4.3) owns the two paths that actually move bytes
//! (`upload_simple`/`upload_resumable`) plus the small stateless helpers
//! they share (`drive_query`, `parse_range_end`, `map_reqwest_error`,
//! `classify_response`). Every provider error funnels through
//! [`classify`]/[`classify_transport`] (`uploaders::error`), same as
//! `s3.rs`'s `map_sdk_error` — this module never invents its own error
//! taxonomy.
//!
//! - **Idempotency** (RF-011, T-4.4): `upload()` calls
//!   [`GDriveUploader::find_existing`] before either upload path — a
//!   `files.list` scoped to `name`+`parent` (SPEC.md §6). A match with an
//!   identical `sha256Checksum` skips the transfer entirely; a match with
//!   a *different* or *absent* checksum falls through to a normal upload
//!   ("reenvia em dúvida" — SPEC.md §6).
//! - **Routing** (RF-012, T-4.2/4.3): files under [`SIMPLE_UPLOAD_MAX`] go
//!   through `upload_simple` (one `multipart/related` request); at or
//!   above it, `upload_resumable` (session + [`CHUNK_SIZE`] chunks,
//!   resumable via `req.resume_state`).
//! - **`test_connection`** (RF-013/014, T-4.4): a `files.get` on the
//!   configured folder — distinguishing "not found" (`Permanent`) from
//!   "not shared with this Service Account" (`Auth`, naming
//!   [`GDriveUploader::client_email`] so the user knows exactly who to
//!   share the folder with) — followed by a create+delete probe of a
//!   zero-byte file, since a read-only Service Account can't otherwise be
//!   told apart from a healthy one (SPEC.md §6, §9).
//! - **`abort`**: a no-op. Drive resumable sessions have no server-side
//!   cancel endpoint; an abandoned one simply expires on its own.

pub mod auth;
pub mod folders;
pub mod upload;

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::config::GDriveConfig;
use crate::state::Destination;
use crate::throttle::Throttle;

use self::auth::TokenProvider;
use self::folders::FolderResolver;

use super::{
    ClassifyContext, ProgressUpdate, TestResult, UploadError, UploadRequest, UploadResult, Uploader,
};

/// Files smaller than this go through [`GDriveUploader::upload_simple`]
/// (one `multipart/related` request). Files at or above it go through
/// [`GDriveUploader::upload_resumable`] (SPEC.md §6: "`>= 8 MB`:
/// resumable, chunk de 16 MB").
pub const SIMPLE_UPLOAD_MAX: u64 = 8 * 1024 * 1024;

/// Resumable upload chunk size (SPEC.md §6: "chunk de 16 MB"). Drive
/// requires every chunk but the last to be a multiple of 256 KiB; 16 MiB
/// (`64 × 256 KiB`) satisfies that while keeping the chunk count small
/// even at RNF-004's 5 GB ceiling (~320 chunks).
pub const CHUNK_SIZE: u64 = 16 * 1024 * 1024;

/// Callback an uploader owner (the worker, T-3.9) supplies via
/// [`GDriveOptions::state_sink`] to persist `jobs.remote_state` (SPEC.md
/// §5) as a resumable upload progresses. Called with
/// `(remote_name, remote_state)` after a session is created, again after
/// every confirmed chunk (`308` + `Range`), and a final time on
/// completion (clearing `session_uri`) — so a crash mid-upload resumes
/// from the last confirmed byte, not byte zero.
pub type StateSink = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

/// Zero-byte file `test_connection` creates then deletes to prove write
/// (not just read) access — a read-only Service Account can't otherwise
/// be told apart from a healthy one (SPEC.md §6, §9).
const PROBE_OBJECT_NAME: &str = ".osystems-sync-probe";

/// How long to wait for the TCP connect (SPEC.md RNF-012: fail fast on a
/// dead network rather than hang). Deliberately *no* total/operation
/// timeout: the throttle (RNF-015) can legitimately stretch a large
/// resumable chunk well past any fixed budget.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Test/deployment-time overrides that production config never sets —
/// `api_base`/`upload_base` point the client at a `wiremock` server
/// instead of `https://www.googleapis.com`(`/upload`). `state_sink` is
/// how the caller persists `remote_state` mid-resumable-upload (SPEC.md
/// §5); `UploadRequest` carries no such callback of its own — it's a
/// `mod.rs` type shared with `s3.rs`, and adding a Drive-only field there
/// would leak this provider's persistence mechanics into the
/// provider-agnostic contract (mirrors `s3.rs`'s `StateSink`/`S3Options`).
pub struct GDriveOptions {
    pub api_base: String,
    pub upload_base: String,
    pub state_sink: Option<StateSink>,
}

impl Default for GDriveOptions {
    fn default() -> Self {
        Self {
            api_base: "https://www.googleapis.com".to_string(),
            upload_base: "https://www.googleapis.com/upload".to_string(),
            state_sink: None,
        }
    }
}

/// `Uploader` implementation backed by the Drive REST v3 API over a plain
/// `reqwest::Client` (no Google API client crate — the surface used here
/// is small enough that hand-rolled requests plus the shared
/// [`classify`]/[`classify_transport`] taxonomy is simpler than pulling
/// in a generated client).
pub struct GDriveUploader {
    http: reqwest::Client,
    tokens: Arc<TokenProvider>,
    cfg: GDriveConfig,
    throttle: Arc<Throttle>,
    opts: GDriveOptions,
    /// The Service Account's `client_email` (RF-010) — never secret, but
    /// still module-private since only `test_connection`'s "share with…"
    /// error message needs it (RNF-015: nothing beyond this ever leaves
    /// `gdrive::auth`).
    client_email: String,
    /// Resolves/creates the `YYYY/MM/DD_backup/` subfolder tree under
    /// `cfg.folder_id` (RF-015, T-5.3) — `None` when
    /// `cfg.date_subfolders` is off, in which case every upload targets
    /// `cfg.folder_id` directly (see [`Self::parent_for_upload`]).
    resolver: Option<FolderResolver>,
}

impl GDriveUploader {
    /// Builds the client. Construction never fails: the only fallible
    /// step is `reqwest::ClientBuilder::build`, and the builder here only
    /// ever sets a connect timeout, which cannot itself cause a build
    /// error on any supported platform.
    pub fn new(
        cfg: GDriveConfig,
        tokens: Arc<TokenProvider>,
        client_email: String,
        throttle: Arc<Throttle>,
        opts: GDriveOptions,
    ) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .expect("reqwest client builder only sets a connect timeout, never fails");

        let resolver = cfg.date_subfolders.then(|| {
            FolderResolver::new(
                http.clone(),
                tokens.clone(),
                cfg.folder_id.clone(),
                opts.api_base.clone(),
            )
        });

        Self {
            http,
            tokens,
            cfg,
            throttle,
            opts,
            client_email,
            resolver,
        }
    }

    /// The parent folder id an upload should target: the resolved
    /// `YYYY/MM/DD_backup/` leaf (RF-015) when `cfg.date_subfolders` is
    /// on, else `cfg.folder_id` directly. Deliberately *not* used by
    /// `test_connection` (`check_folder_access`/`probe_write_access`),
    /// which always probes the configured root folder regardless of
    /// this setting.
    async fn parent_for_upload(&self) -> Result<String, UploadError> {
        match &self.resolver {
            Some(resolver) => resolver.resolve(FolderResolver::today_local()).await,
            None => Ok(self.cfg.folder_id.clone()),
        }
    }

    /// `files.get` on the configured folder (SPEC.md §6
    /// `test_connection`), translating the two outcomes the "Testar
    /// conexão" button needs to tell apart into user-facing messages:
    /// `404` (folder id doesn't exist / was deleted) vs. `403` (exists,
    /// but not shared with this Service Account — names
    /// [`Self::client_email`] so the fix is obvious). Any other failure
    /// (`401`, `5xx`, transport) falls through to the shared classifier.
    async fn check_folder_access(&self) -> Result<(), UploadError> {
        let token = self.tokens.token().await?;
        let url = format!(
            "{}/drive/v3/files/{}",
            self.opts.api_base, self.cfg.folder_id
        );
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .query(&[("fields", "id,name,driveId"), ("supportsAllDrives", "true")])
            .send()
            .await
            .map_err(|err| upload::map_reqwest_error(&err))?;

        match resp.status().as_u16() {
            200..=299 => Ok(()),
            404 => Err(UploadError::Permanent("Pasta não encontrada".to_string())),
            403 => Err(UploadError::Auth(format!(
                "Pasta não compartilhada com a Service Account. Compartilhe com {}",
                self.client_email
            ))),
            _ => Err(upload::classify_response(resp, ClassifyContext::Api).await),
        }
    }

    /// Creates then deletes a zero-byte [`PROBE_OBJECT_NAME`] file in the
    /// configured folder, proving write access (SPEC.md §6, §9) — a
    /// read-only Service Account can read the folder metadata above just
    /// fine but fails here with `Auth`, which is exactly the distinction
    /// "Testar conexão" exists to surface.
    async fn probe_write_access(&self) -> Result<(), UploadError> {
        let token = self.tokens.token().await?;
        let metadata = json!({ "name": PROBE_OBJECT_NAME, "parents": [self.cfg.folder_id] });
        let (content_type, body) = upload::build_multipart_body(&metadata, &[]);

        let create_url = format!("{}/drive/v3/files", self.opts.upload_base);
        let resp = self
            .http
            .post(&create_url)
            .bearer_auth(&token)
            .query(&[
                ("uploadType", "multipart"),
                ("supportsAllDrives", "true"),
                ("fields", "id"),
            ])
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body)
            .send()
            .await
            .map_err(|err| upload::map_reqwest_error(&err))?;

        if !resp.status().is_success() {
            return Err(upload::classify_response(resp, ClassifyContext::Api).await);
        }

        let value: serde_json::Value = resp
            .json()
            .await
            .map_err(|err| upload::map_reqwest_error(&err))?;
        let id = value
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| UploadError::Transient("Drive probe create returned no id".to_string()))?
            .to_string();

        let delete_url = format!("{}/drive/v3/files/{}", self.opts.api_base, id);
        let resp = self
            .http
            .delete(&delete_url)
            .bearer_auth(&token)
            .query(&[("supportsAllDrives", "true")])
            .send()
            .await
            .map_err(|err| upload::map_reqwest_error(&err))?;

        if !resp.status().is_success() {
            return Err(upload::classify_response(resp, ClassifyContext::Api).await);
        }

        Ok(())
    }

    /// `files.list` scoped to `name` + parent folder (SPEC.md §6
    /// idempotency: `q="name='X' and 'folder_id' in parents and
    /// trashed=false"`), returning the first match (Drive doesn't
    /// enforce unique names, but this app only ever creates one file per
    /// `remote_name` per folder). `Ok(None)` covers both "no match" and
    /// "matched but didn't parse" — either way, [`Uploader::upload`]
    /// falls through to a normal upload.
    ///
    /// `parent` is the *resolved* parent (RF-015: the day's
    /// `DD_backup/` folder when `cfg.date_subfolders` is on) — the file
    /// lives there, not necessarily in `cfg.folder_id` directly.
    async fn find_existing(
        &self,
        req: &UploadRequest,
        parent: &str,
    ) -> Result<Option<DriveFile>, UploadError> {
        let token = self.tokens.token().await?;
        let q = upload::drive_query(&req.remote_name, parent);
        let url = format!("{}/drive/v3/files", self.opts.api_base);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .query(&[
                ("q", q.as_str()),
                ("fields", "files(id,name,size,sha256Checksum,webViewLink)"),
                ("supportsAllDrives", "true"),
                ("includeItemsFromAllDrives", "true"),
            ])
            .send()
            .await
            .map_err(|err| upload::map_reqwest_error(&err))?;

        if !resp.status().is_success() {
            return Err(upload::classify_response(resp, ClassifyContext::Api).await);
        }

        let value: serde_json::Value = resp
            .json()
            .await
            .map_err(|err| upload::map_reqwest_error(&err))?;
        let file = value
            .get("files")
            .and_then(|v| v.as_array())
            .and_then(|files| files.first())
            .cloned();

        Ok(file.and_then(|f| serde_json::from_value(f).ok()))
    }
}

/// The subset of a Drive `File` resource this module reads (SPEC.md §6
/// idempotency `fields=files(id,name,size,sha256Checksum,webViewLink)`).
/// `name`/`size` are requested (cheaper to over-fetch than to issue a
/// second call later) but unused here — only `id`, `sha256_checksum` and
/// `web_view_link` drive a decision.
#[derive(Debug, Clone, Deserialize)]
struct DriveFile {
    id: String,
    #[serde(rename = "sha256Checksum")]
    sha256_checksum: Option<String>,
    #[serde(rename = "webViewLink")]
    web_view_link: Option<String>,
}

#[async_trait]
impl Uploader for GDriveUploader {
    fn id(&self) -> Destination {
        Destination::GDrive
    }

    async fn test_connection(&self) -> Result<TestResult, UploadError> {
        let started = Instant::now();
        self.check_folder_access().await?;
        self.probe_write_access().await?;
        Ok(TestResult {
            ok: true,
            message: "Autenticado".to_string(),
            latency_ms: started.elapsed().as_millis() as u32,
        })
    }

    /// Idempotency check ([`Self::find_existing`]) first, then routes to
    /// [`upload::GDriveUploader::upload_simple`]/`upload_resumable` by
    /// size (SPEC.md §6).
    async fn upload(&self, req: UploadRequest) -> Result<UploadResult, UploadError> {
        if req.cancel.is_cancelled() {
            return Err(UploadError::Cancelled);
        }

        let parent = self.parent_for_upload().await?;

        if let Some(existing) = self.find_existing(&req, &parent).await? {
            if existing.sha256_checksum.as_deref() == Some(req.sha256.as_str()) {
                let _ = req.progress.try_send(ProgressUpdate {
                    sent: req.size,
                    total: req.size,
                });
                return Ok(UploadResult {
                    remote_id: existing.id,
                    remote_state: Some(json!({
                        "web_view_link": existing.web_view_link,
                        "skipped": true,
                    })),
                });
            }
            // Present but the hash mismatched, or Drive didn't return one
            // at all (e.g. a Google Docs native file) — resend rather
            // than trust an ambiguous match (SPEC.md §6 "reenvia em
            // dúvida").
        }

        if req.size < SIMPLE_UPLOAD_MAX {
            self.upload_simple(&req, &parent).await
        } else {
            self.upload_resumable(&req, &parent).await
        }
    }

    /// Drive resumable sessions have no server-side cancel endpoint — an
    /// abandoned one simply expires on its own after a week of
    /// inactivity — so there is nothing to clean up here, unlike S3's
    /// `AbortMultipartUpload`.
    async fn abort(&self, _remote_state: &serde_json::Value) -> Result<(), UploadError> {
        Ok(())
    }
}
