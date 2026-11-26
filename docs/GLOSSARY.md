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
| cenario | scenario | traduz | poph-agent, poph-server | 5130 |
| texto | text | traduz | poph-agent, poph-server | 3866 |
| erro | error | traduz | poph-server, poph | 3495 |
| estado | state | traduz | poph-server, poph-fluxos | 3123 |
| agente | agent | traduz | poph-server, poph-agent | 2899 |
| conversa | conversation | traduz | poph-server, poph-agent | 2783 |
| nome | name | traduz | poph-server, poph-fluxos | 2160 |
| novo | new | traduz | poph-server, poph-agent | 1813 |
| de | of | traduz | poph-server, poph-fluxos | 1715 |
| corpo | body | traduz | poph-server, poph-matrix | 1627 |
| fluxo | flow | traduz | poph-agent, poph-server | 1615 |
| com | with | traduz | poph-server, poph-agent | 1604 |
| sala | room | traduz | poph-server, poph-matrix | 1538 |
| pasta | folder | traduz | poph, poph-server | 1508 |
| resposta | response | traduz | poph-server, poph-agent | 1500 |
| da | of | traduz | poph-server, poph-matrix | 1377 |
| motor | engine | traduz | poph-agent, poph-server | 1358 |
| resultado | result | traduz | poph-store, poph-fluxos | 1259 |
| mensagem | message | traduz | poph-agent, poph-server | 1256 |
| saida | output | traduz | poph, poph-server | 1253 |
| turno | turn | traduz | poph-agent, poph-server | 1201 |
| valor | value | traduz | poph-server, poph-fluxos | 1197 |
| cliente | client | traduz | poph-server, poph-matrix | 1189 |
| tipo | kind | traduz | poph-fluxos, poph-server | 1171 |
| caminho | path | traduz | poph-server, poph | 1141 |
| motivo | reason | traduz | poph-server, poph-matrix | 1118 |
| evento | event | traduz | console, poph-server | 1113 |
| dados | data | traduz | poph-matrix, poph-server | 1078 |
| chave | key | traduz | poph-server, poph-fluxos | 1071 |
| metricas | metrics | traduz | poph-server, poph-agent | 1040 |
| acao | action | traduz | poph-fluxos, poph | 1037 |
| campo | field | traduz | poph-fluxos, console | 1027 |
| sem | without | traduz | poph-server, poph-agent | 963 |
| registro | record | traduz | poph-agent, poph-server | 888 |
| codigo | code | traduz | poph-fluxos, poph | 879 |
| etapa | step | traduz | poph-fluxos, poph-server | 819 |
| falha | failure | traduz | poph-server, poph | 795 |
| departamento | department | traduz | poph-server, poph | 790 |
| arquivo | file | traduz | poph, poph-server | 786 |
| resumo | summary | traduz | poph-agent, poph-server | 741 |
| servidor | server | traduz | console, poph-server | 728 |
| opcoes | options | traduz | poph-agent, poph-server | 722 |
| destino | target | traduz | poph-server, poph-fluxos | 717 |
| criar | create | traduz | poph-server, poph | 698 |
| nao | not | traduz | poph-server, poph-fluxos | 694 |
| em | in | traduz | poph-server, poph-fluxos | 661 |
| para | for | traduz | poph-fluxos, poph-server | 658 |
| decisao | decision | traduz | poph-fluxos, poph-jev | 599 |
| espaco | space | traduz | poph-matrix, poph-server | 589 |
| ficha | case | traduz | poph-server, poph-agent | 582 |
| falso | false | traduz | poph-server, poph-agent | 573 |
| alvo | target | traduz | poph-server, poph-fluxos | 572 |
| pedido | request | traduz | poph-agent, poph-server | 550 |
| regra | rule | traduz | poph-fluxos, poph-server | 545 |
| itens | items | traduz | poph-server, poph-fluxos | 541 |
| linha | line | traduz | poph-server, poph-fluxos | 533 |
| pedir | ask | traduz | poph-server, poph | 527 |
| midia | media | traduz | poph-matrix, poph-server | 525 |
| chamada | call | traduz | poph-server, poph-matrix | 520 |
| produto | product | traduz | poph-agent, poph | 516 |
| relatorio | report | traduz | poph-server, poph | 511 |
| gatilho | trigger | traduz | poph-agent, poph-fluxos | 509 |
| fonte | source | traduz | poph-fluxos, poph-server | 496 |
| versao | version | traduz | poph-server, console | 493 |
| indice | index | traduz | poph-rag, poph-agent | 489 |
| canal | channel | traduz | poph-server, poph-agent | 483 |
| nova | new | traduz | poph-fluxos, poph-agent | 466 |
| porta | port | traduz | poph-server, poph-matrix | 465 |
| modelo | model | traduz | poph-agent, poph-server | 464 |
| ao | to | traduz | poph-fluxos, poph-server | 458 |
| lista | list | traduz | poph-fluxos, poph-server | 456 |
| inicio | beginning | traduz | poph-server, poph-agent | 450 |
| entrada | entry | traduz | poph-agent, poph-server | 444 |
| provedor | provider | traduz | poph-agent, poph-server | 443 |
| fim | end | traduz | poph-fluxos, poph-agent | 439 |
| atual | current | traduz | poph-server, poph-agent | 424 |
| ambiente | environment | traduz | poph, poph-matrix | 418 |
| limites | limits | traduz | poph-fluxos, poph-agent | 403 |
| contexto | context | traduz | poph-fluxos, poph-agent | 400 |
| integracao | integration | traduz | poph-server, console | 396 |
| caso | case | traduz | poph-server, poph-fluxos | 391 |
| aviso | notice | traduz | poph-server, poph | 390 |
| grupo | group | traduz | poph, poph-server | 387 |
| ator | actor | traduz | poph-fluxos, poph-store | 381 |
| fluxos | flows | traduz | poph-server, poph-agent | 372 |
| quente | hot | traduz | poph-agent, poph-server | 368 |
| execucao | execution | traduz | poph-server, poph-agent | 362 |
| instancia | instance | traduz | poph-server, poph-evolution | 349 |
| efeito | effect | traduz | poph-fluxos, poph-server | 342 |
| rotulo | label | traduz | poph-server, poph-fluxos | 339 |
| revisao | revision | traduz | poph-server, poph-agent | 338 |
| campos | fields | traduz | poph-fluxos, poph-agent | 336 |
| atores | actors | traduz | poph-agent, poph-fluxos | 333 |
| configuracao | configuration | traduz | poph, console | 327 |
| desligar | disable | traduz | poph-server, poph | 327 |
| origem | origin | traduz | poph, poph-matrix | 324 |
| situacao | situation | traduz | console, poph-retention | 316 |
| mensagens | messages | traduz | poph-agent, poph-server | 311 |
| por | by | traduz | poph-agent, poph | 311 |
| envio | send | traduz | poph-server, poph-agent | 307 |
| avaliador | evaluator | traduz | poph-fluxos, poph-agent | 304 |
| produtos | products | traduz | poph-agent, poph-server | 301 |
| ferramenta | tool | traduz | poph-agent, poph | 299 |
| categoria | category | traduz | poph, poph-server | 297 |
| escolha | choice | traduz | poph-agent, poph-jev | 296 |
| salas | rooms | traduz | poph-server, poph-matrix | 292 |
| teto | cap | traduz | poph-server, poph-fluxos | 292 |
| guarda | guard | traduz | poph-matrix, poph-server | 287 |
| efeitos | effects | traduz | poph-fluxos, poph-server | 284 |
| padrao | default | traduz | poph-agent, poph-server | 284 |
| whatsapp | whatsapp | traduz | console, poph-server | 278 |
| executar | run | traduz | poph-fluxos, poph-agent | 276 |
| relato | report | traduz | poph-server, poph-agent | 276 |
| numero | number | traduz | console, poph-store | 275 |
| vazio | empty | traduz | poph-server, poph-fluxos | 275 |
| conteudo | content | traduz | poph-matrix, poph-server | 271 |
| metodo | method | traduz | poph-server, poph-matrix | 270 |
| chamadas | calls | traduz | poph-agent, poph-server | 267 |
| outro | other | traduz | poph-server, poph | 260 |
| pausa | pause | traduz | poph-agent, poph-server | 260 |
| identidade | identity | traduz | poph-server, poph-rag | 259 |
| rota | route | traduz | poph-server, poph-fluxos | 258 |
| consulta | query | traduz | poph-server, poph-matrix | 257 |
| antes | before | traduz | poph-server, poph-matrix | 251 |
| mapa | map | traduz | poph-server, poph-agent | 250 |
| mudanca | change | traduz | poph-fluxos, poph | 244 |
| pagina | page | traduz | poph-server, poph | 241 |
| eventos | events | traduz | poph-server, poph-fluxos | 239 |
| segredo | secret | traduz | poph-server, poph | 236 |
| falhas | failures | traduz | poph-server, poph-agent | 235 |
| na | towards | traduz | poph-server, poph-matrix | 230 |
| espacos | spaces | traduz | poph-server, poph-matrix | 229 |
| fantasma | ghost | traduz | poph-matrix, poph-server | 228 |
| gravada | stored | traduz | poph-server, poph-agent | 227 |
| responder | reply | traduz | poph-server, poph-matrix | 227 |
| segredos | secrets | traduz | poph, console | 227 |
| gravado | stored | traduz | poph-server, poph-agent | 223 |
| validar | validate | traduz | poph-server, poph-types | 223 |
| primeira | first | traduz | poph-server, poph-agent | 222 |
| pedidos | requests | traduz | poph-agent, poph-jev | 216 |
| nomes | names | traduz | poph-fluxos, poph-agent | 215 |
| ciclo | cycle | traduz | poph-server, poph-fluxos | 214 |
| modulo | module | traduz | poph-server, poph-fluxos | 214 |
| detalhe | detail | traduz | poph, poph-server | 213 |
| enviar | send | traduz | poph-server, poph-matrix | 213 |
| interno | internal | traduz | poph-agent, poph-server | 211 |
| invalido | invalid | traduz | poph-server, poph-fluxos | 211 |
| bloco | block | traduz | poph-agent, poph-fluxos | 209 |
| limite | limit | traduz | poph-server, poph-fluxos | 208 |
| depois | after | traduz | poph-server, console | 205 |
| memoria | memory | traduz | poph-agent, poph-server | 203 |
| esperar | wait | traduz | poph-server, poph-agent | 200 |
| espera | wait | traduz | poph-agent, poph-server | 199 |
| iniciar | start | traduz | poph-fluxos, poph-server | 198 |
| falar | speak | traduz | poph-agent, poph-server | 197 |
| agentes | agents | traduz | poph, poph-server | 196 |
| que | that | traduz | poph-fluxos, poph-server | 192 |
| catalogo | catalog | traduz | poph-agent, poph | 191 |
| externo | external | traduz | poph-server, poph-agent | 191 |
| abrir | open | traduz | poph, poph-server | 190 |
| anexo | attachment | traduz | poph-agent, poph-server | 188 |
| descricao | description | traduz | console, poph-agent | 188 |
| registrar | register | traduz | poph-server, poph-agent | 188 |
| ferramentas | tools | traduz | poph-agent, poph-server | 186 |
| cru | raw | traduz | poph-server, poph-agent | 184 |
| respostas | responses | traduz | poph-agent, poph-server | 182 |
| textos | texts | traduz | poph-rag, poph-server | 182 |
| primeiro | first | traduz | poph-server, poph-agent | 181 |
| modo | mode | traduz | poph-server, console | 180 |
| cabecalhos | headers | traduz | poph-server, poph-matrix | 179 |
| partes | parts | traduz | poph-server, poph-agent | 178 |
| esperado | expected | traduz | poph, poph-server | 177 |
| remetente | sender | traduz | poph-matrix, poph-server | 176 |
| subir | raise | traduz | poph-server, poph-matrix | 176 |
| pendentes | pending | traduz | poph-agent, poph-server | 175 |
| conversas | conversations | traduz | poph-server, poph-store | 174 |
| limitador | limiter | traduz | poph-agent, poph-server | 174 |
| registros | records | traduz | poph-server, poph-matrix | 172 |
| falsa | false | traduz | poph-server, poph-agent | 169 |
| fila | queue | traduz | poph-server, poph-agent | 169 |
| lote | batch | traduz | poph-jev, poph-server | 167 |
| outra | other | traduz | poph-server, poph-agent | 167 |
| busca | search | traduz | poph-agent, poph-rag | 164 |
| impacto | impact | traduz | poph-fluxos | 164 |
| turnos | turns | traduz | poph-agent, poph-server | 164 |
| desfecho | outcome | traduz | poph-agent, poph-server | 161 |
| proprio | own | traduz | poph-agent, poph-server | 161 |
| fantasmas | ghosts | traduz | poph-matrix, poph-server | 160 |
| posicao | position | traduz | poph-matrix, poph-server | 160 |
| arquivos | files | traduz | poph-server, poph-agent | 157 |
| erros | errors | traduz | poph-fluxos, poph-server | 157 |
| transacao | transaction | traduz | poph-server, poph-matrix | 156 |
| segunda | second | traduz | poph-server, poph-agent | 155 |
| endereco | address | traduz | poph-server, poph-matrix | 153 |
| ordem | order | traduz | poph-server, console | 153 |
| transporte | transport | traduz | poph-agent, poph-jev | 153 |
| roteiro | script | traduz | poph-agent, poph-server | 152 |
| agora | now | traduz | poph-server, poph-agent | 150 |
| listar | list | traduz | poph, poph-server | 150 |
| teste | test | traduz | poph, poph-server | 150 |
| carregar | load | traduz | poph, poph-fluxos | 148 |
| instrucoes | instructions | traduz | poph, poph-agent | 148 |
| ultimo | last | traduz | poph-server, poph-agent | 148 |
| recusa | refusal | traduz | poph-server, poph | 147 |
| usuarios | users | traduz | poph-matrix, poph-server | 147 |
| mudou | changed | traduz | poph-fluxos, poph-server | 146 |
| futuro | future | traduz | poph-agent, poph-server | 144 |
| vai | goes | traduz | poph-fluxos, raiz | 144 |
| entrega | delivery | traduz | poph-server, poph-agent | 143 |
| etapas | steps | traduz | poph-fluxos, poph-server | 143 |
| avisos | notices | traduz | poph-server, poph-fluxos | 141 |
| espelho | mirror | traduz | poph-server, poph-matrix | 141 |
| escolhas | choices | traduz | poph-agent, poph-jev | 137 |
| ler | read | traduz | poph, poph-matrix | 137 |
| trechos | passages | traduz | poph-agent, poph-rag | 137 |
| trecho | passage | traduz | poph-agent, poph-fluxos | 136 |
| buscar | search | traduz | poph-agent, poph-rag | 133 |
| corte | cutoff | traduz | poph-agent, poph-matrix | 133 |
| diagnosticos | diagnostics | traduz | poph-fluxos, poph-server | 130 |
| relogio | clock | traduz | poph-server, poph-agent | 129 |
| tamanho | size | traduz | poph-agent, poph-matrix | 129 |
| compilado | compiled | traduz | poph-fluxos, poph-server | 128 |
| manifesto | manifest | traduz | poph-server, console | 128 |
| observador | observer | traduz | poph-agent, poph-server | 127 |
| tentativas | attempts | traduz | poph-server, poph-agent | 127 |
| confere | checks | traduz | poph-fluxos, poph-server | 126 |
| tarefa | task | traduz | poph-agent, poph-server | 126 |
| atendimento | service | traduz | poph-fluxos, poph-server | 124 |
| diagnostico | diagnostic | traduz | poph-fluxos, poph-server | 124 |
| sinal | signal | traduz | poph-server, poph-agent | 124 |
| casos | cases | traduz | poph-jev, poph-fluxos | 123 |
| decisoes | decisions | traduz | poph-fluxos, poph-jev | 123 |
| leitor | reader | traduz | poph-fluxos, poph | 123 |
| uma | a | traduz | poph-server, poph-fluxos | 122 |
| ajustes | adjustments | traduz | poph-agent, poph-server | 121 |
| usuario | user | traduz | poph-matrix, poph-server | 121 |
| envios | sends | traduz | poph-server, poph-agent | 120 |
| uso | use | traduz | poph-agent, poph-fluxos | 120 |
| classificacao | classification | traduz | poph-matrix, poph-server | 119 |
| auditoria | audit | traduz | poph-server, poph-retention | 118 |
| banco | database | traduz | poph-retention, poph | 118 |
| avaliar | evaluate | traduz | poph-fluxos, poph-jev | 117 |
| prefixo | prefix | traduz | poph-server, poph-matrix | 117 |
| fora | outside | traduz | poph-server, poph-fluxos | 116 |
| orientacao | guidance | traduz | poph-jev, poph-agent | 116 |
| sessao | session | traduz | poph, poph-rag | 115 |
| variavel | variable | traduz | poph-fluxos, poph | 115 |
| acoes | actions | traduz | poph-fluxos, raiz | 114 |
| exigir | require | traduz | poph-server, poph | 111 |
| inexistente | missing | traduz | poph-fluxos, poph-server | 111 |
| janela | window | traduz | poph-agent, poph-server | 111 |
| lido | read | traduz | poph, poph-server | 111 |
| confianca | confidence | traduz | poph-jev, poph-agent | 110 |
| grande | large | traduz | poph-server, poph-matrix | 110 |
| argumentos | arguments | traduz | poph-agent, console | 109 |
| das | of | traduz | poph-server, poph-agent | 109 |
| membros | members | traduz | poph-server, poph-matrix | 109 |
| anexos | attachments | traduz | poph-server, poph-agent | 108 |
| ignorar | ignore | traduz | poph-matrix, poph-server | 107 |
| nivel | level | traduz | poph, poph-matrix | 107 |
| quando | when | traduz | poph-fluxos, poph-server | 107 |
| duracao | duration | traduz | poph-agent, poph-server | 106 |
| imprimir | print | traduz | poph, poph-fluxos | 106 |
| atualizar | update | traduz | poph-server, poph | 105 |
| fechada | closed | traduz | poph-store, poph-server | 103 |
| captura | capture | traduz | poph-server, poph | 102 |
| conexao | connection | traduz | poph-server, poph | 102 |
| par | pair | traduz | poph-server, poph | 102 |
| segundo | second | traduz | poph-agent, poph-server | 101 |
| atendente | attendant | traduz | poph-server, poph-matrix | 100 |
| criado | created | traduz | poph-server, poph | 100 |
| invalida | invalid | traduz | poph-fluxos, poph-server | 100 |
| linhas | lines | traduz | poph, poph-server | 100 |
| contadores | counters | traduz | poph-server, poph-fluxos | 99 |
| descrever | describe | traduz | poph, poph-agent | 99 |
| escritas | writings | traduz | poph-server, poph-matrix | 99 |
| recusado | refused | traduz | poph-server, poph-agent | 99 |
| como | as | traduz | poph-fluxos, poph | 98 |
| definir | define | traduz | poph-server, poph-agent | 98 |
| atraso | delay | traduz | poph-server, poph-agent | 97 |
| encerra | closes | traduz | poph-fluxos, poph-agent | 97 |
| prazo | deadline | traduz | poph-fluxos, poph-server | 97 |
| aparelho | device | traduz | poph-server, poph-matrix | 96 |
| mostrar | show | traduz | poph-server, poph | 96 |
| receptor | receiver | traduz | poph-server, poph-agent | 96 |
| parametros | parameters | traduz | poph-agent, poph-server | 95 |
| pergunta | question | traduz | poph-jev, poph-agent | 95 |
| bruto | raw | traduz | poph-fluxos, poph-server | 94 |
| ligado | linked | traduz | poph-server, poph-agent | 94 |
| carga | load | traduz | poph-fluxos, poph-agent | 93 |
| nenhum | none | traduz | poph-fluxos, poph-server | 93 |
| tempos | times | traduz | poph-agent, poph-fluxos | 92 |
| comando | command | traduz | poph, poph-agent | 91 |
| sincronizar | synchronise | traduz | poph-agent, poph-server | 91 |
| instrucao | instruction | traduz | poph-fluxos, raiz | 90 |
| ja | already | traduz | poph-server, poph-matrix | 90 |
| formatar | format | traduz | poph-agent, poph-fluxos | 89 |
| formato | format | traduz | poph-matrix, poph-store | 88 |
| desligado | disabled | traduz | poph-server, poph | 87 |
| gravar | write | traduz | poph-server, poph-retention | 87 |
| buscador | searcher | traduz | poph-agent, poph | 86 |
| limpo | clean | traduz | poph-server, poph-matrix | 86 |
| resto | rest | traduz | poph-server, poph | 86 |
| exige | requires | traduz | poph-fluxos, raiz | 85 |
| ligar | link | traduz | poph-server, poph | 85 |
| ou | or | traduz | poph-fluxos, poph-agent | 85 |
| pronta | ready | traduz | poph-matrix, poph-server | 85 |
| titulo | title | traduz | console, poph-rag | 85 |
| vetor | vector | traduz | poph-rag | 85 |
| mundo | world | traduz | poph-server | 84 |
| montar | build | traduz | poph-agent, poph-server | 83 |
| obrigatorio | required | traduz | poph-fluxos, poph-matrix | 83 |
| saidas | outputs | traduz | poph-agent, poph-server | 83 |
| tem | has | traduz | poph, poph-server | 83 |
| aberta | open | traduz | poph-server, poph-fluxos | 82 |
| servir | serve | traduz | poph-server, poph | 82 |
| validacao | validation | traduz | poph-server, poph | 82 |
| veneno | poison | traduz | poph-server, poph-agent | 82 |
| vivo | live | traduz | poph, poph-server | 82 |
| conhecimento | knowledge | traduz | poph-server, poph-agent | 81 |
| entradas | entries | traduz | poph-agent, poph-fluxos | 81 |
| funcao | function | traduz | poph-fluxos, raiz | 80 |
| dias | days | traduz | poph-retention, console | 79 |
| documento | document | traduz | poph-server, poph | 79 |
| enviados | sent | traduz | poph-server, poph-agent | 79 |
| onde | where | traduz | poph-fluxos, poph | 79 |
| perguntas | questions | traduz | poph-jev, poph-agent | 79 |
| muda | changes | traduz | poph-fluxos, poph-agent | 78 |
| preparo | preparation | traduz | poph-agent, poph | 78 |
| papel | role | traduz | poph-matrix, poph-server | 77 |
| parar | stop | traduz | poph-server, poph-agent | 77 |
| canais | channels | traduz | poph-server, poph | 76 |
| falhar | fail | traduz | poph-server, poph-matrix | 76 |
| globais | globals | traduz | poph-server, poph-jev | 76 |
| idempotencia | idempotency | traduz | poph-store, poph-server | 76 |
| midias | media | traduz | poph-server | 76 |
| processador | processor | traduz | poph-agent, poph-server | 76 |
| apos | after | traduz | poph-server, poph-agent | 75 |
| ate | until | traduz | poph-server, poph-agent | 75 |
| autorizacao | authorization | traduz | poph-matrix, poph-server | 75 |
| instancias | instances | traduz | poph-server, console | 75 |
| preco | price | traduz | poph-agent, console | 75 |
| argumento | argument | traduz | poph-fluxos, poph-agent | 74 |
| ausente | missing | traduz | poph-fluxos, poph-server | 74 |
| expiracao | expiry | traduz | poph-server, poph-matrix | 74 |
| igual | equal | traduz | poph-fluxos, poph-server | 74 |
| operacao | operation | traduz | poph-matrix, poph-server | 74 |
| maior | bigger | traduz | poph-fluxos, poph-server | 73 |
| nulo | null | traduz | poph-agent, poph-server | 73 |
| bruta | raw | traduz | poph-fluxos, poph-server | 72 |
| chamar | call | traduz | poph-agent, poph-server | 72 |
| diretorio | directory | traduz | poph-server, scripts | 72 |
| enfileirar | enqueue | traduz | poph-matrix, poph-agent | 72 |
| entrar | enter | traduz | poph-fluxos, poph-matrix | 72 |
| montagem | assembly | traduz | poph-agent, poph | 72 |
| achado | finding | traduz | poph-rag, poph-agent | 71 |
| enviado | sent | traduz | poph-server, poph-store | 71 |
| revisoes | revisions | traduz | poph-server, poph-fluxos | 71 |
| candidato | candidate | traduz | poph-agent, poph-jev | 70 |
| canonico | canonical | traduz | poph-fluxos, poph-jev | 70 |
| cifra | cipher | traduz | poph-server, poph | 70 |
| historico | history | traduz | poph-server, poph-agent | 70 |
| repetida | repeated | traduz | poph-server, poph-fluxos | 70 |
| departamentos | departments | traduz | poph-server, poph-fluxos | 69 |
| fichas | cases | traduz | poph-server, poph-store | 69 |
| conjunto | set | traduz | poph-jev, poph | 68 |
| excluir | delete | traduz | poph, poph-server | 68 |
| quem | who | traduz | poph-server, poph-agent | 68 |
| compilar | compile | traduz | poph-server, poph-fluxos | 67 |
| quantidade | quantity | traduz | poph-agent, poph-server | 67 |
| escopo | scope | traduz | poph-fluxos, console | 66 |
| pendente | pending | traduz | poph-server, poph-agent | 66 |
| tabela | table | traduz | poph, poph-agent | 66 |
| transitoria | transient | traduz | poph-server, poph-matrix | 66 |
| cabecalho | header | traduz | poph-server, poph-agent | 65 |
| entregar | deliver | traduz | poph-server, poph-agent | 65 |
| provisionador | provisioner | traduz | poph-matrix, poph-server | 65 |
| recebido | received | traduz | poph-server, poph | 65 |
| unica | single | traduz | poph-fluxos, poph-server | 65 |
| rodada | round | traduz | poph-agent, poph-server | 64 |
| ativo | active | traduz | poph-agent, console | 63 |
| baixar | download | traduz | poph-server, poph-matrix | 63 |
| candidatos | candidates | traduz | poph-agent, poph-jev | 63 |
| diferenca | difference | traduz | poph-fluxos, poph-server | 63 |
| nada | nothing | traduz | poph-agent, poph-server | 63 |
| achados | findings | traduz | poph-agent, poph-rag | 62 |
| antigo | old | traduz | poph-matrix, poph-fluxos | 62 |
| carimbo | stamp | traduz | poph-server, poph-fluxos | 62 |
| contar | count | traduz | poph-server, poph-retention | 62 |
| categorias | categories | traduz | poph, poph-server | 61 |
| criacao | creation | traduz | poph-matrix, poph-server | 61 |
| garantir | ensure | traduz | poph-matrix, poph-server | 61 |
| imagem | image | traduz | poph-matrix, poph-agent | 61 |
| metadados | metadata | traduz | poph-server, poph-files | 61 |
| pelo | by | traduz | poph-server, poph-matrix | 61 |
| vira | becomes | traduz | poph-server, poph-matrix | 61 |
| atender | attend | traduz | poph-server, poph-matrix | 60 |
| evolucao | evolution | traduz | poph-server | 60 |
| marcador | marker | traduz | poph-server, poph-agent | 60 |
| referencia | reference | traduz | poph, poph-rag | 60 |
| resolve | resolves | traduz | scripts, poph-fluxos | 60 |
| testes | tests | traduz | poph-server, poph | 60 |
| resolucoes | resolutions | traduz | poph-fluxos, poph-server | 59 |
| vaga | slot | traduz | poph-agent | 58 |
| escrita | writing | traduz | poph-server, poph-matrix | 57 |
| proximo | next | traduz | poph-server, poph-agent | 57 |
| publicada | published | traduz | poph-server, poph-matrix | 57 |
| ouvinte | listener | traduz | poph-matrix, poph-server | 56 |
| passo | step | traduz | poph-server, poph-fluxos | 56 |
| perfil | profile | traduz | poph-matrix, poph-server | 56 |
| proxima | next | traduz | poph-server, poph-matrix | 56 |
| roteiros | scripts | traduz | poph-agent, poph-server | 56 |
| contem | contains | traduz | poph-fluxos, poph-server | 55 |
| dois | two | traduz | poph-fluxos, poph-server | 55 |
| acertos | hits | traduz | poph-rag, poph-jev | 54 |
| definitiva | definitive | traduz | poph-matrix, poph-server | 54 |
| escrever | write | traduz | poph, poph-server | 54 |
| exemplos | examples | traduz | poph-fluxos, poph-server | 54 |
| pedaco | piece | traduz | poph-matrix, poph-server | 54 |
| risco | risk | traduz | poph-jev, poph-agent | 54 |
| coluna | column | traduz | poph-fluxos, poph | 53 |
| comparar | compare | traduz | poph-server, poph | 53 |
| execucoes | executions | traduz | poph-server, poph-store | 53 |
| posicionar | position | traduz | poph-matrix, poph-server | 53 |
| publicar | publish | traduz | poph-server, poph | 53 |
| retrato | snapshot | traduz | poph, poph-server | 53 |
| ultimas | last | traduz | poph-agent | 53 |
| ativa | active | traduz | poph-agent, poph-server | 52 |
| convidados | guests | traduz | poph-matrix, poph-server | 52 |
| estruturado | structured | traduz | poph-agent, poph-server | 52 |
| falhou | failed | traduz | poph-server, poph-agent | 52 |
| assinatura | signature | traduz | poph-server | 51 |
| contador | counter | traduz | poph-rag, poph-server | 51 |
| detalhes | details | traduz | poph-server, poph-jev | 51 |
| esperada | expected | traduz | poph-server, poph | 51 |
| existente | existing | traduz | poph-matrix, poph | 51 |
| trava | lock | traduz | poph-rag, poph-fluxos | 51 |
| vistos | seen | traduz | poph-fluxos, poph-server | 51 |
| anterior | previous | traduz | poph-server, poph-fluxos | 50 |
| antiga | old | traduz | poph-fluxos, poph-server | 50 |
| cortado | cut | traduz | poph-fluxos, poph-matrix | 50 |
| forma | form | traduz | poph-fluxos, poph | 50 |
| laco | loop | traduz | poph-server, poph-agent | 50 |
| maximo | maximum | traduz | poph-agent, poph-jev | 50 |
| orcamento | budget | traduz | poph-agent, poph-rag | 50 |
| rodando | running | traduz | poph-server, poph-retention | 50 |
| tentativa | attempt | traduz | poph-agent, poph-server | 50 |
| classificar | classify | traduz | poph-matrix, poph-server | 49 |
| convite | invitation | traduz | poph-matrix, poph-server | 49 |
| cursores | cursors | traduz | poph-server, poph | 49 |
| despachar | dispatch | traduz | poph-agent, poph-server | 49 |
| existe | exists | traduz | poph-server, poph-matrix | 49 |
| limiares | thresholds | traduz | poph-jev | 49 |
| segundos | seconds | traduz | poph-server, poph-rag | 49 |
| transicao | transition | traduz | poph-server, poph-agent | 49 |
| avancar | advance | traduz | poph-agent, poph-server | 48 |
| cena | scene | traduz | poph-server | 48 |
| errado | wrong | traduz | poph-server, poph-fluxos | 48 |
| poder | power | traduz | poph-matrix | 48 |
| raiz | root | traduz | poph-server, poph-jev | 48 |
| decidir | decide | traduz | poph-matrix, poph-server | 47 |
| entre | between | traduz | poph-fluxos, poph-server | 47 |
| rascunho | draft | traduz | poph-server, poph-store | 47 |
| recusada | refused | traduz | poph-agent, poph | 47 |
| tipos | kinds | traduz | poph-fluxos, poph-server | 47 |
| encontrado | found | traduz | poph-agent, poph | 46 |
| nos | us | traduz | poph-fluxos, poph-server | 46 |
| remoto | remote | traduz | poph, poph-server | 46 |
| ultima | last | traduz | poph-server, poph-agent | 46 |
| aguardando | waiting | traduz | poph-fluxos, poph-agent | 45 |
| biblioteca | library | traduz | poph-server, poph-agent | 45 |
| conferir | check | traduz | poph-server, poph | 45 |
| filho | child | traduz | poph-matrix, poph-fluxos | 45 |
| lida | read | traduz | poph-server, poph-fluxos | 45 |
| mostra | shows | traduz | poph, poph-fluxos | 45 |
| novos | new | traduz | poph-fluxos, poph-rag | 45 |
| parte | part | traduz | poph-agent, poph-server | 45 |
| vez | time | traduz | poph-server, poph-fluxos | 45 |
| decodificar | decode | traduz | poph-server, poph-matrix | 44 |
| dia | day | traduz | poph-matrix, poph-server | 44 |
| redacao | writing | traduz | poph-matrix, poph-server | 44 |
| removida | removed | traduz | poph-fluxos, poph-matrix | 44 |
| sentido | meaning | traduz | poph-matrix, poph-server | 44 |
| tabelas | tables | traduz | console, poph | 44 |
| tempo | time | traduz | poph-agent, poph-jev | 44 |
| usado | used | traduz | poph-fluxos, poph-agent | 44 |
| ajuste | adjustment | traduz | poph-jev, poph-agent | 43 |
| carrinho | cart | traduz | poph-agent, poph-server | 43 |
| espelhado | mirrored | traduz | poph-server, poph-matrix | 43 |
| gravados | stored | traduz | poph-server, poph-plugin-eco | 43 |
| liberar | release | traduz | poph-agent, poph | 43 |
| locais | local | traduz | poph-fluxos | 43 |
| longo | long | traduz | poph-rag, poph-fluxos | 43 |
| similaridade | similarity | traduz | poph-rag, poph-agent | 43 |
| coleta | collection | traduz | poph-fluxos | 42 |
| confiavel | reliable | traduz | poph-jev, poph-agent | 42 |
| criados | created | traduz | poph-server, poph-agent | 42 |
| intervalo | interval | traduz | poph-server, poph-agent | 42 |
| leitura | read | traduz | poph-agent, poph-server | 42 |
| mesmo | same | traduz | poph-fluxos, poph-server | 42 |
| passar | pass | traduz | poph-server, poph-agent | 42 |
| repetido | repeated | traduz | poph-fluxos, poph | 42 |
| valores | values | traduz | poph-server, poph-fluxos | 42 |
| vazia | empty | traduz | poph-agent, poph-matrix | 42 |
| criada | created | traduz | poph-server, poph | 41 |
| identificador | identifier | traduz | poph-matrix, poph-server | 41 |
| interna | internal | traduz | poph-agent | 41 |
| mesma | same | traduz | poph-fluxos, poph-server | 41 |
| modelos | models | traduz | poph, poph-rag | 41 |
| obter | get | traduz | poph-server, poph | 41 |
| problemas | problems | traduz | poph, poph-server | 41 |
| processo | process | traduz | poph-server, poph | 41 |
| adiadas | deferred | traduz | poph-server, poph-agent | 40 |
| apoio | support | traduz | poph, poph-rag | 40 |
| atualizado | updated | traduz | poph-server | 40 |
| nem | nor | traduz | poph-server, poph-agent | 40 |
| nomeado | named | traduz | poph-fluxos, poph-agent | 40 |
| pela | by | traduz | poph-server, poph | 40 |
| posicionada | positioned | traduz | poph-matrix, poph-server | 40 |
| retencao | retention | traduz | console, poph | 40 |
| agenda | schedule | traduz | poph-agent, poph-server | 39 |
| ano | year | traduz | poph-matrix, poph-server | 39 |
| dado | given | traduz | poph-fluxos, poph-server | 39 |
| declarado | declared | traduz | poph-server, poph-agent | 39 |
| definicao | definition | traduz | poph-store, poph-server | 39 |
| entao | then | traduz | poph-fluxos, raiz | 39 |
| esquema | schema | traduz | poph-server, poph | 39 |
| gerar | generate | traduz | poph, poph-server | 39 |
| marca | mark | traduz | poph-server, poph-matrix | 39 |
| objeto | object | traduz | poph-server, poph-agent | 39 |
| senao | else | traduz | poph-fluxos, raiz | 39 |
| volatil | volatile | traduz | poph-agent | 39 |
| compila | compiles | traduz | poph-fluxos, poph | 38 |
| integracoes | integrations | traduz | poph-server, poph | 38 |
| parada | stop | traduz | poph-server, poph-fluxos | 38 |
| poderes | powers | traduz | poph-matrix | 38 |
| relatorios | reports | traduz | poph, poph-agent | 38 |
| ruim | poor | traduz | poph-retention, poph | 38 |
| temperatura | temperature | traduz | poph-core, poph-store | 38 |
| anotadora | annotator | traduz | poph-server | 37 |
| cancelamento | cancellation | traduz | poph-agent, poph | 37 |
| servico | service | traduz | poph-server, poph | 37 |
| vetores | vectors | traduz | poph-rag | 37 |
| centavos | cents | traduz | poph-agent, console | 36 |
| chaves | keys | traduz | poph-fluxos, poph | 36 |
| conflito | conflict | traduz | poph-server, poph | 36 |
| dos | of | traduz | poph-fluxos, poph-agent | 36 |
| duas | two | traduz | poph-server, poph-agent | 36 |
| encaminhar | forward | traduz | poph-fluxos, poph-server | 36 |
| enviadas | sent | traduz | poph-server, poph-agent | 36 |
| funcoes | functions | traduz | poph-fluxos, poph-agent | 36 |
| impresso | printed | traduz | poph-fluxos, poph | 36 |
| proibido | forbidden | traduz | poph-server, poph | 36 |
| volta | return | traduz | poph-server, poph-fluxos | 36 |
| apagar | delete | traduz | poph-rag, poph-matrix | 35 |
| avisar | notify | traduz | poph-server, console | 35 |
| buscas | searches | traduz | poph-agent | 35 |
| caractere | character | traduz | poph-matrix, poph-server | 35 |
| codificar | encode | traduz | poph-matrix, poph-agent | 35 |
| fontes | sources | traduz | poph, poph-agent | 35 |
| processar | process | traduz | poph-agent, poph-server | 35 |
| recebida | received | traduz | poph-agent, poph-retention | 35 |
| removido | removed | traduz | poph-fluxos, poph-rag | 35 |
| valido | valid | traduz | poph-server, poph-fluxos | 35 |
| comportamento | behaviour | traduz | poph-agent | 34 |
| criterio | criterion | traduz | poph-jev | 34 |
| desligada | disabled | traduz | poph-server, poph-jev | 34 |
| filtro | filter | traduz | poph, poph-server | 34 |
| fixa | fixed | traduz | poph-rag, poph-matrix | 34 |
| receber | receive | traduz | poph-agent, poph-server | 34 |
| resultados | results | traduz | poph-agent, poph-server | 34 |
| baratas | cheap | traduz | poph-agent, poph-rag | 33 |
| comparacao | comparison | traduz | poph, poph-fluxos | 33 |
| espelhar | mirror | traduz | poph-matrix, poph-server | 33 |
| fica | stays | traduz | poph-server, poph-fluxos | 33 |
| humana | human | traduz | poph-server, poph-matrix | 33 |
| notificar | notify | traduz | poph-fluxos, poph-server | 33 |
| oferecidas | offered | traduz | poph-agent | 33 |
| palavras | words | traduz | poph-rag, poph-fluxos | 33 |
| semear | seed | traduz | poph-server, poph-retention | 33 |
| simples | simple | traduz | poph-fluxos, poph-server | 33 |
| todas | all | traduz | poph-server, poph-agent | 33 |
| verificacao | verification | traduz | poph | 33 |
| convites | invitations | traduz | poph-matrix, poph-server | 32 |
| falta | missing | traduz | poph-matrix, poph | 32 |
| redacoes | writings | traduz | poph-server, poph-matrix | 32 |
| validade | validity | traduz | poph-server, poph-agent | 32 |
| variaveis | variables | traduz | poph-fluxos, poph | 32 |
| armazem | store | traduz | poph-retention, poph-agent | 31 |
| consultar | query | traduz | poph-agent, poph | 31 |
| divergente | divergent | traduz | poph-fluxos, poph-server | 31 |
| efetivos | effective | traduz | poph-jev, poph-agent | 31 |
| forcar | force | traduz | console, poph | 31 |
| hibrida | hybrid | traduz | poph-rag, poph-server | 31 |
| importar | import | traduz | poph-server, poph | 31 |
| preparado | prepared | traduz | poph-rag, poph-server | 31 |
| transferencia | transfer | traduz | poph-agent, poph-server | 31 |
| transitorio | transient | traduz | poph-matrix, poph-server | 31 |
| acima | above | traduz | poph-server, poph-fluxos | 30 |
| ativos | active | traduz | poph-server, poph-agent | 30 |
| capturados | captured | traduz | poph-server | 30 |
| credencial | credential | traduz | poph-matrix | 30 |
| divergencias | divergences | traduz | poph, poph-server | 30 |
| exemplo | example | traduz | poph-fluxos, poph-jev | 30 |
| fecha | closes | traduz | poph-fluxos, poph-agent | 30 |
| indexar | index | traduz | poph-rag, poph-agent | 30 |
| liberados | released | traduz | poph-agent, poph-server | 30 |
| ligada | linked | traduz | poph-server, poph-matrix | 30 |
| mes | month | traduz | poph-matrix, poph-server | 30 |
| minusculo | lowercase | traduz | poph-fluxos, poph-agent | 30 |
| mutacao | mutation | traduz | poph-fluxos | 30 |
| renderizar | render | traduz | poph-fluxos, poph-agent | 30 |
| verificacoes | verifications | traduz | poph | 30 |
| conectado | connected | traduz | poph-server | 29 |
| custo | cost | traduz | poph-agent, poph-matrix | 29 |
| devolve | returns | traduz | poph-server, poph-fluxos | 29 |
| diferente | different | traduz | poph-fluxos, poph | 29 |
| encerrar | close | traduz | poph-agent, poph-server | 29 |
| fechar | close | traduz | poph-server, poph | 29 |
| gravadas | stored | traduz | poph-server, poph-agent | 29 |
| pede | asks | traduz | poph-fluxos, poph-jev | 29 |
| posicoes | positions | traduz | poph-fluxos, poph-server | 29 |
| recebidos | received | traduz | poph-server, poph-agent | 29 |
| recusar | refuse | traduz | poph-agent, poph-server | 29 |
| relevancia | relevance | traduz | poph-rag, poph-agent | 29 |
| requisicoes | requests | traduz | poph-matrix | 29 |
| reserva | reservation | traduz | poph-agent, poph-server | 29 |
| rodadas | rounds | traduz | poph-agent, console | 29 |
| saltos | jumps | traduz | poph-fluxos, poph-agent | 29 |
| severidade | severity | traduz | poph-fluxos, poph-server | 29 |
| adiados | deferred | traduz | poph-agent, poph-server | 28 |
| agiram | acted | traduz | poph-fluxos, poph-server | 28 |
| ajuda | help | traduz | poph-fluxos, poph | 28 |
| apagadas | deleted | traduz | poph-server, poph-matrix | 28 |
| cai | falls | traduz | poph-agent, poph-matrix | 28 |
| codigos | codes | traduz | poph-fluxos, poph-server | 28 |
| dentro | inside | traduz | poph-fluxos, poph | 28 |
| esperados | expected | traduz | poph-server, poph-rag | 28 |
| falhos | faulty | traduz | poph-matrix, poph-server | 28 |
| preparar | prepare | traduz | poph-server, poph-matrix | 28 |
| provisionar | provision | traduz | poph-matrix, poph-server | 28 |
| responde | answers | traduz | poph-server, poph-agent | 28 |
| segurar | hold | traduz | poph-server | 28 |
| terceira | third | traduz | poph-server, poph-agent | 28 |
| todos | all | traduz | poph, poph-server | 28 |
| usada | used | traduz | poph-fluxos, poph-server | 28 |
| vizinha | neighbouring | traduz | poph-server, poph | 28 |
| adiado | deferred | traduz | poph-server, poph-agent | 27 |
| aparece | appears | traduz | poph-server, poph | 27 |
| definitivos | definitive | traduz | poph-server, poph-matrix | 27 |
| esperas | waits | traduz | poph-agent, poph-fluxos | 27 |
| feita | done | traduz | poph-server, poph | 27 |
| foto | photo | traduz | poph-retention, poph-server | 27 |
| loja | store | traduz | poph-server | 27 |
| posicional | positional | traduz | poph-fluxos | 27 |
| produzido | produced | traduz | poph-fluxos, poph-agent | 27 |
| valida | validates | traduz | poph-fluxos, poph-server | 27 |
| velho | old | traduz | poph-retention, poph-agent | 27 |
| abandonar | abandon | traduz | poph-server, poph-fluxos | 26 |
| compor | compose | traduz | poph-agent, poph | 26 |
| demora | delay | traduz | poph-agent | 26 |
| enviada | sent | traduz | poph-matrix, poph-server | 26 |
| grafo | graph | traduz | poph-fluxos | 26 |
| mantem | keeps | traduz | poph-agent, poph-matrix | 26 |
| opcao | option | traduz | poph-fluxos, poph-jev | 26 |
| plano | plan | traduz | poph-server | 26 |
| profundidade | depth | traduz | poph-rag, poph-fluxos | 26 |
| publicadas | published | traduz | poph-server | 26 |
| roteador | router | traduz | poph-server, poph-matrix | 26 |
| somar | sum | traduz | poph-agent, poph-server | 26 |
| cada | each | traduz | poph-server, poph-fluxos | 25 |
| carregamento | loading | traduz | poph-fluxos, poph-server | 25 |
| conta | counts | traduz | poph-server, poph-fluxos | 25 |
| contagem | count | traduz | poph-server, poph-matrix | 25 |
| desconhecido | unknown | traduz | poph-fluxos, poph-agent | 25 |
| entidade | entity | traduz | poph-server, poph | 25 |
| exigidos | required | traduz | poph-fluxos, poph-agent | 25 |
| inteiro | integer | traduz | poph, poph-agent | 25 |
| permissao | permission | traduz | poph-matrix, poph | 25 |
| peso | weight | traduz | poph-fluxos | 25 |
| recuperado | recovered | traduz | poph-agent | 25 |
| rotas | routes | traduz | poph-server | 25 |
| sao | are | traduz | poph-matrix, poph-server | 25 |
| validas | valid | traduz | poph-jev, poph-fluxos | 25 |
| avulsa | one_off | traduz | poph-server | 24 |
| classe | class | traduz | poph-agent, poph-server | 24 |
| condicao | condition | traduz | poph-fluxos, poph-server | 24 |
| curso | course | traduz | poph-fluxos, poph-agent | 24 |
| liberado | released | traduz | poph-server, poph-agent | 24 |
| livre | free | traduz | poph-server, poph-fluxos | 24 |
| manifestos | manifests | traduz | poph-server | 24 |
| nunca | never | traduz | poph-server, poph-matrix | 24 |
| retentativas | retries | traduz | poph-jev, poph-core | 24 |
| rodar | run | traduz | poph-agent, poph | 24 |
| sobrescrever | overwrite | traduz | poph-server, console | 24 |
| sonda | probe | traduz | poph-server | 24 |
| aguardar | wait | traduz | poph-agent | 23 |
| autorizado | authorized | traduz | poph-server, console | 23 |
| baixa | low | traduz | poph-jev, poph-agent | 23 |
| complementos | complements | traduz | poph-server | 23 |
| consultas | queries | traduz | poph-rag, poph-server | 23 |
| despedida | farewell | traduz | console, poph-agent | 23 |
| dourada | golden | traduz | poph-fluxos | 23 |
| final | final | traduz | poph-matrix, poph-fluxos | 23 |
| indisponivel | unavailable | traduz | poph-server, poph-matrix | 23 |
| latencia | latency | traduz | poph-server, poph-jev | 23 |
| melhores | best | traduz | poph-rag | 23 |
| parcial | partial | traduz | poph-fluxos, poph-server | 23 |
| recuo | backoff | traduz | poph-fluxos, poph | 23 |
| reindexar | reindex | traduz | poph, poph-server | 23 |
| secao | section | traduz | poph-fluxos, poph | 23 |
| transferir | transfer | traduz | poph-jev, poph-server | 23 |
| validador | validator | traduz | poph-fluxos | 23 |
| visita | visit | traduz | poph-fluxos, poph-server | 23 |
| alfa | alpha | traduz | poph-agent, poph-fluxos | 22 |
| atendimentos | services | traduz | poph-fluxos, poph | 22 |
| chegada | arrival | traduz | poph-agent, poph-fluxos | 22 |
| combinador | combinator | traduz | poph-fluxos | 22 |
| continuacao | continuation | traduz | poph-agent | 22 |
| disfarce | disguise | traduz | poph-server, poph-matrix | 22 |
| dona | owner | traduz | poph-server | 22 |
| encerrada | closed | traduz | poph-server, poph-fluxos | 22 |
| estados | states | traduz | poph-server, poph-agent | 22 |
| exclusao | exclusion | traduz | poph-server, poph | 22 |
| exibicao | display | traduz | poph-matrix, poph-fluxos | 22 |
| faltando | missing | traduz | poph-fluxos, poph-agent | 22 |
| gerados | generated | traduz | poph-rag, console | 22 |
| movida | moved | traduz | poph-matrix, poph-server | 22 |
| operacoes | operations | traduz | poph-server, poph | 22 |
| reais | actual | traduz | poph-agent, poph-server | 22 |
| unico | single | traduz | poph-server, poph-fluxos | 22 |
| cargas | loads | traduz | poph-fluxos, poph-server | 21 |
| configurar | configure | traduz | poph-server, poph | 21 |
| declarada | declared | traduz | poph-server, poph-matrix | 21 |
| deixa | leaves | traduz | poph-fluxos, poph-matrix | 21 |
| derivar | derive | traduz | poph-server, poph | 21 |
| diz | says | traduz | poph, poph-fluxos | 21 |
| dominio | domain | traduz | poph-server, poph-agent | 21 |
| fator | factor | traduz | poph-server, poph-agent | 21 |
| feito | done | traduz | poph-server, poph-retention | 21 |
| grupos | groups | traduz | poph, poph-server | 21 |
| importacao | import | traduz | poph, poph-server | 21 |
| inicial | initial | traduz | poph-agent, poph-server | 21 |
| malformado | malformed | traduz | poph-fluxos, poph-server | 21 |
| menor | smaller | traduz | poph-fluxos, poph-server | 21 |
| proprias | own | traduz | poph-agent, poph | 21 |
| quantos | how_many | traduz | poph-server, poph-fluxos | 21 |
| recebidas | received | traduz | poph-agent, poph-jev | 21 |
| relativo | relative | traduz | poph-server, poph | 21 |
| segmento | segment | traduz | poph-server, poph-matrix | 21 |
| sim | yes | traduz | poph-fluxos, poph | 21 |
| tokenizador | tokeniser | traduz | poph-rag | 21 |
| velha | old | traduz | poph-server, poph | 21 |
| aberto | open | traduz | poph-server, poph-fluxos | 20 |
| booleano | boolean | traduz | poph-fluxos, poph | 20 |
| definitivas | definitive | traduz | poph-agent, poph-server | 20 |
| entra | enters | traduz | poph-agent, poph-server | 20 |
| fala | speaks | traduz | poph-agent, poph-server | 20 |
| guardadas | guarded | traduz | poph-server, poph-fluxos | 20 |
| habilitada | enabled | traduz | poph-agent, poph | 20 |
| leva | takes | traduz | poph-matrix, poph-jev | 20 |
| lugar | place | traduz | poph-fluxos, poph-server | 20 |
| mostrado | shown | traduz | poph, poph-server | 20 |
| obtido | obtained | traduz | poph-rag, poph-jev | 20 |
| portugues | portuguese | traduz | poph, poph-server | 20 |
| regras | rules | traduz | poph-fluxos, poph-matrix | 20 |
| requisicao | request | traduz | poph-matrix, poph-jev | 20 |
| secoes | sections | traduz | poph-agent | 20 |
| segue | follows | traduz | poph-server, poph-agent | 20 |
| segura | safe | traduz | poph-agent, poph-server | 20 |
| soma | sum | traduz | poph-agent, poph-jev | 20 |
| virtuais | virtual | traduz | poph-matrix, poph-server | 20 |
| vitima | victim | traduz | poph-server, poph-fluxos | 20 |
| abaixo | below | traduz | poph-agent, poph-jev | 19 |
| aceita | accepts | traduz | poph-fluxos, poph-server | 19 |
| adquirir | acquire | traduz | poph-agent | 19 |
| alvos | target | traduz | poph-rag, poph-matrix | 19 |
| aspas | quotes | traduz | poph-fluxos, poph | 19 |
| atualizacoes | updates | traduz | poph-server, poph | 19 |
| contrato | contract | traduz | poph-matrix, poph-jev | 19 |
| entregue | delivered | traduz | poph-server, poph-agent | 19 |
| ignorado | ignored | traduz | poph-agent, poph-matrix | 19 |
| indexacao | indexing | traduz | poph-agent, poph-rag | 19 |
| invalidas | invalid | traduz | poph-fluxos, poph-agent | 19 |
| juntos | together | traduz | poph-server, poph-matrix | 19 |
| repetir | repeat | traduz | poph-server, poph-matrix | 19 |
| resolvedor | resolver | traduz | poph-agent | 19 |
| tarefas | tasks | traduz | poph-server, poph-rag | 19 |
| transitorios | transient | traduz | poph-server, poph-matrix | 19 |
| usa | uses | traduz | poph-agent, poph-matrix | 19 |
| abstencao | abstention | traduz | poph-jev, poph-agent | 18 |
| aceito | accepted | traduz | poph-fluxos, poph-server | 18 |
| achadas | found | traduz | poph-agent, poph-fluxos | 18 |
| alterada | changed | traduz | poph-server | 18 |
| apaga | deletes | traduz | poph-server, poph | 18 |
| avisadas | notified | traduz | poph-server | 18 |
| comercial | commercial | traduz | poph-server, poph-matrix | 18 |
| configurado | configured | traduz | console, poph-server | 18 |
| convidado | guest | traduz | poph-server, poph | 18 |
| cortar | cut | traduz | poph-agent, poph-matrix | 18 |
| criterios | criteria | traduz | poph-jev, poph-agent | 18 |
| declarados | declared | traduz | poph-server, poph-fluxos | 18 |
| expulsoes | evictions | traduz | poph-fluxos, poph-server | 18 |
| imagens | images | traduz | poph-agent | 18 |
| inicia | starts | traduz | poph-agent, poph-server | 18 |
| lidos | read | traduz | poph-fluxos, poph-jev | 18 |
| limiar | threshold | traduz | poph-agent, poph-jev | 18 |
| ordenadas | sorted | traduz | poph-jev, poph-server | 18 |
| periodo | period | traduz | poph-server | 18 |
| propria | own | traduz | poph-server, poph-agent | 18 |
| quantas | how_many | traduz | poph-server, poph-fluxos | 18 |
| reabrir | reopen | traduz | poph-server, poph-agent | 18 |
| removidos | removed | traduz | poph-rag, poph | 18 |
| respeita | respects | traduz | poph-agent, poph-server | 18 |
| semaforo | semaphore | traduz | poph-agent | 18 |
| temporaria | temporary | traduz | poph | 18 |
| usados | used | traduz | poph-fluxos, poph-agent | 18 |
| vivos | live | traduz | poph-agent | 18 |
| agendar | schedule | traduz | poph-fluxos, poph-agent | 17 |
| amostras | samples | traduz | poph-server, poph-rag | 17 |
| ativas | active | traduz | poph-server, console | 17 |
| baixado | downloaded | traduz | poph-server, poph-matrix | 17 |
| coletada | collected | traduz | poph-fluxos | 17 |
| concluidos | finished | traduz | poph-agent, poph-rag | 17 |
| consultado | queried | traduz | poph-jev, poph-agent | 17 |
| continua | continues | traduz | poph-agent, poph-server | 17 |
| deixou | left | traduz | poph-fluxos | 17 |
| derrubar | drop | traduz | poph-server, poph-fluxos | 17 |
| escolhido | chosen | traduz | poph-agent, poph-server | 17 |
| fechado | closed | traduz | poph-server, poph-agent | 17 |
| folha | sheet | traduz | poph-fluxos | 17 |
| frase | sentence | traduz | poph-fluxos, poph-server | 17 |
| gravacao | recording | traduz | poph-server, poph-matrix | 17 |
| hoje | today | traduz | poph-agent, poph-server | 17 |
| nomeavel | nameable | traduz | poph-server | 17 |
| paginas | pages | traduz | poph-server, poph-fluxos | 17 |
| pai | parent | traduz | poph-matrix, poph | 17 |
| perfis | profiles | traduz | poph-server | 17 |
| publicacao | publication | traduz | poph-server, poph | 17 |
| rede | network | traduz | poph-matrix, poph-server | 17 |
| reenvios | resends | traduz | poph-agent, poph-server | 17 |
| remover | remove | traduz | poph-agent, poph-matrix | 17 |
| renomeacoes | renames | traduz | poph-server | 17 |
| retomada | resumption | traduz | poph-agent, poph-matrix | 17 |
| tres | three | traduz | poph-fluxos, poph-jev | 17 |
| alterados | changed | traduz | poph-rag, console | 16 |
| andamento | progress | traduz | poph-agent | 16 |
| atualizadas | updated | traduz | console, poph-server | 16 |
| conferido | checked | traduz | poph, poph-server | 16 |
| duplicadas | duplicated | traduz | poph-server, poph-agent | 16 |
| encerramento | closure | traduz | poph-agent, poph-server | 16 |
| espelhadas | mirrored | traduz | poph-server | 16 |
| esquecer | forget | traduz | poph-server, poph-agent | 16 |
| hora | hour | traduz | poph-fluxos, poph-server | 16 |
| humano | human | traduz | poph-server, poph-matrix | 16 |
| inalterados | unchanged | traduz | poph-rag, console | 16 |
| inativo | inactive | traduz | poph, poph-server | 16 |
| lotes | batches | traduz | poph-rag | 16 |
| medir | measure | traduz | poph-server, poph-jev | 16 |
| nota | note | traduz | poph-rag, poph-agent | 16 |
| notificacao | notification | traduz | poph-fluxos, poph-server | 16 |
| novas | new | traduz | poph-server, poph-matrix | 16 |
| palavra | word | traduz | poph-agent, poph-rag | 16 |
| passos | steps | traduz | poph-server, poph-agent | 16 |
| permitido | allowed | traduz | poph-server, poph-fluxos | 16 |
| pessoa | person | traduz | poph-matrix, poph-server | 16 |
| recarregar | reload | traduz | poph-server, poph-agent | 16 |
| regenerada | regenerated | traduz | poph-agent | 16 |
| remetentes | senders | traduz | poph-server | 16 |
| rotulado | labelled | traduz | poph-jev, poph-rag | 16 |
| seguidas | consecutive | traduz | poph-server | 16 |
| sintaxe | syntax | traduz | poph-fluxos, poph | 16 |
| sufixo | suffix | traduz | poph-matrix, poph | 16 |
| tratar | handle | traduz | poph-server, poph-agent | 16 |
| travar | lock | traduz | poph-agent, poph-matrix | 16 |
| unidade | unit | traduz | poph-agent, console | 16 |
| ver | see | traduz | poph-server | 16 |
| vezes | times | traduz | poph-server, poph-fluxos | 16 |
| abertura | opening | traduz | poph-server, poph | 15 |
| alheio | foreign | traduz | poph-server, poph-agent | 15 |
| alteradas | changed | traduz | poph-server | 15 |
| alterado | changed | traduz | poph-server, poph | 15 |
| auditorias | audits | traduz | poph-retention, poph-server | 15 |
| capturado | captured | traduz | poph-matrix, poph-server | 15 |
| caracteres | characters | traduz | poph-agent, poph | 15 |
| desconhecida | unknown | traduz | poph-server, console | 15 |
| disponivel | available | traduz | poph-rag, poph-agent | 15 |
| dispositivo | device | traduz | poph-matrix, poph-server | 15 |
| encerradas | closed | traduz | poph-agent, poph-server | 15 |
| escalares | scalars | traduz | poph | 15 |
| espelhada | mirrored | traduz | poph-matrix, poph-server | 15 |
| estavel | stable | traduz | poph, poph-matrix | 15 |
| estimar | estimate | traduz | poph-agent | 15 |
| expirar | expire | traduz | poph-matrix, poph-server | 15 |
| exportar | export | traduz | poph, poph-server | 15 |
| filhos | children | traduz | poph-fluxos, poph-server | 15 |
| inseridas | inserted | traduz | console, poph-server | 15 |
| latencias | latencies | traduz | poph-jev, poph-agent | 15 |
| ligacao | link | traduz | poph | 15 |
| redigidas | drafted | traduz | poph-server | 15 |
| redigir | draft | traduz | poph-matrix, poph-agent | 15 |
| registrada | recorded | traduz | poph-server, poph-agent | 15 |
| resolucao | resolution | traduz | poph-fluxos, poph | 15 |
| situacoes | situations | traduz | console, poph-agent | 15 |
| suporte | support | traduz | poph-matrix, poph-server | 15 |
| terceiro | third | traduz | poph-server, poph-agent | 15 |
| topo | top | traduz | poph-fluxos | 15 |
| agir | act | traduz | poph-server, poph-agent | 14 |
| apertado | tight | traduz | poph-server, poph-fluxos | 14 |
| assinados | signed | traduz | poph-server | 14 |
| colecao | collection | traduz | poph-server | 14 |
| coletor | collector | traduz | poph-agent | 14 |
| conflitos | conflicts | traduz | poph-server, poph | 14 |
| convidar | invite | traduz | poph-matrix | 14 |
| cria | creates | traduz | poph, poph-matrix | 14 |
| curto | short | traduz | poph-matrix, poph-server | 14 |
| demais | others | traduz | poph-fluxos, poph-jev | 14 |
| desconhecidas | unknown | traduz | poph-server | 14 |
| digitos | digits | traduz | poph-fluxos, poph-matrix | 14 |
| escritor | writer | traduz | poph-matrix, poph | 14 |
| esgotadas | exhausted | traduz | poph-server, poph-agent | 14 |
| guardado | stored | traduz | poph-server, poph-agent | 14 |
| isolamento | isolation | traduz | poph-server, poph-retention | 14 |
| legenda | caption | traduz | poph-server | 14 |
| legivel | readable | traduz | poph-agent, poph | 14 |
| leituras | reads | traduz | poph-server, poph-agent | 14 |
| paginacao | pagination | traduz | poph-server | 14 |
| pares | pairs | traduz | poph, poph-fluxos | 14 |
| pontuacao | score | traduz | poph-rag | 14 |
| poupadas | saved | traduz | poph-fluxos, poph-server | 14 |
| processadas | processed | traduz | poph-server | 14 |
| rapidas | fast | traduz | poph-agent | 14 |
| saude | health | traduz | poph-server, raiz | 14 |
| segmentos | segments | traduz | poph-server | 14 |
| telefone | phone | traduz | poph-matrix, poph-server | 14 |
| trocado | swapped | traduz | poph-server, poph-matrix | 14 |
| vazamento | leak | traduz | poph-server | 14 |
| condicoes | conditions | traduz | poph-agent, console | 13 |
| construir | build | traduz | poph-server, poph | 13 |
| esperadas | expected | traduz | poph-server, poph | 13 |
| estatisticas | statistics | traduz | poph-agent, poph-server | 13 |
| extrator | extractor | traduz | poph-files, scripts | 13 |
| gerado | generated | traduz | poph, poph-server | 13 |
| informado | informed | traduz | poph-server, console | 13 |
| instante | instant | traduz | poph-matrix, poph-agent | 13 |
| limpa | clean | traduz | poph-server, poph-fluxos | 13 |
| marcar | mark | traduz | poph-matrix, poph-server | 13 |
| meio | middle | traduz | poph-server, poph-agent | 13 |
| nas | in_the | traduz | poph-server, poph-agent | 13 |
| pareamento | pairing | traduz | poph-server | 13 |
| passa | passes | traduz | poph-fluxos, poph-server | 13 |
| passagem | passage | traduz | poph-server, poph-matrix | 13 |
| percentil | percentile | traduz | poph-jev, poph-rag | 13 |
| pode | can | traduz | poph-agent, poph | 13 |
| renomear | rename | traduz | poph-server, poph-fluxos | 13 |
| termo | term | traduz | poph-agent, poph-rag | 13 |
| versoes | versions | traduz | poph-server, poph | 13 |
| vinculo | link | traduz | poph-server, poph | 13 |
| vistas | views | traduz | poph-server, poph-fluxos | 13 |
| concluido | finished | traduz | poph-agent | 12 |
| confirmacao | confirmation | traduz | poph-agent | 12 |
| criadas | created | traduz | poph-server | 12 |
| degradada | degraded | traduz | console | 12 |
| direcao | direction | traduz | poph-agent, poph-server | 12 |
| disposicao | layout | traduz | poph-server | 12 |
| executada | executed | traduz | poph-agent | 12 |
| extraido | extracted | traduz | poph-agent, poph-server | 12 |
| orfaos | orphans | traduz | poph, poph-retention | 12 |
| precisa | needs | traduz | poph-fluxos, poph-jev | 12 |
| recente | recent | traduz | poph-retention, poph-server | 12 |
| recusados | refused | traduz | poph-agent, poph-server | 12 |
| simulacao | simulation | traduz | poph, poph-retention | 12 |
| traco | trace | traduz | poph-server, poph | 12 |
| transferida | transferred | traduz | poph-server, poph-agent | 12 |
| adicionadas | added | traduz | console, poph-server | 11 |
| antigas | old | traduz | poph-matrix, poph-retention | 11 |
| carrinhos | carts | traduz | console, poph-agent | 11 |
| comentarios | comments | traduz | poph-fluxos, poph | 11 |
| comum | common | traduz | poph-agent | 11 |
| decorrido | elapsed | traduz | poph-agent, poph-jev | 11 |
| edicao | edit | traduz | poph-matrix, poph-server | 11 |
| filas | queues | traduz | poph-agent | 11 |
| iniciado | started | traduz | poph-agent, poph-retention | 11 |
| limpar | clean | traduz | poph-agent, poph-fluxos | 11 |
| mediana | median | traduz | poph-agent, poph-server | 11 |
| metrica | metric | traduz | poph-server | 11 |
| minuscula | lowercase | traduz | poph-server, poph-agent | 11 |
| mudancas | changes | traduz | poph-server, poph | 11 |
| parado | stopped | traduz | poph-fluxos, poph | 11 |
| preenchidos | filled | traduz | console, poph-server | 11 |
| removidas | removed | traduz | console, poph-server | 11 |
| trocaram | swapped | traduz | console, poph-server | 11 |
| acento | accent | traduz | poph-rag, poph-fluxos | 10 |
| apagado | deleted | traduz | poph, poph-agent | 10 |
| aplicar | apply | traduz | poph, poph-agent | 10 |
| compartilhado | shared | traduz | poph-agent, poph | 10 |
| concluida | finished | traduz | poph-fluxos, poph-server | 10 |
| concorrencia | concurrency | traduz | poph-matrix, poph-server | 10 |
| criando | creating | traduz | poph-matrix, poph-server | 10 |
| decididas | decided | traduz | poph-jev, console | 10 |
| declara | declares | traduz | poph-server, poph | 10 |
| desconectar | disconnect | traduz | poph-server, console | 10 |
| direto | direct | traduz | poph-server, poph | 10 |
| duplicados | duplicated | traduz | poph-agent, poph-plugin-eco | 10 |
| enviando | sending | traduz | poph-agent | 10 |
| finalizada | finalised | traduz | poph-store, poph-agent | 10 |
| idade | age | traduz | poph-retention, poph | 10 |
| ilegivel | unreadable | traduz | poph-matrix, poph-server | 10 |
| mais | more | traduz | poph-server, poph-agent | 10 |
| medicao | measurement | traduz | poph-agent, scripts | 10 |
| publicado | published | traduz | poph-server, poph-matrix | 10 |
| trabalho | work | traduz | poph-fluxos | 10 |
| unicos | single | traduz | poph-fluxos, poph-jev | 10 |
| vencidas | expired | traduz | poph-server | 10 |
| verificar | verify | traduz | poph, poph-server | 10 |
| acumulado | accumulated | traduz | poph-matrix | 9 |
| apagou | deleted | traduz | poph-matrix, poph-server | 9 |
| aplica | applies | traduz | poph-jev, poph-matrix | 9 |
| caminhos | paths | traduz | poph-fluxos, poph-server | 9 |
| celulas | cells | traduz | poph-server | 9 |
| conexoes | connections | traduz | poph-jev, poph-server | 9 |
| corpos | bodies | traduz | poph-server | 9 |
| estava | was | traduz | poph-matrix, poph | 9 |
| extracao | extraction | traduz | poph-server, poph-agent | 9 |
| falharam | failed | traduz | poph-agent, poph-plugin-eco | 9 |
| forcado | forced | traduz | poph-server, poph | 9 |
| listagem | listing | traduz | poph-server, poph-agent | 9 |
| margem | margin | traduz | poph-jev, console | 9 |
| metodos | methods | traduz | poph-server | 9 |
| minimo | minimum | traduz | poph-server, poph-fluxos | 9 |
| previa | preview | traduz | poph-server | 9 |
| prontos | ready | traduz | poph-matrix | 9 |
| recibo | receipt | traduz | poph-fluxos | 9 |
| alcancados | reached | traduz | poph-fluxos | 8 |
| atuais | current | traduz | poph-rag, poph-agent | 8 |
| automatico | automatic | traduz | poph-matrix, poph-server | 8 |
| candidata | candidate | traduz | poph-agent, poph-fluxos | 8 |
| cosseno | cosine | traduz | poph-rag | 8 |
| desde | since | traduz | poph, poph-server | 8 |
| destinos | targets | traduz | poph-fluxos | 8 |
| dono | owner | traduz | poph-server, poph-fluxos | 8 |
| entrou | entered | traduz | poph-matrix, poph-store | 8 |
| esta | this | traduz | poph-server, poph-agent | 8 |
| instalacao | installation | traduz | poph, poph-agent | 8 |
| instalado | installed | traduz | poph-server, poph | 8 |
| politica | policy | traduz | poph-agent, poph-server | 8 |
| recurso | resource | traduz | poph-server | 8 |
| tentar | try | traduz | poph-matrix, poph-jev | 8 |
| terminar | finish | traduz | poph-server | 8 |
| tocar | touch | traduz | poph-server, poph-fluxos | 8 |
| adicionar | add | traduz | poph-agent | 7 |
| avancou | advanced | traduz | poph-fluxos, poph-agent | 7 |
| certificado | certificate | traduz | poph-jev | 7 |
| controle | control | traduz | poph-server, poph-matrix | 7 |
| deixar | leave | traduz | poph-agent | 7 |
| guardados | guarded | traduz | poph-server, poph | 7 |
| guardar | keep | traduz | poph-server, poph-fluxos | 7 |
| idempotente | idempotent | traduz | poph, poph-agent | 7 |
| invalidos | invalid | traduz | poph-agent, poph-fluxos | 7 |
| negativo | negative | traduz | poph-fluxos | 7 |
| seguro | safe | traduz | poph-matrix, poph-server | 7 |
| sensivel | sensitive | traduz | poph, poph-agent | 7 |
| subida | rise | traduz | poph-server, poph-rag | 7 |
| vinculado | linked | traduz | poph, poph-server | 7 |
| voltou | returned | traduz | poph-fluxos, poph | 7 |
| antigos | old | traduz | poph-matrix, poph-agent | 6 |
| cifrados | encrypted | traduz | poph | 6 |
| colunas | columns | traduz | poph, poph-retention | 6 |
| nenhuma | none | traduz | poph-fluxos, poph-server | 6 |
| normalizar | normalise | traduz | poph, poph-rag | 6 |
| opcional | optional | traduz | poph-agent, poph-server | 6 |
| propriedades | properties | traduz | poph-agent, poph-server | 6 |
| recarga | reload | traduz | poph-server | 6 |
| repeticao | repetition | traduz | poph-agent, poph-server | 6 |
| retirar | withdraw | traduz | poph | 6 |
| simular | simulate | traduz | poph-retention | 6 |
| tudo | everything | traduz | poph-server, poph-agent | 6 |
| varrer | scan | traduz | poph-rag | 6 |
| vincular | link | traduz | poph, poph-server | 6 |
| aninhado | nested | traduz | poph-jev, poph-server | 5 |
| apagada | deleted | traduz | poph-matrix, poph-server | 5 |
| confirmar | confirm | traduz | poph-server | 5 |
| disco | disk | traduz | poph-server, poph | 5 |
| duplicado | duplicate | traduz | poph, poph-server | 5 |
| encerrado | closed | traduz | poph-fluxos, poph-agent | 5 |
| esgotado | exhausted | traduz | poph-agent, poph-server | 5 |
| este | this | traduz | poph-agent | 5 |
| faixa | range | traduz | poph, poph-server | 5 |
| geracao | generation | traduz | poph-rag | 5 |
| inclui | includes | traduz | poph-agent | 5 |
| linguagem | language | traduz | poph-fluxos | 5 |
| migracao | migration | traduz | poph | 5 |
| ocupada | busy | traduz | poph-agent | 5 |
| pessoas | people | traduz | poph-matrix | 5 |
| remota | remote | traduz | poph-server | 5 |
| revogado | revoked | traduz | poph, poph-server | 5 |
| revogar | revoke | traduz | poph-server, poph | 5 |
| simultaneos | simultaneous | traduz | raiz, poph-agent | 5 |
| sistema | system | traduz | poph-server, poph-matrix | 5 |
| temporario | temporary | traduz | poph | 5 |
| abandonada | abandoned | traduz | poph-fluxos, poph-server | 4 |
| agrupamento | grouping | traduz | poph-agent | 4 |
| alterar | modify | traduz | poph, poph-fluxos | 4 |
| apenas | only | traduz | poph-jev, poph-agent | 4 |
| aplicada | applied | traduz | poph-agent | 4 |
| baixada | downloaded | traduz | poph-server | 4 |
| caixa | box | traduz | poph-fluxos, poph-agent | 4 |
| cancelado | cancelled | traduz | poph-fluxos | 4 |
| derivado | derived | traduz | poph, poph-server | 4 |
| escolher | choose | traduz | poph-server | 4 |
| expirada | expired | traduz | poph-server, poph-matrix | 4 |
| filtros | filters | traduz | poph-server | 4 |
| ida | trip | traduz | poph-fluxos, poph-server | 4 |
| motivos | reasons | traduz | poph-matrix, poph-jev | 4 |
| presenca | presence | traduz | poph, poph-agent | 4 |
| problema | problem | traduz | raiz, poph | 4 |
| visivel | visible | traduz | poph-agent, poph-matrix | 4 |
| atualizada | updated | traduz | poph-server, poph-store | 3 |
| autor | author | traduz | poph-fluxos | 3 |
| capturar | capture | traduz | poph-server | 3 |
| conclusao | completion | traduz | poph-server, poph-agent | 3 |
| contato | contact | traduz | poph-agent, raiz | 3 |
| encontrada | found | traduz | poph-agent | 3 |
| extensao | extension | traduz | poph-server, poph | 3 |
| fechamento | closing | traduz | poph-server, poph-fluxos | 3 |
| finalizar | finalise | traduz | poph-matrix | 3 |
| habilitado | enabled | traduz | poph-agent, poph | 3 |
| livres | free | traduz | poph-fluxos | 3 |
| mudar | change | traduz | poph-agent, poph-rag | 3 |
| norma | norm | traduz | poph-rag | 3 |
| organizacao | organisation | traduz | poph-fluxos | 3 |
| pedacos | pieces | traduz | poph-server, poph-matrix | 3 |
| ponto | point | traduz | poph-agent, poph-server | 3 |
| principio | principle | traduz | poph-agent | 3 |
| probabilidades | probabilities | traduz | poph-jev | 3 |
| prontas | ready | traduz | poph-server | 3 |
| pronto | ready | traduz | poph-agent | 3 |
| quantizar | quantise | traduz | poph-rag | 3 |
| recuperados | recovered | traduz | poph-agent | 3 |
| substituir | replace | traduz | poph-server | 3 |
| varredura | scan | traduz | poph-retention | 3 |
| vinculos | links | traduz | poph-server | 3 |
| aplicavel | applicable | traduz | poph-jev | 2 |
| celula | cell | traduz | poph-server | 2 |
| combina | combines | traduz | poph-matrix | 2 |
| consertar | repair | traduz | poph-server | 2 |
| editar | edit | traduz | poph-agent, poph-rag | 2 |
| equipe | team | traduz | poph-fluxos, poph-server | 2 |
| exportacao | export | traduz | poph-server | 2 |
| familia | family | traduz | poph-matrix | 2 |
| geral | general | traduz | poph-agent, poph-retention | 2 |
| largura | width | traduz | poph-fluxos | 2 |
| orfao | orphan | traduz | poph-retention | 2 |
| sair | exit | traduz | poph-agent | 2 |
| truncar | truncate | traduz | poph | 2 |
| vagas | slots | traduz | poph-agent | 2 |
| alteracao | change | traduz | poph | 1 |
| apagados | deleted | traduz | poph-retention | 1 |
| arquivado | archived | traduz | poph-agent | 1 |
| correcao | fix | traduz | poph-matrix | 1 |
| desabilitado | disabled | traduz | poph-agent | 1 |
| dica | hint | traduz | poph-fluxos | 1 |
| digitado | typed | traduz | poph-agent | 1 |
| enganosa | misleading | traduz | poph | 1 |
| explicacao | explanation | traduz | poph-agent | 1 |
| expor | expose | traduz | poph-server | 1 |
| frente | front | traduz | poph-server | 1 |
| migracoes | migrations | traduz | poph | 1 |
| minutos | minutes | traduz | scripts | 1 |
| organizacoes | organisations | traduz | poph-server | 1 |
| painel | panel | traduz | poph-agent | 1 |
| processado | processed | traduz | poph-agent | 1 |
| programa | program | traduz | poph-server | 1 |
| reiniciar | restart | traduz | poph-server | 1 |
| rotina | routine | traduz | poph-server | 1 |
| salvo | saved | traduz | poph-server | 1 |
| selecionado | selected | traduz | console | 1 |
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
| atual | — | congelado | coluna no_atual; coluna revisao_atual; indice ix_fluxo_execucoes_org_status_no_atual | - |
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
| chave | — | congelado | campo_json chave_idempotencia; chave_toml llm.chave_env; coluna chave; e mais 2 | - |
| com | — | congelado | backup com_conversas; flag_cli --com-conversas | - |
| confianca | — | congelado | chave_toml handoff.orientacao_confianca | - |
| configuracao | — | congelado | protocolo_plugin configuracao | - |
| contato | — | congelado | chave_toml handoff.orientacao_ingestao_contato_produto | - |
| contem | — | congelado | linguagem de fluxos | - |
| conteudo | — | congelado | operation_id arquivos_conteudo; operation_id teste_chat_anexo_conteudo | - |
| conversa | — | congelado | flag_cli --conversa; indice ux_fichas_conversa | - |
| conversas | — | congelado | backup com_conversas; flag_cli --com-conversas; metrica poph_conversas_ativas; e mais 5 | - |
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
| em | — | congelado | backup exportado_em; coluna aberta_em; coluna concluiu_em; e mais 14 | - |
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
| recusa | — | congelado | chave_toml llm.recusa_turno_modelo | - |
| registro | — | congelado | comando_cli poph matrix registro | - |
| regra | — | congelado | chave_toml fluxos.max_heap_regra; chave_toml fluxos.max_ticks_regra; metrica poph_fluxos_erros_regra_total | - |
| reindexar | — | congelado | operation_id reindexar | - |
| remetente | — | congelado | protocolo_plugin remetente | - |
| remoto | — | congelado | flag_cli --remoto | - |
| resolucoes | — | congelado | coluna resolucoes_json | - |
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
| substituir | — | congelado | operation_id substituir | - |
| tabelas | — | congelado | backup tabelas | - |
| temperatura | — | congelado | chave_toml llm.temperatura | - |
| tentativa | — | congelado | coluna proxima_tentativa; indice ix_inbox_proxima_tentativa | - |
| tentativas | — | congelado | chave_toml turno.tentativas_falha_modelo; coluna tentativas | - |
| teste | — | congelado | operation_id teste_chat_anexo_conteudo; operation_id teste_chat_criar; operation_id teste_chat_historico; e mais 3 | - |
| texto | — | congelado | protocolo_plugin max_texto_bytes; protocolo_plugin texto | - |
| tipo | — | congelado | coluna ator_tipo; coluna tipo; protocolo_plugin tipo | - |
| tipos | — | congelado | protocolo_plugin tipos_aceitos | - |
| transferir | — | congelado | operation_id conversas_transferir | - |
| tuple | — | congelado | linguagem de fluxos | - |
| turno | — | congelado | chave_toml llm.recusa_turno_modelo; chave_toml turno; chave_toml turno.atraso_falha_modelo_ms; e mais 3 | - |
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
