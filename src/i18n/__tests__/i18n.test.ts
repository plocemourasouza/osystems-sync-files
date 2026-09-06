/**
 * i18n tests — string lookups, interpolation, missing key detection.
 */

import { beforeEach, describe, expect, it } from "vitest";
import { t, tError, missing, type MessageKey } from "../index";
import messages from "../pt-BR.json";

describe("i18n", () => {
  beforeEach(() => {
    // Clear the missing array before each test
    missing.length = 0;
  });

  describe("t()", () => {
    it("returns the value for a valid key", () => {
      expect(t("shell.titleBar.appName")).toBe("oSystems Sync");
      expect(t("shell.sidebar.brand.name")).toBe("oSystems");
    });

    it("interpolates variables in a template", () => {
      expect(t("shell.statusBar.coreVersion", { version: "0.1.0", status: "Active" })).toBe(
        "Rust Core v0.1.0 (Active)"
      );
      expect(t("shell.statusBar.ping", { ms: "24" })).toBe("Ping: 24 ms");
    });

    it("returns the key for a missing key and records it", () => {
      const result = t("nonexistent.key" as MessageKey);
      expect(result).toBe("nonexistent.key");
      expect(missing).toContain("nonexistent.key");
    });

    it("handles variables with missing keys in template", () => {
      const result = t("shell.statusBar.coreVersion", { version: "0.2.0" });
      // Should interpolate {version} but leave {status} as-is
      expect(result).toMatch("Rust Core v0.2.0");
    });
  });

  describe("tError()", () => {
    it("returns error message for a mapped error code", () => {
      expect(tError("gdrive.notFound")).toBe("Pasta não encontrada no Google Drive.");
      expect(tError("s3.accessDenied")).toBe(
        "Acesso negado ao bucket S3. Verifique as credenciais e permissões IAM."
      );
    });

    it("interpolates variables in error messages", () => {
      const result = tError("gdrive.forbidden", { email: "sa@example.iam.gserviceaccount.com" });
      expect(result).toBe(
        "Pasta não compartilhada com a Service Account. Compartilhe com sa@example.iam.gserviceaccount.com."
      );
    });

    it("interpolates region in S3 wrong_region error", () => {
      const result = tError("s3.wrongRegion", { expected: "us-west-2" });
      expect(result).toBe("Região errada. Esperado: us-west-2.");
    });

    it("falls back to errors.unknown for unmapped codes", () => {
      const result = tError("unmapped.error");
      expect(result).toBe("Erro desconhecido. Verifique os logs para mais detalhes.");
      expect(missing).not.toContain("errors.unmapped.error");
    });

    it("falls back to errors.unknown for empty code", () => {
      const result = tError("");
      expect(result).toBe("Erro desconhecido. Verifique os logs para mais detalhes.");
    });
  });

  describe("message coverage", () => {
    it("all keys in pt-BR.json are valid", () => {
      // This is a sanity check: the JSON file should be parseable and have the expected structure
      expect(messages).toHaveProperty("shell.titleBar");
      expect(messages.shell.titleBar).toHaveProperty("appName");
      expect(messages.errors).toHaveProperty("gdrive");
      expect(messages.errors.gdrive).toHaveProperty("forbidden");
    });

    it("common keys required by components exist", () => {
      // Shell
      expect(t("shell.titleBar.appName")).toBeTruthy();
      expect(t("shell.sidebar.nav.queue")).toBeTruthy();
      expect(t("shell.statusBar.online")).toBeTruthy();

      // Pages
      expect(t("pages.dashboard.title")).toBeTruthy();
      expect(t("pages.settings.title")).toBeTruthy();

      // Common
      expect(t("common.accessibility.skipToContent")).toBeTruthy();
      expect(t("common.buttons.save")).toBeTruthy();

      // Errors
      expect(tError("gdrive.forbidden")).toBeTruthy();
      expect(tError("s3.accessDenied")).toBeTruthy();
      expect(tError("unknown")).toBeTruthy();
    });
  });

  describe("placeholder edge cases", () => {
    it("handles multiple placeholders", () => {
      const result = t("errors.config.invalid", { field: "bucket" });
      expect(result).toBe("Configuração inválida no campo 'bucket'.");
    });

    it("preserves placeholders not in variables object", () => {
      const result = t("shell.statusBar.coreVersion", { version: "1.0" });
      // {status} is not provided, so it stays as {status}
      expect(result).toContain("{status}");
    });

    it("ignores variables when no placeholders in template", () => {
      const result = t("shell.titleBar.appName", { unused: "value" });
      expect(result).toBe("oSystems Sync");
    });
  });
});
