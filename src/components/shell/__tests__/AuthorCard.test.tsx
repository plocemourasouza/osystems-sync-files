import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

vi.mock("@/api/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/ipc")>();
  return { ...actual, openAuthorLink: vi.fn() };
});

import { openAuthorLink } from "@/api/ipc";
import { AuthorCard } from "@/components/shell/AuthorCard";

const mockedOpenAuthorLink = vi.mocked(openAuthorLink);

beforeEach(() => {
  mockedOpenAuthorLink.mockReset();
  mockedOpenAuthorLink.mockResolvedValue(undefined);
});

describe("AuthorCard", () => {
  it("credits the author", () => {
    render(<AuthorCard />);

    expect(screen.getByText("Criado por")).toBeInTheDocument();
    expect(screen.getByText("Paulo Souza")).toBeInTheDocument();
  });

  // The handles are the visible content; the network is what makes the row
  // make sense without seeing the icon.
  it.each([
    ["Instagram", "@plocemourasouza", "instagram"],
    ["Facebook", "@plocemourasouza", "facebook"],
    ["LinkedIn", "in/psouza", "linkedin"],
    ["GitHub", "plocemourasouza", "github"],
  ])("opens %s in the browser when its row is clicked", async (network, handle, link) => {
    const user = userEvent.setup();
    render(<AuthorCard />);

    await user.click(
      screen.getByRole("button", { name: `${network}: ${handle} — abre no navegador` }),
    );

    expect(mockedOpenAuthorLink).toHaveBeenCalledTimes(1);
    expect(mockedOpenAuthorLink).toHaveBeenCalledWith(link);
  });

  // Icons only: the sidebar is 220px and four stacked handles cost more height
  // than the folder card above them. The handle has to survive in the
  // accessible name, or the row becomes an unlabelled glyph.
  it("shows no handle text, but keeps it in the accessible name", () => {
    render(<AuthorCard />);

    expect(screen.queryByText("@plocemourasouza")).not.toBeInTheDocument();
    expect(screen.queryByText("in/psouza")).not.toBeInTheDocument();

    expect(
      screen.getByRole("button", { name: "Instagram: @plocemourasouza — abre no navegador" }),
    ).toBeInTheDocument();
  });

  it("renders exactly the four profiles", () => {
    render(<AuthorCard />);

    expect(screen.getAllByRole("button")).toHaveLength(4);
  });

  // A brand mark carries no information a screen reader needs — the row's
  // accessible name already says which network it is.
  it("hides the brand marks from assistive tech", () => {
    const { container } = render(<AuthorCard />);

    const icons = container.querySelectorAll("svg");
    expect(icons).toHaveLength(4);
    for (const icon of icons) {
      expect(icon).toHaveAttribute("aria-hidden", "true");
    }
  });

  // `<a href>` inside a Tauri webview either does nothing or navigates the app
  // away from itself; the OS handler is reached through the command instead.
  it("uses buttons, never in-app links", () => {
    const { container } = render(<AuthorCard />);

    expect(container.querySelectorAll("a")).toHaveLength(0);
  });
});
