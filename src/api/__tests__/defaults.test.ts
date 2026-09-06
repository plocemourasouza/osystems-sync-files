import { describe, expect, it } from "vitest";

import { DEFAULT_CONFIG } from "@/api/defaults";
import type { AppConfig } from "@/types/generated";

import fixture from "../__fixtures__/default-config.json";

/**
 * Compile-time check: `DEFAULT_CONFIG` must be a valid `AppConfig` (this line
 * fails `tsc` if `defaults.ts`'s `satisfies AppConfig` ever regresses, e.g. after
 * a generated-type rename).
 */
const _typeCheck: AppConfig = DEFAULT_CONFIG;
void _typeCheck;

/**
 * `JSON.stringify` cannot serialize `bigint`, and the fixture (plain JSON,
 * mirroring `AppConfig::default()`) has no way to express one either — so `u64`
 * fields compare as `Number(bigint)` here.
 */
function normalizeBigInts(value: unknown): unknown {
  if (typeof value === "bigint") return Number(value);
  if (Array.isArray(value)) return value.map(normalizeBigInts);
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(([key, v]) => [key, normalizeBigInts(v)]),
    );
  }
  return value;
}

describe("DEFAULT_CONFIG", () => {
  it("matches AppConfig::default() (src-tauri/crates/core/src/config.rs, SPEC.md §5)", () => {
    expect(normalizeBigInts(DEFAULT_CONFIG)).toEqual(fixture);
  });
});
