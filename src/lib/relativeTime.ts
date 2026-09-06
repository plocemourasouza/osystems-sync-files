/**
 * relativeTime — coarse "time ago" label for `JobRow`'s "Arquivo & Origem"
 * cell (PRD.md RF-062: `<name>` / `<path>` · "há N min").
 *
 * Takes `now` explicitly (default `new Date()`) so callers/tests can pin the
 * reference instant instead of depending on wall-clock time. Plain
 * formatting helper, not routed through `t()` — same convention as
 * `formatBytes`/`formatRate`.
 */
const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;

export function relativeTime(iso: string, now: Date = new Date()): string {
  const diffMs = now.getTime() - new Date(iso).getTime();

  if (diffMs < MINUTE_MS) return "agora";

  const minutes = Math.floor(diffMs / MINUTE_MS);
  if (minutes < 60) return `há ${minutes} min`;

  const hours = Math.floor(diffMs / HOUR_MS);
  if (hours < 24) return `há ${hours} h`;

  const days = Math.floor(diffMs / DAY_MS);
  if (days === 1) return "ontem";

  return `há ${days} dias`;
}
