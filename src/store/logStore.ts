/**
 * `logStore` — zustand store backing the Dashboard's collapsible event console
 * (PRD.md RF-066; SPEC.md §7 `get_recent_logs`; event `log-line`).
 *
 * Convention (CLAUDE.md): this is the only place that imports `@/api/ipc`'s
 * `getRecentLogs` wrapper — components call the actions/selectors exported here.
 */
import { create } from "zustand";

import { getRecentLogs } from "@/api/ipc";
import { isMockMode } from "@/dev/mockMode";
import type { LogLine } from "@/types/generated";

/** RF-066: last 500 lines kept in memory, regardless of how many arrive. */
export const LOG_RING_CAPACITY = 500;

const COLLAPSED_STORAGE_KEY = "osystems-sync.console.collapsed";

/** Guarded per RNF-003-style hygiene: storage can be disabled/unavailable (private mode, quota). */
function readCollapsed(): boolean {
  try {
    return localStorage.getItem(COLLAPSED_STORAGE_KEY) === "true";
  } catch {
    return false;
  }
}

function writeCollapsed(collapsed: boolean): void {
  try {
    localStorage.setItem(COLLAPSED_STORAGE_KEY, String(collapsed));
  } catch {
    // Storage unavailable (private browsing, quota, disabled): collapsed state
    // just won't survive a reload — not fatal.
  }
}

export type LogLevelFilter = "all" | "debug" | "info" | "warn" | "error";

/** RF-066 origin tag, derived from `LogLine.target` (a Rust module path / span target). */
export type LogSource = "WATCHER" | "HASH" | "STATE" | "CORE" | "S3" | "GDRIVE" | "APP";

/** Maps a `LogLine.target` to the console's origin tag. */
export function source(line: LogLine): LogSource {
  const target = line.target;

  if (target.includes("::watcher")) return "WATCHER";
  if (target.includes("::hash")) return "HASH";
  if (target.includes("::state") || target.includes("::queue")) return "STATE";
  if (target.includes("uploaders::s3")) return "S3";
  if (target.includes("uploaders::gdrive")) return "GDRIVE";
  if (target.includes("osystems_sync_lib")) return "APP";
  return "CORE";
}

function capRing(lines: LogLine[]): LogLine[] {
  if (lines.length <= LOG_RING_CAPACITY) return lines;
  return lines.slice(lines.length - LOG_RING_CAPACITY);
}

interface LogState {
  /** Ring buffer, oldest first, capped at `LOG_RING_CAPACITY`. */
  lines: LogLine[];
  level: LogLevelFilter;
  collapsed: boolean;

  setLevel: (level: LogLevelFilter) => void;
  toggleCollapsed: () => void;
  /** Appends one `log-line` event, dropping the oldest entry once past capacity. */
  push: (line: LogLine) => void;
  /** Replaces the buffer wholesale, e.g. with `get_recent_logs`'s result on mount. */
  hydrate: (lines: LogLine[]) => void;
  /** Convenience: `getRecentLogs(LOG_RING_CAPACITY)` then `hydrate()`. */
  fetchRecent: () => Promise<void>;
}

export type LogStore = LogState;

export const useLogStore = create<LogStore>()((set, get) => ({
  lines: [],
  level: "all",
  collapsed: readCollapsed(),

  setLevel: (level) => set({ level }),

  toggleCollapsed: () => {
    const next = !get().collapsed;
    writeCollapsed(next);
    set({ collapsed: next });
  },

  push: (line) => {
    set((state) => ({ lines: capRing([...state.lines, line]) }));
  },

  hydrate: (lines) => {
    set({ lines: capRing(lines) });
  },

  fetchRecent: async () => {
    // Dev preview (`?mock=1`): `src/dev/mock.ts` already hydrated `lines`.
    if (isMockMode()) return;
    // Hydration is best-effort: outside the Tauri runtime (browser preview,
    // tests without an IPC mock) the command is unavailable — the console
    // simply starts empty and fills from `log-line` events. Never let this
    // reject into a React effect.
    try {
      const lines = await getRecentLogs(LOG_RING_CAPACITY);
      get().hydrate(lines);
    } catch {
      // Keep whatever is already in the ring; nothing to surface to the user.
    }
  },
}));

/** Filters `lines` by `level` ("all" passes everything through), for the console's list. */
export function selectFilteredLines(lines: LogLine[], level: LogLevelFilter): LogLine[] {
  if (level === "all") return lines;
  const wanted = level.toUpperCase();
  return lines.filter((line) => line.level.toUpperCase() === wanted);
}
