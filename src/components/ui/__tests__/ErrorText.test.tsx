import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { ErrorText } from "../ErrorText";

describe("ErrorText", () => {
  it("renders the message with role=alert", () => {
    render(<ErrorText>Something went wrong</ErrorText>);

    const alert = screen.getByRole("alert");
    expect(alert).toBeInTheDocument();
    expect(alert).toHaveTextContent("Something went wrong");
  });

  it("keeps the message text AA-safe (text-text-primary), carrying the error tone only on the icon", () => {
    render(<ErrorText>Something went wrong</ErrorText>);

    const alert = screen.getByRole("alert");
    expect(alert).toHaveClass("text-text-primary");
    expect(alert).not.toHaveClass("text-error");

    const icon = alert.querySelector("svg");
    expect(icon).not.toBeNull();
    expect(icon).toHaveClass("text-error");
  });

  it("applies the given id, for aria-describedby wiring", () => {
    render(<ErrorText id="field-error">Required</ErrorText>);

    expect(screen.getByRole("alert")).toHaveAttribute("id", "field-error");
  });
});
