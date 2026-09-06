import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { fireEvent } from "@testing-library/react";
import { Slider } from "../Slider";

describe("Slider", () => {
  it("shows the formatted MB/s value for a numeric value", () => {
    render(<Slider label="Google Drive" value={4} onChange={vi.fn()} />);

    expect(screen.getByText("4.0 MB/s")).toBeInTheDocument();
    expect(screen.getByLabelText("Google Drive")).toHaveAttribute("aria-valuetext", "4.0 MB/s");
  });

  it("calls onChange(null) and shows Ilimitado at the rightmost position", () => {
    const handleChange = vi.fn();
    render(<Slider label="Google Drive" value={4} onChange={handleChange} />);

    const input = screen.getByLabelText("Google Drive");
    fireEvent.change(input, { target: { value: input.getAttribute("max") } });

    expect(handleChange).toHaveBeenCalledWith(null);
  });

  it("renders Ilimitado when value is already null", () => {
    render(<Slider label="AWS S3" value={null} onChange={vi.fn()} accent="secondary" />);

    expect(screen.getAllByText("Ilimitado").length).toBeGreaterThan(0);
  });
});
