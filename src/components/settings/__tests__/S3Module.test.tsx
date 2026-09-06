import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

vi.mock("@/api/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/ipc")>();
  return {
    ...actual,
    setCredential: vi.fn(),
    clearCredential: vi.fn(),
    getCredentialStatus: vi.fn(),
    testConnection: vi.fn(),
  };
});

import { clearCredential, getCredentialStatus, setCredential, testConnection, type AppError } from "@/api/ipc";
import { DEFAULT_CONFIG } from "@/api/defaults";
import { useConfigStore } from "@/store/configStore";
import { useCredentialsStore } from "@/store/credentialsStore";
import type { CredentialStatus, S3Config, TestResult, ValidationIssue } from "@/types/generated";

import { S3Module } from "../S3Module";

const mockedSetCredential = vi.mocked(setCredential);
const mockedClearCredential = vi.mocked(clearCredential);
const mockedGetCredentialStatus = vi.mocked(getCredentialStatus);
const mockedTestConnection = vi.mocked(testConnection);

const initialConfigState = useConfigStore.getState();
const initialCredentialsState = useCredentialsStore.getState();

beforeEach(() => {
  useConfigStore.setState(initialConfigState, true);
  useCredentialsStore.setState(initialCredentialsState, true);
  mockedSetCredential.mockReset().mockResolvedValue(undefined);
  mockedClearCredential.mockReset().mockResolvedValue(undefined);
  mockedGetCredentialStatus.mockReset();
  mockedTestConnection.mockReset();
});

function seedS3(overrides: Partial<S3Config> = {}, issues: ValidationIssue[] = []): void {
  useConfigStore.setState({
    config: { ...DEFAULT_CONFIG, s3: { ...DEFAULT_CONFIG.s3, ...overrides } },
    issues,
  });
}

function credentialStatus(overrides: Partial<CredentialStatus["aws"]> = {}): CredentialStatus {
  return {
    aws: { present: false, masked: null, ...overrides },
    gdrive: { present: false, email: null, project_id: null },
  };
}

function seedCredentials(aws: Partial<CredentialStatus["aws"]> = {}): void {
  useCredentialsStore.setState({ status: credentialStatus(aws) });
}

describe("S3Module", () => {
  it("disables destination fields while s3.enabled is off, and enables them once toggled on", async () => {
    const user = userEvent.setup();
    seedS3({ enabled: false });
    render(<S3Module />);

    expect(screen.getByLabelText("Bucket")).toBeDisabled();
    expect(screen.getByLabelText("Prefixo")).toBeDisabled();
    expect(screen.getByLabelText("Região AWS")).toBeDisabled();
    expect(screen.getByLabelText("Storage Class")).toBeDisabled();

    await user.click(screen.getByRole("switch", { name: "Habilitado" }));

    expect(useConfigStore.getState().config.s3.enabled).toBe(true);
    expect(screen.getByLabelText("Bucket")).not.toBeDisabled();
    expect(screen.getByLabelText("Prefixo")).not.toBeDisabled();
    expect(screen.getByLabelText("Região AWS")).not.toBeDisabled();
    expect(screen.getByLabelText("Storage Class")).not.toBeDisabled();
  });

  it("selecting a region updates config.s3.region", async () => {
    const user = userEvent.setup();
    seedS3({ enabled: true });
    render(<S3Module />);

    await user.selectOptions(screen.getByLabelText("Região AWS"), "sa-east-1");

    expect(useConfigStore.getState().config.s3.region).toBe("sa-east-1");
  });

  it("typing into Bucket updates config.s3.bucket", async () => {
    const user = userEvent.setup();
    seedS3({ enabled: true, bucket: "" });
    render(<S3Module />);

    await user.type(screen.getByLabelText("Bucket"), "meu-bucket");

    expect(useConfigStore.getState().config.s3.bucket).toBe("meu-bucket");
  });

  it("shows a validation issue for s3.bucket as an alert", () => {
    seedS3({ enabled: true }, [{ field: "s3.bucket", message: "Bucket é obrigatório." }]);
    render(<S3Module />);

    expect(screen.getByRole("alert")).toHaveTextContent("Bucket é obrigatório.");
  });

  describe("credential badge", () => {
    it("shows the neutral badge when no AWS credentials are saved", () => {
      seedS3({ enabled: true });
      seedCredentials({ present: false });
      render(<S3Module />);

      expect(screen.getByText("Não configurado")).toBeInTheDocument();
    });

    it("shows the info badge once credentials are saved", () => {
      seedS3({ enabled: true });
      seedCredentials({ present: true, masked: "AKIA****PLE" });
      render(<S3Module />);

      expect(screen.getByText("Credenciais salvas")).toBeInTheDocument();
    });

    it("shows the success badge after a successful test", () => {
      seedS3({ enabled: true });
      seedCredentials({ present: true, masked: "AKIA****PLE" });
      useCredentialsStore.setState({ lastTest: { s3: { ok: true, message: "Bucket válido (Put/List OK)", latency_ms: 41 }, gdrive: null } });
      render(<S3Module />);

      expect(screen.getByText("Conectado / Online")).toBeInTheDocument();
    });

    it("shows the error badge after a failed test", () => {
      seedS3({ enabled: true });
      seedCredentials({ present: true, masked: "AKIA****PLE" });
      useCredentialsStore.setState({ lastTest: { s3: { error: "s3.accessDenied" }, gdrive: null } });
      render(<S3Module />);

      expect(screen.getByText("Requer atenção")).toBeInTheDocument();
    });
  });

  describe("credential drafts (no saved credentials yet)", () => {
    it("keeps the secret eye toggle disabled while the draft is empty, and enables it once typed", async () => {
      const user = userEvent.setup();
      seedS3({ enabled: true });
      seedCredentials({ present: false });
      render(<S3Module />);

      const secretInput = screen.getByLabelText("Secret Access Key");
      const eyeButton = screen.getByRole("button", { name: "Mostrar" });
      expect(eyeButton).toBeDisabled();
      expect(secretInput).toHaveAttribute("type", "password");

      await user.type(secretInput, "s3cr3t");

      expect(eyeButton).not.toBeDisabled();
      await user.click(eyeButton);
      expect(secretInput).toHaveAttribute("type", "text");
    });

    it("disables 'Salvar credenciais' until both Access Key ID and Secret are filled", async () => {
      const user = userEvent.setup();
      seedS3({ enabled: true });
      seedCredentials({ present: false });
      render(<S3Module />);

      const saveButton = screen.getByRole("button", { name: "Salvar credenciais" });
      expect(saveButton).toBeDisabled();

      await user.type(screen.getByLabelText("Access Key ID"), "AKIAEXAMPLE");
      expect(saveButton).toBeDisabled();

      await user.type(screen.getByLabelText("Secret Access Key"), "s3cr3t-value");
      expect(saveButton).toBeEnabled();
    });

    it("saves credentials via setCredential (twice, correct keys/values), then shows the masked value with the secret never in the DOM and its eye disabled", async () => {
      const user = userEvent.setup();
      seedS3({ enabled: true });
      seedCredentials({ present: false });
      mockedGetCredentialStatus.mockResolvedValueOnce(credentialStatus({ present: true, masked: "AKIA****PLE" }));
      render(<S3Module />);

      await user.type(screen.getByLabelText("Access Key ID"), "AKIAEXAMPLE");
      await user.type(screen.getByLabelText("Secret Access Key"), "s3cr3t-value");
      await user.click(screen.getByRole("button", { name: "Salvar credenciais" }));

      await waitFor(() => {
        expect(mockedSetCredential).toHaveBeenCalledTimes(2);
      });
      expect(mockedSetCredential).toHaveBeenNthCalledWith(1, "aws.access_key_id", "AKIAEXAMPLE");
      expect(mockedSetCredential).toHaveBeenNthCalledWith(2, "aws.secret_access_key", "s3cr3t-value");

      await waitFor(() => {
        expect(screen.getByLabelText("Access Key ID")).toHaveValue("AKIA****PLE");
      });
      expect(screen.getByText("Credenciais salvas no Windows Credential Manager")).toBeInTheDocument();

      const secretInput = screen.getByLabelText("Secret Access Key");
      expect(secretInput).toHaveValue("••••••••");
      expect(secretInput).toHaveAttribute("readonly");
      expect(document.body.textContent).not.toContain("s3cr3t-value");

      const eyeButton = screen.getByRole("button", { name: "Mostrar" });
      expect(eyeButton).toBeDisabled();
    });
  });

  describe("credentials already saved", () => {
    it("shows the Access Key ID read-only and masked, with a 'Substituir' button that enters edit mode", async () => {
      const user = userEvent.setup();
      seedS3({ enabled: true });
      seedCredentials({ present: true, masked: "AKIA****PLE" });
      render(<S3Module />);

      const accessKeyInput = screen.getByLabelText("Access Key ID");
      expect(accessKeyInput).toHaveValue("AKIA****PLE");
      expect(accessKeyInput).toHaveAttribute("readonly");

      await user.click(screen.getByRole("button", { name: "Substituir" }));

      const editableInput = screen.getByLabelText("Access Key ID");
      expect(editableInput).not.toHaveAttribute("readonly");
      expect(editableInput).toHaveValue("");
    });

    it("removes credentials via a ConfirmDialog calling clearCredential twice", async () => {
      const user = userEvent.setup();
      seedS3({ enabled: true });
      seedCredentials({ present: true, masked: "AKIA****PLE" });
      mockedGetCredentialStatus.mockResolvedValueOnce(credentialStatus({ present: false, masked: null }));
      render(<S3Module />);

      await user.click(screen.getByRole("button", { name: "Remover" }));

      const dialog = screen.getByRole("dialog");
      expect(within(dialog).getByText("Remover credenciais da AWS?")).toBeInTheDocument();

      await user.click(within(dialog).getByRole("button", { name: "Remover" }));

      await waitFor(() => {
        expect(mockedClearCredential).toHaveBeenCalledTimes(2);
      });
      expect(mockedClearCredential).toHaveBeenNthCalledWith(1, "aws.access_key_id");
      expect(mockedClearCredential).toHaveBeenNthCalledWith(2, "aws.secret_access_key");
    });
  });

  describe("Testar bucket", () => {
    it("is disabled when there are no saved credentials", () => {
      seedS3({ enabled: true, bucket: "meu-bucket" });
      seedCredentials({ present: false });
      render(<S3Module />);

      expect(screen.getByRole("button", { name: "Testar bucket" })).toBeDisabled();
    });

    it("is disabled when the bucket field is empty", () => {
      seedS3({ enabled: true, bucket: "" });
      seedCredentials({ present: true, masked: "AKIA****PLE" });
      render(<S3Module />);

      expect(screen.getByRole("button", { name: "Testar bucket" })).toBeDisabled();
    });

    it("calls testConnection('s3') and shows the latency + message on success", async () => {
      const user = userEvent.setup();
      seedS3({ enabled: true, bucket: "meu-bucket" });
      seedCredentials({ present: true, masked: "AKIA****PLE" });
      const result: TestResult = { ok: true, message: "Bucket válido (Put/List OK)", latency_ms: 41 };
      mockedTestConnection.mockResolvedValueOnce(result);
      render(<S3Module />);

      await user.click(screen.getByRole("button", { name: "Testar bucket" }));

      expect(mockedTestConnection).toHaveBeenCalledWith("s3");
      await waitFor(() => {
        expect(screen.getByText("✓ 41 ms • Bucket válido (Put/List OK)")).toBeInTheDocument();
      });
    });

    it("shows the tError() text for the AppError code on failure", async () => {
      const user = userEvent.setup();
      seedS3({ enabled: true, bucket: "meu-bucket" });
      seedCredentials({ present: true, masked: "AKIA****PLE" });
      const failure: AppError = { code: "s3.accessDenied", message: "Acesso negado" };
      mockedTestConnection.mockRejectedValueOnce(failure);
      render(<S3Module />);

      await user.click(screen.getByRole("button", { name: "Testar bucket" }));

      await waitFor(() => {
        expect(
          screen.getByText("✕ Acesso negado ao bucket S3. Verifique as credenciais e permissões IAM."),
        ).toBeInTheDocument();
      });
    });
  });
});
