/**
 * `isMockMode` — dev-only escape hatch to preview the Dashboard with fixture
 * data instead of live Tauri IPC, for browser preview (`npm run dev`) where
 * `invoke()` has nothing to talk to.
 *
 * Gated on `import.meta.env.DEV` so Vite's static replacement folds this to
 * `false` in a production build, dead-code-eliminating every caller's guarded
 * branch (see `src/dev/mock.ts`'s module doc for the tree-shaking contract).
 */
export function isMockMode(): boolean {
  return import.meta.env.DEV && new URLSearchParams(window.location.search).has("mock");
}
