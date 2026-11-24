# Linha de Base Visual do Console (RF-08)

Esta pasta contém a linha de base visual das telas principais do Console do POP Harness, capturadas antes do refactor de internacionalização (código em inglês e remoção de comentários).

O objetivo desta linha de base é servir como referência visual comparativa para garantir paridade exata de layout, tipografia, espaçamentos e componentes durante e após as renomeações e refatores do frontend.

> **Importante — Linha de Base Provisória:**
> O conjunto atual de capturas em `design_assets/20261123-refactor-baseline/` é **provisório**. O layout do Console está atualmente em evolução ativa (tarefas CF-09 em andamento e CF-10 pendente). A captura definitiva da linha de base para portão de CI será gerada após a conclusão e aceite do layout final (CF-10).

---

## 1. Telas Capturadas

Todas as capturas foram geradas em PNG e acompanhadas do respectivo snapshot do DOM HTML:

| Arquivo | Tela / Rota | Estado Capturado |
|---|---|---|
| `01-login.png` | Login (`/login`) | Estado inicial limpo: campo de token administrativo, orientações de uso em memória e botão de submissão. |
| `02-orgs-lista.png` | Organizações (`/orgs`) | Tabela de organizações com dados de demonstração, abas de filtro ("Todas", "Ativas", "Inativas"), busca global e status de cluster. |
| `03-orgs-inspetor.png` | Inspetor de Organização (`/orgs`) | Gaveta lateral do inspetor aberta sobre a organização selecionada, exibindo metadados, contagem de agentes ativos, status de nó e ações. |
| `04-agente-instrucoes.png` | Agente / Instruções (`/orgs/:id/agents/:id/instructions`) | Sub-navegação com as 7 abas do agente, formulário estruturado de identidade/tom/escopo e prévia do prompt compilado em Markdown. |
| `05-agente-base-conhecimento.png` | Agente / Base de Conhecimento (`/orgs/:id/agents/:id/knowledge-base`) | Visão de busca híbrida e tabela de grupos de conhecimento vinculados ao agente. |
| `06-chat-teste.png` | Chat de Teste (`/orgs/:id/agents/:id/test-chat`) | Sessão de teste ativa com troca de mensagens (usuário e agente), indicador de status e caixa de composição. |
| `07-operacao.png` | Operação e Saúde (`/orgs/:id/operacao`) | Painel de telemetria operacional com saúde do motor, métricas de runtime, plugins declarados e cartão de cluster. |

---

## 2. Resolução e Viewport

As capturas seguem a especificação de layout desktop de referência (`console/UX_SPEC.md`):

- **Resolução / Viewport:** `1440 × 900` pixels
- **Device Scale Factor (DPR):** `1.0`
- **Tema:** Dark (fundo canônico `#151719` / `--pop-bg`)
- **Navegador:** Chromium / Google Chrome Headless com aceleração de hardware desabilitada para máxima reprodutibilidade de renderização.

---

## 3. Banco de Dados e Dados de Demonstração (Seed)

O ambiente utiliza um banco SQLite temporário gerado a partir das migrações oficiais (`poph migrate`).

- **Isolamento e Segurança:** Zero dados de clientes reais, zero contas de produção e zero credenciais privadas. O POP Harness é um projeto público e auditável.
- **Entidades Fictícias:**
  - Organizações de demonstração: `Acme Demonstração` (slug: `acme`) e `Beta Serviços` (slug: `beta`).
  - Departamento: `Suporte` (Atendimento Geral).
  - Agente: `Bia Atendente` (slug: `bia-atendente`, modelo `gemini-3.8-flash-tiered`).
  - Grupo e itens de conhecimento: `Políticas e Horários` com itens operacionais.
  - Conversa de teste: Sessão com mensagens predefinidas de pergunta e resposta.
- **Determinismo Temporal:**
  - Todos os registros utilizam timestamps fixados (`1795484000000`, equivalente a 23/11/2026 às 22:33:20 UTC-03:00), eliminando divergências de datas nas listagens e inspetores.

---

## 4. Campos Voláteis Documentados e Tolerâncias do Comparador

O script `scripts/compare.py` valida as capturas pixel a pixel e aplica tolerância rigorosa para diferenciar variações legítimas de regressões visuais:

### Limiares de Tolerância
1. **Tolerância por Canal de Cor (`COLOR_TOLERANCE = 12`):**
   - Variações de cor com delta máximo `<= 12` em qualquer canal (escala 0–255) são desconsideradas.
   - *Motivo:* Absorve diferenças imperceptíveis de sub-pixel antialiasing e suavização tipográfica entre versões do motor de renderização do navegador sem mascarar trocas de tokens visuais ou cores de bordas/foco (cujos deltas superam 50).
2. **Tolerância de Ruído de Antialiasing (`MAX_UNEXPECTED_PIXELS = 15`):**
   - Tolerância residual de até 15 pixels fora das regiões voláteis (equivalente a ~0,001% da área de 1.296.000 px).
   - *Motivo:* Evita falsos positivos decorrentes de micro-fringe de 1 px em curvas de fontes e cantos arredondados, enquanto garante reprovação instantânea diante de qualquer mudança estrutural de layout (um deslocamento de 2 px de margem afeta centenas de pixels).

### Regiões Voláteis Permitidas por Tela

Qualquer alteração fora das caixas delimitadoras abaixo resulta em saída `exit code 1` com a contagem de pixels divergentes e a caixa delimitadora (`bounding box`) da anomalia:

| Tela | Componente / Elemento | Caixa Delimitadora `[x, y, w, h]` | Motivo da Volatilidade |
|---|---|---|---|
| `01-login.png` | *(Nenhum)* | *(Zero tolerância)* | Tela 100% estática e determinística. |
| `02-orgs-lista.png` | Topbar: Chip do Motor (`.poph-topbar__estado`) | `[1050, 10, 214, 38]` | RSS dinâmico do processo (`Engine: 200 OK · [N] MB`). |
| `02-orgs-lista.png` | Telemetria de Orgs (`.poph-orgs__telemetria`) | `[230, 72, 270, 24]` | Contador de `Uptime` e uso de `RAM` do cluster. |
| `03-orgs-inspetor.png` | Topbar: Chip do Motor (`.poph-topbar__estado`) | `[1050, 10, 214, 38]` | RSS dinâmico do processo. |
| `03-orgs-inspetor.png` | Telemetria de Orgs (`.poph-orgs__telemetria`) | `[230, 72, 270, 24]` | Contador de `Uptime` e uso de `RAM` do cluster. |
| `04-agente-instrucoes.png` | Topbar: Chip do Motor (`.poph-topbar__estado`) | `[1050, 10, 214, 38]` | RSS dinâmico do processo. |
| `05-agente-base-conhecimento.png` | Topbar: Chip do Motor (`.poph-topbar__estado`) | `[1050, 10, 214, 38]` | RSS dinâmico do processo. |
| `06-chat-teste.png` | Topbar: Chip do Motor (`.poph-topbar__estado`) | `[1050, 10, 214, 38]` | RSS dinâmico do processo. |
| `07-operacao.png` | Topbar: Chip do Motor (`.poph-topbar__estado`) | `[1050, 10, 214, 38]` | RSS dinâmico do processo. |
| `07-operacao.png` | Timestamp (`.poph-operacao__atualizacao`) | `[1170, 100, 134, 36]` | Horário dinâmico da consulta (`Atualizado às HH:MM`). |
| `07-operacao.png` | Operação: Tempo no ar | `[730, 298, 150, 20]` | Segundos decorridos desde o início do daemon. |
| `07-operacao.png` | Operação: Memória usada pelo processo | `[730, 663, 150, 20]` | Consumo dinâmico de memória heap do servidor. |

---

## 5. Como Subir o Console e Repetir a Captura

### Pré-requisitos
- Binário `poph` compilado (`cargo build -p poph`)
- Node.js (>= 22.18) e dependências do monorepo (`playwright` instalado no workspace)
- Google Chrome ou Chromium

### Reprodução Automática (Recomendado)
A partir da raiz do repositório `apps/pop-harness`:

```bash
bash design_assets/20261123-refactor-baseline/scripts/capturar.sh
```

O script:
1. Cria uma pasta temporária isolada e migra o esquema do banco (`poph migrate`).
2. Semeia os dados estáveis de demonstração (`seed.py`).
3. Inicia o servidor `poph serve` em porta dedicada (padrão: `17485`).
4. Executa a captura das 7 telas via Playwright (`capture.mjs`).
5. Realiza uma segunda rodada de captura e executa a comparação de estabilidade (`compare.py`), comprovando ausência de regressões fora dos campos voláteis documentados.
