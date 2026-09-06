/**
 * Icon-only-button audit (RNF-013, PLAN.md T-6.2).
 *
 * Programmatically renders the Dashboard and Settings routes (via the full
 * `App`, same seeding as `App.a11y.test.tsx`) and every button whose text
 * content is empty — an icon-only trigger — must carry an accessible name
 * via `aria-label` or `aria-labelledby`. This is a narrower, static
 * complement to the axe-core scan: axe's `button-name` rule already covers
 * this, but a targeted assertion here prints the exact list of offending
 * buttons on failure instead of a generic violation dump.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

import { DEFAULT_CONFIG } from "@/api/defaults";
import { makeJob } from "@/store/__fixtures__/jobs";
import { useConfigStore } from "@/store/configStore";
import { useCredentialsStore } from "@/store/credentialsStore";
import { useJobsStore } from "@/store/jobsStore";
import { useStatusStore } from "@/store/statusStore";
import type { AppConfig, AppStatus, CredentialStatus, JobView } from "@/types/generated";

import { App } from "../App";

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    minimize: vi.fn(),
    toggleMaximize: vi.fn(),
    close: vi.fn(),
  }),
}));

vi.mock("@/api/events", () => ({
  useTauriEvent: vi.fn(),
}));

vi.mock("@/api/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/ipc")>();
  return {
    ...actual,
    getConfig: vi.fn(),
    getStatus: vi.fn(),
    listJobs: vi.fn(),
    getRecentLogs: vi.fn(),
    saveConfig: vi.fn(),
  };
});

import { getConfig, getRecentLogs, getStatus, listJobs } from "@/api/ipc";

const mockedGetConfig = vi.mocked(getConfig);
const mockedGetStatus = vi.mocked(getStatus);
const mockedListJobs = vi.mocked(listJobs);
const mockedGetRecentLogs = vi.mocked(getRecentLogs);

const initialConfigState = useConfigStore.getState();
const initialStatusState = useStatusStore.getState();
const initialJobsState = useJobsStore.getState();
const initialCredentialsState = useCredentialsStore.getState();

function config(overrides: Partial<AppConfig> = {}): AppConfig {
  return { ...DEFAULT_CONFIG, ...overrides };
}

function statusWithAuthRequired(): AppStatus {
  return {
    watcher_paused: false,
    destinations: {
      gdrive: { online: false, auth_required: true, latency_ms: null },
      s3: { online: true, auth_required: false, latency_ms: 41 },
    },
    counts_by_status: {
      pending: 1,
      uploading: 1,
      paused: 0,
      cancelled: 0,
      done: 1,
      failed: 1,
      bytes_total: 4096,
      bytes_done: 2048,
    },
    core_version: "0.1.0",
    build_target: "Tauri 2 • Windows x64",
  };
}

function threeFixtureJobs(): JobView[] {
  return [
    makeJob({ name: "relatorio.pdf", gdrive: { status: "uploading" }, s3: { status: "pending" } }),
    makeJob({ name: "planilha.xlsx", gdrive: { status: "done" }, s3: { status: "done" } }),
    makeJob({
      name: "backup.zip",
      gdrive: { status: "done" },
      s3: { status: "failed", last_error: "403 Forbidden: Access Denied" },
    }),
  ];
}

function credentialStatusPresent(): CredentialStatus {
  return {
    aws: { present: true, masked: "AKIA****XYZ" },
    gdrive: { present: true, email: "sync@example.iam.gserviceaccount.com", project_id: "osystems-sync" },
  };
}

function renderApp(initialEntries: string[]) {
  return render(
    <MemoryRouter initialEntries={initialEntries}>
      <App />
    </MemoryRouter>,
  );
}

/** A button is "icon-only" when its full rendered text content is empty (whitespace-only counts as empty). */
function iconOnlyButtonsWithoutAName(): string[] {
  const offenders: string[] = [];
  for (const button of screen.getAllByRole("button")) {
    const hasVisibleText = (button.textContent ?? "").trim().length > 0;
    if (hasVisibleText) continue;

    const hasAriaLabel = (button.getAttribute("aria-label") ?? "").trim().length > 0;
    const labelledbyId = button.getAttribute("aria-labelledby");
    const hasAriaLabelledby = labelledbyId !== null && labelledbyId.trim().length > 0;

    if (!hasAriaLabel && !hasAriaLabelledby) {
      const description = button.outerHTML.length > 200 ? `${button.outerHTML.slice(0, 200)}…` : button.outerHTML;
      offenders.push(description);
    }
  }
  return offenders;
}

beforeEach(() => {
  useConfigStore.setState(initialConfigState, true);
  useStatusStore.setState(initialStatusState, true);
  useJobsStore.setState(initialJobsState, true);
  useCredentialsStore.setState(initialCredentialsState, true);

  mockedGetConfig.mockReset();
  mockedGetStatus.mockReset();
  mockedListJobs.mockReset();
  mockedGetRecentLogs.mockReset();

  mockedGetRecentLogs.mockResolvedValue([]);
});

describe("icon-only buttons carry an accessible name", () => {
  it("every icon-only button on /dashboard has aria-label or aria-labelledby", async () => {
    mockedGetConfig.mockResolvedValue(config({ watch: { ...DEFAULT_CONFIG.watch, path: "C:\\Watch" } }));
    mockedGetStatus.mockResolvedValue(statusWithAuthRequired());
    mockedListJobs.mockResolvedValue({ items: threeFixtureJobs(), total: 3 });

    renderApp(["/dashboard"]);
    await screen.findByText("backup.zip");
    await waitFor(() => expect(screen.getAllByRole("alert").length).toBeGreaterThan(0));

    const offenders = iconOnlyButtonsWithoutAName();
    expect(offenders, `Icon-only buttons missing aria-label/aria-labelledby:\n${offenders.join("\n")}`).toEqual([]);
  });

  it("every icon-only RowActions button on /dashboard has aria-label or aria-labelledby, tooltip open included", async () => {
    mockedGetConfig.mockResolvedValue(config({ watch: { ...DEFAULT_CONFIG.watch, path: "C:\\Watch" } }));
    mockedGetStatus.mockResolvedValue(statusWithAuthRequired());
    mockedListJobs.mockResolvedValue({ items: threeFixtureJobs(), total: 3 });

    renderApp(["/dashboard"]);
    await screen.findByText("backup.zip");

    // The row actions are all icon-only now that nothing hides behind a `⋯`
    // menu; focusing one also opens its `Tooltip`, which must describe the
    // button without becoming its accessible name.
    const explorerButtons = screen.getAllByRole("button", { name: /abrir no explorer/i });
    const firstExplorerButton = explorerButtons[0];
    if (!firstExplorerButton) throw new Error("Expected at least one RowActions button to be rendered");
    firstExplorerButton.focus();
    await screen.findByRole("tooltip");

    const offenders = iconOnlyButtonsWithoutAName();
    expect(offenders, `Icon-only buttons missing aria-label/aria-labelledby:\n${offenders.join("\n")}`).toEqual([]);
  });

  it("every icon-only button on /settings has aria-label or aria-labelledby", async () => {
    mockedGetConfig.mockResolvedValue(config({ s3: { ...DEFAULT_CONFIG.s3, enabled: true } }));
    useCredentialsStore.setState({ status: credentialStatusPresent() });

    renderApp(["/settings"]);
    await waitFor(() => expect(mockedGetConfig).toHaveBeenCalledTimes(1));
    await screen.findByRole("heading", { name: "Configurações & QoS de Banda" });

    const offenders = iconOnlyButtonsWithoutAName();
    expect(offenders, `Icon-only buttons missing aria-label/aria-labelledby:\n${offenders.join("\n")}`).toEqual([]);
  });
});
