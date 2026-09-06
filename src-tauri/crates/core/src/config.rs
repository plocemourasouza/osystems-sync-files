//! `core::config` — application configuration.
//!
//! Mirrors `%APPDATA%/osystems-sync/config.json` (SPEC.md §5). Never carries secrets: those live
//! in the OS keyring (see `crates/core` credentials module, out of scope here).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

/// Hard ceiling for a single file, in MiB (5 TiB) — the smaller of the two
/// destinations' own object limits: S3 caps an object at 5 TiB and Google
/// Drive at 5 TB. `watch.max_size_mb` and `watch.min_size_mb` are validated
/// against this, and `uploaders::s3` refuses anything above it outright.
pub const MAX_FILE_SIZE_MB: u32 = 5 * 1024 * 1024;

/// Full application configuration, persisted as `config.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct AppConfig {
    pub version: u32,
    pub watch: WatchConfig,
    pub s3: S3Config,
    pub gdrive: GDriveConfig,
    pub qos: QosConfig,
    pub retry: RetryConfig,
    pub workers_per_destination: u8,
    pub autostart: bool,
    pub keep_awake: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: 1,
            watch: WatchConfig::default(),
            s3: S3Config::default(),
            gdrive: GDriveConfig::default(),
            qos: QosConfig::default(),
            retry: RetryConfig::default(),
            workers_per_destination: 2,
            autostart: true,
            keep_awake: true,
        }
    }
}

/// Watcher settings: monitored folder, filters, and stabilization window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct WatchConfig {
    pub path: Option<String>,
    pub recursive: bool,
    pub extensions: Vec<String>,
    /// Smallest file that may enter the queue, in MiB. `0` disables the
    /// floor (SPEC.md §5) — anything smaller than this is dropped by
    /// [`crate::queue::passes_filters`] before it becomes a job.
    pub min_size_mb: u32,
    /// Largest file that may enter the queue, in MiB. `0` means "no
    /// ceiling"; the only remaining bound is then the destination's own
    /// (S3 caps an object at [`MAX_FILE_SIZE_MB`], Drive at 5 TB).
    pub max_size_mb: u32,
    pub stabilize_seconds: u32,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            path: None,
            recursive: false,
            extensions: Vec::new(),
            min_size_mb: 0,
            max_size_mb: 0,
            stabilize_seconds: 3,
        }
    }
}

/// S3 storage class, serialized exactly as AWS names them.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StorageClass {
    #[default]
    Standard,
    IntelligentTiering,
    GlacierIr,
}

/// AWS S3 destination settings (no credentials — those live in the keyring).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct S3Config {
    pub enabled: bool,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub storage_class: StorageClass,
}

impl Default for S3Config {
    fn default() -> Self {
        Self {
            enabled: false,
            region: "us-east-1".to_string(),
            bucket: String::new(),
            prefix: String::new(),
            storage_class: StorageClass::default(),
        }
    }
}

/// Google Drive auth strategy. Service Account only (decision C1) — no OAuth code path.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    #[default]
    ServiceAccount,
}

/// Google Drive destination settings (no credentials — those live in the keyring).
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct GDriveConfig {
    pub enabled: bool,
    pub folder_id: String,
    pub auth_mode: AuthMode,
    pub date_subfolders: bool,
}

/// Nightly bandwidth window: while `enabled`, throttle drops to 0 between `start` and `end`
/// (local time, `"HH:MM"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct NightModeConfig {
    pub enabled: bool,
    pub start: String,
    pub end: String,
}

impl Default for NightModeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            start: "23:00".to_string(),
            end: "06:00".to_string(),
        }
    }
}

/// Bandwidth limits per destination, in megabits per second. `None` means unlimited.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct QosConfig {
    pub gdrive_limit_mbps: Option<f64>,
    pub s3_limit_mbps: Option<f64>,
    pub night_mode: NightModeConfig,
}

/// Retry policy applied by `queue::worker` on transient upload failures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub base_delay_seconds: u32,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay_seconds: 5,
        }
    }
}

/// A single failed validation rule, reported to the renderer via `save_config`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ValidationIssue {
    pub field: String,
    pub message: String,
}

/// Errors from loading/saving `config.json`.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse config.json: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("could not determine the user's home/config directory")]
    NoHome,
}

/// Resolves the per-OS application data directory (never uses the `dirs` crate — env vars only).
///
/// - Windows: `%APPDATA%\osystems-sync`
/// - macOS: `$HOME/Library/Application Support/osystems-sync`
/// - Linux/other Unix: `$XDG_CONFIG_HOME/osystems-sync`, falling back to `$HOME/.config/osystems-sync`
pub fn data_dir() -> Result<PathBuf, ConfigError> {
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").map_err(|_| ConfigError::NoHome)?;
        Ok(PathBuf::from(appdata).join("osystems-sync"))
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").map_err(|_| ConfigError::NoHome)?;
        Ok(PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("osystems-sync"))
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            if !xdg.is_empty() {
                return Ok(PathBuf::from(xdg).join("osystems-sync"));
            }
        }
        let home = std::env::var("HOME").map_err(|_| ConfigError::NoHome)?;
        Ok(PathBuf::from(home).join(".config").join("osystems-sync"))
    }
}

/// Full path to `config.json` inside `dir` (usually [`data_dir`]'s result).
pub fn config_path(dir: &Path) -> PathBuf {
    dir.join("config.json")
}

/// Loads the config from `dir`. A missing file yields [`AppConfig::default`]; a malformed file
/// yields [`ConfigError::Parse`].
pub fn load(dir: &Path) -> Result<AppConfig, ConfigError> {
    let path = config_path(dir);
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let contents = std::fs::read_to_string(&path)?;
    let cfg = serde_json::from_str(&contents)?;
    Ok(cfg)
}

/// Persists `cfg` to `dir` atomically (write to a temp file, then rename), creating `dir` if
/// needed. Watch extensions are normalized (lowercase, no leading dot) before writing.
///
/// `config.json` can carry an AWS access key id, service/window settings, and other
/// operational detail — never a secret proper (those live only in the OS keychain, see
/// `credentials.rs`), but still not something any other local account should be able to
/// read. On Unix, MINOR #8 (T-6.3 audit) hardens the file to `0o600` (owner
/// read/write only) right after the rename; see [`harden_permissions`] for why Windows
/// is a deliberate no-op here.
pub fn save(dir: &Path, cfg: &AppConfig) -> Result<(), ConfigError> {
    std::fs::create_dir_all(dir)?;

    let normalized = normalize_for_save(cfg);
    let json = serde_json::to_string_pretty(&normalized)?;

    let path = config_path(dir);
    let tmp_path = dir.join(format!("config.json.tmp-{}", std::process::id()));
    std::fs::write(&tmp_path, json)?;
    std::fs::rename(&tmp_path, &path)?;
    harden_permissions(&path)?;

    Ok(())
}

/// Restricts `path` to owner read/write only (`0o600`) on Unix. A no-op on Windows: NTFS
/// ACLs on `%APPDATA%\osystems-sync` already default to the owning user, and there is no
/// POSIX mode bit to set there — this function exists so `save` has one call site
/// regardless of platform, rather than a `#[cfg(unix)]` block inline in `save` itself.
#[cfg(unix)]
fn harden_permissions(path: &Path) -> Result<(), ConfigError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn harden_permissions(_path: &Path) -> Result<(), ConfigError> {
    Ok(())
}

/// Normalizes fields that should never fail validation but must be canonicalized on write
/// (SPEC.md §5: "extensions lowercase without dot — normalize on save, not reject").
fn normalize_for_save(cfg: &AppConfig) -> AppConfig {
    let mut normalized = cfg.clone();
    normalized.watch.extensions = normalized
        .watch
        .extensions
        .iter()
        .map(|ext| ext.trim_start_matches('.').to_lowercase())
        .collect();
    normalized
}

/// Validates business rules that `serde` cannot express. Returns every violated rule at once
/// (never short-circuits) so the UI can surface all inline errors together.
pub fn validate(cfg: &AppConfig) -> Result<(), Vec<ValidationIssue>> {
    let mut issues = Vec::new();

    if cfg.s3.enabled {
        if cfg.s3.bucket.trim().is_empty() {
            issues.push(ValidationIssue {
                field: "s3.bucket".to_string(),
                message: "bucket must not be empty when S3 is enabled".to_string(),
            });
        }
        if !is_valid_aws_region(&cfg.s3.region) {
            issues.push(ValidationIssue {
                field: "s3.region".to_string(),
                message: format!("'{}' is not a valid AWS region", cfg.s3.region),
            });
        }
    }

    if cfg.gdrive.enabled && cfg.gdrive.folder_id.trim().is_empty() {
        issues.push(ValidationIssue {
            field: "gdrive.folder_id".to_string(),
            message: "folder_id must not be empty when Google Drive is enabled".to_string(),
        });
    }

    if !(1..=4).contains(&cfg.workers_per_destination) {
        issues.push(ValidationIssue {
            field: "workers_per_destination".to_string(),
            message: "must be between 1 and 4".to_string(),
        });
    }

    if !(1..=10).contains(&cfg.retry.max_attempts) {
        issues.push(ValidationIssue {
            field: "retry.max_attempts".to_string(),
            message: "must be between 1 and 10".to_string(),
        });
    }

    if !(1..=60).contains(&cfg.retry.base_delay_seconds) {
        issues.push(ValidationIssue {
            field: "retry.base_delay_seconds".to_string(),
            message: "must be between 1 and 60".to_string(),
        });
    }

    if let Some(limit) = cfg.qos.gdrive_limit_mbps {
        if !(0.5..=10.0).contains(&limit) {
            issues.push(ValidationIssue {
                field: "qos.gdrive_limit_mbps".to_string(),
                message: "must be null or between 0.5 and 10.0".to_string(),
            });
        }
    }

    if let Some(limit) = cfg.qos.s3_limit_mbps {
        if !(0.5..=10.0).contains(&limit) {
            issues.push(ValidationIssue {
                field: "qos.s3_limit_mbps".to_string(),
                message: "must be null or between 0.5 and 10.0".to_string(),
            });
        }
    }

    // RNF-004: a single file is bounded by the destination, not by an
    // arbitrary app-side number -- S3 multipart tops out at
    // `MAX_FILE_SIZE_MB` (5 TiB) and Drive at 5 TB. `0` means "no ceiling".
    if cfg.watch.max_size_mb > MAX_FILE_SIZE_MB {
        issues.push(ValidationIssue {
            field: "watch.max_size_mb".to_string(),
            message: format!("must be 0 (no limit) or at most {MAX_FILE_SIZE_MB}"),
        });
    }

    if cfg.watch.min_size_mb > MAX_FILE_SIZE_MB {
        issues.push(ValidationIssue {
            field: "watch.min_size_mb".to_string(),
            message: format!("must be 0 (no floor) or at most {MAX_FILE_SIZE_MB}"),
        });
    }

    // `stabilize.rs` polls once a second and needs at least one equal read to
    // call a file stable; `0` would make every file instantly stable, which is
    // the one outcome that module exists to prevent. The upper bound matches
    // the range the Settings field already offers.
    if !(1..=60).contains(&cfg.watch.stabilize_seconds) {
        issues.push(ValidationIssue {
            field: "watch.stabilize_seconds".to_string(),
            message: "must be between 1 and 60".to_string(),
        });
    }

    // A floor above the ceiling would silently drop every file.
    if cfg.watch.max_size_mb > 0 && cfg.watch.min_size_mb > cfg.watch.max_size_mb {
        issues.push(ValidationIssue {
            field: "watch.min_size_mb".to_string(),
            message: "must not exceed watch.max_size_mb".to_string(),
        });
    }

    if !is_valid_hh_mm(&cfg.qos.night_mode.start) {
        issues.push(ValidationIssue {
            field: "qos.night_mode.start".to_string(),
            message: "must match HH:MM".to_string(),
        });
    }

    if !is_valid_hh_mm(&cfg.qos.night_mode.end) {
        issues.push(ValidationIssue {
            field: "qos.night_mode.end".to_string(),
            message: "must match HH:MM".to_string(),
        });
    }

    if issues.is_empty() {
        Ok(())
    } else {
        Err(issues)
    }
}

/// Matches `^[a-z]{2}(-[a-z]+)+-\d$` (e.g. `us-east-1`, `ap-southeast-2`) without pulling in the
/// `regex` crate.
fn is_valid_aws_region(region: &str) -> bool {
    let parts: Vec<&str> = region.split('-').collect();
    if parts.len() < 3 {
        return false;
    }

    let Some((last, middle)) = parts.split_last() else {
        return false;
    };
    let Some((first, mid)) = middle.split_first() else {
        return false;
    };

    let first_ok = first.len() == 2 && first.chars().all(|c| c.is_ascii_lowercase());
    let last_ok = last.len() == 1 && last.chars().all(|c| c.is_ascii_digit());
    let mid_ok = !mid.is_empty()
        && mid
            .iter()
            .all(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase()));

    first_ok && last_ok && mid_ok
}

/// Matches `^\d{2}:\d{2}$` without pulling in the `regex` crate.
fn is_valid_hh_mm(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 5
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2] == b':'
        && bytes[3].is_ascii_digit()
        && bytes[4].is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn valid_config() -> AppConfig {
        AppConfig::default()
    }

    // --- Defaults --------------------------------------------------------

    #[test]
    fn default_config_matches_spec() {
        let cfg = AppConfig::default();

        assert_eq!(cfg.version, 1);

        assert_eq!(cfg.watch.path, None);
        assert!(!cfg.watch.recursive);
        assert!(cfg.watch.extensions.is_empty());
        // Both size filters default to off: the queue accepts any file the
        // destination itself accepts (SPEC.md §5).
        assert_eq!(cfg.watch.min_size_mb, 0);
        assert_eq!(cfg.watch.max_size_mb, 0);
        assert_eq!(cfg.watch.stabilize_seconds, 3);

        assert!(!cfg.s3.enabled);
        assert_eq!(cfg.s3.region, "us-east-1");
        assert_eq!(cfg.s3.bucket, "");
        assert_eq!(cfg.s3.prefix, "");
        assert_eq!(cfg.s3.storage_class, StorageClass::Standard);

        assert!(!cfg.gdrive.enabled);
        assert_eq!(cfg.gdrive.folder_id, "");
        assert_eq!(cfg.gdrive.auth_mode, AuthMode::ServiceAccount);
        assert!(!cfg.gdrive.date_subfolders);

        assert_eq!(cfg.qos.gdrive_limit_mbps, None);
        assert_eq!(cfg.qos.s3_limit_mbps, None);
        assert!(!cfg.qos.night_mode.enabled);
        assert_eq!(cfg.qos.night_mode.start, "23:00");
        assert_eq!(cfg.qos.night_mode.end, "06:00");

        assert_eq!(cfg.retry.max_attempts, 5);
        assert_eq!(cfg.retry.base_delay_seconds, 5);

        assert_eq!(cfg.workers_per_destination, 2);
        assert!(cfg.autostart);
        assert!(cfg.keep_awake);
    }

    #[test]
    fn storage_class_serializes_to_spec_strings() {
        assert_eq!(
            serde_json::to_string(&StorageClass::Standard).unwrap(),
            "\"STANDARD\""
        );
        assert_eq!(
            serde_json::to_string(&StorageClass::IntelligentTiering).unwrap(),
            "\"INTELLIGENT_TIERING\""
        );
        assert_eq!(
            serde_json::to_string(&StorageClass::GlacierIr).unwrap(),
            "\"GLACIER_IR\""
        );
    }

    #[test]
    fn auth_mode_serializes_to_service_account() {
        assert_eq!(
            serde_json::to_string(&AuthMode::ServiceAccount).unwrap(),
            "\"service_account\""
        );
    }

    // --- load/save ---------------------------------------------------------

    #[test]
    fn load_missing_file_returns_default() {
        let dir = tempdir().unwrap();
        let cfg = load(dir.path()).unwrap();
        assert_eq!(cfg, AppConfig::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempdir().unwrap();
        let mut cfg = AppConfig::default();
        cfg.watch.path = Some("/tmp/watched".to_string());
        cfg.watch.extensions = vec!["pdf".to_string(), "csv".to_string()];
        cfg.s3.enabled = true;
        cfg.s3.bucket = "meu-bucket".to_string();
        cfg.s3.prefix = "exportacoes/".to_string();
        cfg.gdrive.enabled = true;
        cfg.gdrive.folder_id = "1AbC".to_string();
        cfg.qos.s3_limit_mbps = Some(5.0);

        save(dir.path(), &cfg).unwrap();
        let loaded = load(dir.path()).unwrap();

        assert_eq!(cfg, loaded);
    }

    #[test]
    fn save_creates_missing_directory() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("nested").join("osystems-sync");
        assert!(!nested.exists());

        save(&nested, &AppConfig::default()).unwrap();

        assert!(config_path(&nested).exists());
    }

    #[cfg(unix)]
    #[test]
    fn save_hardens_config_json_to_owner_read_write_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        save(dir.path(), &AppConfig::default()).unwrap();

        let mode = std::fs::metadata(config_path(dir.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "config.json must be owner read/write only, got {mode:o}"
        );
    }

    #[test]
    fn save_normalizes_extensions_without_rejecting() {
        let dir = tempdir().unwrap();
        let mut cfg = AppConfig::default();
        cfg.watch.extensions = vec![".PDF".to_string(), "Csv".to_string(), "txt".to_string()];

        save(dir.path(), &cfg).unwrap();
        let loaded = load(dir.path()).unwrap();

        assert_eq!(
            loaded.watch.extensions,
            vec!["pdf".to_string(), "csv".to_string(), "txt".to_string()]
        );
        // save() must not fail/reject even though the input wasn't normalized yet.
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn load_malformed_json_returns_parse_error() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path()).unwrap();
        std::fs::write(config_path(dir.path()), "{not valid json").unwrap();

        let err = load(dir.path()).unwrap_err();

        assert!(matches!(err, ConfigError::Parse(_)));
    }

    #[test]
    fn loads_spec_example_json() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path()).unwrap();
        // Copied verbatim from SPEC.md §5.
        let json = r#"{
  "version": 1,
  "watch": {
    "path": "C:\\Users\\x\\Exportacoes",
    "recursive": false,
    "extensions": [],
    "min_size_mb": 0,
    "max_size_mb": 0,
    "stabilize_seconds": 3
  },
  "s3": {
    "enabled": true,
    "region": "us-east-1",
    "bucket": "meu-bucket",
    "prefix": "exportacoes/",
    "storage_class": "STANDARD"
  },
  "gdrive": {
    "enabled": true,
    "folder_id": "1AbC...",
    "auth_mode": "service_account",
    "date_subfolders": false
  },
  "qos": {
    "gdrive_limit_mbps": null,
    "s3_limit_mbps": null,
    "night_mode": { "enabled": false, "start": "23:00", "end": "06:00" }
  },
  "retry": { "max_attempts": 5, "base_delay_seconds": 5 },
  "workers_per_destination": 2,
  "autostart": true,
  "keep_awake": true
}"#;
        std::fs::write(config_path(dir.path()), json).unwrap();

        let cfg = load(dir.path()).unwrap();

        assert_eq!(cfg.s3.bucket, "meu-bucket");
        assert_eq!(cfg.s3.storage_class, StorageClass::Standard);
        assert_eq!(cfg.gdrive.auth_mode, AuthMode::ServiceAccount);
        assert_eq!(cfg.gdrive.folder_id, "1AbC...");
        assert_eq!(
            cfg.watch.path,
            Some("C:\\Users\\x\\Exportacoes".to_string())
        );
    }

    #[test]
    fn missing_keys_load_as_defaults() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path()).unwrap();
        std::fs::write(config_path(dir.path()), "{}").unwrap();

        let cfg = load(dir.path()).unwrap();

        assert_eq!(cfg, AppConfig::default());
    }

    // --- validate: s3 -------------------------------------------------------

    #[test]
    fn validate_passes_for_defaults() {
        assert!(validate(&valid_config()).is_ok());
    }

    #[test]
    fn validate_rejects_empty_bucket_when_s3_enabled() {
        let mut cfg = valid_config();
        cfg.s3.enabled = true;
        cfg.s3.bucket = String::new();

        let issues = validate(&cfg).unwrap_err();

        assert!(issues.iter().any(|i| i.field == "s3.bucket"));
    }

    #[test]
    fn validate_accepts_bucket_when_s3_enabled() {
        let mut cfg = valid_config();
        cfg.s3.enabled = true;
        cfg.s3.bucket = "my-bucket".to_string();

        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn validate_rejects_invalid_region_when_s3_enabled() {
        let mut cfg = valid_config();
        cfg.s3.enabled = true;
        cfg.s3.bucket = "my-bucket".to_string();
        cfg.s3.region = "not-a-region".to_string();

        let issues = validate(&cfg).unwrap_err();

        assert!(issues.iter().any(|i| i.field == "s3.region"));
    }

    #[test]
    fn validate_accepts_valid_region_formats() {
        for region in ["us-east-1", "sa-east-1", "ap-southeast-2", "eu-west-1"] {
            let mut cfg = valid_config();
            cfg.s3.enabled = true;
            cfg.s3.bucket = "my-bucket".to_string();
            cfg.s3.region = region.to_string();

            assert!(validate(&cfg).is_ok(), "region {region} should be valid");
        }
    }

    #[test]
    fn validate_ignores_s3_fields_when_disabled() {
        let mut cfg = valid_config();
        cfg.s3.enabled = false;
        cfg.s3.bucket = String::new();
        cfg.s3.region = "garbage".to_string();

        assert!(validate(&cfg).is_ok());
    }

    // --- validate: gdrive ----------------------------------------------------

    #[test]
    fn validate_rejects_empty_folder_id_when_gdrive_enabled() {
        let mut cfg = valid_config();
        cfg.gdrive.enabled = true;
        cfg.gdrive.folder_id = String::new();

        let issues = validate(&cfg).unwrap_err();

        assert!(issues.iter().any(|i| i.field == "gdrive.folder_id"));
    }

    #[test]
    fn validate_accepts_folder_id_when_gdrive_enabled() {
        let mut cfg = valid_config();
        cfg.gdrive.enabled = true;
        cfg.gdrive.folder_id = "1AbC".to_string();

        assert!(validate(&cfg).is_ok());
    }

    // --- validate: workers / retry -------------------------------------------

    #[test]
    fn validate_rejects_workers_out_of_range() {
        let mut cfg = valid_config();
        cfg.workers_per_destination = 0;
        assert!(validate(&cfg).is_err());

        cfg.workers_per_destination = 5;
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn validate_accepts_workers_in_range() {
        for workers in 1..=4u8 {
            let mut cfg = valid_config();
            cfg.workers_per_destination = workers;
            assert!(validate(&cfg).is_ok());
        }
    }

    #[test]
    fn validate_rejects_max_attempts_out_of_range() {
        let mut cfg = valid_config();
        cfg.retry.max_attempts = 0;
        assert!(validate(&cfg).is_err());

        cfg.retry.max_attempts = 11;
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn validate_accepts_max_attempts_in_range() {
        let mut cfg = valid_config();
        cfg.retry.max_attempts = 1;
        assert!(validate(&cfg).is_ok());

        cfg.retry.max_attempts = 10;
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn validate_rejects_base_delay_out_of_range() {
        let mut cfg = valid_config();
        cfg.retry.base_delay_seconds = 0;
        assert!(validate(&cfg).is_err());

        cfg.retry.base_delay_seconds = 61;
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn validate_accepts_base_delay_in_range() {
        let mut cfg = valid_config();
        cfg.retry.base_delay_seconds = 1;
        assert!(validate(&cfg).is_ok());

        cfg.retry.base_delay_seconds = 60;
        assert!(validate(&cfg).is_ok());
    }

    // --- validate: qos / watch ------------------------------------------------

    #[test]
    fn validate_rejects_qos_limit_out_of_range() {
        let mut cfg = valid_config();
        cfg.qos.s3_limit_mbps = Some(0.1);
        assert!(validate(&cfg).is_err());

        cfg.qos.s3_limit_mbps = Some(11.0);
        assert!(validate(&cfg).is_err());

        let mut cfg2 = valid_config();
        cfg2.qos.gdrive_limit_mbps = Some(0.4);
        assert!(validate(&cfg2).is_err());
    }

    #[test]
    fn validate_accepts_qos_limit_none_or_in_range() {
        let mut cfg = valid_config();
        cfg.qos.gdrive_limit_mbps = None;
        cfg.qos.s3_limit_mbps = Some(5.0);
        assert!(validate(&cfg).is_ok());

        cfg.qos.gdrive_limit_mbps = Some(0.5);
        cfg.qos.s3_limit_mbps = Some(10.0);
        assert!(validate(&cfg).is_ok());
    }

    // `0` is the default and means "no ceiling" -- the destination's own
    // limit is what bounds the file then, not this field.
    #[test]
    fn validate_accepts_max_size_mb_zero_as_no_limit() {
        let mut cfg = valid_config();
        cfg.watch.max_size_mb = 0;

        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn validate_accepts_max_size_mb_positive() {
        let mut cfg = valid_config();
        cfg.watch.max_size_mb = 1;

        assert!(validate(&cfg).is_ok());
    }

    // VULN-005: a single file is still bounded -- by the destination, at
    // `MAX_FILE_SIZE_MB` (5 TiB). Above it must be rejected with the correct
    // field name; the boundary itself must be accepted.
    #[test]
    fn validate_rejects_max_size_mb_above_the_destination_limit() {
        let mut cfg = valid_config();
        cfg.watch.max_size_mb = MAX_FILE_SIZE_MB + 1;

        let err = validate(&cfg).expect_err("above the 5 TiB destination limit");
        assert!(
            err.iter().any(|i| i.field == "watch.max_size_mb"),
            "expected a watch.max_size_mb issue, got {err:?}"
        );
    }

    #[test]
    fn validate_accepts_max_size_mb_at_the_destination_boundary() {
        let mut cfg = valid_config();
        cfg.watch.max_size_mb = MAX_FILE_SIZE_MB;

        assert!(validate(&cfg).is_ok());
    }

    // `stabilize_seconds` had no validation at all: the Settings field offered
    // 1..60 and wired up an error slot that could never fire, while a
    // hand-edited `0` disabled stabilization outright.
    #[test]
    fn validate_rejects_stabilize_seconds_outside_one_to_sixty() {
        for bad in [0, 61] {
            let mut cfg = valid_config();
            cfg.watch.stabilize_seconds = bad;

            let err = validate(&cfg).expect_err("outside 1..=60");
            assert!(
                err.iter().any(|i| i.field == "watch.stabilize_seconds"),
                "expected a watch.stabilize_seconds issue for {bad}, got {err:?}"
            );
        }
    }

    #[test]
    fn validate_accepts_stabilize_seconds_at_both_boundaries() {
        for good in [1, 60] {
            let mut cfg = valid_config();
            cfg.watch.stabilize_seconds = good;

            assert!(validate(&cfg).is_ok(), "{good} must be accepted");
        }
    }

    // The floor is opt-in: `0` disables it, and any value up to the ceiling
    // is fine on its own.
    #[test]
    fn validate_accepts_min_size_mb_zero_or_below_the_ceiling() {
        let mut cfg = valid_config();
        cfg.watch.min_size_mb = 0;
        assert!(validate(&cfg).is_ok());

        cfg.watch.max_size_mb = 5000;
        cfg.watch.min_size_mb = 5000;
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn validate_rejects_min_size_mb_above_the_destination_limit() {
        let mut cfg = valid_config();
        cfg.watch.min_size_mb = MAX_FILE_SIZE_MB + 1;

        let err = validate(&cfg).expect_err("above the 5 TiB destination limit");
        assert!(
            err.iter().any(|i| i.field == "watch.min_size_mb"),
            "expected a watch.min_size_mb issue, got {err:?}"
        );
    }

    // A floor above the ceiling would drop every single file -- reject it
    // instead of letting the queue go silently empty.
    #[test]
    fn validate_rejects_min_size_mb_above_max_size_mb() {
        let mut cfg = valid_config();
        cfg.watch.max_size_mb = 100;
        cfg.watch.min_size_mb = 101;

        let err = validate(&cfg).expect_err("floor above the ceiling");
        assert!(
            err.iter().any(|i| i.field == "watch.min_size_mb"),
            "expected a watch.min_size_mb issue, got {err:?}"
        );
    }

    // ...but an unbounded ceiling (`0`) must not be read as "ceiling of 0".
    #[test]
    fn validate_accepts_min_size_mb_when_max_is_unlimited() {
        let mut cfg = valid_config();
        cfg.watch.max_size_mb = 0;
        cfg.watch.min_size_mb = 100_000;

        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn validate_rejects_malformed_night_mode_times() {
        let mut start_bad = valid_config();
        start_bad.qos.night_mode.start = "23h00".to_string();
        assert!(validate(&start_bad).is_err());

        let mut end_bad = valid_config();
        end_bad.qos.night_mode.end = "6:00".to_string();
        assert!(validate(&end_bad).is_err());
    }

    #[test]
    fn validate_accepts_well_formed_night_mode_times() {
        let mut cfg = valid_config();
        cfg.qos.night_mode.start = "00:00".to_string();
        cfg.qos.night_mode.end = "23:59".to_string();

        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn validate_reports_every_violated_rule_at_once() {
        let mut cfg = valid_config();
        cfg.s3.enabled = true;
        cfg.s3.bucket = String::new();
        cfg.s3.region = "bad".to_string();
        cfg.workers_per_destination = 0;
        cfg.retry.max_attempts = 99;

        let issues = validate(&cfg).unwrap_err();

        assert!(issues.len() >= 4);
    }
}
