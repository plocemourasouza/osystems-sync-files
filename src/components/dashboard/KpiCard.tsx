/**
 * KpiCard — Module-like compact card for the Dashboard's 5-KPI row
 * (DESIGN.md §8 "KpiCard"; PRD.md RF-060). Anatomy: 28px header row
 * (label mono uppercase + icon top-right), big `headline-lg` value, and an
 * optional bottom-right sub slot (Badge or mono text). Tones map to the
 * semantic status tokens; `aria-live="polite"` on the value per DESIGN.md §10
 * so screen readers announce KPI updates without re-reading the whole card.
 */
import type { JSX, ReactNode } from "react";
import type { LucideIcon } from "lucide-react";
import { cn } from "@/components/ui/cn";

export type KpiTone = "neutral" | "success" | "info" | "warning" | "error";

export type KpiCardProps = {
  label: string;
  value: string | number;
  sub?: ReactNode;
  tone?: KpiTone;
  icon: LucideIcon;
  loading?: boolean;
};

const TONE_CLASSES: Record<KpiTone, { border: string; icon: string }> = {
  neutral: { border: "border-border-hairline", icon: "text-text-secondary" },
  success: {
    border: "border-[color-mix(in_srgb,var(--color-tertiary)_30%,transparent)]",
    icon: "text-tertiary",
  },
  info: {
    border: "border-[color-mix(in_srgb,var(--color-primary)_30%,transparent)]",
    icon: "text-primary",
  },
  warning: {
    border: "border-[color-mix(in_srgb,var(--color-secondary)_30%,transparent)]",
    icon: "text-secondary",
  },
  error: {
    border: "border-[color-mix(in_srgb,var(--color-error)_30%,transparent)]",
    icon: "text-error",
  },
};

export function KpiCard({ label, value, sub, tone = "neutral", icon: Icon, loading = false }: KpiCardProps): JSX.Element {
  const toneClasses = TONE_CLASSES[tone];

  if (loading) {
    return (
      <div
        className={cn("flex flex-col gap-sm rounded-md border bg-surface-2 p-md", toneClasses.border)}
        aria-hidden="true"
      >
        <div className="flex h-[28px] items-center justify-between">
          <div className="h-3 w-20 animate-pulse rounded-sm bg-surface-hover" />
          <div className="h-4 w-4 animate-pulse rounded-sm bg-surface-hover" />
        </div>
        <div className="h-6 w-16 animate-pulse rounded-sm bg-surface-hover" />
      </div>
    );
  }

  return (
    <div className={cn("flex flex-col gap-sm rounded-md border bg-surface-2 p-md", toneClasses.border)}>
      <div className="flex h-[28px] items-center justify-between gap-xs">
        <span className="truncate font-mono text-label-sm uppercase text-text-secondary">{label}</span>
        <Icon aria-hidden="true" className={cn("h-4 w-4 shrink-0", toneClasses.icon)} />
      </div>
      <div className="flex items-end justify-between gap-xs">
        <span aria-live="polite" className="text-headline-lg tabular-nums text-text-primary">
          {value}
        </span>
        {sub && <div className="shrink-0">{sub}</div>}
      </div>
    </div>
  );
}
