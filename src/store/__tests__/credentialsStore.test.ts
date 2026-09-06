import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AppError, ServiceAccountInfo } from "@/api/ipc";
import type { CredentialStatus, TestResult } from "@/types/generated";

vi.mock("@/api/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/ipc")>();
  return {
    ...actual,
    getCredentialStatus: vi.fn(),
    setCredential: vi.fn(),
    clearCredential: vi.fn(),
    testConnection: vi.fn(),
    pickServiceAccountFile: vi.fn(),
  };
});

import { clearCredential, getCredentialStatus, pickServiceAccountFile, setCredential, testConnection } from "@/api/ipc";
import { useCredentialsStore } from "@/store/credentialsStore";

const mockedGetCredentialStatus = vi.mocked(getCredentialStatus);
const mockedSetCredential = vi.mocked(setCredential);
const mockedClearCredential = vi.mocked(clearCredential);
const mockedTestConnection = vi.mocked(testConnection);
const mockedPickServiceAccountFile = vi.mocked(pickServiceAccountFile);

const initialState = useCredentialsStore.getState();

function status(overrides: Partial<CredentialStatus> = {}): CredentialStatus {
  return {
    aws: { present: false, masked: null },
    gdrive: { present: false, email: null, project_id: null },
    ...overrides,
  };
}

beforeEach(() => {
  useCredentialsStore.setState(initialState, true);
  mockedGetCredentialStatus.mockReset();
  mockedSetCredential.mockReset();
  mockedClearCredential.mockReset();
  mockedTestConnection.mockReset();
  mockedPickServiceAccountFile.mockReset();
});

describe("credentialsStore.refresh()", () => {
  it("fetches get_credential_status and replaces `status`", async () => {
    const snapshot = status({ aws: { present: true, masked: "AKIA****PLE" } });
    mockedGetCredentialStatus.mockResolvedValueOnce(snapshot);

    await useCredentialsStore.getState().refresh();

    const state = useCredentialsStore.getState();
    expect(state.status).toEqual(snapshot);
    expect(state.loading).toBe(false);
    expect(state.error).toBeNull();
  });

  it("sets an error message on rejection without throwing", async () => {
    mockedGetCredentialStatus.mockRejectedValueOnce(new Error("keyring unavailable"));

    await useCredentialsStore.getState().refresh();

    const state = useCredentialsStore.getState();
    expect(state.status).toBeNull();
    expect(state.error).toBe("keyring unavailable");
    expect(state.loading).toBe(false);
  });
});

describe("credentialsStore.setAws()", () => {
  it("calls setCredential twice with the exact CredentialKey literals, then refreshes", async () => {
    mockedSetCredential.mockResolvedValue(undefined);
    mockedGetCredentialStatus.mockResolvedValueOnce(status({ aws: { present: true, masked: "AKIA****PLE" } }));

    await useCredentialsStore.getState().setAws({ accessKeyId: "AKIAEXAMPLE", secret: "s3cr3t" });

    expect(mockedSetCredential).toHaveBeenCalledTimes(2);
    expect(mockedSetCredential).toHaveBeenNthCalledWith(1, "aws.access_key_id", "AKIAEXAMPLE");
    expect(mockedSetCredential).toHaveBeenNthCalledWith(2, "aws.secret_access_key", "s3cr3t");
    expect(mockedGetCredentialStatus).toHaveBeenCalledTimes(1);
    expect(useCredentialsStore.getState().status?.aws.present).toBe(true);
  });
});

describe("credentialsStore.clearAws()", () => {
  it("calls clearCredential twice with the exact CredentialKey literals, then refreshes", async () => {
    mockedClearCredential.mockResolvedValue(undefined);
    mockedGetCredentialStatus.mockResolvedValueOnce(status());

    await useCredentialsStore.getState().clearAws();

    expect(mockedClearCredential).toHaveBeenCalledTimes(2);
    expect(mockedClearCredential).toHaveBeenNthCalledWith(1, "aws.access_key_id");
    expect(mockedClearCredential).toHaveBeenNthCalledWith(2, "aws.secret_access_key");
    expect(mockedGetCredentialStatus).toHaveBeenCalledTimes(1);
    expect(useCredentialsStore.getState().status?.aws.present).toBe(false);
  });
});

describe("credentialsStore.pickServiceAccount()", () => {
  it("calls pickServiceAccountFile, stores the metadata in lastServiceAccount, and refreshes status", async () => {
    const info: ServiceAccountInfo = {
      file_name: "sync-backup-sa.json",
      size: 2380,
      client_email: "sync-backup@my-project.iam.gserviceaccount.com",
      project_id: "my-project",
    };
    mockedPickServiceAccountFile.mockResolvedValueOnce(info);
    mockedGetCredentialStatus.mockResolvedValueOnce(
      status({ gdrive: { present: true, email: info.client_email, project_id: info.project_id } })
    );

    const result = await useCredentialsStore.getState().pickServiceAccount();

    expect(result).toEqual(info);
    expect(mockedGetCredentialStatus).toHaveBeenCalledTimes(1);
    const state = useCredentialsStore.getState();
    expect(state.lastServiceAccount).toEqual(info);
    expect(state.status?.gdrive.present).toBe(true);
  });

  it("resolves null and leaves the store untouched when the user cancels the picker", async () => {
    mockedPickServiceAccountFile.mockResolvedValueOnce(null);

    const result = await useCredentialsStore.getState().pickServiceAccount();

    expect(result).toBeNull();
    expect(mockedGetCredentialStatus).not.toHaveBeenCalled();
    expect(useCredentialsStore.getState().lastServiceAccount).toBeNull();
  });
});

describe("credentialsStore.clearServiceAccount()", () => {
  it("calls clearCredential('gdrive.service_account_json'), clears lastServiceAccount, then refreshes", async () => {
    useCredentialsStore.setState({
      lastServiceAccount: {
        file_name: "sync-backup-sa.json",
        size: 2380,
        client_email: "sync-backup@my-project.iam.gserviceaccount.com",
        project_id: "my-project",
      },
    });
    mockedClearCredential.mockResolvedValueOnce(undefined);
    mockedGetCredentialStatus.mockResolvedValueOnce(status());

    await useCredentialsStore.getState().clearServiceAccount();

    expect(mockedClearCredential).toHaveBeenCalledTimes(1);
    expect(mockedClearCredential).toHaveBeenCalledWith("gdrive.service_account_json");
    expect(mockedGetCredentialStatus).toHaveBeenCalledTimes(1);
    const state = useCredentialsStore.getState();
    expect(state.lastServiceAccount).toBeNull();
    expect(state.status?.gdrive.present).toBe(false);
  });
});

describe("credentialsStore.test()", () => {
  it("toggles `testing[destination]` true then false, and records a successful TestResult in `lastTest`", async () => {
    const result: TestResult = { ok: true, message: "Bucket válido (Put/List OK)", latency_ms: 41 };
    let testingDuringCall: boolean | undefined;
    mockedTestConnection.mockImplementationOnce(async () => {
      testingDuringCall = useCredentialsStore.getState().testing.s3;
      return result;
    });

    await useCredentialsStore.getState().test("s3");

    expect(testingDuringCall).toBe(true);
    const state = useCredentialsStore.getState();
    expect(state.testing.s3).toBe(false);
    expect(state.lastTest.s3).toEqual(result);
  });

  it("records { error: code } in `lastTest` when test_connection rejects with an AppError", async () => {
    const failure: AppError = { code: "s3.accessDenied", message: "Acesso negado" };
    mockedTestConnection.mockRejectedValueOnce(failure);

    await useCredentialsStore.getState().test("s3");

    const state = useCredentialsStore.getState();
    expect(state.testing.s3).toBe(false);
    expect(state.lastTest.s3).toEqual({ error: "s3.accessDenied" });
  });

  it("records { error: 'unknown' } when the rejection is not an AppError", async () => {
    mockedTestConnection.mockRejectedValueOnce(new Error("network down"));

    await useCredentialsStore.getState().test("s3");

    expect(useCredentialsStore.getState().lastTest.s3).toEqual({ error: "unknown" });
  });

  it("keeps destinations independent: testing gdrive does not touch s3's lastTest", async () => {
    const result: TestResult = { ok: true, message: "OK", latency_ms: 12 };
    mockedTestConnection.mockResolvedValueOnce(result);

    await useCredentialsStore.getState().test("gdrive");

    const state = useCredentialsStore.getState();
    expect(state.lastTest.gdrive).toEqual(result);
    expect(state.lastTest.s3).toBeNull();
    expect(state.testing.s3).toBe(false);
  });
});
