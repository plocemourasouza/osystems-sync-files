# STATUS — oSystems Sync (atualizado 2026-09-05)

## Estado
- **Fases 0–6 code-complete.** 69/72 tasks do `PLAN.md` concluídas; T-6.5 (auto-update) fora de escopo (PRD §7 "Futuro"); T-6.6 (72 h) e assinatura dependem do usuário.
- Gates (última execução): Vitest **459/459** · `cargo test` **344/0** · clippy/fmt/tsc/eslint limpos · build ok · core sem `tauri` · 0 CDN.
- Instalador Windows (NSIS x64, **não assinado**) gerado no macOS via `npm run build:win`:
  `dist-windows/oSystems Sync_0.1.0_x64-setup.exe` — SHA-256 `2b248af87ae245c88dc655ea2bffbb6cc787d7a3160d12299b99fec6f59c42e2`.
- Aceite E2E S3 **real** passou (bucket de teste do usuário): put 1 MiB, multipart 20 MiB, reenvio idempotente. Drive E2E ainda não testado (falta Service Account).

## Bugs de campo corrigidos em 2026-09-05
1. `test_connection` falhava sem `s3:DeleteObject` → delete do probe virou best-effort.
2. Windows `canonicalize` gerava `\\?\C:\...` → `paths::canonicalize_clean` + migração automática em `Repo::open`.
3. Dashboard em 1366×768: status × ações sobrepostos, console quebrando, log grande → colunas fixas por breakpoint, ações ≤3 + menu em portal, console compacto.
4. Auditoria de UI: badges sem pill, `ErrorText` (ícone + cor AA), sem `text-white`.

## Pendente do usuário
- Reinstalar e retestar no Windows (tray, autostart, suspensão, layout).
- **Rotacionar a chave S3** de `chaves.md` (arquivo está no `.gitignore`; mover para fora do repo).
- Service Account + Folder ID do Drive para aceite E2E do Drive.
- `git init` + primeiro commit (nunca foi feito commit).
- Certificado de code signing (CI recusa release sem assinatura).
- Teste de 72 h: `npm run longevity -- --dir <pasta>` → `npm run longevity:report`.

## Como rodar
```bash
export PATH="$HOME/.cargo/bin:$PATH"
npm run tauri dev                 # app (macOS/Windows)
npm run dev -- --host 127.0.0.1   # só UI; abra /dashboard?mock=1 para dados de exemplo (dev only)
npm run test && cargo test --workspace --manifest-path src-tauri/Cargo.toml
npm run build:win                 # instalador NSIS via cargo-xwin (ver src-tauri/RELEASE.md §8)
cargo run -p osystems-sync-core --manifest-path src-tauri/Cargo.toml --example s3_diag   # diagnóstico S3 (env OSYSTEMS_SYNC_S3_*)
```

## Backlog v1.1 (decisões no osforge-db)
- `retry_after` de quota do Drive não atravessa `UploadError`.
- `open_in_explorer` sem contenção de caminho (defesa em profundidade).
- Purga de credenciais do keyring na desinstalação.
- Verificação de dono em `find_child` (subpastas do Drive).
- Auto-update (`tauri-plugin-updater`).

## Onde está o quê
`PRD.md` (o quê) · `SPEC.md` (como, §12 decisões) · `design/` (visual) · `PLAN.md` (tasks) · `tasks/lessons.md` (40+ lições) · `.specs/project/DECISIONS.md` (ADRs) · `osforge-db resume osystems-sync` (estado vivo) · Canvas `http://localhost:4242/?a=osystems-sync-final`.
