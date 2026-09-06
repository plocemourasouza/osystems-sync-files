/**
 * Pagination — `JobTable`'s "page X of Y" footer control (PRD.md RF-064:
 * 50 rows/page, server-side). Purely presentational: `JobTable` owns
 * `jobsStore.page`/`setPage`.
 */
import { ChevronLeft, ChevronRight } from "lucide-react";
import type { JSX } from "react";

import { Button } from "@/components/ui";
import { t } from "@/i18n";

export type PaginationProps = {
  /** 0-based, mirrors `jobsStore.page`. */
  page: number;
  pageSize: number;
  total: number;
  onPrev: () => void;
  onNext: () => void;
};

export function Pagination({ page, pageSize, total, onPrev, onNext }: PaginationProps): JSX.Element {
  const totalPages = Math.max(1, Math.ceil(total / pageSize));
  const currentPage = page + 1;

  return (
    <nav
      aria-label={t("pages.dashboard.table.pagination.navLabel")}
      className="flex shrink-0 items-center justify-end gap-sm border-t border-border-hairline px-md py-sm"
    >
      <Button
        variant="ghost"
        size="sm"
        aria-label={t("pages.dashboard.table.pagination.prev")}
        icon={<ChevronLeft aria-hidden="true" size={16} />}
        disabled={currentPage <= 1}
        onClick={onPrev}
      />
      <span className="font-mono text-label-sm text-text-secondary">
        {t("pages.dashboard.table.pagination.pageLabel", { page: currentPage, total: totalPages })}
      </span>
      <Button
        variant="ghost"
        size="sm"
        aria-label={t("pages.dashboard.table.pagination.next")}
        icon={<ChevronRight aria-hidden="true" size={16} />}
        disabled={currentPage >= totalPages}
        onClick={onNext}
      />
    </nav>
  );
}
