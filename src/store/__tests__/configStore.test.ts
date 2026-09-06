import { beforeEach, describe, expect, it, vi } from "vitest";

import { DEFAULT_CONFIG } from "@/api/defaults";
import type { AppError } from "@/api/ipc";
import { tError } from "@/i18n";
import type { AppConfig, ValidationIssue } from "@/types/generated";

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
import { issueFor, useConfigStore } from "@/store/configStore";

const mockedGetConfig = vi.mocked(getConfig);
const mockedSaveConfig = vi.mocked(saveConfig);

const initialState = useConfigStore.getState();

function customConfig(overrides: Partial<AppConfig> = {}): AppConfig {
  return { ...DEFAULT_CONFIG, ...overrides };
}

beforeEach(() => {
  useConfigStore.setState(initialState, true);
  mockedGetConfig.mockReset();
  mockedSaveConfig.mockReset();
});

describe("configStore", () => {
  it("load() resolves getConfig() into config/saved and moves status to ready", async () => {
    const loaded = customConfig({ workers_per_destination: 3 });
    mockedGetConfig.mockResolvedValueOnce(loaded);

    await useConfigStore.getState().load();

    const state = useConfigStore.getState();
    expect(state.status).toBe("ready");
    expect(state.config).toEqual(loaded);
    expect(state.saved).toEqual(loaded);
    expect(state.dirty).toBe(false);
    expect(state.issues).toEqual([]);
  });

  it("a granular setter marks the store dirty", async () => {
    mockedGetConfig.mockResolvedValueOnce(customConfig());
    await useConfigStore.getState().load();

    useConfigStore.getState().setS3({ bucket: "meu-bucket" });

    const state = useConfigStore.getState();
    expect(state.dirty).toBe(true);
    expect(state.config.s3.bucket).toBe("meu-bucket");
    // Untouched sibling fields survive the patch.
    expect(state.config.s3.region).toBe(DEFAULT_CONFIG.s3.region);
  });

  it("save() success clears dirty, updates saved and stamps lastSavedAt", async () => {
    mockedGetConfig.mockResolvedValueOnce(customConfig());
    await useConfigStore.getState().load();
    useConfigStore.getState().setGeneral({ workers_per_destination: 4 });
    mockedSaveConfig.mockResolvedValueOnce(undefined);

    await useConfigStore.getState().save();

    const state = useConfigStore.getState();
    expect(mockedSaveConfig).toHaveBeenCalledWith(state.config);
    expect(state.status).toBe("ready");
    expect(state.dirty).toBe(false);
    expect(state.saved).toEqual(state.config);
    expect(state.lastSavedAt).not.toBeNull();
    expect(() => new Date(state.lastSavedAt as string).toISOString()).not.toThrow();
  });

  it("save() rejection with config.invalid populates issues without discarding the draft", async () => {
    mockedGetConfig.mockResolvedValueOnce(customConfig());
    await useConfigStore.getState().load();
    useConfigStore.getState().setS3({ enabled: true, bucket: "" });

    const issues: ValidationIssue[] = [{ field: "s3.bucket", message: "bucket must not be empty when S3 is enabled" }];
    const invalidError: AppError = { code: "config.invalid", message: JSON.stringify(issues) };
    mockedSaveConfig.mockRejectedValueOnce(invalidError);

    await useConfigStore.getState().save();

    const state = useConfigStore.getState();
    expect(state.status).toBe("ready");
    expect(state.issues).toEqual(issues);
    expect(issueFor(state.issues, "s3.bucket")).toBe("bucket must not be empty when S3 is enabled");
    // The invalid draft is preserved (not reverted to `saved`) so the user can fix it inline.
    expect(state.dirty).toBe(true);
    expect(state.config.s3.bucket).toBe("");
  });

  it("discard() restores config from the last saved snapshot", async () => {
    mockedGetConfig.mockResolvedValueOnce(customConfig());
    await useConfigStore.getState().load();
    const saved = useConfigStore.getState().saved;
    useConfigStore.getState().setQos({ s3_limit_mbps: 5 });
    expect(useConfigStore.getState().dirty).toBe(true);

    useConfigStore.getState().discard();

    const state = useConfigStore.getState();
    expect(state.config).toEqual(saved);
    expect(state.dirty).toBe(false);
    expect(state.issues).toEqual([]);
  });

  it("restoreDefaults() resets QoS/filters/workers/retry but keeps credentials, path and toggles", async () => {
    const customized = customConfig({
      watch: {
        path: "C:\\Users\\x\\Exportacoes",
        recursive: true,
        extensions: ["pdf", "csv"],
        min_size_mb: 10,
        max_size_mb: 9999,
        stabilize_seconds: 30,
      },
      s3: { ...DEFAULT_CONFIG.s3, enabled: true, bucket: "meu-bucket" },
      gdrive: { ...DEFAULT_CONFIG.gdrive, enabled: true, folder_id: "1AbC" },
      qos: {
        gdrive_limit_mbps: 2,
        s3_limit_mbps: 3,
        night_mode: { enabled: true, start: "22:00", end: "05:00" },
      },
      retry: { max_attempts: 9, base_delay_seconds: 40 },
      workers_per_destination: 4,
      autostart: false,
      keep_awake: false,
    });
    mockedGetConfig.mockResolvedValueOnce(customized);
    await useConfigStore.getState().load();

    useConfigStore.getState().restoreDefaults();

    const { config } = useConfigStore.getState();
    // Reset to factory values.
    expect(config.qos).toEqual(DEFAULT_CONFIG.qos);
    expect(config.retry).toEqual(DEFAULT_CONFIG.retry);
    expect(config.workers_per_destination).toBe(DEFAULT_CONFIG.workers_per_destination);
    expect(config.watch.recursive).toBe(DEFAULT_CONFIG.watch.recursive);
    expect(config.watch.extensions).toEqual(DEFAULT_CONFIG.watch.extensions);
    expect(config.watch.max_size_mb).toBe(DEFAULT_CONFIG.watch.max_size_mb);
    expect(config.watch.stabilize_seconds).toBe(DEFAULT_CONFIG.watch.stabilize_seconds);
    // Never touched: path, destinations (credentials-adjacent) and general toggles.
    expect(config.watch.path).toBe("C:\\Users\\x\\Exportacoes");
    expect(config.s3).toEqual(customized.s3);
    expect(config.gdrive).toEqual(customized.gdrive);
    expect(config.autostart).toBe(false);
    expect(config.keep_awake).toBe(false);
    expect(useConfigStore.getState().dirty).toBe(true);
  });

  it("a generic (non-AppError) rejection sets status to error with the tError('unknown') text", async () => {
    mockedGetConfig.mockRejectedValueOnce(new Error("network down"));

    await useConfigStore.getState().load();

    const state = useConfigStore.getState();
    expect(state.status).toBe("error");
    expect(state.error).toBe(tError("unknown"));
  });

  it("markQosSaved() folds only the qos slice into saved, leaving other dirty edits intact", async () => {
    mockedGetConfig.mockResolvedValueOnce(customConfig({ qos: { gdrive_limit_mbps: 1, s3_limit_mbps: 1, night_mode: DEFAULT_CONFIG.qos.night_mode } }));
    await useConfigStore.getState().load();

    // Modify QoS and another section.
    useConfigStore.getState().setS3({ bucket: "unsaved-bucket" });
    useConfigStore.getState().setQos({ gdrive_limit_mbps: 2 });
    expect(useConfigStore.getState().dirty).toBe(true);

    useConfigStore.getState().markQosSaved();

    const state = useConfigStore.getState();
    // QoS is saved.
    expect(state.saved.qos.gdrive_limit_mbps).toBe(2);
    // Other sections are not saved.
    expect(state.saved.s3.bucket).not.toBe("unsaved-bucket");
    // Still dirty because S3 changes are unsaved.
    expect(state.dirty).toBe(true);
    // Draft config unchanged.
    expect(state.config.s3.bucket).toBe("unsaved-bucket");
  });
});
