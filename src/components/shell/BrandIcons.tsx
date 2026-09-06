/**
 * BrandIcons — inline SVG marks for the author's social profiles.
 *
 * Hand-vendored rather than imported: `lucide-react` dropped its brand icons
 * (verified against the installed 1.41 — `Github`, `Instagram`, `Linkedin`
 * and `Facebook` are all absent), and the CSP is `'self'`, so pulling an icon
 * font or an SVG sprite from a CDN is not an option either (CLAUDE.md "Não
 * fazer"). These are drawn from primitives on the same 24×24 grid, 2px
 * stroke, `currentColor` — so they line up with the lucide icons beside them
 * in the sidebar and inherit hover/focus colour for free.
 *
 * Marks are simplified silhouettes of each brand's glyph, used to label a
 * link to that profile — not reproductions of the trademarked logo files.
 */
import type { JSX, SVGProps } from "react";

export type BrandIconProps = Omit<SVGProps<SVGSVGElement>, "children" | "viewBox"> & {
  /** Rendered size in px, both axes. Matches lucide's `size` prop. */
  size?: number;
};

/** Shared frame: 24×24, stroked, non-scaling geometry, hidden from AT. */
function Frame({ size = 16, ...rest }: BrandIconProps): JSX.Element {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      {...rest}
    />
  );
}

/** Rounded square, lens, and the small top-right dot. */
export function InstagramIcon(props: BrandIconProps): JSX.Element {
  return (
    <Frame {...props}>
      <rect x="2" y="2" width="20" height="20" rx="5" />
      <circle cx="12" cy="12" r="4" />
      <circle cx="17.5" cy="6.5" r="1" fill="currentColor" stroke="none" />
    </Frame>
  );
}

/** The lowercase "f" descending out of a rounded square. */
export function FacebookIcon(props: BrandIconProps): JSX.Element {
  return (
    <Frame {...props}>
      <path d="M18 2h-3a5 5 0 0 0-5 5v3H7v4h3v8h4v-8h3l1-4h-4V7a1 1 0 0 1 1-1h3z" />
    </Frame>
  );
}

/** The "in" mark: the dotted stem on the left, the arch on the right. */
export function LinkedinIcon(props: BrandIconProps): JSX.Element {
  return (
    <Frame {...props}>
      <path d="M16 8a6 6 0 0 1 6 6v7h-4v-7a2 2 0 0 0-4 0v7h-4v-13h4z" />
      <rect x="2" y="9" width="4" height="12" />
      <circle cx="4" cy="4" r="2" />
    </Frame>
  );
}

/**
 * The chat bubble with a handset inside — WhatsApp's mark reduced to the two
 * shapes that survive at 17px. The bubble is drawn tail-down-left like the
 * original; the handset is a single stroke rather than the filled glyph,
 * which turns to mush at this size.
 */
export function WhatsappIcon(props: BrandIconProps): JSX.Element {
  return (
    <Frame {...props}>
      <path d="M21 11.5a8.5 8.5 0 0 1-12.5 7.5L3 21l2-5.5A8.5 8.5 0 1 1 21 11.5z" />
      {/* Filled, not stroked: an outlined receiver this small collapses into
          a squiggle — the solid shape is what still reads as a handset. */}
      <path
        fill="currentColor"
        stroke="none"
        d="M9.6 7.6c-.2-.5-.4-.5-.6-.5h-.5c-.2 0-.5.1-.8.4-.3.3-1 1-1 2.4s1 2.8 1.2 3c.1.2 2 3.1 4.9 4.3 2.4 1 2.9.8 3.4.8.5 0 1.6-.7 1.9-1.3.2-.7.2-1.2.2-1.3-.1-.1-.3-.2-.5-.3l-1.9-.9c-.3-.1-.5-.2-.7.1l-.7.9c-.2.2-.3.3-.6.1-.3-.1-1.3-.5-2.4-1.5-.9-.8-1.5-1.8-1.7-2.1-.1-.3 0-.4.2-.6l.4-.5c.2-.2.2-.3.3-.5.1-.2 0-.4 0-.5z"
      />
    </Frame>
  );
}

/** The cat silhouette: rounded body, ears, and the trailing tail. */
export function GithubIcon(props: BrandIconProps): JSX.Element {
  return (
    <Frame {...props}>
      <path d="M15 22v-4a4.8 4.8 0 0 0-1-3.5c3 0 6-2 6-5.5.08-1.25-.27-2.48-1-3.5.28-1.15.28-2.35 0-3.5 0 0-1 0-3 1.5-2.64-.5-5.36-.5-8 0C6 2 5 2 5 2c-.3 1.15-.3 2.35 0 3.5A5.4 5.4 0 0 0 4 9c0 3.5 3 5.5 6 5.5-.39.49-.68 1.05-.85 1.65-.17.6-.22 1.23-.15 1.85v4" />
      <path d="M9 18c-4.5 2-5-2-7-2" />
    </Frame>
  );
}
