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

## [Não lançado]

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
