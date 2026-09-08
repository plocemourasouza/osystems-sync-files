/**
 * RescanFailedBanner — surfaces a background rescan failure that has no request
 * to reject into (`rescan-failed` event, PLAN.md T-1.3). A background rescan
 * (boot, resume from OS sleep, tray, `resume_watcher`) runs without a caller
 * to return an `AppError` to, so the Rust side emits this event instead
 * (`src-tauri/src/events.rs::notify_rescan_failure`) — otherwise a permission
 * error mid-scan is invisible: the watcher just silently stops reconciling.
 *
 * Mirrors `AuthRequiredBanner`'s shape (role="alert", error tone, dismissable),
 * but unlike auth-required there's no persistent `statusStore` flag to derive
 * visibility from: this is purely event-driven local state — the latest
 * message, cleared on dismiss, replaced whenever another `rescan-failed`
 * event fires (even for an already-dismissed banner, same as auth-required).
 */
import { useState } from "react";
import { AlertTriangle, X } from "lucide-react";

import { useTauriEvent } from "@/api/events";
import { Button } from "@/components/ui/Button";
import type { RescanFailed } from "@/types/generated";
import { t } from "@/i18n";

export function RescanFailedBanner() {
  const [message, setMessage] = useState<string | null>(null);

  useTauriEvent("rescan-failed", (event: RescanFailed) => {
    setMessage(event.message);
  });

  if (message === null) {
    return <></>;
  }

  return (
    <div role="alert" className="flex gap-sm rounded border border-error bg-status-error-bg px-md py-sm text-error">
      <AlertTriangle aria-hidden="true" size={20} className="mt-xs shrink-0" />

      <div className="flex min-w-0 flex-1 flex-col gap-xs">
        <h2 className="text-body-md font-semibold">{t("pages.dashboard.rescanFailedBanner.title")}</h2>
        <p className="text-body-sm text-text-primary">
          <code className="whitespace-pre-wrap break-words font-mono text-label-sm">{message}</code>
        </p>
      </div>

      <div className="flex shrink-0 items-start pt-xs">
        <Button
          variant="ghost"
          size="sm"
          icon={<X size={16} aria-hidden="true" />}
          onClick={() => setMessage(null)}
          aria-label={t("pages.dashboard.rescanFailedBanner.dismissAriaLabel")}
        >
          {t("pages.dashboard.rescanFailedBanner.dismiss")}
        </Button>
      </div>
    </div>
  );
}
