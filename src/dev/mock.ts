/**
 * `seedMockData` — Dashboard preview fixtures, loaded ONLY behind
 * `?mock=1` in a dev server (`isMockMode()`, `src/dev/mockMode.ts`).
 *
 * Real-device bug hunting (a 1366×768 Windows laptop, see `JobTable.tsx`'s
 * colgroup comment) needs a populated table/console without a running
 * Tauri backend — this seeds every store directly via `setState`, bypassing
 * `@/api/ipc` entirely (`ipc.ts` itself is untouched, per convention: only
 * `main.tsx`'s dev-guarded dynamic import ever reaches this module).
 *
 * Tree-shaking contract: nothing outside `src/dev/` imports this file. The
 * sole import site (`main.tsx`) wraps it in `if (import.meta.env.DEV)`,
 * which Vite's production build statically resolves to `false` and removes
 * — including the dynamic `import()` itself — so this module (and its
 * fixture strings) never reaches the production bundle.
 */
import { useConfigStore } from "@/store/configStore";
import { useCredentialsStore } from "@/store/credentialsStore";
import { useJobsStore } from "@/store/jobsStore";
import { useLogStore } from "@/store/logStore";
import { useStatusStore } from "@/store/statusStore";
import { useThroughputStore } from "@/store/throughputStore";
import type { AppStatus, CredentialStatus, JobSide, JobView, LogLine } from "@/types/generated";

const WATCH_ROOT = "C:\\monitoramento";

function side(overrides: Partial<JobSide> & Pick<JobSide, "job_id" | "status">): JobSide {
  return {
    attempts: 0,
    next_attempt_at: null,
    remote_id: null,
    last_error: null,
    updated_at: "2026-09-04T13:00:00.000Z",
    ...overrides,
  };
}

const MOCK_JOBS: JobView[] = [
  {
    file_id: "f1",
    path: `${WATCH_ROOT}\\contrato-fornecedor-2026.pdf`,
    name: "contrato-fornecedor-2026.pdf",
    size: 245_000,
    sha256: "a1b2c3",
    detected_at: "2026-09-04T13:24:00.000Z",
    gdrive: side({ job_id: "f1-gdrive", status: "pending" }),
    s3: side({ job_id: "f1-s3", status: "pending" }),
  },
  {
    file_id: "f2",
    path: `${WATCH_ROOT}\\backups\\backup-financeiro-setembro.zip`,
    name: "backup-financeiro-setembro.zip",
    size: 1_800_000_000,
    sha256: "d4e5f6",
    detected_at: "2026-09-04T13:18:00.000Z",
    gdrive: side({ job_id: "f2-gdrive", status: "uploading", attempts: 1 }),
    s3: side({ job_id: "f2-s3", status: "pending" }),
  },
  {
    file_id: "f3",
    path: `${WATCH_ROOT}\\obras\\planilha-custos-obra.xlsx`,
    name: "planilha-custos-obra.xlsx",
    size: 92_000,
    sha256: "071829",
    detected_at: "2026-09-04T12:55:00.000Z",
    gdrive: side({ job_id: "f3-gdrive", status: "paused", attempts: 1 }),
    s3: side({ job_id: "f3-s3", status: "done" }),
  },
  {
    file_id: "f4",
    path: `${WATCH_ROOT}\\contratos\\aditivo-contratual-cliente-premium.docx`,
    name: "aditivo-contratual-cliente-premium.docx",
    size: 58_000,
    sha256: "3a4b5c",
    detected_at: "2026-09-04T11:40:00.000Z",
    gdrive: side({ job_id: "f4-gdrive", status: "done" }),
    s3: side({ job_id: "f4-s3", status: "done" }),
  },
  {
    file_id: "f5",
    path: `${WATCH_ROOT}\\notas\\nota-fiscal-0001234.xml`,
    name: "nota-fiscal-0001234.xml",
    size: 4_200,
    sha256: "9f8e7d",
    detected_at: "2026-09-04T10:05:00.000Z",
    gdrive: side({ job_id: "f5-gdrive", status: "cancelled" }),
    s3: side({ job_id: "f5-s3", status: "done" }),
  },
  {
    file_id: "f6",
    // Verbatim NT prefix, as it actually arrived over IPC on the real
    // 1366×768 Windows run this fixture set is reproducing (`relativeDir.ts`).
    path: "//?/C:/monitoramento/relatorios/relatorio-mensal-consolidado-setembro-2026-versao-final-revisado.pdf",
    name: "relatorio-mensal-consolidado-setembro-2026-versao-final-revisado.pdf",
    size: 5_200_000,
    sha256: "112233",
    detected_at: "2026-09-04T09:30:00.000Z",
    gdrive: side({ job_id: "f6-gdrive", status: "done" }),
    s3: side({
      job_id: "f6-s3",
      status: "failed",
      attempts: 3,
      last_error: "403 Forbidden: Access Denied",
    }),
  },
];

/**
 * Newest LAST, matching what the app actually receives: the core's
 * `recent()` returns "up to `limit` of the most recent lines, oldest first"
 * and `logStore.push` appends. The literal below is written newest-first for
 * readability, so it is sorted by `ts` on the way in — without that the mock
 * console rendered upside down and auto-scrolled to the oldest line. Sorting
 * rather than reversing keeps it right even where the literal's own order
 * slips (two entries share the 13:24:00 second, written ascending).
 */
const MOCK_LOG_LINES_NEWEST_FIRST: LogLine[] = [
  { ts: "2026-09-04T13:24:00.100Z", level: "INFO", target: "osystems_sync_lib::watcher", job_id: null, destination: null, message: "Novo arquivo detectado: contrato-fornecedor-2026.pdf (245 KB)" },
  { ts: "2026-09-04T13:24:00.400Z", level: "DEBUG", target: "osystems_sync_lib::hash", job_id: "f1", destination: null, message: "SHA-256 calculado em 187ms" },
  { ts: "2026-09-04T13:18:02.000Z", level: "INFO", target: "osystems_sync_lib::uploaders::gdrive", job_id: "f2-gdrive", destination: "gdrive", message: "Upload iniciado: backup-financeiro-setembro.zip" },
  { ts: "2026-09-04T13:18:15.250Z", level: "INFO", target: "osystems_sync_lib::uploaders::gdrive", job_id: "f2-gdrive", destination: "gdrive", message: "Progresso: 420.0 MB / 1.80 GB (23%) a 8.4 MB/s" },
  { ts: "2026-09-04T12:55:10.000Z", level: "WARN", target: "osystems_sync_lib::uploaders::gdrive", job_id: "f3-gdrive", destination: "gdrive", message: "Upload pausado manualmente pelo usuário" },
  { ts: "2026-09-04T11:40:05.000Z", level: "INFO", target: "osystems_sync_lib::uploaders::s3", job_id: "f4-s3", destination: "s3", message: "Upload concluído: aditivo-contratual-cliente-premium.docx" },
  { ts: "2026-09-04T11:40:06.000Z", level: "INFO", target: "osystems_sync_lib::state", job_id: "f4", destination: null, message: "Job f4 marcado como sincronizado em ambos os destinos" },
  { ts: "2026-09-04T10:05:20.000Z", level: "WARN", target: "osystems_sync_lib::queue", job_id: "f5-gdrive", destination: "gdrive", message: "Job cancelado pelo usuário antes do envio" },
  {
    ts: "2026-09-04T09:30:45.000Z",
    level: "ERROR",
    target: "osystems_sync_lib::uploaders::s3",
    job_id: "f6-s3",
    destination: "s3",
    message:
      "Falha no upload de relatorio-mensal-consolidado-setembro-2026-versao-final-revisado.pdf: 403 Forbidden: Access Denied — verifique as permissões do bucket e a política IAM associada às credenciais configuradas",
  },
  { ts: "2026-09-04T09:30:46.000Z", level: "INFO", target: "osystems_sync_lib::queue", job_id: "f6-s3", destination: "s3", message: "Nova tentativa agendada (3/5) em 40s" },
  { ts: "2026-09-04T09:25:00.000Z", level: "INFO", target: "osystems_sync_lib_bin", job_id: null, destination: null, message: "oSystems Sync iniciado — versão 0.9.0 (x86_64-pc-windows-msvc)" },
  { ts: "2026-09-04T09:24:58.000Z", level: "DEBUG", target: "osystems_sync_lib::watcher", job_id: null, destination: null, message: "Observando pasta: C:\\monitoramento (recursivo: não)" },
];

const MOCK_STATUS: AppStatus = {
  watcher_paused: false,
  destinations: {
    gdrive: { online: true, auth_required: false, latency_ms: 142 },
    s3: { online: true, auth_required: false, latency_ms: 88 },
  },
  counts_by_status: {
    pending: 2,
    uploading: 1,
    paused: 1,
    cancelled: 1,
    done: 4,
    failed: 1,
    bytes_total: 7_399_200,
    bytes_done: 5_350_000,
  },
  core_version: "0.9.0",
  build_target: "x86_64-pc-windows-msvc",
};

const MOCK_CREDENTIALS: CredentialStatus = {
  aws: { present: true, masked: "AKIA****3F2A" },
  gdrive: { present: true, email: "sync@empresa.com.br", project_id: "osystems-sync" },
};

/** Seeds every store `Dashboard.tsx` reads from, bypassing `@/api/ipc`. */
export function seedMockData(): void {
  useConfigStore.setState((s) => ({
    status: "ready",
    config: { ...s.config, watch: { ...s.config.watch, path: WATCH_ROOT } },
    saved: { ...s.saved, watch: { ...s.saved.watch, path: WATCH_ROOT } },
  }));

  useJobsStore.setState({
    items: MOCK_JOBS,
    total: MOCK_JOBS.length,
    loading: false,
    error: null,
  });

  useStatusStore.setState({ status: MOCK_STATUS, loading: false, error: null });

  useCredentialsStore.setState({ status: MOCK_CREDENTIALS, loading: false, error: null });

  useLogStore
  .getState()
  .hydrate([...MOCK_LOG_LINES_NEWEST_FIRST].sort((a, b) => a.ts.localeCompare(b.ts)));

  useThroughputStore.setState({
    totalBps: 8_400_000,
    gdriveBps: 8_400_000,
    s3Bps: 0,
    limitGdriveBps: null,
    limitS3Bps: 50_000_000,
    updatedAt: "2026-09-04T13:24:10.000Z",
  });
}
