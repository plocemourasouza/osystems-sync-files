/**
 * Shared axe-core setup for accessibility tests (RNF-013, PLAN.md T-6.2).
 *
 * Imported only by `*.a11y.test.tsx` files — kept out of `src/test/setup.ts`
 * so every other test file's startup cost/type surface stays untouched.
 *
 * `configureAxe` disables `color-contrast`: axe-core needs a real layout
 * engine (computed styles + actual pixel colors) to evaluate it, which jsdom
 * does not provide — axe-core's own jsdom guidance is to skip this rule in a
 * jsdom environment. Contrast is verified instead by `src/styles/contrast.test.ts`,
 * which computes WCAG relative luminance directly from `design/tokens.css`'s
 * hex values against the ratios documented in `design/DESIGN.md` §10. No
 * other rule is disabled.
 */
import { expect } from "vitest";
import { configureAxe } from "vitest-axe";
import { toHaveNoViolations, type AxeMatchers } from "vitest-axe/matchers";

expect.extend({ toHaveNoViolations });

declare module "vitest" {
  // `T` must be re-declared to match vitest's own `Assertion<T>` for interface
  // merging to typecheck, even though `AxeMatchers` doesn't use it.
  // eslint-disable-next-line @typescript-eslint/no-empty-object-type, @typescript-eslint/no-unused-vars
  interface Assertion<T> extends AxeMatchers {}
  // eslint-disable-next-line @typescript-eslint/no-empty-object-type
  interface AsymmetricMatchersContaining extends AxeMatchers {}
}

export const axe = configureAxe({
  rules: {
    "color-contrast": { enabled: false },
  },
});
