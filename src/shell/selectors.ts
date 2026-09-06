/**
 * Shell selectors — derive `TitleBarProps`/`StatusBarProps` from the real
 * `AppStatus` snapshot held by `statusStore`, replacing the old shell mock fixture (T-5.6;
 * PRD.md RF-068 "health por destino", RF-069 "ping", RF-097 "badge Daemon:
 * Active"). Mirrors the pattern `statusStore.ts` already uses for
 * `selectKpis`: pure functions, no store/React import, unit-testable with
 * plain fixtures.
 *
 * Kept in `shell/` (not inside `statusStore.ts`) because these map the
 * backend's `DestinationHealth` (`online`/`auth_required`/`latency_ms`) onto
 * `StatusBar`'s UI-facing tri-state `DestinationHealth`
 * (`online`/`offline`/`auth_required` + optional `latencyMs`) — a
 * shell-presentation concern, not store state.
 */
import type { DestinationHealth as UiDestinationHealth, StatusBarProps } from "@/components/shell/StatusBar";
import type { TitleBarProps } from "@/components/shell/TitleBar";
import type { AppStatus, DestinationHealth as BackendDestinationHealth } from "@/types/generated";

const OFFLINE: UiDestinationHealth = { state: "offline" };

/**
 * Maps a backend `DestinationHealth` onto `StatusBar`'s UI shape.
 *
 * `auth_required` takes priority over `online` (a destination's transport
 * can resolve while its stored credentials no longer authenticate).
 * `latencyMs` is carried through whenever the backend reports a number,
 * regardless of state — a last-known ping is still useful context right
 * after a destination flips to `auth_required`/offline. `undefined` (no
 * snapshot yet, e.g. before the first `get_status`) maps to a plain offline
 * state with no latency.
 */
export function toDestinationHealth(destination: BackendDestinationHealth | undefined): UiDestinationHealth {
  if (destination === undefined) return OFFLINE;

  const latencyMs = destination.latency_ms ?? undefined;
  if (destination.auth_required) return { state: "auth_required", latencyMs };
  if (destination.online) return { state: "online", latencyMs };
  return { state: "offline", latencyMs };
}

/** `StatusBar` props while `status` is `null` (not yet loaded / IPC unreachable). */
const LOADING_STATUS_BAR_PROPS: StatusBarProps = {
  coreVersion: "0.0.0",
  coreActive: false,
  gdrive: OFFLINE,
  s3: OFFLINE,
  buildTarget: "",
};

/**
 * Derives `StatusBarProps` from `statusStore`'s `AppStatus` snapshot
 * (RF-068) and `configStore.config.s3.region` (RF-068's "(region)" suffix).
 * `coreActive` is `status !== null`: the core only answers `get_status` once
 * it is up, so a resolved snapshot already proves it is active.
 */
export function selectStatusBarProps(status: AppStatus | null, s3Region: string): StatusBarProps {
  if (status === null) return LOADING_STATUS_BAR_PROPS;

  return {
    coreVersion: status.core_version,
    coreActive: true,
    gdrive: toDestinationHealth(status.destinations.gdrive),
    s3: { ...toDestinationHealth(status.destinations.s3), region: s3Region },
    buildTarget: status.build_target,
  };
}

/**
 * Derives `TitleBarProps` from `statusStore`'s `AppStatus` snapshot and
 * `configStore.config.watch.path` (RF-097). `daemonActive` reflects whether
 * the Rust core has answered `get_status` at all — distinct from the
 * sidebar's watcher badge (`!status.watcher_paused`), which reflects
 * RF-012's "Pausar Watcher" toggle on an already-running core.
 */
export function selectTitleBarProps(status: AppStatus | null, watchPath: string | null): TitleBarProps {
  return {
    watchPath,
    daemonActive: status !== null,
  };
}
