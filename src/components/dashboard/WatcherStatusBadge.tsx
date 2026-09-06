/**
 * WatcherStatusBadge — "Watcher: Ativo · monitorando N arquivos" / "Watcher:
 * Pausado" (PRD.md RF-060; DESIGN.md §9 `/dashboard`).
 *
 * Sits opposite the `JobsFilterBar` on the table's toolbar row rather than
 * next to the page title: the count it reports is the count of the list right
 * below it, so reading it as a caption of that list — on the same line, at the
 * far end — is what makes it mean something. Next to the `h1` it competed with
 * the title and sat two rows away from the number it described.
 *
 * Reads `statusStore` directly instead of taking props: it is the only
 * consumer of this slice, and threading it through `JobTable` would make the
 * table re-render on every status tick for something it does not otherwise
 * care about.
 */
import type { JSX } from "react";

import { Badge } from "@/components/ui/Badge";
import { t } from "@/i18n";
import { selectKpis, useStatusStore } from "@/store/statusStore";

export function WatcherStatusBadge(): JSX.Element {
  const status = useStatusStore((s) => s.status);
  const paused = status?.watcher_paused ?? false;
  const watchedCount = selectKpis(status).detected;

  return (
    <div aria-live="polite">
      <Badge tone={paused ? "neutral" : "success"}>
        {paused
          ? t("pages.dashboard.header.watcherPaused")
          : t("pages.dashboard.header.watcherActive", { count: watchedCount })}
      </Badge>
    </div>
  );
}
