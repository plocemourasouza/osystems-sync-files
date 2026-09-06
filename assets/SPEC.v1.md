# FolderSync — Especificação Técnica

Aplicação desktop (Windows) que monitora uma pasta local e envia cada arquivo novo para uma pasta no Google Drive e para um bucket no AWS S3. Roda na bandeja do sistema por longos períodos (10+ dias) sem intervenção.

Stack: **Tauri 2 + Rust** (core) e **React + TypeScript + Vite** (UI).

---

## 1. Requisitos

### Funcionais

| ID | Requisito |
|----|-----------|
| RF01 | Usuário configura: pasta monitorada, credenciais AWS, bucket/prefixo S3, credenciais Google, ID da pasta no Drive. |
| RF02 | Ao surgir um arquivo novo na pasta, enviar para S3 e Drive de forma independente. |
| RF03 | Cada destino tem retry com backoff exponencial (máx. 5 tentativas, depois marca `failed`). |
| RF04 | Nunca enviar o mesmo arquivo duas vezes ao mesmo destino (chave: caminho + SHA-256). |
| RF05 | Ao iniciar ou retornar de suspensão, varrer a pasta e enfileirar o que ainda não foi enviado. |
| RF06 | UI mostra: status dos destinos, fila atual, histórico, erros, logs. |
| RF07 | Botão "Testar conexão" para cada destino. |
| RF08 | Botão "Reenviar" para itens `failed`. |
| RF09 | Iniciar com o Windows; fechar janela minimiza para tray. |
| RF10 | Filtros opcionais: extensões permitidas, ignorar subpastas, tamanho máximo. |

### Não funcionais

| ID | Requisito |
|----|-----------|
| RNF01 | Uptime contínuo de 10+ dias sem crescimento de memória perceptível. |
| RNF02 | Credenciais nunca em texto plano em disco; usar `keyring` (Windows Credential Manager). |
| RNF03 | Renderer (UI) nunca recebe credenciais completas; só status e máscara (`AKIA****XYZ`). |
| RNF04 | Arquivos até 5 GB; usar upload multipart (S3) e resumable (Drive) acima de 8 MB. |
| RNF05 | Logs rotacionados (max 10 MB × 5 arquivos). |
| RNF06 | Estado persistido em SQLite; reinício não perde fila. |

Fora de escopo v1: sincronização bidirecional, exclusão remota, múltiplas pastas, monitoramento sem usuário logado (serviço Windows).

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
  → uploader.upload() → Ok  → state.mark_done(file_id, dest, remote_id)
                     → Err → state.mark_retry(attempt+1, next_at = now + 2^attempt * 5s)
                              attempt >= 5 → state.mark_failed(error)
  → emit event "job-updated" para o renderer
```

---

## 3. Estrutura do projeto

```
foldersync/
├── CLAUDE.md
├── SPEC.md                      # este arquivo
├── design/                      # ← arquivos de layout/design já existentes
│   └── ...                      #   (Claude Code deve ler antes de criar telas)
├── package.json
├── vite.config.ts
├── src/                         # renderer (React + TS)
│   ├── main.tsx
│   ├── App.tsx
│   ├── api/
│   │   └── ipc.ts               # wrappers tipados de invoke()/listen()
│   ├── pages/
│   │   ├── Dashboard.tsx
│   │   ├── Settings.tsx
│   │   ├── Queue.tsx
│   │   ├── History.tsx
│   │   └── Logs.tsx
│   ├── components/
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
│               ├── state/
│               │   ├── mod.rs
│               │   ├── schema.sql
│               │   └── repo.rs
│               ├── uploaders/
│               │   ├── mod.rs   # trait Uploader
│               │   ├── s3.rs
│               │   └── gdrive/
│               │       ├── mod.rs
│               │       ├── auth.rs
│               │       └── upload.rs
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
yup-oauth2 = "11"
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
tauri-plugin-shell = "2"       # abrir URL OAuth no browser
tauri-plugin-notification = "2"
core = { path = "crates/core" }
```

### Frontend

```
react, react-dom, typescript, vite
@tauri-apps/api, @tauri-apps/plugin-dialog, @tauri-apps/plugin-shell
zustand, react-router-dom
tailwindcss (ou o que o design em /design indicar)
```

---

## 5. Modelo de dados

### `config.json` (em `%APPDATA%/foldersync/`, sem segredos)

```json
{
  "version": 1,
  "watch": {
    "path": "C:\\Users\\x\\Exportacoes",
    "recursive": false,
    "extensions": [],
    "max_size_mb": 5000,
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
    "auth_mode": "oauth"
  },
  "retry": { "max_attempts": 5, "base_delay_seconds": 5 },
  "workers_per_destination": 2,
  "autostart": true,
  "keep_awake": true
}
```

### Credenciais (via `keyring`, service = `foldersync`)

| chave | conteúdo |
|-------|----------|
| `aws.access_key_id` | string |
| `aws.secret_access_key` | string |
| `gdrive.client_id` | string |
| `gdrive.client_secret` | string |
| `gdrive.refresh_token` | string (gerado no fluxo OAuth) |

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
  status      TEXT NOT NULL CHECK (status IN ('pending','uploading','done','failed')),
  attempts    INTEGER NOT NULL DEFAULT 0,
  next_attempt_at TEXT,
  remote_id   TEXT,                       -- S3 key ou Drive fileId
  last_error  TEXT,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  UNIQUE (file_id, destination)
);

CREATE INDEX idx_jobs_status ON jobs(status, next_attempt_at);

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

Classificação de erros: `Transient` = timeout, 5xx, 429, rede. `Auth` = 401/403 → marca destino como "precisa reautenticar" e pausa jobs desse destino. `Permanent` = demais 4xx → `failed` imediato.

### `s3.rs`

- `aws_sdk_s3::Client` com credenciais estáticas do keyring.
- `< 8 MB`: `put_object`. `>= 8 MB`: multipart, parte de 16 MB, 4 partes concorrentes.
- Key final: `{prefix}{remote_name}`. Metadata `x-amz-meta-sha256`.
- `test_connection`: `head_bucket`.
- Idempotência: antes de enviar, `head_object`; se existe com mesmo sha256 na metadata, retorna `Ok` sem reenviar.

### `gdrive/`

- `auth.rs`: fluxo OAuth 2.0 *installed app* com `yup-oauth2` (`InstalledFlowAuthenticator`, redirect em `localhost` porta efêmera). Persiste `refresh_token` no keyring. Renovação automática de access token.
- `upload.rs`: API REST v3.
  - `< 8 MB`: `POST /upload/drive/v3/files?uploadType=multipart`.
  - `>= 8 MB`: `uploadType=resumable`, chunks de 16 MB, retomar via `Content-Range` em caso de falha transitória.
  - Body metadata: `{ "name", "parents": [folder_id] }`.
  - `supportsAllDrives=true` para pastas compartilhadas.
- Idempotência: `files.list` com `q="name='X' and 'folder_id' in parents and trashed=false"` + comparação de `md5Checksum`/`sha256Checksum` (campo `fields`).
- `test_connection`: `GET /drive/v3/files/{folder_id}?fields=id,name`.

### `watcher.rs` + `stabilize.rs`

```rust
pub fn spawn_watcher(cfg: WatchConfig, tx: mpsc::Sender<PathBuf>) -> Result<WatcherHandle>;
pub async fn wait_until_stable(path: &Path, stable_secs: u64, timeout: Duration) -> Result<u64 /*size*/>;
```

`wait_until_stable`: a cada 1s lê `metadata().len()`; considera estável após `stable_secs` leituras iguais consecutivas **e** `OpenOptions::new().read(true).share_mode(0)` (Windows) bem-sucedido. Timeout 30 min → log + tenta mesmo assim.

Ignorar: arquivos temporários (`~$*`, `*.tmp`, `*.crdownload`, `*.part`, ocultos) e diretórios.

### `queue.rs` / `worker.rs`

- Fila é a tabela `jobs`; em memória apenas `Notify` para acordar workers.
- `worker_loop(dest)`: `SELECT ... WHERE destination=? AND status='pending' AND (next_attempt_at IS NULL OR next_attempt_at <= now) ORDER BY created_at LIMIT 1` → marca `uploading` (transação) → upload → atualiza.
- Backoff: `base * 2^attempts` com jitter ±20%, cap 10 min.
- Pool: `workers_per_destination` tasks Tokio por destino.

### `power.rs`

- `keep_awake=true` → `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)` (não força tela ligada).
- Detectar resume: loop com `tokio::time::interval(60s)`; se `elapsed > 2×interval`, assume que dormiu → dispara `rescan()`.

### `rescan()`

Lista a pasta (respeitando filtros), para cada arquivo: se não está em `files` **ou** hash mudou → enfileira. Roda no startup, no resume, e por comando manual da UI.

### `logging.rs`

`tracing` com dois layers: arquivo JSON rotativo (`logs/app.log`) e canal broadcast para a UI (últimas 500 linhas em ring buffer).

---

## 7. IPC (Tauri commands e events)

Todos os tipos anotados com `#[derive(Serialize, Deserialize, TS)] #[ts(export)]` → `src/types/generated.ts`.

### Commands

| Command | Args | Retorno | Notas |
|---------|------|---------|-------|
| `get_config` | — | `AppConfig` | |
| `save_config` | `AppConfig` | `()` | valida; reinicia watcher se `watch.path` mudou |
| `pick_folder` | — | `string \| null` | via plugin-dialog |
| `set_credential` | `{ key, value }` | `()` | grava no keyring |
| `get_credential_status` | — | `CredentialStatus` | `{ aws: { present, masked }, gdrive: { present, email? } }` |
| `clear_credential` | `{ key }` | `()` | |
| `gdrive_start_oauth` | — | `()` | abre browser, aguarda callback, salva refresh token |
| `test_connection` | `{ destination }` | `TestResult` | `{ ok, message, latency_ms }` |
| `get_status` | — | `AppStatus` | watcher ativo, destinos ok/erro, contadores por status |
| `list_jobs` | `{ status?, limit, offset }` | `JobView[]` | join com `files` |
| `retry_job` | `{ job_id }` | `()` | reset attempts, status=pending |
| `retry_all_failed` | — | `u32` | |
| `cancel_job` | `{ job_id }` | `()` | |
| `rescan` | — | `u32` | quantidade enfileirada |
| `get_recent_logs` | `{ limit }` | `LogLine[]` | |
| `open_logs_folder` | — | `()` | |
| `set_autostart` | `{ enabled }` | `()` | |

### Events (main → renderer)

| Event | Payload |
|-------|---------|
| `status-changed` | `AppStatus` |
| `job-updated` | `JobView` |
| `upload-progress` | `{ job_id, sent, total }` (throttle 500 ms) |
| `log-line` | `LogLine` |
| `auth-required` | `{ destination }` |

Erros dos commands: retornar `Result<T, AppError>` onde `AppError { code: string, message: string }` serializável.

---

## 8. UI

**Antes de criar qualquer tela, ler `/design/`** e seguir cores, tipografia, espaçamentos e componentes definidos lá. Se houver tokens, exportar para `src/styles/tokens.css`. Em caso de conflito entre este documento e o design, o design vence no visual e este documento vence no comportamento.

### Telas

**Dashboard**
Estado do watcher (ativo/pausado + pasta), cards S3 e Drive (ok / erro / precisa autenticar), contadores (pendentes, enviando, concluídos hoje, falhas), últimos 10 eventos. Botões: Pausar/Retomar, Rescan.

**Settings**
Seções: Pasta monitorada (picker, recursivo, extensões, tamanho máx.), AWS S3 (access key, secret — campos password, region, bucket, prefixo, Testar), Google Drive (client id/secret, botão "Conectar conta Google", folder id, Testar), Geral (autostart, keep awake, workers, retries). Salvar aplica sem reiniciar o app.

**Queue**
Tabela de jobs `pending`/`uploading`/`failed` com progresso, tentativa, próximo retry, erro. Ações: Reenviar, Cancelar, Reenviar todos com falha.

**History**
Jobs `done`, paginado, filtro por destino/data, link para abrir remoto (S3 console URL / Drive `webViewLink`).

**Logs**
Stream ao vivo (`log-line`), filtro por nível, botão abrir pasta de logs.

### Tray

Ícone com 3 estados (ok / trabalhando / erro). Menu: Abrir, Pausar/Retomar, Rescan, Sair. Fechar janela → `hide()`; Sair → shutdown gracioso (aguarda uploads em andamento até 30 s ou cancela).

---

## 9. Segurança

- `capabilities/default.json` mínimo: apenas commands listados, `dialog:allow-open`, `shell:allow-open` restrito a `https://accounts.google.com/*`.
- CSP no `tauri.conf.json`: `default-src 'self'; connect-src ipc: http://ipc.localhost`.
- Renderer nunca recebe secret; `get_credential_status` devolve só máscara.
- Permissão IAM mínima para S3: `s3:PutObject`, `s3:GetObject`, `s3:ListBucket`, `s3:AbortMultipartUpload`, `s3:ListMultipartUploadParts` no bucket alvo.
- Escopo Google: `https://www.googleapis.com/auth/drive.file` (só arquivos criados pelo app).

---

## 10. Fases de implementação

Cada fase termina com `cargo test`, `cargo clippy -- -D warnings`, `npm run build` e critério de aceite verificado manualmente.

| Fase | Entrega | Aceite |
|------|---------|--------|
| 0 | Scaffold Tauri 2 + React + workspace com `crates/core`; tray; autostart; tokens de design importados. | App abre, minimiza p/ tray, inicia com Windows. |
| 1 | `config`, `state` (SQLite + migrations), `logging`, commands `get/save_config`, tela Settings (sem credenciais). | Config persiste; logs em arquivo. |
| 2 | `watcher` + `stabilize` + `hash` + `rescan`; jobs criados em `pending`; tela Queue (somente leitura). | Copiar arquivo grande → aparece na fila só após estabilizar; reinício não duplica. |
| 3 | `credentials` (keyring) + `uploaders/s3` + `worker` + retry; Testar conexão; Dashboard. | Arquivo chega no bucket; desligar rede → retry com backoff → reconectar → `done`. |
| 4 | `gdrive/auth` (OAuth) + `gdrive/upload` (multipart/resumable). | Fluxo OAuth completo; arquivo 100 MB chega no Drive; token renova sozinho após 1 h. |
| 5 | `power` (keep awake, resume→rescan), History, Logs ao vivo, progresso, notificações de falha. | Suspender/retomar máquina → pendentes processados. |
| 6 | Hardening: teste de 72 h com 500 arquivos, medir RSS; instalador NSIS/MSI; auto-update (opcional). | RSS estável (±10 %), zero jobs perdidos. |

---

## 11. Testes

- **Unit (core)**: `stabilize` com arquivo escrito em chunks; `hash`; backoff; classificação de erros; repo SQLite (in-memory).
- **Integração**: S3 contra **LocalStack** ou **MinIO** (docker); Drive com mock HTTP (`wiremock`) cobrindo resumable com falha no meio.
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
| Hash SHA-256 completo | Idempotência confiável; custo aceitável (arquivos são lidos de qualquer forma para upload). |
