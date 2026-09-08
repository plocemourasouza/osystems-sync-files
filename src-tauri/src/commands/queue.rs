//! Watcher/queue IPC commands (SPEC.md §7): `pick_folder`, `rescan`, `pause_watcher`,
//! `resume_watcher`, `list_jobs`, `get_status`, `retry_job`, `retry_all_failed`,
//! `cancel_job`, `clear_completed`, `test_connection`, `open_remote`, `pause_job`,
//! `resume_job`.

use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use osystems_sync_core::config;
use osystems_sync_core::queue;
use osystems_sync_core::rescan::RescanReport;
use osystems_sync_core::state::{AppStatus, Destination, ListJobsPage, ListJobsQuery};
use osystems_sync_core::uploaders::TestResult;

use crate::error::AppError;
use crate::state::AppState;

/// Upper bound accepted by any command taking a caller-supplied `limit`, so a
/// misbehaving/compromised frontend can't force a multi-gigabyte SELECT or `Vec`
/// allocation (RNF-007/RNF-008 — bounded resource use).
const MAX_LIST_LIMIT: i64 = 1000;

/// Opens the native "choose a folder" dialog (blocking on the dialog thread pool, not
/// the Tokio runtime), and on `Some(path)`: persists it into `watch.path`, saves
/// `config.json`, restarts the watcher against the new folder, and emits
/// `status-changed`. Returns `None` without touching anything if the user cancels.
#[tauri::command]
pub async fn pick_folder(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, AppError> {
    let picked = tauri::async_runtime::spawn_blocking({
        let app = app.clone();
        move || app.dialog().file().blocking_pick_folder()
    })
    .await
    .map_err(|join_err| {
        tracing::error!(error = %join_err, "tarefa de diálogo do pick_folder entrou em panic");
        AppError::new("queue.pick_folder", "failed to open the folder picker")
    })?;

    let Some(file_path) = picked else {
        return Ok(None);
    };
    let path = file_path.to_string();

    let mut new_config = state.config.read().await.clone();
    new_config.watch.path = Some(path.clone());

    let dir = state.data_dir.clone();
    let to_persist = new_config.clone();
    tauri::async_runtime::spawn_blocking(move || config::save(&dir, &to_persist))
        .await
        .map_err(|join_err| {
            tracing::error!(error = %join_err, "tarefa de salvamento de configuração do pick_folder entrou em panic");
            AppError::new("config.io", "failed to persist the chosen folder")
        })??;

    *state.config.write().await = new_config.clone();

    state
        .runtime
        .restart_watcher(app.clone(), state.repo.clone(), new_config.watch.clone())
        .await;

    let status = state
        .runtime
        .build_app_status(&state.repo)
        .await
        .map_err(|err| {
            tracing::error!(error = %err, "falha ao construir o status do app após o pick_folder");
            AppError::new("state.db", "failed to read the current status")
        })?;
    crate::events::emit_status_changed(&app, &status);

    Ok(Some(path))
}

/// Reconciles the queue with the current `watch.path` and filters (SPEC.md §6,
/// RF-004): enqueues every new/changed file, archives the queued jobs of files that no
/// longer pass the filters or are gone from disk, and restores the ones that pass
/// again.
///
/// Returns the whole [`RescanReport`] rather than just the enqueued count: with the
/// reconciliation pass a single number can no longer describe what happened, and the
/// struct already crosses IPC with a generated TS type.
///
/// PLAN.md T-2.5: unlike every other rescan call site, this one does NOT go
/// through `events::notify_rescan_failure` for
/// `RescanError::AlreadyInProgress` — a background initiator can silently
/// no-op on a skip, but this button is the user explicitly asking for a scan,
/// so it must surface "já existe uma varredura em andamento" as the command's
/// own error (`rescan.in_progress`, see `error.rs`) rather than either
/// swallowing it or, worse, returning a `RescanReport` with `enqueued: 0` --
/// which would be indistinguishable from the exact silent-failure bug this
/// guard exists to prevent.
#[tauri::command]
pub async fn rescan(app: AppHandle, state: State<'_, AppState>) -> Result<RescanReport, AppError> {
    let watch = state.config.read().await.watch.clone();

    // Only the manual, button-triggered rescan reports progress: it is the
    // one call site the UI shows a live scan indicator for (PLAN.md T-2.4) --
    // boot/resume/tray rescans still go through the plain `rescan()`.
    let progress_sink = crate::events::rescan_progress_sink(&app);
    let report = osystems_sync_core::rescan::rescan_with_progress(
        state.repo.clone(),
        state.runtime.wakers(),
        &watch,
        &state.runtime.stabilize_config(),
        &progress_sink,
    )
    .await
    .map_err(|err| {
        if matches!(
            err,
            osystems_sync_core::rescan::RescanError::AlreadyInProgress
        ) {
            tracing::debug!("varredura manual: já existe uma varredura em andamento, ignorada");
        } else {
            tracing::warn!(error = %err, "varredura manual falhou");
        }
        AppError::from(err)
    })?;

    let status = state
        .runtime
        .build_app_status(&state.repo)
        .await
        .map_err(AppError::from)?;
    crate::events::emit_status_changed(&app, &status);

    Ok(report)
}

/// Pauses the watcher (SPEC.md §6, RF12): debounced events are discarded from now on,
/// in-flight worker uploads are never affected.
#[tauri::command]
pub async fn pause_watcher(app: AppHandle, state: State<'_, AppState>) -> Result<(), AppError> {
    state.runtime.pause().await;

    let status = state
        .runtime
        .build_app_status(&state.repo)
        .await
        .map_err(AppError::from)?;
    crate::events::emit_status_changed(&app, &status);

    Ok(())
}

/// Resumes the watcher and runs a `rescan()` to catch anything that arrived while
/// paused (SPEC.md §6, RF-005).
#[tauri::command]
pub async fn resume_watcher(app: AppHandle, state: State<'_, AppState>) -> Result<(), AppError> {
    let watch = state.config.read().await.watch.clone();

    // The watcher itself already resumed by this point (`SyncRuntime::resume` flips
    // it back on before running the rescan) — only the reconciliation scan can still
    // fail here, so a failure is surfaced via `rescan-failed` rather than failing the
    // whole command: the toggle the user asked for did work, and `AppError`-ing this
    // command would abort `DashboardHeader`'s `refresh()` right after, hiding that a
    // real state change (watcher resumed) succeeded behind an error banner.
    if let Err(err) = state.runtime.resume(state.repo.clone(), watch).await {
        crate::events::notify_rescan_failure(&app, "resume_watcher", err);
    }

    let status = state
        .runtime
        .build_app_status(&state.repo)
        .await
        .map_err(AppError::from)?;
    crate::events::emit_status_changed(&app, &status);

    Ok(())
}

/// Paginated job listing for the Dashboard/Fila views (SPEC.md §7).
#[tauri::command]
pub async fn list_jobs(
    state: State<'_, AppState>,
    query: ListJobsQuery,
) -> Result<ListJobsPage, AppError> {
    let mut query = query;
    query.limit = clamp_limit(query.limit);

    osystems_sync_core::queue::with_repo(state.repo.clone(), move |repo| repo.list_jobs(&query))
        .await
        .map_err(AppError::from)
}

/// Single aggregated snapshot for the Dashboard header (SPEC.md §7).
#[tauri::command]
pub async fn get_status(state: State<'_, AppState>) -> Result<AppStatus, AppError> {
    state
        .runtime
        .build_app_status(&state.repo)
        .await
        .map_err(AppError::from)
}

/// Resets one job back to `pending` (RF-034 single-job form) and wakes the worker
/// pool so it can pick it up right away instead of waiting for the next poll.
#[tauri::command]
pub async fn retry_job(
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: String,
) -> Result<(), AppError> {
    let id = job_id.clone();
    queue::with_repo(state.repo.clone(), move |r| r.retry_job(&id))
        .await
        .map_err(AppError::from)?;

    state.runtime.wakers().notify_all();
    emit_job_and_status(&app, &state, &job_id).await
}

/// Pauses one job's active side (PLAN.md T-5.5): cancels its in-flight upload (if
/// any) and marks it `paused` so the worker pool leaves it alone until
/// [`resume_job`] is called. A no-op (`Ok(())`) if the side is already paused;
/// `job.invalid_transition` if it's in any other terminal/inapplicable status.
#[tauri::command]
pub async fn pause_job(
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: String,
) -> Result<(), AppError> {
    match state.pool.get() {
        Some(pool) => pool.pause_job(&job_id).await.map_err(AppError::from)?,
        None => {
            tracing::warn!("pause_job chamado antes do pool de workers terminar de iniciar");
            return Err(AppError::new(
                "job.not_ready",
                "worker pool is still starting",
            ));
        }
    }

    emit_job_and_status(&app, &state, &job_id).await
}

/// Resumes a previously [`pause_job`]-paused job, setting it back to `pending` and
/// waking the worker pool for its destination so it's picked up right away.
#[tauri::command]
pub async fn resume_job(
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: String,
) -> Result<(), AppError> {
    match state.pool.get() {
        Some(pool) => pool.resume_job(&job_id).await.map_err(AppError::from)?,
        None => {
            tracing::warn!("resume_job chamado antes do pool de workers terminar de iniciar");
            return Err(AppError::new(
                "job.not_ready",
                "worker pool is still starting",
            ));
        }
    }

    emit_job_and_status(&app, &state, &job_id).await
}

/// RF-034: resets every `failed` job back to `pending`. Returns how many were reset.
#[tauri::command]
pub async fn retry_all_failed(app: AppHandle, state: State<'_, AppState>) -> Result<u32, AppError> {
    let n = queue::with_repo(state.repo.clone(), |r| r.retry_all_failed())
        .await
        .map_err(AppError::from)?;

    if n > 0 {
        state.runtime.wakers().notify_all();
        emit_status_only(&app, &state).await?;
    }
    Ok(n)
}

/// RF-035: cancels a `pending`/`uploading` job. Best-effort aborts any in-progress
/// remote upload first (S3's `AbortMultipartUpload`) so a cancelled job doesn't keep
/// billing on the remote side — a failure to abort is logged but never blocks the
/// cancellation itself, since the job must end up `cancelled` in `state.db` either way.
#[tauri::command]
pub async fn cancel_job(
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: String,
) -> Result<(), AppError> {
    let lookup_id = job_id.clone();
    let row = queue::with_repo(state.repo.clone(), move |r| r.job_by_id(&lookup_id))
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::new("queue.not_found", "job not found"))?;

    if let Some(remote_state) = row
        .remote_state
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
    {
        let uploader = {
            let uploaders = state.uploaders.read().await;
            match row.destination {
                Destination::S3 => uploaders.s3.clone(),
                Destination::GDrive => uploaders.gdrive.clone(),
            }
        };
        if let Some(uploader) = uploader {
            if let Err(err) = uploader.abort(&remote_state).await {
                tracing::warn!(
                    job_id = %job_id,
                    error = %err,
                    "melhor esforço de aborto do upload remoto em andamento falhou; o job continua marcado como cancelado localmente"
                );
            }
        }
    }

    let cancel_id = job_id.clone();
    queue::with_repo(state.repo.clone(), move |r| r.cancel_job(&cancel_id))
        .await
        .map_err(AppError::from)?;

    state.runtime.wakers().notify_all();
    emit_job_and_status(&app, &state, &job_id).await
}

/// RF-036: archives every job whose file is fully `done` on both destinations.
/// Returns how many files were archived.
#[tauri::command]
pub async fn clear_completed(app: AppHandle, state: State<'_, AppState>) -> Result<u32, AppError> {
    let now = chrono::Utc::now().to_rfc3339();
    let n = queue::with_repo(state.repo.clone(), move |r| r.clear_completed(&now))
        .await
        .map_err(AppError::from)?;

    if n > 0 {
        emit_status_only(&app, &state).await?;
    }
    Ok(n)
}

/// "Testar conexão" (SPEC.md §7): probes the currently configured uploader for
/// `destination`. Fails with `credentials.missing` rather than panicking/`None`-ing
/// silently when no uploader is configured yet (e.g. AWS keys never entered, or S3
/// disabled in `config.json`).
#[tauri::command]
pub async fn test_connection(
    state: State<'_, AppState>,
    destination: Destination,
) -> Result<TestResult, AppError> {
    let uploader = {
        let uploaders = state.uploaders.read().await;
        match destination {
            Destination::S3 => uploaders.s3.clone(),
            Destination::GDrive => uploaders.gdrive.clone(),
        }
    };
    let Some(uploader) = uploader else {
        return Err(AppError::new(
            "credentials.missing",
            "no uploader is configured for this destination yet",
        ));
    };

    uploader.test_connection().await.map_err(AppError::from)
}

/// Opens a completed job's remote object in the default browser (SPEC.md §6
/// "Abrir no destino"): the S3 console, deep-linked to the object's key, or Drive's
/// own `web_view_link`.
///
/// S3's `remote_state` only carries `{"etag": ...}` after a job finishes — the
/// in-progress `{"upload_id","key","parts"}` shape is gone by then (`s3.rs`'s
/// `with_state_sink` stops firing once `upload()` returns) — so the object key is
/// reconstructed here as `{s3.prefix}{file name}`, exactly how `worker.rs` derived
/// `UploadRequest::remote_name` (`view.name.clone()`) and `s3.rs` derived the actual
/// object key (`{prefix}{remote_name}`) in the first place.
#[tauri::command]
pub async fn open_remote(
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: String,
) -> Result<(), AppError> {
    let lookup_id = job_id.clone();
    let row = queue::with_repo(state.repo.clone(), move |r| r.job_by_id(&lookup_id))
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::new("queue.not_found", "job not found"))?;

    let url = match row.destination {
        Destination::S3 => {
            let view_id = job_id.clone();
            let view = queue::with_repo(state.repo.clone(), move |r| r.job_view_for_job(&view_id))
                .await
                .map_err(AppError::from)?
                .ok_or_else(|| AppError::new("queue.not_found", "job not found"))?;
            let cfg = state.config.read().await;
            let key = format!("{}{}", cfg.s3.prefix, view.name);
            s3_console_url(&cfg.s3.bucket, &cfg.s3.region, &key)
        }
        Destination::GDrive => row
            .remote_state
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|v| {
                v.get("web_view_link")
                    .and_then(|l| l.as_str())
                    .map(str::to_string)
            })
            .ok_or_else(|| {
                AppError::new("queue.no_remote_link", "this job has no remote link yet")
            })?,
    };

    app.opener()
        .open_url(url, None::<&str>)
        .map_err(AppError::from)
}

/// Builds the S3 console deep link for one object (SPEC.md §6 "Abrir no destino").
/// Pure so it has a direct unit test independent of any live AWS config. `bucket`,
/// `region`, and `key` are percent-encoded (MINOR #9 / T-6.3 audit) before being spliced
/// into the URL — a bucket/prefix/watched-file name is user- or config-controlled and
/// otherwise unescaped input in a URL that's handed straight to the OS opener is exactly
/// the shape of an open-redirect/URL-injection bug (e.g. a `&`/`#` in a file name
/// altering the query string, or a `\n`/control character breaking whatever eventually
/// parses this URL).
fn s3_console_url(bucket: &str, region: &str, key: &str) -> String {
    let bucket = percent_encode_component(bucket);
    let region = percent_encode_component(region);
    let key = percent_encode_component(key);
    format!("https://s3.console.aws.amazon.com/s3/object/{bucket}?region={region}&prefix={key}")
}

/// Percent-encodes `input` for use as one URL component, RFC 3986 §2.3's unreserved set
/// (`A-Za-z0-9-._~`) plus `/` left unescaped — everything else becomes `%XX` (uppercase
/// hex of the raw UTF-8 byte). `/` is deliberately kept literal even though it's not in
/// the unreserved set: the S3 key this function is most often called with is itself a
/// `/`-delimited path (`prefix/sub/file.ext`), and the S3 console reads that `prefix`
/// query parameter the same way — encoding `/` to `%2F` would turn a folder path into an
/// unrecognizable single opaque segment instead of leaving it merely safely escaped. No
/// external crate: this workspace avoids adding dependencies for something this small
/// (see `SPEC.md` §4, same rationale as `logging::redact` avoiding `regex`).
fn percent_encode_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        let c = byte as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~' | '/') {
            out.push(c);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

/// Re-emits `job-updated` for `job_id` (if it still exists) plus a fresh
/// `status-changed` — the common tail of every single-job mutating command above.
async fn emit_job_and_status(
    app: &AppHandle,
    state: &State<'_, AppState>,
    job_id: &str,
) -> Result<(), AppError> {
    let lookup_id = job_id.to_string();
    if let Some(job) = queue::with_repo(state.repo.clone(), move |r| r.job_view_for_job(&lookup_id))
        .await
        .map_err(AppError::from)?
    {
        crate::events::emit_job_updated(app, &job);
    }
    emit_status_only(app, state).await
}

/// Emits a fresh `status-changed` only — the tail of the bulk (`retry_all_failed`,
/// `clear_completed`) commands, which don't name a single job to re-emit.
async fn emit_status_only(app: &AppHandle, state: &State<'_, AppState>) -> Result<(), AppError> {
    let status = state
        .runtime
        .build_app_status(&state.repo)
        .await
        .map_err(AppError::from)?;
    crate::events::emit_status_changed(app, &status);
    Ok(())
}

/// Clamps a caller-supplied `list_jobs` limit to `(0, MAX_LIST_LIMIT]`, defaulting a
/// non-positive value to `MAX_LIST_LIMIT` rather than rejecting it outright — a `0` or
/// negative limit almost certainly means "no limit was set" client-side, not "return
/// nothing".
fn clamp_limit(limit: i64) -> i64 {
    if limit <= 0 {
        MAX_LIST_LIMIT
    } else {
        limit.min(MAX_LIST_LIMIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_limit_defaults_non_positive_to_the_max() {
        assert_eq!(clamp_limit(0), MAX_LIST_LIMIT);
        assert_eq!(clamp_limit(-5), MAX_LIST_LIMIT);
    }

    #[test]
    fn clamp_limit_passes_through_values_within_range() {
        assert_eq!(clamp_limit(1), 1);
        assert_eq!(clamp_limit(200), 200);
        assert_eq!(clamp_limit(MAX_LIST_LIMIT), MAX_LIST_LIMIT);
    }

    #[test]
    fn clamp_limit_caps_values_above_the_max() {
        assert_eq!(clamp_limit(MAX_LIST_LIMIT + 1), MAX_LIST_LIMIT);
        assert_eq!(clamp_limit(i64::MAX), MAX_LIST_LIMIT);
    }

    #[test]
    fn s3_console_url_embeds_bucket_region_and_key() {
        let url = s3_console_url("my-bucket", "sa-east-1", "backups/photo.jpg");
        assert_eq!(
            url,
            "https://s3.console.aws.amazon.com/s3/object/my-bucket?region=sa-east-1&prefix=backups/photo.jpg"
        );
    }

    #[test]
    fn s3_console_url_handles_an_empty_prefix() {
        let url = s3_console_url("bucket", "us-east-1", "photo.jpg");
        assert_eq!(
            url,
            "https://s3.console.aws.amazon.com/s3/object/bucket?region=us-east-1&prefix=photo.jpg"
        );
    }

    #[test]
    fn s3_console_url_percent_encodes_special_characters_in_the_key() {
        let url = s3_console_url("bucket", "us-east-1", "a&b#c d.bin");
        assert_eq!(
            url,
            "https://s3.console.aws.amazon.com/s3/object/bucket?region=us-east-1&prefix=a%26b%23c%20d.bin"
        );
    }

    #[test]
    fn percent_encode_component_leaves_unreserved_characters_and_slash_untouched() {
        assert_eq!(
            percent_encode_component("backups/photo-2026_09.04.jpg~bak"),
            "backups/photo-2026_09.04.jpg~bak"
        );
    }

    #[test]
    fn percent_encode_component_escapes_everything_else() {
        assert_eq!(percent_encode_component("a&b#c d.bin"), "a%26b%23c%20d.bin");
        assert_eq!(percent_encode_component("100%"), "100%25");
        assert_eq!(percent_encode_component("café"), "caf%C3%A9");
    }
}
