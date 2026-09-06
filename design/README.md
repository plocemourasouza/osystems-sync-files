# Design Tokens — oSystems Sync

`tokens.css` foi extraído por união dos hex realmente usados nos dois mockups aprovados
(`assets/osystems_sync_dashboard_fila_de_arquivos/code.html` e
`assets/osystems_sync_configura_es_qos_de_banda/code.html` — tanto o `<script id="tailwind-config">`
quanto as classes `bg-[#...]`/`text-[#...]`/`border-[#...]` inline), com nomes semânticos por papel
(surface, border, text, primary/secondary/tertiary/error, status). Onde as duas telas usavam um hex
levemente diferente para o mesmo papel, os valores foram comparados por um ΔE CIE76 aproximado
(sRGB→Lab); pares com ΔE < ~6 (diferença perceptualmente desprezível) foram fundidos num único
token canônico (valor do mockup do Dashboard, por ser a tela de referência), e o hex do outro mockup
fica documentado em comentário ao lado do token no próprio `tokens.css`. Pares com ΔE maior
(ex.: `#6082a4` vs `#4f7cac`, ΔE≈8.8) foram tratados como cores reais e distintas — não como
duplicata — e viraram tokens `-strong` separados (`--color-primary-strong`, `--color-secondary-strong`)
porque cumprem um papel visual diferente (preenchimento de barra/slider vs. texto/ícone) dentro do
próprio mockup de QoS. Os nomes de escala tipográfica, os nomes `unit-*` de espaçamento, a escala de
`rounded` e as dimensões de componente (altura de linha de tabela, sidebar, titlebar/statusbar) vêm
das seções estruturais de `assets/executive_precision/DESIGN.md` — a paleta de cores desse arquivo
(tanto no frontmatter YAML quanto nas menções de hex no corpo em prosa) foi descartada por completo,
conforme a decisão C10 do `PRD.md` ("paleta do mockup é canônica; DESIGN.md tinha 2 paletas internas
divergentes"), e as famílias de fonte foram trocadas de `hankenGrotesk`/`jetbrainsMono` para
**Inter**/**JetBrains Mono**, conforme a decisão C9 ("fontes vendorizadas, Inter + JetBrains Mono").

## Mapeamento para Tailwind v4 `@theme`

Este arquivo é a fonte da verdade dos tokens, não o `@theme` em si. No CSS de entrada do Tailwind
(ex. `src/index.css`), importe `tokens.css` antes do bloco `@theme` e referencie as custom properties
via `var(...)`:

```css
@import "tailwindcss";
@import "./tokens.css";

@theme {
  --color-surface-0: var(--color-surface-0);
  --color-primary: var(--color-primary);
  --color-primary-container: var(--color-primary-container);
  /* ...demais cores, 1:1 com os nomes já semânticos deste arquivo... */

  --font-sans: var(--font-sans);
  --font-mono: var(--font-mono);

  --text-body-md: var(--text-body-md-size);
  --text-body-md--line-height: var(--text-body-md-leading);
  /* Tailwind v4 usa o namespace --text-*  (+ sufixo --line-height) para a
     escala tipográfica; --font-weight-* e --tracking-* para peso e
     letter-spacing. Os tokens deste arquivo já separam size/leading/weight/
     tracking por step (display, headline-lg/md/sm, title-md, body-lg/md/sm,
     mono-data, label-md/sm) exatamente para caber nesses três namespaces
     sem transformação adicional. */

  --radius: var(--radius);
  --radius-sm: var(--radius-sm);
  --radius-md: var(--radius-md);
  --radius-lg: var(--radius-lg);
  --spacing: var(--space-xs); /* Tailwind v4 deriva toda a escala p-*, gap-*, etc. de --spacing; os --space-2xs…3xl aqui ficam disponíveis para uso direto onde a escala derivada não bater com um valor de grid específico do app (ex. table-row-compact). */
}
```

Manter os tokens em um arquivo próprio (em vez de escrever os hex direto dentro de `@theme`)
preserva a rastreabilidade até os mockups/DESIGN.md documentada acima e permite trocar/curar valores
sem reabrir a extração.

## Fusões (ΔE < 6, mesmo token)

| Token canônico | Valor | Hex do outro mockup fundido | ΔE aprox. |
|---|---|---|---|
| `--color-surface-0` | `#0c0e12` | `#080a0d`, `#0e1017`, `#0e1117` | 1.5–2.4 |
| `--color-surface-2` | `#13171f` | `#12151c`, `#141720` | 0.9–1.3 |
| `--color-surface-3` | `#171c26` | `#191d27` | 0.7 |
| `--color-surface-hover` | `#1f242e` | `#212633` | 2.4 |
| `--color-border-strong` | `#475161` | `#525a6b` | 4.1 |
| `--color-text-primary` | `#e2e4ea` | `#d8dce3` | 3.1 |
| `--color-text-emphasis` | `#cad2df` | `#c5c9d4` | 3.4 |
| `--color-text-status` | `#b4bece` | `#b4bac7` | 2.4 |
| `--color-text-secondary` | `#828ea2` | `#8e95a5` | 4.1 |
| `--color-text-tertiary` | `#788596` | `#7a8192` | 2.4 |
| `--color-text-quaternary` | `#5f6c80` | `#5b6274` | 4.3 |
| `--color-primary` | `#6082a4` | `#5b84ad` | 4.25 |

## Mantidos separados (ΔE alto — não são duplicata)

- `--color-primary` `#6082a4` vs `--color-primary-strong` `#4f7cac` (ΔE≈8.8) — o mockup de QoS usa o
  segundo como cor de preenchimento de barra/thumb, mais saturado que o tom usado em texto/ícone.
- `--color-secondary` `#a38258` vs `--color-secondary-strong` `#8c6e48` (ΔE≈8.3) — mesmo padrão, lado
  S3 do mesmo painel de QoS.
- `--color-danger-hover` `#8f3941` (mockup Dashboard) vs `#78282b` (mockup QoS, mesmo botão de
  fechar) (ΔE≈8.6) — a spec deste token fixa o valor do Dashboard como canônico; o hex do QoS não
  entrou em `tokens.css`.
- `#949bb0` (título da titlebar no mockup de QoS) e `#666d7e` (rótulo de seção do menu no mockup de
  QoS) divergem demais (ΔE 15.8 e 9.3) dos tons de texto equivalentes do Dashboard para serem
  tratados como o mesmo token; ficaram de fora de `tokens.css` por não fazerem parte do conjunto
  pedido nem serem indispensáveis — variação da segunda iteração de mockup.
