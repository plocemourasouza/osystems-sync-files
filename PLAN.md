---
version: 1.0
date: 2026-09-04
sources: [PRD.md, SPEC.md]
gate_commands:
  - "cargo test --workspace --manifest-path src-tauri/Cargo.toml"
  - "cargo clippy --workspace --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings"
  - "cargo fmt --all --manifest-path src-tauri/Cargo.toml -- --check"
  - "npm run build"
  - "npm run test"
  - "npm run typecheck"
---

# PLAN — oSystems Sync (roadmap de desenvolvimento)

> Ordem das fontes de verdade: `PRD.md` (o quê) → `SPEC.md` (como) → `design/` (aparência) → **este arquivo** (quando/quem/em que ordem). Identificadores (`RF-xxx`, `RNF-xxx`, `T-x.y`, nomes de módulo, commands) ficam em inglês; prosa em pt-BR, conforme `CLAUDE.md`.

---

## 0. Como ler este plano

- **Fases** (`§3`) seguem `SPEC.md §10`, na mesma ordem e com o mesmo critério de aceite — não foi criada nem removida nenhuma fase.
- Dentro de cada fase, as tarefas são agrupadas em **waves**: tarefas da mesma wave tocam arquivos diferentes e podem ser despachadas em paralelo (`dispatching-parallel-agents`); a próxima wave só começa quando a anterior fecha. A numeração de wave é **local à fase** (reinicia em cada fase).
- **Checkpoint por wave**: ao final de cada wave, reportar resultado e parar para revisão antes de abrir a próxima (autonomia padrão do `plan-mode.mdc`).
- **Checkpoint de fase**: uma fase só é considerada fechada quando os *gate commands* abaixo passam (equivalentes aos comandos de `CLAUDE.md`, aqui com `--manifest-path` porque a raiz do repo é `oSystems Sync/` e o crate Rust vive em `src-tauri/`):

  ```bash
  cargo test --workspace --manifest-path src-tauri/Cargo.toml
  cargo clippy --workspace --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings
  cargo fmt --all --manifest-path src-tauri/Cargo.toml -- --check
  npm run build
  npm run test
  npm run typecheck
  ```

  mais o critério de aceite manual da fase (SPEC §10) e, para tarefas de `crates/core`, o teste específico listado em `verify`.
- **TDD é regra da casa** (`tdd-workflow`): nenhuma tarefa que toque `crates/core` é dada como concluída sem teste que falha antes e passa depois. Tarefas de UI seguem `verification-before-completion` (Vitest onde há lógica; revisão visual contra `design/DESIGN.md` onde é puro layout).
- **Segurança transversal**: qualquer tarefa que toque `credentials.rs`, `keyring`, `capabilities/default.json` ou `tauri.conf.json` (CSP) recebe revisão do `security-auditor` (skill `security-best-practices`) antes de fechar a wave — não é uma tarefa numerada à parte, é um gate embutido no `done when` dessas tarefas.

---

## 1. Objetivo

Entregar o **oSystems Sync**: um agente desktop Windows (Tauri 2 + núcleo Rust + React/TS) que observa uma pasta local e replica cada arquivo novo, de forma independente e sem intervenção humana por 10+ dias, para uma pasta no Google Drive (via Service Account) e um bucket AWS S3 — com fila persistente em SQLite, retry com backoff, limite de banda por destino e um cockpit de 2 telas para acompanhar e intervir.

---

## 2. Roster global

- **Models** (tiers de `smart-model-dispatch`):
  - `sonnet` — implementação padrão (módulos `core`, commands Tauri, componentes React, integração).
  - `haiku` — boilerplate mecânico (scaffold inicial, vendorização de assets/fontes, CI, i18n, testes triviais de configuração).
  - `opus` — revisão de segurança (`T-6.3`), decisões arquiteturais que a fase reabra e o *readiness gate* final (`T-6.7`).
  - `fable` — não utilizado neste plano (sem síntese criativa fora de escopo técnico).
- **Agents**: `backend-engineer` (Rust/`crates/core`, commands IPC), `frontend-engineer` (React/TS/Tailwind), `test-engineer` (testes unitários dedicados quando não embutidos na tarefa de implementação), `security-auditor` (credenciais, capabilities, CSP, RNF-002/003/009/011/012), `code-reviewer` (fecho de cada fase), `qa-automation-engineer` (teste de 72 h, E2E), `devops-engineer` (CI, instalador, auto-update).
- **Skills**: `tdd-workflow`, `coding-guidelines`, `verification-before-completion` (sempre ativas); `security-best-practices` (manifest — credenciais/keyring/CSP); `rust-pro` (manifest — módulos `crates/core`); `tailwind-patterns` + `frontend-ui-system` (telas e componentes, tokens de `design/tokens.css`); `e2e-testing-patterns` (teste de longa duração, Playwright se aplicável); `i18n-localization` (manifest — `src/i18n/`); `accessibility` (manifest — `T-6.2`); `dispatching-parallel-agents` (execução das waves).

---

## 3. Fases

### Fase 0 — Scaffold

**Objetivo**: ter o esqueleto do app rodando — workspace Tauri 2 + React + Tailwind v4 com `crates/core`, shell visual idêntico ao mockup (com dados mock), tray e autostart, e CI verde.

**Critério de aceite** (SPEC §10, fase 0): o app abre com o shell idêntico ao mockup, minimiza para a bandeja, inicia com o Windows.
Decomposto: (a) `npm run tauri dev` abre janela sem decoração nativa com title bar/sidebar/statusbar nos tokens de `design/tokens.css`; (b) fechar a janela chama `hide()` e o processo continua vivo; (c) toggle de autostart grava a entrada de inicialização do Windows; (d) `cargo test`, `clippy -D warnings`, `npm run build`, `vitest` passam no CI.

**Waves**:
- Wave 1: `T-0.1`
- Wave 2: `T-0.2`, `T-0.7`, `T-0.8`, `T-0.9`, `T-0.10`
- Wave 3: `T-0.3`, `T-0.4`, `T-0.5`
- Wave 4: `T-0.6`, `T-0.11`

**Tasks**:

### T-0.1 — Scaffold do workspace Tauri 2 + React + Vite
- story: infra (pré-requisito de todo RF)
- wave: 1 · depends_on: []
- model: haiku · agent: devops-engineer · skills: coding-guidelines
- files: `package.json`, `vite.config.ts`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, `src-tauri/capabilities/default.json`, `src-tauri/src/main.rs`, `src-tauri/src/lib.rs`, `src-tauri/crates/core/Cargo.toml`
- done when: `npm install && npm run tauri dev` abre uma janela vazia sem decoração nativa (`decorations: false`); `tauri.conf.json` nasce com `app.security.csp = "default-src 'self'; connect-src ipc: http://ipc.localhost; font-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'"` (RNF-011); `capabilities/default.json` nasce sem permissões além dos commands desta fase; `package.json` define `test` (vitest), `typecheck` (tsc --noEmit) e `lint` (eslint) usados pelos gates; workspace Cargo inclui `crates/core` como membro.
- verify: `npm run tauri dev` sobe sem erro; `cargo check --workspace --manifest-path src-tauri/Cargo.toml` exit 0

### T-0.2 — Tokens e fontes vendorizadas
- story: RNF-014
- wave: 2 · depends_on: [T-0.1]
- model: haiku · agent: frontend-engineer · skills: tailwind-patterns
- files: `src/styles/app.css`, `public/fonts/Inter*.woff2`, `public/fonts/JetBrainsMono*.woff2`
- done when: `app.css` importa `design/tokens.css` antes do bloco `@theme` (mapeamento 1:1 dos nomes semânticos, conforme `design/README.md`); Inter e JetBrains Mono carregam de `public/fonts/` (zero requisição a `fonts.googleapis.com` ou CDN).
- verify: `npm run build` seguido de `grep -r "fonts.googleapis\|cdn" dist/assets/*.css` retorna vazio

### T-0.7 — Ícone de bandeja e menu
- story: RF-090
- wave: 2 · depends_on: [T-0.1]
- model: sonnet · agent: backend-engineer · skills: coding-guidelines
- files: `src-tauri/src/tray.rs`, `src-tauri/icons/tray-ok.ico`, `src-tauri/icons/tray-working.ico`, `src-tauri/icons/tray-error.ico`
- done when: tray exibe 3 variantes de ícone (ok/trabalhando/erro, ainda sem lógica real — trocado por `set_icon` chamável manualmente) e menu com Abrir, Pausar/Retomar watcher, Rescan, Sair (itens desabilitados/no-op nesta fase).
- verify: `npm run tauri dev`; clique no tray abre o menu com os 4 itens (checagem manual)

### T-0.8 — Autostart do Windows
- story: RF-092
- wave: 2 · depends_on: [T-0.1]
- model: haiku · agent: backend-engineer · skills: coding-guidelines
- files: `src-tauri/src/lib.rs`, `src-tauri/Cargo.toml` (dep `tauri-plugin-autostart`), `src-tauri/src/commands/system.rs`
- done when: plugin `tauri-plugin-autostart` registrado no `lib.rs`; command `set_autostart({ enabled })` grava/remove a entrada de inicialização.
- verify: chamar `set_autostart({enabled:true})` via devtools console (`window.__TAURI__.core.invoke`) e confirmar entrada em `HKCU\...\Run` (checagem manual no Registry Editor)

### T-0.9 — Pipeline de CI
- story: infra
- wave: 2 · depends_on: [T-0.1]
- model: haiku · agent: devops-engineer · skills: coding-guidelines
- files: `.github/workflows/ci.yml`
- done when: `runs-on: windows-latest` (crates `windows`/`keyring` e `tauri build` não compilam no runner ubuntu); workflow roda, em push/PR, os 6 *gate commands* de `§0` mais `npm run lint`.
- verify: `act -j ci` (ou execução manual local dos mesmos comandos) sai com exit 0

### T-0.10 — Esqueleto do crate `core` isolado de `tauri`
- story: RNF-009
- wave: 2 · depends_on: [T-0.1]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/lib.rs`
- done when: `crates/core` compila como biblioteca sem nenhuma dependência de `tauri` no `Cargo.toml`; teste unitário trivial (`assert!(true)`) prova que o crate testa isoladamente.
- verify: `cargo tree -p core --manifest-path src-tauri/Cargo.toml | grep -i tauri` retorna vazio; `cargo test -p core --manifest-path src-tauri/Cargo.toml` passa

### T-0.11 — Fechar janela minimiza para a bandeja
- story: RF-091
- wave: 4 · depends_on: [T-0.7]
- model: sonnet · agent: backend-engineer · skills: coding-guidelines
- files: `src-tauri/src/lib.rs`
- done when: evento `CloseRequested` da janela principal chama `window.hide()` e `prevent_close()`; clicar no ícone do tray reabre a janela.
- verify: `npm run tauri dev`; fechar a janela pelo X → processo continua no Gerenciador de Tarefas; clique no tray reabre (checagem manual)

### T-0.3 — Componente `TitleBar`
- story: RF-097
- wave: 3 · depends_on: [T-0.2]
- model: sonnet · agent: frontend-engineer · skills: tailwind-patterns, frontend-ui-system
- files: `src/components/shell/TitleBar.tsx`
- done when: barra de 36 px com `data-tauri-drag-region`, logo + nome "oSystems Sync", chip da pasta monitorada (valor mock), badge "Daemon: Active", controles nativos min/max/close funcionais.
- verify: `npx vitest run src/components/shell/__tests__/TitleBar.test.tsx` (render + clique nos 3 controles disparam os commands esperados, mockados)

### T-0.4 — Componente `Sidebar`
- story: RF-067
- wave: 3 · depends_on: [T-0.2]
- model: sonnet · agent: frontend-engineer · skills: tailwind-patterns, frontend-ui-system
- files: `src/components/shell/Sidebar.tsx`
- done when: largura 256 px; navegação com 2 itens (Dashboard, Configurações) e badge de contagem mock; card da pasta monitorada (caminho mock, badge "Watcher: Ativo"); widget de throughput mock (agregado/teto + barra segmentada Drive×S3).
- verify: `npx vitest run src/components/shell/__tests__/Sidebar.test.tsx`

### T-0.5 — Componente `StatusBar`
- story: RF-068
- wave: 3 · depends_on: [T-0.2]
- model: sonnet · agent: frontend-engineer · skills: tailwind-patterns, frontend-ui-system
- files: `src/components/shell/StatusBar.tsx`
- done when: barra de 28 px com "Rust Core vX.Y.Z (Active)", "GDrive: Online/Offline/Auth", "AWS S3: Online/Offline/Auth (region)" e build target à direita — todos com valor mock.
- verify: `npx vitest run src/components/shell/__tests__/StatusBar.test.tsx`

### T-0.6 — `App.tsx` + roteamento das 2 telas
- story: RF-060…RF-086 (estrutura de navegação)
- wave: 4 · depends_on: [T-0.3, T-0.4, T-0.5]
- model: sonnet · agent: frontend-engineer · skills: frontend-ui-system
- files: `src/main.tsx`, `src/App.tsx`, `src/pages/Dashboard.tsx` (placeholder), `src/pages/Settings.tsx` (placeholder)
- done when: `react-router-dom` com rotas `/dashboard` e `/settings`; shell (`TitleBar`+`Sidebar`+`StatusBar`) envolve as duas rotas; navegação pela sidebar troca a rota.
- verify: `npx vitest run src/__tests__/App.test.tsx` (navegação entre rotas)

---

### Fase 1 — Config, estado, logging

**Objetivo**: persistência de configuração e logs, tipos IPC gerados, e a tela de Configurações (campos sem credenciais) já salvando de verdade.

**Critério de aceite** (SPEC §10, fase 1): config persiste; logs em arquivo; salvar aplica sem reiniciar o app.

**Waves**:
- Wave 1: `T-1.1`, `T-1.2`, `T-1.3`, `T-1.6`
- Wave 2: `T-1.4`, `T-1.5`, `T-1.10`
- Wave 3: `T-1.7`, `T-1.8`
- Wave 4: `T-1.9`
- Wave 5: `T-1.11`

**Tasks**:

### T-1.1 — `core::config`
- story: RF-001, RF-003, RF-086
- wave: 1 · depends_on: [T-0.10]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/config.rs`
- done when: struct `AppConfig` (watch/s3/gdrive/qos/retry/workers/autostart/keep_awake, espelhando o JSON de `SPEC.md §5`) com `#[derive(Serialize, Deserialize, TS)] #[ts(export)]`; `load()`/`save()` para `%APPDATA%/osystems-sync/config.json`; teste unitário cobre round-trip load→save→load e valores default.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml config::`

### T-1.2 — `core::state` (SQLite WAL + schema)
- story: RNF-006, RF-038 (schema)
- wave: 1 · depends_on: [T-0.10]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/state/mod.rs`, `src-tauri/crates/core/src/state/schema.sql`, `src-tauri/crates/core/src/state/repo.rs`
- done when: `schema.sql` cria `files`, `jobs`, `events` exatamente como `SPEC.md §5`; `repo.rs` abre `state.db` em modo WAL e roda migrations idempotentes; teste unitário com banco in-memory cobre criação de tabelas e um insert/select básico em `files`.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml state::`

### T-1.3 — `core::logging`
- story: RF-099, RNF-005, RNF-015
- wave: 1 · depends_on: [T-0.10]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/logging.rs`
- done when: `tracing-subscriber` com 2 layers — arquivo JSON rotativo (`tracing-appender`, 10 MB × 5 em `logs/app.log`) e canal `broadcast` que alimenta um ring buffer de 500 linhas em memória; formato de linha `{ ts, level, target, job_id?, destination?, message }`; nunca loga segredo/caminho de JSON da SA (RNF-015); teste unitário confirma que o ring nunca ultrapassa 500 entradas.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml logging::`

### T-1.6 — Scaffolding de i18n
- story: RNF-017
- wave: 1 · depends_on: [T-0.6]
- model: haiku · agent: frontend-engineer · skills: i18n-localization
- files: `src/i18n/pt-BR.json`, `src/i18n/index.ts`
- done when: todas as strings estáticas já escritas nas telas placeholder (fase 0) migradas para `pt-BR.json`; util `t(key)` central; catálogo `errors.*` indexado por `AppError.code` (Drive: 403 `forbidden` com e-mail da SA, 404 folder, JSON inválido, quota; S3: `AccessDenied`, `NoSuchBucket`, `InvalidAccessKeyId`, `SignatureDoesNotMatch`, região errada); nenhuma string de UI hardcoded fora de `src/i18n/`.
- verify: `grep -rnE '>[[:space:]]*[A-Za-zÀ-ú]{4,}' src/pages src/components --include=*.tsx | grep -v 't(' | grep -v '=>' ` retorna vazio (nenhum texto solto em JSX fora de `t()`)

### T-1.4 — Geração de tipos via `ts-rs`
- story: RNF-010
- wave: 2 · depends_on: [T-1.1]
- model: haiku · agent: backend-engineer · skills: rust-pro
- files: `src-tauri/crates/core/src/lib.rs` (test de export), `src/types/generated.ts` (gerado, não editar à mão)
- done when: `cargo test` regenera `src/types/generated.ts` a partir de `AppConfig` e demais structs `#[ts(export)]` já existentes; diff limpo (arquivo versionado bate com o gerado).
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml && git diff --exit-code src/types/generated.ts`

### T-1.5 — Commands `get_config` / `save_config`
- story: RF-083 (persistência), RF-085
- wave: 2 · depends_on: [T-1.1, T-1.4]
- model: sonnet · agent: backend-engineer · skills: rust-pro, security-best-practices
- files: `src-tauri/src/commands/config.rs`, `src-tauri/capabilities/default.json`
- done when: `get_config` retorna `AppConfig`; `save_config(AppConfig)` valida (bucket/region não vazios quando destino habilitado) e persiste via `core::config`; ambos os commands autorizados na `capabilities/default.json` e em nenhum outro; nenhum campo de segredo trafega por esses commands (RF-085).
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml && cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`

### T-1.10 — `zustand` store de config
- story: infra (RF-083)
- wave: 2 · depends_on: [T-1.4]
- model: sonnet · agent: frontend-engineer · skills: frontend-ui-system
- files: `src/store/configStore.ts`, `src/api/ipc.ts` (wrappers `getConfig`/`saveConfig`)
- done when: `ipc.ts` expõe `getConfig()`/`saveConfig()` tipados por `generated.ts` (nenhum componente chama `invoke` direto, conforme `CLAUDE.md`); store carrega config no boot e expõe setters granulares.
- verify: `npx vitest run src/store/__tests__/configStore.test.ts`

### T-1.7 — Tela Configurações: seção Geral
- story: RF-086
- wave: 3 · depends_on: [T-1.6, T-1.10]
- model: sonnet · agent: frontend-engineer · skills: tailwind-patterns, frontend-ui-system
- files: `src/components/settings/GeneralModule.tsx`, `src/pages/Settings.tsx`
- done when: campos iniciar com Windows, manter acordado, workers por destino (1–4), tentativas máx. (1–10), filtros do watcher (extensões, subpastas, tamanho máx.) — todos ligados ao `configStore`; layout conforme `design/DESIGN.md`.
- verify: `npx vitest run src/components/settings/__tests__/GeneralModule.test.tsx`

### T-1.8 — Tela Configurações: campos S3/Drive sem credenciais
- story: RF-080 (campos não-secretos), RF-081 (idem)
- wave: 3 · depends_on: [T-1.10]
- model: sonnet · agent: frontend-engineer · skills: tailwind-patterns, frontend-ui-system
- files: `src/components/settings/S3Module.tsx` (esqueleto, sem Access Key/Secret ainda), `src/components/settings/DriveModule.tsx` (esqueleto, sem upload de JSON ainda)
- done when: Region/Bucket/Prefix/Storage Class (S3) e Folder ID (Drive) renderizam e persistem via `configStore`; campos de credencial aparecem como placeholder desabilitado com nota "disponível na Fase 3/4".
- verify: `npx vitest run src/components/settings/__tests__/S3Module.test.tsx src/components/settings/__tests__/DriveModule.test.tsx`

### T-1.9 — Salvar/Cancelar/Restaurar padrões + `Ctrl+S`
- story: RF-083, RF-084
- wave: 4 · depends_on: [T-1.5, T-1.7, T-1.8]
- model: sonnet · agent: frontend-engineer · skills: frontend-ui-system
- files: `src/pages/Settings.tsx`, `src/components/settings/SettingsFooter.tsx`
- done when: atalho `Ctrl+S` e botão "Salvar Preferências" chamam `saveConfig`; rodapé mostra "Última alteração salva às HH:MM:SS"; "Cancelar" descarta alterações não salvas (recarrega do store); "Restaurar padrões" reseta QoS/filtros/workers de fábrica **sem** tocar credenciais (nenhum command de credencial é chamado); validação inline por campo (bucket vazio, region inválida) bloqueia o save com mensagem.
- verify: `npx vitest run src/pages/__tests__/Settings.test.tsx`

### T-1.11 — Vitest dos wrappers IPC de config
- story: RNF-003 (parcial), infra
- wave: 5 · depends_on: [T-1.9]
- model: haiku · agent: test-engineer · skills: verification-before-completion
- files: `src/api/__tests__/ipc.test.ts`
- done when: `@tauri-apps/api/mocks` cobre `getConfig`/`saveConfig` — payload de retorno não contém nenhum campo de segredo (ainda inexistente nesta fase, mas o teste já fixa o contrato).
- verify: `npx vitest run src/api/__tests__/ipc.test.ts`

**Riscos da fase**: mudar o schema SQLite depois de haver dados reais exige migration versionada (mitigação: `repo.rs` já nasce com tabela de versão de schema); `ts-rs` pode gerar tipo desatualizado silenciosamente se o dev esquecer de rodar `cargo test` (mitigação: `T-1.4` vira gate de CI, diff falha o build).

---

### Fase 2 — Watcher, fila (leitura) e Dashboard

**Objetivo**: detecção real de arquivos, fila em `pending`, e o Dashboard mostrando o estado real (ainda sem upload).

**Critério de aceite** (SPEC §10, fase 2): copiar arquivo grande aparece só após estabilizar; reinício não duplica; pausar watcher ignora novos arquivos.

**Waves**:
- Wave 1: `T-2.1`, `T-2.2`
- Wave 2: `T-2.3`, `T-2.4`
- Wave 3: `T-2.5`, `T-2.7`
- Wave 4: `T-2.6`
- Wave 5: `T-2.8`, `T-2.9`, `T-2.10`
- Wave 6: `T-2.11`, `T-2.12`

**Tasks**:

### T-2.1 — `core::hash`
- story: RF-039
- wave: 1 · depends_on: [T-0.10]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/hash.rs`
- done when: `sha256_file(path) -> Result<String>` faz hashing em streaming (sem carregar o arquivo inteiro na memória); teste unitário compara contra hash SHA-256 conhecido de um arquivo fixture.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml hash::`

### T-2.2 — `core::stabilize`
- story: RF-002
- wave: 1 · depends_on: [T-0.10]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/stabilize.rs`
- done when: `wait_until_stable(path, stable_secs, timeout)` lê `metadata().len()` a cada 1 s, considera estável após N leituras iguais consecutivas **e** abertura exclusiva (`share_mode(0)`) bem-sucedida; timeout de 30 min loga `warn` e segue mesmo assim; teste unitário simula um arquivo escrito em chunks (tokio::time::pause) e confirma que só estabiliza após a última escrita.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml stabilize::`

### T-2.3 — `core::watcher`
- story: RF-001, RF-003, RF-005, RF-006
- wave: 2 · depends_on: [T-2.2, T-1.1]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/watcher.rs`
- done when: `spawn_watcher(cfg, tx)` usa `notify` + `notify-debouncer-full` (debounce 2 s), aplica filtros (extensões, recursivo, tamanho máx.), ignora `~$*`/`*.tmp`/`*.crdownload`/`*.part`/ocultos/diretórios; `WatcherHandle::pause()`/`resume()` descarta eventos sem parar a task; teste de integração em `tempdir` cobre: arquivo criado → 1 evento; arquivo `.tmp` → 0 eventos; watcher pausado → 0 eventos.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml watcher::`

### T-2.4 — `core::queue` (criação de jobs)
- story: RF-030, RF-039, RF-038 (schema em uso)
- wave: 2 · depends_on: [T-1.2, T-2.1]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/queue.rs`
- done when: `upsert_file_and_enqueue(path, hash, size)` insere/atualiza `files` e cria exatamente 2 jobs `pending` (`s3`, `gdrive`) com `UNIQUE(file_id, destination)`; reenviar o mesmo `path+sha256` não cria jobs duplicados; conteúdo diferente (hash novo) **atualiza** a linha de `files` (`sha256`, `size`, `mtime`) e reseta os 2 jobs existentes para `pending` (`attempts=0, remote_id=NULL, remote_state=NULL, archived_at=NULL, last_error=NULL`) — nunca insere um segundo par (violaria `UNIQUE(file_id, destination)`); teste unitário com SQLite in-memory cobre os 3 casos.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml queue::`

### T-2.5 — `rescan()`
- story: RF-004
- wave: 3 · depends_on: [T-2.4]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/lib.rs` (função `rescan`)
- done when: varre a pasta respeitando filtros; para cada arquivo, se não está em `files` **ou** hash mudou, enfileira; teste de integração cria 3 arquivos com "app fechado" (chama `rescan` direto), depois roda de novo e confirma 0 duplicatas.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml rescan`

### T-2.7 — `events.rs` (emissão para o renderer)
- story: RF-040 (canal), infra dos eventos `job-updated`/`status-changed`
- wave: 3 · depends_on: [T-2.4]
- model: sonnet · agent: backend-engineer · skills: rust-pro
- files: `src-tauri/src/events.rs`
- done when: funções `emit_job_updated(app, job_view)` e `emit_status_changed(app, status)` tipadas com as structs `#[ts(export)]` de `JobView`/`AppStatus`; `spawn_log_forwarder(app)` assina o `broadcast` de `core::logging` e emite o evento `log-line` (`{ts, level, target, job_id?, destination?, message}`) por linha.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml && cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`

### T-2.6 — Commands `pick_folder`, `rescan`, `pause_watcher`/`resume_watcher`, `list_jobs`, `get_status`
- story: RF-001, RF-004, RF-005, RF-064
- wave: 4 · depends_on: [T-2.3, T-2.5]
- model: sonnet · agent: backend-engineer · skills: rust-pro, security-best-practices
- files: `src-tauri/src/commands/queue.rs`, `src-tauri/crates/core/src/state/repo.rs`, `src-tauri/capabilities/default.json`
- done when: `get_status` devolve `AppStatus { watcher_paused, destinations{online, auth_required, latency_ms?}, counts_by_status, bytes_total, bytes_done, core_version, build_target }` a partir de uma única query agregada em `state/repo.rs` (consumido por KPIs, StatusBar e tray); `resume_watcher` dispara `rescan()`; `pick_folder` usa `plugin-dialog` e, ao trocar a pasta, reinicia o watcher sem reiniciar o app (`save_config` dispara restart); `list_jobs({statuses?, destination?, include_archived?, limit, offset})` faz join `jobs`×`files` e agrega os 2 destinos por arquivo em `JobView`, paginado; `capabilities/default.json` ganha `dialog:allow-open` e nada além disso.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml`; 10 000 jobs `done` de fixture → `list_jobs` responde em ≤ 200 ms (`cargo test -p core --manifest-path src-tauri/Cargo.toml state::repo::list_jobs_perf`); `cargo test --manifest-path src-tauri/Cargo.toml commands::` cobre os commands

### T-2.8 — KPIs e ações rápidas do Dashboard
- story: RF-060, RF-061
- wave: 5 · depends_on: [T-2.6]
- model: sonnet · agent: frontend-engineer · skills: tailwind-patterns, frontend-ui-system
- files: `src/components/dashboard/KpiCard.tsx`, `src/pages/Dashboard.tsx`
- done when: KPIs (Total detectados+bytes, Concluídos+%, Em transferência, Na fila, Falhas) batem com `list_jobs`/`get_status` reais; botões Atualizar lista (rescan), Pausar/Retomar watcher chamam os commands de `T-2.6` e refletem estado (label muda).
- verify: `npx vitest run src/components/dashboard/__tests__/KpiCard.test.tsx`

### T-2.9 — Tabela dupla (`JobTable`/`JobRow`/`DualProgress`)
- story: RF-062, RF-063, RF-064, RF-065 (parcial: abrir no Explorer)
- wave: 5 · depends_on: [T-2.6]
- model: sonnet · agent: frontend-engineer · skills: tailwind-patterns, frontend-ui-system
- files: `src/components/dashboard/JobTable.tsx`, `src/components/dashboard/JobRow.tsx`, `src/components/dashboard/DualProgress.tsx`
- done when: colunas Arquivo & Origem / Tamanho / Google Drive / AWS S3 / Status / Ações, linha de 56 px; status agregado calculado por (`failed`→Falha, `uploading`→Enviando, ambos `done`→Sincronizado, senão Na Fila); filtro Todos/Ativos/Concluídos/Falhas + paginação de 50 persistem na sessão (RNF-008: 10 000 linhas ≤ 200 ms via paginação real, não client-side).
- verify: `npx vitest run src/components/dashboard/__tests__/JobTable.test.tsx`

### T-2.10 — Console de eventos
- story: RF-066, RF-098 (abrir pasta de logs)
- wave: 5 · depends_on: [T-1.3, T-2.6]
- model: sonnet · agent: frontend-engineer · skills: tailwind-patterns, frontend-ui-system
- files: `src/components/dashboard/LogConsole.tsx`, `src-tauri/src/commands/logs.rs`
- done when: console escuta o evento `log-line` via `listen` (latência ≤ 300 ms, RF-066) e usa `get_recent_logs({limit})` só no mount inicial; `open_logs_folder` abre `%APPDATA%/osystems-sync/logs/` via `plugin-opener`; console colapsável, ring de 500 linhas em memória no frontend, filtro por nível, tag de origem (`WATCHER`, `HASH`, `CORE`).
- verify: `npx vitest run src/components/dashboard/__tests__/LogConsole.test.tsx`

### T-2.11 — Estado vazio: sem pasta configurada
- story: RF-070 (parcial)
- wave: 6 · depends_on: [T-2.8, T-2.9, T-1.7]
- model: sonnet · agent: frontend-engineer · skills: tailwind-patterns, frontend-ui-system
- files: `src/components/dashboard/EmptyStateNoFolder.tsx`
- done when: Dashboard sem `watch.path` configurado mostra tela dedicada com CTA "Escolher pasta" que chama `pick_folder`; layout conforme `design/DESIGN.md`.
- verify: `npx vitest run src/components/dashboard/__tests__/EmptyStateNoFolder.test.tsx`

### T-2.12 — Vitest de paginação/filtro da `JobTable`
- story: RNF-008
- wave: 6 · depends_on: [T-2.9]
- model: haiku · agent: test-engineer · skills: verification-before-completion
- files: `src/components/dashboard/__tests__/JobTable.pagination.test.tsx`
- done when: teste cobre troca de filtro reseta página; navegação entre páginas não refaz fetch desnecessário.
- verify: `npx vitest run src/components/dashboard/__tests__/JobTable.pagination.test.tsx`

**Riscos da fase**: antivírus corporativo pode travar a abertura exclusiva durante `stabilize` (mitigação já no requisito: timeout 30 min + log `warn`, PRD §8); `notify` no Windows pode perder eventos durante cópias muito grandes — mitigado pelo `rescan()` de segurança no startup/resume.

---

### Fase 3 — Credenciais, S3, worker, throttle, health

**Objetivo**: primeiro upload real (S3 completo) com retry, throttle e ações de fila operacionais.

**Critério de aceite** (SPEC §10, fase 3): arquivo chega no bucket; rede off → backoff → on → `done`; teto de 2,5 MB/s respeitado ±10 %; matar processo → jobs retomam.

**Waves**:
- Wave 1: `T-3.1`, `T-3.2`, `T-3.6`
- Wave 2: `T-3.3`, `T-3.7`, `T-3.8`
- Wave 3: `T-3.4`
- Wave 4: `T-3.5`
- Wave 5: `T-3.9`
- Wave 6: `T-3.10`, `T-3.11`, `T-3.12`

**Tasks**:

### T-3.1 — `core::credentials` (keyring)
- story: RF-020, RF-085, RNF-002, RNF-003
- wave: 1 · depends_on: [T-0.10]
- model: sonnet · agent: backend-engineer · skills: rust-pro, security-best-practices, tdd-workflow
- files: `src-tauri/crates/core/src/credentials.rs`
- done when: `set_secret(key, value)`/`get_secret(key)`/`clear_secret(key)` usam `keyring` (service `osystems-sync`) para `aws.access_key_id`/`aws.secret_access_key`; nenhum valor é logado (`tracing`) nem persistido em `config.json`/SQLite; `mask(value)` retorna `AKIA****XYZ`-style; teste unitário cobre `mask()` e um round-trip com backend de keyring mockável (trait `SecretStore`).
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml credentials::`

### T-3.2 — `uploaders::mod.rs` (trait + erros)
- story: RF-032
- wave: 1 · depends_on: [T-0.10]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/uploaders/mod.rs`
- done when: trait `Uploader` (`id`, `test_connection`, `upload`) e `UploadError { Auth, Transient, Permanent, Io }` exatamente como `SPEC.md §6`; função `classify(status: u16, body: Option<&ErrorBody>) -> UploadError` mapeia 401→Auth; 403→Auth só com reason `forbidden`/`insufficientPermissions`/`AccessDenied`, 403 com reason `rateLimitExceeded`/`userRateLimitExceeded`/`storageQuotaExceeded`→Transient (backoff 1 h); 400 `invalid_grant` do token endpoint→Auth (hint clock skew); 5xx/429/timeout/`Io`→Transient; demais 4xx→Permanent; teste unitário cobre a tabela inteira, inclusive os 403 ambíguos.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml uploaders::classify`

### T-3.6 — `core::throttle`
- story: RF-050, RF-051, RF-052
- wave: 1 · depends_on: [T-0.10]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/throttle.rs`
- done when: `Throttle::new(limit_bps)`, `set_limit` (hot-reload), `acquire(bytes)` (token bucket, refil a cada 100 ms, burst = 1 s de teto, `0` = ilimitado); `ThrottledReader<R>` implementa `AsyncRead` chamando `acquire` a cada `poll_read`; um `Arc<Throttle>` é compartilhado entre os workers do mesmo destino (RF-052).
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml throttle::` — teste determinístico com `tokio::time::pause()`: 10 MB a 1 MB/s leva 10 s virtuais; 2 readers concorrentes somam o teto sem ultrapassar

### T-3.3 — `uploaders::s3` (upload simples + `test_connection`)
- story: RF-020, RF-021 (parcial `< 8MB`), RF-023
- wave: 2 · depends_on: [T-3.1, T-3.2]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/uploaders/s3.rs`
- done when: `S3Uploader` com `aws_sdk_s3::Client` de credenciais estáticas do keyring; `< 8 MB` via `put_object` com key `{prefix}{remote_name}` e metadata `x-amz-meta-sha256`; `test_connection` faz `head_bucket` + `put_object` de 0 bytes em `{prefix}.osystems-sync-probe` + delete, com latência medida.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml s3::` contra MinIO local (`docker run -p 9000:9000 minio/minio server /data`, `OSYSTEMS_SYNC_S3_ENDPOINT`)

### T-3.7 — `core::worker`
- story: RF-031, RF-038 (recovery), RF-040
- wave: 2 · depends_on: [T-3.2, T-3.6]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/worker.rs`
- done when: `worker_loop(dest)` seleciona 1 job `pending` elegível (`next_attempt_at` vencido), marca `uploading` em transação, chama `Uploader::upload` (envolvido pelo `Throttle` do destino), atualiza estado no fim; backoff `base * 2^attempts` ± 20 % jitter, teto 10 min, 5 tentativas → `failed`; no startup, `UPDATE jobs SET status='pending' WHERE status='uploading'`; emite progresso via `mpsc::Sender<ProgressUpdate>` a cada ≤ 500 ms; pool de `workers_per_destination` tasks Tokio.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml worker::` (backoff determinístico com `tokio::time::pause()`, recovery de `uploading`→`pending` no boot)

### T-3.8 — `core::health`
- story: RF-068 (Online/Offline)
- wave: 2 · depends_on: [T-3.1]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/health.rs`
- done when: a cada 60 s, `head_bucket` (S3) atualiza `AppStatus.destinations.s3.online`; falha 401/403 → `auth-required`; teste unitário com uploader mock cobre transição online→offline→auth-required.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml health::`

### T-3.4 — `uploaders::s3` multipart + abort
- story: RF-021 (`>= 8MB`), RF-024
- wave: 3 · depends_on: [T-3.3]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/uploaders/s3.rs`
- done when: `>= 8 MB` usa multipart (partes de 16 MB, 4 concorrentes); `CancellationToken` cancelado ou `failed` dispara `AbortMultipartUpload`; teste de integração com MinIO sobe arquivo de 20 MB e confirma ETag multipart; teste de cancelamento confirma `ListMultipartUploads` vazio após abort.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml s3::multipart`

### T-3.5 — `uploaders::s3` idempotência
- story: RF-022, RF-039
- wave: 4 · depends_on: [T-3.4]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/uploaders/s3.rs`
- done when: antes de enviar, `head_object`; se `x-amz-meta-sha256` bate, retorna `Ok` sem tráfego de upload; teste de integração reenvia arquivo já presente e mede que nenhum `PutObject`/`UploadPart` foi chamado (spy/contagem de requests do mock).
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml s3::idempotent`

### T-3.9 — Commands de fila e credenciais (S3)
- story: RF-033, RF-034, RF-035, RF-036, RF-050 (comando), RF-085
- wave: 5 · depends_on: [T-3.1, T-3.5, T-3.6, T-3.7]
- model: sonnet · agent: backend-engineer · skills: rust-pro, security-best-practices
- files: `src-tauri/src/commands/credentials.rs`, `src-tauri/src/commands/queue.rs`, `src-tauri/src/commands/qos.rs`, `src-tauri/capabilities/default.json`
- done when: `set_credential`/`get_credential_status`/`clear_credential` (S3) — `get_credential_status` devolve só `{ present, masked }`; `retry_job`, `retry_all_failed`, `cancel_job` (→ `status='cancelled'`, aborta multipart), `clear_completed` (marca `archived_at` **por arquivo, só quando os 2 jobs estão `done`**; não apaga); `set_qos({destination, limit_mbps})` chama `Throttle::set_limit` em ≤ 2 s; `test_connection({destination:"s3"})`; `open_remote({job_id})` abre `remote_state.web_view_link` (Drive) ou a URL do console S3 — `capabilities/default.json` ganha `opener:allow-open-url` restrito a `https://drive.google.com/*` e `https://*.console.aws.amazon.com/*`; boot recovery aborta `upload_id` órfãos antes de repor `uploading`→`pending`.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml && cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`; inspecionar payload de `get_credential_status` (teste) confirma zero campo `secret`/`value` cru

### T-3.10 — Settings: módulo AWS S3 completo
- story: RF-081
- wave: 6 · depends_on: [T-3.9]
- model: sonnet · agent: frontend-engineer · skills: tailwind-patterns, frontend-ui-system
- files: `src/components/settings/S3Module.tsx`
- done when: Access Key ID, Secret (campo password com "olho" que revela **só** valor ainda não salvo — após salvar, só máscara), Region (dropdown), Bucket, Prefix, Storage Class (`STANDARD`/`INTELLIGENT_TIERING`/`GLACIER_IR`), rodapé com botão "Testar Bucket" mostrando `✓ 41 ms • Bucket válido` ou erro IAM específico.
- verify: `npx vitest run src/components/settings/__tests__/S3Module.test.tsx`

### T-3.11 — Dashboard: ações por linha, progresso e throughput reais
- story: RF-040, RF-065, RF-067 (widget real), RF-098 (abrir no Explorer)
- wave: 6 · depends_on: [T-3.9]
- model: sonnet · agent: frontend-engineer · skills: tailwind-patterns, frontend-ui-system
- files: `src/components/dashboard/JobRow.tsx`, `src/components/shell/Sidebar.tsx`, `src-tauri/src/commands/system.rs`
- done when: ícones reenviar/cancelar/menu `⋯` (copiar caminho, abrir remoto, detalhes do erro) desabilitados conforme status; `open_in_explorer` chama `revealItemInDir(path)` do `plugin-opener` (permissão `opener:allow-reveal-item-in-dir`); barras de progresso escutam `upload-progress`; widget de throughput da Sidebar escuta `throughput` (1 s) com dado real.
- verify: `npx vitest run src/components/dashboard/__tests__/JobRow.actions.test.tsx`

### T-3.12 — Settings: módulo QoS (slider S3)
- story: RF-050, RF-051, RF-082 (parcial)
- wave: 6 · depends_on: [T-3.9]
- model: sonnet · agent: frontend-engineer · skills: tailwind-patterns, frontend-ui-system
- files: `src/components/settings/QosModule.tsx`
- done when: slider S3 (0,5–10 MB/s, ticks 0,5/2,5/5/10, posição "Ilimitado" no extremo) chama `set_qos` ao soltar; badge numérica ao lado reflete o valor.
- verify: `npx vitest run src/components/settings/__tests__/QosModule.test.tsx`

**Riscos da fase**: abort de multipart pode falhar silenciosamente e gerar custo órfão (mitigação: RF-024 + regra de ciclo de vida `AbortIncompleteMultipartUpload` documentada na ajuda, fora do código); throttle sob alta concorrência pode ter erro > 10 % se o refil de 100 ms for grosseiro demais — validar no teste determinístico de `T-3.6` antes de fechar a fase.

---

### Fase 4 — Google Drive (Service Account)

**Objetivo**: segundo destino completo, com autenticação **Service Account** (decisão C1 — não é OAuth).

**Critério de aceite** (SPEC §10, fase 4): JSON válido → "Conectado"; arquivo de 100 MB chega no Drive; queda no chunk 3 retoma; token renova sozinho após 1 h; pasta não compartilhada → `auth-required` com e-mail da SA.

**Waves**:
- Wave 1: `T-4.1`
- Wave 2: `T-4.2`, `T-4.6`, `T-4.9`
- Wave 3: `T-4.3`
- Wave 4: `T-4.4`
- Wave 5: `T-4.7`
- Wave 6: `T-4.8`, `T-4.10`

**Tasks**:

### T-4.1 — `gdrive::auth`
- story: RF-010 (leitura da SA), RF-016
- wave: 1 · depends_on: [T-3.1, T-3.2]
- model: sonnet · agent: backend-engineer · skills: rust-pro, security-best-practices, tdd-workflow
- files: `src-tauri/crates/core/src/uploaders/gdrive/auth.rs`
- done when: lê `gdrive.service_account_json` do keyring → `yup_oauth2::ServiceAccountKey` → `ServiceAccountAuthenticator` (JWT RS256, escopo `https://www.googleapis.com/auth/drive`, validade 1 h); access token cacheado em memória com skew de 5 min, **nunca persistido em disco**; erro de parse do JSON vira `UploadError::Auth`; teste unitário com chave de teste (fixture RSA de teste, não real) cobre parse inválido → `Auth` e cache respeitando skew.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml gdrive::auth`

### T-4.2 — `gdrive::upload` (simples, `< 8 MB`)
- story: RF-012 (parcial)
- wave: 2 · depends_on: [T-4.1]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/uploaders/gdrive/upload.rs`
- done when: `POST /upload/drive/v3/files?uploadType=multipart` com metadata `{ name, parents: [folder_id] }`, `supportsAllDrives=true`, `fields=id,webViewLink`; `webViewLink` persistido em `jobs.remote_state.web_view_link`; teste com `wiremock` cobre requisição bem formada e parse de `id`/`webViewLink`.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml gdrive::upload_simple`

### T-4.6 — Command `pick_service_account_file`
- story: RF-010
- wave: 2 · depends_on: [T-4.1]
- model: sonnet · agent: backend-engineer · skills: rust-pro, security-best-practices
- files: `src-tauri/src/commands/credentials.rs`
- done when: dialog filtrado a `*.json`; valida `type == "service_account"`, presença de `client_email` e `private_key`; grava o conteúdo integral no keyring **sempre fragmentado** em pedaços de 1024 chars (`gdrive.service_account_json.{0..n}` + `.count`; Credential Manager grava UTF-16 ⇒ ~1280 chars úteis por blob), reconcatenando na leitura; retorna `{ file_name, size, client_email, project_id }` — **nunca o conteúdo**; caminho original do arquivo não é logado.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml credentials::chunk_roundtrip` (JSON de 3,2 KB → 4 fragmentos, round-trip idêntico); auditoria manual: `%APPDATA%/osystems-sync/config.json` e `state.db` não contêm a string `private_key` após salvar

### T-4.9 — Settings: slider QoS Drive
- story: RF-050, RF-082
- wave: 2 · depends_on: [T-3.9]
- model: sonnet · agent: frontend-engineer · skills: tailwind-patterns, frontend-ui-system
- files: `src/components/settings/QosModule.tsx`
- done when: segundo slider (Drive) ao lado do de S3 (`T-3.12`), mesmo comportamento, chama `set_qos({destination:"gdrive", ...})`.
- verify: `npx vitest run src/components/settings/__tests__/QosModule.test.tsx`

### T-4.3 — `gdrive::upload` resumable (`>= 8 MB`)
- story: RF-012
- wave: 3 · depends_on: [T-4.2]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/uploaders/gdrive/upload.rs`
- done when: `uploadType=resumable`, chunks de 16 MB, retomada via `Content-Range` após falha transitória, `remote_state` (session_uri + offset) persistido em `jobs.remote_state` para sobreviver a reinício; teste com `wiremock` simula queda no chunk 3 e confirma retomada sem reiniciar do zero.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml gdrive::upload_resumable`

### T-4.4 — `gdrive::mod` idempotência + `test_connection`
- story: RF-011, RF-013
- wave: 4 · depends_on: [T-4.3]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/uploaders/gdrive/mod.rs`
- done when: `files.list` com `q="name='X' and 'folder_id' in parents and trashed=false"` e `fields=files(id,name,size,sha256Checksum,webViewLink)` compara `sha256Checksum` com o hash local (fallback: `size`, reenvia em dúvida); se bate, `done` sem reenviar; `session_uri` que devolve 404/410 → limpa `remote_state` e reinicia do zero; `test_connection` = `GET /drive/v3/files/{folder_id}?fields=id,name,driveId` **+ criar/apagar `.osystems-sync-probe` de 0 bytes** (SA só-leitura não pode reportar Conectado); pasta não compartilhada com a SA retorna `UploadError::Auth` com mensagem incluindo o `client_email`.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml gdrive::idempotent gdrive::test_connection`

### T-4.7 — Command `test_connection` (Drive)
- story: RF-014
- wave: 5 · depends_on: [T-4.4]
- model: sonnet · agent: backend-engineer · skills: rust-pro
- files: `src-tauri/src/commands/credentials.rs`
- done when: `test_connection({destination:"gdrive"})` retorna `{ ok, message, latency_ms }`; erro 403/404/JSON inválido produz mensagem específica e legível.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml`

### T-4.8 — Settings: módulo Google Drive completo
- story: RF-080
- wave: 6 · depends_on: [T-4.6, T-4.7]
- model: sonnet · agent: frontend-engineer · skills: tailwind-patterns, frontend-ui-system
- files: `src/components/settings/DriveModule.tsx`
- done when: card com status (Conectado/Online), upload do JSON via `pick_service_account_file` (mostra nome, tamanho, `client_email`), Folder ID com copiar/abrir, toggle "subpastas por data" **desabilitado** (Should, fase 5) e "checksum pré-upload" sempre ligado (tooltip explicando), rodapé com botão Testar + resultado.
- verify: `npx vitest run src/components/settings/__tests__/DriveModule.test.tsx`

### T-4.10 — Banner `auth-required`
- story: RF-070 (parcial), RF-032 (evento)
- wave: 6 · depends_on: [T-4.4, T-4.7]
- model: sonnet · agent: frontend-engineer · skills: tailwind-patterns, frontend-ui-system
- files: `src/components/dashboard/AuthRequiredBanner.tsx`
- done when: evento `auth-required` (`{ destination, hint }`) exibe banner no Dashboard com o `hint` (e-mail da SA a compartilhar / política IAM faltante) e ação "Ir para Configurações".
- verify: `npx vitest run src/components/dashboard/__tests__/AuthRequiredBanner.test.tsx`

**Riscos da fase**: tenant Google sem Workspace ou pasta não compartilhável com a SA (risco Alto do PRD §8) — mitigado pela mensagem de erro de `T-4.4` que já entrega o `client_email` exato; cota de 750 GB/dia da SA — erro `storageQuotaExceeded` deve ser classificado `Transient` com backoff de 1 h (ajustar `classify_http_status` de `T-3.2` se necessário nesta fase).

---

### Fase 5 — Ciclo de vida, energia e Should-haves

**Objetivo**: robustez de longa duração (suspensão, shutdown) e os recursos Should-have já desenhados nos mockups.

**Critério de aceite** (SPEC §10, fase 5): suspender/retomar → pendentes processados; Sair com upload em voo encerra em ≤ 30 s e retoma ao reabrir.

**Waves**:
- Wave 1: `T-5.1`, `T-5.2`, `T-5.3`, `T-5.4`
- Wave 2: `T-5.5`, `T-5.6`, `T-5.7`
- Wave 3: `T-5.8`, `T-5.9`, `T-5.10`

**Tasks**:

### T-5.1 — `core::power`
- story: RF-093, RF-094
- wave: 1 · depends_on: [T-0.10, T-2.3]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/power.rs`
- done when: `keep_awake=true` chama `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)` (sem forçar tela); loop de `tokio::time::interval(60s)` detecta suspensão (`elapsed > 2×interval`) e dispara `rescan()`; teste unitário simula o gap de tempo com `tokio::time::pause()`.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml power::`

### T-5.2 — `core::worker` pausar/retomar job individual
- story: RF-037
- wave: 1 · depends_on: [T-3.7]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/worker.rs`
- done when: `pause_job(id)` cancela o `CancellationToken` do job e grava `status='paused'` preservando `remote_state`; `resume_job(id)` volta a `pending`; worker ignora jobs `paused`; teste unitário cobre o ciclo pause→resume preservando `remote_state`.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml worker::pause`

### T-5.3 — `gdrive::folders` (subpastas por data)
- story: RF-015
- wave: 1 · depends_on: [T-4.4]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/uploaders/gdrive/folders.rs`
- done when: resolve/cria `YYYY/MM/DD_backup/` sob o `folder_id`; cache `HashMap<date, folder_id>` invalidado à meia-noite; pasta criada uma única vez por dia (teste garante 1 chamada de criação para 3 uploads no mesmo dia).
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml gdrive::folders`

### T-5.4 — `core::throttle` modo noturno
- story: RF-053
- wave: 1 · depends_on: [T-3.6]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/throttle.rs`
- done when: task de 1 min compara hora local com a janela configurável (padrão 23:00–06:00) e chama `set_limit(0)`/restaura o teto anterior; teste com `tokio::time::pause()` cobre entrada e saída da janela.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml throttle::night_mode`

### T-5.5 — Commands `pause_job`/`resume_job` + shutdown gracioso
- story: RF-037 (IPC), RF-095
- wave: 2 · depends_on: [T-5.1, T-5.2]
- model: sonnet · agent: backend-engineer · skills: rust-pro
- files: `src-tauri/src/commands/queue.rs`, `src-tauri/src/lib.rs`
- done when: commands `pause_job`/`resume_job` expostos; handler de saída (tray "Sair" e fechar via `Cmd/Alt+F4` se aplicável) aguarda uploads em voo até 30 s, depois cancela — jobs cancelados voltam a `pending` no próximo boot (reaproveita recovery de `T-3.7`).
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml`; manual: iniciar upload de arquivo grande, clicar "Sair" no tray, cronometrar ≤ 30 s até o processo encerrar, reabrir e confirmar retomada

### T-5.6 — `health` com latência (ping)
- story: RF-069
- wave: 2 · depends_on: [T-3.8]
- model: sonnet · agent: backend-engineer · skills: rust-pro, tdd-workflow
- files: `src-tauri/crates/core/src/health.rs`, `src/components/shell/StatusBar.tsx`
- done when: `HEAD` leve mede round-trip a cada 60 s e popula `latency_ms` em `AppStatus`; StatusBar exibe "Ping: N ms" por destino.
- verify: `cargo test -p core --manifest-path src-tauri/Cargo.toml health::latency`; `npx vitest run src/components/shell/__tests__/StatusBar.test.tsx`

### T-5.7 — Notificações nativas
- story: RF-096
- wave: 2 · depends_on: [T-3.9]
- model: sonnet · agent: backend-engineer · skills: rust-pro
- files: `src-tauri/src/lib.rs`, `src-tauri/Cargo.toml` (dep `tauri-plugin-notification`)
- done when: toast nativo do Windows ao job virar `failed` e ao destino entrar em `auth-required`, agrupado a no máximo 1 por minuto por tipo.
- verify: manual — provocar uma falha (bucket inválido) e confirmar 1 toast com nome do arquivo e destino; disparar 5 falhas em 10 s e confirmar no máximo 1 notificação

### T-5.8 — Estados vazios/erro completos
- story: RF-070 (completo)
- wave: 3 · depends_on: [T-5.5, T-4.10]
- model: sonnet · agent: frontend-engineer · skills: tailwind-patterns, frontend-ui-system
- files: `src/components/dashboard/EmptyStateNoCredentials.tsx`
- done when: além de "sem pasta" (`T-2.11`) e `auth-required` (`T-4.10`), existe o estado "sem credenciais" (CTA "Configurar"); todos os 3 estados alcançáveis navegando o app normalmente (não só via teste).
- verify: `npx vitest run src/components/dashboard/__tests__/EmptyStateNoCredentials.test.tsx`

### T-5.9 — UI dos Should-haves (pausar job, noturno, subpastas)
- story: RF-037, RF-053, RF-015 (habilitação na UI)
- wave: 3 · depends_on: [T-5.5, T-5.3, T-5.4]
- model: sonnet · agent: frontend-engineer · skills: tailwind-patterns, frontend-ui-system
- files: `src/components/dashboard/JobRow.tsx`, `src/components/settings/QosModule.tsx`, `src/components/settings/DriveModule.tsx`
- done when: ícone pausar/retomar por linha chama `pause_job`/`resume_job`; toggle "Modo Noturno" habilitado (antes "em breve"); toggle "subpastas por data" habilitado no DriveModule.
- verify: `npx vitest run src/components/dashboard/__tests__/JobRow.pause.test.tsx src/components/settings/__tests__/QosModule.test.tsx`

### T-5.10 — Tray e title bar com estado real
- story: RF-090 (estados reais), RF-097 (snap layout)
- wave: 3 · depends_on: [T-5.5]
- model: sonnet · agent: backend-engineer · skills: rust-pro
- files: `src-tauri/src/tray.rs`
- done when: ícone do tray muda para "trabalhando" em ≤ 2 s após o primeiro job `uploading` e para "erro" após o primeiro `failed`, refletindo `AppStatus` real (não mais mock de `T-0.7`).
- verify: manual — copiar arquivo para a pasta monitorada e cronometrar a troca do ícone (≤ 2 s); botão maximizar aciona snap layouts do Win11

**Riscos da fase**: `SetThreadExecutionState` pode não ter efeito em VMs/sessões RDP — documentar como limitação conhecida, não bloqueia MVP; upload de 5 GB pode não caber na janela de 30 s do shutdown gracioso — comportamento aceito é cancelar e retomar ao reabrir (já coberto pelo recovery de `T-3.7`).

---

### Fase 6 — Hardening e distribuição

**Objetivo**: provar estabilidade de longa duração, acessibilidade e empacotar o instalador.

**Critério de aceite** (SPEC §10, fase 6): RSS estável (±10 %), zero jobs perdidos, zero duplicatas remotas.

**Waves**:
- Wave 1: `T-6.1`, `T-6.2`, `T-6.3`
- Wave 2: `T-6.4`, `T-6.6`
- Wave 3: `T-6.5`, `T-6.7`

**Tasks**:

### T-6.1 — Script de teste de longa duração
- story: RNF-001
- wave: 1 · depends_on: [todas as fases 0–5 fechadas]
- model: sonnet · agent: qa-automation-engineer · skills: e2e-testing-patterns
- files: `tests/e2e/longevity.ts` (ou script Node/PowerShell equivalente)
- done when: script gera 1 arquivo/minuto por 72 h na pasta monitorada, alternando tamanhos (KB a algumas centenas de MB) e amostra RSS do processo a cada 5 min em CSV.
- verify: execução de teste curto (10 min, 1 arquivo/15 s) confirma que o script gera arquivos e grava amostras de RSS sem erro

### T-6.2 — Auditoria de acessibilidade
- story: RNF-013
- wave: 1 · depends_on: [T-5.8, T-5.9]
- model: sonnet · agent: frontend-engineer · skills: accessibility, tailwind-patterns
- files: `src/components/**/__tests__/*.a11y.test.tsx`
- done when: `axe-core` integrado ao Vitest cobre Dashboard e Settings sem violação; tabela de contraste (texto/superfície ≥ 4.5:1, terciário decorativo ≥ 3:1) documentada e batendo com `design/DESIGN.md`; navegação por teclado testada nas 2 telas; `aria-label` presente em todo ícone de ação.
- verify: `npx vitest run src/components/**/__tests__/*.a11y.test.tsx`

### T-6.3 — Revisão de segurança final
- story: RNF-002, RNF-003, RNF-009, RNF-011, RNF-012, RNF-015
- wave: 1 · depends_on: [todas as fases 0–5 fechadas]
- model: opus · agent: security-auditor · skills: security-best-practices
- files: (revisão, sem novo código) `src-tauri/capabilities/default.json`, `src-tauri/tauri.conf.json`, `src-tauri/crates/core/src/credentials.rs`
- done when: `cargo tree -p core | grep tauri` vazio (RNF-009); `capabilities/default.json` lista só os commands realmente usados; CSP sem CDN; grep em `%APPDATA%`, `config.json`, `state.db` e nos logs confirma zero ocorrência de secret/JSON da SA; permissões IAM/escopo documentadas batem com `SPEC.md §9`.
- verify: `cargo tree -p core --manifest-path src-tauri/Cargo.toml | grep -i tauri` vazio; `grep -rIE "private_key|BEGIN PRIVATE KEY|secret_access_key" "%APPDATA%/osystems-sync"` vazio (ambiente de teste)

### T-6.4 — Instalador NSIS/MSI assinado
- story: RNF-016
- wave: 2 · depends_on: [T-6.3]
- model: sonnet · agent: devops-engineer · skills: coding-guidelines
- files: `src-tauri/tauri.conf.json` (`bundle.windows`)
- done when: `npm run tauri build` gera instalador NSIS/MSI; instalação/desinstalação limpa; autostart removido na desinstalação; assinatura de código documentada (certificado fornecido pelo time, fora do escopo de código).
- verify: `npm run tauri build`; instalar em VM Windows limpa, verificar entrada de autostart, desinstalar e confirmar remoção (checagem manual)

### T-6.6 — Execução e relatório do teste de 72 h
- story: RNF-001, métricas PRD §6
- wave: 2 · depends_on: [T-6.1]
- model: sonnet · agent: qa-automation-engineer · skills: e2e-testing-patterns, verification-before-completion
- files: `tests/e2e/longevity-report.md` (saída do script, não editar à mão)
- done when: 500 arquivos processados em 72 h; contagem local = contagem S3 = contagem Drive; zero duplicatas (`ListObjects`/`files.list` sem nomes repetidos com mesmo hash); RSS varia ≤ 10 %.
- verify: relatório gerado por `T-6.1` mostra as 4 métricas dentro do alvo

### T-6.5 — Auto-update (opcional)
- story: infra (Nice-to-have, PRD §7 "Futuro")
- wave: 3 · depends_on: [T-6.4]
- model: haiku · agent: devops-engineer · skills: coding-guidelines
- files: `src-tauri/tauri.conf.json` (`plugins.updater`), `src-tauri/Cargo.toml`
- done when: `tauri-plugin-updater` configurado apontando para um endpoint de release (placeholder documentado); build continua funcionando sem o endpoint disponível (fail-safe, não bloqueia o app).
- verify: `npm run tauri build` sem erro com o plugin habilitado

### T-6.7 — Readiness gate final
- story: infra (fecho do projeto)
- wave: 3 · depends_on: [T-6.2, T-6.3, T-6.4, T-6.6]
- model: opus · agent: code-reviewer · skills: readiness-gate, verification-before-completion
- files: (nenhum — checklist de fechamento)
- done when: todos os 6 *gate commands* de `§0` passam no workspace inteiro; matriz de rastreabilidade (`§4`) sem nenhum Must-have não mapeado; nenhuma tarefa das fases 0–6 em aberto.
- verify: `cargo test --workspace --manifest-path src-tauri/Cargo.toml && cargo clippy --workspace --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings && cargo fmt --all --manifest-path src-tauri/Cargo.toml -- --check && npm run build && npm run test && npm run typecheck`

**Riscos da fase**: drift de RSS só se manifesta depois de 72 h reais — se aparecer, a correção pode exigir revisitar arquitetura de uma fase anterior (mitigação: rodar uma amostra curta de 6–12 h antes de comprometer os 72 h completos); assinatura de código depende de certificado obtido fora do time de engenharia — risco de cronograma, não técnico.

---

## 4. Matriz de rastreabilidade

| Requisito | Prioridade | Fase | Task(s) |
|---|---|---|---|
| RF-001 | M | 1, 2 | T-1.1, T-2.6 |
| RF-002 | M | 2 | T-2.2, T-2.3 |
| RF-003 | M | 1, 2 | T-1.1, T-2.3 |
| RF-004 | M | 2 | T-2.5 |
| RF-005 | M | 2 | T-2.3, T-2.6 |
| RF-006 | M | 2 | T-2.3 |
| RF-010 | M | 3, 4 | T-3.1, T-4.6 |
| RF-011 | M | 4 | T-4.4, T-4.8 |
| RF-012 | M | 4 | T-4.2, T-4.3 |
| RF-013 | M | 4 | T-4.4 |
| RF-014 | M | 4 | T-4.7, T-4.8 |
| RF-015 | S | 5 | T-5.3, T-5.9 |
| RF-016 | M | 4 | T-4.1 |
| RF-020 | M | 3 | T-3.1, T-3.10 |
| RF-021 | M | 3 | T-3.3, T-3.4 |
| RF-022 | M | 3 | T-3.5 |
| RF-023 | M | 3 | T-3.3, T-3.10 |
| RF-024 | M | 3 | T-3.4 |
| RF-030 | M | 2 | T-2.4 |
| RF-031 | M | 3 | T-3.7 |
| RF-032 | M | 3, 4 | T-3.2, T-4.10 |
| RF-033 | M | 3 | T-3.9, T-3.11 |
| RF-034 | M | 3 | T-3.9 |
| RF-035 | M | 3 | T-3.9, T-3.11 |
| RF-036 | M | 2, 3 | T-2.8, T-3.9 |
| RF-037 | S | 5 | T-5.2, T-5.5, T-5.9 |
| RF-038 | M | 1, 3 | T-1.2, T-3.7 |
| RF-039 | M | 2, 3 | T-2.1, T-2.4, T-3.5 |
| RF-040 | M | 3 | T-3.7, T-3.11 |
| RF-050 | M | 3, 4 | T-3.6, T-3.12, T-4.9 |
| RF-051 | M | 3 | T-3.6, T-3.9 |
| RF-052 | M | 3 | T-3.6 |
| RF-053 | S | 5 | T-5.4, T-5.9 |
| RF-060 | M | 2 | T-2.8 |
| RF-061 | M | 2 | T-2.8 |
| RF-062 | M | 2 | T-2.9 |
| RF-063 | M | 2 | T-2.9 |
| RF-064 | M | 2 | T-2.6, T-2.9 |
| RF-065 | M | 2, 3 | T-2.9, T-3.11 |
| RF-066 | M | 2 | T-2.10 |
| RF-067 | M | 0, 3 | T-0.4, T-3.11 |
| RF-068 | M | 0, 3, 5 | T-0.5, T-3.8, T-5.6 |
| RF-069 | S | 5 | T-5.6 |
| RF-070 | M | 2, 4, 5 | T-2.11, T-4.10, T-5.8 |
| RF-080 | M | 4 | T-4.8 |
| RF-081 | M | 3 | T-3.10 |
| RF-082 | M | 3, 4, 5 | T-3.12, T-4.9, T-5.9 |
| RF-083 | M | 1 | T-1.5, T-1.9 |
| RF-084 | M | 1 | T-1.9 |
| RF-085 | M | 3, 4 | T-3.1, T-3.9, T-4.6 |
| RF-086 | M | 1 | T-1.7 |
| RF-090 | M | 0, 5 | T-0.7, T-5.10 |
| RF-091 | M | 0 | T-0.11 |
| RF-092 | M | 0 | T-0.8 |
| RF-093 | M | 5 | T-5.1 |
| RF-094 | M | 5 | T-5.1 |
| RF-095 | M | 5 | T-5.5 |
| RF-096 | S | 5 | T-5.7 |
| RF-097 | M | 0 | T-0.3 |
| RF-098 | M | 2, 3 | T-2.10, T-3.11 |
| RF-099 | M | 1 | T-1.3 |
| RNF-001 | — | 6 | T-6.1, T-6.6 |
| RNF-002 | — | 3, 4, 6 | T-3.1, T-4.6, T-6.3 |
| RNF-003 | — | 1, 3, 6 | T-1.11, T-3.9, T-6.3 |
| RNF-004 | — | 3, 4 | T-3.4, T-4.3 |
| RNF-005 | — | 1 | T-1.3 |
| RNF-006 | — | 1, 3 | T-1.2, T-3.7 |
| RNF-007 | — | 2 | T-2.3 |
| RNF-008 | — | 2 | T-2.6, T-2.9 |
| RNF-009 | — | 0, 6 | T-0.10, T-6.3 |
| RNF-010 | — | 1 | T-1.4 |
| RNF-011 | — | 0, 6 | T-0.1, T-6.3 |
| RNF-012 | — | 3, 4, 6 | T-3.1, T-4.1, T-6.3 |
| RNF-013 | — | 6 | T-6.2 |
| RNF-014 | — | 0, 6 | T-0.2, T-6.3 |
| RNF-015 | — | 1, 6 | T-1.3, T-6.3 |
| RNF-016 | — | 6 | T-6.4 |
| RNF-017 | — | 1 | T-1.6 |

Todos os requisitos `M` (Must-have) mapeiam para ≥ 1 tarefa nas fases 0–4 (com reforço/ligação de UI nas fases 5–6 onde aplicável). Todos os `S` (Should-have) mapeiam exclusivamente para a fase 5, conforme `PRD.md §7`. Nenhum `N` (Nice-to-have) tem tarefa própria numerada — RF-015/RF-037/RF-053/RF-069/RF-096 já cobrem os únicos itens Should desenhados; os itens "Futuro" restantes (auto-update, en-US, exportar CSV) ficam fora do manifesto de tarefas, exceto auto-update que ganhou `T-6.5` por já estar habilitado no `tauri.conf.json` da fase 6.

---

## 5. Riscos transversais & rollback

| Risco | Fase mais exposta | Mitigação / rollback |
|---|---|---|
| Tenant Google sem Workspace / pasta não compartilhável com a SA | 4 | Mensagem de erro com `client_email` exato (`T-4.4`); fallback OAuth documentado em `SPEC.md §12`, não implementado — se necessário, é um projeto à parte, não um rollback desta fase. |
| Cota do Drive (750 GB/dia) | 4, 6 | Classificar `storageQuotaExceeded` como `Transient` com backoff de 1 h; alertar na UI via `auth-required`-like banner. |
| Upload de 5 GB interrompido no meio | 3, 4 | `remote_state` (S3 `UploadId`+partes, Drive `session_uri`+offset) já persistido por design (`SPEC.md §5`) — retomar em vez de reiniciar do zero. |
| Suspensão do Windows mata sockets sem evento claro | 5 | Detector de gap de relógio (`T-5.1`) + rescan ao acordar; se a detecção falhar, o rescan periódico de startup ainda cobre o caso na próxima abertura. |
| Credential Manager indisponível (perfil roaming, política de grupo) | 3, 4 | Erro fatal legível na primeira tela; app não cai para arquivo plano — rollback é reportar o erro, nunca persistir segredo alternativo. |
| CSP bloqueia fontes/ícones do mockup | 0 | Fontes vendorizadas (`T-0.2`) e `lucide-react` (SVG inline) decididos antes de qualquer tela ser construída. |
| Escopo inflado pelos extras do mockup | todas | Todo `S`/`N` fica isolado na fase 5 (`§4`); o *readiness gate* de cada fase 0–4 falha se uma tarefa Must-have depender de uma tarefa Should. |
| Antivírus corporativo trava arquivo durante estabilização | 2 | Timeout de 30 min em `wait_until_stable` + log `warn`, segue mesmo assim (já no `done when` de `T-2.2`). |
| Custo de multipart órfão no S3 | 3 | `AbortMultipartUpload` em `T-3.4`; regra de ciclo de vida do bucket documentada na ajuda (fora do código). |
| RSS drift só aparece em teste de 72 h (tarde para corrigir arquitetura) | 6 | Amostra curta (6–12 h) antes de comprometer o teste completo (nota em `§3` Fase 6). |
| Rollback geral de uma fase | qualquer | Cada fase é um conjunto de commits atômicos (`feat(core): ...`, `feat(ui): ...`) sobre o *gate* da fase anterior já verde — reverter é `git revert` dos commits da fase, nunca da fase anterior. |

---

## 6. Fora de escopo

(Copiado de `PRD.md §7`.)

- Sincronização bidirecional ou download.
- Exclusão / renomeação remota espelhada.
- Múltiplas pastas monitoradas ou múltiplos buckets/folders.
- Serviço Windows (sem usuário logado) — arquitetura permite migrar depois (isolamento `core`/`tauri` de `RNF-009` existe justamente para isso).
- macOS / Linux.
- OAuth com conta Google pessoal (documentado como fallback em `SPEC.md §12`, não implementado).
- Criptografia client-side dos arquivos.

---

## 7. Verificação de fim de projeto

Executar após `T-6.7` (readiness gate) fechar:

1. **Script de 72 h** (`T-6.1`/`T-6.6`): 1 arquivo/minuto por 72 h, tamanhos variados (KB a algumas centenas de MB — não é necessário chegar a 5 GB nos 500 arquivos, mas ao menos 1 arquivo de teste deve exercitar o caminho de 5 GB isoladamente antes do teste de longa duração).
2. **Amostragem de RSS**: a cada 5 min, todo o teste; critério de aceite é variação ≤ 10 % entre a primeira hora estável e as últimas 24 h.
3. **Zero duplicatas remotas**: `aws s3api list-objects-v2 --bucket <bucket> --prefix <prefix>` e a API `files.list` do Drive não devem ter dois objetos com o mesmo `sha256`/nome.
4. **Zero jobs perdidos**: `SELECT count(*) FROM jobs WHERE status NOT IN ('done','archived')` deve ser 0 ao fim do teste (nenhum job preso em `pending`/`uploading`/`failed`).
5. **Instalador**: instalar o pacote NSIS/MSI (`T-6.4`) em uma VM Windows 11 limpa, configurar do zero (pasta, credenciais S3 e Drive) e cronometrar até o primeiro upload bem-sucedido — meta do PRD é ≤ 10 min; desinstalar e confirmar que a entrada de autostart some do Registro.
6. **Gate final**: os 6 comandos de `§0` (`cargo test`, `cargo clippy -D warnings`, `cargo fmt --check`, `npm run build`, `npm run test`, `npm run typecheck`) verdes no HEAD que será empacotado.
