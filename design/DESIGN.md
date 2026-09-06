# DESIGN.md — oSystems Sync

> Sistema de design da aplicação desktop **oSystems Sync** (Tauri 2 + React/TS + Tailwind v4).
> Tema único, escuro ("Executive Precision"). Fonte de verdade dos tokens: `design/tokens.css`
> (proveniência em `design/README.md`). Este documento cobre paleta, tipografia, layout,
> elevação, movimento, ícones, componentes, telas e acessibilidade das duas rotas aprovadas
> (`/dashboard` e `/settings`, decisão **C3** do `PRD.md`). Todo nome de `--token`, componente,
> tipo IPC e id `RF-xxx` citado aqui é literal — verificável em `design/tokens.css`, `SPEC.md` §7/§8
> e `PRD.md` §4.

---

## 1. Identidade & princípios

O produto atende operadores e administradores de TI monitorando uma sincronização contínua de
arquivos locais para Google Drive e AWS S3, rodando em segundo plano na bandeja do Windows. A
referência estética é o instrumento de aviônica e o terminal financeiro dedicado: extrema
contenção funcional, alta densidade de informação, baixa fadiga óptica em observação contínua.
Escuridão aqui não é ausência de luz — é uma pilha deliberada de degraus tonais (obsidiana → ardósia
fria) que cria hierarquia visual sem depender de brilho, gradiente ou saturação.

Princípios que orientam toda decisão de UI neste documento:

1. **Peso visual por micro-hierarquia, nunca por decoração.** Contraste de texto, alinhamento
   rigoroso e separação estrutural finíssima (hairline) fazem o trabalho que gradientes e sombras
   fariam em outros sistemas.
2. **Sinal antes de estilo.** Cor é reservada a papéis semânticos (`--color-primary` = Google Drive,
   `--color-secondary` = AWS S3, `--color-tertiary` = sucesso/nominal, `--color-error` = falha). Nunca
   decorativa.
3. **Densidade sem ruído.** Grade de 4px, linhas de tabela compactas, tipografia mono para todo dado
   numérico/técnico — o operador lê 50 linhas de fila sem cansar.
4. **Nada de gloss.** Sem gradiente, sem glow neon, sem sombra difusa em cards, sem pill radius, sem
   branco puro. Ver lista completa em §11 (Anti-padrões).
5. **pt-BR em toda a superfície do produto** (rótulo, mensagem, tooltip) — decisão registrada no
   `PRD.md` §3 — mantendo nomes técnicos de token, componente e tipo IPC em inglês, como neste
   próprio documento.

A paleta de cor definitiva vem dos dois mockups Stitch aprovados (`assets/osystems_sync_dashboard_fila_de_arquivos/`
e `assets/osystems_sync_configura_es_qos_de_banda/`) — decisão **C10**: a paleta do YAML de
`assets/executive_precision/DESIGN.md` foi descartada porque o próprio arquivo trazia duas paletas
internas divergentes; apenas sua prosa estrutural (esta seção, e as convenções de nomenclatura de
`unit-*`, densidade de tabela, radius) foi aproveitada. Onde os dois mockups usavam hex quase iguais
para o mesmo papel, `design/README.md` documenta a fusão por ΔE CIE76 que produziu o valor canônico
de cada token — não repetida aqui.

---

## 2. Paleta

Todos os tokens abaixo vivem em `:root` em `design/tokens.css`. A tabela cita o hex real apenas
porque esta é a seção de mapeamento token → hex; em qualquer outro lugar deste documento (e no
código) o valor é sempre referenciado pelo nome do token.

### Superfícies (stacking tonal, ver §5)

| Token | Hex | Uso | Exemplo |
|---|---|---|---|
| `--color-surface-0` | `#0c0e12` | Nível 0 — fundo raiz da janela, calha do app | `body`, `main` do Dashboard |
| `--color-surface-1` | `#101319` | Nível 1 — painéis persistentes | fundo do `Sidebar`, `StatusBar` |
| `--color-surface-2` | `#13171f` | Nível 2 — cards, módulos, inputs, header de tabela | `KpiCard`, `Field`, `JobTable` header |
| `--color-surface-3` | `#171c26` | Nível 3 — item ativo/selecionado dentro de um nível 2 | `NavItem` ativo, chip da pasta |
| `--color-surface-hover` | `#1f242e` | Hover de linha/botão sobre superfície 1–3 | hover de `JobRow`, hover de botão titlebar |

### Bordas

| Token | Hex | Uso | Exemplo |
|---|---|---|---|
| `--color-border-hairline` | `#1f242e` | Separador fino entre linhas/seções — baixo contraste, propositalmente quase invisível | divisor de `JobRow`, borda de card |
| `--color-border-strong` | `#475161` | Contorno de elemento interativo em foco/hover intencional, nunca decorativo | hover de `FolderCard`, borda de `Banner` |

### Texto (ramp por papel, do mais para o menos enfático)

| Token | Hex | Uso | Exemplo |
|---|---|---|---|
| `--color-on-primary` | `#f0f4f9` | Texto sobre preenchimento `--color-primary-container` (nunca sobre `--color-surface-*`) | rótulo do botão primário `Limpar Concluídos` |
| `--color-text-primary` | `#e2e4ea` | Corpo de texto padrão, títulos de linha | nome do arquivo em `JobRow` |
| `--color-text-emphasis` | `#cad2df` | Texto de destaque sem ser título (valor numérico grande, nome de pasta) | valor "BackupLocal" no `FolderCard` |
| `--color-text-label` | `#bcc5d3` | Rótulo uppercase mono de badge/etiqueta | texto de `StatusBadge` |
| `--color-text-status` | `#b4bece` | Texto de status na `StatusBar` | "AWS S3: Online (us-east-1)" |
| `--color-text-secondary` | `#828ea2` | Texto de suporte, ícones inativos, botão ghost | rótulo "Atualizar" do `Button` ghost |
| `--color-text-tertiary` | `#788596` | Texto terciário — ícone de linha, metadado curto | ícone de tipo de arquivo em `JobRow` |
| `--color-text-quaternary` | `#5f6c80` | Texto de menor ênfase — placeholder, timestamp de rodapé | placeholder de `Field`, tick label do `Slider` |

### Marca / semântica

| Token | Hex | Uso | Exemplo |
|---|---|---|---|
| `--color-primary` | `#6082a4` | Texto/ícone/dot do destino **Google Drive**; foco geral do app | dot + label "Google Drive" na `JobTable` |
| `--color-primary-container` | `#27384a` | Preenchimento de botão primário (ação de confirmação forte) | fundo do `Button` primary |
| `--color-primary-strong` | `#4f7cac` | Preenchimento de barra/thumb (nunca texto) — tom mais saturado que `--color-primary` | fill do `DualProgress` lado Drive, thumb do `Slider` Drive |
| `--color-secondary` | `#a38258` | Texto/ícone/dot do destino **AWS S3** | dot + label "AWS S3" |
| `--color-secondary-container` | `#3d3222` | Fundo de estado "aviso" ativo sem ser destrutivo | reservado para variante de `Banner` warning |
| `--color-secondary-strong` | `#8c6e48` | Preenchimento de barra/thumb do lado S3 | fill do `DualProgress` lado S3, thumb do `Slider` S3 |
| `--color-tertiary` | `#4e8774` | Texto/dot de sucesso/nominal | dot "Watcher Ativo", ícone `CheckCircle2` de "Concluído" |
| `--color-tertiary-container` | `#1e372e` | Fundo de chip de sucesso persistente | fundo "Conectado / Online" no módulo Drive/S3 |
| `--color-error` | `#b95c65` | Texto/ícone de falha | badge "Falha 403", KPI "Falhas Detectadas" |
| `--color-error-container` | `#381c20` | Fundo de estado de erro persistente | reservado para variante de `Banner` error |
| `--color-danger-hover` | `#8f3941` | Hover do botão de fechar da titlebar (única ação verdadeiramente destrutiva do shell) | hover do botão `Close` |

### Fundos de status (tintas computadas, não hex fixos)

`--color-status-*-bg` são **sempre** `color-mix()` sobre a cor de marca correspondente — nunca um
hex literal — para que a tinta acompanhe a marca automaticamente se um tom mudar:

```css
--color-status-info-bg: color-mix(in srgb, var(--color-primary) 10%, transparent);
--color-status-success-bg: color-mix(in srgb, var(--color-tertiary) 10%, transparent);
--color-status-warning-bg: color-mix(in srgb, var(--color-secondary) 10%, transparent);
--color-status-error-bg: color-mix(in srgb, var(--color-error) 12%, transparent);
```

### Matriz de status (fila de sincronização, RF-060–RF-070)

| Estado | Dot | Fundo do badge | Texto do badge |
|---|---|---|---|
| **Enviando** | `--color-primary` (pulsando, ver §6) | `--color-status-info-bg` | `--color-primary` |
| **Sincronizado** | `--color-tertiary` | `--color-status-success-bg` | `--color-tertiary` |
| **Na Fila** | `--color-text-quaternary` | `--color-surface-2` + `--color-border-hairline` | `--color-text-secondary` |
| **Falha** | `--color-error` | `--color-status-error-bg` | `--color-error` |
| **Pausado** | `--color-secondary` | `--color-status-warning-bg` | `--color-secondary` |
| **auth-required** (evento IPC) | `--color-error` | `--color-error-container` | `--color-on-primary` |

`auth-required` usa o par de maior contraste (`--color-on-primary` sobre `--color-error-container`,
ver §10) porque é bloqueante — o destino parou de enviar e exige ação da credencial — e aparece tanto
no `Banner` do shell quanto na `StatusBar`.

---

## 3. Tipografia

Duas famílias vendorizadas em `public/fonts/` (RNF-014 — offline, sem CDN):

- **`--font-sans`** → Inter Variable (`/fonts/InterVariable.woff2`, peso 100–900) — interface geral.
- **`--font-mono`** → JetBrains Mono Variable (`/fonts/JetBrainsMono[wght].woff2`, peso 100–800) —
  todo dado técnico: bytes, taxas, timestamps, hashes, rótulos uppercase.

Cada degrau da escala é um grupo de 5 tokens (`-size`, `-leading`, `-weight`, `-tracking`, `-family`):

| Degrau | Tokens | Tamanho | Entrelinha | Peso | Tracking | Uso |
|---|---|---|---|---|---|---|
| `display` | `--text-display-*` | 2rem (32px) | 2.5rem | 600 | −0.025em | reservado, não usado nas 2 telas atuais |
| `headline-lg` | `--text-headline-lg-*` | 1.5rem (24px) | 2rem | 600 | −0.02em | título de página (`h1` "Configurações & QoS de Banda") |
| `headline-md` | `--text-headline-md-*` | 1.25rem (20px) | 1.75rem | 500 | −0.015em | valor numérico grande de `KpiCard` |
| `headline-sm` | `--text-headline-sm-*` | 1.125rem (18px) | 1.5rem | 500 | −0.01em | título de módulo (`Card` Google Drive/AWS S3) |
| `title-md` | `--text-title-md-*` | 0.9375rem (15px) | 1.375rem | 500 | −0.005em | nome de item de navegação, nome de arquivo em destaque |
| `body-lg` | `--text-body-lg-*` | 0.9375rem (15px) | 1.5rem | 400 | 0em | parágrafo de apoio (descrição de módulo) |
| `body-md` | `--text-body-md-*` | 0.8125rem (13px) | 1.25rem | 400 | 0em | texto corrido padrão de componente |
| `body-sm` | `--text-body-sm-*` | 0.75rem (12px) | 1.125rem | 400 | 0.01em | rótulo de campo, texto secundário |
| `mono-data` | `--text-mono-data-*` | 0.8125rem (13px) | 1.25rem | 400 | −0.01em | valor numérico/técnico corrido (caminho de arquivo, IP, versão) |
| `label-md` | `--text-label-md-*` | 0.6875rem (11px) | 1rem | 500 | 0.06em | rótulo uppercase mono padrão (header de tabela, badge) |
| `label-sm` | `--text-label-sm-*` | 0.625rem (10px) | 0.875rem | 500 | 0.08em | rótulo uppercase mono minúsculo (tick de slider, tag de log) |

Duas regras não negociáveis (herdadas de `assets/executive_precision/DESIGN.md` §Typography e
confirmadas nos dois mockups):

- **Todo rótulo uppercase usa `label-md` ou `label-sm`** — nunca `text-transform: uppercase` sobre
  um degrau sans. Tracking de 0.06–0.08em existe justamente para compensar a perda de legibilidade
  do caixa-alta em tamanho minúsculo; um uppercase sem esse tracking fica ilegível.
- **Todo dado numérico contínuo usa `mono-data`, `label-md` ou `label-sm` com `font-variant-numeric:
  tabular-nums`** — taxa de transferência, contador de KPI, timestamp de log. Números proporcionais
  "dançam" horizontalmente a cada atualização de 500ms/1s (RF-040, evento `upload-progress`,
  `throughput`); tabular-nums é obrigatório em qualquer elemento que re-renderiza um número em loop.

Nunca usar branco puro (`#ffffff`) como cor de texto — o degrau mais claro disponível é
`--color-on-primary` (`#f0f4f9`), reservado a texto sobre `--color-primary-container`.

---

## 4. Espaçamento, grid & layout

### Grade de 4px

```
--space-2xs: 0.125rem (2px)   --space-md: 0.75rem (12px)
--space-xs:  0.25rem  (4px)   --space-lg: 1rem     (16px)
--space-sm:  0.5rem   (8px)   --space-xl: 1.5rem   (24px)
                              --space-2xl: 2rem     (32px)
                              --space-3xl: 3rem     (48px)
```

Cadência macro (gap entre blocos, padding de página) em múltiplos de 8px (`--space-sm`/`--space-lg`/
`--space-xl`); cadência micro (padding interno de botão/badge, gap entre ícone e texto) em múltiplos
de 4px (`--space-xs`/`--space-md`). Nunca um valor de espaçamento fora dessa ladder.

### Geometria do shell (fixa nas duas rotas)

| Região | Token | Valor |
|---|---|---|
| Title bar | `--layout-titlebar-h` | 36px |
| Sidebar | `--layout-sidebar-w` | 256px (16rem) |
| Status bar | `--layout-statusbar-h` | 28px |
| Inspector (reservado, não usado no MVP) | `--layout-inspector-w` | 320px (20rem) |
| Linha de tabela compacta | `--layout-row-compact` | 28px (1.75rem) |
| Linha de tabela regular | `--layout-row-regular` | 36px (2.25rem) |

Padding de conteúdo de página: `--space-xl` (24px) nas laterais, `--space-lg` (16px) vertical entre
blocos, seguindo o `px-8 py-5` observado nos dois mockups (32px lateral seria `--space-2xl`, mas os
mockups usam consistentemente 24–32px conforme a tela; adotar `--space-xl` como piso e permitir
`--space-2xl` em telas com `max-w-6xl mx-auto` centralizado, caso do `/settings`).

**Discrepância resolvida entre os dois mockups:** o mockup do Dashboard usa `h-9` (36px) para a
title bar e `top-9`/`bottom-7` para o encaixe de sidebar/conteúdo; o mockup de Configurações usa
`h-10` (40px) para a mesma barra. `design/tokens.css` fixa `--layout-titlebar-h` em 36px, com o
Dashboard como tela de referência (é a rota inicial e a mais densa) — a tela de Configurações deve
ser implementada com a barra de 36px, não 40px. Regra geral de precedência (também vale para
qualquer outro conflito pixel a pixel entre os mockups): **este documento vence sobre o HTML dos
mockups.**

### Breakpoints

| Faixa | Comportamento |
|---|---|
| **≥ 1440px** | Layout de referência dos mockups — sidebar 256px fixa, conteúdo fluido com `max-w-6xl` centralizado nas telas de formulário (`/settings`), `KpiCard` grid completo (5 colunas). |
| **1024–1439px** | Breakpoint padrão de operação — mesmo shell, grid de KPI e módulos QoS colapsam para menos colunas (`grid-cols-1 lg:grid-cols-2` conforme já modelado no mockup de Configurações); nenhuma coluna de tabela é ocultada. |
| **< 1024px** | **Não suportado.** `tauri.conf.json` define `minWidth: 1024, minHeight: 700` na janela principal — o app nunca é redimensionado abaixo disso, então não existe layout de contingência para telas menores. |

---

## 5. Elevação, bordas & formas

### Níveis de superfície (tonal stacking, sem sombra)

Profundidade é comunicada por **degrau de tom**, nunca por `box-shadow` difuso:

- **Nível 0** (`--color-surface-0`) — fundo raiz da janela.
- **Nível 1** (`--color-surface-1`) — painéis persistentes do shell (Sidebar, Status Bar).
- **Nível 2** (`--color-surface-2`) — conteúdo agrupado dentro do nível 0/1: `KpiCard`, `Card`
  de módulo, `Field`, header de `JobTable`.
- **Nível 3** (`--color-surface-3`) — o item ativo/selecionado *dentro* de um nível 2, ex.: chip da
  pasta monitorada, `NavItem` ativo.

Cada subida de nível é sempre acompanhada de uma borda `--color-border-hairline` de 1px — a borda é
o que separa o degrau de tom do fundo, não um `box-shadow`.

### Bordas: hairline vs. forte

- **`--color-border-hairline`** é o padrão para 95% dos casos: divisor de linha de tabela, contorno
  de card, contorno de input em repouso. É deliberadamente de baixo contraste (ver §10) — não deve
  chamar atenção, só definir o degrau.
- **`--color-border-strong`** é reservado a um contorno que precisa comunicar estado (hover
  intencional do `FolderCard`, borda do `Banner` de `auth-required`, contorno de foco *como
  fallback* quando `--ring-focus` não se aplica). Usar `--color-border-strong` em todo lugar
  esvaziaria seu sinal.

### Sombra — só para elementos flutuantes

`--shadow-popover` (`0 8px 24px rgba(0,0,0,0.65)`) é a **única** sombra do sistema, e só se aplica a
elementos que flutuam sobre o layout normal: `Tooltip`, dropdown do `select`
de QoS, `Toast`, diálogo de detalhes de erro. **Nunca** em `Card`, `KpiCard` ou qualquer elemento que já pertence a um nível de
superfície — esses comunicam profundidade só por tom + borda (ver §11, anti-padrão "sombra em
card").

### Formas (radius)

| Token | Valor | Uso |
|---|---|---|
| `--radius-sm` | 2px | badge minúsculo, tick de slider |
| `--radius` | 4px | controle padrão: `Button`, `Field`, `Toggle` track |
| `--radius-md` | 6px | `Card`, `KpiCard`, módulo de `/settings` |
| `--radius-lg` | 8px | container flutuante (`Toast`, menu de contexto) |
| `--radius-full` | 9999px | exclusivamente para elementos circulares: dot de status, thumb de `Slider`, avatar |

Nunca usar `--radius-full` em botão ou badge retangular (ver §11 — pill radius é anti-padrão).

---

## 6. Movimento

Dois tokens de duração e uma curva, deliberadamente poucos:

| Token | Valor | Uso |
|---|---|---|
| `--duration-fast` | 120ms | hover/active de botão, ícone, linha de tabela |
| `--duration-base` | 200ms | transição de progress bar, abertura/fechamento de `LogConsole`, troca de estado de badge |
| `--ease-standard` | `cubic-bezier(0.2, 0, 0, 1)` | curva única de todo o sistema — entrada rápida, chegada suave, sem overshoot |

Regras:

- **Barras de progresso (`DualProgress`) são sempre lineares.** `width` anima com
  `--duration-base` + `--ease-standard`; nunca `ease-in-out` com overshoot, nunca "spring" — o valor
  reportado por `upload-progress` é um número real de rede, uma barra "elástica" mentiria sobre o
  dado.
- **Pulso do dot "Enviando":** `opacity` de `1` para `0.4` e de volta, `1.6s`, `ease-in-out`, loop
  infinito. Não é um token de duração do sistema (é uma animação de estado contínuo, não uma
  transição de interação), mas usa a mesma família de curva suave — implementar como:
  ```css
  @keyframes status-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }
  .status-dot.is-sending { animation: status-pulse 1.6s ease-in-out infinite; }
  ```
- **`prefers-reduced-motion: reduce` desliga o pulso e reduz toda transição a `duration: 1ms`** —
  aplicado com escopo local (no componente, não em `*` global) para não quebrar `transition` que
  dependem de duração real para não gerar "salto" visual sem transição nenhuma:
  ```css
  @media (prefers-reduced-motion: reduce) {
    .status-dot.is-sending { animation: none; opacity: 1; }
  }
  ```
- Nenhuma transição do sistema usa física de mola (`spring`) — ver §11.

---

## 7. Ícones

Os dois mockups Stitch usam `material-symbols-outlined` via Google Fonts CDN. A aplicação real
bloqueia CDN de fonte/ícone por CSP (`default-src 'self'`, RNF-014), então todo ícone é substituído
por **`lucide-react`** (SVG inline, sem rede). A tabela abaixo é a extração literal de cada
`<span class="material-symbols-outlined">` encontrado nos dois arquivos `code.html` aprovados, com o
componente Lucide equivalente e o tamanho padronizado em 16/18/20px (os tamanhos brutos do mockup,
de 11px a 19px, foram bucketizados nesses três degraus — nunca usar um `size` arbitrário fora deles).

| Ícone Material (mockup) | Componente `lucide-react` | Tamanho | Onde aparece |
|---|---|---|---|
| `folder` | `Folder` | 16 | chip de pasta na title bar, ícone dentro do `FolderCard` |
| `folder_open` | `FolderOpen` | 18–20 | ícone principal do `FolderCard` |
| `folder_shared` | `FolderSymlink` | 16 | campo "ID da Pasta de Destino" |
| `folder_zip` | `FileArchive` | 16 | ícone de linha — arquivo `.zip` |
| `description` | `FileText` | 16 | ícone de header da coluna "Arquivo & Origem" |
| `inventory_2` | `Package` | 16 | KPI "Total Detectados" |
| `database` | `Database` | 16 | ícone de linha — arquivo de banco; botão "Testar Bucket" |
| `movie` | `Film` | 16 | ícone de linha — arquivo de vídeo |
| `terminal` | `Terminal` | 16 | ícone de linha — arquivo de script |
| `table_chart` | `Table` | 16 | ícone de linha — arquivo `.parquet`/planilha |
| `archive` | `Archive` | 16 | ícone de linha — arquivo `.tar` |
| `chevron_right` | `ChevronRight` | 16 | "Alterar" no `FolderCard`, breadcrumb |
| `expand_more` | `ChevronDown` | 16 | caret do `LogConsole`, seta de `select` |
| `sync` | `RefreshCw` | 16 | widget de throughput ("14 itens"), botão "Testar Conexão" |
| `sync_alt` | `ArrowLeftRight` | 16–18 | item de navegação "Arquivos & Fila" |
| `swap_horiz` | `ArrowLeftRight` | 16 | botão "Alterar pasta" do `FolderCard` (mesmo componente Lucide de `sync_alt`, papel visual diferente) |
| `refresh` | `RefreshCw` | 16 | botão "Atualizar lista" |
| `replay` | `RotateCcw` | 16 | ação de linha "Reenviar arquivo" |
| `pause` | `Pause` | 16 | ação de linha "Pausar transferência" |
| `pause_circle` | `PauseCircle` | 16 | botão "Pausar Watcher" |
| `cleaning_services` | `Eraser` | 16 | botão "Limpar Concluídos" |
| `delete_sweep` | `Trash2` | 16 | ação de limpar console de log |
| `vertical_align_top` | `ArrowUpToLine` | 16 | ação de linha "Priorizar na fila" |
| `more_vert` | `MoreVertical` | 16 | ação de linha "Mais opções" (`RowActions`) |
| `check` | `Check` | 16 | ícone inline de confirmação (estado salvo do botão "Salvar Preferências") |
| `check_circle` | `CheckCircle2` | 16 | KPI "Concluídos", ícone de teste "Autenticado" |
| `hourglass_empty` | `Hourglass` | 16 | KPI "Na Fila", `StatusBadge` "Na Fila" |
| `error` | `AlertCircle` | 16 | KPI "Falhas Detectadas", `StatusBadge` "Falha" |
| `warning` | `AlertTriangle` | 16 | aviso inline de erro no `DualProgress` (ex. "403 Denied") |
| `verified` | `BadgeCheck` | 16 | resultado de teste "Bucket Válido" |
| `tune` | `SlidersHorizontal` | 16–18 | item de navegação "Configurações" |
| `speed` | `Gauge` | 18–20 | header do módulo QoS |
| `add_to_drive` | `HardDrive` | 18–20 | header do módulo Google Drive |
| `cloud_sync` | `CloudCog` | 18–20 | header do módulo AWS S3 |
| `key` | `Key` | 16 | ícone do chip do JSON da Service Account |
| `content_copy` | `Copy` | 16 | botão "Copiar ID" |
| `open_in_new` | `ExternalLink` | 16 | botão "Abrir Google Drive" |
| `visibility` | `Eye` | 16 | olho do campo Secret Access Key (oculto) |
| `visibility_off` | `EyeOff` | 16 | olho do campo Secret Access Key (revelado) |
| `history` | `History` | 16 | "Última alteração salva às..." no rodapé de `/settings` |
| `save` | `Save` | 16 | botão "Salvar Preferências" |
| `remove` | `Minus` | 16 | botão de minimizar da title bar |
| `check_box_outline_blank` / `crop_square` | `Square` | 16 | botão de maximizar da title bar (os dois mockups usam ícones Material diferentes para o mesmo botão — ambos mapeiam para o mesmo componente Lucide) |
| `close` | `X` | 16 | botão de fechar da title bar |

38 ícones Material distintos mapeados para 32 componentes `lucide-react` distintos (algumas
duplicidades intencionais: `sync`/`refresh` → `RefreshCw`, `sync_alt`/`swap_horiz` →
`ArrowLeftRight`, `check_box_outline_blank`/`crop_square` → `Square`, porque os dois mockups usam
ícones Material diferentes para o mesmo papel visual). Todo ícone é importado individualmente
(`import { Folder } from 'lucide-react'`), nunca via barrel (`import * as Icons`), para não inflar o
bundle.

---

## 8. Componentes

Convenções gerais de todos os componentes interativos: foco visível via `--ring-focus` em
`:focus-visible` (nunca em `:focus` puro, para não mostrar anel em clique de mouse); estado
`disabled` reduz opacidade para 50% e remove `cursor: pointer`; todo ícone que é a única pista de uma
ação (`RowActions`, botões da title bar) leva `aria-label`.

### TitleBar
- **Anatomia:** logo + nome do app · separador vertical · chip da pasta monitorada (ícone `Folder`
  + caminho em `mono-data`) · badge "Daemon: Active" (dot `--color-tertiary` + `label-md`) · botões
  de janela (minimizar/maximizar/fechar).
- **Dimensões:** altura `--layout-titlebar-h` (36px); botão minimizar/maximizar 40×36px, botão
  fechar 44×36px (área maior — alvo de clique de saída, convenção Windows).
- **Tokens:** fundo `--color-surface-0`; borda inferior `--color-border-hairline`; texto do chip
  `--color-text-secondary` em `mono-data`.
- **Estados:** hover dos botões de janela → `--color-surface-hover`; hover do botão fechar →
  `--color-danger-hover` com texto `--color-on-primary` (feedback padrão de SO para "fechar" — usa o
  teto de claridade do sistema, nunca `#ffffff` literal, per §11).
- **A11y:** `data-tauri-drag-region` na área não-interativa; `aria-label="Minimize"` /
  `"Maximize"` / `"Close"` em cada botão.

### Sidebar
- **Anatomia:** logo + nome (topo) · navegação (`NavItem` × 2: "Arquivos & Fila", "Configurações",
  com badge de contagem) · `FolderCard` (pasta monitorada) · `ThroughputWidget` (rodapé).
- **Dimensões:** largura `--layout-sidebar-w` (256px), fixa, `position: fixed` entre a title bar e a
  status bar; padding interno `--space-md` (12px).
- **Tokens:** fundo `--color-surface-1`; borda direita `--color-border-hairline`.

**NavItem**
- Item ativo: fundo `--color-surface-3`, texto `--color-text-primary`, borda
  `--color-border-hairline`. Item inativo: texto `--color-text-secondary`, hover → fundo
  `--color-surface-2` + texto `--color-text-primary`.
- Badge de contagem (à direita): `label-sm` mono sobre `color-mix(in srgb, var(--color-primary) 15%,
  transparent)`, texto `--color-primary` — reflete `AppStatus.destinations` agregado (RF-060).

**FolderCard**
- Estrutura de duas camadas: uma aba superior (`Pasta`, `label-sm` uppercase) e o corpo (ícone
  `FolderOpen` em container 32×32px nível 0 + nome da pasta em `title-md` + dot "Watcher Ativo" +
  contador "N itens"). Clique abre `pick_folder` (troca de pasta monitorada).
- Estado do watcher: dot `--color-tertiary` pulsante (RF-060, `AppStatus.watcher_active`) quando
  ativo; dot `--color-text-quaternary` estático quando pausado (RF-012, "Pausar Watcher" — o
  watcher para de emitir novos jobs, mas uploads em andamento continuam, decisão **C8**).
- **A11y:** `role="button"`, `aria-label="Pasta monitorada: {nome}. Clique para alterar."`.

**ThroughputWidget**
- Barra dupla horizontal (nível 0, 4px altura) com dois segmentos preenchidos (`--color-primary-strong`
  para Drive, `--color-secondary-strong` para S3), somando a largura proporcional a `throughput.total_bps`
  vs. os limites de QoS. Rodapé com dois valores `mono-data`: "GDrive: N MB/s" / "S3: N MB/s",
  atualizados pelo evento `throughput` (a cada 1s).

### StatusBar
- **Anatomia:** versão do core Rust (`AppStatus.core_version`) · separador · status por destino
  (`GDrive: Online/Offline/Auth`, `AWS S3: Online/Offline/Auth (region)`) · build target.
- **Dimensões:** altura `--layout-statusbar-h` (28px), `position: fixed` no rodapé.
- **Tokens:** fundo `--color-surface-1`; borda superior `--color-border-hairline`; texto
  `--color-text-status` em `mono-data`; dot de status por destino usa a cor de marca do destino
  (`--color-primary`/`--color-secondary`) quando online, `--color-error` quando `auth-required`.
- **A11y:** `aria-live="polite"` no container do status por destino — mudança de Online → Auth
  precisa ser anunciada sem foco.

### KpiCard
- **Anatomia:** rótulo (`label-sm` uppercase) · valor grande (`headline-md`, `mono-data` quando é
  contagem/bytes) · ícone de contexto (canto superior direito, 16px) · badge inferior opcional
  (ex. "57% carga", "Requer atenção").
- **Dimensões:** parte de um grid responsivo (5 colunas em ≥1440px, ver §4); padding `--space-md`.
- **Tokens:** fundo `--color-surface-2`; borda `--color-border-hairline`, ou a cor de marca do KPI
  em 30–35% de opacidade quando o card representa um estado ativo (ex. borda `--color-error` no KPI
  de falhas) — o mesmo padrão usado nos 5 `KpiCard` do mockup do Dashboard.
- **A11y:** o valor numérico do card fica dentro de um `aria-live="polite"` — KPIs mudam a cada
  `job-updated`/`status-changed` e precisam ser lidos por leitor de tela sem o usuário precisar
  focar o card manualmente.

### JobTable
- **Header:** `label-md` uppercase sobre `--color-surface-1`, altura 28px. Três colunas são
  determinadas pelo conteúdo e vão em px fixo — Tamanho (90px), Status (130px, cabe "Sincronizado"),
  Ações (180px, cabe o cluster de 6 ícones do `RowActions`); as outras três dividem o resto em
  percentual — Arquivo & Origem (30% · 34% em ≥1440px), Google Drive e AWS S3 (12% cada · 15% em
  ≥1440px). A tabela carrega `min-width: 880px`; abaixo disso o wrapper rola na horizontal em vez
  de espremer as colunas (ver ADR-016).
- **JobRow:** altura de **40px**. O nome do arquivo trunca em uma linha `body-xs` (11px) e o caminho
  relativo aparece embaixo em `label-sm` com `line-height` apertado (13px) — 28px de conteúdo dentro
  dos 40px, sem padding vertical nas células (`py-0`, alinhamento por `align-middle`). Substituiu os
  56px originais por densidade; ver ADR-016.
  - Hover: fundo `--color-surface-hover` a 60% (transição `--duration-fast`).
  - Cada `JobView` (RF-060, `list_jobs`) vira uma linha, unindo os dois `jobs` internos (`gdrive` +
    `s3`) do mesmo arquivo — cada destino tem sua própria célula `DualProgress` e seu próprio
    `StatusBadge`, mas a linha é uma só.
- **DualProgress (célula):** barra 4px de altura, fundo `--color-surface-0`, fill `--color-primary-strong`
  (Drive) ou `--color-secondary-strong` (S3), largura proporcional a `sent/total` do evento
  `upload-progress`; texto `mono-data` abaixo com percentual + taxa (`rate_bps` formatado em MB/s,
  `label-sm`, tabular-nums).
- **StatusBadge:** dot 6px (`--radius-full`) + `label-sm` uppercase, cores por estado conforme a
  matriz de §2. Badge de `Falha` inclui um link "Detalhes" (`body-sm`, `--color-error`) que expande
  a mensagem de erro (`AppError.message`) inline.
- **RowActions:** botões ghost-icon quadrados de **24px** (`size="icon"`, ícone 14px), todos inline —
  **não existe popover nem menu `MoreVertical`** (ver ADR-016). Só as ações aplicáveis ao job são
  renderizadas, em ordem fixa: Abrir no Explorer (`FolderOpen`) e Copiar caminho (`Copy`) sempre ·
  Reenviar (`RotateCw`) · Pausar (`Pause`, RF-014) · Retomar (`Play`) · Cancelar (`X`) ·
  Abrir remoto (`ExternalLink`, um por destino concluído) · Detalhes do erro (`AlertCircle`).
  Cada botão é embrulhado em `Tooltip` — ícone sem rótulo visível precisa da dica no hover e no foco.
- **A11y:** `<table>` semântica com `scope="col"` nos headers; cada `StatusBadge` tem texto real (não
  só cor) — dot + label sempre juntos, nunca só a cor comunicando o estado.

### LogConsole
- **Anatomia:** cabeçalho colapsável (ícone `ChevronDown`/rotacionado, título "Console de Eventos",
  filtro por nível, botão "Limpar" `Trash2`, botão "Abrir pasta de logs" `FolderOpen`) · corpo com
  ring buffer de até 500 `LogLine` (`get_recent_logs`, evento `log-line`).
- **LogLine:** uma linha de **20px** por evento — timestamp `label-md` mono (`--color-text-quaternary`) + tag de
  nível colorida (`label-sm`, cores: `info` → `--color-text-secondary`, `warn` → `--color-secondary`,
  `error` → `--color-error`) + mensagem `label-md` mono (`--color-text-primary`). Campos opcionais `job_id`/
  `destination` do payload `LogLine` aparecem como um mono-data secundário truncável.
- **Tokens:** fundo `--color-surface-0` (mais escuro que o card ao redor, para reforçar leitura de
  "terminal dentro do painel"); borda `--color-border-hairline`.
- **A11y:** `role="log"` no container do corpo, `aria-live="polite"` — cada `LogLine` inserida é
  anunciada sem roubar foco; altura máxima com `overflow-y: auto` e `aria-relevant="additions"`.

### Field
- Container com label (`body-sm`, `--color-text-secondary`) acima do input.
- **Input de texto:** altura **30px**, fundo `--color-surface-0`, borda `--color-border-hairline`,
  texto `mono-data` (a maioria dos campos de `/settings` é técnica: Access Key, Bucket, Folder ID).
  Foco: borda `--color-border-strong` + `--ring-focus`.
- **Select:** mesma altura/tokens do input de texto, ícone `ChevronDown` (16px, `--color-text-quaternary`)
- **NumberField:** spinner nativo desligado (`appearance: none` em `app.css`); no lugar, `ChevronUp`/`ChevronDown` de 12px empilhados à direita, `text-text-quaternary` — o mesmo token do chevron do `Select`. Desabilita a seta que cruzaria `min`/`max`. `tabindex="-1"` + `aria-hidden`: é afordância de ponteiro para o que ↑/↓ já fazem no input, que continua sendo um `spinbutton` com `min`/`max`. Motivo: o spinner do engine saía **preto** no Windows/WebView2 e só no hover no macOS.
  fixo à direita, `appearance: none`.
- **Password com olho:** input `type="password"` + botão ghost-icon `Eye`/`EyeOff` (16px) dentro do
  campo, à direita — alterna `type` e o ícone. Regra do `PRD.md` (§3): o olho revela **apenas o valor
  ainda não salvo** digitado na sessão atual; um segredo já persistido no keyring nunca é
  redecifrado para exibição em texto puro, mesmo com o olho aberto.
- **A11y:** `<label for>` associado; campo de senha usa `aria-label` adicional indicando o estado
  atual do olho ("Mostrar segredo" / "Ocultar segredo").

### ErrorText
- Mensagem de erro inline (`Field`, `Select`, `DriveModule`, `S3Module`, `QosModule`): `<p role="alert">`
  com ícone `AlertCircle` 14px em `--color-error` + texto em `text-body-sm`/`--color-text-primary`
  (nunca a frase inteira em `--color-error` — falha AA sobre `--color-surface-2`, §10). O ícone carrega
  o tom de erro (piso não-textual 1.4.11, 3,0:1), o texto fica sempre legível (14,12:1, Pass).
- `id` opcional para `aria-describedby` do campo associado; `role="alert"` mantém o anúncio automático
  por leitor de tela sem exigir `aria-live` do chamador.

### FileDropField
- Variante de `Field` para o JSON da Service Account (`pick_service_account_file`): estado vazio é
  uma zona tracejada (`--color-border-hairline`, `border-style: dashed`) com ícone `Key` +
  instrução; estado preenchido vira um chip (ícone `Key` em container 28px, nome do arquivo em
  `mono-data`, tamanho + `client_email` em `label-sm` `--color-text-quaternary`, botão "Substituir").
- Nunca exibe o conteúdo do JSON — só os metadados que o command devolve (`file_name`, `size`,
  `client_email`, `project_id`), conforme contrato do IPC (§9 do `SPEC.md`: "nunca o conteúdo").

### Slider (QoS)
- **Anatomia:** rótulo + dot de marca (`--color-primary`/`--color-secondary`) · badge numérico atual
  (`mono-data`, fundo `--color-surface-0`, texto na cor de marca) · trilho `<input type="range">` ·
  4 tick labels abaixo (`label-sm`, `--color-text-quaternary`): **0,5 MB/s · 2,5 MB/s · 5,0 MB/s ·
  10,0 MB/s** (`step="0.5"`, `min="0.5"`, `max="10.0"`, RF-050) · opção "Ilimitado" fora da faixa
  numérica (extremo direito do trilho ou toggle separado, desabilita o teto e sincroniza com o
  campo `limit_mbps: null` do command `set_qos`).
- **Trilho:** 4px altura, `--radius-full`, fundo `--color-surface-hover`. **Thumb:** 13px, círculo
  (`--radius-full`), preenchido com `--color-text-primary`, anel de 2px na cor de marca do destino
  (`--color-primary-strong`/`--color-secondary-strong`) — dois sliders visualmente distintos por
  destino mesmo lado a lado.
- **A11y:** `aria-valuemin`/`aria-valuemax`/`aria-valuenow`/`aria-valuetext` (com unidade "MB/s")
  no `input[type=range]`, navegável por seta do teclado (RF-050 aplica em ≤ 2s via `set_qos`).

### Toggle
- Track 28×16px, `--radius-full`, fundo `--color-surface-hover` (off) / `--color-primary-container`
  (on); thumb 12px `--color-text-secondary` (off) / `--color-text-primary` (on), desloca 12px.
  Usado em "Modo Noturno" (Should-have, RF-053) e nos toggles de módulo (subpastas por data,
  checksum pré-upload).
- **A11y:** `role="switch"` + `aria-checked`.

### Checkbox
- 14×14px, `--radius-sm`, borda `--color-border-strong`; marcado: fundo `--color-primary`, ícone
  `Check` (`--color-on-primary`, 10px). Usado nos toggles "sempre on" do módulo Drive (ex. checksum
  MD5 pré-upload).

### Button
Quatro variantes, duas alturas:

| Variante | Fundo | Texto | Borda | Uso |
|---|---|---|---|---|
| **primary** | `--color-primary-container` | `--color-on-primary` | `--color-primary` a 30% | ação de confirmação forte (ex. "Limpar Concluídos", "Salvar Preferências") |
| **secondary** | `--color-surface-2` | cor de marca do contexto (`--color-secondary` em "Pausar Watcher") | cor de marca a 30% | ação reversível marcada (ex. "Pausar Watcher") |
| **destructive** | `--color-surface-2` | `--color-error` | `--color-error` a 30% | ação irreversível (reservado — nenhuma das 2 telas tem hoje um botão puramente destrutivo; cancelar job usa o padrão ghost-icon) |
| **ghost-icon** | transparente | `--color-text-secondary` | nenhuma | ação secundária de toolbar (ex. "Atualizar lista"), toda `RowActions` |

- **Alturas:** 24px quadrado (`size="icon"`) para os botões inline do `RowActions`; 28px para botão
  de toolbar; 32px para botão de rodapé/modal (ex. "Salvar Preferências", "Cancelar / Restaurar
  Padrões").
- **Hover:** `--color-surface-hover` (ghost/secondary) ou leve clareamento do próprio
  `--color-primary-container` (primary — variação de luminosidade sem token dedicado; implementar
  via `filter: brightness(1.1)` — não existe um token de hover dedicado hoje, não
  inventar um hex novo).
- **A11y:** todo `ghost-icon` sem texto visível leva `aria-label` describindo a ação (ex.
  `aria-label="Pausar transferência"`); `:focus-visible` sempre com `--ring-focus`.

### Tooltip
- Dica curta para controle só-ícone (todo o `RowActions`). Fundo `--color-surface-2`, borda
  `--color-border-hairline`, `--radius` (4px), texto `label-md`, padding `--space-xs`/`--space-2xs`,
  `--shadow-popover`. Aparece 4px acima do gatilho e vira para baixo quando não há espaço.
- **Timing:** 250ms de espera no hover (evita piscar ao varrer a linha com o mouse); **imediato** no
  foco por teclado — quem navega por Tab não deveria esperar para saber o que o ícone faz.
- **A11y:** `role="tooltip"` ligado por `aria-describedby`; **nunca** substitui o `aria-label` do
  botão — a dica descreve, o rótulo nomeia. `Escape` fecha sem tirar o foco do controle.
- Renderizado por portal em `document.body`: dentro da `JobTable` o `overflow-x-auto` do wrapper
  recortaria a bolha.

### Card / Module
- Container de nível 2 (`--color-surface-2`, borda `--color-border-hairline`, `--radius-md`),
  cabeçalho com ícone (18–20px) + título `headline-sm` + status chip, corpo com `Field`s agrupados
  por linha estrutural (`border-top: 1px solid --color-border-hairline`) em vez de gap de espaço em
  branco — reduz a "distância ocular" entre campos relacionados. Rodapé opcional com resultado de
  `test_connection` (`TestResult`: ok/message/latency_ms) + botão "Testar Conexão"/"Testar Bucket".

### Banner (auth-required)
- Faixa de largura total, fundo `--color-error-container`, texto `--color-on-primary`, ícone
  `AlertCircle` (18px), mensagem com o `hint` do evento `auth-required` (e-mail da SA a compartilhar,
  ou política IAM faltante) + ação "Ir para Configurações".
- Aparece no topo do Dashboard sempre que `AppStatus.destinations[x].auth_required` é verdadeiro —
  estado bloqueante, listado como um dos "estados vazios" da tela (§9).
- **A11y:** `role="alert"` (interrupção ativa, diferente do `aria-live="polite"` usado em KPIs/status
  contínuos — `auth-required` é uma mudança que exige ação, não só leitura).

### EmptyState
- Ícone grande (24px, `--color-text-quaternary`), título `title-md`, descrição `body-md`
  `--color-text-secondary`, ação primária opcional. Três variantes nas 2 telas: sem pasta
  selecionada (RF-060), sem credenciais configuradas, fila vazia (`list_jobs` retorna `total: 0`).

### Toast / Notification
- Container `--radius-lg`, `--shadow-popover` (única sombra do sistema fora de popover, por ser
  também flutuante), fundo `--color-surface-3`, borda `--color-border-strong`; ícone de status +
  mensagem `body-md` + fechar (`X`, 16px). Usado para confirmações pontuais (ex. resultado de
  `retry_all_failed`) e para o feedback de "Preferências salvas" quando o rodapé de `/settings` não
  está visível (fora do viewport rolado).
- **A11y:** `role="status"` + `aria-live="polite"`; nunca a única forma de comunicar um erro crítico
  (auth-required sempre usa `Banner` persistente, não `Toast` que some sozinho).

24 componentes documentados nesta seção (`TitleBar`, `Sidebar`, `NavItem`, `FolderCard`,
`ThroughputWidget`, `StatusBar`, `KpiCard`, `JobTable`, `JobRow`, `DualProgress`, `StatusBadge`,
`RowActions`, `LogConsole`, `LogLine`, `Field`, `ErrorText`, `FileDropField`, `Slider`, `Toggle`,
`Checkbox`, `Button`, `Card`/`Module`, `Banner`, `EmptyState`, `Toast` — 25 ao contar `Toast` separadamente).

---

## 9. Telas

Ambas as rotas compartilham o shell fixo (`TitleBar` + `Sidebar` + `StatusBar`, ver §8) — os mapas
abaixo cobrem só a área de conteúdo rolável entre eles.

### `/dashboard` — Arquivos & Fila

```
┌─────────────────────────────────────────────────────────────────┐
│ [Banner auth-required]  (condicional, RF-060)                    │
├─────────────────────────────────────────────────────────────────┤
│ Cabeçalho: título + botões (Atualizar · Pausar Watcher · Limpar) │
├─────────────────────────────────────────────────────────────────┤
│ [KpiCard] [KpiCard] [KpiCard] [KpiCard] [KpiCard]                │
│  Total     Concl.    Enviando  Na Fila   Falhas                  │
├─────────────────────────────────────────────────────────────────┤
│ Filtro: Todos/Ativos/Concluídos/Falhas    [Watcher: Ativo · N]  │
│ ┌───────────────────────────────────────────────────────────┐   │
│ │ JobTable header (Arquivo·Tamanho·GDrive·S3·Status·Ações)   │   │
│ │ JobRow × N (paginação de 50)                                │   │
│ └───────────────────────────────────────────────────────────┘   │
├─────────────────────────────────────────────────────────────────┤
│ ▾ LogConsole (ancorado no rodapé, colapsável, ring 500 linhas)   │
│   recolhido: header vira ticker — última linha após "Rust Core"  │
└─────────────────────────────────────────────────────────────────┘
```

**Altura:** o `/dashboard` preenche a área de conteúdo (`h-full`) como coluna
flex em vez de fluir livre dentro dela. Cabeçalho, banner, grade de KPI e
`LogConsole` são `shrink-0`; só a região da `JobTable` é `flex-1 min-h-0`, com
o corpo da tabela rolando por conta própria (`overflow-auto`) e a linha de
cabeçalho `sticky`. Consequência: recolher o `LogConsole` **entrega** os 220px
do corpo dele à lista, em vez de só encurtar a página. A tabela tem piso de
`min-h-[200px]` — abaixo disso a área de conteúdo volta a rolar, em vez de
espremer as linhas.

**`LogConsole` recolhido:** o header (32px) passa a exibir a **última linha**
logo após o chip "Rust Core" — `LogTicker`, mesmo `formatLogTs`, mesmo chip de
origem e mesma escalada de cor por nível que o `LogLineRow` do corpo. Uma linha
só, sem marquee e sem animação (nada a desfazer sob `prefers-reduced-motion`);
o valor troca no lugar conforme os eventos chegam. Respeita o filtro de nível.
Timestamp some abaixo de 1200px, mesmo breakpoint em que o header já esconde o
texto "ao vivo". `aria-live="off"` de propósito: anunciar cada linha soterraria
o resto da página, e o corpo expandido (`role="log"`, `aria-live="polite"`)
segue sendo a superfície anunciada.

| Região | Componente | Data binding | RF |
|---|---|---|---|
| Banner | `Banner` (auth-required) | `AppStatus.destinations[].auth_required`, evento `auth-required` | RF-060 |
| Botões de ação | `Button` ghost/secondary/primary | commands `list_jobs`, `pause_watcher`/`resume_watcher`, `clear_completed` | RF-012, RF-013 |
| Grade de KPI | `KpiCard` × 5 | `AppStatus` (contadores por status) + agregados de `list_jobs` | RF-060 a RF-064 |
| Filtro de status | tabs/segmented control | parâmetro `statuses` de `list_jobs` | RF-064 |
| Status do watcher | `WatcherStatusBadge` | `AppStatus.watcher_paused` + `selectKpis().detected` | RF-060 |
| Tabela | `JobTable` / `JobRow` / `DualProgress` / `StatusBadge` / `RowActions` | `list_jobs` (paginação `limit/offset` de 50) + eventos `job-updated`, `upload-progress` | RF-060, RF-064 |
| Console | `LogConsole` / `LogLine` | `get_recent_logs` + evento `log-line` | — |

**Estados:**
- **Carregando:** skeleton de `KpiCard` (blocos `--color-surface-hover` pulsando lento) e de
  `JobRow` (3 linhas placeholder) enquanto a primeira chamada de `list_jobs`/`get_status` resolve.
- **Vazio — sem pasta:** `EmptyState` cobrindo a área da tabela, ação "Selecionar pasta"
  (`pick_folder`).
- **Vazio — sem credenciais:** `EmptyState` com ação "Ir para Configurações".
- **Vazio — fila zerada:** `EmptyState` leve (ícone `CheckCircle2`, "Tudo sincronizado") quando
  `list_jobs` retorna `total: 0` com filtro "Ativos".
- **Erro:** `StatusBadge` "Falha" por linha + link "Detalhes" (não há um estado de erro de página
  inteira — falha é sempre por job, a tela em si sempre carrega se o core responde).
- **auth-required:** `Banner` persistente no topo + jobs do destino afetado ficam com `StatusBadge`
  "Pausado" até nova credencial (RNF de resiliência).
- **Watcher pausado:** dot do `FolderCard` e badge da `Sidebar` mudam para `--color-text-quaternary`
  estático; botão "Pausar Watcher" vira "Retomar Watcher" (ícone troca para `Play`, fora da tabela
  de §7 — adicionar ao mapeamento se `Play` for necessário futuramente).
- **Filtro "Concluídos" com paginação:** navegação de 50 em 50 (`limit/offset` de `list_jobs`),
  controles de paginação em `mono-data` no rodapé da tabela (ex. "1–50 de 214").

**Atalhos de teclado:** `Ctrl+R` — Atualizar lista (`list_jobs` forçado); `Esc` — fecha
`RowActions`/menu de contexto aberto ou limpa o filtro de detalhes de erro expandido.

### `/settings` — Configurações & QoS

```
┌─────────────────────────────────────────────────────────────────┐
│ Cabeçalho: título + status compacto (GDrive: OK · AWS S3: OK)    │
├───────────────────────────────┬───────────────────────────────┤
│ [Card] Google Drive            │ [Card] Amazon S3                │
│  status · SA JSON · Folder ID  │  status · Access Key · Region   │
│  · toggles · rodapé teste      │  · Secret (olho) · Bucket ·     │
│                                 │  Storage Class · rodapé teste   │
├───────────────────────────────┴───────────────────────────────┤
│ [Card full-width] QoS — Slider Drive · Slider S3 · Modo Noturno │
├─────────────────────────────────────────────────────────────────┤
│ Rodapé fixo: "Última alteração salva às HH:MM"                   │
│              [Cancelar/Restaurar Padrões]  [Salvar Preferências] │
└─────────────────────────────────────────────────────────────────┘
```

| Região | Componente | Data binding | RF |
|---|---|---|---|
| Módulo Google Drive | `Card`/`Module`, `FileDropField`, `Field` | `AppConfig` (drive), `CredentialStatus.gdrive`, command `pick_service_account_file`/`set_credential` | RF-080 a RF-083 |
| Módulo AWS S3 | `Card`/`Module`, `Field` (password c/ olho) | `AppConfig` (s3), `CredentialStatus.aws`, commands `set_credential`/`test_connection` | RF-084 a RF-086 |
| Módulo QoS | `Slider` × 2, `Toggle` (Modo Noturno) | `AppConfig.qos`, command `set_qos` | RF-050, RF-053 |
| Rodapé | `Button` primary/secondary, `mono-data` timestamp | command `save_config`, `Ctrl+S` | RF-097 |

**Estados:**
- **Carregando:** `Field`s em skeleton até `get_config`/`get_credential_status` resolverem.
- **Vazio — sem credencial:** módulo mostra estado "Não configurado" (chip cinza `--color-text-quaternary`
  em vez de "Conectado / Online") e o `FileDropField`/campos ficam na variante vazia.
- **Erro de teste:** rodapé do módulo mostra `TestResult.message` em `--color-error` no lugar do
  texto de sucesso.
- **auth-required:** mesmo `Banner` do Dashboard pode aparecer no topo desta tela também, já que o
  evento é global ao `AppStatus`.
- **Salvo:** botão "Salvar Preferências" troca temporariamente para um estado de confirmação (ícone
  `Check`, fundo levemente esverdeado) antes de voltar ao rótulo padrão — o próprio mockup de QoS já
  demonstra essa troca via JS (`handleSavePreferences`).

**Atalhos de teclado:** `Ctrl+S` — Salvar Preferências (aplica em runtime sem reiniciar o app,
exceto se `watch.path` mudou); `Esc` — fecha um `select` aberto ou reverte um campo em edição não
salva para o valor persistido.

---

## 10. Acessibilidade

Meta geral (RNF-013): WCAG 2.1 nível AA — 4,5:1 para texto normal, 3,0:1 para texto grande
(≥18,66px regular ou ≥14px em negrito) e para componentes de interface não-textuais. Todos os
valores abaixo foram computados pela fórmula de luminância relativa do WCAG
(`L = 0.2126·R + 0.7152·G + 0.0722·B` em sRGB linearizado, `contraste = (L1+0.05)/(L2+0.05)`) sobre
os hex reais de `design/tokens.css` — não estimados.

### Texto sobre superfície

| Par (texto / fundo) | Contraste | AA normal (4,5:1) | AA grande (3,0:1) |
|---|---|---|---|
| `--color-text-primary` / `--color-surface-0` | 15,19:1 | Pass | Pass |
| `--color-text-primary` / `--color-surface-1` | 14,63:1 | Pass | Pass |
| `--color-text-primary` / `--color-surface-2` | 14,12:1 | Pass | Pass |
| `--color-text-primary` / `--color-surface-3` | 13,42:1 | Pass | Pass |
| `--color-text-emphasis` / `--color-surface-2` | 11,79:1 | Pass | Pass |
| `--color-text-label` / `--color-surface-1` | 10,68:1 | Pass | Pass |
| `--color-text-status` / `--color-surface-2` | 9,57:1 | Pass | Pass |
| `--color-text-secondary` / `--color-surface-0` | 5,83:1 | Pass | Pass |
| `--color-text-secondary` / `--color-surface-1` | 5,61:1 | Pass | Pass |
| `--color-text-secondary` / `--color-surface-2` | 5,42:1 | Pass | Pass |
| `--color-text-secondary` / `--color-surface-3` | 5,15:1 | Pass | Pass |
| `--color-text-tertiary` / `--color-surface-0` | 5,15:1 | Pass | Pass |
| `--color-text-tertiary` / `--color-surface-2` | 4,78:1 | Pass | Pass |
| `--color-text-tertiary` / `--color-surface-3` | 4,55:1 | Pass (margem de 0,05) | Pass |
| `--color-text-quaternary` / `--color-surface-0` | 3,63:1 | **Fail** | Pass |
| `--color-text-quaternary` / `--color-surface-2` | 3,37:1 | **Fail** | Pass |

**Mitigação `--color-text-quaternary`:** falha para texto normal em qualquer superfície — o token é
usado por design só para placeholder, tick label de slider e timestamp de rodapé, nunca para uma
frase que o usuário precise ler com atenção. Regra de implementação: `--color-text-quaternary` só em
`label-sm`/`body-sm` de contexto puramente decorativo/redundante (o valor real também está disponível
em outro elemento com contraste AA — ex. o tick "0,5 MB/s" do slider é redundante com o badge
numérico `mono-data` do próprio slider, que usa `--color-text-emphasis`/cor de marca). Nunca usar em
mensagem de erro, rótulo de campo obrigatório ou qualquer texto que seja a única fonte da informação.

### Marca sobre superfície / container

| Par | Contraste | AA normal | AA grande |
|---|---|---|---|
| `--color-on-primary` / `--color-primary-container` (Button primary) | 10,86:1 | Pass | Pass |
| `--color-secondary` / `--color-surface-1` | 5,22:1 | Pass | Pass |
| `--color-secondary` / `--color-surface-2` | 5,04:1 | Pass | Pass |
| `--color-primary` / `--color-surface-1` | 4,63:1 | Pass | Pass |
| `--color-primary` / `--color-surface-2` | 4,47:1 | **Fail** (por 0,03) | Pass |
| `--color-tertiary` / `--color-surface-1` | 4,46:1 | **Fail** (por 0,04) | Pass |
| `--color-tertiary` / `--color-surface-2` | 4,31:1 | **Fail** | Pass |
| `--color-error` / `--color-surface-2` | 4,09:1 | **Fail** | Pass |
| `--color-primary` / `--color-primary-container` | 2,99:1 | Fail | **Fail** |
| `--color-secondary` / `--color-secondary-container` | 3,51:1 | Fail | Pass |
| `--color-tertiary` / `--color-tertiary-container` | 3,07:1 | Fail | Pass |
| `--color-error` / `--color-error-container` | 3,53:1 | Fail | Pass |

**Mitigação `--color-primary`/`--color-tertiary`/`--color-error` sobre `--color-surface-1/2`:** as
falhas são todas por margem pequena (0,03–0,4) e o uso real nunca é texto corrido — é sempre um
rótulo curto (`label-sm`/`mono-data`, ex. "Google Drive", "Watcher Ativo", "Falha 403") **sempre
emparelhado com um dot de 6px da mesma cor** (§2, §8 `StatusBadge`). O dot não carrega
responsabilidade de contraste de texto, mas garante que a informação de estado não depende só da
legibilidade do texto colorido — quem não distingue o texto ainda vê a posição/presença do dot e lê
o rótulo textual ao lado (`--color-text-*`, sempre Pass). Regra de implementação: **nunca** usar
`--color-primary`/`--color-secondary`/`--color-tertiary`/`--color-error` como cor de um parágrafo ou
frase longa — reservados a rótulo curto + dot, ou a ícone (onde WCAG 1.4.11, não 1.4.3, se aplica,
piso 3,0:1, que todos os pares acima cumprem).

**Mitigação implementada — `--color-error` em frase de erro (`ErrorText`, §8):** todo ponto do app
que antes exibia uma frase completa em `text-error` (falha de 4,09:1 sobre `--color-surface-2` acima)
usa o componente `ErrorText`: ícone `AlertCircle` 14px em `--color-error` (carrega o tom, piso
1.4.11 cumprido) + texto da mensagem em `--color-text-primary` (14,12:1, Pass). Aplicado em `Field`,
`Select`, `DriveModule`, `S3Module`, `QosModule`; o parágrafo de hint do `AuthRequiredBanner` segue o
mesmo princípio (texto em `--color-text-primary` dentro do container `role="alert"`, que mantém
`--color-error` só na borda/fundo/ícone/título curto).

**Mitigação texto-sobre-próprio-container** (`primary`/`primary-container`,
`tertiary`/`tertiary-container` etc.): esse par **não é usado como texto** em nenhum componente
documentado no §8 — é reservado para os casos em que a marca aparece como *ícone* sobre seu próprio
container (ex. ícone do módulo Google Drive, `--color-primary` sobre um chip de fundo
`--color-primary-container`-like a 10%, não o container sólido). Se um componente futuro precisar de
texto sobre o container sólido, o par correto é `--color-on-primary`/`--color-primary-container`
(10,86:1, Pass) — nunca a cor de marca plana sobre seu próprio container.

### Badges de status (texto sobre fundo `color-mix`)

| Par | Fundo efetivo (aprox., blend sobre `--color-surface-2`) | Contraste | AA grande |
|---|---|---|---|
| `--color-primary` / `--color-status-info-bg` | `#1b222c` | 3,98:1 | Pass |
| `--color-secondary` / `--color-status-warning-bg` | `#212225` | 4,46:1 | Pass |
| `--color-tertiary` / `--color-status-success-bg` | `#192228` | 3,88:1 | Pass |
| `--color-error` / `--color-status-error-bg` | `#271f27` | 3,65:1 | Pass |

Todos os quatro passam no piso de 3,0:1 (texto curto + dot redundante, mesma justificativa acima);
nenhum atinge 4,5:1, então nenhum badge de status deve carregar texto além do rótulo curto de uma
palavra (`Enviando`, `Sincronizado`, `Na Fila`, `Falha`, `Pausado`).

### Bordas e foco (WCAG 1.4.11, não-texto, piso 3,0:1)

| Par | Contraste | Piso 3,0:1 |
|---|---|---|
| `--color-border-strong` / `--color-surface-0` | 2,41:1 | **Fail** |
| `--color-border-strong` / `--color-surface-2` | 2,24:1 | **Fail** |
| `--color-border-hairline` / `--color-surface-0` | 1,24:1 | Fail (esperado — ver abaixo) |

`--color-border-hairline` é *deliberadamente* subcontraste — é um divisor estrutural decorativo
(linha de tabela, contorno de card), não um limite de componente que o usuário precisa perceber para
operar a interface; WCAG 1.4.11 não se aplica a divisores puramente decorativos. `--color-border-strong`
falhando o piso de 3,0:1 é uma lacuna real, mas **não é o mecanismo de foco do sistema** — todo
elemento interativo usa `--ring-focus` (`0 0 0 1px color-mix(in srgb, var(--color-primary) 33%,
transparent)`, calculado sobre `--color-primary` puro a 33% de opacidade, que blendado sobre
`--color-surface-0` resulta em contraste equivalente ao par `--color-primary`/`--color-surface-0`
listado acima — 4,63:1 pelo componente de cor plena, e o próprio anel de foco tem 1px de espessura
mais halo, o que em navegadores modernos passa no teste de foco visível do WCAG 2.4.7 apesar do
valor do canal alpha isolado). Recomendação de implementação: nunca depender só de
`--color-border-strong` para indicar que um elemento é interativo/focável — sempre combinar com
`--ring-focus` no estado de foco e com mudança de fundo (`--color-surface-hover`) no hover.

### Mecânica geral de acessibilidade

- **Foco visível:** `:focus-visible { box-shadow: var(--ring-focus); }` em todo elemento interativo
  — nunca `outline: none` sem substituto.
- **`aria-live="polite"`:** container de KPIs (§8 `KpiCard`), status por destino da `StatusBar`, badge
  de contagem da `Sidebar` — todo valor que muda via evento IPC sem o usuário ter iniciado a ação.
- **`role="log"` + `aria-live="polite"`:** corpo do `LogConsole` (§8) — histórico que cresce, não
  须 role="alert" (não interrompe).
- **`role="alert"`:** exclusivo do `Banner` de `auth-required` — única condição que exige interrupção
  ativa do fluxo do usuário.
- **`aria-label` obrigatório:** todo botão ghost-icon sem texto visível (`RowActions`, botões da
  title bar, olho de senha, copiar ID, abrir pasta).
- **`prefers-reduced-motion: reduce`:** desativa o pulso do dot "Enviando" (§6) e reduz toda
  `transition`/`animation` a 1ms, aplicado com escopo local por componente, nunca com um seletor
  universal `*` que quebraria transições que dependem de duração real.
- **Navegação por teclado:** `Tab`/`Shift+Tab` percorre `NavItem` → `FolderCard` → conteúdo da
  página → `LogConsole`; `Slider` responde a setas (RF-050); atalhos globais `Ctrl+S` (salvar),
  `Ctrl+R` (atualizar lista), `Esc` (fechar popover/menu) não conflitam com o foco de nenhum campo de
  texto (não são capturados dentro de um `<input>` focado, exceto onde o próprio campo trata `Esc`
  para reverter edição não salva).

---

## 11. Anti-padrões

Proibido em qualquer componente ou tela deste app, sem exceção:

- **Gradiente decorativo** em fundo, botão ou card (gradientes utilitários de *mask*/opacidade em
  overlay técnico, se algum dia necessários, não contam como decorativos — mas nenhum componente
  documentado neste sistema usa gradiente de nenhum tipo).
- **Glow neon ou halo saturado** em qualquer estado — inclusive hover e foco (`--ring-focus` é um
  anel sólido de 1px, não um `box-shadow` difuso brilhante).
- **`--radius-full` em botão ou badge retangular** (pill shape) — `--radius-full` é exclusivo de
  elementos circulares (dot, thumb, avatar).
- **`#ffffff` (branco puro)** como cor de texto ou fundo — o teto de claridade é
  `--color-on-primary` (`#f0f4f9`), e só sobre `--color-primary-container`.
- **Fonte ou ícone via CDN** — Inter e JetBrains Mono são vendorizadas em `public/fonts/`; ícones são
  `lucide-react` (SVG inline), nunca Google Fonts Material Symbols (bloqueado por CSP, RNF-014).
- **`box-shadow` difuso em `Card`/`KpiCard`/módulo** — profundidade vem só de tom + borda hairline
  (§5); sombra é exclusiva de elementos flutuantes (`--shadow-popover`).
- **Animação com física de mola (`spring`/`bounce`)** em qualquer transição, especialmente em barra
  de progresso — dado de rede real não "quica" (§6).
- **Ícones Material Symbols no produto final** — só existem nos mockups Stitch como referência de
  layout; todo ícone do app é `lucide-react` (§7).
- **Tema claro** — o produto é escuro-only; não implementar um toggle de tema nem estilos
  condicionais `prefers-color-scheme: light`.
- **Hex literal fora da tabela de §2** — qualquer cor em CSS/componente referencia um `--color-*`
  token; um hex direto em `className`/`style` é sempre um bug de implementação.

---

## 12. Implementação

`src/styles/app.css` é o ponto de entrada de estilo do renderer React e faz a ponte entre
`design/tokens.css` e o `@theme` do Tailwind v4:

```css
@import "tailwindcss";
@import "../../design/tokens.css";

@theme {
  --color-surface-0: var(--color-surface-0);
  --color-surface-1: var(--color-surface-1);
  --color-surface-2: var(--color-surface-2);
  --color-surface-3: var(--color-surface-3);
  --color-surface-hover: var(--color-surface-hover);
  --color-border-hairline: var(--color-border-hairline);
  --color-border-strong: var(--color-border-strong);
  --color-primary: var(--color-primary);
  --color-primary-container: var(--color-primary-container);
  --color-primary-strong: var(--color-primary-strong);
  --color-secondary: var(--color-secondary);
  --color-secondary-container: var(--color-secondary-container);
  --color-secondary-strong: var(--color-secondary-strong);
  --color-tertiary: var(--color-tertiary);
  --color-tertiary-container: var(--color-tertiary-container);
  --color-error: var(--color-error);
  --color-error-container: var(--color-error-container);
  /* ...restante das cores, 1:1 com os nomes semânticos de tokens.css... */

  --font-sans: var(--font-sans);
  --font-mono: var(--font-mono);

  --text-body-md: var(--text-body-md-size);
  --text-body-md--line-height: var(--text-body-md-leading);
  /* Tailwind v4 usa o namespace --text-* (+ sufixo --line-height); os tokens de
     tokens.css já separam size/leading/weight/tracking/family por degrau
     (display, headline-lg/md/sm, title-md, body-lg/md/sm, mono-data, label-md/sm)
     — mapear cada um da mesma forma. */

  --radius-sm: var(--radius-sm);
  --radius: var(--radius);
  --radius-md: var(--radius-md);
  --radius-lg: var(--radius-lg);
  --radius-full: var(--radius-full);
  --spacing: var(--space-xs);
}
```

Fontes ficam em `public/fonts/` (`InterVariable.woff2`, `JetBrainsMono[wght].woff2`) e são
declaradas via `@font-face` dentro do próprio `design/tokens.css` (já presente), consumidas pelo
Vite como asset estático — nenhuma referência de rede em tempo de execução, cumprindo RNF-014
(offline) e a CSP `default-src 'self'`.

**Ordem de importação:** `tokens.css` antes do bloco `@theme`, para que as variáveis já existam
quando o Tailwind gera as classes utilitárias derivadas. Manter os tokens em arquivo próprio (em vez
de hex direto em `@theme`) preserva a rastreabilidade até os mockups documentada em
`design/README.md`.

**Regras de precedência entre este documento e as demais fontes:**

- **Mockup HTML (`assets/*/code.html`) vs. este documento** → este documento vence, sempre — os
  mockups são referência de layout/densidade, não a spec final (ver a resolução da divergência de
  altura da title bar em §4).
- **Este documento vs. `PRD.md`/`SPEC.md` em questão de comportamento** (o que uma tela faz, quais
  campos existem, qual command é chamado) → `PRD.md`/`SPEC.md` vencem — este documento define
  aparência, dimensão, token e estado visual, não regra de negócio.
- **`SPEC.md` §8 é explícito** sobre essa mesma regra ("em caso de conflito... o design vence no
  visual e este documento [SPEC.md] vence no comportamento") — este documento a aplica sem exceção.
