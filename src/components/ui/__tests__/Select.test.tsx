import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Select } from "../Select";

const options = [
  { value: "us-east-1", label: "US East (N. Virginia)" },
  { value: "us-west-2", label: "US West (Oregon)" },
];

describe("Select", () => {
  it("associates the label and fires onChange with the selected value", async () => {
    const user = userEvent.setup();
    const handleChange = vi.fn();
    render(<Select label="Região" value="us-east-1" options={options} onChange={handleChange} />);

    const select = screen.getByLabelText("Região");
    await user.selectOptions(select, "us-west-2");

    expect(handleChange).toHaveBeenCalledWith("us-west-2");
  });
});
