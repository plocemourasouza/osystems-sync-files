# Changelog

Formato baseado em [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/).
Versionamento [semântico](https://semver.org/lang/pt-BR/).

A versão vive em **`VERSION`**, na raiz. Para subir:

```bash
node scripts/version.mjs 0.3.0   # grava VERSION e propaga
```

`package.json` e `[workspace.package]` são escritos pelo script;
`crates/core` herda do workspace e o instalador herda do `package.json`
(`tauri.conf.json` aponta para ele). `npm run version:check` roda no CI e falha
se algum divergir.

---

## [0.3.0] — 2026-09-08

Corrige a detecção de arquivos em pasta monitorada e a cegueira do app a erros.
Relatado em campo: monitorando a raiz de um volume (`E:\`), o console repetia
`varredura manual falhou` sem nenhum detalhe e arquivos já presentes no disco
nunca entravam na fila.

### Corrigido

- **A varredura abortava inteira no primeiro diretório ilegível.** `walk_dir`
  propagava com `?` qualquer erro de `read_dir`/`next_entry`/`symlink_metadata`.
  Na raiz de um volume Windows, `System Volume Information` nega acesso a todo
  mundo (os error 5) — e essa negativa, sozinha, fazia a varredura retornar
  `Err` com zero arquivos, em 100% das tentativas. Agora o erro é contado por
  entrada e a varredura segue; só falha de verdade se a **raiz** for
  inacessível. Efeito colateral do bug: como a tabela `files` nunca era
  populada, o atalho `size+mtime` nunca ativava e cada tentativa re-hasheava
  tudo do zero.
- **Caminhos de sistema do Windows são pulados** antes do `stat`
  (`System Volume Information`, `$RECYCLE.BIN`, `$Extend`, `Config.Msi`,
  `Recovery`, `$WinREAgent`, `pagefile.sys`, `hiberfil.sys`, `swapfile.sys`,
  `DumpStack.log*`).
- **Todo detalhe de erro do app era descartado.** `LogVisitor` só guardava
  `message`, `job_id` e `destination`, então **todo** `error = %err` do código
  sumia — no console, no stream da UI e no `app.log` em JSON. `LogLine` passa a
  carregar `error` e `path`, ambos com `redact` aplicado (um erro pode embutir
  URL pré-assinada, token ou fragmento de Service Account).
- **Quatro caminhos de varredura engoliam a falha em silêncio** (boot, retomada
  de suspensão, tray e `resume_watcher`): agora emitem `rescan-failed`, com
  banner na UI.
- **"Atualizar Lista" descartava a rejeição do `invoke`** — o único canal que
  carregava a mensagem real. Passa a exibi-la, junto de `scanned`, `errors` e
  `skipped_unreadable`.
- **Arquivo travado por antivírus ou gravador ficava 30 minutos mudo.** O
  sharing violation era ligado a `_sharing_violation` e nunca logado, nem em
  `debug`. Agora classifica os códigos 5 / 32 / 33 e loga com throttle (primeira
  ocorrência, mudança de tipo, depois no máximo a cada 60 s). O aviso de timeout
  carrega o último erro observado.

### Adicionado

- **Varredura de reconciliação periódica** (15 min). O `notify` 6.1.1 **não tem
  como reportar** estouro do buffer do `ReadDirectoryChangesW`: o `handle_event`
  do backend Windows ignora `_bytes_written` e só compara `error_code` com
  `ERROR_OPERATION_ABORTED`. Copiar 100 GB estoura esse buffer e os eventos são
  perdidos em silêncio, sem erro algum. Como a perda é indetectável por
  construção, a recuperação tem que ser incondicional.
- **Progresso da varredura na UI.** O botão mostra "Escaneando N/M" em vez de um
  spinner mudo — uma varredura de 100 GB leva minutos e era indistinguível de
  travamento.

### Alterado

- **Intake concorrente** (4 arquivos em voo). Era estritamente sequencial: um
  arquivo de 9,44 GB segurava a detecção de todos os outros por até 30 min de
  estabilização mais o SHA-256 completo. **A ordem da fila passa a refletir o
  fim do hash, não a detecção** — um arquivo pequeno detectado depois de um
  grande entra antes dele.
- **Uma varredura por vez, em todo o processo.** Havia seis iniciadores (timer,
  botão, tray, boot, retomada de suspensão, `resume_watcher`) sem coordenação
  alguma; duas varreduras simultâneas sobre 100 GB dobram a leitura de disco e
  disputam o mesmo disco que o uploader S3 precisa. A segunda chamada retorna
  `AlreadyInProgress` em vez de enfileirar — repetir a caminhada não traz nada,
  e a reconciliação é idempotente.
- **`read_metadata` e a abertura exclusiva saíram do runtime async** para
  `spawn_blocking`, com a semântica `share_mode(0)` intacta. Passa a importar
  agora que são 4 arquivos estabilizando ao mesmo tempo.

### Interno

- **Fonte única da versão.** O número vivia em quatro manifestos e subia à mão
  nos quatro. Agora vive em `VERSION`: `scripts/version.mjs` escreve dois
  derivados, os outros dois herdam sozinhos (Cargo workspace inheritance e o
  fallback do `tauri.conf.json` para o `package.json`). `npm run version:check`
  entra no CI para pegar divergência.
- **`dist-windows/` guarda só a última versão.** A cópia do instalador era
  manual e a pasta acumulou uma 0.1.0 ao lado da 0.2.0, sem indicar qual era a
  atual. `npm run build:win` agora termina em `scripts/package-win.mjs`, que
  limpa a pasta, copia o instalador da versão corrente e grava o `.sha256`.
  Recusa rodar se o build não bater com o `VERSION` — o diretório de bundle do
  Tauri também acumula, e publicar o instalador de um build anterior era
  possível.

## [0.2.0] — 2026-09-06

### Adicionado

- **Filtro de tamanho mínimo** (`watch.min_size_mb`). Junto com o máximo,
  define uma faixa fechada; `0` desliga cada ponta.
- **Reconciliação de filtros no rescan.** "Atualizar Lista" passa a agir nos
  dois sentidos: enfileira o que passou a qualificar e arquiva os jobs
  `pending`/`paused`/`failed` de arquivos que deixaram de qualificar, saíram do
  escopo (`recursive`) ou sumiram do disco — e restaura os arquivados que
  voltam a passar. Nunca toca `uploading` nem `done`, e nunca apaga: só
  `archived_at`.
- **Ticker no console recolhido.** A barra do console de eventos passa a exibir
  a última linha ao lado do chip "Rust Core", então o daemon continua
  observável com o painel fechado.
- **Card do autor** na sidebar, com contatos (Instagram, Facebook, LinkedIn,
  GitHub, e-mail e WhatsApp) que abrem no app externo correspondente.
- **README.**

### Alterado

- **Sem teto próprio de tamanho de arquivo.** `watch.max_size_mb` vira filtro
  opcional (`0` = sem limite); o limite passa a ser o do destino. A parte do
  multipart do S3 cresce com o arquivo, então o teto efetivo vai de 160 GiB
  para **5 TiB**. A concorrência encolhe conforme a parte cresce, para o pico
  de memória continuar limitado.
- **`Limpar Concluídos` agora baixa os KPIs.** `status_counts` passa a ignorar
  jobs arquivados — antes a tabela encolhia e os contadores ficavam parados.
- **Console de eventos ancorado no rodapé.** Recolher o painel entrega os
  ~220px dele à lista de arquivos, em vez de só encurtar a página. A lista rola
  por conta própria, com cabeçalho fixo.
- **Badge do watcher** movido para a barra de filtros da tabela, no extremo
  oposto de "Todos / Ativos / Concluídos / Falhas".
- Salvar qualquer campo de `watch` reinicia o watcher (comparação estrutural em
  vez de lista de campos mantida à mão).

### Corrigido

- **Filtros de tamanho não chegavam ao watcher em execução.** `min_size_mb` e
  `max_size_mb` eram gravados mas ignorados até reiniciar o app.
- **`stabilize_seconds` não fazia nada.** Era validado, persistido e exibido,
  enquanto o runtime usava o valor padrão fixo. Agora é derivado a cada geração
  de watcher, e ganhou validação (1..60).
- **`Atualizar Lista` não atualizava a tabela.** Só os KPIs eram recarregados;
  a lista só mudava ao trocar o filtro de status.
- **`claim_next` e `retry_all_failed` ignoravam `archived_at`** — um job
  arquivado seria enviado assim mesmo, e "Reenviar Falhas" o ressuscitaria.
- **Knob dos toggles saía do trilho.** O thumb era `absolute` sem inset
  horizontal, então partia do centro do botão.
- **Setas dos campos numéricos apareciam pretas no Windows.** O documento não
  declarava `color-scheme: dark`; o spinner nativo foi substituído por
  chevrons próprios, iguais aos do `Select`.

## [0.1.0] — 2026-09-05

Primeira versão. Watcher, fila persistente, uploaders de Google Drive
(Service Account) e Amazon S3, throttle por destino com modo noturno,
credenciais no keyring, dashboard e tela de ajustes.
