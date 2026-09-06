//! `core::logging` — structured logging for `osystems-sync-core` (T-1.3).
//!
//! Wires up `tracing` with:
//! - a size-rotating JSON-lines file at `<logs_dir>/app.log` (10 MiB × 5 backups, RF-099 /
//!   RNF-005), and
//! - an in-memory ring buffer (last 500 lines, RF-066) plus a [`tokio::sync::broadcast`]
//!   channel that `src-tauri/src/events.rs` subscribes to in order to emit the `log-line`
//!   event described in `SPEC.md` §7.
//!
//! Both destinations are fed by a single custom [`tracing_subscriber::Layer`]
//! (see [`AppLoggingLayer`]) so that every event is parsed into a [`LogLine`] exactly
//! once and the same redaction is guaranteed everywhere the line ends up — see the
//! **Redaction (RNF-015)** section below.
//!
//! # Redaction (RNF-015)
//!
//! `RNF-015` forbids ever logging a secret, a Service Account JSON path, or a token.
//! [`redact`] scans every event's `message` field for the patterns below before it
//! reaches the ring, the broadcast channel, or the log file, and replaces the offending
//! part with `[REDACTED]`:
//!
//! - AWS access key ids: `AKIA[0-9A-Z]{16}`
//! - AWS STS temporary access key ids: `ASIA[0-9A-Z]{16}`
//! - Service Account private keys embedded in JSON: `"private_key": "..."`
//! - OAuth `client_secret` values embedded in JSON: `"client_secret": "..."`
//! - AWS SigV4 credential scopes: `AWS4-HMAC-SHA256 Credential=...`
//! - AWS SigV4 signatures: `Signature=[0-9a-f]{64}`
//! - Google OAuth2 access tokens: `ya29.<token>`
//! - Bearer HTTP authorization headers: `Bearer <token>`
//! - Any other 40-character base64-alphabet run immediately adjacent to the word
//!   `secret`/`Secret` (catches ad-hoc secret values, e.g. AWS secret access keys, that
//!   don't match a more specific pattern above)
//!
//! Every fixed-length pattern above also checks its right-hand boundary: a run is only
//! masked when the character right after it (if any) does *not* also belong to the run's
//! alphabet — otherwise what looks like, say, an access key id is actually a longer
//! token this pattern doesn't understand, and is left untouched rather than
//! partially masked.
//!
//! There is no `regex` crate in this workspace (see `SPEC.md` §4), so `redact` is
//! implemented with plain `str`/byte scanning instead of a regex engine.
//!
//! # `RUST_LOG` (VULN-006)
//!
//! [`build_filter`] ensures a release build never honors the `RUST_LOG` environment
//! variable — only a debug build does. Production log verbosity is controlled solely by
//! the app's own `level` config, never by an environment variable an attacker or a
//! misconfigured launcher could set (e.g. to `trace`, to make the app log — and
//! therefore [`redact`] have to scrub — far more than intended).

use std::collections::VecDeque;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, Layer};
use ts_rs::TS;

/// Number of lines kept in the in-memory ring buffer (RF-066).
const RING_CAPACITY: usize = 500;
/// Capacity of the broadcast channel backing `subscribe()`. Independent from
/// [`RING_CAPACITY`]: a slow/absent receiver only loses live updates, it never affects
/// the ring or the file.
const BROADCAST_CAPACITY: usize = 1024;
/// Default file rotation threshold: 10 MiB (RF-099 / RNF-005).
const DEFAULT_MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
/// Number of rotated backups kept alongside the active file (`app.log.1` .. `app.log.5`).
const MAX_BACKUPS: u32 = 5;

/// One structured log line, mirrored to the ring buffer, the `log-line` event
/// (`src-tauri/src/events.rs`) and the JSON log file.
///
/// Matches the payload documented in `SPEC.md` §7: `{ ts, level, target, job_id?,
/// destination?, message }`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, TS)]
#[ts(export)]
pub struct LogLine {
    /// RFC3339 timestamp in UTC, millisecond precision.
    pub ts: String,
    /// Level name (`TRACE`, `DEBUG`, `INFO`, `WARN`, `ERROR`).
    pub level: String,
    /// Event target (module path / span target).
    pub target: String,
    /// Present when the event carried a `job_id` field.
    pub job_id: Option<String>,
    /// Present when the event carried a `destination` field.
    pub destination: Option<String>,
    /// Human-readable message, already redacted (see module docs).
    pub message: String,
}

/// Errors that can occur while setting up logging.
#[derive(Debug, thiserror::Error)]
pub enum LoggingError {
    /// A `tracing` global default subscriber is already installed for this process.
    /// `init` must only be called once — subsequent calls always return this variant.
    #[error("logging is already initialized")]
    AlreadyInitialized,
    /// Creating the logs directory or opening the log file failed.
    #[error("failed to prepare the log file: {0}")]
    Io(#[from] io::Error),
}

/// Handle returned by [`init`] / [`init_for_tests`]. Keeps the broadcast sender, the ring
/// buffer, and the non-blocking file writer's worker guard alive for as long as logging
/// should keep working — dropping it stops the background flush thread.
pub struct LoggingHandle {
    /// Sender side of the broadcast channel; `src-tauri/src/events.rs` calls
    /// [`LoggingHandle::subscribe`] to get a receiver and re-emit `log-line` events.
    pub tx: broadcast::Sender<LogLine>,
    ring: Arc<Mutex<VecDeque<LogLine>>>,
    _guard: tracing_appender::non_blocking::WorkerGuard,
}

impl LoggingHandle {
    /// Subscribes to live log lines as they are produced.
    pub fn subscribe(&self) -> broadcast::Receiver<LogLine> {
        self.tx.subscribe()
    }

    /// Returns up to `limit` of the most recent lines, oldest first.
    pub fn recent(&self, limit: usize) -> Vec<LogLine> {
        let ring = match self.ring.lock() {
            Ok(ring) => ring,
            Err(poisoned) => poisoned.into_inner(),
        };
        let skip = ring.len().saturating_sub(limit);
        ring.iter().skip(skip).cloned().collect()
    }
}

/// Initializes global logging: a size-rotating JSON file under `logs_dir` plus the
/// ring/broadcast destinations. Installs a `tracing` global default subscriber, so it
/// must be called exactly once per process — a second call returns
/// [`LoggingError::AlreadyInitialized`].
///
/// `level` is the default [`EnvFilter`] directive (e.g. `"info"`); the `RUST_LOG`
/// environment variable overrides it when present.
pub fn init(logs_dir: &Path, level: &str) -> Result<LoggingHandle, LoggingError> {
    fs::create_dir_all(logs_dir)?;
    let log_path = logs_dir.join("app.log");
    let writer = SizeRotatingWriter::new(log_path, DEFAULT_MAX_FILE_BYTES)?;
    let (non_blocking, guard) = tracing_appender::non_blocking(writer);

    let ring = Arc::new(Mutex::new(VecDeque::with_capacity(RING_CAPACITY)));
    let (tx, _rx) = broadcast::channel(BROADCAST_CAPACITY);

    let layer = AppLoggingLayer {
        writer: non_blocking,
        ring: ring.clone(),
        tx: tx.clone(),
    };
    let filter = build_filter(
        level,
        std::env::var("RUST_LOG").ok(),
        cfg!(debug_assertions),
    );
    let subscriber = tracing_subscriber::registry().with(filter).with(layer);

    tracing::subscriber::set_global_default(subscriber)
        .map_err(|_| LoggingError::AlreadyInitialized)?;

    Ok(LoggingHandle {
        tx,
        ring,
        _guard: guard,
    })
}

/// Builds the [`EnvFilter`] used by [`init`] (VULN-006): `RUST_LOG` may only ever override
/// the configured `level` in a debug build. A release build ignores `env_override`
/// entirely — production log verbosity is controlled solely by the app's own `level`
/// config, never by an environment variable that an attacker or a misconfigured launcher
/// could set to `trace` to exfiltrate more log detail. A malformed `env_override` (one
/// `EnvFilter::try_new` rejects) also falls back to `level`, in debug or release alike.
fn build_filter(level: &str, env_override: Option<String>, debug: bool) -> EnvFilter {
    if debug {
        if let Some(raw) = env_override {
            if let Ok(filter) = EnvFilter::try_new(&raw) {
                return filter;
            }
        }
    }
    EnvFilter::new(level)
}

/// Test-only setup: ring + broadcast only, nothing written to disk. Installs the
/// subscriber as the calling thread's default (via [`tracing::subscriber::set_default`])
/// rather than the process-wide global default, so it is safe to call from many tests
/// without conflicting with each other or with a real [`init`] elsewhere in the binary.
/// The guard is intentionally leaked (`std::mem::forget`) so the override stays active
/// for the rest of the calling test thread without the caller having to hold it.
pub fn init_for_tests() -> LoggingHandle {
    let (non_blocking, guard) = tracing_appender::non_blocking(io::sink());

    let ring = Arc::new(Mutex::new(VecDeque::with_capacity(RING_CAPACITY)));
    let (tx, _rx) = broadcast::channel(BROADCAST_CAPACITY);

    let layer = AppLoggingLayer {
        writer: non_blocking,
        ring: ring.clone(),
        tx: tx.clone(),
    };
    let subscriber = tracing_subscriber::registry()
        .with(EnvFilter::new("trace"))
        .with(layer);

    let default_guard = tracing::subscriber::set_default(subscriber);
    std::mem::forget(default_guard);

    LoggingHandle {
        tx,
        ring,
        _guard: guard,
    }
}

/// The single `tracing_subscriber::Layer` that turns every event into a [`LogLine`] and
/// fans it out to the ring buffer, the broadcast channel, and the (already
/// non-blocking, already size-rotating) file writer. Keeping this in one place — rather
/// than one layer per destination — guarantees the redaction in [`redact`] is applied
/// exactly once and identically everywhere the line goes.
struct AppLoggingLayer {
    writer: tracing_appender::non_blocking::NonBlocking,
    ring: Arc<Mutex<VecDeque<LogLine>>>,
    tx: broadcast::Sender<LogLine>,
}

impl<S> Layer<S> for AppLoggingLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = LogVisitor::default();
        event.record(&mut visitor);
        let metadata = event.metadata();

        let line = LogLine {
            ts: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            level: metadata.level().to_string(),
            target: metadata.target().to_string(),
            job_id: visitor.job_id,
            destination: visitor.destination,
            message: redact(&visitor.message),
        };

        match self.ring.lock() {
            Ok(mut ring) => {
                if ring.len() >= RING_CAPACITY {
                    ring.pop_front();
                }
                ring.push_back(line.clone());
            }
            Err(poisoned) => {
                let mut ring = poisoned.into_inner();
                if ring.len() >= RING_CAPACITY {
                    ring.pop_front();
                }
                ring.push_back(line.clone());
            }
        }

        // No subscribers currently listening is not an error — the UI console may
        // simply not be open.
        let _ = self.tx.send(line.clone());

        // Never let a logging I/O hiccup take down the caller; best-effort only.
        if let Ok(json) = serde_json::to_string(&line) {
            let mut writer = self.writer.clone();
            let _ = writeln!(writer, "{json}");
        }
    }
}

/// Extracts `message`, `job_id`, and `destination` fields from a `tracing::Event`.
#[derive(Default)]
struct LogVisitor {
    message: String,
    job_id: Option<String>,
    destination: Option<String>,
}

impl Visit for LogVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => self.message = value.to_string(),
            "job_id" => self.job_id = Some(value.to_string()),
            "destination" => self.destination = Some(value.to_string()),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let formatted = format!("{value:?}");
        match field.name() {
            "message" => self.message = formatted,
            "job_id" => self.job_id = Some(formatted),
            "destination" => self.destination = Some(formatted),
            _ => {}
        }
    }
}

/// A [`std::io::Write`] sink over a single log file that rotates by size instead of
/// time: once the active file has grown past `max_bytes`, the next write shifts
/// `app.log.{1..4}` up by one (`app.log.4` -> `app.log.5`, dropping the previous
/// `app.log.5`), renames `app.log` to `app.log.1`, and starts a fresh `app.log`.
///
/// `max_bytes` is a constructor parameter (rather than a hard-coded 10 MiB) so tests can
/// exercise rotation with a small threshold.
pub(crate) struct SizeRotatingWriter {
    log_path: PathBuf,
    file: Option<File>,
    size: u64,
    max_bytes: u64,
}

impl SizeRotatingWriter {
    pub(crate) fn new(log_path: impl Into<PathBuf>, max_bytes: u64) -> io::Result<Self> {
        let log_path = log_path.into();
        let file = open_log_file(&log_path)?;
        let size = file.metadata()?.len();
        Ok(Self {
            log_path,
            file: Some(file),
            size,
            max_bytes,
        })
    }

    fn backup_path(&self, index: u32) -> PathBuf {
        let base = self
            .log_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("app.log");
        self.log_path.with_file_name(format!("{base}.{index}"))
    }

    fn rotate(&mut self) -> io::Result<()> {
        // Close the current handle before renaming: on Windows a rename of an
        // open file fails.
        self.file = None;

        let oldest = self.backup_path(MAX_BACKUPS);
        if oldest.exists() {
            fs::remove_file(&oldest)?;
        }
        for index in (1..MAX_BACKUPS).rev() {
            let src = self.backup_path(index);
            if src.exists() {
                fs::rename(&src, self.backup_path(index + 1))?;
            }
        }
        if self.log_path.exists() {
            fs::rename(&self.log_path, self.backup_path(1))?;
        }

        self.file = Some(open_log_file(&self.log_path)?);
        self.size = 0;
        Ok(())
    }
}

/// Opens (creating if needed, appending otherwise) the log file at `path`. On Unix,
/// MINOR #8 (T-6.3 audit) hardens it to `0o600` (owner read/write only) — `logs/app.log`
/// can carry redacted-but-still-operationally-sensitive detail (file paths, job ids,
/// bucket/folder names), so it gets the same treatment as `config.json`
/// (`config.rs::harden_permissions`). Sets the mode both at creation (`OpenOptionsExt`,
/// covers the common case cheaply) and via an explicit `set_permissions` afterwards
/// (covers a file that already existed with looser permissions from before this
/// hardening existed, or from a rotation's `create(true)` reopening a file `rename`d out
/// from under a previous, differently-permissioned one). A no-op on Windows for the same
/// reason as `config.rs`: no POSIX mode bit to set there.
#[cfg(unix)]
fn open_log_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

#[cfg(not(unix))]
fn open_log_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

impl Write for SizeRotatingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.size >= self.max_bytes && !buf.is_empty() {
            self.rotate()?;
        }
        let file = self
            .file
            .as_mut()
            .expect("SizeRotatingWriter always holds an open file handle between calls");
        let written = file.write(buf)?;
        self.size += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

/// Masks known secret shapes in `s` (RNF-015): AWS access key ids, Service Account
/// `private_key` JSON values, Google OAuth2 (`ya29.`) tokens, and `Bearer` HTTP
/// authorization tokens. Text that matches none of these patterns is returned
/// unchanged.
pub fn redact(s: &str) -> String {
    // Credential scope masking runs before the bare AKIA pass below: the scope embeds
    // an access key id (`Credential=AKIA.../date/region/service/aws4_request`), and
    // masking the whole value here — rather than letting the AKIA pass fire first and
    // leave the date/region/service metadata exposed — is what "mask credential scope"
    // requires.
    let masked = mask_greedy_run(
        s,
        "Credential=",
        |c| c.is_ascii_alphanumeric() || c == '/' || c == '-' || c == '_',
        "Credential=[REDACTED]",
    );
    let masked = mask_fixed_run(
        &masked,
        "AKIA",
        16,
        |c| c.is_ascii_digit() || c.is_ascii_uppercase(),
        "[REDACTED]",
    );
    let masked = mask_fixed_run(
        &masked,
        "ASIA",
        16,
        |c| c.is_ascii_digit() || c.is_ascii_uppercase(),
        "[REDACTED]",
    );
    let masked = mask_json_string_field(&masked, "private_key");
    let masked = mask_json_string_field(&masked, "client_secret");
    let masked = mask_fixed_run(
        &masked,
        "Signature=",
        64,
        |c| c.is_ascii_digit() || ('a'..='f').contains(&c),
        "Signature=[REDACTED]",
    );
    let masked = mask_greedy_run(
        &masked,
        "ya29.",
        |c| c.is_ascii_alphanumeric() || c == '_' || c == '-',
        "[REDACTED]",
    );
    let masked = mask_greedy_run(
        &masked,
        "Bearer ",
        |c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-',
        "Bearer [REDACTED]",
    );
    mask_secret_adjacent_base64(&masked)
}

/// Replaces every occurrence of `prefix` followed by exactly `run_len` characters
/// matching `is_run_char` (a fixed-length token, e.g. an AWS access key id) with
/// `replacement`. A `prefix` not followed by a full matching run is left untouched — and
/// so is one immediately followed by *more* than `run_len` matching characters: that
/// means the token is actually longer than this pattern expects, so masking only the
/// first `run_len` characters would leave part of a real secret in the clear while
/// giving the false impression the whole thing was redacted. The boundary check is what
/// tells the two cases apart.
fn mask_fixed_run(
    input: &str,
    prefix: &str,
    run_len: usize,
    is_run_char: impl Fn(char) -> bool,
    replacement: &str,
) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(idx) = rest.find(prefix) {
        out.push_str(&rest[..idx]);
        let after_prefix = &rest[idx + prefix.len()..];
        let run: String = after_prefix.chars().take(run_len).collect();
        let boundary_ok = after_prefix
            .chars()
            .nth(run_len)
            .map(|next| !is_run_char(next))
            .unwrap_or(true);
        if run.chars().count() == run_len && run.chars().all(&is_run_char) && boundary_ok {
            out.push_str(replacement);
            rest = &after_prefix[run.len()..];
        } else {
            out.push_str(prefix);
            rest = after_prefix;
        }
    }
    out.push_str(rest);
    out
}

/// Replaces every occurrence of `prefix` followed by one or more characters matching
/// `is_run_char` (a variable-length token, e.g. a bearer token) with `replacement`. A
/// `prefix` not followed by at least one matching character is left untouched.
fn mask_greedy_run(
    input: &str,
    prefix: &str,
    is_run_char: impl Fn(char) -> bool,
    replacement: &str,
) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(idx) = rest.find(prefix) {
        out.push_str(&rest[..idx]);
        let after_prefix = &rest[idx + prefix.len()..];
        let run_len_bytes = after_prefix
            .char_indices()
            .find(|&(_, c)| !is_run_char(c))
            .map(|(byte_idx, _)| byte_idx)
            .unwrap_or(after_prefix.len());
        if run_len_bytes > 0 {
            out.push_str(replacement);
            rest = &after_prefix[run_len_bytes..];
        } else {
            out.push_str(prefix);
            rest = after_prefix;
        }
    }
    out.push_str(rest);
    out
}

/// Replaces the value of every `"<key>": "..."` JSON field with `"<key>":"[REDACTED]"`,
/// keeping the surrounding text intact. A `"<key>"` not followed by a well-formed
/// `: "..."` value is left untouched. Shared by both `private_key` (Service Account
/// JSON) and `client_secret` (OAuth client credentials JSON).
fn mask_json_string_field(input: &str, key: &str) -> String {
    let quoted_key = format!("\"{key}\"");
    let replacement = format!("\"{key}\":\"[REDACTED]\"");
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(idx) = rest.find(quoted_key.as_str()) {
        out.push_str(&rest[..idx]);
        let after_key = &rest[idx + quoted_key.len()..];
        let after_ws1 = after_key.trim_start();
        if let Some(after_colon) = after_ws1.strip_prefix(':') {
            let after_ws2 = after_colon.trim_start();
            if let Some(value_start) = after_ws2.strip_prefix('"') {
                if let Some(end) = value_start.find('"') {
                    out.push_str(&replacement);
                    rest = &value_start[end + 1..];
                    continue;
                }
            }
        }
        // Not a well-formed `"<key>": "..."` field — leave it as-is and keep scanning
        // past this occurrence of the key.
        out.push_str(&quoted_key);
        rest = after_key;
    }
    out.push_str(rest);
    out
}

/// Finds every case-insensitive occurrence of the word `secret`, followed by up to 3
/// separator characters (e.g. `="`, `: "`) and then a 40-character run of base64
/// alphabet characters (`A-Za-z0-9+/`), and masks just that 40-character run with
/// `[REDACTED]`. Catches ad-hoc secret values (AWS secret access keys are exactly 40
/// such characters, unpadded) that don't match any of the other, more specific patterns
/// in [`redact`]. The `secret`/`Secret` marker itself and the separator are left
/// untouched — only the value is masked. Deliberately excludes `=` from the run
/// alphabet: real unpadded 40-character secrets never contain it, and treating it as a
/// value character would make it ambiguous with the `key="value"` separator this
/// function also has to recognize. Like [`mask_fixed_run`], a run immediately followed
/// by another base64-alphabet character is left untouched (it's longer than 40
/// characters and therefore not this exact-width pattern) — and a marker followed by
/// more than 3 non-alphabet characters before any candidate run (e.g. an identifier like
/// `aws_secret_access_key=`) is also left untouched, since by then the value is no
/// longer "adjacent" to the marker.
fn mask_secret_adjacent_base64(input: &str) -> String {
    const MARKER_LEN: usize = "secret".len();
    const RUN_LEN: usize = 40;
    let is_b64 = |c: char| c.is_ascii_alphanumeric() || c == '+' || c == '/';

    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while rest.len() >= MARKER_LEN {
        // ASCII-only lowercasing preserves byte offsets, so indices found here apply
        // unchanged to `rest` itself.
        let lower_rest = rest.to_ascii_lowercase();
        let Some(marker_idx) = lower_rest.find("secret") else {
            break;
        };
        out.push_str(&rest[..marker_idx + MARKER_LEN]);
        let after_marker = &rest[marker_idx + MARKER_LEN..];

        let mut sep_end = 0usize;
        for (sep_count, c) in after_marker.chars().enumerate() {
            if is_b64(c) || sep_count >= 3 {
                break;
            }
            sep_end += c.len_utf8();
        }
        let (sep, after_sep) = after_marker.split_at(sep_end);
        let run: String = after_sep.chars().take(RUN_LEN).collect();
        let boundary_ok = after_sep
            .chars()
            .nth(RUN_LEN)
            .map(|next| !is_b64(next))
            .unwrap_or(true);

        out.push_str(sep);
        if run.chars().count() == RUN_LEN && run.chars().all(is_b64) && boundary_ok {
            out.push_str("[REDACTED]");
            rest = &after_sep[run.len()..];
        } else {
            rest = after_sep;
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_never_exceeds_capacity_and_keeps_the_newest_lines() {
        let handle = init_for_tests();

        for i in 0..1_200u32 {
            tracing::info!("event {i}");
        }

        let recent = handle.recent(10_000);
        assert_eq!(recent.len(), RING_CAPACITY);
        assert_eq!(recent.first().unwrap().message, "event 700");
        assert_eq!(recent.last().unwrap().message, "event 1199");
    }

    #[test]
    fn log_line_carries_job_id_and_destination_when_present() {
        let handle = init_for_tests();

        tracing::info!(job_id = "job-42", destination = "gdrive", "upload started");
        tracing::info!("no structured fields here");

        let recent = handle.recent(10);
        let with_fields = recent
            .iter()
            .find(|l| l.message == "upload started")
            .expect("event with fields should be in the ring");
        assert_eq!(with_fields.job_id.as_deref(), Some("job-42"));
        assert_eq!(with_fields.destination.as_deref(), Some("gdrive"));

        let without_fields = recent
            .iter()
            .find(|l| l.message == "no structured fields here")
            .expect("event without fields should be in the ring");
        assert_eq!(without_fields.job_id, None);
        assert_eq!(without_fields.destination, None);
    }

    #[test]
    fn broadcast_subscriber_receives_the_event() {
        let handle = init_for_tests();
        let mut rx = handle.subscribe();

        tracing::warn!(job_id = "job-7", "disk almost full");

        let line = rx.try_recv().expect("a line should have been broadcast");
        assert_eq!(line.message, "disk almost full");
        assert_eq!(line.level, "WARN");
        assert_eq!(line.job_id.as_deref(), Some("job-7"));
    }

    #[test]
    fn build_filter_in_debug_honors_rust_log_override() {
        let filter = build_filter("info", Some("warn".to_string()), true);
        assert_eq!(filter.to_string(), "warn");
    }

    #[test]
    fn build_filter_in_release_ignores_rust_log_override() {
        let filter = build_filter("info", Some("trace".to_string()), false);
        assert_eq!(
            filter.to_string(),
            "info",
            "a release build must never let RUST_LOG raise verbosity above `level`"
        );
    }

    #[test]
    fn build_filter_falls_back_to_level_when_no_override_or_override_is_malformed() {
        // No override at all, in debug.
        assert_eq!(build_filter("warn", None, true).to_string(), "warn");
        // Override present but not parseable as a directive, in debug.
        assert_eq!(
            build_filter("warn", Some(">>> not a directive <<<".to_string()), true).to_string(),
            "warn"
        );
        // No override at all, in release.
        assert_eq!(build_filter("warn", None, false).to_string(), "warn");
    }

    #[test]
    fn redact_masks_aws_access_key_id() {
        let input = "using key AKIAABCDEFGHIJKLMNOP for upload";
        assert_eq!(redact(input), "using key [REDACTED] for upload");
    }

    #[test]
    fn redact_masks_service_account_private_key_json_field() {
        let input = r#"credentials: {"type":"service_account","private_key":"-----BEGIN PRIVATE KEY-----\nMIIExyz\n-----END PRIVATE KEY-----\n","client_email":"x@y.iam.gserviceaccount.com"}"#;
        let redacted = redact(input);
        assert!(redacted.contains(r#""private_key":"[REDACTED]""#));
        assert!(!redacted.contains("BEGIN PRIVATE KEY"));
        // Unrelated fields survive untouched.
        assert!(redacted.contains(r#""client_email":"x@y.iam.gserviceaccount.com""#));
    }

    #[test]
    fn redact_masks_google_oauth_access_token() {
        let input = "authorization token ya29.a0ARrdaM-abcDEF_123 attached";
        assert_eq!(redact(input), "authorization token [REDACTED] attached");
    }

    #[test]
    fn redact_masks_bearer_authorization_header() {
        let input = "sending header Authorization: Bearer abc123.def-456_ghi";
        assert_eq!(
            redact(input),
            "sending header Authorization: Bearer [REDACTED]"
        );
    }

    #[test]
    fn redact_leaves_normal_text_intact() {
        let input = "uploaded file report-2026-09-04.csv (12.3 MB) to gdrive in 1.2s";
        assert_eq!(redact(input), input);
    }

    /// Table test (VULN-006): each row is `(label, input, expected_output)`. Positive
    /// rows assert a secret shape gets masked; negative rows assert `redact` leaves
    /// look-alike or unrelated text alone.
    #[test]
    fn redact_table() {
        let cases: &[(&str, &str, &str)] = &[
            (
                "AWS access key id",
                "key AKIAABCDEFGHIJKLMNOP in use",
                "key [REDACTED] in use",
            ),
            (
                "AWS STS temporary access key id",
                "key ASIAABCDEFGHIJKLMNOP in use",
                "key [REDACTED] in use",
            ),
            (
                "OAuth client_secret JSON field",
                r#"{"client_id":"abc","client_secret":"s3cr3t-value-123"}"#,
                r#"{"client_id":"abc","client_secret":"[REDACTED]"}"#,
            ),
            (
                "AWS SigV4 credential scope",
                "Authorization: AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, SignedHeaders=host",
                "Authorization: AWS4-HMAC-SHA256 Credential=[REDACTED], SignedHeaders=host",
            ),
            (
                "AWS SigV4 signature",
                "... Signature=1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef done",
                "... Signature=[REDACTED] done",
            ),
            (
                "generic 40-char base64 run adjacent to `secret=\"...\"`",
                "secret=\"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\" appended",
                "secret=\"[REDACTED]\" appended",
            ),
            (
                "generic secret run, capitalized marker, `Secret: ...`",
                "Secret: wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY end",
                "Secret: [REDACTED] end",
            ),
            (
                "negative: bare word secret with no adjacent value",
                "this log message mentions a secret but carries no value",
                "this log message mentions a secret but carries no value",
            ),
            (
                "negative: secret value not adjacent (identifier text in between)",
                "aws_secret_access_key=\"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\" configured",
                "aws_secret_access_key=\"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\" configured",
            ),
            (
                "negative: AKIA prefix with a run shorter than 16 chars",
                "key AKIASHORT stays as-is",
                "key AKIASHORT stays as-is",
            ),
            (
                "negative: AKIA run boundary — a 17th alnum char means it isn't this token",
                "key AKIAABCDEFGHIJKLMNOPQ stays as-is",
                "key AKIAABCDEFGHIJKLMNOPQ stays as-is",
            ),
            (
                "negative: ordinary text with numbers and punctuation",
                "retry 3/5 for job job-42 after 200ms",
                "retry 3/5 for job job-42 after 200ms",
            ),
        ];

        for (label, input, expected) in cases {
            assert_eq!(&redact(input), expected, "case failed: {label}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn size_rotating_writer_creates_app_log_as_owner_read_write_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("app.log");
        let _writer = SizeRotatingWriter::new(&log_path, DEFAULT_MAX_FILE_BYTES).unwrap();

        let mode = std::fs::metadata(&log_path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "app.log must be owner read/write only, got {mode:o}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn size_rotating_writer_rehardens_permissions_after_rotation() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("app.log");
        let threshold = 16u64;
        let mut writer = SizeRotatingWriter::new(&log_path, threshold).unwrap();

        writer.write_all(&[b'x'; 32]).unwrap();
        writer.write_all(b"trigger rotation").unwrap();

        let mode = std::fs::metadata(&log_path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "app.log must stay owner read/write only across rotation, got {mode:o}"
        );
    }

    #[test]
    fn size_rotation_produces_up_to_five_backups_and_never_a_sixth() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("app.log");
        let threshold = 1024u64; // 1 KiB
        let mut writer = SizeRotatingWriter::new(&log_path, threshold).unwrap();

        // Force well over 5 rotations: each write is 300 bytes, threshold is 1 KiB,
        // so a rotation happens roughly every 4 writes.
        let chunk = vec![b'x'; 300];
        for _ in 0..200 {
            writer.write_all(&chunk).unwrap();
        }
        writer.flush().unwrap();

        assert!(log_path.exists(), "active app.log must still exist");
        for i in 1..=5 {
            assert!(
                dir.path().join(format!("app.log.{i}")).exists(),
                "app.log.{i} should exist after enough rotations"
            );
        }
        assert!(
            !dir.path().join("app.log.6").exists(),
            "app.log.6 must never be created"
        );
    }

    #[test]
    fn init_second_call_is_already_initialized() {
        let dir = tempfile::tempdir().unwrap();
        // The first call may itself fail with `AlreadyInitialized` if another test in
        // this binary already installed a global default first — that's fine, the
        // process only ever allows one. What matters is that by the time we make our
        // second call here, *some* global default is guaranteed to be installed, so it
        // must fail.
        let _ = init(dir.path(), "info");
        let second = init(dir.path(), "info");
        assert!(matches!(second, Err(LoggingError::AlreadyInitialized)));
    }
}
