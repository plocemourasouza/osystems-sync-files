/**
 * `configStore` — zustand store backing the Configurações screen (PRD.md RF-083,
 * RF-084, RF-086; SPEC.md §7 `get_config`/`save_config`).
 *
 * `config` is the in-memory draft the UI edits; `saved` is the last snapshot
 * known to be persisted (from `load()` or a successful `save()`). `dirty` is
 * recomputed on every edit by deep-comparing `config` against `saved`.
 *
 * Convention (CLAUDE.md): this is the only place that imports `@/api/ipc`'s
 * config wrappers — components call the actions/selectors exported here.
 */
import { create } from "zustand";

import { DEFAULT_CONFIG } from "@/api/defaults";
import { getConfig, isAppError, parseValidationIssues, saveConfig } from "@/api/ipc";
import { isMockMode } from "@/dev/mockMode";
import { tError } from "@/i18n";
import type {
  AppConfig,
  GDriveConfig,
  QosConfig,
  RetryConfig,
  S3Config,
  ValidationIssue,
  WatchConfig,
} from "@/types/generated";

export type ConfigStatus = "idle" | "loading" | "ready" | "saving" | "error";

type GeneralPatch = Partial<Pick<AppConfig, "workers_per_destination" | "autostart" | "keep_awake">>;

interface ConfigState {
  status: ConfigStatus;
  config: AppConfig;
  /** Last snapshot known to be persisted — the baseline `dirty` compares against. */
  saved: AppConfig;
  dirty: boolean;
  issues: ValidationIssue[];
  error: string | null;
  lastSavedAt: string | null;
}

interface ConfigActions {
  load: () => Promise<void>;
  save: () => Promise<void>;
  discard: () => void;
  restoreDefaults: () => void;
  setWatch: (patch: Partial<WatchConfig>) => void;
  setS3: (patch: Partial<S3Config>) => void;
  setGDrive: (patch: Partial<GDriveConfig>) => void;
  setQos: (patch: Partial<QosConfig>) => void;
  markQosSaved: () => void;
  setRetry: (patch: Partial<RetryConfig>) => void;
  setGeneral: (patch: GeneralPatch) => void;
}

export type ConfigStore = ConfigState & ConfigActions;

/**
 * Structural deep-equality that tolerates `bigint` fields (`watch.max_size_mb`,
 * `watch.stabilize_seconds`, `retry.base_delay_seconds` — see `@/api/defaults`),
 * which `JSON.stringify` cannot serialize for a shortcut comparison.
 */
function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;

  if (typeof a === "bigint" || typeof b === "bigint") {
    return typeof a === typeof b && a === b;
  }

  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b)) return false;
    return a.length === b.length && a.every((value, index) => deepEqual(value, b[index]));
  }

  if (typeof a === "object" && a !== null && typeof b === "object" && b !== null) {
    const aRecord = a as Record<string, unknown>;
    const bRecord = b as Record<string, unknown>;
    const aKeys = Object.keys(aRecord);
    const bKeys = Object.keys(bRecord);
    if (aKeys.length !== bKeys.length) return false;
    return aKeys.every((key) => deepEqual(aRecord[key], bRecord[key]));
  }

  return false;
}

/** Reads a field's validation message, e.g. `issueFor(issues, "s3.bucket")`. */
export function issueFor(issues: ValidationIssue[], field: string): string | undefined {
  return issues.find((issue) => issue.field === field)?.message;
}

export const useConfigStore = create<ConfigStore>()((set, get) => ({
  status: "idle",
  config: DEFAULT_CONFIG,
  saved: DEFAULT_CONFIG,
  dirty: false,
  issues: [],
  error: null,
  lastSavedAt: null,

  load: async () => {
    // Dev preview (`?mock=1`, no Tauri runtime): `src/dev/mock.ts` already
    // seeded `config`/`saved` — a real `get_config` call here would just
    // reject and flash `status: "loading"` for nothing.
    if (isMockMode()) return;
    set({ status: "loading", error: null });
    try {
      const config = await getConfig();
      set({ status: "ready", config, saved: config, dirty: false, issues: [], error: null });
    } catch (e) {
      set({
        status: "error",
        error: isAppError(e) ? tError(e.code) : tError("unknown"),
      });
    }
  },

  save: async () => {
    const { config } = get();
    set({ status: "saving", error: null });
    try {
      await saveConfig(config);
      set({
        status: "ready",
        saved: config,
        dirty: false,
        issues: [],
        error: null,
        lastSavedAt: new Date().toISOString(),
      });
    } catch (e) {
      if (isAppError(e) && e.code === "config.invalid") {
        set({ status: "ready", issues: parseValidationIssues(e) });
        return;
      }
      set({
        status: "error",
        error: isAppError(e) ? tError(e.code) : tError("unknown"),
      });
    }
  },

  discard: () => {
    const { saved } = get();
    set({ config: saved, dirty: false, issues: [], error: null });
  },

  // RF-084: resets QoS, watcher filters/limits and workers/retry to factory
  // values only — never touches watch.path, s3.*, gdrive.* or the general
  // toggles (autostart/keep_awake), so credentials and destinations survive.
  restoreDefaults: () => {
    const { config } = get();
    const next: AppConfig = {
      ...config,
      watch: {
        ...config.watch,
        recursive: DEFAULT_CONFIG.watch.recursive,
        extensions: DEFAULT_CONFIG.watch.extensions,
        min_size_mb: DEFAULT_CONFIG.watch.min_size_mb,
        max_size_mb: DEFAULT_CONFIG.watch.max_size_mb,
        stabilize_seconds: DEFAULT_CONFIG.watch.stabilize_seconds,
      },
      qos: DEFAULT_CONFIG.qos,
      retry: DEFAULT_CONFIG.retry,
      workers_per_destination: DEFAULT_CONFIG.workers_per_destination,
    };
    set({ config: next, dirty: true });
  },

  setWatch: (patch) => {
    const { config, saved } = get();
    const next: AppConfig = { ...config, watch: { ...config.watch, ...patch } };
    set({ config: next, dirty: !deepEqual(next, saved) });
  },

  setS3: (patch) => {
    const { config, saved } = get();
    const next: AppConfig = { ...config, s3: { ...config.s3, ...patch } };
    set({ config: next, dirty: !deepEqual(next, saved) });
  },

  setGDrive: (patch) => {
    const { config, saved } = get();
    const next: AppConfig = { ...config, gdrive: { ...config.gdrive, ...patch } };
    set({ config: next, dirty: !deepEqual(next, saved) });
  },

  setQos: (patch) => {
    const { config, saved } = get();
    const next: AppConfig = { ...config, qos: { ...config.qos, ...patch } };
    set({ config: next, dirty: !deepEqual(next, saved) });
  },

  // `QosModule` (T-3.12/T-4.9): `set_qos` persists `config.json` on the Rust
  // side as soon as a slider commits, independent of the page's Ctrl+S save
  // cycle. Folding `config.qos` into `saved.qos` here — instead of a full
  // `load()`, which would clobber any other unsaved edit in flight — keeps
  // the footer's dirty indicator honest without disturbing the rest of the
  // draft.
  markQosSaved: () => {
    const { config, saved } = get();
    const nextSaved: AppConfig = { ...saved, qos: config.qos };
    set({ saved: nextSaved, dirty: !deepEqual(config, nextSaved) });
  },

  setRetry: (patch) => {
    const { config, saved } = get();
    const next: AppConfig = { ...config, retry: { ...config.retry, ...patch } };
    set({ config: next, dirty: !deepEqual(next, saved) });
  },

  setGeneral: (patch) => {
    const { config, saved } = get();
    const next: AppConfig = { ...config, ...patch };
    set({ config: next, dirty: !deepEqual(next, saved) });
  },
}));

/** Selector helper for a single `AppConfig` field, e.g. `useConfigField("autostart")`. */
export function useConfigField<K extends keyof AppConfig>(key: K): AppConfig[K] {
  return useConfigStore((state) => state.config[key]);
}
