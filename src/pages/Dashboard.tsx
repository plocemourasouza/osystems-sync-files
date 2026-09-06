/**
 * Dashboard — `/dashboard` route (RF-060 a RF-064, RF-070, RF-037; DESIGN.md
 * §9 region map).
 *
 * Final composition (T-2.12, T-5.8): header → KPI row → JobTable →
 * LogConsole. When `configStore.config.watch.path` is `null` (RF-070's "sem
 * pasta configurada" empty state), the KPI row and table are replaced by
 * `EmptyStateNoFolder`. When a folder IS configured but neither destination
 * has a saved credential, `EmptyStateNoCredentials` renders ABOVE the KPI
 * row/table — a nudge, not a gate: both stay mounted below so files already
 * detected in that folder remain visible. The header keeps showing the page
 * title/watcher badge in every case, and `LogConsole` stays mounted (logs
 * are useful even before a folder is picked).
 *
 * Layout: the page fills `<main>` (`h-full`) as a flex column instead of
 * flowing freely inside its scroller, so `LogConsole` sits docked at the
 * bottom and the job list — the one region that grows without bound — takes
 * whatever vertical space is left. Collapsing the console therefore *hands*
 * its ~220px to the table rather than just shortening the page. Everything
 * else (header, banner, KPI row, console) is `shrink-0`; only the table
 * region is `flex-1 min-h-0`, and it keeps a floor (`min-h-[200px]`, set on
 * `JobTable` itself) so a short window makes `<main>` scroll rather than
 * squashing the rows to nothing.
 *
 * `configStore` is loaded on mount the same way `Settings.tsx` does it
 * (guarded by `status === "idle"` so remounts/navigations don't refetch) —
 * this page is the other consumer of `watch.path`, so it needs the config
 * loaded independently of whether `/settings` has been visited yet.
 * `credentialsStore` has no such guard (no `status: "idle"`-style load flag,
 * just `status: CredentialStatus | null`) — refreshed unconditionally on
 * mount, mirroring `statusStore.refresh()` below, since no other page
 * currently refreshes it.
 */
import { useEffect } from "react";
import { useTauriEvent } from "@/api/events";
import {
  AuthRequiredBanner,
  DashboardHeader,
  EmptyStateNoCredentials,
  EmptyStateNoFolder,
  JobTable,
  KpiRow,
  LogConsole,
} from "@/components/dashboard";
import { useConfigStore } from "@/store/configStore";
import { useCredentialsStore } from "@/store/credentialsStore";
import { useStatusStore } from "@/store/statusStore";

export function Dashboard() {
  const refresh = useStatusStore((s) => s.refresh);
  const applyEvent = useStatusStore((s) => s.applyEvent);

  const configStatus = useConfigStore((s) => s.status);
  const loadConfig = useConfigStore((s) => s.load);
  const watchPath = useConfigStore((s) => s.config.watch.path);

  const refreshCredentials = useCredentialsStore((s) => s.refresh);
  const credentialsStatus = useCredentialsStore((s) => s.status);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    if (configStatus === "idle") {
      void loadConfig();
    }
  }, [configStatus, loadConfig]);

  useEffect(() => {
    void refreshCredentials();
  }, [refreshCredentials]);

  useTauriEvent("status-changed", applyEvent);

  const hasFolder = watchPath !== null;
  const hasAnyCredential = (credentialsStatus?.aws.present ?? false) || (credentialsStatus?.gdrive.present ?? false);
  const showNoCredentials = hasFolder && !hasAnyCredential;

  return (
    <section aria-labelledby="dashboard-title" className="flex h-full min-h-0 flex-col gap-md">
      <DashboardHeader />
      <AuthRequiredBanner />

      {hasFolder ? (
        <>
          {showNoCredentials && <EmptyStateNoCredentials />}
          <KpiRow />
          <JobTable />
        </>
      ) : (
        <EmptyStateNoFolder />
      )}

      <LogConsole />
    </section>
  );
}
