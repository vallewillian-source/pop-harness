# POP Harness: dados (`harness.db` e `rag.db`)

Entrega das **HR-04b** e **HR-07**. Descreve o esquema, as migrações, a concorrência e o que já
tem repositório. O contrato de conteúdo é o PRD §7.2; o que mudou de nome ou de forma está marcado
como **decisão** abaixo.

## 1. Os dois bancos

| Arquivo | O que guarda | Situação |
|---|---|---|
| `data/harness.db` | tudo que é durável: orgs, agentes, instruções, base, produtos, integrações, conversas, mensagens, entrada/saída, turnos, auditoria, fichas, fluxos e execuções | **esquema completo** (versão 18: migrações 0001 a 0018) |
| `data/rag.db` | trechos, vetores int8 e índice de texto (FTS5) | arquivo criado, **sem tabelas** (a HR-16 cria as migrações) |

Caminhos vêm da configuração (`dados.diretorio`, `dados.harness_db`, `dados.rag_db`) — ver
`docs/CONFIGURACAO.md`. `Store::abrir` cria a pasta e migra os dois; `poph migrate` faz o mesmo
pelo terminal, com o servidor parado.

### Pragmas (todas as conexões)

| Pragma | Valor | Por quê |
|---|---|---|
| `journal_mode` | `WAL` | leitura não bloqueia escrita; verificado na abertura — se não ficar `wal`, a abertura falha |
| `synchronous` | `NORMAL` | PRD §7.1; com `FULL` a vazão de escrita cai |
| `foreign_keys` | `ON` | senão o SQLite ignora as chaves estrangeiras silenciosamente |
| `busy_timeout` | 2.000 ms | curto de propósito: com um escritor só, espera longa é defeito, não carga |

### Convenções

- **`org_id` em toda tabela de negócio.** Exceções: `orgs` (é a própria org) e `schema_migrations`
  (infraestrutura). O teste `every_business_table_has_org_id` pergunta ao `sqlite_master` e
  reprova tabela nova sem `org_id`.
- **`version` nas entidades editáveis**, exigida no `If-Match` da escrita. `conversations.version`
  existe, mas mover a conversa (mensagem nova) **não** incrementa: escrituração de turno não pode
  invalidar o `If-Match` de quem está com a tela aberta. O que incrementa: mudar situação/reabrir.
- **Tempo em `INTEGER`** (milissegundos desde a época Unix). Ordenável, sem dependência de data e
  comparável com o relógio injetável do `Store`.
- **Dinheiro em `INTEGER` de centavos** (`price_cents`, `unit_price_cents`). Evita erro de
  arredondamento de ponto flutuante na soma do carrinho.
- **`STRICT` em todas as tabelas**: tipo errado falha na escrita em vez de virar lixo.
- **Ids são UUID v7 em `TEXT`**: ordenáveis por tempo, o que também dá a paginação (`id > cursor`).
- JSON guardado como `TEXT` (`*_json`): o esquema não muda quando o contrato do JSON evolui.
- **Referências compostas com `org_id`** nas colunas obrigatórias (`FOREIGN KEY (org_id, x_id)
  REFERENCES x (org_id, id)`): o banco recusa vínculo entre orgs diferentes, não só a aplicação.
  Cada tabela-pai tem índice único em `(org_id, id)` para isso. Exceção: anexo → mensagem usa
  chave simples de propósito — o índice único extra em `messages` custava o dobro da vazão de
  escrita (medido), e a org do anexo é conferida no repositório.
- **Referências opcionais** (departamento no agente, departamento alvo da conversa, produto no
  carrinho, agente do turno) usam chave simples com `ON DELETE SET NULL`: a composta obrigaria a
  anular o `org_id` na exclusão. Nessas, o repositório confere a org antes de gravar.

## 2. Concorrência: um escritor, vários leitores

- **Escritor dedicado**: uma thread recebe comandos por canal, junta até **512** comandos numa
  transação `BEGIN IMMEDIATE`, roda cada comando num `SAVEPOINT` (um erro não derruba o lote) e só
  responde **depois do commit**. Nenhum chamador recebe "ok" de escrita que não foi para o disco.
- **Leitores**: pool de até 8 conexões só de leitura; se todas estiverem ocupadas, abre uma extra e
  descarta ao devolver.
- `Store` é `Send + Sync`: as threads da aplicação compartilham o mesmo escritor.
- **Contador de execução conta dentro do comando, antes do `COMMIT` do lote.** Se o commit falhar
  depois, o contador já somou uma operação que o banco desfez: é métrica de operação, não dado do
  atendimento. O erro fica no log de quem chamou e a contagem segue; sem catálogo publicado
  (`poph migrate` e a CLI), nenhuma conta é feita.
- **Erro de restrição do banco vira validação com o campo certo.** `StoreError::from_write` (e
  `from_write_with`, que recebe o mapa coluna→campo do chamador) traduz a violação em
  `DomainError::Validation`: `UNIQUE` e `PRIMARY KEY` apontam o campo que o chamador informou (o
  valor repetido é dele); `CHECK` e `NOT NULL` apontam a coluna citada na mensagem do SQLite quando
  ela está no mapa, senão o campo do chamador; `FOREIGN KEY` aponta o campo do chamador, com o
  texto de que o registro não existe nesta org. Qualquer outro código estendido continua erro cru
  do banco (`StoreError::Sqlite`).
- **Medido** (`cargo test --release -p poph-store --test throughput -- --ignored --nocapture`, esta
  estação x86_64 de 12 núcleos):

| Cenário | Resultado (duas rodadas) | Meta (D-07 / HR-01) |
|---|---|---|
| uma thread, uma transação por escrita | 21.600 – 26.100 msg/s | — (piso) |
| 32 concorrentes, lote de 512 | **70.000 – 82.200 msg/s** | ≥ 50.000 msg/s ✅ |
| 256 concorrentes, lote de 512 | **75.400 – 89.300 msg/s** | ≥ 50.000 msg/s ✅ |
| chegada constante de 2.000 msg/s por 3 s | p50 **48–57 µs**, p95 **70–108 µs** | p95 ≤ 5 ms ✅ |
| 10 mil escritas concorrentes (teste do CI) | sem `SQLITE_BUSY`, sem perda | obrigatório ✅ |

A faixa vem da carga da estação (outros trabalhos compilando em paralelo); a rodada mais lenta é a
referência conservadora.

O tamanho do lote foi escolhido por medida: 256 → 55 mil msg/s; 512 → 60 mil; 1024 → 60 mil (não
ganha acima de 512). Números com o esquema final (chaves compostas incluídas).

Duas decisões de vazão, medidas nesta entrega:
- **`write` de mensagem não atualiza a conversa.** Atualizar `last_message_at` a cada mensagem
  derrubava de 75 mil para 54 mil msg/s. O marcador é movido pelo turno, uma vez por lote
  (`Conversas::tocar`), e a lista de conversas fica com precisão de um turno.
- **Anexo usa chave estrangeira simples para a mensagem** (e não composta com `org_id`): o índice
  único extra em `messages` derrubava de 75 mil para 30 mil msg/s.

## 3. Migrações

Runner próprio (decisão: ~60 linhas, sem `rusqlite_migration`), porque o PRD §7.2 já exige a tabela
`schema_migrations` com versão, nome e data para o `poph doctor` ler.

- Cada migração é um arquivo SQL embutido no binário por `include_str!` e registrado em
  `crates/poph-store/src/migrations.rs`.
- `poph migrate` (e a abertura do `Store`) aplica só o que falta; rodar de novo não faz nada.
- Cada migração roda num `SAVEPOINT`: funciona sozinha ou dentro da transação do escritor, e uma
  migração que falha não leva as outras.
- **Para criar uma migração nova:** acrescente o arquivo em `migrations/harness/NNNN_nome.sql` e
  registre em `HARNESS_MIGRATIONS` com o próximo número. Nunca edite uma migração já publicada.
- `rag.db` hoje tem lista vazia; a versão dele é `0` e o doctor mostra `esquema versão 0`.
- A numeração pula a `0014`: o arquivo não existe no repositório e `migrations.rs` não a registra. As
  migrações do módulo de Fluxos e Fichas são a `0012` e a `0013`; o próximo número livre é o `0017`.

O `poph doctor` lê a versão **sem abrir o banco para escrita**: `file_version` abre em modo
somente leitura e consulta `schema_migrations` (antes ele lia o cabeçalho `user_version`, que não
acompanha as migrações).

## 4. Tabelas de `harness.db`

Abaixo, cada tabela com o que a coluna guarda quando o nome não é óbvio. Colunas `id`, `org_id`,
`version`, `created_at` e `updated_at` estão em todas as entidades editáveis e não se repetem na
descrição.

### Organizações e acesso

| Tabela | Colunas próprias |
|---|---|
| `orgs` | `name`, `slug` (único), `status` (`active`/`inactive`/`archived`), `config_json` (retenção, limites e padrões de modelo da org) |
| `admin_tokens` | `org_id` **nulo = token global** (os globais vêm da configuração, só o hash), `label`, `token_hash` (único), `revoked_at` |

### Produtos

| Tabela | Colunas próprias |
|---|---|
| `product_categories` | `name` (único por org, sem diferenciar maiúsculas), `description`, `status` |
| `products` | `category_id`, `name`, `slug` (único por org), `aliases_json`, `description`, `price_text` (como o cliente vê), `price_cents`, `unit`, `includes`, `excludes`, `conditions`, `tags_json`, `status` |

### Departamentos, agentes e instruções

| Tabela | Colunas próprias |
|---|---|
| `departments` | `name` (único por org), `description`, `status` |
| `agents` | ver §5 |
| `agent_product_categories` | `agent_id` + `category_id` (chave primária composta), `org_id` |
| `agent_instructions` | `agent_id` (chave primária), `structured_json` (`StructuredInstructions` da HR-03 §9.1), `markdown` (texto composto, prefixo do prompt), `derived_from_json` (`1` = markdown ainda derivado do JSON; `0` = editado à mão) |

### Base de conhecimento

| Tabela | Colunas próprias |
|---|---|
| `knowledge_groups` | `name` (único por org), `description`, `status` |
| `knowledge_items` | `group_id`, `title`, `content` (**1 a 1000 caracteres**, limite do POP Atende), `tags_json`, `source_type` (`manual`), `position`, `status` |
| `agent_knowledge_groups` | `agent_id` + `group_id`, `version` (paridade com o vínculo do POP Atende) |

### Arquivos e integrações

| Tabela | Colunas próprias |
|---|---|
| `file_library` | `agent_id`, `name`, `description`, `mime`, `size_bytes`, `sha256`, `blob_path` (relativo a `data/blobs`; o conteúdo não fica no SQLite) |
| `integrations` | `agent_id`, `kind`, `config_json`, `secrets_enc` (BLOB cifrado na HR-26), `status`, `connection_state` |

### Conversas, mensagens e anexos

| Tabela | Colunas próprias |
|---|---|
| `conversations` | `agent_id`, `integration_id` (nulo no console/CLI), `channel`, `external_peer`, `status` (`open`/`transferred`/`closed`), `target_department_id`, `summary` (resumo rolante, HR-14), `last_message_at`, `client_name` (nome do cliente no WhatsApp, nulo até chegar; migração `0008`, ver abaixo) |
| `messages` | `conversation_id`, `direction` (`in`/`out`), `role` (`user`/`assistant`/`tool`/`system`), `origin` (`agent`/`human`/`device`), `author_name`, `text`, `attachments_json`, `external_id`, `turn_id`, `created_at`. Gravar mensagem **não** mexe em `conversations.last_message_at` (ver §2) |

`messages.origin` diz quem produziu a mensagem: `agent` (o turno, e tudo que existia antes da coluna),
`human` (resposta de atendente, usada pelo módulo Matrix) e `device` (digitada no celular do cliente,
fase 2 do módulo). O padrão é `agent`, e a gravação do turno não muda. `author_name` guarda o nome do
atendente e fica nulo nas demais origens. Mensagens de entrada (`direction = 'in'`) ficam com o padrão
`agent`; esse valor não tem significado para elas. Colunas adicionadas pela migração `0003`, aditiva:
nenhuma linha existente é alterada.

**Migração `0004` (MX-04): `audit_log` aceita `matrix_user`.** A auditoria `human_takeover` da
resposta do atendente pelo Element grava `actor_kind = 'matrix_user'` (o id Matrix do atendente vai em
`actor_id`). O CHECK de `actor_kind` da `0001` não aceitava esse valor, então a tabela é reconstruída
com o CHECK estendido, no padrão de 12 passos do SQLite: todas as linhas existentes são copiadas sem
mudança e os dois índices (`ix_audit_log_time`, `ix_audit_log_org`) são recriados. Um valor fora da
lista continua recusado.

**Resposta humana (MX-04).** `Mensagens::registrar_resposta_humana` grava, numa única escrita e num
savepoint do escritor: a mensagem (`direction out`, `role assistant`, `origin human`, `author_name`,
`external_id = id do evento Matrix`, `turn_id` novo), o item da `outbox` (`text`, seq 0, payload
`{"texto": ...}` já formatado, mesmo `turn_id`), a passagem de `open` para `transferred` (a versão sobe
uma vez só; conversa já transferida não sobe) e a auditoria `human_takeover`. Devolve `Gravada`,
`Duplicada` (mesmo id de evento na conversa: nada é gravado de novo), `ConversaFechada` ou
`ConversaInexistente` (inclusive quando a conversa é de outra org). Se a auditoria falhar, a mensagem,
o item de saída e a passagem também não ficam gravados.
| `attachments` | `message_id`, `name`, `mime`, `size_bytes`, `sha256`, `blob_path`, `extracted_text`, `extraction_status` (`pending`/`ok`/`failed`/`skipped`) |

**Resposta humana com arquivo (MX-44).** `Mensagens::registrar_resposta_humana_com_arquivo` é a
mesma operação da MX-04 (mesma idempotência pelo id do evento, mesma passagem para `transferred`,
mesma auditoria `human_takeover`), com três diferenças:

- `messages.text` fica **vazio** — o atendente não digitou nada — e `messages.attachments_json`
  aponta para o anexo gravado na mesma escrita;
- entra uma linha em `attachments` com o tipo MIME **real** (conferido pelo conteúdo no `poph-files`,
  nunca pelo `mimetype` do evento) e `extraction_status = 'skipped'`: o arquivo do atendente não passa
  pelo extrator, porque nenhum turno do agente lê o que o atendente manda;
- o item da `outbox` é `file`, seq 0, com o payload do HR-24 mais a legenda do canal:
  `{"blob_ref": "<org>/<sha256>", "arquivo_id": "<id do anexo>", "nome": …, "mime": …,
  "tamanho_bytes": …, "sha256": …, "legenda": …}`. O `blob_ref` é relativo à pasta de blobs
  (`config.blobs()`), o mesmo lugar de onde o consumidor da fila lê o arquivo.

O blob já está no disco quando a operação roda: quem baixa o arquivo da sala (com teto de 16 MB e sem
seguir redirecionamento), confere o tipo e grava como blob da org é o módulo Matrix, antes de chamar o
repositório. Reentrega do mesmo evento devolve `Duplicada` e não cria segundo anexo nem segundo item
de fila.

`messages` tem índice único parcial em `(conversation_id, external_id)`: gravar a mesma mensagem
externa de novo devolve a original em vez de duplicar (reentrega do webhook).

**Migração `0008` (MX-32): `conversations.client_name`.** Coluna de texto, nulável e sem valor
padrão, aditiva: conversas existentes ficam com `NULL`. Guarda o nome que o cliente tem no WhatsApp
(o `pushName` do webhook).

- **Quem grava:** `receive_from_channel`, o caminho que o webhook da Evolution usa, na mesma transação da
  mensagem. A gravação roda num savepoint próprio (`customer_name`): se falhar, vira log com código
  curto e a mensagem entra do mesmo jeito. O nome nunca vai para o log.
- **Regra de normalização** (`normalise_name_client`, em `crates/poph-store/src/repos/conversations.rs`):
  espaços nas pontas saem, espaços repetidos viram um só, caractere de controle e de largura zero ou
  de direção de texto some (U+200B–U+200F, U+202A–U+202E, U+2066–U+2069), e o resultado tem no máximo
  100 caracteres. Nome que fica vazio não é gravado.
- **Atualização:** o `UPDATE` só age quando o valor muda. Sobe `updated_at` (é o que o espelho do
  Matrix lê para ver a mudança), e **não** sobe `version` (não gera conflito de `If-Match` no Console)
  nem `last_message_at`. Sempre filtrado por org.
- **Reentrega:** uma entrega repetida (mesmo id externo) devolve a mensagem já gravada e não regrava o
  nome. O próximo `pushName` do cliente, numa mensagem nova, é que atualiza.
- **Operação pública:** `Conversas::definir_nome_cliente` faz a mesma gravação, com transação própria,
  para uso fora do webhook.

### Listagem de conversas por cliente e por atividade (RP-02)

**Migração `0015` (RP-02): `ix_conversations_org_peer_id (org_id, external_peer, id)`.** Aditiva, só cria
o índice; nenhuma tabela nem dado muda.

| Consulta | Chave e ordem | Filtro | Índice |
|---|---|---|---|
| `Conversas::listar_filtrado` (lista sem `sort`) | `id` crescente, depois do cursor | `status` e `external_peer` opcionais | com cliente: `ix_conversations_org_peer_id`; sem cliente: o que o planejador escolhe (`ux_conversations_org_id`) |
| `Conversas::listar_recentes` (`sort=recent`) | `COALESCE(last_message_at, created_at)` decrescente, desempate `id` decrescente; sem cursor | igual | com cliente: `ix_conversations_org_peer_id`, fixado com `INDEXED BY` |

Por que esse índice e não o antigo: `ix_conversations_peer (org_id, channel, external_peer)` começa pelo
canal, que o filtro não traz, então a busca por cliente cairia numa varredura. O índice novo começa por
`(org_id, external_peer)`. Ele termina em `id` de propósito: sem isso, a ordem por id faz o planejador
escolher `ux_conversations_org_id` e ler a org inteira, filtrando o cliente linha a linha.

Por que `INDEXED BY` na ordenação por atividade: com `status` junto, o planejador prefere
`ix_conversations_status` e varre as conversas da situação na org inteira. A consulta com cliente fixa o
índice novo. Se o índice não existir, a consulta falha com erro de banco, em vez de ficar lenta sem aviso.

O teste `customer_query_plan_uses_new_index_and_does_not_scan_table`
(`crates/poph-store/tests/conversation_filters.rs`) confere o plano (`EXPLAIN QUERY PLAN`) das duas ordens.

**Identificador do cliente.** `Conversa::identificador_cliente()` é o `external_peer` sem espaço nas
pontas, ou o id da conversa quando ele está vazio. É o valor que a sala do Matrix usa como
`client_identifier`. Os módulos Matrix ainda calculam a mesma regra em cada ponto de uso; o teste
`customer_identifier_is_peer_without_space_or_id_when_empty` fixa o que ela devolve.
Limite: o filtro compara o texto exato. Um `external_peer` gravado com espaço nas pontas (o caminho do
canal não grava assim, só dígitos) não casaria com o identificador já aparado.

### Fichas e a linha do tempo do atendimento (FX-05)

A Ficha é o registro do atendimento, não do fluxo: ela nasce junto com a conversa e só fecha quando o
atendimento fecha. Migração `0012_fichas_e_eventos`, aditiva (não altera `conversations`,
`messages` nem `audit_log`).

| Tabela | Colunas próprias |
|---|---|
| `fichas` | `conversation_id` (**único**), `agent_id`, `canal`, `peer_externo`, `status` (`open`/`closed`), `aberta_em`, `fechada_em`, `fechada_por` (`cliente`/`agente`/`atendente`/`admin`/`fluxo`/`sistema`/`migracao`), `fechada_motivo`, `version` |
| `ficha_eventos` | `ficha_id`, `execucao_id` (nulo quando o evento não é de fluxo), `seq`, `tipo`, `ator_tipo` (`agente`/`atendente`/`sistema`/`jev`/`admin`), `ator_ref`, `payload_json`, `chave_idempotencia` |

Regras que o banco garante sozinho (teste `crates/poph-store/tests/cases_migration.rs`):

- **Uma Ficha por conversa**: índice único em `fichas.conversation_id`. No SQLite vários `NULL` são
  distintos num índice único, então a Ficha que sobreviveu à conversa apagada (`ON DELETE SET NULL`)
  não colide com as outras.
- **Ficha fechada carrega o fechamento, Ficha aberta não**: dois `CHECK` amarram `status` a
  `fechada_em` e a `fechada_por`, num sentido e no outro.
- **`seq` única por Ficha** (`ux_ficha_eventos_seq`): é o cursor da paginação da linha do tempo.
- **Idempotência opcional**: `ux_ficha_eventos_idem` é único parcial em `(ficha_id,
  chave_idempotencia)`, então evento sem chave nunca colide.
- **Evento não cruza org**: a referência é composta, `(org_id, ficha_id)` → `fichas (org_id, id)`, com
  `ON DELETE CASCADE`.
- **Os textos aceitos nas colunas `status`, `fechada_por`, `tipo` e `ator_tipo`** são exatamente os do
  vocabulário de `poph-types` (FX-03); o teste grava um de cada e confirma que valor inventado é
  recusado.

Duas decisões que o leitor do esquema precisa saber:

- **Os nomes de coluna destas duas tabelas são os do PRD §7.1 e estão em português** (`canal`,
  `peer_externo`, `aberta_em`, `fechada_em`), ao contrário do resto do banco (`channel`,
  `external_peer`, `created_at`). Foi decisão de contrato do mantenedor na FX-05: são os nomes que a API
  e o Console vão usar.
- **`agent_id`, `canal` e `peer_externo` são retrato, sem chave estrangeira**: apagar o agente apaga as
  conversas dele (cascata) e a Ficha fica para o histórico. `execucao_id` também ficou sem FK: na
  FX-05 `fluxo_execucoes` ainda não existia, e o SQLite não acrescenta chave estrangeira a tabela
  existente sem reconstruí-la. Na FX-06 o mantenedor decidiu **não** reconstruir `ficha_eventos` — quem
  confere a org e a execução é o repositório (FX-07).

**Backfill (PRD §6.1-5).** A migração cria uma Ficha para cada conversa que já existia: `open` e
`transferred` viram Ficha `open`; `closed` vira Ficha `closed` com `fechada_por = 'migracao'`,
`fechada_em` igual ao `updated_at` da conversa (numa conversa fechada é o instante do fechamento) e
`aberta_em` igual ao `created_at`. O id é um UUID v7 montado no próprio SQL (tempo da conversa nos 48
bits altos), então continua ordenável por tempo como os ids do código. O comando tem guarda
`NOT EXISTS`, então repeti-lo não cria Ficha nova. O backfill **não** grava evento em `ficha_eventos`
nem linha em `audit_log`: quem registra o que houve é o `fechada_por = 'migracao'`.

**A invariante vale para conversa nova desde a FX-07.** A Ficha nasce **na mesma escrita** que cria a
conversa, em dois pontos — os únicos que criam conversa no repositório:

- `Conversas::criar` (todos os chamadores passam por ela: `fila_integracao.rs`, `teste_chat.rs`,
  `poph/src/chat.rs`, `poph-plugin-eco/src/demo.rs`);
- `CaixaEntrada::receber_do_canal`, que insere a conversa em SQL próprio — é o caminho que o WhatsApp
  usa de fato, e é por isso que a Ficha também é criada lá. Ficha criada só em `Conversas::criar`
  deixaria o atendimento real sem registro.

Nascer com a conversa inclui o evento `ficha_aberta` (ator `sistema`), então a linha do tempo de um
atendimento novo começa preenchida. A Ficha **não** nasce ao receber cada mensagem: a mensagem
seguinte na mesma conversa não cria Ficha nova (teste
`channel_reception_creates_one_case_per_conversation_and_not_per_message`).

**Fechar a conversa fecha a Ficha, na mesma transação** (PRD L5 e L6). Os caminhos são
`Conversas::fechar_auditado` (`POST …/close`, com `fechada_por = 'admin'`) e
`Conversas::mudar_status(Fechada)` (`fechada_por = 'sistema'`, porque esse caminho genérico não recebe
autor). Na mesma escrita, a execução de fluxo `ativa` da Ficha vira `abandonada` com
`motivo_fim = 'atendimento_encerrado'`, e a linha do tempo ganha `execucao_abandonada` (quando havia
execução) e `ficha_fechada`, nessa ordem. O savepoint do escritor é a garantia: falha injetada no meio
não deixa conversa fechada com Ficha aberta (teste
`failure_midway_through_closing_leaves_no_closed_conversation_with_open_case`).

**Conversa `closed` não reabre** (PRD L13): `reopen_audited` e `change_status(Open)` devolvem
`ConversaEncerrada` (409 `conversa_encerrada`). Conversa `transferred` continua reabrindo e a Ficha
dela segue aberta — transferir não fecha atendimento. A mensagem seguinte de um cliente com a conversa
fechada abre **outra** conversa e **outra** Ficha; a Ficha velha continua fechada e recusa evento novo.

**O repositório (`Store::cases()`)** oferece `create`, `get`, `by_conversation`, `list` (filtros
`status`, `agente`, `canal` e `since`, com paginação por cursor de id), `events` (paginado por `seq`,
que é o cursor da linha do tempo) e `register_event` (idempotente por `chave_idempotencia`). O `seq`
é calculado como `MAX(seq) + 1` dentro da escrita, então dois eventos não disputam a mesma posição.
`ficha_eventos.execucao_id` continua sem chave estrangeira: quem confere que a execução existe **nesta
org** é o repositório, antes de gravar.

**O que ainda não existe:** rota de Ficha (FX-29), início de execução e transição do motor (FX-08,
FX-13) e a fila de efeitos (FX-21). Até lá, ninguém escreve em `fluxo_execucoes` pelo repositório: o
único efeito do fechamento numa execução é o abandono descrito acima.

### Fluxos, revisões, execuções e efeitos (FX-06)

O fluxo pertence à **organização**, não ao agente: o agente recebe uma permissão explícita para usar
cada fluxo. Migração `0013_fluxos_revisoes_execucoes_e_efeitos`, aditiva — não altera nenhuma tabela
existente; o único acréscimo fora das seis tabelas é o índice `ix_fichas_org_agente` em `fichas`
(o filtro `?agente=` da lista de Fichas, que faltou na 0012).

| Tabela | Colunas próprias |
|---|---|
| `fluxos` | `slug` (único por org, vem do `id` do script), `nome`, `descricao`, `status` (`active`/`inactive`), `rascunho_fonte`, `revisao_atual`, `version` |
| `fluxo_revisoes` | `fluxo_id`, `numero` (único por fluxo), `fonte`, `fonte_sha256`, `definicao_json` (a IR), `schema_versao`, `publicada_em`, `publicada_por`, `resolucoes_json`. **Imutável** |
| `agente_fluxos` | `agent_id` + `fluxo_id` (chave primária com `org_id`), `liberado_em` — a permissão |
| `fluxo_execucoes` | `ficha_id`, `fluxo_id`, `revisao_id`, `status` (`ativa`/`concluida`/`abandonada`/`falha`), `no_atual`, `dados_json`, `resultado`, `motivo_fim`, `iniciada_em`, `finalizada_em`, `version` |
| `fluxo_execucao_etapas` | `execucao_id`, `no_id`, `ordem`, `status` (`pendente`/`parcial`/`concluida`), `dados_json`, `atores_que_agiram_json`, `entrou_em`, `concluiu_em` |
| `fluxo_efeitos` | `ficha_id`, `execucao_id`, `chave` (única por execução), `tipo`, `payload_json`, `estado`, `tentativas`, `ultimo_erro`, `proxima_tentativa`, `enviado_em`, `created_at` |

Regras que o banco garante sozinho (teste `crates/poph-store/tests/flows_migration.rs`):

- **Uma execução `ativa` por Ficha**: `ux_fluxo_execucoes_ativa` é único **parcial**
  (`WHERE status = 'ativa'`). Execuções encerradas se acumulam como histórico de qual fluxo passou
  por aquele atendimento.
- **Revisão publicada é imutável**: os gatilhos `tr_fluxo_revisoes_sem_update` e
  `tr_fluxo_revisoes_sem_delete` recusam `UPDATE` e `DELETE` com mensagem em português. O gatilho de
  exclusão tem um `WHEN` de propósito: apagar o **fluxo** (`DELETE /flows/{id}?force=true`) continua
  funcionando e leva as revisões dele na cascata — nesse instante o fluxo já não existe, e o gatilho
  só barra a exclusão de revisão solta, com o fluxo de pé.
- **Nada cruza organização**: as referências são compostas — revisão → fluxo, permissão → agente e →
  fluxo, execução → Ficha, etapa → execução, efeito → Ficha e → execução. Todas com
  `ON DELETE CASCADE` no pai.
- **Apagar o agente apaga só a permissão**: `agente_fluxos` cascateia com `agents`, e nem o fluxo nem
  as revisões (que são da organização) somem junto.
- **`slug` único por org, `numero` único por fluxo**: o mesmo slug em outra organização é aceito (o
  fluxo é dela); o número 1 existe em cada fluxo.
- **Idempotência do efeito**: `ux_fluxo_efeitos_chave` é único em `(org_id, execucao_id, chave)`;
  repetir a mesma transição não gera segunda mensagem.
- **Coerência entre situação e instante**: `CHECK` amarra `fluxo_execucoes.status` a `finalizada_em`,
  `fluxo_execucao_etapas.status` a `concluiu_em` e `fluxo_efeitos.estado` a `enviado_em`, num sentido
  e no outro.
- **Os textos de situação** (`status`, `estado`) e os três tipos de efeito (`mensagem_ao_cliente`,
  `notificar_whatsapp`, `encaminhar`) são lista fechada; o teste cruza com o vocabulário de
  `poph-types` (FX-03) e confirma que valor inventado é recusado.

Decisões que o leitor do esquema precisa saber:

- **`fluxo_execucoes.fluxo_id` e `revisao_id` ficam sem chave estrangeira.** Apagar um fluxo não pode
  apagar o histórico do atendimento (§6.2: "Fichas e eventos permanecem"); com FK em cascata, a
  exclusão forçada do fluxo levaria as execuções junto. Quem confere org e existência é o
  repositório, na mesma transação (mesmo precedente de `ficha_eventos.execucao_id`; decisão do
  mantenedor na FX-06: **não** reconstruir `ficha_eventos` para acrescentar FK).
- **`fluxos.revisao_atual` é número, sem FK para `fluxo_revisoes`.** As duas tabelas se referenciariam
  (revisão aponta o fluxo, fluxo aponta a revisão atual) e este projeto não usa FK adiável: quem
  garante a coerência é a transação de publicação (FX-09).
- **`fluxo_revisoes.publicada_por` guarda o tipo do ator** (`admin_token`, `cli`, `system`, `plugin`,
  `matrix_user` — o vocabulário de `audit_log.actor_kind`). O id do ator e o detalhe ficam em
  `audit_log` (`flow_publish`), para não duplicar auditoria dentro de uma linha imutável.
- **`fluxos` não tem `created_at`/`updated_at`**: a lista de fluxos ordena por nome, e as colunas são
  exatamente as do PRD §7.1. Se a tela precisar de "criado em", é migração nova.
- **Os nomes de tabela e coluna estão em português**, como os das Fichas: são os nomes que a API e o
  Console vão usar (decisão de contrato do mantenedor na FX-06).

**Vazão de escrita de mensagens, antes e depois** (`cargo test --release -p poph-store --test throughput
-- --ignored --nocapture`; a estação estava com outros agentes compilando, então vale a comparação,
não o valor absoluto):

| Rodada | Sem a 0013 | Com a 0013 |
|---|---|---|
| 1 thread | 22.253 msg/s | 21.615 msg/s |
| 32 concorrentes | 62.388 msg/s | 61.741 msg/s |
| 256 concorrentes | 69.774 msg/s | 67.319 / 68.723 msg/s |
| p95 da escrita | 105 µs | 97 / 204 µs |

As faixas se sobrepõem e o caminho de escrita de `messages` não foi tocado: os índices novos são de
tabelas que a mensagem não escreve (`fichas` recebe só leitura do novo índice).

**O que ainda não existe:** os repositórios de fluxos, revisões, permissão e Fichas são a FX-07 e a
FX-09; o motor que escreve execução, etapa e efeito é a FX-13 em diante. Até lá, ninguém grava nestas
tabelas — só o esquema existe.

### Repositório de fluxos e permissão por agente (FX-08)

`Store::flows()` (`crates/poph-store/src/repos/flows.rs`) é a única porta para as tabelas `fluxos` e
`agente_fluxos`. Não compila script e não publica revisão: quem grava `fluxo_revisoes` é a FX-09.

- **Cadastro.** `create`, `get`, `by_slug` (devolve `None` quando não existe), `list` (filtro de
  situação, cursor pelo id) e `update` com `If-Match` (`expected` = `version`). Versão velha devolve
  `ConflitoDeVersao` (412) e não grava nada. `archived` é recusado: o fluxo fica `active` ou `inactive`.
- **Slug.** Segue a regra da linguagem (`FLX13`): minúsculas, números e `_`, começando por letra, de 2 a
  64 caracteres. Repetido na mesma org é recusado (`Validacao` no campo `slug`); em outra org é aceito.
- **Exclusão (`excluir(org, fluxo, force)`).** Devolve quantas execuções **ativas** o fluxo tinha.
  - Sem execução ativa: exclui e devolve `0`.
  - Com execução ativa e `force = false`: recusa (`Conflito`, 409) com a contagem na mensagem; nada muda.
  - Com `force = true`: cada execução ativa vira `abandonada` com `motivo_fim = fluxo_excluido` e
    `finalizada_em` agora, e ganha o evento `execucao_abandonada` na linha do tempo da Ficha (ator
    `sistema`). Revisões e permissões saem junto (cascata). **Execuções e Fichas ficam**: o histórico do
    atendimento não tem chave estrangeira para o fluxo.
  - Tudo numa escrita só: ou o fluxo sai com as execuções abandonadas, ou nada muda.
- **Permissão.** `release`, `revoke`, `list_from_agent` (fluxos liberados, por slug),
  `list_agents_of_flow` e `replace_list`.
  - **Teto de 20 fluxos por agente** (`CAP_FLOWS_BY_AGENT`, PRD §15). A liberação do 21º é recusada
    com `TetoDeFluxosPorAgente` (código de API `teto_de_fluxos_por_agente`, 409). Repetir a liberação de
    um fluxo que já está liberado não conta de novo.
  - **Troca em lote (`substituir_lista(org, agente, esperada, fluxos)`).** Numa escrita só: confere a
    versão do **agente** (`If-Match`), o teto, que cada fluxo é da org, e troca a lista inteira. Se
    qualquer passo falha, a lista antiga e a versão do agente ficam como estavam. Fluxo repetido conta uma
    vez; lista vazia é permitida (tira todas as permissões).
  - **Org.** Fluxo ou agente de outra org (ou inexistente) é recusado com `NaoEncontrado`, não com erro de
    chave estrangeira.
  - **Versão do agente.** `release`, `revoke` (quando tira algo) e `replace_list` avançam
    `agents.version` e `agents.updated_at`. Assim quem leu a lista com a versão antiga leva 412 no `If-Match`
    seguinte e não sobrescreve a mudança de outra pessoa. Decisão desta entrega: a permissão faz parte da
    configuração do agente para efeito de `If-Match`.
- **Apagar agente** leva só a linha de `agente_fluxos`; o fluxo e as revisões são da organização.
- **Isolamento.** Todas as leituras e escritas recebem `org_id`. O teste
  `no_repository_reads_record_of_other_org` (`storage.rs`) cobre fluxos, permissão e exclusão.

O teto é uma constante do repositório. A configuração `fluxos.max_fluxos_por_org` do PRD §9 (100 fluxos por
org) **não** foi implementada nesta entrega.

### Carrinho

| Tabela | Colunas próprias |
|---|---|
| `carts` | `conversation_id`, `status`; índice único parcial garante **um carrinho aberto por conversa** |
| `cart_items` | `cart_id`, `product_id`, `quantity` (> 0), `unit_price_cents` |

### Entrada, saída, turnos e auditoria

| Tabela | Colunas próprias |
|---|---|
| `inbox` | `integration_id`, `external_message_id`, `payload_json`, `received_at`, `processed_at`, `attempts`, `last_error` |
| `outbox` | `conversation_id`, `turn_id`, `seq`, `kind` (`text`/`file`/`presence`), `payload_json`, `state` (`pending`/`sending`/`sent`/`failed`), `attempts`, `last_error`, `sent_at`, `external_message_id` |
| `turns` | `conversation_id`, `agent_id`, `state` (`running`/`done`/`failed`/`skipped`), `jev_json`, `retrieved_json`, `tools_json`, `timings_json`, `input_tokens`, `output_tokens`, `cost_micros`, `error`, `started_at`, `finished_at` |
| `audit_log` | `org_id` nulo = ação global, `actor_kind` (`admin_token`/`system`/`plugin`/`cli`/`matrix_user`), `actor_id`, `action`, `entity`, `entity_id`, `detail_json` |
| `schema_migrations` | `version` (chave primária), `name`, `applied_at` |

Chaves únicas de idempotência (PRD §6):
- entrada: `ux_inbox_external` em `(integration_id, external_message_id)`;
- saída: `ux_outbox_turn_seq` em `(turn_id, seq)`.

**Migração `0011` (MX-40): `outbox.external_message_id`.** Coluna de texto, nulável e sem valor
padrão, aditiva: itens já enviados ficam com `NULL` (o id deles nunca foi guardado).

- **O que guarda:** o id que o canal deu à mensagem enviada — na Evolution, o `key.id` da resposta
  do `sendText`/`sendMedia`. É o que a fase 2 usa para reconhecer o eco do próprio envio (`fromMe`
  com este id = eco, descartar) e distinguir a mensagem digitada no aparelho.
- **Quem grava:** a saída do WhatsApp (`crates/poph-server/src/saida_whatsapp.rs`) ao marcar o item como `sent`, na mesma
  escrita da confirmação (`confirm_send_with_id`). Envio que falha não deixa id: a coluna só é
  escrita quando o canal confirma.
- **Canal sem id:** plugin em processo externo pode não devolver id, e a Evolution pode responder
  sem `key.id`. Aí a coluna fica `NULL`, e isso **não** é falha de envio: o contrato do canal
  (`FuturoComando`, em `poph-types`) não carrega id; quem precisa dele usa
  `WhatsAppEvolution::send_with_receipt` (`ReceiptFuture`).
- **Leitura:** `caixa_saida().id_externo(org, id)`. Fica fora do formato de `ItemSaida` de
  propósito — o formato do item é usado por outros crates e a coluna é detalhe do canal.
- **Backup:** a coluna viaja com `outbox` no `--com-conversas`; arquivo antigo, sem ela, importa e
  deixa o id nulo.

## 5. A tabela `agents` em detalhe

Os campos são os que a HR-03 marcou `V1`, mais os que o PRD §7.2 pede e hoje são globais no POP
Atende (e passam a ser por agente no Harness).

| Coluna | Padrão | Observação |
|---|---|---|
| `name`, `slug`, `description`, `status`, `agent_type`, `category` | — | identidade e perfil; `slug` único por org; `agent_type` `custom`/`standard` e `category` são paridade (sem efeito no motor) |
| `default_department_id` | nulo | departamento das conversas novas |
| `human_handoff_enabled` / `human_handoff_department_id` | `0` / nulo | gates G1 e G4 |
| `provider`, `model`, `temperature`, `max_output_tokens` | `antigravity2api`, `gemini-3.8-flash-tiered`, nulo, nulo | **vale por agente** no Harness (PRD D6); nulo = padrão da configuração |
| `max_context_messages` | 30 | mantido **por paridade/importador**; o motor orça por tokens |
| `max_context_tokens` | 64.000 | orçamento de contexto |
| `grouping_window_ms` | 2.000 (0 a 8.000) | janela de agrupamento (PRD §6 passo 3) |
| `max_tool_rounds` | 4 | rodadas de ferramenta por turno |
| `max_tool_calls_per_message` | 10 | chamadas de ferramenta por mensagem |
| `rag_top_k` / `rag_min_score` | 3 / 0.0 | top-k e corte de relevância (novos no Harness, PRD §9) |
| `jev_json` | `{}` | flags e limiares do Jev deste agente; vazio = herda a configuração |

Decisões registradas:
- **`product_category_ids` virou tabela de vínculo** (`agent_product_categories`), em vez de lista
  dentro de `agents`. O conteúdo do contrato é o mesmo e o vínculo ganha integridade referencial.
- **Mudança de campo `Option` = "não mexer".** O `update` usa `COALESCE`: passar `None` mantém o
  valor atual; não há como **limpar** um departamento por esse caminho. Limpar exige método próprio
  (a HR-21 decide, quando a API precisar).
- **Regras de rota por intenção não existem** (decisão D-06): sem tabela, sem coluna, sem rota.

## 6. Isolamento entre orgs

Todo repositório recebe `org_id` e filtra por ele — não existe método de negócio sem org na
assinatura. O teste `no_repository_reads_record_of_other_org` cria dados na org A e consulta
tudo com a org B, repositório por repositório (categorias, departamentos, agentes, categorias do
agente, instruções, conversas, mensagens, turnos, entrada, saída, tokens e auditoria).

**Leituras sem org, de propósito (duas):**
- `Tokens::por_hash` — passo de autenticação, que ainda não sabe a qual org o token pertence. Ela
  devolve só token não revogado.
- `Matrix::orgs_ligadas` — passo de descoberta do laço de espelhamento, que também ainda não sabe
  qual org olhar. Ela devolve só org, corte do backfill e carimbos: nada de conversa nem cliente.

## 7. Repositórios desta entrega

Todos com `org_id` na assinatura, `create`/`get`/`list` e `update` com `expected` (versão) nas
editáveis:

| Repositório | Operações |
|---|---|
| `orgs()` | `create`, `get`, `by_slug`, `list`, `update`, `delete` (devolve quantos agentes havia), `delete_forced` |
| `tokens()` | `create`, `get` (org), `by_hash`, `by_id`, `list`, `revoke` |
| `categories()` | `create`, `get`, `list`, `update`, `delete` |
| `departments()` | `create`, `get`, `by_name`, `list`, `update`, `delete` |
| `agents()` | `create`, `get`, `by_slug`, `version`, `list`, `names`, `count_active_by_org`, `get_for_turn`, `list_active` (D-05), `update`, `delete`, `define_categories`, `categories` |
| `instructions()` | `get`, `version`, `save` (`expected = 0` cria), `list`, `delete` |
| `conversations()` | `create`, `get`, `list`, `list_filter`, `list_recents`, `list_by_agent_channel`, `change_status` (reabrir/transferir/fechar), `touch`, `define_summary`, `agent`, `changed_since`, `set_customer_name`, `reopen_audited`, `close_audited`, `close_by_agent`, `transfer`, `transfer_audited`, `identifier_client`, `normalise_name_client`, `listing_plan` |
| `messages()` | `write` (idempotente pelo id externo), `write_with_attachments`, `write_with_output`, `list`, `last_set`, `get`, `had_messages_before`, `last_rowid`, `count_rows`, `since`, `register_response_human` (MX-04: mensagem, saída, passagem e auditoria numa escrita só). Não move o marcador da conversa |
| `inbox()` | `receive` (Novo/Duplicado), `mark_processed`, `receive_with_message`, `count_pending`, `list_pending`, `list_open`, `defer`, `attempts`, `get` |
| `outbox()` | `enqueue` (idempotente por turno+seq), `change_state`, `start_send`, `confirm_send`, `confirm_send_with_id`, `fail_send`, `count_by_state`, `list_pending`, `external_id`, `sent_with_external_id`, `list_files_of_conversation`, `get`, `delivery_received`, `receive_from_channel`, `receive_from_device`, `attachments` |
| `turns()` | `start`, `finalise`, `fail_orphans`, `get`, `in_progress`, `list` |
| `audit()` | `register`, `list` |
| `products()` | `create`, `get`, `by_slug`, `list`, `list_active`, `update`, `delete`, `references`, `categories_visible`, `agent_visible`, `for_index`, `active_by_ids`, `active_of_category`, `category_name`, `index_text` |
| `knowledge()` | `create_group`, `get_group`, `list_groups`, `update_group`, `delete_group`, `list_items_by_id`, `create_item`, `get_item`, `list_items`, `update_item`, `delete_item`, `link_agent`, `unlink_agent`, `agent_groups`, `agents_from_group`, `agent_items`, `group_name_of_item` |
| `library()` | `create`, `get`, `list`, `count_rows`, `delete`, `delete_audited` |
| `carts()` | `summary`, `add`, `remove`, `clean` |
| `cases()` | `create`, `get`, `by_conversation`, `list`, `events`, `close_without_conversation`, `register_event` |
| `flows()` | `create`, `get`, `by_slug`, `list`, `count_rows`, `update`, `delete`, `release`, `revoke`, `list_from_agent`, `list_agents_of_flow`, `replace_list` |
| `revisions()` | `publish_revision`, `get`, `get_by_id`, `current`, `list` |
| `executions()` | `start_execution`, `apply_transition`, `transition_already_applied`, `opening_already_done`, `get`, `active_of_case`, `from_case`, `active_by_revision`, `stages`, `effects` |
| `effects()` | `orgs_with_pending`, `list_pending`, `get`, `start_send`, `confirm_send`, `fail_send`, `defer` |
| `matrix()` | `link`, `unlink`, `state`, `linked_orgs`, `ensure_room`, `mark_room_ready`, `mark_room_ready_with_name`, `set_mirrored_customer_name`, `room`, `rooms_ready`, `room_by_room_id`, `define_last_status`, `set_space_of_room`, `ensure_space`, `space`, `list_spaces`, `disable_space`, `read_cursors`, `delay_of_messages`, `advance_messages`, `advance_messages_with_id`, `rewind_cursor_messages`, `advance_conversations`, `register_failure`, `count_failures`, `list_failures`, `remove_failure`, `human_response_failures`, `record_notice_outcome` |
| `matrix_asset()` | `register`, `pending`, `mark_draft`, `mark_deleted`, `defer`, `org_pending` |
| `retention()` | `scan_table` (org, corte, simulação) |
| `analytics()`, `conversation_analytics()`, `sla_analytics()`, `usage()` | leituras agregadas (ver §10 e as seções de análise) |

O crate não tem mais repositório pendente: todos os módulos acima estão em
`crates/poph-store/src/repos/`. `rag.db` (HR-16) fica em outro crate.

## 8. Ordem de subida

1. `poph migrate` (ou a primeira abertura do `Store`) cria `data/`, `harness.db` e `rag.db`.
2. Migra `harness.db` até a versão 1 e registra em `schema_migrations`.
3. Cria `rag.db` com a tabela de controle (versão 0).
4. `poph doctor` confere pasta, bancos e versões; avisa quando falta migração e falha quando o
   arquivo não é SQLite.

## 9. Limites conhecidos desta entrega

- `rag.db` sem tabelas (HR-16) — o `poph doctor` mostra versão 0, e é o esperado.
- Sem cifra de `secrets_enc` (HR-26): a coluna existe, o conteúdo cifrado entra lá.
- Sem retenção/limpeza automática (HR-51): as colunas de tempo já existem para a varredura.
- Sem migração do POP Atende (HR-52): o importador lê o Postgres e escreve por estes repositórios.
- `unnest`/lote em massa: o caminho otimizado é concorrência no mesmo escritor (o lote é dele); não
  existe API de "inserir N mensagens de uma vez" — se a HR-13/HR-23 precisar, é acrescentar um
  comando de lote, não mudar o desenho.
- ARM: o SQLite `bundled` é C; `cargo check` aarch64 só roda nos crates sem C nesta estação (D-02).
  O binário ARM é compilado na máquina ARM64 de testes (HR-56).

## 10. Produtos e o índice de produtos (HR-18)

Repositório: `store.products()` (`crates/poph-store/src/repos/products.rs`). As tabelas
`products`/`product_categories`/`agent_product_categories` já existiam da HR-04b — **nenhuma migração
nova**. Operações: `create`, `get` (org), `by_slug`, `list`, `list_active`, `update`
(`If-Match`), `delete`, `categories_visible`, `agent_visible`, `for_index`, `active_by_ids`,
`active_of_category` e `category_name`. Tudo escopado por `org_id`.

Regras do repositório que o esquema não expressa sozinho:

- `slug` é derivado do nome quando não vem; único por org (índice `ux_products_slug`).
- `preco_centavos >= 0` (o `CHECK` existe; o erro de validação diz o campo).
- `category_id` é conferido no repositório antes da gravação: categoria de outra org dá "não
  encontrado", não erro cru de SQLite.
- `None` em `MudancaProduto` = "não mexer" (mesma regra da HR-07).

No `rag.db`, os produtos ganharam tabelas próprias (`rag_produtos`, `rag_produtos_trechos`,
`rag_produtos_fts`), criadas pelo mesmo `CREATE TABLE IF NOT EXISTS` das tabelas da HR-16 — um
`rag.db` existente só ganha as tabelas na próxima abertura, sem migração e sem perder os vetores da
base de conhecimento. A limpeza por divergência de modelo também apaga as tabelas de produto.

Preço, unidade, condições, inclui e exclui **não** entram no índice nem no hash de origem: mudar
preço não reembeda nada e aparece na resposta seguinte (ver `docs/PRODUTOS.md` §1).

## 11. Módulo Matrix (MX-08)

Migração `0005_tabelas_matrix`, com as cinco tabelas da seção 8 do PRD do módulo. Repositório em
`poph-store` (`store.matrix()`, `crates/poph-store/src/repos/matrix.rs`) e **não** em
`poph-matrix`: o banco e as linhas são deste crate, `poph-matrix` é cliente HTTP e continua
dependendo só de `poph-types`, e o `poph-server` (que importa os dois) liga uma coisa na outra.
A tabela de dependências do `AGENTS.md` não ganha aresta nova.

| Tabela | O que guarda |
|---|---|
| `matrix_orgs` | `org_id` (chave primária), `enabled`, `enabled_at` (corte do backfill), `updated_at` |
| `matrix_rooms` | `conversation_id` (chave primária), `org_id`, `room_id` (único), `space_room_id`, `last_status`, `client_name_espelhado` (§11.7), `state` (`creating`/`ready`), carimbos |
| `matrix_spaces` | `id`, `org_id`, `department_id`, `space_room_id`, `alias`, `origin` (`auto`/`linked`), carimbos |
| `matrix_estado` | `org_id` (chave primária), `messages_cursor`, `messages_cursor_id`, `conversations_cursor_at`, `conversations_cursor_id`, `version`, `updated_at` |
| `matrix_falhas` | `id`, `org_id`, `message_id`, `motivo` (código curto, até 64 caracteres), `tentativas`, carimbos |

Regras que o esquema e o repositório garantem:

- **Toda linha tem `org_id`** e apagar a org apaga as linhas do módulo (`ON DELETE CASCADE`); a sala
  aponta para a conversa por chave composta `(org_id, conversation_id)`.
- **Nenhuma coluna guarda texto de cliente, telefone ou token** — a única exceção, de propósito, é
  `matrix_rooms.client_name_espelhado` (§11.7), que guarda o nome de exibição já enviado à sala.
  Coluna nova nessas tabelas passa pelo teste
  `no_column_of_module_tables_stores_customer_text_phone_or_token`
  (`crates/poph-store/tests/matrix.rs`), que compara a lista de colunas com a revisada.
- **`matrix_falhas.message_id` sem chave estrangeira**, de propósito: o índice único extra em
  `messages` derrubava a vazão de escrita (§2). A org da mensagem é conferida pelo repositório.
- **Um espaço por departamento** e um espaço/alias por org não serve a dois departamentos; a
  unicidade é por org porque duas orgs do mesmo Synapse podem ligar o mesmo espaço (fase 3).
  `matrix_rooms.room_id` é único **global**: uma sala pertence a uma conversa só.
- **Cursor de mensagens** é o `rowid` da última mensagem espelhada (o mesmo de `Messages::since`).
  **Cursor de conversas** é o par (`updated_at`, `id`) — o id desempata quando duas conversas mudam
  no mesmo milissegundo. Os dois só andam para a frente: repetir um lote não muda nada.
- **Backup**: as tabelas `matrix_*` não viajam no backup de org (ver `BACKUP.md`).

### 11.1 A porta da provisão de sala (MX-11)

A provisão da sala (`crates/poph-matrix/src/salas.rs`) não escreve SQL nem fala com o banco: ela
recebe a persistência pela porta `PortaSalas`, cuja implementação em produção é o `store.matrix()`
deste crate, ligada pelo `poph-server`. A porta pede quatro coisas, e é o que a implementação tem
de garantir:

| Operação da porta | O que a implementação faz |
|---|---|
| `room` | lê a linha da conversa (`room` do repositório) |
| `ensure_room` | grava em `creating` e devolve a linha que já existia (o primeiro `room_id` ganha; sala de outra conversa é conflito) |
| `marcar_pronta` | `creating` → `ready` |
| `had_messages_before` | existe mensagem da conversa anterior ao corte `enabled_at`? (decide o aviso de histórico não espelhado) |

Assim a decisão de criar a sala fica no módulo (que precisa dela para o teste de queda injetada) e
o banco continua sendo responsabilidade deste crate — a tabela de dependências do `AGENTS.md` não
ganha aresta nova.

**Poder na criação.** O `createRoom` manda o fantasma do agente como criador com poder **100**
(`PODER_CRIACAO`): é o que o Synapse exige para o criador escrever o próprio `m.room.power_levels`
na criação. Terminada a sala (nome, tópico, metadata, participantes e aviso), a criação lê o
`m.room.power_levels`, troca **só** o poder do agente para `matrix.poder_agente` respeitando o piso
de 50 (`PODER_MINIMO_AGENTE`, o mínimo para o módulo continuar reescrevendo metadata e nome) e
reescreve o evento preservando os demais campos. Se o rebaixamento falhar, a sala continua
utilizável com o poder da criação e o log leva `rebaixamento_de_poder_falhou` (não há nova
tentativa). Com o criador abaixo de 100 o Synapse responde `403 M_FORBIDDEN`
(`user_level (50) < send_level (100)`) e deixa a sala criada pela metade — foi o defeito que derrubou
o A1 do aceite (MX-31); o Synapse falso dos testes passou a validar essa regra.

Operações do repositório (todas com `org_id`, exceto a descoberta `linked_orgs`):

| Operação | Comportamento |
|---|---|
| `ligar` / `desligar` / `estado` | idempotentes; repetir não falha. Religar grava um `enabled_at` novo (o backfill não volta no tempo); desligar preserva o corte |
| `ensure_room` | idempotente por conversa: devolve a sala que existe; sala já ligada a outra conversa é conflito |
| `mark_room_ready` | `creating` → `ready`; já pronta não é reescrita |
| `room` / `room_by_room_id` | leitura por conversa ou pelo id do Synapse |
| `define_last_status` | grava a última situação da conversa espelhada |
| `ensure_space` / `espaco` / `list_spaces` | idempotente por departamento; espaço de outro departamento é conflito |
| `disable_space` | apaga a linha de `matrix_spaces` do departamento e zera `space_room_id` das salas daquele espaço (§11.4); nada é apagado no Synapse; repetir devolve `removido = false` |
| `set_space_of_room` | grava (ou tira, com `None`) o `space_room_id` da sala; repetir o mesmo espaço não reescreve |
| `read_cursors` / `advance_messages` / `advance_conversations` | cursores monotônicos; org sem linha lê zeros |
| `delay_of_messages` | conta as mensagens da org depois do cursor (mesmo filtro do laço: `user`/`assistant`) e devolve a criação da mais antiga — é o "atraso" da API da MX-22 |
| `register_failure` / `count_failures` / `list_failures` / `remove_failure` | repetir a falha da mesma mensagem atualiza a linha em vez de duplicar |

### 11.2 Leitura incremental (MX-05)

Duas leituras para o espelhamento, ambas no **leitor** do pool (nunca no escritor), para que o
turno não dispute com elas:

| Consulta | Chave e ordem | Filtro | Índice |
|---|---|---|---|
| `Mensagens::desde(apos, limite)` | `rowid` crescente, depois de `apos` | só `role` `user` e `assistant` (`tool` e `system` nunca entram) | o `rowid` da própria tabela |
| `Conversas::alteradas_desde(apos, limite)` | `(updated_at, id)` crescente, depois do par `apos` (`None` começa do início) | nenhum (leitura global) | `ix_conversations_atualizada (updated_at, id)` |

- `since` devolve `MessageSince`: `rowid` (o cursor), `id` (de onde o `txnId` sai, MX-13), org, conversa,
  direção, papel, origem, autor, texto, anexos, id externo e `created_at`. Teto de 1000 linhas por página.
- `alteradas_desde` devolve a `Conversa` completa. O id desempata quando duas conversas mudam no mesmo
  milissegundo, então nenhuma some entre páginas. Teto de 1000.
- As duas são globais, sem filtro de org: cada linha traz `org_id`, e quem espelha filtra. Os cursores
  de `matrix_estado` são por org (ver §11), então o laço (MX-14) decide como usá-los.

**Migração `0006` (MX-05): `ix_conversations_atualizada (updated_at, id)`.** Aditiva, só cria o índice.
Motivo: sem ele, `alteradas_desde` faz `SCAN conversations` e ordena num temporário a cada página
(plano medido antes do índice). Com ele, o plano vira `SEARCH conversations USING INDEX
ix_conversations_atualizada`. O índice mais simples `(org_id, updated_at)`, sugerido na tarefa, não
serve à consulta global, por isso a chave é `(updated_at, id)`.

**Cursor por `rowid` e reaproveitamento (resolvido na MX-14).** O `rowid` pode ser reaproveitado quando a
linha de maior `rowid` é apagada (retenção ou apagar conversa): a mensagem seguinte recebe o mesmo número e,
sem mais nada, ficaria abaixo do cursor. Por isso `matrix_estado` guarda também o **id** da mensagem da
posição do cursor (`messages_cursor_id`, migração `0007`). Regra adotada pelo laço (ver §11.3): linha na
posição do cursor com o mesmo id já foi espelhada e é pulada; com outro id é mensagem nova e é espelhada.

### 11.3 Espelho de mensagens (MX-14)

O laço (`crates/poph-server/src/matrix_espelho.rs`, `EspelhoMatrix`) lê `messages` pela leitura de §11.2 e
entrega ao Synapse pelo destino injetado (`DestinoMatrix`). Regras que o código e os testes garantem:

- **Cursor por org, leitura global.** Cada org ligada tem a sua linha em `matrix_estado`. O laço lê uma vez
  a partir do **menor** cursor entre as orgs ligadas e filtra por org depois de ler: uma consulta serve a
  todas, e cada org só processa linhas dela, posteriores ao próprio cursor. Orgs desligadas não geram
  nenhuma chamada.
- **Marca-d'água contígua.** O cursor de todas as orgs ligadas anda só até a primeira linha não resolvida.
  Resolvida = enviada, pulada por regra (resposta humana, texto vazio, ferramenta), isolada em
  `matrix_falhas`, ou já espelhada. Nada é pulado: o que ficou pendente volta no ciclo seguinte, e o
  `txnId` (estável por mensagem) impede duplicata no Synapse.
- **Corte do backfill.** Mensagem com `created_at` anterior a `enabled_at` da org não é espelhada (P8).
- **Falha transitória** (rede, 5xx, 429, prazo): fica pendente, o ciclo para naquele ponto e o próximo
  espera um atraso que dobra a cada ciclo com falha, até 60 s.
- **Falha definitiva** (4xx do Synapse): a tentativa é contada em memória por mensagem. Até 3 tentativas
  fica pendente; na terceira grava `matrix_falhas` com o código curto do erro (ex.: `M_FORBIDDEN`) e a fila
  segue. Sem código, `envio_rejeitado`; falha da sala, `sala_rejeitada`.
- **Entrega em paralelo entre salas**, até `matrix.concorrencia`; dentro de uma sala, em ordem.
- **Memória limitada.** Lê em páginas de 200 linhas e no máximo 25 páginas por ciclo. Medido: 50 mil
  mensagens pendentes com o Synapse fora do ar; o ciclo lê só a primeira página e para (~0,7 ms em
  release; teste `memoria_com_50_mil_pendentes`, `--ignored`).
- **R5 (cursor à frente).** Se o cursor de uma org passa do maior `rowid` que existe (banco restaurado ou
  linhas apagadas), o cursor volta para essa última linha antes de ler.
- **Conversa sem `peer_externo`** (ex.: canal de console): o cliente da sala usa o id da conversa como
  identificador. Decisão local da MX-14, para não gerar falha em todas as mensagens desse canal.
- **Log:** só ids, contadores e código de erro. Nenhum texto de mensagem.

Ainda não ligado no `poph serve`: quem liga é a MX-20.

### 11.4 Espaços por departamento (MX-12)

No Element o atendente vê o trabalho organizado por departamento: cada departamento é um **espaço** do
Matrix e a sala de cada conversa fica dentro do espaço do departamento que a atende. O serviço
(`crates/poph-matrix/src/espacos.rs`, `Espacos`) decide e executa; o banco entra pela porta
`PortaEspacos`, e é o `poph-server` quem a implementa sobre `store.matrix()` (mesma fronteira da
`PortaSalas`, §11.1).

| Operação da porta | O que a implementação faz |
|---|---|
| `espaco` | lê a linha do departamento em `matrix_spaces` |
| `ensure_space` | grava espaço + alias + origem (`auto`/`linked`) e devolve a linha que já existia |
| `espaco_da_sala` | lê `matrix_rooms.space_room_id` |
| `set_space_of_room` | grava (ou tira) `matrix_rooms.space_room_id` |

Regras que o serviço garante:

- **Departamento (P3):** `target_department_id` da conversa; senão `default_department_id` do agente;
  senão nenhum. Sem departamento a sala não entra em espaço nenhum, não é erro e **nenhuma** chamada de
  espaço é feita ao Synapse.
- **Espaço criado pelo módulo (`origin = auto`):** alias determinístico
  `#dep_<id do departamento sem hífen>:<servidor>`, `creation_content.type = m.space`, nome = nome do
  departamento, tópico curto, `com.sofacoding.departamento` com o id do departamento. O alias é a trava:
  repetir cai em `M_ROOM_IN_USE`, resolve o alias e continua no **mesmo** espaço (o mesmo dois-passos do
  P6, comprovado na MX-02). O alias `#dep_.*` é compartilhado com o registro do Application Service do
  Python (não é exclusivo de ninguém).
- **Espaço que já existe no Element (`origin = linked`):** o operador informa o id (`!…`) ou o alias
  (`#…`). Antes de usar, o módulo confere que o **bot do Application Service** (`@<prefixo>_as_bot`) é
  membro do espaço e tem poder para escrever `m.space.child` (o `state_default` do espaço, 50 no padrão).
  Faltando qualquer um dos dois, o erro diz o que fazer no Element — convidar o bot e dar o poder — e
  **nada é gravado**. Essa falta de permissão é classificada como **transitória** de propósito: quando o
  operador corrige, a sala entra no ciclo seguinte; tratá-la como definitiva faria a sala ser isolada e
  nunca mais tentada.
- **Quem escreve o quê:** o espaço é do bot (dono único por org, independente do agente da conversa) e é
  ele que escreve `m.space.child`; a sala é do fantasma do agente (MX-11), então é ele que escreve
  `m.space.parent` e a regra de entrada.
- **Sala dentro do espaço:** `m.space.child` no espaço (`state_key` = sala, corpo com `via`),
  `m.space.parent` na sala (`state_key` = espaço, `via` + `canonical: true`) e
  `m.room.join_rules = restricted` com `allow` apontando para o espaço.
- **Convites:** os membros do espaço que não são o bot nem fantasmas desta instância são convidados para a
  sala, em lotes (`LimitesConvite`: 20 por lote, 4 simultâneos, pausa de 100 ms, teto de 200 por passagem —
  todos parâmetros de código, sem chave nova no `poph.toml`). Quem já está na sala não é convidado de novo.
- **Convite pendente não grava o vínculo:** `set_space_of_room` só é chamado quando **todos** os
  convites da passagem foram entregues. Com um convite falho, a passagem devolve `Posicionada` com
  `convites_falhos > 0` e **não** grava `space_room_id` — se gravasse, a passagem seguinte sairia por
  `JaPosicionada` e o membro que ficou de fora nunca mais seria convidado (defeito achado na MX-29). A
  passagem seguinte refaz o caminho (as escritas no Synapse são idempotentes) e convida só quem ainda não
  está na sala; o laço de estado registra `convite_pendente_no_espaco` no log. Nada é repetido dentro da
  mesma chamada: a nova tentativa é a passagem seguinte.
- **Mudança de departamento (transferência):** o mesmo caminho põe a sala no espaço novo primeiro
  (`m.space.child`, `m.space.parent`, regra de entrada, convites) e só depois tira do antigo
  (`m.space.child` com conteúdo vazio e `m.space.parent` vazio). Uma queda no meio deixa a sala visível em
  dois espaços — fácil de corrigir — em vez de sumir de todos.
- **Sem departamento depois de ter espaço:** a sala sai do espaço (`Removida`).
- **Idempotência:** a sala já gravada no espaço do departamento não gera **nenhuma** requisição; o espaço
  já gravado não é recriado nem revalidado.
- **Quem chama `posicionar`:** o laço de estado (MX-15) a cada conversa alterada **e** o espelho de
  mensagens (`matrix_espelho`, MX-14) logo depois de criar a sala (MX-31, item 2). O segundo caminho
  existe porque o laço de estado pode ter visto a conversa antes de a sala existir: sem ele, a sala
  criada depois só entrava no espaço na próxima alteração da conversa. Falha do posicionamento na
  criação não derruba a criação da sala (fica no log, com org, conversa e código curto; o laço de
  estado tenta de novo na próxima alteração).
- **Sem dado de cliente:** os eventos levam id de departamento, nome de departamento e ids de sala; nada
  de telefone, texto de conversa ou token.

### 11.5 Estado da conversa (MX-15)

O serviço de estado (`crates/poph-server/src/matrix_estado.rs`, `EstadoDasConversas`) percorre
`conversations` pela leitura incremental de §11.2 e compara a situação de cada conversa com o que a sala já
espelhou (`matrix_rooms.last_status`):

- **Cursor de conversas por org**, no par (`conversations_cursor_at`, `conversations_cursor_id`) de
  `matrix_estado`: o mesmo par da leitura incremental, que desempata duas conversas alteradas no mesmo
  milissegundo. Ele só anda **depois** de a passagem dar certo para aquelas conversas: falha transitória
  (banco ou Synapse) deixa o cursor atrás da conversa pendente e a passagem seguinte tenta de novo; falha
  definitiva (4xx fora 429) é registrada com o código curto e a conversa é pulada, para a fila não travar.
- **Marca-d'água por org.** A leitura é global (uma consulta serve a todas as orgs ligadas); cada org
  filtra o que é dela pelo próprio par, e o cursor da org para na primeira conversa pendente dela.
- **O que fica gravado** é `matrix_rooms.last_status` (a situação já espelhada) e o metadata da sala. Nada
  de tabela nova: a spec da MX-15 não prevê tabela de falhas do estado, e o código não inventa uma.
- **`last_status` vazio** (sala recém-criada pela MX-11) é lido como conversa aberta — o estado em que toda
  conversa nasce —, então a primeira transferência já gera aviso.
- **Sem dado de cliente:** o aviso é texto fixo por situação; o log leva org, conversa, situação e o código
  do erro.

### 11.6 Aviso de falha da resposta humana (MX-19)

Quando a resposta do atendente não sai pelo WhatsApp de vez, o módulo escreve um aviso na sala (serviço
`crates/poph-server/src/matrix_falhas_aviso.rs`, `AvisoDeFalhas`). Os dados que ele usa:

- **Nada de tabela de fila nova.** A falha é lida da `outbox`: item em `failed` cuja mensagem tem o mesmo
  `turn_id` e `origin = 'human'`. O aviso não mexe em `outbox`.
- **Índice parcial `ix_messages_turno_humano`** (`org_id, turn_id`, `WHERE origin = 'human'`), na migração
  `0009`. É parcial de propósito: só as respostas humanas entram nele, e as demais mensagens não pagam custo de
  escrita a mais (ver §2).
- **Marca `matrix_avisos_falha`** (chave `org_id, outbox_id`): uma linha por item de saída que já teve
  desfecho na sala. Colunas: `desfecho` (`avisado`, `sem_sala` ou `recusado`), `motivo` (código curto de até
  64 caracteres, por exemplo `recusado_pelo_canal` ou `tentativas_esgotadas`) e `registrado_em`. Não guarda o
  erro bruto da fila, que pode trazer dado do canal.
- **Idempotência que sobrevive a reinício.** A marca é a trava: depois de gravada, a falha não volta na
  listagem, então a passagem seguinte nem chega ao Synapse. A linha é gravada **depois** de o Synapse aceitar
  o aviso, e o `txnId` do aviso é derivado do item de saída, então, se o processo morrer no meio, a repetição
  usa o mesmo `txnId` e o Synapse não duplica.
- **Só a org com o módulo ligado.** A listagem percorre as orgs de `matrix_orgs` com `enabled = 1`. O
  repositório também confere que o item de saída é da org (`INSERT … SELECT … FROM outbox WHERE id AND org_id`):
  uma org não grava desfecho para item de outra.
- **Motivo de falha.** A fila grava `tentativas de envio esgotadas` (constante `MOTIVO_TENTATIVAS_ESGOTADAS`,
  em `saida_whatsapp.rs`) ou o erro definitivo do canal. O aviso traduz para um de três códigos fixos
  (`tentativas_esgotadas`, `recusado_pelo_canal`, `nao_informado`); o texto do canal não entra na sala nem no
  log.

Regra de retomada: falha transitória (item em `sending`) não aparece na listagem. Erro transitório do
Synapse no momento do aviso também não marca nada: a passagem seguinte tenta de novo. Erro definitivo do
Synapse marca `recusado` e não é repetido.

As tabelas `matrix_*` (incluindo `matrix_avisos_falha`) continuam fora do backup de org (ver `BACKUP.md`).

Provas: `crates/poph-server/tests/matrix_falhas_aviso.rs` (falha definitiva com um aviso só, reinício do
processo sem repetir, transitória seguida de sucesso sem aviso, Synapse fora adia e a passagem seguinte avisa,
tentativas esgotadas com motivo próprio, conversa sem sala, saída que não é humana, org desligada, canário de
texto, telefone e token, e o laço de fundo que para no pedido).

### 11.7 Nome do cliente na sala já criada (MX-33)

O nome do cliente vem do WhatsApp (`conversations.client_name`, MX-32) e pode chegar **depois** de a sala
existir — a sala nasce com o identificador. Duas peças guardam o que já foi feito:

- **`matrix_rooms.client_name_espelhado`** (migração `0010`, nulável): último nome já enviado ao Matrix para
  aquela sala — o nome da sala, o `client_name` do metadata e o nome de exibição do fantasma do cliente.
  `NULL` quer dizer "esta sala ainda não recebeu nome": é o estado da sala criada antes da migração e o da
  sala cujo cliente ainda não tem `pushName`. A coluna é a **única** das tabelas do módulo com dado de
  cliente, e de propósito: sem ela o laço renomearia a sala em toda passagem.
- **`matrix_rooms` continua fora do backup de org** (ver `BACKUP.md`). O nome em si viaja com
  `conversations`; o que fica no módulo é só a marca do que já foi espelhado no Synapse daquele ambiente.

Regras:

- **MX-11 grava o nome que a sala recebeu** junto com o `ready` (`mark_room_ready_with_name`, uma escrita
  só): o `client_name` do metadata, ou `NULL` quando a sala nasceu com o identificador.
- **MX-15 renomeia** quando a conversa tem nome não vazio diferente do espelhado, nesta ordem: (a)
  `m.room.name`; (b) metadata com o estado **atual** da conversa (a conversa é relida antes de escrever, para
  um nome atrasado nunca devolver uma conversa `closed` a `active`); (c) fantasma do cliente com o nome novo;
  (d) só então grava o espelhado. Cair no meio custa repetir as três chamadas — todas idempotentes —, nunca
  sala nem aviso duplicado.
- **Sem aviso (`m.notice`)**: trocar nome não é evento de atendimento.
- **O nome nunca volta a vazio**: conversa sem nome mantém o que a sala já tem. Não há varredura de
  backfill: a sala antiga (criada só com o telefone) se corrige na próxima alteração da conversa. Se um
  atendente renomeou a sala à mão, o próximo nome do cliente sobrescreve.
- **O histórico antigo da sala não muda**: as mensagens já enviadas ficam como foram; só o nome atual da
  sala, o metadata e o perfil do fantasma são atualizados.
- **O nome não vai para o log**: as linhas levam org, conversa e o código do erro.

## 12. Regras que os repositórios garantem (e o esquema não expressa)

Esta seção reúne as invariantes que estavam escritas como comentário dentro de
`crates/poph-store/src/repos/` e que continuam valendo. O esquema só tem o que dá para expressar em
`CHECK`, índice único e gatilho; o resto é conferido no código.

### 12.1 Regras comuns a todos os repositórios

- **`org_id` em toda assinatura de leitura e escrita.** Não existe método que consulte tabela de
  negócio sem o escopo da org; o teste `every_business_table_has_org_id` pergunta ao `sqlite_master`
  e o `no_repository_reads_record_of_other_org` cria dados na org A e consulta pela org B.
- **Escrita pelo escritor dedicado, leitura pelo pool.** Toda escrita passa por uma transação
  (`BEGIN IMMEDIATE`) do escritor; as funções internas que outros repositórios chamam **não** abrem
  transação própria — a atomicidade é a de quem chamou, e o `SAVEPOINT` volta inteiro no primeiro
  erro.
- **Entidade editável tem `version`**, exigida no `If-Match` (`expected`): versão velha devolve
  conflito (412) e não grava nada. `None` em campo de mudança significa "não mexer", nunca "zerar".
- **Nome publicado fica congelado:** tabela, coluna, índice, gatilho, rota, `operationId`, chave JSON,
  chave de configuração, métrica e termo de protocolo de plugin não mudam com o código interno.

### 12.2 Retenção (`retention().scan_table`)

- **O que sai**, sempre com a org varrida e data anterior ao corte: mensagens e os anexos delas;
  linhas de `outbox` já enviadas; linhas de `inbox` já processadas; turnos encerrados (fora
  `running`), sem saída pendente e sem mensagem que aponte para eles; conversas vazias (sem
  mensagem, turno, saída nem carrinho) criadas antes do corte.
- **O que fica:** pendente de `inbox` e de `outbox` (qualquer estado que não seja `sent`), turno
  `running` e qualquer conversa com algo que ainda não venceu. Base de conhecimento, produtos,
  biblioteca de arquivos, orgs, agentes, tokens, departamentos, categorias e `audit_log` nunca entram
  na varredura.
- **`simulate` desfaz no fim:** cada org varrida roda num `SAVEPOINT` próprio; com simulação ele é
  desfeito e as contagens devolvidas são as que sairiam de verdade. Erro de banco não deixa nada pela
  metade.
- Depois da varredura, o resultado lista os `sha256` que ainda têm dono (anexos vivos e biblioteca do
  agente) — é o que a limpeza de blob usa para não apagar arquivo em uso.

### 12.3 Fila de efeitos do fluxo (`effects()`)

| Operação | Transição |
|---|---|
| `list_pending` | lê o que está por entregar (e o que já venceu a espera) |
| `start_send` | `pending`/`sending` → `sending`, mais uma tentativa |
| `confirm_send` | → `sent`, com o carimbo de entrega |
| `fail_send` | transitória → `pending` com espera; definitiva → `failed` |
| `defer` | empurra a espera **sem** contar tentativa (falta integração) |

- **A tentativa é gravada antes da chamada externa** (mesma regra da fila de saída): se o processo
  morrer entre a marcação e a confirmação, a passagem seguinte encontra o efeito em `sending` e
  entrega de novo. É entrega **pelo menos uma vez**; duplicar nessa janela é aceito e documentado.
- O efeito repetido não é reenfileirado: o índice único `(org_id, execution_id, chave)` protege a
  chave do motor (`nó + índice`).
- **Nada de rede aqui:** quem entrega é o laço do servidor. O teto e a espera crescente das
  tentativas são de quem entrega; este repositório só grava o que ele decidir.

### 12.4 Transição de execução (`executions().apply_transition`)

Uma única escrita grava: `flow_executions` (`status`, nó atual, dados, resultado, motivo do fim,
`finished_at`, `version + 1` conferindo a versão esperada), a linha do tempo por etapa, um evento de
Ficha por evento do motor (com `seq` calculado na mesma escrita), os efeitos na fila durável e, quando
o fluxo pede encerramento, a conversa `closed` com a Ficha fechada na mesma transação.

- **Ordem das recusas:** idempotência **antes** da versão — repetir a chave de uma transição que já
  entrou devolve o mesmo desfecho, não 412; depois versão (412), execução que não está `ativa` (409) e
  Ficha fechada (409). Só então a escrita.
- A chave de idempotência é obrigatória (vazia é 400): sem ela, duas transições diferentes colidiriam
  no mesmo marcador e a segunda seria lida como repetição.
- O nó é gravado pelo **id**: quem reconstrói o destino tipado precisa da IR em mãos para saber se é
  etapa, decisão ou fim. Encaminhamento não é executado no banco: vai como efeito.
- Transição não é mensagem do cliente: não mexe em `conversations.last_message_at`.

### 12.5 Carrinho (`carts()`)

- **Um carrinho aberto por conversa.** Ele nasce na primeira adição; conversa nova começa vazia, e
  esvaziar mantém o carrinho aberto.
- **Preço é do registro, não do pedido:** quem chama passa o id do produto e a quantidade; o preço é
  lido do catálogo dentro da mesma escrita e gravado na linha (`unit_price_cents`). O total é a soma de
  quantidade × preço de cada linha.
- **Mesmo produto soma na mesma linha**, e o preço da linha passa a ser o do catálogo no momento da
  última adição.
- **Só entra produto ativo e visível ao agente** (a mesma regra da busca), e a **quantidade vai de 1 a
  999 por linha**; a soma acima de 999 é recusada sem gravar nada.

### 12.6 Biblioteca de arquivos (`library()`)

- É a **única origem de arquivo** que o agente pode enviar ao cliente: ele escolhe um id daqui, nunca
  um caminho de disco.
- **Sem `If-Match`**: a tabela não é editável; para trocar o conteúdo, o arquivo é excluído e enviado
  de novo (o blob é deduplicado por hash).
- **O conteúdo não passa por aqui:** `blob_path` é o endereço relativo à pasta de blobs, e quem grava o
  arquivo é outro crate. Apagar da biblioteca não apaga o blob; órfão é assunto da retenção.
- Toda leitura filtra **org e agente**: id de outro agente, na mesma org, devolve "não encontrado".

### 12.7 Tokens de administração (`tokens()`)

- Tokens **globais** vêm da configuração (só o hash); os de org ficam nesta tabela.
- **O valor do token nunca é gravado** — só o hash —, e ele não volta em nenhuma leitura.
- A leitura por id é **dentro da org**: token global não aparece por ali, só pela descoberta por hash.

### 12.8 Produtos e base de conhecimento

- **Visibilidade por categoria:** agente com categorias vinculadas só vê os produtos ativos delas;
  agente sem vínculo vê todos os produtos ativos da org. Inativo ou arquivado fica fora da busca, da
  ferramenta e da resposta, e continua listado e editável pela CLI/Console.
- **Conhecimento:** o conteúdo tem teto de 1000 caracteres (o `CHECK` existe; a validação no
  repositório é para o erro dizer qual campo e quantos caracteres vieram), e a leitura para o índice
  devolve **só item ativo de grupo ativo**.

### 12.9 Mídia publicada no Matrix (`matrix_asset()`)

- Gravar a mesma mídia duas vezes **não duplica**: o id é o id do anexo e a segunda gravação não muda
  nada (o `txnId` do envio é derivado dele).
- **Cada etapa tem carimbo próprio:** `redacted_at` e `deleted_at` são gravados separados, e a fila só
  devolve a linha enquanto faltar uma das duas — repetir a passagem não repete chamada.
- **A espera da falha é do item**, não da rotina: `attempts` e `next_attempt_at` tiram o item
  problemático da frente. Nada de dado de cliente: ficam o `mxc`, o tipo e o código curto do erro.
