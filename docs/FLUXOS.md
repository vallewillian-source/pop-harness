# Fluxos e Fichas (POP Harness)

> Referência do módulo: o que existe, como se comporta e o que foi testado. A linguagem dos scripts (declarações, regras, diagnósticos) está em [`FLUXOS_LINGUAGEM.md`](FLUXOS_LINGUAGEM.md); as diferenças em relação ao POP Atende, em [`FLUXOS_PARIDADE.md`](FLUXOS_PARIDADE.md). Cada afirmação aponta o teste ou o relatório que a sustenta. Os números de desempenho estão na seção [Medidas](#medidas).

**Status (23/11/2026):** implementado de FX-01 a FX-36, com commits locais no submódulo, sem push e sem produção. Pendente: a medição do G9 com o Jev real (FX-37), que depende de autorização do Willian para gastar a chave do Jev. Só backend: a interface do Console não mudou.

The administrative routes for flows and cases live in [`api/flows.rs`](../crates/poph-server/src/api/flows.rs) and [`api/cases.rs`](../crates/poph-server/src/api/cases.rs). The public route names and JSON fields are documented in [API.md](API.md).

## 1. Visão

Cada atendimento (uma conversa) tem uma **Ficha**: o registro do que aconteceu nele. Dentro da Ficha ficam as **execuções de fluxo**, com as respostas e o status de cada etapa.

Um **fluxo** é um roteiro de atendimento escrito num arquivo de script (Starlark, uma linguagem parecida com Python, sem `while` e sem acesso a relógio, rede ou arquivo). O fluxo pertence à organização; cada agente só usa os fluxos que alguém liberou a ele. Quando o cliente pede aquele roteiro, o fluxo começa; o motor avança as etapas, guarda os dados coletados, decide pelos caminhos e produz os efeitos (mensagem ao cliente, aviso no WhatsApp da equipe, encaminhamento para um departamento).

Regras que valem hoje (decisões do Willian de 21/11/2026, e as que tomei com base nelas):

- Uma Ficha por atendimento, criada no começo de cada conversa. Depois que o atendimento fecha, a próxima mensagem do cliente abre outra Ficha.
- Uma execução ativa por Ficha por vez. Fluxos podem se suceder na mesma Ficha.
- Concluir um fluxo não fecha a Ficha. Só um `fim` que peça o encerramento, ou o encerramento explícito do atendimento, fecha.
- Fluxo novo não é usado por nenhum agente até ser liberado (padrão negado).
- Publicar cria uma revisão numerada e imutável. Cada execução segue presa à revisão em que começou.
- Quem decide iniciar um fluxo é o código, com a escolha do Jev (G9), antes de o modelo de texto responder.
- Falha de efeito (ex.: WhatsApp fora do ar) não desfaz a etapa: aparece como evento na Ficha e é reenviada até o limite.

Para desligar o motor sem perder nada, `[fluxos] ativo = false`: as Fichas continuam sendo criadas e fechadas, e o turno volta ao comportamento anterior.

## 2. Ciclo da Ficha

**Criação.** Toda conversa nova ganha a Ficha na mesma escrita que a cria, tanto na API quanto na mensagem que chega pelo canal. O banco garante uma Ficha por conversa (índice único). A migração `0012` criou uma Ficha para cada conversa que já existia. Testes: `cases_invariant.rs` (`creating_conversation_creates_case_in_same_write`, `channel_reception_creates_one_case_per_conversation_and_not_per_message`) e `cases_migration.rs` (`backfill_creates_exactly_one_case_per_conversation`).

**Estados.** Aberta ou fechada. A Ficha continua aberta enquanto a conversa está aberta **ou transferida** para atendente humano. Fecha quando a conversa fecha.

**Quem fecha, e como.** Fechar a conversa fecha a Ficha na mesma transação e abandona a execução ativa, se houver (motivo `atendimento_encerrado`). Os caminhos são:

- o agente chama `atendimento_encerrar` (ferramenta do turno);
- um fluxo chega a `fim(encerra_atendimento=True)`;
- o atendente ou o admin fecha pela API (`POST .../fichas/{ficha_id}/close`), que também vale quando a conversa já foi apagada pela retenção;
- o fechamento da conversa pelo repositório (`close_audited` e `change_status`).

Testes: `cases_invariant.rs` (`closing_conversation_closes_case_and_records_event`, `changing_status_to_closed_also_closes_case`, `closing_with_active_execution_abandons_execution_in_same_write`, `failure_midway_through_closing_leaves_no_closed_conversation_with_open_case`), `fichas_acoes.rs` (close com e sem conversa) e `fluxos_e2e.rs` (encerramento pelo agente). A lista dos caminhos é mantida à mão; não há teste que a enumere sozinho.

**Depois de fechar.** Conversa fechada não reabre: reabrir devolve `409 conversa_encerrada` (PRD L13). Conversa transferida reabre normalmente. A mensagem seguinte do cliente, depois de fechado, abre conversa e Ficha novas (`message_after_closing_opens_new_conversation_and_case`, `reopening_closed_conversation_is_refused_and_transferred_keeps_reopening`, em `cases_invariant.rs`; e o encerramento com Ficha seguinte em `fluxos_e2e.rs`, `encerramento_do_agente_e_proxima_mensagem_abrem_nova_ficha`).

**Histórico.** Cada Ficha tem uma linha do tempo em `ficha_eventos`, numerada por `seq` e com idempotência por chave. Os tipos são `ficha_aberta`, `fluxo_iniciado`, `etapa_entrou`, `campos_preenchidos`, `acao_executada`, `decisao_resolvida`, `etapa_concluida`, `efeito_criado`, `efeito_enviado`, `efeito_falhou`, `execucao_concluida`, `execucao_abandonada`, `execucao_falhou`, `atendimento_transferido` e `ficha_fechada`. A API lê a Ficha com as execuções, as etapas, os campos respondidos e os eventos (`cases_invariant.rs`, `events_leave_in_order_and_paginate_by_seq`). O histórico completo de cada Ficha é conferido na prova ponta a ponta (`fluxos_e2e.rs`).

**Quem fechou.** A coluna `fechada_por` aceita `cliente`, `agente`, `atendente`, `admin`, `fluxo`, `sistema` e `migracao`. Cada caminho acima grava o seu valor; não há teste que cubra cada valor separadamente.

**Retenção.** A retenção existente não apaga Fichas, execuções, etapas, eventos, efeitos, fluxos nem revisões. Quando a conversa é apagada, a Ficha fica com a referência à conversa nula (`conversation_id` vazio). Teste: `poph-retention/tests/database.rs` (`case_execution_step_event_and_effect_survive_the_sweep`). Uma política própria de retenção de Fichas não existe (ver Limites).

## 3. Execução

**Início: antes do modelo de texto.** Em cada turno, com o motor ligado e a Ficha sem execução ativa:

1. Se há fluxo liberado ao agente, o estado do turno leva a lista (nome e critério de cada fluxo) e o gate **G9** do Jev escolhe um. Ver [JEV.md §5-I](JEV.md).
2. Escolha acima dos limiares (confiança 0,7 e margem 0,1 sobre a segunda opção) → o **código** valida e abre a execução. A validação confere: fluxo da organização, ativo, liberado ao agente da Ficha, com revisão publicada, Ficha aberta e nenhuma outra execução ativa.
3. `nenhum`, confiança baixa ou margem curta → nada inicia, e o catálogo não vai ao modelo.
4. Jev fora do ar, gate desligado ou resposta inutilizável → **reserva**: o modelo recebe a lista de fluxos e pode chamar `fluxo_iniciar`, que passa pela mesma validação.

Se já existe execução ativa, o turno não pergunta nada ao Jev: o bloco da etapa atual entra no prompt. Testes: `poph-jev/tests/g9_fluxo.rs` (opções, cortes em 400 caracteres, limiares, fallback, flag), `poph-agent/tests/flow_turn.rs` (início antes do modelo, reserva, bloco da etapa) e `poph-agent/tests/flow_tools.rs`.

**Revisão presa.** A execução guarda a revisão com que começou. Publicar a revisão 2 não muda a execução que está na 1 (`fluxos_e2e.rs`, `revisao_antiga_acao_humana_e_dois_fluxos_na_mesma_ficha`; `flows_migration.rs`, `published_revision_cannot_be_changed_nor_deleted`). `poph flow diff` mostra quantos atendimentos estão presos em cada revisão.

**Etapas e ações.** Cada etapa pede campos (com tipo) e oferece ações. Quem age é sempre o ator da ação: `agente` (ferramenta do turno, sem escolha do modelo) ou `atendente` (pela API). Ação que não existe na etapa atual é recusada com a lista das válidas, e nada é gravado: o motor não executa ações intermediárias sozinho. Etapa que exige campo não avança sem ele, e a resposta diz o que falta (`engine.rs`: `action_validates_records_fields_and_advances_for_step_target`, `required_field_missing_returns_list_of_what_is_missing`).

**Decisões.** Uma decisão escolhe o ramo pela regra (combinadores ou uma função `def`). Decisões encadeadas atravessam até 10 saltos numa mesma passagem (`[fluxos] max_saltos_decisao`); ciclo ou excesso leva a execução a falha (`engine_outcome.rs`: `ten_jumps_pass_and_eleven_crash_execution`, `decision_cycle_puts_execution_in_failure`).

**Fim.** `fim(...)` conclui a execução. Com `encerra_atendimento=True`, também fecha a conversa e a Ficha (ver §2). Execução concluída, abandonada ou em falha não aceita mais ação (`closed_execution_accepts_no_transition`, em `poph-store/tests/flow_executions.rs`).

**Abandono.** O agente abandona com `fluxo_abandonar(motivo)`; o atendente, com `POST .../flow-runs/{run_id}/abandon`. Abandono sem motivo é recusado.

**Erro de regra.** Regra que não avalia (campo ausente, passos ou memória esgotados, prazo estourado) leva a execução a `falha`, com um código `FLXR…`, e o atendimento segue. Os limites de uma avaliação: `max_ticks_regra` (10.000 por padrão), `max_heap_regra` (1 MiB) e 200 ms de prazo (`rules.rs`, `limits.rs`).

**Dados da execução.** O `dados_json` tem teto de 64 KB (`max_dados_bytes`). Script acima de 32 KB é recusado antes de compilar (`max_fonte_bytes`), e script com mais nós que `max_nos` também é recusado (`limits.rs`: `script_of_40_kib_refused_with_default`, `node_cap_from_configuration_refuses_what_exceeds_it`).

**Agente e atendente agindo ao mesmo tempo.** Só um vence; o outro recebe `412` (versão velha) e nada é gravado (`fichas_acoes.rs`: `corrida_atendente_com_agente_da_412_e_nao_grava_nada`; `flow_executions.rs`: `two_threads_agent_and_attendant_only_one_wins`).

**Ações humanas na API.** O atendente abre um fluxo liberado ao agente da Ficha, sem passar pelo Jev (`POST .../flow-runs`), age (`.../actions`), abandona e fecha a Ficha. Todas exigem `If-Match` e `chave_idempotencia`; repetir a chave devolve o resultado da primeira chamada, sem duplicar evento (`fichas_acoes.rs`).

**Cache de revisões.** As revisões compiladas ficam em memória, com teto (`cache_revisoes_mb`, padrão 16 MB), e a menos usada que não esteja em uso sai primeiro. Carga concorrente da mesma revisão compila uma vez só. Falha de carga não derruba o processo: a execução vai para falha (`poph-fluxos/tests/cache.rs`).

## 4. Efeitos

Efeitos são o que sai do atendimento para fora da conversa. O motor só **produz** efeitos; a entrega é feita por um laço do servidor. São três tipos:

| Efeito | O que faz | Por onde sai |
|---|---|---|
| `mensagem_ao_cliente(texto)` | Texto fixo para o cliente | Fila de saída da conversa. Espera o turno da conversa terminar, para não chegar antes da resposta do agente. |
| `notificar_whatsapp(para, mensagem)` | Aviso para outro número (equipe) | Integração de WhatsApp do agente da Ficha. Não usa a fila da conversa. |
| `encaminhar(departamento)` | Transfere a conversa para o departamento pelo nome | Transferência pela operação do turno. A Ficha continua aberta e o agente para de responder. |

**Gravação e entrega.** Os efeitos são gravados na mesma transação que a transição da etapa. O laço de entrega roda uma vez por segundo, e só com o motor ligado.

**Estados.** `pendente` → `enviando` → `enviado` ou `falhou`. A tentativa é gravada antes da chamada externa. Falha transitória (rede, 429, 5xx) volta a `pendente` com espera crescente. Falha definitiva (4xx, departamento inexistente, conversa encerrada, Ficha sem conversa) vai a `falhou`, com o motivo. Depois de 8 tentativas (`max_tentativas`), também. Sem integração de WhatsApp, o efeito fica `pendente` e não consome tentativa.

**Falha não desfaz a transição.** A transição da etapa continua valendo. A falha vira o evento `efeito_falhou` na Ficha, e o atendimento segue (PRD L11).

**Entrega "pelo menos uma vez".** Se o servidor cair depois do envio e antes de gravar `enviado`, o efeito pode sair de novo. Isso é aceito e documentado. Efeito já enviado não sai de novo quando a execução segue: a prova ponta a ponta roda um ciclo extra de efeitos e saída e confere que nada é reenviado (`fluxos_e2e.rs`, `exemplo_entrega_notificacao_unica_e_transferencia_com_ficha_aberta`).

**Testes.** Laço de entrega em `poph-server/tests/fluxos_efeitos.rs` (`mensagem_ao_cliente_chega_ao_cliente_uma_vez`, `notificar_whatsapp_vai_para_a_equipe_e_nao_para_o_cliente`, `sem_integracao_o_efeito_fica_pendente_e_nao_trava_o_laco`, `queda_entre_enfileirar_e_confirmar_reenvia_e_aceita_duplicata`, `falha_definitiva_vira_falhou_com_evento_e_metrica`); transição, efeitos na mesma escrita e falha no meio sem evento órfão em `poph-store/tests/flow_executions.rs` (`transition_writes_state_step_event_and_effect_in_one_write`, `failure_midway_leaves_no_event_without_state`); os três exemplos com efeitos em `fluxos_e2e.rs`.

## 5. Operação

### 5.1 Configuração `[fluxos]`

| Chave | Padrão | Faixa | O que controla |
|---|---|---|---|
| `ativo` | `true` | — | Liga o motor. Desligado: G9, prompt, ferramentas e efeitos param; as Fichas continuam. |
| `max_fluxos_por_org` | 100 | 1 a 10.000 | Teto de fluxos publicados por organização. |
| `max_fonte_bytes` | 32.768 | 1.024 a 1.048.576 | Tamanho máximo do script. |
| `max_nos` | 60 | 1 a 60 | Etapas, decisões e fins por fluxo. |
| `max_dados_bytes` | 65.536 | 1.024 a 10.485.760 | Tamanho do `dados_json` de uma execução. |
| `max_ticks_regra` | 10.000 | 100 a 1.000.000 | Passos de uma avaliação de regra. |
| `max_heap_regra` | 1.048.576 | 65.536 a 67.108.864 | Memória de uma avaliação de regra. |
| `max_saltos_decisao` | 10 | 1 a 100 | Decisões encadeadas numa passagem. |
| `cache_revisoes_mb` | 16 | 0 a 4.096 | Teto de RAM do cache de revisões. `0` não guarda nenhuma. |

Valor fora da faixa é recusado com o nome da chave. O gate G9 vem ligado no padrão de instalação (`[jev].gates.g9`), e cada agente pode desligá-lo no `jev_json`. Ver [CONFIGURACAO.md](CONFIGURACAO.md).

### 5.2 Comandos

- `poph flow validar|publicar|listar|mostrar|diff|formatar|liberar|retirar`. Depois da primeira publicação, `publicar` exige `--if-match`; `liberar` e `retirar` mudam a permissão do agente. Passo a passo em [RUNBOOKS.md](RUNBOOKS.md) (seção 7).
- `poph ficha listar|mostrar`, com filtros `--status`, `--agente`, `--canal` e `--desde`.
- `poph doctor`: mostra se o motor roda e os tetos em uso.
- `poph --offline backup export|import`: o backup leva fluxos, revisões e permissões; com `--com-conversas`, leva também as Fichas e o histórico. Importar recusa sobrescrever por padrão ([BACKUP.md](BACKUP.md)).

### 5.3 API

São 19 rotas novas: 12 de fluxos e de permissão por agente, e 7 de Fichas e execuções. Todas ficam sob o token da organização, com `If-Match` nas escritas. Caminhos, corpos e erros estão em [API.md](API.md). O cliente gerado e o `schema.d.ts` foram regenerados na FX-32. As rotas entram no encaminhamento do cluster (`encaminhamento_cluster.rs`, `rotas_de_fluxos_e_fichas_sao_encaminhadas_para_a_dona`).

### 5.4 Métricas

Só existem com `[fluxos] ativo = true`. Estado de cada uma, verificado no código:

- **Incrementadas hoje:** `poph_fluxos_cache_*`, `poph_fluxos_expulsoes_total` e `poph_fluxos_revisoes_*` (cache); `poph_fluxos_g9_iniciou_total`, `poph_fluxos_g9_nenhum_total`, `poph_fluxos_g9_abaixo_do_limiar_total` e `poph_fluxos_g9_fallback_total` (turno); `poph_fluxos_efeitos_pendentes_total`, `poph_fluxos_efeitos_enviados_total` e `poph_fluxos_efeitos_falhas_total` (laço de efeitos); `poph_fluxos_execucoes_iniciadas_total`, `poph_fluxos_execucoes_concluidas_total`, `poph_fluxos_execucoes_abandonadas_total`, `poph_fluxos_execucoes_falhas_total` e `poph_fluxos_erros_regra_total` (o `poph-store`, único lugar que escreve `fluxo_execucoes` — FX-40).
- **Como os contadores de execução contam:** na abertura (turno, reserva e API), no desfecho da transição (concluída, abandonada ou falha), no fechamento da Ficha (`atendimento_encerrado`), na exclusão do fluxo com `force` (`fluxo_excluido`) e, no caso do erro de regra, quando a regra não avalia. Repetir a chave de idempotência não soma de novo. A CLI e o `poph migrate` escrevem no banco sem publicar catálogo, então não contam — o número é da instância que roda com `poph serve`.

### 5.5 Retenção, backup e cluster

- **Retenção:** não apaga Fichas, execuções, etapas, eventos, efeitos, fluxos nem revisões ([RETENCAO.md](RETENCAO.md)).
- **Backup:** ida e volta com fluxos, revisões e permissões; com conversas, também as Fichas e o histórico. Um backup no formato anterior (sem o bloco de Fichas) importa e recria uma Ficha por conversa ([BACKUP.md](BACKUP.md)).
- **Cluster:** as rotas de fluxos e Fichas vão para a instância dona da organização ([CLUSTER.md](CLUSTER.md)).

### 5.6 Quando um fluxo não inicia

O passo a passo para descobrir o motivo está em [RUNBOOKS.md](RUNBOOKS.md), na seção de fluxos.

## 6. Limites e o que não foi testado

- **G9 com o Jev real: medido em laboratório, não em produção.** A medição da FX-37 (com autorização do Willian e a chave do Jev) deu 48/50 de comportamento certo nas duas execuções; o conjunto é de 50 falas escritas para medir, não tráfego real. Números e recomendação em [JEV.md](JEV.md) §5-I.
- **Sem LLM, WhatsApp, Matrix ou Synapse reais.** A prova ponta a ponta (`fluxos_e2e.rs`) usa LLM, Jev e Evolution falsos, sem rede e sem credencial.
- **CI completa não rodada.** `scripts/ci.sh` (Console, `cargo deny`, `cargo audit` e checagem aarch64) não foi executado nas entregas. Foram rodados os testes dos crates afetados, `clippy` e `fmt`. A checagem ARM64 depende de autorização (VM3).
- **Contadores de execução só na instância que serve.** São contados no `poph-store`, a partir do catálogo que o `poph serve` publica na subida (FX-40); a CLI e o `poph migrate` não publicam e não contam. Depois de reiniciar o processo, os contadores começam em zero com o resto do `/metrics`.
- **Notificação "uma vez" é garantida em camadas.** Queda do servidor entre o envio e a gravação de `enviado` pode repetir o efeito (aceito, ver §4).
- **Medidas em máquina carregada.** A estação estava com outros builds rodando; as comparações foram intercaladas. A RAM foi medida com fluxo de 8 etapas; a meta de 40 nós é extrapolação linear, não medida. Não há medida em ARM64 nem sob carga de produção.
- **Alocação grande no Starlark.** `"x" * 10**9` é cortado pelo prazo, não pelo limite de memória: a alocação chega perto de 2 GB antes do corte ([FLUXOS_LINGUAGEM.md](FLUXOS_LINGUAGEM.md), §3.4).
- **Sem política própria de retenção de Fichas.** Decisão do Willian de 23/11/2026: Fichas não são apagadas, não há retenção própria (D-31 em [DECISOES.md](DECISOES.md)).
- **Decisões do PRD §15 confirmadas pelo Willian em 23/11/2026** (D-31 em [DECISOES.md](DECISOES.md)): o G8 não fecha o atendimento, Fichas não são apagadas e o efeito `notificar_whatsapp` sem integração fica `pendente` (não vai a `falhou` depois do limite de tentativas). Seguem em vigor sem decisão dele: não existe liberação "para todos os agentes" (só a permissão por agente) e o teto de 20 fluxos por agente está implementado.
- **Sem interface.** A UI do Console não muda (decisão W8). O editor visual de fluxos fica para depois.
- **Numeração de migrações.** A migração `0014` não existe no repositório; a sequência vai da `0013` para a `0015`. Não é do módulo e não foi preenchida.

## 7. Onde cada coisa está provada

| Tema | Testes ou relatórios |
|---|---|
| Ficha em todo caminho de criação e fechamento | `poph-store/tests/cases_invariant.rs`, `cases_migration.rs` |
| Esquema (tabelas, gatilhos, uma execução ativa) | `poph-store/tests/flows_migration.rs`, `flows_repo.rs` |
| Linguagem (compilador, validador, regras, limites) | `poph-fluxos/tests/compiler.rs`, `validator.rs`, `rules.rs`, `limits.rs` |
| Motor (início, ações, decisões, fim, efeitos) | `poph-fluxos/tests/engine.rs`, `engine_outcome.rs`, `poph-store/tests/flow_executions.rs` |
| Gate G9 e turno | `poph-jev/tests/g9_fluxo.rs`, `poph-agent/tests/flow_turn.rs`, `flow_tools.rs` |
| Paridade com o POP Atende | `poph-fluxos/tests/parity.rs` e [FLUXOS_PARIDADE.md](FLUXOS_PARIDADE.md) |
| API, permissão, Fichas e ações humanas | `poph-server/tests/agente_fluxos.rs`, `fichas_leitura.rs`, `fichas_acoes.rs`, `isolamento_orgs.rs` |
| CLI | `poph/tests/cli_flow.rs` |
| Backup | testes de backup em `poph-store` (19), `poph-server/tests/backup_http.rs` (9) e `poph/tests/cli_backup.rs` (6) |
| Retenção e cluster | `poph-retention/tests/database.rs`, `poph/tests/cli_retencao.rs`, `poph-server/tests/encaminhamento_cluster.rs` |
| Prova ponta a ponta | `poph-server/tests/fluxos_e2e.rs` (7 testes) |
| Contadores das execuções | `poph-server/tests/fluxos_contadores.rs` (7 testes) e o caso de contadores em `fluxos_e2e.rs` |
| Desempenho | seção [Medidas](#medidas) abaixo |

## 8. Crate layout

`poph-fluxos` is pure: no I/O, no database, no network. It depends on `poph-types` and the script
library (Starlark) only, and imports no other workspace crate.

| Module | Responsibility |
|---|---|
| `compilador` | script → IR, with shape diagnostics. |
| `validador` | semantic diagnostics over the IR and department resolution. |
| `regras` | combinators and advanced functions (`def`), with tick, heap and deadline caps. |
| `motor` | one run of an execution: start, record fields, run an action, advance, resolve decisions, reach the end, abandon and produce effects. |
| `prompt` | the step block of the prompt and its rendering. |
| `diff` | difference between revisions in plain language. |
| `formatter` | canonical form of the script (script ⇄ structure). |
| `cache` | cache of compiled revisions (LRU with an MB cap, single-flight and fail-closed). |
| `ir`, `language`, `diagnostics` | the compiled structure, the dialect and the closed diagnostic list. |

The IR schema rules (every field always present, declaration order, additive changes only) are in
[FLUXOS_LINGUAGEM.md](FLUXOS_LINGUAGEM.md) §8; the closed diagnostic list, the two directions
(line/column over the text, node path over the IR) and the rule that only an error blocks a
publication are in §10.

**Load path.** A script is loaded as parse → evaluation with limits → freeze. The engine uses it to
have the module with the `def`s loaded; the compiler uses the same path as the last barrier, so with
a script of valid shape the load can only fail on a limit (§3.4).

**Declaration functions do nothing.** The functions registered in the Starlark module return `None`:
the compiler builds the IR from the syntax tree (that is what gives line and column for each
diagnostic) and the engine only needs the frozen module to evaluate the `def`s.

**Why `blake3` is a dependency.** It comes with Starlark and has no direct use in this crate: the
cache key uses `sha2`, the same `sha256` as `fluxo_revisoes.fonte_sha256`. `use blake3 as _;` only
marks the dependency as used, because the Rust `unused_dependencies` lint breaks clippy with
`-D warnings`.

### 8.1 Cache of compiled revisions (`cache`)

- A published revision is immutable, so the cache key is the flow id, the revision number and the
  sha256 of the script text. The hash is what closes the key: the publisher stores it in
  `fluxo_revisoes.fonte_sha256`, and the cache computes the same value from the text
  (`sha256_of_source`). `RevisionKey::with_hash` takes the value already stored in the database.
- Loads are single-flight: concurrent calls for the same key load once and the others wait for that
  result. A waiter stops waiting after 30 s (`MAX_BATCH_WAIT`) and loads by itself, so a loader
  that vanished (panic) cannot block the key.
- A failed load (`ir_divergente`, `fluxo_invalido`) is returned as a typed error and is not cached.
  The cache keeps working; the execution ends in failure (FLXR06) and the rest of the module is
  unaffected.
- The cap is in bytes. `RevisionCache::new` takes megabytes, the unit of
  `fluxos.cache_revisoes_mb`; the default is 16 MB. Each entry weighs twice the length of the
  canonical JSON of the IR plus 256 bytes. The RAM measurement (`ram_por_revisao_medida`) found
  5,857 to 6,389 bytes per revision for 3,666 bytes of JSON, a factor of 1.60 to 1.74; the factor 2
  errs on the safe side, so the cache sits a little below the cap in real RAM.
- Eviction removes the least recently used entry that no caller holds (the `Arc` count is 1).
  If there is still no room, the new revision is not cached (counted in `not_cached`) and the call
  still returns the IR. A full cache never fails an execution; it only stops speeding it up.
  A revision larger than the cap is not cached. A cap of 0 caches nothing and is not an error.
- Eviction scans the entries (O(n)); this is intended, since the cap holds dozens of revisions.
  One cache per process; `Clone` shares the state. The mutex tolerates poisoning.
- The cache reads no configuration, touches no database and no clock (the order of use is a
  counter), and keeps no entry forever.
- `get` counts as use but not as hit or miss. `load` is the entry point that counts hits, misses,
  loads and waits.

### 8.2 Prompt block (`prompt`)

- It is the volatile part of the agent prompt: what the agent sees about the flow in progress in
  this turn. The stable prefix is not built here. The same function feeds the Fichas screen
  (`stage_block`, the `preview` of PRD §6.6-5): it shows what the agent would see on entering a
  step, with no values filled. A step the IR does not have is "not found" (HTTP 404).
- Text is Portuguese, one piece of information per line, always ending with a newline. Order: the
  flow in progress; the current step with its instruction; who is still missing (only when the
  step is `parcial`); the step fields (type, required, value already filled); the actions (what
  each requires, who runs it and where it leads, in words); and the step map last.
- The essential sections come before the map on purpose: when the cap cuts, the map is cut first.
- The only text that comes from the client is the field values. Each value is quoted as JSON text,
  with quotes, line breaks and backslashes escaped, so a value that tries to open a new section
  stays one line marked as data. The header warns the agent of this.
- Deterministic: the same input gives the same bytes. Fields come in IR order and object keys
  (for example an attachment `{name, type, size}`) come sorted, so the output does not depend on
  whether `serde_json` was built with `preserve_order`.
- Cap: at most `BLOCK_CAP` characters. When it is passed, whole lines of the map are cut and
  `…` ends the block on its own line. If the essential part alone is over the cap (an instruction
  written by hand that is too long), the text is cut by characters at the cap and ends with `…`, so
  the agent still sees the start of the step. The cut depends only on the text.
- Booleans show as the canonical `sim` / `nao`.

### 8.3 Rule evaluator (`regras`)

- A rule is evaluated over a snapshot of the execution data, `{"step": {"field": value}}`, sorted
  (`BTreeMap`), so evaluation is deterministic. The data enters as a value and never as code.
- Combinators (`igual`, `diferente`, `maior`, `menor`, `contem`, `vazio`, `e`, `ou`, `nao`) run in
  native Rust, outside Starlark. `funcao("name")` runs the `def` in the Starlark module loaded once
  per pass (`RuleEvaluator::new`) and must return `True` or `False`. A rule without `funcao` uses
  `RuleEvaluator::without_functions`, which loads no module.
- Missing field (not filled or `null`), by combinator:

  | Combinator | Missing field | Why |
  |---|---|---|
  | `igual` | false | there is no value to compare |
  | `diferente` | true | what does not exist differs from the literal |
  | `maior` / `menor` | false | there is no number |
  | `contem` | false | there is no text |
  | `vazio` | **true** | not filled counts as empty |
  | `e` / `ou` / `nao` | combine the results above | — |
  | `funcao` | the `def` gets the data as it is and decides | — |

- Canonical text (`canonical_text`): `igual` compares canonical text without the outer spaces.
  Booleans accept `sim`/`nao` and `true`/`false`/`1`/`0`. A field without a single text (a list)
  is false, so `diferente` is true. An attachment object's text is its name. Numbers are written
  without trailing zeros (`123`, `12.5`).
- `maior` / `menor` compare numbers. Text that is a number compares by its number (`"12"` is greater
  than `"9"`). Text that is not a number, a list, an object or a boolean gives false (the publish
  warning FLX60). A literal that is not a number is `regra_invalida`: the rule could never be true.
- `contem` ignores case and accents. On a list, it is true when any item contains the text.
- `preenchido` (null, text of only spaces and an empty list do not count as filled) is the same
  rule as `vazio`. The engine uses it for an action's `exige` and a field's `obrigatorio`, so the
  two cannot diverge.
- Time and size limits, all in the source as constants: 10,000 ticks (§3.4), 1 MiB of heap, call
  depth 128, nesting depth 32 (an IR written by hand that is deeper gives a typed error, not a
  crash), deadline 200 ms plus a 25 ms tolerance. The Starlark deadline is not preemptive: a huge
  constant computed inside a `def` (`"x" * 1000000000`) is done when the script compiles, and
  Starlark notices the deadline only seconds later (about 4 s for 10⁸, 35 s for 10⁹). So the `def`
  runs in its own thread, the caller waits at most the deadline plus the tolerance, and a rule that
  passes the limit answers `regra_limite` at once. The abandoned thread ends by itself.
- Loading the script follows the same rule: its own thread and a 1 s deadline
  (`LOAD_DEADLINE`) plus the margin. Without it, a published revision like that would stall the
  attendance turn.
- Starlark has no error type for ticks, heap or deadline, so a load or call error counts as a
  limit when its message says so (covered by a test). Any other error is `regra_invalida`.
- Two kinds of error, and both say who is at fault. `regra_invalida` is a defect of the flow's
  author: `funcao` without a `def`, a `def` that returned text, a `def` that failed (missing key,
  division by zero), a literal that is not a number, a field written outside `etapa.campo`.
  `regra_limite` means the rule passed a tick, heap, call-depth or deadline limit. Neither stops the
  process or the turn: the engine (FX-13) marks the execution as `falha` (FLXR01–FLXR03) and the
  Ficha stays open.
- A malformed `etapa.campo` (no dot, empty step or field) is `regra_invalida`, not a missing field.
  The compiler (FLX12) already refuses this shape, but the IR can come from outside.
- `snapshot_from_json`: unreadable text or JSON that is not an object is a rule error (FLXR05 in the
  engine). Nothing is evaluated on half the data.

### 8.4 Script compilation (`compilador/form`)

- The script is read as a syntax tree and is never executed: there is no interpreter call and no allocation guided by script data. The only Starlark evaluation is loading the module (`language::load_module`), which runs after the shape is valid and the limits are set.
- Two passes, on purpose. **Collection** turns each declaration into a raw structure that keeps the position of each piece, and checks the local shape (unknown argument, wrong type, invalid id, combinator arity). **Closure** runs with every declaration known: it resolves destinations (`inicio`, `vai_para`, `entao`, `senao`), checks each rule against its step and field, detects decision cycles and emits the reach and use warnings. A decision may point to a step declared later, and a `def` may be used before it is defined; only the closure pass can know that.
- The IR is produced only when there is no error. Warnings do not block it.
- `FLX62` (step with an actor and no action for that actor) is reported only when every action in the step is restricted to an actor. An action without `ator` is run by any actor, so the warning would be false.
- The body of a `def` loses its trailing blank lines. The text of a `def` runs up to the next declaration and would otherwise carry the spacing after it. The IR must not depend on the file's formatting, so the canonical formatter can print the `def` again without that spacing.

### 8.5 Script compiler (`compilador`)

- `compile` is the entry point: it takes the `.star` text and returns the `Compiled` result
  (`schema_versao = 1`, `ir`) plus the shape diagnostics with line and column. `load` is the other
  side: it takes the IR stored in a revision and the text, recompiles and checks that both are the
  same — when they are not, it fails closed with `ir_divergente` instead of guessing which of the
  two versions is right (FLXR06).
- The path is: source size → parse in the dialect (§3) → tree read, which builds the IR and the
  shape diagnostics (`form`) → module load in Starlark with limits, only to prove that the script
  loads and that the limits are really enforced. The compiler is **pure**: no database, no network,
  no clock, no file. Two compilations of the same text produce the same IR, byte by byte. The tree
  read executes nothing from the script; the module load, which runs after it, evaluates the
  top-level declarations and is where the load limits apply.
- The limits of one compilation are fixed in this version (`FLUXOS_LINGUAGEM.md` §3.4) and exist
  as a parameter so a test can tighten them and prove they are enforced. `Limites::from_config` is
  the only point that turns the process `[fluxos]` configuration into compiler limits; fields
  without a key in `[fluxos]` (module load ticks and heap, callstack, deadline) stay at the default.
  Rule evaluation limits come from `RuleLimits::from_config`.
- Default source cap: 32 KB (`FLUXOS_LINGUAGEM.md` §3.4 and decision D1), the default of
  `fluxos.max_fonte_bytes`; it is checked before the parse, so an oversized script is refused
  without being read.
- The IR exists only when there is **no error**: a warning does not block publishing, an error
  does. Diagnostics come in the order in which they were found (errors and warnings together).
  The node cap of the configuration (`fluxos.max_nos`) is checked after the shape cap already
  enforced by the tree read; the process cap may be lower.
- `LoadFailure::code` returns the stable code used in log and API: `fluxo_invalido` when the text
  does not compile, `ir_divergente` when it compiles to something else.
- Parse errors: only the first line of the parser message goes into the API message, because the
  rest is the source snippet. Lines and columns are 1-based. Constructions outside the dialect
  (`load`, `lambda`, f-string, `for`, `if`, `while`, `class`, `try`) get their own code and a
  message naming the construction; any other parse error is a syntax diagnostic.
- Load errors: a message about ticks, heap, callstack, cancellation, stack overflow or recursion
  counts as a load limit (`FLX04`); a name that does not exist has its own code; anything else is
  a syntax diagnostic. The load limit only triggers on a script whose shape is already valid, so
  the only possible error there is a limit.

### 8.6 Revision diff (`diff`)

- The diff compares nodes by id, not by source order. Its deterministic output lists the flow header,
  stages in the new revision order with removals after them, decisions, ends and `def` blocks.
- A new or removed stage, decision, end or `def` is one change; its children are not repeated as
  individual changes. Effects of a new or removed stage or action are reported separately only when
  the node is reachable through an action or decision.
- `nenhum` means text-only change, `atendimentos_novos` affects only new attendances,
  `pede_dado_novo` means that a new value becomes necessary, `muda_rota` changes the execution path,
  and `dispara_efeito` adds or removes a customer message, team notice or transfer.
- For new or removed nodes, impact depends on reachability. Existing nodes use the impact of their
  own change. The module is pure; counts of attendances attached to a revision are added by the API.

### 8.7 Semantic validator (`validador`)

- `validar` checks an already compiled IR. Diagnostics use a node path because an external IR has no
  source line or column. The compiler performs the position-based checks over source text.
- The validator checks references, ids, options, actors, rules, variables, reachability, cycles and
  node limits in a deterministic order. It is pure: no database, network, clock or file access.
- `resolve_departments` preserves the first-reference order, trims department names and compares
  case-insensitively while keeping accents significant. Unknown or ambiguous names produce `FLX65`
  and a resolution with `departamento_id = None`; publication stores the resolution and the runtime
  effect fails closed at execution time.

### 8.8 Flow engine (`motor`)

- The engine is **pure**: it takes the compiled IR and the execution state that came from the
  database and returns the new state with the events the atomic write will store. There is no
  database, clock, network or `async`; the time and the `version` belong to the caller. The same
  input gives the same output, as many times as it runs.
- It describes, it does not persist: transaction, `version`, isolation between the case and the
  flow, event write, closing the conversation and closing the case are not its job. Reaching a
  `fim(encerra_atendimento=True)` only **asks** for the close (`Resultado::encerra_atendimento`);
  the caller closes in the same transaction. Delivering an effect is also not its job: it returns
  `ProducedEffect` with the idempotency key, and the queue does the rest.
- `execute_action` checks in this order: execution active → idempotency key already applied →
  execution halted on a decision (resolved first) → current node is a step → the action exists in
  the step (otherwise the error carries the list of valid ones) → the actor acts in this step →
  the action is not restricted to another actor → every informed field exists with the right
  type/option → every field of `exige` is filled → store the fields, decide the advance, produce
  the effects and follow the destination. Any refusal leaves the state untouched; the exception is
  a **defective** flow (decision cycle, jump cap, rule that does not evaluate), which returns `Ok`
  with the execution marked `falha`, because the caller must persist that outcome.
- The automatic advance is derived from the **actions**, never from the step's actor list: an
  action restricted to an actor that leads forward (step or decision) requires that actor; an
  unrestricted action requires nobody; an action that goes to `fim(...)` requires nobody and is
  decisive (it reaches the outcome even with another actor pending); an actor that only appears in
  the step list does not block the step; the client is never an action actor. While someone is
  missing, the step stays `parcial` and `waiting` names who.
- Each accepted mutation with an idempotency key stores a `Receipt` in the state. Repeating the
  key returns `repeated = true` with the same outcome and **no** new event; in the database the
  guarantee is the unique index on the idempotency key.
- Effects come out in the declared order, with the text resolved and the key `node + index`
  (`etapa:<id>/0`, `acao:<step>.<action>/0`), without the execution id. Two visits to the **same**
  step (a flow that loops back) reuse the key, so its `ao_entrar` is not enqueued twice: one
  message per effect, and a loop with `mensagem_ao_cliente` in `ao_entrar` reaches the customer
  once.
- Interpolation applies to text only: `{step.field}`, `{ficha.id}` and `{client.peer}`. A variable
  that does not exist becomes empty text (the publish warning FLX59 says so), and an unmatched `{`
  stays as it is. The `para` of `notificar_whatsapp` and the department of `encaminhar` are
  identifiers and are not interpolated.
- Field values are validated strictly against the declared type (JSON number, JSON boolean, one of
  the declared option ids) and text is stored without the outer spaces. `null` means "not filled":
  accepted in an optional field, refused in a required one. `exige` is checked **after** storing
  this call's fields and together with what the step already had, so the agent may inform the data
  in one call and ask for the action that requires it in the next.
- Field errors come out in alphabetical order **on purpose**, not in map order: in the workspace
  build `serde_json` is compiled with `preserve_order` (because of the server's `utoipa`), while
  standalone it sorts by key. The message the agent reads cannot depend on which build ran.
- Failure causes are classified. Cycle of decisions, jump cap and a rule that does not evaluate all
  leave the execution `falha` with the reason in the state; only the failed rule sets
  `rule_error`, which feeds the `poph_fluxos_erros_regra_total` metric (FX-40), because the database
  mirror cannot tell the cause from the reason text.
- Known limits: the ISO date check validates shape and ranges (month 1–12, day 1–31, hour/minute/
  second), not the calendar (`2026-02-30` passes); `dados[step]` that is not an object is treated as
  a step without fields (the engine neither fixes nor accuses data that is not its own); who may
  abandon an execution (agent or attendant) is the caller's rule, and the engine only records who
  did it.

### 8.9 Canonical formatter (`formatter`)

- Entry points: `format(source)` and `format_with(source, limits)` compile the text (FX-10) and
  reprint the structure; `print(&ir)` goes the other way (IR → script). All three are **pure**: no
  database, clock, network or file. A script that does not compile returns `FormatFailure` with its
  diagnostics and **never partial text** ("it formatted as far as it could" does not exist); a
  warning does not block formatting. The same IR gives the same text byte for byte.
- One declaration per block, separated by one blank line and with one trailing newline, in the order
  **`fluxo`, `def`, `etapa`, `decisao`**. Every `def` goes before its users — the `funcao("name")`
  of a decision and the calls between `def`s only resolve upward — so a `def` written at the end of
  the source moves up, and this is the one rule that supersedes the older "def at the end" line in
  [FLUXOS_LINGUAGEM.md](FLUXOS_LINGUAGEM.md) §9. Stage and decision order follows the IR.
- Inside a top-level declaration, one argument per line with a 4-space indent. A nested call fits on
  one line when it fits in `WIDTH` (112 columns) and opens otherwise. Three deliberate exceptions:
  `campos`, `acoes` and `ao_entrar` of a stage are **always** open (they are the stage structure,
  one item per line, and that is what makes a review diff readable; `efeitos` of an action does not
  follow it and fits on the line when short); the composed combinators `e`, `ou` and `nao` are
  always open (a composed rule reads as a list); and text is never split, because breaking a literal
  would change the value.
- Optional arguments at their default are not written: without `atores` the stage is `["agente"]`,
  without `obrigatorio` the field is optional, without `opcoes` the field is not `opcao`, and an
  empty list is not written. The three examples under `tests/fluxos/exemplos/` fit in `WIDTH` and
  come back identical once their `def`s are not out of place.
- Comments: one **attached** to a declaration — the block of `#` lines whose last line is
  immediately above the declaration — is preserved and reprinted left-aligned above it. A **loose**
  comment (inside a declaration, at the end of a line, at the end of the file, or separated from the
  declaration by a blank line) is discarded, so the canonical text is a fixed point after the first
  pass. A comment inside a `def` comes for free: the IR stores the `def` text literally.
- Known limit: the `fim(...)` order in the IR follows the first appearance. Grouping `etapa` before
  `decisao` reorders two `fins` when a decision declares a `fim(...)` before a stage declares
  another. The flow content is the same (same `inicio`, stages, decisions and ends); only the `fins`
  list order changes, and it is the single case where `load` (FX-10) would report divergence after
  formatting. Avoiding it needs an IR that keeps the position of each declaration, which is a
  contract change.

## Medidas

**Máquina:** Fedora 44, x86_64, AMD Ryzen 5 5600GT (6 núcleos, 12 threads), 32 GB de RAM.
**Data:** 23/11/2026, entre 19:15 e 19:25 (UTC-03:00).
**Estação compartilhada:** outros builds rodavam ao mesmo tempo (`load average` entre 5 e 7 nos
momentos das medidas). Por isso toda comparação antes/depois foi feita **intercalada**, na mesma
condição de carga: os números absolutos de hoje são menores que os da tabela de `DADOS.md`
(estação ociosa quando foram tomados), mas a comparação entre as duas versões não é afetada.

Todas as medidas usam binários `release`, `PATH` com `~/.cargo/bin` e `CARGO_INCREMENTAL=0`, em
`apps/pop-harness`.

### 1. RAM do processo com revisões carregadas no cache

Comando (o teto do cache vem de `POPH_RAM_TETO_MB` e cada valor roda num processo próprio, porque o
alocador não devolve ao sistema o que uma fase liberou):

```text
POPH_RAM_TETO_MB=4  cargo test --release -p poph-fluxos --test ram -- --ignored --nocapture
POPH_RAM_TETO_MB=16 cargo test --release -p poph-fluxos --test ram -- --ignored --nocapture
```

Revisões de 8 etapas com instruções de 200 caracteres: peso estimado pelo cache
(`entry_weight` = JSON × 2 + 256 B) **7.592 B** por revisão.

| Revisões carregadas | Teto de 4 MB (cache enche e expulsa) | Teto padrão de 16 MB (as 1.000 cabem) |
|---|---|---|
| 0 | 9.092 KB | 8.924 KB |
| 100 | 9.856 KB (+764 KB · **7.823 B por revisão**) | 9.688 KB (+764 KB · **7.823 B por revisão**) |
| 1.000 | 13.060 KB (+3.968 KB · 4.063 B por revisão) | 15.956 KB (+7.032 KB · **7.201 B por revisão**) |

O que o cache diz ter guardado em cada caso:

| Teto | Entradas | Expulsões | Não cacheadas | Bytes guardados |
|---|---|---|---|---|
| 4 MB | 551 | 449 | 0 | 4.187.604 B |
| 16 MB | 1.000 | 0 | 0 | 7.599.572 B |

Leitura: o RSS acompanha as revisões que **cabem** no teto e para de crescer quando o teto é
atingido (4 MB guardados → ~4 MB de RSS a mais, com 449 revisões já expulsas). Com o teto padrão,
1.000 revisões custam ~7 MB de RSS e ainda sobram ~9 MB de folga dentro dos 16 MB de
`fluxos.cache_revisoes_mb`.

**Contra a meta [proposta]:** a FX-01 propôs **≤ 300 KB por fluxo carregado**. A revisão medida
custa **7,2 a 7,8 KB** reais — ~2,6% da meta. Limite: o fluxo medido tem 8 etapas e a meta fala de
40 nós; extrapolando linearmente em número de nós (o que **não** foi medido), 40 nós ficariam em
~36–39 KB, ainda muito abaixo da meta.

### 2. Vazão de escrita de mensagens antes e depois da Ficha na conversa

Comando:

```text
cargo test --release -p poph-store --test throughput -- --ignored --nocapture
```

"Antes" é uma cópia isolada da árvore (worktree próprio em `/tmp`, alvo de build próprio) com **uma
única** diferença: a criação da Ficha removida de `Conversas::criar`. É o caminho de escrita de
antes da FX-07, montado à mão porque aquele estado não está no histórico deste repositório. As
rodadas foram intercaladas (antes, depois, antes, depois…), três de cada lado.

| Cenário | Antes (sem Ficha) | Depois (com Ficha) | Diferença |
|---|---|---|---|
| uma thread, sem lote | 19.232 · 22.460 · 23.349 msg/s | 23.243 · 22.355 · 19.986 msg/s | +0,8% (médias), faixas sobrepostas |
| 32 concorrentes, lote de 512 | 61.923 · 65.447 · 66.832 msg/s (média 64.834) | 62.991 · 64.639 · 57.075 msg/s (média 61.568) | **−5,0%** |
| 256 concorrentes, lote de 512 | 71.577 · 70.779 · 67.051 msg/s (média 69.802) | 67.707 · 61.271 · 65.527 msg/s (média 64.835) | **−7,1%** |
| chegada constante de 2.000 msg/s | p50 54–76 µs, p95 87–111 µs | p50 59–69 µs, p95 96–115 µs | p95 +4 a +9 µs |

**Regressão acima de 10%: nenhuma.** A maior diferença é −7,1% com 256 escritores, dentro da
dispersão das três rodadas e com as faixas sobrepostas (a rodada mais lenta de um lado é pior que a
mais rápida do outro). As duas versões ficam **acima** da meta de D-07 (≥ 50 mil msg/s) e **abaixo**
do p95 de 5 ms na chegada de 2.000 msg/s.

O que a Ficha custa de verdade é na **criação de conversa**, que é o caminho raro (uma vez por
atendimento, não por mensagem). Medida isolada, 200 amostras por rodada, três rodadas de cada lado,
no mesmo par de árvores:

| `Conversas::criar` | Rodadas (p50 / p95) | Vazão |
|---|---|---|
| Antes, sem Ficha | 84 / 156 µs · 81 / 125 µs · 79 / 126 µs | 10.160 a 11.465 conversas/s |
| Depois, com Ficha | 281 / 402 µs · 292 / 432 µs · 281 / 410 µs | 3.262 a 3.382 conversas/s |

Ou seja: **+~200 µs por conversa** (p50 ~80 → ~285 µs, cerca de 3,5×), pagos uma vez por
atendimento. Mensagem não consulta nem escreve Ficha, e é isso que a tabela acima mostra.

### 3. Latência da transição atômica (`aplicar_transicao`)

Comando:

```text
cargo test --release -p poph-store --test transicao_latencia -- --ignored --nocapture
```

Duas rodadas; cada uma com banco próprio e as amostras em série (uma thread). A transição é uma
escrita só no escritor do `poph-store`.

| Cenário | p50 | p95 | p99 | Vazão em série |
|---|---|---|---|---|
| na etapa (estado + 2 eventos + 1 efeito) | 211 · 219 µs | 248 · 344 µs | 346 · 428 µs | 4.244 a 4.614/s |
| avançando de etapa (estado + 3 eventos + linha da etapa nova) | 297 · 301 µs | 454 · 491 µs | 486 · 537 µs | 2.933 a 3.027/s |
| `fim` que encerra o atendimento (fecha conversa **e** Ficha) | 406 · 427 µs | 503 · 684 µs | 567 · 800 µs | 2.312 a 2.543/s |

Leitura: a transição mais cara (a que encerra o atendimento e fecha conversa e Ficha na mesma
escrita) fica em **~0,4 ms p50 e ~0,7 ms p95** — dentro do p95 de 5 ms que D-07 fixa para escrita, e
muito abaixo do custo de uma chamada ao modelo de texto. Não há meta específica para a transição no
PRD; estes são os primeiros números.

### 4. Custo do G9 no lote, com 20 fluxos

Comando:

```text
cargo test --release -p poph-jev --test g9_medicao -- --ignored --nocapture
```

Estado de um turno com base disponível e uma resposta do agente, para o lote ter mais de uma
pergunta (G3 e G8 também perguntam). Texto de gatilho de cada fluxo no máximo que a pergunta aceita
(400 caracteres), que é o pior caso. O tempo medido é o do Harness **antes** da chamada — montar o
estado, montar as perguntas e serializar o pedido (`FakeJev`, sem rede). Duas rodadas.

| Lote | Perguntas | p50 | p95 | Pedido serializado | Estado | Perguntas |
|---|---|---|---|---|---|---|
| sem fluxo liberado (G9 inaplicável) | 2 | 30 · 32 µs | 56 · 76 µs | 2.521 B | 444 B | 2.077 B |
| 20 fluxos, G9 ligado | 3 | 181 · 183 µs | 250 · 266 µs | **42.057 B** | **19.399 B** | **22.658 B** |
| 20 fluxos, G9 desligado pela flag | 2 | 88 µs | 113 · 115 µs | 21.476 B | 19.399 B | 2.077 B |

Leitura:

- o G9 no topo (20 fluxos com textos longos) acrescenta **uma pergunta**, **~22,7 KB de critérios** e
  **~19,4 KB de estado** ao pedido — o pedido vai de ~2,5 KB para ~42 KB, e o custo de montagem e
  serialização vai de ~30 µs para ~180 µs;
- o estado leva os 20 fluxos **mesmo com a flag do gate desligada** (21,5 KB de pedido): desligar o
  G9 economiza a pergunta, não o estado;
- o tamanho do pedido é o que vira token de entrada no Jev; quanto isso custa em dinheiro e latência
  é a medição com o Jev real (FX-37), que depende de autorização.

### Comparação com as metas e com D-07

| Meta | Número | Resultado |
|---|---|---|
| FX-01 [proposta]: RAM por fluxo carregado ≤ 300 KB | 300 KB | **atendida**: 7,2–7,8 KB por revisão de 8 etapas |
| D-07 [oficial desde 23/11/2026]: escrita de mensagens ≥ 50 mil msg/s | 50.000 | **atendida**: 57.075–66.832 msg/s com 32 e 256 escritores |
| D-07 [oficial desde 23/11/2026]: p95 da escrita ≤ 5 ms com 2.000 msg/s | 5 ms | **atendida**: 96–115 µs |
| `fluxos.cache_revisoes_mb` = 16 MB (PRD §9) | 16 MB | **respeitada**: 1.000 revisões cabem em 7,6 MB guardados; acima do teto, para de crescer |
| PRD §11: nenhuma chamada de rede dentro da transação | — | **confirmada**: 0,4–0,7 ms p95 na transição que fecha o atendimento |
| CI e resto de D-07 (12 KB por agente ocioso, 150 MB com 10 mil agentes, ARM64) | — | **não medido aqui** — continua pendente, como antes desta tarefa |

As duas metas de D-07 (≥ 50 mil msg/s e p95 ≤ 5 ms) passaram a ser **oficiais em 23/11/2026**, por
decisão do mantenedor tomada depois desta medição: a decisão oficializa o que já estava medido, e
nenhum número desta seção mudou com ela (D-31 em [DECISOES.md](DECISOES.md)). A meta da FX-01
(≤ 300 KB por fluxo carregado) **segue proposta**, e o resto de D-07 (12 KB por agente ocioso,
150 MB com 10 mil agentes, ARM64) continua **não medido**. A HR-55 (teste de carga em escala)
continua suspensa e **não** foi reaberta.

### O que não foi medido

- **G9 com o Jev real** (FX-37): tempo de resposta do provedor, acerto da escolha e custo em tokens.
  A FX-36 mede só o que o Harness gasta antes de chamar.
- **Turno completo**: tempo de ponta a ponta de uma mensagem do cliente até a resposta com uma etapa
  de fluxo, e o custo do bloco de etapa no prompt (FX-18/FX-23 já medem o bloco em caracteres).
- **Entrega de efeitos** (FX-21): vazão da fila e tempo de `mensagem_ao_cliente` no WhatsApp real.
- **ARM64 e carga em escala**: exigem a VM3 e a HR-55, com autorização.
- **Fluxo de 40 nós**: a medida de RAM usou um fluxo de 8 etapas; a comparação com a meta da FX-01
  para 40 nós é extrapolação linear, não medida.
