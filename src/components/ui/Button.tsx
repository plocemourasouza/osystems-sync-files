/**
 * Button — DESIGN.md §8 "Button": 4 variants, 3 heights (24px square `icon`
 * for the JobTable's inline row actions, 28px `sm` for toolbar actions, 32px
 * `md` for footer/modal actions). `loading` swaps
 * the leading `icon` for a spinning `Loader2` and disables the button
 * (network actions like `test_connection`/`save_config` must not be
 * double-fired while in flight).
 */
import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react";
import { Loader2 } from "lucide-react";
import { cn } from "./cn";

export type ButtonVariant = "primary" | "secondary" | "destructive" | "ghost";
export type ButtonSize = "icon" | "sm" | "md";

export type ButtonProps = Omit<ButtonHTMLAttributes<HTMLButtonElement>, "className"> & {
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: ReactNode;
  loading?: boolean;
  className?: string;
};

const VARIANT_CLASSES: Record<ButtonVariant, string> = {
  primary:
    "bg-primary-container text-on-primary border border-[color-mix(in_srgb,var(--color-primary)_30%,transparent)] hover:brightness-110",
  secondary:
    "bg-surface-2 text-secondary border border-[color-mix(in_srgb,var(--color-secondary)_30%,transparent)] hover:bg-surface-hover",
  destructive:
    "bg-surface-2 text-error border border-[color-mix(in_srgb,var(--color-error)_30%,transparent)] hover:bg-surface-hover",
  ghost: "bg-transparent text-text-secondary border border-transparent hover:bg-surface-hover hover:text-text-primary",
};

const SIZE_CLASSES: Record<ButtonSize, string> = {
  // 24px square: 6 of these plus their gaps fit the 40px row's "Ações" column.
  icon: "h-6 w-6 gap-0 p-0 text-label-md",
  sm: "h-7 gap-xs px-sm text-label-md",
  md: "h-8 gap-sm px-md text-body-sm",
};

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "primary", size = "md", icon, loading = false, disabled, children, className, type = "button", ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      type={type}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      className={cn(
        "inline-flex items-center justify-center rounded font-medium transition-colors duration-fast",
        "focus-visible:outline-none focus-visible:shadow-focus",
        "disabled:cursor-not-allowed disabled:opacity-50",
        VARIANT_CLASSES[variant],
        SIZE_CLASSES[size],
        className,
      )}
      {...rest}
    >
      {loading ? <Loader2 aria-hidden="true" size={size === "icon" ? 14 : 16} className="animate-spin" /> : icon}
      {children}
    </button>
  );
});
