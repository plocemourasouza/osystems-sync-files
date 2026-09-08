/**
 * LogLineRow — one 20px row inside `LogConsole`'s body (PRD.md RF-066;
 * DESIGN.md §8 "LogConsole" / "LogLine").
 *
 * Anatomy: `[HH:MM:SS.mmm]` timestamp (mono, quaternary, `formatLogTs`) ·
 * origin tag chip (`source(line)`, tone per source — see `SOURCE_TONE`) ·
 * message (`truncate` + `title` for the full text on hover, `#<job_id>`
 * suffix appended when the event carried one) · optional `error`/`path`
 * detail (`detailText`), rendered as a second truncated mono span in
 * `--color-text-tertiary` right after the message (task contract T-1.2).
 * `level` "WARN"/"ERROR" additionally recolors the message text (task
 * contract T-2.10) — the timestamp stays quaternary, the tag chip keeps
 * its own fixed tone, and the detail span stays tertiary regardless of
 * level; only the message itself escalates.
 *
 * The detail span is capped at `max-w-[38%]` and independently truncated
 * (own `min-w-0`/`truncate`/`title`) so a long `error` or `path` value —
 * an S3 SDK error message, a deep Windows path — narrows itself instead of
 * pushing the row past its fixed 20px height or forcing horizontal scroll;
 * the message span keeps first claim on the remaining width.
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

/**
 * Joins `error`/`path` (when present) into the row's secondary detail text —
 * `error` first, since a failure's cause outranks the path it happened on.
 * `null` when the line carries neither field, so callers can skip rendering
 * the detail span entirely rather than rendering an empty one.
 */
export function detailText(line: LogLine): string | null {
  const parts = [line.error, line.path].filter((value): value is string => Boolean(value));
  return parts.length > 0 ? parts.join(" — ") : null;
}

export function LogLineRow({ line }: LogLineRowProps): JSX.Element {
  const tag = source(line);
  const suffix = line.job_id ? ` #${line.job_id}` : "";
  const fullText = `${line.message}${suffix}`;
  const detail = detailText(line);

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
      {detail && (
        <span
          className="min-w-0 max-w-[38%] shrink truncate font-mono text-label-sm leading-4 text-text-tertiary"
          title={detail}
        >
          {detail}
        </span>
      )}
    </div>
  );
}
