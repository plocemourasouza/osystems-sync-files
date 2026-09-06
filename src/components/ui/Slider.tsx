/**
 * Slider — QoS bandwidth control (DESIGN.md §8 "Slider (QoS)", RF-050).
 *
 * Semantic range is 0.5–10.0 MB/s in 0.5 steps (19 numeric positions), plus
 * one extra rightmost position meaning "Ilimitado" (`value === null`,
 * `set_qos`'s `limit_mbps: null`). The underlying `<input type="range">`
 * moves over a plain integer index (0..20); `value`/`onChange` at the
 * component boundary stay in the real domain (`number | null`) so callers
 * never deal with the index encoding.
 *
 * `accent` picks the destination's brand color (Google Drive = primary, AWS
 * S3 = secondary) for the value badge, dot, and native range accent color —
 * DESIGN.md's "dois sliders visualmente distintos por destino".
 */
import { forwardRef, useId, type JSX } from "react";
import { cn } from "./cn";
import { t } from "../../i18n";

export type SliderAccent = "primary" | "secondary";

export type SliderProps = {
  id?: string;
  label: string;
  value: number | null;
  onChange: (value: number | null) => void;
  accent?: SliderAccent;
  disabled?: boolean;
  className?: string;
};

const MIN = 0.5;
const MAX = 10;
const STEP = 0.5;
const NUMERIC_STEPS = Math.round((MAX - MIN) / STEP);
const UNLIMITED_INDEX = NUMERIC_STEPS + 1;
const TICKS = [0.5, 2.5, 5, 10];

function valueToIndex(value: number | null): number {
  if (value === null) return UNLIMITED_INDEX;
  const clamped = Math.min(MAX, Math.max(MIN, value));
  return Math.round((clamped - MIN) / STEP);
}

function indexToValue(index: number): number | null {
  if (index >= UNLIMITED_INDEX) return null;
  return Number((MIN + index * STEP).toFixed(1));
}

function formatValue(value: number | null, unlimitedLabel: string): string {
  return value === null ? unlimitedLabel : `${value.toFixed(1)} MB/s`;
}

export const Slider = forwardRef<HTMLInputElement, SliderProps>(function Slider(
  { id, label, value, onChange, accent = "primary", disabled, className },
  ref,
): JSX.Element {
  const generatedId = useId();
  const sliderId = id ?? generatedId;
  const unlimitedLabel = t("common.status.unlimited");
  const displayValue = formatValue(value, unlimitedLabel);
  const index = valueToIndex(value);

  const accentText = accent === "primary" ? "text-primary" : "text-secondary";
  const accentDot = accent === "primary" ? "bg-primary" : "bg-secondary";
  const accentRange = accent === "primary" ? "accent-primary-strong" : "accent-secondary-strong";

  return (
    <div className={cn("flex flex-col gap-xs", className)}>
      <div className="flex items-center justify-between gap-sm">
        <label htmlFor={sliderId} className="flex items-center gap-xs text-body-sm text-text-secondary">
          <span aria-hidden="true" className={cn("h-1.5 w-1.5 rounded-full", accentDot)} />
          {label}
        </label>
        <span
          className={cn(
            "rounded bg-surface-0 px-xs py-[1px] font-mono text-label-md tabular-nums",
            accentText,
          )}
        >
          {displayValue}
        </span>
      </div>
      <input
        ref={ref}
        id={sliderId}
        type="range"
        min={0}
        max={UNLIMITED_INDEX}
        step={1}
        value={index}
        disabled={disabled}
        aria-valuetext={displayValue}
        onChange={(event) => onChange(indexToValue(Number(event.target.value)))}
        className={cn(
          "h-1 w-full cursor-pointer appearance-none rounded-full bg-surface-hover",
          accentRange,
          disabled ? "cursor-not-allowed opacity-50" : undefined,
        )}
      />
      <div className="flex justify-between font-mono text-label-sm text-text-quaternary">
        {TICKS.map((tick) => (
          <span key={tick}>{tick.toFixed(1)} MB/s</span>
        ))}
        <span>{unlimitedLabel}</span>
      </div>
    </div>
  );
});
