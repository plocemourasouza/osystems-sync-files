import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";

vi.mock("@/api/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/ipc")>();
  return {
    ...actual,
    getRecentLogs: vi.fn(),
    openLogsFolder: vi.fn(),
  };
});

import { getRecentLogs, openLogsFolder } from "@/api/ipc";
import { useLogStore } from "@/store/logStore";
import type { LogLine } from "@/types/generated";

import { LogConsole } from "../LogConsole";

const mockedGetRecentLogs = vi.mocked(getRecentLogs);
const mockedOpenLogsFolder = vi.mocked(openLogsFolder);

const initialState = useLogStore.getState();

function line(overrides: Partial<LogLine> = {}): LogLine {
  return {
    ts: "2026-01-01T12:00:00.000Z",
    level: "INFO",
    target: "osystems_sync_core::watcher",
    job_id: null,
    destination: null,
    message: "hello",
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

/** All `[data-level]` rows currently mounted — cheaper than matching on message text. */
function rows(container: HTMLElement): NodeListOf<Element> {
  return container.querySelectorAll("[data-level]");
}

beforeEach(() => {
  useLogStore.setState(initialState, true);
  mockedGetRecentLogs.mockReset().mockResolvedValue([]);
  mockedOpenLogsFolder.mockReset().mockResolvedValue(undefined);
  // `useTauriEvent("log-line", push)` subscribes via `@tauri-apps/api/event`'s
  // `listen`, which round-trips through Tauri's IPC transport even though we
  // never emit an event in these tests — without this it rejects immediately.
  mockIPC(() => undefined, { shouldMockEvents: true });
});

afterEach(() => {
  clearMocks();
  vi.unstubAllGlobals();
});

describe("LogConsole", () => {
  it("hydrates via get_recent_logs on mount and renders one row per line", async () => {
    mockedGetRecentLogs.mockResolvedValue([
      line({ message: "first" }),
      line({ message: "second" }),
      line({ message: "third" }),
    ]);

    const { container } = render(<LogConsole />);

    await waitFor(() => {
      expect(rows(container)).toHaveLength(3);
    });
    expect(screen.getByText("first")).toBeInTheDocument();
    expect(screen.getByText("second")).toBeInTheDocument();
    expect(screen.getByText("third")).toBeInTheDocument();
  });

  it("push()ing an error line adds a row and colors its message with the error tone", async () => {
    const { container } = render(<LogConsole />);
    await waitFor(() => expect(mockedGetRecentLogs).toHaveBeenCalled());

    useLogStore.getState().push(line({ level: "ERROR", message: "boom" }));

    await waitFor(() => {
      expect(rows(container)).toHaveLength(1);
    });
    expect(screen.getByText("boom")).toHaveClass("text-error");
  });

  it("push()ing a line with an error field renders the error detail alongside the message", async () => {
    const { container } = render(<LogConsole />);
    await waitFor(() => expect(mockedGetRecentLogs).toHaveBeenCalled());

    useLogStore.getState().push(
      line({
        level: "WARN",
        message: "varredura manual falhou",
        error: "disk full",
        path: "/tmp/inbox/report.pdf",
      }),
    );

    await waitFor(() => {
      expect(rows(container)).toHaveLength(1);
    });
    expect(screen.getByText("varredura manual falhou")).toBeInTheDocument();
    expect(screen.getByText("disk full — /tmp/inbox/report.pdf")).toBeInTheDocument();
  });

  it("selecting the error level filter shows only error lines", async () => {
    const user = userEvent.setup();
    mockedGetRecentLogs.mockResolvedValue([
      line({ level: "INFO", message: "info-line" }),
      line({ level: "WARN", message: "warn-line" }),
      line({ level: "ERROR", message: "error-line" }),
    ]);
    const { container } = render(<LogConsole />);

    await waitFor(() => {
      expect(rows(container)).toHaveLength(3);
    });

    await user.selectOptions(screen.getByLabelText("Nível"), "error");

    await waitFor(() => {
      expect(rows(container)).toHaveLength(1);
    });
    expect(screen.getByText("error-line")).toBeInTheDocument();
  });

  it("toggling collapse hides the console body and persists via localStorage", async () => {
    const storage = localStorageMock();
    vi.stubGlobal("localStorage", storage);
    const user = userEvent.setup();
    mockedGetRecentLogs.mockResolvedValue([line()]);

    render(<LogConsole />);

    await waitFor(() => {
      expect(screen.getByRole("log")).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: "Recolher console de eventos" }));

    expect(screen.queryByRole("log")).not.toBeInTheDocument();
    expect(storage.getItem("osystems-sync.console.collapsed")).toBe("true");
  });

  it('clicking "Abrir pasta de logs" calls openLogsFolder', async () => {
    const user = userEvent.setup();
    render(<LogConsole />);
    await waitFor(() => expect(mockedGetRecentLogs).toHaveBeenCalled());

    await user.click(screen.getByRole("button", { name: "Abrir pasta de logs" }));

    expect(mockedOpenLogsFolder).toHaveBeenCalledTimes(1);
  });

  it("renders the header level label visually hidden (sr-only) so the header stays one row", async () => {
    render(<LogConsole />);
    await waitFor(() => expect(mockedGetRecentLogs).toHaveBeenCalled());

    const label = screen.getByText("Nível");
    expect(label.tagName).toBe("LABEL");
    expect(label).toHaveClass("sr-only");
    expect(screen.getByLabelText("Nível")).toBe(screen.getByRole("combobox"));
  });

  it("caps rendered rows at 200 for 600 pushed lines, while the count label reflects the store's 500-line ring cap", async () => {
    const { container } = render(<LogConsole />);
    await waitFor(() => expect(mockedGetRecentLogs).toHaveBeenCalled());

    for (let i = 0; i < 600; i += 1) {
      useLogStore.getState().push(line({ message: `line-${i}` }));
    }

    await waitFor(() => {
      expect(screen.getByText("500 linhas")).toBeInTheDocument();
    });
    expect(rows(container).length).toBeLessThanOrEqual(200);
  });

  // The collapsed console is a live indicator, not a dead bar: the newest
  // line rides in its header so the daemon stays observable while the body
  // is shut and the job table has the space.
  describe("collapsed ticker", () => {
    async function renderCollapsed(lines: LogLine[]): Promise<void> {
      const user = userEvent.setup();
      mockedGetRecentLogs.mockResolvedValue(lines);

      render(<LogConsole />);
      await waitFor(() => expect(screen.getByRole("log")).toBeInTheDocument());
      await user.click(screen.getByRole("button", { name: "Recolher console de eventos" }));
    }

    it("shows the newest line — the last one, matching the body's scroll target", async () => {
      await renderCollapsed([
        line({ ts: "2026-01-01T12:00:00.000Z", message: "oldest" }),
        line({ ts: "2026-01-01T12:00:05.000Z", message: "newest" }),
      ]);

      const ticker = screen.getByTestId("log-ticker");
      expect(ticker).toHaveTextContent("newest");
      expect(ticker).not.toHaveTextContent("oldest");
    });

    it("is absent while expanded — the body already ends on that line", async () => {
      mockedGetRecentLogs.mockResolvedValue([line({ message: "visible" })]);

      render(<LogConsole />);
      await waitFor(() => expect(screen.getByRole("log")).toBeInTheDocument());

      expect(screen.queryByTestId("log-ticker")).not.toBeInTheDocument();
    });

    it("advances as new lines arrive while still collapsed", async () => {
      await renderCollapsed([line({ ts: "2026-01-01T12:00:00.000Z", message: "first" })]);
      expect(screen.getByTestId("log-ticker")).toHaveTextContent("first");

      act(() => {
        useLogStore.getState().push(line({ ts: "2026-01-01T12:00:09.000Z", message: "second" }));
      });

      await waitFor(() => {
        expect(screen.getByTestId("log-ticker")).toHaveTextContent("second");
      });
      expect(screen.queryByRole("log")).not.toBeInTheDocument();
    });

    it("respects the level filter, like the body does", async () => {
      await renderCollapsed([
        line({ ts: "2026-01-01T12:00:00.000Z", level: "ERROR", message: "boom" }),
        line({ ts: "2026-01-01T12:00:05.000Z", level: "INFO", message: "chatter" }),
      ]);
      expect(screen.getByTestId("log-ticker")).toHaveTextContent("chatter");

      act(() => {
        useLogStore.getState().setLevel("error");
      });

      await waitFor(() => {
        expect(screen.getByTestId("log-ticker")).toHaveTextContent("boom");
      });
    });

    it("renders nothing rather than an empty bar when there are no lines", async () => {
      await renderCollapsed([line()]);
      act(() => {
        useLogStore.getState().hydrate([]);
      });

      await waitFor(() => {
        expect(screen.queryByTestId("log-ticker")).not.toBeInTheDocument();
      });
    });
  });
});
