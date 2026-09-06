import { describe, expect, it } from "vitest";

import { relativeDir } from "../relativeDir";

describe("relativeDir()", () => {
  it.each([
    // [path, root, expected]
    ["C:\\monitoramento\\file.txt", "C:\\monitoramento", "./"],
    ["C:\\monitoramento\\sub\\file.txt", "C:\\monitoramento", "./sub"],
    ["C:\\monitoramento\\sub\\deep\\file.txt", "C:/monitoramento", "./sub/deep"],
    ["//?/C:/monitoramento/file.txt", "C:\\monitoramento", "./"],
    ["\\\\?\\C:\\monitoramento\\file.txt", "C:\\monitoramento", "./"],
    ["\\\\?\\C:\\monitoramento\\sub\\file.txt", "//?/C:/monitoramento", "./sub"],
    // Trailing slash on root shouldn't affect matching.
    ["C:\\monitoramento\\sub\\file.txt", "C:\\monitoramento\\", "./sub"],
    // Case-insensitive root comparison (Windows drive letters/paths).
    ["c:\\monitoramento\\sub\\file.txt", "C:\\Monitoramento", "./sub"],
  ])("relativeDir(%j, %j) -> %j", (path, root, expected) => {
    expect(relativeDir(path, root)).toBe(expected);
  });

  it("falls back to the raw normalized directory when no root is configured", () => {
    expect(relativeDir("//?/C:/temp/file.txt", null)).toBe("C:/temp");
    expect(relativeDir("C:\\temp\\sub\\file.txt", null)).toBe("C:/temp/sub");
  });

  it("falls back to the raw normalized directory when the file is outside the root", () => {
    expect(relativeDir("C:\\other\\file.txt", "C:\\monitoramento")).toBe("C:/other");
  });

  it("treats an empty root the same as no root", () => {
    expect(relativeDir("//?/C:/temp/file.txt", "")).toBe("C:/temp");
  });
});
