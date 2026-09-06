/**
 * NumberField — Field wired for numeric input with min/max/step clamping.
 *
 * Keeps an internal string "draft" so a user can type transitional values
 * (empty string, a lone "-") without the field fighting them; `onChange`
 * still fires with a real `number` on every keystroke that parses cleanly.
 * On blur the draft is clamped into `[min, max]` and re-emitted — this is
 * the single source of truth restoring the field to a valid state, per
 * DESIGN.md §8 (Slider's numeric siblings in `/settings` — QoS overrides,
 * ports, etc.). While the draft is out of range (before blur clamps it) the
 * field shows a live error via `Field`'s `error` slot, unless the caller
 * passed its own `error` (which always wins).
 */
import { ChevronDown, ChevronUp } from "lucide-react";
import { forwardRef, useEffect, useState, type ChangeEvent, type FocusEvent } from "react";
import { Field, type FieldProps } from "./Field";
import { t } from "../../i18n";

export type NumberFieldProps = Omit<FieldProps, "type" | "value" | "onChange" | "trailing"> & {
  value: number;
  onChange: (value: number) => void;
  min?: number;
  max?: number;
  step?: number;
};

export const NumberField = forwardRef<HTMLInputElement, NumberFieldProps>(function NumberField(
  { value, onChange, min, max, step = 1, error, onBlur, disabled, ...rest },
  ref,
) {
  const [draft, setDraft] = useState(String(value));

  useEffect(() => {
    setDraft(String(value));
  }, [value]);

  const draftNumber = Number(draft);
  const hasValidDraft = draft.trim() !== "" && !Number.isNaN(draftNumber);
  const isOutOfRange =
    hasValidDraft && ((min !== undefined && draftNumber < min) || (max !== undefined && draftNumber > max));
  const rangeError = isOutOfRange
    ? t("common.valueOutOfRange", { min: min ?? "-∞", max: max ?? "∞" })
    : undefined;

  function handleChange(event: ChangeEvent<HTMLInputElement>): void {
    const raw = event.target.value;
    setDraft(raw);
    const parsed = Number(raw);
    if (raw.trim() !== "" && !Number.isNaN(parsed)) {
      onChange(parsed);
    }
  }

  function handleBlur(event: FocusEvent<HTMLInputElement>): void {
    let next = Number(draft);
    if (draft.trim() === "" || Number.isNaN(next)) {
      next = value;
    }
    if (min !== undefined) next = Math.max(min, next);
    if (max !== undefined) next = Math.min(max, next);
    setDraft(String(next));
    onChange(next);
    onBlur?.(event);
  }

  /** Steps by `step`, clamped — the same arithmetic the native spinner did. */
  function nudge(direction: 1 | -1): void {
    const from = hasValidDraft ? draftNumber : value;
    let next = from + direction * step;
    if (min !== undefined) next = Math.max(min, next);
    if (max !== undefined) next = Math.min(max, next);
    setDraft(String(next));
    onChange(next);
  }

  const atMax = max !== undefined && (hasValidDraft ? draftNumber : value) >= max;
  const atMin = min !== undefined && (hasValidDraft ? draftNumber : value) <= min;

  /*
   * The native spinner is hidden in `app.css` and replaced here, so the
   * stepper is the same `ChevronDown` at the same `text-text-quaternary` as
   * `Select`'s — the engine's own arrows rendered black on Windows and only
   * on hover on macOS, matching neither each other nor the Select beside them.
   *
   * `aria-hidden` + `tabIndex={-1}`: these are a pointer affordance for a
   * function the input already exposes to the keyboard (↑/↓ still step the
   * value natively) and to assistive tech (it is a spinbutton with min/max).
   * Exposing them would add two tab stops per field announcing nothing new.
   */
  const stepper = (
    <span aria-hidden="true" className="flex flex-col justify-center">
      {(
        [
          [1, ChevronUp, atMax],
          [-1, ChevronDown, atMin],
        ] as const
      ).map(([direction, Icon, atBound]) => (
        <button
          key={direction}
          type="button"
          tabIndex={-1}
          disabled={disabled || atBound}
          onClick={() => nudge(direction)}
          className="flex h-3 w-4 items-center justify-center rounded text-text-quaternary transition-colors duration-fast hover:text-text-primary disabled:pointer-events-none disabled:opacity-30"
        >
          <Icon size={12} />
        </button>
      ))}
    </span>
  );

  return (
    <Field
      ref={ref}
      type="number"
      value={draft}
      onChange={handleChange}
      onBlur={handleBlur}
      error={error ?? rangeError}
      min={min}
      max={max}
      step={step}
      disabled={disabled}
      trailing={stepper}
      {...rest}
    />
  );
});
