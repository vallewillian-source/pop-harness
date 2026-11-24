# Especificação de UX/UI — Console Web Administrativo do POP Harness (Rust)

> **Documento:** `console/UX_SPEC.md`  
> **Tarefa:** HR-37  
> **Base de Requisitos:** Visão geral `docs/VISAO_GERAL.md` (o antigo PRD §10 e §13 está em `docs/JEV.md` e `docs/CONSOLE.md`), Decisões `DECISOES.md` (D-03 a D-10) e Console `docs/CONSOLE.md`  
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
1. **Sidebar Retrátil (240 px; 64 px no modo foco):**
   - **Topo do rail (CF-06, rodada 1):** o topo depende da tela, como no desenho — nunca os dois.
     - **Na lista (`/orgs`, prancha 01): o chip de identidade do cluster.** Caixa de 32 px de altura, fundo `--pop-surface-glass`, borda de 1 px, glifo de servidor de 14 px em `--pop-text-muted` (o mesmo desenho do FigCraft; a SofaUI não tem esse glifo), nome "POP Harness Cluster" em 12,5 px Medium e o ponto de situação à direita. O ponto é verde com a instância respondendo, vermelho quando `/healthz` responde degradada (503) ou não responde, e neutro enquanto a saúde não chegou; a situação também vai no `title` e num texto só para leitor de tela ("POP Harness Cluster · instância respondendo"). No modo foco vira um tile de 32×32 com o nome no `title`; abaixo de 900 px também.
     - **Com uma organização aberta (`/orgs/:orgId/...`, pranchas 02 e 05): o seletor de organização (combobox).** Caixa de 32 px com fundo `--pop-surface-glass`, borda de 1 px, ícone de prédio âmbar, o nome da organização aberta e as setas para cima e para baixo. O texto muda conforme a URL: "Selecionar organização" fora de uma organização e "Nenhuma organização" sem nenhuma.
     - Ao abrir, o popover (no `body`, fixo, ao lado do rail no modo foco) põe o foco no campo "Filtrar organizações…", que filtra por nome ou slug sem diferenciar acento nem caixa. A lista traz "Todas as organizações" (só com token global e sem filtro) e as organizações que o token enxerga, cada uma com nome, slug, ponto de situação (ativa em verde, inativa em cinza) e check na atual.
     - Teclado: seta para baixo abre o popover ou move a opção; setas, Home e End percorrem a lista; Enter escolhe; Esc fecha e devolve o foco ao gatilho. Clicar fora fecha sem devolver o foco. Escolher passa pela guarda de alterações (mesmo caminho do clique na linha da lista); escolher a organização que já está aberta só fecha.
     - Acessibilidade: o gatilho é `role="combobox"` com `aria-haspopup="listbox"`, `aria-expanded` e `aria-controls`; a lista é `role="listbox"` e o campo usa `aria-activedescendant`. Não usa o `SofaSelect` do formulário, porque a SofaUI não tem combobox de navegação com popover.
     - Token de organização: o gatilho mostra a própria organização, fica desabilitado e não abre a lista.
     - Modo foco: o gatilho vira um tile de 32×32 só com o ícone, com `title` e `aria-label` com o nome da organização. Abaixo de 900 px o gatilho também vira o tile.
     - **Na visão do agente (`/orgs/:orgId/agents/:agentId/...`, prancha 03, CF-07): o link de volta e o combobox do agente**, no lugar do seletor de organização.
       - **Link de volta:** "← Agentes da Org" em 22 px de altura, seta de 12 px e texto de 11 px em `--pop-text-muted`, com recuo de 4 px. Vai para `/orgs/:orgId/agents` e passa pela guarda de alterações como qualquer link interno. No modo foco fica só a seta, com `aria-label` e `title` "Agentes da Org".
       - **Combobox do agente:** caixa de 36 px (medida do desenho), fundo `--pop-surface-glass`, borda de 1 px que fica âmbar com o popover aberto, ícone de bot de 14 px em âmbar, nome do agente em 12,5 px Medium e subtítulo de 10,5 px em tom mudo com **modelo e situação reais** ("gemini-3.8-flash · Ativo"; sem modelo próprio, "modelo padrão do servidor · Ativo"). As setas up/down ficam à direita.
       - Ao abrir, o popover (no `body`, fixo, ao lado do rail no modo foco) põe o foco no campo "Filtrar agentes…", que filtra por nome ou slug sem diferenciar acento nem caixa. A lista traz os agentes da organização com nome e o mesmo subtítulo de modelo e situação, e check no agente aberto.
       - Estados: "Carregando agentes…", "Nenhum agente nesta organização", "Nenhum agente encontrado para “termo”" e, em falha, "Não foi possível ler os agentes: motivo" — o resto do menu continua funcionando. Toda leitura termina em pronto ou erro, inclusive por prazo de 12 s.
       - Escolher navega para `/orgs/:orgId/agents/:novoId/instructions` pela guarda de alterações; o agente que já está aberto só fecha o popover. Teclado e foco: os mesmos do seletor de organização (seta abre, setas/Home/End percorrem, Enter escolhe, Esc fecha e devolve o foco ao gatilho).
       - Dados: **uma** consulta por organização (`GET …/agents`), guardada enquanto o rail viver; um agente que não esteja nessa lista é lido sozinho (`GET …/agents/{id}`) e, se não existir mais, o gatilho mostra "Agente não encontrado".
     - A versão da instância continua no rodapé (o desenho também a mostra no chip do cabeçalho).
   - **Navegação Agrupada:** os três grupos de `navegacao.ts` — *TENANTS E RECURSOS*, *RUNTIME E MOTOR* e *SISTEMA*. O item ativo usa o degrau tonal da SofaUI (fundo elevado, sem borda lateral colorida); o contador à direita é um chip de borda fina.
   - **Cockpit do agente (CF-09, prancha 03):** nas rotas `/orgs/:orgId/agents/:agentId/...` o menu deixa de ser a navegação da organização e passa a ser o cockpit vertical do agente, na ordem do desenho:
     - primeiro bloco, **sem título de seção**: *Instruções & Prompts*, *Base de Conhecimento*, *Arquivos do Agente*, *Roteamento & Habilidades*, *Modelo & Parâmetros (Jev)*;
     - seção **Diagnóstico & Teste** (caixa de frase, 10,5 px Medium, sem o caixa-alta e o espaçamento das outras seções — é o que a prancha 03 desenha): *Chat de Teste* e *Inspeção de Turnos*;
     - cada item ativo aponta para a aba real do agente em `App.tsx` (`instructions`, `knowledge-base`, `files`, `routing`, `model-jev`, `test-chat`) e passa pela guarda de alterações. *Inspeção de Turnos* ainda não tem tela: fica `emBreve`, desabilitado e sem navegar — nada de página inventada;
     - os itens do cockpit têm rótulo de 12,5 px (o resto do rail segue em 12 px), como no desenho. Os glifos vêm da SofaUI: `IconFile` (documento), `IconDatabase` (livro), `IconFolder` (arquivos), `IconWorkflow` (fluxo), `IconSliders` (cpu/parâmetros), `IconMessage` (chat) e `IconList` (terminal);
     - *Integrações* (`/integrations`) não aparece no desenho e continua alcançável pelas abas de dentro das telas do agente, que este menu não substitui;
     - fora das rotas de agente o rail não muda: os grupos e itens de `navegacao.ts` continuam iguais (eles também alimentam a busca global do cabeçalho).
   - **Rodapé:** versão da instância à esquerda, botão `?` dos atalhos e, à direita, o botão que recolhe o menu. Na visão do agente (CF-09) a versão dá lugar a **"Agente: <nome>"**, como na prancha 03 — o nome vem da mesma leitura do combobox do agente, sem consulta nova, e corta com reticências quando é longo. O botão `?` fica (é o único caminho visível para os atalhos); a versão continua visível no chip do cabeçalho.
   - **Modo foco (prancha 04 do desenho):** rail de 64 px com ícones de 34×34 px, o botão de expandir no topo, o chip do cluster (na lista) ou o seletor de organização (com organização aberta) reduzido ao tile de 32×32 e os atalhos no rodapé. Na visão do agente, o link de volta vira a seta e o combobox vira um tile de 32×32 com o ícone de bot (CF-07). O rótulo de cada item continua no DOM (escondido por recorte, para leitores de tela) e volta como `title` no hover — inclusive nos itens "em breve" do cockpit, que no modo expandido não têm `title`. No modo foco saem o título de seção "Diagnóstico & Teste" e o "Agente: <nome>" do rodapé, como na prancha 04 (que não tem texto no rail recolhido). O foco de teclado acompanha a troca de estado.
2. **Topbar Global (56 px altura):**
   - Borda inferior de 1 px em âmbar (`--pop-accent`, `#F5A623`) atravessando a tela toda; fundo `--pop-surface-raised`.
   - Três colunas: à esquerda, monograma `P`, nome da marca, chip de versão da instância (`v0.1.0`, do `/healthz`) e a trilha de navegação (breadcrumbs clicáveis, ex.: `Organizações / Exemplo / Agentes / Dra. Beatriz`); no centro, a busca global; à direita, estado, escopo do token, saída e ajuda.
   - **Largura das colunas (CF-08, medidas em 1440 px):** a esquerda e a direita têm a largura do próprio conteúdo e a busca fica com até 420 px entre elas, no mínimo 220 px (`clamp(220px, 35vw, 420px)`); o espaço que sobra é dividido em partes iguais dos dois lados da busca (`justify-content: space-between`), como no desenho ("hug" nas pontas, busca "fill" com teto). Em 1440 px a busca mede 420 px, em 1280 px 420 px e em 1100 px 385 px. O que não couber é cortado dentro da própria coluna, nunca por cima da busca.
   - **Trilha de navegação (CF-08):** cada migalha é um passo com o separador dentro. A migalha da tela atual (a última) **nunca encolhe**: aparece inteira, com `aria-current="page"` e sem `title`. As anteriores encolhem por prioridade — quanto mais longe da atual, mais cedo cortam com reticências (`flex-shrink` = distância até a atual) — e cada uma leva o texto completo no `title`. Em rota de agente (cinco migalhas: organização / Agentes / agente / aba) as cinco cabem inteiras em 1440 px; com nomes muito longos, as anteriores cortam e a atual continua inteira.
   - **Busca global** (no centro, até 420 px): um só campo para organizações, telas e comandos do `poph`, com o atalho `⌘K` (ou `Ctrl K`) visível dentro do campo. O resultado sai em grupos, nesta ordem: *Organizações* (nome ou slug, como antes), *Telas* (as entradas do menu lateral que existem para a URL atual: uma organização ou um agente só aparecem quando estão abertos) e *Comandos do poph* (só os comandos conferidos no binário). Setas percorrem todos os grupos; Enter abre a organização ou a tela pela guarda de alterações, ou copia o comando para a área de transferência com aviso curto; Esc limpa e fecha. Com nada encontrado: "Nada encontrado para “termo”".
   - **Estado da instância** (texto mono): `Engine: 200 OK · 125 MB` com a situação de `/healthz` e a memória residente de `/metrics`, consultados a cada 15 s. Sem a memória, mostra só `Engine: 200 OK`; com `/healthz` respondendo 503 (um banco fora), mostra `Engine: 503 · degradada` em vermelho; sem resposta, `Sem conexão`. Nunca mostra "0 MB" no lugar de um dado que não veio.
   - O endereço da instância (`Local :porta`) e o escopo do token ficam na dica (`title`) do indicador de estado; o escopo do token segue visível como selo ao lado do estado.
   - **Larguras menores:** até 1320 px a trilha mostra as duas últimas migalhas (agente e aba, ou organização e tela), com o separador do primeiro passo visível escondido para não começar com "/"; até 1200 px mostra só a migalha da tela atual e os espaços da direita encolhem para o indicador não ser cortado; até 1020 px a trilha sai inteira, com o divisor, porque nem a migalha da tela atual cabe ao lado da marca; até 900 px saem também a versão, o nome da marca e o selo do token, e a busca ocupa o espaço que sobra. Abaixo de 520 px o texto do estado some e fica só o ponto de situação.

3. **Workspace Central (Largura fluida, mín. 1200 px):**
   - Contêiner denso com rolagem independente, evitando rolagem dupla na página.

Abaixo de 900 px o rail entra no modo foco pelo layout e o alternador manual sai de cena: a largura não comporta os dois estados.

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
Layout da prancha 01 do FigCraft (`design/pop-harness-console-v2.figcraft`), CF-03b.

- **Objetivo:** ver as organizações da instância e do cluster, com o nó de cada uma e os agentes ativos, e gerenciá-las (criar, editar, reativar, exportar, excluir).
- **Cabeçalho:** título "Organizações" com a contagem ao lado; campo de busca por nome ou slug; botão "Atualizar" (só ícone); "Nova organização" (primário âmbar, só token global).
- **Barra:** abas Todas / Ativas / Inativas (e Arquivadas, quando houver), cada uma com a contagem, num trilho segmentado. À direita, a telemetria do nó local: nós conectados, uptime e RAM (de `/healthz` e `/metrics`). Um dado que não chega some da linha.
- **Tabela (`SofaTable`):** caixa de 1 px, cabeçalho de 34 px, linhas de 52 px. Colunas: Organização / slug (com ícone), Situação, Nó / instância, Agentes ativos, Retenção, Atualizada (data e versão) e Ações.
  - Linha com o inspetor aberto: fundo tonal e nome em âmbar (sem barra lateral colorida).
  - Ações: "Acessar →" (em organização inativa, "Reativar", só token global) e o menu ⋮, sempre visível: Ver detalhes, Editar, Exportar, e para inativa também Acessar e Reativar; Excluir por último.
  - Organização que mora em outro nó: o nome não é link, "Acessar" fica desabilitado e a tela explica o nó.
- **Nó / instância:** vem de `instancia` (cada organização) e de `instancias` (situação de cada nó). Instância sem id configurado aparece como "nó local". Sem `instancias` (token de org), a coluna mostra "—".
- **Agentes ativos:** vem de `agents_active_count`. Enquanto a instância não informa o campo, mostra "—" com explicação no `title`.
- **Responsivo:** as colunas secundárias saem conforme a largura da área da lista, que encolhe com a gaveta aberta. Retenção sai abaixo de 1000 px; Nó / instância abaixo de 920 px; Atualizada abaixo de 760 px. Os cards vão de 3 colunas para 2 abaixo de 1000 px e para 1 abaixo de 640 px. Abaixo de 1100 px, a gaveta do inspetor vai para baixo da lista.
- **Cards de operação:** Espelho Matrix Synapse (mostra "indisponível": o estado do Matrix é por organização, e a lista do cluster não traz esse dado); Isolamento Multi-Tenant (texto explicativo); Comandos CLI Úteis (`poph org list`, `poph org create --name "nome"`, `poph doctor`).
- **Dados da API:** `GET /admin/v1/orgs?cluster=true` (uma chamada, sem consulta por linha), `GET /healthz` e `GET /metrics`. O filtro por aba e texto é local.
- **Diferenças do desenho, por decisão:** a API não devolve o endereço do nó, então a telemetria não mostra endereço; não há nomes dos agentes ativos (a API devolve só a contagem); a busca não tem o atalho `/`; o rodapé não tem as setas ↑↓ nem ⌘↵ (o atalho de teclado que existe é Tab e Enter); "Criada em" saiu da tabela e continua no inspetor; a contagem do título é neutra, como na prancha, e não âmbar.
- **Estados:**
  - *Carregando:* 5 linhas de esqueleto com a altura da linha.
  - *Vazio:* "Nenhuma organização cadastrada" com CTA "Criar primeira organização" (só token global).
  - *Erro:* "Não consegui carregar as organizações" com "Tentar de novo".
  - *Filtro sem resultado:* "Nenhuma organização encontrada" com "Limpar filtros".

---

### 3.2.1 Inspetor lateral da organização (gaveta da prancha 01b)
Gaveta de 380 px à direita da lista, aberta ao selecionar a linha (`InspetorOrg`), CF-05.

- **Objetivo:** inspecionar uma organização sem sair da lista: identidade, situação, nó, estado do espelho Matrix, agentes vinculados e as ações de gestão.
- **Cabeçalho:** tile de 26 px com o ícone de prédio (contorno âmbar), nome em 15 px, botão ✕ (nome acessível "Fechar inspetor") e, abaixo, em mono de 11 px, `slug: <slug> · versão vN (ETag/If-Match)` — a versão é a do `If-Match` das alterações.
- **Badges:** situação em pílula com ponto (mesmos tons da lista), chip do nó (`Nó primário` quando a organização está no nó desta instância, `Nó <id>` quando está em vizinho, `nó local` na instalação de nó único) e chip do id abreviado em 8 caracteres com o id inteiro no `title`.
- **Caixa de comando:** `poph chat --org <slug> <slug-do-primeiro-agente-ativo>` (ou `<agente>` quando não há agente ativo), em âmbar, quebrando linha, com botão de copiar e anúncio "Copiado" em `aria-live`.
- **Propriedades (`dl`, rótulo à esquerda e valor à direita, sem fio entre as linhas):** Retenção de histórico (`config.retention_days`, "padrão" quando ausente); Espelho Matrix; Instância alocada; Atualizada em; Criada em — as datas completas em pt-BR (`dd/mm/aaaa às hh:mm`).
- **Espelho Matrix:** de `GET /admin/v1/orgs/{org}/matrix`. `Não configurado` quando `connected` é `null` (módulo não configurado na instância), `Desligado nesta organização` quando `enabled` é `false`, `Desconectado` quando `connected` é `false` e `Conectado` quando é `true`. O `title` traz o atraso, as falhas isoladas e a última falha. Falha da rota vira `indisponível` com o motivo no `title` — nunca um estado inventado.
- **Agentes vinculados (N):** cards com nome, modelo (ou "modelo padrão do servidor") e situação, vindos da mesma consulta única de `GET …/agents` que a gaveta já fazia; cada card é link para `/orgs/:id/agents/:agentId`. Até 8 cards e, acima disso, o link "Ver todos os N agentes →". Lista vazia diz "Nenhum agente cadastrado nesta organização."; falha aparece só no bloco, com o motivo no `title`, sem derrubar a gaveta.
- **Terminação das leituras:** cada recurso (agentes e Matrix) carrega sozinho e sempre termina em pronto ou erro — inclusive por prazo de 12 s —, então a gaveta nunca fica em "carregando…".
- **Rodapé fixo:** "Acessar Organização →" primário em âmbar na largura toda, mais Editar, Exportar e Excluir (ícone de lixeira, `aria-label` "Excluir <nome>"). Editar abre o `ModalOrg`, Exportar usa `exportarOrganizacao` + `baixarArquivo` sem conversas, Excluir abre o `DialogoExclusao` com confirmação por slug. Com token de organização o rodapé mostra só "Acessar Organização →". Organização em outro nó desabilita Acessar e Editar.
- **Teclado e foco:** o foco entra na gaveta ao abrir e volta à linha ao fechar (o `Orgs` guarda a linha de origem); `Esc` fecha, menos quando um menu ou diálogo aberto por cima consome a tecla primeiro.
- **Diferenças do desenho, por decisão:**
  - a rota de estado do Matrix não traz o espaço/canal, então o valor é "Conectado" sem `(#canal)`;
  - não há latência do agente (o card mostra modelo e situação, que existem) nem "nó primário do agente";
  - o endereço `host:porta` do nó não existe na resposta: a propriedade e o chip usam o id da instância, e `nó local` na instalação de nó único;
  - datas completas (`dd/mm/aaaa às hh:mm`) em vez de "Hoje às 13:06", como pede a leitura técnica;
  - o rodapé não repete a dica "Esc fecha o inspetor" (ela fica na tabela);
  - componentes da SofaUI mantêm as medidas da biblioteca: os secundários têm 32 px (o desenho pede 30) e rótulo SemiBold, o botão de copiar tem 24 px (o desenho pede 22, e o `SofaIconButton` só tem 24/28/32/36) e os valores das propriedades usam o peso 500 real da IBM Plex (no render do FigCraft o Medium é sintetizado, e sai mais leve).

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
