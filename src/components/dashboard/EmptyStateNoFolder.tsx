/**
 * EmptyStateNoFolder — displayed in Dashboard when `watch.path` is null.
 * Contract: centered section with dashed card, FolderSearch icon (40 px),
 * title + body text, primary CTA "Escolher pasta" (loading while picking),
 * secondary hint "Você também pode alterar pela sidebar."
 *
 * Fires `useStatusStore(s => s.actions.pickFolder)` on CTA click;
 * on rejection (user cancels), does nothing; on error, shows inline alert.
 */
import { useState } from "react";
import { FolderOpen, FolderSearch } from "lucide-react";
import { t, tError } from "@/i18n";
import { useStatusStore } from "@/store/statusStore";
import { Button } from "@/components/ui/Button";
import { cn } from "@/components/ui/cn";
import { ErrorText } from "@/components/ui";

export function EmptyStateNoFolder() {
  const pickFolder = useStatusStore((s) => s.actions.pickFolder);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handlePickFolder = async () => {
    setIsLoading(true);
    setError(null);
    try {
      const result = await pickFolder();
      // result is null if user cancelled; nothing to do
      if (result === null) {
        setIsLoading(false);
      }
      // if result is a string (path), the store already updated configStore,
      // so the parent Dashboard will re-render and hide this component
    } catch (err) {
      const errorMsg = err instanceof Error ? err.message : String(err);
      setError(tError(errorMsg));
      setIsLoading(false);
    }
  };

  return (
    <section
      aria-labelledby="empty-state-title"
      className={cn(
        "flex flex-col items-center justify-center gap-lg py-xl px-md",
        "rounded-lg border border-dashed border-border-strong bg-surface-1"
      )}
    >
      <FolderSearch
        size={40}
        className="text-text-quaternary"
        aria-hidden="true"
      />

      <div className="flex flex-col items-center gap-xs max-w-96">
        <h2 id="empty-state-title" className="text-headline-md">
          {t("pages.dashboard.empty.noFolder.title")}
        </h2>
        <p className="text-body-md text-text-secondary text-center">
          {t("pages.dashboard.empty.noFolder.body")}
        </p>
      </div>

      <Button
        variant="primary"
        size="md"
        icon={<FolderOpen size={16} aria-hidden="true" />}
        loading={isLoading}
        onClick={handlePickFolder}
      >
        {t("pages.dashboard.empty.noFolder.cta")}
      </Button>

      <p className="text-body-sm text-text-secondary">
        {t("pages.dashboard.empty.noFolder.hint")}
      </p>

      {error && (
        <ErrorText>
          {error}
        </ErrorText>
      )}
    </section>
  );
}
