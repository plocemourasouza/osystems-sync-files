/**
 * TitleBar — custom title bar shell component (RF-097, DESIGN.md §8).
 *
 * Replaces the native Windows chrome (`decorations: false`, T-0.1): logo +
 * app name, a chip for the watched folder, the daemon status badge, and the
 * three native window controls (minimize/maximize/close) wired to
 * `@tauri-apps/api/window`.
 *
 * `data-tauri-drag-region` is required on every non-interactive element so
 * the whole bar (not just the root) lets the user drag the window — Tauri
 * only starts a drag when the element under the cursor carries the
 * attribute. The three buttons intentionally do NOT carry it, or clicking
 * them would be swallowed by the drag gesture instead of firing `onClick`.
 *
 * Strings are pt-BR per `PRD.md` §3 (product-wide language), sourced from
 * `src/i18n/pt-BR.json` via `t()` (T-1.6).
 */
import type { JSX } from "react";
import { Folder, Minus, Square, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { t } from "../../i18n";

/**
 * Thin wrapper around the Tauri window API, isolated in its own object so
 * tests can `vi.mock("@tauri-apps/api/window")` and assert on calls without
 * touching a real window.
 */
const windowControls = {
  minimize: () => getCurrentWindow().minimize(),
  toggleMaximize: () => getCurrentWindow().toggleMaximize(),
  close: () => getCurrentWindow().close(),
};

export type TitleBarProps = {
  /** Absolute path of the folder being watched, or `null` when none is selected yet. */
  watchPath: string | null;
  /** Whether the background sync daemon is currently running. */
  daemonActive: boolean;
};

export function TitleBar({ watchPath, daemonActive }: TitleBarProps): JSX.Element {
  const daemonLabel = daemonActive ? t("shell.titleBar.daemonActive") : t("shell.titleBar.daemonInactive");
  const folderLabel = watchPath ?? t("shell.titleBar.noFolder");

  return (
    <header
      data-tauri-drag-region
      role="banner"
      className="flex h-titlebar select-none items-center justify-between border-b border-border-hairline bg-surface-0 pl-md"
    >
      <div data-tauri-drag-region className="flex shrink-0 items-center gap-sm">
        <svg
          aria-hidden="true"
          viewBox="0 0 20 20"
          className="h-5 w-5 shrink-0 text-text-secondary"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
        >
          <path strokeLinecap="round" strokeLinejoin="round" d="M2 10c2-4 4-4 6 0s4 4 6 0s4-4 4 0" />
        </svg>
        <span data-tauri-drag-region className="text-title-md text-text-primary">
          {t("shell.titleBar.appName")}
        </span>
        <span data-tauri-drag-region className="hidden text-body-sm text-text-secondary xl:inline">
          {t("shell.titleBar.appTagline")}
        </span>
      </div>

      <div data-tauri-drag-region className="mx-md h-4 w-px shrink-0 bg-border-hairline" />

      <div data-tauri-drag-region className="flex min-w-0 flex-1 items-center justify-center gap-xs">
        <Folder aria-hidden="true" size={14} className="shrink-0 text-primary" />
        <span
          data-tauri-drag-region
          title={watchPath ?? undefined}
          className="max-w-[28rem] truncate font-mono text-body-sm text-text-secondary"
        >
          {folderLabel}
        </span>
      </div>

      <div data-tauri-drag-region className="flex items-center gap-md pr-md">
        <span data-tauri-drag-region className="flex items-center gap-xs">
          <span
            aria-hidden="true"
            className={`h-1.5 w-1.5 shrink-0 rounded-full ${daemonActive ? "bg-tertiary" : "bg-text-quaternary"}`}
          />
          <span className="font-mono text-label-md text-text-label">{daemonLabel}</span>
        </span>

        <div className="flex h-titlebar items-stretch">
          <button
            type="button"
            aria-label={t("shell.titleBar.minimize")}
            onClick={() => void windowControls.minimize()}
            className="flex h-titlebar w-10 items-center justify-center text-text-secondary transition-colors duration-fast hover:bg-surface-hover hover:text-text-primary focus-visible:outline-none focus-visible:shadow-focus"
          >
            <Minus aria-hidden="true" size={16} />
          </button>
          <button
            type="button"
            aria-label={t("shell.titleBar.maximize")}
            onClick={() => void windowControls.toggleMaximize()}
            className="flex h-titlebar w-10 items-center justify-center text-text-secondary transition-colors duration-fast hover:bg-surface-hover hover:text-text-primary focus-visible:outline-none focus-visible:shadow-focus"
          >
            <Square aria-hidden="true" size={16} />
          </button>
          <button
            type="button"
            aria-label={t("shell.titleBar.close")}
            onClick={() => void windowControls.close()}
            className="flex h-titlebar w-11 items-center justify-center text-text-secondary transition-colors duration-fast hover:bg-danger-hover hover:text-on-primary focus-visible:outline-none focus-visible:shadow-focus"
          >
            <X aria-hidden="true" size={16} />
          </button>
        </div>
      </div>
    </header>
  );
}
