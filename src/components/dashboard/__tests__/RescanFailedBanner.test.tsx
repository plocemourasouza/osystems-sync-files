/**
 * RescanFailedBanner.test.tsx — `rescan-failed` event integration.
 *
 * Mocks @/api/events to capture the `useTauriEvent` handler (mirrors
 * AuthRequiredBanner.test.tsx); fires the event to verify the message renders,
 * dismissal hides it, and a later event reinstates it (even for a dismissed
 * banner) with the new message.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { RescanFailed } from "@/types/generated";
import { RescanFailedBanner } from "../RescanFailedBanner";

let rescanFailedHandler: ((payload: RescanFailed) => void) | null = null;

vi.mock("@/api/events", () => ({
  useTauriEvent: vi.fn((eventName: string, handler: (payload: RescanFailed) => void) => {
    if (eventName === "rescan-failed") {
      rescanFailedHandler = handler;
    }
  }),
}));

beforeEach(() => {
  rescanFailedHandler = null;
});

describe("RescanFailedBanner", () => {
  it("renders nothing before any rescan-failed event fires", () => {
    render(<RescanFailedBanner />);

    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("shows the message from a rescan-failed event", async () => {
    render(<RescanFailedBanner />);

    rescanFailedHandler?.({ message: "io error while scanning: Acesso negado. (os error 5)" });

    await waitFor(() => {
      expect(screen.getByRole("alert")).toBeInTheDocument();
      expect(screen.getByText("io error while scanning: Acesso negado. (os error 5)")).toBeInTheDocument();
    });
  });

  it("dismisses the banner when 'Dispensar' is clicked", async () => {
    const user = userEvent.setup();
    render(<RescanFailedBanner />);

    rescanFailedHandler?.({ message: "state.db error: locked" });
    await screen.findByRole("alert");

    await user.click(screen.getByRole("button", { name: /dispensar/i }));

    await waitFor(() => {
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });
  });

  it("reinstates the banner with a new message when another event fires after dismissal", async () => {
    const user = userEvent.setup();
    render(<RescanFailedBanner />);

    rescanFailedHandler?.({ message: "first failure" });
    await screen.findByRole("alert");
    await user.click(screen.getByRole("button", { name: /dispensar/i }));
    await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());

    rescanFailedHandler?.({ message: "second failure" });

    await waitFor(() => {
      expect(screen.getByRole("alert")).toBeInTheDocument();
      expect(screen.getByText("second failure")).toBeInTheDocument();
    });
  });

  it("uses error tone styling (role alert, error colors, icon)", async () => {
    render(<RescanFailedBanner />);

    rescanFailedHandler?.({ message: "io error" });
    const alert = await screen.findByRole("alert");

    expect(alert).toHaveClass("bg-status-error-bg", "border-error", "text-error");
    expect(alert.querySelector("svg")).not.toBeNull();
  });
});
