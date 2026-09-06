import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { Field } from "../Field";

describe("Field", () => {
  it("renders the label and associates it with the input via htmlFor/id", () => {
    render(<Field label="Bucket" id="bucket" value="" onChange={vi.fn()} />);

    const input = screen.getByLabelText("Bucket");
    expect(input).toHaveAttribute("id", "bucket");
  });

  it("sets aria-invalid and announces the error via role=alert", () => {
    render(<Field label="Bucket" value="" onChange={vi.fn()} error="Campo obrigatório" />);

    const input = screen.getByLabelText("Bucket");
    expect(input).toHaveAttribute("aria-invalid", "true");
    expect(input).toHaveAttribute("aria-describedby");

    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("Campo obrigatório");
  });
});
