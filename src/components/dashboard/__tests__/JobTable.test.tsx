/**
 * JobTable — integration test (PRD.md RF-062–RF-065). Mocks `@/api/ipc.listJobs`
 * with 3 fixture jobs (uploading/pending, done/done, done/failed) and drives
 * `jobsStore` directly (`applyProgress`, `setFilter` via the UI) rather than
 * emitting real Tauri events — `mockIPC` only needs to keep `useTauriEvent`'s
 * internal `listen()` calls from rejecting in jsdom.
 */
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/api/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/ipc")>();
  return { ...actual, listJobs: vi.fn() };
});

import { listJobs } from "@/api/ipc";
import { makeJob } from "@/store/__fixtures__/jobs";
import { useJobsStore } from "@/store/jobsStore";
import type { JobView } from "@/types/generated";

import { JobTable } from "../JobTable";

const mockedListJobs = vi.mocked(listJobs);
const initialState = useJobsStore.getState();

function buildFixtureJobs(): JobView[] {
  const uploading = makeJob({
    name: "relatorio.pdf",
    gdrive: { status: "uploading" },
    s3: { status: "pending" },
  });
  const done = makeJob({
    name: "planilha.xlsx",
    gdrive: { status: "done" },
    s3: { status: "done" },
  });
  const failed = makeJob({
    name: "backup.zip",
    gdrive: { status: "done" },
    s3: { status: "failed", last_error: "403 Forbidden: Access Denied" },
  });
  return [uploading, done, failed];
}

describe("JobTable", () => {
  let jobs: JobView[];

  beforeEach(() => {
    useJobsStore.setState(initialState, true);
    mockedListJobs.mockReset();
    jobs = buildFixtureJobs();
    mockedListJobs.mockResolvedValue({ items: jobs, total: 120 });
    mockIPC(() => undefined, { shouldMockEvents: true });
  });

  afterEach(() => {
    clearMocks();
  });

  it("fetches on mount and renders one row per job with aggregated status badges", async () => {
    render(<JobTable />);

    await screen.findByText("Enviando"); // waits past the loading skeleton
    const rows = screen.getAllByRole("row");
    expect(rows).toHaveLength(4); // header + 3 data rows

    expect(screen.getByText("Enviando")).toBeInTheDocument();
    expect(screen.getByText("Sincronizado")).toBeInTheDocument();
    expect(screen.getByText("Falha")).toBeInTheDocument();
  });

  it("reflects an applyProgress() update in the uploading job's Drive cell", async () => {
    render(<JobTable />);
    await screen.findByText("Enviando");

    await act(async () => {
      useJobsStore.getState().applyProgress({
        job_id: jobs[0]!.gdrive.job_id,
        sent: 74,
        total: 100,
        rate_bps: 1_887_436,
      });
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    expect(screen.getByText("74%")).toBeInTheDocument();
  });

  it("shows the failed side's error text in the S3 cell", async () => {
    render(<JobTable />);
    await screen.findByText("Falha");

    expect(screen.getByText(/403 Forbidden/)).toBeInTheDocument();
  });

  it("shows page 1 of 3 for total=120 (pageSize 50)", async () => {
    render(<JobTable />);
    expect(await screen.findByText("1 / 3")).toBeInTheDocument();
  });

  it("clicking 'Falhas' filters server-side and resets to page 0", async () => {
    const user = userEvent.setup();
    render(<JobTable />);
    await screen.findByText("Enviando");

    await user.click(screen.getByRole("button", { name: "Falhas" }));

    await waitFor(() => {
      expect(mockedListJobs).toHaveBeenLastCalledWith(
        expect.objectContaining({ statuses: ["failed"], offset: 0 }),
      );
    });
    expect(useJobsStore.getState().page).toBe(0);
  });

  // Layout contract with Dashboard/LogConsole: the rows are the page's only
  // flexible region, so this section must claim the leftover height and
  // scroll its own body. Without `flex-1`/`min-h-0` here the page flows
  // freely again and collapsing the console just shortens the page instead
  // of handing its space to the list. jsdom does no layout, so the classes
  // are what is asserted.
  it("claims the leftover height and scrolls the rows in its own body", async () => {
    const { container } = render(<JobTable />);
    await screen.findByText("Enviando");

    const section = container.querySelector("section");
    expect(section).toHaveClass("flex-1", "min-h-[200px]", "flex-col");

    const scroller = screen.getByRole("table").parentElement;
    expect(scroller).toHaveClass("flex-1", "min-h-0", "overflow-auto");
  });

  // Placement, not just presence: the badge captions the list, so it belongs
  // at the far end of the same toolbar row as the filter bar.
  it("puts the watcher badge opposite the filter bar on the toolbar row", async () => {
    render(<JobTable />);
    await screen.findByText("Enviando");

    const filterBar = screen.getByRole("group", { name: "Filtro de status" });
    const badge = screen.getByText(/Watcher/);
    const row = filterBar.parentElement;

    expect(row).toContainElement(badge);
    expect(row).toHaveClass("justify-between");
  });

  // The header must survive that vertical scroll.
  it("keeps the header row sticky at the top of the scroll body", async () => {
    render(<JobTable />);
    await screen.findByText("Enviando");

    const headerRow = screen.getByRole("table").querySelector("thead tr");
    expect(headerRow?.className).toContain("[&>th]:sticky");
    expect(headerRow?.className).toContain("[&>th]:top-0");
  });
});
