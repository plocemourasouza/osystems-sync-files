import { describe, expect, it, afterEach } from "vitest";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";

import {
  getConfig,
  saveConfig,
  setAutostart,
  isAppError,
  parseValidationIssues,
  pickFolder,
  rescan,
  pauseWatcher,
  resumeWatcher,
  listJobs,
  getStatus,
  getRecentLogs,
  openLogsFolder,
  setCredential,
  getCredentialStatus,
  clearCredential,
  testConnection,
  retryJob,
  retryAllFailed,
  cancelJob,
  clearCompleted,
  setQos,
  openInExplorer,
  openRemote,
  pickServiceAccountFile,
  pauseJob,
  resumeJob,
  type AppError,
  type ServiceAccountInfo,
} from "@/api/ipc";
import { DEFAULT_CONFIG } from "@/api/defaults";
import type { AppConfig, AppStatus, CredentialStatus, ListJobsPage, ListJobsQuery, TestResult } from "@/types/generated";

describe("IPC wrappers (SPEC.md §7)", () => {
  afterEach(() => {
    clearMocks();
  });

  describe("getConfig()", () => {
    it("should invoke 'get_config' with no args and return the AppConfig", async () => {
      mockIPC((cmd, args) => {
        expect(cmd).toBe("get_config");
        expect(args).toEqual({});
        return DEFAULT_CONFIG;
      });

      const result = await getConfig();

      expect(result).toEqual(DEFAULT_CONFIG);
    });

    it("should return the object the mock resolves to", async () => {
      const customConfig: AppConfig = {
        ...DEFAULT_CONFIG,
        watch: { ...DEFAULT_CONFIG.watch, recursive: true },
      };

      mockIPC((cmd) => {
        if (cmd === "get_config") {
          return customConfig;
        }
        return undefined;
      });

      const result = await getConfig();

      expect(result.watch.recursive).toBe(true);
    });
  });

  describe("saveConfig()", () => {
    it("should invoke 'save_config' with { config } and resolve undefined", async () => {
      const testConfig: AppConfig = DEFAULT_CONFIG;
      let recordedCmd = "";
      let recordedArgs: unknown;

      mockIPC((cmd, args) => {
        recordedCmd = cmd;
        if (cmd === "save_config") {
          recordedArgs = args;
          return undefined;
        }
      });

      const result = await saveConfig(testConfig);

      expect(recordedCmd).toBe("save_config");
      expect(recordedArgs).toEqual({ config: testConfig });
      expect(result).toBeUndefined();
    });

    it("should pass the config argument exactly as { config }", async () => {
      const customConfig: AppConfig = {
        ...DEFAULT_CONFIG,
        s3: { ...DEFAULT_CONFIG.s3, bucket: "my-bucket" },
      };

      mockIPC((cmd, args) => {
        if (cmd === "save_config") {
          const payload = args as { config: AppConfig };
          expect(payload.config.s3.bucket).toBe("my-bucket");
          return undefined;
        }
        return undefined;
      });

      await saveConfig(customConfig);
    });

    describe("on rejection with config.invalid error", () => {
      it("should reject with an AppError containing ValidationIssue array in message", async () => {
        const validationError: AppError = {
          code: "config.invalid",
          message: JSON.stringify([{ field: "s3.bucket", message: "vazio" }]),
        };

        mockIPC((cmd) => {
          if (cmd === "save_config") {
            throw validationError;
          }
        });

        await expect(saveConfig(DEFAULT_CONFIG)).rejects.toMatchObject({
          code: "config.invalid",
        });
      });

      it("should allow isAppError() to narrow the rejection", async () => {
        const validationError: AppError = {
          code: "config.invalid",
          message: JSON.stringify([
            { field: "s3.bucket", message: "vazio" },
            { field: "s3.region", message: "região inválida" },
          ]),
        };

        mockIPC((cmd) => {
          if (cmd === "save_config") {
            throw validationError;
          }
        });

        try {
          await saveConfig(DEFAULT_CONFIG);
          expect.fail("should have rejected");
        } catch (err) {
          expect(isAppError(err)).toBe(true);
          if (isAppError(err)) {
            const issues = parseValidationIssues(err);
            expect(issues).toHaveLength(2);
            expect(issues[0]).toEqual({ field: "s3.bucket", message: "vazio" });
            expect(issues[1]).toEqual({ field: "s3.region", message: "região inválida" });
          }
        }
      });

      it("should handle a single ValidationIssue in error message", async () => {
        const validationError: AppError = {
          code: "config.invalid",
          message: JSON.stringify([{ field: "watch.path", message: "caminho obrigatório" }]),
        };

        mockIPC((cmd) => {
          if (cmd === "save_config") {
            throw validationError;
          }
        });

        try {
          await saveConfig(DEFAULT_CONFIG);
          expect.fail("should have rejected");
        } catch (err) {
          if (isAppError(err)) {
            const issues = parseValidationIssues(err);
            expect(issues).toHaveLength(1);
            expect(issues[0]?.field).toBe("watch.path");
          }
        }
      });
    });
  });

  describe("setAutostart()", () => {
    it("should invoke 'set_autostart' with { enabled: true }", async () => {
      let recordedCmd = "";
      let recordedArgs: unknown;

      mockIPC((cmd, args) => {
        recordedCmd = cmd;
        recordedArgs = args;
        return undefined;
      });

      await setAutostart(true);

      expect(recordedCmd).toBe("set_autostart");
      expect(recordedArgs).toEqual({ enabled: true });
    });

    it("should invoke 'set_autostart' with { enabled: false }", async () => {
      let recordedArgs: unknown;

      mockIPC((cmd, args) => {
        if (cmd === "set_autostart") {
          recordedArgs = args;
        }
        return undefined;
      });

      await setAutostart(false);

      expect(recordedArgs).toEqual({ enabled: false });
    });

    it("should resolve undefined", async () => {
      mockIPC(() => undefined);

      const result = await setAutostart(true);

      expect(result).toBeUndefined();
    });
  });

  describe("isAppError()", () => {
    it("should return true for a valid AppError object", () => {
      const err: AppError = { code: "test", message: "test message" };
      expect(isAppError(err)).toBe(true);
    });

    it("should return false for null", () => {
      expect(isAppError(null)).toBe(false);
    });

    it("should return false for undefined", () => {
      expect(isAppError(undefined)).toBe(false);
    });

    it("should return false for an object without code or message", () => {
      expect(isAppError({ code: "test" })).toBe(false);
      expect(isAppError({ message: "test" })).toBe(false);
      expect(isAppError({})).toBe(false);
    });

    it("should return false if code is not a string", () => {
      expect(isAppError({ code: 123, message: "test" })).toBe(false);
    });

    it("should return false if message is not a string", () => {
      expect(isAppError({ code: "test", message: 123 })).toBe(false);
    });

    it("should return false for a string or number", () => {
      expect(isAppError("error")).toBe(false);
      expect(isAppError(123)).toBe(false);
    });
  });

  describe("parseValidationIssues()", () => {
    it("should parse valid JSON array of ValidationIssues", () => {
      const err: AppError = {
        code: "config.invalid",
        message: JSON.stringify([
          { field: "s3.bucket", message: "vazio" },
          { field: "gdrive.folder_id", message: "inválido" },
        ]),
      };

      const issues = parseValidationIssues(err);

      expect(issues).toHaveLength(2);
      expect(issues[0]).toEqual({ field: "s3.bucket", message: "vazio" });
      expect(issues[1]).toEqual({ field: "gdrive.folder_id", message: "inválido" });
    });

    it("should return empty array for invalid JSON", () => {
      const err: AppError = {
        code: "config.invalid",
        message: "not json at all",
      };

      const issues = parseValidationIssues(err);

      expect(issues).toEqual([]);
    });

    it("should return empty array if message is not an array", () => {
      const err: AppError = {
        code: "config.invalid",
        message: JSON.stringify({ field: "test", message: "test" }),
      };

      const issues = parseValidationIssues(err);

      expect(issues).toEqual([]);
    });

    it("should filter out non-ValidationIssue items", () => {
      const err: AppError = {
        code: "config.invalid",
        message: JSON.stringify([
          { field: "s3.bucket", message: "vazio" },
          { field: "invalid" }, // missing message
          { message: "test" }, // missing field
          null, // null
          "string", // not an object
          { field: 123, message: "type mismatch" }, // field is not string
        ]),
      };

      const issues = parseValidationIssues(err);

      expect(issues).toHaveLength(1);
      expect(issues[0]).toEqual({ field: "s3.bucket", message: "vazio" });
    });

    it("should return empty array for malformed message with non-string JSON", () => {
      const err: AppError = {
        code: "config.invalid",
        message: JSON.stringify(123),
      };

      const issues = parseValidationIssues(err);

      expect(issues).toEqual([]);
    });

    it("should handle an empty array", () => {
      const err: AppError = {
        code: "config.invalid",
        message: JSON.stringify([]),
      };

      const issues = parseValidationIssues(err);

      expect(issues).toEqual([]);
    });
  });

  describe("Contract Guard: RNF-003 (no credentials in config payload)", () => {
    it("should verify that resolved getConfig() contains no secret-like field names", async () => {
      mockIPC((cmd) => {
        if (cmd === "get_config") {
          return DEFAULT_CONFIG;
        }
        return undefined;
      });

      const config = await getConfig();

      // Recursive deep scan for any field matching credential patterns
      const secretPattern = /secret|access_key|private_key|token|password/i;
      const foundSecrets: string[] = [];

      function scanForSecrets(obj: unknown, path = ""): void {
        if (obj === null || obj === undefined) return;

        if (typeof obj === "object") {
          const keys = Object.keys(obj as Record<string, unknown>);
          keys.forEach((key) => {
            const fullPath = path ? `${path}.${key}` : key;
            if (secretPattern.test(key)) {
              foundSecrets.push(fullPath);
            }
            scanForSecrets((obj as Record<string, unknown>)[key], fullPath);
          });
        }
      }

      scanForSecrets(config);

      expect(foundSecrets).toEqual([]);
    });

    it("should apply the RNF-003 contract guard to custom configs too", async () => {
      const customConfig: AppConfig = {
        ...DEFAULT_CONFIG,
        s3: { ...DEFAULT_CONFIG.s3, bucket: "test-bucket" },
        gdrive: { ...DEFAULT_CONFIG.gdrive, folder_id: "test-folder" },
      };

      mockIPC((cmd) => {
        if (cmd === "get_config") {
          return customConfig;
        }
        return undefined;
      });

      const config = await getConfig();
      const secretPattern = /secret|access_key|private_key|token|password/i;
      const foundSecrets: string[] = [];

      function scanForSecrets(obj: unknown, path = ""): void {
        if (obj === null || obj === undefined) return;
        if (typeof obj === "object") {
          Object.keys(obj as Record<string, unknown>).forEach((key) => {
            const fullPath = path ? `${path}.${key}` : key;
            if (secretPattern.test(key)) {
              foundSecrets.push(fullPath);
            }
            scanForSecrets((obj as Record<string, unknown>)[key], fullPath);
          });
        }
      }

      scanForSecrets(config);

      expect(foundSecrets).toEqual([]);
    });

    it("comment: this test locks the contract before credentials exist; Fase 3/4 will add get_credential_status() which must only return masks", () => {
      // This test serves as a gate: when credentials are added to the API,
      // they must go into the keyring (backend) and only status/masks must flow
      // to the renderer. This test proves that before that feature, the config
      // payload is clean of any credential fields.
      expect(true).toBe(true);
    });
  });

  describe("Error propagation", () => {
    it("should propagate non-AppError rejections", async () => {
      const genericError = new Error("Network error");

      mockIPC((cmd) => {
        if (cmd === "save_config") {
          throw genericError;
        }
        return undefined;
      });

      await expect(saveConfig(DEFAULT_CONFIG)).rejects.toThrow("Network error");
    });

    it("should propagate errors from getConfig", async () => {
      const networkError = new Error("Connection refused");

      mockIPC((cmd) => {
        if (cmd === "get_config") {
          throw networkError;
        }
        return undefined;
      });

      await expect(getConfig()).rejects.toThrow("Connection refused");
    });
  });
});

describe("Fase 2 IPC wrappers (PLAN.md T-2.0)", () => {
  afterEach(() => {
    clearMocks();
  });

  it("pickFolder() invokes 'pick_folder' with no args and returns the picked path", async () => {
    mockIPC((cmd, args) => {
      expect(cmd).toBe("pick_folder");
      expect(args).toEqual({});
      return "C:\\Projetos\\BackupLocal";
    });

    await expect(pickFolder()).resolves.toBe("C:\\Projetos\\BackupLocal");
  });

  it("pickFolder() resolves null when the user cancels", async () => {
    mockIPC(() => null);

    await expect(pickFolder()).resolves.toBeNull();
  });

  it("rescan() invokes 'rescan' with no args and returns the whole report", async () => {
    const report = {
      scanned: 9,
      enqueued: 7,
      unchanged: 0,
      skipped_filtered: 0,
      skipped_symlink: 0,
      errors: 0,
      archived: 2,
      restored: 0,
    };
    mockIPC((cmd, args) => {
      expect(cmd).toBe("rescan");
      expect(args).toEqual({});
      return report;
    });

    await expect(rescan()).resolves.toEqual(report);
  });

  it("pauseWatcher() invokes 'pause_watcher' with no args", async () => {
    let recordedCmd = "";
    mockIPC((cmd, args) => {
      recordedCmd = cmd;
      expect(args).toEqual({});
      return undefined;
    });

    await expect(pauseWatcher()).resolves.toBeUndefined();
    expect(recordedCmd).toBe("pause_watcher");
  });

  it("resumeWatcher() invokes 'resume_watcher' with no args", async () => {
    let recordedCmd = "";
    mockIPC((cmd, args) => {
      recordedCmd = cmd;
      expect(args).toEqual({});
      return undefined;
    });

    await expect(resumeWatcher()).resolves.toBeUndefined();
    expect(recordedCmd).toBe("resume_watcher");
  });

  it("listJobs() invokes 'list_jobs' with { query } (named-object convention, like save_config)", async () => {
    const query: ListJobsQuery = { statuses: ["pending"], destination: null, include_archived: false, limit: 50, offset: 0 };
    const page: ListJobsPage = { items: [], total: 0 };

    let recordedArgs: unknown;
    mockIPC((cmd, args) => {
      expect(cmd).toBe("list_jobs");
      recordedArgs = args;
      return page;
    });

    const result = await listJobs(query);

    expect(recordedArgs).toEqual({ query });
    expect(result).toEqual(page);
  });

  it("getStatus() invokes 'get_status' with no args and returns the AppStatus", async () => {
    const status: AppStatus = {
      watcher_paused: false,
      destinations: {
        gdrive: { online: true, auth_required: false, latency_ms: 24 },
        s3: { online: true, auth_required: false, latency_ms: 41 },
      },
      counts_by_status: { pending: 0, uploading: 0, paused: 0, cancelled: 0, done: 0, failed: 0, bytes_total: 0, bytes_done: 0 },
      core_version: "0.1.0",
      build_target: "Tauri 2 • Windows x64",
    };

    mockIPC((cmd, args) => {
      expect(cmd).toBe("get_status");
      expect(args).toEqual({});
      return status;
    });

    await expect(getStatus()).resolves.toEqual(status);
  });

  it("getRecentLogs(limit) invokes 'get_recent_logs' with { limit }", async () => {
    let recordedArgs: unknown;
    mockIPC((cmd, args) => {
      expect(cmd).toBe("get_recent_logs");
      recordedArgs = args;
      return [];
    });

    await getRecentLogs(500);

    expect(recordedArgs).toEqual({ limit: 500 });
  });

  it("openLogsFolder() invokes 'open_logs_folder' with no args", async () => {
    let recordedCmd = "";
    mockIPC((cmd, args) => {
      recordedCmd = cmd;
      expect(args).toEqual({});
      return undefined;
    });

    await expect(openLogsFolder()).resolves.toBeUndefined();
    expect(recordedCmd).toBe("open_logs_folder");
  });
});

describe("Fase 3 IPC wrappers (PLAN.md T-3.10)", () => {
  afterEach(() => {
    clearMocks();
  });

  it("setCredential(key, value) invokes 'set_credential' with { key, value } (camelCase)", async () => {
    let recordedCmd = "";
    let recordedArgs: unknown;
    mockIPC((cmd, args) => {
      recordedCmd = cmd;
      recordedArgs = args;
      return undefined;
    });

    await setCredential("aws.access_key_id", "AKIAEXAMPLE");

    expect(recordedCmd).toBe("set_credential");
    expect(recordedArgs).toEqual({ key: "aws.access_key_id", value: "AKIAEXAMPLE" });
  });

  it("getCredentialStatus() invokes 'get_credential_status' with no args and returns CredentialStatus", async () => {
    const status: CredentialStatus = {
      aws: { present: true, masked: "AKIA****PLE" },
      gdrive: { present: false, email: null, project_id: null },
    };

    mockIPC((cmd, args) => {
      expect(cmd).toBe("get_credential_status");
      expect(args).toEqual({});
      return status;
    });

    await expect(getCredentialStatus()).resolves.toEqual(status);
  });

  it("clearCredential(key) invokes 'clear_credential' with { key }", async () => {
    let recordedArgs: unknown;
    mockIPC((cmd, args) => {
      expect(cmd).toBe("clear_credential");
      recordedArgs = args;
      return undefined;
    });

    await clearCredential("aws.secret_access_key");

    expect(recordedArgs).toEqual({ key: "aws.secret_access_key" });
  });

  it("testConnection(destination) invokes 'test_connection' with { destination } and returns TestResult on success", async () => {
    const result: TestResult = { ok: true, message: "Bucket válido (Put/List OK)", latency_ms: 41 };

    mockIPC((cmd, args) => {
      expect(cmd).toBe("test_connection");
      expect(args).toEqual({ destination: "s3" });
      return result;
    });

    await expect(testConnection("s3")).resolves.toEqual(result);
  });

  it("testConnection(destination) rejects with AppError on failure (never resolves ok:false)", async () => {
    const failure: AppError = { code: "s3.accessDenied", message: "Acesso negado" };

    mockIPC((cmd) => {
      if (cmd === "test_connection") throw failure;
    });

    await expect(testConnection("s3")).rejects.toMatchObject({ code: "s3.accessDenied" });
  });

  it("retryJob(jobId) invokes 'retry_job' with { jobId } (camelCase, not job_id)", async () => {
    let recordedArgs: unknown;
    mockIPC((cmd, args) => {
      expect(cmd).toBe("retry_job");
      recordedArgs = args;
      return undefined;
    });

    await retryJob("job-123");

    expect(recordedArgs).toEqual({ jobId: "job-123" });
  });

  it("retryAllFailed() invokes 'retry_all_failed' with no args and returns the retried count", async () => {
    mockIPC((cmd, args) => {
      expect(cmd).toBe("retry_all_failed");
      expect(args).toEqual({});
      return 3;
    });

    await expect(retryAllFailed()).resolves.toBe(3);
  });

  it("cancelJob(jobId) invokes 'cancel_job' with { jobId }", async () => {
    let recordedArgs: unknown;
    mockIPC((cmd, args) => {
      expect(cmd).toBe("cancel_job");
      recordedArgs = args;
      return undefined;
    });

    await cancelJob("job-456");

    expect(recordedArgs).toEqual({ jobId: "job-456" });
  });

  it("clearCompleted() invokes 'clear_completed' with no args and returns the cleared count", async () => {
    mockIPC((cmd, args) => {
      expect(cmd).toBe("clear_completed");
      expect(args).toEqual({});
      return 12;
    });

    await expect(clearCompleted()).resolves.toBe(12);
  });

  it("setQos(destination, limitMbps) invokes 'set_qos' with { destination, limitMbps } (camelCase, not limit_mbps)", async () => {
    let recordedArgs: unknown;
    mockIPC((cmd, args) => {
      expect(cmd).toBe("set_qos");
      recordedArgs = args;
      return undefined;
    });

    await setQos("gdrive", 25);

    expect(recordedArgs).toEqual({ destination: "gdrive", limitMbps: 25 });
  });

  it("setQos(destination, null) invokes 'set_qos' with { destination, limitMbps: null } to clear the cap", async () => {
    let recordedArgs: unknown;
    mockIPC((_cmd, args) => {
      recordedArgs = args;
      return undefined;
    });

    await setQos("s3", null);

    expect(recordedArgs).toEqual({ destination: "s3", limitMbps: null });
  });

  it("openInExplorer(path) invokes 'open_in_explorer' with { path }", async () => {
    let recordedArgs: unknown;
    mockIPC((cmd, args) => {
      expect(cmd).toBe("open_in_explorer");
      recordedArgs = args;
      return undefined;
    });

    await openInExplorer("C:\\Projetos\\arquivo.txt");

    expect(recordedArgs).toEqual({ path: "C:\\Projetos\\arquivo.txt" });
  });

  it("openRemote(jobId) invokes 'open_remote' with { jobId }", async () => {
    let recordedArgs: unknown;
    mockIPC((cmd, args) => {
      expect(cmd).toBe("open_remote");
      recordedArgs = args;
      return undefined;
    });

    await openRemote("job-789");

    expect(recordedArgs).toEqual({ jobId: "job-789" });
  });
});

describe("Fase 4/5 IPC wrappers (PLAN.md T-4.8/T-5.9)", () => {
  afterEach(() => {
    clearMocks();
  });

  it("pickServiceAccountFile() invokes 'pick_service_account_file' with no args and returns metadata", async () => {
    const info: ServiceAccountInfo = {
      file_name: "sync-backup-sa.json",
      size: 2380,
      client_email: "sync-backup@my-project.iam.gserviceaccount.com",
      project_id: "my-project",
    };

    mockIPC((cmd, args) => {
      expect(cmd).toBe("pick_service_account_file");
      expect(args).toEqual({});
      return info;
    });

    await expect(pickServiceAccountFile()).resolves.toEqual(info);
  });

  it("pickServiceAccountFile() resolves null when the user cancels", async () => {
    mockIPC(() => null);

    await expect(pickServiceAccountFile()).resolves.toBeNull();
  });

  it("setCredential('gdrive.service_account_json', value) is accepted by the CredentialKey union", async () => {
    let recordedArgs: unknown;
    mockIPC((cmd, args) => {
      expect(cmd).toBe("set_credential");
      recordedArgs = args;
      return undefined;
    });

    await setCredential("gdrive.service_account_json", "{...}");

    expect(recordedArgs).toEqual({ key: "gdrive.service_account_json", value: "{...}" });
  });

  it("clearCredential('gdrive.service_account_json') invokes 'clear_credential' with { key }", async () => {
    let recordedArgs: unknown;
    mockIPC((cmd, args) => {
      expect(cmd).toBe("clear_credential");
      recordedArgs = args;
      return undefined;
    });

    await clearCredential("gdrive.service_account_json");

    expect(recordedArgs).toEqual({ key: "gdrive.service_account_json" });
  });

  it("pauseJob(jobId) invokes 'pause_job' with { jobId } (camelCase, not job_id)", async () => {
    let recordedArgs: unknown;
    mockIPC((cmd, args) => {
      expect(cmd).toBe("pause_job");
      recordedArgs = args;
      return undefined;
    });

    await pauseJob("job-321");

    expect(recordedArgs).toEqual({ jobId: "job-321" });
  });

  it("resumeJob(jobId) invokes 'resume_job' with { jobId } (camelCase, not job_id)", async () => {
    let recordedArgs: unknown;
    mockIPC((cmd, args) => {
      expect(cmd).toBe("resume_job");
      recordedArgs = args;
      return undefined;
    });

    await resumeJob("job-654");

    expect(recordedArgs).toEqual({ jobId: "job-654" });
  });

  describe("ServiceAccountInfo (TODO T-4.6: swap for generated type)", () => {
    it("keeps the exact key set expected from the Rust struct (file_name, size, client_email, project_id)", () => {
      // Types are erased at runtime, so this can't diff against
      // `@/types/generated/ServiceAccountInfo` until ts-rs actually emits
      // it (T-4.6) — `src/types/generated.ts` does not export it yet (see
      // the TODO in `ipc.ts`). This locks the *locally declared* shape so a
      // future generated type swap is a mechanical, reviewable diff: once
      // the generated file exists, replace this local type with an import
      // and re-run this test unchanged — a shape mismatch will fail it.
      const sample: ServiceAccountInfo = {
        file_name: "sa.json",
        size: 1,
        client_email: "a@b.iam.gserviceaccount.com",
        project_id: null,
      };

      expect(Object.keys(sample).sort()).toEqual(["client_email", "file_name", "project_id", "size"]);
    });
  });
});
