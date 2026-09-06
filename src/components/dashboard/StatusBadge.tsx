/**
 * StatusBadge — `JobRow`'s aggregated status pill (PRD.md RF-063;
 * DESIGN.md §2 "Matriz de status" / §8 "JobTable").
 *
 * Wraps `@/components/ui`'s `Badge` (tone + 6px dot) rather than
 * reimplementing it. The "Enviando" pulse (§6 "Movimento") targets the
 * dot — `Badge`'s first child span — via an arbitrary-variant className
 * since `Badge` itself takes no pulse prop; `motion-reduce:` turns it back
 * off, matching §6's reduced-motion rule.
 */
import type { JSX } from "react";

import { Badge, cn, type BadgeTone } from "@/components/ui";
import { t } from "@/i18n";
import type { AggregatedStatus } from "@/store/jobsStore";

const TONE_BY_STATUS: Record<AggregatedStatus, BadgeTone> = {
  uploading: "info",
  done: "success",
  queued: "neutral",
  failed: "error",
  paused: "warning",
  cancelled: "neutral",
};

function labelFor(status: AggregatedStatus): string {
  switch (status) {
    case "uploading":
      return t("pages.dashboard.status.uploading");
    case "done":
      return t("pages.dashboard.status.done");
    case "queued":
      return t("pages.dashboard.status.queued");
    case "failed":
      return t("pages.dashboard.status.failed");
    case "paused":
      return t("pages.dashboard.status.paused");
    case "cancelled":
      return t("pages.dashboard.status.cancelled");
  }
}

/** Targets `Badge`'s dot (its first child span) — see module docstring. */
const PULSE_CLASSNAME = "[&>span:first-child]:animate-pulse motion-reduce:[&>span:first-child]:animate-none";

export type StatusBadgeProps = {
  status: AggregatedStatus;
  className?: string;
};

export function StatusBadge({ status, className }: StatusBadgeProps): JSX.Element {
  return (
    <Badge
      tone={TONE_BY_STATUS[status]}
      className={cn(status === "uploading" ? PULSE_CLASSNAME : undefined, className)}
    >
      {labelFor(status)}
    </Badge>
  );
}
