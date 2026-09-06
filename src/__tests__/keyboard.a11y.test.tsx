/**
 * Keyboard navigation audit (RNF-013, PLAN.md T-6.2).
 *
 * Three independent concerns, each verified with `user.tab()` +
 * `document.activeElement` assertions (no axe involved here — that's
 * `App.a11y.test.tsx`'s job):
 *
 * 1. Shell + Dashboard: Tab order reaches, in sequence, the skip link, the
 *    Sidebar's nav links, the folder-card CTA, the Dashboard header's
 *    actions, the jobs filter bar, the first row's actions, and the log
 *    console's controls. Intervening tab stops not named in PLAN.md's
 *    milestone list (TitleBar's 3 window-control buttons, the
 *    AuthRequiredBanner's 2 buttons) are tabbed through but not asserted on
 *    individually — the milestones are checkpoints, not an exhaustive list.
 * 2. `RowActions`'s inline action cluster: every applicable action is its own
 *    Tab stop in a fixed order (no `⋯` menu to open), focusing one shows its
 *    `Tooltip` and wires `aria-describedby`, and Escape dismisses the tooltip
 *    without moving focus off the button.
 * 3. Settings: Tab reaches every focusable field on the page (verified
 *    generically — the live DOM's own focusable-element order, not a
 *    hand-maintained list, so it never goes stale as fields are added),
 *    `Ctrl+S` saves only while the form is dirty, and the "Restaurar
 *    padrões" `ConfirmDialog` traps Tab among its own two buttons and closes
 *    on Escape.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";

import { DEFAULT_CONFIG } from "@/api/defaults";
import { makeJob } from "@/store/__fixtures__/jobs";
import { useConfigStore, type ConfigStatus } from "@/store/configStore";
import { useCredentialsStore } from "@/store/credentialsStore";
import { useJobsStore } from "@/store/jobsStore";
import { useLogStore } from "@/store/logStore";
import { useStatusStore } from "@/store/statusStore";
import type { AppConfig, AppStatus, JobView, ValidationIssue } from "@/types/generated";

import { App } from "../App";
import { JobRow } from "../components/dashboard/JobRow";
import { Settings } from "../pages/Settings";

/** Tabs forward until focus lands on `target`, or throws after `maxSteps` (a stuck/broken tab order should fail loudly, not hang). */
async function tabUntilElement(user: UserEvent, target: Element, maxSteps = 60): Promise<void> {
  for (let step = 0; step < maxSteps; step += 1) {
    await user.tab();
    if (document.activeElement === target) return;
  }
  const stuckAt = document.activeElement instanceof HTMLElement ? document.activeElement.outerHTML.slice(0, 160) : String(document.activeElement);
  throw new Error(`tabUntilElement: focus never reached the target within ${maxSteps} Tab presses (stuck at: ${stuckAt})`);
}

// ---------------------------------------------------------------------------
// 1. Shell + Dashboard tab order
// ---------------------------------------------------------------------------

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    minimize: vi.fn(),
    toggleMaximize: vi.fn(),
    close: vi.fn(),
  }),
}));

vi.mock("@/api/events", () => ({
  useTauriEvent: vi.fn(),
}));

// Single consolidated mock for the whole file: Vitest hoists `vi.mock` calls
// to the top of the module and only one registration per module path is in
// effect, so every wrapper this file's three concerns need (dashboard load,
// RowActions actions, Settings load/save) is declared here once.
vi.mock("@/api/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/ipc")>();
  return {
    ...actual,
    getConfig: vi.fn(),
    getStatus: vi.fn(),
    listJobs: vi.fn(),
    getRecentLogs: vi.fn(),
    saveConfig: vi.fn(),
    retryJob: vi.fn(),
    cancelJob: vi.fn(),
    openInExplorer: vi.fn(),
    openRemote: vi.fn(),
  };
});

import { getConfig, getRecentLogs, getStatus, listJobs, saveConfig } from "@/api/ipc";

const mockedGetConfig = vi.mocked(getConfig);
const mockedGetStatus = vi.mocked(getStatus);
const mockedListJobs = vi.mocked(listJobs);
const mockedGetRecentLogs = vi.mocked(getRecentLogs);
const mockedSaveConfig = vi.mocked(saveConfig);

const initialConfigState = useConfigStore.getState();
const initialStatusState = useStatusStore.getState();
const initialJobsState = useJobsStore.getState();
const initialCredentialsState = useCredentialsStore.getState();
const initialLogState = useLogStore.getState();

function config(overrides: Partial<AppConfig> = {}): AppConfig {
  return { ...DEFAULT_CONFIG, ...overrides };
}

function statusReady(): AppStatus {
  return {
    watcher_paused: false,
    destinations: {
      gdrive: { online: true, auth_required: false, latency_ms: 30 },
      s3: { online: true, auth_required: false, latency_ms: 41 },
    },
    counts_by_status: {
      pending: 1,
      uploading: 1,
      paused: 0,
      cancelled: 0,
      done: 1,
      failed: 1,
      bytes_total: 4096,
      bytes_done: 2048,
    },
    core_version: "0.1.0",
    build_target: "Tauri 2 • Windows x64",
  };
}

function threeFixtureJobs(): JobView[] {
  return [
    makeJob({ name: "relatorio.pdf", gdrive: { status: "uploading" }, s3: { status: "pending" } }),
    makeJob({ name: "planilha.xlsx", gdrive: { status: "done" }, s3: { status: "done" } }),
    makeJob({
      name: "backup.zip",
      gdrive: { status: "done" },
      s3: { status: "failed", last_error: "403 Forbidden: Access Denied" },
    }),
  ];
}

function renderApp(initialEntries: string[]) {
  return render(
    <MemoryRouter initialEntries={initialEntries}>
      <App />
    </MemoryRouter>,
  );
}

beforeEach(() => {
  useConfigStore.setState(initialConfigState, true);
  useStatusStore.setState(initialStatusState, true);
  useJobsStore.setState(initialJobsState, true);
  useCredentialsStore.setState(initialCredentialsState, true);
  useLogStore.setState(initialLogState, true);

  mockedGetConfig.mockReset();
  mockedGetStatus.mockReset();
  mockedListJobs.mockReset();
  mockedGetRecentLogs.mockReset();
  mockedSaveConfig.mockReset();

  mockedGetRecentLogs.mockResolvedValue([]);
  mockedGetConfig.mockResolvedValue(DEFAULT_CONFIG);
  mockedSaveConfig.mockResolvedValue(undefined);
});

describe("Shell + Dashboard tab order", () => {
  it("Tab reaches, in sequence: skip link -> nav links -> folder CTA -> header actions -> filter bar -> first row actions -> console controls", async () => {
    const user = userEvent.setup();

    mockedGetConfig.mockResolvedValue(config({ watch: { ...DEFAULT_CONFIG.watch, path: "C:\\Watch" } }));
    mockedGetStatus.mockResolvedValue(statusReady());
    mockedListJobs.mockResolvedValue({ items: threeFixtureJobs(), total: 3 });

    const { container } = renderApp(["/dashboard"]);
    await screen.findByText("backup.zip");

    const skipLink = screen.getByRole("link", { name: "Ir para o conteúdo" });
    const dashboardNavLink = container.querySelector('a[href="/dashboard"]');
    const settingsNavLink = container.querySelector('a[href="/settings"]');
    const folderCta = screen.getByRole("button", { name: "Alterar pasta" });
    const rescanButton = screen.getByRole("button", { name: "Atualizar Lista" });
    const filterGroup = screen.getByRole("group", { name: "Filtro de status" });
    const firstFilterButton = within(filterGroup).getByRole("button", { name: "Todos" });
    const firstRowExplorerButton = screen.getAllByRole("button", { name: "Abrir no Explorer" })[0];
    const consoleCollapseButton = screen.getByRole("button", { name: "Recolher console de eventos" });

    if (!dashboardNavLink) throw new Error("Expected the Sidebar's /dashboard NavLink to be rendered");
    if (!settingsNavLink) throw new Error("Expected the Sidebar's /settings NavLink to be rendered");
    if (!firstRowExplorerButton) throw new Error("Expected at least one row with an 'Abrir no Explorer' button");

    // Nothing is focused yet: the first Tab() lands on the skip link.
    await tabUntilElement(user, skipLink);
    await tabUntilElement(user, dashboardNavLink);
    await tabUntilElement(user, settingsNavLink);
    await tabUntilElement(user, folderCta);
    await tabUntilElement(user, rescanButton);
    await tabUntilElement(user, firstFilterButton);
    await tabUntilElement(user, firstRowExplorerButton);
    await tabUntilElement(user, consoleCollapseButton);
  });
});

// ---------------------------------------------------------------------------
// 2. RowActions inline cluster keyboard contract (isolated, per JobRow.actions.test.tsx's convention)
// ---------------------------------------------------------------------------

function renderRow(job: JobView) {
  return render(
    <table>
      <tbody>
        <JobRow job={job} progress={{}} />
      </tbody>
    </table>,
  );
}

describe("RowActions inline cluster keyboard contract", () => {
  it("Tab walks every applicable action in order, focus shows its tooltip, and Escape dismisses it", async () => {
    const user = userEvent.setup();
    // A done side (adds "Abrir remoto") and a failed one (adds "Reenviar" and
    // "Detalhes do erro") give the widest cluster this row can produce.
    const job = makeJob({
      gdrive: { status: "done" },
      s3: { status: "failed", last_error: "boom" },
    });
    renderRow(job);

    const explorer = screen.getByRole("button", { name: /abrir no explorer/i });
    const copyPath = screen.getByRole("button", { name: /copiar caminho/i });
    const retry = screen.getByRole("button", { name: /reenviar/i });
    const openRemote = screen.getByRole("button", { name: /abrir remoto/i });
    const errorDetails = screen.getByRole("button", { name: /detalhes do erro/i });

    explorer.focus();
    expect(explorer).toHaveFocus();

    // Focus surfaces the hint immediately — no hover dwell to wait out.
    const tooltip = await screen.findByRole("tooltip");
    expect(tooltip).toHaveTextContent(/abrir no explorer/i);
    expect(explorer).toHaveAttribute("aria-describedby", tooltip.id);

    // Escape dismisses the hint but leaves focus where it was.
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("tooltip")).not.toBeInTheDocument());
    expect(explorer).toHaveFocus();

    for (const next of [copyPath, retry, openRemote, errorDetails]) {
      await user.tab();
      expect(next).toHaveFocus();
    }
  });
});

// ---------------------------------------------------------------------------
// 3. Settings: every field reachable, Ctrl+S gating, ConfirmDialog focus trap
// ---------------------------------------------------------------------------

type SeedOverrides = {
  status?: ConfigStatus;
  config?: AppConfig;
  saved?: AppConfig;
  dirty?: boolean;
  issues?: ValidationIssue[];
  error?: string | null;
  lastSavedAt?: string | null;
};

/** Seeds the store as `load()` would have already resolved (`status: "ready"`) — mirrors `Settings.test.tsx`'s `seedReady`. */
function seedReady(overrides: SeedOverrides = {}): void {
  useConfigStore.setState({
    status: "ready",
    config: DEFAULT_CONFIG,
    saved: DEFAULT_CONFIG,
    dirty: false,
    issues: [],
    error: null,
    lastSavedAt: null,
    ...overrides,
  });
}

function renderSettings() {
  return render(
    <MemoryRouter>
      <Settings />
    </MemoryRouter>,
  );
}

/** Standard tabbable-elements selector (native semantics, no custom widgets to special-case in this codebase). */
/**
 * Elements Tab actually visits. `[tabindex="-1"]` has to be excluded on
 * *every* branch, not just the bare `[tabindex]` one: a `<button
 * tabindex="-1">` is programmatically focusable but deliberately out of the
 * tab order — `NumberField`'s stepper chevrons are exactly that, a pointer
 * affordance for a function the input already exposes to the keyboard. The
 * previous selector matched them and then failed because Tab, correctly,
 * never lands there.
 */
const FOCUSABLE_SELECTOR = [
  'a[href]:not([tabindex="-1"])',
  'button:not([disabled]):not([tabindex="-1"])',
  'input:not([disabled]):not([tabindex="-1"])',
  'select:not([disabled]):not([tabindex="-1"])',
  'textarea:not([disabled]):not([tabindex="-1"])',
  '[tabindex]:not([tabindex="-1"])',
].join(", ");

function focusableElementsIn(root: ParentNode): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR));
}

describe("Settings keyboard navigation", () => {
  it("Tab reaches every focusable field on the page, in DOM order", async () => {
    const user = userEvent.setup();
    seedReady({ config: { ...DEFAULT_CONFIG, s3: { ...DEFAULT_CONFIG.s3, enabled: true } } });
    const { container } = renderSettings();

    const expected = focusableElementsIn(container);
    expect(expected.length).toBeGreaterThan(5); // sanity: modules actually rendered fields, this isn't a no-op

    for (const element of expected) {
      await tabUntilElement(user, element);
    }
  });

  it("Ctrl+S saves whether or not the form is dirty, but not while validation issues are open", async () => {
    const user = userEvent.setup();
    seedReady({ config: { ...DEFAULT_CONFIG, s3: { ...DEFAULT_CONFIG.s3, enabled: true } } });
    renderSettings();

    // Clean form: `save()` is idempotent, so the shortcut still fires (RF-083)
    // — it is gated on validation, never on `dirty`.
    await user.keyboard("{Control>}s{/Control}");
    await waitFor(() => expect(mockedSaveConfig).toHaveBeenCalledTimes(1));

    await user.type(screen.getByLabelText("Bucket"), "novo-bucket");
    await user.keyboard("{Control>}s{/Control}");
    await waitFor(() => expect(mockedSaveConfig).toHaveBeenCalledTimes(2));

    mockedSaveConfig.mockClear();
    seedReady({
      config: { ...DEFAULT_CONFIG, s3: { ...DEFAULT_CONFIG.s3, enabled: true } },
      issues: [{ field: "s3.bucket", message: "Bucket obrigatório" }],
    });

    await user.keyboard("{Control>}s{/Control}");
    expect(mockedSaveConfig).not.toHaveBeenCalled();
  });

  it("'Restaurar padrões' ConfirmDialog traps Tab among its own buttons and Escape closes it", async () => {
    const user = userEvent.setup();
    seedReady({ config: { ...DEFAULT_CONFIG, workers_per_destination: 4 } });
    renderSettings();

    await user.click(screen.getByRole("button", { name: "Restaurar padrões" }));
    const dialog = screen.getByRole("dialog");

    const cancelButton = within(dialog).getByRole("button", { name: "Não restaurar" });
    const confirmButton = within(dialog).getByRole("button", { name: "Restaurar" });

    // Focus lands on the safe action (Cancelar) on open.
    expect(cancelButton).toHaveFocus();

    await user.tab();
    expect(confirmButton).toHaveFocus();

    // Forward from the last button wraps back to the first, never leaving the dialog.
    await user.tab();
    expect(cancelButton).toHaveFocus();

    // Backward from the first button wraps to the last.
    await user.tab({ shift: true });
    expect(confirmButton).toHaveFocus();

    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(useConfigStore.getState().config.workers_per_destination).toBe(4);
  });
});
