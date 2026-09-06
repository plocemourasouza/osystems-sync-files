import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { formatRate, Sidebar, type SidebarProps } from "../Sidebar";

const baseThroughput = {
  totalBps: 2.8e6,
  gdriveBps: 1.6e6,
  s3Bps: 1.2e6,
  capBps: 6e6,
};

function renderSidebar(overrides: Partial<SidebarProps> = {}) {
  const onChangeFolder = vi.fn();
  const props = {
    queueCount: 14,
    watchPath: "D:\\Projetos\\BackupLocal",
    watcherActive: true,
    throughput: baseThroughput,
    onChangeFolder,
    ...overrides,
  };

  render(
    <MemoryRouter initialEntries={["/dashboard"]}>
      <Sidebar {...props} />
    </MemoryRouter>,
  );

  return { onChangeFolder };
}

describe("Sidebar", () => {
  it("renders both nav links with correct hrefs and the queue badge, marking the active route", () => {
    renderSidebar();

    const dashboardLink = screen.getByRole("link", { name: /arquivos & fila/i });
    const settingsLink = screen.getByRole("link", { name: /configurações/i });

    expect(dashboardLink).toHaveAttribute("href", "/dashboard");
    expect(settingsLink).toHaveAttribute("href", "/settings");
    expect(dashboardLink).toHaveAttribute("aria-current", "page");
    expect(settingsLink).not.toHaveAttribute("aria-current");

    expect(screen.getByText("14")).toBeInTheDocument();
  });

  it("renders the watched folder path", () => {
    renderSidebar();

    expect(screen.getByText("D:\\Projetos\\BackupLocal")).toBeInTheDocument();
  });

  it("shows the active watcher badge when watcherActive is true", () => {
    renderSidebar({ watcherActive: true });
    expect(screen.getByText("Watcher: Ativo")).toBeInTheDocument();
  });

  it("shows the paused watcher badge when watcherActive is false", () => {
    render(
      <MemoryRouter initialEntries={["/dashboard"]}>
        <Sidebar
          queueCount={14}
          watchPath="D:\\Projetos\\BackupLocal"
          watcherActive={false}
          throughput={baseThroughput}
        />
      </MemoryRouter>,
    );

    expect(screen.getByText("Watcher: Pausado")).toBeInTheDocument();
  });

  it("calls onChangeFolder when clicking Alterar pasta", async () => {
    const user = userEvent.setup();
    const { onChangeFolder } = renderSidebar();

    await user.click(screen.getByRole("button", { name: "Alterar pasta" }));

    expect(onChangeFolder).toHaveBeenCalledTimes(1);
  });

  it("renders the aggregate throughput and the QoS cap", () => {
    renderSidebar();

    expect(screen.getByText("2.8 MB/s / 6.0 MB/s")).toBeInTheDocument();
  });

  it("renders Ilimitado when capBps is null", () => {
    renderSidebar({ throughput: { ...baseThroughput, capBps: null } });

    expect(screen.getByText(/Ilimitado/)).toBeInTheDocument();
  });

  it("shows the empty state and 'Escolher pasta' CTA when watchPath is null", () => {
    renderSidebar({ watchPath: null });

    expect(screen.getByText("Nenhuma pasta selecionada")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Escolher pasta" })).toBeInTheDocument();
  });
});

describe("formatRate", () => {
  it("formats sub-megabyte rates in whole KB/s", () => {
    expect(formatRate(820_000)).toBe("820 KB/s");
  });

  it("formats rates at or above 1 MB/s with one decimal", () => {
    expect(formatRate(2.8e6)).toBe("2.8 MB/s");
    expect(formatRate(6e6)).toBe("6.0 MB/s");
  });
});
