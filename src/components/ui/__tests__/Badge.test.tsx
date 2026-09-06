import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { Badge } from "../Badge";

describe("Badge", () => {
  it("applies the tone's text/background classes", () => {
    render(<Badge tone="error">Falha</Badge>);

    const badge = screen.getByText("Falha");
    expect(badge).toHaveClass("text-error");
    expect(badge).toHaveClass("bg-status-error-bg");
  });

  it("defaults to the neutral tone", () => {
    render(<Badge>Na Fila</Badge>);

    expect(screen.getByText("Na Fila")).toHaveClass("bg-surface-2");
  });
});
