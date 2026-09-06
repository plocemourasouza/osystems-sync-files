/**
 * LogConsole — collapsible real-time event console at the bottom of the
 * Dashboard (PRD.md RF-066, RF-098; DESIGN.md §8 "LogConsole"/"LogLine", §9
 * dashboard map).
 *
 * Self-contained: hydrates the ring buffer via `logStore.fetchRecent()`
 * (`get_recent_logs`) on mount, then appends every live `log-line` event
 * (`useTauriEvent`, `@/api/events`) through `logStore.push`. The store
 * already caps the ring at `LOG_RING_CAPACITY` (500) — see `@/store/logStore`.
 *
 * RNF-008 ("UI responsiva… sem jank perceptível"): on top of the store's
 * 500-line cap, this component renders at most `MAX_VISIBLE_LINES` (200)
 * DOM rows — the cheapest form of virtualization that still satisfies "last
 * 500 lines kept, console never grows unbounded in memory" without pulling
 * in a virtualization library. The line-count label reads the full
 * filtered/ring-capped length, not the visible slice, so it stays accurate.
 *
 * Auto-scroll: the body scrolls to bottom on every new line unless the user
 * has scrolled up more than `SCROLL_PAUSE_THRESHOLD_PX`, in which case a
 * "↓ Novas linhas" pill appears instead of yanking their scroll position.
 */
import { ChevronDown, FolderOpen } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type JSX, type UIEvent } from "react";

import { useTauriEvent } from "@/api/events";
import { openLogsFolder } from "@/api/ipc";
import { Badge, Button, cn, Select } from "@/components/ui";
import { t, type MessageKey } from "@/i18n";
import { selectFilteredLines, useLogStore, type LogLevelFilter } from "@/store/logStore";

import { LogLineRow } from "./LogLineRow";
import { LogTicker } from "./LogTicker";

/** RNF-008: DOM row cap — independent of the store's 500-line ring buffer cap. */
const MAX_VISIBLE_LINES = 200;
/** Distance (px) from the bottom past which auto-scroll pauses and the "new lines" pill shows. */
const SCROLL_PAUSE_THRESHOLD_PX = 40;

const LEVEL_OPTIONS: { value: LogLevelFilter; labelKey: MessageKey }[] = [
  { value: "all", labelKey: "pages.dashboard.console.levelAll" },
  { value: "debug", labelKey: "pages.dashboard.console.levelDebug" },
  { value: "info", labelKey: "pages.dashboard.console.levelInfo" },
  { value: "warn", labelKey: "pages.dashboard.console.levelWarn" },
  { value: "error", labelKey: "pages.dashboard.console.levelError" },
];

function isLogLevelFilter(value: string): value is LogLevelFilter {
  return LEVEL_OPTIONS.some((option) => option.value === value);
}

export function LogConsole(): JSX.Element {
  const lines = useLogStore((state) => state.lines);
  const level = useLogStore((state) => state.level);
  const collapsed = useLogStore((state) => state.collapsed);
  const setLevel = useLogStore((state) => state.setLevel);
  const toggleCollapsed = useLogStore((state) => state.toggleCollapsed);
  const push = useLogStore((state) => state.push);
  const fetchRecent = useLogStore((state) => state.fetchRecent);

  const bodyRef = useRef<HTMLDivElement>(null);
  const previousLineCount = useRef(0);
  const [isScrolledUp, setIsScrolledUp] = useState(false);
  const [hasNewLines, setHasNewLines] = useState(false);

  useEffect(() => {
    void fetchRecent();
  }, [fetchRecent]);

  useTauriEvent("log-line", push);

  const filtered = useMemo(() => selectFilteredLines(lines, level), [lines, level]);
  const visible = useMemo(() => filtered.slice(-MAX_VISIBLE_LINES), [filtered]);
  // `push` appends, so the newest line is the last one — the same line the
  // body auto-scrolls to. `LogTicker` shows it while collapsed.
  const latest = filtered.at(-1);

  useEffect(() => {
    if (filtered.length === previousLineCount.current) return;
    previousLineCount.current = filtered.length;

    const body = bodyRef.current;
    if (!body) return;

    if (isScrolledUp) {
      setHasNewLines(true);
      return;
    }
    body.scrollTop = body.scrollHeight;
  }, [filtered.length, isScrolledUp]);

  function handleScroll(event: UIEvent<HTMLDivElement>): void {
    const body = event.currentTarget;
    const distanceFromBottom = body.scrollHeight - body.scrollTop - body.clientHeight;
    const scrolledUp = distanceFromBottom >= SCROLL_PAUSE_THRESHOLD_PX;
    setIsScrolledUp(scrolledUp);
    if (!scrolledUp) setHasNewLines(false);
  }

  function scrollToBottom(): void {
    const body = bodyRef.current;
    if (!body) return;
    body.scrollTop = body.scrollHeight;
    setIsScrolledUp(false);
    setHasNewLines(false);
  }

  return (
    <section className="flex shrink-0 flex-col border-t border-border-hairline bg-surface-1">
      <header className="flex h-8 shrink-0 items-center gap-sm border-b border-border-hairline px-sm">
        <button
          type="button"
          aria-expanded={!collapsed}
          aria-label={t(collapsed ? "pages.dashboard.console.expandLabel" : "pages.dashboard.console.collapseLabel")}
          onClick={toggleCollapsed}
          className="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded text-text-secondary transition-colors duration-fast hover:bg-surface-hover hover:text-text-primary"
        >
          {/*
            The console is docked at the bottom of the Dashboard, so the
            chevron reads as the direction the panel will move: down while
            expanded (click to drop it), up once collapsed (click to raise
            it). The old `-rotate-90` pointed sideways, which said nothing
            about where the body went.
          */}
          <ChevronDown
            aria-hidden="true"
            size={16}
            className={cn("transition-transform duration-fast", collapsed && "rotate-180")}
          />
        </button>

        <h2 className="shrink-0 truncate text-label-md text-text-primary">
          {t("pages.dashboard.console.title")}
        </h2>
        <Badge tone="neutral" className="shrink-0">
          {t("pages.dashboard.console.daemonChip")}
        </Badge>

        {/*
          Collapsed, the header is the only thing left of the console, so the
          newest line rides here — right after the "Rust Core" chip — and the
          panel keeps reporting while shut. Expanded it would just duplicate
          the last row of the body, so it is not rendered.
        */}
        {collapsed && latest && <LogTicker line={latest} />}

        <div className="ml-auto flex shrink-0 flex-nowrap items-center gap-sm">
          <span className="shrink-0 whitespace-nowrap font-mono text-label-sm text-text-quaternary">
            {t("pages.dashboard.console.lineCount", { count: filtered.length })}
          </span>

          <Select
            id="log-console-level"
            label={t("pages.dashboard.console.levelLabel")}
            hideLabel
            aria-label={t("pages.dashboard.console.levelLabel")}
            value={level}
            onChange={(value) => {
              if (isLogLevelFilter(value)) setLevel(value);
            }}
            options={LEVEL_OPTIONS.map((option) => ({ value: option.value, label: t(option.labelKey) }))}
            className="w-28 shrink-0"
          />

          <span
            aria-live="off"
            className="flex shrink-0 items-center gap-2xs"
            aria-label={t("pages.dashboard.console.live")}
          >
            <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-primary" />
            <span className="hidden whitespace-nowrap text-label-sm text-tertiary min-[1200px]:inline">
              {t("pages.dashboard.console.liveText")}
            </span>
          </span>

          <Button
            variant="ghost"
            size="sm"
            icon={<FolderOpen aria-hidden="true" size={16} />}
            aria-label={t("pages.dashboard.console.openFolder")}
            onClick={() => void openLogsFolder()}
          />
        </div>
      </header>

      {!collapsed && (
        <div className="relative">
          <div
            ref={bodyRef}
            role="log"
            aria-live="polite"
            aria-relevant="additions"
            onScroll={handleScroll}
            className="h-[220px] overflow-auto bg-surface-0 px-sm py-2xs"
          >
            {visible.map((line, index) => (
              <LogLineRow key={`${line.ts}-${index}`} line={line} />
            ))}
          </div>

          {hasNewLines && (
            <button
              type="button"
              onClick={scrollToBottom}
              className="absolute bottom-sm left-1/2 -translate-x-1/2 rounded bg-primary-container px-sm py-[2px] text-label-sm text-on-primary shadow-popover transition-colors duration-fast hover:brightness-110"
            >
              {t("pages.dashboard.console.newLinesPill")}
            </button>
          )}
        </div>
      )}
    </section>
  );
}
