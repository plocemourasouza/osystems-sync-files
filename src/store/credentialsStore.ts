/**
 * `credentialsStore` — zustand store backing the Settings › Amazon S3 /
 * Google Drive credential UI (PRD.md RF-020/RF-023/RF-085; SPEC.md §7
 * `set_credential`, `get_credential_status`, `clear_credential`,
 * `test_connection`).
 *
 * Convention (CLAUDE.md): this is the only place that imports `@/api/ipc`'s
 * credential/test wrappers — components (`S3Module`, later `GDriveModule`)
 * call the actions/selectors exported here, never `@/api/ipc` directly.
 *
 * RNF-003: `status.aws.masked` / `status.gdrive.email` are the only
 * credential-shaped values this store ever holds — `setAws` takes the raw
 * secret only to forward it to `setCredential` (which sends it over IPC and
 * discards the local variable); it is never stored in state.
 */
import { create } from "zustand";

import {
  clearCredential,
  getCredentialStatus,
  isAppError,
  pickServiceAccountFile,
  setCredential,
  testConnection,
  type ServiceAccountInfo,
} from "@/api/ipc";
import { isMockMode } from "@/dev/mockMode";
import type { CredentialStatus, Destination, TestResult } from "@/types/generated";

/**
 * Outcome of the last `test_connection` call for a destination.
 *
 * `test_connection` only ever *resolves* on success (`uploaders/s3.rs`
 * always returns `Err` on any failing step, never `Ok({ ok: false })`), so
 * a failure is represented here as `{ error: code }` from the caught
 * {@link AppError}, not as a `TestResult` with `ok: false`.
 */
export type TestOutcome = TestResult | { error: string };

interface CredentialsState {
  status: CredentialStatus | null;
  loading: boolean;
  error: string | null;
  /** Whether a `test_connection` call is in flight, per destination. */
  testing: Record<Destination, boolean>;
  /** Result of the last `test_connection` call, per destination (`null` = never tested this session). */
  lastTest: Record<Destination, TestOutcome | null>;
  /**
   * Metadata of the Service Account JSON picked in this session via
   * {@link CredentialsActions.pickServiceAccount} (file name/size/e-mail —
   * never the raw JSON, RNF-003). `null` until a pick succeeds; cleared on
   * `clearServiceAccount`. Not persisted/refetched from `status` because the
   * backend doesn't re-expose the picked file's name/size after reload —
   * only `status.gdrive.email`/`project_id` survive a refresh.
   */
  lastServiceAccount: ServiceAccountInfo | null;
}

interface CredentialsActions {
  /** Fetches `get_credential_status` and replaces `status`. */
  refresh: () => Promise<void>;
  /** Saves both AWS credential parts, then refreshes `status`. */
  setAws: (params: { accessKeyId: string; secret: string }) => Promise<void>;
  /** Clears both AWS credential parts, then refreshes `status`. */
  clearAws: () => Promise<void>;
  /**
   * Opens the native picker for a Google Drive Service Account JSON
   * (`pick_service_account_file` persists it to the OS keyring on the Rust
   * side — the renderer never receives the raw JSON, RNF-003). On success,
   * keeps the returned metadata in `lastServiceAccount` and refreshes
   * `status`. Resolves `null` (no store mutation) when the user cancels.
   */
  pickServiceAccount: () => Promise<ServiceAccountInfo | null>;
  /** Clears the stored Service Account credential, then refreshes `status`. */
  clearServiceAccount: () => Promise<void>;
  /** Runs `test_connection` for one destination, recording the outcome in `lastTest`. */
  test: (destination: Destination) => Promise<void>;
}

export type CredentialsStore = CredentialsState & CredentialsActions;

export const useCredentialsStore = create<CredentialsStore>()((set, get) => ({
  status: null,
  loading: false,
  error: null,
  testing: { s3: false, gdrive: false },
  lastTest: { s3: null, gdrive: null },
  lastServiceAccount: null,

  refresh: async () => {
    // Dev preview (`?mock=1`): `src/dev/mock.ts` already seeded `status`.
    if (isMockMode()) return;
    set({ loading: true, error: null });
    try {
      const status = await getCredentialStatus();
      set({ status, loading: false });
    } catch (e) {
      set({ loading: false, error: e instanceof Error ? e.message : "unknown" });
    }
  },

  setAws: async ({ accessKeyId, secret }) => {
    await setCredential("aws.access_key_id", accessKeyId);
    await setCredential("aws.secret_access_key", secret);
    await get().refresh();
  },

  clearAws: async () => {
    await clearCredential("aws.access_key_id");
    await clearCredential("aws.secret_access_key");
    await get().refresh();
  },

  pickServiceAccount: async () => {
    const info = await pickServiceAccountFile();
    if (info === null) return null;
    set({ lastServiceAccount: info });
    await get().refresh();
    return info;
  },

  clearServiceAccount: async () => {
    await clearCredential("gdrive.service_account_json");
    set({ lastServiceAccount: null });
    await get().refresh();
  },

  test: async (destination) => {
    set((s) => ({ testing: { ...s.testing, [destination]: true } }));
    try {
      const result = await testConnection(destination);
      set((s) => ({
        testing: { ...s.testing, [destination]: false },
        lastTest: { ...s.lastTest, [destination]: result },
      }));
    } catch (e) {
      const code = isAppError(e) ? e.code : "unknown";
      set((s) => ({
        testing: { ...s.testing, [destination]: false },
        lastTest: { ...s.lastTest, [destination]: { error: code } },
      }));
    }
  },
}));
