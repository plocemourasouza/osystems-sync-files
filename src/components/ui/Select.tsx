/**
 * Select — native `<select>` styled per DESIGN.md §8 ("Field — Select"):
 * same 30px height/tokens as `Field`, `appearance: none` with a fixed
 * `ChevronDown` (16px) on the right.
 */
import { forwardRef, useId, type SelectHTMLAttributes } from "react";
import { ChevronDown } from "lucide-react";
import { cn } from "./cn";
import { ErrorText } from "./ErrorText";

export type SelectOption = {
  value: string;
  label: string;
};

export type SelectProps = Omit<SelectHTMLAttributes<HTMLSelectElement>, "id" | "className" | "onChange"> & {
  id?: string;
  label: string;
  options: SelectOption[];
  value: string;
  onChange: (value: string) => void;
  hint?: string;
  error?: string;
  className?: string;
  /** Keeps `label` as the accessible name (via `htmlFor`) but hides it visually (`sr-only`). */
  hideLabel?: boolean;
};

export const Select = forwardRef<HTMLSelectElement, SelectProps>(function Select(
  { id, label, options, value, onChange, hint, error, className, hideLabel, disabled, ...rest },
  ref,
) {
  const generatedId = useId();
  const selectId = id ?? generatedId;
  const hintId = hint ? `${selectId}-hint` : undefined;
  const errorId = error ? `${selectId}-error` : undefined;
  const describedBy = [hintId, errorId].filter(Boolean).join(" ") || undefined;

  return (
    <div className={cn("flex flex-col gap-xs", className)}>
      <label
        htmlFor={selectId}
        className={hideLabel ? "sr-only" : "text-body-sm text-text-secondary"}
      >
        {label}
      </label>
      <div className="relative flex items-center">
        <select
          ref={ref}
          id={selectId}
          value={value}
          disabled={disabled}
          aria-invalid={error ? true : undefined}
          aria-describedby={describedBy}
          onChange={(event) => onChange(event.target.value)}
          className={cn(
            "h-[30px] w-full appearance-none rounded border border-border-hairline bg-surface-0 pl-sm pr-2xl text-body-sm text-text-primary transition-colors duration-fast",
            "focus-visible:outline-none focus-visible:border-border-strong focus-visible:shadow-focus",
            error ? "border-error" : undefined,
            disabled ? "cursor-not-allowed opacity-50" : undefined,
          )}
          {...rest}
        >
          {options.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
        <ChevronDown
          aria-hidden="true"
          size={16}
          className="pointer-events-none absolute right-sm text-text-quaternary"
        />
      </div>
      {hint && !error && (
        <p id={hintId} className="text-label-sm text-text-quaternary">
          {hint}
        </p>
      )}
      {error && <ErrorText id={errorId}>{error}</ErrorText>}
    </div>
  );
});
