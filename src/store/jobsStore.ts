/**
 * `jobsStore` — zustand store backing the Dashboard's sync table (PRD.md
 * RF-062/063/064; SPEC.md §7 `list_jobs`; events `job-updated`, `upload-progress`).
 *
 * Convention (CLAUDE.md): this is the only place that imports `@/api/ipc`'s
 * `listJobs` wrapper — components call the actions/selectors exported here.
 */
import { create } from "zustand";

import { listJobs } from "@/api/ipc";
import { isMockMode } from "@/dev/mockMode";
import type { JobSide, JobStatus, JobView, ListJobsQuery, UploadProgress } from "@/types/generated";

export type JobsFilter = "all" | "active" | "done" | "failed";

export const JOBS_PAGE_SIZE = 50;

/**
 * Maps a table filter (RF-064) to a `list_jobs` query. `done` needs
 * `include_archived: true` because RF-061's "Limpar concluídos" archives
 * every visible `done` job — without it, the Concluídos tab would empty out
 * right after the user clears the table.
 */
export function buildJobsQuery(filter: JobsFilter, page: number): ListJobsQuery {
  const base = {
    destination: null,
    limit: JOBS_PAGE_SIZE,
    offset: page * JOBS_PAGE_SIZE,
  };

  switch (filter) {
    case "active":
      return { ...base, statuses: ["pending", "uploading", "paused"], include_archived: false };
    case "done":
      return { ...base, statuses: ["done"], include_archived: true };
    case "failed":
      return { ...base, statuses: ["failed"], include_archived: false };
    case "all":
      return { ...base, statuses: null, include_archived: false };
  }
}

/** Whether a job side is still "in flight" in some sense (RF-063). */
function isActiveSide(status: JobStatus): boolean {
  return status === "pending" || status === "uploading" || status === "paused";
}

export type AggregatedStatus = "failed" | "uploading" | "paused" | "done" | "cancelled" | "queued";

/**
 * Aggregates a `JobView`'s two sides (`gdrive` + `s3`) into the single badge
 * shown per row (RF-063): any `failed` wins, then any `uploading`, then any
 * `paused`; both `done` is "Sincronizado"; any `cancelled` with neither side
 * still active is "Cancelado"; everything else is "Na Fila".
 */
export function aggregateStatus(job: JobView): AggregatedStatus {
  const statuses: JobStatus[] = [job.gdrive.status, job.s3.status];

  if (statuses.some((s) => s === "failed")) return "failed";
  if (statuses.some((s) => s === "uploading")) return "uploading";
  if (statuses.some((s) => s === "paused")) return "paused";
  if (statuses.every((s) => s === "done")) return "done";
  if (statuses.some((s) => s === "cancelled") && !statuses.some(isActiveSide)) return "cancelled";
  return "queued";
}

interface JobsState {
  filter: JobsFilter;
  /** 0-based. */
  page: number;
  pageSize: number;
  items: JobView[];
  total: number;
  loading: boolean;
  error: string | null;
  /** Latest `upload-progress` event payload per job id (one entry per side). */
  progress: Record<string, UploadProgress>;

  /** Changes the active table filter (RF-064) and resets to the first page. */
  setFilter: (filter: JobsFilter) => void;
  setPage: (page: number) => void;
  /**
   * Fetches the current filter/page from `list_jobs`.
   *
   * Pass `{ force: true }` after an action that changed the queue server-side
   * (rescan, retry-all, clear-completed). The de-dup below exists to collapse
   * *redundant* repeats of the same request; after a mutation the identical
   * request is no longer redundant, and skipping it leaves the table showing
   * pre-mutation rows.
   */
  fetch: (options?: { force?: boolean }) => Promise<void>;
  /**
   * Applies a `job-updated` event in place: replaces the row sharing
   * `file_id` if it's on the current page (even if the row would no longer
   * match `filter` — removing it here would flicker; the next `fetch()`
   * settles it). If it's a genuinely new row and we're looking at page 0 of
   * `all`/`active`, it's prepended and the page is trimmed back to `pageSize`.
   */
  upsertFromEvent: (job: JobView) => void;
  /** Batches `upload-progress` events into `progress`, coalesced onto one macrotask. */
  applyProgress: (progress: UploadProgress) => void;

  /** @internal buffer for `applyProgress`'s coalescing — reset with the rest of the store in tests. */
  _pendingProgress: Record<string, UploadProgress>;
  /** @internal whether a flush of `_pendingProgress` is already scheduled. */
  _flushScheduled: boolean;
  /**
   * @internal `{filter}:{page}` key of the in-flight `fetch()` call, if any —
   * lets a second `fetch()` for the identical filter/page skip while the
   * first is still resolving (RNF-008: rapid page/filter clicks that land on
   * the same request shouldn't double up on `list_jobs`).
   */
  _inFlightKey: string | null;
  /**
   * @internal Monotonic id of the most recently *issued* `fetch()`. A response
   * is applied only if it still carries the latest id, so a request that was
   * already in flight when the queue changed can't land last and overwrite the
   * fresher rows with pre-mutation ones.
   */
  _fetchSeq: number;
}

export type JobsStore = JobsState;

export const useJobsStore = create<JobsStore>()((set, get) => ({
  filter: "all",
  page: 0,
  pageSize: JOBS_PAGE_SIZE,
  items: [],
  total: 0,
  loading: false,
  error: null,
  progress: {},
  _pendingProgress: {},
  _flushScheduled: false,
  _inFlightKey: null,
  _fetchSeq: 0,

  setFilter: (filter) => set({ filter, page: 0 }),

  setPage: (page) => set({ page }),

  fetch: async (options) => {
    // Dev preview (`?mock=1`): `src/dev/mock.ts` already seeded `items`.
    if (isMockMode()) return;
    const { filter, page, loading, _inFlightKey } = get();
    const key = `${filter}:${page}`;

    // De-dup: an identical filter/page request is already in flight — skip
    // rather than firing a redundant `list_jobs` call (RNF-008). `force`
    // opts out: after a rescan the same query returns different rows, so the
    // repeat is the point.
    if (!options?.force && loading && _inFlightKey === key) return;

    const seq = get()._fetchSeq + 1;
    set({ loading: true, error: null, _inFlightKey: key, _fetchSeq: seq });
    try {
      const result = await listJobs(buildJobsQuery(filter, page));
      // A slower earlier request must not clobber a newer one's rows.
      if (get()._fetchSeq !== seq) return;
      set({ items: result.items, total: result.total, loading: false, _inFlightKey: null });
    } catch (e) {
      if (get()._fetchSeq !== seq) return;
      set({ loading: false, error: e instanceof Error ? e.message : "unknown", _inFlightKey: null });
    }
  },

  upsertFromEvent: (job) => {
    const { items, filter, page, pageSize } = get();
    const index = items.findIndex((item) => item.file_id === job.file_id);

    if (index !== -1) {
      const next = [...items];
      next[index] = job;
      set({ items: next });
      return;
    }

    if ((filter === "all" || filter === "active") && page === 0) {
      set({ items: [job, ...items].slice(0, pageSize) });
    }
  },

  applyProgress: (progress) => {
    get()._pendingProgress[progress.job_id] = progress;

    if (get()._flushScheduled) return;
    set({ _flushScheduled: true });

    setTimeout(() => {
      const batch = get()._pendingProgress;
      set((state) => ({
        progress: { ...state.progress, ...batch },
        _pendingProgress: {},
        _flushScheduled: false,
      }));
    }, 0);
  },
}));

/** A job's side for the given destination, for the table's two per-destination columns. */
export function jobSide(job: JobView, destination: "s3" | "gdrive"): JobSide {
  return job[destination];
}
