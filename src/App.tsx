/**
 * App — shell composition + routing (T-0.6, RF-060…RF-086).
 *
 * Wraps the two routes (`/dashboard`, `/settings`) with the fixed shell
 * (`TitleBar` + `Sidebar` + `StatusBar`, `design/DESIGN.md` §4/§8): a 3-row
 * grid (titlebar / body / statusbar) with the body split into a 2-column
 * grid (sidebar / content). The router itself lives in `main.tsx` so tests
 * can swap in a `MemoryRouter`.
 *
 * `Sidebar` is fed by the real stores (PRD.md RF-040/RF-067, PLAN.md
 * T-3.11): `queueCount`/`watcherActive` from `statusStore`, `watchPath` from
 * `configStore`, and `throughput` from `throughputStore`, which this
 * component hydrates by subscribing once to the `throughput` Tauri event
 * (there is no snapshot IPC command — see `throughputStore`'s doc comment).
 * `TitleBar`/`StatusBar` are fed the same way (T-5.6, PRD.md RF-068/RF-069/
 * RF-097): `selectTitleBarProps`/`selectStatusBarProps` (`shell/selectors.ts`)
 * derive their props from `statusStore`'s `AppStatus` snapshot and
 * `configStore`'s `watch.path`/`s3.region` — no more shell mock fixture. `Dashboard`
 * is the component that calls `statusStore.refresh()` on mount and subscribes
 * to `status-changed`; since it's the router's default route, that's enough
 * to hydrate the shared `status` this component reads here too.
 */
import { Navigate, Route, Routes } from "react-router-dom";
import { useTauriEvent } from "./api/events";
import { Sidebar, StatusBar, TitleBar } from "./components/shell";
import { Dashboard, Settings } from "./pages";
import { selectStatusBarProps, selectTitleBarProps } from "./shell/selectors";
import { t } from "./i18n";
import { useConfigStore } from "./store/configStore";
import { selectKpis, useStatusStore } from "./store/statusStore";
import { selectCapBps, useThroughputStore } from "./store/throughputStore";

export function App() {
  const status = useStatusStore((s) => s.status);
  const watchPath = useConfigStore((s) => s.config.watch.path);
  const s3Region = useConfigStore((s) => s.config.s3.region);

  const totalBps = useThroughputStore((s) => s.totalBps);
  const gdriveBps = useThroughputStore((s) => s.gdriveBps);
  const s3Bps = useThroughputStore((s) => s.s3Bps);
  const capBps = useThroughputStore(selectCapBps);
  const applyThroughput = useThroughputStore((s) => s.apply);

  useTauriEvent("throughput", applyThroughput);

  const kpis = selectKpis(status);
  const titleBarProps = selectTitleBarProps(status, watchPath);
  const statusBarProps = selectStatusBarProps(status, s3Region);

  return (
    <div className="grid h-screen w-screen grid-rows-[var(--spacing-titlebar)_1fr_var(--spacing-statusbar)] overflow-hidden bg-surface-0 font-sans text-text-primary">
      <a
        href="#content"
        className="sr-only focus:not-sr-only focus:absolute focus:left-sm focus:top-sm focus:z-50 focus:rounded-md focus:bg-surface-2 focus:px-sm focus:py-xs focus:text-body-sm focus:text-text-primary focus-visible:outline-none focus-visible:shadow-focus"
      >
        {t("common.accessibility.skipToContent")}
      </a>

      <TitleBar {...titleBarProps} />

      <div className="grid min-h-0 grid-cols-[var(--spacing-sidebar)_1fr]">
        <Sidebar
          queueCount={kpis.queued + kpis.uploading}
          watchPath={watchPath}
          watcherActive={!(status?.watcher_paused ?? false)}
          throughput={{ totalBps, gdriveBps, s3Bps, capBps }}
        />

        <main id="content" className="min-h-0 overflow-y-auto overflow-x-hidden p-xl">
          <Routes>
            <Route path="/dashboard" element={<Dashboard />} />
            <Route path="/settings" element={<Settings />} />
            <Route path="/" element={<Navigate to="/dashboard" replace />} />
            <Route path="*" element={<Navigate to="/dashboard" replace />} />
          </Routes>
        </main>
      </div>

      <StatusBar {...statusBarProps} />
    </div>
  );
}
