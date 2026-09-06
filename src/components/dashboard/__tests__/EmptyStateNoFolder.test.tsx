import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { EmptyStateNoFolder } from "../EmptyStateNoFolder";
import * as statusStore from "@/store/statusStore";
import type { StatusStore } from "@/store/statusStore";

// Mock the store
vi.mock("@/store/statusStore", () => ({
  useStatusStore: vi.fn(),
}));

describe("EmptyStateNoFolder", () => {
  let mockPickFolder: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    mockPickFolder = vi.fn();
    vi.mocked(statusStore.useStatusStore).mockImplementation(
      (selector) =>
        selector({
          status: null,
          loading: false,
          error: null,
          refresh: vi.fn(),
          applyEvent: vi.fn(),
          actions: {
            rescan: vi.fn(),
            pauseWatcher: vi.fn(),
            resumeWatcher: vi.fn(),
            pickFolder: mockPickFolder,
          },
        } as StatusStore)
    );
  });

  it("renders title and CTA button", () => {
    render(<EmptyStateNoFolder />);

    expect(screen.getByRole("heading", { level: 2 })).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /escolher pasta/i })
    ).toBeInTheDocument();
  });

  it("calls pickFolder when button is clicked", async () => {
    const user = userEvent.setup();
    mockPickFolder.mockResolvedValue("/path/to/folder");

    render(<EmptyStateNoFolder />);
    const button = screen.getByRole("button", { name: /escolher pasta/i });

    await user.click(button);

    expect(mockPickFolder).toHaveBeenCalledTimes(1);
  });

  it("shows loading state while pickFolder is pending", async () => {
    const user = userEvent.setup();
    let resolvePickFolder: (value: string | null) => void;
    const pickFolderPromise = new Promise<string | null>((resolve) => {
      resolvePickFolder = resolve;
    });
    mockPickFolder.mockReturnValue(pickFolderPromise);

    render(<EmptyStateNoFolder />);
    const button = screen.getByRole("button", { name: /escolher pasta/i });

    await user.click(button);

    // Button should be disabled while loading
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute("aria-busy", "true");

    // Resolve with null (user cancelled) — button becomes enabled again
    resolvePickFolder!(null);
    await waitFor(() => {
      expect(button).not.toBeDisabled();
    });
  });

  it("shows error alert on rejection", async () => {
    const user = userEvent.setup();
    const errorMessage = "Permission denied";
    mockPickFolder.mockRejectedValue(new Error(errorMessage));

    render(<EmptyStateNoFolder />);
    const button = screen.getByRole("button", { name: /escolher pasta/i });

    await user.click(button);

    await waitFor(() => {
      expect(screen.getByRole("alert")).toBeInTheDocument();
    });

    expect(button).not.toBeDisabled();
  });

  it("does nothing when pickFolder returns null (cancelled)", async () => {
    const user = userEvent.setup();
    mockPickFolder.mockResolvedValue(null);

    render(<EmptyStateNoFolder />);
    const button = screen.getByRole("button", { name: /escolher pasta/i });

    await user.click(button);

    await waitFor(() => {
      expect(button).not.toBeDisabled();
    });

    // No error should be shown
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});
