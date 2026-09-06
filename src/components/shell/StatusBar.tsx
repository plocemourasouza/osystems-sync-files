/**
 * StatusBar — bottom shell bar (RF-068, RF-069).
 *
 * Anatomy per design/DESIGN.md §8 "StatusBar" (28px, `--layout-statusbar-h`):
 * Rust core version/health · per-destination health (Google Drive, AWS S3)
 * · build target, right-aligned. RF-069 (Should-have, PLAN.md T-5.6) adds a
 * "Ping: N ms" segment *per destination* — appended to that destination's
 * own segment (e.g. "GDrive: Online · Ping: 24 ms") rather than as one
 * combined top-level segment, so a reader doesn't have to guess which
 * number belongs to which destination. Rendered only when that
 * destination's `latencyMs` is a number (§8 doesn't fix a position for it;
 * this is the documented choice for T-5.6).
 *
 * Colors follow §8 verbatim: a destination dot/text uses its own brand color
 * (`--color-primary` for Google Drive, `--color-secondary` for AWS S3) when
 * `online`, `--color-error` when `auth_required` (also the treatment named in
 * §2's "auth-required" matrix row), and the neutral `--color-text-tertiary`
 * when `offline`.
 *
 * Strings sourced from `src/i18n/pt-BR.json` via `t()` (T-1.6).
 */
import type { JSX } from "react";
import { t } from "../../i18n";

export type DestinationHealth = {
  state: "online" | "offline" | "auth_required";
  latencyMs?: number;
};

export type StatusBarProps = {
  coreVersion: string;
  coreActive: boolean;
  gdrive: DestinationHealth;
  s3: DestinationHealth & { region?: string };
  buildTarget: string;
};

/** Pure label lookup for a destination health state — exported for tests. */
export function healthLabel(state: DestinationHealth["state"]): string {
  const labelMap: Record<DestinationHealth["state"], string> = {
    online: t("shell.statusBar.online"),
    offline: t("shell.statusBar.offline"),
    auth_required: t("shell.statusBar.authRequired"),
  };
  return labelMap[state];
}

type DestinationBrand = "primary" | "secondary";

function brandColorClassName(brand: DestinationBrand, prefix: "bg" | "text"): string {
  return brand === "primary" ? `${prefix}-primary` : `${prefix}-secondary`;
}

function destinationDotClassName(state: DestinationHealth["state"], brand: DestinationBrand): string {
  if (state === "auth_required") return "bg-error";
  if (state === "offline") return "bg-text-tertiary";
  return brandColorClassName(brand, "bg");
}

function destinationTextClassName(state: DestinationHealth["state"], brand: DestinationBrand): string {
  if (state === "auth_required") return "text-error";
  if (state === "offline") return "text-text-tertiary";
  return brandColorClassName(brand, "text");
}

function Separator(): JSX.Element {
  return <span aria-hidden="true" className="h-3 w-px shrink-0 bg-border-hairline" />;
}

/** "Ping: N ms" (RF-069), or `""` when this destination hasn't reported a latency. */
function pingSuffix(latencyMs: number | undefined): string {
  return latencyMs === undefined ? "" : ` · ${t("shell.statusBar.ping", { ms: String(latencyMs) })}`;
}

type DestinationSegmentProps = {
  label: string;
  health: DestinationHealth;
  brand: DestinationBrand;
  ariaLabel: string;
  suffix?: string;
};

function DestinationSegment({ label, health, brand, ariaLabel, suffix }: DestinationSegmentProps): JSX.Element {
  return (
    <span className="flex items-center gap-xs" aria-label={ariaLabel}>
      <span
        aria-hidden="true"
        className={`h-1.5 w-1.5 shrink-0 rounded-full ${destinationDotClassName(health.state, brand)}`}
      />
      <span className={destinationTextClassName(health.state, brand)}>
        {`${label}: ${healthLabel(health.state)}${suffix ?? ""}`}
      </span>
    </span>
  );
}

export function StatusBar({ coreVersion, coreActive, gdrive, s3, buildTarget }: StatusBarProps): JSX.Element {
  return (
    <footer
      role="contentinfo"
      aria-live="polite"
      className="h-statusbar flex items-center gap-lg border-t border-border-hairline bg-surface-1 px-md font-mono text-label-sm text-text-tertiary"
    >
      <span className="flex items-center gap-xs">
        <span
          aria-hidden="true"
          className={`h-1.5 w-1.5 shrink-0 rounded-full ${coreActive ? "bg-tertiary" : "bg-text-tertiary"}`}
        />
        <span className="text-text-status">
          {t("shell.statusBar.coreVersion", {
            version: coreVersion,
            status: coreActive ? t("shell.statusBar.coreStatusActive") : t("shell.statusBar.coreStatusInactive"),
          })}
        </span>
      </span>

      <Separator />
      <DestinationSegment
        label={t("shell.statusBar.gdrive")}
        health={gdrive}
        brand="primary"
        ariaLabel={`Google Drive: ${healthLabel(gdrive.state)}${pingSuffix(gdrive.latencyMs)}`}
        suffix={pingSuffix(gdrive.latencyMs)}
      />

      <Separator />
      <DestinationSegment
        label={t("shell.statusBar.s3")}
        health={s3}
        brand="secondary"
        ariaLabel={`AWS S3: ${healthLabel(s3.state)}${s3.region ? ` (${s3.region})` : ""}${pingSuffix(s3.latencyMs)}`}
        suffix={`${s3.region ? ` (${s3.region})` : ""}${pingSuffix(s3.latencyMs)}`}
      />

      <span className="ml-auto text-text-tertiary">{buildTarget}</span>
    </footer>
  );
}
