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
| texto | text | traduz | poph-agent, poph-server | 3667 |
| estado | state | traduz | poph-server, poph-fluxos | 3057 |
| erro | error | traduz | poph-server, poph | 2976 |
| conversa | conversation | traduz | poph-server, poph-agent | 2670 |
| agente | agent | traduz | poph-server, poph-agent | 2665 |
| cenario | scenario | traduz | poph-server, poph | 2664 |
| nome | name | traduz | poph-server, poph-fluxos | 2121 |
| corpo | body | traduz | poph-server, poph-matrix | 1585 |
| sala | room | traduz | poph-server, poph-matrix | 1538 |
| de | of | traduz | poph-server, poph | 1515 |
| fluxo | flow | traduz | poph-agent, poph-server | 1364 |
| novo | new | traduz | poph-server, poph-fluxos | 1248 |
| com | with | traduz | poph-server, poph-agent | 1239 |
| resposta | response | traduz | poph-server, poph-matrix | 1216 |
| resultado | result | traduz | poph-store, poph-fluxos | 1192 |
| da | of | traduz | poph-server, poph-matrix | 1151 |
| saida | output | traduz | poph, poph-server | 1142 |
| tipo | kind | traduz | poph-fluxos, poph-server | 1130 |
| mensagem | message | traduz | poph-server, poph-agent | 1126 |
| cliente | client | traduz | poph-server, poph-matrix | 1106 |
| motivo | reason | traduz | poph-server, poph-matrix | 1071 |
| dados | data | traduz | poph-matrix, poph-server | 1057 |
| valor | value | traduz | poph-server, poph-fluxos | 1039 |
| chave | key | traduz | poph-server, poph-fluxos | 1031 |
| evento | event | traduz | console, poph-server | 1028 |
| caminho | path | traduz | poph-server, poph | 1023 |
| pasta | folder | traduz | poph, poph-server | 1023 |
| campo | field | traduz | poph-fluxos, console | 983 |
| acao | action | traduz | poph-fluxos, poph | 969 |
| turno | turn | traduz | poph-agent, poph-server | 834 |
| etapa | step | traduz | poph-fluxos, poph-server | 806 |
| departamento | department | traduz | poph-server, poph | 770 |
| registro | record | traduz | poph-agent, poph-server | 763 |
| sem | without | traduz | poph-server, poph-agent | 736 |
| servidor | server | traduz | console, poph-server | 718 |
| criar | create | traduz | poph-server, poph | 698 |
| arquivo | file | traduz | poph, poph-server | 682 |
| falha | failure | traduz | poph-server, poph | 634 |
| nao | not | traduz | poph-server, poph-matrix | 592 |
| espaco | space | traduz | poph-matrix, poph-server | 587 |
| em | in | traduz | poph-server, poph-fluxos | 576 |
| ficha | case | traduz | poph-server, poph-agent | 568 |
| para | for | traduz | poph-fluxos, poph-server | 537 |
| destino | target | traduz | poph-server, poph-fluxos | 533 |
| decisao | decision | traduz | poph-fluxos, poph-jev | 532 |
| pedir | ask | traduz | poph-server, poph | 527 |
| midia | media | traduz | poph-matrix, poph-server | 525 |
| alvo | target | traduz | poph-server, poph-fluxos | 518 |
| metricas | metrics | traduz | poph-server, poph-fluxos | 492 |
| gatilho | trigger | traduz | poph-agent, poph-fluxos | 488 |
| versao | version | traduz | poph-server, console | 484 |
| fonte | source | traduz | poph-fluxos, poph-server | 481 |
| porta | port | traduz | poph-server, poph-matrix | 463 |
| codigo | code | traduz | poph, poph-server | 450 |
| canal | channel | traduz | poph-server, poph-store | 449 |
| fim | end | traduz | poph-fluxos, poph-agent | 428 |
| lista | list | traduz | poph-server, poph-fluxos | 416 |
| ambiente | environment | traduz | poph, poph-matrix | 415 |
| itens | items | traduz | poph-server, poph-fluxos | 395 |
| chamada | call | traduz | poph-server, poph-matrix | 388 |
| integracao | integration | traduz | poph-server, console | 388 |
| linha | line | traduz | poph-server, poph | 381 |
| aviso | notice | traduz | poph-server, poph | 371 |
| provedor | provider | traduz | poph-agent, poph-server | 366 |
| falso | false | traduz | poph-server, poph-matrix | 363 |
| relatorio | report | traduz | poph-server, poph | 345 |
| caso | case | traduz | poph-server, poph-fluxos | 343 |
| fluxos | flows | traduz | poph-server, poph-agent | 339 |
| instancia | instance | traduz | poph-server, poph-evolution | 339 |
| atual | current | traduz | poph-server, poph-fluxos | 333 |
| ator | actor | traduz | poph-store, poph-fluxos | 327 |
| configuracao | configuration | traduz | poph, console | 326 |
| desligar | disable | traduz | poph-server, poph | 324 |
| campos | fields | traduz | poph-fluxos, poph-server | 322 |
| limites | limits | traduz | poph-fluxos, poph-server | 316 |
| situacao | situation | traduz | console, poph-retention | 316 |
| modelo | model | traduz | poph-agent, poph-server | 314 |
| grupo | group | traduz | poph-server, poph | 300 |
| motor | engine | traduz | poph-agent, poph-server | 297 |
| salas | rooms | traduz | poph-server, poph-matrix | 292 |
| origem | origin | traduz | poph, poph-matrix | 280 |
| nova | new | traduz | poph-fluxos, poph-server | 279 |
| metodo | method | traduz | poph-server, poph-matrix | 270 |
| revisao | revision | traduz | poph-server, poph-fluxos | 269 |
| vazio | empty | traduz | poph-server, poph-fluxos | 269 |
| numero | number | traduz | console, poph-store | 267 |
| pedido | request | traduz | poph-server, poph-agent | 267 |
| ao | to | traduz | poph-server, poph-matrix | 260 |
| guarda | guard | traduz | poph-matrix, poph-server | 259 |
| rota | route | traduz | poph-server, poph-fluxos | 258 |
| whatsapp | whatsapp | traduz | console, poph-server | 258 |
| regra | rule | traduz | poph-fluxos, poph-server | 256 |
| inicio | beginning | traduz | poph-server, poph-fluxos | 248 |
| identidade | identity | traduz | poph-server, poph | 241 |
| contexto | context | traduz | poph-fluxos, poph | 240 |
| mudanca | change | traduz | poph-fluxos, poph | 240 |
| teto | cap | traduz | poph-server, poph-fluxos | 240 |
| efeitos | effects | traduz | poph-server, poph-fluxos | 239 |
| execucao | execution | traduz | poph-server, poph-store | 239 |
| segredo | secret | traduz | poph-server, poph | 235 |
| rotulo | label | traduz | poph-server, console | 232 |
| conteudo | content | traduz | poph-matrix, poph-server | 231 |
| efeito | effect | traduz | poph-fluxos, poph-server | 231 |
| por | by | traduz | poph-agent, poph-server | 229 |
| espacos | spaces | traduz | poph-server, poph-matrix | 228 |
| fantasma | ghost | traduz | poph-matrix, poph-server | 228 |
| segredos | secrets | traduz | poph, console | 227 |
| categoria | category | traduz | poph, poph-server | 226 |
| validar | validate | traduz | poph-server, poph-types | 225 |
| envio | send | traduz | poph-server, poph-matrix | 223 |
| responder | reply | traduz | poph-server, poph-matrix | 223 |
| pagina | page | traduz | poph-server, poph | 222 |
| outro | other | traduz | poph-server, poph | 213 |
| modulo | module | traduz | poph-server, poph-fluxos | 209 |
| na | towards | traduz | poph-server, poph-matrix | 208 |
| eventos | events | traduz | poph-server, poph-fluxos | 206 |
| executar | run | traduz | poph-fluxos, poph | 202 |
| ciclo | cycle | traduz | poph-server, poph-fluxos | 200 |
| relato | report | traduz | poph-server | 199 |
| antes | before | traduz | poph-server, poph-matrix | 197 |
| falar | speak | traduz | poph-agent, poph-server | 197 |
| produto | product | traduz | poph, poph-server | 197 |
| avaliador | evaluator | traduz | poph-fluxos, poph-server | 192 |
| depois | after | traduz | poph-server, console | 189 |
| mapa | map | traduz | poph-server, poph-fluxos | 189 |
| descricao | description | traduz | console, poph-agent | 188 |
| entrada | entry | traduz | poph-fluxos, poph-server | 188 |
| consulta | query | traduz | poph-server, poph-matrix | 186 |
| cru | raw | traduz | poph-server, poph-agent | 184 |
| externo | external | traduz | poph-server, poph-agent | 183 |
| agentes | agents | traduz | poph, poph-server | 181 |
| modo | mode | traduz | poph-server, console | 180 |
| falhas | failures | traduz | poph-server, poph-fluxos | 177 |
| que | that | traduz | poph-fluxos, poph-server | 176 |
| remetente | sender | traduz | poph-matrix, poph-server | 176 |
| subir | raise | traduz | poph-server, poph-matrix | 176 |
| cabecalhos | headers | traduz | poph-server, poph-matrix | 175 |
| conversas | conversations | traduz | poph-server, poph-store | 174 |
| iniciar | start | traduz | poph-fluxos, poph-server | 174 |
| esperar | wait | traduz | poph-server, poph-agent | 173 |
| primeira | first | traduz | poph-server, poph-fluxos | 172 |
| registros | records | traduz | poph-server, poph-matrix | 172 |
| gravada | stored | traduz | poph-server, poph-matrix | 170 |
| indice | index | traduz | poph-server, poph-fluxos | 170 |
| falsa | false | traduz | poph-server, poph-agent | 169 |
| gravado | stored | traduz | poph-server, poph-matrix | 168 |
| impacto | impact | traduz | poph-fluxos | 164 |
| fantasmas | ghosts | traduz | poph-matrix, poph-server | 160 |
| invalido | invalid | traduz | poph-server, poph-matrix | 160 |
| opcoes | options | traduz | poph-server, poph-fluxos | 160 |
| pedidos | requests | traduz | poph-agent, poph-server | 159 |
| enviar | send | traduz | poph-server, poph-matrix | 158 |
| anexo | attachment | traduz | poph-server, poph-agent | 157 |
| nomes | names | traduz | poph-fluxos, poph-server | 157 |
| detalhe | detail | traduz | poph, poph-server | 156 |
| transacao | transaction | traduz | poph-server, poph-matrix | 156 |
| endereco | address | traduz | poph-server, poph-matrix | 152 |
| resumo | summary | traduz | poph-server, poph | 150 |
| ordem | order | traduz | poph-server, console | 149 |
| limite | limit | traduz | poph-server, poph | 147 |
| usuarios | users | traduz | poph-matrix, poph-server | 147 |
| mudou | changed | traduz | poph-fluxos, poph-server | 146 |
| erros | errors | traduz | poph-fluxos, poph-server | 145 |
| entrega | delivery | traduz | poph-server, poph-agent | 143 |
| listar | list | traduz | poph, poph-server | 143 |
| segunda | second | traduz | poph-server, poph-agent | 143 |
| atores | actors | traduz | poph-fluxos, poph-agent | 141 |
| avisos | notices | traduz | poph-server, poph-fluxos | 141 |
| espelho | mirror | traduz | poph-server, poph-matrix | 141 |
| teste | test | traduz | poph, poph-server | 140 |
| recusa | refusal | traduz | poph-server, poph | 139 |
| mensagens | messages | traduz | poph-server, poph-matrix | 138 |
| roteiro | script | traduz | poph-agent, poph-server | 136 |
| padrao | default | traduz | poph-agent, poph-server | 132 |
| abrir | open | traduz | poph, poph-server | 131 |
| outra | other | traduz | poph-server, poph-agent | 131 |
| diagnosticos | diagnostics | traduz | poph-fluxos, poph-server | 130 |
| espera | wait | traduz | poph-agent, poph-server | 130 |
| registrar | register | traduz | poph-server, poph-matrix | 129 |
| respostas | responses | traduz | poph-server, poph-agent | 129 |
| manifesto | manifest | traduz | poph-server, console | 128 |
| pausa | pause | traduz | poph-agent, poph-matrix | 128 |
| turnos | turns | traduz | poph-agent, poph-server | 128 |
| partes | parts | traduz | poph-server, poph-matrix | 126 |
| primeiro | first | traduz | poph-server, poph-fluxos | 125 |
| confere | checks | traduz | poph-fluxos, poph-server | 124 |
| posicao | position | traduz | poph-matrix, poph-server | 124 |
| chamadas | calls | traduz | poph-server, poph-matrix | 122 |
| tentativas | attempts | traduz | poph-server, poph-agent | 122 |
| sinal | signal | traduz | poph-server, poph | 121 |
| usuario | user | traduz | poph-matrix, poph-server | 121 |
| envios | sends | traduz | poph-server, poph-agent | 120 |
| leitor | reader | traduz | poph-fluxos, poph | 120 |
| esperado | expected | traduz | poph, poph-server | 119 |
| carregar | load | traduz | poph, poph-fluxos | 118 |
| classificacao | classification | traduz | poph-matrix, poph-server | 118 |
| compilado | compiled | traduz | poph-fluxos, poph-server | 118 |
| ultimo | last | traduz | poph-server, poph-agent | 116 |
| uma | a | traduz | poph-server, poph-fluxos | 113 |
| vai | goes | traduz | poph-fluxos, raiz | 112 |
| exigir | require | traduz | poph-server, poph | 111 |
| ler | read | traduz | poph, poph-matrix | 110 |
| arquivos | files | traduz | poph-server, poph | 109 |
| desfecho | outcome | traduz | poph-server, poph-agent | 109 |
| membros | members | traduz | poph-server, poph-matrix | 109 |
| prefixo | prefix | traduz | poph-server, poph-matrix | 108 |
| ignorar | ignore | traduz | poph-matrix, poph-server | 107 |
| nivel | level | traduz | poph, poph-matrix | 107 |
| auditoria | audit | traduz | poph-server, poph-agent | 105 |
| agora | now | traduz | poph-server, poph-fluxos | 103 |
| bloco | block | traduz | poph-fluxos, poph-matrix | 102 |
| fechada | closed | traduz | poph-store, poph-server | 101 |
| atendente | attendant | traduz | poph-server, poph-matrix | 100 |
| imprimir | print | traduz | poph, poph-fluxos | 100 |
| lido | read | traduz | poph, poph-server | 100 |
| par | pair | traduz | poph-server, poph | 100 |
| escritas | writings | traduz | poph-server, poph-matrix | 99 |
| criado | created | traduz | poph-server, poph | 98 |
| definir | define | traduz | poph-server, poph-agent | 98 |
| etapas | steps | traduz | poph-fluxos, poph-server | 97 |
| fila | queue | traduz | poph-server, poph-agent | 97 |
| aparelho | device | traduz | poph-server, poph-matrix | 96 |
| mostrar | show | traduz | poph-server, poph | 96 |
| tamanho | size | traduz | poph-matrix, poph-server | 96 |
| captura | capture | traduz | poph-server, poph | 95 |
| das | of | traduz | poph-server, poph-agent | 95 |
| atualizar | update | traduz | poph-server, poph | 94 |
| descrever | describe | traduz | poph, poph-fluxos | 94 |
| instrucoes | instructions | traduz | poph, poph-server | 94 |
| receptor | receiver | traduz | poph-server | 93 |
| catalogo | catalog | traduz | poph, poph-server | 92 |
| textos | texts | traduz | poph-server, poph-agent | 92 |
| fora | outside | traduz | poph-server, poph-agent | 89 |
| grande | large | traduz | poph-server, poph-matrix | 89 |
| interno | internal | traduz | poph-server, poph-fluxos | 86 |
| conexao | connection | traduz | poph-server, poph | 85 |
| formato | format | traduz | poph-matrix, poph-store | 85 |
| pronta | ready | traduz | poph-matrix, poph-server | 85 |
| relogio | clock | traduz | poph-server, poph-fluxos | 85 |
| anexos | attachments | traduz | poph-server, poph-agent | 84 |
| mundo | world | traduz | poph-server | 84 |
| resto | rest | traduz | poph-server, poph | 83 |
| atendimento | service | traduz | poph-fluxos, poph-server | 82 |
| validacao | validation | traduz | poph-server, poph | 82 |
| vivo | live | traduz | poph, poph-server | 82 |
| funcao | function | traduz | poph-fluxos, raiz | 80 |
| invalida | invalid | traduz | poph-fluxos, poph-server | 79 |
| desligado | disabled | traduz | poph-server, poph | 78 |
| duracao | duration | traduz | poph-server, poph-agent | 78 |
| sessao | session | traduz | poph, console | 78 |
| documento | document | traduz | poph-server, poph | 77 |
| servir | serve | traduz | poph-server, poph | 77 |
| aberta | open | traduz | poph-server, poph-fluxos | 76 |
| acoes | actions | traduz | poph-fluxos, raiz | 76 |
| canais | channels | traduz | poph-server, poph | 76 |
| ligado | linked | traduz | poph-server, poph | 76 |
| midias | media | traduz | poph-server | 76 |
| pendentes | pending | traduz | poph-server, poph-fluxos | 76 |
| autorizacao | authorization | traduz | poph-matrix, poph-server | 75 |
| bruto | raw | traduz | poph-fluxos, poph-server | 75 |
| instancias | instances | traduz | poph-server, console | 75 |
| ligar | link | traduz | poph-server, poph | 75 |
| linhas | lines | traduz | poph, poph-server | 75 |
| muda | changes | traduz | poph-fluxos, poph-agent | 75 |
| onde | where | traduz | poph-fluxos, poph | 75 |
| variavel | variable | traduz | poph, poph-fluxos | 75 |
| expiracao | expiry | traduz | poph-server, poph-matrix | 74 |
| igual | equal | traduz | poph-fluxos, poph-server | 74 |
| operacao | operation | traduz | poph-matrix, poph-server | 74 |
| recusado | refused | traduz | poph-server, poph | 74 |
| tem | has | traduz | poph, poph-server | 74 |
| busca | search | traduz | poph-agent, poph | 73 |
| casos | cases | traduz | poph-fluxos, poph-agent | 73 |
| comando | command | traduz | poph, poph-server | 73 |
| como | as | traduz | poph, poph-fluxos | 73 |
| ja | already | traduz | poph-server, poph-matrix | 73 |
| prazo | deadline | traduz | poph-server, poph-fluxos | 73 |
| falhar | fail | traduz | poph-server, poph-matrix | 72 |
| chamar | call | traduz | poph-agent, poph-server | 71 |
| idempotencia | idempotency | traduz | poph-store, poph-server | 71 |
| atraso | delay | traduz | poph-server, poph-matrix | 70 |
| cifra | cipher | traduz | poph-server, poph | 70 |
| dias | days | traduz | console, poph-retention | 70 |
| fichas | cases | traduz | poph-server, poph-store | 69 |
| parar | stop | traduz | poph-server, poph-fluxos | 69 |
| apos | after | traduz | poph-server, poph | 68 |
| contadores | counters | traduz | poph-server, poph-fluxos | 68 |
| excluir | delete | traduz | poph, poph-server | 68 |
| nenhum | none | traduz | poph-fluxos, poph-server | 68 |
| compilar | compile | traduz | poph-server, poph-fluxos | 67 |
| lote | batch | traduz | poph-server, poph | 67 |
| produtos | products | traduz | poph-server, poph | 67 |
| bruta | raw | traduz | poph-fluxos, poph-server | 66 |
| entregar | deliver | traduz | poph-server, poph-agent | 65 |
| instrucao | instruction | traduz | poph-fluxos, raiz | 65 |
| provisionador | provisioner | traduz | poph-matrix, poph-server | 65 |
| quando | when | traduz | poph-fluxos, poph-server | 65 |
| recebido | received | traduz | poph-server, poph | 65 |
| repetida | repeated | traduz | poph-server, poph-fluxos | 65 |
| revisoes | revisions | traduz | poph-server, poph-fluxos | 65 |
| saidas | outputs | traduz | poph-agent, poph-server | 65 |
| unica | single | traduz | poph-fluxos, poph-server | 65 |
| avaliar | evaluate | traduz | poph-fluxos | 64 |
| ou | or | traduz | poph-fluxos, poph-agent | 64 |
| papel | role | traduz | poph-matrix, poph-server | 64 |
| baixar | download | traduz | poph-server, poph-matrix | 63 |
| corte | cutoff | traduz | poph-matrix, poph-server | 63 |
| diferenca | difference | traduz | poph-fluxos, poph-server | 63 |
| enviado | sent | traduz | poph-server, poph-store | 63 |
| maior | bigger | traduz | poph-fluxos, poph-server | 63 |
| tabela | table | traduz | poph, poph-agent | 63 |
| antigo | old | traduz | poph-matrix, poph-fluxos | 62 |
| ativo | active | traduz | poph-agent, console | 62 |
| carimbo | stamp | traduz | poph-server, poph-fluxos | 62 |
| segundo | second | traduz | poph-server, poph-fluxos | 62 |
| cabecalho | header | traduz | poph-server, poph-agent | 61 |
| categorias | categories | traduz | poph, poph-server | 61 |
| criacao | creation | traduz | poph-matrix, poph-server | 61 |
| encerra | closes | traduz | poph-fluxos, poph-agent | 61 |
| garantir | ensure | traduz | poph-matrix, poph-server | 61 |
| metadados | metadata | traduz | poph-server, poph-files | 61 |
| banco | database | traduz | poph, poph-server | 60 |
| departamentos | departments | traduz | poph-server, poph-fluxos | 60 |
| diagnostico | diagnostic | traduz | poph-fluxos, poph | 60 |
| evolucao | evolution | traduz | poph-server | 60 |
| resolve | resolves | traduz | scripts, poph-fluxos | 60 |
| enfileirar | enqueue | traduz | poph-matrix, poph-server | 59 |
| imagem | image | traduz | poph-matrix, poph-agent | 59 |
| limpo | clean | traduz | poph-server, poph-matrix | 59 |
| obrigatorio | required | traduz | poph-fluxos, poph-matrix | 59 |
| resolucoes | resolutions | traduz | poph-fluxos, poph-server | 59 |
| atender | attend | traduz | poph-server, poph-matrix | 58 |
| escopo | scope | traduz | poph-fluxos, console | 58 |
| ferramentas | tools | traduz | poph-agent, poph-server | 58 |
| marcador | marker | traduz | poph-server, poph-agent | 58 |
| historico | history | traduz | poph-server, poph-matrix | 57 |
| canonico | canonical | traduz | poph-fluxos, poph-server | 56 |
| perfil | profile | traduz | poph-matrix, poph-server | 56 |
| publicada | published | traduz | poph-server, poph-matrix | 56 |
| roteiros | scripts | traduz | poph-agent, poph-server | 56 |
| contem | contains | traduz | poph-fluxos, poph-server | 55 |
| decisoes | decisions | traduz | poph-fluxos, console | 55 |
| escrever | write | traduz | poph, poph-server | 54 |
| inexistente | missing | traduz | poph-server, poph | 54 |
| transitoria | transient | traduz | poph-server, poph-matrix | 54 |
| trecho | passage | traduz | poph-fluxos, poph | 54 |
| comparar | compare | traduz | poph-server, poph | 53 |
| diretorio | directory | traduz | poph-server, poph-rag | 53 |
| escrita | writing | traduz | poph-server, poph-matrix | 53 |
| execucoes | executions | traduz | poph-server, poph-store | 53 |
| montar | build | traduz | poph-server, poph-agent | 53 |
| posicionar | position | traduz | poph-matrix, poph-server | 53 |
| publicar | publish | traduz | poph-server, poph | 53 |
| retrato | snapshot | traduz | poph, poph-server | 53 |
| convidados | guests | traduz | poph-matrix, poph-server | 52 |
| enviados | sent | traduz | poph-server | 52 |
| pendente | pending | traduz | poph-server, poph | 52 |
| proxima | next | traduz | poph-server, poph-matrix | 52 |
| assinatura | signature | traduz | poph-server | 51 |
| esperada | expected | traduz | poph-server, poph | 51 |
| exige | requires | traduz | poph-fluxos, raiz | 51 |
| formatar | format | traduz | poph-fluxos, poph | 51 |
| globais | globals | traduz | poph-server, poph-agent | 51 |
| antiga | old | traduz | poph-fluxos, poph-server | 50 |
| entrar | enter | traduz | poph-fluxos, poph-matrix | 50 |
| existente | existing | traduz | poph-matrix, poph | 50 |
| veneno | poison | traduz | poph-server | 50 |
| ate | until | traduz | poph-server, poph-agent | 49 |
| convite | invitation | traduz | poph-matrix, poph-server | 49 |
| cursores | cursors | traduz | poph-server, poph | 49 |
| referencia | reference | traduz | poph, poph-matrix | 49 |
| uso | use | traduz | poph-fluxos, poph | 49 |
| avancar | advance | traduz | poph-agent, poph-server | 48 |
| cena | scene | traduz | poph-server | 48 |
| dois | two | traduz | poph-fluxos, poph-server | 48 |
| escolhas | choices | traduz | poph-agent, poph-server | 48 |
| existe | exists | traduz | poph-server, poph-matrix | 48 |
| poder | power | traduz | poph-matrix | 48 |
| quem | who | traduz | poph-server, poph-matrix | 48 |
| classificar | classify | traduz | poph-matrix, poph-server | 47 |
| ferramenta | tool | traduz | poph-agent, poph | 47 |
| ouvinte | listener | traduz | poph-matrix, poph-server | 47 |
| vira | becomes | traduz | poph-server, poph-matrix | 47 |
| forma | form | traduz | poph-fluxos, poph | 46 |
| remoto | remote | traduz | poph, poph-server | 46 |
| decidir | decide | traduz | poph-matrix, poph-server | 45 |
| entradas | entries | traduz | poph-fluxos, poph-server | 45 |
| entre | between | traduz | poph-fluxos, poph-server | 45 |
| filho | child | traduz | poph-matrix, poph-fluxos | 45 |
| rascunho | draft | traduz | poph-server, poph-store | 45 |
| rodando | running | traduz | poph-server, poph-agent | 45 |
| testes | tests | traduz | poph-server, poph | 45 |
| tipos | kinds | traduz | poph-fluxos, poph-server | 45 |
| conferir | check | traduz | poph-server, poph | 44 |
| pelo | by | traduz | poph-server, poph-matrix | 44 |
| proximo | next | traduz | poph-server, poph-matrix | 44 |
| redacao | writing | traduz | poph-matrix, poph-server | 44 |
| removida | removed | traduz | poph-fluxos, poph-matrix | 44 |
| sentido | meaning | traduz | poph-matrix, poph-server | 44 |
| anterior | previous | traduz | poph-server, poph-fluxos | 43 |
| argumento | argument | traduz | poph-fluxos, poph-agent | 43 |
| contar | count | traduz | poph-server, poph | 43 |
| espelhado | mirrored | traduz | poph-server, poph-matrix | 43 |
| gravar | write | traduz | poph-server, poph | 43 |
| lida | read | traduz | poph-server, poph-fluxos | 43 |
| locais | local | traduz | poph-fluxos | 43 |
| memoria | memory | traduz | poph-server, poph-matrix | 43 |
| nada | nothing | traduz | poph-server, poph-agent | 43 |
| parte | part | traduz | poph-agent, poph-server | 43 |
| tarefa | task | traduz | poph-server, poph-matrix | 43 |
| coleta | collection | traduz | poph-fluxos | 42 |
| cortado | cut | traduz | poph-fluxos, poph-matrix | 42 |
| criados | created | traduz | poph-server, poph-agent | 42 |
| detalhes | details | traduz | poph-server | 42 |
| mostra | shows | traduz | poph, poph-server | 42 |
| passar | pass | traduz | poph-server, poph-agent | 42 |
| pedaco | piece | traduz | poph-matrix, poph-server | 42 |
| tabelas | tables | traduz | console, poph | 42 |
| valores | values | traduz | poph-server, poph-fluxos | 42 |
| vistos | seen | traduz | poph-fluxos, poph-server | 42 |
| aguardando | waiting | traduz | poph-fluxos, poph-server | 41 |
| identificador | identifier | traduz | poph-matrix, poph-server | 41 |
| laco | loop | traduz | poph-server, poph-agent | 41 |
| obter | get | traduz | poph-server, poph | 41 |
| problemas | problems | traduz | poph, poph-server | 41 |
| raiz | root | traduz | poph-server, poph | 41 |
| titulo | title | traduz | console, poph | 41 |
| atualizado | updated | traduz | poph-server | 40 |
| nomeado | named | traduz | poph-fluxos, poph-agent | 40 |
| passo | step | traduz | poph-server | 40 |
| pela | by | traduz | poph-server, poph | 40 |
| posicionada | positioned | traduz | poph-matrix, poph-server | 40 |
| segundos | seconds | traduz | poph-server, poph | 40 |
| vez | time | traduz | poph-server, poph-fluxos | 40 |
| dado | given | traduz | poph-fluxos, poph-server | 39 |
| definicao | definition | traduz | poph-store, poph-server | 39 |
| definitiva | definitive | traduz | poph-matrix, poph-server | 39 |
| dia | day | traduz | poph-matrix, poph-server | 39 |
| processo | process | traduz | poph-server, poph | 39 |
| ajustes | adjustments | traduz | poph-server, poph | 38 |
| ausente | missing | traduz | poph-server, poph | 38 |
| compila | compiles | traduz | poph-fluxos, poph | 38 |
| declarado | declared | traduz | poph-server, poph-agent | 38 |
| escolha | choice | traduz | poph, console | 38 |
| integracoes | integrations | traduz | poph-server, poph | 38 |
| mesma | same | traduz | poph-fluxos, poph-server | 38 |
| nem | nor | traduz | poph-server, poph-agent | 38 |
| parada | stop | traduz | poph-server, poph-fluxos | 38 |
| poderes | powers | traduz | poph-matrix | 38 |
| retencao | retention | traduz | console, poph | 38 |
| temperatura | temperature | traduz | poph-core, poph-store | 38 |
| anotadora | annotator | traduz | poph-server | 37 |
| esquema | schema | traduz | poph-server, poph | 37 |
| gerar | generate | traduz | poph, poph-server | 37 |
| servico | service | traduz | poph-server, poph | 37 |
| vazia | empty | traduz | poph-agent, poph-matrix | 37 |
| conhecimento | knowledge | traduz | poph-server, console | 36 |
| funcoes | functions | traduz | poph-fluxos, poph-agent | 36 |
| impresso | printed | traduz | poph-fluxos, poph | 36 |
| intervalo | interval | traduz | poph-server, poph | 36 |
| marca | mark | traduz | poph-server, poph-matrix | 36 |
| preparo | preparation | traduz | poph, poph-server | 36 |
| ano | year | traduz | poph-matrix, poph-server | 35 |
| avisar | notify | traduz | poph-server, console | 35 |
| caractere | character | traduz | poph-matrix, poph-server | 35 |
| conflito | conflict | traduz | poph-server, poph | 35 |
| criada | created | traduz | poph-server, poph | 35 |
| decodificar | decode | traduz | poph-server, poph-matrix | 35 |
| objeto | object | traduz | poph-server, poph-matrix | 35 |
| valido | valid | traduz | poph-server, poph-fluxos | 35 |
| apoio | support | traduz | poph, poph-agent | 34 |
| chaves | keys | traduz | poph-fluxos, poph | 34 |
| comportamento | behaviour | traduz | poph-agent | 34 |
| duas | two | traduz | poph-server, poph-agent | 34 |
| encaminhar | forward | traduz | poph-fluxos, poph-server | 33 |
| espelhar | mirror | traduz | poph-matrix, poph-server | 33 |
| fontes | sources | traduz | poph, console | 33 |
| humana | human | traduz | poph-server, poph-matrix | 33 |
| liberar | release | traduz | poph, console | 33 |
| removido | removed | traduz | poph-fluxos, console | 33 |
| resultados | results | traduz | poph-agent, poph-server | 33 |
| transporte | transport | traduz | poph-agent, poph | 33 |
| trechos | passages | traduz | console, poph-server | 33 |
| verificacao | verification | traduz | poph | 33 |
| convites | invitations | traduz | poph-matrix, poph-server | 32 |
| falta | missing | traduz | poph-matrix, poph | 32 |
| filtro | filter | traduz | poph, poph-server | 32 |
| nos | us | traduz | poph-fluxos, poph-server | 32 |
| proibido | forbidden | traduz | poph-server, poph | 32 |
| redacoes | writings | traduz | poph-server, poph-matrix | 32 |
| rodada | round | traduz | poph-server, scripts | 32 |
| todas | all | traduz | poph-server, poph-agent | 32 |
| variaveis | variables | traduz | poph-fluxos, poph | 32 |
| volta | return | traduz | poph-server, poph-fluxos | 32 |
| conjunto | set | traduz | poph-agent, poph-server | 31 |
| forcar | force | traduz | console, poph | 31 |
| importar | import | traduz | poph-server, poph | 31 |
| mesmo | same | traduz | poph-fluxos, poph-server | 31 |
| modelos | models | traduz | poph, poph-server | 31 |
| simples | simple | traduz | poph-fluxos, poph-server | 31 |
| baratas | cheap | traduz | poph-agent | 30 |
| buscas | searches | traduz | poph-agent | 30 |
| capturados | captured | traduz | poph-server | 30 |
| codificar | encode | traduz | poph-matrix | 30 |
| credencial | credential | traduz | poph-matrix | 30 |
| desligada | disabled | traduz | poph-server, poph-matrix | 30 |
| divergencias | divergences | traduz | poph, poph-server | 30 |
| enviadas | sent | traduz | poph-server, poph-agent | 30 |
| fica | stays | traduz | poph-server, poph-fluxos | 30 |
| mutacao | mutation | traduz | poph-fluxos | 30 |
| novos | new | traduz | poph-fluxos, poph-server | 30 |
| renderizar | render | traduz | poph-fluxos, poph-agent | 30 |
| tentativa | attempt | traduz | poph-server, poph-store | 30 |
| verificacoes | verifications | traduz | poph | 30 |
| carrinho | cart | traduz | poph-server, console | 29 |
| coluna | column | traduz | poph-fluxos, poph | 29 |
| conectado | connected | traduz | poph-server | 29 |
| errado | wrong | traduz | poph-server, poph | 29 |
| exemplos | examples | traduz | poph-fluxos, poph-server | 29 |
| ligada | linked | traduz | poph-server, poph-matrix | 29 |
| posicoes | positions | traduz | poph-fluxos, poph-server | 29 |
| relatorios | reports | traduz | poph, poph-server | 29 |
| requisicoes | requests | traduz | poph-matrix | 29 |
| ultima | last | traduz | poph-server, poph | 29 |
| apagadas | deleted | traduz | poph-server, poph-matrix | 28 |
| biblioteca | library | traduz | poph-server, poph | 28 |
| dos | of | traduz | poph-fluxos, poph-server | 28 |
| fechar | close | traduz | poph-server, poph | 28 |
| provisionar | provision | traduz | poph-matrix, poph-server | 28 |
| responde | answers | traduz | poph-server, poph-agent | 28 |
| segurar | hold | traduz | poph-server | 28 |
| terceira | third | traduz | poph-server, poph-agent | 28 |
| transitorio | transient | traduz | poph-matrix, poph-server | 28 |
| trava | lock | traduz | poph-fluxos, poph-server | 28 |
| validade | validity | traduz | poph-server | 28 |
| vizinha | neighbouring | traduz | poph-server, poph | 28 |
| agiram | acted | traduz | poph-fluxos, poph-server | 27 |
| definitivos | definitive | traduz | poph-server, poph-matrix | 27 |
| dentro | inside | traduz | poph-fluxos, poph | 27 |
| esperas | waits | traduz | poph-agent, poph-fluxos | 27 |
| feita | done | traduz | poph-server, poph | 27 |
| loja | store | traduz | poph-server | 27 |
| posicional | positional | traduz | poph-fluxos | 27 |
| recusada | refused | traduz | poph, poph-server | 27 |
| sincronizar | synchronise | traduz | poph-server, poph | 27 |
| codigos | codes | traduz | poph-fluxos, poph-server | 26 |
| demora | delay | traduz | poph-agent | 26 |
| devolve | returns | traduz | poph-server, poph-fluxos | 26 |
| fecha | closes | traduz | poph-fluxos, poph-agent | 26 |
| grafo | graph | traduz | poph-fluxos | 26 |
| leitura | read | traduz | poph-server, poph-matrix | 26 |
| mes | month | traduz | poph-matrix, poph-server | 26 |
| montagem | assembly | traduz | poph, console | 26 |
| plano | plan | traduz | poph-server | 26 |
| preparar | prepare | traduz | poph-server, poph-matrix | 26 |
| publicadas | published | traduz | poph-server | 26 |
| quantidade | quantity | traduz | poph-agent, poph-server | 26 |
| roteador | router | traduz | poph-server, poph-matrix | 26 |
| todos | all | traduz | poph, poph-server | 26 |
| transferencia | transfer | traduz | poph-agent, poph-server | 26 |
| adiadas | deferred | traduz | poph-server | 25 |
| ativos | active | traduz | poph-server, poph | 25 |
| carga | load | traduz | poph-fluxos | 25 |
| carregamento | loading | traduz | poph-fluxos, poph-server | 25 |
| diferente | different | traduz | poph-fluxos, poph | 25 |
| entao | then | traduz | poph-fluxos, raiz | 25 |
| entidade | entity | traduz | poph-server, poph | 25 |
| exemplo | example | traduz | poph-fluxos, poph-server | 25 |
| exigidos | required | traduz | poph-fluxos, poph-agent | 25 |
| falhos | faulty | traduz | poph-matrix, poph-server | 25 |
| falhou | failed | traduz | poph-server, poph | 25 |
| janela | window | traduz | poph-agent, poph-server | 25 |
| permissao | permission | traduz | poph-matrix, poph | 25 |
| peso | weight | traduz | poph-fluxos | 25 |
| rotas | routes | traduz | poph-server | 25 |
| senao | else | traduz | poph-fluxos, raiz | 25 |
| transicao | transition | traduz | poph-server | 25 |
| valida | validates | traduz | poph-fluxos, poph-server | 25 |
| acima | above | traduz | poph-server, poph-fluxos | 24 |
| aparece | appears | traduz | poph-server, poph | 24 |
| avulsa | one_off | traduz | poph-server | 24 |
| esperados | expected | traduz | poph-server | 24 |
| livre | free | traduz | poph-server, poph-fluxos | 24 |
| manifestos | manifests | traduz | poph-server | 24 |
| mantem | keeps | traduz | poph-agent, poph-matrix | 24 |
| minusculo | lowercase | traduz | poph-fluxos | 24 |
| processador | processor | traduz | poph-server, poph | 24 |
| retentativas | retries | traduz | poph-jev, poph-core | 24 |
| sobrescrever | overwrite | traduz | poph-server, console | 24 |
| sonda | probe | traduz | poph-server | 24 |
| abandonar | abandon | traduz | poph-server, poph-fluxos | 23 |
| adiado | deferred | traduz | poph-server, poph-agent | 23 |
| autorizado | authorized | traduz | poph-server, console | 23 |
| complementos | complements | traduz | poph-server | 23 |
| dourada | golden | traduz | poph-fluxos | 23 |
| enviada | sent | traduz | poph-matrix, poph-server | 23 |
| final | final | traduz | poph-matrix, poph-fluxos | 23 |
| gravadas | stored | traduz | poph-server | 23 |
| indisponivel | unavailable | traduz | poph-server, poph-matrix | 23 |
| parcial | partial | traduz | poph-fluxos, poph-server | 23 |
| proprio | own | traduz | poph-server, poph | 23 |
| recuo | backoff | traduz | poph-fluxos, poph | 23 |
| rodar | run | traduz | poph-agent, poph | 23 |
| saltos | jumps | traduz | poph-fluxos, poph-server | 23 |
| secao | section | traduz | poph-fluxos, poph | 23 |
| validador | validator | traduz | poph-fluxos | 23 |
| visita | visit | traduz | poph-fluxos, poph-server | 23 |
| achado | finding | traduz | poph-fluxos, poph-server | 22 |
| alfa | alpha | traduz | poph-agent, poph-fluxos | 22 |
| atendimentos | services | traduz | poph-fluxos, poph | 22 |
| comparacao | comparison | traduz | poph, poph-server | 22 |
| contador | counter | traduz | poph-server, poph-agent | 22 |
| continuacao | continuation | traduz | poph-agent | 22 |
| disfarce | disguise | traduz | poph-server, poph-matrix | 22 |
| divergente | divergent | traduz | poph-server, poph-fluxos | 22 |
| dona | owner | traduz | poph-server | 22 |
| exclusao | exclusion | traduz | poph-server, poph | 22 |
| exibicao | display | traduz | poph-matrix, poph-fluxos | 22 |
| movida | moved | traduz | poph-matrix, poph-server | 22 |
| nunca | never | traduz | poph-server, poph-matrix | 22 |
| operacoes | operations | traduz | poph-server, poph | 22 |
| orientacao | guidance | traduz | poph-agent, raiz | 22 |
| pede | asks | traduz | poph-fluxos, poph | 22 |
| reindexar | reindex | traduz | poph, poph-server | 22 |
| tempos | times | traduz | poph-fluxos, poph-agent | 22 |
| cada | each | traduz | poph-server, poph-fluxos | 21 |
| cargas | loads | traduz | poph-fluxos, poph-server | 21 |
| configurar | configure | traduz | poph-server, poph | 21 |
| conta | counts | traduz | poph-server, poph-fluxos | 21 |
| declarada | declared | traduz | poph-server, poph-matrix | 21 |
| derivar | derive | traduz | poph-server, poph | 21 |
| despedida | farewell | traduz | console, poph-agent | 21 |
| encerrada | closed | traduz | poph-server, poph-fluxos | 21 |
| grupos | groups | traduz | poph, poph-server | 21 |
| importacao | import | traduz | poph, poph-server | 21 |
| menor | smaller | traduz | poph-fluxos, poph-server | 21 |
| quantos | how_many | traduz | poph-server, poph-fluxos | 21 |
| relativo | relative | traduz | poph-server, poph | 21 |
| segmento | segment | traduz | poph-server, poph-matrix | 21 |
| semear | seed | traduz | poph-server, poph | 21 |
| sim | yes | traduz | poph-fluxos, poph | 21 |
| argumentos | arguments | traduz | console, poph-server | 20 |
| booleano | boolean | traduz | poph-fluxos, poph | 20 |
| contagem | count | traduz | poph-server, poph-matrix | 20 |
| deixa | leaves | traduz | poph-fluxos, poph-matrix | 20 |
| guardadas | guarded | traduz | poph-server, poph-fluxos | 20 |
| lugar | place | traduz | poph-fluxos, poph-server | 20 |
| mostrado | shown | traduz | poph, poph-server | 20 |
| parametros | parameters | traduz | poph-server, poph-agent | 20 |
| pergunta | question | traduz | poph-agent, poph-server | 20 |
| portugues | portuguese | traduz | poph, poph-server | 20 |
| preco | price | traduz | console, poph-agent | 20 |
| regras | rules | traduz | poph-fluxos, poph-matrix | 20 |
| virtuais | virtual | traduz | poph-matrix, poph-server | 20 |
| vitima | victim | traduz | poph-server, poph-fluxos | 20 |
| acertos | hits | traduz | poph-server, poph-fluxos | 19 |
| achados | findings | traduz | poph-server, poph | 19 |
| aspas | quotes | traduz | poph-fluxos, poph | 19 |
| atualizacoes | updates | traduz | poph-server, poph | 19 |
| cai | falls | traduz | poph-matrix, poph-agent | 19 |
| confianca | confidence | traduz | poph, console | 19 |
| curso | course | traduz | poph-fluxos | 19 |
| diz | says | traduz | poph, poph-fluxos | 19 |
| entregue | delivered | traduz | poph-server, poph-agent | 19 |
| feito | done | traduz | poph-server | 19 |
| inicial | initial | traduz | poph-server, poph-agent | 19 |
| inteiro | integer | traduz | poph, poph-fluxos | 19 |
| juntos | together | traduz | poph-server, poph-matrix | 19 |
| liberado | released | traduz | poph-server, poph-agent | 19 |
| opcao | option | traduz | poph-fluxos | 19 |
| produzido | produced | traduz | poph-fluxos, poph-server | 19 |
| repetido | repeated | traduz | poph, poph-server | 19 |
| repetir | repeat | traduz | poph-server, poph-matrix | 19 |
| segue | follows | traduz | poph-server, poph-fluxos | 19 |
| transitorios | transient | traduz | poph-server, poph-matrix | 19 |
| velha | old | traduz | poph-server, poph | 19 |
| aceito | accepted | traduz | poph-fluxos, poph-server | 18 |
| achadas | found | traduz | poph-agent, poph-fluxos | 18 |
| alterada | changed | traduz | poph-server | 18 |
| apagar | delete | traduz | poph-matrix, poph | 18 |
| avisadas | notified | traduz | poph-server | 18 |
| comercial | commercial | traduz | poph-server, poph-matrix | 18 |
| convidado | guest | traduz | poph-server, poph | 18 |
| encontrado | found | traduz | poph, poph-server | 18 |
| expulsoes | evictions | traduz | poph-fluxos, poph-server | 18 |
| faltando | missing | traduz | poph-fluxos, poph-agent | 18 |
| fixa | fixed | traduz | poph-matrix, poph-agent | 18 |
| periodo | period | traduz | poph-server | 18 |
| quantas | how_many | traduz | poph-server, poph-fluxos | 18 |
| reabrir | reopen | traduz | poph-server, poph-agent | 18 |
| requisicao | request | traduz | poph-matrix | 18 |
| ruim | poor | traduz | poph, poph-server | 18 |
| tempo | time | traduz | poph-agent, poph-server | 18 |
| temporaria | temporary | traduz | poph | 18 |
| usa | uses | traduz | poph-agent, poph-matrix | 18 |
| ativas | active | traduz | poph-server, console | 17 |
| baixado | downloaded | traduz | poph-server, poph-matrix | 17 |
| coletada | collected | traduz | poph-fluxos | 17 |
| configurado | configured | traduz | console, poph-server | 17 |
| deixou | left | traduz | poph-fluxos | 17 |
| derrubar | drop | traduz | poph-server, poph-fluxos | 17 |
| dominio | domain | traduz | poph-server | 17 |
| encerrar | close | traduz | poph-server, poph-fluxos | 17 |
| fechado | closed | traduz | poph-server, poph-agent | 17 |
| folha | sheet | traduz | poph-fluxos | 17 |
| frase | sentence | traduz | poph-fluxos, poph-server | 17 |
| gravacao | recording | traduz | poph-server, poph-matrix | 17 |
| hoje | today | traduz | poph-agent, poph-server | 17 |
| nomeavel | nameable | traduz | poph-server | 17 |
| observador | observer | traduz | poph, poph-agent | 17 |
| paginas | pages | traduz | poph-server, poph-fluxos | 17 |
| pai | parent | traduz | poph-matrix, poph | 17 |
| perfis | profiles | traduz | poph-server | 17 |
| publicacao | publication | traduz | poph-server, poph | 17 |
| recusar | refuse | traduz | poph-server, poph-matrix | 17 |
| rede | network | traduz | poph-matrix, poph-server | 17 |
| renomeacoes | renames | traduz | poph-server | 17 |
| respeita | respects | traduz | poph-agent, poph-server | 17 |
| sao | are | traduz | poph-matrix, poph-server | 17 |
| unico | single | traduz | poph-server, poph-fluxos | 17 |
| usada | used | traduz | poph-fluxos, poph-server | 17 |
| aceita | accepts | traduz | poph-fluxos, poph-server | 16 |
| atualizadas | updated | traduz | console, poph-server | 16 |
| conferido | checked | traduz | poph, poph-server | 16 |
| entra | enters | traduz | poph-agent, poph-server | 16 |
| espelhadas | mirrored | traduz | poph-server | 16 |
| estruturado | structured | traduz | poph-server, poph | 16 |
| fala | speaks | traduz | poph-server, poph-agent | 16 |
| humano | human | traduz | poph-server, poph-matrix | 16 |
| leva | takes | traduz | poph-matrix, poph-agent | 16 |
| limitador | limiter | traduz | poph-agent, poph-server | 16 |
| passos | steps | traduz | poph-server, poph-agent | 16 |
| pessoa | person | traduz | poph-matrix, poph-server | 16 |
| recebidos | received | traduz | poph-server | 16 |
| remetentes | senders | traduz | poph-server | 16 |
| seguidas | consecutive | traduz | poph-server | 16 |
| ver | see | traduz | poph-server | 16 |
| aberto | open | traduz | poph-server | 15 |
| ajuda | help | traduz | poph, poph-fluxos | 15 |
| alheio | foreign | traduz | poph-server, poph-agent | 15 |
| alteradas | changed | traduz | poph-server | 15 |
| alterado | changed | traduz | poph-server, poph | 15 |
| capturado | captured | traduz | poph-matrix, poph-server | 15 |
| contrato | contract | traduz | poph-matrix, poph-agent | 15 |
| declarados | declared | traduz | poph-server, poph-fluxos | 15 |
| dispositivo | device | traduz | poph-matrix, poph-server | 15 |
| escalares | scalars | traduz | poph | 15 |
| espelhada | mirrored | traduz | poph-matrix, poph-server | 15 |
| esquecer | forget | traduz | poph-server | 15 |
| expirar | expire | traduz | poph-matrix, poph-server | 15 |
| exportar | export | traduz | poph, poph-server | 15 |
| filhos | children | traduz | poph-fluxos, poph-server | 15 |
| hora | hour | traduz | poph-fluxos, poph-server | 15 |
| inativo | inactive | traduz | poph, poph-server | 15 |
| inseridas | inserted | traduz | console, poph-server | 15 |
| latencia | latency | traduz | poph-server, poph-agent | 15 |
| ligacao | link | traduz | poph | 15 |
| processar | process | traduz | poph-agent, poph-server | 15 |
| receber | receive | traduz | poph-server | 15 |
| redigidas | drafted | traduz | poph-server | 15 |
| reserva | reservation | traduz | poph-server, poph-agent | 15 |
| resolucao | resolution | traduz | poph-fluxos, poph | 15 |
| segura | safe | traduz | poph-server, poph-agent | 15 |
| situacoes | situations | traduz | console, poph-agent | 15 |
| sufixo | suffix | traduz | poph-matrix, poph | 15 |
| suporte | support | traduz | poph-matrix, poph-server | 15 |
| terceiro | third | traduz | poph-server, poph-agent | 15 |
| vezes | times | traduz | poph-server, poph-fluxos | 15 |
| apertado | tight | traduz | poph-server, poph-fluxos | 14 |
| assinados | signed | traduz | poph-server | 14 |
| buscar | search | traduz | poph-matrix, poph-server | 14 |
| caracteres | characters | traduz | poph-agent, poph | 14 |
| colecao | collection | traduz | poph-server | 14 |
| coletor | collector | traduz | poph-agent | 14 |
| conflitos | conflicts | traduz | poph-server, poph | 14 |
| continua | continues | traduz | poph-agent, poph-server | 14 |
| convidar | invite | traduz | poph-matrix | 14 |
| cria | creates | traduz | poph, poph-matrix | 14 |
| curto | short | traduz | poph-matrix, poph-server | 14 |
| desconhecida | unknown | traduz | poph-server, console | 14 |
| desconhecidas | unknown | traduz | poph-server | 14 |
| estados | states | traduz | poph-server | 14 |
| gravados | stored | traduz | poph-server, console | 14 |
| legenda | caption | traduz | poph-server | 14 |
| leituras | reads | traduz | poph-server, poph-agent | 14 |
| nulo | null | traduz | poph-fluxos, poph-server | 14 |
| paginacao | pagination | traduz | poph-server | 14 |
| pares | pairs | traduz | poph, poph-fluxos | 14 |
| poupadas | saved | traduz | poph-fluxos, poph-server | 14 |
| preparado | prepared | traduz | poph-server | 14 |
| processadas | processed | traduz | poph-server | 14 |
| rapidas | fast | traduz | poph-agent | 14 |
| saude | health | traduz | poph-server, raiz | 14 |
| segmentos | segments | traduz | poph-server | 14 |
| telefone | phone | traduz | poph-matrix, poph-server | 14 |
| tratar | handle | traduz | poph-server | 14 |
| usado | used | traduz | poph-fluxos, poph-matrix | 14 |
| usados | used | traduz | poph-fluxos | 14 |
| vazamento | leak | traduz | poph-server | 14 |
| adiados | deferred | traduz | poph-server | 13 |
| apaga | deletes | traduz | poph-server, poph | 13 |
| chegada | arrival | traduz | poph-fluxos, poph-server | 13 |
| consultar | query | traduz | poph-agent, poph | 13 |
| esperadas | expected | traduz | poph-server, poph | 13 |
| estavel | stable | traduz | poph, poph-matrix | 13 |
| extrator | extractor | traduz | poph-files, scripts | 13 |
| futuro | future | traduz | poph-server, poph | 13 |
| isolamento | isolation | traduz | poph-server | 13 |
| marcar | mark | traduz | poph-matrix, poph-server | 13 |
| meio | middle | traduz | poph-server, poph-agent | 13 |
| notificar | notify | traduz | poph-server, raiz | 13 |
| pareamento | pairing | traduz | poph-server | 13 |
| passagem | passage | traduz | poph-server, poph-matrix | 13 |
| recarregar | reload | traduz | poph-server | 13 |
| renomear | rename | traduz | poph-server, poph-fluxos | 13 |
| risco | risk | traduz | poph-agent, poph | 13 |
| somar | sum | traduz | poph-server | 13 |
| tarefas | tasks | traduz | poph-server | 13 |
| versoes | versions | traduz | poph-server, poph | 13 |
| vinculo | link | traduz | poph-server, poph | 13 |
| vistas | views | traduz | poph-server, poph-fluxos | 13 |
| concluido | finished | traduz | poph-agent | 12 |
| condicao | condition | traduz | poph-server | 12 |
| criadas | created | traduz | poph-server | 12 |
| degradada | degraded | traduz | console | 12 |
| digitos | digits | traduz | poph-fluxos, poph-matrix | 12 |
| disposicao | layout | traduz | poph-server | 12 |
| escolhido | chosen | traduz | poph-server, poph-agent | 12 |
| escritor | writer | traduz | poph-matrix, poph | 12 |
| extraido | extracted | traduz | poph-agent, poph-server | 12 |
| ignorado | ignored | traduz | poph-agent, poph-matrix | 12 |
| informado | informed | traduz | poph-server, console | 12 |
| lidos | read | traduz | poph-fluxos, poph-server | 12 |
| medir | measure | traduz | poph-server, poph-agent | 12 |
| nas | in_the | traduz | poph-server, poph-agent | 12 |
| novas | new | traduz | poph-server, poph-matrix | 12 |
| propria | own | traduz | poph-server, poph | 12 |
| traco | trace | traduz | poph-server, poph | 12 |
| transferida | transferred | traduz | poph-server, poph-agent | 12 |
| trocado | swapped | traduz | poph-server, poph-matrix | 12 |
| vaga | slot | traduz | poph-agent | 12 |
| abertura | opening | traduz | poph-server, poph | 11 |
| adicionadas | added | traduz | console, poph-server | 11 |
| agendar | schedule | traduz | poph-fluxos, poph-server | 11 |
| baixa | low | traduz | poph-agent, poph-server | 11 |
| duplicadas | duplicated | traduz | poph-server | 11 |
| edicao | edit | traduz | poph-matrix, poph-server | 11 |
| foto | photo | traduz | poph-server, poph | 11 |
| gerado | generated | traduz | poph, poph-server | 11 |
| guardado | stored | traduz | poph-server, poph-agent | 11 |
| instante | instant | traduz | poph-matrix, poph-agent | 11 |
| longo | long | traduz | poph-server, poph | 11 |
| mediana | median | traduz | poph-agent, poph-server | 11 |
| metrica | metric | traduz | poph-server | 11 |
| mudancas | changes | traduz | poph-server, poph | 11 |
| orfaos | orphans | traduz | poph | 11 |
| passa | passes | traduz | poph-fluxos, poph-server | 11 |
| permitido | allowed | traduz | poph-server | 11 |
| preenchidos | filled | traduz | console, poph-server | 11 |
| recebidas | received | traduz | poph-agent | 11 |
| redigir | draft | traduz | poph-matrix, poph-server | 11 |
| removidas | removed | traduz | console, poph-server | 11 |
| soma | sum | traduz | poph-agent, poph-server | 11 |
| tres | three | traduz | poph-fluxos, poph-agent | 11 |
| trocaram | swapped | traduz | console, poph-server | 11 |
| amostras | samples | traduz | poph-server | 10 |
| comentarios | comments | traduz | poph-fluxos, poph | 10 |
| concluida | finished | traduz | poph-fluxos, poph-server | 10 |
| concorrencia | concurrency | traduz | poph-matrix, poph-server | 10 |
| construir | build | traduz | poph-server, poph | 10 |
| criando | creating | traduz | poph-matrix, poph-server | 10 |
| desconectar | disconnect | traduz | poph-server, console | 10 |
| direto | direct | traduz | poph-server, poph | 10 |
| esgotadas | exhausted | traduz | poph-server | 10 |
| finalizada | finalised | traduz | poph-store, poph-agent | 10 |
| ilegivel | unreadable | traduz | poph-matrix, poph-server | 10 |
| limpa | clean | traduz | poph-server, poph-fluxos | 10 |
| maximo | maximum | traduz | poph-server, poph | 10 |
| parado | stopped | traduz | poph-fluxos, poph | 10 |
| publicado | published | traduz | poph-server, poph-matrix | 10 |
| quente | hot | traduz | poph, poph-server | 10 |
| registrada | recorded | traduz | poph-server | 10 |
| trabalho | work | traduz | poph-fluxos | 10 |
| vencidas | expired | traduz | poph-server | 10 |
| verificar | verify | traduz | poph, poph-server | 10 |
| acumulado | accumulated | traduz | poph-matrix | 9 |
| agir | act | traduz | poph-server, poph-fluxos | 9 |
| apagado | deleted | traduz | poph, poph-agent | 9 |
| apagou | deleted | traduz | poph-matrix, poph-server | 9 |
| caminhos | paths | traduz | poph-fluxos, poph-server | 9 |
| celulas | cells | traduz | poph-server | 9 |
| centavos | cents | traduz | poph-agent, console | 9 |
| corpos | bodies | traduz | poph-server | 9 |
| declara | declares | traduz | poph-server, poph | 9 |
| estava | was | traduz | poph-matrix, poph | 9 |
| extracao | extraction | traduz | poph-server, poph-agent | 9 |
| forcado | forced | traduz | poph-server, poph | 9 |
| liberados | released | traduz | poph-server, poph | 9 |
| medicao | measurement | traduz | poph-agent, scripts | 9 |
| metodos | methods | traduz | poph-server | 9 |
| minimo | minimum | traduz | poph-server, poph-fluxos | 9 |
| previa | preview | traduz | poph-server | 9 |
| profundidade | depth | traduz | poph-fluxos | 9 |
| prontos | ready | traduz | poph-matrix | 9 |
| recibo | receipt | traduz | poph-fluxos | 9 |
| transferir | transfer | traduz | poph-server, poph-matrix | 9 |
| ajuste | adjustment | traduz | poph-agent, poph-server | 8 |
| alcancados | reached | traduz | poph-fluxos | 8 |
| antigas | old | traduz | poph-matrix | 8 |
| aplicar | apply | traduz | poph, poph-fluxos | 8 |
| armazem | store | traduz | poph, poph-agent | 8 |
| automatico | automatic | traduz | poph-matrix, poph-server | 8 |
| candidata | candidate | traduz | poph-agent, poph-fluxos | 8 |
| classe | class | traduz | poph-server | 8 |
| desconhecido | unknown | traduz | poph-server, poph-agent | 8 |
| desde | since | traduz | poph, poph-server | 8 |
| destinos | targets | traduz | poph-fluxos | 8 |
| entrou | entered | traduz | poph-matrix, poph-store | 8 |
| fator | factor | traduz | poph-server | 8 |
| instalacao | installation | traduz | poph, poph-agent | 8 |
| instalado | installed | traduz | poph-server, poph | 8 |
| mais | more | traduz | poph-server, poph-agent | 8 |
| recurso | resource | traduz | poph-server | 8 |
| remover | remove | traduz | poph-matrix, poph-agent | 8 |
| simulacao | simulation | traduz | poph | 8 |
| sintaxe | syntax | traduz | poph, poph-server | 8 |
| terminar | finish | traduz | poph-server | 8 |
| confirmacao | confirmation | traduz | poph-agent | 7 |
| controle | control | traduz | poph-server, poph-matrix | 7 |
| custo | cost | traduz | poph-agent, poph-matrix | 7 |
| deixar | leave | traduz | poph-agent | 7 |
| dono | owner | traduz | poph-server, poph-fluxos | 7 |
| guardados | guarded | traduz | poph-server, poph | 7 |
| guardar | keep | traduz | poph-server, poph-fluxos | 7 |
| idempotente | idempotent | traduz | poph, poph-agent | 7 |
| precisa | needs | traduz | poph-fluxos, poph | 7 |
| retomada | resumption | traduz | poph-agent, poph-matrix | 7 |
| seguro | safe | traduz | poph-matrix, poph-server | 7 |
| tocar | touch | traduz | poph-server, poph-fluxos | 7 |
| vinculado | linked | traduz | poph, poph-server | 7 |
| acento | accent | traduz | poph-fluxos, poph | 6 |
| antigos | old | traduz | poph-matrix, poph-agent | 6 |
| candidato | candidate | traduz | poph-fluxos | 6 |
| carrinhos | carts | traduz | console | 6 |
| cifrados | encrypted | traduz | poph | 6 |
| compartilhado | shared | traduz | poph-agent, poph | 6 |
| cortar | cut | traduz | poph-matrix | 6 |
| decorrido | elapsed | traduz | poph-agent, poph-matrix | 6 |
| direcao | direction | traduz | poph-server | 6 |
| efetivos | effective | traduz | poph-agent | 6 |
| encerramento | closure | traduz | poph-agent, poph-fluxos | 6 |
| esta | this | traduz | poph-server, poph | 6 |
| imagens | images | traduz | poph-agent | 6 |
| latencias | latencies | traduz | poph-agent | 6 |
| notificacao | notification | traduz | poph-server, poph-fluxos | 6 |
| pode | can | traduz | poph, poph-server | 6 |
| propriedades | properties | traduz | poph-agent, poph-server | 6 |
| recarga | reload | traduz | poph-server | 6 |
| retirar | withdraw | traduz | poph | 6 |
| rodadas | rounds | traduz | console, poph-server | 6 |
| subida | rise | traduz | poph-server | 6 |
| tentar | try | traduz | poph-matrix | 6 |
| travar | lock | traduz | poph-matrix, poph-agent | 6 |
| tudo | everything | traduz | poph-server, poph-agent | 6 |
| unicos | single | traduz | poph-fluxos, poph-server | 6 |
| velho | old | traduz | poph-agent, poph-server | 6 |
| vincular | link | traduz | poph, poph-server | 6 |
| agenda | schedule | traduz | poph-server | 5 |
| apagada | deleted | traduz | poph-matrix, poph-server | 5 |
| ativa | active | traduz | poph-server, poph-agent | 5 |
| avancou | advanced | traduz | poph-fluxos | 5 |
| confirmar | confirm | traduz | poph-server | 5 |
| consultas | queries | traduz | poph-server | 5 |
| disco | disk | traduz | poph-server, poph | 5 |
| duplicado | duplicate | traduz | poph, poph-server | 5 |
| estatisticas | statistics | traduz | poph-server | 5 |
| faixa | range | traduz | poph, poph-server | 5 |
| gerados | generated | traduz | console, poph-agent | 5 |
| habilitada | enabled | traduz | poph-agent, poph | 5 |
| legivel | readable | traduz | poph, poph-agent | 5 |
| limpar | clean | traduz | poph-fluxos, poph-agent | 5 |
| listagem | listing | traduz | poph-server | 5 |
| migracao | migration | traduz | poph | 5 |
| minuscula | lowercase | traduz | poph-server | 5 |
| nenhuma | none | traduz | poph-fluxos, poph-server | 5 |
| ocupada | busy | traduz | poph-agent | 5 |
| orcamento | budget | traduz | poph-agent, poph-matrix | 5 |
| ordenadas | sorted | traduz | poph-server | 5 |
| pessoas | people | traduz | poph-matrix | 5 |
| remota | remote | traduz | poph-server | 5 |
| removidos | removed | traduz | console, poph | 5 |
| revogado | revoked | traduz | poph, poph-server | 5 |
| revogar | revoke | traduz | poph-server, poph | 5 |
| sensivel | sensitive | traduz | poph, poph-server | 5 |
| simultaneos | simultaneous | traduz | raiz, poph-agent | 5 |
| sistema | system | traduz | poph-server, poph-matrix | 5 |
| temporario | temporary | traduz | poph | 5 |
| voltou | returned | traduz | poph, poph-fluxos | 5 |
| abaixo | below | traduz | poph, poph-server | 4 |
| abandonada | abandoned | traduz | poph-fluxos, poph-server | 4 |
| abstencao | abstention | traduz | poph, poph-agent | 4 |
| alterar | modify | traduz | poph, poph-fluxos | 4 |
| auditorias | audits | traduz | poph-server | 4 |
| baixada | downloaded | traduz | poph-server | 4 |
| cancelado | cancelled | traduz | poph-fluxos | 4 |
| cancelamento | cancellation | traduz | poph, poph-agent | 4 |
| combinador | combinator | traduz | poph-fluxos | 4 |
| definitivas | definitive | traduz | poph-server | 4 |
| derivado | derived | traduz | poph, poph-server | 4 |
| escolher | choose | traduz | poph-server | 4 |
| expirada | expired | traduz | poph-server, poph-matrix | 4 |
| filtros | filters | traduz | poph-server | 4 |
| idade | age | traduz | poph, poph-server | 4 |
| inicia | starts | traduz | poph-agent, poph-server | 4 |
| limiar | threshold | traduz | poph-server, poph | 4 |
| margem | margin | traduz | console | 4 |
| obtido | obtained | traduz | poph-agent, poph-fluxos | 4 |
| palavra | word | traduz | raiz | 4 |
| perguntas | questions | traduz | console | 4 |
| problema | problem | traduz | raiz, poph | 4 |
| recusados | refused | traduz | poph-server, poph | 4 |
| repeticao | repetition | traduz | poph-server, poph-agent | 4 |
| alterados | changed | traduz | console | 3 |
| atualizada | updated | traduz | poph-server, poph-store | 3 |
| autor | author | traduz | poph-fluxos | 3 |
| capturar | capture | traduz | poph-server | 3 |
| colunas | columns | traduz | poph | 3 |
| conclusao | completion | traduz | poph-server, poph-agent | 3 |
| confiavel | reliable | traduz | poph-agent | 3 |
| contato | contact | traduz | poph-agent, raiz | 3 |
| criterios | criteria | traduz | poph-agent | 3 |
| decididas | decided | traduz | console | 3 |
| encerradas | closed | traduz | poph-server | 3 |
| encerrado | closed | traduz | poph-fluxos | 3 |
| extensao | extension | traduz | poph-server, poph | 3 |
| fechamento | closing | traduz | poph-server, poph-fluxos | 3 |
| finalizar | finalise | traduz | poph-matrix | 3 |
| ida | trip | traduz | poph-fluxos, poph-server | 3 |
| inalterados | unchanged | traduz | console | 3 |
| invalidos | invalid | traduz | poph-fluxos | 3 |
| livres | free | traduz | poph-fluxos | 3 |
| malformado | malformed | traduz | poph-server | 3 |
| motivos | reasons | traduz | poph-matrix | 3 |
| normalizar | normalise | traduz | poph | 3 |
| opcional | optional | traduz | poph-server, poph-fluxos | 3 |
| organizacao | organisation | traduz | poph-fluxos | 3 |
| palavras | words | traduz | poph-fluxos | 3 |
| pedacos | pieces | traduz | poph-server, poph-matrix | 3 |
| politica | policy | traduz | poph-server, poph-agent | 3 |
| presenca | presence | traduz | poph | 3 |
| prontas | ready | traduz | poph-server | 3 |
| proprias | own | traduz | poph, poph-matrix | 3 |
| reais | actual | traduz | poph-server | 3 |
| relevancia | relevance | traduz | console | 3 |
| similaridade | similarity | traduz | console | 3 |
| substituir | replace | traduz | poph-server | 3 |
| termo | term | traduz | poph-agent | 3 |
| validas | valid | traduz | poph-fluxos, poph-agent | 3 |
| vinculos | links | traduz | poph-server | 3 |
| alvos | target | traduz | poph-matrix | 2 |
| andamento | progress | traduz | poph-agent | 2 |
| aplica | applies | traduz | poph-matrix, poph-server | 2 |
| caixa | box | traduz | poph-fluxos | 2 |
| candidatos | candidates | traduz | poph-agent | 2 |
| celula | cell | traduz | poph-server | 2 |
| combina | combines | traduz | poph-matrix | 2 |
| condicoes | conditions | traduz | console | 2 |
| consertar | repair | traduz | poph-server | 2 |
| despachar | dispatch | traduz | poph-server | 2 |
| equipe | team | traduz | poph-fluxos, poph-server | 2 |
| esgotado | exhausted | traduz | poph-server | 2 |
| executada | executed | traduz | poph-agent | 2 |
| exportacao | export | traduz | poph-server | 2 |
| familia | family | traduz | poph-matrix | 2 |
| indexacao | indexing | traduz | poph | 2 |
| largura | width | traduz | poph-fluxos | 2 |
| mudar | change | traduz | poph-agent | 2 |
| recebida | received | traduz | poph-server | 2 |
| reenvios | resends | traduz | poph-server | 2 |
| topo | top | traduz | poph-fluxos | 2 |
| truncar | truncate | traduz | poph | 2 |
| unidade | unit | traduz | console | 2 |
| visivel | visible | traduz | poph-matrix | 2 |
| agrupamento | grouping | traduz | poph-agent | 1 |
| alteracao | change | traduz | poph | 1 |
| aninhado | nested | traduz | poph-server | 1 |
| apenas | only | traduz | poph-agent | 1 |
| arquivado | archived | traduz | poph-agent | 1 |
| conexoes | connections | traduz | poph-server | 1 |
| correcao | fix | traduz | poph-matrix | 1 |
| desabilitado | disabled | traduz | poph-agent | 1 |
| dica | hint | traduz | poph-fluxos | 1 |
| editar | edit | traduz | poph-agent | 1 |
| enganosa | misleading | traduz | poph | 1 |
| expor | expose | traduz | poph-server | 1 |
| frente | front | traduz | poph-server | 1 |
| geral | general | traduz | poph-agent | 1 |
| habilitado | enabled | traduz | poph | 1 |
| migracoes | migrations | traduz | poph | 1 |
| minutos | minutes | traduz | scripts | 1 |
| organizacoes | organisations | traduz | poph-server | 1 |
| painel | panel | traduz | poph-agent | 1 |
| ponto | point | traduz | poph-server | 1 |
| processado | processed | traduz | poph-agent | 1 |
| programa | program | traduz | poph-server | 1 |
| recente | recent | traduz | poph-server | 1 |
| reiniciar | restart | traduz | poph-server | 1 |
| rotina | routine | traduz | poph-server | 1 |
| salvo | saved | traduz | poph-server | 1 |
| selecionado | selected | traduz | console | 1 |
| volatil | volatile | traduz | poph-agent | 1 |
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
