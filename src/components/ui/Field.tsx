/**
 * Field — labeled text/number/password input (DESIGN.md §8 "Field").
 *
 * Base primitive consumed directly for plain text inputs, and composed by
 * `PasswordField` (eye toggle in `trailing`) and `NumberField` (draft/clamp
 * logic on top). 30px height, `mono-data` typography opt-in via `mono` (most
 * `/settings` fields are technical — path, id, key — but the prop keeps
 * plain-text fields, like a display name, from getting the mono treatment).
 *
 * A11y (DESIGN.md §10): `<label for>` associated via `useId()` fallback,
 * `aria-invalid` + `aria-describedby` pointing at the hint/error paragraph,
 * error text always rendered with `role="alert"` so it is announced.
 */
import { forwardRef, useId, type InputHTMLAttributes, type ReactNode } from "react";
import { cn } from "./cn";
import { ErrorText } from "./ErrorText";

export type FieldProps = Omit<InputHTMLAttributes<HTMLInputElement>, "id" | "className" | "type"> & {
  id?: string;
  label: string;
  type?: "text" | "number" | "password";
  hint?: string;
  error?: string;
  mono?: boolean;
  trailing?: ReactNode;
  className?: string;
};

export const Field = forwardRef<HTMLInputElement, FieldProps>(function Field(
  { id, label, type = "text", hint, error, mono = false, trailing, className, disabled, readOnly, ...rest },
  ref,
) {
  const generatedId = useId();
  const inputId = id ?? generatedId;
  const hintId = hint ? `${inputId}-hint` : undefined;
  const errorId = error ? `${inputId}-error` : undefined;
  const describedBy = [hintId, errorId].filter(Boolean).join(" ") || undefined;

  return (
    <div className={cn("flex flex-col gap-xs", className)}>
      <label htmlFor={inputId} className="text-body-sm text-text-secondary">
        {label}
      </label>
      <div className="relative flex items-center">
        <input
          ref={ref}
          id={inputId}
          type={type}
          disabled={disabled}
          readOnly={readOnly}
          aria-invalid={error ? true : undefined}
          aria-describedby={describedBy}
          className={cn(
            "h-[30px] w-full rounded border border-border-hairline bg-surface-0 px-sm text-text-primary placeholder:text-text-quaternary transition-colors duration-fast",
            mono ? "font-mono text-mono-data" : "text-body-sm",
            "focus-visible:outline-none focus-visible:border-border-strong focus-visible:shadow-focus",
            trailing ? "pr-2xl" : undefined,
            error ? "border-error" : undefined,
            disabled ? "cursor-not-allowed opacity-50" : undefined,
          )}
          {...rest}
        />
        {trailing && <div className="absolute right-xs flex items-center">{trailing}</div>}
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
