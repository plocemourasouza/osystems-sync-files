//! `core::credentials` — secret storage (SPEC.md §5 "Credenciais", §7, §9; PRD.md RF-010, RF-020,
//! RF-085; RNF-002, RNF-003, RNF-015).
//!
//! Secrets are held **only** in the OS keychain, under the keyring service `osystems-sync`. They
//! never touch `config.json`/`state.db` and never cross the IPC boundary except through
//! [`CredentialStatus`] — presence flags, a masked AWS key, and the Service Account's derived
//! `client_email`/`project_id`. This module never emits structured log lines carrying that
//! material: platform keyring failures are mapped to [`CredError::Backend`] using only the
//! backend error's `Display` output, which never echoes back what was being stored/read.

#[cfg(any(test, feature = "test-support"))]
use std::collections::HashMap;
#[cfg(any(test, feature = "test-support"))]
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use ts_rs::TS;

/// Keyring service name used for every entry this module creates.
pub const SERVICE_NAME: &str = "osystems-sync";

pub const KEY_AWS_ACCESS_KEY_ID: &str = "aws.access_key_id";
pub const KEY_AWS_SECRET_ACCESS_KEY: &str = "aws.secret_access_key";

/// Key prefix for Service Account JSON chunks: `{KEY_GDRIVE_SA_JSON_PREFIX}.{0..n}`.
pub const KEY_GDRIVE_SA_JSON_PREFIX: &str = "gdrive.service_account_json";
/// Holds the chunk count, as a base-10 string (e.g. `"4"`).
pub const KEY_GDRIVE_SA_JSON_COUNT: &str = "gdrive.service_account_json.count";

/// Max chars per Service Account JSON chunk. The Windows Credential Manager stores blobs as
/// UTF-16, capped at 2560 bytes (~1280 ASCII chars) per entry (SPEC.md §5); 1024 keeps headroom
/// on every platform, so the JSON is always split into `ceil(len / 1024)` chunks before it is
/// written, and reassembled in that same order on read.
pub const SA_CHUNK_CHARS: usize = 1024;

/// Errors from reading/writing secrets.
#[derive(Debug, Error)]
pub enum CredError {
    /// The underlying platform keychain reported a failure. The message is the backend error's
    /// `Display` output only — never the secret value that was being set/read.
    #[error("secret backend error: {0}")]
    Backend(String),
    /// No secure secret backend is available. Production code must return this instead of
    /// silently falling back to an in-memory (non-persistent, non-OS-protected) store.
    #[error("no secure secret backend is available")]
    Unavailable,
    /// The stored Service Account JSON is missing a chunk, or its chunk count is missing/invalid
    /// while chunk data is still present.
    #[error("stored service account JSON is corrupt or incomplete")]
    Corrupt,
    /// The JSON handed to [`Credentials::set_service_account_json`] failed validation.
    #[error("invalid service account JSON: {0}")]
    InvalidServiceAccount(String),
}

/// A key/value secret backend. `key` is an opaque identifier (e.g. `aws.access_key_id`); `value`
/// is the secret material. Implementations must never log `value`.
pub trait SecretStore: Send + Sync {
    fn set(&self, key: &str, value: &str) -> Result<(), CredError>;
    fn get(&self, key: &str) -> Result<Option<String>, CredError>;
    fn delete(&self, key: &str) -> Result<(), CredError>;
}

/// [`SecretStore`] backed by the OS keychain (Keychain on macOS, Credential Manager on Windows,
/// Secret Service on Linux) via the `keyring` crate.
pub struct KeyringStore {
    service: String,
}

impl KeyringStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry, CredError> {
        keyring::Entry::new(&self.service, key).map_err(|e| CredError::Backend(e.to_string()))
    }
}

impl Default for KeyringStore {
    /// Uses the production service name [`SERVICE_NAME`] (`"osystems-sync"`).
    fn default() -> Self {
        Self::new(SERVICE_NAME)
    }
}

impl SecretStore for KeyringStore {
    fn set(&self, key: &str, value: &str) -> Result<(), CredError> {
        self.entry(key)?
            .set_password(value)
            .map_err(|e| CredError::Backend(e.to_string()))
    }

    fn get(&self, key: &str) -> Result<Option<String>, CredError> {
        match self.entry(key)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(CredError::Backend(e.to_string())),
        }
    }

    fn delete(&self, key: &str) -> Result<(), CredError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(CredError::Backend(e.to_string())),
        }
    }
}

/// In-memory [`SecretStore`], for tests only. Not a production fallback: if the real keychain is
/// genuinely unavailable, production code must surface [`CredError::Unavailable`] rather than
/// silently downgrading to this (non-persistent, OS-unprotected) store.
///
/// Compiled in only for this crate's own tests (`cfg(test)`) or when a dependent crate opts in
/// via the `test-support` feature (MINOR #7 / T-6.3 audit) — never for an ordinary release
/// build. `cfg(test)` alone would not be enough for the latter case: Cargo evaluates it
/// per-crate, so `osystems-sync`'s own `cfg(test)` when running *its* tests does not make
/// `cfg(test)` true while compiling `osystems-sync-core` as its dependency. The `test-support`
/// feature is how `osystems-sync`'s dev-dependency on this crate asks for `MemoryStore` without
/// it ever reaching a real build (Cargo's resolver `"2"` keeps dev-dependency features out of
/// non-test builds).
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

#[cfg(any(test, feature = "test-support"))]
impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl SecretStore for MemoryStore {
    fn set(&self, key: &str, value: &str) -> Result<(), CredError> {
        let mut guard = self
            .0
            .lock()
            .map_err(|_| CredError::Backend("in-memory store mutex poisoned".to_string()))?;
        guard.insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<String>, CredError> {
        let guard = self
            .0
            .lock()
            .map_err(|_| CredError::Backend("in-memory store mutex poisoned".to_string()))?;
        Ok(guard.get(key).cloned())
    }

    fn delete(&self, key: &str) -> Result<(), CredError> {
        let mut guard = self
            .0
            .lock()
            .map_err(|_| CredError::Backend("in-memory store mutex poisoned".to_string()))?;
        guard.remove(key);
        Ok(())
    }
}

/// AWS static credentials read back from the keyring.
///
/// **Never serialize or log this struct.** It deliberately does not derive `Serialize`/`TS`, so
/// it cannot accidentally become an IPC payload; its [`std::fmt::Debug`] impl masks both fields
/// so an incidental debug print in a future call site still can't leak the raw secret (RNF-003).
#[derive(Clone, PartialEq, Eq)]
pub struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
}

impl std::fmt::Debug for AwsCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AwsCredentials")
            .field("access_key_id", &mask(&self.access_key_id))
            .field("secret_access_key", &"****")
            .finish()
    }
}

/// AWS S3 credential presence, as exposed over IPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AwsStatus {
    pub present: bool,
    /// `mask(access_key_id)`, e.g. `"AKIA****XYZ"`. `None` when not present.
    pub masked: Option<String>,
}

/// Google Drive Service Account presence, as exposed over IPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GDriveStatus {
    pub present: bool,
    /// `client_email` derived from the stored Service Account JSON.
    pub email: Option<String>,
    /// `project_id` derived from the stored Service Account JSON.
    pub project_id: Option<String>,
}

/// The only credential shape that ever crosses the IPC boundary (RNF-003, SPEC.md §7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CredentialStatus {
    pub aws: AwsStatus,
    pub gdrive: GDriveStatus,
}

/// Info returned to the UI after a Service Account JSON file is picked and stored
/// (RF-014). Carries only what the picker dialog needs to display — never the
/// JSON content or the original file path (SPEC.md §9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ServiceAccountInfo {
    pub file_name: String,
    #[ts(type = "number")]
    pub size: u64,
    pub client_email: String,
    pub project_id: Option<String>,
}

/// Typed credential API over a [`SecretStore`] backend.
pub struct Credentials<S: SecretStore> {
    store: S,
}

impl<S: SecretStore> Credentials<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    /// Direct access to the backing store — mainly for tests that need to inspect/corrupt raw
    /// keys (e.g. deleting the SA chunk-count key to exercise [`CredError::Corrupt`]).
    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn set_aws(&self, access_key_id: &str, secret_access_key: &str) -> Result<(), CredError> {
        self.store.set(KEY_AWS_ACCESS_KEY_ID, access_key_id)?;
        self.store
            .set(KEY_AWS_SECRET_ACCESS_KEY, secret_access_key)?;
        Ok(())
    }

    pub fn get_aws(&self) -> Result<Option<AwsCredentials>, CredError> {
        let access_key_id = self.store.get(KEY_AWS_ACCESS_KEY_ID)?;
        let secret_access_key = self.store.get(KEY_AWS_SECRET_ACCESS_KEY)?;
        Ok(match (access_key_id, secret_access_key) {
            (Some(access_key_id), Some(secret_access_key)) => Some(AwsCredentials {
                access_key_id,
                secret_access_key,
            }),
            _ => None,
        })
    }

    pub fn clear_aws(&self) -> Result<(), CredError> {
        self.store.delete(KEY_AWS_ACCESS_KEY_ID)?;
        self.store.delete(KEY_AWS_SECRET_ACCESS_KEY)?;
        Ok(())
    }

    /// Validates `json` (must parse as an object with `type == "service_account"` and non-empty
    /// `client_email`, `private_key`, `project_id`), then stores it split into
    /// [`SA_CHUNK_CHARS`]-char chunks under `gdrive.service_account_json.{0..n}` plus a
    /// `gdrive.service_account_json.count` key. Any previously stored SA JSON (including a
    /// corrupt/partial one) is cleared first, so a smaller replacement never leaves stale trailing
    /// chunks. On validation failure, nothing is written.
    pub fn set_service_account_json(&self, json: &str) -> Result<(), CredError> {
        validate_service_account_json(json)?;
        self.clear_service_account_json()?;

        let chunks = chunk_str(json, SA_CHUNK_CHARS);
        for (i, chunk) in chunks.iter().enumerate() {
            self.store.set(&sa_chunk_key(i), chunk)?;
        }
        self.store
            .set(KEY_GDRIVE_SA_JSON_COUNT, &chunks.len().to_string())?;
        Ok(())
    }

    /// Reassembles the Service Account JSON from its chunks, in order. `Ok(None)` when nothing is
    /// stored. `Err(CredError::Corrupt)` when chunk data is present but the count key is
    /// missing/unparsable, or when a chunk in range `0..count` is missing.
    pub fn get_service_account_json(&self) -> Result<Option<String>, CredError> {
        let first_chunk = self.store.get(&sa_chunk_key(0))?;
        let count_str = self.store.get(KEY_GDRIVE_SA_JSON_COUNT)?;

        let (first_chunk, count_str) = match (first_chunk, count_str) {
            (None, None) => return Ok(None),
            (Some(_), None) | (None, Some(_)) => return Err(CredError::Corrupt),
            (Some(first_chunk), Some(count_str)) => (first_chunk, count_str),
        };

        let count: usize = count_str.trim().parse().map_err(|_| CredError::Corrupt)?;
        if count == 0 {
            return Err(CredError::Corrupt);
        }

        let mut json = first_chunk;
        for i in 1..count {
            let chunk = self
                .store
                .get(&sa_chunk_key(i))?
                .ok_or(CredError::Corrupt)?;
            json.push_str(&chunk);
        }
        Ok(Some(json))
    }

    /// Deletes every SA JSON chunk plus the count key. Tolerant of a missing/corrupt count (falls
    /// back to probing sequential chunk keys until the first hole), so it can also clean up after
    /// a partial write.
    pub fn clear_service_account_json(&self) -> Result<(), CredError> {
        let count = self
            .store
            .get(KEY_GDRIVE_SA_JSON_COUNT)?
            .and_then(|s| s.trim().parse::<usize>().ok());

        match count {
            Some(count) => {
                for i in 0..count {
                    self.store.delete(&sa_chunk_key(i))?;
                }
            }
            None => {
                let mut i = 0;
                while self.store.get(&sa_chunk_key(i))?.is_some() {
                    self.store.delete(&sa_chunk_key(i))?;
                    i += 1;
                }
            }
        }
        self.store.delete(KEY_GDRIVE_SA_JSON_COUNT)?;
        Ok(())
    }

    /// Status derived purely from what's readable right now. A backend error or corrupt SA JSON
    /// is reported as "not present" rather than propagated — `status()` must never fail the UI.
    pub fn status(&self) -> CredentialStatus {
        let aws = match self.get_aws() {
            Ok(Some(creds)) => AwsStatus {
                present: true,
                masked: Some(mask(&creds.access_key_id)),
            },
            _ => AwsStatus {
                present: false,
                masked: None,
            },
        };

        let gdrive = match self.get_service_account_json() {
            Ok(Some(json)) => GDriveStatus {
                present: true,
                email: sa_email(&json),
                project_id: sa_project_id(&json),
            },
            _ => GDriveStatus {
                present: false,
                email: None,
                project_id: None,
            },
        };

        CredentialStatus { aws, gdrive }
    }
}

fn sa_chunk_key(i: usize) -> String {
    format!("{KEY_GDRIVE_SA_JSON_PREFIX}.{i}")
}

/// Splits `s` into `String` chunks of at most `max_chars` **characters** (not bytes), so a
/// multi-byte UTF-8 boundary is never split.
fn chunk_str(s: &str, max_chars: usize) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    chars
        .chunks(max_chars)
        .map(|c| c.iter().collect())
        .collect()
}

fn validate_service_account_json(json: &str) -> Result<(), CredError> {
    let value: Value = serde_json::from_str(json)
        .map_err(|e| CredError::InvalidServiceAccount(format!("invalid JSON: {e}")))?;
    let obj = value
        .as_object()
        .ok_or_else(|| CredError::InvalidServiceAccount("expected a JSON object".to_string()))?;

    if obj.get("type").and_then(Value::as_str) != Some("service_account") {
        return Err(CredError::InvalidServiceAccount(
            "missing or invalid \"type\" (expected \"service_account\")".to_string(),
        ));
    }

    for field in ["client_email", "private_key", "project_id"] {
        let present = obj
            .get(field)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty());
        if !present {
            return Err(CredError::InvalidServiceAccount(format!(
                "missing required field \"{field}\""
            )));
        }
    }

    Ok(())
}

/// Derives `client_email` from a Service Account JSON. `None` if `json` doesn't parse or the
/// field is absent/not a string.
pub fn sa_email(json: &str) -> Option<String> {
    serde_json::from_str::<Value>(json)
        .ok()?
        .get("client_email")?
        .as_str()
        .map(str::to_owned)
}

/// Derives `project_id` from a Service Account JSON. `None` if `json` doesn't parse or the field
/// is absent/not a string.
pub fn sa_project_id(json: &str) -> Option<String> {
    serde_json::from_str::<Value>(json)
        .ok()?
        .get("project_id")?
        .as_str()
        .map(str::to_owned)
}

/// Masks a secret for display: keeps the first 4 and last 3 chars, `****` in between
/// (`AKIAIOSFODNN7EXAMPLE` → `AKIA****PLE`). Values of 8 chars or fewer become `"****"` with no
/// characters revealed at all.
pub fn mask(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 8 {
        return "****".to_string();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 3..].iter().collect();
    format!("{head}****{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sa_json_of_len(target_len: usize) -> String {
        let mut value = serde_json::json!({
            "type": "service_account",
            "project_id": "proj-x",
            "private_key_id": "abc123",
            "private_key": "-----BEGIN PRIVATE KEY-----\nMIIFAKE==\n-----END PRIVATE KEY-----\n",
            "client_email": "sa@proj-x.iam.gserviceaccount.com",
            "client_id": "1234567890",
            "padding": "",
        });
        let base_len = value.to_string().len();
        assert!(
            target_len >= base_len,
            "target_len {target_len} smaller than unpadded json ({base_len} chars)"
        );
        value["padding"] = Value::String("x".repeat(target_len - base_len));
        let json = value.to_string();
        assert_eq!(json.len(), target_len);
        json
    }

    // --- mask() ---------------------------------------------------------

    #[test]
    fn mask_examples() {
        assert_eq!(mask("AKIAIOSFODNN7EXAMPLE"), "AKIA****PLE");
        assert_eq!(mask("123456789"), "1234****789"); // 9 chars: head4 + tail3, 2 hidden
        assert_eq!(mask("12345678"), "****"); // exactly 8 chars -> fully masked
        assert_eq!(mask("1234567"), "****"); // fewer than 8
        assert_eq!(mask(""), "****");
    }

    // --- AWS round-trip ---------------------------------------------------

    #[test]
    fn aws_round_trip() {
        let creds = Credentials::new(MemoryStore::default());
        assert_eq!(creds.get_aws().unwrap(), None);

        creds
            .set_aws("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI/secret")
            .unwrap();
        let got = creds.get_aws().unwrap().expect("aws creds present");
        assert_eq!(got.access_key_id, "AKIAIOSFODNN7EXAMPLE");
        assert_eq!(got.secret_access_key, "wJalrXUtnFEMI/secret");

        creds.clear_aws().unwrap();
        assert_eq!(creds.get_aws().unwrap(), None);
    }

    #[test]
    fn aws_credentials_debug_masks_both_fields() {
        let creds = AwsCredentials {
            access_key_id: "AKIAIOSFODNN7EXAMPLE".to_string(),
            secret_access_key: "wJalrXUtnFEMI/secret".to_string(),
        };
        let debug = format!("{creds:?}");
        assert!(debug.contains("AKIA****PLE"));
        assert!(!debug.contains("wJalrXUtnFEMI/secret"));
    }

    // --- Service Account JSON chunking -----------------------------------

    #[test]
    fn sa_json_chunks_into_exact_count_and_round_trips() {
        let json = sa_json_of_len(3200); // ~3.2 KB -> ceil(3200/1024) = 4 chunks
        let creds = Credentials::new(MemoryStore::default());
        creds.set_service_account_json(&json).unwrap();

        assert_eq!(
            creds
                .store()
                .get(KEY_GDRIVE_SA_JSON_COUNT)
                .unwrap()
                .as_deref(),
            Some("4")
        );
        assert!(creds.store().get(&sa_chunk_key(4)).unwrap().is_none());

        let round_tripped = creds.get_service_account_json().unwrap();
        assert_eq!(round_tripped.as_deref(), Some(json.as_str()));
    }

    #[test]
    fn sa_json_shorter_than_one_chunk_round_trips() {
        let json = sa_json_of_len(300);
        let creds = Credentials::new(MemoryStore::default());
        creds.set_service_account_json(&json).unwrap();

        assert_eq!(
            creds
                .store()
                .get(KEY_GDRIVE_SA_JSON_COUNT)
                .unwrap()
                .as_deref(),
            Some("1")
        );
        assert_eq!(
            creds.get_service_account_json().unwrap().as_deref(),
            Some(json.as_str())
        );
    }

    #[test]
    fn replacing_sa_json_with_fewer_chunks_drops_stale_trailing_chunks() {
        let creds = Credentials::new(MemoryStore::default());
        creds
            .set_service_account_json(&sa_json_of_len(3200))
            .unwrap(); // 4 chunks
        creds
            .set_service_account_json(&sa_json_of_len(300))
            .unwrap(); // 1 chunk

        assert_eq!(
            creds
                .store()
                .get(KEY_GDRIVE_SA_JSON_COUNT)
                .unwrap()
                .as_deref(),
            Some("1")
        );
        assert!(creds.store().get(&sa_chunk_key(1)).unwrap().is_none());
        assert!(creds.store().get(&sa_chunk_key(3)).unwrap().is_none());
    }

    #[test]
    fn nothing_stored_reads_back_as_none() {
        let creds = Credentials::new(MemoryStore::default());
        assert_eq!(creds.get_service_account_json().unwrap(), None);
    }

    #[test]
    fn missing_count_after_chunks_written_is_corrupt() {
        let creds = Credentials::new(MemoryStore::default());
        creds
            .set_service_account_json(&sa_json_of_len(2000))
            .unwrap();
        creds.store().delete(KEY_GDRIVE_SA_JSON_COUNT).unwrap();

        assert!(matches!(
            creds.get_service_account_json(),
            Err(CredError::Corrupt)
        ));
    }

    #[test]
    fn missing_middle_chunk_is_corrupt() {
        let creds = Credentials::new(MemoryStore::default());
        creds
            .set_service_account_json(&sa_json_of_len(2500))
            .unwrap(); // 3 chunks
        creds.store().delete(&sa_chunk_key(1)).unwrap();

        assert!(matches!(
            creds.get_service_account_json(),
            Err(CredError::Corrupt)
        ));
    }

    #[test]
    fn clear_service_account_json_removes_all_chunks_and_count() {
        let creds = Credentials::new(MemoryStore::default());
        creds
            .set_service_account_json(&sa_json_of_len(3200))
            .unwrap();
        creds.clear_service_account_json().unwrap();

        assert_eq!(creds.get_service_account_json().unwrap(), None);
        assert!(creds
            .store()
            .get(KEY_GDRIVE_SA_JSON_COUNT)
            .unwrap()
            .is_none());
        for i in 0..4 {
            assert!(creds.store().get(&sa_chunk_key(i)).unwrap().is_none());
        }
    }

    #[test]
    fn clear_service_account_json_cleans_up_partial_state_without_count() {
        let creds = Credentials::new(MemoryStore::default());
        creds
            .set_service_account_json(&sa_json_of_len(3200))
            .unwrap();
        creds.store().delete(KEY_GDRIVE_SA_JSON_COUNT).unwrap(); // simulate corruption

        creds.clear_service_account_json().unwrap();

        for i in 0..4 {
            assert!(creds.store().get(&sa_chunk_key(i)).unwrap().is_none());
        }
    }

    // --- validation --------------------------------------------------------

    #[test]
    fn invalid_service_account_json_missing_private_key_is_rejected() {
        let creds = Credentials::new(MemoryStore::default());
        let bad = serde_json::json!({
            "type": "service_account",
            "client_email": "sa@proj.iam.gserviceaccount.com",
            "project_id": "proj",
        })
        .to_string();

        let err = creds.set_service_account_json(&bad).unwrap_err();
        assert!(matches!(err, CredError::InvalidServiceAccount(_)));
        assert_eq!(creds.get_service_account_json().unwrap(), None);
    }

    #[test]
    fn invalid_service_account_json_wrong_type_is_rejected() {
        let creds = Credentials::new(MemoryStore::default());
        let bad = serde_json::json!({
            "type": "authorized_user",
            "client_email": "sa@proj.iam.gserviceaccount.com",
            "private_key": "x",
            "project_id": "proj",
        })
        .to_string();

        assert!(matches!(
            creds.set_service_account_json(&bad),
            Err(CredError::InvalidServiceAccount(_))
        ));
    }

    #[test]
    fn malformed_json_syntax_is_rejected() {
        let creds = Credentials::new(MemoryStore::default());
        assert!(matches!(
            creds.set_service_account_json("{not json"),
            Err(CredError::InvalidServiceAccount(_))
        ));
    }

    #[test]
    fn set_service_account_json_never_writes_partial_state_on_failure() {
        let creds = Credentials::new(MemoryStore::default());
        creds
            .set_service_account_json(&sa_json_of_len(3200))
            .unwrap();

        // A subsequent invalid write must not disturb the previously stored valid credentials.
        assert!(creds.set_service_account_json("{not json").is_err());
        assert_eq!(
            creds.get_service_account_json().unwrap().as_deref(),
            Some(sa_json_of_len(3200).as_str())
        );
    }

    // --- sa_email / sa_project_id -------------------------------------------

    #[test]
    fn sa_email_and_project_id_are_derived() {
        let json = sa_json_of_len(300);
        assert_eq!(
            sa_email(&json).as_deref(),
            Some("sa@proj-x.iam.gserviceaccount.com")
        );
        assert_eq!(sa_project_id(&json).as_deref(), Some("proj-x"));
    }

    #[test]
    fn sa_email_is_none_for_garbage() {
        assert_eq!(sa_email("not json"), None);
        assert_eq!(sa_project_id("not json"), None);
    }

    // --- ServiceAccountInfo (T-4.6/4.7: serde + ts-rs export) ------------------

    #[test]
    fn service_account_info_round_trips_through_json() {
        let info = ServiceAccountInfo {
            file_name: "service-account.json".to_string(),
            size: 3200,
            client_email: "sa@proj-x.iam.gserviceaccount.com".to_string(),
            project_id: Some("proj-x".to_string()),
        };

        let json = serde_json::to_string(&info).expect("serialize ServiceAccountInfo");
        let decoded: ServiceAccountInfo =
            serde_json::from_str(&json).expect("deserialize ServiceAccountInfo");
        assert_eq!(decoded, info);
    }

    #[test]
    fn service_account_info_project_id_is_optional() {
        let info = ServiceAccountInfo {
            file_name: "service-account.json".to_string(),
            size: 300,
            client_email: "sa@proj-x.iam.gserviceaccount.com".to_string(),
            project_id: None,
        };

        let json = serde_json::to_value(&info).expect("serialize ServiceAccountInfo");
        assert_eq!(json["project_id"], Value::Null);
    }

    /// `#[ts(export)]` already generates its own hidden `cargo test`-time export test;
    /// this one additionally asserts the export actually succeeds (rather than just
    /// existing implicitly), so a broken derive/attribute on `ServiceAccountInfo`
    /// fails loudly here too — per PLAN.md T-4.6/4.7's "confirm ServiceAccountInfo.ts
    /// exists" verification step.
    #[test]
    fn service_account_info_exports_to_ts() {
        use ts_rs::TS;
        ServiceAccountInfo::export()
            .unwrap_or_else(|e| panic!("ts-rs export failed for ServiceAccountInfo: {e}"));
    }

    // --- status() ------------------------------------------------------------

    #[test]
    fn status_reflects_presence_mask_and_derived_fields() {
        let creds = Credentials::new(MemoryStore::default());

        let empty = creds.status();
        assert!(!empty.aws.present);
        assert_eq!(empty.aws.masked, None);
        assert!(!empty.gdrive.present);
        assert_eq!(empty.gdrive.email, None);
        assert_eq!(empty.gdrive.project_id, None);

        creds
            .set_aws("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI/secret")
            .unwrap();
        creds
            .set_service_account_json(&sa_json_of_len(300))
            .unwrap();

        let full = creds.status();
        assert!(full.aws.present);
        assert_eq!(full.aws.masked.as_deref(), Some("AKIA****PLE"));
        assert!(full.gdrive.present);
        assert_eq!(
            full.gdrive.email.as_deref(),
            Some("sa@proj-x.iam.gserviceaccount.com")
        );
        assert_eq!(full.gdrive.project_id.as_deref(), Some("proj-x"));
    }

    #[test]
    fn status_treats_corrupt_sa_json_as_not_present() {
        let creds = Credentials::new(MemoryStore::default());
        creds
            .set_service_account_json(&sa_json_of_len(3200))
            .unwrap();
        creds.store().delete(KEY_GDRIVE_SA_JSON_COUNT).unwrap();

        let status = creds.status();
        assert!(!status.gdrive.present);
        assert_eq!(status.gdrive.email, None);
    }

    // --- KeyringStore (real OS keychain; run manually) -----------------------

    /// Exercises the real OS keychain. Marked `#[ignore]` because it needs an interactive
    /// desktop session keychain/credential store, and to keep `cargo test` hermetic. Run with
    /// `cargo test -p osystems-sync-core --manifest-path src-tauri/Cargo.toml \
    ///   credentials:: -- --ignored`. Uses a unique service name and cleans up after itself.
    #[test]
    #[ignore = "touches the OS keychain; run manually"]
    fn keyring_store_real_round_trip() {
        let service = format!("osystems-sync-test-{}", std::process::id());
        let store = KeyringStore::new(service);
        let key = "credentials-test-roundtrip";

        // Clean slate, in case a previous run crashed before cleanup.
        let _ = store.delete(key);
        assert_eq!(store.get(key).unwrap(), None);

        store.set(key, "super-secret-value").unwrap();
        assert_eq!(
            store.get(key).unwrap().as_deref(),
            Some("super-secret-value")
        );

        store.delete(key).unwrap();
        assert_eq!(store.get(key).unwrap(), None);
    }
}
