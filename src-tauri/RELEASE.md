# RELEASE.md — oSystems Sync

Guia de build, assinatura e distribuição do instalador Windows (NSIS/MSI). Referência: `PLAN.md`
T-6.4 (RNF-016), `SPEC.md §12` (decisão "Fase 6 NSIS `currentUser` + hook `NSIS_HOOK_PREUNINSTALL`").

> Esta aplicação só produz instalador **no Windows** — o bundler NSIS/MSI do Tauri não roda em
> macOS/Linux. A máquina de desenvolvimento atual é macOS (`SPEC.md §12`, decisão "Fase 0 Máquina de
> dev é macOS"); este documento cobre o fluxo real de release, que roda em `windows-latest` via CI
> (`.github/workflows/release.yml`) ou numa máquina/VM Windows.

---

## 1. Build local (Windows)

Pré-requisitos: Node 22, Rust stable (`rustup`), WebView2 Runtime (já vem no Windows 11), e as
dependências nativas do Tauri (`cargo install tauri-cli` não é necessário — o projeto usa
`@tauri-apps/cli` via `npm`).

```powershell
npm ci
npm run build          # build do frontend (tsc + vite)
cargo test --workspace --manifest-path src-tauri/Cargo.toml
npm run tauri build    # gera NSIS + MSI em src-tauri/target/release/bundle/
```

Saída:
- `src-tauri/target/release/bundle/nsis/oSystems Sync_<version>_x64-setup.exe`
- `src-tauri/target/release/bundle/msi/oSystems Sync_<version>_x64_pt-BR.msi`

O perfil `[profile.release]` em `src-tauri/Cargo.toml` (`opt-level = 3`, `lto = "thin"`,
`codegen-units = 1`, `strip = true`) prioriza throughput de upload (hash SHA-256 full-file, chunking
S3/Drive — caminhos quentes do daemon) sobre tamanho de binário; o build de release é mais lento que
um `cargo build` comum por causa do LTO.

---

## 2. Assinatura de código (`signtool`)

O certificado de assinatura é **fornecido pelo time**, fora do escopo de código deste projeto —
`tauri.conf.json` já traz os campos prontos, só falta o certificado real:

```json
"bundle": {
  "windows": {
    "certificateThumbprint": null,      // preencher com o thumbprint SHA-1 do certificado
    "digestAlgorithm": "sha256",
    "timestampUrl": "http://timestamp.digicert.com"
  }
}
```

Duas formas de assinar, dependendo do que o time entregar:

### Opção A — certificado instalado no cert store do Windows (thumbprint)
1. Instalar o `.pfx` no cert store do usuário/máquina que roda o build (`certmgr.msc` ou
   `Import-PfxCertificate`).
2. Obter o thumbprint SHA-1: `(Get-PfxCertificate -FilePath cert.pfx).Thumbprint`.
3. Preencher `bundle.windows.certificateThumbprint` em `tauri.conf.json` com esse valor.
4. `npm run tauri build` invoca `signtool.exe` automaticamente (já vem no Windows SDK, presente em
   `windows-latest`).

### Opção B — comando de assinatura customizado (HSM / assinatura em nuvem)
Se o time usa um provedor como Azure Key Vault / AWS Signer em vez de um `.pfx` local, usar
`bundle.windows.signCommand` em vez de `certificateThumbprint`:

```json
"signCommand": "azuresigntool sign -kvu %AZURE_KEY_VAULT_URI% -kvi %AZURE_CLIENT_ID% -kvs %AZURE_CLIENT_SECRET% -kvc %AZURE_CERT_NAME% -tr http://timestamp.digicert.com -td sha256 %1"
```

`%1` é substituído pelo caminho do artefato a assinar. Nunca commitar segredos do provedor — passar
via variável de ambiente no runner de CI (`secrets.*` no GitHub Actions).

Em ambos os casos, `timestampUrl` garante que a assinatura continue válida após o certificado expirar
(RFC 3161 timestamping) — não removê-lo.

O passo de assinatura no CI (`.github/workflows/release.yml`) está **comentado** com instruções: a
Opção A e a Opção B ficam documentadas ali, prontas para descomentar assim que o certificado chegar.

**Release gate (T-6.3 audit, VULN-002):** todo push de tag `v*` roda um step
"Verify installer will be signed" *antes* do build, que falha (`::error::`) se
`bundle.windows.certificateThumbprint` **e** `bundle.windows.signCommand` estiverem
ambos nulos/ausentes em `tauri.conf.json` — um instalador não assinado nunca deve sair
de uma tag de release por acidente. `workflow_dispatch` manual não é bloqueado (usado
para testes locais com certificado de dev). Para um release de emergência sem
certificado, definir a variável de repositório `ALLOW_UNSIGNED=true` (break-glass
documentado; o gate ainda emite `::warning::` no log para deixar rastro).

---

## 3. Modos de instalação (NSIS)

`bundle.windows.nsis.installMode: "currentUser"` (decisão registrada em `SPEC.md §12`):

- Instala em um diretório que **não exige Administrador** (ex.: `%LOCALAPPDATA%\Programs\oSystems Sync`).
- Metadados do instalador ficam em `HKCU`, não `HKLM` — consistente com `keyring`/Windows Credential
  Manager (por usuário) e com `tauri-plugin-autostart`, que também escreve em `HKCU`.
- Alternativas disponíveis no schema, não usadas no MVP: `perMachine` (exige Administrador, grava em
  `HKLM`, instala em `Program Files`) e `both` (instalador pergunta ao usuário). Revisitar apenas se
  surgir um requisito de deploy multiusuário/central.

Idiomas do instalador: `["PortugueseBR", "English"]`, sem seletor de idioma na tela
(`displayLanguageSelector: false`) — segue o idioma do sistema, com pt-BR como padrão do produto
(`design/DESIGN.md`, princípio "pt-BR em toda a superfície do produto"). O MSI (WiX) usa
`language: ["pt-BR"]`.

---

## 4. O que o hook de desinstalação faz

`src-tauri/nsis/hooks.nsh` define `NSIS_HOOK_PREUNINSTALL`, executado pelo desinstalador do NSIS
**antes** de remover arquivos/registro (RNF-016: "autostart removido na desinstalação"):

1. **Mata o processo em execução** — `taskkill /IM osystems-sync.exe /F` — para que o desinstalador
   consiga apagar o `.exe`/DLLs sem "arquivo em uso". `osystems-sync.exe` é o nome do binário gerado a
   partir do pacote Cargo `osystems-sync` (`src-tauri/Cargo.toml`), sem `[[bin]]` explícito.
2. **Remove a entrada de autostart** — `DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "oSystems Sync"`.
   O nome do valor é `"oSystems Sync"` porque `tauri-plugin-autostart` (v2.5.1), quando nenhum
   `.app_name()` é configurado em `src-tauri/src/lib.rs`, usa `app.package_info().name` — que resolve
   para `productName` de `tauri.conf.json` (confirmado lendo o código-fonte de
   `tauri-plugin-autostart-2.5.1` e de sua dependência `auto-launch-0.5.0`, que escreve/apaga
   exatamente esse valor em `enable()`/`disable()`).
3. **Limpa o espelho do Task Manager** — `auto-launch` também grava um valor com o mesmo nome em
   `HKCU\...\Explorer\StartupApproved\Run` quando o autostart é habilitado (para o Task Manager não
   marcá-lo como "desabilitado"); o hook remove essa entrada também. Não falha se a chave/valor não
   existir (autostart nunca habilitado).

---

## 5. Como verificar a remoção do autostart (checagem manual, VM Windows limpa)

1. Instalar o `.exe`/`.msi`, abrir o app, habilitar "Iniciar com o Windows" (RF-092) na tela de
   Configurações.
2. Confirmar a entrada: `reg query "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v "oSystems Sync"`
   deve retornar um valor (caminho do `.exe` + `--minimized`).
3. Desinstalar via "Aplicativos e recursos" (ou `oSystems Sync/Uninstall oSystems Sync.exe`).
4. Repetir a consulta do passo 2 — deve retornar `ERROR: The system was unable to find the specified
   registry key or value.` (chave/valor removido).
5. Opcional: `reg query "HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" /v "oSystems Sync"` também deve falhar.
6. Confirmar que a pasta de instalação e o atalho do Menu Iniciar (`oSystems Sync`,
   `startMenuFolder`) foram removidos.

---

## 6. Fluxo de CI (tags → release draft)

`.github/workflows/release.yml`:

- Dispara em `push` de tag `v*` (ex.: `v1.0.0`) ou manualmente (`workflow_dispatch`).
- Roda em `windows-latest` (mesmo runner do `ci.yml`, pelos mesmos motivos: crates `windows` e
  `keyring` só compilam ali).
- `npm ci` → `npm run build` → `cargo test --workspace` (mesmo gate de qualidade do `ci.yml`) →
  `npm run tauri build`.
- Sobe os instaladores (`.exe` do NSIS, `.msi` do WiX) como artifact do workflow e, se o gatilho foi
  uma tag `v*`, cria uma **GitHub Release em modo draft** (`softprops/action-gh-release@v2`) com os
  dois arquivos anexados — revisão humana antes de publicar.
- Assinatura de código fica comentada no workflow (§2 acima) até o certificado do time chegar.
- `TAURI_SIGNING_PRIVATE_KEY`/`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` já são passados ao `tauri build`
  como env vars vazias — inofensivo enquanto `bundle.createUpdaterArtifacts: false` (T-6.5, opcional);
  passam a ser obrigatórios (via GitHub Secrets) quando o `tauri-plugin-updater` for habilitado.

---

## 7. O que só pode ser verificado no Windows

Esta lista existe porque a máquina de dev é macOS (`SPEC.md §12`) — nenhum destes itens pode ser
provado localmente, apenas em CI (`windows-latest`) ou VM Windows:

- Geração real dos artefatos NSIS/MSI (`npm run tauri build`, sem `--debug`/`--bundles app`).
- Instalação/desinstalação limpas (arquivos, atalhos, registro).
- Comportamento do hook `NSIS_HOOK_PREUNINSTALL` (`taskkill`, `DeleteRegValue`).
- Entrada de autostart em `HKCU\...\Run` e sua remoção (§5 acima).
- Assinatura de código com `signtool` (Opção A/B, §2 acima) e verificação via
  `signtool verify /pa "instalador.exe"`.
- Seletor de idioma do NSIS (`PortugueseBR`/`English`) e o MSI em `pt-BR`.

No macOS, o que foi provado localmente (T-6.4) foi apenas que a configuração do bundler é válida e os
ícones estão corretos, via `npm run tauri build -- --debug --bundles app` (gera `.app` do macOS, não
o instalador Windows — ver saída do comando de verificação no PR/task).

## 8. Cross-compile a partir do macOS (NSIS apenas)

Gera o instalador NSIS x64 sem máquina Windows (MSI/WiX continua exigindo Windows):

```bash
brew install nsis llvm cmake nasm
rustup target add x86_64-pc-windows-msvc
cargo install cargo-xwin --locked
npm run build:win     # → src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/*.exe
```

Pré-requisito de dependências: o SDK da AWS usa `rustls` + `ring` (sem `aws-lc-sys`, que
não cruza para MSVC) — ver `SPEC.md §12`. O binário sai **sem assinatura** (SmartScreen avisa);
assine com `signtool` no Windows ou pela CI (`release.yml`) antes de distribuir.
Warnings `LNK4099` (PDB do CRT ausente) são esperados e inofensivos.
