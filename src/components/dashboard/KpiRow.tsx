/**
 * KpiRow — the Dashboard's 5-KPI grid, sourced from `selectKpis(status)`
 * (DESIGN.md §9 region map; PRD.md RF-060). Responsive grid: stacks on
 * narrow widths, 5 columns from `xl` up (Tailwind's default breakpoint,
 * approximating DESIGN.md's 1440px — see Settings.tsx's `xl:` precedent).
 */
import type { JSX } from "react";
import { AlertTriangle, CheckCircle2, Hourglass, Package, RefreshCw } from "lucide-react";
import { KpiCard } from "@/components/dashboard/KpiCard";
import { Badge } from "@/components/ui/Badge";
import { t } from "@/i18n";
import { formatBytes } from "@/lib/format";
import { selectKpis, useStatusStore } from "@/store/statusStore";

export function KpiRow(): JSX.Element {
  const status = useStatusStore((s) => s.status);
  const storeLoading = useStatusStore((s) => s.loading);
  const kpis = selectKpis(status);
  const loading = storeLoading && status === null;

  return (
    <div className="grid grid-cols-1 gap-sm sm:grid-cols-2 xl:grid-cols-5">
      <KpiCard
        label={t("pages.dashboard.kpi.detected.label")}
        value={kpis.detected}
        sub={<span className="font-mono text-label-sm text-text-secondary">{formatBytes(kpis.bytesTotal)}</span>}
        icon={Package}
        loading={loading}
      />
      <KpiCard
        label={t("pages.dashboard.kpi.done.label")}
        value={kpis.done}
        tone="success"
        sub={
          <Badge tone="success">{t("pages.dashboard.kpi.done.sub", { pct: kpis.donePct })}</Badge>
        }
        icon={CheckCircle2}
        loading={loading}
      />
      <KpiCard
        label={t("pages.dashboard.kpi.uploading.label")}
        value={kpis.uploading}
        tone="info"
        sub={<Badge tone="info">{t("pages.dashboard.kpi.uploading.sub")}</Badge>}
        icon={RefreshCw}
        loading={loading}
      />
      <KpiCard
        label={t("pages.dashboard.kpi.queued.label")}
        value={kpis.queued}
        sub={<span className="font-mono text-label-sm text-text-secondary">{t("pages.dashboard.kpi.queued.sub")}</span>}
        icon={Hourglass}
        loading={loading}
      />
      <KpiCard
        label={t("pages.dashboard.kpi.failed.label")}
        value={kpis.failed}
        tone={kpis.failed > 0 ? "error" : "neutral"}
        sub={
          kpis.failed > 0 ? (
            <Badge tone="error">{t("pages.dashboard.kpi.failed.sub")}</Badge>
          ) : undefined
        }
        icon={AlertTriangle}
        loading={loading}
      />
    </div>
  );
}
