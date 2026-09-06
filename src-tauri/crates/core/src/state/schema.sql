-- osystems-sync `state.db` schema (SPEC.md §5, PLAN.md T-1.2).
--
-- PRAGMAs (journal_mode, foreign_keys, busy_timeout, synchronous) are set in
-- code (`repo.rs::configure_connection`) — they are connection-scoped, not
-- part of the persisted schema, and an in-memory database (used in tests)
-- cannot use WAL at all. Keep this file to pure DDL so it stays idempotent
-- and portable between `Connection::open` and `Connection::open_in_memory`.

-- Tracks the schema version applied to this database file so `repo::open`
-- can run migrations idempotently (open twice == open once).
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS files (
    id          TEXT PRIMARY KEY,          -- uuid
    path        TEXT NOT NULL UNIQUE,
    sha256      TEXT NOT NULL,
    size        INTEGER NOT NULL,
    mtime       TEXT NOT NULL,             -- ISO8601 UTC, mtime of the file at last (re)hash
    detected_at TEXT NOT NULL              -- ISO8601 UTC
);

CREATE TABLE IF NOT EXISTS jobs (
    id              TEXT PRIMARY KEY,
    file_id         TEXT NOT NULL REFERENCES files(id),
    destination     TEXT NOT NULL CHECK (destination IN ('s3', 'gdrive')),
    status          TEXT NOT NULL CHECK (status IN ('pending', 'uploading', 'paused', 'cancelled', 'done', 'failed')),
    attempts        INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TEXT,
    remote_id       TEXT,                  -- S3 key or Drive fileId
    remote_state    TEXT,                  -- JSON: S3 {upload_id, parts[]} | Drive {session_uri, offset, web_view_link}
    last_error      TEXT,
    archived_at     TEXT,                  -- "Clear completed" (RF-036); NULL = visible
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    UNIQUE (file_id, destination)
);

CREATE INDEX IF NOT EXISTS idx_jobs_status ON jobs (status, next_attempt_at);
CREATE INDEX IF NOT EXISTS idx_jobs_visible ON jobs (archived_at, created_at DESC);

CREATE TABLE IF NOT EXISTS events (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    ts      TEXT NOT NULL,
    level   TEXT NOT NULL,
    job_id  TEXT,
    message TEXT NOT NULL
);
