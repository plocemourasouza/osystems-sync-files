/**
 * DashboardHeader — page title, watcher status badge, and the quick actions
 * from PRD.md RF-034/RF-036/RF-061 (Atualizar Lista, Pausar/Retomar Watcher,
 * Reenviar Falhas, Limpar Concluídos). Owns the `<h1 id="dashboard-title">`
 * landmark that `Dashboard.tsx`'s `aria-labelledby` points at.
 *
 * "Reenviar Falhas" (RF-034) and "Limpar Concluídos" (RF-036) call
 * `@/api/ipc` directly rather than through `statusStore`/`jobsStore` — those
 * two bulk commands don't belong to either store's domain (mirrors
 * `RowActions`, which does the same for `retryJob`/`cancelJob`). Both then
 * refresh `statusStore` (KPI counts) and `jobsStore` (current table page) so
 * the UI reflects the change immediately instead of waiting on per-job
 * `job-updated` events to trickle in.
 */
import type { JSX } from "react";
import { useEffect, useRef, useState } from "react";
import { Pause, Play, RefreshCw, RotateCw, Trash2 } from "lucide-react";
import { clearCompleted, retryAllFailed } from "@/api/ipc";
import { Button } from "@/components/ui/Button";
import { t } from "@/i18n";
import { useJobsStore } from "@/store/jobsStore";
import { selectKpis, useStatusStore } from "@/store/statusStore";
import type { RescanReport } from "@/types/generated";

const ACTION_COUNT_DISPLAY_MS = 3000;

export function DashboardHeader(): JSX.Element {
  const status = useStatusStore((s) => s.status);
  const { rescan, pauseWatcher, resumeWatcher } = useStatusStore((s) => s.actions);
  const refreshStatus = useStatusStore((s) => s.refresh);
  const refetchJobs = useJobsStore((s) => s.fetch);

  const [rescanning, setRescanning] = useState(false);
  // The whole report, not just a count: a rescan now both enqueues and
  // archives, and the header reports each half separately.
  const [rescanResult, setRescanResult] = useState<RescanReport | null>(null);
  const [togglingWatcher, setTogglingWatcher] = useState(false);
  const [retryingFailed, setRetryingFailed] = useState(false);
  const [retryFailedCount, setRetryFailedCount] = useState<number | null>(null);
  const [clearingDone, setClearingDone] = useState(false);
  const [clearDoneCount, setClearDoneCount] = useState<number | null>(null);

  const rescanCountTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const retryFailedCountTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const clearDoneCountTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => {
      if (rescanCountTimer.current) clearTimeout(rescanCountTimer.current);
      if (retryFailedCountTimer.current) clearTimeout(retryFailedCountTimer.current);
      if (clearDoneCountTimer.current) clearTimeout(clearDoneCountTimer.current);
    };
  }, []);

  const watcherPaused = status?.watcher_paused ?? false;
  const kpis = selectKpis(status);

  const handleRescan = async (): Promise<void> => {
    setRescanning(true);
    try {
      const report = await rescan();
      setRescanResult(report);
      if (rescanCountTimer.current) clearTimeout(rescanCountTimer.current);
      rescanCountTimer.current = setTimeout(() => {
        setRescanResult(null);
      }, ACTION_COUNT_DISPLAY_MS);
      // `rescan()` refreshes the status counters itself, but nothing refreshes
      // the table — and a rescan now enqueues *and* archives, so the rows on
      // screen are stale the moment it returns. Without this the KPIs moved
      // while the list sat still until the user touched the filter.
      await refetchJobs({ force: true });
    } finally {
      setRescanning(false);
    }
  };

  const handleToggleWatcher = async (): Promise<void> => {
    setTogglingWatcher(true);
    try {
      if (watcherPaused) {
        await resumeWatcher();
      } else {
        await pauseWatcher();
      }
    } finally {
      setTogglingWatcher(false);
    }
  };

  const handleRetryFailed = async (): Promise<void> => {
    setRetryingFailed(true);
    try {
      const retried = await retryAllFailed();
      setRetryFailedCount(retried);
      if (retryFailedCountTimer.current) clearTimeout(retryFailedCountTimer.current);
      retryFailedCountTimer.current = setTimeout(() => {
        setRetryFailedCount(null);
      }, ACTION_COUNT_DISPLAY_MS);
      await Promise.all([refreshStatus(), refetchJobs({ force: true })]);
    } finally {
      setRetryingFailed(false);
    }
  };

  const handleClearDone = async (): Promise<void> => {
    setClearingDone(true);
    try {
      const cleared = await clearCompleted();
      setClearDoneCount(cleared);
      if (clearDoneCountTimer.current) clearTimeout(clearDoneCountTimer.current);
      clearDoneCountTimer.current = setTimeout(() => {
        setClearDoneCount(null);
      }, ACTION_COUNT_DISPLAY_MS);
      await Promise.all([refreshStatus(), refetchJobs({ force: true })]);
    } finally {
      setClearingDone(false);
    }
  };

  return (
    <div className="flex flex-wrap items-center justify-between gap-sm">
      {/* The watcher badge used to live here; it now sits opposite the filter
          bar on the table's toolbar row (`WatcherStatusBadge`), next to the
          list whose count it reports. */}
      <h1 id="dashboard-title" className="text-headline-lg text-text-primary">
        {t("pages.dashboard.title")}
      </h1>

      <div className="flex items-center gap-sm">
        {rescanResult !== null && (
          <span aria-live="polite" className="font-mono text-label-sm text-text-secondary">
            {t("pages.dashboard.actions.rescanQueued", { count: rescanResult.enqueued })}
          </span>
        )}
        {/* Only when it happened: on a normal rescan nothing is archived, and a
            permanent "0 removidos" would be noise next to the enqueued count. */}
        {rescanResult !== null && rescanResult.archived > 0 && (
          <span aria-live="polite" className="font-mono text-label-sm text-text-secondary">
            {t("pages.dashboard.actions.rescanArchived", { count: rescanResult.archived })}
          </span>
        )}
        {rescanResult !== null && rescanResult.restored > 0 && (
          <span aria-live="polite" className="font-mono text-label-sm text-text-secondary">
            {t("pages.dashboard.actions.rescanRestored", { count: rescanResult.restored })}
          </span>
        )}
        {retryFailedCount !== null && (
          <span aria-live="polite" className="font-mono text-label-sm text-text-secondary">
            {t("pages.dashboard.actions.retryFailedCount", { count: retryFailedCount })}
          </span>
        )}
        {clearDoneCount !== null && (
          <span aria-live="polite" className="font-mono text-label-sm text-text-secondary">
            {t("pages.dashboard.actions.clearDoneCount", { count: clearDoneCount })}
          </span>
        )}

        <Button
          variant="ghost"
          size="sm"
          icon={<RefreshCw size={16} aria-hidden="true" />}
          loading={rescanning}
          onClick={() => void handleRescan()}
        >
          {t("pages.dashboard.actions.rescan")}
        </Button>

        <Button
          variant="secondary"
          size="sm"
          icon={watcherPaused ? <Play size={16} aria-hidden="true" /> : <Pause size={16} aria-hidden="true" />}
          loading={togglingWatcher}
          onClick={() => void handleToggleWatcher()}
        >
          {watcherPaused ? t("pages.dashboard.actions.resume") : t("pages.dashboard.actions.pause")}
        </Button>

        {kpis.failed > 0 && (
          <Button
            variant="secondary"
            size="sm"
            icon={<RotateCw size={16} aria-hidden="true" />}
            loading={retryingFailed}
            onClick={() => void handleRetryFailed()}
          >
            {t("pages.dashboard.actions.retryFailed")}
          </Button>
        )}

        <Button
          variant="primary"
          size="sm"
          icon={<Trash2 size={16} aria-hidden="true" />}
          loading={clearingDone}
          onClick={() => void handleClearDone()}
        >
          {t("pages.dashboard.actions.clearDone")}
        </Button>
      </div>
    </div>
  );
}
