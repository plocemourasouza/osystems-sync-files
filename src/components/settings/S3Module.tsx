import { useState, type JSX } from "react";
import { Database } from "lucide-react";
import { Badge, Button, ErrorText, Field, Module, PasswordField, Select, Toggle, type SelectOption } from "@/components/ui";
import { t, tError } from "@/i18n";
import { issueFor, useConfigStore } from "@/store/configStore";
import { useCredentialsStore } from "@/store/credentialsStore";
import type { StorageClass } from "@/types/generated";
import { ConfirmDialog } from "./ConfirmDialog";

const REGION_OPTIONS: SelectOption[] = [
  { value: "us-east-1", label: "us-east-1 (N. Virginia)" },
  { value: "us-east-2", label: "us-east-2 (Ohio)" },
  { value: "us-west-1", label: "us-west-1 (N. California)" },
  { value: "us-west-2", label: "us-west-2 (Oregon)" },
  { value: "eu-west-1", label: "eu-west-1 (Ireland)" },
  { value: "eu-central-1", label: "eu-central-1 (Frankfurt)" },
  { value: "sa-east-1", label: "sa-east-1 (São Paulo)" },
  { value: "ap-southeast-1", label: "ap-southeast-1 (Singapore)" },
  { value: "ap-southeast-2", label: "ap-southeast-2 (Sydney)" },
  { value: "ap-northeast-1", label: "ap-northeast-1 (Tokyo)" },
];

const STORAGE_CLASS_OPTIONS: SelectOption[] = [
  { value: "STANDARD", label: "STANDARD" },
  { value: "INTELLIGENT_TIERING", label: "INTELLIGENT_TIERING" },
  { value: "GLACIER_IR", label: "GLACIER_IR" },
];

/**
 * Settings › Amazon S3 module (PLAN.md T-3.10; PRD.md RF-020/RF-023/RF-085;
 * SPEC.md §7). Region/Bucket/Prefix/Storage Class stay bound to
 * `configStore` (saved via the page's Ctrl+S, like every other Fase-1
 * field). Credentials (Access Key ID / Secret Access Key) are bound to
 * `credentialsStore` instead — each "Salvar"/"Remover" writes to the OS
 * keyring immediately, independent of the page's dirty/save cycle, because
 * RNF-003 requires the secret to never sit in in-memory app config state.
 */
export function S3Module(): JSX.Element {
  const s3 = useConfigStore((state) => state.config.s3);
  const issues = useConfigStore((state) => state.issues);
  const dirty = useConfigStore((state) => state.dirty);
  const setS3 = useConfigStore((state) => state.setS3);

  const awsStatus = useCredentialsStore((state) => state.status?.aws ?? null);
  const testingS3 = useCredentialsStore((state) => state.testing.s3);
  const lastTestS3 = useCredentialsStore((state) => state.lastTest.s3);
  const setAws = useCredentialsStore((state) => state.setAws);
  const clearAws = useCredentialsStore((state) => state.clearAws);
  const test = useCredentialsStore((state) => state.test);

  const [accessKeyDraft, setAccessKeyDraft] = useState("");
  const [secretDraft, setSecretDraft] = useState("");
  const [editingAccessKey, setEditingAccessKey] = useState(false);
  const [removeOpen, setRemoveOpen] = useState(false);
  const [savedNotice, setSavedNotice] = useState(false);
  const [saving, setSaving] = useState(false);

  const fieldsDisabled = !s3.enabled;
  const awsPresent = awsStatus?.present ?? false;
  const editingCredentials = !awsPresent || editingAccessKey;
  const canSave = editingCredentials && accessKeyDraft.trim().length > 0 && secretDraft.trim().length > 0;

  const testFailed = lastTestS3 !== null && "error" in lastTestS3;
  const testOk = lastTestS3 !== null && "ok" in lastTestS3 && lastTestS3.ok;

  let badgeTone: "neutral" | "info" | "success" | "error" = "neutral";
  let badgeLabel = t("pages.settings.s3.statusUnconfigured");
  if (testFailed) {
    badgeTone = "error";
    badgeLabel = t("pages.settings.s3.statusAttention");
  } else if (testOk) {
    badgeTone = "success";
    badgeLabel = t("pages.settings.s3.statusConnected");
  } else if (awsPresent) {
    badgeTone = "info";
    badgeLabel = t("pages.settings.s3.statusSaved");
  }

  async function handleSave(): Promise<void> {
    setSaving(true);
    try {
      await setAws({ accessKeyId: accessKeyDraft.trim(), secret: secretDraft.trim() });
      setAccessKeyDraft("");
      setSecretDraft("");
      setEditingAccessKey(false);
      setSavedNotice(true);
    } finally {
      setSaving(false);
    }
  }

  function handleCancelEdit(): void {
    setAccessKeyDraft("");
    setSecretDraft("");
    setEditingAccessKey(false);
  }

  async function handleConfirmRemove(): Promise<void> {
    setRemoveOpen(false);
    setSavedNotice(false);
    await clearAws();
  }

  async function handleTest(): Promise<void> {
    await test("s3");
  }

  const bucketEmpty = s3.bucket.trim().length === 0;
  const testDisabled = fieldsDisabled || !awsPresent || bucketEmpty || testingS3;

  let footerHint: string | null = null;
  if (!awsPresent) {
    footerHint = t("pages.settings.s3.testHintNoCredentials");
  } else if (bucketEmpty) {
    footerHint = t("pages.settings.s3.testHintNoBucket");
  } else if (dirty) {
    footerHint = t("pages.settings.s3.testHintDirty");
  } else if (!lastTestS3) {
    footerHint = t("pages.settings.s3.testHintReady");
  }

  return (
    <Module
      className="min-w-0"
      icon={<Database aria-hidden="true" size={16} />}
      title={t("pages.settings.s3.title")}
      subtitle={t("pages.settings.s3.subtitle")}
      status={
        <div className="flex items-center gap-sm">
          <Toggle checked={s3.enabled} onChange={(c) => setS3({ enabled: c })} label={t("pages.settings.s3.enabledLabel")} />
          <Badge tone={badgeTone}>{badgeLabel}</Badge>
        </div>
      }
      footer={
        <div className="flex flex-col gap-xs">
          {lastTestS3 && testFailed && (
            <ErrorText>{t("pages.settings.s3.testResultError", { message: tError(lastTestS3.error) })}</ErrorText>
          )}
          {lastTestS3 && !testFailed && "ok" in lastTestS3 && (
            <p role="status" className="text-label-sm text-tertiary">
              {t("pages.settings.s3.testResultSuccess", { ms: lastTestS3.latency_ms, message: lastTestS3.message })}
            </p>
          )}
          <div className="flex items-center justify-between gap-sm">
            {footerHint && <p className="text-label-sm text-text-tertiary">{footerHint}</p>}
            <Button
              variant="secondary"
              size="sm"
              loading={testingS3}
              disabled={testDisabled}
              onClick={handleTest}
              className="ml-auto"
            >
              {t("pages.settings.s3.testButton")}
            </Button>
          </div>
        </div>
      }
    >
      <div className="grid grid-cols-2 gap-md px-md py-sm">
        {editingCredentials ? (
          <Field
            label={t("pages.settings.s3.accessKeyLabel")}
            mono
            placeholder="AKIA…"
            value={accessKeyDraft}
            onChange={(e) => setAccessKeyDraft(e.target.value)}
            disabled={fieldsDisabled || saving}
          />
        ) : (
          <Field
            label={t("pages.settings.s3.accessKeyLabel")}
            mono
            readOnly
            value={awsStatus?.masked ?? ""}
            disabled={fieldsDisabled}
            trailing={
              <Button variant="ghost" size="sm" disabled={fieldsDisabled} onClick={() => setEditingAccessKey(true)}>
                {t("pages.settings.s3.replaceButton")}
              </Button>
            }
          />
        )}

        <Select
          label={t("pages.settings.s3.regionLabel")}
          options={REGION_OPTIONS}
          value={s3.region}
          onChange={(v) => setS3({ region: v })}
          disabled={fieldsDisabled}
        />

        {editingCredentials ? (
          <PasswordField
            label={t("pages.settings.s3.secretKeyLabel")}
            mono
            placeholder="••••••••••••••••"
            value={secretDraft}
            onChange={(e) => setSecretDraft(e.target.value)}
            revealDisabled={secretDraft.length === 0}
            disabled={fieldsDisabled || saving}
          />
        ) : (
          <PasswordField
            label={t("pages.settings.s3.secretKeyLabel")}
            mono
            readOnly
            revealDisabled
            value="••••••••"
            hint={t("pages.settings.s3.secretTag")}
            disabled={fieldsDisabled}
          />
        )}

        <Field
          label={t("pages.settings.s3.bucketLabel")}
          mono
          value={s3.bucket}
          onChange={(e) => setS3({ bucket: e.target.value })}
          error={issueFor(issues, "s3.bucket")}
          disabled={fieldsDisabled}
        />

        <Field
          label={t("pages.settings.s3.prefixLabel")}
          mono
          value={s3.prefix}
          onChange={(e) => setS3({ prefix: e.target.value })}
          hint={t("pages.settings.s3.prefixHint")}
          disabled={fieldsDisabled}
        />

        <Select
          label={t("pages.settings.s3.storageClassLabel")}
          options={STORAGE_CLASS_OPTIONS}
          value={s3.storage_class}
          onChange={(v) => setS3({ storage_class: v as StorageClass })}
          disabled={fieldsDisabled}
        />
      </div>

      <div className="flex items-center justify-between gap-sm px-md py-sm">
        {savedNotice && !editingCredentials ? (
          <p className="text-label-sm text-tertiary">{t("pages.settings.s3.savedConfirmation")}</p>
        ) : (
          <span />
        )}
        <div className="flex items-center gap-sm">
          {editingAccessKey && (
            <Button variant="ghost" size="sm" onClick={handleCancelEdit} disabled={saving}>
              {t("common.buttons.cancel")}
            </Button>
          )}
          {editingCredentials && (
            <Button variant="primary" size="sm" disabled={!canSave || saving} loading={saving} onClick={handleSave}>
              {t("pages.settings.s3.saveCredentialsButton")}
            </Button>
          )}
          {awsPresent && !editingAccessKey && (
            <Button variant="destructive" size="sm" onClick={() => setRemoveOpen(true)}>
              {t("pages.settings.s3.removeButton")}
            </Button>
          )}
        </div>
      </div>

      <ConfirmDialog
        open={removeOpen}
        title={t("pages.settings.s3.removeDialog.title")}
        confirmLabel={t("pages.settings.s3.removeDialog.confirm")}
        cancelLabel={t("pages.settings.s3.removeDialog.cancel")}
        onConfirm={handleConfirmRemove}
        onCancel={() => setRemoveOpen(false)}
      >
        {t("pages.settings.s3.removeDialog.body")}
      </ConfirmDialog>
    </Module>
  );
}
