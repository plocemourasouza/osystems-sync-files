/**
 * Badge — 6px status dot + uppercase mono label (DESIGN.md §8 "StatusBadge",
 * status matrix in §2). The dot is never the sole carrier of meaning — the
 * text label always ships alongside it (DESIGN.md §10: "dot + label sempre
 * juntos, nunca só a cor comunicando o estado").
 */
import type { JSX, ReactNode } from "react";
import { cn } from "./cn";

export type BadgeTone = "success" | "warning" | "error" | "neutral" | "info";

export type BadgeProps = {
  tone?: BadgeTone;
  children: ReactNode;
  className?: string;
};

const TONE_CLASSES: Record<BadgeTone, { bg: string; text: string; dot: string }> = {
  success: { bg: "bg-status-success-bg", text: "text-tertiary", dot: "bg-tertiary" },
  warning: { bg: "bg-status-warning-bg", text: "text-secondary", dot: "bg-secondary" },
  error: { bg: "bg-status-error-bg", text: "text-error", dot: "bg-error" },
  info: { bg: "bg-status-info-bg", text: "text-primary", dot: "bg-primary" },
  neutral: { bg: "bg-surface-2", text: "text-text-secondary", dot: "bg-text-quaternary" },
};

export function Badge({ tone = "neutral", children, className }: BadgeProps): JSX.Element {
  const toneClasses = TONE_CLASSES[tone];

  return (
    <span
      className={cn(
        "inline-flex w-fit items-center gap-xs rounded px-xs py-[1px] font-mono text-label-sm uppercase",
        toneClasses.bg,
        toneClasses.text,
        className,
      )}
    >
      <span aria-hidden="true" className={cn("h-1.5 w-1.5 shrink-0 rounded-full", toneClasses.dot)} />
      {children}
    </span>
  );
}
