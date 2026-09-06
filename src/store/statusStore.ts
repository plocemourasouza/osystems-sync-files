/**
 * `statusStore` — zustand store backing the Dashboard KPIs, sidebar throughput
 * card and watcher controls (PRD.md RF-060/061/067; SPEC.md §7 `get_status`,
 * `rescan`, `pause_watcher`, `resume_watcher`, `pick_folder`; event `status-changed`).
 *
 * Convention (CLAUDE.md): this is the only place that imports `@/api/ipc`'s
 * status/watcher wrappers — components call the actions/selectors exported here.
 */
import { create } from "zustand";

import {
  getStatus,
  pauseWatcher as ipcPauseWatcher,
  pickFolder as ipcPickFolder,
  rescan as ipcRescan,
  resumeWatcher as ipcResumeWatcher,
} from "@/api/ipc";
import { isMockMode } from "@/dev/mockMode";
import { useConfigStore } from "@/store/configStore";
import type { AppStatus, RescanReport } from "@/types/generated";

export interface StatusActions {
  /**
   * RF-061 "Atualizar lista": reconciles the queue with the current filters,
   * then refreshes the counters. Resolves the full report — the scan both
   * enqueues and archives, so the header has two numbers to show.
   */
  rescan: () => Promise<RescanReport>;
  /** RF12/RF-061: pauses the watcher, then refreshes. */
  pauseWatcher: () => Promise<void>;
  /** RF12/RF-061: resumes the watcher, then refreshes. */
  resumeWatcher: () => Promise<void>;
  /**
   * Opens the native folder picker. On a non-cancelled pick, `configStore` is
   * reloaded so the sidebar's "pasta monitorada" card reflects the new
   * `watch.path` immediately (the backend restarts the watcher on save).
   */
  pickFolder: () => Promise<string | null>;
}

interface StatusState {
  status: AppStatus | null;
  loading: boolean;
  error: string | null;
  /** Fetches `get_status` and replaces `status`. */
  refresh: () => Promise<void>;
  /** Applies a `status-changed` event payload without a round-trip fetch. */
  applyEvent: (status: AppStatus) => void;
  actions: StatusActions;
}

export type StatusStore = StatusState;

export const useStatusStore = create<StatusStore>()((set, get) => ({
  status: null,
  loading: false,
  error: null,

  refresh: async () => {
    // Dev preview (`?mock=1`): `src/dev/mock.ts` already seeded `status`.
    if (isMockMode()) return;
    set({ loading: true, error: null });
    try {
      const status = await getStatus();
      set({ status, loading: false });
    } catch (e) {
      set({ loading: false, error: e instanceof Error ? e.message : "unknown" });
    }
  },

  applyEvent: (status) => set({ status }),

  actions: {
    rescan: async () => {
      const report = await ipcRescan();
      await get().refresh();
      return report;
    },

    pauseWatcher: async () => {
      await ipcPauseWatcher();
      await get().refresh();
    },

    resumeWatcher: async () => {
      await ipcResumeWatcher();
      await get().refresh();
    },

    pickFolder: async () => {
      const path = await ipcPickFolder();
      if (path !== null) {
        await useConfigStore.getState().load();
      }
      await get().refresh();
      return path;
    },
  },
}));

/** Dashboard KPI row (RF-060), derived from `AppStatus.counts_by_status`. */
export interface StatusKpis {
  /**
   * Approximate count of *files* detected. `StatusCounts` counters are
   * per-job (2 job rows per file: `s3` + `gdrive`), so this sums every
   * counter and halves it — exact when both sides of every file share the
   * same status, off by at most a few units otherwise (acceptable for a KPI
   * headline; the authoritative per-file view is the jobs table).
   */
  detected: number;
  /** Sum of `files.size` for every detected file (`StatusCounts.bytes_total`). */
  bytesTotal: number;
  /** Sum of `files.size` for files whose both jobs are done (`StatusCounts.bytes_done`). */
  bytesDone: number;
  /**
   * Per-job count of `done` sides (`StatusCounts.done`) — not divided by 2,
   * so it can exceed the number of fully-synced *files*. Use `donePct` (byte-based)
   * for the "% da carga" figure RF-060 asks for.
   */
  done: number;
  /** `bytesDone / bytesTotal`, rounded to a whole percent. `0` when nothing detected yet. */
  donePct: number;
  /** Per-job count of `uploading` sides. */
  uploading: number;
  /** Per-job count of jobs still queued: `pending + paused`. */
  queued: number;
  /** Per-job count of `failed` sides. */
  failed: number;
}

const EMPTY_KPIS: StatusKpis = {
  detected: 0,
  bytesTotal: 0,
  bytesDone: 0,
  done: 0,
  donePct: 0,
  uploading: 0,
  queued: 0,
  failed: 0,
};

/** Derives the Dashboard KPI row from an `AppStatus` snapshot (or `null` while loading). */
export function selectKpis(status: AppStatus | null): StatusKpis {
  if (status === null) return EMPTY_KPIS;

  const c = status.counts_by_status;
  const detected = Math.round((c.pending + c.uploading + c.paused + c.cancelled + c.done + c.failed) / 2);
  const donePct = c.bytes_total > 0 ? Math.round((c.bytes_done / c.bytes_total) * 100) : 0;

  return {
    detected,
    bytesTotal: c.bytes_total,
    bytesDone: c.bytes_done,
    done: c.done,
    donePct,
    uploading: c.uploading,
    queued: c.pending + c.paused,
    failed: c.failed,
  };
}
