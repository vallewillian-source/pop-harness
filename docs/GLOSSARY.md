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
| texto | text | traduz | poph-agent, poph | 2910 |
| cenario | scenario | traduz | poph-server, poph | 2399 |
| estado | state | traduz | poph-server, poph-fluxos | 2015 |
| agente | agent | traduz | poph-server, poph-agent | 1987 |
| conversa | conversation | traduz | poph-agent, poph-server | 1744 |
| erro | error | traduz | poph, poph-server | 1679 |
| nome | name | traduz | poph-fluxos, console | 1548 |
| fluxo | flow | traduz | poph-agent, poph-server | 1246 |
| resultado | result | traduz | poph-store, poph-fluxos | 1149 |
| de | of | traduz | poph-server, poph | 1074 |
| saida | output | traduz | poph, poph-server | 1052 |
| pasta | folder | traduz | poph, poph-server | 1006 |
| resposta | response | traduz | poph-server, poph | 928 |
| corpo | body | traduz | poph-server, poph | 901 |
| chave | key | traduz | poph-server, poph-fluxos | 860 |
| campo | field | traduz | poph-fluxos, console | 840 |
| tipo | kind | traduz | poph-server, poph-store | 826 |
| mensagem | message | traduz | poph-agent, poph-server | 786 |
| com | with | traduz | poph-server, poph-fluxos | 780 |
| evento | event | traduz | console, poph-server | 747 |
| turno | turn | traduz | poph-agent, poph-server | 731 |
| acao | action | traduz | poph-fluxos, poph | 728 |
| servidor | server | traduz | console, poph-server | 701 |
| etapa | step | traduz | poph-fluxos, poph-server | 700 |
| registro | record | traduz | poph-agent, poph-server | 683 |
| valor | value | traduz | poph-server, poph | 679 |
| dados | data | traduz | poph-server, poph-fluxos | 663 |
| motivo | reason | traduz | poph-server, poph-agent | 652 |
| arquivo | file | traduz | poph, poph-server | 597 |
| caminho | path | traduz | poph-server, poph | 596 |
| novo | new | traduz | poph-server, poph | 565 |
| criar | create | traduz | poph-server, poph | 560 |
| da | of | traduz | poph-server, poph-fluxos | 544 |
| falha | failure | traduz | poph-server, poph | 532 |
| pedir | ask | traduz | poph-server, poph | 527 |
| ficha | case | traduz | poph-server, poph-agent | 489 |
| gatilho | trigger | traduz | poph-agent, poph-fluxos | 464 |
| metricas | metrics | traduz | poph-server, poph-fluxos | 455 |
| sem | without | traduz | poph-server, poph | 448 |
| decisao | decision | traduz | poph-fluxos, poph-jev | 445 |
| fonte | source | traduz | poph-fluxos, poph-server | 435 |
| destino | target | traduz | poph-server, poph | 432 |
| em | in | traduz | poph-server, poph-fluxos | 431 |
| departamento | department | traduz | poph, poph-server | 412 |
| ambiente | environment | traduz | poph, poph-server | 409 |
| fim | end | traduz | poph-fluxos, poph-agent | 398 |
| versao | version | traduz | poph-server, console | 398 |
| canal | channel | traduz | poph-server, poph-store | 397 |
| para | for | traduz | poph-fluxos, poph-server | 375 |
| integracao | integration | traduz | poph-server, console | 370 |
| cliente | client | traduz | poph-server, poph | 358 |
| provedor | provider | traduz | poph-agent, poph-server | 330 |
| relatorio | report | traduz | poph-server, poph | 327 |
| codigo | code | traduz | poph, poph-server | 324 |
| nao | not | traduz | poph-server, poph-fluxos | 324 |
| instancia | instance | traduz | poph-server, poph-evolution | 322 |
| situacao | situation | traduz | console, poph-retention | 316 |
| alvo | target | traduz | poph-server, poph | 315 |
| desligar | disable | traduz | poph-server, poph | 311 |
| itens | items | traduz | poph-server, console | 311 |
| falso | false | traduz | poph-server, poph | 308 |
| linha | line | traduz | poph-server, poph | 307 |
| caso | case | traduz | poph-server, poph-fluxos | 301 |
| fluxos | flows | traduz | poph-server, poph-agent | 300 |
| lista | list | traduz | poph-fluxos, poph | 294 |
| atual | current | traduz | poph-server, poph-fluxos | 291 |
| modelo | model | traduz | poph-agent, poph-server | 282 |
| sala | room | traduz | poph-server, poph | 277 |
| porta | port | traduz | poph-server, poph | 268 |
| campos | fields | traduz | poph-fluxos, console | 267 |
| configuracao | configuration | traduz | poph, console | 267 |
| grupo | group | traduz | poph, poph-server | 263 |
| whatsapp | whatsapp | traduz | console, poph-server | 258 |
| ator | actor | traduz | poph-store, poph-server | 250 |
| limites | limits | traduz | poph-fluxos, poph-agent | 250 |
| numero | number | traduz | console, poph-store | 247 |
| aviso | notice | traduz | poph-server, poph | 228 |
| metodo | method | traduz | poph-server, poph | 221 |
| rotulo | label | traduz | poph-server, console | 217 |
| rota | route | traduz | poph-server, poph-fluxos | 214 |
| segredo | secret | traduz | poph-server, poph | 213 |
| execucao | execution | traduz | poph-server, poph-store | 207 |
| validar | validate | traduz | poph-types, poph-server | 205 |
| relato | report | traduz | poph-server | 197 |
| pedido | request | traduz | poph-server, poph | 195 |
| inicio | beginning | traduz | poph-server, poph-fluxos | 194 |
| vazio | empty | traduz | poph-server, poph-fluxos | 189 |
| descricao | description | traduz | console, poph-agent | 185 |
| externo | external | traduz | poph-server, poph-agent | 183 |
| revisao | revision | traduz | poph-server, poph | 183 |
| cru | raw | traduz | poph-server | 180 |
| modo | mode | traduz | poph-server, console | 180 |
| ciclo | cycle | traduz | poph-server, poph-fluxos | 172 |
| falhas | failures | traduz | poph-server, poph-fluxos | 171 |
| categoria | category | traduz | poph, poph-server | 166 |
| identidade | identity | traduz | poph-server, poph | 161 |
| contexto | context | traduz | poph-fluxos, poph | 160 |
| depois | after | traduz | poph-server, console | 159 |
| subir | raise | traduz | poph-server, poph | 159 |
| origem | origin | traduz | poph, poph-server | 158 |
| efeitos | effects | traduz | poph-server, poph-fluxos | 155 |
| por | by | traduz | poph, poph-server | 154 |
| ao | to | traduz | poph-server, poph-fluxos | 153 |
| primeira | first | traduz | poph-server, poph-fluxos | 151 |
| produto | product | traduz | poph, poph-server | 150 |
| responder | reply | traduz | poph-server, poph | 149 |
| na | towards | traduz | poph-server, poph-fluxos | 148 |
| antes | before | traduz | poph-server, console | 147 |
| motor | engine | traduz | poph-server, poph | 143 |
| agentes | agents | traduz | poph, poph-server | 135 |
| esperar | wait | traduz | poph-server, poph | 134 |
| mapa | map | traduz | poph-server, poph | 134 |
| segredos | secrets | traduz | poph, console | 133 |
| espelho | mirror | traduz | poph-server | 132 |
| conteudo | content | traduz | poph, poph-server | 131 |
| conversas | conversations | traduz | poph-server, poph-store | 131 |
| invalido | invalid | traduz | poph-server, poph | 130 |
| ordem | order | traduz | poph-server, console | 129 |
| midia | media | traduz | poph-server, poph-store | 126 |
| outro | other | traduz | poph-server, poph | 124 |
| pagina | page | traduz | poph-server, poph | 124 |
| manifesto | manifest | traduz | poph-server, console | 122 |
| indice | index | traduz | poph-server, poph | 121 |
| abrir | open | traduz | poph, poph-server | 119 |
| sinal | signal | traduz | poph-server, poph | 119 |
| avisos | notices | traduz | poph-server, poph | 117 |
| entrada | entry | traduz | poph-server, poph | 116 |
| erros | errors | traduz | poph-server, poph-fluxos | 116 |
| detalhe | detail | traduz | poph, poph-server | 114 |
| endereco | address | traduz | poph-server, poph | 114 |
| falsa | false | traduz | poph-server, poph | 113 |
| modulo | module | traduz | poph-server, poph | 112 |
| primeiro | first | traduz | poph-server, poph-fluxos | 111 |
| mensagens | messages | traduz | poph-server, scripts | 107 |
| exigir | require | traduz | poph-server, poph | 106 |
| avaliador | evaluator | traduz | poph-fluxos, poph-server | 105 |
| tentativas | attempts | traduz | poph-server, poph-agent | 105 |
| transacao | transaction | traduz | poph-server, scripts | 105 |
| teto | cap | traduz | poph-server, poph-fluxos | 103 |
| vai | goes | traduz | poph-fluxos, raiz | 103 |
| que | that | traduz | poph-fluxos, poph-server | 102 |
| compilado | compiled | traduz | poph-fluxos, poph-server | 101 |
| chamada | call | traduz | poph-server, poph-fluxos | 100 |
| imprimir | print | traduz | poph, poph-fluxos | 100 |
| espaco | space | traduz | console, poph-server | 99 |
| nivel | level | traduz | poph, console | 99 |
| cabecalhos | headers | traduz | poph-server, poph | 98 |
| desfecho | outcome | traduz | poph-server, poph-agent | 98 |
| envios | sends | traduz | poph-server | 98 |
| guarda | guard | traduz | poph-server, poph | 96 |
| segunda | second | traduz | poph-server, poph-fluxos | 95 |
| criado | created | traduz | poph-server, poph | 94 |
| escritas | writings | traduz | poph-server | 94 |
| carregar | load | traduz | poph, poph-fluxos | 93 |
| descrever | describe | traduz | poph | 92 |
| opcoes | options | traduz | poph-server, poph | 92 |
| regra | rule | traduz | poph-fluxos, poph-server | 92 |
| agora | now | traduz | poph-server, poph | 91 |
| recusa | refusal | traduz | poph-server, poph | 91 |
| registros | records | traduz | poph-server | 91 |
| outra | other | traduz | poph-server, poph | 90 |
| lido | read | traduz | poph, poph-server | 89 |
| entrega | delivery | traduz | poph-server | 88 |
| resolve | resolves | traduz | scripts, poph-fluxos | 88 |
| gravado | stored | traduz | poph-server, poph | 87 |
| captura | capture | traduz | poph-server, poph | 86 |
| chamadas | calls | traduz | poph-server, poph-fluxos | 86 |
| ultimo | last | traduz | poph-server, poph-store | 86 |
| efeito | effect | traduz | poph-server, poph-fluxos | 85 |
| uma | a | traduz | poph-server, poph-fluxos | 85 |
| mundo | world | traduz | poph-server | 84 |
| teste | test | traduz | poph, poph-server | 84 |
| conexao | connection | traduz | poph-server, poph | 83 |
| listar | list | traduz | poph, poph-server | 83 |
| resumo | summary | traduz | poph, poph-server | 83 |
| instrucoes | instructions | traduz | poph, poph-server | 82 |
| vivo | live | traduz | poph, poph-server | 82 |
| fechada | closed | traduz | poph-store, poph-server | 81 |
| executar | run | traduz | poph, poph-server | 80 |
| validacao | validation | traduz | poph-server, poph | 80 |
| esperado | expected | traduz | poph, poph-server | 79 |
| espacos | spaces | traduz | poph-server, poph | 78 |
| limite | limit | traduz | poph, poph-server | 78 |
| par | pair | traduz | poph-server, poph | 78 |
| sessao | session | traduz | poph, console | 78 |
| servir | serve | traduz | poph-server, poph | 77 |
| atendente | attendant | traduz | poph-server, poph-fluxos | 76 |
| canais | channels | traduz | poph-server, poph | 76 |
| midias | media | traduz | poph-server | 76 |
| aberta | open | traduz | poph-server, poph-fluxos | 74 |
| padrao | default | traduz | poph-server, poph | 74 |
| relogio | clock | traduz | poph-server | 74 |
| anexos | attachments | traduz | poph-server, poph-agent | 73 |
| comando | command | traduz | poph, poph-server | 73 |
| textos | texts | traduz | poph-server, poph-fluxos | 73 |
| arquivos | files | traduz | poph-server, poph | 71 |
| cifra | cipher | traduz | poph-server, poph | 70 |
| definir | define | traduz | poph-server, poph | 69 |
| eventos | events | traduz | poph-server, poph-store | 69 |
| respostas | responses | traduz | poph-server | 69 |
| fila | queue | traduz | poph-server, poph | 68 |
| igual | equal | traduz | poph-fluxos, poph-server | 68 |
| iniciar | start | traduz | poph-server, poph | 68 |
| instancias | instances | traduz | poph-server, console | 68 |
| ler | read | traduz | poph, poph-server | 68 |
| pendentes | pending | traduz | poph-server, poph-store | 68 |
| enviar | send | traduz | poph-server, poph | 67 |
| gravada | stored | traduz | poph-server, poph | 66 |
| idempotencia | idempotency | traduz | poph-store, poph-server | 65 |
| parar | stop | traduz | poph-server, scripts | 65 |
| ligar | link | traduz | poph-server, poph | 64 |
| catalogo | catalog | traduz | poph, poph-server | 63 |
| fora | outside | traduz | poph-server, poph | 63 |
| nomes | names | traduz | poph-server, poph | 63 |
| registrar | register | traduz | poph-server | 63 |
| dias | days | traduz | console, poph-retention | 62 |
| linhas | lines | traduz | poph, poph-server | 62 |
| maior | bigger | traduz | poph-fluxos, poph-server | 62 |
| aparelho | device | traduz | poph-server | 61 |
| documento | document | traduz | poph, poph-server | 60 |
| evolucao | evolution | traduz | poph-server | 60 |
| nova | new | traduz | poph-fluxos, poph-server | 60 |
| banco | database | traduz | poph, poph-server | 59 |
| ativo | active | traduz | poph-agent, console | 58 |
| ligado | linked | traduz | poph-server, poph | 57 |
| partes | parts | traduz | poph-server, poph-fluxos | 57 |
| diagnostico | diagnostic | traduz | poph-fluxos, poph | 56 |
| envio | send | traduz | poph-server, poph-fluxos | 56 |
| funcao | function | traduz | poph-fluxos, raiz | 56 |
| mudou | changed | traduz | poph-fluxos, poph-server | 56 |
| recusado | refused | traduz | poph-server, poph | 56 |
| resolucoes | resolutions | traduz | poph-fluxos, poph-server | 56 |
| variavel | variable | traduz | poph, poph-server | 56 |
| espera | wait | traduz | poph-server, poph-fluxos | 55 |
| falhar | fail | traduz | poph-server | 55 |
| formato | format | traduz | poph-store, poph | 55 |
| tem | has | traduz | poph, poph-server | 54 |
| carimbo | stamp | traduz | poph-server, poph | 53 |
| desligado | disabled | traduz | poph, poph-server | 53 |
| execucoes | executions | traduz | poph-server, poph-store | 53 |
| instrucao | instruction | traduz | poph-fluxos, raiz | 53 |
| unica | single | traduz | poph-fluxos, poph-server | 53 |
| enviado | sent | traduz | poph-server, poph-store | 52 |
| recebido | received | traduz | poph-server, poph | 52 |
| segundo | second | traduz | poph-server, poph-fluxos | 52 |
| assinatura | signature | traduz | poph-server | 51 |
| duracao | duration | traduz | poph-server, console | 51 |
| atualizar | update | traduz | poph, console | 50 |
| inexistente | missing | traduz | poph, poph-server | 50 |
| interno | internal | traduz | poph-server, poph-fluxos | 50 |
| publicar | publish | traduz | poph-server, poph | 50 |
| contadores | counters | traduz | poph-server | 49 |
| diretorio | directory | traduz | poph-server, poph-rag | 49 |
| enviados | sent | traduz | poph-server | 49 |
| atores | actors | traduz | poph-fluxos, poph-server | 48 |
| cena | scene | traduz | poph-server | 48 |
| contem | contains | traduz | poph-fluxos, poph-files | 48 |
| escrever | write | traduz | poph, poph-server | 48 |
| fichas | cases | traduz | poph-store, poph-server | 48 |
| formatar | format | traduz | poph-fluxos, poph | 48 |
| turnos | turns | traduz | poph-server, raiz | 48 |
| apos | after | traduz | poph-server, poph | 47 |
| casos | cases | traduz | poph-fluxos, poph-server | 47 |
| consulta | query | traduz | poph-server, poph | 47 |
| cursores | cursors | traduz | poph-server, poph | 47 |
| escrita | writing | traduz | poph-server, poph | 47 |
| pedidos | requests | traduz | poph-server, poph | 47 |
| pendente | pending | traduz | poph-server, poph | 47 |
| quando | when | traduz | poph-server, poph-fluxos | 47 |
| fantasma | ghost | traduz | poph-server | 46 |
| remoto | remote | traduz | poph, poph-server | 46 |
| trecho | passage | traduz | poph-fluxos, poph | 46 |
| anexo | attachment | traduz | poph-server, poph | 45 |
| atraso | delay | traduz | poph-server, poph | 45 |
| etapas | steps | traduz | poph-fluxos, poph-server | 45 |
| metadados | metadata | traduz | poph-files, poph-server | 45 |
| mostrar | show | traduz | poph | 45 |
| retrato | snapshot | traduz | poph | 45 |
| globais | globals | traduz | poph-server, poph-types | 44 |
| grande | large | traduz | poph-server, poph-fluxos | 44 |
| veneno | poison | traduz | poph-server | 44 |
| atendimento | service | traduz | poph-fluxos, poph-server | 43 |
| auditoria | audit | traduz | poph-server | 43 |
| como | as | traduz | poph, poph-server | 43 |
| publicada | published | traduz | poph-server, poph-store | 43 |
| das | of | traduz | poph-server, poph | 42 |
| detalhes | details | traduz | poph-server | 42 |
| entregar | deliver | traduz | poph-server | 42 |
| produtos | products | traduz | poph, console | 42 |
| resto | rest | traduz | poph-server, poph | 42 |
| contar | count | traduz | poph-server, poph | 41 |
| rascunho | draft | traduz | poph-server, poph-store | 41 |
| rodando | running | traduz | poph-server | 41 |
| salas | rooms | traduz | poph-server, scripts | 41 |
| tipos | kinds | traduz | poph-fluxos, poph-server | 41 |
| titulo | title | traduz | console, poph | 41 |
| ate | until | traduz | poph-server, scripts | 40 |
| atualizado | updated | traduz | poph-server | 40 |
| comparar | compare | traduz | poph-server, poph | 40 |
| exige | requires | traduz | poph-fluxos, raiz | 40 |
| laco | loop | traduz | poph-server | 40 |
| lida | read | traduz | poph-server, poph-fluxos | 40 |
| passo | step | traduz | poph-server | 40 |
| referencia | reference | traduz | poph, poph-server | 40 |
| anterior | previous | traduz | poph-server, poph-fluxos | 39 |
| criados | created | traduz | poph-server, poph-fluxos | 39 |
| definicao | definition | traduz | poph-store, poph-server | 39 |
| diagnosticos | diagnostics | traduz | poph, poph-server | 39 |
| ja | already | traduz | poph-server, poph | 39 |
| mostra | shows | traduz | poph, poph-server | 39 |
| obter | get | traduz | poph-server, poph | 39 |
| pela | by | traduz | poph-server, poph | 39 |
| processo | process | traduz | poph-server, poph | 39 |
| proxima | next | traduz | poph-server, poph-store | 39 |
| roteiro | script | traduz | poph-server | 39 |
| ajustes | adjustments | traduz | poph-server, poph | 38 |
| categorias | categories | traduz | poph, poph-server | 38 |
| posicao | position | traduz | poph-server, poph-fluxos | 38 |
| prefixo | prefix | traduz | poph-server, poph | 38 |
| problemas | problems | traduz | poph, poph-server | 38 |
| retencao | retention | traduz | console, poph | 38 |
| tabelas | tables | traduz | console, poph | 38 |
| temperatura | temperature | traduz | poph-core, poph-store | 38 |
| acoes | actions | traduz | poph-fluxos, raiz | 37 |
| cabecalho | header | traduz | poph-server | 37 |
| marcador | marker | traduz | poph-server | 37 |
| raiz | root | traduz | poph-server, poph | 37 |
| repetida | repeated | traduz | poph-server, poph-fluxos | 37 |
| servico | service | traduz | poph-server, poph | 37 |
| baixar | download | traduz | poph-server, poph | 36 |
| compila | compiles | traduz | poph-fluxos, poph | 36 |
| dois | two | traduz | poph-fluxos, poph-server | 36 |
| impresso | printed | traduz | poph-fluxos, poph | 36 |
| intervalo | interval | traduz | poph-server, poph | 36 |
| ou | or | traduz | poph-fluxos, poph | 36 |
| parada | stop | traduz | poph-server, poph-fluxos | 36 |
| preparo | preparation | traduz | poph, poph-server | 36 |
| tabela | table | traduz | poph, poph-server | 36 |
| excluir | delete | traduz | poph, poph-server | 35 |
| expiracao | expiry | traduz | poph-server | 35 |
| passar | pass | traduz | poph-server | 35 |
| existe | exists | traduz | poph-server, poph | 34 |
| prazo | deadline | traduz | poph-server, poph-fluxos | 34 |
| vez | time | traduz | poph-server, poph-fluxos | 34 |
| avisar | notify | traduz | poph-server, console | 33 |
| encaminhar | forward | traduz | poph-fluxos, poph-server | 33 |
| gerar | generate | traduz | poph | 33 |
| liberar | release | traduz | poph, console | 33 |
| lote | batch | traduz | poph-server, poph | 33 |
| mudanca | change | traduz | poph, poph-fluxos | 33 |
| verificacao | verification | traduz | poph | 33 |
| muda | changes | traduz | poph-fluxos, poph | 32 |
| nomeado | named | traduz | poph-fluxos | 32 |
| proibido | forbidden | traduz | poph-server, poph | 32 |
| argumento | argument | traduz | poph-fluxos | 31 |
| esperada | expected | traduz | poph-server, poph | 31 |
| forcar | force | traduz | console, poph | 31 |
| historico | history | traduz | poph-server, poph | 31 |
| invalida | invalid | traduz | poph-server, poph-fluxos | 31 |
| modelos | models | traduz | poph, poph-server | 31 |
| vira | becomes | traduz | poph-server, poph | 31 |
| apoio | support | traduz | poph | 30 |
| canonico | canonical | traduz | poph-fluxos, poph-server | 30 |
| capturados | captured | traduz | poph-server | 30 |
| conferir | check | traduz | poph-server, poph | 30 |
| conflito | conflict | traduz | poph-server, poph | 30 |
| divergencias | divergences | traduz | poph, poph-server | 30 |
| mutacao | mutation | traduz | poph-fluxos | 30 |
| obrigatorio | required | traduz | poph-fluxos, raiz | 30 |
| quem | who | traduz | poph-server, poph | 30 |
| verificacoes | verifications | traduz | poph | 30 |
| conectado | connected | traduz | poph-server | 29 |
| espelhado | mirrored | traduz | poph-server, poph-store | 29 |
| esquema | schema | traduz | poph-server, poph | 29 |
| gravar | write | traduz | poph-server, poph | 29 |
| ouvinte | listener | traduz | poph-server, poph | 29 |
| remetente | sender | traduz | console, poph-server | 29 |
| rodada | round | traduz | poph-server, scripts | 29 |
| corte | cutoff | traduz | poph-server, poph | 28 |
| enviadas | sent | traduz | poph-server | 28 |
| importar | import | traduz | poph-server, poph | 28 |
| montar | build | traduz | poph-server, poph | 28 |
| nada | nothing | traduz | poph-server, poph | 28 |
| pelo | by | traduz | poph-server, poph-fluxos | 28 |
| segundos | seconds | traduz | poph-server, poph | 28 |
| segurar | hold | traduz | poph-server | 28 |
| simples | simple | traduz | poph-fluxos, poph-server | 28 |
| testes | tests | traduz | poph-server, poph | 28 |
| validade | validity | traduz | poph-server | 28 |
| vizinha | neighbouring | traduz | poph-server, poph | 28 |
| ausente | missing | traduz | poph-server, poph | 27 |
| criacao | creation | traduz | poph-server, poph | 27 |
| criada | created | traduz | poph-server, poph | 27 |
| escolha | choice | traduz | poph, console | 27 |
| loja | store | traduz | poph-server | 27 |
| mesma | same | traduz | poph-fluxos, poph-server | 27 |
| tarefa | task | traduz | poph-server | 27 |
| tentativa | attempt | traduz | poph-server, poph-store | 27 |
| dado | given | traduz | poph-server, poph-fluxos | 26 |
| definitivos | definitive | traduz | poph-server | 26 |
| fontes | sources | traduz | poph, console | 26 |
| montagem | assembly | traduz | poph, console | 26 |
| plano | plan | traduz | poph-server | 26 |
| publicadas | published | traduz | poph-server | 26 |
| tamanho | size | traduz | poph-server, poph | 26 |
| todos | all | traduz | poph, poph-server | 26 |
| adiadas | deferred | traduz | poph-server | 25 |
| bloco | block | traduz | poph-server, poph-fluxos | 25 |
| desligada | disabled | traduz | poph-server | 25 |
| entre | between | traduz | poph-fluxos, poph-server | 25 |
| errado | wrong | traduz | poph-server, poph | 25 |
| fica | stays | traduz | poph-server, poph-fluxos | 25 |
| transicao | transition | traduz | poph-server | 25 |
| avancar | advance | traduz | poph-server, poph-fluxos | 24 |
| avulsa | one_off | traduz | poph-server | 24 |
| bruta | raw | traduz | poph-server, poph | 24 |
| carrinho | cart | traduz | poph-server, console | 24 |
| chamar | call | traduz | poph-server | 24 |
| departamentos | departments | traduz | poph, poph-fluxos | 24 |
| esperados | expected | traduz | poph-server | 24 |
| exemplo | example | traduz | poph-fluxos, poph-server | 24 |
| relatorios | reports | traduz | poph | 24 |
| retentativas | retries | traduz | poph-jev, poph-core | 24 |
| roteador | router | traduz | poph-server | 24 |
| saidas | outputs | traduz | poph-server, poph | 24 |
| sonda | probe | traduz | poph-server | 24 |
| apagadas | deleted | traduz | poph-server | 23 |
| autorizado | authorized | traduz | poph-server, console | 23 |
| diferente | different | traduz | poph-fluxos, poph | 23 |
| dourada | golden | traduz | poph-fluxos | 23 |
| duas | two | traduz | poph-server, poph-fluxos | 23 |
| escopo | scope | traduz | console, poph-fluxos | 23 |
| gravadas | stored | traduz | poph-server | 23 |
| mesmo | same | traduz | poph-fluxos, poph-server | 23 |
| nos | us | traduz | poph-fluxos, poph-server | 23 |
| parcial | partial | traduz | poph-fluxos, poph-server | 23 |
| pedaco | piece | traduz | poph-server, poph | 23 |
| recuo | backoff | traduz | poph-fluxos, poph | 23 |
| recusada | refused | traduz | poph, poph-server | 23 |
| vistos | seen | traduz | poph-server, poph-fluxos | 23 |
| volta | return | traduz | poph-server, poph-fluxos | 23 |
| decodificar | decode | traduz | poph-server, poph | 22 |
| dona | owner | traduz | poph-server | 22 |
| exemplos | examples | traduz | raiz, poph-fluxos | 22 |
| fechar | close | traduz | poph-server, poph | 22 |
| filtro | filter | traduz | poph, poph-server | 22 |
| integracoes | integrations | traduz | poph-server, poph | 22 |
| ligada | linked | traduz | poph-server | 22 |
| nem | nor | traduz | poph-server, poph | 22 |
| pronta | ready | traduz | poph-server | 22 |
| roteiros | scripts | traduz | poph-server | 22 |
| trechos | passages | traduz | console, poph | 22 |
| uso | use | traduz | poph, poph-fluxos | 22 |
| autorizacao | authorization | traduz | poph-server, poph | 21 |
| codigos | codes | traduz | poph-fluxos, poph-server | 21 |
| disfarce | disguise | traduz | poph-server | 21 |
| encerrada | closed | traduz | poph-server, poph-fluxos | 21 |
| grupos | groups | traduz | poph, poph-server | 21 |
| nenhum | none | traduz | poph-server, poph-fluxos | 21 |
| processador | processor | traduz | poph-server, poph | 21 |
| proprio | own | traduz | poph-server, poph | 21 |
| revisoes | revisions | traduz | poph-server, poph-store | 21 |
| semear | seed | traduz | poph-server, poph | 21 |
| sim | yes | traduz | poph-fluxos, poph | 21 |
| ultima | last | traduz | poph-server, poph | 21 |
| adiado | deferred | traduz | poph-server | 20 |
| comparacao | comparison | traduz | poph, poph-server | 20 |
| falhou | failed | traduz | poph-server, poph | 20 |
| ferramentas | tools | traduz | console, poph | 20 |
| menor | smaller | traduz | poph-fluxos, poph-server | 20 |
| mostrado | shown | traduz | poph, poph-server | 20 |
| objeto | object | traduz | poph-server, poph | 20 |
| operacoes | operations | traduz | poph-server, poph | 20 |
| portugues | portuguese | traduz | poph, poph-server | 20 |
| responde | answers | traduz | poph-server, poph-fluxos | 20 |
| transitoria | transient | traduz | poph-server | 20 |
| valido | valid | traduz | poph-server, poph | 20 |
| cada | each | traduz | poph-server, poph-fluxos | 19 |
| caractere | character | traduz | poph-server, poph | 19 |
| declarada | declared | traduz | poph-server | 19 |
| declarado | declared | traduz | poph-server, poph-fluxos | 19 |
| derivar | derive | traduz | poph-server, poph | 19 |
| devolve | returns | traduz | poph-server, poph-fluxos | 19 |
| entidade | entity | traduz | poph-server, poph | 19 |
| feito | done | traduz | poph-server | 19 |
| final | final | traduz | poph-matrix, console | 19 |
| livre | free | traduz | poph-server, poph | 19 |
| manifestos | manifests | traduz | poph-server | 19 |
| marca | mark | traduz | poph-server, poph | 19 |
| velha | old | traduz | poph-server, poph | 19 |
| acima | above | traduz | poph-server, poph-fluxos | 18 |
| alterada | changed | traduz | poph-server | 18 |
| argumentos | arguments | traduz | console, poph-server | 18 |
| aspas | quotes | traduz | poph-fluxos, poph | 18 |
| avisadas | notified | traduz | poph-server | 18 |
| coluna | column | traduz | poph-fluxos, poph | 18 |
| configurar | configure | traduz | poph-server, poph | 18 |
| convite | invitation | traduz | poph-server, poph | 18 |
| cortado | cut | traduz | poph-fluxos, poph-server | 18 |
| entregue | delivered | traduz | poph-server | 18 |
| feita | done | traduz | poph, poph-server | 18 |
| liberado | released | traduz | poph-server, poph-store | 18 |
| periodo | period | traduz | poph-server | 18 |
| redacoes | writings | traduz | poph-server | 18 |
| relativo | relative | traduz | poph-server, poph | 18 |
| removida | removed | traduz | poph-fluxos | 18 |
| rotas | routes | traduz | poph-server | 18 |
| ruim | poor | traduz | poph, poph-server | 18 |
| temporaria | temporary | traduz | poph | 18 |
| transitorios | transient | traduz | poph-server | 18 |
| valores | values | traduz | poph-server, poph-fluxos | 18 |
| achados | findings | traduz | poph-server, poph | 17 |
| conta | counts | traduz | poph-server, poph-fluxos | 17 |
| decisoes | decisions | traduz | poph-fluxos, console | 17 |
| dentro | inside | traduz | poph-fluxos, poph | 17 |
| derrubar | drop | traduz | poph-server, poph-fluxos | 17 |
| despedida | farewell | traduz | console | 17 |
| diz | says | traduz | poph, poph-fluxos | 17 |
| dos | of | traduz | poph-fluxos, poph-server | 17 |
| encerra | closes | traduz | poph-fluxos, raiz | 17 |
| entradas | entries | traduz | poph-server | 17 |
| fantasmas | ghosts | traduz | poph-server | 17 |
| humana | human | traduz | poph-server | 17 |
| limpo | clean | traduz | poph, poph-server | 17 |
| nomeavel | nameable | traduz | poph-server | 17 |
| paginas | pages | traduz | poph-server, poph-fluxos | 17 |
| perfis | profiles | traduz | poph-server | 17 |
| quantos | how_many | traduz | poph-server, poph-fluxos | 17 |
| receptor | receiver | traduz | poph-server | 17 |
| renomeacoes | renames | traduz | poph-server | 17 |
| visita | visit | traduz | poph-fluxos, poph-server | 17 |
| aparece | appears | traduz | poph-server, poph | 16 |
| atender | attend | traduz | poph-server, poph | 16 |
| atualizadas | updated | traduz | console, poph-server | 16 |
| conferido | checked | traduz | poph, poph-server | 16 |
| convidado | guest | traduz | poph-server, poph | 16 |
| encerrar | close | traduz | poph-server, poph-fluxos | 16 |
| espelhadas | mirrored | traduz | poph-server | 16 |
| guardadas | guarded | traduz | poph-server, poph-fluxos | 16 |
| indisponivel | unavailable | traduz | poph-server, console | 16 |
| posicoes | positions | traduz | poph-server, poph-fluxos | 16 |
| quantas | how_many | traduz | poph-server, poph-fluxos | 16 |
| reindexar | reindex | traduz | poph, console | 16 |
| remetentes | senders | traduz | poph-server | 16 |
| seguidas | consecutive | traduz | poph-server | 16 |
| todas | all | traduz | poph-server, console | 16 |
| ver | see | traduz | poph-server | 16 |
| vitima | victim | traduz | poph-server | 16 |
| aberto | open | traduz | poph-server | 15 |
| alteradas | changed | traduz | poph-server | 15 |
| alterado | changed | traduz | poph-server, poph | 15 |
| conjunto | set | traduz | poph, poph-server | 15 |
| encontrado | found | traduz | poph, poph-server | 15 |
| escalares | scalars | traduz | poph | 15 |
| esquecer | forget | traduz | poph-server | 15 |
| fechado | closed | traduz | poph-server, poph-fluxos | 15 |
| ferramenta | tool | traduz | poph, poph-server | 15 |
| inseridas | inserted | traduz | console, poph-server | 15 |
| leitura | read | traduz | poph-server, poph | 15 |
| ligacao | link | traduz | poph | 15 |
| redigidas | drafted | traduz | poph-server | 15 |
| sincronizar | synchronise | traduz | poph, poph-server | 15 |
| sobrescrever | overwrite | traduz | console, poph | 15 |
| terceira | third | traduz | poph-server, poph-fluxos | 15 |
| aceita | accepts | traduz | poph-fluxos, poph-server | 14 |
| achado | finding | traduz | poph-fluxos, poph-server | 14 |
| apertado | tight | traduz | poph-server, poph-fluxos | 14 |
| assinados | signed | traduz | poph-server | 14 |
| ativas | active | traduz | poph-server, console | 14 |
| ativos | active | traduz | poph-server, poph | 14 |
| chaves | keys | traduz | poph, poph-fluxos | 14 |
| colecao | collection | traduz | poph-server | 14 |
| comercial | commercial | traduz | poph-server, scripts | 14 |
| confianca | confidence | traduz | poph, console | 14 |
| configurado | configured | traduz | console, poph-server | 14 |
| contador | counter | traduz | poph-server, poph-fluxos | 14 |
| desconhecidas | unknown | traduz | poph-server | 14 |
| entao | then | traduz | poph-fluxos, raiz | 14 |
| estados | states | traduz | poph-server | 14 |
| gravados | stored | traduz | poph-server, console | 14 |
| inteiro | integer | traduz | poph, poph-fluxos | 14 |
| legenda | caption | traduz | poph-server | 14 |
| nulo | null | traduz | poph-fluxos, poph-server | 14 |
| opcao | option | traduz | poph-fluxos | 14 |
| removido | removed | traduz | poph-fluxos, console | 14 |
| saltos | jumps | traduz | poph-fluxos, poph-server | 14 |
| segmentos | segments | traduz | poph-server | 14 |
| senao | else | traduz | poph-fluxos, raiz | 14 |
| tempos | times | traduz | poph-fluxos | 14 |
| tratar | handle | traduz | poph-server | 14 |
| vazamento | leak | traduz | poph-server | 14 |
| adiados | deferred | traduz | poph-server | 13 |
| ajuda | help | traduz | poph, poph-fluxos | 13 |
| apaga | deletes | traduz | poph-server, poph | 13 |
| biblioteca | library | traduz | poph-server, poph | 13 |
| busca | search | traduz | poph, poph-server | 13 |
| chegada | arrival | traduz | poph-fluxos, poph-server | 13 |
| desconhecida | unknown | traduz | poph-server, console | 13 |
| divergente | divergent | traduz | poph-server, poph-fluxos | 13 |
| extrator | extractor | traduz | poph-files, scripts | 13 |
| filho | child | traduz | poph, poph-server | 13 |
| isolamento | isolation | traduz | poph-server | 13 |
| juntos | together | traduz | poph-server, poph | 13 |
| notificar | notify | traduz | poph-server, raiz | 13 |
| posicional | positional | traduz | poph-fluxos | 13 |
| publicacao | publication | traduz | poph-server, poph | 13 |
| receber | receive | traduz | poph-server | 13 |
| renomear | rename | traduz | poph-server, poph-fluxos | 13 |
| repetido | repeated | traduz | poph, poph-server | 13 |
| somar | sum | traduz | poph-server | 13 |
| tarefas | tasks | traduz | poph-server | 13 |
| trava | lock | traduz | poph-server, poph-fluxos | 13 |
| versoes | versions | traduz | poph-server, poph | 13 |
| baixado | downloaded | traduz | poph-server, poph | 12 |
| compilar | compile | traduz | poph-server, poph | 12 |
| condicao | condition | traduz | poph-server | 12 |
| confere | checks | traduz | poph-server, poph | 12 |
| criadas | created | traduz | poph-server | 12 |
| degradada | degraded | traduz | console | 12 |
| estruturado | structured | traduz | poph-server, poph | 12 |
| exportar | export | traduz | poph, poph-server | 12 |
| falta | missing | traduz | poph-fluxos, poph | 12 |
| identificador | identifier | traduz | poph-server | 12 |
| imagem | image | traduz | poph-server | 12 |
| importacao | import | traduz | poph, poph-server | 12 |
| inativo | inactive | traduz | poph, poph-server | 12 |
| leitor | reader | traduz | poph | 12 |
| passos | steps | traduz | poph-server | 12 |
| preco | price | traduz | console, poph-server | 12 |
| recebidos | received | traduz | poph-server | 12 |
| regras | rules | traduz | poph-fluxos, poph-agent | 12 |
| repetir | repeat | traduz | poph-server, poph-fluxos | 12 |
| reserva | reservation | traduz | poph-server | 12 |
| segmento | segment | traduz | poph-server, poph | 12 |
| usuario | user | traduz | poph-server, poph | 12 |
| abertura | opening | traduz | poph-server, poph | 11 |
| adicionadas | added | traduz | console, poph-server | 11 |
| agendar | schedule | traduz | poph-fluxos, poph-server | 11 |
| deixa | leaves | traduz | poph-server, poph-fluxos | 11 |
| duplicadas | duplicated | traduz | poph-server | 11 |
| esperadas | expected | traduz | poph-server, poph | 11 |
| foto | photo | traduz | poph-server, poph | 11 |
| gerado | generated | traduz | poph, poph-server | 11 |
| humano | human | traduz | poph-server, poph-store | 11 |
| informado | informed | traduz | poph-server, console | 11 |
| inicial | initial | traduz | poph-server, poph-fluxos | 11 |
| janela | window | traduz | poph-server, poph | 11 |
| leituras | reads | traduz | poph-server | 11 |
| medir | measure | traduz | poph-server | 11 |
| memoria | memory | traduz | poph, poph-server | 11 |
| metrica | metric | traduz | poph-server | 11 |
| mudancas | changes | traduz | poph-server, poph | 11 |
| novos | new | traduz | poph-fluxos, console | 11 |
| nunca | never | traduz | poph-server, poph | 11 |
| orfaos | orphans | traduz | poph | 11 |
| passa | passes | traduz | poph-fluxos, poph-server | 11 |
| passagem | passage | traduz | poph-server, poph-fluxos | 11 |
| recusar | refuse | traduz | poph-server, poph | 11 |
| redacao | writing | traduz | poph-server | 11 |
| removidas | removed | traduz | console, poph-server | 11 |
| situacoes | situations | traduz | console | 11 |
| trocaram | swapped | traduz | console, poph-server | 11 |
| variaveis | variables | traduz | poph | 11 |
| vezes | times | traduz | poph-server, poph-fluxos | 11 |
| vinculo | link | traduz | poph-server, poph | 11 |
| vistas | views | traduz | poph-server | 11 |
| aceito | accepted | traduz | poph-fluxos | 10 |
| amostras | samples | traduz | poph-server | 10 |
| antiga | old | traduz | poph-server, poph | 10 |
| bruto | raw | traduz | poph-server | 10 |
| comentarios | comments | traduz | poph-fluxos, poph | 10 |
| concluida | finished | traduz | poph-fluxos, poph-server | 10 |
| construir | build | traduz | poph-server, poph | 10 |
| declarados | declared | traduz | poph-server | 10 |
| direto | direct | traduz | poph-server, poph | 10 |
| entrar | enter | traduz | poph-fluxos, raiz | 10 |
| esgotadas | exhausted | traduz | poph-server | 10 |
| finalizada | finalised | traduz | poph-store, poph-agent | 10 |
| forma | form | traduz | poph, poph-fluxos | 10 |
| gravacao | recording | traduz | poph-server | 10 |
| operacao | operation | traduz | poph-server, poph | 10 |
| preparar | prepare | traduz | poph-server, poph | 10 |
| propria | own | traduz | poph-server, poph | 10 |
| quente | hot | traduz | poph, poph-server | 10 |
| recarregar | reload | traduz | poph-server | 10 |
| registrada | recorded | traduz | poph-server | 10 |
| secao | section | traduz | poph, poph-server | 10 |
| segue | follows | traduz | poph-server, poph | 10 |
| segura | safe | traduz | poph-server, console | 10 |
| terceiro | third | traduz | poph-server | 10 |
| unico | single | traduz | poph-server, poph-fluxos | 10 |
| vencidas | expired | traduz | poph-server | 10 |
| verificar | verify | traduz | poph, poph-server | 10 |
| alheio | foreign | traduz | poph-server | 9 |
| booleano | boolean | traduz | poph-fluxos, poph | 9 |
| carga | load | traduz | poph-fluxos | 9 |
| celulas | cells | traduz | poph-server | 9 |
| cria | creates | traduz | poph, poph-server | 9 |
| declara | declares | traduz | poph-server, poph | 9 |
| diferenca | difference | traduz | poph-server | 9 |
| enfileirar | enqueue | traduz | poph-server | 9 |
| latencia | latency | traduz | poph-server, poph | 9 |
| meio | middle | traduz | poph-server | 9 |
| metodos | methods | traduz | poph-server | 9 |
| orientacao | guidance | traduz | raiz, poph | 9 |
| pares | pairs | traduz | poph, poph-server | 9 |
| parte | part | traduz | poph-server, poph | 9 |
| pede | asks | traduz | poph-fluxos, poph | 9 |
| permitido | allowed | traduz | poph-server | 9 |
| peso | weight | traduz | poph-fluxos | 9 |
| preenchidos | filled | traduz | console | 9 |
| proximo | next | traduz | poph-server | 9 |
| publicado | published | traduz | poph-server | 9 |
| sao | are | traduz | poph-server, poph | 9 |
| transferencia | transfer | traduz | poph-server | 9 |
| caracteres | characters | traduz | poph, poph-fluxos | 8 |
| classe | class | traduz | poph-server | 8 |
| continua | continues | traduz | poph-server, poph | 8 |
| definitiva | definitive | traduz | poph-server | 8 |
| estavel | stable | traduz | poph, poph-server | 8 |
| exclusao | exclusion | traduz | poph-server, poph | 8 |
| fator | factor | traduz | poph-server | 8 |
| guardado | stored | traduz | poph-server, poph | 8 |
| instalado | installed | traduz | poph-server, poph | 8 |
| liberados | released | traduz | poph-server, poph | 8 |
| limitador | limiter | traduz | poph-server | 8 |
| longo | long | traduz | poph-server, poph | 8 |
| maximo | maximum | traduz | poph-server, poph | 8 |
| onde | where | traduz | poph, console | 8 |
| parado | stopped | traduz | poph, poph-server | 8 |
| permissao | permission | traduz | poph | 8 |
| recurso | resource | traduz | poph-server | 8 |
| simulacao | simulation | traduz | poph | 8 |
| sintaxe | syntax | traduz | poph, poph-server | 8 |
| terminar | finish | traduz | poph-server | 8 |
| transporte | transport | traduz | poph, poph-server | 8 |
| tres | three | traduz | poph-fluxos, poph-server | 8 |
| trocado | swapped | traduz | poph-server, poph | 8 |
| abandonar | abandon | traduz | poph-server | 7 |
| anotadora | annotator | traduz | poph-server | 7 |
| atendimentos | services | traduz | poph-fluxos, poph | 7 |
| atualizacoes | updates | traduz | poph | 7 |
| buscar | search | traduz | poph-server, poph | 7 |
| caminhos | paths | traduz | poph-fluxos, poph-server | 7 |
| conhecimento | knowledge | traduz | console, poph-server | 7 |
| contagem | count | traduz | poph-server, poph | 7 |
| curto | short | traduz | poph-server, poph | 7 |
| desconectar | disconnect | traduz | poph-server, console | 7 |
| disposicao | layout | traduz | poph-server | 7 |
| escolhido | chosen | traduz | poph-server | 7 |
| fala | speaks | traduz | poph-server | 7 |
| faltando | missing | traduz | poph-fluxos, poph | 7 |
| guardados | guarded | traduz | poph-server, poph | 7 |
| limpa | clean | traduz | poph-server, poph-fluxos | 7 |
| minimo | minimum | traduz | poph-server, poph-fluxos | 7 |
| nas | in_the | traduz | poph-server, poph | 7 |
| novas | new | traduz | poph-server | 7 |
| posicionar | position | traduz | poph-server | 7 |
| previa | preview | traduz | poph-server | 7 |
| respeita | respects | traduz | poph-server, poph | 7 |
| suporte | support | traduz | poph-server, scripts | 7 |
| telefone | phone | traduz | poph-server | 7 |
| usada | used | traduz | poph-server, poph-fluxos | 7 |
| valida | validates | traduz | poph-server, poph-fluxos | 7 |
| vinculado | linked | traduz | poph, poph-server | 7 |
| agir | act | traduz | poph-server, poph-fluxos | 6 |
| apagado | deleted | traduz | poph | 6 |
| armazem | store | traduz | poph | 6 |
| carrinhos | carts | traduz | console | 6 |
| cifrados | encrypted | traduz | poph | 6 |
| corpos | bodies | traduz | poph-server | 6 |
| deixou | left | traduz | poph-fluxos | 6 |
| desconhecido | unknown | traduz | poph-server, poph | 6 |
| direcao | direction | traduz | poph-server | 6 |
| dominio | domain | traduz | poph-server | 6 |
| entra | enters | traduz | poph-server, poph-fluxos | 6 |
| esperas | waits | traduz | poph-fluxos, poph-server | 6 |
| falar | speak | traduz | poph-server | 6 |
| filhos | children | traduz | poph-server | 6 |
| notificacao | notification | traduz | poph-server, poph-fluxos | 6 |
| observador | observer | traduz | poph | 6 |
| pai | parent | traduz | poph | 6 |
| pode | can | traduz | poph, poph-server | 6 |
| processar | process | traduz | poph-server | 6 |
| provisionador | provisioner | traduz | poph-server | 6 |
| quantidade | quantity | traduz | poph-server | 6 |
| reabrir | reopen | traduz | poph-server | 6 |
| recarga | reload | traduz | poph-server | 6 |
| retirar | withdraw | traduz | poph | 6 |
| subida | rise | traduz | poph-server | 6 |
| transferida | transferred | traduz | poph-server | 6 |
| usa | uses | traduz | poph-server, poph-fluxos | 6 |
| acertos | hits | traduz | poph-server | 5 |
| agenda | schedule | traduz | poph-server | 5 |
| apagar | delete | traduz | poph, poph-server | 5 |
| avancou | advanced | traduz | poph-fluxos | 5 |
| centavos | cents | traduz | console, poph-server | 5 |
| conflitos | conflicts | traduz | poph-server, poph | 5 |
| controle | control | traduz | poph-server | 5 |
| disco | disk | traduz | poph-server, poph | 5 |
| dono | owner | traduz | poph-server | 5 |
| duplicado | duplicate | traduz | poph, poph-server | 5 |
| estatisticas | statistics | traduz | poph-server | 5 |
| faixa | range | traduz | poph, poph-server | 5 |
| futuro | future | traduz | poph | 5 |
| guardar | keep | traduz | poph-server | 5 |
| idempotente | idempotent | traduz | poph, poph-fluxos | 5 |
| instalacao | installation | traduz | poph | 5 |
| listagem | listing | traduz | poph-server | 5 |
| lugar | place | traduz | poph-server, poph | 5 |
| medicao | measurement | traduz | scripts, poph-server | 5 |
| migracao | migration | traduz | poph | 5 |
| minuscula | lowercase | traduz | poph-server | 5 |
| nenhuma | none | traduz | poph-fluxos, poph-server | 5 |
| ordenadas | sorted | traduz | poph-server | 5 |
| remota | remote | traduz | poph-server | 5 |
| removidos | removed | traduz | console, poph | 5 |
| resultados | results | traduz | poph-server, poph-fluxos | 5 |
| revogado | revoked | traduz | poph, poph-server | 5 |
| rodadas | rounds | traduz | console, poph-server | 5 |
| sensivel | sensitive | traduz | poph, poph-server | 5 |
| simultaneos | simultaneous | traduz | raiz, poph-agent | 5 |
| soma | sum | traduz | poph-server | 5 |
| temporario | temporary | traduz | poph | 5 |
| tudo | everything | traduz | poph-server, scripts | 5 |
| vazia | empty | traduz | poph, poph-fluxos | 5 |
| voltou | returned | traduz | poph, poph-fluxos | 5 |
| abaixo | below | traduz | poph, poph-server | 4 |
| abandonada | abandoned | traduz | poph-fluxos, poph-server | 4 |
| acento | accent | traduz | poph, poph-fluxos | 4 |
| alterar | modify | traduz | poph, poph-fluxos | 4 |
| ano | year | traduz | poph | 4 |
| aplicar | apply | traduz | poph | 4 |
| ativa | active | traduz | poph-server, poph-store | 4 |
| auditorias | audits | traduz | poph-server | 4 |
| baixada | downloaded | traduz | poph-server | 4 |
| cai | falls | traduz | poph, poph-server | 4 |
| cargas | loads | traduz | poph-server | 4 |
| combinador | combinator | traduz | poph-fluxos | 4 |
| convites | invitations | traduz | poph-server | 4 |
| definitivas | definitive | traduz | poph-server | 4 |
| desde | since | traduz | poph | 4 |
| encerramento | closure | traduz | poph-fluxos, poph-server | 4 |
| entrou | entered | traduz | poph-store, poph-server | 4 |
| escolher | choose | traduz | poph-server | 4 |
| esta | this | traduz | poph-server, poph | 4 |
| existente | existing | traduz | poph | 4 |
| expulsoes | evictions | traduz | poph-server | 4 |
| filtros | filters | traduz | poph-server | 4 |
| leva | takes | traduz | poph-server, poph | 4 |
| mais | more | traduz | poph-server, poph-fluxos | 4 |
| margem | margin | traduz | console | 4 |
| palavra | word | traduz | raiz | 4 |
| perfil | profile | traduz | poph-server | 4 |
| perguntas | questions | traduz | console | 4 |
| problema | problem | traduz | raiz, poph | 4 |
| resolucao | resolution | traduz | poph | 4 |
| rodar | run | traduz | poph | 4 |
| saude | health | traduz | poph-server, raiz | 4 |
| sistema | system | traduz | poph-server, scripts | 4 |
| sufixo | suffix | traduz | poph, poph-server | 4 |
| tocar | touch | traduz | poph-server | 4 |
| traco | trace | traduz | poph | 4 |
| achadas | found | traduz | poph-fluxos | 3 |
| agiram | acted | traduz | poph-server, poph-store | 3 |
| alterados | changed | traduz | console | 3 |
| antigo | old | traduz | poph-server | 3 |
| apagada | deleted | traduz | poph-server, scripts | 3 |
| apagou | deleted | traduz | poph-server | 3 |
| atualizada | updated | traduz | poph-server, poph-store | 3 |
| cancelamento | cancellation | traduz | poph | 3 |
| capturar | capture | traduz | poph-server | 3 |
| colunas | columns | traduz | poph | 3 |
| compartilhado | shared | traduz | poph, poph-fluxos | 3 |
| confirmar | confirm | traduz | poph-server | 3 |
| consultar | query | traduz | poph | 3 |
| consultas | queries | traduz | poph-server | 3 |
| convidados | guests | traduz | poph-server | 3 |
| decididas | decided | traduz | console | 3 |
| encerradas | closed | traduz | poph-server | 3 |
| encerrado | closed | traduz | poph-fluxos | 3 |
| enviada | sent | traduz | poph-server | 3 |
| escritor | writer | traduz | poph | 3 |
| espelhada | mirrored | traduz | poph-server | 3 |
| estava | was | traduz | poph | 3 |
| expirada | expired | traduz | poph-server | 3 |
| extensao | extension | traduz | poph-server, poph | 3 |
| fechamento | closing | traduz | poph-server, poph-fluxos | 3 |
| forcado | forced | traduz | poph | 3 |
| frase | sentence | traduz | poph-server, console | 3 |
| gerados | generated | traduz | console | 3 |
| hoje | today | traduz | poph-server | 3 |
| ida | trip | traduz | poph-fluxos, poph-server | 3 |
| inalterados | unchanged | traduz | console | 3 |
| legivel | readable | traduz | poph, poph-server | 3 |
| lidos | read | traduz | poph-server | 3 |
| limiar | threshold | traduz | poph-server, poph | 3 |
| mantem | keeps | traduz | poph, poph-fluxos | 3 |
| normalizar | normalise | traduz | poph | 3 |
| organizacao | organisation | traduz | poph-fluxos | 3 |
| paginacao | pagination | traduz | poph-server | 3 |
| poupadas | saved | traduz | poph-server | 3 |
| presenca | presence | traduz | poph | 3 |
| prontas | ready | traduz | poph-server | 3 |
| reais | actual | traduz | poph-server | 3 |
| recusados | refused | traduz | poph-server, poph | 3 |
| relevancia | relevance | traduz | console | 3 |
| risco | risk | traduz | poph, raiz | 3 |
| seguro | safe | traduz | poph-server | 3 |
| similaridade | similarity | traduz | console | 3 |
| validador | validator | traduz | poph-fluxos | 3 |
| velho | old | traduz | poph-server, poph | 3 |
| vincular | link | traduz | poph | 3 |
| abstencao | abstention | traduz | poph | 2 |
| aguardando | waiting | traduz | poph-server | 2 |
| ajuste | adjustment | traduz | poph-server | 2 |
| alfa | alpha | traduz | poph-fluxos | 2 |
| baixa | low | traduz | poph-server | 2 |
| caixa | box | traduz | poph-fluxos | 2 |
| candidata | candidate | traduz | poph-fluxos | 2 |
| celula | cell | traduz | poph-server | 2 |
| conclusao | completion | traduz | poph-server | 2 |
| condicoes | conditions | traduz | console | 2 |
| consertar | repair | traduz | poph-server | 2 |
| decidir | decide | traduz | poph-server | 2 |
| derivado | derived | traduz | poph | 2 |
| despachar | dispatch | traduz | poph-server | 2 |
| dia | day | traduz | poph, poph-server | 2 |
| equipe | team | traduz | poph-fluxos, poph-server | 2 |
| escolhas | choices | traduz | poph-server | 2 |
| esgotado | exhausted | traduz | poph-server | 2 |
| espelhar | mirror | traduz | poph-server | 2 |
| exibicao | display | traduz | poph-fluxos | 2 |
| extracao | extraction | traduz | poph-server | 2 |
| extraido | extracted | traduz | poph-server | 2 |
| fecha | closes | traduz | poph-server | 2 |
| hora | hour | traduz | poph-server | 2 |
| idade | age | traduz | poph | 2 |
| indexacao | indexing | traduz | poph | 2 |
| instante | instant | traduz | poph, poph-server | 2 |
| largura | width | traduz | poph-fluxos | 2 |
| limpar | clean | traduz | poph-fluxos, scripts | 2 |
| malformado | malformed | traduz | poph-server | 2 |
| marcar | mark | traduz | poph | 2 |
| mediana | median | traduz | poph-server | 2 |
| mes | month | traduz | poph | 2 |
| obtido | obtained | traduz | poph-fluxos | 2 |
| papel | role | traduz | poph-server | 2 |
| parametros | parameters | traduz | poph-server | 2 |
| pareamento | pairing | traduz | poph-server | 2 |
| pedacos | pieces | traduz | poph-server | 2 |
| pergunta | question | traduz | poph-server | 2 |
| politica | policy | traduz | poph-server | 2 |
| precisa | needs | traduz | poph, poph-server | 2 |
| proprias | own | traduz | poph | 2 |
| propriedades | properties | traduz | poph-server | 2 |
| recebida | received | traduz | poph-server | 2 |
| reenvios | resends | traduz | poph-server | 2 |
| repeticao | repetition | traduz | poph-server | 2 |
| revogar | revoke | traduz | poph | 2 |
| sentido | meaning | traduz | poph-server | 2 |
| transferir | transfer | traduz | poph-server | 2 |
| truncar | truncate | traduz | poph | 2 |
| unicos | single | traduz | poph-server | 2 |
| unidade | unit | traduz | console | 2 |
| validas | valid | traduz | poph-fluxos | 2 |
| alteracao | change | traduz | poph | 1 |
| aninhado | nested | traduz | poph-server | 1 |
| aplica | applies | traduz | poph-server | 1 |
| conexoes | connections | traduz | poph-server | 1 |
| contato | contact | traduz | raiz | 1 |
| dica | hint | traduz | poph-fluxos | 1 |
| edicao | edit | traduz | poph-server | 1 |
| enganosa | misleading | traduz | poph | 1 |
| exigidos | required | traduz | poph-fluxos | 1 |
| expor | expose | traduz | poph-server | 1 |
| fixa | fixed | traduz | poph-fluxos | 1 |
| frente | front | traduz | poph-server | 1 |
| habilitada | enabled | traduz | poph | 1 |
| habilitado | enabled | traduz | poph | 1 |
| ilegivel | unreadable | traduz | poph | 1 |
| impacto | impact | traduz | poph-fluxos | 1 |
| inicia | starts | traduz | poph-server | 1 |
| migracoes | migrations | traduz | poph | 1 |
| minutos | minutes | traduz | scripts | 1 |
| opcional | optional | traduz | poph-fluxos | 1 |
| organizacoes | organisations | traduz | poph-server | 1 |
| palavras | words | traduz | poph-fluxos | 1 |
| pausa | pause | traduz | poph-server | 1 |
| pessoa | person | traduz | poph-server | 1 |
| ponto | point | traduz | poph-server | 1 |
| preparado | prepared | traduz | poph-server | 1 |
| programa | program | traduz | poph-server | 1 |
| recente | recent | traduz | poph-server | 1 |
| rede | network | traduz | poph-server | 1 |
| reiniciar | restart | traduz | poph-server | 1 |
| rotina | routine | traduz | poph-server | 1 |
| salvo | saved | traduz | poph-server | 1 |
| selecionado | selected | traduz | console | 1 |
| travar | lock | traduz | poph-fluxos | 1 |
| usado | used | traduz | poph-fluxos | 1 |
| usuarios | users | traduz | poph-server | 1 |
| vinculos | links | traduz | poph-server | 1 |
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
