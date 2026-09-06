import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { NumberField } from "../NumberField";

function Controlled({ onChange }: { onChange: (value: number) => void }) {
  return <NumberField label="Limite" value={5} min={0} max={10} onChange={onChange} />;
}

describe("NumberField", () => {
  it("clamps an out-of-range value to max on blur and calls onChange with a number", async () => {
    const user = userEvent.setup();
    const handleChange = vi.fn();
    render(<Controlled onChange={handleChange} />);

    const input = screen.getByLabelText("Limite");
    await user.clear(input);
    await user.type(input, "999");
    await user.tab();

    expect(handleChange).toHaveBeenLastCalledWith(10);
    expect(input).toHaveValue(10);
  });

  // The engine's own spinner rendered black on Windows and only on hover on
  // macOS — matching neither each other nor the `Select` chevron beside it. It
  // is hidden in `app.css` and replaced by these, so they have to actually
  // step the value.
  describe("stepper", () => {
    /** `[up, down]`, asserted present so `noUncheckedIndexedAccess` is happy. */
    function steppers(container: HTMLElement): [HTMLButtonElement, HTMLButtonElement] {
      const buttons = [...container.querySelectorAll("button")];
      expect(buttons).toHaveLength(2);
      return [buttons[0] as HTMLButtonElement, buttons[1] as HTMLButtonElement];
    }

    // Stateful, like every real caller (the settings fields are backed by
    // `configStore`): a stepper that only works while the parent ignores it
    // would be worthless.
    function Stateful({ initial = 5 }: { initial?: number }) {
      const [value, setValue] = useState(initial);
      return (
        <>
          <NumberField label="Limite" value={value} min={0} max={10} onChange={setValue} />
          <output>{value}</output>
        </>
      );
    }

    it("steps the value up and down by `step`", async () => {
      const user = userEvent.setup();
      const { container } = render(<Stateful />);
      const [up, down] = steppers(container);

      await user.click(up);
      expect(screen.getByRole("status")).toHaveTextContent("6");

      await user.click(down);
      await user.click(down);
      expect(screen.getByRole("status")).toHaveTextContent("4");
      expect(screen.getByLabelText("Limite")).toHaveValue(4);
    });

    it("respects a custom step", async () => {
      const user = userEvent.setup();
      const handleChange = vi.fn();
      const { container } = render(
        <NumberField label="Limite" value={10} step={5} max={100} onChange={handleChange} />,
      );

      await user.click(steppers(container)[0]);

      expect(handleChange).toHaveBeenLastCalledWith(15);
    });

    it("disables the button that would cross a bound", () => {
      const { container } = render(
        <NumberField label="Limite" value={10} min={0} max={10} onChange={vi.fn()} />,
      );
      const [up, down] = steppers(container);

      expect(up).toBeDisabled();
      expect(down).toBeEnabled();
    });

    // A pointer affordance for something the input already exposes: ↑/↓ step
    // it natively and it is a `spinbutton` with min/max. Two extra tab stops
    // per field announcing nothing new would be noise.
    it("stays out of the tab order and out of the accessibility tree", () => {
      const { container } = render(<Controlled onChange={vi.fn()} />);

      for (const button of steppers(container)) {
        expect(button).toHaveAttribute("tabindex", "-1");
        expect(button.closest('[aria-hidden="true"]')).not.toBeNull();
      }
      expect(screen.queryAllByRole("button")).toHaveLength(0);
    });
  });

  it("shows an error while the typed draft is out of range", async () => {
    const user = userEvent.setup();
    render(<Controlled onChange={vi.fn()} />);

    const input = screen.getByLabelText("Limite");
    await user.clear(input);
    await user.type(input, "999");

    expect(input).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });
});
