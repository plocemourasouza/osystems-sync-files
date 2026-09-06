/**
 * ConfirmDialog — minimal accessible confirmation modal, no external deps
 * (DESIGN.md §5 "Sombra — só para elementos flutuantes": `shadow-popover` +
 * `surface-2` + `radius-lg`, same treatment as a context menu/Toast).
 *
 * Used by `SettingsFooter`'s "Restaurar padrões" (PRD.md RF-084: "diálogo de
 * confirmação"). `role="dialog"` + `aria-modal` + `aria-labelledby` per
 * RNF-013; on open, focus lands on the Cancel button — the safe action —
 * so an accidental `Enter` never confirms. `Esc` and a backdrop click both
 * cancel. `Tab`/`Shift+Tab` are trapped among the dialog's own focusable
 * elements (RNF-013 "navegação por teclado" — a11y audit, PLAN.md T-6.2):
 * without this, Tab from the last button would leave the dialog and land on
 * page content hidden behind the backdrop.
 */
import { useEffect, useId, useRef, type JSX, type MouseEvent, type ReactNode } from "react";
import { Button } from "@/components/ui";

export type ConfirmDialogProps = {
  open: boolean;
  title: string;
  children?: ReactNode;
  confirmLabel: string;
  cancelLabel: string;
  onConfirm: () => void;
  onCancel: () => void;
};

export function ConfirmDialog({
  open,
  title,
  children,
  confirmLabel,
  cancelLabel,
  onConfirm,
  onCancel,
}: ConfirmDialogProps): JSX.Element | null {
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;

    cancelRef.current?.focus();

    function focusableElements(): HTMLElement[] {
      if (!dialogRef.current) return [];
      return Array.from(
        dialogRef.current.querySelectorAll<HTMLElement>(
          'button:not(:disabled), [href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])'
        )
      );
    }

    function handleKeyDown(event: KeyboardEvent): void {
      if (event.key === "Escape") {
        event.preventDefault();
        onCancel();
        return;
      }

      if (event.key === "Tab") {
        const elements = focusableElements();
        const first = elements[0];
        const last = elements[elements.length - 1];
        if (!first || !last) return;

        const active = document.activeElement;
        const inside = dialogRef.current?.contains(active) ?? false;

        if (event.shiftKey) {
          if (!inside || active === first) {
            event.preventDefault();
            last.focus();
          }
        } else {
          if (!inside || active === last) {
            event.preventDefault();
            first.focus();
          }
        }
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [open, onCancel]);

  if (!open) return null;

  function handleBackdropClick(event: MouseEvent<HTMLDivElement>): void {
    if (event.target === event.currentTarget) {
      onCancel();
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-[color-mix(in_srgb,var(--color-surface-0)_75%,transparent)] px-md"
      onClick={handleBackdropClick}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="flex w-full max-w-sm flex-col gap-md rounded-lg border border-border-hairline bg-surface-2 p-lg shadow-popover"
      >
        <h2 id={titleId} className="text-headline-sm text-text-primary">
          {title}
        </h2>
        {children && <div className="text-body-sm text-text-secondary">{children}</div>}
        <div className="flex justify-end gap-sm">
          <Button ref={cancelRef} variant="secondary" size="sm" onClick={onCancel}>
            {cancelLabel}
          </Button>
          <Button variant="primary" size="sm" onClick={onConfirm}>
            {confirmLabel}
          </Button>
        </div>
      </div>
    </div>
  );
}
