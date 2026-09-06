import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Toggle } from "../Toggle";

describe("Toggle", () => {
  it("flips aria-checked and calls onChange on click", async () => {
    const user = userEvent.setup();
    const handleChange = vi.fn();
    render(<Toggle label="Modo Noturno" checked={false} onChange={handleChange} />);

    const toggle = screen.getByRole("switch", { name: "Modo Noturno" });
    expect(toggle).toHaveAttribute("aria-checked", "false");

    await user.click(toggle);
    expect(handleChange).toHaveBeenCalledWith(true);
  });

  it("flips on Space via keyboard", async () => {
    const user = userEvent.setup();
    const handleChange = vi.fn();
    render(<Toggle label="Modo Noturno" checked={false} onChange={handleChange} />);

    await user.tab();
    expect(screen.getByRole("switch")).toHaveFocus();
    await user.keyboard(" ");

    expect(handleChange).toHaveBeenCalledWith(true);
  });

  // Regression: the thumb was `absolute` with no horizontal inset, so it fell
  // back to its static position — which a <button> centers. The transforms
  // below then started from the middle of the track and pushed the thumb
  // outside it. jsdom does no layout, so what is asserted is the anchor and
  // the offsets: 2px inset off, 14px on, i.e. the 12px travel DESIGN.md §8
  // specifies for a 28×16 track with a 12px thumb.
  it("anchors the thumb to the left edge of the track in both states", () => {
    const { rerender } = render(<Toggle label="Modo Noturno" checked={false} onChange={vi.fn()} />);

    const thumbOff = screen.getByRole("switch").querySelector("span");
    expect(thumbOff).toHaveClass("absolute", "left-0", "translate-x-[2px]");

    rerender(<Toggle label="Modo Noturno" checked onChange={vi.fn()} />);

    const thumbOn = screen.getByRole("switch").querySelector("span");
    expect(thumbOn).toHaveClass("absolute", "left-0", "translate-x-[14px]");
  });
});
