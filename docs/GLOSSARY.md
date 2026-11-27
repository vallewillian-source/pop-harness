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
| texto | text | traduz | poph-agent, poph | 2776 |
| cenario | scenario | traduz | poph-server, poph | 2399 |
| agente | agent | traduz | poph-server, poph-agent | 1961 |
| conversa | conversation | traduz | poph-agent, poph-server | 1605 |
| nome | name | traduz | poph-flows, console | 1430 |
| estado | state | traduz | poph-server, poph-flows | 1407 |
| resultado | result | traduz | poph-store, poph-flows | 1105 |
| fluxo | flow | traduz | poph-agent, poph-server | 1039 |
| pasta | folder | traduz | poph, poph-server | 970 |
| erro | error | traduz | poph, poph-server | 963 |
| saida | output | traduz | poph, poph-server | 926 |
| corpo | body | traduz | poph-server, poph | 837 |
| tipo | kind | traduz | poph-store, poph-server | 776 |
| mensagem | message | traduz | poph-agent, poph-store | 756 |
| campo | field | traduz | poph-flows, console | 755 |
| servidor | server | traduz | console, poph-server | 698 |
| turno | turn | traduz | poph-agent, console | 698 |
| evento | event | traduz | console, poph-agent | 695 |
| chave | key | traduz | poph, poph-server | 693 |
| de | of | traduz | poph-server, poph | 670 |
| resposta | response | traduz | poph-server, poph | 668 |
| registro | record | traduz | poph-agent, console | 607 |
| acao | action | traduz | poph, poph-flows | 601 |
| dados | data | traduz | poph-flows, console | 592 |
| etapa | step | traduz | poph-flows, poph-agent | 565 |
| criar | create | traduz | poph-server, poph | 557 |
| motivo | reason | traduz | poph-agent, poph-jev | 545 |
| arquivo | file | traduz | poph, poph-server | 537 |
| pedir | ask | traduz | poph-server, poph | 516 |
| caminho | path | traduz | poph-server, poph | 488 |
| gatilho | trigger | traduz | poph-agent, poph-flows | 461 |
| ficha | case | traduz | poph-server, poph-agent | 458 |
| valor | value | traduz | poph, poph-server | 455 |
| com | with | traduz | poph-server, poph | 445 |
| ambiente | environment | traduz | poph | 406 |
| fonte | source | traduz | poph-flows, poph-store | 381 |
| decisao | decision | traduz | poph-jev, poph-flows | 376 |
| versao | version | traduz | poph-server, console | 375 |
| departamento | department | traduz | poph, poph-agent | 370 |
| novo | new | traduz | poph-server, poph | 370 |
| provedor | provider | traduz | poph-agent, poph-server | 330 |
| falha | failure | traduz | poph, poph-server | 327 |
| destino | target | traduz | poph-server, poph | 317 |
| integracao | integration | traduz | poph-server, console | 313 |
| sem | without | traduz | poph-server, poph | 313 |
| desligar | disable | traduz | poph-server, poph | 311 |
| falso | false | traduz | poph-server, poph | 305 |
| fim | end | traduz | poph-flows, poph-agent | 305 |
| cliente | client | traduz | poph-server, poph | 293 |
| situacao | situation | traduz | console, poph-retention | 291 |
| modelo | model | traduz | poph-agent, poph-server | 282 |
| da | of | traduz | poph-server, poph | 268 |
| itens | items | traduz | poph-server, console | 266 |
| relatorio | report | traduz | poph, poph-server | 259 |
| whatsapp | whatsapp | traduz | console, poph-server | 258 |
| configuracao | configuration | traduz | poph, console | 255 |
| canal | channel | traduz | poph-server, poph-store | 253 |
| em | in | traduz | poph-store, poph-server | 252 |
| linha | line | traduz | poph-server, poph | 247 |
| limites | limits | traduz | poph-flows, poph-agent | 241 |
| numero | number | traduz | console, poph-store | 232 |
| grupo | group | traduz | poph, poph-server | 230 |
| atual | current | traduz | poph-flows, poph-server | 228 |
| codigo | code | traduz | poph, poph-server | 223 |
| metodo | method | traduz | poph-server, poph | 210 |
| nao | not | traduz | poph-server, poph | 206 |
| instancia | instance | traduz | poph-server, poph-evolution | 203 |
| rotulo | label | traduz | poph-server, console | 200 |
| alvo | target | traduz | poph-server, poph | 199 |
| rota | route | traduz | poph-server | 197 |
| campos | fields | traduz | poph-flows, console | 196 |
| validar | validate | traduz | poph-types, poph | 191 |
| sala | room | traduz | poph-server, poph | 190 |
| descricao | description | traduz | console, poph-agent | 185 |
| ator | actor | traduz | poph-store, poph-server | 184 |
| caso | case | traduz | poph-server, poph | 183 |
| cru | raw | traduz | poph-server | 180 |
| externo | external | traduz | poph-server, poph-agent | 178 |
| execucao | execution | traduz | poph-server, poph-store | 174 |
| modo | mode | traduz | poph-server, console | 173 |
| pedido | request | traduz | poph-server, poph | 173 |
| vazio | empty | traduz | poph-server, poph-flows | 172 |
| categoria | category | traduz | poph, poph-server | 160 |
| subir | raise | traduz | poph-server, poph | 159 |
| para | for | traduz | poph-flows, poph-server | 153 |
| lista | list | traduz | poph, poph-server | 150 |
| produto | product | traduz | poph, poph-server | 150 |
| responder | reply | traduz | poph-server, poph | 147 |
| origem | origin | traduz | poph, console | 144 |
| segredo | secret | traduz | poph-server, poph | 138 |
| agentes | agents | traduz | poph, poph-server | 135 |
| aviso | notice | traduz | poph-server, poph | 134 |
| depois | after | traduz | poph-server, console | 133 |
| segredos | secrets | traduz | poph, console | 133 |
| por | by | traduz | poph, poph-server | 132 |
| esperar | wait | traduz | poph-server, poph | 128 |
| fluxos | flows | traduz | poph, poph-agent | 126 |
| metricas | metrics | traduz | poph-server, poph | 124 |
| porta | port | traduz | poph-server, poph | 124 |
| abrir | open | traduz | poph, poph-server | 119 |
| antes | before | traduz | poph-server, console | 114 |
| conteudo | content | traduz | poph, poph-server | 113 |
| falsa | false | traduz | poph-server, poph | 113 |
| endereco | address | traduz | poph-server, poph | 112 |
| primeira | first | traduz | poph-server, poph | 112 |
| revisao | revision | traduz | poph-server, poph-store | 107 |
| na | towards | traduz | poph-server, poph | 106 |
| relato | report | traduz | poph-server | 105 |
| ao | to | traduz | poph-server, poph | 103 |
| sinal | signal | traduz | poph-server, poph | 103 |
| detalhe | detail | traduz | poph, poph-server | 101 |
| nivel | level | traduz | poph, console | 99 |
| envios | sends | traduz | poph-server | 98 |
| espaco | space | traduz | console, poph-server | 95 |
| ordem | order | traduz | console, poph-server | 95 |
| criado | created | traduz | poph-server, poph | 94 |
| escritas | writings | traduz | poph-server | 94 |
| avisos | notices | traduz | poph-server, poph | 93 |
| guarda | guard | traduz | poph-server, poph | 93 |
| mensagens | messages | traduz | poph-server, scripts | 93 |
| conversas | conversations | traduz | poph-server, poph-store | 92 |
| descrever | describe | traduz | poph | 92 |
| registros | records | traduz | poph-server | 91 |
| lido | read | traduz | poph, poph-server | 89 |
| manifesto | manifest | traduz | console, poph-server | 88 |
| imprimir | print | traduz | poph | 87 |
| motor | engine | traduz | poph-server, poph | 87 |
| outro | other | traduz | poph-server, poph | 87 |
| captura | capture | traduz | poph-server, poph | 86 |
| resolve | resolves | traduz | scripts, poph-flows | 86 |
| chamadas | calls | traduz | poph-server | 85 |
| falhas | failures | traduz | poph-server, poph-store | 85 |
| espelho | mirror | traduz | poph-server | 84 |
| inicio | beginning | traduz | poph-server, poph | 84 |
| mundo | world | traduz | poph-server | 84 |
| indice | index | traduz | poph, poph-server | 81 |
| instrucoes | instructions | traduz | poph, poph-server | 81 |
| listar | list | traduz | poph, poph-server | 81 |
| fechada | closed | traduz | poph-store, poph-server | 80 |
| outra | other | traduz | poph-server, poph | 80 |
| recusa | refusal | traduz | poph-server, poph | 79 |
| cabecalhos | headers | traduz | poph-server, poph | 78 |
| pagina | page | traduz | poph, poph-server | 78 |
| sessao | session | traduz | poph, console | 78 |
| segunda | second | traduz | poph-server, poph | 75 |
| teste | test | traduz | poph, poph-server | 75 |
| mapa | map | traduz | poph-server, poph | 73 |
| efeitos | effects | traduz | poph-server, poph-store | 70 |
| eventos | events | traduz | poph-server, poph-store | 69 |
| tentativas | attempts | traduz | poph-server, poph-agent | 69 |
| executar | run | traduz | poph, poph-server | 68 |
| transacao | transaction | traduz | poph-server, scripts | 68 |
| comando | command | traduz | poph | 67 |
| primeiro | first | traduz | poph-server, poph | 66 |
| enviar | send | traduz | poph-server, poph | 65 |
| idempotencia | idempotency | traduz | poph-store, poph-server | 65 |
| igual | equal | traduz | poph-flows, poph-server | 64 |
| ler | read | traduz | poph, poph-server | 64 |
| modulo | module | traduz | poph-server, poph | 64 |
| relogio | clock | traduz | poph-server | 63 |
| fila | queue | traduz | poph-server, poph | 62 |
| evolucao | evolution | traduz | poph-server | 60 |
| textos | texts | traduz | poph-server, poph | 60 |
| ultimo | last | traduz | poph-server, poph-store | 60 |
| aberta | open | traduz | poph-server, poph-store | 59 |
| entrega | delivery | traduz | poph-server | 59 |
| ativo | active | traduz | poph-agent, console | 58 |
| conexao | connection | traduz | poph, poph-server | 58 |
| esperado | expected | traduz | poph, poph-server | 58 |
| limite | limit | traduz | poph, poph-server | 58 |
| anexos | attachments | traduz | poph-server, poph-agent | 57 |
| dias | days | traduz | console, poph-retention | 57 |
| documento | document | traduz | poph, poph-server | 57 |
| entrada | entry | traduz | poph, poph-server | 57 |
| agora | now | traduz | poph-server, poph | 56 |
| instancias | instances | traduz | console, poph-server | 55 |
| arquivos | files | traduz | poph-server, poph | 54 |
| catalogo | catalog | traduz | poph, poph-server | 54 |
| ligar | link | traduz | poph-server, poph | 54 |
| carregar | load | traduz | poph, poph-server | 53 |
| desligado | disabled | traduz | poph, poph-server | 53 |
| aparelho | device | traduz | poph-server | 52 |
| gravado | stored | traduz | poph, poph-server | 52 |
| linhas | lines | traduz | poph, poph-server | 51 |
| atualizar | update | traduz | poph, console | 50 |
| enviado | sent | traduz | poph-server, poph-store | 50 |
| fora | outside | traduz | poph-server, poph | 50 |
| diretorio | directory | traduz | poph-server, poph-rag | 49 |
| par | pair | traduz | poph, poph-server | 49 |
| variavel | variable | traduz | poph, poph-server | 49 |
| cena | scene | traduz | poph-server | 48 |
| definir | define | traduz | poph-server, poph | 48 |
| gravada | stored | traduz | poph-server, poph | 48 |
| ligado | linked | traduz | poph-server, poph | 48 |
| que | that | traduz | poph-server, poph | 48 |
| resumo | summary | traduz | poph, poph-server | 48 |
| chamada | call | traduz | poph-server, console | 47 |
| consulta | query | traduz | poph-server, poph | 47 |
| contem | contains | traduz | poph-flows, poph-files | 47 |
| pedidos | requests | traduz | poph-server, poph | 47 |
| formato | format | traduz | poph-store, poph | 46 |
| duracao | duration | traduz | poph-server, console | 45 |
| fichas | cases | traduz | poph-store, poph-server | 45 |
| maior | bigger | traduz | poph-flows, poph-server | 45 |
| midias | media | traduz | poph-server | 45 |
| mostrar | show | traduz | poph | 45 |
| retrato | snapshot | traduz | poph | 45 |
| turnos | turns | traduz | poph-server, raiz | 45 |
| pendentes | pending | traduz | poph-server, poph-store | 44 |
| publicar | publish | traduz | poph-server, poph | 44 |
| auditoria | audit | traduz | poph-server | 43 |
| uma | a | traduz | poph-server, poph | 43 |
| inexistente | missing | traduz | poph, poph-server | 42 |
| produtos | products | traduz | poph, console | 42 |
| vivo | live | traduz | poph, poph-server | 42 |
| banco | database | traduz | poph, poph-server | 41 |
| escrita | writing | traduz | poph-server, poph | 41 |
| respostas | responses | traduz | poph-server | 41 |
| titulo | title | traduz | console, poph | 41 |
| atendente | attendant | traduz | poph-server, scripts | 40 |
| contexto | context | traduz | poph, poph-server | 40 |
| funcao | function | traduz | poph-flows, raiz | 40 |
| padrao | default | traduz | poph, poph-server | 40 |
| referencia | reference | traduz | poph, poph-server | 40 |
| ate | until | traduz | poph-server, scripts | 39 |
| carimbo | stamp | traduz | poph-server, poph | 39 |
| falhar | fail | traduz | poph-server | 39 |
| metadados | metadata | traduz | poph-files, poph | 39 |
| processo | process | traduz | poph-server, poph | 39 |
| roteiro | script | traduz | poph-server | 39 |
| tem | has | traduz | poph, console | 39 |
| categorias | categories | traduz | poph, poph-server | 38 |
| escrever | write | traduz | poph, console | 38 |
| tabelas | tables | traduz | console, poph | 38 |
| temperatura | temperature | traduz | poph-core, poph-store | 38 |
| criados | created | traduz | poph-server | 37 |
| marcador | marker | traduz | poph-server | 37 |
| proxima | next | traduz | poph-server, poph-store | 37 |
| servir | serve | traduz | poph-server, poph | 37 |
| comparar | compare | traduz | poph-server, poph | 36 |
| mostra | shows | traduz | poph, poph-server | 36 |
| pela | by | traduz | poph-server, poph | 36 |
| preparo | preparation | traduz | poph, poph-server | 36 |
| como | as | traduz | poph, poph-server | 35 |
| excluir | delete | traduz | poph, poph-server | 35 |
| globais | globals | traduz | poph-server, poph-types | 35 |
| opcoes | options | traduz | poph, poph-server | 35 |
| baixar | download | traduz | poph-server, poph | 34 |
| iniciar | start | traduz | poph-server, poph | 34 |
| raiz | root | traduz | poph-server, poph | 34 |
| definicao | definition | traduz | poph-store, poph-server | 33 |
| entregar | deliver | traduz | poph-server | 33 |
| envio | send | traduz | poph-server | 33 |
| gerar | generate | traduz | poph | 33 |
| rascunho | draft | traduz | poph-store, poph-server | 33 |
| rodando | running | traduz | poph-server | 33 |
| verificacao | verification | traduz | poph | 33 |
| compilado | compiled | traduz | poph-server, poph | 32 |
| nomes | names | traduz | poph, poph-server | 32 |
| recusado | refused | traduz | poph-server, poph | 32 |
| retencao | retention | traduz | console, poph | 32 |
| segundo | second | traduz | poph-server, poph | 32 |
| tabela | table | traduz | poph, poph-server | 32 |
| existe | exists | traduz | poph-server, poph | 31 |
| forcar | force | traduz | console, poph | 31 |
| historico | history | traduz | poph-server, poph | 31 |
| modelos | models | traduz | poph, poph-server | 31 |
| ou | or | traduz | poph-flows, poph | 31 |
| partes | parts | traduz | poph-server, poph | 31 |
| repetida | repeated | traduz | poph-server, poph | 31 |
| servico | service | traduz | poph-server | 31 |
| anexo | attachment | traduz | poph-server, poph | 30 |
| apoio | support | traduz | poph | 30 |
| capturados | captured | traduz | poph-server | 30 |
| divergencias | divergences | traduz | poph, poph-server | 30 |
| invalido | invalid | traduz | poph-server, poph | 30 |
| laco | loop | traduz | poph-server | 30 |
| liberar | release | traduz | poph, console | 30 |
| verificacoes | verifications | traduz | poph | 30 |
| canais | channels | traduz | poph-server, poph | 29 |
| midia | media | traduz | poph-server, poph-store | 29 |
| ouvinte | listener | traduz | poph-server, poph | 29 |
| encaminhar | forward | traduz | poph-flows, poph-server | 28 |
| espacos | spaces | traduz | poph-server, poph | 28 |
| grande | large | traduz | poph-server, poph | 28 |
| importar | import | traduz | poph-server, poph | 28 |
| proibido | forbidden | traduz | poph-server, poph | 28 |
| resolucoes | resolutions | traduz | poph-store, poph-server | 28 |
| resto | rest | traduz | poph, poph-server | 28 |
| segurar | hold | traduz | poph-server | 28 |
| vizinha | neighbouring | traduz | poph-server, poph | 28 |
| escolha | choice | traduz | poph, console | 27 |
| execucoes | executions | traduz | poph-store, poph-server | 27 |
| fantasma | ghost | traduz | poph-server | 27 |
| gravar | write | traduz | poph-server, poph | 27 |
| loja | store | traduz | poph-server | 27 |
| remoto | remote | traduz | poph | 27 |
| rodada | round | traduz | poph-server, scripts | 27 |
| cabecalho | header | traduz | poph-server | 26 |
| contar | count | traduz | poph-server, poph | 26 |
| cursores | cursors | traduz | poph-server, poph | 26 |
| desfecho | outcome | traduz | poph-agent, poph-store | 26 |
| fontes | sources | traduz | poph, console | 26 |
| montagem | assembly | traduz | poph, console | 26 |
| publicadas | published | traduz | poph-server | 26 |
| recebido | received | traduz | poph-server, poph | 26 |
| criada | created | traduz | poph-server, poph | 25 |
| quando | when | traduz | raiz, poph-server | 25 |
| quem | who | traduz | poph-server, poph | 25 |
| vai | goes | traduz | raiz, poph-server | 25 |
| avulsa | one_off | traduz | poph-server | 24 |
| bruta | raw | traduz | poph-server, poph | 24 |
| carrinho | cart | traduz | poph-server, console | 24 |
| chamar | call | traduz | poph-server | 24 |
| criacao | creation | traduz | poph-server, poph | 24 |
| esperados | expected | traduz | poph-server | 24 |
| nada | nothing | traduz | poph-server, poph | 24 |
| problemas | problems | traduz | poph, console | 24 |
| relatorios | reports | traduz | poph | 24 |
| remetente | sender | traduz | console, poph-evolution | 24 |
| retentativas | retries | traduz | poph-jev, poph-core | 24 |
| roteador | router | traduz | poph-server | 24 |
| diagnostico | diagnostic | traduz | poph, poph-server | 23 |
| errado | wrong | traduz | poph-server, poph | 23 |
| montar | build | traduz | poph-server, poph | 23 |
| pedaco | piece | traduz | poph-server, poph | 23 |
| tarefa | task | traduz | poph-server | 23 |
| vira | becomes | traduz | poph-server, poph | 23 |
| casos | cases | traduz | poph-server, poph | 22 |
| decodificar | decode | traduz | poph-server, poph | 22 |
| desligada | disabled | traduz | poph-server | 22 |
| dona | owner | traduz | poph-server | 22 |
| enviados | sent | traduz | poph-server | 22 |
| fechar | close | traduz | poph-server, poph | 22 |
| filtro | filter | traduz | poph, poph-server | 22 |
| integracoes | integrations | traduz | poph-server, poph | 22 |
| mudanca | change | traduz | poph, poph-server | 22 |
| obter | get | traduz | poph-server, poph | 22 |
| pendente | pending | traduz | poph-server, poph | 22 |
| roteiros | scripts | traduz | poph-server | 22 |
| saidas | outputs | traduz | poph-server, poph | 22 |
| trechos | passages | traduz | console, poph | 22 |
| autorizacao | authorization | traduz | poph-server, poph | 21 |
| diferente | different | traduz | poph-flows, poph | 21 |
| disfarce | disguise | traduz | poph-server | 21 |
| escopo | scope | traduz | console, poph-server | 21 |
| gravadas | stored | traduz | poph-server | 21 |
| ja | already | traduz | poph-server, poph | 21 |
| processador | processor | traduz | poph-server, poph | 21 |
| semear | seed | traduz | poph-server, poph | 21 |
| teto | cap | traduz | poph-server, poph | 21 |
| todos | all | traduz | poph, poph-server | 21 |
| vez | time | traduz | poph-server, poph | 21 |
| erros | errors | traduz | poph-server, poph | 20 |
| exige | requires | traduz | raiz, poph-server | 20 |
| ferramentas | tools | traduz | console, poph | 20 |
| instrucao | instruction | traduz | raiz, poph-server | 20 |
| lote | batch | traduz | poph, poph-server | 20 |
| mostrado | shown | traduz | poph, poph-server | 20 |
| mudou | changed | traduz | console, poph | 20 |
| ausente | missing | traduz | poph, poph-server | 19 |
| final | final | traduz | poph-matrix, console | 19 |
| nem | nor | traduz | poph-server, poph | 19 |
| proprio | own | traduz | poph-server, poph | 19 |
| tamanho | size | traduz | poph-server, poph | 19 |
| argumentos | arguments | traduz | console, poph-server | 18 |
| configurar | configure | traduz | poph-server, poph | 18 |
| convite | invitation | traduz | poph-server, poph | 18 |
| diagnosticos | diagnostics | traduz | poph, poph-server | 18 |
| entregue | delivered | traduz | poph-server | 18 |
| livre | free | traduz | poph-server, poph | 18 |
| nova | new | traduz | poph-server, poph | 18 |
| portugues | portuguese | traduz | poph | 18 |
| publicada | published | traduz | poph-store, poph-server | 18 |
| redacoes | writings | traduz | poph-server | 18 |
| responde | answers | traduz | poph-server | 18 |
| ruim | poor | traduz | poph, poph-server | 18 |
| temporaria | temporary | traduz | poph | 18 |
| achados | findings | traduz | poph-server, poph | 17 |
| atores | actors | traduz | poph-server, poph | 17 |
| atraso | delay | traduz | poph-server, poph | 17 |
| ciclo | cycle | traduz | poph-server | 17 |
| conferir | check | traduz | poph, poph-server | 17 |
| departamentos | departments | traduz | poph, poph-server | 17 |
| despedida | farewell | traduz | console | 17 |
| entradas | entries | traduz | poph-server | 17 |
| exemplos | examples | traduz | raiz, console | 17 |
| fica | stays | traduz | poph-server, poph | 17 |
| humana | human | traduz | poph-server | 17 |
| ligada | linked | traduz | poph-server | 17 |
| pelo | by | traduz | poph-server | 17 |
| perfis | profiles | traduz | poph-server | 17 |
| renomeacoes | renames | traduz | poph-server | 17 |
| rotas | routes | traduz | poph-server | 17 |
| acoes | actions | traduz | raiz | 16 |
| atualizadas | updated | traduz | console, poph-server | 16 |
| convidado | guest | traduz | poph-server, poph | 16 |
| derrubar | drop | traduz | poph-server | 16 |
| recusada | refused | traduz | poph, poph-server | 16 |
| reindexar | reindex | traduz | poph, console | 16 |
| relativo | relative | traduz | poph-server, poph | 16 |
| todas | all | traduz | poph-server, console | 16 |
| valores | values | traduz | poph-server | 16 |
| ver | see | traduz | poph-server | 16 |
| vitima | victim | traduz | poph-server | 16 |
| aberto | open | traduz | poph-server | 15 |
| adiado | deferred | traduz | poph-server | 15 |
| alterado | changed | traduz | poph-server, poph | 15 |
| conjunto | set | traduz | poph, poph-server | 15 |
| dado | given | traduz | poph-server, poph | 15 |
| duas | two | traduz | poph-server, poph | 15 |
| escalares | scalars | traduz | poph | 15 |
| fantasmas | ghosts | traduz | poph-server | 15 |
| ferramenta | tool | traduz | poph, poph-server | 15 |
| grupos | groups | traduz | poph | 15 |
| inseridas | inserted | traduz | console, poph-server | 15 |
| ligacao | link | traduz | poph | 15 |
| menor | smaller | traduz | poph-flows | 15 |
| sobrescrever | overwrite | traduz | console, poph | 15 |
| tentativa | attempt | traduz | poph-store, poph-server | 15 |
| transitoria | transient | traduz | poph-server | 15 |
| velha | old | traduz | poph-server, poph | 15 |
| volta | return | traduz | poph-server, poph | 15 |
| assinados | signed | traduz | poph-server | 14 |
| atender | attend | traduz | poph-server, poph | 14 |
| ativos | active | traduz | poph-server, poph | 14 |
| caractere | character | traduz | poph, poph-server | 14 |
| colecao | collection | traduz | poph-server | 14 |
| comercial | commercial | traduz | poph-server, scripts | 14 |
| conferido | checked | traduz | poph | 14 |
| confianca | confidence | traduz | poph, console | 14 |
| exigir | require | traduz | poph | 14 |
| fechado | closed | traduz | poph-server | 14 |
| formatar | format | traduz | poph, console | 14 |
| impresso | printed | traduz | poph | 14 |
| lida | read | traduz | poph-server | 14 |
| nenhum | none | traduz | poph-server | 14 |
| posicoes | positions | traduz | poph-server | 14 |
| pronta | ready | traduz | poph-server | 14 |
| trecho | passage | traduz | poph, poph-server | 14 |
| unica | single | traduz | poph-server, poph | 14 |
| vazamento | leak | traduz | poph-server | 14 |
| veneno | poison | traduz | poph-server | 14 |
| ajustes | adjustments | traduz | poph-server, poph | 13 |
| anterior | previous | traduz | poph-server | 13 |
| apaga | deletes | traduz | poph-server, poph | 13 |
| aparece | appears | traduz | poph-server, poph | 13 |
| biblioteca | library | traduz | poph-server, poph | 13 |
| busca | search | traduz | poph, poph-server | 13 |
| desconhecida | unknown | traduz | poph-server, console | 13 |
| encerrada | closed | traduz | poph-server | 13 |
| espelhado | mirrored | traduz | poph-server, poph-store | 13 |
| esperada | expected | traduz | poph, poph-server | 13 |
| extrator | extractor | traduz | poph-files, scripts | 13 |
| filho | child | traduz | poph, poph-server | 13 |
| isolamento | isolation | traduz | poph-server | 13 |
| marca | mark | traduz | poph-server, poph | 13 |
| notificar | notify | traduz | poph-server, raiz | 13 |
| obrigatorio | required | traduz | raiz, poph | 13 |
| prefixo | prefix | traduz | poph, poph-server | 13 |
| quantos | how_many | traduz | poph-server | 13 |
| revisoes | revisions | traduz | poph-store, poph-server | 13 |
| testes | tests | traduz | poph | 13 |
| tipos | kinds | traduz | poph-server | 13 |
| validacao | validation | traduz | poph | 13 |
| versoes | versions | traduz | poph-server, poph | 13 |
| atualizado | updated | traduz | poph-server | 12 |
| avancar | advance | traduz | poph-server | 12 |
| baixado | downloaded | traduz | poph-server, poph | 12 |
| bloco | block | traduz | poph-server, poph | 12 |
| condicao | condition | traduz | poph-server | 12 |
| criadas | created | traduz | poph-server | 12 |
| degradada | degraded | traduz | console | 12 |
| estruturado | structured | traduz | poph-server, poph | 12 |
| exportar | export | traduz | poph, poph-server | 12 |
| falhou | failed | traduz | poph, poph-server | 12 |
| imagem | image | traduz | poph-server | 12 |
| importacao | import | traduz | poph, poph-server | 12 |
| inativo | inactive | traduz | poph, poph-server | 12 |
| inteiro | integer | traduz | poph | 12 |
| leitor | reader | traduz | poph | 12 |
| nos | us | traduz | poph-flows, poph-server | 12 |
| preco | price | traduz | console, poph-server | 12 |
| recebidos | received | traduz | poph-server | 12 |
| repetido | repeated | traduz | poph, poph-server | 12 |
| saltos | jumps | traduz | poph-server, poph-flows | 12 |
| trava | lock | traduz | poph-server | 12 |
| ultima | last | traduz | poph-server, poph | 12 |
| usuario | user | traduz | poph-server, poph | 12 |
| abertura | opening | traduz | poph-server, poph | 11 |
| ajuda | help | traduz | poph, scripts | 11 |
| apos | after | traduz | poph-server, poph | 11 |
| chaves | keys | traduz | poph | 11 |
| comparacao | comparison | traduz | poph, poph-server | 11 |
| configurado | configured | traduz | console, poph | 11 |
| conflito | conflict | traduz | poph, poph-server | 11 |
| conta | counts | traduz | poph-server, poph | 11 |
| corte | cutoff | traduz | poph-server, poph | 11 |
| das | of | traduz | poph-server, poph | 11 |
| devolve | returns | traduz | poph-server | 11 |
| diz | says | traduz | poph, poph-server | 11 |
| dois | two | traduz | poph-server, poph | 11 |
| encerrar | close | traduz | poph-server, poph | 11 |
| esperadas | expected | traduz | poph-server, poph | 11 |
| etapas | steps | traduz | poph-server, poph-store | 11 |
| feita | done | traduz | poph | 11 |
| foto | photo | traduz | poph-server, poph | 11 |
| humano | human | traduz | poph-server, poph-store | 11 |
| juntos | together | traduz | poph-server, poph | 11 |
| leituras | reads | traduz | poph-server | 11 |
| memoria | memory | traduz | poph, poph-server | 11 |
| metrica | metric | traduz | poph-server | 11 |
| orfaos | orphans | traduz | poph | 11 |
| posicao | position | traduz | poph-server, poph | 11 |
| receptor | receiver | traduz | poph-server | 11 |
| recusar | refuse | traduz | poph-server, poph | 11 |
| redacao | writing | traduz | poph-server | 11 |
| salas | rooms | traduz | scripts, poph-server | 11 |
| situacoes | situations | traduz | console | 11 |
| variaveis | variables | traduz | poph | 11 |
| vinculo | link | traduz | poph-server, poph | 11 |
| acima | above | traduz | poph-server, poph | 10 |
| amostras | samples | traduz | poph-server | 10 |
| bruto | raw | traduz | poph-server | 10 |
| dentro | inside | traduz | poph, poph-server | 10 |
| direto | direct | traduz | poph-server, poph | 10 |
| espera | wait | traduz | poph-server | 10 |
| finalizada | finalised | traduz | poph-store, poph-agent | 10 |
| gravados | stored | traduz | poph-server, console | 10 |
| liberado | released | traduz | poph-server, poph-store | 10 |
| limpo | clean | traduz | poph, poph-server | 10 |
| mesma | same | traduz | poph-server | 10 |
| mesmo | same | traduz | poph, poph-server | 10 |
| operacao | operation | traduz | poph-server, poph | 10 |
| passagem | passage | traduz | poph-server | 10 |
| preparar | prepare | traduz | poph-server, poph | 10 |
| quente | hot | traduz | poph, poph-server | 10 |
| registrada | recorded | traduz | poph-server | 10 |
| secao | section | traduz | poph, poph-server | 10 |
| segmento | segment | traduz | poph-server, poph | 10 |
| segura | safe | traduz | poph-server, console | 10 |
| terceira | third | traduz | poph-server | 10 |
| terceiro | third | traduz | poph-server | 10 |
| uso | use | traduz | poph | 10 |
| verificar | verify | traduz | poph, poph-server | 10 |
| vistos | seen | traduz | poph-server | 10 |
| alheio | foreign | traduz | poph-server | 9 |
| celulas | cells | traduz | poph-server | 9 |
| conectado | connected | traduz | poph-server | 9 |
| contadores | counters | traduz | poph-server | 9 |
| cria | creates | traduz | poph, poph-server | 9 |
| enfileirar | enqueue | traduz | poph-server | 9 |
| invalida | invalid | traduz | poph-server, poph | 9 |
| meio | middle | traduz | poph-server | 9 |
| metodos | methods | traduz | poph-server | 9 |
| nunca | never | traduz | poph-server, poph | 9 |
| orientacao | guidance | traduz | raiz, poph | 9 |
| preenchidos | filled | traduz | console | 9 |
| publicado | published | traduz | poph-server | 9 |
| transferencia | transfer | traduz | poph-server | 9 |
| aceita | accepts | traduz | poph, poph-server | 8 |
| adicionadas | added | traduz | console | 8 |
| apagadas | deleted | traduz | poph-server | 8 |
| apertado | tight | traduz | poph-server | 8 |
| aspas | quotes | traduz | poph | 8 |
| contador | counter | traduz | poph-server | 8 |
| exclusao | exclusion | traduz | poph-server, poph | 8 |
| gerado | generated | traduz | poph, poph-server | 8 |
| instalado | installed | traduz | poph-server, poph | 8 |
| limitador | limiter | traduz | poph-server | 8 |
| operacoes | operations | traduz | poph, poph-server | 8 |
| parar | stop | traduz | poph-server, scripts | 8 |
| parte | part | traduz | poph, poph-server | 8 |
| permissao | permission | traduz | poph | 8 |
| recarregar | reload | traduz | poph-server | 8 |
| recurso | resource | traduz | poph-server | 8 |
| removidas | removed | traduz | console | 8 |
| repetir | repeat | traduz | poph-server, poph | 8 |
| simulacao | simulation | traduz | poph | 8 |
| sincronizar | synchronise | traduz | poph | 8 |
| tarefas | tasks | traduz | poph-server | 8 |
| terminar | finish | traduz | poph-server | 8 |
| trocado | swapped | traduz | poph-server, poph | 8 |
| trocaram | swapped | traduz | console | 8 |
| unico | single | traduz | poph-server | 8 |
| valido | valid | traduz | poph, poph-server | 8 |
| visita | visit | traduz | poph-server | 8 |
| abandonar | abandon | traduz | poph-server | 7 |
| anotadora | annotator | traduz | poph-server | 7 |
| atualizacoes | updates | traduz | poph | 7 |
| buscar | search | traduz | poph-server, poph | 7 |
| cada | each | traduz | poph-server, poph | 7 |
| conhecimento | knowledge | traduz | console, poph-server | 7 |
| deixa | leaves | traduz | poph-server, poph | 7 |
| desconectar | disconnect | traduz | poph-server, console | 7 |
| disposicao | layout | traduz | poph-server | 7 |
| dos | of | traduz | poph, poph-server | 7 |
| encontrado | found | traduz | poph | 7 |
| entidade | entity | traduz | poph-server, poph | 7 |
| entre | between | traduz | poph-server, poph | 7 |
| fala | speaks | traduz | poph-server | 7 |
| forma | form | traduz | poph | 7 |
| janela | window | traduz | poph-server, poph | 7 |
| novas | new | traduz | poph-server | 7 |
| pares | pairs | traduz | poph | 7 |
| propria | own | traduz | poph-server, poph | 7 |
| quantas | how_many | traduz | poph-server | 7 |
| segue | follows | traduz | poph-server, poph | 7 |
| suporte | support | traduz | poph-server, scripts | 7 |
| telefone | phone | traduz | poph-server | 7 |
| vezes | times | traduz | poph-server, poph | 7 |
| vinculado | linked | traduz | poph, poph-server | 7 |
| apagado | deleted | traduz | poph | 6 |
| armazem | store | traduz | poph | 6 |
| atendimento | service | traduz | scripts, raiz | 6 |
| autorizado | authorized | traduz | poph-server, console | 6 |
| avaliador | evaluator | traduz | poph-server, poph | 6 |
| carrinhos | carts | traduz | console | 6 |
| cifrados | encrypted | traduz | poph | 6 |
| construir | build | traduz | poph, poph-server | 6 |
| continua | continues | traduz | poph-server, poph | 6 |
| corpos | bodies | traduz | poph-server | 6 |
| falar | speak | traduz | poph-server | 6 |
| falta | missing | traduz | poph, poph-server | 6 |
| filhos | children | traduz | poph-server | 6 |
| inicial | initial | traduz | poph-server | 6 |
| leitura | read | traduz | poph-server, poph | 6 |
| liberados | released | traduz | poph-server, poph | 6 |
| observador | observer | traduz | poph | 6 |
| onde | where | traduz | poph, console | 6 |
| pai | parent | traduz | poph | 6 |
| parado | stopped | traduz | poph, poph-server | 6 |
| passa | passes | traduz | poph-server, poph | 6 |
| pode | can | traduz | poph, poph-server | 6 |
| quantidade | quantity | traduz | poph-server | 6 |
| reabrir | reopen | traduz | poph-server | 6 |
| regra | rule | traduz | poph-server, raiz | 6 |
| retirar | withdraw | traduz | poph | 6 |
| sao | are | traduz | poph-server, poph | 6 |
| segundos | seconds | traduz | poph, raiz | 6 |
| sonda | probe | traduz | poph-server | 6 |
| subida | rise | traduz | poph-server | 6 |
| transferida | transferred | traduz | poph-server | 6 |
| transporte | transport | traduz | poph | 6 |
| usada | used | traduz | poph-server | 6 |
| agir | act | traduz | poph-server | 5 |
| apagar | delete | traduz | poph, poph-server | 5 |
| avisar | notify | traduz | console | 5 |
| canonico | canonical | traduz | poph-server | 5 |
| caracteres | characters | traduz | poph, poph-server | 5 |
| centavos | cents | traduz | console, poph-server | 5 |
| cifra | cipher | traduz | poph | 5 |
| conflitos | conflicts | traduz | poph-server, poph | 5 |
| controle | control | traduz | poph-server | 5 |
| decisoes | decisions | traduz | console | 5 |
| definitiva | definitive | traduz | poph-server | 5 |
| disco | disk | traduz | poph-server, poph | 5 |
| dono | owner | traduz | poph-server | 5 |
| duplicado | duplicate | traduz | poph, poph-server | 5 |
| estatisticas | statistics | traduz | poph-server | 5 |
| estavel | stable | traduz | poph | 5 |
| futuro | future | traduz | poph | 5 |
| instalacao | installation | traduz | poph | 5 |
| intervalo | interval | traduz | poph, poph-server | 5 |
| legenda | caption | traduz | poph-server | 5 |
| listagem | listing | traduz | poph-server | 5 |
| longo | long | traduz | poph, poph-server | 5 |
| medicao | measurement | traduz | scripts, poph-server | 5 |
| migracao | migration | traduz | poph | 5 |
| minuscula | lowercase | traduz | poph-server | 5 |
| mudancas | changes | traduz | poph-server, poph | 5 |
| nas | in_the | traduz | poph-server, poph | 5 |
| novos | new | traduz | console, poph | 5 |
| objeto | object | traduz | poph, poph-server | 5 |
| ordenadas | sorted | traduz | poph-server | 5 |
| parcial | partial | traduz | poph-server, poph | 5 |
| posicionar | position | traduz | poph-server | 5 |
| prazo | deadline | traduz | poph-server | 5 |
| receber | receive | traduz | poph-server | 5 |
| remota | remote | traduz | poph-server | 5 |
| removidos | removed | traduz | console, poph | 5 |
| revogado | revoked | traduz | poph, poph-server | 5 |
| rodadas | rounds | traduz | console, poph-server | 5 |
| simultaneos | simultaneous | traduz | raiz, poph-agent | 5 |
| temporario | temporary | traduz | poph | 5 |
| tudo | everything | traduz | poph-server, scripts | 5 |
| valida | validates | traduz | poph-server | 5 |
| alteradas | changed | traduz | poph-server | 4 |
| ano | year | traduz | poph | 4 |
| antiga | old | traduz | poph, poph-server | 4 |
| aplicar | apply | traduz | poph | 4 |
| assinatura | signature | traduz | poph-server | 4 |
| ativa | active | traduz | poph-server, poph-store | 4 |
| auditorias | audits | traduz | poph-server | 4 |
| booleano | boolean | traduz | poph | 4 |
| cai | falls | traduz | poph, poph-server | 4 |
| chegada | arrival | traduz | poph-server | 4 |
| concluida | finished | traduz | poph-server | 4 |
| convites | invitations | traduz | poph-server | 4 |
| curto | short | traduz | poph, poph-server | 4 |
| derivar | derive | traduz | poph | 4 |
| desconhecido | unknown | traduz | poph-server, poph | 4 |
| desde | since | traduz | poph | 4 |
| direcao | direction | traduz | poph-server | 4 |
| divergente | divergent | traduz | poph-server, poph | 4 |
| encerra | closes | traduz | raiz, console | 4 |
| entra | enters | traduz | poph-server | 4 |
| entrou | entered | traduz | poph-store, poph-server | 4 |
| exemplo | example | traduz | poph-server | 4 |
| existente | existing | traduz | poph | 4 |
| faixa | range | traduz | poph, poph-server | 4 |
| filtros | filters | traduz | poph-server | 4 |
| idempotente | idempotent | traduz | poph, poph-server | 4 |
| identificador | identifier | traduz | poph-server | 4 |
| interno | internal | traduz | poph | 4 |
| limpa | clean | traduz | poph-server | 4 |
| lugar | place | traduz | poph-server, poph | 4 |
| margem | margin | traduz | console | 4 |
| nulo | null | traduz | poph-server, poph | 4 |
| palavra | word | traduz | raiz | 4 |
| perfil | profile | traduz | poph-server | 4 |
| perguntas | questions | traduz | console | 4 |
| processar | process | traduz | poph-server | 4 |
| publicacao | publication | traduz | poph-server | 4 |
| reserva | reservation | traduz | poph-server | 4 |
| resolucao | resolution | traduz | poph | 4 |
| respeita | respects | traduz | poph, poph-server | 4 |
| rodar | run | traduz | poph | 4 |
| saude | health | traduz | poph-server, raiz | 4 |
| sensivel | sensitive | traduz | poph, poph-server | 4 |
| sintaxe | syntax | traduz | poph | 4 |
| soma | sum | traduz | poph-server | 4 |
| sufixo | suffix | traduz | poph, poph-server | 4 |
| tocar | touch | traduz | poph-server | 4 |
| traco | trace | traduz | poph | 4 |
| usa | uses | traduz | poph-server, poph | 4 |
| validade | validity | traduz | poph-server | 4 |
| vistas | views | traduz | poph-server | 4 |
| agiram | acted | traduz | poph-server, poph-store | 3 |
| alterados | changed | traduz | console | 3 |
| ativas | active | traduz | console | 3 |
| caminhos | paths | traduz | poph-server | 3 |
| cancelamento | cancellation | traduz | poph | 3 |
| capturar | capture | traduz | poph-server | 3 |
| codigos | codes | traduz | poph, poph-server | 3 |
| colunas | columns | traduz | poph | 3 |
| consultar | query | traduz | poph | 3 |
| convidados | guests | traduz | poph-server | 3 |
| decididas | decided | traduz | console | 3 |
| efeito | effect | traduz | poph-server, poph | 3 |
| entrar | enter | traduz | raiz, poph-server | 3 |
| escritor | writer | traduz | poph | 3 |
| esquema | schema | traduz | poph | 3 |
| esta | this | traduz | poph-server, poph | 3 |
| estava | was | traduz | poph | 3 |
| extensao | extension | traduz | poph-server, poph | 3 |
| forcado | forced | traduz | poph | 3 |
| frase | sentence | traduz | poph-server, console | 3 |
| gerados | generated | traduz | console | 3 |
| inalterados | unchanged | traduz | console | 3 |
| indisponivel | unavailable | traduz | console, poph-server | 3 |
| informado | informed | traduz | console, poph | 3 |
| legivel | readable | traduz | poph, poph-server | 3 |
| lidos | read | traduz | poph-server | 3 |
| normalizar | normalise | traduz | poph | 3 |
| paginacao | pagination | traduz | poph-server | 3 |
| permitido | allowed | traduz | poph-server | 3 |
| presenca | presence | traduz | poph | 3 |
| problema | problem | traduz | raiz, poph | 3 |
| reais | actual | traduz | poph-server | 3 |
| recuo | backoff | traduz | poph | 3 |
| registrar | register | traduz | poph-server | 3 |
| relevancia | relevance | traduz | console | 3 |
| resultados | results | traduz | poph-server | 3 |
| risco | risk | traduz | poph, raiz | 3 |
| similaridade | similarity | traduz | console | 3 |
| transicao | transition | traduz | poph-server | 3 |
| velho | old | traduz | poph-server, poph | 3 |
| vincular | link | traduz | poph | 3 |
| voltou | returned | traduz | poph, poph-server | 3 |
| abstencao | abstention | traduz | poph | 2 |
| acento | accent | traduz | poph | 2 |
| achado | finding | traduz | poph | 2 |
| aguardando | waiting | traduz | poph-server | 2 |
| ajuste | adjustment | traduz | poph-server | 2 |
| alterada | changed | traduz | poph-server | 2 |
| alterar | modify | traduz | poph | 2 |
| antigo | old | traduz | poph-server | 2 |
| apagada | deleted | traduz | poph-server, scripts | 2 |
| baixa | low | traduz | poph-server | 2 |
| celula | cell | traduz | poph-server | 2 |
| coluna | column | traduz | poph | 2 |
| compartilhado | shared | traduz | poph | 2 |
| compila | compiles | traduz | poph, poph-server | 2 |
| conclusao | completion | traduz | poph-server | 2 |
| condicoes | conditions | traduz | console | 2 |
| consertar | repair | traduz | poph-server | 2 |
| contagem | count | traduz | poph | 2 |
| cortado | cut | traduz | poph-server | 2 |
| decidir | decide | traduz | poph-server | 2 |
| derivado | derived | traduz | poph | 2 |
| encerramento | closure | traduz | poph-server | 2 |
| entao | then | traduz | raiz | 2 |
| escolhas | choices | traduz | poph-server | 2 |
| esgotadas | exhausted | traduz | poph-server | 2 |
| estados | states | traduz | poph-server | 2 |
| faltando | missing | traduz | poph, poph-server | 2 |
| fecha | closes | traduz | poph-server | 2 |
| fechamento | closing | traduz | poph-server | 2 |
| guardado | stored | traduz | poph | 2 |
| guardados | guarded | traduz | poph | 2 |
| hora | hour | traduz | poph-server | 2 |
| idade | age | traduz | poph | 2 |
| identidade | identity | traduz | poph | 2 |
| indexacao | indexing | traduz | poph | 2 |
| leva | takes | traduz | poph, poph-server | 2 |
| mais | more | traduz | poph-server | 2 |
| malformado | malformed | traduz | poph-server | 2 |
| mantem | keeps | traduz | poph, poph-server | 2 |
| marcar | mark | traduz | poph | 2 |
| maximo | maximum | traduz | poph | 2 |
| mediana | median | traduz | poph-server | 2 |
| medir | measure | traduz | poph-server | 2 |
| mes | month | traduz | poph | 2 |
| nenhuma | none | traduz | poph-server | 2 |
| parametros | parameters | traduz | poph-server | 2 |
| pareamento | pairing | traduz | poph-server | 2 |
| pedacos | pieces | traduz | poph-server | 2 |
| pede | asks | traduz | poph | 2 |
| pergunta | question | traduz | poph-server | 2 |
| precisa | needs | traduz | poph, poph-server | 2 |
| proprias | own | traduz | poph | 2 |
| recarga | reload | traduz | poph-server | 2 |
| reenvios | resends | traduz | poph-server | 2 |
| repeticao | repetition | traduz | poph-server | 2 |
| revogar | revoke | traduz | poph | 2 |
| senao | else | traduz | raiz | 2 |
| sim | yes | traduz | poph | 2 |
| transferir | transfer | traduz | poph-server | 2 |
| truncar | truncate | traduz | poph | 2 |
| unicos | single | traduz | poph-server | 2 |
| unidade | unit | traduz | console | 2 |
| vazia | empty | traduz | poph | 2 |
| abaixo | below | traduz | poph | 1 |
| abandonada | abandoned | traduz | poph-server | 1 |
| alteracao | change | traduz | poph | 1 |
| aplica | applies | traduz | poph-server | 1 |
| atendimentos | services | traduz | poph | 1 |
| atualizada | updated | traduz | poph-store | 1 |
| comentarios | comments | traduz | poph | 1 |
| conexoes | connections | traduz | poph-server | 1 |
| confere | checks | traduz | poph | 1 |
| confirmar | confirm | traduz | poph-server | 1 |
| consultas | queries | traduz | poph-server | 1 |
| contato | contact | traduz | raiz | 1 |
| declara | declares | traduz | poph-server | 1 |
| declarada | declared | traduz | poph-server | 1 |
| declarados | declared | traduz | poph-server | 1 |
| dia | day | traduz | poph | 1 |
| edicao | edit | traduz | poph-server | 1 |
| enganosa | misleading | traduz | poph | 1 |
| equipe | team | traduz | poph-server | 1 |
| espelhada | mirrored | traduz | poph-server | 1 |
| expirada | expired | traduz | poph-server | 1 |
| expor | expose | traduz | poph-server | 1 |
| frente | front | traduz | poph-server | 1 |
| gravacao | recording | traduz | poph-server | 1 |
| habilitada | enabled | traduz | poph | 1 |
| habilitado | enabled | traduz | poph | 1 |
| ilegivel | unreadable | traduz | poph | 1 |
| inicia | starts | traduz | poph-server | 1 |
| instante | instant | traduz | poph | 1 |
| latencia | latency | traduz | poph | 1 |
| limpar | clean | traduz | scripts | 1 |
| migracoes | migrations | traduz | poph | 1 |
| minutos | minutes | traduz | scripts | 1 |
| muda | changes | traduz | poph | 1 |
| notificacao | notification | traduz | poph-server | 1 |
| organizacoes | organisations | traduz | poph-server | 1 |
| paginas | pages | traduz | poph-server | 1 |
| parada | stop | traduz | poph-server | 1 |
| pausa | pause | traduz | poph-server | 1 |
| ponto | point | traduz | poph-server | 1 |
| preparado | prepared | traduz | poph-server | 1 |
| recente | recent | traduz | poph-server | 1 |
| recusados | refused | traduz | poph | 1 |
| rede | network | traduz | poph-server | 1 |
| reiniciar | restart | traduz | poph-server | 1 |
| removido | removed | traduz | console | 1 |
| rotina | routine | traduz | poph-server | 1 |
| seguro | safe | traduz | poph-server | 1 |
| selecionado | selected | traduz | console | 1 |
| sistema | system | traduz | scripts | 1 |
| tres | three | traduz | poph-server | 1 |
| usuarios | users | traduz | poph-server | 1 |
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
