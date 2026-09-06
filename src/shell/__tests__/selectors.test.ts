import { describe, expect, it } from "vitest";

import { selectStatusBarProps, selectTitleBarProps, toDestinationHealth } from "@/shell/selectors";
import type { AppStatus, DestinationHealth, StatusCounts } from "@/types/generated";

function counts(overrides: Partial<StatusCounts> = {}): StatusCounts {
  return { pending: 0, uploading: 0, paused: 0, cancelled: 0, done: 0, failed: 0, bytes_total: 0, bytes_done: 0, ...overrides };
}

function status(overrides: Partial<AppStatus> = {}): AppStatus {
  return {
    watcher_paused: false,
    destinations: {
      gdrive: { online: true, auth_required: false, latency_ms: 24 },
      s3: { online: true, auth_required: false, latency_ms: 41 },
    },
    counts_by_status: counts(),
    core_version: "0.1.0",
    build_target: "Tauri 2 • Windows x64",
    ...overrides,
  };
}

function destination(overrides: Partial<DestinationHealth> = {}): DestinationHealth {
  return { online: false, auth_required: false, latency_ms: null, ...overrides };
}

describe("toDestinationHealth", () => {
  it("maps an online destination with a latency to online + latencyMs", () => {
    expect(toDestinationHealth(destination({ online: true, latency_ms: 24 }))).toEqual({
      state: "online",
      latencyMs: 24,
    });
  });

  it("maps auth_required over online (credentials can expire while the transport still resolves)", () => {
    expect(toDestinationHealth(destination({ online: true, auth_required: true, latency_ms: 24 }))).toEqual({
      state: "auth_required",
      latencyMs: 24,
    });
  });

  it("maps neither online nor auth_required to offline", () => {
    expect(toDestinationHealth(destination({ online: false, auth_required: false }))).toEqual({
      state: "offline",
      latencyMs: undefined,
    });
  });

  it("converts a null latency_ms to an absent latencyMs", () => {
    expect(toDestinationHealth(destination({ online: true, latency_ms: null }))).toEqual({
      state: "online",
      latencyMs: undefined,
    });
  });

  it("maps `undefined` (no snapshot yet) to a plain offline state", () => {
    expect(toDestinationHealth(undefined)).toEqual({ state: "offline" });
  });
});

describe("selectStatusBarProps", () => {
  it("returns an inactive/offline placeholder while `status` is null", () => {
    expect(selectStatusBarProps(null, "us-east-1")).toEqual({
      coreVersion: "0.0.0",
      coreActive: false,
      gdrive: { state: "offline" },
      s3: { state: "offline" },
      buildTarget: "",
    });
  });

  it("derives coreVersion/coreActive/buildTarget and per-destination health from a real snapshot", () => {
    const props = selectStatusBarProps(status(), "us-east-1");

    expect(props.coreVersion).toBe("0.1.0");
    expect(props.coreActive).toBe(true);
    expect(props.buildTarget).toBe("Tauri 2 • Windows x64");
    expect(props.gdrive).toEqual({ state: "online", latencyMs: 24 });
    expect(props.s3).toEqual({ state: "online", latencyMs: 41, region: "us-east-1" });
  });

  it("surfaces auth_required on a single destination without disturbing the other", () => {
    const props = selectStatusBarProps(
      status({ destinations: { gdrive: destination({ auth_required: true }), s3: destination({ online: true, latency_ms: 41 }) } }),
      "us-east-1",
    );

    expect(props.gdrive).toEqual({ state: "auth_required", latencyMs: undefined });
    expect(props.s3).toEqual({ state: "online", latencyMs: 41, region: "us-east-1" });
  });
});

describe("selectTitleBarProps", () => {
  it("marks the daemon inactive while `status` is null, regardless of watchPath", () => {
    expect(selectTitleBarProps(null, "D:\\Watched")).toEqual({ watchPath: "D:\\Watched", daemonActive: false });
  });

  it("marks the daemon active once a status snapshot exists, even if the watcher itself is paused", () => {
    expect(selectTitleBarProps(status({ watcher_paused: true }), "D:\\Watched")).toEqual({
      watchPath: "D:\\Watched",
      daemonActive: true,
    });
  });

  it("passes a null watchPath through unchanged", () => {
    expect(selectTitleBarProps(status(), null)).toEqual({ watchPath: null, daemonActive: true });
  });
});
