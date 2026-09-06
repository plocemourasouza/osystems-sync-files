/**
 * Module — surface-2 card with header (icon + title/subtitle + status slot),
 * a divided body, and an optional footer (DESIGN.md §8 "Card / Module").
 * Body children are separated by a hairline `border-top` rule instead of
 * gap spacing — DESIGN.md: "reduz a distância ocular entre campos
 * relacionados" — via `divide-y` on the body wrapper.
 *
 * `title` renders as an `<h2>` (RNF-013 — a11y audit, PLAN.md T-6.2): every
 * `Settings` module sits directly under the page's own `<h1
 * id="settings-title">`, so an `<h3>` here skipped a level (axe-core
 * `heading-order`). `text-headline-sm` fully controls the visual size, so
 * the tag change has no visible effect.
 */
import type { JSX, ReactNode } from "react";
import { cn } from "./cn";

export type ModuleProps = {
  icon?: ReactNode;
  title: string;
  subtitle?: string;
  status?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  className?: string;
};

export function Module({ icon, title, subtitle, status, children, footer, className }: ModuleProps): JSX.Element {
  return (
    <section className={cn("flex flex-col rounded-md border border-border-hairline bg-surface-2", className)}>
      <header className="flex flex-wrap items-center justify-between gap-sm border-b border-border-hairline px-md py-sm">
        <div className="flex min-w-0 items-center gap-sm">
          {icon && (
            <span aria-hidden="true" className="text-text-secondary">
              {icon}
            </span>
          )}
          <div className="flex min-w-0 flex-col">
            <h2 className="text-headline-sm text-text-primary">{title}</h2>
            {subtitle && <p className="text-body-sm text-text-secondary">{subtitle}</p>}
          </div>
        </div>
        {status && <div className="max-w-full shrink-0">{status}</div>}
      </header>
      <div className="flex flex-col divide-y divide-border-hairline">{children}</div>
      {footer && <footer className="border-t border-border-hairline px-md py-sm">{footer}</footer>}
    </section>
  );
}
