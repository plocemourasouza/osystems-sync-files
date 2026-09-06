import { describe, expect, it } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Tooltip } from "../Tooltip";

describe("Tooltip", () => {
  it("describes the child on focus without becoming its accessible name", async () => {
    render(
      <Tooltip label="Reenviar">
        <button type="button" aria-label="Reenviar" />
      </Tooltip>,
    );

    const button = screen.getByRole("button", { name: "Reenviar" });
    expect(button).not.toHaveAttribute("aria-describedby");

    button.focus();

    const tooltip = await screen.findByRole("tooltip");
    expect(tooltip).toHaveTextContent("Reenviar");
    expect(button).toHaveAttribute("aria-describedby", tooltip.id);
  });

  it("waits out the dwell delay on hover, then closes when the pointer leaves", async () => {
    const user = userEvent.setup();
    render(
      <Tooltip label="Copiar caminho">
        <button type="button" aria-label="Copiar caminho" />
      </Tooltip>,
    );

    const button = screen.getByRole("button");
    await user.hover(button);

    // Hovering is not enough on its own — the hint only appears after the dwell.
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
    await screen.findByRole("tooltip");

    await user.unhover(button);
    await waitFor(() => expect(screen.queryByRole("tooltip")).not.toBeInTheDocument());
  });

  it("dismisses on Escape but leaves focus on the child", async () => {
    const user = userEvent.setup();
    render(
      <Tooltip label="Cancelar">
        <button type="button" aria-label="Cancelar" />
      </Tooltip>,
    );

    const button = screen.getByRole("button");
    button.focus();
    await screen.findByRole("tooltip");

    await user.keyboard("{Escape}");

    await waitFor(() => expect(screen.queryByRole("tooltip")).not.toBeInTheDocument());
    expect(button).toHaveFocus();
  });

  it("renders through a portal to document.body, not inside the wrapper", async () => {
    const { container } = render(
      <Tooltip label="Abrir no Explorer">
        <button type="button" aria-label="Abrir no Explorer" />
      </Tooltip>,
    );

    screen.getByRole("button").focus();
    const tooltip = await screen.findByRole("tooltip");

    expect(container.contains(tooltip)).toBe(false);
    expect(document.body.contains(tooltip)).toBe(true);
  });
});
