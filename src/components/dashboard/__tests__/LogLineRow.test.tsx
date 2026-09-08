import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import type { LogLine } from "@/types/generated";

import { detailText, LogLineRow } from "../LogLineRow";

function line(overrides: Partial<LogLine> = {}): LogLine {
  return {
    ts: "2026-01-01T12:00:00.000Z",
    level: "INFO",
    target: "osystems_sync_core::rescan",
    job_id: null,
    destination: null,
    message: "varredura manual falhou",
    error: null,
    path: null,
    ...overrides,
  };
}

describe("detailText", () => {
  it("returns null when neither error nor path is present", () => {
    expect(detailText(line())).toBeNull();
  });

  it("joins error and path when both are present", () => {
    expect(
      detailText(line({ error: "disk full", path: "/tmp/inbox/report.pdf" })),
    ).toBe("disk full — /tmp/inbox/report.pdf");
  });

  it("returns just the error when path is absent", () => {
    expect(detailText(line({ error: "disk full" }))).toBe("disk full");
  });

  it("returns just the path when error is absent", () => {
    expect(detailText(line({ path: "/tmp/inbox/report.pdf" }))).toBe("/tmp/inbox/report.pdf");
  });
});

describe("LogLineRow", () => {
  it("renders the error detail as secondary text alongside the message", () => {
    render(<LogLineRow line={line({ error: "disk full" })} />);

    expect(screen.getByText("varredura manual falhou")).toBeInTheDocument();
    expect(screen.getByText("disk full")).toBeInTheDocument();
  });

  it("renders both error and path when the event carried both", () => {
    render(<LogLineRow line={line({ error: "disk full", path: "/tmp/inbox/report.pdf" })} />);

    expect(screen.getByText("disk full — /tmp/inbox/report.pdf")).toBeInTheDocument();
  });

  it("sets the full detail text as the title attribute for hover", () => {
    render(<LogLineRow line={line({ error: "disk full", path: "/tmp/inbox/report.pdf" })} />);

    expect(screen.getByTitle("disk full — /tmp/inbox/report.pdf")).toBeInTheDocument();
  });

  it("renders no detail span when the line has neither error nor path", () => {
    const { container } = render(<LogLineRow line={line()} />);

    // Only the message span carries the mono `label-md` class; a detail span would
    // add a second, `label-sm`-toned mono span as a sibling.
    expect(container.querySelectorAll(".text-text-tertiary")).toHaveLength(0);
  });
});
