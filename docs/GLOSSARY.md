# Glossary: Portuguese terms and their English names

Most of the technical documentation in this folder is still written in Portuguese, and so is part of the code
(tables, CLI commands, crate names). The English documentation (`README.md`, `CONTRIBUTING.md`,
`ROADMAP.md`, `AGENTS.md`, `CHANGELOG.md`) uses the English names below. Published names are **not** renamed: a
*Case* is stored in the `fichas` table and listed with `poph ficha`.

| English (use in English text) | Portuguese (docs and code) | What it is |
|---|---|---|
| **Case** | **Ficha** | The record opened with each conversation. It accumulates messages, turns, flow executions and events, and closes with the conversation. A new customer message after the close opens another conversation and another Case. A closed Case never changes. |
| Flow | Fluxo | A plain-text Starlark script, owned by an org, with immutable revisions. An agent can only start flows that were granted to it. |
| Flow revision | Revisão de fluxo | One published, immutable version of a flow. |
| Flow execution | Execução de fluxo | One run of a flow revision inside a Case. |
| Turn | Turno | One cycle from an incoming customer message to the reply (see `docs/TURNO.md`). |
| Org | Org / organização | A tenant. Data and settings never cross orgs. |
| Handoff | Transferência para humano | Passing a conversation to a person, through Matrix/Element. |
| Knowledge base | Base de conhecimento | Documents an agent searches before answering. |

When you write English text about a Case, use *Case*, not *Ficha*, *ticket* or *file*. When you point at a table,
route, command or file name, keep the Portuguese identifier in code formatting (`fichas`, `poph ficha`,
`crates/poph-store/src/repos/fichas.rs`).

## Identifier terms: Portuguese to English

The table below is the single source for the identifier refactor: the rename tool (RF-09) reads the
left column and the name gate (RF-05) derives its dictionary from it. It is **measured**, not
invented: `scripts/glossary-terms.py --medir` counts every term split out of an identifier in the
code, ignoring strings and comments, and the table lists the Portuguese terms found that way with
their current count.

Two decisions per term:

- **`traduz`** — the term names code (a type, function, field, variant, constant, module, CSS class,
  test name). Its English form is the one to use when the module is renamed.
- **`congelado`** — the term appears inside a **published name**, which does not change: the 41
  words of the flow language, table/column/index/trigger names, routes, `operationId`s, JSON fields,
  `poph.toml` keys, CLI commands and flags, environment variables, `poph_*` metrics, plugin protocol
  terms and backup format keys. Also frozen is the Rust function that registers a flow word: it gets
  an English name and the explicit registration name (`GlobalsBuilder::set("fluxo", …)`).

A term with both roles has two rows — `mensagem` is renamed in code, `mensagem_ao_cliente` is not.
The `onde` column says where the term occurs in code (top source areas) or which published names
carry it; `ocorrências` is the measured count, `-` for frozen names, which are not counted.

Regenerate with `python3 scripts/glossary-terms.py --gerar`; check with `--validar` (the same command
`scripts/ci.sh` runs). Add a missing term to `scripts/glossary-map.txt`, one `português<TAB>inglês`
per line. Terms below the current cut-off are listed by `--medir --candidatos` for the next round.

<!-- glossario:tabela:inicio -->
| termo_pt | termo_en | decisão | onde | ocorrências |
| --- | --- | --- | --- | --- |
| texto | text | traduz | poph-agent, poph-flows | 2270 |
| agente | agent | traduz | poph-agent, console | 1400 |
| conversa | conversation | traduz | poph-agent, poph-server | 1335 |
| estado | state | traduz | poph-flows, poph-jev | 1274 |
| nome | name | traduz | poph-flows, console | 1270 |
| resultado | result | traduz | poph-store, poph-flows | 1081 |
| fluxo | flow | traduz | poph-agent, poph-server | 944 |
| tipo | kind | traduz | poph-store, poph-server | 711 |
| campo | field | traduz | poph-flows, console | 705 |
| saida | output | traduz | poph, poph-agent | 689 |
| mensagem | message | traduz | poph-agent, poph-store | 686 |
| servidor | server | traduz | console, poph-server | 648 |
| turno | turn | traduz | poph-agent, console | 639 |
| evento | event | traduz | console, poph-agent | 603 |
| etapa | step | traduz | poph-flows, poph-agent | 565 |
| dados | data | traduz | poph-flows, console | 537 |
| chave | key | traduz | console, poph-flows | 531 |
| registro | record | traduz | poph-agent, console | 530 |
| motivo | reason | traduz | poph-agent, poph-jev | 514 |
| gatilho | trigger | traduz | poph-agent, poph-flows | 461 |
| acao | action | traduz | poph-flows, poph-agent | 390 |
| decisao | decision | traduz | poph-jev, poph-flows | 376 |
| fonte | source | traduz | poph-flows, poph-store | 350 |
| ficha | case | traduz | poph-server, poph-agent | 340 |
| fim | end | traduz | poph-flows, poph-agent | 302 |
| provedor | provider | traduz | poph-agent, poph-server | 298 |
| situacao | situation | traduz | console, poph-retention | 282 |
| departamento | department | traduz | poph-agent, console | 279 |
| integracao | integration | traduz | poph-server, console | 278 |
| versao | version | traduz | poph-server, console | 263 |
| whatsapp | whatsapp | traduz | console, poph-server | 258 |
| desligar | disable | traduz | poph-server, poph | 250 |
| canal | channel | traduz | poph-store, poph-server | 246 |
| limites | limits | traduz | poph-flows, poph-agent | 235 |
| numero | number | traduz | console, poph-store | 224 |
| modelo | model | traduz | poph-agent, poph-core | 211 |
| campos | fields | traduz | poph-flows, console | 193 |
| de | of | traduz | poph-store, scripts | 193 |
| descricao | description | traduz | console, poph-agent | 183 |
| em | in | traduz | poph-store, console | 183 |
| ator | actor | traduz | poph-store, poph-server | 180 |
| externo | external | traduz | poph-agent, poph-server | 176 |
| resolve | resolves | traduz | poph, scripts | 168 |
| atual | current | traduz | poph-flows, poph-store | 165 |
| configuracao | configuration | traduz | console, poph | 143 |
| validar | validate | traduz | poph-types, poph-server | 142 |
| instancia | instance | traduz | poph-server, poph-evolution | 139 |
| para | for | traduz | poph-flows, console | 112 |
| com | with | traduz | poph-flows, poph-agent | 107 |
| endereco | address | traduz | poph-server, poph | 100 |
| nivel | level | traduz | poph, console | 96 |
| segredos | secrets | traduz | console, poph | 94 |
| fluxos | flows | traduz | poph-agent, poph-store | 87 |
| ordem | order | traduz | console, poph-store | 83 |
| porta | port | traduz | poph-server, poph | 82 |
| conversas | conversations | traduz | poph-server, poph-store | 80 |
| manifesto | manifest | traduz | console, poph-server | 75 |
| vazio | empty | traduz | poph-flows, poph-agent | 72 |
| idempotencia | idempotency | traduz | poph-store, poph-server | 65 |
| execucao | execution | traduz | poph-store, poph-server | 60 |
| tentativas | attempts | traduz | poph-agent, poph-server | 59 |
| espaco | space | traduz | console, poph-server | 58 |
| fechada | closed | traduz | poph-store, poph-server | 58 |
| igual | equal | traduz | poph-flows, raiz | 58 |
| por | by | traduz | poph-store, poph-agent | 58 |
| anexos | attachments | traduz | poph-server, poph-agent | 57 |
| ao | to | traduz | poph-flows, poph-matrix | 53 |
| ativo | active | traduz | poph-agent, console | 53 |
| erro | error | traduz | poph-store, poph-server | 48 |
| contem | contains | traduz | poph-flows, poph-files | 47 |
| ultimo | last | traduz | poph-store, poph-server | 47 |
| criar | create | traduz | poph-agent, poph-server | 46 |
| dias | days | traduz | console, poph-retention | 46 |
| diretorio | directory | traduz | poph-rag, poph-server | 43 |
| funcao | function | traduz | poph-flows, raiz | 40 |
| temperatura | temperature | traduz | poph-core, poph-store | 38 |
| fichas | cases | traduz | poph-store, poph-server | 37 |
| formato | format | traduz | poph-store, poph-files | 37 |
| globais | globals | traduz | poph-server, poph-types | 36 |
| revisao | revision | traduz | poph-store, poph-server | 36 |
| maior | bigger | traduz | poph-flows, poph-rag | 35 |
| definicao | definition | traduz | poph-store, poph-server | 33 |
| rascunho | draft | traduz | poph-store, poph-server | 33 |
| metadados | metadata | traduz | poph-files, poph-retention | 32 |
| instancias | instances | traduz | console | 28 |
| publicar | publish | traduz | poph-server, poph-agent | 28 |
| resolucoes | resolutions | traduz | poph-store, poph-server | 28 |
| atualizar | update | traduz | poph, console | 27 |
| encaminhar | forward | traduz | poph-flows, poph-server | 27 |
| nao | not | traduz | poph-flows, raiz | 26 |
| desfecho | outcome | traduz | poph-agent, poph-store | 25 |
| tabelas | tables | traduz | console, poph-store | 25 |
| listar | list | traduz | poph, poph-server | 24 |
| remetente | sender | traduz | console, poph-evolution | 24 |
| retentativas | retries | traduz | poph-jev, poph-core | 24 |
| liberar | release | traduz | console, poph-agent | 23 |
| retencao | retention | traduz | console, poph-store | 23 |
| final | final | traduz | poph-matrix, console | 22 |
| vai | goes | traduz | raiz | 22 |
| ou | or | traduz | poph-flows | 21 |
| efeitos | effects | traduz | poph-store, raiz | 20 |
| ligar | link | traduz | poph-server, poph | 20 |
| enviado | sent | traduz | poph-store, poph-agent | 19 |
| forcar | force | traduz | console | 19 |
| modo | mode | traduz | console | 19 |
| proxima | next | traduz | poph-store, poph-server | 18 |
| segredo | secret | traduz | console | 18 |
| tentativa | attempt | traduz | poph-store, poph-server | 18 |
| despedida | farewell | traduz | console | 17 |
| diferente | different | traduz | poph-flows | 17 |
| acoes | actions | traduz | raiz | 16 |
| antes | before | traduz | console | 16 |
| depois | after | traduz | console | 16 |
| instrucao | instruction | traduz | raiz | 16 |
| menor | smaller | traduz | poph-flows | 15 |
| execucoes | executions | traduz | poph-store | 14 |
| mostrar | show | traduz | poph | 14 |
| origem | origin | traduz | poph-store, console | 14 |
| aberta | open | traduz | poph-store, poph-server | 12 |
| argumentos | arguments | traduz | console | 12 |
| obrigatorio | required | traduz | raiz | 12 |
| quando | when | traduz | raiz | 12 |
| categoria | category | traduz | poph, poph-server | 11 |
| exemplos | examples | traduz | raiz, console | 11 |
| falha | failure | traduz | console, poph-agent | 11 |
| publicada | published | traduz | poph-store, poph-server | 11 |
| situacoes | situations | traduz | console | 11 |
| arquivo | file | traduz | console, poph-server | 10 |
| escolha | choice | traduz | console | 10 |
| exige | requires | traduz | raiz | 10 |
| finalizada | finalised | traduz | poph-store, poph-agent | 10 |
| ja | already | traduz | poph-server, poph | 10 |
| tem | has | traduz | console | 10 |
| configurado | configured | traduz | console | 9 |
| extrator | extractor | traduz | poph-files, scripts | 9 |
| mudou | changed | traduz | console | 9 |
| que | that | traduz | console, poph-store | 9 |
| recusa | refusal | traduz | poph-core, poph-server | 9 |
| adicionadas | added | traduz | console | 8 |
| duracao | duration | traduz | console | 8 |
| escrever | write | traduz | console, poph-agent | 8 |
| nos | us | traduz | poph-flows, raiz | 8 |
| preenchidos | filled | traduz | console | 8 |
| removidas | removed | traduz | console | 8 |
| revisoes | revisions | traduz | poph-store, raiz | 8 |
| sobrescrever | overwrite | traduz | console, poph-store | 8 |
| trocaram | swapped | traduz | console | 8 |
| confianca | confidence | traduz | console, raiz | 7 |
| inicio | beginning | traduz | raiz, poph-server | 7 |
| itens | items | traduz | console, poph | 7 |
| remoto | remote | traduz | poph | 7 |
| rodada | round | traduz | console | 7 |
| desconectar | disconnect | traduz | poph-server, console | 6 |
| excluir | delete | traduz | poph | 6 |
| opcoes | options | traduz | raiz | 6 |
| orientacao | guidance | traduz | raiz | 6 |
| problemas | problems | traduz | console | 6 |
| resposta | response | traduz | console, poph-server | 6 |
| resumo | summary | traduz | raiz | 6 |
| saltos | jumps | traduz | poph-flows, raiz | 6 |
| trechos | passages | traduz | console | 6 |
| decisoes | decisions | traduz | console | 5 |
| eventos | events | traduz | poph-store | 5 |
| falhas | failures | traduz | poph-store | 5 |
| fila | queue | traduz | console, poph-store | 5 |
| grupo | group | traduz | console | 5 |
| midia | media | traduz | poph-store | 5 |
| preco | price | traduz | console | 5 |
| produto | product | traduz | console, poph-server | 5 |
| simultaneos | simultaneous | traduz | raiz, poph-agent | 5 |
| chamada | call | traduz | console | 4 |
| desde | since | traduz | poph | 4 |
| entrou | entered | traduz | poph-store, poph-server | 4 |
| formatar | format | traduz | console | 4 |
| margem | margin | traduz | console | 4 |
| palavra | word | traduz | raiz | 4 |
| perguntas | questions | traduz | console | 4 |
| reindexar | reindex | traduz | console, poph | 4 |
| segura | safe | traduz | console | 4 |
| titulo | title | traduz | console | 4 |
| agentes | agents | traduz | console | 3 |
| atualizadas | updated | traduz | console | 3 |
| centavos | cents | traduz | console | 3 |
| cliente | client | traduz | raiz, poph-flows | 3 |
| da | of | traduz | poph-agent | 3 |
| decididas | decided | traduz | console | 3 |
| etapas | steps | traduz | poph-store | 3 |
| ferramenta | tool | traduz | console | 3 |
| inseridas | inserted | traduz | console | 3 |
| linhas | lines | traduz | console | 3 |
| montagem | assembly | traduz | console | 3 |
| notificar | notify | traduz | raiz, poph-flows | 3 |
| primeiro | first | traduz | console | 3 |
| produtos | products | traduz | console | 3 |
| relevancia | relevance | traduz | console | 3 |
| retirar | withdraw | traduz | poph | 3 |
| rodadas | rounds | traduz | console | 3 |
| segundos | seconds | traduz | raiz | 3 |
| sem | without | traduz | poph-store, console | 3 |
| similaridade | similarity | traduz | console | 3 |
| vincular | link | traduz | poph | 3 |
| atendimento | service | traduz | raiz | 2 |
| ativa | active | traduz | poph-store, raiz | 2 |
| atraso | delay | traduz | poph-server, raiz | 2 |
| aviso | notice | traduz | console | 2 |
| condicoes | conditions | traduz | console | 2 |
| conhecimento | knowledge | traduz | console | 2 |
| encerra | closes | traduz | raiz | 2 |
| entao | then | traduz | raiz | 2 |
| entrada | entry | traduz | console | 2 |
| entrar | enter | traduz | raiz | 2 |
| indice | index | traduz | console | 2 |
| indisponivel | unavailable | traduz | console | 2 |
| informado | informed | traduz | console | 2 |
| problema | problem | traduz | raiz | 2 |
| regra | rule | traduz | raiz | 2 |
| senao | else | traduz | raiz | 2 |
| turnos | turns | traduz | raiz | 2 |
| unidade | unit | traduz | console | 2 |
| agiram | acted | traduz | poph-store | 1 |
| alterados | changed | traduz | console | 1 |
| atores | actors | traduz | poph-store | 1 |
| atualizada | updated | traduz | poph-store | 1 |
| avisos | notices | traduz | poph-store | 1 |
| carrinho | cart | traduz | console | 1 |
| carrinhos | carts | traduz | console | 1 |
| contato | contact | traduz | raiz | 1 |
| conteudo | content | traduz | console | 1 |
| corpo | body | traduz | poph-server | 1 |
| criacao | creation | traduz | poph-server | 1 |
| criada | created | traduz | console | 1 |
| criado | created | traduz | poph-server | 1 |
| desligado | disabled | traduz | poph-server | 1 |
| espacos | spaces | traduz | poph-server | 1 |
| espelhado | mirrored | traduz | poph-store | 1 |
| gerados | generated | traduz | console | 1 |
| gravados | stored | traduz | console | 1 |
| humano | human | traduz | poph-store | 1 |
| inalterados | unchanged | traduz | console | 1 |
| intervalo | interval | traduz | raiz | 1 |
| liberado | released | traduz | poph-store | 1 |
| lista | list | traduz | poph-server | 1 |
| metricas | metrics | traduz | poph-server | 1 |
| nomes | names | traduz | console | 1 |
| novos | new | traduz | console | 1 |
| outro | other | traduz | raiz | 1 |
| pareamento | pairing | traduz | poph-server | 1 |
| pendentes | pending | traduz | poph-store | 1 |
| publicacao | publication | traduz | poph-server | 1 |
| removido | removed | traduz | console | 1 |
| removidos | removed | traduz | console | 1 |
| risco | risk | traduz | raiz | 1 |
| saude | health | traduz | raiz | 1 |
| segunda | second | traduz | console | 1 |
| teste | test | traduz | poph-server | 1 |
| transferencia | transfer | traduz | poph-server | 1 |
| False | — | congelado | linguagem de fluxos | - |
| None | — | congelado | linguagem de fluxos | - |
| True | — | congelado | linguagem de fluxos | - |
| abaixo | — | congelado | metrica poph_fluxos_g9_abaixo_do_limiar_total | - |
| abandonar | — | congelado | operation_id fichas_abandonar | - |
| aberta | — | congelado | coluna aberta_em | - |
| abs | — | congelado | linguagem de fluxos | - |
| acao | — | congelado | campo_json acao; linguagem de fluxos; operation_id fichas_acao; e mais 1 | - |
| acertos | — | congelado | metrica poph_fluxos_cache_acertos_total | - |
| agente | — | congelado | flag_cli --agente; indice ix_agente_fluxos_fluxo; indice ix_fichas_org_agente; e mais 3 | - |
| agentes | — | congelado | metrica poph_agentes_carregados | - |
| agiram | — | congelado | coluna atores_que_agiram_json | - |
| all | — | congelado | linguagem de fluxos | - |
| anexo | — | congelado | operation_id teste_chat_anexo_conteudo; protocolo_plugin max_anexo_bytes | - |
| anexos | — | congelado | protocolo_plugin anexos | - |
| any | — | congelado | linguagem de fluxos | - |
| ao | — | congelado | variavel_ambiente POPH_FLUXOS_MAX_SALTOS_DECISAO; variavel_ambiente POPH_RETENCAO_DIAS | - |
| arquivo | — | congelado | schema_openapi ArquivoJson; schema_openapi ListaJson_ArquivoJson | - |
| arquivos | — | congelado | operation_id arquivos_conteudo; operation_id arquivos_criar; operation_id arquivos_excluir; e mais 2 | - |
| ativa | — | congelado | chave_toml handoff.orientacao_ativa; indice ux_fluxo_execucoes_ativa | - |
| ativas | — | congelado | metrica poph_conversas_ativas | - |
| ativo | — | congelado | chave_toml fluxos.ativo; chave_toml jev.ativo | - |
| ator | — | congelado | coluna ator_ref; coluna ator_tipo | - |
| atores | — | congelado | coluna atores_que_agiram_json | - |
| atraso | — | congelado | chave_toml turno.atraso_falha_modelo_ms; schema_openapi AtrasoJson | - |
| atual | — | congelado | coluna no_atual; coluna revisao_atual; indice ix_fluxo_execucoes_org_status_no_atual | - |
| atualizada | — | congelado | indice ix_conversations_atualizada | - |
| atualizar | — | congelado | operation_id atualizar; operation_id atualizar_item; operation_id categorias_atualizar; e mais 7 | - |
| avisos | — | congelado | tabela matrix_avisos_falha | - |
| bool | — | congelado | linguagem de fluxos | - |
| campo | — | congelado | linguagem de fluxos | - |
| campos | — | congelado | campo_json campos | - |
| canal | — | congelado | coluna canal; flag_cli --canal | - |
| cargas | — | congelado | metrica poph_fluxos_cache_cargas_total | - |
| categoria | — | congelado | schema_openapi AtualizarCategoria; schema_openapi CategoriaJson; schema_openapi CriarCategoria; e mais 1 | - |
| categorias | — | congelado | operation_id categorias_atualizar; operation_id categorias_criar; operation_id categorias_excluir; e mais 2 | - |
| chamadas | — | congelado | metrica poph_chamadas_evolution_total; metrica poph_chamadas_jev_total; metrica poph_chamadas_llm_total | - |
| chave | — | congelado | campo_json chave_idempotencia; chave_toml llm.chave_env; coluna chave; e mais 2 | - |
| com | — | congelado | backup com_conversas; flag_cli --com-conversas | - |
| confianca | — | congelado | chave_toml handoff.orientacao_confianca | - |
| configuracao | — | congelado | protocolo_plugin configuracao | - |
| contato | — | congelado | chave_toml handoff.orientacao_ingestao_contato_produto | - |
| contem | — | congelado | linguagem de fluxos | - |
| conteudo | — | congelado | operation_id arquivos_conteudo; operation_id teste_chat_anexo_conteudo | - |
| conversa | — | congelado | flag_cli --conversa; indice ux_fichas_conversa | - |
| conversas | — | congelado | backup com_conversas; flag_cli --com-conversas; metrica poph_conversas_ativas; e mais 5 | - |
| corpo | — | congelado | schema_openapi CorpoTransferencia | - |
| criacao | — | congelado | schema_openapi DadosCriacao | - |
| criado | — | congelado | schema_openapi TokenCriadoJson | - |
| criar | — | congelado | operation_id arquivos_criar; operation_id categorias_criar; operation_id criar; e mais 10 | - |
| dados | — | congelado | chave_toml dados; chave_toml dados.diretorio; chave_toml fluxos.max_dados_bytes; e mais 15 | - |
| de | — | congelado | flag_cli --de | - |
| decisao | — | congelado | chave_toml fluxos.max_saltos_decisao; linguagem de fluxos | - |
| definicao | — | congelado | coluna definicao_json | - |
| departamento | — | congelado | flag_cli --departamento; schema_openapi AtualizarDepartamento; schema_openapi CriarDepartamento; e mais 2 | - |
| departamentos | — | congelado | operation_id departamentos_atualizar; operation_id departamentos_criar; operation_id departamentos_excluir; e mais 2 | - |
| desconectar | — | congelado | operation_id desconectar | - |
| descricao | — | congelado | coluna descricao | - |
| desde | — | congelado | flag_cli --desde | - |
| desfecho | — | congelado | coluna desfecho | - |
| desligado | — | congelado | schema_openapi EspacoDesligadoJson | - |
| desligar | — | congelado | comando_cli poph matrix desligar; comando_cli poph matrix espaco desligar; operation_id matrix_espacos_desligar; e mais 1 | - |
| dias | — | congelado | chave_toml retencao.dias | - |
| dict | — | congelado | linguagem de fluxos | - |
| diferente | — | congelado | linguagem de fluxos | - |
| diretorio | — | congelado | chave_toml dados.diretorio | - |
| e | — | congelado | linguagem de fluxos | - |
| efeitos | — | congelado | indice ix_fluxo_efeitos_fila; indice ix_fluxo_efeitos_org; indice ux_fluxo_efeitos_chave; e mais 4 | - |
| em | — | congelado | backup exportado_em; coluna aberta_em; coluna concluiu_em; e mais 14 | - |
| encaminhar | — | congelado | linguagem de fluxos | - |
| endereco | — | congelado | chave_toml servidor.endereco; flag_cli --somente-endereco | - |
| entrou | — | congelado | coluna entrou_em | - |
| enumerate | — | congelado | linguagem de fluxos | - |
| enviado | — | congelado | coluna enviado_em | - |
| enviados | — | congelado | metrica poph_fluxos_efeitos_enviados_total | - |
| erro | — | congelado | coluna ultimo_erro; schema_openapi RespostaErro | - |
| erros | — | congelado | metrica poph_erros_evolution_total; metrica poph_erros_jev_total; metrica poph_erros_llm_total; e mais 2 | - |
| escrever | — | congelado | flag_cli --escrever | - |
| espaco | — | congelado | comando_cli poph matrix espaco; comando_cli poph matrix espaco desligar; flag_cli --espaco; e mais 3 | - |
| espacos | — | congelado | operation_id matrix_espacos_desligar; operation_id matrix_espacos_ligar; operation_id matrix_espacos_listar; e mais 1 | - |
| espelhado | — | congelado | coluna client_name_espelhado | - |
| esperas | — | congelado | metrica poph_fluxos_cache_esperas_total | - |
| estado | — | congelado | coluna estado; comando_cli poph matrix estado; operation_id estado; e mais 2 | - |
| etapa | — | congelado | linguagem de fluxos; metrica poph_latencia_etapa_segundos | - |
| etapas | — | congelado | indice ux_fluxo_execucao_etapas_ordem; tabela fluxo_execucao_etapas | - |
| evento | — | congelado | indice ux_matrix_midia_evento; protocolo_plugin evento | - |
| eventos | — | congelado | indice ux_ficha_eventos_idem; indice ux_ficha_eventos_seq; operation_id fichas_eventos; e mais 1 | - |
| excluir | — | congelado | operation_id arquivos_excluir; operation_id categorias_excluir; operation_id departamentos_excluir; e mais 5 | - |
| execucao | — | congelado | coluna execucao_id; indice ux_fluxo_execucao_etapas_ordem; tabela fluxo_execucao_etapas | - |
| execucoes | — | congelado | indice ix_fluxo_execucoes_ficha; indice ix_fluxo_execucoes_org_status_no_atual; indice ix_fluxo_execucoes_revisao; e mais 7 | - |
| exportar | — | congelado | operation_id backup_exportar | - |
| expulsoes | — | congelado | metrica poph_fluxos_expulsoes_total | - |
| externo | — | congelado | coluna peer_externo; protocolo_plugin id_externo | - |
| extrator | — | congelado | binario poph-extrator | - |
| falha | — | congelado | chave_toml turno.atraso_falha_modelo_ms; chave_toml turno.tentativas_falha_modelo; tabela matrix_avisos_falha | - |
| falhas | — | congelado | indice ix_matrix_falhas_org_time; indice ux_matrix_falhas_org_message; metrica poph_fluxos_efeitos_falhas_total; e mais 2 | - |
| fechada | — | congelado | coluna fechada_em; coluna fechada_motivo; coluna fechada_por; e mais 1 | - |
| fechar | — | congelado | operation_id conversas_fechar; operation_id fichas_fechar | - |
| ficha | — | congelado | campo_json conversation_to_ficha_rate; campo_json ficha_id; coluna ficha_id; e mais 13 | - |
| fichas | — | congelado | backup fichas; campo_json closed_fichas; campo_json open_fichas; e mais 20 | - |
| fila | — | congelado | indice ix_fluxo_efeitos_fila; metrica poph_fila_saida; metrica poph_fila_turnos | - |
| fim | — | congelado | coluna motivo_fim; linguagem de fluxos | - |
| finalizada | — | congelado | coluna finalizada_em | - |
| float | — | congelado | linguagem de fluxos | - |
| fluxo | — | congelado | campo_json fluxo_id; coluna fluxo_id; flag_cli --fluxo; e mais 19 | - |
| fluxos | — | congelado | backup fluxos; chave_toml fluxos; chave_toml fluxos.ativo; e mais 49 | - |
| fonte | — | congelado | chave_toml fluxos.max_fonte_bytes; coluna fonte; coluna fonte_sha256; e mais 1 | - |
| forcar | — | congelado | flag_cli --forcar | - |
| formatar | — | congelado | comando_cli poph flow formatar; operation_id fluxos_formatar | - |
| formato | — | congelado | backup formato; backup versao_formato; chave_toml log.formato | - |
| funcao | — | congelado | linguagem de fluxos | - |
| gatilho | — | congelado | linguagem de fluxos | - |
| globais | — | congelado | chave_toml admin.tokens_globais | - |
| historico | — | congelado | operation_id teste_chat_historico | - |
| humano | — | congelado | indice ix_messages_turno_humano | - |
| idempotencia | — | congelado | campo_json chave_idempotencia; coluna chave_idempotencia | - |
| igual | — | congelado | linguagem de fluxos | - |
| importar | — | congelado | operation_id backup_importar | - |
| iniciar | — | congelado | operation_id fichas_iniciar_fluxo | - |
| inicio | — | congelado | schema_openapi DadosInicio | - |
| instancia | — | congelado | campo_json instancia; protocolo_plugin instancia; schema_openapi InstanciaStatusJson | - |
| int | — | congelado | linguagem de fluxos | - |
| integracao | — | congelado | protocolo_plugin integracao | - |
| intervalo | — | congelado | chave_toml turno.intervalo_despacho_ms | - |
| itens | — | congelado | operation_id listar_itens | - |
| ja | — | congelado | variavel_ambiente POPH_EVO_SEGREDO_LOJA | - |
| latencia | — | congelado | metrica poph_latencia_etapa_segundos | - |
| len | — | congelado | linguagem de fluxos | - |
| liberado | — | congelado | coluna liberado_em | - |
| liberar | — | congelado | comando_cli poph flow liberar | - |
| ligar | — | congelado | comando_cli poph matrix ligar; operation_id matrix_espacos_ligar; schema_openapi LigarDesligar; e mais 1 | - |
| limiar | — | congelado | metrica poph_fluxos_g9_abaixo_do_limiar_total | - |
| limites | — | congelado | chave_toml limites; chave_toml limites.taxa_por_org_por_minuto; chave_toml limites.turnos_simultaneos; e mais 2 | - |
| list | — | congelado | linguagem de fluxos | - |
| lista | — | congelado | schema_openapi ListaJson_ArquivoJson; schema_openapi ListaJson_CategoriaJson; schema_openapi ListaJson_DepartamentoJson; e mais 3 | - |
| listar | — | congelado | comando_cli poph ficha listar; comando_cli poph flow listar; operation_id arquivos_listar; e mais 16 | - |
| maior | — | congelado | linguagem de fluxos | - |
| manifesto | — | congelado | protocolo_plugin manifesto | - |
| max | — | congelado | linguagem de fluxos | - |
| memoria | — | congelado | metrica poph_memoria_processo_bytes | - |
| menor | — | congelado | linguagem de fluxos | - |
| mensagem | — | congelado | campo_json mensagem; chave_toml handoff.mensagem_automatica; operation_id teste_chat_mensagem; e mais 1 | - |
| mensagem_ao_cliente | — | congelado | linguagem de fluxos | - |
| mensagens | — | congelado | operation_id mensagens_listar | - |
| metadados | — | congelado | protocolo_plugin metadados | - |
| metricas | — | congelado | schema_openapi MetricasJson | - |
| midia | — | congelado | indice ix_matrix_midia_pendentes; indice ux_matrix_midia_evento; tabela matrix_midia | - |
| min | — | congelado | linguagem de fluxos | - |
| modelo | — | congelado | chave_toml jev.modelo; chave_toml llm.modelo; chave_toml llm.recusa_turno_modelo; e mais 2 | - |
| mostrar | — | congelado | comando_cli poph ficha mostrar; comando_cli poph flow mostrar; operation_id arquivos_mostrar; e mais 11 | - |
| motivo | — | congelado | campo_json motivo; coluna fechada_motivo; coluna motivo; e mais 1 | - |
| nao | — | congelado | linguagem de fluxos; metrica poph_fluxos_cache_nao_cacheadas_total | - |
| nenhum | — | congelado | metrica poph_fluxos_g9_nenhum_total | - |
| nivel | — | congelado | chave_toml log.nivel | - |
| nome | — | congelado | coluna nome; protocolo_plugin nome | - |
| nos | — | congelado | chave_toml fluxos.max_nos | - |
| notificar_whatsapp | — | congelado | linguagem de fluxos | - |
| numero | — | congelado | coluna numero; indice ux_fluxo_revisoes_numero | - |
| operacao | — | congelado | operation_id operacao_healthz; operation_id operacao_metrics | - |
| ordem | — | congelado | coluna ordem; indice ux_fluxo_execucao_etapas_ordem | - |
| orientacao | — | congelado | chave_toml handoff.orientacao_animal_peconhento; chave_toml handoff.orientacao_ativa; chave_toml handoff.orientacao_confianca; e mais 3 | - |
| origem | — | congelado | backup org_origem | - |
| ou | — | congelado | linguagem de fluxos | - |
| outro | — | congelado | chave_toml handoff.orientacao_outro_risco_saude | - |
| para | — | congelado | flag_cli --para | - |
| pareamento | — | congelado | schema_openapi DadosPareamento | - |
| pendentes | — | congelado | indice ix_matrix_midia_pendentes; metrica poph_fluxos_efeitos_pendentes_total | - |
| por | — | congelado | chave_toml fluxos.max_fluxos_por_org; chave_toml limites.taxa_por_org_por_minuto; chave_toml limites.turnos_simultaneos_por_org; e mais 2 | - |
| porta | — | congelado | chave_toml servidor.porta | - |
| poupadas | — | congelado | metrica poph_fluxos_cache_em_uso_poupadas_total | - |
| processo | — | congelado | metrica poph_memoria_processo_bytes | - |
| produto | — | congelado | chave_toml handoff.orientacao_ingestao_contato_produto; schema_openapi DadosProduto | - |
| produtos | — | congelado | operation_id produtos_excluir; operation_id produtos_listar; operation_id produtos_mostrar | - |
| provedor | — | congelado | chave_toml llm.provedor | - |
| proxima | — | congelado | coluna proxima_tentativa; indice ix_inbox_proxima_tentativa | - |
| publicacao | — | congelado | schema_openapi DadosPublicacao | - |
| publicada | — | congelado | coluna publicada_em; coluna publicada_por | - |
| publicar | — | congelado | comando_cli poph flow publicar; operation_id fluxos_publicar | - |
| que | — | congelado | coluna atores_que_agiram_json | - |
| range | — | congelado | linguagem de fluxos | - |
| rascunho | — | congelado | coluna rascunho_fonte | - |
| reabrir | — | congelado | operation_id conversas_reabrir; operation_id teste_chat_reabrir | - |
| recarregar | — | congelado | operation_id recarregar_cluster | - |
| recusa | — | congelado | chave_toml llm.recusa_turno_modelo | - |
| registro | — | congelado | comando_cli poph matrix registro | - |
| regra | — | congelado | chave_toml fluxos.max_heap_regra; chave_toml fluxos.max_ticks_regra; metrica poph_fluxos_erros_regra_total | - |
| reindexar | — | congelado | operation_id reindexar | - |
| remetente | — | congelado | protocolo_plugin remetente | - |
| remoto | — | congelado | flag_cli --remoto | - |
| resolucoes | — | congelado | coluna resolucoes_json | - |
| resposta | — | congelado | schema_openapi RespostaErro | - |
| resultado | — | congelado | coluna resultado | - |
| resumo | — | congelado | operation_id analytics_resumo_operacional | - |
| retencao | — | congelado | chave_toml retencao; chave_toml retencao.dias | - |
| retentativas | — | congelado | chave_toml jev.retentativas; chave_toml llm.retentativas | - |
| retirar | — | congelado | comando_cli poph flow retirar | - |
| reversed | — | congelado | linguagem de fluxos | - |
| revisao | — | congelado | coluna revisao_atual; coluna revisao_id; indice ix_fluxo_execucoes_revisao; e mais 1 | - |
| revisoes | — | congelado | chave_toml fluxos.cache_revisoes_mb; gatilho tr_fluxo_revisoes_sem_delete; gatilho tr_fluxo_revisoes_sem_update; e mais 5 | - |
| revogar | — | congelado | operation_id tokens_revogar | - |
| risco | — | congelado | chave_toml handoff.orientacao_outro_risco_saude | - |
| saida | — | congelado | chave_toml llm.max_tokens_saida; flag_cli --saida; metrica poph_fila_saida | - |
| saltos | — | congelado | chave_toml fluxos.max_saltos_decisao | - |
| saude | — | congelado | chave_toml handoff.orientacao_outro_risco_saude | - |
| segredos | — | congelado | protocolo_plugin segredos | - |
| segundos | — | congelado | chave_toml handoff.orientacao_timeout_segundos; chave_toml jev.timeout_segundos; chave_toml llm.timeout_segundos; e mais 1 | - |
| sem | — | congelado | gatilho tr_fluxo_revisoes_sem_delete; gatilho tr_fluxo_revisoes_sem_update | - |
| servidor | — | congelado | chave_toml servidor; chave_toml servidor.endereco; chave_toml servidor.porta | - |
| simultaneos | — | congelado | chave_toml limites.turnos_simultaneos; chave_toml limites.turnos_simultaneos_por_org; chave_toml llm.simultaneos | - |
| situacao | — | congelado | campo_json situacao | - |
| sobrescrever | — | congelado | flag_cli --sobrescrever | - |
| sorted | — | congelado | linguagem de fluxos | - |
| str | — | congelado | linguagem de fluxos | - |
| tabelas | — | congelado | backup tabelas | - |
| temperatura | — | congelado | chave_toml llm.temperatura | - |
| tentativa | — | congelado | coluna proxima_tentativa; indice ix_inbox_proxima_tentativa | - |
| tentativas | — | congelado | chave_toml turno.tentativas_falha_modelo; coluna tentativas | - |
| teste | — | congelado | operation_id teste_chat_anexo_conteudo; operation_id teste_chat_criar; operation_id teste_chat_historico; e mais 4 | - |
| texto | — | congelado | protocolo_plugin max_texto_bytes; protocolo_plugin texto | - |
| tipo | — | congelado | coluna ator_tipo; coluna tipo; protocolo_plugin tipo | - |
| tipos | — | congelado | protocolo_plugin tipos_aceitos | - |
| transferencia | — | congelado | schema_openapi CorpoTransferencia | - |
| transferir | — | congelado | operation_id conversas_transferir | - |
| tuple | — | congelado | linguagem de fluxos | - |
| turno | — | congelado | chave_toml llm.recusa_turno_modelo; chave_toml turno; chave_toml turno.atraso_falha_modelo_ms; e mais 3 | - |
| turnos | — | congelado | chave_toml limites.turnos_simultaneos; chave_toml limites.turnos_simultaneos_por_org; metrica poph_fila_turnos; e mais 1 | - |
| ultimo | — | congelado | coluna ultimo_erro | - |
| uso | — | congelado | metrica poph_fluxos_cache_em_uso_poupadas_total; metrica poph_fluxos_revisoes_em_uso | - |
| validar | — | congelado | comando_cli poph flow validar; operation_id fluxos_validar | - |
| vazio | — | congelado | linguagem de fluxos | - |
| verificar | — | congelado | flag_cli --verificar | - |
| versao | — | congelado | backup versao_formato; coluna schema_versao; protocolo_plugin versao; e mais 1 | - |
| vincular | — | congelado | operation_id vincular | - |
| whatsapp | — | congelado | rota /admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/disconnect; rota /admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/qrcode; rota /admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/state | - |
| zip | — | congelado | linguagem de fluxos | - |
<!-- glossario:tabela:fim -->
