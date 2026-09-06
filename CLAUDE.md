# CLAUDE.md — oSystems Sync

App desktop Tauri 2 (Rust) + React/TS que monitora uma pasta no Windows e envia arquivos novos para Google Drive (Service Account) e S3. Especificação completa em **`PRD.md`** (o quê) → **`SPEC.md`** (como) → **`design/DESIGN.md`** + **`design/tokens.css`** (aparência) → **`PLAN.md`** (fases/waves). **Leia `PRD.md` e `SPEC.md` antes de qualquer tarefa; leia `design/DESIGN.md` + `design/tokens.css` antes de criar telas.**

## Fontes de verdade

1. **`PRD.md`**: escopo e requisitos (RF-xxx). Conflito de escopo vence aqui.
2. **`SPEC.md`**: arquitetura, contratos, modelo de dados. Conflito de comportamento vence aqui.
3. **`design/DESIGN.md` + `design/tokens.css`**: layout, cores, tipografia, componentes. Conflito visual vence aqui.
4. **`PLAN.md`**: ordem de execução por fase/wave; `osforge-db` é o tracker.

Se ambíguo, pergunte.

## Regras de arquitetura

- `src-tauri/crates/core` **não importa `tauri`**. Toda lógica de negócio (watcher, fila, uploaders, estado, credenciais) fica lá. `src-tauri/src` só adapta (commands, events, tray).
- **`crates/core` expõe um `Throttle` por destino** — uploaders **DEVEM** envolver seu body stream com ele.
- `paused` é status válido de job; pausar job não afeta outros jobs nem o watcher.
- Google Drive: **Service Account only** (sem OAuth code paths); sem browser, sem redirect_uri.
- Usar `tauri-plugin-opener` (não `plugin-shell`).
- Renderer nunca recebe segredos. Credenciais entram pelo command `set_credential` e saem só como máscara.
- Toda struct trafegada por IPC usa `#[derive(Serialize, Deserialize, ts_rs::TS)] #[ts(export)]`. Rode `cargo test` para regenerar `src/types/generated.ts`; não edite esse arquivo à mão.
- Erros: `thiserror` no core; commands retornam `Result<T, AppError>`.
- Async: Tokio. Nada de `std::thread` no core, exceto onde `notify` exigir.
- Logs: `tracing`, nunca `println!`.

## Comandos

```bash
npm install && npm run tauri dev          # dev
npm run tauri build                       # instalador
cargo test --workspace --manifest-path src-tauri/Cargo.toml
cargo clippy --workspace --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo fmt --all --manifest-path src-tauri/Cargo.toml
npm run lint && npm run typecheck
```

Integração S3 local: `docker run -p 9000:9000 minio/minio server /data` e configure `endpoint_url` via env `OSYSTEMS_SYNC_S3_ENDPOINT`.

## Convenções

- Rust: `snake_case`, módulos pequenos, um `pub` por responsabilidade. Sem `unwrap()` fora de testes.
- TS: componentes funcionais, `zustand` para estado global, wrappers de IPC em `src/api/ipc.ts` (nunca chamar `invoke` direto em componentes).
- Commits: `feat(core): ...`, `feat(ui): ...`, `fix(s3): ...`.
- Cada fase do `SPEC.md §10` deve passar em `clippy -D warnings` + testes antes de avançar.

## Ao implementar

1. Diga qual fase do `SPEC.md` está atacando.
2. Liste arquivos que vai criar/alterar.
3. Implemente; adicione teste quando o módulo for do core.
4. Rode lint/testes e reporte o resultado.

## Não fazer

- Não carregar fontes, ícones ou CSS de CDN (CSP `'self'`); todas vendorizadas em `public/`.
- Não gravar o JSON da Service Account fora do keyring; nunca em `%APPDATA%` ou config.json.
- Não criar rotas além de `/dashboard` e `/settings` sem decisão registrada em `.specs/project/DECISIONS.md`.
- Não usar Material Symbols (usar `lucide-react` SVG inline).
- Não adicionar dependência sem justificar no `SPEC.md §12`.
- Não gravar credencial em JSON, `.env` ou SQLite.
- Não bloquear o runtime Tokio com I/O síncrono pesado (use `spawn_blocking` para hash e SQLite).
- Não alterar `capabilities/default.json` além do mínimo necessário para o command em questão.
