/**
 * PasswordField — Field + eye toggle (DESIGN.md §8 "Field — Password com olho").
 *
 * The eye only ever reveals what the user is typing in the current session;
 * per PRD.md §3, a secret already persisted in the OS keyring is never
 * redecrypted for plaintext display — enforcing that is the caller's job
 * (it controls `value`), this component only toggles the input's `type`.
 *
 * `revealDisabled` (T-3.10, RF-020/RF-085) disables just the eye button —
 * used for an empty draft (nothing typed yet to reveal) and for a value
 * already saved to the keyring (masked display, where there is nothing to
 * reveal because the backend never returns the real secret). While
 * disabled, the field is forced back to `type="password"` regardless of
 * whatever the internal toggle state was before it became disabled.
 */
import { forwardRef, useState } from "react";
import { Eye, EyeOff } from "lucide-react";
import { cn } from "./cn";
import { Field, type FieldProps } from "./Field";
import { t } from "../../i18n";

export type PasswordFieldProps = Omit<FieldProps, "type" | "trailing"> & {
  revealDisabled?: boolean;
};

export const PasswordField = forwardRef<HTMLInputElement, PasswordFieldProps>(function PasswordField(
  { revealDisabled = false, ...props },
  ref,
) {
  const [visible, setVisible] = useState(false);
  const effectiveVisible = revealDisabled ? false : visible;

  return (
    <Field
      ref={ref}
      {...props}
      type={effectiveVisible ? "text" : "password"}
      trailing={
        <button
          type="button"
          disabled={revealDisabled}
          aria-pressed={effectiveVisible}
          aria-label={effectiveVisible ? t("common.hide") : t("common.show")}
          onClick={() => setVisible((current) => !current)}
          className={cn(
            "flex h-4 w-4 items-center justify-center text-text-secondary transition-colors duration-fast hover:text-text-primary focus-visible:outline-none focus-visible:shadow-focus",
            revealDisabled && "cursor-not-allowed opacity-40 hover:text-text-secondary",
          )}
        >
          {effectiveVisible ? <EyeOff aria-hidden="true" size={16} /> : <Eye aria-hidden="true" size={16} />}
        </button>
      }
    />
  );
});
