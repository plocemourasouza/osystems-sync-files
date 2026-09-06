import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { App } from "../App";

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    minimize: vi.fn(),
    toggleMaximize: vi.fn(),
    close: vi.fn(),
  }),
}));

function renderApp(initialEntries: string[]) {
  render(
    <MemoryRouter initialEntries={initialEntries}>
      <App />
    </MemoryRouter>,
  );
}

describe("App", () => {
  it("redirects `/` to the Dashboard route", () => {
    renderApp(["/"]);

    expect(screen.getByRole("heading", { name: /fila de sincronização em tempo real/i })).toBeInTheDocument();
  });

  it("redirects an unknown route to the Dashboard route", () => {
    renderApp(["/nope"]);

    expect(screen.getByRole("heading", { name: /fila de sincronização em tempo real/i })).toBeInTheDocument();
  });

  it("navigates to Settings and back to Dashboard via the sidebar", async () => {
    const user = userEvent.setup();
    renderApp(["/"]);

    const settingsLink = screen.getByRole("link", { name: /configurações/i });
    await user.click(settingsLink);

    expect(screen.getByRole("heading", { name: /configurações & qos de banda/i })).toBeInTheDocument();
    expect(settingsLink).toHaveAttribute("aria-current", "page");

    const dashboardLink = screen.getByRole("link", { name: /arquivos & fila/i });
    await user.click(dashboardLink);

    expect(screen.getByRole("heading", { name: /fila de sincronização em tempo real/i })).toBeInTheDocument();
    expect(dashboardLink).toHaveAttribute("aria-current", "page");
  });

  it("renders all shell landmarks around the routed content", () => {
    renderApp(["/"]);

    // Two `<header>` landmarks exist once Dashboard is fully composed (T-2.12):
    // the shell's TitleBar and LogConsole's own collapsible header — both are
    // legitimately role="banner" per testing-library's untyped implicit-role
    // matching (it doesn't apply the HTML-AAM ancestor exclusion). Assert the
    // shell's, identified by its drag-region marker.
    expect(screen.getAllByRole("banner")[0]).toHaveAttribute("data-tauri-drag-region", "true");
    expect(screen.getByRole("navigation")).toBeInTheDocument();
    expect(screen.getByRole("main")).toHaveAttribute("id", "content");
    expect(screen.getByRole("contentinfo")).toBeInTheDocument();
  });
});
