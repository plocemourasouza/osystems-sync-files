import { describe, expect, it } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";

import { EmptyStateNoCredentials } from "../EmptyStateNoCredentials";

describe("EmptyStateNoCredentials", () => {
  it("renders a heading, body and 'Configurar' CTA", () => {
    render(
      <MemoryRouter>
        <EmptyStateNoCredentials />
      </MemoryRouter>
    );

    expect(screen.getByRole("heading", { level: 2 })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Configurar" })).toBeInTheDocument();
  });

  it("navigates to /settings when the CTA is clicked", async () => {
    const user = userEvent.setup();

    render(
      <MemoryRouter initialEntries={["/dashboard"]}>
        <Routes>
          <Route path="/dashboard" element={<EmptyStateNoCredentials />} />
          <Route path="/settings" element={<div>Settings Page</div>} />
        </Routes>
      </MemoryRouter>
    );

    await user.click(screen.getByRole("button", { name: "Configurar" }));

    await waitFor(() => {
      expect(screen.getByText("Settings Page")).toBeInTheDocument();
    });
  });
});
