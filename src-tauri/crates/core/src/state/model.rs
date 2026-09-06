//! Data types for `core::state`.
//!
//! Types that cross IPC (`JobView`, `JobSide`, `JobStatus`, `Destination`,
//! `ListJobsQuery`, `ListJobsPage`, `StatusCounts`) derive `Serialize` /
//! `Deserialize` / `TS` and are annotated `#[ts(export)]`, mirroring the
//! convention already used by `core::config::AppConfig` (T-1.1). Internal
//! row/outcome types (`FileRow`, `JobRow`, `UpsertOutcome`, `RecoveredJob`,
//! `EventRow`) never leave the core crate, so they stay plain Rust structs.

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Upload destination. Serializes as `"s3"` / `"gdrive"` (SPEC.md §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Destination {
    S3,
    GDrive,
}

impl Destination {
    pub const fn as_str(self) -> &'static str {
        match self {
            Destination::S3 => "s3",
            Destination::GDrive => "gdrive",
        }
    }
}

impl ToSql for Destination {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_str()))
    }
}

impl FromSql for Destination {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value.as_str()? {
            "s3" => Ok(Destination::S3),
            "gdrive" => Ok(Destination::GDrive),
            other => Err(FromSqlError::Other(
                format!("invalid destination: {other}").into(),
            )),
        }
    }
}

/// Job lifecycle status (SPEC.md §5 `jobs.status` CHECK constraint).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum JobStatus {
    Pending,
    Uploading,
    Paused,
    Cancelled,
    Done,
    Failed,
}

impl JobStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            JobStatus::Pending => "pending",
            JobStatus::Uploading => "uploading",
            JobStatus::Paused => "paused",
            JobStatus::Cancelled => "cancelled",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
        }
    }

    pub const ALL: [JobStatus; 6] = [
        JobStatus::Pending,
        JobStatus::Uploading,
        JobStatus::Paused,
        JobStatus::Cancelled,
        JobStatus::Done,
        JobStatus::Failed,
    ];
}

impl ToSql for JobStatus {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_str()))
    }
}

impl FromSql for JobStatus {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value.as_str()? {
            "pending" => Ok(JobStatus::Pending),
            "uploading" => Ok(JobStatus::Uploading),
            "paused" => Ok(JobStatus::Paused),
            "cancelled" => Ok(JobStatus::Cancelled),
            "done" => Ok(JobStatus::Done),
            "failed" => Ok(JobStatus::Failed),
            other => Err(FromSqlError::Other(
                format!("invalid job status: {other}").into(),
            )),
        }
    }
}

/// Raw `files` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub id: String,
    pub path: String,
    pub sha256: String,
    pub size: i64,
    pub mtime: String,
    pub detected_at: String,
}

/// Raw `jobs` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRow {
    pub id: String,
    pub file_id: String,
    pub destination: Destination,
    pub status: JobStatus,
    pub attempts: i64,
    pub next_attempt_at: Option<String>,
    pub remote_id: Option<String>,
    pub remote_state: Option<String>,
    pub last_error: Option<String>,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Result of `Repo::upsert_file_and_enqueue` (SPEC.md §5 "Regras de escrita").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpsertOutcome {
    /// `path` was never seen before: a new `files` row plus 2 pending jobs
    /// (`s3`, `gdrive`) were created.
    Created,
    /// `path` already existed with the same `sha256`: nothing changed.
    Unchanged,
    /// `path` already existed with a different `sha256`: the `files` row was
    /// updated in place and both jobs were reset to `pending` (never a second
    /// pair of jobs — would violate `UNIQUE(file_id, destination)`).
    Rehashed,
}

/// One side (destination) of a `JobView` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobSide {
    pub job_id: String,
    pub status: JobStatus,
    #[ts(type = "number")]
    pub attempts: i64,
    pub next_attempt_at: Option<String>,
    pub remote_id: Option<String>,
    pub last_error: Option<String>,
    pub updated_at: String,
}

/// One row of `list_jobs`: the 2 jobs of a file (`s3` + `gdrive`) aggregated
/// into a single view (SPEC.md §7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobView {
    pub file_id: String,
    pub path: String,
    pub name: String,
    #[ts(type = "number")]
    pub size: i64,
    pub sha256: String,
    pub detected_at: String,
    pub gdrive: JobSide,
    pub s3: JobSide,
}

/// `list_jobs` query arguments (SPEC.md §7).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ListJobsQuery {
    #[serde(default)]
    pub statuses: Option<Vec<JobStatus>>,
    #[serde(default)]
    pub destination: Option<Destination>,
    #[serde(default)]
    pub include_archived: bool,
    #[ts(type = "number")]
    pub limit: i64,
    #[ts(type = "number")]
    pub offset: i64,
}

/// `list_jobs` result page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ListJobsPage {
    pub items: Vec<JobView>,
    #[ts(type = "number")]
    pub total: i64,
}

/// Aggregate counters for the Dashboard KPIs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StatusCounts {
    #[ts(type = "number")]
    pub pending: i64,
    #[ts(type = "number")]
    pub uploading: i64,
    #[ts(type = "number")]
    pub paused: i64,
    #[ts(type = "number")]
    pub cancelled: i64,
    #[ts(type = "number")]
    pub done: i64,
    #[ts(type = "number")]
    pub failed: i64,
    /// Sum of `files.size` for every detected file.
    #[ts(type = "number")]
    pub bytes_total: i64,
    /// Sum of `files.size` for files whose *both* jobs (`s3` and `gdrive`)
    /// are `done`.
    #[ts(type = "number")]
    pub bytes_done: i64,
}

/// One `jobs` row recovered on boot (`status` was `uploading`, reset to
/// `pending`). The caller uses `remote_state` to abort any dangling
/// multipart/resumable upload before the job is retried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredJob {
    pub job_id: String,
    pub destination: Destination,
    pub remote_state: Option<String>,
}

/// Raw `events` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRow {
    pub id: i64,
    pub ts: String,
    pub level: String,
    pub job_id: Option<String>,
    pub message: String,
}

// --- IPC event payloads (SPEC §7) ---
//
// The types below never touch `state.db` — they only exist to give the events emitted
// by `src-tauri/src/events.rs` (T-2.7) a typed, `#[ts(export)]`-mirrored shape. They
// live in `core` rather than the app crate purely so `ts-rs` generates their
// `src/types/generated/*.ts` bindings the same way every other IPC type does (T-1.4).

/// Health of a single upload destination, refreshed by the periodic connectivity ping
/// (SPEC.md §7 `get_status`; PLAN.md T-4.9). Nested twice inside [`AppStatus`], once
/// per [`Destination`].
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DestinationHealth {
    pub online: bool,
    pub auth_required: bool,
    /// Round-trip time of the last successful ping, in milliseconds. `None` until the
    /// first ping completes.
    pub latency_ms: Option<u32>,
}

/// [`DestinationHealth`] for both upload destinations (SPEC.md §7 `get_status`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DestinationsHealth {
    pub gdrive: DestinationHealth,
    pub s3: DestinationHealth,
}

/// Snapshot returned by the `get_status` command and re-broadcast as the
/// `status-changed` event (SPEC.md §7) whenever it changes.
///
/// `bytes_total`/`bytes_done` are not separate fields here: they already live on
/// [`StatusCounts`] (reused as `counts_by_status`), so exposing them again at the top
/// level would just be the same two numbers under two names.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AppStatus {
    pub watcher_paused: bool,
    pub destinations: DestinationsHealth,
    pub counts_by_status: StatusCounts,
    pub core_version: String,
    pub build_target: String,
}

/// `upload-progress` event payload (SPEC.md §7), throttled by the emitter to at most
/// once per 500 ms per job.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UploadProgress {
    pub job_id: String,
    #[ts(type = "number")]
    pub sent: u64,
    #[ts(type = "number")]
    pub total: u64,
    #[ts(type = "number")]
    pub rate_bps: u64,
}

/// `throughput` event payload (SPEC.md §7), emitted once per second with the current
/// aggregate transfer rate and the configured QoS ceilings (`None` while a destination
/// is unlimited).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Throughput {
    #[ts(type = "number")]
    pub total_bps: u64,
    #[ts(type = "number")]
    pub gdrive_bps: u64,
    #[ts(type = "number")]
    pub s3_bps: u64,
    // NOTE: `#[ts(type = "...")]` replaces the *entire* generated type for the field,
    // including the `| null` that ts-rs would otherwise add for `Option<T>` — so the
    // override has to spell out the union itself, or an unlimited destination
    // (`None`, serialized as JSON `null`) would type as a bare `number` on the
    // frontend and silently lie about nullability.
    #[ts(type = "number | null")]
    pub limit_gdrive_bps: Option<u64>,
    #[ts(type = "number | null")]
    pub limit_s3_bps: Option<u64>,
}

/// `auth-required` event payload (SPEC.md §7): a destination started failing with a
/// 401/403 (or, for the Drive Service Account, a clock-skew `invalid_grant`) and its
/// jobs were paused until the user re-authenticates.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AuthRequired {
    pub destination: Destination,
    /// Human-readable remediation hint: the Service Account email to share the Drive
    /// folder with, or the missing IAM policy, depending on `destination`.
    pub hint: String,
}

#[cfg(test)]
mod ipc_event_payload_tests {
    use super::*;

    #[test]
    fn app_status_serializes_with_the_exact_key_names_from_spec_section_7() {
        let status = AppStatus {
            watcher_paused: false,
            destinations: DestinationsHealth {
                gdrive: DestinationHealth {
                    online: true,
                    auth_required: false,
                    latency_ms: Some(42),
                },
                s3: DestinationHealth {
                    online: false,
                    auth_required: true,
                    latency_ms: None,
                },
            },
            counts_by_status: StatusCounts::default(),
            core_version: "0.1.0".to_string(),
            build_target: "x86_64-pc-windows-msvc".to_string(),
        };

        let value = serde_json::to_value(&status).expect("AppStatus must serialize");
        let mut keys: Vec<&str> = value
            .as_object()
            .expect("AppStatus serializes as a JSON object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "build_target",
                "core_version",
                "counts_by_status",
                "destinations",
                "watcher_paused",
            ]
        );

        let mut destination_keys: Vec<&str> = value["destinations"]
            .as_object()
            .expect("destinations serializes as a JSON object")
            .keys()
            .map(String::as_str)
            .collect();
        destination_keys.sort_unstable();
        assert_eq!(destination_keys, vec!["gdrive", "s3"]);

        let mut gdrive_keys: Vec<&str> = value["destinations"]["gdrive"]
            .as_object()
            .expect("gdrive health serializes as a JSON object")
            .keys()
            .map(String::as_str)
            .collect();
        gdrive_keys.sort_unstable();
        assert_eq!(gdrive_keys, vec!["auth_required", "latency_ms", "online"]);
    }
}
