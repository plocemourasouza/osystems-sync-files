/**
 * Settings page — mount load, Salvar/Cancelar/Restaurar padrões + `Ctrl+S`
 * (PRD.md RF-083, RF-084; T-1.9).
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";

import { DEFAULT_CONFIG } from "@/api/defaults";
import type { AppError } from "@/api/ipc";
import { useConfigStore, type ConfigStatus } from "@/store/configStore";
import type { AppConfig, ValidationIssue } from "@/types/generated";

import { Settings } from "../Settings";

vi.mock("@/api/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/ipc")>();
  return {
    ...actual,
    getConfig: vi.fn(),
    saveConfig: vi.fn(),
  };
});

// Imported after the mock so `getConfig`/`saveConfig` resolve to the mocked fns.
import { getConfig, saveConfig } from "@/api/ipc";

const mockedGetConfig = vi.mocked(getConfig);
const mockedSaveConfig = vi.mocked(saveConfig);

const initialState = useConfigStore.getState();

function renderSettings() {
  return render(
    <MemoryRouter>
      <Settings />
    </MemoryRouter>,
  );
}

type SeedOverrides = {
  status?: ConfigStatus;
  config?: AppConfig;
  saved?: AppConfig;
  dirty?: boolean;
  issues?: ValidationIssue[];
  error?: string | null;
  lastSavedAt?: string | null;
};

/** Seeds the store as `load()` would have already resolved (`status: "ready"`). */
function seedReady(overrides: SeedOverrides = {}): void {
  useConfigStore.setState({
    status: "ready",
    config: DEFAULT_CONFIG,
    saved: DEFAULT_CONFIG,
    dirty: false,
    issues: [],
    error: null,
    lastSavedAt: null,
    ...overrides,
  });
}

beforeEach(() => {
  useConfigStore.setState(initialState, true);
  mockedGetConfig.mockReset();
  mockedSaveConfig.mockReset();
  mockedGetConfig.mockResolvedValue(DEFAULT_CONFIG);
  mockedSaveConfig.mockResolvedValue(undefined);
});

describe("Settings page", () => {
  it("loads the config on mount and renders every module heading", async () => {
    renderSettings();

    await waitFor(() => expect(mockedGetConfig).toHaveBeenCalledTimes(1));

    expect(screen.getByRole("heading", { name: "Configurações & QoS de Banda" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Google Drive" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Amazon S3" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Geral" })).toBeInTheDocument();
  });

  it("marks the config dirty when a field changes, and keeps Save clickable either way", async () => {
    const user = userEvent.setup();
    seedReady({ config: { ...DEFAULT_CONFIG, s3: { ...DEFAULT_CONFIG.s3, enabled: true } } });
    renderSettings();

    // The badge tracks `dirty`; the button does not (RF-083) — a clean form
    // still accepts the click, since `save()` is idempotent.
    expect(screen.queryByText("Alterações não salvas")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /salvar preferências/i })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Cancelar" })).toBeEnabled();

    await user.type(screen.getByLabelText("Bucket"), "novo-bucket");

    expect(screen.getByText("Alterações não salvas")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /salvar preferências/i })).toBeEnabled();
  });

  it("disables Save while validation issues are open", () => {
    seedReady({
      config: { ...DEFAULT_CONFIG, s3: { ...DEFAULT_CONFIG.s3, enabled: true } },
      issues: [{ field: "s3.bucket", message: "Bucket é obrigatório." }],
    });
    renderSettings();

    expect(screen.getByRole("button", { name: /salvar preferências/i })).toBeDisabled();
  });

  it("Salvar preferências persists the edited config and updates the footer", async () => {
    const user = userEvent.setup();
    seedReady({ config: { ...DEFAULT_CONFIG, s3: { ...DEFAULT_CONFIG.s3, enabled: true } } });
    renderSettings();

    await user.type(screen.getByLabelText("Bucket"), "novo-bucket");
    await user.click(screen.getByRole("button", { name: /salvar preferências/i }));

    await waitFor(() =>
      expect(mockedSaveConfig).toHaveBeenCalledWith(
        expect.objectContaining({ s3: expect.objectContaining({ bucket: "novo-bucket" }) }),
      ),
    );
    expect(screen.queryByText("Alterações não salvas")).not.toBeInTheDocument();
    expect(screen.getByText(/Última alteração salva às/)).toBeInTheDocument();
  });

  it("Ctrl+S saves whether the form is dirty or clean", async () => {
    const user = userEvent.setup();
    seedReady({ config: { ...DEFAULT_CONFIG, s3: { ...DEFAULT_CONFIG.s3, enabled: true } } });
    renderSettings();

    await user.keyboard("{Control>}s{/Control}");
    await waitFor(() => expect(mockedSaveConfig).toHaveBeenCalledTimes(1));

    await user.type(screen.getByLabelText("Bucket"), "novo-bucket");
    await user.keyboard("{Control>}s{/Control}");

    await waitFor(() => expect(mockedSaveConfig).toHaveBeenCalledTimes(2));
  });

  it("shows the issues count and the field alert when save rejects with config.invalid", async () => {
    const user = userEvent.setup();
    const issues: ValidationIssue[] = [{ field: "s3.bucket", message: "Bucket é obrigatório." }];
    const invalidError: AppError = { code: "config.invalid", message: JSON.stringify(issues) };
    mockedSaveConfig.mockRejectedValueOnce(invalidError);

    seedReady({
      config: { ...DEFAULT_CONFIG, s3: { ...DEFAULT_CONFIG.s3, enabled: true, bucket: "" } },
      saved: { ...DEFAULT_CONFIG, s3: { ...DEFAULT_CONFIG.s3, enabled: false, bucket: "" } },
      dirty: true,
    });
    renderSettings();

    await user.click(screen.getByRole("button", { name: /salvar preferências/i }));

    await waitFor(() => expect(screen.getByText("1 campo(s) com erro de validação")).toBeInTheDocument());
    expect(screen.getByRole("alert")).toHaveTextContent("Bucket é obrigatório.");
  });

  it("Cancelar discards unsaved edits and restores the last saved value", async () => {
    const user = userEvent.setup();
    seedReady({
      config: { ...DEFAULT_CONFIG, s3: { ...DEFAULT_CONFIG.s3, enabled: true, bucket: "bucket-editado" } },
      saved: { ...DEFAULT_CONFIG, s3: { ...DEFAULT_CONFIG.s3, enabled: true, bucket: "bucket-original" } },
      dirty: true,
    });
    renderSettings();

    expect(screen.getByLabelText("Bucket")).toHaveValue("bucket-editado");

    await user.click(screen.getByRole("button", { name: "Cancelar" }));

    expect(useConfigStore.getState().config.s3.bucket).toBe("bucket-original");
    expect(screen.getByLabelText("Bucket")).toHaveValue("bucket-original");
    expect(screen.queryByText("Alterações não salvas")).not.toBeInTheDocument();
  });

  it("Restaurar padrões resets workers/QoS/filters after confirmation, but keeps destination fields", async () => {
    const user = userEvent.setup();
    seedReady({
      config: {
        ...DEFAULT_CONFIG,
        workers_per_destination: 4,
        s3: { ...DEFAULT_CONFIG.s3, enabled: true, bucket: "bucket-mantido" },
      },
    });
    renderSettings();

    await user.click(screen.getByRole("button", { name: "Restaurar padrões" }));

    const dialog = screen.getByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Restaurar" }));

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(useConfigStore.getState().config.workers_per_destination).toBe(DEFAULT_CONFIG.workers_per_destination);
    expect(useConfigStore.getState().config.s3.bucket).toBe("bucket-mantido");
  });

  it("Esc closes the Restaurar padrões dialog without resetting anything", async () => {
    const user = userEvent.setup();
    seedReady({ config: { ...DEFAULT_CONFIG, workers_per_destination: 4 } });
    renderSettings();

    await user.click(screen.getByRole("button", { name: "Restaurar padrões" }));
    expect(screen.getByRole("dialog")).toBeInTheDocument();

    await user.keyboard("{Escape}");

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(useConfigStore.getState().config.workers_per_destination).toBe(4);
  });
});
