//! Drive REST v3 upload paths (T-4.2 simple / T-4.3 resumable; SPEC.md §6)
//! plus the stateless helpers both they and `mod.rs` share. Split out of
//! `mod.rs` purely for file size — both `impl GDriveUploader` blocks below
//! are the same type as `mod.rs`'s (Rust allows splitting `impl` blocks
//! for one type across files in the same crate).
//!
//! - [`GDriveUploader::upload_simple`] (T-4.2): one `multipart/related`
//!   request — JSON metadata + raw bytes, boundary-delimited by hand
//!   since `reqwest::multipart` builds `multipart/form-data`, not the
//!   `multipart/related` Drive's `uploadType=multipart` requires.
//! - [`GDriveUploader::upload_resumable`] (T-4.3): probe-or-create a
//!   session, then a strictly sequential loop of `PUT` chunks (Drive
//!   resumable sessions don't allow out-of-order/concurrent chunks,
//!   unlike S3 multipart parts) of [`super::CHUNK_SIZE`] bytes each,
//!   persisting `{session_uri, offset, total}` via
//!   [`super::GDriveOptions::state_sink`] after every confirmed chunk so
//!   a crash resumes from the last confirmed byte (SPEC.md §6, §5).
//! - `drive_query`/`parse_range_end`/`map_reqwest_error`/
//!   `classify_response`/`build_multipart_body`: the small stateless
//!   helpers used by both this file and `mod.rs`'s read-only calls,
//!   kept `pub(super)` so `mod.rs` can call them without duplicating.

use std::io::SeekFrom;

use serde_json::{json, Value};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

use crate::throttle::ThrottledReader;

use super::super::{
    classify, classify_transport, ClassifyContext, ProgressUpdate, UploadError, UploadRequest,
    UploadResult,
};
use super::GDriveUploader;

/// Boundary used for the hand-built `multipart/related` body. Fixed
/// (rather than random) is fine: Drive only needs *a* boundary string
/// that doesn't collide with the body content, and JSON metadata plus
/// raw file bytes never legitimately contain this exact token.
const MULTIPART_BOUNDARY: &str = "osystems_sync_boundary_7f3a9c";

/// What a resumable-session probe (`PUT` with `Content-Range: bytes
/// */{total}` and no body) found.
enum ProbeOutcome {
    /// `308` + a `Range` header: upload is partway through: resume at
    /// `offset` (the byte *after* the last confirmed one).
    Resume { offset: u64 },
    /// `200`/`201`: Drive already has the whole file (e.g. the previous
    /// attempt's final `PUT` succeeded but the state write or process
    /// crashed before that could be recorded).
    AlreadyComplete(UploadResult),
    /// `404`/`410`: the session is gone (expired, or never existed) —
    /// start a fresh one.
    Gone,
}

/// Where a resumable upload should pick up: either continue writing
/// chunks to a session ([`ResumeStart::Session`], with the offset to
/// start at) or the file already fully exists ([`ResumeStart::Done`], a
/// probe found `200`/`201` — nothing left to upload).
enum ResumeStart {
    Session(String, u64),
    Done(UploadResult),
}

impl GDriveUploader {
    /// Single `multipart/related` request (T-4.2, SPEC.md §6): JSON
    /// metadata (`name` + `parents`) followed by the file's raw bytes,
    /// streamed through a [`ThrottledReader`](crate::throttle::ThrottledReader)
    /// so simple uploads respect RNF-015 same as resumable ones.
    pub(super) async fn upload_simple(
        &self,
        req: &UploadRequest,
        parent: &str,
    ) -> Result<UploadResult, UploadError> {
        let token = self.tokens.token().await?;
        let file = File::open(&req.local_path).await?;
        let mut reader = ThrottledReader::new(file, self.throttle.clone());
        let mut bytes = Vec::with_capacity(req.size as usize);
        reader.read_to_end(&mut bytes).await?;

        let metadata = json!({ "name": req.remote_name, "parents": [parent] });
        let (content_type, body) = build_multipart_body(&metadata, &bytes);

        let url = format!("{}/drive/v3/files", self.opts.upload_base);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .query(&[
                ("uploadType", "multipart"),
                ("supportsAllDrives", "true"),
                ("fields", "id,webViewLink"),
            ])
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body)
            .send()
            .await
            .map_err(|err| map_reqwest_error(&err))?;

        if !resp.status().is_success() {
            return Err(classify_response(resp, ClassifyContext::Api).await);
        }

        let value: Value = resp.json().await.map_err(|err| map_reqwest_error(&err))?;
        let result = parse_file_result(&value)?;

        let _ = req.progress.try_send(ProgressUpdate {
            sent: req.size,
            total: req.size,
        });

        Ok(result)
    }

    /// Resumable upload (T-4.3, SPEC.md §6): resumes from
    /// `req.resume_state` when present (probing it first — a session can
    /// have expired since it was persisted), otherwise starts a fresh
    /// session; then loops [`super::CHUNK_SIZE`]-byte `PUT`s from the
    /// confirmed offset, persisting state after every one.
    pub(super) async fn upload_resumable(
        &self,
        req: &UploadRequest,
        parent: &str,
    ) -> Result<UploadResult, UploadError> {
        let (session_uri, mut offset) = match self.resume_or_start_session(req, parent).await? {
            ResumeStart::Done(result) => return Ok(result),
            ResumeStart::Session(session_uri, offset) => (session_uri, offset),
        };

        if req.cancel.is_cancelled() {
            return Err(UploadError::Cancelled);
        }

        loop {
            if offset >= req.size {
                // Only reachable if the probe above already reported
                // completion, which returns early — defensive guard
                // against an off-by-one in the loop bounds below.
                return Err(UploadError::Transient(
                    "resumable upload offset reached total without a completion response"
                        .to_string(),
                ));
            }

            let end = (offset + super::CHUNK_SIZE - 1).min(req.size - 1);
            let chunk_len = end - offset + 1;

            let mut file = File::open(&req.local_path).await?;
            file.seek(SeekFrom::Start(offset)).await?;
            let limited = file.take(chunk_len);
            let throttled = ThrottledReader::new(limited, self.throttle.clone());
            let stream = ReaderStream::new(throttled);
            let body = reqwest::Body::wrap_stream(stream);

            let put = self
                .http
                .put(&session_uri)
                .header(reqwest::header::CONTENT_LENGTH, chunk_len.to_string())
                .header(
                    reqwest::header::CONTENT_RANGE,
                    format!("bytes {}-{}/{}", offset, end, req.size),
                )
                .body(body)
                .send();

            let resp = tokio::select! {
                biased;
                () = req.cancel.cancelled() => return Err(UploadError::Cancelled),
                result = put => result.map_err(|err| map_reqwest_error(&err))?,
            };

            match resp.status().as_u16() {
                308 => {
                    let range = resp
                        .headers()
                        .get(reqwest::header::RANGE)
                        .and_then(|v| v.to_str().ok())
                        .and_then(parse_range_end);
                    let new_offset = range.map(|end| end + 1).unwrap_or(end + 1);
                    offset = new_offset;
                    self.persist_state(
                        &req.remote_name,
                        json!({ "session_uri": session_uri, "offset": offset, "total": req.size }),
                    );
                    let _ = req.progress.try_send(ProgressUpdate {
                        sent: offset,
                        total: req.size,
                    });
                }
                200 | 201 => {
                    let value: Value = resp.json().await.map_err(|err| map_reqwest_error(&err))?;
                    let result = parse_file_result(&value)?;
                    self.persist_state(
                        &req.remote_name,
                        json!({ "session_uri": Value::Null, "web_view_link": result.remote_state.as_ref().and_then(|s| s.get("web_view_link").cloned()) }),
                    );
                    let _ = req.progress.try_send(ProgressUpdate {
                        sent: req.size,
                        total: req.size,
                    });
                    return Ok(result);
                }
                401 => return Err(UploadError::Auth(read_error_message(resp).await)),
                429 | 500..=599 => {
                    // State was already persisted after the last
                    // confirmed chunk (or is the caller-supplied resume
                    // state, untouched) — safe to surface as retryable
                    // without losing progress.
                    return Err(classify_response(resp, ClassifyContext::Api).await);
                }
                _ => return Err(classify_response(resp, ClassifyContext::Api).await),
            }
        }
    }

    /// Decides where a resumable upload starts: probes
    /// `req.resume_state.session_uri` if present, falling back to a
    /// fresh session on `404`/`410` (SPEC.md §6). Returns
    /// `(session_uri, offset)` — `offset == 0` for a brand-new session.
    async fn resume_or_start_session(
        &self,
        req: &UploadRequest,
        parent: &str,
    ) -> Result<ResumeStart, UploadError> {
        if let Some(state) = &req.resume_state {
            if let Some(session_uri) = state.get("session_uri").and_then(|v| v.as_str()) {
                match self.probe_session(session_uri, req.size).await? {
                    ProbeOutcome::Resume { offset } => {
                        return Ok(ResumeStart::Session(session_uri.to_string(), offset))
                    }
                    ProbeOutcome::AlreadyComplete(result) => {
                        self.persist_state(&req.remote_name, json!({ "session_uri": Value::Null }));
                        return Ok(ResumeStart::Done(result));
                    }
                    ProbeOutcome::Gone => { /* fall through to start_session below */ }
                }
            }
        }

        let session_uri = self.start_session(req, parent).await?;
        self.persist_state(
            &req.remote_name,
            json!({ "session_uri": session_uri, "offset": 0, "total": req.size }),
        );
        Ok(ResumeStart::Session(session_uri, 0))
    }

    /// `PUT` with `Content-Range: bytes */{total}` and no body — Drive's
    /// documented way to ask "how much of this session do you have?"
    /// without transferring any bytes.
    async fn probe_session(
        &self,
        session_uri: &str,
        total: u64,
    ) -> Result<ProbeOutcome, UploadError> {
        let resp = self
            .http
            .put(session_uri)
            .header(reqwest::header::CONTENT_RANGE, format!("bytes */{total}"))
            .header(reqwest::header::CONTENT_LENGTH, "0")
            .send()
            .await
            .map_err(|err| map_reqwest_error(&err))?;

        match resp.status().as_u16() {
            308 => {
                let offset = resp
                    .headers()
                    .get(reqwest::header::RANGE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(parse_range_end)
                    .map(|end| end + 1)
                    .unwrap_or(0);
                Ok(ProbeOutcome::Resume { offset })
            }
            200 | 201 => {
                let value: Value = resp.json().await.map_err(|err| map_reqwest_error(&err))?;
                Ok(ProbeOutcome::AlreadyComplete(parse_file_result(&value)?))
            }
            404 | 410 => Ok(ProbeOutcome::Gone),
            _ => Err(classify_response(resp, ClassifyContext::Api).await),
        }
    }

    /// Creates a brand-new resumable session (`uploadType=resumable`),
    /// returning the `Location` header the rest of the upload targets.
    async fn start_session(
        &self,
        req: &UploadRequest,
        parent: &str,
    ) -> Result<String, UploadError> {
        let token = self.tokens.token().await?;
        let metadata = json!({ "name": req.remote_name, "parents": [parent] });

        let url = format!("{}/drive/v3/files", self.opts.upload_base);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .query(&[
                ("uploadType", "resumable"),
                ("supportsAllDrives", "true"),
                ("fields", "id,webViewLink"),
            ])
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/json; charset=UTF-8",
            )
            .header("X-Upload-Content-Type", "application/octet-stream")
            .header("X-Upload-Content-Length", req.size.to_string())
            .body(metadata.to_string())
            .send()
            .await
            .map_err(|err| map_reqwest_error(&err))?;

        if !resp.status().is_success() {
            return Err(classify_response(resp, ClassifyContext::Api).await);
        }

        resp.headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                UploadError::Transient(
                    "Drive resumable session response had no Location header".to_string(),
                )
            })
    }

    /// Forwards `remote_state` to [`super::GDriveOptions::state_sink`]
    /// when one is configured (SPEC.md §5 `jobs.remote_state`
    /// persistence) — a no-op otherwise (e.g. in tests that don't wire
    /// one up).
    fn persist_state(&self, remote_name: &str, value: Value) {
        if let Some(sink) = &self.opts.state_sink {
            sink(remote_name, value);
        }
    }
}

/// Builds Drive's `q` filter for "the file named `name` directly inside
/// `folder_id`, not trashed" (SPEC.md §6 idempotency lookup). Escapes `'`
/// and `\` per Drive's query-string syntax (backslash-escaped inside
/// single-quoted string literals) — a name containing either would
/// otherwise break out of the literal and corrupt the query.
pub(super) fn drive_query(name: &str, folder_id: &str) -> String {
    let escape = |s: &str| s.replace('\\', "\\\\").replace('\'', "\\'");
    format!(
        "name = '{}' and '{}' in parents and trashed = false",
        escape(name),
        escape(folder_id)
    )
}

/// Parses the end byte from a `Range: bytes=0-N` (request-side, unused
/// here) or a response's `Range`/`Content-Range`-style `bytes=0-N`
/// value. Drive's `308` responses send `Range: bytes=0-N` (no total) —
/// this pulls out just `N`. Returns `None` on anything that doesn't
/// parse, so a malformed header degrades to "resume from 0" rather than
/// panicking.
pub(super) fn parse_range_end(header: &str) -> Option<u64> {
    let bytes_part = header
        .strip_prefix("bytes=")
        .or_else(|| header.strip_prefix("bytes "))?;
    let end_part = bytes_part.split('-').nth(1)?;
    let end_part = end_part.split('/').next()?;
    end_part.trim().parse().ok()
}

/// Funnels a transport-level `reqwest::Error` (connect/timeout — never
/// reached the server) into the shared taxonomy, same as `s3.rs`'s
/// equivalent for SDK transport errors.
pub(super) fn map_reqwest_error(err: &reqwest::Error) -> UploadError {
    if err.is_timeout() || err.is_connect() {
        return classify_transport(err.is_timeout(), err.is_connect());
    }
    UploadError::Transient(format!("Drive request failed: {err}"))
}

/// Reads a response's status/body/`Retry-After` and funnels it through
/// [`classify`] — the single point every non-2xx Drive response (other
/// than the two callers that special-case `404`/`403` themselves) passes
/// through, exactly mirroring `s3.rs`'s `map_sdk_error`.
pub(super) async fn classify_response(
    resp: reqwest::Response,
    ctx: ClassifyContext,
) -> UploadError {
    let status = resp.status().as_u16();
    let retry_after = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let body = resp.bytes().await.unwrap_or_default();
    let error_body = super::super::parse_error_body(&body);
    classify(status, Some(&error_body), ctx, retry_after).error
}

/// `resp.text()` best-effort, for the one place (`401` on a resumable
/// chunk `PUT`) that reports the raw body as the `Auth` message rather
/// than going through [`classify_response`] — a `401` mid-upload is
/// unambiguous (expired/invalid token) so there's no reason/ambiguity
/// table to consult, just the detail to surface.
async fn read_error_message(resp: reqwest::Response) -> String {
    resp.text()
        .await
        .unwrap_or_else(|_| "unauthorized".to_string())
}

/// Extracts `{id, webViewLink}` from a Drive `File` resource JSON body
/// into an [`UploadResult`] (`remote_state: {"web_view_link": ...}`).
fn parse_file_result(value: &Value) -> Result<UploadResult, UploadError> {
    let id = value
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| UploadError::Transient("Drive upload response had no id".to_string()))?
        .to_string();
    let web_view_link = value.get("webViewLink").and_then(|v| v.as_str());
    Ok(UploadResult {
        remote_id: id,
        remote_state: Some(json!({ "web_view_link": web_view_link })),
    })
}

/// Hand-builds a `multipart/related` body (JSON metadata part + raw
/// bytes part) — `reqwest::multipart` only builds `multipart/form-data`,
/// which Drive's `uploadType=multipart` does not accept. `pub(super)` so
/// `mod.rs`'s zero-byte write probe can reuse it.
pub(super) fn build_multipart_body(metadata: &Value, bytes: &[u8]) -> (String, Vec<u8>) {
    let mut body = Vec::with_capacity(bytes.len() + 512);
    body.extend_from_slice(format!("--{MULTIPART_BOUNDARY}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Type: application/json; charset=UTF-8\r\n\r\n");
    body.extend_from_slice(metadata.to_string().as_bytes());
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{MULTIPART_BOUNDARY}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{MULTIPART_BOUNDARY}--").as_bytes());

    let content_type = format!("multipart/related; boundary={MULTIPART_BOUNDARY}");
    (content_type, body)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use serde_json::json;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::config::GDriveConfig;
    use crate::throttle::Throttle;
    use crate::uploaders::{ProgressUpdate, UploadError, UploadRequest, Uploader};

    use super::super::auth::{parse_service_account, TokenProvider};
    use super::super::{GDriveOptions, GDriveUploader};
    use super::{drive_query, parse_range_end};

    /// Throwaway 2048-bit RSA key, PKCS8 PEM (see `gdrive::auth`'s test
    /// module for provenance) — copied here rather than re-exported so
    /// this suite has no `#[cfg(test)]`-only dependency on `auth`'s
    /// internals.
    const TEST_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7Oyt3p8SoGEJP
bIsMFnQeQmYlR0NasbR3t6yCBKF6w/hzTMYKmWiBhxMoAtDWSpbNBwW0YoyEb9+m
hikW5GxE8wIBA0Xf1bBJuG50YA6wueZg/lDzcnKfEYGeD9VLoQC2fdN3AWvD1C33
UOowbMhZgslWxjHbRfICz989ZhZYB+RO6afajtdi2Fijju9sj+0v9GzhGZpL8RAa
NVjo5Dh6tVDJEOypTBqcx9IkeydD/eOQQA4ruhmsalHQWMfQUPJwyxEoj9dkvuj3
dSQcd4grxhniNPcPEKJ3xNp8JgbGxVVflJhU1Uu3GOMRtApwFuMwuPNQ7Fao0dYv
3DM4Q5CDAgMBAAECggEAGpEK72eyQKnEivGGCgPg8RmPKzNL7CwH7+7I1GAPgP8a
m1L694O3WkhjdaDEp4tzj18I4QS/bIaqmj3cybSaukPYS8go8Nmo3HpbeI7YDB0s
rSRDh0T83POby+TyPnt3bELkBQsCErTiMClejtrN/jQlGSIhQ+b5BTYgC93Bm4ww
7WZir/1E+5T8rqO3Ms3wmdC4dWTOQuDDWK0ljaLOqG0bW+t4eS6XHgn9CPS3DuJt
8wcmVlwPE2UXDkdv9r0DUk3D6FMcxU+2SdXehH/PqwzGk+NHrvn9lBlknF3vscs4
DvwATqvHoystxgI9LC52xIQbO+Uw0i82qUMUShrUTQKBgQDlIixIhRQpTCQGwiKc
/ZiDhO2ALMd5VxGY9xzsM+m2bTNSBy6ZgYEitZvy8txg3C0ExyyaBcqcRLeQ9gN7
DRvx9JEqsapjATfdqBt3JCv8AtjCxky44hkl3dFvjl5zLhu9O44Lcw0+2Y9KmrLz
+psqYkdCjoRUrYXA3pqzUOh9pwKBgQDRLzokZaKzBUGVgewLVp9DBgtDG09cw3m0
xbrVEUig0vt3EtpC/KcRAsRCLoVXlCzZssK+9qlKs63lUIMl29Uqftkm1t9LbDDI
Zgjj51wvM6DekTffx5XK8DEgzISsiBZtLPKyvmwUeVz9jVNb2U0K7n3iOG06T2Ok
ukYRbZAJxQKBgCU/vu8zIynrhNfMa5AV8ds/mtSBcxQYwXWahosnjVDow7UMEdlG
olWgLG/8ZzMf1/m0311Sn7NzwFvCgqJYaTiWR5snMsnRguF32K8vpC7dz5sqXYKY
zvnG66s0+8nBryS+L8NQutCC0baRG5JqJRtoyqjZPk39v4axKXkJKCJ1AoGBAKAD
lGJLLM3sc2K+Y6W4uVM3yF2pAmhfTzYtGuHpurjrK1jGnxcm1VV53E8T7wQzYKuW
xsn1PULbd2Y21Fudcc50AgBn1Z+IPzjMdHiBfk7NG32lcCxKLBd07N++Eq832o/h
FjYM2/g9bhi2htF3xCtcjAcESumT2RElPHwQZ2JRAoGAJMpVd75YW4YrgVdfV1Fl
AY61Vt5BYt7cXfwinvI5mc2BdT3Y7bzVuozxBmqbeo2ZyeCS+JEw3SeF5ckeBazT
Eqz5sfZq1hAiWB5WE9lc7DIUDxHEahm+RQwe4i939Sb5IRD9nvLRfvVFUL1OD/gu
h5sWg6OzGIQ6XQbasKq+W/8=
-----END PRIVATE KEY-----";

    const CLIENT_EMAIL: &str = "sync@my-project.iam.gserviceaccount.com";

    fn sa_json(private_key: &str) -> String {
        json!({
            "type": "service_account",
            "project_id": "my-project",
            "private_key_id": "abc123",
            "private_key": private_key,
            "client_email": CLIENT_EMAIL,
            "client_id": "123456789",
            "token_uri": "https://oauth2.googleapis.com/token",
        })
        .to_string()
    }

    fn valid_sa_json() -> String {
        sa_json(TEST_PRIVATE_KEY)
    }

    async fn mount_token_ok(token_server: &MockServer) {
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "tok-1",
                "expires_in": 3600,
            })))
            .mount(token_server)
            .await;
    }

    async fn token_provider_for(token_server: &MockServer) -> Arc<TokenProvider> {
        let sa = parse_service_account(&valid_sa_json()).expect("valid service account json");
        TokenProvider::new(sa, Some(format!("{}/token", token_server.uri())))
            .await
            .expect("provider construction should succeed")
    }

    fn test_cfg(folder_id: &str) -> GDriveConfig {
        GDriveConfig {
            enabled: true,
            folder_id: folder_id.to_string(),
            ..Default::default()
        }
    }

    async fn uploader_for(
        token_server: &MockServer,
        api_server: &MockServer,
        upload_server: &MockServer,
    ) -> GDriveUploader {
        let tokens = token_provider_for(token_server).await;
        GDriveUploader::new(
            test_cfg("folder123"),
            tokens,
            CLIENT_EMAIL.to_string(),
            Throttle::new(0),
            GDriveOptions {
                api_base: api_server.uri(),
                upload_base: upload_server.uri(),
                state_sink: None,
            },
        )
    }

    async fn mount_no_existing_file(api_server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/drive/v3/files"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "files": [] })))
            .mount(api_server)
            .await;
    }

    fn noop_request(size: u64, sha256: &str) -> (UploadRequest, mpsc::Receiver<ProgressUpdate>) {
        let (tx, rx) = mpsc::channel(1024);
        let req = UploadRequest {
            local_path: PathBuf::from("/nonexistent/should-not-be-read"),
            remote_name: "backup.zip".to_string(),
            size,
            sha256: sha256.to_string(),
            progress: tx,
            cancel: CancellationToken::new(),
            resume_state: None,
        };
        (req, rx)
    }

    fn sparse_tempfile(size: u64) -> tempfile::NamedTempFile {
        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        tmp.as_file().set_len(size).expect("set_len sparse file");
        tmp
    }

    fn drain(rx: &mut mpsc::Receiver<ProgressUpdate>) -> Vec<ProgressUpdate> {
        let mut updates = Vec::new();
        while let Ok(update) = rx.try_recv() {
            updates.push(update);
        }
        updates
    }

    // ---------------------------------------------------------------
    // 1. Simple upload (< 8 MiB)
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn upload_simple_sends_one_multipart_request_and_returns_result() {
        let token_server = MockServer::start().await;
        let api_server = MockServer::start().await;
        let upload_server = MockServer::start().await;
        mount_token_ok(&token_server).await;
        mount_no_existing_file(&api_server).await;

        let size = 1024 * 1024u64; // 1 MiB, well under SIMPLE_UPLOAD_MAX.
        let sha256 = "a".repeat(64);
        let tmp = sparse_tempfile(size);

        Mock::given(method("POST"))
            .and(path("/drive/v3/files"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "file-1",
                "webViewLink": "https://drive.google.com/file/d/file-1/view",
            })))
            .expect(1)
            .mount(&upload_server)
            .await;

        let uploader = uploader_for(&token_server, &api_server, &upload_server).await;
        let (mut req, mut rx) = noop_request(size, &sha256);
        req.local_path = tmp.path().to_path_buf();

        let result = uploader
            .upload(req)
            .await
            .expect("simple upload should succeed");
        assert_eq!(result.remote_id, "file-1");
        assert_eq!(
            result.remote_state.unwrap()["web_view_link"],
            "https://drive.google.com/file/d/file-1/view"
        );

        let updates = drain(&mut rx);
        let last = updates.last().expect("at least one progress update");
        assert_eq!(last.sent, size);
        assert_eq!(last.total, size);
    }

    // ---------------------------------------------------------------
    // 2. Resumable upload (>= 8 MiB), two chunks, happy path
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn upload_resumable_uploads_two_chunks_and_reports_progress() {
        let token_server = MockServer::start().await;
        let api_server = MockServer::start().await;
        let upload_server = MockServer::start().await;
        mount_token_ok(&token_server).await;
        mount_no_existing_file(&api_server).await;

        let size = 20 * 1024 * 1024u64; // 16 MiB + 4 MiB => 2 chunks.
        let sha256 = "b".repeat(64);
        let tmp = sparse_tempfile(size);

        let session_path = "/session/abc";
        Mock::given(method("POST"))
            .and(path("/drive/v3/files"))
            .and(header("X-Upload-Content-Length", "20971520"))
            .respond_with(ResponseTemplate::new(200).insert_header(
                "Location",
                format!("{}{}", upload_server.uri(), session_path),
            ))
            .expect(1)
            .mount(&upload_server)
            .await;

        Mock::given(method("PUT"))
            .and(path(session_path))
            .and(header("Content-Range", "bytes 0-16777215/20971520"))
            .respond_with(ResponseTemplate::new(308).insert_header("Range", "bytes=0-16777215"))
            .expect(1)
            .mount(&upload_server)
            .await;

        Mock::given(method("PUT"))
            .and(path(session_path))
            .and(header("Content-Range", "bytes 16777216-20971519/20971520"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "file-2",
                "webViewLink": "https://drive.google.com/file/d/file-2/view",
            })))
            .expect(1)
            .mount(&upload_server)
            .await;

        let sink_calls: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_calls_clone = sink_calls.clone();
        let tokens = token_provider_for(&token_server).await;
        let uploader = GDriveUploader::new(
            test_cfg("folder123"),
            tokens,
            CLIENT_EMAIL.to_string(),
            Throttle::new(0),
            GDriveOptions {
                api_base: api_server.uri(),
                upload_base: upload_server.uri(),
                state_sink: Some(Arc::new(move |_remote_name, state| {
                    sink_calls_clone.lock().unwrap().push(state);
                })),
            },
        );

        let (mut req, mut rx) = noop_request(size, &sha256);
        req.local_path = tmp.path().to_path_buf();

        let result = uploader
            .upload(req)
            .await
            .expect("resumable upload should succeed");
        assert_eq!(result.remote_id, "file-2");

        let updates = drain(&mut rx);
        let last = updates.last().expect("at least one progress update");
        assert_eq!(last.sent, size);
        assert_eq!(last.total, size);

        let calls = sink_calls.lock().unwrap();
        assert!(
            calls.len() >= 3,
            "expected create + chunk1 + completion, got {calls:?}"
        );
        assert_eq!(calls[0]["offset"], 0);
        assert_eq!(calls[0]["total"], size);
        assert_eq!(calls[1]["offset"], 16777216);
        assert_eq!(
            calls.last().unwrap()["session_uri"],
            serde_json::Value::Null
        );
    }

    // ---------------------------------------------------------------
    // 3. Transient failure mid-chunk, then resume from persisted state
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn upload_resumable_transient_failure_then_resumes_from_persisted_offset() {
        let token_server = MockServer::start().await;
        let api_server = MockServer::start().await;
        let upload_server = MockServer::start().await;
        mount_token_ok(&token_server).await;
        mount_no_existing_file(&api_server).await;

        let size = 40 * 1024 * 1024u64; // 3 chunks: 16 + 16 + 8 MiB.
        let sha256 = "c".repeat(64);
        let tmp = sparse_tempfile(size);

        let session_path = "/session/drop";
        Mock::given(method("POST"))
            .and(path("/drive/v3/files"))
            .respond_with(ResponseTemplate::new(200).insert_header(
                "Location",
                format!("{}{}", upload_server.uri(), session_path),
            ))
            .expect(1)
            .mount(&upload_server)
            .await;

        // First attempt: chunk 1 succeeds (308), chunk 2 fails (503).
        Mock::given(method("PUT"))
            .and(path(session_path))
            .and(header("Content-Range", "bytes 0-16777215/41943040"))
            .respond_with(ResponseTemplate::new(308).insert_header("Range", "bytes=0-16777215"))
            .expect(1)
            .mount(&upload_server)
            .await;

        Mock::given(method("PUT"))
            .and(path(session_path))
            .and(header("Content-Range", "bytes 16777216-33554431/41943040"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&upload_server)
            .await;

        let sink_calls: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_calls_clone = sink_calls.clone();
        let tokens = token_provider_for(&token_server).await;
        let uploader = GDriveUploader::new(
            test_cfg("folder123"),
            tokens,
            CLIENT_EMAIL.to_string(),
            Throttle::new(0),
            GDriveOptions {
                api_base: api_server.uri(),
                upload_base: upload_server.uri(),
                state_sink: Some(Arc::new(move |_remote_name, state| {
                    sink_calls_clone.lock().unwrap().push(state);
                })),
            },
        );

        let (mut req, _rx) = noop_request(size, &sha256);
        req.local_path = tmp.path().to_path_buf();

        let err = uploader
            .upload(req)
            .await
            .expect_err("chunk 2 failure should surface");
        assert!(matches!(err, UploadError::Transient(_)), "got {err:?}");

        let last_state = sink_calls
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("at least one state_sink call");
        assert_eq!(last_state["offset"], 16777216);
        let session_uri = last_state["session_uri"]
            .as_str()
            .expect("session_uri")
            .to_string();
        assert!(session_uri.ends_with(session_path));

        // Second attempt: probe reports 16 MiB confirmed, only the
        // remaining two chunks (2 and 3) are uploaded.
        Mock::given(method("PUT"))
            .and(path(session_path))
            .and(header("Content-Range", "bytes */41943040"))
            .respond_with(ResponseTemplate::new(308).insert_header("Range", "bytes=0-16777215"))
            .expect(1)
            .mount(&upload_server)
            .await;

        Mock::given(method("PUT"))
            .and(path(session_path))
            .and(header("Content-Range", "bytes 16777216-33554431/41943040"))
            .respond_with(ResponseTemplate::new(308).insert_header("Range", "bytes=0-33554431"))
            .expect(1)
            .mount(&upload_server)
            .await;

        Mock::given(method("PUT"))
            .and(path(session_path))
            .and(header("Content-Range", "bytes 33554432-41943039/41943040"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "file-3",
                "webViewLink": "https://drive.google.com/file/d/file-3/view",
            })))
            .expect(1)
            .mount(&upload_server)
            .await;

        let (mut req2, mut rx2) = noop_request(size, &sha256);
        req2.local_path = tmp.path().to_path_buf();
        req2.resume_state =
            Some(json!({ "session_uri": session_uri, "offset": 16777216, "total": size }));

        let result = uploader.upload(req2).await.expect("resume should succeed");
        assert_eq!(result.remote_id, "file-3");
        let updates = drain(&mut rx2);
        let last = updates.last().expect("progress update on resume");
        assert_eq!(last.sent, size);
    }

    // ---------------------------------------------------------------
    // 4. Session gone (410) => starts a brand-new session from 0
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn upload_resumable_session_gone_starts_new_session_from_zero() {
        let token_server = MockServer::start().await;
        let api_server = MockServer::start().await;
        let upload_server = MockServer::start().await;
        mount_token_ok(&token_server).await;
        mount_no_existing_file(&api_server).await;

        let size = 10 * 1024 * 1024u64; // single chunk (< CHUNK_SIZE).
        let sha256 = "d".repeat(64);
        let tmp = sparse_tempfile(size);

        let dead_session_path = "/session/dead";
        Mock::given(method("PUT"))
            .and(path(dead_session_path))
            .and(header("Content-Range", format!("bytes */{size}")))
            .respond_with(ResponseTemplate::new(410))
            .expect(1)
            .mount(&upload_server)
            .await;

        let new_session_path = "/session/new";
        Mock::given(method("POST"))
            .and(path("/drive/v3/files"))
            .respond_with(ResponseTemplate::new(200).insert_header(
                "Location",
                format!("{}{}", upload_server.uri(), new_session_path),
            ))
            .expect(1)
            .mount(&upload_server)
            .await;

        Mock::given(method("PUT"))
            .and(path(new_session_path))
            .and(header(
                "Content-Range",
                format!("bytes 0-{}/{size}", size - 1),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "file-4",
                "webViewLink": "https://drive.google.com/file/d/file-4/view",
            })))
            .expect(1)
            .mount(&upload_server)
            .await;

        let uploader = uploader_for(&token_server, &api_server, &upload_server).await;
        let (mut req, _rx) = noop_request(size, &sha256);
        req.local_path = tmp.path().to_path_buf();
        req.resume_state = Some(json!({
            "session_uri": format!("{}{}", upload_server.uri(), dead_session_path),
            "offset": 4 * 1024 * 1024,
            "total": size,
        }));

        let result = uploader
            .upload(req)
            .await
            .expect("should fall back to a new session");
        assert_eq!(result.remote_id, "file-4");
    }

    // ---------------------------------------------------------------
    // 5. Idempotency: exact checksum match skips upload; mismatch resends
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn upload_skips_when_existing_checksum_matches() {
        let token_server = MockServer::start().await;
        let api_server = MockServer::start().await;
        let upload_server = MockServer::start().await;
        mount_token_ok(&token_server).await;

        let sha256 = "e".repeat(64);
        Mock::given(method("GET"))
            .and(path("/drive/v3/files"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "files": [{
                    "id": "existing-1",
                    "name": "backup.zip",
                    "size": "123",
                    "sha256Checksum": sha256,
                    "webViewLink": "https://drive.google.com/file/d/existing-1/view",
                }]
            })))
            .mount(&api_server)
            .await;

        let uploader = uploader_for(&token_server, &api_server, &upload_server).await;
        let (req, mut rx) = noop_request(123, &sha256);

        let result = uploader
            .upload(req)
            .await
            .expect("idempotent skip should succeed");
        assert_eq!(result.remote_id, "existing-1");
        let state = result.remote_state.expect("remote_state");
        assert_eq!(state["skipped"], true);
        assert_eq!(
            state["web_view_link"],
            "https://drive.google.com/file/d/existing-1/view"
        );

        let updates = drain(&mut rx);
        assert_eq!(updates.last().expect("progress update").sent, 123);

        assert!(
            upload_server
                .received_requests()
                .await
                .expect("recording is on")
                .is_empty(),
            "a checksum match must not touch the upload endpoint at all"
        );
    }

    #[tokio::test]
    async fn upload_resends_when_existing_checksum_differs_or_is_absent() {
        let token_server = MockServer::start().await;
        let api_server = MockServer::start().await;
        let upload_server = MockServer::start().await;
        mount_token_ok(&token_server).await;

        let sha256 = "f".repeat(64);
        Mock::given(method("GET"))
            .and(path("/drive/v3/files"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "files": [{
                    "id": "existing-2",
                    "name": "backup.zip",
                    "size": "1048576",
                    "webViewLink": "https://drive.google.com/file/d/existing-2/view",
                    // no sha256Checksum at all — e.g. a native Google Docs file.
                }]
            })))
            .mount(&api_server)
            .await;

        let size = 1024 * 1024u64;
        let tmp = sparse_tempfile(size);
        Mock::given(method("POST"))
            .and(path("/drive/v3/files"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "file-5",
                "webViewLink": "https://drive.google.com/file/d/file-5/view",
            })))
            .expect(1)
            .mount(&upload_server)
            .await;

        let uploader = uploader_for(&token_server, &api_server, &upload_server).await;
        let (mut req, _rx) = noop_request(size, &sha256);
        req.local_path = tmp.path().to_path_buf();

        let result = uploader
            .upload(req)
            .await
            .expect("should resend when checksum is absent");
        assert_eq!(result.remote_id, "file-5");
    }

    // ---------------------------------------------------------------
    // 6. test_connection: ok / 403 (Auth) / 404 (Permanent)
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_connection_ok_reports_authenticated_and_latency() {
        let token_server = MockServer::start().await;
        let api_server = MockServer::start().await;
        let upload_server = MockServer::start().await;
        mount_token_ok(&token_server).await;

        Mock::given(method("GET"))
            .and(path("/drive/v3/files/folder123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "folder123",
                "name": "Backups",
            })))
            .mount(&api_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/drive/v3/files"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "probe-1" })))
            .mount(&upload_server)
            .await;

        Mock::given(method("DELETE"))
            .and(path("/drive/v3/files/probe-1"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&api_server)
            .await;

        let uploader = uploader_for(&token_server, &api_server, &upload_server).await;
        let result = uploader
            .test_connection()
            .await
            .expect("test_connection should succeed");
        assert!(result.ok);
        assert_eq!(result.message, "Autenticado");
    }

    #[tokio::test]
    async fn test_connection_403_is_auth_and_names_client_email() {
        let token_server = MockServer::start().await;
        let api_server = MockServer::start().await;
        let upload_server = MockServer::start().await;
        mount_token_ok(&token_server).await;

        Mock::given(method("GET"))
            .and(path("/drive/v3/files/folder123"))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({
                "error": { "errors": [{ "reason": "insufficientPermissions" }], "message": "no access" }
            })))
            .mount(&api_server)
            .await;

        let uploader = uploader_for(&token_server, &api_server, &upload_server).await;
        let err = uploader
            .test_connection()
            .await
            .expect_err("403 should be an auth error");
        match err {
            UploadError::Auth(msg) => assert!(
                msg.contains(CLIENT_EMAIL),
                "message should name the client_email: {msg}"
            ),
            other => panic!("expected Auth, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_connection_404_is_permanent() {
        let token_server = MockServer::start().await;
        let api_server = MockServer::start().await;
        let upload_server = MockServer::start().await;
        mount_token_ok(&token_server).await;

        Mock::given(method("GET"))
            .and(path("/drive/v3/files/folder123"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "error": { "errors": [{ "reason": "notFound" }], "message": "not found" }
            })))
            .mount(&api_server)
            .await;

        let uploader = uploader_for(&token_server, &api_server, &upload_server).await;
        let err = uploader
            .test_connection()
            .await
            .expect_err("404 should be permanent");
        assert!(matches!(err, UploadError::Permanent(_)), "got {err:?}");
    }

    // ---------------------------------------------------------------
    // 7. 401 => Auth
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn upload_401_on_idempotency_check_is_auth() {
        let token_server = MockServer::start().await;
        let api_server = MockServer::start().await;
        let upload_server = MockServer::start().await;
        mount_token_ok(&token_server).await;

        Mock::given(method("GET"))
            .and(path("/drive/v3/files"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": { "errors": [{ "reason": "authError" }], "message": "invalid credentials" }
            })))
            .mount(&api_server)
            .await;

        let uploader = uploader_for(&token_server, &api_server, &upload_server).await;
        let (req, _rx) = noop_request(1024, &"0".repeat(64));

        let err = uploader
            .upload(req)
            .await
            .expect_err("401 should be an auth error");
        assert!(matches!(err, UploadError::Auth(_)), "got {err:?}");
    }

    // ---------------------------------------------------------------
    // 8. Unit tests: drive_query / parse_range_end
    // ---------------------------------------------------------------

    #[test]
    fn drive_query_builds_expected_filter() {
        let q = drive_query("backup.zip", "folder123");
        assert_eq!(
            q,
            "name = 'backup.zip' and 'folder123' in parents and trashed = false"
        );
    }

    #[test]
    fn drive_query_escapes_quotes_and_backslashes() {
        let q = drive_query("weird'name\\file.zip", "folder123");
        assert_eq!(
            q,
            "name = 'weird\\'name\\\\file.zip' and 'folder123' in parents and trashed = false"
        );
    }

    #[test]
    fn parse_range_end_reads_response_style_header() {
        assert_eq!(parse_range_end("bytes=0-16777215"), Some(16777215));
    }

    #[test]
    fn parse_range_end_reads_content_range_style_value() {
        assert_eq!(parse_range_end("bytes 0-100/200"), Some(100));
    }

    #[test]
    fn parse_range_end_returns_none_for_malformed_input() {
        assert_eq!(parse_range_end("not-a-range"), None);
    }
}
