/**
 * SettingsFooter — sticky footer for `/settings` (PRD.md RF-083, RF-084;
 * DESIGN.md §9 region map: "Rodapé fixo: timestamp · Cancelar/Restaurar
 * Padrões · Salvar Preferências (Ctrl+S)").
 *
 * Left: last-saved timestamp, an "unsaved changes" badge while `dirty`, a
 * validation issues count while `issues.length > 0`, and the store's
 * generic `error` (a distinct failure path from `config.invalid` — see
 * `configStore.save()`) as an `alert`. Right: Cancelar (discard, RF-084) /
 * Restaurar padrões (behind a confirmation dialog, RF-084) / Salvar
 * preferências (RF-083), the last one also armed on `Ctrl+S` via
 * `useSaveShortcut`.
 */
import { History } from "lucide-react";
import { useState, type JSX } from "react";
import { Badge, Button, ErrorText } from "@/components/ui";
import { useSaveShortcut } from "@/hooks/useSaveShortcut";
import { t } from "@/i18n";
import { useConfigStore } from "@/store/configStore";
import { ConfirmDialog } from "./ConfirmDialog";

function formatSavedAt(iso: string): string {
  return new Date(iso).toLocaleTimeString("pt-BR", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}

export function SettingsFooter(): JSX.Element {
  const status = useConfigStore((state) => state.status);
  const dirty = useConfigStore((state) => state.dirty);
  const issues = useConfigStore((state) => state.issues);
  const error = useConfigStore((state) => state.error);
  const lastSavedAt = useConfigStore((state) => state.lastSavedAt);
  const save = useConfigStore((state) => state.save);
  const discard = useConfigStore((state) => state.discard);
  const restoreDefaults = useConfigStore((state) => state.restoreDefaults);

  const [restoreOpen, setRestoreOpen] = useState(false);

  const saving = status === "saving";
  // RF-083: o botão precisa responder ao clique mesmo sem alterações pendentes
  // (`save()` é idempotente). Só bloqueia enquanto salva ou com validação aberta.
  const saveDisabled = saving || issues.length > 0;

  useSaveShortcut(save, { disabled: saveDisabled });

  function handleRestoreConfirm(): void {
    restoreDefaults();
    setRestoreOpen(false);
  }

  return (
    <footer className="sticky -bottom-xl z-10 -mx-xl -mb-xl mt-auto flex flex-wrap items-center justify-between gap-md border-t border-border-hairline bg-surface-1 px-xl py-sm">
      <div className="flex flex-col gap-2xs">
        <div className="flex flex-wrap items-center gap-sm">
          <History aria-hidden="true" size={14} className="shrink-0 text-text-quaternary" />
          <span className="font-mono text-mono-data text-text-secondary">
            {lastSavedAt
              ? t("pages.settings.footer.lastSaved", { time: formatSavedAt(lastSavedAt) })
              : t("pages.settings.footer.neverSaved")}
          </span>
          {dirty && <Badge tone="warning">{t("pages.settings.footer.unsavedBadge")}</Badge>}
        </div>
        {issues.length > 0 && (
          <span className="text-label-sm text-error">
            {t("pages.settings.footer.issuesCount", { count: issues.length })}
          </span>
        )}
        {status === "error" && error && <ErrorText className="text-label-sm">{error}</ErrorText>}
      </div>

      <div className="flex items-center gap-sm">
        <Button variant="secondary" size="md" disabled={saving} onClick={discard}>
          {t("pages.settings.footer.cancelButton")}
        </Button>
        <Button variant="secondary" size="md" onClick={() => setRestoreOpen(true)}>
          {t("pages.settings.footer.restoreButton")}
        </Button>
        <Button variant="primary" size="md" loading={saving} disabled={saveDisabled} onClick={() => void save()}>
          {t("pages.settings.footer.saveButton")}
          <kbd className="ml-xs rounded border border-[color-mix(in_srgb,var(--color-on-primary)_35%,transparent)] px-2xs font-mono text-label-sm">
            {t("pages.settings.footer.saveShortcutHint")}
          </kbd>
        </Button>
      </div>

      <ConfirmDialog
        open={restoreOpen}
        title={t("pages.settings.footer.restoreDialog.title")}
        confirmLabel={t("pages.settings.footer.restoreDialog.confirm")}
        cancelLabel={t("pages.settings.footer.restoreDialog.cancel")}
        onConfirm={handleRestoreConfirm}
        onCancel={() => setRestoreOpen(false)}
      >
        {t("pages.settings.footer.restoreDialog.body")}
      </ConfirmDialog>
    </footer>
  );
}
