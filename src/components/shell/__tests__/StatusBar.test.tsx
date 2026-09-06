import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { healthLabel, StatusBar } from "../StatusBar";

describe("StatusBar", () => {
  it("renders core health and per-destination health + ping when latencies are present", () => {
    render(
      <StatusBar
        coreVersion="0.1.0"
        coreActive={true}
        gdrive={{ state: "online", latencyMs: 24 }}
        s3={{ state: "online", latencyMs: 41, region: "us-east-1" }}
        buildTarget="Tauri 2 • Windows x64"
      />,
    );

    expect(screen.getByText("Rust Core v0.1.0 (Active)")).toBeInTheDocument();
    expect(screen.getByText("GDrive: Online · Ping: 24 ms")).toBeInTheDocument();
    expect(screen.getByText("AWS S3: Online (us-east-1) · Ping: 41 ms")).toBeInTheDocument();
    expect(screen.getByText("Tauri 2 • Windows x64")).toBeInTheDocument();
  });

  it("omits each destination's ping segment and reflects auth-required/offline states without latencies", () => {
    render(
      <StatusBar
        coreVersion="0.1.0"
        coreActive={true}
        gdrive={{ state: "auth_required" }}
        s3={{ state: "offline" }}
        buildTarget="Tauri 2 • Windows x64"
      />,
    );

    expect(screen.getByText("GDrive: Autenticação necessária")).toBeInTheDocument();
    expect(screen.getByText("AWS S3: Offline")).toBeInTheDocument();
    expect(screen.queryByText(/Ping:/)).not.toBeInTheDocument();
  });

  it("shows only the reporting destination's ping when the other has none", () => {
    render(
      <StatusBar
        coreVersion="0.1.0"
        coreActive={true}
        gdrive={{ state: "online", latencyMs: 24 }}
        s3={{ state: "auth_required" }}
        buildTarget="Tauri 2 • Windows x64"
      />,
    );

    expect(screen.getByText("GDrive: Online · Ping: 24 ms")).toBeInTheDocument();
    expect(screen.getByText("AWS S3: Autenticação necessária")).toBeInTheDocument();
  });

  it("healthLabel maps each destination state to its display label", () => {
    expect(healthLabel("offline")).toBe("Offline");
    expect(healthLabel("online")).toBe("Online");
    expect(healthLabel("auth_required")).toBe("Autenticação necessária");
  });
});
