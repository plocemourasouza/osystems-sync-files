/**
 * Typed IPC wrappers around Tauri commands (SPEC.md §7).
 *
 * Convention (CLAUDE.md): components/hooks never call `invoke` directly —
 * they go through the wrappers in this file.
 */
import { invoke } from "@tauri-apps/api/core";

import type {
  AppConfig,
  AuthorLink,
  AppStatus,
  CredentialStatus,
  Destination,
  ListJobsPage,
  ListJobsQuery,
  LogLine,
  RescanReport,
  TestResult,
  ValidationIssue,
  ServiceAccountInfo,
} from "@/types/generated";

/**
 * Enables or disables launching oSystems Sync on Windows login
 * (`set_autostart` command, backed by `tauri-plugin-autostart`).
 */
export async function setAutostart(enabled: boolean): Promise<void> {
  await invoke("set_autostart", { enabled });
}

/**
 * Reads the persisted `config.json` (`get_config` command, SPEC.md §7). A missing
 * file on the Rust side yields `AppConfig::default()`, never an error.
 */
export async function getConfig(): Promise<AppConfig> {
  return invoke<AppConfig>("get_config");
}

/**
 * Validates and persists `config.json` (`save_config` command, SPEC.md §7).
 * Rejects with an {@link AppError} of code `"config.invalid"` (message: JSON array
 * of {@link ValidationIssue}) when validation fails.
 */
export async function saveConfig(config: AppConfig): Promise<void> {
  await invoke("save_config", { config });
}

/**
 * Opens the native folder picker (`pick_folder` command, SPEC.md §7, via
 * `tauri-plugin-dialog`). Resolves `null` when the user cancels.
 */
export async function pickFolder(): Promise<string | null> {
  return invoke<string | null>("pick_folder");
}

/**
 * Reconciles the queue with `watch.path` and the current filters (`rescan`
 * command, SPEC.md §7 / RF-061 / RF-004).
 *
 * Resolves the full `RescanReport`: since the scan both enqueues and archives,
 * a single number can no longer describe what it did.
 */
export async function rescan(): Promise<RescanReport> {
  return invoke<RescanReport>("rescan");
}

/** Pauses the local watcher (`pause_watcher` command, RF12). Jobs already queued keep uploading. */
export async function pauseWatcher(): Promise<void> {
  await invoke("pause_watcher");
}

/** Resumes a paused watcher (`resume_watcher` command, RF12). */
export async function resumeWatcher(): Promise<void> {
  await invoke("resume_watcher");
}

/**
 * Fetches one server-side-paginated page of jobs (`list_jobs` command, SPEC.md §7 /
 * RF-064). Mirrors `saveConfig`'s calling convention: Tauri commands receive their
 * argument as a named object (`{ query }`), not positionally.
 */
export async function listJobs(query: ListJobsQuery): Promise<ListJobsPage> {
  return invoke<ListJobsPage>("list_jobs", { query });
}

/**
 * Reads the current watcher/destinations/counters snapshot (`get_status` command,
 * SPEC.md §7). Re-broadcast as the `status-changed` event whenever it changes —
 * see `@/api/events`.
 */
export async function getStatus(): Promise<AppStatus> {
  return invoke<AppStatus>("get_status");
}

/**
 * Reads up to `limit` of the most recent structured log lines (`get_recent_logs`
 * command, SPEC.md §7), used to hydrate the Console's ring buffer on mount
 * (RF-066) — see `@/store/logStore`.
 */
export async function getRecentLogs(limit: number): Promise<LogLine[]> {
  return invoke<LogLine[]>("get_recent_logs", { limit });
}

/** Opens the logs folder in the OS file explorer (`open_logs_folder` command, RF-066). */
export async function openLogsFolder(): Promise<void> {
  await invoke("open_logs_folder");
}

/**
 * Opens one of the author's public profiles in the default browser
 * (`open_author_link` command, RF-093).
 *
 * Takes the profile's name, never a URL: the addresses live in
 * `commands::system::AuthorLink` so the renderer cannot ask the OS handler to
 * open something else.
 */
export async function openAuthorLink(link: AuthorLink): Promise<void> {
  await invoke("open_author_link", { link });
}

// ---------------------------------------------------------------------------
// Fase 3 (PLAN.md T-3.10) — credentials, connection tests, queue actions, QoS.
//
// Arg-naming convention (binding for T-3.9/T-3.11/T-3.12): none of the Rust
// `#[tauri::command]` handlers in src-tauri/src/commands/*.rs declare
// `#[tauri::command(rename_all = "snake_case")]`, so Tauri applies its
// default per-command argument renaming: every multi-word snake_case Rust
// parameter (e.g. `job_id`, `limit_mbps`) is invoked from JS as camelCase
// (`jobId`, `limitMbps`). This mirrors the existing Fase 2 wrappers above
// (`getRecentLogs` -> `{ limit }` is single-word so it doesn't show the
// rename, but `list_jobs` -> `{ query }` follows the same named-object
// pattern). SPEC.md §7 documents command params in Rust snake_case; that is
// backend-side documentation only — always convert to camelCase here.
// ---------------------------------------------------------------------------

/**
 * Keyring key for a stored credential (`set_credential`/`clear_credential`,
 * SPEC.md §7). Values are the literal constants from
 * `core::credentials::KEY_AWS_ACCESS_KEY_ID` / `KEY_AWS_SECRET_ACCESS_KEY` —
 * do not invent new ones without checking that Rust module first.
 */
export type CredentialKey = "aws.access_key_id" | "aws.secret_access_key" | "gdrive.service_account_json";

/**
 * Stores one credential value in the OS keyring (`set_credential` command).
 * RNF-003 / CLAUDE.md: the renderer only ever sends a secret in, it never
 * reads one back — call {@link getCredentialStatus} afterwards for a mask.
 */
export async function setCredential(key: CredentialKey, value: string): Promise<void> {
  await invoke("set_credential", { key, value });
}

/**
 * Reads whether AWS/GDrive credentials are present, returning only masked
 * values (`get_credential_status` command) — never the real secret.
 */
export async function getCredentialStatus(): Promise<CredentialStatus> {
  return invoke<CredentialStatus>("get_credential_status");
}

/** Removes a stored credential from the OS keyring (`clear_credential` command). */
export async function clearCredential(key: CredentialKey): Promise<void> {
  await invoke("clear_credential", { key });
}

/**
 * Probes a destination with the currently *saved* config + credentials
 * (`test_connection` command). Note: per the backend uploader contract
 * (`uploaders/s3.rs::test_connection`), this only ever resolves with
 * `{ ok: true, ... }` on success — any failure step rejects with an
 * {@link AppError}, it never resolves `{ ok: false }`. Callers must
 * try/catch, not branch on `result.ok`.
 */
export async function testConnection(destination: Destination): Promise<TestResult> {
  return invoke<TestResult>("test_connection", { destination });
}

/**
 * Metadata of a Service Account JSON picked via `pick_service_account_file`
 * (T-4.6/T-4.8). Rust returns only non-secret fields — the `private_key`
 * (and every other JSON claim) never crosses the IPC boundary (RNF-003).
 * Type comes from the ts-rs export (`src/types/generated.ts`, RNF-010).
 */
export type { ServiceAccountInfo } from "@/types/generated";

/**
 * Opens the native file picker for a Google Drive Service Account JSON
 * (`pick_service_account_file` command, T-4.8, via `tauri-plugin-dialog`).
 * Resolves `null` when the user cancels. Never returns raw JSON/`private_key`
 * content (RNF-003) — only the metadata in {@link ServiceAccountInfo}.
 */
export async function pickServiceAccountFile(): Promise<ServiceAccountInfo | null> {
  return invoke<ServiceAccountInfo | null>("pick_service_account_file");
}

/** Re-enqueues one failed job (`retry_job` command). */
export async function retryJob(jobId: string): Promise<void> {
  await invoke("retry_job", { jobId });
}

/** Re-enqueues every failed job, returning how many were retried (`retry_all_failed` command). */
export async function retryAllFailed(): Promise<number> {
  return invoke<number>("retry_all_failed");
}

/** Cancels one queued/uploading job (`cancel_job` command). */
export async function cancelJob(jobId: string): Promise<void> {
  await invoke("cancel_job", { jobId });
}

/** Removes every `done`/`cancelled` job from the queue view, returning the count cleared (`clear_completed` command). */
export async function clearCompleted(): Promise<number> {
  return invoke<number>("clear_completed");
}

/** Pauses one `pending`/`uploading` job's active side(s) (`pause_job` command, T-5.9/RF-037). */
export async function pauseJob(jobId: string): Promise<void> {
  await invoke("pause_job", { jobId });
}

/** Resumes one `paused` job (`resume_job` command, T-5.9/RF-037). */
export async function resumeJob(jobId: string): Promise<void> {
  await invoke("resume_job", { jobId });
}

/** Sets (or clears, with `null`) the bandwidth cap for one destination (`set_qos` command). */
export async function setQos(destination: Destination, limitMbps: number | null): Promise<void> {
  await invoke("set_qos", { destination, limitMbps });
}

/** Reveals a local file in the OS file explorer (`open_in_explorer` command). */
export async function openInExplorer(path: string): Promise<void> {
  await invoke("open_in_explorer", { path });
}

/** Opens a job's remote object (S3 console / GDrive file) in the default browser (`open_remote` command). */
export async function openRemote(jobId: string): Promise<void> {
  await invoke("open_remote", { jobId });
}

/**
 * Shape of every error rejected by a Tauri command (SPEC.md §7: "Erros dos
 * commands: retornar `Result<T, AppError>` onde `AppError { code, message }`").
 */
export type AppError = { code: string; message: string };

/** Narrows an unknown `invoke` rejection to an {@link AppError}. */
export function isAppError(e: unknown): e is AppError {
  if (typeof e !== "object" || e === null) return false;
  const candidate = e as Record<string, unknown>;
  return typeof candidate.code === "string" && typeof candidate.message === "string";
}

/**
 * Parses the `message` of a `code: "config.invalid"` {@link AppError} into its
 * `ValidationIssue[]` payload. Never throws — returns `[]` if `message` is not
 * valid JSON or is not shaped like `ValidationIssue[]`.
 */
export function parseValidationIssues(err: AppError): ValidationIssue[] {
  let parsed: unknown;
  try {
    parsed = JSON.parse(err.message);
  } catch {
    return [];
  }

  if (!Array.isArray(parsed)) return [];

  return parsed.filter((item): item is ValidationIssue => {
    if (typeof item !== "object" || item === null) return false;
    const candidate = item as Record<string, unknown>;
    return typeof candidate.field === "string" && typeof candidate.message === "string";
  });
}
