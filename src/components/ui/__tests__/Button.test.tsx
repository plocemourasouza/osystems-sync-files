import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Button } from "../Button";

describe("Button", () => {
  it("fires onClick when enabled", async () => {
    const user = userEvent.setup();
    const handleClick = vi.fn();
    render(<Button onClick={handleClick}>Salvar</Button>);

    await user.click(screen.getByRole("button", { name: "Salvar" }));
    expect(handleClick).toHaveBeenCalledTimes(1);
  });

  it("is disabled and shows a spinner while loading", () => {
    render(<Button loading>Salvar</Button>);

    const button = screen.getByRole("button", { name: "Salvar" });
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute("aria-busy", "true");
  });

  it("defaults to type=button so it never submits a form by accident", () => {
    render(<Button>Salvar</Button>);
    expect(screen.getByRole("button", { name: "Salvar" })).toHaveAttribute("type", "button");
  });
});
