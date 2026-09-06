/**
 * LogTicker — the newest log line, shown inline in `LogConsole`'s header
 * while the console is collapsed (PRD.md RF-066; DESIGN.md §8 "LogConsole").
 *
 * Collapsing the console hands its ~220px to the job table, but it also used
 * to hand over all visibility: a collapsed console said nothing about what
 * the daemon was doing. This renders the single most recent line right after
 * the "Rust Core" chip, so the console stays a live indicator even shut.
 *
 * Presentation deliberately mirrors `LogLineRow` — same `formatLogTs`, same
 * `SOURCE_TONE` chip, same `messageToneClassName` escalation — so expanding
 * the console shows the same line styled identically, not a second dialect.
 * It is one line, never a scrolling marquee: no animation means nothing to
 * undo under `prefers-reduced-motion`, and a value that changes in place is
 * what a status readout is.
 *
 * A11y: `aria-live="off"` on purpose. The line changes as often as the
 * daemon logs, and announcing every one would bury everything else on the
 * page; the expanded body (`role="log"`, `aria-live="polite"`) remains the
 * announced surface, and this keeps an `aria-label` so it is still reachable
 * on demand.
 */
import type { JSX } from "react";

import { Badge } from "@/components/ui";
import { t } from "@/i18n";
import { source } from "@/store/logStore";
import type { LogLine } from "@/types/generated";

import { formatLogTs, messageToneClassName, SOURCE_TONE } from "./LogLineRow";

export type LogTickerProps = {
  line: LogLine;
};

export function LogTicker({ line }: LogTickerProps): JSX.Element {
  const tag = source(line);
  const suffix = line.job_id ? ` #${line.job_id}` : "";
  const fullText = `${line.message}${suffix}`;

  return (
    <div
      aria-live="off"
      aria-label={t("pages.dashboard.console.tickerLabel")}
      data-level={line.level}
      data-testid="log-ticker"
      className="flex min-w-0 flex-1 items-center gap-sm"
    >
      {/* Hidden below 1200px: the same breakpoint at which the header drops
          its "ao vivo" text, and the first thing worth losing when the
          message itself needs the room. */}
      <span className="hidden shrink-0 whitespace-nowrap font-mono text-label-sm text-text-quaternary min-[1200px]:inline">
        {formatLogTs(line.ts)}
      </span>
      <Badge tone={SOURCE_TONE[tag]} className="shrink-0">
        {tag}
      </Badge>
      <span
        className={`min-w-0 flex-1 truncate font-mono text-label-sm leading-4 ${messageToneClassName(line.level)}`}
        title={fullText}
      >
        {fullText}
      </span>
    </div>
  );
}
