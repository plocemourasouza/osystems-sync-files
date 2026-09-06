/**
 * QosModule — Settings › "Controle de Vazão e QoS de Largura de Banda"
 * (PLAN.md T-3.12/T-4.9; PRD.md RF-050 a RF-053, RF-082; design/DESIGN.md §8
 * "Slider (QoS)"/"Toggle", §9 `/settings`).
 *
 * Two `Slider`s bound to `configStore.config.qos.{gdrive_limit_mbps,
 * s3_limit_mbps}` (RF-050: 0.5–10 MB/s in 0.5 steps + "Ilimitado" at the
 * rightmost position). Every `onChange` (fired on every drag tick, per the
 * native `<input type="range">` `input` event) does two things:
 *  1. patches the draft config immediately via `configStore.setQos` so the
 *     slider position/value badge and Ctrl+S's normal save flow stay in
 *     sync — this is NOT the "commit" RF-051 talks about;
 *  2. (re)schedules a 150 ms-debounced call to the `set_qos` IPC command —
 *     the actual "on release" commit RF-051 requires, applied at runtime in
 *     ≤ 2 s without hammering the backend on every tick.
 *
 * `set_qos` already persists `config.json` on the Rust side (T-3.9), so a
 * successful call also folds the whole `config.qos` draft into `saved.qos`
 * via the additive `markQosSaved()` store action — otherwise the footer
 * would keep showing "Alterações não salvas" for a value already on disk.
 * A rejected call surfaces `tError(e.code)` next to that slider instead,
 * and never touches `saved`.
 *
 * Modo Noturno (RF-053/RF-082) toggles `config.qos.night_mode.enabled` and
 * its `start`/`end` `HH:MM` window via `configStore.setQos` — like Folder
 * ID/bucket, these persist through the normal draft/dirty + Ctrl+S save
 * cycle (T-3.9's `set_config`), NOT the sliders' debounced-immediate
 * `set_qos` commit. `start`/`end` are validated locally against
 * `^\d{2}:\d{2}$` with an inline `Field` error; both inputs disable
 * whenever the toggle is off.
 */
import { Check, Gauge } from "lucide-react";
import { useEffect, useRef, useState, type JSX } from "react";
import { isAppError, setQos as setQosRemote } from "@/api/ipc";
import { ErrorText, Field, Module, Slider, Toggle } from "@/components/ui";
import { t, tError } from "@/i18n";
import { useConfigStore } from "@/store/configStore";
import type { Destination } from "@/types/generated";

const APPLY_DEBOUNCE_MS = 150;
const APPLIED_NOTICE_MS = 2000;
const TIME_RE = /^\d{2}:\d{2}$/;

type DestinationTimers = Record<Destination, ReturnType<typeof setTimeout> | null>;

function clearTimer(timers: DestinationTimers, destination: Destination): void {
  const existing = timers[destination];
  if (existing) clearTimeout(existing);
}

export function QosModule(): JSX.Element {
  const qos = useConfigStore((state) => state.config.qos);
  const setQos = useConfigStore((state) => state.setQos);
  const markQosSaved = useConfigStore((state) => state.markQosSaved);

  const [applied, setApplied] = useState<Record<Destination, boolean>>({
    gdrive: false,
    s3: false,
  });
  const [errors, setErrors] = useState<Record<Destination, string | null>>({
    gdrive: null,
    s3: null,
  });

  const debounceTimers = useRef<DestinationTimers>({ gdrive: null, s3: null });
  const appliedTimers = useRef<DestinationTimers>({ gdrive: null, s3: null });

  // Cancel any pending debounce/notice timers on unmount so they never fire
  // (and call `setState`) after the module is gone.
  useEffect(() => {
    const debounceSnapshot = debounceTimers.current;
    const appliedSnapshot = appliedTimers.current;
    return () => {
      clearTimer(debounceSnapshot, "gdrive");
      clearTimer(debounceSnapshot, "s3");
      clearTimer(appliedSnapshot, "gdrive");
      clearTimer(appliedSnapshot, "s3");
    };
  }, []);

  async function applyLimit(destination: Destination, value: number | null): Promise<void> {
    try {
      await setQosRemote(destination, value);
      markQosSaved();
      setErrors((prev) => ({ ...prev, [destination]: null }));
      setApplied((prev) => ({ ...prev, [destination]: true }));

      clearTimer(appliedTimers.current, destination);
      appliedTimers.current[destination] = setTimeout(() => {
        setApplied((prev) => ({ ...prev, [destination]: false }));
      }, APPLIED_NOTICE_MS);
    } catch (e) {
      setErrors((prev) => ({
        ...prev,
        [destination]: isAppError(e) ? tError(e.code) : tError("unknown"),
      }));
    }
  }

  function handleChange(destination: Destination, value: number | null): void {
    setQos(destination === "gdrive" ? { gdrive_limit_mbps: value } : { s3_limit_mbps: value });

    clearTimer(debounceTimers.current, destination);
    debounceTimers.current[destination] = setTimeout(() => {
      void applyLimit(destination, value);
    }, APPLY_DEBOUNCE_MS);
  }

  return (
    <Module
      className="min-w-0"
      icon={<Gauge aria-hidden="true" size={16} />}
      title={t("pages.settings.qos.title")}
      subtitle={t("pages.settings.qos.subtitle")}
      status={
        <Toggle
          checked={qos.night_mode.enabled}
          onChange={(checked) => setQos({ night_mode: { ...qos.night_mode, enabled: checked } })}
          label={t("pages.settings.qos.nightMode.label")}
        />
      }
    >
      <div className="flex flex-wrap items-center gap-xs px-md py-sm">
        <Field
          label={t("pages.settings.qos.nightMode.startLabel")}
          mono
          value={qos.night_mode.start}
          onChange={(event) =>
            setQos({
              night_mode: { ...qos.night_mode, start: event.target.value },
            })
          }
          error={
            qos.night_mode.enabled && !TIME_RE.test(qos.night_mode.start)
              ? t("pages.settings.qos.nightMode.timeInvalid")
              : undefined
          }
          disabled={!qos.night_mode.enabled}
          className="w-20"
        />
        <span aria-hidden="true" className="pt-lg text-text-quaternary">
          –
        </span>
        <Field
          label={t("pages.settings.qos.nightMode.endLabel")}
          mono
          value={qos.night_mode.end}
          onChange={(event) =>
            setQos({
              night_mode: { ...qos.night_mode, end: event.target.value },
            })
          }
          error={
            qos.night_mode.enabled && !TIME_RE.test(qos.night_mode.end)
              ? t("pages.settings.qos.nightMode.timeInvalid")
              : undefined
          }
          disabled={!qos.night_mode.enabled}
          className="w-20"
        />
      </div>

      <div className="grid grid-cols-1 gap-lg px-md py-md lg:grid-cols-2">
        <div className="flex flex-col gap-xs">
          <Slider
            label={t("pages.settings.qos.gdrive.label")}
            value={qos.gdrive_limit_mbps}
            onChange={(value) => handleChange("gdrive", value)}
            accent="primary"
          />
          <QosStatus applied={applied.gdrive} error={errors.gdrive} />
        </div>
        <div className="flex flex-col gap-xs">
          <Slider
            label={t("pages.settings.qos.s3.label")}
            value={qos.s3_limit_mbps}
            onChange={(value) => handleChange("s3", value)}
            accent="secondary"
          />
          <QosStatus applied={applied.s3} error={errors.s3} />
        </div>
      </div>
    </Module>
  );
}

function QosStatus({ applied, error }: { applied: boolean; error: string | null }): JSX.Element | null {
  if (error) {
    return <ErrorText>{error}</ErrorText>;
  }

  if (applied) {
    return (
      <p role="status" className="flex items-center gap-2xs text-label-sm text-tertiary">
        <Check aria-hidden="true" size={12} />
        {t("pages.settings.qos.applied")}
      </p>
    );
  }

  return null;
}
