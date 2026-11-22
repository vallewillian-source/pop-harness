# Especificação de UX/UI — Console Web Administrativo do POP Harness (Rust)

> **Documento:** `console/UX_SPEC.md`  
> **Tarefa:** HR-37  
> **Base de Requisitos:** Visão geral `docs/VISAO_GERAL.md` (o antigo PRD §10 e §13 está em `docs/JEV.md` e `docs/CONSOLE.md`), Decisões `DECISOES.md` (D-03 a D-10), Paridade `PARIDADE_AGENTE.md`, e Spike de UI `RELATORIO_HR05.md`  
> **Data:** 19/11/2026

---

## 1. Identidade Visual, Tokens e Fundações Técnicas

O Console Web do POP Harness é uma ferramenta de uso **estritamente interno** (administração, suporte e engenharia da SofaCoding). O objetivo primário de design é a **alta densidade de informação, ergonomia operacional prolongada e ausência de ruído estético**. Não se trata de uma landing page comercial: a interface privilegia escaneabilidade, alinhamento tabular rígido, tipografia proporcional e monoespacada nos lugares certos, e clareza imediata sobre o estado operacional do motor.

### 1.1 Decisão Formal de Tokens e Cores de Fundo

| Papel | Token SofaUI | Valor Hexadecimal | Uso no Console |
|---|---|---|---|
| **Fundo Escuro Padrão (Dark Default)** | `--pop-bg` / `primary` / `surface` | **`#151719`** | **Fundo oficial e canônico.** A instrução legada que citava `#121a24` fica **formalmente revogada e rejeitada** nesta especificação, consolidando a paridade absoluta com o `DESIGN.md` e os componentes da SofaUI. |
| **Fundo Claro (Light Peer)** | `--pop-bg-light` | **`#F5F6F7`** | Ativado via atributo `data-theme="light"` no elemento raiz `<html>` ou classe `cpd-theme-light` no `<body>`. |
| **Superfície Elevada (Cards/Sidebar)** | `--pop-surface-raised` | **`#1C1F22`** | Sidebar lateral, Topbar, cabeçalhos de seções, painéis de formulários e caixas de diálogo. |
| **Superfície Recuada / Vidro Fosco** | `--pop-surface-glass` | **`#212529`** | Fundo de inputs, cabeçalhos de tabela (`<thead>`), blocos de código e cartões secundários. |
| **Superfície Hover / Interativa** | `--pop-surface-hover` | **`#2A2F34`** | Linhas de tabela selecionadas, itens de menu ativos e botões secundários ao passar o mouse. |
| **Linha Divisória Sutil** | `--pop-line` | **`#2C3236`** | Bordas de cards, divisores horizontais e contornos de campos de formulário. |
| **Linha de Ênfase / Foco** | `--pop-line-strong` | **`#494F57`** | Bordas de zonas de upload drag & drop e contornos de elementos com foco. |
| **Texto Primário (Títulos e Dados)** | `--pop-text-primary` | **`#EEF1F4`** | Textos principais, rótulos de campos e valores numéricos importantes. Contraste 13.8:1 sobre `#151719`. |
| **Texto Secundário (Descrições)** | `--pop-text-secondary` | **`#C0C7CE`** | Parágrafos de apoio, textos auxiliares de formulários e valores de tabelas. Contraste 8.9:1. |
| **Texto Suavizado (Metadados/Muted)** | `--pop-text-muted` | **`#8D959E`** | Slugs, hashes SHA-256, timestamps, atalhos de teclado e placeholders. Contraste 5.2:1 (atende WCAG AA). |
| **Ação Principal / Destaque (Accent)** | `--pop-accent` | **`#F5A623`** | Botões de ação primária (CTA), abas ativas, cursores de streaming e indicadores de status em dirty. |
| **Destaque Suave (Accent Soft)** | `--pop-accent-soft` | **`#372C1A`** | Fundo de badges de destaque, alertas de aviso e banners de conversas transferidas. |
| **Texto do Accent** | `--pop-accent-text` | **`#16120A`** | Texto escuro de alto contraste sobre o botão âmbar `--pop-accent`. |
| **Sucesso / Operação Saudável** | `--pop-success` | **`#68D39A`** | Badges de agentes ativos, instâncias online, scores aprovados no RAG e 200 OK. |
| **Perigo / Ação Destrutiva / Erro** | `--pop-danger` / `danger-fill` | **`#F28B8B`** / **`#C24A4A`** | Badges de erro, botões de exclusão irreversível, HTTP 5xx e cancelamento de turno. |
| **Informativo / Processamento** | `--pop-blue` | **`#3B82F6`** | Balões de mensagem do cliente de teste, indicadores de streaming e gráficos de waterfall. |

### 1.2 Consequências Mandatórias do Consumo da SofaUI (HR-05 / D-03)

O teste de integração da biblioteca `@sofacoding/sofa-ui` realizado na HR-05 estabeleceu três requisitos estruturais invioláveis para o Console em Vite/React (HR-38):

1. **Roteamento Estático de Fontes:** A SofaUI utiliza caminhos absolutos `/res/fonts/...` no seu CSS compilado. O Console Web (através do `rust-embed` e do servidor axum) **deve obrigatoriamente servir os arquivos WOFF2 de `IBM Plex Sans` e `IBM Plex Mono` na rota `/res/fonts/`**. No build de desenvolvimento Vite, a pasta `apps/frontend/res` deve ser mapeada via alias ou link público em `apps/pop-harness/console/public/res`.
2. **Estilização Canônica do `<body>`:** A SofaUI é uma biblioteca apresentacional pura que não estiliza o elemento raiz do hospedeiro. O Console deve declarar explicitamente em seu arquivo de entrada (`src/main.css` ou `index.html`):
   ```css
   html, body {
     margin: 0;
     padding: 0;
     background-color: var(--pop-bg, #151719);
     color: var(--pop-text-primary, #EEF1F4);
     font-family: var(--pop-font-sans, "IBM Plex Sans", -apple-system, BlinkMacSystemFont, sans-serif);
     -webkit-font-smoothing: antialiased;
     -moz-osx-font-smoothing: grayscale;
   }
   code, pre, .font-mono {
     font-family: var(--pop-font-mono, "IBM Plex Mono", monospace);
   }
   ```
3. **Singleton do React:** A biblioteca consome o React e o ReactDOM fornecidos pelo app hospedeiro (peer dependency). O bundler Vite deve garantir uma única instância em runtime (`resolve.dedupe: ["react", "react-dom"]`).

---

## 2. Arquitetura de Informação e Mapa de Navegação

O Console é servido como Single Page Application (SPA) embutida no binário `poph`, acessada na raiz `/`. O acesso requer um Bearer Token administrativo validado pelo axum.

### 2.1 Hierarquia de Rotas e Telas

```text
/login                                         --> Tela de Login Admin (Token)
│
├── /orgs                                      --> Lista de Organizações (Multi-tenant)
│   ├── /orgs/new                              --> Modal de Criação de Org
│   │
│   └── /orgs/:org_id                          --> Painel da Organização Selecionada
│       ├── /structure                         --> Categorias de Produtos & Departamentos
│       ├── /agents                            --> Catálogo de Agentes de Atendimento
│       │   ├── /agents/new                    --> Modal de Novo Agente (Create)
│       │   │
│       │   └── /agents/:agent_id              --> Detalhes do Agente (7 Abas)
│       │       ├── #instructions              --> Aba 1: Instruções (Formulário + Prévia)
│       │       ├── #knowledge-base            --> Aba 2: Base de Conhecimento & RAG
│       │       ├── #products                  --> Aba 3: Produtos & Preços Oficiais
│       │       ├── #files                     --> Aba 4: Biblioteca de Arquivos (16 MB)
│       │       ├── #routing                   --> Aba 5: Roteamento & Handoff Humano
│       │       ├── #integrations              --> Aba 6: WhatsApp & Plugins Externos
│       │       └── #model-jev                 --> Aba 7: Parâmetros LLM & Gates Jev
│       │
│       └── /agents/:agent_id/test-chat        --> Chat de Teste Completo
│           └── ?turn=:turn_id (Drawer)        --> Painel Lateral do "Por Quê" por Turno
│
└── /operation                                 --> Painel Operacional da Instância
    ├── #health                                --> Métricas de Saúde (RAM, Filas, Latências)
    ├── #cluster                               --> Topologia do Cluster Estático (cluster.toml)
    ├── #plugins                               --> Supervisor de Processos de Plugins
    └── #migration                             --> Importador POP Atende & Backup de Org
```

### 2.2 Estrutura do Shell Global (App Shell)

O layout divide-se em três zonas fixas:
1. **Sidebar Retrátil (240 px fixos):**
   - **Cabeçalho:** Logotipo "POP HARNESS", badge de runtime "CORE RUST" e versão do binário.
   - **Seletor de Organização:** Dropdown com busca rápida. Quando logado com token de org dedicada, o seletor fica bloqueado exibindo a org única.
   - **Navegação Agrupada:**
     - *GESTÃO:* Organizações, Categorias, Departamentos, Agentes.
     - *OPERAÇÃO:* Saúde da Instância, Topologia do Cluster, Plugins Externos, Importar / Exportar.
   - **Rodapé de Telemetria:** Indicador de pulso verde com o nó local (`node-01:8080`), escopo do token ativo e atalho para o diálogo de atalhos de teclado (`[?]`).
2. **Topbar Global (56 px altura):**
   - Breadcrumbs hierárquicos clicáveis (`Organizações / Exemplo / Agentes / Dra. Beatriz`).
   - Badge de ambiente (`DEV LOCAL`, `HOMOLOGAÇÃO`, `PRODUÇÃO`).
   - Indicador de status da conexão SSE em tempo real (`● SSE CONECTADO`).
   - Perfil de autenticação e botão de Logout / Troca de Token.
3. **Workspace Central (Largura fluida, mín. 1200 px):**
   - Contêiner denso com rolagem independente, evitando rolagem dupla na página.

---

## 3. Especificação Detalhada de Cada Tela e Componente

Abaixo, cada tela é detalhada com: objetivo, campos, contratos de API (rotas de HR-20 a HR-53), comportamento em falha e estados (populado, vazio, carregando e erro).

---

### 3.1 Tela de Login por Token (`/login`)
- **Objetivo:** Autenticação interna sem cadastro de usuários finais. O operador informa um token gerado via CLI (`poph token create`) ou configurado nas variáveis de ambiente do binário.
- **Campos e Controles:**
  - `admin_token`: Campo de texto mascarado (`type="password"` com toggle para revelar), estilo `SofaInput` escuro.
  - `token_scope`: Seletor de escopo (Global / Todas as Orgs ou Restrito por Org).
  - `target_instance`: URL do endpoint axum (padrão `http://127.0.0.1:8080`).
  - Botão de Ação: `Entrar no Console` (`SofaButton` variante primária âmbar).
- **Dados da API:**
  - `POST /v1/auth/verify` com payload `{ token: string }`.
  - Resposta de Sucesso: `200 OK` `{ valid: true, scope: "global" | "org", org_id: string | null, permissions: string[] }`.
- **Tratamento de Falha:**
  - `401 Unauthorized`: Mensagem inline em vermelho "Token administrativo inválido ou revogado. Verifique com a equipe de DevOps ou gere um novo via CLI."
  - Falha de Rede / Conexão recusada: Banner de erro "Não foi possível alcançar a instância em http://127.0.0.1:8080. Verifique se o processo poph está em execução."
- **Armazenamento:** O token é mantido exclusivamente na memória do cliente React (em um contexto/store de sessão) e repassado no header `Authorization: Bearer <token>`. **Nunca é persistido em `localStorage` sem autorização explícita**.

---

### 3.2 Lista de Organizações (`/orgs`)
- **Objetivo:** Visualização multi-tenant das organizações cadastradas na instância e no cluster, alocação de nós e limites operacionais.
- **Tabela Densa (`SofaTable`):**
  - Colunas: `Organização / Slug`, `Instância Alocada`, `Agentes Ativos`, `Retenção (dias)`, `Limite de Concorrência`, `Ações`.
  - Ações por linha: `Acessar Organização`, `Editar Configuração`, `Backup (.poph-org)`.
- **Dados da API:**
  - `GET /v1/orgs?cluster=true`
  - Resposta: `200 OK` `[{ id, name, slug, instance_id, active_agents_count, retention_days, concurrency_limit, is_local, version }]`.
- **Comportamento em Falha:**
  - Erro 503 / Timeout: Se a listagem do cluster falhar ou o nó remoto não responder em 3 segundos, exibe a tabela com o nó inacessível marcado em vermelho e banner de aviso com botão de retentativa rápida.
- **Estados:**
  - *Populado:* Linhas com status verde, slugs monoespacados e métricas numéricas alinhadas.
  - *Vazio:* Card com contorno tracejado, ícone `[ ∅ ]`, título "Nenhuma organização cadastrada" e CTA "+ Criar Primeira Organização".
  - *Carregando:* Shimmer de 5 linhas simulando a estrutura exata da tabela.
  - *Erro:* Card com borda vermelha `--pop-danger-fill`, stacktrace do erro HTTP 503 e botão para rodar `poph doctor`.

---

### 3.3 Categorias e Departamentos da Organização (`/orgs/:org_id/structure`)
- **Objetivo:** Gestão estrutural em duas colunas complementares: Departamentos (destinos de handoff do Jev) e Categorias de Produtos (agrupamento do catálogo).
- **Coluna 1 — Departamentos da Organização:**
  - Lista de departamentos com ID único (`dep_general`, `dep_clinico`), descrição, quantidade de agentes vinculados e badge `ATIVO`/`INATIVO`.
  - Botão `+ Novo Departamento` abrindo modal com nome e descrição.
  - Dados: `GET/POST /v1/orgs/:org_id/departments`, `PATCH /v1/orgs/:org_id/departments/:id` (com `If-Match`).
- **Coluna 2 — Categorias de Produtos:**
  - Lista de categorias com slug (`ortodontia`, `implantes`), contagem de produtos vinculados e status ativo.
  - Conforme D-05: categorias inativas ocultam seus produtos da busca do agente.
  - Dados: `GET/POST /v1/orgs/:org_id/product-categories`, `PATCH /v1/orgs/:org_id/product-categories/:id` (com `If-Match`).
- **Tratamento de Falha:**
  - Em falha de carregamento, o lado com erro exibe estado de falha com retry independente, sem bloquear o outro painel.

---

### 3.4 Catálogo de Agentes de Atendimento (`/orgs/:org_id/agents`)
- **Objetivo:** Ponto central de acesso aos agentes de IA configurados na organização selecionada.
- **Tabela de Agentes:**
  - `Avatar e Cor de Identidade`: Círculo com iniciais e cor característica (8 opções canônicas: lima, ciano, violeta, âmbar, azul, coral, rosa, verde).
  - `Nome e Slug`: Nome operacional e identificador estável único na org (`dra-beatriz`).
  - `Departamento Padrão`: Sala/depto padrão onde conversas do agente são criadas.
  - `Modelo LLM`: Nome do modelo configurado (ex: `gemini-3.8-flash-high`).
  - `WhatsApp`: Número conectado (`+55 11 98765-4321`) ou badge "Desconectado".
  - `Status (D-05)`: Badge `ATIVO` (verde) ou `INATIVO` (cinza escuro). Conforme D-05: agente inativo não abre turnos nem recebe mensagens.
  - `Ações`: Botão `Editar` (navega para as 7 abas) e atalho `Chat de Teste`.
- **Dados da API:**
  - `GET /v1/orgs/:org_id/agents`
  - Resposta: `200 OK` `[{ id, name, slug, identity_color, department_name, model_name, whatsapp_number, status, version }]`.

---

### 3.5 As Sete Abas do Agente (`/orgs/:org_id/agents/:agent_id`)

O cabeçalho do agente é fixo em todas as abas, exibindo: Avatar, Nome, Slug, Departamento Padrão, Badge de Status, Indicador de versão (`If-Match: v4`), Botão primário `Salvar (v4)` com indicador visual de alterações pendentes (ponto âmbar pulsante quando o formulário for dirty) e botão `Chat de Teste ↗`.

Abaixo do cabeçalho, o `SofaTabSwitcher` organiza as 7 abas funcionais:

#### Aba 1: Instruções (`#instructions` — HR-40)
- **Layout:** Split-View em duas colunas de proporção 55% / 45%.
- **Coluna da Esquerda — Formulário Estruturado:**
  1. *Identidade:* Nome do agente (`identity.name`), Papel/Função (`identity.role`), Empresa (`identity.company`), Descrição detalhada da empresa.
  2. *Princípios e Regras Invioláveis:* Lista dinâmica de regras com campo de texto e seleção de Fallback Action (`say_dont_know`, `transfer`, `escalate`).
  3. *Escopo:* "O que o agente faz" e "O que o agente NÃO faz" (textareas de 3 linhas).
  4. *Tom de Voz:* Seletores de Idioma (`pt-BR`, `en`, `es`), Estilo (`professional`, `casual`, `technical`, `friendly`), Emojis (`moderate`, `none`, `free`), Formato (`simple_lists`, `free_text`, `tables`) e Restrições adicionais.
  5. *Ferramentas Habilitadas (D-04):* Toggles reais que filtram a lista de tools enviada ao provedor LLM (`consultar_base`, `buscar_produtos`, `enviar_arquivo`, etc.).
  6. *Instruções Extras:* Textarea livre para diretrizes operacionais de exceção.
- **Coluna da Direita — Prévia do Prompt Composto em Markdown:**
  - Caixa de código monoespacada que exibe em tempo real o markdown que o servidor compila (`instructions_markdown`), com realce de sintaxe suave e botão "Atualizar Prévia".
- **Dados da API e Concorrência (`If-Match`):**
  - Leitura: `GET /v1/orgs/:org_id/agents/:id/instructions`
  - Escrita: `PATCH /v1/orgs/:org_id/agents/:id/instructions` com header `If-Match: "{version}"`.
  - Tratamento de Conflito de Versão (`412 Precondition Failed`):
    - Abre o modal `VersionConflictModal` com exibição de diff lado a lado (Versão no Servidor vs Rascunho Local).
    - Opções: "Descartar e Recarregar vX", "Copiar Rascunho para Clipboard" ou "Sobrescrever Forçado (Incrementar Versão)".

#### Aba 2: Base de Conhecimento & RAG (`#knowledge-base` — HR-41)
- **Objetivo:** Gestão dos trechos de conhecimento do agente, métricas do índice vetorial local e ferramenta de teste de recuperação híbrida.
- **Barra de Métricas do RAG:**
  - Total de chunks indexados, Dimensão vetorial (384d int8), Versão do Chunker (~350 caracteres / 80 sobreposição conforme D-09) e Status do Índice (`ATUALIZADO` ou `REINDEXAÇÃO NECESSÁRIA`).
  - Botão de ação: `Reindexar Agente` (`POST /v1/orgs/:org_id/agents/:id/reindex`).
- **Coluna da Esquerda — Grupos e Trechos:**
  - Lista de grupos de conhecimento vinculados ao agente.
  - Editor de Item com **contador rígido de 1000 caracteres**:
    - Barra de progresso visual que transiciona de verde para âmbar (> 800 caracteres) e vermelho (> 950 caracteres).
    - Bloqueio rígido de envio caso exceda 1000 caracteres.
- **Coluna da Direita — Busca de Teste Híbrida (Vetor + FTS5):**
  - Campo para digitar perguntas de teste e simular a recuperação do motor.
  - Exibição do Top-3 com scores de relevância normalizados, indicação de fonte e rótulo claro se o item ficou abaixo do limiar de corte (`0.70`).
- **Dados da API:**
  - `GET /v1/orgs/:org_id/knowledge-bases`
  - `POST/PATCH/DELETE /v1/orgs/:org_id/knowledge-bases/:id/items` com `If-Match`.
  - `POST /v1/orgs/:org_id/agents/:id/test-search` `{ query: string, top_k: number }`.

#### Aba 3: Produtos (`#products` — HR-42)
- **Objetivo:** Catálogo de produtos vinculados ao agente com preços e condições imutáveis.
- **Regra Mandatória de Negócio (PRD §7.2 e §9):** Preço, prazos e condições financeiras são campos tipados e estruturados que vêm exclusivamente do banco de dados oficial. O LLM não calcula nem inventa preços: ele lê a resposta estruturada da tool `buscar_produtos`.
- **Tabela Densa de Produtos:**
  - Colunas: `SKU / Código`, `Produto / Serviço`, `Categoria`, `Preço Oficial / Condições`, `Aliases / Sinônimos`, `Status`, `Ações`.
  - Exibição de aliases fundamentais para a busca por intenção (ex: "aparelho transparente", "invisalign").
- **Dados da API:**
  - `GET/POST /v1/orgs/:org_id/products`, `PATCH/DELETE /v1/orgs/:org_id/products/:id`.
  - `GET /v1/orgs/:org_id/agents/:id/products` (vínculos).

#### Aba 4: Biblioteca de Arquivos (`#files` — HR-43 / HR-24)
- **Objetivo:** Gestão de arquivos e mídias que o agente tem permissão de enviar via ferramenta `enviar_arquivo(id)` durante o atendimento.
- **Zona de Upload Drag & Drop:**
  - Suporte a PDF, DOCX, XLSX, PPTX e imagens (JPEG, PNG, WEBP).
  - Limite rígido de **16 MB por arquivo**.
- **Tabela de Arquivos do Agente:**
  - Colunas: `Nome do Arquivo`, `Tipo MIME`, `Tamanho`, `SHA-256`, `Extração de Texto`, `Descrição para o LLM`, `Ações`.
  - *Extração de Texto:* Exibe a contagem de caracteres extraídos em processo isolado. Se o PDF for escaneado (sem OCR), exibe badge amarelo de aviso: "PDF sem texto (sem OCR)".
  - *Descrição para o LLM:* Metadado crítico que é injetado no system prompt para orientar o modelo sobre quando enviar o arquivo.
- **Dados da API:**
  - `GET /v1/orgs/:org_id/agents/:id/files`
  - `POST /v1/orgs/:org_id/agents/:id/files` (multipart/form-data até 16 MB).
  - `DELETE /v1/orgs/:org_id/agents/:id/files/:file_id`.

#### Aba 5: Roteamento & Handoff Humano (`#routing` — HR-43)
- **Objetivo:** Configuração de encaminhamento humano baseado nas decisões do Jev (Gates G1 e G4).
- **Seção 1 — Departamento Padrão:** Select do departamento padrão da org onde a conversa se inicia.
- **Seção 2 — Atendimento Humano (Gates G1/G4):**
  - Toggle de ativação do handoff humano.
  - Select do Departamento Humano de Destino (`v1_agents.human_handoff_department_id`).
  - Aviso de texto prévio de orientação de risco (ingestão de produto, peçonhento, emergência à saúde).
- **Seção 3 — Motivos de Transferência (`jev_json.situacoes_de_handoff`):**
  - Lista de até 15 motivos de texto livre (máx. 300 caracteres cada) enviados diretamente ao prompt do Jev G1.
  - *Onde é gravado:* no JSON do Jev do agente (`agents.jev_json.situacoes_de_handoff`), que é o que o motor lê (`poph_jev::situacoes_do_agente`, chamada no turno). `scope.handoff.reasons` foi o endereço no POP Atende V1; no Harness gravar lá não muda o atendimento. As outras chaves do `jev_json` (despedida, gates) são preservadas na gravação.
- **Registro Explícito da Decisão D-06:** Card informativo destacando que regras de rota por intenção (palavras-chave/regex) foram descontinuadas na V1.

#### Aba 6: Integrações (`#integrations` — HR-44)
- **Objetivo:** Gestão da conexão com WhatsApp (Evolution API) e configuração de plugins externos via manifesto dinâmico.
- **WhatsApp (Evolution API):**
  - Painel de status: Badge verde `CONECTADO`, número de telefone E.164, nome da instância, latência de webhook.
  - Botões de Ação: `Pausar Atendimento` (suspende respostas automáticas), `Desconectar Instância` (com confirmação) e `Ver Logs Webhook`.
  - Conexão Nova: Modal com QR Code gerado pela Evolution (data URL), rodada do código (`count`) e código de pareamento por dígitos alternativo. *A Evolution v2 não informa validade do QR* (`expira_em` é sempre nulo): não há contador regressivo; a tela avisa que o código gira sozinho (~20 s por rodada) e oferece "Gerar outro QR".
- **Plugins Externos (HR-25 / HR-44):**
  - Renderização dinâmica de formulário a partir de JSON Schema fornecido pelo manifesto do plugin (provado com o plugin de exemplo `poph-plugin-echo`).
  - Mascaramento obrigatório de credenciais e chaves secretas (`type="password"`).

#### Aba 7: Modelo & Jev (`#model-jev` — HR-45)
- **Objetivo:** Parametrização fina do runtime de IA, provedor LLM e limiares matemáticos dos 8 Gates do Jev.
- **Parâmetros do LLM e RAG:**
  - Provedor (antigravity2api local), Modelo LLM, Temperatura (slider 0.0 a 2.0 com valor numérico), Janela de agrupamento de mensagens (batching em segundos, padrão 2.0s), Contexto máximo (tokens).
  - Parâmetros RAG: Top-k de recuperação (padrão 3) e Corte mínimo de relevância (padrão 0.70).
- **Tabela dos 8 Gates do Jev (G1–G8):**
  - Tabela estruturada com: `Gate ID`, `Função no Turno`, `Status (Toggle)`, `Limiar de Confiança (0.00–1.00)`, `Margem Mínima (0.00–1.00)` e `Obrigatoriedade V1`.
  - Explicação concisa e valores padrão visíveis para cada gate:
    - *G1:* Transferir para humano e risco de saúde (0.60 / 0.10)
    - *G2:* Esperar mais ou processar agora (0.70 / 0.10)
    - *G3:* Precisa de base, produtos ou nada / saudação (0.75 / 0.10)
    - *G4:* Qual departamento recebe a transferência (0.65 / 0.10)
    - *G5:* Resolução de produto ambíguo (0.70 / 0.10)
    - *G6:* Auditoria da resposta pré-envio (0.80 / 0.10)
    - *G7:* Anti-injeção de prompt e jailbreak (0.75 / 0.10)
    - *G8:* Fechamento de conversa sem gastar LLM (0.80 / 0.10)

---

### 3.6 Chat de Teste Completo (`/orgs/:org_id/agents/:agent_id/test-chat` — HR-46)
- **Objetivo:** Ambiente de homologação e teste funcional interativo do agente, reproduzindo com fidelidade a experiência do WhatsApp com ferramentas de depuração em tempo real.
- **Barra Lateral de Sessões (280 px):**
  - Botão destacado no topo: `+ Nova Conversa (Memória Zerada)` (`POST /v1/orgs/:org_id/agents/:id/test-chat`).
  - Lista de conversas anteriores com ID, prévia da última mensagem, timestamp e badge de status (`Ativa` ou `Transferida`).
- **Área Central de Conversação:**
  - **Balões de Mensagem:** Cliente (azul escuro à direita), Agente (cinza escuro à esquerda).
  - **Streaming SSE em Tempo Real:**
    - Exibição de deltas de texto token a token com cursor piscante `▌`.
    - Indicador de chamada de ferramentas em execução: `⚙ Executando ferramenta: buscar_produtos(alinhador)...`.
  - **Botão de Cancelamento de Turno:** Botão vermelho visível durante o streaming `⏹ Cancelar Turno em Andamento`. O clique aborta a conexão SSE e envia sinal de cancelamento ao runtime axum, liberando a fila sem prender recursos.
  - **Anexos e Mídia:** Suporte a envio de imagens (processadas multimodais pelo LLM) e documentos PDF/Office (com extração isolada de texto). Arquivos enviados pelo agente aparecem como cards de download com visualização prévia.
  - **Estado de Conversa Transferida:**
    - Quando o Jev G1 aciona o handoff humano, a área de input é bloqueada e substituída pelo banner âmbar de transferência:
      - Destino do handoff (`Suporte Clínico & Emergência`).
      - Motivo detectado pelo Jev e nível de confiança (ex: `Dor aguda intensa / sangramento bucal — 94.2%`).
      - Botão de ação: `Reabrir Atendimento da IA nesta Conversa` (`POST /reopen`), que destrava a conversa e permite ao agente voltar a responder.
  - **Botão de Acesso à Telemetria:** Em cada turno do agente, um botão sutil `Ver Por Quê (Turno #N) ↗` abre o Drawer lateral de diagnóstico.

---

### 3.7 Painel Lateral do "Por Quê" por Turno (Drawer — HR-47)
- **Objetivo:** Explicabilidade matemática e operacional completa de cada turno do agente. Abre deslizando da direita sobre o chat de teste sem perder o contexto da conversa.
- **Cabeçalho de Telemetria:**
  - Identificador do turno (`Turno #1042-3`), timestamp ISO 8601, Duração total da resposta em milissegundos (`1.240 ms`), Custo financeiro estimado (`$0.0018 / R$ 0,01`) e Total de tokens consumidos (`1.960 tokens`).
- **Seção 1 — Decisões dos Gates do Jev:**
  - Cartões individuais para cada gate invocado no turno, exibindo: Decisão/Escolha, Confiança percentual, Margem de vitória em relação à segunda opção, Probabilidades e Latência em ms.
- **Seção 2 — Contexto RAG & Catálogo Recuperados:**
  - Lista de trechos e produtos retornados para compor o contexto do turno, com nome do item/produto, link para a base oficial, score de similaridade e indicação de corte.
- **Seção 3 — Ferramentas Invocadas (Tool Calls):**
  - Nome da ferramenta executada, argumentos passados em JSON formatado, payload de retorno recebido e tempo de execução em milissegundos.
- **Seção 4 — Linha do Tempo da Latência (Waterfall):**
  - Gráfico horizontal detalhando os tempos de cada etapa consecutiva:
    1. Gating do Jev (Gates concorrentes G1/G2/G3): ~275 ms
    2. Busca Vetorial Híbrida RAG: ~42 ms
    3. LLM Time-to-First-Token (TTFT): ~480 ms
    4. Streaming de Saída Completo: ~443 ms
- **Seção 5 — Telemetria de Tokens:**
  - Detalhamento de Tokens de Prompt, Tokens de Conclusão e cálculo de custo por modelo.

---

### 3.8 Painel Operacional da Instância (`/operation` — HR-48)
- **Objetivo:** Observabilidade, governança de nós do cluster, supervisão de plugins e ferramentas de migração/backup.
- **Aba Saúde da Instância (`#health`):**
  - Cards de métricas em tempo real (`/metrics` e `poph doctor`):
    - *Memória RSS do Processo:* Medição em MB contra a Meta M3 (≤ 150 MB com 10 mil agentes ociosos).
    - *Filas de Mensagens:* Quantidade de mensagens pendentes em `inbox` e `outbox`.
    - *Latência Interna:* Percentis p50 e p95 em milissegundos contra a Meta M7 (≤ 2 ms p50 / ≤ 5 ms p95).
    - *Agentes Carregados:* Total de agentes alocados em memória RAM.
- **Aba Topologia do Cluster (`#cluster`):**
  - Visualização da configuração estática de `cluster.toml` (D12).
  - Tabela com todas as instâncias pares: ID, URL HTTP, Organizações mapeadas, Status de conectividade (verde se acessível, vermelho se timeout) e Latência de rede entre nós.
- **Aba Plugins Externos (`#plugins`):**
  - Tabela do supervisor de processos de plugins (HR-25): Nome do binário, PID do sistema operacional, Status (`EM EXECUÇÃO`, `PARADO`), Contador de reinícios automáticos e Consumo de RSS em MB.
- **Aba Migração & Backup (`#migration`):**
  - *Importador do POP Atende (HR-52):* Formulário com URL e token de leitura do POP Atende, botão de execução de **Dry-Run de Paridade** com relatório prévio de compatibilidade (campos importáveis vs campos descartados conforme D-06) e botão de importação idempotente.
  - *Exportação de Organização (HR-53):* Botão para gerar pacote de backup completo `.poph-org` (banco SQLite cifrado, blobs e índices) e restaurador em instâncias limpas.

---

## 4. Modais Globais e Mecanismos Transversais de UX


### 4.1 Modal de Alterações Não Salvas (`UnsavedChangesModal`)
- **Gatilho:** Disparado pelo hook de navegação do React Router sempre que o operador tenta trocar de aba, selecionar outro agente ou sair da tela com formulários em estado "dirty".
- **Comportamento:** Bloqueia a transição e exibe diálogo centralizado:
  - Título: "Alterações Não Salvas"
  - Resumo dos campos modificados recentemente.
  - Ações:
    - `Descartar Alterações`: Limpa o rascunho local e prossegue com a navegação.
    - `Cancelar Navegação`: Fecha o modal e mantém o operador exatamente no formulário onde estava.
    - `Salvar e Sair`: Executa o salvamento via API com `If-Match` e, após sucesso, navega para o destino.

### 4.2 Modal de Confirmação de Ação Destrutiva (`DestructiveConfirmModal`)
- **Gatilho:** Exclusão de organizações, exclusão de agentes, remoção de arquivos da biblioteca ou desvinculação permanente de instâncias WhatsApp.
- **Mecanismo de Alta Fricção:** Para prevenir acidentes operacionais, o botão vermelho de confirmação permanece estritamente desabilitado até que o operador digite exatamente o slug ou identificador do recurso (ex: digitar `dra-beatriz`).
- **Teclado:** `Esc` cancela a operação e fecha o modal.

### 4.3 Tratamento de Erros Globais e Skeletons
- **Zero Cumulative Layout Shift (CLS):** Enquanto os dados de agentes, tabelas e métricas são carregados via API, a interface exibe skeletons (`Shimmer`) que reproduzem exatamente a grade, altura e espaçamento das linhas finais, prevenindo saltos visuais na renderização.
- **Erro Crítico de Armazenamento (500):** Em caso de falha de banco SQLite ou erro interno do axum, a tela renderiza card técnico detalhado com o stacktrace exato, caminho do arquivo afetado e sugestões diretas de ação de recuperação (ex: executar `poph doctor` ou restaurar backup).

---

## 5. Diretrizes de Acessibilidade e Navegação por Teclado

O Console é uma ferramenta de trabalho diário prolongado e deve atender integralmente ao padrão **WCAG 2.1 nível AA**:

1. **Foco Visível e Contraste:** Todos os elementos focáveis (botões, inputs, abas, links de tabela) possuem contorno de foco de 2 px em `--pop-accent` (`#F5A623`) ou `--pop-blue` (`#3B82F6`) com deslocamento (offset) de 2 px, garantindo contraste mínimo de 3:1 sobre superfícies escuras.
2. **Navegação Integral por Teclado:**
   - `Tab` e `Shift+Tab`: Navegação sequencial lógica por todos os controles interativos.
   - `Enter` / `Space`: Ativação de botões, checkboxes e expansão de itens.
   - `Esc`: Fecha imediatamente modais, drawers e menus dropdown.
   - `Ctrl+S` / `Cmd+S`: Atalho universal para acionar o botão "Salvar" na aba ativa com formulário dirty.
   - `Alt+1` a `Alt+7`: Alternância rápida entre as 7 abas do agente.
   - `Alt+N`: Atalho para criar nova conversa no Chat de Teste.
   - `?`: Abre modal com resumo de todos os atalhos de teclado do Console.
3. **Landmarks ARIA e Leitores de Tela:**
   - Sidebar marcada com `role="navigation"` e `aria-label="Navegação Principal"`.
   - Área central marcada com `role="main"`.
   - Drawer do Por Quê marcado com `role="complementary"` e `aria-label="Diagnóstico do Turno"`.
   - Streaming do chat marcado com `aria-live="polite"` para atualização acessível de novos deltas de mensagem sem poluir a fala.

---

## 6. Mapeamento de Rotas da API e Tratamento de Falhas (HR-20 a HR-53)

| Tela / Recurso | Rota HTTP da API (axum) | Método | Cabeçalhos / Payload | Resposta de Sucesso | Tratamento de Falha na UI |
|---|---|---|---|---|---|
| **Login** | `/v1/auth/verify` | `POST` | `{ token }` | `200 OK` `{ valid, scope, org_id }` | Inline error em vermelho destacando token revogado ou inválido |
| **Orgs** | `/v1/orgs` | `GET` | `?cluster=true` | `200 OK` `[{ id, name, slug, ... }]` | Banner 503 com nó inacessível marcado e botão de retry |
| **Nova Org** | `/v1/orgs` | `POST` | `{ name, slug, retention_days }` | `201 Created` `{ org }` | Erro 409 se o slug já existir; realce no campo |
| **Categorias** | `/v1/orgs/:id/product-categories` | `GET / POST` | `{ name, slug }` | `200 OK / 201 Created` | Erro inline no modal de criação |
| **Departamentos** | `/v1/orgs/:id/departments` | `GET / POST` | `{ name, description }` | `200 OK / 201 Created` | Erro inline no modal de criação |
| **Agentes** | `/v1/orgs/:id/agents` | `GET` | — | `200 OK` `[{ agent }]` | Skeletons de carregamento; card de erro com retry |
| **Instruções** | `/v1/orgs/:id/agents/:ag_id/instructions` | `GET / PATCH` | `If-Match: "{v}"`, `{ structured }` | `200 OK` `{ markdown, version }` | `412 Precondition Failed` abre `VersionConflictModal` |
| **Base RAG** | `/v1/orgs/:id/knowledge-bases` | `GET` | — | `200 OK` `[{ base, items }]` | Exibe estado vazio ou erro com botão de recarga |
| **Reindexar** | `/v1/orgs/:id/agents/:ag_id/reindex` | `POST` | — | `202 Accepted` `{ status: "indexing" }` | Badge de indexação em andamento com polling a cada 2s |
| **Produtos** | `/v1/orgs/:id/products` | `GET / POST` | `{ sku, name, price, ... }` | `200 OK / 201 Created` | Tabela com filtro; erro 400 se faltar preço oficial |
| **Arquivos** | `/v1/orgs/:id/agents/:ag_id/files` | `POST` | `multipart/form-data` | `201 Created` `{ file_id, hash, chars }` | Rejeição imediata se > 16 MB ou extensão bloqueada |
| **Roteamento** | `/v1/orgs/:id/agents/:ag_id/routing` | `GET / PUT` | `If-Match: "{v}"`, `{ department_id, ... }` | `200 OK` `{ config, version }` | Erro 412 se versão concorrente for alterada |
| **WhatsApp QR** | `/v1/orgs/:id/integrations/whatsapp/qrcode` | `GET` | — | `200 OK` `{ qrcode_base64, timeout }` | Polling a cada 5s; aviso se a instância não subir |
| **Plugins** | `/v1/plugins` | `GET / POST` | Config do plugin | `200 OK` `[{ pid, status, rss }]` | Supervisor marca processo parado em cinza/vermelho |
| **Chat Iniciar** | `/v1/orgs/:id/agents/:ag_id/test-chat` | `POST` | `{ reset_memory: true }` | `201 Created` `{ session_id }` | Cria sessão zerada na lista lateral |
| **Chat SSE** | `/v1/orgs/:id/agents/:ag_id/test-chat/events` | `GET` | `session_id` | `text/event-stream` (Deltas, tools, fim) | Desconexão cancela turno; timeout exibe badge de erro |
| **Chat Reabrir**| `/v1/orgs/:id/test-chat/:sess_id/reopen` | `POST` | — | `200 OK` `{ status: "active" }` | Destrava a conversa transferida para novas mensagens |
| **Telemetria** | `/v1/orgs/:id/turns/:turn_id/telemetry` | `GET` | — | `200 OK` `{ jev, rag, tools, latency }` | Drawer exibe skeletons enquanto busca detalhes do turno |
| **Saúde/Métricas**| `/metrics` e `/v1/health` | `GET` | — | `200 OK` `{ rss, queues, p50, p95 }` | Atualização em tempo real com indicador visual |
| **Importar POP**| `/v1/migration/import-popatende` | `POST` | `{ url, token, agent_id, dry_run }` | `200 OK` `{ report, parity }` | Relatório de compatibilidade detalhado antes de gravar |

---

## 7. Catálogo de Telas e Evidências Exportadas

Os frames de alta fidelidade do desenho original não fazem parte deste repositório.


---

## 8. Critério de Aceite e Handoff para Engenharia

Esta especificação cumpre integralmente os requisitos da tarefa HR-37 e destrava o desenvolvimento das tarefas subsequentes:
