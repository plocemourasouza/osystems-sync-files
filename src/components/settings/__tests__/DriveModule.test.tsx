import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

vi.mock("@/api/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/ipc")>();
  return {
    ...actual,
    pickServiceAccountFile: vi.fn(),
    clearCredential: vi.fn(),
    getCredentialStatus: vi.fn(),
    testConnection: vi.fn(),
  };
});

import {
  clearCredential,
  getCredentialStatus,
  pickServiceAccountFile,
  testConnection,
  type AppError,
  type ServiceAccountInfo,
} from "@/api/ipc";
import { DEFAULT_CONFIG } from "@/api/defaults";
import { useConfigStore } from "@/store/configStore";
import { useCredentialsStore } from "@/store/credentialsStore";
import type { CredentialStatus, GDriveConfig, TestResult, ValidationIssue } from "@/types/generated";

import { DriveModule } from "../DriveModule";

const mockedPickServiceAccountFile = vi.mocked(pickServiceAccountFile);
const mockedClearCredential = vi.mocked(clearCredential);
const mockedGetCredentialStatus = vi.mocked(getCredentialStatus);
const mockedTestConnection = vi.mocked(testConnection);

const initialState = useConfigStore.getState();
const initialCredentialsState = useCredentialsStore.getState();

beforeEach(() => {
  useConfigStore.setState(initialState, true);
  useCredentialsStore.setState(initialCredentialsState, true);
  mockedPickServiceAccountFile.mockReset();
  mockedClearCredential.mockReset().mockResolvedValue(undefined);
  mockedGetCredentialStatus.mockReset();
  mockedTestConnection.mockReset();
});

function seedGDrive(overrides: Partial<GDriveConfig> = {}, issues: ValidationIssue[] = []): void {
  useConfigStore.setState({
    config: { ...DEFAULT_CONFIG, gdrive: { ...DEFAULT_CONFIG.gdrive, ...overrides } },
    issues,
  });
}

function credentialStatus(overrides: Partial<CredentialStatus["gdrive"]> = {}): CredentialStatus {
  return {
    aws: { present: false, masked: null },
    gdrive: { present: false, email: null, project_id: null, ...overrides },
  };
}

function seedCredentials(gdrive: Partial<CredentialStatus["gdrive"]> = {}): void {
  useCredentialsStore.setState({ status: credentialStatus(gdrive) });
}

const SERVICE_ACCOUNT: ServiceAccountInfo = {
  file_name: "sync-backup-sa.json",
  size: 2380,
  client_email: "sync-backup@my-project.iam.gserviceaccount.com",
  project_id: "my-project",
};

describe("DriveModule", () => {
  it("typing into Folder ID updates config.gdrive.folder_id", async () => {
    const user = userEvent.setup();
    seedGDrive({ enabled: true, folder_id: "" });
    render(<DriveModule />);

    await user.type(screen.getByLabelText("ID da pasta de destino (Folder ID)"), "abc123");

    expect(useConfigStore.getState().config.gdrive.folder_id).toBe("abc123");
  });

  it("toggling the date subfolders checkbox updates config.gdrive.date_subfolders", async () => {
    const user = userEvent.setup();
    seedGDrive({ enabled: true, date_subfolders: false });
    render(<DriveModule />);

    await user.click(screen.getByLabelText(/Criar subpastas automaticamente por data/));

    expect(useConfigStore.getState().config.gdrive.date_subfolders).toBe(true);
  });

  it("renders the checksum checkbox always checked and disabled", () => {
    seedGDrive({ enabled: true });
    render(<DriveModule />);

    const checksum = screen.getByLabelText(/Habilitar checksum SHA-256 pré-upload/);

    expect(checksum).toBeChecked();
    expect(checksum).toBeDisabled();
  });

  it("disables Folder ID and the date subfolders checkbox while gdrive.enabled is off", () => {
    seedGDrive({ enabled: false });
    render(<DriveModule />);

    expect(screen.getByLabelText("ID da pasta de destino (Folder ID)")).toBeDisabled();
    expect(screen.getByLabelText(/Criar subpastas automaticamente por data/)).toBeDisabled();
  });

  it("shows a validation issue for gdrive.folder_id as an alert", () => {
    seedGDrive({ enabled: true }, [{ field: "gdrive.folder_id", message: "Folder ID é obrigatório." }]);
    render(<DriveModule />);

    expect(screen.getByRole("alert")).toHaveTextContent("Folder ID é obrigatório.");
  });

  it("copy button writes the folder id to the clipboard", async () => {
    const user = userEvent.setup();
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText } });
    seedGDrive({ enabled: true, folder_id: "1BxiMVs0XRA5nFMdKvBHKrZkF8Duhv7kH9" });
    render(<DriveModule />);

    await user.click(screen.getByRole("button", { name: "Copiar ID da pasta" }));

    expect(writeText).toHaveBeenCalledWith("1BxiMVs0XRA5nFMdKvBHKrZkF8Duhv7kH9");

    vi.unstubAllGlobals();
  });

  it("does not throw when navigator.clipboard is unavailable", async () => {
    const user = userEvent.setup();
    vi.stubGlobal("navigator", { ...navigator, clipboard: undefined });
    seedGDrive({ enabled: true, folder_id: "abc123" });
    render(<DriveModule />);

    await expect(user.click(screen.getByRole("button", { name: "Copiar ID da pasta" }))).resolves.not.toThrow();

    vi.unstubAllGlobals();
  });

  describe("credential absent", () => {
    it("shows 'Não configurado' and a 'Selecionar JSON da Service Account' button that calls pickServiceAccountFile", async () => {
      const user = userEvent.setup();
      seedGDrive({ enabled: true });
      seedCredentials();
      mockedPickServiceAccountFile.mockResolvedValueOnce(SERVICE_ACCOUNT);
      mockedGetCredentialStatus.mockResolvedValueOnce(
        credentialStatus({ present: true, email: SERVICE_ACCOUNT.client_email, project_id: SERVICE_ACCOUNT.project_id })
      );
      render(<DriveModule />);

      expect(screen.getByText("Não configurado")).toBeInTheDocument();

      await user.click(screen.getByRole("button", { name: "Selecionar JSON da Service Account" }));

      expect(mockedPickServiceAccountFile).toHaveBeenCalledTimes(1);
      await waitFor(() => expect(mockedGetCredentialStatus).toHaveBeenCalledTimes(1));
      expect(await screen.findByText(SERVICE_ACCOUNT.client_email)).toBeInTheDocument();
      expect(screen.getByText("Service Account salva")).toBeInTheDocument();
    });

    it("does nothing when the user cancels the picker", async () => {
      const user = userEvent.setup();
      seedGDrive({ enabled: true });
      seedCredentials();
      mockedPickServiceAccountFile.mockResolvedValueOnce(null);
      render(<DriveModule />);

      await user.click(screen.getByRole("button", { name: "Selecionar JSON da Service Account" }));

      expect(mockedPickServiceAccountFile).toHaveBeenCalledTimes(1);
      expect(mockedGetCredentialStatus).not.toHaveBeenCalled();
      expect(screen.getByText("Não configurado")).toBeInTheDocument();
    });

    it("shows a tError alert when picking fails", async () => {
      const user = userEvent.setup();
      seedGDrive({ enabled: true });
      seedCredentials();
      const error: AppError = { code: "unknown", message: "boom" };
      mockedPickServiceAccountFile.mockRejectedValueOnce(error);
      render(<DriveModule />);

      await user.click(screen.getByRole("button", { name: "Selecionar JSON da Service Account" }));

      expect(await screen.findByRole("alert")).toHaveTextContent(/./);
    });

    it("never renders raw JSON/private_key content", () => {
      seedGDrive({ enabled: true });
      seedCredentials();
      render(<DriveModule />);

      expect(screen.queryByText(/private_key/i)).not.toBeInTheDocument();
    });
  });

  describe("credential present", () => {
    it("shows the Service Account e-mail/project id and 'Substituir JSON'/'Remover' buttons", () => {
      seedGDrive({ enabled: true });
      seedCredentials({ present: true, email: SERVICE_ACCOUNT.client_email, project_id: SERVICE_ACCOUNT.project_id });
      render(<DriveModule />);

      expect(screen.getByText(SERVICE_ACCOUNT.client_email)).toBeInTheDocument();
      expect(screen.getByText(SERVICE_ACCOUNT.project_id as string)).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Substituir JSON" })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Remover" })).toBeInTheDocument();
    });

    it("'Substituir JSON' calls pickServiceAccountFile again", async () => {
      const user = userEvent.setup();
      seedGDrive({ enabled: true });
      seedCredentials({ present: true, email: SERVICE_ACCOUNT.client_email, project_id: SERVICE_ACCOUNT.project_id });
      mockedPickServiceAccountFile.mockResolvedValueOnce(SERVICE_ACCOUNT);
      mockedGetCredentialStatus.mockResolvedValueOnce(
        credentialStatus({ present: true, email: SERVICE_ACCOUNT.client_email, project_id: SERVICE_ACCOUNT.project_id })
      );
      render(<DriveModule />);

      await user.click(screen.getByRole("button", { name: "Substituir JSON" }));

      expect(mockedPickServiceAccountFile).toHaveBeenCalledTimes(1);
    });

    it("'Remover' opens a confirm dialog and, on confirm, calls clearCredential('gdrive.service_account_json')", async () => {
      const user = userEvent.setup();
      seedGDrive({ enabled: true });
      seedCredentials({ present: true, email: SERVICE_ACCOUNT.client_email, project_id: SERVICE_ACCOUNT.project_id });
      mockedGetCredentialStatus.mockResolvedValueOnce(credentialStatus());
      render(<DriveModule />);

      await user.click(screen.getByRole("button", { name: "Remover" }));
      const dialog = screen.getByRole("dialog");
      expect(dialog).toBeInTheDocument();

      await user.click(within(dialog).getByRole("button", { name: "Remover" }));

      await waitFor(() => expect(mockedClearCredential).toHaveBeenCalledWith("gdrive.service_account_json"));
    });
  });

  describe("test connection", () => {
    it("is disabled when credentials are absent or the folder id is empty", () => {
      seedGDrive({ enabled: true, folder_id: "" });
      seedCredentials();
      render(<DriveModule />);

      expect(screen.getByRole("button", { name: "Testar conexão" })).toBeDisabled();
    });

    it("calls testConnection('gdrive') and shows latency + message on success", async () => {
      const user = userEvent.setup();
      seedGDrive({ enabled: true, folder_id: "abc123" });
      seedCredentials({ present: true, email: SERVICE_ACCOUNT.client_email });
      const result: TestResult = { ok: true, message: "Pasta válida (List OK)", latency_ms: 55 };
      mockedTestConnection.mockResolvedValueOnce(result);
      render(<DriveModule />);

      await user.click(screen.getByRole("button", { name: "Testar conexão" }));

      expect(mockedTestConnection).toHaveBeenCalledWith("gdrive");
      expect(await screen.findByText(/55 ms/)).toBeInTheDocument();
      expect(screen.getByText("Conectado / Online")).toBeInTheDocument();
    });

    it("shows the tError message and 'Requer atenção' badge on failure", async () => {
      const user = userEvent.setup();
      seedGDrive({ enabled: true, folder_id: "abc123" });
      seedCredentials({ present: true, email: SERVICE_ACCOUNT.client_email });
      const error: AppError = { code: "gdrive.forbidden", message: "boom" };
      mockedTestConnection.mockRejectedValueOnce(error);
      render(<DriveModule />);

      await user.click(screen.getByRole("button", { name: "Testar conexão" }));

      expect(await screen.findByText("Requer atenção")).toBeInTheDocument();
      expect(screen.getByText(/Pasta não compartilhada/)).toBeInTheDocument();
    });
  });
});
