/**
 * Checkbox — 14×14px custom check (DESIGN.md §8 "Checkbox"): a real
 * `<input type="checkbox">` (native semantics, keyboard support) visually
 * hidden under a styled box, with a `Check` icon shown via the `peer`
 * variant when checked.
 */
import { forwardRef, useId, type InputHTMLAttributes } from "react";
import { Check } from "lucide-react";
import { cn } from "./cn";

export type CheckboxProps = Omit<
  InputHTMLAttributes<HTMLInputElement>,
  "id" | "className" | "type" | "onChange" | "checked"
> & {
  id?: string;
  label: string;
  description?: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  className?: string;
};

export const Checkbox = forwardRef<HTMLInputElement, CheckboxProps>(function Checkbox(
  { id, label, description, checked, onChange, disabled, className, ...rest },
  ref,
) {
  const generatedId = useId();
  const checkboxId = id ?? generatedId;

  return (
    <label
      htmlFor={checkboxId}
      className={cn(
        "flex items-start gap-sm",
        disabled ? "cursor-not-allowed opacity-50" : "cursor-pointer",
        className,
      )}
    >
      <span className="relative mt-[1px] flex h-[14px] w-[14px] shrink-0 items-center justify-center rounded-sm border border-border-strong">
        <input
          ref={ref}
          id={checkboxId}
          type="checkbox"
          checked={checked}
          disabled={disabled}
          onChange={(event) => onChange(event.target.checked)}
          className="peer absolute inset-0 m-0 h-full w-full cursor-pointer appearance-none focus-visible:outline-none"
          {...rest}
        />
        <span className="pointer-events-none absolute inset-0 rounded-sm peer-checked:bg-primary peer-focus-visible:shadow-focus" />
        <Check
          aria-hidden="true"
          size={10}
          className="pointer-events-none relative hidden text-on-primary peer-checked:block"
        />
      </span>
      <span className="flex flex-col">
        <span className="text-body-sm text-text-primary">{label}</span>
        {description && <span className="text-label-sm text-text-tertiary">{description}</span>}
      </span>
    </label>
  );
});
