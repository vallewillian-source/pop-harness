# Memória quente e concorrência (HR-11)

> Desenho do crate `poph-agent` (D-08): o que fica em memória no processo, como o turno entra na
> fila, quanta memória é permitida e como as outras tarefas usam isto. Números medidos nesta
> máquina (x86_64), não estimados.

## 1. Onde cada coisa mora

| Peça | Módulo | O que guarda |
|---|---|---|
| Configuração quente | `config_quente` | agente + instruções + ferramentas habilitadas + top-k/corte do RAG, por `(org, agente)` |
| Atores por conversa | `atores` | uma tarefa tokio e uma fila por `(org, conversa)` |
| Janela de contexto | `contexto` | últimas mensagens que cabem no orçamento de tokens do agente |
| Limites | `limites` | simultaneidade por org, por agente e por provedor; cota por minuto da org |
| Métricas | `metricas` | atores vivos, despejos, acertos do cache, memória do cache, espera por vaga |
| Compositor | `compositor` | o texto do prefixo estável do prompt (saiu do `poph-core` nesta tarefa) |

Tudo é perdível: reiniciar o processo só custa reler do SQLite. O estado de verdade continua em
`harness.db` (`docs/DADOS.md`).

## 2. Cache de configuração quente

- **Chave**: `(org_id, agent_id)`. Não existe leitura de agente sem org.
- **Conteúdo**: `Agente` (modelo, limites de ferramenta, top-k/corte do RAG, Jev), instruções
  gravadas, JSON estruturado já interpretado, markdown do prefixo e a lista de ferramentas
  habilitadas.
- **D-04 aplicada**: `AgenteQuente::ferramentas` é a lista fechada; `ferramenta_habilitada(nome)`
  é o filtro que a HR-13 usa no laço de ferramentas. Ferramenta desabilitada não entra.
- **D-05 aplicada**: só `active` entra no cache — `Agentes::obter_para_turno` recusa inativo ou
  arquivado com `DomainError::AgenteInativo`, então nunca há agente que não pode responder em cache.
- **Invalidação**: `carregar_verificando` lê **duas** versões baratas (agente e instruções) e
  recarrega quando qualquer uma mudou. Editar só o texto do prompt conta: a versão do agente não
  muda, mas a das instruções sim. Há também `invalidar(org, agente)` e `esquecer_org(org)` para
  quem edita em lote, e `carregar` (sem verificação) para quando o chamador já sabe que nada mudou.
- **Validade**: 60 s por padrão (`OpcoesConfigQuente::validade`). Depois disso, o próximo
  `carregar` relê.
- **Teto**: 64 MB de memória estimada por padrão, com peso por entrada (`bytes`). O cache não
  cresce além do teto; com 10 mil agentes ele despeja os menos usados (teste do grupo 5).
- **Markdown**: usa o texto **gravado** — inclusive o editado à mão. O cache não recompõe o prompt:
  recompõe quem grava (CLI/API), não quem lê.

## 3. Atores por conversa

- Uma tarefa tokio e uma fila (`mpsc`) por `(org, conversa)`. O consumidor é único, então **dois
  turnos da mesma conversa nunca rodam em paralelo** e a ordem de chegada é preservada.
- **Despejo por ociosidade**: depois de `ociosidade` (90 s padrão) sem evento, o ator sai da
  memória. A saída só acontece com a fila vazia e sob o mesmo cadeado que o envio usa — não existe
  janela em que um evento fique órfão nem em que dois atores sirvam a mesma conversa.
- **Recarga**: quando a conversa volta, o processador recebe `ao_acordar(org, conversa)` antes do
  primeiro evento e recarrega o que precisar do banco. Nada do estado durável mora no ator.
- **Fila cheia**: `enviar` devolve `ErroAgente::FilaCheia` (padrão: 64 eventos por conversa) em vez
  de crescer sem limite. Quem segura e reenvia é a `inbox` (HR-12).
- **Teto de atores**: 4096 por padrão. Ao passar do teto, só um ator **ocioso** é despejado; se
  todos estiverem ocupados, o teto é excedido (e contado em `poph_atores_acima_do_teto_total`),
  porque despejar ator ocupado quebraria "um turno por conversa".
- **Sem runtime tokio**, `enviar` devolve `ErroAgente::Interno` em vez de entrar em pânico.

## 4. Janela de contexto

- Entram as mensagens mais recentes, em ordem, até o orçamento de `agents.max_context_tokens` e o
  teto de `agents.max_context_messages`.
- Contagem estimada: ~4 caracteres por token, mais 4 tokens fixos por mensagem. O tokenizador real é
  do provedor; a HR-13 desconta o uso informado e a HR-14 gasta parte do orçamento com o resumo.
- Uma mensagem que sozinha não cabe é **cortada** (com `…`) quando é a mais recente: um turno sem a
  fala do cliente não existe. As demais que não caibam ficam de fora (`descartadas`), para virar
  resumo na HR-14.
- `Janela::folga()` diz quanto sobrou — é o número que a HR-14 usa para decidir o tamanho do resumo.

## 5. Limites

| Nível | Chave da config | Padrão |
|---|---|---|
| Instância | `limites.turnos_simultaneos` | 200 (D-07) |
| Org | `limites.turnos_simultaneos_por_org` | 50 |
| Agente | `OpcoesLimites::com_agente` (0 = sem teto próprio) | sem teto |
| Provedor | `OpcoesLimites::com_provedor` (`llm.simultaneos`) | 32 |
| Cota da org | `limites.taxa_por_org_por_minuto` | 600 |

- Semáforos separados por org: **org saturada não atrasa outra org** (não há fila nem lock global
  compartilhado entre orgs além do teto da instância, que é o desenho).
- `adquirir_turno` devolve uma `Vaga`; soltar é sair de escopo. Espera com teto de tempo →
  `ErroAgente::SemVaga`, que é transitório (`eh_transitorio()`), para o chamador reenfileirar.
- `consumir_cota(org)` é um balde de fichas por minuto; acabou, devolve `CotaEsgotada` com o tempo
  de espera sugerido.

## 6. Métricas (`/metrics` da HR-20)

`poph_atores_vivos`, `poph_atores_despejados_total`, `poph_atores_acima_do_teto_total`,
`poph_eventos_entregues_total`, `poph_eventos_recusados_total`,
`poph_config_quente_acertos_total`, `poph_config_quente_erros_total`,
`poph_config_quente_invalidadas_total`, `poph_config_quente_bytes`, `poph_config_agentes_total`,
`poph_espera_vaga_segundos`. Não repetem nome do catálogo base (`poph-types::MetricasBase`), então
os dois podem ser registrados no mesmo registro.

## 7. Como a HR-12 e a HR-13 usam

HR-12 (inbox/outbox):
1. grava a entrada e chama `atores.enviar(Tarefa { org, conversa, mensagem })`;
2. se vier `FilaCheia`, a mensagem fica `pending` na `inbox` e é reenviada depois (o ACK do webhook
   já foi dado; a garantia é a chave única da `inbox`);
3. na subida, o que ficou `pending` volta para a fila do ator.

HR-13 (turno):
1. `config.carregar_verificando(org, agente)` → `AgenteQuente`;
2. `limitador.adquirir_turno(org, agente, espera)` e `limitar_ao_provedor` antes de chamar o LLM;
3. `contexto::do_banco(store, org, conversa, agente)` para a janela;
4. `agente_quente.markdown` é o prefixo estável; `agente_quente.ferramentas` filtra o toolset;
5. o processador roda dentro do ator, então já tem "um turno por vez" garantido.

## 8. Números medidos (x86_64, 12 núcleos, build debug)

Comando: `cargo test -p poph-agent --test memoria_e_concorrencia -- --ignored --nocapture custo`

| Medida | Valor |
|---|---|
| Enfileirar um evento no ator (cadeado + canal) | ~0,97 µs/evento |
| Laço completo do ator (receber + processar no-op + liberar) | ~1,73 µs/evento |
| Leitura de config quente em cache (acerto) | ~1,18 µs/leitura |
| Carga do teste do teto: 10 mil agentes no cache com teto de 1 MB | cache fica ≤ 1 MB e < 10 mil entradas |

O custo do ator está três ordens de grandeza abaixo da meta de latência interna (M6: ≤ 2 ms p50) e
some perto do LLM (6,5 s até o primeiro texto, HR-01). A leitura do cache evita duas consultas por
turno; a diferença aparece como ~1 µs, mas o ganho real é não tocar o disco no caminho crítico.

## 9. Limites conhecidos (o que não está coberto)

- **A invalidação é conferida por leitura de versão**, não por aviso do escritor: se a CLI/API
  gravar instruções, o turno seguinte já vê; se gravar em lote, cada chave paga uma consulta por
  chave primária. Não existe barramento de invalidação (não foi necessário na V1).
- **O teto de atores pode ser excedido** se todos estiverem ocupados (por desenho: preferir exceder
  a quebrar "um turno por conversa").
- **Evento recusado por fila cheia não é retentado aqui**: é a HR-12 que o mantém na `inbox`.
- **Não há cancelamento de turno em andamento**: o turno roda até terminar. Cancelamento do LLM é
  da HR-13.
- **A janela é uma estimativa de tokens**; não usa o tokenizador do Gemini.
- **Não medido**: carga longa (horas, vazamento/fragmentação), ARM64 (HR-56), comportamento com
  500 conversas ativas simultâneas (HR-55).
- **`poph` ainda não usa este crate**: a ligação com o turno e a CLI é da HR-13.
