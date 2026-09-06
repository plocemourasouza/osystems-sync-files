# Decisões de arquitetura — oSystems Sync

Registra as decisões técnicas não-óbvias que moldaram o escopo, a arquitetura e a UI do oSystems Sync.

**Data**: 2026-09-04 · **Status**: aceita

---

## ADR-001 — Service Account em vez de OAuth para Google Drive

**Contexto**
A aplicação precisa rodar de forma ininterrupta por 10+ dias sem intervenção humana. OAuth com conta pessoal do Google exige refresh token, que pode expirar ou ser revogado pelo usuário. OAuth com *installed app* exige browser e redirect_uri no cliente desktop.

**Decisão**
Usar **Service Account (JSON com chave privada)** para autenticar contra Google Drive. Zero interação humana; JWT gerado localmente com validade de 1 h, renovado automaticamente.

**Consequências**
- ✅ Sem browser, sem redirect_uri, sem expiração surpresa de token.
- ✅ Previsível e isolado: SA só tem acesso ao que for compartilhado com seu e-mail.
- ❌ Pasta no Drive deve ser compartilhada com a SA (ou estar em Shared Drive).
- ❌ Escopo necessário é `drive` (não `drive.file`), pois a SA escreve em pasta criada por terceiro.
- ❌ JSON com chave privada é sensível — requer keyring do SO.

**Alternativas descartadas**
- OAuth *installed app*: exige browser; escape hatch para fases futuras, documentado em SPEC §12.

---

## ADR-002 — Nome do produto: oSystems Sync

**Contexto**
O projeto v1 tinha nome de trabalho genérico. Os assets v2 definiram a identidade visual e funcional do produto.

**Decisão**
Nome canônico: **oSystems Sync**. Subtitle: *"Files to Drive & S3 Buckets"*. Identificadores: `com.osystems.sync` (bundle), `osystems-sync` (pasta, slug).

**Consequências**
- ✅ Identidade consistente em todos os artefatos (UI, tray, title bar, installer, logs).
- ✅ Marca clara nos assets e documentação.
- ❌ Versionamento manual em `tauri.conf.json`, `Cargo.toml`, `package.json`, `CLAUDE.md`.

**Alternativas descartadas**
- FolderSync, Drive2S3, SyncAgent, etc.: nomes genéricos ou sem conexão com a identidade aprovada.

---

## ADR-003 — Duas telas em vez de cinco

**Contexto**
O mockup aprovado no design mostra 5 telas: Dashboard, Fila, Histórico, Logs, Configurações. Consolidar reduz complexidade de roteamento, minimiza IPC e simplifica UX do operador.

**Decisão**
**2 rotas apenas**: `/dashboard` (KPIs, tabela dupla, filtro/paginação, console de log embutido) e `/settings` (credenciais Drive/S3, QoS, Geral).

**Consequências**
- ✅ Menos rotas, menos queries SQL, menos eventos IPC.
- ✅ Operador não navega entre abas; tudo visível com filtros.
- ✅ Console de log integrado ao dashboard evita aba separada.
- ❌ Tabela pode ficar longa com 10 000+ jobs; mitigado com paginação (50/página) e índices SQLite.

**Alternativas descartadas**
- 5 telas separadas: overhead de roteamento; operador perde contexto ao navegar.

---

## ADR-004 — QoS (throttle) no MVP; modo noturno é Should-have

**Contexto**
Preservar o link corporativo compartilhado é requisito crítico do admin (RF-050). Modo noturno (suspender limite após 23:00) é conveniência, não necessidade.

**Decisão**
**MVP**: sliders por destino (0,5–10 MB/s, ilimitado), aplicados em tempo real ao salvar config. **Should** (fase 5): modo noturno com janela configurável.

**Consequências**
- ✅ Admin pode proteger a rede imediatamente.
- ✅ Throttle é genérico (token bucket); modo noturno é só lógica de relógio.
- ❌ Sem modo noturno, uploads em horário comercial em limites ajustados manualmente.

**Alternativas descartadas**
- Sem throttle no MVP: risco de saturar a VPN; feedback crítico do admin.

---

## ADR-005 — SHA-256 para integridade

**Contexto**
Arquivos podem ser corrompidos em trânsito. Drive armazena `sha256Checksum` nativamente; S3 permite metadados customizados.

**Decisão**
**SHA-256** para hash local e remoto. Cálculo completo (não incremental) durante leitura antes do upload.

**Consequências**
- ✅ Idempotência confiável: comparar SHA-256 antes de enviar.
- ✅ Auditoria: `x-amz-meta-sha256` no S3, campo `sha256Checksum` no Drive.
- ✅ Mesmo custo que a leitura normal (arquivo lido de qualquer forma).
- ❌ BLAKE3 no mockup era decorativo; foi descartado em favor de SHA-256 mais universalmente suportado.

**Alternativas descartadas**
- BLAKE3: mais rápido, mas menos portável; overkill para a velocidade de rede.
- MD5: inseguro; Drive oferece nativamente, mas SHA-256 é standard.

---

## ADR-006 — Subpastas por data (YYYY/MM/DD_backup) é Should-have

**Contexto**
Organizar arquivos por data no Drive é conveniência para auditoria, mas não bloqueia uploads. Toggle está no mockup; lógica é 15 linhas de Rust.

**Decisão**
**Should-have (fase 5)**: toggle de subpastas por data (`YYYY/MM/DD_backup/`); cache de IDs por dia.

**Consequências**
- ✅ Sem toggle: arquivos vão todos para a pasta raiz.
- ✅ Implementação simples: HashMap de cache invalidado à meia-noite.
- ❌ Sem feature no MVP; admin deve organizar manualmente ou ativar após primeira implementação.

**Alternativas descartadas**
- No MVP: reduz escopo, prioridade no backlog.

---

## ADR-007 — Pausar arquivo individual (paused) é Should-have

**Contexto**
Ícone de pausa no mockup para pausar um upload específico. Exige estado novo no job e tratamento especial no worker.

**Decisão**
**Should-have (fase 5)**: status `paused` no job; libera slot do worker sem cancelar permanentemente.

**Consequências**
- ✅ UI mostra ícone consistente com Pausar Watcher.
- ✅ Implementação: worker ignora status `paused`; `resume_job` volta a `pending`.
- ❌ MVP não oferece a feature; operador deve reenviar após resolver o erro.

**Alternativas descartadas**
- No MVP: simplifica worker loop; status bastam `pending`, `uploading`, `done`, `failed`.

---

## ADR-008 — "Pausar" afeta apenas o watcher

**Contexto**
Botão "Pausar" na UI pode significar: congelar tudo (watcher + workers) ou só novos arquivos.

**Decisão**
**Pausa do watcher** (`WatcherHandle::pause()`) descarta eventos novos de `notify`; uploads em voo **continuam até a conclusão**. Pausar é um `Option<bool>` no handler do watcher, não afeta o pool de workers.

**Consequências**
- ✅ Admin pausa detecção, não perde trabalho em andamento.
- ✅ Watcher e worker têm handles independentes.
- ✅ Semanticamente claro: "Pausar Watcher" = "parar de assistir", não "parar tudo".
- ❌ Após pausar, uploads já iniciados continuam; pode parecer confuso sem explicação na UI.

**Alternativas descartadas**
- Pausar tudo (watcher + workers): admin perde uploads; difícil de retomar.

---

## ADR-009 — Inter + JetBrains Mono vendorizadas

**Contexto**
CSP `'self'` proíbe carregar fontes de Google Fonts ou CDN. Mockup usa essas duas fontes. App é offline-first.

**Decisão**
Vendorizar **Inter** (UI) e **JetBrains Mono** (telemetria/console) em `public/fonts/` como variáveis WOFF2.

**Consequências**
- ✅ CSP `'self'` atendido; sem dependência de CDN.
- ✅ Zero delay de carregamento de fonte.
- ✅ App funciona completamente offline.
- ❌ Tamanho do bundle +500 KB (aceitável para desktop).

**Alternativas descartadas**
- Google Fonts via CDN: quebra CSP; app falha se offline.
- System fonts: não batem com mockup aprovado.

---

## ADR-010 — Paleta canônica do mockup

**Contexto**
DESIGN.md tinha 2 paletas internas divergentes (UI vs tokens). Mockups Stitch foram aprovados com uma paleta específica.

**Decisão**
**Paleta canônica = PNGs aprovados do mockup**. Deduplicar cores com ΔE < 2 registrado em `design/README.md`.

**Consequências**
- ✅ UI visual bate exatamente com mockup.
- ✅ Tokens em `design/tokens.css` alimentam `@theme` do Tailwind v4.
- ❌ Possíveis tweaks posteriores na paleta precisam refletir nos mockups.

**Alternativas descartadas**
- Paleta derivada: divergia dos PNGs; mockup aprovado vence.

---

## ADR-011 — Telemetria da statusbar (online/offline Must; ping Should)

**Contexto**
Admin quer saber se Drive e S3 estão acessíveis. Latência é informação secundária.

**Decisão**
**MVP**: status `Online`, `Offline`, `Auth` (401/403) por destino + versão do core + build target na statusbar. **Should** (fase 5): latência em ms (exige health-check periódico com medição de round-trip).

**Consequências**
- ✅ MVP oferece visibilidade essencial sem overhead.
- ✅ Health-check genérico (HEAD + timeout); latência é overhead adicional.
- ❌ MVP não mostra latência; admin não vê degradação de performance.

**Alternativas descartadas**
- Ping sempre: overhead de task a cada 60 s; not worth sem necessidade explícita.

---

## ADR-012 — Escopo Google: drive (não drive.file)

**Contexto**
Service Account precisa escrever em pasta compartilhada que **não criou**. Escopo `drive.file` (recomendado para drive.file) restringe acesso apenas a arquivos criados pela própria aplicação.

**Decisão**
Escopo OAuth: `https://www.googleapis.com/auth/drive`. Mitigação: SA só tem acesso ao que for explicitamente compartilhado com seu e-mail.

**Consequências**
- ✅ SA consegue escrever em pasta compartilhada pré-existente.
- ✅ Não cria pasta raiz nem arquivos não pedidos.
- ❌ Escopo amplo (drive vs drive.file); acesso é restringido pelo sharing da pasta, não pela API.

**Alternativas descartadas**
- `drive.file`: impossível usar pasta pré-existente; admin teria que criar sempre.

---

## ADR-013 — tauri-plugin-opener em vez de plugin-shell

**Contexto**
Necessário: abrir Explorer com `/select`, abrir pasta de logs, abrir links remotos (Drive webViewLink, console S3). Plugin-shell é genérico; plugin-opener é especializado e oferece allowlist.

**Decisão**
Usar **tauri-plugin-opener** com restrições: `opener:allow-open-path` para caminhos locais e URLs `https://drive.google.com/*`, `https://*.console.aws.amazon.com/*`.

**Consequências**
- ✅ Allowlist impede abrir URLs arbitrárias.
- ✅ Sem `invoke_command` genérico que poderia executar binários.
- ✅ Cobertura total: Explorer, pasta de logs, links remotos.
- ❌ Menos flexível que plugin-shell; mudanças futuras precisam atualizar allowlist.

**Alternativas descartadas**
- plugin-shell: permite executar scripts; não oferece allowlist, maior surface de ataque.

---

## ADR-014 — lucide-react em vez de Material Symbols

**Contexto**
Material Symbols do mockup requer carregamento de CDN (Google Fonts Icons). CSP `'self'` bloqueia; app offline não funciona.

**Decisão**
Substituir Material Symbols por **lucide-react** (SVG inline, tree-shaken). Mapeamento 1:1 dos ícones do mockup documentado em `design/DESIGN.md`.

**Consequências**
- ✅ SVG inline, sem CDN, sem bloqueio de CSP.
- ✅ Ícones tree-shaken; apenas os usados no build final.
- ✅ Tamanho do bundle: lucide ~30 KB vs Material Symbols ~200 KB.
- ❌ Mockup visual em Material Symbols; desenvolvedor precisa consultar mapeamento.

**Alternativas descartadas**
- Google Material Symbols via CDN: quebra CSP; infeasible offline.
- Desenhar ícones à mão: overhead de design; lucide cobre 99% dos casos.

---

## ADR-015 — Mensagens de log traduzidas na fonte Rust

**Contexto**
O console de eventos (RF-066) mostra as linhas de `tracing` cruas: `LogLine` carrega só `ts`/`level`/`target`/`job_id`/`destination`/`message`, sem código ou `kind` enumerado. As 95 mensagens estavam em inglês enquanto todo o resto do produto já é pt-BR (notificações nativas, `TestResult`, erros via `tError`), quebrando RNF-017 justamente na superfície que o usuário abre quando algo dá errado.

**Decisão**
Traduzir os literais dos macros `tracing::{trace,debug,info,warn,error}!` diretamente no Rust. Campos estruturados (`%error`, `path`, `job_id`), nomes de função citados nas mensagens, comentários e código continuam em inglês.

**Consequências**
- ✅ Console e `app.log` em pt-BR sem camada de tradução nova, sem risco de chave faltando em runtime.
- ✅ Zero custo por linha: nada de lookup a cada evento de log.
- ✅ Glossário fixo (watcher, upload, job, fila, varredura, modo noturno, destino, credencial) mantém um termo por conceito.
- ❌ Exceção deliberada ao ADR-011 (repositório em inglês) — limitada a mensagens voltadas ao usuário final; `app.log` é artefato de suporte que ele lê e envia.
- ❌ Um segundo idioma exigiria refazer o trabalho como `kind` enumerado + catálogo no frontend.

**Alternativas descartadas**
- Mapa string→chave no frontend: frágil (quebra ao editar qualquer literal) e as mensagens com campos estruturados exigiriam regex.
- Campo `kind: Option<String>` em `LogLine` + `logs.*` no `pt-BR.json`: robusto e o caminho certo se um dia houver 2 idiomas, mas 95 call sites a anotar, regeneração ts-rs e um catálogo a manter — custo desproporcional para um app single-locale.

---

## ADR-016 — JobRow de 40px e RowActions sem popover

**Contexto**
`design/DESIGN.md` §8 especificava a linha da `JobTable` em 56px e o `RowActions` como ghost-icons "em popover", com um `MoreVertical` abrindo o menu. Na prática: (a) a densidade custava caro — 6 linhas ocupavam a tela inteira num monitor de 768px de altura; (b) o menu escondia metade das ações atrás de dois cliques e de um alvo de 28px sem rótulo; (c) o `colgroup` tinha sido alargado às pressas para resolver uma sobreposição da coluna Ações em 1366×768.

**Decisão**
- Linha de **40px**: nome em `body-xs` (11px, degrau novo) + origem em `label-sm` com `line-height` 13px, células com `py-0`. As duas linhas de texto continuam visíveis.
- `RowActions` renderiza **todas as ações aplicáveis inline**, como botões quadrados de 24px com ícone de 14px, cada um embrulhado em `Tooltip`. O `MoreVertical`, o menu e a navegação por setas foram removidos.
- Só as ações aplicáveis ao estado do job aparecem (nada de placeholder desabilitado). Pior caso real: 6 botões (um lado enviando, outro falhado) = 178px.
- `colgroup` misto: Tamanho (90px), Status (130px) e Ações (180px) em px fixo porque a largura é determinada pelo conteúdo; as outras três em percentual. Tabela com `min-width: 880px` — abaixo disso o wrapper rola na horizontal em vez de espremer o cluster. Na janela padrão (1280px, `tauri.conf.json`) a caixa de conteúdo é 976px e não há barra.
- `Tooltip` novo em `components/ui`, por portal, 250ms no hover e imediato no foco.

**Consequências**
- ✅ Toda ação a um clique, com dica no hover **e** no foco por teclado — o menu antigo era o único caminho para Copiar caminho, Abrir remoto e Detalhes do erro.
- ✅ ~40% mais linhas visíveis sem rolar; "Sincronizado" deixou de ser cortado (os 13% antigos já cortavam abaixo de ~1100px).
- ✅ Alvo de 24px atende o mínimo do WCAG 2.2 SC 2.5.8.
- ❌ Janelas entre 1024 e ~1184px ganham rolagem horizontal dentro da tabela. Percentual puro não acomoda 400px de colunas determinadas por conteúdo nessa largura; a página em si nunca rola de lado.
- ❌ DESIGN.md §8 ("JobTable", "RowActions", "LogConsole", "Button") e a regra de sombra foram reescritos para bater com o que foi entregue.

**Alternativas descartadas**
- Manter o menu e só reduzir a altura: não resolve o custo de descoberta das ações escondidas, que era a queixa original.
- Botões de 20px para caber 6 numa coluna percentual em 1024px: fica abaixo do alvo mínimo de 24px do WCAG 2.2.
- Renderizar as 8 ações sempre, desabilitando as inaplicáveis: posição estável entre linhas, mas 8 alvos por linha e ruído visual permanente num app cuja fila costuma ter linhas em estados diferentes.

---

## ADR-017 — Sem teto de tamanho próprio do app; parte do multipart cresce com o arquivo

**Data:** 2026-09-05
**Status:** Aceita

**Contexto**
O RNF-004 fixava "arquivos até 5 GB" e `validate()` recusava `watch.max_size_mb` acima de 5000. Eram dois números distintos, e nenhum deles era o limite real:

| Camada | Teto |
|---|---|
| `watch.max_size_mb` (validação) | 5 GB — artificial |
| S3 multipart | 160 GiB — `PART_SIZE` de 16 MiB × 10 000 partes (`s3.rs`, VULN-005) |
| Drive resumable | 5 TB (limite do próprio Google) |

O campo era 32× mais apertado que a implementação já aguentava, e o próprio S3 aguenta 5 TiB por objeto — 160 GiB era consequência de uma parte fixa, não uma restrição do serviço. O uso pretendido é justamente arquivo grande, então o teto artificial só produzia descarte silencioso na entrada da fila.

**Decisão**
- `watch.max_size_mb` vira **filtro opcional**: `0` (o novo default) = sem teto. Validado contra `config::MAX_FILE_SIZE_MB` (5 TiB), o menor dos limites dos dois destinos.
- Novo `watch.min_size_mb`, mesmo formato (`0` = desligado), aplicado em `queue::passes_filters` antes do teto. Piso acima do teto é erro de validação — dropava tudo em silêncio.
- `s3::part_size_for(total)`: 16 MiB até `PART_SIZE * MAX_PARTS` (160 GiB), acima disso a menor parte alinhada a MiB que cabe o arquivo em 10 000 partes. Determinístico só em `total`, que é o que mantém o resume válido — a sessão persistida recalcula o mesmo layout na execução seguinte.
- O guard de `upload_multipart` passa de "mais de 10 000 partes" para "acima de `MAX_OBJECT_SIZE` (5 TiB)", já que a contagem de partes deixou de ser o que estoura primeiro.
- `s3::part_concurrency(part_size)`: `PART_CONCURRENCY` (4) encolhido para caber em `PART_MEMORY_BUDGET` (256 MiB), nunca abaixo de 1. `read_part` bufferiza a parte inteira; sem isso uma parte de 512 MiB × 4 workers seria 2 GiB de RSS e quebraria o RNF-001. Em 16 MiB o resultado é 4 — comportamento atual inalterado.

**Consequências**
- ✅ Teto efetivo passa de 160 GiB para 5 TiB no S3; no Drive continua o limite do Google.
- ✅ Filtro de tamanho vira ferramenta do usuário (banda `[min, max]`), não regra embutida.
- ✅ Pico de memória do multipart continua limitado, agora explicitamente.
- ❌ **`config.json` já existente mantém `max_size_mb: 5000`.** `#[serde(default)]` só preenche campo ausente; não há migração de versão. Quem já usava o app continua capado em 5 GB até zerar o campo em Ajustes.
- ❌ Arquivo > 160 GiB usa parte > 16 MiB, então uma parte que falha custa mais retransmissão. Aceito: a alternativa era recusar o arquivo.
- ❌ `PART_SIZE * 10_000 + 1` deixou de ser rejeitado; o teste da VULN-005 foi reescrito para o limite de objeto.

**Alternativas descartadas**
- Só soltar a validação para 160 GiB, mantendo `PART_SIZE` fixo: menos código, mas trava num número que é artefato da constante, não do S3.
- `max_size_mb` default no valor máximo (5 242 880) em vez de `0`: mesmo efeito prático, mas o campo passa a exibir um número de 7 dígitos que ninguém escolheu, e perde a simetria com o `0` do piso.
- Migrar `5000 → 0` automaticamente ao carregar: mexe em configuração do usuário sem ele pedir, e 5 GB é um valor que alguém pode ter escolhido de propósito.

---

## ADR-018 — Filtros do watcher passam a valer: rescan reconcilia nos dois sentidos

**Data:** 2026-09-05
**Status:** Aceita

**Contexto**
Usuário definiu tamanho mínimo e lista exclusiva de extensões, salvou, clicou em "Atualizar Lista" — e a tela não mudou. Três defeitos independentes, todos do mesmo feitio: o que está na tela de configuração não corresponde ao que o app executa.

1. **`watch_changed` era allowlist manual** (`commands/config.rs`). Comparava `path`, `recursive`, `extensions`, `stabilize_seconds`. `max_size_mb` nunca esteve na lista; `min_size_mb` (ADR-017) também não foi adicionado. Como `run_intake_loop` recebe `WatchConfig` **por valor** no spawn, mudar tamanho gravava `config.json` e `state.config` mas o loop vivo seguia com os valores antigos até reiniciar o app. Havia um teste, `watch_changed_ignores_fields_the_watcher_does_not_consume`, que **fixava o defeito** — o nome afirmava que o watcher não consome `max_size_mb`, mas `passes_filters` consome.
2. **`stabilize_seconds` não tinha consumidor.** `SyncRuntime.stabilize` era `StabilizeConfig::default()` fixo. O campo era validado (não era — ver abaixo), persistido, exibido na UI e ainda disparava reinício de watcher, sem mudar nada. Também não tinha validação alguma no core: a UI oferecia 1..60 e ligava um slot de erro que nunca podia acender, enquanto um `config.json` editado à mão com `0` desligava a estabilização.
3. **Job enfileirado nunca era reavaliado.** `passes_filters` só rodava na entrada. `rescan` lia config fresca mas só **acrescentava**. `list_jobs` não filtra por tamanho nem extensão. `worker.rs` não reconfere. Um arquivo que entrou sob filtros frouxos continuava listado **e continuava subindo**.

Dois defeitos latentes só apareceram ao desenhar a correção, e teriam tornado a solução cosmética:

4. **`claim_next` ignorava `archived_at`.** Arquivar um job `pending` não impediria o worker de pegá-lo: o arquivo sumiria da tabela e iria para o bucket assim mesmo — pior que o defeito original. `retry_all_failed` tinha o mesmo furo: "Reenviar Falhas" ressuscitaria arquivados.
5. **`status_counts` ignorava `archived_at`.** A tabela encolheria depois da varredura enquanto os KPIs e o badge "N arquivos" ficavam parados. Como a queixa literal foi "não deu pra perceber", corrigir só o item 3 resolveria metade.

**Decisão**
- `watch_changed` vira `old != new`. A allowlist é deletada, não consertada.
- `StabilizeConfig::from_watch` mapeia `stabilize_seconds` → `stable_reads` (interval de 1s), derivado a cada geração de watcher. O invariante de `runtime.rs` ("`Wakers` e `StabilizeConfig` compartilhados pela vida do app") é **estreitado para só `Wakers`** — a justificativa (worker parado em `Wakers::waiter`) nunca se aplicou a um struct `Copy` em que ninguém estaciona.
- `validate` passa a exigir `stabilize_seconds` em 1..=60.
- `rescan()` ganha segunda passada, `reconcile`: arquiva jobs `pending`/`paused`/`failed` de arquivos fora de escopo, sumidos do disco ou reprovados nos filtros; **e restaura** os arquivados que voltaram a passar. Roda nos cinco gatilhos.
- `claim_next`, `retry_all_failed` e `status_counts` passam a respeitar `archived_at IS NULL`.
- `rescan` (command) devolve `RescanReport` em vez de `u32`.

**Consequências**
- ✅ Filtro salvo vale imediatamente para arquivo novo, e em um clique para a fila existente.
- ✅ Reversível: `archived_at`, nunca `DELETE`. O app segue sem nenhum caminho destrutivo no banco.
- ✅ Caminho de volta existe. Afrouxar o filtro recupera; sem a metade `restored` seria porta de mão única, porque o atalho `size`+`mtime` devolve `Unchanged` para arquivo que não mudou.
- ✅ Transferência em voo nunca é interrompida: `uploading` fora do `IN`, e SELECT+UPDATE na mesma transação, no mesmo mutex do `claim_next`.
- ❌ **"Limpar Concluídos" agora baixa os KPIs.** Mudança visível no RF-036. Considero correção: arquivado é fora da visão, e contar o que não se vê era o que escondia a varredura.
- ❌ Arquivo com um lado `done` e outro varrido **some da visão padrão**, apesar de ter um upload concluído no histórico. Continua visível com `include_archived`.
- ❌ Toda gravação de Ajustes reinicia o watcher, com uma janela de sub-milissegundo em que eventos do `notify` caem. Já era assim para `path`/`recursive`/`extensions`.
- ❌ Uma pasta de rede que monte **vazia** arquivaria tudo. Não há perda: a metade `restored` traz tudo de volta no primeiro rescan depois que a rede volta.

**Alternativas descartadas**
- **Destructuring exaustivo em vez de `!=`**: quebraria a compilação ao adicionar campo (bom), mas não pega mudança semântica, e depois destas correções não sobra campo que a função pudesse corretamente ignorar. `!=` pega os dois casos.
- **`status = 'cancelled'` além de arquivar**: com a guarda em `claim_next` é redundante, polui o KPI de cancelados e confunde intenção do usuário ("eu cancelei") com reconciliação de filtro.
- **Filtrar só a visualização em `list_jobs`**: deixaria o upload rodando em background, com o filtro puramente cosmético.
- **Varrer só o que o walk encontrou nesta rodada**: perde exatamente o caso do `recursive: true → false`, em que o arquivo nunca mais é visitado e ficaria preso na fila para sempre.
- **Existência em disco como teste de escopo**: foi o primeiro desenho e um teste o derrubou — o arquivo aninhado continua existindo depois de `recursive: false`. Escopo é propriedade do caminho, não do disco.
- **Varrer só no rescan manual**: recria o padrão "apodrece calado" que esta ADR está removendo do `watch_changed`.
