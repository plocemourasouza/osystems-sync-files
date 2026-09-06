# oSystems Sync Files to Drive & S3 Buckets
## Documentação Funcional e Arquitetura de Recursos (v2.0)

**Plataforma:** Windows 11 Desktop nativo (Tauri v2 + Rust Core + React/Tailwind)  
**Identidade Visual:** Executive Precision Dark Mode, estética acrílica translúcida, tipografia Inter com suporte técnico JetBrains Mono e paleta sobria fosca (`#101319` / `#0b0e14`).

---

### 1. Visão Geral da Aplicação
O **oSystems Sync Files to Drive & S3 Buckets** é um agente desktop de alta performance projetado para automação contínua de backup e replicação paralela de arquivos locais para duas infraestruturas de nuvem simultaneamente: **Google Drive** e **Amazon Web Services (AWS S3)**.

O sistema opera com um núcleo de baixa latência em **Rust**, proporcionando consumo mínimo de memória RAM, rate limiting preciso e tolerância a falhas com re-tentativas automáticas.

---

### 2. Estrutura Global e Componentes Permanentes (Shell / Janela Nativa)

#### 2.1. Barra de Título Customizada (Windows 11 Chrome - 40px)
- **Identificador de Aplicação:** Logotipo vetorial sutil integrado com o nome da solução: `oSystems Sync Files to Drive & S3 Buckets`.
- **Badge do Motor de Execução:** Indicador de status do backend Rust (`Core Engine: Active [Rust v1.78]`).
- **Controles de Janela Nativos:** Botões de Minimizar, Maximizar/Restaurar e Fechar estilizados com comportamento do Windows 11.

#### 2.2. Barra Lateral Fixa (Sidebar de Navegação & Telemetria)
- **Menu de Navegação:**
  - **Dashboard & Fila (`/dashboard`):** Acesso à visualização em tempo real das transferências, fila de arquivos e logs.
  - **Configurações & QoS (`/settings`):** Gerenciamento de credenciais, buckets, pastas remotas e limitação de taxa de upload.
- **Card Interativo de Diretório Monitorado (*Folder Watcher*):**
  - Formato visual de pasta estilizada interativa com efeito translúcido e bordas sutis.
  - Exibição limpa do nome do diretório ativo (ex: `D:\Projetos\BackupLocal`).
  - Indicador de estado em tempo real: Badge pulsante `Watcher: Ativo`.
  - Ação de clique direto para abrir o explorador de arquivos ou reconfigurar o diretório monitorado.
- **Widget de Telemetria de Banda Global:**
  - Medidor de vazão combinada instantânea (ex: `2.8 MB/s / 6.0 MB/s`).
  - Gráfico de barras segmentado demonstrando o rate individual distribuído em tempo real entre Google Drive e AWS S3.

#### 2.3. Barra de Status Inferior (Statusbar - 28px)
- **Latência de Rede:** Indicador em milissegundos para os endpoints de API da Google e AWS.
- **Status das Conexões:** Badges contextuais de conectividade (`GDrive: Online`, `AWS S3: Online`).
- **Target da Build:** Metadados de compilação do binário (`Tauri 2.0 • Windows x64 • SHA-256 Engine`).

---

### 3. Tela 1: Dashboard & Fila de Arquivos (`/dashboard`)

#### 3.1. Topbar Operacional e Indicadores Chave (KPIs)
- **Cards de Métricas de Transferência:**
  - **Total de Arquivos Detectados:** Volume de itens catalogados no ciclo atual.
  - **Arquivos Sincronizados:** Quantidade de itens transferidos com validação de checksum MD5/ETag.
  - **Em Transferência / Na Fila:** Total de processos ativos e aguardando thread de envio.
  - **Falhas / Re-tentativas:** Alertas operacionais para intervenção rápida.
- **Botões de Controle Rápido:**
  - **Atualizar Fila (`Refresh`):** Força uma nova varredura no sistema de arquivos local.
  - **Pausar / Retomar Watcher:** Congela a detecção de novos eventos no sistema de arquivos sem interromper uploads em andamento.

#### 3.2. Tabela de Sincronização Dupla Paralela (Grid de Arquivos)
Projetada com layout sóbrio sem molduras pesadas ou bordas quadradas artificiais ao redor dos elementos:
- **Coluna 1 — Arquivo & Origem (30%):**
  - Ícone vetorial sem moldura contextual ao tipo de arquivo (`.dump`, `.zip`, `.mp4`, `.tar`, `.sql`).
  - Nome completo do arquivo e caminho relativo em fonte monoespaçada.
  - Timestamp da última modificação local.
- **Coluna 2 — Tamanho (9%):**
  - Tamanho real do arquivo formatado (`MB` / `GB`) com alinhamento tabular à direita.
- **Coluna 3 — Google Drive (19%):**
  - Barra de progresso dedicada com gradiente dinâmico sutil.
  - Exibição de percentual de conclusão (`%`) e taxa individual de upload em tempo real (`MB/s`).
- **Coluna 4 — AWS S3 (19%):**
  - Barra de progresso dedicada para o bucket AWS.
  - Taxa de transmissão concorrente individualizada.
- **Coluna 5 — Status (13%):**
  - Célula centralizada e independente sem risco de sobreposição.
  - Badges textuais de alta legibilidade:
    - `Enviando` (com pulso azul elétrico sutil).
    - `Sincronizado` (verde esmeralda suave).
    - `Na Fila` (cinza neutro).
    - `Falha / Erro` (vermelho fosco com código HTTP/API).
- **Coluna 6 — Ações (10%):**
  - Ações rápidas contextuais representadas por ícones limpos e transparentes:
    - Abrir localização no explorador local.
    - Pausar envio individual do arquivo.
    - Reenviar / Forçar novo upload.
    - Menu de opções avançadas (`...`).

#### 3.3. Painel de Log de Eventos e Telemetria em Tempo Real
- Console de logs rolável na base da tela exibindo eventos do Rust Core:
  - Criação de buffers, chunks de multipart upload S3, handshake Google Drive API e confirmações de integridade.

---

### 4. Tela 2: Configurações & QoS de Banda (`/settings`)

#### 4.1. Módulo de Integração: Google Drive
- **Credencial da Conta de Serviço (Service Account):**
  - Upload/Seleção de arquivo de credenciais criptografadas no formato `.json`.
  - Exibição do status do certificado e e-mail da conta de serviço vinculada.
- **ID da Pasta de Destino (*Folder ID*):**
  - Campo de entrada com acabamento escuro fosco (`#0c0e12`) e micro-bordas sóbrias de 1px (`#1f242e`), sem contornos brancos reflexivos.
  - Suporte a unidades compartilhadas (*Google Workspace Shared Drives*).
- **Teste de Conectividade:**
  - Botão de verificação de autenticação e permissões de escrita com feedback inline.

#### 4.2. Módulo de Integração: AWS S3
- **Campos de Acesso e Segurança:**
  - `Access Key ID`: Chave pública de autenticação IAM.
  - `Secret Access Key`: Chave secreta de autenticação mascarada com visualizador de alternância rápida.
  - `AWS Region`: Dropdown com seleção de região (ex: `us-east-1`, `sa-east-1`, `eu-west-1`).
  - `Bucket Name`: Nome do bucket de armazenamento de destino.
  - `Storage Class`: Seleção da classe de armazenamento (Standard, Intelligent-Tiering, Glacier Instant Retrieval).
  - *Todos os campos padronizados com background escuro fosco e ausência de bordas claras invasivas.*
- **Teste de Conectividade:**
  - Botão de validação de rota e permissão de `s3:PutObject` / `s3:ListBucket`.

#### 4.3. Módulo de Controle de Banda (QoS / Rate Limiting)
- **Controle Dedicado por Provedor:**
  - Slider independente para **Google Drive Upload Limit** (0.5 MB/s a 10.0 MB/s / Ilimitado).
  - Slider independente para **AWS S3 Upload Limit** (0.5 MB/s a 10.0 MB/s / Ilimitado).
- **Indicadores Numéricos de Precisão:**
  - Badge dinâmica refletindo o teto de banda configurado no momento da regulagem.
- **Prevenção de Saturação de Link:**
  - Opção de limitação de consumo de banda para preservar conexões corporativas de rede.

#### 4.4. Ações Globais de Configuração
- **Restaurar Padrões:** Redefine os limites de QoS e caminhos recomendados de fábrica.
- **Salvar Preferências (*Ctrl+S*):** Persiste os parâmetros no arquivo de configuração encriptado local da aplicação e atualiza o watcher em tempo de execução sem necessidade de reinicialização.

---

### 5. Resumo Tecnológico e Regras de Negócio
- **Concorrência Assíncrona:** Uploads em paralelo sem concorrência destrutiva entre os destinos.
- **Validação de Hash:** Validação de integridade entre os arquivos locais e os objetos remotos para evitar uploads duplicados.
- **Design Executivo:** Interface visual de nível corporativo, com acabamento escuro de alta legibilidade, efeitos translúcidos e total ausência de poluição visual.
