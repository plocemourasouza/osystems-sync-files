# Lessons — oSystems Sync

Formato: 🐛 Gotcha · 📐 Pattern · ⚡ Performance · 🔒 Security · 🧠 Context

## Fase 0 (2026-09-04)

- 🐛 **Gotcha** — Aliasar um crate local como `core` em `Cargo.toml` (`core = { path = …, package = … }`) sombreia `::core` e quebra `tauri::generate_context!()` com `E0433`. Use o nome real (`osystems-sync-core`) e importe como `osystems_sync_core`.
- 🐛 **Gotcha** — Tailwind v4: mapear tokens com o mesmo nome exige `@theme inline`; `@theme` puro gera auto-referência (`--x: var(--x)`) e o valor vira inválido.
- 🐛 **Gotcha** — `@tailwindcss/vite` intercepta imports de `.css` (inclusive `?raw`) dentro do Vitest; testes que precisam do CSS bruto devem ler pelo `node:fs`.
- 🐛 **Gotcha** — `grep -c "@font-face" dist/*.css` conta linhas, não ocorrências; CSS minificado é 1 linha. Use `grep -o … | wc -l`.
- 🐛 **Gotcha** — Em JSX, `watchPath="D:\\Projetos"` não é desescapado como string JS; monte caminhos Windows em variáveis JS (`{path}`) nos testes.
- 📐 **Pattern** — Tasks paralelas que precisariam adicionar as mesmas deps npm: instalar uma vez no orquestrador antes da wave (evita 3 agentes editando `package.json`). Idem para `vitest.config.ts` + `src/test/setup.ts`.
- 📐 **Pattern** — Tasks que editam o mesmo arquivo Rust (`lib.rs`, `Cargo.toml`) fundem-se num único agente mesmo que o plano as liste separadas (T-0.7 + T-0.8).
- 📐 **Pattern** — `data-tauri-drag-region` precisa estar no elemento *sob o cursor*: aplicar em cada wrapper não-interativo da title bar, nunca nos botões.
- 🔒 **Security** — CSP do Tauri 2 vive em `tauri.conf.json → app.security.csp`; `capabilities/*.json` só carrega permissões. Colocar CSP em capabilities silenciosamente não faz nada.
- 🔒 **Security** — Windows Credential Manager grava blobs em UTF-16 (limite 2560 bytes ⇒ ~1280 chars ASCII): o JSON da Service Account (2–3 KB) **sempre** precisa ser fragmentado no keyring.
- 🧠 **Context** — O agente `validator` com `model: opus` subiu sem tools (Read/Bash indisponíveis); re-despachar como `general-purpose` resolveu. Verificar acesso a tools no início de agentes read-only.
- 🧠 **Context** — Runtime `tauri 2.11.5` / `plugin-autostart 2.5.1`: `TrayIconBuilder::with_id`, `Image::from_bytes` (feature `image-png`), `tray_by_id`, `ManagerExt::autolaunch()` — confirmados lendo o código-fonte baixado via `cargo fetch`, não por memória.

## Fase 1 (2026-09-04)

- 🐛 **Gotcha** — `ts-rs` mapeia `u64`/`i64`/`usize` para `bigint`. Regra do projeto: campos de config usam `u32`; tamanhos/contadores `u64` em structs IPC levam `#[ts(type = "number")]` (≤ 2^53 é seguro para 5 GB). Nunca deixar `bigint` chegar ao renderer.
- 🐛 **Gotcha** — Cargo resolve `.cargo/config.toml` a partir do **cwd**, não do `--manifest-path`; `TS_RS_EXPORT_DIR` precisa estar em `<repo>/.cargo/config.toml` para valer com `cargo test --manifest-path src-tauri/Cargo.toml`.
- 🐛 **Gotcha** — BSD `sed` (macOS) não entende `\b`; usar classes `([0-9]+)n([,} ])`.
- 📐 **Pattern** — Primitivos de UI compartilhados por duas tasks paralelas viram uma task própria antes da wave (T-1.7a), como já feito com deps npm e setup do Vitest.
- 🐛 **Gotcha** — `#[ts(type = "number")]` em `Option<u64>` **substitui** o tipo inteiro (perde o `| null`); usar `#[ts(type = "number | null")]`.
- 🐛 **Gotcha** — `notify` no macOS reporta caminhos canonicalizados (`/private/var/...`); testes devem canonicalizar o `TempDir` antes de comparar.
- 🐛 **Gotcha** — Agentes paralelos que 'anexam' num barrel `index.ts` podem sobrescrever; instruir 'append only' e verificar o arquivo no merge.

## Fase 2 (2026-09-04)

- 🐛 **Gotcha (bug real, pego no aceite)** — `notify` entrega caminhos canônicos (`/private/tmp/...` no macOS) e o `rescan` caminhava o `watch.path` bruto (`/tmp/...`): mesmo arquivo virou 2 linhas em `files`. Regra: canonicalizar no **único** ponto de entrada (`queue::intake`) e a raiz do `rescan`; teste de regressão `rescan_with_non_canonical_root_does_not_duplicate_watcher_intakes`. No Windows o equivalente é case/`\\?\`/UNC — mesma defesa.
- 📐 **Pattern** — Aceite de fase roda **de verdade** (app em background + `sqlite3` + arquivo em chunks), não só testes unitários: foi o que revelou o bug acima.
- 🐛 **Gotcha** — `cargo test a b::` não aceita dois filtros; rodar duas invocações.
- 🧠 **Context** — 5 subagentes morreram simultaneamente por limite de sessão (HTTP 429) no meio da edição. Recuperação: (1) `tsc`/`vitest`/`cargo check --all-targets` para medir o estado real; (2) relançar cada task com prompt "RESUME: leia os arquivos existentes, termine o que falta, não reescreva". Arquivos meio-editados compilaram exceto testes (`s3.rs`), que ficaram isolados por arquivo.

## Fases 3–6 (2026-09-04)

- 🔒 **Security (auditoria T-6.3)** — override de endpoint S3 por variável de ambiente estava ativo em release: qualquer processo local sem privilégio redirecionaria uploads + assinaturas SigV4. Regra: overrides de rede só sob `cfg(debug_assertions)` e com allowlist de esquema/host.
- 🔒 **Security** — Pasta monitorada pode conter symlink/junction (no Windows `mklink /J` não exige privilégio): sem checagem de contenção (`canonical.starts_with(root)`), o agente exfiltraria arquivos de fora. Canonicalizar **e** conter, sempre.
- 🔒 **Security** — Multipart S3 abandonado após esgotar tentativas cobra para sempre; abortar no `failed` e recomendar lifecycle `AbortIncompleteMultipartUpload` no bucket.
- 🐛 **Gotcha** — `S3 HeadBucket` path-style bate em `/{bucket}/` (barra final); operações de objeto não. Descoberto via `MockServer::received_requests()`.
- 🐛 **Gotcha** — `yup-oauth2` fixa `exp - iat = 3595` s e descarta o status HTTP do token endpoint; classificar `AuthError` como 400 sintético para reaproveitar `classify()`.
- 🐛 **Gotcha** — `DispatchFailure` do aws-sdk não tem `is_connect()`; usar `is_timeout()` para separar timeout de falha de conexão.
- 🐛 **Gotcha** — `<header>` aninhado dentro de `<section>`/`<main>` ainda recebe `role=banner` no jsdom/testing-library (não aplica exclusão por ancestral) → colisão com a title bar; usar `<div>` para cabeçalhos internos ou `getAllByRole`.
- 🐛 **Gotcha** — `node --test <dir>` falha no Node 26 (`MODULE_NOT_FOUND`); usar glob de arquivos.
- 📐 **Pattern** — Quando várias tasks restantes convergem nos mesmos arquivos de integração (`lib.rs`, `runtime.rs`, `ipc.ts`), fundir em 1 agente por lado (backend/frontend) é mais rápido do que serializar 5 agentes.
- 📐 **Pattern** — `state_sink` (callback `Fn(&str, Value)`) nos uploaders para persistir `remote_state` no meio do upload: crash deixa estado retomável e o boot consegue abortar órfãos.
- 🧠 **Context** — Rate limit (429) matou 5 agentes de uma vez; ao relançar, o prompt "RESUME: leia o que existe" evitou retrabalho — só `s3.rs` precisou de correção de testes.
- 🐛 **Gotcha** — `UnlistenFn` do `@tauri-apps/api/event` é tipado `() => void` mas retorna uma Promise (`plugin:event|unlisten`); fora do runtime Tauri rejeita **assincronamente** → `try/catch` não pega; vitest sai com exit 1 mesmo com todos os testes passando. Tratar o retorno como thenable e `.catch`. Idem para hidratação best-effort (`fetchRecent`): nunca deixar rejeição escapar de um `useEffect`.
- 🐛 **Gotcha (bug de campo)** — Política IAM write-only (Put/List, sem `DeleteObject`) derrubava o `test_connection` porque o probe tentava apagar o próprio arquivo → 403 → `Auth` → "autenticação necessária". Teste de conexão só pode exigir o que o app realmente precisa para operar; limpeza é best-effort. Diagnóstico passo a passo (`examples/s3_diag.rs`) achou em 1 execução.
- 🔒 **Security** — Credenciais chegaram num `.md` dentro do repo: `.gitignore` imediato + pedir rotação após o teste; nunca ecoar valores em comandos/logs (usar `set -a` a partir do arquivo).
- 🐛 **Gotcha (bug de campo, Windows)** — `std::fs::canonicalize` no Windows devolve caminho *verbatim* (`\\?\C:\...`); foi parar em `files.path` e na UI como `//?/C:/monitoramento`, e quebraria `starts_with` de contenção se só um lado fosse verbatim. Regra: todo `canonicalize` passa por `paths::canonicalize_clean` (remove `\\?\` e `\\?\UNC\`); migração idempotente limpa linhas antigas no `Repo::open`.
- 📐 **Pattern** — Modo `?mock=1` (só `import.meta.env.DEV`, tree-shaken em prod) semeando as stores permite verificar layout no Chrome sem o runtime Tauri — reproduziu o caso da foto (1366×768) em segundos.
