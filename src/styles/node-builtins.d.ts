/**
 * Minimal ambient declarations for the Node built-ins used by
 * `tokens.test.ts`. This project has no `@types/node` dependency (it's a
 * strict frontend-only TS config — see PLAN.md "no extra npm deps" for
 * T-0.2), so the small, stable slice of `node:fs` / `node:path` / `node:url`
 * that test needs is declared here instead of pulling in the full package.
 *
 * Reading the two CSS files via plain Node fs (rather than
 * `import.meta.glob(..., { query: "?raw" })`) is deliberate: under Vitest,
 * `@tailwindcss/vite`'s loader intercepts every `.css` module — including
 * `?raw` ones — and returns its own (here, empty) compiled output instead of
 * the literal source text, so a Vite-pipeline read of these files is
 * unreliable for this guard. Filesystem reads bypass that pipeline entirely.
 */
declare module "node:fs" {
  export function readFileSync(path: string, encoding: "utf-8"): string;
}

declare module "node:path" {
  export function resolve(...segments: string[]): string;
  export function dirname(path: string): string;
}

declare module "node:url" {
  export function fileURLToPath(url: string): string;
}
