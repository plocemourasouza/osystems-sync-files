/**
 * `throughputStore` — Sidebar bandwidth widget state (PRD.md RF-067; SPEC.md §7
 * `throughput` event, emitted once per second with the current aggregate
 * transfer rate and the configured QoS ceilings).
 *
 * There is no `get_throughput` IPC command to hydrate a snapshot on mount —
 * this store is populated exclusively by `apply()`, fed from the `throughput`
 * Tauri event (subscribed once in `App.tsx` via `useTauriEvent`).
 */
import { create } from "zustand";

import type { Throughput } from "@/types/generated";

interface ThroughputState {
  totalBps: number;
  gdriveBps: number;
  s3Bps: number;
  /** QoS cap for Google Drive, in bytes/sec, or `null` when unlimited. */
  limitGdriveBps: number | null;
  /** QoS cap for S3, in bytes/sec, or `null` when unlimited. */
  limitS3Bps: number | null;
  /** ISO timestamp of the last applied `throughput` event, or `null` before the first one. */
  updatedAt: string | null;
  /** Applies a `throughput` event payload, replacing the previous snapshot. */
  apply: (event: Throughput) => void;
}

export type ThroughputStore = ThroughputState;

export const useThroughputStore = create<ThroughputStore>()((set) => ({
  totalBps: 0,
  gdriveBps: 0,
  s3Bps: 0,
  limitGdriveBps: null,
  limitS3Bps: null,
  updatedAt: null,

  apply: (event) =>
    set({
      totalBps: event.total_bps,
      gdriveBps: event.gdrive_bps,
      s3Bps: event.s3_bps,
      limitGdriveBps: event.limit_gdrive_bps,
      limitS3Bps: event.limit_s3_bps,
      updatedAt: new Date().toISOString(),
    }),
}));

/**
 * Derives `Sidebar`'s `throughput.capBps` (RF-067): the sum of both
 * destinations' QoS ceilings when *both* are set, `null` ("Ilimitado")
 * otherwise — a single unlimited destination makes the aggregate cap
 * meaningless as a ceiling.
 */
export function selectCapBps(state: Pick<ThroughputState, "limitGdriveBps" | "limitS3Bps">): number | null {
  const { limitGdriveBps, limitS3Bps } = state;
  if (limitGdriveBps === null || limitS3Bps === null) return null;
  return limitGdriveBps + limitS3Bps;
}
