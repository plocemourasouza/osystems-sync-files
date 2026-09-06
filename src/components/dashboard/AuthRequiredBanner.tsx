/**
 * AuthRequiredBanner — displays per-destination authentication alerts when
 * credentials become invalid (RF-070 auth-required state, PLAN.md T-4.10).
 *
 * Shows one banner per destination with:
 * - role="alert" for accessibility
 * - AlertTriangle icon + error tone (bg-status-error-bg, text-error, border-error)
 * - Destination-specific title
 * - Hint from event (e-mail, policy, or fallback)
 * - Actions: "Ir para Configurações" (navigate to /settings), "Dispensar" (hide)
 *
 * Derives visible destinations from `statusStore.status.destinations[x].auth_required`
 * and keeps latest hints in local state (keyed by destination) from the `auth-required`
 * event. Dismissal persists until the next event fires for that destination.
 */
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { AlertTriangle, X } from "lucide-react";

import { useTauriEvent } from "@/api/events";
import { Button } from "@/components/ui/Button";
import { useStatusStore } from "@/store/statusStore";
import type { AuthRequired, Destination } from "@/types/generated";
import { t } from "@/i18n";

type DismissedSet = Set<Destination>;
type HintMap = Record<Destination, string>;

export function AuthRequiredBanner() {
  const navigate = useNavigate();

  // Locally track hints from events and dismissed banners per destination.
  const [hints, setHints] = useState<HintMap>({ gdrive: "", s3: "" });
  const [dismissed, setDismissed] = useState<DismissedSet>(new Set());

  // Subscribe to `auth-required` events: add the hint and un-dismiss if needed.
  useTauriEvent("auth-required", (event: AuthRequired) => {
    setHints((prev) => ({ ...prev, [event.destination]: event.hint }));
    // Event fired = user must re-authenticate, so clear dismissal for this destination.
    setDismissed((prev) => {
      const next = new Set(prev);
      next.delete(event.destination);
      return next;
    });
  });

  // Derive which destinations have auth_required = true in the store.
  const status = useStatusStore((s) => s.status);
  const authRequiredDestinations: Destination[] = [];
  if (status) {
    if (status.destinations.gdrive.auth_required && !dismissed.has("gdrive")) {
      authRequiredDestinations.push("gdrive");
    }
    if (status.destinations.s3.auth_required && !dismissed.has("s3")) {
      authRequiredDestinations.push("s3");
    }
  }

  const handleDismiss = (destination: Destination) => {
    setDismissed((prev) => new Set(prev).add(destination));
  };

  const handleNavigateToSettings = () => {
    navigate("/settings");
  };

  if (authRequiredDestinations.length === 0) {
    return <></>;
  }

  return (
    <div className="flex flex-col gap-sm">
      {authRequiredDestinations.map((destination) => {
        const displayName = destination === "gdrive" ? "Google Drive" : "AWS S3";
        const hint = hints[destination];

        return (
          <div
            key={destination}
            role="alert"
            className="flex gap-sm rounded border border-error bg-status-error-bg px-md py-sm text-error"
          >
            <AlertTriangle aria-hidden="true" size={20} className="mt-xs shrink-0" />

            <div className="flex min-w-0 flex-1 flex-col gap-xs">
              <h2 className="text-body-md font-semibold">
                {t("pages.dashboard.authBanner.title", { destination: displayName })}
              </h2>
              <p className="text-body-sm text-text-primary">
                {hint ? (
                  <code className="whitespace-pre-wrap break-words font-mono text-label-sm">
                    {hint}
                  </code>
                ) : (
                  t("pages.dashboard.authBanner.defaultHint")
                )}
              </p>
            </div>

            <div className="flex shrink-0 flex-col gap-xs pt-xs">
              <Button
                variant="primary"
                size="sm"
                onClick={handleNavigateToSettings}
                aria-label={t("pages.dashboard.authBanner.settingsAriaLabel", { destination: displayName })}
              >
                {t("pages.dashboard.authBanner.settings")}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                icon={<X size={16} aria-hidden="true" />}
                onClick={() => handleDismiss(destination)}
                aria-label={t("pages.dashboard.authBanner.dismissAriaLabel", {
                  destination: displayName,
                })}
              >
                {t("pages.dashboard.authBanner.dismiss")}
              </Button>
            </div>
          </div>
        );
      })}
    </div>
  );
}
