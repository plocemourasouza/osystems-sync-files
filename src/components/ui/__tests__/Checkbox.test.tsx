import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Checkbox } from "../Checkbox";

describe("Checkbox", () => {
  it("associates the label via htmlFor/id and calls onChange when clicked", async () => {
    const user = userEvent.setup();
    const handleChange = vi.fn();
    render(<Checkbox label="Checksum MD5 pré-upload" checked={false} onChange={handleChange} id="md5" />);

    const checkbox = screen.getByLabelText("Checksum MD5 pré-upload");
    expect(checkbox).toHaveAttribute("id", "md5");

    await user.click(checkbox);
    expect(handleChange).toHaveBeenCalledWith(true);
  });
});
