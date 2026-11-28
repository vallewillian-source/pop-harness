# API administrativa do POP Harness (HR-20)

O servidor HTTP do Harness é o mesmo binário da CLI (`poph serve`). Nesta etapa ele expõe a
administração dos recursos listados abaixo, a saúde, as métricas e o chat de teste com
`text/event-stream` (HR-22). Console, roteamento entre instâncias e retenção entram nas tarefas
seguintes.

```bash
poph migrate            # cria/atualiza o banco (o servidor recusa subir com esquema atrasado)
poph serve              # sobe em http://127.0.0.1:7480 (configuração `servidor.endereco`/`porta`)
poph openapi --out api.json   # o mesmo documento que /admin/v1/openapi.json
```

`poph serve` com um banco existente numa versão anterior **recusa subir** e diz qual comando rodar:

```
o banco .../harness.db está na versão 0 e o código espera a 1: rode `poph migrate` com o servidor
parado e suba de novo
```

## Route implementation

The administrative router and OpenAPI registration live in [`crates/poph-server/src/api/mod.rs`](../crates/poph-server/src/api/mod.rs). Handlers declare their published operation IDs and schema names explicitly so Rust module and function names can change independently of the generated Console client.

## Autenticação

Tudo em `/admin/v1` pede `Authorization: Bearer <valor>`. São dois tipos de token:

| Tipo | Formato | Onde vive | Alcance |
| --- | --- | --- | --- |
| **de org** | `poph_<uuid do token>_<segredo>` | `admin_tokens` (hash argon2 do segredo) | só a própria org |
| **global** | `pophg_<segredo>` | configuração `[admin].tokens_globais` (hash) | todas as orgs |

- O token de org é criado por `poph --offline --org <slug> token create --label <rótulo>`. O valor
  em claro aparece **uma única vez**, na saída da criação; nenhum comando e nenhuma rota o
  devolvem depois (o banco guarda só o hash do segredo).
- A API acha o registro pelo uuid que vai no valor e confere o segredo contra o argon2
  (`spawn_blocking`, porque o argon2 é caro de propósito).
- Token de org **não** cria nem remove orgs e tokens (`403`), e recebe `403` ao pedir outra org.
- Token revogado (`poph token delete`) deixa de autenticar na hora.
- There is no token update: the secret never changes. To replace a token, create another one and revoke
  the previous one.
- Tokens criados antes deste formato (hash do valor inteiro, sem uuid no valor) **não autenticam**
  mais: recrie-os com `poph token create`. O registro antigo pode ser revogado normalmente.

## Erros

Formato único em toda a API (PRD §8.1):

```json
{ "code": "validation_error", "message": "campo `slug` inválido", "details": { "field": "slug", "reason": "..." } }
```

| Código | Status | Quando |
| --- | --- | --- |
| `unauthorized` | 401 | sem token, token malformado, desconhecido ou revogado |
| `forbidden` | 403 | token de org fora da própria org; ação que exige token global |
| `not_found` | 404 | registro inexistente (na org do token) |
| `invalid_id` | 400 | id que não é UUID v7, ou cursor ilegível |
| `precondition_required` | 428 | `PATCH` sem `If-Match` |
| `validation_error` | 400 | campo recusado pela validação |
| `invalid_request` | 400 | JSON inválido, corpo que não bate com o contrato |
| `version_conflict` | 412 | `If-Match` diferente da versão atual |
| `conflict` | 409 | slug/nome repetido, org com agentes sem `?force=true`, categoria com produtos ou agentes vinculados sem `?force=true`, produto em itens de carrinho sem `?force=true` |
| `integracao_nao_e_whatsapp` | 409 | rota de WhatsApp numa integração de outro `kind` |
| `whatsapp_nao_configurado` | 409 | integração sem instância da Evolution declarada em `[evolution].instancias` |
| `evolution_recusou` | 502 | a Evolution respondeu erro HTTP (detalhe em `details.evolution_status`) |
| `evolution_indisponivel` | 503 | não deu para falar com a Evolution (rede, prazo) ou falta `EVOLUTION_API_KEY` |
| `evolution_sem_qr` | 502 | a Evolution respondeu o pareamento sem QR e sem código de pareamento |
| `evolution_falhou` | 502 | outra falha ao falar com a Evolution (resposta que não serve, mídia, blob) |
| `invalid_backup` | 400 | arquivo de backup corrompido, de outro formato ou de versão desconhecida (detalhe em `details.version` quando é a versão) |
| `backup_incompleto` | 409 | exportação de org com arquivo da biblioteca sem conteúdo no disco (a exportação não sai pela metade) |
| `importacao_sobrescreve` | 409 | importação de backup que atualizaria linhas que já existem na org, sem `?sobrescrever=true` (detalhe em `details.tabelas`, `details.linhas`, `details.exemplos`); nada foi gravado |
| `payload_too_large` | 413 | corpo acima do teto da rota (importação de backup 64 MB, mensagem do chat de teste 24 MB, webhook 1 MB) |
| `cluster_invalido` | 400 | `cluster.toml` inválido na recarga: o mapa anterior continua em uso (detalhe em `details.problemas`) |
| `assinatura_invalida` | 401 | encaminhamento entre instâncias com cabeçalho de par (inclusive o nonce) ausente/ilegível/malformado, assinatura errada, carimbo fora da janela, instância desconhecida ou nonce repetido |
| `rota_em_laco` | 421 | pedido já encaminhado precisaria ser encaminhado de novo (`x-poph-hop > 1`) ou a org não vive nesta instância segundo o mapa |
| `mapa_divergente` | 421 | o mapa do cluster de quem encaminhou não é o desta instância (detalhe em `details.fingerprint_recebido` e `details.fingerprint_local`) |
| `instancia_fora` | 503 | a instância dona da org não respondeu ao encaminhamento (detalhe em `details.instancia` e `details.url`) |
| `conversa_encerrada` | 409 | reabrir conversa `closed`: nada muda, nada é auditado (FX-07) |
| `ficha_encerrada` | 409 | a Ficha já está fechada: não abre fluxo, não age, não abandona e não fecha de novo |
| `execucao_ativa` | 409 | a Ficha já tem uma execução de fluxo ativa; nada é aberto |
| `fluxo_nao_liberado` | 403 | fluxo não liberado ao agente da Ficha; nenhuma execução é aberta |
| `acao_invalida` | 409 | ação que o motor não aceita naquela etapa, inclusive ação que só o agente pode fazer |
| `campos_invalidos` | 400 | campos enviados não batem com a etapa atual |
| `regra_invalida` / `regra_limite` | 400 | regra do fluxo recusada pelo motor (defeito do fluxo publicado, ou custo acima do limite) |
| `fluxo_invalido` | 422 | script não passa na validação; `details.diagnostics` traz código, gravidade, mensagem, linha e coluna; nada é gravado |
| `fluxo_nao_publicado` | 409 | o fluxo só tem rascunho; `diff` e `preview` pedem uma revisão publicada |
| `teto_de_fluxos_por_org` | 409 | a org chegou ao máximo de fluxos (`details.flows` e `details.limit`); nada é criado |
| `teto_de_fluxos_por_agente` | 409 | mais de 20 fluxos liberados a um agente; nada muda |
| `ir_divergente` | 409 | a IR gravada não confere com o texto publicado; a leitura falha fechada |
| `internal` | 500 | falha inesperada (mensagem sem SQL nem caminho interno) |

Rota que não existe responde `404 not_found` e método errado numa rota existente responde
`405 method_not_allowed`, os dois no mesmo formato `{code, message, details}` — o cliente nunca
recebe corpo vazio. O cabeçalho `WWW-Authenticate` só aparece no `401`.

O Console e os scripts casam pelo `code`, nunca pela `message`.

## Rotas

| Método | Rota | Token | Observação |
| --- | --- | --- | --- |
| GET/POST | `/admin/v1/orgs` | global cria; org lista só a própria | |
| GET/PATCH/DELETE | `/admin/v1/orgs/{org_id}` | global; org na própria | `DELETE` com agentes pede `force` |
| GET/POST | `/admin/v1/orgs/{org_id}/tokens` | global | `POST` devolve o valor **uma vez** |
| GET/DELETE | `/admin/v1/orgs/{org_id}/tokens/{token_id}` | global | `DELETE` revoga |
| GET/POST | `/admin/v1/orgs/{org_id}/categories` | escopo da org | |
| GET/PATCH/DELETE | `/admin/v1/orgs/{org_id}/categories/{category_id}` | escopo da org | `PATCH` exige `If-Match`; `DELETE` com produtos ou agentes vinculados pede `force=true` (sem ele: 409, nada apagado; com ele: apaga os produtos e tira a categoria dos agentes) |
| GET/PATCH/DELETE | `/admin/v1/orgs/{org_id}/products/{product_id}` | escopo da org | `PATCH` exige `If-Match`; `DELETE` de produto que está em itens de carrinho pede `force=true` (sem ele: 409 com `details.itens_de_carrinho` e `details.carrinhos`, nada apagado; com ele: apaga, e os itens ficam no carrinho com o preço gravado, como "produto removido do catálogo"). Produto sem carrinho apaga sem `force` |
| GET/POST | `/admin/v1/orgs/{org_id}/departments` | escopo da org | |
| GET/PATCH/DELETE | `/admin/v1/orgs/{org_id}/departments/{department_id}` | escopo da org | `PATCH` exige `If-Match` |
| POST/GET/POST | `…/agents/{agent_id}/integrations/{integration_id}/whatsapp/{qrcode,state,disconnect}` | escopo da org | pareamento, estado e desconexão; `If-Match` nas duas escritas |
| GET | `…/agents/{agent_id}/files/{file_id}/content` | escopo da org | bytes do arquivo da biblioteca (`Content-Type` gravado, `Content-Disposition` com o nome, sha256 em `x-content-sha256`) |
| GET | `…/agents/{agent_id}/test-chat/{conversation_id}/messages/{message_id}/attachments/{attachment_id}/content` | escopo da org | bytes do anexo de uma mensagem do chat de teste |
| GET | `/admin/v1/orgs/{org_id}/backup` | escopo da org | arquivo do retrato da org (`poph-backup-org` v1) como anexo; `?com_conversas=true` inclui o histórico (padrão: não) |
| POST | `/admin/v1/orgs/{org_id}/backup/import` | escopo da org | corpo cru `application/json` (teto de 64 MB) com o arquivo de `…/backup`; reconstrói a org do caminho e devolve o relatório. Sem `?sobrescrever=true`, recusa (`409 importacao_sobrescreve`) quando atualizaria linha existente |
| GET | `/admin/v1/orgs/{org_id}/manifests` | escopo da org | plugins declarados em `[plugins].integracoes`: um item por `kind` com `nome`, `versao` e `situacao` (`ok`/`indisponivel`) |
| GET | `/admin/v1/orgs/{org_id}/manifests/{kind}` | escopo da org | manifesto do plugin declarado em `[plugins].integracoes`: capacidades, limites e JSON Schema de configuração e segredos |
| POST | `/admin/v1/cluster/reload` | **global** | relê o `cluster.toml` no processo vivo e troca o mapa; arquivo inválido mantém o mapa anterior (`400 cluster_invalido`) |
| GET | `/healthz` | — | `{status, version, uptime_seconds, databases}` e, no modo cluster, `instance_id` e `cluster_map_fingerprint`; 503 se o banco não responde |
| GET | `/metrics` | — | texto Prometheus |
| GET | `/admin/v1/openapi.json` e `/openapi.json` | — | OpenAPI 3.1 gerado por utoipa |

## Organization deletion

`DELETE /admin/v1/orgs/{org_id}` deletes everything inside the organization. The API requires `force`
when the organization has agents. The CLI (`poph org delete`) checks the content first — agents,
categories, departments and tokens — and requires `--force` when anything is inside; with `--force`
it deletes the organization and the products of its categories. The check runs before the repository
call because the delete reports the agent count only after deleting.

**Paginação:** `?limit=1..200` (padrão 50) e `?cursor=<next_cursor>`. A resposta é
`{ "items": [...], "next_cursor": "<uuid>" | null }`. O cursor é o último id da página (UUID v7,
ordenável por tempo) e é exclusivo.

**Concorrência:** toda edição (`PATCH`, e o `PATCH` de org) pede `If-Match: <versão>`. Sem o
cabeçalho: `428`; com versão velha: `412 version_conflict` com `details.expected_version` e
`details.current_version`.

## Métricas (`/metrics`)

Texto no formato de exposição do Prometheus. Os que o servidor atualiza a cada leitura:

- `poph_memoria_processo_bytes` — RAM residente do processo (lida de `/proc/self/status`);
- `poph_fila_turnos` e `poph_fila_saida` — pendentes em todas as orgs (entrada e saída);
- `poph_harness_db_bytes` e `poph_rag_db_bytes` — tamanho de cada banco.

O catálogo registra também `poph_agentes_carregados`, `poph_conversas_ativas` e o histograma
`poph_latencia_etapa_segundos` (etapas do turno), além dos contadores de chamadas e erros de LLM,
Jev e Evolution. Os medidores que dependem do motor do turno aparecem zerados até o motor estar
ligado no servidor; quem preenche a latência é o turno, não o HTTP.

### Métricas do módulo Matrix (`poph_matrix_*`, MX-26)

Estas só aparecem depois que o módulo Matrix é preparado no processo: com `[matrix]` desligado, nenhuma
`poph_matrix_*` entra no `/metrics`. Quem prepara (a ligação no `poph serve`, MX-20) chama
`Estado::metricas_matrix()`, que devolve `None` quando o módulo não está ligado nesta instância, e passa
o handle ao laço (`EspelhoMatrix::com_metricas`), à entrega do atendente (`RespostaDoAtendente::com_metricas`)
e ao destino real (`DestinoReal::com_metricas`). Chamar duas vezes devolve o mesmo handle; nada se duplica
na saída. Não há rótulos por org: o atraso é o pior entre as orgs ligadas.

- `poph_matrix_atraso_cursor_segundos` — idade, em segundos, da mensagem mais antiga que ainda não foi
  espelhada. Sobe enquanto o Synapse recusa e volta a zero quando o laço chega ao fim.
- `poph_matrix_salas_prontas_total` — salas que ficaram prontas nesta passagem, criadas agora ou
  concluídas depois de uma queda. Não separa as duas coisas.
- `poph_matrix_mensagens_espelhadas_total` — mensagens do Harness entregues ao Synapse.
- `poph_matrix_respostas_enfileiradas_total` — respostas do atendente gravadas para envio ao cliente.
- `poph_matrix_respostas_duplicadas_total` — reenvios do Synapse de uma resposta já gravada: nada foi
  enviado de novo.
- `poph_matrix_respostas_conversa_encerrada_total` — respostas cuja conversa já estava encerrada: não
  foram ao cliente, só o aviso na sala.
- `poph_matrix_salas_desconhecidas_total` — eventos de uma sala que este Harness não reconhece: nada
  gravado.
- `poph_matrix_avisos_de_midia_total` — avisos de mídia do atendente escritos na sala (o envio de
  arquivo pelo Element não existe na fase 1).
- `poph_matrix_erros_transitorios_total` e `poph_matrix_erros_definitivos_total` — chamadas ao Synapse
  que falharam, por classe. O `429` conta como transitório.
- `poph_matrix_respostas_429_total` — respostas `429` (limite de taxa) do Synapse.
- `poph_matrix_tentativas_total` — chamadas ao Synapse feitas pelo espelho, inclusive as que falharam.
- `poph_matrix_mensagens_isoladas_total` — mensagens guardadas em `matrix_falhas` (formato inválido ou
  falha definitiva repetida). Elas não voltam sozinhas.
- `poph_matrix_latencia_synapse_segundos` — duração de cada envio de evento e de cada provisão de sala
  feitos pelo espelho.

Logs do módulo: org, conversa e código curto do erro (por exemplo `M_FORBIDDEN` ou
`armazenamento_indisponivel`). Não vão texto de cliente, telefone, corpo de mensagem nem token; o teste
`canary_value_never_appears_in_module_log` confere isso.

Limites: as chamadas do receptor do Application Service não contam (o receptor só recebe, não chama o
Synapse). A latência é medida por operação do espelho, não por chamada HTTP individual do cliente do
Matrix.

`/healthz` e `/metrics` ficam fora do `/admin/v1` e sem autenticação: o servidor escuta só na
própria máquina por padrão e as duas rotas não devolvem dado de cliente. Ao mudar
`servidor.endereco` para a rede, quem protege é o proxy.

## Cliente TypeScript do Console

O Console não escreve o cliente à mão: ele sai do OpenAPI.

```bash
bash console/gen-api.sh                 # grava console/src/api/schema.d.ts
bash console/gen-api.sh --url http://127.0.0.1:7480
bash console/gen-api.sh --out /tmp/schema.d.ts
```

O script roda `poph openapi` (ou lê `/admin/v1/openapi.json` de um servidor no ar) e chama o
`openapi-typescript` pelo `npx` (primeira execução precisa de rede; depois fica no cache do npm).
Ele falha se o arquivo sair vazio ou sem `export interface paths`.

## O que ainda não existe

- Autenticação de usuário/cliente (o Console usa o mesmo token de administração).
- Rate limit por token: também tarefa seguinte. O que existe hoje é o limite por org do provedor de
  LLM (fora da API).
- **Encaminhamento entre instâncias** (webhook/admin/SSE assinados, `x-poph-hop`, `?cluster=true`,
  dona fora do ar → 503): feito; veja a seção "Roteamento entre instâncias" mais abaixo e `docs/CLUSTER.md`.
- O `/metrics` é montado a partir do registro do processo; as latências são do próprio servidor HTTP,
  não do turno do agente (essas vêm do registro de turno).

## Inventário da HR-21 — rotas, paridade, versão, auditoria e isolamento

Uma linha por operação publicada no OpenAPI. A tabela é conferida por teste
(`api_docs_cover_all_published_routes`, em `crates/poph-server/tests/api.rs`): publicar rota nova
sem documentar aqui, documentar rota que não existe, ou trocar uma coluna (412, auditoria,
isolamento) faz o teste falhar.

Colunas:

- **CLI** — existe operação equivalente na CLI (`sim`) ou o recurso não tem comando (`exceção`, com o
  motivo). Integrações e conversas são exceções declaradas: a API é a referência.
- **412** — a edição exige `If-Match` e devolve `412 version_conflict` com versão velha (`sim`), ou a
  operação não edita nada (`n/a`).
- **Auditoria** — a escrita registra ator identificado em `audit_log` (`sim`) ou a operação é de
  leitura (`n/a`). Nunca guarda o valor do token.
- **Isolamento** — a rota é escopada por org: token da org A recebe 403/404 nas coleções e nos itens
  da org B.

| Recurso | Método | Caminho | CLI | 412 | Auditoria | Isolamento |
| --- | --- | --- | --- | --- | --- | --- |
| Organizações | GET | `/admin/v1/orgs` | sim | n/a | n/a | sim |
| Organizações | POST | `/admin/v1/orgs` | sim | n/a | sim | sim |
| Organizações | GET | `/admin/v1/orgs/{org_id}` | sim | n/a | n/a | sim |
| Organizações | PATCH | `/admin/v1/orgs/{org_id}` | sim | sim | sim | sim |
| Organizações | DELETE | `/admin/v1/orgs/{org_id}` | sim | n/a | sim | sim |
| Tokens | GET | `/admin/v1/orgs/{org_id}/tokens` | sim | n/a | n/a | sim |
| Tokens | POST | `/admin/v1/orgs/{org_id}/tokens` | sim | n/a | sim | sim |
| Tokens | GET | `/admin/v1/orgs/{org_id}/tokens/{token_id}` | sim | n/a | n/a | sim |
| Tokens | DELETE | `/admin/v1/orgs/{org_id}/tokens/{token_id}` | sim | n/a | sim | sim |
| Categorias | GET | `/admin/v1/orgs/{org_id}/categories` | sim | n/a | n/a | sim |
| Categorias | POST | `/admin/v1/orgs/{org_id}/categories` | sim | n/a | sim | sim |
| Categorias | GET | `/admin/v1/orgs/{org_id}/categories/{category_id}` | sim | n/a | n/a | sim |
| Categorias | PATCH | `/admin/v1/orgs/{org_id}/categories/{category_id}` | sim | sim | sim | sim |
| Categorias | DELETE | `/admin/v1/orgs/{org_id}/categories/{category_id}` | sim | n/a | sim | sim |
| Departamentos | GET | `/admin/v1/orgs/{org_id}/departments` | sim | n/a | n/a | sim |
| Departamentos | POST | `/admin/v1/orgs/{org_id}/departments` | sim | n/a | sim | sim |
| Departamentos | GET | `/admin/v1/orgs/{org_id}/departments/{department_id}` | sim | n/a | n/a | sim |
| Departamentos | PATCH | `/admin/v1/orgs/{org_id}/departments/{department_id}` | sim | sim | sim | sim |
| Departamentos | DELETE | `/admin/v1/orgs/{org_id}/departments/{department_id}` | sim | n/a | sim | sim |
| Agentes | GET | `/admin/v1/orgs/{org_id}/agents` | sim | n/a | n/a | sim |
| Agentes | POST | `/admin/v1/orgs/{org_id}/agents` | sim | n/a | sim | sim |
| Agentes | GET | `/admin/v1/orgs/{org_id}/agents/{agent_id}` | sim | n/a | n/a | sim |
| Agentes | PATCH | `/admin/v1/orgs/{org_id}/agents/{agent_id}` | sim | sim | sim | sim |
| Agentes | DELETE | `/admin/v1/orgs/{org_id}/agents/{agent_id}` | sim | n/a | sim | sim |
| Fluxos do agente | GET | `/admin/v1/orgs/{org_id}/agents/{agent_id}/flows` | exceção: CLI vem na FX-31 | n/a | n/a | sim |
| Fluxos do agente | PUT | `/admin/v1/orgs/{org_id}/agents/{agent_id}/flows` | exceção: CLI vem na FX-31 | sim | sim | sim |
| Instruções | GET | `/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions` | sim | n/a | n/a | sim |
| Instruções | POST | `/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions` | sim | n/a | sim | sim |
| Instruções | GET | `/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current` | sim | n/a | n/a | sim |
| Instruções | PATCH | `/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current` | sim | sim | sim | sim |
| Instruções | DELETE | `/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current` | sim | n/a | sim | sim |
| Base de conhecimento | GET | `/admin/v1/orgs/{org_id}/knowledge` | sim | n/a | n/a | sim |
| Base de conhecimento | POST | `/admin/v1/orgs/{org_id}/knowledge` | sim | n/a | sim | sim |
| Base de conhecimento | GET | `/admin/v1/orgs/{org_id}/knowledge/{group_id}` | sim | n/a | n/a | sim |
| Base de conhecimento | PATCH | `/admin/v1/orgs/{org_id}/knowledge/{group_id}` | sim | sim | sim | sim |
| Base de conhecimento | DELETE | `/admin/v1/orgs/{org_id}/knowledge/{group_id}` | sim | n/a | sim | sim |
| Base de conhecimento | GET | `/admin/v1/orgs/{org_id}/knowledge/{group_id}/items` | sim | n/a | n/a | sim |
| Base de conhecimento | POST | `/admin/v1/orgs/{org_id}/knowledge/{group_id}/items` | sim | n/a | sim | sim |
| Base de conhecimento | GET | `/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}` | sim | n/a | n/a | sim |
| Base de conhecimento | PATCH | `/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}` | sim | sim | sim | sim |
| Base de conhecimento | DELETE | `/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}` | sim | n/a | sim | sim |
| Base × agente | POST | `/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/reindex` | sim | n/a | sim | sim |
| Base × agente | POST | `/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/{group_id}` | sim | n/a | sim | sim |
| Base × agente | DELETE | `/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/{group_id}` | sim | n/a | sim | sim |
| Produtos | GET | `/admin/v1/orgs/{org_id}/products` | sim | n/a | n/a | sim |
| Produtos | POST | `/admin/v1/orgs/{org_id}/products` | sim | n/a | sim | sim |
| Produtos | GET | `/admin/v1/orgs/{org_id}/products/{product_id}` | sim | n/a | n/a | sim |
| Produtos | PATCH | `/admin/v1/orgs/{org_id}/products/{product_id}` | sim | sim | sim | sim |
| Produtos | DELETE | `/admin/v1/orgs/{org_id}/products/{product_id}` | sim | n/a | sim | sim |
| Backup | GET | `/admin/v1/orgs/{org_id}/backup` | sim | n/a | n/a | sim |
| Backup | POST | `/admin/v1/orgs/{org_id}/backup/import` | sim | n/a | sim | sim |
| Manifestos | GET | `/admin/v1/orgs/{org_id}/manifests` | exceção: a CLI não tem comando de manifesto | n/a | n/a | sim |
| Manifestos | GET | `/admin/v1/orgs/{org_id}/manifests/{kind}` | exceção: a CLI não tem comando de manifesto | n/a | n/a | sim |
| Cluster | POST | `/admin/v1/cluster/reload` | exceção: a CLI lê e valida o mapa (`poph cluster show/check`), mas a recarga do processo vivo é só por HTTP | n/a | n/a | n/a |
| Integrações | GET | `/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations` | exceção: a CLI não tem comando de integração | n/a | n/a | sim |
| Integrações | POST | `/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations` | exceção: a CLI não tem comando de integração | n/a | sim | sim |
| Integrações | GET | `/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}` | exceção: a CLI não tem comando de integração | n/a | n/a | sim |
| Integrações | PATCH | `/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}` | exceção: a CLI não tem comando de integração | sim | sim | sim |
| Integrações | DELETE | `/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}` | exceção: a CLI não tem comando de integração | n/a | sim | sim |
| Integrações WhatsApp | POST | `/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/qrcode` | exceção: a CLI não tem comando de integração | sim | sim | sim |
| Integrações WhatsApp | GET | `/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/state` | exceção: a CLI não tem comando de integração | n/a | n/a | sim |
| Integrações WhatsApp | POST | `/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/disconnect` | exceção: a CLI não tem comando de integração | sim | sim | sim |
| Biblioteca de arquivos | GET | `/admin/v1/orgs/{org_id}/agents/{agent_id}/files` | sim | n/a | n/a | sim |
| Biblioteca de arquivos | POST | `/admin/v1/orgs/{org_id}/agents/{agent_id}/files` | sim | n/a | sim | sim |
| Biblioteca de arquivos | GET | `/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}` | sim | n/a | n/a | sim |
| Biblioteca de arquivos | GET | `/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}/content` | exceção: a CLI lê o blob local (`poph file show` imprime o caminho relativo) | n/a | n/a | sim |
| Biblioteca de arquivos | DELETE | `/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}` | sim | n/a | sim | sim |
| Conversas | GET | `/admin/v1/orgs/{org_id}/conversations` | exceção: a CLI não tem comando de conversa | n/a | n/a | sim |
| Conversas | GET | `/admin/v1/orgs/{org_id}/conversations/{conversation_id}` | exceção: a CLI não tem comando de conversa | n/a | n/a | sim |
| Conversas | GET | `/admin/v1/orgs/{org_id}/conversations/{conversation_id}/messages` | exceção: a CLI não tem comando de conversa | n/a | n/a | sim |
| Conversas | POST | `/admin/v1/orgs/{org_id}/conversations/{conversation_id}/reopen` | exceção: a CLI não tem comando de conversa | sim | sim | sim |
| Conversas | POST | `/admin/v1/orgs/{org_id}/conversations/{conversation_id}/close` | exceção: a CLI não tem comando de conversa | sim | sim | sim |
| Conversas | POST | `/admin/v1/orgs/{org_id}/conversations/{conversation_id}/transfer` | exceção: a CLI não tem comando de conversa | n/a | sim | sim |
| Conversas | GET | `/admin/v1/orgs/{org_id}/conversations/{conversation_id}/turns` | exceção: a CLI não tem comando de conversa | n/a | n/a | sim |
| Chat de teste | GET | `/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat` | exceção: a CLI não tem comando de chat de teste | n/a | n/a | sim |
| Chat de teste | POST | `/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat` | exceção: a CLI não tem comando de chat de teste | n/a | sim | sim |
| Chat de teste | GET | `/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}` | exceção: a CLI não tem comando de chat de teste | n/a | n/a | sim |
| Chat de teste | POST | `/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/messages` | exceção: a CLI não tem comando de chat de teste | n/a | sim | sim |
| Chat de teste | POST | `/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/reopen` | exceção: a CLI não tem comando de chat de teste | sim | sim | sim |
| Chat de teste | GET | `/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/messages/{message_id}/attachments/{attachment_id}/content` | exceção: a CLI não tem comando de chat de teste | n/a | n/a | sim |
| Módulo Matrix | GET | `/admin/v1/orgs/{org_id}/matrix` | sim | n/a | n/a | sim |
| Módulo Matrix | PUT | `/admin/v1/orgs/{org_id}/matrix` | sim | sim | sim | sim |
| Espaços do Matrix | GET | `/admin/v1/orgs/{org_id}/matrix/spaces` | sim | n/a | n/a | sim |
| Espaços do Matrix | PUT | `/admin/v1/orgs/{org_id}/matrix/spaces` | sim | n/a | sim | sim |
| Espaços do Matrix | DELETE | `/admin/v1/orgs/{org_id}/matrix/spaces/{department_id}` | sim | n/a | sim | sim |
| Fluxos (script) | POST | `/admin/v1/orgs/{org_id}/flows/validate` | exceção: CLI vem na FX-31 | n/a | n/a | sim |
| Fluxos (script) | POST | `/admin/v1/orgs/{org_id}/flows/format` | exceção: CLI vem na FX-31 | n/a | n/a | sim |
| Fluxos | GET | `/admin/v1/orgs/{org_id}/flows` | exceção: CLI vem na FX-31 | n/a | n/a | sim |
| Fluxos | POST | `/admin/v1/orgs/{org_id}/flows` | exceção: CLI vem na FX-31 | n/a | sim | sim |
| Fluxos | GET | `/admin/v1/orgs/{org_id}/flows/{flow_id}` | exceção: CLI vem na FX-31 | n/a | n/a | sim |
| Fluxos | PATCH | `/admin/v1/orgs/{org_id}/flows/{flow_id}` | exceção: CLI vem na FX-31 | sim | sim | sim |
| Fluxos | DELETE | `/admin/v1/orgs/{org_id}/flows/{flow_id}` | exceção: CLI vem na FX-31 | n/a | sim | sim |
| Revisões de fluxo | POST | `/admin/v1/orgs/{org_id}/flows/{flow_id}/publish` | exceção: CLI vem na FX-31 | sim | sim | sim |
| Revisões de fluxo | GET | `/admin/v1/orgs/{org_id}/flows/{flow_id}/revisions` | exceção: CLI vem na FX-31 | n/a | n/a | sim |
| Revisões de fluxo | GET | `/admin/v1/orgs/{org_id}/flows/{flow_id}/revisions/{number}` | exceção: CLI vem na FX-31 | n/a | n/a | sim |
| Revisões de fluxo | GET | `/admin/v1/orgs/{org_id}/flows/{flow_id}/diff` | exceção: CLI vem na FX-31 | n/a | n/a | sim |
| Revisões de fluxo | GET | `/admin/v1/orgs/{org_id}/flows/{flow_id}/preview` | exceção: CLI vem na FX-31 | n/a | n/a | sim |
| Fichas | GET | `/admin/v1/orgs/{org_id}/fichas` | exceção: CLI vem na FX-31 | n/a | n/a | sim |
| Fichas | GET | `/admin/v1/orgs/{org_id}/fichas/{ficha_id}` | exceção: CLI vem na FX-31 | n/a | n/a | sim |
| Fichas | GET | `/admin/v1/orgs/{org_id}/fichas/{ficha_id}/events` | exceção: CLI vem na FX-31 | n/a | n/a | sim |
| Fichas | POST | `/admin/v1/orgs/{org_id}/fichas/{ficha_id}/close` | exceção: CLI vem na FX-31 | sim | sim | sim |
| Execuções de fluxo | POST | `/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs` | exceção: CLI vem na FX-31 | sim | sim | sim |
| Execuções de fluxo | POST | `/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs/{run_id}/actions` | exceção: CLI vem na FX-31 | sim | sim | sim |
| Execuções de fluxo | POST | `/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs/{run_id}/abandon` | exceção: CLI vem na FX-31 | sim | sim | sim |
| Conversation analytics | GET | `/admin/v1/orgs/{org_id}/analytics/conversations` | exceção: sem equivalente na CLI | n/a | n/a | sim |
| SLA analytics | GET | `/admin/v1/orgs/{org_id}/analytics/sla` | exceção: sem equivalente na CLI | n/a | n/a | sim |
| Token usage | GET | `/admin/v1/orgs/{org_id}/usage/tokens` | exceção: sem equivalente na CLI | n/a | n/a | sim |
| Atendimento (resumo) | GET | `/admin/v1/orgs/{org_id}/analytics/operational-summary` | exceção: sem equivalente na CLI | n/a | n/a | sim |
| Conversas do site | POST | `/admin/v1/orgs/{org_id}/agents/{agent_id}/site-conversations` | exceção: sem equivalente na CLI | n/a | sim | sim |
| Conversas do site | GET | `/admin/v1/orgs/{org_id}/site-conversations/{id}/messages` | exceção: sem equivalente na CLI | n/a | n/a | sim |
| Conversas do site | POST | `/admin/v1/orgs/{org_id}/site-conversations/{id}/messages` | exceção: sem equivalente na CLI | n/a | sim | sim |
| Conversas do site | GET | `/admin/v1/orgs/{org_id}/site-conversations/{id}/turns/{turn_id}` | exceção: sem equivalente na CLI | n/a | n/a | sim |
| Conversas do site | POST | `/admin/v1/orgs/{org_id}/site-conversations/{id}/turns/{turn_id}/cancel` | exceção: sem equivalente na CLI | n/a | sim | sim |
| Conversas do site | GET | `/admin/v1/orgs/{org_id}/site-conversations/{id}/events` | exceção: sem equivalente na CLI | n/a | n/a | sim |
| Conversas do site | GET | `/admin/v1/orgs/{org_id}/site-conversations/{id}/events/stream` | exceção: sem equivalente na CLI | n/a | n/a | sim |
| Conversas de adaptador | POST | `/admin/v1/orgs/{org_id}/agents/{agent_id}/adapter-conversations` | exceção: sem equivalente na CLI | n/a | sim | sim |
| Conversas de adaptador | GET | `/admin/v1/orgs/{org_id}/adapter-conversations/{id}` | exceção: sem equivalente na CLI | n/a | n/a | sim |
| Conversas de adaptador | POST | `/admin/v1/orgs/{org_id}/adapter-conversations/{id}/messages` | exceção: sem equivalente na CLI | n/a | sim | sim |
| Conversas de adaptador | GET | `/admin/v1/orgs/{org_id}/adapter-conversations/{id}/messages` | exceção: sem equivalente na CLI | n/a | n/a | sim |
| Conversas de adaptador | POST | `/admin/v1/orgs/{org_id}/adapter-conversations/{id}/turns` | exceção: sem equivalente na CLI | n/a | sim | sim |
| Conversas de adaptador | GET | `/admin/v1/orgs/{org_id}/adapter-conversations/{id}/turns/{turn_id}` | exceção: sem equivalente na CLI | n/a | n/a | sim |
| Conversas de adaptador | POST | `/admin/v1/orgs/{org_id}/adapter-conversations/{id}/turns/{turn_id}/cancel` | exceção: sem equivalente na CLI | n/a | sim | sim |

## Fechar conversa (MX-06)

`POST /admin/v1/orgs/{org_id}/conversations/{conversation_id}/close` encerra a conversa. Exige
`If-Match` com a versão atual da conversa. Ainda não há botão para isso no Console.

| Situação | Resposta | Efeito |
| --- | --- | --- |
| sem token | `401` | nada muda |
| token de outra org | `403` | nada muda |
| sem `If-Match` | `428` | nada muda |
| conversa não existe na org | `404` | nada muda |
| `If-Match` diferente da versão atual | `412` | nada muda, nada é auditado |
| conversa já está `closed` (com a versão atual) | `409` | nada muda, nada é auditado |
| sucesso | `200` | status `closed`, versão +1, auditoria `close` |

Com versão antiga numa conversa que já foi fechada, a resposta é `412`. Depois de atualizar a versão,
a mesma chamada passa a dar `409`.

A auditoria `close` grava o token que fechou como ator (`actor_kind = admin_token`). Fechar não apaga
o histórico: mensagens, turnos e a fila de saída continuam como estavam. A próxima mensagem do cliente
abre uma conversa nova (ver `docs/ENTRADA_E_SAIDA.md`, seção 4).

Provas: `close_conversation_returns_expected_statuses_and_audits_once` (os seis status, auditoria, versão) e
`message_after_closure_opens_new_conversation` em `crates/poph-server/tests/api.rs`.

## Transferir conversa para outro departamento (PA-119)

`POST /admin/v1/orgs/{org_id}/conversations/{conversation_id}/transfer` marca a conversa como
`transferred` e, quando o `department_id` vem no corpo, troca o departamento alvo dela. É a rota que
o `pop-api` usa para transferir conversa viva: antes dela, só o agente ou um fluxo conseguiam
transferir, de dentro do turno.

Corpo: `{"department_id": "<uuid do departamento>", "reason": "<motivo>"}`. O `department_id` é
opcional (sem ele, só a situação muda); o `reason` é obrigatório e não pode ser vazio.

| Situação | Resposta | Efeito |
| --- | --- | --- |
| sem token | `401` | nada muda |
| token de outra org | `403` | nada muda |
| conversa não existe na org | `404` | nada muda |
| departamento não existe na org | `404` | nada muda |
| corpo que não é objeto JSON | `400` `invalid_request` | nada muda |
| `reason` ausente ou vazio, `department_id` malformado | `400` `validation_error` | nada muda |
| conversa `closed` | `409` `conversa_encerrada` | nada muda, nada é auditado |
| sucesso | `200` | status `transferred`, departamento alvo gravado, versão +1, auditoria `transfer` |

A rota não exige `If-Match`: a versão sobe do mesmo jeito, então um `PATCH` de quem estava com a
versão antiga continua recebendo `412`. A auditoria `transfer` grava o token como ator e o detalhe
`{"motivo": ..., "ator": "administrador", "department_id": ...}`.

Quem move a sala do Matrix é o laço de estado: no ciclo seguinte, a conversa que mudou de departamento
recebe um aviso e um `m.space.child` no espaço do departamento novo (o mesmo caminho do agente que
transfere dentro do turno — provas em `crates/poph-server/tests/matrix_estado.rs`).

Provas: `conversas_transferencia.rs` (`transfer_marks_conversation_and_audits_reason`,
`transfer_refuses_closed_conversation_from_other_org_and_invalid_body`) e, no repositório,
`transferencia_administrativa_*` em `crates/poph-store/tests/transfer.rs`.

## Reabrir conversa (FX-07, PRD L13)

`POST /admin/v1/orgs/{org_id}/conversations/{conversation_id}/reopen` continua reabrindo conversa
`transferred` (`200`). O que mudou: **conversa `closed` não reabre**.

| Situação | Resposta | Efeito |
| --- | --- | --- |
| `If-Match` diferente da versão atual | `412` | nada muda |
| conversa `transferred` (versão atual) | `200` | status `open`, versão +1, auditoria `reopen` |
| conversa `closed` (versão atual) | `409` `conversa_encerrada` | nada muda, nada é auditado |

O `409` vem antes de qualquer gravação e vale também para o `reopen` do chat de teste. O caminho para
o cliente voltar a ser atendido é a mensagem nova dele, que abre outra conversa **e outra Ficha**: a
conversa encerrada e a Ficha dela ficam fechadas como registro do atendimento. O mesmo vale para o
`PATCH` genérico de situação quando ele tentaria devolver uma conversa `closed` para `open`.

Provas: `reopen_closed_conversation_returns_conflict` em
`crates/poph-server/tests/api.rs` e `reopening_closed_conversation_is_refused_and_transferred_keeps_reopening`
em `crates/poph-store/tests/cases_invariant.rs`.

## Ficha do atendimento (FX-07)

Toda conversa nova ganha uma **Ficha** na mesma escrita que a cria, e fechar a conversa fecha a Ficha
junto (tabelas `fichas` e `ficha_eventos`, `docs/DADOS.md`). Ainda não há rota de Ficha: as rotas de
leitura e de ação do atendente são a FX-29 e a FX-30.

## Dados da conversa para o Element (RP-01)

Os quatro endpoints que devolvem uma conversa, `GET …/conversations` (lista),
`GET …/conversations/{conversation_id}`, `POST …/close` e `POST …/reopen`, trazem três campos a mais.
Nenhum campo que já existia mudou de nome ou de tipo.

| Campo | Tipo | O que é |
| --- | --- | --- |
| `client_name` | texto ou `null` | Nome do cliente no WhatsApp (o último `pushName` recebido). `null` quando ainda não chegou ou veio vazio. |
| `agent_name` | texto ou `null` | Nome do agente que atende a conversa. `null` só se o agente não existir mais. |
| `matrix_room_id` | texto ou `null` | Id da sala do Matrix da conversa (`!…:servidor`). Sai só quando a sala está `ready` **e** o módulo Matrix está ligado na org. |

Regras do `matrix_room_id`:

- `null` quando a sala ainda está em criação (`creating`), quando a conversa não tem sala, e quando o
  módulo está desligado na org (o mesmo `enabled` que `GET …/matrix` mostra).
- Desligar o módulo não apaga a sala no Synapse: só deixa de mostrar o id nesta API. Ligar de novo
  devolve o id sem criar sala nova.
- Conversa de outra org continua `404`, e o nome e a sala dela não saem em nenhuma resposta.

Leitura: a lista busca os nomes dos agentes e as salas da página inteira em consultas únicas, e não
uma por conversa. Uma página de 2 conversas e uma de 30 gastam o mesmo número de leituras no banco
(medido: 5 nos dois casos, em `crates/poph-server/tests/conversas_element.rs`). `limit` e `cursor`
seguem como antes.

## Listagem de conversas: filtros e ordem por atividade (RP-02)

`GET …/conversations` aceita quatro parâmetros a mais. Sem eles, a resposta é a de sempre.

| Parâmetro | Valores | Efeito |
| --- | --- | --- |
| `external_peer` | identificador do cliente no canal (no WhatsApp, o número) | só as conversas desse cliente, dentro da org. Espaços nas pontas são tirados. Vazio devolve `400`. |
| `status` | `open`, `transferred` ou `closed` | só as conversas nessa situação. Outro valor devolve `400`. |
| `sort` | `recent` | ordena pela atividade mais recente (última mensagem; sem mensagem, a data de criação), mais recente primeiro. Empate: id maior primeiro. |
| `last_activity_before` | instante em milissegundos desde 1970 | só as conversas cuja última atividade é **anterior** a esse instante. É o corte que acha conversa parada; valor que não é inteiro devolve `400`. |

Regras:

- Os filtros se combinam (os dois precisam valer). A lista é sempre da org do token: conversa de outra
  org não aparece em nenhum filtro.
- Sem `sort`: ordem de id crescente, com `limit` e `cursor` como antes. `limit` fora de 1 a 200 é
  ajustado, não recusado.
- Com `sort=recent`: **não usa cursor**. `limit` (padrão 50, de 1 a 200) é o único controle, e
  `next_cursor` sai sempre `null`. Mandar `cursor` com um valor ou `limit` fora da faixa devolve `400`;
  `cursor` vazio é o mesmo que não mandar.
- Os erros `400` trazem uma mensagem curta e não repetem o valor que foi recebido.
- O `client_identifier` da sala do Matrix é o `external_peer` sem espaço nas pontas, ou o id da
  conversa quando o `external_peer` está vazio. Para achar o histórico de um cliente a partir da sala,
  use esse mesmo valor como `external_peer`. Conversa sem `external_peer` não aparece num filtro por
  cliente, porque o id da conversa não é um cliente.

Exemplos:

```
GET …/conversations?external_peer=5511999999999&status=open
GET …/conversations?sort=recent&limit=20
GET …/conversations?status=open&sort=recent&limit=100&last_activity_before=1764000000000
```

`last_activity_before` combinado com `status=open` e `sort=recent` é o que o worker de inatividade do
`pop-api` usa para achar conversa viva parada e fechá-la (o corte em si é do `pop-api`, que decide
quanto tempo é inatividade).

Testes: `crates/poph-server/tests/conversas_filtros.rs` (filtros, ordem, `limit`, cursor recusado e
valores inválidos), `crates/poph-server/tests/conversas_transferencia.rs` (corte por última atividade)
e `crates/poph-store/tests/conversation_filters.rs` (plano da consulta e identificador).
O índice que sustenta o filtro por cliente está em `docs/DADOS.md`, seção 4 (migração `0015`).

## Chat de teste do Console (HR-22)

O Console abre uma conversa de teste do agente, manda a mensagem do cliente com anexos e recebe o
turno pela conexão, sem esperar a resposta inteira. É o contrato da aba de chat (HR-46) e do painel
do porquê (HR-47).

### Rotas

| Operação | Rota | Efeito |
| --- | --- | --- |
| criar conversa | `POST /admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat` | abre conversa nova com memória zerada (`channel = "test"`, sem integração) e devolve `201` com `id`, `status`, `version` |
| listar | `GET …/test-chat?limit=&cursor=` | conversas de teste do agente, mais recentes primeiro, com `message_count` |
| histórico | `GET …/test-chat/{conversation_id}` | `{conversation, messages, turns}`; cada mensagem traz `attachments` (nome, mime, tamanho, `sha256`, `extraction_status`, `has_text`), e cada turno traz `jev`, `retrieved`, `tools`, `tool_calls`, `timings`, tokens, `error` e `files_out[]` |
| enviar mensagem | `POST …/test-chat/{conversation_id}/messages` | corpo `multipart/form-data` com `text` (opcional) e `file` (zero ou mais) → resposta `text/event-stream` |
| reabrir | `POST …/test-chat/{conversation_id}/reopen` | exige `If-Match`; devolve a conversa para `open` (o mesmo efeito de `conversations/{id}/reopen`) |

### Arquivos que o agente enviou (`files_out`, HR-46d)

Cada turno do histórico traz `files_out[]` com os arquivos que o **agente** enfileirou para enviar
naquele turno (`enviar_arquivo`, HR-24). É o que deixa o Console baixar ou pré-visualizar o arquivo
pelo caminho de conteúdo da biblioteca — antes, o histórico só trazia os anexos do cliente.

| Campo | Conteúdo |
| --- | --- |
| `id` | id do item na fila de saída |
| `turn_id` | turno que o gerou |
| `seq` | sequência do item dentro do turno, derivada do arquivo (é a chave de idempotência do envio, não a ordem `1, 2, 3`) |
| `arquivo_id` | id na biblioteca do agente — é o que a rota `/files/{file_id}/content` aceita |
| `name`, `mime`, `size_bytes`, `sha256` | metadados de apresentação, os mesmos que o cliente recebeu |
| `state` | estado real do item na fila (`pending`, `sending`, `sent`, `failed`) |
| `created_at` | criação do item (ms) |

Regras:

- **Só arquivo.** Itens de texto e de presença não entram na lista; turno que não mandou arquivo traz
  `files_out: []` (nunca `null`).
- **Nada de caminho de blob.** A resposta não traz `blob_ref`, `conteudo_json` nem o caminho do
  blob; o conteúdo só sai pela rota de conteúdo, com autorização.
- **Arquivo excluído da biblioteca depois do envio** continua listado (o histórico é registro do que
  saiu), e o download responde `404 not_found` — comportamento esperado, não erro de contrato.
- Sem rota nova: o `files_out` sai dentro do histórico que já existe.

### Chamadas de ferramenta por turno (`tool_calls`, HR-47b)

Cada turno do histórico traz `tool_calls[]`, o detalhe de cada chamada de ferramenta que o modelo
fez. `tools` continua sendo a lista de nomes, no mesmo formato de antes; `tool_calls` é o que deixa
o painel do por quê separar chamada executada de chamada recusada e mostrar o que voltou ao modelo.

| Campo | Conteúdo |
| --- | --- |
| `nome` | nome que o modelo pediu |
| `rodada` | rodada do laço de ferramentas (0 = primeira resposta do modelo) |
| `ordem` | ordem da chamada dentro do turno, começando em 1 |
| `estado` | `executada` ou `recusada` |
| `motivo` | quando `recusada`: `nao_habilitada`, `desconhecida`, `argumento_invalido`, `erro_de_execucao`, `limite_de_chamadas` ou `fora_do_turno`; `null` quando executada |
| `duracao_ms` | duração da execução (zero nas recusas antes de rodar) |
| `argumentos` | resumo dos argumentos: JSON compacto em texto, cortado em 1024 caracteres com `…[cortado]`; valor de campo com nome de segredo (`token`, `senha`, `secret`, `chave`, `api_key`, `password`, `authorization`, `credencial`) sai como `[redigido]` |
| `resultado` | resumo do texto devolvido ao modelo, cortado em 1024 caracteres com `…[cortado]` |

Regras:

- Turno sem chamada traz `tool_calls: []`; turno antigo (gravado antes da HR-47b) traz `tools` com a
  lista de nomes e `tool_calls: []` — nenhuma leitura quebra.
- `tools` mantém o comportamento anterior: lista toda chamada que chegou ao laço, inclusive a
  recusada. Quem separa é `tool_calls`.
- Nenhum segredo de integração, chave de provedor ou token sai aqui: os argumentos são do modelo e
  passam pela redação por nome de campo; o resultado é a saída das próprias ferramentas.
- O mesmo campo sai na listagem de turnos da conversa (`GET …/conversations/{id}/turns`).

### Eventos do `text/event-stream`

Cada evento tem `id` sequencial (começa em `1`), `event:` com o nome e `data:` com JSON de uma
linha. A ordem é a do turno:

| Evento | Dados | Quando |
| --- | --- | --- |
| `mensagem` | `conversation_id`, `message_id`, `text`, `attachments[]` | primeiro, antes de o turno abrir: confirma o que foi gravado |
| `inicio` | `turn_id`, `agent_id`, `conversation_id` | o turno abriu no banco |
| `texto` | `delta` | cada pedaço da resposta do modelo, na ordem |
| `base` | `sources[]` | trechos da base que entraram no prompt |
| `ferramenta` | `name`, `round` | o modelo pediu uma ferramenta |
| `transferencia` | `department_id`, `risk_type`, `guidance` | o G1 transferiu a conversa |
| `fim` | `turn_id`, `state`, `reason`, `text`, `error`, `model_call_failed`, `messages[]`, `input_tokens`, `output_tokens`, `model`, `timings` | o turno terminou (com ou sem resposta) |
| `erro` | `code`, `message` | o turno não pôde nem começar (falha de armazenamento) |

O `fim` é sempre o último evento de um turno que chegou a começar. `state` usa o vocabulário do
banco: `done`, `failed` ou `skipped`.

### Regras

- **Um turno por conversa.** Com outro turno em voo, a rota responde `409 turno_em_andamento` antes
  de gravar a mensagem; a segunda aba não entra no turno que já está rodando.
- **Desconexão cancela.** Fechar a conexão aciona o cancelamento do turno: ele termina `failed`, a
  reserva é liberada e **nada** é gravado na `outbox` — não existe resposta parcial enviada.
- **Sem chave do modelo** a rota responde `503 turno_indisponivel` (o turno não está ligado neste
  servidor), em vez de abrir um turno que nunca responderia.
- **Anexos** passam pelo recebimento da HR-23: 16 MB por arquivo, tipo real pelo conteúdo,
  extração do documento em processo isolado. O texto extraído entra no prompt como conteúdo não
  confiável; **imagem** (JPEG/PNG/GIF/WebP) vai ao modelo como parte multimodal (até 4 por turno,
  5 MB por imagem, 8 MB somados — `docs/TURNO.md` §13). Arquivo recusado responde `400
  validation_error` com `details.field = "file"`.
- **Isolamento:** token de outra org recebe `403` em todas as rotas; conversa de outro agente
  recebe `404`.
- O servidor não segura o fluxo atrás de proxy: a resposta leva `x-accel-buffering: no` e
  `cache-control: no-cache`, e um `ping` a cada 10 s mantém a conexão viva na janela de agrupamento.

## Estado da prova (o que cada teste garante)

Paridade é "mesma operação pelos dois caminhos chega ao mesmo estado no banco/JSON", não "os dois
responderam 200". Todos os testes abaixo estão em `crates/poph-server/tests/api.rs`.

| Recurso | Prova de paridade CLI×API | O que é comparado |
| --- | --- | --- |
| Organizações | `org_and_token_cli_api_parity` | criar, alterar e excluir: campos do JSON e ausência no banco depois da exclusão |
| Tokens | `org_and_token_cli_api_parity` | criar, revogar (delete) e ler: rótulo e situação |
| Categorias | `category_and_department_cli_api_parity` | criar, alterar e excluir: nome, descrição e ausência depois da exclusão |
| Departamentos | `category_and_department_cli_api_parity` | criar, alterar e excluir: nome, descrição e ausência depois da exclusão |
| Agentes | `agent_cli_api_parity` | criar, alterar e excluir: campos do agente e ausência no banco depois da exclusão |
| Instruções | `instructions_api_parity_isolation_and_audit` | gravar, alterar e apagar: campos, JSON estruturado e markdown composto, pelos dois formatos de entrada |
| Base de conhecimento (grupos e itens) | `knowledge_cli_api_create_update_delete_parity` | criar, alterar e excluir grupo e item: campos gravados e ausência depois da exclusão |
| Base × agente (vínculo e reindexação) | `agent_knowledge_group_link_cli_api_parity`, `reindex_cli_api_parity` | vínculo lido do banco pelos dois caminhos; reindexação cruzada (CLI e depois API, e o inverso) devolvendo o mesmo relatório de índice |
| Produtos | `product_cli_api_persistence_parity` | criar, alterar e excluir: preço em centavos, categoria, situação e vínculo |
| Biblioteca de arquivos | `library_cli_api_upload_parity`, `library_cli_api_lookup_and_delete_parity` | envio: nome, descrição, mime, tamanho, `sha256` e caminho do blob iguais; consulta e exclusão: mesmos campos e ausência no banco |
| Integrações | não se aplica (exceção) | a CLI não tem o recurso; a API é a referência (`integration_lifecycle_isolation_and_secrets`) |
| Conversas, mensagens, turnos e `reopen` | não se aplica (exceção) | a CLI não tem comando de conversa; provado por `conversations_messages_and_turns_expose_org_scoped_data` |
| Chat de teste (SSE, anexos, cancelamento, duas abas) | não se aplica (exceção) | a CLI não tem comando de chat de teste; provado por `crates/poph-server/tests/test_chat.rs` |
| Conteúdo de arquivo (biblioteca e anexo) | não se aplica (exceção) | a CLI lê o blob local e não tem comando de chat de teste; provado por `crates/poph-server/tests/file_content.rs` |

Outras provas exigidas pela HR-21, cada uma num teste só:

- **Conflito de versão (412) por recurso editável:** `all_updates_reject_stale_version`
  percorre todas as edições publicadas (org, categoria, departamento, agente, instruções, integração,
  grupo e item da base, produto e `reopen`) e exige `412 version_conflict` em cada uma.
- **Isolamento por org de toda rota publicada:** `org_isolation_covers_all_published_operations`
  compara o conjunto de operações do OpenAPI com uma lista explícita e chama cada uma com token da
  org A contra a org B, exigindo 403/404. Rota publicada nova sem entrada na lista faz o teste falhar.
  As duas operações globais de org (`GET` e `POST /admin/v1/orgs`, que só o token global alcança)
  ficam fora dele e são provadas por `org_token_lists_only_its_org` e
  `org_token_cannot_create_org_or_token`.
- **Auditoria de toda escrita publicada:** `all_openapi_writes_require_audit_coverage`
  compara as escritas do OpenAPI com uma lista explícita; rota de escrita nova sem cobertura faz o
  teste falhar.

## O que a HR-21 entrega e o que não entrega

**Entrega:** administração por HTTP (e pelo cliente TypeScript gerado) de orgs, tokens, categorias,
departamentos, agentes, instruções, base de conhecimento (grupos, itens, vínculo ao agente e
reindexação), produtos, integrações (ciclo de vida), biblioteca de arquivos (envio, listagem, consulta
e exclusão) e consulta ao histórico (conversas, mensagens, turnos e reabertura de conversa
transferida). Toda escrita é auditada com ator identificado; toda edição tem controle de versão; toda
coleção é paginada por cursor; todo erro sai no formato `{code, message, details}` — inclusive 404 e
405.

**Não entrega:**

- **Configuração de integração.** `POST`/`PATCH` recusam qualquer chave em `config_json` com
  `422 manifest_indisponivel`: sem o manifesto confiável vindo do servidor, um campo marcado como
  segredo poderia ser gravado como público. Segredo de integração, cifra e nome da integração entram
  quando o registro de manifesto existir — o Console vai precisar disso.
- **Edição de metadados da biblioteca de arquivos.** O contrato é enviar, listar, mostrar e excluir
  (nem a CLI nem o esquema têm versão de arquivo). Editar nome/descrição exige coluna nova e migração
  aprovada — tarefa própria.
- **Versionamento do histórico.** Mensagens e turnos são só de leitura. As escritas em conversa são
  `reopen` e `close` (MX-06), ambas com `If-Match`.
- **Cluster, Console e retenção.** Ficam nas tarefas seguintes (HR-50 em diante);
  `poph serve` sobe um processo só.

## Manifesto de plugin e configuração de integração (HR-44c)

Uma integração de plugin só pode ser configurada se o servidor conhecer o **manifesto** daquele
`kind` — é o manifesto que diz quais campos existem, quais são obrigatórios e quais são segredo. O
manifesto nasce no handshake do plugin (`docs/PLUGIN_PROTOCOLO.md`) e a instalação declara onde ele
mora:

```toml
[[plugins.integracoes]]
kind = "eco"
programa = "/opt/poph/plugins/poph-plugin-eco"
# argumentos = []
```

**Rota do manifesto:** `GET /admin/v1/orgs/{org_id}/manifests/{kind}` devolve o manifesto inteiro
(`nome`, `kind`, `versao`, `protocolo`, `capacidades`, `limites`, `configuracao`, `segredos`), com os
dois últimos no formato de JSON Schema. Erros:

| Situação | Resposta |
| --- | --- |
| `kind` sem plugin declarado | `404 not_found` (`details.kind`) |
| plugin declarado que não responde ao handshake | `503 manifesto_indisponivel` |
| token de outra org | `403 forbidden` |

O manifesto é resolvido **uma vez por processo**: o servidor sobe o plugin pelo supervisor, lê o
manifesto, encerra o processo e guarda o resultado em memória (falha fica em cache por 30 s). Mudou
o plugin, reinicie o servidor. A rota é de leitura, não audita e não devolve dado de cliente — o
escopo da org existe só para herdar o escopo de token das outras rotas.

**Lista dos plugins declarados (HR-48b).** `GET /admin/v1/orgs/{org_id}/manifests` responde quais
tipos existem, para a tela não precisar adivinhar o `kind`:

```json
{ "items": [
  { "kind": "eco", "situacao": "ok", "nome": "Eco", "versao": "0.1.0" },
  { "kind": "whatsapp", "situacao": "indisponivel",
    "mensagem": "o plugin deste tipo não respondeu ao handshake; verifique o programa declarado em `[plugins].integracoes`" }
] }
```

- Um item por `kind` declarado, na ordem do `kind` (determinística). Não pagina: é config **da
  instância**, não cresce com o uso — por isso a lista não tem `next_cursor` nem `?limit`.
- `situacao` é `ok` (manifesto lido: `nome` e `versao` preenchidos) ou `indisponivel` (plugin
  declarado que não respondeu: `nome`/`versao` ausentes e `mensagem` com a frase de operação).
  **Um plugin quebrado não derruba a lista** — o item sai marcado e os outros respondem.
- A resolução é a mesma da consulta por tipo e usa o mesmo cache: a primeira listagem sobe cada
  plugin uma vez; as seguintes leem da memória (inclusive a falha, por 30 s).
- O item traz só metadado do plugin. **Nunca** caminho do programa, argumentos, `config_json`,
  segredo ou caminho de blob.
- `401` sem token; `403` com token de outra org (a lista é da instância, mas o caminho é da org,
  como as demais rotas — a org não é inferida do token).

**Configuração e segredo no `POST`/`PATCH` de integração.** Com manifesto disponível:

```json
{
  "kind": "eco",
  "config_json": { "remetente": "Loja", "modo": "inverter" },
  "secrets": { "token": "..." }
}
```

- `config_json` é validado contra o `configuracao` do manifesto; `secrets` contra o `segredos`.
  Campo obrigatório ausente, tipo errado, valor fora do `enum`, número fora da faixa, texto fora de
  `minLength`/`maxLength` ou campo que não está no esquema → `400 validation_error`, com
  `details.field` no caminho (`config_json.remetente`, `secrets.token`, e aninhado quando houver).
- Sem manifesto (tipo não declarado, como `whatsapp`), `config_json`/`secrets` continuam recusados
  com `422 manifest_indisponivel`; integração **sem** configuração é criada como antes.
- Com manifesto, a criação já é validada: o que o esquema exige é cobrado na hora, mesmo que o
  corpo não traga configuração.
- `config_json` no `PATCH` **substitui** a configuração inteira; em `secrets`, só os nomes enviados
  mudam (`""` remove). Não dá para remover um segredo obrigatório.
- **Segredo nunca volta:** a resposta traz `tem_segredo` e `segredos_preenchidos` (nomes). O valor é
  gravado cifrado em `integrations.secrets_enc` (AES-256-GCM, chave derivada por HKDF-SHA256 de
  `POPH_CHAVE_CIFRA`) e só sai do processo no handshake do plugin.
- **Sem a chave de cifra** o servidor não grava segredo: `503 cifra_indisponivel`. Se a variável
  mudar, os segredos antigos ficam ilegíveis (`tem_segredo: true`, `segredos_preenchidos: null`) —
  regrave-os.
- `config_json` volta na leitura como `configuracao`; toda escrita exige `If-Match` (428/412) e sobe
  a versão.

## Exportar e importar a organização (HR-48e-1)

Duas rotas para o Console fechar a parte de operação. O **formato do arquivo é o mesmo** da CLI
(`poph backup export`/`import`, HR-53): quem gera um lado abre no outro.

### Exportar — `GET /admin/v1/orgs/{org_id}/backup`

- **Sem `com_conversas`** (padrão) leva a configuração da org, o catálogo (categorias,
  departamentos, produtos), os agentes com instruções e vínculos, a base de conhecimento, a
  biblioteca de arquivos e as integrações. **Com `com_conversas=true`** leva também conversas,
  mensagens, anexos, turnos, as duas filas e os carrinhos.
- Resposta: o arquivo JSON (`Content-Type: application/json`,
  `Content-Disposition: attachment; filename="<slug>-backup-<AAAAMMDD-HHMM>.json"`,
  `X-Content-Type-Options: nosniff`, `Cache-Control: private, no-store`). O carimbo é UTC.
- **Segredo nunca sai:** a integração leva `tem_segredo` e os **nomes** dos campos preenchidos
  (`segredos_campos`); o valor e o blob cifrado (`secrets_enc`) ficam no banco. Sem a chave de
  cifra, `segredos_campos` vem `null` e só a marca diz que havia segredo.
- **Caminho de blob nunca sai:** o arquivo guarda `sha256` e o conteúdo em base64; o caminho é
  recalculado na importação.
- `com_conversas` com valor que não é `true`/`false` → `400 validation_error`.
- Arquivo da biblioteca sem conteúdo no disco → `409 backup_incompleto` (nada é entregue pela
  metade); o nome do arquivo vai em `details.name` e o caminho não aparece.
- **Acesso:** sem token → `401`; token de outra org → `403`. Vale para as duas rotas e é checado
  antes de ler o corpo.

### Importar — `POST /admin/v1/orgs/{org_id}/backup/import`

- **Corpo cru** `application/json` com o arquivo de `…/backup` (ou de `poph backup export`), até
  **64 MB**; acima disso → `413 payload_too_large`, recusado sem carregar o corpo inteiro. Não é
  multipart.
- **A org do caminho é o destino** e precisa existir (`404` se não existir). Os ids do arquivo são
  preservados; importar não duplica nem **apaga** o que não está no arquivo.
- **Sobrescrita é recusada por padrão (HR-48e-3):** se alguma linha do arquivo já existe na org
  (localizada pela chave da tabela), a importação **não grava nada** e responde
  `409 importacao_sobrescreve`, com `details.tabelas` (tabela → linhas), `details.linhas` (total) e
  `details.exemplos` (até 5, no formato `tabela: slug` ou `tabela: id`). É o caso de reimportar um
  arquivo antigo, que reverteria o que mudou depois dele. `?sobrescrever=true` autoriza a
  atualização; o relatório então mostra `atualizadas` por tabela. Destino limpo (org sem essas
  linhas) entra sem o parâmetro; `sobrescrever` com valor que não é `true`/`false` → `400
  validation_error`.
- **Conflito recusa antes de gravar:** se o destino já tem agente/produto/categoria/departamento/
  grupo com a mesma chave e outro id — ou se um id do arquivo já existe na instância em **outra**
  org — a resposta é `409 conflict`, com até 5 itens em `details.conflicts` e o resto em
  `details.conflicts_more`, e **nada** é gravado. Conflito de chave natural tem precedência sobre
  `importacao_sobrescreve` e é recusado **mesmo com `?sobrescrever=true`** (não há "merge").
- **Arquivo inválido** (JSON truncado, outro formato, `versao_formato` desconhecida) → `400
  invalid_backup`, sem gravar nada.
- **Segredos:** a integração entra sem o valor do segredo; o relatório traz
  `segredos_a_redigitar` e o valor precisa ser digitado de novo na aba Integrações.
- **Reindexação automática:** com o modelo de embedding instalado, a base e o catálogo da org
  ficam buscáveis na hora (`indice` no relatório). Sem o modelo, o material entra, `indice` vem
  zerado e `aviso` diz o que rodar (`poph kb reindex`, `poph product reindex`) — nunca um índice
  vazio em silêncio.
- **Relatório (`200`):**

```json
{
  "org": { "id": "0192…", "criada": false },
  "com_conversas": false,
  "tabelas": { "agents": { "inseridas": 1, "atualizadas": 0 }, "…": {} },
  "total": 9,
  "segredos_a_redigitar": 1,
  "indice": { "base_conhecimento": { "agentes": 1, "trechos": 2 }, "produtos": { "trechos": 1 } },
  "aviso": null
}
```

- A importação é auditada (`backup_import`); a exportação é leitura e não gera registro.

## WhatsApp da integração: pareamento, estado e desconexão (HR-44b)

Três rotas de admin para a integração de `kind = "whatsapp"`. Elas falam com a instância da
EvolutionAPI correspondente — a mesma que recebe o webhook.

| Operação | Rota | Efeito |
| --- | --- | --- |
| gerar QR | `POST …/whatsapp/qrcode` | cria a instância na Evolution quando ela não existe (reaproveita quando existe) e pede o pareamento |
| estado | `GET …/whatsapp/state` | pergunta o estado ao vivo e grava em `connection_state` |
| desconectar | `POST …/whatsapp/disconnect` | faz o logout do aparelho e grava `connection_state = disconnected` |

**Onde mora a configuração.** Cada integração precisa estar declarada em `[evolution].instancias`
(`org_id`, `instancia`, `integracao_id`, `segredo_env`); o endereço vem de `[evolution].url_base` e
a chave de `EVOLUTION_API_KEY` no ambiente. Sem a declaração, as três rotas respondem
`409 whatsapp_nao_configurado`; sem a chave, `503 evolution_indisponivel`. A rota não inventa nome
de instância.

**Corpo do pareamento** (opcional): `{"number": "5511999998888"}` — com número, a Evolution também
devolve o código de 8 caracteres. Resposta:

```json
{
  "integration_id": "...", "instance": "loja",
  "state": "connecting", "connection_state": "awaiting_scan",
  "qrcode": { "base64": "data:image/png;base64,...", "code": "2@...", "count": 3 },
  "expira_em": null,
  "pairing_code": null
}
```

`expira_em` é **sempre `null`**: a Evolution v2 não informa validade do QR — ele gira sozinho a
cada ~20 s e o `count` é o único sinal de que mudou. Quem desenha o QR pede outro quando o cliente
demorar. Com o aparelho já conectado (`state = open`), a rota responde `qrcode: null` e não pede
pareamento de novo.

**Estado:** resposta `{integration_id, instance, state, connection_state}`, com `state` no
vocabulário da Evolution (`open`, `connecting`, `close`) e `connection_state` no nosso
(`connected`, `awaiting_scan`, `disconnected`; valor fora desse vocabulário vira `desconhecido`).

**Concorrência:** `qrcode` e `disconnect` exigem `If-Match` (428 sem ele, 412 `version_conflict`
com versão velha), mas **não sobem a versão**: `connection_state` é estado de execução, não edição
de configuração — a mesma regra do webhook. Quem está com a tela aberta pode gerar QR quantas vezes
quiser com a mesma versão; se outra pessoa editar a integração no meio, a próxima escrita dá 412.

**Segredo:** a chave da Evolution vai só no cabeçalho `apikey` e não aparece em resposta, log nem
mensagem de erro — o texto do provedor é limpo pelo cliente (sem chave, sem número/JID). A resposta
não traz configuração nem segredo.

**Sem CLI:** a CLI não tem subcomando de integração, então a paridade destas rotas é provada só por
HTTP (`crates/poph-server/tests/whatsapp_admin.rs`, com Evolution falsa).

## Conteúdo de arquivo: biblioteca e anexo (HR-46b)

Duas rotas entregam os **bytes** do arquivo. As demais rotas de arquivo entregam só metadados; sem
estas, o Console não tinha como baixar nem pré-visualizar o arquivo que o agente pode enviar ao
cliente nem o anexo do histórico do chat de teste.

| Operação | Rota |
| --- | --- |
| arquivo da biblioteca | `GET /admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}/content` |
| anexo de mensagem | `GET /admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/messages/{message_id}/attachments/{attachment_id}/content` |

**Resposta:** os bytes crus, com `Content-Type` do MIME gravado, `Content-Length`,
`Content-Disposition` com o nome sanitizado (`filename="..."` em ASCII e `filename*=UTF-8''...` com
o nome original), `ETag` e `x-content-sha256` com o SHA-256 do conteúdo, mais
`X-Content-Type-Options: nosniff` e `Content-Security-Policy: default-src 'none'; sandbox` — o
navegador não executa nada do arquivo. Imagem e PDF saem `inline` (o Console mostra no lugar);
os outros formatos saem `attachment`.

**Escopo:** o caminho inteiro. Arquivo é alcançado por org + agente + id; anexo por org + agente +
conversa + mensagem + id do anexo. Id de outra org, de outro agente da mesma org, de outra conversa
ou inexistente responde `404 not_found` — a resposta não confirma se o registro existe em outro
lugar.

**Leitura do blob:** sempre pelo `poph_files::blob::ler`, que valida o hash (64 hex minúsculos)
antes de tocar a pasta e confere o tamanho; o handler nunca monta caminho. Blob ausente no disco,
hash inválido ou blob acima do teto de recebimento (`FileConfig::byte_limit`, 16 MB por
padrão) viram `404 not_found` com mensagem genérica — o motivo real fica só no log, nunca o caminho
do blob na resposta.

**Metadados sem caminho interno:** a resposta das rotas de metadados (`…/files` e `…/files/{file_id}`)
continua **sem** `blob_path`; a CLI obtém o caminho lendo o próprio banco local (`poph file show`),
não a API.

**Sem CLI:** não há subcomando que baixe por HTTP; a CLI lê o blob direto do disco. Por isso as duas
rotas estão no inventário como exceção, e a prova é o teste de servidor
(`crates/poph-server/tests/file_content.rs`).

## Roteamento entre instâncias: mapa e recarga (HR-50a)

O mapa de qual org vive em qual instância é o `cluster.toml` (PRD §12), **separado do
`poph.toml`** e igual em todas as instâncias. Formato, validação e comandos estão em
`docs/CLUSTER.md`. Aqui fica só o contrato da rota de recarga.

`POST /admin/v1/cluster/reload` **exige token global** (é configuração da instalação, não de uma
org): token de org → `403 forbidden`; sem token → `401 unauthorized`. Não tem corpo.

- **Relê o arquivo no processo vivo** (o caminho vem de `POPH_CLUSTER` ou do `cluster.toml` ao lado
  do `poph.toml`) e troca o mapa em uso de uma vez.
- **Arquivo inválido → `400 cluster_invalido`**, com `details.problemas` (uma entrada por problema,
  cada uma dizendo a chave) e `details.arquivo`; **o mapa anterior continua em uso**.
- **Arquivo ausente** volta ao modo instância única (é o que a recarga reflete).
- Resposta `200`:

```json
{
  "mudou": true,
  "fingerprint_antes": "5b0c…",
  "fingerprint_depois": "0a91…",
  "antes": { "modo": "cluster", "instance_id": "a", "default_instance": "b",
             "segredo_configurado": true,
             "instancias": [{ "id": "a", "url": "http://127.0.0.1:7480", "local": true }],
             "orgs": { "org_01": "a" } },
  "depois": { "…": "…" },
  "orgs_que_trocaram": [{ "org": "org_01", "de": "a", "para": "b" }],
  "instancias_adicionadas": ["b"],
  "instancias_removidas": []
}
```

- O **fingerprint** é estável: mesmo mapa, mesma impressão (a ordem do arquivo não importa); org
  trocada de instância muda a impressão. É o que a HR-50b compara entre instâncias.
- O **segredo compartilhado** (`POPH_CLUSTER_SECRET`) é lido na subida; a resposta só diz se está
  configurado (`segredo_configurado`), nunca o valor. Trocar o segredo exige reiniciar o processo.
- A recarga **não** é auditada: não grava dado de org nem de cliente; o registro é o log de subida
  e a resposta com antes/depois.

## Roteamento entre instâncias: encaminhamento (HR-50b-1)

Com o mapa do `cluster.toml` valendo (HR-50a), um pedido de org que vive em **outra** instância é
encaminhado à dona, assinado. A partir da HR-50b o mapa deixou de ser só projeção: uma org fora do
`[orgs]` vai para `default_instance`, e é isso que o encaminhamento aplica.

**O que é encaminhado:** `POST /webhook/evolution/{org_id}/{instancia}` e todas as rotas
`/admin/v1/orgs/{org_id}/…`. **O que fica local, sempre:** `/healthz`, `/metrics`, `/openapi.json`,
`/admin/v1/cluster/reload`, `/admin/v1/orgs` (sem org no caminho, como criar/listar orgs) e o
Console — nenhuma delas tem org no caminho.

**Cabeçalhos de par** (nunca presentes num pedido de cliente comum):

| Cabeçalho | Conteúdo |
| --- | --- |
| `x-poph-instance` | `instance_id` de quem encaminha |
| `x-poph-timestamp` | instante do envio, em segundos Unix |
| `x-poph-signature` | HMAC-SHA256 em hexadecimal |
| `x-poph-hop` | `1` no primeiro (e único) salto |
| `x-poph-mapa` | impressão digital **do mapa** (sem a instância local) de quem encaminha |
| `x-poph-nonce` | 16 bytes aleatórios em hexadecimal (32 caracteres), um por pedido |

Texto assinado, com a chave `POPH_CLUSTER_SECRET` (linhas separadas por `\n`, sem linha no fim):

```text
método
caminho+query
hex(sha256(corpo))
timestamp
instância
hop
nonce
```

- O `Authorization` original segue junto: o token de org continua valendo **na dona**, que confere o
  escopo como sempre. Token de org nunca é aceito em outra instância por causa do encaminhamento.
- Janela aceita para o carimbo: **60 s** para trás e para frente. O **nonce** é o que torna cada
  pedido único: dois pedidos idênticos no mesmo segundo passam os dois; o mesmo nonce recebido de
  novo dentro de 120 s é recusado (`401 assinatura_invalida`) — proteção contra repetição, em
  memória do processo (teto de 10.000 nonces; acima disso a memória é zerada com aviso no log).
  Nonce ausente ou fora do formato de 32 hexadecimais é `401 assinatura_invalida`.
- `x-poph-hop > 1` → `421 rota_em_laco`. Org que não pertence a esta instância segundo o mapa, num
  pedido que já chegou encaminhado → `421 rota_em_laco`.
- `x-poph-mapa` diferente do mapa local → `421 mapa_divergente`, com os dois fingerprints em
  `details`. É o que impede encaminhar com mapas desatualizados (laço por mapa velho).
- Dona fora do ar (conexão recusada, prazo de 5 s para conectar ou 60 s para começar a responder) →
  `503 instancia_fora`, com `details.instancia` e `details.url`, sem caminho interno nem segredo.
- A **resposta** da dona volta em streaming (status, cabeçalhos e corpo), inclusive o **SSE do chat
  de teste**: o primeiro evento chega sem esperar o fim do turno, e cliente que desconecta cancela o
  turno na dona (o pedido encaminhado é abandonado). O **corpo do pedido** é lido inteiro antes de
  seguir, porque a assinatura usa o sha256 dele; o teto é o da própria rota (webhook 1 MB, mensagem
  do chat de teste 24 MB, importação de backup 64 MB, demais 2 MB).
- `GET /admin/v1/orgs?cluster=true` (token global): além das orgs desta instância, consulta as
  vizinhas do mapa em paralelo (`/admin/v1/orgs`, com o `Authorization` do chamador, prazo de **2 s**
  por instância) e devolve todas em `items`, cada uma com `instancia`; `instancias` traz `id`, `url`,
  `situacao` (`local`, `ok` ou `inacessivel`) e quantas orgs vieram de cada uma. Instância fora do
  ar aparece marcada e **não** derruba a lista. Sem o parâmetro, a resposta é exatamente a de antes
  (sem `instancias` e sem `instancia` nos itens); com ele não há paginação (`limit`/`cursor` são
  ignorados, `next_cursor` é `null`). Token de org ignora o parâmetro e continua vendo só a própria
  org.
- `agents_active_count` (CF-03a): todo item da listagem de orgs traz esse inteiro, nas duas formas
  (com e sem `cluster=true`) e também em `GET /admin/v1/orgs/{org_id}` — é quantos agentes da org
  estão com `status = active` (`inactive`/`archived` não contam; org sem agente ativo devolve `0`).
  A contagem de uma página sai de **uma** consulta agregada (`org_id IN (…) GROUP BY org_id`), nunca
  de uma consulta por linha. No `cluster=true` o número vem no payload da própria instância dona da
  org; instância de versão anterior que ainda não devolva o campo aparece com `0`.
- `/healthz` no modo cluster passa a devolver `instance_id` e `cluster_map_fingerprint` (aditivo;
  ausentes no modo instância única) — é o que o `poph cluster check --remoto` compara entre as
  instâncias.
- Instância que recebe um pedido com qualquer cabeçalho de par sem ter segredo compartilhado
  configurado responde `401 assinatura_invalida`.

## Rota pública fora do admin: webhook da Evolution (HR-26b)

`POST /webhook/evolution/{org_id}/{instancia}` **não faz parte** da API administrativa: não usa
token de administração, não aparece no OpenAPI (`/admin/v1/openapi.json`) e fica fora do inventário
acima e dos testes de isolamento e auditoria gerados dele. Quem autentica é o segredo do webhook de
cada instância, no cabeçalho `X-Evolution-Webhook-Secret`. Comportamento completo em
`docs/EVOLUTION.md`, seção "Entrada de mensagens".

## Application Service do Matrix: as rotas que o Synapse chama (MX-16)

Estas rotas **não** fazem parte da API administrativa: não usam token de org, não aparecem no
OpenAPI (`/admin/v1/openapi.json`), não estão no inventário acima e o Console nunca as vê. Quem
chama é o Synapse, do outro lado do Application Service.

| Método | Rota | O que é |
| --- | --- | --- |
| PUT | `/_matrix/app/v1/transactions/{txnId}` | lote de eventos de uma sala (é o caminho da resposta do atendente) |
| GET | `/_matrix/app/v1/users/{userId}` | "este usuário é seu?" |
| GET | `/_matrix/app/v1/rooms/{roomAlias}` | "este alias é seu?" |
| POST | `/_matrix/app/v1/ping` | existência do serviço (o Synapse não usa hoje) |

As mesmas quatro rotas também respondem sem o `/v1` (`/_matrix/app/transactions/…`), na raiz
(`/transactions/…`) e com o prefixo `/matrix` na frente (`/matrix/_matrix/app/v1/…` e
`/matrix/_matrix/app/…`), por compatibilidade com registro antigo.

O prefixo existe porque o registro do Application Service no Synapse de desenvolvimento — e o de
produção, que ele reflete — aponta para uma base com `/matrix`, e o Synapse chama
`<url>/_matrix/app/v1/…`: sem essas rotas a entrega respondia `404` e a resposta do atendente nunca
chegava ao cliente (defeito achado no aceite, MX-31). As duas famílias respondem igual — mesmo
`hs_token`, mesma idempotência por `txnId`, mesmos `200`/`503` — e **o registro do Synapse não muda**.

- **Autenticação:** `Authorization: Bearer <hs_token>` (`POPH_MATRIX_HS_TOKEN`), comparado em tempo
  constante. Sem cabeçalho, sem `Bearer` ou com token vazio → `401 unauthorized`; token diferente →
  `403 forbidden`. O token nunca aparece em resposta, log ou erro.
- **Módulo desligado** (`[matrix]` ausente ou `habilitado = false`, ou nenhum receptor injetado) →
  `404 not_found` nas quatro rotas, e não a página do Console.
- **Contrato de resposta da transação** (medido no spike da MX-02): qualquer resposta não-2xx marca o
  Application Service como fora do ar no Synapse, que passa a reenviar com espera de 2 s, 4 s, 8 s… até
  512 s. Por isso:

| Situação | Resposta |
| --- | --- |
| transação processada | `200 {}` |
| transação já processada (mesmo `txnId`) | `200 {}` sem reprocessar |
| corpo acima de 1 MiB, JSON inválido ou sem `events` | `200 {}` sem processar (nunca `413`/`400`) |
| erro transitório nosso (banco indisponível) | `503 matrix_indisponivel` — o Synapse reenvia |
| token ausente ou errado | `401`/`403` (erro de configuração, some quando o token é corrigido) |

- **Idempotência:** a memória de `txnId` já processados é por processo, FIFO e limitada a 4096
  entradas (`TETO_PADRAO_TRANSACOES`). Uma transação é marcada **só depois** de processada, então uma
  falha nossa nunca fica "lembrada" como feita. Se a memória esquecer uma transação antiga, o pior caso
  é reprocessar (a gravação da resposta do atendente é idempotente por `event_id`).
- **Tamanho do corpo:** o teto é aplicado na leitura (1 MiB), não pelo `axum`, justamente para não
  responder `413` ao Synapse.
- **O que está ligado nesta entrega:** receber, autenticar, validar, deduplicar e entregar os eventos a
  quem liga o módulo (MX-17/MX-18 fazem a classificação e a gravação; MX-20 liga no `poph serve`).

### Ligação no `poph serve` (MX-20)

Com `[matrix] habilitado = true` e os dois tokens no ambiente, o servidor sobe o módulo na própria
subida (`matrix_inicio::iniciar`, chamado por `servir_em`): laço das mensagens (MX-14), laço do estado
(MX-15), receptor do Application Service (MX-16) com a entrega da resposta do atendente (MX-18) e as
métricas `poph_matrix_*` (MX-26).

| Situação | O que acontece |
| --- | --- |
| `[matrix]` ausente ou `habilitado = false` | nenhuma tarefa do módulo, nenhuma `poph_matrix_*` e as rotas do Synapse em `404` |
| módulo ligado sem `POPH_MATRIX_AS_TOKEN`/`POPH_MATRIX_HS_TOKEN` | o módulo **não** sobe, o log diz `token_ausente` e o servidor atende normalmente (rotas em `404`) |
| Synapse fora do ar na subida | o servidor sobe e atende; o módulo fica degradado e retenta (sonda a cada 30 s, espera dobrando até 60 s) |
| org desligada em `matrix_orgs` (API da MX-22 ou CLI) | o trabalho daquela org para até o ciclo seguinte, sem reiniciar o processo |
| `Ctrl-C` | os laços terminam o que estão fazendo (prazo de 10 s) antes de o processo sair; nada pendente se perde |

O estado da conexão com o Synapse não é publicado em `/metrics` nesta entrega. Ele fica no `Estado`:
`matrix_conectado()` devolve `Some(true)` (conectado), `Some(false)` (degradado, o servidor segue
atendendo e os laços retentam) ou `None` (módulo desligado nesta instância). É o que a API
administrativa da MX-22 lê como "conectado".

## API do módulo Matrix: estado da org e espaços por departamento (MX-22)

Estas rotas fazem parte da API administrativa (`/admin/v1`) e do OpenAPI. Token de org opera só a
própria org (token da org A em rota da org B → `403`); sem token é `401`; org inexistente é `404`.
**Nenhuma delas muda a configuração do homeserver** — isso é `poph.toml` + ambiente.

A mesma operação está na CLI, sem token e sem servidor (`--offline --org <org>`, MX-23): `poph matrix estado`,
`poph matrix ligar`, `poph matrix desligar`, `poph matrix espaco` e `poph matrix espaco desligar`. Por isso a
coluna CLI das cinco rotas é `sim`.

### `GET /admin/v1/orgs/{org_id}/matrix`

Estado do módulo na org, com `ETag` igual ao `updated_at` da linha:

```json
{
  "org_id": "org_…",
  "enabled": true,
  "enabled_at": 1795300000000,
  "updated_at": 1795300000000,
  "connected": true,
  "backlog": { "messages": 12, "oldest_seconds": 340 },
  "pending_failures": 1,
  "last_failure": "formato_invalido"
}
```

| Campo | O que é |
| --- | --- |
| `enabled` | `matrix_orgs.enabled`: o módulo trabalha esta org |
| `enabled_at` | corte do backfill (só o que foi gravado depois dele é espelhado); `null` se a org nunca ligou |
| `updated_at` | última alteração da linha — é o valor do `If-Match` |
| `connected` | sonda da instância contra o Synapse: `true` conectado, `false` degradado, `null` módulo não configurado aqui |
| `backlog.messages` | mensagens gravadas depois do cursor e ainda não espelhadas (mesmo filtro do laço: só `user`/`assistant`) |
| `backlog.oldest_seconds` | idade da mais antiga delas; `0` quando não há nenhuma |
| `pending_failures` | linhas de `matrix_falhas` da org |
| `last_failure` | motivo curto (≤ 64 caracteres) da falha mais recente, ou `null` |

Nada de telefone, texto de cliente ou token aparece aqui: são ids, estados, números e o código curto
do erro.

### `PUT /admin/v1/orgs/{org_id}/matrix`

Corpo `{"enabled": true}` ou `{"enabled": false}`, com `If-Match` obrigatório (`updated_at` lido no
`GET`). Devolve o mesmo JSON do `GET`.

| Situação | Resposta |
| --- | --- |
| sem `If-Match` | `428 precondition_required` |
| `If-Match` diferente do `updated_at` atual | `412 version_conflict` |
| ligar | grava `enabled = 1` e um `enabled_at` novo (o corte do backfill) |
| desligar | grava `enabled = 0`, **preserva** `enabled_at`, não apaga sala nem espaço |
| repetir o mesmo pedido com o `If-Match` atual | `200`, sem mudar `enabled_at` (idempotente) |

A escrita registra auditoria (`action: update`, `entity: matrix`). O efeito nos laços é no ciclo
seguinte: desligar não aborta o item em voo.

### `GET /admin/v1/orgs/{org_id}/matrix/spaces`

Lista os espaços por departamento: `{"items": [{"department_id", "department_name",
"space_room_id", "alias", "origin", "created_at", "updated_at"}]}`. `origin` é `auto` (o módulo criou
o espaço) ou `linked` (já existia no Element e foi ligado ao departamento). Num espaço ligado, o
`alias` é o nome do departamento **para o módulo**; quem deu alias no Element foi o operador.

### `PUT /admin/v1/orgs/{org_id}/matrix/spaces`

Corpo `{"department_id": "…", "space": "!sala:servidor" | "#alias:servidor" | null}`:

- `space` ausente ou `null` → o módulo **cria** o espaço do departamento pelo alias
  `#dep_<id do departamento>:<servidor>` e escreve nome, tópico e o evento
  `com.sofacoding.departamento`;
- `space` com id (`!…`) ou alias (`#…`) → **liga** um espaço que já existe, conferindo que o bot do
  Application Service (`@<prefixo>_as_bot`) é membro **e** tem poder para escrever `m.space.child`
  (o `state_default` do espaço, 50 no padrão do Matrix). Faltando qualquer um, a resposta é `409
  conflict` com a instrução do que fazer no Element (convidar o bot e dar poder 50); nada é gravado.

Repetir o `PUT` do mesmo departamento devolve o mesmo espaço (o alias é a trava; não há segundo
espaço). Departamento de outra org → `404`. Espaço que não é id nem alias → `400 validation_error`.
Synapse fora do ar → `503 synapse_indisponivel` (tentar de novo resolve). Falha do armazenamento →
`503 matrix_indisponivel`. Módulo não configurado nesta instância → `409 matrix_desligado`.

### `DELETE /admin/v1/orgs/{org_id}/matrix/spaces/{department_id}`

Desliga o espaço do departamento. **Nada é apagado no Synapse**: o espaço continua existindo no
Element e as salas também. O que acontece, exatamente:

1. a linha de `matrix_spaces` daquele departamento sai;
2. as salas da org registradas naquele espaço têm `matrix_rooms.space_room_id` zerado
   (`rooms_forgotten` na resposta) — o módulo esquece que elas estão no espaço;
3. as conversas continuam existindo e **não** saem do espaço no Element; o `join_rules` da sala
   também fica como está;
4. o efeito é reversível: a próxima alteração de uma conversa daquele departamento volta a pôr a
   sala no espaço (MX-15), e o alias leva o módulo ao **mesmo** espaço do Element — sem criar um
   segundo.

Sem espaço gravado o `DELETE` responde `200` com `removed: false` (é idempotente). Departamento
inexistente ou de outra org → `404`. A escrita registra auditoria (`action: delete`,
`entity: matrix_space`).

## Fluxos liberados ao agente (FX-28)

Quais fluxos da org cada agente pode usar. O fluxo é da organização; o agente só tem a permissão.
Liberar ou tirar é a lista inteira de uma vez.

- `GET /admin/v1/orgs/{org_id}/agents/{agent_id}/flows` → `200` com `{"items": [{"id", "slug",
  "name", "status"}], "next_cursor": null}`, em ordem de `slug`. Sem paginação (o teto é 20).
- `PUT /admin/v1/orgs/{org_id}/agents/{agent_id}/flows` com corpo `{"flow_ids": ["…"]}` troca a
  lista inteira numa escrita só. Resposta `200` com `{"agent_id", "version", "added", "removed"}`:
  `added` e `removed` são os fluxos que entraram e saíram (`id`, `slug`, `name`, `status`), e
  `version` é a versão nova do agente.

| Situação | Resposta | Efeito |
| --- | --- | --- |
| sem `If-Match` (no `PUT`) | `428 precondition_required` | nada muda |
| `If-Match` diferente da versão atual do **agente** | `412 version_conflict` | nada muda |
| mais de 20 fluxos na lista | `409 teto_de_fluxos_por_agente` | nada muda |
| fluxo de outra org, ou id que não existe | `404 not_found` | nada muda |
| agente de outra org ou inexistente | `404 not_found` | nada muda |
| token de outra org | `403 forbidden` | nada muda |
| `PUT` bem-sucedido, lista igual à anterior | `200` | versão do agente +1; auditoria `agent_flows_update` |

- Id de fluxo que não é UUID v7 → `400 invalid_id`. Corpo sem `flow_ids` → `400 invalid_request`.
- Cada `PUT` bem-sucedido grava uma linha de auditoria (`agent_flows_update`, entidade `agent`) com o
  autor do token. O corpo da lista não entra na auditoria.
- Agente `inactive` continua aceitando a permissão.
- Repetir o mesmo id na lista conta uma vez.
- A versão do agente sobe a cada troca, então quem leu a lista antes e tenta gravar depois recebe
  `412` e relê. Não há "liberar para todos" (decisão em aberto no PRD §15).

Provas: `crates/poph-server/tests/agent_flows.rs` (ponta a ponta, com servidor e banco
temporários). As listas de cobertura de `crates/poph-server/tests/api.rs` (OpenAPI, auditoria,
isolamento por org e documentação da API) também conhecem as duas rotas.

## Fluxos: script, publicação e revisões (FX-27)

Rotas sob `/admin/v1/orgs/{org_id}`, com token da org (`401` sem token, `403` no caminho de outra org).
O fluxo é da organização; o agente só tem a permissão de usar (seção "Fluxos liberados ao agente").
A CLI ainda não tem comandos para isso: entram na FX-31.

| Rota | O que faz |
| --- | --- |
| `POST …/flows/validate` | Compila o script (`{"source": "…"}`) e devolve `valid`, a IR, os `diagnostics` (código, gravidade, mensagem, linha e coluna) e as `resolutions` de departamento. Não grava nada. Script com erro sai como `200` com `valid: false`: validar não recusa |
| `POST …/flows/format` | Devolve o script na forma canônica (`{"source"}`). Script que não compila: `422 fluxo_invalido` |
| `GET …/flows` | Lista da org, `?status=active\|inactive`, `?limit`, `?cursor` (`items`, `next_cursor`) |
| `POST …/flows` | Cria o fluxo a partir do script, `{"source", "publish"}`. `publish` padrão `false` (só rascunho); com `true`, publica a revisão 1 na mesma chamada. `201` com `draft_source` e `revision` (`null` sem publicar) |
| `GET …/flows/{flow_id}` | Detalhe: fluxo, `draft_source` e `revision` (resumo da revisão que vale hoje; `null` se nunca publicou) |
| `PATCH …/flows/{flow_id}` | Exige `If-Match` (versão do fluxo). Muda `name`, `description`, `status` (`active` ou `inactive`) e `source` (o rascunho, que pode ficar inválido: só publicar exige script bom). Só o que vier no corpo muda |
| `DELETE …/flows/{flow_id}?force=` | Exclui. Com atendimento ativo e sem `force=true`: `409 conflict` com `details.active_runs`, nada apagado. Com `force=true`: abandona as execuções ativas, apaga e devolve `abandoned_runs`. Não pede `If-Match` |
| `POST …/flows/{flow_id}/publish` | Exige `If-Match` (versão do fluxo). Corpo opcional `{"source"}`; sem corpo, publica o rascunho. Devolve `201` com `revision`, `version` e `warnings` |
| `GET …/flows/{flow_id}/revisions` | Revisões em ordem de número. `?cursor` é o número da última revisão da página anterior |
| `GET …/flows/{flow_id}/revisions/{number}` | Uma revisão, com o texto publicado, a IR, o hash e as resoluções |
| `GET …/flows/{flow_id}/diff?de=&para=` | Frases do que mudou entre duas revisões e, para cada uma, `active_runs` (quantos atendimentos estão nela agora) |
| `GET …/flows/{flow_id}/preview?etapa=` | O bloco de prompt que o agente veria naquela etapa (padrão: a etapa de início), com a revisão usada |

Regras:

- Publicar com `If-Match` antigo responde `412` antes de compilar. Script com erro responde `422 fluxo_invalido`
  com a lista de diagnósticos. Nos dois casos, nada é gravado. Sem `If-Match`: `428`.
- Duas publicações com a mesma versão: uma vence e a outra leva `412`, sem revisão sobrando.
- Revisão publicada não muda. Publicar de novo cria a revisão seguinte, e a anterior segue idêntica
  (texto e IR).
- A publicação só passa com o script limpo no compilador e na validação semântica. Os nomes de
  departamento citados no script são resolvidos para o id na publicação e ficam gravados.
- Auditoria: `flow_create`, `flow_update` e `flow_delete` em cada escrita; a publicação grava `flow_publish`
  na mesma transação da revisão.

Provas: `crates/poph-server/tests/fluxos_api.rs` (validar, formatar, criar, isolamento, publicar com as
recusas, revisão 2 com a 1 idêntica, diff com contagem de atendimentos ativos, exclusão com `force`, teto).

## Fichas: leitura e ações humanas (FX-29 e FX-30)

A Ficha é o registro de um atendimento. Ela e as execuções de fluxo que rodam nela ficam legíveis
mesmo depois de fechadas.

### Leitura

- `GET …/fichas`: filtros `status` (`open` ou `closed`), `agente` (id), `canal` (por exemplo `whatsapp`),
  `desde` (instante em milissegundos desde 1970), `limit` (1 a 200, padrão 50) e `cursor`. O cursor é o id
  da última Ficha da página. Filtro inválido: `400 validation_error` com o campo.
- `GET …/fichas/{ficha_id}`: a Ficha com as execuções, as etapas de cada execução com os campos já
  respondidos (é o que o atendente vê dentro da Ficha) e os fluxos liberados ao agente. Forma abreviada:

```json
{
  "id": "…", "conversation_id": null, "agent_id": "…", "canal": "whatsapp", "status": "open",
  "version": 3,
  "execucoes": [{
    "id": "…", "fluxo_id": "…", "revisao_id": "…", "status": "ativa", "no_atual": "coleta",
    "etapas": [{ "no_id": "coleta", "status": "parcial", "dados": { "nome": "…" } }]
  }],
  "permissoes": { "fluxos_liberados_ao_agente": [{ "id": "…", "slug": "…", "name": "…", "status": "active" }] }
}
```

  O detalhe não traz o texto das mensagens do cliente nem os efeitos enviados. Lê até 500 execuções.
- `GET …/fichas/{ficha_id}/events`: linha do tempo em ordem de `seq`. Cursor é o `seq` do último evento.
  Traz só o metadado do motor (chave, tipo e nó).

Ficha de outra org ou inexistente responde `404` nas três rotas. Token de outra org responde `403`.

### Ações humanas do atendente

Todas exigem `If-Match`. As três primeiras também exigem `chave_idempotencia`. Repetir a mesma chave não
cria execução, evento nem efeito novo, e devolve o que a primeira chamada fez (com `repetida: true` na
abertura).

| Rota | Corpo | `If-Match` é a versão de | Situação | Resposta |
| --- | --- | --- | --- | --- |
| `POST …/fichas/{ficha_id}/flow-runs` | `{"fluxo_id", "chave_idempotencia"}` | a Ficha | abre um fluxo liberado ao agente, sem o Jev | `201` (aberto) ou `200` (chave repetida) |
| `POST …/flow-runs/{run_id}/actions` | `{"acao", "campos", "chave_idempotencia"}` | a execução | o atendente executa uma ação da etapa atual; a ação fica com ator `atendente` e o id do token em `ator_ref` | `200` com a execução |
| `POST …/flow-runs/{run_id}/abandon` | `{"motivo", "chave_idempotencia"}` | a execução | para a execução onde ela está; não gera efeito | `200` com a execução |
| `POST …/fichas/{ficha_id}/close` | sem corpo | a Ficha | fecha a conversa (quando há) e a Ficha; a execução ativa vai para `abandonada` com motivo `atendimento_encerrado` | `200` com a Ficha |

| Situação | Resposta |
| --- | --- |
| sem `If-Match` | `428 precondition_required` |
| `If-Match` diferente da versão atual (corrida com o agente, por exemplo) | `412 version_conflict`; nada é gravado |
| Ficha fechada | `409 ficha_encerrada` |
| fluxo não liberado ao agente da Ficha | `403 fluxo_nao_liberado`; nenhuma execução é aberta |
| fluxo sem revisão publicada | `409 conflict` |
| já existe execução ativa na Ficha | `409 execucao_ativa` |
| ação que só o agente pode fazer | `409 acao_invalida` |
| chave vazia ou campos que não batem com a etapa | `400` |
| `close` repetido depois de fechar | `409 ficha_encerrada` (não tem chave: não há repetição a devolver) |

- `close` sem conversa ligada (conversa apagada pela retenção) fecha só a Ficha, com o mesmo motivo do
  fechamento pela conversa.
- A chave repetida é conferida antes da versão: reenviar uma ação que já passou não vira `412`.

Provas: `crates/poph-server/tests/fichas_leitura.rs` (filtros, cursor, Ficha de outra org, eventos,
campos respondidos dentro da Ficha, Ficha fechada legível) e `crates/poph-server/tests/fichas_acoes.rs`
(Ficha fechada, fluxo não liberado, corrida atendente × agente, ação restrita, repetição de chave,
`close` com e sem conversa, isolamento).

## Operational summary of the service desk

`GET /admin/v1/orgs/{org_id}/analytics/operational-summary?from=<ms>&to=<ms>` answers with the
counters the Console home reads, in one round trip and without walking Fichas one by one:

```json
{
  "open_fichas": 2,
  "closed_fichas": 1,
  "open_conversations": 2,
  "avg_resolution_ms": 5400000.0,
  "active_flow_nodes": [{ "node_id": "aprovacao_orcamento", "count": 1 }]
}
```

| Field | Rule |
| --- | --- |
| `from`, `to` | Window in milliseconds since 1970. Both required. `to` is exclusive. |
| `open_fichas` | Fichas with `status = 'open'` and `aberta_em < to`. The start of the window does not cut them: an open Ficha is open now, whatever window is asked. |
| `closed_fichas` | Fichas with `status = 'closed'` and `fechada_em` inside `[from, to)`. |
| `open_conversations` | Conversations with `status` `open` or `transferred`. A transferred conversation is still being handled. |
| `avg_resolution_ms` | Mean of `fechada_em - aberta_em` over the Fichas counted by `closed_fichas`, as a float in milliseconds. `null` when the window has no closed Ficha. |
| `active_flow_nodes` | Flow runs with `status = 'ativa'` on Fichas that are `open` and were created before `to`, grouped by `no_atual`. Always a list, empty when nothing is running, ordered by `count` descending and then by node id. |

An empty org answers zeros, `avg_resolution_ms` `null` and an empty list — never an error and never a
partial number. A Harness failure answers an upstream error, not zeros.

Errors: `from` or `to` missing or not an integer, and `from >= to`, answer
`422 validation_error` with `details.field`; `from >= to` carries
`reason = "from must be before to."`. Org that does not exist answers `404`, a token from another
org answers `403`, and no token answers `401`.

Two indexes carry the read (migration `0017_analytics_operational_summary`): `fichas (org_id, status,
fechada_em)`, for the closed window and its average, and `fluxo_execucoes (org_id, status, no_atual)`,
for the active runs grouped by node. The open count reuses `fichas (org_id, status, aberta_em)` and
the conversation count reuses `conversations (org_id, status, updated_at)`.

Proofs: `crates/poph-server/tests/analytics_summary.rs` (empty org, window with Ficha open before and
after `to`, closed inside and outside the window, mean of two closed, active runs per node with a
finished run left out, two orgs isolated, the six invalid windows with field and reason, `401`, `403`
and `404`) and the isolation case of `crates/poph-server/tests/org_isolation.rs`.

Paridade de CLI: ainda não existe; entra na FX-31.

## Limites conhecidos da API

- **Recarga do cluster sem auditoria (baixa):** é configuração da instância em memória; fica o log e
  a resposta com `fingerprint_antes`/`fingerprint_depois`, não uma linha no `audit_log`.
- **Segredo do cluster só na subida (baixa):** trocar `POPH_CLUSTER_SECRET` com o processo no ar não
  muda o segredo em uso; reinicie.
- **Auditoria fora da transação (média):** quase todas as escritas gravam a mudança e depois a linha
  de auditoria, em duas transações; falha de banco exatamente entre as duas deixa a mudança sem
  registro. Fora da transação: criar/alterar/excluir org, token, categoria, departamento, agente,
  instruções, grupo, item, vínculo, produto, integração e envio de arquivo. Dentro da mesma
  transação: exclusão de arquivo (`excluir_auditado`) e reabertura de conversa (`reopen_audited`). O
  registro é sempre "quem fez o quê", nunca o valor de token ou segredo.
- **Criação de agente com departamento de transferência e alteração conjunta de campos e categorias
  (média):** usam mais de uma escrita; falha no meio deixa a operação parcial, igual à CLI.
- **Sincronização do índice em produto e vínculo (baixa):** a resposta traz `index_warning` quando a
  sincronização falha; o dado fica salvo e o índice pode ficar atrasado até um `reindex`.
- **Cache do índice por processo (média, HR-50):** com mais de uma instância servindo a mesma org, a
  reindexação de uma não invalida o cache da outra.
- **Tipos de resposta genéricos (baixa):** parte das respostas (base de conhecimento, produtos,
  integrações) aparece como JSON genérico no OpenAPI; o contrato real está nos testes.
- **Manifesto depende de plugin declarado (média):** sem `[plugins].integracoes`, nenhum `kind` tem
  manifesto e a configuração de integração continua recusada; o programa do plugin é caminho local
  do servidor e mudá-lo exige reiniciar o processo.
- **Manifesto resolvido subindo o plugin (média, ao estrear):** a primeira leitura do manifesto (e a
  primeira configuração de integração daquele `kind`) custa a subida do plugin, com o prazo do
  handshake; falha fica em cache por 30 s.
- **Validação de esquema é subconjunto (média):** a API valida `type`, `required`, `enum`,
  `minLength`/`maxLength`, `minimum`/`maximum` e `properties` aninhados; palavras-chave fora disso
  (`pattern`, `oneOf`, `allOf`…) são ignoradas e **campo fora do esquema é recusado** (salvo com
  `additionalProperties: true`), ao contrário do padrão do JSON Schema.
- **Troca de `POPH_CHAVE_CIFRA` inutiliza segredos (alta, ao operar):** não há re-cifragem; os
  segredos precisam ser regravados.
- **Integração identificada por `kind` (média):** o esquema não tem coluna de nome nem de manifesto,
  então duas integrações do mesmo tipo na mesma org não têm identificação legível.
- **Fechar Ficha sem conversa não grava linha em `audit_log` (média):** o fechamento fica registrado no
  evento `ficha_fechada` da própria Ficha, com quem fechou. Só o fechamento com conversa passa pela
  auditoria de conversa. A tabela de inventário marca `sim` porque a escrita é auditada no caminho com conversa.
- **Auditoria das ações do atendente fora da transação (média):** `flow_run_start`, `flow_run_action` e
  `flow_run_abandon` são gravados logo depois da transição, em outra escrita. Se essa escrita falhar, a ação já
  ficou gravada e a resposta sai como erro; o evento da Ficha continua certo.
- **Leitura da Ficha não é foto única (baixa):** o detalhe lê Ficha, execuções e etapas em consultas separadas;
  uma transição concorrente pode aparecer no meio. A leitura seguinte sai certa.
- **Detalhe da Ficha corta em 500 execuções sem aviso (baixa).**
- **Chave repetida devolve o estado atual (baixa):** a repetição devolve a execução como está agora, não como
  estava na primeira chamada. Não gera evento nem efeito novo.
- **`diff` com `de` maior que `para` (baixa):** devolve a diferença invertida, sem erro.
- **`DELETE` de fluxo sem `If-Match` (baixa):** a proteção contra exclusão acidental é o `force=true` exigido com
  atendimento ativo, mais a auditoria.
- **`preview` sem dados preenchidos (baixa):** mostra a etapa com os campos vazios; não simula respostas.

## Token usage by organization

`GET /admin/v1/orgs/{org_id}/usage/tokens?from=<ms>&to=<ms>` requires an organization or global bearer token and an existing organization. Both integer epoch-millisecond bounds are required; the interval is `[from, to]` over `turns.started_at`. Invalid bounds return 422, unauthorized requests 401, another organization's token 403, and a missing organization 404.

The response contains `total_input_tokens`, `total_output_tokens`, `total_cost_micros` and `turn_count`, plus `by_agent` and `by_day` arrays. Each agent entry has `agent_id`, `input_tokens`, `output_tokens` and `turn_count`; a null agent is included and sorted last. Day entries contain `date` (UTC, YYYY-MM-DD), `output_tokens` and `turn_count`, ordered by date. Empty periods produce zero totals and empty arrays. Counts refer to turns, not model calls; retained history may be incomplete. The queries share a read snapshot. Migration 0018 indexes `turns(org_id, started_at)`.


### Conversation analytics

`GET /admin/v1/orgs/{org_id}/analytics/conversations` requires an organization token and
`from` and `to` in epoch milliseconds, selecting `[from, to)`. It uses the first message
timestamp, or conversation creation when no message exists. Optional `agent_id` and `channel`
filter the whole result; empty values do not filter. `tz_offset_minutes` defaults to zero and
must be between -840 and 840; it changes daily grouping only.

The response contains `summary`, `by_day`, `by_agent`, and `by_channel`. Counts separate open,
transferred, and closed statuses. The message average excludes conversations with no messages
and counts all retained messages per selected conversation. The Case rate is the fraction of
selected conversations linked to a Case. Both are null without a denominator. Agent names
reflect the current record. All blocks are read from the same database snapshot. Retention can
change message counts and the earliest message available.

### SLA analytics

`GET /admin/v1/orgs/{org_id}/analytics/sla` returns age buckets and the most idle open Cases. `to` is required in milliseconds; only Cases opened before it are included. `now` defaults to the server clock. `limit` defaults to 20 and accepts 1 through 100. `age_bounds_minutes` defaults to `60,240,1440,2880,4320,10080` and accepts up to 16 strictly increasing positive integers. Buckets are lower-inclusive and upper-exclusive, with negative ages in the first bucket.

Each stale item provides `ficha_id`, `conversation_id`, `channel`, `peer`, `client_name`, `current_step_key`, `age_ms`, and `idle_ms`. The last activity is the latest Case event, falling back to opening time. The active flow supplies the current step. Deleted conversations leave their channel snapshot on the Case. Items are ordered by idle time, age and Case ID, all descending. Both lists use the same database snapshot.

## Site conversations

A trusted HTTP adapter calls these routes under `/admin/v1/orgs/{org_id}` using the same
organization-scoped service credential as the other administrative routes. The adapter links
each visitor and thread to an opaque reference and keeps the organization credential server-side.
The session, origin checks and rate limits seen by the visitor belong to the adapter.

| Method and path | Contract |
| --- | --- |
| `POST /agents/{agent_id}/site-conversations` | Takes `{"external_ref":"…"}` with 1–128 characters and returns `{"conversation_id":"…"}`. Creates a `site` conversation with no integration and uses the reference as its external peer. Returns 201 for a new `(org, agent, external_ref)` and 200 for the same conversation on repeat. |
| `POST /site-conversations/{id}/messages` | Takes `{"text":"…","idempotency_key":"…"}`. Text has 1 to `site_chat.max_text_bytes` UTF-8 bytes and the key has 1–128 characters. Replies with 202 and `{message_id, turn_id, status:"accepted"}`. The same key and text return 200 with the same response. A reused key with different text returns 409 `idempotency_conflict`. A different key while a turn is queued or running returns 409 `turn_in_progress` with `details.turn_id`. |
| `GET /site-conversations/{id}/turns/{turn_id}` | Returns `{status, error_code, started_at, finished_at}`. States are `queued`, `running`, `succeeded`, `failed`, and `cancelled`. Timestamps are UTC epoch milliseconds. While queued, `started_at` is the acceptance time. It becomes the execution start time when claimed. `finished_at` and `error_code` may be null. |
| `POST /site-conversations/{id}/turns/{turn_id}/cancel` | Returns the same turn shape with its terminal status. If the turn already finished, the response describes that existing result. |
| `GET /site-conversations/{id}/messages?before=<id>&limit=<n>` | Returns `{"messages":[...]}` in descending message ID order. Each row has `id`, `role` (`user` or `assistant`), `text` and `created_at`. The cursor is exclusive and omitted on the first page. The limit defaults to 50 and accepts 1–100. |
| `GET /site-conversations/{id}/events?after=<seq>&limit=<n>` | Returns `{"events":[...]}` in ascending sequence order. The cursor is exclusive and defaults to 0. The limit defaults to 100 and accepts 1–200. |
| `GET /site-conversations/{id}/events/stream?after=<seq>` | Returns a persistent SSE subscription. The event ID is the sequence. Replay begins after the greater of `after` and `Last-Event-ID`. The server sends keep-alives while idle. |

Events have `seq`, `type` and `turn_id`. `message` adds `message_id`, `role` and `text`.
`text_delta` adds `text` and `turn_failed` adds `error_code`. The other event types are
`turn_start`, `turn_end` and `turn_cancelled`. A delta represents provisional text. Only the
assistant's committed `message` event and text history represent the final reply. The stream
exposes visitor and assistant text and turn lifecycle events.

An accepted message and its first event are persisted together. The turn continues after the
HTTP client disconnects. Startup resumes queued turns and marks interrupted running turns failed
with `server_restarted`. Cancellation atomically records its terminal event and prevents any
later completion from publishing an assistant message. The organization retention policy
expires a site's replay events and idempotency record when it removes the corresponding visitor
message. The conversation sequence remains monotonic across retention.

The organization bearer token follows the usual 401 and 403 rules. Lookup within the requested
organization returns 404 for missing conversations, non-site conversations and turns belonging
to another conversation. The adapter must enforce visitor and configured-agent ownership before
calling routes addressed by conversation ID. A rejected field returns 422 `validation_error`
with `details.field`. An agent with an enabled tool outside the site's server-side policy
returns 422 `agent_not_public`. The runtime checks the policy again when it starts a turn.
Site turns cannot create Cases, transfer conversations, enqueue outbound channel work or
notify a human. Attachments and lead capture are reserved for later versions.

## Adapter conversations

A trusted adapter, such as the e-mail channel or the POP Bot, calls these routes under
`/admin/v1/orgs/{org_id}` with an organization-scoped service credential. The adapter owns delivery and
brings its own idempotency keys; the Harness owns the conversation, the Case, the history and the adapter
turn. The message routes record messages in either direction without starting a turn; the turn routes run
the agent with the full tool set of the agent and never enqueue outbound channel work.

| Method and path | Contract |
| --- | --- |
| `POST /agents/{agent_id}/adapter-conversations` | Takes `{"channel":"…","external_ref":"…","create_case":true}`. `create_case` defaults to true and opens a Case together with the conversation; false leaves the conversation without a Case, now and at close. Returns the conversation object. A new active `(org, agent, channel, external_ref)` answers 201; the existing one answers 200. A conversation in `open` or `transferred` is resumed; after `closed`, the next call creates another conversation and, with `create_case`, another Case. |
| `GET /adapter-conversations/{id}` | Returns `{id, org_id, agent_id, channel, external_ref, create_case, status, version, created_at, updated_at, last_message_at, ficha_id}`. `ficha_id` is null when the conversation has no Case. |
| `POST /adapter-conversations/{id}/messages` | Takes `{"text":"…","origin":"…","idempotency_key":"…","author_name":"…","external_message_id":"…"}`. `origin` is `customer` (inbound user message, stored with origin `agent`), `human` (outbound assistant message, origin `human`) or `agent` (outbound assistant message, origin `agent`). Text has 1 to `adapter_chat.max_text_bytes` UTF-8 bytes. `idempotency_key` has 1–128 characters; `author_name` and `external_message_id` accept up to 128 characters and are optional. Returns the message with 201. The same key and payload returns 200 with the same message; the same key with another payload returns 409 `idempotency_conflict`; a closed conversation returns 409 `conversa_encerrada`. |
| `GET /adapter-conversations/{id}/messages?before=<id>&limit=<n>` | Returns `{"messages":[...]}` in descending message ID order. Each row has `id`, `role` (`user` or `assistant`), `origin`, `author_name`, `text`, `external_message_id` and `created_at`. The cursor is exclusive and omitted on the first page. The limit defaults to 50 and accepts 1–100. |
| `POST /adapter-conversations/{id}/turns` | Takes `{"text":"…","idempotency_key":"…","author_name":"…","external_message_id":"…"}`. Text has 1 to `adapter_chat.max_text_bytes` UTF-8 bytes; `idempotency_key` has 1–128 characters. Stores the customer message and answers 202 `{"turn_id","message_id","status":"queued"}`. The same key and payload returns 200 with the same turn and its current status; the same key with another payload returns 409 `idempotency_conflict`. A second turn while one is `queued` or `running` returns 409 `turn_in_progress` with `details.turn_id`. A conversation that is not `open` returns 409 `conversa_encerrada`. |
| `GET /adapter-conversations/{id}/turns/{turn_id}` | Returns `{id, conversation_id, status, reply_text, error_code, input_tokens, output_tokens, created_at, started_at, finished_at}`. `status` is `queued`, `running`, `succeeded`, `failed` or `cancelled`. `reply_text` is the stored agent message and is null until the turn succeeds. |
| `POST /adapter-conversations/{id}/turns/{turn_id}/cancel` | Marks a `queued` or `running` turn as `cancelled` and signals the running turn. It is idempotent: a terminal turn answers 200 with its current state. |

The channel matches `[a-z0-9_-]{1,32}`. The channels `test`, `site`, `whatsapp`, `matrix` and `api` are
reserved and answered with 422 `validation_error` with `details.field` = `channel`, because each one has its
own rules. An agent outside the organization answers 404 on creation.

Adapter conversations appear in the general routes: `/conversations` lists them with the informed channel,
the detail and the close, transfer and reopen routes accept them, and conversation analytics counts them
under that channel. They carry no integration and no participant policy: the adapter is responsible for the
customer and for the agent in the organization. The Matrix mirror skips only `site`, so an `email`
conversation is mirrored like any other channel; the conversation JSON exposes `matrix_room_id` when the
Matrix module is linked and the room is ready.

A message and its idempotency record are written in one transaction. These routes never write `outbox`
rows. The organization bearer token follows the usual 401 and 403 rules. Lookup within the requested
organization returns 404 for missing conversations, conversations of another channel and conversations of
another organization.

The turn follows the same rules as a channel turn: one turn per conversation, a durable state in
`adapter_turns`, and the same `TurnEngine::run` used by WhatsApp and the test chat, with the full tool set
of the agent (the site policy does not apply). The difference is the delivery: the reply is stored as an
agent message in the history and returned in `reply_text`, and no `outbox` row is written. The adapter is
responsible for delivering the reply. The input limit comes from `adapter_chat.max_text_bytes` (16 KiB by
default, accepted range 1 byte to 256 KiB in `poph.toml`) and applies to both the message and the turn
routes. A turn that was `running` when the process stopped becomes `failed` with `error_code`
`server_restarted` on the next boot, unless the reply message was already stored for that turn: then the
turn is marked `succeeded` with the stored reply, so a restart never answers the same turn twice. A
`queued` turn is started again. A successful turn is never answered twice because the reply message is
written before the turn is marked `succeeded`.

Flow effects triggered during an adapter turn follow the normal queue. A `notificar_whatsapp` effect cannot
be delivered from an adapter conversation because it has no WhatsApp integration: `FlowEffects::plan`
returns `Plan::Wait`, the effect keeps `state = pending` with `proxima_tentativa` set and consumes no
attempt, instead of failing. A `mensagem_ao_cliente` effect is deferred the same way, because adapter
conversations never write `outbox` rows and delivery belongs to the adapter; the turn reply is the message
the adapter reads from the history.

## Server internals: boot order, Console delivery and tokens (EN-13c)

These rules used to live as comments in `crates/poph-server/src`. They still hold.

- **Boot order** (`lib.rs`): an interrupted turn from a previous process becomes a recorded failure before any service is answered, so the history never says "in progress" forever. The WhatsApp output loop starts only when an instance is configured and stops with the server. The flow effects queue starts only when the flow engine is on. The turn runtime starts only when the flow engine is on at the composition root. The Matrix module is off by configuration; when on, it starts its loops and injects the Application Service receiver. A configuration or connection failure does not stop the server: the Matrix module stays degraded. On shutdown, the Matrix module stops first, so its loops finish what they are doing and nothing pending is lost (the cursor only moves after success).
- **Console delivery** (`console.rs`): an exact file in the bundle is served first; then `index.html` for client routes; then `404` when the path looks like a file (a missing asset must not come back as HTML). Server routes (`/admin/v1`, `/healthz`, `/metrics`, `/webhook`) are never shadowed. The comparison is by prefix with a `/` boundary, so `/metricsx` stays a missing route. `/_matrix` answers only on fixed paths; with the Matrix module off, an unknown Matrix path returns `404`, not the Console page. Vite file names carry a hash, so they may be cached forever. The server builds without a bundle: it serves a warning page at `/`.
- **Token checks** (`auth.rs`): a token of an org passes only in its own org; a wrong org gives `403`, not `404`, because the client knows which org it asked for. Creating or deleting orgs and creating or revoking tokens require the global token. The Bearer value is never logged; the audit trail records the token id or label. The argon2 check is deliberately slow and runs in `spawn_blocking` so it does not hold a Tokio worker.
- **Cipher** (`cipher.rs`): the cipher key is derived once at start-up and kept in memory; the key value itself is not stored or logged. An invalid key value does not stop the server: it becomes "not configured", and writing a secret then returns `503` with the orientation. Decryption failure never shows the secret content.
- **Domain errors** (`error.rs`): a domain error with no dedicated mapping takes its status and its stable `code` from the domain error itself (the domain declares both). Before FX-07 every new case fell into `400 invalid_request`, which hid, for example, the `409 conversa_encerrada` of reopen. The message to the client is the domain error text; the internal failure detail goes to the log, not to the body.
- **Error body** (`error.rs`): the body is `{code, message, details}` (PRD §8.1). The `WWW-Authenticate: Bearer` header appears only on `401`. The `code` is what the Console and scripts match on, never the `message`.

## Organization isolation coverage (HR-54)

The integration suite in `crates/poph-server/tests/org_isolation.rs` uses a running HTTP server
and temporary databases. It checks for a test case for every published organization operation and
rejects stale cases for removed operations. The published OpenAPI supplies the concurrency headers
and optional force parameter used by the suite. Each organization must be able to read its own
seeded resources, and the before and after snapshots confirm that denied requests leave its data
intact. Lists and resource references are also checked for organization scoping.

The response checks distinguish data returned by the server from values echoed from the request.
A unique seeded label must remain private on all responses. Resource IDs are checked for leaks on
successful responses, since an error can repeat an ID supplied by the caller.

The same isolation rules apply to agent tools and knowledge search. Tool tests run an actual turn
with a fake model and confirm that the owner's requests work while results and writes stay within
the organization. Search tests also check the agent boundary within the organization.
