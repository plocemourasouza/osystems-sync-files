/**
 * JobRow row actions — integration test (PRD.md RF-033/035/065/098; PLAN.md
 * T-3.11). Mocks `@/api/ipc`'s row-action wrappers directly (the same
 * pattern `JobTable.test.tsx` uses for `listJobs`) and drives `RowActions`
 * through the rendered `JobRow`, since that's the only way a user reaches it.
 */
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/api/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/ipc")>();
  return {
    ...actual,
    retryJob: vi.fn(),
    cancelJob: vi.fn(),
    pauseJob: vi.fn(),
    resumeJob: vi.fn(),
    openInExplorer: vi.fn(),
    openRemote: vi.fn(),
  };
});

import { cancelJob, openInExplorer, openRemote, pauseJob, resumeJob, retryJob } from "@/api/ipc";
import { makeJob } from "@/store/__fixtures__/jobs";
import type { JobView } from "@/types/generated";

import { JobRow } from "../JobRow";

const mockedRetryJob = vi.mocked(retryJob);
const mockedCancelJob = vi.mocked(cancelJob);
const mockedPauseJob = vi.mocked(pauseJob);
const mockedResumeJob = vi.mocked(resumeJob);
const mockedOpenInExplorer = vi.mocked(openInExplorer);
const mockedOpenRemote = vi.mocked(openRemote);

function renderRow(job: JobView) {
  return render(
    <table>
      <tbody>
        <JobRow job={job} progress={{}} />
      </tbody>
    </table>,
  );
}

beforeEach(() => {
  mockedRetryJob.mockReset().mockResolvedValue(undefined);
  mockedCancelJob.mockReset().mockResolvedValue(undefined);
  mockedPauseJob.mockReset().mockResolvedValue(undefined);
  mockedResumeJob.mockReset().mockResolvedValue(undefined);
  mockedOpenInExplorer.mockReset().mockResolvedValue(undefined);
  mockedOpenRemote.mockReset().mockResolvedValue(undefined);
});

describe("JobRow actions", () => {
  it("does not render Reenviar when no side is failed/cancelled", () => {
    const job = makeJob({ gdrive: { status: "uploading" }, s3: { status: "pending" } });
    renderRow(job);

    expect(screen.queryByRole("button", { name: /reenviar/i })).not.toBeInTheDocument();
  });

  it("enables Reenviar for a failed side and calls retryJob with that side's job_id", async () => {
    const user = userEvent.setup();
    const job = makeJob({ gdrive: { status: "done" }, s3: { status: "failed" } });
    renderRow(job);

    const retryButton = screen.getByRole("button", { name: /reenviar/i });
    expect(retryButton).toBeEnabled();
    await user.click(retryButton);

    expect(mockedRetryJob).toHaveBeenCalledTimes(1);
    expect(mockedRetryJob).toHaveBeenCalledWith(job.s3.job_id);
  });

  it("enables Reenviar for a cancelled side and calls retryJob with that side's job_id", async () => {
    const user = userEvent.setup();
    const job = makeJob({ gdrive: { status: "cancelled" }, s3: { status: "done" } });
    renderRow(job);

    await user.click(screen.getByRole("button", { name: /reenviar/i }));

    expect(mockedRetryJob).toHaveBeenCalledWith(job.gdrive.job_id);
  });

  it("enables Cancelar for an active side (pending/uploading/paused) and calls cancelJob", async () => {
    const user = userEvent.setup();
    const job = makeJob({ gdrive: { status: "uploading" }, s3: { status: "done" } });
    renderRow(job);

    const cancelButton = screen.getByRole("button", { name: /^cancelar$/i });
    expect(cancelButton).toBeEnabled();
    await user.click(cancelButton);

    expect(mockedCancelJob).toHaveBeenCalledWith(job.gdrive.job_id);
  });

  it("does not render Cancelar when no side is pending/uploading/paused", () => {
    const job = makeJob({ gdrive: { status: "done" }, s3: { status: "failed" } });
    renderRow(job);

    expect(screen.queryByRole("button", { name: /^cancelar$/i })).not.toBeInTheDocument();
  });

  it("enables Pausar for a pending/uploading side and calls pauseJob with that side's job_id", async () => {
    const user = userEvent.setup();
    const job = makeJob({ gdrive: { status: "uploading" }, s3: { status: "done" } });
    renderRow(job);

    const pauseButton = screen.getByRole("button", { name: /^pausar$/i });
    expect(pauseButton).toBeEnabled();
    await user.click(pauseButton);

    expect(mockedPauseJob).toHaveBeenCalledTimes(1);
    expect(mockedPauseJob).toHaveBeenCalledWith(job.gdrive.job_id);
  });

  it("does not render Pausar when no side is pending/uploading", () => {
    const job = makeJob({ gdrive: { status: "paused" }, s3: { status: "done" } });
    renderRow(job);

    expect(screen.queryByRole("button", { name: /^pausar$/i })).not.toBeInTheDocument();
  });

  it("enables Retomar for a paused side and calls resumeJob with that side's job_id", async () => {
    const user = userEvent.setup();
    const job = makeJob({ gdrive: { status: "paused" }, s3: { status: "done" } });
    renderRow(job);

    const resumeButton = screen.getByRole("button", { name: /^retomar$/i });
    expect(resumeButton).toBeEnabled();
    await user.click(resumeButton);

    expect(mockedResumeJob).toHaveBeenCalledTimes(1);
    expect(mockedResumeJob).toHaveBeenCalledWith(job.gdrive.job_id);
  });

  it("does not render Retomar when no side is paused", () => {
    const job = makeJob({ gdrive: { status: "uploading" }, s3: { status: "done" } });
    renderRow(job);

    expect(screen.queryByRole("button", { name: /^retomar$/i })).not.toBeInTheDocument();
  });

  it("Abrir no Explorer is always enabled and calls openInExplorer with the job's path", async () => {
    const user = userEvent.setup();
    const job = makeJob({ gdrive: { status: "pending" }, s3: { status: "pending" } });
    renderRow(job);

    await user.click(screen.getByRole("button", { name: /abrir no explorer/i }));

    expect(mockedOpenInExplorer).toHaveBeenCalledWith(job.path);
  });

  it("does not render Abrir remoto when nothing is done", () => {
    const job = makeJob({ gdrive: { status: "pending" }, s3: { status: "uploading" } });
    renderRow(job);

    expect(screen.queryByRole("button", { name: /abrir remoto/i })).not.toBeInTheDocument();
  });

  it("renders Abrir remoto and calls openRemote when a side is done", async () => {
    const user = userEvent.setup();
    const job = makeJob({ gdrive: { status: "pending" }, s3: { status: "done" } });
    renderRow(job);

    const openRemoteButton = screen.getByRole("button", { name: /abrir remoto/i });
    expect(openRemoteButton).toBeEnabled();

    await user.click(openRemoteButton);

    expect(mockedOpenRemote).toHaveBeenCalledWith(job.s3.job_id);
  });

  it("names the destination on each Abrir remoto button when both sides are done", () => {
    const job = makeJob({ gdrive: { status: "done" }, s3: { status: "done" } });
    renderRow(job);

    expect(screen.getByRole("button", { name: /abrir no google drive/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /abrir no aws s3/i })).toBeInTheDocument();
  });

  it("shows the failed side's last_error in the details dialog", async () => {
    const user = userEvent.setup();
    const job = makeJob({
      gdrive: { status: "done" },
      s3: { status: "failed", last_error: "403 Forbidden: Access Denied" },
    });
    renderRow(job);

    await user.click(screen.getByRole("button", { name: /detalhes do erro/i }));

    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText("403 Forbidden: Access Denied")).toBeInTheDocument();
  });

  it("disables the other buttons while one action is busy", async () => {
    const user = userEvent.setup();
    let resolveRetry: (() => void) | undefined;
    mockedRetryJob.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          resolveRetry = resolve;
        }),
    );
    const job = makeJob({ gdrive: { status: "done" }, s3: { status: "failed" } });
    renderRow(job);

    await user.click(screen.getByRole("button", { name: /reenviar/i }));

    expect(screen.getByRole("button", { name: /abrir no explorer/i })).toBeDisabled();
    expect(screen.getByRole("button", { name: /copiar caminho/i })).toBeDisabled();
    expect(screen.queryByRole("button", { name: /^cancelar$/i })).not.toBeInTheDocument();

    resolveRetry?.();
    await waitFor(() => expect(screen.getByRole("button", { name: /abrir no explorer/i })).toBeEnabled());
  });

  it("shows a failed action's error message inline for a few seconds", async () => {
    const user = userEvent.setup();
    mockedOpenInExplorer.mockRejectedValue(new Error("boom"));
    const job = makeJob({ gdrive: { status: "pending" }, s3: { status: "pending" } });
    renderRow(job);

    await user.click(screen.getByRole("button", { name: /abrir no explorer/i }));

    expect(await screen.findByRole("alert")).toBeInTheDocument();
  });

  it("renders every applicable action inline, with no ⋯ menu left to open", async () => {
    const user = userEvent.setup();
    // gdrive=uploading -> cancellable + pausable; s3=paused -> cancellable + resumable.
    // The union (cancel/pause/resume) used to overflow the 2-inline cap into the menu.
    const job = makeJob({ gdrive: { status: "uploading" }, s3: { status: "paused" } });
    renderRow(job);

    // Explorer + Copiar caminho + Pausar + Retomar + Cancelar.
    expect(screen.getAllByRole("button")).toHaveLength(5);
    expect(screen.queryByRole("button", { name: /mais opções/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();

    expect(screen.getByRole("button", { name: /abrir no explorer/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /copiar caminho/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^cancelar$/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^pausar$/i })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /^retomar$/i }));
    expect(mockedResumeJob).toHaveBeenCalledWith(job.s3.job_id);
  });

  it("describes each action button with a tooltip portalled out of the table", async () => {
    const job = makeJob({ gdrive: { status: "pending" }, s3: { status: "uploading" } });
    const { container } = renderRow(job);

    const explorerButton = screen.getByRole("button", { name: /abrir no explorer/i });
    expect(explorerButton).not.toHaveAttribute("aria-describedby");

    // Focus (keyboard) shows the hint without waiting out the hover dwell.
    explorerButton.focus();

    const tooltip = await screen.findByRole("tooltip");
    expect(tooltip).toHaveTextContent(/abrir no explorer/i);
    expect(explorerButton).toHaveAttribute("aria-describedby", tooltip.id);

    // Portalled, so `JobTable`'s `overflow-x-auto` wrapper cannot clip it.
    expect(container.contains(tooltip)).toBe(false);
    expect(document.body.contains(tooltip)).toBe(true);

    explorerButton.blur();
    await waitFor(() => expect(screen.queryByRole("tooltip")).not.toBeInTheDocument());
  });

  it("flips the tooltip below the button when there is no room above it", async () => {
    const rectSpy = vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue({
      top: 4,
      bottom: 28,
      left: 100,
      right: 124,
      width: 24,
      height: 24,
      x: 100,
      y: 4,
      toJSON: () => ({}),
    } as DOMRect);
    const offsetHeightSpy = vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(200);

    try {
      const job = makeJob({ gdrive: { status: "pending" }, s3: { status: "uploading" } });
      renderRow(job);

      screen.getByRole("button", { name: /abrir no explorer/i }).focus();
      const tooltip = await screen.findByRole("tooltip");

      // 4 - 200 - 4 < 0, so it lands under the anchor's bottom edge instead.
      expect(Number.parseFloat(tooltip.style.top)).toBeGreaterThan(28);
    } finally {
      rectSpy.mockRestore();
      offsetHeightSpy.mockRestore();
    }
  });
});
