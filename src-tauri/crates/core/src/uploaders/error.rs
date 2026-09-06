//! Error classification shared by every [`super::Uploader`] implementation
//! (SPEC.md §6 "Classificação de erros"; PRD.md RF-032, §8 risks).
//!
//! The worker (T-3.5) never inspects HTTP status codes or provider error
//! bodies itself — every provider funnels its failure through
//! [`classify`] (HTTP responses) or [`classify_transport`] (network-level
//! failures, before an HTTP response even exists) so the retry/pause
//! behaviour in `worker.rs` stays provider-agnostic: it only ever looks at
//! [`UploadError::is_retryable`], [`UploadError::code`] and
//! [`Classified::retry_after`].
//!
//! **Fail-secure default (PRD.md §8):** an unrecognised 403 reason is
//! classified as [`UploadError::Auth`], never [`UploadError::Permanent`] or
//! [`UploadError::Transient`] — a destination we can't positively identify
//! as "just rate-limited" is treated as "needs reauthentication" (pauses
//! the destination, RF-032) rather than silently retried forever or
//! silently marked failed.

use std::time::Duration;

use serde::Deserialize;

/// Why a request was made, needed to disambiguate a bare `400` response:
/// Google's OAuth2 token endpoint uses `400 invalid_grant` to mean "the
/// service account JWT is invalid — likely clock skew", while every other
/// API on both providers uses `400` for a plain bad request (`Permanent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassifyContext {
    /// The request that failed was a token exchange (`POST
    /// https://oauth2.googleapis.com/token`, SPEC.md §6 `gdrive/auth.rs`).
    TokenEndpoint,
    /// Any other provider API call (S3 REST, Drive `files.*`, …).
    Api,
}

/// Loosely-typed provider error body, tolerant to the three shapes
/// [`parse_error_body`] understands. Only the fields [`classify`] needs to
/// make a decision are kept — the raw `message` is for logs, never shown
/// to the user (Skill 3: generic errors to the client).
#[derive(Debug, Default, Clone, Deserialize)]
pub struct ErrorBody {
    /// Google Drive's `error.errors[0].reason` (e.g. `"forbidden"`,
    /// `"rateLimitExceeded"`).
    pub reason: Option<String>,
    /// AWS S3's `<Code>` (e.g. `"AccessDenied"`) or Google's token-endpoint
    /// `error` field (e.g. `"invalid_grant"`) — two providers, same slot,
    /// because [`classify`] never needs to tell them apart: both are a
    /// short machine-readable code and nothing in the classification table
    /// checks `reason` and `code` differently.
    pub code: Option<String>,
    /// Human-readable detail (Drive's `error.message`, the token
    /// endpoint's `error_description`, S3's `<Message>`). Server-side
    /// logging only.
    pub message: Option<String>,
}

/// Parses an error response body, tolerant to the three shapes emitted by
/// the two providers this app talks to (SPEC.md §6):
///
/// - Google API: `{ "error": { "errors": [ { "reason": "…" } ], "message": "…" } }`
/// - Google OAuth2 token endpoint: `{ "error": "invalid_grant", "error_description": "…" }`
/// - AWS S3 XML: `<Error><Code>AccessDenied</Code><Message>…</Message></Error>`
///
/// Any input that matches none of the three (empty body, unrelated JSON,
/// truncated response) yields [`ErrorBody::default`] — [`classify`] then
/// falls back to the status-code-only rules, never panics on malformed
/// input.
pub fn parse_error_body(bytes: &[u8]) -> ErrorBody {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) {
        if let Some(error) = value.get("error") {
            match error {
                serde_json::Value::Object(_) => {
                    let reason = error
                        .get("errors")
                        .and_then(|errors| errors.as_array())
                        .and_then(|errors| errors.first())
                        .and_then(|first| first.get("reason"))
                        .and_then(|reason| reason.as_str())
                        .map(String::from);
                    let message = error
                        .get("message")
                        .and_then(|message| message.as_str())
                        .map(String::from);
                    return ErrorBody {
                        reason,
                        code: None,
                        message,
                    };
                }
                serde_json::Value::String(code) => {
                    let message = value
                        .get("error_description")
                        .and_then(|d| d.as_str())
                        .map(String::from);
                    return ErrorBody {
                        reason: None,
                        code: Some(code.clone()),
                        message,
                    };
                }
                _ => {}
            }
        }
        // Parsed as JSON but didn't match either known shape.
        return ErrorBody::default();
    }

    parse_xml_error(bytes)
}

/// Cheap substring extraction of AWS's `<Error><Code>…</Code><Message>…</Message></Error>`.
/// No XML crate: the two tags we need never nest or repeat in S3's error
/// body, so a `find`/`find` pair is enough and avoids pulling in a parser
/// dependency for one call site.
fn parse_xml_error(bytes: &[u8]) -> ErrorBody {
    let text = String::from_utf8_lossy(bytes);
    ErrorBody {
        reason: None,
        code: extract_xml_tag(&text, "Code"),
        message: extract_xml_tag(&text, "Message"),
    }
}

fn extract_xml_tag(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.find(&open)? + open.len();
    let end = start + text[start..].find(&close)?;
    Some(text[start..end].trim().to_string())
}

/// Classification result: the [`UploadError`] the worker should act on,
/// plus an optional retry delay override for cases where the provider
/// tells us (or SPEC.md §6 hardcodes) a specific backoff — quota errors
/// use a fixed 1 h wait instead of the default exponential backoff
/// (`retry.base_delay_seconds`, RF-086).
#[derive(Debug)]
pub struct Classified {
    pub error: UploadError,
    pub retry_after: Option<Duration>,
}

impl Classified {
    fn auth(message: String) -> Self {
        Self {
            error: UploadError::Auth(message),
            retry_after: None,
        }
    }

    fn transient(message: String) -> Self {
        Self {
            error: UploadError::Transient(message),
            retry_after: None,
        }
    }

    fn transient_after(message: String, retry_after: Duration) -> Self {
        Self {
            error: UploadError::Transient(message),
            retry_after: Some(retry_after),
        }
    }

    fn permanent(message: String) -> Self {
        Self {
            error: UploadError::Permanent(message),
            retry_after: None,
        }
    }
}

/// Quota/rate-limit reasons on a Drive `403` — retried, not treated as an
/// auth failure (PRD.md §8 "Cota do Drive").
const QUOTA_403: &[&str] = &[
    "rateLimitExceeded",
    "userRateLimitExceeded",
    "storageQuotaExceeded",
    "dailyLimitExceeded",
];

/// Permission-denied reasons on a `403`, across both providers — always
/// `Auth`, never retried (RF-032).
const AUTH_403: &[&str] = &[
    "forbidden",
    "insufficientPermissions",
    "insufficientFilePermissions",
    "AccessDenied",
    "InvalidAccessKeyId",
    "SignatureDoesNotMatch",
];

/// True if the body's `reason` or `code` (whichever the provider used,
/// see [`ErrorBody::code`]) is one of `set`.
fn reason_or_code_in(body: Option<&ErrorBody>, set: &[&str]) -> bool {
    let Some(body) = body else {
        return false;
    };
    body.reason.as_deref().is_some_and(|r| set.contains(&r))
        || body.code.as_deref().is_some_and(|c| set.contains(&c))
}

/// Classifies an HTTP-level failure into an [`UploadError`] plus retry
/// hint, per SPEC.md §6's table. `retry_after_header` is the provider's
/// `Retry-After` (seconds), when the caller parsed one off a `429`
/// response; pass `None` when there isn't one.
pub fn classify(
    status: u16,
    body: Option<&ErrorBody>,
    ctx: ClassifyContext,
    retry_after_header: Option<u64>,
) -> Classified {
    let detail = body
        .and_then(|b| b.message.clone())
        .unwrap_or_else(|| format!("HTTP {status}"));

    match status {
        401 => Classified::auth(format!("401 unauthorized: {detail}")),

        403 if reason_or_code_in(body, QUOTA_403) => {
            Classified::transient_after(format!("403 quota exceeded: {detail}"), Duration::from_secs(3600))
        }
        403 if reason_or_code_in(body, AUTH_403) => Classified::auth(format!("403 forbidden: {detail}")),
        // Any 403 reason not in either list above lands here too —
        // fail-secure default, see module docs.
        403 => Classified::auth(format!("403 forbidden (unrecognised reason): {detail}")),

        400 if ctx == ClassifyContext::TokenEndpoint && reason_or_code_in(body, &["invalid_grant"]) => {
            Classified::auth(format!(
                "400 invalid_grant from token endpoint: {detail} [hint: clock skew — check the system clock]"
            ))
        }

        408 => Classified::transient(format!("408 request timeout: {detail}")),

        429 => Classified::transient_after(
            format!("429 too many requests: {detail}"),
            Duration::from_secs(retry_after_header.unwrap_or(60)),
        ),

        500..=599 => Classified::transient(format!("{status} server error: {detail}")),

        400..=499 => Classified::permanent(format!("{status} client error: {detail}")),

        other => Classified::permanent(format!("{other} unexpected status: {detail}")),
    }
}

/// Classifies a transport-level failure — one that never produced an HTTP
/// response at all (connect refused, DNS failure, request/read timeout).
/// Always [`UploadError::Transient`] (SPEC.md §6: "timeout … rede").
/// `reqwest` isn't a dependency of this crate yet (S3/Drive HTTP clients
/// land in later tasks); callers that do depend on it pass in
/// `error.is_timeout()` / `error.is_connect()` instead of the error value
/// itself, keeping this module decoupled from any particular HTTP client.
pub fn classify_transport(is_timeout: bool, is_connect: bool) -> UploadError {
    if is_timeout {
        UploadError::Transient("request timed out".to_string())
    } else if is_connect {
        UploadError::Transient("connection failed".to_string())
    } else {
        UploadError::Transient("transport error".to_string())
    }
}

/// Uniform failure type for every [`super::Uploader`] method (SPEC.md §6).
///
/// The four HTTP-derived variants (`Auth`/`Transient`/`Permanent`/`Io`)
/// carry a server-log-only message (never shown to the user verbatim —
/// Skill 3: generic errors to the client); `Cancelled` carries none since
/// it's an expected outcome of [`super::UploadRequest::cancel`], not a
/// failure to report.
#[derive(thiserror::Error, Debug)]
pub enum UploadError {
    /// Not retried. Marks the destination `auth-required` and pauses its
    /// jobs (RF-032).
    #[error("auth: {0}")]
    Auth(String),
    /// Retried with backoff, up to `retry.max_attempts` (RF-086).
    #[error("transient: {0}")]
    Transient(String),
    /// Not retried. Job goes straight to `failed` (RF-032).
    #[error("permanent: {0}")]
    Permanent(String),
    /// Local filesystem failure (e.g. antivirus lock). Retried like
    /// `Transient`, same attempt cap (PRD.md §8 "Antivírus corporativo").
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// The caller's [`super::UploadRequest::cancel`] token fired. Not a
    /// failure — the worker records the job as `cancelled`, doesn't retry,
    /// and never surfaces this as an error to the user.
    #[error("cancelled")]
    Cancelled,
}

impl UploadError {
    /// Whether the worker should schedule a retry (RF-032: `Transient` and
    /// `Io` do; `Auth`, `Permanent` and `Cancelled` are all terminal for
    /// the current attempt).
    pub fn is_retryable(&self) -> bool {
        matches!(self, UploadError::Transient(_) | UploadError::Io(_))
    }

    /// Stable machine-readable tag, e.g. for the `last_error` column and
    /// structured logs (`{ action, tenantId, userId, error, duration }`).
    pub fn code(&self) -> &'static str {
        match self {
            UploadError::Auth(_) => "auth",
            UploadError::Transient(_) => "transient",
            UploadError::Permanent(_) => "permanent",
            UploadError::Io(_) => "io",
            UploadError::Cancelled => "cancelled",
        }
    }

    /// Extracts an actionable hint embedded in the message by
    /// [`classify`] (currently only the clock-skew hint on a token-endpoint
    /// `invalid_grant`), formatted as `"… [hint: <text>]"`. `None` for
    /// every other error, including `Io`/`Cancelled` which never carry one.
    pub fn hint(&self) -> Option<&str> {
        let message = match self {
            UploadError::Auth(m) | UploadError::Transient(m) | UploadError::Permanent(m) => {
                m.as_str()
            }
            UploadError::Io(_) | UploadError::Cancelled => return None,
        };
        let start = message.find("[hint: ")? + "[hint: ".len();
        let end = start + message[start..].find(']')?;
        Some(&message[start..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(reason: Option<&str>, code: Option<&str>) -> ErrorBody {
        ErrorBody {
            reason: reason.map(String::from),
            code: code.map(String::from),
            message: Some("detail".to_string()),
        }
    }

    /// One row per SPEC.md §6 classification rule (≥ 18 cases): status,
    /// optional body, context, optional `Retry-After` header → expected
    /// `code()`, `is_retryable()` and whether a `retry_after` is set.
    struct Case {
        name: &'static str,
        status: u16,
        body: Option<ErrorBody>,
        ctx: ClassifyContext,
        retry_after_header: Option<u64>,
        expect_code: &'static str,
        expect_retryable: bool,
        expect_retry_after: Option<Duration>,
    }

    #[test]
    fn classify_table() {
        let cases = vec![
            Case {
                name: "401 -> auth",
                status: 401,
                body: None,
                ctx: ClassifyContext::Api,
                retry_after_header: None,
                expect_code: "auth",
                expect_retryable: false,
                expect_retry_after: None,
            },
            Case {
                name: "403 drive forbidden (reason) -> auth",
                status: 403,
                body: Some(body(Some("forbidden"), None)),
                ctx: ClassifyContext::Api,
                retry_after_header: None,
                expect_code: "auth",
                expect_retryable: false,
                expect_retry_after: None,
            },
            Case {
                name: "403 drive insufficientPermissions (reason) -> auth",
                status: 403,
                body: Some(body(Some("insufficientPermissions"), None)),
                ctx: ClassifyContext::Api,
                retry_after_header: None,
                expect_code: "auth",
                expect_retryable: false,
                expect_retry_after: None,
            },
            Case {
                name: "403 drive insufficientFilePermissions (reason) -> auth",
                status: 403,
                body: Some(body(Some("insufficientFilePermissions"), None)),
                ctx: ClassifyContext::Api,
                retry_after_header: None,
                expect_code: "auth",
                expect_retryable: false,
                expect_retry_after: None,
            },
            Case {
                name: "403 s3 AccessDenied (code) -> auth",
                status: 403,
                body: Some(body(None, Some("AccessDenied"))),
                ctx: ClassifyContext::Api,
                retry_after_header: None,
                expect_code: "auth",
                expect_retryable: false,
                expect_retry_after: None,
            },
            Case {
                name: "403 s3 InvalidAccessKeyId (code) -> auth",
                status: 403,
                body: Some(body(None, Some("InvalidAccessKeyId"))),
                ctx: ClassifyContext::Api,
                retry_after_header: None,
                expect_code: "auth",
                expect_retryable: false,
                expect_retry_after: None,
            },
            Case {
                name: "403 s3 SignatureDoesNotMatch (code) -> auth",
                status: 403,
                body: Some(body(None, Some("SignatureDoesNotMatch"))),
                ctx: ClassifyContext::Api,
                retry_after_header: None,
                expect_code: "auth",
                expect_retryable: false,
                expect_retry_after: None,
            },
            Case {
                name: "403 drive rateLimitExceeded -> transient 1h",
                status: 403,
                body: Some(body(Some("rateLimitExceeded"), None)),
                ctx: ClassifyContext::Api,
                retry_after_header: None,
                expect_code: "transient",
                expect_retryable: true,
                expect_retry_after: Some(Duration::from_secs(3600)),
            },
            Case {
                name: "403 drive userRateLimitExceeded -> transient 1h",
                status: 403,
                body: Some(body(Some("userRateLimitExceeded"), None)),
                ctx: ClassifyContext::Api,
                retry_after_header: None,
                expect_code: "transient",
                expect_retryable: true,
                expect_retry_after: Some(Duration::from_secs(3600)),
            },
            Case {
                name: "403 drive storageQuotaExceeded -> transient 1h",
                status: 403,
                body: Some(body(Some("storageQuotaExceeded"), None)),
                ctx: ClassifyContext::Api,
                retry_after_header: None,
                expect_code: "transient",
                expect_retryable: true,
                expect_retry_after: Some(Duration::from_secs(3600)),
            },
            Case {
                name: "403 drive dailyLimitExceeded -> transient 1h",
                status: 403,
                body: Some(body(Some("dailyLimitExceeded"), None)),
                ctx: ClassifyContext::Api,
                retry_after_header: None,
                expect_code: "transient",
                expect_retryable: true,
                expect_retry_after: Some(Duration::from_secs(3600)),
            },
            Case {
                name: "403 unknown reason -> auth (fail-secure)",
                status: 403,
                body: Some(body(Some("somethingNew"), None)),
                ctx: ClassifyContext::Api,
                retry_after_header: None,
                expect_code: "auth",
                expect_retryable: false,
                expect_retry_after: None,
            },
            Case {
                name: "403 no body at all -> auth (fail-secure)",
                status: 403,
                body: None,
                ctx: ClassifyContext::Api,
                retry_after_header: None,
                expect_code: "auth",
                expect_retryable: false,
                expect_retry_after: None,
            },
            Case {
                name: "400 invalid_grant at token endpoint -> auth + clock skew hint",
                status: 400,
                body: Some(body(None, Some("invalid_grant"))),
                ctx: ClassifyContext::TokenEndpoint,
                retry_after_header: None,
                expect_code: "auth",
                expect_retryable: false,
                expect_retry_after: None,
            },
            Case {
                name: "400 invalid_grant at a plain API call -> permanent (context matters)",
                status: 400,
                body: Some(body(None, Some("invalid_grant"))),
                ctx: ClassifyContext::Api,
                retry_after_header: None,
                expect_code: "permanent",
                expect_retryable: false,
                expect_retry_after: None,
            },
            Case {
                name: "400 plain bad request -> permanent",
                status: 400,
                body: None,
                ctx: ClassifyContext::Api,
                retry_after_header: None,
                expect_code: "permanent",
                expect_retryable: false,
                expect_retry_after: None,
            },
            Case {
                name: "404 NoSuchBucket -> permanent",
                status: 404,
                body: Some(body(None, Some("NoSuchBucket"))),
                ctx: ClassifyContext::Api,
                retry_after_header: None,
                expect_code: "permanent",
                expect_retryable: false,
                expect_retry_after: None,
            },
            Case {
                name: "408 request timeout -> transient",
                status: 408,
                body: None,
                ctx: ClassifyContext::Api,
                retry_after_header: None,
                expect_code: "transient",
                expect_retryable: true,
                expect_retry_after: None,
            },
            Case {
                name: "429 without Retry-After -> transient, default backoff",
                status: 429,
                body: None,
                ctx: ClassifyContext::Api,
                retry_after_header: None,
                expect_code: "transient",
                expect_retryable: true,
                expect_retry_after: Some(Duration::from_secs(60)),
            },
            Case {
                name: "429 with Retry-After: 120 -> transient, honours header",
                status: 429,
                body: None,
                ctx: ClassifyContext::Api,
                retry_after_header: Some(120),
                expect_code: "transient",
                expect_retryable: true,
                expect_retry_after: Some(Duration::from_secs(120)),
            },
            Case {
                name: "500 -> transient",
                status: 500,
                body: None,
                ctx: ClassifyContext::Api,
                retry_after_header: None,
                expect_code: "transient",
                expect_retryable: true,
                expect_retry_after: None,
            },
            Case {
                name: "503 -> transient",
                status: 503,
                body: None,
                ctx: ClassifyContext::Api,
                retry_after_header: None,
                expect_code: "transient",
                expect_retryable: true,
                expect_retry_after: None,
            },
            Case {
                name: "other 4xx (422) -> permanent",
                status: 422,
                body: None,
                ctx: ClassifyContext::Api,
                retry_after_header: None,
                expect_code: "permanent",
                expect_retryable: false,
                expect_retry_after: None,
            },
        ];

        assert!(cases.len() >= 18, "expected a table of at least 18 rows");

        for case in cases {
            let classified = classify(
                case.status,
                case.body.as_ref(),
                case.ctx,
                case.retry_after_header,
            );
            assert_eq!(
                classified.error.code(),
                case.expect_code,
                "case `{}`: wrong code",
                case.name
            );
            assert_eq!(
                classified.error.is_retryable(),
                case.expect_retryable,
                "case `{}`: wrong is_retryable",
                case.name
            );
            assert_eq!(
                classified.retry_after, case.expect_retry_after,
                "case `{}`: wrong retry_after",
                case.name
            );
        }
    }

    #[test]
    fn classify_token_endpoint_invalid_grant_carries_clock_skew_hint() {
        let body = body(None, Some("invalid_grant"));
        let classified = classify(400, Some(&body), ClassifyContext::TokenEndpoint, None);
        assert_eq!(
            classified.error.hint(),
            Some("clock skew — check the system clock")
        );
    }

    #[test]
    fn hint_is_none_for_errors_without_one() {
        assert_eq!(
            UploadError::Auth("401 unauthorized".to_string()).hint(),
            None
        );
        assert_eq!(UploadError::Transient("503".to_string()).hint(), None);
        assert_eq!(UploadError::Permanent("422".to_string()).hint(), None);
        assert_eq!(UploadError::Cancelled.hint(), None);
        let io_err = UploadError::Io(std::io::Error::other("locked"));
        assert_eq!(io_err.hint(), None);
    }

    #[test]
    fn is_retryable_matches_spec_table() {
        assert!(UploadError::Transient("x".into()).is_retryable());
        assert!(UploadError::Io(std::io::Error::other("locked")).is_retryable());
        assert!(!UploadError::Auth("x".into()).is_retryable());
        assert!(!UploadError::Permanent("x".into()).is_retryable());
        assert!(!UploadError::Cancelled.is_retryable());
    }

    #[test]
    fn code_matches_spec_table() {
        assert_eq!(UploadError::Auth("x".into()).code(), "auth");
        assert_eq!(UploadError::Transient("x".into()).code(), "transient");
        assert_eq!(UploadError::Permanent("x".into()).code(), "permanent");
        assert_eq!(UploadError::Io(std::io::Error::other("x")).code(), "io");
        assert_eq!(UploadError::Cancelled.code(), "cancelled");
    }

    #[test]
    fn classify_transport_is_always_transient() {
        assert_eq!(classify_transport(true, false).code(), "transient");
        assert_eq!(classify_transport(false, true).code(), "transient");
        assert_eq!(classify_transport(false, false).code(), "transient");
        assert!(classify_transport(true, false).is_retryable());
    }

    #[test]
    fn parse_error_body_google_api_shape() {
        let raw = br#"{
            "error": {
                "code": 403,
                "message": "The user does not have sufficient permissions.",
                "errors": [
                    { "domain": "global", "reason": "insufficientPermissions", "message": "denied" }
                ]
            }
        }"#;
        let parsed = parse_error_body(raw);
        assert_eq!(parsed.reason.as_deref(), Some("insufficientPermissions"));
        assert_eq!(parsed.code, None);
        assert_eq!(
            parsed.message.as_deref(),
            Some("The user does not have sufficient permissions.")
        );
    }

    #[test]
    fn parse_error_body_google_token_endpoint_shape() {
        let raw = br#"{
            "error": "invalid_grant",
            "error_description": "Invalid JWT: Token must be a short-lived token."
        }"#;
        let parsed = parse_error_body(raw);
        assert_eq!(parsed.code.as_deref(), Some("invalid_grant"));
        assert_eq!(parsed.reason, None);
        assert_eq!(
            parsed.message.as_deref(),
            Some("Invalid JWT: Token must be a short-lived token.")
        );
    }

    #[test]
    fn parse_error_body_aws_xml_shape() {
        let raw = br#"<?xml version="1.0" encoding="UTF-8"?>
<Error>
  <Code>AccessDenied</Code>
  <Message>Access Denied</Message>
  <RequestId>ABCD1234</RequestId>
</Error>"#;
        let parsed = parse_error_body(raw);
        assert_eq!(parsed.code.as_deref(), Some("AccessDenied"));
        assert_eq!(parsed.message.as_deref(), Some("Access Denied"));
        assert_eq!(parsed.reason, None);
    }

    #[test]
    fn parse_error_body_unrecognised_input_yields_default() {
        assert!(matches!(
            parse_error_body(b"not json, not xml"),
            ErrorBody {
                reason: None,
                code: None,
                message: None,
            }
        ));
        assert!(matches!(
            parse_error_body(b"{}"),
            ErrorBody {
                reason: None,
                code: None,
                message: None,
            }
        ));
        assert!(matches!(
            parse_error_body(b""),
            ErrorBody {
                reason: None,
                code: None,
                message: None,
            }
        ));
    }
}
