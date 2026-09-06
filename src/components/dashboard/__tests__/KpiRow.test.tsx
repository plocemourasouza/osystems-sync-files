import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import type { AppStatus, StatusCounts } from "@/types/generated";
import { KpiRow } from "@/components/dashboard/KpiRow";
import { useStatusStore } from "@/store/statusStore";

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

const initialState = useStatusStore.getState();

beforeEach(() => {
  useStatusStore.setState(initialState, true);
});

describe("KpiRow", () => {
  it("maps selectKpis output from the fixture AppStatus onto the 5 cards", () => {
    useStatusStore.setState({
      status: status({
        counts_by_status: counts({
          pending: 2,
          uploading: 3,
          done: 16,
          failed: 1,
          bytes_total: 6.42e9,
          bytes_done: 3.66e9,
        }),
      }),
    });

    render(<KpiRow />);

    // detected = round((2 + 3 + 0 + 0 + 16 + 1) / 2) = 11
    expect(screen.getByText("11")).toBeInTheDocument();
    // donePct = round(3.66e9 / 6.42e9 * 100) = 57
    expect(screen.getByText(/57%/)).toBeInTheDocument();
    expect(screen.getByText("Requer atenção")).toBeInTheDocument();
  });
});
