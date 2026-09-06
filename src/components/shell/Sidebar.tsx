/**
 * Sidebar — persistent navigation shell component (RF-067, DESIGN.md §8).
 *
 * Anatomy (top to bottom): brand block, primary navigation (`/dashboard`,
 * `/settings`, each with a count badge), the watched-folder card
 * (`FolderCard`), and the bandwidth throughput widget pinned to the bottom
 * (`ThroughputWidget`). All three inner pieces are private to this file —
 * only `Sidebar` and the `formatRate` helper are part of the public contract
 * (T-0.4).
 *
 * Strings sourced from `src/i18n/pt-BR.json` via `t()` (T-1.6).
 */
import type { JSX, ReactNode } from "react";
import { ArrowLeftRight, FolderOpen, SlidersHorizontal } from "lucide-react";
import { NavLink } from "react-router-dom";

import { AuthorCard } from "./AuthorCard";
import { t } from "../../i18n";

export type SidebarProps = {
  /** Number of jobs currently in queue, shown as the badge of the "Arquivos & Fila" nav item. */
  queueCount: number;
  /** Absolute path of the folder being watched, or `null` when none is selected yet. */
  watchPath: string | null;
  /** Whether the background watcher is currently emitting new jobs. */
  watcherActive: boolean;
  /** Aggregate + per-destination upload rates, in bytes/sec, for the throughput widget. */
  throughput: {
    totalBps: number;
    gdriveBps: number;
    s3Bps: number;
    /** QoS cap, in bytes/sec, or `null` when unlimited. */
    capBps: number | null;
  };
  /** Called when the user clicks the folder card's "Alterar pasta" / "Escolher pasta" button. */
  onChangeFolder?: () => void;
};

/**
 * Formats a bytes/sec rate for display: below 1 MB/s it is shown in whole
 * KB/s (no decimals — small values don't need the precision), at or above
 * 1 MB/s it is shown in MB/s with one decimal.
 */
export function formatRate(bps: number): string {
  const megabytesPerSecond = bps / 1_000_000;
  if (megabytesPerSecond < 1) {
    return `${Math.round(bps / 1_000)} KB/s`;
  }
  return `${megabytesPerSecond.toFixed(1)} MB/s`;
}

type NavItemProps = {
  to: string;
  label: string;
  icon: ReactNode;
  badge: ReactNode;
};

function NavItem({ to, label, icon, badge }: NavItemProps): JSX.Element {
  return (
    <NavLink
      to={to}
      className={({ isActive }) =>
        [
          "relative flex items-center gap-sm rounded-md px-sm py-xs text-body-sm transition-colors duration-fast",
          "focus-visible:outline-none focus-visible:shadow-focus",
          isActive
            ? "bg-surface-3 text-text-primary before:absolute before:inset-y-0 before:left-0 before:w-[2px] before:rounded-full before:bg-primary"
            : "text-text-secondary hover:bg-surface-2 hover:text-text-primary",
        ].join(" ")
      }
    >
      {icon}
      <span className="flex-1 truncate">{label}</span>
      {badge}
    </NavLink>
  );
}

function NavBadge({ children, live }: { children: ReactNode; live?: boolean }): JSX.Element {
  return (
    <span
      aria-live={live ? "polite" : undefined}
      className="rounded bg-[color-mix(in_srgb,var(--color-primary)_15%,transparent)] px-xs py-[1px] font-mono text-label-sm text-primary"
    >
      {children}
    </span>
  );
}

type FolderCardProps = {
  path: string | null;
  watcherActive: boolean;
  onChangeFolder?: () => void;
};

function FolderCard({ path, watcherActive, onChangeFolder }: FolderCardProps): JSX.Element {
  return (
    <div className="flex flex-col gap-xs rounded-md border border-border-hairline bg-surface-2 p-md">
      <span className="font-mono text-label-sm uppercase tracking-wide text-text-label">
        {t("shell.sidebar.folderCard.title")}
      </span>

      {path !== null ? (
        <span title={path} className="truncate font-mono text-body-sm text-text-emphasis">
          {path}
        </span>
      ) : (
        <span className="text-body-sm text-text-secondary">{t("shell.sidebar.folderCard.emptyPath")}</span>
      )}

      <span
        className={`inline-flex w-fit items-center gap-xs rounded px-xs py-[1px] font-mono text-label-sm uppercase ${
          watcherActive ? "bg-status-success-bg text-tertiary" : "bg-status-warning-bg text-secondary"
        }`}
      >
        <span
          aria-hidden="true"
          className={`h-1.5 w-1.5 shrink-0 rounded-full ${
            watcherActive ? "animate-pulse bg-tertiary" : "bg-text-quaternary"
          }`}
        />
        {watcherActive ? t("shell.sidebar.watcher.activeBadge") : t("shell.sidebar.watcher.pausedBadge")}
      </span>

      <button
        type="button"
        onClick={onChangeFolder}
        className="mt-xs inline-flex items-center justify-center gap-xs rounded-md border border-border-hairline bg-surface-2 px-sm py-xs text-label-sm text-text-secondary transition-colors duration-fast hover:bg-surface-hover hover:text-text-primary focus-visible:outline-none focus-visible:shadow-focus"
      >
        <FolderOpen aria-hidden="true" size={16} />
        {path !== null ? t("shell.sidebar.folderCard.changeCta") : t("shell.sidebar.folderCard.emptyCta")}
      </button>
    </div>
  );
}

function ThroughputWidget({ throughput }: { throughput: SidebarProps["throughput"] }): JSX.Element {
  const { totalBps, gdriveBps, s3Bps, capBps } = throughput;
  const denominator = capBps !== null ? capBps : totalBps > 0 ? totalBps : 1;
  const gdrivePercent = Math.min(100, (gdriveBps / denominator) * 100);
  const s3Percent = Math.min(100, (s3Bps / denominator) * 100);
  const capLabel = capBps !== null ? formatRate(capBps) : t("shell.sidebar.throughput.unlimited");

  return (
    <div className="mt-auto flex flex-col gap-xs border-t border-border-hairline p-md">
      <span className="font-mono text-label-sm uppercase tracking-wide text-text-label">
        {t("shell.sidebar.throughput.title")}
      </span>
      <span className="font-mono text-body-sm text-text-emphasis">
        {formatRate(totalBps)} / {capLabel}
      </span>
      <div
        role="progressbar"
        aria-valuenow={Math.round(totalBps)}
        aria-valuemin={0}
        aria-valuemax={capBps ?? Math.max(totalBps, 1)}
        aria-label={t("shell.sidebar.throughput.title")}
        className="flex h-1 w-full overflow-hidden rounded-full bg-surface-0"
      >
        <span aria-hidden="true" className="h-full bg-primary-strong" style={{ width: `${gdrivePercent}%` }} />
        <span aria-hidden="true" className="h-full bg-secondary-strong" style={{ width: `${s3Percent}%` }} />
      </div>
      <span className="font-mono text-label-sm text-text-secondary">
        {t("shell.sidebar.throughput.gdrive")}: {formatRate(gdriveBps)} · {t("shell.sidebar.throughput.s3")}: {formatRate(s3Bps)}
      </span>
    </div>
  );
}

export function Sidebar({
  queueCount,
  watchPath,
  watcherActive,
  throughput,
  onChangeFolder,
}: SidebarProps): JSX.Element {
  return (
    <aside
      aria-label="Navegação principal"
      className="flex h-full w-sidebar flex-col border-r border-border-hairline bg-surface-1"
    >
      <div className="flex items-center gap-sm border-b border-border-hairline p-md">
        <span
          aria-hidden="true"
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-surface-2 font-mono text-label-md text-primary"
        >
          oS
        </span>
        <div className="flex min-w-0 flex-col">
          <span className="truncate text-title-md text-text-primary">{t("shell.sidebar.brand.name")}</span>
          <span className="truncate text-label-sm text-text-tertiary">{t("shell.sidebar.brand.tagline")}</span>
        </div>
      </div>

      <nav className="flex flex-col gap-xs p-sm">
        <NavItem
          to="/dashboard"
          label={t("shell.sidebar.nav.queue")}
          icon={<ArrowLeftRight aria-hidden="true" size={16} />}
          badge={<NavBadge live>{queueCount}</NavBadge>}
        />
        <NavItem
          to="/settings"
          label={t("shell.sidebar.nav.settings")}
          icon={<SlidersHorizontal aria-hidden="true" size={16} />}
          badge={<NavBadge>{t("shell.sidebar.nav.settingsBadge")}</NavBadge>}
        />
      </nav>

      <div className="flex flex-col gap-sm p-sm">
        <FolderCard path={watchPath} watcherActive={watcherActive} onChangeFolder={onChangeFolder} />
        <AuthorCard />
      </div>

      <ThroughputWidget throughput={throughput} />
    </aside>
  );
}
