/**
 * AuthorCard — credits block under the sidebar's `FolderCard`: who built the
 * app, and links to their public profiles (RF-093).
 *
 * Each link goes out through `openAuthorLink`, which names a profile rather
 * than passing a URL — the addresses live in `commands::system::AuthorLink`,
 * so the renderer cannot ask the OS handler to open anything else, and
 * `capabilities/default.json` gates the four hosts independently.
 *
 * They are `<button>`, not `<a href>`: nothing here navigates the webview (an
 * `href` inside a Tauri window either does nothing or, worse, replaces the
 * app).
 *
 * Icons only, in one row — the sidebar is 220px and four handles stacked
 * vertically cost more height than the folder card above them for
 * information nobody reads twice. The handle is not lost: it stays in the
 * `title` tooltip and in the accessible name, so hovering or a screen reader
 * still gets `Instagram: @plocemourasouza`.
 */
import type { JSX } from "react";

import { openAuthorLink } from "@/api/ipc";
import { t } from "@/i18n";
import type { AuthorLink } from "@/types/generated";

import { FacebookIcon, GithubIcon, InstagramIcon, LinkedinIcon, type BrandIconProps } from "./BrandIcons";

type Profile = {
  link: AuthorLink;
  network: string;
  /** What the user sees — a handle for the two that have one, a URL otherwise. */
  handle: string;
  Icon: (props: BrandIconProps) => JSX.Element;
};

const PROFILES: Profile[] = [
  { link: "instagram", network: "Instagram", handle: "@plocemourasouza", Icon: InstagramIcon },
  { link: "facebook", network: "Facebook", handle: "@plocemourasouza", Icon: FacebookIcon },
  { link: "linkedin", network: "LinkedIn", handle: "in/psouza", Icon: LinkedinIcon },
  { link: "github", network: "GitHub", handle: "plocemourasouza", Icon: GithubIcon },
];

export function AuthorCard(): JSX.Element {
  return (
    <div className="flex flex-col gap-xs rounded-md border border-border-hairline bg-surface-2 p-md">
      {/*
        `text-label-sm` (10px) is already the floor of the type scale, so the
        label reads smaller by widening the gap to the name rather than by
        shrinking further: the name goes to `text-title-md` (15px) in the
        primary colour, a 3px and two-step-of-contrast jump over the 12px
        secondary it replaced.
      */}
      <span className="text-label-sm text-text-quaternary">
        {t("shell.sidebar.author.title")}
      </span>
      <span className="truncate text-title-md text-text-primary">
        {t("shell.sidebar.author.name")}
      </span>

      {/*
        `justify-between` on a full-width row, not a fixed gap: the sidebar has
        a fixed width, so spreading the four marks edge to edge uses the space
        that a `gap` would leave dead on the right.
      */}
      <ul className="mt-xs flex w-full flex-row items-center justify-between">
        {PROFILES.map(({ link, network, handle, Icon }) => (
          <li key={link}>
            <button
              type="button"
              onClick={() => {
                // Fire-and-forget: the OS handler owns the outcome from here,
                // and a failure to open a browser is not something this card
                // can act on. `openAuthorLink` already logs on the Rust side.
                void openAuthorLink(link);
              }}
              aria-label={t("shell.sidebar.author.linkLabel", { network, handle })}
              title={t("shell.sidebar.author.linkLabel", { network, handle })}
              className="flex h-8 w-8 items-center justify-center rounded-md border border-border-hairline text-text-secondary transition-colors duration-fast hover:bg-surface-hover hover:text-text-primary focus-visible:outline-none focus-visible:shadow-focus"
            >
              <Icon size={17} />
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
