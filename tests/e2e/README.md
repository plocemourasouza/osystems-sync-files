# Teste de longa duração (T-6.1 / T-6.6)

Script único, sem dependências (`node:fs`, `node:child_process`, `node:crypto`,
`node:os` — só built-ins de Node), que valida **RNF-001** (RSS varia ≤ 10 % em
72 h) e **RF-039** (nunca enviar o mesmo arquivo duas vezes ao mesmo destino,
ou seja, zero duplicatas remotas).

Arquivo: `tests/e2e/longevity.mjs`. Duas fases: **gerar** (roda ao lado do app
por 72 h) e **relatório** (roda depois, cruzando manifest + CSV + listagens
remotas).

## 1. Fase de geração

Roda em paralelo com o app `osystems-sync` de verdade, apontando `--dir` para
a **mesma pasta que o app está monitorando**.

### macOS / Linux

```bash
node tests/e2e/longevity.mjs \
  --dir "$HOME/OneDrive-teste" \
  --hours 72 \
  --interval-min 1 \
  --process-name osystems-sync \
  --out tests/e2e/longevity-$(date +%Y%m%d-%H%M).csv \
  --sizes "16KB,512KB,4MB,64MB,256MB" \
  --seed 42
```

### Windows (PowerShell)

```powershell
node tests/e2e/longevity.mjs `
  --dir "C:\Users\voce\OneDrive-teste" `
  --hours 72 `
  --interval-min 1 `
  --process-name osystems-sync `
  --out "tests/e2e/longevity-$(Get-Date -Format yyyyMMdd-HHmm).csv" `
  --sizes "16KB,512KB,4MB,64MB,256MB" `
  --seed 42
```

Ou via npm (mesmos argumentos depois de `--`):

```bash
npm run longevity -- --dir /caminho/pasta-monitorada --hours 72
```

O script:

- cria um arquivo `lt_<n>_<tamanho>.bin` por intervalo (`--interval-min`,
  padrão 1 min), alternando pela lista `--sizes`, com conteúdo pseudo-aleatório
  determinístico (seed `xorshift32`) escrito em chunks de 1 MB (simula uma
  cópia real). Cada arquivo tem sha256 único — a idempotência do app nunca
  deveria descartar um desses arquivos como duplicata.
- grava cada arquivo gerado em `manifest.json` (dentro de `--dir`, ou em
  `--manifest <path>`): `{ ts, n, file, size, sha256 }`.
- a cada `--sample-interval-sec` (padrão 300 s = 5 min) amostra:
  - RSS do processo `--process-name` (`ps -o rss=` no macOS/Linux;
    `Get-Process ... WorkingSet64` via PowerShell no Windows);
  - contagens da fila via `sqlite3` CLI no `state.db` do app (
    `%APPDATA%/osystems-sync/state.db` no Windows,
    `~/Library/Application Support/osystems-sync/state.db` no macOS),
    override com `--db <path>`.
  - grava tudo em uma linha do CSV (`--out`).
- `Ctrl+C` interrompe com segurança: grava uma amostra final, faz flush do
  manifest e imprime um resumo.

Se o `sqlite3` CLI não estiver instalado, ou o `state.db` não existir ainda,
o script **avisa uma vez** e deixa as colunas de fila em branco no CSV — ele
nunca inventa um valor. Mesma coisa se o processo `--process-name` não for
encontrado: RSS fica em branco naquela amostra.

### Colunas do CSV

| Coluna | Significado |
|---|---|
| `ts` | timestamp ISO 8601 da amostra |
| `rss_kb` | RSS do processo do app, em KB (vazio se o processo não foi encontrado) |
| `files_local` | `select count(*) from files` no `state.db` (vazio se sem acesso ao DB) |
| `jobs_pending` / `jobs_uploading` / `jobs_done` / `jobs_failed` | `select status, count(*) from jobs group by status` |

## 2. Coletando as listagens remotas (opcional, mas recomendado)

Para provar "contagem local = contagem S3 = contagem Drive" e "zero
duplicatas remotas" (métricas do PRD §6), gere dois arquivos texto no formato
`nome sha256` (uma linha por objeto) e passe para o modo relatório.

### S3

```bash
aws s3 ls --recursive s3://SEU_BUCKET/ | awk '{print $4}' | while read -r key; do
  sha=$(aws s3api head-object --bucket SEU_BUCKET --key "$key" --query Metadata.sha256 --output text)
  echo "$(basename "$key") $sha"
done > tests/e2e/s3-list.txt
```

Se o app não grava o sha256 como metadata do objeto (`x-amz-meta-sha256`),
baixe e recalcule (`sha256sum`) ou ajuste o upload para setar esse metadata —
sem isso o cross-check de hash fica limitado à comparação de nomes.

### Google Drive

Não há CLI oficial equivalente ao `aws s3 ls`; use a API `files.list`
(escopo `drive`, mesma Service Account do app) filtrando pela pasta de
destino, paginando com `pageToken`, e para cada arquivo leia o campo
`appProperties.sha256` (se o app gravar) ou baixe e recalcule o hash.
Formate a saída no mesmo padrão `nome sha256` em `tests/e2e/drive-list.txt`.

## 3. Fase de relatório

Depois que o teste terminar (72 h ou interrompido):

```bash
node tests/e2e/longevity.mjs --mode report \
  --manifest /caminho/pasta-monitorada/manifest.json \
  --csv tests/e2e/longevity-20260101-0000.csv \
  --s3-list tests/e2e/s3-list.txt \
  --drive-list tests/e2e/drive-list.txt
```

Ou via npm:

```bash
npm run longevity:report -- --manifest <manifest.json> --csv <csv>
```

Gera `tests/e2e/longevity-report.md` (e imprime o mesmo conteúdo no
terminal) com:

- contagem local (manifest) vs S3 vs Drive, e se batem;
- duplicatas de sha256 **locais** (bug do próprio gerador — se acontecer,
  desconfie do resto do relatório) e **remotas** (violação de RF-039: mesmo
  arquivo enviado duas vezes ao mesmo destino);
- arquivos "perdidos" (gerados localmente, ausentes na listagem remota);
- RSS mínimo/máximo, mediana da primeira hora vs mediana da última hora, e o
  drift percentual entre elas;
- tabela **PASS / FAIL** contra RNF-001 (drift de RSS ≤ 10 %) e RF-039 (zero
  duplicatas por destino).

**Importante**: se `--s3-list`/`--drive-list` não forem passados, essas linhas
aparecem como `SKIPPED (no listing provided)` — o script nunca marca `PASS`
sem ter de fato comparado os dados. Da mesma forma, sem amostras de RSS
válidas o RNF-001 fica `SKIPPED`.

## 4. Self-test rápido (não é o teste de 72 h)

Para validar que o script funciona antes de rodar o teste de verdade, use
`--dry-run` com uma janela curta:

```bash
node tests/e2e/longevity.mjs --dir /tmp/osync-lt-selftest \
  --dry-run --minutes 2 --interval-sec 10 --out /tmp/osync-lt.csv

node tests/e2e/longevity.mjs --mode report \
  --manifest /tmp/osync-lt-selftest/manifest.json --csv /tmp/osync-lt.csv
```

Isso só confirma que o gerador escreve arquivos/manifest/CSV corretamente e
que o relatório roda sem erro — com uma janela de minutos o drift de RSS não
tem significado estatístico (o relatório mostra um aviso quando a janela é
menor que 2h).

## 5. Testes unitários da lógica pura

```bash
npm run test:e2e:unit
```

Cobre: sha256/tamanho exatos por arquivo gerado, determinismo do gerador
(mesma seed + índice → mesmo conteúdo), unicidade de sha256 entre índices
diferentes, e a matemática do relatório (mediana, drift de RSS, detecção de
duplicatas locais/remotas, cross-check de listagem remota) sobre fixtures em
memória — sem tocar o app real.

Roda com `node --test` (não Vitest) **de propósito**: fica fora de
`npm run test`, para não acoplar o teste de longa duração ao runner unitário
padrão do projeto (a suíte real fala com o filesystem/relógio e não deve
disparar em todo `npm run test`).

## 6. Limites conhecidos

- O relatório assume que cada arquivo gerado é um **caminho novo**
  (`lt_<n>_<tamanho>.bin`) — ele não exercita o caminho "mesmo `path`,
  conteúdo mudou" de RF-039 (`upsert_file_and_enqueue`, ver `SPEC.md §5`).
  Isso precisa de um teste dedicado que reescreve um arquivo existente.
- A amostragem de RSS depende de `ps`/`Get-Process` conseguirem enxergar o
  processo do app rodando na mesma máquina — não funciona contra um app
  rodando em outra máquina/container.
- O cross-check de hash remoto só é tão bom quanto o metadata que o app
  grava no S3/Drive; sem isso, o script cai para comparação por nome.
