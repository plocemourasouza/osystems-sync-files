import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

vi.mock("@/api/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/ipc")>();
  return {
    ...actual,
    setQos: vi.fn(),
  };
});

import { setQos as setQosRemote, type AppError } from "@/api/ipc";
import { DEFAULT_CONFIG } from "@/api/defaults";
import { useConfigStore } from "@/store/configStore";
import type { QosConfig } from "@/types/generated";

import { QosModule } from "../QosModule";

const mockedSetQosRemote = vi.mocked(setQosRemote);

const initialState = useConfigStore.getState();

function seedQos(overrides: Partial<QosConfig> = {}): void {
  const config = { ...DEFAULT_CONFIG, qos: { ...DEFAULT_CONFIG.qos, ...overrides } };
  useConfigStore.setState({
    config,
    saved: config,
    dirty: false,
    issues: [],
    status: "ready",
    error: null,
  });
}

beforeEach(() => {
  useConfigStore.setState(initialState, true);
  mockedSetQosRemote.mockReset().mockResolvedValue(undefined);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("QosModule", () => {
  it("renders both sliders bound to the current config values", () => {
    seedQos({ gdrive_limit_mbps: 3.5, s3_limit_mbps: null });
    render(<QosModule />);

    expect(screen.getByLabelText("Teto Google Drive")).toHaveAttribute("aria-valuetext", "3.5 MB/s");
    expect(screen.getByLabelText("Teto AWS S3")).toHaveAttribute("aria-valuetext", "Ilimitado");
    expect(screen.getByText("3.5 MB/s")).toBeInTheDocument();
    expect(screen.getAllByText("Ilimitado").length).toBeGreaterThan(0);
  });

  it("moving the S3 slider patches the store, debounces the IPC call, and shows Aplicado on success", async () => {
    vi.useFakeTimers();
    seedQos({ gdrive_limit_mbps: 3.5, s3_limit_mbps: 5 });
    render(<QosModule />);

    const s3Slider = screen.getByLabelText("Teto AWS S3");
    // Index 4 == (2.5 - 0.5) / 0.5 -> the 2.5 MB/s position.
    fireEvent.change(s3Slider, { target: { value: "4" } });

    expect(useConfigStore.getState().config.qos.s3_limit_mbps).toBe(2.5);
    expect(mockedSetQosRemote).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(150);

    expect(mockedSetQosRemote).toHaveBeenCalledTimes(1);
    expect(mockedSetQosRemote).toHaveBeenCalledWith("s3", 2.5);

    await vi.waitFor(() => {
      expect(screen.getAllByText("Aplicado")).toHaveLength(1);
    });
    expect(useConfigStore.getState().saved.qos.s3_limit_mbps).toBe(2.5);
  });

  it("only fires one debounced IPC call for a burst of drag ticks", async () => {
    vi.useFakeTimers();
    seedQos({ gdrive_limit_mbps: 3.5, s3_limit_mbps: 5 });
    render(<QosModule />);

    const s3Slider = screen.getByLabelText("Teto AWS S3");
    fireEvent.change(s3Slider, { target: { value: "3" } });
    await vi.advanceTimersByTimeAsync(50);
    fireEvent.change(s3Slider, { target: { value: "4" } });
    await vi.advanceTimersByTimeAsync(50);
    fireEvent.change(s3Slider, { target: { value: "5" } });

    await vi.advanceTimersByTimeAsync(150);

    expect(mockedSetQosRemote).toHaveBeenCalledTimes(1);
    expect(mockedSetQosRemote).toHaveBeenCalledWith("s3", 3);
  });

  it("moving the Drive slider to the rightmost position commits null (Ilimitado)", async () => {
    vi.useFakeTimers();
    seedQos({ gdrive_limit_mbps: 3.5, s3_limit_mbps: 5 });
    render(<QosModule />);

    const gdriveSlider = screen.getByLabelText("Teto Google Drive");
    fireEvent.change(gdriveSlider, { target: { value: gdriveSlider.getAttribute("max") } });

    expect(useConfigStore.getState().config.qos.gdrive_limit_mbps).toBeNull();

    await vi.advanceTimersByTimeAsync(150);

    expect(mockedSetQosRemote).toHaveBeenCalledWith("gdrive", null);
  });

  it("shows the error text next to the slider when the IPC call rejects", async () => {
    vi.useFakeTimers();
    const rejection: AppError = { code: "qos.invalid", message: "boom" };
    mockedSetQosRemote.mockRejectedValueOnce(rejection);
    seedQos({ gdrive_limit_mbps: 3.5, s3_limit_mbps: 5 });
    render(<QosModule />);

    const s3Slider = screen.getByLabelText("Teto AWS S3");
    fireEvent.change(s3Slider, { target: { value: "4" } });

    await vi.advanceTimersByTimeAsync(150);
    await vi.waitFor(() => {
      expect(screen.getByRole("alert")).toBeInTheDocument();
    });

    expect(useConfigStore.getState().saved.qos.s3_limit_mbps).not.toBe(2.5);
    expect(screen.queryByText("Aplicado")).not.toBeInTheDocument();
  });

  describe("night mode (RF-053/RF-082)", () => {
    it("renders the toggle enabled, reflecting config.qos.night_mode.enabled", () => {
      seedQos({ night_mode: { enabled: true, start: "23:00", end: "06:00" } });
      render(<QosModule />);

      const toggle = screen.getByRole("switch", { name: "Modo Noturno (Sem Limites)" });
      expect(toggle).not.toBeDisabled();
      expect(toggle).toHaveAttribute("aria-checked", "true");
    });

    it("clicking the toggle patches config.qos.night_mode.enabled without touching start/end", async () => {
      const user = userEvent.setup();
      seedQos({ night_mode: { enabled: false, start: "23:00", end: "06:00" } });
      render(<QosModule />);

      await user.click(screen.getByRole("switch", { name: "Modo Noturno (Sem Limites)" }));

      expect(useConfigStore.getState().config.qos.night_mode).toEqual({
        enabled: true,
        start: "23:00",
        end: "06:00",
      });
      expect(mockedSetQosRemote).not.toHaveBeenCalled();
    });

    it("renders the start/end fields with the current HH:MM values, disabled while off", () => {
      seedQos({ night_mode: { enabled: false, start: "23:00", end: "06:00" } });
      render(<QosModule />);

      const start = screen.getByLabelText("Início");
      const end = screen.getByLabelText("Fim");
      expect(start).toHaveValue("23:00");
      expect(end).toHaveValue("06:00");
      expect(start).toBeDisabled();
      expect(end).toBeDisabled();
    });

    it("typing a valid HH:MM into start updates config.qos.night_mode.start", async () => {
      const user = userEvent.setup();
      seedQos({ night_mode: { enabled: true, start: "23:00", end: "06:00" } });
      render(<QosModule />);

      const start = screen.getByLabelText("Início");
      await user.clear(start);
      await user.type(start, "22:30");

      expect(useConfigStore.getState().config.qos.night_mode.start).toBe("22:30");
      expect(mockedSetQosRemote).not.toHaveBeenCalled();
    });

    it("shows an inline error under end when its value doesn't match HH:MM while enabled", async () => {
      const user = userEvent.setup();
      seedQos({ night_mode: { enabled: true, start: "23:00", end: "06:00" } });
      render(<QosModule />);

      const end = screen.getByLabelText("Fim");
      await user.clear(end);
      await user.type(end, "6h");

      expect(screen.getByText("Use o formato HH:MM")).toBeInTheDocument();
      expect(useConfigStore.getState().config.qos.night_mode.end).toBe("6h");
    });
  });

  it("markQosSaved folds only the qos slice into saved, leaving other dirty edits intact", () => {
    seedQos({ gdrive_limit_mbps: 1, s3_limit_mbps: 1 });
    useConfigStore.getState().setS3({ bucket: "unsaved-bucket" });
    useConfigStore.getState().setQos({ gdrive_limit_mbps: 2 });
    expect(useConfigStore.getState().dirty).toBe(true);

    useConfigStore.getState().markQosSaved();

    const state = useConfigStore.getState();
    expect(state.saved.qos.gdrive_limit_mbps).toBe(2);
    expect(state.saved.s3.bucket).not.toBe("unsaved-bucket");
    expect(state.dirty).toBe(true);
    expect(state.config.s3.bucket).toBe("unsaved-bucket");
  });
});
