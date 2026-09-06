/**
 * GeneralModule — Settings § "Geral" (PRD.md RF-086; design/DESIGN.md §8
 * Module/Field/Toggle). Sits below the QoS module in the `/settings` layout
 * (RF-086: "abaixo do QoS") — QoS itself doesn't exist yet in this phase, see
 * the commented slot in `Settings.tsx`.
 *
 * Three groups inside a single `Module`, each bound to `configStore`'s
 * granular setters:
 *  - Sistema: autostart (RF-092) and keep-awake (RF-093 — `ES_SYSTEM_REQUIRED`,
 *    never forces the display on, only sleep).
 *  - Desempenho: workers per destination and retry policy.
 *  - Filtros do watcher: recursive scan, allowed extensions, max file size and
 *    stabilization window (RF-003).
 *
 * `error` on every `NumberField` comes from `issueFor(issues, "<rust path>")`
 * so a `config.invalid` response from `save_config` highlights the exact
 * control that failed validation.
 *
 * The watched folder itself is read-only here: picking/changing it is the
 * sidebar's job (T-2.6), this module only displays the current value.
 */
import { Settings2 } from "lucide-react";
import { useEffect, useState, type FocusEvent, type JSX } from "react";
import { Checkbox, Field, Module, NumberField, Toggle } from "@/components/ui";
import { t } from "@/i18n";
import { issueFor, useConfigStore } from "@/store/configStore";

/**
 * Hard ceiling for both size filters, in MiB (5 TiB) — mirrors
 * `config::MAX_FILE_SIZE_MB` in the core, which is the smaller of the two
 * destinations' own object limits (S3 5 TiB, Drive 5 TB). `0` in either
 * field means the filter is off.
 */
const MAX_FILE_SIZE_MB = 5 * 1024 * 1024;

/** "PDF, .csv  txt" → ["pdf", "csv", "txt"] — comma/space separated, lowercase, no leading dots. */
function parseExtensions(raw: string): string[] {
  return raw
    .split(/[\s,]+/)
    .map((token) => token.trim().replace(/^\.+/, "").toLowerCase())
    .filter((token) => token.length > 0);
}

function GroupHeading({ children }: { children: string }): JSX.Element {
  return (
    <span className="font-mono text-label-sm uppercase tracking-wide text-text-label">{children}</span>
  );
}

export function GeneralModule(): JSX.Element {
  const config = useConfigStore((state) => state.config);
  const issues = useConfigStore((state) => state.issues);
  const setWatch = useConfigStore((state) => state.setWatch);
  const setRetry = useConfigStore((state) => state.setRetry);
  const setGeneral = useConfigStore((state) => state.setGeneral);

  const [extensionsDraft, setExtensionsDraft] = useState(config.watch.extensions.join(", "));

  useEffect(() => {
    setExtensionsDraft(config.watch.extensions.join(", "));
  }, [config.watch.extensions]);

  function handleExtensionsBlur(event: FocusEvent<HTMLInputElement>): void {
    const parsed = parseExtensions(event.target.value);
    setExtensionsDraft(parsed.join(", "));
    setWatch({ extensions: parsed });
  }

  return (
    <Module className="min-w-0" icon={<Settings2 aria-hidden="true" size={16} />} title={t("pages.settings.general.title")}>
      <div className="flex flex-col gap-sm px-md py-sm">
        <GroupHeading>{t("pages.settings.general.system.heading")}</GroupHeading>
        <Toggle
          label={t("pages.settings.general.system.autostart.label")}
          checked={config.autostart}
          onChange={(checked) => setGeneral({ autostart: checked })}
        />
        <Toggle
          label={t("pages.settings.general.system.keepAwake.label")}
          description={t("pages.settings.general.system.keepAwake.description")}
          checked={config.keep_awake}
          onChange={(checked) => setGeneral({ keep_awake: checked })}
        />
      </div>

      <div className="flex flex-col gap-sm px-md py-sm">
        <GroupHeading>{t("pages.settings.general.performance.heading")}</GroupHeading>
        <div className="grid grid-cols-1 gap-sm sm:grid-cols-3">
          <NumberField
            label={t("pages.settings.general.performance.workers.label")}
            value={config.workers_per_destination}
            min={1}
            max={4}
            onChange={(value) => setGeneral({ workers_per_destination: value })}
            error={issueFor(issues, "workers_per_destination")}
          />
          <NumberField
            label={t("pages.settings.general.performance.maxAttempts.label")}
            value={config.retry.max_attempts}
            min={1}
            max={10}
            onChange={(value) => setRetry({ max_attempts: value })}
            error={issueFor(issues, "retry.max_attempts")}
          />
          <NumberField
            label={t("pages.settings.general.performance.baseDelay.label")}
            value={config.retry.base_delay_seconds}
            min={1}
            max={60}
            onChange={(value) => setRetry({ base_delay_seconds: value })}
            error={issueFor(issues, "retry.base_delay_seconds")}
          />
        </div>
      </div>

      <div className="flex flex-col gap-sm px-md py-sm">
        <GroupHeading>{t("pages.settings.general.watcher.heading")}</GroupHeading>

        {/* Where the confusion started: filters gate intake, so tightening one
            leaves the existing queue alone until a rescan reconciles it. */}
        <p className="text-label-sm text-text-quaternary">
          {t("pages.settings.general.watcher.scopeNote")}
        </p>

        <div className="flex flex-col gap-xs">
          <span className="text-label-sm text-text-tertiary">
            {t("pages.settings.general.watcher.pathLabel")}
          </span>
          <div className="flex items-center rounded-md border border-border-hairline bg-surface-1 px-sm py-xs">
            <span
              className="truncate font-mono text-body-sm text-text-emphasis"
              title={config.watch.path ?? undefined}
            >
              {config.watch.path ?? t("pages.settings.general.watcher.noFolder")}
            </span>
          </div>
          <span className="text-label-sm text-text-quaternary">
            {t("pages.settings.general.watcher.pathHint")}
          </span>
        </div>

        <Checkbox
          label={t("pages.settings.general.watcher.recursive.label")}
          checked={config.watch.recursive}
          onChange={(checked) => setWatch({ recursive: checked })}
        />

        <Field
          label={t("pages.settings.general.watcher.extensions.label")}
          hint={t("pages.settings.general.watcher.extensions.hint")}
          value={extensionsDraft}
          onChange={(event) => setExtensionsDraft(event.target.value)}
          onBlur={handleExtensionsBlur}
          mono
        />

        <div className="grid grid-cols-1 gap-sm sm:grid-cols-2">
          <NumberField
            label={t("pages.settings.general.watcher.minSize.label")}
            hint={t("pages.settings.general.watcher.minSize.hint")}
            value={config.watch.min_size_mb}
            min={0}
            max={MAX_FILE_SIZE_MB}
            onChange={(value) => setWatch({ min_size_mb: value })}
            error={issueFor(issues, "watch.min_size_mb")}
          />
          <NumberField
            label={t("pages.settings.general.watcher.maxSize.label")}
            hint={t("pages.settings.general.watcher.maxSize.hint")}
            value={config.watch.max_size_mb}
            min={0}
            max={MAX_FILE_SIZE_MB}
            onChange={(value) => setWatch({ max_size_mb: value })}
            error={issueFor(issues, "watch.max_size_mb")}
          />
          <NumberField
            label={t("pages.settings.general.watcher.stabilize.label")}
            value={config.watch.stabilize_seconds}
            min={1}
            max={60}
            onChange={(value) => setWatch({ stabilize_seconds: value })}
            error={issueFor(issues, "watch.stabilize_seconds")}
          />
        </div>
      </div>
    </Module>
  );
}
