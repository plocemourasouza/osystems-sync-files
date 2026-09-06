/**
 * LogLineRow — one 20px row inside `LogConsole`'s body (PRD.md RF-066;
 * DESIGN.md §8 "LogConsole" / "LogLine").
 *
 * Anatomy: `[HH:MM:SS.mmm]` timestamp (mono, quaternary, `formatLogTs`) ·
 * origin tag chip (`source(line)`, tone per source — see `SOURCE_TONE`) ·
 * message (`truncate` + `title` for the full text on hover, `#<job_id>`
 * suffix appended when the event carried one). `level` "WARN"/"ERROR"
 * additionally recolors the message text (task contract T-2.10) — the
 * timestamp stays quaternary and the tag chip keeps its own fixed tone
 * regardless of level, only the message itself escalates.
 */
import type { JSX } from "react";

import { Badge, type BadgeTone } from "@/components/ui";
import { source, type LogSource } from "@/store/logStore";
import type { LogLine } from "@/types/generated";

export type LogLineRowProps = {
  line: LogLine;
};

/**
 * Origin tag → `Badge` tone (task contract T-2.10; `Badge` only ships 5
 * tones). Exported for `LogTicker`, which shows a single line in the
 * collapsed console header and must tag it identically.
 */
export const SOURCE_TONE: Record<LogSource, BadgeTone> = {
  WATCHER: "info",
  HASH: "neutral",
  STATE: "neutral",
  S3: "warning",
  GDRIVE: "info",
  CORE: "neutral",
  APP: "neutral",
};

/**
 * Formats an RFC3339 UTC timestamp as `[HH:MM:SS.mmm]` (DESIGN.md §8).
 * Uses UTC getters deliberately — the source `ts` is already UTC and this
 * keeps rendering deterministic regardless of the machine's local timezone
 * (both at runtime and in tests). Malformed input renders a dashed placeholder
 * instead of "Invalid Date".
 */
export function formatLogTs(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "[--:--:--.---]";

  const hh = String(date.getUTCHours()).padStart(2, "0");
  const mm = String(date.getUTCMinutes()).padStart(2, "0");
  const ss = String(date.getUTCSeconds()).padStart(2, "0");
  const ms = String(date.getUTCMilliseconds()).padStart(3, "0");
  return `[${hh}:${mm}:${ss}.${ms}]`;
}

/**
 * `warn`/`error` (case-insensitive `LogLine.level`) escalate the message's
 * text tone. Exported so `LogTicker` colours the collapsed header's line the
 * same way the expanded body colours it.
 */
export function messageToneClassName(level: string): string {
  const upper = level.toUpperCase();
  if (upper === "ERROR") return "text-error";
  if (upper === "WARN" || upper === "WARNING") return "text-secondary";
  return "text-text-primary";
}

export function LogLineRow({ line }: LogLineRowProps): JSX.Element {
  const tag = source(line);
  const suffix = line.job_id ? ` #${line.job_id}` : "";
  const fullText = `${line.message}${suffix}`;

  return (
    <div className="flex h-5 shrink-0 items-center gap-sm" data-level={line.level}>
      <span className="shrink-0 whitespace-nowrap font-mono text-label-md text-text-quaternary">
        {formatLogTs(line.ts)}
      </span>
      <Badge tone={SOURCE_TONE[tag]} className="shrink-0">
        {tag}
      </Badge>
      <span
        className={`min-w-0 flex-1 truncate font-mono text-label-md leading-4 ${messageToneClassName(line.level)}`}
        title={fullText}
      >
        {fullText}
      </span>
    </div>
  );
}
