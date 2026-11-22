# Synapse de desenvolvimento isolado com dois Application Services (MX-01)

Ambiente da **MX-01** do módulo Matrix do POP Harness: um Synapse de desenvolvimento que carrega ao
mesmo tempo **dois registros de Application Service** — o do POP Atende (Python), com tokens
fictícios, e o **novo** do Harness (prefixos `ph_`). Serve de base para a MX-02 (spike dos contratos
do Synapse) e, mais adiante, para os testes `#[ignore]` do módulo.

- **Não é** o dev do POP Atende (`popdev-synapse`, porta 8008), **não é** a máquina ARM64 de testes e **não é** produção.
  Container, rede, portas e dados próprios; nada aqui abre porta para fora de `127.0.0.1`.
- Imagem: `matrixdotorg/synapse:v1.120.0` (já presente nesta estação).
- Tudo fora do git, em `~/.cache/poph-matrix` (dá para trocar com `POPH_MATRIX_DEV_DIR`).

## Subir, parar, limpar

```bash
cd apps/pop-harness
bash scripts/matrix-dev.sh subir    # sobe/reinicia e espera /_matrix/client/versions responder
bash scripts/matrix-dev.sh parar    # para o container, preserva os dados
bash scripts/matrix-dev.sh limpar   # apaga SÓ o que o script criou
bash scripts/matrix-dev.sh url      # imprime a URL e o server_name
```

`limpar` remove o container `poph-matrix-synapse`, a rede `poph-matrix-net` e o diretório
`~/.cache/poph-matrix`. Não há volume Docker do projeto (o `/data` é bind mount), então
`docker volume ls` fica igual antes e depois. Nenhum container de outro projeto é tocado.

## Peças

| Peça | Onde | Porta |
|---|---|---|
| Synapse de dev | container `poph-matrix-synapse`, rede `poph-matrix-net` | `127.0.0.1:18008` |
| Registro do Python (teste) | `~/.cache/poph-matrix/data/registro-python.yaml` | url padrão `http://<IP-LAN>:18009` |
| Registro do Harness (novo) | `~/.cache/poph-matrix/data/registro-harness.yaml` | url padrão `http://<IP-LAN>:18010/matrix` |
| `homeserver.yaml`, chave de assinatura, banco SQLite | `~/.cache/poph-matrix/data/` | — |
| Tokens fictícios (para a MX-02) | `~/.cache/poph-matrix/credenciais.env` (0600) | — |
| Modelos dos registros (versionados, sem token) | `apps/pop-harness/tests/matrix/*.template` | — |

`server_name` do ambiente: `matrix-dev.local`.

## Os dois registros

| | Python (teste) | Harness (novo) |
|---|---|---|
| `id` | o do registro do Python | `poph-matrix-dev` (modelo) ou `poph-matrix` (gerado pelo `poph`) |
| `sender_localpart` | `wa_as_bot` | `ph_as_bot` |
| usuários exclusivos | `@wa_.*`, `@agt_.*`, `@em_.*`, `@test_.*` | `@ph_.*` |
| aliases exclusivos | `#wa_.*`, `#em_.*` | `#ph_.*` |
| alias compartilhado | `#dep_.*` (não exclusivo) | `#dep_.*` (não exclusivo) |
| `rate_limited` | `false` | `false` |

Os namespaces são disjuntos (PRD §12): o Synapse sobe sem conflito e o log mostra as duas linhas
`Loaded application service` (`id=poph-matrix-dev` e `id=popatende-python-teste`).

## Credenciais ficam fora do git

Os templates versionados têm só os placeholders `__AS_TOKEN__`, `__HS_TOKEN__` e `__AS_URL__`.
O `subir` gera **quatro tokens fictícios novos a cada execução** (Python e Harness, `as_token` e
`hs_token`), grava os registros renderizados em `~/.cache/poph-matrix/data/` e os valores em
`~/.cache/poph-matrix/credenciais.env` (0600). O script **não imprime token nenhum**; quem precisa
deles (MX-02, testes) lê o `credenciais.env`. Nada disso entra em commit: o diretório fica em
`~/.cache`.

Por que os registros renderizados ficam em `0644`: o Synapse do container roda como uid `991`, que
não é o uid do host neste docker rootless; um arquivo `0600` do host ficaria ilegível para ele. O
sigilo vem do diretório (fora do repo, `credenciais.env` em 0600) e de os tokens serem fictícios.

## Rede: por que a url usa o IP da LAN

Neste docker rootless (slirp4netns `--disable-host-loopback`), o container **não alcança
`127.0.0.1` do host**. Por isso as `url` dos registros usam o **IP da LAN** (`hostname -I`) e portas
altas: 18009 para o AS do Python e 18010 para o do Harness. A do Harness tem o caminho `/matrix` de
propósito — a MX-02 precisa medir se o Synapse chama `<url>/_matrix/app/v1/transactions/{txnId}`
quando a `url` tem prefixo.

Nada precisa estar escutando nessas portas para a MX-01: o Synapse só chama a `url` quando há evento
para o AS. Para a MX-02, suba o receptor nas portas 18009/18010 (ou troque as urls antes de subir):

```bash
POPH_MATRIX_DEV_URL_H="http://192.168.0.123:18010/matrix" bash scripts/matrix-dev.sh subir
```

Variáveis aceitas: `POPH_MATRIX_DEV_DIR`, `POPH_MATRIX_DEV_PORTA`, `POPH_MATRIX_DEV_PORTA_PY`,
`POPH_MATRIX_DEV_PORTA_H`, `POPH_MATRIX_DEV_URL_PY`, `POPH_MATRIX_DEV_URL_H`,
`POPH_MATRIX_DEV_IMAGEM`, `POPH_MATRIX_DEV_REGISTRO_H` e `POPH_MATRIX_DEV_REUSAR_TOKENS`
(as duas últimas na seção seguinte).

## Registro do Harness gerado pelo `poph` (MX-21)

Até a MX-20 o registro do Harness era o modelo renderizado pelo script. A partir da MX-21 quem gera
o registro de verdade é a CLI, com os tokens do ambiente:

```bash
# 1. sobe (ou reinicia) o ambiente e grava os tokens fictícios em credenciais.env
bash scripts/matrix-dev.sh subir

# 2. gera o registro do Harness com os MESMOS tokens, fora do git
. ~/.cache/poph-matrix/credenciais.env
POPH_MATRIX_AS_TOKEN="$POPH_MATRIX_DEV_HARNESS_AS_TOKEN" \
POPH_MATRIX_HS_TOKEN="$POPH_MATRIX_DEV_HARNESS_HS_TOKEN" \
  ./target/debug/poph --config ~/.cache/poph-matrix/poph-matrix.toml matrix registro \
    --saida ~/.cache/poph-matrix/registro-gerado.yaml \
    --url "http://$(hostname -I | awk '{print $1}'):18010/matrix"

# 3. o Synapse passa a carregar esse arquivo, reaproveitando os tokens do passo 1
POPH_MATRIX_DEV_REUSAR_TOKENS=1 \
POPH_MATRIX_DEV_REGISTRO_H="$HOME/.cache/poph-matrix/registro-gerado.yaml" \
  bash scripts/matrix-dev.sh subir
```

- `POPH_MATRIX_DEV_REGISTRO_H=<arquivo>` manda o script copiar um registro **pronto** para
  `/data/registro-harness.yaml` em vez de renderizar o modelo. Use `--verificar` do próprio comando
  para saber se ele ainda bate com a seção `[matrix]`.
- `POPH_MATRIX_DEV_REUSAR_TOKENS=1` **não** gera tokens novos: relê o `credenciais.env`. Sem isso, o
  `subir` trocaria os tokens e o registro pronto ficaria com `as_token`/`hs_token` velhos.
- `poph matrix registro` grava com permissão **0600** (o Synapse lê pela cópia em `0644` que o script
  faz dentro do container) e nunca imprime token. `poph matrix registro --verificar <arquivo>` aponta
  cada divergência nomeando o campo (`url`, `sender_localpart`, `rate_limited`, `namespaces.users`,
  `namespaces.aliases`, `as_token`, `hs_token`).
- O `--url` é o endereço **por onde o Synapse alcança o Harness** (não é o `homeserver_url`, que é o
  Synapse): aqui, o IP da LAN com a porta 18010 e o caminho `/matrix`. Sem `--url`, vale a variável
  `POPH_MATRIX_URL_AS`.
- A configuração do roteiro é um `poph.toml` só com `[matrix]` (`habilitado = true`,
  `homeserver_url = "http://127.0.0.1:18008"`, `server_name = "matrix-dev.local"`), fora do git.

Prova de que o Synapse aceitou o registro gerado (MX-21): o log mostra `Loaded application service`
com `id=poph-matrix`, `url=http://<IP-LAN>:18010/matrix`, `sender=@ph_as_bot:matrix-dev.local`,
`rate_limited: False`, e uma chamada `createRoom` autenticada com o `as_token` do arquivo devolve
`200` com `room_id` (token errado devolve `401 M_UNKNOWN_TOKEN`).

## Como conferir que está no ar

```bash
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:18008/_matrix/client/versions   # 200
docker logs poph-matrix-synapse 2>&1 | grep 'Loaded application service'                  # duas linhas
docker ps --filter name=poph-matrix --format '{{.Names}} {{.Status}}'                     # healthy
```

O `subir` já faz a primeira e imprime as duas linhas de `Loaded application service`. O container
tem healthcheck da própria imagem (`healthy`).

## Limites deste ambiente

- Sem federação (`trusted_key_servers: []`), sem TLS, sem cadastro aberto.
- Dados descartáveis: `limpar` apaga tudo. Não guarde nada aqui que precise sobreviver.
- Um Synapse lê os registros só na subida: trocar token ou url exige `subir` de novo.
- É da MX-01/MX-21 (validação e testes). O registro real do Harness é gerado pelo
  `poph matrix registro` (MX-21), com tokens de verdade, e exige reiniciar o Synapse do ambiente —
  nunca este script.
