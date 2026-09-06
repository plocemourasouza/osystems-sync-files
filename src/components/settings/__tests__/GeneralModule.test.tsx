import { beforeEach, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DEFAULT_CONFIG } from "@/api/defaults";
import { useConfigStore } from "@/store/configStore";
import { GeneralModule } from "../GeneralModule";
import { Settings } from "@/pages/Settings";

function seedStore(overrides?: Partial<ReturnType<typeof useConfigStore.getState>>): void {
  useConfigStore.setState({
    config: DEFAULT_CONFIG,
    saved: DEFAULT_CONFIG,
    dirty: false,
    issues: [],
    status: "ready",
    error: null,
    ...overrides,
  });
}

describe("GeneralModule", () => {
  beforeEach(() => {
    seedStore();
  });

  it("toggles autostart and marks the config dirty", async () => {
    const user = userEvent.setup();
    render(<GeneralModule />);

    const toggle = screen.getByRole("switch", { name: "Iniciar com o Windows" });
    expect(toggle).toHaveAttribute("aria-checked", String(DEFAULT_CONFIG.autostart));

    await user.click(toggle);

    expect(useConfigStore.getState().config.autostart).toBe(!DEFAULT_CONFIG.autostart);
    expect(useConfigStore.getState().dirty).toBe(true);
  });

  it("updates workers per destination when typing a number", async () => {
    const user = userEvent.setup();
    render(<GeneralModule />);

    const input = screen.getByLabelText("Workers por destino");
    await user.clear(input);
    await user.type(input, "3");

    expect(useConfigStore.getState().config.workers_per_destination).toBe(3);
  });

  it("parses the extensions field into a lowercase, dot-free array on blur", async () => {
    const user = userEvent.setup();
    render(<GeneralModule />);

    const input = screen.getByLabelText("Extensões permitidas");
    await user.clear(input);
    await user.type(input, "PDF, .csv");
    await user.tab();

    expect(useConfigStore.getState().config.watch.extensions).toEqual(["pdf", "csv"]);
  });

  it("renders a store validation issue as the field error", () => {
    seedStore({ issues: [{ field: "retry.max_attempts", message: "x" }] });
    render(<GeneralModule />);

    const input = screen.getByLabelText("Tentativas máximas");
    expect(input).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByRole("alert")).toHaveTextContent("x");
  });

  it("shows the read-only watched path with a note to change it via the sidebar", () => {
    seedStore({
      config: { ...DEFAULT_CONFIG, watch: { ...DEFAULT_CONFIG.watch, path: "C:\\Backups" } },
    });
    render(<GeneralModule />);

    expect(screen.getByText("C:\\Backups")).toBeInTheDocument();
    expect(screen.getByText("Altere pela sidebar.")).toBeInTheDocument();
  });
});

describe("Settings page", () => {
  beforeEach(() => {
    seedStore();
  });

  it("renders the General module below the page header", () => {
    render(<Settings />);

    expect(screen.getByRole("heading", { name: "Configurações & QoS de Banda" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Geral" })).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Iniciar com o Windows" })).toBeInTheDocument();
  });
});
