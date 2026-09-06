//! SQLite-backed repository for `files` / `jobs` / `events` (SPEC.md §5, §6).
//!
//! Synchronous `rusqlite` on purpose — this module never touches Tokio.
//! Callers (commands, watcher, worker) are responsible for running these
//! methods inside `spawn_blocking` (CLAUDE.md: "Não bloquear o runtime Tokio
//! com I/O síncrono pesado").

use std::path::Path;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, ToSql};
use uuid::Uuid;

use super::model::{
    Destination, EventRow, FileRow, JobRow, JobSide, JobStatus, JobView, ListJobsPage,
    ListJobsQuery, RecoveredJob, StatusCounts, UpsertOutcome,
};

/// Errors surfaced by `core::state`.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

type Result<T> = std::result::Result<T, StateError>;

const SCHEMA_SQL: &str = include_str!("schema.sql");
const CURRENT_SCHEMA_VERSION: i64 = 1;

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Owns the `state.db` connection and every read/write against it.
pub struct Repo {
    conn: Connection,
}

/// A file that still has at least one job the filter sweep may act on
/// (`pending`/`paused`/`failed`, archived or not).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepCandidate {
    pub file_id: String,
    /// Canonical path as stored in `files.path`.
    pub path: String,
    /// Size in bytes as of the last intake — refreshed by every rescan that
    /// sees the file change, so the sweep needs no `stat` of its own.
    pub size: i64,
}

/// Candidates for [`Repo::reconcile_jobs_with_filters`].
///
/// Deliberately **not** limited to what the current walk found on disk: a
/// file that fell out of scope because `recursive` went `true → false` is
/// never walked again, and would otherwise sit in the queue forever. The
/// `EXISTS` keeps the scan bounded to the active queue instead of all history,
/// and archived rows are included so the sweep can also restore them.
const SWEEPABLE_FILES_SQL: &str = "SELECT f.id, f.path, f.size FROM files f \
     WHERE EXISTS ( \
        SELECT 1 FROM jobs j \
        WHERE j.file_id = f.id \
          AND j.status IN ('pending', 'paused', 'failed') \
     )";

fn map_sweep_candidate(row: &rusqlite::Row<'_>) -> rusqlite::Result<SweepCandidate> {
    Ok(SweepCandidate {
        file_id: row.get(0)?,
        path: row.get(1)?,
        size: row.get(2)?,
    })
}

impl Repo {
    /// Opens (creating if needed) the database file at `path` in WAL mode.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        run_migrations(&conn)?;
        let repo = Self { conn };
        // One-shot cleanup for rows written before `paths::canonicalize_clean`
        // existed: any `files.path` still carrying a Windows verbatim (`\\?\`)
        // prefix from an older build gets normalized on the next open. Cheap
        // (a `LIKE` scan) and idempotent, so it is safe to run unconditionally
        // every time rather than gating it behind a schema-version bump.
        repo.normalize_verbatim_paths()?;
        Ok(repo)
    }

    /// In-memory database for tests. SQLite cannot use WAL for `:memory:`
    /// (it silently stays on the default journal mode), so only
    /// `foreign_keys` is set here.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", true)?;
        run_migrations(&conn)?;
        Ok(Self { conn })
    }

    /// Implements the SPEC.md §5 "Regras de escrita" for a detected file:
    /// - new `path` → insert `files` row + 2 `pending` jobs (`s3`, `gdrive`).
    /// - same `path`, same `sha256` → nothing to do (`Unchanged`), though
    ///   `size`/`mtime` are refreshed so a later rescan does not think the
    ///   file changed.
    /// - same `path`, different `sha256` → update the `files` row in place
    ///   and reset both jobs to `pending` (never a second pair of jobs —
    ///   would violate `UNIQUE(file_id, destination)`).
    pub fn upsert_file_and_enqueue(
        &self,
        path: &str,
        sha256: &str,
        size: i64,
        mtime: &str,
    ) -> Result<UpsertOutcome> {
        let tx = self.conn.unchecked_transaction()?;
        let now = now_iso();

        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT id, sha256 FROM files WHERE path = ?1",
                params![path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        let outcome = match existing {
            None => {
                let file_id = Uuid::new_v4().to_string();
                tx.execute(
                    "INSERT INTO files (id, path, sha256, size, mtime, detected_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![file_id, path, sha256, size, mtime, now],
                )?;
                for dest in [Destination::S3, Destination::GDrive] {
                    tx.execute(
                        "INSERT INTO jobs (id, file_id, destination, status, attempts, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, 'pending', 0, ?4, ?4)",
                        params![Uuid::new_v4().to_string(), file_id, dest, now],
                    )?;
                }
                UpsertOutcome::Created
            }
            Some((file_id, existing_sha)) if existing_sha == sha256 => {
                tx.execute(
                    "UPDATE files SET size = ?1, mtime = ?2 WHERE id = ?3",
                    params![size, mtime, file_id],
                )?;
                UpsertOutcome::Unchanged
            }
            Some((file_id, _)) => {
                tx.execute(
                    "UPDATE files SET sha256 = ?1, size = ?2, mtime = ?3 WHERE id = ?4",
                    params![sha256, size, mtime, file_id],
                )?;
                tx.execute(
                    "UPDATE jobs SET status = 'pending', attempts = 0, next_attempt_at = NULL, \
                     remote_id = NULL, remote_state = NULL, archived_at = NULL, last_error = NULL, \
                     updated_at = ?1 \
                     WHERE file_id = ?2",
                    params![now, file_id],
                )?;
                UpsertOutcome::Rehashed
            }
        };

        tx.commit()?;
        Ok(outcome)
    }

    /// Claims the oldest due `pending` job for `dest` (SPEC.md §6
    /// `worker_loop`): selects it and flips it to `uploading` in one
    /// transaction so two workers can never claim the same job.
    pub fn claim_next(&self, dest: Destination, now: &str) -> Result<Option<JobRow>> {
        let tx = self.conn.unchecked_transaction()?;

        let claimed = tx
            .query_row(
                "SELECT id, file_id, destination, status, attempts, next_attempt_at, remote_id, \
                        remote_state, last_error, archived_at, created_at, updated_at \
                 FROM jobs \
                 WHERE destination = ?1 AND status = 'pending' \
                   AND archived_at IS NULL \
                   AND (next_attempt_at IS NULL OR next_attempt_at <= ?2) \
                 ORDER BY created_at \
                 LIMIT 1",
                params![dest, now],
                map_job_row,
            )
            .optional()?;

        let Some(job) = claimed else {
            tx.commit()?;
            return Ok(None);
        };

        tx.execute(
            "UPDATE jobs SET status = 'uploading', updated_at = ?1 WHERE id = ?2 AND status = 'pending'",
            params![now, job.id],
        )?;
        tx.commit()?;

        Ok(Some(JobRow {
            status: JobStatus::Uploading,
            updated_at: now.to_string(),
            ..job
        }))
    }

    pub fn mark_done(
        &self,
        job_id: &str,
        remote_id: &str,
        remote_state_json: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET status = 'done', remote_id = ?1, remote_state = ?2, \
             last_error = NULL, updated_at = ?3 WHERE id = ?4",
            params![remote_id, remote_state_json, now_iso(), job_id],
        )?;
        Ok(())
    }

    pub fn mark_retry(&self, job_id: &str, attempt: i64, next_at: &str, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET status = 'pending', attempts = ?1, next_attempt_at = ?2, \
             last_error = ?3, updated_at = ?4 WHERE id = ?5",
            params![attempt, next_at, error, now_iso(), job_id],
        )?;
        Ok(())
    }

    pub fn mark_failed(&self, job_id: &str, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET status = 'failed', last_error = ?1, updated_at = ?2 WHERE id = ?3",
            params![error, now_iso(), job_id],
        )?;
        Ok(())
    }

    pub fn set_status(&self, job_id: &str, status: JobStatus) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status, now_iso(), job_id],
        )?;
        Ok(())
    }

    pub fn set_remote_state(&self, job_id: &str, json: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET remote_state = ?1, updated_at = ?2 WHERE id = ?3",
            params![json, now_iso(), job_id],
        )?;
        Ok(())
    }

    /// RF-033: reset a single job back to `pending` (e.g. a `failed` row the
    /// user clicked "reenviar" on).
    pub fn retry_job(&self, job_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET status = 'pending', attempts = 0, next_attempt_at = NULL, \
             last_error = NULL, updated_at = ?1 WHERE id = ?2",
            params![now_iso(), job_id],
        )?;
        Ok(())
    }

    /// RF-034: reset every visible `failed` job back to `pending`. Returns how
    /// many jobs were affected.
    ///
    /// Archived jobs are skipped: an archived row is out of the user's view
    /// (`list_jobs` hides it, `clear_completed` and the filter sweep put it
    /// there), so resurrecting it from a button labelled "reenviar falhas"
    /// would undo a decision the user never revisited.
    pub fn retry_all_failed(&self) -> Result<u32> {
        let n = self.conn.execute(
            "UPDATE jobs SET status = 'pending', attempts = 0, next_attempt_at = NULL, \
             last_error = NULL, updated_at = ?1 \
             WHERE status = 'failed' AND archived_at IS NULL",
            params![now_iso()],
        )?;
        Ok(n as u32)
    }

    /// RF-035: cancel a `pending`/`uploading` job (terminal status).
    pub fn cancel_job(&self, job_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET status = 'cancelled', updated_at = ?1 WHERE id = ?2",
            params![now_iso(), job_id],
        )?;
        Ok(())
    }

    /// RF-036: archives (`archived_at = now`) every job of every file whose
    /// **both** jobs (`s3` and `gdrive`) are `done`. Returns the number of
    /// files archived.
    pub fn clear_completed(&self, now: &str) -> Result<u32> {
        let tx = self.conn.unchecked_transaction()?;

        let affected: i64 = tx.query_row(
            "SELECT COUNT(*) FROM ( \
                SELECT file_id FROM jobs \
                WHERE archived_at IS NULL \
                GROUP BY file_id \
                HAVING COUNT(*) = 2 AND SUM(CASE WHEN status = 'done' THEN 1 ELSE 0 END) = 2 \
             )",
            [],
            |row| row.get(0),
        )?;

        tx.execute(
            "UPDATE jobs SET archived_at = ?1 \
             WHERE archived_at IS NULL AND file_id IN ( \
                SELECT file_id FROM jobs \
                WHERE archived_at IS NULL \
                GROUP BY file_id \
                HAVING COUNT(*) = 2 AND SUM(CASE WHEN status = 'done' THEN 1 ELSE 0 END) = 2 \
             )",
            params![now],
        )?;

        tx.commit()?;
        Ok(affected as u32)
    }

    /// One row of [`Repo::files_with_sweepable_jobs`] — a file whose queue
    /// entry may still be reconciled against the current filters.
    pub fn files_with_sweepable_jobs(&self) -> Result<Vec<SweepCandidate>> {
        let mut stmt = self.conn.prepare(SWEEPABLE_FILES_SQL)?;
        let rows = stmt.query_map([], map_sweep_candidate)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// RF-004 (reconciliação de filtros): brings the queue back in line with
    /// the current `watch` filters, in both directions, in one transaction.
    ///
    /// `keep(path, size)` decides whether a file still belongs in the queue —
    /// the caller builds it from `queue::passes_filters` plus a `WatchConfig`.
    /// Taking a closure keeps `repo.rs` free of any dependency on `config` or
    /// `queue`, and keeps the SELECT and the UPDATEs inside the same
    /// `unchecked_transaction`, which is what makes the sweep safe against a
    /// worker: `claim_next` goes through the same `Mutex<Repo>`, so no row can
    /// flip `pending → uploading` between the read and the write.
    ///
    /// - Files `keep` rejects have their **visible** `pending`/`paused`/`failed`
    ///   jobs archived (`archived_at = now`).
    /// - Files `keep` accepts have their **archived** `pending`/`paused`/`failed`
    ///   jobs restored (`archived_at = NULL`) — without this, widening a filter
    ///   back would be a one-way door: `rescan`'s `size`+`mtime` fast path
    ///   returns `Unchanged` for a file that did not move on disk, so nothing
    ///   else would ever bring it back.
    ///
    /// `uploading` is never touched (a transfer is in flight) and neither is
    /// `done`/`cancelled` (history, and `clear_completed`'s territory). Status
    /// itself is never changed — `archived_at` alone hides the row from
    /// `list_jobs`, `status_counts`, `claim_next` and `retry_all_failed`, so
    /// there is no need to overload `cancelled`, which means "the user
    /// cancelled this" and feeds its own KPI.
    ///
    /// Returns `(archived, restored)` counted in **files**, mirroring
    /// `clear_completed`'s per-file count.
    pub fn reconcile_jobs_with_filters<F>(&self, now: &str, keep: F) -> Result<(u32, u32)>
    where
        F: Fn(&str, i64) -> bool,
    {
        let tx = self.conn.unchecked_transaction()?;

        let candidates: Vec<SweepCandidate> = {
            let mut stmt = tx.prepare(SWEEPABLE_FILES_SQL)?;
            let rows = stmt.query_map([], map_sweep_candidate)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        let mut archived = 0u32;
        let mut restored = 0u32;

        for SweepCandidate {
            file_id,
            path,
            size,
        } in candidates
        {
            if keep(&path, size) {
                let n = tx.execute(
                    "UPDATE jobs SET archived_at = NULL, updated_at = ?1 \
                     WHERE file_id = ?2 AND archived_at IS NOT NULL \
                       AND status IN ('pending', 'paused', 'failed')",
                    params![now, file_id],
                )?;
                if n > 0 {
                    restored += 1;
                }
            } else {
                let n = tx.execute(
                    "UPDATE jobs SET archived_at = ?1, updated_at = ?1 \
                     WHERE file_id = ?2 AND archived_at IS NULL \
                       AND status IN ('pending', 'paused', 'failed')",
                    params![now, file_id],
                )?;
                if n > 0 {
                    archived += 1;
                }
            }
        }

        tx.commit()?;
        Ok((archived, restored))
    }

    /// RNF-006: boot crash recovery. Every job stuck in `uploading` (the
    /// process died mid-upload) goes back to `pending`; the caller uses the
    /// returned `remote_state` to `AbortMultipartUpload` (S3) / discard the
    /// resumable session (Drive) before the job is retried, so no orphaned
    /// upload is left billing on the remote side.
    pub fn recover_on_boot(&self) -> Result<Vec<RecoveredJob>> {
        let tx = self.conn.unchecked_transaction()?;

        let recovered: Vec<RecoveredJob> = {
            let mut stmt = tx.prepare(
                "SELECT id, destination, remote_state FROM jobs WHERE status = 'uploading'",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(RecoveredJob {
                    job_id: row.get(0)?,
                    destination: row.get(1)?,
                    remote_state: row.get(2)?,
                })
            })?;
            rows.collect::<rusqlite::Result<_>>()?
        };

        tx.execute(
            "UPDATE jobs SET status = 'pending', updated_at = ?1 WHERE status = 'uploading'",
            params![now_iso()],
        )?;
        tx.commit()?;

        Ok(recovered)
    }

    /// SPEC.md §7: one row per file, aggregating its `s3` and `gdrive` jobs.
    /// `statuses` matches if *either* side has one of the given statuses,
    /// unless `destination` narrows the check to that side only.
    pub fn list_jobs(&self, q: &ListJobsQuery) -> Result<ListJobsPage> {
        let (where_sql, mut params_vec) = build_where(q);

        let count_sql = format!(
            "SELECT COUNT(*) FROM files f \
             JOIN jobs js3 ON js3.file_id = f.id AND js3.destination = 's3' \
             JOIN jobs jg ON jg.file_id = f.id AND jg.destination = 'gdrive' \
             WHERE {where_sql}"
        );
        let total: i64 = self.conn.query_row(
            &count_sql,
            rusqlite::params_from_iter(params_vec.iter()),
            |row| row.get(0),
        )?;

        params_vec.push(Box::new(q.limit));
        params_vec.push(Box::new(q.offset));

        let select_sql = format!(
            "SELECT f.id, f.path, f.size, f.sha256, f.detected_at, \
                    js3.id, js3.status, js3.attempts, js3.next_attempt_at, js3.remote_id, js3.last_error, js3.updated_at, \
                    jg.id, jg.status, jg.attempts, jg.next_attempt_at, jg.remote_id, jg.last_error, jg.updated_at \
             FROM files f \
             JOIN jobs js3 ON js3.file_id = f.id AND js3.destination = 's3' \
             JOIN jobs jg ON jg.file_id = f.id AND jg.destination = 'gdrive' \
             WHERE {where_sql} \
             ORDER BY f.detected_at DESC, f.id DESC \
             LIMIT ? OFFSET ?"
        );

        let mut stmt = self.conn.prepare(&select_sql)?;
        let items = stmt
            .query_map(rusqlite::params_from_iter(params_vec.iter()), map_job_view)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(ListJobsPage { items, total })
    }

    /// Single-file `JobView` lookup, by `files.id`. Used by the intake pipeline right after
    /// `upsert_file_and_enqueue`/`intake` to emit `job-updated` without a second full query.
    /// Returns `None` if the file has no s3+gdrive job pair (should not happen for a `file_id`
    /// freshly returned by intake, but callers may race a deletion).
    pub fn job_view_for_file(&self, file_id: &str) -> Result<Option<JobView>> {
        let sql = "SELECT f.id, f.path, f.size, f.sha256, f.detected_at, \
                    js3.id, js3.status, js3.attempts, js3.next_attempt_at, js3.remote_id, js3.last_error, js3.updated_at, \
                    jg.id, jg.status, jg.attempts, jg.next_attempt_at, jg.remote_id, jg.last_error, jg.updated_at \
             FROM files f \
             JOIN jobs js3 ON js3.file_id = f.id AND js3.destination = 's3' \
             JOIN jobs jg ON jg.file_id = f.id AND jg.destination = 'gdrive' \
             WHERE f.id = ?1";
        Ok(self
            .conn
            .query_row(sql, params![file_id], map_job_view)
            .optional()?)
    }

    /// Single-file `JobView` lookup, by a `jobs.id` (either destination's row for that
    /// file). Resolves the owning `file_id` first, then delegates to
    /// [`Repo::job_view_for_file`] — added for T-3.9's `AppEvents::job_updated(job_id)`,
    /// which only has the `job_id` the worker/health code operates on, never the
    /// `file_id`. Returns `None` if `job_id` doesn't exist or its file lost its job
    /// pair (same race as `job_view_for_file`).
    pub fn job_view_for_job(&self, job_id: &str) -> Result<Option<JobView>> {
        let file_id: Option<String> = self
            .conn
            .query_row(
                "SELECT file_id FROM jobs WHERE id = ?1",
                params![job_id],
                |row| row.get(0),
            )
            .optional()?;
        match file_id {
            Some(file_id) => self.job_view_for_file(&file_id),
            None => Ok(None),
        }
    }

    /// Resolves the `jobs.id` of the row currently `uploading` for `dest` whose owning
    /// file's basename equals `name`. Added for T-3.9's S3 `StateSink`: the sink only
    /// receives `remote_name` (== `UploadRequest::remote_name`, the file's basename —
    /// see `worker::run_job`), never the `job_id` it needs to call
    /// [`Repo::set_remote_state`] with. There is no `name` column on `jobs`/`files` (the
    /// display name is always derived from `files.path` at query time, same as
    /// [`map_job_view`]), so this scans the small set of currently-`uploading` rows for
    /// `dest` (bounded by `workers_per_destination`, at most 4) rather than fighting
    /// SQLite over path manipulation. Returns `None` if no `uploading` job for `dest`
    /// has that basename (e.g. it finished or was cancelled between the multipart part
    /// completing and the sink firing — the persisted state is then simply dropped,
    /// which is safe: `recover_on_boot` will re-derive it from a fresh `ListParts` call
    /// on the next resume).
    pub fn uploading_job_id_for_name(
        &self,
        dest: Destination,
        name: &str,
    ) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT j.id, f.path FROM jobs j \
             JOIN files f ON f.id = j.file_id \
             WHERE j.destination = ?1 AND j.status = 'uploading'",
        )?;
        let mut rows = stmt.query(params![dest.as_str()])?;
        while let Some(row) = rows.next()? {
            let job_id: String = row.get(0)?;
            let path: String = row.get(1)?;
            let basename = Path::new(&path)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned());
            if basename.as_deref() == Some(name) {
                return Ok(Some(job_id));
            }
        }
        Ok(None)
    }

    /// Full `jobs` row for a single `job_id`, or `None` if it doesn't exist. Added for
    /// T-3.9's `commands::queue::{cancel_job, open_remote}`: both need the row's
    /// `destination` (to pick the right uploader for `abort`/to know which URL scheme
    /// to build) and raw `remote_state` JSON (S3's `key`/`upload_id`, Drive's
    /// `web_view_link`) — neither of which [`JobView`]/[`JobSide`] exposes, since those
    /// are shaped for the UI, not for driving an uploader call.
    pub fn job_by_id(&self, job_id: &str) -> Result<Option<JobRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, file_id, destination, status, attempts, next_attempt_at, \
                 remote_id, remote_state, last_error, archived_at, created_at, updated_at \
                 FROM jobs WHERE id = ?1",
                params![job_id],
                map_job_row,
            )
            .optional()?)
    }

    /// Aggregate counters for the Dashboard KPIs.
    pub fn status_counts(&self) -> Result<StatusCounts> {
        let mut counts = StatusCounts::default();

        {
            let mut stmt = self.conn.prepare(
                "SELECT status, COUNT(*) FROM jobs \
                     WHERE archived_at IS NULL GROUP BY status",
            )?;
            let rows = stmt.query_map([], |row| {
                let status: JobStatus = row.get(0)?;
                let n: i64 = row.get(1)?;
                Ok((status, n))
            })?;
            for row in rows {
                let (status, n) = row?;
                match status {
                    JobStatus::Pending => counts.pending = n,
                    JobStatus::Uploading => counts.uploading = n,
                    JobStatus::Paused => counts.paused = n,
                    JobStatus::Cancelled => counts.cancelled = n,
                    JobStatus::Done => counts.done = n,
                    JobStatus::Failed => counts.failed = n,
                }
            }
        }

        counts.bytes_total = self.conn.query_row(
            "SELECT COALESCE(SUM(f.size), 0) FROM files f WHERE EXISTS ( \
                SELECT 1 FROM jobs j WHERE j.file_id = f.id AND j.archived_at IS NULL \
             )",
            [],
            |row| row.get(0),
        )?;

        counts.bytes_done = self.conn.query_row(
            "SELECT COALESCE(SUM(f.size), 0) FROM files f WHERE f.id IN ( \
                SELECT file_id FROM jobs \
                WHERE archived_at IS NULL \
                GROUP BY file_id \
                HAVING COUNT(*) = 2 AND SUM(CASE WHEN status = 'done' THEN 1 ELSE 0 END) = 2 \
             )",
            [],
            |row| row.get(0),
        )?;

        Ok(counts)
    }

    pub fn insert_event(&self, level: &str, job_id: Option<&str>, message: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO events (ts, level, job_id, message) VALUES (?1, ?2, ?3, ?4)",
            params![now_iso(), level, job_id, message],
        )?;
        Ok(())
    }

    pub fn recent_events(&self, limit: i64) -> Result<Vec<EventRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, ts, level, job_id, message FROM events ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok(EventRow {
                id: row.get(0)?,
                ts: row.get(1)?,
                level: row.get(2)?,
                job_id: row.get(3)?,
                message: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// `files.id` for a given `path`, or `None` if no row exists yet.
    ///
    /// This replaces the `queue::find_file_id` workaround (a full
    /// `list_jobs` scan-and-match) with a direct, indexed lookup — `path` is
    /// `UNIQUE` in `schema.sql`.
    pub fn file_id_for_path(&self, path: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM files WHERE path = ?1",
                params![path],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Full `files` row for a given `path`, or `None` if no row exists yet.
    /// Used by `rescan()` (T-2.5) to compare `size`/`mtime` before deciding
    /// whether a file needs hashing at all.
    pub fn file_by_path(&self, path: &str) -> Result<Option<FileRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, path, sha256, size, mtime, detected_at FROM files WHERE path = ?1",
                params![path],
                |row| {
                    Ok(FileRow {
                        id: row.get(0)?,
                        path: row.get(1)?,
                        sha256: row.get(2)?,
                        size: row.get(3)?,
                        mtime: row.get(4)?,
                        detected_at: row.get(5)?,
                    })
                },
            )
            .optional()?)
    }

    /// One-shot migration for `files.path` rows written by a build that predates
    /// `core::paths::canonicalize_clean` (bug: `\\?\`-prefixed paths — see
    /// `core::paths` doc comment for why that prefix is a problem). Strips the
    /// verbatim disk prefix (`\\?\C:\x` → `C:\x`) and the verbatim UNC prefix
    /// (`\\?\UNC\server\share\x` → `\\server\share\x`) directly in SQL, matching
    /// `paths::strip_verbatim`'s logic byte-for-byte.
    ///
    /// Idempotent: once a row is normalized neither `LIKE` pattern matches it
    /// again, so a second call (e.g. the next `Repo::open`) touches zero rows.
    /// Returns the number of rows updated.
    pub fn normalize_verbatim_paths(&self) -> Result<u32> {
        // UNC rows first: `path LIKE '\\?\%'` also matches `\\?\UNC\...`, so the
        // plain-disk update below excludes them explicitly to avoid double-editing.
        let unc_rows = self.conn.execute(
            r"UPDATE files SET path = '\\' || substr(path, 9) WHERE path LIKE '\\?\UNC\%'",
            [],
        )?;
        let disk_rows = self.conn.execute(
            r"UPDATE files SET path = substr(path, 5) WHERE path LIKE '\\?\%' AND path NOT LIKE '\\?\UNC\%'",
            [],
        )?;
        Ok((unc_rows + disk_rows) as u32)
    }
}

fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_SQL)?;

    let version: Option<i64> = conn
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
            row.get(0)
        })
        .optional()?;

    if version.is_none() {
        conn.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            params![CURRENT_SCHEMA_VERSION],
        )?;
    }

    Ok(())
}

fn map_job_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobRow> {
    Ok(JobRow {
        id: row.get(0)?,
        file_id: row.get(1)?,
        destination: row.get(2)?,
        status: row.get(3)?,
        attempts: row.get(4)?,
        next_attempt_at: row.get(5)?,
        remote_id: row.get(6)?,
        remote_state: row.get(7)?,
        last_error: row.get(8)?,
        archived_at: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

fn map_job_view(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobView> {
    let path: String = row.get(1)?;
    let name = Path::new(&path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());

    Ok(JobView {
        file_id: row.get(0)?,
        path,
        name,
        size: row.get(2)?,
        sha256: row.get(3)?,
        detected_at: row.get(4)?,
        s3: JobSide {
            job_id: row.get(5)?,
            status: row.get(6)?,
            attempts: row.get(7)?,
            next_attempt_at: row.get(8)?,
            remote_id: row.get(9)?,
            last_error: row.get(10)?,
            updated_at: row.get(11)?,
        },
        gdrive: JobSide {
            job_id: row.get(12)?,
            status: row.get(13)?,
            attempts: row.get(14)?,
            next_attempt_at: row.get(15)?,
            remote_id: row.get(16)?,
            last_error: row.get(17)?,
            updated_at: row.get(18)?,
        },
    })
}

/// Builds the shared `WHERE` clause + bound params for `list_jobs`, reused
/// for both the `COUNT(*)` and the paginated `SELECT`.
fn build_where(q: &ListJobsQuery) -> (String, Vec<Box<dyn ToSql>>) {
    let mut clauses: Vec<String> = Vec::new();
    let mut params_vec: Vec<Box<dyn ToSql>> = Vec::new();

    if !q.include_archived {
        clauses.push("js3.archived_at IS NULL AND jg.archived_at IS NULL".to_string());
    }

    if let Some(statuses) = q.statuses.as_ref().filter(|s| !s.is_empty()) {
        let placeholders = vec!["?"; statuses.len()].join(", ");
        match q.destination {
            Some(Destination::S3) => {
                clauses.push(format!("js3.status IN ({placeholders})"));
                for status in statuses {
                    params_vec.push(Box::new(*status));
                }
            }
            Some(Destination::GDrive) => {
                clauses.push(format!("jg.status IN ({placeholders})"));
                for status in statuses {
                    params_vec.push(Box::new(*status));
                }
            }
            None => {
                clauses.push(format!(
                    "(js3.status IN ({placeholders}) OR jg.status IN ({placeholders}))"
                ));
                for status in statuses {
                    params_vec.push(Box::new(*status));
                }
                for status in statuses {
                    params_vec.push(Box::new(*status));
                }
            }
        }
    }

    let where_sql = if clauses.is_empty() {
        "1 = 1".to_string()
    } else {
        clauses.join(" AND ")
    };

    (where_sql, params_vec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn new_repo() -> Repo {
        Repo::open_in_memory().expect("open_in_memory should succeed")
    }

    /// Enqueues `path` and returns its `file_id`. Every file gets the same
    /// two jobs (`s3` + `gdrive`, both `pending`) that `upsert_file_and_enqueue`
    /// creates in production.
    fn seed_file(repo: &Repo, path: &str, size: i64) -> String {
        repo.upsert_file_and_enqueue(path, "sha-1", size, "2026-01-01T00:00:00Z")
            .expect("seed enqueue");
        repo.file_id_for_path(path)
            .expect("file_id lookup")
            .expect("seeded file must exist")
    }

    fn set_job_status(repo: &Repo, file_id: &str, dest: &str, status: &str) {
        repo.conn
            .execute(
                "UPDATE jobs SET status = ?1 WHERE file_id = ?2 AND destination = ?3",
                params![status, file_id, dest],
            )
            .expect("force job status");
    }

    fn archived_count(repo: &Repo, file_id: &str) -> i64 {
        repo.conn
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE file_id = ?1 AND archived_at IS NOT NULL",
                params![file_id],
                |row| row.get(0),
            )
            .expect("count archived")
    }

    const NOW: &str = "2026-02-02T00:00:00Z";

    // The sweep must respect the same boundary the UI implies: things still in
    // the queue can be pulled back out, things in flight or already finished
    // cannot.
    #[test]
    fn reconcile_archives_only_pending_paused_and_failed() {
        let repo = new_repo();
        let sweepable = seed_file(&repo, "C:/x/small.txt", 10);
        set_job_status(&repo, &sweepable, "gdrive", "paused");

        let in_flight = seed_file(&repo, "C:/x/inflight.txt", 10);
        set_job_status(&repo, &in_flight, "s3", "uploading");
        set_job_status(&repo, &in_flight, "gdrive", "done");

        let (archived, restored) = repo
            .reconcile_jobs_with_filters(NOW, |_, _| false)
            .expect("reconcile");

        assert_eq!((archived, restored), (1, 0));
        assert_eq!(archived_count(&repo, &sweepable), 2);
        assert_eq!(
            archived_count(&repo, &in_flight),
            0,
            "an uploading/done pair must never be swept"
        );
    }

    #[test]
    fn reconcile_leaves_accepted_files_untouched() {
        let repo = new_repo();
        let file_id = seed_file(&repo, "C:/x/keep.pdf", 10);

        let (archived, restored) = repo
            .reconcile_jobs_with_filters(NOW, |_, _| true)
            .expect("reconcile");

        assert_eq!((archived, restored), (0, 0));
        assert_eq!(archived_count(&repo, &file_id), 0);
    }

    // Counted per file, like `clear_completed` — two jobs per file must not
    // read as two rows swept.
    #[test]
    fn reconcile_counts_files_not_jobs() {
        let repo = new_repo();
        seed_file(&repo, "C:/x/a.txt", 10);
        seed_file(&repo, "C:/x/b.txt", 20);

        let (archived, _) = repo
            .reconcile_jobs_with_filters(NOW, |_, _| false)
            .expect("reconcile");

        assert_eq!(archived, 2);
    }

    #[test]
    fn reconcile_is_idempotent() {
        let repo = new_repo();
        seed_file(&repo, "C:/x/a.txt", 10);

        let first = repo.reconcile_jobs_with_filters(NOW, |_, _| false).unwrap();
        let second = repo.reconcile_jobs_with_filters(NOW, |_, _| false).unwrap();

        assert_eq!(first, (1, 0));
        assert_eq!(second, (0, 0), "a second sweep must find nothing to do");
    }

    // The predicate sees the stored path and size, which is what lets the
    // caller run `passes_filters` without a syscall.
    #[test]
    fn reconcile_hands_the_predicate_the_stored_path_and_size() {
        let repo = new_repo();
        seed_file(&repo, "C:/x/report.pdf", 4096);

        let seen = std::cell::RefCell::new(Vec::new());
        repo.reconcile_jobs_with_filters(NOW, |path, size| {
            seen.borrow_mut().push((path.to_string(), size));
            true
        })
        .expect("reconcile");

        assert_eq!(
            seen.into_inner(),
            vec![("C:/x/report.pdf".to_string(), 4096)]
        );
    }

    // Without this half, widening a filter back would be a one-way door:
    // `rescan`'s size+mtime fast path returns `Unchanged` for a file that did
    // not move, so nothing else would ever un-archive it.
    #[test]
    fn reconcile_restores_jobs_when_the_file_passes_again() {
        let repo = new_repo();
        let file_id = seed_file(&repo, "C:/x/a.txt", 10);
        repo.reconcile_jobs_with_filters(NOW, |_, _| false).unwrap();
        assert_eq!(archived_count(&repo, &file_id), 2);

        let (archived, restored) = repo
            .reconcile_jobs_with_filters(NOW, |_, _| true)
            .expect("reconcile");

        assert_eq!((archived, restored), (0, 1));
        assert_eq!(archived_count(&repo, &file_id), 0);
    }

    // A file half-uploaded when the filter tightened: the finished side stays
    // as history, only the still-queued side is swept.
    #[test]
    fn reconcile_sweeps_only_the_queued_side_of_a_mixed_pair() {
        let repo = new_repo();
        let file_id = seed_file(&repo, "C:/x/half.txt", 10);
        set_job_status(&repo, &file_id, "s3", "done");

        let (archived, _) = repo
            .reconcile_jobs_with_filters(NOW, |_, _| false)
            .expect("reconcile");

        assert_eq!(archived, 1);
        assert_eq!(archived_count(&repo, &file_id), 1);
        let done_archived: i64 = repo
            .conn
            .query_row(
                "SELECT COUNT(*) FROM jobs \
                 WHERE file_id = ?1 AND status = 'done' AND archived_at IS NOT NULL",
                params![file_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(done_archived, 0, "the completed side is history, not queue");
    }

    // Archiving would be cosmetic if the worker still picked the job up — the
    // file would vanish from the table and land in the bucket anyway.
    #[test]
    fn claim_next_skips_archived_jobs() {
        let repo = new_repo();
        seed_file(&repo, "C:/x/a.txt", 10);
        repo.reconcile_jobs_with_filters(NOW, |_, _| false).unwrap();

        let claimed = repo
            .claim_next(Destination::S3, NOW)
            .expect("claim_next must not error");

        assert!(claimed.is_none(), "an archived job must not be claimable");
    }

    // "Reenviar Falhas" must not resurrect rows the user never sees.
    #[test]
    fn retry_all_failed_skips_archived_jobs() {
        let repo = new_repo();
        let file_id = seed_file(&repo, "C:/x/a.txt", 10);
        set_job_status(&repo, &file_id, "s3", "failed");
        set_job_status(&repo, &file_id, "gdrive", "failed");
        repo.reconcile_jobs_with_filters(NOW, |_, _| false).unwrap();

        let retried = repo.retry_all_failed().expect("retry_all_failed");

        assert_eq!(retried, 0);
    }

    // The KPI row and the "N arquivos" badge read from here. Without this the
    // table would shrink after a sweep while every counter stayed put — which
    // is most of what "I could not see the change" meant.
    #[test]
    fn status_counts_excludes_archived_jobs() {
        let repo = new_repo();
        seed_file(&repo, "C:/x/a.txt", 4096);
        assert_eq!(repo.status_counts().unwrap().pending, 2);

        repo.reconcile_jobs_with_filters(NOW, |_, _| false).unwrap();

        let counts = repo.status_counts().expect("status_counts");
        assert_eq!(counts.pending, 0);
        assert_eq!(counts.bytes_total, 0, "archived bytes leave the KPI too");
    }

    #[test]
    fn migrations_are_idempotent_on_a_real_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("state.db");

        {
            let _repo = Repo::open(&db_path).expect("first open should create schema");
        }
        let repo = Repo::open(&db_path).expect("second open should be a no-op migration");

        let version: i64 = repo
            .conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))
            .expect("schema_version should have exactly one row");
        assert_eq!(version, 1);
    }

    // --- normalize_verbatim_paths ---------------------------------------

    /// Inserts a bare `files` row (no `jobs`, unlike `upsert_file_and_enqueue`) so
    /// these tests only exercise the `path` column being normalized.
    fn insert_raw_file(repo: &Repo, id: &str, path: &str) {
        repo.conn
            .execute(
                "INSERT INTO files (id, path, sha256, size, mtime, detected_at) \
                 VALUES (?1, ?2, 'deadbeef', 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                params![id, path],
            )
            .expect("insert raw file row");
    }

    #[test]
    fn normalize_verbatim_paths_strips_the_disk_prefix() {
        let repo = new_repo();
        insert_raw_file(&repo, "f1", r"\\?\C:\monitoramento\file.bin");

        let updated = repo
            .normalize_verbatim_paths()
            .expect("normalize_verbatim_paths should succeed");

        assert_eq!(updated, 1);
        let row = repo
            .file_by_path(r"C:\monitoramento\file.bin")
            .expect("file_by_path should succeed")
            .expect("row should now exist under the stripped path");
        assert_eq!(row.id, "f1");
    }

    #[test]
    fn normalize_verbatim_paths_strips_the_unc_prefix_and_restores_leading_backslashes() {
        let repo = new_repo();
        insert_raw_file(&repo, "f1", r"\\?\UNC\server\share\file.bin");

        let updated = repo
            .normalize_verbatim_paths()
            .expect("normalize_verbatim_paths should succeed");

        assert_eq!(updated, 1);
        let row = repo
            .file_by_path(r"\\server\share\file.bin")
            .expect("file_by_path should succeed")
            .expect("row should now exist under the stripped UNC path");
        assert_eq!(row.id, "f1");
    }

    #[test]
    fn normalize_verbatim_paths_leaves_already_clean_rows_untouched() {
        let repo = new_repo();
        insert_raw_file(&repo, "f1", "/private/tmp/monitoramento/file.bin");
        insert_raw_file(&repo, "f2", r"C:\monitoramento\file.bin");

        let updated = repo
            .normalize_verbatim_paths()
            .expect("normalize_verbatim_paths should succeed");

        assert_eq!(updated, 0, "no verbatim-prefixed rows to touch");
    }

    #[test]
    fn normalize_verbatim_paths_is_idempotent() {
        let repo = new_repo();
        insert_raw_file(&repo, "f1", r"\\?\C:\a\b.bin");
        insert_raw_file(&repo, "f2", r"\\?\UNC\server\share\c.bin");

        let first = repo
            .normalize_verbatim_paths()
            .expect("first normalize call should succeed");
        assert_eq!(first, 2);

        let second = repo
            .normalize_verbatim_paths()
            .expect("second normalize call should succeed");
        assert_eq!(second, 0, "already-normalized rows must not match again");
    }

    #[test]
    fn open_runs_normalize_verbatim_paths_automatically() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("state.db");

        {
            let repo = Repo::open(&db_path).expect("open should create schema");
            insert_raw_file(&repo, "f1", r"\\?\C:\legacy\file.bin");
        }

        // Reopening must run the one-shot cleanup again and normalize the row
        // written by a (simulated) pre-`canonicalize_clean` build.
        let repo = Repo::open(&db_path).expect("reopen should succeed");
        let row = repo
            .file_by_path(r"C:\legacy\file.bin")
            .expect("file_by_path should succeed")
            .expect("row should have been normalized on open");
        assert_eq!(row.id, "f1");
    }

    #[test]
    fn upsert_creates_unchanged_and_rehashed() {
        let repo = new_repo();

        let created = repo
            .upsert_file_and_enqueue("C:/x/a.txt", "sha-1", 100, "2026-01-01T00:00:00Z")
            .unwrap();
        assert_eq!(created, UpsertOutcome::Created);

        let jobs_after_create: i64 = repo
            .conn
            .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(jobs_after_create, 2, "one pending job per destination");

        let unchanged = repo
            .upsert_file_and_enqueue("C:/x/a.txt", "sha-1", 100, "2026-01-01T00:00:01Z")
            .unwrap();
        assert_eq!(unchanged, UpsertOutcome::Unchanged);

        let files_count: i64 = repo
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
            .unwrap();
        assert_eq!(files_count, 1, "same path never duplicates the files row");

        // Move one job forward so we can prove `Rehashed` resets it.
        let job_id: String = repo
            .conn
            .query_row("SELECT id FROM jobs WHERE destination = 's3'", [], |row| {
                row.get(0)
            })
            .unwrap();
        repo.mark_done(&job_id, "remote-1", Some("{}")).unwrap();

        let rehashed = repo
            .upsert_file_and_enqueue("C:/x/a.txt", "sha-2", 200, "2026-01-02T00:00:00Z")
            .unwrap();
        assert_eq!(rehashed, UpsertOutcome::Rehashed);

        let jobs_after_rehash: i64 = repo
            .conn
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE file_id = (SELECT id FROM files WHERE path = 'C:/x/a.txt')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(jobs_after_rehash, 2, "never a second pair of jobs");

        let status: String = repo
            .conn
            .query_row(
                "SELECT status FROM jobs WHERE id = ?1",
                params![job_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "pending", "rehash resets even an already-done job");
    }

    #[test]
    fn claim_next_respects_next_attempt_at() {
        let repo = new_repo();
        repo.upsert_file_and_enqueue("C:/x/a.txt", "sha-1", 100, "2026-01-01T00:00:00Z")
            .unwrap();

        let job_id: String = repo
            .conn
            .query_row("SELECT id FROM jobs WHERE destination = 's3'", [], |row| {
                row.get(0)
            })
            .unwrap();
        repo.mark_retry(&job_id, 1, "2026-01-01T01:00:00Z", "boom")
            .unwrap();

        let too_early = repo
            .claim_next(Destination::S3, "2026-01-01T00:30:00Z")
            .unwrap();
        assert!(too_early.is_none(), "job is not due yet");

        let due = repo
            .claim_next(Destination::S3, "2026-01-01T01:00:00Z")
            .unwrap();
        let due = due.expect("job is due now");
        assert_eq!(due.id, job_id);
        assert_eq!(due.status, JobStatus::Uploading);
    }

    #[test]
    fn claim_next_is_not_double_claimable() {
        let repo = new_repo();
        repo.upsert_file_and_enqueue("C:/x/a.txt", "sha-1", 100, "2026-01-01T00:00:00Z")
            .unwrap();
        repo.upsert_file_and_enqueue("C:/x/b.txt", "sha-2", 100, "2026-01-01T00:00:00Z")
            .unwrap();

        let now = "2026-01-01T00:00:00Z";
        let first = repo.claim_next(Destination::S3, now).unwrap().unwrap();
        let second = repo.claim_next(Destination::S3, now).unwrap().unwrap();
        assert_ne!(first.id, second.id, "second claim gets the other job");

        let third = repo.claim_next(Destination::S3, now).unwrap();
        assert!(third.is_none(), "no pending s3 jobs left");
    }

    #[test]
    fn retry_failed_and_cancel_transitions() {
        let repo = new_repo();
        repo.upsert_file_and_enqueue("C:/x/a.txt", "sha-1", 100, "2026-01-01T00:00:00Z")
            .unwrap();
        let job_id: String = repo
            .conn
            .query_row("SELECT id FROM jobs WHERE destination = 's3'", [], |row| {
                row.get(0)
            })
            .unwrap();

        repo.mark_failed(&job_id, "boom").unwrap();
        let status: JobStatus = repo
            .conn
            .query_row(
                "SELECT status FROM jobs WHERE id = ?1",
                params![job_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, JobStatus::Failed);

        repo.retry_job(&job_id).unwrap();
        let (status, attempts, last_error): (JobStatus, i64, Option<String>) = repo
            .conn
            .query_row(
                "SELECT status, attempts, last_error FROM jobs WHERE id = ?1",
                params![job_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(status, JobStatus::Pending);
        assert_eq!(attempts, 0);
        assert!(last_error.is_none());

        repo.cancel_job(&job_id).unwrap();
        let status: JobStatus = repo
            .conn
            .query_row(
                "SELECT status FROM jobs WHERE id = ?1",
                params![job_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, JobStatus::Cancelled);
    }

    #[test]
    fn retry_all_failed_resets_every_failed_job() {
        let repo = new_repo();
        repo.upsert_file_and_enqueue("C:/x/a.txt", "sha-1", 100, "2026-01-01T00:00:00Z")
            .unwrap();
        repo.upsert_file_and_enqueue("C:/x/b.txt", "sha-2", 100, "2026-01-01T00:00:00Z")
            .unwrap();

        let job_ids: Vec<String> = {
            let mut stmt = repo
                .conn
                .prepare("SELECT id FROM jobs WHERE destination = 's3'")
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        for id in &job_ids {
            repo.mark_failed(id, "boom").unwrap();
        }

        let affected = repo.retry_all_failed().unwrap();
        assert_eq!(affected, 2);

        let pending: i64 = repo
            .conn
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE status = 'pending'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pending, 4, "the 2 s3 jobs plus the 2 untouched gdrive jobs");
    }

    #[test]
    fn clear_completed_only_when_both_jobs_are_done() {
        let repo = new_repo();
        repo.upsert_file_and_enqueue("C:/x/both-done.txt", "sha-1", 100, "2026-01-01T00:00:00Z")
            .unwrap();
        repo.upsert_file_and_enqueue("C:/x/one-done.txt", "sha-2", 100, "2026-01-01T00:00:00Z")
            .unwrap();

        for path in ["C:/x/both-done.txt", "C:/x/one-done.txt"] {
            let job_id: String = repo
                .conn
                .query_row(
                    "SELECT jobs.id FROM jobs JOIN files ON files.id = jobs.file_id \
                     WHERE files.path = ?1 AND jobs.destination = 's3'",
                    params![path],
                    |row| row.get(0),
                )
                .unwrap();
            repo.mark_done(&job_id, "remote", None).unwrap();
        }
        let gdrive_job_id: String = repo
            .conn
            .query_row(
                "SELECT jobs.id FROM jobs JOIN files ON files.id = jobs.file_id \
                 WHERE files.path = 'C:/x/both-done.txt' AND jobs.destination = 'gdrive'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        repo.mark_done(&gdrive_job_id, "remote", None).unwrap();

        let archived = repo.clear_completed("2026-02-01T00:00:00Z").unwrap();
        assert_eq!(archived, 1, "only the file with both jobs done is archived");

        let visible: i64 = repo
            .conn
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE archived_at IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(visible, 2, "the 2 jobs of one-done.txt stay visible");
    }

    #[test]
    fn recover_on_boot_resets_uploading_and_returns_remote_state() {
        let repo = new_repo();
        repo.upsert_file_and_enqueue("C:/x/a.txt", "sha-1", 100, "2026-01-01T00:00:00Z")
            .unwrap();
        let job_id: String = repo
            .conn
            .query_row("SELECT id FROM jobs WHERE destination = 's3'", [], |row| {
                row.get(0)
            })
            .unwrap();
        repo.conn
            .execute(
                "UPDATE jobs SET status = 'uploading', remote_state = ?1 WHERE id = ?2",
                params![r#"{"upload_id":"abc"}"#, job_id],
            )
            .unwrap();

        let recovered = repo.recover_on_boot().unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].job_id, job_id);
        assert_eq!(recovered[0].destination, Destination::S3);
        assert_eq!(
            recovered[0].remote_state.as_deref(),
            Some(r#"{"upload_id":"abc"}"#)
        );

        let status: JobStatus = repo
            .conn
            .query_row(
                "SELECT status FROM jobs WHERE id = ?1",
                params![job_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, JobStatus::Pending);
    }

    #[test]
    fn list_jobs_paginates_filters_by_status_and_reports_total() {
        let repo = new_repo();
        for i in 0..5 {
            repo.upsert_file_and_enqueue(
                &format!("C:/x/f{i}.txt"),
                &format!("sha-{i}"),
                100,
                "2026-01-01T00:00:00Z",
            )
            .unwrap();
        }

        // Fail the s3 side of file 0 only.
        let job_id: String = repo
            .conn
            .query_row(
                "SELECT jobs.id FROM jobs JOIN files ON files.id = jobs.file_id \
                 WHERE files.path = 'C:/x/f0.txt' AND jobs.destination = 's3'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        repo.mark_failed(&job_id, "boom").unwrap();

        let page = repo
            .list_jobs(&ListJobsQuery {
                statuses: None,
                destination: None,
                include_archived: false,
                limit: 2,
                offset: 0,
            })
            .unwrap();
        assert_eq!(page.total, 5);
        assert_eq!(page.items.len(), 2);

        let page2 = repo
            .list_jobs(&ListJobsQuery {
                statuses: None,
                destination: None,
                include_archived: false,
                limit: 2,
                offset: 4,
            })
            .unwrap();
        assert_eq!(page2.items.len(), 1, "last page has the remainder");

        let failed_only = repo
            .list_jobs(&ListJobsQuery {
                statuses: Some(vec![JobStatus::Failed]),
                destination: None,
                include_archived: false,
                limit: 10,
                offset: 0,
            })
            .unwrap();
        assert_eq!(failed_only.total, 1);
        assert_eq!(failed_only.items[0].path, "C:/x/f0.txt");
        assert_eq!(failed_only.items[0].s3.status, JobStatus::Failed);
        assert_eq!(failed_only.items[0].gdrive.status, JobStatus::Pending);

        let failed_gdrive_only = repo
            .list_jobs(&ListJobsQuery {
                statuses: Some(vec![JobStatus::Failed]),
                destination: Some(Destination::GDrive),
                include_archived: false,
                limit: 10,
                offset: 0,
            })
            .unwrap();
        assert_eq!(
            failed_gdrive_only.total, 0,
            "destination narrows the status filter to that side"
        );
    }

    #[test]
    fn list_jobs_include_archived_toggles_visibility() {
        let repo = new_repo();
        repo.upsert_file_and_enqueue("C:/x/a.txt", "sha-1", 100, "2026-01-01T00:00:00Z")
            .unwrap();
        for dest in ["s3", "gdrive"] {
            let job_id: String = repo
                .conn
                .query_row(
                    "SELECT id FROM jobs WHERE destination = ?1",
                    params![dest],
                    |row| row.get(0),
                )
                .unwrap();
            repo.mark_done(&job_id, "remote", None).unwrap();
        }
        repo.clear_completed("2026-02-01T00:00:00Z").unwrap();

        let default_page = repo
            .list_jobs(&ListJobsQuery {
                statuses: None,
                destination: None,
                include_archived: false,
                limit: 10,
                offset: 0,
            })
            .unwrap();
        assert_eq!(default_page.total, 0);

        let with_archived = repo
            .list_jobs(&ListJobsQuery {
                statuses: None,
                destination: None,
                include_archived: true,
                limit: 10,
                offset: 0,
            })
            .unwrap();
        assert_eq!(with_archived.total, 1);
    }

    #[test]
    fn status_counts_reports_per_status_and_bytes() {
        let repo = new_repo();
        repo.upsert_file_and_enqueue("C:/x/a.txt", "sha-1", 100, "2026-01-01T00:00:00Z")
            .unwrap();
        repo.upsert_file_and_enqueue("C:/x/b.txt", "sha-2", 300, "2026-01-01T00:00:00Z")
            .unwrap();

        for dest in ["s3", "gdrive"] {
            let job_id: String = repo
                .conn
                .query_row(
                    "SELECT jobs.id FROM jobs JOIN files ON files.id = jobs.file_id \
                     WHERE files.path = 'C:/x/a.txt' AND jobs.destination = ?1",
                    params![dest],
                    |row| row.get(0),
                )
                .unwrap();
            repo.mark_done(&job_id, "remote", None).unwrap();
        }

        let counts = repo.status_counts().unwrap();
        assert_eq!(counts.done, 2, "both jobs of a.txt");
        assert_eq!(counts.pending, 2, "both jobs of b.txt");
        assert_eq!(counts.bytes_total, 400);
        assert_eq!(counts.bytes_done, 100, "only a.txt has both jobs done");
    }

    #[test]
    fn insert_and_read_recent_events() {
        let repo = new_repo();
        repo.insert_event("info", None, "watcher started").unwrap();
        repo.insert_event("warn", Some("job-1"), "retrying")
            .unwrap();

        let events = repo.recent_events(10).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].message, "retrying", "most recent first");
        assert_eq!(events[0].job_id.as_deref(), Some("job-1"));
    }

    #[test]
    #[ignore = "run locally with `-- --ignored` to check the RNF-008 200ms budget"]
    fn list_jobs_perf_10k_rows_under_200ms() {
        let repo = new_repo();
        for i in 0..10_000 {
            repo.upsert_file_and_enqueue(
                &format!("C:/x/f{i}.txt"),
                &format!("sha-{i}"),
                100,
                "2026-01-01T00:00:00Z",
            )
            .unwrap();
        }

        let start = Instant::now();
        let page = repo
            .list_jobs(&ListJobsQuery {
                statuses: None,
                destination: None,
                include_archived: false,
                limit: 200,
                offset: 0,
            })
            .unwrap();
        let elapsed = start.elapsed();

        println!("list_jobs_perf: 10000 rows, page of 200, took {elapsed:?}");
        assert_eq!(page.total, 10_000);
        assert!(
            elapsed < Duration::from_millis(200),
            "list_jobs took {elapsed:?}, budget is 200ms (RNF-008)"
        );
    }

    #[test]
    fn job_view_for_file_returns_the_pair_and_none_when_missing() {
        let repo = new_repo();
        repo.upsert_file_and_enqueue("C:/x/a.txt", "sha-1", 100, "2026-01-01T00:00:00Z")
            .unwrap();
        let file_id: String = repo
            .conn
            .query_row(
                "SELECT id FROM files WHERE path = 'C:/x/a.txt'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        let view = repo.job_view_for_file(&file_id).unwrap();
        assert!(
            view.is_some(),
            "freshly-enqueued file should have a job pair"
        );
        let view = view.unwrap();
        assert_eq!(view.path, "C:/x/a.txt");
        assert_eq!(view.file_id, file_id);

        let missing = repo.job_view_for_file("does-not-exist").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn job_view_for_job_resolves_the_owning_file_and_none_when_missing() {
        let repo = new_repo();
        repo.upsert_file_and_enqueue("C:/x/b.txt", "sha-2", 200, "2026-01-01T00:00:00Z")
            .unwrap();
        let (file_id, s3_job_id): (String, String) = repo
            .conn
            .query_row(
                "SELECT f.id, j.id FROM files f JOIN jobs j ON j.file_id = f.id \
                 WHERE f.path = 'C:/x/b.txt' AND j.destination = 's3'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        let view = repo
            .job_view_for_job(&s3_job_id)
            .unwrap()
            .expect("job_id should resolve to its file's JobView");
        assert_eq!(view.file_id, file_id);
        assert_eq!(view.path, "C:/x/b.txt");

        let missing = repo.job_view_for_job("does-not-exist").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn uploading_job_id_for_name_matches_by_basename_and_ignores_other_statuses() {
        let repo = new_repo();
        repo.upsert_file_and_enqueue("C:/x/c.txt", "sha-3", 300, "2026-01-01T00:00:00Z")
            .unwrap();
        let s3_job_id: String = repo
            .conn
            .query_row("SELECT id FROM jobs WHERE destination = 's3'", [], |row| {
                row.get(0)
            })
            .unwrap();

        // Not 'uploading' yet: no match even though the basename is right.
        assert_eq!(
            repo.uploading_job_id_for_name(Destination::S3, "c.txt")
                .unwrap(),
            None
        );

        repo.conn
            .execute(
                "UPDATE jobs SET status = 'uploading' WHERE id = ?1",
                params![s3_job_id],
            )
            .unwrap();

        assert_eq!(
            repo.uploading_job_id_for_name(Destination::S3, "c.txt")
                .unwrap(),
            Some(s3_job_id.clone())
        );

        // Wrong destination: gdrive's row for the same file is still 'pending'.
        assert_eq!(
            repo.uploading_job_id_for_name(Destination::GDrive, "c.txt")
                .unwrap(),
            None
        );

        // Wrong name: no uploading job for this dest has this basename.
        assert_eq!(
            repo.uploading_job_id_for_name(Destination::S3, "does-not-exist.txt")
                .unwrap(),
            None
        );
    }

    #[test]
    fn job_by_id_returns_the_full_row_and_none_when_missing() {
        let repo = new_repo();
        repo.upsert_file_and_enqueue("C:/x/d.txt", "sha-4", 400, "2026-01-01T00:00:00Z")
            .unwrap();
        let s3_job_id: String = repo
            .conn
            .query_row("SELECT id FROM jobs WHERE destination = 's3'", [], |row| {
                row.get(0)
            })
            .unwrap();

        repo.set_remote_state(&s3_job_id, r#"{"key":"prefix/d.txt"}"#)
            .unwrap();

        let row = repo
            .job_by_id(&s3_job_id)
            .unwrap()
            .expect("job row should exist");
        assert_eq!(row.id, s3_job_id);
        assert_eq!(row.destination, Destination::S3);
        assert_eq!(row.status, JobStatus::Pending);
        assert_eq!(
            row.remote_state.as_deref(),
            Some(r#"{"key":"prefix/d.txt"}"#)
        );

        assert!(repo.job_by_id("does-not-exist").unwrap().is_none());
    }
}
