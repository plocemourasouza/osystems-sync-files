import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { AppStatus, StatusCounts, RescanProgress, RescanReport } from "@/types/generated";
import { DashboardHeader } from "@/components/dashboard/DashboardHeader";
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
    retryAllFailed: vi.fn(),
    clearCompleted: vi.fn(),
    listJobs: vi.fn(),
  };
});

// Mirrors RescanFailedBanner.test.tsx: capture the `useTauriEvent` handler for
// "rescan-progress" so tests can fire it directly instead of going through a
// real Tauri event bridge (absent in jsdom).
let rescanProgressHandler: ((payload: RescanProgress) => void) | null = null;

vi.mock("@/api/events", () => ({
  useTauriEvent: vi.fn((eventName: string, handler: (payload: RescanProgress) => void) => {
    if (eventName === "rescan-progress") {
      rescanProgressHandler = handler;
    }
  }),
}));

import { clearCompleted, getStatus, listJobs, pauseWatcher, rescan, resumeWatcher, retryAllFailed } from "@/api/ipc";
import { useJobsStore } from "@/store/jobsStore";

const mockedGetStatus = vi.mocked(getStatus);
const mockedRescan = vi.mocked(rescan);
const mockedPauseWatcher = vi.mocked(pauseWatcher);
const mockedResumeWatcher = vi.mocked(resumeWatcher);
const mockedRetryAllFailed = vi.mocked(retryAllFailed);
const mockedClearCompleted = vi.mocked(clearCompleted);
const mockedListJobs = vi.mocked(listJobs);

const initialJobsState = useJobsStore.getState();

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
  useJobsStore.setState(initialJobsState, true);
  mockedGetStatus.mockReset();
  mockedRescan.mockReset();
  mockedPauseWatcher.mockReset();
  mockedResumeWatcher.mockReset();
  mockedRetryAllFailed.mockReset();
  mockedClearCompleted.mockReset();
  mockedListJobs.mockReset();
  mockedListJobs.mockResolvedValue({ items: [], total: 0 });
  rescanProgressHandler = null;
});

describe("DashboardHeader", () => {
  it("calls pauseWatcher and flips the label to Retomar once watcher_paused is true", async () => {
    const user = userEvent.setup();
    mockedPauseWatcher.mockResolvedValue(undefined);
    mockedGetStatus.mockResolvedValue(status({ watcher_paused: true }));
    useStatusStore.setState({ status: status({ watcher_paused: false }) });

    render(<DashboardHeader />);

    await user.click(screen.getByRole("button", { name: /pausar watcher/i }));

    expect(mockedPauseWatcher).toHaveBeenCalledTimes(1);
    expect(await screen.findByRole("button", { name: /retomar watcher/i })).toBeInTheDocument();
  });

  /** A `RescanReport` with everything zero except the fields a test cares about. */
  function report(overrides: Partial<RescanReport> = {}): RescanReport {
    return {
      scanned: 0,
      enqueued: 0,
      unchanged: 0,
      skipped_filtered: 0,
      skipped_symlink: 0,
      skipped_unreadable: 0,
      errors: 0,
      archived: 0,
      restored: 0,
      ...overrides,
    };
  }

  it("calls rescan and shows '3 enfileirados' when it resolves 3", async () => {
    const user = userEvent.setup();
    mockedRescan.mockResolvedValue(report({ scanned: 3, enqueued: 3 }));
    mockedGetStatus.mockResolvedValue(status());
    useStatusStore.setState({ status: status() });

    render(<DashboardHeader />);

    await user.click(screen.getByRole("button", { name: /atualizar lista/i }));

    expect(mockedRescan).toHaveBeenCalledTimes(1);
    expect(await screen.findByText("3 enfileirados")).toBeInTheDocument();
  });

  // T-2.4: a first scan over a large folder hashes every file and can take
  // minutes -- a bare spinner is indistinguishable from a hang (the exact bug
  // report this exists to fix). While `rescan()` is in flight, `rescan-progress`
  // events must render a live "scanned/total" count, then disappear once the
  // scan settles (whatever result it settles with).
  it("shows a live scanned/total count while rescanning, and clears it once the scan settles", async () => {
    const user = userEvent.setup();
    mockedGetStatus.mockResolvedValue(status());
    useStatusStore.setState({ status: status() });

    let resolveRescan: (value: RescanReport) => void = () => {};
    mockedRescan.mockImplementation(
      () =>
        new Promise<RescanReport>((resolve) => {
          resolveRescan = resolve;
        }),
    );

    render(<DashboardHeader />);
    await user.click(screen.getByRole("button", { name: /atualizar lista/i }));

    // Nothing to show before the first event of this scan arrives.
    expect(screen.queryByText(/escaneando/i)).not.toBeInTheDocument();

    expect(rescanProgressHandler).not.toBeNull();
    rescanProgressHandler?.({ scanned: 4, total: 10 });
    expect(await screen.findByText("Escaneando 4/10…")).toBeInTheDocument();

    rescanProgressHandler?.({ scanned: 9, total: 10 });
    expect(await screen.findByText("Escaneando 9/10…")).toBeInTheDocument();

    resolveRescan(report({ scanned: 10, enqueued: 10 }));

    await waitFor(() => {
      expect(screen.queryByText(/escaneando/i)).not.toBeInTheDocument();
    });
    // The idle label is back and the completion count took over instead.
    expect(screen.getByRole("button", { name: /atualizar lista/i })).toBeInTheDocument();
    expect(await screen.findByText("10 enfileirados")).toBeInTheDocument();
  });

  // The reconciliation half: tightening a filter and hitting "Atualizar Lista"
  // has to say so, otherwise the screen looks unchanged — which is the exact
  // complaint that prompted this.
  it("reports archived and restored counts alongside the enqueued one", async () => {
    const user = userEvent.setup();
    mockedRescan.mockResolvedValue(report({ enqueued: 1, archived: 2, restored: 4 }));
    mockedGetStatus.mockResolvedValue(status());
    useStatusStore.setState({ status: status() });

    render(<DashboardHeader />);
    await user.click(screen.getByRole("button", { name: /atualizar lista/i }));

    expect(await screen.findByText("1 enfileirados")).toBeInTheDocument();
    expect(screen.getByText("2 removidos pelo filtro")).toBeInTheDocument();
    expect(screen.getByText("4 recuperados")).toBeInTheDocument();
  });

  // The reported bug: KPIs moved after "Atualizar Lista" but the table sat
  // still until the user touched the status filter, because only `handleRescan`
  // — alone among the three queue-mutating actions — never refetched the jobs.
  it("refetches the job list after a rescan, not just the status counters", async () => {
    const user = userEvent.setup();
    mockedRescan.mockResolvedValue(report({ enqueued: 0, archived: 3 }));
    mockedGetStatus.mockResolvedValue(status());
    useStatusStore.setState({ status: status() });

    render(<DashboardHeader />);
    mockedListJobs.mockClear();

    await user.click(screen.getByRole("button", { name: /atualizar lista/i }));

    expect(mockedListJobs).toHaveBeenCalledTimes(1);
  });

  // The other two queue-mutating actions must keep doing it too — the point of
  // the fix is that all three behave the same.
  it.each([
    [
      "reenviar falhas",
      () => {
        mockedRetryAllFailed.mockResolvedValue(1);
        // The button only renders when something has actually failed.
        useStatusStore.setState({ status: status({ counts_by_status: counts({ failed: 1 }) }) });
      },
    ],
    [
      "limpar concluídos",
      () => {
        mockedClearCompleted.mockResolvedValue(1);
        useStatusStore.setState({ status: status() });
      },
    ],
  ])("refetches the job list after %s", async (label, arrange) => {
    const user = userEvent.setup();
    mockedGetStatus.mockResolvedValue(status());
    arrange();

    render(<DashboardHeader />);
    mockedListJobs.mockClear();

    await user.click(screen.getByRole("button", { name: new RegExp(label, "i") }));

    expect(mockedListJobs).toHaveBeenCalledTimes(1);
  });

  // A plain rescan archives nothing; a permanent "0 removidos" would be noise.
  it("omits the archived and restored chips when both are zero", async () => {
    const user = userEvent.setup();
    mockedRescan.mockResolvedValue(report({ enqueued: 2 }));
    mockedGetStatus.mockResolvedValue(status());
    useStatusStore.setState({ status: status() });

    render(<DashboardHeader />);
    await user.click(screen.getByRole("button", { name: /atualizar lista/i }));

    expect(await screen.findByText("2 enfileirados")).toBeInTheDocument();
    expect(screen.queryByText(/removidos pelo filtro/)).not.toBeInTheDocument();
    expect(screen.queryByText(/recuperados/)).not.toBeInTheDocument();
  });

  // The full breakdown: a scan that skipped a permission-denied subtree (e.g.
  // Windows' `System Volume Information` on E:\) must not look identical to a
  // fully successful one — `scanned` always shows, `errors`/`skipped_unreadable`
  // only when nonzero (same zero-is-noise treatment as archived/restored).
  it("reports scanned, errors and skipped_unreadable alongside the enqueued count", async () => {
    const user = userEvent.setup();
    mockedRescan.mockResolvedValue(report({ scanned: 12, enqueued: 3, errors: 2, skipped_unreadable: 5 }));
    mockedGetStatus.mockResolvedValue(status());
    useStatusStore.setState({ status: status() });

    render(<DashboardHeader />);
    await user.click(screen.getByRole("button", { name: /atualizar lista/i }));

    expect(await screen.findByText("12 escaneados")).toBeInTheDocument();
    expect(screen.getByText("3 enfileirados")).toBeInTheDocument();
    expect(screen.getByText("2 erros")).toBeInTheDocument();
    expect(screen.getByText("5 pulados por permissão")).toBeInTheDocument();
  });

  // The reported bug: a rejected `rescan()` used to leave the button just
  // stop spinning with no trace of why — `AppError.message` carries the real
  // cause (an I/O error on the watched folder) and must reach the screen.
  it("shows the AppError message when rescan() rejects, and stops the spinner", async () => {
    const user = userEvent.setup();
    mockedRescan.mockRejectedValue({
      code: "rescan.error",
      message: "io error while scanning: Acesso negado. (os error 5)",
    });
    mockedGetStatus.mockResolvedValue(status());
    useStatusStore.setState({ status: status() });

    render(<DashboardHeader />);
    const rescanButton = screen.getByRole("button", { name: /atualizar lista/i });
    await user.click(rescanButton);

    expect(await screen.findByRole("alert")).toHaveTextContent("Acesso negado. (os error 5)");
    expect(rescanButton).not.toHaveAttribute("aria-busy", "true");
  });

  it("dismisses the rescan error banner when its dismiss button is clicked", async () => {
    const user = userEvent.setup();
    mockedRescan.mockRejectedValue({ code: "rescan.error", message: "io error: acesso negado" });
    mockedGetStatus.mockResolvedValue(status());
    useStatusStore.setState({ status: status() });

    render(<DashboardHeader />);
    await user.click(screen.getByRole("button", { name: /atualizar lista/i }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /dispensar erro de atualização/i }));

    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("calls clearCompleted and shows '2 arquivos arquivados' when it resolves 2", async () => {
    const user = userEvent.setup();
    mockedClearCompleted.mockResolvedValue(2);
    mockedGetStatus.mockResolvedValue(status());
    useStatusStore.setState({ status: status() });

    render(<DashboardHeader />);

    const clearButton = screen.getByRole("button", { name: /limpar concluídos/i });
    expect(clearButton).toBeEnabled();
    await user.click(clearButton);

    expect(mockedClearCompleted).toHaveBeenCalledTimes(1);
    expect(await screen.findByText("2 arquivos arquivados")).toBeInTheDocument();
  });

  it("hides Reenviar Falhas when there are no failed jobs", () => {
    useStatusStore.setState({ status: status({ counts_by_status: counts({ failed: 0 }) }) });
    render(<DashboardHeader />);

    expect(screen.queryByRole("button", { name: /reenviar falhas/i })).not.toBeInTheDocument();
  });

  it("shows Reenviar Falhas when kpis.failed > 0, calls retryAllFailed, and shows '4 reenviados'", async () => {
    const user = userEvent.setup();
    mockedRetryAllFailed.mockResolvedValue(4);
    mockedGetStatus.mockResolvedValue(status());
    useStatusStore.setState({ status: status({ counts_by_status: counts({ failed: 4 }) }) });

    render(<DashboardHeader />);

    const retryButton = screen.getByRole("button", { name: /reenviar falhas/i });
    await user.click(retryButton);

    expect(mockedRetryAllFailed).toHaveBeenCalledTimes(1);
    expect(await screen.findByText("4 reenviados")).toBeInTheDocument();
  });
});
