//! Service Account JWT auth (T-4.1; SPEC.md §6 "gdrive/ › auth.rs"; PRD.md
//! RF-010, RF-016; Decision C1, SPEC.md §12).
//!
//! Google Drive access is **Service-Account-only**: no OAuth code paths, no
//! browser, no `redirect_uri`, no refresh token. A Service Account JSON key
//! is exchanged for a short-lived (~1h) bearer token via the JWT-bearer
//! grant (RFC 6749 §4.5), cached here with a 5-minute skew (RF-016) so an
//! in-flight upload never blocks on a token fetch. The Service Account JSON
//! itself is never persisted, logged, or echoed by this module (RNF-002,
//! RNF-003, RNF-015) — only `client_email`/`project_id` ever leave it.
//!
//! [`yup_oauth2::authenticator::Authenticator`] (built via
//! [`yup_oauth2::ServiceAccountAuthenticator`]) is the only public surface
//! capable of performing the JWT-bearer exchange — the crate's JWT/claims
//! machinery (`service_account::{JWTSigner, Claims, ServiceAccountFlow}`) is
//! private. [`TokenProvider`] wraps it with our own skew-aware cache instead
//! of relying on yup's built-in one, which uses a fixed 1-minute skew where
//! SPEC.md §6 requires 5.

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use crate::credentials::{Credentials, SecretStore};
use crate::uploaders::error::{classify, ClassifyContext, ErrorBody, UploadError};

/// OAuth2 scope requested for every token (SPEC.md §6).
pub const DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive";

/// Refresh a cached token this far ahead of its real expiry (RF-016).
pub const TOKEN_SKEW: Duration = Duration::from_secs(300);

/// A parsed, validated Service Account key.
///
/// `key` holds the raw [`yup_oauth2::ServiceAccountKey`] (including the
/// private key material) and is never exposed outside this module; only the
/// two fields safe to show in the UI/logs (RF-010, RNF-015) are public.
pub struct ServiceAccount {
    key: yup_oauth2::ServiceAccountKey,
    pub client_email: String,
    pub project_id: Option<String>,
}

impl fmt::Debug for ServiceAccount {
    /// Redacts `private_key` (and every other key field) — only
    /// `client_email`/`project_id` are ever safe to log (RNF-015).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceAccount")
            .field("client_email", &self.client_email)
            .field("project_id", &self.project_id)
            .field("private_key", &"<redacted>")
            .finish()
    }
}

/// Parses and validates a Service Account JSON key (SPEC.md §6). Never
/// echoes `json`, or any field beyond `client_email`, in the returned error
/// — errors are shown to the user, so they must stay generic (Skill 3;
/// RNF-015).
pub fn parse_service_account(json: &str) -> Result<ServiceAccount, UploadError> {
    let key: yup_oauth2::ServiceAccountKey = serde_json::from_str(json).map_err(|_| {
        UploadError::Auth(
            "JSON da Service Account inválido: campos obrigatórios ausentes ou malformados"
                .to_string(),
        )
    })?;

    if key.key_type.as_deref() != Some("service_account") {
        return Err(UploadError::Auth(
            "JSON da Service Account inválido: campo \"type\" deve ser \"service_account\""
                .to_string(),
        ));
    }

    if !key.private_key.contains("PRIVATE KEY") {
        return Err(UploadError::Auth(
            "JSON da Service Account inválido: \"private_key\" não parece um PEM válido"
                .to_string(),
        ));
    }

    let client_email = key.client_email.clone();
    let project_id = key.project_id.clone();
    Ok(ServiceAccount {
        key,
        client_email,
        project_id,
    })
}

/// Reads the stored Service Account JSON from `creds` and parses it.
/// `Ok(None)` when nothing is stored yet (RF-010) — distinct from an `Err`,
/// which means something *is* stored but is unreadable/invalid.
pub fn from_credentials<S: SecretStore>(
    creds: &Credentials<S>,
) -> Result<Option<ServiceAccount>, UploadError> {
    let json = creds
        .get_service_account_json()
        .map_err(|err| UploadError::Auth(format!("falha ao ler a Service Account: {err}")))?;
    match json {
        Some(json) => parse_service_account(&json).map(Some),
        None => Ok(None),
    }
}

/// A cached bearer token and when it stops being usable.
struct CachedToken {
    value: String,
    expires_at: SystemTime,
}

/// Issues and caches Drive bearer tokens for one Service Account.
pub struct TokenProvider {
    auth: yup_oauth2::authenticator::DefaultAuthenticator,
    client_email: String,
    cache: Mutex<Option<CachedToken>>,
}

impl TokenProvider {
    /// Builds the authenticator for `sa`. `token_url`, when `Some`,
    /// overrides the token endpoint baked into the Service Account JSON —
    /// the hook tests use to point the JWT-bearer exchange at a `wiremock`
    /// server instead of `https://oauth2.googleapis.com/token`.
    pub async fn new(
        mut sa: ServiceAccount,
        token_url: Option<String>,
    ) -> Result<Arc<Self>, UploadError> {
        if let Some(url) = token_url {
            sa.key.token_uri = url;
        }
        let client_email = sa.client_email.clone();

        let auth = yup_oauth2::ServiceAccountAuthenticator::builder(sa.key)
            .build()
            .await
            .map_err(|err| UploadError::Auth(format!("JSON da Service Account inválido: {err}")))?;

        Ok(Arc::new(Self {
            auth,
            client_email,
            cache: Mutex::new(None),
        }))
    }

    /// Returns a valid bearer token, refreshing it when absent or within
    /// [`TOKEN_SKEW`] of expiry (RF-016). Never logs the token or the
    /// Service Account JSON — only `client_email`, on refresh (RNF-015).
    pub async fn token(&self) -> Result<String, UploadError> {
        let now = SystemTime::now();
        if let Some(value) = self.fresh_cached(now) {
            return Ok(value);
        }

        tracing::debug!(
            client_email = %self.client_email,
            "renovando token de acesso do Drive"
        );

        let access_token = self
            .auth
            .force_refreshed_token(&[DRIVE_SCOPE])
            .await
            .map_err(map_yup_error)?;

        let value = access_token
            .token()
            .ok_or_else(|| {
                UploadError::Transient("token endpoint returned no access_token".to_string())
            })?
            .to_string();
        let expires_at = access_token
            .expiration_time()
            .map(SystemTime::from)
            .unwrap_or_else(|| now + Duration::from_secs(3600));

        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *cache = Some(CachedToken {
            value: value.clone(),
            expires_at,
        });
        Ok(value)
    }

    /// Returns the cached token's value if it won't expire within
    /// [`TOKEN_SKEW`] of `now`.
    fn fresh_cached(&self, now: SystemTime) -> Option<String> {
        let cache = self
            .cache
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let cached = cache.as_ref()?;
        is_fresh(cached, now).then(|| cached.value.clone())
    }
}

/// Whether `cached` is still usable `TOKEN_SKEW` ahead of `now`.
fn is_fresh(cached: &CachedToken, now: SystemTime) -> bool {
    match cached.expires_at.checked_sub(TOKEN_SKEW) {
        Some(skewed) => skewed > now,
        None => false,
    }
}

/// Maps a [`yup_oauth2::error::Error`] to [`UploadError`], reusing
/// [`classify`] for the one variant that carries a real token-endpoint
/// error body.
///
/// yup-oauth2's `Error` never carries the HTTP status code the server
/// actually sent — `ServiceAccountFlow::token` reads `(head, body)` off the
/// response but only logs `head`, discarding the status before it reaches
/// any public API. RFC 6749 §5.2 mandates `400` for every token-endpoint
/// error response, so [`yup_oauth2::error::Error::AuthError`] is classified
/// as though the server had sent exactly that; other yup error codes at
/// that synthesized status fall through to [`classify`]'s generic
/// `400..=499 => Permanent` rule, a documented limitation of this mapping.
fn map_yup_error(err: yup_oauth2::error::Error) -> UploadError {
    use yup_oauth2::error::Error as YupError;

    match err {
        YupError::AuthError(auth_err) => {
            let body = ErrorBody {
                reason: None,
                code: Some(auth_err.error.as_str().to_string()),
                message: auth_err.error_description.clone(),
            };
            classify(400, Some(&body), ClassifyContext::TokenEndpoint, None).error
        }
        // No HTTP response was ever received, or the transport itself
        // failed — always retryable (SPEC.md §6).
        YupError::HttpError(_) | YupError::HttpClientError(_) | YupError::LowLevelError(_) => {
            UploadError::Transient(format!("falha de transporte no token endpoint: {err}"))
        }
        // A response arrived but wasn't valid JSON — can't have been an
        // `AuthError` (that shape parses fine), so treat as a transient
        // server-side hiccup rather than a permanent auth failure.
        YupError::JSONError(_) => {
            UploadError::Transient(format!("resposta do token endpoint ilegível: {err}"))
        }
        YupError::MissingAccessToken => {
            UploadError::Transient("token endpoint não retornou access_token".to_string())
        }
        // Bad input to yup itself (e.g. scopes) — a configuration problem,
        // not something a retry fixes.
        YupError::UserError(msg) => {
            UploadError::Auth(format!("JSON da Service Account inválido: {msg}"))
        }
        YupError::OtherError(err) => {
            UploadError::Transient(format!("erro no token endpoint: {err}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    /// Throwaway 2048-bit RSA key, PKCS8 PEM, generated once for this test
    /// suite only (`openssl genpkey -algorithm RSA -pkeyopt
    /// rsa_keygen_bits:2048`). Never used outside these tests.
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

    fn sa_json(private_key: &str) -> String {
        json!({
            "type": "service_account",
            "project_id": "my-project",
            "private_key_id": "abc123",
            "private_key": private_key,
            "client_email": "sync@my-project.iam.gserviceaccount.com",
            "client_id": "123456789",
            "token_uri": "https://oauth2.googleapis.com/token",
        })
        .to_string()
    }

    fn valid_sa_json() -> String {
        sa_json(TEST_PRIVATE_KEY)
    }

    // ---- parse_service_account -------------------------------------------------

    #[test]
    fn parse_service_account_accepts_a_well_formed_key() {
        let sa = parse_service_account(&valid_sa_json()).expect("should parse");
        assert_eq!(sa.client_email, "sync@my-project.iam.gserviceaccount.com");
        assert_eq!(sa.project_id.as_deref(), Some("my-project"));
    }

    #[test]
    fn parse_service_account_rejects_invalid_json() {
        let err = parse_service_account("not json at all").unwrap_err();
        assert!(matches!(err, UploadError::Auth(_)));
        assert!(err.to_string().contains("JSON da Service Account inválido"));
    }

    #[test]
    fn parse_service_account_rejects_wrong_type() {
        let json = json!({
            "type": "authorized_user",
            "private_key": TEST_PRIVATE_KEY,
            "client_email": "sync@my-project.iam.gserviceaccount.com",
            "token_uri": "https://oauth2.googleapis.com/token",
        })
        .to_string();

        let err = parse_service_account(&json).unwrap_err();
        assert!(matches!(err, UploadError::Auth(_)));
    }

    #[test]
    fn parse_service_account_rejects_missing_private_key() {
        let json = json!({
            "type": "service_account",
            "client_email": "sync@my-project.iam.gserviceaccount.com",
            "token_uri": "https://oauth2.googleapis.com/token",
        })
        .to_string();

        let err = parse_service_account(&json).unwrap_err();
        assert!(matches!(err, UploadError::Auth(_)));
    }

    #[test]
    fn parse_service_account_rejects_missing_client_email() {
        let json = json!({
            "type": "service_account",
            "private_key": TEST_PRIVATE_KEY,
            "token_uri": "https://oauth2.googleapis.com/token",
        })
        .to_string();

        let err = parse_service_account(&json).unwrap_err();
        assert!(matches!(err, UploadError::Auth(_)));
    }

    #[test]
    fn debug_output_never_contains_the_private_key() {
        let sa = parse_service_account(&valid_sa_json()).expect("should parse");
        let debug = format!("{sa:?}");
        assert!(!debug.contains("BEGIN PRIVATE KEY"));
        assert!(debug.contains("sync@my-project.iam.gserviceaccount.com"));
    }

    // ---- from_credentials --------------------------------------------------

    #[test]
    fn from_credentials_returns_none_when_nothing_is_stored() {
        let creds = Credentials::new(crate::credentials::MemoryStore::default());
        let result = from_credentials(&creds).expect("should not error");
        assert!(result.is_none());
    }

    #[test]
    fn from_credentials_parses_the_stored_json() {
        let creds = Credentials::new(crate::credentials::MemoryStore::default());
        creds
            .set_service_account_json(&valid_sa_json())
            .expect("store should accept a valid key");

        let sa = from_credentials(&creds)
            .expect("should not error")
            .expect("should be present");
        assert_eq!(sa.client_email, "sync@my-project.iam.gserviceaccount.com");
    }

    // ---- TokenProvider — wiremock-backed token endpoint ---------------------

    /// Minimal, dependency-free `application/x-www-form-urlencoded` decoder
    /// for the one request shape this suite needs to inspect
    /// (`grant_type=...&assertion=...`). No crate is added for this —
    /// `url`/`base64` are only transitive dependencies here (pulled in by
    /// `reqwest`/`wiremock`), and Rust's 2018+ extern prelude only exposes
    /// direct dependencies, so they can't be `use`d without adding a Cargo
    /// line this task doesn't permit.
    fn percent_decode(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'+' => {
                    out.push(b' ');
                    i += 1;
                }
                b'%' if i + 2 < bytes.len() => {
                    let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                    match u8::from_str_radix(hex, 16) {
                        Ok(byte) => {
                            out.push(byte);
                            i += 3;
                        }
                        Err(_) => {
                            out.push(bytes[i]);
                            i += 1;
                        }
                    }
                }
                b => {
                    out.push(b);
                    i += 1;
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn parse_form(body: &[u8]) -> std::collections::HashMap<String, String> {
        String::from_utf8_lossy(body)
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .map(|(k, v)| (percent_decode(k), percent_decode(v)))
            .collect()
    }

    /// Hand-rolled base64 (`URL_SAFE`, i.e. `-`/`_`, optionally `=`-padded —
    /// the alphabet yup-oauth2 signs JWTs with) decoder, for the same reason
    /// `percent_decode` is hand-rolled above: no `base64` crate direct
    /// dependency to `use` it from.
    fn base64url_decode(s: &str) -> Vec<u8> {
        let mut lookup = [255u8; 256];
        for (i, c) in "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"
            .bytes()
            .enumerate()
        {
            lookup[c as usize] = i as u8;
        }

        let clean: Vec<u8> = s.bytes().filter(|&b| b != b'=').collect();
        let mut out = Vec::new();
        for chunk in clean.chunks(4) {
            let mut buf = [0u8; 4];
            let mut n = 0usize;
            for &c in chunk {
                let v = lookup[c as usize];
                if v != 255 {
                    buf[n] = v;
                    n += 1;
                }
            }
            out.push((buf[0] << 2) | (buf[1] >> 4));
            if n > 2 {
                out.push((buf[1] << 4) | (buf[2] >> 2));
            }
            if n > 3 {
                out.push((buf[2] << 6) | buf[3]);
            }
        }
        out
    }

    fn decode_jwt_part(part: &str) -> serde_json::Value {
        let bytes = base64url_decode(part);
        serde_json::from_slice(&bytes).expect("JWT part should be valid JSON")
    }

    async fn provider_for(mock_server: &MockServer) -> Arc<TokenProvider> {
        let sa = parse_service_account(&valid_sa_json()).expect("should parse");
        TokenProvider::new(sa, Some(format!("{}/token", mock_server.uri())))
            .await
            .expect("should build the authenticator")
    }

    #[tokio::test]
    async fn token_signs_a_jwt_bearer_request_and_returns_the_access_token() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "access_token": "tok-1", "expires_in": 3600 })),
            )
            .mount(&mock_server)
            .await;

        let provider = provider_for(&mock_server).await;
        let token = provider.token().await.expect("should succeed");
        assert_eq!(token, "tok-1");

        let requests = mock_server
            .received_requests()
            .await
            .expect("request recording is on by default");
        assert_eq!(requests.len(), 1);

        let form = parse_form(&requests[0].body);
        assert_eq!(
            form.get("grant_type").map(String::as_str),
            Some("urn:ietf:params:oauth:grant-type:jwt-bearer")
        );
        let assertion = form.get("assertion").expect("assertion field present");
        let mut parts = assertion.split('.');
        let header = decode_jwt_part(parts.next().expect("header part"));
        let claims = decode_jwt_part(parts.next().expect("claims part"));

        assert_eq!(header["alg"], "RS256");
        assert_eq!(claims["iss"], "sync@my-project.iam.gserviceaccount.com");
        assert_eq!(claims["aud"], format!("{}/token", mock_server.uri()));
        assert_eq!(claims["scope"], DRIVE_SCOPE);
        let exp = claims["exp"].as_i64().expect("exp is a number");
        let iat = claims["iat"].as_i64().expect("iat is a number");
        // yup-oauth2 sets `exp = iat + 3600 - 5` ("Max validity is 1h",
        // with a 5s safety margin) — not exactly 3600, and not something
        // this module can change (the JWT claims are private to yup).
        assert_eq!(exp - iat, 3595);
    }

    #[tokio::test]
    async fn token_is_cached_within_the_skew_window() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "access_token": "tok-1", "expires_in": 3600 })),
            )
            .mount(&mock_server)
            .await;

        let provider = provider_for(&mock_server).await;
        provider.token().await.expect("first call succeeds");
        provider.token().await.expect("second call succeeds");

        let requests = mock_server.received_requests().await.unwrap_or_default();
        assert_eq!(requests.len(), 1, "second call should be served from cache");
    }

    #[tokio::test]
    async fn token_refreshes_when_remaining_life_is_under_the_skew() {
        let mock_server = MockServer::start().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let counted_calls = calls.clone();
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(move |_req: &wiremock::Request| {
                counted_calls.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "access_token": "tok-1", "expires_in": 200 }))
            })
            .mount(&mock_server)
            .await;

        let provider = provider_for(&mock_server).await;
        provider.token().await.expect("first call succeeds");
        provider.token().await.expect("second call succeeds");

        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "200s expiry is under the 300s skew, so the second call must refetch"
        );
    }

    #[tokio::test]
    async fn invalid_grant_maps_to_auth_with_a_clock_skew_hint() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": "invalid_grant",
                "error_description": "Invalid JWT: Token must be a short-lived token",
            })))
            .mount(&mock_server)
            .await;

        let provider = provider_for(&mock_server).await;
        let err = provider.token().await.unwrap_err();
        assert!(matches!(err, UploadError::Auth(_)));
        assert!(!err.is_retryable());
        let hint = err.hint().expect("invalid_grant should carry a hint");
        assert!(
            hint.to_lowercase().contains("rel\u{f3}gio") || hint.to_lowercase().contains("clock"),
            "hint should mention clock skew, got: {hint}"
        );
    }

    #[tokio::test]
    async fn server_error_maps_to_transient() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(
                ResponseTemplate::new(500).set_body_string("internal server error, not json"),
            )
            .mount(&mock_server)
            .await;

        let provider = provider_for(&mock_server).await;
        let err = provider.token().await.unwrap_err();
        assert!(matches!(err, UploadError::Transient(_)));
        assert!(err.is_retryable());
    }
}
