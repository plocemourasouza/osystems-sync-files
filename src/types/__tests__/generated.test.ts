import { describe, expect, it } from "vitest";

import type { AppConfig, LogLine } from "@/types/generated";

// This test exists to prove `src/types/generated.ts` (the ts-rs barrel,
// PLAN.md T-1.4 / SPEC.md §7 / PRD.md RNF-010) is actually consumable from
// application code via the `@/*` path alias — not just present on disk.
// The real guarantee is compile-time: if `AppConfig` or `LogLine` drift out
// of sync with the Rust structs, these literals stop type-checking and
// `npm run typecheck` fails.

describe("generated IPC types", () => {
  it("AppConfig: a literal object satisfies the generated type", () => {
    const config: AppConfig = {
      version: 1,
      watch: {
        path: null,
        recursive: true,
        extensions: ["jpg", "png"],
        min_size_mb: 0,
        max_size_mb: 500,
        stabilize_seconds: 5,
      },
      s3: {
        enabled: false,
        region: "us-east-1",
        bucket: "my-bucket",
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
        s3_limit_mbps: 5,
        night_mode: { enabled: false, start: "22:00", end: "06:00" },
      },
      retry: { max_attempts: 5, base_delay_seconds: 2 },
      workers_per_destination: 2,
      autostart: true,
      keep_awake: true,
    };

    expect(config.version).toBe(1);
  });

  it("LogLine: a literal object satisfies the generated type", () => {
    const line: LogLine = {
      ts: "2026-09-04T13:00:00.000Z",
      level: "INFO",
      target: "osystems_sync_core::watcher",
      job_id: null,
      destination: null,
      message: "watcher started",
      error: null,
      path: null,
    };

    expect(line.level).toBe("INFO");
  });
});
