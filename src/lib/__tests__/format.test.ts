import { describe, expect, it } from "vitest";
import { formatBytes, formatPct } from "@/lib/format";

describe("formatBytes", () => {
  it("formats gigabytes with two decimals", () => {
    expect(formatBytes(6.42e9)).toBe("6.42 GB");
  });

  it("formats megabytes with one decimal", () => {
    expect(formatBytes(420e6)).toBe("420.0 MB");
  });

  it("formats kilobytes with one decimal", () => {
    expect(formatBytes(12.4e3)).toBe("12.4 KB");
  });

  it("formats whole bytes without a decimal", () => {
    expect(formatBytes(512)).toBe("512 B");
  });

  it("formats zero as 0 B", () => {
    expect(formatBytes(0)).toBe("0 B");
  });

  it("formats negative/non-finite input as 0 B", () => {
    expect(formatBytes(-10)).toBe("0 B");
    expect(formatBytes(Number.NaN)).toBe("0 B");
  });
});

describe("formatPct", () => {
  it("formats a 0..1 ratio as a rounded whole-number percentage", () => {
    expect(formatPct(0.57)).toBe("57%");
  });

  it("rounds to nearest whole percent", () => {
    expect(formatPct(0.005)).toBe("1%");
    expect(formatPct(0.004)).toBe("0%");
  });

  it("clamps above 1 to 100%", () => {
    expect(formatPct(1.5)).toBe("100%");
  });

  it("formats zero/negative as 0%", () => {
    expect(formatPct(0)).toBe("0%");
    expect(formatPct(-0.2)).toBe("0%");
  });
});
