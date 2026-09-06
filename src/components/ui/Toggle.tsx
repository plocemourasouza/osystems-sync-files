/**
 * Toggle — `role="switch"` (DESIGN.md §8 "Toggle"), 28×16px track,
 * `--color-primary-container` fill when on. Implemented as a native
 * `<button>` so Space/Enter activation is free (browsers fire `click` for
 * both on a focused `<button>`) — no manual `onKeyDown` needed.
 */
import { useId, type JSX } from "react";
import { cn } from "./cn";

export type ToggleProps = {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  description?: string;
  disabled?: boolean;
  id?: string;
  className?: string;
};

export function Toggle({
  checked,
  onChange,
  label,
  description,
  disabled,
  id,
  className,
}: ToggleProps): JSX.Element {
  const generatedId = useId();
  const switchId = id ?? generatedId;
  const labelId = `${switchId}-label`;

  return (
    <div className={cn("flex items-center justify-between gap-md", className)}>
      <div className="flex flex-col">
        <span id={labelId} className="text-body-sm text-text-primary">
          {label}
        </span>
        {description && <span className="text-label-sm text-text-tertiary">{description}</span>}
      </div>
      <button
        type="button"
        id={switchId}
        role="switch"
        aria-checked={checked}
        aria-labelledby={labelId}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={cn(
          "relative h-4 w-7 shrink-0 rounded-full transition-colors duration-fast",
          "focus-visible:outline-none focus-visible:shadow-focus",
          checked ? "bg-primary-container" : "bg-surface-hover",
          disabled ? "cursor-not-allowed opacity-50" : undefined,
        )}
      >
        <span
          aria-hidden="true"
          className={cn(
            // `left-0` is load-bearing: without an inset the thumb falls back
            // to its static position, which a <button> centers -- the offsets
            // below would then start from the middle of the track and push the
            // thumb outside it. Anchored left, 2px/14px is the symmetric 2px
            // inset and the 12px travel DESIGN.md §8 specifies.
            "absolute left-0 top-1/2 h-3 w-3 -translate-y-1/2 rounded-full transition-transform duration-fast",
            checked ? "translate-x-[14px] bg-text-primary" : "translate-x-[2px] bg-text-secondary",
          )}
        />
      </button>
    </div>
  );
}
