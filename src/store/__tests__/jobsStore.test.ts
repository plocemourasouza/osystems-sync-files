import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { JobStatus, JobView, ListJobsPage } from "@/types/generated";
import { makeJob } from "@/store/__fixtures__/jobs";

vi.mock("@/api/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/ipc")>();
  return {
    ...actual,
    listJobs: vi.fn(),
  };
});

import { listJobs } from "@/api/ipc";
import { aggregateStatus, buildJobsQuery, JOBS_PAGE_SIZE, useJobsStore } from "@/store/jobsStore";

const mockedListJobs = vi.mocked(listJobs);

const initialState = useJobsStore.getState();

beforeEach(() => {
  useJobsStore.setState(initialState, true);
  mockedListJobs.mockReset();
});

describe("buildJobsQuery() (RF-064 filter -> list_jobs query)", () => {
  it("'all' has no status filter and archived jobs excluded", () => {
    expect(buildJobsQuery("all", 0)).toEqual({
      statuses: null,
      destination: null,
      include_archived: false,
      limit: JOBS_PAGE_SIZE,
      offset: 0,
    });
  });

  it("'active' filters to pending/uploading/paused", () => {
    expect(buildJobsQuery("active", 0)).toMatchObject({
      statuses: ["pending", "uploading", "paused"],
      include_archived: false,
    });
  });

  it("'done' filters to done and includes archived jobs", () => {
    expect(buildJobsQuery("done", 0)).toMatchObject({
      statuses: ["done"],
      include_archived: true,
    });
  });

  it("'failed' filters to failed, archived excluded", () => {
    expect(buildJobsQuery("failed", 0)).toMatchObject({
      statuses: ["failed"],
      include_archived: false,
    });
  });

  it("computes offset from the 0-based page and the fixed page size", () => {
    expect(buildJobsQuery("all", 3)).toMatchObject({ limit: JOBS_PAGE_SIZE, offset: 3 * JOBS_PAGE_SIZE });
  });
});

describe("jobsStore.setFilter()", () => {
  it("changes the filter and resets page to 0", () => {
    useJobsStore.getState().setPage(4);

    useJobsStore.getState().setFilter("failed");

    const state = useJobsStore.getState();
    expect(state.filter).toBe("failed");
    expect(state.page).toBe(0);
  });
});

describe("jobsStore.fetch()", () => {
  it("builds the query from current filter/page and stores items/total", async () => {
    useJobsStore.getState().setFilter("active");
    const job = makeJob();
    const page: ListJobsPage = { items: [job], total: 1 };
    mockedListJobs.mockResolvedValueOnce(page);

    await useJobsStore.getState().fetch();

    expect(mockedListJobs).toHaveBeenCalledWith(buildJobsQuery("active", 0));
    const state = useJobsStore.getState();
    expect(state.items).toEqual([job]);
    expect(state.total).toBe(1);
    expect(state.loading).toBe(false);
    expect(state.error).toBeNull();
  });

  it("sets an error message on rejection", async () => {
    mockedListJobs.mockRejectedValueOnce(new Error("db locked"));

    await useJobsStore.getState().fetch();

    const state = useJobsStore.getState();
    expect(state.error).toBe("db locked");
    expect(state.loading).toBe(false);
  });

  // RNF-008: rapid clicks landing on the same filter/page must not double up.
  it("skips a duplicate request for the same filter/page while one is in flight", async () => {
    let release: (page: ListJobsPage) => void = () => {};
    mockedListJobs.mockReturnValueOnce(
      new Promise<ListJobsPage>((resolve) => {
        release = resolve;
      }),
    );

    const first = useJobsStore.getState().fetch();
    await useJobsStore.getState().fetch();

    expect(mockedListJobs).toHaveBeenCalledTimes(1);
    release({ items: [], total: 0 });
    await first;
  });

  // ...but after a rescan the identical query returns different rows, so the
  // repeat is exactly what the caller wants. This is the bug that made
  // "Atualizar Lista" leave the table untouched.
  it("issues the request anyway when force is set", async () => {
    let release: (page: ListJobsPage) => void = () => {};
    mockedListJobs.mockReturnValueOnce(
      new Promise<ListJobsPage>((resolve) => {
        release = resolve;
      }),
    );
    mockedListJobs.mockResolvedValueOnce({ items: [makeJob()], total: 1 });

    const first = useJobsStore.getState().fetch();
    await useJobsStore.getState().fetch({ force: true });

    expect(mockedListJobs).toHaveBeenCalledTimes(2);
    expect(useJobsStore.getState().total).toBe(1);
    release({ items: [], total: 0 });
    await first;
  });

  // With two requests in flight, the older one must not land last and paint
  // pre-rescan rows over the fresh ones.
  it("drops a stale response that resolves after a newer request", async () => {
    const stale = makeJob({ name: "before-rescan.pdf" });
    const fresh = makeJob({ name: "after-rescan.pdf" });

    let releaseStale: (page: ListJobsPage) => void = () => {};
    mockedListJobs.mockReturnValueOnce(
      new Promise<ListJobsPage>((resolve) => {
        releaseStale = resolve;
      }),
    );
    mockedListJobs.mockResolvedValueOnce({ items: [fresh], total: 1 });

    const first = useJobsStore.getState().fetch();
    await useJobsStore.getState().fetch({ force: true });
    expect(useJobsStore.getState().items).toEqual([fresh]);

    releaseStale({ items: [stale], total: 99 });
    await first;

    expect(useJobsStore.getState().items).toEqual([fresh]);
    expect(useJobsStore.getState().total).toBe(1);
  });
});

describe("jobsStore.upsertFromEvent()", () => {
  it("replaces the row sharing file_id when it's on the current page", () => {
    const original = makeJob({ name: "a.pdf" });
    const other = makeJob({ name: "b.pdf" });
    useJobsStore.setState({ items: [original, other] });

    const updated: JobView = { ...original, name: "a-renamed.pdf" };
    useJobsStore.getState().upsertFromEvent(updated);

    const state = useJobsStore.getState();
    expect(state.items).toHaveLength(2);
    expect(state.items[0]).toEqual(updated);
    expect(state.items[1]).toEqual(other);
  });

  it("keeps a row in place even if the update would no longer match the active filter (no flicker)", () => {
    const job = makeJob({ gdrive: { status: "failed" }, s3: { status: "failed" } });
    useJobsStore.setState({ filter: "failed", page: 0, items: [job] });

    const nowDone: JobView = { ...job, gdrive: { ...job.gdrive, status: "done" }, s3: { ...job.s3, status: "done" } };
    useJobsStore.getState().upsertFromEvent(nowDone);

    expect(useJobsStore.getState().items).toEqual([nowDone]);
  });

  it("prepends a genuinely new row on page 0 of 'all' and trims to pageSize", () => {
    useJobsStore.setState({ filter: "all", page: 0, pageSize: 2, items: [makeJob(), makeJob()] });
    const newJob = makeJob({ name: "new.pdf" });

    useJobsStore.getState().upsertFromEvent(newJob);

    const state = useJobsStore.getState();
    expect(state.items).toHaveLength(2);
    expect(state.items[0]).toEqual(newJob);
  });

  it("prepends on 'active' page 0 too", () => {
    useJobsStore.setState({ filter: "active", page: 0, pageSize: 50, items: [] });
    const newJob = makeJob();

    useJobsStore.getState().upsertFromEvent(newJob);

    expect(useJobsStore.getState().items).toEqual([newJob]);
  });

  it("does not insert a new row when filter is 'done' or page isn't 0", () => {
    useJobsStore.setState({ filter: "done", page: 0, items: [] });
    useJobsStore.getState().upsertFromEvent(makeJob());
    expect(useJobsStore.getState().items).toEqual([]);

    useJobsStore.setState({ filter: "all", page: 1, items: [] });
    useJobsStore.getState().upsertFromEvent(makeJob());
    expect(useJobsStore.getState().items).toEqual([]);
  });
});

describe("jobsStore.applyProgress() (RNF-008 throttling)", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.runOnlyPendingTimers();
    vi.useRealTimers();
  });

  it("coalesces multiple synchronous updates into a single flush", () => {
    const store = useJobsStore.getState();

    store.applyProgress({ job_id: "job-1", sent: 10, total: 100, rate_bps: 1000 });
    store.applyProgress({ job_id: "job-2", sent: 20, total: 200, rate_bps: 2000 });
    store.applyProgress({ job_id: "job-1", sent: 15, total: 100, rate_bps: 1500 });

    // Not applied yet: still batched, pre-flush.
    expect(useJobsStore.getState().progress).toEqual({});

    vi.runAllTimers();

    const progress = useJobsStore.getState().progress;
    expect(progress["job-1"]).toEqual({ job_id: "job-1", sent: 15, total: 100, rate_bps: 1500 });
    expect(progress["job-2"]).toEqual({ job_id: "job-2", sent: 20, total: 200, rate_bps: 2000 });
  });

  it("a hundred events scheduled synchronously still produce exactly one flush", () => {
    const store = useJobsStore.getState();
    for (let i = 0; i < 100; i += 1) {
      store.applyProgress({ job_id: "job-1", sent: i, total: 100, rate_bps: 1000 });
    }

    expect(vi.getTimerCount()).toBe(1);

    vi.runAllTimers();

    expect(useJobsStore.getState().progress["job-1"]).toEqual({ job_id: "job-1", sent: 99, total: 100, rate_bps: 1000 });
  });
});

describe("aggregateStatus() (RF-063)", () => {
  const cases: Array<[JobStatus, JobStatus, ReturnType<typeof aggregateStatus>]> = [
    ["failed", "done", "failed"],
    ["done", "failed", "failed"],
    ["uploading", "pending", "uploading"],
    ["pending", "uploading", "uploading"],
    ["paused", "pending", "paused"],
    ["done", "done", "done"],
    ["cancelled", "done", "cancelled"],
    ["cancelled", "pending", "queued"],
  ];

  it.each(cases)("gdrive=%s, s3=%s -> %s", (gdriveStatus, s3Status, expected) => {
    const job = makeJob({ gdrive: { status: gdriveStatus }, s3: { status: s3Status } });

    expect(aggregateStatus(job)).toBe(expected);
  });

  it("both cancelled, neither active -> cancelled", () => {
    const job = makeJob({ gdrive: { status: "cancelled" }, s3: { status: "cancelled" } });
    expect(aggregateStatus(job)).toBe("cancelled");
  });

  it("both pending -> queued", () => {
    const job = makeJob({ gdrive: { status: "pending" }, s3: { status: "pending" } });
    expect(aggregateStatus(job)).toBe("queued");
  });
});
