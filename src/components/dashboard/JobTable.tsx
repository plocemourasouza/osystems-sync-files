/**
 * JobTable — the Dashboard's dual sync queue (PRD.md RF-062–RF-065;
 * DESIGN.md §8 "JobTable"/"JobRow", §9 `/dashboard` map).
 *
 * Self-contained: fetches `jobsStore` on mount and on every filter/page
 * change, and wires the two live IPC events (`job-updated`, `upload-progress`)
 * straight into the store. `Dashboard.tsx` (T-2.12) just mounts this.
 */
import type { JSX } from "react";
import { useEffect } from "react";

import { useTauriEvent } from "@/api/events";
import { t } from "@/i18n";
import { ErrorText } from "@/components/ui";
import { useJobsStore } from "@/store/jobsStore";

import { JobRow } from "./JobRow";
import { JobsFilterBar } from "./JobsFilterBar";
import { Pagination } from "./Pagination";
import { WatcherStatusBadge } from "./WatcherStatusBadge";

/** DOM rows rendered while `jobsStore.loading` is true. */
const SKELETON_ROW_COUNT = 5;

function SkeletonRow(): JSX.Element {
  return (
    <tr className="h-10 border-b border-border-hairline">
      <td colSpan={6} className="px-md py-0">
        <div className="h-4 w-full max-w-md animate-pulse rounded bg-surface-2" />
      </td>
    </tr>
  );
}

function HeaderDot({ className }: { className: string }): JSX.Element {
  return <span aria-hidden="true" className={`h-1.5 w-1.5 shrink-0 rounded-full ${className}`} />;
}

export function JobTable(): JSX.Element {
  const filter = useJobsStore((s) => s.filter);
  const page = useJobsStore((s) => s.page);
  const pageSize = useJobsStore((s) => s.pageSize);
  const items = useJobsStore((s) => s.items);
  const total = useJobsStore((s) => s.total);
  const loading = useJobsStore((s) => s.loading);
  const error = useJobsStore((s) => s.error);
  const progress = useJobsStore((s) => s.progress);
  const setFilter = useJobsStore((s) => s.setFilter);
  const setPage = useJobsStore((s) => s.setPage);
  const fetchJobs = useJobsStore((s) => s.fetch);
  const upsertFromEvent = useJobsStore((s) => s.upsertFromEvent);
  const applyProgress = useJobsStore((s) => s.applyProgress);

  // Store actions are stable references (defined once in the `create()`
  // closure), so including `fetchJobs` here doesn't cause a refetch loop —
  // this effect only re-runs when `filter` or `page` actually change.
  useEffect(() => {
    void fetchJobs();
  }, [fetchJobs, filter, page]);

  useTauriEvent("job-updated", upsertFromEvent);
  useTauriEvent("upload-progress", applyProgress);

  return (
    <section className="flex min-h-[200px] flex-1 flex-col">
      {/* `justify-between` was already here waiting for a second child: the
          watcher badge belongs at the far end of this row, captioning the list
          it counts. */}
      <div className="flex shrink-0 flex-wrap items-center justify-between gap-sm px-md py-sm">
        <JobsFilterBar filter={filter} onChange={setFilter} />
        <WatcherStatusBadge />
      </div>

      {error && <ErrorText className="shrink-0 px-md pb-sm">{error}</ErrorText>}

      {/*
        The rows are the page's only unbounded region, so this wrapper — not
        `<main>` — is what scrolls them: `flex-1 min-h-0` lets it absorb the
        space `LogConsole` gives back when collapsed, and `overflow-auto`
        keeps the horizontal scroll the fixed columns below still need. The
        header row is `sticky` so it survives that vertical scroll.
      */}
      <div className="min-h-0 flex-1 overflow-auto">
        <table role="table" className="w-full min-w-[880px] table-fixed border-collapse">
          {/*
            DESIGN.md §8 specifies 30/9/19/19/13/10% for the ≥1440px reference
            layout, sized for a `⋯` menu that no longer exists.

            Three columns are content-determined, so they are fixed in px
            rather than re-tuned percentages: Ações fits `RowActions`' widest
            cluster (6 buttons of 24px + 2px gaps + the cell's `px-md` = 178px,
            reached by a job with one side uploading and the other failed);
            Status fits "Sincronizado", the longest label at 90px plus padding,
            which the old 13% was already clipping below ~1100px; Tamanho fits
            its own header. The remaining three share what is left by
            percentage and truncate gracefully.

            `min-w-[880px]` is the floor those fixed columns imply: 400px of
            them plus the other three at 54% only fit while 400 <= 0.46 * W,
            i.e. W >= 870. Below a ~1184px window the wrapper's
            `overflow-x-auto` scrolls rather than squashing the cluster — at
            the 1280px default window (`tauri.conf.json`) the content box is
            976px, so it doesn't. The page itself never scrolls sideways.

            The `min-[1440px]` breakpoint is deliberately exact rather than the
            codebase's usual `xl:` (1280px) approximation (see `KpiRow.tsx`).
          */}
          <colgroup>
            <col className="w-[30%] min-[1440px]:w-[34%]" />
            <col className="w-[90px]" />
            <col className="w-[12%] min-[1440px]:w-[15%]" />
            <col className="w-[12%] min-[1440px]:w-[15%]" />
            <col className="w-[130px]" />
            <col className="w-[180px]" />
          </colgroup>
          {/*
            `sticky` lives on the cells, not on `<tr>`/`<thead>`: with
            `border-collapse: collapse` a sticky row's own border stops
            painting once it detaches, so the hairline is drawn as a
            `box-shadow` on each cell instead (`shadow-[inset_0_-1px_0]`),
            which scrolls with the cell and can't drop out.
          */}
          <thead>
            <tr className="h-7 text-left [&>th]:sticky [&>th]:top-0 [&>th]:z-10 [&>th]:bg-surface-1 [&>th]:shadow-[inset_0_-1px_0_var(--color-border-hairline)]">
              <th scope="col" className="truncate px-md text-label-sm uppercase text-text-tertiary">
                {t("pages.dashboard.table.headers.fileOrigin")}
              </th>
              <th scope="col" className="truncate px-md text-right text-label-sm uppercase text-text-tertiary">
                {t("pages.dashboard.table.headers.size")}
              </th>
              <th scope="col" className="min-w-0 px-md text-label-sm uppercase text-text-tertiary">
                <span className="flex min-w-0 items-center gap-2xs">
                  <HeaderDot className="bg-primary-strong" />
                  <span className="truncate">{t("pages.dashboard.table.headers.gdrive")}</span>
                </span>
              </th>
              <th scope="col" className="min-w-0 px-md text-label-sm uppercase text-text-tertiary">
                <span className="flex min-w-0 items-center gap-2xs">
                  <HeaderDot className="bg-secondary-strong" />
                  <span className="truncate">{t("pages.dashboard.table.headers.s3")}</span>
                </span>
              </th>
              <th scope="col" className="truncate px-md text-label-sm uppercase text-text-tertiary">
                {t("pages.dashboard.table.headers.status")}
              </th>
              <th scope="col" className="truncate px-md text-right text-label-sm uppercase text-text-tertiary">
                {t("pages.dashboard.table.headers.actions")}
              </th>
            </tr>
          </thead>
          <tbody>
            {loading ? (
              Array.from({ length: SKELETON_ROW_COUNT }, (_, index) => <SkeletonRow key={index} />)
            ) : items.length === 0 ? (
              <tr>
                <td colSpan={6} className="py-xl text-center text-body-sm text-text-secondary">
                  {t("pages.dashboard.table.emptyFiltered")}
                </td>
              </tr>
            ) : (
              items.map((job) => <JobRow key={job.file_id} job={job} progress={progress} />)
            )}
          </tbody>
        </table>
      </div>

      <Pagination
        page={page}
        pageSize={pageSize}
        total={total}
        onPrev={() => setPage(Math.max(0, page - 1))}
        onNext={() => setPage(page + 1)}
      />
    </section>
  );
}
