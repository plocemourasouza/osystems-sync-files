/**
 * Tooltip — hover/focus hint for icon-only controls (DESIGN.md §8 "Tooltip").
 *
 * Wraps a single child in an `inline-flex` anchor span that carries the
 * pointer/focus handlers, so the tooltip also works for `disabled` children
 * (a disabled `<button>` fires no pointer events of its own). The child keeps
 * its own accessible name (`aria-label`) and gains `aria-describedby`
 * pointing at the bubble, which is the WAI-ARIA pattern for a supplementary
 * description rather than a replacement name.
 *
 * The bubble renders through `createPortal` into `document.body` with
 * `position: fixed`, positioned from the anchor's `getBoundingClientRect()`
 * and flipped below when it would clip the viewport top — the same escape
 * hatch `RowActions` needs so the bubble is not clipped by `JobTable`'s
 * `overflow-x-auto` wrapper.
 */
import {
  cloneElement,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type JSX,
  type ReactElement,
} from "react";
import { createPortal } from "react-dom";

/** Matches DESIGN.md's "delay before a hint appears"; short enough to feel instant on purpose. */
const OPEN_DELAY_MS = 250;
/** Gap between the anchor and the bubble, in px (`--space-xs`). */
const OFFSET = 4;
/** Bubble height used for the flip decision before it is measured. */
const ESTIMATED_HEIGHT = 24;

export type TooltipProps = {
  /** The hint text. Rendered as-is; keep it to a short phrase. */
  label: string;
  /** Single interactive element the hint describes. */
  children: ReactElement<{ "aria-describedby"?: string }>;
};

export function Tooltip({ label, children }: TooltipProps): JSX.Element {
  const tooltipId = useId();
  const anchorRef = useRef<HTMLSpanElement>(null);
  const bubbleRef = useRef<HTMLDivElement>(null);
  const openTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState<{ top: number; left: number } | null>(null);

  const close = useCallback(() => {
    if (openTimer.current) {
      clearTimeout(openTimer.current);
      openTimer.current = null;
    }
    setOpen(false);
    setPosition(null);
  }, []);

  const scheduleOpen = useCallback(() => {
    if (openTimer.current) clearTimeout(openTimer.current);
    openTimer.current = setTimeout(() => setOpen(true), OPEN_DELAY_MS);
  }, []);

  /** Focus (keyboard) shows the hint immediately — no dwell to wait out. */
  const openNow = useCallback(() => {
    if (openTimer.current) clearTimeout(openTimer.current);
    setOpen(true);
  }, []);

  useEffect(() => {
    return () => {
      if (openTimer.current) clearTimeout(openTimer.current);
    };
  }, []);

  useLayoutEffect(() => {
    if (!open) return;

    const anchor = anchorRef.current;
    if (!anchor) return;

    const rect = anchor.getBoundingClientRect();
    const height = bubbleRef.current?.offsetHeight ?? ESTIMATED_HEIGHT;
    const width = bubbleRef.current?.offsetWidth ?? 0;

    const above = rect.top - height - OFFSET;
    const top = above >= 0 ? above : rect.bottom + OFFSET;

    const centered = rect.left + rect.width / 2 - width / 2;
    const left = Math.max(OFFSET, Math.min(centered, window.innerWidth - width - OFFSET));

    setPosition({ top, left });
  }, [open, label]);

  useEffect(() => {
    if (!open) return;

    function handleKeyDown(event: KeyboardEvent): void {
      if (event.key === "Escape") close();
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [open, close]);

  const child = cloneElement(children, { "aria-describedby": open ? tooltipId : undefined });

  return (
    <>
      <span
        ref={anchorRef}
        className="inline-flex"
        onMouseEnter={scheduleOpen}
        onMouseLeave={close}
        onFocus={openNow}
        onBlur={close}
      >
        {child}
      </span>

      {open &&
        createPortal(
          <div
            ref={bubbleRef}
            id={tooltipId}
            role="tooltip"
            style={{ top: position?.top ?? 0, left: position?.left ?? 0 }}
            className="pointer-events-none fixed z-50 max-w-64 whitespace-nowrap rounded border border-border-hairline bg-surface-2 px-xs py-2xs text-label-md text-text-primary shadow-popover"
          >
            {label}
          </div>,
          document.body,
        )}
    </>
  );
}
