//! Credential IPC commands (SPEC.md §5 "Credenciais", §7; PRD.md RF-010, RF-014,
//! RF-020, RF-085; RNF-003/RNF-015): `set_credential`, `get_credential_status`,
//! `clear_credential`, `pick_service_account_file`.
//!
//! AWS's two keys go through `set_credential`. The Google Drive Service Account JSON
//! (PLAN.md T-4.6/4.7) has its own shape/validation
//! (`Credentials::set_service_account_json`) and its own command,
//! [`pick_service_account_file`], since it's picked as a whole file rather than typed
//! in field-by-field.
//!
//! **Never log a credential value or the picked file's path.** `key`/`destination`
//! (metadata) are fine to log; `value`, the SA JSON content, and the file path never
//! are — every log line below only ever names the key/kind, not its content or
//! location (SPEC.md §9).

use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use osystems_sync_core::credentials::{
    sa_email, sa_project_id, CredentialStatus, SecretStore, ServiceAccountInfo,
    KEY_AWS_ACCESS_KEY_ID, KEY_AWS_SECRET_ACCESS_KEY,
};
use osystems_sync_core::state::Destination;

use crate::error::AppError;
use crate::state::AppState;

/// The only keys `set_credential`/`clear_credential` accept — an explicit allowlist
/// rather than passing `key` straight through to the keyring, so a compromised/buggy
/// frontend can't write arbitrary keyring entries under this app's service name.
const ALLOWED_KEYS: &[&str] = &[KEY_AWS_ACCESS_KEY_ID, KEY_AWS_SECRET_ACCESS_KEY];

/// Rejects any `key` outside [`ALLOWED_KEYS`]. Kept as a free function (rather than
/// inlined) so it has one obvious unit test.
fn check_key_allowed(key: &str) -> Result<(), AppError> {
    if ALLOWED_KEYS.contains(&key) {
        Ok(())
    } else {
        Err(AppError::new(
            "credentials.invalid_key",
            "unknown credential key",
        ))
    }
}

/// Writes one secret value under `key` (must be one of [`ALLOWED_KEYS`]).
///
/// `Credentials::set_aws` sets both AWS keys atomically, which doesn't fit a
/// per-field "type in the access key, then the secret" form — so this goes straight
/// through `credentials.store()` instead. Once *both* AWS parts are present in the
/// keyring, this rebuilds the S3 uploader, un-pauses the S3 worker, clears the
/// `auth_required` health flag, and kicks an immediate health probe — mirroring
/// exactly what the user expects after fixing a broken credential: uploads should
/// resume without needing an app restart.
#[tauri::command]
pub async fn set_credential(
    state: State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), AppError> {
    check_key_allowed(&key)?;

    state
        .credentials
        .store()
        .set(&key, &value)
        .map_err(AppError::from)?;

    if key == KEY_AWS_ACCESS_KEY_ID || key == KEY_AWS_SECRET_ACCESS_KEY {
        let has_both = state
            .credentials
            .get_aws()
            .map_err(AppError::from)?
            .is_some();
        if has_both {
            state
                .runtime
                .rebuild_s3_uploader(
                    &state.repo,
                    &state.config,
                    &state.credentials,
                    &state.throttles,
                    &state.uploaders,
                )
                .await?;

            if let Some(pool) = state.pool.get() {
                pool.resume_destination(Destination::S3);
            }
            if let Some(health) = state.runtime.health_handle() {
                health.set_auth_required(Destination::S3, false).await;
                health.probe_now();
            }
        }
    }

    Ok(())
}

/// Presence/masked-value snapshot for both destinations (SPEC.md §7) — never returns
/// a raw secret, only [`osystems_sync_core::credentials::AwsStatus::masked`] /
/// [`osystems_sync_core::credentials::GDriveStatus::email`].
#[tauri::command]
pub async fn get_credential_status(
    state: State<'_, AppState>,
) -> Result<CredentialStatus, AppError> {
    Ok(state.credentials.status())
}

/// Deletes one destination's stored credentials and takes its uploader offline
/// immediately, so a "Desconectar" action in the UI can't race a worker into using a
/// half-cleared credential.
#[tauri::command]
pub async fn clear_credential(
    state: State<'_, AppState>,
    destination: Destination,
) -> Result<(), AppError> {
    match destination {
        Destination::S3 => {
            state.credentials.clear_aws().map_err(AppError::from)?;
            state.uploaders.write().await.s3 = None;
        }
        Destination::GDrive => {
            state
                .credentials
                .clear_service_account_json()
                .map_err(AppError::from)?;
            state.uploaders.write().await.gdrive = None;
        }
    }
    Ok(())
}

/// A legitimate Service Account JSON is a few KB. This is checked *before* the file is
/// read into memory (MINOR #6 / T-6.3 audit): without it, a user (or a compromised
/// frontend) pointing the picker at an arbitrarily large file would have that whole
/// file read into memory first, and only rejected afterwards. A file past this bound
/// cannot be a real Service Account credential, so it is treated the same as malformed
/// JSON rather than paying the cost of reading it.
const MAX_SERVICE_ACCOUNT_JSON_BYTES: u64 = 64 * 1024;

/// Pure guard behind the size check above — kept as a free function so it has one
/// obvious unit test, same rationale as [`check_key_allowed`].
fn check_service_account_file_size(size: u64) -> Result<(), AppError> {
    if size > MAX_SERVICE_ACCOUNT_JSON_BYTES {
        Err(AppError::new(
            "gdrive.invalid_json",
            format!(
                "service account file is too large ({size} bytes, max {MAX_SERVICE_ACCOUNT_JSON_BYTES})"
            ),
        ))
    } else {
        Ok(())
    }
}

/// Opens the native "choose a file" dialog filtered to `*.json` (blocking on the
/// dialog thread pool, not the Tokio runtime), reads the picked file, validates +
/// stores it as the GDrive Service Account credential, then rebuilds/un-pauses the
/// GDrive uploader exactly like [`set_credential`] does for AWS. Returns `None` without
/// touching anything only when the user actually cancels the dialog — every other
/// failure (path resolution, stat, size guard, read) is surfaced as a typed
/// [`AppError`] instead of being folded into that same `None` (MINOR #6 / T-6.3 audit:
/// the previous `.ok()?` chain silently reported "user cancelled" for what could be a
/// permissions error or an oversized file, hiding the real problem from both the user
/// and the logs).
///
/// The file's path is read only inside the blocking closure and is never logged or
/// returned to the caller — only [`ServiceAccountInfo`] (file name, size, and the
/// email/project id parsed out of the JSON itself) crosses back over IPC.
#[tauri::command]
pub async fn pick_service_account_file(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<ServiceAccountInfo>, AppError> {
    let picked = tauri::async_runtime::spawn_blocking(move || -> Result<_, AppError> {
        let Some(file_path) = app
            .dialog()
            .file()
            .add_filter("Service Account JSON", &["json"])
            .blocking_pick_file()
        else {
            // The dialog itself returned nothing — the user cancelled. This is the
            // only case that becomes `Ok(None)` all the way back to the caller.
            return Ok(None);
        };

        let path = file_path
            .into_path()
            .map_err(|e| AppError::new("io", format!("could not resolve picked file: {e}")))?;
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let size = std::fs::metadata(&path)
            .map(|m| m.len())
            .map_err(|e| AppError::new("io", e.to_string()))?;

        check_service_account_file_size(size)?;

        let json =
            std::fs::read_to_string(&path).map_err(|e| AppError::new("io", e.to_string()))?;

        Ok(Some((file_name, size, json)))
    })
    .await
    .map_err(|join_err| {
        tracing::error!(error = %join_err, "tarefa de diálogo do pick_service_account_file entrou em panic");
        AppError::new("credentials.pick_file", "failed to open the file picker")
    })??;

    let Some((file_name, size, json)) = picked else {
        return Ok(None);
    };

    state
        .credentials
        .set_service_account_json(&json)
        .map_err(AppError::from)?;

    let client_email = sa_email(&json).unwrap_or_default();
    let project_id = sa_project_id(&json);

    state
        .runtime
        .rebuild_gdrive_uploader(
            &state.repo,
            &state.config,
            &state.credentials,
            &state.throttles,
            &state.uploaders,
        )
        .await?;

    if let Some(pool) = state.pool.get() {
        pool.resume_destination(Destination::GDrive);
    }
    if let Some(health) = state.runtime.health_handle() {
        health.set_auth_required(Destination::GDrive, false).await;
        health.probe_now();
    }

    Ok(Some(ServiceAccountInfo {
        file_name,
        size,
        client_email,
        project_id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use osystems_sync_core::credentials::{Credentials, MemoryStore};

    #[test]
    fn check_key_allowed_accepts_only_the_two_aws_keys() {
        assert!(check_key_allowed(KEY_AWS_ACCESS_KEY_ID).is_ok());
        assert!(check_key_allowed(KEY_AWS_SECRET_ACCESS_KEY).is_ok());
    }

    #[test]
    fn check_key_allowed_rejects_anything_else() {
        assert!(check_key_allowed("gdrive.service_account_json.0").is_err());
        assert!(check_key_allowed("").is_err());
        assert!(check_key_allowed("aws.access_key_id ").is_err());
    }

    #[test]
    fn check_service_account_file_size_accepts_up_to_the_64kib_boundary() {
        assert!(check_service_account_file_size(0).is_ok());
        assert!(check_service_account_file_size(1024).is_ok());
        assert!(check_service_account_file_size(MAX_SERVICE_ACCOUNT_JSON_BYTES).is_ok());
    }

    #[test]
    fn check_service_account_file_size_rejects_anything_past_the_boundary() {
        let err = check_service_account_file_size(MAX_SERVICE_ACCOUNT_JSON_BYTES + 1)
            .expect_err("one byte over the limit must be rejected");
        assert_eq!(err.code, "gdrive.invalid_json");

        let err = check_service_account_file_size(10 * 1024 * 1024)
            .expect_err("a much larger file must also be rejected");
        assert_eq!(err.code, "gdrive.invalid_json");
    }

    fn fixture_sa_json() -> String {
        serde_json::json!({
            "type": "service_account",
            "project_id": "proj-x",
            "private_key_id": "abc123",
            "private_key": "-----BEGIN PRIVATE KEY-----\nMIIFAKE==\n-----END PRIVATE KEY-----\n",
            "client_email": "sa@proj-x.iam.gserviceaccount.com",
            "client_id": "1234567890",
        })
        .to_string()
    }

    /// [`pick_service_account_file`]'s core composition — `set_service_account_json`
    /// followed by `sa_email`/`sa_project_id` + a file name/size — without the file
    /// picker dialog itself (which needs a real `AppHandle` and isn't unit-testable).
    /// Exercised here against `Credentials<MemoryStore>` per PLAN.md T-4.6's test
    /// instructions.
    #[test]
    fn service_account_info_is_built_from_a_stored_fixture() {
        let json = fixture_sa_json();
        let credentials = Credentials::new(MemoryStore::default());
        credentials
            .set_service_account_json(&json)
            .expect("set_service_account_json against MemoryStore");

        let info = ServiceAccountInfo {
            file_name: "service-account.json".to_string(),
            size: json.len() as u64,
            client_email: sa_email(&json).unwrap_or_default(),
            project_id: sa_project_id(&json),
        };

        assert_eq!(info.client_email, "sa@proj-x.iam.gserviceaccount.com");
        assert_eq!(info.project_id.as_deref(), Some("proj-x"));
        assert_eq!(info.file_name, "service-account.json");
        assert_eq!(info.size, json.len() as u64);

        // Round-trips through the keyring-backed store, confirming the exact JSON
        // `sa_email`/`sa_project_id` were derived from is what actually got persisted.
        let stored = credentials
            .get_service_account_json()
            .expect("get_service_account_json")
            .expect("a Service Account JSON was just stored");
        assert_eq!(stored, json);
    }
}
