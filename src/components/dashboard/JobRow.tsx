/**
 * JobRow — one 40px `JobTable` row: file identity, size, the two
 * `DualProgress` destination cells, the aggregated `StatusBadge`, and row
 * actions (PRD.md RF-062/063/065; DESIGN.md §8 "JobTable"/"JobRow").
 *
 * Row actions are delegated to `RowActions` (PRD.md RF-033/035/065/098): this
 * component only owns the transient action-error banner — `RowActions`
 * reports a failed action's message via `onError`, which is rendered inline
 * as a `role="alert"` line under the Ações cell for `ACTION_ERROR_DISPLAY_MS`
 * before self-clearing (PLAN.md T-3.11).
 */
import { useEffect, useRef, useState, type JSX } from "react";

import { formatBytes } from "@/lib/format";
import { relativeDir } from "@/lib/relativeDir";
import { relativeTime } from "@/lib/relativeTime";
import { ErrorText } from "@/components/ui";
import { useConfigStore } from "@/store/configStore";
import { aggregateStatus, jobSide } from "@/store/jobsStore";
import type { JobView, UploadProgress } from "@/types/generated";

import { DualProgress } from "./DualProgress";
import { FileTypeIcon } from "./FileTypeIcon";
import { RowActions } from "./RowActions";
import { StatusBadge } from "./StatusBadge";

export type JobRowProps = {
  job: JobView;
  /** Full `jobsStore.progress` map, keyed by the *side's* `job_id`. */
  progress: Record<string, UploadProgress>;
};

/** How long a failed action's error message stays visible under the row's actions. */
const ACTION_ERROR_DISPLAY_MS = 5000;

export function JobRow({ job, progress }: JobRowProps): JSX.Element {
  const gdrive = jobSide(job, "gdrive");
  const s3 = jobSide(job, "s3");
  const status = aggregateStatus(job);
  const watchRoot = useConfigStore((s) => s.config.watch.path);
  const origin = relativeDir(job.path, watchRoot);

  const [actionError, setActionError] = useState<string | null>(null);
  const errorTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => {
      if (errorTimer.current) clearTimeout(errorTimer.current);
    };
  }, []);

  function handleActionError(message: string | null): void {
    if (errorTimer.current) {
      clearTimeout(errorTimer.current);
      errorTimer.current = null;
    }
    setActionError(message);
    if (message !== null) {
      errorTimer.current = setTimeout(() => {
        setActionError(null);
        errorTimer.current = null;
      }, ACTION_ERROR_DISPLAY_MS);
    }
  }

  return (
    <tr className="h-10 border-b border-border-hairline last:border-b-0 hover:bg-surface-hover">
      <td className="min-w-0 px-md py-0 align-middle">
        <div className="flex min-w-0 items-center gap-sm">
          <FileTypeIcon name={job.name} className="shrink-0 text-text-tertiary" />
          <div className="flex min-w-0 flex-col">
            <span className="truncate text-body-xs text-text-primary" title={job.name}>
              {job.name}
            </span>
            <span className="truncate font-mono text-label-sm leading-[0.8125rem] text-text-quaternary" title={origin}>
              {origin} · {relativeTime(job.detected_at)}
            </span>
          </div>
        </div>
      </td>
      <td className="min-w-0 whitespace-nowrap px-md py-0 text-right align-middle font-mono text-label-md tabular-nums text-text-secondary">
        {formatBytes(job.size)}
      </td>
      <td className="min-w-0 px-md py-0 align-middle">
        <DualProgress side={gdrive} progress={progress[gdrive.job_id]} accent="primary" />
      </td>
      <td className="min-w-0 px-md py-0 align-middle">
        <DualProgress side={s3} progress={progress[s3.job_id]} accent="secondary" />
      </td>
      <td className="min-w-0 py-0 pl-md pr-sm align-middle">
        <StatusBadge status={status} className="max-w-full truncate" />
      </td>
      <td className="min-w-0 px-md py-0 align-middle">
        <div className="flex flex-col items-end gap-2xs">
          <RowActions job={job} onError={handleActionError} />
          {actionError !== null && <ErrorText className="text-right text-label-sm">{actionError}</ErrorText>}
        </div>
      </td>
    </tr>
  );
}
