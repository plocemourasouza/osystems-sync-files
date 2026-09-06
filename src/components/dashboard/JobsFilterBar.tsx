/**
 * JobsFilterBar — the "Todos / Ativos / Concluídos / Falhas" segmented
 * control above `JobTable` (PRD.md RF-064; DESIGN.md §9 `/dashboard`).
 * Purely presentational: `JobTable` owns `jobsStore.filter`/`setFilter`.
 */
import type { JSX } from "react";

import { cn } from "@/components/ui";
import { t, type MessageKey } from "@/i18n";
import type { JobsFilter } from "@/store/jobsStore";

const FILTER_OPTIONS: { value: JobsFilter; labelKey: MessageKey }[] = [
  { value: "all", labelKey: "pages.dashboard.filters.all" },
  { value: "active", labelKey: "pages.dashboard.filters.active" },
  { value: "done", labelKey: "pages.dashboard.filters.done" },
  { value: "failed", labelKey: "pages.dashboard.filters.failed" },
];

export type JobsFilterBarProps = {
  filter: JobsFilter;
  onChange: (filter: JobsFilter) => void;
};

export function JobsFilterBar({ filter, onChange }: JobsFilterBarProps): JSX.Element {
  return (
    <div
      role="group"
      aria-label={t("pages.dashboard.filters.groupLabel")}
      className="inline-flex items-center gap-2xs rounded-md border border-border-hairline bg-surface-1 p-2xs"
    >
      {FILTER_OPTIONS.map((option) => {
        const active = option.value === filter;
        return (
          <button
            key={option.value}
            type="button"
            aria-pressed={active}
            onClick={() => onChange(option.value)}
            className={cn(
              "rounded px-sm py-[3px] text-label-md transition-colors duration-fast",
              active ? "bg-surface-3 text-text-primary" : "text-text-secondary hover:bg-surface-hover hover:text-text-primary",
            )}
          >
            {t(option.labelKey)}
          </button>
        );
      })}
    </div>
  );
}
