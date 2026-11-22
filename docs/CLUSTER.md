# Cluster do POP Harness — roteamento estático entre instâncias (HR-50a)

Cada organização vive em **uma** instância, com os dados no SQLite dela (PRD §12). O mapa de qual
org vive em qual instância é um arquivo de configuração — `cluster.toml` — separado do `poph.toml`
e **igual em todas as instâncias**. É estático de propósito: mudou o mapa, edita o arquivo e pede
recarga; não existe hash dinâmico que "mova" orgs sozinho e deixe dados órfãos.

Este documento cobre o **mapa** (formato, validação, impressão digital, os comandos `poph cluster
show|check` e a recarga do processo vivo, `POST /admin/v1/cluster/reload`) e o **encaminhamento** de
webhook, chamadas admin, chat ao vivo (SSE) e `?cluster=true` entre instâncias (seção "Encaminhamento
entre instâncias"; HR-50a, HR-50b-1 e HR-50b-2, todas feitas).

## Onde o arquivo mora

| Origem | Caminho |
| --- | --- |
| `POPH_CLUSTER` (ambiente) | o caminho indicado, quando não vazio |
| padrão | `cluster.toml` **no mesmo diretório do `poph.toml`** resolvido (`--config`, `POPH_CONFIG` ou `./poph.toml`) |
| sem `poph.toml` | `cluster.toml` na pasta em que o processo roda |

**Sem `cluster.toml`, a instalação roda em modo instância única**: todas as orgs vivem nela, e nada
muda em relação a hoje. Um arquivo vazio (só comentários) também é modo instância única; um arquivo
com qualquer chave **precisa** se identificar com `instance_id`.

O **segredo compartilhado** entre as instâncias (o HMAC do encaminhamento, HR-50b) vem **só** de
`POPH_CLUSTER_SECRET`, no ambiente. Ele nunca vai no arquivo, nunca é impresso, nunca aparece em
resposta, log ou erro. O `poph serve` lê o segredo na subida; trocá-lo exige reiniciar o processo.

## Formato

```toml
# Nome desta instância no mapa. Obrigatório quando existe cluster.toml.
instance_id = "a"

# Para onde vão as orgs que não estão no mapa. Obrigatória quando o cluster tem mais de uma
# instância; com uma só, ela é a padrão implícita.
default_instance = "b"

# Instâncias do cluster: id curto e endereço base (http:// ou https://, com destino).
[[instances]]
id = "a"
url = "http://10.0.0.11:7480"

[[instances]]
id = "b"
url = "https://harness-b.exemplo"

# Mapa explícito org → instância, no formato [orgs] do TOML. A chave é o id da org (UUIDv7, sem prefixo).
[orgs]
01a1ff21-8f3c-7a2b-9c4d-5e6f70819203 = "a"
01a1ff21-8f3c-7d10-8e2f-a1b2c3d4e5f6 = "b"
```

Regras conferidas (cada problema diz a chave):

- `instance_id` obrigatório, não vazio, até 64 caracteres (`letras`, números, `-`, `_`, `.`);
- cada instância: `id` no mesmo formato, sem repetir, e `url` começando com `http://` ou `https://`
  com destino (sem espaços);
- a instância local (`instance_id`) precisa estar declarada em `instances`;
- `default_instance`, quando declarada, precisa existir em `instances`; é **obrigatória** quando o
  cluster tem mais de uma instância (senão as orgs fora do mapa não teriam dono);
- cada org do `[orgs]` precisa apontar para uma instância declarada; chave repetida no `[orgs]` é
  recusada pelo próprio TOML (o arquivo nem é lido como configuração);
- campo desconhecido, TOML inválido e arquivo ilegível dão erro com o **arquivo** e a **chave**
  (quando há uma).

## Impressão digital (fingerprint)

`poph cluster show` e a recarga devolvem uma impressão digital estável do mapa: instância local,
instância padrão resolvida, instâncias ordenadas por id e orgs ordenadas por chave. Mesmo mapa,
mesma impressão (a ordem do arquivo não importa); org trocada de instância muda a impressão. Não é
criptográfica e **não inclui o segredo** — serve para detectar mudança, não para autenticar.

Como a instância local entra nessa impressão, duas instâncias do mesmo cluster têm impressões
**diferentes**. Para comparar o mapa entre elas existe uma segunda impressão, a **do mapa** (sem a
instância local): é a que o encaminhamento manda em `x-poph-mapa` e a que o `show` imprime na linha
"fingerprint do mapa (sem a instância local)" (`fingerprint_mapa` no `--json`). Mesmo `cluster.toml`
a menos do `instance_id` = mesma impressão do mapa.

## Comandos

```bash
poph cluster show            # mapa efetivo: modo, instância local, orgs por instância, fingerprint
poph cluster show --json     # formato estável: versao_relatorio = 1, sem segredo
poph cluster check           # valida e lista TODOS os problemas; sai com 1 quando há algum
poph cluster check --json
```

- Sem `cluster.toml`: `show` diz "modo: instância única"; `check` diz que não há nada a corrigir.
- Cluster declarado sem `POPH_CLUSTER_SECRET`: `check` reprova dizendo qual variável definir.
- `show`/`check` **não** sobem servidor nem tocam no banco; só leem o arquivo e o ambiente.
- Nenhuma saída traz o valor do segredo (só `segredo compartilhado: definido`/`não definido`).

## Recarga sem reiniciar

`POST /admin/v1/cluster/reload` (token **global**; token de org → `403`, sem token → `401`) relê o
arquivo no processo vivo. Contrato completo em `docs/API.md`, seção "Roteamento entre instâncias:
mapa e recarga".

- Arquivo novo válido → o mapa passa a valer na hora; a resposta traz `fingerprint_antes`,
  `fingerprint_depois`, o resumo dos dois mapas, as orgs que trocaram de instância e as instâncias
  adicionadas/removidas.
- Arquivo inválido → `400 cluster_invalido` com `details.problemas`; **o mapa anterior continua em
  uso** — um erro de digitação não derruba o roteamento.
- Arquivo removido → volta ao modo instância única (é o que a recarga reflete).

## Encaminhamento entre instâncias (HR-50b-1, HR-50b-2)

O mapa agora é aplicado de verdade: um pedido de org que vive em outra instância é encaminhado à
dona, assinado. **Chamada admin** (`/admin/v1/orgs/{org_id}/…`) e **webhook do Evolution** são
encaminhados; saúde, métricas, OpenAPI, recarga, `/admin/v1/orgs` e o Console ficam sempre na
instância que recebeu — nenhum deles tem org no caminho. Sem `cluster.toml` (modo instância única),
nada é encaminhado.

**Cabeçalhos de par**

| Cabeçalho | Conteúdo |
| --- | --- |
| `x-poph-instance` | `instance_id` de quem encaminha |
| `x-poph-timestamp` | instante do envio, em segundos Unix |
| `x-poph-signature` | HMAC-SHA256 em hexadecimal |
| `x-poph-hop` | `1` no primeiro (e único) salto |
| `x-poph-mapa` | impressão digital **do mapa** de quem encaminha |
| `x-poph-nonce` | 16 bytes aleatórios em hexadecimal (32 caracteres), um por pedido |

**Texto assinado** (chave: `POPH_CLUSTER_SECRET`; linhas separadas por `\n`, sem linha no fim):

```text
método
caminho+query
hex(sha256(corpo))
timestamp
instância
hop
nonce
```

O `x-poph-mapa` é o `ClusterConfig::fingerprint_do_mapa`: o mesmo `cluster.toml` em duas
instâncias produz a mesma impressão mesmo com `instance_id` diferente — é o que permite detectar
mapa divergente. A impressão do `poph cluster show` continua incluindo a instância local (ela
identifica o arquivo daquela instância); são valores diferentes com propósitos diferentes.

**Regras**

- Janela do carimbo: 60 s para trás e para frente.
- O **nonce** é o que torna cada pedido único: dois pedidos idênticos no mesmo segundo, com nonces
  diferentes, passam os dois; o mesmo nonce dentro de 120 s é `401 assinatura_invalida` (proteção
  contra repetição, em memória do processo; teto de 10.000 nonces). Nonce ausente ou fora do
  formato (32 hexadecimais) é `401 assinatura_invalida`.
- `x-poph-hop > 1`, ou org que não é desta instância num pedido já encaminhado: `421 rota_em_laco`.
- `x-poph-mapa` diferente do local: `421 mapa_divergente` com os dois fingerprints em `details`.
- Dona fora do ar: `503 instancia_fora` com `details.instancia` e `details.url`. Tempos: 5 s para
  conectar, 60 s para a dona começar a responder; o corpo, depois disso, segue em streaming.
- Pedido **sem** cabeçalho de par segue o caminho de token de sempre, sem nenhuma mudança.
- O segredo compartilhado nunca aparece em resposta, erro ou log.

**SSE do chat de teste (HR-50b-2).** `GET …/test-chat/{conversa}/messages` (e o `POST` que abre o
stream) atravessa o encaminhamento sem buffer: o primeiro evento chega em milissegundos, os eventos
fluem um a um e o prazo de 60 s vale só até a dona começar a responder — depois disso o stream não
tem prazo. Cliente que desconecta derruba o pedido encaminhado e o turno é cancelado na dona.
Medido no teste de duas instâncias: primeiro evento em ~0,36 s com o turno pendurado por 30 s.

**Listagem de orgs com `?cluster=true` (HR-50b-2).** `GET /admin/v1/orgs` continua local (não é
encaminhada); com o parâmetro e um **token global**, ela consulta as instâncias vizinhas do mapa em
paralelo (`/admin/v1/orgs` de cada uma, com o `Authorization` do chamador, prazo de 2 s por
instância) e devolve todas as orgs em `items`, cada uma com `instancia`, mais `instancias` com
`id`, `url`, `situacao` (`local`, `ok`, `inacessivel`) e a contagem de orgs. Instância fora do ar
entra marcada e não derruba a lista. Sem o parâmetro a resposta é a de sempre; com ele não há
paginação. Token de org ignora o parâmetro e vê só a própria org.

**`/healthz` no modo cluster** devolve `instance_id` e `cluster_map_fingerprint` (aditivo, ausentes
no modo instância única).

**`poph cluster check --remoto`.** Consulta o `/healthz` de cada instância declarada (menos a local,
que é o próprio arquivo), em paralelo, com prazo de 3 s cada, e aponta:

- **inacessível** — não respondeu, respondeu erro ou respondeu sem dizer o mapa (versão anterior);
- **mapa divergente** — a impressão digital do mapa dela é outra (atualize o `cluster.toml` dos dois
  lados e recarregue as duas);
- **identidade divergente** — ela se declara com outro `instance_id` (corrija o `instance_id` dela).

Sai com código 1 quando há qualquer problema. Sem `--remoto`, o comando continua validando só o
arquivo, como antes. Em JSON, o relatório traz `remoto` (um item por instância) e
`problemas_remotos`.
## O que ainda NÃO existe

- **Mover uma org de instância automaticamente:** trocar o mapa e recarregar só muda o roteamento;
  os dados continuam na instância antiga até um exportar/importar manual.
- **Retentativa/failover:** se a dona falhar no meio de um pedido, o erro volta ao cliente; não há
  nova tentativa em outra instância.
- **Encaminhar com token global:** o `Authorization` original segue junto e quem valida é a dona.
- **Mapa no `/healthz` de versões antigas:** uma instância anterior à HR-50b-2 aparece como
  inacessível no `cluster check --remoto` (não diz o mapa).

## Limites conhecidos

- **Média, corpo do pedido encaminhado:** o corpo é lido inteiro antes de seguir (a assinatura usa
  o sha256); a resposta da dona vai em streaming. Um corpo grande encaminhado ocupa memória na
  instância que recebeu e na dona ao mesmo tempo, até o teto da rota.
- **Média, listagem `?cluster=true` sem paginação:** devolve o retrato do cluster inteiro (até 20
  páginas de 200 orgs por instância vizinha); numa instalação com muitas orgs a resposta é grande e
  a tela do Console precisa paginar no cliente.
- **Média, listagem remota com o token do chamador:** se o token for válido nesta instância e não na
  vizinha (tokens globais diferentes), a vizinha aparece como `inacessivel` com o código HTTP.
- **Baixa, `cluster check --remoto` não confere a instância local:** ela é o próprio arquivo em uso;
  o comando olha as vizinhas. A URL local do mapa não é consultada.
- **Baixa, `cluster check --remoto` usa `/healthz`:** rota pública na rede interna, sem o segredo do
  cluster; quem alcança a porta pode ler a impressão digital do mapa (não é segredo).
- **Média, `cluster.toml` recarregado só na instância que recebeu a recarga:** as demais precisam
  de `POST /admin/v1/cluster/reload` também; mapa diferente entre elas dá `421 mapa_divergente`
  (o pedido é recusado, nunca processado com mapa errado).
- **Média, recarga reflete o arquivo:** se o arquivo desaparece (ou `POPH_CLUSTER` aponta para um
  caminho errado) e o operador pede recarga, a instância volta ao modo instância única. A recarga é
  explícita; não há proteção contra isso.
- **Baixa, segredo na subida:** trocar `POPH_CLUSTER_SECRET` com o processo no ar não troca o
  segredo em uso; reinicie.
- **Baixa, cache de mapa:** o mapa é lido do arquivo na subida e na recarga; editar o arquivo sem
  pedir recarga não muda nada (é o desenho: estático).
- **Baixa, `[orgs]` por id:** a chave deve ser o id da org (UUIDv7, como `01a1ff21-8f3c-7…`, sem prefixo
  `org_`), que é o que aparece no caminho das rotas e no webhook. Slug não é resolvido pelo mapa.
- **Baixa, sem auditoria:** a recarga não grava no `audit_log` (é configuração da instância, não dado
  de org); ficam o log e a resposta com o antes/depois.
