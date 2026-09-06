/**
 * Dashboard page — composition test (T-2.12, T-5.8; PRD.md RF-064/RF-070/
 * RF-037, RNF-008).
 *
 * Mocks `@/api/ipc` and `@/api/events` (`useTauriEvent` as a no-op) so this
 * test only exercises Dashboard's own composition/gating logic — the
 * three reachable empty/error states (no-folder → `EmptyStateNoFolder`;
 * folder-but-no-credentials → `EmptyStateNoCredentials`; `auth_required` →
 * `AuthRequiredBanner`, covered in its own test file) — and
 * `configStore.load()`/`statusStore.refresh()`/`credentialsStore.refresh()`
 * firing on mount. `JobTable`'s and `LogConsole`'s own fetch/event wiring is
 * covered by their test files.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

import { DEFAULT_CONFIG } from "@/api/defaults";
import { useConfigStore } from "@/store/configStore";
import { useCredentialsStore } from "@/store/credentialsStore";
import { useStatusStore } from "@/store/statusStore";
import type { AppConfig, AppStatus, CredentialStatus } from "@/types/generated";

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
    getCredentialStatus: vi.fn(),
  };
});

import { getConfig, getCredentialStatus, getRecentLogs, getStatus, listJobs } from "@/api/ipc";

import { Dashboard } from "../Dashboard";

const mockedGetConfig = vi.mocked(getConfig);
const mockedGetStatus = vi.mocked(getStatus);
const mockedListJobs = vi.mocked(listJobs);
const mockedGetRecentLogs = vi.mocked(getRecentLogs);
const mockedGetCredentialStatus = vi.mocked(getCredentialStatus);

const initialConfigState = useConfigStore.getState();
const initialStatusState = useStatusStore.getState();
const initialCredentialsState = useCredentialsStore.getState();

function credentialStatus(overrides: Partial<CredentialStatus> = {}): CredentialStatus {
  return {
    aws: { present: false, masked: null },
    gdrive: { present: false, email: null, project_id: null },
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
    counts_by_status: {
      pending: 0,
      uploading: 0,
      paused: 0,
      cancelled: 0,
      done: 0,
      failed: 0,
      bytes_total: 0,
      bytes_done: 0,
    },
    core_version: "0.1.0",
    build_target: "Tauri 2 • Windows x64",
    ...overrides,
  };
}

function config(overrides: Partial<AppConfig> = {}): AppConfig {
  return { ...DEFAULT_CONFIG, ...overrides };
}

beforeEach(() => {
  useConfigStore.setState(initialConfigState, true);
  useStatusStore.setState(initialStatusState, true);
  useCredentialsStore.setState(initialCredentialsState, true);

  mockedGetConfig.mockReset();
  mockedGetStatus.mockReset();
  mockedListJobs.mockReset();
  mockedGetRecentLogs.mockReset();
  mockedGetCredentialStatus.mockReset();

  mockedGetStatus.mockResolvedValue(status());
  mockedListJobs.mockResolvedValue({ items: [], total: 0 });
  mockedGetRecentLogs.mockResolvedValue([]);
  mockedGetCredentialStatus.mockResolvedValue(credentialStatus());
});

describe("Dashboard", () => {
  it("shows EmptyStateNoFolder and hides the KPI row/table when watch.path is null", async () => {
    mockedGetConfig.mockResolvedValue(config({ watch: { ...DEFAULT_CONFIG.watch, path: null } }));

    render(
      <MemoryRouter>
        <Dashboard />
      </MemoryRouter>
    );

    expect(await screen.findByText("Nenhuma pasta configurada")).toBeInTheDocument();
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
    expect(screen.queryByText("Total Detectados")).not.toBeInTheDocument();

    // LogConsole stays mounted even without a folder — logs are useful regardless.
    expect(screen.getByText("Console de Eventos em Tempo Real")).toBeInTheDocument();
    expect(mockedGetRecentLogs).toHaveBeenCalledTimes(1);

    // Header title is always visible.
    expect(screen.getByRole("heading", { name: "Fila de Sincronização em Tempo Real" })).toBeInTheDocument();

    await waitFor(() => expect(mockedGetStatus).toHaveBeenCalledTimes(1));
    expect(mockedListJobs).not.toHaveBeenCalled();
  });

  it("shows the KPI row, table and console when a folder is configured and a credential is present", async () => {
    mockedGetConfig.mockResolvedValue(config({ watch: { ...DEFAULT_CONFIG.watch, path: "C:\\Watch" } }));
    mockedGetCredentialStatus.mockResolvedValue(
      credentialStatus({ aws: { present: true, masked: "AKIA****PLE" } })
    );

    render(
      <MemoryRouter>
        <Dashboard />
      </MemoryRouter>
    );

    await waitFor(() => expect(mockedListJobs).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(mockedGetCredentialStatus).toHaveBeenCalledTimes(1));

    expect(screen.queryByText("Nenhuma pasta configurada")).not.toBeInTheDocument();
    expect(screen.queryByText("Nenhuma credencial configurada")).not.toBeInTheDocument();
    expect(screen.getByText("Total Detectados")).toBeInTheDocument();
    expect(screen.getByRole("table")).toBeInTheDocument();
    expect(screen.getByText("Console de Eventos em Tempo Real")).toBeInTheDocument();

    await waitFor(() => expect(mockedGetStatus).toHaveBeenCalledTimes(1));
  });

  it("shows EmptyStateNoCredentials above the KPI row/table (still rendered) when a folder is configured but no destination has a credential", async () => {
    mockedGetConfig.mockResolvedValue(config({ watch: { ...DEFAULT_CONFIG.watch, path: "C:\\Watch" } }));
    mockedGetCredentialStatus.mockResolvedValue(credentialStatus());

    render(
      <MemoryRouter>
        <Dashboard />
      </MemoryRouter>
    );

    await waitFor(() => expect(mockedGetCredentialStatus).toHaveBeenCalledTimes(1));

    expect(await screen.findByText("Nenhuma credencial configurada")).toBeInTheDocument();
    // KPI row/table stay mounted below so already-detected files remain visible.
    await waitFor(() => expect(mockedListJobs).toHaveBeenCalledTimes(1));
    expect(screen.getByText("Total Detectados")).toBeInTheDocument();
    expect(screen.getByRole("table")).toBeInTheDocument();
  });

  it("does not show EmptyStateNoCredentials when only gdrive has a saved credential", async () => {
    mockedGetConfig.mockResolvedValue(config({ watch: { ...DEFAULT_CONFIG.watch, path: "C:\\Watch" } }));
    mockedGetCredentialStatus.mockResolvedValue(
      credentialStatus({ gdrive: { present: true, email: "sa@my-project.iam.gserviceaccount.com", project_id: "my-project" } })
    );

    render(
      <MemoryRouter>
        <Dashboard />
      </MemoryRouter>
    );

    await waitFor(() => expect(mockedGetCredentialStatus).toHaveBeenCalledTimes(1));

    expect(screen.queryByText("Nenhuma credencial configurada")).not.toBeInTheDocument();
  });

  it("does not show EmptyStateNoCredentials (or fetch credentials-gated content) when no folder is configured", async () => {
    mockedGetConfig.mockResolvedValue(config({ watch: { ...DEFAULT_CONFIG.watch, path: null } }));
    mockedGetCredentialStatus.mockResolvedValue(credentialStatus());

    render(
      <MemoryRouter>
        <Dashboard />
      </MemoryRouter>
    );

    expect(await screen.findByText("Nenhuma pasta configurada")).toBeInTheDocument();
    expect(screen.queryByText("Nenhuma credencial configurada")).not.toBeInTheDocument();
  });

  it("calls getStatus and getCredentialStatus on mount regardless of folder state", async () => {
    mockedGetConfig.mockResolvedValue(config());

    render(
      <MemoryRouter>
        <Dashboard />
      </MemoryRouter>
    );

    await waitFor(() => expect(mockedGetStatus).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(mockedGetCredentialStatus).toHaveBeenCalledTimes(1));
  });
});
