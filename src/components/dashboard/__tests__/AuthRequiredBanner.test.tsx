/**
 * AuthRequiredBanner.test.tsx — auth-required event integration & state management.
 *
 * Mocks @/api/events to capture the `useTauriEvent` handler; seeds `statusStore`
 * with auth_required states; fires events to verify hint capture and dismissal;
 * tests navigation to /settings and persist-until-next-event behavior.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";

import { useStatusStore } from "@/store/statusStore";
import type { AppStatus, AuthRequired } from "@/types/generated";
import { AuthRequiredBanner } from "../AuthRequiredBanner";

// Track the event handler so we can fire it manually in tests
let authRequiredHandler: ((payload: AuthRequired) => void) | null = null;

vi.mock("@/api/events", () => ({
  useTauriEvent: vi.fn((eventName: string, handler: (payload: AuthRequired) => void) => {
    if (eventName === "auth-required") {
      authRequiredHandler = handler;
    }
  }),
}));

const initialStatusState = useStatusStore.getState();

function status(overrides: Partial<AppStatus> = {}): AppStatus {
  return {
    watcher_paused: false,
    destinations: {
      gdrive: { online: true, auth_required: false, latency_ms: 24 },
      s3: { online: true, auth_required: false, latency_ms: 41 },
    },
    counts_by_status: {
      pending: 0,
      uploading: 0,
      paused: 0,
      cancelled: 0,
      done: 0,
      failed: 0,
      bytes_total: 0,
      bytes_done: 0,
    },
    core_version: "0.1.0",
    build_target: "Tauri 2 • Windows x64",
    ...overrides,
  };
}

beforeEach(() => {
  useStatusStore.setState(initialStatusState, true);
  authRequiredHandler = null;
});

describe("AuthRequiredBanner", () => {
  it("renders nothing when no destinations have auth_required", () => {
    useStatusStore.setState({ status: status() });

    render(
      <MemoryRouter>
        <AuthRequiredBanner />
      </MemoryRouter>
    );

    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("shows banner for gdrive when auth_required is true", () => {
    useStatusStore.setState({
      status: status({
        destinations: {
          gdrive: { online: true, auth_required: true, latency_ms: 24 },
          s3: { online: true, auth_required: false, latency_ms: 41 },
        },
      }),
    });

    render(
      <MemoryRouter>
        <AuthRequiredBanner />
      </MemoryRouter>
    );

    const alert = screen.getByRole("alert");
    expect(alert).toBeInTheDocument();
    expect(screen.getByText(/Autenticação necessária em Google Drive/i)).toBeInTheDocument();
  });

  it("shows banner for s3 when auth_required is true", () => {
    useStatusStore.setState({
      status: status({
        destinations: {
          gdrive: { online: true, auth_required: false, latency_ms: 24 },
          s3: { online: true, auth_required: true, latency_ms: 41 },
        },
      }),
    });

    render(
      <MemoryRouter>
        <AuthRequiredBanner />
      </MemoryRouter>
    );

    const alert = screen.getByRole("alert");
    expect(alert).toBeInTheDocument();
    expect(screen.getByText(/Autenticação necessária em AWS S3/i)).toBeInTheDocument();
  });

  it("shows both banners when both destinations have auth_required", () => {
    useStatusStore.setState({
      status: status({
        destinations: {
          gdrive: { online: true, auth_required: true, latency_ms: 24 },
          s3: { online: true, auth_required: true, latency_ms: 41 },
        },
      }),
    });

    render(
      <MemoryRouter>
        <AuthRequiredBanner />
      </MemoryRouter>
    );

    const alerts = screen.getAllByRole("alert");
    expect(alerts).toHaveLength(2);
    expect(screen.getByText(/Autenticação necessária em Google Drive/i)).toBeInTheDocument();
    expect(screen.getByText(/Autenticação necessária em AWS S3/i)).toBeInTheDocument();
  });

  it("displays hint from auth-required event", async () => {
    useStatusStore.setState({
      status: status({
        destinations: {
          gdrive: { online: true, auth_required: true, latency_ms: 24 },
          s3: { online: true, auth_required: false, latency_ms: 41 },
        },
      }),
    });

    render(
      <MemoryRouter>
        <AuthRequiredBanner />
      </MemoryRouter>
    );

    // Fire the auth-required event with a hint
    const hint = "share-with@gserviceaccount.com";
    authRequiredHandler?.({ destination: "gdrive", hint });

    await waitFor(() => {
      expect(screen.getByText(hint)).toBeInTheDocument();
    });
  });

  it("shows default hint when event hint is empty", async () => {
    useStatusStore.setState({
      status: status({
        destinations: {
          gdrive: { online: true, auth_required: true, latency_ms: 24 },
          s3: { online: true, auth_required: false, latency_ms: 41 },
        },
      }),
    });

    render(
      <MemoryRouter>
        <AuthRequiredBanner />
      </MemoryRouter>
    );

    // Fire the event with an empty hint
    authRequiredHandler?.({ destination: "gdrive", hint: "" });

    await waitFor(() => {
      expect(screen.getByText(/Verifique as credenciais/i)).toBeInTheDocument();
    });
  });

  it("navigates to /settings when 'Ir para Configurações' button is clicked", async () => {
    const user = userEvent.setup();

    useStatusStore.setState({
      status: status({
        destinations: {
          gdrive: { online: true, auth_required: true, latency_ms: 24 },
          s3: { online: true, auth_required: false, latency_ms: 41 },
        },
      }),
    });

    render(
      <MemoryRouter initialEntries={["/dashboard"]}>
        <Routes>
          <Route path="/dashboard" element={<AuthRequiredBanner />} />
          <Route path="/settings" element={<div>Settings Page</div>} />
        </Routes>
      </MemoryRouter>
    );

    const settingsButton = screen.getByRole("button", { name: /Navegar para Configurações para reautenticar/i });
    await user.click(settingsButton);

    await waitFor(() => {
      expect(screen.getByText("Settings Page")).toBeInTheDocument();
    });
  });

  it("dismisses banner when 'Dispensar' button is clicked", async () => {
    const user = userEvent.setup();

    useStatusStore.setState({
      status: status({
        destinations: {
          gdrive: { online: true, auth_required: true, latency_ms: 24 },
          s3: { online: true, auth_required: false, latency_ms: 41 },
        },
      }),
    });

    render(
      <MemoryRouter>
        <AuthRequiredBanner />
      </MemoryRouter>
    );

    expect(screen.getByRole("alert")).toBeInTheDocument();

    const dismissButton = screen.getByRole("button", { name: /Dispensar/i });
    await user.click(dismissButton);

    await waitFor(() => {
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });
  });

  it("resets dismissal when auth-required event fires for dismissed destination", async () => {
    const user = userEvent.setup();

    useStatusStore.setState({
      status: status({
        destinations: {
          gdrive: { online: true, auth_required: true, latency_ms: 24 },
          s3: { online: true, auth_required: false, latency_ms: 41 },
        },
      }),
    });

    render(
      <MemoryRouter>
        <AuthRequiredBanner />
      </MemoryRouter>
    );

    // Dismiss the banner
    const dismissButton = screen.getByRole("button", { name: /Dispensar/i });
    await user.click(dismissButton);

    await waitFor(() => {
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });

    // Fire the event again — banner should reappear
    authRequiredHandler?.({ destination: "gdrive", hint: "New hint from event" });

    await waitFor(() => {
      expect(screen.getByRole("alert")).toBeInTheDocument();
      expect(screen.getByText("New hint from event")).toBeInTheDocument();
    });
  });

  it("handles multiple destinations being dismissed independently", async () => {
    const user = userEvent.setup();

    useStatusStore.setState({
      status: status({
        destinations: {
          gdrive: { online: true, auth_required: true, latency_ms: 24 },
          s3: { online: true, auth_required: true, latency_ms: 41 },
        },
      }),
    });

    render(
      <MemoryRouter>
        <AuthRequiredBanner />
      </MemoryRouter>
    );

    const dismissButtons = screen.getAllByRole("button", { name: /Dispensar/i });
    expect(dismissButtons).toHaveLength(2);

    // Dismiss only the first one (gdrive)
    await user.click(dismissButtons[0]!);

    await waitFor(() => {
      const alerts = screen.queryAllByRole("alert");
      expect(alerts).toHaveLength(1);
      expect(screen.getByText(/Autenticação necessária em AWS S3/i)).toBeInTheDocument();
    });
  });

  it("displays hint from event even when starting with empty hints", async () => {
    useStatusStore.setState({
      status: status({
        destinations: {
          gdrive: { online: true, auth_required: true, latency_ms: 24 },
          s3: { online: true, auth_required: false, latency_ms: 41 },
        },
      }),
    });

    render(
      <MemoryRouter>
        <AuthRequiredBanner />
      </MemoryRouter>
    );

    // Should show default hint initially
    expect(screen.getByText(/Verifique as credenciais/i)).toBeInTheDocument();

    // Fire event with hint
    const emailHint = "sa-gdrive@myproject.iam.gserviceaccount.com";
    authRequiredHandler?.({ destination: "gdrive", hint: emailHint });

    await waitFor(() => {
      expect(screen.getByText(emailHint)).toBeInTheDocument();
      expect(screen.queryByText(/Verifique as credenciais/i)).not.toBeInTheDocument();
    });
  });

  it("uses error tone styling (role alert, error colors, icons)", () => {
    useStatusStore.setState({
      status: status({
        destinations: {
          gdrive: { online: true, auth_required: true, latency_ms: 24 },
          s3: { online: true, auth_required: false, latency_ms: 41 },
        },
      }),
    });

    render(
      <MemoryRouter>
        <AuthRequiredBanner />
      </MemoryRouter>
    );

    const alert = screen.getByRole("alert");
    expect(alert).toHaveClass("bg-status-error-bg", "border-error", "text-error");

    // AlertTriangle icon should be present (SVG with AlertTriangle)
    const icon = alert.querySelector("svg");
    expect(icon).not.toBeNull();
  });
});
