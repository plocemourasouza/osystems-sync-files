import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/**
 * Guards drift between `design/tokens.css` (source of truth, read-only) and
 * the Tailwind `@theme` block in `src/styles/app.css`: every `--color-*`,
 * `--font-*` and `--radius-*` custom property declared in tokens.css must be
 * referenced somewhere inside app.css's `@theme` block, or the token
 * silently stops being usable as a Tailwind utility.
 *
 * Reads via `node:fs` (typed by the local `node-builtins.d.ts` ambient
 * declarations, since this project has no `@types/node` dependency) rather
 * than a Vite-pipeline import: `@tailwindcss/vite` intercepts `.css` module
 * loads — including `?raw` ones — under Vitest and returns its own compiled
 * (here, empty) output instead of the literal source text.
 */

const currentDir = dirname(fileURLToPath(import.meta.url));

const tokensCss = readFileSync(resolve(currentDir, "../../design/tokens.css"), "utf-8");
const appCss = readFileSync(resolve(currentDir, "./app.css"), "utf-8");

function extractThemeBlock(css: string): string {
  const opening = css.match(/@theme[^{]*\{/);
  if (!opening || opening.index === undefined) {
    throw new Error("No @theme block found in app.css");
  }
  const openingText = opening[0];
  if (openingText === undefined) {
    throw new Error("Malformed @theme match in app.css");
  }

  let depth = 0;
  const start = opening.index + openingText.length - 1;
  for (let i = start; i < css.length; i++) {
    const char = css[i];
    if (char === "{") depth++;
    if (char === "}") {
      depth--;
      if (depth === 0) {
        return css.slice(start, i + 1);
      }
    }
  }
  throw new Error("Unterminated @theme block in app.css");
}

function extractTokenNames(css: string, prefix: "--color-" | "--font-" | "--radius-"): string[] {
  const pattern = new RegExp(`^\\s*(${prefix}[a-zA-Z0-9-]+)\\s*:`, "gm");
  const names = new Set<string>();
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(css)) !== null) {
    const name = match[1];
    if (name !== undefined) {
      names.add(name);
    }
  }
  return [...names];
}

const themeBlock = extractThemeBlock(appCss);

const prefixes = ["--color-", "--font-", "--radius-"] as const;

describe("design tokens -> Tailwind @theme mapping", () => {
  for (const prefix of prefixes) {
    const tokenNames = extractTokenNames(tokensCss, prefix);

    it(`declares at least one ${prefix}* token in tokens.css`, () => {
      expect(tokenNames.length).toBeGreaterThan(0);
    });

    it.each(tokenNames)(`%s is referenced in app.css's @theme block`, (name) => {
      expect(themeBlock).toContain(name);
    });
  }
});
