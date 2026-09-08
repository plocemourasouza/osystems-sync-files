/**
 * Typed wrapper around Tauri's event system (SPEC.md §7).
 *
 * Convention (CLAUDE.md): components/hooks never call `listen` directly — they go
 * through `subscribe`/`useTauriEvent` here, mirroring `@/api/ipc` for commands.
 */
import { useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";

import type {
  AppStatus,
  AuthRequired,
  JobView,
  LogLine,
  RescanFailed,
  RescanProgress,
  Throughput,
  UploadProgress,
} from "@/types/generated";

/** Every event the Rust core emits (SPEC.md §7 "Events"). */
export type EventName =
  | "status-changed"
  | "job-updated"
  | "upload-progress"
  | "throughput"
  | "log-line"
  | "auth-required"
  | "rescan-failed"
  | "rescan-progress";

/** Payload type for each {@link EventName}. */
export interface EventPayloadMap {
  "status-changed": AppStatus;
  "job-updated": JobView;
  "upload-progress": UploadProgress;
  throughput: Throughput;
  "log-line": LogLine;
  "auth-required": AuthRequired;
  "rescan-failed": RescanFailed;
  "rescan-progress": RescanProgress;
}

export type PayloadOf<E extends EventName> = EventPayloadMap[E];

/**
 * Subscribes to a Tauri event with a payload type resolved from {@link EventName},
 * unwrapping Tauri's `Event<T>` envelope so `handler` receives just the payload.
 * Returns the `UnlistenFn` the caller must invoke to stop listening.
 */
export async function subscribe<E extends EventName>(
  name: E,
  handler: (payload: PayloadOf<E>) => void,
): Promise<UnlistenFn> {
  const unlisten = await listen<PayloadOf<E>>(name, (event) => {
    handler(event.payload);
  });
  // Cleanup must never throw into a React effect teardown (or a test's
  // unmount): outside the Tauri runtime (browser preview, jsdom) the event
  // plugin internals used by `unlisten` may be absent.
  return () => {
    try {
      // `UnlistenFn` is typed `() => void`, but the real implementation returns
      // the promise of an async `plugin:event|unlisten` call — a rejection there
      // would surface as an unhandled rejection, so swallow it explicitly.
      const result: unknown = unlisten();
      if (result && typeof (result as PromiseLike<unknown>).then === "function") {
        void Promise.resolve(result).catch(() => undefined);
      }
    } catch {
      // Listener is already gone or the runtime is not Tauri — nothing to undo.
    }
  };
}

/**
 * React hook wrapping {@link subscribe}: listens on mount, unlistens on unmount
 * or when `name` changes. `handler` is kept in a ref so callers can pass an
 * inline closure every render without re-subscribing.
 */
export function useTauriEvent<E extends EventName>(name: E, handler: (payload: PayloadOf<E>) => void): void {
  const handlerRef = useRef(handler);
  handlerRef.current = handler;

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let disposed = false;

    void subscribe(name, (payload) => {
      handlerRef.current(payload);
    })
      .then((fn) => {
        if (disposed) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch(() => {
        // Not running inside Tauri (browser preview / tests without an event
        // mock): the UI keeps working from polled state; nothing to subscribe.
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [name]);
}
