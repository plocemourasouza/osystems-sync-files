/**
 * `DEFAULT_CONFIG` — the renderer's mirror of `AppConfig::default()`
 * (`src-tauri/crates/core/src/config.rs`, SPEC.md §5).
 *
 * Used as the initial `configStore` state (before `load()` resolves) and as the
 * source of "factory" values for `restoreDefaults()` (PRD.md RF-084).
 *
 * `watch.max_size_mb`, `watch.stabilize_seconds` and `retry.base_delay_seconds`
 * are Rust `u64` fields — `ts-rs` maps `u64` to TypeScript `bigint`, not `number`
 * (see `src/types/generated/WatchConfig.ts`, `RetryConfig.ts`). Their literals
 * below must use the `n` suffix accordingly.
 */
import type { AppConfig } from "@/types/generated";

export const DEFAULT_CONFIG = {
  version: 1,
  watch: {
    path: null,
    recursive: false,
    extensions: [],
    min_size_mb: 0,
    max_size_mb: 0,
    stabilize_seconds: 3,
  },
  s3: {
    enabled: false,
    region: "us-east-1",
    bucket: "",
    prefix: "",
    storage_class: "STANDARD",
  },
  gdrive: {
    enabled: false,
    folder_id: "",
    auth_mode: "service_account",
    date_subfolders: false,
  },
  qos: {
    gdrive_limit_mbps: null,
    s3_limit_mbps: null,
    night_mode: {
      enabled: false,
      start: "23:00",
      end: "06:00",
    },
  },
  retry: {
    max_attempts: 5,
    base_delay_seconds: 5,
  },
  workers_per_destination: 2,
  autostart: true,
  keep_awake: true,
} satisfies AppConfig;
