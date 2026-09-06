/**
 * DetailsDialog — minimal read-only modal used by `RowActions`'s "Detalhes
 * do erro" menu item (PRD.md RF-065; DESIGN.md §5 "Sombra — só para
 * elementos flutuantes": `shadow-popover` + `surface-2` + `radius-lg`, same
 * treatment as a context menu/Toast).
 *
 * Same a11y shape as `@/components/settings/ConfirmDialog` (`role="dialog"`
 * + `aria-modal` + `aria-labelledby` per RNF-013, focus lands on the action
 * button on open, `Esc`/backdrop click both close, `Tab`/`Shift+Tab` trapped
 * among the dialog's own focusable elements — a11y audit, PLAN.md T-6.2) but
 * with a single "Fechar" action instead of confirm/cancel — this is
 * presentation-only, there is nothing to confirm. Kept dashboard-local (not
 * imported from `components/settings/`) since that directory is not part of
 * the dashboard's dependency surface.
 */
import { useEffect, useId, useRef, type JSX, type MouseEvent, type ReactNode } from "react";

import { Button } from "@/components/ui";
import { t } from "@/i18n";

export type DetailsDialogProps = {
  open: boolean;
  title: string;
  children?: ReactNode;
  onClose: () => void;
};

export function DetailsDialog({ open, title, children, onClose }: DetailsDialogProps): JSX.Element | null {
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;

    closeRef.current?.focus();

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
        onClose();
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
  }, [open, onClose]);

  if (!open) return null;

  function handleBackdropClick(event: MouseEvent<HTMLDivElement>): void {
    if (event.target === event.currentTarget) {
      onClose();
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
        className="flex w-full max-w-md flex-col gap-md rounded-lg border border-border-hairline bg-surface-2 p-lg shadow-popover"
      >
        <h2 id={titleId} className="text-headline-sm text-text-primary">
          {title}
        </h2>
        {children && <div className="flex flex-col gap-sm">{children}</div>}
        <div className="flex justify-end">
          <Button ref={closeRef} variant="secondary" size="sm" onClick={onClose}>
            {t("pages.dashboard.table.errorDialog.close")}
          </Button>
        </div>
      </div>
    </div>
  );
}
