---
type: osforge-prd
project: "oSystems Sync"
status: ready
created: "2026-09-04"
version: 2.0
sources:
  - SPEC.md (v1, comportamento)
  - assets/osystems_sync_documenta_o_funcional_e_recursos.md (v2.0, produto)
  - assets/executive_precision/DESIGN.md + mockups Stitch (visual)
sections_completed: [problema, personas, decisoes, rf, rnf, metricas, escopo, riscos]
---

# PRD — oSystems Sync Files to Drive & S3 Buckets

> **Ordem das fontes de verdade:** `PRD.md` (o quê) → `SPEC.md` (como) → `design/` (aparência).
> Conflito de comportamento → `SPEC.md`; conflito visual → `design/`; conflito de escopo → este documento.
> Identificadores (`RF-xxx`, nomes de módulos, commands IPC) ficam em inglês; prosa em pt-BR.

---

## 1. Problema e Contexto

### Problema
Equipes de operação geram artefatos locais continuamente (dumps de banco, builds, mídia, logs) numa
máquina Windows e precisam que **cada arquivo novo chegue, sem intervenção humana, a dois destinos
independentes**: uma pasta no Google Drive e um bucket S3. Soluções atuais (Drive Desktop, AWS CLI
em agendador, scripts) falham em pelo menos um ponto: não replicam para dois provedores em paralelo,
não sobrevivem a suspensão da máquina, não têm fila persistente com retry, ou consomem toda a banda
corporativa.

### Contexto
- Máquina Windows 11 com usuário logado, ligada 10+ dias seguidos.
- Volume esperado: dezenas a centenas de arquivos por dia, de KB a 5 GB.
- Rede corporativa compartilhada: upload não pode saturar o link.
- Não há equipe de infra dedicada: a configuração precisa caber numa tela.

### Por que agora
Já existe especificação técnica (`SPEC.md` v1) e design aprovado (mockups Stitch). Este PRD
reconcilia as duas versões e destrava a implementação.

### Proposta
Agente desktop nativo (Tauri 2 + núcleo Rust + React/TS) que vive na bandeja do Windows, observa
uma pasta, e para cada arquivo novo cria dois jobs independentes (Drive e S3) numa fila SQLite,
com retry exponencial, idempotência por hash, limite de banda por provedor e UI de cockpit para
acompanhar e intervir.

---

## 2. Personas

| Persona | Contexto | Dores | O que precisa ver |
|---|---|---|---|
| **Operador de backup** (primário) | Roda a máquina que gera os arquivos; abre o app 1–2× por dia | Não sabe se o arquivo de ontem chegou; refazer upload manual; app "morre" depois da suspensão | KPIs de hoje, falhas em vermelho com motivo, botão "Reenviar" |
| **Administrador de TI** (secundário) | Configura credenciais e política de banda uma vez; volta só quando há erro de permissão | Credenciais em `.env` vazando; upload derrubando VPN; erro 403 sem contexto | Tela de configuração única, teste de conexão com feedback, slider de banda, logs exportáveis |
| **Auditor / gestor** (terciário) | Quer prova de que a replicação é confiável | Sem histórico consultável | Histórico filtrável por status/destino, checksum registrado |

---

## 3. Decisões de reconciliação (SPEC v1 × assets v2)

Fechadas em 2026-09-04. Detalhe técnico em `SPEC.md §12`; ADRs em `.specs/project/DECISIONS.md`.

| # | Tema | Decisão | Motivo |
|---|---|---|---|
| C1 | Autenticação Google Drive | **Service Account (chave JSON)** | Zero interação humana em 10+ dias; refresh token OAuth pode ser revogado. API Key simples não escreve no Drive. AWS segue Access Key + Secret. |
| C2 | Nome do produto | **oSystems Sync** ("oSystems Sync Files to Drive & S3 Buckets" no título) | Identidade dos assets v2; o nome de trabalho da v1 foi descartado. |
| C3 | Telas | **2 telas**: `Dashboard & Fila` e `Configurações & QoS` | Mockups aprovados. Histórico = filtro de status + paginação na mesma tabela; logs = console embutido + "abrir pasta de logs". |
| C4 | QoS de banda | **Sliders por provedor no MVP**; agenda noturna é Should-have | Preservar link corporativo é dor real do admin; agenda adiciona lógica de relógio. |
| C5 | Hash de integridade | **SHA-256** | Comportamento do SPEC vence; BLAKE3 no mockup é decorativo. Drive compara `sha256Checksum`; S3 guarda `x-amz-meta-sha256`. |
| C6 | Subpastas por data no Drive | Should-have (`YYYY/MM/DD_backup/`) | Toggle presente no mockup; não bloqueia MVP. |
| C7 | Pausar arquivo individual | Should-have (estado `paused`) | Ícone presente no mockup; exige estado extra no job. |
| C8 | Semântica de "Pausar" | **Pausa só o watcher**; uploads em voo continuam | Doc v2 é explícito. |
| C9 | Tipografia | **Inter** (UI) + **JetBrains Mono** (telemetria), fontes vendorizadas | Mockups + doc v2 concordam; app é offline (CSP `'self'`). |
| C10 | Tokens de cor | **Paleta do mockup** (`tailwind-config`) é canônica | Bate com os PNGs aprovados; DESIGN.md tinha 2 paletas internas divergentes. |
| C11 | Telemetria da statusbar | Online/offline por destino + versão do core + build = Must; latência em ms = Should | Ping exige health-check periódico extra. |
| — | Botão "olho" no secret | Revela **só valor ainda não salvo**; após salvar, só máscara | Preserva RNF-003. |
| — | Idioma dos artefatos | pt-BR; identificadores em inglês | Consistência com SPEC/CLAUDE existentes e UI. |

---

## 4. Requisitos Funcionais

Prioridade: **M** = Must-have (MVP) · **S** = Should-have · **N** = Nice-to-have.
Cada requisito tem critério de aceite (AC) verificável.

### 4.1 Watcher (detecção local)

| ID | Requisito | AC | Pri |
|---|---|---|---|
| RF-001 | Usuário escolhe **uma** pasta monitorada via seletor nativo; caminho exibido na sidebar e na title bar. | Após escolher, `config.json` contém `watch.path`; sidebar mostra o caminho; trocar pasta reinicia o watcher sem reiniciar o app. | M |
| RF-002 | Arquivo novo ou modificado é detectado e só entra na fila após **estabilizar** (tamanho inalterado por N leituras de 1 s **e** abertura exclusiva bem-sucedida). | Copiar arquivo de 1 GB → linha aparece em `Na Fila` só após a cópia terminar; nunca durante. | M |
| RF-003 | Filtros opcionais: extensões permitidas, incluir subpastas (recursivo), tamanho máximo. | Arquivo fora do filtro não gera job e gera 1 linha de log `debug`. | M |
| RF-004 | **Rescan (reconciliação)**: ao iniciar, ao retornar de suspensão e por comando manual, varrer a pasta e **(a)** enfileirar o que não está em `files` ou cujo hash mudou; **(b)** arquivar os jobs `pending`/`paused`/`failed` de arquivos que não passam mais nos filtros, saíram do escopo (`recursive`) ou sumiram do disco; **(c)** recuperar os arquivados que voltaram a passar. Nunca toca `uploading` nem `done`; nunca apaga — só `archived_at`. | Criar 3 arquivos com app fechado → abrir app → 3 linhas na fila; reiniciar de novo → nenhuma duplicata. Apertar o filtro de tamanho → Atualizar Lista → os que não passam saem da lista e dos KPIs; afrouxar de volta → voltam. | M |
| RF-005 | **Pausar / Retomar watcher** (botão topbar + tray). Pausado: novos eventos ignorados; uploads em andamento continuam. | Pausar durante upload de 500 MB → barra continua; copiar arquivo novo → não aparece; Retomar → rescan captura o arquivo. | M |
| RF-006 | Ignorar temporários (`~$*`, `*.tmp`, `*.crdownload`, `*.part`, ocultos) e diretórios. | Nenhum desses padrões gera job. | M |

### 4.2 Destino: Google Drive

| ID | Requisito | AC | Pri |
|---|---|---|---|
| RF-010 | Admin carrega o **JSON da Service Account** (seletor de arquivo). Conteúdo vai para o keyring; UI mostra só nome do arquivo, tamanho, `client_email` e `project_id`. | Após salvar, o JSON não existe em `%APPDATA%`, `config.json` nem SQLite; `get_credential_status` retorna `{ present: true, email }`. | M |
| RF-011 | Campo **Folder ID** de destino com suporte a Shared Drives (`supportsAllDrives=true`); botão copiar e abrir no navegador. | Folder em Shared Drive aceita upload; Folder não compartilhado com a SA → erro `Auth` legível ("compartilhe a pasta com `<email>`"). | M |
| RF-012 | Upload: `< 8 MB` multipart simples; `≥ 8 MB` **resumable** em chunks de 16 MB com retomada por `Content-Range`. | Arquivo de 100 MB chega íntegro; derrubar rede no chunk 3 → retomar sem reiniciar do zero. | M |
| RF-013 | **Idempotência**: antes de enviar, `files.list` por nome + parent com `fields=…sha256Checksum`; se o `sha256Checksum` bate com o hash local, marca `done` sem reenviar (campo ausente → compara `size` e reenvia em caso de dúvida). | Reenviar arquivo já presente → job `done` em < 2 s sem tráfego de upload. | M |
| RF-014 | **Testar conexão**: `GET files/{folder_id}` + tentativa de criar/apagar arquivo vazio; feedback inline com latência. | Botão mostra `✓ 24 ms • Autenticado` ou mensagem de erro específica (403/404/JSON inválido). | M |
| RF-015 | Toggle **subpastas por data** `YYYY/MM/DD_backup/` criadas sob o Folder ID; cache de IDs por dia. | Ligado: arquivo de 04/09 vai para `2026/09/04_backup/`; pasta criada uma única vez por dia. | S |
| RF-016 | Token de acesso da SA renovado automaticamente antes de expirar (JWT 1 h, skew 5 min). | App rodando 3 h envia arquivos nas horas 1, 2 e 3 sem erro 401. | M |

### 4.3 Destino: AWS S3

| ID | Requisito | AC | Pri |
|---|---|---|---|
| RF-020 | Admin informa Access Key ID, Secret Access Key (campo password com "olho" só pré-salvamento), Region (dropdown), Bucket, Prefix, Storage Class (`STANDARD`, `INTELLIGENT_TIERING`, `GLACIER_IR`). | Secret vai para keyring; após salvar, UI mostra `AKIA****XYZ`. | M |
| RF-021 | Upload: `< 8 MB` `put_object`; `≥ 8 MB` multipart (partes de 16 MB, 4 concorrentes), metadata `x-amz-meta-sha256`. | Arquivo de 2 GB chega com ETag multipart e metadata correta. | M |
| RF-022 | **Idempotência**: `head_object` antes; se `x-amz-meta-sha256` igual, `done` sem reenviar. | Idem RF-013. | M |
| RF-023 | **Testar bucket**: `head_bucket` + `put_object` de 0 bytes em `{prefix}.osystems-sync-probe` + delete **best-effort** (sem `DeleteObject` o teste passa e avisa que o probe permanece). | `✓ 41 ms • Bucket válido (Put/List OK)` ou erro IAM específico; política sem `DeleteObject` → `✓ … sem permissão DeleteObject`. | M |
| RF-024 | Multipart abortado em caso de `failed`/cancelamento (`AbortMultipartUpload`) para não gerar custo órfão. | Cancelar upload de 1 GB → `ListMultipartUploads` no bucket retorna vazio. | M |

### 4.4 Fila, retry e estado

| ID | Requisito | AC | Pri |
|---|---|---|---|
| RF-030 | Cada arquivo gera **dois jobs independentes** (`gdrive`, `s3`); falha em um não afeta o outro. | Bucket inválido + Drive OK → linha mostra `S3: Falha 403` e `Drive: 100 %`. | M |
| RF-031 | **Retry com backoff** exponencial `5 s × 2^n` ± 20 % jitter, teto 10 min, máx. 5 tentativas → `failed`. | Desligar rede → tentativas em ~5 s, 10 s, 20 s…; religar → `done`. 5 falhas → `failed` com `last_error`. | M |
| RF-032 | **Classificação de erro**: `Transient` (timeout, 5xx, 429, rede) → retry; `Auth` (401/403) → pausa destino + evento `auth-required`; `Permanent` (demais 4xx) → `failed` imediato. | 403 no S3 → card S3 fica "Requer atenção", jobs S3 param, Drive segue. | M |
| RF-033 | **Reenviar** item `failed` (reset `attempts`, `status=pending`). | Clique → linha volta a `Na Fila` e é processada. | M |
| RF-034 | **Reenviar todos com falha** (ação em lote). | Retorna contagem; todos `failed` viram `pending`. | M |
| RF-035 | **Cancelar** job `pending`/`uploading` (CancellationToken; aborta multipart) → status terminal `cancelled`. | Linha some da fila ativa (visível só no filtro Todos como `Cancelado`); nenhum tráfego residual após 2 s; `ListMultipartUploads` vazio. | M |
| RF-036 | **Limpar concluídos**: remove da visão (não do banco) jobs `done` — marca `archived_at`. | Botão → tabela mostra só ativos; filtro "Concluídos" ainda lista com paginação. | M |
| RF-037 | **Pausar / retomar arquivo individual** (estado `paused`; libera slot do worker). | Pausar → barra congela, próximo job assume o slot; Retomar → continua (resumable) ou reinicia (S3 sem retomada de parte). | S |
| RF-038 | Fila persistida em **SQLite**; reinício/crash não perde nem duplica jobs. | Matar processo com 5 jobs `uploading` → reabrir → 5 jobs voltam a `pending` e concluem. | M |
| RF-039 | Nunca enviar o mesmo arquivo duas vezes ao mesmo destino (chave `path + sha256`). | Sobrescrever arquivo com conteúdo idêntico → nenhum job novo; conteúdo diferente → job novo. | M |
| RF-040 | Progresso por job (`bytes enviados / total`) emitido a cada ≤ 500 ms; taxa individual (MB/s) calculada em janela de 5 s. | Barras das colunas Drive/S3 avançam de forma independente. | M |

### 4.5 QoS de banda

| ID | Requisito | AC | Pri |
|---|---|---|---|
| RF-050 | **Limite de upload por provedor**: slider `0.5–10 MB/s` + `Ilimitado`, badge numérica ao lado. | Limite 2,5 MB/s no S3 → throughput medido do S3 fica em 2,3–2,7 MB/s com 2 workers ativos. | M |
| RF-051 | Limite aplicado em **tempo de execução** ao salvar, sem reiniciar uploads. | Mudar slider durante upload → taxa muda em ≤ 2 s. | M |
| RF-052 | Limite é **por destino, compartilhado entre os workers** daquele destino (token bucket único). | 2 workers S3 somados nunca ultrapassam o teto. | M |
| RF-053 | **Modo Noturno**: janela configurável (padrão 23:00–06:00) em que os limites são suspensos. | Às 23:01 local, throughput sobe ao máximo; às 06:01 volta ao limite. | S |

### 4.6 UI — Dashboard & Fila (`/dashboard`)

| ID | Requisito | AC | Pri |
|---|---|---|---|
| RF-060 | **KPIs** no topo: Total detectados (+ bytes), Concluídos (+ % da carga), Em transferência, Na fila, Falhas. | Valores batem com `SELECT count(*) GROUP BY status` em ≤ 1 s após mudança. | M |
| RF-061 | **Ações rápidas** no topo: Atualizar lista (rescan), Pausar/Retomar watcher, Limpar concluídos. Atualizar lista informa os três números da reconciliação (enfileirados, removidos pelo filtro, recuperados); os dois últimos só aparecem quando não-zero. | Cada botão dispara o command correspondente e reflete estado (label muda para "Retomar"). | M |
| RF-062 | **Tabela de sincronização dupla**: colunas Arquivo & Origem (ícone por tipo, nome, caminho relativo mono, "há N min"), Tamanho, Google Drive (barra + % + MB/s), AWS S3 (idem), Status (badge), Ações. | Layout idêntico ao mockup em ≥ 1280 px; linha 56 px; texto nunca sobrepõe. | M |
| RF-063 | **Status agregado por linha**: `Enviando` (pulso), `Sincronizado`, `Na Fila`, `Falha <código>`; tooltip com detalhe por destino. | Regra: qualquer `failed` → Falha; qualquer `uploading` → Enviando; qualquer `paused` → Pausado; ambos `done` → Sincronizado; qualquer `cancelled` e nenhum ativo → Cancelado; senão Na Fila. | M |
| RF-064 | **Filtro de status** (Todos / Ativos / Concluídos / Falhas) + paginação (50 por página) — substitui a tela History. | 10 000 jobs `done` → página carrega em ≤ 200 ms; filtro persiste na sessão. | M |
| RF-065 | **Ações por linha**: abrir no Explorer, reenviar, cancelar, menu `⋯` (copiar caminho, abrir remoto — link S3 console / Drive `webViewLink`, detalhes do erro). Pausar individual = RF-037. | Cada ícone chama seu command; ícones desabilitados quando não aplicáveis ao status. | M |
| RF-066 | **Console de eventos** colapsável na base: últimas 500 linhas (ring buffer), tag de origem (`WATCHER`, `HASH`, `GDRIVE`, `S3`, `CORE`), filtro por nível, botão "abrir pasta de logs". | Linha aparece em ≤ 300 ms do evento; console nunca cresce além de 500 linhas em memória. | M |
| RF-067 | **Sidebar**: navegação (2 itens com badge de contagem), card da pasta monitorada (caminho, badge `Watcher: Ativo/Pausado`, "Alterar pasta"), widget de **throughput** (agregado / teto + barra segmentada Drive×S3). | Throughput = soma das taxas dos jobs `uploading`, atualizado a cada 1 s. | M |
| RF-068 | **Statusbar** (28 px): `Rust Core vX.Y.Z (Active)`, `GDrive: Online/Offline/Auth`, `AWS S3: Online/Offline/Auth (region)`, build target à direita. | Estado muda em ≤ 60 s após queda de rede (health-check periódico). | M |
| RF-069 | Latência (`Ping: 24 ms`) por endpoint na statusbar. | Health-check mede round-trip de `HEAD` leve a cada 60 s. | S |
| RF-070 | Estados vazios e de erro desenhados: sem pasta configurada (CTA "Escolher pasta"), sem credenciais (CTA "Configurar"), destino em `auth-required` (banner com ação "Ir para Configurações"). | Cada estado tem tela dedicada no `design/DESIGN.md` e é alcançável. | M |

### 4.7 UI — Configurações & QoS (`/settings`)

| ID | Requisito | AC | Pri |
|---|---|---|---|
| RF-080 | **Módulo Google Drive**: card com status (`Conectado / Online`), upload do JSON da SA (nome, tamanho, `client_email`), Folder ID (copiar / abrir), toggles "subpastas por data" (RF-015) e "checksum pré-upload" (sempre ligado no MVP, toggle desabilitado com tooltip), rodapé com resultado do teste + botão Testar. | Igual ao mockup; teste reflete RF-014. | M |
| RF-081 | **Módulo AWS S3**: Access Key, Region, Secret (olho pré-save), Bucket, Prefix, Storage Class, rodapé com teste + botão Testar Bucket. | Igual ao mockup; teste reflete RF-023. | M |
| RF-082 | **Módulo QoS**: dois sliders (RF-050) + toggle Modo Noturno (RF-053, desabilitado no MVP com "em breve"). | Slider com ticks 0,5 / 2,5 / 5 / 10 e posição "Ilimitado" no extremo direito. | M |
| RF-083 | **Salvar Preferências** (botão + `Ctrl+S`): valida, persiste `config.json` (sem segredos) + keyring, aplica em runtime (watcher, throttle, uploaders). Rodapé mostra "Última alteração salva às HH:MM:SS". | Trocar bucket e salvar → próximo job usa o novo bucket sem reiniciar. Validação inline por campo (bucket vazio, region inválida, JSON malformado). | M |
| RF-084 | **Cancelar / Restaurar padrões**: descarta alterações não salvas; restaurar redefine QoS (ilimitado), filtros e workers para fábrica; **nunca apaga credenciais**. | Diálogo de confirmação; credenciais permanecem `present: true`. | M |
| RF-085 | Segredos nunca chegam ao renderer após salvos; campos mostram máscara e botão "Substituir". | Inspecionar payload IPC de `get_credential_status` → só máscara/e-mail. | M |
| RF-086 | Seção **Geral** (dentro de Configurações, abaixo do QoS): iniciar com Windows, manter acordado, workers por destino (1–4), tentativas máx. (1–10), filtros do watcher (RF-003). | Todos persistem e aplicam sem reinício, exceto autostart (aplica no próximo boot). | M |

### 4.8 Sistema, tray e ciclo de vida

| ID | Requisito | AC | Pri |
|---|---|---|---|
| RF-090 | **Ícone na bandeja** com 3 estados (ok / trabalhando / erro) e menu: Abrir, Pausar/Retomar watcher, Rescan, Sair. | Ícone muda em ≤ 2 s após primeiro job `uploading` ou `failed`. | M |
| RF-091 | Fechar a janela **minimiza para a bandeja**; app continua processando. | Fechar → processo vivo, uploads continuam; clique no ícone reabre. | M |
| RF-092 | **Iniciar com o Windows** (toggle). | Reiniciar Windows → app na bandeja sem janela. | M |
| RF-093 | **Manter acordado** (`ES_SYSTEM_REQUIRED`, sem forçar tela). | Com toggle ligado, máquina não suspende com uploads pendentes. | M |
| RF-094 | Detectar **retorno de suspensão** e disparar rescan (RF-004). | Suspender 10 min com 3 arquivos criados antes → ao acordar, 3 jobs em ≤ 60 s. | M |
| RF-095 | **Sair** = shutdown gracioso: aguarda uploads em voo até 30 s, depois cancela; jobs voltam a `pending`. | Sair com upload de 5 GB → processo encerra em ≤ 30 s; ao reabrir, job retoma. | M |
| RF-096 | **Notificação nativa** ao job virar `failed` e ao destino entrar em `auth-required` (agrupada: máx. 1 por minuto). | Toast do Windows com nome do arquivo e destino. | S |
| RF-097 | **Title bar customizada** (36 px, `decorations: false`): logo + nome, chip da pasta monitorada, badge `Daemon: Active`, controles nativos (min/max/close). | Arrastar pela barra move a janela; botões respondem; snap layouts do Win11 funcionam no botão maximizar. | M |
| RF-098 | Abrir no Explorer (arquivo selecionado) e abrir pasta de logs. | `revealItemInDir(path)` abre o Explorer com o arquivo destacado. | M |
| RF-099 | Logs rotacionados em `%APPDATA%/osystems-sync/logs/` (10 MB × 5), formato JSON por linha. | Após 60 MB de log, existem no máximo 5 arquivos + o ativo. | M |

---

## 5. Requisitos Não Funcionais

| ID | Requisito | Verificação |
|---|---|---|
| RNF-001 | **Uptime** 10+ dias sem crescimento perceptível de memória: RSS varia ≤ 10 % em teste de 72 h com 500 arquivos. | Script de longa duração (SPEC §11) + amostragem de RSS a cada 5 min. |
| RNF-002 | **Credenciais nunca em texto plano em disco**: keyring (Windows Credential Manager), serviço `osystems-sync`. | Auditoria de `%APPDATA%`, `config.json`, `state.db`, logs → zero ocorrências de secret/JSON da SA. |
| RNF-003 | **Renderer nunca recebe segredo completo**; só máscara / e-mail / `present`. | Teste Vitest no wrapper IPC + revisão de `capabilities/default.json`. |
| RNF-004 | Arquivos **sem teto próprio do app**; o limite é o do destino (S3 5 TiB por objeto, Drive 5 TB). Multipart (S3) e resumable (Drive) acima de 8 MB; a parte do multipart cresce com o arquivo para caber nas 10 000 partes do S3. `watch.min_size_mb`/`watch.max_size_mb` são filtros opcionais do usuário (`0` = desligado). | Upload acima de 160 GiB (a antiga fronteira de 10 000 partes) em ambos os destinos com rede instável simulada. |
| RNF-005 | **Logs rotacionados** (10 MB × 5). | Ver RF-099. |
| RNF-006 | **Estado persistido** em SQLite (WAL); reinício não perde fila. | Ver RF-038. |
| RNF-007 | **Idle**: CPU < 1 % e I/O de disco ≈ 0 quando não há arquivos novos (watcher por eventos, não polling). | Monitor de recursos 10 min ocioso. |
| RNF-008 | **UI responsiva**: `list_jobs` com 10 000 linhas em ≤ 200 ms (paginado); eventos de progresso throttled a 500 ms; sem jank perceptível. | Vitest + medição manual com React Profiler. |
| RNF-009 | **Isolamento arquitetural**: crate `core` não importa `tauri`; `src-tauri` só adapta (commands, events, tray). | `cargo tree -p core | grep tauri` vazio; CI falha se violar. |
| RNF-010 | **Tipos IPC gerados** (`ts-rs`) — zero tipos duplicados à mão no renderer. | `cargo test` regenera `src/types/generated.ts`; diff limpo em CI. |
| RNF-011 | **Segurança Tauri**: capabilities mínimas (só commands listados, `dialog:allow-open`, `opener` restrito a caminhos locais e `https://drive.google.com/*`, `https://*.console.aws.amazon.com/*`); CSP em `tauri.conf.json → app.security.csp` = `default-src 'self'; connect-src ipc: http://ipc.localhost; font-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'`. | Revisão de `tauri.conf.json` + `capabilities/default.json` no code review de cada fase. |
| RNF-012 | **Permissões mínimas** nos provedores: IAM `s3:PutObject, GetObject, ListBucket, AbortMultipartUpload, ListMultipartUploadParts, DeleteObject` (probe) no bucket; Drive escopo `https://www.googleapis.com/auth/drive` (SA precisa escrever em pasta que não criou). | Documentado na tela de ajuda do teste de conexão. |
| RNF-013 | **Acessibilidade**: contraste texto/superfície ≥ 4.5:1 (exceto texto terciário decorativo ≥ 3:1), foco visível, navegação por teclado nas duas telas, `aria-label` em ícones de ação. | Tabela de contraste em `design/DESIGN.md`; axe-core no Vitest de componentes. |
| RNF-014 | **Offline-first UI**: nenhuma dependência de CDN (fontes, ícones, Tailwind) — tudo bundlado. | Build sem rede renderiza idêntico. |
| RNF-015 | **Observabilidade**: logs estruturados `{ ts, level, target, job_id?, destination?, message }`; nunca logar caminho completo de segredo, token ou conteúdo do JSON da SA. | Grep nos logs após suíte de integração. |
| RNF-016 | **Instalador** NSIS/MSI assinado (fase 6); auto-update opcional. | Instala/desinstala limpo; autostart removido na desinstalação. |
| RNF-017 | **i18n**: UI em pt-BR no MVP; strings centralizadas para permitir en-US depois. | Nenhuma string de UI hardcoded fora de `src/i18n/`. |

---

## 6. Métricas de sucesso

### MVP (fases 0–5, todos os `M`)
| Métrica | Alvo | Como medir |
|---|---|---|
| Jobs perdidos em teste de 72 h / 500 arquivos | **0** | Contagem local = contagem S3 = contagem Drive |
| Duplicatas remotas | **0** | `ListObjects` / `files.list` sem nomes repetidos com mesmo hash |
| Tempo detecção → `pending` (arquivo estável) | ≤ 5 s | Log `detected_at` vs `created_at` do job |
| Recuperação após queda de rede de 5 min | 100 % dos jobs `done` sem intervenção | Teste com rede desligada |
| Recuperação após suspensão | Pendentes processados em ≤ 60 s após acordar | RF-094 |
| Precisão do limite de banda | Throughput medido dentro de ±10 % do teto | RF-050 |

### Produto completo (Should-haves da fase 5 + fase 6)
| Métrica | Alvo |
|---|---|
| RSS após 72 h | drift ≤ 10 % |
| Intervenções manuais por semana (operador) | ≤ 1 |
| Tempo para o admin configurar do zero até primeiro upload | ≤ 10 min |
| Falhas `auth-required` sem notificação | 0 |

---

## 7. Escopo

### MVP (dentro) — todos os `M` (fases 0–5; RF-093/094/095 e estados `auth-required` fecham na fase 5 junto com `power`)
- Watcher + estabilização + filtros + rescan + pausa (4.1)
- Drive via Service Account com resumable e idempotência (4.2, exceto RF-015)
- S3 com multipart, idempotência, abort (4.3)
- Fila SQLite, retry, classificação de erro, reenviar / cancelar / limpar (4.4, exceto RF-037)
- QoS por provedor com hot-reload (4.5, exceto RF-053)
- 2 telas conforme mockups + estados vazios (4.6, 4.7, exceto RF-069)
- Tray, autostart, keep-awake, resume, shutdown gracioso, title bar (4.8, exceto RF-096)

### Futuro (Should / Nice, já desenhado)
- RF-015 subpastas por data · RF-037 pausar arquivo · RF-053 modo noturno · RF-069 ping · RF-096 notificações
- Auto-update · en-US · exportar histórico CSV

### Fora de escopo (v1 e v2)
- Sincronização bidirecional ou download
- Exclusão / renomeação remota espelhada
- Múltiplas pastas monitoradas ou múltiplos buckets/folders
- Serviço Windows (sem usuário logado) — arquitetura permite migrar depois
- macOS / Linux
- OAuth com conta Google pessoal (documentado como fallback em `SPEC.md §12`, não implementado)
- Criptografia client-side dos arquivos

---

## 8. Riscos e mitigações

| Risco | Severidade | Mitigação |
|---|---|---|
| Tenant Google sem Workspace ou pasta não compartilhável com a SA | **Alta** | Teste de conexão devolve o `client_email` e instrução exata; risco documentado na ajuda; fallback OAuth registrado em `SPEC.md §12` para uma fase futura. |
| Cota do Drive: 750 GB/dia por usuário (SA conta como usuário) e limite de arquivo 5 TB | Média | Erro 403 `storageQuotaExceeded` classificado como `Transient` com backoff de 1 h; alerta na UI. |
| Upload de 5 GB interrompido no meio (rede/suspensão) | Média | Resumable (Drive) e multipart com `UploadId` persistido em `jobs.remote_state` (S3) para retomar partes já enviadas. |
| Suspensão do Windows mata sockets sem evento claro | Média | Detector de gap de relógio (60 s × 2) + `SetThreadExecutionState` opcional; rescan ao acordar. |
| Credential Manager indisponível (perfil roaming, política de grupo) | Baixa | Erro fatal legível na primeira tela; app não cai para arquivo plano. |
| CSP bloqueia fontes/ícones do mockup (Google Fonts, Material Symbols) | Baixa | Fontes vendorizadas; ícones via `lucide-react` (SVG inline) mapeados 1:1 dos Material Symbols usados. |
| Paleta do mockup com hexes quase duplicados gera tokens inconsistentes | Baixa | Dedupe com ΔE < 2 registrado em `design/README.md`. |
| Escopo inflado pelos extras do mockup | Média | Todos os extras são `S`/`N` e ficam na fase 5; gate de readiness verifica que nenhum `M` depende deles. |
| Antivírus corporativo trava arquivo durante estabilização | Média | Estabilização exige abertura exclusiva; timeout 30 min → tenta mesmo assim + log `warn`. |
| Custo S3 de multipart órfão | Baixa | RF-024 + regra de ciclo de vida sugerida no bucket (`AbortIncompleteMultipartUpload` 1 dia) na ajuda. |

---

## 9. Glossário do projeto

| Termo | Significado | Evitar |
|---|---|---|
| **Watcher** | Componente que observa a pasta e emite caminhos de arquivos estáveis. | monitor, listener |
| **Job** | Uma unidade de upload = (arquivo, destino). Um arquivo gera 2 jobs. | task, transfer, item |
| **Destino** | `gdrive` ou `s3`. | provedor, conector, target |
| **Estabilizar** | Esperar o arquivo parar de crescer e liberar lock antes de enfileirar. | settle, debounce (é só a 1ª etapa) |
| **Rescan** | Varredura completa da pasta comparando com `files`. | refresh, sync |
| **Throttle** | Token bucket por destino que impõe o teto de banda. | rate limiter, QoS (nome da tela) |
| **auth-required** | Estado de destino após 401/403: jobs pausados até nova credencial. | erro de login |

---

## Checkpoint

- **[A] Aprovar** → `status: ready`; PRD vira fonte para `SPEC.md` v2 e `PLAN.md`.
- **[E] Editar** seção específica.
- **[V] Validar** → `adversarial-review` + `readiness-gate` (executado automaticamente em T-08).
