import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TitleBar } from "../TitleBar";

const { minimize, toggleMaximize, close } = vi.hoisted(() => ({
  minimize: vi.fn(),
  toggleMaximize: vi.fn(),
  close: vi.fn(),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ minimize, toggleMaximize, close }),
}));

describe("TitleBar", () => {
  beforeEach(() => {
    minimize.mockClear();
    toggleMaximize.mockClear();
    close.mockClear();
  });

  it("renders app name, watched path and daemon badge, and drags only from non-interactive areas", async () => {
    const user = userEvent.setup();
    const watchPath = "D:\\Projetos\\BackupLocal";
    render(<TitleBar watchPath={watchPath} daemonActive />);

    expect(screen.getByText("oSystems Sync")).toBeInTheDocument();
    expect(screen.getByText(watchPath)).toBeInTheDocument();
    expect(screen.getByText("Daemon: Active")).toBeInTheDocument();

    const header = screen.getByRole("banner");
    expect(header).toHaveAttribute("data-tauri-drag-region");

    const minimizeButton = screen.getByRole("button", { name: "Minimizar" });
    const maximizeButton = screen.getByRole("button", { name: "Maximizar" });
    const closeButton = screen.getByRole("button", { name: "Fechar" });

    for (const button of [minimizeButton, maximizeButton, closeButton]) {
      expect(button).not.toHaveAttribute("data-tauri-drag-region");
    }

    await user.click(minimizeButton);
    expect(minimize).toHaveBeenCalledTimes(1);
    expect(toggleMaximize).not.toHaveBeenCalled();
    expect(close).not.toHaveBeenCalled();

    await user.click(maximizeButton);
    expect(toggleMaximize).toHaveBeenCalledTimes(1);

    await user.click(closeButton);
    expect(close).toHaveBeenCalledTimes(1);
  });

  it("shows a placeholder and the inactive badge when there is no watched folder", () => {
    render(<TitleBar watchPath={null} daemonActive={false} />);

    expect(screen.getByText("Nenhuma pasta")).toBeInTheDocument();
    expect(screen.getByText("Daemon: Inativo")).toBeInTheDocument();
  });
});
