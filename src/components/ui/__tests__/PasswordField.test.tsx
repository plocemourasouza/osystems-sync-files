import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PasswordField } from "../PasswordField";

describe("PasswordField", () => {
  it("toggles the input type between password and text", async () => {
    const user = userEvent.setup();
    render(<PasswordField label="Secret Access Key" value="s3cr3t" onChange={vi.fn()} />);

    const input = screen.getByLabelText("Secret Access Key");
    expect(input).toHaveAttribute("type", "password");

    const toggle = screen.getByRole("button", { name: "Mostrar" });
    await user.click(toggle);

    expect(input).toHaveAttribute("type", "text");
    expect(screen.getByRole("button", { name: "Ocultar" })).toHaveAttribute("aria-pressed", "true");
  });
});
