/**
 * EmptyStateNoCredentials — displayed in Dashboard when a folder IS
 * configured (`watch.path !== null`) but neither destination has a saved
 * credential (`credentialsStore.status.{aws,gdrive}.present` both false).
 * PLAN.md T-5.8; PRD.md RF-037.
 *
 * Rendered above the KPI row/table (which stay mounted below so already
 * detected files remain visible) — this is a nudge, not a blocking gate.
 * CTA navigates to `/settings`, where DriveModule/S3Module handle the
 * actual credential setup.
 */
import { KeyRound } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { Button } from "@/components/ui/Button";
import { cn } from "@/components/ui/cn";
import { t } from "@/i18n";

export function EmptyStateNoCredentials() {
  const navigate = useNavigate();

  return (
    <section
      aria-labelledby="empty-state-no-credentials-title"
      className={cn(
        "flex flex-col items-center justify-center gap-lg py-xl px-md",
        "rounded-lg border border-dashed border-border-strong bg-surface-1"
      )}
    >
      <KeyRound size={40} className="text-text-quaternary" aria-hidden="true" />

      <div className="flex flex-col items-center gap-xs max-w-96">
        <h2 id="empty-state-no-credentials-title" className="text-headline-md">
          {t("pages.dashboard.empty.noCredentials.title")}
        </h2>
        <p className="text-body-md text-text-secondary text-center">
          {t("pages.dashboard.empty.noCredentials.body")}
        </p>
      </div>

      <Button variant="primary" size="md" onClick={() => navigate("/settings")}>
        {t("pages.dashboard.empty.noCredentials.cta")}
      </Button>
    </section>
  );
}
