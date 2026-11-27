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
| texto | text | traduz | poph-agent, poph | 2733 |
| agente | agent | traduz | poph-agent, poph-server | 1853 |
| conversa | conversation | traduz | poph-agent, poph-server | 1506 |
| cenario | scenario | traduz | poph-server, poph | 1457 |
| nome | name | traduz | poph-flows, console | 1407 |
| estado | state | traduz | poph-server, poph-flows | 1383 |
| resultado | result | traduz | poph-store, poph-flows | 1105 |
| fluxo | flow | traduz | poph-agent, poph-server | 1039 |
| erro | error | traduz | poph, console | 947 |
| saida | output | traduz | poph, poph-server | 916 |
| tipo | kind | traduz | poph-store, poph-server | 775 |
| pasta | folder | traduz | poph, poph-server | 767 |
| campo | field | traduz | poph-flows, console | 755 |
| mensagem | message | traduz | poph-agent, poph-store | 736 |
| corpo | body | traduz | poph-server, poph | 698 |
| chave | key | traduz | poph, poph-server | 693 |
| servidor | server | traduz | console, poph-server | 690 |
| turno | turn | traduz | poph-agent, console | 685 |
| evento | event | traduz | console, poph-agent | 683 |
| registro | record | traduz | poph-agent, console | 605 |
| acao | action | traduz | poph, poph-flows | 601 |
| dados | data | traduz | poph-flows, console | 584 |
| etapa | step | traduz | poph-flows, poph-agent | 565 |
| motivo | reason | traduz | poph-agent, poph-jev | 544 |
| de | of | traduz | poph, poph-server | 519 |
| arquivo | file | traduz | poph, poph-server | 502 |
| gatilho | trigger | traduz | poph-agent, poph-flows | 461 |
| resposta | response | traduz | poph-server, poph | 460 |
| ficha | case | traduz | poph-server, poph-agent | 458 |
| pedir | ask | traduz | poph-server, poph | 449 |
| caminho | path | traduz | poph, poph-server | 437 |
| ambiente | environment | traduz | poph | 406 |
| fonte | source | traduz | poph-flows, poph-store | 381 |
| com | with | traduz | poph-server, poph | 377 |
| decisao | decision | traduz | poph-jev, poph-flows | 376 |
| versao | version | traduz | poph-server, console | 375 |
| valor | value | traduz | poph, poph-server | 369 |
| criar | create | traduz | poph-server, poph | 365 |
| departamento | department | traduz | poph, poph-agent | 354 |
| provedor | provider | traduz | poph-agent, poph-server | 330 |
| novo | new | traduz | poph-server, poph | 328 |
| integracao | integration | traduz | poph-server, console | 313 |
| desligar | disable | traduz | poph-server, poph | 309 |
| fim | end | traduz | poph-flows, poph-agent | 305 |
| situacao | situation | traduz | console, poph-retention | 290 |
| falha | failure | traduz | poph, poph-server | 288 |
| modelo | model | traduz | poph-agent, poph-server | 281 |
| whatsapp | whatsapp | traduz | console, poph-server | 258 |
| configuracao | configuration | traduz | poph, console | 255 |
| canal | channel | traduz | poph-server, poph-store | 253 |
| itens | items | traduz | poph-server, console | 248 |
| sem | without | traduz | poph, poph-server | 243 |
| em | in | traduz | poph-store, poph-server | 239 |
| limites | limits | traduz | poph-flows, poph-agent | 236 |
| numero | number | traduz | console, poph-store | 232 |
| linha | line | traduz | poph-server, poph | 224 |
| atual | current | traduz | poph-flows, poph-server | 219 |
| grupo | group | traduz | poph, poph-server | 216 |
| codigo | code | traduz | poph, poph-server | 215 |
| instancia | instance | traduz | poph-server, poph-evolution | 200 |
| campos | fields | traduz | poph-flows, console | 196 |
| validar | validate | traduz | poph-types, poph | 191 |
| cliente | client | traduz | poph-server, poph | 187 |
| descricao | description | traduz | console, poph-agent | 185 |
| ator | actor | traduz | poph-store, poph-server | 184 |
| externo | external | traduz | poph-server, poph-agent | 178 |
| caso | case | traduz | poph-server, poph | 177 |
| execucao | execution | traduz | poph-server, poph-store | 174 |
| vazio | empty | traduz | poph-server, poph-flows | 172 |
| da | of | traduz | poph-server, poph | 169 |
| cru | raw | traduz | poph-server | 161 |
| relatorio | report | traduz | poph, poph-server | 158 |
| rotulo | label | traduz | poph-server, console | 157 |
| categoria | category | traduz | poph, poph-server | 155 |
| rota | route | traduz | poph-server | 154 |
| metodo | method | traduz | poph-server, poph | 153 |
| destino | target | traduz | poph, poph-server | 144 |
| origem | origin | traduz | poph, console | 144 |
| lista | list | traduz | poph, poph-server | 141 |
| nao | not | traduz | poph-server, poph | 133 |
| segredo | secret | traduz | poph-server, poph | 133 |
| segredos | secrets | traduz | poph, console | 133 |
| para | for | traduz | poph-flows, console | 130 |
| fluxos | flows | traduz | poph, poph-agent | 126 |
| produto | product | traduz | poph, poph-server | 125 |
| pedido | request | traduz | poph-server, poph | 123 |
| por | by | traduz | poph, poph-server | 123 |
| agentes | agents | traduz | poph, console | 122 |
| abrir | open | traduz | poph, poph-server | 112 |
| endereco | address | traduz | poph-server, poph | 112 |
| depois | after | traduz | poph-server, console | 109 |
| revisao | revision | traduz | poph-server, poph-store | 107 |
| detalhe | detail | traduz | poph, poph-server | 99 |
| nivel | level | traduz | poph, console | 99 |
| ao | to | traduz | poph-server, poph | 98 |
| conteudo | content | traduz | poph, poph-server | 96 |
| alvo | target | traduz | poph-server, poph | 93 |
| antes | before | traduz | poph-server, console | 92 |
| descrever | describe | traduz | poph | 92 |
| porta | port | traduz | poph-server, poph | 92 |
| registros | records | traduz | poph-server | 91 |
| conversas | conversations | traduz | poph-server, poph-store | 88 |
| lido | read | traduz | poph, poph-server | 88 |
| manifesto | manifest | traduz | console, poph-server | 88 |
| imprimir | print | traduz | poph | 87 |
| resolve | resolves | traduz | scripts, poph-flows | 86 |
| ordem | order | traduz | console, poph-store | 85 |
| instrucoes | instructions | traduz | poph, poph-server | 81 |
| listar | list | traduz | poph, poph-server | 81 |
| sessao | session | traduz | poph, console | 78 |
| espaco | space | traduz | console, poph | 76 |
| fechada | closed | traduz | poph-store, poph-server | 76 |
| motor | engine | traduz | poph-server, poph | 73 |
| metricas | metrics | traduz | poph-server, poph | 72 |
| pagina | page | traduz | poph, poph-server | 71 |
| efeitos | effects | traduz | poph-server, poph-store | 70 |
| relato | report | traduz | poph-server | 70 |
| eventos | events | traduz | poph-server, poph-store | 68 |
| subir | raise | traduz | poph-server, poph | 68 |
| comando | command | traduz | poph | 67 |
| executar | run | traduz | poph, poph-server | 66 |
| criado | created | traduz | poph-server, poph | 65 |
| idempotencia | idempotency | traduz | poph-store, poph-server | 65 |
| mapa | map | traduz | poph-server, poph | 65 |
| captura | capture | traduz | poph-server, poph | 64 |
| indice | index | traduz | poph, poph-server | 64 |
| ler | read | traduz | poph, poph-server | 64 |
| primeira | first | traduz | poph-server, poph | 64 |
| falhas | failures | traduz | poph-server, poph-store | 63 |
| igual | equal | traduz | poph-flows, poph | 61 |
| outro | other | traduz | poph, poph-server | 61 |
| teste | test | traduz | poph, poph-server | 61 |
| responder | reply | traduz | poph, poph-server | 60 |
| aberta | open | traduz | poph-server, poph-store | 59 |
| ativo | active | traduz | poph-agent, console | 58 |
| anexos | attachments | traduz | poph-server, poph-agent | 57 |
| dias | days | traduz | console, poph-retention | 57 |
| documento | document | traduz | poph, poph-server | 57 |
| na | towards | traduz | poph-server, poph | 57 |
| outra | other | traduz | poph-server, poph | 57 |
| aviso | notice | traduz | poph, console | 56 |
| ultimo | last | traduz | poph-store, poph-server | 56 |
| instancias | instances | traduz | console, poph-server | 55 |
| tentativas | attempts | traduz | poph-server, poph-agent | 55 |
| catalogo | catalog | traduz | poph, poph-server | 54 |
| carregar | load | traduz | poph, poph-server | 53 |
| conexao | connection | traduz | poph, poph-server | 53 |
| entrada | entry | traduz | poph, poph-server | 53 |
| falsa | false | traduz | poph-server, poph | 53 |
| recusa | refusal | traduz | poph, poph-server | 53 |
| atualizar | update | traduz | poph, console | 50 |
| esperado | expected | traduz | poph, poph-server | 50 |
| cabecalhos | headers | traduz | poph-server, poph | 49 |
| diretorio | directory | traduz | poph-server, poph-rag | 49 |
| fila | queue | traduz | poph-server, poph | 48 |
| contem | contains | traduz | poph-flows, poph-files | 47 |
| ligar | link | traduz | poph-server, poph | 47 |
| esperar | wait | traduz | poph-server, poph | 46 |
| formato | format | traduz | poph-store, poph | 46 |
| variavel | variable | traduz | poph, poph-server | 46 |
| fichas | cases | traduz | poph-store, poph-server | 45 |
| guarda | guard | traduz | poph-server, poph | 45 |
| limite | limit | traduz | poph, poph-server | 45 |
| mostrar | show | traduz | poph | 45 |
| retrato | snapshot | traduz | poph | 45 |
| gravado | stored | traduz | poph, poph-server | 44 |
| pedidos | requests | traduz | poph-server, poph | 44 |
| publicar | publish | traduz | poph-server, poph | 44 |
| auditoria | audit | traduz | poph-server | 43 |
| resumo | summary | traduz | poph, raiz | 43 |
| sinal | signal | traduz | poph-server, poph | 43 |
| arquivos | files | traduz | poph-server, poph | 42 |
| produtos | products | traduz | poph, console | 42 |
| modo | mode | traduz | console, scripts | 41 |
| titulo | title | traduz | console, poph | 41 |
| turnos | turns | traduz | poph-server, raiz | 41 |
| vivo | live | traduz | poph, poph-server | 41 |
| chamada | call | traduz | poph-server, console | 40 |
| contexto | context | traduz | poph, poph-server | 40 |
| enviado | sent | traduz | poph-server, poph-store | 40 |
| funcao | function | traduz | poph-flows, raiz | 40 |
| banco | database | traduz | poph, poph-server | 39 |
| enviar | send | traduz | poph-server, poph | 39 |
| linhas | lines | traduz | poph, poph-server | 39 |
| metadados | metadata | traduz | poph-files, poph | 39 |
| categorias | categories | traduz | poph, poph-server | 38 |
| desligado | disabled | traduz | poph, scripts | 38 |
| evolucao | evolution | traduz | poph-server | 38 |
| mensagens | messages | traduz | poph-server, scripts | 38 |
| par | pair | traduz | poph, poph-server | 38 |
| referencia | reference | traduz | poph, poph-server | 38 |
| segunda | second | traduz | poph-server, poph | 38 |
| tabelas | tables | traduz | console, poph | 38 |
| tem | has | traduz | poph, console | 38 |
| temperatura | temperature | traduz | poph-core, poph-store | 38 |
| comparar | compare | traduz | poph-server, poph | 36 |
| pela | by | traduz | poph-server, poph | 36 |
| preparo | preparation | traduz | poph, poph-server | 36 |
| sala | room | traduz | poph-server, poph | 36 |
| envios | sends | traduz | poph-server | 35 |
| excluir | delete | traduz | poph, poph-server | 35 |
| globais | globals | traduz | poph-server, poph-types | 35 |
| maior | bigger | traduz | poph-flows, poph-rag | 35 |
| mostra | shows | traduz | poph, poph-server | 35 |
| pendentes | pending | traduz | poph-server, poph-store | 35 |
| escrever | write | traduz | poph, console | 34 |
| gravada | stored | traduz | poph-server, poph | 34 |
| inicio | beginning | traduz | poph-server, poph | 34 |
| padrao | default | traduz | poph, poph-server | 34 |
| definicao | definition | traduz | poph-store, poph-server | 33 |
| gerar | generate | traduz | poph | 33 |
| opcoes | options | traduz | poph, poph-server | 33 |
| que | that | traduz | poph, poph-server | 33 |
| rascunho | draft | traduz | poph-store, poph-server | 33 |
| verificacao | verification | traduz | poph | 33 |
| compilado | compiled | traduz | poph-server, poph | 32 |
| entregar | deliver | traduz | poph-server | 32 |
| nomes | names | traduz | poph, poph-server | 32 |
| retencao | retention | traduz | console, poph | 32 |
| tabela | table | traduz | poph, poph-server | 32 |
| falso | false | traduz | poph-server, poph | 31 |
| forcar | force | traduz | console, poph | 31 |
| inexistente | missing | traduz | poph, poph-server | 31 |
| iniciar | start | traduz | poph-server, poph | 31 |
| modelos | models | traduz | poph, poph-server | 31 |
| respostas | responses | traduz | poph-server | 31 |
| apoio | support | traduz | poph | 30 |
| divergencias | divergences | traduz | poph, poph-server | 30 |
| historico | history | traduz | poph-server, poph | 30 |
| liberar | release | traduz | poph, console | 30 |
| ou | or | traduz | poph-flows, poph | 30 |
| verificacoes | verifications | traduz | poph | 30 |
| repetida | repeated | traduz | poph-server, poph | 29 |
| encaminhar | forward | traduz | poph-flows, poph-server | 28 |
| fora | outside | traduz | poph-server, poph | 28 |
| importar | import | traduz | poph-server, poph | 28 |
| proibido | forbidden | traduz | poph-server, poph | 28 |
| resolucoes | resolutions | traduz | poph-store, poph-server | 28 |
| resto | rest | traduz | poph, poph-server | 28 |
| roteiro | script | traduz | poph-server | 28 |
| vizinha | neighbouring | traduz | poph-server, poph | 28 |
| escolha | choice | traduz | poph, console | 27 |
| execucoes | executions | traduz | poph-store, poph-server | 27 |
| ligado | linked | traduz | poph, poph-server | 27 |
| proxima | next | traduz | poph-server, poph-store | 27 |
| raiz | root | traduz | poph-server, poph | 27 |
| recusado | refused | traduz | poph-server, poph | 27 |
| remoto | remote | traduz | poph | 27 |
| invalido | invalid | traduz | poph, poph-server | 26 |
| montagem | assembly | traduz | poph, console | 26 |
| publicadas | published | traduz | poph-server | 26 |
| uma | a | traduz | poph-server, poph | 26 |
| canais | channels | traduz | poph-server, poph | 25 |
| carimbo | stamp | traduz | poph-server, poph | 25 |
| como | as | traduz | poph, poph-server | 25 |
| desfecho | outcome | traduz | poph-agent, poph-store | 25 |
| fontes | sources | traduz | poph, console | 25 |
| servir | serve | traduz | poph-server, poph | 25 |
| agora | now | traduz | poph-server, poph | 24 |
| avulsa | one_off | traduz | poph-server | 24 |
| capturados | captured | traduz | poph-server | 24 |
| chamar | call | traduz | poph-server | 24 |
| primeiro | first | traduz | poph-server, poph | 24 |
| relatorios | reports | traduz | poph | 24 |
| remetente | sender | traduz | console, poph-evolution | 24 |
| retentativas | retries | traduz | poph-jev, poph-core | 24 |
| vai | goes | traduz | raiz, poph | 24 |
| contar | count | traduz | poph-server, poph | 23 |
| diagnostico | diagnostic | traduz | poph, poph-server | 23 |
| duracao | duration | traduz | poph-server, console | 23 |
| espacos | spaces | traduz | poph-server, poph | 23 |
| casos | cases | traduz | poph-server, poph | 22 |
| consulta | query | traduz | poph, poph-server | 22 |
| errado | wrong | traduz | poph-server, poph | 22 |
| fechar | close | traduz | poph-server, poph | 22 |
| integracoes | integrations | traduz | poph-server, poph | 22 |
| problemas | problems | traduz | poph, console | 22 |
| trechos | passages | traduz | console, poph | 22 |
| dona | owner | traduz | poph-server | 21 |
| escopo | scope | traduz | console, poph-server | 21 |
| mudanca | change | traduz | poph | 21 |
| todos | all | traduz | poph, poph-server | 21 |
| chamadas | calls | traduz | poph-server | 20 |
| diferente | different | traduz | poph-flows, poph | 20 |
| ferramentas | tools | traduz | console, poph | 20 |
| final | final | traduz | poph-matrix, console | 20 |
| instrucao | instruction | traduz | raiz, poph-server | 20 |
| mostrado | shown | traduz | poph, poph-server | 20 |
| quando | when | traduz | raiz, console | 20 |
| saidas | outputs | traduz | poph-server, poph | 20 |
| ausente | missing | traduz | poph, poph-server | 19 |
| exige | requires | traduz | raiz, poph-server | 19 |
| filtro | filter | traduz | poph, poph-server | 19 |
| argumentos | arguments | traduz | console, poph-server | 18 |
| diagnosticos | diagnostics | traduz | poph, poph-server | 18 |
| existe | exists | traduz | poph-server, poph | 18 |
| grande | large | traduz | poph-server, poph | 18 |
| portugues | portuguese | traduz | poph | 18 |
| rodada | round | traduz | scripts, console | 18 |
| roteiros | scripts | traduz | poph-server | 18 |
| ruim | poor | traduz | poph, poph-server | 18 |
| temporaria | temporary | traduz | poph | 18 |
| departamentos | departments | traduz | poph, poph-server | 17 |
| despedida | farewell | traduz | console | 17 |
| entradas | entries | traduz | poph-server | 17 |
| exemplos | examples | traduz | raiz, console | 17 |
| gravar | write | traduz | poph, poph-server | 17 |
| montar | build | traduz | poph-server, poph | 17 |
| mudou | changed | traduz | console, poph | 17 |
| nova | new | traduz | poph-server, poph | 17 |
| recebido | received | traduz | poph, poph-server | 17 |
| teto | cap | traduz | poph-server, poph | 17 |
| acoes | actions | traduz | raiz | 16 |
| ja | already | traduz | poph-server, poph | 16 |
| nada | nothing | traduz | poph, poph-server | 16 |
| reindexar | reindex | traduz | poph, console | 16 |
| relativo | relative | traduz | poph-server, poph | 16 |
| vitima | victim | traduz | poph-server | 16 |
| aberto | open | traduz | poph-server | 15 |
| adiado | deferred | traduz | poph-server | 15 |
| alterado | changed | traduz | poph-server, poph | 15 |
| conjunto | set | traduz | poph, poph-server | 15 |
| erros | errors | traduz | poph-server, poph | 15 |
| escalares | scalars | traduz | poph | 15 |
| gravadas | stored | traduz | poph-server | 15 |
| grupos | groups | traduz | poph | 15 |
| inseridas | inserted | traduz | console, poph-server | 15 |
| ligacao | link | traduz | poph | 15 |
| marcador | marker | traduz | poph-server | 15 |
| menor | smaller | traduz | poph-flows | 15 |
| pedaco | piece | traduz | poph, poph-server | 15 |
| sobrescrever | overwrite | traduz | console, poph | 15 |
| tarefa | task | traduz | poph-server | 15 |
| tentativa | attempt | traduz | poph-store, poph-server | 15 |
| achados | findings | traduz | poph-server, poph | 14 |
| assinados | signed | traduz | poph-server | 14 |
| baixar | download | traduz | poph-server, poph | 14 |
| carrinho | cart | traduz | console, poph | 14 |
| colecao | collection | traduz | poph-server | 14 |
| conferido | checked | traduz | poph | 14 |
| confianca | confidence | traduz | poph, console | 14 |
| dado | given | traduz | poph-server, poph | 14 |
| duas | two | traduz | poph-server, poph | 14 |
| exigir | require | traduz | poph | 14 |
| ferramenta | tool | traduz | poph, poph-server | 14 |
| formatar | format | traduz | poph, console | 14 |
| impresso | printed | traduz | poph | 14 |
| nenhum | none | traduz | poph-server | 14 |
| proprio | own | traduz | poph-server, poph | 14 |
| publicada | published | traduz | poph-store, poph | 14 |
| trecho | passage | traduz | poph, poph-server | 14 |
| valores | values | traduz | poph-server | 14 |
| vazamento | leak | traduz | poph-server | 14 |
| anexo | attachment | traduz | poph-server, poph | 13 |
| atualizadas | updated | traduz | console, poph-server | 13 |
| busca | search | traduz | poph, poph-server | 13 |
| cabecalho | header | traduz | poph-server | 13 |
| criacao | creation | traduz | poph-server, poph | 13 |
| criada | created | traduz | poph, poph-server | 13 |
| desconhecida | unknown | traduz | poph-server, console | 13 |
| esperada | expected | traduz | poph, poph-server | 13 |
| extrator | extractor | traduz | poph-files, scripts | 13 |
| fica | stays | traduz | poph-server, poph | 13 |
| isolamento | isolation | traduz | poph-server | 13 |
| lote | batch | traduz | poph, poph-server | 13 |
| nem | nor | traduz | poph-server, poph | 13 |
| notificar | notify | traduz | poph-server, raiz | 13 |
| obrigatorio | required | traduz | raiz, poph | 13 |
| quantos | how_many | traduz | poph-server | 13 |
| recusada | refused | traduz | poph, poph-server | 13 |
| revisoes | revisions | traduz | poph-store, poph-server | 13 |
| testes | tests | traduz | poph | 13 |
| validacao | validation | traduz | poph | 13 |
| baixado | downloaded | traduz | poph-server, poph | 12 |
| bruta | raw | traduz | poph, poph-server | 12 |
| degradada | degraded | traduz | console | 12 |
| estruturado | structured | traduz | poph-server, poph | 12 |
| exportar | export | traduz | poph, poph-server | 12 |
| falhou | failed | traduz | poph, poph-server | 12 |
| fechado | closed | traduz | poph-server | 12 |
| importacao | import | traduz | poph, poph-server | 12 |
| inteiro | integer | traduz | poph | 12 |
| leitor | reader | traduz | poph | 12 |
| lida | read | traduz | poph-server | 12 |
| midia | media | traduz | poph-server, poph-store | 12 |
| nos | us | traduz | poph-flows, poph-server | 12 |
| partes | parts | traduz | poph-server, poph | 12 |
| processador | processor | traduz | poph-server, poph | 12 |
| recebidos | received | traduz | poph-server | 12 |
| repetido | repeated | traduz | poph, poph-server | 12 |
| rotas | routes | traduz | poph-server | 12 |
| saltos | jumps | traduz | poph-server, poph-flows | 12 |
| tipos | kinds | traduz | poph-server | 12 |
| vira | becomes | traduz | poph, poph-server | 12 |
| abertura | opening | traduz | poph-server, poph | 11 |
| ajuda | help | traduz | poph, scripts | 11 |
| atores | actors | traduz | poph-server, poph | 11 |
| biblioteca | library | traduz | poph-server, poph | 11 |
| chaves | keys | traduz | poph | 11 |
| comparacao | comparison | traduz | poph, poph-server | 11 |
| conflito | conflict | traduz | poph, poph-server | 11 |
| dois | two | traduz | poph-server, poph | 11 |
| encerrada | closed | traduz | poph-server | 11 |
| esperadas | expected | traduz | poph-server, poph | 11 |
| etapas | steps | traduz | poph-server, poph-store | 11 |
| feita | done | traduz | poph | 11 |
| filho | child | traduz | poph, poph-server | 11 |
| foto | photo | traduz | poph-server, poph | 11 |
| livre | free | traduz | poph-server, poph | 11 |
| orfaos | orphans | traduz | poph | 11 |
| ouvinte | listener | traduz | poph | 11 |
| pendente | pending | traduz | poph, poph-server | 11 |
| relogio | clock | traduz | poph-server | 11 |
| segundo | second | traduz | poph-server, poph | 11 |
| situacoes | situations | traduz | console | 11 |
| variaveis | variables | traduz | poph | 11 |
| velha | old | traduz | poph-server, poph | 11 |
| vez | time | traduz | poph-server, poph | 11 |
| vinculo | link | traduz | poph-server, poph | 11 |
| aparece | appears | traduz | poph, poph-server | 10 |
| aparelho | device | traduz | poph-server | 10 |
| bruto | raw | traduz | poph-server | 10 |
| configurado | configured | traduz | console, poph | 10 |
| conta | counts | traduz | poph-server, poph | 10 |
| direto | direct | traduz | poph-server, poph | 10 |
| diz | says | traduz | poph, poph-server | 10 |
| entrega | delivery | traduz | poph-server | 10 |
| escrita | writing | traduz | poph-server, poph | 10 |
| finalizada | finalised | traduz | poph-store, poph-agent | 10 |
| gravados | stored | traduz | poph-server, console | 10 |
| inativo | inactive | traduz | poph, poph-server | 10 |
| liberado | released | traduz | poph-server, poph-store | 10 |
| limpo | clean | traduz | poph, poph-server | 10 |
| metrica | metric | traduz | poph-server | 10 |
| operacao | operation | traduz | poph-server, poph | 10 |
| prefixo | prefix | traduz | poph, poph-server | 10 |
| quem | who | traduz | poph, poph-server | 10 |
| roteador | router | traduz | poph-server | 10 |
| salas | rooms | traduz | scripts, poph-server | 10 |
| uso | use | traduz | poph | 10 |
| veneno | poison | traduz | poph-server | 10 |
| verificar | verify | traduz | poph, poph-server | 10 |
| celulas | cells | traduz | poph-server | 9 |
| conferir | check | traduz | poph | 9 |
| cria | creates | traduz | poph, poph-server | 9 |
| dentro | inside | traduz | poph, poph-server | 9 |
| devolve | returns | traduz | poph-server | 9 |
| enfileirar | enqueue | traduz | poph-server | 9 |
| metodos | methods | traduz | poph-server | 9 |
| modulo | module | traduz | poph, poph-server | 9 |
| obter | get | traduz | poph | 9 |
| orientacao | guidance | traduz | raiz, poph | 9 |
| preco | price | traduz | console, poph | 9 |
| preenchidos | filled | traduz | console | 9 |
| publicado | published | traduz | poph-server | 9 |
| semear | seed | traduz | poph, poph-server | 9 |
| tamanho | size | traduz | poph | 9 |
| adicionadas | added | traduz | console | 8 |
| aspas | quotes | traduz | poph | 8 |
| ate | until | traduz | poph-server, scripts | 8 |
| atendente | attendant | traduz | poph-server, scripts | 8 |
| contador | counter | traduz | poph-server | 8 |
| criadas | created | traduz | poph-server | 8 |
| das | of | traduz | poph, poph-server | 8 |
| envio | send | traduz | poph-server | 8 |
| gerado | generated | traduz | poph, poph-server | 8 |
| instalado | installed | traduz | poph-server, poph | 8 |
| laco | loop | traduz | poph-server | 8 |
| nunca | never | traduz | poph-server, poph | 8 |
| operacoes | operations | traduz | poph, poph-server | 8 |
| parte | part | traduz | poph, poph-server | 8 |
| permissao | permission | traduz | poph | 8 |
| preparar | prepare | traduz | poph-server, poph | 8 |
| quente | hot | traduz | poph, poph-server | 8 |
| recurso | resource | traduz | poph-server | 8 |
| removidas | removed | traduz | console | 8 |
| repetir | repeat | traduz | poph-server, poph | 8 |
| secao | section | traduz | poph, poph-server | 8 |
| simulacao | simulation | traduz | poph | 8 |
| sincronizar | synchronise | traduz | poph | 8 |
| todas | all | traduz | poph-server, console | 8 |
| transferencia | transfer | traduz | poph-server | 8 |
| trocaram | swapped | traduz | console | 8 |
| visita | visit | traduz | poph-server | 8 |
| abandonar | abandon | traduz | poph-server | 7 |
| aceita | accepts | traduz | poph, poph-server | 7 |
| apertado | tight | traduz | poph-server | 7 |
| atraso | delay | traduz | poph, poph-server | 7 |
| atualizacoes | updates | traduz | poph | 7 |
| caractere | character | traduz | poph | 7 |
| conhecimento | knowledge | traduz | console, poph-server | 7 |
| desconectar | disconnect | traduz | poph-server, console | 7 |
| encontrado | found | traduz | poph | 7 |
| entidade | entity | traduz | poph-server, poph | 7 |
| enviados | sent | traduz | poph-server | 7 |
| forma | form | traduz | poph | 7 |
| janela | window | traduz | poph-server, poph | 7 |
| marca | mark | traduz | poph-server, poph | 7 |
| memoria | memory | traduz | poph, scripts | 7 |
| mesmo | same | traduz | poph, poph-server | 7 |
| pares | pairs | traduz | poph | 7 |
| pelo | by | traduz | poph-server | 7 |
| processo | process | traduz | poph | 7 |
| quantas | how_many | traduz | poph-server | 7 |
| terceira | third | traduz | poph-server | 7 |
| ultima | last | traduz | poph-server, poph | 7 |
| vinculado | linked | traduz | poph, poph-server | 7 |
| vistos | seen | traduz | poph-server | 7 |
| alheio | foreign | traduz | poph-server | 6 |
| anterior | previous | traduz | poph-server | 6 |
| apagado | deleted | traduz | poph | 6 |
| armazem | store | traduz | poph | 6 |
| atender | attend | traduz | poph, poph-server | 6 |
| atendimento | service | traduz | scripts, raiz | 6 |
| ativos | active | traduz | poph-server, poph | 6 |
| atualizado | updated | traduz | poph-server | 6 |
| autorizado | authorized | traduz | poph-server, console | 6 |
| avaliador | evaluator | traduz | poph-server, poph | 6 |
| avancar | advance | traduz | poph-server | 6 |
| cada | each | traduz | poph, poph-server | 6 |
| carrinhos | carts | traduz | console | 6 |
| cifrados | encrypted | traduz | poph | 6 |
| conectado | connected | traduz | poph-server | 6 |
| construir | build | traduz | poph, poph-server | 6 |
| continua | continues | traduz | poph-server, poph | 6 |
| convidado | guest | traduz | poph, poph-server | 6 |
| decodificar | decode | traduz | poph, poph-server | 6 |
| desligada | disabled | traduz | poph-server | 6 |
| falar | speak | traduz | poph-server | 6 |
| falta | missing | traduz | poph, poph-server | 6 |
| liberados | released | traduz | poph-server, poph | 6 |
| limitador | limiter | traduz | poph-server | 6 |
| mesma | same | traduz | poph-server | 6 |
| observador | observer | traduz | poph | 6 |
| onde | where | traduz | poph, console | 6 |
| pai | parent | traduz | poph | 6 |
| parado | stopped | traduz | poph, poph-server | 6 |
| reabrir | reopen | traduz | poph-server | 6 |
| regra | rule | traduz | poph-server, raiz | 6 |
| retirar | withdraw | traduz | poph | 6 |
| segundos | seconds | traduz | poph, raiz | 6 |
| transacao | transaction | traduz | poph-server, scripts | 6 |
| transferida | transferred | traduz | poph-server | 6 |
| transitoria | transient | traduz | poph-server | 6 |
| transporte | transport | traduz | poph | 6 |
| trocado | swapped | traduz | poph-server, poph | 6 |
| usada | used | traduz | poph-server | 6 |
| versoes | versions | traduz | poph, poph-server | 6 |
| acima | above | traduz | poph-server, poph | 5 |
| agir | act | traduz | poph-server | 5 |
| apaga | deletes | traduz | poph | 5 |
| apos | after | traduz | poph, poph-server | 5 |
| autorizacao | authorization | traduz | poph-server, poph | 5 |
| avisar | notify | traduz | console | 5 |
| canonico | canonical | traduz | poph-server | 5 |
| caracteres | characters | traduz | poph, poph-server | 5 |
| cifra | cipher | traduz | poph | 5 |
| comercial | commercial | traduz | scripts, poph-server | 5 |
| conflitos | conflicts | traduz | poph-server, poph | 5 |
| controle | control | traduz | poph-server | 5 |
| corte | cutoff | traduz | poph, poph-server | 5 |
| decisoes | decisions | traduz | console | 5 |
| espelhado | mirrored | traduz | poph-server, poph-store | 5 |
| estavel | stable | traduz | poph | 5 |
| exclusao | exclusion | traduz | poph-server, poph | 5 |
| futuro | future | traduz | poph | 5 |
| imagem | image | traduz | poph-server | 5 |
| instalacao | installation | traduz | poph | 5 |
| juntos | together | traduz | poph, poph-server | 5 |
| leituras | reads | traduz | poph-server | 5 |
| ligada | linked | traduz | poph-server | 5 |
| longo | long | traduz | poph, poph-server | 5 |
| migracao | migration | traduz | poph | 5 |
| minuscula | lowercase | traduz | poph-server | 5 |
| mudancas | changes | traduz | poph-server, poph | 5 |
| novos | new | traduz | console, poph | 5 |
| objeto | object | traduz | poph, poph-server | 5 |
| ordenadas | sorted | traduz | poph-server | 5 |
| removidos | removed | traduz | console, poph | 5 |
| revogado | revoked | traduz | poph, poph-server | 5 |
| sao | are | traduz | poph-server, poph | 5 |
| simultaneos | simultaneous | traduz | raiz, poph-agent | 5 |
| temporario | temporary | traduz | poph | 5 |
| tudo | everything | traduz | poph-server, scripts | 5 |
| valido | valid | traduz | poph | 5 |
| ajustes | adjustments | traduz | poph, poph-server | 4 |
| ano | year | traduz | poph | 4 |
| antiga | old | traduz | poph, poph-server | 4 |
| apagar | delete | traduz | poph, poph-server | 4 |
| aplicar | apply | traduz | poph | 4 |
| ativa | active | traduz | poph-server, poph-store | 4 |
| auditorias | audits | traduz | poph-server | 4 |
| booleano | boolean | traduz | poph | 4 |
| concluida | finished | traduz | poph-server | 4 |
| configurar | configure | traduz | poph | 4 |
| contadores | counters | traduz | poph-server | 4 |
| convite | invitation | traduz | poph, poph-server | 4 |
| definir | define | traduz | poph | 4 |
| deixa | leaves | traduz | poph, poph-server | 4 |
| derivar | derive | traduz | poph | 4 |
| desconhecido | unknown | traduz | poph-server, poph | 4 |
| desde | since | traduz | poph | 4 |
| disco | disk | traduz | poph-server, poph | 4 |
| divergente | divergent | traduz | poph-server, poph | 4 |
| dos | of | traduz | poph | 4 |
| entre | between | traduz | poph, poph-server | 4 |
| entrou | entered | traduz | poph-store, poph-server | 4 |
| espera | wait | traduz | poph-server | 4 |
| esperados | expected | traduz | poph-server | 4 |
| exemplo | example | traduz | poph-server | 4 |
| existente | existing | traduz | poph | 4 |
| faixa | range | traduz | poph, poph-server | 4 |
| filtros | filters | traduz | poph-server | 4 |
| humana | human | traduz | poph-server | 4 |
| interno | internal | traduz | poph | 4 |
| limpa | clean | traduz | poph-server | 4 |
| margem | margin | traduz | console | 4 |
| nas | in_the | traduz | poph-server, poph | 4 |
| palavra | word | traduz | raiz | 4 |
| parcial | partial | traduz | poph-server, poph | 4 |
| passa | passes | traduz | poph-server, poph | 4 |
| perguntas | questions | traduz | console | 4 |
| publicacao | publication | traduz | poph-server | 4 |
| remota | remote | traduz | poph-server | 4 |
| resolucao | resolution | traduz | poph | 4 |
| respeita | respects | traduz | poph, poph-server | 4 |
| rodar | run | traduz | poph | 4 |
| segmento | segment | traduz | poph, poph-server | 4 |
| segura | safe | traduz | console | 4 |
| sensivel | sensitive | traduz | poph, poph-server | 4 |
| sintaxe | syntax | traduz | poph | 4 |
| soma | sum | traduz | poph-server | 4 |
| telefone | phone | traduz | poph-server | 4 |
| terminar | finish | traduz | poph-server | 4 |
| traco | trace | traduz | poph | 4 |
| unica | single | traduz | poph, poph-server | 4 |
| valida | validates | traduz | poph-server | 4 |
| agiram | acted | traduz | poph-server, poph-store | 3 |
| alterados | changed | traduz | console | 3 |
| assinatura | signature | traduz | poph-server | 3 |
| ativas | active | traduz | console | 3 |
| cai | falls | traduz | poph | 3 |
| caminhos | paths | traduz | poph-server | 3 |
| cancelamento | cancellation | traduz | poph | 3 |
| centavos | cents | traduz | console | 3 |
| codigos | codes | traduz | poph, poph-server | 3 |
| colunas | columns | traduz | poph | 3 |
| consultar | query | traduz | poph | 3 |
| curto | short | traduz | poph | 3 |
| decididas | decided | traduz | console | 3 |
| disposicao | layout | traduz | poph-server | 3 |
| dono | owner | traduz | poph-server | 3 |
| duplicado | duplicate | traduz | poph | 3 |
| efeito | effect | traduz | poph-server, poph | 3 |
| encerra | closes | traduz | raiz, console | 3 |
| encerrar | close | traduz | poph | 3 |
| entrar | enter | traduz | raiz, poph-server | 3 |
| escritor | writer | traduz | poph | 3 |
| esquema | schema | traduz | poph | 3 |
| estava | was | traduz | poph | 3 |
| extensao | extension | traduz | poph-server, poph | 3 |
| falhar | fail | traduz | poph-server | 3 |
| forcado | forced | traduz | poph | 3 |
| frase | sentence | traduz | poph-server, console | 3 |
| gerados | generated | traduz | console | 3 |
| idempotente | idempotent | traduz | poph | 3 |
| inalterados | unchanged | traduz | console | 3 |
| informado | informed | traduz | console, poph | 3 |
| intervalo | interval | traduz | poph, raiz | 3 |
| invalida | invalid | traduz | poph | 3 |
| legivel | readable | traduz | poph, poph-server | 3 |
| leitura | read | traduz | poph, poph-server | 3 |
| listagem | listing | traduz | poph-server | 3 |
| normalizar | normalise | traduz | poph | 3 |
| nulo | null | traduz | poph-server, poph | 3 |
| permitido | allowed | traduz | poph-server | 3 |
| pode | can | traduz | poph | 3 |
| posicao | position | traduz | poph, poph-server | 3 |
| presenca | presence | traduz | poph | 3 |
| problema | problem | traduz | raiz, poph | 3 |
| pronta | ready | traduz | poph-server | 3 |
| propria | own | traduz | poph, poph-server | 3 |
| recuo | backoff | traduz | poph | 3 |
| registrar | register | traduz | poph-server | 3 |
| relevancia | relevance | traduz | console | 3 |
| responde | answers | traduz | poph-server | 3 |
| resultados | results | traduz | poph-server | 3 |
| risco | risk | traduz | poph, raiz | 3 |
| rodadas | rounds | traduz | console | 3 |
| segue | follows | traduz | poph-server, poph | 3 |
| similaridade | similarity | traduz | console | 3 |
| subida | rise | traduz | poph-server | 3 |
| sufixo | suffix | traduz | poph | 3 |
| suporte | support | traduz | scripts | 3 |
| transicao | transition | traduz | poph-server | 3 |
| usuario | user | traduz | poph | 3 |
| velho | old | traduz | poph-server, poph | 3 |
| vincular | link | traduz | poph | 3 |
| volta | return | traduz | poph-server, poph | 3 |
| abstencao | abstention | traduz | poph | 2 |
| acento | accent | traduz | poph | 2 |
| achado | finding | traduz | poph | 2 |
| aguardando | waiting | traduz | poph-server | 2 |
| ajuste | adjustment | traduz | poph-server | 2 |
| alterar | modify | traduz | poph | 2 |
| avisos | notices | traduz | poph, poph-store | 2 |
| bloco | block | traduz | poph | 2 |
| buscar | search | traduz | poph | 2 |
| celula | cell | traduz | poph-server | 2 |
| coluna | column | traduz | poph | 2 |
| compartilhado | shared | traduz | poph | 2 |
| compila | compiles | traduz | poph, poph-server | 2 |
| conclusao | completion | traduz | poph-server | 2 |
| condicao | condition | traduz | poph-server | 2 |
| condicoes | conditions | traduz | console | 2 |
| contagem | count | traduz | poph | 2 |
| cursores | cursors | traduz | poph | 2 |
| derivado | derived | traduz | poph | 2 |
| entao | then | traduz | raiz | 2 |
| escolhas | choices | traduz | poph-server | 2 |
| escritas | writings | traduz | poph-server | 2 |
| estados | states | traduz | poph-server | 2 |
| faltando | missing | traduz | poph, poph-server | 2 |
| fecha | closes | traduz | poph-server | 2 |
| guardado | stored | traduz | poph | 2 |
| guardados | guarded | traduz | poph | 2 |
| idade | age | traduz | poph | 2 |
| identidade | identity | traduz | poph | 2 |
| indexacao | indexing | traduz | poph | 2 |
| indisponivel | unavailable | traduz | console | 2 |
| legenda | caption | traduz | poph-server | 2 |
| malformado | malformed | traduz | poph-server | 2 |
| mantem | keeps | traduz | poph, poph-server | 2 |
| marcar | mark | traduz | poph | 2 |
| maximo | maximum | traduz | poph | 2 |
| mes | month | traduz | poph | 2 |
| nenhuma | none | traduz | poph-server | 2 |
| paginacao | pagination | traduz | poph-server | 2 |
| parametros | parameters | traduz | poph-server | 2 |
| pareamento | pairing | traduz | poph-server | 2 |
| passagem | passage | traduz | poph-server | 2 |
| pede | asks | traduz | poph | 2 |
| prazo | deadline | traduz | poph-server | 2 |
| precisa | needs | traduz | poph, poph-server | 2 |
| proprias | own | traduz | poph | 2 |
| recusar | refuse | traduz | poph | 2 |
| revogar | revoke | traduz | poph | 2 |
| senao | else | traduz | raiz | 2 |
| sim | yes | traduz | poph | 2 |
| terceiro | third | traduz | poph-server | 2 |
| transferir | transfer | traduz | poph-server | 2 |
| truncar | truncate | traduz | poph | 2 |
| unidade | unit | traduz | console | 2 |
| usa | uses | traduz | poph, poph-server | 2 |
| vazia | empty | traduz | poph | 2 |
| voltou | returned | traduz | poph | 2 |
| abaixo | below | traduz | poph | 1 |
| abandonada | abandoned | traduz | poph-server | 1 |
| alteracao | change | traduz | poph | 1 |
| apagada | deleted | traduz | scripts | 1 |
| atendimentos | services | traduz | poph | 1 |
| atualizada | updated | traduz | poph-store | 1 |
| baixa | low | traduz | poph-server | 1 |
| ciclo | cycle | traduz | poph-server | 1 |
| comentarios | comments | traduz | poph | 1 |
| conexoes | connections | traduz | poph-server | 1 |
| confere | checks | traduz | poph | 1 |
| confirmar | confirm | traduz | poph-server | 1 |
| consultas | queries | traduz | poph-server | 1 |
| contato | contact | traduz | raiz | 1 |
| corpos | bodies | traduz | poph-server | 1 |
| cortado | cut | traduz | poph-server | 1 |
| declara | declares | traduz | poph-server | 1 |
| declarada | declared | traduz | poph-server | 1 |
| declarados | declared | traduz | poph-server | 1 |
| definitiva | definitive | traduz | poph-server | 1 |
| derrubar | drop | traduz | poph-server | 1 |
| dia | day | traduz | poph | 1 |
| edicao | edit | traduz | poph-server | 1 |
| encerramento | closure | traduz | poph-server | 1 |
| enganosa | misleading | traduz | poph | 1 |
| entra | enters | traduz | poph-server | 1 |
| equipe | team | traduz | poph-server | 1 |
| esgotadas | exhausted | traduz | poph-server | 1 |
| esta | this | traduz | poph | 1 |
| expirada | expired | traduz | poph-server | 1 |
| expor | expose | traduz | poph-server | 1 |
| fechamento | closing | traduz | poph-server | 1 |
| gravacao | recording | traduz | poph-server | 1 |
| habilitada | enabled | traduz | poph | 1 |
| habilitado | enabled | traduz | poph | 1 |
| humano | human | traduz | poph-store | 1 |
| ilegivel | unreadable | traduz | poph | 1 |
| inicia | starts | traduz | poph-server | 1 |
| instante | instant | traduz | poph | 1 |
| latencia | latency | traduz | poph | 1 |
| leva | takes | traduz | poph | 1 |
| limpar | clean | traduz | scripts | 1 |
| lugar | place | traduz | poph | 1 |
| mais | more | traduz | poph-server | 1 |
| medicao | measurement | traduz | scripts | 1 |
| migracoes | migrations | traduz | poph | 1 |
| minutos | minutes | traduz | scripts | 1 |
| muda | changes | traduz | poph | 1 |
| notificacao | notification | traduz | poph-server | 1 |
| organizacoes | organisations | traduz | poph-server | 1 |
| parar | stop | traduz | scripts | 1 |
| pausa | pause | traduz | poph-server | 1 |
| reais | actual | traduz | poph-server | 1 |
| recente | recent | traduz | poph-server | 1 |
| recusados | refused | traduz | poph | 1 |
| rede | network | traduz | poph-server | 1 |
| removido | removed | traduz | console | 1 |
| repeticao | repetition | traduz | poph-server | 1 |
| reserva | reservation | traduz | poph-server | 1 |
| saude | health | traduz | raiz | 1 |
| selecionado | selected | traduz | console | 1 |
| sistema | system | traduz | scripts | 1 |
| textos | texts | traduz | poph | 1 |
| trava | lock | traduz | poph-server | 1 |
| tres | three | traduz | poph-server | 1 |
| unico | single | traduz | poph-server | 1 |
| vezes | times | traduz | poph | 1 |
| vinculos | links | traduz | poph-server | 1 |
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
