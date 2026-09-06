//! `core::state` — SQLite (WAL) persistence for `files` / `jobs` / `events`
//! (SPEC.md §5, §6; PLAN.md T-1.2).
//!
//! Synchronous `rusqlite` on purpose: this module never spawns a thread or
//! imports `tauri`/Tokio for its own sake. The caller (commands, watcher,
//! worker) is responsible for running [`Repo`] methods inside
//! `spawn_blocking`.

mod model;
mod repo;

pub use model::{
    AppStatus, AuthRequired, Destination, DestinationHealth, DestinationsHealth, EventRow, FileRow,
    JobRow, JobSide, JobStatus, JobView, ListJobsPage, ListJobsQuery, RecoveredJob, StatusCounts,
    Throughput, UploadProgress, UpsertOutcome,
};
pub use repo::{Repo, StateError};
