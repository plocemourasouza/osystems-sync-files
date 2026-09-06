/**
 * `throughputStore` — unit tests (PRD.md RF-067). Covers `apply()` replacing
 * the snapshot from a `throughput` event payload and `selectCapBps()`'s
 * "both set or unlimited" rule.
 */
import { beforeEach, describe, expect, it } from "vitest";

import { selectCapBps, useThroughputStore } from "@/store/throughputStore";
import type { Throughput } from "@/types/generated";

const initialState = useThroughputStore.getState();

function event(overrides: Partial<Throughput> = {}): Throughput {
  return {
    total_bps: 2_800_000,
    gdrive_bps: 1_600_000,
    s3_bps: 1_200_000,
    limit_gdrive_bps: null,
    limit_s3_bps: null,
    ...overrides,
  };
}

beforeEach(() => {
  useThroughputStore.setState(initialState, true);
});

describe("throughputStore", () => {
  it("starts zeroed out with no cap and no update timestamp", () => {
    const state = useThroughputStore.getState();
    expect(state.totalBps).toBe(0);
    expect(state.gdriveBps).toBe(0);
    expect(state.s3Bps).toBe(0);
    expect(state.limitGdriveBps).toBeNull();
    expect(state.limitS3Bps).toBeNull();
    expect(state.updatedAt).toBeNull();
  });

  it("apply() maps every snake_case field from the event onto the store", () => {
    useThroughputStore.getState().apply(
      event({ total_bps: 3_000_000, gdrive_bps: 1_800_000, s3_bps: 1_200_000, limit_gdrive_bps: 2_000_000, limit_s3_bps: 4_000_000 }),
    );

    const state = useThroughputStore.getState();
    expect(state.totalBps).toBe(3_000_000);
    expect(state.gdriveBps).toBe(1_800_000);
    expect(state.s3Bps).toBe(1_200_000);
    expect(state.limitGdriveBps).toBe(2_000_000);
    expect(state.limitS3Bps).toBe(4_000_000);
    expect(state.updatedAt).not.toBeNull();
  });

  it("apply() replaces the previous snapshot rather than merging it", () => {
    useThroughputStore.getState().apply(event({ limit_gdrive_bps: 2_000_000, limit_s3_bps: 4_000_000 }));
    useThroughputStore.getState().apply(event({ limit_gdrive_bps: null, limit_s3_bps: null }));

    const state = useThroughputStore.getState();
    expect(state.limitGdriveBps).toBeNull();
    expect(state.limitS3Bps).toBeNull();
  });

  it("selectCapBps sums both limits when both destinations are capped", () => {
    expect(selectCapBps({ limitGdriveBps: 2_000_000, limitS3Bps: 4_000_000 })).toBe(6_000_000);
  });

  it("selectCapBps returns null when either destination is unlimited", () => {
    expect(selectCapBps({ limitGdriveBps: null, limitS3Bps: 4_000_000 })).toBeNull();
    expect(selectCapBps({ limitGdriveBps: 2_000_000, limitS3Bps: null })).toBeNull();
    expect(selectCapBps({ limitGdriveBps: null, limitS3Bps: null })).toBeNull();
  });
});
