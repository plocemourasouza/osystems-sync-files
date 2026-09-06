/**
 * Settings — `/settings` route (RF-080 a RF-086).
 *
 * Loads `configStore` on first mount (guarded by `status === "idle"` so
 * remounts/navigations don't refetch), then composes the destination
 * modules — `DriveModule`/`S3Module` side by side from `xl` (≥1280px,
 * T-1.9's explicit breakpoint) and stacked below that — followed by
 * `QosModule` (T-3.12/T-4.9), `GeneralModule`, and the sticky
 * `SettingsFooter` (T-1.9).
 */
import { useEffect } from "react";
import { DriveModule, GeneralModule, QosModule, S3Module, SettingsFooter } from "../components/settings";
import { t } from "../i18n";
import { useConfigStore } from "../store/configStore";

export function Settings() {
  const status = useConfigStore((state) => state.status);
  const load = useConfigStore((state) => state.load);

  useEffect(() => {
    if (status === "idle") {
      void load();
    }
  }, [status, load]);

  return (
    <section aria-labelledby="settings-title" className="flex min-h-full flex-col gap-lg">
      <header className="flex flex-col gap-xs">
        <h1 id="settings-title" className="text-headline-lg">
          {t("pages.settings.title")}
        </h1>
        <p className="text-body-md text-text-secondary">{t("pages.settings.subtitle")}</p>
      </header>

      <div className="grid grid-cols-1 gap-lg xl:grid-cols-2">
        <DriveModule />
        <S3Module />
      </div>

      <QosModule />

      <GeneralModule />

      <SettingsFooter />
    </section>
  );
}
