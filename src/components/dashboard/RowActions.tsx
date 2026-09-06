/**
 * RowActions — `JobRow`'s per-row action cluster (PRD.md RF-065; DESIGN.md §8
 * "JobTable"/"RowActions"). Every action is an inline icon button with a
 * `Tooltip`: Abrir no Explorer and Copiar caminho always apply; Reenviar
 * (`failed`/`cancelled`), Pausar (`pending`/`uploading`), Retomar (`paused`),
 * Cancelar (`pending`/`uploading`/`paused`), Abrir remoto (`done`) and
 * Detalhes do erro (`last_error`) render only when they apply to at least one
 * side of the job. Nothing hides behind a `⋯` menu.
 *
 * Only applicable actions render (never a disabled placeholder), so the widest
 * realistic cluster is 6 buttons — a mixed job with one side failed and the
 * other uploading. At 24px per button (`size="icon"`) plus 2px gaps that is
 * ~154px, which is what the widened "Ações" column in `JobTable`'s `colgroup`
 * budgets for; the 1366×768 overlap that the old `⋯` menu worked around is
 * paid for there instead.
 *
 * Each action tracks its own `busy` kind so only the button in flight shows a
 * spinner while the others are disabled alongside it. Failures surface through
 * `onError` — `JobRow` renders that message as an inline `role="alert"` row for
 * 5s (PLAN.md T-3.11) — this component never refetches: the store's
 * `job-updated` subscription (already wired in `JobTable`) is what settles the
 * row once the backend catches up.
 */
import { AlertCircle, Copy, ExternalLink, FolderOpen, type LucideIcon, Pause, Play, RotateCw, X } from "lucide-react";
import { useState, type JSX } from "react";

import { cancelJob, isAppError, openInExplorer, openRemote, pauseJob, resumeJob, retryJob } from "@/api/ipc";
import { Button, Tooltip } from "@/components/ui";
import { t, tError } from "@/i18n";
import { relativeTime } from "@/lib/relativeTime";
import { jobSide } from "@/store/jobsStore";
import type { JobSide, JobView } from "@/types/generated";

import { DetailsDialog } from "./DetailsDialog";

export type RowActionsProps = {
  job: JobView;
  /** Reports the message of the last failed action (`null` clears it). */
  onError: (message: string | null) => void;
};

type Destination = "gdrive" | "s3";
type BusyKind = "explorer" | "retry" | "cancel" | "pause" | "resume" | "remote" | null;

const RETRYABLE_STATUSES: JobSide["status"][] = ["failed", "cancelled"];
const CANCELLABLE_STATUSES: JobSide["status"][] = ["pending", "uploading", "paused"];
const PAUSABLE_STATUSES: JobSide["status"][] = ["pending", "uploading"];
const RESUMABLE_STATUSES: JobSide["status"][] = ["paused"];

/** One rendered icon button. `busyKind` is `null` for actions that don't hit the backend. */
type RowAction = {
  key: string;
  label: string;
  Icon: LucideIcon;
  busyKind: BusyKind;
  onClick: () => void;
};

function destinationLabel(destination: Destination): string {
  return destination === "gdrive" ? t("pages.dashboard.table.headers.gdrive") : t("pages.dashboard.table.headers.s3");
}

function errorMessage(e: unknown): string {
  return tError(isAppError(e) ? e.code : "unknown");
}

export function RowActions({ job, onError }: RowActionsProps): JSX.Element {
  const [busy, setBusy] = useState<BusyKind>(null);
  const [dialogOpen, setDialogOpen] = useState(false);

  const sides: Array<{ destination: Destination; side: JobSide }> = [
    { destination: "gdrive", side: jobSide(job, "gdrive") },
    { destination: "s3", side: jobSide(job, "s3") },
  ];

  const retryableSides = sides.filter(({ side }) => RETRYABLE_STATUSES.includes(side.status));
  const cancellableSides = sides.filter(({ side }) => CANCELLABLE_STATUSES.includes(side.status));
  const pausableSides = sides.filter(({ side }) => PAUSABLE_STATUSES.includes(side.status));
  const resumableSides = sides.filter(({ side }) => RESUMABLE_STATUSES.includes(side.status));
  const doneSides = sides.filter(({ side }) => side.status === "done");
  const errorSides = sides.filter(({ side }) => side.last_error !== null);

  const anyBusy = busy !== null;

  async function run(kind: BusyKind, action: () => Promise<unknown>): Promise<void> {
    setBusy(kind);
    onError(null);
    try {
      await action();
    } catch (e) {
      onError(errorMessage(e));
    } finally {
      setBusy(null);
    }
  }

  function handleOpenExplorer(): void {
    void run("explorer", () => openInExplorer(job.path));
  }

  function handleCopyPath(): void {
    function fail(): void {
      onError(t("pages.dashboard.table.actions.copyPathFailed"));
    }

    // `navigator.clipboard` is absent in insecure contexts and in jsdom, and
    // `writeText` rejects when the WebView denies the permission — both paths
    // have to land on the same inline error.
    try {
      const written = navigator.clipboard?.writeText(job.path);
      if (written === undefined) {
        fail();
        return;
      }
      void written.catch(fail);
    } catch {
      fail();
    }
  }

  function handleRetry(): void {
    void run("retry", () => Promise.all(retryableSides.map(({ side }) => retryJob(side.job_id))));
  }

  function handleCancel(): void {
    void run("cancel", () => Promise.all(cancellableSides.map(({ side }) => cancelJob(side.job_id))));
  }

  function handlePause(): void {
    void run("pause", () => Promise.all(pausableSides.map(({ side }) => pauseJob(side.job_id))));
  }

  function handleResume(): void {
    void run("resume", () => Promise.all(resumableSides.map(({ side }) => resumeJob(side.job_id))));
  }

  function handleOpenRemote(jobId: string): void {
    void run("remote", () => openRemote(jobId));
  }

  // Fixed order, so an icon keeps the same relative position from row to row
  // even though the set itself varies with status.
  const actions: RowAction[] = [
    {
      key: "explorer",
      label: t("pages.dashboard.table.actions.openExplorer"),
      Icon: FolderOpen,
      busyKind: "explorer",
      onClick: handleOpenExplorer,
    },
    {
      key: "copyPath",
      label: t("pages.dashboard.table.actions.copyPath"),
      Icon: Copy,
      busyKind: null,
      onClick: handleCopyPath,
    },
  ];

  if (retryableSides.length > 0) {
    actions.push({
      key: "retry",
      label: t("pages.dashboard.table.actions.retry"),
      Icon: RotateCw,
      busyKind: "retry",
      onClick: handleRetry,
    });
  }

  if (pausableSides.length > 0) {
    actions.push({
      key: "pause",
      label: t("pages.dashboard.table.actions.pause"),
      Icon: Pause,
      busyKind: "pause",
      onClick: handlePause,
    });
  }

  if (resumableSides.length > 0) {
    actions.push({
      key: "resume",
      label: t("pages.dashboard.table.actions.resume"),
      Icon: Play,
      busyKind: "resume",
      onClick: handleResume,
    });
  }

  if (cancellableSides.length > 0) {
    actions.push({
      key: "cancel",
      label: t("pages.dashboard.table.actions.cancel"),
      Icon: X,
      busyKind: "cancel",
      onClick: handleCancel,
    });
  }

  // One button per finished destination when both are done, so the tooltip can
  // name which remote it opens; a single button ("Abrir remoto") otherwise.
  for (const { destination, side } of doneSides) {
    actions.push({
      key: `openRemote-${destination}`,
      label:
        doneSides.length > 1
          ? t("pages.dashboard.table.actions.openRemoteTo", { destination: destinationLabel(destination) })
          : t("pages.dashboard.table.actions.openRemote"),
      Icon: ExternalLink,
      busyKind: "remote",
      onClick: () => handleOpenRemote(side.job_id),
    });
  }

  if (errorSides.length > 0) {
    actions.push({
      key: "errorDetails",
      label: t("pages.dashboard.table.actions.errorDetails"),
      Icon: AlertCircle,
      busyKind: null,
      onClick: () => setDialogOpen(true),
    });
  }

  return (
    <div className="flex flex-nowrap items-center justify-end gap-2xs overflow-visible">
      {actions.map(({ key, label, Icon, busyKind, onClick }) => (
        <Tooltip key={key} label={label}>
          <Button
            variant="ghost"
            size="icon"
            loading={busyKind !== null && busy === busyKind}
            disabled={anyBusy && busy !== busyKind}
            aria-label={label}
            icon={<Icon aria-hidden="true" size={14} />}
            onClick={onClick}
          />
        </Tooltip>
      ))}

      <DetailsDialog
        open={dialogOpen}
        title={t("pages.dashboard.table.actions.errorDetails")}
        onClose={() => setDialogOpen(false)}
      >
        {errorSides.map(({ destination, side }) => (
          <div
            key={destination}
            className="flex flex-col gap-2xs rounded-md border border-border-hairline bg-surface-1 p-sm"
          >
            <span className="font-mono text-label-sm uppercase text-text-tertiary">
              {destinationLabel(destination)}
            </span>
            <dl className="grid grid-cols-[auto_1fr] gap-x-sm gap-y-2xs text-body-sm">
              <dt className="text-text-tertiary">{t("pages.dashboard.table.errorDialog.attempts")}</dt>
              <dd className="font-mono text-text-primary">{side.attempts}</dd>
              <dt className="text-text-tertiary">{t("pages.dashboard.table.errorDialog.nextAttempt")}</dt>
              <dd className="font-mono text-text-primary">
                {side.next_attempt_at !== null
                  ? relativeTime(side.next_attempt_at)
                  : t("pages.dashboard.table.errorDialog.noNextAttempt")}
              </dd>
            </dl>
            <pre className="whitespace-pre-wrap break-words rounded bg-surface-0 p-xs font-mono text-label-sm text-error">
              {side.last_error}
            </pre>
          </div>
        ))}
      </DetailsDialog>
    </div>
  );
}
