import { describe, expect, it } from "vitest";

import { relativeTime } from "../relativeTime";

const NOW = new Date("2026-01-01T12:00:00.000Z");

describe("relativeTime()", () => {
  it.each([
    ["2026-01-01T12:00:00.000Z", "agora"],
    ["2026-01-01T11:59:30.000Z", "agora"],
    ["2026-01-01T11:58:00.000Z", "há 2 min"],
    ["2026-01-01T11:12:00.000Z", "há 48 min"],
    ["2026-01-01T09:00:00.000Z", "há 3 h"],
    ["2025-12-31T12:00:00.000Z", "ontem"],
    ["2025-12-28T12:00:00.000Z", "há 4 dias"],
  ])("relativeTime(%s) -> %s", (iso, expected) => {
    expect(relativeTime(iso, NOW)).toBe(expected);
  });

  it("defaults `now` to the current wall-clock time", () => {
    const justNow = new Date(Date.now() - 1000).toISOString();
    expect(relativeTime(justNow)).toBe("agora");
  });
});
