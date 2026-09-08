import { beforeEach, describe, expect, it, vi } from "vitest";

import type { LogLine } from "@/types/generated";

vi.mock("@/api/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/ipc")>();
  return {
    ...actual,
    getRecentLogs: vi.fn(),
  };
});

import { getRecentLogs } from "@/api/ipc";
import { LOG_RING_CAPACITY, selectFilteredLines, source, useLogStore } from "@/store/logStore";

const mockedGetRecentLogs = vi.mocked(getRecentLogs);

const initialState = useLogStore.getState();

function line(overrides: Partial<LogLine> = {}): LogLine {
  return {
    ts: "2026-01-01T00:00:00.000Z",
    level: "INFO",
    target: "osystems_sync_core::watcher",
    job_id: null,
    destination: null,
    message: "hi",
    error: null,
    path: null,
    ...overrides,
  };
}

function localStorageMock() {
  const backing = new Map<string, string>();
  return {
    getItem: (key: string) => backing.get(key) ?? null,
    setItem: (key: string, value: string) => {
      backing.set(key, value);
    },
    removeItem: (key: string) => {
      backing.delete(key);
    },
    clear: () => backing.clear(),
  };
}

beforeEach(() => {
  useLogStore.setState(initialState, true);
  mockedGetRecentLogs.mockReset();
});

describe("logStore ring buffer", () => {
  it("push() appends a line", () => {
    useLogStore.getState().push(line({ message: "one" }));

    expect(useLogStore.getState().lines).toHaveLength(1);
    expect(useLogStore.getState().lines[0]?.message).toBe("one");
  });

  it("push() drops the oldest line once past LOG_RING_CAPACITY (500)", () => {
    for (let i = 0; i < LOG_RING_CAPACITY; i += 1) {
      useLogStore.getState().push(line({ message: `line-${i}` }));
    }
    expect(useLogStore.getState().lines).toHaveLength(LOG_RING_CAPACITY);

    useLogStore.getState().push(line({ message: "overflow" }));

    const lines = useLogStore.getState().lines;
    expect(lines).toHaveLength(LOG_RING_CAPACITY);
    expect(lines[0]?.message).toBe("line-1");
    expect(lines[lines.length - 1]?.message).toBe("overflow");
  });

  it("hydrate() replaces the buffer and also caps it at LOG_RING_CAPACITY", () => {
    const many = Array.from({ length: LOG_RING_CAPACITY + 10 }, (_, i) => line({ message: `h-${i}` }));

    useLogStore.getState().hydrate(many);

    const lines = useLogStore.getState().lines;
    expect(lines).toHaveLength(LOG_RING_CAPACITY);
    expect(lines[0]?.message).toBe("h-10");
  });

  it("fetchRecent() calls getRecentLogs(LOG_RING_CAPACITY) and hydrates from the result", async () => {
    const fetched = [line({ message: "fetched-1" }), line({ message: "fetched-2" })];
    mockedGetRecentLogs.mockResolvedValueOnce(fetched);

    await useLogStore.getState().fetchRecent();

    expect(mockedGetRecentLogs).toHaveBeenCalledWith(LOG_RING_CAPACITY);
    expect(useLogStore.getState().lines).toEqual(fetched);
  });
});

describe("logStore level filter", () => {
  it("setLevel() updates the level", () => {
    useLogStore.getState().setLevel("error");
    expect(useLogStore.getState().level).toBe("error");
  });

  it("selectFilteredLines('all') passes every line through", () => {
    const lines = [line({ level: "INFO" }), line({ level: "ERROR" })];
    expect(selectFilteredLines(lines, "all")).toEqual(lines);
  });

  it("selectFilteredLines(level) matches case-insensitively", () => {
    const info = line({ level: "INFO" });
    const warn = line({ level: "WARN" });
    const error = line({ level: "ERROR" });

    expect(selectFilteredLines([info, warn, error], "warn")).toEqual([warn]);
    expect(selectFilteredLines([info, warn, error], "error")).toEqual([error]);
  });
});

describe("logStore.collapsed (localStorage, guarded)", () => {
  it("toggleCollapsed() flips state and persists to localStorage", () => {
    const store = localStorageMock();
    vi.stubGlobal("localStorage", store);

    expect(useLogStore.getState().collapsed).toBe(false);

    useLogStore.getState().toggleCollapsed();

    expect(useLogStore.getState().collapsed).toBe(true);
    expect(store.getItem("osystems-sync.console.collapsed")).toBe("true");

    vi.unstubAllGlobals();
  });

  it("toggleCollapsed() does not throw when localStorage access throws (private browsing)", () => {
    vi.stubGlobal("localStorage", {
      getItem: () => {
        throw new Error("blocked");
      },
      setItem: () => {
        throw new Error("blocked");
      },
    });

    expect(() => useLogStore.getState().toggleCollapsed()).not.toThrow();

    vi.unstubAllGlobals();
  });
});

describe("source() (RF-066 origin tag)", () => {
  it.each<[string, ReturnType<typeof source>]>([
    ["osystems_sync_core::watcher", "WATCHER"],
    ["osystems_sync_core::hash", "HASH"],
    ["osystems_sync_core::state", "STATE"],
    ["osystems_sync_core::queue", "STATE"],
    ["osystems_sync_core::uploaders::s3", "S3"],
    ["osystems_sync_core::uploaders::gdrive", "GDRIVE"],
    ["osystems_sync_lib::tray", "APP"],
    ["osystems_sync_core::db", "CORE"],
  ])("target %s -> %s", (target, expected) => {
    expect(source(line({ target }))).toBe(expected);
  });
});
