import { afterEach, describe, expect, it, vi } from "vitest";
import { render, cleanup } from "@testing-library/react";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";

import { subscribe, useTauriEvent } from "@/api/events";
import type { JobView } from "@/types/generated";
import { makeJob } from "@/store/__fixtures__/jobs";

describe("subscribe() (SPEC.md §7 events)", () => {
  afterEach(() => {
    clearMocks();
    cleanup();
  });

  it("registers a listener via `listen` and forwards the unwrapped payload", async () => {
    mockIPC(() => undefined, { shouldMockEvents: true });

    const handler = vi.fn();
    const unlisten = await subscribe("job-updated", handler);

    const job: JobView = makeJob();
    await emit("job-updated", job);

    expect(handler).toHaveBeenCalledTimes(1);
    expect(handler).toHaveBeenCalledWith(job);

    unlisten();
  });

  it("stops receiving events once unlisten() is called", async () => {
    mockIPC(() => undefined, { shouldMockEvents: true });

    const handler = vi.fn();
    const unlisten = await subscribe("status-changed", handler);
    unlisten();

    await emit("status-changed", { watcher_paused: true });

    expect(handler).not.toHaveBeenCalled();
  });
});

describe("useTauriEvent() (React hook)", () => {
  afterEach(() => {
    clearMocks();
    cleanup();
  });

  it("subscribes on mount and receives events", async () => {
    mockIPC(() => undefined, { shouldMockEvents: true });
    const handler = vi.fn();

    function Probe() {
      useTauriEvent("log-line", handler);
      return null;
    }

    render(<Probe />);
    // Effects run synchronously in React 19's test renderer, but the
    // subscription itself is async (`await listen(...)`); flush microtasks.
    await Promise.resolve();
    await Promise.resolve();

    await emit("log-line", { ts: "2026-01-01T00:00:00.000Z", level: "INFO", target: "core", job_id: null, destination: null, message: "hi" });

    expect(handler).toHaveBeenCalledTimes(1);
  });

  it("unlistens on unmount", async () => {
    mockIPC(() => undefined, { shouldMockEvents: true });
    const handler = vi.fn();

    function Probe() {
      useTauriEvent("log-line", handler);
      return null;
    }

    const { unmount } = render(<Probe />);
    await new Promise((resolve) => setTimeout(resolve, 0));

    unmount();
    // `unlisten()` (Tauri's `_unlisten`) is itself async — it round-trips
    // through `invoke('plugin:event|unlisten', ...)` — so give it a macrotask
    // to actually deregister before emitting.
    await new Promise((resolve) => setTimeout(resolve, 0));

    await emit("log-line", { ts: "2026-01-01T00:00:00.000Z", level: "INFO", target: "core", job_id: null, destination: null, message: "hi" });

    expect(handler).not.toHaveBeenCalled();
  });

  it("keeps the subscription stable across re-renders with a new inline handler (handler ref)", async () => {
    mockIPC(() => undefined, { shouldMockEvents: true });
    const calls: number[] = [];

    function Probe({ tag }: { tag: number }) {
      useTauriEvent("log-line", () => {
        calls.push(tag);
      });
      return null;
    }

    const { rerender } = render(<Probe tag={1} />);
    await Promise.resolve();
    await Promise.resolve();

    rerender(<Probe tag={2} />);

    await emit("log-line", { ts: "2026-01-01T00:00:00.000Z", level: "INFO", target: "core", job_id: null, destination: null, message: "hi" });

    // Only one subscription should exist (effect deps = [name], stable), and
    // it must call the *latest* handler (tag 2), not stack a second listener.
    expect(calls).toEqual([2]);
  });
});
