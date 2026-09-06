# oSystems Sync — Especificação Técnica (v2)

Aplicação desktop (Windows) que monitora uma pasta local e envia cada arquivo novo para uma pasta no Google Drive e para um bucket no AWS S3. Roda na bandeja do sistema por longos períodos (10+ dias) sem intervenção.

Stack: **Tauri 2 + Rust** (core) e **React + TypeScript + Vite + Tailwind v4** (UI).

> **v2 (2026-09-04)** — reconciliada com `PRD.md` e os assets de design. Mudanças: autenticação Drive por **Service Account**, módulo **`throttle`** (QoS por destino), **2 telas** em vez de 5, status `paused`, tokens em `design/tokens.css`. Decisões em §12 (C1–C11). Versão anterior preservada em `assets/SPEC.v1.md`.
> Ordem das fontes de verdade: `PRD.md` (o quê) → este arquivo (como) → `design/` (aparência). IDs `RF-xxx` abaixo referem-se ao PRD.

---

## 1. Requisitos

### Funcionais

| ID | Requisito |
|----|-----------|
| RF01 | Usuário configura: pasta monitorada, credenciais AWS (Access Key + Secret), bucket/prefixo/storage class S3, **JSON da Service Account** Google, ID da pasta no Drive (PRD RF-001, RF-010, RF-020). |
| RF02 | Ao surgir um arquivo novo na pasta, enviar para S3 e Drive de forma independente. |
| RF03 | Cada destino tem retry com backoff exponencial (máx. 5 tentativas, depois marca `failed`). |
| RF04 | Nunca enviar o mesmo arquivo duas vezes ao mesmo destino (chave: caminho + SHA-256). |
| RF05 | Ao iniciar ou retornar de suspensão, varrer a pasta e enfileirar o que ainda não foi enviado. |
| RF06 | UI mostra: status dos destinos, fila atual, histórico, erros, logs. |
| RF07 | Botão "Testar conexão" para cada destino. |
| RF08 | Botão "Reenviar" para itens `failed`. |
| RF09 | Iniciar com o Windows; fechar janela minimiza para tray. |
| RF10 | Filtros opcionais: extensões permitidas, ignorar subpastas, tamanho máximo. |
| RF11 | **QoS**: limite de upload por destino (0,5–10 MB/s ou ilimitado), token bucket compartilhado pelos workers do destino, aplicado em runtime ao salvar (PRD RF-050…052). |
| RF12 | **Pausar watcher** congela só a detecção; uploads em voo continuam (PRD RF-005). |
| RF13 | **Limpar concluídos** arquiva jobs `done` da visão (`archived_at`), sem apagar (PRD RF-036). |
| RF14 | Ações por job: reenviar, cancelar, abrir no Explorer, abrir remoto; **pausar/retomar job** é Should-have (status `paused`, PRD RF-037). |
| RF15 | Token de acesso da SA renovado automaticamente (JWT 1 h, skew 5 min) (PRD RF-016). |

### Não funcionais

| ID | Requisito |
|----|-----------|
| RNF01 | Uptime contínuo de 10+ dias sem crescimento de memória perceptível. |
| RNF02 | Credenciais (AWS secret **e JSON completo da Service Account**) nunca em texto plano em disco; usar `keyring` (Windows Credential Manager, service `osystems-sync`). |
| RNF03 | Renderer (UI) nunca recebe credenciais completas; só status e máscara (`AKIA****XYZ`). |
| RNF04 | Arquivos até 5 GB; usar upload multipart (S3) e resumable (Drive) acima de 8 MB. |
| RNF05 | Logs rotacionados (max 10 MB × 5 arquivos). |
| RNF06 | Estado persistido em SQLite; reinício não perde fila. |

Fora de escopo: sincronização bidirecional, exclusão remota, múltiplas pastas, monitoramento sem usuário logado (serviço Windows), **OAuth com conta Google pessoal** (fallback documentado em §12, não implementado).

---

## 2. Arquitetura

```
┌─────────────────────────────────────────────────────────┐
│ Tauri App (processo único)                              │
│                                                         │
│  ┌──────────────┐   IPC (commands/events)   ┌────────┐  │
│  │  Renderer    │ ◄───────────────────────► │ Tauri  │  │
│  │  React/TS    │                           │ shell  │  │
│  └──────────────┘                           └───┬────┘  │
│                                                 │        │
│                                    ┌────────────▼──────┐ │
│                                    │  core (crate Rust)│ │
│                                    │  watcher → queue  │ │
│                                    │  → uploaders      │ │
│                                    │  state (SQLite)   │ │
│                                    │  credentials      │ │
│                                    └───────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

Regra central: **`core` não depende de Tauri.** É um crate de biblioteca com API própria; `src-tauri` é apenas adaptador (comandos IPC, tray, autostart). Isso permite extrair para serviço Windows no futuro sem reescrever.

### Fluxo de um arquivo

```
notify event (Create/Modify)
  → debounce 2s
  → stabilize(): tamanho inalterado por 3 leituras (1s) E abre com share_mode exclusivo
  → hash SHA-256
  → state.upsert_file(path, hash, size)  [status=pending]
  → queue.push(FileJob { file_id, dest: S3 }), queue.push(FileJob { file_id, dest: Drive })
  → worker pool (2 workers por destino) consome
  → throttle[dest].acquire(bytes) envolve cada chunk do body (RF11)
  → uploader.upload() → Ok  → state.mark_done(file_id, dest, remote_id)
                     → Err → state.mark_retry(attempt+1, next_at = now + 2^attempt * 5s)
                              attempt >= 5 → state.mark_failed(error)
  → emit event "job-updated" para o renderer
```

---

## 3. Estrutura do projeto

```
osystems-sync/
├── CLAUDE.md
├── PRD.md                       # o quê (requisitos RF-xxx / RNF-xxx)
├── SPEC.md                      # este arquivo (como)
├── PLAN.md                      # roadmap por fases/waves
├── design/
│   ├── DESIGN.md                # design system + specs das 2 telas (ler antes de criar telas)
│   ├── tokens.css               # CSS custom properties (fonte do @theme Tailwind v4)
│   └── README.md                # proveniência dos tokens
├── assets/                      # mockups Stitch, doc funcional v2, SPEC.v1.md (só referência)
├── package.json
├── vite.config.ts
├── src/                         # renderer (React + TS)
│   ├── main.tsx
│   ├── App.tsx
│   ├── api/
│   │   └── ipc.ts               # wrappers tipados de invoke()/listen()
│   ├── pages/
│   │   ├── Dashboard.tsx        # /dashboard — KPIs + tabela dupla + console de log
│   │   └── Settings.tsx         # /settings — Drive, S3, QoS, Geral
│   ├── components/
│   │   ├── shell/               # TitleBar, Sidebar, StatusBar
│   │   ├── dashboard/           # KpiCard, JobTable, JobRow, DualProgress, LogConsole
│   │   └── settings/            # DriveModule, S3Module, QosModule, GeneralModule
│   ├── i18n/                    # strings pt-BR (RNF-017)
│   ├── styles/
│   │   └── app.css              # @import '../../design/tokens.css'; @theme {...}
│   ├── store/                   # zustand
│   └── types/
│       └── generated.ts         # gerado por ts-rs a partir do Rust
├── src-tauri/
│   ├── Cargo.toml               # workspace root
│   ├── tauri.conf.json
│   ├── capabilities/default.json
│   ├── icons/
│   ├── src/
│   │   ├── main.rs
│   │   ├── lib.rs               # setup: tray, autostart, spawn core
│   │   ├── commands/
│   │   │   ├── config.rs
│   │   │   ├── credentials.rs
│   │   │   ├── queue.rs
│   │   │   ├── qos.rs
│   │   │   ├── system.rs        # open_in_explorer, open_logs_folder, set_autostart
│   │   │   └── logs.rs
│   │   ├── events.rs            # emissão de eventos p/ renderer
│   │   └── tray.rs
│   └── crates/
│       └── core/
│           ├── Cargo.toml
│           └── src/
│               ├── lib.rs
│               ├── config.rs
│               ├── watcher.rs
│               ├── stabilize.rs
│               ├── hash.rs
│               ├── queue.rs
│               ├── worker.rs
│               ├── throttle.rs  # token bucket por destino (RF11)
│               ├── health.rs    # online/offline (+ latência, Should) por destino
│               ├── state/
│               │   ├── mod.rs
│               │   ├── schema.sql
│               │   └── repo.rs
│               ├── uploaders/
│               │   ├── mod.rs   # trait Uploader
│               │   ├── s3.rs
│               │   └── gdrive/
│               │       ├── mod.rs
│               │       ├── auth.rs      # Service Account JWT → access token
│               │       ├── upload.rs
│               │       └── folders.rs   # subpastas por data (Should, RF-015)
│               ├── credentials.rs
│               ├── power.rs     # sleep/resume + SetThreadExecutionState
│               └── logging.rs
└── tests/
    └── e2e/                     # opcional
```

---

## 4. Dependências

### Rust (`crates/core`)

```toml
tokio = { version = "1", features = ["full"] }
notify = "6"
notify-debouncer-full = "0.3"
aws-config = "1"
aws-sdk-s3 = "1"
reqwest = { version = "0.12", features = ["json", "stream", "rustls-tls"] }
yup-oauth2 = "11"                 # apenas ServiceAccountAuthenticator (sem installed flow)
tokio-util = { version = "0.7", features = ["io"] }   # CancellationToken + stream adapters
async-trait = "0.1"
rusqlite = { version = "0.32", features = ["bundled"] }
keyring = "3"
sha2 = "0.10"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
tracing = "0.1"
tracing-appender = "0.2"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
ts-rs = "9"
chrono = "0.4"
uuid = { version = "1", features = ["v4"] }
windows = { version = "0.58", features = ["Win32_System_Power"] }
```

### Rust (`src-tauri`)

```toml
tauri = { version = "2", features = ["tray-icon"] }
tauri-plugin-autostart = "2"
tauri-plugin-dialog = "2"      # seletor de pasta
tauri-plugin-opener = "2"      # abrir Explorer (/select), pasta de logs, links remotos
tauri-plugin-notification = "2"
core = { path = "crates/core" }
```

### Frontend

```
react, react-dom, typescript, vite
@tauri-apps/api, @tauri-apps/plugin-dialog, @tauri-apps/plugin-opener
zustand, react-router-dom
tailwindcss@4 (@theme alimentado por design/tokens.css), lucide-react (ícones SVG inline; substitui Material Symbols do mockup)
fontes vendorizadas: Inter Variable, JetBrains Mono Variable (public/fonts/)
```

---

## 5. Modelo de dados

### `config.json` (em `%APPDATA%/osystems-sync/`, sem segredos)

```json
{
  "version": 1,
  "watch": {
    "path": "C:\\Users\\x\\Exportacoes",
    "recursive": false,
    "extensions": [],
    "min_size_mb": 0,
    "max_size_mb": 0,
    "stabilize_seconds": 3
  },
  "s3": {
    "enabled": true,
    "region": "us-east-1",
    "bucket": "meu-bucket",
    "prefix": "exportacoes/",
    "storage_class": "STANDARD"
  },
  "gdrive": {
    "enabled": true,
    "folder_id": "1AbC...",
    "auth_mode": "service_account",
    "date_subfolders": false
  },
  "qos": {
    "gdrive_limit_mbps": null,
    "s3_limit_mbps": null,
    "night_mode": { "enabled": false, "start": "23:00", "end": "06:00" }
  },
  "retry": { "max_attempts": 5, "base_delay_seconds": 5 },
  "workers_per_destination": 2,
  "autostart": true,
  "keep_awake": true
}
```

### Credenciais (via `keyring`, service = `osystems-sync`)

| chave | conteúdo |
|-------|----------|
| `aws.access_key_id` | string |
| `aws.secret_access_key` | string |
| `gdrive.service_account_json.{0..n}` + `gdrive.service_account_json.count` | conteúdo integral do JSON da SA (≈ 2–3 KB), **sempre fragmentado**: `keyring` grava o blob em UTF-16 no Credential Manager (limite 2560 bytes ⇒ ~1280 chars ASCII), então o JSON é dividido em pedaços de 1024 chars, com a contagem em `.count`, e reconcatenado na leitura. Teste obrigatório: JSON de 3,2 KB → 4 fragmentos, round-trip idêntico. |

`get_credential_status` deriva `client_email` e `project_id` do JSON e devolve **só** isso ao renderer.

### SQLite (`state.db`)

```sql
CREATE TABLE files (
  id          TEXT PRIMARY KEY,          -- uuid
  path        TEXT NOT NULL UNIQUE,
  sha256      TEXT NOT NULL,
  size        INTEGER NOT NULL,
  detected_at TEXT NOT NULL              -- ISO8601 UTC
);

CREATE TABLE jobs (
  id          TEXT PRIMARY KEY,
  file_id     TEXT NOT NULL REFERENCES files(id),
  destination TEXT NOT NULL CHECK (destination IN ('s3','gdrive')),
  status      TEXT NOT NULL CHECK (status IN ('pending','uploading','paused','cancelled','done','failed')),
  attempts    INTEGER NOT NULL DEFAULT 0,
  next_attempt_at TEXT,
  remote_id   TEXT,                       -- S3 key ou Drive fileId
  remote_state TEXT,                      -- JSON: S3 {upload_id, parts[]} | Drive {session_uri, offset} p/ retomar
  last_error  TEXT,
  archived_at TEXT,                       -- "Limpar concluídos" (RF13); NULL = visível
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  UNIQUE (file_id, destination)
);

CREATE INDEX idx_jobs_status ON jobs(status, next_attempt_at);
CREATE INDEX idx_jobs_visible ON jobs(archived_at, created_at DESC);
```

**Regras de escrita (obrigatórias):**
- **Caminho canônico**: `files.path` guarda sempre `paths::canonicalize_clean(path)` (= `fs::canonicalize` sem o prefixo verbatim `\\?\` / `\\?\UNC\` do Windows); a canonicalização acontece em `queue::intake` (único ponto de escrita) e na raiz do `rescan`. Motivo: `notify` entrega caminhos canônicos (`/private/tmp/…` no macOS; `\\?\`/case no Windows) enquanto `watch.path` é o texto do usuário — sem isso o mesmo arquivo vira duas linhas (bug pego no aceite da Fase 2).
- **Hash mudou para um `path` já existente** (RF-039): `upsert_file_and_enqueue` **atualiza** a linha em `files` (`sha256`, `size`, `mtime`) e faz `UPDATE jobs SET status='pending', attempts=0, remote_id=NULL, remote_state=NULL, archived_at=NULL, last_error=NULL WHERE file_id=?` — nunca insere um segundo par de jobs (violaria `UNIQUE(file_id, destination)`).
- `files` ganha coluna `mtime TEXT NOT NULL`: o rescan compara `size`+`mtime` primeiro e só re-hasheia em caso de divergência (RNF-007, RF-094).
- `cancel_job` → `status='cancelled'` (terminal, oculto da fila ativa; visível no filtro Todos). `clear_completed` arquiva **por arquivo**, só quando os 2 jobs do `file_id` estão `done`.
- Crash recovery no boot: `UPDATE jobs SET status='pending' WHERE status='uploading'` **e**, para cada job recuperado cujo `remote_state` tenha `upload_id`, `AbortMultipartUpload` antes de reenviar (sem órfãos cobráveis).

```sql
-- (fim do schema)

CREATE TABLE events (                     -- histórico p/ UI e auditoria
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  ts        TEXT NOT NULL,
  level     TEXT NOT NULL,
  job_id    TEXT,
  message   TEXT NOT NULL
);
```

---

## 6. Core — contratos

### `uploaders/mod.rs`

```rust
#[async_trait]
pub trait Uploader: Send + Sync {
    fn id(&self) -> Destination;              // S3 | GDrive
    async fn test_connection(&self) -> Result<(), UploadError>;
    async fn upload(&self, req: UploadRequest) -> Result<UploadResult, UploadError>;
}

pub struct UploadRequest {
    pub local_path: PathBuf,
    pub remote_name: String,
    pub size: u64,
    pub sha256: String,
    pub progress: mpsc::Sender<ProgressUpdate>,   // bytes enviados
    pub cancel: CancellationToken,
}

pub struct UploadResult { pub remote_id: String }

#[derive(thiserror::Error, Debug)]
pub enum UploadError {
    #[error("auth")]      Auth(String),          // não faz retry
    #[error("transient")] Transient(String),     // faz retry
    #[error("permanent")] Permanent(String),     // não faz retry (ex.: 4xx exceto 401/429)
    #[error("io")]        Io(#[from] std::io::Error),
}
```

Classificação de erros: `Transient` = timeout, 5xx, 429, rede, **`Io` (lock de antivírus — mesmo teto de 5 tentativas)** e **403 do Drive cujo `error.errors[0].reason` seja `rateLimitExceeded` / `userRateLimitExceeded` / `storageQuotaExceeded` (backoff fixo de 1 h)**. `Auth` = 401, **403 com reason `forbidden` / `insufficientPermissions` (Drive) ou `AccessDenied` (S3)**, e **400 `invalid_grant` do endpoint de token da SA (relógio desviado — `hint` de clock skew)** → marca destino como "precisa reautenticar", pausa jobs desse destino e emite `auth-required`. `Permanent` = demais 4xx → `failed` imediato. A classificação lê o corpo JSON do erro, não só o status HTTP.

### `s3.rs`

- `aws_sdk_s3::Client` com credenciais estáticas do keyring.
- `< 8 MB`: `put_object`. `>= 8 MB`: multipart, parte de 16 MB, 4 partes concorrentes.
- Key final: `{prefix}{remote_name}`. Metadata `x-amz-meta-sha256`.
- `test_connection`: `head_bucket`.
- Idempotência: antes de enviar, `head_object`; se existe com mesmo sha256 na metadata, retorna `Ok` sem reenviar.

### `gdrive/`

- `auth.rs`: **Service Account** (decisão C1). Lê `gdrive.service_account_json` do keyring → `yup_oauth2::ServiceAccountKey` → `ServiceAccountAuthenticator` (JWT RS256, `aud=https://oauth2.googleapis.com/token`, validade 1 h, escopo `https://www.googleapis.com/auth/drive`). Access token cacheado em memória com skew de 5 min; nunca persistido. Erros de parse do JSON → `UploadError::Auth`. Sem browser, sem redirect, sem refresh token.
- `folders.rs` (Should): resolve/cria `YYYY/MM/DD_backup/` sob `folder_id`; cache `HashMap<date, folder_id>` invalidado à meia-noite.
- `upload.rs`: API REST v3.
  - `< 8 MB`: `POST /upload/drive/v3/files?uploadType=multipart`.
  - `>= 8 MB`: `uploadType=resumable`, chunks de 16 MB, retomar via `Content-Range` em caso de falha transitória.
  - Body metadata: `{ "name", "parents": [folder_id] }`.
  - `supportsAllDrives=true` para pastas compartilhadas.
- Idempotência: `files.list` com `q="name='X' and 'folder_id' in parents and trashed=false"` e `fields=files(id,name,size,sha256Checksum,webViewLink)`; compara `sha256Checksum` com o hash local (única hash calculada pelo pipeline); se o campo vier ausente, compara `size` e **reenvia em caso de dúvida**.
- Resposta do upload pedida com `fields=id,webViewLink`; `webViewLink` persistido em `jobs.remote_state.web_view_link` (usado por `open_remote`).
- Sessão resumable expira em ~1 semana: se `session_uri` guardado devolver 404/410, limpar `remote_state` e reiniciar do byte 0.
- `test_connection`: `GET /drive/v3/files/{folder_id}?fields=id,name,driveId` **+ criar e apagar `.osystems-sync-probe` (0 bytes) na pasta** — SA só-leitura não pode reportar `Conectado`.

### `watcher.rs` + `stabilize.rs`

```rust
pub fn spawn_watcher(cfg: WatchConfig, tx: mpsc::Sender<PathBuf>) -> Result<WatcherHandle>;
pub async fn wait_until_stable(path: &Path, stable_secs: u64, timeout: Duration) -> Result<u64 /*size*/>;
```

`wait_until_stable`: a cada 1s lê `metadata().len()`; considera estável após `stable_secs` leituras iguais consecutivas **e** `OpenOptions::new().read(true).share_mode(0)` (Windows) bem-sucedido. Timeout 30 min → log + tenta mesmo assim.

Ignorar: arquivos temporários (`~$*`, `*.tmp`, `*.crdownload`, `*.part`, ocultos) e diretórios.

### `throttle.rs` (RF11)

```rust
pub struct Throttle { limit_bps: AtomicU64 /* 0 = ilimitado */, bucket: Mutex<Bucket> }
impl Throttle {
    pub fn new(limit_bps: u64) -> Arc<Self>;
    pub fn set_limit(&self, limit_bps: u64);          // hot-reload em save_config
    pub async fn acquire(&self, bytes: usize);         // bloqueia até haver tokens; refil a cada 100 ms; burst = 1 s de teto
}
pub struct ThrottledReader<R> { inner: R, throttle: Arc<Throttle> }  // impl AsyncRead: chama acquire(n) após cada poll_read
```

- Um `Arc<Throttle>` **por destino**, compartilhado pelos `workers_per_destination` (RF-052).
- Uploaders envolvem o body (`ThrottledReader` → `reqwest::Body::wrap_stream` / `ByteStream`) — o limite vale para multipart e resumable.
- Throughput medido (para UI) = bytes confirmados em janela deslizante de 5 s, por job e agregado por destino → evento `throughput` a cada 1 s.
- Night mode (Should): task de 1 min compara hora local com a janela e chama `set_limit(0)` / restaura.

### `health.rs`

- A cada 60 s: `HEAD https://www.googleapis.com/drive/v3/about?fields=user` (com token) e `head_bucket`; resultado → `AppStatus.destinations[dest].online` (+ `latency_ms`, Should). Falha 401/403 → `auth-required`.

### `queue.rs` / `worker.rs`

- Fila é a tabela `jobs`; em memória apenas `Notify` para acordar workers.
- `worker_loop(dest)`: `SELECT ... WHERE destination=? AND status='pending' AND (next_attempt_at IS NULL OR next_attempt_at <= now) ORDER BY created_at LIMIT 1` → marca `uploading` (transação) → upload → atualiza.
- Watcher pausado (RF12) não afeta o worker: `WatcherHandle::pause()` só descarta eventos do `notify`; o pool continua drenando `pending`. `resume_watcher` chama `rescan()` ao retomar (RF-005).
- `save_config` aplica em runtime: `workers_per_destination` redimensiona o pool (spawn/cancel de tasks ociosas) e `retry.max_attempts` / `retry.base_delay_seconds` são lidos da config compartilhada (`Arc<RwLock<AppConfig>>`) a cada tentativa — nada de "5" hardcoded (RF-086).
- `paused` (RF14, Should): `pause_job` dispara `cancel` no token do job e grava `status='paused'` mantendo `remote_state`; `resume_job` → `pending`. Worker ignora `paused`.
- No startup: `UPDATE jobs SET status='pending' WHERE status='uploading'` (crash recovery, RNF-006).
- Backoff: `base * 2^attempts` com jitter ±20%, cap 10 min.
- Pool: `workers_per_destination` tasks Tokio por destino.

### `power.rs`

- `keep_awake=true` → `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)` (não força tela ligada).
- Detectar resume: loop com `tokio::time::interval(60s)`; se `elapsed > 2×interval`, assume que dormiu → dispara `rescan()`.

`watch.stabilize_seconds` (1..60, validado) vira `StabilizeConfig.stable_reads` a cada geração de watcher — com `interval` de 1s, N leituras iguais consecutivas são N segundos de quietude. `min_size_mb`/`max_size_mb` aceitam `0` = filtro desligado (ADR-017); o teto duro é `config::MAX_FILE_SIZE_MB` (5 TiB).

**Salvar config reinicia o watcher sempre que `watch` muda** — comparação estrutural, não lista de campos. Uma lista apodrece a cada campo novo, e apodreceu: `max_size_mb` nunca esteve nela.

### `rescan()`

Duas passadas, nesta ordem — e a ordem é estrutural, não estética.

**1. Enfileirar.** Lista a pasta (respeitando filtros), para cada arquivo: se não está em `files` **ou** hash mudou → enfileira. Atalho `size`+`mtime` evita o hash quando a linha de `files` já bate.

**2. Reconciliar** (`reconcile`, RF-004). Reavalia toda linha de `files` que ainda tenha job `pending`/`paused`/`failed` — arquivada ou não. Um arquivo sai da fila (`archived_at = now`) quando é qualquer um de:

- **fora de escopo**: fora da raiz, ou aninhado com `recursive: false`. Escopo é calculado do caminho, não de "o walk viu": um arquivo que saiu de escopo é justamente um que o walk não visita mais, então existência em disco não distingue "fora de escopo" de "apagado".
- **sumiu do disco**: um `stat`, só para os candidatos que o walk não acabou de ver.
- **reprovado nos filtros**: `passes_filters` contra `files.size`, que a passada 1 já atualizou para tudo que mudou de tamanho. É por isso que a passada 2 não precisa de `stat` próprio — inverter a ordem obrigaria a um.

O caminho de volta é simétrico: um arquivo arquivado que volta a passar tem `archived_at = NULL`. Sem isso, afrouxar um filtro seria porta de mão única — o atalho `size`+`mtime` devolve `Unchanged` para um arquivo que não mudou em disco, então nada mais o traria de volta.

`uploading` nunca é tocado (transferência em voo) e `done`/`cancelled` também não (histórico, território do `clear_completed`). O status nunca muda: `archived_at` sozinho já esconde a linha de `list_jobs`, `status_counts`, `claim_next` e `retry_all_failed`.

Todo o I/O da passada 2 acontece fora do `with_repo`, para que a closure bloqueante que roda a transação nunca espere disco. A transação é a mesma para SELECT e UPDATE, e passa pelo mesmo `Mutex<Repo>` que o `claim_next` — então nenhuma linha vira `uploading` entre a leitura e a escrita.

Roda no startup, no resume, na bandeja, na volta de suspensão e por comando manual da UI — os cinco gatilhos, para não haver estado divergente conforme o disparo.

### `logging.rs`

`tracing` com dois layers: arquivo JSON rotativo em `%APPDATA%/osystems-sync/logs/app.log` (10 MB × 5, caminho absoluto) e canal broadcast para a UI (últimas 500 linhas em ring buffer; `src-tauri/src/events.rs` assina o broadcast e emite `log-line`).

---

## 7. IPC (Tauri commands e events)

Todos os tipos anotados com `#[derive(Serialize, Deserialize, TS)] #[ts(export)]` → `src/types/generated.ts`.

### Commands

| Command | Args | Retorno | Notas |
|---------|------|---------|-------|
| `get_config` | — | `AppConfig` | |
| `save_config` | `AppConfig` | `()` | valida; reinicia watcher se `watch.path` mudou |
| `pick_folder` | — | `string \| null` | via plugin-dialog |
| `pick_service_account_file` | — | `ServiceAccountInfo \| null` | dialog filtro `*.json`; lê, valida (`type == service_account`, `client_email`, `private_key`), grava no keyring, devolve `{ file_name, size, client_email, project_id }` — **nunca o conteúdo** |
| `set_credential` | `{ key, value }` | `()` | grava no keyring |
| `get_credential_status` | — | `CredentialStatus` | `{ aws: { present, masked }, gdrive: { present, email? } }` |
| `clear_credential` | `{ key }` | `()` | |
| `test_connection` | `{ destination }` | `TestResult` | `{ ok, message, latency_ms }` |
| `get_status` | — | `AppStatus` | watcher ativo/pausado, destinos `{ online, auth_required, latency_ms? }`, contadores por status, `core_version`, `build_target` |
| `list_jobs` | `{ statuses?: JobStatus[], destination?, include_archived?, limit, offset }` | `{ items: JobView[], total }` | join com `files`; `JobView` agrega os 2 jobs do arquivo numa linha (`gdrive`, `s3`) |
| `pause_watcher` / `resume_watcher` | — | `()` | RF12 |
| `pause_job` / `resume_job` | `{ job_id }` | `()` | RF14 (Should) |
| `clear_completed` | — | `u32` | seta `archived_at` em todos `done` visíveis (RF13) |
| `set_qos` | `{ destination, limit_mbps: number \| null }` | `()` | atalho de `save_config` só para o throttle; aplica em ≤ 2 s |
| `open_in_explorer` | `{ path }` | `()` | `revealItemInDir(path)` do `tauri-plugin-opener` (permissão `opener:allow-reveal-item-in-dir`) |
| `open_remote` | `{ job_id }` | `()` | abre `webViewLink` (Drive) ou URL do console S3 |
| `retry_job` | `{ job_id }` | `()` | reset attempts, status=pending |
| `retry_all_failed` | — | `u32` | |
| `cancel_job` | `{ job_id }` | `()` | |
| `rescan` | — | `RescanReport` | reconciliação: `enqueued`, `archived`, `restored` (+ contadores de diagnóstico) |
| `get_recent_logs` | `{ limit }` | `LogLine[]` | |
| `open_logs_folder` | — | `()` | |
| `set_autostart` | `{ enabled }` | `()` | |

### Events (main → renderer)

| Event | Payload |
|-------|---------|
| `status-changed` | `AppStatus` |
| `job-updated` | `JobView` |
| `upload-progress` | `{ job_id, sent, total, rate_bps }` (throttle 500 ms) |
| `throughput` | `{ total_bps, gdrive_bps, s3_bps, limit_gdrive_bps?, limit_s3_bps? }` (1 s) |
| `log-line` | `LogLine` `{ ts, level, target, job_id?, destination?, message }` |
| `auth-required` | `{ destination, hint }` (`hint` = e-mail da SA para compartilhar / política IAM faltante) |

Erros dos commands: retornar `Result<T, AppError>` onde `AppError { code: string, message: string }` serializável.

---

## 8. UI

**Antes de criar qualquer tela, ler `design/DESIGN.md`** e consumir `design/tokens.css` via `@theme` do Tailwind v4 (`src/styles/app.css`). Em caso de conflito entre este documento e o design, o design vence no visual e este documento vence no comportamento. Ícones: `lucide-react` (SVG inline) — o mockup usa Material Symbols via CDN, bloqueado pela CSP.

### Shell permanente

- **Title bar** 36 px (`decorations: false`, `data-tauri-drag-region`): logo + nome, chip da pasta, badge `Daemon: Active`, min/max/close.
- **Sidebar** 256 px: navegação (2 itens + badge de contagem), card da pasta monitorada (`Watcher: Ativo/Pausado`, Alterar pasta), widget de throughput (evento `throughput`).
- **Statusbar** 28 px: `Rust Core vX.Y.Z`, `GDrive: Online/Offline/Auth`, `AWS S3: Online/Offline/Auth (region)`, (`Ping: N ms` — Should), build target.

### Telas (2 rotas — decisão C3)

**Dashboard & Fila** (`/dashboard`)
KPIs (total detectados + bytes, concluídos + %, em transferência, na fila, falhas) · botões Atualizar lista / Pausar-Retomar watcher / Limpar concluídos · **tabela dupla** (Arquivo & Origem, Tamanho, Google Drive [barra+%+MB/s], AWS S3 [idem], Status agregado, Ações) com filtro de status (Todos/Ativos/Concluídos/Falhas) e paginação de 50 — substitui Queue + History · **console de eventos** colapsável (ring de 500 linhas, filtro por nível, abrir pasta de logs) — substitui Logs · estados vazios: sem pasta, sem credenciais, `auth-required`.

**Configurações & QoS** (`/settings`)
Módulo **Google Drive** (status, JSON da SA via `pick_service_account_file` — mostra nome/tamanho/`client_email`, Folder ID com copiar/abrir, toggles subpastas por data [Should] e checksum pré-upload [sempre on], rodapé com teste) · Módulo **AWS S3** (Access Key, Region, Secret com olho pré-save, Bucket, Prefix, Storage Class, rodapé com teste) · Módulo **QoS** (2 sliders 0,5–10 MB/s + Ilimitado, toggle Modo Noturno [Should, desabilitado]) · Seção **Geral** (autostart, keep awake, workers 1–4, tentativas 1–10, filtros do watcher) · rodapé fixo: "Última alteração salva às", Cancelar/Restaurar padrões (nunca apaga credenciais), Salvar Preferências (`Ctrl+S`, aplica em runtime).

### Tray

Ícone com 3 estados (ok / trabalhando / erro). Menu: Abrir, Pausar/Retomar, Rescan, Sair. Fechar janela → `hide()`; Sair → shutdown gracioso (aguarda uploads em andamento até 30 s ou cancela).

---

## 9. Segurança

- `capabilities/default.json` mínimo: apenas commands listados, `dialog:allow-open`, `opener:allow-reveal-item-in-dir` (Explorer com item selecionado), `opener:allow-open-path` (pasta de logs) e `opener:allow-open-url` restrito a `https://drive.google.com/*` e `https://*.console.aws.amazon.com/*`. **CSP fica em `tauri.conf.json → app.security.csp`** (capabilities só carregam permissões).
- CSP no `tauri.conf.json`: `default-src 'self'; connect-src ipc: http://ipc.localhost; font-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'` (Tailwind v4 injeta estilos em dev). Nenhum recurso de CDN (RNF-014).
- JSON da Service Account: lido uma vez pelo command, validado, gravado no keyring, **nunca** copiado para `%APPDATA%`; o caminho original não é logado.
- Renderer nunca recebe secret; `get_credential_status` devolve só máscara.
- Permissão IAM mínima para S3: `s3:PutObject`, `s3:GetObject`, `s3:ListBucket`, `s3:AbortMultipartUpload`, `s3:ListMultipartUploadParts` no bucket alvo.
- Escopo Google: `https://www.googleapis.com/auth/drive` — a SA precisa escrever numa pasta que **não** criou (compartilhada com ela), o que `drive.file` não permite. Mitigação: a SA só tem acesso ao que for explicitamente compartilhado com seu e-mail.
- `s3:DeleteObject` é **opcional**: só usado para limpar o probe de `test_connection` (`{prefix}.osystems-sync-probe`); sem ela o teste continua `ok` e informa que o arquivo de teste (0 B) permanece no bucket. Políticas write-only/append-only (comuns em buckets de backup) são suportadas.
- **T-6.3 (VULN-004)** `worker.rs` faz `abort()` best-effort ao desistir de um job (`Permanent` ou esgotamento de retries) que tinha `remote_state.upload_id`, mas isso é best-effort — se a chamada falhar ou o processo cair antes dela, a parte já enviada fica órfã no bucket, cobrando armazenamento indefinidamente. Backstop recomendado: regra de lifecycle no bucket S3, `AbortIncompleteMultipartUpload` com `DaysAfterInitiation: 1`, para que qualquer upload multipart nunca completado/abortado seja limpo automaticamente pela AWS em até 1 dia.

---

## 10. Fases de implementação

Cada fase termina com `cargo test`, `cargo clippy -- -D warnings`, `npm run build` e critério de aceite verificado manualmente.

| Fase | Entrega | Aceite |
|------|---------|--------|
| 0 | Scaffold Tauri 2 + React + Tailwind v4 + workspace com `crates/core`; `design/tokens.css` → `@theme`; fontes vendorizadas; shell (title bar custom, sidebar, statusbar) com dados mock; tray; autostart; CI (`cargo test`, `clippy -D warnings`, `npm run build`, `vitest`). | App abre com o shell idêntico ao mockup, minimiza p/ tray, inicia com Windows. |
| 1 | `config`, `state` (SQLite WAL + migrations), `logging` (arquivo + ring 500), `ts-rs`, commands `get/save_config`, tela **Configurações** (Geral + campos S3/Drive sem credenciais) com `Ctrl+S`. | Config persiste; logs em arquivo; salvar aplica sem reiniciar. |
| 2 | `watcher` + `stabilize` + `hash` + `rescan` + pausar/retomar watcher; jobs em `pending`; **Dashboard** com tabela dupla (só leitura), KPIs, filtro/paginação, console de log ao vivo. | Copiar arquivo grande → aparece só após estabilizar; reinício não duplica; pausar watcher ignora novos arquivos. |
| 3 | `credentials` (keyring) + `uploaders/s3` (put/multipart/abort, idempotência) + `worker` + retry/backoff + classificação de erro + **`throttle`** + `health`; Testar bucket; ações reenviar/cancelar/limpar concluídos/abrir no Explorer; progresso + throughput na UI; slider QoS S3. | Arquivo chega no bucket; rede off → backoff → on → `done`; teto 2,5 MB/s respeitado ±10 %; matar processo → jobs retomam. |
| 4 | `gdrive/auth` (**Service Account**) + `gdrive/upload` (multipart/resumable com retomada) + idempotência `sha256Checksum` + `pick_service_account_file` + Testar conexão + slider QoS Drive. | JSON válido → `Conectado`; arquivo 100 MB chega no Drive; queda no chunk 3 retoma; token renova sozinho após 1 h; pasta não compartilhada → `auth-required` com e-mail da SA. |
| 5 | `power` (keep awake, resume→rescan), shutdown gracioso, estados vazios/`auth-required`, **Should-haves**: pausar job, subpastas por data, modo noturno, ping na statusbar, notificações nativas. | Suspender/retomar → pendentes processados; Sair com upload em voo encerra ≤ 30 s e retoma ao reabrir. |
| 6 | Hardening: teste de 72 h com 500 arquivos, medir RSS; a11y (axe) e contraste; instalador NSIS/MSI assinado; auto-update (opcional). | RSS estável (±10 %), zero jobs perdidos, zero duplicatas remotas. |

---

## 11. Testes

- **Unit (core)**: `stabilize` com arquivo escrito em chunks; `hash`; backoff; classificação de erros; repo SQLite (in-memory).
- **Integração**: S3 contra **LocalStack** ou **MinIO** (docker); Drive com mock HTTP (`wiremock`) cobrindo token JWT da SA, resumable com falha no meio e 403 de pasta não compartilhada.
- **Throttle**: teste determinístico com `tokio::time::pause()` — 10 MB a 1 MB/s leva 10 s virtuais; 2 readers concorrentes somam o teto.
- **Watcher**: teste real em `tempdir` com `notify`.
- **UI**: Vitest para `api/ipc.ts` com `@tauri-apps/api/mocks`.
- **Longa duração**: script que gera 1 arquivo/minuto por 72 h; verificar contagem no S3/Drive = contagem local.

---

## 12. Decisões registradas

| Decisão | Motivo |
|---------|--------|
| App na tray, não serviço Windows | Requisito é usuário logado por 10 dias; serviço adiciona instalação e IPC sem ganho. Core isolado permite migrar depois via crate `windows-service`. |
| SQLite como fila | Sobrevive a reinício, consulta simples, sem dependência externa. |
| Drive via REST + `yup-oauth2` | Sem SDK oficial Rust; REST v3 é estável e bem documentada. |
| `keyring` em vez de `stronghold` | Integra com Credential Manager nativo; menos código; suficiente para 5 segredos. |
| **C5** Hash SHA-256 completo, único checksum do pipeline | Idempotência confiável; custo aceitável (arquivos são lidos de qualquer forma para upload). Drive compara `sha256Checksum`; S3 usa `x-amz-meta-sha256`. BLAKE3 do mockup é decorativo. |
| **C1** Drive via **Service Account**, não OAuth | 10+ dias sem intervenção: refresh token pode expirar/ser revogado; SA não tem browser nem consentimento. API Key simples não escreve no Drive. Custo: exige pasta compartilhada com a SA (ou Shared Drive) e escopo `drive`. Fallback OAuth *installed app* (v1) fica documentado, não implementado. |
| **C2** Nome `oSystems Sync`; ids `com.osystems.sync` / `osystems-sync` | Identidade dos assets v2. |
| **C3** 2 telas (Dashboard & Fila, Configurações & QoS) | Mockups aprovados; History = filtro+paginação; Logs = console embutido + pasta de logs. Menos rotas, menos IPC. |
| **C4** `throttle` no MVP; modo noturno Should | Preservar link corporativo é requisito do admin; agenda é lógica de relógio separável. |
| **C6** subpastas por data `YYYY/MM/DD_backup/` são Should | Toggle presente no mockup; não bloqueia o fluxo principal. |
| **C7** pausar job individual (`paused`) é Should; `paused` e `cancelled` entram no CHECK desde a migração 1 | Ícone presente no mockup; evita migração de schema depois. |
| **C8** Pausar = só watcher | Doc funcional v2 explícito; workers e watcher têm handles independentes. |
| **C9** Inter + JetBrains Mono vendorizadas | Mockups + doc v2; CSP `'self'` proíbe Google Fonts. |
| **C10** Paleta do `tailwind-config` do mockup | Única que bate com os PNGs aprovados; DESIGN.md tinha 2 paletas internas divergentes. |
| **C11** Ping na statusbar é Should | Exige health-check com medição; online/offline já basta para o MVP. |
| `tauri-plugin-opener` em vez de `plugin-shell` | Sem OAuth não há URL para abrir em browser genérico; `opener` cobre Explorer `/select`, pasta de logs e links remotos com allowlist. |
| `lucide-react` em vez de Material Symbols | Ícones SVG inline, sem CDN, tree-shaken. Mapeamento 1:1 em `design/DESIGN.md`. |
| Escopo Google `drive` (não `drive.file`) | SA precisa escrever em pasta compartilhada que não criou. |
| **Fase 0** `osystems-sync-core = { path = "crates/core" }` sem alias `core` | Aliasar a dependência como `core` sombreia o crate built-in `::core` e quebra `tauri::generate_context!()` (`E0433`). Importa-se como `osystems_sync_core`. |
| **Fase 0** `@theme inline` (Tailwind v4) em vez de `@theme` | O mapeamento re-declara cada variável com o mesmo nome do token (`--color-surface-0: var(--color-surface-0)`); com `@theme` puro vira auto-referência inválida. `inline` é o padrão documentado do Tailwind para esse caso. |
| **Fase 0** `JetBrainsMono[wght].woff2` convertido localmente (fontTools) | Upstream (v2.304) não publica WOFF2 variável, só TTF variável + WOFF2 estáticos por peso. Conversão lossless, eixo `wght 100–800` preservado; OFL incluída em `public/fonts/`. |
| **Fase 0** `println!` provisório em `lib.rs`/`tray.rs` | `tracing` entra em T-1.3 (Fase 1); remover os `println!` faz parte do done-when de T-1.3. |
| **Fase 0** `tauri` feature `image-png`; `tauri-plugin-autostart` já na Fase 0 | `Image::from_bytes` exige a feature para decodificar os PNGs do tray; autostart é RF-092 (T-0.8). |
| **Fase 0** `capabilities/default.json` ganhou `core:window:allow-{minimize,toggle-maximize,close,start-dragging,is-maximized,show,hide}`, `core:tray:default`, `autostart:default` | Title bar customizada (`decorations:false`) precisa dos controles de janela via IPC; tray e autostart pelos plugins. Continua sem `dialog`/`opener` até as fases 2–3. |
| **Release** TLS do AWS SDK = `rustls` + `ring` (`aws-smithy-http-client` `rustls-ring`; `aws-config`/`aws-sdk-s3` sem default features) | O default do SDK puxa `aws-lc-sys` (C + cmake + nasm), que inviabiliza cross-compile `x86_64-pc-windows-msvc` via `cargo-xwin` a partir do macOS e engorda o build. `ring` já era a base de `reqwest`/`yup-oauth2`; árvore fica com um único provedor criptográfico. |
| **Fase 1** Inteiros IPC: config em `u32`; `u64` de tamanho/contagem com `#[ts(type = "number")]` | `ts-rs` gera `bigint` para `u64`, que não serializa em JSON nem casa com `<input type=number>`. `Number.MAX_SAFE_INTEGER` (9 PB) cobre 5 GB com folga. |
| **Fase 0** Máquina de dev é macOS | Toolchain instalada via rustup (1.98.1, `--no-modify-path`); crates Windows gated com `cfg(windows)`; critérios de aceite que dependem de Windows (tray/autostart/suspensão/Credential Manager) ficam para CI `windows-latest` ou VM. |
| **Fase 6** NSIS `installMode: currentUser` + hook `NSIS_HOOK_PREUNINSTALL` | Sem exigir Administrador no MVP (RNF-016), consistente com `keyring`/Credential Manager por usuário e com `tauri-plugin-autostart` escrevendo em `HKCU`. Custo: `installMode: perMachine`/`both` fica fora de escopo — se um deploy multiusuário exigir instalação por máquina, revisitar junto com a migração de `HKCU` para `HKLM`. O hook mata o processo (`taskkill /IM osystems-sync.exe /F`) e apaga o valor de autostart em `HKCU\...\Run` (e seu espelho em `...\Explorer\StartupApproved\Run`) antes da desinstalação — sem isso a entrada de login sobrevive à desinstalação (RNF-016: "autostart removido na desinstalação"). |
| **Fase 5** `tauri-plugin-notification` para "Falha no upload"/"Autenticação necessária" (T-5.7) | Nativa por plataforma (Notification Center/Toast) via `NotificationExt`, sem depender do frontend estar em foco; disparada do lado Rust (`events.rs`), sem pacote npm correspondente (nenhum código JS chama a API de notificação diretamente). Rate-limit de 1/60s por tipo (`failed`, `auth`) evita flood em rajadas de falha. Permissão `notification:default` adicionada a `capabilities/default.json`. |
| **Fase 6** `axe-core` + `vitest-axe` (dev) | Auditoria de acessibilidade automatizada (RNF-013, T-6.2): `axe-core` cobre as regras WCAG que uma leitura manual do JSX facilmente deixa passar (nome acessível de progressbar, ordem de headings, `aria-expanded`/foco em menus e diálogos) e roda como parte da suíte Vitest existente, sem browser real. `color-contrast` é a única regra desabilitada (jsdom não calcula layout/cor computada) — contraste é verificado à parte por `src/styles/contrast.test.ts`, que recalcula os pares do DESIGN.md §10 diretamente dos hex de `design/tokens.css`. Dependências apenas de dev; `src-tauri` não é afetado. |
| **T-6.3 (VULN-002)** Release gate: `release.yml` falha em tag `v*` sem assinatura configurada | Um instalador Windows não assinado distribuído por engano (tag `v*` sem `certificateThumbprint`/`signCommand` em `tauri.conf.json`) é indistinguível de malware para o usuário final e para o SmartScreen. Novo step "Verify installer will be signed" roda antes do build em push de tag e falha o job (`::error::`) se ambos os campos estiverem nulos; `workflow_dispatch` manual fica isento (testes locais com certificado de dev). Break-glass: variável de repositório `ALLOW_UNSIGNED=true` permite um release de emergência sem certificado, deixando `::warning::` no log como rastro. Detalhe em `src-tauri/RELEASE.md §2`. |
