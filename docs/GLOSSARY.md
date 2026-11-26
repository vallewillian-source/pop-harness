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
| cenario | scenario | traduz | poph-agent, poph-server | 5179 |
| erro | error | traduz | poph-server, poph | 4320 |
| texto | text | traduz | poph-agent, poph-server | 4158 |
| estado | state | traduz | poph-server, poph-fluxos | 3274 |
| agente | agent | traduz | poph-server, poph-agent | 2935 |
| conversa | conversation | traduz | poph-server, poph-agent | 2780 |
| nome | name | traduz | poph-server, poph-fluxos | 2264 |
| novo | new | traduz | poph-server, poph-agent | 1912 |
| de | of | traduz | poph-server, poph-fluxos | 1848 |
| corpo | body | traduz | poph-server, poph-matrix | 1838 |
| resposta | response | traduz | poph-server, poph-jev | 1816 |
| com | with | traduz | poph-server, poph-agent | 1764 |
| fluxo | flow | traduz | poph-agent, poph-server | 1672 |
| pasta | folder | traduz | poph, poph-server | 1593 |
| sala | room | traduz | poph-server, poph-matrix | 1537 |
| da | of | traduz | poph-server, poph-matrix | 1440 |
| cliente | client | traduz | poph-server, poph-matrix | 1369 |
| mensagem | message | traduz | poph-agent, poph-server | 1368 |
| motor | engine | traduz | poph-agent, poph-server | 1349 |
| valor | value | traduz | poph-server, poph-fluxos | 1343 |
| saida | output | traduz | poph, poph-server | 1280 |
| resultado | result | traduz | poph-store, poph-fluxos | 1249 |
| caminho | path | traduz | poph-server, poph | 1229 |
| tipo | kind | traduz | poph-fluxos, poph-server | 1223 |
| turno | turn | traduz | poph-agent, poph-server | 1189 |
| evento | event | traduz | console, poph-server | 1171 |
| motivo | reason | traduz | poph-server, poph-matrix | 1159 |
| chave | key | traduz | poph-server, poph-fluxos | 1120 |
| dados | data | traduz | poph-matrix, poph-server | 1106 |
| metricas | metrics | traduz | poph-agent, poph-server | 1093 |
| acao | action | traduz | poph-fluxos, poph | 1056 |
| sem | without | traduz | poph-server, poph-agent | 1047 |
| campo | field | traduz | poph-fluxos, console | 1034 |
| arquivo | file | traduz | poph, poph-server | 1019 |
| codigo | code | traduz | poph-fluxos, poph-server | 920 |
| registro | record | traduz | poph-agent, poph-server | 899 |
| etapa | step | traduz | poph-fluxos, poph-server | 819 |
| falha | failure | traduz | poph-server, poph | 800 |
| servidor | server | traduz | console, poph-server | 791 |
| departamento | department | traduz | poph-server, poph | 790 |
| opcoes | options | traduz | poph-agent, poph-server | 781 |
| nao | not | traduz | poph-server, poph-fluxos | 775 |
| para | for | traduz | poph-fluxos, poph-server | 736 |
| resumo | summary | traduz | poph-agent, poph-server | 736 |
| destino | target | traduz | poph-server, poph-fluxos | 732 |
| decisao | decision | traduz | poph-jev, poph-fluxos | 731 |
| em | in | traduz | poph-server, poph-fluxos | 729 |
| criar | create | traduz | poph-server, poph | 700 |
| midia | media | traduz | poph-matrix, poph-server | 618 |
| espaco | space | traduz | poph-matrix, poph-server | 593 |
| pedido | request | traduz | poph-agent, poph-server | 588 |
| ficha | case | traduz | poph-server, poph-agent | 582 |
| alvo | target | traduz | poph-server, poph-fluxos | 576 |
| falso | false | traduz | poph-server, poph-agent | 573 |
| linha | line | traduz | poph-server, poph-fluxos | 568 |
| ao | to | traduz | poph-fluxos, poph-server | 566 |
| entrada | entry | traduz | poph-agent, poph-server | 563 |
| itens | items | traduz | poph-server, poph-fluxos | 563 |
| regra | rule | traduz | poph-fluxos, poph-server | 549 |
| relatorio | report | traduz | poph-server, poph | 544 |
| canal | channel | traduz | poph-server, poph-agent | 543 |
| pedir | ask | traduz | poph-server, poph-evolution | 539 |
| produto | product | traduz | poph-agent, poph | 538 |
| chamada | call | traduz | poph-server, poph-matrix | 531 |
| modelo | model | traduz | poph-agent, poph-rag | 520 |
| fonte | source | traduz | poph-fluxos, poph-server | 518 |
| gatilho | trigger | traduz | poph-agent, poph-fluxos | 509 |
| versao | version | traduz | poph-server, console | 500 |
| indice | index | traduz | poph-rag, poph-agent | 495 |
| inicio | beginning | traduz | poph-server, poph-agent | 481 |
| atual | current | traduz | poph-server, poph-agent | 477 |
| nova | new | traduz | poph-fluxos, poph-agent | 474 |
| lista | list | traduz | poph-fluxos, poph-server | 473 |
| origem | origin | traduz | poph-jev, poph | 469 |
| porta | port | traduz | poph-server, poph-matrix | 467 |
| ambiente | environment | traduz | poph, poph-files | 456 |
| provedor | provider | traduz | poph-agent, poph-server | 446 |
| fim | end | traduz | poph-fluxos, poph-agent | 443 |
| escolha | choice | traduz | poph-jev, poph-agent | 426 |
| grupo | group | traduz | poph, poph-server | 426 |
| integracao | integration | traduz | poph-server, console | 425 |
| limites | limits | traduz | poph-fluxos, poph-agent | 412 |
| aviso | notice | traduz | poph-server, poph | 406 |
| contexto | context | traduz | poph-fluxos, poph-agent | 400 |
| fluxos | flows | traduz | poph-server, poph-agent | 397 |
| caso | case | traduz | poph-server, poph-fluxos | 384 |
| ator | actor | traduz | poph-fluxos, poph-store | 381 |
| mensagens | messages | traduz | poph-agent, poph-server | 373 |
| execucao | execution | traduz | poph-server, poph-agent | 372 |
| envio | send | traduz | poph-server, poph-agent | 370 |
| configuracao | configuration | traduz | poph, poph-server | 364 |
| quente | hot | traduz | poph-agent, poph-server | 363 |
| instancia | instance | traduz | poph-server, poph-evolution | 355 |
| efeito | effect | traduz | poph-fluxos, poph-server | 342 |
| teto | cap | traduz | poph-server, poph-fluxos | 341 |
| rotulo | label | traduz | poph-server, poph-fluxos | 339 |
| revisao | revision | traduz | poph-server, poph-agent | 338 |
| situacao | situation | traduz | console, poph-retention | 337 |
| campos | fields | traduz | poph-fluxos, poph-agent | 336 |
| conteudo | content | traduz | poph-matrix, poph-server | 335 |
| por | by | traduz | poph-agent, poph | 334 |
| atores | actors | traduz | poph-agent, poph-fluxos | 333 |
| identidade | identity | traduz | poph-server, poph-rag | 331 |
| lote | batch | traduz | poph-rag, poph-jev | 329 |
| desligar | disable | traduz | poph-server, poph | 327 |
| padrao | default | traduz | poph-agent, poph-server | 324 |
| produtos | products | traduz | poph-agent, poph-server | 320 |
| executar | run | traduz | poph-fluxos, poph-agent | 312 |
| guarda | guard | traduz | poph-matrix, poph-server | 309 |
| avaliador | evaluator | traduz | poph-fluxos, poph-agent | 304 |
| ferramenta | tool | traduz | poph-agent, poph | 299 |
| categoria | category | traduz | poph, poph-server | 297 |
| eventos | events | traduz | poph-server, poph-fluxos | 292 |
| pedidos | requests | traduz | poph-agent, poph-core | 292 |
| salas | rooms | traduz | poph-server, poph-matrix | 292 |
| numero | number | traduz | console, poph-store | 285 |
| efeitos | effects | traduz | poph-fluxos, poph-server | 284 |
| outro | other | traduz | poph-server, poph | 281 |
| metodo | method | traduz | poph-server, poph-matrix | 278 |
| whatsapp | whatsapp | traduz | console, poph-server | 278 |
| relato | report | traduz | poph-server, poph-agent | 276 |
| segredo | secret | traduz | poph-server, poph | 275 |
| vazio | empty | traduz | poph-server, poph-fluxos | 274 |
| mapa | map | traduz | poph-server, poph-agent | 268 |
| chamadas | calls | traduz | poph-agent, poph-server | 266 |
| fila | queue | traduz | poph-rag, poph-server | 263 |
| enviar | send | traduz | poph-server, poph-matrix | 261 |
| pausa | pause | traduz | poph-agent, poph-server | 260 |
| antes | before | traduz | poph-server, poph-matrix | 259 |
| rota | route | traduz | poph-server, poph-fluxos | 258 |
| consulta | query | traduz | poph-server, poph-matrix | 257 |
| banco | database | traduz | poph-retention, poph-rag | 250 |
| limite | limit | traduz | poph-server, poph-fluxos | 248 |
| arquivos | files | traduz | poph-server, poph-agent | 247 |
| mudanca | change | traduz | poph-fluxos, poph | 245 |
| pagina | page | traduz | poph-server, poph | 245 |
| responder | reply | traduz | poph-server, poph-matrix | 244 |
| gravado | stored | traduz | poph-server, poph-agent | 240 |
| na | towards | traduz | poph-server, poph-matrix | 238 |
| espacos | spaces | traduz | poph-server, poph-matrix | 236 |
| falhas | failures | traduz | poph-server, poph-agent | 236 |
| bloco | block | traduz | poph-agent, poph-fluxos | 234 |
| segredos | secrets | traduz | poph, console | 231 |
| detalhe | detail | traduz | poph, poph-server | 230 |
| primeira | first | traduz | poph-server, poph-agent | 229 |
| fantasma | ghost | traduz | poph-matrix, poph-server | 228 |
| iniciar | start | traduz | poph-fluxos, poph-server | 228 |
| gravada | stored | traduz | poph-server, poph-agent | 226 |
| memoria | memory | traduz | poph-agent, poph-server | 225 |
| validar | validate | traduz | poph-server, poph-types | 225 |
| nomes | names | traduz | poph-fluxos, poph-agent | 222 |
| interno | internal | traduz | poph-agent, poph-server | 221 |
| invalido | invalid | traduz | poph-server, poph-fluxos | 216 |
| pendentes | pending | traduz | poph-agent, poph-server | 216 |
| ciclo | cycle | traduz | poph-server, poph-fluxos | 215 |
| modulo | module | traduz | poph-server, poph-fluxos | 214 |
| partes | parts | traduz | poph-server, poph-agent | 213 |
| depois | after | traduz | poph-server, console | 209 |
| que | that | traduz | poph-fluxos, poph-server | 207 |
| esperar | wait | traduz | poph-server, poph-agent | 206 |
| registrar | register | traduz | poph-server, poph-agent | 202 |
| textos | texts | traduz | poph-rag, poph-server | 202 |
| anexo | attachment | traduz | poph-agent, poph-server | 201 |
| espera | wait | traduz | poph-agent, poph-server | 198 |
| falar | speak | traduz | poph-agent, poph-server | 197 |
| agentes | agents | traduz | poph, poph-server | 196 |
| esperado | expected | traduz | poph, poph-rag | 195 |
| primeiro | first | traduz | poph-server, poph-agent | 195 |
| abrir | open | traduz | poph, poph-server | 194 |
| modo | mode | traduz | poph-server, console | 194 |
| pergunta | question | traduz | poph-jev, poph-agent | 194 |
| respostas | responses | traduz | poph-agent, poph-server | 194 |
| tamanho | size | traduz | poph-agent, poph-matrix | 194 |
| cabecalhos | headers | traduz | poph-server, poph-matrix | 192 |
| catalogo | catalog | traduz | poph-agent, poph | 190 |
| externo | external | traduz | poph-server, poph-agent | 190 |
| ferramentas | tools | traduz | poph-agent, poph-server | 190 |
| descricao | description | traduz | console, poph-agent | 188 |
| ler | read | traduz | poph, poph-server | 186 |
| cru | raw | traduz | poph-server, poph-agent | 184 |
| remetente | sender | traduz | poph-matrix, poph-server | 181 |
| conversas | conversations | traduz | poph-server, poph-retention | 180 |
| recusa | refusal | traduz | poph-server, poph-files | 180 |
| confianca | confidence | traduz | poph-jev, poph-agent | 179 |
| posicao | position | traduz | poph-matrix, poph-rag | 178 |
| subir | raise | traduz | poph-server, poph-matrix | 176 |
| outra | other | traduz | poph-server, poph-agent | 174 |
| registros | records | traduz | poph-server, poph-matrix | 172 |
| turnos | turns | traduz | poph-agent, poph-server | 172 |
| limitador | limiter | traduz | poph-agent, poph-server | 171 |
| trechos | passages | traduz | poph-rag, poph-agent | 171 |
| agora | now | traduz | poph-server, poph-agent | 169 |
| falsa | false | traduz | poph-server, poph-agent | 169 |
| impacto | impact | traduz | poph-fluxos | 164 |
| busca | search | traduz | poph-agent, poph-rag | 163 |
| roteiro | script | traduz | poph-agent, poph-server | 162 |
| desfecho | outcome | traduz | poph-agent, poph-server | 161 |
| segunda | second | traduz | poph-server, poph-agent | 161 |
| erros | errors | traduz | poph-fluxos, poph-server | 160 |
| fantasmas | ghosts | traduz | poph-matrix, poph-server | 160 |
| proprio | own | traduz | poph-agent, poph-server | 160 |
| corte | cutoff | traduz | poph-agent, poph-matrix | 159 |
| futuro | future | traduz | poph-agent, poph-rag | 159 |
| transacao | transaction | traduz | poph-server, poph-matrix | 156 |
| endereco | address | traduz | poph-server, poph-matrix | 154 |
| listar | list | traduz | poph, poph-server | 153 |
| manifesto | manifest | traduz | poph-server, console | 153 |
| ordem | order | traduz | poph-server, console | 153 |
| transporte | transport | traduz | poph-agent, poph-jev | 153 |
| ultimo | last | traduz | poph-server, poph-agent | 151 |
| teste | test | traduz | poph, poph-server | 150 |
| vai | goes | traduz | poph-fluxos, raiz | 149 |
| carregar | load | traduz | poph, poph-fluxos | 147 |
| instrucoes | instructions | traduz | poph, poph-agent | 147 |
| usuarios | users | traduz | poph-matrix, poph-server | 147 |
| mudou | changed | traduz | poph-fluxos, poph-server | 146 |
| entrega | delivery | traduz | poph-server, poph-agent | 144 |
| trecho | passage | traduz | poph-agent, poph-rag | 144 |
| etapas | steps | traduz | poph-fluxos, poph-server | 143 |
| avisos | notices | traduz | poph-server, poph-fluxos | 141 |
| espelho | mirror | traduz | poph-server, poph-matrix | 141 |
| escolhas | choices | traduz | poph-agent, poph-jev | 137 |
| grande | large | traduz | poph-server, poph-matrix | 137 |
| conexao | connection | traduz | poph-server, poph | 136 |
| leitor | reader | traduz | poph-fluxos, poph | 134 |
| quando | when | traduz | poph-fluxos, poph-jev | 134 |
| buscar | search | traduz | poph-agent, poph-rag | 132 |
| como | as | traduz | poph-fluxos, poph | 132 |
| confere | checks | traduz | poph-fluxos, poph-server | 132 |
| decisoes | decisions | traduz | poph-fluxos, poph-jev | 131 |
| limiares | thresholds | traduz | poph-jev | 131 |
| diagnosticos | diagnostics | traduz | poph-fluxos, poph-server | 130 |
| casos | cases | traduz | poph-jev, poph-fluxos | 129 |
| relogio | clock | traduz | poph-server, poph-agent | 129 |
| compilado | compiled | traduz | poph-fluxos, poph-server | 128 |
| janela | window | traduz | poph-agent, poph-rag | 128 |
| tentativas | attempts | traduz | poph-server, poph-agent | 127 |
| uma | a | traduz | poph-server, poph-fluxos | 127 |
| sinal | signal | traduz | poph-server, poph-agent | 126 |
| tarefa | task | traduz | poph-agent, poph-server | 126 |
| receptor | receiver | traduz | poph-server, poph-plugins | 125 |
| diagnostico | diagnostic | traduz | poph-fluxos, poph-server | 124 |
| fora | outside | traduz | poph-server, poph-fluxos | 124 |
| atendimento | service | traduz | poph-fluxos, poph-server | 123 |
| lido | read | traduz | poph, poph-server | 123 |
| observador | observer | traduz | poph-agent, poph-server | 123 |
| uso | use | traduz | poph-agent, poph-fluxos | 123 |
| ajustes | adjustments | traduz | poph-agent, poph-server | 121 |
| prefixo | prefix | traduz | poph-server, poph-matrix | 121 |
| usuario | user | traduz | poph-matrix, poph-server | 121 |
| envios | sends | traduz | poph-server, poph-agent | 120 |
| atraso | delay | traduz | poph-server, poph-agent | 119 |
| auditoria | audit | traduz | poph-server, poph-retention | 119 |
| classificacao | classification | traduz | poph-matrix, poph-server | 119 |
| argumentos | arguments | traduz | poph-agent, console | 118 |
| captura | capture | traduz | poph-server, poph | 118 |
| avaliar | evaluate | traduz | poph-fluxos, poph-jev | 117 |
| orientacao | guidance | traduz | poph-jev, poph-agent | 116 |
| anexos | attachments | traduz | poph-server, poph-agent | 115 |
| comando | command | traduz | poph, poph-evolution | 115 |
| das | of | traduz | poph-server, poph-agent | 115 |
| inexistente | missing | traduz | poph-fluxos, poph-server | 115 |
| sessao | session | traduz | poph, poph-rag | 115 |
| variavel | variable | traduz | poph-fluxos, poph | 115 |
| acoes | actions | traduz | poph-fluxos, raiz | 114 |
| gravar | write | traduz | poph-retention, poph-server | 114 |
| recusado | refused | traduz | poph-server, poph-agent | 114 |
| vetor | vector | traduz | poph-rag | 113 |
| duracao | duration | traduz | poph-agent, poph-server | 112 |
| linhas | lines | traduz | poph, poph-server | 112 |
| invalida | invalid | traduz | poph-fluxos, poph-server | 111 |
| exigir | require | traduz | poph-server, poph | 110 |
| segundo | second | traduz | poph-agent, poph-server | 110 |
| membros | members | traduz | poph-server, poph-matrix | 109 |
| contadores | counters | traduz | poph-server, poph-fluxos | 108 |
| bruto | raw | traduz | poph-fluxos, poph-server | 107 |
| ignorar | ignore | traduz | poph-matrix, poph-server | 107 |
| nivel | level | traduz | poph, poph-matrix | 107 |
| prazo | deadline | traduz | poph-fluxos, poph-server | 107 |
| ajuste | adjustment | traduz | poph-jev, poph-agent | 106 |
| imprimir | print | traduz | poph, poph-fluxos | 106 |
| atualizar | update | traduz | poph-server, poph | 105 |
| fechada | closed | traduz | poph-store, poph-server | 105 |
| imagem | image | traduz | poph-files, poph-matrix | 105 |
| ja | already | traduz | poph-server, poph-matrix | 105 |
| limpo | clean | traduz | poph-server, poph-matrix | 105 |
| par | pair | traduz | poph-server, poph | 105 |
| montar | build | traduz | poph-agent, poph-server | 104 |
| carga | load | traduz | poph-rag, poph-fluxos | 101 |
| dias | days | traduz | poph-retention, console | 101 |
| atendente | attendant | traduz | poph-server, poph-matrix | 100 |
| candidato | candidate | traduz | poph-jev, poph-agent | 100 |
| criado | created | traduz | poph-server, poph | 100 |
| definir | define | traduz | poph-server, poph-agent | 100 |
| recebido | received | traduz | poph-server, poph-files | 100 |
| descrever | describe | traduz | poph, poph-agent | 99 |
| escritas | writings | traduz | poph-server, poph-matrix | 99 |
| aparelho | device | traduz | poph-server, poph-matrix | 98 |
| encerra | closes | traduz | poph-fluxos, poph-agent | 97 |
| papel | role | traduz | poph-matrix, poph-jev | 97 |
| titulo | title | traduz | console, poph-rag | 97 |
| mostrar | show | traduz | poph-server, poph | 96 |
| retencao | retention | traduz | poph-retention, console | 96 |
| documento | document | traduz | poph-server, poph | 95 |
| midias | media | traduz | poph-server, poph-evolution | 95 |
| parametros | parameters | traduz | poph-agent, poph-server | 95 |
| ligado | linked | traduz | poph-server, poph-agent | 94 |
| nenhum | none | traduz | poph-fluxos, poph-server | 94 |
| ou | or | traduz | poph-fluxos, poph-agent | 94 |
| formato | format | traduz | poph-matrix, poph-store | 92 |
| tempos | times | traduz | poph-agent, poph-fluxos | 92 |
| sincronizar | synchronise | traduz | poph-agent, poph-server | 91 |
| familia | family | traduz | poph-files, poph-matrix | 90 |
| instrucao | instruction | traduz | poph-fluxos, raiz | 90 |
| aberta | open | traduz | poph-server, poph-fluxos | 89 |
| formatar | format | traduz | poph-agent, poph-fluxos | 89 |
| diretorio | directory | traduz | poph-rag, poph-server | 88 |
| entradas | entries | traduz | poph-agent, poph-fluxos | 88 |
| resto | rest | traduz | poph-server, poph | 88 |
| veneno | poison | traduz | poph-server, poph-agent | 88 |
| ausente | missing | traduz | poph-fluxos, poph-server | 87 |
| desligado | disabled | traduz | poph-server, poph | 87 |
| tem | has | traduz | poph, poph-server | 87 |
| buscador | searcher | traduz | poph-agent, poph | 86 |
| enviados | sent | traduz | poph-server, poph-agent | 86 |
| maior | bigger | traduz | poph-fluxos, poph-rag | 86 |
| baixar | download | traduz | poph-server, poph-matrix | 85 |
| exige | requires | traduz | poph-fluxos, raiz | 85 |
| ligar | link | traduz | poph-server, poph | 85 |
| pronta | ready | traduz | poph-matrix, poph-server | 85 |
| mundo | world | traduz | poph-server | 84 |
| obrigatorio | required | traduz | poph-fluxos, poph-matrix | 84 |
| saidas | outputs | traduz | poph-agent, poph-server | 83 |
| validacao | validation | traduz | poph-server, poph | 82 |
| vivo | live | traduz | poph, poph-server | 82 |
| cabecalho | header | traduz | poph-server, poph-core | 81 |
| conhecimento | knowledge | traduz | poph-server, poph-agent | 81 |
| departamentos | departments | traduz | poph-server, poph-fluxos | 81 |
| interpretar | interpret | traduz | poph-jev, poph-evolution | 81 |
| montagem | assembly | traduz | poph-agent, poph | 81 |
| receber | receive | traduz | poph-files, poph-agent | 81 |
| servir | serve | traduz | poph-server, poph | 81 |
| funcao | function | traduz | poph-fluxos, raiz | 80 |
| muda | changes | traduz | poph-fluxos, poph-agent | 79 |
| onde | where | traduz | poph-fluxos, poph | 79 |
| operacao | operation | traduz | poph-matrix, poph-server | 79 |
| perguntas | questions | traduz | poph-jev, poph-agent | 79 |
| apos | after | traduz | poph-server, poph-agent | 78 |
| igual | equal | traduz | poph-fluxos, poph-server | 78 |
| instancias | instances | traduz | poph-server, console | 78 |
| candidatos | candidates | traduz | poph-jev, poph-agent | 77 |
| parar | stop | traduz | poph-server, poph-agent | 77 |
| preparo | preparation | traduz | poph-agent, poph | 77 |
| canais | channels | traduz | poph-server, poph | 76 |
| falhar | fail | traduz | poph-server, poph-matrix | 76 |
| globais | globals | traduz | poph-server, poph-jev | 76 |
| idempotencia | idempotency | traduz | poph-store, poph-server | 76 |
| processador | processor | traduz | poph-agent, poph-server | 76 |
| ate | until | traduz | poph-server, poph-agent | 75 |
| autorizacao | authorization | traduz | poph-matrix, poph-server | 75 |
| entrar | enter | traduz | poph-fluxos, poph-matrix | 75 |
| expiracao | expiry | traduz | poph-server, poph-matrix | 75 |
| preco | price | traduz | poph-agent, console | 75 |
| argumento | argument | traduz | poph-fluxos, poph-agent | 74 |
| raiz | root | traduz | poph-server, poph-evolution | 74 |
| classificar | classify | traduz | poph-matrix, poph-files | 73 |
| enviado | sent | traduz | poph-server, poph-store | 73 |
| nulo | null | traduz | poph-agent, poph-server | 73 |
| testes | tests | traduz | poph-server, poph | 73 |
| bruta | raw | traduz | poph-fluxos, poph-server | 72 |
| chamar | call | traduz | poph-agent, poph-server | 72 |
| enfileirar | enqueue | traduz | poph-matrix, poph-agent | 72 |
| historico | history | traduz | poph-server, poph-agent | 72 |
| achado | finding | traduz | poph-rag, poph-agent | 71 |
| revisoes | revisions | traduz | poph-server, poph-fluxos | 71 |
| canonico | canonical | traduz | poph-fluxos, poph-jev | 70 |
| cifra | cipher | traduz | poph-server, poph | 70 |
| escopo | scope | traduz | poph-fluxos, console | 70 |
| quantidade | quantity | traduz | poph-agent, poph-server | 70 |
| quem | who | traduz | poph-server, poph-agent | 70 |
| repetida | repeated | traduz | poph-server, poph-fluxos | 70 |
| vira | becomes | traduz | poph-server, poph-matrix | 70 |
| entregar | deliver | traduz | poph-server, poph-agent | 69 |
| metadados | metadata | traduz | poph-files, poph-server | 69 |
| tabela | table | traduz | poph, poph-agent | 69 |
| conjunto | set | traduz | poph-jev, poph | 68 |
| excluir | delete | traduz | poph, poph-server | 68 |
| fichas | cases | traduz | poph-server, poph-store | 68 |
| pelo | by | traduz | poph-server, poph-matrix | 68 |
| proximo | next | traduz | poph-server, poph-agent | 68 |
| segundos | seconds | traduz | poph-server, poph-rag | 68 |
| ativo | active | traduz | poph-agent, console | 67 |
| compilar | compile | traduz | poph-server, poph-fluxos | 67 |
| exemplos | examples | traduz | poph-fluxos, poph-jev | 67 |
| leitura | read | traduz | poph-files, poph-agent | 67 |
| transitoria | transient | traduz | poph-server, poph-matrix | 67 |
| antigo | old | traduz | poph-matrix, poph-fluxos | 66 |
| conferir | check | traduz | poph-server, poph-evolution | 66 |
| diferenca | difference | traduz | poph-fluxos, poph-server | 66 |
| pendente | pending | traduz | poph-server, poph-agent | 66 |
| cortado | cut | traduz | poph-fluxos, poph-matrix | 65 |
| parte | part | traduz | poph-agent, poph-files | 65 |
| provisionador | provisioner | traduz | poph-matrix, poph-server | 65 |
| unica | single | traduz | poph-fluxos, poph-server | 65 |
| atender | attend | traduz | poph-server, poph-matrix | 64 |
| contar | count | traduz | poph-server, poph-retention | 64 |
| ignorado | ignored | traduz | poph-evolution, poph-agent | 64 |
| rodada | round | traduz | poph-agent, poph-server | 64 |
| garantir | ensure | traduz | poph-matrix, poph-server | 63 |
| nada | nothing | traduz | poph-agent, poph-server | 63 |
| pedaco | piece | traduz | poph-matrix, poph-server | 63 |
| tempo | time | traduz | poph-agent, poph-jev | 63 |
| tipos | kinds | traduz | poph-fluxos, poph-jev | 63 |
| achados | findings | traduz | poph-agent, poph-rag | 62 |
| carimbo | stamp | traduz | poph-server, poph-fluxos | 62 |
| ouvinte | listener | traduz | poph-matrix, poph-server | 62 |
| recebidos | received | traduz | poph-evolution, poph-server | 62 |
| categorias | categories | traduz | poph, poph-server | 61 |
| confiavel | reliable | traduz | poph-jev, poph-agent | 61 |
| criacao | creation | traduz | poph-matrix, poph-server | 61 |
| contador | counter | traduz | poph-rag, poph-server | 60 |
| evolucao | evolution | traduz | poph-server | 60 |
| marcador | marker | traduz | poph-server, poph-agent | 60 |
| referencia | reference | traduz | poph, poph-rag | 60 |
| resolve | resolves | traduz | scripts, poph-fluxos | 60 |
| vaga | slot | traduz | poph-agent, poph-rag | 60 |
| escrita | writing | traduz | poph-server, poph-matrix | 59 |
| passo | step | traduz | poph-server, poph-fluxos | 59 |
| resolucoes | resolutions | traduz | poph-fluxos, poph-server | 59 |
| proxima | next | traduz | poph-server, poph-matrix | 58 |
| ultima | last | traduz | poph-server, poph-agent | 58 |
| publicada | published | traduz | poph-server, poph-matrix | 57 |
| armazem | store | traduz | poph-retention, poph | 56 |
| escrever | write | traduz | poph, poph-server | 56 |
| perfil | profile | traduz | poph-matrix, poph-server | 56 |
| roteiros | scripts | traduz | poph-agent, poph-server | 56 |
| coletor | collector | traduz | poph-plugins, poph-agent | 55 |
| contem | contains | traduz | poph-fluxos, poph-server | 55 |
| declarado | declared | traduz | poph-server, poph-evolution | 55 |
| dois | two | traduz | poph-fluxos, poph-server | 55 |
| laco | loop | traduz | poph-server, poph-agent | 55 |
| novos | new | traduz | poph-fluxos, poph-rag | 55 |
| acertos | hits | traduz | poph-rag, poph-jev | 54 |
| contagem | count | traduz | poph-rag, poph-server | 54 |
| definitiva | definitive | traduz | poph-matrix, poph-server | 54 |
| liberados | released | traduz | poph-agent, poph-jev | 54 |
| politica | policy | traduz | poph-evolution, poph-agent | 54 |
| risco | risk | traduz | poph-jev, poph-agent | 54 |
| trava | lock | traduz | poph-rag, poph-fluxos | 54 |
| assinatura | signature | traduz | poph-server, poph-files | 53 |
| coluna | column | traduz | poph-fluxos, poph | 53 |
| comparar | compare | traduz | poph-server, poph | 53 |
| execucoes | executions | traduz | poph-server, poph-store | 53 |
| filho | child | traduz | poph-matrix, poph-fluxos | 53 |
| objeto | object | traduz | poph-server, poph-jev | 53 |
| posicionar | position | traduz | poph-matrix, poph-server | 53 |
| publicar | publish | traduz | poph-server, poph | 53 |
| recusada | refused | traduz | poph-agent, poph | 53 |
| retrato | snapshot | traduz | poph, poph-server | 53 |
| antiga | old | traduz | poph-fluxos, poph-server | 52 |
| ativa | active | traduz | poph-agent, poph-server | 52 |
| caracteres | characters | traduz | poph-files, poph-rag | 52 |
| convidados | guests | traduz | poph-matrix, poph-server | 52 |
| falhou | failed | traduz | poph-server, poph-agent | 52 |
| mesmo | same | traduz | poph-fluxos, poph-server | 52 |
| tentativa | attempt | traduz | poph-agent, poph-server | 52 |
| ultimas | last | traduz | poph-agent | 52 |
| esperada | expected | traduz | poph-server, poph | 51 |
| estruturado | structured | traduz | poph-agent, poph-server | 51 |
| existente | existing | traduz | poph-matrix, poph | 51 |
| vistos | seen | traduz | poph-fluxos, poph-server | 51 |
| anterior | previous | traduz | poph-server, poph-fluxos | 50 |
| existe | exists | traduz | poph-server, poph-matrix | 50 |
| forma | form | traduz | poph-fluxos, poph | 50 |
| longo | long | traduz | poph-rag, poph-fluxos | 50 |
| maximo | maximum | traduz | poph-agent, poph-jev | 50 |
| orcamento | budget | traduz | poph-agent, poph-rag | 50 |
| rodando | running | traduz | poph-server, poph-retention | 50 |
| apoio | support | traduz | poph, poph-core | 49 |
| compartilhado | shared | traduz | poph-rag, poph-agent | 49 |
| convite | invitation | traduz | poph-matrix, poph-server | 49 |
| cursores | cursors | traduz | poph-server, poph | 49 |
| despachar | dispatch | traduz | poph-agent, poph-server | 49 |
| detalhes | details | traduz | poph-server, poph-jev | 49 |
| entre | between | traduz | poph-fluxos, poph-server | 49 |
| errado | wrong | traduz | poph-server, poph-fluxos | 49 |
| todas | all | traduz | poph-server, poph-agent | 49 |
| transicao | transition | traduz | poph-server, poph-agent | 49 |
| avancar | advance | traduz | poph-agent, poph-server | 48 |
| cena | scene | traduz | poph-server | 48 |
| margem | margin | traduz | poph-jev, console | 48 |
| poder | power | traduz | poph-matrix | 48 |
| decidir | decide | traduz | poph-matrix, poph-server | 47 |
| rascunho | draft | traduz | poph-server, poph-store | 47 |
| tabelas | tables | traduz | console, poph | 47 |
| usado | used | traduz | poph-fluxos, poph-agent | 47 |
| biblioteca | library | traduz | poph-server, poph-agent | 46 |
| marca | mark | traduz | poph-server, poph-matrix | 46 |
| mostra | shows | traduz | poph, poph-fluxos | 46 |
| nos | us | traduz | poph-fluxos, poph-server | 46 |
| remoto | remote | traduz | poph, poph-server | 46 |
| vez | time | traduz | poph-server, poph-fluxos | 46 |
| aguardando | waiting | traduz | poph-fluxos, poph-agent | 45 |
| criada | created | traduz | poph-server, poph | 45 |
| encontrado | found | traduz | poph-agent, poph | 45 |
| lida | read | traduz | poph-server, poph-fluxos | 45 |
| valores | values | traduz | poph-server, poph-fluxos | 45 |
| decodificar | decode | traduz | poph-server, poph-matrix | 44 |
| dia | day | traduz | poph-matrix, poph-server | 44 |
| intervalo | interval | traduz | poph-server, poph-agent | 44 |
| orfaos | orphans | traduz | poph-retention, poph | 44 |
| processo | process | traduz | poph-server, poph | 44 |
| redacao | writing | traduz | poph-matrix, poph-server | 44 |
| removida | removed | traduz | poph-fluxos, poph-matrix | 44 |
| sentido | meaning | traduz | poph-matrix, poph-server | 44 |
| vazia | empty | traduz | poph-agent, poph-matrix | 44 |
| carrinho | cart | traduz | poph-agent, poph-server | 43 |
| espelhado | mirrored | traduz | poph-server, poph-matrix | 43 |
| extensao | extension | traduz | poph-files, poph-server | 43 |
| gravados | stored | traduz | poph-server, poph-plugin-eco | 43 |
| liberar | release | traduz | poph-agent, poph | 43 |
| locais | local | traduz | poph-fluxos | 43 |
| nem | nor | traduz | poph-server, poph-agent | 43 |
| obter | get | traduz | poph-server, poph | 43 |
| recebida | received | traduz | poph-agent, poph-evolution | 43 |
| similaridade | similarity | traduz | poph-rag, poph-agent | 43 |
| aceito | accepted | traduz | poph-files, poph-server | 42 |
| coleta | collection | traduz | poph-fluxos | 42 |
| criados | created | traduz | poph-server, poph-agent | 42 |
| passar | pass | traduz | poph-server, poph-agent | 42 |
| repetido | repeated | traduz | poph-fluxos, poph | 42 |
| ruim | poor | traduz | poph-retention, poph | 42 |
| vetores | vectors | traduz | poph-rag | 42 |
| identificador | identifier | traduz | poph-matrix, poph-server | 41 |
| interna | internal | traduz | poph-agent | 41 |
| mesma | same | traduz | poph-fluxos, poph-server | 41 |
| minusculo | lowercase | traduz | poph-fluxos, poph-evolution | 41 |
| modelos | models | traduz | poph, poph-rag | 41 |
| pela | by | traduz | poph-server, poph | 41 |
| problemas | problems | traduz | poph, poph-server | 41 |
| transitorio | transient | traduz | poph-matrix, poph-server | 41 |
| adiadas | deferred | traduz | poph-server, poph-agent | 40 |
| apagar | delete | traduz | poph-rag, poph-matrix | 40 |
| atualizado | updated | traduz | poph-server | 40 |
| caractere | character | traduz | poph-matrix, poph-server | 40 |
| nomeado | named | traduz | poph-fluxos, poph-agent | 40 |
| posicionada | positioned | traduz | poph-matrix, poph-server | 40 |
| agenda | schedule | traduz | poph-agent, poph-server | 39 |
| ano | year | traduz | poph-matrix, poph-server | 39 |
| dado | given | traduz | poph-fluxos, poph-server | 39 |
| definicao | definition | traduz | poph-store, poph-server | 39 |
| entao | then | traduz | poph-fluxos, raiz | 39 |
| esquema | schema | traduz | poph-server, poph | 39 |
| gerar | generate | traduz | poph, poph-server | 39 |
| senao | else | traduz | poph-fluxos, raiz | 39 |
| simples | simple | traduz | poph-fluxos, poph-server | 39 |
| volta | return | traduz | poph-server, poph-fluxos | 39 |
| compila | compiles | traduz | poph-fluxos, poph | 38 |
| integracoes | integrations | traduz | poph-server, poph | 38 |
| parada | stop | traduz | poph-server, poph-fluxos | 38 |
| poderes | powers | traduz | poph-matrix | 38 |
| relatorios | reports | traduz | poph, poph-agent | 38 |
| validade | validity | traduz | poph-server, poph-jev | 38 |
| anotadora | annotator | traduz | poph-server | 37 |
| cancelamento | cancellation | traduz | poph-agent, poph | 37 |
| devolve | returns | traduz | poph-server, poph-fluxos | 37 |
| duas | two | traduz | poph-server, poph-agent | 37 |
| nunca | never | traduz | poph-evolution, poph-server | 37 |
| processar | process | traduz | poph-agent, poph-server | 37 |
| servico | service | traduz | poph-server, poph | 37 |
| temperatura | temperature | traduz | poph-core, poph-store | 37 |
| volatil | volatile | traduz | poph-agent | 37 |
| centavos | cents | traduz | poph-agent, console | 36 |
| chaves | keys | traduz | poph-fluxos, poph | 36 |
| conflito | conflict | traduz | poph-server, poph | 36 |
| dos | of | traduz | poph-fluxos, poph-agent | 36 |
| encaminhar | forward | traduz | poph-fluxos, poph-server | 36 |
| enviadas | sent | traduz | poph-server, poph-agent | 36 |
| fica | stays | traduz | poph-server, poph-fluxos | 36 |
| funcoes | functions | traduz | poph-fluxos, poph-agent | 36 |
| impresso | printed | traduz | poph-fluxos, poph | 36 |
| proibido | forbidden | traduz | poph-server, poph | 36 |
| valido | valid | traduz | poph-server, poph-fluxos | 36 |
| acima | above | traduz | poph-server, poph-fluxos | 35 |
| avisar | notify | traduz | poph-server, console | 35 |
| buscas | searches | traduz | poph-agent | 35 |
| codificar | encode | traduz | poph-matrix, poph-agent | 35 |
| dentro | inside | traduz | poph-fluxos, poph | 35 |
| desligada | disabled | traduz | poph-server, poph-jev | 35 |
| fontes | sources | traduz | poph, poph-agent | 35 |
| rede | network | traduz | poph-matrix, poph-evolution | 35 |
| removido | removed | traduz | poph-fluxos, poph-rag | 35 |
| transferencia | transfer | traduz | poph-agent, poph-server | 35 |
| unidade | unit | traduz | poph-rag, poph-agent | 35 |
| verificacao | verification | traduz | poph, poph-evolution | 35 |
| comportamento | behaviour | traduz | poph-agent | 34 |
| criterio | criterion | traduz | poph-jev | 34 |
| despedida | farewell | traduz | console, poph-jev | 34 |
| encerrada | closed | traduz | poph-server, poph-rag | 34 |
| filtro | filter | traduz | poph, poph-server | 34 |
| fixa | fixed | traduz | poph-rag, poph-matrix | 34 |
| palavra | word | traduz | poph-rag, poph-agent | 34 |
| resultados | results | traduz | poph-agent, poph-server | 34 |
| baratas | cheap | traduz | poph-agent, poph-rag | 33 |
| comparacao | comparison | traduz | poph, poph-fluxos | 33 |
| encerrar | close | traduz | poph-agent, poph-server | 33 |
| espelhar | mirror | traduz | poph-matrix, poph-server | 33 |
| frase | sentence | traduz | poph-rag, poph-fluxos | 33 |
| humana | human | traduz | poph-server, poph-matrix | 33 |
| notificar | notify | traduz | poph-fluxos, poph-server | 33 |
| oferecidas | offered | traduz | poph-agent | 33 |
| palavras | words | traduz | poph-rag, poph-fluxos | 33 |
| semear | seed | traduz | poph-server, poph-retention | 33 |
| aplicavel | applicable | traduz | poph-jev | 32 |
| carregamento | loading | traduz | poph-fluxos, poph-rag | 32 |
| convites | invitations | traduz | poph-matrix, poph-server | 32 |
| desconhecido | unknown | traduz | poph-fluxos, poph-agent | 32 |
| exemplo | example | traduz | poph-fluxos, poph-jev | 32 |
| falta | missing | traduz | poph-matrix, poph | 32 |
| obtido | obtained | traduz | poph-rag, poph-jev | 32 |
| redacoes | writings | traduz | poph-server, poph-matrix | 32 |
| variaveis | variables | traduz | poph-fluxos, poph | 32 |
| conectado | connected | traduz | poph-server, poph-evolution | 31 |
| consultar | query | traduz | poph-agent, poph | 31 |
| diferente | different | traduz | poph-fluxos, poph | 31 |
| divergente | divergent | traduz | poph-fluxos, poph-server | 31 |
| efetivos | effective | traduz | poph-jev, poph-agent | 31 |
| fecha | closes | traduz | poph-fluxos, poph-agent | 31 |
| forcar | force | traduz | console, poph | 31 |
| hibrida | hybrid | traduz | poph-rag, poph-server | 31 |
| importar | import | traduz | poph-server, poph | 31 |
| preparado | prepared | traduz | poph-rag, poph-server | 31 |
| ativos | active | traduz | poph-server, poph-agent | 30 |
| capturados | captured | traduz | poph-server | 30 |
| credencial | credential | traduz | poph-matrix | 30 |
| divergencias | divergences | traduz | poph, poph-server | 30 |
| fechar | close | traduz | poph-server, poph | 30 |
| indexar | index | traduz | poph-rag, poph-agent | 30 |
| ligada | linked | traduz | poph-server, poph-matrix | 30 |
| mes | month | traduz | poph-matrix, poph-server | 30 |
| mutacao | mutation | traduz | poph-fluxos | 30 |
| opcao | option | traduz | poph-fluxos, poph-jev | 30 |
| pareamento | pairing | traduz | poph-evolution, poph-server | 30 |
| renderizar | render | traduz | poph-fluxos, poph-agent | 30 |
| somar | sum | traduz | poph-agent, poph-server | 30 |
| valida | validates | traduz | poph-fluxos, poph-server | 30 |
| verificacoes | verifications | traduz | poph | 30 |
| aparece | appears | traduz | poph-server, poph | 29 |
| cai | falls | traduz | poph-agent, poph-matrix | 29 |
| custo | cost | traduz | poph-agent, poph-matrix | 29 |
| declarada | declared | traduz | poph-server, poph-files | 29 |
| enviada | sent | traduz | poph-matrix, poph-server | 29 |
| gravadas | stored | traduz | poph-server, poph-agent | 29 |
| parcial | partial | traduz | poph-fluxos, poph-rag | 29 |
| pede | asks | traduz | poph-fluxos, poph-jev | 29 |
| posicoes | positions | traduz | poph-fluxos, poph-server | 29 |
| recusar | refuse | traduz | poph-agent, poph-server | 29 |
| relevancia | relevance | traduz | poph-rag, poph-agent | 29 |
| requisicoes | requests | traduz | poph-matrix | 29 |
| retentativas | retries | traduz | poph-core, poph-evolution | 29 |
| rodadas | rounds | traduz | poph-agent, console | 29 |
| saltos | jumps | traduz | poph-fluxos, poph-agent | 29 |
| severidade | severity | traduz | poph-fluxos, poph-server | 29 |
| adiados | deferred | traduz | poph-agent, poph-server | 28 |
| agiram | acted | traduz | poph-fluxos, poph-server | 28 |
| ajuda | help | traduz | poph-fluxos, poph | 28 |
| apagadas | deleted | traduz | poph-server, poph-matrix | 28 |
| codigos | codes | traduz | poph-fluxos, poph-server | 28 |
| esperados | expected | traduz | poph-server, poph-rag | 28 |
| falhos | faulty | traduz | poph-matrix, poph-server | 28 |
| inferencia | inference | traduz | poph-rag, poph-agent | 28 |
| preparar | prepare | traduz | poph-server, poph-matrix | 28 |
| probabilidades | probabilities | traduz | poph-jev, poph-agent | 28 |
| provisionar | provision | traduz | poph-matrix, poph-server | 28 |
| reserva | reservation | traduz | poph-agent, poph-server | 28 |
| responde | answers | traduz | poph-server, poph-agent | 28 |
| segurar | hold | traduz | poph-server | 28 |
| terceira | third | traduz | poph-server, poph-agent | 28 |
| todos | all | traduz | poph, poph-server | 28 |
| usada | used | traduz | poph-fluxos, poph-server | 28 |
| varredura | scan | traduz | poph-retention | 28 |
| vizinha | neighbouring | traduz | poph-server, poph | 28 |
| adiado | deferred | traduz | poph-server, poph-agent | 27 |
| condicao | condition | traduz | poph-fluxos, poph-server | 27 |
| definitivos | definitive | traduz | poph-server, poph-matrix | 27 |
| esperas | waits | traduz | poph-agent, poph-fluxos | 27 |
| feita | done | traduz | poph-server, poph | 27 |
| foto | photo | traduz | poph-retention, poph-server | 27 |
| legenda | caption | traduz | poph-server, poph-evolution | 27 |
| legivel | readable | traduz | poph-core, poph-agent | 27 |
| loja | store | traduz | poph-server | 27 |
| mantem | keeps | traduz | poph-agent, poph-matrix | 27 |
| posicional | positional | traduz | poph-fluxos | 27 |
| produzido | produced | traduz | poph-fluxos, poph-agent | 27 |
| tokenizador | tokeniser | traduz | poph-rag | 27 |
| transferir | transfer | traduz | poph-jev, poph-agent | 27 |
| validas | valid | traduz | poph-jev, poph-fluxos | 27 |
| velho | old | traduz | poph-retention, poph-agent | 27 |
| abandonar | abandon | traduz | poph-server, poph-fluxos | 26 |
| compor | compose | traduz | poph-agent, poph | 26 |
| demora | delay | traduz | poph-agent | 26 |
| grafo | graph | traduz | poph-fluxos | 26 |
| inteiro | integer | traduz | poph, poph-agent | 26 |
| lidos | read | traduz | poph-fluxos, poph-rag | 26 |
| lotes | batches | traduz | poph-rag | 26 |
| plano | plan | traduz | poph-server | 26 |
| profundidade | depth | traduz | poph-rag, poph-fluxos | 26 |
| publicadas | published | traduz | poph-server | 26 |
| relativo | relative | traduz | poph-server, poph | 26 |
| roteador | router | traduz | poph-server, poph-matrix | 26 |
| sao | are | traduz | poph-matrix, poph-server | 26 |
| soma | sum | traduz | poph-jev, poph-agent | 26 |
| cada | each | traduz | poph-server, poph-fluxos | 25 |
| conta | counts | traduz | poph-server, poph-fluxos | 25 |
| disponivel | available | traduz | poph-rag, poph-jev | 25 |
| entidade | entity | traduz | poph-server, poph | 25 |
| entra | enters | traduz | poph-agent, poph-server | 25 |
| estados | states | traduz | poph-server, poph-agent | 25 |
| exigidos | required | traduz | poph-fluxos, poph-agent | 25 |
| leva | takes | traduz | poph-matrix, poph-evolution | 25 |
| norma | norm | traduz | poph-rag | 25 |
| permissao | permission | traduz | poph-matrix, poph | 25 |
| peso | weight | traduz | poph-fluxos | 25 |
| recuperado | recovered | traduz | poph-agent | 25 |
| rotas | routes | traduz | poph-server | 25 |
| aberto | open | traduz | poph-server, poph-fluxos | 24 |
| avulsa | one_off | traduz | poph-server | 24 |
| baixado | downloaded | traduz | poph-server, poph-matrix | 24 |
| classe | class | traduz | poph-agent, poph-server | 24 |
| configurar | configure | traduz | poph-server, poph | 24 |
| curso | course | traduz | poph-fluxos, poph-agent | 24 |
| habilitada | enabled | traduz | poph-agent, poph-jev | 24 |
| indisponivel | unavailable | traduz | poph-server, poph-matrix | 24 |
| instalacao | installation | traduz | poph, poph-rag | 24 |
| liberado | released | traduz | poph-server, poph-agent | 24 |
| livre | free | traduz | poph-server, poph-fluxos | 24 |
| manifestos | manifests | traduz | poph-server | 24 |
| recibo | receipt | traduz | poph-evolution, poph-fluxos | 24 |
| requisicao | request | traduz | poph-matrix, poph-core | 24 |
| rodar | run | traduz | poph-agent, poph | 24 |
| saude | health | traduz | poph-server, poph-evolution | 24 |
| simultaneos | simultaneous | traduz | poph-rag, raiz | 24 |
| sobrescrever | overwrite | traduz | poph-server, console | 24 |
| sonda | probe | traduz | poph-server | 24 |
| aceita | accepts | traduz | poph-fluxos, poph-server | 23 |
| aguardar | wait | traduz | poph-agent | 23 |
| autorizado | authorized | traduz | poph-server, console | 23 |
| baixa | low | traduz | poph-jev, poph-agent | 23 |
| complementos | complements | traduz | poph-server | 23 |
| consultas | queries | traduz | poph-rag, poph-server | 23 |
| cortar | cut | traduz | poph-agent, poph-matrix | 23 |
| demais | others | traduz | poph-fluxos, poph-evolution | 23 |
| dourada | golden | traduz | poph-fluxos | 23 |
| escritor | writer | traduz | poph-matrix, poph-plugins | 23 |
| fator | factor | traduz | poph-server, poph-agent | 23 |
| latencia | latency | traduz | poph-server, poph-jev | 23 |
| melhores | best | traduz | poph-rag | 23 |
| recuo | backoff | traduz | poph-fluxos, poph | 23 |
| reindexar | reindex | traduz | poph, poph-server | 23 |
| repeticao | repetition | traduz | poph-evolution, poph-agent | 23 |
| secao | section | traduz | poph-fluxos, poph | 23 |
| segura | safe | traduz | poph-agent, poph-server | 23 |
| situacoes | situations | traduz | console, poph-jev | 23 |
| travar | lock | traduz | poph-agent, poph-jev | 23 |
| validador | validator | traduz | poph-fluxos | 23 |
| visita | visit | traduz | poph-fluxos, poph-server | 23 |
| alfa | alpha | traduz | poph-agent, poph-fluxos | 22 |
| atendimentos | services | traduz | poph-fluxos, poph | 22 |
| chegada | arrival | traduz | poph-agent, poph-fluxos | 22 |
| combinador | combinator | traduz | poph-fluxos | 22 |
| continua | continues | traduz | poph-agent, poph-server | 22 |
| continuacao | continuation | traduz | poph-agent | 22 |
| deixa | leaves | traduz | poph-fluxos, poph-matrix | 22 |
| disfarce | disguise | traduz | poph-server, poph-matrix | 22 |
| diz | says | traduz | poph, poph-fluxos | 22 |
| dona | owner | traduz | poph-server | 22 |
| exclusao | exclusion | traduz | poph-server, poph | 22 |
| exibicao | display | traduz | poph-matrix, poph-fluxos | 22 |
| extracao | extraction | traduz | poph-files, poph-server | 22 |
| faltando | missing | traduz | poph-fluxos, poph-agent | 22 |
| final | final | traduz | poph-matrix, poph-fluxos | 22 |
| gerados | generated | traduz | poph-rag, console | 22 |
| gravacao | recording | traduz | poph-server, poph-matrix | 22 |
| inicia | starts | traduz | poph-agent, poph-rag | 22 |
| juntos | together | traduz | poph-server, poph-matrix | 22 |
| movida | moved | traduz | poph-matrix, poph-server | 22 |
| operacoes | operations | traduz | poph-server, poph | 22 |
| reais | actual | traduz | poph-agent, poph-server | 22 |
| remover | remove | traduz | poph-agent, poph-matrix | 22 |
| segue | follows | traduz | poph-server, poph-agent | 22 |
| semaforo | semaphore | traduz | poph-agent, poph-rag | 22 |
| unico | single | traduz | poph-server, poph-fluxos | 22 |
| apaga | deletes | traduz | poph-retention, poph-server | 21 |
| cargas | loads | traduz | poph-fluxos, poph-server | 21 |
| configurado | configured | traduz | console, poph-evolution | 21 |
| derivar | derive | traduz | poph-server, poph | 21 |
| dominio | domain | traduz | poph-server, poph-agent | 21 |
| dono | owner | traduz | poph-retention, poph-server | 21 |
| extraido | extracted | traduz | poph-files, poph-agent | 21 |
| extrator | extractor | traduz | poph-files, poph-server | 21 |
| fechado | closed | traduz | poph-server, poph-evolution | 21 |
| feito | done | traduz | poph-server, poph-retention | 21 |
| grupos | groups | traduz | poph, poph-server | 21 |
| importacao | import | traduz | poph, poph-server | 21 |
| inicial | initial | traduz | poph-agent, poph-server | 21 |
| malformado | malformed | traduz | poph-fluxos, poph-server | 21 |
| menor | smaller | traduz | poph-fluxos, poph-server | 21 |
| mim | me | traduz | poph-evolution, poph-server | 21 |
| minuscula | lowercase | traduz | poph-files, poph-server | 21 |
| novas | new | traduz | poph-server, poph-matrix | 21 |
| proprias | own | traduz | poph-agent, poph | 21 |
| quantos | how_many | traduz | poph-server, poph-fluxos | 21 |
| recebidas | received | traduz | poph-agent, poph-jev | 21 |
| repetir | repeat | traduz | poph-server, poph-matrix | 21 |
| respeita | respects | traduz | poph-agent, poph-server | 21 |
| segmento | segment | traduz | poph-server, poph-matrix | 21 |
| sim | yes | traduz | poph-fluxos, poph | 21 |
| velha | old | traduz | poph-server, poph | 21 |
| booleano | boolean | traduz | poph-fluxos, poph | 20 |
| definitivas | definitive | traduz | poph-agent, poph-server | 20 |
| disco | disk | traduz | poph-evolution, poph-server | 20 |
| entregue | delivered | traduz | poph-server, poph-agent | 20 |
| fala | speaks | traduz | poph-agent, poph-server | 20 |
| guardadas | guarded | traduz | poph-server, poph-fluxos | 20 |
| lugar | place | traduz | poph-fluxos, poph-server | 20 |
| mostrado | shown | traduz | poph, poph-server | 20 |
| portugues | portuguese | traduz | poph, poph-server | 20 |
| regras | rules | traduz | poph-fluxos, poph-matrix | 20 |
| secoes | sections | traduz | poph-agent | 20 |
| usa | uses | traduz | poph-agent, poph-matrix | 20 |
| virtuais | virtual | traduz | poph-matrix, poph-server | 20 |
| vitima | victim | traduz | poph-server, poph-fluxos | 20 |
| vivos | live | traduz | poph-agent, poph-jev | 20 |
| abaixo | below | traduz | poph-agent, poph-jev | 19 |
| alvos | target | traduz | poph-rag, poph-matrix | 19 |
| aspas | quotes | traduz | poph-fluxos, poph | 19 |
| atualizacoes | updates | traduz | poph-server, poph | 19 |
| conexoes | connections | traduz | poph-core, poph-jev | 19 |
| contrato | contract | traduz | poph-matrix, poph-jev | 19 |
| decorrido | elapsed | traduz | poph-agent, poph-files | 19 |
| imagens | images | traduz | poph-agent, poph-files | 19 |
| indexacao | indexing | traduz | poph-agent, poph-rag | 19 |
| informado | informed | traduz | poph-jev, poph-server | 19 |
| invalidas | invalid | traduz | poph-fluxos, poph-agent | 19 |
| propria | own | traduz | poph-server, poph-agent | 19 |
| quantizar | quantise | traduz | poph-rag | 19 |
| resolvedor | resolver | traduz | poph-agent | 19 |
| sufixo | suffix | traduz | poph-matrix, poph | 19 |
| tarefas | tasks | traduz | poph-server, poph-rag | 19 |
| transitorios | transient | traduz | poph-server, poph-matrix | 19 |
| vagas | slots | traduz | poph-rag, poph-agent | 19 |
| abstencao | abstention | traduz | poph-jev, poph-agent | 18 |
| achadas | found | traduz | poph-agent, poph-fluxos | 18 |
| adquirir | acquire | traduz | poph-agent | 18 |
| alterada | changed | traduz | poph-server | 18 |
| andamento | progress | traduz | poph-agent, poph-rag | 18 |
| avisadas | notified | traduz | poph-server | 18 |
| comercial | commercial | traduz | poph-server, poph-matrix | 18 |
| conferido | checked | traduz | poph, poph-evolution | 18 |
| convidado | guest | traduz | poph-server, poph | 18 |
| criterios | criteria | traduz | poph-jev, poph-agent | 18 |
| declarados | declared | traduz | poph-server, poph-fluxos | 18 |
| derrubar | drop | traduz | poph-server, poph-fluxos | 18 |
| digitos | digits | traduz | poph-fluxos, poph-matrix | 18 |
| dispositivo | device | traduz | poph-matrix, poph-evolution | 18 |
| enganosa | misleading | traduz | poph-files, poph-server | 18 |
| expulsoes | evictions | traduz | poph-fluxos, poph-server | 18 |
| limiar | threshold | traduz | poph-agent, poph-jev | 18 |
| ordenadas | sorted | traduz | poph-jev, poph-server | 18 |
| passos | steps | traduz | poph-server, poph-agent | 18 |
| periodo | period | traduz | poph-server | 18 |
| presenca | presence | traduz | poph-evolution, poph | 18 |
| quantas | how_many | traduz | poph-server, poph-fluxos | 18 |
| reabrir | reopen | traduz | poph-server, poph-agent | 18 |
| removidos | removed | traduz | poph-rag, poph | 18 |
| temporaria | temporary | traduz | poph | 18 |
| usados | used | traduz | poph-fluxos, poph-agent | 18 |
| agendar | schedule | traduz | poph-fluxos, poph-agent | 17 |
| amostras | samples | traduz | poph-server, poph-rag | 17 |
| ativas | active | traduz | poph-server, console | 17 |
| baixada | downloaded | traduz | poph-evolution, poph-server | 17 |
| candidata | candidate | traduz | poph-jev, poph-agent | 17 |
| coletada | collected | traduz | poph-fluxos | 17 |
| concluidos | finished | traduz | poph-agent, poph-rag | 17 |
| consultado | queried | traduz | poph-jev, poph-agent | 17 |
| cosseno | cosine | traduz | poph-rag | 17 |
| deixou | left | traduz | poph-fluxos | 17 |
| folha | sheet | traduz | poph-fluxos | 17 |
| guardados | guarded | traduz | poph-evolution, poph-server | 17 |
| hoje | today | traduz | poph-agent, poph-server | 17 |
| hora | hour | traduz | poph-fluxos, poph-server | 17 |
| instante | instant | traduz | poph-retention, poph-matrix | 17 |
| limpa | clean | traduz | poph-server, poph-rag | 17 |
| nomeavel | nameable | traduz | poph-server | 17 |
| paginas | pages | traduz | poph-server, poph-fluxos | 17 |
| pai | parent | traduz | poph-matrix, poph | 17 |
| pares | pairs | traduz | poph, poph-fluxos | 17 |
| perfis | profiles | traduz | poph-server | 17 |
| publicacao | publication | traduz | poph-server, poph | 17 |
| reenvios | resends | traduz | poph-agent, poph-server | 17 |
| renomeacoes | renames | traduz | poph-server | 17 |
| retomada | resumption | traduz | poph-agent, poph-matrix | 17 |
| simulacao | simulation | traduz | poph, poph-retention | 17 |
| tres | three | traduz | poph-fluxos, poph-jev | 17 |
| vezes | times | traduz | poph-server, poph-fluxos | 17 |
| alterados | changed | traduz | poph-rag, console | 16 |
| atualizadas | updated | traduz | console, poph-server | 16 |
| certificado | certificate | traduz | poph-core, poph-jev | 16 |
| desconhecida | unknown | traduz | poph-server, console | 16 |
| duplicadas | duplicated | traduz | poph-server, poph-agent | 16 |
| encerramento | closure | traduz | poph-agent, poph-server | 16 |
| escolhido | chosen | traduz | poph-agent, poph-server | 16 |
| espelhadas | mirrored | traduz | poph-server | 16 |
| esquecer | forget | traduz | poph-server, poph-agent | 16 |
| estava | was | traduz | poph-rag, poph-matrix | 16 |
| humano | human | traduz | poph-server, poph-matrix | 16 |
| inalterados | unchanged | traduz | poph-rag, console | 16 |
| inativo | inactive | traduz | poph, poph-server | 16 |
| isolamento | isolation | traduz | poph-server, poph-files | 16 |
| medir | measure | traduz | poph-server, poph-jev | 16 |
| nota | note | traduz | poph-rag, poph-agent | 16 |
| notificacao | notification | traduz | poph-fluxos, poph-server | 16 |
| passa | passes | traduz | poph-fluxos, poph-server | 16 |
| permitido | allowed | traduz | poph-server, poph-fluxos | 16 |
| pessoa | person | traduz | poph-matrix, poph-server | 16 |
| pico | peak | traduz | poph-rag, poph-files | 16 |
| recarregar | reload | traduz | poph-server, poph-agent | 16 |
| regenerada | regenerated | traduz | poph-agent | 16 |
| remetentes | senders | traduz | poph-server | 16 |
| rotulado | labelled | traduz | poph-jev, poph-rag | 16 |
| seguidas | consecutive | traduz | poph-server | 16 |
| simular | simulate | traduz | poph-retention | 16 |
| sintaxe | syntax | traduz | poph-fluxos, poph | 16 |
| tratar | handle | traduz | poph-server, poph-agent | 16 |
| ver | see | traduz | poph-server | 16 |
| abertura | opening | traduz | poph-server, poph | 15 |
| alheio | foreign | traduz | poph-server, poph-agent | 15 |
| alteradas | changed | traduz | poph-server | 15 |
| alterado | changed | traduz | poph-server, poph | 15 |
| apagados | deleted | traduz | poph-retention, poph | 15 |
| auditorias | audits | traduz | poph-retention, poph-server | 15 |
| capturado | captured | traduz | poph-matrix, poph-server | 15 |
| celulas | cells | traduz | poph-server, poph-evolution | 15 |
| curto | short | traduz | poph-matrix, poph-server | 15 |
| destinos | targets | traduz | poph-fluxos, poph-rag | 15 |
| embutido | embedded | traduz | poph-rag, poph-evolution | 15 |
| encerradas | closed | traduz | poph-agent, poph-server | 15 |
| escalares | scalars | traduz | poph | 15 |
| espelhada | mirrored | traduz | poph-matrix, poph-server | 15 |
| estavel | stable | traduz | poph, poph-matrix | 15 |
| estimar | estimate | traduz | poph-agent | 15 |
| expirar | expire | traduz | poph-matrix, poph-server | 15 |
| exportar | export | traduz | poph, poph-server | 15 |
| filhos | children | traduz | poph-fluxos, poph-server | 15 |
| guardar | keep | traduz | poph-evolution, poph-server | 15 |
| ilegivel | unreadable | traduz | poph-evolution, poph-matrix | 15 |
| inseridas | inserted | traduz | console, poph-server | 15 |
| latencias | latencies | traduz | poph-jev, poph-agent | 15 |
| ligacao | link | traduz | poph | 15 |
| meio | middle | traduz | poph-server, poph-agent | 15 |
| orfao | orphan | traduz | poph-retention | 15 |
| redigidas | drafted | traduz | poph-server | 15 |
| redigir | draft | traduz | poph-matrix, poph-agent | 15 |
| registrada | recorded | traduz | poph-server, poph-agent | 15 |
| renomear | rename | traduz | poph-server, poph-files | 15 |
| resolucao | resolution | traduz | poph-fluxos, poph | 15 |
| suporte | support | traduz | poph-matrix, poph-server | 15 |
| temporario | temporary | traduz | poph-files, poph | 15 |
| terceiro | third | traduz | poph-server, poph-agent | 15 |
| topo | top | traduz | poph-fluxos | 15 |
| trocado | swapped | traduz | poph-server, poph-matrix | 15 |
| truncar | truncate | traduz | poph-jev, poph | 15 |
| agir | act | traduz | poph-server, poph-agent | 14 |
| apertado | tight | traduz | poph-server, poph-fluxos | 14 |
| assinados | signed | traduz | poph-server | 14 |
| colecao | collection | traduz | poph-server | 14 |
| comum | common | traduz | poph-agent, poph-rag | 14 |
| conflitos | conflicts | traduz | poph-server, poph | 14 |
| convidar | invite | traduz | poph-matrix | 14 |
| cria | creates | traduz | poph, poph-matrix | 14 |
| deixar | leave | traduz | poph-agent, poph-core | 14 |
| desconhecidas | unknown | traduz | poph-server | 14 |
| esgotadas | exhausted | traduz | poph-server, poph-agent | 14 |
| guardado | stored | traduz | poph-server, poph-agent | 14 |
| leituras | reads | traduz | poph-server, poph-agent | 14 |
| nas | in_the | traduz | poph-server, poph-agent | 14 |
| paginacao | pagination | traduz | poph-server | 14 |
| pontuacao | score | traduz | poph-rag | 14 |
| poupadas | saved | traduz | poph-fluxos, poph-server | 14 |
| processadas | processed | traduz | poph-server | 14 |
| rapidas | fast | traduz | poph-agent | 14 |
| recente | recent | traduz | poph-retention, poph-server | 14 |
| segmentos | segments | traduz | poph-server | 14 |
| telefone | phone | traduz | poph-matrix, poph-server | 14 |
| vazamento | leak | traduz | poph-server | 14 |
| condicoes | conditions | traduz | poph-agent, console | 13 |
| construir | build | traduz | poph-server, poph | 13 |
| direcao | direction | traduz | poph-agent, poph-server | 13 |
| esperadas | expected | traduz | poph-server, poph | 13 |
| esta | this | traduz | poph-rag, poph-server | 13 |
| estatisticas | statistics | traduz | poph-agent, poph-server | 13 |
| gerado | generated | traduz | poph, poph-server | 13 |
| instalado | installed | traduz | poph-rag, poph-server | 13 |
| marcar | mark | traduz | poph-matrix, poph-server | 13 |
| passagem | passage | traduz | poph-server, poph-matrix | 13 |
| percentil | percentile | traduz | poph-jev, poph-rag | 13 |
| recusados | refused | traduz | poph-agent, poph-server | 13 |
| termo | term | traduz | poph-agent, poph-rag | 13 |
| varrer | scan | traduz | poph-retention, poph-rag | 13 |
| versoes | versions | traduz | poph-server, poph | 13 |
| vinculo | link | traduz | poph-server, poph | 13 |
| vistas | views | traduz | poph-server, poph-fluxos | 13 |
| adicionar | add | traduz | poph-agent, poph-rag | 12 |
| apenas | only | traduz | poph-jev, poph-agent | 12 |
| aplicar | apply | traduz | poph, poph-agent | 12 |
| concluido | finished | traduz | poph-agent | 12 |
| confirmacao | confirmation | traduz | poph-agent | 12 |
| criadas | created | traduz | poph-server | 12 |
| degradada | degraded | traduz | console | 12 |
| disposicao | layout | traduz | poph-server | 12 |
| dividir | split | traduz | poph-rag | 12 |
| executada | executed | traduz | poph-agent | 12 |
| expirada | expired | traduz | poph-evolution, poph-server | 12 |
| idade | age | traduz | poph-retention, poph | 12 |
| limpar | clean | traduz | poph-agent, poph-fluxos | 12 |
| pode | can | traduz | poph, poph-agent | 12 |
| precisa | needs | traduz | poph-fluxos, poph-jev | 12 |
| tentar | try | traduz | poph-matrix, poph-evolution | 12 |
| traco | trace | traduz | poph-server, poph | 12 |
| transferida | transferred | traduz | poph-server, poph-agent | 12 |
| vencidas | expired | traduz | poph-server, poph-jev | 12 |
| acumulado | accumulated | traduz | poph-matrix, poph-rag | 11 |
| adicionadas | added | traduz | console, poph-server | 11 |
| antigas | old | traduz | poph-matrix, poph-retention | 11 |
| apagado | deleted | traduz | poph, poph-agent | 11 |
| capacidades | capabilities | traduz | poph-jev, poph-agent | 11 |
| carrinhos | carts | traduz | console, poph-agent | 11 |
| comentarios | comments | traduz | poph-fluxos, poph | 11 |
| declara | declares | traduz | poph-server, poph | 11 |
| edicao | edit | traduz | poph-matrix, poph-server | 11 |
| esgotado | exhausted | traduz | poph-agent, poph-jev | 11 |
| filas | queues | traduz | poph-agent | 11 |
| iniciado | started | traduz | poph-agent, poph-retention | 11 |
| instalar | install | traduz | poph-rag, poph | 11 |
| mais | more | traduz | poph-server, poph-agent | 11 |
| mediana | median | traduz | poph-agent, poph-server | 11 |
| metrica | metric | traduz | poph-server | 11 |
| motivos | reasons | traduz | poph-retention, poph-matrix | 11 |
| mudancas | changes | traduz | poph-server, poph | 11 |
| parado | stopped | traduz | poph-fluxos, poph | 11 |
| preenchidos | filled | traduz | console, poph-server | 11 |
| removidas | removed | traduz | console, poph-server | 11 |
| trocaram | swapped | traduz | console, poph-server | 11 |
| acento | accent | traduz | poph-rag, poph-fluxos | 10 |
| celula | cell | traduz | poph-files, poph-evolution | 10 |
| comandos | commands | traduz | poph-plugins | 10 |
| concluida | finished | traduz | poph-fluxos, poph-server | 10 |
| concorrencia | concurrency | traduz | poph-matrix, poph-server | 10 |
| criando | creating | traduz | poph-matrix, poph-server | 10 |
| decididas | decided | traduz | poph-jev, console | 10 |
| desconectar | disconnect | traduz | poph-server, console | 10 |
| desde | since | traduz | poph, poph-server | 10 |
| direto | direct | traduz | poph-server, poph | 10 |
| duplicados | duplicated | traduz | poph-agent, poph-plugin-eco | 10 |
| enviando | sending | traduz | poph-agent | 10 |
| finalizada | finalised | traduz | poph-store, poph-agent | 10 |
| medicao | measurement | traduz | poph-agent, scripts | 10 |
| publicado | published | traduz | poph-server, poph-matrix | 10 |
| seguro | safe | traduz | poph-matrix, poph-server | 10 |
| trabalho | work | traduz | poph-fluxos | 10 |
| unicos | single | traduz | poph-fluxos, poph-jev | 10 |
| verificar | verify | traduz | poph, poph-server | 10 |
| apagou | deleted | traduz | poph-matrix, poph-server | 9 |
| aplica | applies | traduz | poph-jev, poph-matrix | 9 |
| caminhos | paths | traduz | poph-fluxos, poph-server | 9 |
| corpos | bodies | traduz | poph-server | 9 |
| falharam | failed | traduz | poph-agent, poph-plugin-eco | 9 |
| forcado | forced | traduz | poph-server, poph | 9 |
| listagem | listing | traduz | poph-server, poph-agent | 9 |
| metodos | methods | traduz | poph-server | 9 |
| minimo | minimum | traduz | poph-server, poph-fluxos | 9 |
| negativo | negative | traduz | poph-fluxos, poph-retention | 9 |
| pedacos | pieces | traduz | poph-rag, poph-server | 9 |
| previa | preview | traduz | poph-server | 9 |
| prontos | ready | traduz | poph-matrix | 9 |
| alcancados | reached | traduz | poph-fluxos | 8 |
| atuais | current | traduz | poph-rag, poph-agent | 8 |
| automatico | automatic | traduz | poph-matrix, poph-server | 8 |
| capturas | captures | traduz | poph-evolution | 8 |
| entrou | entered | traduz | poph-matrix, poph-store | 8 |
| recurso | resource | traduz | poph-server | 8 |
| terminar | finish | traduz | poph-server | 8 |
| tocar | touch | traduz | poph-server, poph-fluxos | 8 |
| aninhado | nested | traduz | poph-jev, poph-server | 7 |
| antigos | old | traduz | poph-matrix, poph-agent | 7 |
| apagada | deleted | traduz | poph-evolution, poph-matrix | 7 |
| avancou | advanced | traduz | poph-fluxos, poph-agent | 7 |
| controle | control | traduz | poph-server, poph-matrix | 7 |
| idempotente | idempotent | traduz | poph, poph-agent | 7 |
| invalidos | invalid | traduz | poph-agent, poph-fluxos | 7 |
| lixo | garbage | traduz | poph-plugins | 7 |
| sensivel | sensitive | traduz | poph, poph-agent | 7 |
| sistema | system | traduz | poph-server, poph-retention | 7 |
| subida | rise | traduz | poph-server, poph-rag | 7 |
| tudo | everything | traduz | poph-server, poph-agent | 7 |
| vinculado | linked | traduz | poph, poph-server | 7 |
| voltou | returned | traduz | poph-fluxos, poph | 7 |
| cifrados | encrypted | traduz | poph | 6 |
| colunas | columns | traduz | poph, poph-retention | 6 |
| nenhuma | none | traduz | poph-fluxos, poph-server | 6 |
| normalizar | normalise | traduz | poph, poph-rag | 6 |
| opcional | optional | traduz | poph-agent, poph-server | 6 |
| propriedades | properties | traduz | poph-agent, poph-server | 6 |
| recarga | reload | traduz | poph-server | 6 |
| retirar | withdraw | traduz | poph | 6 |
| sair | exit | traduz | poph-rag, poph-agent | 6 |
| vincular | link | traduz | poph, poph-server | 6 |
| confirmar | confirm | traduz | poph-server | 5 |
| duplicado | duplicate | traduz | poph, poph-server | 5 |
| encerrado | closed | traduz | poph-fluxos, poph-agent | 5 |
| este | this | traduz | poph-agent | 5 |
| faixa | range | traduz | poph, poph-server | 5 |
| geracao | generation | traduz | poph-rag | 5 |
| inclui | includes | traduz | poph-agent | 5 |
| linguagem | language | traduz | poph-fluxos | 5 |
| migracao | migration | traduz | poph | 5 |
| ocupada | busy | traduz | poph-agent | 5 |
| pessoas | people | traduz | poph-matrix | 5 |
| programa | program | traduz | poph-files, poph-server | 5 |
| pronto | ready | traduz | poph-agent, poph-evolution | 5 |
| remota | remote | traduz | poph-server | 5 |
| revogado | revoked | traduz | poph, poph-server | 5 |
| revogar | revoke | traduz | poph-server, poph | 5 |
| abandonada | abandoned | traduz | poph-fluxos, poph-server | 4 |
| agrupamento | grouping | traduz | poph-agent | 4 |
| alterar | modify | traduz | poph, poph-fluxos | 4 |
| aplicada | applied | traduz | poph-agent | 4 |
| caixa | box | traduz | poph-fluxos, poph-agent | 4 |
| cancelado | cancelled | traduz | poph-fluxos | 4 |
| contato | contact | traduz | poph-agent, poph-evolution | 4 |
| derivado | derived | traduz | poph, poph-server | 4 |
| digitando | typing | traduz | poph-evolution | 4 |
| escolher | choose | traduz | poph-server | 4 |
| filtros | filters | traduz | poph-server | 4 |
| ida | trip | traduz | poph-fluxos, poph-server | 4 |
| mudar | change | traduz | poph-agent, poph-core | 4 |
| problema | problem | traduz | raiz, poph | 4 |
| visivel | visible | traduz | poph-agent, poph-matrix | 4 |
| atualizada | updated | traduz | poph-server, poph-store | 3 |
| autor | author | traduz | poph-fluxos | 3 |
| capturar | capture | traduz | poph-server | 3 |
| conclusao | completion | traduz | poph-server, poph-agent | 3 |
| encontrada | found | traduz | poph-agent | 3 |
| fechamento | closing | traduz | poph-server, poph-fluxos | 3 |
| finalizar | finalise | traduz | poph-matrix | 3 |
| habilitado | enabled | traduz | poph-agent, poph | 3 |
| livres | free | traduz | poph-fluxos | 3 |
| organizacao | organisation | traduz | poph-fluxos | 3 |
| ponto | point | traduz | poph-agent, poph-server | 3 |
| principio | principle | traduz | poph-agent | 3 |
| prontas | ready | traduz | poph-server | 3 |
| recuperados | recovered | traduz | poph-agent | 3 |
| substituir | replace | traduz | poph-server | 3 |
| vinculos | links | traduz | poph-server | 3 |
| combina | combines | traduz | poph-matrix | 2 |
| consertar | repair | traduz | poph-server | 2 |
| descartada | discarded | traduz | poph-evolution | 2 |
| editar | edit | traduz | poph-agent, poph-rag | 2 |
| equipe | team | traduz | poph-fluxos, poph-server | 2 |
| exportacao | export | traduz | poph-server | 2 |
| frente | front | traduz | poph-rag, poph-server | 2 |
| geral | general | traduz | poph-agent, poph-retention | 2 |
| largura | width | traduz | poph-fluxos | 2 |
| leitores | readers | traduz | poph-files | 2 |
| quadro | board | traduz | poph-evolution | 2 |
| reiniciar | restart | traduz | poph-plugins, poph-server | 2 |
| alteracao | change | traduz | poph | 1 |
| arquivado | archived | traduz | poph-agent | 1 |
| correcao | fix | traduz | poph-matrix | 1 |
| desabilitado | disabled | traduz | poph-agent | 1 |
| dica | hint | traduz | poph-fluxos | 1 |
| digitado | typed | traduz | poph-agent | 1 |
| explicacao | explanation | traduz | poph-agent | 1 |
| expor | expose | traduz | poph-server | 1 |
| iniciando | starting | traduz | poph-evolution | 1 |
| migracoes | migrations | traduz | poph | 1 |
| minutos | minutes | traduz | scripts | 1 |
| organizacoes | organisations | traduz | poph-server | 1 |
| painel | panel | traduz | poph-agent | 1 |
| porque | because | traduz | poph-evolution | 1 |
| processado | processed | traduz | poph-agent | 1 |
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
