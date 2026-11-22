# Repasse do Element ao Harness (ponte transitória)

Este documento descreve a ponte que deixa o app de chat (Element) enxergar conversas que existem só no
POP Harness. A ponte fica no backend Python, que já é o endereço que o Element chama. Ela é transitória: sai
junto com o backend Python (seção 10).

Estado: a ponte está implementada e foi aceita em ambiente de desenvolvimento, com os cinco cenários de
aceite no app. Em produção não há organização vinculada. Ligar uma organização real depende de
autorização e de pré-requisitos que estão fora deste documento.

## 1. O que é e por que existe

- O Element continua chamando a API do backend Python, com o mesmo endereço e a mesma sessão. Nada muda no app.
- Para uma organização **com vínculo ativo**, três chamadas passam a consultar o Harness: o detalhe da
  conversa, o histórico do cliente e o fechamento de atendimento. As conversas novas dessas organizações nascem
  no Harness.
- Para uma organização **sem vínculo** (ou com o vínculo desligado), nada muda: o Python responde como antes e
  não faz nenhuma chamada ao Harness.
- Por que o Python repassa, e não o Element falar direto com o Harness: o Element usa uma única base de API e uma
  única sessão para centenas de rotas que continuam no Python. O Harness não tem login de usuário. Falar direto
  exigiria duas bases, duas sessões e esse login.
- O token de organização do Harness é guardado pelo Python, uma vez por organização vinculada. O token do usuário
  nunca sai do Python.
- Esta ponte é transitória: some quando o backend Python for desligado (seção 10).

## 2. Quando cada caminho responde

| Pedido | Organização sem vínculo ativo | Organização com vínculo ativo |
| --- | --- | --- |
| Detalhe da conversa | Python. Se não existir, 404. | Python, se a conversa existir lá. Se não, consulta o Harness. |
| Histórico do cliente (`client_identifier`, sem `cursor` e sem `agent_id`) | Só Python. | Python e Harness juntos. |
| Histórico com `cursor` ou com `agent_id` | Só Python. | Só Python (o Harness não tem filtro por agente nem cursor nesta lista). |
| Fechar atendimento | Python. Se não existir, 404. | Python, se a conversa existir lá. Se não, age no Harness. |

Regras que valem para as três chamadas:

- **Vínculo ativo** quer dizer: a linha da organização existe, `enabled` é verdadeiro e a variável do token
  existe no processo com valor. Qualquer outra situação é tratada como organização sem vínculo.
- A organização usada no Harness vem **sempre** da linha de vínculo da organização de quem pediu. Nunca vem do
  pedido.
- Ids no formato do Python (começam com `cnv_`) não são enviados ao Harness. O Python responde 404 direto.

## 3. As três chamadas, na ordem exata

### 3.1 Detalhe: `GET /v1/conversations/{id}`

1. Confere a permissão de leitura de conversas.
2. Procura a conversa no Python, na organização de quem pediu. Se achar, responde e para aqui.
3. Não achou: se o id não tem formato de UUID, responde 404 sem consultar nada.
4. Lê o vínculo da organização. Sem vínculo ativo, responde 404.
5. Consulta o Harness: `GET …/conversations/{id}`. Repete uma vez em falha de rede ou de tempo esgotado.
6. Resposta do Harness:
   - 200: converte (seção 4) e responde com `engine = "harness"`;
   - 404: responde 404 `NOT_FOUND`;
   - qualquer outra falha: responde 503 `HARNESS_UNAVAILABLE`.

### 3.2 Histórico do cliente: `GET /v1/conversations?client_identifier=…`

1. Confere a permissão de leitura de conversas.
2. Só vai ao caminho do Harness se houver `client_identifier`, sem `cursor` e sem `agent_id`. Lê o vínculo.
   Sem vínculo ativo, responde só com o Python.
3. Com vínculo ativo, faz **em paralelo**:
   - a consulta no Python, com os mesmos filtros e `limit + 1`;
   - as consultas no Harness, com `external_peer` (o identificador do cliente, sem espaços nas pontas),
     `sort=recent` e `limit + 1`. Situação: sem filtro, uma consulta; `open`, duas (`open` e `transferred`);
     `closed`, uma. O canal não vai ao Harness: é filtrado depois, sobre o resultado.
4. Junta as duas origens, converte as do Harness (seção 4), ordena pela atividade mais recente (última mensagem;
   se não houver, a data de criação), com desempate por id decrescente, e corta em `limit`.
5. `meta.pagination.has_more` fica verdadeiro quando o total juntado passa de `limit`, ou quando alguma consulta
   ao Harness trouxe mais que `limit`. `next_cursor` sai sempre nulo.
6. Se o Harness falhar, a resposta traz só as conversas do Python e `meta.harness_indisponivel = true`. Não é erro.

Cada item da lista traz `engine`: `"python"` ou `"harness"`.

### 3.3 Fechar: `POST /v1/conversations/{id}/close`

1. Confere a permissão de escrita de conversas.
2. Procura a conversa no Python. Se achar, fecha no Python como antes. Registra `close`, ou `close_noop` se já
   estava fechada.
3. Não achou: se o id não é UUID, ou se a organização não tem vínculo ativo, responde 404.
4. Lê a conversa no Harness (`GET`) para saber a situação e a versão.
   - Já `closed`: não envia fechamento nenhum. Responde 200 com a conversa e registra `close_noop`.
5. Envia `POST …/close` com o cabeçalho `If-Match` igual à versão lida. Este POST **não** é repetido sozinho.
   - 200: responde com `status = "closed"` e registra `close`.
   - 412 (a versão mudou entre a leitura e o envio): lê de novo e envia o fechamento **uma única vez**. Se der 412
     de novo, responde 409 `CONFLICT`.
   - 409 (já fechada): lê de novo e responde 200 com a conversa fechada. Registra `close_noop`.
   - 404: responde 404 `NOT_FOUND`. Não registra nada.
   - qualquer outra falha: responde 503 `HARNESS_UNAVAILABLE`. **Nada é registrado como fechado.**

## 4. Tradução do Harness para o formato do Python

A conversão é uma função pura. Ela entrega o mesmo formato que a rota do Python já entrega ao Element, mais `engine`.

| Campo do Harness | Campo entregue ao Element | Regra |
| --- | --- | --- |
| `id` | `id` | igual |
| `agent_id` | `agent_id` | texto ou `null` |
| `agent_name` | `agent_name` | texto ou `null` |
| `client_name` | `client_display_name` | vazio vira `null` |
| `external_peer` | `customer_external_id` | vazio vira `null` |
| `channel` | `channel` | obrigatório; vazio é erro de formato |
| `status` | `status` | `open` → `open`; `transferred` → `open`; `closed` → `closed` |
| `created_at` (ms) | `first_message_at` | ISO UTC com `Z`, igual ao do Python |
| `last_message_at` (ms) | `last_message_at` | ISO UTC com `Z`; `null` se não houver |
| `summary` | `summary` | vazio vira `null` |
| `matrix_room_id` | `matrix_room_id` | só vem com a sala `ready` (ver regra no Harness) |
| — | `engine` | `"harness"` |

Saem sempre `null`, porque só existem no Python: `client_id`, `client_matrix_ghost_user_id`,
`client_matrix_avatar_mxc_uri`, `avatar_url` e `whatsapp_instance_name`. Na conferência do formato, não apareceu
nenhuma tela que mostre `avatar_url` ou `whatsapp_instance_name` vindos do Harness. Os campos de cliente do Matrix
(`client_matrix_*`) não foram conferidos tela a tela: se uma tela passar a usá-los, é preciso rever esta tabela.

Se a resposta do Harness não tiver a forma esperada (campo obrigatório ausente, status desconhecido, canal
vazio, data que não é número), a conversão falha e a rota responde 503. O corpo da resposta do Harness nunca é
repassado ao Element.

## 5. Vínculo da organização

### 5.1 Tabela `v1_org_engine_links` (migração 189)

| Coluna | O que é |
| --- | --- |
| `org_id` | organização do Python (chave; apagar a organização apaga o vínculo) |
| `engine` | origem do vínculo; só `harness` hoje |
| `harness_org_id` | organização correspondente no Harness |
| `base_url` | endereço do Harness (origem, sem caminho) |
| `token_env` | **nome** da variável de ambiente com o token. O valor nunca vai ao banco |
| `enabled` | interruptor. Começa desligado |
| `created_at`, `updated_at` | datas da linha |

O banco recusa `token_env` que não tenha cara de nome de variável (letras maiúsculas, números e `_`), recusa
`engine` diferente de `harness` e recusa campos vazios. Assim, um valor de token não consegue entrar no banco por
engano.

A migração roda com `scripts/run_migrations.py` do backend. Ela **não** roda sozinha quando o backend sobe.

### 5.2 Token de organização

- Cada organização vinculada tem um token de organização do Harness. Ele só opera essa organização.
- O valor fica só no ambiente do processo Python, numa variável cujo nome está em `token_env`.
- Não vai a banco, arquivo versionado, log, resposta nem mensagem de erro.
- O Python lê o valor a cada chamada, no cabeçalho de autorização. Trocar o valor exige reiniciar o processo.
  Quando o ambiente vem de um arquivo de variáveis usado na criação do contêiner, é preciso recriar o contêiner.
- Criação do token no Harness: pela linha de comando offline do Harness, com `token create` e a organização. O
  valor aparece uma vez, então precisa ser gravado no mesmo momento, direto no arquivo de ambiente com permissão
  restrita. Não passe o valor como argumento de comando nem deixe ele no histórico do shell.

### 5.3 Criar, ligar e desligar uma organização

Crie a linha **desligada** primeiro. Use parâmetros do cliente do banco; os marcadores `<…>` são só para leitura.

```sql
-- 1. Criar (desligado). Reexecutar não liga nem desliga: o ON CONFLICT não mexe em enabled.
INSERT INTO v1_org_engine_links (org_id, engine, harness_org_id, base_url, token_env, enabled)
VALUES ('<org_id>', 'harness', '<harness_org_id>', '<base_url>', '<NOME_DA_VARIAVEL>', false)
ON CONFLICT (org_id) DO UPDATE SET
  engine         = EXCLUDED.engine,
  harness_org_id = EXCLUDED.harness_org_id,
  base_url       = EXCLUDED.base_url,
  token_env      = EXCLUDED.token_env,
  updated_at     = NOW();

-- 2. Conferir (não existe token no banco; só o nome da variável).
SELECT org_id, harness_org_id, base_url, token_env, enabled
FROM v1_org_engine_links WHERE org_id = '<org_id>';

-- 3. Ligar.
UPDATE v1_org_engine_links SET enabled = true, updated_at = NOW() WHERE org_id = '<org_id>';

-- 4. Desligar (desfaz o repasse dessa organização).
UPDATE v1_org_engine_links SET enabled = false, updated_at = NOW() WHERE org_id = '<org_id>';
```

Atenção: reexecutar o passo 1 **troca** `base_url`, `token_env` e `harness_org_id` da linha existente, e isso vale
na hora. Confira a linha antes de reexecutar.

Antes de ligar, confira também que a variável nomeada em `token_env` existe no processo com valor. Confira a
presença, sem imprimir o valor.

### 5.4 Efeito de ligar e desligar

- Vale na consulta seguinte, sem reiniciar o processo. Não há cache do vínculo. Testado com alternância
  desligado → ligado → desligado no mesmo processo.
- Sem linha, com `enabled = false`, ou com a variável ausente ou vazia: a organização fica 100% Python.
- A variável ausente gera um aviso no log (`vinculo_harness_indisponivel`). Sem linha ou desligado, o log é só de
  depuração.
- O vínculo confere se a variável **existe**, não se o token é **válido**. Um token errado aparece como 503 e, nos
  registros, como `harness_recusou_token`.

## 6. Erros e o que o atendente vê

| Situação no Harness | Detalhe | Histórico | Fechar |
| --- | --- | --- | --- |
| 200 | conversa, `engine = harness` | item da lista | 200, `closed` |
| 404 | 404 `NOT_FOUND` | não aparece | 404 `NOT_FOUND` |
| 409 no fechamento (já fechada) | — | — | 200 com a conversa fechada (`close_noop`) |
| 412 no fechamento (versão mudou) | — | — | relê e tenta uma vez; se falhar de novo, 409 `CONFLICT` |
| 401 ou 403 (token recusado) | 503 `HARNESS_UNAVAILABLE` | `meta.harness_indisponivel` | 503 `HARNESS_UNAVAILABLE` |
| 400, 422 ou outro 4xx | 503 `HARNESS_UNAVAILABLE` | `meta.harness_indisponivel` | 503 `HARNESS_UNAVAILABLE` |
| 5xx, rede ou tempo esgotado | 503 `HARNESS_UNAVAILABLE` | `meta.harness_indisponivel` | 503 `HARNESS_UNAVAILABLE` |
| Resposta fora da forma esperada | 503 `HARNESS_UNAVAILABLE` | `meta.harness_indisponivel` | 503 `HARNESS_UNAVAILABLE` |

Regras de segurança do fechamento:

- Em qualquer 503, nada é registrado como fechado no Python. A auditoria só é gravada quando o Harness confirma.
- Um 404 do Harness não é tratado como falha: significa "a conversa não está lá".
- Um 503 não quer dizer que a conversa não existe. Por isso o Python não responde 404 nesse caso.

Textos que o Element mostra: a mensagem de 503 e a de 409 (`CONFLICT`) estão em inglês no código. Isso aparece
na tela (ver limites).

## 7. Observabilidade

- **Log:** uma linha por chamada do Element que chega ao Harness, com a mensagem `harness_repasse`. Ela traz a rota
  (`detalhe`, `lista` ou `fechar`), o resultado, a duração em milissegundos, o `request_id`, a organização do Python
  e a do Harness.
- **Resultados possíveis:** `sucesso`, `nao_encontrado`, `conflito`, `ja_fechada`, `recusado`, `indisponivel` e
  `erro_interno` (exceção fora do previsto, que não conta como indisponibilidade do Harness).
- **Não entram no log:** token, telefone do cliente, nome do cliente, texto de mensagem, nem o endereço do Harness.
- **Sem registro** quando não há repasse: organização sem vínculo, id no formato do Python, lista com `cursor` ou
  com `agent_id`.
- **Contadores:** `harness_repasse.<rota>.<resultado>` e `harness_repasse.indisponivel`, no registro de métricas
  que o backend já usa. Nenhum endpoint expõe esses contadores. Eles vivem na memória do processo e zeram ao
  reiniciar.
- **Auditoria do Python:** o fechamento grava `close` ou `close_noop` no registro `audit_event` do log, com o
  usuário que clicou. No Harness, o autor da mesma ação é o token.
- **Log do httpx:** o logger da biblioteca HTTP fica em aviso, para não gravar a URL da lista (que leva o telefone
  do cliente). Isso vale para o processo inteiro, inclusive para outras integrações que usam essa biblioteca.

## 8. Limites conhecidos

1. **Lista unificada sem cursor.** O Element usa `limit = 20` e não pagina essa chamada. O limite máximo do
   Python é 100.
2. **Auditoria no Harness registra o token, não a pessoa.** O atendente que clicou fica só no log do Python.
   Decisão tomada pelo mantenedor.
3. **Fechar não expulsa humanos da sala Matrix.** No caminho do Harness, a sala fica fechada e o módulo Matrix
   avisa na sala. Se um humano escrever nela, nada é enviado ao cliente. Decisão tomada pelo mantenedor.
4. **Latência.** Cada chamada autenticada ao Harness leva cerca de 370 a 500 ms, medido em abertura de detalhe e na
   lista. A causa provável é a conferência do token no Harness, feita a cada pedido. Isso não foi confirmado no
   código do Harness. Abrir uma conversa do Harness e montar o histórico com cliente ficam mais lentos por causa
   disso.
5. **Tempo de resposta do Python ao Harness: 5 s** (conexão: 2 s). Se o cluster do Harness demorar mais que isso
   para responder, o Python vê tempo esgotado e responde 503. A conversa pode existir.
6. **`agent_name` vem vazio na lista** para conversas do Python, como já era antes do repasse. Só o detalhe traz
   o nome. As conversas do Harness trazem o nome na lista.
7. **O cartão do histórico mostra data e situação**, não de qual origem veio cada conversa. A origem fica só na
   resposta da API (`engine`). Observado no aceite no app.
8. **Filtro de canal depois do Harness.** Como o canal é filtrado sobre o resultado, a página pode vir com menos
   itens que `limit` quando o Harness trouxe conversas de outro canal.
9. **Filtro por situação `open` faz duas consultas ao Harness.** O tempo é o mesmo porque roda em paralelo, mas a
   carga no Harness dobra nesse caso.
10. **Identificador do cliente com espaço nas pontas.** O Python compara o texto exato. O Harness compara sem os
    espaços. Só diverge se o dado estiver gravado com espaço, o que o caminho do canal não faz.
11. **Ordem por atividade com chaves ligeiramente diferentes.** No Python, a data de referência é a última
    mensagem, depois a primeira mensagem, depois a criação. No Harness, é a última mensagem, depois a criação.
12. **Resposta perdida no fechamento.** Se o Harness fechar mas a resposta se perder, o Element recebe 503 e nada
    é registrado no Python. Na tentativa seguinte, a conversa já aparece fechada e a resposta é 200 com
    `close_noop`.
13. **Transferida aparece como aberta.** Conversa `transferred` no Harness é mostrada como `open`. Quem atende não
    vê a diferença nessa tela.
14. **Organização sem vínculo que pede conversa inexistente** faz uma leitura a mais no vínculo antes do 404.
    A resposta é igual.
15. **Tabela ausente.** Se a migração 189 não tiver sido aplicada, as chamadas que consultam o vínculo respondem
    erro interno (500), pelo que está no código. Isso vale para o detalhe e o fechamento de conversa que não
    está no Python, e para o histórico com `client_identifier`. Não foi testado num banco sem a tabela.
16. **Vínculo confere presença, não validade.** Um token errado só aparece na primeira chamada, como 503.
17. **`base_url` só é validada como texto não vazio.** O banco não confere o formato do endereço.
18. **Trocar token exige reiniciar** o processo (ou recriar o contêiner, quando o ambiente vem de arquivo).
19. **`X-Request-ID`.** O Python grava o cabeçalho do Element como chegou. Só repassa ao Harness quando o formato é
    seguro (letras, números e `._:-`, até 128 caracteres).
20. **Contadores sem endpoint.** Só dá para ler de dentro do processo, e zeram ao reiniciar.
21. **Sem login de usuário no Harness.** Toda ação no Harness é do token da organização. O Element continua
    autenticando o usuário no Python.

## 9. Como reproduzir o aceite

O aceite tem dois passos, ambos em ambiente de desenvolvimento. Os roteiros que os executam são materiais internos
do projeto e não ficam neste repositório.

1. **Subir o ambiente.** Um roteiro sobe o servidor Matrix de desenvolvimento, o Harness com banco próprio, o
   backend Python com as migrações aplicadas numa base de teste, o app de chat apontando para o servidor de
   desenvolvimento e um vínculo de teste ligado. Ele termina com uma chamada do Python ao Harness, que precisa
   responder com sucesso. Isso prova o caminho Python → Harness.
2. **Rodar os cinco cenários no app:**
   - (1) abrir a sala de uma conversa do Harness: o remetente de sistema mostra o nome do agente;
   - (2) o cartão de histórico do cliente lista a conversa do Harness e a do Python do mesmo cliente; clicar
     na do Harness abre a sala certa;
   - (3) "Encerrar atendimento" muda o cabeçalho para fechado, fecha a conversa no Harness e avisa na sala; a
     mensagem seguinte do cliente abre conversa e sala novas;
   - (4) com o Harness parado, o histórico mostra só as do Python e o erro ao encerrar aparece com a mensagem
     de indisponível;
   - (5) com o vínculo desligado, o histórico volta a mostrar só as do Python, sem reiniciar.

Testes automatizados do repasse (no backend Python, com Harness falso e banco de teste): os arquivos
`tests/test_harness_bridge_*.py` e `tests/test_dashboard_v1_*_harness.py`. Os que usam banco precisam da variável
`POP_ATENDE_TEST_DSN`.

## 10. Quando remover tudo

A ponte sai junto com o backend Python, quando todas as organizações estiverem no Harness. Antes disso, na ordem:

1. Desligar o vínculo de todas as organizações (`enabled = false`).
2. Apontar o webhook de cada instância de WhatsApp de volta ao Python, se o corte já tiver sido feito. Isso é um
   procedimento separado.
3. Confirmar que não há conversa ativa que só o Python precise atender.
4. Retirar as rotas de repasse, o pacote de integração com o Harness, o registro de observabilidade do repasse e
   os testes correspondentes.
5. Remover a tabela `v1_org_engine_links` por migração própria, depois de confirmar que nenhuma linha ativa é
   necessária.
6. Remover este documento e a referência em `docs/CONTRATO_CANAL.md`.

Enquanto o backend Python existir, o Element continua chamando o Python e a ponte continua valendo.

## 11. Onde estão as regras do Harness que a ponte usa

- `docs/API.md`, seção "Dados da conversa para o Element (RP-01)": campos `client_name`, `agent_name` e
  `matrix_room_id`, e a regra da sala `ready`.
- `docs/API.md`, seção "Listagem de conversas: filtros e ordem por atividade (RP-02)": `external_peer`,
  `status`, `sort=recent` e o `400` de cursor com ordenação por atividade.
- `docs/API.md`, seção "Fechar conversa (MX-06)": `If-Match`, os códigos 412 e 409 e a auditoria `close`.
- `docs/CLUSTER.md`: o que acontece com `/admin/v1/orgs/{org}/…` quando o Harness roda em cluster.
