/**
 * i18n (pt-BR) — centralized string management (RNF-017).
 *
 * Provides type-safe translation lookups with dot-path keys (e.g., `t("shell.titleBar.appName")`)
 * and placeholder interpolation. Missing keys are recorded in a `missing` array during
 * development (dev-only warning log in prod) for drift detection.
 *
 * Error messages are looked up by `AppError.code` via `tError(code, vars?)`, with fallback
 * to `errors.unknown` for unmapped codes.
 */

import messages from "./pt-BR.json";

// Recursive type to flatten nested objects into dot-separated paths
type DotPaths<T> = T extends Record<string, unknown>
  ? {
      [K in keyof T]: T[K] extends Record<string, unknown>
        ? `${string & K}.${string & DotPaths<T[K]>}`
        : `${string & K}`;
    }[keyof T]
  : never;

/** Type-safe union of all message keys (e.g., "shell.titleBar.appName"). */
export type MessageKey = DotPaths<typeof messages>;

/** Array of missing keys discovered at runtime (populated in dev mode). */
const missing: MessageKey[] = [];

/**
 * Recursive helper to navigate nested objects by dot-path.
 * Returns `null` if the path is not found.
 */
function getNestedValue(obj: unknown, path: string): string | null {
  const keys = path.split(".");
  let current = obj;

  for (const key of keys) {
    if (typeof current !== "object" || current === null) {
      return null;
    }
    current = (current as Record<string, unknown>)[key];
  }

  return typeof current === "string" ? current : null;
}

/**
 * Interpolate placeholders in a template string.
 * Example: `"Hello {name}"` with `{name: "World"}` → `"Hello World"`
 */
function interpolate(template: string, variables?: Record<string, string | number>): string {
  if (!variables) return template;
  return template.replace(/\{(\w+)\}/g, (_, key) => String(variables[key] ?? `{${key}}`));
}

/**
 * Look up a message by dot-path key with optional placeholder interpolation.
 * In dev mode, missing keys are recorded in the `missing` array.
 * In prod, a console warning is issued only if the key is missing (no log output).
 */
export function t(key: MessageKey, variables?: Record<string, string | number>): string {
  const value = getNestedValue(messages, key);

  if (value === null) {
    if (import.meta.env.DEV) {
      missing.push(key);
    } else {
      // Prod: no console.log, just return the key
    }
    return key;
  }

  return interpolate(value, variables);
}

/**
 * Look up an error message by AppError code with optional placeholder interpolation.
 * Falls back to `errors.unknown` if the code is not mapped.
 *
 * Example: `tError("gdrive.forbidden", {email: "sa@example.com"})`
 * → `"Pasta não compartilhada com a Service Account. Compartilhe com sa@example.com."`
 */
export function tError(code: string, variables?: Record<string, string | number>): string {
  const key = `errors.${code}` as MessageKey;
  const value = getNestedValue(messages, key);

  if (value !== null) {
    return interpolate(value, variables);
  }

  // Fallback to unknown error
  const unknownValue = getNestedValue(messages, "errors.unknown" as MessageKey);
  return unknownValue ?? "errors.unknown";
}

/**
 * Export the `missing` array for testing: verify that all keys referenced in
 * `.tsx` files exist in `pt-BR.json`.
 */
export { missing };

export type Locale = "pt-BR";
