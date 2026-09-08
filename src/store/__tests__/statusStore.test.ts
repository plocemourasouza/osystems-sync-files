import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AppStatus, StatusCounts } from "@/types/generated";

vi.mock("@/api/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/ipc")>();
  return {
    ...actual,
    getStatus: vi.fn(),
    rescan: vi.fn(),
    pauseWatcher: vi.fn(),
    resumeWatcher: vi.fn(),
    pickFolder: vi.fn(),
  };
});

vi.mock("@/store/configStore", () => ({
  useConfigStore: { getState: vi.fn(() => ({ load: vi.fn() })) },
}));

import { getStatus, pauseWatcher, pickFolder, rescan, resumeWatcher } from "@/api/ipc";
import { useConfigStore } from "@/store/configStore";
import { selectKpis, useStatusStore } from "@/store/statusStore";

const mockedGetStatus = vi.mocked(getStatus);
const mockedRescan = vi.mocked(rescan);
const mockedPauseWatcher = vi.mocked(pauseWatcher);
const mockedResumeWatcher = vi.mocked(resumeWatcher);
const mockedPickFolder = vi.mocked(pickFolder);

const initialState = useStatusStore.getState();

function counts(overrides: Partial<StatusCounts> = {}): StatusCounts {
  return {
    pending: 0,
    uploading: 0,
    paused: 0,
    cancelled: 0,
    done: 0,
    failed: 0,
    bytes_total: 0,
    bytes_done: 0,
    ...overrides,
  };
}

function status(overrides: Partial<AppStatus> = {}): AppStatus {
  return {
    watcher_paused: false,
    destinations: {
      gdrive: { online: true, auth_required: false, latency_ms: 24 },
      s3: { online: true, auth_required: false, latency_ms: 41 },
    },
    counts_by_status: counts(),
    core_version: "0.1.0",
    build_target: "Tauri 2 • Windows x64",
    ...overrides,
  };
}

beforeEach(() => {
  useStatusStore.setState(initialState, true);
  mockedGetStatus.mockReset();
  mockedRescan.mockReset();
  mockedPauseWatcher.mockReset();
  mockedResumeWatcher.mockReset();
  mockedPickFolder.mockReset();
  vi.mocked(useConfigStore.getState).mockClear();
});

describe("statusStore.refresh()", () => {
  it("fetches get_status and replaces `status`", async () => {
    const snapshot = status({ counts_by_status: counts({ pending: 2 }) });
    mockedGetStatus.mockResolvedValueOnce(snapshot);

    await useStatusStore.getState().refresh();

    const state = useStatusStore.getState();
    expect(state.status).toEqual(snapshot);
    expect(state.loading).toBe(false);
    expect(state.error).toBeNull();
  });

  it("sets an error message on rejection without throwing", async () => {
    mockedGetStatus.mockRejectedValueOnce(new Error("core offline"));

    await useStatusStore.getState().refresh();

    const state = useStatusStore.getState();
    expect(state.status).toBeNull();
    expect(state.error).toBe("core offline");
    expect(state.loading).toBe(false);
  });
});

describe("statusStore.applyEvent()", () => {
  it("replaces `status` synchronously, without an ipc round-trip", () => {
    const snapshot = status({ watcher_paused: true });

    useStatusStore.getState().applyEvent(snapshot);

    expect(useStatusStore.getState().status).toEqual(snapshot);
    expect(mockedGetStatus).not.toHaveBeenCalled();
  });
});

describe("statusStore.actions", () => {
  it("rescan() calls ipc.rescan() then refreshes status, returning the whole report", async () => {
    const report = {
      scanned: 6,
      enqueued: 5,
      unchanged: 1,
      skipped_filtered: 0,
      skipped_symlink: 0,
      skipped_unreadable: 0,
      errors: 0,
      archived: 2,
      restored: 0,
    };
    mockedRescan.mockResolvedValueOnce(report);
    mockedGetStatus.mockResolvedValueOnce(status());

    const result = await useStatusStore.getState().actions.rescan();

    expect(result).toEqual(report);
    expect(mockedGetStatus).toHaveBeenCalledTimes(1);
  });

  // The `rescan` command surfaces the real cause (RescanError::to_string(), e.g.
  // an I/O permission error) as an AppError — the store must not swallow it, and
  // must record it in `error` so it's the single source of truth even for a
  // future caller that doesn't handle the rejection itself.
  it("rescan() records the AppError message in `error` and rethrows, without refreshing", async () => {
    mockedRescan.mockRejectedValueOnce({
      code: "rescan.error",
      message: "io error while scanning: Acesso negado. (os error 5)",
    });

    await expect(useStatusStore.getState().actions.rescan()).rejects.toEqual({
      code: "rescan.error",
      message: "io error while scanning: Acesso negado. (os error 5)",
    });

    expect(useStatusStore.getState().error).toBe("io error while scanning: Acesso negado. (os error 5)");
    expect(mockedGetStatus).not.toHaveBeenCalled();
  });

  it("pauseWatcher() calls ipc.pauseWatcher() then refreshes", async () => {
    mockedPauseWatcher.mockResolvedValueOnce(undefined);
    mockedGetStatus.mockResolvedValueOnce(status({ watcher_paused: true }));

    await useStatusStore.getState().actions.pauseWatcher();

    expect(mockedPauseWatcher).toHaveBeenCalledTimes(1);
    expect(useStatusStore.getState().status?.watcher_paused).toBe(true);
  });

  it("resumeWatcher() calls ipc.resumeWatcher() then refreshes", async () => {
    mockedResumeWatcher.mockResolvedValueOnce(undefined);
    mockedGetStatus.mockResolvedValueOnce(status());

    await useStatusStore.getState().actions.resumeWatcher();

    expect(mockedResumeWatcher).toHaveBeenCalledTimes(1);
    expect(mockedGetStatus).toHaveBeenCalledTimes(1);
  });

  it("pickFolder() reloads configStore and refreshes when a folder is picked", async () => {
    const load = vi.fn();
    vi.mocked(useConfigStore.getState).mockReturnValue({ load } as unknown as ReturnType<
      typeof useConfigStore.getState
    >);
    mockedPickFolder.mockResolvedValueOnce("C:\\NovaPasta");
    mockedGetStatus.mockResolvedValueOnce(status());

    const result = await useStatusStore.getState().actions.pickFolder();

    expect(result).toBe("C:\\NovaPasta");
    expect(load).toHaveBeenCalledTimes(1);
    expect(mockedGetStatus).toHaveBeenCalledTimes(1);
  });

  it("pickFolder() does not reload configStore when the user cancels", async () => {
    const load = vi.fn();
    vi.mocked(useConfigStore.getState).mockReturnValue({ load } as unknown as ReturnType<
      typeof useConfigStore.getState
    >);
    mockedPickFolder.mockResolvedValueOnce(null);
    mockedGetStatus.mockResolvedValueOnce(status());

    const result = await useStatusStore.getState().actions.pickFolder();

    expect(result).toBeNull();
    expect(load).not.toHaveBeenCalled();
  });
});

describe("selectKpis()", () => {
  it("returns all-zero KPIs for a null status (loading state)", () => {
    expect(selectKpis(null)).toEqual({
      detected: 0,
      bytesTotal: 0,
      bytesDone: 0,
      done: 0,
      donePct: 0,
      uploading: 0,
      queued: 0,
      failed: 0,
    });
  });

  it("halves the summed per-job counters into an approximate file count", () => {
    // 4 files, both sides `done`: 8 per-job counters total -> 4 files.
    const s = status({ counts_by_status: counts({ done: 8, bytes_total: 4000, bytes_done: 4000 }) });

    const kpis = selectKpis(s);

    expect(kpis.detected).toBe(4);
    expect(kpis.done).toBe(8);
    expect(kpis.donePct).toBe(100);
  });

  it("computes queued as pending + paused and donePct from bytes", () => {
    const s = status({
      counts_by_status: counts({ pending: 3, paused: 1, uploading: 2, failed: 1, bytes_total: 1000, bytes_done: 250 }),
    });

    const kpis = selectKpis(s);

    expect(kpis.queued).toBe(4);
    expect(kpis.uploading).toBe(2);
    expect(kpis.failed).toBe(1);
    expect(kpis.donePct).toBe(25);
  });

  it("donePct is 0 when bytes_total is 0 (nothing detected yet), not NaN", () => {
    const s = status({ counts_by_status: counts({ bytes_total: 0, bytes_done: 0 }) });

    expect(selectKpis(s).donePct).toBe(0);
  });
});
