/**
 * JobTable — pagination + filter-reset + de-dup test (T-2.12; PRD.md RF-064,
 * RNF-008). Complements `JobTable.test.tsx` (row rendering/badges) with the
 * server-side paging contract: `limit/offset` math, filter change resetting
 * the page, no redundant `list_jobs` calls on a no-op re-render, and a single
 * `list_jobs` call when two "próxima" clicks land on the same resulting page
 * before the in-flight request resolves (`jobsStore`'s `_inFlightKey` guard).
 */
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/api/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/ipc")>();
  return { ...actual, listJobs: vi.fn() };
});

import { listJobs } from "@/api/ipc";
import { makeJob } from "@/store/__fixtures__/jobs";
import { useJobsStore } from "@/store/jobsStore";
import type { JobView, ListJobsPage } from "@/types/generated";

import { JobTable } from "../JobTable";

const mockedListJobs = vi.mocked(listJobs);
const initialState = useJobsStore.getState();

function fixturePage(): JobView[] {
  return Array.from({ length: 50 }, () => makeJob());
}

describe("JobTable pagination", () => {
  beforeEach(() => {
    useJobsStore.setState(initialState, true);
    mockedListJobs.mockReset();
    mockedListJobs.mockResolvedValue({ items: fixturePage(), total: 120 });
    mockIPC(() => undefined, { shouldMockEvents: true });
  });

  afterEach(() => {
    clearMocks();
  });

  it("fetches page 0 (offset 0, limit 50) on mount", async () => {
    render(<JobTable />);

    await waitFor(() =>
      expect(mockedListJobs).toHaveBeenCalledWith(expect.objectContaining({ offset: 0, limit: 50 })),
    );
    expect(await screen.findByText("1 / 3")).toBeInTheDocument();
  });

  it("'próxima' advances offset by 50 each click, disabling on the last page", async () => {
    const user = userEvent.setup();
    render(<JobTable />);
    await screen.findByText("1 / 3");

    const nextButton = screen.getByRole("button", { name: "Próxima página" });

    await user.click(nextButton);
    await waitFor(() =>
      expect(mockedListJobs).toHaveBeenLastCalledWith(expect.objectContaining({ offset: 50, limit: 50 })),
    );
    expect(await screen.findByText("2 / 3")).toBeInTheDocument();

    await user.click(nextButton);
    await waitFor(() =>
      expect(mockedListJobs).toHaveBeenLastCalledWith(expect.objectContaining({ offset: 100, limit: 50 })),
    );
    expect(await screen.findByText("3 / 3")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Próxima página" })).toBeDisabled();
  });

  it("switching to 'Concluídos' resets to page 0 and queries done+archived", async () => {
    const user = userEvent.setup();
    render(<JobTable />);
    await screen.findByText("1 / 3");

    // Move off page 0 first so the reset is observable.
    await user.click(screen.getByRole("button", { name: "Próxima página" }));
    await screen.findByText("2 / 3");

    await user.click(screen.getByRole("button", { name: "Concluídos" }));

    await waitFor(() =>
      expect(mockedListJobs).toHaveBeenLastCalledWith(
        expect.objectContaining({ offset: 0, statuses: ["done"], include_archived: true }),
      ),
    );
    expect(useJobsStore.getState().page).toBe(0);
    expect(await screen.findByText("1 / 3")).toBeInTheDocument();
  });

  it("re-clicking the already-active filter or re-rendering doesn't trigger an extra list_jobs call", async () => {
    const user = userEvent.setup();
    const { rerender } = render(<JobTable />);
    await screen.findByText("1 / 3");

    expect(mockedListJobs).toHaveBeenCalledTimes(1);

    // "Todos" is already the active filter — clicking it again is a no-op:
    // filter and page (0) are both unchanged, so the fetch effect shouldn't re-run.
    await user.click(screen.getByRole("button", { name: "Todos" }));
    expect(mockedListJobs).toHaveBeenCalledTimes(1);

    // A plain re-render (no store change) must not re-fetch either.
    rerender(<JobTable />);
    expect(mockedListJobs).toHaveBeenCalledTimes(1);
  });

  it("a second fetch() for the same {filter, page} while the first is in flight is skipped (RNF-008 de-dup)", async () => {
    let resolveFirst: (page: ListJobsPage) => void = () => undefined;
    mockedListJobs.mockReset();
    mockedListJobs.mockImplementationOnce(
      () =>
        new Promise<ListJobsPage>((resolve) => {
          resolveFirst = resolve;
        }),
    );

    // Simulates two "próxima" clicks landing on the identical resulting page
    // before the first request resolves — e.g. a real double-click, or
    // StrictMode's double mount-effect invocation (see the next test).
    const first = useJobsStore.getState().fetch();
    const second = useJobsStore.getState().fetch();

    expect(mockedListJobs).toHaveBeenCalledTimes(1);
    expect(useJobsStore.getState().loading).toBe(true);

    resolveFirst({ items: fixturePage(), total: 120 });
    await Promise.all([first, second]);

    expect(mockedListJobs).toHaveBeenCalledTimes(1);
    expect(useJobsStore.getState().loading).toBe(false);
    expect(useJobsStore.getState().total).toBe(120);
  });

  it("StrictMode's double effect-invocation on mount still fetches page 0 exactly once", async () => {
    const { StrictMode } = await import("react");
    render(
      <StrictMode>
        <JobTable />
      </StrictMode>,
    );

    await screen.findByText("1 / 3");

    // React 18 StrictMode mounts, runs effects, cleans them up and mounts
    // again — the mount effect calls `fetch()` twice back to back for the
    // identical {filter: "all", page: 0} while the first call is still
    // in flight. `jobsStore`'s `_inFlightKey` guard collapses the second.
    expect(mockedListJobs).toHaveBeenCalledTimes(1);
    expect(mockedListJobs).toHaveBeenCalledWith(expect.objectContaining({ offset: 0, limit: 50 }));
  });
});
