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
| erro | error | traduz | poph-server, poph-store | 6724 |
| cenario | scenario | traduz | poph-agent, poph-server | 6483 |
| texto | text | traduz | poph-store, poph-agent | 5910 |
| agente | agent | traduz | poph-store, poph-server | 4801 |
| estado | state | traduz | poph-server, poph-agent | 4237 |
| conversa | conversation | traduz | poph-server, poph-agent | 4106 |
| nome | name | traduz | poph-server, poph-store | 3407 |
| de | of | traduz | console, poph-server | 3180 |
| novo | new | traduz | poph-server, poph-agent | 3004 |
| da | of | traduz | poph-server, poph-store | 2521 |
| fluxo | flow | traduz | poph-agent, poph-store | 2414 |
| corpo | body | traduz | poph-server, console | 2407 |
| com | with | traduz | poph-server, poph-agent | 2287 |
| resposta | response | traduz | poph-server, poph-jev | 2081 |
| valor | value | traduz | poph-server, console | 2042 |
| evento | event | traduz | console, poph-server | 1928 |
| mensagem | message | traduz | poph-agent, poph-server | 1927 |
| pasta | folder | traduz | poph, poph-server | 1877 |
| linha | line | traduz | poph-store, poph-server | 1873 |
| saida | output | traduz | poph, poph-server | 1843 |
| sala | room | traduz | poph-server, poph-matrix | 1818 |
| turno | turn | traduz | poph-agent, console | 1599 |
| motivo | reason | traduz | poph-server, poph-matrix | 1568 |
| tipo | kind | traduz | poph-fluxos, poph-server | 1539 |
| cliente | client | traduz | poph-server, poph-matrix | 1525 |
| caminho | path | traduz | poph-server, poph | 1472 |
| criar | create | traduz | poph-server, poph-store | 1449 |
| arquivo | file | traduz | poph, poph-server | 1438 |
| campo | field | traduz | poph-fluxos, console | 1434 |
| acao | action | traduz | poph-fluxos, poph | 1427 |
| ficha | case | traduz | poph-store, poph-server | 1426 |
| falha | failure | traduz | console, poph-server | 1379 |
| motor | engine | traduz | poph-agent, poph-server | 1342 |
| sem | without | traduz | poph-server, poph-agent | 1337 |
| em | in | traduz | poph-store, poph-server | 1324 |
| resultado | result | traduz | poph-store, poph-fluxos | 1319 |
| ao | to | traduz | console, poph-fluxos | 1295 |
| nova | new | traduz | poph-agent, poph-server | 1267 |
| dados | data | traduz | poph-matrix, poph-server | 1238 |
| metricas | metrics | traduz | poph-server, poph-agent | 1234 |
| servidor | server | traduz | console, poph-server | 1204 |
| chave | key | traduz | poph-server, poph-fluxos | 1181 |
| versao | version | traduz | poph-server, poph-store | 1138 |
| para | for | traduz | console, poph-fluxos | 1104 |
| departamento | department | traduz | poph-server, poph | 1102 |
| nao | not | traduz | poph-server, poph-store | 1077 |
| entrada | entry | traduz | poph-agent, poph-store | 1062 |
| codigo | code | traduz | poph-fluxos, poph-server | 1026 |
| execucao | execution | traduz | poph-store, poph-server | 1005 |
| registro | record | traduz | poph-agent, poph-server | 997 |
| usuario | user | traduz | console, poph-matrix | 974 |
| ator | actor | traduz | poph-fluxos, poph-store | 944 |
| opcoes | options | traduz | poph-agent, poph-server | 935 |
| etapa | step | traduz | poph-fluxos, poph-store | 910 |
| origem | origin | traduz | poph-store, poph-server | 908 |
| destino | target | traduz | poph-server, poph-fluxos | 902 |
| integracao | integration | traduz | poph-server, console | 891 |
| atual | current | traduz | console, poph-server | 850 |
| produto | product | traduz | poph-agent, console | 848 |
| indice | index | traduz | poph-rag, console | 843 |
| alvo | target | traduz | poph-server, poph-fluxos | 828 |
| midia | media | traduz | poph-server, poph-matrix | 818 |
| grupo | group | traduz | console, poph-server | 815 |
| resumo | summary | traduz | poph-agent, poph-server | 813 |
| espaco | space | traduz | poph-matrix, poph-server | 804 |
| decisao | decision | traduz | poph-jev, poph-fluxos | 793 |
| pedido | request | traduz | poph-agent, poph-server | 776 |
| falso | false | traduz | poph-server, console | 770 |
| lista | list | traduz | console, poph-fluxos | 764 |
| itens | items | traduz | console, poph-server | 752 |
| fluxos | flows | traduz | poph-server, poph-agent | 751 |
| agentes | agents | traduz | console, poph-server | 744 |
| mensagens | messages | traduz | poph-agent, poph-server | 728 |
| chamada | call | traduz | poph-server, poph-matrix | 719 |
| relatorio | report | traduz | poph-server, poph | 679 |
| conversas | conversations | traduz | poph-server, poph-store | 674 |
| por | by | traduz | poph-store, poph-agent | 672 |
| rotulo | label | traduz | console, poph-server | 667 |
| modelo | model | traduz | poph-agent, console | 666 |
| canal | channel | traduz | poph-server, poph-store | 652 |
| pedir | ask | traduz | poph-server, console | 645 |
| revisao | revision | traduz | poph-store, poph-server | 636 |
| responder | reply | traduz | console, poph-server | 633 |
| aviso | notice | traduz | poph-server, console | 619 |
| pagina | page | traduz | poph-server, poph-store | 614 |
| fim | end | traduz | poph-fluxos, poph-agent | 613 |
| regra | rule | traduz | poph-fluxos, poph-types | 602 |
| agora | now | traduz | poph-store, poph-server | 601 |
| inicio | beginning | traduz | poph-server, poph-agent | 592 |
| fonte | source | traduz | poph-fluxos, poph-server | 591 |
| mudanca | change | traduz | poph-fluxos, poph-store | 591 |
| ferramenta | tool | traduz | poph-agent, console | 567 |
| obter | get | traduz | poph-server, poph-store | 561 |
| categoria | category | traduz | poph, poph-server | 560 |
| limite | limit | traduz | poph-store, poph-server | 558 |
| montar | build | traduz | console, poph-agent | 553 |
| listar | list | traduz | poph-server, poph-store | 552 |
| segredo | secret | traduz | poph-server, poph | 545 |
| chamadas | calls | traduz | console, poph-agent | 539 |
| gatilho | trigger | traduz | poph-agent, poph-fluxos | 539 |
| porta | port | traduz | poph-server, poph-matrix | 522 |
| conteudo | content | traduz | poph-server, poph-matrix | 512 |
| limites | limits | traduz | poph-fluxos, poph-agent | 506 |
| eventos | events | traduz | poph-server, poph-store | 505 |
| situacao | situation | traduz | console, poph-store | 499 |
| efeito | effect | traduz | poph-fluxos, poph-server | 498 |
| padrao | default | traduz | poph-agent, console | 487 |
| abrir | open | traduz | poph, poph-server | 481 |
| contexto | context | traduz | poph-fluxos, poph-agent | 476 |
| provedor | provider | traduz | poph-agent, poph-server | 475 |
| guarda | guard | traduz | poph-matrix, console | 471 |
| envio | send | traduz | poph-server, poph-agent | 461 |
| escolha | choice | traduz | poph-jev, poph-agent | 453 |
| instancia | instance | traduz | poph-server, console | 452 |
| consulta | query | traduz | poph-store, poph-server | 448 |
| produtos | products | traduz | poph-agent, console | 444 |
| ambiente | environment | traduz | poph, poph-files | 437 |
| banco | database | traduz | poph-store, poph-retention | 422 |
| papel | role | traduz | poph-agent, poph-store | 418 |
| erros | errors | traduz | console, poph-fluxos | 397 |
| efeitos | effects | traduz | poph-server, poph-fluxos | 389 |
| linhas | lines | traduz | poph-store, console | 389 |
| campos | fields | traduz | poph-fluxos, console | 385 |
| caso | case | traduz | poph-server, poph-fluxos | 385 |
| depois | after | traduz | poph-store, poph-server | 385 |
| descricao | description | traduz | console, poph-store | 384 |
| atores | actors | traduz | poph-agent, poph-fluxos | 382 |
| mapa | map | traduz | poph-server, poph-agent | 380 |
| teto | cap | traduz | poph-server, poph-fluxos | 380 |
| configuracao | configuration | traduz | poph, console | 379 |
| outro | other | traduz | poph-server, poph-store | 376 |
| ler | read | traduz | console, poph | 371 |
| antes | before | traduz | poph-server, console | 366 |
| externo | external | traduz | poph-store, poph-agent | 364 |
| desligar | disable | traduz | poph-server, poph | 362 |
| tabela | table | traduz | poph-store, console | 360 |
| na | towards | traduz | poph-server, console | 353 |
| quente | hot | traduz | poph-agent, poph-server | 353 |
| anexo | attachment | traduz | poph-server, poph-agent | 352 |
| metodo | method | traduz | poph-server, console | 351 |
| identidade | identity | traduz | poph-server, poph-rag | 349 |
| tamanho | size | traduz | poph-server, poph-agent | 349 |
| conhecimento | knowledge | traduz | poph-server, console | 345 |
| bloco | block | traduz | poph-agent, console | 343 |
| lote | batch | traduz | poph-rag, poph-jev | 342 |
| pendentes | pending | traduz | poph-agent, poph-server | 342 |
| rota | route | traduz | poph-server, console | 342 |
| que | that | traduz | console, poph-fluxos | 339 |
| vazio | empty | traduz | poph-server, poph-fluxos | 339 |
| numero | number | traduz | console, poph-fluxos | 336 |
| partes | parts | traduz | console, poph-server | 336 |
| falhas | failures | traduz | poph-server, poph-agent | 334 |
| gravada | stored | traduz | poph-store, poph-server | 334 |
| primeira | first | traduz | poph-server, poph-store | 332 |
| excluir | delete | traduz | console, poph-server | 331 |
| enviar | send | traduz | poph-server, poph-agent | 330 |
| registrar | register | traduz | poph-server, poph-store | 329 |
| detalhe | detail | traduz | poph-server, console | 326 |
| salas | rooms | traduz | poph-server, poph-matrix | 326 |
| arquivos | files | traduz | poph-server, console | 325 |
| executar | run | traduz | poph-fluxos, poph-agent | 324 |
| sessao | session | traduz | console, poph | 324 |
| roteiro | script | traduz | poph-agent, poph-server | 323 |
| nomes | names | traduz | poph-fluxos, poph-agent | 317 |
| validar | validate | traduz | poph-types, poph-server | 317 |
| coluna | column | traduz | poph-store, poph-fluxos | 315 |
| segredos | secrets | traduz | poph, console | 311 |
| pedidos | requests | traduz | poph-agent, poph-core | 309 |
| espacos | spaces | traduz | poph-server, poph-matrix | 307 |
| interno | internal | traduz | poph-agent, poph-server | 307 |
| iniciar | start | traduz | poph-fluxos, poph-server | 306 |
| instrucoes | instructions | traduz | poph, poph-agent | 306 |
| avaliador | evaluator | traduz | poph-fluxos, poph-agent | 304 |
| como | as | traduz | poph-store, poph-server | 302 |
| direcao | direction | traduz | poph-agent, poph-server | 301 |
| esperar | wait | traduz | poph-server, console | 301 |
| modulo | module | traduz | poph-server, console | 300 |
| anexos | attachments | traduz | poph-server, poph-agent | 299 |
| relato | report | traduz | poph-server, poph-agent | 294 |
| turnos | turns | traduz | poph-agent, poph-server | 293 |
| fila | queue | traduz | poph-rag, poph-server | 291 |
| posicao | position | traduz | console, poph-matrix | 290 |
| gravado | stored | traduz | poph-server, poph-agent | 285 |
| contador | counter | traduz | poph-agent, poph-server | 284 |
| carregar | load | traduz | console, poph | 282 |
| outra | other | traduz | poph-store, poph-server | 282 |
| teste | test | traduz | console, poph | 281 |
| invalido | invalid | traduz | poph-server, poph-fluxos | 278 |
| fechada | closed | traduz | poph-store, poph-server | 277 |
| comando | command | traduz | poph, poph-plugins | 275 |
| ferramentas | tools | traduz | poph-agent, console | 273 |
| dialogo | dialogue | traduz | console | 272 |
| aberto | open | traduz | console, poph-server | 271 |
| recusa | refusal | traduz | console, poph-server | 271 |
| retrato | snapshot | traduz | poph-store, poph | 269 |
| gravar | write | traduz | poph-store, poph-server | 268 |
| titulo | title | traduz | console, poph-rag | 267 |
| conexao | connection | traduz | poph-store, poph-server | 264 |
| manifesto | manifest | traduz | console, poph-server | 264 |
| espera | wait | traduz | poph-agent, poph-server | 263 |
| aberta | open | traduz | poph-store, poph-server | 260 |
| pausa | pause | traduz | poph-agent, poph-server | 260 |
| dominio | domain | traduz | poph-store, poph-server | 254 |
| cabecalhos | headers | traduz | poph-server, console | 252 |
| fechar | close | traduz | console, poph-store | 252 |
| memoria | memory | traduz | poph-agent, poph-server | 252 |
| busca | search | traduz | poph-agent, console | 250 |
| definir | define | traduz | poph-server, poph-store | 244 |
| leitura | read | traduz | poph-store, console | 244 |
| atualizar | update | traduz | poph-server, console | 241 |
| recebidos | received | traduz | poph-agent, poph-evolution | 241 |
| escrita | writing | traduz | poph-store, poph-server | 239 |
| transicao | transition | traduz | poph-store, poph-server | 238 |
| conflito | conflict | traduz | poph-store, console | 234 |
| contar | count | traduz | poph-store, poph-server | 233 |
| primeiro | first | traduz | poph-server, poph-agent | 231 |
| encontrado | found | traduz | poph-store, poph-agent | 230 |
| fantasma | ghost | traduz | poph-matrix, poph-server | 230 |
| uso | use | traduz | poph-agent, poph-core | 228 |
| caixa | box | traduz | poph-agent, poph-store | 227 |
| segunda | second | traduz | poph-server, poph-store | 227 |
| textos | texts | traduz | poph-rag, poph-server | 225 |
| valores | values | traduz | console, poph-store | 225 |
| esperado | expected | traduz | poph, poph-rag | 223 |
| controle | control | traduz | console, poph-plugins | 222 |
| ativo | active | traduz | console, poph-agent | 220 |
| ciclo | cycle | traduz | poph-server, poph-fluxos | 216 |
| modo | mode | traduz | poph-server, console | 216 |
| ultimo | last | traduz | poph-server, poph-agent | 216 |
| documento | document | traduz | poph-store, poph-server | 215 |
| relogio | clock | traduz | poph-server, poph-store | 215 |
| fichas | cases | traduz | poph-store, poph-server | 213 |
| criado | created | traduz | poph-server, poph-store | 212 |
| validacao | validation | traduz | poph-store, poph-server | 211 |
| leitor | reader | traduz | poph-fluxos, poph-core | 210 |
| confianca | confidence | traduz | poph-jev, poph-agent | 209 |
| departamentos | departments | traduz | console, poph-server | 208 |
| corte | cutoff | traduz | poph-agent, poph-matrix | 206 |
| trechos | passages | traduz | poph-agent, poph-rag | 206 |
| catalogo | catalog | traduz | poph-agent, poph | 205 |
| esperada | expected | traduz | poph-store, poph-server | 205 |
| fora | outside | traduz | poph-server, poph-evolution | 201 |
| recebido | received | traduz | poph-store, poph-server | 201 |
| respostas | responses | traduz | poph-agent, poph-server | 201 |
| pergunta | question | traduz | poph-jev, poph-agent | 198 |
| entidade | entity | traduz | poph-store, poph-server | 197 |
| desfecho | outcome | traduz | poph-server, poph-agent | 196 |
| limitador | limiter | traduz | poph-agent, poph-core | 195 |
| auditoria | audit | traduz | poph-server, poph-store | 194 |
| falar | speak | traduz | poph-agent, poph-server | 194 |
| tentativas | attempts | traduz | poph-server, poph-agent | 194 |
| ativa | active | traduz | console, poph-agent | 193 |
| pronta | ready | traduz | poph-store, poph-matrix | 193 |
| acoes | actions | traduz | poph-fluxos, console | 190 |
| trecho | passage | traduz | poph-agent, console | 190 |
| recusado | refused | traduz | poph-server, poph-store | 188 |
| apos | after | traduz | poph-store, poph-server | 187 |
| preco | price | traduz | poph-agent, console | 186 |
| cru | raw | traduz | poph-server, poph-agent | 185 |
| preparar | prepare | traduz | console, poph-server | 184 |
| contadores | counters | traduz | poph-plugins, poph-server | 183 |
| confere | checks | traduz | poph-fluxos, poph-types | 182 |
| etapas | steps | traduz | poph-fluxos, poph-store | 182 |
| futuro | future | traduz | poph-agent, poph-rag | 182 |
| subir | raise | traduz | poph-server, poph-matrix | 182 |
| orgaos | orgs | traduz | poph-server, poph-store | 181 |
| remetente | sender | traduz | poph-matrix, poph-server | 181 |
| salvar | save | traduz | console, poph-agent | 181 |
| saude | health | traduz | console, poph-plugins | 181 |
| atendente | attendant | traduz | poph-server, poph-fluxos | 180 |
| prefixo | prefix | traduz | poph-server, poph-matrix | 180 |
| uma | a | traduz | poph-server, poph-store | 179 |
| registros | records | traduz | poph-server, poph-matrix | 178 |
| categorias | categories | traduz | poph, poph-store | 177 |
| instalar | install | traduz | console, poph-rag | 175 |
| falsa | false | traduz | poph-server, poph-agent | 174 |
| ou | or | traduz | poph-fluxos, poph-store | 174 |
| execucoes | executions | traduz | poph-store, poph-server | 173 |
| formatar | format | traduz | console, poph-agent | 173 |
| transporte | transport | traduz | poph-agent, poph-jev | 173 |
| concluido | finished | traduz | poph-agent, poph-server | 172 |
| ordem | order | traduz | poph-server, poph-store | 172 |
| carga | load | traduz | console, poph-rag | 169 |
| historico | history | traduz | console, poph-server | 169 |
| argumentos | arguments | traduz | poph-agent, console | 168 |
| janela | window | traduz | poph-agent, poph-server | 168 |
| carregando | loading | traduz | console | 167 |
| bruto | raw | traduz | poph-fluxos, console | 166 |
| limpo | clean | traduz | console, poph-store | 166 |
| rascunho | draft | traduz | console, poph-server | 166 |
| transacao | transaction | traduz | poph-server, poph-store | 166 |
| receptor | receiver | traduz | poph-server, poph-plugins | 165 |
| impacto | impact | traduz | poph-fluxos | 164 |
| escopo | scope | traduz | console, poph-fluxos | 163 |
| ja | already | traduz | poph-server, poph-matrix | 163 |
| mudou | changed | traduz | poph-fluxos, console | 163 |
| detalhes | details | traduz | console, poph-server | 162 |
| endereco | address | traduz | poph-server, poph-matrix | 162 |
| whatsapp | whatsapp | traduz | poph-server, console | 162 |
| lido | read | traduz | poph, console | 161 |
| ligar | link | traduz | poph-server, poph | 161 |
| proprio | own | traduz | poph-agent, poph-server | 161 |
| quando | when | traduz | poph-fluxos, poph-jev | 161 |
| secao | section | traduz | console, poph-types | 161 |
| fantasmas | ghosts | traduz | poph-matrix, poph-server | 160 |
| tempos | times | traduz | poph-agent, console | 159 |
| atendimento | service | traduz | poph-fluxos, poph-store | 158 |
| casos | cases | traduz | poph-jev, poph-fluxos | 158 |
| revisoes | revisions | traduz | poph-store, poph-server | 158 |
| exigir | require | traduz | poph-server, poph-store | 155 |
| ligado | linked | traduz | poph-server, poph | 155 |
| duracao | duration | traduz | poph-agent, poph-server | 154 |
| entrega | delivery | traduz | poph-server, poph-agent | 154 |
| tabelas | tables | traduz | poph-store, poph | 154 |
| usuarios | users | traduz | poph-matrix, poph-server | 154 |
| objeto | object | traduz | console, poph-store | 153 |
| vai | goes | traduz | poph-fluxos, raiz | 152 |
| biblioteca | library | traduz | poph-server, poph-store | 150 |
| diretorio | directory | traduz | poph-server, poph-rag | 149 |
| espelho | mirror | traduz | poph-server, console | 149 |
| grande | large | traduz | poph-server, poph-matrix | 149 |
| importacao | import | traduz | poph-store, console | 149 |
| avisos | notices | traduz | poph-server, poph-fluxos | 148 |
| humana | human | traduz | poph-server, poph-store | 147 |
| atraso | delay | traduz | poph-server, poph-agent | 146 |
| decisoes | decisions | traduz | poph-fluxos, poph-jev | 146 |
| enviado | sent | traduz | poph-server, poph-store | 146 |
| opcao | option | traduz | console, poph-fluxos | 145 |
| das | of | traduz | poph-server, poph-agent | 143 |
| prazo | deadline | traduz | poph-fluxos, poph-server | 142 |
| imagem | image | traduz | poph-files, poph-matrix | 141 |
| tarefa | task | traduz | poph-agent, poph-server | 141 |
| cabecalho | header | traduz | console, poph-server | 140 |
| vivo | live | traduz | console, poph | 140 |
| buscar | search | traduz | poph-agent, poph-rag | 139 |
| aparelho | device | traduz | poph-server, poph-store | 138 |
| enviando | sending | traduz | console, poph-agent | 138 |
| parte | part | traduz | poph-agent, poph-core | 138 |
| sinal | signal | traduz | poph-server, console | 138 |
| escolhas | choices | traduz | poph-agent, poph-jev | 137 |
| instancias | instances | traduz | console, poph-server | 137 |
| retencao | retention | traduz | poph-retention, console | 137 |
| contagem | count | traduz | console, poph-rag | 136 |
| operacao | operation | traduz | poph-matrix, console | 136 |
| orientacao | guidance | traduz | poph-jev, poph-agent | 136 |
| autor | author | traduz | poph-store, poph-agent | 135 |
| pendente | pending | traduz | poph-server, console | 135 |
| atualizado | updated | traduz | poph-server, poph-store | 134 |
| envios | sends | traduz | poph-server, poph-agent | 133 |
| filtro | filter | traduz | poph-store, poph-server | 133 |
| esquema | schema | traduz | console, poph-server | 132 |
| invalida | invalid | traduz | poph-fluxos, poph-server | 132 |
| limiares | thresholds | traduz | poph-jev | 131 |
| segundo | second | traduz | poph-agent, poph-server | 131 |
| diagnosticos | diagnostics | traduz | poph-fluxos, poph-server | 130 |
| resto | rest | traduz | poph-server, console | 130 |
| desligado | disabled | traduz | poph-server, console | 129 |
| exige | requires | traduz | poph-fluxos, console | 129 |
| incrementar | increment | traduz | poph-agent, poph-server | 129 |
| avaliar | evaluate | traduz | poph-fluxos, poph-jev | 128 |
| compilado | compiled | traduz | poph-fluxos, poph-server | 128 |
| receber | receive | traduz | poph-files, poph-store | 128 |
| parametros | parameters | traduz | poph-agent, poph-store | 126 |
| diagnostico | diagnostic | traduz | poph-fluxos, poph-server | 124 |
| inexistente | missing | traduz | poph-fluxos, poph-server | 124 |
| ausente | missing | traduz | poph-store, poph-fluxos | 123 |
| candidato | candidate | traduz | poph-jev, poph-agent | 123 |
| encerra | closes | traduz | poph-fluxos, poph-store | 123 |
| novos | new | traduz | console, poph-fluxos | 123 |
| observador | observer | traduz | poph-agent, poph-server | 123 |
| repetida | repeated | traduz | poph-server, poph-store | 123 |
| veneno | poison | traduz | poph-server, poph-agent | 122 |
| ajustes | adjustments | traduz | poph-agent, poph-server | 121 |
| variavel | variable | traduz | poph-fluxos, poph | 121 |
| centavos | cents | traduz | console, poph-agent | 119 |
| classificacao | classification | traduz | poph-matrix, poph-server | 119 |
| dias | days | traduz | poph-retention, console | 119 |
| captura | capture | traduz | poph-server, poph | 118 |
| gaveta | drawer | traduz | console | 117 |
| falhou | failed | traduz | poph-server, poph-agent | 114 |
| formato | format | traduz | poph, poph-matrix | 114 |
| mostrar | show | traduz | poph-server, poph | 114 |
| vetor | vector | traduz | poph-rag, console | 114 |
| garantir | ensure | traduz | poph-matrix, poph-server | 113 |
| segundos | seconds | traduz | poph-server, poph-rag | 112 |
| membros | members | traduz | poph-server, poph-matrix | 111 |
| par | pair | traduz | poph-server, poph | 111 |
| quantidade | quantity | traduz | poph-agent, poph-store | 111 |
| imprimir | print | traduz | poph, poph-fluxos | 110 |
| tem | has | traduz | console, poph | 110 |
| existe | exists | traduz | poph-store, poph-server | 109 |
| ajuste | adjustment | traduz | poph-jev, poph-agent | 108 |
| ignorar | ignore | traduz | poph-matrix, poph-server | 107 |
| termo | term | traduz | scripts, console | 107 |
| carrinho | cart | traduz | poph-store, poph-agent | 105 |
| nenhum | none | traduz | poph-fluxos, poph-jev | 105 |
| tempo | time | traduz | poph-agent, poph-jev | 105 |
| transferida | transferred | traduz | poph-server, poph-store | 105 |
| baixar | download | traduz | poph-server, poph-matrix | 104 |
| pelo | by | traduz | poph-server, poph-store | 103 |
| comum | common | traduz | poph-plugins, poph-agent | 102 |
| proxima | next | traduz | poph-plugins, poph-server | 102 |
| raiz | root | traduz | poph-server, console | 102 |
| midias | media | traduz | poph-server, poph-evolution | 101 |
| escritas | writings | traduz | poph-server, poph-matrix | 100 |
| interpretar | interpret | traduz | poph-jev, poph-evolution | 100 |
| descrever | describe | traduz | poph, poph-agent | 99 |
| falhar | fail | traduz | poph-server, poph-matrix | 99 |
| integracoes | integrations | traduz | console, poph-server | 99 |
| nivel | level | traduz | poph, poph-matrix | 99 |
| publicar | publish | traduz | poph-server, poph-store | 99 |
| bruta | raw | traduz | poph-fluxos, console | 98 |
| antigo | old | traduz | poph-matrix, poph-store | 97 |
| criada | created | traduz | poph-store, poph-server | 97 |
| enfileirar | enqueue | traduz | poph-matrix, poph-agent | 97 |
| exemplos | examples | traduz | poph-fluxos, poph-jev | 97 |
| processo | process | traduz | poph-plugins, poph-server | 97 |
| roteiros | scripts | traduz | poph-agent, poph-server | 97 |
| achado | finding | traduz | poph-rag, poph-agent | 96 |
| cursores | cursors | traduz | poph-server, poph-store | 96 |
| existente | existing | traduz | poph-matrix, poph-store | 96 |
| idempotencia | idempotency | traduz | poph-store, poph-server | 96 |
| testes | tests | traduz | poph-server, poph | 96 |
| confirmar | confirm | traduz | console, poph-server | 95 |
| grupos | groups | traduz | console, poph | 95 |
| liberar | release | traduz | poph-store, console | 95 |
| parcial | partial | traduz | console, poph-store | 95 |
| ate | until | traduz | poph-server, poph-agent | 93 |
| obrigatorio | required | traduz | poph-fluxos, poph-matrix | 93 |
| proximo | next | traduz | poph-server, console | 93 |
| alteracoes | changes | traduz | console | 92 |
| foco | focus | traduz | console | 92 |
| passo | step | traduz | poph-server, console | 92 |
| sincronizar | synchronise | traduz | poph-agent, poph-server | 92 |
| enviados | sent | traduz | poph-server, poph-agent | 91 |
| familia | family | traduz | poph-files, poph-matrix | 90 |
| inicial | initial | traduz | poph-store, console | 90 |
| instrucao | instruction | traduz | poph-fluxos, raiz | 90 |
| entradas | entries | traduz | poph-agent, poph-fluxos | 89 |
| geral | general | traduz | console, poph-agent | 89 |
| nulo | null | traduz | poph-agent, poph-store | 89 |
| rodada | round | traduz | poph-agent, console | 89 |
| expiracao | expiry | traduz | poph-server, poph-matrix | 88 |
| importar | import | traduz | poph-store, console | 88 |
| tentar | try | traduz | console, poph-matrix | 88 |
| conferir | check | traduz | poph-server, poph-store | 87 |
| montagem | assembly | traduz | poph-agent, poph | 87 |
| previa | preview | traduz | console, poph-server | 87 |
| tentativa | attempt | traduz | poph-agent, poph-server | 87 |
| buscador | searcher | traduz | poph-agent, poph | 86 |
| cancelamento | cancellation | traduz | poph-agent, poph-core | 86 |
| entrar | enter | traduz | poph-fluxos, poph-matrix | 86 |
| maior | bigger | traduz | poph-fluxos, poph-rag | 86 |
| saidas | outputs | traduz | poph-agent, poph-server | 86 |
| onde | where | traduz | poph-fluxos, scripts | 85 |
| parar | stop | traduz | poph-server, poph-agent | 85 |
| perguntas | questions | traduz | poph-jev, poph-agent | 85 |
| ultima | last | traduz | poph-server, poph-agent | 85 |
| candidatos | candidates | traduz | poph-jev, poph-agent | 84 |
| compacta | compact | traduz | console | 84 |
| mundo | world | traduz | poph-server | 84 |
| cortado | cut | traduz | poph-fluxos, poph-matrix | 83 |
| problemas | problems | traduz | poph-types, poph-server | 83 |
| colunas | columns | traduz | poph-store, console | 82 |
| estrutura | structure | traduz | console | 82 |
| muda | changes | traduz | poph-fluxos, poph-agent | 82 |
| todas | all | traduz | console, poph-server | 82 |
| mesmo | same | traduz | console, poph-store | 81 |
| quem | who | traduz | poph-server, poph-agent | 81 |
| funcao | function | traduz | poph-fluxos, raiz | 80 |
| pedaco | piece | traduz | poph-matrix, poph-server | 80 |
| pode | can | traduz | console, poph | 80 |
| tipos | kinds | traduz | poph-fluxos, poph-jev | 80 |
| transitoria | transient | traduz | poph-server, poph-matrix | 80 |
| anterior | previous | traduz | poph-server, console | 79 |
| globais | globals | traduz | poph-server, poph-jev | 79 |
| encerrar | close | traduz | poph-plugins, poph-agent | 78 |
| igual | equal | traduz | poph-fluxos, poph-server | 78 |
| metadados | metadata | traduz | poph-files, poph-server | 78 |
| migracao | migration | traduz | poph-store, poph | 78 |
| atuais | current | traduz | console, poph-rag | 77 |
| nada | nothing | traduz | poph-agent, poph-server | 77 |
| preparo | preparation | traduz | poph-agent, poph | 77 |
| propriedade | property | traduz | console | 77 |
| publicacao | publication | traduz | poph-store, poph-server | 77 |
| canais | channels | traduz | poph-server, poph | 76 |
| coletor | collector | traduz | poph-plugins, poph-agent | 76 |
| processador | processor | traduz | poph-agent, poph-server | 76 |
| servir | serve | traduz | poph-server, poph | 76 |
| autorizacao | authorization | traduz | poph-matrix, poph-server | 75 |
| ignorado | ignored | traduz | poph-evolution, poph-agent | 75 |
| mudar | change | traduz | console, poph-store | 75 |
| argumento | argument | traduz | poph-fluxos, poph-agent | 74 |
| classificar | classify | traduz | poph-matrix, poph-files | 73 |
| escrever | write | traduz | poph, poph-server | 73 |
| laco | loop | traduz | poph-server, console | 73 |
| sistema | system | traduz | poph-store, poph-agent | 73 |
| antiga | old | traduz | poph-fluxos, poph-store | 72 |
| chaves | keys | traduz | poph-store, poph-fluxos | 72 |
| entregar | deliver | traduz | poph-server, poph-agent | 72 |
| ouvinte | listener | traduz | poph-matrix, poph-server | 72 |
| vira | becomes | traduz | poph-server, poph-matrix | 72 |
| conjunto | set | traduz | poph-jev, poph | 71 |
| dos | of | traduz | console, poph-fluxos | 71 |
| intervalo | interval | traduz | poph-server, poph-types | 71 |
| marcador | marker | traduz | poph-server, poph-agent | 71 |
| canonico | canonical | traduz | poph-fluxos, poph-jev | 70 |
| cifra | cipher | traduz | poph-server, poph | 70 |
| unica | single | traduz | poph-fluxos, poph-server | 70 |
| avancar | advance | traduz | poph-server, poph-agent | 69 |
| chamar | call | traduz | poph-agent, poph-server | 69 |
| consultar | query | traduz | console, poph-agent | 69 |
| estruturado | structured | traduz | poph-agent, poph-server | 69 |
| filho | child | traduz | poph-plugins, poph-matrix | 69 |
| recusada | refused | traduz | poph-agent, poph | 69 |
| unidade | unit | traduz | poph-agent, poph-rag | 69 |
| espelhado | mirrored | traduz | poph-server, poph-store | 68 |
| organizacao | organisation | traduz | console, poph-fluxos | 68 |
| volta | return | traduz | poph-server, poph-types | 68 |
| botao | button | traduz | console | 67 |
| caractere | character | traduz | poph-store, poph-matrix | 67 |
| compilar | compile | traduz | poph-server, poph-fluxos | 67 |
| declarado | declared | traduz | poph-server, poph-evolution | 67 |
| exemplo | example | traduz | poph-types, poph-fluxos | 67 |
| vistos | seen | traduz | poph-fluxos, poph-server | 67 |
| atender | attend | traduz | poph-server, poph-matrix | 66 |
| caracteres | characters | traduz | poph-files, console | 66 |
| diferenca | difference | traduz | poph-fluxos, poph-server | 66 |
| encerrada | closed | traduz | poph-plugins, poph-server | 66 |
| forcar | force | traduz | console, poph | 65 |
| provisionador | provisioner | traduz | poph-matrix, poph-server | 65 |
| resolucoes | resolutions | traduz | poph-fluxos, poph-store | 65 |
| transitorio | transient | traduz | poph-matrix, poph-server | 65 |
| condicoes | conditions | traduz | poph-store, console | 64 |
| criacao | creation | traduz | poph-matrix, poph-server | 64 |
| humano | human | traduz | console, poph-server | 64 |
| identificador | identifier | traduz | poph-matrix, poph-server | 63 |
| sujo | dirty | traduz | console | 63 |
| ultimas | last | traduz | poph-agent, scripts | 63 |
| achados | findings | traduz | poph-agent, poph-rag | 62 |
| carimbo | stamp | traduz | poph-server, poph-fluxos | 62 |
| concluida | finished | traduz | poph-fluxos, poph-store | 62 |
| confiavel | reliable | traduz | poph-jev, poph-agent | 62 |
| nos | us | traduz | poph-fluxos, poph-types | 62 |
| vazia | empty | traduz | poph-agent, poph-store | 62 |
| aba | tab | traduz | console | 61 |
| apagar | delete | traduz | poph-rag, poph-store | 61 |
| inspetor | inspector | traduz | console | 61 |
| poder | power | traduz | poph-matrix, poph-types | 61 |
| referencia | reference | traduz | poph, poph-rag | 61 |
| rodando | running | traduz | poph-server, poph-agent | 61 |
| evolucao | evolution | traduz | poph-server | 60 |
| lida | read | traduz | poph-server, poph-store | 60 |
| nem | nor | traduz | poph-server, poph-store | 60 |
| risco | risk | traduz | poph-jev, poph-agent | 60 |
| rotas | routes | traduz | console, poph-server | 60 |
| vaga | slot | traduz | poph-agent, poph-rag | 60 |
| assistente | assistant | traduz | poph-agent, poph-store | 59 |
| dois | two | traduz | poph-fluxos, poph-server | 59 |
| publicada | published | traduz | poph-server, poph-matrix | 59 |
| trava | lock | traduz | poph-rag, poph-fluxos | 59 |
| ativos | active | traduz | poph-server, poph-store | 58 |
| indisponivel | unavailable | traduz | poph-server, poph-plugins | 58 |
| marcar | mark | traduz | poph-store, poph-server | 58 |
| perfil | profile | traduz | poph-matrix, poph-server | 58 |
| acertos | hits | traduz | poph-rag, poph-jev | 57 |
| agiram | acted | traduz | poph-fluxos, poph-store | 57 |
| longo | long | traduz | poph-rag, poph-fluxos | 57 |
| margem | margin | traduz | poph-jev, console | 57 |
| maximo | maximum | traduz | poph-agent, poph-jev | 57 |
| motivos | reasons | traduz | console, poph-retention | 57 |
| armazem | store | traduz | poph-retention, poph | 56 |
| atualizacoes | updates | traduz | poph-store, poph-server | 56 |
| codificar | encode | traduz | poph-matrix, poph-core | 56 |
| entre | between | traduz | poph-fluxos, poph-server | 56 |
| inativo | inactive | traduz | poph-store, poph-agent | 56 |
| marca | mark | traduz | poph-server, poph-matrix | 56 |
| politica | policy | traduz | poph-evolution, poph-agent | 56 |
| recurso | resource | traduz | console, poph-server | 56 |
| contem | contains | traduz | poph-fluxos, poph-server | 55 |
| exclusao | exclusion | traduz | console, poph-server | 55 |
| ligada | linked | traduz | poph-server, poph-types | 55 |
| medidor | meter | traduz | poph-types, poph-server | 55 |
| repetido | repeated | traduz | poph-fluxos, poph-store | 55 |
| valido | valid | traduz | poph-server, poph-fluxos | 55 |
| definitiva | definitive | traduz | poph-matrix, poph-server | 54 |
| desde | since | traduz | poph-store, poph-server | 54 |
| dia | day | traduz | poph-matrix, poph-server | 54 |
| liberados | released | traduz | poph-agent, poph-jev | 54 |
| mesma | same | traduz | poph-fluxos, poph-store | 54 |
| modelos | models | traduz | poph, poph-server | 54 |
| mostra | shows | traduz | poph, console | 54 |
| palavra | word | traduz | scripts, poph-rag | 54 |
| assinatura | signature | traduz | poph-server, poph-files | 53 |
| comparar | compare | traduz | poph-server, poph | 53 |
| compartilhado | shared | traduz | poph-rag, poph-agent | 53 |
| esta | this | traduz | console, poph-plugins | 53 |
| forma | form | traduz | poph-fluxos, poph | 53 |
| posicionar | position | traduz | poph-matrix, poph-server | 53 |
| rodadas | rounds | traduz | poph-agent, console | 53 |
| convidados | guests | traduz | poph-matrix, poph-server | 52 |
| errado | wrong | traduz | poph-server, poph-fluxos | 52 |
| editando | editing | traduz | console | 51 |
| enviada | sent | traduz | poph-server, poph-matrix | 51 |
| gravados | stored | traduz | poph-server, poph-plugin-eco | 51 |
| presenca | presence | traduz | poph-evolution, poph-agent | 51 |
| relatorios | reports | traduz | poph, poph-store | 51 |
| simultaneos | simultaneous | traduz | poph-rag, poph-types | 51 |
| usado | used | traduz | poph-fluxos, poph-agent | 51 |
| ver | see | traduz | console, poph-server | 51 |
| aguardando | waiting | traduz | poph-fluxos, console | 50 |
| ajuda | help | traduz | poph-fluxos, console | 50 |
| orcamento | budget | traduz | poph-agent, poph-rag | 50 |
| preenchidos | filled | traduz | console, poph-fluxos | 50 |
| redacao | writing | traduz | poph-matrix, poph-server | 50 |
| rede | network | traduz | console, poph-matrix | 50 |
| somar | sum | traduz | poph-plugins, poph-agent | 50 |
| alerta | alert | traduz | console | 49 |
| conflitos | conflicts | traduz | poph-store, console | 49 |
| convite | invitation | traduz | poph-matrix, poph-server | 49 |
| despachar | dispatch | traduz | poph-agent, poph-server | 49 |
| gerar | generate | traduz | poph, console | 49 |
| permissao | permission | traduz | poph-core, poph-matrix | 49 |
| similaridade | similarity | traduz | poph-rag, poph-agent | 49 |
| cena | scene | traduz | poph-server | 48 |
| classe | class | traduz | console, poph-agent | 48 |
| custo | cost | traduz | poph-agent, console | 48 |
| devolve | returns | traduz | poph-server, poph-store | 48 |
| duas | two | traduz | poph-server, poph-agent | 48 |
| leituras | reads | traduz | poph-server, console | 48 |
| roteamento | routing | traduz | console | 48 |
| tela | screen | traduz | console | 48 |
| todos | all | traduz | poph, console | 48 |
| vez | time | traduz | poph-server, poph-fluxos | 48 |
| aplicar | apply | traduz | poph-store, poph-server | 47 |
| apoio | support | traduz | poph, poph-core | 47 |
| ativas | active | traduz | poph-store, poph-server | 47 |
| decidir | decide | traduz | poph-matrix, poph-server | 47 |
| edicao | edit | traduz | console, poph-matrix | 47 |
| vinculo | link | traduz | console, poph-server | 47 |
| conectado | connected | traduz | poph-server, console | 46 |
| exportar | export | traduz | console, poph | 46 |
| falta | missing | traduz | poph-matrix, poph-server | 46 |
| gravacao | recording | traduz | poph-store, poph-server | 46 |
| locais | local | traduz | poph-fluxos, poph-store | 46 |
| recarga | reload | traduz | console, poph-server | 46 |
| remoto | remote | traduz | poph, poph-server | 46 |
| removido | removed | traduz | poph-fluxos, console | 46 |
| ruim | poor | traduz | poph-retention, poph | 46 |
| semear | seed | traduz | poph-server, poph-store | 46 |
| transferir | transfer | traduz | poph-jev, poph-store | 46 |
| agrupamento | grouping | traduz | poph-agent, poph-server | 45 |
| comandos | commands | traduz | poph-plugins, poph-store | 45 |
| recebida | received | traduz | poph-agent, poph-server | 45 |
| sao | are | traduz | poph-store, console | 45 |
| aceito | accepted | traduz | poph-files, poph-server | 44 |
| alteradas | changed | traduz | poph-store, poph-server | 44 |
| cada | each | traduz | console, poph-server | 44 |
| decodificar | decode | traduz | poph-server, poph-matrix | 44 |
| desligada | disabled | traduz | poph-server, poph-store | 44 |
| escritor | writer | traduz | poph-store, poph-matrix | 44 |
| frase | sentence | traduz | poph-rag, poph-fluxos | 44 |
| navegar | navigate | traduz | console | 44 |
| pela | by | traduz | poph-server, poph | 44 |
| removida | removed | traduz | poph-fluxos, poph-matrix | 44 |
| renderizar | render | traduz | poph-fluxos, poph-server | 44 |
| sentido | meaning | traduz | poph-matrix, poph-server | 44 |
| criados | created | traduz | poph-server, poph-agent | 43 |
| encerrado | closed | traduz | console, poph-core | 43 |
| escolher | choose | traduz | console, poph-server | 43 |
| extensao | extension | traduz | poph-files, poph-server | 43 |
| palavras | words | traduz | poph-rag, scripts | 43 |
| passar | pass | traduz | poph-server, poph-agent | 43 |
| temperatura | temperature | traduz | poph-core, poph-store | 43 |
| varredura | scan | traduz | poph-retention, poph-store | 43 |
| vetores | vectors | traduz | poph-rag, poph-core | 43 |
| coleta | collection | traduz | poph-fluxos | 42 |
| extracao | extraction | traduz | poph-files, poph-server | 42 |
| legenda | caption | traduz | poph-evolution, poph-server | 42 |
| recarregar | reload | traduz | console, poph-server | 42 |
| avisar | notify | traduz | poph-server, console | 41 |
| dado | given | traduz | poph-fluxos, poph-server | 41 |
| divergente | divergent | traduz | poph-fluxos, poph-server | 41 |
| entao | then | traduz | poph-fluxos, console | 41 |
| escolhido | chosen | traduz | console, poph-agent | 41 |
| interna | internal | traduz | poph-agent | 41 |
| limpar | clean | traduz | scripts, console | 41 |
| minusculo | lowercase | traduz | poph-fluxos, poph-evolution | 41 |
| simples | simple | traduz | poph-fluxos, poph-server | 41 |
| acima | above | traduz | poph-server, poph-fluxos | 40 |
| adiadas | deferred | traduz | poph-server, poph-agent | 40 |
| carrinhos | carts | traduz | poph-agent, console | 40 |
| fica | stays | traduz | poph-server, poph-fluxos | 40 |
| legivel | readable | traduz | poph-core, console | 40 |
| lidos | read | traduz | poph-fluxos, poph-rag | 40 |
| medicao | measurement | traduz | scripts, poph-agent | 40 |
| normalizar | normalise | traduz | console, poph-store | 40 |
| nunca | never | traduz | poph-evolution, poph-server | 40 |
| posicionada | positioned | traduz | poph-matrix, poph-server | 40 |
| processar | process | traduz | poph-agent, poph-server | 40 |
| reais | actual | traduz | poph-agent, console | 40 |
| removidos | removed | traduz | console, poph-rag | 40 |
| resultados | results | traduz | poph-agent, poph-store | 40 |
| transferencia | transfer | traduz | poph-agent, console | 40 |
| agenda | schedule | traduz | poph-agent, poph-server | 39 |
| ano | year | traduz | poph-matrix, poph-server | 39 |
| casca | shell | traduz | console | 39 |
| definicao | definition | traduz | poph-store, poph-server | 39 |
| obtido | obtained | traduz | poph-rag, poph-jev | 39 |
| retentativas | retries | traduz | poph-core, poph-jev | 39 |
| senao | else | traduz | poph-fluxos, raiz | 39 |
| validade | validity | traduz | poph-server, poph-jev | 39 |
| candidata | candidate | traduz | console, poph-jev | 38 |
| compila | compiles | traduz | poph-fluxos, poph | 38 |
| curso | course | traduz | poph-fluxos, console | 38 |
| encerramento | closure | traduz | console, poph-agent | 38 |
| esgotado | exhausted | traduz | poph-plugins, poph-agent | 38 |
| latencia | latency | traduz | poph-server, poph-jev | 38 |
| orfaos | orphans | traduz | poph-retention, poph-store | 38 |
| parada | stop | traduz | poph-server, poph-fluxos | 38 |
| poderes | powers | traduz | poph-matrix | 38 |
| reabrir | reopen | traduz | poph-server, console | 38 |
| saltos | jumps | traduz | poph-fluxos, poph-agent | 38 |
| selecionada | selected | traduz | console | 38 |
| abas | tabs | traduz | console | 37 |
| amostras | samples | traduz | poph-store, poph-server | 37 |
| anotadora | annotator | traduz | poph-server | 37 |
| apagou | deleted | traduz | poph-store, poph-matrix | 37 |
| criando | creating | traduz | console, poph-store | 37 |
| dentro | inside | traduz | poph-fluxos, poph | 37 |
| desconhecido | unknown | traduz | poph-fluxos, poph-evolution | 37 |
| despedida | farewell | traduz | console, poph-jev | 37 |
| digitado | typed | traduz | console, poph-agent | 37 |
| extraido | extracted | traduz | poph-agent, poph-files | 37 |
| formulario | form | traduz | console | 37 |
| instalacao | installation | traduz | console, poph | 37 |
| lidas | read | traduz | poph-store, console | 37 |
| navegacao | navigation | traduz | console | 37 |
| salvo | saved | traduz | console, poph-server | 37 |
| servico | service | traduz | poph-server, poph | 37 |
| volatil | volatile | traduz | poph-agent | 37 |
| aceita | accepts | traduz | poph-fluxos, poph-server | 36 |
| codigos | codes | traduz | poph-fluxos, poph-server | 36 |
| criterio | criterion | traduz | poph-jev, poph-types | 36 |
| encaminhar | forward | traduz | poph-fluxos, poph-server | 36 |
| enviadas | sent | traduz | poph-server, poph-agent | 36 |
| funcoes | functions | traduz | poph-fluxos, poph-agent | 36 |
| geracao | generation | traduz | poph-plugins, poph-rag | 36 |
| impresso | printed | traduz | poph-fluxos, poph | 36 |
| proibido | forbidden | traduz | poph-server, poph | 36 |
| relevancia | relevance | traduz | poph-rag, poph-agent | 36 |
| remover | remove | traduz | console, poph-agent | 36 |
| tocar | touch | traduz | poph-store, poph-server | 36 |
| valida | validates | traduz | poph-fluxos, poph-server | 36 |
| buscas | searches | traduz | poph-agent | 35 |
| convites | invitations | traduz | poph-matrix, poph-server | 35 |
| espelhar | mirror | traduz | poph-matrix, poph-server | 35 |
| executada | executed | traduz | poph-agent, poph-fluxos | 35 |
| fecha | closes | traduz | poph-fluxos, poph-agent | 35 |
| fontes | sources | traduz | poph, poph-agent | 35 |
| habilitado | enabled | traduz | poph-server, poph-matrix | 35 |
| icone | icon | traduz | console | 35 |
| inteiro | integer | traduz | poph, console | 35 |
| liberado | released | traduz | poph-server, poph-store | 35 |
| pertence | belongs | traduz | poph-store, console | 35 |
| plano | plan | traduz | poph-server, poph-store | 35 |
| ponto | point | traduz | console, poph-agent | 35 |
| aguardar | wait | traduz | poph-agent, poph-core | 34 |
| alterada | changed | traduz | poph-server, console | 34 |
| atalhos | shortcuts | traduz | console | 34 |
| booleano | boolean | traduz | poph-fluxos, poph | 34 |
| cancelado | cancelled | traduz | console, poph-core | 34 |
| comportamento | behaviour | traduz | poph-agent | 34 |
| continua | continues | traduz | poph-store, poph-agent | 34 |
| fixa | fixed | traduz | poph-rag, poph-matrix | 34 |
| gravadas | stored | traduz | poph-server, poph-agent | 34 |
| ligacao | link | traduz | console, poph | 34 |
| nomeado | named | traduz | poph-fluxos, poph-store | 34 |
| pareamento | pairing | traduz | poph-evolution, poph-server | 34 |
| principio | principle | traduz | console, poph-types | 34 |
| pronto | ready | traduz | poph-plugins, console | 34 |
| resolve | resolves | traduz | scripts, poph-fluxos | 34 |
| soma | sum | traduz | poph-jev, poph-agent | 34 |
| variaveis | variables | traduz | poph-fluxos, poph | 34 |
| aparece | appears | traduz | poph-server, poph | 33 |
| baratas | cheap | traduz | poph-agent, poph-rag | 33 |
| comparacao | comparison | traduz | poph, poph-fluxos | 33 |
| conta | counts | traduz | poph-server, poph-fluxos | 33 |
| declarada | declared | traduz | poph-server, poph-files | 33 |
| estados | states | traduz | poph-server, poph-plugins | 33 |
| notificar | notify | traduz | poph-fluxos, poph-server | 33 |
| oferecidas | offered | traduz | poph-agent | 33 |
| painel | panel | traduz | console, poph-agent | 33 |
| porque | because | traduz | console, poph-evolution | 33 |
| programa | program | traduz | poph-plugins, poph-plugin-eco | 33 |
| sonda | probe | traduz | poph-server, console | 33 |
| abaixo | below | traduz | console, poph-agent | 32 |
| alterar | modify | traduz | console, poph | 32 |
| aplicavel | applicable | traduz | poph-jev | 32 |
| carregamento | loading | traduz | poph-fluxos, poph-rag | 32 |
| credencial | credential | traduz | poph-matrix, console | 32 |
| diferente | different | traduz | poph-fluxos, poph | 32 |
| hibrida | hybrid | traduz | poph-rag, poph-server | 32 |
| leva | takes | traduz | poph-matrix, poph-store | 32 |
| loja | store | traduz | poph-server, console | 32 |
| ordenadas | sorted | traduz | poph-store, poph-jev | 32 |
| passos | steps | traduz | poph-server, console | 32 |
| recuperados | recovered | traduz | console, poph-agent | 32 |
| redacoes | writings | traduz | poph-server, poph-matrix | 32 |
| requisicoes | requests | traduz | poph-matrix, scripts | 32 |
| segura | safe | traduz | console, poph-agent | 32 |
| usa | uses | traduz | poph-agent, console | 32 |
| verificacao | verification | traduz | poph, poph-evolution | 32 |
| cai | falls | traduz | poph-agent, poph-matrix | 31 |
| capacidades | capabilities | traduz | poph-jev, poph-types | 31 |
| condicao | condition | traduz | poph-fluxos, poph-server | 31 |
| efetivos | effective | traduz | poph-jev, poph-agent | 31 |
| esperados | expected | traduz | poph-server, poph-rag | 31 |
| fator | factor | traduz | poph-server, poph-agent | 31 |
| feita | done | traduz | poph-server, poph | 31 |
| inseridas | inserted | traduz | console, poph-store | 31 |
| mais | more | traduz | console, poph-server | 31 |
| preparado | prepared | traduz | poph-rag, poph-server | 31 |
| quadro | board | traduz | console, scripts | 31 |
| salvando | saving | traduz | console | 31 |
| temporaria | temporary | traduz | poph, console | 31 |
| terceira | third | traduz | poph-agent, poph-server | 31 |
| trilha | trail | traduz | console | 31 |
| apagadas | deleted | traduz | poph-server, poph-matrix | 30 |
| capturados | captured | traduz | poph-server | 30 |
| consultas | queries | traduz | poph-rag, console | 30 |
| deixa | leaves | traduz | poph-store, poph-fluxos | 30 |
| dica | hint | traduz | console, poph-fluxos | 30 |
| divergencias | divergences | traduz | poph, poph-server | 30 |
| expor | expose | traduz | poph-evolution, poph | 30 |
| indexar | index | traduz | poph-rag, poph-agent | 30 |
| mantem | keeps | traduz | poph-agent, poph-matrix | 30 |
| mes | month | traduz | poph-matrix, poph-server | 30 |
| mutacao | mutation | traduz | poph-fluxos | 30 |
| pede | asks | traduz | poph-fluxos, poph-jev | 30 |
| profundidade | depth | traduz | poph-rag, poph-fluxos | 30 |
| responde | answers | traduz | poph-server, poph-agent | 30 |
| sair | exit | traduz | console, poph-rag | 30 |
| segurar | hold | traduz | poph-server, console | 30 |
| tratar | handle | traduz | poph-server, poph-plugins | 30 |
| abertura | opening | traduz | poph-store, poph-server | 29 |
| alternar | toggle | traduz | console | 29 |
| atualizada | updated | traduz | poph-store, console | 29 |
| avulsa | one_off | traduz | poph-server, console | 29 |
| demais | others | traduz | poph-fluxos, poph-evolution | 29 |
| entrou | entered | traduz | poph-store, poph-fluxos | 29 |
| estruturadas | structured | traduz | console | 29 |
| livre | free | traduz | poph-server, poph-fluxos | 29 |
| migrar | migrate | traduz | poph-store, poph | 29 |
| posicoes | positions | traduz | poph-fluxos, poph-server | 29 |
| probabilidades | probabilities | traduz | poph-jev, poph-agent | 29 |
| quantas | how_many | traduz | poph-server, poph-store | 29 |
| recusar | refuse | traduz | poph-agent, poph-server | 29 |
| relativo | relative | traduz | poph-server, poph | 29 |
| severidade | severity | traduz | poph-fluxos, poph-server | 29 |
| adiados | deferred | traduz | poph-agent, poph-server | 28 |
| cancelar | cancel | traduz | console, poph-core | 28 |
| concorrencia | concurrency | traduz | poph-server, poph-matrix | 28 |
| configurar | configure | traduz | poph-server, poph | 28 |
| dono | owner | traduz | poph-retention, poph-store | 28 |
| estava | was | traduz | poph-store, poph | 28 |
| falhos | faulty | traduz | poph-matrix, poph-server | 28 |
| inclui | includes | traduz | console, poph-agent | 28 |
| inferencia | inference | traduz | poph-rag, poph-agent | 28 |
| limpa | clean | traduz | poph-server, poph-store | 28 |
| organizacoes | organisations | traduz | console, poph-server | 28 |
| passa | passes | traduz | console, poph-fluxos | 28 |
| provisionar | provision | traduz | poph-matrix, poph-server | 28 |
| reindexar | reindex | traduz | poph, console | 28 |
| reserva | reservation | traduz | poph-agent, poph-server | 28 |
| sensivel | sensitive | traduz | poph, poph-types | 28 |
| usada | used | traduz | poph-fluxos, poph-server | 28 |
| vizinha | neighbouring | traduz | poph-server, poph | 28 |
| adiado | deferred | traduz | poph-server, poph-agent | 27 |
| apaga | deletes | traduz | poph-retention, poph-server | 27 |
| atualizadas | updated | traduz | console, poph-server | 27 |
| auditado | audited | traduz | poph-server, poph-store | 27 |
| configurado | configured | traduz | console, poph-evolution | 27 |
| confirmando | confirming | traduz | console | 27 |
| definitivos | definitive | traduz | poph-server, poph-matrix | 27 |
| dona | owner | traduz | poph-server, poph-store | 27 |
| esperas | waits | traduz | poph-agent, poph-fluxos | 27 |
| filtros | filters | traduz | console, poph-server | 27 |
| foto | photo | traduz | poph-retention, poph-server | 27 |
| novas | new | traduz | poph-server, console | 27 |
| operacoes | operations | traduz | poph-server, poph | 27 |
| posicional | positional | traduz | poph-fluxos | 27 |
| produzido | produced | traduz | poph-fluxos, poph-agent | 27 |
| seletor | selector | traduz | console | 27 |
| situacoes | situations | traduz | console, poph-jev | 27 |
| telemetria | telemetry | traduz | console | 27 |
| tokenizador | tokeniser | traduz | poph-rag | 27 |
| topo | top | traduz | poph-fluxos, console | 27 |
| validas | valid | traduz | poph-jev, poph-fluxos | 27 |
| velho | old | traduz | poph-retention, poph-agent | 27 |
| abandonar | abandon | traduz | poph-server, poph-fluxos | 26 |
| adquirir | acquire | traduz | poph-agent, poph-core | 26 |
| cliques | clicks | traduz | console | 26 |
| comercial | commercial | traduz | poph-server, poph-store | 26 |
| compor | compose | traduz | poph-agent, poph | 26 |
| degradada | degraded | traduz | console | 26 |
| demora | delay | traduz | poph-agent | 26 |
| disponivel | available | traduz | poph-rag, poph-jev | 26 |
| dispositivo | device | traduz | poph-matrix, poph-server | 26 |
| diz | says | traduz | poph, poph-fluxos | 26 |
| dormir | sleep | traduz | scripts | 26 |
| entra | enters | traduz | poph-agent, poph-server | 26 |
| faixa | range | traduz | poph-types, poph-store | 26 |
| grafo | graph | traduz | poph-fluxos | 26 |
| lotes | batches | traduz | poph-rag | 26 |
| publicadas | published | traduz | poph-server | 26 |
| rodar | run | traduz | poph-agent, poph | 26 |
| roteador | router | traduz | poph-server, poph-matrix | 26 |
| vezes | times | traduz | poph-server, console | 26 |
| visiveis | visible | traduz | console, scripts | 26 |
| baixado | downloaded | traduz | poph-server, poph-matrix | 25 |
| conferido | checked | traduz | poph, poph-store | 25 |
| confirmacao | confirmation | traduz | console, poph-agent | 25 |
| cria | creates | traduz | poph-store, poph | 25 |
| encontrada | found | traduz | poph-store, poph-agent | 25 |
| exigidos | required | traduz | poph-fluxos, poph-agent | 25 |
| expirar | expire | traduz | poph-matrix, console | 25 |
| impressao | printout | traduz | console | 25 |
| invalidos | invalid | traduz | poph-types, poph-fluxos | 25 |
| linguagem | language | traduz | scripts, poph-fluxos | 25 |
| migalhas | crumbs | traduz | console | 25 |
| minimo | minimum | traduz | poph-types, poph-server | 25 |
| movida | moved | traduz | poph-matrix, poph-server | 25 |
| norma | norm | traduz | poph-rag | 25 |
| opcional | optional | traduz | poph-store, poph-agent | 25 |
| peso | weight | traduz | poph-fluxos | 25 |
| recuperado | recovered | traduz | poph-agent | 25 |
| respeita | respects | traduz | poph-agent, poph-server | 25 |
| revogar | revoke | traduz | console, poph-store | 25 |
| segue | follows | traduz | poph-server, poph-agent | 25 |
| semaforo | semaphore | traduz | poph-agent, poph-rag | 25 |
| sobrescrever | overwrite | traduz | poph-server, console | 25 |
| tarefas | tasks | traduz | poph-server, poph-rag | 25 |
| vinculos | links | traduz | console, poph-server | 25 |
| vivos | live | traduz | poph-agent, scripts | 25 |
| voltar | return | traduz | console | 25 |
| adicionar | add | traduz | poph-agent, console | 24 |
| alterado | changed | traduz | console, poph-server | 24 |
| autorizado | authorized | traduz | poph-server, console | 24 |
| derivado | derived | traduz | poph-store, poph | 24 |
| fechado | closed | traduz | poph-server, poph-evolution | 24 |
| habilitada | enabled | traduz | poph-agent, poph-jev | 24 |
| hora | hour | traduz | poph-fluxos, poph-store | 24 |
| indexacao | indexing | traduz | poph-agent, poph-rag | 24 |
| lixo | garbage | traduz | poph-plugins, poph-types | 24 |
| manifestos | manifests | traduz | poph-server | 24 |
| paginas | pages | traduz | poph-server, console | 24 |
| recibo | receipt | traduz | poph-evolution, poph-fluxos | 24 |
| recuo | backoff | traduz | poph-fluxos, poph | 24 |
| requisicao | request | traduz | poph-matrix, poph-core | 24 |
| sim | yes | traduz | poph-fluxos, console | 24 |
| sufixo | suffix | traduz | poph-matrix, console | 24 |
| teclar | keypress | traduz | console | 24 |
| travar | lock | traduz | poph-agent, poph-jev | 24 |
| unico | single | traduz | poph-server, poph-fluxos | 24 |
| velha | old | traduz | poph-server, poph | 24 |
| abandonada | abandoned | traduz | poph-store, poph-fluxos | 23 |
| baixa | low | traduz | poph-jev, poph-agent | 23 |
| cifrados | encrypted | traduz | poph-store, poph | 23 |
| congelados | frozen | traduz | scripts | 23 |
| cortar | cut | traduz | poph-agent, poph-matrix | 23 |
| curto | short | traduz | console, poph-matrix | 23 |
| desabilitado | disabled | traduz | console, poph-agent | 23 |
| dourada | golden | traduz | poph-fluxos | 23 |
| expirada | expired | traduz | console, poph-evolution | 23 |
| filas | queues | traduz | console, poph-agent | 23 |
| instante | instant | traduz | poph-retention, poph-store | 23 |
| juntos | together | traduz | poph-server, poph-matrix | 23 |
| melhores | best | traduz | poph-rag | 23 |
| nota | note | traduz | poph-rag, console | 23 |
| propria | own | traduz | poph-server, console | 23 |
| repeticao | repetition | traduz | poph-evolution, poph-agent | 23 |
| repetir | repeat | traduz | poph-server, poph-matrix | 23 |
| resolvedor | resolver | traduz | poph-agent, poph-plugin-eco | 23 |
| validador | validator | traduz | poph-fluxos | 23 |
| verificar | verify | traduz | console, poph | 23 |
| vinculado | linked | traduz | console, poph | 23 |
| visita | visit | traduz | poph-fluxos, poph-server | 23 |
| alfa | alpha | traduz | poph-agent, poph-fluxos | 22 |
| atendimentos | services | traduz | poph-fluxos, poph | 22 |
| automatico | automatic | traduz | poph-store, poph-server | 22 |
| chegada | arrival | traduz | poph-agent, poph-fluxos | 22 |
| combinador | combinator | traduz | poph-fluxos | 22 |
| continuacao | continuation | traduz | poph-agent | 22 |
| disfarce | disguise | traduz | poph-server, poph-matrix | 22 |
| exibicao | display | traduz | poph-matrix, poph-fluxos | 22 |
| faltando | missing | traduz | poph-fluxos, poph-agent | 22 |
| final | final | traduz | poph-matrix, poph-fluxos | 22 |
| finalizacao | finalisation | traduz | poph-agent, poph-store | 22 |
| gerados | generated | traduz | poph-rag, console | 22 |
| inicia | starts | traduz | poph-agent, poph-rag | 22 |
| iniciado | started | traduz | poph-agent, poph-store | 22 |
| livres | free | traduz | poph-store, poph-core | 22 |
| meio | middle | traduz | poph-server, poph-store | 22 |
| nenhuma | none | traduz | console, poph-fluxos | 22 |
| pai | parent | traduz | poph-matrix, poph | 22 |
| precisa | needs | traduz | scripts, poph-fluxos | 22 |
| suporte | support | traduz | poph-matrix, poph-store | 22 |
| verificacoes | verifications | traduz | poph | 22 |
| vistas | views | traduz | poph-server, poph-store | 22 |
| andamento | progress | traduz | poph-agent, poph-rag | 21 |
| apagados | deleted | traduz | poph-retention, console | 21 |
| atalho | shortcut | traduz | console | 21 |
| auditorias | audits | traduz | poph-retention, poph-store | 21 |
| avulso | one_off | traduz | console | 21 |
| cargas | loads | traduz | poph-fluxos, poph-server | 21 |
| celula | cell | traduz | console, poph-files | 21 |
| contato | contact | traduz | poph-types, poph-evolution | 21 |
| derivar | derive | traduz | poph-server, poph | 21 |
| desconhecida | unknown | traduz | poph-server, console | 21 |
| duplicado | duplicate | traduz | poph-store, poph-agent | 21 |
| extrator | extractor | traduz | poph-files, poph-server | 21 |
| feito | done | traduz | poph-server, poph-retention | 21 |
| ida | trip | traduz | poph-types, poph-fluxos | 21 |
| informado | informed | traduz | poph-jev, poph-server | 21 |
| malformado | malformed | traduz | poph-fluxos, poph-server | 21 |
| mediana | median | traduz | poph-store, poph-agent | 21 |
| menor | smaller | traduz | poph-fluxos, poph-server | 21 |
| mim | me | traduz | poph-evolution, poph-server | 21 |
| minuscula | lowercase | traduz | poph-files, poph-server | 21 |
| nesta | in_this | traduz | console, poph-store | 21 |
| orfao | orphan | traduz | poph-retention, poph-store | 21 |
| proprias | own | traduz | poph-agent, poph | 21 |
| quantos | how_many | traduz | poph-server, poph-fluxos | 21 |
| recebidas | received | traduz | poph-agent, poph-jev | 21 |
| recente | recent | traduz | poph-retention, poph-store | 21 |
| regras | rules | traduz | poph-fluxos, poph-matrix | 21 |
| segmento | segment | traduz | poph-server, poph-matrix | 21 |
| telefone | phone | traduz | poph-matrix, poph-server | 21 |
| trocado | swapped | traduz | poph-server, poph-matrix | 21 |
| usados | used | traduz | poph-fluxos, poph-agent | 21 |
| achadas | found | traduz | poph-agent, poph-fluxos | 20 |
| arquivado | archived | traduz | poph-store, console | 20 |
| celulas | cells | traduz | poph-server, scripts | 20 |
| complementos | complements | traduz | poph-server | 20 |
| criadas | created | traduz | poph-server, console | 20 |
| declara | declares | traduz | poph-server, console | 20 |
| definitivas | definitive | traduz | poph-agent, poph-server | 20 |
| derrubar | drop | traduz | poph-server, poph-store | 20 |
| disco | disk | traduz | poph-evolution, poph-server | 20 |
| entregue | delivered | traduz | poph-server, poph-agent | 20 |
| exportacao | export | traduz | console, poph-server | 20 |
| fala | speaks | traduz | poph-agent, poph-server | 20 |
| guardadas | guarded | traduz | poph-server, poph-fluxos | 20 |
| guardados | guarded | traduz | poph-evolution, poph-server | 20 |
| listagem | listing | traduz | poph-store, poph-server | 20 |
| lugar | place | traduz | poph-fluxos, poph-server | 20 |
| medir | measure | traduz | poph-server, poph-jev | 20 |
| migalha | crumb | traduz | console | 20 |
| mostrado | shown | traduz | poph, poph-server | 20 |
| observar | observe | traduz | poph-agent, poph-server | 20 |
| percentil | percentile | traduz | poph-jev, poph-store | 20 |
| portugues | portuguese | traduz | poph, poph-server | 20 |
| redigir | draft | traduz | poph-matrix, poph-server | 20 |
| remota | remote | traduz | console, poph-server | 20 |
| secoes | sections | traduz | poph-agent | 20 |
| selecionado | selected | traduz | console | 20 |
| trocaram | swapped | traduz | console, poph-server | 20 |
| vagas | slots | traduz | poph-rag, poph-agent | 20 |
| versoes | versions | traduz | poph-server, console | 20 |
| virtuais | virtual | traduz | poph-matrix, poph-server | 20 |
| vitima | victim | traduz | poph-server, poph-fluxos | 20 |
| alvos | target | traduz | poph-rag, poph-matrix | 19 |
| aspas | quotes | traduz | poph-fluxos, poph | 19 |
| certificado | certificate | traduz | poph-core, poph-jev | 19 |
| conexoes | connections | traduz | poph-core, poph-jev | 19 |
| contrato | contract | traduz | poph-matrix, poph-jev | 19 |
| copiar | copy | traduz | console, scripts | 19 |
| decorrido | elapsed | traduz | poph-agent, poph-files | 19 |
| desconectar | disconnect | traduz | console, poph-server | 19 |
| espelhadas | mirrored | traduz | poph-server, scripts | 19 |
| estavel | stable | traduz | poph, poph-matrix | 19 |
| este | this | traduz | console, poph-agent | 19 |
| ilegivel | unreadable | traduz | poph-evolution, poph-matrix | 19 |
| imagens | images | traduz | poph-agent, poph-files | 19 |
| invalidas | invalid | traduz | poph-fluxos, poph-agent | 19 |
| latencias | latencies | traduz | poph-jev, poph-agent | 19 |
| publicado | published | traduz | poph-server, console | 19 |
| quantizar | quantise | traduz | poph-rag | 19 |
| transitorios | transient | traduz | poph-server, poph-matrix | 19 |
| unicos | single | traduz | poph-store, poph-fluxos | 19 |
| abstencao | abstention | traduz | poph-jev, poph-agent | 18 |
| acento | accent | traduz | console, poph-rag | 18 |
| avisadas | notified | traduz | poph-server | 18 |
| convidado | guest | traduz | poph-server, poph | 18 |
| criterios | criteria | traduz | poph-jev, poph-agent | 18 |
| digitos | digits | traduz | poph-fluxos, poph-matrix | 18 |
| editar | edit | traduz | console, poph-agent | 18 |
| enganosa | misleading | traduz | poph-files, poph-server | 18 |
| esperadas | expected | traduz | poph-server, poph-store | 18 |
| expulsoes | evictions | traduz | poph-fluxos, poph-server | 18 |
| feitas | done | traduz | poph-store, poph-server | 18 |
| guardado | stored | traduz | poph-server, poph-store | 18 |
| hoje | today | traduz | poph-agent, poph-server | 18 |
| importando | importing | traduz | console | 18 |
| isolamento | isolation | traduz | poph-server, poph-files | 18 |
| limiar | threshold | traduz | poph-agent, poph-jev | 18 |
| migracoes | migrations | traduz | poph-store, poph | 18 |
| periodo | period | traduz | poph-server | 18 |
| pico | peak | traduz | poph-rag, poph-files | 18 |
| simular | simulate | traduz | poph-retention, poph-store | 18 |
| tudo | everything | traduz | poph-store, poph-server | 18 |
| vincular | link | traduz | poph-server, poph | 18 |
| adicionadas | added | traduz | console, poph-server | 17 |
| agendar | schedule | traduz | poph-fluxos, poph-agent | 17 |
| alheio | foreign | traduz | poph-server, poph-agent | 17 |
| apenas | only | traduz | poph-jev, poph-agent | 17 |
| avancou | advanced | traduz | poph-store, poph-fluxos | 17 |
| baixada | downloaded | traduz | poph-evolution, poph-server | 17 |
| caminhos | paths | traduz | scripts, poph-fluxos | 17 |
| coletada | collected | traduz | poph-fluxos | 17 |
| concluidos | finished | traduz | poph-agent, poph-rag | 17 |
| consultado | queried | traduz | poph-jev, poph-agent | 17 |
| cosseno | cosine | traduz | poph-rag | 17 |
| deixou | left | traduz | poph-fluxos | 17 |
| desconhecidas | unknown | traduz | poph-server, console | 17 |
| explicacao | explanation | traduz | console, poph-agent | 17 |
| filhos | children | traduz | poph-fluxos, poph-server | 17 |
| folha | sheet | traduz | poph-fluxos | 17 |
| nomeavel | nameable | traduz | poph-server | 17 |
| pares | pairs | traduz | poph, poph-fluxos | 17 |
| pedacos | pieces | traduz | console, poph-rag | 17 |
| perfis | profiles | traduz | poph-server | 17 |
| pessoa | person | traduz | poph-matrix, console | 17 |
| reenvios | resends | traduz | poph-agent, poph-server | 17 |
| removidas | removed | traduz | console, poph-server | 17 |
| renomeacoes | renames | traduz | poph-server | 17 |
| retomada | resumption | traduz | poph-agent, poph-matrix | 17 |
| simulacao | simulation | traduz | poph, poph-retention | 17 |
| tres | three | traduz | poph-fluxos, poph-jev | 17 |
| varrer | scan | traduz | poph-retention, poph-rag | 17 |
| acessar | access | traduz | console | 16 |
| acumulado | accumulated | traduz | poph-matrix, poph-types | 16 |
| apagada | deleted | traduz | poph-store, poph-server | 16 |
| aplicada | applied | traduz | poph-store, poph-agent | 16 |
| construir | build | traduz | poph-server, poph | 16 |
| declarados | declared | traduz | poph-server, poph-fluxos | 16 |
| dividir | split | traduz | poph-rag, scripts | 16 |
| duplicadas | duplicated | traduz | poph-server, poph-agent | 16 |
| esquecer | forget | traduz | poph-server, poph-agent | 16 |
| estatisticas | statistics | traduz | poph-agent, poph-server | 16 |
| finalizada | finalised | traduz | poph-store, poph-agent | 16 |
| finalizar | finalise | traduz | poph-agent, poph-core | 16 |
| gerado | generated | traduz | poph, poph-server | 16 |
| guardar | keep | traduz | poph-evolution, poph-server | 16 |
| mudancas | changes | traduz | poph-server, console | 16 |
| notificacao | notification | traduz | poph-fluxos, poph-server | 16 |
| passagem | passage | traduz | poph-server, poph-store | 16 |
| permitido | allowed | traduz | poph-server, poph-fluxos | 16 |
| recarregando | reloading | traduz | console | 16 |
| regenerada | regenerated | traduz | poph-agent | 16 |
| registrada | recorded | traduz | poph-server, poph-agent | 16 |
| remetentes | senders | traduz | poph-server | 16 |
| rotulado | labelled | traduz | poph-jev, poph-rag | 16 |
| seguidas | consecutive | traduz | poph-server | 16 |
| sintaxe | syntax | traduz | poph-fluxos, poph | 16 |
| traco | trace | traduz | poph-server, poph | 16 |
| vencidas | expired | traduz | poph-server, poph-store | 16 |
| alterados | changed | traduz | poph-rag, poph | 15 |
| aninhado | nested | traduz | console, poph-jev | 15 |
| capturado | captured | traduz | poph-matrix, poph-server | 15 |
| capturar | capture | traduz | scripts, poph-server | 15 |
| copiado | copied | traduz | console | 15 |
| destinos | targets | traduz | poph-fluxos, poph-rag | 15 |
| duplicados | duplicated | traduz | poph-agent, poph-plugin-eco | 15 |
| embutido | embedded | traduz | poph-rag, poph-evolution | 15 |
| encerradas | closed | traduz | poph-agent, poph-server | 15 |
| escalares | scalars | traduz | poph | 15 |
| espelhada | mirrored | traduz | poph-matrix, poph-server | 15 |
| estimar | estimate | traduz | poph-agent | 15 |
| fechamento | closing | traduz | poph-store, poph-server | 15 |
| filtrar | filter | traduz | console, poph-store | 15 |
| forcado | forced | traduz | poph-server, poph | 15 |
| gerenciar | manage | traduz | console | 15 |
| inalterados | unchanged | traduz | poph-rag, poph | 15 |
| leitores | readers | traduz | poph-store, poph-files | 15 |
| processado | processed | traduz | poph-agent, poph-store | 15 |
| recusados | refused | traduz | poph-agent, poph-server | 15 |
| redigidas | drafted | traduz | poph-server | 15 |
| renomear | rename | traduz | poph-server, poph-files | 15 |
| resolucao | resolution | traduz | poph-fluxos, poph | 15 |
| revogado | revoked | traduz | poph, poph-server | 15 |
| temporario | temporary | traduz | poph-files, poph | 15 |
| terceiro | third | traduz | poph-server, poph-agent | 15 |
| truncar | truncate | traduz | poph-jev, poph | 15 |
| agir | act | traduz | poph-server, poph-agent | 14 |
| alcancados | reached | traduz | poph-fluxos, console | 14 |
| antigos | old | traduz | console, poph-matrix | 14 |
| apagado | deleted | traduz | poph, console | 14 |
| apertado | tight | traduz | poph-server, poph-fluxos | 14 |
| aplica | applies | traduz | poph-jev, poph-store | 14 |
| assinados | signed | traduz | poph-server | 14 |
| clicar | click | traduz | scripts, console | 14 |
| colecao | collection | traduz | poph-server | 14 |
| convidar | invite | traduz | poph-matrix | 14 |
| decididas | decided | traduz | console, poph-jev | 14 |
| deixar | leave | traduz | poph-agent, poph-core | 14 |
| descartada | discarded | traduz | poph-plugins, poph-evolution | 14 |
| digitando | typing | traduz | poph-evolution, console | 14 |
| direto | direct | traduz | poph-server, console | 14 |
| disposicao | layout | traduz | poph-server, console | 14 |
| esgotadas | exhausted | traduz | poph-server, poph-agent | 14 |
| idade | age | traduz | poph-retention, console | 14 |
| idempotente | idempotent | traduz | poph-store, poph | 14 |
| iniciando | starting | traduz | poph-evolution, poph-plugins | 14 |
| instalado | installed | traduz | poph-rag, poph-server | 14 |
| minutos | minutes | traduz | scripts, console | 14 |
| nas | in_the | traduz | poph-server, poph-agent | 14 |
| paginacao | pagination | traduz | poph-server | 14 |
| parado | stopped | traduz | poph-fluxos, poph | 14 |
| pontuacao | score | traduz | poph-rag | 14 |
| poupadas | saved | traduz | poph-fluxos, poph-server | 14 |
| processadas | processed | traduz | poph-server | 14 |
| rapidas | fast | traduz | poph-agent | 14 |
| reiniciar | restart | traduz | poph-plugins, poph-server | 14 |
| segmentos | segments | traduz | poph-server | 14 |
| substituir | replace | traduz | poph-store, poph-server | 14 |
| vazamento | leak | traduz | poph-server | 14 |
| visivel | visible | traduz | console, poph-agent | 14 |
| voltou | returned | traduz | poph-store, poph-fluxos | 14 |
| comentarios | comments | traduz | poph-fluxos, scripts | 13 |
| seguro | safe | traduz | poph-matrix, poph-server | 13 |
| enquete | poll | traduz | poph-evolution, poph-types | 12 |
| alteracao | change | traduz | poph-store, poph-server | 11 |
| antigas | old | traduz | poph-matrix, poph-retention | 11 |
| metrica | metric | traduz | poph-server | 11 |
| propriedades | properties | traduz | poph-agent, console | 11 |
| negativo | negative | traduz | poph-fluxos, poph-retention | 10 |
| trabalho | work | traduz | poph-fluxos | 10 |
| corpos | bodies | traduz | poph-server | 9 |
| falharam | failed | traduz | poph-agent, poph-plugin-eco | 9 |
| metodos | methods | traduz | poph-server | 9 |
| ouvintes | listeners | traduz | scripts, console | 9 |
| prontos | ready | traduz | poph-matrix | 9 |
| subida | rise | traduz | poph-server, console | 9 |
| terminar | finish | traduz | poph-server, poph-core | 9 |
| capturas | captures | traduz | poph-evolution | 8 |
| ocupada | busy | traduz | poph-agent, scripts | 8 |
| prontas | ready | traduz | poph-server, poph-store | 8 |
| rotulos | labels | traduz | console | 8 |
| atualizacao | update | traduz | console, poph-store | 7 |
| referencias | references | traduz | poph-store, poph | 7 |
| cartao | card | traduz | console | 6 |
| frente | front | traduz | poph-store, poph-rag | 6 |
| largura | width | traduz | scripts, poph-fluxos | 6 |
| pessoas | people | traduz | poph-matrix, console | 6 |
| problema | problem | traduz | console, raiz | 6 |
| retirar | withdraw | traduz | poph | 6 |
| conclusao | completion | traduz | poph-server, poph-agent | 3 |
| combina | combines | traduz | poph-matrix | 2 |
| consertar | repair | traduz | poph-server | 2 |
| equipe | team | traduz | poph-fluxos, poph-server | 2 |
| recursos | resources | traduz | console | 2 |
| selecionar | select | traduz | console | 2 |
| correcao | fix | traduz | poph-matrix | 1 |
| rotina | routine | traduz | poph-server | 1 |
| False | — | congelado | linguagem de fluxos | - |
| None | — | congelado | linguagem de fluxos | - |
| True | — | congelado | linguagem de fluxos | - |
| abaixo | — | congelado | metrica poph_fluxos_g9_abaixo_do_limiar_total | - |
| abandonar | — | congelado | operation_id fichas_abandonar | - |
| aberta | — | congelado | coluna aberta_em | - |
| abs | — | congelado | linguagem de fluxos | - |
| acao | — | congelado | campo_json acao; linguagem de fluxos; operation_id fichas_acao | - |
| acertos | — | congelado | metrica poph_fluxos_cache_acertos_total | - |
| agente | — | congelado | flag_cli --agente; indice ix_agente_fluxos_fluxo; indice ix_fichas_org_agente; e mais 1 | - |
| agentes | — | congelado | metrica poph_agentes_carregados | - |
| agiram | — | congelado | coluna atores_que_agiram_json | - |
| all | — | congelado | linguagem de fluxos | - |
| anexo | — | congelado | operation_id teste_chat_anexo_conteudo; protocolo_plugin max_anexo_bytes | - |
| anexos | — | congelado | protocolo_plugin anexos | - |
| any | — | congelado | linguagem de fluxos | - |
| ao | — | congelado | variavel_ambiente POPH_FLUXOS_MAX_SALTOS_DECISAO; variavel_ambiente POPH_RETENCAO_DIAS | - |
| arquivos | — | congelado | operation_id arquivos_conteudo; operation_id arquivos_criar; operation_id arquivos_excluir; e mais 2 | - |
| ativa | — | congelado | chave_toml handoff.orientacao_ativa; indice ux_fluxo_execucoes_ativa | - |
| ativas | — | congelado | metrica poph_conversas_ativas | - |
| ativo | — | congelado | chave_toml fluxos.ativo; chave_toml jev.ativo | - |
| ator | — | congelado | coluna ator_ref; coluna ator_tipo | - |
| atores | — | congelado | coluna atores_que_agiram_json | - |
| atraso | — | congelado | chave_toml turno.atraso_falha_modelo_ms | - |
| atual | — | congelado | coluna no_atual; coluna revisao_atual | - |
| atualizada | — | congelado | indice ix_conversations_atualizada | - |
| atualizar | — | congelado | operation_id atualizar; operation_id atualizar_item; operation_id categorias_atualizar; e mais 4 | - |
| avisos | — | congelado | tabela matrix_avisos_falha | - |
| bool | — | congelado | linguagem de fluxos | - |
| campo | — | congelado | linguagem de fluxos | - |
| campos | — | congelado | campo_json campos | - |
| canal | — | congelado | coluna canal; flag_cli --canal | - |
| capacidades | — | congelado | protocolo_plugin capacidades | - |
| cargas | — | congelado | metrica poph_fluxos_cache_cargas_total | - |
| categorias | — | congelado | operation_id categorias_atualizar; operation_id categorias_criar; operation_id categorias_excluir; e mais 2 | - |
| chamadas | — | congelado | metrica poph_chamadas_evolution_total; metrica poph_chamadas_jev_total; metrica poph_chamadas_llm_total | - |
| chave | — | congelado | campo_json chave_idempotencia; coluna chave; coluna chave_idempotencia; e mais 1 | - |
| com | — | congelado | backup com_conversas; flag_cli --com-conversas | - |
| confianca | — | congelado | chave_toml handoff.orientacao_confianca | - |
| configuracao | — | congelado | protocolo_plugin configuracao | - |
| contato | — | congelado | chave_toml handoff.orientacao_ingestao_contato_produto | - |
| contem | — | congelado | linguagem de fluxos | - |
| conteudo | — | congelado | operation_id arquivos_conteudo; operation_id teste_chat_anexo_conteudo | - |
| conversa | — | congelado | flag_cli --conversa; indice ux_fichas_conversa | - |
| conversas | — | congelado | backup com_conversas; flag_cli --com-conversas; metrica poph_conversas_ativas; e mais 4 | - |
| criar | — | congelado | operation_id arquivos_criar; operation_id categorias_criar; operation_id criar; e mais 6 | - |
| dados | — | congelado | chave_toml dados; chave_toml dados.diretorio; chave_toml fluxos.max_dados_bytes; e mais 1 | - |
| de | — | congelado | flag_cli --de | - |
| decisao | — | congelado | chave_toml fluxos.max_saltos_decisao; linguagem de fluxos | - |
| definicao | — | congelado | coluna definicao_json | - |
| departamento | — | congelado | flag_cli --departamento | - |
| departamentos | — | congelado | operation_id departamentos_atualizar; operation_id departamentos_criar; operation_id departamentos_excluir; e mais 2 | - |
| desconectar | — | congelado | operation_id desconectar | - |
| descricao | — | congelado | coluna descricao | - |
| desde | — | congelado | flag_cli --desde | - |
| desfecho | — | congelado | coluna desfecho | - |
| desligar | — | congelado | comando_cli poph matrix desligar; comando_cli poph matrix espaco desligar; operation_id matrix_espacos_desligar | - |
| dias | — | congelado | chave_toml retencao.dias | - |
| dict | — | congelado | linguagem de fluxos | - |
| diferente | — | congelado | linguagem de fluxos | - |
| diretorio | — | congelado | chave_toml dados.diretorio | - |
| e | — | congelado | linguagem de fluxos | - |
| efeitos | — | congelado | indice ix_fluxo_efeitos_fila; indice ix_fluxo_efeitos_org; indice ux_fluxo_efeitos_chave; e mais 4 | - |
| em | — | congelado | backup exportado_em; coluna aberta_em; coluna concluiu_em; e mais 13 | - |
| encaminhar | — | congelado | linguagem de fluxos | - |
| endereco | — | congelado | chave_toml servidor.endereco; flag_cli --somente-endereco | - |
| entrou | — | congelado | coluna entrou_em | - |
| enumerate | — | congelado | linguagem de fluxos | - |
| enviado | — | congelado | coluna enviado_em | - |
| enviados | — | congelado | metrica poph_fluxos_efeitos_enviados_total | - |
| erro | — | congelado | coluna ultimo_erro | - |
| erros | — | congelado | metrica poph_erros_evolution_total; metrica poph_erros_jev_total; metrica poph_erros_llm_total; e mais 2 | - |
| escrever | — | congelado | flag_cli --escrever | - |
| espaco | — | congelado | comando_cli poph matrix espaco; comando_cli poph matrix espaco desligar; flag_cli --espaco | - |
| espacos | — | congelado | operation_id matrix_espacos_desligar; operation_id matrix_espacos_ligar; operation_id matrix_espacos_listar | - |
| espelhado | — | congelado | coluna client_name_espelhado | - |
| esperas | — | congelado | metrica poph_fluxos_cache_esperas_total | - |
| estado | — | congelado | coluna estado; comando_cli poph matrix estado; operation_id estado; e mais 1 | - |
| etapa | — | congelado | linguagem de fluxos; metrica poph_latencia_etapa_segundos | - |
| etapas | — | congelado | indice ux_fluxo_execucao_etapas_ordem; tabela fluxo_execucao_etapas | - |
| evento | — | congelado | indice ux_matrix_midia_evento; protocolo_plugin evento | - |
| eventos | — | congelado | indice ux_ficha_eventos_idem; indice ux_ficha_eventos_seq; operation_id fichas_eventos; e mais 1 | - |
| excluir | — | congelado | operation_id arquivos_excluir; operation_id categorias_excluir; operation_id departamentos_excluir; e mais 5 | - |
| execucao | — | congelado | coluna execucao_id; indice ux_fluxo_execucao_etapas_ordem; tabela fluxo_execucao_etapas | - |
| execucoes | — | congelado | indice ix_fluxo_execucoes_ficha; indice ix_fluxo_execucoes_revisao; indice ux_fluxo_execucoes_ativa; e mais 6 | - |
| exportar | — | congelado | operation_id backup_exportar | - |
| expulsoes | — | congelado | metrica poph_fluxos_expulsoes_total | - |
| externo | — | congelado | coluna peer_externo; protocolo_plugin id_externo | - |
| extrator | — | congelado | binario poph-extrator | - |
| falha | — | congelado | chave_toml turno.atraso_falha_modelo_ms; chave_toml turno.tentativas_falha_modelo; tabela matrix_avisos_falha | - |
| falhas | — | congelado | indice ix_matrix_falhas_org_time; indice ux_matrix_falhas_org_message; metrica poph_fluxos_efeitos_falhas_total; e mais 2 | - |
| fechada | — | congelado | coluna fechada_em; coluna fechada_motivo; coluna fechada_por | - |
| fechar | — | congelado | operation_id conversas_fechar; operation_id fichas_fechar | - |
| ficha | — | congelado | coluna ficha_id; comando_cli poph ficha; comando_cli poph ficha listar; e mais 11 | - |
| fichas | — | congelado | backup fichas; indice ix_fichas_org_agente; indice ix_fichas_org_status; e mais 17 | - |
| fila | — | congelado | indice ix_fluxo_efeitos_fila; metrica poph_fila_saida; metrica poph_fila_turnos | - |
| fim | — | congelado | coluna motivo_fim; linguagem de fluxos | - |
| finalizada | — | congelado | coluna finalizada_em | - |
| float | — | congelado | linguagem de fluxos | - |
| fluxo | — | congelado | campo_json fluxo_id; coluna fluxo_id; flag_cli --fluxo; e mais 18 | - |
| fluxos | — | congelado | backup fluxos; chave_toml fluxos; chave_toml fluxos.ativo; e mais 48 | - |
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
| instancia | — | congelado | campo_json instancia; protocolo_plugin instancia | - |
| int | — | congelado | linguagem de fluxos | - |
| integracao | — | congelado | protocolo_plugin integracao | - |
| intervalo | — | congelado | chave_toml turno.intervalo_despacho_ms | - |
| itens | — | congelado | operation_id listar_itens | - |
| ja | — | congelado | variavel_ambiente POPH_EVO_SEGREDO_LOJA | - |
| latencia | — | congelado | metrica poph_latencia_etapa_segundos | - |
| len | — | congelado | linguagem de fluxos | - |
| liberado | — | congelado | coluna liberado_em | - |
| liberar | — | congelado | comando_cli poph flow liberar | - |
| ligar | — | congelado | comando_cli poph matrix ligar; operation_id matrix_espacos_ligar | - |
| limiar | — | congelado | metrica poph_fluxos_g9_abaixo_do_limiar_total | - |
| limites | — | congelado | chave_toml limites; chave_toml limites.taxa_por_org_por_minuto; chave_toml limites.turnos_simultaneos; e mais 2 | - |
| list | — | congelado | linguagem de fluxos | - |
| listar | — | congelado | comando_cli poph ficha listar; comando_cli poph flow listar; operation_id arquivos_listar; e mais 16 | - |
| maior | — | congelado | linguagem de fluxos | - |
| manifesto | — | congelado | protocolo_plugin manifesto | - |
| max | — | congelado | linguagem de fluxos | - |
| memoria | — | congelado | metrica poph_memoria_processo_bytes | - |
| menor | — | congelado | linguagem de fluxos | - |
| mensagem | — | congelado | campo_json mensagem; chave_toml handoff.mensagem_automatica; operation_id teste_chat_mensagem | - |
| mensagem_ao_cliente | — | congelado | linguagem de fluxos | - |
| mensagens | — | congelado | operation_id mensagens_listar | - |
| metadados | — | congelado | protocolo_plugin metadados | - |
| midia | — | congelado | indice ix_matrix_midia_pendentes; indice ux_matrix_midia_evento; tabela matrix_midia | - |
| min | — | congelado | linguagem de fluxos | - |
| modelo | — | congelado | chave_toml jev.modelo; chave_toml llm.modelo; chave_toml turno.atraso_falha_modelo_ms; e mais 1 | - |
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
| pendentes | — | congelado | indice ix_matrix_midia_pendentes; metrica poph_fluxos_efeitos_pendentes_total | - |
| por | — | congelado | chave_toml fluxos.max_fluxos_por_org; chave_toml limites.taxa_por_org_por_minuto; chave_toml limites.turnos_simultaneos_por_org; e mais 2 | - |
| porta | — | congelado | chave_toml servidor.porta | - |
| poupadas | — | congelado | metrica poph_fluxos_cache_em_uso_poupadas_total | - |
| processo | — | congelado | metrica poph_memoria_processo_bytes | - |
| produto | — | congelado | chave_toml handoff.orientacao_ingestao_contato_produto | - |
| produtos | — | congelado | operation_id produtos_excluir; operation_id produtos_listar; operation_id produtos_mostrar | - |
| provedor | — | congelado | chave_toml llm.provedor | - |
| proxima | — | congelado | coluna proxima_tentativa; indice ix_inbox_proxima_tentativa | - |
| publicada | — | congelado | coluna publicada_em; coluna publicada_por | - |
| publicar | — | congelado | comando_cli poph flow publicar; operation_id fluxos_publicar | - |
| que | — | congelado | coluna atores_que_agiram_json | - |
| range | — | congelado | linguagem de fluxos | - |
| rascunho | — | congelado | coluna rascunho_fonte | - |
| reabrir | — | congelado | operation_id conversas_reabrir; operation_id teste_chat_reabrir | - |
| recarregar | — | congelado | operation_id recarregar_cluster | - |
| registro | — | congelado | comando_cli poph matrix registro | - |
| regra | — | congelado | chave_toml fluxos.max_heap_regra; chave_toml fluxos.max_ticks_regra; metrica poph_fluxos_erros_regra_total | - |
| reindexar | — | congelado | operation_id reindexar | - |
| remetente | — | congelado | protocolo_plugin remetente | - |
| remoto | — | congelado | flag_cli --remoto | - |
| resolucoes | — | congelado | coluna resolucoes_json | - |
| resultado | — | congelado | coluna resultado | - |
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
| substituir | — | congelado | operation_id substituir | - |
| tabelas | — | congelado | backup tabelas | - |
| temperatura | — | congelado | chave_toml llm.temperatura | - |
| tentativa | — | congelado | coluna proxima_tentativa; indice ix_inbox_proxima_tentativa | - |
| tentativas | — | congelado | chave_toml turno.tentativas_falha_modelo; coluna tentativas | - |
| teste | — | congelado | operation_id teste_chat_anexo_conteudo; operation_id teste_chat_criar; operation_id teste_chat_historico; e mais 3 | - |
| texto | — | congelado | protocolo_plugin max_texto_bytes; protocolo_plugin texto | - |
| tipo | — | congelado | coluna ator_tipo; coluna tipo; protocolo_plugin tipo | - |
| tipos | — | congelado | protocolo_plugin tipos_aceitos | - |
| tuple | — | congelado | linguagem de fluxos | - |
| turno | — | congelado | chave_toml turno; chave_toml turno.atraso_falha_modelo_ms; chave_toml turno.intervalo_despacho_ms; e mais 2 | - |
| turnos | — | congelado | chave_toml limites.turnos_simultaneos; chave_toml limites.turnos_simultaneos_por_org; metrica poph_fila_turnos; e mais 1 | - |
| ultimo | — | congelado | coluna ultimo_erro | - |
| uso | — | congelado | metrica poph_fluxos_cache_em_uso_poupadas_total; metrica poph_fluxos_revisoes_em_uso | - |
| validar | — | congelado | comando_cli poph flow validar; operation_id fluxos_validar | - |
| vazio | — | congelado | linguagem de fluxos | - |
| verificar | — | congelado | flag_cli --verificar | - |
| versao | — | congelado | backup versao_formato; coluna schema_versao; protocolo_plugin versao | - |
| vincular | — | congelado | operation_id vincular | - |
| whatsapp | — | congelado | rota /admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/disconnect; rota /admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/qrcode; rota /admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/state | - |
| zip | — | congelado | linguagem de fluxos | - |
<!-- glossario:tabela:fim -->
