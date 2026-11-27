# Paridade com o POP Atende (FX-15)

O motor do Harness substitui o do POP Atende (`apps/backend/core/fichas/state_engine.py`). Este
documento fecha a conta: **cada linha da tabela do PRD §8** ("O que muda em relação ao POP Atende")
com o teste que a prova ou, quando não há como provar no crate puro, a razão.

A suíte que reúne os casos que **permanecem** é `crates/poph-fluxos/tests/parity.rs`. Cada caso
dela cita no comentário de onde veio no Python — o teste do POP Atende
(`apps/backend/tests/test_fichas/`) ou a função (`state_engine.py`, `conditionals.py`,
`validators.py`).

Comportamento que o PRD **removeu** não é reproduzido: os testes do Python que só existiam para
provar o que saiu (`completion` `any`/`all`, auto-transição por busca em largura, atores
`client`/`provider`, `next_step: "closed"` fechando a Ficha) ficaram de fora, e onde faz sentido a
própria suíte prova que a remoção vale — esses casos estão nomeados como tal.

## A tabela do PRD §8, linha por linha

| # | Linha do PRD §8 (antes → agora) | Teste que prova |
|---|---|---|
| 1 | Fluxo do **agente** (`linked_template_id`) / template da org com associação → **fluxo da org**, permissão por agente | `crates/poph-server/tests/agente_fluxos.rs::agente_fluxos_lista_substitui_com_versao_teto_isolamento_e_auditoria` (FX-28: lista de fluxos do agente, `If-Match`, teto de 20, isolamento entre orgs e auditoria) |
| 2 | Ficha só nasce com o fluxo → **Ficha sempre nasce com a conversa** | **Não testável no `poph-fluxos`**: o motor é puro e não cria Ficha. É a FX-20 (`poph-store`: Ficha aberta com a conversa, fluxo como sub-item). Justificativa registrada aqui até essa tarefa. |
| 3 | `next_step: "closed"` fecha a Ficha; estado `closing_pending` → `fim(...)` conclui **só a execução**; fechar atendimento é decisão explícita | `paridade::closed_do_pop_atende_virou_fim_que_conclui_a_execucao`, `paridade::fim_so_pede_o_fechamento_do_atendimento_quando_declara_isso`, `paridade::nao_existe_estado_closing_pending`, `paridade::execucao_encerrada_nao_aceita_acao` |
| 4 | Várias Fichas ativas por conversa (até 3 no prompt) → **uma execução ativa por Ficha** | **Não testável no `poph-fluxos`**: o motor não conhece Ficha nem concorrência; a garantia é o índice único parcial de `fluxo_execucoes` (FX-20, PRD L6). O tipo documenta a regra (`poph_types::StatusExecucao`). |
| 5 | `completion` `auto`/`any`/`all` → **só o comportamento `auto`** | `paridade::auto_fica_parcial_quando_falta_o_outro_ator_restrito` e `paridade::auto_avanca_quando_o_segundo_ator_restrito_age` (o comportamento `auto`); `paridade::auto_a_acao_que_leva_ao_fim_e_decisiva_com_outro_ator_pendente`. O critério **não é declarado** na IR (a etapa não tem campo de critério), então não existe como escolher `any`/`all`: `test_any_completion_advances_immediately` e `test_any_completion_keeps_first_action_wins` **não foram portados** (provam o que saiu). |
| 6 | Atores `client`, `attendant`, `agent`, `system`, `provider`, `operator` → `agente`, `atendente` (e `sistema` interno) | `paridade::so_existem_dois_atores_de_acao`, `paridade::ator_fora_dos_atores_da_etapa_e_recusado`, `paridade::acao_restrita_ao_atendente_recusada_para_o_agente`. `test_all_completion_closes_when_last_actor_confirms` (ator `client`), `test_provider_negotiation.py` e `test_provider_interactions.py` **não foram portados**: o ator não existe. |
| 7 | Auto-transição: o motor executa ações intermediárias por busca em largura → **removida**: erro com as ações válidas | `paridade::acao_inexistente_devolve_as_validas_e_nao_muda_nada` (devolve as válidas e não executa nada) e `paridade::auto_a_acao_que_leva_ao_fim_e_decisiva_com_outro_ator_pendente` (nenhuma ação de outro ator roda sozinha). O Python não tem teste dedicado à busca em largura: a origem é `state_engine.execute_action`. |
| 8 | Condicional: expressão `pop_expr` em texto, ou regra simples → **combinadores declarativos** (`igual`, `maior`, `contem`, `vazio`, `e`, `ou`, `nao`) **ou** `def` | `parity::conditional_with_operators_pop_atende_chooses_branch` (cada operador do `TestEvaluatePopExpr`, inclusive `>=` montado como `nao(menor(...))`), `parity::two_conditionals_chained_choose_step_final`, `parity::conditional_with_def_chooses_branch`. Diferença de erro: `test_unknown_variable_raises` levantava `Unknown conditional variable`; aqui campo ausente tem semântica fixa por combinador (§6.1 da linguagem) e o defeito real vira **execução em `falha`** — `parity::conditional_to_missing_step_fails_closed`, mais os avisos `FLX21`/`FLX22` de `tests/compiler.rs`. |
| 9 | Notificação e encaminhamento eram nós "satélite" ligados por arestas → **efeitos** dentro da ação ou do ramo | `paridade::notificacao_e_encaminhamento_saem_como_efeito_da_acao` e `paridade::mensagem_de_entrada_da_etapa_sai_ao_iniciar_a_execucao` (o `messages` da etapa). Estrutura do "nó `forward`" do Python (`test_department_forward.py`) **não foi portada**: ela é validação de template, e o encaminhamento virou efeito (`Efeito::Encaminhar`). |
| 10 | Variável inexistente na mensagem vira vazio, **sem aviso** → vira vazio **e** avisa ao publicar | `parity::missing_variable_in_text_renders_empty` (o texto sai com o trecho vazio) + `FLX59` em `tests/compiler.rs` (o aviso na publicação, dentro de `cases()`, exigido por `each_code_list_closed_has_case`). |
| 11 | Seleção do fluxo pelo Jev, fallback LLM → **igual (G9)**, mas o **código inicia** a execução antes do modelo | **Não testável no `poph-fluxos`**: o gate G9 é a FX-22 e vive no turno. O motor só recebe `iniciar(ir)`, que não tem Jev nem modelo. |
| 12 | Etapa inicial marcada por `is_entry` → **`inicio = "id"`** no `fluxo(...)` | `paridade::inicio_e_declarado_no_fluxo_e_nao_marcado_na_etapa` (a entrada é o `inicio` declarado, não a primeira etapa da lista) |
| 13 | Executor/atribuição, calendário, PDF, prestadores → **fora** | **Fora do escopo** (PRD §4). A lista de casos do Python que ficaram sem equivalente está na seção "Sem equivalente" abaixo. |

## Outras diferenças observadas (fora da tabela do §8)

Diferenças que apareceram ao portar os casos, todas intencionais:

| Diferença | Teste |
|---|---|
| O POP Atende descobria a etapa de fechamento **pelo nome** (`find_completion_step_key`: "Encerrado", "Fechamento", "FIM"); aqui o encerramento é declarado no `fim(encerra_atendimento = True)` | `paridade::o_encerramento_e_declarado_e_nao_deduzido_pelo_nome_da_etapa` |
| Condicional que aponta para etapa inexistente: o Python ficava parado no nó; aqui o motor **falha fechado** (`ir_divergente`) | `parity::conditional_to_missing_step_fails_closed` |
| `max_length` de texto e `min`/`max` de número não existem na linguagem: o tipo é conferido, os limites não | `paridade::campo_de_texto_recusa_numero`, `paridade::campo_de_numero_recusa_texto` |
| `data` confere **forma e faixas** do ISO 8601, não o calendário: `2026-02-30` passa. O `datetime.fromisoformat` do Python recusaria | **Sem teste próprio** (é limite conhecido do motor, documentado em `docs/FLUXOS.md` §8.8). Quem consome a data valida o calendário. |
| O estado global da Ficha (`open`, `awaiting_executor`, `closing_pending`, `closed`) virou `RecordStatus` (`open`/`closed`) + `ExecutionStatus` (`ativa`, `concluida`, `abandonada`, `falha`). `awaiting_executor` não existe: atribuição de executor está fora | `paridade::nao_existe_estado_closing_pending` (a lista fechada de situações de execução). `test_awaiting_executor_state`, `test_open_state_with_executor` e `test_derive_global_state` **não foram portados**: falam de atribuição de executor, que saiu do escopo. |
| `pending_client_interaction` (a etapa que espera resposta do cliente) não existe: o cliente não é ator | Sem equivalente (ver abaixo) |

## Comportamento portado: onde está cada caso

| Comportamento que permanece | Casos em `parity.rs` |
|---|---|
| Avanço `auto` | `auto_avanca_quando_o_unico_ator_exigido_e_quem_agiu`, `auto_fica_parcial_quando_falta_o_outro_ator_restrito`, `auto_avanca_quando_o_segundo_ator_restrito_age`, `auto_os_dois_atores_chegam_ao_mesmo_destino_em_qualquer_ordem`, `auto_ator_sem_acao_propria_nao_trava_a_etapa`, `auto_a_acao_que_leva_ao_fim_e_decisiva_com_outro_ator_pendente` |
| Ação restrita por ator | `acao_restrita_ao_atendente_recusada_para_o_agente`, `ator_fora_dos_atores_da_etapa_e_recusado`, `acao_inexistente_devolve_as_validas_e_nao_muda_nada`, `execucao_encerrada_nao_aceita_acao` |
| Encadeamento de condicionais | `condicional_com_os_operadores_do_pop_atende_escolhe_o_ramo`, `duas_condicionais_encadeadas_escolhem_a_etapa_final`, `condicional_com_def_escolhe_o_ramo`, `condicional_que_vai_para_etapa_inexistente_falha_fechado` |
| Campos obrigatórios | `campo_obrigatorio_faltando_devolve_a_lista` |
| Tipos de campo | `campo_de_texto_recusa_numero`, `campo_de_numero_recusa_texto`, `campo_de_opcao_recusa_valor_fora_da_lista`, `campo_de_data_aceita_iso_e_recusa_texto_livre` |
| `closed` → `fim` | `closed_do_pop_atende_virou_fim_que_conclui_a_execucao`, `fim_so_pede_o_fechamento_do_atendimento_quando_declara_isso`, `o_encerramento_e_declarado_e_nao_deduzido_pelo_nome_da_etapa` |
| Atores | `so_existem_dois_atores_de_acao` |
| Efeitos | `notificacao_e_encaminhamento_saem_como_efeito_da_acao`, `mensagem_de_entrada_da_etapa_sai_ao_iniciar_a_execucao` |
| Variável inexistente | `variavel_inexistente_no_texto_sai_vazia` |
| Início do fluxo | `inicio_e_declarado_no_fluxo_e_nao_marcado_na_etapa` |

## Sem equivalente: o que ficou fora e por quê

Casos da suíte do POP Atende que **não têm** equivalente no Harness, porque o assunto está fora do
escopo do módulo (PRD §4). Não são falha: são escopo menor, e voltam por especificação própria.

| Assunto | Casos/funções do Python | Por que não tem equivalente |
|---|---|---|
| Executor e atribuição | `test_executor_selection.py`, `test_executor_sync.py`, `test_awaiting_executor_state`, `test_open_state_with_executor` | Atribuir um executor à Ficha está fora (PRD §4); o Harness não tem executor por Ficha. |
| Calendário | `test_calendar.py`, `test_calendar_sync_config.py`, `test_scheduled_transitions.py`, `test_calendar_action_uses_current_step_definition`, `_extract_event_date*` | Integração CalDAV está fora; `calendar_action` não existe na linguagem. |
| PDF e documentos | `documents`/`messages` do template, `test_seed_*` que montam documento | Geração de documento está fora (módulo de documentos é outro). |
| Prestadores | `test_provider_interactions.py`, `test_provider_negotiation.py`, `test_all_completion_closes_when_last_actor_confirms` | Atores `provider` e `client` não existem no Harness (PRD §8, linha dos atores; decisão D5). |
| Espera pelo cliente | `pending_client_interaction_json`, `test_all_completion_closes_when_last_actor_confirms` | O cliente conversa pelo canal, não executa ação de etapa. |
| CRUD de template e API | `test_api.py`, `test_template_crud.py`, `test_ficha_service.py`, `test_tools.py` | A API de fluxos do Harness é outra (`/admin/v1/orgs/{org}/fluxos`, FX-27/FX-32) e a edição é por script (FX-16). |
| Gravação de evento e fila | `test_events.py`, `test_outbox.py`, `test_matrix_projection.py` | O motor devolve os eventos; gravar, enfileirar e entregar é do `poph-store` (FX-20/FX-21). |
