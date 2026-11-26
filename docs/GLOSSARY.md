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
| cenario | scenario | traduz | poph-agent, poph-server | 6557 |
| erro | error | traduz | poph-server, poph-store | 6171 |
| texto | text | traduz | poph-store, poph-agent | 5586 |
| agente | agent | traduz | poph-server, poph-store | 4309 |
| conversa | conversation | traduz | poph-server, poph-agent | 4128 |
| estado | state | traduz | poph-server, poph-agent | 3986 |
| nome | name | traduz | poph-server, poph-store | 3248 |
| novo | new | traduz | poph-server, poph-agent | 2934 |
| de | of | traduz | poph-server, poph-store | 2457 |
| corpo | body | traduz | poph-server, console | 2330 |
| da | of | traduz | poph-server, poph-store | 2324 |
| fluxo | flow | traduz | poph-agent, poph-store | 2273 |
| com | with | traduz | poph-server, poph-agent | 2254 |
| resposta | response | traduz | poph-server, poph-jev | 1995 |
| pasta | folder | traduz | poph, poph-server | 1903 |
| linha | line | traduz | poph-store, poph-server | 1845 |
| mensagem | message | traduz | poph-agent, poph-server | 1834 |
| sala | room | traduz | poph-server, poph-matrix | 1811 |
| saida | output | traduz | poph, poph-server | 1796 |
| valor | value | traduz | poph-server, poph-fluxos | 1626 |
| evento | event | traduz | console, poph-server | 1569 |
| cliente | client | traduz | poph-server, poph-matrix | 1505 |
| turno | turn | traduz | poph-agent, poph-server | 1501 |
| caminho | path | traduz | poph-server, poph | 1432 |
| criar | create | traduz | poph-server, poph-store | 1425 |
| motivo | reason | traduz | poph-server, poph-matrix | 1372 |
| motor | engine | traduz | poph-agent, poph-server | 1351 |
| resultado | result | traduz | poph-store, poph-fluxos | 1300 |
| em | in | traduz | poph-store, poph-server | 1285 |
| arquivo | file | traduz | poph, poph-server | 1275 |
| nova | new | traduz | poph-agent, poph-server | 1251 |
| tipo | kind | traduz | poph-fluxos, poph-server | 1244 |
| chave | key | traduz | poph-server, poph-fluxos | 1226 |
| metricas | metrics | traduz | poph-server, poph-agent | 1191 |
| sem | without | traduz | poph-server, poph-agent | 1186 |
| dados | data | traduz | poph-matrix, poph-server | 1147 |
| falha | failure | traduz | poph-server, poph | 1129 |
| campo | field | traduz | poph-fluxos, console | 1113 |
| departamento | department | traduz | poph-server, poph | 1093 |
| ficha | case | traduz | poph-store, poph-server | 1079 |
| acao | action | traduz | poph-fluxos, poph | 1061 |
| para | for | traduz | console, poph-fluxos | 1046 |
| versao | version | traduz | poph-server, poph-store | 1045 |
| servidor | server | traduz | console, poph-server | 994 |
| entrada | entry | traduz | poph-agent, poph-store | 970 |
| codigo | code | traduz | poph-fluxos, poph-server | 962 |
| registro | record | traduz | poph-agent, poph-server | 953 |
| nao | not | traduz | poph-server, poph-fluxos | 913 |
| opcoes | options | traduz | poph-agent, poph-server | 889 |
| origem | origin | traduz | poph-store, poph-server | 869 |
| destino | target | traduz | poph-server, poph-fluxos | 867 |
| etapa | step | traduz | poph-fluxos, poph-server | 843 |
| execucao | execution | traduz | poph-store, poph-server | 828 |
| resumo | summary | traduz | poph-agent, poph-server | 795 |
| alvo | target | traduz | poph-server, poph-fluxos | 786 |
| espaco | space | traduz | poph-matrix, poph-server | 785 |
| pedido | request | traduz | poph-agent, poph-server | 778 |
| decisao | decision | traduz | poph-jev, poph-fluxos | 770 |
| ao | to | traduz | console, poph-fluxos | 756 |
| midia | media | traduz | poph-server, poph-matrix | 756 |
| indice | index | traduz | poph-rag, scripts | 755 |
| atual | current | traduz | console, poph-server | 738 |
| usuario | user | traduz | console, poph-matrix | 726 |
| mensagens | messages | traduz | poph-agent, poph-server | 725 |
| grupo | group | traduz | poph-server, poph | 692 |
| produto | product | traduz | poph-agent, poph | 690 |
| itens | items | traduz | poph-server, console | 678 |
| conversas | conversations | traduz | poph-server, poph-store | 660 |
| canal | channel | traduz | poph-server, poph-store | 643 |
| responder | reply | traduz | console, poph-server | 639 |
| chamada | call | traduz | poph-server, poph-matrix | 633 |
| relatorio | report | traduz | poph-server, poph | 620 |
| agentes | agents | traduz | poph-server, poph-store | 616 |
| pagina | page | traduz | poph-server, poph-store | 616 |
| lista | list | traduz | poph-server, poph-fluxos | 612 |
| fim | end | traduz | poph-fluxos, poph-agent | 606 |
| integracao | integration | traduz | poph-server, console | 603 |
| por | by | traduz | poph-agent, poph-store | 595 |
| fonte | source | traduz | poph-fluxos, poph-server | 593 |
| agora | now | traduz | poph-store, poph-server | 592 |
| modelo | model | traduz | poph-agent, poph-rag | 588 |
| inicio | beginning | traduz | poph-server, poph-agent | 582 |
| falso | false | traduz | poph-server, poph-agent | 581 |
| fluxos | flows | traduz | poph-server, poph-store | 578 |
| revisao | revision | traduz | poph-store, poph-server | 577 |
| regra | rule | traduz | poph-fluxos, poph-store | 575 |
| aviso | notice | traduz | poph-server, console | 570 |
| pedir | ask | traduz | poph-server, scripts | 566 |
| provedor | provider | traduz | poph-agent, poph-server | 560 |
| limite | limit | traduz | poph-store, poph-server | 528 |
| gatilho | trigger | traduz | poph-agent, poph-fluxos | 512 |
| ator | actor | traduz | poph-store, poph-fluxos | 511 |
| eventos | events | traduz | poph-server, poph-store | 510 |
| mudanca | change | traduz | poph-fluxos, poph-store | 510 |
| chamadas | calls | traduz | console, poph-agent | 502 |
| obter | get | traduz | poph-server, poph-store | 500 |
| categoria | category | traduz | poph, poph-server | 496 |
| listar | list | traduz | poph-server, poph-store | 492 |
| rotulo | label | traduz | console, poph-server | 485 |
| porta | port | traduz | poph-server, poph-matrix | 483 |
| ambiente | environment | traduz | poph, poph-files | 477 |
| conteudo | content | traduz | poph-server, poph-matrix | 470 |
| limites | limits | traduz | poph-fluxos, poph-agent | 458 |
| situacao | situation | traduz | console, poph-store | 452 |
| padrao | default | traduz | poph-agent, console | 449 |
| consulta | query | traduz | poph-store, poph-server | 444 |
| envio | send | traduz | poph-server, poph-agent | 443 |
| ferramenta | tool | traduz | poph-agent, poph-core | 439 |
| escolha | choice | traduz | poph-jev, poph-agent | 438 |
| abrir | open | traduz | poph, poph-server | 434 |
| efeito | effect | traduz | poph-fluxos, poph-server | 431 |
| banco | database | traduz | poph-store, poph-retention | 423 |
| papel | role | traduz | poph-agent, poph-store | 420 |
| contexto | context | traduz | poph-fluxos, poph-agent | 416 |
| montar | build | traduz | console, poph-agent | 412 |
| produtos | products | traduz | poph-agent, poph-server | 402 |
| efeitos | effects | traduz | poph-server, poph-fluxos | 389 |
| caso | case | traduz | poph-server, poph-fluxos | 387 |
| atores | actors | traduz | poph-agent, poph-fluxos | 381 |
| linhas | lines | traduz | poph-store, scripts | 372 |
| configuracao | configuration | traduz | poph, poph-server | 370 |
| teto | cap | traduz | poph-server, poph-fluxos | 369 |
| instancia | instance | traduz | poph-server, poph-evolution | 368 |
| desligar | disable | traduz | poph-server, poph | 365 |
| quente | hot | traduz | poph-agent, poph-server | 363 |
| externo | external | traduz | poph-store, poph-agent | 361 |
| depois | after | traduz | poph-store, poph-server | 357 |
| descricao | description | traduz | console, poph-store | 357 |
| guarda | guard | traduz | poph-matrix, poph-server | 357 |
| bloco | block | traduz | poph-agent, console | 354 |
| outro | other | traduz | poph-server, poph-store | 354 |
| campos | fields | traduz | poph-fluxos, poph-agent | 349 |
| antes | before | traduz | poph-server, poph-store | 348 |
| gravada | stored | traduz | poph-store, poph-server | 340 |
| metodo | method | traduz | poph-server, console | 340 |
| pendentes | pending | traduz | poph-agent, poph-server | 340 |
| detalhe | detail | traduz | poph-server, poph | 338 |
| lote | batch | traduz | poph-rag, poph-jev | 337 |
| primeira | first | traduz | poph-server, poph-store | 334 |
| mapa | map | traduz | poph-server, poph-agent | 333 |
| roteiro | script | traduz | poph-agent, poph-server | 331 |
| falhas | failures | traduz | poph-server, poph-agent | 328 |
| identidade | identity | traduz | poph-server, poph-rag | 326 |
| salas | rooms | traduz | poph-server, poph-matrix | 324 |
| executar | run | traduz | poph-fluxos, poph-agent | 321 |
| rota | route | traduz | poph-server, console | 314 |
| arquivos | files | traduz | poph-server, console | 313 |
| registrar | register | traduz | poph-server, poph-store | 311 |
| tamanho | size | traduz | poph-server, poph-agent | 310 |
| coluna | column | traduz | poph-store, poph-fluxos | 309 |
| interno | internal | traduz | poph-agent, poph-server | 308 |
| vazio | empty | traduz | poph-server, poph-fluxos | 307 |
| numero | number | traduz | console, poph-store | 306 |
| avaliador | evaluator | traduz | poph-fluxos, poph-agent | 304 |
| anexo | attachment | traduz | poph-server, poph-agent | 303 |
| iniciar | start | traduz | poph-fluxos, poph-server | 303 |
| direcao | direction | traduz | poph-agent, poph-server | 301 |
| pedidos | requests | traduz | poph-agent, poph-core | 298 |
| espacos | spaces | traduz | poph-server, poph-matrix | 296 |
| nomes | names | traduz | poph-fluxos, poph-agent | 295 |
| relato | report | traduz | poph-server, poph-agent | 294 |
| outra | other | traduz | poph-store, poph-server | 293 |
| modulo | module | traduz | poph-server, console | 292 |
| segredo | secret | traduz | poph-server, poph | 288 |
| partes | parts | traduz | poph-server, scripts | 282 |
| tabela | table | traduz | poph-store, poph | 279 |
| anexos | attachments | traduz | poph-server, poph-agent | 278 |
| carregar | load | traduz | console, poph | 278 |
| whatsapp | whatsapp | traduz | console, poph-server | 273 |
| enviar | send | traduz | poph-server, poph-matrix | 272 |
| ler | read | traduz | poph, poph-server | 270 |
| que | that | traduz | poph-fluxos, poph-server | 270 |
| fila | queue | traduz | poph-rag, poph-server | 269 |
| retrato | snapshot | traduz | poph-store, poph | 269 |
| validar | validate | traduz | poph-server, poph-types | 267 |
| conhecimento | knowledge | traduz | poph-server, poph-agent | 266 |
| segredos | secrets | traduz | poph, console | 266 |
| na | towards | traduz | poph-server, poph-matrix | 265 |
| gravado | stored | traduz | poph-server, poph-agent | 263 |
| instrucoes | instructions | traduz | poph, poph-agent | 262 |
| gravar | write | traduz | poph-store, poph-server | 261 |
| pausa | pause | traduz | poph-agent, poph-server | 261 |
| turnos | turns | traduz | poph-agent, poph-server | 261 |
| espera | wait | traduz | poph-agent, poph-server | 260 |
| ferramentas | tools | traduz | poph-agent, poph-server | 250 |
| excluir | delete | traduz | console, poph-server | 248 |
| dominio | domain | traduz | poph-store, poph-server | 246 |
| memoria | memory | traduz | poph-agent, poph-server | 245 |
| como | as | traduz | poph-store, poph-server | 239 |
| conexao | connection | traduz | poph-store, poph-server | 239 |
| escrita | writing | traduz | poph-store, poph-server | 239 |
| transicao | transition | traduz | poph-store, poph-server | 238 |
| recebidos | received | traduz | poph-agent, poph-evolution | 236 |
| fantasma | ghost | traduz | poph-matrix, poph-server | 230 |
| invalido | invalid | traduz | poph-server, poph-fluxos | 229 |
| segunda | second | traduz | poph-server, poph-store | 229 |
| cabecalhos | headers | traduz | poph-server, console | 228 |
| uso | use | traduz | poph-agent, poph-core | 227 |
| leitura | read | traduz | poph-store, poph-files | 226 |
| esperar | wait | traduz | poph-server, poph-agent | 224 |
| fichas | cases | traduz | poph-store, poph-server | 224 |
| caixa | box | traduz | poph-agent, poph-store | 223 |
| primeiro | first | traduz | poph-server, poph-agent | 222 |
| contar | count | traduz | poph-store, poph-server | 221 |
| posicao | position | traduz | poph-matrix, poph-server | 221 |
| erros | errors | traduz | poph-fluxos, poph-server | 220 |
| aberta | open | traduz | poph-store, poph-server | 219 |
| auditoria | audit | traduz | poph-server, poph-store | 219 |
| ciclo | cycle | traduz | poph-server, poph-fluxos | 216 |
| documento | document | traduz | poph-store, poph-server | 215 |
| textos | texts | traduz | poph-rag, poph-server | 215 |
| esperado | expected | traduz | poph, poph-rag | 213 |
| relogio | clock | traduz | poph-server, poph-store | 212 |
| ultimo | last | traduz | poph-server, poph-agent | 206 |
| departamentos | departments | traduz | console, poph-server | 205 |
| modo | mode | traduz | poph-server, console | 205 |
| corte | cutoff | traduz | poph-agent, poph-server | 204 |
| recusa | refusal | traduz | poph-server, poph-files | 204 |
| atualizar | update | traduz | poph-server, poph-agent | 203 |
| criado | created | traduz | poph-server, poph-store | 201 |
| respostas | responses | traduz | poph-agent, poph-server | 201 |
| limitador | limiter | traduz | poph-agent, poph-core | 199 |
| catalogo | catalog | traduz | poph-agent, poph | 197 |
| falar | speak | traduz | poph-agent, poph-server | 197 |
| leitor | reader | traduz | poph-fluxos, poph-core | 197 |
| pergunta | question | traduz | poph-jev, poph-agent | 197 |
| desfecho | outcome | traduz | poph-server, poph-agent | 196 |
| recebido | received | traduz | poph-store, poph-server | 196 |
| aberto | open | traduz | console, poph-server | 193 |
| fechada | closed | traduz | poph-store, poph-server | 193 |
| esperada | expected | traduz | poph-store, poph-server | 190 |
| tentativas | attempts | traduz | poph-server, poph-agent | 190 |
| pronta | ready | traduz | poph-store, poph-matrix | 188 |
| trechos | passages | traduz | poph-agent, poph-rag | 188 |
| definir | define | traduz | poph-server, poph-store | 187 |
| comando | command | traduz | poph, poph-plugins | 185 |
| confianca | confidence | traduz | poph-jev, poph-agent | 185 |
| cru | raw | traduz | poph-server, poph-agent | 185 |
| titulo | title | traduz | console, poph-rag | 185 |
| orgaos | orgs | traduz | poph-server, poph-store | 184 |
| contadores | counters | traduz | poph-plugins, poph-server | 183 |
| apos | after | traduz | poph-store, poph-server | 182 |
| manifesto | manifest | traduz | poph-server, console | 182 |
| subir | raise | traduz | poph-server, poph-matrix | 182 |
| remetente | sender | traduz | poph-matrix, poph-server | 181 |
| etapas | steps | traduz | poph-fluxos, poph-store | 180 |
| registros | records | traduz | poph-server, poph-matrix | 178 |
| execucoes | executions | traduz | poph-store, poph-server | 175 |
| busca | search | traduz | poph-agent, poph-rag | 174 |
| concluido | finished | traduz | poph-agent, poph-server | 173 |
| ordem | order | traduz | poph-server, poph-store | 173 |
| transporte | transport | traduz | poph-agent, poph-jev | 173 |
| instalar | install | traduz | console, poph-rag | 172 |
| carga | load | traduz | console, poph-rag | 169 |
| falsa | false | traduz | poph-server, poph-agent | 169 |
| janela | window | traduz | poph-agent, poph-server | 168 |
| transacao | transaction | traduz | poph-server, poph-store | 166 |
| trecho | passage | traduz | poph-agent, poph-rag | 166 |
| impacto | impact | traduz | poph-fluxos | 164 |
| acoes | actions | traduz | poph-fluxos, console | 163 |
| teste | test | traduz | poph, poph-server | 163 |
| fantasmas | ghosts | traduz | poph-matrix, poph-server | 160 |
| proprio | own | traduz | poph-agent, poph-server | 160 |
| futuro | future | traduz | poph-agent, poph-rag | 159 |
| exigir | require | traduz | poph-server, poph-store | 158 |
| uma | a | traduz | poph-server, poph-store | 157 |
| categorias | categories | traduz | poph, poph-store | 156 |
| endereco | address | traduz | poph-server, poph-matrix | 156 |
| entrega | delivery | traduz | poph-server, poph-agent | 154 |
| preco | price | traduz | poph-agent, poph-store | 154 |
| usuarios | users | traduz | poph-matrix, poph-server | 154 |
| argumentos | arguments | traduz | poph-agent, poph-core | 153 |
| encontrado | found | traduz | poph-store, poph-agent | 153 |
| tabelas | tables | traduz | poph-store, poph | 153 |
| ligar | link | traduz | poph-server, poph | 152 |
| vai | goes | traduz | poph-fluxos, raiz | 152 |
| atendimento | service | traduz | poph-fluxos, poph-store | 151 |
| recusado | refused | traduz | poph-server, poph-store | 151 |
| biblioteca | library | traduz | poph-server, poph-store | 148 |
| avisos | notices | traduz | poph-server, poph-fluxos | 147 |
| bruto | raw | traduz | poph-fluxos, poph-server | 147 |
| grande | large | traduz | poph-server, poph-matrix | 147 |
| rascunho | draft | traduz | console, poph-server | 147 |
| ligado | linked | traduz | poph-server, poph | 145 |
| revisoes | revisions | traduz | poph-store, poph-server | 145 |
| casos | cases | traduz | poph-jev, poph-fluxos | 144 |
| humana | human | traduz | poph-server, poph-store | 144 |
| mudou | changed | traduz | poph-fluxos, poph-server | 144 |
| sessao | session | traduz | poph, console | 144 |
| atraso | delay | traduz | poph-server, poph-agent | 143 |
| carregando | loading | traduz | console | 143 |
| dialogo | dialogue | traduz | console | 143 |
| ou | or | traduz | poph-fluxos, poph-store | 143 |
| espelho | mirror | traduz | poph-server, poph-matrix | 141 |
| tarefa | task | traduz | poph-agent, poph-server | 141 |
| confere | checks | traduz | poph-fluxos, poph-server | 139 |
| das | of | traduz | poph-server, poph-agent | 138 |
| filtro | filter | traduz | poph-store, poph-server | 138 |
| historico | history | traduz | console, poph-server | 138 |
| escolhas | choices | traduz | poph-agent, poph-jev | 137 |
| lido | read | traduz | poph, poph-server | 137 |
| limpo | clean | traduz | poph-store, poph-server | 137 |
| parte | part | traduz | poph-agent, poph-core | 137 |
| quando | when | traduz | poph-fluxos, poph-jev | 137 |
| receptor | receiver | traduz | poph-server, poph-plugins | 136 |
| ja | already | traduz | poph-server, poph-matrix | 135 |
| atualizado | updated | traduz | poph-server, poph-store | 134 |
| autor | author | traduz | poph-store, poph-agent | 134 |
| sinal | signal | traduz | poph-server, console | 134 |
| imagem | image | traduz | poph-files, poph-matrix | 133 |
| prazo | deadline | traduz | poph-fluxos, poph-server | 133 |
| buscar | search | traduz | poph-agent, poph-rag | 132 |
| aparelho | device | traduz | poph-server, poph-store | 131 |
| fora | outside | traduz | poph-server, poph-fluxos | 131 |
| limiares | thresholds | traduz | poph-jev | 131 |
| decisoes | decisions | traduz | poph-fluxos, poph-jev | 130 |
| diagnosticos | diagnostics | traduz | poph-fluxos, poph-server | 130 |
| segundo | second | traduz | poph-agent, poph-server | 130 |
| avaliar | evaluate | traduz | poph-fluxos, poph-jev | 128 |
| compilado | compiled | traduz | poph-fluxos, poph-server | 128 |
| enviado | sent | traduz | poph-server, poph-store | 128 |
| tempos | times | traduz | poph-agent, poph-store | 128 |
| envios | sends | traduz | poph-server, poph-agent | 126 |
| valores | values | traduz | poph-store, poph-server | 126 |
| inexistente | missing | traduz | poph-fluxos, poph-server | 125 |
| diagnostico | diagnostic | traduz | poph-fluxos, poph-server | 124 |
| ativo | active | traduz | poph-agent, poph | 123 |
| observador | observer | traduz | poph-agent, poph-server | 123 |
| parametros | parameters | traduz | poph-agent, poph-store | 123 |
| prefixo | prefix | traduz | poph-server, poph-matrix | 123 |
| formatar | format | traduz | poph-agent, console | 122 |
| ajustes | adjustments | traduz | poph-agent, poph-server | 121 |
| ausente | missing | traduz | poph-store, poph-fluxos | 120 |
| duracao | duration | traduz | poph-agent, poph-server | 120 |
| entidade | entity | traduz | poph-server, poph-store | 120 |
| invalida | invalid | traduz | poph-fluxos, poph-server | 120 |
| repetida | repeated | traduz | poph-server, poph-store | 120 |
| candidato | candidate | traduz | poph-jev, poph-agent | 119 |
| classificacao | classification | traduz | poph-matrix, poph-server | 119 |
| conflito | conflict | traduz | poph-store, poph-server | 119 |
| captura | capture | traduz | poph-server, poph | 118 |
| receber | receive | traduz | poph-files, poph-store | 118 |
| atendente | attendant | traduz | poph-server, poph-matrix | 116 |
| orientacao | guidance | traduz | poph-jev, poph-agent | 116 |
| encerra | closes | traduz | poph-fluxos, poph-store | 115 |
| variavel | variable | traduz | poph-fluxos, poph | 115 |
| veneno | poison | traduz | poph-server, poph-agent | 114 |
| vivo | live | traduz | poph, poph-server | 114 |
| contagem | count | traduz | poph-rag, poph-store | 113 |
| garantir | ensure | traduz | poph-matrix, poph-server | 113 |
| vetor | vector | traduz | poph-rag | 113 |
| importacao | import | traduz | poph-store, poph-server | 112 |
| membros | members | traduz | poph-server, poph-matrix | 111 |
| pendente | pending | traduz | poph-server, poph-agent | 111 |
| ativa | active | traduz | poph-agent, console | 110 |
| imprimir | print | traduz | poph, poph-fluxos | 110 |
| ajuste | adjustment | traduz | poph-jev, poph-agent | 108 |
| par | pair | traduz | poph-server, poph | 108 |
| retencao | retention | traduz | poph-retention, console | 108 |
| ignorar | ignore | traduz | poph-matrix, poph-server | 107 |
| nivel | level | traduz | poph, poph-matrix | 106 |
| transferida | transferred | traduz | poph-server, poph-store | 106 |
| desligado | disabled | traduz | poph-server, poph | 105 |
| existe | exists | traduz | poph-store, poph-server | 105 |
| dias | days | traduz | poph-retention, console | 102 |
| falhar | fail | traduz | poph-server, poph-matrix | 102 |
| proxima | next | traduz | poph-plugins, poph-server | 102 |
| quantidade | quantity | traduz | poph-agent, poph-store | 102 |
| midias | media | traduz | poph-server, poph-evolution | 101 |
| carrinho | cart | traduz | poph-store, poph-agent | 100 |
| escritas | writings | traduz | poph-server, poph-matrix | 100 |
| roteiros | scripts | traduz | poph-agent, poph-server | 100 |
| comum | common | traduz | poph-plugins, poph-agent | 99 |
| descrever | describe | traduz | poph, poph-agent | 99 |
| resto | rest | traduz | poph-server, poph | 99 |
| antigo | old | traduz | poph-matrix, poph-store | 97 |
| diretorio | directory | traduz | poph-rag, poph-server | 97 |
| enfileirar | enqueue | traduz | poph-matrix, poph-agent | 97 |
| formato | format | traduz | poph-matrix, poph-store | 97 |
| cursores | cursors | traduz | poph-server, poph-store | 96 |
| falhou | failed | traduz | poph-agent, poph-server | 96 |
| fechar | close | traduz | poph-store, poph-server | 96 |
| mostrar | show | traduz | poph-server, poph | 96 |
| liberar | release | traduz | poph-store, console | 95 |
| nenhum | none | traduz | poph-fluxos, poph-server | 95 |
| tem | has | traduz | poph, poph-server | 95 |
| baixar | download | traduz | poph-server, poph-matrix | 94 |
| existente | existing | traduz | poph-matrix, poph-store | 94 |
| tempo | time | traduz | poph-agent, poph-jev | 94 |
| segundos | seconds | traduz | poph-server, poph-rag | 93 |
| validacao | validation | traduz | poph-server, poph | 93 |
| detalhes | details | traduz | console, poph-server | 92 |
| grupos | groups | traduz | console, poph | 92 |
| objeto | object | traduz | poph-store, poph-server | 92 |
| publicar | publish | traduz | poph-server, poph-store | 92 |
| sincronizar | synchronise | traduz | poph-agent, poph-server | 91 |
| cabecalho | header | traduz | poph-server, poph-core | 90 |
| enviados | sent | traduz | poph-server, poph-agent | 90 |
| familia | family | traduz | poph-files, poph-matrix | 90 |
| instancias | instances | traduz | console, poph-server | 90 |
| instrucao | instruction | traduz | poph-fluxos, raiz | 90 |
| ate | until | traduz | poph-server, poph-agent | 89 |
| entradas | entries | traduz | poph-agent, poph-fluxos | 89 |
| saude | health | traduz | console, poph-plugins | 89 |
| achado | finding | traduz | poph-rag, poph-agent | 88 |
| parcial | partial | traduz | console, poph-store | 88 |
| pelo | by | traduz | poph-server, poph-store | 88 |
| processo | process | traduz | poph-plugins, poph-server | 88 |
| raiz | root | traduz | poph-server, poph-evolution | 88 |
| testes | tests | traduz | poph-server, poph | 88 |
| conferir | check | traduz | poph-server, poph-store | 87 |
| secao | section | traduz | console, poph-fluxos | 87 |
| buscador | searcher | traduz | poph-agent, poph | 86 |
| cancelamento | cancellation | traduz | poph-agent, poph-core | 86 |
| entrar | enter | traduz | poph-fluxos, poph-matrix | 86 |
| exige | requires | traduz | poph-fluxos, raiz | 86 |
| maior | bigger | traduz | poph-fluxos, poph-rag | 86 |
| saidas | outputs | traduz | poph-agent, poph-server | 86 |
| servir | serve | traduz | poph-server, console | 86 |
| nulo | null | traduz | poph-agent, poph-store | 85 |
| obrigatorio | required | traduz | poph-fluxos, poph-matrix | 85 |
| operacao | operation | traduz | poph-matrix, poph-server | 85 |
| salvar | save | traduz | console, poph-agent | 85 |
| candidatos | candidates | traduz | poph-jev, poph-agent | 84 |
| criada | created | traduz | poph-store, poph-server | 84 |
| inicial | initial | traduz | poph-store, poph-server | 84 |
| mundo | world | traduz | poph-server | 84 |
| onde | where | traduz | poph-fluxos, scripts | 84 |
| parar | stop | traduz | poph-server, poph-agent | 84 |
| proximo | next | traduz | poph-server, poph-agent | 84 |
| controle | control | traduz | console, poph-plugins | 83 |
| bruta | raw | traduz | poph-fluxos, poph-server | 81 |
| colunas | columns | traduz | poph-store, console | 81 |
| expiracao | expiry | traduz | poph-server, poph-matrix | 81 |
| interpretar | interpret | traduz | poph-jev, poph-evolution | 81 |
| montagem | assembly | traduz | poph-agent, poph | 81 |
| muda | changes | traduz | poph-fluxos, poph-agent | 81 |
| quem | who | traduz | poph-server, poph-agent | 81 |
| ultima | last | traduz | poph-server, poph-agent | 81 |
| exemplos | examples | traduz | poph-fluxos, poph-jev | 80 |
| funcao | function | traduz | poph-fluxos, raiz | 80 |
| globais | globals | traduz | poph-server, poph-jev | 80 |
| cortado | cut | traduz | poph-fluxos, poph-matrix | 79 |
| perguntas | questions | traduz | poph-jev, poph-agent | 79 |
| centavos | cents | traduz | poph-agent, poph-store | 78 |
| igual | equal | traduz | poph-fluxos, poph-server | 78 |
| pedaco | piece | traduz | poph-matrix, poph-server | 78 |
| contador | counter | traduz | poph-rag, poph-server | 77 |
| escopo | scope | traduz | poph-fluxos, console | 77 |
| idempotencia | idempotency | traduz | poph-store, poph-server | 77 |
| preparo | preparation | traduz | poph-agent, poph | 77 |
| publicacao | publication | traduz | poph-store, poph-server | 77 |
| tipos | kinds | traduz | poph-fluxos, poph-jev | 77 |
| canais | channels | traduz | poph-server, poph | 76 |
| coletor | collector | traduz | poph-plugins, poph-agent | 76 |
| processador | processor | traduz | poph-agent, poph-server | 76 |
| autorizacao | authorization | traduz | poph-matrix, poph-server | 75 |
| encerrar | close | traduz | poph-plugins, poph-agent | 75 |
| ignorado | ignored | traduz | poph-evolution, poph-agent | 75 |
| migracao | migration | traduz | poph-store, poph | 75 |
| argumento | argument | traduz | poph-fluxos, poph-agent | 74 |
| marcador | marker | traduz | poph-server, poph-agent | 74 |
| nada | nothing | traduz | poph-agent, poph-server | 74 |
| rodada | round | traduz | poph-agent, scripts | 74 |
| chamar | call | traduz | poph-agent, poph-server | 73 |
| classificar | classify | traduz | poph-matrix, poph-files | 73 |
| mesmo | same | traduz | poph-store, poph-fluxos | 73 |
| antiga | old | traduz | poph-fluxos, poph-store | 72 |
| preparar | prepare | traduz | console, poph-server | 72 |
| tentativa | attempt | traduz | poph-agent, poph-server | 72 |
| vira | becomes | traduz | poph-server, poph-matrix | 72 |
| chaves | keys | traduz | poph-store, poph-fluxos | 71 |
| conjunto | set | traduz | poph-jev, poph | 71 |
| termo | term | traduz | scripts, poph-agent | 71 |
| anterior | previous | traduz | poph-server, poph-fluxos | 70 |
| canonico | canonical | traduz | poph-fluxos, poph-jev | 70 |
| cifra | cipher | traduz | poph-server, poph | 70 |
| escrever | write | traduz | poph, poph-server | 70 |
| avancar | advance | traduz | poph-server, poph-agent | 69 |
| entregar | deliver | traduz | poph-server, poph-agent | 69 |
| enviando | sending | traduz | console, poph-agent | 69 |
| estruturado | structured | traduz | poph-agent, poph-server | 69 |
| filho | child | traduz | poph-plugins, poph-matrix | 69 |
| metadados | metadata | traduz | poph-files, poph-server | 69 |
| importar | import | traduz | poph-store, poph-server | 68 |
| compilar | compile | traduz | poph-server, poph-fluxos | 67 |
| espelhado | mirrored | traduz | poph-server, poph-store | 67 |
| integracoes | integrations | traduz | poph-server, console | 67 |
| novos | new | traduz | poph-fluxos, console | 67 |
| transitoria | transient | traduz | poph-server, poph-matrix | 67 |
| unica | single | traduz | poph-fluxos, poph-server | 67 |
| atender | attend | traduz | poph-server, poph-matrix | 66 |
| diferenca | difference | traduz | poph-fluxos, poph-server | 66 |
| provisionador | provisioner | traduz | poph-matrix, poph-server | 65 |
| resolucoes | resolutions | traduz | poph-fluxos, poph-store | 65 |
| criacao | creation | traduz | poph-matrix, poph-server | 64 |
| passo | step | traduz | poph-server, poph-fluxos | 64 |
| dois | two | traduz | poph-fluxos, poph-server | 63 |
| todas | all | traduz | poph-server, poph-store | 63 |
| ultimas | last | traduz | poph-agent, scripts | 63 |
| achados | findings | traduz | poph-agent, poph-rag | 62 |
| caractere | character | traduz | poph-store, poph-matrix | 62 |
| carimbo | stamp | traduz | poph-server, poph-fluxos | 62 |
| confiavel | reliable | traduz | poph-jev, poph-agent | 62 |
| identificador | identifier | traduz | poph-matrix, poph-server | 62 |
| ouvinte | listener | traduz | poph-matrix, poph-server | 62 |
| assistente | assistant | traduz | poph-agent, poph-store | 61 |
| humano | human | traduz | poph-server, poph-store | 61 |
| rodando | running | traduz | poph-server, poph-agent | 61 |
| vazia | empty | traduz | poph-agent, poph-store | 61 |
| evolucao | evolution | traduz | poph-server | 60 |
| recusada | refused | traduz | poph-agent, poph | 60 |
| referencia | reference | traduz | poph, poph-rag | 60 |
| vaga | slot | traduz | poph-agent, poph-rag | 60 |
| apagar | delete | traduz | poph-rag, poph-store | 59 |
| caracteres | characters | traduz | poph-files, poph-rag | 59 |
| laco | loop | traduz | poph-server, poph-agent | 59 |
| lida | read | traduz | poph-server, poph-store | 59 |
| previa | preview | traduz | console, poph-server | 59 |
| publicada | published | traduz | poph-server, poph-matrix | 59 |
| trava | lock | traduz | poph-rag, poph-fluxos | 59 |
| vistos | seen | traduz | poph-fluxos, poph-server | 59 |
| marcar | mark | traduz | poph-store, poph-server | 58 |
| perfil | profile | traduz | poph-matrix, poph-server | 58 |
| unidade | unit | traduz | poph-agent, poph-rag | 58 |
| acertos | hits | traduz | poph-rag, poph-jev | 57 |
| agiram | acted | traduz | poph-fluxos, poph-store | 57 |
| ativos | active | traduz | poph-server, poph-store | 57 |
| longo | long | traduz | poph-rag, poph-fluxos | 57 |
| armazem | store | traduz | poph-retention, poph | 56 |
| atualizacoes | updates | traduz | poph-store, poph-server | 56 |
| codificar | encode | traduz | poph-matrix, poph-core | 56 |
| nem | nor | traduz | poph-server, poph-store | 56 |
| politica | policy | traduz | poph-evolution, poph-agent | 56 |
| risco | risk | traduz | poph-jev, poph-agent | 56 |
| transferir | transfer | traduz | poph-store, poph-jev | 56 |
| atuais | current | traduz | console, poph-rag | 55 |
| contem | contains | traduz | poph-fluxos, poph-server | 55 |
| declarado | declared | traduz | poph-server, poph-evolution | 55 |
| rotas | routes | traduz | console, poph-server | 55 |
| confirmar | confirm | traduz | console, poph-server | 54 |
| definitiva | definitive | traduz | poph-matrix, poph-server | 54 |
| dia | day | traduz | poph-matrix, poph-server | 54 |
| entre | between | traduz | poph-fluxos, poph-server | 54 |
| liberados | released | traduz | poph-agent, poph-jev | 54 |
| mesma | same | traduz | poph-fluxos, poph-store | 54 |
| palavra | word | traduz | scripts, poph-rag | 54 |
| repetido | repeated | traduz | poph-fluxos, poph-store | 54 |
| assinatura | signature | traduz | poph-server, poph-files | 53 |
| comparar | compare | traduz | poph-server, poph | 53 |
| compartilhado | shared | traduz | poph-rag, poph-agent | 53 |
| posicionar | position | traduz | poph-matrix, poph-server | 53 |
| condicoes | conditions | traduz | poph-store, poph-agent | 52 |
| convidados | guests | traduz | poph-matrix, poph-server | 52 |
| desde | since | traduz | poph-store, poph-server | 51 |
| marca | mark | traduz | poph-server, poph-matrix | 51 |
| poder | power | traduz | poph-matrix, scripts | 51 |
| relatorios | reports | traduz | poph, poph-store | 51 |
| errado | wrong | traduz | poph-server, poph-fluxos | 50 |
| forma | form | traduz | poph-fluxos, poph | 50 |
| gaveta | drawer | traduz | console | 50 |
| maximo | maximum | traduz | poph-agent, poph-jev | 50 |
| orcamento | budget | traduz | poph-agent, poph-rag | 50 |
| problemas | problems | traduz | poph, poph-server | 50 |
| redacao | writing | traduz | poph-matrix, poph-server | 50 |
| somar | sum | traduz | poph-plugins, poph-agent | 50 |
| apoio | support | traduz | poph, poph-core | 49 |
| convite | invitation | traduz | poph-matrix, poph-server | 49 |
| despachar | dispatch | traduz | poph-agent, poph-server | 49 |
| orfaos | orphans | traduz | poph-retention, poph | 49 |
| cena | scene | traduz | poph-server | 48 |
| leituras | reads | traduz | poph-server, console | 48 |
| permissao | permission | traduz | poph-core, poph-matrix | 48 |
| vez | time | traduz | poph-server, poph-fluxos | 48 |
| aplicar | apply | traduz | poph-store, poph-server | 47 |
| decidir | decide | traduz | poph-matrix, poph-server | 47 |
| devolve | returns | traduz | poph-server, poph-store | 47 |
| edicao | edit | traduz | console, poph-matrix | 47 |
| gravados | stored | traduz | poph-server, poph-plugin-eco | 47 |
| margem | margin | traduz | poph-jev, console | 47 |
| mostra | shows | traduz | poph, poph-fluxos | 47 |
| transitorio | transient | traduz | poph-matrix, poph-server | 47 |
| usado | used | traduz | poph-fluxos, poph-agent | 47 |
| duas | two | traduz | poph-server, poph-agent | 46 |
| esquema | schema | traduz | poph-server, scripts | 46 |
| locais | local | traduz | poph-fluxos, poph-store | 46 |
| nos | us | traduz | poph-fluxos, poph-server | 46 |
| remoto | remote | traduz | poph, poph-server | 46 |
| rodadas | rounds | traduz | poph-agent, poph-server | 46 |
| semear | seed | traduz | poph-server, poph-store | 46 |
| aguardando | waiting | traduz | poph-fluxos, poph-agent | 45 |
| falta | missing | traduz | poph-matrix, poph-server | 45 |
| gerar | generate | traduz | poph, scripts | 45 |
| inativo | inactive | traduz | poph-store, poph-agent | 45 |
| recebida | received | traduz | poph-agent, poph-server | 45 |
| rede | network | traduz | poph-matrix, console | 45 |
| removido | removed | traduz | poph-fluxos, console | 45 |
| volta | return | traduz | poph-server, poph-fluxos | 45 |
| agrupamento | grouping | traduz | poph-agent, poph-server | 44 |
| alteradas | changed | traduz | poph-store, poph-server | 44 |
| ativas | active | traduz | poph-store, poph-server | 44 |
| decodificar | decode | traduz | poph-server, poph-matrix | 44 |
| dos | of | traduz | poph-fluxos, console | 44 |
| escritor | writer | traduz | poph-store, poph-matrix | 44 |
| gravacao | recording | traduz | poph-store, poph-server | 44 |
| intervalo | interval | traduz | poph-server, poph-agent | 44 |
| removida | removed | traduz | poph-fluxos, poph-matrix | 44 |
| sentido | meaning | traduz | poph-matrix, poph-server | 44 |
| transferencia | transfer | traduz | poph-agent, poph-server | 44 |
| aceito | accepted | traduz | poph-files, poph-server | 43 |
| desligada | disabled | traduz | poph-server, poph-store | 43 |
| extensao | extension | traduz | poph-files, poph-server | 43 |
| opcao | option | traduz | poph-fluxos, console | 43 |
| palavras | words | traduz | poph-rag, scripts | 43 |
| pela | by | traduz | poph-server, poph | 43 |
| similaridade | similarity | traduz | poph-rag, poph-agent | 43 |
| sistema | system | traduz | poph-agent, poph-store | 43 |
| varredura | scan | traduz | poph-retention, poph-store | 43 |
| vetores | vectors | traduz | poph-rag, poph-core | 43 |
| coleta | collection | traduz | poph-fluxos | 42 |
| criados | created | traduz | poph-server, poph-agent | 42 |
| extracao | extraction | traduz | poph-files, poph-server | 42 |
| nomeado | named | traduz | poph-fluxos, poph-agent | 42 |
| passar | pass | traduz | poph-server, poph-agent | 42 |
| ruim | poor | traduz | poph-retention, poph | 42 |
| dado | given | traduz | poph-fluxos, poph-server | 41 |
| encerrada | closed | traduz | poph-server, poph-rag | 41 |
| encerramento | closure | traduz | console, poph-agent | 41 |
| enviada | sent | traduz | poph-server, poph-matrix | 41 |
| interna | internal | traduz | poph-agent | 41 |
| minusculo | lowercase | traduz | poph-fluxos, poph-evolution | 41 |
| modelos | models | traduz | poph, poph-rag | 41 |
| adiadas | deferred | traduz | poph-server, poph-agent | 40 |
| conflitos | conflicts | traduz | poph-store, poph-server | 40 |
| custo | cost | traduz | poph-agent, poph-store | 40 |
| forcar | force | traduz | console, poph | 40 |
| medicao | measurement | traduz | scripts, poph-agent | 40 |
| posicionada | positioned | traduz | poph-matrix, poph-server | 40 |
| processar | process | traduz | poph-agent, poph-server | 40 |
| resultados | results | traduz | poph-agent, poph-store | 40 |
| valido | valid | traduz | poph-server, poph-fluxos | 40 |
| agenda | schedule | traduz | poph-agent, poph-server | 39 |
| ano | year | traduz | poph-matrix, poph-server | 39 |
| comandos | commands | traduz | poph-plugins, poph-store | 39 |
| conectado | connected | traduz | poph-server, console | 39 |
| definicao | definition | traduz | poph-store, poph-server | 39 |
| entao | then | traduz | poph-fluxos, raiz | 39 |
| obtido | obtained | traduz | poph-rag, poph-jev | 39 |
| recarga | reload | traduz | console, poph-server | 39 |
| senao | else | traduz | poph-fluxos, raiz | 39 |
| simples | simple | traduz | poph-fluxos, poph-server | 39 |
| classe | class | traduz | poph-agent, console | 38 |
| compila | compiles | traduz | poph-fluxos, poph | 38 |
| curso | course | traduz | poph-fluxos, console | 38 |
| indisponivel | unavailable | traduz | poph-server, poph-core | 38 |
| nunca | never | traduz | poph-evolution, poph-server | 38 |
| parada | stop | traduz | poph-server, poph-fluxos | 38 |
| poderes | powers | traduz | poph-matrix | 38 |
| resolve | resolves | traduz | scripts, poph-fluxos | 38 |
| todos | all | traduz | poph, poph-server | 38 |
| validade | validity | traduz | poph-server, poph-jev | 38 |
| amostras | samples | traduz | poph-store, poph-server | 37 |
| anotadora | annotator | traduz | poph-server | 37 |
| apagou | deleted | traduz | poph-store, poph-matrix | 37 |
| consultar | query | traduz | poph-agent, poph | 37 |
| criando | creating | traduz | console, poph-store | 37 |
| desconhecido | unknown | traduz | poph-fluxos, poph-agent | 37 |
| extraido | extracted | traduz | poph-agent, poph-files | 37 |
| servico | service | traduz | poph-server, poph | 37 |
| temperatura | temperature | traduz | poph-core, poph-store | 37 |
| volatil | volatile | traduz | poph-agent | 37 |
| acima | above | traduz | poph-server, poph-fluxos | 36 |
| encaminhar | forward | traduz | poph-fluxos, poph-server | 36 |
| enviadas | sent | traduz | poph-server, poph-agent | 36 |
| fica | stays | traduz | poph-server, poph-fluxos | 36 |
| fontes | sources | traduz | poph, console | 36 |
| funcoes | functions | traduz | poph-fluxos, poph-agent | 36 |
| geracao | generation | traduz | poph-plugins, poph-rag | 36 |
| impresso | printed | traduz | poph-fluxos, poph | 36 |
| proibido | forbidden | traduz | poph-server, poph | 36 |
| salvo | saved | traduz | console, poph-server | 36 |
| sujo | dirty | traduz | console | 36 |
| tentar | try | traduz | console, poph-matrix | 36 |
| tocar | touch | traduz | poph-store, poph-server | 36 |
| avisar | notify | traduz | poph-server, console | 35 |
| buscas | searches | traduz | poph-agent | 35 |
| convites | invitations | traduz | poph-matrix, poph-server | 35 |
| dentro | inside | traduz | poph-fluxos, poph | 35 |
| fecha | closes | traduz | poph-fluxos, poph-agent | 35 |
| frase | sentence | traduz | poph-rag, poph-fluxos | 35 |
| lidos | read | traduz | poph-fluxos, poph-rag | 35 |
| limpar | clean | traduz | scripts, poph-agent | 35 |
| plano | plan | traduz | poph-server, poph-store | 35 |
| sao | are | traduz | poph-store, poph-matrix | 35 |
| terceira | third | traduz | poph-server, poph-agent | 35 |
| verificacao | verification | traduz | poph, poph-evolution | 35 |
| aguardar | wait | traduz | poph-agent, poph-core | 34 |
| comportamento | behaviour | traduz | poph-agent | 34 |
| conta | counts | traduz | poph-server, poph-store | 34 |
| criterio | criterion | traduz | poph-jev | 34 |
| espelhar | mirror | traduz | poph-matrix, poph-server | 34 |
| exemplo | example | traduz | poph-fluxos, poph-jev | 34 |
| fixa | fixed | traduz | poph-rag, poph-matrix | 34 |
| legenda | caption | traduz | poph-server, poph-evolution | 34 |
| legivel | readable | traduz | poph-core, poph-agent | 34 |
| ligacao | link | traduz | console, poph | 34 |
| ligada | linked | traduz | poph-server, poph-matrix | 34 |
| navegar | navigate | traduz | console | 34 |
| pertence | belongs | traduz | poph-store | 34 |
| pode | can | traduz | console, poph | 34 |
| variaveis | variables | traduz | poph-fluxos, poph | 34 |
| aceita | accepts | traduz | poph-fluxos, poph-server | 33 |
| baratas | cheap | traduz | poph-agent, poph-rag | 33 |
| comparacao | comparison | traduz | poph, poph-fluxos | 33 |
| declarada | declared | traduz | poph-server, poph-files | 33 |
| estados | states | traduz | poph-server, poph-plugins | 33 |
| gravadas | stored | traduz | poph-server, poph-agent | 33 |
| notificar | notify | traduz | poph-fluxos, poph-server | 33 |
| oferecidas | offered | traduz | poph-agent | 33 |
| reabrir | reopen | traduz | poph-server, poph-agent | 33 |
| renderizar | render | traduz | poph-fluxos, scripts | 33 |
| temporaria | temporary | traduz | poph, console | 33 |
| aplicavel | applicable | traduz | poph-jev | 32 |
| botao | button | traduz | console | 32 |
| carregamento | loading | traduz | poph-fluxos, poph-rag | 32 |
| carrinhos | carts | traduz | poph-agent, poph-store | 32 |
| diferente | different | traduz | poph-fluxos, poph | 32 |
| leva | takes | traduz | poph-matrix, poph-store | 32 |
| loja | store | traduz | poph-server, console | 32 |
| ordenadas | sorted | traduz | poph-store, poph-jev | 32 |
| redacoes | writings | traduz | poph-server, poph-matrix | 32 |
| requisicoes | requests | traduz | poph-matrix, scripts | 32 |
| auditado | audited | traduz | poph-server, poph-store | 31 |
| candidata | candidate | traduz | console, poph-jev | 31 |
| condicao | condition | traduz | poph-fluxos, poph-server | 31 |
| credencial | credential | traduz | poph-matrix, scripts | 31 |
| divergente | divergent | traduz | poph-fluxos, poph-server | 31 |
| efetivos | effective | traduz | poph-jev, poph-agent | 31 |
| esperados | expected | traduz | poph-server, poph-rag | 31 |
| fator | factor | traduz | poph-server, poph-agent | 31 |
| hibrida | hybrid | traduz | poph-rag, poph-server | 31 |
| latencia | latency | traduz | poph-server, poph-jev | 31 |
| mudar | change | traduz | poph-store, console | 31 |
| preparado | prepared | traduz | poph-rag, poph-server | 31 |
| recusar | refuse | traduz | poph-agent, poph-server | 31 |
| ver | see | traduz | poph-server, console | 31 |
| alterada | changed | traduz | poph-server, console | 30 |
| apagadas | deleted | traduz | poph-server, poph-matrix | 30 |
| aparece | appears | traduz | poph-server, poph | 30 |
| cai | falls | traduz | poph-agent, poph-matrix | 30 |
| capturados | captured | traduz | poph-server | 30 |
| continua | continues | traduz | poph-store, poph-agent | 30 |
| divergencias | divergences | traduz | poph, poph-server | 30 |
| feita | done | traduz | poph-server, poph | 30 |
| geral | general | traduz | console, poph-agent | 30 |
| indexar | index | traduz | poph-rag, poph-agent | 30 |
| mes | month | traduz | poph-matrix, poph-server | 30 |
| motivos | reasons | traduz | console, poph-retention | 30 |
| mutacao | mutation | traduz | poph-fluxos | 30 |
| pareamento | pairing | traduz | poph-evolution, poph-server | 30 |
| profundidade | depth | traduz | poph-rag, poph-fluxos | 30 |
| propriedade | property | traduz | console | 30 |
| relevancia | relevance | traduz | poph-rag, poph-agent | 30 |
| segurar | hold | traduz | poph-server, console | 30 |
| simultaneos | simultaneous | traduz | poph-rag, poph-core | 30 |
| tratar | handle | traduz | poph-server, poph-plugins | 30 |
| valida | validates | traduz | poph-fluxos, poph-server | 30 |
| verificacoes | verifications | traduz | poph | 30 |
| abertura | opening | traduz | poph-store, poph-server | 29 |
| codigos | codes | traduz | poph-fluxos, poph-server | 29 |
| inteiro | integer | traduz | poph, poph-agent | 29 |
| lidas | read | traduz | poph-store, console | 29 |
| migrar | migrate | traduz | poph-store, poph | 29 |
| pede | asks | traduz | poph-fluxos, poph-jev | 29 |
| posicoes | positions | traduz | poph-fluxos, poph-server | 29 |
| quantas | how_many | traduz | poph-server, poph-store | 29 |
| relativo | relative | traduz | poph-server, poph | 29 |
| remover | remove | traduz | poph-agent, poph-matrix | 29 |
| retentativas | retries | traduz | poph-core, poph-evolution | 29 |
| saltos | jumps | traduz | poph-fluxos, poph-agent | 29 |
| severidade | severity | traduz | poph-fluxos, poph-server | 29 |
| vinculo | link | traduz | console, poph-server | 29 |
| adiados | deferred | traduz | poph-agent, poph-server | 28 |
| ajuda | help | traduz | poph-fluxos, poph | 28 |
| deixa | leaves | traduz | poph-store, poph-fluxos | 28 |
| dono | owner | traduz | poph-retention, poph-store | 28 |
| esgotado | exhausted | traduz | poph-plugins, poph-agent | 28 |
| falhos | faulty | traduz | poph-matrix, poph-server | 28 |
| inferencia | inference | traduz | poph-rag, poph-agent | 28 |
| inseridas | inserted | traduz | console, poph-store | 28 |
| mantem | keeps | traduz | poph-agent, poph-matrix | 28 |
| probabilidades | probabilities | traduz | poph-jev, poph-agent | 28 |
| provisionar | provision | traduz | poph-matrix, poph-server | 28 |
| reserva | reservation | traduz | poph-agent, poph-server | 28 |
| responde | answers | traduz | poph-server, poph-agent | 28 |
| telemetria | telemetry | traduz | console | 28 |
| usada | used | traduz | poph-fluxos, poph-server | 28 |
| vizinha | neighbouring | traduz | poph-server, poph | 28 |
| adiado | deferred | traduz | poph-server, poph-agent | 27 |
| atualizada | updated | traduz | poph-store, console | 27 |
| cada | each | traduz | poph-server, poph-fluxos | 27 |
| configurar | configure | traduz | poph-server, poph | 27 |
| definitivos | definitive | traduz | poph-server, poph-matrix | 27 |
| dona | owner | traduz | poph-server, poph-store | 27 |
| esperas | waits | traduz | poph-agent, poph-fluxos | 27 |
| foto | photo | traduz | poph-retention, poph-server | 27 |
| liberado | released | traduz | poph-server, poph-agent | 27 |
| operacoes | operations | traduz | poph-server, poph | 27 |
| posicional | positional | traduz | poph-fluxos | 27 |
| produzido | produced | traduz | poph-fluxos, poph-agent | 27 |
| recarregar | reload | traduz | poph-server, console | 27 |
| tokenizador | tokeniser | traduz | poph-rag | 27 |
| usa | uses | traduz | poph-agent, poph-store | 27 |
| validas | valid | traduz | poph-jev, poph-fluxos | 27 |
| velho | old | traduz | poph-retention, poph-agent | 27 |
| abandonar | abandon | traduz | poph-server, poph-fluxos | 26 |
| adquirir | acquire | traduz | poph-agent, poph-core | 26 |
| comercial | commercial | traduz | poph-server, poph-store | 26 |
| compor | compose | traduz | poph-agent, poph | 26 |
| cria | creates | traduz | poph-store, poph | 26 |
| demora | delay | traduz | poph-agent | 26 |
| dispositivo | device | traduz | poph-matrix, poph-server | 26 |
| dormir | sleep | traduz | scripts | 26 |
| entra | enters | traduz | poph-agent, poph-server | 26 |
| estava | was | traduz | poph-store, poph | 26 |
| filas | queues | traduz | console, poph-agent | 26 |
| grafo | graph | traduz | poph-fluxos | 26 |
| livre | free | traduz | poph-server, poph-fluxos | 26 |
| lotes | batches | traduz | poph-rag | 26 |
| publicadas | published | traduz | poph-server | 26 |
| rodar | run | traduz | poph-agent, poph | 26 |
| roteador | router | traduz | poph-server, poph-matrix | 26 |
| soma | sum | traduz | poph-jev, poph-agent | 26 |
| unico | single | traduz | poph-server, poph-fluxos | 26 |
| alterar | modify | traduz | console, poph | 25 |
| conferido | checked | traduz | poph, poph-store | 25 |
| despedida | farewell | traduz | poph-jev, console | 25 |
| disponivel | available | traduz | poph-rag, poph-jev | 25 |
| encontrada | found | traduz | poph-store, poph-agent | 25 |
| exigidos | required | traduz | poph-fluxos, poph-agent | 25 |
| instalacao | installation | traduz | poph, poph-rag | 25 |
| linguagem | language | traduz | scripts, poph-fluxos | 25 |
| movida | moved | traduz | poph-matrix, poph-server | 25 |
| norma | norm | traduz | poph-rag | 25 |
| normalizar | normalise | traduz | poph-store, console | 25 |
| opcional | optional | traduz | poph-store, poph-agent | 25 |
| peso | weight | traduz | poph-fluxos | 25 |
| reais | actual | traduz | poph-agent, poph-server | 25 |
| recuperado | recovered | traduz | poph-agent | 25 |
| respeita | respects | traduz | poph-agent, poph-server | 25 |
| semaforo | semaphore | traduz | poph-agent, poph-rag | 25 |
| tarefas | tasks | traduz | poph-server, poph-rag | 25 |
| vivos | live | traduz | poph-agent, scripts | 25 |
| apaga | deletes | traduz | poph-retention, poph-server | 24 |
| avulsa | one_off | traduz | poph-server | 24 |
| baixado | downloaded | traduz | poph-server, poph-matrix | 24 |
| diz | says | traduz | poph, poph-fluxos | 24 |
| exclusao | exclusion | traduz | poph-server, console | 24 |
| fechado | closed | traduz | poph-server, poph-evolution | 24 |
| filtros | filters | traduz | console, poph-server | 24 |
| habilitada | enabled | traduz | poph-agent, poph-jev | 24 |
| limpa | clean | traduz | poph-server, poph-store | 24 |
| manifestos | manifests | traduz | poph-server | 24 |
| novas | new | traduz | poph-server, poph-matrix | 24 |
| recibo | receipt | traduz | poph-evolution, poph-fluxos | 24 |
| recuo | backoff | traduz | poph-fluxos, poph | 24 |
| requisicao | request | traduz | poph-matrix, poph-core | 24 |
| sobrescrever | overwrite | traduz | poph-server, console | 24 |
| sonda | probe | traduz | poph-server | 24 |
| travar | lock | traduz | poph-agent, poph-jev | 24 |
| velha | old | traduz | poph-server, poph | 24 |
| autorizado | authorized | traduz | poph-server, console | 23 |
| baixa | low | traduz | poph-jev, poph-agent | 23 |
| cifrados | encrypted | traduz | poph-store, poph | 23 |
| complementos | complements | traduz | poph-server | 23 |
| congelados | frozen | traduz | scripts | 23 |
| consultas | queries | traduz | poph-rag, poph-server | 23 |
| cortar | cut | traduz | poph-agent, poph-matrix | 23 |
| demais | others | traduz | poph-fluxos, poph-evolution | 23 |
| dourada | golden | traduz | poph-fluxos | 23 |
| encerrado | closed | traduz | poph-core, poph-store | 23 |
| instante | instant | traduz | poph-retention, poph-store | 23 |
| juntos | together | traduz | poph-server, poph-matrix | 23 |
| melhores | best | traduz | poph-rag | 23 |
| organizacoes | organisations | traduz | console, poph-server | 23 |
| programa | program | traduz | poph-plugin-eco, poph-plugins | 23 |
| reindexar | reindex | traduz | poph, poph-server | 23 |
| repeticao | repetition | traduz | poph-evolution, poph-agent | 23 |
| repetir | repeat | traduz | poph-server, poph-matrix | 23 |
| resolvedor | resolver | traduz | poph-agent, poph-plugin-eco | 23 |
| salvando | saving | traduz | console | 23 |
| segue | follows | traduz | poph-server, poph-agent | 23 |
| segura | safe | traduz | poph-agent, poph-server | 23 |
| selecionada | selected | traduz | console | 23 |
| situacoes | situations | traduz | console, poph-jev | 23 |
| validador | validator | traduz | poph-fluxos | 23 |
| visita | visit | traduz | poph-fluxos, poph-server | 23 |
| alfa | alpha | traduz | poph-agent, poph-fluxos | 22 |
| atendimentos | services | traduz | poph-fluxos, poph | 22 |
| atualizadas | updated | traduz | console, poph-server | 22 |
| automatico | automatic | traduz | poph-store, poph-server | 22 |
| booleano | boolean | traduz | poph-fluxos, poph | 22 |
| chegada | arrival | traduz | poph-agent, poph-fluxos | 22 |
| combinador | combinator | traduz | poph-fluxos | 22 |
| continuacao | continuation | traduz | poph-agent | 22 |
| disfarce | disguise | traduz | poph-server, poph-matrix | 22 |
| exibicao | display | traduz | poph-matrix, poph-fluxos | 22 |
| faltando | missing | traduz | poph-fluxos, poph-agent | 22 |
| final | final | traduz | poph-matrix, poph-fluxos | 22 |
| finalizacao | finalisation | traduz | poph-agent, poph-store | 22 |
| hora | hour | traduz | poph-fluxos, poph-store | 22 |
| inclui | includes | traduz | poph-agent, poph-store | 22 |
| inicia | starts | traduz | poph-agent, poph-rag | 22 |
| livres | free | traduz | poph-store, poph-core | 22 |
| lixo | garbage | traduz | poph-plugins, poph-core | 22 |
| mais | more | traduz | poph-server, poph-store | 22 |
| meio | middle | traduz | poph-server, poph-store | 22 |
| pai | parent | traduz | poph-matrix, poph | 22 |
| precisa | needs | traduz | scripts, poph-fluxos | 22 |
| propria | own | traduz | poph-server, poph-agent | 22 |
| removidos | removed | traduz | poph-rag, poph | 22 |
| suporte | support | traduz | poph-matrix, poph-store | 22 |
| vezes | times | traduz | poph-server, poph-fluxos | 22 |
| aba | tab | traduz | console | 21 |
| auditorias | audits | traduz | poph-retention, poph-store | 21 |
| cargas | loads | traduz | poph-fluxos, poph-server | 21 |
| derivado | derived | traduz | poph-store, poph | 21 |
| derivar | derive | traduz | poph-server, poph | 21 |
| duplicado | duplicate | traduz | poph-store, poph-agent | 21 |
| extrator | extractor | traduz | poph-files, poph-server | 21 |
| feito | done | traduz | poph-server, poph-retention | 21 |
| gerados | generated | traduz | poph-rag, console | 21 |
| malformado | malformed | traduz | poph-fluxos, poph-server | 21 |
| mediana | median | traduz | poph-store, poph-agent | 21 |
| menor | smaller | traduz | poph-fluxos, poph-server | 21 |
| mim | me | traduz | poph-evolution, poph-server | 21 |
| minuscula | lowercase | traduz | poph-files, poph-server | 21 |
| orfao | orphan | traduz | poph-retention, poph-store | 21 |
| organizacao | organisation | traduz | console, poph-fluxos | 21 |
| passos | steps | traduz | poph-server, poph-agent | 21 |
| presenca | presence | traduz | poph-evolution, poph | 21 |
| proprias | own | traduz | poph-agent, poph | 21 |
| quantos | how_many | traduz | poph-server, poph-fluxos | 21 |
| recebidas | received | traduz | poph-agent, poph-jev | 21 |
| recente | recent | traduz | poph-retention, poph-store | 21 |
| segmento | segment | traduz | poph-server, poph-matrix | 21 |
| sim | yes | traduz | poph-fluxos, poph | 21 |
| sufixo | suffix | traduz | poph-matrix, poph | 21 |
| telefone | phone | traduz | poph-matrix, poph-server | 21 |
| vinculado | linked | traduz | console, poph | 21 |
| visiveis | visible | traduz | console, scripts | 21 |
| achadas | found | traduz | poph-agent, poph-fluxos | 20 |
| adicionar | add | traduz | poph-agent, poph-rag | 20 |
| andamento | progress | traduz | poph-agent, poph-rag | 20 |
| arquivado | archived | traduz | poph-store, console | 20 |
| celulas | cells | traduz | poph-server, scripts | 20 |
| definitivas | definitive | traduz | poph-agent, poph-server | 20 |
| derrubar | drop | traduz | poph-server, poph-store | 20 |
| disco | disk | traduz | poph-evolution, poph-server | 20 |
| entregue | delivered | traduz | poph-server, poph-agent | 20 |
| exportar | export | traduz | poph, poph-server | 20 |
| fala | speaks | traduz | poph-agent, poph-server | 20 |
| guardadas | guarded | traduz | poph-server, poph-fluxos | 20 |
| listagem | listing | traduz | poph-store, poph-server | 20 |
| lugar | place | traduz | poph-fluxos, poph-server | 20 |
| medir | measure | traduz | poph-server, poph-jev | 20 |
| mostrado | shown | traduz | poph, poph-server | 20 |
| nota | note | traduz | poph-rag, poph-agent | 20 |
| percentil | percentile | traduz | poph-jev, poph-store | 20 |
| portugues | portuguese | traduz | poph, poph-server | 20 |
| redigir | draft | traduz | poph-matrix, poph-server | 20 |
| regras | rules | traduz | poph-fluxos, poph-matrix | 20 |
| revogar | revoke | traduz | poph-store, console | 20 |
| secoes | sections | traduz | poph-agent | 20 |
| vagas | slots | traduz | poph-rag, poph-agent | 20 |
| virtuais | virtual | traduz | poph-matrix, poph-server | 20 |
| vitima | victim | traduz | poph-server, poph-fluxos | 20 |
| abaixo | below | traduz | poph-agent, poph-jev | 19 |
| alvos | target | traduz | poph-rag, poph-matrix | 19 |
| aspas | quotes | traduz | poph-fluxos, poph | 19 |
| certificado | certificate | traduz | poph-core, poph-jev | 19 |
| cliques | clicks | traduz | console | 19 |
| conexoes | connections | traduz | poph-core, poph-jev | 19 |
| contrato | contract | traduz | poph-matrix, poph-jev | 19 |
| criadas | created | traduz | poph-server, console | 19 |
| decorrido | elapsed | traduz | poph-agent, poph-files | 19 |
| escolhido | chosen | traduz | poph-agent, poph-server | 19 |
| espelhadas | mirrored | traduz | poph-server, scripts | 19 |
| imagens | images | traduz | poph-agent, poph-files | 19 |
| impressao | printout | traduz | console | 19 |
| indexacao | indexing | traduz | poph-agent, poph-rag | 19 |
| invalidas | invalid | traduz | poph-fluxos, poph-agent | 19 |
| latencias | latencies | traduz | poph-jev, poph-agent | 19 |
| quantizar | quantise | traduz | poph-rag | 19 |
| transitorios | transient | traduz | poph-server, poph-matrix | 19 |
| unicos | single | traduz | poph-store, poph-fluxos | 19 |
| usados | used | traduz | poph-fluxos, poph-agent | 19 |
| abstencao | abstention | traduz | poph-jev, poph-agent | 18 |
| acento | accent | traduz | console, poph-rag | 18 |
| avisadas | notified | traduz | poph-server | 18 |
| celula | cell | traduz | poph-files, console | 18 |
| configurado | configured | traduz | console, poph-evolution | 18 |
| confirmacao | confirmation | traduz | poph-agent, console | 18 |
| convidado | guest | traduz | poph-server, poph | 18 |
| criterios | criteria | traduz | poph-jev, poph-agent | 18 |
| curto | short | traduz | poph-matrix, poph-server | 18 |
| declarados | declared | traduz | poph-server, poph-fluxos | 18 |
| digitos | digits | traduz | poph-fluxos, poph-matrix | 18 |
| enganosa | misleading | traduz | poph-files, poph-server | 18 |
| esperadas | expected | traduz | poph-server, poph-store | 18 |
| expulsoes | evictions | traduz | poph-fluxos, poph-server | 18 |
| guardado | stored | traduz | poph-server, poph-store | 18 |
| informado | informed | traduz | poph-jev, poph-server | 18 |
| limiar | threshold | traduz | poph-agent, poph-jev | 18 |
| migracoes | migrations | traduz | poph-store, poph | 18 |
| paginas | pages | traduz | poph-server, poph-fluxos | 18 |
| periodo | period | traduz | poph-server | 18 |
| pico | peak | traduz | poph-rag, poph-files | 18 |
| quadro | board | traduz | console, scripts | 18 |
| selecionado | selected | traduz | console | 18 |
| simular | simulate | traduz | poph-retention, poph-store | 18 |
| agendar | schedule | traduz | poph-fluxos, poph-agent | 17 |
| alheio | foreign | traduz | poph-server, poph-agent | 17 |
| apagados | deleted | traduz | poph-retention, poph-store | 17 |
| avancou | advanced | traduz | poph-store, poph-fluxos | 17 |
| baixada | downloaded | traduz | poph-evolution, poph-server | 17 |
| coletada | collected | traduz | poph-fluxos | 17 |
| concluidos | finished | traduz | poph-agent, poph-rag | 17 |
| consultado | queried | traduz | poph-jev, poph-agent | 17 |
| cosseno | cosine | traduz | poph-rag | 17 |
| deixou | left | traduz | poph-fluxos | 17 |
| desconhecida | unknown | traduz | poph-server, console | 17 |
| escolher | choose | traduz | console, poph-server | 17 |
| feitas | done | traduz | poph-store, poph-server | 17 |
| filhos | children | traduz | poph-fluxos, poph-server | 17 |
| folha | sheet | traduz | poph-fluxos | 17 |
| guardados | guarded | traduz | poph-evolution, poph-server | 17 |
| hoje | today | traduz | poph-agent, poph-server | 17 |
| isolamento | isolation | traduz | poph-server, poph-files | 17 |
| nomeavel | nameable | traduz | poph-server | 17 |
| pares | pairs | traduz | poph, poph-fluxos | 17 |
| perfis | profiles | traduz | poph-server | 17 |
| reenvios | resends | traduz | poph-agent, poph-server | 17 |
| renomeacoes | renames | traduz | poph-server | 17 |
| retomada | resumption | traduz | poph-agent, poph-matrix | 17 |
| simulacao | simulation | traduz | poph, poph-retention | 17 |
| terceiro | third | traduz | poph-server, poph-agent | 17 |
| tres | three | traduz | poph-fluxos, poph-jev | 17 |
| trocado | swapped | traduz | poph-server, poph-matrix | 17 |
| varrer | scan | traduz | poph-retention, poph-rag | 17 |
| vistas | views | traduz | poph-server, poph-store | 17 |
| apagada | deleted | traduz | poph-store, poph-server | 16 |
| apenas | only | traduz | poph-jev, poph-agent | 16 |
| aplicada | applied | traduz | poph-store, poph-agent | 16 |
| caminhos | paths | traduz | scripts, poph-fluxos | 16 |
| construir | build | traduz | poph-server, poph | 16 |
| dica | hint | traduz | console, poph-fluxos | 16 |
| dividir | split | traduz | poph-rag, scripts | 16 |
| duplicadas | duplicated | traduz | poph-server, poph-agent | 16 |
| esquecer | forget | traduz | poph-server, poph-agent | 16 |
| estatisticas | statistics | traduz | poph-agent, poph-server | 16 |
| finalizada | finalised | traduz | poph-store, poph-agent | 16 |
| finalizar | finalise | traduz | poph-agent, poph-core | 16 |
| gerado | generated | traduz | poph, poph-server | 16 |
| minutos | minutes | traduz | scripts, console | 16 |
| notificacao | notification | traduz | poph-fluxos, poph-server | 16 |
| passa | passes | traduz | poph-fluxos, poph-server | 16 |
| passagem | passage | traduz | poph-server, poph-store | 16 |
| permitido | allowed | traduz | poph-server, poph-fluxos | 16 |
| pessoa | person | traduz | poph-matrix, poph-server | 16 |
| recurso | resource | traduz | poph-server, poph-store | 16 |
| regenerada | regenerated | traduz | poph-agent | 16 |
| remetentes | senders | traduz | poph-server | 16 |
| rotulado | labelled | traduz | poph-jev, poph-rag | 16 |
| seguidas | consecutive | traduz | poph-server | 16 |
| sintaxe | syntax | traduz | poph-fluxos, poph | 16 |
| traco | trace | traduz | poph-server, poph | 16 |
| vencidas | expired | traduz | poph-server, poph-store | 16 |
| versoes | versions | traduz | poph-server, console | 16 |
| alterado | changed | traduz | poph-server, poph | 15 |
| alterados | changed | traduz | poph-rag, poph | 15 |
| cancelado | cancelled | traduz | console, poph-core | 15 |
| capturado | captured | traduz | poph-matrix, poph-server | 15 |
| capturar | capture | traduz | scripts, poph-server | 15 |
| destinos | targets | traduz | poph-fluxos, poph-rag | 15 |
| duplicados | duplicated | traduz | poph-agent, poph-plugin-eco | 15 |
| embutido | embedded | traduz | poph-rag, poph-evolution | 15 |
| encerradas | closed | traduz | poph-agent, poph-server | 15 |
| escalares | scalars | traduz | poph | 15 |
| espelhada | mirrored | traduz | poph-matrix, poph-server | 15 |
| esta | this | traduz | poph-rag, poph-server | 15 |
| estavel | stable | traduz | poph, poph-matrix | 15 |
| estimar | estimate | traduz | poph-agent | 15 |
| expirada | expired | traduz | poph-evolution, poph-server | 15 |
| expirar | expire | traduz | poph-matrix, poph-server | 15 |
| fechamento | closing | traduz | poph-store, poph-server | 15 |
| forcado | forced | traduz | poph-server, poph | 15 |
| guardar | keep | traduz | poph-evolution, poph-server | 15 |
| ilegivel | unreadable | traduz | poph-evolution, poph-matrix | 15 |
| inalterados | unchanged | traduz | poph-rag, poph | 15 |
| iniciado | started | traduz | poph-agent, poph-retention | 15 |
| leitores | readers | traduz | poph-store, poph-files | 15 |
| processado | processed | traduz | poph-agent, poph-store | 15 |
| publicado | published | traduz | poph-server, scripts | 15 |
| recarregando | reloading | traduz | console | 15 |
| redigidas | drafted | traduz | poph-server | 15 |
| registrada | recorded | traduz | poph-server, poph-agent | 15 |
| renomear | rename | traduz | poph-server, poph-files | 15 |
| resolucao | resolution | traduz | poph-fluxos, poph | 15 |
| revogado | revoked | traduz | poph, poph-server | 15 |
| tela | screen | traduz | console | 15 |
| temporario | temporary | traduz | poph-files, poph | 15 |
| topo | top | traduz | poph-fluxos | 15 |
| truncar | truncate | traduz | poph-jev, poph | 15 |
| vincular | link | traduz | poph-server, poph | 15 |
| agir | act | traduz | poph-server, poph-agent | 14 |
| alcancados | reached | traduz | poph-fluxos, console | 14 |
| apertado | tight | traduz | poph-server, poph-fluxos | 14 |
| aplica | applies | traduz | poph-jev, poph-store | 14 |
| assinados | signed | traduz | poph-server | 14 |
| colecao | collection | traduz | poph-server | 14 |
| convidar | invite | traduz | poph-matrix | 14 |
| deixar | leave | traduz | poph-agent, poph-core | 14 |
| descartada | discarded | traduz | poph-plugins, poph-evolution | 14 |
| desconectar | disconnect | traduz | poph-server, console | 14 |
| desconhecidas | unknown | traduz | poph-server | 14 |
| disposicao | layout | traduz | poph-server, console | 14 |
| esgotadas | exhausted | traduz | poph-server, poph-agent | 14 |
| idempotente | idempotent | traduz | poph-store, poph | 14 |
| minimo | minimum | traduz | poph-server, poph-fluxos | 14 |
| nas | in_the | traduz | poph-server, poph-agent | 14 |
| nenhuma | none | traduz | console, poph-fluxos | 14 |
| paginacao | pagination | traduz | poph-server | 14 |
| parado | stopped | traduz | poph-fluxos, poph | 14 |
| pontuacao | score | traduz | poph-rag | 14 |
| poupadas | saved | traduz | poph-fluxos, poph-server | 14 |
| preenchidos | filled | traduz | console, poph-server | 14 |
| processadas | processed | traduz | poph-server | 14 |
| rapidas | fast | traduz | poph-agent | 14 |
| recusados | refused | traduz | poph-agent, poph-server | 14 |
| reiniciar | restart | traduz | poph-plugins, poph-server | 14 |
| segmentos | segments | traduz | poph-server | 14 |
| substituir | replace | traduz | poph-store, poph-server | 14 |
| tudo | everything | traduz | poph-store, poph-server | 14 |
| vazamento | leak | traduz | poph-server | 14 |
| comentarios | comments | traduz | poph-fluxos, scripts | 13 |
| confirmando | confirming | traduz | console | 13 |
| instalado | installed | traduz | poph-rag, poph-server | 13 |
| invalidos | invalid | traduz | poph-plugins, poph-agent | 13 |
| visivel | visible | traduz | console, poph-agent | 13 |
| voltou | returned | traduz | poph-store, poph-fluxos | 13 |
| abas | tabs | traduz | console | 12 |
| degradada | degraded | traduz | console | 12 |
| executada | executed | traduz | poph-agent | 12 |
| idade | age | traduz | poph-retention, poph | 12 |
| pedacos | pieces | traduz | poph-rag, console | 12 |
| acumulado | accumulated | traduz | poph-matrix, poph-rag | 11 |
| alteracao | change | traduz | poph-store, poph-server | 11 |
| antigas | old | traduz | poph-matrix, poph-retention | 11 |
| apagado | deleted | traduz | poph, poph-agent | 11 |
| cancelar | cancel | traduz | poph-core, poph-agent | 11 |
| capacidades | capabilities | traduz | poph-jev, poph-agent | 11 |
| declara | declares | traduz | poph-server, poph | 11 |
| filtrar | filter | traduz | console, poph-store | 11 |
| metrica | metric | traduz | poph-server | 11 |
| mudancas | changes | traduz | poph-server, poph | 11 |
| seguro | safe | traduz | poph-matrix, poph-server | 11 |
| verificar | verify | traduz | poph, poph-server | 11 |
| concluida | finished | traduz | poph-fluxos, poph-server | 10 |
| concorrencia | concurrency | traduz | poph-matrix, poph-server | 10 |
| decididas | decided | traduz | poph-jev, console | 10 |
| digitado | typed | traduz | console, poph-agent | 10 |
| direto | direct | traduz | poph-server, poph | 10 |
| gerenciar | manage | traduz | console | 10 |
| pronto | ready | traduz | poph-plugins, poph-agent | 10 |
| remota | remote | traduz | console, poph-server | 10 |
| trabalho | work | traduz | poph-fluxos | 10 |
| alerta | alert | traduz | console | 9 |
| clicar | click | traduz | scripts | 9 |
| contato | contact | traduz | poph-store, poph-agent | 9 |
| corpos | bodies | traduz | poph-server | 9 |
| faixa | range | traduz | poph-store, poph | 9 |
| falharam | failed | traduz | poph-agent, poph-plugin-eco | 9 |
| metodos | methods | traduz | poph-server | 9 |
| negativo | negative | traduz | poph-fluxos, poph-retention | 9 |
| prontos | ready | traduz | poph-matrix | 9 |
| teclar | keypress | traduz | console | 9 |
| terminar | finish | traduz | poph-server, poph-core | 9 |
| trocaram | swapped | traduz | console, poph-server | 9 |
| adicionadas | added | traduz | console, poph-server | 8 |
| alternar | toggle | traduz | console | 8 |
| capturas | captures | traduz | poph-evolution | 8 |
| entrou | entered | traduz | poph-matrix, poph-store | 8 |
| este | this | traduz | poph-agent, console | 8 |
| ocupada | busy | traduz | poph-agent, scripts | 8 |
| painel | panel | traduz | console, poph-agent | 8 |
| prontas | ready | traduz | poph-server, poph-store | 8 |
| removidas | removed | traduz | console, poph-server | 8 |
| roteamento | routing | traduz | console | 8 |
| seletor | selector | traduz | console | 8 |
| subida | rise | traduz | poph-server, poph-rag | 8 |
| aninhado | nested | traduz | poph-jev, poph-server | 7 |
| antigos | old | traduz | poph-matrix, poph-agent | 7 |
| atalho | shortcut | traduz | console | 7 |
| estrutura | structure | traduz | console | 7 |
| ida | trip | traduz | poph-fluxos, poph-store | 7 |
| nesta | in_this | traduz | console, poph-store | 7 |
| referencias | references | traduz | poph-store, poph | 7 |
| sair | exit | traduz | poph-rag, poph-agent | 7 |
| sensivel | sensitive | traduz | poph, poph-agent | 7 |
| abandonada | abandoned | traduz | poph-fluxos, poph-store | 6 |
| acessar | access | traduz | console | 6 |
| editando | editing | traduz | console | 6 |
| formulario | form | traduz | console | 6 |
| frente | front | traduz | poph-store, poph-rag | 6 |
| largura | width | traduz | scripts, poph-fluxos | 6 |
| problema | problem | traduz | console, raiz | 6 |
| propriedades | properties | traduz | poph-agent, poph-server | 6 |
| retirar | withdraw | traduz | poph | 6 |
| ouvintes | listeners | traduz | scripts | 5 |
| pessoas | people | traduz | poph-matrix | 5 |
| principio | principle | traduz | poph-agent, console | 5 |
| digitando | typing | traduz | poph-evolution | 4 |
| editar | edit | traduz | console, poph-agent | 4 |
| foco | focus | traduz | console | 4 |
| ponto | point | traduz | poph-agent, poph-server | 4 |
| compacta | compact | traduz | console | 3 |
| conclusao | completion | traduz | poph-server, poph-agent | 3 |
| explicacao | explanation | traduz | console, poph-agent | 3 |
| habilitado | enabled | traduz | poph-agent, poph | 3 |
| recuperados | recovered | traduz | poph-agent | 3 |
| vinculos | links | traduz | poph-server | 3 |
| atalhos | shortcuts | traduz | console | 2 |
| casca | shell | traduz | console | 2 |
| combina | combines | traduz | poph-matrix | 2 |
| consertar | repair | traduz | poph-server | 2 |
| copiar | copy | traduz | scripts | 2 |
| equipe | team | traduz | poph-fluxos, poph-server | 2 |
| exportacao | export | traduz | poph-server | 2 |
| inspetor | inspector | traduz | console | 2 |
| porque | because | traduz | poph-evolution, poph-store | 2 |
| selecionar | select | traduz | console | 2 |
| atualizacao | update | traduz | poph-store | 1 |
| correcao | fix | traduz | poph-matrix | 1 |
| desabilitado | disabled | traduz | poph-agent | 1 |
| expor | expose | traduz | poph-server | 1 |
| iniciando | starting | traduz | poph-evolution | 1 |
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
| ficha | — | congelado | coluna ficha_id; comando_cli poph ficha; comando_cli poph ficha listar; e mais 11 | - |
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
