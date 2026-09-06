/**
 * ErrorText — inline error message (DESIGN.md §8 "ErrorText", §10 mitigation
 * for the `--color-error`/`--color-surface-2` contrast failure below AA).
 *
 * A full sentence colored `text-error` fails WCAG AA (4,09:1 on
 * `--color-surface-2`, §10) — `--color-error` is reserved for a short
 * label + dot pairing (`Badge`) or an icon, never body text. `ErrorText`
 * carries the error tone entirely through a 14px `AlertCircle` icon
 * (WCAG 1.4.11 non-text floor, 3,0:1, which `--color-error` passes) while
 * the message itself renders in `--color-text-primary` (AA-safe on every
 * surface, §10).
 *
 * `role="alert"` is kept so screen readers announce the message without
 * the caller needing to manage `aria-live` — the same contract every
 * pre-existing error paragraph in this codebase relies on. Pass `id` to
 * wire `aria-describedby` from the associated field.
 */
import type { JSX, ReactNode } from "react";
import { AlertCircle } from "lucide-react";
import { cn } from "./cn";

export type ErrorTextProps = {
  children: ReactNode;
  id?: string;
  className?: string;
};

export function ErrorText({ children, id, className }: ErrorTextProps): JSX.Element {
  return (
    <p id={id} role="alert" className={cn("flex items-start gap-xs text-body-sm text-text-primary", className)}>
      <AlertCircle aria-hidden="true" size={14} className="mt-[3px] shrink-0 text-error" />
      {children}
    </p>
  );
}
