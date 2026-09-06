import { afterEach, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import type { AppStatus, StatusCounts } from "@/types/generated";
import { WatcherStatusBadge } from "@/components/dashboard/WatcherStatusBadge";
import { useStatusStore } from "@/store/statusStore";

const initialStatusState = useStatusStore.getState();

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
    watching_path: "C:/monitoramento",
    counts_by_status: counts(),
    destinations: null,
    ...overrides,
  } as AppStatus;
}

afterEach(() => {
  useStatusStore.setState(initialStatusState, true);
});

describe("WatcherStatusBadge", () => {
  it("reports the number of files being watched while active", () => {
    // `selectKpis().detected` halves the per-status totals: one file is two
    // jobs (s3 + gdrive).
    useStatusStore.setState({ status: status({ counts_by_status: counts({ pending: 6 }) }) });

    render(<WatcherStatusBadge />);

    expect(screen.getByText("Watcher Ativo • Monitorando 3 arquivos")).toBeInTheDocument();
  });

  it("drops the count and says paused when the watcher is paused", () => {
    useStatusStore.setState({
      status: status({ watcher_paused: true, counts_by_status: counts({ pending: 6 }) }),
    });

    render(<WatcherStatusBadge />);

    expect(screen.getByText("Watcher Pausado")).toBeInTheDocument();
    expect(screen.queryByText(/Monitorando/)).not.toBeInTheDocument();
  });

  it("renders a zero count rather than nothing before the first status arrives", () => {
    useStatusStore.setState({ status: null });

    render(<WatcherStatusBadge />);

    expect(screen.getByText("Watcher Ativo • Monitorando 0 arquivos")).toBeInTheDocument();
  });

  // The count changes as files are detected; a screen reader should hear it
  // without the user hunting for it.
  it("announces changes politely", () => {
    useStatusStore.setState({ status: status() });

    const { container } = render(<WatcherStatusBadge />);

    expect(container.querySelector('[aria-live="polite"]')).toBeInTheDocument();
  });
});
