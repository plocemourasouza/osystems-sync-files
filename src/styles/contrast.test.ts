import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/**
 * WCAG 2.1 AA contrast guard (RNF-013, PLAN.md T-6.2, a11y audit).
 *
 * Parses every contrast pair table in `design/DESIGN.md` §10 "Acessibilidade"
 * and recomputes each ratio directly from `design/tokens.css`'s real hex
 * values, using the standard WCAG relative-luminance formula (§10's own
 * "sRGB linearizado" wording — i.e. gamma-corrected channels, not raw 0–255
 * values). Two things are asserted per row:
 *
 * 1. **Drift guard** — the freshly computed ratio must match the ratio
 *    documented in DESIGN.md within ±0.05, so the doc can never silently go
 *    stale if a token's hex changes.
 * 2. **Classification guard** — the computed ratio must fall on the same
 *    side of the relevant AA threshold (4.5:1 normal text / 3.0:1 large
 *    text or non-text UI) as DESIGN.md's own "Pass"/"Fail" verdict. This
 *    intentionally does NOT require every pair to pass — §10 documents
 *    several pairs (`--color-text-quaternary`, `--color-primary` over
 *    `--color-surface-2`, the two `--color-border-*` non-text pairs, etc.)
 *    as *known, mitigated* failures with a written rationale in prose. This
 *    test verifies the documented verdict is honest, not that every pair
 *    is AA-compliant. A small epsilon around each threshold absorbs
 *    DESIGN.md's own 2-decimal rounding and its explicitly noted
 *    "Pass (margem de 0,0N)" / "Fail (por 0,0N)" boundary cases.
 *
 * Reads via `node:fs` (per `src/styles/tokens.test.ts`'s established
 * pattern): `@tailwindcss/vite` intercepts `.css` module loads under Vitest,
 * so a Vite-pipeline import would not yield the literal source text.
 */

const currentDir = dirname(fileURLToPath(import.meta.url));

const tokensCss = readFileSync(resolve(currentDir, "../../design/tokens.css"), "utf-8");
const designMd = readFileSync(resolve(currentDir, "../../design/DESIGN.md"), "utf-8");

// ---------------------------------------------------------------------------
// Token hex resolution (literal `--color-*: #rrggbb;` declarations only —
// the `color-mix()` status-background tokens are read from DESIGN.md's own
// precomputed "Fundo efetivo" column instead, see the badges section below).
// ---------------------------------------------------------------------------

const HEX_TOKEN_PATTERN = /(--color-[a-z0-9-]+):\s*(#[0-9a-fA-F]{6});/g;

const tokenHex = new Map<string, string>();
for (const match of tokensCss.matchAll(HEX_TOKEN_PATTERN)) {
  const name = match[1];
  const hex = match[2];
  if (name !== undefined && hex !== undefined) {
    tokenHex.set(name, hex);
  }
}

function hexOf(token: string): string {
  const hex = tokenHex.get(token);
  if (hex === undefined) {
    throw new Error(`Token "${token}" has no literal hex declaration in design/tokens.css`);
  }
  return hex;
}

// ---------------------------------------------------------------------------
// WCAG relative luminance / contrast ratio
// ---------------------------------------------------------------------------

function linearizeChannel(channel8bit: number): number {
  const c = channel8bit / 255;
  return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

function relativeLuminance(hex: string): number {
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  return 0.2126 * linearizeChannel(r) + 0.7152 * linearizeChannel(g) + 0.0722 * linearizeChannel(b);
}

function contrastRatio(hexA: string, hexB: string): number {
  const lA = relativeLuminance(hexA);
  const lB = relativeLuminance(hexB);
  const lighter = Math.max(lA, lB);
  const darker = Math.min(lA, lB);
  return (lighter + 0.05) / (darker + 0.05);
}

// ---------------------------------------------------------------------------
// DESIGN.md §10 table extraction
// ---------------------------------------------------------------------------

function extractSection(heading: string): string {
  const start = designMd.indexOf(heading);
  if (start === -1) {
    throw new Error(`Heading not found in design/DESIGN.md: "${heading}"`);
  }
  const searchFrom = start + heading.length;
  const nextH3 = designMd.indexOf("\n### ", searchFrom);
  const nextH2 = designMd.indexOf("\n## ", searchFrom);
  const candidates = [nextH3, nextH2].filter((i) => i !== -1);
  const end = candidates.length > 0 ? Math.min(...candidates) : designMd.length;
  return designMd.slice(start, end);
}

/** Splits a markdown table into cell arrays, dropping the header and separator rows. */
function tableDataRows(section: string): string[][] {
  return section
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.startsWith("|") && !line.includes("---"))
    .map((line) =>
      line
        .split("|")
        .map((cell) => cell.trim())
        .slice(1, -1),
    )
    .filter((cells) => cells.length > 0 && !(cells[0] ?? "").startsWith("Par"));
}

function parseRatio(cell: string): number {
  const match = cell.match(/(\d+,\d+):1/);
  if (!match?.[1]) {
    throw new Error(`Could not parse a "N,NN:1" ratio out of cell: "${cell}"`);
  }
  return Number(match[1].replace(",", "."));
}

/** DESIGN.md marks a verdict as "Pass"/"**Pass**" or "Fail"/"**Fail**", each optionally followed by a margin note. */
function isPass(cell: string): boolean {
  return cell.replace(/\*/g, "").trim().startsWith("Pass");
}

function colorTokensIn(cell: string): string[] {
  return [...cell.matchAll(/--color-[a-z0-9-]+/g)].map((m) => m[0]);
}

/** ±0.05 — the drift guard's tolerance, matching DESIGN.md's own 2-decimal precision (RNF-013). */
const DRIFT_TOLERANCE = 0.05;
/**
 * DESIGN.md documents several boundary cases explicitly as "Pass (margem de
 * 0,0N)" / "Fail (por 0,0N)" — up to a few hundredths from the nominal
 * threshold. This epsilon keeps the classification guard from flagging
 * those as mismatches while still catching a real, non-trivial drift.
 */
const CLASSIFICATION_EPSILON = 0.06;

function assertDriftGuard(pairLabel: string, computed: number, documented: number): void {
  const drift = Math.abs(computed - documented);
  expect(drift, `${pairLabel}: computed ${computed.toFixed(2)}:1 drifted from DESIGN.md's documented ${documented}:1 by ${drift.toFixed(3)} (tolerance ${DRIFT_TOLERANCE})`).toBeLessThanOrEqual(
    DRIFT_TOLERANCE,
  );
}

function assertClassification(pairLabel: string, computed: number, threshold: number, shouldPass: boolean): void {
  if (shouldPass) {
    expect(
      computed,
      `${pairLabel}: DESIGN.md documents Pass at the ${threshold}:1 threshold, but computed ${computed.toFixed(2)}:1 falls well short`,
    ).toBeGreaterThanOrEqual(threshold - CLASSIFICATION_EPSILON);
  } else {
    expect(
      computed,
      `${pairLabel}: DESIGN.md documents Fail at the ${threshold}:1 threshold, but computed ${computed.toFixed(2)}:1 clears it convincingly`,
    ).toBeLessThan(threshold + CLASSIFICATION_EPSILON);
  }
}

// ---------------------------------------------------------------------------
// "Texto sobre superfície" (RNF-013 normal-text pairs, AA 4.5:1 / AA-large 3.0:1)
// ---------------------------------------------------------------------------

describe("contrast: Texto sobre superfície (design/DESIGN.md §10)", () => {
  const rows = tableDataRows(extractSection("### Texto sobre superfície"));

  it("found rows to verify", () => {
    expect(rows.length).toBeGreaterThan(0);
  });

  it.each(rows)("%s / %s", (pairCell, ratioCell, aaNormalCell, aaGrandeCell) => {
    const [fg, bg] = colorTokensIn(pairCell ?? "");
    if (fg === undefined || bg === undefined) throw new Error(`Could not extract two tokens from "${pairCell}"`);

    const documented = parseRatio(ratioCell ?? "");
    const computed = contrastRatio(hexOf(fg), hexOf(bg));
    const label = `${fg} / ${bg}`;

    assertDriftGuard(label, computed, documented);
    assertClassification(label, computed, 4.5, isPass(aaNormalCell ?? ""));
    assertClassification(label, computed, 3.0, isPass(aaGrandeCell ?? ""));
  });
});

// ---------------------------------------------------------------------------
// "Marca sobre superfície / container" (same 4-column shape as above)
// ---------------------------------------------------------------------------

describe("contrast: Marca sobre superfície / container (design/DESIGN.md §10)", () => {
  const rows = tableDataRows(extractSection("### Marca sobre superfície / container"));

  it("found rows to verify", () => {
    expect(rows.length).toBeGreaterThan(0);
  });

  it.each(rows)("%s", (pairCell, ratioCell, aaNormalCell, aaGrandeCell) => {
    const [fg, bg] = colorTokensIn(pairCell ?? "");
    if (fg === undefined || bg === undefined) throw new Error(`Could not extract two tokens from "${pairCell}"`);

    const documented = parseRatio(ratioCell ?? "");
    const computed = contrastRatio(hexOf(fg), hexOf(bg));
    const label = `${fg} / ${bg}`;

    assertDriftGuard(label, computed, documented);
    assertClassification(label, computed, 4.5, isPass(aaNormalCell ?? ""));
    assertClassification(label, computed, 3.0, isPass(aaGrandeCell ?? ""));
  });
});

// ---------------------------------------------------------------------------
// "Badges de status" — foreground token vs. a precomputed color-mix() blend
// hex given directly in DESIGN.md's "Fundo efetivo" column (the token itself,
// e.g. `--color-status-info-bg`, is a `color-mix()` expression with no
// literal hex to resolve from tokens.css). AA-large (3.0:1) only — badges
// never carry more than a one-word label per §10.
// ---------------------------------------------------------------------------

describe("contrast: Badges de status (design/DESIGN.md §10)", () => {
  const rows = tableDataRows(extractSection("### Badges de status"));

  it("found rows to verify", () => {
    expect(rows.length).toBeGreaterThan(0);
  });

  it.each(rows)("%s", (pairCell, effectiveBgCell, ratioCell, aaGrandeCell) => {
    const [fg] = colorTokensIn(pairCell ?? "");
    if (fg === undefined) throw new Error(`Could not extract a foreground token from "${pairCell}"`);

    const bgHexMatch = (effectiveBgCell ?? "").match(/#[0-9a-fA-F]{6}/);
    if (!bgHexMatch) throw new Error(`Could not extract the effective background hex from "${effectiveBgCell}"`);

    const documented = parseRatio(ratioCell ?? "");
    const computed = contrastRatio(hexOf(fg), bgHexMatch[0]);
    const label = `${fg} / ${bgHexMatch[0]}`;

    assertDriftGuard(label, computed, documented);
    assertClassification(label, computed, 3.0, isPass(aaGrandeCell ?? ""));
  });
});

// ---------------------------------------------------------------------------
// "Bordas e foco" — non-text UI components, WCAG 1.4.11, floor 3.0:1 only.
// ---------------------------------------------------------------------------

describe("contrast: Bordas e foco (design/DESIGN.md §10)", () => {
  const rows = tableDataRows(extractSection("### Bordas e foco"));

  it("found rows to verify", () => {
    expect(rows.length).toBeGreaterThan(0);
  });

  it.each(rows)("%s", (pairCell, ratioCell, floorCell) => {
    const [fg, bg] = colorTokensIn(pairCell ?? "");
    if (fg === undefined || bg === undefined) throw new Error(`Could not extract two tokens from "${pairCell}"`);

    const documented = parseRatio(ratioCell ?? "");
    const computed = contrastRatio(hexOf(fg), hexOf(bg));
    const label = `${fg} / ${bg}`;

    assertDriftGuard(label, computed, documented);
    assertClassification(label, computed, 3.0, isPass(floorCell ?? ""));
  });
});
