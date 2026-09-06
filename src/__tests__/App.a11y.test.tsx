/**
 * Full-App accessibility audit (RNF-013, PLAN.md T-6.2).
 *
 * Renders the whole `App` — shell (TitleBar/Sidebar/StatusBar) + routed page
 * — at `/dashboard` and at `/settings`, each seeded with realistic, non-empty
 * state (per `Dashboard.test.tsx`/`JobTable.test.tsx`/`Settings.test.tsx`'s
 * own mocking conventions: mock `@/api/ipc`'s wrappers, let the pages' own
 * mount-time loads populate the stores), and asserts axe-core finds zero
 * violations. Also opens a `RowActions` tooltip and Settings' "Restaurar
 * padrões" `ConfirmDialog` and re-runs axe on each, since neither is present
 * in the base render tree.
 *
 * `color-contrast` is disabled in `@/test/a11y`'s shared `axe` config (jsdom
 * cannot compute it); contrast is instead verified by
 * `src/styles/contrast.test.ts`. No other rule is excluded here.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";

import { axe } from "@/test/a11y";

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

/** A status snapshot with the GDrive destination requiring re-auth, so `AuthRequiredBanner` renders. */
function statusWithAuthRequired(overrides: Partial<AppStatus> = {}): AppStatus {
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
    ...overrides,
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

describe("App accessibility audit — /dashboard", () => {
  async function renderDashboardReady(): Promise<HTMLElement> {
    mockedGetConfig.mockResolvedValue(config({ watch: { ...DEFAULT_CONFIG.watch, path: "C:\\Watch" } }));
    mockedGetStatus.mockResolvedValue(statusWithAuthRequired());
    mockedListJobs.mockResolvedValue({ items: threeFixtureJobs(), total: 3 });

    const { container } = renderApp(["/dashboard"]);

    // Wait for the seeded rows and the auth-required banner to actually land.
    await screen.findByText("backup.zip");
    await waitFor(() => expect(screen.getAllByRole("alert").length).toBeGreaterThan(0));

    return container;
  }

  it("has no automated accessibility violations (KPI row, 3-row table with a failed job, auth-required banner)", async () => {
    const container = await renderDashboardReady();

    expect(await axe(container)).toHaveNoViolations();
  });

  it("has no automated accessibility violations with a RowActions tooltip open", async () => {
    const container = await renderDashboardReady();

    const explorerButtons = screen.getAllByRole("button", { name: /abrir no explorer/i });
    const firstExplorerButton = explorerButtons[0];
    if (!firstExplorerButton) throw new Error("Expected at least one RowActions button to be rendered");
    firstExplorerButton.focus();

    await screen.findByRole("tooltip");

    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("App accessibility audit — /settings", () => {
  async function renderSettingsReady(): Promise<HTMLElement> {
    mockedGetConfig.mockResolvedValue(config({ s3: { ...DEFAULT_CONFIG.s3, enabled: true } }));
    useCredentialsStore.setState({ status: credentialStatusPresent() });

    const { container } = renderApp(["/settings"]);

    await waitFor(() => expect(mockedGetConfig).toHaveBeenCalledTimes(1));
    await screen.findByRole("heading", { name: "Configurações & QoS de Banda" });

    return container;
  }

  it("has no automated accessibility violations (config loaded, credentials present)", async () => {
    const container = await renderSettingsReady();

    expect(await axe(container)).toHaveNoViolations();
  });

  it("has no automated accessibility violations with the 'Restaurar padrões' ConfirmDialog open", async () => {
    const user = userEvent.setup();
    const container = await renderSettingsReady();

    await user.click(screen.getByRole("button", { name: "Restaurar padrões" }));
    await screen.findByRole("dialog");

    expect(await axe(container)).toHaveNoViolations();
  });
});
