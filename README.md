# oSystems Sync

App desktop que observa uma pasta no Windows e envia cada arquivo novo para o
**Google Drive** e o **Amazon S3**, em paralelo, com fila persistente e retomada
automática.

Tauri 2 (Rust) + React/TypeScript.

---

## O que ele faz

- **Observa uma pasta** (opcionalmente com subpastas) e detecta arquivos novos
  ou alterados.
- **Espera o arquivo estabilizar** antes de enviar — tamanho constante por N
  segundos *e* abertura exclusiva bem-sucedida, para não subir um arquivo que
  ainda está sendo escrito ou que o antivírus segura.
- **Envia para os dois destinos** de forma independente: um pode concluir
  enquanto o outro tenta de novo.
- **Retoma de onde parou** — multipart no S3, sessão resumable no Drive —
  inclusive depois de fechar o app.
- **Deduplica por SHA-256**: reenviar a mesma pasta não duplica nada.
- **Limita banda** por destino, com modo noturno.
- **Filtra na entrada** por extensão e por tamanho mínimo/máximo. "Atualizar
  Lista" reconcilia a fila com os filtros atuais nos dois sentidos: enfileira o
  que passou a qualificar e arquiva o que deixou de qualificar.

Credenciais ficam no **keyring do sistema** — nunca em JSON, `.env` ou SQLite, e
o renderer só recebe máscaras.

## Arquitetura

```
src-tauri/crates/core/   lógica de negócio, sem dependência de tauri
                         watcher · estabilização · fila SQLite · uploaders
                         throttle · credenciais · logging
src-tauri/src/           adapter Tauri: commands, events, tray, runtime
src/                     React: dashboard (fila + console de eventos), ajustes
```

O `core` não importa `tauri`. Toda struct que atravessa o IPC é exportada para
TypeScript por `ts-rs`, então `src/types/generated/` é gerado — nunca editado à
mão.

## Rodando

Requer Node 20+, Rust estável e as
[dependências do Tauri 2](https://tauri.app/start/prerequisites/).

```bash
npm install
npm run tauri dev
```

Verificação completa:

```bash
cargo test --workspace --manifest-path src-tauri/Cargo.toml
cargo clippy --workspace --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run test && npm run typecheck && npm run lint
```

Instalador Windows (NSIS, cruzado a partir do macOS/Linux via `cargo-xwin`):

```bash
npm run build:win
```

Para testar o S3 localmente sem AWS:

```bash
docker run -p 9000:9000 minio/minio server /data
# e aponte OSYSTEMS_SYNC_S3_ENDPOINT para http://localhost:9000
```

### Ao clonar

Dois caminhos ficam fora deste repositório e precisam ser recriados para
desenvolver a partir de um clone:

- **`.cargo/config.toml`** define `TS_RS_EXPORT_DIR = "src/types/generated"`.
  Sem ele, `cargo test` grava os tipos gerados em
  `src-tauri/crates/core/bindings/` e o `src/types/generated.ts` fica
  desatualizado em silêncio.
- **`tests/e2e/`** é o alvo de `npm run test:e2e:unit` e `npm run longevity`;
  esses dois scripts falham sem ela.

O restante — `cargo test`, `npm run test`, build e execução — funciona num clone
limpo.

## Autor

**Paulo Souza**

- [@plocemourasouza](https://www.instagram.com/plocemourasouza) no Instagram
- [@plocemourasouza](https://www.facebook.com/plocemourasouza) no Facebook
- [in/psouza](https://www.linkedin.com/in/psouza/) no LinkedIn
- [plocemourasouza](https://www.github.com/plocemourasouza) no GitHub
- plocemourasouza@gmail.com

## Licença

MIT — veja [LICENSE](LICENSE).
