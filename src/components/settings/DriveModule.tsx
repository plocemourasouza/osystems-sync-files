/**
 * DriveModule — Google Drive destination card (PRD.md RF-010/011/014/015,
 * RF-080; DESIGN.md §8 "Card / Module"; T-4.8).
 *
 * Credential UX mirrors `S3Module`'s pattern but swaps the AWS key-pair
 * inputs for a single Service Account JSON file, picked via
 * `credentialsStore.pickServiceAccount()` (`pick_service_account_file`
 * command) — the renderer never receives (and therefore never renders) the
 * raw JSON/`private_key` content (RNF-003): only the metadata in
 * `status.gdrive` (`email`/`project_id`, survives a refresh) and, for the
 * duration of this session, `lastServiceAccount` (`file_name`/`size` from
 * the most recent successful pick).
 *
 * Folder ID and "subpastas por data" (RF-015, now enabled) read/write
 * `configStore.config.gdrive` directly; per-field validation comes from
 * `issueFor(issues, "gdrive.<field>")`. The checksum toggle is always on in
 * the MVP (PRD.md RF-080) — rendered checked and permanently disabled,
 * never wired to a config field. Every other editable field disables when
 * `gdrive.enabled` is off, mirroring `S3Module`.
 *
 * The "Abrir no Google Drive" external-link button stays disabled — there
 * is no generic open-url command yet (only `open_remote`, which targets a
 * specific job's file, not a folder) — with `title` explaining it's a
 * future phase.
 */
import { useState, type JSX } from "react";
import { Cloud, Copy, ExternalLink } from "lucide-react";
import { isAppError } from "@/api/ipc";
import { Badge, Button, Checkbox, ErrorText, Field, Module, Toggle } from "@/components/ui";
import { t, tError } from "@/i18n";
import { formatBytes } from "@/lib/format";
import { issueFor, useConfigStore } from "@/store/configStore";
import { useCredentialsStore } from "@/store/credentialsStore";
import { ConfirmDialog } from "./ConfirmDialog";

export function DriveModule(): JSX.Element {
  const gdrive = useConfigStore((state) => state.config.gdrive);
  const issues = useConfigStore((state) => state.issues);
  const setGDrive = useConfigStore((state) => state.setGDrive);

  const gdriveStatus = useCredentialsStore((state) => state.status?.gdrive ?? null);
  const lastServiceAccount = useCredentialsStore((state) => state.lastServiceAccount);
  const testingGDrive = useCredentialsStore((state) => state.testing.gdrive);
  const lastTestGDrive = useCredentialsStore((state) => state.lastTest.gdrive);
  const pickServiceAccount = useCredentialsStore((state) => state.pickServiceAccount);
  const clearServiceAccount = useCredentialsStore((state) => state.clearServiceAccount);
  const test = useCredentialsStore((state) => state.test);

  const [picking, setPicking] = useState(false);
  const [pickError, setPickError] = useState<string | null>(null);
  const [removeOpen, setRemoveOpen] = useState(false);

  const fieldsDisabled = !gdrive.enabled;
  const gdrivePresent = gdriveStatus?.present ?? false;

  const testFailed = lastTestGDrive !== null && "error" in lastTestGDrive;
  const testOk = lastTestGDrive !== null && "ok" in lastTestGDrive && lastTestGDrive.ok;

  let badgeTone: "neutral" | "info" | "success" | "error" = "neutral";
  let badgeLabel = t("pages.settings.gdrive.statusUnconfigured");
  if (testFailed) {
    badgeTone = "error";
    badgeLabel = t("pages.settings.gdrive.statusAttention");
  } else if (testOk) {
    badgeTone = "success";
    badgeLabel = t("pages.settings.gdrive.statusConnected");
  } else if (gdrivePresent) {
    badgeTone = "info";
    badgeLabel = t("pages.settings.gdrive.statusSaved");
  }

  async function handlePick(): Promise<void> {
    setPicking(true);
    setPickError(null);
    try {
      await pickServiceAccount();
    } catch (e) {
      setPickError(tError(isAppError(e) ? e.code : "unknown"));
    } finally {
      setPicking(false);
    }
  }

  async function handleConfirmRemove(): Promise<void> {
    setRemoveOpen(false);
    await clearServiceAccount();
  }

  async function handleTest(): Promise<void> {
    await test("gdrive");
  }

  const folderIdEmpty = gdrive.folder_id.trim().length === 0;
  const testDisabled = fieldsDisabled || !gdrivePresent || folderIdEmpty || testingGDrive;

  let footerHint: string | null = null;
  if (!gdrivePresent) {
    footerHint = t("pages.settings.gdrive.testHintNoCredentials");
  } else if (folderIdEmpty) {
    footerHint = t("pages.settings.gdrive.testHintNoFolder");
  } else if (!lastTestGDrive) {
    footerHint = t("pages.settings.gdrive.testHintReady");
  }

  return (
    <Module
      className="min-w-0"
      icon={<Cloud aria-hidden="true" size={18} />}
      title={t("pages.settings.gdrive.title")}
      subtitle={t("pages.settings.gdrive.subtitle")}
      status={
        <div className="flex items-center gap-sm">
          <Toggle
            checked={gdrive.enabled}
            onChange={(checked) => setGDrive({ enabled: checked })}
            label={t("pages.settings.gdrive.enabledLabel")}
          />
          <Badge tone={badgeTone}>{badgeLabel}</Badge>
        </div>
      }
      footer={
        <div className="flex flex-col gap-xs">
          {lastTestGDrive && testFailed && (
            <ErrorText>{t("pages.settings.gdrive.testResultError", { message: tError(lastTestGDrive.error) })}</ErrorText>
          )}
          {lastTestGDrive && !testFailed && "ok" in lastTestGDrive && (
            <p role="status" className="text-label-sm text-tertiary">
              {t("pages.settings.gdrive.testResultSuccess", { ms: lastTestGDrive.latency_ms, message: lastTestGDrive.message })}
            </p>
          )}
          <div className="flex items-center justify-between gap-sm">
            {footerHint && <p className="text-label-sm text-text-tertiary">{footerHint}</p>}
            <Button
              variant="secondary"
              size="sm"
              loading={testingGDrive}
              disabled={testDisabled}
              onClick={handleTest}
              className="ml-auto"
            >
              {t("pages.settings.gdrive.testButton")}
            </Button>
          </div>
        </div>
      }
    >
      <div className="flex flex-col gap-xs px-md py-sm">
        <span className="text-body-sm text-text-secondary">{t("pages.settings.gdrive.credentialLabel")}</span>

        {gdrivePresent ? (
          <div className="flex flex-col gap-xs rounded border border-border-hairline bg-surface-0 px-sm py-xs">
            {gdriveStatus?.email && (
              <div className="flex items-center justify-between gap-sm">
                <span className="text-label-sm text-text-tertiary">{t("pages.settings.gdrive.clientEmailLabel")}</span>
                <span className="truncate font-mono text-mono-data text-text-primary">{gdriveStatus.email}</span>
              </div>
            )}
            {gdriveStatus?.project_id && (
              <div className="flex items-center justify-between gap-sm">
                <span className="text-label-sm text-text-tertiary">{t("pages.settings.gdrive.projectIdLabel")}</span>
                <span className="truncate font-mono text-mono-data text-text-primary">{gdriveStatus.project_id}</span>
              </div>
            )}
            {lastServiceAccount && (
              <span className="text-label-sm text-text-quaternary">
                {t("pages.settings.gdrive.fileInfo", {
                  fileName: lastServiceAccount.file_name,
                  size: formatBytes(lastServiceAccount.size),
                })}
              </span>
            )}
            <div className="flex items-center justify-end gap-sm">
              <Button variant="ghost" size="sm" loading={picking} disabled={fieldsDisabled} onClick={() => void handlePick()}>
                {t("pages.settings.gdrive.replaceButton")}
              </Button>
              <Button variant="destructive" size="sm" disabled={fieldsDisabled} onClick={() => setRemoveOpen(true)}>
                {t("pages.settings.gdrive.removeButton")}
              </Button>
            </div>
          </div>
        ) : (
          <div className="flex items-center justify-between gap-sm rounded border border-dashed border-border-hairline bg-surface-0 px-sm py-xs">
            <span className="font-mono text-mono-data text-text-quaternary">—</span>
            <Button variant="secondary" size="sm" loading={picking} disabled={fieldsDisabled} onClick={() => void handlePick()}>
              {t("pages.settings.gdrive.selectJsonButton")}
            </Button>
          </div>
        )}

        {pickError && <ErrorText>{pickError}</ErrorText>}
      </div>

      <div className="px-md py-sm">
        <Field
          label={t("pages.settings.gdrive.folderIdLabel")}
          mono
          value={gdrive.folder_id}
          onChange={(event) => setGDrive({ folder_id: event.target.value })}
          error={issueFor(issues, "gdrive.folder_id")}
          disabled={fieldsDisabled}
          trailing={
            <div className="flex items-center gap-2xs">
              <button
                type="button"
                aria-label={t("pages.settings.gdrive.copyAria")}
                onClick={() => {
                  void navigator.clipboard?.writeText(gdrive.folder_id);
                }}
                className="flex h-4 w-4 items-center justify-center text-text-secondary transition-colors duration-fast hover:text-text-primary focus-visible:outline-none focus-visible:shadow-focus"
              >
                <Copy aria-hidden="true" size={14} />
              </button>
              <button
                type="button"
                aria-label={t("pages.settings.gdrive.openAria")}
                title={t("pages.settings.gdrive.openTitleDisabled")}
                disabled
                className="flex h-4 w-4 items-center justify-center text-text-quaternary disabled:cursor-not-allowed"
              >
                <ExternalLink aria-hidden="true" size={14} />
              </button>
            </div>
          }
        />
      </div>

      <div className="px-md py-sm">
        <Checkbox
          label={t("pages.settings.gdrive.dateSubfoldersLabel")}
          description={`${t("pages.settings.gdrive.dateSubfoldersDescription")} · ${t("pages.settings.gdrive.dateSubfoldersNote")}`}
          checked={gdrive.date_subfolders}
          onChange={(checked) => setGDrive({ date_subfolders: checked })}
          disabled={fieldsDisabled}
        />
      </div>

      <div className="px-md py-sm">
        <Checkbox
          label={t("pages.settings.gdrive.checksumLabel")}
          description={t("pages.settings.gdrive.checksumDescription")}
          checked
          onChange={() => undefined}
          disabled
        />
      </div>

      <ConfirmDialog
        open={removeOpen}
        title={t("pages.settings.gdrive.removeDialog.title")}
        confirmLabel={t("pages.settings.gdrive.removeDialog.confirm")}
        cancelLabel={t("pages.settings.gdrive.removeDialog.cancel")}
        onConfirm={() => void handleConfirmRemove()}
        onCancel={() => setRemoveOpen(false)}
      >
        {t("pages.settings.gdrive.removeDialog.body")}
      </ConfirmDialog>
    </Module>
  );
}
