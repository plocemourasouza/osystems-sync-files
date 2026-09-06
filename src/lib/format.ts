/**
 * format — byte/percentage formatting helpers for KPI display (T-2.8).
 * Decimal (SI, 1000-based) units, matching Sidebar.tsx's `formatRate` convention.
 */

const UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

/**
 * Formats a byte count as a human-readable decimal string (e.g. "6.42 GB",
 * "420.0 MB", "12.4 KB", "0 B"). One decimal place from MB up; two from GB up.
 */
export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return "0 B";

  let value = n;
  let unitIndex = 0;
  while (value >= 1000 && unitIndex < UNITS.length - 1) {
    value /= 1000;
    unitIndex += 1;
  }

  if (unitIndex === 0) return `${Math.round(value)} ${UNITS[unitIndex]}`;

  const decimals = unitIndex >= 3 ? 2 : 1;
  return `${value.toFixed(decimals)} ${UNITS[unitIndex]}`;
}

/**
 * Formats a 0..1 ratio as a whole-number percentage string (e.g. 0.57 → "57%").
 */
export function formatPct(ratio: number): string {
  if (!Number.isFinite(ratio) || ratio <= 0) return "0%";
  const clamped = Math.min(ratio, 1);
  return `${Math.round(clamped * 100)}%`;
}
