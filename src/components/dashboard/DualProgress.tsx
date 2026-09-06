/**
 * DualProgress — per-destination progress cell in `JobTable`'s "Google
 * Drive"/"AWS S3" columns (PRD.md RF-062; DESIGN.md §6 "Movimento" linear
 * bars, §7 icons, §8 "JobTable"). One `JobSide` in, one 4px bar + right-hand
 * status text out.
 *
 * Reuses `formatRate` from `@/components/shell/Sidebar` (already exported
 * there as "part of the public contract") instead of duplicating it.
 */
import { AlertTriangle } from "lucide-react";
import type { JSX } from "react";

import { formatRate } from "@/components/shell/Sidebar";
import { t } from "@/i18n";
import type { JobSide, UploadProgress } from "@/types/generated";

export type DualProgressAccent = "primary" | "secondary";

export type DualProgressProps = {
  side: JobSide;
  progress?: UploadProgress;
  accent: DualProgressAccent;
};

const FILL_CLASSNAME: Record<DualProgressAccent, string> = {
  primary: "bg-primary-strong",
  secondary: "bg-secondary-strong",
};

/** Truncates a raw `AppError`/`last_error` message for the compact cell text. */
function shortError(message: string): string {
  const trimmed = message.trim();
  return trimmed.length <= 40 ? trimmed : `${trimmed.slice(0, 37)}…`;
}

function percentFor(side: JobSide, progress?: UploadProgress): number {
  if (side.status === "done") return 100;
  if (side.status === "uploading" && progress && progress.total > 0) {
    return Math.min(100, Math.round((progress.sent / progress.total) * 100));
  }
  return 0;
}

function RightText({ side, progress, accent }: DualProgressProps): JSX.Element {
  if (side.status === "failed") {
    const message = side.last_error ?? t("pages.dashboard.table.unknownError");
    return (
      <span className="flex min-w-0 items-center gap-2xs text-body-sm text-error">
        <AlertTriangle aria-hidden="true" size={12} className="shrink-0" />
        <span className="truncate">{shortError(message)}</span>
        <span
          className="shrink-0 underline decoration-dotted underline-offset-2"
          title={side.last_error ?? undefined}
        >
          {t("pages.dashboard.table.details")}
        </span>
      </span>
    );
  }

  if (side.status === "uploading") {
    return (
      <span className="font-mono text-label-sm text-text-secondary">{formatRate(progress?.rate_bps ?? 0)}</span>
    );
  }

  if (side.status === "done") {
    return (
      <span className="font-mono text-label-sm text-tertiary">
        {accent === "primary" ? t("pages.dashboard.table.shaOk") : t("pages.dashboard.table.etagOk")}
      </span>
    );
  }

  if (side.status === "paused") {
    return <span className="font-mono text-label-sm text-secondary">{t("pages.dashboard.table.paused")}</span>;
  }

  if (side.status === "cancelled") {
    return (
      <span className="font-mono text-label-sm text-text-quaternary">{t("pages.dashboard.status.cancelled")}</span>
    );
  }

  return <span className="font-mono text-label-sm text-text-quaternary">{t("pages.dashboard.table.queued")}</span>;
}

export function DualProgress({ side, progress, accent }: DualProgressProps): JSX.Element {
  const percent = percentFor(side, progress);
  const label =
    accent === "primary" ? t("pages.dashboard.table.gdriveProgressLabel") : t("pages.dashboard.table.s3ProgressLabel");

  return (
    <div className="flex flex-col gap-2xs">
      <div className="flex items-baseline justify-between gap-sm">
        <span className="font-mono text-label-sm text-text-secondary">{percent}%</span>
        <RightText side={side} progress={progress} accent={accent} />
      </div>
      <div
        role="progressbar"
        aria-valuenow={percent}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label={label}
        className="h-1 w-full overflow-hidden rounded-full bg-surface-0"
      >
        <span
          aria-hidden="true"
          className={`block h-full transition-[width] duration-base ease-standard ${
            side.status === "failed" ? "bg-error" : FILL_CLASSNAME[accent]
          }`}
          style={{ width: `${percent}%` }}
        />
      </div>
    </div>
  );
}
